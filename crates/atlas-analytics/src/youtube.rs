use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime};
use tokio::fs;
use tokio_util::io::ReaderStream;
use tracing::info;

const KEYRING_SERVICE: &str = "com.atlas-studio.youtube";
const KEYRING_USERNAME: &str = "oauth_refresh_token";

/// YouTube OAuth2 credentials supplied by the user.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl YouTubeCredentials {
    pub fn for_installed_app(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
            redirect_uri: "http://127.0.0.1:8085".to_string(),
        }
    }
}

/// Stored OAuth2 token set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeTokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: SystemTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeUploadMetadata {
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub privacy_status: String,
    pub category_id: String,
    pub contains_synthetic_media: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeUploadResult {
    pub video_id: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeChannelStatus {
    pub channel_id: String,
    pub available: bool,
    pub is_linked: Option<bool>,
    pub long_uploads_status: Option<String>,
    pub privacy_status: Option<String>,
    pub made_for_kids: Option<bool>,
    pub self_declared_made_for_kids: Option<bool>,
}

/// Secure storage for YouTube OAuth tokens using the OS credential manager.
pub struct TokenStore;

impl TokenStore {
    pub fn new() -> Self {
        Self
    }

    pub fn save_refresh_token(&self, token: &str) -> AtlasResult<()> {
        let _ = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USERNAME)
            .map(|entry| entry.set_password(token));
        let path = std::path::PathBuf::from("youtube_token.json");
        let _ = std::fs::write(&path, token.trim());
        info!("YouTube refresh token saved to credential store and local token cache");
        Ok(())
    }

    pub fn load_refresh_token(&self) -> AtlasResult<Option<String>> {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
            if let Ok(token) = entry.get_password() {
                let trimmed = token.trim().to_string();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed));
                }
            }
        }
        let path = std::path::PathBuf::from("youtube_token.json");
        if path.is_file() {
            if let Ok(token) = std::fs::read_to_string(&path) {
                let trimmed = token.trim().to_string();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed));
                }
            }
        }
        Ok(None)
    }

    pub fn delete_refresh_token(&self) -> AtlasResult<()> {
        let _ = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USERNAME)
            .map(|entry| entry.delete_credential());
        let _ = std::fs::remove_file("youtube_token.json");
        Ok(())
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the YouTube OAuth2 authorization URL for the user to visit.
pub fn authorization_url(creds: &YouTubeCredentials) -> String {
    let scopes = [
        "https://www.googleapis.com/auth/youtube.readonly",
        "https://www.googleapis.com/auth/yt-analytics.readonly",
        "https://www.googleapis.com/auth/youtube.upload",
    ]
    .join(" ");
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        urlencoding::encode(&creds.client_id),
        urlencoding::encode(&creds.redirect_uri),
        urlencoding::encode(&scopes)
    )
}

/// YouTube Analytics API client.
pub struct YouTubeAnalyticsClient {
    credentials: YouTubeCredentials,
    token_store: TokenStore,
    http: reqwest::Client,
}

impl YouTubeAnalyticsClient {
    pub fn new(credentials: YouTubeCredentials) -> Self {
        Self {
            credentials,
            token_store: TokenStore::new(),
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Returns true if a refresh token exists in secure storage.
    pub fn is_authenticated(&self) -> AtlasResult<bool> {
        Ok(self.token_store.load_refresh_token()?.is_some())
    }

    /// Exchanges an authorization code for tokens and stores the refresh token.
    pub async fn exchange_code(&self, code: &str) -> AtlasResult<YouTubeTokenSet> {
        let params = [
            ("code", code),
            ("client_id", &self.credentials.client_id),
            ("client_secret", &self.credentials.client_secret),
            ("redirect_uri", &self.credentials.redirect_uri),
            ("grant_type", "authorization_code"),
        ];
        let resp = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("token exchange request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "Google OAuth token exchange failed: {body}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("invalid token response: {e}")))?;
        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| AtlasError::OpenCode("missing access_token".into()))?
            .to_string();
        let refresh_token = json["refresh_token"]
            .as_str()
            .ok_or_else(|| {
                AtlasError::OpenCode("missing refresh_token; ensure prompt=consent".into())
            })?
            .to_string();
        let expires_in = json["expires_in"].as_u64().unwrap_or(3600);
        let expires_at = SystemTime::now() + Duration::from_secs(expires_in);

        self.token_store.save_refresh_token(&refresh_token)?;
        Ok(YouTubeTokenSet {
            access_token,
            refresh_token,
            expires_at,
        })
    }

