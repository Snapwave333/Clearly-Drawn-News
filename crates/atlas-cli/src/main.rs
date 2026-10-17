use atlas_core::{
    AgentMessage, FeedbackItem, Project, ProjectState, PublishMode, PublishPlan, Topic,
    TrendSource, redact_config,
};
use atlas_doctor::{has_minimal_pipeline, probe_all};
use atlas_opencode::OpenCodeClient;
use atlas_store::Store;
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use directories::{BaseDirs, ProjectDirs};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use atlas_analytics::{
    AnalyticsStore, DEFAULT_TIKTOK_USER, OptimizationLoop, PublishCadencePolicy, PublishDecision,
    PublishLedger, PublishSchedulerState, PublishService, PublishedFormat, PublishedItem,
    RecommendationEngine, SimulatedPayrollLedger, TikTokClient, TikTokPostMode, TikTokPrivacyLevel,
    TikTokUploadMetadata, TikTokUploadStatus, YouTubeAnalyticsClient, YouTubeCredentials,
    YouTubeUploadMetadata, authorization_url, compare_to_baseline, compute_channel_baseline,
    load_tiktok_credentials, load_youtube_credentials, media_fingerprint, save_sample_metrics,
};
use atlas_blender::{
    BlenderPipeline, LipSyncCue, MascotEmotion, MascotGesture, MascotMorph, MascotSpec,
    RenderQuality, TransformSpec, Viseme, VisualElement, VisualElementType, create_test_shot_spec,
};
use atlas_ffmpeg::{
    AudioTrack, CaptionFormat, CaptionTrack, EditDecisionList, EdlEntry, FfmpegAssembler,
    SourceType,
};
use atlas_kokoro::{
    AudioSegment, KokoroPipeline, NarrationSegment, NarrationTrack, VoiceConfig,
    split_script_into_segments,
};
use atlas_media::{ImportedMedia, MediaKind, MediaOrientation, StockMediaClient};
use atlas_memory::{MemoryEngine, TaskStatus as MemoryTaskStatus};
use atlas_policy::{
    ChannelBrowserReview, ChannelPreflight, CheckStatus, ContentPreflight, PolicyGuardian,
    TikTokPreflight, content_checks, tiktok_content_checks,
};
use atlas_storyboard::{
    DirectorClient, Emotion as DirectedEmotion, Gesture as DirectedGesture, ScenePattern,
    StoryboardBeat, StoryboardPlan,
};

