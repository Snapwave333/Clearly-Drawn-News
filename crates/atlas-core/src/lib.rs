use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

/// Errors that can occur across the Clearly Drawn Studios ATLAS engine.
#[derive(Error, Debug)]
pub enum AtlasError {
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("tool probe failed for {tool}: {message}")]
    ToolProbeFailed { tool: String, message: String },
    #[error("configuration error: {0}")]
    Config(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("OpenCode integration error: {0}")]
    OpenCode(String),
    #[error("render error: {0}")]
    Render(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("memory error: {0}")]
    Memory(String),
    #[error("unknown error: {0}")]
    Other(String),
}

impl AtlasError {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }
}

pub type AtlasResult<T> = Result<T, AtlasError>;

/// A tool or external engine that ATLAS can depend on.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    OpenCode,
    Blender,
    UnrealEditor,
    VectorCraft,
    PhotoCraft,
    EffectCraft,
    Kokoro,
    Ffmpeg,
    Ffprobe,
}

impl fmt::Display for ToolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ToolKind::OpenCode => "OpenCode",
            ToolKind::Blender => "Blender",
            ToolKind::UnrealEditor => "Unreal Editor",
            ToolKind::VectorCraft => "VectorCraft",
            ToolKind::PhotoCraft => "PhotoCraft",
            ToolKind::EffectCraft => "EffectCraft",
            ToolKind::Kokoro => "Kokoro TTS",
            ToolKind::Ffmpeg => "FFmpeg",
            ToolKind::Ffprobe => "ffprobe",
        };
        write!(f, "{}", s)
    }
}

/// The result of probing a single tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Available { version: String, path: String },
    Unavailable { reason: String },
    OptionalUnavailable { reason: String },
}

impl ToolStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, ToolStatus::Available { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolProbeResult {
    pub kind: ToolKind,
    pub status: ToolStatus,
}

/// Channel/production profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChannelProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_privacy: PrivacyMode,
    pub auto_publish: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    #[default]
    Private,
    Unlisted,
    Public,
    Scheduled,
}

/// Application-wide configuration, persisted in SQLite and shown with `--redact`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AtlasConfig {
    pub version: String,
    pub data_dir: String,
    pub opencode_account_hint: Option<String>,
    pub preferred_nvidia_model: Option<String>,
    pub channel_profile: ChannelProfile,
    pub render_presets: Vec<RenderPreset>,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            data_dir: "data".to_string(),
            opencode_account_hint: None,
            preferred_nvidia_model: Some(
                "nvidia/nvidia/llama-3.1-nemotron-70b-instruct".to_string(),
            ),
            channel_profile: ChannelProfile::default(),
            render_presets: vec![RenderPreset::default()],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderPreset {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video_codec: String,
    pub audio_codec: String,
}

impl Default for RenderPreset {
    fn default() -> Self {
        Self {
            id: "youtube-1080p".to_string(),
            name: "YouTube 1080p".to_string(),
            width: 1920,
            height: 1080,
            fps: 30,
            video_codec: "h264".to_string(),
            audio_codec: "aac".to_string(),
        }
    }
}

