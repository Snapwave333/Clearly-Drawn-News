//! Official TikTok Content Posting API (Direct Post) with Login Kit for Desktop OAuth.
//!
//! Flow: `creator_info/query` (privacy options and the account's max video length) ->
//! `post/publish/video/init` (FILE_UPLOAD) -> sequential chunk PUTs to `upload_url` ->
//! `post/publish/status/fetch`. Needs the `video.publish` scope. Content from an unaudited
//! client can only be posted as `SELF_ONLY` (private) until TikTok audits the app.

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const TIKTOK_AUTHORIZE_URL: &str = "https://www.tiktok.com/v2/auth/authorize/";
pub const TIKTOK_API_BASE: &str = "https://open.tiktokapis.com";
pub const TIKTOK_SCOPES: &str = "user.info.basic,video.publish";
pub const TIKTOK_DEFAULT_REDIRECT: &str = "http://localhost:3455/callback/";
/// Every TikTok creator can post 3 minutes; some accounts allow more (creator_info reports it).
pub const TIKTOK_BASELINE_MAX_SECS: u32 = 180;
/// Caption limit in UTF-16 code units.
pub const TIKTOK_CAPTION_MAX_UTF16: usize = 2200;

const MIN_CHUNK: u64 = 5 * 1024 * 1024;
const DEFAULT_CHUNK: u64 = 10 * 1024 * 1024;
const MAX_WHOLE: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TikTokAppConfig {
    pub client_key: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub token_path: PathBuf,
}

impl TikTokAppConfig {
    /// `TIKTOK_CLIENT_KEY`, `TIKTOK_CLIENT_SECRET`, optional `TIKTOK_REDIRECT_URI` and
    /// `ATLAS_TIKTOK_TOKEN` (default `tiktok_token.json` in the working directory).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let get = |name: &str| lookup(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let client_key = get("TIKTOK_CLIENT_KEY").ok_or_else(|| anyhow!("TIKTOK_CLIENT_KEY is not set"))?;
        let client_secret = get("TIKTOK_CLIENT_SECRET").ok_or_else(|| anyhow!("TIKTOK_CLIENT_SECRET is not set"))?;
        Ok(Self {
            client_key,
            client_secret,
            redirect_uri: get("TIKTOK_REDIRECT_URI").unwrap_or_else(|| TIKTOK_DEFAULT_REDIRECT.into()),
            token_path: PathBuf::from(get("ATLAS_TIKTOK_TOKEN").unwrap_or_else(|| "tiktok_token.json".into())),
        })
    }

    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }
}

