use crate::news_desk::{AnimationCategory, Sourcing, VerifiedNewsItem};
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
    /// Region tag read on air and used to mix regions ("US", "Japan", "Korea", ...).
    pub region: &'static str,
    /// ISO 639-1 language of the feed. Non-English entries are translated by the director
    /// before grouping; attribution and fact checks keep working on the original text.
    pub language: &'static str,
    /// General-news feeds (Yonhap, Sixth Tone, Playback, ...) only contribute entries that are
    /// about animation.
    pub animation_only: bool,
    /// Set for an organisation's own newsroom: its posts are official statements by that
    /// organisation (see `Sourcing::official_primary`).
    pub official_for: Option<&'static str>,
}

impl FeedSource {
    const fn region(mut self, region: &'static str, language: &'static str) -> Self {
        self.region = region;
        self.language = language;
        self
    }

    const fn animation_only(mut self) -> Self {
        self.animation_only = true;
        self
    }

    const fn newsroom(mut self, organization: &'static str) -> Self {
        self.official_for = Some(organization);
        self
    }

    /// On-air credit: "Natalie, Japan"; US outlets are credited by name alone.
    pub fn credit(&self) -> String {
        credit_line(self.name, self.region)
    }
}

/// "via Natalie, Japan" style credit; US and global outlets are credited by name alone.
pub fn credit_line(name: &str, region: &str) -> String {
    if region.is_empty() || region == "US" || region == "Global" { name.to_string() } else { format!("{name}, {region}") }
}

/// Honest, identifiable User-Agent. Some publishers (Animation Magazine) reject bare tool
/// agents with a 403 but serve this one; every feed path below is allowed by its robots.txt.
pub const WIRE_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; ClearlyDrawnNews/1.1; +https://www.youtube.com/channel/UC16fbSsDJcc6RVvNsH3Cbfg)";

const fn feed(name: &'static str, url: &'static str, default_category: AnimationCategory) -> FeedSource {
    FeedSource { name, url, default_category, region: "US", language: "en", animation_only: false, official_for: None }
}

/// Reputable animation news feeds (RSS/Atom), tagged by region and language. The Variety, THR,
/// Deadline and IndieWire feeds are their animation tag feeds. Crunchyroll's news feed lives on
/// its news API host (crunchyroll.com/news/rss now serves an HTML app shell).
///
/// Checked 2026-10-18 and NOT listed (no feed, or the feed refuses automated readers):
/// Natalie (natalie.mu answers 405 to non-browser clients), Animate Times (no feed), MANTANWEB
/// (index.rss is empty, category feeds 403), Oricon (feeds 410 Gone), Famitsu (no feed), Toho,
/// Toei Animation, Aniplex and Kadokawa corporate newsrooms (no RSS; their official YouTube
/// channels are followed instead, see `news_social`), Annecy (no feed; covered by AWN, Cartoon
/// Brew, Animation Magazine), AWN Europe (403), Cartoon Brew's Europe tag (empty), SCMP (404).
pub const FEEDS: &[FeedSource] = &[
    // United States / global English-language trade press
    feed("Anime News Network", "https://www.animenewsnetwork.com/all/rss.xml", AnimationCategory::Anime).region("Global", "en"),
    feed("Cartoon Brew", "https://www.cartoonbrew.com/feed", AnimationCategory::Cartoons),
    feed("Animation Magazine", "https://www.animationmagazine.net/feed/", AnimationCategory::Industry),
    feed("Crunchyroll News", "https://cr-news-api-service.prd.crunchyrollsvc.com/v1/en-US/rss", AnimationCategory::Anime).region("Global", "en"),
    feed("Animation World Network", "https://www.awn.com/news/rss.xml", AnimationCategory::Industry),
    feed("Variety", "https://variety.com/t/animation/feed/", AnimationCategory::Industry),
    feed("The Hollywood Reporter", "https://www.hollywoodreporter.com/t/animation/feed/", AnimationCategory::Industry),
    feed("Deadline", "https://deadline.com/tag/animation/feed/", AnimationCategory::Industry),
    feed("IndieWire", "https://www.indiewire.com/t/animation/feed/", AnimationCategory::Industry),
    feed("The Animation Guild", "https://animationguild.org/feed/", AnimationCategory::Industry),
    // Japan (English-language coverage, then official newsrooms in Japanese)
    feed("Anime Corner", "https://animecorner.me/feed/", AnimationCategory::Anime).region("Japan", "en"),
    feed("MAPPA", "https://www.mappa.co.jp/feed", AnimationCategory::Anime).region("Japan", "ja").newsroom("MAPPA"),
    feed("Studio Ghibli", "https://www.ghibli.jp/index.xml", AnimationCategory::Anime).region("Japan", "ja").newsroom("Studio Ghibli"),
    // Europe
    feed("Skwigly", "https://www.skwigly.co.uk/feed/", AnimationCategory::Cartoons).region("UK", "en"),
    feed("Animation Magazine", "https://www.animationmagazine.net/category/europe/feed/", AnimationCategory::Industry).region("Europe", "en"),
    // Korea
    feed("Yonhap", "https://en.yna.co.kr/RSS/culture.xml", AnimationCategory::Industry).region("Korea", "en").animation_only(),
    feed("Soompi", "https://www.soompi.com/feed", AnimationCategory::Cartoons).region("Korea", "en").animation_only(),
    feed("The Korea Herald", "https://www.koreaherald.com/rss/newsAll", AnimationCategory::Industry).region("Korea", "en").animation_only(),
    // China (reputable outlets only, for Bilibili and donghua news)
    feed("Sixth Tone", "https://www.sixthtone.com/rss/index.xml", AnimationCategory::Industry).region("China", "en").animation_only(),
    // Canada
    feed("Playback", "https://playbackonline.ca/feed/", AnimationCategory::Industry).region("Canada", "en").animation_only(),
];