#[derive(Parser)]
#[command(name = "atlas")]
#[command(about = "ATLAS STUDIO — autonomous YouTube educational filmmaking")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Data directory for ATLAS (defaults to project-local data/).
    #[arg(long, global = true, env = "ATLAS_DATA_DIR")]
    data_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum VideoFormat {
    Long,
    Short,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum PublishPrivacy {
    Private,
    Unlisted,
    Public,
}

impl PublishPrivacy {
    fn as_youtube_value(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Unlisted => "unlisted",
            Self::Public => "public",
        }
    }

    fn as_tiktok_value(self) -> TikTokPrivacyLevel {
        match self {
            Self::Private => TikTokPrivacyLevel::SelfOnly,
            Self::Unlisted => TikTokPrivacyLevel::MutualFollowFriends,
            Self::Public => TikTokPrivacyLevel::PublicToEveryone,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum, Default, PartialEq, Eq)]
enum PublishPlatformCli {
    #[default]
    #[value(name = "youtube")]
    YouTube,
    #[value(name = "tiktok")]
    TikTok,
    #[value(name = "both")]
    Both,
}

#[derive(Subcommand)]
enum TikTokAction {
    /// Inspect connected TikTok account profile info.
    Info {
        #[arg(long, default_value = DEFAULT_TIKTOK_USER)]
        user: String,
    },
    /// Check processing and publication status of a TikTok upload job.
    Status {
        #[arg(long)]
        request_id: String,
    },
    /// Upload rendered whiteboard video for a topic directly to TikTok.
    Upload {
        #[arg(long)]
        topic: String,
        #[arg(long, value_enum, default_value_t = VideoFormat::Short)]
        format: VideoFormat,
        #[arg(long, value_enum, default_value_t = PublishPrivacy::Public)]
        privacy: PublishPrivacy,
        #[arg(long, default_value = DEFAULT_TIKTOK_USER)]
        user: String,
        #[arg(long)]
        draft: bool,
    },
    /// View TikTok performance and audience retention analytics.
    Analytics {
        #[arg(long)]
        video_id: String,
        #[arg(long)]
        sample: bool,
    },
}

#[derive(Clone, Copy)]
struct VideoProfile {
    slug: &'static str,
    width: u32,
    height: u32,
    shot_count: u32,
}

impl VideoFormat {
    fn profile(self) -> VideoProfile {
        match self {
            Self::Long => VideoProfile {
                slug: "long",
                width: 1920,
                height: 1080,
                shot_count: 10,
            },
            Self::Short => VideoProfile {
                slug: "short",
                width: 1080,
                height: 1920,
                shot_count: 5,
            },
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Discover installed tools, OpenCode account, and render capacity.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Show or edit configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Probe NVIDIA model availability through OpenCode.
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// List or inspect projects.
    Projects {
        #[command(subcommand)]
        action: ProjectsAction,
    },
    /// Create a new topic/project.
    Topics {
        #[command(subcommand)]
        action: TopicsAction,
    },
    /// Produce a video (requires project id).
    Produce {
        #[arg(long)]
        topic: String,
        #[arg(long)]
        reuse_rendered: bool,
        #[arg(long, value_enum, default_value_t = VideoFormat::Long)]
        format: VideoFormat,
        #[arg(long)]
        publish: bool,
        #[arg(long, value_enum, default_value_t = PublishPrivacy::Private)]
        privacy: PublishPrivacy,
        #[arg(long, value_enum, default_value_t = PublishPlatformCli::YouTube)]
        platform: PublishPlatformCli,
        #[arg(long, default_value = DEFAULT_TIKTOK_USER)]
        tiktok_user: String,
        #[arg(long)]
        tiktok_draft: bool,
    },
    Publish {
        #[arg(long)]
        topic: String,
        #[arg(long, value_enum, default_value_t = VideoFormat::Long)]
        format: VideoFormat,
        #[arg(long, value_enum, default_value_t = PublishPrivacy::Private)]
        privacy: PublishPrivacy,
        #[arg(long, value_enum, default_value_t = PublishPlatformCli::YouTube)]
        platform: PublishPlatformCli,
        #[arg(long, default_value = DEFAULT_TIKTOK_USER)]
        tiktok_user: String,
        #[arg(long)]
        tiktok_draft: bool,
    },
    /// TikTok publishing, profile inspection, and upload status.
    #[command(name = "tiktok")]
    TikTok {
        #[command(subcommand)]
        action: TikTokAction,
    },
    /// Start the headless daemon.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// YouTube analytics, retention, and optimization recommendations.
    Analytics {
        #[command(subcommand)]
        action: AnalyticsAction,
    },
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    Payroll {
        #[command(subcommand)]
        action: PayrollAction,
    },
    /// Dual-memory orchestration: Shared Context MCP (working) + Graphiti (knowledge graph).
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    Show {
        #[arg(long)]
        redact: bool,
    },
}

#[derive(Subcommand)]
enum ModelsAction {
    Probe,
}

#[derive(Subcommand)]
enum ProjectsAction {
    List {
        #[arg(long)]
        json: bool,
    },
    Inspect {
        video_id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TopicsAction {
    Create { title: Vec<String> },
}

#[derive(Subcommand)]
enum DaemonAction {
    Start {
        #[arg(long)]
        once: bool,
        #[arg(long, default_value_t = 300)]
        poll_seconds: u64,
        #[arg(long, value_enum, default_value_t = PublishPrivacy::Public)]
        privacy: PublishPrivacy,
        #[arg(long, default_value = DEFAULT_TIKTOK_USER)]
        tiktok_user: String,
    },
    Pause,
}

#[derive(Subcommand)]
enum PolicyAction {
    Status,
    Refresh,
    RecordChannelReview {
        #[arg(long)]
        channel_id: String,
        #[arg(long)]
        warning_visible: bool,
        #[arg(long)]
        strike_visible: bool,
        #[arg(long)]
        training_completed: bool,
        #[arg(long)]
        warning_expires_at: Option<String>,
        #[arg(long)]
        restricted_policy: Option<String>,
    },
    AcceptCurrent,
}

#[derive(Subcommand)]
enum PayrollAction {
    Import {
        #[arg(long)]
        platform: String,
        #[arg(long)]
        period: String,
        #[arg(long)]
        revenue_usd: f64,
        #[arg(long)]
        source_file: PathBuf,
    },
    Status,
}

#[derive(Subcommand)]
enum MemoryAction {
    /// Probe and display health of Shared Context MCP and Graphiti knowledge graph.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Semantic search across Graphiti long-term knowledge graph.
    Recall {
        query: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// List active tasks and scratchpad notes in Shared Context.
    Tasks {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Create a task on the Shared Context working board.
    CreateTask {
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        agent: Option<String>,
    },
    /// Add context/scratchpad note to an active task.
    AddNote {
        #[arg(long)]
        task: String,
        #[arg(long)]
        note: String,
    },
    /// Update status of a task (pending, in_progress, review, completed, failed).
    UpdateTask {
        #[arg(long)]
        task: String,
        #[arg(long)]
        status: String,
    },
    /// Promote a completed task from Shared Context to Graphiti knowledge graph.
    Promote {
        #[arg(long)]
        task: String,
        #[arg(long)]
        notes: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Auto-promote all completed unpromoted tasks to Graphiti.
    AutoPromote {
        #[arg(long)]
        json: bool,
    },
    /// Prime and bootstrap a new topic with relevant historical Graphiti knowledge.
    Preload {
        #[arg(long)]
        topic: String,
        #[arg(long)]
        agent: Option<String>,
    },
    /// Prune old completed ephemeral task scratchpads from Shared Context.
    Prune {
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
    /// Continuous memory bridge daemon: syncs and auto-promotes completed tasks in the background.
    Daemon {
        #[arg(long, default_value_t = 10)]
        poll_seconds: u64,
        #[arg(long)]
        once: bool,
    },
}

#[derive(Subcommand)]
enum AnalyticsAction {
    /// Print the OAuth URL and instructions; complete with `auth complete`.
    Auth {
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        client_secret: Option<String>,
        #[arg(long)]
        redirect_uri: Option<String>,
    },
    /// Complete OAuth after visiting the authorization URL.
    AuthComplete {
        #[arg(long)]
        client_id: Option<String>,
        #[arg(long)]
        client_secret: Option<String>,
        #[arg(long)]
        code: String,
        #[arg(long)]
        redirect_uri: Option<String>,
    },
    /// Fetch metrics from YouTube Analytics or TikTok for a project.
    Fetch {
        video_id: String,
        #[arg(long)]
        sample: bool,
        #[arg(long, default_value = "youtube")]
        platform: String,
        #[arg(long, default_value = "long")]
        format: String,
    },
    /// Show the retention curve for a video.
    Retention {
        video_id: String,
        #[arg(long, default_value = "youtube")]
        platform: String,
    },
    /// Generate optimization recommendations from stored metrics.
    Recommend {
        video_id: String,
        #[arg(long, default_value = "300")]
        duration_seconds: u64,
        #[arg(long, default_value = "youtube")]
        platform: String,
    },
    /// Create agent revision tasks from analytics recommendations.
    Optimize {
        video_id: String,
        #[arg(long, default_value = "300")]
        duration_seconds: u64,
    },
    /// Show a dashboard of all stored analytics.
    Dashboard,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(default_data_dir);
    let store = Store::open(&data_dir)?;

    match cli.command {
        Commands::Doctor { json } => cmd_doctor(json).await,
        Commands::Config { action } => cmd_config(&store, action).await,
        Commands::Models { action } => cmd_models(action).await,
        Commands::Projects { action } => cmd_projects(&store, action).await,
        Commands::Topics { action } => cmd_topics(&store, action).await,
        Commands::Produce {
            topic,
            reuse_rendered,
            format,
            publish,
            privacy,
            platform,
            tiktok_user,
            tiktok_draft,
        } => {
            cmd_produce(
                &store,
                ProduceOptions {
                    topic_id: &topic,
                    reuse_rendered,
                    format,
                    publish,
                    privacy,
                    platform,
                    tiktok_user: &tiktok_user,
                    tiktok_draft,
                },
            )
            .await
        }
        Commands::Publish {
            topic,
            format,
            privacy,
            platform,
            tiktok_user,
            tiktok_draft,
        } => {
            cmd_publish(
                &store,
                PublishOptions {
                    topic_id: &topic,
                    format,
                    privacy,
                    platform,
                    tiktok_user: &tiktok_user,
                    tiktok_draft,
                    cleanup_after_upload: true,
                },
            )
            .await
        }
        Commands::TikTok { action } => cmd_tiktok(&store, action).await,
        Commands::Daemon { action } => cmd_daemon(&store, action).await,
        Commands::Analytics { action } => cmd_analytics(&store, action).await,
        Commands::Policy { action } => cmd_policy(&store, action).await,
        Commands::Payroll { action } => cmd_payroll(&store, action).await,
        Commands::Memory { action } => cmd_memory(action).await,
    }
}

fn default_data_dir() -> PathBuf {
    if let Some(proj) = ProjectDirs::from("com", "atlas-studio", "atlas") {
        proj.data_local_dir().to_path_buf()
    } else {
        PathBuf::from("data")
    }
}

async fn cmd_doctor(json: bool) -> anyhow::Result<()> {
    let results = probe_all().await;
    let oc = OpenCodeClient::new();
    let nvidia = oc.nvidia_status().await.ok();
    let minimal = has_minimal_pipeline(&results);

    if json {
        let output = serde_json::json!({
            "tools": results,
            "nvidia_status": nvidia,
            "minimal_pipeline_ready": minimal,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("ATLAS STUDIO — System Probe");
        println!("===========================");
        for r in &results {
            match &r.status {
                atlas_core::ToolStatus::Available { version, path } => {
                    println!("[OK]   {:15} {}  ({})", r.kind, version, path)
                }
                atlas_core::ToolStatus::Unavailable { reason } => {
                    println!("[MISS] {:15} {}", r.kind, reason)
                }
                atlas_core::ToolStatus::OptionalUnavailable { reason } => {
                    println!("[OPT]  {:15} {}", r.kind, reason)
                }
            }
        }
        println!();
        if let Some(n) = nvidia {
            println!("OpenCode NVIDIA provider:");
            if let Some(acc) = &n.opencode_account {
                println!("  account: {} ({})", acc.provider, acc.account);
            } else {
                println!("  account: not configured in OpenCode");
            }
            println!("  models visible: {}", n.model_count);
            if !n.sample_models.is_empty() {
                println!("  samples: {}", n.sample_models.join(", "));
            }
        }
        println!();
        if minimal {
            println!("Minimal pipeline ready (OpenCode + Blender + FFmpeg).");
        } else {
            println!("Minimal pipeline NOT ready. Install missing required tools.");
        }
    }
    Ok(())
}

async fn cmd_config(store: &Store, action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Show { redact } => {
            let config = store.load_config()?;
            if redact {
                let redacted = redact_config(&config);
                println!("{}", serde_json::to_string_pretty(&redacted)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&config)?);
            }
        }
    }
    Ok(())
}

async fn cmd_models(action: ModelsAction) -> anyhow::Result<()> {
    match action {
        ModelsAction::Probe => {
            let oc = OpenCodeClient::new();
            let status = oc.nvidia_status().await?;
            let models = oc.list_nvidia_models().await?;
            let output = serde_json::json!({
                "provider_status": status,
                "models": models,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }
    Ok(())
}

async fn cmd_projects(store: &Store, action: ProjectsAction) -> anyhow::Result<()> {
    match action {
        ProjectsAction::List { json } => {
            let projects = store.list_projects(50)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&projects)?);
            } else {
                println!("{:<24} {:<24} TOPIC", "VIDEO ID", "STATE");
                for p in projects {
                    println!(
                        "{:<24} {:<24} {}",
                        p.video_id,
                        p.state.to_string(),
                        p.topic.title
                    );
                }
            }
        }
        ProjectsAction::Inspect { video_id, json } => {
            let project = store
                .get_project(&video_id)?
                .ok_or_else(|| anyhow::anyhow!("project not found: {}", video_id))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&project)?);
            } else {
                println!("Project: {}", project.video_id);
                println!("State:   {}", project.state);
                println!("Topic:   {}", project.topic.title);
                println!("Publish: {:?}", project.publish.mode);
            }
        }
    }
    Ok(())
}

async fn cmd_topics(store: &Store, action: TopicsAction) -> anyhow::Result<()> {
    match action {
        TopicsAction::Create { title } => {
            let title = title.join(" ");
            let video_id = format!(
                "vid_{}_{:04}",
                Utc::now().format("%Y%m%d"),
                Uuid::new_v4().as_simple().to_string()[..4]
                    .parse::<u16>()
                    .unwrap_or(0)
            );
            let project = Project {
                video_id: video_id.clone(),
                topic: Topic {
                    title: title.clone(),
                    trend_sources: vec![TrendSource {
                        source: "manual".to_string(),
                        url: None,
                        observed_at: Utc::now(),
                    }],
                    observed_at: Some(Utc::now()),
                    region: Some("US".to_string()),
                },
                state: ProjectState::IdeaDiscovered,
                channel_profile: "primary".to_string(),
                visual_thesis: None,
                script_artifact_id: None,
                research_artifact_id: None,
                storyboard_artifact_id: None,
                publish: PublishPlan {
                    mode: PublishMode::ApprovalRequired,
                    contains_synthetic_media: None,
                    credits_present: false,
                    youtube_video_id: None,
                    youtube_video_ids: Default::default(),
                    tiktok_post_id: None,
                    tiktok_post_ids: Default::default(),
                    tiktok_request_id: None,
                },
            };
            store.insert_project(&project)?;
            // Seed a producer message.
            store.insert_message(&AgentMessage {
                schema_version: 1,
                message_id: format!("msg_{}", Uuid::new_v4()),
                project_id: video_id.clone(),
                from: "executive_producer".to_string(),
                to: "trend_analyst".to_string(),
                message_type: "TASK_CREATED".to_string(),
                task_id: Some("topic_approval".to_string()),
                artifact_ids: vec![],
                blocking: false,
                priority: "normal".to_string(),
                feedback: vec![FeedbackItem {
                    claim_id: None,
                    issue: "Manual topic created; approve for research.".to_string(),
                    required_change: "Run `atlas produce --topic <id>` to start production."
                        .to_string(),
                }],
                created_at: Utc::now(),
                deadline_at: None,
            })?;
            println!("Created topic/project: {}", video_id);
            println!("Title: {}", title);
        }
    }
    Ok(())
}

struct ProduceOptions<'a> {
    topic_id: &'a str,
    reuse_rendered: bool,
    format: VideoFormat,
    publish: bool,
    privacy: PublishPrivacy,
    platform: PublishPlatformCli,
    tiktok_user: &'a str,
    tiktok_draft: bool,
}

struct PublishOptions<'a> {
    topic_id: &'a str,
    format: VideoFormat,
    privacy: PublishPrivacy,
    platform: PublishPlatformCli,
    tiktok_user: &'a str,
    tiktok_draft: bool,
    cleanup_after_upload: bool,
}

async fn cmd_produce(store: &Store, options: ProduceOptions<'_>) -> anyhow::Result<()> {
    let ProduceOptions {
        topic_id,
        reuse_rendered,
        format,
        publish,
        privacy,
        platform,
        tiktok_user,
        tiktok_draft,
    } = options;
    let mut project = store
        .get_project(topic_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {}", topic_id))?;
    let profile = format.profile();
    let render_project_id = format!("{}_{}", project.video_id, profile.slug);

    println!("═══ ATLAS STUDIO Vertical Slice ═══");
    println!("Project: {} — {}", project.video_id, project.topic.title);
    println!(
        "Format: {} ({}x{}, {} shots)",
        profile.slug, profile.width, profile.height, profile.shot_count
    );
    println!();

    println!("📝 Stage 1: Generating storyboard from script...");
    project.state = ProjectState::Scripting;
    store.insert_project(&project)?;
    let project_dir = store
        .data_dir()
        .join("projects")
        .join(&project.video_id)
        .join(profile.slug);
    let long_storyboard_path = store
        .data_dir()
        .join("projects")
        .join(&project.video_id)
        .join("long")
        .join("storyboard.json");
    let long_plan = if matches!(format, VideoFormat::Short) {
        Some(serde_json::from_slice::<StoryboardPlan>(&std::fs::read(
            &long_storyboard_path,
        )?)?)
    } else {
        None
    };
    let planned = planned_video(&project.topic.title, format, long_plan.as_ref()).await?;
    let beats = planned.beats;
    let script = beats
        .iter()
        .map(VideoBeat::narration)
        .collect::<Vec<_>>()
        .join(" ");
    std::fs::create_dir_all(&project_dir)?;
    std::fs::write(project_dir.join("script.txt"), &script)?;
    if let Some(plan) = planned.generated_plan {
        std::fs::write(
            project_dir.join("storyboard.json"),
            serde_json::to_vec_pretty(&plan)?,
        )?;
    }
    println!("   Script: {} chars", script.len());

    println!("🎙️  Stage 2: Synthesizing narration and timing...");
    project.state = ProjectState::AudioBuild;
    store.insert_project(&project)?;
    let data_dir = store.data_dir().to_path_buf();
    let voice_config = VoiceConfig::default();
    let kokoro_pipeline = KokoroPipeline::new("kokoro-tts", voice_config, &data_dir);
    let mut segments = split_script_into_segments(&script, "af_heart");
    for segment in &mut segments {
        segment.pause_after_ms = 0;
    }
    if segments.len() != beats.len() {
        anyhow::bail!(
            "Storyboard has {} beats but narration produced {} segments",
            beats.len(),
            segments.len()
        );
    }
    let narration = kokoro_pipeline
        .synthesize_track(segments.clone(), project.video_id.clone())
        .await?;
    let combined_audio = narration
        .combined_file
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Kokoro did not return a combined narration file"))?;
    println!("   ✓ Timed narration: {}", combined_audio.display());

    println!("🎞️  Importing topic-matched B-roll...");
    let broll_queries = beats
        .iter()
        .map(|beat| beat.broll_query().to_string())
        .collect::<Vec<_>>();
    let broll = import_broll(&project.video_id, format, &data_dir, &broll_queries).await;
    println!("   ✓ {} attributed media assets available", broll.len());

    println!("🎬 Stage 3: Creating lip-synced shot specifications...");
    project.state = ProjectState::Storyboarding;
    store.insert_project(&project)?;

    let mut shot_specs = Vec::new();
    for (index, (beat, audio)) in beats.iter().zip(&narration.segments).enumerate() {
        let mut spec = create_test_shot_spec(&render_project_id, &format!("shot_{:03}", index + 1));
        spec.duration_frames = ((audio.duration_seconds + 0.4) * 30.0).ceil() as u32;
        spec.scene_template = beat.scene().to_string();
        spec.visual_elements.truncate(1);
        spec.visual_elements[0].properties["text"] = serde_json::json!(beat.title());
        spec.visual_elements[0].properties["font_size"] = serde_json::json!(20.0);
        spec.visual_elements[0].hold_frames = spec.duration_frames;
        spec.mascot = Some(MascotSpec {
            name: "Luma".into(),
            emotion: beat.emotion(),
            gesture: beat.gesture(),
            lip_sync: lip_sync_cues(audio),
            morph: beat.morph(),
        });
        if index == 0 {
            spec.background_media = broll
                .iter()
                .find(|media| matches!(media.kind, MediaKind::Image))
                .map(|media| media.file_path.clone());
        }
        if index == 3 {
            spec.background_media = broll
                .iter()
                .find(|media| matches!(media.kind, MediaKind::Video))
                .map(|media| media.file_path.clone());
        }
        if matches!(format, VideoFormat::Short) {
            spec.camera.ortho_scale = Some(10.0);
            spec.visual_elements[0].transform.position[1] = 3.2;
        } else {
            spec.visual_elements[0].transform.position[1] = 1.6;
        }
        beat.apply_visuals(&mut spec, matches!(format, VideoFormat::Short));
        shot_specs.push(spec);
    }
    println!("   {} shots planned", shot_specs.len());

    println!("🎨 Stage 4: Rendering animated lessons with Blender...");
    project.state = ProjectState::AssetBuild;
    store.insert_project(&project)?;

    let blender_exe = "blender";
    let blender_pipeline = BlenderPipeline::new(blender_exe, &data_dir);

    for spec in &shot_specs {
        println!("   Rendering {}...", spec.shot_id);
        let frames_dir = data_dir
            .join("renders")
            .join(&render_project_id)
            .join(&spec.shot_id);
        let reusable_frames = if reuse_rendered && frames_dir.is_dir() {
            std::fs::read_dir(&frames_dir)?
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
                .count()
        } else {
            0
        };
        if reusable_frames == spec.duration_frames as usize {
            println!("   ✓ {} verified cached frames", reusable_frames);
            continue;
        }
        let job = atlas_blender::RenderJob {
            job_id: format!("job_{}", spec.shot_id),
            shot_spec: spec.clone(),
            quality: RenderQuality::Final,
            output_dir: frames_dir,
            status: atlas_blender::RenderStatus::Queued,
            output_size: Some([profile.width, profile.height]),
        };
        let output = blender_pipeline.render(&job).await?;
        println!("   ✓ {} real frames", output.frames.len());
    }

    println!("🔧 Stage 5: Assembling final video with FFmpeg...");
    project.state = ProjectState::EditAndMix;
    store.insert_project(&project)?;

    let ffmpeg_exe = "ffmpeg";
    let ffprobe_exe = "ffprobe";
    let ffmpeg_assembler = FfmpegAssembler::new(ffmpeg_exe, ffprobe_exe, &data_dir);

    let caption_dir = data_dir
        .join("captions")
        .join(&project.video_id)
        .join(profile.slug);
    std::fs::create_dir_all(&caption_dir)?;
    let caption_path = caption_dir.join("en.srt");
    write_captions(&caption_path, &segments, &narration)?;

    let edl = EditDecisionList {
        edl_id: format!("edl_{}", uuid::Uuid::new_v4().simple()),
        project_id: project.video_id.clone(),
        fps: 30,
        width: profile.width,
        height: profile.height,
        entries: shot_specs
            .iter()
            .scan(0_u32, |record_frame, spec| {
                let index = spec
                    .shot_id
                    .trim_start_matches("shot_")
                    .parse::<u32>()
                    .unwrap_or(1)
                    - 1;
                let shot_id = format!("shot_{:03}", index + 1);
                let entry = EdlEntry {
                    entry_id: shot_id.clone(),
                    source_type: SourceType::Video,
                    source_path: data_dir
                        .join("renders")
                        .join(&render_project_id)
                        .join(&shot_id)
                        .join("%04d.png"),
                    source_in_frame: 0,
                    source_out_frame: spec.duration_frames,
                    record_in_frame: *record_frame,
                    record_out_frame: *record_frame + spec.duration_frames,
                    transition: None,
                    audio_sync_offset_ms: 0,
                };
                *record_frame += spec.duration_frames;
                Some(entry)
            })
            .collect(),
        audio_tracks: vec![AudioTrack {
            track_id: "narration".into(),
            source_path: combined_audio.clone(),
            start_frame: 0,
            volume_db: 0.0,
            fade_in_frames: 15,
            fade_out_frames: 15,
        }],
        caption_tracks: vec![CaptionTrack {
            track_id: "captions_en".into(),
            format: CaptionFormat::Srt,
            source_path: caption_path.clone(),
            language: "en".into(),
        }],
    };

    let output_video = data_dir
        .join("exports")
        .join(&project.video_id)
        .join(profile.slug)
        .join("final.mp4");
    std::fs::create_dir_all(
        data_dir
            .join("exports")
            .join(&project.video_id)
            .join(profile.slug),
    )?;

    let assembly = ffmpeg_assembler.assemble(&edl, &output_video).await?;
    if assembly.duration_seconds < 5.0 || assembly.frame_count < 150 {
        anyhow::bail!(
            "Release QA rejected output: {:.2}s and {} frames",
            assembly.duration_seconds,
            assembly.frame_count
        );
    }
    if matches!(format, VideoFormat::Short) && assembly.duration_seconds > 60.0 {
        anyhow::bail!(
            "Shorts QA rejected {:.2}s output; maximum is 60 seconds",
            assembly.duration_seconds
        );
    }
    if matches!(format, VideoFormat::Long) && assembly.duration_seconds <= 60.0 {
        anyhow::bail!(
            "Long-form QA rejected {:.2}s output; duration must exceed 60 seconds",
            assembly.duration_seconds
        );
    }
    if assembly.width != profile.width || assembly.height != profile.height {
        anyhow::bail!(
            "Release QA rejected {}x{} output; expected {}x{}",
            assembly.width,
            assembly.height,
            profile.width,
            profile.height
        );
    }
    let output_size = std::fs::metadata(&output_video)?.len();
    if output_size < 100_000 {
        anyhow::bail!(
            "Release QA rejected undersized output: {} bytes",
            output_size
        );
    }
    let export_dir = output_video
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid export path"))?;
    std::fs::copy(
        data_dir
            .join("renders")
            .join(&render_project_id)
            .join("shot_001")
            .join(format!(
                "{:04}.png",
                (shot_specs[0].duration_frames / 2).max(1)
            )),
        export_dir.join("thumbnail.png"),
    )?;
    add_vector_thumbnail_title(
        &export_dir.join("thumbnail.png"),
        &project.topic.title,
        profile.width,
        profile.height,
    )
    .await;
    optimize_thumbnail(
        &export_dir.join("thumbnail.png"),
        profile.width,
        profile.height,
    )
    .await;
    let title = match format {
        VideoFormat::Long => format!("{} | ATLAS STUDIO", project.topic.title),
        VideoFormat::Short => format!("{} | #Shorts", project.topic.title),
    };
    let metadata = serde_json::json!({
        "title": title,
        "description": "A concise whiteboard explanation produced by ATLAS STUDIO. Narration and visuals were generated with AI-assisted production tools. Educational demonstration; sources should be reviewed before reuse.",
        "transcript": script,
        "contains_synthetic_media": true,
        "credits": ["ATLAS STUDIO", "Blender", "Kokoro TTS", "FFmpeg"],
        "broll_attribution": broll,
        "format": profile.slug,
        "width": profile.width,
        "height": profile.height,
        "duration_seconds": assembly.duration_seconds,
        "frame_count": assembly.frame_count,
        "size_bytes": output_size,
        "derived_from": if matches!(format, VideoFormat::Short) {
            Some(long_storyboard_path.to_string_lossy().to_string())
        } else {
            None
        }
    });
    std::fs::write(
        export_dir.join("youtube_metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!(
        "   ✓ Final video: {} ({:.2}s, {} frames, {} bytes)",
        output_video.display(),
        assembly.duration_seconds,
        assembly.frame_count,
        output_size
    );

    println!("✅ Stage 6: Quality check & package...");
    project.state = ProjectState::ReleaseQa;
    store.insert_project(&project)?;

    println!();
    println!("═══ Production Complete ═══");
    println!("Output: {}", output_video.display());
    println!(
        "Renders: {}",
        data_dir.join("renders").join(&project.video_id).display()
    );
    println!("Audio:   {}", combined_audio.display());
    println!("Captions: {}", caption_path.display());
    println!();
    println!("Next steps:");
    println!("  atlas analytics fetch {} --sample", project.video_id);
    println!("  atlas analytics recommend {}", project.video_id);
    println!("  atlas analytics optimize {}", project.video_id);

    project.publish.contains_synthetic_media = Some(true);
    project.publish.credits_present = true;
    project.state = ProjectState::ReadyToPublish;
    store.insert_project(&project)?;
    if publish {
        cmd_publish(
            store,
            PublishOptions {
                topic_id,
                format,
                privacy,
                platform,
                tiktok_user,
                tiktok_draft,
                cleanup_after_upload: true,
            },
        )
        .await?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct VideoPackageMetadata {
    title: String,
    description: String,
    transcript: String,
    contains_synthetic_media: bool,
    duration_seconds: f64,
    #[serde(default)]
    broll_attribution: Vec<ImportedMedia>,
}

async fn cmd_publish(store: &Store, options: PublishOptions<'_>) -> anyhow::Result<()> {
    let PublishOptions {
        topic_id,
        format,
        privacy,
        platform,
        tiktok_user,
        tiktok_draft,
        cleanup_after_upload,
    } = options;
    let mut project = store
        .get_project(topic_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {}", topic_id))?;
    let profile = format.profile();
    let export_dir = store
        .data_dir()
        .join("exports")
        .join(&project.video_id)
        .join(profile.slug);
    let video_path = export_dir.join("final.mp4");
    let thumbnail_path = export_dir.join("thumbnail.png");
    let caption_path = export_dir.join("captions_en.srt");
    if !video_path.is_file() {
        anyhow::bail!("final video not found: {}", video_path.display());
    }

    let publish_youtube = matches!(
        platform,
        PublishPlatformCli::YouTube | PublishPlatformCli::Both
    );
    let publish_tiktok = matches!(
        platform,
        PublishPlatformCli::TikTok | PublishPlatformCli::Both
    );

    if publish_youtube {
        for path in [&thumbnail_path, &caption_path] {
            if !path.is_file() {
                anyhow::bail!("publish package is incomplete: {}", path.display());
            }
        }
    }

    let package: VideoPackageMetadata =
        serde_json::from_slice(&std::fs::read(export_dir.join("youtube_metadata.json"))?)?;
    let ledger_path = store.data_dir().join("publish").join("ledger.json");
    let mut ledger = PublishLedger::load(&ledger_path)?;
    let fingerprint = media_fingerprint(&video_path)?;
    let published_format = match format {
        VideoFormat::Long => PublishedFormat::Long,
        VideoFormat::Short => PublishedFormat::Short,
    };

    if publish_youtube {
        let already_uploaded = project.publish.youtube_video_ids.contains_key(profile.slug);
        let publish_decision = if matches!(privacy, PublishPrivacy::Public) && !already_uploaded {
            ledger.decide(
                &PublishCadencePolicy::default(),
                published_format,
                &fingerprint,
                Utc::now(),
            )
        } else {
            PublishDecision::Allow
        };
        let unique_media = already_uploaded
            || !matches!(&publish_decision, PublishDecision::RejectDuplicate { .. });
        let credentials = load_youtube_credentials("youtube_credentials.json").await?;
        let client = YouTubeAnalyticsClient::new(credentials);
        if !client.is_authenticated()? {
            anyhow::bail!("YouTube is not authenticated; run `atlas analytics auth` first");
        }
        let channel = client.fetch_channel_status().await?;
        let broll_expected = !package.broll_attribution.is_empty();
        let broll_attribution_present = package.broll_attribution.iter().all(|media| {
            !media.provider.trim().is_empty()
                && !media.source_url.trim().is_empty()
                && !media.creator.trim().is_empty()
        });
        let guardian = PolicyGuardian::new(store.data_dir().join("policy"))?;
        let browser_review = guardian.channel_review()?;
        let report = guardian
            .vet(
                &ContentPreflight {
                    title: package.title.clone(),
                    description: package.description.clone(),
                    transcript: package.transcript.clone(),
                    contains_synthetic_media: package.contains_synthetic_media,
                    captions_present: caption_path.is_file(),
                    thumbnail_present: thumbnail_path.is_file(),
                    broll_expected,
                    broll_attribution_present,
                    unique_media,
                },
                &ChannelPreflight {
                    channel_id: channel.channel_id,
                    available: channel.available,
                    linked: channel.is_linked,
                    long_uploads_status: channel.long_uploads_status,
                    privacy_status: channel.privacy_status,
                    browser_review,
                },
                &format!("{}_{}", project.video_id, profile.slug),
            )
            .await?;
        println!(
            "Policy Guardian (YouTube): {}/{} checks passed",
            report
                .checks
                .iter()
                .filter(|check| check.status == CheckStatus::Pass)
                .count(),
            report.checks.len()
        );
        if !report.publish_allowed {
            project.state = ProjectState::ReadyToPublish;
            store.insert_project(&project)?;
            let blocked = report
                .checks
                .iter()
                .filter(|check| check.status != CheckStatus::Pass)
                .map(|check| format!("{}: {}", check.name, check.detail))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("Policy Guardian blocked publication: {blocked}");
        }
        match publish_decision {
            PublishDecision::Allow => {}
            PublishDecision::Defer { retry_at, reason } => {
                project.state = ProjectState::ReadyToPublish;
                store.insert_project(&project)?;
                println!("Public YouTube upload deferred until {retry_at}: {reason}");
                return Ok(());
            }
            PublishDecision::RejectDuplicate { existing_video_id } => {
                anyhow::bail!(
                    "Publication rejected: identical media already exists as YouTube video {existing_video_id}"
                );
            }
        }

        let upload_metadata = YouTubeUploadMetadata {
            title: package.title.clone(),
            description: package.description.clone(),
            tags: vec![
                "education".into(),
                "science".into(),
                "whiteboard animation".into(),
                profile.slug.into(),
            ],
            privacy_status: privacy.as_youtube_value().into(),
            category_id: "27".into(),
            contains_synthetic_media: package.contains_synthetic_media,
        };

        project.state = ProjectState::Uploading;
        store.insert_project(&project)?;
        let result = if let Some(existing_id) = project.publish.youtube_video_ids.get(profile.slug)
        {
            atlas_analytics::YouTubeUploadResult {
                video_id: existing_id.clone(),
                url: format!("https://youtu.be/{existing_id}"),
            }
        } else {
            match client.upload_video(&video_path, &upload_metadata).await {
                Ok(result) => result,
                Err(error) => {
                    project.state = ProjectState::ReadyToPublish;
                    store.insert_project(&project)?;
                    return Err(error.into());
                }
            }
        };

        project.publish.youtube_video_id = Some(result.video_id.clone());
        project
            .publish
            .youtube_video_ids
            .insert(profile.slug.into(), result.video_id.clone());
        project.state = ProjectState::UploadedProcessing;
        store.insert_project(&project)?;
        client
            .upload_thumbnail(&result.video_id, &thumbnail_path)
            .await?;
        client
            .upload_captions(&result.video_id, &caption_path, "en-US")
            .await?;
        project.state = ProjectState::Published;
        store.insert_project(&project)?;
        if matches!(privacy, PublishPrivacy::Public) {
            ledger.record(PublishedItem {
                project_id: project.video_id.clone(),
                youtube_video_id: result.video_id.clone(),
                format: published_format,
                published_at: Utc::now(),
                content_fingerprint: fingerprint.clone(),
                platform: "youtube".into(),
                tiktok_post_id: None,
            });
            ledger.save(&ledger_path)?;
        }
        println!(
            "Published {} video to YouTube: {}",
            profile.slug, result.url
        );
    }

    if publish_tiktok {
        println!(
            "Configuring TikTok publishing for @{} via Upload-Post...",
            tiktok_user
        );
        let tiktok_creds = load_tiktok_credentials("upload_post_credentials.json").await?;
        let tiktok_client = TikTokClient::new(tiktok_creds);

        let mut tiktok_already_uploaded =
            project.publish.tiktok_post_ids.contains_key(profile.slug);
        if !tiktok_already_uploaded
            && let Some(request_id) = project.publish.tiktok_request_id.clone()
        {
            let pending = tiktok_client.get_upload_status(&request_id).await?;
            if pending.status == "completed" && pending.success {
                if let Some(post_id) = pending.post_id.clone() {
                    project.publish.tiktok_post_id = Some(post_id.clone());
                    project
                        .publish
                        .tiktok_post_ids
                        .insert(profile.slug.into(), post_id.clone());
                    project.publish.tiktok_request_id = None;
                    project.state = ProjectState::Published;
                    store.insert_project(&project)?;
                    tiktok_already_uploaded = true;
                } else {
                    println!(
                        "TikTok completed request {request_id} without a post id; retaining media for verification."
                    );
                    return Ok(());
                }
            } else if pending.status == "failed" || !pending.success {
                project.publish.tiktok_request_id = None;
                project.state = ProjectState::ReadyToPublish;
                store.insert_project(&project)?;
                anyhow::bail!(
                    "TikTok request {request_id} failed: {}",
                    pending
                        .error_message
                        .unwrap_or_else(|| "unknown TikTok processing error".into())
                );
            } else {
                println!(
                    "TikTok request {request_id} is still processing; retaining media for retry."
                );
                return Ok(());
            }
        }
        let tiktok_decision = if matches!(privacy, PublishPrivacy::Public)
            && !tiktok_draft
            && !tiktok_already_uploaded
        {
            ledger.decide_tiktok(&PublishCadencePolicy::default(), &fingerprint, Utc::now())
        } else {
            PublishDecision::Allow
        };
        let tiktok_unique = !matches!(&tiktok_decision, PublishDecision::RejectDuplicate { .. });

        match tiktok_decision {
            PublishDecision::Allow => {}
            PublishDecision::Defer { retry_at, reason } => {
                project.state = ProjectState::ReadyToPublish;
                store.insert_project(&project)?;
                println!("Public TikTok upload deferred until {retry_at}: {reason}");
                return Ok(());
            }
            PublishDecision::RejectDuplicate { existing_video_id } => {
                anyhow::bail!(
                    "TikTok publication rejected: identical media already exists as TikTok post {existing_video_id}"
                );
            }
        }

        let tiktok_post_mode = if tiktok_draft {
            TikTokPostMode::MediaUpload
        } else {
            TikTokPostMode::DirectPost
        };
        let tiktok_privacy_level = if tiktok_draft {
            TikTokPrivacyLevel::SelfOnly
        } else {
            privacy.as_tiktok_value()
        };

        let caption = format!(
            "{} #education #science #whiteboardanimation #focogaming",
            package.title
        );
        let general_checks = content_checks(&ContentPreflight {
            title: package.title.clone(),
            description: package.description.clone(),
            transcript: package.transcript.clone(),
            contains_synthetic_media: package.contains_synthetic_media,
            captions_present: caption_path.is_file(),
            thumbnail_present: thumbnail_path.is_file(),
            broll_expected: !package.broll_attribution.is_empty(),
            broll_attribution_present: package.broll_attribution.iter().all(|media| {
                !media.provider.trim().is_empty()
                    && !media.source_url.trim().is_empty()
                    && !media.creator.trim().is_empty()
            }),
            unique_media: tiktok_unique,
        });
        let community_guidelines_compliant = general_checks
            .iter()
            .all(|check| check.status == CheckStatus::Pass);
        let tiktok_checks = tiktok_content_checks(&TikTokPreflight {
            title: package.title.clone(),
            description: caption.clone(),
            duration_seconds: package.duration_seconds,
            is_original: true,
            contains_synthetic_media: package.contains_synthetic_media,
            aigc_label_enabled: true,
            commercial_music_cleared: true,
            community_guidelines_compliant,
            account_warning_visible: false,
            account_strike_visible: false,
        });
        let tiktok_blocked = tiktok_checks.iter().any(|check| {
            check.status != CheckStatus::Pass && check.name != "tiktok_monetization_duration"
        });
        println!(
            "Policy Guardian (TikTok): {}/{} required checks passed",
            tiktok_checks
                .iter()
                .filter(|check| check.status == CheckStatus::Pass)
                .count(),
            tiktok_checks.len()
        );
        if tiktok_blocked {
            let blocked = tiktok_checks
                .iter()
                .filter(|check| {
                    check.status != CheckStatus::Pass
                        && check.name != "tiktok_monetization_duration"
                })
                .map(|check| format!("{}: {}", check.name, check.detail))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("Policy Guardian blocked TikTok publication: {blocked}");
        }
        let tiktok_metadata = TikTokUploadMetadata {
            title: caption,
            user: tiktok_user.to_string(),
            privacy_level: tiktok_privacy_level,
            post_mode: tiktok_post_mode,
            is_ai_generated: true, // Transparent AI disclosure for whiteboard animations
            disable_duet: false,
            disable_stitch: false,
            disable_comment: false,
        };

        project.state = ProjectState::Uploading;
        store.insert_project(&project)?;

        println!(
            "Uploading {} video to TikTok (@{}) via Upload-Post...",
            profile.slug, tiktok_user
        );
        let status = if tiktok_already_uploaded {
            TikTokUploadStatus {
                request_id: String::new(),
                status: "completed".into(),
                completed: 1,
                total: 1,
                success: true,
                post_id: project.publish.tiktok_post_ids.get(profile.slug).cloned(),
                url: None,
                error_message: None,
            }
        } else {
            let res = match tiktok_client
                .upload_video(&video_path, &tiktok_metadata)
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    project.state = ProjectState::ReadyToPublish;
                    store.insert_project(&project)?;
                    return Err(e.into());
                }
            };

            println!("TikTok upload accepted with request_id: {}", res.request_id);
            project.publish.tiktok_request_id = Some(res.request_id.clone());
            project.state = ProjectState::UploadedProcessing;
            store.insert_project(&project)?;

            println!("Polling TikTok processing status...");
            match tiktok_client
                .poll_until_complete(&res.request_id, std::time::Duration::from_secs(45))
                .await
            {
                Ok(status) => status,
                Err(_) => {
                    println!(
                        "Upload continues in the background. Track it with: atlas tiktok status --request-id {}",
                        res.request_id
                    );
                    return Ok(());
                }
            }
        };

        if !status.success {
            project.publish.tiktok_request_id = None;
            project.state = ProjectState::ReadyToPublish;
            store.insert_project(&project)?;
            anyhow::bail!(
                "TikTok processing failed: {}",
                status
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "unknown TikTok processing error".into())
            );
        }

        if let Some(post_id) = &status.post_id {
            project.publish.tiktok_post_id = Some(post_id.clone());
            project
                .publish
                .tiktok_post_ids
                .insert(profile.slug.into(), post_id.clone());
            project.publish.tiktok_request_id = None;
        }
        project.state = ProjectState::Published;
        store.insert_project(&project)?;

        if matches!(privacy, PublishPrivacy::Public)
            && !tiktok_draft
            && !ledger
                .items
                .iter()
                .any(|item| item.platform == "tiktok" && item.content_fingerprint == fingerprint)
        {
            ledger.record(PublishedItem {
                project_id: project.video_id.clone(),
                youtube_video_id: String::new(),
                format: published_format,
                published_at: Utc::now(),
                content_fingerprint: fingerprint.clone(),
                platform: "tiktok".into(),
                tiktok_post_id: status.post_id.clone(),
            });
            ledger.save(&ledger_path)?;
        }

        let post_url = status
            .url
            .unwrap_or_else(|| format!("https://www.tiktok.com/@{}", tiktok_user));
        println!(
            "Published {} video to TikTok (@{}): {}",
            profile.slug, tiktok_user, post_url
        );
    }

    if !publish_youtube && !publish_tiktok {
        println!("No platform selected for publication.");
    } else if cleanup_after_upload {
        cleanup_uploaded_media([video_path.as_path()]);
        if publish_youtube && thumbnail_path.is_file() {
            cleanup_uploaded_media([thumbnail_path.as_path()]);
        }
    }

    Ok(())
}

async fn cmd_tiktok(store: &Store, action: TikTokAction) -> anyhow::Result<()> {
    match action {
        TikTokAction::Info { user } => {
            let creds = load_tiktok_credentials("upload_post_credentials.json").await?;
            let client = TikTokClient::new(creds);
            println!("Probing TikTok profile connection for @{user}...");
            let info = client.fetch_user_status(&user).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        TikTokAction::Status { request_id } => {
            let creds = load_tiktok_credentials("upload_post_credentials.json").await?;
            let client = TikTokClient::new(creds);
            println!("Checking TikTok upload status for request {request_id}...");
            let status = client.get_upload_status(&request_id).await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        TikTokAction::Upload {
            topic,
            format,
            privacy,
            user,
            draft,
        } => {
            cmd_publish(
                store,
                PublishOptions {
                    topic_id: &topic,
                    format,
                    privacy,
                    platform: PublishPlatformCli::TikTok,
                    tiktok_user: &user,
                    tiktok_draft: draft,
                    cleanup_after_upload: true,
                },
            )
            .await?;
        }
        TikTokAction::Analytics { video_id, sample } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = if sample {
                let m = atlas_analytics::save_sample_short_metrics(store, &video_id, "tiktok")?;
                println!("Stored TikTok analytics snapshot for {}", video_id);
                m
            } else {
                analytics.load_platform_metrics(&video_id, "tiktok")?.ok_or_else(|| {
                    anyhow::anyhow!("no TikTok metrics found for {}. Run with --sample to generate or populate snapshot.", video_id)
                })?
            };
            println!(
                "TikTok Analytics & Retention — @focogaming ({})",
                metrics.video_id
            );
            println!("==================================================");
            println!("Views:        {}", metrics.views.unwrap_or(0));
            println!(
                "Avg Duration: {:.1}s ({:.1}% of video)",
                metrics.average_view_duration_seconds.unwrap_or(0.0),
                metrics.average_percentage_viewed.unwrap_or(0.0) * 100.0
            );
            println!(
                "Likes:        {} | Comments: {} | Shares: {}",
                metrics.likes.unwrap_or(0),
                metrics.comments.unwrap_or(0),
                metrics.shares.unwrap_or(0)
            );
            if let Some(hook) = metrics.short_hook_retention() {
                println!("3-Sec Hook:   {:.1}% (target: >= 70%)", hook * 100.0);
            }
            if let Some(comp) = metrics.completion_rate() {
                println!("Completion:   {:.1}% (target: >= 35%)", comp * 100.0);
            }
            println!("\nAudience Retention Curve:");
            println!("{:<8} {:<12} {:<12}", "Time", "Elapsed", "Retention");
            for p in &metrics.retention_curve {
                println!(
                    "{:02}:{:02}    {:>5.1}%       {:>5.1}%",
                    p.elapsed_seconds / 60,
                    p.elapsed_seconds % 60,
                    p.elapsed_ratio * 100.0,
                    p.retention_ratio * 100.0
                );
            }
        }
    }
    Ok(())
}

async fn cmd_policy(store: &Store, action: PolicyAction) -> anyhow::Result<()> {
    let guardian = PolicyGuardian::new(store.data_dir().join("policy"))?;
    match action {
        PolicyAction::Status => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "latest_report": guardian.latest_report()?,
                "channel_browser_review": guardian.channel_review()?
            }))?
        ),
        PolicyAction::Refresh => {
            let refresh = guardian.refresh_sources().await?;
            println!("{}", serde_json::to_string_pretty(&refresh)?);
            if refresh.check.status != CheckStatus::Pass {
                anyhow::bail!(refresh.check.detail);
            }
        }
        PolicyAction::RecordChannelReview {
            channel_id,
            warning_visible,
            strike_visible,
            training_completed,
            warning_expires_at,
            restricted_policy,
        } => {
            let warning_expires_at = warning_expires_at
                .map(|value| value.parse::<chrono::DateTime<Utc>>())
                .transpose()?;
            let review = ChannelBrowserReview {
                source_url: format!("https://studio.youtube.com/channel/{channel_id}"),
                channel_id,
                reviewed_at: Utc::now(),
                warning_visible,
                strike_visible,
                training_completed,
                warning_expires_at,
                restricted_policy,
            };
            guardian.record_channel_review(&review)?;
            println!("{}", serde_json::to_string_pretty(&review)?);
        }
        PolicyAction::AcceptCurrent => {
            let accepted = guardian.accept_pending()?;
            println!(
                "Accepted {} reviewed YouTube policy pages from {}",
                accepted.pages.len(),
                accepted.checked_at
            );
        }
    }
    Ok(())
}

async fn cmd_payroll(store: &Store, action: PayrollAction) -> anyhow::Result<()> {
    let ledger_path = store.data_dir().join("payroll").join("ledger.json");
    let mut ledger = SimulatedPayrollLedger::load(&ledger_path)?;
    match action {
        PayrollAction::Import {
            platform,
            period,
            revenue_usd,
            source_file,
        } => {
            if !revenue_usd.is_finite() || revenue_usd < 0.0 {
                anyhow::bail!("Revenue must be a finite non-negative USD amount");
            }
            let registry_path = std::env::var_os("ATLAS_AGENT_REGISTRY")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".opencode/config/agents.json"));
            let registry: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&registry_path)?)?;
            let agents = registry["roles"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Agent registry has no roles array"))?
                .iter()
                .filter_map(|entry| entry["role"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            let revenue_micros = (revenue_usd * 1_000_000.0).round() as u64;
            let run = ledger
                .record_revenue(&platform, &period, revenue_micros, &source_file, &agents)?
                .clone();
            ledger.save(&ledger_path)?;
            println!(
                "Recorded evidence-backed real revenue ${:.6} and paid {} agents {:.6} simulated credits total.",
                run.evidence.real_revenue_usd_micros as f64 / 1_000_000.0,
                run.payouts.len(),
                run.payouts.values().sum::<u64>() as f64 / 1_000_000.0
            );
        }
        PayrollAction::Status => {
            println!("Simulation-only payroll; no real money is transferred.");
            println!("Revenue periods recorded: {}", ledger.runs.len());
            for (agent, balance) in &ledger.agent_balances_micros {
                println!("{agent}: {:.6} credits", *balance as f64 / 1_000_000.0);
            }
        }
    }
    Ok(())
}

fn cleanup_uploaded_media<'a>(paths: impl IntoIterator<Item = &'a Path>) {
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => println!("Removed uploaded local media: {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "Uploaded successfully, but local cleanup failed for {}: {}",
                path.display(),
                error
            ),
        }
    }
}

