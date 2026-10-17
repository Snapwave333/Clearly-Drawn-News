use atlas_core::{AtlasError, AtlasResult};
use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublishedFormat {
    Long,
    Short,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishedItem {
    pub project_id: String,
    pub youtube_video_id: String,
    pub format: PublishedFormat,
    pub published_at: DateTime<Utc>,
    pub content_fingerprint: String,
    #[serde(default = "default_platform")]
    pub platform: String,
    #[serde(default)]
    pub tiktok_post_id: Option<String>,
}

fn default_platform() -> String {
    "youtube".into()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PublishLedger {
    pub items: Vec<PublishedItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishCadencePolicy {
    pub max_public_per_24_hours: usize,
    pub max_long_per_24_hours: usize,
    pub max_shorts_per_24_hours: usize,
    pub max_tiktok_per_24_hours: usize,
    pub minimum_spacing_minutes: i64,
    pub video_insert_daily_reserve: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishDecision {
    Allow,
    Defer {
        retry_at: DateTime<Utc>,
        reason: String,
    },
    RejectDuplicate {
        existing_video_id: String,
    },
}

impl Default for PublishCadencePolicy {
    fn default() -> Self {
        Self {
            max_public_per_24_hours: 3,
            max_long_per_24_hours: 1,
            max_shorts_per_24_hours: 2,
            max_tiktok_per_24_hours: 15,
            minimum_spacing_minutes: 240,
            video_insert_daily_reserve: 10,
        }
    }
}

impl PublishLedger {
    pub fn load(path: &Path) -> AtlasResult<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&std::fs::read(path)?).map_err(|error| {
            AtlasError::Config(format!(
                "Invalid publish ledger {}: {error}",
                path.display()
            ))
        })
    }

    pub fn save(&self, path: &Path) -> AtlasResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            path,
            serde_json::to_vec_pretty(self).map_err(|error| {
                AtlasError::Config(format!("Unable to serialize publish ledger: {error}"))
            })?,
        )?;
        Ok(())
    }

    pub fn decide(
        &self,
        policy: &PublishCadencePolicy,
        format: PublishedFormat,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> PublishDecision {
        if let Some(existing) = self
            .items
            .iter()
            .find(|item| item.content_fingerprint == fingerprint)
        {
            return PublishDecision::RejectDuplicate {
                existing_video_id: existing.youtube_video_id.clone(),
            };
        }
        let window_start = now - Duration::hours(24);
        let recent = self
            .items
            .iter()
            .filter(|item| item.published_at > window_start)
            .collect::<Vec<_>>();
        let same_format = recent.iter().filter(|item| item.format == format).count();
        let format_limit = match format {
            PublishedFormat::Long => policy.max_long_per_24_hours,
            PublishedFormat::Short => policy.max_shorts_per_24_hours,
        };
        if recent.len() >= policy.max_public_per_24_hours || same_format >= format_limit {
            let retry_at = recent
                .iter()
                .map(|item| item.published_at + Duration::hours(24))
                .min()
                .unwrap_or(now + Duration::hours(24));
            return PublishDecision::Defer {
                retry_at,
                reason: "rolling 24-hour publication cap reached".into(),
            };
        }
        if let Some(last) = recent.iter().map(|item| item.published_at).max() {
            let retry_at = last + Duration::minutes(policy.minimum_spacing_minutes);
            if retry_at > now {
                return PublishDecision::Defer {
                    retry_at,
                    reason: "minimum spacing between public videos has not elapsed".into(),
                };
            }
        }
        PublishDecision::Allow
    }

    pub fn decide_tiktok(
        &self,
        policy: &PublishCadencePolicy,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> PublishDecision {
        if let Some(existing) = self
            .items
            .iter()
            .find(|item| item.platform == "tiktok" && item.content_fingerprint == fingerprint)
        {
            return PublishDecision::RejectDuplicate {
                existing_video_id: existing
                    .tiktok_post_id
                    .clone()
                    .unwrap_or_else(|| existing.youtube_video_id.clone()),
            };
        }
        let window_start = now - Duration::hours(24);
        let recent_tiktok = self
            .items
            .iter()
            .filter(|item| item.platform == "tiktok" && item.published_at > window_start)
            .count();
        if recent_tiktok >= policy.max_tiktok_per_24_hours {
            return PublishDecision::Defer {
                retry_at: now + Duration::hours(1),
                reason: "TikTok 24-hour limit (15 posts) reached".into(),
            };
        }
        PublishDecision::Allow
    }

    pub fn record(&mut self, item: PublishedItem) {
        self.items.push(item);
        let cutoff = Utc::now() - Duration::days(90);
        self.items.retain(|entry| entry.published_at > cutoff);
    }
}

pub fn media_fingerprint(path: &Path) -> AtlasResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

pub fn next_daily_reset(now: DateTime<Utc>) -> DateTime<Utc> {
    let pacific_offset_hours = if (3..=11).contains(&now.month()) {
        7
    } else {
        8
    };
    let pacific = now - Duration::hours(pacific_offset_hours);
    let next_midnight = (pacific + Duration::days(1))
        .with_hour(0)
        .and_then(|value| value.with_minute(0))
        .and_then(|value| value.with_second(0))
        .unwrap_or(pacific + Duration::days(1));
    next_midnight + Duration::hours(pacific_offset_hours)
}

use chrono::Datelike;

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, format: PublishedFormat, hours_ago: i64) -> PublishedItem {
        PublishedItem {
            project_id: id.into(),
            youtube_video_id: id.into(),
            format,
            published_at: Utc::now() - Duration::hours(hours_ago),
            content_fingerprint: id.into(),
            platform: "youtube".into(),
            tiktok_post_id: None,
        }
    }

    #[test]
    fn enforces_format_and_daily_caps() {
        let ledger = PublishLedger {
            items: vec![
                item("long", PublishedFormat::Long, 10),
                item("short", PublishedFormat::Short, 5),
            ],
        };
        assert!(matches!(
            ledger.decide(
                &PublishCadencePolicy::default(),
                PublishedFormat::Long,
                "new",
                Utc::now()
            ),
            PublishDecision::Defer { .. }
        ));
    }

    #[test]
    fn rejects_duplicate_media() {
        let ledger = PublishLedger {
            items: vec![item("same", PublishedFormat::Short, 30)],
        };
        assert!(matches!(
            ledger.decide(
                &PublishCadencePolicy::default(),
                PublishedFormat::Short,
                "same",
                Utc::now()
            ),
            PublishDecision::RejectDuplicate { .. }
        ));
    }

    #[test]
    fn enforces_tiktok_duplicate_and_caps() {
        let mut tiktok_item = item("tiktok1", PublishedFormat::Short, 2);
        tiktok_item.platform = "tiktok".into();
        tiktok_item.tiktok_post_id = Some("post_123".into());

        let ledger = PublishLedger {
            items: vec![tiktok_item],
        };

        // Rejects duplicate on TikTok
        assert!(matches!(
            ledger.decide_tiktok(&PublishCadencePolicy::default(), "tiktok1", Utc::now()),
            PublishDecision::RejectDuplicate { .. }
        ));

        // Allows fresh fingerprint
        assert!(matches!(
            ledger.decide_tiktok(
                &PublishCadencePolicy::default(),
                "fresh_fingerprint",
                Utc::now()
            ),
            PublishDecision::Allow
        ));
    }
}