/// Words that make a general-news entry an animation story.
const ANIMATION_WORDS: &[&str] = &[
    "animation", "animated", "animator", "anime", "cartoon", "webtoon", "donghua", "pixar", "ghibli",
    "dreamworks", "stop-motion", "stop motion", "annecy", "manga adaptation", "cg film",
];

pub fn is_animation_story(text: &str) -> bool {
    let lower = text.to_lowercase();
    ANIMATION_WORDS.iter().any(|w| lower.contains(w))
}

/// Japanese, Korean and Chinese names of known organisations, mapped to their English names.
/// Matched as plain substrings (CJK text has no word boundaries).
const ORGANIZATION_ALIASES: &[(&str, &str)] = &[
    ("東映アニメーション", "Toei Animation"), ("スタジオジブリ", "Studio Ghibli"), ("京都アニメーション", "Kyoto Animation"),
    ("アニプレックス", "Aniplex"), ("東宝", "Toho"), ("サンライズ", "Sunrise"), ("ボンズ", "Bones"),
    ("シャフト", "Shaft"), ("マッドハウス", "Madhouse"), ("プロダクション・アイジー", "Production I.G"),
    ("トムス・エンタテインメント", "TMS Entertainment"), ("集英社", "Shueisha"), ("講談社", "Kodansha"),
    ("小学館", "Shogakukan"), ("サイエンスSARU", "Science SARU"), ("スタジオぴえろ", "Studio Pierrot"),
    ("ぴえろ", "Pierrot"), ("ポニーキャニオン", "Pony Canyon"), ("バンダイナムコ", "Bandai Namco"),
    ("コミックス・ウェーブ・フィルム", "CoMix Wave Films"), ("ポリゴン・ピクチュアズ", "Polygon Pictures"),
    ("スタジオ地図", "Studio Chizu"), ("テレビ東京", "TV Tokyo"), ("フジテレビ", "Fuji TV"),
    ("角川", "Kadokawa"), ("ネットフリックス", "Netflix"), ("ディズニー", "Disney"), ("ピクサー", "Pixar"),
    ("스튜디오 미르", "Studio Mir"), ("네이버웹툰", "Naver Webtoon"), ("哔哩哔哩", "Bilibili"), ("腾讯", "Tencent"),
    ("追光动画", "Light Chaser Animation"), ("彩条屋", "Coloroom Pictures"),
];


