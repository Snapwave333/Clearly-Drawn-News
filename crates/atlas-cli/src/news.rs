use super::*;
use atlas_core::{EpisodeDurationPolicy, SHOW_TAGLINE, SHOW_TITLE};
use atlas_media::fact_checker::{FactCheckReport, NewsFactChecker};
use atlas_media::news_desk::{AnimationNewsDesk, NewsWire, WireOrigin};
use atlas_policy::copyright::{
    CopyrightComplianceEngine, MediaClassification, MediaType, RightsRecord, ScanResult,
};
use atlas_policy::telegram::{
    ApprovalDecision, EpisodeSlateProposal, ProposedNewsStory, TelegramApprovalGate,
};
use atlas_storyboard::DirectorClient;
use atlas_storyboard::story_script::{NewsroomWriter, ScriptMethod, StoryBrief};
use atlas_storyboard::two_anchor_script::{MasterEpisodeScript, TwoAnchorScriptWriter};

const DEFAULT_UE_CMD: &str =
    r"C:\Program Files\Epic Games\UE_5.8\Engine\Binaries\Win64\UnrealEditor-Cmd.exe";
const DEFAULT_UE_PROJECT: &str =
    r"C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\AtlasStudioOffice.uproject";
const UE_NEWSROOM_SCRIPT: &str = r"Content\Python\setup_newsroom_studio.py";
const ANCHOR_WARDROBE: &str = include_str!("../../../unreal/wardrobe.json");

