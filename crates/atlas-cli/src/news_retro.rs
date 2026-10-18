//! Animation-history retrospectives that fill an episode when verified news runs short.
//!
//! Topics come from `data/retro_catalog.json` (films and series with release dates, studio,
//! technique and people deep dives). Facts never come from the catalog: each topic is researched
//! at run time from its Wikipedia article (`/wiki/` pages, allowed by robots.txt), and a lead
//! sentence is kept only when a reputable source that the article itself cites (Library of
//! Congress, BFI, AFI, Variety, The Guardian, ...; robots.txt honoured; Wayback copies accepted)
//! carries the same years and names. A topic needs at least 3 corroborated facts. Anniversaries
//! are claimed on air only when the article states the exact date. Topics don't repeat within
//! 60 days (`news/aired_retros.json`).
use atlas_media::news_wire::WIRE_USER_AGENT;
use atlas_storyboard::story_script::{RetroBrief, StoryBrief};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

const CATALOG: &str = include_str!("../data/retro_catalog.json");
pub(crate) const RETRO_REPEAT_DAYS: i64 = 60;
const MIN_FACTS: usize = 3;
const MAX_FACTS: usize = 10;
const MAX_SOURCE_FETCHES: usize = 6;

/// Reputable outlets and archives a corroborating citation may come from, with on-air names.
const REPUTABLE: &[(&str, &str)] = &[
    ("loc.gov", "the Library of Congress"), ("bfi.org.uk", "the BFI"), ("afi.com", "the American Film Institute"),
    ("oscars.org", "the Academy"), ("si.edu", "the Smithsonian"), ("smithsonianmag.com", "Smithsonian Magazine"),
    ("variety.com", "Variety"), ("hollywoodreporter.com", "The Hollywood Reporter"), ("deadline.com", "Deadline"),
    ("indiewire.com", "IndieWire"), ("nytimes.com", "The New York Times"), ("latimes.com", "the Los Angeles Times"),
    ("washingtonpost.com", "The Washington Post"), ("theguardian.com", "The Guardian"), ("bbc.co.uk", "the BBC"),
    ("bbc.com", "the BBC"), ("npr.org", "NPR"), ("time.com", "Time"), ("theatlantic.com", "The Atlantic"),
    ("newyorker.com", "The New Yorker"), ("apnews.com", "the Associated Press"), ("reuters.com", "Reuters"),
    ("cnn.com", "CNN"), ("usatoday.com", "USA Today"), ("chicagotribune.com", "the Chicago Tribune"),
    ("rogerebert.com", "RogerEbert.com"), ("empireonline.com", "Empire"), ("ew.com", "Entertainment Weekly"),
    ("avclub.com", "The A.V. Club"), ("vanityfair.com", "Vanity Fair"), ("wired.com", "Wired"),
    ("telegraph.co.uk", "The Telegraph"), ("independent.co.uk", "The Independent"), ("ign.com", "IGN"),
    ("cartoonbrew.com", "Cartoon Brew"), ("awn.com", "Animation World Network"), ("animationmagazine.net", "Animation Magazine"),
    ("animenewsnetwork.com", "Anime News Network"), ("d23.com", "D23, Disney's official fan club"),
    ("waltdisney.org", "the Walt Disney Family Museum"), ("pixar.com", "Pixar"), ("ghibli.jp", "Studio Ghibli"),
    ("nfb.ca", "the National Film Board of Canada"), ("moma.org", "MoMA"), ("britannica.com", "Encyclopaedia Britannica"),
];

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Topic {
    pub(crate) wiki: String,
    pub(crate) label: String,
    pub(crate) kind: String,
    pub(crate) date: Option<String>,
    #[serde(default)]
    pub(crate) popularity: u32,
}

#[derive(Deserialize)]
struct Catalog {
    topics: Vec<Topic>,
}