const KNOWN_ORGANIZATIONS: &[&str] = &[
    // Japan
    "MAPPA", "Toei Animation", "Studio Ghibli", "Kyoto Animation", "ufotable", "Science SARU",
    "Production I.G", "Wit Studio", "CloverWorks", "A-1 Pictures", "Bones", "Sunrise", "Bandai Namco Filmworks",
    "Studio Trigger", "Trigger", "TMS Entertainment", "Madhouse", "Shaft", "David Production", "Aniplex",
    "Crunchyroll", "Shueisha", "Kodansha", "Shogakukan", "Kadokawa", "Toho", "Pierrot", "Studio Pierrot",
    "J.C.Staff", "Lerche", "P.A. Works", "Doga Kobo", "Kinema Citrus", "OLM", "Production Reed", "Studio Deen",
    "Silver Link", "Studio Bind", "Studio Chizu", "CoMix Wave Films", "Polygon Pictures", "Orange", "Studio 4°C",
    "Studio Colorido", "Bandai Namco", "Sony Music", "Pony Canyon", "TOHO Animation", "Arc System Works",
    "Square Enix", "Nintendo", "Sega", "Capcom", "Konami", "Netflix Japan", "Fuji TV", "TV Tokyo", "NHK",
    // North America / Europe
    "Pixar", "Walt Disney Animation Studios", "Walt Disney Animation", "Disney Television Animation", "Disney",
    "DreamWorks Animation", "DreamWorks", "Sony Pictures Animation", "Sony Pictures Imageworks", "Illumination",
    "Warner Bros. Animation", "Warner Bros. Pictures Animation", "Warner Bros", "Cartoon Network", "Adult Swim",
    "Nickelodeon", "Paramount Animation", "Paramount", "Netflix", "Amazon MGM Studios", "Amazon", "Prime Video",
    "Apple TV+", "Apple", "HBO", "Max", "Hulu", "Peacock", "Universal", "Skydance Animation", "Fortiche",
    "Riot Games", "Aardman", "Laika", "Cartoon Saloon", "Titmouse", "Bento Box", "ShadowMachine", "Lionsgate",
    "Annapurna", "Annecy", "Annie Awards", "ASIFA-Hollywood", "ASIFA", "The Animation Guild", "Academy of Motion Picture Arts and Sciences",
    "Oscars", "Sony", "Nelvana", "Guru Studio", "Mercury Filmworks", "Xilam", "Studio Canal", "StudioCanal",
    "Blue Zoo", "Moonbot", "Sesame Workshop", "PBS Kids", "BBC", "Channel 4", "Cartoonito", "WildBrain",
    "Hasbro", "Mattel", "Spin Master", "Lego", "Epic Games", "Unreal Engine", "Autodesk", "Toon Boom",
    "Blender", "Adobe", "SIGGRAPH", "GKIDS", "Mubi", "Neon", "A24", "Elevation Pictures", "Gaumont", "Studio Mir",
    "Locus", "Tencent", "Bilibili", "Light Chaser Animation", "Haoliners", "Ghibli", "Toon Boom Animation",
];

const RUMOR_WORDS: [&str; 9] = [
    "rumor", "rumour", "leak", "leaked", "reportedly", "unconfirmed", "insider", "allegedly",
    "speculation",
];

/// Rumour markers in Japanese, Korean and Chinese source text.
const RUMOR_WORDS_NATIVE: &[&str] = &["噂", "リーク", "未確認", "情報筋", "루머", "유출", "传闻", "爆料", "疑似"];

const STOPWORDS: [&str; 24] = [
    "the", "a", "an", "and", "of", "to", "in", "for", "on", "with", "at", "by", "from", "is",
    "its", "as", "new", "season", "anime", "film", "movie", "series", "gets", "reveals",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedEntry {
    pub source: String,
    /// Region tag of the outlet ("US", "Japan", ...).
    #[serde(default)]
    pub region: String,
    /// Language the entry was published in (ISO 639-1).
    #[serde(default)]
    pub language: String,
    /// The organisation whose own newsroom or verified official account published this entry.
    #[serde(default)]
    pub official_for: Option<String>,
    /// "YouTube", "Bluesky", "newsroom", ... for official entries.
    #[serde(default)]
    pub platform: Option<String>,
    /// Headline and summary as published, kept when the entry was translated into English.
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub original_summary: Option<String>,
    /// Longer article text from the feed (content:encoded / description), for attribution.
    #[serde(default)]
    pub body: String,
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
                    // YouTube repeats the title as <media:title>; only the entry's own title counts.
                    if tag.name().as_ref() == b"media:title" {
                        field = Some("media_title".into());
                        continue;
                    }
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
                        if current.body.is_empty() {
                            current.body = current.summary.clone();
                        }
                        current.body = clean_body(&current.body);
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
        region: source.region.to_string(),
        language: source.language.to_string(),
        official_for: source.official_for.map(str::to_string),
        platform: source.official_for.map(|_| "newsroom".to_string()),
        default_category: source.default_category,
        ..FeedEntry::default()
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
        Some("encoded") | Some("content") if entry.body.is_empty() => entry.body = value.to_string(),
        _ => {}
    }
}

