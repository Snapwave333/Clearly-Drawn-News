use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationCategory {
    Anime,
    Cartoons,
    AdultAnimation,
    Industry,
    UpcomingReleases,
}

impl AnimationCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anime => "Anime",
            Self::Cartoons => "Western Cartoons",
            Self::AdultAnimation => "Adult Animation",
            Self::Industry => "Industry & Production",
            Self::UpcomingReleases => "Upcoming Releases",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsFeedSource {
    pub outlet: String,
    pub url: String,
    #[serde(default)]
    pub anime_focused: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireItem {
    pub id: String,
    pub outlet: String,
    pub title: String,
    pub url: String,
    pub published: Option<DateTime<Utc>>,
    pub summary: String,
    pub feed_categories: Vec<String>,
    pub anime_focused_outlet: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorySource {
    pub outlet: String,
    pub title: String,
    pub url: String,
    pub published: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsStory {
    pub id: String,
    pub title: String,
    pub category: AnimationCategory,
    pub studio_or_organization: String,
    pub summary: String,
    pub published: Option<DateTime<Utc>>,
    pub sources: Vec<StorySource>,
    #[serde(default)]
    pub article_text: String,
}

impl NewsStory {
    pub fn outlets(&self) -> Vec<String> {
        let mut outlets: Vec<String> = Vec::new();
        for source in &self.sources {
            if !outlets.contains(&source.outlet) {
                outlets.push(source.outlet.clone());
            }
        }
        outlets
    }

    pub fn corroboration(&self) -> usize {
        self.outlets().len()
    }

    pub fn primary_url(&self) -> &str {
        self.sources.first().map(|s| s.url.as_str()).unwrap_or_default()
    }

    pub fn fact_sheet(&self) -> String {
        let mut sheet = format!("Headline: {}\n", self.title);
        sheet.push_str(&format!("Category: {}\n", self.category.as_str()));
        if !self.studio_or_organization.is_empty() {
            sheet.push_str(&format!("Organization: {}\n", self.studio_or_organization));
        }
        if let Some(published) = self.published {
            sheet.push_str(&format!("Published: {}\n", published.format("%Y-%m-%d")));
        }
        for source in &self.sources {
            sheet.push_str(&format!("Reported by {}: {} ({})\n", source.outlet, source.title, source.url));
        }
        sheet.push_str(&format!("Summary: {}\n", self.summary));
        if !self.article_text.is_empty() {
            sheet.push_str(&format!("Article notes: {}\n", self.article_text));
        }
        sheet
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WireFetchReport {
    pub items: Vec<WireItem>,
    pub feed_errors: Vec<(String, String)>,
    pub feeds_ok: usize,
}

pub fn default_sources() -> Vec<NewsFeedSource> {
    vec![
        NewsFeedSource {
            outlet: "Anime News Network".into(),
            url: "https://www.animenewsnetwork.com/news/rss.xml?ann-edition=us".into(),
            anime_focused: true,
        },
        NewsFeedSource {
            outlet: "Cartoon Brew".into(),
            url: "https://www.cartoonbrew.com/feed".into(),
            anime_focused: false,
        },
        NewsFeedSource {
            outlet: "Animation Magazine".into(),
            url: "https://www.animationmagazine.net/feed/".into(),
            anime_focused: false,
        },
        NewsFeedSource {
            outlet: "Anime Corner".into(),
            url: "https://animecorner.me/feed/".into(),
            anime_focused: true,
        },
        NewsFeedSource {
            outlet: "Otaku USA Magazine".into(),
            url: "https://otakuusamagazine.com/feed/".into(),
            anime_focused: true,
        },
        NewsFeedSource {
            outlet: "Skwigly".into(),
            url: "https://www.skwigly.co.uk/feed/".into(),
            anime_focused: false,
        },
    ]
}

pub fn sources_from_env() -> Option<Vec<NewsFeedSource>> {
    parse_sources_spec(&std::env::var("ATLAS_NEWS_FEEDS").ok()?)
}

pub fn parse_sources_spec(raw: &str) -> Option<Vec<NewsFeedSource>> {
    let sources: Vec<NewsFeedSource> = raw
        .split([',', '\n'])
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let mut parts = entry.splitn(3, '|');
            let outlet = parts.next()?.trim().to_string();
            let url = parts.next()?.trim().to_string();
            let anime_focused = parts.next().map(|f| f.trim() == "anime").unwrap_or(false);
            Some(NewsFeedSource {
                outlet,
                url,
                anime_focused,
            })
        })
        .collect();
    if sources.is_empty() { None } else { Some(sources) }
}

pub fn load_sources(path: &Path) -> Result<Vec<NewsFeedSource>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

pub struct AnimationNewsDesk {
    client: reqwest::Client,
    sources: Vec<NewsFeedSource>,
    max_age: Duration,
}

impl Default for AnimationNewsDesk {
    fn default() -> Self {
        Self::new(sources_from_env().unwrap_or_else(default_sources), 72)
    }
}

impl AnimationNewsDesk {
    pub fn new(sources: Vec<NewsFeedSource>, max_age_hours: i64) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .user_agent("ClearlyDrawnNews/0.1 (+https://github.com/Snapwave333/Clearly-Drawn-News)")
                .build()
                .unwrap_or_default(),
            sources,
            max_age: Duration::hours(max_age_hours.max(1)),
        }
    }

    pub fn sources(&self) -> &[NewsFeedSource] {
        &self.sources
    }

    pub async fn fetch_wire(&self) -> WireFetchReport {
        let mut report = WireFetchReport::default();
        let cutoff = Utc::now() - self.max_age;
        for source in &self.sources {
            match self.fetch_bytes(&source.url).await {
                Ok(bytes) => match parse_feed(source, &bytes) {
                    Ok(items) => {
                        report.feeds_ok += 1;
                        report.items.extend(
                            items
                                .into_iter()
                                .filter(|item| item.published.is_none_or(|p| p >= cutoff)),
                        );
                    }
                    Err(error) => report.feed_errors.push((source.outlet.clone(), error)),
                },
                Err(error) => report.feed_errors.push((source.outlet.clone(), error)),
            }
        }
        report
    }

    pub async fn enrich_article_text(&self, story: &mut NewsStory, max_chars: usize) {
        let mut collected = String::new();
        for source in &story.sources {
            if collected.len() >= max_chars {
                break;
            }
            if let Ok(bytes) = self.fetch_bytes(&source.url).await {
                let text = extract_article_text(&String::from_utf8_lossy(&bytes), max_chars);
                if !text.is_empty() {
                    if !collected.is_empty() {
                        collected.push(' ');
                    }
                    collected.push_str(&format!("[{}] {}", source.outlet, text));
                }
            }
        }
        story.article_text = truncate_chars(&collected, max_chars);
    }

    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        if let Some(path) = url.strip_prefix("file://") {
            return std::fs::read(path).map_err(|e| format!("{path}: {e}"));
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return std::fs::read(url).map_err(|e| format!("{url}: {e}"));
        }
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("{url}: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("{url}: HTTP {}", response.status()));
        }
        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("{url}: {e}"))
    }
}

