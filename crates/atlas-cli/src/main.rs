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
mod news_aired;
mod news_health;
mod news_lipsync;
mod news_preview;
mod news_voice;
mod planning;
mod produce;
mod production;
mod publish;

use analytics::cmd_analytics;
use audio::*;
use broll::import_broll;
use memory::cmd_memory;
use planning::*;
use produce::*;
use publish::*;
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
    #[command(about = "Render (or re-render) an episode's preview animatic from its voiced script")]
    Animatic {
        #[arg(long)]
        episode: String,
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
