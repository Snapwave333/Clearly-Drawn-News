use super::*;

pub(crate) const MAX_PRODUCTION_ATTEMPTS: u32 = 3;
pub(crate) const PRODUCTION_RETRY_BASE_MINUTES: i64 = 30;
const AUTO_PRUNE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(7 * 86_400);
const AUTO_PRUNE_RETENTION: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);

fn auto_prune_if_due(
    data_dir: &Path,
    daemon_dir: &Path,
    now: std::time::SystemTime,
) -> anyhow::Result<Option<(usize, u64)>> {
    let marker = daemon_dir.join("last_cache_prune");
    let last_run = std::fs::metadata(&marker).and_then(|meta| meta.modified()).ok();
    let due = last_run.is_none_or(|last| {
        now.duration_since(last).unwrap_or_default() >= AUTO_PRUNE_INTERVAL
    });
    if !due {
        return Ok(None);
    }
    let result = cache::prune_cache(data_dir, now - AUTO_PRUNE_RETENTION, false)?;
    std::fs::create_dir_all(daemon_dir)?;
    std::fs::write(&marker, Utc::now().to_rfc3339())?;
    Ok(Some(result))
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ProductionFailureLedger {
    #[serde(default)]
    entries: std::collections::BTreeMap<String, ProductionFailure>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ProductionFailure {
    attempts: u32,
    last_error: String,
    retry_after: chrono::DateTime<Utc>,
}

impl ProductionFailureLedger {
    fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    fn key(video_id: &str, slug: &str) -> String {
        format!("{video_id}:{slug}")
    }

    fn is_blocked(&self, key: &str, now: chrono::DateTime<Utc>) -> bool {
        self.entries.get(key).is_some_and(|failure| {
            failure.attempts >= MAX_PRODUCTION_ATTEMPTS || now < failure.retry_after
        })
    }

    fn is_quarantined(&self, video_id: &str) -> bool {
        let prefix = format!("{video_id}:");
        self.entries.iter().any(|(key, failure)| {
            key.starts_with(&prefix) && failure.attempts >= MAX_PRODUCTION_ATTEMPTS
        })
    }

    fn record_failure(&mut self, key: &str, error: &str, now: chrono::DateTime<Utc>) -> u32 {
        let attempts = self.entries.get(key).map_or(0, |failure| failure.attempts) + 1;
        let backoff_minutes =
            (PRODUCTION_RETRY_BASE_MINUTES << (attempts - 1).min(5)).min(24 * 60);
        self.entries.insert(
            key.to_string(),
            ProductionFailure {
                attempts,
                last_error: error.to_string(),
                retry_after: now + chrono::Duration::minutes(backoff_minutes),
            },
        );
        attempts
    }

    fn clear(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }
}


fn news_schedule_hours(lookup: impl Fn(&str) -> Option<String>) -> Vec<u32> {
    let raw = lookup("ATLAS_NEWS_HOURS")
        .or_else(|| lookup("ATLAS_NEWS_DAILY_HOUR"))
        .unwrap_or_else(|| DEFAULT_NEWS_HOURS.to_string());
    let mut hours = raw
        .split(',')
        .filter_map(|value| value.trim().parse::<u32>().ok())
        .filter(|hour| *hour < 24)
        .collect::<Vec<_>>();
    hours.sort_unstable();
    hours.dedup();
    hours
}

/// One quality episode a day at noon local time (ATLAS_NEWS_HOURS overrides, e.g. "12,20").
const DEFAULT_NEWS_HOURS: &str = "12";

fn current_news_slot(hours: &[u32], now: chrono::NaiveDateTime) -> Option<String> {
    use chrono::Timelike;
    let today = now.date();
    match hours.iter().rev().find(|hour| **hour <= now.hour()) {
        Some(hour) => Some(format!("{today}@{hour:02}")),
        None => hours.last().map(|hour| format!("{}@{hour:02}", today.pred_opt().unwrap_or(today))),
    }
}

/// The slot's approval path: automatic by default; `ATLAS_NEWS_APPROVAL=telegram` needs a bot token.
fn news_approval(lookup: impl Fn(&str) -> Option<String>) -> Option<news::NewsApprovalMode> {
    let mode = news::news_approval_mode(&lookup);
    let token_set = lookup("TELEGRAM_BOT_TOKEN").is_some_and(|value| !value.trim().is_empty());
    match mode {
        news::NewsApprovalMode::Telegram if !token_set => None,
        mode => Some(mode),
    }
}

fn last_news_slot(marker: &Path) -> Option<String> {
    std::fs::read_to_string(marker).ok().map(|value| value.trim().to_string())
}

async fn run_daily_news(store: &Store, daemon_dir: &Path, now: chrono::DateTime<Utc>) -> anyhow::Result<()> {
    let env = |name: &str| std::env::var(name).ok();
    let local_now = now.with_timezone(&chrono::Local).naive_local();
    let Some(slot) = current_news_slot(&news_schedule_hours(env), local_now) else {
        return Ok(());
    };
    let marker = daemon_dir.join("news_last_run");
    if last_news_slot(&marker).as_deref() == Some(slot.as_str()) {
        return Ok(());
    }
    let failures_path = daemon_dir.join("production_failures.json");
    let mut failures = ProductionFailureLedger::load(&failures_path);
    let key = format!("news:{slot}");
    if failures.is_blocked(&key, now) {
        return Ok(());
    }
    let Some(approval) = news_approval(env) else {
        eprintln!(
            "Clearly Drawn News slot {slot} is due but ATLAS_NEWS_APPROVAL=telegram has no TELEGRAM_BOT_TOKEN; set the token or use ATLAS_NEWS_APPROVAL=auto."
        );
        std::fs::write(&marker, &slot)?;
        return Ok(());
    };
    println!("Clearly Drawn News slot {slot} is due ({approval:?}); producing the episode.");
    let produced = news::cmd_news(
        store,
        NewsAction::Produce {
            token: None,
            chat_id: env("TELEGRAM_CHAT_ID").and_then(|value| value.trim().parse().ok()),
            skip_telegram_wait: approval == news::NewsApprovalMode::Auto,
            dry_run_publish: false,
        },
    )
    .await;
    match produced {
        Ok(()) => {
            std::fs::write(&marker, &slot)?;
            if failures.clear(&key) {
                failures.save(&failures_path)?;
            }
        }
        Err(error) => {
            let attempts = failures.record_failure(&key, &error.to_string(), now);
            failures.save(&failures_path)?;
            eprintln!("News slot {slot} attempt {attempts}/{MAX_PRODUCTION_ATTEMPTS} failed; backing off: {error}");
        }
    }
    Ok(())
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

pub(crate) async fn cmd_daemon(store: &Store, action: DaemonAction) -> anyhow::Result<()> {
    let daemon_dir = store.data_dir().join("daemon");
    let pause_path = daemon_dir.join("paused");
    match action {
        DaemonAction::Start {
            once,
            poll_seconds,
            privacy,
            tiktok_user,
            news_only,
        } => {
            std::fs::create_dir_all(&daemon_dir)?;
            if pause_path.is_file() {
                std::fs::remove_file(&pause_path)?;
            }
            if news_only {
                println!("ATLAS daemon started for Clearly Drawn News only (explainer publisher off).");
            } else {
                println!("ATLAS continuous publisher started for YouTube and TikTok.");
            }
            println!(
                "Clearly Drawn News publishing: {:?} (ATLAS_NEWS_PUBLISH), privacy {} (ATLAS_NEWS_PRIVACY).",
                news_publish::publish_mode(env_var),
                news_publish::news_privacy(env_var)
            );
            let env = |name: &str| std::env::var(name).ok();
            let hours = news_schedule_hours(env);
            let listed = hours.iter().map(|h| format!("{h:02}:00")).collect::<Vec<_>>().join(", ");
            match (hours.is_empty(), news_approval(env)) {
                (true, _) => println!("Clearly Drawn News scheduling is off (set ATLAS_NEWS_HOURS, e.g. 12)."),
                (false, Some(approval)) => println!("Clearly Drawn News: {} episodes/day at {listed} local time ({approval:?}).", hours.len()),
                (false, None) => println!("Clearly Drawn News: {} slots/day at {listed} local time, but ATLAS_NEWS_APPROVAL=telegram has no TELEGRAM_BOT_TOKEN, so nothing will run.", hours.len()),
            }
            loop {
                if pause_path.is_file() {
                    println!("ATLAS continuous publisher paused.");
                    break;
                }
                if !news_only {
                    if let Err(error) = run_daemon_cycle(store, privacy, &tiktok_user).await {
                        eprintln!("Daemon cycle failed safely: {error}");
                    }
                }
                if let Err(error) = run_daily_news(store, &daemon_dir, Utc::now()).await {
                    eprintln!("Daily news step failed safely: {error}");
                }
                match auto_prune_if_due(store.data_dir(), &daemon_dir, std::time::SystemTime::now()) {
                    Ok(Some((removed, freed))) if removed > 0 => println!(
                        "Weekly cache prune removed {removed} stale entries, freeing {}",
                        cache::format_bytes(freed)
                    ),
                    Ok(_) => {}
                    Err(error) => eprintln!("Weekly cache prune skipped: {error}"),
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

pub(crate) async fn run_daemon_cycle(
    store: &Store,
    privacy: PublishPrivacy,
    tiktok_user: &str,
) -> anyhow::Result<()> {
    let ledger_path = store.data_dir().join("publish").join("ledger.json");
    let scheduler_path = store
        .data_dir()
        .join("publish")
        .join("scheduler_state.json");
    let failures_path = store
        .data_dir()
        .join("daemon")
        .join("production_failures.json");
    let ledger = PublishLedger::load(&ledger_path)?;
    let mut scheduler = PublishSchedulerState::load(&scheduler_path)?;
    let mut failures = ProductionFailureLedger::load(&failures_path);
    let now = Utc::now();
    let mut projects = store.list_projects(100)?;
    let active_count = projects
        .iter()
        .filter(|project| {
            matches!(project.publish.mode, PublishMode::Autonomous)
                && !daemon_project_complete(project)
                && !failures.is_quarantined(&project.video_id)
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
                let failure_key = ProductionFailureLedger::key(&project.video_id, profile.slug);
                if failures.is_blocked(&failure_key, now) {
                    continue;
                }
                let missing = missing_pipeline_tools(&probe_all().await);
                if !missing.is_empty() {
                    let names = missing
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    eprintln!(
                        "Production paused: required tools unavailable ({names}); run `atlas doctor`"
                    );
                    return Ok(());
                }
                let produced = cmd_produce(
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
                .await;
                if let Err(error) = produced {
                    let attempts = failures.record_failure(&failure_key, &error.to_string(), now);
                    failures.save(&failures_path)?;
                    if attempts >= MAX_PRODUCTION_ATTEMPTS {
                        eprintln!(
                            "Quarantined {failure_key} after {attempts} failed productions: {error}"
                        );
                    } else {
                        eprintln!(
                            "Production {failure_key} failed (attempt {attempts}/{MAX_PRODUCTION_ATTEMPTS}); backing off: {error}"
                        );
                    }
                    return Ok(());
                }
                if failures.clear(&failure_key) {
                    failures.save(&failures_path)?;
                }
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

pub(crate) async fn run_daemon_publish(
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

pub(crate) async fn create_autonomous_project(store: &Store, existing: &[Project]) -> anyhow::Result<Project> {
    let director = DirectorClient::from_env().ok_or_else(|| {
        anyhow::anyhow!(
            "Autonomous topic generation requires NVIDIA_API_KEY or ATLAS_DIRECTOR_API_KEY"
        )
    })?;
    let existing_titles = existing
        .iter()
        .map(|project| project.topic.title.clone())
        .collect::<Vec<_>>();
    let idea = match director.create_topic(&existing_titles).await {
        Ok(idea) => idea,
        Err(err) => {
            eprintln!("Director topic API fallback: {err}");
            let candidates = [
                ("How do solar panels turn light into electricity?", "Photovoltaic effect explained with electron excitation in silicon."),
                ("Why do airplanes stay in the air?", "Fluid dynamics of airfoils, pressure differentials, and Bernoulli vs Newton."),
                ("How does computer memory actually store bits?", "Capacitors and transistors in DRAM refreshing millions of times per second."),
                ("Why does sound travel faster in water than air?", "Particle density, bulk modulus, and wave propagation mechanics."),
                ("How do noise-cancelling headphones erase sound?", "Destructive phase interference and real-time audio sampling inversion."),
                ("How do nuclear power reactors control fission?", "Neutron moderation, control rods, and self-sustaining thermal balance."),
            ];
            let picked = candidates
                .iter()
                .find(|(title, _)| !existing_titles.iter().any(|e| e.eq_ignore_ascii_case(title)))
                .unwrap_or(&candidates[0]);
            atlas_storyboard::TopicIdea {
                title: picked.0.to_string(),
                rationale: picked.1.to_string(),
            }
        }
    };
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

pub(crate) fn daemon_project_complete(project: &Project) -> bool {
    ["long", "short"].iter().all(|format| {
        project.publish.youtube_video_ids.contains_key(*format)
            && project.publish.tiktok_post_ids.contains_key(*format)
    })
}

pub(crate) fn cleanup_completed_daemon_package(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(stamp: &str) -> chrono::DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339(stamp).unwrap().with_timezone(&Utc)
    }

    fn local(stamp: &str) -> chrono::NaiveDateTime {
        at(stamp).naive_utc()
    }

    #[test]
    fn news_slots_follow_the_schedule() {
        let hours = vec![0, 6, 12, 18];
        assert_eq!(current_news_slot(&hours, local("2026-10-17T05:59:00Z")).as_deref(), Some("2026-10-17@00"));
        assert_eq!(current_news_slot(&hours, local("2026-10-17T06:00:00Z")).as_deref(), Some("2026-10-17@06"));
        assert_eq!(current_news_slot(&hours, local("2026-10-17T23:10:00Z")).as_deref(), Some("2026-10-17@18"));
        assert_eq!(current_news_slot(&[14], local("2026-10-17T09:00:00Z")).as_deref(), Some("2026-10-16@14"));
        assert_eq!(current_news_slot(&[], local("2026-10-17T09:00:00Z")), None);
    }

    #[test]
    fn daily_news_needs_a_schedule_and_an_approval_path() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        assert_eq!(news_schedule_hours(env(&[])), vec![12]);
        assert!(news_schedule_hours(env(&[("ATLAS_NEWS_HOURS", "off")])).is_empty());
        assert_eq!(news_schedule_hours(env(&[("ATLAS_NEWS_DAILY_HOUR", "14")])), vec![14]);
        assert!(news_schedule_hours(env(&[("ATLAS_NEWS_DAILY_HOUR", "25")])).is_empty());
        assert_eq!(news_schedule_hours(env(&[("ATLAS_NEWS_HOURS", "18, 0,6,12,6,99")])), vec![0, 6, 12, 18]);
        use news::NewsApprovalMode::{Auto, Telegram};
        assert_eq!(news_approval(env(&[])), Some(Auto));
        assert_eq!(news_approval(env(&[("TELEGRAM_BOT_TOKEN", "1:x")])), Some(Auto), "a token alone must not re-enable the gate");
        assert_eq!(news_approval(env(&[("ATLAS_NEWS_APPROVAL", "telegram")])), None);
        assert_eq!(news_approval(env(&[("ATLAS_NEWS_APPROVAL", "telegram"), ("TELEGRAM_BOT_TOKEN", " ")])), None);
        assert_eq!(news_approval(env(&[("ATLAS_NEWS_APPROVAL", "telegram"), ("TELEGRAM_BOT_TOKEN", "1:x")])), Some(Telegram));
    }

    #[test]
    fn auto_prune_runs_at_most_weekly() {
        let dir = std::env::temp_dir().join(format!("atlas_daemon_{}", uuid::Uuid::new_v4().simple()));
        let daemon_dir = dir.join("daemon");
        let now = std::time::SystemTime::now();
        assert_eq!(auto_prune_if_due(&dir, &daemon_dir, now).unwrap(), Some((0, 0)));
        assert_eq!(auto_prune_if_due(&dir, &daemon_dir, now).unwrap(), None);
        let next_week = now + std::time::Duration::from_secs(8 * 86_400);
        assert!(auto_prune_if_due(&dir, &daemon_dir, next_week).unwrap().is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn production_failures_back_off_then_quarantine() {
        let now = Utc::now();
        let mut ledger = ProductionFailureLedger::default();
        let key = ProductionFailureLedger::key("vid_1", "long");
        assert!(!ledger.is_blocked(&key, now));

        assert_eq!(ledger.record_failure(&key, "blender crashed", now), 1);
        assert!(ledger.is_blocked(&key, now));
        assert!(!ledger.is_blocked(&key, now + chrono::Duration::minutes(31)));
        assert!(!ledger.is_quarantined("vid_1"));

        ledger.record_failure(&key, "blender crashed", now);
        assert!(ledger.is_blocked(&key, now + chrono::Duration::minutes(59)));
        assert!(!ledger.is_blocked(&key, now + chrono::Duration::minutes(61)));

        ledger.record_failure(&key, "blender crashed", now);
        assert!(ledger.is_quarantined("vid_1"));
        assert!(!ledger.is_quarantined("vid_10"));
        assert!(ledger.is_blocked(&key, now + chrono::Duration::days(7)));

        assert!(ledger.clear(&key));
        assert!(!ledger.is_blocked(&key, now));
    }
}
