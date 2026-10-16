use crate::models::*;
use uuid::Uuid;

/// Generates retention-driven recommendations for a video.
pub struct RecommendationEngine;

impl RecommendationEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn analyze(&self, performance: &VideoPerformance, video_duration_seconds: u64) -> Vec<OptimizationRecommendation> {
        let mut recs = Vec::new();

        // Hook analysis: first 30 seconds vs channel baseline.
        if let Some(hook) = performance.metrics.hook_retention(video_duration_seconds) {
            if hook < performance.channel_average.average_percentage_viewed * 0.9 {
                recs.push(OptimizationRecommendation {
                    recommendation_id: format!("rec_{}", Uuid::new_v4().simple()),
                    priority: RecommendationPriority::Critical,
                    category: RecommendationCategory::Hook,
                    issue: "Hook retention is below the channel baseline.".to_string(),
                    evidence: format!(
                        "First-30s retention {:.1}% vs channel average {:.1}%.",
                        hook * 100.0,
                        performance.channel_average.average_percentage_viewed * 100.0
                    ),
                    suggested_action: "Rewrite the opening question/visual to establish stakes faster; test a stronger concrete image in the first 5 seconds.".to_string(),
                    affected_timecodes: Some(vec!["00:00-00:30".to_string()]),
                    expected_impact: ExpectedImpact {
                        metric: "average_view_duration".to_string(),
                        direction: "increase".to_string(),
                        rationale: "Higher hook retention lifts overall average view duration and signals quality to the recommendation algorithm.".to_string(),
                    },
                });
            }
        }

        // Largest retention cliff.
        if let Some((idx, drop)) = performance.metrics.largest_retention_drop() {
            if drop > 0.15 {
                if let Some(point) = performance.metrics.retention_curve.get(idx) {
                    let seconds = point.elapsed_seconds;
                    let mm = seconds / 60;
                    let ss = seconds % 60;
                    let timecode = format!("{:02}:{:02}", mm, ss);
                    recs.push(OptimizationRecommendation {
                        recommendation_id: format!("rec_{}", Uuid::new_v4().simple()),
                        priority: RecommendationPriority::High,
                        category: RecommendationCategory::Pacing,
                        issue: format!("Sharp audience drop ({:.1}%) at {}.", drop * 100.0, timecode),
                        evidence: format!("Retention falls from {:.1}% to {:.1}% across adjacent sample points.",
                            performance.metrics.retention_curve[idx - 1].retention_ratio * 100.0,
                            point.retention_ratio * 100.0),
                        suggested_action: "Insert a visual reveal, tighten the explanation, or break the section into two shorter beats.".to_string(),
                        affected_timecodes: Some(vec![format!("{}-{:02}:{:02}", timecode, (point.elapsed_seconds + 15) / 60, (point.elapsed_seconds + 15) % 60)]),
                        expected_impact: ExpectedImpact {
                            metric: "audience_retention".to_string(),
                            direction: "flatten".to_string(),
                            rationale: "Smoothing the largest cliff has outsized impact on total watch time.".to_string(),
                        },
                    });
                }
            }
        }

        // CTR analysis.
        if let Some(ctr) = performance.metrics.impressions_click_through_rate {
            if ctr < performance.channel_average.average_ctr * 0.8 {
                recs.push(OptimizationRecommendation {
                    recommendation_id: format!("rec_{}", Uuid::new_v4().simple()),
                    priority: RecommendationPriority::High,
                    category: RecommendationCategory::Thumbnail,
                    issue: "Click-through rate is well below channel average.".to_string(),
                    evidence: format!("CTR {:.2}% vs channel average {:.2}%.", ctr * 100.0, performance.channel_average.average_ctr * 100.0),
                    suggested_action: "Generate alternate thumbnail concepts with higher contrast, one focal object, and readable text; run internal A/B preview tests.".to_string(),
                    affected_timecodes: None,
                    expected_impact: ExpectedImpact {
                        metric: "impressions_click_through_rate".to_string(),
                        direction: "increase".to_string(),
                        rationale: "CTR is the primary lever for impressions on the browse and suggested surfaces.".to_string(),
                    },
                });
            }
        }

