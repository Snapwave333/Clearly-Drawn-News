use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_platform_metrics() -> String {
    "youtube".into()
}

fn default_format_metrics() -> String {
    "long".into()
}

/// Aggregate performance metrics for a published video.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VideoMetrics {
    pub video_id: String,
    pub youtube_video_id: String,
    pub retrieved_at: DateTime<Utc>,
    #[serde(default = "default_platform_metrics")]
    pub platform: String,
    #[serde(default = "default_format_metrics")]
    pub format: String,
    pub views: Option<u64>,
    pub watch_time_minutes: Option<f64>,
    pub average_view_duration_seconds: Option<f64>,
    pub average_percentage_viewed: Option<f64>,
    pub impressions: Option<u64>,
    pub impressions_click_through_rate: Option<f64>,
    pub likes: Option<u64>,
    pub comments: Option<u64>,
    pub shares: Option<u64>,
    pub subscribers_gained: Option<u64>,
    pub subscribers_lost: Option<u64>,
    pub retention_curve: Vec<RetentionPoint>,
    pub traffic_sources: Vec<TrafficSource>,
    pub demographics: Vec<DemographicSlice>,
}

/// A single point on an audience-retention curve.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RetentionPoint {
    /// Elapsed ratio through the video, 0.0–1.0.
    pub elapsed_ratio: f64,
    /// Percentage of viewers still watching at this point, 0.0–1.0.
    pub retention_ratio: f64,
    /// Absolute elapsed seconds.
    pub elapsed_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrafficSource {
    pub source: String,
    pub views: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DemographicSlice {
    pub dimension: String,
    pub value: String,
    pub views: u64,
}

/// A video compared to the channel's recent baseline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoPerformance {
    pub video_id: String,
    pub metrics: VideoMetrics,
    pub channel_average: ChannelBaseline,
    pub percentiles: PerformancePercentiles,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChannelBaseline {
    pub average_view_duration_seconds: f64,
    pub average_percentage_viewed: f64,
    pub average_ctr: f64,
    pub median_views: u64,
    pub sample_size: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PerformancePercentiles {
    pub view_duration_vs_channel: f64,
    pub retention_vs_channel: f64,
    pub ctr_vs_channel: f64,
}

/// A specific, actionable recommendation produced from analytics data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OptimizationRecommendation {
    pub recommendation_id: String,
    pub priority: RecommendationPriority,
    pub category: RecommendationCategory,
    pub issue: String,
    pub evidence: String,
    pub suggested_action: String,
    pub affected_timecodes: Option<Vec<String>>,
    pub expected_impact: ExpectedImpact,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationPriority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationCategory {
    Hook,
    Pacing,
    Retention,
    Thumbnail,
    Title,
    Audio,
    Visual,
    Topic,
    Structure,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExpectedImpact {
    pub metric: String,
    pub direction: String,
    pub rationale: String,
}

impl VideoMetrics {
    /// Find the largest single retention drop between consecutive points.
    pub fn largest_retention_drop(&self) -> Option<(usize, f64)> {
        let mut max_drop: f64 = 0.0;
        let mut max_idx = None;
        for (i, window) in self.retention_curve.windows(2).enumerate() {
            let drop = window[0].retention_ratio - window[1].retention_ratio;
            if drop > max_drop {
                max_drop = drop;
                max_idx = Some(i + 1);
            }
        }
        max_idx.map(|idx| (idx, max_drop))
    }

    /// Average retention in the first 30 seconds.
    pub fn hook_retention(&self, video_duration_seconds: u64) -> Option<f64> {
        let hook_end = 30.0 / video_duration_seconds.max(1) as f64;
        let points: Vec<&RetentionPoint> = self
            .retention_curve
            .iter()
            .filter(|p| p.elapsed_ratio <= hook_end)
            .collect();
        if points.is_empty() {
            None
        } else {
            Some(points.iter().map(|p| p.retention_ratio).sum::<f64>() / points.len() as f64)
        }
    }

    /// For short-form content (<= 60s), retention in the first 3 seconds (critical hook window).
    pub fn short_hook_retention(&self) -> Option<f64> {
        let points: Vec<&RetentionPoint> = self
            .retention_curve
            .iter()
            .filter(|p| p.elapsed_seconds <= 3)
            .collect();
        if points.is_empty() {
            self.retention_curve.first().map(|p| p.retention_ratio)
        } else {
            points.last().map(|p| p.retention_ratio)
        }
    }

    /// Completion rate: percentage of viewers watching until the final retention sample.
    pub fn completion_rate(&self) -> Option<f64> {
        self.retention_curve.last().map(|p| p.retention_ratio)
    }
}

/// Convert raw YouTube Analytics rows into structured metrics.
pub fn rows_to_metrics(
    video_id: &str,
    youtube_video_id: &str,
    rows: &[serde_json::Value],
) -> VideoMetrics {
    let mut metrics = VideoMetrics {
        video_id: video_id.to_string(),
        youtube_video_id: youtube_video_id.to_string(),
        retrieved_at: Utc::now(),
        ..Default::default()
    };
    for row in rows {
        // Simplified parsing: rows contain dimensions + metrics from the API.
        if let Some(dims) = row.get("dimensions").and_then(|v| v.as_array()) {
            if dims.is_empty() {
                if let Some(vals) = row.get("metrics").and_then(|v| v.as_array()) {
                    if vals.len() >= 8 {
                        metrics.views = vals[0].as_str().and_then(|s| s.parse().ok());
                        metrics.watch_time_minutes = vals[1].as_str().and_then(|s| s.parse().ok());
                        metrics.average_view_duration_seconds =
                            vals[2].as_str().and_then(|s| s.parse().ok());
                        metrics.average_percentage_viewed =
                            vals[3].as_str().and_then(|s| s.parse().ok());
                        metrics.impressions = vals[4].as_str().and_then(|s| s.parse().ok());
                        metrics.impressions_click_through_rate =
                            vals[5].as_str().and_then(|s| s.parse().ok());
                        metrics.likes = vals[6].as_str().and_then(|s| s.parse().ok());
                        metrics.comments = vals[7].as_str().and_then(|s| s.parse().ok());
                    }
                }
            }
        }
    }
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn largest_retention_drop_finds_biggest_cliff() {
        let metrics = VideoMetrics {
            video_id: "v1".into(),
            youtube_video_id: "yt1".into(),
            retrieved_at: Utc::now(),
            retention_curve: vec![
                RetentionPoint {
                    elapsed_ratio: 0.0,
                    retention_ratio: 1.0,
                    elapsed_seconds: 0,
                },
                RetentionPoint {
                    elapsed_ratio: 0.1,
                    retention_ratio: 0.8,
                    elapsed_seconds: 10,
                },
                RetentionPoint {
                    elapsed_ratio: 0.2,
                    retention_ratio: 0.4,
                    elapsed_seconds: 20,
                },
                RetentionPoint {
                    elapsed_ratio: 0.3,
                    retention_ratio: 0.35,
                    elapsed_seconds: 30,
                },
            ],
            ..Default::default()
        };
        assert_eq!(metrics.largest_retention_drop(), Some((2, 0.4)));
    }

    #[test]
    fn hook_retention_averages_first_thirty_seconds() {
        let metrics = VideoMetrics {
            video_id: "v1".into(),
            youtube_video_id: "yt1".into(),
            retrieved_at: Utc::now(),
            retention_curve: vec![
                RetentionPoint {
                    elapsed_ratio: 0.0,
                    retention_ratio: 1.0,
                    elapsed_seconds: 0,
                },
                RetentionPoint {
                    elapsed_ratio: 0.05,
                    retention_ratio: 0.9,
                    elapsed_seconds: 5,
                },
                RetentionPoint {
                    elapsed_ratio: 0.1,
                    retention_ratio: 0.8,
                    elapsed_seconds: 10,
                },
            ],
            ..Default::default()
        };
        assert!((metrics.hook_retention(100).unwrap() - 0.9).abs() < 0.01);
    }

    #[test]
    fn short_hook_and_completion_rate_computed_correctly() {
        let metrics = VideoMetrics {
            video_id: "short_1".into(),
            youtube_video_id: "yt_short".into(),
            retrieved_at: Utc::now(),
            platform: "tiktok".into(),
            format: "short".into(),
            retention_curve: vec![
                RetentionPoint {
                    elapsed_ratio: 0.0,
                    retention_ratio: 1.0,
                    elapsed_seconds: 0,
                },
                RetentionPoint {
                    elapsed_ratio: 0.1,
                    retention_ratio: 0.88,
                    elapsed_seconds: 3,
                },
                RetentionPoint {
                    elapsed_ratio: 0.5,
                    retention_ratio: 0.55,
                    elapsed_seconds: 15,
                },
                RetentionPoint {
                    elapsed_ratio: 1.0,
                    retention_ratio: 0.42,
                    elapsed_seconds: 30,
                },
            ],
            ..Default::default()
        };
        assert_eq!(metrics.short_hook_retention(), Some(0.88));
        assert_eq!(metrics.completion_rate(), Some(0.42));
    }
}
