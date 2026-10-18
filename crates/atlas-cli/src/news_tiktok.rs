//! Clearly Drawn News on TikTok: a vertical 9:16 highlight of each episode, posted with the
//! official Content Posting API (Direct Post). Off by default (`ATLAS_NEWS_TIKTOK=on` to post);
//! the cut and caption are still built so a dry run shows exactly what would go out.
use crate::news_voice::VoicedLine;
use atlas_analytics::tiktok_direct::{
    TIKTOK_BASELINE_MAX_SECS, TikTokAppConfig, TikTokDirectClient, TikTokPostInfo, authorize_url, clip_caption,
    pkce_pair,
};
use atlas_media::fact_checker::FactCheckReport;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const NEWS_TIKTOK_HASHTAGS: &str =
    "#ClearlyDrawnNews #animation #animationnews #cartoon #anime #animationindustry";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum TikTokMode {
    Off,
    On,
}

/// `ATLAS_NEWS_TIKTOK=on|1|true|yes` posts to TikTok; anything else (the default) never does.
pub(crate) fn tiktok_mode(lookup: impl Fn(&str) -> Option<String>) -> TikTokMode {
    match lookup("ATLAS_NEWS_TIKTOK").map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if matches!(v.as_str(), "on" | "1" | "true" | "yes") => TikTokMode::On,
        _ => TikTokMode::Off,
    }
}

/// `ATLAS_TIKTOK_PRIVACY` (default `PUBLIC_TO_EVERYONE`); must be one of the account's options.
/// Unaudited TikTok apps may only use `SELF_ONLY`.
pub(crate) fn tiktok_privacy(lookup: impl Fn(&str) -> Option<String>) -> String {
    lookup("ATLAS_TIKTOK_PRIVACY")
        .map(|v| v.trim().to_ascii_uppercase())
        .filter(|v| matches!(v.as_str(), "PUBLIC_TO_EVERYONE" | "MUTUAL_FOLLOW_FRIENDS" | "FOLLOWER_OF_CREATOR" | "SELF_ONLY"))
        .unwrap_or_else(|| "PUBLIC_TO_EVERYONE".into())
}

/// Highlight length cap: `ATLAS_TIKTOK_MAX_SECS` (default 180, which every creator can post).
pub(crate) fn tiktok_max_secs(lookup: impl Fn(&str) -> Option<String>) -> u32 {
    lookup("ATLAS_TIKTOK_MAX_SECS")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|v| (10..=600).contains(v))
        .unwrap_or(TIKTOK_BASELINE_MAX_SECS)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Highlight window: from the top of the show, whole voiced lines only, ending before `max_secs`.
pub(crate) fn highlight_window(lines: &[VoicedLine], max_secs: u32) -> Option<(f32, f32)> {
    let limit = max_secs as f32 - 0.5;
    let start = lines.first()?.start_seconds.max(0.0);
    let end = lines
        .iter()
        .map(|l| l.start_seconds + l.duration_seconds)
        .take_while(|end| end - start <= limit)
        .last()?;
    Some((start, (end + 0.4).min(start + limit) - start))
}

pub(crate) fn build_caption(headline: &str, episode_id: &str, report: &FactCheckReport, youtube_url: Option<&str>) -> String {
    let mut sources: Vec<String> = report
        .passed_stories
        .iter()
        .flat_map(|s| std::iter::once(&s.primary_source_url).chain(s.secondary_source_url.as_ref()))
        .filter_map(|u| url::Url::parse(u).ok()?.host_str().map(|h| h.trim_start_matches("www.").to_string()))
        .collect();
    sources.sort();
    sources.dedup();
    let full = match youtube_url {
        Some(url) => format!("Full episode on YouTube: {url}"),
        None => "Full episode on YouTube: Clearly Drawn News".into(),
    };
    clip_caption(&format!(
        "{headline}\n\n{} {episode_id}. {full}\nAI-generated anchors; facts from {}.\n\n{NEWS_TIKTOK_HASHTAGS}",
        atlas_core::SHOW_TITLE,
        if sources.is_empty() { "the linked outlets".into() } else { sources.join(", ") }
    ))
}

pub(crate) fn post_info(caption: String, privacy: String, cover_ms: u32) -> TikTokPostInfo {
    TikTokPostInfo {
        title: caption,
        privacy_level: privacy,
        disable_duet: false,
        disable_comment: false,
        disable_stitch: false,
        video_cover_timestamp_ms: cover_ms,
        brand_content_toggle: false,
        brand_organic_toggle: false,
        is_aigc: true,
    }
}