fn strip_markup(raw: &str) -> String {
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
    decode_entities(&text).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&#8217;", "'")
        .replace("&#8216;", "'")
        .replace("&#8220;", "\"")
        .replace("&#8221;", "\"")
        .replace("&#8211;", "-")
        .replace("&#8212;", "-")
        .replace("&#8230;", "...")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
}

/// Article text kept for attribution and corroboration (first ~4000 characters).
fn clean_body(raw: &str) -> String {
    strip_markup(raw).chars().take(4000).collect()
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

fn normalize_token(token: &str) -> String {
    let token = token.to_lowercase();
    let token = token.trim_end_matches("'s").trim_end_matches('\u{2019}');
    if token.len() > 4 && token.ends_with('s') && !token.ends_with("ss") {
        token[..token.len() - 1].to_string()
    } else {
        token.to_string()
    }
}

fn title_tokens(title: &str) -> HashSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(normalize_token)
        .filter(|t| t.len() > 2 && !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Distinctive names in a headline: runs of capitalised words ("Black Clover", "Spider-Verse").
fn title_names(title: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut run: Vec<String> = Vec::new();
    let words: Vec<&str> = title.split_whitespace().collect();
    for (index, raw) in words.iter().enumerate() {
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric());
        let capital = word.chars().next().is_some_and(char::is_uppercase);
        let generic = STOPWORDS.contains(&word.to_lowercase().as_str());
        if capital && !generic && !(index == 0 && words.len() > 1 && run.is_empty() && is_verb_like(word)) {
            run.push(word.to_lowercase());
        } else {
            if run.len() >= 2 {
                names.insert(run.join(" "));
            }
            run.clear();
        }
    }
    if run.len() >= 2 {
        names.insert(run.join(" "));
    }
    names
}

fn is_verb_like(word: &str) -> bool {
    matches!(word.to_lowercase().as_str(), "watch" | "exclusive" | "first" | "review" | "trailer" | "interview")
}

#[derive(Clone)]
struct StoryKey {
    tokens: HashSet<String>,
    names: HashSet<String>,
}

impl StoryKey {
    fn of(entry: &FeedEntry) -> Self {
        Self { tokens: title_tokens(&entry.title), names: title_names(&entry.title) }
    }
}

/// Fuzzy "same story" test across outlets: either a shared multi-word name plus one more shared
/// token, or enough shared distinctive title tokens (half of the shorter title, at least 2;
/// or 3 or more outright).
fn same_story(a: &StoryKey, b: &StoryKey) -> bool {
    let shared = a.tokens.intersection(&b.tokens).count();
    let smaller = a.tokens.len().min(b.tokens.len());
    let shared_name = a.names.intersection(&b.names).next().is_some();
    (shared_name && shared >= 3) || (smaller > 0 && shared >= 2 && shared * 2 >= smaller) || shared >= 4
}

fn mentions(lower: &str, needle: &str) -> bool {
    lower.match_indices(needle).any(|(i, _)| {
        // Only Latin letters and digits join words; "MAPPAが" still names MAPPA.
        let before = lower[..i].chars().next_back();
        let after = lower[i + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// Studio names that are also ordinary words: matched only with their capital letter and not as
/// the first word of a sentence ("Sunrise" the studio, not "sunrise" or "Sunrise over Tokyo...").
const AMBIGUOUS_ORGANIZATIONS: &[&str] = &[
    "Bones", "Sunrise", "Trigger", "Shaft", "Orange", "Apple", "Neon", "Max", "Universal", "Locus", "Lego",
    "Blender", "Pierrot", "Sony", "Paramount", "Amazon", "Disney", "Oscars",
];

fn organization(text: &str) -> Option<&'static str> {
    known_organization(text)
        .or_else(|| ORGANIZATION_ALIASES.iter().find(|(native, _)| text.contains(native)).map(|(_, english)| *english))
}

fn known_organization(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    KNOWN_ORGANIZATIONS.iter().copied().find(|org| {
        if AMBIGUOUS_ORGANIZATIONS.contains(org) {
            text.match_indices(org).any(|(i, _)| {
                let before = text[..i].trim_end();
                let after = text[i + org.len()..].chars().next();
                let starts_sentence = before.is_empty() || before.ends_with(['.', '!', '?', ':']);
                !starts_sentence
                    && !text[..i].chars().next_back().is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
            })
        } else {
            mentions(&lower, &org.to_lowercase())
        }
    })
}

pub(crate) fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        current.push(*ch);
        let next = chars.get(i + 1).copied();
        let western_stop = matches!(ch, '.' | '!' | '?') && next.is_none_or(char::is_whitespace) && current.trim().len() > 20;
        // Japanese and Chinese full stops end a sentence wherever they fall.
        let cjk_stop = matches!(ch, '。' | '！' | '？') && current.trim().chars().count() > 6;
        if western_stop || cjk_stop {
            out.push(current.trim().to_string());
            current.clear();
        }
    }
    if current.trim().chars().count() > 6 && (current.trim().len() > 20 || !current.is_ascii()) {
        out.push(current.trim().to_string());
    }
    out
}

const ORG_SUFFIXES: &[&str] = &[
    "Animation Studios", "Animation Studio", "Animation", "Studios", "Studio", "Pictures", "Entertainment",
    "Productions", "Films", "Filmworks", "Media", "Games", "Network",
];

/// Organisation named by pattern ("Studio Orange", "Cartoon Saloon Studios", "Bandai Namco Pictures"):
/// a run of capitalised words that starts with "Studio" or ends with a company suffix.
fn pattern_organization(sentence: &str) -> Option<String> {
    let words: Vec<&str> = sentence.split_whitespace().collect();
    for (index, raw) in words.iter().enumerate() {
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '.');
        if word == "Studio" {
            let next = words.get(index + 1).map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()));
            if let Some(name) = next.filter(|n| n.chars().next().is_some_and(char::is_uppercase)) {
                return Some(format!("Studio {name}"));
            }
        }
        for suffix in ORG_SUFFIXES {
            let suffix_words: Vec<&str> = suffix.split(' ').collect();
            let end = index + suffix_words.len();
            if end > words.len() {
                continue;
            }
            let matches = words[index..end]
                .iter()
                .zip(&suffix_words)
                .all(|(w, s)| w.trim_matches(|c: char| !c.is_alphanumeric()) == *s);
            if !matches || index == 0 {
                continue;
            }
            let mut start = index;
            while start > 0 {
                let prev = words[start - 1].trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '&');
                let capital = prev.chars().next().is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
                if !capital || STOPWORDS.contains(&prev.to_lowercase().as_str()) || index - start >= 3 {
                    break;
                }
                start -= 1;
            }
            if start < index {
                let name = words[start..end]
                    .iter()
                    .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '&'))
                    .collect::<Vec<_>>()
                    .join(" ");
                return Some(name.trim_end_matches('.').to_string());
            }
        }
    }
    None
}

