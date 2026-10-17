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
        }
    }

    pub fn story_word_budget(&self, story_count: usize, bulletin_count: usize, target_secs: f64) -> usize {
        let fixed_secs = 45.0 + 50.0 + if bulletin_count > 0 { 12.0 + bulletin_count as f64 * 7.0 } else { 0.0 };
        let cards = SEGMENT_CARD_SECONDS * (story_count + 3) as f64;
        let available = (target_secs - fixed_secs - cards).max(story_count as f64 * 120.0);
        let per_story_secs = available / story_count.max(1) as f64;
        ((per_story_secs - 12.0 * LINE_PAUSE_SECONDS) * WORDS_PER_SECOND).clamp(280.0, 1100.0) as usize
    }

    pub fn cold_open(&self, episode_number: u32, stories: &[ScriptStoryBrief], bulletin_count: usize) -> NewsSegmentScript {
        let headlines: Vec<&str> = stories.iter().take(3).map(|s| s.headline.as_str()).collect();
        let teaser = match headlines.len() {
            0 => "a packed rundown from across the animation world".to_string(),
            1 => headlines[0].to_string(),
            2 => format!("{}, and {}", headlines[0], headlines[1]),
            _ => format!("{}; {}; and {}", headlines[0], headlines[1], headlines[2]),
        };
        let remaining = stories.len().saturating_sub(3) + bulletin_count;
        let mut lines = vec![
            self.line(
                AnchorRole::Enthusiast,
                &format!("Welcome to {SHOW_TITLE}, episode {episode_number}. I'm Marcus."),
                "WarmConfident",
                0,
            ),
            self.line(
                AnchorRole::Counterpart,
                &format!("And I'm Elena. On today's desk: {teaser}."),
                "SharpDirect",
                1,
            ),
        ];
        if remaining > 0 {
            lines.push(self.line(
                AnchorRole::Enthusiast,
                &format!("Plus {remaining} more stories in our rapid-fire round. Every story today is sourced, and the links are in the description."),
                "Excited",
                2,
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
        target_secs: f64,
    ) -> AtlasResult<NewsSegmentScript> {
        let mut lines = Vec::new();
        for (index, raw) in raw_lines.iter().enumerate() {
            let speaker = AnchorRole::parse(&raw.speaker).ok_or_else(|| {
                AtlasError::Config(format!("Unknown speaker \"{}\" in story script", raw.speaker))
            })?;
            let text = raw.text.trim();
            if text.is_empty() {
                continue;
            }
            lines.push(self.line(
                speaker,
                text,
                raw.expression.as_deref().unwrap_or("Engaged"),
                index,
            ));
        }
        let attribution = format!("Sources: {}", brief.outlets.join(" · "));
        Ok(self.segment(
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
        ))
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
    issues.extend(audit_facts(lines, fact_sheet));
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
            let raw = match parse_lines(&content) {
                Ok(raw) => raw,
                Err(error) => {
                    last_error = error.clone();
                    feedback = vec![format!("Your previous reply was not valid JSON in the required shape: {error}")];
                    continue;
                }
            };
            let segment = match writer.story_segment(number, brief, &raw, target_secs) {
                Ok(segment) => segment,
                Err(error) => {
                    last_error = error.to_string();
                    feedback = vec![error.to_string()];
                    continue;
                }
            };
            let issues = validate_story_lines(&segment.lines, target_words, &brief.fact_sheet);
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

const SYSTEM_PROMPT: &str = "You are the head writer of Clearly Drawn News, a daily two-anchor animation news show. Marcus is the Enthusiast: energetic, warm, an animation-history nerd who loves craft details. Elena is the Sharp Counterpart: witty, deadpan, skeptical, and focused on what is actually confirmed. You write natural spoken dialogue for text-to-speech. You never invent facts, dates, numbers, names, quotes, or claims that are not in the provided fact sheet. When the sources leave something unknown, the anchors say it is not yet confirmed. Return valid JSON only.";

fn story_prompt(brief: &ScriptStoryBrief, target_words: usize, feedback: &[String]) -> String {
    let mut prompt = format!(
        "Write the dialogue for one story segment.\n\nFACT SHEET (the only facts you may use):\n{}\n\nREQUIREMENTS:\n- About {target_words} spoken words in total, in 8 to 24 alternating lines between Marcus and Elena.\n- Open by stating the news plainly and attributing it to the reporting outlet(s): {}.\n- Explain why it matters for animation fans and for the craft or industry, using only the fact sheet; you may add widely known, non-controversial background about the franchise or studio but no new claims about this news.\n- Include light banter that fits the characters, but keep at least 80% of the words on the story.\n- Do not quote more than ten consecutive words from any source; paraphrase.\n- Write numbers as digits only when they appear in the fact sheet.\n- No stage directions, no markdown, no emojis, no speaker names inside the text.\n\nReturn JSON exactly in this shape: {{\"lines\": [{{\"speaker\": \"Marcus\" or \"Elena\", \"text\": \"...\", \"expression\": \"one or two words\"}}]}}",
        brief.fact_sheet,
        brief.outlets.join(", ")
    );
    if !feedback.is_empty() {
        prompt.push_str("\n\nYOUR PREVIOUS DRAFT WAS REJECTED. Fix all of these problems:\n- ");
        prompt.push_str(&feedback.join("\n- "));
    }
    prompt
}

pub fn parse_lines(content: &str) -> Result<Vec<RawLine>, String> {
    let trimmed = content.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|v| v.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    let start = body.find(['{', '[']).ok_or("no JSON object found")?;
    let value: serde_json::Value = serde_json::from_str(&body[start..]).map_err(|e| e.to_string())?;
    let array = value
        .get("lines")
        .cloned()
        .or_else(|| value.is_array().then(|| value.clone()))
        .ok_or("missing \"lines\" array")?;
    let lines: Vec<RawLine> = serde_json::from_value(array).map_err(|e| e.to_string())?;
    if lines.is_empty() {
        return Err("\"lines\" is empty".into());
    }
    Ok(lines)
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
        let raw = parse_lines(json).unwrap();
        let segment = writer.story_segment(2, &brief(), &raw, 200.0).unwrap();
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
        let open = writer.cold_open(7, &[brief()], 2);
        assert!(open.lines[1].spoken_text.contains("Chainsaw Man"));
        let rapid = writer
            .rapid_headlines(5, &[BulletinBrief { headline: "Pixar dates sequel".into(), outlet: "Cartoon Brew".into() }])
            .unwrap();
        assert_eq!(rapid.lines[1].spoken_text, "Cartoon Brew reports: Pixar dates sequel.");
        assert!(writer.rapid_headlines(5, &[]).is_none());
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