/// `pad` (default): the 16:9 frame fit to the width over a blurred fill, so both anchors stay in
/// shot. `crop`: a centre 9:16 crop.
fn vertical_filter(framing: &str) -> &'static str {
    if framing.eq_ignore_ascii_case("crop") {
        "[0:v]scale=-2:1920,crop=1080:1920,setsar=1,fps=30[v]"
    } else {
        "[0:v]split[a][b];[a]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=24:2[bg];\
         [b]scale=1080:-2[fg];[bg][fg]overlay=(W-w)/2:(H-h)/2,setsar=1,fps=30[v]"
    }
}

pub(crate) async fn build_vertical_cut(source: &Path, out: &Path, start: f32, duration: f32) -> anyhow::Result<()> {
    let framing = env("ATLAS_TIKTOK_FRAMING").unwrap_or_else(|| "pad".into());
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-ss", &format!("{start:.2}"), "-t", &format!("{duration:.2}"), "-i"])
        .arg(source)
        .args(["-filter_complex", vertical_filter(&framing), "-map", "[v]", "-map", "0:a?"])
        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "128k", "-ar", "48000", "-movflags", "+faststart"])
        .arg(out)
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() || !out.is_file() {
        anyhow::bail!("ffmpeg vertical cut failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TikTokPackage {
    pub(crate) video: PathBuf,
    pub(crate) source: PathBuf,
    /// True when the cut came from the preview animatic (filming skipped): never posted.
    pub(crate) preview_source: bool,
    pub(crate) start_seconds: f32,
    pub(crate) duration_seconds: f32,
    pub(crate) caption: String,
    pub(crate) privacy_level: String,
    pub(crate) is_aigc: bool,
}

/// Builds `publish/tiktok_cut.mp4` and `publish/tiktok_metadata.json` from the filmed episode, or
/// from the preview animatic when filming was skipped (marked `preview_source`, for review only).
pub(crate) async fn build_package(
    episode_dir: &Path,
    final_video: Option<&Path>,
    lines: &[VoicedLine],
    caption: String,
) -> anyhow::Result<TikTokPackage> {
    let publish_dir = episode_dir.join("publish");
    let preview = episode_dir.join("preview").join("animatic.mp4");
    let (source, preview_source) = match final_video.filter(|p| p.is_file()) {
        Some(path) => (path.to_path_buf(), false),
        None if preview.is_file() => (preview, true),
        None => anyhow::bail!("no filmed episode or preview animatic to cut from"),
    };
    let (start, duration) =
        highlight_window(lines, tiktok_max_secs(env)).ok_or_else(|| anyhow::anyhow!("no voiced lines for the highlight"))?;
    let video = publish_dir.join("tiktok_cut.mp4");
    build_vertical_cut(&source, &video, start, duration).await?;
    let package = TikTokPackage {
        video,
        source,
        preview_source,
        start_seconds: start,
        duration_seconds: duration,
        caption,
        privacy_level: tiktok_privacy(env),
        is_aigc: true,
    };
    std::fs::write(publish_dir.join("tiktok_metadata.json"), serde_json::to_vec_pretty(&package)?)?;
    Ok(package)
}

/// Posts the package. Checks the account's privacy options and length limit first.
/// Returns (publish_id, final status text).
pub(crate) async fn post_package(package: &TikTokPackage) -> anyhow::Result<(String, String)> {
    if package.preview_source {
        anyhow::bail!("the TikTok cut came from the preview animatic, not a filmed episode");
    }
    let client = TikTokDirectClient::new(TikTokAppConfig::from_env()?);
    let creator = client.creator_info().await?;
    if !creator.privacy_level_options.iter().any(|o| o == &package.privacy_level) {
        anyhow::bail!(
            "TikTok privacy {} is not offered for @{} (options: {}); unaudited apps can only post SELF_ONLY",
            package.privacy_level,
            creator.creator_username,
            creator.privacy_level_options.join(", ")
        );
    }
    if creator.max_video_post_duration_sec > 0 && package.duration_seconds > creator.max_video_post_duration_sec as f32 {
        anyhow::bail!(
            "TikTok cut is {:.0}s but @{} can post {}s",
            package.duration_seconds,
            creator.creator_username,
            creator.max_video_post_duration_sec
        );
    }
    let mut info = post_info(package.caption.clone(), package.privacy_level.clone(), 1000);
    info.disable_comment = creator.comment_disabled;
    info.disable_duet = creator.duet_disabled;
    info.disable_stitch = creator.stitch_disabled;
    let publish_id = client.post_video(&package.video, &info).await?;
    let mut last = String::from("PROCESSING_UPLOAD");
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        let status = client.publish_status(&publish_id).await?;
        last = status.status.clone();
        match last.as_str() {
            "PUBLISH_COMPLETE" => break,
            "FAILED" => anyhow::bail!("TikTok rejected the post: {}", status.fail_reason.unwrap_or_default()),
            _ => {}
        }
    }
    Ok((publish_id, last))
}

