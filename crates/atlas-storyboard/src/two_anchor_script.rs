use crate::pronunciation::PronunciationDictionary;
use atlas_core::{AtlasError, AtlasResult, EpisodeDurationPolicy, SHOW_TITLE};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const WORDS_PER_SECOND: f64 = 2.6;
pub const LINE_PAUSE_SECONDS: f64 = 0.35;
pub const SEGMENT_CARD_SECONDS: f64 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorRole {
    Enthusiast,
    Counterpart,
    Duo,
}

impl AnchorRole {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Enthusiast => "Marcus (The Enthusiast)",
            Self::Counterpart => "Elena (The Sharp Counterpart)",
            Self::Duo => "Marcus & Elena",
        }
    }

    pub fn first_name(&self) -> &'static str {
        match self {
            Self::Enthusiast => "Marcus",
            Self::Counterpart => "Elena",
            Self::Duo => "Marcus & Elena",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        let v = value.trim().to_lowercase();
        if v.contains("marcus") && v.contains("elena") || v == "both" || v == "duo" {
            Some(Self::Duo)
        } else if v.contains("marcus") || v.contains("enthusiast") {
            Some(Self::Enthusiast)
        } else if v.contains("elena") || v.contains("counterpart") {
            Some(Self::Counterpart)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudioCameraCut {
    WideTwoShot,
    MarcusCloseUp,
    ElenaCloseUp,
    VideoWallCenter,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptDialogueLine {
    pub speaker: AnchorRole,
    pub camera_cut: StudioCameraCut,
    pub spoken_text: String,
    pub phonetic_text: String,
    pub expression: String,
    pub estimated_seconds: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewsSegmentScript {
    pub segment_number: usize,
    pub segment_title: String,
    pub segment_category: String,
    pub story_id: Option<String>,
    pub headline: String,
    pub attribution: String,
    pub target_duration_secs: f64,
    pub actual_duration_secs: f64,
    pub lines: Vec<ScriptDialogueLine>,
    #[serde(default)]
    pub short: Option<ShortClip>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShortClip {
    pub start_line: usize,
    pub end_line: usize,
    pub title: String,
    pub hook: String,
}

impl NewsSegmentScript {
    pub fn word_count(&self) -> usize {
        self.lines
            .iter()
            .map(|l| l.spoken_text.split_whitespace().count())
            .sum()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MasterEpisodeScript {
    pub show_title: String,
    pub episode_title: String,
    pub episode_date: String,
    pub target_duration_secs: f64,
    pub total_duration_secs: f64,
    pub segments: Vec<NewsSegmentScript>,
}

impl MasterEpisodeScript {
    pub fn validate_duration(&self, policy: &EpisodeDurationPolicy) -> AtlasResult<()> {
        policy
            .validate(self.total_duration_secs)
            .map_err(|e| AtlasError::Config(format!("Estimated script length: {e}")))
    }

    pub fn transcript(&self) -> String {
        self.segments
            .iter()
            .flat_map(|s| s.lines.iter())
            .map(|l| format!("{}: {}", l.speaker.first_name(), l.spoken_text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptStoryBrief {
    pub story_id: String,
    pub headline: String,
    pub category: String,
    pub organization: String,
    pub outlets: Vec<String>,
    pub fact_sheet: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BulletinBrief {
    pub headline: String,
    pub outlet: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawLine {
    pub speaker: String,
    pub text: String,
    #[serde(default)]
    pub expression: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawShort {
    pub start: usize,
    pub end: usize,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawReply {
    pub lines: Vec<RawLine>,
    #[serde(default)]
    pub short: Option<RawShort>,
}

pub const SHORT_MIN_WORDS: usize = 90;
pub const SHORT_MAX_WORDS: usize = 150;

const CONTEXT_OPENERS: &[&str] = &[
    "and", "but", "so", "that", "that's", "this", "it", "it's", "he", "she", "they", "exactly",
    "right", "yeah", "yes", "well", "also", "plus", "which", "those", "these", "true", "agreed",
    "absolutely", "totally", "okay", "ok", "oh",
];

const BANNED_PHRASES: &[&str] = &[
    "let's dive in",
    "dive into",
    "deep dive",
    "in today's video",
    "game-changer",
    "game changer",
    "buckle up",
    "without further ado",
    "smash that like",
    "stay tuned",
    "in conclusion",
    "it's worth noting",
    "the landscape of",
    "a testament to",
    "tapestry",
    "delve",
];

pub fn estimate_seconds(text: &str) -> f64 {
    text.split_whitespace().count() as f64 / WORDS_PER_SECOND + LINE_PAUSE_SECONDS
}

pub fn audit_facts(lines: &[ScriptDialogueLine], fact_sheet: &str) -> Vec<String> {
    let known: HashSet<String> = number_tokens(fact_sheet).into_iter().collect();
    let mut issues = Vec::new();
    for line in lines {
        for number in number_tokens(&line.spoken_text) {
            if !known.contains(&number) && !issues.iter().any(|i: &String| i.contains(&number)) {
                issues.push(format!(
                    "The figure \"{number}\" does not appear in the fact sheet; remove it or use only figures from the sources."
                ));
            }
        }
    }
    issues
}

fn number_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (i, c) in chars.iter().enumerate() {
        let joins_digits = matches!(c, ',' | '.')
            && !current.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_ascii_digit() {
            current.push(*c);
        } else if !joins_digits && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens.retain(|t| t.len() > 1);
    tokens
}

struct SegmentMeta<'a> {
    title: &'a str,
    category: &'a str,
    story_id: Option<String>,
    headline: &'a str,
    attribution: &'a str,
    target: f64,
}

pub struct TwoAnchorScriptWriter {
    pronunciation: PronunciationDictionary,
}

impl Default for TwoAnchorScriptWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl TwoAnchorScriptWriter {
    pub fn new() -> Self {
        Self {
            pronunciation: PronunciationDictionary::new(),
        }
    }

    pub fn line(&self, speaker: AnchorRole, spoken: &str, expression: &str, index: usize) -> ScriptDialogueLine {
        let camera_cut = if index == 0 || speaker == AnchorRole::Duo {
            StudioCameraCut::WideTwoShot
        } else if index % 5 == 4 {
            StudioCameraCut::VideoWallCenter
        } else if speaker == AnchorRole::Enthusiast {
            StudioCameraCut::MarcusCloseUp
        } else {
            StudioCameraCut::ElenaCloseUp
        };
        ScriptDialogueLine {
            speaker,
            camera_cut,
            spoken_text: spoken.trim().to_string(),
            phonetic_text: self.pronunciation.phonetically_prepare(spoken.trim()),
            expression: expression.to_string(),
            estimated_seconds: estimate_seconds(spoken),
        }
    }

    fn segment(&self, number: usize, meta: SegmentMeta<'_>, lines: Vec<ScriptDialogueLine>) -> NewsSegmentScript {
        let actual = SEGMENT_CARD_SECONDS + lines.iter().map(|l| l.estimated_seconds).sum::<f64>();
        NewsSegmentScript {
            segment_number: number,
            segment_title: meta.title.to_string(),
            segment_category: meta.category.to_string(),
            story_id: meta.story_id,
            headline: meta.headline.to_string(),
            attribution: meta.attribution.to_string(),
            target_duration_secs: meta.target,
            actual_duration_secs: actual,
            lines,
            short: None,
        }
    }

    pub fn append_tease(&self, segment: &mut NewsSegmentScript, next_headline: &str, variant: usize) {
        let headline = next_headline.trim_end_matches(['.', '!', '?']);
        let (speaker, text) = match variant % 3 {
            0 => (AnchorRole::Counterpart, format!("Still ahead: {headline}.")),
            1 => (AnchorRole::Enthusiast, format!("Stick around, because next up: {headline}.")),
            _ => (AnchorRole::Counterpart, format!("Coming up after this: {headline}.")),
        };
        let line = self.line(speaker, &text, "Tease", segment.lines.len());
        segment.actual_duration_secs += line.estimated_seconds;
        segment.lines.push(line);
    }

    pub fn story_word_budget(&self, story_count: usize, bulletin_count: usize, target_secs: f64) -> usize {
        let fixed_secs = 45.0 + 50.0 + if bulletin_count > 0 { 12.0 + bulletin_count as f64 * 7.0 } else { 0.0 };
        let cards = SEGMENT_CARD_SECONDS * (story_count + 3) as f64;
        let available = (target_secs - fixed_secs - cards).max(story_count as f64 * 120.0);
        let per_story_secs = available / story_count.max(1) as f64;
        ((per_story_secs - 12.0 * LINE_PAUSE_SECONDS) * WORDS_PER_SECOND).clamp(280.0, 1100.0) as usize
    }

    pub fn cold_open(
        &self,
        episode_number: u32,
        stories: &[ScriptStoryBrief],
        bulletin_count: usize,
        lead_hook: Option<&str>,
    ) -> NewsSegmentScript {
        let skip = usize::from(lead_hook.is_some());
        let headlines: Vec<&str> = stories.iter().skip(skip).take(3).map(|s| s.headline.as_str()).collect();
        let teaser = match headlines.len() {
            0 => "a packed rundown from across the animation world".to_string(),
            1 => headlines[0].to_string(),
            2 => format!("{}, and {}", headlines[0], headlines[1]),
            _ => format!("{}; {}; and {}", headlines[0], headlines[1], headlines[2]),
        };
        let remaining = stories.len().saturating_sub(3 + skip) + bulletin_count;
        let mut lines = Vec::new();
        if let Some(hook) = lead_hook {
            lines.push(self.line(AnchorRole::Enthusiast, hook, "Hook", 0));
            lines.push(self.line(
                AnchorRole::Counterpart,
                &format!("That's our lead. Welcome to {SHOW_TITLE}, episode {episode_number}. I'm Elena."),
                "SharpDirect",
                1,
            ));
            lines.push(self.line(
                AnchorRole::Enthusiast,
                &format!("I'm Marcus. Also on today's desk: {teaser}."),
                "WarmConfident",
                2,
            ));
        } else {
            lines.push(self.line(
                AnchorRole::Enthusiast,
                &format!("Welcome to {SHOW_TITLE}, episode {episode_number}. I'm Marcus."),
                "WarmConfident",
                0,
            ));
            lines.push(self.line(
                AnchorRole::Counterpart,
                &format!("And I'm Elena. On today's desk: {teaser}."),
                "SharpDirect",
                1,
            ));
        }
        if remaining > 0 {
            lines.push(self.line(
                AnchorRole::Enthusiast,
                &format!("Plus {remaining} more stories in our rapid-fire round. Every story today is sourced, and the links are in the description."),
                "Excited",
                lines.len(),
            ));
        }
        lines.push(self.line(
            AnchorRole::Counterpart,
            "Let's get into it.",
            "DeadpanDryWit",
            lines.len(),
        ));
        self.segment(
            1,
            SegmentMeta { title: "Cold Open", category: "Introduction", story_id: None, headline: SHOW_TITLE, attribution: "", target: 45.0 },
            lines,
        )
    }

    pub fn rapid_headlines(&self, number: usize, bulletins: &[BulletinBrief]) -> Option<NewsSegmentScript> {
        if bulletins.is_empty() {
            return None;
        }
        let mut lines = vec![self.line(
            AnchorRole::Counterpart,
            "Time for rapid-fire headlines. These are single-source reports, so we're telling you exactly who reported each one.",
            "FastPacedAlert",
            0,
        )];
        for (i, bulletin) in bulletins.iter().enumerate() {
            let speaker = if i % 2 == 0 { AnchorRole::Enthusiast } else { AnchorRole::Counterpart };
            let headline = bulletin.headline.trim_end_matches(['.', '!', '?']);
            lines.push(self.line(
                speaker,
                &format!("{} reports: {headline}.", bulletin.outlet),
                "PunchyDelivery",
                i + 1,
            ));
        }
        let attribution = dedup(bulletins.iter().map(|b| b.outlet.clone())).join(" · ");
        Some(self.segment(
            number,
            SegmentMeta {
                title: "Rapid-Fire Headlines",
                category: "Upcoming Releases",
                story_id: None,
                headline: "Rapid-Fire Headlines",
                attribution: &attribution,
                target: 12.0 + bulletins.len() as f64 * 7.0,
            },
            lines,
        ))
    }

    pub fn closing(&self, number: usize, stories: &[ScriptStoryBrief]) -> NewsSegmentScript {
        let question = match stories {
            [first, second, ..] => format!(
                "Tell us in the comments: which story are you following, {} or {}?",
                first.headline.trim_end_matches(['.', '!', '?']),
                second.headline.trim_end_matches(['.', '!', '?'])
            ),
            [only] => format!(
                "Tell us in the comments what you think about {}.",
                only.headline.trim_end_matches(['.', '!', '?'])
            ),
            [] => "Tell us in the comments what you're watching this week.".to_string(),
        };
        let lines = vec![
            self.line(AnchorRole::Counterpart, "That's the desk for today.", "ReflectiveClosing", 0),
            self.line(AnchorRole::Enthusiast, &question, "DirectEngagement", 1),
            self.line(
                AnchorRole::Counterpart,
                "A note on how we make this show: our anchors are voiced with AI text-to-speech, the script is written with AI assistance from the reporting linked in the description, and every story is checked against its sources before air.",
                "WarmProfessional",
                2,
            ),
            self.line(AnchorRole::Enthusiast, "Subscribe so you don't miss tomorrow's edition.", "PlayfulChallenge", 3),
            self.line(AnchorRole::Duo, "Until next time, keep drawing clearly.", "UnisonSignoff", 4),
        ];
        self.segment(
            number,
            SegmentMeta { title: "Sign-off & Credits", category: "Credits & Outro", story_id: None, headline: SHOW_TITLE, attribution: "", target: 50.0 },
            lines,
        )
    }

    pub fn story_segment(
        &self,
        number: usize,
        brief: &ScriptStoryBrief,
        raw_lines: &[RawLine],
        raw_short: Option<&RawShort>,
        target_secs: f64,
    ) -> AtlasResult<NewsSegmentScript> {
        let mut lines = Vec::new();
        let mut kept: Vec<Option<usize>> = Vec::with_capacity(raw_lines.len());
        for (index, raw) in raw_lines.iter().enumerate() {
            let speaker = AnchorRole::parse(&raw.speaker).ok_or_else(|| {
                AtlasError::Config(format!("Unknown speaker \"{}\" in story script", raw.speaker))
            })?;
            let text = raw.text.trim();
            if text.is_empty() {
                kept.push(None);
                continue;
            }
            kept.push(Some(lines.len()));
            lines.push(self.line(
                speaker,
                text,
                raw.expression.as_deref().unwrap_or("Engaged"),
                index,
            ));
        }
        let short = raw_short.and_then(|r| {
            let start = kept.get(r.start).copied().flatten()?;
            let end = kept.get(r.end).copied().flatten()?;
            (start <= end).then(|| ShortClip {
                start_line: start,
                end_line: end,
                title: r.title.trim().to_string(),
                hook: lines[start].spoken_text.clone(),
            })
        });
        let attribution = format!("Sources: {}", brief.outlets.join(" · "));
        let mut segment = self.segment(
            number,
            SegmentMeta {
                title: &brief.headline,
                category: &brief.category,
                story_id: Some(brief.story_id.clone()),
                headline: &brief.headline,
                attribution: &attribution,
                target: target_secs,
            },
            lines,
        );
        segment.short = short;
        Ok(segment)
    }

    pub fn assemble(&self, episode_title: &str, episode_date: &str, target_secs: f64, segments: Vec<NewsSegmentScript>) -> MasterEpisodeScript {
        let total_duration_secs = segments.iter().map(|s| s.actual_duration_secs).sum();
        MasterEpisodeScript {
            show_title: SHOW_TITLE.into(),
            episode_title: episode_title.into(),
            episode_date: episode_date.into(),
            target_duration_secs: target_secs,
            total_duration_secs,
            segments,
        }
    }
}

fn dedup(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

pub fn validate_story_lines(lines: &[ScriptDialogueLine], target_words: usize, fact_sheet: &str) -> Vec<String> {
    let mut issues = Vec::new();
    let words: usize = lines.iter().map(|l| l.spoken_text.split_whitespace().count()).sum();
    let min_words = target_words * 3 / 4;
    let max_words = target_words * 3 / 2;
    if words < min_words {
        issues.push(format!("The segment has {words} words; write at least {min_words} words (target {target_words})."));
    }
    if words > max_words {
        issues.push(format!("The segment has {words} words; keep it under {max_words} words (target {target_words})."));
    }
    if lines.len() < 6 {
        issues.push(format!("The segment has {} lines; write at least 6 alternating lines.", lines.len()));
    }
    if !lines.iter().any(|l| l.speaker == AnchorRole::Enthusiast) || !lines.iter().any(|l| l.speaker == AnchorRole::Counterpart) {
        issues.push("Both Marcus and Elena must speak in the segment.".into());
    }
    if let Some(long) = lines.iter().find(|l| l.spoken_text.split_whitespace().count() > 110) {
        issues.push(format!(
            "A line by {} is longer than 110 words; split long turns.",
            long.speaker.first_name()
        ));
    }
    if let Some(first) = lines.first()
        && first.spoken_text.split_whitespace().count() > 30
    {
        issues.push("The first line is the hook: make it one punchy sentence of at most 30 words.".into());
    }
    for line in lines {
        let lower = line.spoken_text.to_lowercase();
        if let Some(phrase) = BANNED_PHRASES.iter().find(|p| lower.contains(*p)) {
            issues.push(format!("Remove the stock phrase \"{phrase}\"; say something specific instead."));
        }
    }
    issues.extend(audit_facts(lines, fact_sheet));
    issues
}

pub fn validate_short(lines: &[ScriptDialogueLine], short: Option<&ShortClip>, fact_sheet: &str) -> Vec<String> {
    let Some(short) = short else {
        return vec!["Include the \"short\" object marking the best self-contained clip.".into()];
    };
    let mut issues = Vec::new();
    if short.end_line >= lines.len() || short.start_line > short.end_line {
        return vec![format!("The short range {}..{} is outside the {} lines.", short.start_line, short.end_line, lines.len())];
    }
    let span = &lines[short.start_line..=short.end_line];
    let words: usize = span.iter().map(|l| l.spoken_text.split_whitespace().count()).sum();
    if !(SHORT_MIN_WORDS..=SHORT_MAX_WORDS).contains(&words) {
        issues.push(format!(
            "The short spans {words} words; choose consecutive lines totalling {SHORT_MIN_WORDS} to {SHORT_MAX_WORDS} words (about 35 to 58 seconds)."
        ));
    }
    let first_word = short
        .hook
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_lowercase();
    if CONTEXT_OPENERS.contains(&first_word.as_str()) {
        issues.push(format!(
            "The short starts with \"{first_word}\", which needs earlier context; start it on a line that works for a viewer who saw nothing before."
        ));
    }
    let title_len = short.title.chars().count();
    if !(10..=70).contains(&title_len) {
        issues.push("The short title must be 10 to 70 characters.".into());
    }
    let known: HashSet<String> = number_tokens(fact_sheet).into_iter().collect();
    if number_tokens(&short.title).iter().any(|n| !known.contains(n)) {
        issues.push("The short title uses a figure that is not in the fact sheet.".into());
    }
    issues
}

pub struct NewsScriptDirector {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl NewsScriptDirector {
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var("ATLAS_DIRECTOR_URL")
            .unwrap_or_else(|_| "https://integrate.api.nvidia.com/v1/chat/completions".into());
        let api_key = std::env::var("ATLAS_DIRECTOR_API_KEY")
            .or_else(|_| std::env::var("NVIDIA_API_KEY"))
            .ok()?;
        let model = std::env::var("ATLAS_NEWS_SCRIPT_MODEL")
            .or_else(|_| std::env::var("ATLAS_DIRECTOR_MODEL"))
            .unwrap_or_else(|_| "meta/llama-3.3-70b-instruct".into());
        Some(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .unwrap_or_default(),
            endpoint,
            api_key,
            model,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn write_story_segment(
        &self,
        writer: &TwoAnchorScriptWriter,
        number: usize,
        brief: &ScriptStoryBrief,
        target_words: usize,
    ) -> AtlasResult<NewsSegmentScript> {
        let target_secs = target_words as f64 / WORDS_PER_SECOND;
        let mut feedback: Vec<String> = Vec::new();
        let mut last_error = String::new();
        for _attempt in 0..3 {
            let prompt = story_prompt(brief, target_words, &feedback);
            let content = match self.complete(&prompt).await {
                Ok(content) => content,
                Err(error) => {
                    last_error = error.to_string();
                    continue;
                }
            };
            let reply = match parse_reply(&content) {
                Ok(reply) => reply,
                Err(error) => {
                    last_error = error.clone();
                    feedback = vec![format!("Your previous reply was not valid JSON in the required shape: {error}")];
                    continue;
                }
            };
            let segment = match writer.story_segment(number, brief, &reply.lines, reply.short.as_ref(), target_secs) {
                Ok(segment) => segment,
                Err(error) => {
                    last_error = error.to_string();
                    feedback = vec![error.to_string()];
                    continue;
                }
            };
            let mut issues = validate_story_lines(&segment.lines, target_words, &brief.fact_sheet);
            issues.extend(validate_short(&segment.lines, segment.short.as_ref(), &brief.fact_sheet));
            if issues.is_empty() {
                return Ok(segment);
            }
            last_error = issues.join(" ");
            feedback = issues;
        }
        Err(AtlasError::Config(format!(
            "Script for \"{}\" failed validation after 3 attempts: {last_error}",
            brief.headline
        )))
    }

    async fn complete(&self, prompt: &str) -> AtlasResult<String> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "temperature": 0.7,
                "max_tokens": 4096,
                "response_format": {"type": "json_object"},
                "messages": [
                    {"role": "system", "content": SYSTEM_PROMPT},
                    {"role": "user", "content": prompt}
                ]
            }))
            .send()
            .await
            .map_err(|e| AtlasError::Config(format!("Script director request failed: {e}")))?;
        let status = response.status();
        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AtlasError::Config(format!("Script director returned invalid JSON: {e}")))?;
        if !status.is_success() {
            return Err(AtlasError::Config(format!("Script director returned {status}: {payload}")));
        }
        payload["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| AtlasError::Config("Script director response omitted message content".into()))
    }
}

const SYSTEM_PROMPT: &str = "You are the head writer of Clearly Drawn News, a daily two-anchor animation news show built for retention on YouTube. Marcus is the Enthusiast: energetic, warm, an animation-history nerd who notices craft details like layouts, timing, compositing and staff. Elena is the Sharp Counterpart: witty, deadpan, skeptical, and focused on what is actually confirmed versus hoped for. Their chemistry comes from friendly disagreement, not agreement. You write natural spoken dialogue for text-to-speech: contractions, short punchy sentences mixed with longer ones, no tongue-twisters, no parentheses. You never invent facts, dates, numbers, names, quotes, or claims that are not in the provided fact sheet. When the sources leave something unknown, the anchors say it is not yet confirmed. Return valid JSON only.";

fn story_prompt(brief: &ScriptStoryBrief, target_words: usize, feedback: &[String]) -> String {
    let mut prompt = format!(
        "Write the dialogue for one story segment.\n\nFACT SHEET (the only facts you may use):\n{facts}\n\nSTRUCTURE (in this order):\n1. HOOK: line 1 is one punchy sentence of at most 30 words that states the news with a curiosity gap. It must make sense to someone who saw nothing before it.\n2. THE NEWS: what happened, plainly, attributed to {outlets}.\n3. WHY IT MATTERS: for fans, and for the craft or the industry.\n4. CRAFT ANGLE: Marcus brings one specific, widely known, non-controversial background detail about the studio, staff or franchise. Never present background as part of this news.\n5. REALITY CHECK: Elena separates what is confirmed from what is not.\n6. THE DISAGREEMENT: a short, good-natured clash of takes that viewers will want to weigh in on.\n7. PAYOFF: end with a direct question to viewers that invites a comment.\n\nRULES:\n- About {target_words} spoken words in total, in 10 to 26 alternating lines. Vary line length: some lines under 10 words, a few up to 60.\n- Keep at least 80% of the words on the story; banter must be about the story.\n- Do not quote more than ten consecutive words from any source; paraphrase.\n- Write numbers as digits only when they appear in the fact sheet.\n- No stock filler (for example: let's dive in, game-changer, buckle up, stay tuned, a testament to).\n- No stage directions, markdown, emojis, hashtags or speaker names inside the text.\n\nSHORT CLIP: also pick the best self-contained clip for a vertical Short: consecutive lines totalling {min_short} to {max_short} words (about 35 to 58 seconds) that start on a line that works with zero context (not on And, But, So, That, It, Exactly, Yeah and so on), contain a complete idea, and end on a strong line. Give it a title of 10 to 70 characters that is accurate, specific and not clickbait.\n\nReturn JSON exactly in this shape: {{\"lines\": [{{\"speaker\": \"Marcus\" or \"Elena\", \"text\": \"...\", \"expression\": \"one or two words\"}}], \"short\": {{\"start\": first line index (0-based), \"end\": last line index (inclusive), \"title\": \"...\"}}}}",
        facts = brief.fact_sheet,
        outlets = brief.outlets.join(", "),
        min_short = SHORT_MIN_WORDS,
        max_short = SHORT_MAX_WORDS,
    );
    if !feedback.is_empty() {
        prompt.push_str("\n\nYOUR PREVIOUS DRAFT WAS REJECTED. Fix all of these problems:\n- ");
        prompt.push_str(&feedback.join("\n- "));
    }
    prompt
}

pub fn parse_reply(content: &str) -> Result<RawReply, String> {
    let trimmed = content.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|v| v.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    let start = body.find(['{', '[']).ok_or("no JSON object found")?;
    let value: serde_json::Value = serde_json::from_str(&body[start..]).map_err(|e| e.to_string())?;
    let reply = if value.is_array() {
        RawReply {
            lines: serde_json::from_value(value).map_err(|e| e.to_string())?,
            short: None,
        }
    } else {
        if value.get("lines").is_none() {
            return Err("missing \"lines\" array".into());
        }
        serde_json::from_value(value).map_err(|e| e.to_string())?
    };
    if reply.lines.is_empty() {
        return Err("\"lines\" is empty".into());
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brief() -> ScriptStoryBrief {
        ScriptStoryBrief {
            story_id: "csm".into(),
            headline: "MAPPA Sets Chainsaw Man Season 2 for July 2027".into(),
            category: "Anime".into(),
            organization: "MAPPA".into(),
            outlets: vec!["Anime News Network".into(), "Anime Corner".into()],
            fact_sheet: "Headline: MAPPA Sets Chainsaw Man Season 2 for July 2027\nSummary: The season premieres in July 2027 with 12 episodes.".into(),
        }
    }

    #[test]
    fn builds_story_segment_from_llm_lines_and_audits_numbers() {
        let writer = TwoAnchorScriptWriter::new();
        let json = r#"{"lines":[{"speaker":"Marcus","text":"MAPPA says the season lands in July 2027."},{"speaker":"Elena","text":"With 12 episodes, and nothing else confirmed."},{"speaker":"Marcus","text":"It sold 40 million copies!"}]}"#;
        let reply = parse_reply(json).unwrap();
        let segment = writer.story_segment(2, &brief(), &reply.lines, reply.short.as_ref(), 200.0).unwrap();
        assert_eq!(segment.lines.len(), 3);
        assert_eq!(segment.lines[0].camera_cut, StudioCameraCut::WideTwoShot);
        let issues = validate_story_lines(&segment.lines, 40, &brief().fact_sheet);
        assert!(issues.iter().any(|i| i.contains("\"40\"")));
        assert!(issues.iter().any(|i| i.contains("at least 6")));
        assert!(!issues.iter().any(|i| i.contains("2027")));
    }

    #[test]
    fn fixed_segments_use_real_headlines() {
        let writer = TwoAnchorScriptWriter::new();
        let open = writer.cold_open(7, &[brief()], 2, None);
        assert!(open.lines[1].spoken_text.contains("Chainsaw Man"));
        let hooked = writer.cold_open(7, &[brief(), brief()], 0, Some("MAPPA just put a date on it."));
        assert_eq!(hooked.lines[0].spoken_text, "MAPPA just put a date on it.");
        assert!(hooked.lines[1].spoken_text.contains("episode 7"));
        let rapid = writer
            .rapid_headlines(5, &[BulletinBrief { headline: "Pixar dates sequel".into(), outlet: "Cartoon Brew".into() }])
            .unwrap();
        assert_eq!(rapid.lines[1].spoken_text, "Cartoon Brew reports: Pixar dates sequel.");
        assert!(writer.rapid_headlines(5, &[]).is_none());
    }

    fn long_reply(short_start: usize, first_text: &str) -> String {
        let filler = "The craft details here are worth slowing down for because the staff list matters a lot.";
        let mut lines = vec![serde_json::json!({"speaker": "Marcus", "text": first_text})];
        for i in 0..9 {
            lines.push(serde_json::json!({"speaker": if i % 2 == 0 { "Elena" } else { "Marcus" }, "text": filler}));
        }
        serde_json::json!({"lines": lines, "short": {"start": short_start, "end": short_start + 5, "title": "Chainsaw Man Season 2 has a date"}}).to_string()
    }

    #[test]
    fn short_clip_must_be_self_contained_and_sized() {
        let writer = TwoAnchorScriptWriter::new();
        let reply = parse_reply(&long_reply(0, "MAPPA just dated Chainsaw Man Season 2 for July 2027, and the details matter.")).unwrap();
        let segment = writer.story_segment(2, &brief(), &reply.lines, reply.short.as_ref(), 200.0).unwrap();
        let short = segment.short.as_ref().unwrap();
        assert_eq!(short.hook, segment.lines[0].spoken_text);
        assert!(validate_short(&segment.lines, Some(short), &brief().fact_sheet).is_empty());

        let reply = parse_reply(&long_reply(0, "And that is why it matters for the studio this year.")).unwrap();
        let segment = writer.story_segment(2, &brief(), &reply.lines, reply.short.as_ref(), 200.0).unwrap();
        let issues = validate_short(&segment.lines, segment.short.as_ref(), &brief().fact_sheet);
        assert!(issues.iter().any(|i| i.contains("needs earlier context")));

        assert!(validate_short(&segment.lines, None, "")[0].contains("short"));
    }

    #[test]
    fn banned_phrases_and_long_hooks_are_rejected() {
        let writer = TwoAnchorScriptWriter::new();
        let lines = vec![
            writer.line(AnchorRole::Enthusiast, &"word ".repeat(35), "x", 0),
            writer.line(AnchorRole::Counterpart, "Let's dive in to the details.", "x", 1),
        ];
        let issues = validate_story_lines(&lines, 40, "");
        assert!(issues.iter().any(|i| i.contains("hook")));
        assert!(issues.iter().any(|i| i.contains("let's dive in")));
    }

    #[test]
    fn tease_extends_segment_duration() {
        let writer = TwoAnchorScriptWriter::new();
        let reply = parse_reply(&long_reply(0, "MAPPA dated it.")).unwrap();
        let mut segment = writer.story_segment(2, &brief(), &reply.lines, None, 200.0).unwrap();
        let before = segment.actual_duration_secs;
        writer.append_tease(&mut segment, "Pixar dates Hoppers 2", 0);
        assert_eq!(segment.lines.last().unwrap().spoken_text, "Still ahead: Pixar dates Hoppers 2.");
        assert!(segment.actual_duration_secs > before);
    }

    #[test]
    fn word_budget_scales_with_story_count() {
        let writer = TwoAnchorScriptWriter::new();
        let four = writer.story_word_budget(4, 6, 1320.0);
        let six = writer.story_word_budget(6, 6, 1320.0);
        assert!(four > six);
        let total_secs = 4.0 * four as f64 / WORDS_PER_SECOND + 45.0 + 50.0 + 54.0;
        assert!(EpisodeDurationPolicy::default().is_valid(total_secs));
    }

    #[test]
    fn number_tokens_handle_separators() {
        assert_eq!(number_tokens("It earned 1,200,000 in 2026."), vec!["1200000", "2026"]);
        assert!(number_tokens("Season 2 airs").is_empty());
    }
}
