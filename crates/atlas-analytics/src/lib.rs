pub mod compensation;
pub mod models;
pub mod optimization;
pub mod publishing;
pub mod recommend;
pub mod store;
pub mod tiktok;
pub mod youtube;

pub use compensation::{MonetizationEvidence, SimulatedPayrollLedger, SimulatedPayrollRun};
pub use models::*;
pub use optimization::OptimizationLoop;
pub use publishing::{
    PublishCadencePolicy, PublishDecision, PublishLedger, PublishSchedulerState, PublishService,
    PublishedFormat, PublishedItem, media_fingerprint,
};
pub use recommend::RecommendationEngine;
pub use store::AnalyticsStore;
pub use tiktok::{
    DEFAULT_TIKTOK_USER, DEFAULT_UPLOAD_POST_BASE_URL, TikTokClient, TikTokCredentials,
    TikTokPostMode, TikTokPrivacyLevel, TikTokUploadMetadata, TikTokUploadResult,
    TikTokUploadStatus, TikTokUserStatus, load_tiktok_credentials,
};
pub use youtube::{
    TokenStore, YouTubeAnalyticsClient, YouTubeChannelStatus, YouTubeCredentials, YouTubeTokenSet,
    YouTubeUploadMetadata, YouTubeUploadResult, authorization_url, load_youtube_credentials,
};

use atlas_core::AtlasResult;
use atlas_store::Store;
use chrono::Utc;

/// Computes channel baseline from a collection of stored metrics.
pub fn compute_channel_baseline(metrics: &[VideoMetrics]) -> ChannelBaseline {
    let n = metrics.len().max(1);
    let avg_duration: f64 = metrics
        .iter()
        .filter_map(|m| m.average_view_duration_seconds)
        .sum::<f64>()
        / n as f64;
    let avg_pct: f64 = metrics
        .iter()
        .filter_map(|m| m.average_percentage_viewed)
        .sum::<f64>()
        / n as f64;
    let avg_ctr: f64 = metrics
        .iter()
        .filter_map(|m| m.impressions_click_through_rate)
        .sum::<f64>()
        / n as f64;
    let median_views = metrics
        .iter()
        .filter_map(|m| m.views)
        .nth(metrics.len() / 2)
        .unwrap_or(0);
    ChannelBaseline {
        average_view_duration_seconds: avg_duration,
        average_percentage_viewed: avg_pct,
        average_ctr: avg_ctr,
        median_views,
        sample_size: metrics.len(),
    }
}

/// Builds a performance comparison for a single video against the channel baseline.
pub fn compare_to_baseline(metrics: &VideoMetrics, baseline: &ChannelBaseline) -> VideoPerformance {
    let pct_retention = percentile_for(
        metrics.average_percentage_viewed,
        baseline.average_percentage_viewed,
    );
    let pct_duration = percentile_for(
        metrics.average_view_duration_seconds,
        baseline.average_view_duration_seconds,
    );
    let pct_ctr = percentile_for(metrics.impressions_click_through_rate, baseline.average_ctr);

    VideoPerformance {
        video_id: metrics.video_id.clone(),
        metrics: metrics.clone(),
        channel_average: baseline.clone(),
        percentiles: PerformancePercentiles {
            view_duration_vs_channel: pct_duration,
            retention_vs_channel: pct_retention,
            ctr_vs_channel: pct_ctr,
        },
    }
}

fn percentile_for(value: Option<f64>, baseline: f64) -> f64 {
    match value {
        Some(v) if baseline > 0.0 => (v / baseline).clamp(0.0, 2.0) / 2.0,
        _ => 0.5,
    }
}

