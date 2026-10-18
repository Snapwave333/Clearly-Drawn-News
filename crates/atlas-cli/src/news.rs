use super::*;
use atlas_core::{EpisodeDurationPolicy, SHOW_TAGLINE, SHOW_TITLE};
use atlas_media::fact_checker::{FactCheckReport, NewsFactChecker};
use atlas_media::news_desk::{AnimationNewsDesk, NewsWire, WireOrigin};
use atlas_policy::copyright::{
    CopyrightComplianceEngine, MediaClassification, MediaType, RightsRecord, ScanResult,
};
use atlas_policy::telegram::{
    ApprovalDecision, EpisodeSlateProposal, ProposedNewsStory, TelegramApprovalGate,
    notify_best_effort,
};
use atlas_storyboard::DirectorClient;
use atlas_storyboard::story_script::{NewsroomWriter, ScriptMethod, StoryBrief};
use atlas_storyboard::two_anchor_script::{MasterEpisodeScript, TwoAnchorScriptWriter};

const DEFAULT_UE_CMD: &str =
    r"C:\Program Files\Epic Games\UE_5.8\Engine\Binaries\Win64\UnrealEditor-Cmd.exe";
const DEFAULT_UE_PROJECT: &str =
    r"C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\AtlasStudioOffice.uproject";
const ANCHOR_WARDROBE: &str = include_str!("../../../unreal/wardrobe.json");
/// Clearly Drawn News only airs stories confirmed by two independent outlets. Not configurable.
const NEWS_MIN_SOURCES: usize = 2;

/// Episode runtime policy: target `ATLAS_NEWS_TARGET_MINUTES` (default 30, 10 to 60). Episodes are
/// held below `ATLAS_NEWS_MIN_MINUTES` (default target - 5) or above `ATLAS_NEWS_MAX_MINUTES`
/// (default target + 5).
pub(crate) fn news_policy(lookup: impl Fn(&str) -> Option<String>) -> EpisodeDurationPolicy {
    let minutes = |name: &str| lookup(name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|m| (5.0..=90.0).contains(m));
    let target = minutes("ATLAS_NEWS_TARGET_MINUTES").filter(|m| (10.0..=60.0).contains(m)).unwrap_or(30.0);
    let min = minutes("ATLAS_NEWS_MIN_MINUTES").filter(|m| *m <= target).unwrap_or(target - 5.0);
    let max = minutes("ATLAS_NEWS_MAX_MINUTES").filter(|m| *m >= target).unwrap_or(target + 5.0);
    EpisodeDurationPolicy { min_seconds: min * 60.0, max_seconds: max * 60.0, target_seconds: target * 60.0 }
}

/// Who signs off on a news slate before production continues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NewsApprovalMode {
    /// Unattended (default): no Telegram gate, even when a bot token is set. Telegram only gets notices.
    Auto,
    /// Wait for a human to press Approve in Telegram (`ATLAS_NEWS_APPROVAL=telegram`).
    Telegram,
}

pub(crate) fn news_approval_mode(lookup: impl Fn(&str) -> Option<String>) -> NewsApprovalMode {
    match lookup("ATLAS_NEWS_APPROVAL").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("telegram") => NewsApprovalMode::Telegram,
        _ => NewsApprovalMode::Auto,
    }
}