pub(crate) fn catalog() -> Vec<Topic> {
    serde_json::from_str::<Catalog>(CATALOG).map(|c| c.topics).unwrap_or_default()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RetroFact {
    pub(crate) sentence: String,
    /// URLs (besides Wikipedia) whose text carries this fact's years and names.
    pub(crate) corroborated_by: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Retrospective {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) kind: String,
    pub(crate) framing: String,
    pub(crate) anniversary: Option<String>,
    pub(crate) wikipedia_url: String,
    pub(crate) facts: Vec<RetroFact>,
    /// Every citation: Wikipedia first, then each corroborating source.
    pub(crate) citations: Vec<String>,
    pub(crate) source_names: Vec<String>,
    pub(crate) researched_at: DateTime<Utc>,
}

impl Retrospective {
    pub(crate) fn brief(&self) -> StoryBrief {
        StoryBrief {
            headline: self.title.clone(),
            summary: self.facts.iter().map(|f| f.sentence.as_str()).collect::<Vec<_>>().join(" "),
            category: "Retrospective".into(),
            organization: String::new(),
            outlets: self.source_names.clone(),
            region: String::new(),
            official_announcement: None,
            original_headline: None,
            retrospective: Some(RetroBrief {
                framing: self.framing.clone(),
                anniversary: self.anniversary.clone(),
                facts: self.facts.iter().map(|f| f.sentence.clone()).collect(),
                sources: self.source_names.clone(),
            }),
        }
    }
}

// ---------- topic selection ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AiredRetro {
    key: String,
    episode: String,
    aired_at: DateTime<Utc>,
}

fn aired_path(news_dir: &Path) -> std::path::PathBuf {
    news_dir.join("aired_retros.json")
}

fn load_aired(news_dir: &Path) -> Vec<AiredRetro> {
    std::fs::read(aired_path(news_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub(crate) fn recently_aired(news_dir: &Path, now: DateTime<Utc>) -> Vec<String> {
    let cutoff = now - chrono::Duration::days(RETRO_REPEAT_DAYS);
    load_aired(news_dir).into_iter().filter(|a| a.aired_at >= cutoff).map(|a| a.key).collect()
}

pub(crate) fn record_aired(news_dir: &Path, episode: &str, retros: &[Retrospective], now: DateTime<Utc>) -> anyhow::Result<()> {
    let cutoff = now - chrono::Duration::days(RETRO_REPEAT_DAYS * 2);
    let mut aired: Vec<AiredRetro> = load_aired(news_dir).into_iter().filter(|a| a.aired_at >= cutoff).collect();
    aired.extend(retros.iter().map(|r| AiredRetro { key: r.key.clone(), episode: episode.into(), aired_at: now }));
    std::fs::create_dir_all(news_dir)?;
    std::fs::write(aired_path(news_dir), serde_json::to_vec_pretty(&aired)?)?;
    Ok(())
}

fn day_distance(a: NaiveDate, month: u32, day: u32) -> Option<i64> {
    let this_year = NaiveDate::from_ymd_opt(a.year(), month, day)?;
    let candidates = [this_year, NaiveDate::from_ymd_opt(a.year() - 1, month, day)?, NaiveDate::from_ymd_opt(a.year() + 1, month, day)?];
    candidates.iter().map(|d| (*d - a).num_days().abs()).min()
}

fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| (acc ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Candidate order: the Thursday Throwback guest (on its booking day), anniversaries within
/// 3 days (round anniversaries first), then a daily rotation across kinds.
pub(crate) fn candidate_topics(topics: &[Topic], today: NaiveDate, throwback_guest: Option<&str>, skip: &[String]) -> Vec<Topic> {
    let fresh = |t: &Topic| !skip.contains(&t.wiki);
    let mut ordered: Vec<Topic> = Vec::new();
    if let Some(guest) = throwback_guest {
        let guest_lower = guest.to_lowercase();
        let matched = topics.iter().find(|t| t.label.to_lowercase().contains(&guest_lower)).cloned().unwrap_or_else(|| Topic {
            wiki: guest.replace(' ', "_"),
            label: guest.to_string(),
            kind: "throwback".into(),
            date: None,
            popularity: 0,
        });
        let throwback = Topic { kind: "throwback".into(), ..matched };
        if fresh(&throwback) {
            ordered.push(throwback);
        }
    }
    let mut anniversaries: Vec<(i64, bool, u32, Topic)> = topics
        .iter()
        .filter(|t| fresh(t))
        .filter_map(|t| {
            let date = NaiveDate::parse_from_str(t.date.as_deref()?, "%Y-%m-%d").ok()?;
            let distance = day_distance(today, date.month(), date.day())?;
            let years = today.year() - date.year();
            (distance <= 3 && years >= 5).then(|| (distance, years % 25 == 0 || years % 10 == 0, t.popularity, t.clone()))
        })
        .collect();
    anniversaries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)).then(b.2.cmp(&a.2)));
    for (_, _, _, topic) in anniversaries {
        if !ordered.iter().any(|t| t.wiki == topic.wiki) {
            ordered.push(topic);
        }
    }
    let seed = today.format("%Y-%m-%d").to_string();
    let mut by_kind: HashMap<String, Vec<Topic>> = HashMap::new();
    for topic in topics.iter().filter(|t| fresh(t) && !ordered.iter().any(|o| o.wiki == t.wiki)) {
        by_kind.entry(topic.kind.clone()).or_default().push(topic.clone());
    }
    for list in by_kind.values_mut() {
        list.sort_by_key(|t| stable_hash(&format!("{seed}{}", t.wiki)));
    }
    let kinds = ["studio", "series", "technique", "film", "person"];
    let mut round = 0;
    loop {
        let mut added = false;
        for kind in kinds {
            if let Some(topic) = by_kind.get(kind).and_then(|list| list.get(round)) {
                ordered.push(topic.clone());
                added = true;
            }
        }
        if !added {
            break;
        }
        round += 1;
    }
    ordered
}

pub(crate) fn framing(kind: &str, anniversary_verified: bool) -> &'static str {
    match (kind, anniversary_verified) {
        ("throwback", _) => "Thursday Throwback Warm-Up",
        (_, true) => "This Week in Animation History",
        ("series", false) => "Classic Series Spotlight",
        ("film", false) => "Classic Film Spotlight",
        ("studio", _) => "Studio Deep Dive",
        ("technique", _) => "Technique Deep Dive",
        ("person", _) => "Animation Pioneers",
        _ => "From the Animation Archives",
    }
}