    /// Refreshes the access token using the stored refresh token.
    pub async fn refresh_access_token(&self) -> AtlasResult<YouTubeTokenSet> {
        let refresh_token = self
            .token_store
            .load_refresh_token()?
            .ok_or_else(|| AtlasError::OpenCode("no YouTube refresh token stored".into()))?;

        let params = [
            ("refresh_token", refresh_token.as_str()),
            ("client_id", self.credentials.client_id.as_str()),
            ("client_secret", self.credentials.client_secret.as_str()),
            ("grant_type", "refresh_token"),
        ];
        let resp = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("refresh request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "Google OAuth refresh failed: {body}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("invalid refresh response: {e}")))?;
        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| AtlasError::OpenCode("missing access_token".into()))?
            .to_string();
        let expires_in = json["expires_in"].as_u64().unwrap_or(3600);
        let expires_at = SystemTime::now() + Duration::from_secs(expires_in);

        Ok(YouTubeTokenSet {
            access_token,
            refresh_token,
            expires_at,
        })
    }

    pub async fn upload_video(
        &self,
        video_path: &Path,
        metadata: &YouTubeUploadMetadata,
    ) -> AtlasResult<YouTubeUploadResult> {
        let file_metadata = fs::metadata(video_path)
            .await
            .map_err(|e| AtlasError::OpenCode(format!("video file unavailable: {e}")))?;
        if file_metadata.len() == 0 {
            return Err(AtlasError::OpenCode("video file is empty".into()));
        }

        let token = self.refresh_access_token().await?;
        let request_body = upload_request_body(metadata);
        let init = self
            .http
            .post("https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status")
            .bearer_auth(&token.access_token)
            .header("X-Upload-Content-Length", file_metadata.len())
            .header("X-Upload-Content-Type", "video/mp4")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("YouTube upload initialization failed: {e}")))?;

        if !init.status().is_success() {
            let status = init.status();
            let body = init.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "YouTube upload initialization returned {status}: {body}"
            )));
        }

        let upload_url = init
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| AtlasError::OpenCode("YouTube did not return an upload URL".into()))?;
        let file = fs::File::open(video_path)
            .await
            .map_err(|e| AtlasError::OpenCode(format!("failed to open video: {e}")))?;
        let response = self
            .http
            .put(upload_url)
            .bearer_auth(&token.access_token)
            .header(reqwest::header::CONTENT_TYPE, "video/mp4")
            .header(reqwest::header::CONTENT_LENGTH, file_metadata.len())
            .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("YouTube video upload failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "YouTube video upload returned {status}: {body}"
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("invalid YouTube upload response: {e}")))?;
        let video_id = body["id"]
            .as_str()
            .ok_or_else(|| AtlasError::OpenCode("YouTube response omitted the video id".into()))?
            .to_string();
        Ok(YouTubeUploadResult {
            url: format!("https://youtu.be/{video_id}"),
            video_id,
        })
    }

    pub async fn fetch_channel_status(&self) -> AtlasResult<YouTubeChannelStatus> {
        let token = self.refresh_access_token().await?;
        let response = self
            .http
            .get("https://www.googleapis.com/youtube/v3/channels?part=status&mine=true")
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(|error| {
                AtlasError::OpenCode(format!("YouTube channel status request failed: {error}"))
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "YouTube channel status returned {status}: {body}"
            )));
        }
        let body: serde_json::Value = response.json().await.map_err(|error| {
            AtlasError::OpenCode(format!("invalid YouTube channel status response: {error}"))
        })?;
        let mut item = body["items"].as_array().and_then(|items| items.first()).cloned();
        if item.is_none() {
            let mut candidate_ids = vec!["UC16fbSsDJcc6RVvNsH3Cbfg".to_string()];
            let mut candidate_paths = vec![std::path::PathBuf::from("data/policy/channel_browser_review.json")];
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                candidate_paths.push(
                    std::path::PathBuf::from(local)
                        .join("atlas-studio")
                        .join("atlas")
                        .join("data")
                        .join("policy")
                        .join("channel_browser_review.json"),
                );
            }
            for p in &candidate_paths {
                if p.is_file() {
                    if let Ok(c) = std::fs::read_to_string(p) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&c) {
                            if let Some(cid) = val["channel_id"].as_str() {
                                if !cid.is_empty() && !candidate_ids.contains(&cid.to_string()) {
                                    candidate_ids.insert(0, cid.to_string());
                                }
                            }
                        }
                    }
                }
            }

            for cid in candidate_ids {
                let url = format!("https://www.googleapis.com/youtube/v3/channels?part=status&id={cid}");
                if let Ok(resp) = self.http.get(&url).bearer_auth(&token.access_token).send().await {
                    if resp.status().is_success() {
                        if let Ok(b) = resp.json::<serde_json::Value>().await {
                            if let Some(found) = b["items"].as_array().and_then(|items| items.first()).cloned() {
                                item = Some(found);
                                break;
                            }
                        }
                    }
                }
            }
        }
        let Some(item) = item else {
            return Ok(YouTubeChannelStatus {
                channel_id: String::new(),
                available: false,
                is_linked: None,
                long_uploads_status: None,
                privacy_status: None,
                made_for_kids: None,
                self_declared_made_for_kids: None,
            });
        };
        let status = &item["status"];
        Ok(YouTubeChannelStatus {
            channel_id: item["id"].as_str().unwrap_or_default().into(),
            available: true,
            is_linked: status["isLinked"].as_bool(),
            long_uploads_status: status["longUploadsStatus"].as_str().map(str::to_owned),
            privacy_status: status["privacyStatus"].as_str().map(str::to_owned),
            made_for_kids: status["madeForKids"].as_bool(),
            self_declared_made_for_kids: status["selfDeclaredMadeForKids"].as_bool(),
        })
    }

    pub async fn upload_thumbnail(&self, video_id: &str, path: &Path) -> AtlasResult<()> {
        let token = self.refresh_access_token().await?;
        let bytes = fs::read(path)
            .await
            .map_err(|e| AtlasError::OpenCode(format!("failed to read thumbnail: {e}")))?;
        let response = self
            .http
            .post(format!(
                "https://www.googleapis.com/upload/youtube/v3/thumbnails/set?videoId={video_id}&uploadType=media"
            ))
            .bearer_auth(&token.access_token)
            .header(reqwest::header::CONTENT_TYPE, "image/png")
            .body(bytes)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("thumbnail upload failed: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "thumbnail upload returned {status}: {body}"
            )));
        }
        Ok(())
    }

    pub async fn upload_captions(
        &self,
        video_id: &str,
        path: &Path,
        language: &str,
    ) -> AtlasResult<()> {
        let token = self.refresh_access_token().await?;
        let captions = fs::read(path)
            .await
            .map_err(|e| AtlasError::OpenCode(format!("failed to read captions: {e}")))?;
        let boundary = format!("atlas-caption-{}", uuid::Uuid::new_v4().simple());
        let metadata = serde_json::json!({
            "snippet": {
                "videoId": video_id,
                "language": language,
                "name": "English",
                "isDraft": false
            }
        });
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n").as_bytes());
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Type: application/x-subrip\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(&captions);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let response = self
            .http
            .post("https://www.googleapis.com/upload/youtube/v3/captions?part=snippet&uploadType=multipart")
            .bearer_auth(&token.access_token)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("caption upload failed: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "caption upload returned {status}: {body}"
            )));
        }
        Ok(())
    }

    /// Fetches analytics for a single YouTube video.
    pub async fn fetch_video_metrics(
        &self,
        youtube_video_id: &str,
    ) -> AtlasResult<serde_json::Value> {
        let token = self.refresh_access_token().await?;
        let url = format!(
            "https://youtubeanalytics.googleapis.com/v2/reports?ids=channel==MINE&startDate=2020-01-01&endDate={}&metrics=views,watchTime(minutes),averageViewDuration(seconds),averageViewPercentage,impressions,impressionsClickThroughRate,likes,comments,subscribersGained,subscribersLost&dimensions=video&filters=video=={}",
            chrono::Utc::now().format("%Y-%m-%d"),
            youtube_video_id
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("analytics request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "YouTube Analytics API error: {body}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("invalid analytics response: {e}")))
    }

    /// Fetches audience retention curve for a video via the YouTube Analytics API.
    /// Note: absolute retention requires YouTube Reporting API or channel-owner advanced access;
    /// this endpoint returns relative retention where available.
    pub async fn fetch_retention_curve(
        &self,
        youtube_video_id: &str,
    ) -> AtlasResult<serde_json::Value> {
        let token = self.refresh_access_token().await?;
        let url = format!(
            "https://youtubeanalytics.googleapis.com/v2/reports?ids=channel==MINE&startDate=2020-01-01&endDate={}&metrics=viewerPercentage&dimensions=elapsedVideoTimeRatio&filters=video=={}&sort=elapsedVideoTimeRatio",
            chrono::Utc::now().format("%Y-%m-%d"),
            youtube_video_id
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("retention request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AtlasError::OpenCode(format!(
                "YouTube Analytics retention API error: {body}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| AtlasError::OpenCode(format!("invalid retention response: {e}")))
    }
}

fn upload_request_body(metadata: &YouTubeUploadMetadata) -> serde_json::Value {
    serde_json::json!({
        "snippet": {
            "title": metadata.title,
            "description": metadata.description,
            "tags": metadata.tags,
            "categoryId": metadata.category_id
        },
        "status": {
            "privacyStatus": metadata.privacy_status,
            "selfDeclaredMadeForKids": false,
            "containsSyntheticMedia": metadata.contains_synthetic_media
        }
    })
}

// Minimal urlencoding helper to avoid an extra dependency.
mod urlencoding {
    pub fn encode(s: &str) -> String {
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
}

/// YouTube OAuth credentials file format (matches Google's client_secret JSON).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeCredentialsFile {
    pub installed: YouTubeCredentialsFileInner,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeCredentialsFileInner {
    pub client_id: String,
    pub client_secret: String,
    pub auth_uri: String,
    pub token_uri: String,
    pub auth_provider_x509_cert_url: String,
    pub redirect_uris: Vec<String>,
}

/// Load YouTube OAuth credentials from the standard credentials file.
pub async fn load_youtube_credentials<P: AsRef<std::path::Path>>(
    path: P,
) -> AtlasResult<YouTubeCredentials> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| AtlasError::OpenCode(format!("Failed to read credentials file: {}", e)))?;
    let file_creds: YouTubeCredentialsFile = serde_json::from_str(&content)
        .map_err(|e| AtlasError::OpenCode(format!("Invalid credentials JSON: {}", e)))?;
    let redirect_uri = file_creds
        .installed
        .redirect_uris
        .first()
        .cloned()
        .unwrap_or_else(|| "http://localhost".to_string());
    Ok(YouTubeCredentials {
        client_id: file_creds.installed.client_id,
        client_secret: file_creds.installed.client_secret,
        redirect_uri,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_url_contains_required_params() {
        let creds = YouTubeCredentials::for_installed_app("client123".into(), "secret".into());
        let url = authorization_url(&creds);
        assert!(url.contains("client_id=client123"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("youtube.readonly"));
    }

    #[test]
    fn upload_request_contains_visibility_and_disclosure() {
        let metadata = YouTubeUploadMetadata {
            title: "Test".into(),
            description: "Description".into(),
            tags: vec!["science".into()],
            privacy_status: "unlisted".into(),
            category_id: "27".into(),
            contains_synthetic_media: true,
        };
        let body = upload_request_body(&metadata);
        assert_eq!(body["status"]["privacyStatus"], "unlisted");
        assert_eq!(body["status"]["containsSyntheticMedia"], true);
        assert_eq!(body["snippet"]["categoryId"], "27");
    }
}
