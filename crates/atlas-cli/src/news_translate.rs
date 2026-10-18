//! Translation of non-English wire entries (Japanese newsrooms and channels, ...) with the
//! director, before stories are grouped. The original headline and summary stay on the entry
//! (`original_title`, `original_summary`) and the article body is never translated, so
//! attribution and fact checks keep running on the original text. A translation only makes an
//! entry groupable with English coverage: a translated story still needs a real second source.

use atlas_media::news_wire::FeedEntry;
use atlas_storyboard::DirectorClient;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

const BATCH: usize = 15;
const CACHE_FILE: &str = "translations.json";

#[derive(Deserialize)]
struct Translated {
    i: usize,
    title: String,
    #[serde(default)]
    summary: String,
}

#[derive(Deserialize)]
struct Reply {
    items: Vec<Translated>,
}

fn prompt(batch: &[&FeedEntry]) -> String {
    let items: Vec<serde_json::Value> = batch
        .iter()
        .enumerate()
        .map(|(i, e)| serde_json::json!({ "i": i, "language": e.language, "title": e.title, "summary": e.summary.chars().take(600).collect::<String>() }))
        .collect();
    format!(
        "Translate these animation news headlines and summaries into natural English for a news broadcast.\n\
         Rules:\n\
         - Translate faithfully. Add nothing, drop nothing, never guess facts.\n\
         - Names must be exactly right: use a title's official English name only when the text itself gives it; \
         otherwise romanize it faithfully (Hepburn for Japanese, Revised Romanization for Korean, pinyin for Chinese). \
         Keep people's names in their usual order and spelling, with accents and macrons (Ōtomo, Pokémon).\n\
         - Keep studio and company names as the companies write them in English (MAPPA, Toei Animation, Studio Ghibli).\n\
         - Keep numbers, dates and episode counts exactly.\n\
         Items:\n{}\n\n\
         Return JSON: {{\"items\": [{{\"i\": number, \"title\": \"English headline\", \"summary\": \"English summary\"}}]}}",
        serde_json::to_string_pretty(&items).unwrap_or_default()
    )
}

fn load_cache(news_dir: &Path) -> HashMap<String, (String, String)> {
    std::fs::read_to_string(news_dir.join(CACHE_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_cache(news_dir: &Path, cache: &HashMap<String, (String, String)>) {
    let _ = std::fs::create_dir_all(news_dir);
    if let Ok(text) = serde_json::to_string(cache) {
        let _ = std::fs::write(news_dir.join(CACHE_FILE), text);
    }
}

/// Applies a translation, keeping the original wording on the entry.
pub(crate) fn apply(entry: &mut FeedEntry, title: &str, summary: &str) {
    if entry.original_title.is_none() {
        entry.original_title = Some(std::mem::take(&mut entry.title));
        entry.original_summary = Some(std::mem::take(&mut entry.summary));
    }
    entry.title = title.trim().to_string();
    entry.summary = summary.trim().to_string();
}

fn needs_translation(entry: &FeedEntry, cutoff: DateTime<Utc>) -> bool {
    !entry.language.is_empty() && entry.language != "en" && entry.published.is_none_or(|d| d >= cutoff)
}

/// Translates every fresh non-English entry; entries that cannot be translated (no director,
/// or a failed reply) are dropped rather than aired untranslated. Returns (translated, dropped).
pub(crate) async fn translate_entries(entries: &mut Vec<FeedEntry>, news_dir: &Path, max_age_days: i64) -> (usize, usize) {
    let cutoff = Utc::now() - chrono::Duration::days(max_age_days);
    let mut cache = load_cache(news_dir);
    let mut pending: Vec<usize> = Vec::new();
    let mut translated = 0;
    for (index, entry) in entries.iter_mut().enumerate() {
        if !needs_translation(entry, cutoff) {
            continue;
        }
        if let Some((title, summary)) = cache.get(&entry.link) {
            apply(entry, title, summary);
            translated += 1;
        } else {
            pending.push(index);
        }
    }
    if !pending.is_empty() && let Some(director) = DirectorClient::from_env() {
        for chunk in pending.chunks(BATCH) {
            let batch: Vec<&FeedEntry> = chunk.iter().map(|&i| &entries[i]).collect();
            let reply = director
                .complete_json("You translate news text faithfully into English. Return valid JSON only.", &prompt(&batch), 0.2)
                .await
                .ok()
                .and_then(|v| serde_json::from_value::<Reply>(v).ok());
            for item in reply.map(|r| r.items).unwrap_or_default() {
                let Some(&index) = chunk.get(item.i) else { continue };
                if item.title.trim().is_empty() {
                    continue;
                }
                let entry = &mut entries[index];
                cache.insert(entry.link.clone(), (item.title.clone(), item.summary.clone()));
                apply(entry, &item.title, &item.summary);
                translated += 1;
            }
        }
        save_cache(news_dir, &cache);
    }
    let before = entries.len();
    entries.retain(|e| !needs_translation(e, cutoff) || e.original_title.is_some());
    (translated, before - entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_keeps_the_original_headline_and_body() {
        let mut entry = FeedEntry {
            source: "MAPPA".into(),
            language: "ja".into(),
            title: "「チェンソーマン」新作発表".into(),
            summary: "MAPPAが制作する。".into(),
            body: "MAPPAが制作する。".into(),
            link: "https://www.mappa.co.jp/news/1".into(),
            ..FeedEntry::default()
        };
        apply(&mut entry, "New Chainsaw Man project announced", "MAPPA will produce it.");
        apply(&mut entry, "New Chainsaw Man Project Announced", "MAPPA will produce it.");
        assert_eq!(entry.original_title.as_deref(), Some("「チェンソーマン」新作発表"));
        assert_eq!(entry.title, "New Chainsaw Man Project Announced");
        assert_eq!(entry.body, "MAPPAが制作する。");
        assert_eq!(entry.link, "https://www.mappa.co.jp/news/1");
    }

    #[test]
    fn only_fresh_non_english_entries_need_translation() {
        let now = Utc::now();
        let entry = |language: &str, days: i64| FeedEntry { language: language.into(), published: Some(now - chrono::Duration::days(days)), ..FeedEntry::default() };
        let cutoff = now - chrono::Duration::days(7);
        assert!(needs_translation(&entry("ja", 1), cutoff));
        assert!(!needs_translation(&entry("en", 1), cutoff));
        assert!(!needs_translation(&entry("ja", 30), cutoff));
    }

    #[test]
    fn prompt_asks_for_exact_names_and_accents() {
        let e = FeedEntry { language: "ja".into(), title: "新作".into(), ..FeedEntry::default() };
        let text = prompt(&[&e]);
        assert!(text.contains("accents and macrons"));
        assert!(text.contains("Add nothing"));
    }
}