// ---------- HTML and corroboration ----------

static RE_DROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<sup[^>]*>.*?</sup>|<style[^>]*>.*?</style>|<script[^>]*>.*?</script>").unwrap());
static RE_PARA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<p(?:\s[^>]*)?>(.*?)</p>").unwrap());
static RE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
static RE_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\s*(?:\d+|[a-z ]+|citation needed)\s*\]").unwrap());
static RE_EXTERNAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a\b[^>]*class="[^"]*external[^"]*"[^>]*>|<a\b[^>]*rel="[^"]*mw:ExtLink[^"]*"[^>]*>"#).unwrap());
static RE_HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="([^"]+)""#).unwrap());
static RE_WAYBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://web\.archive\.org/web/\d+[a-z_]*/(https?://.+)$").unwrap());
static RE_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(1[89]\d\d|20\d\d)\b").unwrap());
static RE_CAPWORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Z][A-Za-z'\-]{2,}\b").unwrap());
static RE_SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[.!?][\)\]\x22']?\s+").unwrap());

fn decode(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&#x27;", "'")
        .replace("&#160;", " ")
        .replace("&nbsp;", " ")
        .replace("&#91;", "[")
        .replace("&#93;", "]")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#8211;", "-")
        .replace("&ndash;", "-")
        .replace("&mdash;", "-")
}

