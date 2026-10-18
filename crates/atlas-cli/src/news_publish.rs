//! Clearly Drawn News publish stage: automatic holds, the YouTube package (metadata, thumbnail,
//! captions), and the upload. Runs after filming in `atlas news produce`, or on an existing
//! episode with `atlas news publish --episode EP-012`.
//!
//! Uploads happen only when `ATLAS_NEWS_PUBLISH=on` and the run is not a dry run. Otherwise the
//! package is built and the episode is marked ready, which is the default.
use super::*;
use crate::news_voice::VoicedLine;
use atlas_core::EpisodeDurationPolicy;
use atlas_media::fact_checker::FactCheckReport;
use atlas_policy::copyright::{CopyrightComplianceEngine, RightsRecord, ScanResult};
use atlas_policy::telegram::notify_best_effort;
use atlas_storyboard::two_anchor_script::MasterEpisodeScript;

/// YouTube category 1 (Film & Animation). Clearly Drawn News covers the animation industry, so it
/// sits with its audience there; 25 (News & Politics) is aimed at civic/political news and brings
/// news-authority treatment that does not fit entertainment-trade coverage.
pub(crate) const NEWS_CATEGORY_ID: &str = "1";
const TITLE_LIMIT: usize = 100;
const DESCRIPTION_LIMIT: usize = 4900;
const TAGS_CHAR_LIMIT: usize = 450;
const CAPTION_CHARS: usize = 84;
const CAPTION_LINE_CHARS: usize = 42;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NewsPublishMode {
    /// Build the package only (default until Eli turns publishing on).
    Off,
    /// Upload to YouTube when no hold applies.
    On,
}

pub(crate) fn publish_mode(lookup: impl Fn(&str) -> Option<String>) -> NewsPublishMode {
    match lookup("ATLAS_NEWS_PUBLISH").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("on" | "1" | "true" | "yes") => NewsPublishMode::On,
        _ => NewsPublishMode::Off,
    }
}

pub(crate) fn news_privacy(lookup: impl Fn(&str) -> Option<String>) -> &'static str {
    match lookup("ATLAS_NEWS_PRIVACY").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("private") => "private",
        Some("unlisted") => "unlisted",
        _ => "public",
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.trim().is_empty())
}

// ---------------------------------------------------------------- holds

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GuestBooking {
    pub(crate) date: String,
    pub(crate) character: String,
    pub(crate) caveat: String,
    pub(crate) confirmed: bool,
}

impl GuestBooking {
    /// A booking is flagged when its trademark/rights caveat names a restriction ("None known…" is not).
    pub(crate) fn flagged(&self) -> bool {
        let caveat = self.caveat.trim().to_ascii_lowercase();
        !caveat.is_empty() && !caveat.starts_with("none")
    }
}

pub(crate) fn casting_csv_path() -> PathBuf {
    env("ATLAS_CASTING_CSV").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../AtlasStudioOffice/casting/CDN_Interview_Bookings_Month1.csv")
    })
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => fields.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    fields.push(current);
    fields
}

/// Reads guest bookings. Columns used: `date`, `character`, `trademark_caveat`, and the optional
/// `rights_confirmed` (yes/true/1/confirmed) that a human sets after clearing the caveat.
pub(crate) fn parse_bookings(csv: &str) -> Vec<GuestBooking> {
    let mut lines = csv.lines().filter(|line| !line.trim().is_empty());
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let header = split_csv_line(header.trim_start_matches('\u{feff}'))
        .into_iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let col = |name: &str| header.iter().position(|h| h == name);
    let (Some(date), Some(character)) = (col("date"), col("character")) else {
        return Vec::new();
    };
    let caveat = col("trademark_caveat");
    let confirmed = col("rights_confirmed");
    lines
        .map(split_csv_line)
        .map(|row| {
            let get = |index: Option<usize>| index.and_then(|i| row.get(i)).map(|v| v.trim().to_string()).unwrap_or_default();
            GuestBooking {
                date: get(Some(date)),
                character: get(Some(character)),
                caveat: get(caveat),
                confirmed: matches!(get(confirmed).to_ascii_lowercase().as_str(), "yes" | "true" | "1" | "confirmed"),
            }
        })
        .collect()
}

/// Guest bookings on `date` whose rights flag is set but not confirmed.
pub(crate) fn unconfirmed_guest_flags(bookings: &[GuestBooking], date: &str) -> Vec<GuestBooking> {
    bookings
        .iter()
        .filter(|b| b.date == date && b.flagged() && !b.confirmed)
        .cloned()
        .collect()
}

