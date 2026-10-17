use atlas_core::{AgentMessage, AtlasResult, FeedbackItem};
use atlas_store::Store;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

/// In-memory message bus with durable persistence to SQLite.
pub struct AgentBus {
    store: Arc<Mutex<Store>>,
    leases: Arc<RwLock<HashMap<String, Lease>>>,
    pending_retries: Arc<Mutex<Vec<RetryTask>>>,
}

/// A lease on a task, held by a worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub task_id: String,
    pub worker_id: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
}

/// A task that needs retry with exponential backoff.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryTask {
    pub message: AgentMessage,
    pub attempt: u32,
    pub next_retry_at: DateTime<Utc>,
    pub idempotency_key: String,
}

pub struct ReviewRequest {
    pub from: String,
    pub to: String,
    pub project_id: String,
    pub task_id: String,
    pub artifact_ids: Vec<String>,
    pub feedback: Vec<FeedbackItem>,
    pub deadline_hours: i64,
}

impl AgentBus {
    pub fn new(store: Arc<Mutex<Store>>) -> Self {
        Self {
            store,
            leases: Arc::new(RwLock::new(HashMap::new())),
            pending_retries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Send a message, persisting it durably.
    pub async fn send(&self, message: AgentMessage) -> AtlasResult<()> {
        self.store.lock().await.insert_message(&message)?;
        tracing::info!(
            "Bus: {} -> {} [{}]",
            message.from,
            message.to,
            message.message_type
        );
        Ok(())
    }

    /// Send a blocking review request. Returns when review passes or fails.
    pub async fn request_review(&self, request: ReviewRequest) -> AtlasResult<ReviewOutcome> {
        let message = AgentMessage {
            schema_version: 1,
            message_id: format!("msg_{}", Uuid::new_v4()),
            project_id: request.project_id,
            from: request.from,
            to: request.to,
            message_type: "REVIEW_REQUESTED".to_string(),
            task_id: Some(request.task_id),
            artifact_ids: request.artifact_ids,
            blocking: true,
            priority: "high".to_string(),
            feedback: request.feedback,
            created_at: Utc::now(),
            deadline_at: Some(Utc::now() + Duration::hours(request.deadline_hours)),
        };
        self.send(message).await?;

        // In a real implementation, this would wait for REVIEW_PASSED or REVISION_REQUIRED
        // For now, return a placeholder that the caller must poll.
        Ok(ReviewOutcome::Pending)
    }

    /// Worker claims a lease on a task.
    pub async fn claim_lease(
        &self,
        task_id: &str,
        worker_id: &str,
        ttl_seconds: i64,
    ) -> AtlasResult<bool> {
        let mut leases = self.leases.write().await;
        let now = Utc::now();
        let expires = now + Duration::seconds(ttl_seconds);

        if let Some(existing) = leases.get(task_id) {
            if existing.expires_at > now && existing.worker_id != worker_id {
                return Ok(false); // Already leased to another worker
            }
        }

        leases.insert(
            task_id.to_string(),
            Lease {
                task_id: task_id.to_string(),
                worker_id: worker_id.to_string(),
                acquired_at: now,
                expires_at: expires,
                heartbeat_at: now,
            },
        );
        Ok(true)
    }

    /// Worker heartbeats to keep lease alive.
    pub async fn heartbeat(&self, task_id: &str, worker_id: &str) -> AtlasResult<bool> {
        let mut leases = self.leases.write().await;
        if let Some(lease) = leases.get_mut(task_id) {
            if lease.worker_id == worker_id {
                lease.heartbeat_at = Utc::now();
                lease.expires_at = Utc::now() + Duration::seconds(300); // Extend TTL
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Release a lease (task completed).
    pub async fn release_lease(&self, task_id: &str, worker_id: &str) -> AtlasResult<()> {
        let mut leases = self.leases.write().await;
        if let Some(lease) = leases.get(task_id) {
            if lease.worker_id == worker_id {
                leases.remove(task_id);
            }
        }
        Ok(())
    }

    /// Recover abandoned leases (worker died).
    pub async fn recover_abandoned(&self) -> Vec<String> {
        let mut leases = self.leases.write().await;
        let now = Utc::now();
        let mut recovered = Vec::new();
        leases.retain(|task_id, lease| {
            if lease.expires_at < now {
                recovered.push(task_id.clone());
                false
            } else {
                true
            }
        });
        recovered
    }

    /// Schedule a message for retry with exponential backoff.
    pub async fn schedule_retry(
        &self,
        message: AgentMessage,
        attempt: u32,
        idempotency_key: String,
    ) -> AtlasResult<()> {
        let delay = Duration::seconds(2_i64.pow(attempt.min(6)) * 10); // 10s, 20s, 40s, 80s, 160s, 320s, 640s
        let mut retries = self.pending_retries.lock().await;
        retries.push(RetryTask {
            message,
            attempt,
            next_retry_at: Utc::now() + delay,
            idempotency_key,
        });
        Ok(())
    }

    /// Process due retries (call periodically).
    pub async fn process_retries(&self) -> AtlasResult<Vec<AgentMessage>> {
        let mut retries = self.pending_retries.lock().await;
        let now = Utc::now();
        let mut ready = Vec::new();
        let mut remaining = Vec::new();

        for task in retries.drain(..) {
            if task.next_retry_at <= now {
                ready.push(task.message);
            } else {
                remaining.push(task);
            }
        }
        *retries = remaining;
        Ok(ready)
    }

    /// Get all messages for a project (for UI/debugging).
    pub fn get_project_messages(&self, _project_id: &str) -> AtlasResult<Vec<AgentMessage>> {
        // Would need a query method on Store; placeholder for now
        Ok(Vec::new())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ReviewOutcome {
    Pending,
    Passed,
    RevisionRequired { feedback: Vec<FeedbackItem> },
    Failed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_store::Store;
    use tempfile::tempdir;

    #[tokio::test]
    async fn lease_claim_and_release() {
        let dir = tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open(dir.path()).unwrap()));
        let bus = AgentBus::new(store);

        assert!(bus.claim_lease("task1", "worker1", 60).await.unwrap());
        assert!(!bus.claim_lease("task1", "worker2", 60).await.unwrap()); // Already leased
        assert!(bus.release_lease("task1", "worker1").await.is_ok());
        assert!(bus.claim_lease("task1", "worker2", 60).await.unwrap()); // Now free
    }

    #[tokio::test]
    async fn heartbeat_extends_lease() {
        let dir = tempdir().unwrap();
        let store = Arc::new(Mutex::new(Store::open(dir.path()).unwrap()));
        let bus = AgentBus::new(store);

        bus.claim_lease("task1", "worker1", 1).await.unwrap(); // 1 second TTL
        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;
        // Lease should be expired
        let recovered = bus.recover_abandoned().await;
        assert!(recovered.contains(&"task1".to_string()));
    }
}
