use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use tokio::fs;
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

/// Secure storage for YouTube OAuth tokens using the OS credential manager.
pub struct TokenStore;

impl TokenStore {
    pub fn new() -> Self {
        Self
    }

    pub fn save_refresh_token(&self, token: &str) -> AtlasResult<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USERNAME)
            .map_err(|e| AtlasError::OpenCode(format!("keyring entry failed: {e}")))?;
        entry
            .set_password(token)
            .map_err(|e| AtlasError::OpenCode(format!("failed to save refresh token: {e}")))?;
        info!("YouTube refresh token saved to OS credential store");
        Ok(())
    }

    pub fn load_refresh_token(&self) -> AtlasResult<Option<String>> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USERNAME)
            .map_err(|e| AtlasError::OpenCode(format!("keyring entry failed: {e}")))?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AtlasError::OpenCode(format!(
                "failed to load refresh token: {e}"
            ))),
        }
    }

    pub fn delete_refresh_token(&self) -> AtlasResult<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USERNAME)
            .map_err(|e| AtlasError::OpenCode(format!("keyring entry failed: {e}")))?;
        entry
            .delete_credential()
            .map_err(|e| AtlasError::OpenCode(format!("failed to delete token: {e}")))?;
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
            http: reqwest::Client::new(),
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
            .ok_or_else(|| AtlasError::OpenCode("missing refresh_token; ensure prompt=consent".into()))?
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

    /// Fetches analytics for a single YouTube video.
    pub async fn fetch_video_metrics(&self, youtube_video_id: &str) -> AtlasResult<serde_json::Value> {
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
pub async fn load_youtube_credentials<P: AsRef<std::path::Path>>(path: P) -> AtlasResult<YouTubeCredentials> {
    let content = fs::read_to_string(path).await
        .map_err(|e| AtlasError::OpenCode(format!("Failed to read credentials file: {}", e)))?;
    let file_creds: YouTubeCredentialsFile = serde_json::from_str(&content)
        .map_err(|e| AtlasError::OpenCode(format!("Invalid credentials JSON: {}", e)))?;
    let redirect_uri = file_creds.installed.redirect_uris.first()
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
}