/// Evaluates every automatic hold for an episode package from its saved files.
pub(crate) fn evaluate_holds(episode_dir: &Path, policy: &EpisodeDurationPolicy) -> Vec<String> {
    let mut holds = Vec::new();
    let status = read_value(&episode_dir.join("status.json")).unwrap_or_default();

    match status["wire_origin"].as_str() {
        Some("live_feeds") => {}
        Some(other) => holds.push(format!("stories are not from live feeds ({other})")),
        None => holds.push("story source unknown (no wire_origin in status.json)".into()),
    }

    let runtime = status["runtime_seconds"].as_f64().unwrap_or(0.0);
    if let Err(problem) = policy.validate(runtime) {
        holds.push(format!("runtime fails policy: {problem}"));
    }

    let video = status["final_video"].as_str().map(PathBuf::from);
    match video {
        Some(path) if path.is_file() => {}
        Some(path) => holds.push(format!("final video missing at {}", path.display())),
        None => holds.push(format!(
            "filming skipped or failed: {}",
            status["filming"].as_str().unwrap_or("no filming result")
        )),
    }

    let rights_clear = match status["rights_clear"].as_bool() {
        Some(clear) => clear,
        None => std::fs::read(episode_dir.join("rights_manifest.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<RightsRecord>>(&bytes).ok())
            .map(|manifest| CopyrightComplianceEngine::new().scan_production_manifest(&manifest) == ScanResult::AllClear)
            .unwrap_or(false),
    };
    if !rights_clear {
        holds.push("rights check is not all clear".into());
    }

    let date = status["local_date"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let csv_path = casting_csv_path();
    match std::fs::read_to_string(&csv_path) {
        Ok(csv) => {
            for booking in unconfirmed_guest_flags(&parse_bookings(&csv), &date) {
                holds.push(format!(
                    "guest {} on {} has an unconfirmed rights flag ({}); set rights_confirmed=yes in the casting CSV after review",
                    booking.character, booking.date, booking.caveat
                ));
            }
        }
        Err(_) => eprintln!("  Casting CSV not found at {}; no guest rights flags checked", csv_path.display()),
    }
    holds
}

// ---------------------------------------------------------------- package

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct NewsVideoMetadata {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) tags: Vec<String>,
    pub(crate) category_id: String,
    pub(crate) privacy_status: String,
    pub(crate) contains_synthetic_media: bool,
    pub(crate) transcript: String,
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

pub(crate) fn build_metadata(script: &MasterEpisodeScript, report: &FactCheckReport, transcript: String, privacy: &str) -> NewsVideoMetadata {
    let title = clip_chars(&format!("{} {}", atlas_core::SHOW_TITLE, script.episode_title), TITLE_LIMIT);
    let mut description = format!(
        "{} — {}\n{}\n\nIn this episode:\n",
        script.episode_title,
        atlas_core::SHOW_TAGLINE,
        script.episode_date
    );
    let corroborated = report.passed_stories.iter().all(|s| s.secondary_source_url.is_some());
    let sourcing = if corroborated {
        "Every story comes from the outlets linked above and was confirmed by at least two of them"
    } else {
        "Every story comes from the outlets linked above"
    };
    let disclosure = format!(
        "\nAbout this video: {show} is produced by {company} with AI. The anchors Marcus and Elena, \
         their voices, and the virtual studio are synthetic, AI-generated media. {sourcing}; the anchors' \
         opinions are their own commentary.\n\n#AnimationNews #Anime #Cartoons #ClearlyDrawnNews",
        show = atlas_core::SHOW_TITLE,
        company = atlas_core::COMPANY_NAME,
    );
    for story in &report.passed_stories {
        let mut entry = format!("• {}\n  Source: {}\n", story.title, story.primary_source_url);
        if let Some(second) = &story.secondary_source_url {
            entry.push_str(&format!("  Also reported: {second}\n"));
        }
        if description.chars().count() + entry.chars().count() + disclosure.chars().count() > DESCRIPTION_LIMIT {
            description.push_str("• …more stories in the episode\n");
            break;
        }
        description.push_str(&entry);
    }
    description.push_str(&disclosure);

    let mut tags: Vec<String> = vec![
        atlas_core::SHOW_TITLE.into(),
        "CDN".into(),
        "animation news".into(),
        "anime news".into(),
        "cartoon news".into(),
        "animation industry".into(),
    ];
    for story in &report.passed_stories {
        for candidate in [story.category.as_str().to_string(), story.studio_or_organization.clone()] {
            let candidate = candidate.trim().to_string();
            if !candidate.is_empty() && !tags.iter().any(|t| t.eq_ignore_ascii_case(&candidate)) {
                tags.push(candidate);
            }
        }
    }
    let mut used = 0;
    tags.retain(|tag| {
        used += tag.chars().count() + 1;
        used <= TAGS_CHAR_LIMIT
    });

    NewsVideoMetadata {
        title,
        description,
        tags,
        category_id: NEWS_CATEGORY_ID.into(),
        privacy_status: privacy.into(),
        contains_synthetic_media: true,
        transcript,
    }
}

fn srt_time(seconds: f32) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", millis / 3_600_000, millis / 60_000 % 60, millis / 1000 % 60, millis % 1000)
}

fn chunk_words(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Splits text into the fewest cues of at most `max_chars`, balanced in length so no cue is a
/// one-word flash.
fn balanced_cues(text: &str, max_chars: usize) -> Vec<String> {
    let greedy = chunk_words(text, max_chars);
    if greedy.len() <= 1 {
        return greedy;
    }
    let total = text.split_whitespace().map(|w| w.chars().count() + 1).sum::<usize>();
    for width in (total / greedy.len()).max(1)..=max_chars {
        let attempt = chunk_words(text, width);
        if attempt.len() <= greedy.len() {
            return attempt;
        }
    }
    greedy
}

/// SubRip captions from the voiced timeline: long lines are split into two-line cues timed in
/// proportion to their length.
pub(crate) fn build_srt(lines: &[VoicedLine]) -> String {
    let mut out = String::new();
    let mut index = 1;
    for line in lines {
        let text = crate::news_voice::strip_stage_tags(&line.text);
        let chunks = balanced_cues(&text, CAPTION_CHARS);
        let total_chars = chunks.iter().map(|c| c.chars().count()).sum::<usize>().max(1) as f32;
        let mut start = line.start_seconds;
        for chunk in chunks {
            let span = line.duration_seconds * chunk.chars().count() as f32 / total_chars;
            let end = start + span;
            out.push_str(&format!(
                "{index}\n{} --> {}\n{}\n\n",
                srt_time(start),
                srt_time(end),
                chunk_words(&chunk, CAPTION_LINE_CHARS).join("\n")
            ));
            index += 1;
            start = end;
        }
    }
    out
}

fn story_cards_script() -> PathBuf {
    env("ATLAS_STORY_CARDS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../blender/story_cards.py"))
}

fn python_exe() -> String {
    env("ATLAS_PYTHON").unwrap_or_else(|| if cfg!(windows) { "py".into() } else { "python3".into() })
}

/// 1280x720 PNG thumbnail in the CDN brand style (Midnight / Signal Cyan / Inter) built with
/// `blender/story_cards.py card` (system Python + Pillow; it does not open Blender), then scaled.
async fn build_thumbnail(publish_dir: &Path, headline: &str, kicker: &str) -> anyhow::Result<PathBuf> {
    let card = publish_dir.join("thumbnail_card.png");
    let thumbnail = publish_dir.join("thumbnail.png");
    let script = story_cards_script();
    let made = tokio::process::Command::new(python_exe())
        .arg(&script)
        .args(["card", headline, "--kicker", kicker, "--out"])
        .arg(&card)
        .kill_on_drop(true)
        .output()
        .await?;
    if !made.status.success() || !card.is_file() {
        anyhow::bail!("story_cards.py failed: {}", String::from_utf8_lossy(&made.stderr).trim());
    }
    let scaled = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&card)
        .args(["-vf", "scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=0x0B1020"])
        .arg(&thumbnail)
        .output()
        .await?;
    if !scaled.status.success() || !thumbnail.is_file() {
        anyhow::bail!("ffmpeg could not scale the thumbnail: {}", String::from_utf8_lossy(&scaled.stderr).trim());
    }
    let size = std::fs::metadata(&thumbnail)?.len();
    if size > 2 * 1024 * 1024 {
        anyhow::bail!("thumbnail is {size} bytes; YouTube allows 2 MB");
    }
    Ok(thumbnail)
}

// ---------------------------------------------------------------- stage

pub(crate) struct PublishRequest {
    pub(crate) dry_run: bool,
    pub(crate) override_hold: bool,
}

fn read_value(path: &Path) -> Option<serde_json::Value> {
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn read_typed<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let bytes = std::fs::read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

#[derive(Deserialize)]
struct Timeline {
    lines: Vec<VoicedLine>,
}

fn youtube_credentials_path() -> PathBuf {
    if let Some(path) = env("ATLAS_YOUTUBE_CREDENTIALS") {
        return PathBuf::from(path);
    }
    let local = PathBuf::from("youtube_credentials.json");
    if local.is_file() {
        local
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../youtube_credentials.json")
    }
}

fn update_status(episode_dir: &Path, status_text: &str, holds: &[String], publish: serde_json::Value) -> anyhow::Result<()> {
    let path = episode_dir.join("status.json");
    let mut status = read_value(&path).unwrap_or_else(|| serde_json::json!({}));
    status["status"] = serde_json::json!(status_text);
    status["held"] = serde_json::json!(!holds.is_empty());
    status["hold_reasons"] = serde_json::json!(holds);
    status["publish"] = publish;
    std::fs::write(&path, serde_json::to_vec_pretty(&status)?)?;
    Ok(())
}

/// Builds the package, applies holds, and (only when publishing is on) uploads to YouTube.
pub(crate) async fn publish_episode(store: &Store, episode_dir: &Path, request: PublishRequest) -> anyhow::Result<()> {
    let policy = EpisodeDurationPolicy::default();
    let episode_id = episode_dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mode = publish_mode(env);
    let privacy = news_privacy(env);
    println!("\n[PHASE 7: PUBLISH ({mode:?}{}; privacy {privacy})]", if request.dry_run { ", dry run" } else { "" });

    let status = read_value(&episode_dir.join("status.json")).unwrap_or_default();
    if let Some(existing) = status["publish"]["youtube_url"].as_str() {
        println!("  Already on YouTube: {existing}");
        return Ok(());
    }

    let mut holds = evaluate_holds(episode_dir, &policy);
    let publish_dir = episode_dir.join("publish");
    std::fs::create_dir_all(&publish_dir)?;

    let mut package = serde_json::json!({ "mode": mode, "dry_run": request.dry_run, "privacy": privacy });
    let script: Option<MasterEpisodeScript> = read_typed(&episode_dir.join("script.json")).ok();
    let report: Option<FactCheckReport> = read_typed(&episode_dir.join("fact_check.json")).ok();
    let timeline: Option<Timeline> = read_typed(&episode_dir.join("voice_timeline.json")).ok();

    let (metadata_path, srt_path, thumbnail_path) = (
        publish_dir.join("youtube_metadata.json"),
        publish_dir.join("captions_en.srt"),
        publish_dir.join("thumbnail.png"),
    );
    match (&script, &report, &timeline) {
        (Some(script), Some(report), Some(timeline)) => {
            let srt = build_srt(&timeline.lines);
            std::fs::write(&srt_path, &srt)?;
            let transcript = timeline.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
            let metadata = build_metadata(script, report, transcript, privacy);
            std::fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)?;
            println!("  Metadata: {} ({} chars description, {} tags, category {})", metadata.title, metadata.description.chars().count(), metadata.tags.len(), metadata.category_id);
            println!("  Captions: {} cues", srt.matches(" --> ").count());
            let headline = report.passed_stories.first().map(|s| s.title.clone()).unwrap_or_else(|| script.episode_title.clone());
            match build_thumbnail(&publish_dir, &headline, &format!("{} · {episode_id}", atlas_core::SHOW_TITLE)).await {
                Ok(path) => println!("  Thumbnail: {}", path.display()),
                Err(error) => holds.push(format!("thumbnail could not be built: {error}")),
            }
        }
        _ => holds.push("package incomplete: script.json, fact_check.json and voice_timeline.json are all required".into()),
    }
    package["metadata"] = serde_json::json!(metadata_path);
    package["captions"] = serde_json::json!(srt_path);
    package["thumbnail"] = serde_json::json!(thumbnail_path);

    if !holds.is_empty() && !request.override_hold {
        println!("  HELD — not uploading:");
        for reason in &holds {
            println!("    • {reason}");
        }
        update_status(episode_dir, "held", &holds, package)?;
        notify_best_effort(&format!(
            "⏸ {} {episode_id} is HELD (no upload):\n• {}",
            atlas_core::SHOW_TITLE,
            holds.join("\n• ")
        ))
        .await;
        return Ok(());
    }
    if request.override_hold && !holds.is_empty() {
        println!("  Holds overridden by --override-hold: {}", holds.join("; "));
        package["overridden_holds"] = serde_json::json!(holds);
    }

    if request.dry_run || mode == NewsPublishMode::Off {
        let why = if request.dry_run { "dry run" } else { "ATLAS_NEWS_PUBLISH is off" };
        println!("  Ready to publish; not uploading ({why}).");
        update_status(episode_dir, &format!("ready to publish ({why})"), &[], package)?;
        return Ok(());
    }

    let video = PathBuf::from(status["final_video"].as_str().unwrap_or_default());
    let metadata: NewsVideoMetadata = read_typed(&metadata_path)?;
    let credentials = load_youtube_credentials(youtube_credentials_path()).await?;
    let client = YouTubeAnalyticsClient::new(credentials);
    if !client.is_authenticated()? {
        anyhow::bail!("YouTube is not authenticated; run `atlas analytics auth` first");
    }
    let channel = client.fetch_channel_status().await?;
    let guardian = PolicyGuardian::new(store.data_dir().join("policy"))?;
    let report = guardian
        .vet(
            &ContentPreflight {
                title: metadata.title.clone(),
                description: metadata.description.clone(),
                transcript: metadata.transcript.clone(),
                contains_synthetic_media: metadata.contains_synthetic_media,
                captions_present: srt_path.is_file(),
                thumbnail_present: thumbnail_path.is_file(),
                broll_expected: false,
                broll_attribution_present: true,
                unique_media: true,
            },
            &ChannelPreflight {
                channel_id: channel.channel_id,
                available: channel.available,
                linked: channel.is_linked,
                long_uploads_status: channel.long_uploads_status,
                privacy_status: channel.privacy_status,
                browser_review: guardian.channel_review()?,
                require_browser_review: false,
            },
            &format!("news_{episode_id}"),
        )
        .await?;
    if !report.publish_allowed {
        let blocked = report
            .checks
            .iter()
            .filter(|check| check.status != CheckStatus::Pass)
            .map(|check| format!("policy {}: {}", check.name, check.detail))
            .collect::<Vec<_>>();
        update_status(episode_dir, "held", &blocked, package)?;
        notify_best_effort(&format!("⏸ {episode_id} HELD by the policy check:\n• {}", blocked.join("\n• "))).await;
        return Ok(());
    }

    let upload = YouTubeUploadMetadata {
        title: metadata.title.clone(),
        description: metadata.description.clone(),
        tags: metadata.tags.clone(),
        privacy_status: metadata.privacy_status.clone(),
        category_id: metadata.category_id.clone(),
        contains_synthetic_media: metadata.contains_synthetic_media,
    };
    let result = client.upload_video(&video, &upload).await?;
    package["youtube_video_id"] = serde_json::json!(result.video_id);
    package["youtube_url"] = serde_json::json!(result.url);
    package["published_at"] = serde_json::json!(Utc::now());
    update_status(episode_dir, "published", &[], package.clone())?;
    if let Err(error) = client.upload_thumbnail(&result.video_id, &thumbnail_path).await {
        eprintln!("  Thumbnail upload failed (video is live): {error}");
    }
    if let Err(error) = client.upload_captions(&result.video_id, &srt_path, "en-US").await {
        eprintln!("  Caption upload failed (video is live): {error}");
    }
    println!("  Published to YouTube ({privacy}): {}", result.url);
    notify_best_effort(&format!("✅ {} {episode_id} is live on YouTube ({privacy}):\n{}\n{}", atlas_core::SHOW_TITLE, metadata.title, result.url)).await;
    Ok(())
}

/// Read-only YouTube check: refreshes the stored OAuth token and reads the channel status
/// (no uploads, nothing printed but channel facts).
pub(crate) async fn youtube_check() -> anyhow::Result<()> {
    let path = youtube_credentials_path();
    println!("\nYouTube credentials: {}", path.display());
    let client = YouTubeAnalyticsClient::new(load_youtube_credentials(&path).await?);
    if !client.is_authenticated()? {
        anyhow::bail!("no stored YouTube refresh token; run `atlas analytics auth` and sign in");
    }
    let channel = client.fetch_channel_status().await?;
    println!("  Token refresh: OK");
    println!("  Channel: {} (available: {})", channel.channel_id, channel.available);
    println!("  Linked: {:?} | Long uploads: {:?} | Privacy: {:?}", channel.is_linked, channel.long_uploads_status, channel.privacy_status);
    println!("  Made for kids: {:?}", channel.made_for_kids);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_storyboard::two_anchor_script::AnchorRole;

    const CSV: &str = "date,day,segment,character,pd_version_year,lead_anchor,duration_min,title,cameras,pd_source_url,trademark_caveat\n\
2026-10-22,Thu,Thursday Throwback,Gertie the Dinosaur,1914,Elena,15,\"Gertie, Prehistoric\",\"A; B\",https://x,None known; use original score\n\
2026-11-05,Thu,Thursday Throwback,Mickey Mouse,Steamboat Willie (1928) only,Elena,25,\"Steamboat Willie on Shipping, Sound\",\"A; B\",https://y,Disney trademarks; no Disney logos/branding; 1928 design only\n";

    #[test]
    fn steamboat_willie_is_held_until_confirmed() {
        let bookings = parse_bookings(CSV);
        assert_eq!(bookings.len(), 2);
        assert_eq!(bookings[1].character, "Mickey Mouse");
        assert!(!bookings[0].flagged());
        assert!(bookings[1].flagged());
        assert!(unconfirmed_guest_flags(&bookings, "2026-10-22").is_empty());
        assert_eq!(unconfirmed_guest_flags(&bookings, "2026-11-05").len(), 1);
        assert!(unconfirmed_guest_flags(&bookings, "2026-11-06").is_empty());
        let confirmed = CSV.replace("trademark_caveat\n", "trademark_caveat,rights_confirmed\n").replace("1928 design only\n", "1928 design only,yes\n");
        assert!(unconfirmed_guest_flags(&parse_bookings(&confirmed), "2026-11-05").is_empty());
    }

    #[test]
    fn publishing_is_off_and_public_by_default() {
        assert_eq!(publish_mode(|_| None), NewsPublishMode::Off);
        assert_eq!(publish_mode(|_| Some("on".into())), NewsPublishMode::On);
        assert_eq!(publish_mode(|_| Some("maybe".into())), NewsPublishMode::Off);
        assert_eq!(news_privacy(|_| None), "public");
        assert_eq!(news_privacy(|_| Some("Unlisted".into())), "unlisted");
    }

    #[test]
    fn srt_cues_are_timed_and_wrapped() {
        let line = |start: f32, text: &str| VoicedLine {
            segment_number: 1,
            line_index: 0,
            speaker: AnchorRole::Enthusiast,
            voice: "m".into(),
            text: text.into(),
            file: PathBuf::from("a.wav"),
            start_seconds: start,
            duration_seconds: 4.0,
        };
        let long = "word ".repeat(40);
        let srt = build_srt(&[line(0.0, "[BANTER] Hello there."), line(3661.5, &long)]);
        assert!(srt.starts_with("1\n00:00:00,000 --> 00:00:04,000\nHello there.\n\n2\n01:01:01,500"));
        assert!(srt.lines().all(|l| l.chars().count() <= CAPTION_LINE_CHARS || l.contains("-->")));
        assert!(srt.contains("\n3\n"));
        let cues = balanced_cues("And I'm Elena. We read the animation news so you don't have to, and we check it twice.", CAPTION_CHARS);
        assert_eq!(cues.len(), 2);
        assert!(cues.iter().all(|c| c.split_whitespace().count() >= 5), "{cues:?}");
    }

    #[test]
    fn holds_cover_curated_runtime_filming_and_rights() {
        let dir = std::env::temp_dir().join(format!("atlas_holds_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("status.json"),
            serde_json::json!({ "wire_origin": "curated_fallback", "runtime_seconds": 489.0, "filming": "Unreal Engine filming skipped: test", "final_video": null, "rights_clear": false, "local_date": "1999-01-01" }).to_string(),
        )
        .unwrap();
        let holds = evaluate_holds(&dir, &EpisodeDurationPolicy::default());
        assert!(holds.iter().any(|h| h.contains("not from live feeds")));
        assert!(holds.iter().any(|h| h.contains("runtime fails policy")));
        assert!(holds.iter().any(|h| h.contains("filming skipped")));
        assert!(holds.iter().any(|h| h.contains("rights check")));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
