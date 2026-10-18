//! Official studio and network accounts the news researcher follows, through keyless,
//! public routes only: YouTube channel RSS and Bluesky's public AppView API. Organisation
//! newsrooms are ordinary feeds in `news_wire::FEEDS` (marked `newsroom`).
//!
//! The account list is configurable (`ATLAS_NEWS_ACCOUNTS` points at a JSON file in the same
//! shape as `data/official_accounts.json`). Every account is pinned by its platform ID, and a
//! fetch fails if the platform reports a different channel or handle for that ID, so a renamed
//! or hijacked handle is never trusted silently. X, Instagram, TikTok and Threads are not used.

use crate::news_desk::AnimationCategory;
use crate::news_wire::{FeedEntry, FeedSource, FeedStatus, WIRE_USER_AGENT};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

const DEFAULT_ACCOUNTS: &str = include_str!("../data/official_accounts.json");
const BLUESKY_APPVIEW: &str = "https://public.api.bsky.app/xrpc";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Youtube,
    Bluesky,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Youtube => "YouTube",
            Self::Bluesky => "Bluesky",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfficialAccount {
    pub organization: String,
    pub region: String,
    pub language: String,
    pub platform: Platform,
    /// Pinned platform ID: YouTube channel ID ("UC...") or Bluesky DID.
    pub id: String,
    pub handle: String,
    /// Channel title / display name as the platform reports it.
    pub name: String,
    /// How the account was verified as the organisation's own.
    pub verification: String,
}

impl OfficialAccount {
    pub fn label(&self) -> String {
        format!("{} ({} {})", self.organization, self.platform.label(), self.handle)
    }
}

#[derive(Deserialize)]
struct AccountsFile {
    accounts: Vec<OfficialAccount>,
}

pub fn parse_accounts(json: &str) -> Result<Vec<OfficialAccount>, String> {
    let file: AccountsFile = serde_json::from_str(json).map_err(|e| format!("official accounts list: {e}"))?;
    for account in &file.accounts {
        let pinned = match account.platform {
            Platform::Youtube => account.id.starts_with("UC") && account.id.len() == 24,
            Platform::Bluesky => account.id.starts_with("did:"),
        };
        if !pinned {
            return Err(format!("{}: id {} is not a pinned {} ID", account.label(), account.id, account.platform.label()));
        }
    }
    Ok(file.accounts)
}

pub fn default_accounts() -> Vec<OfficialAccount> {
    parse_accounts(DEFAULT_ACCOUNTS).unwrap_or_default()
}

/// The configured list: `ATLAS_NEWS_ACCOUNTS` (a JSON file) when set and valid, else the built-in list.
pub fn load_accounts(lookup: impl Fn(&str) -> Option<String>) -> Vec<OfficialAccount> {
    match lookup("ATLAS_NEWS_ACCOUNTS").filter(|p| !p.trim().is_empty()) {
        Some(path) => match std::fs::read_to_string(Path::new(path.trim())).map_err(|e| e.to_string()).and_then(|j| parse_accounts(&j)) {
            Ok(accounts) => accounts,
            Err(error) => {
                eprintln!("  ATLAS_NEWS_ACCOUNTS ignored ({error}); using the built-in official accounts list.");
                default_accounts()
            }
        },
        None => default_accounts(),
    }
}

/// Official uploads and posts are mostly clips and promos; only announcements become stories.
const ANNOUNCEMENT_WORDS: &[&str] = &[
    "trailer", "teaser", "announce", "first look", "release date", "premiere", "coming soon", "coming to",
    "in theaters", "in cinemas", "renewed", "greenlit", "greenlight", "official clip", "key visual", "sneak peek",
    "now in production", "new series", "new film", "date announcement", "pv",
];
const ANNOUNCEMENT_WORDS_NATIVE: &[&str] = &["予告", "特報", "解禁", "決定", "ティザー", "発表", "制作決定", "放送開始", "公開日", "ＰＶ"];