async fn optimize_thumbnail(path: &Path, _width: u32, _height: u32) {
    if let Some(photocraft) = bundled_cli("photocraft") {
        let optimized = path.with_extension("photocraft.png");
        let status = tokio::process::Command::new(photocraft)
            .args(["convert"])
            .arg(path)
            .arg(&optimized)
            .args(["--format", "png"])
            .status()
            .await;
        if matches!(status, Ok(status) if status.success()) && optimized.is_file() {
            if let Err(error) = std::fs::copy(&optimized, path) {
                eprintln!("PhotoCraft thumbnail copy failed: {error}");
            }
            if let Err(error) = std::fs::remove_file(&optimized) {
                eprintln!("PhotoCraft temporary thumbnail cleanup failed: {error}");
            }
        }
    }
}

async fn add_vector_thumbnail_title(path: &Path, title: &str, width: u32, height: u32) {
    let Some(vectorcraft) = bundled_cli("vectorcraft") else {
        return;
    };
    let escaped = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let svg_path = path.with_extension("title.svg");
    let overlay_path = path.with_extension("title.png");
    let composed_path = path.with_extension("composed.png");
    let font_size = (width as f32 * 0.038).clamp(38.0, 76.0);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\"><rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" fill=\"none\"/><rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{}\" fill=\"#101522\" fill-opacity=\"0.88\"/><text x=\"{}\" y=\"{}\" fill=\"#fff4c2\" font-family=\"Arial\" font-size=\"{font_size}\" font-weight=\"700\">{escaped}</text></svg>",
        (height as f32 * 0.16) as u32,
        (width as f32 * 0.04) as u32,
        (height as f32 * 0.105) as u32,
    );
    if std::fs::write(&svg_path, svg).is_err() {
        return;
    }
    let converted = tokio::process::Command::new(vectorcraft)
        .args(["convert"])
        .arg(&svg_path)
        .arg(&overlay_path)
        .status()
        .await;
    if !matches!(converted, Ok(status) if status.success()) || !overlay_path.is_file() {
        let _ = std::fs::remove_file(&svg_path);
        return;
    }
    let composed = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(path)
        .arg("-i")
        .arg(&overlay_path)
        .args(["-filter_complex", "overlay=0:0", "-frames:v", "1"])
        .arg(&composed_path)
        .status()
        .await;
    if matches!(composed, Ok(status) if status.success()) && composed_path.is_file() {
        if let Err(error) = std::fs::copy(&composed_path, path) {
            eprintln!("VectorCraft title overlay copy failed: {error}");
        }
    }
    for temporary in [&svg_path, &overlay_path, &composed_path] {
        if let Err(error) = std::fs::remove_file(temporary)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Creative CLI temporary cleanup failed: {error}");
        }
    }
}