pub(crate) fn html_text(html: &str) -> String {
    let text = decode(&RE_TAG.replace_all(&RE_DROP.replace_all(html, " "), " "));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tidy(text: &str) -> String {
    let text = RE_BRACKET.replace_all(text, "");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.replace(" .", ".").replace(" ,", ",").replace(" ;", ";").replace(" :", ":").replace("( ", "(").replace(" )", ")")
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    for m in RE_SENTENCE_END.find_iter(text) {
        let next_upper = text[m.end()..].chars().next().is_some_and(|c| c.is_uppercase() || c == '"');
        let before = &text[start..m.start()];
        let abbreviation = before.ends_with(" c") || before.ends_with(" Jr") || before.ends_with(" Dr") || before.ends_with(" St")
            || before.ends_with(" Mr") || before.ends_with(" Mrs") || before.ends_with(" vs") || before.split_whitespace().last().is_some_and(|w| w.len() == 1);
        if next_upper && !abbreviation {
            out.push(text[start..m.end()].trim().to_string());
            start = m.end();
        }
    }
    if start < text.len() {
        out.push(text[start..].trim().to_string());
    }
    out.into_iter().filter(|s| s.chars().count() >= 40 && s.chars().count() <= 400).collect()
}

/// Lead section sentences of a Wikipedia article (Parsoid HTML).
pub(crate) fn wikipedia_lead(html: &str) -> Vec<String> {
    let body = html.split_once("id=\"mw-content-text\"").map(|(_, rest)| rest).unwrap_or(html);
    let lead = body.split("<h2").next().unwrap_or(body);
    let cleaned = RE_DROP.replace_all(lead, " ");
    let text = RE_PARA
        .captures_iter(&cleaned)
        .map(|c| decode(&RE_TAG.replace_all(&c[1], " ")))
        .collect::<Vec<_>>()
        .join(" ");
    split_sentences(&tidy(&text))
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(|h| h.trim_start_matches("www.").to_lowercase())
}

pub(crate) fn reputable_name(url: &str) -> Option<&'static str> {
    let host = host_of(url)?;
    REPUTABLE.iter().find(|(domain, _)| host == *domain || host.ends_with(&format!(".{domain}"))).map(|(_, name)| *name)
}

/// Reputable external citations in the article, as (fetch URL, original URL). Wayback copies
/// of reputable pages count as their original.
pub(crate) fn reputable_citations(html: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for tag in RE_EXTERNAL.find_iter(html) {
        let Some(href) = RE_HREF.captures(tag.as_str()).map(|c| decode(&c[1])) else { continue };
        let href = if href.starts_with("//") { format!("https:{href}") } else { href };
        let original = RE_WAYBACK.captures(&href).map(|c| c[1].to_string()).unwrap_or_else(|| href.clone());
        if reputable_name(&original).is_some() && !out.iter().any(|(fetch, _)| *fetch == href) {
            out.push((href, original));
        }
    }
    // Originals first (fetched directly), Wayback copies after.
    out.sort_by_key(|(fetch, _)| fetch.contains("web.archive.org"));
    out
}

const NOT_NAMES: &[&str] = &[
    "The", "This", "That", "These", "Those", "It", "In", "On", "Of", "And", "But", "For", "With", "As", "At", "By",
    "From", "He", "She", "They", "His", "Her", "Its", "Their", "After", "Before", "During", "When", "While", "However",
    "Although", "Following", "Despite", "Since", "Upon", "American", "British", "Japanese", "English", "Its",
];

fn fact_keys(sentence: &str) -> (Vec<String>, Vec<String>) {
    let years = RE_YEAR.find_iter(sentence).map(|m| m.as_str().to_string()).collect();
    let mut names: Vec<String> = RE_CAPWORD
        .find_iter(sentence)
        .map(|m| m.as_str().trim_end_matches("'s").to_string())
        .filter(|w| !NOT_NAMES.contains(&w.as_str()))
        .collect();
    names.dedup();
    (years, names)
}

/// A sentence is corroborated when the other source carries every year in it and at least 60%
/// of its proper names (and it has at least two such keys to check).
pub(crate) fn corroborates(sentence: &str, source_text: &str) -> bool {
    let (years, names) = fact_keys(sentence);
    if years.len() + names.len() < 2 {
        return false;
    }
    if !years.iter().all(|y| source_text.contains(y.as_str())) {
        return false;
    }
    if names.is_empty() {
        return true;
    }
    let lower = source_text.to_lowercase();
    let hits = names.iter().filter(|n| lower.contains(&n.to_lowercase())).count();
    hits * 10 >= names.len() * 6
}

fn month_day_forms(date: NaiveDate) -> [String; 2] {
    [format!("{} {}, {}", date.format("%B"), date.day(), date.year()), format!("{} {} {}", date.day(), date.format("%B"), date.year())]
}

// ---------- robots.txt ----------

#[derive(Default)]
struct RobotsCache {
    rules: HashMap<String, Vec<(bool, String)>>,
}