pub fn parse_feed(source: &NewsFeedSource, bytes: &[u8]) -> Result<Vec<WireItem>, String> {
    let feed = feed_rs::parser::parse(bytes).map_err(|e| format!("feed parse failed: {e}"))?;
    let mut items = Vec::new();
    for entry in feed.entries {
        let title = entry
            .title
            .as_ref()
            .map(|t| clean_text(&t.content))
            .unwrap_or_default();
        let url = entry
            .links
            .first()
            .map(|l| l.href.clone())
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let summary_html = entry
            .summary
            .as_ref()
            .map(|s| s.content.clone())
            .or_else(|| entry.content.as_ref().and_then(|c| c.body.clone()))
            .unwrap_or_default();
        let summary = truncate_chars(&clean_text(&summary_html), 1200);
        let published = entry.published.or(entry.updated);
        let feed_categories = entry.categories.iter().map(|c| c.term.clone()).collect();
        items.push(WireItem {
            id: entry.id.clone(),
            outlet: source.outlet.clone(),
            title,
            url,
            published,
            summary,
            feed_categories,
            anime_focused_outlet: source.anime_focused,
        });
    }
    Ok(items)
}

const ORGANIZATIONS: &[&str] = &[
    "MAPPA",
    "ufotable",
    "Kyoto Animation",
    "Science SARU",
    "Wit Studio",
    "CloverWorks",
    "Bones",
    "Madhouse",
    "Production I.G",
    "Studio Trigger",
    "Toei Animation",
    "Sunrise",
    "Bandai Namco",
    "A-1 Pictures",
    "Studio Ghibli",
    "CoMix Wave Films",
    "David Production",
    "Shaft",
    "Studio Pierrot",
    "Pierrot",
    "Lerche",
    "J.C.Staff",
    "P.A. Works",
    "OLM",
    "Polygon Pictures",
    "Orange",
    "TMS Entertainment",
    "Aniplex",
    "Toho",
    "Shueisha",
    "Kodansha",
    "Crunchyroll",
    "Pixar",
    "Walt Disney Animation Studios",
    "Disney",
    "DreamWorks Animation",
    "DreamWorks",
    "Illumination",
    "Sony Pictures Animation",
    "Netflix",
    "Warner Bros. Animation",
    "Warner Bros.",
    "Cartoon Network",
    "Nickelodeon",
    "Adult Swim",
    "Fortiche",
    "Riot Games",
    "Laika",
    "Aardman",
    "Cartoon Saloon",
    "Titmouse",
    "Bento Box",
    "Prime Video",
    "Amazon",
    "Apple TV+",
    "HBO",
    "Paramount",
    "Skydance Animation",
    "Annecy",
    "Studio Mir",
    "Sony",
    "GKIDS",
    "Fleischer",
    "Hasbro",
    "Mattel",
    "Lucasfilm",
    "Marvel Animation",
    "Spumco",
    "Rough Draft Studios",
    "Kugali",
    "Blue Sky",
];

