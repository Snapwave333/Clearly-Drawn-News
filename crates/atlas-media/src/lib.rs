use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod fact_checker;
pub mod news_desk;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaOrientation {
    Landscape,
    Portrait,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedMedia {
    pub provider: String,
    pub provider_id: String,
    pub kind: MediaKind,
    pub file_path: PathBuf,
    pub source_url: String,
    pub creator: String,
    pub creator_url: String,
}

#[derive(Clone)]
pub struct StockMediaClient {
    client: reqwest::Client,
    unsplash_key: Option<String>,
    pixabay_key: Option<String>,
}

impl StockMediaClient {
    pub fn from_properties(path: &Path) -> AtlasResult<Self> {
        let properties = std::fs::read_to_string(path).map_err(|error| {
            AtlasError::Config(format!("Unable to read {}: {error}", path.display()))
        })?;
        let values = parse_properties(&properties);
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .unwrap_or_default(),
            unsplash_key: values
                .get("unsplash.key")
                .filter(|v| !v.is_empty())
                .cloned(),
            pixabay_key: values.get("pixabay.key").filter(|v| !v.is_empty()).cloned(),
        })
    }

    pub async fn import(
        &self,
        query: &str,
        kind: MediaKind,
        orientation: MediaOrientation,
        output_dir: &Path,
    ) -> AtlasResult<ImportedMedia> {
        std::fs::create_dir_all(output_dir)?;
        match kind {
            MediaKind::Video => {
                self.import_pixabay_video(query, orientation, output_dir)
                    .await
            }
            MediaKind::Image if self.unsplash_key.is_some() => {
                self.import_unsplash_image(query, orientation, output_dir)
                    .await
            }
            MediaKind::Image => {
                self.import_pixabay_image(query, orientation, output_dir)
                    .await
            }
        }
    }

    async fn import_pixabay_video(
        &self,
        query: &str,
        orientation: MediaOrientation,
        output_dir: &Path,
    ) -> AtlasResult<ImportedMedia> {
        let key = self.pixabay_key.as_deref().ok_or_else(|| {
            AtlasError::Config("Pixabay key is missing from the configured properties file".into())
        })?;
        let response: PixabayVideoResponse = self
            .client
            .get("https://pixabay.com/api/videos/")
            .query(&[
                ("key", key),
                ("q", query),
                ("orientation", orientation_value(orientation)),
                ("safesearch", "true"),
                ("per_page", "10"),
            ])
            .send()
            .await
            .map_err(media_error)?
            .error_for_status()
            .map_err(media_error)?
            .json()
            .await
            .map_err(media_error)?;
        let item = response
            .hits
            .into_iter()
            .find(|item| item.duration >= 4 && item.duration <= 30)
            .ok_or_else(|| {
                AtlasError::Config(format!("Pixabay found no usable video for {query}"))
            })?;
        let source = item.videos.medium.or(item.videos.large).ok_or_else(|| {
            AtlasError::Config(format!(
                "Pixabay returned no downloadable video for {query}"
            ))
        })?;
        let file_path = output_dir.join(format!("pixabay_{}.mp4", item.id));
        download(&self.client, &source.url, &file_path).await?;
        Ok(ImportedMedia {
            provider: "Pixabay".into(),
            provider_id: item.id.to_string(),
            kind: MediaKind::Video,
            file_path,
            source_url: item.page_url,
            creator: item.user,
            creator_url: item.user_url,
        })
    }

    async fn import_pixabay_image(
        &self,
        query: &str,
        orientation: MediaOrientation,
        output_dir: &Path,
    ) -> AtlasResult<ImportedMedia> {
        let key = self.pixabay_key.as_deref().ok_or_else(|| {
            AtlasError::Config("Pixabay key is missing from the configured properties file".into())
        })?;
        let response: PixabayImageResponse = self
            .client
            .get("https://pixabay.com/api/")
            .query(&[
                ("key", key),
                ("q", query),
                ("orientation", orientation_value(orientation)),
                ("safesearch", "true"),
                ("per_page", "10"),
                ("image_type", "photo"),
            ])
            .send()
            .await
            .map_err(media_error)?
            .error_for_status()
            .map_err(media_error)?
            .json()
            .await
            .map_err(media_error)?;
        let item = response
            .hits
            .into_iter()
            .next()
            .ok_or_else(|| AtlasError::Config(format!("Pixabay found no image for {query}")))?;
        let file_path = output_dir.join(format!("pixabay_{}.jpg", item.id));
        download(&self.client, &item.large_image_url, &file_path).await?;
        Ok(ImportedMedia {
            provider: "Pixabay".into(),
            provider_id: item.id.to_string(),
            kind: MediaKind::Image,
            file_path,
            source_url: item.page_url,
            creator: item.user,
            creator_url: item.user_url,
        })
    }

    async fn import_unsplash_image(
        &self,
        query: &str,
        orientation: MediaOrientation,
        output_dir: &Path,
    ) -> AtlasResult<ImportedMedia> {
        let key = self.unsplash_key.as_deref().ok_or_else(|| {
            AtlasError::Config("Unsplash key is missing from the configured properties file".into())
        })?;
        let response: UnsplashResponse = self
            .client
            .get("https://api.unsplash.com/search/photos")
            .header("Authorization", format!("Client-ID {key}"))
            .query(&[
                ("query", query),
                ("orientation", unsplash_orientation_value(orientation)),
                ("content_filter", "high"),
                ("per_page", "10"),
            ])
            .send()
            .await
            .map_err(media_error)?
            .error_for_status()
            .map_err(media_error)?
            .json()
            .await
            .map_err(media_error)?;
        let item =
            response.results.into_iter().next().ok_or_else(|| {
                AtlasError::Config(format!("Unsplash found no image for {query}"))
            })?;
        let download_location = format!("{}?client_id={}", item.links.download_location, key);
        let tracked: UnsplashDownload = self
            .client
            .get(download_location)
            .send()
            .await
            .map_err(media_error)?
            .error_for_status()
            .map_err(media_error)?
            .json()
            .await
            .map_err(media_error)?;
        let file_path = output_dir.join(format!("unsplash_{}.jpg", item.id));
        download(&self.client, &tracked.url, &file_path).await?;
        Ok(ImportedMedia {
            provider: "Unsplash".into(),
            provider_id: item.id,
            kind: MediaKind::Image,
            file_path,
            source_url: item.links.html,
            creator: item.user.name,
            creator_url: item.user.links.html,
        })
    }
}