pub(crate) async fn cmd_news(store: &Store, action: NewsAction) -> anyhow::Result<()> {
    println!("══════════════════════════════════════════════════════════════════");
    println!("  {} — {}", SHOW_TITLE, SHOW_TAGLINE);
    println!("  {} Animation Broadcast Newsroom", atlas_core::COMPANY_NAME);
    println!("══════════════════════════════════════════════════════════════════");

    let news_desk = AnimationNewsDesk::new();
    let min_sources = NEWS_MIN_SOURCES;
    let fact_checker = NewsFactChecker::new(min_sources, true);
    let policy = news_policy(|name| std::env::var(name).ok());
    let news_dir = store.data_dir().join("news");

    match action {
        NewsAction::Research => {
            println!("\n[1/3] Ingesting developing animation industry wire...");
            let (_, report, _) = checked_wire(&news_desk, &fact_checker, &news_dir, "research").await;

            println!("\n[2/3] Fact-Checking & Corroboration Engine results:");
            println!("  Passed:   {} verified stories", report.passed_stories.len());
            println!("  Rejected: {} unverified/duplicate items", report.rejected_stories.len());
            for (story, reason) in &report.rejected_stories {
                println!("    ✗ {} — {}", story.title, reason);
            }

            println!("\n[3/3] Curated Storyline Lineup:");
            for (idx, story) in report.passed_stories.iter().enumerate() {
                println!(
                    "  {:02}. [{:?}] {} (Est: {:.1}m) — Studio: {}",
                    idx + 1,
                    story.category,
                    story.title,
                    story.estimated_duration_mins,
                    story.studio_or_organization
                );
            }
            println!(
                "\nEstimated Full Episode Runtime: {:.1} minutes (Target: {:.0} min, {:.0} to {:.0} allowed)",
                report.estimated_total_minutes,
                policy.target_seconds / 60.0,
                policy.min_seconds / 60.0,
                policy.max_seconds / 60.0
            );
        }
        NewsAction::Slate => {
            let (origin, report, _) = checked_wire(&news_desk, &fact_checker, &news_dir, "slate").await;
            let (episode_id, _) = next_episode(&news_dir);
            let title = episode_title(&episode_id, &report);
            println!("\nWriting the two-anchor broadcast script from {} verified stories...", report.passed_stories.len());
            let (episode, provenance) = draft_script(origin, &report, &[], &title, &policy).await;

            println!("\nEpisode Slate: {}", episode.episode_title);
            println!("Script method: {}", provenance["method"].as_str().unwrap_or("unknown"));
            let compliance = match episode.validate_duration(&policy) {
                Ok(()) => format!("Compliant with the {:.0}–{:.0} min policy", policy.min_seconds / 60.0, policy.max_seconds / 60.0),
                Err(error) => format!("NOT COMPLIANT: {error}"),
            };
            println!(
                "Total Runtime: {:.1}s ({:.1} min) [{}]",
                episode.total_duration_secs,
                episode.total_duration_secs / 60.0,
                compliance
            );
            println!("Segments ({} total):", episode.segments.len());
            for seg in &episode.segments {
                println!(
                    "  Segment {:02}: {:<42} | Category: {:<18} | Duration: {:>5.1}s | Dialogue Lines: {}",
                    seg.segment_number,
                    seg.segment_title,
                    seg.segment_category,
                    seg.actual_duration_secs,
                    seg.lines.len()
                );
            }
        }
        NewsAction::Telegram { token, chat_id } => {
            let mut gate = TelegramApprovalGate::new(token, chat_id).map_err(anyhow::Error::msg)?;
            let (_, report, _) = checked_wire(&news_desk, &fact_checker, &news_dir, "telegram").await;
            let (episode_id, episode_number) = next_episode(&news_dir);

            println!("\nConnecting to Telegram Bot API...");
            let bot = gate.get_bot_info().await.map_err(anyhow::Error::msg)?;
            println!(
                "Connected to bot: @{} ({})",
                bot["username"].as_str().unwrap_or("bot"),
                bot["first_name"].as_str().unwrap_or("Bot")
            );

            let proposal = slate_proposal(&episode_id, episode_number, &report);
            println!("\nTransmitting production approval slate to Telegram...");
            let msg_id = gate.send_slate(&proposal).await.map_err(anyhow::Error::msg)?;
            println!("Slate delivered! Message ID: {}", msg_id);
            println!("Awaiting human approval inline button click in Telegram...");
            let decision = gate
                .poll_approval(&episode_id, 60)
                .await
                .map_err(anyhow::Error::msg)?;
            println!("Received Telegram Approval Gate Decision: {:?}", decision);
        }
        NewsAction::Animatic { episode } => {
            let episode_dir = news_dir.join(&episode);
            let script: MasterEpisodeScript = serde_json::from_slice(&std::fs::read(episode_dir.join("script.json"))?)?;
            let preview = news_preview::render_animatic(&episode_dir, &script).await?;
            println!("\nPreview animatic: {} ({:.1} min)", preview.path.display(), preview.duration_seconds / 60.0);
        }
        NewsAction::Health { runs } => {
            news_health::print_health(&news_dir, runs.max(1));
        }
        NewsAction::Copyright => {
            println!("\nRunning Dedicated Copyright Compliance Agent Scanner...");
            let engine = CopyrightComplianceEngine::new();
            let manifest = newsroom_rights_manifest();
            for rec in &manifest {
                println!(
                    "  [REGISTERED] {:<26} | {:<20} | Allowed in render: {} | Audio Muted: {}",
                    rec.asset_id,
                    format!("{:?}", rec.classification),
                    engine.is_cleared_for_render(rec),
                    rec.audio_muted_for_clearance
                );
            }
            match engine.scan_production_manifest(&manifest) {
                ScanResult::AllClear => {
                    println!("\nPre-Render Publication Firewall Check: PASS");
                    println!("All third-party video trailer excerpts have original audio MUTED to prevent Content ID strikes.");
                }
                res => println!("\nScan Result: {:?}", res),
            }
        }
        NewsAction::YoutubeCheck => news_publish::youtube_check().await?,
        NewsAction::AccountsCheck => accounts_check().await?,
        NewsAction::DirectorCheck => director_check(),
        NewsAction::TiktokCheck => crate::news_tiktok::tiktok_check().await?,
        NewsAction::TiktokAuth => crate::news_tiktok::tiktok_auth()?,
        NewsAction::TiktokAuthComplete { redirect } => crate::news_tiktok::tiktok_auth_complete(&redirect).await?,
        NewsAction::Publish { episode, dry_run, override_hold } => {
            let episode_dir = news_dir.join(&episode);
            if !episode_dir.join("status.json").is_file() {
                anyhow::bail!("no episode package at {}", episode_dir.display());
            }
            news_publish::publish_episode(store, &episode_dir, news_publish::PublishRequest { dry_run, override_hold }).await?;
        }
        NewsAction::Produce {
            token,
            chat_id,
            skip_telegram_wait,
            dry_run_publish,
        } => {
            let (episode_id, episode_number) = next_episode(&news_dir);
            let episode_dir = news_dir.join(&episode_id);
            std::fs::create_dir_all(&episode_dir)?;
            let mut timer = StageTimer::new();
            println!("\nEpisode: {} → {}", episode_id, episode_dir.display());

            timer.begin("research");
            println!("\n[PHASE 1: RESEARCH & FACT-CHECK]");
            let (wire_origin, mut report, wire_stories) = checked_wire(&news_desk, &fact_checker, &news_dir, "produce").await;
            if wire_origin == WireOrigin::LiveFeeds {
                let dropped = news_aired::drop_already_aired(&news_dir, &mut report, Utc::now());
                if dropped > 0 {
                    println!("  Skipped {dropped} stories already aired in the last 7 days.");
                }
            }
            if report.passed_stories.is_empty() {
                notify_best_effort(&format!(
                    "⚠️ {SHOW_TITLE} {episode_id}: no fresh stories confirmed by {min_sources} sources; nothing produced."
                ))
                .await;
                anyhow::bail!("Production halted: no fresh stories passed fact-checking");
            }
            println!(
                "  Curated {} verified stories ({} rejected).",
                report.passed_stories.len(),
                report.rejected_stories.len()
            );

            // News first; animation-history retrospectives fill the rest of the runtime.
            let retros_wanted = if wire_origin == WireOrigin::LiveFeeds {
                atlas_storyboard::story_script::retros_needed(report.passed_stories.len(), &policy)
            } else {
                0
            };
            let today_local = chrono::Local::now().date_naive();
            let throwback_guest = thursday_throwback_guest(today_local);
            let retro_research = if retros_wanted > 0 {
                println!("\n[PHASE 1b: RETROSPECTIVES ({retros_wanted} needed to reach the runtime target)]");
                news_retro::research_retros(&news_dir, retros_wanted, today_local, throwback_guest.as_deref()).await
            } else {
                news_retro::RetroResearch { retros: Vec::new(), rejected: Vec::new() }
            };
            let retros = retro_research.retros.clone();
            if retros.len() < retros_wanted {
                eprintln!("  ⚠ Only {} of {retros_wanted} retrospectives could be grounded; the runtime hold may apply.", retros.len());
            }
            write_json(&episode_dir.join("fact_check.json"), &fact_check_record(&report, &wire_stories, &retro_research))?;

            timer.begin("approval");
            let mode = if skip_telegram_wait {
                NewsApprovalMode::Auto
            } else {
                news_approval_mode(|name| std::env::var(name).ok())
            };
            println!("\n[PHASE 2: EDITORIAL APPROVAL ({mode:?})]");
            let proposal = slate_proposal(&episode_id, episode_number, &report);
            write_json(&episode_dir.join("slate.json"), &proposal)?;
            let (approval, approved) = if mode == NewsApprovalMode::Auto {
                println!("  Auto-approved: {} stories passed {min_sources}-source fact-checking (Telegram gets a notice only).", report.passed_stories.len());
                notify_best_effort(&format!(
                    "🎬 {SHOW_TITLE} {episode_id} production started: {} stories ({}).",
                    report.passed_stories.len(),
                    report.passed_stories.iter().map(|s| s.title.as_str()).collect::<Vec<_>>().join(" | ")
                ))
                .await;
                (serde_json::json!({ "status": "auto", "mode": mode, "at": Utc::now() }), true)
            } else {
                let mut gate =
                    TelegramApprovalGate::new(token, chat_id).map_err(anyhow::Error::msg)?;
                println!("  Transmitting slate to Telegram for editorial approval...");
                gate.send_slate(&proposal).await.map_err(anyhow::Error::msg)?;
                println!("  Waiting for human approval from Telegram...");
                let decision = gate
                    .poll_approval(&episode_id, 30)
                    .await
                    .map_err(anyhow::Error::msg)?;
                let approved = matches!(decision, ApprovalDecision::Approved { .. });
                (
                    serde_json::json!({ "status": "decided", "decision": format!("{decision:?}"), "at": Utc::now() }),
                    approved,
                )
            };
            write_json(&episode_dir.join("approval.json"), &approval)?;
            if !approved {
                println!("Production halted: Telegram approval gate did not approve {episode_id}");
                return Ok(());
            }

            timer.begin("script");
            println!("\n[PHASE 3: SCRIPT & DIALOGUE GENERATION]");
            let title = episode_title(&episode_id, &report);
            let (episode, provenance) = draft_script(wire_origin, &report, &retros, &title, &policy).await;
            write_json(&episode_dir.join("script.json"), &episode)?;
            write_json(&episode_dir.join("script_provenance.json"), &provenance)?;
            println!("  Script method: {}", provenance["method"].as_str().unwrap_or("unknown"));
            if let Err(error) = episode.validate_duration(&policy) {
                let hint = if provenance["method"] == "director" {
                    ""
                } else {
                    " Set ATLAS_DIRECTOR_URL/ATLAS_DIRECTOR_API_KEY (Anthropic) or NVIDIA_API_KEY so the director writes full-length segments from live stories."
                };
                eprintln!("  ⚠ Script runtime {:.1} min: {error}.{hint}", episode.total_duration_secs / 60.0);
            }
            println!(
                "  {}: {} segments totaling {:.1}s ({:.1} min).",
                episode.episode_title,
                episode.segments.len(),
                episode.total_duration_secs,
                episode.total_duration_secs / 60.0
            );

            timer.begin("rights");
            println!("\n[PHASE 4: COPYRIGHT & RIGHTS REGISTRY]");
            let manifest = newsroom_rights_manifest();
            write_json(&episode_dir.join("rights_manifest.json"), &manifest)?;
            std::fs::write(episode_dir.join("wardrobe.json"), ANCHOR_WARDROBE)?;
            let local_date = chrono::Local::now().format("%Y-%m-%d").to_string();
            match CopyrightComplianceEngine::new().scan_production_manifest(&manifest) {
                ScanResult::AllClear => println!(
                    "  {} assets scanned: all clear; third-party trailer audio muted.",
                    manifest.len()
                ),
                blocked => {
                    let reason = format!("rights check is not all clear: {blocked:?}");
                    println!("  HELD — {reason}");
                    write_json(
                        &episode_dir.join("status.json"),
                        &serde_json::json!({
                            "episode_id": episode_id,
                            "title": episode.episode_title,
                            "status": "held",
                            "held": true,
                            "hold_reasons": [reason],
                            "rights_clear": false,
                            "wire_origin": wire_origin,
                            "local_date": local_date,
                        }),
                    )?;
                    notify_best_effort(&format!("⏸ {SHOW_TITLE} {episode_id} is HELD before voicing: {reason}")).await;
                    return Ok(());
                }
            }

            timer.begin("voices");
            println!("\n[PHASE 5: ANCHOR VOICE SYNTHESIS]");
            let voices = news_voice::voice_episode(store.data_dir(), &episode_dir, &episode_id, &episode).await?;
            println!("  {}", voices.summary());
            if let news_voice::VoiceOutcome::Voiced { narration, .. } = &voices {
                println!("  Narration: {}", narration.display());
            }
            let animatic = if voices.is_voiced() {
                timer.begin("animatic");
                match news_preview::render_animatic(&episode_dir, &episode).await {
                    Ok(preview) => {
                        println!("  Preview animatic: {} ({:.1} min)", preview.path.display(), preview.duration_seconds / 60.0);
                        Some(preview.path)
                    }
                    Err(error) => {
                        eprintln!("  Preview animatic skipped: {error}");
                        None
                    }
                }
            } else {
                None
            };

            timer.begin("filming");
            println!("\n[PHASE 6: UNREAL ENGINE 5.8 VIRTUAL STUDIO FILMING]");
            let filming = match film_in_unreal(&episode_dir, voices.is_voiced()).await {
                Ok(outcome) => outcome,
                Err(error) => FilmingOutcome::Skipped(format!("failed: {error}")),
            };
            println!("  {}", filming.summary());

            let timings = timer.finish();
            write_json(&episode_dir.join("stage_timings.json"), &timings)?;
            let (runtime_seconds, runtime_source) = match (&filming, &voices) {
                (FilmingOutcome::Filmed { duration_seconds, .. }, _) => (*duration_seconds, "final_video"),
                (_, news_voice::VoiceOutcome::Voiced { total_seconds, .. }) => (f64::from(*total_seconds), "synthesized_audio"),
                _ => (episode.total_duration_secs, "script_estimate"),
            };
            let runtime_check = policy.validate(runtime_seconds);
            if let Err(problem) = &runtime_check {
                eprintln!("  ⚠ Runtime check: {problem}");
            }
            let status = match (filming.is_filmed(), voices.is_voiced(), runtime_check.is_ok()) {
                (true, _, true) => "FILMED — final/episode.mp4 ready for review and fine-tuning",
                (true, _, false) => "FILMED — runtime outside the runtime policy (ATLAS_NEWS_TARGET_MINUTES); needs a different script length",
                (false, true, false) => "VOICED — runtime outside the runtime policy (ATLAS_NEWS_TARGET_MINUTES); needs a different script length",
                (false, true, true) => "VOICED — Unreal filming pending",
                (false, false, _) => "SCRIPT PACKAGE READY — voices and Unreal filming pending",
            };
            write_json(
                &episode_dir.join("status.json"),
                &serde_json::json!({
                    "episode_id": episode_id,
                    "title": episode.episode_title,
                    "status": status,
                    "voices": voices.summary(),
                    "preview_animatic": animatic,
                    "filming": filming.summary(),
                    "runtime_seconds": runtime_seconds,
                    "runtime_source": runtime_source,
                    "final_video": match &filming {
                        FilmingOutcome::Filmed { video, .. } => Some(video.clone()),
                        FilmingOutcome::Skipped(_) => None,
                    },
                    "render": match &filming {
                        FilmingOutcome::Filmed { render_seconds, duration_seconds, .. } => serde_json::json!({
                            "render_seconds": render_seconds,
                            "realtime_factor": render_seconds / duration_seconds.max(1.0),
                            "estimated_episodes_per_day": daily_capacity(*render_seconds),
                        }),
                        FilmingOutcome::Skipped(_) => serde_json::Value::Null,
                    },
                    "unreal": match &filming {
                        FilmingOutcome::Filmed { handoff, .. } => handoff.clone(),
                        FilmingOutcome::Skipped(_) => serde_json::Value::Null,
                    },
                    "runtime_compliant": runtime_check.is_ok(),
                    "rights_clear": true,
                    "local_date": local_date,
                    "approval_mode": mode,
                    "wire_origin": wire_origin,
                    "script_method": provenance["method"],
                    "company": atlas_core::COMPANY_NAME,
                }),
            )?;

            // A dry run never uses up stories or retrospective topics.
            if wire_origin == WireOrigin::LiveFeeds && !dry_run_publish {
                news_aired::record_aired(&news_dir, &episode_id, &report.passed_stories, Utc::now())?;
                news_retro::record_aired(&news_dir, &episode_id, &retros, Utc::now())?;
            }

            let request = news_publish::PublishRequest { dry_run: dry_run_publish, override_hold: false };
            if let Err(error) = news_publish::publish_episode(store, &episode_dir, request).await {
                eprintln!("  Publish stage failed: {error}");
                notify_best_effort(&format!(
                    "❌ {SHOW_TITLE} {episode_id} publish failed: {error}\nRetry: atlas news publish --episode {episode_id}"
                ))
                .await;
            }

            println!("\n══════════════════════════════════════════════════════════════════");
            println!("  {} — {}", SHOW_TITLE, episode.episode_title);
            println!(
                "  Runtime: {:.1} minutes | Status: {}",
                runtime_seconds / 60.0,
                status
            );
            println!("  Package: {}", episode_dir.display());
            println!("══════════════════════════════════════════════════════════════════\n");
        }
    }

    Ok(())
}