pub fn is_announcement(title: &str) -> bool {
    let lower = title.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    ANNOUNCEMENT_WORDS.iter().any(|w| if w.contains(' ') { lower.contains(w) } else { words.iter().any(|t| t.starts_with(w)) && (*w != "pv" || words.contains(&"pv")) })
        || ANNOUNCEMENT_WORDS_NATIVE.iter().any(|w| title.contains(w))
}

const YOUTUBE_SOURCE: FeedSource = FeedSource {
    name: "YouTube",
    url: "https://www.youtube.com/feeds/videos.xml",
    default_category: AnimationCategory::UpcomingReleases,
    region: "US",
    language: "en",
    animation_only: false,
    official_for: None,
};

fn tag_official(entry: &mut FeedEntry, account: &OfficialAccount) {
    entry.source = account.organization.clone();
    entry.region = account.region.clone();
    entry.language = account.language.clone();
    entry.official_for = Some(account.organization.clone());
    entry.platform = Some(account.platform.label().to_string());
}

/// Parses a channel's RSS, refusing it unless the feed is for the pinned channel ID.
pub fn parse_youtube(xml: &str, account: &OfficialAccount) -> Result<Vec<FeedEntry>, String> {
    if !xml.contains(&format!("<yt:channelId>{}</yt:channelId>", account.id)) {
        return Err(format!("{}: feed is not for pinned channel {}", account.label(), account.id));
    }
    let mut entries = crate::news_wire::parse_feed(xml, &YOUTUBE_SOURCE);
    entries.retain(|e| is_announcement(&e.title));
    for entry in &mut entries {
        tag_official(entry, account);
    }
    Ok(entries)
}

#[derive(Deserialize)]
struct BskyFeed {
    feed: Vec<BskyItem>,
}
#[derive(Deserialize)]
struct BskyItem {
    post: BskyPost,
    reason: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct BskyPost {
    uri: String,
    author: BskyAuthor,
    record: BskyRecord,
    embed: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct BskyAuthor {
    did: String,
    handle: String,
}
#[derive(Deserialize)]
struct BskyRecord {
    #[serde(default)]
    text: String,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

fn first_sentence(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim();
    let cut = line.find(['.', '!', '?']).map(|i| &line[..=i]).unwrap_or(line);
    let mut out: String = cut.chars().take(140).collect();
    if cut.chars().count() > 140 {
        out.push('…');
    }
    out
}

/// Parses an author feed, refusing posts whose author is not the pinned DID and handle.
/// Reposts are skipped (they are someone else's post).
pub fn parse_bluesky(json: &str, account: &OfficialAccount) -> Result<Vec<FeedEntry>, String> {
    let feed: BskyFeed = serde_json::from_str(json).map_err(|e| format!("{}: {e}", account.label()))?;
    let mut entries = Vec::new();
    for item in feed.feed {
        if item.reason.is_some() {
            continue;
        }
        let post = item.post;
        if post.author.did != account.id || post.author.handle != account.handle {
            return Err(format!(
                "{}: platform reports {} / {}; pinned account no longer matches",
                account.label(),
                post.author.did,
                post.author.handle
            ));
        }
        let external = post.embed.as_ref().and_then(|e| e.get("external"));
        let link_title = external.and_then(|x| x.get("title")).and_then(|v| v.as_str()).unwrap_or_default();
        let link_desc = external.and_then(|x| x.get("description")).and_then(|v| v.as_str()).unwrap_or_default();
        let text = post.record.text.trim().to_string();
        if !is_announcement(&format!("{text} {link_title}")) {
            continue;
        }
        let rkey = post.uri.rsplit('/').next().unwrap_or_default();
        let mut entry = FeedEntry {
            title: first_sentence(&text),
            link: format!("https://bsky.app/profile/{}/post/{rkey}", account.handle),
            published: post.record.created_at.as_deref().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc)),
            summary: text.chars().take(400).collect(),
            body: format!("{text} {link_title} {link_desc}").trim().to_string(),
            default_category: AnimationCategory::UpcomingReleases,
            ..FeedEntry::default()
        };
        tag_official(&mut entry, account);
        if !entry.title.is_empty() {
            entries.push(entry);
        }
    }
    Ok(entries)
}

async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    client
        .get(url)
        .header("User-Agent", WIRE_USER_AGENT)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.without_url().to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())
}