pub fn detect_organization(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    ORGANIZATIONS
        .iter()
        .find(|org| contains_word(&lower, &org.to_lowercase()))
        .map(|org| org.to_string())
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let begin = start + pos;
        let end = begin + needle.len();
        let before_ok = haystack[..begin]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

const ADULT_TERMS: &[&str] = &[
    "adult swim",
    "adult animation",
    "adult animated",
    "rick and morty",
    "the simpsons",
    "family guy",
    "south park",
    "bob's burgers",
    "futurama",
    "invincible",
    "arcane",
    "harley quinn",
    "hazbin hotel",
    "helluva boss",
    "big mouth",
    "smiling friends",
    "primal",
    "castlevania",
    "mature audiences",
];

const ANIME_TERMS: &[&str] = &[
    "anime",
    "manga",
    "light novel",
    "isekai",
    "shonen",
    "crunchyroll",
    "aniplex",
    "mappa",
    "ufotable",
    "kyoto animation",
    "science saru",
    "toei animation",
    "studio ghibli",
    "cloverworks",
    "wit studio",
];

const INDUSTRY_TERMS: &[&str] = &[
    "box office",
    "layoff",
    "layoffs",
    "union",
    "acquisition",
    "acquires",
    "merger",
    "ceo",
    "executive",
    "festival",
    "award",
    "awards",
    "annecy",
    "oscar",
    "emmy",
    "annie awards",
    "software",
    "pipeline",
    "hiring",
    "budget",
    "strike",
    "deal",
    "studio opens",
    "revenue",
    "earnings",
];

const RELEASE_TERMS: &[&str] = &[
    "release date",
    "premiere",
    "premieres",
    "trailer",
    "teaser",
    "first look",
    "coming to",
    "debuts",
    "key visual",
    "announced for",
    "slated for",
    "greenlit",
    "renewed",
    "season 2",
    "season 3",
];

pub fn classify(item: &WireItem) -> AnimationCategory {
    let text = format!(
        "{} {} {}",
        item.title,
        item.summary,
        item.feed_categories.join(" ")
    )
    .to_lowercase();
    let title = item.title.to_lowercase();
    let has = |terms: &[&str], hay: &str| terms.iter().any(|t| contains_word(hay, t));
    if has(ADULT_TERMS, &text) {
        AnimationCategory::AdultAnimation
    } else if has(ANIME_TERMS, &title) || (item.anime_focused_outlet && !has(INDUSTRY_TERMS, &title)) {
        AnimationCategory::Anime
    } else if has(INDUSTRY_TERMS, &title) {
        AnimationCategory::Industry
    } else if has(RELEASE_TERMS, &title) {
        AnimationCategory::UpcomingReleases
    } else if has(ANIME_TERMS, &text) {
        AnimationCategory::Anime
    } else {
        AnimationCategory::Cartoons
    }
}

const RUMOR_TERMS: &[&str] = &[
    "rumor",
    "rumors",
    "rumour",
    "rumours",
    "rumored",
    "leak",
    "leaked",
    "leaks",
    "reportedly",
    "allegedly",
    "unconfirmed",
    "speculation",
    "insider",
    "fan theory",
    "fans think",
];

pub fn is_rumor(item: &WireItem) -> bool {
    let text = format!("{} {}", item.title, item.summary).to_lowercase();
    RUMOR_TERMS.iter().any(|t| contains_word(&text, t))
}

pub fn is_non_news(item: &WireItem) -> bool {
    let title = item.title.to_lowercase();
    let url = item.url.to_lowercase();
    url.contains("/review")
        || url.contains("/feature/")
        || url.contains("/column")
        || url.contains("/interview")
        || url.contains("/sponsored")
        || title.starts_with("review")
        || title.contains("episode review")
        || title.contains("sponsored")
        || title.contains("giveaway")
        || title.contains("[review]")
        || title.contains("this week in anime")
        || title.contains("weekly roundup")
}

fn tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<[^>]*>").expect("valid regex"))
}