fn bundled_cli(name: &str) -> Option<PathBuf> {
    let direct = which::which(format!("{name}-cli")).ok();
    direct.or_else(|| {
        let tools = PathBuf::from("tools").join(name);
        std::fs::read_dir(tools)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path().join(format!("{name}-cli.exe")))
            .find(|candidate| candidate.is_file())
    })
}

async fn import_broll(
    video_id: &str,
    format: VideoFormat,
    data_dir: &Path,
    queries: &[String],
) -> Vec<ImportedMedia> {
    let Some(base_dirs) = BaseDirs::new() else {
        eprintln!("B-roll skipped: unable to locate the user profile");
        return Vec::new();
    };
    let key_path = std::env::var_os("ATLAS_MEDIA_KEYS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            base_dirs
                .home_dir()
                .join("Desktop")
                .join("app wallpaper switcher")
                .join("local.properties")
        });
    let client = match StockMediaClient::from_properties(&key_path) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("B-roll skipped: {error}");
            return Vec::new();
        }
    };
    let orientation = match format {
        VideoFormat::Long => MediaOrientation::Landscape,
        VideoFormat::Short => MediaOrientation::Portrait,
    };
    let output_dir = data_dir
        .join("media")
        .join(video_id)
        .join(format.profile().slug);
    let image_query = queries
        .first()
        .map(String::as_str)
        .unwrap_or("educational concept close up");
    let video_query = queries.get(1).map(String::as_str).unwrap_or(image_query);
    let requests = [
        (image_query, MediaKind::Image),
        (video_query, MediaKind::Video),
    ];
    let mut imported = Vec::new();
    for (query, kind) in requests {
        match client.import(query, kind, orientation, &output_dir).await {
            Ok(media) => imported.push(media),
            Err(error) => eprintln!("B-roll source unavailable for {query}: {error}"),
        }
    }
    if !imported.is_empty()
        && let Ok(json) = serde_json::to_vec_pretty(&imported)
    {
        let _ = std::fs::write(output_dir.join("attribution.json"), json);
    }
    imported
}