/// High-level project/video state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub video_id: String,
    pub topic: Topic,
    pub state: ProjectState,
    pub channel_profile: String,
    pub visual_thesis: Option<String>,
    pub script_artifact_id: Option<String>,
    pub research_artifact_id: Option<String>,
    pub storyboard_artifact_id: Option<String>,
    pub publish: PublishPlan,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Topic {
    pub title: String,
    pub trend_sources: Vec<TrendSource>,
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub region: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrendSource {
    pub source: String,
    pub url: Option<String>,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectState {
    #[default]
    IdeaDiscovered,
    TopicApproved,
    Researching,
    EvidenceApproved,
    Scripting,
    ScriptApproved,
    Storyboarding,
    StoryboardApproved,
    AudioBuild,
    AssetBuild,
    PreviewRender,
    PreviewQa,
    FinalRender,
    EditAndMix,
    ReleaseQa,
    ReadyToPublish,
    Uploading,
    UploadedProcessing,
    Published,
    MetricsReview,
}

impl fmt::Display for ProjectState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string(self)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        write!(f, "{}", s)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PublishPlan {
    pub mode: PublishMode,
    pub contains_synthetic_media: Option<bool>,
    pub credits_present: bool,
    pub youtube_video_id: Option<String>,
    #[serde(default)]
    pub youtube_video_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub tiktok_post_id: Option<String>,
    #[serde(default)]
    pub tiktok_post_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub tiktok_request_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublishPlatform {
    #[default]
    YouTube,
    TikTok,
    Both,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishMode {
    #[default]
    ApprovalRequired,
    Autonomous,
}

/// Shared agent message envelope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMessage {
    pub schema_version: u32,
    pub message_id: String,
    pub project_id: String,
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub task_id: Option<String>,
    pub artifact_ids: Vec<String>,
    pub blocking: bool,
    pub priority: String,
    pub feedback: Vec<FeedbackItem>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deadline_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackItem {
    pub claim_id: Option<String>,
    pub issue: String,
    pub required_change: String,
}

/// Redacted view of configuration for safe display.
pub fn redact_config(config: &AtlasConfig) -> serde_json::Value {
    let mut value = serde_json::to_value(config).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        obj.insert("_redacted".to_string(), serde_json::Value::Bool(true));
        obj.insert(
            "opencode_account_hint".to_string(),
            config
                .opencode_account_hint
                .as_ref()
                .map(|h| format!("{}...", &h[..h.len().min(8)]))
                .unwrap_or_else(|| "not configured".into())
                .into(),
        );
    }
    value
}

/// Content-addressed artifact reference.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_id: String,
    pub project_id: String,
    pub kind: String,
    pub sha256: String,
    pub path: String,
    pub metadata: serde_json::Value,
}

/// Clearly Drawn News Show Identity
pub const COMPANY_NAME: &str = "Clearly Drawn Studios";
pub const COMPANY_TAGLINE: &str = "Every story, clearly drawn.";
pub const PRODUCTION_ENGINE: &str = "ATLAS";
pub const SHOW_TITLE: &str = "Clearly Drawn News";
pub const CHANNEL_NAME: &str = "Clearly Drawn";
pub const SHOW_TAGLINE: &str = "Animation news with personality.";

/// Broadcast format strictly enforcing native 16:9 widescreen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BroadcastFormat {
    /// 3840 x 2160 (4K UHD)
    Uhd4K,
    /// 1920 x 1080 (Full HD fallback)
    Fhd1080p,
}

impl BroadcastFormat {
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Uhd4K => (3840, 2160),
            Self::Fhd1080p => (1920, 1080),
        }
    }

    pub fn fps(self) -> u32 {
        30
    }
}

/// Enforced Episode Duration Policy: 15 to 30 minutes of actual finished content.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct EpisodeDurationPolicy {
    pub min_seconds: f64,
    pub max_seconds: f64,
    pub target_seconds: f64,
}

impl Default for EpisodeDurationPolicy {
    fn default() -> Self {
        Self {
            min_seconds: 900.0,
            max_seconds: 1800.0,
            target_seconds: 1320.0,
        }
    }
}

impl EpisodeDurationPolicy {
    pub fn is_valid(&self, duration_seconds: f64) -> bool {
        duration_seconds >= self.min_seconds && duration_seconds <= self.max_seconds
    }

    pub fn validate(&self, duration_seconds: f64) -> Result<(), String> {
        if duration_seconds < self.min_seconds {
            Err(format!(
                "Episode duration {:.2}s is below the 15-minute minimum (900s required)",
                duration_seconds
            ))
        } else if duration_seconds > self.max_seconds {
            Err(format!(
                "Episode duration {:.2}s exceeds the 30-minute maximum (1800s maximum)",
                duration_seconds
            ))
        } else {
            Ok(())
        }
    }
}

/// 17-stage state machine for Clearly Drawn News production lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewsProductionStage {
    Research,
    SourceVerification,
    EditorialStorySelection,
    TelegramApprovalPending,
    TelegramApproved,
    ProducerActivation,
    LongFormScriptGeneration,
    ScriptFactAudit,
    ComedyAndDialoguePolish,
    VoicePerformanceGeneration,
    MetaHumanAnimation,
    UnrealEngineStudioFilming,
    GraphicsAndAssetIntegration,
    FinalEditorialAssembly,
    AudioMixAndMastering,
    TechnicalAndEditorialQa,
    Published,
}