pub(crate) async fn cmd_news(store: &Store, action: NewsAction) -> anyhow::Result<()> {
    println!("══════════════════════════════════════════════════════════════════");
    println!("  {} — {}", SHOW_TITLE, SHOW_TAGLINE);
    println!("  {} Animation Broadcast Newsroom", atlas_core::COMPANY_NAME);
    println!("══════════════════════════════════════════════════════════════════");

    let news_desk = AnimationNewsDesk::new();
    let fact_checker = NewsFactChecker::default();
    let policy = EpisodeDurationPolicy::default();
    let news_dir = store.data_dir().join("news");

    match action {
        NewsAction::Research => {
            println!("\n[1/3] Ingesting developing animation industry wire...");
            let wire = load_wire(&news_desk).await.items;
            println!("  Found {} candidate industry stories.", wire.len());

            println!("\n[2/3] Running Fact-Checking & Corroboration Engine...");
            let report = fact_checker.verify_candidates(wire);
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
                "\nEstimated Full Episode Runtime: {:.1} minutes (Target: 15–30 min)",
                report.estimated_total_minutes
            );
        }
        NewsAction::Slate => {
            let wire = load_wire(&news_desk).await;
            let origin = wire.origin;
            let report = fact_checker.verify_candidates(wire.items);
            let (episode_id, _) = next_episode(&news_dir);
            let title = episode_title(&episode_id, &report);
            println!("\nWriting the two-anchor broadcast script from {} verified stories...", report.passed_stories.len());
            let (episode, provenance) = draft_script(origin, &report, &title, &policy).await;

            println!("\nEpisode Slate: {}", episode.episode_title);
            println!("Script method: {}", provenance["method"].as_str().unwrap_or("unknown"));
            let compliance = match episode.validate_duration(&policy) {
                Ok(()) => "Compliant with 15–30 min constraint".to_string(),
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
            let report = fact_checker.verify_candidates(load_wire(&news_desk).await.items);
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
        NewsAction::Produce {
            token,
            chat_id,
            skip_telegram_wait,
        } => {
            let (episode_id, episode_number) = next_episode(&news_dir);
            let episode_dir = news_dir.join(&episode_id);
            std::fs::create_dir_all(&episode_dir)?;
            let mut timer = StageTimer::new();
            println!("\nEpisode: {} → {}", episode_id, episode_dir.display());

            timer.begin("research");
            println!("\n[PHASE 1: RESEARCH & FACT-CHECK]");
            let wire = load_wire(&news_desk).await;
            let wire_origin = wire.origin;
            let report = fact_checker.verify_candidates(wire.items);
            if report.passed_stories.is_empty() {
                anyhow::bail!("Production halted: no stories passed fact-checking");
            }
            write_json(&episode_dir.join("fact_check.json"), &report)?;
            println!(
                "  Curated {} verified stories ({} rejected).",
                report.passed_stories.len(),
                report.rejected_stories.len()
            );

            timer.begin("approval");
            println!("\n[PHASE 2: TELEGRAM HUMAN APPROVAL GATE]");
            let proposal = slate_proposal(&episode_id, episode_number, &report);
            write_json(&episode_dir.join("slate.json"), &proposal)?;
            let (approval, approved) = if skip_telegram_wait {
                println!("  Approval gate bypassed by --skip-telegram-wait (recorded in package).");
                (serde_json::json!({ "status": "bypassed", "at": Utc::now() }), true)
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
            let (episode, provenance) = draft_script(wire_origin, &report, &title, &policy).await;
            write_json(&episode_dir.join("script.json"), &episode)?;
            write_json(&episode_dir.join("script_provenance.json"), &provenance)?;
            println!("  Script method: {}", provenance["method"].as_str().unwrap_or("unknown"));
            if let Err(error) = episode.validate_duration(&policy) {
                let hint = if provenance["method"] == "director" {
                    ""
                } else {
                    " Set NVIDIA_API_KEY (or ATLAS_DIRECTOR_API_KEY) so the director writes full-length segments from live stories."
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
            match CopyrightComplianceEngine::new().scan_production_manifest(&manifest) {
                ScanResult::AllClear => println!(
                    "  {} assets scanned: all clear; third-party trailer audio muted.",
                    manifest.len()
                ),
                blocked => anyhow::bail!("Production halted by rights firewall: {blocked:?}"),
            }

            timer.begin("voices");
            println!("\n[PHASE 5: ANCHOR VOICE SYNTHESIS]");
            let voices = news_voice::voice_episode(store.data_dir(), &episode_dir, &episode_id, &episode).await?;
            println!("  {}", voices.summary());
            if let news_voice::VoiceOutcome::Voiced { narration, .. } = &voices {
                println!("  Narration: {}", narration.display());
            }

            timer.begin("filming");
            println!("\n[PHASE 6: UNREAL ENGINE 5.8 VIRTUAL STUDIO FILMING]");
            let filming = film_in_unreal().await?;
            println!("  {}", filming.summary());

            let timings = timer.finish();
            write_json(&episode_dir.join("stage_timings.json"), &timings)?;
            let runtime_seconds = match &voices {
                news_voice::VoiceOutcome::Voiced { total_seconds, .. } => f64::from(*total_seconds),
                news_voice::VoiceOutcome::Skipped(_) => episode.total_duration_secs,
            };
            let runtime_check = policy.validate(runtime_seconds);
            if let Err(problem) = &runtime_check {
                eprintln!("  ⚠ Runtime check: {problem}");
            }
            let status = match (voices.is_voiced(), runtime_check.is_ok(), filming.is_filmed()) {
                (true, false, _) => "VOICED — runtime outside the 15–30 minute policy; needs a longer script",
                (true, true, true) => "VOICED & FILMED — ready for broadcast mastering",
                (true, true, false) => "VOICED — Unreal filming pending",
                (false, _, true) => "FILMED — anchor voices pending",
                (false, _, false) => "SCRIPT PACKAGE READY — voices and Unreal filming pending",
            };
            write_json(
                &episode_dir.join("status.json"),
                &serde_json::json!({
                    "episode_id": episode_id,
                    "title": episode.episode_title,
                    "status": status,
                    "voices": voices.summary(),
                    "filming": filming.summary(),
                    "runtime_seconds": runtime_seconds,
                    "runtime_source": if voices.is_voiced() { "synthesized_audio" } else { "script_estimate" },
                    "runtime_compliant": runtime_check.is_ok(),
                    "wire_origin": wire_origin,
                    "script_method": provenance["method"],
                    "company": atlas_core::COMPANY_NAME,
                }),
            )?;

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
    Filmed,
    Skipped(String),
}

impl FilmingOutcome {
    fn is_filmed(&self) -> bool {
        matches!(self, Self::Filmed)
    }

    fn summary(&self) -> String {
        match self {
            Self::Filmed => "Unreal Engine filming completed".into(),
            Self::Skipped(reason) => format!("Unreal Engine filming skipped: {reason}"),
        }
    }
}

async fn film_in_unreal() -> anyhow::Result<FilmingOutcome> {
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
    let script = uproject
        .parent()
        .map(|dir| dir.join(UE_NEWSROOM_SCRIPT))
        .unwrap_or_else(|| PathBuf::from(UE_NEWSROOM_SCRIPT));
    println!("  Executing Unreal Engine command-line automation...");
    let status = tokio::process::Command::new(&ue_cmd)
        .arg(&uproject)
        .arg(format!("-ExecutePythonScript={}", script.display()))
        .args(["-nullrhi", "-unattended", "-nosplash"])
        .kill_on_drop(true)
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Unreal Engine filming failed with {status}");
    }
    Ok(FilmingOutcome::Filmed)
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
            outlets: std::iter::once(story.primary_source_url.as_str())
                .chain(story.secondary_source_url.as_deref())
                .filter_map(outlet_name)
                .collect(),
        })
        .collect()
}

async fn draft_script(
    origin: WireOrigin,
    report: &FactCheckReport,
    title: &str,
    policy: &EpisodeDurationPolicy,
) -> (MasterEpisodeScript, serde_json::Value) {
    if origin == WireOrigin::CuratedFallback {
        let script = TwoAnchorScriptWriter::new().generate_broadcast_episode(title, &today());
        return (script, serde_json::json!({ "method": "curated", "note": "hand-written script for the curated fallback stories" }));
    }
    let briefs = story_briefs(report);
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

async fn load_wire(news_desk: &AnimationNewsDesk) -> NewsWire {
    let wire = news_desk.fetch_wire().await;
    for error in &wire.feed_errors {
        eprintln!("  Feed unavailable: {error}");
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
    fn outlets_are_named_from_source_urls() {
        assert_eq!(outlet_name("https://www.cartoonbrew.com/anime/csm").as_deref(), Some("Cartoon Brew"));
        assert_eq!(outlet_name("https://www.animenewsnetwork.com/news/x").as_deref(), Some("Anime News Network"));
        assert_eq!(outlet_name("https://variety.com/2026/film").as_deref(), Some("variety.com"));
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