/// PKCE pair for Login Kit for Desktop: TikTok wants the hex SHA-256 of the verifier.
pub fn pkce_pair() -> (String, String) {
    let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

pub fn pkce_challenge(verifier: &str) -> String {
    hex::encode(Sha256::digest(verifier.as_bytes()))
}

pub fn authorize_url(config: &TikTokAppConfig, state: &str, code_challenge: &str) -> String {
    let mut url = url::Url::parse(TIKTOK_AUTHORIZE_URL).expect("static url");
    url.query_pairs_mut()
        .append_pair("client_key", &config.client_key)
        .append_pair("response_type", "code")
        .append_pair("scope", TIKTOK_SCOPES)
        .append_pair("redirect_uri", &config.redirect_uri)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    url.to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokToken {
    pub access_token: String,
    pub refresh_token: String,
    pub open_id: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    open_id: Option<String>,
    scope: Option<String>,
    expires_in: Option<i64>,
    refresh_expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

impl TokenResponse {
    fn into_token(self) -> anyhow::Result<TikTokToken> {
        if let Some(error) = self.error.filter(|e| !e.is_empty()) {
            bail!("TikTok OAuth error {error}: {}", self.error_description.unwrap_or_default());
        }
        let now = Utc::now();
        Ok(TikTokToken {
            access_token: self.access_token.ok_or_else(|| anyhow!("TikTok OAuth: no access_token"))?,
            refresh_token: self.refresh_token.unwrap_or_default(),
            open_id: self.open_id.unwrap_or_default(),
            scope: self.scope.unwrap_or_default(),
            expires_at: now + Duration::seconds(self.expires_in.unwrap_or(86_400)),
            refresh_expires_at: now + Duration::seconds(self.refresh_expires_in.unwrap_or(0)),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TikTokCreatorInfo {
    #[serde(default)]
    pub creator_username: String,
    #[serde(default)]
    pub creator_nickname: String,
    #[serde(default)]
    pub privacy_level_options: Vec<String>,
    #[serde(default)]
    pub comment_disabled: bool,
    #[serde(default)]
    pub duet_disabled: bool,
    #[serde(default)]
    pub stitch_disabled: bool,
    #[serde(default)]
    pub max_video_post_duration_sec: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TikTokPostInfo {
    /// Caption: text, hashtags and mentions; at most 2200 UTF-16 units.
    pub title: String,
    pub privacy_level: String,
    pub disable_duet: bool,
    pub disable_comment: bool,
    pub disable_stitch: bool,
    pub video_cover_timestamp_ms: u32,
    pub brand_content_toggle: bool,
    pub brand_organic_toggle: bool,
    pub is_aigc: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkPlan {
    pub chunk_size: u64,
    pub total_chunk_count: u64,
}

impl ChunkPlan {
    /// (first_byte, last_byte) of chunk `index`; the final chunk absorbs the trailing bytes.
    pub fn range(&self, index: u64, video_size: u64) -> (u64, u64) {
        let first = index * self.chunk_size;
        let last = if index + 1 == self.total_chunk_count { video_size - 1 } else { first + self.chunk_size - 1 };
        (first, last)
    }
}

/// TikTok chunk rules: files under 5 MB (and up to 64 MB) may go whole; larger ones go in
/// 10 MB chunks, count = floor(size / chunk), the last chunk carrying the remainder.
pub fn plan_chunks(video_size: u64) -> ChunkPlan {
    if video_size <= MAX_WHOLE {
        return ChunkPlan { chunk_size: video_size, total_chunk_count: 1 };
    }
    let chunk_size = DEFAULT_CHUNK.max(MIN_CHUNK);
    ChunkPlan { chunk_size, total_chunk_count: (video_size / chunk_size).max(1) }
}

/// Clips a caption to TikTok's 2200 UTF-16 unit limit on a character boundary.
pub fn clip_caption(text: &str) -> String {
    let mut used = 0;
    let mut out = String::new();
    for ch in text.chars() {
        used += ch.len_utf16();
        if used > TIKTOK_CAPTION_MAX_UTF16 {
            break;
        }
        out.push(ch);
    }
    out
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: Option<T>,
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct InitData {
    publish_id: String,
    upload_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TikTokPublishStatus {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub fail_reason: Option<String>,
    #[serde(default, alias = "publicaly_available_post_id")]
    pub publicly_available_post_id: Vec<serde_json::Value>,
}

pub struct TikTokDirectClient {
    config: TikTokAppConfig,
    http: reqwest::Client,
    base: String,
}

impl TikTokDirectClient {
    pub fn new(config: TikTokAppConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .unwrap_or_default(),
            base: TIKTOK_API_BASE.into(),
        }
    }

    pub fn config(&self) -> &TikTokAppConfig {
        &self.config
    }

    pub fn load_token(&self) -> Option<TikTokToken> {
        std::fs::read(&self.config.token_path).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    fn save_token(&self, token: &TikTokToken) -> anyhow::Result<()> {
        std::fs::write(&self.config.token_path, serde_json::to_vec_pretty(token)?)
            .with_context(|| format!("writing {}", self.config.token_path.display()))
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> anyhow::Result<TikTokToken> {
        let response = self
            .http
            .post(format!("{}/v2/oauth/token/", self.base))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(form)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        let parsed: TokenResponse = serde_json::from_str(&body)
            .map_err(|_| anyhow!("TikTok OAuth HTTP {status}: {}", body.chars().take(300).collect::<String>()))?;
        let token = parsed.into_token()?;
        self.save_token(&token)?;
        Ok(token)
    }

    /// Exchanges the `code` from the redirect URL (with the PKCE verifier) and stores the token.
    pub async fn exchange_code(&self, code: &str, code_verifier: &str) -> anyhow::Result<TikTokToken> {
        let c = &self.config;
        self.token_request(&[
            ("client_key", c.client_key.as_str()),
            ("client_secret", c.client_secret.as_str()),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", c.redirect_uri.as_str()),
            ("code_verifier", code_verifier),
        ])
        .await
    }

    /// Valid access token, refreshing it (and saving) when it expires within 5 minutes.
    pub async fn access_token(&self) -> anyhow::Result<String> {
        let token = self.load_token().ok_or_else(|| {
            anyhow!("no TikTok token at {}; run `atlas news tiktok-auth`", self.config.token_path.display())
        })?;
        if token.expires_at > Utc::now() + Duration::minutes(5) {
            return Ok(token.access_token);
        }
        if token.refresh_token.is_empty() || token.refresh_expires_at <= Utc::now() {
            bail!("TikTok refresh token expired; run `atlas news tiktok-auth` again");
        }
        let c = &self.config;
        let fresh = self
            .token_request(&[
                ("client_key", c.client_key.as_str()),
                ("client_secret", c.client_secret.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", token.refresh_token.as_str()),
            ])
            .await?;
        Ok(fresh.access_token)
    }

    async fn post_json<T: serde::de::DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> anyhow::Result<T> {
        let token = self.access_token().await?;
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(token)
            .header("Content-Type", "application/json; charset=UTF-8")
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        let envelope: Envelope<T> = serde_json::from_str(&text)
            .map_err(|_| anyhow!("TikTok {path} HTTP {status}: {}", text.chars().take(300).collect::<String>()))?;
        if let Some(error) = &envelope.error {
            if !error.code.is_empty() && error.code != "ok" {
                bail!("TikTok {path} {}: {}", error.code, error.message);
            }
        }
        envelope.data.ok_or_else(|| anyhow!("TikTok {path} HTTP {status}: no data"))
    }

    /// Read-only: the account's privacy options, interaction settings and max video length.
    pub async fn creator_info(&self) -> anyhow::Result<TikTokCreatorInfo> {
        self.post_json("/v2/post/publish/creator_info/query/", serde_json::json!({})).await
    }

    /// Direct Post of a local MP4. Returns the publish_id.
    pub async fn post_video(&self, video: &Path, post: &TikTokPostInfo) -> anyhow::Result<String> {
        let size = tokio::fs::metadata(video).await.with_context(|| video.display().to_string())?.len();
        if size == 0 {
            bail!("{} is empty", video.display());
        }
        let plan = plan_chunks(size);
        let init: InitData = self
            .post_json(
                "/v2/post/publish/video/init/",
                serde_json::json!({
                    "post_info": post,
                    "source_info": {
                        "source": "FILE_UPLOAD",
                        "video_size": size,
                        "chunk_size": plan.chunk_size,
                        "total_chunk_count": plan.total_chunk_count,
                    }
                }),
            )
            .await?;
        let upload_url = init.upload_url.ok_or_else(|| anyhow!("TikTok init returned no upload_url"))?;
        let bytes = tokio::fs::read(video).await?;
        for index in 0..plan.total_chunk_count {
            let (first, last) = plan.range(index, size);
            let chunk = bytes[first as usize..=last as usize].to_vec();
            let response = self
                .http
                .put(&upload_url)
                .header("Content-Type", "video/mp4")
                .header("Content-Length", chunk.len())
                .header("Content-Range", format!("bytes {first}-{last}/{size}"))
                .body(chunk)
                .send()
                .await?;
            let status = response.status().as_u16();
            if status != 201 && status != 206 {
                bail!("TikTok chunk {}/{} upload HTTP {status}", index + 1, plan.total_chunk_count);
            }
        }
        Ok(init.publish_id)
    }

    pub async fn publish_status(&self, publish_id: &str) -> anyhow::Result<TikTokPublishStatus> {
        self.post_json("/v2/post/publish/status/fetch/", serde_json::json!({ "publish_id": publish_id })).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_plan_follows_tiktok_rules() {
        assert_eq!(plan_chunks(4_194_304), ChunkPlan { chunk_size: 4_194_304, total_chunk_count: 1 });
        assert_eq!(plan_chunks(60_000_000).total_chunk_count, 1);
        let size = 100_000_123;
        let plan = plan_chunks(size);
        assert_eq!(plan.total_chunk_count, size / DEFAULT_CHUNK);
        let (first, last) = plan.range(plan.total_chunk_count - 1, size);
        assert_eq!(last, size - 1);
        assert!(last - first + 1 >= DEFAULT_CHUNK && last - first < 128 * 1024 * 1024);
        assert_eq!(plan.range(0, size), (0, DEFAULT_CHUNK - 1));
    }

    #[test]
    fn pkce_challenge_is_hex_sha256() {
        assert_eq!(
            pkce_challenge("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let (verifier, challenge) = pkce_pair();
        assert_eq!(verifier.len(), 64);
        assert_eq!(challenge.len(), 64);
    }

    #[test]
    fn authorize_url_carries_scope_and_pkce() {
        let config = TikTokAppConfig::from_lookup(|name| match name {
            "TIKTOK_CLIENT_KEY" => Some("key123".into()),
            "TIKTOK_CLIENT_SECRET" => Some("secret".into()),
            _ => None,
        })
        .unwrap();
        let url = authorize_url(&config, "st", "ch");
        assert!(url.starts_with(TIKTOK_AUTHORIZE_URL));
        assert!(url.contains("client_key=key123"));
        assert!(url.contains("scope=user.info.basic%2Cvideo.publish"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(!url.contains("secret"));
        assert!(TikTokAppConfig::from_lookup(|_| None).is_err());
    }

    #[test]
    fn caption_is_clipped_in_utf16_units() {
        let long = "🎬".repeat(2000);
        assert_eq!(clip_caption(&long).encode_utf16().count(), 2200);
    }
}