enum FilmingOutcome {
    Filmed {
        video: PathBuf,
        duration_seconds: f64,
        render_seconds: f64,
        handoff: serde_json::Value,
    },
    Skipped(String),
}

impl FilmingOutcome {
    fn is_filmed(&self) -> bool {
        matches!(self, Self::Filmed { .. })
    }

    fn summary(&self) -> String {
        match self {
            Self::Filmed { video, duration_seconds, render_seconds, .. } => format!(
                "Unreal Engine episode rendered: {} ({:.1} min of video in {:.1} min, {:.1}x real time; about {} episodes/day fit on this PC)",
                video.display(),
                duration_seconds / 60.0,
                render_seconds / 60.0,
                render_seconds / duration_seconds.max(1.0),
                daily_capacity(*render_seconds)
            ),
            Self::Skipped(reason) => format!("Unreal Engine filming skipped: {reason}"),
        }
    }
}

fn daily_capacity(render_seconds: f64) -> u32 {
    let per_episode = render_seconds * 1.15 + 600.0;
    (86_400.0 / per_episode).floor() as u32
}

fn unreal_scripts_dir() -> PathBuf {
    std::env::var("ATLAS_UNREAL_SCRIPTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../unreal/Content/Python"))
}

fn editor_executable(ue_cmd: &Path) -> PathBuf {
    if let Ok(explicit) = std::env::var("ATLAS_UE_EDITOR") {
        return PathBuf::from(explicit);
    }
    let windowed = ue_cmd.with_file_name(
        ue_cmd
            .file_name()
            .map(|name| name.to_string_lossy().replace("-Cmd", ""))
            .unwrap_or_default(),
    );
    if windowed.is_file() { windowed } else { ue_cmd.to_path_buf() }
}

