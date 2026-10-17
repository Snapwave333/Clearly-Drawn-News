use atlas_core::AtlasResult;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::{MemoryConfig, SharedContextMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Review,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedTask {
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub assigned_to: Option<String>,
    pub status: TaskStatus,
    pub context_notes: Vec<String>,
    pub artifacts: Vec<String>,
    pub promoted_to_graphiti: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentHeartbeat {
    pub agent_id: String,
    pub role: String,
    pub status: String,
    pub current_task_id: Option<String>,
    pub last_seen: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LiveChatMessage {
    pub message_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedContextStatus {
    pub connected: bool,
    pub endpoint: String,
    pub mode: SharedContextMode,
    pub active_tasks: usize,
    pub completed_unpromoted_tasks: usize,
    pub active_agents: usize,
    pub latency_ms: Option<u64>,
}

pub struct SharedContextClient {
    config: MemoryConfig,
    tasks: Arc<RwLock<HashMap<String, SharedTask>>>,
    agents: Arc<RwLock<HashMap<String, AgentHeartbeat>>>,
    messages: Arc<RwLock<Vec<LiveChatMessage>>>,
    http_client: reqwest::Client,
}

impl SharedContextClient {
    pub fn new(config: MemoryConfig) -> Self {
        let initial_tasks = Self::load_tasks_from_disk(&config);
        Self {
            config,
            tasks: Arc::new(RwLock::new(initial_tasks)),
            agents: Arc::new(RwLock::new(HashMap::new())),
            messages: Arc::new(RwLock::new(Vec::new())),
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(1500))
                .build()
                .unwrap_or_default(),
        }
    }

    fn storage_path(config: &MemoryConfig, filename: &str) -> std::path::PathBuf {
        let dir = config
            .local_cache_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("data").join("memory"));
        let _ = std::fs::create_dir_all(&dir);
        dir.join(filename)
    }

    fn load_tasks_from_disk(config: &MemoryConfig) -> HashMap<String, SharedTask> {
        let path = Self::storage_path(config, "shared_context_tasks.json");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(tasks) = serde_json::from_str::<HashMap<String, SharedTask>>(&content) {
                    return tasks;
                }
            }
        }
        HashMap::new()
    }

    fn persist_tasks(&self, tasks: &HashMap<String, SharedTask>) {
        let path = Self::storage_path(&self.config, "shared_context_tasks.json");
        if let Ok(content) = serde_json::to_string_pretty(tasks) {
            let _ = std::fs::write(path, content);
        }
    }

    /// Creates a new task in working memory.
    pub async fn create_task(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
    ) -> AtlasResult<SharedTask> {
        let task_id = format!("task_{}", Uuid::new_v4().simple());
        let now = Utc::now();
        let task = SharedTask {
            task_id: task_id.clone(),
            title: title.to_string(),
            description: description.to_string(),
            assigned_to: assigned_to.map(|s| s.to_string()),
            status: TaskStatus::Pending,
            context_notes: Vec::new(),
            artifacts: Vec::new(),
            promoted_to_graphiti: false,
            created_at: now,
            updated_at: now,
        };

        let mut tasks = self.tasks.write().await;
        tasks.insert(task_id.clone(), task.clone());
        self.persist_tasks(&tasks);
        Ok(task)
    }

    /// Retrieve a task by ID.
    pub async fn get_task(&self, task_id: &str) -> AtlasResult<Option<SharedTask>> {
        let tasks = self.tasks.read().await;
        Ok(tasks.get(task_id).cloned())
    }

    /// List all tasks, optionally filtered by status.
    pub async fn list_tasks(&self, status: Option<TaskStatus>) -> AtlasResult<Vec<SharedTask>> {
        let tasks = self.tasks.read().await;
        let mut list: Vec<SharedTask> = match status {
            Some(s) => tasks.values().filter(|t| t.status == s).cloned().collect(),
            None => tasks.values().cloned().collect(),
        };
        list.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(list)
    }

    /// Update status of an existing task.
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> AtlasResult<Option<SharedTask>> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = status;
            task.updated_at = Utc::now();
            let cloned = task.clone();
            self.persist_tasks(&tasks);
            Ok(Some(cloned))
        } else {
            Ok(None)
        }
    }

    /// Assign a task to an agent.
    pub async fn assign_task(
        &self,
        task_id: &str,
        agent_id: &str,
    ) -> AtlasResult<Option<SharedTask>> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.assigned_to = Some(agent_id.to_string());
            task.status = TaskStatus::InProgress;
            task.updated_at = Utc::now();
            let cloned = task.clone();
            self.persist_tasks(&tasks);
            Ok(Some(cloned))
        } else {
            Ok(None)
        }
    }

    /// Add working context note to a task.
    pub async fn add_context_note(&self, task_id: &str, note: &str) -> AtlasResult<bool> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.context_notes.push(note.to_string());
            task.updated_at = Utc::now();
            self.persist_tasks(&tasks);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Associate an artifact path or hash with a task.
    pub async fn add_artifact(&self, task_id: &str, artifact_id: &str) -> AtlasResult<bool> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.artifacts.push(artifact_id.to_string());
            task.updated_at = Utc::now();
            self.persist_tasks(&tasks);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Mark a task as having been promoted to Graphiti long-term memory.
    pub async fn mark_promoted(&self, task_id: &str) -> AtlasResult<bool> {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.promoted_to_graphiti = true;
            task.updated_at = Utc::now();
            self.persist_tasks(&tasks);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Register/update agent heartbeat.
    pub async fn heartbeat(
        &self,
        agent_id: &str,
        role: &str,
        status: &str,
        current_task_id: Option<&str>,
    ) -> AtlasResult<()> {
        let mut agents = self.agents.write().await;
        let hb = AgentHeartbeat {
            agent_id: agent_id.to_string(),
            role: role.to_string(),
            status: status.to_string(),
            current_task_id: current_task_id.map(|s| s.to_string()),
            last_seen: Utc::now(),
        };
        agents.insert(agent_id.to_string(), hb);
        Ok(())
    }

    /// List active agent heartbeats.
    pub async fn list_agents(&self) -> AtlasResult<Vec<AgentHeartbeat>> {
        let agents = self.agents.read().await;
        Ok(agents.values().cloned().collect())
    }

    /// Send a message across the live inter-agent bus.
    pub async fn post_message(
        &self,
        from_agent: &str,
        to_agent: &str,
        content: &str,
    ) -> AtlasResult<LiveChatMessage> {
        let msg = LiveChatMessage {
            message_id: format!("msg_{}", Uuid::new_v4().simple()),
            from_agent: from_agent.to_string(),
            to_agent: to_agent.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
        };
        let mut msgs = self.messages.write().await;
        msgs.push(msg.clone());
        Ok(msg)
    }

    /// Fetch recent inter-agent messages.
    pub async fn get_messages(&self, limit: usize) -> AtlasResult<Vec<LiveChatMessage>> {
        let msgs = self.messages.read().await;
        let count = msgs.len();
        let skip = count.saturating_sub(limit);
        Ok(msgs.iter().skip(skip).cloned().collect())
    }

    /// Prune completed/failed ephemeral tasks older than `days`.
    pub async fn prune_ephemeral(&self, days: u32) -> AtlasResult<usize> {
        let cutoff = Utc::now() - Duration::days(days as i64);
        let mut tasks = self.tasks.write().await;
        let before_count = tasks.len();
        tasks.retain(|_, t| {
            !(t.promoted_to_graphiti
                && (t.status == TaskStatus::Completed || t.status == TaskStatus::Failed)
                && t.updated_at < cutoff)
        });
        Ok(before_count - tasks.len())
    }

    /// Probe health and connectivity of Shared Context endpoint.
    pub async fn probe_health(&self) -> SharedContextStatus {
        let start = std::time::Instant::now();
        let (connected, latency_ms) = match self.config.shared_context_mode {
            SharedContextMode::Http => {
                match self
                    .http_client
                    .get(format!("{}/health", self.config.shared_context_url))
                    .send()
                    .await
                {
                    Ok(resp) => (
                        resp.status().is_success(),
                        Some(start.elapsed().as_millis() as u64),
                    ),
                    Err(_) => (false, None),
                }
            }
            SharedContextMode::LocalBridge | SharedContextMode::Stdio => {
                // Local in-process bridge is always live and responsive
                (true, Some(start.elapsed().as_millis() as u64))
            }
        };

        let tasks = self.tasks.read().await;
        let agents = self.agents.read().await;
        let active = tasks
            .values()
            .filter(|t| t.status == TaskStatus::InProgress || t.status == TaskStatus::Review)
            .count();
        let unpromoted = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed && !t.promoted_to_graphiti)
            .count();

        SharedContextStatus {
            connected,
            endpoint: self.config.shared_context_url.clone(),
            mode: self.config.shared_context_mode.clone(),
            active_tasks: active,
            completed_unpromoted_tasks: unpromoted,
            active_agents: agents.len(),
            latency_ms,
        }
    }
}