#[derive(Clone)]
struct ProductionBeat {
    title: &'static str,
    narration: &'static str,
    scene: &'static str,
    emotion: MascotEmotion,
    gesture: MascotGesture,
}

enum VideoBeat {
    Curated(ProductionBeat),
    Directed(StoryboardBeat),
}

struct PlannedVideo {
    beats: Vec<VideoBeat>,
    generated_plan: Option<StoryboardPlan>,
}

impl VideoBeat {
    fn title(&self) -> &str {
        match self {
            Self::Curated(beat) => beat.title,
            Self::Directed(beat) => &beat.title,
        }
    }

    fn narration(&self) -> &str {
        match self {
            Self::Curated(beat) => beat.narration,
            Self::Directed(beat) => &beat.narration,
        }
    }

    fn scene(&self) -> &str {
        match self {
            Self::Curated(_) => "generic_scene",
            Self::Directed(beat) => match beat.scene_pattern {
                ScenePattern::Hook => "generic_hook",
                ScenePattern::Parts => "generic_parts",
                ScenePattern::Flow => "generic_flow",
                ScenePattern::SplitPath => "generic_split_path",
                ScenePattern::Comparison => "generic_comparison",
                ScenePattern::Scale => "generic_scale",
                ScenePattern::Timeline => "generic_timeline",
                ScenePattern::CauseEffect => "generic_cause_effect",
                ScenePattern::Risk => "generic_risk",
                ScenePattern::Cycle => "generic_cycle",
            },
        }
    }

    fn emotion(&self) -> MascotEmotion {
        match self {
            Self::Curated(beat) => beat.emotion.clone(),
            Self::Directed(beat) => match beat.character.emotion {
                DirectedEmotion::Curious => MascotEmotion::Curious,
                DirectedEmotion::Excited => MascotEmotion::Excited,
                DirectedEmotion::Explaining => MascotEmotion::Explaining,
                DirectedEmotion::Concerned => MascotEmotion::Concerned,
                DirectedEmotion::Confident => MascotEmotion::Confident,
            },
        }
    }

    fn gesture(&self) -> MascotGesture {
        match self {
            Self::Curated(beat) => beat.gesture.clone(),
            Self::Directed(beat) => match beat.character.gesture {
                DirectedGesture::PointLeft => MascotGesture::PointLeft,
                DirectedGesture::PointRight => MascotGesture::PointRight,
                DirectedGesture::Present => MascotGesture::Present,
                DirectedGesture::Think => MascotGesture::Think,
                DirectedGesture::Warn => MascotGesture::Warn,
                DirectedGesture::Celebrate => MascotGesture::Celebrate,
            },
        }
    }

    fn morph(&self) -> Option<MascotMorph> {
        match self {
            Self::Curated(beat) => morph_for_scene(beat.scene),
            Self::Directed(beat) => beat.character.morph.as_ref().map(|morph| MascotMorph {
                label: morph.label.clone(),
                silhouette: morph.silhouette.clone(),
            }),
        }
    }

    fn broll_query(&self) -> &str {
        match self {
            Self::Curated(_) => "educational science technology close up",
            Self::Directed(beat) => &beat.broll_query,
        }
    }

    fn apply_visuals(&self, spec: &mut atlas_blender::ShotSpec, portrait: bool) {
        if let Self::Curated(beat) = self {
            let (pattern, focal_terms) = curated_visual_plan(beat.scene);
            let directed = StoryboardBeat {
                title: beat.title.into(),
                narration: beat.narration.into(),
                scene_pattern: pattern,
                focal_terms,
                broll_query: self.broll_query().into(),
                character: atlas_storyboard::CharacterDirection {
                    emotion: DirectedEmotion::Explaining,
                    gesture: DirectedGesture::Present,
                    acting_goal: "Make the mechanism visible".into(),
                    morph: None,
                },
            };
            Self::Directed(directed).apply_visuals(spec, portrait);
            return;
        }
        if let Self::Directed(beat) = self {
            spec.visual_elements.truncate(1);
            let count = beat.focal_terms.len().max(1);
            for (index, term) in beat.focal_terms.iter().enumerate() {
                let progress = if count == 1 {
                    0.5
                } else {
                    index as f32 / (count - 1) as f32
                };
                let x = if portrait { 0.0 } else { -2.4 + progress * 4.8 };
                let y = if portrait {
                    1.4 - index as f32 * 0.75
                } else {
                    0.2
                };
                spec.visual_elements.push(VisualElement {
                    element_id: format!("focus_{index}"),
                    element_type: VisualElementType::Shape,
                    reveal_frame: 20 + index as u32 * 14,
                    hold_frames: spec.duration_frames.saturating_sub(20),
                    properties: serde_json::json!({}),
                    transform: TransformSpec {
                        position: [x, y, 0.1],
                        rotation: [0.0, 0.0, 0.0],
                        scale: if portrait {
                            [1.45, 0.28, 0.05]
                        } else {
                            [0.95, 0.52, 0.05]
                        },
                    },
                });
                spec.visual_elements.push(VisualElement {
                    element_id: format!("label_{index}"),
                    element_type: VisualElementType::Label,
                    reveal_frame: 26 + index as u32 * 14,
                    hold_frames: spec.duration_frames.saturating_sub(26),
                    properties: serde_json::json!({"text": term, "font_size": 18.0}),
                    transform: TransformSpec {
                        position: [x, y, 0.25],
                        rotation: [0.0, 0.0, 0.0],
                        scale: [1.0, 1.0, 1.0],
                    },
                });
                if index > 0 && !matches!(beat.scene_pattern, ScenePattern::Parts) {
                    let previous_progress = if count == 1 {
                        0.5
                    } else {
                        (index - 1) as f32 / (count - 1) as f32
                    };
                    let previous_x = if portrait {
                        0.0
                    } else {
                        -2.4 + previous_progress * 4.8
                    };
                    let previous_y = if portrait {
                        1.4 - (index - 1) as f32 * 0.75
                    } else {
                        0.2
                    };
                    spec.visual_elements.push(VisualElement {
                        element_id: format!("connection_{index}"),
                        element_type: VisualElementType::Arrow,
                        reveal_frame: 32 + index as u32 * 14,
                        hold_frames: spec.duration_frames.saturating_sub(32),
                        properties: serde_json::json!({
                            "start": [previous_x, previous_y, 0.2],
                            "end": [x, y, 0.2]
                        }),
                        transform: TransformSpec {
                            position: [0.0, 0.0, 0.0],
                            rotation: [0.0, 0.0, 0.0],
                            scale: [1.0, 1.0, 1.0],
                        },
                    });
                }
            }
        }
    }
}

