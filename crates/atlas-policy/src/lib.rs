use atlas_core::{AtlasError, AtlasResult};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const POLICY_SOURCES: [(&str, &str); 5] = [
    (
        "community_guidelines",
        "https://support.google.com/youtube/answer/9288567",
    ),
    (
        "spam_and_deceptive_practices",
        "https://support.google.com/youtube/answer/2801973",
    ),
    (
        "channel_monetization",
        "https://support.google.com/youtube/answer/1311392",
    ),
    (
        "altered_and_synthetic_content",
        "https://support.google.com/youtube/answer/14328491",
    ),
    (
        "upload_limits",
        "https://support.google.com/youtube/answer/57407",
    ),
];

pub const TIKTOK_POLICY_SOURCES: [(&str, &str); 4] = [
    (
        "tiktok_community_guidelines",
        "https://www.tiktok.com/community-guidelines/en/",
    ),
    (
        "tiktok_creator_rewards_monetization",
        "https://www.tiktok.com/legal/page/row/creator-rewards-program-terms/en",
    ),
    (
        "tiktok_aigc_policy",
        "https://support.tiktok.com/en/using-tiktok/creating-videos/ai-generated-content-labels",
    ),
    (
        "tiktok_commercial_music_policy",
        "https://www.tiktok.com/legal/page/global/commercial-music-library/en",
    ),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokPreflight {
    pub title: String,
    pub description: String,
    pub duration_seconds: f64,
    pub is_original: bool,
    pub contains_synthetic_media: bool,
    pub aigc_label_enabled: bool,
    pub commercial_music_cleared: bool,
    pub community_guidelines_compliant: bool,
    pub account_warning_visible: bool,
    pub account_strike_visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Review,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyPageSnapshot {
    pub name: String,
    pub url: String,
    pub normalized_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicySnapshot {
    pub checked_at: DateTime<Utc>,
    pub pages: Vec<PolicyPageSnapshot>,
}

#[derive(Clone, Debug)]
pub struct ContentPreflight {
    pub title: String,
    pub description: String,
    pub transcript: String,
    pub contains_synthetic_media: bool,
    pub captions_present: bool,
    pub thumbnail_present: bool,
    pub broll_expected: bool,
    pub broll_attribution_present: bool,
    pub unique_media: bool,
}

#[derive(Clone, Debug)]
pub struct ChannelPreflight {
    pub channel_id: String,
    pub available: bool,
    pub linked: Option<bool>,
    pub long_uploads_status: Option<String>,
    pub privacy_status: Option<String>,
    pub browser_review: Option<ChannelBrowserReview>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelBrowserReview {
    pub channel_id: String,
    pub reviewed_at: DateTime<Utc>,
    pub warning_visible: bool,
    pub strike_visible: bool,
    #[serde(default)]
    pub training_completed: bool,
    #[serde(default)]
    pub warning_expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub restricted_policy: Option<String>,
    pub source_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyReport {
    pub checked_at: DateTime<Utc>,
    pub publish_allowed: bool,
    pub checks: Vec<PolicyCheck>,
    pub policy_pages: Vec<PolicyPageSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicySourceRefresh {
    pub current: PolicySnapshot,
    pub check: PolicyCheck,
}

pub struct PolicyGuardian {
    http: reqwest::Client,
    root: PathBuf,
}

impl PolicyGuardian {
    pub fn new(root: impl Into<PathBuf>) -> AtlasResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent("ATLAS-STUDIO-Policy-Guardian/0.1")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|error| AtlasError::Other(format!("policy HTTP client failed: {error}")))?;
        Ok(Self {
            http,
            root: root.into(),
        })
    }

    pub async fn vet(
        &self,
        content: &ContentPreflight,
        channel: &ChannelPreflight,
        report_name: &str,
    ) -> AtlasResult<PolicyReport> {
        let refresh = self.refresh_sources().await?;
        let mut checks = content_checks(content);
        checks.extend(channel_checks(channel));
        checks.push(refresh.check);
        let publish_allowed = checks.iter().all(|check| check.status == CheckStatus::Pass);
        let report = PolicyReport {
            checked_at: Utc::now(),
            publish_allowed,
            checks,
            policy_pages: refresh.current.pages,
        };
        write_json(
            &self
                .root
                .join("reports")
                .join(format!("{report_name}.json")),
            &report,
        )?;
        write_json(&self.root.join("latest_report.json"), &report)?;
        Ok(report)
    }

    pub async fn refresh_sources(&self) -> AtlasResult<PolicySourceRefresh> {
        let current = self.fetch_current_snapshot().await?;
        std::fs::create_dir_all(self.root.join("reports"))?;
        let baseline_path = self.root.join("accepted_baseline.json");
        let pending_path = self.root.join("pending_baseline.json");
        let baseline = read_snapshot(&baseline_path)?;
        let first_observation = baseline.is_none();
        if first_observation {
            write_json(&baseline_path, &current)?;
        }
        let accepted = baseline.as_ref().unwrap_or(&current);
        let check = if first_observation || accepted.pages == current.pages {
            if pending_path.exists() {
                std::fs::remove_file(&pending_path)?;
            }
            PolicyCheck {
                name: "official_policy_snapshot".into(),
                status: CheckStatus::Pass,
                detail: if first_observation {
                    "Current official YouTube policy pages established as the accepted baseline"
                        .into()
                } else {
                    "Current official YouTube policy pages match the accepted baseline".into()
                },
            }
        } else {
            write_json(&pending_path, &current)?;
            PolicyCheck {
                name: "official_policy_snapshot".into(),
                status: CheckStatus::Review,
                detail: changed_pages(accepted, &current),
            }
        };
        Ok(PolicySourceRefresh { current, check })
    }

    pub fn latest_report(&self) -> AtlasResult<Option<PolicyReport>> {
        read_json(&self.root.join("latest_report.json"))
    }

    pub fn channel_review(&self) -> AtlasResult<Option<ChannelBrowserReview>> {
        read_json(&self.root.join("channel_browser_review.json"))
    }

    pub fn record_channel_review(&self, review: &ChannelBrowserReview) -> AtlasResult<()> {
        write_json(&self.root.join("channel_browser_review.json"), review)
    }

    pub fn accept_pending(&self) -> AtlasResult<PolicySnapshot> {
        let pending_path = self.root.join("pending_baseline.json");
        let pending = read_snapshot(&pending_path)?.ok_or_else(|| {
            AtlasError::Other("no changed policy snapshot is waiting for review".into())
        })?;
        std::fs::create_dir_all(&self.root)?;
        write_json(&self.root.join("accepted_baseline.json"), &pending)?;
        std::fs::remove_file(pending_path)?;
        Ok(pending)
    }

    async fn fetch_current_snapshot(&self) -> AtlasResult<PolicySnapshot> {
        let mut pages = Vec::with_capacity(POLICY_SOURCES.len());
        for (name, url) in POLICY_SOURCES {
            let response = self
                .http
                .get(url)
                .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
                .send()
                .await
                .map_err(|error| {
                    AtlasError::Other(format!("policy source unavailable at {url}: {error}"))
                })?;
            if !response.status().is_success() {
                return Err(AtlasError::Other(format!(
                    "policy source returned {} at {url}",
                    response.status()
                )));
            }
            let html = response.text().await.map_err(|error| {
                AtlasError::Other(format!("policy source could not be read at {url}: {error}"))
            })?;
            let normalized = normalize_html(&html)?;
            if normalized.len() < 500 {
                return Err(AtlasError::Other(format!(
                    "policy source content was unexpectedly short at {url}"
                )));
            }
            pages.push(PolicyPageSnapshot {
                name: name.into(),
                url: url.into(),
                normalized_sha256: hex::encode(Sha256::digest(normalized.as_bytes())),
            });
        }
        Ok(PolicySnapshot {
            checked_at: Utc::now(),
            pages,
        })
    }
}

pub fn content_checks(content: &ContentPreflight) -> Vec<PolicyCheck> {
    let mut checks = Vec::new();
    add_check(
        &mut checks,
        "title_length",
        !content.title.trim().is_empty() && content.title.chars().count() <= 100,
        "Title must contain 1 to 100 characters",
    );
    add_check(
        &mut checks,
        "description_length",
        !content.description.trim().is_empty() && content.description.chars().count() <= 5000,
        "Description must contain 1 to 5000 characters",
    );
    add_check(
        &mut checks,
        "transcript_present",
        !content.transcript.trim().is_empty(),
        "The complete narration transcript must be available for review",
    );
    add_check(
        &mut checks,
        "synthetic_media_declaration",
        content.contains_synthetic_media,
        "Synthetic media must be declared in YouTube metadata",
    );
    let lower_description = content.description.to_lowercase();
    add_check(
        &mut checks,
        "viewer_disclosure",
        lower_description.contains("ai")
            || lower_description.contains("synthetic")
            || lower_description.contains("generated"),
        "Description must disclose AI-generated or synthetic production",
    );
    add_check(
        &mut checks,
        "captions",
        content.captions_present,
        "Captions must exist before upload",
    );
    add_check(
        &mut checks,
        "thumbnail",
        content.thumbnail_present,
        "A thumbnail must exist before upload",
    );
    add_check(
        &mut checks,
        "broll_attribution",
        !content.broll_expected || content.broll_attribution_present,
        "Imported B-roll must include source attribution",
    );
    add_check(
        &mut checks,
        "duplicate_media",
        content.unique_media,
        "Identical media must not be published twice",
    );
    let combined = format!(
        "{} {} {}",
        content.title, content.description, content.transcript
    )
    .to_lowercase();
    let sensitive_phrases = [
        "suicide method",
        "how to self harm",
        "child sexual",
        "sexual exploitation",
        "join the terrorist",
        "buy illegal drugs",
        "guaranteed medical cure",
        "stop taking your medication",
        "vote twice",
        "guaranteed investment return",
        "send your password",
    ];
    let matches = sensitive_phrases
        .iter()
        .filter(|phrase| combined.contains(**phrase))
        .copied()
        .collect::<Vec<_>>();
    checks.push(PolicyCheck {
        name: "sensitive_content_review".into(),
        status: if matches.is_empty() {
            CheckStatus::Pass
        } else {
            CheckStatus::Review
        },
        detail: if matches.is_empty() {
            "No high-risk policy phrase requires escalation".into()
        } else {
            format!(
                "High-risk policy language requires review: {}",
                matches.join(", ")
            )
        },
    });
    checks
}

pub fn channel_checks(channel: &ChannelPreflight) -> Vec<PolicyCheck> {
    let mut checks = Vec::new();
    add_check(
        &mut checks,
        "youtube_channel_available",
        channel.available,
        "The authenticated YouTube account must expose a channel",
    );
    add_check(
        &mut checks,
        "youtube_channel_linked",
        channel.linked.unwrap_or(false),
        "The YouTube channel must be linked and active",
    );
    let uploads_allowed = channel
        .long_uploads_status
        .as_deref()
        .map(|status| !matches!(status, "disallowed" | "ineligible"))
        .unwrap_or(false);
    add_check(
        &mut checks,
        "youtube_upload_eligibility",
        uploads_allowed,
        "YouTube must report an eligible long-upload status",
    );
    add_check(
        &mut checks,
        "youtube_channel_privacy",
        channel.privacy_status.is_some(),
        "YouTube channel status must include a privacy state",
    );
    let review_matches = channel
        .browser_review
        .as_ref()
        .map(|review| review.channel_id == channel.channel_id)
        .unwrap_or(false);
    let review_fresh = channel
        .browser_review
        .as_ref()
        .map(|review| Utc::now() - review.reviewed_at <= chrono::Duration::hours(24))
        .unwrap_or(false);
    let channel_clear = channel
        .browser_review
        .as_ref()
        .map(|review| {
            let warning_remediated = review.warning_visible
                && review.training_completed
                && review
                    .warning_expires_at
                    .map(|expires_at| expires_at > Utc::now())
                    .unwrap_or(false);
            !review.strike_visible && (!review.warning_visible || warning_remediated)
        })
        .unwrap_or(false);
    add_check(
        &mut checks,
        "browser_channel_identity",
        review_matches,
        "The real-browser channel review must match the authenticated API channel",
    );
    add_check(
        &mut checks,
        "browser_channel_review_freshness",
        review_fresh,
        "The real YouTube Studio channel review must be less than 24 hours old",
    );
    add_check(
        &mut checks,
        "community_guidelines_channel_state",
        channel_clear,
        "YouTube Studio must show no active strike, and any warning must have completed training with a future expiry",
    );
    checks
}

pub fn tiktok_content_checks(preflight: &TikTokPreflight) -> Vec<PolicyCheck> {
    let mut checks = Vec::new();

    // 1. TikTok Title / Caption Length (max 2,200 chars)
    add_check(
        &mut checks,
        "tiktok_caption_length",
        !preflight.description.trim().is_empty() && preflight.description.chars().count() <= 2200,
        "TikTok post caption must contain 1 to 2,200 characters",
    );

    // 2. TikTok Community Guidelines Compliance (Safety, Civility, Authenticity)
    add_check(
        &mut checks,
        "tiktok_community_guidelines",
        preflight.community_guidelines_compliant && !preflight.account_strike_visible,
        "Content and account must strictly adhere to TikTok Community Guidelines (zero strikes)",
    );

    // 3. TikTok AI-Generated Content (AIGC) Labeling Policy
    let aigc_valid = if preflight.contains_synthetic_media {
        preflight.aigc_label_enabled
    } else {
        true
    };
    add_check(
        &mut checks,
        "tiktok_aigc_disclosure",
        aigc_valid,
        "Realistic AI voice, avatars, or procedural visuals must have the TikTok AIGC label enabled",
    );

    // 4. TikTok Monetization / Creator Rewards Program: Originality Requirement
    add_check(
        &mut checks,
        "tiktok_monetization_originality",
        preflight.is_original,
        "Creator Rewards Program requires 100% original, transformative educational media (no scraped or unedited clips)",
    );

    // 5. TikTok Monetization / Creator Rewards Program: Duration Requirement (>= 60s for RPM)
    add_check(
        &mut checks,
        "tiktok_monetization_duration",
        preflight.duration_seconds >= 60.0,
        "Creator Rewards Program monetization payouts require video duration >= 60 seconds (1+ minute)",
    );

    // 6. Commercial Music Rights (Commercial Music Library or original stem)
    add_check(
        &mut checks,
        "tiktok_commercial_music_rights",
        preflight.commercial_music_cleared,
        "Monetized TikTok videos must use audio from the TikTok Commercial Music Library (CML) or original score",
    );

    checks
}

fn add_check(checks: &mut Vec<PolicyCheck>, name: &str, passed: bool, detail: &str) {
    checks.push(PolicyCheck {
        name: name.into(),
        status: if passed {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        detail: detail.into(),
    });
}

fn normalize_html(html: &str) -> AtlasResult<String> {
    let script = Regex::new("(?is)<script[^>]*>.*?</script>")
        .map_err(|error| AtlasError::Other(error.to_string()))?;
    let style = Regex::new("(?is)<style[^>]*>.*?</style>")
        .map_err(|error| AtlasError::Other(error.to_string()))?;
    let tags = Regex::new("(?is)<[^>]+>").map_err(|error| AtlasError::Other(error.to_string()))?;
    let spaces = Regex::new(r"\s+").map_err(|error| AtlasError::Other(error.to_string()))?;
    let without_script = script.replace_all(html, " ");
    let without_style = style.replace_all(&without_script, " ");
    let text = tags.replace_all(&without_style, " ");
    let normalized = spaces.replace_all(&text, " ").trim().to_lowercase();
    Ok(normalized
        .rsplit_once("this help content &amp; information")
        .map(|(stable, _)| stable)
        .unwrap_or(&normalized)
        .trim()
        .to_string())
}

fn changed_pages(accepted: &PolicySnapshot, current: &PolicySnapshot) -> String {
    let changed = current
        .pages
        .iter()
        .filter(|page| {
            accepted
                .pages
                .iter()
                .find(|candidate| candidate.name == page.name)
                .map(|candidate| candidate.normalized_sha256 != page.normalized_sha256)
                .unwrap_or(true)
        })
        .map(|page| page.name.as_str())
        .collect::<Vec<_>>();
    format!(
        "Official YouTube policy changed: {}. Review the current pages, then run `atlas policy accept-current`",
        changed.join(", ")
    )
}

fn read_snapshot(path: &Path) -> AtlasResult<Option<PolicySnapshot>> {
    read_json(path)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> AtlasResult<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
}

fn write_json(path: &Path, value: &impl Serialize) -> AtlasResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_content() -> ContentPreflight {
        ContentPreflight {
            title: "Why batteries age".into(),
            description: "AI-assisted educational animation.".into(),
            transcript: "A battery moves ions between two electrodes.".into(),
            contains_synthetic_media: true,
            captions_present: true,
            thumbnail_present: true,
            broll_expected: true,
            broll_attribution_present: true,
            unique_media: true,
        }
    }

    #[test]
    fn complete_content_passes() {
        assert!(
            content_checks(&complete_content())
                .iter()
                .all(|check| check.status == CheckStatus::Pass)
        );
    }

    #[test]
    fn missing_disclosure_and_captions_fail() {
        let mut content = complete_content();
        content.contains_synthetic_media = false;
        content.description = "Educational animation.".into();
        content.captions_present = false;
        let failed = content_checks(&content)
            .into_iter()
            .filter(|check| check.status == CheckStatus::Fail)
            .map(|check| check.name)
            .collect::<Vec<_>>();
        assert_eq!(
            failed,
            vec![
                "synthetic_media_declaration",
                "viewer_disclosure",
                "captions"
            ]
        );
    }

    #[test]
    fn changed_policy_pages_are_named() {
        let accepted = PolicySnapshot {
            checked_at: Utc::now(),
            pages: vec![PolicyPageSnapshot {
                name: "community_guidelines".into(),
                url: "https://example.test".into(),
                normalized_sha256: "old".into(),
            }],
        };
        let current = PolicySnapshot {
            checked_at: Utc::now(),
            pages: vec![PolicyPageSnapshot {
                name: "community_guidelines".into(),
                url: "https://example.test".into(),
                normalized_sha256: "new".into(),
            }],
        };
        assert!(changed_pages(&accepted, &current).contains("community_guidelines"));
    }

    #[test]
    fn sensitive_content_requires_review() {
        let mut content = complete_content();
        content.transcript = "This promises a guaranteed medical cure.".into();
        assert!(content_checks(&content).iter().any(|check| {
            check.name == "sensitive_content_review" && check.status == CheckStatus::Review
        }));
    }

    #[test]
    fn volatile_help_center_tail_is_ignored() {
        let first = "<main>Stable policy text</main> This help content &amp; information 123";
        let second = "<main>Stable policy text</main> This help content &amp; information 456";
        assert_eq!(
            normalize_html(first).unwrap(),
            normalize_html(second).unwrap()
        );
    }

    #[test]
    fn browser_warning_blocks_channel() {
        let channel = ChannelPreflight {
            channel_id: "channel-1".into(),
            available: true,
            linked: Some(true),
            long_uploads_status: Some("allowed".into()),
            privacy_status: Some("public".into()),
            browser_review: Some(ChannelBrowserReview {
                channel_id: "channel-1".into(),
                reviewed_at: Utc::now(),
                warning_visible: true,
                strike_visible: false,
                training_completed: false,
                warning_expires_at: None,
                restricted_policy: Some("third_party_content_live_streams".into()),
                source_url: "https://studio.youtube.com/channel/channel-1".into(),
            }),
        };
        assert!(channel_checks(&channel).iter().any(|check| {
            check.name == "community_guidelines_channel_state" && check.status == CheckStatus::Fail
        }));
    }

    #[test]
    fn trained_warning_allows_channel() {
        let channel = ChannelPreflight {
            channel_id: "channel-1".into(),
            available: true,
            linked: Some(true),
            long_uploads_status: Some("allowed".into()),
            privacy_status: Some("public".into()),
            browser_review: Some(ChannelBrowserReview {
                channel_id: "channel-1".into(),
                reviewed_at: Utc::now(),
                warning_visible: true,
                strike_visible: false,
                training_completed: true,
                warning_expires_at: Some(Utc::now() + chrono::Duration::days(90)),
                restricted_policy: Some("third_party_content_live_streams".into()),
                source_url: "https://studio.youtube.com/channel/channel-1".into(),
            }),
        };
        assert!(
            channel_checks(&channel)
                .iter()
                .all(|check| check.status == CheckStatus::Pass)
        );
    }

    #[test]
    fn tiktok_preflight_passes_and_fails_appropriately() {
        let valid_tiktok = TikTokPreflight {
            title: "Inside a Lithium Cell".into(),
            description:
                "How lithium-ion batteries work under the microscope. #Science #Engineering".into(),
            duration_seconds: 75.0,
            is_original: true,
            contains_synthetic_media: true,
            aigc_label_enabled: true,
            commercial_music_cleared: true,
            community_guidelines_compliant: true,
            account_warning_visible: false,
            account_strike_visible: false,
        };
        let checks = tiktok_content_checks(&valid_tiktok);
        assert!(checks.iter().all(|c| c.status == CheckStatus::Pass));

        // Test unlabeled AIGC failure
        let mut unlabeled_aigc = valid_tiktok.clone();
        unlabeled_aigc.aigc_label_enabled = false;
        let checks = tiktok_content_checks(&unlabeled_aigc);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "tiktok_aigc_disclosure" && c.status == CheckStatus::Fail)
        );

        // Test non-original content failure for Creator Rewards
        let mut unoriginal = valid_tiktok.clone();
        unoriginal.is_original = false;
        let checks = tiktok_content_checks(&unoriginal);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "tiktok_monetization_originality"
                    && c.status == CheckStatus::Fail)
        );

        // Test sub-60s duration failure for Creator Rewards monetization
        let mut short_duration = valid_tiktok.clone();
        short_duration.duration_seconds = 31.0;
        let checks = tiktok_content_checks(&short_duration);
        assert!(
            checks
                .iter()
                .any(|c| c.name == "tiktok_monetization_duration" && c.status == CheckStatus::Fail)
        );

        // Test unlicensed personal music failure
        let mut unlicensed_music = valid_tiktok.clone();
        unlicensed_music.commercial_music_cleared = false;
        let checks = tiktok_content_checks(&unlicensed_music);
        assert!(
            checks.iter().any(
                |c| c.name == "tiktok_commercial_music_rights" && c.status == CheckStatus::Fail
            )
        );
    }
}
