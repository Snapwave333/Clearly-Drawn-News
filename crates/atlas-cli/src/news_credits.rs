//! CC BY asset credits for every published description and the TikTok caption.
//!
//! CC BY 4.0 requires attribution (title, creator, source, license). The studio manifest's
//! `asset_credits` array is the source of truth; `CREDITS_CC_BY.md` in
//! `AtlasStudioOffice/assets/fab_cc_by/` is the fallback.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_LICENSE: &str = "CC BY 4.0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AssetCredit {
    pub(crate) title: String,
    pub(crate) creator: String,
    pub(crate) url: String,
    pub(crate) license: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn studio_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../AtlasStudioOffice")
}

pub(crate) fn manifest_path() -> PathBuf {
    env("ATLAS_STUDIO_MANIFEST").map(PathBuf::from).unwrap_or_else(|| studio_dir().join("studio_manifest.json"))
}

pub(crate) fn credits_file_path() -> PathBuf {
    env("ATLAS_ASSET_CREDITS")
        .map(PathBuf::from)
        .unwrap_or_else(|| studio_dir().join("assets/fab_cc_by/CREDITS_CC_BY.md"))
}

fn text_field(value: &serde_json::Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| value.get(*k).and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// `asset_credits` from the studio manifest. Accepts `title`/`name`, `creator`/`author`,
/// `url`/`source`, `license` (default CC BY 4.0). Entries without title, creator and URL are skipped.
pub(crate) fn parse_manifest_credits(manifest: &serde_json::Value) -> Vec<AssetCredit> {
    let Some(items) = manifest.get("asset_credits").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let credit = AssetCredit {
                title: text_field(item, &["title", "name", "asset"]),
                creator: text_field(item, &["creator", "author", "artist"]),
                url: text_field(item, &["url", "source", "source_url", "link"]),
                license: Some(text_field(item, &["license", "licence"])).filter(|l| !l.is_empty()).unwrap_or_else(|| DEFAULT_LICENSE.into()),
            };
            (!credit.title.is_empty() && !credit.creator.is_empty() && credit.url.starts_with("http")).then_some(credit)
        })
        .collect()
}

/// Lines like `Title, Creator, https://... (note)`. A title may itself contain commas; the creator
/// is the last comma field before the URL. Any trailing parenthetical note is dropped.
pub(crate) fn parse_credits_markdown(text: &str) -> Vec<AssetCredit> {
    let license = if text.contains("CC BY-SA 4.0") { "CC BY-SA 4.0" } else { DEFAULT_LICENSE };
    text.lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches(['-', '*', '•']).trim();
            if line.starts_with('#') {
                return None;
            }
            let at = line.find("https://").or_else(|| line.find("http://"))?;
            let url = line[at..].split_whitespace().next()?.trim_end_matches([',', ')', '.']).to_string();
            let head = line[..at].trim().trim_end_matches(',').trim();
            let (title, creator) = head.rsplit_once(',')?;
            let (title, creator) = (title.trim().to_string(), creator.trim().to_string());
            (!title.is_empty() && !creator.is_empty()).then(|| AssetCredit { title, creator, url, license: license.into() })
        })
        .collect()
}

/// Manifest first, then the credits file. Empty when neither has credits.
pub(crate) fn load_asset_credits() -> Vec<AssetCredit> {
    let from_manifest = std::fs::read_to_string(manifest_path())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t.trim_start_matches('\u{feff}')).ok())
        .map(|v| parse_manifest_credits(&v))
        .unwrap_or_default();
    if !from_manifest.is_empty() {
        return from_manifest;
    }
    std::fs::read_to_string(credits_file_path()).map(|t| parse_credits_markdown(&t)).unwrap_or_default()
}

/// YouTube block: one line per asset with title, creator, URL and license.
pub(crate) fn description_block(credits: &[AssetCredit]) -> String {
    if credits.is_empty() {
        return String::new();
    }
    let mut block = String::from("\nAsset credits (studio props used under Creative Commons):\n");
    for c in credits {
        block.push_str(&format!("• \"{}\" by {}, {} ({})\n", c.title, c.creator, c.url, c.license));
    }
    block
}

/// Compact TikTok form; same facts, fewer words.
pub(crate) fn compact_block(credits: &[AssetCredit]) -> String {
    if credits.is_empty() {
        return String::new();
    }
    let items = credits.iter().map(|c| format!("\"{}\" by {} {} ({})", c.title, c.creator, c.url, c.license)).collect::<Vec<_>>();
    format!("Asset credits: {}", items.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Fab props (CC BY 4.0), credit required in video descriptions/credits\n\
Studio Softbox Light, Glowbox3D, https://www.fab.com/listings/f75c95dd (NoAI: not for datasets/genAI inputs; render use only)\n\
Lockers (old school lockers), Normal Map Enjoyer, https://www.fab.com/listings/a4bb96b2\n\
Coffee Machine, Mazor, https://www.fab.com/listings/c6dfe564\n";

    #[test]
    fn credits_file_lines_parse_title_creator_url() {
        let credits = parse_credits_markdown(FILE);
        assert_eq!(credits.len(), 3);
        assert_eq!(credits[0].title, "Studio Softbox Light");
        assert_eq!(credits[0].creator, "Glowbox3D");
        assert_eq!(credits[0].url, "https://www.fab.com/listings/f75c95dd");
        assert_eq!(credits[1].title, "Lockers (old school lockers)");
        assert_eq!(credits[1].creator, "Normal Map Enjoyer");
        assert!(credits.iter().all(|c| c.license == "CC BY 4.0"));
    }

    #[test]
    fn manifest_credits_win_and_skip_incomplete_entries() {
        let manifest = serde_json::json!({"asset_credits": [
            {"title": "Frying Pan", "creator": "dimension Dazzle", "url": "https://www.fab.com/listings/4ab9", "license": "CC BY 4.0"},
            {"name": "Mug", "author": "Marcela Duarte Lopez", "source": "https://www.fab.com/listings/5410"},
            {"title": "No URL", "creator": "Someone"}
        ]});
        let credits = parse_manifest_credits(&manifest);
        assert_eq!(credits.len(), 2);
        assert_eq!(credits[1].license, DEFAULT_LICENSE);
        let block = description_block(&credits);
        assert!(block.contains("Asset credits"));
        assert!(block.contains("\"Frying Pan\" by dimension Dazzle, https://www.fab.com/listings/4ab9 (CC BY 4.0)"));
        assert!(description_block(&[]).is_empty());
    }
}