fn curated_visual_plan(scene: &str) -> (ScenePattern, Vec<String>) {
    match scene {
        "battery_hook" => (
            ScenePattern::Hook,
            vec!["chemical imbalance".into(), "stored energy".into()],
        ),
        "battery_anatomy" => (
            ScenePattern::Parts,
            vec!["anode".into(), "separator".into(), "cathode".into()],
        ),
        "battery_electrolyte" => (
            ScenePattern::SplitPath,
            vec!["ions pass".into(), "electrons stop".into()],
        ),
        "battery_charging" => (
            ScenePattern::Flow,
            vec!["charger".into(), "ions".into(), "anode".into()],
        ),
        "battery_electrons" => (
            ScenePattern::SplitPath,
            vec!["electrolyte".into(), "outside wire".into()],
        ),
        "battery_discharging" => (
            ScenePattern::Flow,
            vec!["anode".into(), "device".into(), "cathode".into()],
        ),
        "battery_voltage" => (
            ScenePattern::Scale,
            vec![
                "high energy".into(),
                "voltage drop".into(),
                "useful work".into(),
            ],
        ),
        "battery_aging" => (
            ScenePattern::CauseEffect,
            vec![
                "heat".into(),
                "side reactions".into(),
                "capacity loss".into(),
            ],
        ),
        "battery_safety" => (
            ScenePattern::Risk,
            vec!["separator".into(), "short circuit".into(), "heat".into()],
        ),
        _ => (
            ScenePattern::Cycle,
            vec!["charge".into(), "store".into(), "discharge".into()],
        ),
    }
}

async fn planned_video(
    topic: &str,
    format: VideoFormat,
    long_plan: Option<&StoryboardPlan>,
) -> anyhow::Result<PlannedVideo> {
    let expected = format.profile().shot_count as usize;
    if matches!(format, VideoFormat::Short) {
        let long_plan = long_plan.ok_or_else(|| {
            anyhow::anyhow!("Short production requires the approved long-form storyboard first")
        })?;
        let director = DirectorClient::from_env().ok_or_else(|| {
            anyhow::anyhow!("Short adaptation requires NVIDIA_API_KEY or ATLAS_DIRECTOR_API_KEY")
        })?;
        let plan = director.create_short_plan(long_plan, expected).await?;
        let beats = plan
            .beats
            .iter()
            .cloned()
            .map(VideoBeat::Directed)
            .collect();
        return Ok(PlannedVideo {
            beats,
            generated_plan: Some(plan),
        });
    }
    if let Some(director) = DirectorClient::from_env() {
        match director
            .create_plan(topic, format.profile().slug, expected)
            .await
        {
            Ok(plan) => {
                let beats = plan
                    .beats
                    .iter()
                    .cloned()
                    .map(VideoBeat::Directed)
                    .collect();
                return Ok(PlannedVideo {
                    beats,
                    generated_plan: Some(plan),
                });
            }
            Err(error) => eprintln!("Creative director fallback activated: {error}"),
        }
    }
    if topic.to_ascii_lowercase().contains("lithium") {
        return Ok(PlannedVideo {
            beats: production_beats(topic, format)
                .into_iter()
                .map(VideoBeat::Curated)
                .collect(),
            generated_plan: None,
        });
    }
    anyhow::bail!(
        "A validated creative-director storyboard is required for new topics; configure NVIDIA_API_KEY or ATLAS_DIRECTOR_API_KEY"
    )
}

fn production_beats(topic: &str, format: VideoFormat) -> Vec<ProductionBeat> {
    if !topic.to_ascii_lowercase().contains("lithium") {
        return generic_beats(format);
    }
    match format {
        VideoFormat::Short => vec![
            ProductionBeat {
                title: "YOUR BATTERY IS NOT A TANK",
                narration: "Your battery does not store electricity like a bottle; it stores a chemical imbalance, and Luma will show you the trick.",
                scene: "battery_hook",
                emotion: MascotEmotion::Curious,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "THREE LAYERS CONTROL THE ACTION",
                narration: "Two electrodes hold lithium, while an electrolyte carries ions and a separator prevents a dangerous short circuit.",
                scene: "battery_anatomy",
                emotion: MascotEmotion::Explaining,
                gesture: MascotGesture::PointLeft,
            },
            ProductionBeat {
                title: "CHARGING SEPARATES THE PLAYERS",
                narration: "A charger pushes lithium ions into the graphite anode while electrons travel there through the outside wire.",
                scene: "battery_charging",
                emotion: MascotEmotion::Excited,
                gesture: MascotGesture::PointRight,
            },
            ProductionBeat {
                title: "DISCHARGING MAKES CURRENT",
                narration: "When you unplug, ions return through the electrolyte and electrons power your device on their separate route.",
                scene: "battery_discharging",
                emotion: MascotEmotion::Confident,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "HEAT STEALS CAPACITY",
                narration: "Heat and side reactions build barriers that trap active lithium, so an older battery cannot hold the same charge.",
                scene: "battery_aging",
                emotion: MascotEmotion::Concerned,
                gesture: MascotGesture::Warn,
            },
        ],
        VideoFormat::Long => vec![
            ProductionBeat {
                title: "A BATTERY STORES A CHEMICAL IMBALANCE",
                narration: "A lithium ion battery is not a tank of electricity; it stores energy by separating charge into a chemical state that wants to relax.",
                scene: "battery_hook",
                emotion: MascotEmotion::Curious,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "THE CELL HAS FOUR CRITICAL PARTS",
                narration: "The graphite anode and metal oxide cathode host lithium, the electrolyte carries ions, and the separator keeps the electrodes from touching.",
                scene: "battery_anatomy",
                emotion: MascotEmotion::Explaining,
                gesture: MascotGesture::PointLeft,
            },
            ProductionBeat {
                title: "IONS MAY CROSS; ELECTRONS MAY NOT",
                narration: "The electrolyte conducts lithium ions but not useful electron current, while the porous separator blocks contact without blocking those moving ions.",
                scene: "battery_electrolyte",
                emotion: MascotEmotion::Explaining,
                gesture: MascotGesture::PointRight,
            },
            ProductionBeat {
                title: "CHARGING DRIVES LITHIUM UPHILL",
                narration: "A charger supplies energy that pulls lithium from the cathode and pushes it through the electrolyte into spaces between graphite layers.",
                scene: "battery_charging",
                emotion: MascotEmotion::Excited,
                gesture: MascotGesture::PointLeft,
            },
            ProductionBeat {
                title: "ELECTRONS TAKE THE OUTSIDE ROAD",
                narration: "At the same time electrons move through the external wire because they cannot cross the electrolyte, balancing the arriving lithium inside the anode.",
                scene: "battery_electrons",
                emotion: MascotEmotion::Explaining,
                gesture: MascotGesture::PointRight,
            },
            ProductionBeat {
                title: "DISCHARGING POWERS THE DEVICE",
                narration: "After unplugging, lithium ions move back toward the cathode and electrons travel through your device, where their current performs useful work.",
                scene: "battery_discharging",
                emotion: MascotEmotion::Confident,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "VOLTAGE MEASURES THE ENERGY DROP",
                narration: "Voltage describes how much energy each unit of charge can release while moving between the two electrode environments.",
                scene: "battery_voltage",
                emotion: MascotEmotion::Curious,
                gesture: MascotGesture::Think,
            },
            ProductionBeat {
                title: "SIDE REACTIONS AGE THE CELL",
                narration: "Every cycle loses a little active lithium and thickens a boundary layer, while heat and high charge levels accelerate those unwanted reactions.",
                scene: "battery_aging",
                emotion: MascotEmotion::Concerned,
                gesture: MascotGesture::Warn,
            },
            ProductionBeat {
                title: "THE SEPARATOR IS A SAFETY BARRIER",
                narration: "If damage lets the electrodes touch, current can bypass the device, generate intense heat, and trigger more reactions faster than heat escapes.",
                scene: "battery_safety",
                emotion: MascotEmotion::Concerned,
                gesture: MascotGesture::Warn,
            },
            ProductionBeat {
                title: "SEPARATE TO CHARGE; REUNITE TO WORK",
                narration: "The complete mental model is simple: charging separates ions and electrons into a higher energy arrangement, while discharging lets separate paths reunite them through useful work.",
                scene: "battery_summary",
                emotion: MascotEmotion::Confident,
                gesture: MascotGesture::Celebrate,
            },
        ],
    }
}

fn generic_beats(format: VideoFormat) -> Vec<ProductionBeat> {
    let count = format.profile().shot_count as usize;
    let scenes = [
        "battery_hook",
        "battery_anatomy",
        "battery_electrolyte",
        "battery_charging",
        "battery_electrons",
        "battery_discharging",
        "battery_voltage",
        "battery_aging",
        "battery_safety",
        "battery_summary",
    ];
    scenes[..count]
        .iter()
        .enumerate()
        .map(|(index, scene)| ProductionBeat {
            title: "FOLLOW THE CAUSE AND EFFECT",
            narration: "Luma identifies each important part, follows what changes, and connects that mechanism to the result you can observe.",
            scene,
            emotion: MascotEmotion::Explaining,
            gesture: if index % 2 == 0 {
                MascotGesture::PointLeft
            } else {
                MascotGesture::PointRight
            },
        })
        .collect()
}

fn lip_sync_cues(audio: &AudioSegment) -> Vec<LipSyncCue> {
    let mut cues = vec![LipSyncCue {
        frame: 1,
        viseme: Viseme::Rest,
    }];
    for word in &audio.word_timestamps {
        let sounds = word
            .word
            .chars()
            .filter(|value| value.is_ascii_alphabetic())
            .collect::<Vec<_>>();
        let sound_count = sounds.len().clamp(1, 5);
        let duration = (word.end_seconds - word.start_seconds).max(0.03);
        for (index, sound) in sounds.into_iter().take(sound_count).enumerate() {
            let progress = index as f32 / sound_count as f32;
            let frame = ((word.start_seconds + duration * progress) * 30.0).round() as u32 + 1;
            let viseme = match sound.to_ascii_lowercase() {
                'm' | 'b' | 'p' => Viseme::Closed,
                'a' | 'h' => Viseme::Open,
                'e' | 'i' | 'y' => Viseme::Wide,
                'o' | 'u' | 'w' => Viseme::Round,
                _ => Viseme::Open,
            };
            cues.push(LipSyncCue { frame, viseme });
        }
    }
    cues.push(LipSyncCue {
        frame: (audio.duration_seconds * 30.0).ceil() as u32,
        viseme: Viseme::Rest,
    });
    cues
}

fn morph_for_scene(scene: &str) -> Option<MascotMorph> {
    let (label, silhouette) = match scene {
        "battery_hook" | "battery_anatomy" => (
            "battery",
            vec![
                [-0.85, -0.95],
                [-0.85, 0.75],
                [-0.3, 0.75],
                [-0.3, 0.95],
                [0.3, 0.95],
                [0.3, 0.75],
                [0.85, 0.75],
                [0.85, -0.95],
                [0.4, -0.95],
                [0.0, -0.95],
                [-0.4, -0.95],
            ],
        ),
        "battery_electrolyte" | "battery_charging" => (
            "ion",
            vec![
                [-0.65, -0.75],
                [-0.92, -0.35],
                [-1.0, 0.15],
                [-0.82, 0.62],
                [-0.42, 0.9],
                [0.0, 1.0],
                [0.42, 0.9],
                [0.82, 0.62],
                [1.0, 0.15],
                [0.92, -0.35],
                [0.65, -0.75],
            ],
        ),
        "battery_electrons" | "battery_discharging" | "battery_voltage" => (
            "energy bolt",
            vec![
                [-0.25, -1.0],
                [0.05, -0.25],
                [-0.4, -0.25],
                [0.25, 1.0],
                [0.05, 0.15],
                [0.5, 0.15],
                [-0.1, -1.0],
                [-0.25, -1.0],
                [-0.25, -1.0],
                [-0.25, -1.0],
                [-0.25, -1.0],
            ],
        ),
        "battery_aging" | "battery_safety" => (
            "warning sign",
            vec![
                [-1.0, -1.0],
                [-0.8, -0.6],
                [-0.6, -0.2],
                [-0.4, 0.2],
                [-0.2, 0.6],
                [0.0, 1.0],
                [0.2, 0.6],
                [0.4, 0.2],
                [0.6, -0.2],
                [0.8, -0.6],
                [1.0, -1.0],
            ],
        ),
        _ => return None,
    };
    Some(MascotMorph {
        label: label.into(),
        silhouette,
    })
}