        // End-of-video retention.
        if let Some(last) = performance.metrics.retention_curve.last() {
            if last.retention_ratio < 0.25 {
                recs.push(OptimizationRecommendation {
                    recommendation_id: format!("rec_{}", Uuid::new_v4().simple()),
                    priority: RecommendationPriority::Medium,
                    category: RecommendationCategory::Structure,
                    issue: "Retention is very low by the video's conclusion.".to_string(),
                    evidence: format!("Only {:.1}% of viewers reach the end.", last.retention_ratio * 100.0),
                    suggested_action: "Move the payoff earlier, shorten the conclusion, or add a late-stage reveal to re-engage viewers.".to_string(),
                    affected_timecodes: Some(vec!["end".to_string()]),
                    expected_impact: ExpectedImpact {
                        metric: "average_view_duration".to_string(),
                        direction: "increase".to_string(),
                        rationale: "Late retention affects total watch time and repeat-view probability.".to_string(),
                    },
                });
            }
        }

        // Overall relative to channel.
        if performance.percentiles.retention_vs_channel < 0.35 {
            recs.push(OptimizationRecommendation {
                recommendation_id: format!("rec_{}", Uuid::new_v4().simple()),
                priority: RecommendationPriority::High,
                category: RecommendationCategory::Topic,
                issue: "Overall retention is in the bottom third of recent channel uploads.".to_string(),
                evidence: format!("Percentile vs channel: {:.0}.", performance.percentiles.retention_vs_channel * 100.0),
                suggested_action: "Re-evaluate the topic angle, title promise, and visual thesis; consider shelving similar underperforming concepts.".to_string(),
                affected_timecodes: None,
                expected_impact: ExpectedImpact {
                    metric: "average_percentage_viewed".to_string(),
                    direction: "increase".to_string(),
                    rationale: "Consistently bottom-quartile retention suggests a topic/promise mismatch.".to_string(),
                },
            });
        }

        recs
    }

    /// Ranks recommendations by priority and expected impact.
    pub fn rank(&self, mut recs: Vec<OptimizationRecommendation>) -> Vec<OptimizationRecommendation> {
        recs.sort_by(|a, b| {
            let priority_order = |p: &RecommendationPriority| match p {
                RecommendationPriority::Critical => 0,
                RecommendationPriority::High => 1,
                RecommendationPriority::Medium => 2,
                RecommendationPriority::Low => 3,
            };
            priority_order(&a.priority).cmp(&priority_order(&b.priority))
        });
        recs
    }
}

impl Default for RecommendationEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_metrics() -> VideoMetrics {
        VideoMetrics {
            video_id: "v1".into(),
            youtube_video_id: "yt1".into(),
            retrieved_at: Utc::now(),
            views: Some(10000),
            average_view_duration_seconds: Some(120.0),
            average_percentage_viewed: Some(0.25),
            impressions_click_through_rate: Some(0.02),
            retention_curve: vec![
                RetentionPoint { elapsed_ratio: 0.0, retention_ratio: 1.0, elapsed_seconds: 0 },
                RetentionPoint { elapsed_ratio: 0.05, retention_ratio: 0.35, elapsed_seconds: 15 },
                RetentionPoint { elapsed_ratio: 0.1, retention_ratio: 0.3, elapsed_seconds: 30 },
                RetentionPoint { elapsed_ratio: 0.2, retention_ratio: 0.25, elapsed_seconds: 60 },
                RetentionPoint { elapsed_ratio: 1.0, retention_ratio: 0.15, elapsed_seconds: 300 },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn recommends_when_hook_and_ctr_are_weak() {
        let performance = VideoPerformance {
            video_id: "v1".into(),
            metrics: sample_metrics(),
            channel_average: ChannelBaseline {
                average_view_duration_seconds: 180.0,
                average_percentage_viewed: 0.80,
                average_ctr: 0.05,
                median_views: 15000,
                sample_size: 10,
            },
            percentiles: PerformancePercentiles {
                view_duration_vs_channel: 0.2,
                retention_vs_channel: 0.2,
                ctr_vs_channel: 0.2,
            },
        };
        let engine = RecommendationEngine::new();
        let recs = engine.analyze(&performance, 300);
        assert!(!recs.is_empty());
        assert!(recs.iter().any(|r| matches!(r.category, RecommendationCategory::Hook)));
        assert!(recs.iter().any(|r| matches!(r.category, RecommendationCategory::Thumbnail)));
    }
}