fn parse_properties(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

fn orientation_value(orientation: MediaOrientation) -> &'static str {
    match orientation {
        MediaOrientation::Landscape => "horizontal",
        MediaOrientation::Portrait => "vertical",
    }
}

fn unsplash_orientation_value(orientation: MediaOrientation) -> &'static str {
    match orientation {
        MediaOrientation::Landscape => "landscape",
        MediaOrientation::Portrait => "portrait",
    }
}

async fn download(client: &reqwest::Client, url: &str, path: &Path) -> AtlasResult<()> {
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(media_error)?
        .error_for_status()
        .map_err(media_error)?
        .bytes()
        .await
        .map_err(media_error)?;
    tokio::fs::write(path, bytes).await?;
    Ok(())
}

fn media_error(error: reqwest::Error) -> AtlasError {
    AtlasError::Config(format!("Stock media request failed: {error}"))
}

#[derive(Deserialize)]
struct PixabayVideoResponse {
    hits: Vec<PixabayVideo>,
}

#[derive(Deserialize)]
struct PixabayVideo {
    id: u64,
    duration: u32,
    #[serde(rename = "pageURL")]
    page_url: String,
    user: String,
    #[serde(rename = "userImageURL")]
    user_url: String,
    videos: PixabayVideos,
}

#[derive(Deserialize)]
struct PixabayVideos {
    medium: Option<PixabayVideoFile>,
    large: Option<PixabayVideoFile>,
}

#[derive(Deserialize)]
struct PixabayVideoFile {
    url: String,
}

#[derive(Deserialize)]
struct PixabayImageResponse {
    hits: Vec<PixabayImage>,
}

#[derive(Deserialize)]
struct PixabayImage {
    id: u64,
    #[serde(rename = "pageURL")]
    page_url: String,
    #[serde(rename = "largeImageURL")]
    large_image_url: String,
    user: String,
    #[serde(rename = "userImageURL")]
    user_url: String,
}

#[derive(Deserialize)]
struct UnsplashResponse {
    results: Vec<UnsplashImage>,
}

#[derive(Deserialize)]
struct UnsplashImage {
    id: String,
    links: UnsplashLinks,
    user: UnsplashUser,
}

#[derive(Deserialize)]
struct UnsplashLinks {
    html: String,
    download_location: String,
}

#[derive(Deserialize)]
struct UnsplashUser {
    name: String,
    links: UnsplashUserLinks,
}

#[derive(Deserialize)]
struct UnsplashUserLinks {
    html: String,
}

#[derive(Deserialize)]
struct UnsplashDownload {
    url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_provider_keys_without_exposing_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.properties");
        std::fs::write(&path, "unsplash.key=alpha\npixabay.key=beta\n").unwrap();
        let client = StockMediaClient::from_properties(&path).unwrap();
        assert!(client.unsplash_key.is_some());
        assert!(client.pixabay_key.is_some());
    }
}
