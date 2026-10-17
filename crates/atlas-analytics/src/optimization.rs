use atlas_core::{AgentMessage, FeedbackItem};
use atlas_store::Store;
use chrono::Utc;
use uuid::Uuid;

use crate::{AnalyticsStore, RecommendationEngine, compare_to_baseline, compute_channel_baseline};

/// Drives a continuous content-improvement loop from analytics data.
pub struct OptimizationLoop;

impl OptimizationLoop {
    pub fn new() -> Self {
        Self
    }

    /// Runs analytics through the recommendation engine and writes actionable
    /// agent messages to the bus for the executive producer and creative director.
    pub fn generate_revision_tasks(
        &self,
        store: &Store,
        video_id: &str,
        video_duration_seconds: u64,
    ) -> anyhow::Result<Vec<String>> {
        let analytics = AnalyticsStore::new(store);
        let metrics = match analytics.load_metrics(video_id)? {
            Some(m) => m,
            None => return Ok(Vec::new()),
        };

        let all = analytics.list_metrics(100)?;
        let baseline = compute_channel_baseline(&all);
        let performance = compare_to_baseline(&metrics, &baseline);

        let engine = RecommendationEngine::new();
        let recs = engine.rank(engine.analyze(&performance, video_duration_seconds));

        let mut message_ids = Vec::new();
        for rec in &recs {
            let msg = AgentMessage {
                schema_version: 1,
                message_id: format!("msg_{}", Uuid::new_v4()),
                project_id: video_id.to_string(),
                from: "publisher_analyst".to_string(),
                to: match rec.category {
                    crate::RecommendationCategory::Hook
                    | crate::RecommendationCategory::Pacing
                    | crate::RecommendationCategory::Structure
                    | crate::RecommendationCategory::Topic => "executive_producer".to_string(),
                    _ => "creative_director".to_string(),
                },
                message_type: "REVISION_REQUIRED".to_string(),
                task_id: Some(format!("analytics_{}", rec.recommendation_id)),
                artifact_ids: vec![format!("analytics_{}", video_id)],
                blocking: matches!(rec.priority, crate::RecommendationPriority::Critical),
                priority: format!("{:?}", rec.priority).to_lowercase(),
                feedback: vec![FeedbackItem {
                    claim_id: None,
                    issue: rec.issue.clone(),
                    required_change: format!(
                        "{} (expected impact: {} {})",
                        rec.suggested_action,
                        rec.expected_impact.metric,
                        rec.expected_impact.direction
                    ),
                }],
                created_at: Utc::now(),
                deadline_at: None,
            };
            store.insert_message(&msg)?;
            message_ids.push(msg.message_id);
        }
        Ok(message_ids)
    }
}

impl Default for OptimizationLoop {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{Project, PublishPlan, Topic};
    use atlas_store::Store;
    use tempfile::tempdir;

    #[test]
    fn generates_revision_tasks_for_underperforming_video() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .insert_project(&Project {
                video_id: "vid_test".into(),
                topic: Topic::default(),
                state: atlas_core::ProjectState::Published,
                channel_profile: "primary".into(),
                visual_thesis: None,
                script_artifact_id: None,
                research_artifact_id: None,
                storyboard_artifact_id: None,
                publish: PublishPlan::default(),
            })
            .unwrap();
        let _ = crate::save_sample_metrics(&store, "vid_test", "yt_test").unwrap();

        let loop_ = OptimizationLoop::new();
        let ids = loop_
            .generate_revision_tasks(&store, "vid_test", 500)
            .unwrap();
        assert!(!ids.is_empty());
    }
}
