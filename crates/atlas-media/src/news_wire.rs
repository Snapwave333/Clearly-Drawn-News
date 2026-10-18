use crate::news_desk::{AnimationCategory, VerifiedNewsItem};
use chrono::{DateTime, Utc};
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub struct FeedSource {
    pub name: &'static str,
    pub url: &'static str,
    pub default_category: AnimationCategory,
}

pub const FEEDS: [FeedSource; 4] = [
    FeedSource {
        name: "Anime News Network",
        url: "https://www.animenewsnetwork.com/all/rss.xml",
        default_category: AnimationCategory::Anime,
    },
    FeedSource {
        name: "Cartoon Brew",
        url: "https://www.cartoonbrew.com/feed",
        default_category: AnimationCategory::Cartoons,
    },
    FeedSource {
        name: "Animation Magazine",
        url: "https://www.animationmagazine.net/feed/",
        default_category: AnimationCategory::Industry,
    },
    FeedSource {
        name: "Crunchyroll News",
        url: "https://www.crunchyroll.com/news/rss",
        default_category: AnimationCategory::Anime,
    },
];

const KNOWN_ORGANIZATIONS: [&str; 44] = [
    "MAPPA", "Toei Animation", "Studio Ghibli", "Kyoto Animation", "ufotable", "Science SARU",
    "Production I.G", "Wit Studio", "CloverWorks", "A-1 Pictures", "Bones", "Sunrise", "Trigger",
    "TMS Entertainment", "Madhouse", "Shaft", "David Production", "Aniplex", "Crunchyroll",
    "Shueisha", "Kodansha", "Pixar", "Walt Disney Animation", "Disney", "DreamWorks",
    "Sony Pictures Animation", "Illumination", "Warner Bros", "Cartoon Network", "Adult Swim",
    "Nickelodeon", "Netflix", "Amazon", "Prime Video", "HBO", "Max", "Fortiche", "Riot Games",
    "Aardman", "Laika", "Cartoon Saloon", "Titmouse", "Annecy", "The Animation Guild",
];

const RUMOR_WORDS: [&str; 9] = [
    "rumor", "rumour", "leak", "leaked", "reportedly", "unconfirmed", "insider", "allegedly",
    "speculation",
];

const STOPWORDS: [&str; 24] = [
    "the", "a", "an", "and", "of", "to", "in", "for", "on", "with", "at", "by", "from", "is",
    "its", "as", "new", "season", "anime", "film", "movie", "series", "gets", "reveals",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedEntry {
    pub source: String,
    pub title: String,
    pub link: String,
    pub published: Option<DateTime<Utc>>,
    pub summary: String,
    pub default_category: AnimationCategory,
}

pub fn parse_feed(xml: &str, source: &FeedSource) -> Vec<FeedEntry> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut entries = Vec::new();
    let mut in_item = false;
    let mut field: Option<String> = None;
    let mut current = blank_entry(source);
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                let name = local_name(tag.name().as_ref());
                if name == "item" || name == "entry" {
                    in_item = true;
                    current = blank_entry(source);
                } else if in_item {
                    if name == "link" {
                        if let Some(href) = tag
                            .attributes()
                            .filter_map(Result::ok)
                            .find(|a| a.key.as_ref() == b"href")
                        {
                            current.link = String::from_utf8_lossy(&href.value).into_owned();
                        }
                    }
                    field = Some(name);
                }
            }
            Ok(Event::Empty(tag)) if in_item => {
                if local_name(tag.name().as_ref()) == "link" && current.link.is_empty() {
                    if let Some(href) = tag
                        .attributes()
                        .filter_map(Result::ok)
                        .find(|a| a.key.as_ref() == b"href")
                    {
                        current.link = String::from_utf8_lossy(&href.value).into_owned();
                    }
                }
            }
            Ok(Event::Text(text)) if in_item => {
                let value = text.unescape().map(|v| v.into_owned()).unwrap_or_default();
                apply_field(&mut current, field.as_deref(), &value);
            }
            Ok(Event::CData(data)) if in_item => {
                let value = String::from_utf8_lossy(&data.into_inner()).into_owned();
                apply_field(&mut current, field.as_deref(), &value);
            }
            Ok(Event::End(tag)) => {
                let name = local_name(tag.name().as_ref());
                if name == "item" || name == "entry" {
                    in_item = false;
                    if !current.title.is_empty() && !current.link.is_empty() {
                        current.summary = clean_summary(&current.summary);
                        entries.push(current.clone());
                    }
                }
                field = None;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    entries
}