fn paragraph_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<p\b[^>]*>(.*?)</p>").expect("valid regex"))
}

fn junk_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?is)<(script|style|noscript|nav|footer|header|aside|form)\b.*?</(script|style|noscript|nav|footer|header|aside|form)>")
            .expect("valid regex")
    })
}

pub fn clean_text(html: &str) -> String {
    let stripped = tag_regex().replace_all(html, " ");
    let decoded = decode_entities(&stripped);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn extract_article_text(html: &str, max_chars: usize) -> String {
    let body = junk_regex().replace_all(html, " ");
    let mut text = String::new();
    for capture in paragraph_regex().captures_iter(&body) {
        let paragraph = clean_text(&capture[1]);
        let lower = paragraph.to_lowercase();
        if paragraph.chars().count() < 60
            || lower.contains("cookie")
            || lower.contains("subscribe")
            || lower.contains("newsletter")
            || lower.contains("all rights reserved")
            || lower.contains("advertisement")
        {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&paragraph);
        if text.len() >= max_chars {
            break;
        }
    }
    truncate_chars(&text, max_chars)
}

pub fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        if let Some(semi) = tail[..tail.len().min(12)].find(';') {
            let entity = &tail[1..semi];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                "hellip" => Some('…'),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "rsquo" | "lsquo" => Some('\''),
                "rdquo" | "ldquo" => Some('"'),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(c) = decoded {
                out.push(c);
                rest = &tail[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    match cut.rfind(' ') {
        Some(pos) if pos > max_chars / 2 => format!("{}…", &cut[..pos]),
        _ => format!("{cut}…"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Test</title>
<item><title>MAPPA Announces Chainsaw Man Season 2 Release Date</title>
<link>https://example.com/news/csm</link>
<pubDate>Fri, 16 Oct 2026 10:00:00 GMT</pubDate>
<description>&lt;p&gt;MAPPA confirmed the &amp;quot;Reze&amp;quot; arc.&lt;/p&gt;</description></item>
<item><title>Rumor: Studio leak hints at sequel</title>
<link>https://example.com/news/leak</link></item>
</channel></rss>"#;

    fn source() -> NewsFeedSource {
        NewsFeedSource {
            outlet: "Example".into(),
            url: "file:///dev/null".into(),
            anime_focused: false,
        }
    }

    #[test]
    fn parses_rss_items_and_decodes_html() {
        let items = parse_feed(&source(), RSS.as_bytes()).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].summary, "MAPPA confirmed the \"Reze\" arc.");
        assert!(items[0].published.is_some());
        assert_eq!(classify(&items[0]), AnimationCategory::Anime);
        assert!(is_rumor(&items[1]));
        assert_eq!(detect_organization(&items[0].title).as_deref(), Some("MAPPA"));
    }

    #[test]
    fn organization_matching_respects_word_boundaries() {
        assert_eq!(detect_organization("The Orangery opens"), None);
        assert_eq!(detect_organization("Pixar unveils new film").as_deref(), Some("Pixar"));
    }

    #[test]
    fn extracts_article_paragraphs() {
        let html = "<html><script>var x = 1;</script><p>Short.</p><p>The studio confirmed that production on the second season has begun with the returning staff.</p></html>";
        let text = extract_article_text(html, 500);
        assert!(text.starts_with("The studio confirmed"));
    }

    #[test]
    fn env_sources_parse() {
        let parsed = parse_sources_spec("A|https://a.test/feed|anime,B|https://b.test/rss").unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].anime_focused);
        assert!(!parsed[1].anime_focused);
        assert!(parse_sources_spec(" , ").is_none());
    }
}