async fn fetch_account(client: &reqwest::Client, account: &OfficialAccount) -> Result<Vec<FeedEntry>, String> {
    match account.platform {
        Platform::Youtube => {
            let xml = get_text(client, &format!("https://www.youtube.com/feeds/videos.xml?channel_id={}", account.id))
                .await
                .map_err(|e| format!("{}: {e}", account.label()))?;
            parse_youtube(&xml, account)
        }
        Platform::Bluesky => {
            let json = get_text(
                client,
                &format!("{BLUESKY_APPVIEW}/app.bsky.feed.getAuthorFeed?actor={}&limit=30&filter=posts_no_replies", account.id),
            )
            .await
            .map_err(|e| format!("{}: {e}", account.label()))?;
            parse_bluesky(&json, account)
        }
    }
}

/// Fetches every followed official account (skipping those named in `skip`).
pub async fn fetch_official(client: &reqwest::Client, accounts: &[OfficialAccount], skip: &[String]) -> (Vec<FeedEntry>, Vec<FeedStatus>) {
    let mut tasks = tokio::task::JoinSet::new();
    let mut statuses = Vec::new();
    for account in accounts {
        let label = account.label();
        if skip.contains(&label) {
            statuses.push(FeedStatus { source: label, entries: 0, error: None, skipped: true });
            continue;
        }
        let account = account.clone();
        let client = client.clone();
        tasks.spawn(async move { (label, fetch_account(&client, &account).await) });
    }
    let mut entries = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok((label, result)) = joined else { continue };
        match result {
            Ok(mut items) => {
                statuses.push(FeedStatus { source: label, entries: items.len(), error: None, skipped: false });
                entries.append(&mut items);
            }
            Err(error) => statuses.push(FeedStatus { source: label, entries: 0, error: Some(error), skipped: false }),
        }
    }
    statuses.sort_by(|a, b| a.source.cmp(&b.source));
    (entries, statuses)
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountCheck {
    pub account: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Deserialize)]
struct BskyProfile {
    did: String,
    handle: String,
    verification: Option<serde_json::Value>,
}