/// Saves a sample metrics snapshot (useful for tests and demos without live API access).
pub fn save_sample_metrics(
    store: &Store,
    video_id: &str,
    youtube_video_id: &str,
) -> AtlasResult<VideoMetrics> {
    let metrics = VideoMetrics {
        video_id: video_id.to_string(),
        youtube_video_id: youtube_video_id.to_string(),
        retrieved_at: Utc::now(),
        platform: "youtube".into(),
        format: "long".into(),
        views: Some(12_450),
        watch_time_minutes: Some(2_489.0),
        average_view_duration_seconds: Some(120.0),
        average_percentage_viewed: Some(0.24),
        impressions: Some(145_000),
        impressions_click_through_rate: Some(0.023),
        likes: Some(890),
        comments: Some(134),
        shares: Some(78),
        subscribers_gained: Some(45),
        subscribers_lost: Some(2),
        retention_curve: vec![
            RetentionPoint {
                elapsed_ratio: 0.0,
                retention_ratio: 1.0,
                elapsed_seconds: 0,
            },
            RetentionPoint {
                elapsed_ratio: 0.05,
                retention_ratio: 0.82,
                elapsed_seconds: 25,
            },
            RetentionPoint {
                elapsed_ratio: 0.10,
                retention_ratio: 0.61,
                elapsed_seconds: 50,
            },
            RetentionPoint {
                elapsed_ratio: 0.20,
                retention_ratio: 0.44,
                elapsed_seconds: 100,
            },
            RetentionPoint {
                elapsed_ratio: 0.30,
                retention_ratio: 0.38,
                elapsed_seconds: 150,
            },
            RetentionPoint {
                elapsed_ratio: 0.50,
                retention_ratio: 0.31,
                elapsed_seconds: 250,
            },
            RetentionPoint {
                elapsed_ratio: 0.75,
                retention_ratio: 0.22,
                elapsed_seconds: 375,
            },
            RetentionPoint {
                elapsed_ratio: 1.0,
                retention_ratio: 0.16,
                elapsed_seconds: 500,
            },
        ],
        traffic_sources: vec![
            TrafficSource {
                source: "Browse features".into(),
                views: 5200,
            },
            TrafficSource {
                source: "Suggested videos".into(),
                views: 4100,
            },
            TrafficSource {
                source: "YouTube search".into(),
                views: 2100,
            },
            TrafficSource {
                source: "External".into(),
                views: 1050,
            },
        ],
        demographics: vec![
            DemographicSlice {
                dimension: "age".into(),
                value: "18-24".into(),
                views: 3200,
            },
            DemographicSlice {
                dimension: "age".into(),
                value: "25-34".into(),
                views: 5400,
            },
            DemographicSlice {
                dimension: "gender".into(),
                value: "male".into(),
                views: 7200,
            },
            DemographicSlice {
                dimension: "gender".into(),
                value: "female".into(),
                views: 5230,
            },
        ],
    };
    let analytics = AnalyticsStore::new(store);
    analytics.save_metrics(&metrics)?;
    Ok(metrics)
}

/// Saves a sample vertical Short metrics snapshot (useful for TikTok and YouTube Shorts evaluation).
pub fn save_sample_short_metrics(
    store: &Store,
    video_id: &str,
    platform: &str,
) -> AtlasResult<VideoMetrics> {
    let metrics = VideoMetrics {
        video_id: video_id.to_string(),
        youtube_video_id: format!("{}_{}", video_id, platform),
        retrieved_at: Utc::now(),
        platform: platform.to_string(),
        format: "short".to_string(),
        views: Some(38_400),
        watch_time_minutes: Some(12_288.0),
        average_view_duration_seconds: Some(19.2),
        average_percentage_viewed: Some(0.64),
        impressions: Some(210_000),
        impressions_click_through_rate: Some(0.068),
        likes: Some(3_250),
        comments: Some(218),
        shares: Some(412),
        subscribers_gained: Some(164),
        subscribers_lost: Some(1),
        retention_curve: vec![
            RetentionPoint {
                elapsed_ratio: 0.0,
                retention_ratio: 1.0,
                elapsed_seconds: 0,
            },
            RetentionPoint {
                elapsed_ratio: 0.10,
                retention_ratio: 0.88,
                elapsed_seconds: 3,
            },
            RetentionPoint {
                elapsed_ratio: 0.20,
                retention_ratio: 0.76,
                elapsed_seconds: 6,
            },
            RetentionPoint {
                elapsed_ratio: 0.40,
                retention_ratio: 0.64,
                elapsed_seconds: 12,
            },
            RetentionPoint {
                elapsed_ratio: 0.60,
                retention_ratio: 0.55,
                elapsed_seconds: 18,
            },
            RetentionPoint {
                elapsed_ratio: 0.80,
                retention_ratio: 0.48,
                elapsed_seconds: 24,
            },
            RetentionPoint {
                elapsed_ratio: 1.0,
                retention_ratio: 0.42,
                elapsed_seconds: 30,
            },
        ],
        traffic_sources: vec![
            TrafficSource {
                source: if platform.eq_ignore_ascii_case("tiktok") {
                    "For You Feed"
                } else {
                    "Shorts feed"
                }
                .into(),
                views: 31_200,
            },
            TrafficSource {
                source: "Search".into(),
                views: 4_300,
            },
            TrafficSource {
                source: "Personal profile".into(),
                views: 2_900,
            },
        ],
        demographics: vec![
            DemographicSlice {
                dimension: "age".into(),
                value: "18-24".into(),
                views: 18_400,
            },
            DemographicSlice {
                dimension: "age".into(),
                value: "25-34".into(),
                views: 14_100,
            },
        ],
    };
    let analytics = AnalyticsStore::new(store);
    analytics.save_metrics(&metrics)?;
    Ok(metrics)
}
