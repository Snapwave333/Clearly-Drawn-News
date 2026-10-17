pub mod config;
pub mod graphiti;
pub mod promoter;
pub mod shared_context;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;

pub use config::{MemoryConfig, SharedContextMode};
pub use graphiti::{Episode, GraphSearchResult, GraphitiClient, GraphitiStatus};
pub use promoter::{MemoryPromoter, PromotionReport};
pub use shared_context::{
    AgentHeartbeat, LiveChatMessage, SharedContextClient, SharedContextStatus, SharedTask,
    TaskStatus,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MemorySystemStatus {
    pub shared_context: SharedContextStatus,
    pub graphiti: GraphitiStatus,
    pub auto_promote_enabled: bool,
    pub prune_retention_days: u32,
}

pub struct MemoryEngine {
    config: MemoryConfig,
    shared_context: Arc<SharedContextClient>,
    graphiti: Arc<GraphitiClient>,
    promoter: Arc<MemoryPromoter>,
}

impl MemoryEngine {
    pub fn new(config: MemoryConfig) -> Self {
        let sc = Arc::new(SharedContextClient::new(config.clone()));
        let gr = Arc::new(GraphitiClient::new(config.clone()));
        let pr = Arc::new(MemoryPromoter::new(Arc::clone(&sc), Arc::clone(&gr)));

        Self {
            config,
            shared_context: sc,
            graphiti: gr,
            promoter: pr,
        }
    }

    pub fn from_workspace(base_dir: &Path) -> Self {
        let cfg = MemoryConfig::load_from_dir(base_dir);
        Self::new(cfg)
    }

    pub fn config(&self) -> &MemoryConfig {
        &self.config
    }

    pub fn shared_context(&self) -> &Arc<SharedContextClient> {
        &self.shared_context
    }

    pub fn graphiti(&self) -> &Arc<GraphitiClient> {
        &self.graphiti
    }

    pub fn promoter(&self) -> &Arc<MemoryPromoter> {
        &self.promoter
    }

    /// Combined health check and metrics for both memory layers.
    pub async fn status(&self) -> MemorySystemStatus {
        let sc_status = self.shared_context.probe_health().await;
        let gr_status = self.graphiti.probe_health().await;

        MemorySystemStatus {
            shared_context: sc_status,
            graphiti: gr_status,
            auto_promote_enabled: self.config.auto_promote_on_completion,
            prune_retention_days: self.config.prune_ephemeral_after_days,
        }
    }
}