/// Re-verifies every pinned account against the platform, read-only and keyless.
pub async fn check_accounts(client: &reqwest::Client, accounts: &[OfficialAccount]) -> Vec<AccountCheck> {
    let mut out = Vec::new();
    for account in accounts {
        let result: Result<String, String> = match account.platform {
            Platform::Youtube => {
                match get_text(client, &format!("https://www.youtube.com/feeds/videos.xml?channel_id={}", account.id)).await {
                    Ok(xml) => {
                        let named = xml.contains(&format!("<title>{}</title>", account.name));
                        match parse_youtube(&xml, account) {
                            Ok(found) if named => Ok(format!("channel {} \"{}\"; {} recent announcements", account.id, account.name, found.len())),
                            Ok(_) => Err(format!("channel title is no longer \"{}\"", account.name)),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            Platform::Bluesky => match get_text(client, &format!("{BLUESKY_APPVIEW}/app.bsky.actor.getProfile?actor={}", account.id)).await {
                Ok(json) => match serde_json::from_str::<BskyProfile>(&json) {
                    Ok(p) if p.did == account.id && p.handle == account.handle => {
                        let status = p
                            .verification
                            .as_ref()
                            .and_then(|v| v.get("verifiedStatus"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("none")
                            .to_string();
                        Ok(format!("{} = {}, Bluesky verification: {status}", p.handle, p.did))
                    }
                    Ok(p) => Err(format!("DID {} now has handle {}", p.did, p.handle)),
                    Err(e) => Err(e.to_string()),
                },
                Err(e) => Err(e),
            },
        };
        out.push(match result {
            Ok(detail) => AccountCheck { account: account.label(), ok: true, detail: format!("{detail} ({})", account.verification) },
            Err(detail) => AccountCheck { account: account.label(), ok: false, detail },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixar() -> OfficialAccount {
        OfficialAccount {
            organization: "Pixar".into(),
            region: "US".into(),
            language: "en".into(),
            platform: Platform::Youtube,
            id: "UC_IRYSp4auq7hKLvziWVH6w".into(),
            handle: "@Pixar".into(),
            name: "Pixar".into(),
            verification: "test".into(),
        }
    }

    const YT: &str = r#"<?xml version="1.0"?><feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns="http://www.w3.org/2005/Atom">
<yt:channelId>UC_IRYSp4auq7hKLvziWVH6w</yt:channelId><title>Pixar</title>
<entry><title>Toy Story 5 | Official Trailer</title><link rel="alternate" href="https://www.youtube.com/watch?v=abc"/>
<published>2026-10-16T17:00:14+00:00</published><media:group><media:title>Toy Story 5 | Official Trailer</media:title><media:description>Watch the new trailer.</media:description></media:group></entry>
<entry><title>Best Woody Moments Compilation</title><link rel="alternate" href="https://www.youtube.com/watch?v=def"/>
<published>2026-10-16T18:00:00+00:00</published></entry></feed>"#;

    #[test]
    fn built_in_accounts_are_pinned_and_skip_x() {
        let accounts = default_accounts();
        assert!(accounts.len() >= 15);
        assert!(accounts.iter().any(|a| a.organization == "Pixar" && a.platform == Platform::Youtube));
        assert!(accounts.iter().all(|a| !a.verification.is_empty()));
        assert!(!DEFAULT_ACCOUNTS.contains("x.com/") && !DEFAULT_ACCOUNTS.contains("twitter.com"));
        assert!(parse_accounts(r#"{"accounts":[{"organization":"Fake","region":"US","language":"en","platform":"youtube","id":"@fake","handle":"@fake","name":"Fake","verification":"none"}]}"#).is_err());
    }

    #[test]
    fn youtube_feed_keeps_announcements_from_the_pinned_channel_only() {
        let entries = parse_youtube(YT, &pixar()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Toy Story 5 | Official Trailer");
        assert_eq!(entries[0].summary, "Watch the new trailer.");
        assert_eq!(entries[0].official_for.as_deref(), Some("Pixar"));
        assert_eq!(entries[0].platform.as_deref(), Some("YouTube"));
        let mut impostor = pixar();
        impostor.id = "UCxxxxxxxxxxxxxxxxxxxxxx".into();
        assert!(parse_youtube(YT, &impostor).is_err());
    }

    #[test]
    fn bluesky_posts_must_come_from_the_pinned_did_and_handle() {
        let account = OfficialAccount {
            platform: Platform::Bluesky,
            id: "did:plc:abc".into(),
            handle: "crunchyroll.com".into(),
            organization: "Crunchyroll".into(),
            ..pixar()
        };
        let post = |did: &str, text: &str| {
            format!(
                r#"{{"feed":[{{"post":{{"uri":"at://{did}/app.bsky.feed.post/3k1","author":{{"did":"{did}","handle":"crunchyroll.com"}},"record":{{"text":"{text}","createdAt":"2026-10-18T14:00:00Z"}}}}}}]}}"#
            )
        };
        let ok = parse_bluesky(&post("did:plc:abc", "Watch the new trailer for Dan Da Dan Season 3! Premieres in January."), &account).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].link, "https://bsky.app/profile/crunchyroll.com/post/3k1");
        assert!(parse_bluesky(&post("did:plc:abc", "Yuno got serious (via Black Clover)"), &account).unwrap().is_empty());
        assert!(parse_bluesky(&post("did:plc:evil", "New trailer!"), &account).is_err());
    }

    #[test]
    fn announcement_filter_reads_japanese() {
        assert!(is_announcement("TVアニメ「呪術廻戦」第3期 本予告"));
        assert!(is_announcement("Spirited Away | Official Teaser"));
        assert!(!is_announcement("SpongeBob's Funniest Moments"));
        assert!(!is_announcement("Opening ceremony highlights"));
    }
}
