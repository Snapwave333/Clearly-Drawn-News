use atlas_core::{AgentMessage, AtlasConfig, AtlasError, AtlasResult, Project};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct Store {
    conn: Connection,
    data_dir: PathBuf,
}

impl Store {
    pub fn open(data_dir: impl AsRef<Path>) -> AtlasResult<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)?;
        let db_path = data_dir.join("atlas.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| AtlasError::Store(format!("failed to open {db_path:?}: {e}")))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| AtlasError::Store(format!("failed to set WAL: {e}")))?;

        let store = Self { conn, data_dir };
        store.migrate()?;
        info!("SQLite store opened at {:?}", db_path);
        Ok(store)
    }

    fn migrate(&self) -> AtlasResult<()> {
        self.conn
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS config (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS projects (
                    video_id TEXT PRIMARY KEY,
                    topic_json TEXT NOT NULL,
                    state TEXT NOT NULL,
                    channel_profile TEXT NOT NULL,
                    visual_thesis TEXT,
                    script_artifact_id TEXT,
                    research_artifact_id TEXT,
                    storyboard_artifact_id TEXT,
                    publish_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS artifacts (
                    artifact_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(video_id),
                    kind TEXT NOT NULL,
                    sha256 TEXT NOT NULL,
                    path TEXT NOT NULL,
                    metadata_json TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS messages (
                    message_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(video_id),
                    from_agent TEXT NOT NULL,
                    to_agent TEXT NOT NULL,
                    message_type TEXT NOT NULL,
                    task_id TEXT,
                    artifact_ids TEXT,
                    blocking INTEGER NOT NULL,
                    priority TEXT NOT NULL,
                    feedback_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    deadline_at TEXT
                );

                CREATE INDEX IF NOT EXISTS idx_projects_state ON projects(state);
                CREATE INDEX IF NOT EXISTS idx_messages_project ON messages(project_id);
                CREATE INDEX IF NOT EXISTS idx_artifacts_project ON artifacts(project_id);
                "#,
            )
            .map_err(|e| AtlasError::Store(format!("migration failed: {e}")))?;
        Ok(())
    }

    pub fn load_config(&self) -> AtlasResult<AtlasConfig> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM config WHERE key = 'config'")
            .map_err(|e| AtlasError::Store(e.to_string()))?;
        let value: Option<String> = stmt
            .query_row([], |row| row.get(0))
            .optional()
            .map_err(|e| AtlasError::Store(e.to_string()))?;
        match value {
            Some(json) => serde_json::from_str(&json).map_err(|e| AtlasError::Store(e.to_string())),
            None => Ok(AtlasConfig::default()),
        }
    }

    pub fn save_config(&self, config: &AtlasConfig) -> AtlasResult<()> {
        let json = serde_json::to_string(config)?;
        self.conn
            .execute(
                "INSERT INTO config (key, value) VALUES ('config', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![json],
            )
            .map_err(|e| AtlasError::Store(format!("failed to save config: {e}")))?;
        Ok(())
    }

    pub fn insert_project(&self, project: &Project) -> AtlasResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO projects (
                    video_id, topic_json, state, channel_profile, visual_thesis,
                    script_artifact_id, research_artifact_id, storyboard_artifact_id,
                    publish_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(video_id) DO UPDATE SET
                    topic_json = excluded.topic_json,
                    state = excluded.state,
                    channel_profile = excluded.channel_profile,
                    visual_thesis = excluded.visual_thesis,
                    script_artifact_id = excluded.script_artifact_id,
                    research_artifact_id = excluded.research_artifact_id,
                    storyboard_artifact_id = excluded.storyboard_artifact_id,
                    publish_json = excluded.publish_json,
                    updated_at = excluded.updated_at",
                params![
                    project.video_id,
                    serde_json::to_string(&project.topic)?,
                    serde_json::to_string(&project.state)?,
                    project.channel_profile,
                    project.visual_thesis,
                    project.script_artifact_id,
                    project.research_artifact_id,
                    project.storyboard_artifact_id,
                    serde_json::to_string(&project.publish)?,
                    now.clone(),
                    now,
                ],
            )
            .map_err(|e| AtlasError::Store(format!("failed to insert project: {e}")))?;
        Ok(())
    }

    pub fn get_project(&self, video_id: &str) -> AtlasResult<Option<Project>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT video_id, topic_json, state, channel_profile, visual_thesis,
                        script_artifact_id, research_artifact_id, storyboard_artifact_id,
                        publish_json
                 FROM projects WHERE video_id = ?1",
            )
            .map_err(|e| AtlasError::Store(e.to_string()))?;
        let row = stmt
            .query_row([video_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .optional()
            .map_err(|e| AtlasError::Store(e.to_string()))?;

        match row {
            Some((video_id, topic_json, state_json, channel_profile, visual_thesis, script, research, storyboard, publish_json)) => {
                let topic = serde_json::from_str(&topic_json)?;
                let state = serde_json::from_str(&state_json)?;
                let publish = serde_json::from_str(&publish_json)?;
                Ok(Some(Project {
                    video_id,
                    topic,
                    state,
                    channel_profile,
                    visual_thesis,
                    script_artifact_id: script,
                    research_artifact_id: research,
                    storyboard_artifact_id: storyboard,
                    publish,
                }))
            }
            None => Ok(None),
        }
    }

    pub fn list_projects(&self, limit: usize) -> AtlasResult<Vec<Project>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT video_id, topic_json, state, channel_profile, visual_thesis,
                        script_artifact_id, research_artifact_id, storyboard_artifact_id,
                        publish_json
                 FROM projects ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(|e| AtlasError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| AtlasError::Store(e.to_string()))?;

        let mut projects = Vec::new();
        for row in rows {
            let (video_id, topic_json, state_json, channel_profile, visual_thesis, script, research, storyboard, publish_json) =
                row.map_err(|e| AtlasError::Store(e.to_string()))?;
            projects.push(Project {
                video_id,
                topic: serde_json::from_str(&topic_json)?,
                state: serde_json::from_str(&state_json)?,
                channel_profile,
                visual_thesis,
                script_artifact_id: script,
                research_artifact_id: research,
                storyboard_artifact_id: storyboard,
                publish: serde_json::from_str(&publish_json)?,
            });
        }
        Ok(projects)
    }

    pub fn insert_message(&self, message: &AgentMessage) -> AtlasResult<()> {
        self.conn
            .execute(
                "INSERT INTO messages (
                    message_id, project_id, from_agent, to_agent, message_type, task_id,
                    artifact_ids, blocking, priority, feedback_json, created_at, deadline_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ON CONFLICT(message_id) DO NOTHING",
                params![
                    message.message_id,
                    message.project_id,
                    message.from,
                    message.to,
                    message.message_type,
                    message.task_id,
                    message.artifact_ids.join(","),
                    message.blocking as i32,
                    message.priority,
                    serde_json::to_string(&message.feedback)?,
                    message.created_at.to_rfc3339(),
                    message.deadline_at.map(|d| d.to_rfc3339()),
                ],
            )
            .map_err(|e| AtlasError::Store(format!("failed to insert message: {e}")))?;
        Ok(())
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
