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
    pub youtube_minimum_spacing_minutes: i64,
    pub tiktok_minimum_spacing_minutes: i64,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishService {
    YouTube,
    TikTok,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServiceBackoff {
    pub consecutive_failures: u32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PublishSchedulerState {
    pub youtube: ServiceBackoff,
    pub tiktok: ServiceBackoff,
}

impl PublishSchedulerState {
    pub fn load(path: &Path) -> AtlasResult<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&std::fs::read(path)?).map_err(|error| {
            AtlasError::Config(format!(
                "Invalid publish scheduler state {}: {error}",
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
                AtlasError::Config(format!("Unable to serialize scheduler state: {error}"))
            })?,
        )?;
        Ok(())
    }

    pub fn can_attempt(&self, service: PublishService, now: DateTime<Utc>) -> bool {
        self.service(service)
            .next_attempt_at
            .is_none_or(|retry_at| retry_at <= now)
    }

    pub fn record_success(&mut self, service: PublishService) {
        *self.service_mut(service) = ServiceBackoff::default();
    }

    pub fn record_failure(
        &mut self,
        service: PublishService,
        error: &str,
        now: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let normalized = error.to_ascii_lowercase();
        let failures = self.service(service).consecutive_failures.saturating_add(1);
        let quota_exhausted = normalized.contains("dailylimitexceeded")
            || normalized.contains("quotaexceeded")
            || normalized.contains("daily upload limit")
            || normalized.contains("spam_risk_too_many_pending_share");
        let throttled = normalized.contains("429")
            || normalized.contains("rate_limit")
            || normalized.contains("too many requests");
        let retry_at = if quota_exhausted {
            match service {
                PublishService::YouTube => next_daily_reset(now),
                PublishService::TikTok => now + Duration::hours(24),
            }
        } else {
            let base_minutes = if throttled { 15_i64 } else { 5_i64 };
            let multiplier = 2_i64.pow(failures.saturating_sub(1).min(6));
            now + Duration::minutes((base_minutes * multiplier).min(24 * 60))
        };
        *self.service_mut(service) = ServiceBackoff {
            consecutive_failures: failures,
            next_attempt_at: Some(retry_at),
            last_error: Some(error.chars().take(1000).collect()),
        };
        retry_at
    }

    pub fn next_attempt_at(&self, service: PublishService) -> Option<DateTime<Utc>> {
        self.service(service).next_attempt_at
    }

    fn service(&self, service: PublishService) -> &ServiceBackoff {
        match service {
            PublishService::YouTube => &self.youtube,
            PublishService::TikTok => &self.tiktok,
        }
    }

    fn service_mut(&mut self, service: PublishService) -> &mut ServiceBackoff {
        match service {
            PublishService::YouTube => &mut self.youtube,
            PublishService::TikTok => &mut self.tiktok,
        }
    }
}

impl Default for PublishCadencePolicy {
    fn default() -> Self {
        Self {
            max_public_per_24_hours: 12,
            max_long_per_24_hours: 4,
            max_shorts_per_24_hours: 8,
            max_tiktok_per_24_hours: 12,
            youtube_minimum_spacing_minutes: 120,
            tiktok_minimum_spacing_minutes: 90,
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
            let retry_at = last + Duration::minutes(policy.youtube_minimum_spacing_minutes);
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
            .collect::<Vec<_>>();
        if recent_tiktok.len() >= policy.max_tiktok_per_24_hours {
            let retry_at = recent_tiktok
                .iter()
                .map(|item| item.published_at + Duration::hours(24))
                .min()
                .unwrap_or(now + Duration::hours(24));
            return PublishDecision::Defer {
                retry_at,
                reason: format!(
                    "TikTok rolling 24-hour safety cap ({} posts) reached",
                    policy.max_tiktok_per_24_hours
                ),
            };
        }
        if let Some(last) = recent_tiktok.iter().map(|item| item.published_at).max() {
            let retry_at = last + Duration::minutes(policy.tiktok_minimum_spacing_minutes);
            if retry_at > now {
                return PublishDecision::Defer {
                    retry_at,
                    reason: "TikTok minimum publication spacing has not elapsed".into(),
                };
            }
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
        let policy = PublishCadencePolicy::default();
        let mut items = Vec::new();
        for i in 0..policy.max_long_per_24_hours {
            items.push(item(&format!("long_{i}"), PublishedFormat::Long, (i as i64) + 1));
        }
        let ledger = PublishLedger { items };
        assert!(matches!(
            ledger.decide(
                &policy,
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
        let mut tiktok_item = item("tiktok1", PublishedFormat::Short, 1);
        tiktok_item.platform = "tiktok".into();
        tiktok_item.tiktok_post_id = Some("post_123".into());

        let ledger = PublishLedger {
            items: vec![tiktok_item],
        };

        assert!(matches!(
            ledger.decide_tiktok(&PublishCadencePolicy::default(), "tiktok1", Utc::now()),
            PublishDecision::RejectDuplicate { .. }
        ));

        assert!(matches!(
            ledger.decide_tiktok(
                &PublishCadencePolicy::default(),
                "fresh_fingerprint",
                Utc::now()
            ),
            PublishDecision::Defer { .. }
        ));
    }

    #[test]
    fn allows_tiktok_after_spacing_window() {
        let mut tiktok_item = item("tiktok1", PublishedFormat::Short, 4);
        tiktok_item.platform = "tiktok".into();
        let ledger = PublishLedger {
            items: vec![tiktok_item],
        };
        assert!(matches!(
            ledger.decide_tiktok(
                &PublishCadencePolicy::default(),
                "fresh_fingerprint",
                Utc::now()
            ),
            PublishDecision::Allow
        ));
    }

    #[test]
    fn backs_off_rate_limits_exponentially() {
        let now = Utc::now();
        let mut state = PublishSchedulerState::default();
        let first = state.record_failure(PublishService::TikTok, "429 rate_limit_exceeded", now);
        let second = state.record_failure(PublishService::TikTok, "429 rate_limit_exceeded", now);
        assert_eq!(first, now + Duration::minutes(15));
        assert_eq!(second, now + Duration::minutes(30));
        assert!(!state.can_attempt(PublishService::TikTok, now));
    }

    #[test]
    fn defers_daily_quota_to_platform_reset() {
        let now = Utc::now();
        let mut state = PublishSchedulerState::default();
        let retry_at = state.record_failure(PublishService::YouTube, "403 dailyLimitExceeded", now);
        assert_eq!(retry_at, next_daily_reset(now));
    }
}
