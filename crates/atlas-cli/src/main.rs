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

mod analytics;
mod audio;
mod broll;
mod cache;
mod daemon;
mod memory;
mod news;
mod news_health;
mod news_voice;
mod planning;
mod production;

use analytics::cmd_analytics;
use audio::*;
use broll::import_broll;
use memory::cmd_memory;
use planning::*;
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