fn env_number(name: &str, default: u32) -> u32 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).filter(|v| *v > 0).unwrap_or(default)
}

async fn film_in_unreal(episode_dir: &Path, voiced: bool) -> anyhow::Result<FilmingOutcome> {
    if !voiced {
        return Ok(FilmingOutcome::Skipped("needs synthesized anchor voices first".into()));
    }
    let ue_cmd = PathBuf::from(std::env::var("ATLAS_UE_CMD").unwrap_or_else(|_| DEFAULT_UE_CMD.into()));
    let uproject = PathBuf::from(
        std::env::var("ATLAS_UE_PROJECT").unwrap_or_else(|_| DEFAULT_UE_PROJECT.into()),
    );
    if !ue_cmd.is_file() {
        return Ok(FilmingOutcome::Skipped(format!(
            "editor not found at {} (set ATLAS_UE_CMD)",
            ue_cmd.display()
        )));
    }
    if !uproject.is_file() {
        return Ok(FilmingOutcome::Skipped(format!(
            "project not found at {} (set ATLAS_UE_PROJECT)",
            uproject.display()
        )));
    }
    let script = unreal_scripts_dir().join("clearly_drawn_produce.py");
    if !script.is_file() {
        return Ok(FilmingOutcome::Skipped(format!(
            "production script not found at {} (set ATLAS_UNREAL_SCRIPTS)",
            script.display()
        )));
    }

    let render_dir = episode_dir.join("render");
    let frames_dir = render_dir.join("frames");
    let handoff_path = render_dir.join("handoff.json");
    if frames_dir.is_dir() {
        std::fs::remove_dir_all(&frames_dir)?;
    }
    std::fs::create_dir_all(&frames_dir)?;
    let _ = std::fs::remove_file(&handoff_path);

    let editor = editor_executable(&ue_cmd);
    let (width, height) = (env_number("ATLAS_RENDER_WIDTH", 1920), env_number("ATLAS_RENDER_HEIGHT", 1080));
    let preset = std::env::var("ATLAS_RENDER_PRESET")
        .ok()
        .filter(|p| matches!(p.as_str(), "fast" | "balanced" | "quality"))
        .unwrap_or_else(|| "fast".into());
    println!("  Launching {} to dress the anchors, build the sequence, and render {width}x{height} ({preset} preset) with Movie Render Queue...", editor.display());
    println!("  Leave the Unreal window open; it closes itself when the render finishes.");
    let mut command = tokio::process::Command::new(&editor);
    command
        .arg(&uproject)
        .arg(format!("-ExecutePythonScript={}", script.display()))
        .args(["-unattended", "-nosplash", "-log"])
        .env("CLEARLY_DRAWN_EPISODE", episode_dir)
        .env("CLEARLY_DRAWN_FRAMES", &frames_dir)
        .env("CLEARLY_DRAWN_HANDOFF", &handoff_path)
        .env("CLEARLY_DRAWN_WIDTH", width.to_string())
        .env("CLEARLY_DRAWN_HEIGHT", height.to_string())
        .env("CLEARLY_DRAWN_RENDER_PRESET", &preset)
        .kill_on_drop(true);
    if let Ok(map) = std::env::var("ATLAS_UE_MAP") {
        command.env("CLEARLY_DRAWN_MAP", map);
    }
    let render_started = std::time::Instant::now();
    let status = command.status().await?;
    let render_seconds = render_started.elapsed().as_secs_f64();

    let handoff: serde_json::Value = std::fs::read(&handoff_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| anyhow::anyhow!("Unreal exited ({status}) without a production handoff; check the project's Saved/Logs"))?;
    if handoff["ok"] != true {
        anyhow::bail!(
            "Unreal production failed: {}",
            handoff["error"].as_str().unwrap_or("unknown error (see render/handoff.json)")
        );
    }
    if let Some(lipsync) = handoff["lipsync"].as_str() {
        println!("  Lip sync: {lipsync}");
    }

    let frames = std::fs::read_dir(&frames_dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".png"))
        .count();
    if frames == 0 {
        anyhow::bail!("Movie Render Queue finished but wrote no frames to {}", frames_dir.display());
    }
    println!("  ✓ {frames} rendered frames; muxing with the anchor narration...");

    let final_dir = episode_dir.join("final");
    std::fs::create_dir_all(&final_dir)?;
    let video = final_dir.join("episode.mp4");
    let fps = env_number("ATLAS_RENDER_FPS", 30).to_string();
    let muxed = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-framerate", &fps, "-i"])
        .arg(frames_dir.join("frame.%06d.png"))
        .arg("-i")
        .arg(episode_dir.join("audio").join("episode_narration.wav"))
        .args(["-c:v", "libx264", "-preset", "slow", "-crf", "18", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart"])
        .arg(&video)
        .kill_on_drop(true)
        .output()
        .await?;
    if !muxed.status.success() || !video.is_file() {
        anyhow::bail!("FFmpeg could not assemble the episode: {}", String::from_utf8_lossy(&muxed.stderr).trim());
    }
    let probe = tokio::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(&video)
        .output()
        .await?;
    let duration_seconds = String::from_utf8_lossy(&probe.stdout).trim().parse::<f64>().unwrap_or(0.0);
    Ok(FilmingOutcome::Filmed { video, duration_seconds, render_seconds, handoff })
}

fn outlet_name(url: &str) -> Option<String> {
    let host = url.split("//").nth(1)?.split('/').next()?.trim_start_matches("www.");
    let known = atlas_media::news_wire::FEEDS
        .iter()
        .find(|feed| feed.url.contains(host))
        .map(|feed| feed.name.to_string());
    Some(known.unwrap_or_else(|| host.to_string()))
}

fn story_briefs(report: &FactCheckReport) -> Vec<StoryBrief> {
    report
        .passed_stories
        .iter()
        .map(|story| StoryBrief {
            headline: story.title.clone(),
            summary: story.factual_summary.clone(),
            category: story.category.as_str().to_string(),
            organization: story.studio_or_organization.clone(),
            outlets: if story.sourcing.credits.is_empty() {
                std::iter::once(story.primary_source_url.as_str())
                    .chain(story.secondary_source_url.as_deref())
                    .filter_map(outlet_name)
                    .collect()
            } else {
                story.sourcing.credits.clone()
            },
            region: story.sourcing.region.clone(),
            official_announcement: story.sourcing.official_primary.clone(),
            original_headline: story.sourcing.original_headline.clone(),
            retrospective: None,
        })
        .collect()
}

/// Thursday Throwback guest booked for today (casting CSV), so a retrospective can lead into it.
fn thursday_throwback_guest(today: chrono::NaiveDate) -> Option<String> {
    if chrono::Datelike::weekday(&today) != chrono::Weekday::Thu {
        return None;
    }
    let csv = std::fs::read_to_string(news_publish::casting_csv_path()).ok()?;
    let date = today.format("%Y-%m-%d").to_string();
    news_publish::parse_bookings(&csv).into_iter().find(|b| b.date == date).map(|b| b.character)
}

/// fact_check.json: the fact-check report plus the attribution evidence and outlets of every
/// passed story and the full citation record of every retrospective.
fn fact_check_record(
    report: &FactCheckReport,
    stories: &[atlas_media::news_wire::WireStory],
    retros: &news_retro::RetroResearch,
) -> serde_json::Value {
    let mut record = serde_json::to_value(report).unwrap_or_default();
    record["min_sources"] = serde_json::json!(NEWS_MIN_SOURCES);
    record["attribution"] = serde_json::json!(report
        .passed_stories
        .iter()
        .filter_map(|passed| stories.iter().find(|s| s.item.id == passed.id))
        .map(|s| serde_json::json!({
            "story_id": s.item.id,
            "title": s.item.title,
            "organization": s.item.studio_or_organization,
            "evidence": s.attribution_evidence,
            "method": s.attribution_method,
            "outlets": s.outlets,
            "source_urls": s.source_urls,
        }))
        .collect::<Vec<_>>());
    record["retrospectives"] = serde_json::json!(retros.retros);
    record["retrospectives_rejected"] = serde_json::json!(retros
        .rejected
        .iter()
        .map(|(topic, reason)| serde_json::json!({ "topic": topic, "reason": reason }))
        .collect::<Vec<_>>());
    record
}

async fn draft_script(
    origin: WireOrigin,
    report: &FactCheckReport,
    retros: &[news_retro::Retrospective],
    title: &str,
    policy: &EpisodeDurationPolicy,
) -> (MasterEpisodeScript, serde_json::Value) {
    if origin == WireOrigin::CuratedFallback {
        let script = TwoAnchorScriptWriter::new().generate_broadcast_episode(title, &today());
        return (script, serde_json::json!({ "method": "curated", "note": "hand-written script for the curated fallback stories" }));
    }
    let mut briefs = story_briefs(report);
    briefs.extend(retros.iter().map(news_retro::Retrospective::brief));
    let writer = NewsroomWriter::new();
    let drafted = match DirectorClient::from_env() {
        Some(director) => writer.director_episode(&director, title, &today(), &briefs, policy).await,
        None => writer.template_episode(title, &today(), &briefs, policy),
    };
    for error in &drafted.director_errors {
        eprintln!("  Director fell back to template: {error}");
    }
    let director_segments = drafted
        .segment_methods
        .iter()
        .filter(|method| **method == ScriptMethod::Director)
        .count();
    let method = match director_segments {
        0 => "template",
        n if n == drafted.segment_methods.len() => "director",
        _ => "director_with_template_fallbacks",
    };
    let provenance = serde_json::json!({
        "method": method,
        "segment_methods": drafted.segment_methods,
        "director_errors": drafted.director_errors,
        "briefs": briefs,
    });
    (drafted.script, provenance)
}

async fn checked_wire(
    news_desk: &AnimationNewsDesk,
    fact_checker: &NewsFactChecker,
    news_dir: &Path,
    command: &str,
) -> (WireOrigin, FactCheckReport, Vec<atlas_media::news_wire::WireStory>) {
    let skip = news_health::feeds_to_skip(news_dir, Utc::now());
    for (feed, retry) in &skip {
        println!("  Skipping {feed}: failed repeatedly; next retry after {}", retry.format("%Y-%m-%d %H:%M UTC"));
    }
    let skip = skip.into_iter().map(|(feed, _)| feed).collect::<Vec<_>>();
    let mut wire = load_wire(news_desk, &skip, news_dir).await;
    println!("  Found {} candidate industry stories.", wire.items.len());
    if wire.origin == WireOrigin::LiveFeeds {
        let by_text = wire.stories.iter().filter(|s| !s.item.studio_or_organization.is_empty()).count();
        let by_director = news_attribution::attribute_missing(&mut wire.stories).await;
        println!(
            "  Attribution: {by_text} from feed text, {by_director} quoted by the director, {} still unattributed.",
            wire.stories.iter().filter(|s| s.item.studio_or_organization.is_empty()).count()
        );
        wire.items = wire.stories.iter().map(|s| s.item.clone()).collect();
        let corroborated = wire.stories.iter().filter(|s| s.item.sourcing.independent_sources >= NEWS_MIN_SOURCES).count();
        let official = wire.stories.iter().filter(|s| s.item.sourcing.official_primary.is_some()).count();
        println!(
            "  Corroborated by {NEWS_MIN_SOURCES}+ independent sources: {corroborated} of {}; official studio announcements: {official}.",
            wire.stories.len()
        );
        let mut regions: Vec<(String, usize)> = Vec::new();
        for story in &wire.stories {
            match regions.iter_mut().find(|(r, _)| *r == story.item.sourcing.region) {
                Some((_, n)) => *n += 1,
                None => regions.push((story.item.sourcing.region.clone(), 1)),
            }
        }
        println!("  Regions: {}", regions.iter().map(|(r, n)| format!("{r} {n}")).collect::<Vec<_>>().join(", "));
    }
    let report = fact_checker.verify_candidates(wire.items);
    if let Err(error) = news_health::record(news_dir, command, wire.origin, &wire.feeds, &report) {
        eprintln!("  Wire health log not updated: {error}");
    }
    (wire.origin, report, wire.stories)
}

/// `atlas news accounts-check`: every pinned official account, re-verified against its platform.
async fn accounts_check() -> anyhow::Result<()> {
    let accounts = atlas_media::news_social::load_accounts(|n| std::env::var(n).ok());
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?;
    println!("\nOfficial accounts followed: {} (keyless: YouTube channel RSS, Bluesky public AppView)", accounts.len());
    let checks = atlas_media::news_social::check_accounts(&client, &accounts).await;
    for check in &checks {
        println!("  {} {} — {}", if check.ok { "✓" } else { "✗" }, check.account, check.detail);
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    println!("\n{} verified, {failed} failed. X is not used; Instagram, TikTok and Threads need logins or app tokens and are not used.", checks.len() - failed);
    if failed > 0 {
        anyhow::bail!("{failed} official account(s) failed verification");
    }
    Ok(())
}

/// `atlas news director-check`: the failover chain and which keys exist (never their values).
fn director_check() {
    let has = |name: &str| std::env::var(name).is_ok_and(|v| !v.trim().is_empty());
    println!("\nDirector keys: ANTHROPIC_API_KEY {}, NVIDIA_API_KEY {}, ATLAS_DIRECTOR_API_KEY {}",
        if has("ANTHROPIC_API_KEY") { "set" } else { "missing" },
        if has("NVIDIA_API_KEY") { "set" } else { "missing" },
        if has("ATLAS_DIRECTOR_API_KEY") { "set" } else { "not set" });
    match DirectorClient::from_env() {
        Some(director) => println!("Director chain: {}", director.chain()),
        None => println!("Director chain: none (no key); scripts use the template writer and non-English entries are skipped."),
    }
    if !has("NVIDIA_API_KEY") {
        println!("No NVIDIA NIM fallback: when the Claude credits run out the director stops (set NVIDIA_API_KEY).");
    }
}

async fn load_wire(news_desk: &AnimationNewsDesk, skip: &[String], news_dir: &Path) -> NewsWire {
    let (mut entries, feeds) = news_desk.fetch_entries(skip).await;
    let official = feeds.iter().filter(|f| f.source.contains(" (YouTube ") || f.source.contains(" (Bluesky ")).count();
    println!("  Following {official} official studio/network accounts (YouTube RSS, Bluesky).");
    let (translated, dropped) = news_translate::translate_entries(&mut entries, news_dir, news_desk.max_age_days()).await;
    if translated + dropped > 0 {
        println!("  Translated {translated} non-English entries with the director; {dropped} dropped (could not translate).");
    }
    let wire = news_desk.wire_from_entries(entries, feeds).await;
    for feed in &wire.feeds {
        if let Some(error) = &feed.error {
            eprintln!("  Feed unavailable: {error}");
        }
    }
    match wire.origin {
        WireOrigin::LiveFeeds => println!("  Wire source: live feeds ({} stories)", wire.items.len()),
        WireOrigin::CuratedFallback => println!("  Wire source: CURATED FALLBACK — no live feed stories were reachable"),
    }
    wire
}

fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn next_episode(news_dir: &Path) -> (String, u32) {
    let latest = std::fs::read_dir(news_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()?
                .strip_prefix("EP-")?
                .parse::<u32>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    let number = latest + 1;
    (format!("EP-{number:03}"), number)
}

fn episode_title(episode_id: &str, report: &FactCheckReport) -> String {
    let headlines = report
        .passed_stories
        .iter()
        .take(2)
        .map(|story| story.studio_or_organization.as_str())
        .collect::<Vec<_>>();
    match headlines.as_slice() {
        [] => format!("{episode_id}: Animation News Roundup"),
        [only] => format!("{episode_id}: {only} Headlines"),
        [first, second, ..] => format!("{episode_id}: {first} & {second} Headlines"),
    }
}

fn slate_proposal(
    episode_id: &str,
    episode_number: u32,
    report: &FactCheckReport,
) -> EpisodeSlateProposal {
    EpisodeSlateProposal {
        episode_id: episode_id.into(),
        episode_number,
        target_duration_mins: report.estimated_total_minutes,
        stories: report
            .passed_stories
            .iter()
            .map(|s| ProposedNewsStory {
                story_id: s.id.clone(),
                category: s.category.as_str().to_string(),
                headline: s.title.clone(),
                factual_summary: s.factual_summary.clone(),
                source_url: s.primary_source_url.clone(),
                secondary_source_url: s.secondary_source_url.clone(),
                announcement_date: s.announcement_date.clone(),
                estimated_duration_mins: s.estimated_duration_mins,
                editorial_angle: s.editorial_angle.clone(),
                verified: s.is_verified,
            })
            .collect(),
    }
}

fn newsroom_rights_manifest() -> Vec<RightsRecord> {
    vec![
        RightsRecord {
            asset_id: "ue_newsroom_stage".into(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            media_type: MediaType::OriginalUnrealFootage,
            creator_or_holder: atlas_core::COMPANY_NAME.into(),
            source_url: None,
            acquisition_timestamp: Utc::now(),
            license_type: "Epic Games UE5 License".into(),
            commercial_use_permitted: true,
            restrictions: vec![],
            classification: MediaClassification::Original,
            fair_use_assessment: None,
            audio_muted_for_clearance: false,
        },
        RightsRecord {
            asset_id: "mh_anchors_marcus_elena".into(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            media_type: MediaType::OriginalMetaHuman,
            creator_or_holder: "Epic MetaHuman Creator".into(),
            source_url: None,
            acquisition_timestamp: Utc::now(),
            license_type: "MetaHuman Commercial License".into(),
            commercial_use_permitted: true,
            restrictions: vec![],
            classification: MediaClassification::Original,
            fair_use_assessment: None,
            audio_muted_for_clearance: false,
        },
        RightsRecord {
            asset_id: "mappa_csm_teaser_clip".into(),
            sha256: "1111111111111111111111111111111111111111111111111111111111111111".into(),
            media_type: MediaType::Trailer,
            creator_or_holder: "MAPPA / Shueisha".into(),
            source_url: Some("https://mappa.co.jp".into()),
            acquisition_timestamp: Utc::now(),
            license_type: "Fair Use Commentary § 107".into(),
            commercial_use_permitted: true,
            restrictions: vec!["Trailer audio MUST be muted to prevent Content ID claims".into()],
            classification: MediaClassification::ExceptionReviewed,
            fair_use_assessment: None,
            audio_muted_for_clearance: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episodes_number_after_the_latest_package() {
        let dir = std::env::temp_dir().join(format!("atlas_news_{}", uuid::Uuid::new_v4().simple()));
        assert_eq!(next_episode(&dir), ("EP-001".into(), 1));
        for name in ["EP-001", "EP-007", "notes"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        assert_eq!(next_episode(&dir), ("EP-008".into(), 8));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn approval_is_automatic_unless_telegram_is_chosen() {
        assert_eq!(news_approval_mode(|_| None), NewsApprovalMode::Auto);
        assert_eq!(news_approval_mode(|_| Some("auto".into())), NewsApprovalMode::Auto);
        assert_eq!(news_approval_mode(|_| Some(" Telegram ".into())), NewsApprovalMode::Telegram);
        assert_eq!(news_approval_mode(|_| Some("whatever".into())), NewsApprovalMode::Auto);
    }

    #[test]
    fn news_requires_two_sources() {
        assert_eq!(NEWS_MIN_SOURCES, 2);
    }

    #[test]
    fn runtime_policy_defaults_to_30_minutes_and_is_configurable() {
        let policy = news_policy(|_| None);
        assert_eq!((policy.min_seconds, policy.target_seconds, policy.max_seconds), (1500.0, 1800.0, 2100.0));
        let custom = news_policy(|name| match name {
            "ATLAS_NEWS_TARGET_MINUTES" => Some("20".into()),
            "ATLAS_NEWS_MIN_MINUTES" => Some("15".into()),
            _ => None,
        });
        assert_eq!((custom.min_seconds, custom.target_seconds, custom.max_seconds), (900.0, 1200.0, 1500.0));
        assert_eq!(news_policy(|_| Some("999".into())).target_seconds, 1800.0);
    }

    #[test]
    fn daily_capacity_accounts_for_overhead() {
        assert_eq!(daily_capacity(3600.0), 18);
        assert_eq!(daily_capacity(4.0 * 3600.0), 5);
        assert_eq!(daily_capacity(12.0 * 3600.0), 1);
    }

    #[test]
    fn outlets_are_named_from_source_urls() {
        assert_eq!(outlet_name("https://www.cartoonbrew.com/anime/csm").as_deref(), Some("Cartoon Brew"));
        assert_eq!(outlet_name("https://www.animenewsnetwork.com/news/x").as_deref(), Some("Anime News Network"));
        assert_eq!(outlet_name("https://variety.com/2026/film").as_deref(), Some("Variety"));
        assert_eq!(outlet_name("https://example.org/2026/film").as_deref(), Some("example.org"));
        assert_eq!(outlet_name("not a url"), None);
    }

    #[test]
    fn anchor_wardrobe_is_smart_casual() {
        let spec: serde_json::Value = serde_json::from_str(ANCHOR_WARDROBE).unwrap();
        let banned = spec["banned_keywords"].as_array().unwrap();
        for word in ["tie", "suit", "hoodie", "jogger"] {
            assert!(banned.iter().any(|b| b == word));
        }
        let anchors = spec["characters"].as_array().unwrap();
        assert!(anchors.iter().any(|c| c["match"][0] == "marcus"));
        assert!(anchors.iter().any(|c| c["match"][0] == "elena"));
        assert_eq!(spec["staff_looks"].as_array().unwrap().len(), 7);
    }

    #[test]
    fn newsroom_manifest_passes_rights_firewall() {
        let manifest = newsroom_rights_manifest();
        assert_eq!(
            CopyrightComplianceEngine::new().scan_production_manifest(&manifest),
            ScanResult::AllClear
        );
    }
}