fn blank_entry(source: &FeedSource) -> FeedEntry {
    FeedEntry {
        source: source.name.to_string(),
        title: String::new(),
        link: String::new(),
        published: None,
        summary: String::new(),
        default_category: source.default_category,
    }
}

fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit(':').next().unwrap_or_default().to_ascii_lowercase()
}

fn apply_field(entry: &mut FeedEntry, field: Option<&str>, value: &str) {
    match field {
        Some("title") => entry.title.push_str(value.trim()),
        Some("link") if entry.link.is_empty() => entry.link = value.trim().to_string(),
        Some("pubdate") | Some("published") | Some("updated") if entry.published.is_none() => {
            entry.published = DateTime::parse_from_rfc2822(value.trim())
                .or_else(|_| DateTime::parse_from_rfc3339(value.trim()))
                .ok()
                .map(|date| date.with_timezone(&Utc));
        }
        Some("description") | Some("summary") | Some("content") if entry.summary.is_empty() => {
            entry.summary = value.to_string();
        }
        _ => {}
    }
}

fn clean_summary(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let text = text
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#8217;", "'")
        .replace("&nbsp;", " ");
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= 400 {
        return collapsed;
    }
    let cut = collapsed.chars().take(400).collect::<String>();
    match cut.rfind(". ") {
        Some(end) if end > 120 => cut[..=end].to_string(),
        _ => format!("{}…", cut.trim_end()),
    }
}

fn title_tokens(title: &str) -> HashSet<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2 && !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect()
}

fn same_story(a: &HashSet<String>, b: &HashSet<String>) -> bool {
    let shared = a.intersection(b).count();
    let smaller = a.len().min(b.len());
    smaller > 0 && shared >= 2 && shared * 2 >= smaller
}