/// Studio or organisation named in the source text, with the sentence that names it.
/// Known names first, then name patterns. Nothing is ever inferred from outside the text.
pub fn attribute_organization(title: &str, text: &str) -> Option<(String, String)> {
    let mut sentences = vec![title.to_string()];
    sentences.extend(split_sentences(text));
    for sentence in &sentences {
        if let Some(org) = organization(sentence) {
            return Some((org.to_string(), sentence.clone()));
        }
    }
    sentences
        .iter()
        .find_map(|sentence| pattern_organization(sentence).map(|org| (org, sentence.clone())))
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

/// Facts for the script: the feed summary, or more of the article body when the feed carries it
/// (up to ~900 characters of whole sentences, minus feed boilerplate).
fn story_facts(entry: &FeedEntry) -> String {
    if entry.body.chars().count() <= entry.summary.chars().count() + 80 {
        return entry.summary.clone();
    }
    let mut out = String::new();
    for sentence in split_sentences(&entry.body) {
        let lower = sentence.to_lowercase();
        if lower.contains("appeared first on") || lower.contains("read more") || lower.contains("subscribe") {
            continue;
        }
        if out.chars().count() + sentence.chars().count() > 900 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&sentence);
    }
    if out.is_empty() { entry.summary.clone() } else { out }
}

/// One grouped story with everything the fact-check record keeps about it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireStory {
    pub item: VerifiedNewsItem,
    /// Every outlet link that reported the story (primary first).
    pub source_urls: Vec<String>,
    pub outlets: Vec<String>,
    /// Feed text the story was built from (title, summary and article body of every member).
    pub source_text: String,
    /// The sentence that names `item.studio_or_organization`, quoted from `source_text`.
    pub attribution_evidence: Option<String>,
    pub attribution_method: Option<String>,
}

