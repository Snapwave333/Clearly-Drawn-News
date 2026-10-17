use super::*;

pub(crate) async fn import_broll(
    video_id: &str,
    format: VideoFormat,
    data_dir: &Path,
    queries: &[String],
) -> Vec<ImportedMedia> {
    let output_dir = data_dir
        .join("media")
        .join(video_id)
        .join(format.profile().slug);
    if let Some(cached) = cached_broll(&output_dir) {
        println!("   ✓ Reusing {} cached B-roll assets", cached.len());
        return cached;
    }
    let Some(base_dirs) = BaseDirs::new() else {
        eprintln!("B-roll skipped: unable to locate the user profile");
        return Vec::new();
    };
    let key_path = std::env::var_os("ATLAS_MEDIA_KEYS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            base_dirs
                .home_dir()
                .join("Desktop")
                .join("app wallpaper switcher")
                .join("local.properties")
        });
    let client = match StockMediaClient::from_properties(&key_path) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("B-roll skipped: {error}");
            return Vec::new();
        }
    };
    let orientation = match format {
        VideoFormat::Long => MediaOrientation::Landscape,
        VideoFormat::Short => MediaOrientation::Portrait,
    };
    let image_query = queries
        .first()
        .map(String::as_str)
        .unwrap_or("educational concept close up");
    let video_query = queries.get(1).map(String::as_str).unwrap_or(image_query);
    let (image, video) = tokio::join!(
        import_media_with_retry(&client, image_query, MediaKind::Image, orientation, &output_dir),
        import_media_with_retry(&client, video_query, MediaKind::Video, orientation, &output_dir),
    );
    let mut imported = Vec::new();
    for (query, result) in [(image_query, image), (video_query, video)] {
        match result {
            Ok(media) => imported.push(media),
            Err(error) => eprintln!("B-roll source unavailable for {query}: {error}"),
        }
    }
    if !imported.is_empty()
        && let Ok(json) = serde_json::to_vec_pretty(&imported)
    {
        let _ = std::fs::write(output_dir.join("attribution.json"), json);
    }
    imported
}

pub(crate) fn cached_broll(output_dir: &Path) -> Option<Vec<ImportedMedia>> {
    let bytes = std::fs::read(output_dir.join("attribution.json")).ok()?;
    let media = serde_json::from_slice::<Vec<ImportedMedia>>(&bytes).ok()?;
    let complete = media.len() == 2 && media.iter().all(|item| item.file_path.is_file());
    complete.then_some(media)
}

pub(crate) async fn import_media_with_retry(
    client: &StockMediaClient,
    query: &str,
    kind: MediaKind,
    orientation: MediaOrientation,
    output_dir: &Path,
) -> atlas_core::AtlasResult<ImportedMedia> {
    let mut attempt = 1;
    loop {
        match client.import(query, kind, orientation, output_dir).await {
            Ok(media) => return Ok(media),
            Err(error) if attempt < BROLL_IMPORT_ATTEMPTS => {
                let delay = std::time::Duration::from_secs(2_u64.pow(attempt));
                eprintln!(
                    "B-roll attempt {attempt}/{BROLL_IMPORT_ATTEMPTS} for {query} failed ({error}); retrying in {}s",
                    delay.as_secs()
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) const BROLL_IMPORT_ATTEMPTS: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_broll_requires_complete_package() {
        let dir = std::env::temp_dir().join(format!("atlas_broll_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(cached_broll(&dir).is_none());

        let media = |kind, name: &str| ImportedMedia {
            provider: "pixabay".into(),
            provider_id: name.into(),
            kind,
            file_path: dir.join(name),
            source_url: "https://pixabay.com".into(),
            creator: "creator".into(),
            creator_url: "https://pixabay.com/users/creator".into(),
        };
        let items = vec![media(MediaKind::Image, "a.jpg"), media(MediaKind::Video, "b.mp4")];
        std::fs::write(dir.join("attribution.json"), serde_json::to_vec(&items).unwrap()).unwrap();
        std::fs::write(dir.join("a.jpg"), b"img").unwrap();
        assert!(cached_broll(&dir).is_none());

        std::fs::write(dir.join("b.mp4"), b"vid").unwrap();
        assert_eq!(cached_broll(&dir).map(|cached| cached.len()), Some(2));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
