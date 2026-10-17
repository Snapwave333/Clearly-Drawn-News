use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedContextMode {
    Http,
    Stdio,
    LocalBridge,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// URL or endpoint for Shared Context MCP server.
    pub shared_context_url: String,
    /// Mode for Shared Context connection.
    pub shared_context_mode: SharedContextMode,
    /// URL or endpoint for Graphiti MCP / API server.
    pub graphiti_url: String,
    /// Namespace or group ID for Graphiti knowledge graph.
    pub graphiti_group_id: String,
    /// Optional Graphiti API key.
    pub graphiti_api_key: Option<String>,
    /// Whether tasks marked completed automatically promote distilled memory to Graphiti.
    pub auto_promote_on_completion: bool,
    /// Minimum relevance threshold for graph retrieval (0.0 - 1.0).
    pub min_promotion_relevance: f64,
    /// Days to retain ephemeral task scratchpads in Shared Context before pruning.
    pub prune_ephemeral_after_days: u32,
    /// Path to fallback local storage directory.
    pub local_cache_dir: Option<PathBuf>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            shared_context_url: "http://localhost:8765".to_string(),
            shared_context_mode: SharedContextMode::LocalBridge,
            graphiti_url: "http://localhost:8000".to_string(),
            graphiti_group_id: "atlas_crew".to_string(),
            graphiti_api_key: None,
            auto_promote_on_completion: true,
            min_promotion_relevance: 0.65,
            prune_ephemeral_after_days: 7,
            local_cache_dir: None,
        }
    }
}

impl MemoryConfig {
    pub fn load_from_dir(base_dir: &Path) -> Self {
        let config_file = base_dir
            .join(".opencode")
            .join("config")
            .join("memory.json");
        if config_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_file) {
                if let Ok(parsed) = serde_json::from_str::<MemoryConfig>(&content) {
                    return parsed;
                }
            }
        }
        Self::default()
    }

    pub fn save_to_dir(&self, base_dir: &Path) -> std::io::Result<()> {
        let config_dir = base_dir.join(".opencode").join("config");
        std::fs::create_dir_all(&config_dir)?;
        let config_file = config_dir.join("memory.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(config_file, content)?;
        Ok(())
    }
}