pub fn build_wire(entries: Vec<FeedEntry>, now: DateTime<Utc>, max_age_days: i64, max_stories: usize) -> Vec<VerifiedNewsItem> {
    build_wire_detailed(entries, now, max_age_days, max_stories).into_iter().map(|story| story.item).collect()
}

pub fn build_wire_detailed(
    mut entries: Vec<FeedEntry>,
    now: DateTime<Utc>,
    max_age_days: i64,
    max_stories: usize,
) -> Vec<WireStory> {
    let cutoff = now - chrono::Duration::days(max_age_days);
    entries.retain(|entry| entry.published.is_none_or(|date| date >= cutoff && date <= now + chrono::Duration::hours(6)));
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.published));

    let mut groups: Vec<(StoryKey, Vec<FeedEntry>)> = Vec::new();
    for entry in entries {
        let key = StoryKey::of(&entry);
        match groups.iter_mut().find(|(group_key, _)| same_story(group_key, &key)) {
            Some((group_key, members)) => {
                group_key.tokens.extend(key.tokens);
                group_key.names.extend(key.names);
                members.push(entry);
            }
            None => groups.push((key, vec![entry])),
        }
    }

    let mut stories = groups.into_iter().map(|(_, members)| group_story(members, now)).collect::<Vec<_>>();
    stories.sort_by_key(|story| std::cmp::Reverse(story.item.sourcing.independent_sources));
    stories.truncate(max_stories);
    stories
}

/// Independence key: an organisation's own channels and newsroom are one source between them.
fn independence_key(entry: &FeedEntry) -> String {
    match &entry.official_for {
        Some(org) => format!("official:{}", org.to_lowercase()),
        None => entry.source.to_lowercase(),
    }
}

fn platform_phrase(platform: Option<&str>) -> &'static str {
    match platform {
        Some("YouTube") => "YouTube channel",
        Some("Bluesky") => "Bluesky account",
        Some("Mastodon") => "Mastodon account",
        _ => "newsroom",
    }
}

/// Headline + summary as published (the original language), then the article body.
fn original_text(entry: &FeedEntry) -> String {
    let title = entry.original_title.as_deref().unwrap_or(&entry.title);
    let summary = entry.original_summary.as_deref().unwrap_or(&entry.summary);
    format!("{}. {} {}", title.trim_end_matches('.'), summary, entry.body)
}

fn native_rumor(text: &str) -> bool {
    RUMOR_WORDS_NATIVE.iter().any(|w| text.contains(w))
}