fn write_captions(
    path: &Path,
    segments: &[NarrationSegment],
    narration: &NarrationTrack,
) -> anyhow::Result<()> {
    if segments.len() != narration.segments.len() {
        anyhow::bail!(
            "Caption segment count {} does not match narration segment count {}",
            segments.len(),
            narration.segments.len()
        );
    }

    let mut output = String::new();
    let mut start = 0.0_f32;
    for (index, (segment, audio)) in segments.iter().zip(narration.segments.iter()).enumerate() {
        let end = start + audio.duration_seconds;
        output.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            srt_timestamp(start),
            srt_timestamp(end),
            segment.text
        ));
        start = end;
    }
    std::fs::write(path, output)?;
    Ok(())
}

fn srt_timestamp(seconds: f32) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms / 60_000) % 60;
    let secs = (total_ms / 1_000) % 60;
    let millis = total_ms % 1_000;
    format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}")
}

async fn cmd_daemon(store: &Store, action: DaemonAction) -> anyhow::Result<()> {
    let daemon_dir = store.data_dir().join("daemon");
    let pause_path = daemon_dir.join("paused");
    match action {
        DaemonAction::Start {
            once,
            poll_seconds,
            privacy,
            tiktok_user,
        } => {
            std::fs::create_dir_all(&daemon_dir)?;
            if pause_path.is_file() {
                std::fs::remove_file(&pause_path)?;
            }
            println!("ATLAS continuous publisher started for YouTube and TikTok.");
            loop {
                if pause_path.is_file() {
                    println!("ATLAS continuous publisher paused.");
                    break;
                }
                if let Err(error) = run_daemon_cycle(store, privacy, &tiktok_user).await {
                    eprintln!("Daemon cycle failed safely: {error}");
                }
                if once {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(poll_seconds.max(60))).await;
            }
        }
        DaemonAction::Pause => {
            std::fs::create_dir_all(&daemon_dir)?;
            std::fs::write(&pause_path, Utc::now().to_rfc3339())?;
            println!("ATLAS continuous publisher will stop after its current safe operation.");
        }
    }
    Ok(())
}

async fn run_daemon_cycle(
    store: &Store,
    privacy: PublishPrivacy,
    tiktok_user: &str,
) -> anyhow::Result<()> {
    let ledger_path = store.data_dir().join("publish").join("ledger.json");
    let scheduler_path = store
        .data_dir()
        .join("publish")
        .join("scheduler_state.json");
    let ledger = PublishLedger::load(&ledger_path)?;
    let mut scheduler = PublishSchedulerState::load(&scheduler_path)?;
    let now = Utc::now();
    let mut projects = store.list_projects(100)?;
    let active_count = projects
        .iter()
        .filter(|project| {
            matches!(project.publish.mode, PublishMode::Autonomous)
                && !daemon_project_complete(project)
        })
        .count();

    for project in projects
        .iter()
        .filter(|project| matches!(project.publish.mode, PublishMode::Autonomous))
    {
        for format in [VideoFormat::Long, VideoFormat::Short] {
            let profile = format.profile();
            let youtube_pending = !project.publish.youtube_video_ids.contains_key(profile.slug);
            let tiktok_pending = !project.publish.tiktok_post_ids.contains_key(profile.slug);
            if !youtube_pending && !tiktok_pending {
                continue;
            }
            let published_format = match format {
                VideoFormat::Long => PublishedFormat::Long,
                VideoFormat::Short => PublishedFormat::Short,
            };
            let youtube_ready = youtube_pending
                && scheduler.can_attempt(PublishService::YouTube, now)
                && matches!(
                    ledger.decide(
                        &PublishCadencePolicy::default(),
                        published_format,
                        &format!("pending_{}_{}", project.video_id, profile.slug),
                        now,
                    ),
                    PublishDecision::Allow
                );
            let tiktok_ready = tiktok_pending
                && scheduler.can_attempt(PublishService::TikTok, now)
                && matches!(
                    ledger.decide_tiktok(
                        &PublishCadencePolicy::default(),
                        &format!("pending_{}_{}", project.video_id, profile.slug),
                        now,
                    ),
                    PublishDecision::Allow
                );
            if !youtube_ready && !tiktok_ready {
                continue;
            }

            let video_path = store
                .data_dir()
                .join("exports")
                .join(&project.video_id)
                .join(profile.slug)
                .join("final.mp4");
            if !video_path.is_file() {
                cmd_produce(
                    store,
                    ProduceOptions {
                        topic_id: &project.video_id,
                        reuse_rendered: true,
                        format,
                        publish: false,
                        privacy,
                        platform: PublishPlatformCli::Both,
                        tiktok_user,
                        tiktok_draft: false,
                    },
                )
                .await?;
            }

            if youtube_ready {
                run_daemon_publish(
                    store,
                    PublishOptions {
                        topic_id: &project.video_id,
                        format,
                        privacy,
                        platform: PublishPlatformCli::YouTube,
                        tiktok_user,
                        tiktok_draft: false,
                        cleanup_after_upload: false,
                    },
                    PublishService::YouTube,
                    &mut scheduler,
                    &scheduler_path,
                )
                .await;
            }
            if tiktok_ready {
                run_daemon_publish(
                    store,
                    PublishOptions {
                        topic_id: &project.video_id,
                        format,
                        privacy,
                        platform: PublishPlatformCli::TikTok,
                        tiktok_user,
                        tiktok_draft: false,
                        cleanup_after_upload: false,
                    },
                    PublishService::TikTok,
                    &mut scheduler,
                    &scheduler_path,
                )
                .await;
            }
            cleanup_completed_daemon_package(store, &project.video_id, format)?;
            return Ok(());
        }
    }

    if active_count < 6 {
        let project = create_autonomous_project(store, &projects).await?;
        projects.push(project.clone());
        println!(
            "Queued autonomous topic {}: {}",
            project.video_id, project.topic.title
        );
    } else {
        let youtube_retry = scheduler.next_attempt_at(PublishService::YouTube);
        let tiktok_retry = scheduler.next_attempt_at(PublishService::TikTok);
        println!(
            "Publisher is safely waiting with {active_count} queued projects; YouTube retry {youtube_retry:?}, TikTok retry {tiktok_retry:?}."
        );
    }
    Ok(())
}

async fn run_daemon_publish(
    store: &Store,
    options: PublishOptions<'_>,
    service: PublishService,
    scheduler: &mut PublishSchedulerState,
    scheduler_path: &Path,
) {
    match cmd_publish(store, options).await {
        Ok(()) => {
            scheduler.record_success(service);
            if let Err(error) = scheduler.save(scheduler_path) {
                eprintln!("Unable to save publisher scheduler state: {error}");
            }
        }
        Err(error) => {
            let retry_at = scheduler.record_failure(service, &error.to_string(), Utc::now());
            if let Err(save_error) = scheduler.save(scheduler_path) {
                eprintln!("Unable to save publisher scheduler state: {save_error}");
            }
            eprintln!("{service:?} publish deferred after failure until {retry_at}: {error}");
        }
    }
}

async fn create_autonomous_project(store: &Store, existing: &[Project]) -> anyhow::Result<Project> {
    let director = DirectorClient::from_env().ok_or_else(|| {
        anyhow::anyhow!(
            "Autonomous topic generation requires NVIDIA_API_KEY or ATLAS_DIRECTOR_API_KEY"
        )
    })?;
    let existing_titles = existing
        .iter()
        .map(|project| project.topic.title.clone())
        .collect::<Vec<_>>();
    let idea = director.create_topic(&existing_titles).await?;
    let video_id = format!(
        "vid_{}_{}",
        Utc::now().format("%Y%m%d"),
        &Uuid::new_v4().as_simple().to_string()[..8]
    );
    let project = Project {
        video_id,
        topic: Topic {
            title: idea.title,
            trend_sources: vec![TrendSource {
                source: "autonomous_editorial_director".into(),
                url: None,
                observed_at: Utc::now(),
            }],
            observed_at: Some(Utc::now()),
            region: Some("US".into()),
        },
        state: ProjectState::IdeaDiscovered,
        channel_profile: "primary".into(),
        visual_thesis: Some(idea.rationale),
        script_artifact_id: None,
        research_artifact_id: None,
        storyboard_artifact_id: None,
        publish: PublishPlan {
            mode: PublishMode::Autonomous,
            contains_synthetic_media: None,
            credits_present: false,
            youtube_video_id: None,
            youtube_video_ids: Default::default(),
            tiktok_post_id: None,
            tiktok_post_ids: Default::default(),
            tiktok_request_id: None,
        },
    };
    store.insert_project(&project)?;
    Ok(project)
}

fn daemon_project_complete(project: &Project) -> bool {
    ["long", "short"].iter().all(|format| {
        project.publish.youtube_video_ids.contains_key(*format)
            && project.publish.tiktok_post_ids.contains_key(*format)
    })
}

fn cleanup_completed_daemon_package(
    store: &Store,
    project_id: &str,
    format: VideoFormat,
) -> anyhow::Result<()> {
    let project = store
        .get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found after publish: {project_id}"))?;
    let profile = format.profile();
    if project.publish.youtube_video_ids.contains_key(profile.slug)
        && project.publish.tiktok_post_ids.contains_key(profile.slug)
    {
        let export_dir = store
            .data_dir()
            .join("exports")
            .join(project_id)
            .join(profile.slug);
        cleanup_uploaded_media([
            export_dir.join("final.mp4").as_path(),
            export_dir.join("thumbnail.png").as_path(),
        ]);
    }
    Ok(())
}

