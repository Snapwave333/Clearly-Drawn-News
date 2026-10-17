use atlas_core::{AtlasError, AtlasResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::graphiti::{GraphSearchResult, GraphitiClient};
use crate::shared_context::{SharedContextClient, SharedTask, TaskStatus};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromotionReport {
    pub task_id: String,
    pub task_title: String,
    pub episode_id: String,
    pub distilled_summary: String,
    pub promoted_at: DateTime<Utc>,
    pub artifact_count: usize,
    pub note_count: usize,
}

pub struct MemoryPromoter {
    shared_context: Arc<SharedContextClient>,
    graphiti: Arc<GraphitiClient>,
}

impl MemoryPromoter {
    pub fn new(shared_context: Arc<SharedContextClient>, graphiti: Arc<GraphitiClient>) -> Self {
        Self {
            shared_context,
            graphiti,
        }
    }

    /// Promote a specific completed task into Graphiti long-term knowledge graph.
    pub async fn promote_task(
        &self,
        task_id: &str,
        custom_notes: Option<&str>,
    ) -> AtlasResult<PromotionReport> {
        let task = self
            .shared_context
            .get_task(task_id)
            .await?
            .ok_or_else(|| {
                AtlasError::Memory(format!("Task not found in shared context: {task_id}"))
            })?;

        if task.status != TaskStatus::Completed && task.status != TaskStatus::Review {
            return Err(AtlasError::Memory(format!(
                "Task {} cannot be promoted; current status is {:?}",
                task_id, task.status
            )));
        }

        // Distill knowledge payload
        let distilled = self.distill_task_knowledge(&task, custom_notes);
        let source_desc = format!(
            "Promoted from Task {} (assigned to: {})",
            task.task_id,
            task.assigned_to.as_deref().unwrap_or("unassigned")
        );

        let episode = self
            .graphiti
            .add_episode(&task.title, &distilled, &source_desc)
            .await?;

        // Mark promoted in shared context
        self.shared_context.mark_promoted(task_id).await?;

        Ok(PromotionReport {
            task_id: task.task_id,
            task_title: task.title,
            episode_id: episode.episode_id,
            distilled_summary: distilled,
            promoted_at: Utc::now(),
            artifact_count: task.artifacts.len(),
            note_count: task.context_notes.len(),
        })
    }

    /// Scan and promote all completed tasks that haven't been promoted yet.
    pub async fn auto_promote_completed(&self) -> AtlasResult<Vec<PromotionReport>> {
        let completed_tasks = self
            .shared_context
            .list_tasks(Some(TaskStatus::Completed))
            .await?;

        let mut reports = Vec::new();
        for task in completed_tasks {
            if !task.promoted_to_graphiti {
                match self.promote_task(&task.task_id, None).await {
                    Ok(rep) => reports.push(rep),
                    Err(e) => {
                        tracing::warn!("Failed auto-promoting task {}: {}", task.task_id, e);
                    }
                }
            }
        }

        Ok(reports)
    }

    /// Preloads and bootstraps historical knowledge from Graphiti into a new Shared Context task.
    pub async fn bootstrap_topic_context(
        &self,
        topic: &str,
        assigned_to: Option<&str>,
    ) -> AtlasResult<(SharedTask, Vec<GraphSearchResult>)> {
        // Query Graphiti for related historical context
        let search_results = self.graphiti.search(topic, 5).await?;

        let mut priming_notes = Vec::new();
        priming_notes.push(format!("### Graphiti Historical Context for '{}'", topic));

        if search_results.is_empty() {
            priming_notes.push(
                "No prior historical knowledge graph entries found. Starting fresh topic."
                    .to_string(),
            );
        } else {
            for res in &search_results {
                priming_notes.push(format!(
                    "• [Score: {:.2}] **{}**: {}",
                    res.score, res.title, res.summary
                ));
            }
        }

        let mut task = self
            .shared_context
            .create_task(
                &format!("Pipeline: {}", topic),
                &format!("Production and coordination for topic: {}", topic),
                assigned_to,
            )
            .await?;

        for note in &priming_notes {
            self.shared_context
                .add_context_note(&task.task_id, note)
                .await?;
            task.context_notes.push(note.clone());
        }

        Ok((task, search_results))
    }

    fn distill_task_knowledge(&self, task: &SharedTask, custom_notes: Option<&str>) -> String {
        let mut out = String::new();
        out.push_str(&format!("## Distilled Knowledge: {}\n\n", task.title));
        out.push_str(&format!("- **Task ID:** {}\n", task.task_id));
        out.push_str(&format!("- **Description:** {}\n", task.description));
        if let Some(agent) = &task.assigned_to {
            out.push_str(&format!("- **Lead Agent:** {}\n", agent));
        }

        if !task.artifacts.is_empty() {
            out.push_str("\n### Verified Artifacts\n");
            for art in &task.artifacts {
                out.push_str(&format!("- `{}`\n", art));
            }
        }

        if !task.context_notes.is_empty() {
            out.push_str("\n### Key Decisions & Execution Notes\n");
            for note in &task.context_notes {
                out.push_str(&format!("- {}\n", note));
            }
        }

        if let Some(notes) = custom_notes {
            out.push_str("\n### Post-Mortem & Verified Takeaways\n");
            out.push_str(notes);
            out.push('\n');
        }

        out
    }
}
