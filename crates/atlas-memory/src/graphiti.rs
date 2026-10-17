use atlas_core::AtlasResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::MemoryConfig;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Episode {
    pub episode_id: String,
    pub name: String,
    pub body: String,
    pub source_description: String,
    pub group_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityNode {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub valid_from: DateTime<Utc>,
    pub invalid_at: Option<DateTime<Utc>>,
    pub weight: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphSearchResult {
    pub episode_id: String,
    pub title: String,
    pub score: f64,
    pub summary: String,
    pub matched_entities: Vec<String>,
    pub matched_relations: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphitiStatus {
    pub connected: bool,
    pub endpoint: String,
    pub group_id: String,
    pub total_episodes: usize,
    pub total_entities: usize,
    pub total_edges: usize,
    pub latency_ms: Option<u64>,
}

pub struct GraphitiClient {
    config: MemoryConfig,
    episodes: Arc<RwLock<HashMap<String, Episode>>>,
    nodes: Arc<RwLock<HashMap<String, EntityNode>>>,
    edges: Arc<RwLock<Vec<EntityEdge>>>,
    http_client: reqwest::Client,
}

impl GraphitiClient {
    pub fn new(config: MemoryConfig) -> Self {
        let initial_episodes = Self::load_episodes_from_disk(&config);
        Self {
            config,
            episodes: Arc::new(RwLock::new(initial_episodes)),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            edges: Arc::new(RwLock::new(Vec::new())),
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(2000))
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

    fn load_episodes_from_disk(config: &MemoryConfig) -> HashMap<String, Episode> {
        let path = Self::storage_path(config, "graphiti_episodes.json");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(eps) = serde_json::from_str::<HashMap<String, Episode>>(&content) {
                    return eps;
                }
            }
        }
        HashMap::new()
    }

    fn persist_episodes(&self, episodes: &HashMap<String, Episode>) {
        let path = Self::storage_path(&self.config, "graphiti_episodes.json");
        if let Ok(content) = serde_json::to_string_pretty(episodes) {
            let _ = std::fs::write(path, content);
        }
    }

    /// Add a new episode / distilled memory to Graphiti.
    pub async fn add_episode(
        &self,
        name: &str,
        body: &str,
        source_description: &str,
    ) -> AtlasResult<Episode> {
        let episode_id = format!("ep_{}", Uuid::new_v4().simple());
        let episode = Episode {
            episode_id: episode_id.clone(),
            name: name.to_string(),
            body: body.to_string(),
            source_description: source_description.to_string(),
            group_id: self.config.graphiti_group_id.clone(),
            created_at: Utc::now(),
        };

        // If remote Graphiti endpoint is active, post to it
        if !self.config.graphiti_url.is_empty() {
            let payload = serde_json::json!({
                "group_id": self.config.graphiti_group_id,
                "name": name,
                "episode_body": body,
                "source": "message",
                "source_description": source_description,
            });

            let mut req = self
                .http_client
                .post(format!("{}/episodes", self.config.graphiti_url))
                .json(&payload);

            if let Some(ref key) = self.config.graphiti_api_key {
                req = req.header("Authorization", format!("Bearer {}", key));
            }

            // Fire and let failover to local graph store
            let _ = req.send().await;
        }

        // Store locally in-memory graph cache
        let mut episodes = self.episodes.write().await;
        episodes.insert(episode_id.clone(), episode.clone());
        self.persist_episodes(&episodes);

        // Extract and index basic entities and edges locally
        self.extract_and_index_entities(&episode).await;

        Ok(episode)
    }

    /// Search the knowledge graph for relevant context.
    pub async fn search(&self, query: &str, limit: usize) -> AtlasResult<Vec<GraphSearchResult>> {
        let q_lower = query.to_lowercase();
        let query_terms: Vec<&str> = q_lower.split_whitespace().collect();

        let episodes = self.episodes.read().await;
        let mut results = Vec::new();

        for ep in episodes.values() {
            let body_lower = ep.body.to_lowercase();
            let name_lower = ep.name.to_lowercase();

            let mut matched_terms = 0;
            for term in &query_terms {
                if body_lower.contains(term) || name_lower.contains(term) {
                    matched_terms += 1;
                }
            }

            if matched_terms > 0 || query.is_empty() {
                let score = if query_terms.is_empty() {
                    1.0
                } else {
                    matched_terms as f64 / query_terms.len() as f64
                };

                if score >= self.config.min_promotion_relevance || query.is_empty() {
                    let summary = if ep.body.len() > 300 {
                        format!("{}...", &ep.body[..300])
                    } else {
                        ep.body.clone()
                    };

                    results.push(GraphSearchResult {
                        episode_id: ep.episode_id.clone(),
                        title: ep.name.clone(),
                        score,
                        summary,
                        matched_entities: vec![ep.name.clone()],
                        matched_relations: vec![ep.source_description.clone()],
                        created_at: ep.created_at,
                    });
                }
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// Invalidate an edge when a previous decision or fact is superseded.
    pub async fn invalidate_edge(
        &self,
        source: &str,
        target: &str,
        relation: &str,
    ) -> AtlasResult<bool> {
        let mut edges = self.edges.write().await;
        let mut found = false;
        let now = Utc::now();
        for edge in edges.iter_mut() {
            if edge.source == source && edge.target == target && edge.relation == relation {
                edge.invalid_at = Some(now);
                found = true;
            }
        }
        Ok(found)
    }

    /// Probe health of Graphiti service.
    pub async fn probe_health(&self) -> GraphitiStatus {
        let start = std::time::Instant::now();
        let mut req = self
            .http_client
            .get(format!("{}/healthz", self.config.graphiti_url));
        if let Some(ref key) = self.config.graphiti_api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let (connected, latency_ms) = match req.send().await {
            Ok(resp) => (
                resp.status().is_success(),
                Some(start.elapsed().as_millis() as u64),
            ),
            Err(_) => (false, None),
        };

        let eps = self.episodes.read().await;
        let nds = self.nodes.read().await;
        let edg = self.edges.read().await;

        GraphitiStatus {
            connected,
            endpoint: self.config.graphiti_url.clone(),
            group_id: self.config.graphiti_group_id.clone(),
            total_episodes: eps.len(),
            total_entities: nds.len(),
            total_edges: edg.len(),
            latency_ms,
        }
    }

    async fn extract_and_index_entities(&self, episode: &Episode) {
        let mut nodes = self.nodes.write().await;
        let mut edges = self.edges.write().await;

        let node_id = format!("node_{}", Uuid::new_v4().simple());
        nodes.insert(
            episode.name.clone(),
            EntityNode {
                id: node_id.clone(),
                name: episode.name.clone(),
                entity_type: "TopicEpisode".to_string(),
                summary: episode.source_description.clone(),
            },
        );

        edges.push(EntityEdge {
            source: episode.group_id.clone(),
            target: episode.name.clone(),
            relation: "CONTAINS_EPISODE".to_string(),
            valid_from: episode.created_at,
            invalid_at: None,
            weight: 1.0,
        });
    }
}