impl RobotsCache {
    async fn allowed(&mut self, client: &reqwest::Client, url: &str) -> bool {
        let Ok(parsed) = url::Url::parse(url) else { return false };
        let origin = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or_default());
        if !self.rules.contains_key(&origin) {
            let text = match client.get(format!("{origin}/robots.txt")).send().await {
                Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
                _ => String::new(),
            };
            self.rules.insert(origin.clone(), parse_robots(&text));
        }
        let path = &url[origin.len()..];
        let path = if path.is_empty() { "/" } else { path };
        robots_allows(&self.rules[&origin], path)
    }
}

/// Rules for `User-agent: *` (and ours), as (allow, prefix).
pub(crate) fn parse_robots(text: &str) -> Vec<(bool, String)> {
    let mut rules = Vec::new();
    let mut applies = false;
    let mut in_agents = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim().to_lowercase(), value.trim());
        match key.as_str() {
            "user-agent" => {
                if !in_agents {
                    applies = false;
                }
                in_agents = true;
                let agent = value.to_lowercase();
                applies |= agent == "*" || agent.contains("clearlydrawn");
            }
            "allow" | "disallow" => {
                in_agents = false;
                if applies && !value.is_empty() {
                    rules.push((key == "allow", value.to_string()));
                }
            }
            _ => in_agents = false,
        }
    }
    rules
}

pub(crate) fn robots_allows(rules: &[(bool, String)], path: &str) -> bool {
    let matches = |prefix: &str| {
        let prefix = prefix.trim_end_matches('$');
        if let Some((head, tail)) = prefix.split_once('*') {
            path.starts_with(head) && path[head.len()..].contains(tail.trim_matches('*'))
        } else {
            path.starts_with(prefix)
        }
    };
    rules
        .iter()
        .filter(|(_, prefix)| matches(prefix))
        .max_by_key(|(allow, prefix)| (prefix.len(), *allow))
        .is_none_or(|(allow, _)| *allow)
}

// ---------- research ----------

async fn fetch_text(client: &reqwest::Client, url: &str) -> Option<String> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let kind = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if !kind.is_empty() && !kind.contains("html") && !kind.contains("text") {
        return None;
    }
    response.text().await.ok()
}

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(WIRE_USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
}

/// Researches one topic; None when it can't be grounded in Wikipedia plus another source.
async fn research(client: &reqwest::Client, robots: &mut RobotsCache, topic: &Topic) -> Result<Retrospective, String> {
    let wikipedia_url = format!("https://en.wikipedia.org/wiki/{}", topic.wiki);
    if !robots.allowed(client, &wikipedia_url).await {
        return Err("Wikipedia robots.txt disallows the page".into());
    }
    let page = fetch_text(client, &wikipedia_url).await.ok_or("Wikipedia page unavailable")?;
    let lead = wikipedia_lead(&page);
    if lead.len() < MIN_FACTS {
        return Err(format!("Wikipedia lead too short ({} sentences)", lead.len()));
    }
    let mut corroborated: Vec<RetroFact> = lead.iter().map(|s| RetroFact { sentence: s.clone(), corroborated_by: Vec::new() }).collect();
    let mut citations = vec![wikipedia_url.clone()];
    let mut names = vec!["Wikipedia".to_string()];
    let mut fetched = 0;
    let mut done_originals: Vec<String> = Vec::new();
    for (fetch, original) in reputable_citations(&page) {
        if fetched >= MAX_SOURCE_FETCHES || done_originals.contains(&original) {
            continue;
        }
        if !robots.allowed(client, &fetch).await {
            continue;
        }
        fetched += 1;
        let Some(html) = fetch_text(client, &fetch).await else { continue };
        let text = html_text(&html);
        let mut supported = false;
        for fact in &mut corroborated {
            if corroborates(&fact.sentence, &text) {
                fact.corroborated_by.push(original.clone());
                supported = true;
            }
        }
        if supported {
            done_originals.push(original.clone());
            citations.push(original.clone());
            if let Some(name) = reputable_name(&original) {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
        }
        if corroborated.iter().filter(|f| !f.corroborated_by.is_empty()).count() >= MAX_FACTS {
            break;
        }
    }
    let facts: Vec<RetroFact> = corroborated.into_iter().filter(|f| !f.corroborated_by.is_empty()).take(MAX_FACTS).collect();
    if facts.len() < MIN_FACTS {
        return Err(format!("only {} facts corroborated outside Wikipedia (need {MIN_FACTS})", facts.len()));
    }
    let page_text = html_text(&page);
    let anniversary = topic
        .date
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|date| month_day_forms(date).into_iter().find(|form| page_text.contains(form.as_str())))
        .map(|form| format!("first released {form}"));
    let label = topic.label.clone();
    Ok(Retrospective {
        key: topic.wiki.clone(),
        title: label,
        kind: topic.kind.clone(),
        framing: framing(&topic.kind, anniversary.is_some()).to_string(),
        anniversary,
        wikipedia_url,
        facts,
        citations,
        source_names: names,
        researched_at: Utc::now(),
    })
}

