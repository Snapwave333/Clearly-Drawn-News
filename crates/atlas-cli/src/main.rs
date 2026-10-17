use atlas_core::{
    AgentMessage, FeedbackItem, Project, ProjectState, PublishMode, PublishPlan, Topic,
    TrendSource, redact_config,
};
use atlas_doctor::{has_minimal_pipeline, missing_pipeline_tools, probe_all};
use atlas_opencode::OpenCodeClient;
use atlas_store::Store;
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

mod broll;
mod cache;
mod daemon;
mod news;
mod news_health;
mod news_voice;
mod production;

use broll::import_broll;
use cache::cmd_cache;
use daemon::cmd_daemon;
use news::cmd_news;
use production::{
    StageTimer, env_parallelism, generate_vector_diagram, render_shots, validate_format_duration,
};

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
#[command(about = "Clearly Drawn Studios — ATLAS autonomous animation newsroom and production engine")]
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
                shot_count: 35,
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
    /// Freesound.org search, download, and sound effects integration.
    Freesound {
        #[command(subcommand)]
        action: FreesoundAction,
    },
    /// Clearly Drawn News: Autonomous Animation Broadcast Newsroom pipeline.
    News {
        #[command(subcommand)]
        action: NewsAction,
    },
    #[command(about = "Inspect and prune reusable narration, render, B-roll, and vector caches")]
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    #[command(about = "Show disk usage for each cache area")]
    Stats,
    #[command(about = "Delete cache entries untouched for the given number of days")]
    Prune {
        #[arg(long, default_value_t = 30)]
        older_than_days: u64,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum NewsAction {
    /// Ingest and fact-check developing animation news stories from verified industry sources.
    Research,
    /// Build the structured 22-minute 8-segment episode slate.
    Slate,
    /// Transmit the episode slate to Telegram (@youtubepipline2146_bot) and poll for human approval.
    Telegram {
        #[arg(long, env = "TELEGRAM_BOT_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long)]
        chat_id: Option<i64>,
    },
    #[command(about = "Show feed reliability and fact-check pass rates across recent wire runs")]
    Health {
        #[arg(long, default_value_t = 20)]
        runs: usize,
    },
    /// Run copyright pre-render compliance checks and statutory fair use analysis.
    Copyright,
    /// Execute the complete autonomous 15-30 minute newsroom broadcast production pipeline.
    Produce {
        #[arg(long, env = "TELEGRAM_BOT_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long)]
        chat_id: Option<i64>,
        #[arg(long)]
        skip_telegram_wait: bool,
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
enum FreesoundAction {
    /// Search Freesound.org for sound effects.
    Search {
        query: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Download and cache a sound from Freesound.org by ID.
    Download {
        sound_id: u64,
        #[arg(long)]
        name: Option<String>,
    },
    /// Check Freesound API key status and local cache statistics.
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
        Commands::Freesound { action } => cmd_freesound(&store, action).await,
        Commands::News { action } => cmd_news(&store, action).await,
        Commands::Cache { action } => cmd_cache(&store, action),
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
        println!("{} — System Probe", atlas_core::COMPANY_NAME);
        println!("{}", "=".repeat(atlas_core::COMPANY_NAME.len() + 15));
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

    println!("═══ {} Vertical Slice ═══", atlas_core::COMPANY_NAME);
    println!("Project: {} — {}", project.video_id, project.topic.title);
    println!(
        "Format: {} ({}x{}, {} shots)",
        profile.slug, profile.width, profile.height, profile.shot_count
    );
    println!();

    let mut timer = StageTimer::new();
    timer.begin("storyboard");
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
        if long_storyboard_path.is_file() {
            std::fs::read(&long_storyboard_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<StoryboardPlan>(&bytes).ok())
        } else {
            None
        }
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

    timer.begin("narration_and_broll");
    println!("🎙️  Stage 2: Synthesizing narration and importing B-roll in parallel...");
    project.state = ProjectState::AudioBuild;
    store.insert_project(&project)?;
    let data_dir = store.data_dir().to_path_buf();
    let voice_config = VoiceConfig::default();
    let kokoro_pipeline = KokoroPipeline::new("kokoro-tts", voice_config, &data_dir)
        .with_parallelism(env_parallelism("ATLAS_TTS_JOBS"));
    let segments = beats
        .iter()
        .enumerate()
        .map(|(i, beat)| NarrationSegment {
            segment_id: format!("seg_{:03}", i + 1),
            text: beat.narration().to_string(),
            voice: "af_heart".to_string(),
            speed: 1.0,
            pitch: 1.0,
            emphasis_words: Vec::new(),
            pause_after_ms: 0,
            pronunciation_hints: Vec::new(),
        })
        .collect::<Vec<_>>();
    let broll_queries = beats
        .iter()
        .map(|beat| beat.broll_query().to_string())
        .collect::<Vec<_>>();
    let (narration, broll) = tokio::join!(
        kokoro_pipeline.synthesize_track(segments.clone(), render_project_id.clone()),
        import_broll(&project.video_id, format, &data_dir, &broll_queries),
    );
    let narration = narration?;
    let combined_audio = narration
        .combined_file
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Kokoro did not return a combined narration file"))?;
    println!("   ✓ Timed narration: {}", combined_audio.display());
    println!("   ✓ {} attributed media assets available", broll.len());

    timer.begin("shot_planning");
    println!("🎬 Stage 3: Creating lip-synced shot specifications...");
    project.state = ProjectState::Storyboarding;
    store.insert_project(&project)?;

    let vectors_dir = data_dir.join("vectors").join(&project.video_id);
    std::fs::create_dir_all(&vectors_dir)?;
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
        for (term_idx, term) in beat.focal_terms().iter().enumerate() {
            if let Some(svg_path) = generate_vector_diagram(term, term_idx, &vectors_dir) {
                if let Some(elem) = spec.visual_elements.iter_mut().find(|el| el.element_id == format!("focus_{term_idx}")) {
                    elem.properties["svg_path"] = serde_json::json!(svg_path.to_string_lossy());
                }
            }
        }
        shot_specs.push(spec);
    }
    println!("   {} shots planned", shot_specs.len());

    let total_timeline_duration = shot_specs
        .iter()
        .map(|spec| spec.duration_frames as f64 / spec.fps as f64)
        .sum::<f64>();
    validate_format_duration(format, total_timeline_duration)
        .map_err(|error| anyhow::anyhow!("Pre-render plan QA: {error}"))?;
    println!("   ✓ Planned runtime {:.1}s passes {} QA", total_timeline_duration, profile.slug);

    timer.begin("render_and_score");
    let render_parallelism = env_parallelism("ATLAS_RENDER_JOBS");
    println!(
        "🎨 Stage 4: Rendering with Blender ({} parallel job{}) while scoring with AIWaves...",
        render_parallelism,
        if render_parallelism == 1 { "" } else { "s" }
    );
    project.state = ProjectState::AssetBuild;
    store.insert_project(&project)?;

    let blender_exe = "blender";
    let blender_pipeline = BlenderPipeline::new(blender_exe, &data_dir);

    let mut render_jobs = Vec::new();
    for spec in &shot_specs {
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
            println!("   ✓ {} {} verified cached frames", spec.shot_id, reusable_frames);
            continue;
        }
        render_jobs.push(atlas_blender::RenderJob {
            job_id: format!("job_{}", spec.shot_id),
            shot_spec: spec.clone(),
            quality: RenderQuality::Final,
            output_dir: frames_dir,
            status: atlas_blender::RenderStatus::Queued,
            output_size: Some([profile.width, profile.height]),
        });
    }

    let (render_result, (original_score, original_sfx)) = tokio::join!(
        render_shots(&blender_pipeline, render_jobs, render_parallelism),
        generate_aiwaves_audio(
            &project.video_id,
            &project.topic.title,
            &project_dir,
            &data_dir,
            total_timeline_duration,
            &shot_specs,
        ),
    );
    render_result?;

    timer.begin("assembly_and_qa");
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

    let mut audio_tracks = vec![AudioTrack {
        track_id: "narration".into(),
        source_path: combined_audio.clone(),
        start_frame: 0,
        volume_db: 0.0,
        fade_in_frames: 15,
        fade_out_frames: 15,
    }];
    if let Some(score_path) = original_score {
        println!("   ✓ Original score synthesized: {}", score_path.display());
        audio_tracks.push(AudioTrack {
            track_id: "original_score".into(),
            source_path: score_path,
            start_frame: 0,
            volume_db: -20.0,
            fade_in_frames: 45,
            fade_out_frames: 60,
        });
    }
    if let Some(sfx_path) = original_sfx {
        println!("   ✓ Procedural sound effects synthesized: {}", sfx_path.display());
        audio_tracks.push(AudioTrack {
            track_id: "original_sfx".into(),
            source_path: sfx_path,
            start_frame: 0,
            volume_db: -12.0,
            fade_in_frames: 5,
            fade_out_frames: 10,
        });
    }

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
        audio_tracks,
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
    let export_dir = output_video
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid export path"))?;
    if let Some(effectcraft_mov) = generate_motion_graphic_overlay(export_dir, assembly.duration_seconds, profile.width, profile.height).await {
        println!("✨ Applying EffectCraft motion overlay: {}", effectcraft_mov.display());
        let composited = output_video.with_extension("composited.mp4");
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-i"]).arg(&output_video)
            .args(["-i"]).arg(&effectcraft_mov)
            .args(["-filter_complex", "[0:v][1:v]overlay=0:main_h-overlay_h:enable='between(t,1,5)'[v]", "-map", "[v]", "-map", "0:a?", "-c:v", "libx264", "-c:a", "copy"])
            .arg(&composited)
            .status()
            .await;
        if matches!(status, Ok(s) if s.success()) && composited.is_file() {
            let _ = std::fs::copy(&composited, &output_video);
            let _ = std::fs::remove_file(composited);
            let _ = std::fs::remove_file(effectcraft_mov);
            println!("   ✓ Motion graphic overlay composited into master video");
        }
    }
    if assembly.duration_seconds < 5.0 || assembly.frame_count < 150 {
        anyhow::bail!(
            "Release QA rejected output: {:.2}s and {} frames",
            assembly.duration_seconds,
            assembly.frame_count
        );
    }
    validate_format_duration(format, f64::from(assembly.duration_seconds))?;
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
        VideoFormat::Long => format!("{} | {}", project.topic.title, atlas_core::COMPANY_NAME),
        VideoFormat::Short => format!("{} | #Shorts", project.topic.title),
    };
    let metadata = serde_json::json!({
        "title": title,
        "description": format!("A concise whiteboard explanation produced by {}. Narration and visuals were generated with AI-assisted production tools. Educational demonstration; sources should be reviewed before reuse.", atlas_core::COMPANY_NAME),
        "transcript": script,
        "contains_synthetic_media": true,
        "credits": [atlas_core::COMPANY_NAME, "Blender", "Kokoro TTS", "FFmpeg"],
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

    timer.begin("package");
    println!("✅ Stage 6: Quality check & package...");
    project.state = ProjectState::ReleaseQa;
    store.insert_project(&project)?;

    let timings = timer.finish();
    std::fs::write(
        export_dir.join("stage_timings.json"),
        serde_json::to_vec_pretty(&timings)?,
    )?;

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
    println!("Stage timings ({:.1}s total):", timings.total_seconds);
    for stage in &timings.stages {
        let marker = if timings.slowest.as_deref() == Some(stage.stage) { "  ← slowest" } else { "" };
        println!("  {:22} {:>8.1}s{}", stage.stage, stage.seconds, marker);
    }
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
    #[serde(default)]
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
    let caption_path = if export_dir.join("captions_en.srt").is_file() {
        export_dir.join("captions_en.srt")
    } else {
        export_dir.join("captions.srt")
    };
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
        if let Err(caption_err) = client
            .upload_captions(&result.video_id, &caption_path, "en-US")
            .await
        {
            eprintln!("Warning: caption upload failed ({caption_err}), proceeding with published video.");
        }
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

async fn generate_motion_graphic_overlay(
    output_dir: &Path,
    _duration_seconds: f32,
    _width: u32,
    _height: u32,
) -> Option<PathBuf> {
    let overlay_path = output_dir.join("effectcraft_overlay.mov");
    if overlay_path.is_file() {
        return Some(overlay_path);
    }
    let effectcraft = bundled_cli("effectcraft")?;
    let status = tokio::process::Command::new(effectcraft)
        .arg("--demo")
        .arg("render")
        .args(["--comp", "Lower Third", "--format", "prores", "--channels", "rgba", "--start", "0", "--end", "2", "--out"])
        .arg(&overlay_path)
        .status()
        .await;
    if matches!(status, Ok(s) if s.success()) && overlay_path.is_file() {
        Some(overlay_path)
    } else {
        None
    }
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
struct FreesoundSound {
    id: u64,
    name: String,
    duration: f64,
    preview_url: Option<String>,
}

struct FreesoundClient {
    api_key: Option<String>,
    cache_dir: PathBuf,
    client: reqwest::Client,
}

impl FreesoundClient {
    fn new(data_dir: &Path) -> Self {
        let key = std::env::var("FREESOUND_API_KEY").ok().or_else(|| {
            let key_file = data_dir.join("freesound_key.txt");
            std::fs::read_to_string(key_file).ok().map(|s| s.trim().to_string())
        }).or_else(|| {
            let creds_file = PathBuf::from("freesound_credentials.json");
            std::fs::read_to_string(creds_file).ok().and_then(|text| {
                serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| {
                    v.get("api_key").or_else(|| v.get("token")).and_then(|t| t.as_str()).map(|s| s.to_string())
                })
            })
        });

        let cache_dir = data_dir.join("audio").join("freesound_cache");
        let _ = std::fs::create_dir_all(&cache_dir);

        Self {
            api_key: key,
            cache_dir,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build()
                .unwrap_or_default(),
        }
    }

    fn has_key(&self) -> bool {
        self.api_key.as_ref().is_some_and(|k| !k.is_empty())
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<FreesoundSound>> {
        let key = self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Freesound API key not configured. Set FREESOUND_API_KEY env var or save in freesound_credentials.json")
        })?;
        let encoded_query = query.replace(' ', "+");
        let url = format!(
            "https://freesound.org/apiv2/search/text/?query={encoded_query}&token={key}&fields=id,name,previews,duration&filter=duration:[0.1+TO+6.0]&page_size={}",
            limit.clamp(1, 15)
        );

        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        let results = json.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();

        let mut sounds = Vec::new();
        for r in results {
            if let (Some(id), Some(name)) = (r.get("id").and_then(|v| v.as_u64()), r.get("name").and_then(|v| v.as_str())) {
                let preview_url = r.get("previews")
                    .and_then(|p| p.get("preview-hq-mp3").or_else(|| p.get("preview-lq-mp3")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let duration = r.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);
                sounds.push(FreesoundSound {
                    id,
                    name: name.to_string(),
                    duration,
                    preview_url,
                });
            }
        }
        Ok(sounds)
    }

    async fn download(&self, sound_id: u64, preview_url: Option<&str>, target_name: Option<&str>) -> anyhow::Result<PathBuf> {
        let filename = target_name
            .map(|n| format!("{n}.wav"))
            .unwrap_or_else(|| format!("freesound_{sound_id}.wav"));
        let target_path = self.cache_dir.join(&filename);
        if target_path.is_file() {
            return Ok(target_path);
        }

        let url = if let Some(u) = preview_url {
            u.to_string()
        } else {
            let key = self.api_key.as_deref().ok_or_else(|| anyhow::anyhow!("Missing Freesound API key"))?;
            let info_url = format!("https://freesound.org/apiv2/sounds/{}/?token={key}", sound_id);
            let resp = self.client.get(&info_url).send().await?.error_for_status()?;
            let info: serde_json::Value = resp.json().await?;
            info.get("previews")
                .and_then(|p| p.get("preview-hq-mp3").or_else(|| p.get("preview-lq-mp3")))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("No preview audio available for Freesound ID {sound_id}"))?
                .to_string()
        };

        let temp_mp3 = self.cache_dir.join(format!("temp_{sound_id}.mp3"));
        let bytes = self.client.get(&url).send().await?.error_for_status()?.bytes().await?;
        tokio::fs::write(&temp_mp3, &bytes).await?;

        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(&temp_mp3)
            .args(["-ar", "44100", "-ac", "2", "-c:a", "pcm_s16le"])
            .arg(&target_path)
            .status()
            .await;
        let _ = tokio::fs::remove_file(&temp_mp3).await;

        if matches!(status, Ok(s) if s.success()) && target_path.is_file() {
            Ok(target_path)
        } else {
            anyhow::bail!("FFmpeg failed converting Freesound {sound_id} preview to WAV");
        }
    }

    async fn ensure_cached_effect(&self, semantic_name: &str, search_query: &str) -> Option<PathBuf> {
        let cached = self.cache_dir.join(format!("{semantic_name}.wav"));
        if cached.is_file() {
            return Some(cached);
        }
        if !self.has_key() {
            return None;
        }
        match self.search(search_query, 3).await {
            Ok(sounds) if !sounds.is_empty() => {
                let first = &sounds[0];
                match self.download(first.id, first.preview_url.as_deref(), Some(semantic_name)).await {
                    Ok(path) => {
                        println!("   ✓ Freesound.org cached: {semantic_name} ({})", first.name);
                        Some(path)
                    }
                    Err(e) => {
                        eprintln!("Freesound download warning for {semantic_name}: {e}");
                        None
                    }
                }
            }
            Ok(_) => None,
            Err(e) => {
                eprintln!("Freesound query warning for {semantic_name}: {e}");
                None
            }
        }
    }
}

async fn cmd_freesound(store: &Store, action: FreesoundAction) -> anyhow::Result<()> {
    let client = FreesoundClient::new(store.data_dir());
    match action {
        FreesoundAction::Status => {
            println!("Freesound.org Integration Status");
            println!("================================");
            if client.has_key() {
                println!("API Key:       Configured (Active)");
            } else {
                println!("API Key:       Not Configured");
                println!("  To activate: set FREESOUND_API_KEY environment variable or save key in freesound_credentials.json");
            }
            println!("Cache Directory: {}", client.cache_dir.display());
            let cached_count = std::fs::read_dir(&client.cache_dir)
                .ok()
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav"))
                        .count()
                })
                .unwrap_or(0);
            println!("Cached SFX:    {} sound file(s)", cached_count);
        }
        FreesoundAction::Search { query, limit } => {
            println!("Searching Freesound.org for: '{}' (limit {})", query, limit);
            println!("------------------------------------------------------------");
            let results = client.search(&query, limit).await?;
            if results.is_empty() {
                println!("No sounds found matching query.");
            } else {
                for sound in results {
                    println!("[ID: {:>8}] {:.2}s  {}", sound.id, sound.duration, sound.name);
                    if let Some(preview) = sound.preview_url {
                        println!("             Preview: {}", preview);
                    }
                }
            }
        }
        FreesoundAction::Download { sound_id, name } => {
            println!("Downloading Freesound sound #{}...", sound_id);
            let path = client.download(sound_id, None, name.as_deref()).await?;
            println!("Successfully saved to: {}", path.display());
        }
    }
    Ok(())
}

async fn generate_aiwaves_audio(
    project_id: &str,
    topic: &str,
    project_dir: &Path,
    data_dir: &Path,
    duration: f64,
    shot_specs: &[atlas_blender::ShotSpec],
) -> (Option<PathBuf>, Option<PathBuf>) {
    let aiwaves_dir = PathBuf::from(
        std::env::var("AIWAVES_DIR").unwrap_or_else(|_| r"C:\Users\chrom\Desktop\aiwaves".to_string()),
    );
    let audio_dir = project_dir.join("audio");
    let _ = tokio::fs::create_dir_all(&audio_dir).await;

    let score_path = audio_dir.join(format!("aiwaves_score_{project_id}.wav"));
    let sfx_path = audio_dir.join(format!("aiwaves_sfx_{project_id}.wav"));

    let freesound = FreesoundClient::new(data_dir);
    if freesound.has_key() {
        println!("🎵 Freesound.org active: syncing authentic whiteboard sound effects...");
    } else {
        println!("🎵 Freesound.org: using local cache and AIWaves sound library (set FREESOUND_API_KEY to search online)");
    }

    // 1. Generate unique, topic-attuned original musical score
    let score_res = if score_path.is_file() {
        Some(score_path.clone())
    } else {
        let hash: u64 = topic
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let root_freqs = [130.81, 146.83, 164.81, 174.61, 196.00, 220.00];
        let root = root_freqs[(hash % root_freqs.len() as u64) as usize];
        let fifth = root * 1.5;
        let ninth = root * 2.25;
        let sub = root * 0.5;
        let safe_dur = duration.max(5.0);
        let fade_out_start = (safe_dur - 2.5).max(1.0);

        let aiwaves_master = aiwaves_dir
            .join("fixtures")
            .join("longing")
            .join("audio")
            .join("master.wav");
        let status = if aiwaves_master.is_file() {
            let filter = format!(
                "[0:a]lowpass=f=600,volume=0.18,atrim=duration={safe_dur:.2}[a0];[1:a]lowpass=f=420,volume=0.25[a1];[a0][a1]amix=inputs=2:duration=first:dropout_transition=0,afade=t=in:st=0:d=2.0,afade=t=out:st={fade_out_start:.2}:d=2.5"
            );
            tokio::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error"])
                .args(["-stream_loop", "-1", "-i"])
                .arg(&aiwaves_master)
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    &format!(
                        "aevalsrc=0.03*sin(2*PI*{root:.2}*t)+0.02*sin(2*PI*{fifth:.2}*t)+0.015*sin(2*PI*{ninth:.2}*t)+0.025*sin(2*PI*{sub:.2}*t):d={safe_dur:.2}:s=44100"
                    ),
                ])
                .args(["-filter_complex", &filter, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"])
                .arg(&score_path)
                .status()
                .await
        } else {
            let filter = format!(
                "aevalsrc=0.04*sin(2*PI*{root:.2}*t)+0.03*sin(2*PI*{fifth:.2}*t)+0.02*sin(2*PI*{ninth:.2}*t)+0.035*sin(2*PI*{sub:.2}*t):d={safe_dur:.2}:s=44100,lowpass=f=450,afade=t=in:st=0:d=2.0,afade=t=out:st={fade_out_start:.2}:d=2.5"
            );
            tokio::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", &filter, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"])
                .arg(&score_path)
                .status()
                .await
        };
        if matches!(status, Ok(s) if s.success()) && score_path.is_file() {
            Some(score_path)
        } else {
            None
        }
    };

    // 2. Generate original synchronized procedural sound effects (SFX)
    let sfx_res = if sfx_path.is_file() {
        Some(sfx_path.clone())
    } else {
        let safe_dur = duration.max(5.0);

        // Pre-fetch or query freesound effects if available
        let marker_draw = freesound.ensure_cached_effect("marker_draw", "marker whiteboard drawing").await;
        let eraser_swoosh = freesound.ensure_cached_effect("whiteboard_swoosh", "whiteboard eraser whoosh").await;

        let intro_sfx = aiwaves_dir.join("media").join("intro").join("intro_sfx.wav");
        let pan_notes = [
            aiwaves_dir.join("panNoteC.wav"),
            aiwaves_dir.join("panNoteD.wav"),
            aiwaves_dir.join("panNoteE.wav"),
            aiwaves_dir.join("panNoteG.wav"),
            aiwaves_dir.join("panNoteA.wav"),
        ];

        let mut cmd = tokio::process::Command::new("ffmpeg");
        cmd.args(["-y", "-loglevel", "error"]);

        let mut inputs_count = 0;
        let mut filter_parts = Vec::new();

        // Opening sound effect (intro reveal)
        if intro_sfx.is_file() {
            cmd.arg("-i").arg(&intro_sfx);
            filter_parts.push(format!("[{inputs_count}:a]volume=0.35,atrim=duration=4[s{inputs_count}]"));
            inputs_count += 1;
        }

        // Shot boundary chimes & marker sound effects
        let mut current_frame = 0u32;
        for (i, spec) in shot_specs.iter().enumerate() {
            if i > 0 {
                let delay_ms = (current_frame as f64 / spec.fps as f64 * 1000.0).round() as u64;

                // Alternate between Freesound swoosh and AIWaves melodic chimes
                if let Some(swoosh) = &eraser_swoosh {
                    if i % 2 == 1 {
                        cmd.arg("-i").arg(swoosh);
                        filter_parts.push(format!("[{inputs_count}:a]adelay={delay_ms}:all=1,volume=0.28,atrim=duration=2.5[s{inputs_count}]"));
                        inputs_count += 1;
                    }
                }

                let note_idx = (i - 1) % pan_notes.len();
                let note_path = &pan_notes[note_idx];
                if note_path.is_file() {
                    cmd.arg("-i").arg(note_path);
                    filter_parts.push(format!("[{inputs_count}:a]adelay={delay_ms}:all=1,volume=0.32,atrim=duration=3[s{inputs_count}]"));
                    inputs_count += 1;
                }
            }
            current_frame += spec.duration_frames;
        }

        // Add marker stroke Foley at start of first shot
        if let Some(draw) = &marker_draw {
            cmd.arg("-i").arg(draw);
            filter_parts.push(format!("[{inputs_count}:a]adelay=400:all=1,volume=0.22,atrim=duration=3.5[s{inputs_count}]"));
            inputs_count += 1;
        }

        if inputs_count > 0 {
            let stream_tags = (0..inputs_count).map(|idx| format!("[s{idx}]")).collect::<String>();
            let complex = format!(
                "{};{stream_tags}amix=inputs={inputs_count}:duration=first:dropout_transition=0,atrim=duration={safe_dur:.2}",
                filter_parts.join(";")
            );
            cmd.args(["-filter_complex", &complex, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"]).arg(&sfx_path);
            let status = cmd.status().await;
            if matches!(status, Ok(s) if s.success()) && sfx_path.is_file() {
                Some(sfx_path)
            } else {
                None
            }
        } else {
            let complex = format!(
                "anoisesrc=d={safe_dur:.2}:c=white:r=44100,bandpass=f=1100:w=500,volume=0.18,afade=t=in:st=0:d=0.08,afade=t=out:st=0.12:d=0.20"
            );
            let status = tokio::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", &complex, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"])
                .arg(&sfx_path)
                .status()
                .await;
            if matches!(status, Ok(s) if s.success()) && sfx_path.is_file() {
                Some(sfx_path)
            } else {
                None
            }
        }
    };

    (score_res, sfx_res)
}

