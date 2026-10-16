use atlas_core::{AtlasError, AtlasResult};
use atlas_store::Store;
use rusqlite::OptionalExtension;

use crate::models::VideoMetrics;

/// Persists analytics metrics in the ATLAS SQLite store.
pub struct AnalyticsStore<'a> {
    store: &'a Store,
}

impl<'a> AnalyticsStore<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn ensure_schema(&self) -> AtlasResult<()> {
        self.store
            .conn()
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS youtube_metrics (
                    video_id TEXT PRIMARY KEY REFERENCES projects(video_id),
                    youtube_video_id TEXT NOT NULL,
                    retrieved_at TEXT NOT NULL,
                    metrics_json TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS retention_curves (
                    video_id TEXT NOT NULL REFERENCES projects(video_id),
                    youtube_video_id TEXT NOT NULL,
                    elapsed_ratio REAL NOT NULL,
                    retention_ratio REAL NOT NULL,
                    elapsed_seconds INTEGER NOT NULL,
                    retrieved_at TEXT NOT NULL,
                    PRIMARY KEY (video_id, elapsed_ratio)
                );

                CREATE INDEX IF NOT EXISTS idx_retention_video ON retention_curves(video_id);
                "#,
            )
            .map_err(|e| AtlasError::Store(format!("analytics schema failed: {e}")))?;
        Ok(())
    }

    pub fn save_metrics(&self, metrics: &VideoMetrics) -> AtlasResult<()> {
        self.ensure_schema()?;
        let json = serde_json::to_string(metrics)?;
        self.store.conn().execute(
            "INSERT INTO youtube_metrics (video_id, youtube_video_id, retrieved_at, metrics_json)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(video_id) DO UPDATE SET
                 youtube_video_id = excluded.youtube_video_id,
                 retrieved_at = excluded.retrieved_at,
                 metrics_json = excluded.metrics_json",
            rusqlite::params![metrics.video_id, metrics.youtube_video_id, metrics.retrieved_at.to_rfc3339(), json],
        ).map_err(|e| AtlasError::Store(format!("failed to save metrics: {e}")))?;

        // Save retention curve separately for efficient querying.
        for point in &metrics.retention_curve {
            self.store.conn().execute(
                "INSERT INTO retention_curves (video_id, youtube_video_id, elapsed_ratio, retention_ratio, elapsed_seconds, retrieved_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(video_id, elapsed_ratio) DO UPDATE SET
                     retention_ratio = excluded.retention_ratio,
                     elapsed_seconds = excluded.elapsed_seconds,
                     retrieved_at = excluded.retrieved_at",
                rusqlite::params![
                    metrics.video_id,
                    metrics.youtube_video_id,
                    point.elapsed_ratio,
                    point.retention_ratio,
                    point.elapsed_seconds,
                    metrics.retrieved_at.to_rfc3339()
                ],
            ).map_err(|e| AtlasError::Store(format!("failed to save retention point: {e}")))?;
        }
        Ok(())
    }

    pub fn load_metrics(&self, video_id: &str) -> AtlasResult<Option<VideoMetrics>> {
        self.ensure_schema()?;
        let mut stmt = self.store.conn().prepare(
            "SELECT metrics_json FROM youtube_metrics WHERE video_id = ?1"
        ).map_err(|e| AtlasError::Store(e.to_string()))?;
        let json: Option<String> = stmt
            .query_row([video_id], |row| row.get(0))
            .optional()
            .map_err(|e| AtlasError::Store(e.to_string()))?;
        match json {
            Some(j) => serde_json::from_str(&j).map_err(|e| AtlasError::Store(e.to_string())),
            None => Ok(None),
        }
    }

    pub fn list_metrics(&self, limit: usize) -> AtlasResult<Vec<VideoMetrics>> {
        self.ensure_schema()?;
        let mut stmt = self.store.conn().prepare(
            "SELECT metrics_json FROM youtube_metrics ORDER BY retrieved_at DESC LIMIT ?1"
        ).map_err(|e| AtlasError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([limit], |row| row.get::<_, String>(0))
            .map_err(|e| AtlasError::Store(e.to_string()))?;
        let mut metrics = Vec::new();
        for row in rows {
            let json: String = row.map_err(|e| AtlasError::Store(e.to_string()))?;
            metrics.push(serde_json::from_str(&json).map_err(|e| AtlasError::Store(e.to_string()))?);
        }
        Ok(metrics)
    }
}
