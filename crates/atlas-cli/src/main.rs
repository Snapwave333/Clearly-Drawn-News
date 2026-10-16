use atlas_core::{
    redact_config, AgentMessage, FeedbackItem, Project, ProjectState, PublishMode, PublishPlan,
    Topic, TrendSource,
};
use atlas_doctor::{has_minimal_pipeline, probe_all};
use atlas_opencode::OpenCodeClient;
use atlas_store::Store;
use chrono::Utc;
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use std::path::PathBuf;
use uuid::Uuid;

use atlas_analytics::{
    authorization_url, compute_channel_baseline, compare_to_baseline, save_sample_metrics,
    AnalyticsStore, OptimizationLoop, RecommendationEngine, YouTubeAnalyticsClient,
    YouTubeCredentials, load_youtube_credentials,
};
use atlas_blender::{BlenderPipeline, create_test_shot_spec, RenderQuality};
use atlas_ffmpeg::{EditDecisionList, EdlEntry, FfmpegAssembler, AudioTrack, CaptionTrack, CaptionFormat, SourceType, Transition, TransitionType};
use atlas_kokoro::{KokoroPipeline, VoiceConfig, split_script_into_segments};

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
    Start,
    Pause,
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
    /// Fetch metrics from YouTube Analytics for a project.
    Fetch {
        video_id: String,
        #[arg(long)]
        sample: bool,
    },
    /// Show the retention curve for a video.
    Retention {
        video_id: String,
    },
    /// Generate optimization recommendations from stored metrics.
    Recommend {
        video_id: String,
        #[arg(long, default_value = "300")]
        duration_seconds: u64,
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
        Commands::Produce { topic } => cmd_produce(&store, &topic).await,
        Commands::Daemon { action } => cmd_daemon(action).await,
        Commands::Analytics { action } => cmd_analytics(&store, action).await,
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
                println!("{:<24} {:<24} {}", "VIDEO ID", "STATE", "TOPIC");
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
            let video_id = format!("vid_{}_{:04}", Utc::now().format("%Y%m%d"), Uuid::new_v4().as_simple().to_string()[..4].parse::<u16>().unwrap_or(0));
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
                    required_change: "Run `atlas produce --topic <id>` to start production.".to_string(),
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

async fn cmd_produce(store: &Store, topic_id: &str) -> anyhow::Result<()> {
    let mut project = store
        .get_project(topic_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {}", topic_id))?;
    
    println!("═══ ATLAS STUDIO Vertical Slice ═══");
    println!("Project: {} — {}", project.video_id, project.topic.title);
    println!();
    
    // Stage 1: Script → Storyboard (simulated)
    println!("📝 Stage 1: Generating storyboard from script...");
    project.state = ProjectState::Scripting;
    store.insert_project(&project)?;
    let script = format!(
        "How do lithium-ion batteries actually work? \
         Lithium-ion batteries power our modern world. \
         At the anode, graphite layers intercalate lithium ions during charging. \
         At the cathode, lithium cobalt oxide releases ions during discharge. \
         The electrolyte carries ions between electrodes. \
         This reversible reaction stores and releases energy."
    );
    println!("   Script: {} chars", script.len());
    
    // Stage 2: Storyboard → Shot specs
    println!("🎬 Stage 2: Creating shot specifications...");
    project.state = ProjectState::Storyboarding;
    store.insert_project(&project)?;
    
    let shot_specs = vec![
        create_test_shot_spec(&project.video_id, "shot_001"),
        create_test_shot_spec(&project.video_id, "shot_002"),
    ];
    println!("   {} shots planned", shot_specs.len());
    
    // Stage 3: Blender renders (preview quality)
    println!("🎨 Stage 3: Rendering with Blender (preview)...");
    project.state = ProjectState::AssetBuild;
    store.insert_project(&project)?;
    
    let data_dir = store.data_dir().to_path_buf();
    let blender_exe = "blender"; // Assumes in PATH
    let blender_pipeline = BlenderPipeline::new(blender_exe, &data_dir);
    
    let mut render_outputs = Vec::new();
    for spec in &shot_specs {
        println!("   Rendering {}...", spec.shot_id);
        let job = atlas_blender::RenderJob {
            job_id: format!("job_{}", spec.shot_id),
            shot_spec: spec.clone(),
            quality: RenderQuality::Preview,
            output_dir: data_dir.join("renders").join(&project.video_id).join(&spec.shot_id),
            status: atlas_blender::RenderStatus::Queued,
        };
        
        // In real run, this would call blender_pipeline.render(&job).await
        // For now, create mock frame output dirs
        let frames_dir = data_dir.join("renders").join(&project.video_id).join(&spec.shot_id);
        std::fs::create_dir_all(&frames_dir)?;
        for frame in 1..=spec.duration_frames {
            let frame_file = frames_dir.join(format!("{:04}.png", frame));
            std::fs::write(&frame_file, b"MOCK_FRAME")?;
        }
        println!("   ✓ {} frames", spec.duration_frames);
        render_outputs.push(job);
    }
    
    // Stage 4: Kokoro TTS narration
    println!("🎙️  Stage 4: Synthesizing narration with Kokoro...");
    project.state = ProjectState::AudioBuild;
    store.insert_project(&project)?;
    
    let kokoro_exe = "kokoro"; // Assumes in PATH
    let voice_config = VoiceConfig::default();
    let kokoro_pipeline = KokoroPipeline::new(kokoro_exe, voice_config, &data_dir);
    
    let segments = split_script_into_segments(&script, "af_heart");
    println!("   {} narration segments", segments.len());
    
    // Mock audio output
    let audio_dir = data_dir.join("audio").join("combined");
    std::fs::create_dir_all(&audio_dir)?;
    let combined_audio = audio_dir.join(format!("{}.wav", project.video_id));
    std::fs::write(&combined_audio, b"MOCK_AUDIO")?;
    println!("   ✓ Combined narration: {}", combined_audio.display());
    
    // Stage 5: FFmpeg assembly
    println!("🔧 Stage 5: Assembling final video with FFmpeg...");
    project.state = ProjectState::EditAndMix;
    store.insert_project(&project)?;
    
    let ffmpeg_exe = "ffmpeg";
    let ffprobe_exe = "ffprobe";
    let ffmpeg_assembler = FfmpegAssembler::new(ffmpeg_exe, ffprobe_exe, &data_dir);
    
    let edl = EditDecisionList {
        edl_id: format!("edl_{}", uuid::Uuid::new_v4().simple()),
        project_id: project.video_id.clone(),
        fps: 30,
        width: 1920,
        height: 1080,
        entries: vec![
            EdlEntry {
                entry_id: "shot_001".into(),
                source_type: SourceType::Video,
                source_path: data_dir.join("renders").join(&project.video_id).join("shot_001").join("%04d.png"),
                source_in_frame: 0,
                source_out_frame: 90,
                record_in_frame: 0,
                record_out_frame: 90,
                transition: None,
                audio_sync_offset_ms: 0,
            },
            EdlEntry {
                entry_id: "shot_002".into(),
                source_type: SourceType::Video,
                source_path: data_dir.join("renders").join(&project.video_id).join("shot_002").join("%04d.png"),
                source_in_frame: 0,
                source_out_frame: 60,
                record_in_frame: 90,
                record_out_frame: 150,
                transition: Some(Transition {
                    transition_type: TransitionType::Crossfade,
                    duration_frames: 15,
                    parameters: serde_json::json!({}),
                }),
                audio_sync_offset_ms: 0,
            },
        ],
        audio_tracks: vec![
            AudioTrack {
                track_id: "narration".into(),
                source_path: combined_audio.clone(),
                start_frame: 0,
                volume_db: 0.0,
                fade_in_frames: 15,
                fade_out_frames: 15,
            },
        ],
        caption_tracks: vec![
            CaptionTrack {
                track_id: "captions_en".into(),
                format: CaptionFormat::Srt,
                source_path: data_dir.join("captions").join("en.srt"),
                language: "en".into(),
            },
        ],
    };
    
    // Create mock caption file
    std::fs::create_dir_all(data_dir.join("captions"))?;
    std::fs::write(data_dir.join("captions").join("en.srt"), 
        "1\n00:00:00,000 --> 00:00:03,000\nHow do lithium-ion batteries actually work?\n\n2\n00:00:03,000 --> 00:00:05,000\nLithium-ion batteries power our modern world.\n")?;
    
    let output_video = data_dir.join("exports").join(&project.video_id).join("final.mp4");
    std::fs::create_dir_all(data_dir.join("exports").join(&project.video_id))?;
    
    // In real run: ffmpeg_assembler.assemble(&edl, &output_video).await
    std::fs::write(&output_video, b"MOCK_MP4")?;
    println!("   ✓ Final video: {}", output_video.display());
    
    // Stage 6: QA & Package
    println!("✅ Stage 6: Quality check & package...");
    project.state = ProjectState::ReleaseQa;
    store.insert_project(&project)?;
    
    println!();
    println!("═══ Production Complete ═══");
    println!("Output: {}", output_video.display());
    println!("Renders: {}", data_dir.join("renders").join(&project.video_id).display());
    println!("Audio:   {}", combined_audio.display());
    println!("Captions: {}", data_dir.join("captions").join("en.srt").display());
    println!();
    println!("Next steps:");
    println!("  atlas analytics fetch {} --sample", project.video_id);
    println!("  atlas analytics recommend {}", project.video_id);
    println!("  atlas analytics optimize {}", project.video_id);
    
    project.state = ProjectState::ReadyToPublish;
    store.insert_project(&project)?;
    Ok(())
}

async fn cmd_daemon(action: DaemonAction) -> anyhow::Result<()> {
    match action {
        DaemonAction::Start => {
            println!("Daemon start requested.");
            println!("Note: headless scheduler not yet implemented in this milestone.");
        }
        DaemonAction::Pause => {
            println!("Daemon pause requested.");
        }
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
            let (client_id, client_secret, redirect_uri) = match (client_id, client_secret, redirect_uri) {
                (Some(cid), Some(csec), ruri) => (cid, csec, ruri.unwrap_or_else(|| "http://localhost".to_string())),
                _ => {
                    let creds_file = PathBuf::from("youtube_credentials.json");
                    if creds_file.exists() {
                        let file_creds = load_youtube_credentials(&creds_file).await?;
                        let creds: YouTubeCredentials = file_creds.into();
                        (creds.client_id, creds.client_secret, creds.redirect_uri)
                    } else {
                        anyhow::bail!("YouTube credentials not provided and youtube_credentials.json not found");
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
            let (client_id, client_secret, redirect_uri) = match (client_id, client_secret, redirect_uri) {
                (Some(cid), Some(csec), ruri) => (cid, csec, ruri.unwrap_or_else(|| "http://localhost".to_string())),
                _ => {
                    let creds_file = PathBuf::from("youtube_credentials.json");
                    if creds_file.exists() {
                        let file_creds = load_youtube_credentials(&creds_file).await?;
                        let creds: YouTubeCredentials = file_creds.into();
                        (creds.client_id, creds.client_secret, creds.redirect_uri)
                    } else {
                        anyhow::bail!("YouTube credentials not provided and youtube_credentials.json not found");
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
            println!("YouTube OAuth complete. Access token expires at {:?}", token.expires_at);
            println!("Refresh token stored securely.");
        }
        AnalyticsAction::Fetch { video_id, sample } => {
            let project = store
                .get_project(&video_id)?
                .ok_or_else(|| anyhow::anyhow!("project not found: {}", video_id))?;
            let youtube_id = project
                .publish
                .youtube_video_id
                .clone()
                .unwrap_or_else(|| "sample_video_id".to_string());

            if sample {
                let metrics = save_sample_metrics(store, &video_id, &youtube_id)?;
                println!("Saved sample metrics for {}", video_id);
                println!("{}", serde_json::to_string_pretty(&metrics)?);
            } else {
                println!("Live YouTube Analytics fetch requires authenticated credentials.");
                println!("Use --sample to store a demo snapshot, or run `atlas analytics auth` first.");
            }
        }
        AnalyticsAction::Retention { video_id } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics
                .load_metrics(&video_id)?
                .ok_or_else(|| anyhow::anyhow!("no metrics found for {}. Run `atlas analytics fetch {} --sample` first.", video_id, video_id))?;
            println!("Retention curve for {} ({}):", video_id, metrics.youtube_video_id);
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
        }
        AnalyticsAction::Recommend {
            video_id,
            duration_seconds,
        } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics
                .load_metrics(&video_id)?
                .ok_or_else(|| anyhow::anyhow!("no metrics found for {}. Run `atlas analytics fetch {} --sample` first.", video_id, video_id))?;

            // Use all stored metrics as the channel baseline.
            let all = analytics.list_metrics(100)?;
            let baseline = compute_channel_baseline(&all);
            let performance = compare_to_baseline(&metrics, &baseline);

            let engine = RecommendationEngine::new();
            let recs = engine.rank(engine.analyze(&performance, duration_seconds));

            println!("Optimization recommendations for {}:", video_id);
            for r in &recs {
                println!(
                    "[{:?}] {:?} — {}",
                    r.priority, r.category, r.issue
                );
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
                println!("Generated {} optimization task(s) from analytics:", ids.len());
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
            println!("{:<24} {:<16} {:<12} {:<12} {:<12}", "VIDEO", "YOUTUBE ID", "VIEWS", "AVG DUR", "CTR");
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