fn bundled_cli(name: &str) -> Option<PathBuf> {
    let direct = which::which(format!("{name}-cli")).ok();
    direct
        .or_else(|| {
            let artcraft_dir = PathBuf::from(r"C:\Users\chrom\Tools\ArtCraft")
                .join(name)
                .join(format!("{name}-cli.exe"));
            if artcraft_dir.is_file() {
                Some(artcraft_dir)
            } else {
                None
            }
        })
        .or_else(|| {
            let tools = PathBuf::from("tools").join(name);
            std::fs::read_dir(tools)
                .ok()?
                .filter_map(Result::ok)
                .map(|entry| entry.path().join(format!("{name}-cli.exe")))
                .find(|candidate| candidate.is_file())
        })
}

#[derive(Clone)]
struct ProductionBeat {
    title: String,
    narration: String,
    scene: String,
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
            Self::Curated(beat) => &beat.title,
            Self::Directed(beat) => &beat.title,
        }
    }

    fn narration(&self) -> &str {
        match self {
            Self::Curated(beat) => &beat.narration,
            Self::Directed(beat) => &beat.narration,
        }
    }

    fn scene(&self) -> &str {
        match self {
            Self::Curated(beat) => &beat.scene,
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
            Self::Curated(beat) => morph_for_scene(&beat.scene),
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

    fn focal_terms(&self) -> Vec<String> {
        match self {
            Self::Curated(beat) => curated_visual_plan(&beat.scene).1,
            Self::Directed(beat) => beat.focal_terms.clone(),
        }
    }

    fn apply_visuals(&self, spec: &mut atlas_blender::ShotSpec, portrait: bool) {
        if let Self::Curated(beat) = self {
            let (pattern, focal_terms) = curated_visual_plan(&beat.scene);
            let directed = StoryboardBeat {
                title: beat.title.clone(),
                narration: beat.narration.clone(),
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
    let s = scene.to_ascii_lowercase();
    if s.contains("hook") {
        (
            ScenePattern::Hook,
            vec!["chemical imbalance".into(), "stored energy".into()],
        )
    } else if s.contains("anatomy") || s.contains("part") {
        (
            ScenePattern::Parts,
            vec!["anode".into(), "separator".into(), "cathode".into()],
        )
    } else if s.contains("electrolyte") || s.contains("split") {
        (
            ScenePattern::SplitPath,
            vec!["ions pass".into(), "electrons stop".into()],
        )
    } else if s.contains("flow") || s.contains("charg") {
        (
            ScenePattern::Flow,
            vec!["intake flow".into(), "transport channel".into(), "reaction core".into()],
        )
    } else if s.contains("electron") || s.contains("wire") {
        (
            ScenePattern::SplitPath,
            vec!["conduction path".into(), "external circuit".into()],
        )
    } else if s.contains("discharg") {
        (
            ScenePattern::Flow,
            vec!["driving potential".into(), "load circuit".into(), "lower state".into()],
        )
    } else if s.contains("voltage") || s.contains("scale") {
        (
            ScenePattern::Scale,
            vec![
                "high energy".into(),
                "voltage drop".into(),
                "useful work".into(),
            ],
        )
    } else if s.contains("aging") || s.contains("cause") {
        (
            ScenePattern::CauseEffect,
            vec![
                "stress".into(),
                "side reactions".into(),
                "capacity loss".into(),
            ],
        )
    } else if s.contains("safe") || s.contains("risk") || s.contains("limit") {
        (
            ScenePattern::Risk,
            vec!["barrier".into(), "thermal stress".into(), "protection".into()],
        )
    } else {
        (
            ScenePattern::Cycle,
            vec!["input work".into(), "storage state".into(), "useful power".into()],
        )
    }
}

async fn planned_video(
    topic: &str,
    format: VideoFormat,
    long_plan: Option<&StoryboardPlan>,
) -> anyhow::Result<PlannedVideo> {
    let expected = format.profile().shot_count as usize;
    if matches!(format, VideoFormat::Short) {
        if let Some(long_plan) = long_plan {
            if let Some(director) = DirectorClient::from_env() {
                match director.create_short_plan(long_plan, expected).await {
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
                    Err(error) => eprintln!("Creative director fallback for short adaptation: {error}"),
                }
            }
        }
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
    Ok(PlannedVideo {
        beats: production_beats(topic, format)
            .into_iter()
            .map(VideoBeat::Curated)
            .collect(),
        generated_plan: None,
    })
}

fn production_beats(topic: &str, format: VideoFormat) -> Vec<ProductionBeat> {
    if !topic.to_ascii_lowercase().contains("lithium") {
        return generic_beats(topic, format);
    }
    match format {
        VideoFormat::Short => vec![
            ProductionBeat {
                title: "YOUR BATTERY IS NOT A TANK".into(),
                narration: "Your battery does not store electricity like a bottle; it stores a chemical imbalance, and Luma will show you the trick.".into(),
                scene: "battery_hook".into(),
                emotion: MascotEmotion::Curious,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "THREE LAYERS CONTROL THE ACTION".into(),
                narration: "Two electrodes hold lithium, while an electrolyte carries ions and a separator prevents a dangerous short circuit.".into(),
                scene: "battery_anatomy".into(),
                emotion: MascotEmotion::Explaining,
                gesture: MascotGesture::PointLeft,
            },
            ProductionBeat {
                title: "CHARGING SEPARATES THE PLAYERS".into(),
                narration: "A charger pushes lithium ions into the graphite anode while electrons travel there through the outside wire.".into(),
                scene: "battery_charging".into(),
                emotion: MascotEmotion::Excited,
                gesture: MascotGesture::PointRight,
            },
            ProductionBeat {
                title: "DISCHARGING MAKES CURRENT".into(),
                narration: "When you unplug, ions return through the electrolyte and electrons power your device on their separate route.".into(),
                scene: "battery_discharging".into(),
                emotion: MascotEmotion::Confident,
                gesture: MascotGesture::Present,
            },
            ProductionBeat {
                title: "HEAT STEALS CAPACITY".into(),
                narration: "Heat and side reactions build barriers that trap active lithium, so an older battery cannot hold the same charge.".into(),
                scene: "battery_aging".into(),
                emotion: MascotEmotion::Concerned,
                gesture: MascotGesture::Warn,
            },
        ],
        VideoFormat::Long => {
            let core_beats = [
                ("A BATTERY STORES A CHEMICAL IMBALANCE", "A lithium ion battery is not a tank of electricity; it stores energy by separating charge into a chemical state that actively wants to relax toward equilibrium.", "battery_hook", MascotEmotion::Curious, MascotGesture::Present),
                ("THE CELL HAS FOUR CRITICAL PARTS", "The graphite anode and metal oxide cathode host lithium ions, the electrolyte carries those ions between them, and the porous separator prevents physical short circuits.", "battery_anatomy", MascotEmotion::Explaining, MascotGesture::PointLeft),
                ("IONS MAY CROSS; ELECTRONS MAY NOT", "The electrolyte conducts positively charged lithium ions smoothly, but it is an electrical insulator for electrons, forcing charge into external conductors.", "battery_electrolyte", MascotEmotion::Explaining, MascotGesture::PointRight),
                ("CHARGING DRIVES LITHIUM UPHILL", "When connected to a power supply, external electrical work pulls lithium atoms from the metal oxide cathode and drives them as ions into graphite interstitial sheets.", "battery_charging", MascotEmotion::Excited, MascotGesture::PointLeft),
                ("ELECTRONS TAKE THE OUTSIDE ROAD", "Simultaneously, electrons travel through the external circuit wire because the internal separator and electrolyte completely block electron transit.", "battery_electrons", MascotEmotion::Explaining, MascotGesture::PointRight),
                ("DISCHARGING POWERS THE OUTSIDE WORLD", "Upon connecting a load, spontaneous chemical potentials drive ions back through the electrolyte while electrons flow through your external device.", "battery_discharging", MascotEmotion::Confident, MascotGesture::Present),
                ("VOLTAGE MEASURES THE CHEMICAL POTENTIAL", "Terminal voltage is a direct measure of the Gibbs free energy difference between lithium intercalated in graphite versus the metal oxide crystal lattice.", "battery_voltage", MascotEmotion::Curious, MascotGesture::Think),
                ("THE SOLID ELECTROLYTE INTERPHASE FORMS", "During the very first charging cycles, electrolyte decomposition forms a passivating SEI film on the anode that protects the electrode from further continuous solvent breakdown.", "battery_aging", MascotEmotion::Explaining, MascotGesture::PointRight),
                ("PARASITIC SIDE REACTIONS CONSUME CAPACITY", "Elevated temperatures and prolonged exposure to high states of charge accelerate secondary parasitic chemical reactions that permanently trap active lithium ions.", "battery_aging", MascotEmotion::Concerned, MascotGesture::Warn),
                ("LITHIUM PLATING AND DENDRITE FORMATION", "Charging too rapidly at freezing temperatures forces lithium ions to deposit as metallic needles rather than intercalating into graphite, risking internal punctures.", "battery_safety", MascotEmotion::Concerned, MascotGesture::Warn),
                ("THE SEPARATOR AS A CRUCIAL SAFETY BARRIER", "A ceramic-coated polymer separator maintains a microscopic gap between opposing electrodes, with shutdown pores that seal up if the cell overheats.", "battery_safety", MascotEmotion::Explaining, MascotGesture::PointLeft),
                ("CATHODE CHEMISTRY GOVERNS ENERGY DENSITY", "Commercial cells choose between nickel-manganese-cobalt formulations for high specific energy density and lithium iron phosphate for superior thermal stability and cyclic longevity.", "battery_anatomy", MascotEmotion::Explaining, MascotGesture::Present),
                ("THE COMPLETE THERMODYNAMIC CYCLE", "Every battery cycle represents a controlled round-trip migration: electrical work organizes high-energy states, and controlled discharge extracts that energy to power modern life.", "battery_summary", MascotEmotion::Confident, MascotGesture::Celebrate),
            ];
            let mut beats = Vec::new();
            for i in 0..35 {
                let (title, narration, scene, emotion, gesture) = &core_beats[i % core_beats.len()];
                beats.push(ProductionBeat {
                    title: (*title).to_string(),
                    narration: (*narration).to_string(),
                    scene: (*scene).to_string(),
                    emotion: emotion.clone(),
                    gesture: gesture.clone(),
                });
            }
            beats
        }
    }
}

fn generic_beats(topic: &str, format: VideoFormat) -> Vec<ProductionBeat> {
    let count = format.profile().shot_count as usize;
    let clean_topic = topic.trim();
    let upper = clean_topic.to_ascii_uppercase();
    let lower = clean_topic.to_ascii_lowercase();

    let is_fluid_wave = lower.contains("vort")
        || lower.contains("wave")
        || lower.contains("vibrat")
        || lower.contains("flow")
        || lower.contains("aero")
        || lower.contains("hydro");
    let is_bio_chem = lower.contains("gene")
        || lower.contains("crispr")
        || lower.contains("cell")
        || lower.contains("dna")
        || lower.contains("protein")
        || lower.contains("enzyme")
        || lower.contains("organ");
    let is_compute = lower.contains("comput")
        || lower.contains("neural")
        || lower.contains("algorithm")
        || lower.contains("code")
        || lower.contains("quantum")
        || lower.contains("silicon")
        || lower.contains("chip");

    let archetypes = if is_fluid_wave {
        vec![
            (
                format!("THE PUZZLE OF {upper}"),
                format!("When examining {clean_topic}, subtle fluid boundary layer interactions trigger unexpected macroscopic motion that defies simple intuition."),
                "wave_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "FLUID BOUNDARIES AND SHEDDING".to_string(),
                format!("As moving flow impinges upon {clean_topic}, boundary layer separation produces asymmetric pressure gradients."),
                "vort_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "HYDRODYNAMIC DISPLACEMENT".to_string(),
                format!("Alternating vortex shedding forces the physical structure of {clean_topic} into cyclical transverse oscillation."),
                "flow_transport",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "RESONANCE AND LOCK-IN ENVELOPE".to_string(),
                format!("When vortex shedding frequency synchronizes with the natural resonance of {clean_topic}, lock-in amplifies displacement into rapid fatigue."),
                "vort_resonance",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "ENGINEERING SUPPRESSION AND MITIGATION".to_string(),
                format!("Helical strakes, tuned mass dampers, and hydrodynamic fairings disrupt coherent shedding to protect {clean_topic}."),
                "risk_mitigation",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    } else if is_bio_chem {
        vec![
            (
                format!("THE CODE OF {upper}"),
                format!("In the molecular machinery of {clean_topic}, precise biochemical interactions orchestrate biological function with atomic fidelity."),
                "bio_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "MOLECULAR STRUCTURE AND DOMAINS".to_string(),
                format!("The three-dimensional architecture of {clean_topic} relies on specific active sites and complementary binding domains."),
                "cell_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "CATALYTIC AND TRANSPORT KINETICS".to_string(),
                format!("Substrates migrate through molecular pathways in {clean_topic}, lowering activation barriers for spontaneous reactions."),
                "transport_flow",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "REGULATION AND CELLULAR EQUILIBRIUM".to_string(),
                format!("Feedback mechanisms monitor reaction thresholds in {clean_topic}, preventing metabolic instability or toxic build-up."),
                "risk_boundary",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "THE REGENERATIVE LIFE CYCLE".to_string(),
                format!("By coupling thermodynamic driving forces with cellular repair, {clean_topic} maintains homeostatic continuity."),
                "cycle_summary",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    } else if is_compute {
        vec![
            (
                format!("HOW {upper} COMPUTES"),
                format!("At the foundation of {clean_topic}, microscopic state transitions coordinate to solve complex computational problems."),
                "circuit_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "LOGIC GATES AND SYSTEM ARCHITECTURE".to_string(),
                format!("Information in {clean_topic} flows across interconnected logic gates and memory registers structured in deterministic hierarchies."),
                "chip_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "SIGNAL PROPAGATION AND CONCURRENCY".to_string(),
                format!("Synchronous clock pulses propagate high-frequency data streams across the routing buses of {clean_topic}."),
                "signal_flow",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "LATENCY, BOTTLENECKS, AND THERMAL LIMITS".to_string(),
                format!("Parasitic capacitance and thermal dissipation create fundamental scaling limits for modern {clean_topic}."),
                "thermal_risk",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "THE UNIFIED COMPUTATIONAL MODEL".to_string(),
                format!("From basic bit operations to global distributed throughput, {clean_topic} transforms simple rules into immense capability."),
                "system_summary",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    } else {
        vec![
            (
                format!("INVESTIGATING {upper}"),
                format!("Every physical principle in {clean_topic} operates under strict laws governing conservation of energy and equilibrium."),
                "universal_hook",
                MascotEmotion::Curious,
                MascotGesture::Present,
            ),
            (
                "SYSTEM BOUNDARIES AND ARCHITECTURE".to_string(),
                format!("By separating {clean_topic} into core functional modules, we can observe how each component behaves under operational load."),
                "universal_anatomy",
                MascotEmotion::Explaining,
                MascotGesture::PointLeft,
            ),
            (
                "DYNAMIC EQUILIBRIUM AND TRANSPORT".to_string(),
                format!("Continuous mass and energy transfer through {clean_topic} establishes steady-state operation."),
                "universal_flow",
                MascotEmotion::Explaining,
                MascotGesture::PointRight,
            ),
            (
                "OPERATIONAL THRESHOLDS AND STRESS".to_string(),
                format!("Subjecting {clean_topic} to excessive operating conditions exposes boundary failure points and safety limits."),
                "universal_risk",
                MascotEmotion::Concerned,
                MascotGesture::Warn,
            ),
            (
                "SYNTHESIS OF THE COMPLETE MODEL".to_string(),
                format!("Understanding the governing relationships of {clean_topic} provides a deterministic mental model of cause and effect."),
                "universal_cycle",
                MascotEmotion::Confident,
                MascotGesture::Celebrate,
            ),
        ]
    };

    let mut beats = Vec::with_capacity(count);
    for i in 0..count {
        let (title, narration, scene, emotion, gesture) = &archetypes[i % archetypes.len()];
        let beat_title = if count > archetypes.len() {
            format!("{title} [PART {}]", (i / archetypes.len()) + 1)
        } else {
            title.clone()
        };
        beats.push(ProductionBeat {
            title: beat_title,
            narration: narration.clone(),
            scene: scene.to_string(),
            emotion: emotion.clone(),
            gesture: gesture.clone(),
        });
    }
    beats
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
            println!("{} — Analytics Dashboard", atlas_core::COMPANY_NAME);
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
                println!("{} — Dual-Memory Stack Status", atlas_core::COMPANY_NAME);
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
