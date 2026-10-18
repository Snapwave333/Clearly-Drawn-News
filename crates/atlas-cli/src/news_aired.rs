use super::*;
use atlas_media::fact_checker::FactCheckReport;
use atlas_media::news_desk::VerifiedNewsItem;

const AIR_MEMORY_DAYS: i64 = 7;
const MAX_REMEMBERED: usize = 2000;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AiredStory {
    id: String,
    url: String,
    title_key: String,
    episode: String,
    aired_at: chrono::DateTime<Utc>,
}

#[derive(Default, Serialize, Deserialize)]
struct AiredLog {
    #[serde(default)]
    stories: Vec<AiredStory>,
}

fn log_path(news_dir: &Path) -> PathBuf {
    news_dir.join("aired_stories.json")
}

fn title_key(title: &str) -> String {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn load(news_dir: &Path) -> AiredLog {
    std::fs::read(log_path(news_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn aired_in<'a>(log: &'a AiredLog, story: &VerifiedNewsItem, now: chrono::DateTime<Utc>) -> Option<&'a AiredStory> {
    let cutoff = now - chrono::Duration::days(AIR_MEMORY_DAYS);
    let key = title_key(&story.title);
    log.stories.iter().rev().find(|aired| {
        aired.aired_at >= cutoff
            && (aired.id == story.id || aired.url == story.primary_source_url || aired.title_key == key)
    })
}

pub(crate) fn drop_already_aired(news_dir: &Path, report: &mut FactCheckReport, now: chrono::DateTime<Utc>) -> usize {
    let log = load(news_dir);
    let mut fresh = Vec::with_capacity(report.passed_stories.len());
    let mut dropped = 0;
    for story in std::mem::take(&mut report.passed_stories) {
        match aired_in(&log, &story, now) {
            Some(aired) => {
                let reason = format!("Already aired in {} on {}", aired.episode, aired.aired_at.format("%Y-%m-%d %H:%M UTC"));
                report.rejected_stories.push((story, reason));
                dropped += 1;
            }
            None => fresh.push(story),
        }
    }
    report.estimated_total_minutes = fresh.iter().map(|s| s.estimated_duration_mins).sum::<f64>() + 3.0;
    report.passed_stories = fresh;
    dropped
}

pub(crate) fn record_aired(news_dir: &Path, episode_id: &str, stories: &[VerifiedNewsItem], now: chrono::DateTime<Utc>) -> anyhow::Result<()> {
    let mut log = load(news_dir);
    log.stories.extend(stories.iter().map(|story| AiredStory {
        id: story.id.clone(),
        url: story.primary_source_url.clone(),
        title_key: title_key(&story.title),
        episode: episode_id.to_string(),
        aired_at: now,
    }));
    if log.stories.len() > MAX_REMEMBERED {
        let excess = log.stories.len() - MAX_REMEMBERED;
        log.stories.drain(..excess);
    }
    std::fs::create_dir_all(news_dir)?;
    std::fs::write(log_path(news_dir), serde_json::to_vec_pretty(&log)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_media::news_desk::AnimationCategory;

    fn story(id: &str, title: &str, url: &str) -> VerifiedNewsItem {
        VerifiedNewsItem {
            id: id.into(),
            title: title.into(),
            category: AnimationCategory::Anime,
            primary_source_url: url.into(),
            secondary_source_url: None,
            announcement_date: "2026-10-17".into(),
            studio_or_organization: "MAPPA".into(),
            factual_summary: "Facts.".into(),
            editorial_angle: "Angle.".into(),
            is_verified: true,
            is_rumor: false,
            estimated_duration_mins: 3.0,
        }
    }

    fn report(stories: Vec<VerifiedNewsItem>) -> FactCheckReport {
        FactCheckReport { total_candidates: stories.len(), passed_stories: stories, rejected_stories: Vec::new(), estimated_total_minutes: 0.0 }
    }

    #[test]
    fn aired_stories_are_dropped_for_a_week() {
        let dir = std::env::temp_dir().join(format!("atlas_aired_{}", uuid::Uuid::new_v4().simple()));
        let now = Utc::now();
        record_aired(&dir, "EP-001", &[story("a", "MAPPA Confirms Season 2!", "https://ann/a")], now - chrono::Duration::hours(6)).unwrap();
        record_aired(&dir, "EP-000", &[story("old", "Old Story", "https://ann/old")], now - chrono::Duration::days(9)).unwrap();

        let mut fresh = report(vec![
            story("a", "Different headline", "https://other/x"),
            story("b", "mappa confirms season 2", "https://brew/b"),
            story("c", "New Toei Film", "https://ann/c"),
            story("d", "Something", "https://ann/a"),
            story("old", "Old Story", "https://ann/old"),
        ]);
        assert_eq!(drop_already_aired(&dir, &mut fresh, now), 3);
        let kept = fresh.passed_stories.iter().map(|s| s.id.as_str()).collect::<Vec<_>>();
        assert_eq!(kept, ["c", "old"]);
        assert!(fresh.rejected_stories[0].1.starts_with("Already aired in EP-001"));
        assert_eq!(fresh.estimated_total_minutes, 9.0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