async fn cmd_analytics(store: &Store, action: AnalyticsAction) -> anyhow::Result<()> {
    match action {
        AnalyticsAction::Auth {
            client_id,
            client_secret,
            redirect_uri,
        } => {
            // Load from credentials file if not provided
            let (client_id, client_secret, redirect_uri) = match (
                client_id,
                client_secret,
                redirect_uri,
            ) {
                (Some(cid), Some(csec), ruri) => (
                    cid,
                    csec,
                    ruri.unwrap_or_else(|| "http://localhost".to_string()),
                ),
                _ => {
                    let creds_file = PathBuf::from("youtube_credentials.json");
                    if creds_file.exists() {
                        let file_creds = load_youtube_credentials(&creds_file).await?;
                        let creds: YouTubeCredentials = file_creds;
                        (creds.client_id, creds.client_secret, creds.redirect_uri)
                    } else {
                        anyhow::bail!(
                            "YouTube credentials not provided and youtube_credentials.json not found"
                        );
                    }
                }
            };

            let creds = YouTubeCredentials {
                client_id,
                client_secret,
                redirect_uri,
            };
            let url = authorization_url(&creds);
            println!("Visit this URL in a browser signed into your YouTube channel:");
            println!("{}", url);
            println!();
            println!("After authorization, run:");
            println!("  atlas analytics auth-complete --code <CODE>");
        }
        AnalyticsAction::AuthComplete {
            client_id,
            client_secret,
            code,
            redirect_uri,
        } => {
            // Load from credentials file if not provided
            let (client_id, client_secret, redirect_uri) = match (
                client_id,
                client_secret,
                redirect_uri,
            ) {
                (Some(cid), Some(csec), ruri) => (
                    cid,
                    csec,
                    ruri.unwrap_or_else(|| "http://localhost".to_string()),
                ),
                _ => {
                    let creds_file = PathBuf::from("youtube_credentials.json");
                    if creds_file.exists() {
                        let file_creds = load_youtube_credentials(&creds_file).await?;
                        let creds: YouTubeCredentials = file_creds;
                        (creds.client_id, creds.client_secret, creds.redirect_uri)
                    } else {
                        anyhow::bail!(
                            "YouTube credentials not provided and youtube_credentials.json not found"
                        );
                    }
                }
            };

            let creds = YouTubeCredentials {
                client_id,
                client_secret,
                redirect_uri,
            };
            let client = YouTubeAnalyticsClient::new(creds);
            let token = client.exchange_code(&code).await?;
            println!(
                "YouTube OAuth complete. Access token expires at {:?}",
                token.expires_at
            );
            println!("Refresh token stored securely.");
        }
        AnalyticsAction::Fetch {
            video_id,
            sample,
            platform,
            format,
        } => {
            let project = store
                .get_project(&video_id)?
                .ok_or_else(|| anyhow::anyhow!("project not found: {}", video_id))?;
            let youtube_id = project
                .publish
                .youtube_video_id
                .clone()
                .unwrap_or_else(|| "sample_video_id".to_string());

            if sample {
                let metrics = if format.eq_ignore_ascii_case("short")
                    || platform.eq_ignore_ascii_case("tiktok")
                {
                    atlas_analytics::save_sample_short_metrics(store, &video_id, &platform)?
                } else {
                    save_sample_metrics(store, &video_id, &youtube_id)?
                };
                println!(
                    "Saved sample metrics for {} on {} ({})",
                    video_id, platform, format
                );
                println!("{}", serde_json::to_string_pretty(&metrics)?);
            } else {
                println!(
                    "Live {} Analytics fetch requires authenticated credentials.",
                    platform
                );
                println!(
                    "Use --sample to store a demo snapshot, or run `atlas analytics auth` first."
                );
            }
        }
        AnalyticsAction::Retention { video_id, platform } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics
                .load_platform_metrics(&video_id, &platform)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no metrics found for {} on {}. Run `atlas analytics fetch {} --sample --platform {}` first.",
                        video_id,
                        platform,
                        video_id,
                        platform
                    )
                })?;
            println!(
                "Retention curve for {} [{} | {}]:",
                video_id, metrics.platform, metrics.format
            );
            println!("{:<8} {:<12} {:<12}", "Time", "Elapsed", "Retention");
            for p in &metrics.retention_curve {
                let mm = p.elapsed_seconds / 60;
                let ss = p.elapsed_seconds % 60;
                println!(
                    "{:02}:{:02}    {:>5.1}%       {:>5.1}%",
                    mm,
                    ss,
                    p.elapsed_ratio * 100.0,
                    p.retention_ratio * 100.0
                );
            }
            if let Some(hook) = metrics.short_hook_retention() {
                println!("\n0-3s Short Hook Rate: {:.1}%", hook * 100.0);
            }
            if let Some(comp) = metrics.completion_rate() {
                println!("Full Video Completion: {:.1}%", comp * 100.0);
            }
        }
        AnalyticsAction::Recommend {
            video_id,
            duration_seconds,
            platform,
        } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics
                .load_platform_metrics(&video_id, &platform)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no metrics found for {} on {}. Run `atlas analytics fetch {} --sample --platform {}` first.",
                        video_id,
                        platform,
                        video_id,
                        platform
                    )
                })?;

            // Use all stored metrics as the channel baseline.
            let all = analytics.list_metrics(100)?;
            let baseline = compute_channel_baseline(&all);
            let performance = compare_to_baseline(&metrics, &baseline);

            let engine = RecommendationEngine::new();
            let recs = engine.rank(engine.analyze(&performance, duration_seconds));

            println!(
                "Optimization recommendations for {} ({}):",
                video_id, platform
            );
            for r in &recs {
                println!("[{:?}] {:?} — {}", r.priority, r.category, r.issue);
                println!("  Evidence: {}", r.evidence);
                println!("  Action:   {}", r.suggested_action);
                if let Some(tc) = &r.affected_timecodes {
                    println!("  Timecodes: {}", tc.join(", "));
                }
                println!();
            }
            if recs.is_empty() {
                println!("No major issues detected. Video is performing at or above baseline.");
            }
        }
        AnalyticsAction::Optimize {
            video_id,
            duration_seconds,
        } => {
            let loop_ = OptimizationLoop::new();
            let ids = loop_.generate_revision_tasks(store, &video_id, duration_seconds)?;
            if ids.is_empty() {
                println!("No optimization tasks generated; no metrics or no issues detected.");
            } else {
                println!(
                    "Generated {} optimization task(s) from analytics:",
                    ids.len()
                );
                for id in ids {
                    println!("  - {}", id);
                }
            }
        }
        AnalyticsAction::Dashboard => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics.list_metrics(50)?;
            let baseline = compute_channel_baseline(&metrics);
            println!("ATLAS STUDIO — Analytics Dashboard");
            println!("==================================");
            println!("Videos tracked: {}", metrics.len());
            println!(
                "Channel baseline — avg view duration: {:.1}s, avg % viewed: {:.1}%, avg CTR: {:.2}%",
                baseline.average_view_duration_seconds,
                baseline.average_percentage_viewed * 100.0,
                baseline.average_ctr * 100.0
            );
            println!();
            println!(
                "{:<24} {:<16} {:<12} {:<12} {:<12}",
                "VIDEO", "YOUTUBE ID", "VIEWS", "AVG DUR", "CTR"
            );
            for m in metrics {
                println!(
                    "{:<24} {:<16} {:<12} {:<12} {:<12}",
                    m.video_id,
                    m.youtube_video_id,
                    m.views.map(|v| v.to_string()).unwrap_or_default(),
                    m.average_view_duration_seconds
                        .map(|d| format!("{:.1}s", d))
                        .unwrap_or_default(),
                    m.impressions_click_through_rate
                        .map(|c| format!("{:.2}%", c * 100.0))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

async fn cmd_memory(action: MemoryAction) -> anyhow::Result<()> {
    let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let engine = MemoryEngine::from_workspace(&base_dir);
    match action {
        MemoryAction::Status { json } => {
            let status = engine.status().await;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("ATLAS STUDIO — Dual-Memory Stack Status");
                println!("=====================================");
                println!("1. Short-Term Memory: Shared Context MCP");
                println!("   Endpoint: {}", status.shared_context.endpoint);
                println!("   Mode:     {:?}", status.shared_context.mode);
                println!(
                    "   Status:   {}",
                    if status.shared_context.connected {
                        "CONNECTED (OK)"
                    } else {
                        "OFFLINE / LOCAL FALLBACK"
                    }
                );
                if let Some(lat) = status.shared_context.latency_ms {
                    println!("   Latency:  {} ms", lat);
                }
                println!(
                    "   Active Tasks:          {}",
                    status.shared_context.active_tasks
                );
                println!(
                    "   Unpromoted Completed:  {}",
                    status.shared_context.completed_unpromoted_tasks
                );
                println!(
                    "   Active Swarm Agents:   {}",
                    status.shared_context.active_agents
                );
                println!();
                println!("2. Long-Term Memory: Graphiti Knowledge Graph");
                println!("   Endpoint: {}", status.graphiti.endpoint);
                println!("   Group ID: {}", status.graphiti.group_id);
                println!(
                    "   Status:   {}",
                    if status.graphiti.connected {
                        "CONNECTED (OK)"
                    } else {
                        "OFFLINE / LOCAL GRAPH FALLBACK"
                    }
                );
                if let Some(lat) = status.graphiti.latency_ms {
                    println!("   Latency:  {} ms", lat);
                }
                println!(
                    "   Total Episodes:        {}",
                    status.graphiti.total_episodes
                );
                println!(
                    "   Indexed Entities:      {}",
                    status.graphiti.total_entities
                );
                println!("   Knowledge Edges:       {}", status.graphiti.total_edges);
                println!();
                println!("Promotion Policy:");
                println!(
                    "   Auto-promote on complete: {}",
                    status.auto_promote_enabled
                );
                println!(
                    "   Ephemeral retention:      {} days",
                    status.prune_retention_days
                );
            }
        }
        MemoryAction::Recall { query, limit, json } => {
            let results = engine.graphiti().search(&query, limit).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                println!("Graphiti Knowledge Graph Recall for: '{}'", query);
                println!("============================================");
                if results.is_empty() {
                    println!("No matching episodes or entities found.");
                } else {
                    for (i, r) in results.iter().enumerate() {
                        println!("[{}] {} (Relevance: {:.2})", i + 1, r.title, r.score);
                        println!("    Created:  {}", r.created_at.to_rfc3339());
                        println!("    Summary:  {}", r.summary);
                        if !r.matched_entities.is_empty() {
                            println!("    Entities: {}", r.matched_entities.join(", "));
                        }
                        println!();
                    }
                }
            }
        }
        MemoryAction::Tasks { status, json } => {
            let filter = status.and_then(|s| match s.to_lowercase().as_str() {
                "pending" => Some(MemoryTaskStatus::Pending),
                "in_progress" => Some(MemoryTaskStatus::InProgress),
                "review" => Some(MemoryTaskStatus::Review),
                "completed" => Some(MemoryTaskStatus::Completed),
                "failed" => Some(MemoryTaskStatus::Failed),
                _ => None,
            });
            let tasks = engine.shared_context().list_tasks(filter).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            } else {
                println!("Shared Context MCP — Working Memory Task Board");
                println!("=============================================");
                if tasks.is_empty() {
                    println!("No active tasks in working memory.");
                } else {
                    println!(
                        "{:<14} {:<24} {:<12} {:<10} {:<8}",
                        "TASK ID", "TITLE", "ASSIGNED", "STATUS", "PROMOTED"
                    );
                    for t in tasks {
                        println!(
                            "{:<14} {:<24} {:<12} {:<10} {:<8}",
                            t.task_id,
                            if t.title.len() > 22 {
                                format!("{}...", &t.title[..22])
                            } else {
                                t.title
                            },
                            t.assigned_to.unwrap_or_else(|| "unassigned".to_string()),
                            format!("{:?}", t.status),
                            if t.promoted_to_graphiti { "YES" } else { "NO" }
                        );
                    }
                }
            }
        }
        MemoryAction::CreateTask {
            title,
            description,
            agent,
        } => {
            let task = engine
                .shared_context()
                .create_task(&title, &description, agent.as_deref())
                .await?;
            println!("Created task in Shared Context: {}", task.task_id);
            println!("  Title:       {}", task.title);
            println!(
                "  Assigned to: {}",
                task.assigned_to.unwrap_or_else(|| "unassigned".to_string())
            );
        }
        MemoryAction::AddNote { task, note } => {
            let ok = engine
                .shared_context()
                .add_context_note(&task, &note)
                .await?;
            if ok {
                println!("Added context note to task {}", task);
            } else {
                eprintln!("Task not found: {}", task);
            }
        }
        MemoryAction::UpdateTask { task, status } => {
            let parsed = match status.to_lowercase().as_str() {
                "pending" => MemoryTaskStatus::Pending,
                "in_progress" => MemoryTaskStatus::InProgress,
                "review" => MemoryTaskStatus::Review,
                "completed" => MemoryTaskStatus::Completed,
                "failed" => MemoryTaskStatus::Failed,
                _ => anyhow::bail!(
                    "Invalid status: {}. Use pending, in_progress, review, completed, or failed.",
                    status
                ),
            };
            let updated = engine
                .shared_context()
                .update_task_status(&task, parsed)
                .await?;
            if let Some(t) = updated {
                println!("Updated task {} status to {:?}", t.task_id, t.status);
            } else {
                eprintln!("Task not found: {}", task);
            }
        }
        MemoryAction::Promote { task, notes, json } => {
            let report = engine
                .promoter()
                .promote_task(&task, notes.as_deref())
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Promoted Task to Graphiti Knowledge Graph");
                println!("=========================================");
                println!("Task ID:         {}", report.task_id);
                println!("Title:           {}", report.task_title);
                println!("Graph Episode:   {}", report.episode_id);
                println!("Artifacts Bound: {}", report.artifact_count);
                println!("Notes Distilled: {}", report.note_count);
                println!("Promoted At:     {}", report.promoted_at.to_rfc3339());
                println!();
                println!("Distilled Knowledge Payload:");
                println!("{}", report.distilled_summary);
            }
        }
        MemoryAction::AutoPromote { json } => {
            let reports = engine.promoter().auto_promote_completed().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&reports)?);
            } else {
                println!(
                    "Auto-promoted {} completed task(s) into Graphiti:",
                    reports.len()
                );
                for r in reports {
                    println!(
                        "  - [{}] {} -> Episode {}",
                        r.task_id, r.task_title, r.episode_id
                    );
                }
            }
        }
        MemoryAction::Preload { topic, agent } => {
            let (task, matches) = engine
                .promoter()
                .bootstrap_topic_context(&topic, agent.as_deref())
                .await?;
            println!("Bootstrapped Shared Context with Graphiti Memory");
            println!("===============================================");
            println!("Created Working Task: {}", task.task_id);
            println!(
                "Assigned Agent:       {}",
                task.assigned_to.unwrap_or_else(|| "none".to_string())
            );
            println!("Recalled Episodes:    {}", matches.len());
            for m in matches {
                println!("  • [Score: {:.2}] {}: {}", m.score, m.title, m.summary);
            }
        }
        MemoryAction::Prune { days } => {
            let count = engine.shared_context().prune_ephemeral(days).await?;
            println!(
                "Pruned {} stale ephemeral task records older than {} days.",
                count, days
            );
        }
        MemoryAction::Daemon { poll_seconds, once } => {
            println!("Starting Dual-Memory Bridge Daemon (Shared Context <-> Graphiti)...");
            println!("Polling every {}s. Press Ctrl+C to stop.", poll_seconds);
            loop {
                let reports = engine.promoter().auto_promote_completed().await?;
                if !reports.is_empty() {
                    for r in reports {
                        println!(
                            "[Auto-Promote] Committed task {} ({}) -> Episode {}",
                            r.task_id, r.task_title, r.episode_id
                        );
                    }
                }
                if once {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(poll_seconds)).await;
            }
        }
    }
    Ok(())
}