pub(crate) struct RetroResearch {
    pub(crate) retros: Vec<Retrospective>,
    pub(crate) rejected: Vec<(String, String)>,
}

/// Researches candidates in order until `count` retrospectives are grounded (bounded attempts).
pub(crate) async fn research_retros(news_dir: &Path, count: usize, today: NaiveDate, throwback_guest: Option<&str>) -> RetroResearch {
    let mut result = RetroResearch { retros: Vec::new(), rejected: Vec::new() };
    if count == 0 {
        return result;
    }
    let skip = recently_aired(news_dir, Utc::now());
    let candidates = candidate_topics(&catalog(), today, throwback_guest, &skip);
    let client = http_client();
    let mut robots = RobotsCache::default();
    for topic in candidates.iter().take(count * 5 + 2) {
        if result.retros.len() >= count {
            break;
        }
        match research(&client, &mut robots, topic).await {
            Ok(retro) => {
                println!(
                    "  ✓ {}: {} ({} facts; sources: {})",
                    retro.framing,
                    retro.title,
                    retro.facts.len(),
                    retro.source_names.join(", ")
                );
                result.retros.push(retro);
            }
            Err(reason) => {
                println!("  ✗ {} skipped: {reason}", topic.label);
                result.rejected.push((topic.label.clone(), reason));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_loads_with_dated_and_deep_dive_topics() {
        let topics = catalog();
        assert!(topics.len() > 400);
        assert!(topics.iter().any(|t| t.kind == "studio"));
        assert!(topics.iter().any(|t| t.kind == "technique"));
        assert!(topics.iter().filter(|t| t.date.is_some()).count() > 300);
    }

    #[test]
    fn anniversaries_and_throwback_lead_the_order_and_aired_topics_are_skipped() {
        let topics = vec![
            Topic { wiki: "The_Jungle_Book_(1967_film)".into(), label: "The Jungle Book (1967 film)".into(), kind: "film".into(), date: Some("1967-10-18".into()), popularity: 90 },
            Topic { wiki: "Dumbo".into(), label: "Dumbo".into(), kind: "film".into(), date: Some("1941-10-23".into()), popularity: 100 },
            Topic { wiki: "Toy_Story".into(), label: "Toy Story".into(), kind: "film".into(), date: Some("1995-11-22".into()), popularity: 110 },
            Topic { wiki: "Fleischer_Studios".into(), label: "Fleischer Studios".into(), kind: "studio".into(), date: None, popularity: 0 },
            Topic { wiki: "Gertie_the_Dinosaur".into(), label: "Gertie the Dinosaur".into(), kind: "film".into(), date: Some("1914-02-18".into()), popularity: 30 },
        ];
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        let order = candidate_topics(&topics, today, Some("Gertie the Dinosaur"), &[]);
        assert_eq!(order[0].wiki, "Gertie_the_Dinosaur");
        assert_eq!(order[0].kind, "throwback");
        assert_eq!(order[1].wiki, "Dumbo"); // 85 years, 2 days away; The Jungle Book is 3 days back
        assert_eq!(order[2].wiki, "The_Jungle_Book_(1967_film)");
        assert!(order.iter().any(|t| t.wiki == "Fleischer_Studios"));
        assert_eq!(order.iter().filter(|t| t.wiki == "Gertie_the_Dinosaur").count(), 1);
        let skipped = candidate_topics(&topics, today, None, &["Dumbo".to_string()]);
        assert!(skipped.iter().all(|t| t.wiki != "Dumbo"));
    }

    #[test]
    fn aired_retros_are_not_repeated_within_60_days() {
        let dir = tempfile_dir();
        let retro = Retrospective {
            key: "Dumbo".into(),
            title: "Dumbo".into(),
            kind: "film".into(),
            framing: "Classic Film Spotlight".into(),
            anniversary: None,
            wikipedia_url: "https://en.wikipedia.org/wiki/Dumbo".into(),
            facts: Vec::new(),
            citations: Vec::new(),
            source_names: Vec::new(),
            researched_at: Utc::now(),
        };
        let then = Utc::now() - chrono::Duration::days(59);
        record_aired(&dir, "EP-001", &[retro], then).unwrap();
        assert_eq!(recently_aired(&dir, Utc::now()), vec!["Dumbo".to_string()]);
        assert!(recently_aired(&dir, Utc::now() + chrono::Duration::days(2)).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas_retro_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const PAGE: &str = r#"<html><div id="mw-content-text"><p class="mw-empty-elt"></p>
<p><b>Dumbo</b> is a 1941 American animated film produced by <a href="/wiki/Walt_Disney_Productions">Walt Disney Productions</a><sup class="reference">[1]</sup> and released by RKO Radio Pictures.</p>
<p>It was released on October 23, 1941, by RKO Radio Pictures. The film was a box office success for Walt Disney Productions.</p>
<p>In 2017, Dumbo was selected for preservation in the United States National Film Registry by the Library of Congress.</p>
<h2>Plot</h2><p>Ignored text after the lead.</p>
<ol class="references"><li><a rel="mw:ExtLink nofollow" class="external text" href="https://www.loc.gov/item/prn-17-178/">LoC</a></li>
<li><a rel="mw:ExtLink nofollow" class="external text" href="https://web.archive.org/web/20211210214840/https://catalog.afi.com/Film/26684-DUMBO">AFI</a></li>
<li><a class="external text" href="https://randomblog.example/dumbo">blog</a></li></ol></div></html>"#;

    #[test]
    fn wikipedia_lead_and_reputable_citations_are_extracted() {
        let lead = wikipedia_lead(PAGE);
        assert_eq!(lead.len(), 4, "{lead:?}");
        assert!(lead[0].starts_with("Dumbo is a 1941 American animated film produced by Walt Disney Productions and released"));
        assert!(!lead.iter().any(|s| s.contains("[1]") || s.contains("Ignored")));
        let cites = reputable_citations(PAGE);
        assert_eq!(cites.len(), 2);
        assert_eq!(cites[0].0, "https://www.loc.gov/item/prn-17-178/");
        assert_eq!(cites[1].1, "https://catalog.afi.com/Film/26684-DUMBO");
        assert_eq!(reputable_name(&cites[1].1), Some("the American Film Institute"));
    }

    #[test]
    fn corroboration_needs_matching_years_and_names() {
        let source = "Library of Congress National Film Registry 2017: Dumbo (1941), Walt Disney Productions.";
        assert!(corroborates("In 2017, Dumbo was selected for preservation in the United States National Film Registry by the Library of Congress.", source));
        assert!(!corroborates("Dumbo was re-released in 1949 by Walt Disney Productions.", source));
        assert!(!corroborates("It is a lovely film about an elephant and his mother.", source));
    }

    #[test]
    fn robots_rules_are_honoured() {
        let rules = parse_robots("User-agent: Googlebot\nDisallow: /\n\nUser-agent: *\nAllow: /w/load.php?\nDisallow: /w/\nDisallow: /api/\nDisallow: /wiki/Special:\n");
        assert!(robots_allows(&rules, "/wiki/Dumbo"));
        assert!(!robots_allows(&rules, "/api/rest_v1/page/summary/Dumbo"));
        assert!(!robots_allows(&rules, "/wiki/Special:Search"));
        assert!(robots_allows(&rules, "/w/load.php?x=1"));
        assert!(robots_allows(&parse_robots(""), "/anything"));
    }

    #[test]
    fn framing_never_claims_an_unverified_anniversary() {
        assert_eq!(framing("film", true), "This Week in Animation History");
        assert_eq!(framing("film", false), "Classic Film Spotlight");
        assert_eq!(framing("throwback", true), "Thursday Throwback Warm-Up");
        assert_eq!(framing("technique", false), "Technique Deep Dive");
    }
}