/// Read-only: confirms the TikTok app keys and token, prints the account's posting options.
pub(crate) async fn tiktok_check() -> anyhow::Result<()> {
    let client = TikTokDirectClient::new(TikTokAppConfig::from_env()?);
    let info = client.creator_info().await?;
    println!("\nTikTok token: OK (token file {})", client.config().token_path.display());
    println!("  Account: @{} ({})", info.creator_username, info.creator_nickname);
    println!("  Privacy options: {}", info.privacy_level_options.join(", "));
    println!("  Max video length: {}s", info.max_video_post_duration_sec);
    println!("  Configured privacy: {} | posting: {:?}", tiktok_privacy(env), tiktok_mode(env));
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct PendingAuth {
    state: String,
    code_verifier: String,
}

fn pending_path(config: &TikTokAppConfig) -> PathBuf {
    config.token_path.with_file_name("tiktok_pkce_pending.json")
}

/// Prints the TikTok authorization URL (Login Kit for Desktop with PKCE).
pub(crate) fn tiktok_auth() -> anyhow::Result<()> {
    let config = TikTokAppConfig::from_env()?;
    let (code_verifier, challenge) = pkce_pair();
    let state = uuid::Uuid::new_v4().simple().to_string();
    std::fs::write(pending_path(&config), serde_json::to_vec(&PendingAuth { state: state.clone(), code_verifier })?)?;
    println!("Open this URL, sign in to the TikTok account, and approve:\n\n{}\n", authorize_url(&config, &state, &challenge));
    println!("Then run: atlas news tiktok-auth-complete --redirect \"<the full URL you were sent to>\"");
    Ok(())
}

/// Finishes OAuth from the redirect URL (or a bare code) and stores the token.
pub(crate) async fn tiktok_auth_complete(redirect: &str) -> anyhow::Result<()> {
    let config = TikTokAppConfig::from_env()?;
    let pending_file = pending_path(&config);
    let pending: PendingAuth = serde_json::from_slice(&std::fs::read(&pending_file)?)
        .map_err(|_| anyhow::anyhow!("no pending TikTok login; run `atlas news tiktok-auth` first"))?;
    let (code, state) = match url::Url::parse(redirect) {
        Ok(url) => {
            let get = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.to_string());
            if let Some(error) = get("error") {
                anyhow::bail!("TikTok login failed: {error} {}", get("error_description").unwrap_or_default());
            }
            (get("code").ok_or_else(|| anyhow::anyhow!("redirect URL has no code"))?, get("state"))
        }
        Err(_) => (redirect.trim().to_string(), None),
    };
    if let Some(state) = state {
        if state != pending.state {
            anyhow::bail!("TikTok state mismatch; run `atlas news tiktok-auth` again");
        }
    }
    let token = TikTokDirectClient::new(config).exchange_code(&code, &pending.code_verifier).await?;
    let _ = std::fs::remove_file(pending_file);
    println!("TikTok connected (scopes: {}). Token saved; nothing was posted.", token.scope);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_storyboard::two_anchor_script::AnchorRole;

    fn line(start: f32, duration: f32) -> VoicedLine {
        VoicedLine {
            segment_number: 1,
            line_index: 0,
            speaker: AnchorRole::Enthusiast,
            voice: "am_michael".into(),
            text: "x".into(),
            file: PathBuf::new(),
            start_seconds: start,
            duration_seconds: duration,
        }
    }

    #[test]
    fn tiktok_is_off_and_public_by_default() {
        assert_eq!(tiktok_mode(|_| None), TikTokMode::Off);
        assert_eq!(tiktok_mode(|_| Some("on".into())), TikTokMode::On);
        assert_eq!(tiktok_privacy(|_| None), "PUBLIC_TO_EVERYONE");
        assert_eq!(tiktok_privacy(|_| Some("self_only".into())), "SELF_ONLY");
        assert_eq!(tiktok_privacy(|_| Some("everyone".into())), "PUBLIC_TO_EVERYONE");
        assert_eq!(tiktok_max_secs(|_| None), 180);
        assert_eq!(tiktok_max_secs(|_| Some("600".into())), 600);
    }

    #[test]
    fn highlight_ends_on_a_line_boundary_under_the_cap() {
        let lines: Vec<_> = (0..40).map(|i| line(i as f32 * 10.0, 9.5)).collect();
        let (start, duration) = highlight_window(&lines, 180).unwrap();
        assert_eq!(start, 0.0);
        assert!(duration <= 179.5 && duration > 160.0, "{duration}");
        let short = vec![line(0.0, 20.0)];
        assert!((highlight_window(&short, 180).unwrap().1 - 20.4).abs() < 0.01);
    }
}
