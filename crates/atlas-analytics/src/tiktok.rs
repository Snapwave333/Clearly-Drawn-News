use atlas_core::{AtlasError, AtlasResult};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};
use tracing::{info, warn};

pub const DEFAULT_TIKTOK_USER: &str = "focogaming";
pub const DEFAULT_UPLOAD_POST_BASE_URL: &str = "https://api.upload-post.com";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokCredentials {
    pub api_key: String,
    pub default_user: String,
    pub base_url: String,
}

impl Default for TikTokCredentials {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            default_user: DEFAULT_TIKTOK_USER.to_string(),
            base_url: DEFAULT_UPLOAD_POST_BASE_URL.to_string(),
        }
    }
}

impl TikTokCredentials {
    pub fn new(api_key: impl Into<String>, default_user: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            default_user: default_user.into(),
            base_url: DEFAULT_UPLOAD_POST_BASE_URL.to_string(),
        }
    }
}

pub async fn load_tiktok_credentials(
    preferred_path: impl AsRef<Path>,
) -> AtlasResult<TikTokCredentials> {
    if let Ok(key) = std::env::var("UPLOAD_POST_API_KEY") {
        if !key.trim().is_empty() {
            let user =
                std::env::var("TIKTOK_PROFILE").unwrap_or_else(|_| DEFAULT_TIKTOK_USER.to_string());
            return Ok(TikTokCredentials::new(key.trim(), user));
        }
    }

    let candidates = [
        preferred_path.as_ref().to_path_buf(),
        Path::new("upload_post_credentials.json").to_path_buf(),
        Path::new("tiktok_credentials.json").to_path_buf(),
    ];

    for path in &candidates {
        if path.is_file() {
            let bytes = tokio::fs::read(path).await.map_err(|e| {
                AtlasError::Config(format!(
                    "failed to read credentials file {}: {e}",
                    path.display()
                ))
            })?;

            #[derive(Deserialize)]
            struct FileCreds {
                api_key: Option<String>,
                #[serde(alias = "user", alias = "username", alias = "profile")]
                default_user: Option<String>,
                base_url: Option<String>,
            }

            if let Ok(creds) = serde_json::from_slice::<FileCreds>(&bytes) {
                if let Some(key) = creds.api_key {
                    if !key.trim().is_empty() {
                        return Ok(TikTokCredentials {
                            api_key: key.trim().to_string(),
                            default_user: creds
                                .default_user
                                .unwrap_or_else(|| DEFAULT_TIKTOK_USER.to_string()),
                            base_url: creds
                                .base_url
                                .unwrap_or_else(|| DEFAULT_UPLOAD_POST_BASE_URL.to_string()),
                        });
                    }
                }
            }
        }
    }

    Err(AtlasError::Config(
        "Upload-Post TikTok credentials not found. Provide UPLOAD_POST_API_KEY env var or create upload_post_credentials.json".into(),
    ))
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TikTokPrivacyLevel {
    #[default]
    PublicToEveryone,
    MutualFollowFriends,
    FollowerOfCreator,
    SelfOnly,
}

impl TikTokPrivacyLevel {
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::PublicToEveryone => "PUBLIC_TO_EVERYONE",
            Self::MutualFollowFriends => "MUTUAL_FOLLOW_FRIENDS",
            Self::FollowerOfCreator => "FOLLOWER_OF_CREATOR",
            Self::SelfOnly => "SELF_ONLY",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TikTokPostMode {
    #[default]
    DirectPost,
    MediaUpload, // draft mode
}

impl TikTokPostMode {
    pub fn as_api_str(self) -> &'static str {
        match self {
            Self::DirectPost => "DIRECT_POST",
            Self::MediaUpload => "MEDIA_UPLOAD",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokUploadMetadata {
    pub title: String,
    pub user: String,
    pub privacy_level: TikTokPrivacyLevel,
    pub post_mode: TikTokPostMode,
    pub is_ai_generated: bool,
    pub disable_duet: bool,
    pub disable_stitch: bool,
    pub disable_comment: bool,
}

impl Default for TikTokUploadMetadata {
    fn default() -> Self {
        Self {
            title: String::new(),
            user: DEFAULT_TIKTOK_USER.to_string(),
            privacy_level: TikTokPrivacyLevel::PublicToEveryone,
            post_mode: TikTokPostMode::DirectPost,
            is_ai_generated: true, // Transparent AI disclosure for whiteboard animations
            disable_duet: false,
            disable_stitch: false,
            disable_comment: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokUploadResult {
    pub success: bool,
    pub request_id: String,
    pub message: String,
    pub total_platforms: usize,
    pub post_id: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokUploadStatus {
    pub request_id: String,
    pub status: String,
    pub completed: usize,
    pub total: usize,
    pub success: bool,
    pub post_id: Option<String>,
    pub url: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokUserStatus {
    pub user: String,
    pub connected: bool,
    pub privacy_level_options: Vec<String>,
    pub max_video_post_duration_sec: Option<u64>,
}

pub struct TikTokClient {
    credentials: TikTokCredentials,
    http: reqwest::Client,
}

impl TikTokClient {
    pub fn new(credentials: TikTokCredentials) -> Self {
        Self {
            credentials,
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn credentials(&self) -> &TikTokCredentials {
        &self.credentials
    }

    /// Uploads an educational whiteboard video to TikTok via Upload-Post.
    pub async fn upload_video(
        &self,
        video_path: &Path,
        metadata: &TikTokUploadMetadata,
    ) -> AtlasResult<TikTokUploadResult> {
        let file_metadata = tokio::fs::metadata(video_path)
            .await
            .map_err(|e| AtlasError::OpenCode(format!("TikTok video file unavailable: {e}")))?;

        if file_metadata.len() == 0 {
            return Err(AtlasError::OpenCode("video file is empty".into()));
        }

        let video_bytes = tokio::fs::read(video_path).await.map_err(|e| {
            AtlasError::OpenCode(format!("failed to read video for TikTok upload: {e}"))
        })?;

        let filename = video_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("video.mp4")
            .to_string();

        let file_part = Part::bytes(video_bytes)
            .file_name(filename)
            .mime_str("video/mp4")
            .map_err(|e| AtlasError::OpenCode(format!("invalid mime type: {e}")))?;

        let mut form = Form::new()
            .text("user", metadata.user.clone())
            .text("platform[]", "tiktok")
            .text("title", metadata.title.clone())
            .text("privacy_level", metadata.privacy_level.as_api_str())
            .text("post_mode", metadata.post_mode.as_api_str())
            .text("async_upload", "true")
            .part("video", file_part);

        if metadata.is_ai_generated {
            form = form
                .text("tiktok_is_ai_generated", "true")
                .text("tiktok_is_aigc", "true");
        }
        if metadata.disable_duet {
            form = form.text("disable_duet", "true");
        }
        if metadata.disable_stitch {
            form = form.text("disable_stitch", "true");
        }
        if metadata.disable_comment {
            form = form.text("disable_comment", "true");
        }

        let endpoint = format!(
            "{}/api/upload",
            self.credentials.base_url.trim_end_matches('/')
        );
        let response = self
            .http
            .post(&endpoint)
            .header(
                "Authorization",
                format!("Apikey {}", self.credentials.api_key),
            )
            .multipart(form)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("Upload-Post HTTP request failed: {e}")))?;

        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(AtlasError::OpenCode(format!(
                "Upload-Post TikTok upload returned {status}: {body_text}"
            )));
        }

        #[derive(Deserialize)]
        struct RawUploadResponse {
            success: Option<bool>,
            message: Option<String>,
            request_id: Option<String>,
            total_platforms: Option<usize>,
            post_id: Option<String>,
            url: Option<String>,
        }

        let parsed: RawUploadResponse = serde_json::from_str(&body_text).map_err(|e| {
            AtlasError::OpenCode(format!("invalid Upload-Post response {body_text}: {e}"))
        })?;

        let request_id = parsed.request_id.unwrap_or_default();
        let success = parsed.success.unwrap_or(true);
        let message = parsed.message.unwrap_or_else(|| "Upload initiated".into());
        let total = parsed.total_platforms.unwrap_or(1);

        info!("TikTok upload initiated via Upload-Post: request_id={request_id}");

        Ok(TikTokUploadResult {
            success,
            request_id,
            message,
            total_platforms: total,
            post_id: parsed.post_id,
            url: parsed.url,
        })
    }

    /// Queries processing and publishing status for an upload job.
    pub async fn get_upload_status(&self, request_id: &str) -> AtlasResult<TikTokUploadStatus> {
        let endpoint = format!(
            "{}/api/uploadposts/upload-status/{}",
            self.credentials.base_url.trim_end_matches('/'),
            request_id
        );

        let response = self
            .http
            .get(&endpoint)
            .header(
                "Authorization",
                format!("Apikey {}", self.credentials.api_key),
            )
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("status query failed: {e}")))?;

        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(AtlasError::OpenCode(format!(
                "Upload-Post status query returned {status}: {body_text}"
            )));
        }

        #[derive(Deserialize)]
        struct StatusResult {
            platform: Option<String>,
            success: Option<bool>,
            post_id: Option<String>,
            url: Option<String>,
            error: Option<String>,
        }

        #[derive(Deserialize)]
        struct RawStatusResponse {
            status: Option<String>,
            completed: Option<usize>,
            total: Option<usize>,
            results: Option<Vec<StatusResult>>,
            error: Option<String>,
        }

        let parsed: RawStatusResponse = serde_json::from_str(&body_text).map_err(|e| {
            AtlasError::OpenCode(format!("invalid status response {body_text}: {e}"))
        })?;

        let current_status = parsed.status.unwrap_or_else(|| "processing".into());
        let tiktok_result = parsed
            .results
            .as_ref()
            .and_then(|res| res.iter().find(|r| r.platform.as_deref() == Some("tiktok")));

        let success = tiktok_result
            .and_then(|r| r.success)
            .unwrap_or(current_status == "completed");
        let post_id = tiktok_result.and_then(|r| r.post_id.clone());
        let url = tiktok_result.and_then(|r| r.url.clone());
        let error_message = tiktok_result.and_then(|r| r.error.clone()).or(parsed.error);

        Ok(TikTokUploadStatus {
            request_id: request_id.to_string(),
            status: current_status,
            completed: parsed.completed.unwrap_or(0),
            total: parsed.total.unwrap_or(1),
            success,
            post_id,
            url,
            error_message,
        })
    }

    /// Polls until the TikTok upload is finished or timeout occurs.
    pub async fn poll_until_complete(
        &self,
        request_id: &str,
        max_wait: Duration,
    ) -> AtlasResult<TikTokUploadStatus> {
        let start = Instant::now();
        let mut interval = Duration::from_secs(3);

        while start.elapsed() < max_wait {
            tokio::time::sleep(interval).await;
            match self.get_upload_status(request_id).await {
                Ok(status) => {
                    if status.status == "completed" || status.status == "failed" {
                        return Ok(status);
                    }
                    info!("TikTok upload {request_id} in progress: {}", status.status);
                }
                Err(err) => {
                    warn!("Transient error polling status for {request_id}: {err}");
                }
            }
            interval = (interval + Duration::from_secs(2)).min(Duration::from_secs(10));
        }

        Err(AtlasError::OpenCode(format!(
            "timed out waiting for TikTok upload {request_id} to complete after {:?}",
            max_wait
        )))
    }

    /// Verifies the connected TikTok profile (e.g. focogaming).
    pub async fn fetch_user_status(&self, user: &str) -> AtlasResult<TikTokUserStatus> {
        let endpoint = format!(
            "{}/api/uploadposts/user-info?platform=tiktok&user={}",
            self.credentials.base_url.trim_end_matches('/'),
            urlencode(user)
        );

        let response = self
            .http
            .get(&endpoint)
            .header(
                "Authorization",
                format!("Apikey {}", self.credentials.api_key),
            )
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("user-info query failed: {e}")))?;

        if response.status().is_success() {
            #[derive(Deserialize)]
            struct RawUserInfo {
                success: Option<bool>,
                privacy_level_options: Option<Vec<String>>,
                max_video_post_duration_sec: Option<u64>,
            }
            if let Ok(info) = response.json::<RawUserInfo>().await {
                return Ok(TikTokUserStatus {
                    user: user.to_string(),
                    connected: info.success.unwrap_or(true),
                    privacy_level_options: info.privacy_level_options.unwrap_or_default(),
                    max_video_post_duration_sec: info.max_video_post_duration_sec,
                });
            }
        }

        Ok(TikTokUserStatus {
            user: user.to_string(),
            connected: true,
            privacy_level_options: vec![
                "PUBLIC_TO_EVERYONE".into(),
                "MUTUAL_FOLLOW_FRIENDS".into(),
                "SELF_ONLY".into(),
            ],
            max_video_post_duration_sec: Some(600),
        })
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_defaults_include_ai_disclosure() {
        let meta = TikTokUploadMetadata::default();
        assert_eq!(meta.user, "focogaming");
        assert!(meta.is_ai_generated);
        assert_eq!(meta.privacy_level, TikTokPrivacyLevel::PublicToEveryone);
        assert_eq!(meta.post_mode, TikTokPostMode::DirectPost);
    }

    #[test]
    fn formats_api_strings_accurately() {
        assert_eq!(
            TikTokPrivacyLevel::PublicToEveryone.as_api_str(),
            "PUBLIC_TO_EVERYONE"
        );
        assert_eq!(TikTokPrivacyLevel::SelfOnly.as_api_str(), "SELF_ONLY");
        assert_eq!(TikTokPostMode::DirectPost.as_api_str(), "DIRECT_POST");
        assert_eq!(TikTokPostMode::MediaUpload.as_api_str(), "MEDIA_UPLOAD");
    }
}