fn group_story(mut members: Vec<FeedEntry>, now: DateTime<Utc>) -> WireStory {
    members.sort_by_key(|member| member.published);
    // An organisation's own announcement leads the story when it is part of the group.
    if let Some(index) = members.iter().position(|m| m.official_for.is_some()) {
        let official = members.remove(index);
        members.insert(0, official);
    }
    let primary = members[0].clone();
    let mut outlets = Vec::new();
    let mut source_urls = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut credits = Vec::new();
    for member in &members {
        let key = independence_key(member);
        if keys.contains(&key) {
            continue;
        }
        keys.push(key);
        outlets.push(member.source.clone());
        source_urls.push(member.link.clone());
        credits.push(match &member.official_for {
            Some(org) => format!("{org}'s official {}", platform_phrase(member.platform.as_deref())),
            None => credit_line(&member.source, &member.region),
        });
    }
    let secondary = source_urls.get(1).cloned();
    let source_text = members.iter().map(original_text).collect::<Vec<_>>().join("\n");
    let english_text = members
        .iter()
        .map(|m| format!("{}. {} {}", m.title.trim_end_matches('.'), m.summary, m.body))
        .collect::<Vec<_>>()
        .join("\n");
    let lower = source_text.to_lowercase();
    let category = categorize(&english_text, primary.default_category);
    let official = members.iter().find_map(|m| m.official_for.clone().map(|org| (org, m.clone())));
    let (attribution, method) = match &official {
        Some((org, post)) => (
            Some((
                org.clone(),
                format!(
                    "Official {} post by {org}: \"{}\"",
                    post.platform.as_deref().unwrap_or("newsroom"),
                    post.original_title.as_deref().unwrap_or(&post.title)
                ),
            )),
            Some("official account".to_string()),
        ),
        None => {
            let found = members.iter().find_map(|m| {
                let title = m.original_title.as_deref().unwrap_or(&m.title);
                let summary = m.original_summary.as_deref().unwrap_or(&m.summary);
                attribute_organization(title, &format!("{summary} {}", m.body))
            });
            let method = found.as_ref().map(|_| "feed text match".to_string());
            (found, method)
        }
    };
    let official_primary = official
        .as_ref()
        .map(|(org, post)| format!("{org} announced it on its official {}", platform_phrase(post.platform.as_deref())));
    let official_video_url = members
        .iter()
        .find(|m| m.official_for.is_some() && m.platform.as_deref() == Some("YouTube"))
        .map(|m| m.link.clone());
    let sourcing = Sourcing {
        region: primary.region.clone(),
        independent_sources: keys.len(),
        credits,
        official_primary,
        original_headline: primary.original_title.clone(),
        original_language: primary.original_title.as_ref().map(|_| primary.language.clone()),
        official_video_url,
    };
    let facts = story_facts(&primary);
    let item = VerifiedNewsItem {
        id: story_id(&primary.link),
        title: primary.title.clone(),
        category,
        primary_source_url: primary.link.clone(),
        secondary_source_url: secondary.clone(),
        announcement_date: primary.published.unwrap_or(now).format("%Y-%m-%d").to_string(),
        studio_or_organization: attribution.as_ref().map(|(org, _)| org.clone()).unwrap_or_default(),
        factual_summary: if facts.is_empty() {
            format!("Reported by {}.", primary.source)
        } else {
            format!("{facts} (Source: {})", primary.source)
        },
        editorial_angle: editorial_angle(category).to_string(),
        is_verified: true,
        is_rumor: RUMOR_WORDS.iter().any(|w| lower.split(|c: char| !c.is_alphanumeric()).any(|t| t == *w))
            || native_rumor(&source_text),
        estimated_duration_mins: if secondary.is_some() { 3.5 } else { 2.5 },
        sourcing,
    };
    WireStory {
        item,
        source_urls,
        outlets,
        source_text,
        attribution_method: method,
        attribution_evidence: attribution.map(|(_, sentence)| sentence),
    }
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
    for source in FEEDS.iter().copied() {
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
        .header("User-Agent", WIRE_USER_AGENT)
        .header("Accept", "application/rss+xml, application/atom+xml, application/xml;q=0.9, */*;q=0.5")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(describe)?
        .text()
        .await
        .map_err(describe)?;
    let mut entries = parse_feed(&body, source);
    if source.animation_only {
        entries.retain(|e| is_animation_story(&format!("{} {} {}", e.title, e.summary, e.body)));
    }
    Ok(entries)
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
    fn fuzzy_matching_groups_differently_worded_headlines() {
        let entry = |source: &str, title: &str| FeedEntry {
            source: source.into(),
            body: String::new(),
            title: title.into(),
            link: format!("https://{source}.example/{}", title.len()),
            published: Some(now()),
            summary: String::new(),
            default_category: AnimationCategory::Anime,
            ..FeedEntry::default()
        };
        let a = StoryKey::of(&entry("ann", "Crunchyroll Reveals English Dub Cast for Black Clover Season 2"));
        let b = StoryKey::of(&entry("cr", "Black Clover Season 2 English Dub Cast Announced"));
        let c = StoryKey::of(&entry("brew", "Pixar Sets Release Date for Toy Story 5"));
        assert!(same_story(&a, &b));
        assert!(!same_story(&a, &c));
        let wire = build_wire_detailed(
            vec![
                entry("Anime News Network", "Crunchyroll Reveals English Dub Cast for Black Clover Season 2"),
                entry("Crunchyroll News", "Black Clover Season 2 English Dub Cast Announced"),
            ],
            now(),
            7,
            8,
        );
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0].outlets, vec!["Anime News Network", "Crunchyroll News"]);
        assert!(wire[0].item.secondary_source_url.is_some());
    }

    #[test]
    fn attribution_quotes_the_sentence_that_names_the_studio() {
        let (org, sentence) = attribute_organization(
            "Kokamumo's Null-Meta Manga Gets TV Anime in 2027",
            "The adaptation was announced on Friday. The anime will be produced at Studio Bind with direction by a newcomer.",
        )
        .unwrap();
        assert_eq!(org, "Studio Bind");
        assert!(sentence.contains("produced at Studio Bind"));
        let (org, _) = attribute_organization("New short", "The film comes from Cartoon Saloon in Kilkenny.").unwrap();
        assert_eq!(org, "Cartoon Saloon");
        let (org, _) = attribute_organization("Trailer drops", "The teaser comes from Orbit Pictures.").unwrap();
        assert_eq!(org, "Orbit Pictures");
        assert!(attribute_organization("Voice actor announces marriage", "She shared the news on social media.").is_none());
        assert!(attribute_organization("A sunrise over the bay", "Sunrise colors and bones of the set.").is_none());
    }

    #[test]
    fn organization_matching_respects_word_boundaries() {
        assert_eq!(organization("Maximum effort from Netflix"), Some("Netflix"));
        assert_eq!(organization("A maximum budget"), None);
    }

    fn dated(source: &str, title: &str, link: &str) -> FeedEntry {
        FeedEntry {
            source: source.into(),
            title: title.into(),
            link: link.into(),
            published: Some(now()),
            region: "US".into(),
            language: "en".into(),
            default_category: AnimationCategory::Anime,
            ..FeedEntry::default()
        }
    }

    #[test]
    fn translated_story_corroborates_across_languages_but_still_needs_a_second_source() {
        let mut natalie = dated("Natalie", "Chainsaw Man Assassins Arc Anime Announced for 2027", "https://natalie.mu/comic/news/1");
        natalie.region = "Japan".into();
        natalie.language = "ja".into();
        natalie.original_title = Some("「チェンソーマン」刺客篇アニメ化決定、2027年に".into());
        natalie.body = "MAPPAが制作を担当する。".into();
        let ann = dated("Anime News Network", "Chainsaw Man Assassins Arc Anime Set for 2027", "https://www.animenewsnetwork.com/news/2");
        let alone = build_wire_detailed(vec![natalie.clone()], now(), 7, 8);
        assert_eq!(alone[0].item.sourcing.independent_sources, 1);
        assert!(alone[0].item.secondary_source_url.is_none());
        let wire = build_wire_detailed(vec![natalie, ann], now(), 7, 8);
        assert_eq!(wire.len(), 1);
        let story = &wire[0];
        assert_eq!(story.item.sourcing.independent_sources, 2);
        assert!(story.item.sourcing.credits.contains(&"Natalie, Japan".to_string()));
        assert!(story.item.sourcing.credits.contains(&"Anime News Network".to_string()));
        // Attribution runs on the original Japanese text and quotes it.
        assert_eq!(story.item.studio_or_organization, "MAPPA");
        assert!(story.source_text.contains("刺客篇"));
    }

    #[test]
    fn official_posts_attribute_and_count_once_per_organisation() {
        let mut youtube = dated("Pixar", "Toy Story 5 | Official Trailer", "https://www.youtube.com/watch?v=abc");
        youtube.official_for = Some("Pixar".into());
        youtube.platform = Some("YouTube".into());
        let mut bluesky = youtube.clone();
        bluesky.platform = Some("Bluesky".into());
        bluesky.link = "https://bsky.app/profile/pixar/post/1".into();
        let wire = build_wire_detailed(vec![youtube.clone(), bluesky], now(), 7, 8);
        let story = &wire[0];
        assert_eq!(story.item.studio_or_organization, "Pixar");
        assert_eq!(story.item.sourcing.independent_sources, 1);
        assert!(story.item.secondary_source_url.is_none());
        assert_eq!(story.item.sourcing.official_primary.as_deref(), Some("Pixar announced it on its official YouTube channel"));
        assert_eq!(story.item.sourcing.official_video_url.as_deref(), Some("https://www.youtube.com/watch?v=abc"));
        assert_eq!(story.attribution_method.as_deref(), Some("official account"));
        let outlet = dated("Cartoon Brew", "Toy Story 5 Official Trailer Arrives", "https://www.cartoonbrew.com/ts5");
        let wire = build_wire_detailed(vec![youtube, outlet], now(), 7, 8);
        assert_eq!(wire[0].item.sourcing.independent_sources, 2);
        assert_eq!(wire[0].item.primary_source_url, "https://www.youtube.com/watch?v=abc");
    }

    #[test]
    fn japanese_names_and_sentences_are_recognised() {
        let (org, sentence) = attribute_organization("新作発表", "本日発表された。東映アニメーションが制作する新作映画だ。").unwrap();
        assert_eq!(org, "Toei Animation");
        assert!(sentence.contains("東映アニメーション"));
        assert!(native_rumor("関係者によるリーク情報"));
        assert!(is_animation_story("Naver Webtoon series gets an anime adaptation"));
        assert!(!is_animation_story("K-pop group announces world tour"));
        assert_eq!(credit_line("Natalie", "Japan"), "Natalie, Japan");
        assert_eq!(credit_line("Variety", "US"), "Variety");
    }
}