fn organization(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    KNOWN_ORGANIZATIONS.iter().copied().find(|org| {
        let needle = org.to_lowercase();
        lower.match_indices(&needle).any(|(i, _)| {
            let before = lower[..i].chars().next_back();
            let after = lower[i + needle.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

fn categorize(text: &str, fallback: AnimationCategory) -> AnimationCategory {
    let lower = text.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["adult swim", "adult animation", "mature audiences", "invincible", "arcane"]) {
        AnimationCategory::AdultAnimation
    } else if has(&["box office", "layoff", "union", "acquisition", "acquire", "award", "festival", "earnings", "studio closes"]) {
        AnimationCategory::Industry
    } else if has(&["trailer", "premiere", "release date", "teaser", "announced for", "coming to", "debuts"]) {
        AnimationCategory::UpcomingReleases
    } else if has(&["anime", "manga", "crunchyroll", "isekai"]) {
        AnimationCategory::Anime
    } else {
        fallback
    }
}

fn editorial_angle(category: AnimationCategory) -> &'static str {
    match category {
        AnimationCategory::Anime => "The Enthusiast celebrates the craft; The Counterpart checks production health and schedule realism.",
        AnimationCategory::Cartoons => "The Enthusiast spotlights the artistry; The Counterpart asks who the audience really is.",
        AnimationCategory::AdultAnimation => "The Enthusiast hypes the tone; The Counterpart weighs whether the risk pays off.",
        AnimationCategory::Industry => "The Counterpart leads on the business facts; The Enthusiast explains what it means for artists.",
        AnimationCategory::UpcomingReleases => "The Enthusiast breaks down the footage; The Counterpart keeps expectations grounded.",
    }
}

fn story_id(link: &str) -> String {
    let hash = link
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| (acc ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    format!("wire_{hash:012x}")
}

pub fn build_wire(
    mut entries: Vec<FeedEntry>,
    now: DateTime<Utc>,
    max_age_days: i64,
    max_stories: usize,
) -> Vec<VerifiedNewsItem> {
    let cutoff = now - chrono::Duration::days(max_age_days);
    entries.retain(|entry| entry.published.is_none_or(|date| date >= cutoff && date <= now + chrono::Duration::hours(6)));
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.published));

    let mut groups: Vec<(HashSet<String>, Vec<FeedEntry>)> = Vec::new();
    for entry in entries {
        let tokens = title_tokens(&entry.title);
        match groups.iter_mut().find(|(group_tokens, _)| same_story(group_tokens, &tokens)) {
            Some((_, members)) => members.push(entry),
            None => groups.push((tokens, vec![entry])),
        }
    }

    let mut stories = groups
        .into_iter()
        .map(|(_, mut members)| {
            members.sort_by_key(|member| member.published);
            let primary = members[0].clone();
            let secondary = members
                .iter()
                .find(|other| other.source != primary.source)
                .map(|other| other.link.clone());
            let text = members
                .iter()
                .map(|m| format!("{} {} {}", m.title, m.summary, m.link))
                .collect::<Vec<_>>()
                .join(" ");
            let lower = text.to_lowercase();
            let category = categorize(&text, primary.default_category);
            let corroborated = secondary.is_some();
            VerifiedNewsItem {
                id: story_id(&primary.link),
                title: primary.title.clone(),
                category,
                primary_source_url: primary.link.clone(),
                secondary_source_url: secondary,
                announcement_date: primary
                    .published
                    .unwrap_or(now)
                    .format("%Y-%m-%d")
                    .to_string(),
                studio_or_organization: organization(&text).unwrap_or_default().to_string(),
                factual_summary: if primary.summary.is_empty() {
                    format!("Reported by {}.", primary.source)
                } else {
                    format!("{} (Source: {})", primary.summary, primary.source)
                },
                editorial_angle: editorial_angle(category).to_string(),
                is_verified: true,
                is_rumor: RUMOR_WORDS.iter().any(|w| {
                    lower.split(|c: char| !c.is_alphanumeric()).any(|t| t == *w)
                }),
                estimated_duration_mins: if corroborated { 3.5 } else { 2.5 },
            }
        })
        .collect::<Vec<_>>();
    stories.sort_by_key(|story| std::cmp::Reverse(story.secondary_source_url.is_some()));
    stories.truncate(max_stories);
    stories
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedStatus {
    pub source: String,
    pub entries: usize,
    pub error: Option<String>,
    #[serde(default)]
    pub skipped: bool,
}

pub async fn fetch_feeds(client: &reqwest::Client, skip: &[String]) -> (Vec<FeedEntry>, Vec<FeedStatus>) {
    let mut tasks = tokio::task::JoinSet::new();
    let mut statuses = Vec::new();
    for source in FEEDS {
        if skip.iter().any(|name| name == source.name) {
            statuses.push(FeedStatus { source: source.name.into(), entries: 0, error: None, skipped: true });
            continue;
        }
        let client = client.clone();
        tasks.spawn(async move { (source.name, fetch_feed(&client, &source).await) });
    }
    let mut entries = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok((source, result)) = joined else {
            continue;
        };
        match result {
            Ok(mut items) => {
                statuses.push(FeedStatus { source: source.into(), entries: items.len(), error: None, skipped: false });
                entries.append(&mut items);
            }
            Err(error) => statuses.push(FeedStatus { source: source.into(), entries: 0, error: Some(error), skipped: false }),
        }
    }
    statuses.sort_by(|a, b| a.source.cmp(&b.source));
    (entries, statuses)
}

async fn fetch_feed(client: &reqwest::Client, source: &FeedSource) -> Result<Vec<FeedEntry>, String> {
    let describe = |e: reqwest::Error| format!("{}: {}", source.name, e.without_url());
    let body = client
        .get(source.url)
        .header("User-Agent", "ClearlyDrawnNews/1.0")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(describe)?
        .text()
        .await
        .map_err(describe)?;
    Ok(parse_feed(&body, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>ANN</title>
<item><title>MAPPA Confirms Chainsaw Man Season 2 Production</title>
<link>https://www.animenewsnetwork.com/news/2026-10-15/csm</link>
<pubDate>Thu, 15 Oct 2026 10:00:00 GMT</pubDate>
<description><![CDATA[<p>MAPPA confirmed <b>production</b> of the sequel &amp; a new staff list.</p>]]></description></item>
<item><title>Rumor: Unannounced Isekai Leaked by Insider</title>
<link>https://www.animenewsnetwork.com/news/2026-10-16/leak</link>
<pubDate>Fri, 16 Oct 2026 08:00:00 GMT</pubDate>
<description>An insider leaked a project from Toei Animation.</description></item>
<item><title>Old News From Last Year</title>
<link>https://www.animenewsnetwork.com/news/2025-01-01/old</link>
<pubDate>Wed, 01 Jan 2025 08:00:00 GMT</pubDate>
<description>Pixar history.</description></item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<entry><title>Chainsaw Man Season 2 Production Confirmed at MAPPA</title>
<link href="https://www.cartoonbrew.com/anime/csm-season-2"/>
<updated>2026-10-15T14:00:00Z</updated>
<summary>Studio MAPPA moves ahead on the sequel.</summary></entry>
<entry><title>Sony Pictures Animation Releases Beyond the Spider-Verse Trailer</title>
<link href="https://www.cartoonbrew.com/feature-film/spider-verse"/>
<updated>2026-10-16T09:00:00Z</updated>
<summary>The first trailer debuts.</summary></entry>
</feed>"#;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-17T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_rss_and_atom_entries() {
        let rss = parse_feed(RSS, &FEEDS[0]);
        assert_eq!(rss.len(), 3);
        assert_eq!(rss[0].summary, "MAPPA confirmed production of the sequel & a new staff list.");
        assert!(rss[0].published.is_some());
        let atom = parse_feed(ATOM, &FEEDS[1]);
        assert_eq!(atom.len(), 2);
        assert_eq!(atom[0].link, "https://www.cartoonbrew.com/anime/csm-season-2");
        assert_eq!(atom[1].source, "Cartoon Brew");
    }

    #[test]
    fn builds_corroborated_fresh_wire() {
        let mut entries = parse_feed(RSS, &FEEDS[0]);
        entries.extend(parse_feed(ATOM, &FEEDS[1]));
        let wire = build_wire(entries, now(), 7, 8);
        assert_eq!(wire.len(), 3);

        let csm = &wire[0];
        assert!(csm.title.contains("Chainsaw Man"));
        assert!(csm.secondary_source_url.is_some());
        assert_eq!(csm.studio_or_organization, "MAPPA");
        assert_eq!(csm.category, AnimationCategory::Anime);

        let leak = wire.iter().find(|s| s.title.starts_with("Rumor")).unwrap();
        assert!(leak.is_rumor);

        let spider = wire.iter().find(|s| s.title.contains("Spider-Verse")).unwrap();
        assert_eq!(spider.studio_or_organization, "Sony Pictures Animation");
        assert_eq!(spider.category, AnimationCategory::UpcomingReleases);
        assert!(wire.iter().all(|s| !s.title.contains("Old News")));
    }

    #[test]
    fn organization_matching_respects_word_boundaries() {
        assert_eq!(organization("Maximum effort from Netflix"), Some("Netflix"));
        assert_eq!(organization("A maximum budget"), None);
    }
}
