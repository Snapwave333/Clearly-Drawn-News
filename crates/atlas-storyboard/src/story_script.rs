use crate::DirectorClient;
use crate::pronunciation::PronunciationDictionary;
use crate::two_anchor_script::{
    AnchorRole, MasterEpisodeScript, NewsSegmentScript, ScriptDialogueLine, StudioCameraCut,
};
use atlas_core::{AtlasError, AtlasResult, EpisodeDurationPolicy};
use serde::{Deserialize, Serialize};

const WORDS_PER_SECOND: f64 = 2.6;
const LINE_PAUSE_SECONDS: f64 = 0.5;
const MIN_WORDS_PER_STORY: usize = 180;
const MAX_WORDS_PER_STORY: usize = 900;
const BOOKENDS_SECONDS: f64 = 75.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoryBrief {
    pub headline: String,
    pub summary: String,
    pub category: String,
    pub organization: String,
    /// On-air credits, one per independent source ("Natalie, Japan", "Pixar's official YouTube channel").
    pub outlets: Vec<String>,
    /// Region tag of the story ("US", "Japan", "Korea", ...).
    #[serde(default)]
    pub region: String,
    /// Set when the studio's own verified account announced it:
    /// "Pixar announced it on its official YouTube channel".
    #[serde(default)]
    pub official_announcement: Option<String>,
    /// Headline as originally published, when the story was translated.
    #[serde(default)]
    pub original_headline: Option<String>,
    /// Set for animation-history retrospectives (never breaking news).
    #[serde(default)]
    pub retrospective: Option<RetroBrief>,
}

/// On-air framing and sourcing for a retrospective segment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetroBrief {
    /// "This Week in Animation History", "Studio Deep Dive", "Thursday Throwback Warm-Up", ...
    pub framing: String,
    /// Verified anniversary wording, e.g. "released October 18, 1967".
    pub anniversary: Option<String>,
    /// Each fact is confirmed by Wikipedia and at least one other reputable source.
    pub facts: Vec<String>,
    /// Source names read on air ("Wikipedia", "the Library of Congress", ...).
    pub sources: Vec<String>,
}

impl StoryBrief {
    pub fn is_retrospective(&self) -> bool {
        self.retrospective.is_some()
    }
}

/// Words per segment: news first, then retrospectives fill the rest of the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentWords {
    pub news: usize,
    pub retro: usize,
}

pub const NEWS_WORDS_WITH_RETROS: usize = 520;
pub const RETRO_WORDS: usize = 600;
pub const MAX_RETROS: usize = 8;

pub fn segment_words(news: usize, retros: usize, policy: &EpisodeDurationPolicy) -> SegmentWords {
    if retros == 0 {
        SegmentWords { news: words_budget(news, policy), retro: RETRO_WORDS }
    } else {
        SegmentWords { news: NEWS_WORDS_WITH_RETROS, retro: RETRO_WORDS }
    }
}

/// Retrospectives needed so news plus retros reach the fill target (the hold floor plus 3 minutes,
/// a margin for segments that come in short, capped at the target). News always goes first;
/// `feature_seconds` is the planned length of a feature segment (Debate Corner, cooking) that
/// airs between the news and the retrospectives.
pub fn retros_needed(news: usize, policy: &EpisodeDurationPolicy) -> usize {
    retros_needed_with_feature(news, 0.0, policy)
}

pub fn retros_needed_with_feature(news: usize, feature_seconds: f64, policy: &EpisodeDurationPolicy) -> usize {
    let fill_seconds = (policy.min_seconds + 180.0).min(policy.target_seconds);
    let news_seconds = news as f64 * (NEWS_WORDS_WITH_RETROS as f64 / WORDS_PER_SECOND);
    let gap = fill_seconds - BOOKENDS_SECONDS - news_seconds - feature_seconds;
    if gap <= 0.0 {
        return 0;
    }
    ((gap / (RETRO_WORDS as f64 / WORDS_PER_SECOND)).ceil() as usize).min(MAX_RETROS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptMethod {
    Director,
    Template,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DraftedEpisode {
    pub script: MasterEpisodeScript,
    pub segment_methods: Vec<ScriptMethod>,
    pub director_errors: Vec<String>,
}

#[derive(Deserialize)]
struct DirectorSegment {
    lines: Vec<DirectorLine>,
}

#[derive(Deserialize)]
struct DirectorLine {
    speaker: String,
    text: String,
    #[serde(default)]
    expression: Option<String>,
    #[serde(default)]
    broll: Option<String>,
}

struct Beat {
    speaker: AnchorRole,
    text: String,
    expression: String,
    broll: Option<String>,
}

pub fn estimate_seconds(text: &str) -> f64 {
    text.split_whitespace().count() as f64 / WORDS_PER_SECOND + LINE_PAUSE_SECONDS
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn facts(summary: &str) -> String {
    match summary.rfind(" (Source:") {
        Some(index) => summary[..index].trim().to_string(),
        None => summary.trim().to_string(),
    }
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') && current.trim().len() > 12 {
            out.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn subject(brief: &StoryBrief) -> String {
    if brief.organization.is_empty() {
        "this one".to_string()
    } else {
        brief.organization.clone()
    }
}

fn reaction(category: &str, org: &str) -> String {
    match category {
        "Anime" => format!("Okay, this is the kind of news that makes the whole fandom sit up. When {org} moves, people pay attention."),
        "Western Cartoons" => format!("I love this. Western animation needs more swings like this, and {org} knows how to land a character."),
        "Adult Animation" => "Adult animation keeps proving it can be bold and still find a huge audience, and this is another example.".to_string(),
        "Industry & Production" => "This is the behind-the-scenes stuff that decides what actually ends up on our screens.".to_string(),
        _ => "Release news is my favorite news. A date on the calendar makes it real.".to_string(),
    }
}

fn implication(category: &str, org: &str) -> String {
    match category {
        "Anime" => format!("If {org} gets the schedule right, the animators get the time to do their best work, and that always shows on screen."),
        "Western Cartoons" => "Shows like this build the next generation of animation fans, and that matters for every studio.".to_string(),
        "Adult Animation" => format!("For {org}, a hit here opens the door to even more ambitious projects."),
        "Industry & Production" => "For artists, decisions like this shape where the jobs are and how much time they get to do the work.".to_string(),
        _ => format!("And for {org}, the countdown is officially on."),
    }
}

fn counterpoint(category: &str) -> &'static str {
    match category {
        "Anime" => "The question I'd ask is whether the production schedule is realistic. Great announcements don't always mean healthy productions.",
        "Western Cartoons" => "The real test is whether it finds its audience, or gets buried by the algorithm in a crowded streaming lineup.",
        "Adult Animation" => "Bold is good. Bold and coherent is better. We'll see which one this turns out to be.",
        "Industry & Production" => "And we should be clear-eyed about the business side. Not every headline is good news for the people doing the drawing.",
        _ => "I'll believe the date when it doesn't move. Release windows have a way of sliding.",
    }
}

fn sourcing(brief: &StoryBrief) -> String {
    if let Some(official) = &brief.official_announcement {
        let outlets: Vec<&String> = brief.outlets.iter().filter(|o| !o.contains("'s official ")).collect();
        return match outlets.first() {
            Some(outlet) => format!("{official}, and {outlet} reported it too."),
            None => format!("{official}, so this one comes straight from the source."),
        };
    }
    match brief.outlets.as_slice() {
        [] => "We're still confirming the original source on this one, so treat the details as early.".to_string(),
        [only] => format!("For now this comes from a single outlet, via {only}, so we're treating the details as early."),
        [first, second, ..] => format!("And this one is confirmed by two outlets, via {first} and via {second}."),
    }
}

fn lead_in(index: usize, category: &str) -> String {
    match index {
        0 => "Our top story tonight:".to_string(),
        1 => "Next up:".to_string(),
        _ if index % 2 == 0 => format!("Turning to {}:", category.to_lowercase()),
        _ => "Also making news:".to_string(),
    }
}

fn template_story_beats(index: usize, brief: &StoryBrief) -> Vec<Beat> {
    let org = subject(brief);
    let mut fact_sentences = sentences(&facts(&brief.summary));
    if fact_sentences.is_empty() {
        fact_sentences.push(format!("{} is the headline, and the details are still coming in.", brief.headline));
    }
    let first_fact = fact_sentences.remove(0);
    let remaining = if fact_sentences.is_empty() {
        "That's what has been confirmed so far.".to_string()
    } else {
        fact_sentences.join(" ")
    };
    let headline = brief.headline.trim_end_matches('.');
    vec![
        Beat { speaker: AnchorRole::Enthusiast, text: format!("{} {headline}.", lead_in(index, &brief.category)), expression: "AuthoritativePassion".into(), broll: Some(format!("Video wall: {headline}")) },
        Beat { speaker: AnchorRole::Counterpart, text: first_fact, expression: "ClearJournalistic".into(), broll: None },
        Beat { speaker: AnchorRole::Enthusiast, text: reaction(&brief.category, &org), expression: "IntenseExuberance".into(), broll: None },
        Beat { speaker: AnchorRole::Counterpart, text: remaining, expression: "NeutralObservant".into(), broll: Some("Lower third: key facts".into()) },
        Beat { speaker: AnchorRole::Counterpart, text: sourcing(brief), expression: "SeriousCritical".into(), broll: None },
        Beat { speaker: AnchorRole::Enthusiast, text: implication(&brief.category, &org), expression: "PassionateExpertise".into(), broll: None },
        Beat { speaker: AnchorRole::Counterpart, text: counterpoint(&brief.category).into(), expression: "DryWit".into(), broll: None },
        Beat { speaker: AnchorRole::Enthusiast, text: format!("Fair. We'll keep an eye on {org} and bring you the update when there is one."), expression: "WarmConfident".into(), broll: None },
    ]
}

fn template_retro_beats(brief: &StoryBrief) -> Vec<Beat> {
    let retro = brief.retrospective.as_ref().expect("retro brief");
    let title = brief.headline.trim_end_matches('.');
    let mut beats = vec![
        Beat { speaker: AnchorRole::Enthusiast, text: format!("And now, something that is not breaking news at all. It's time for {}: {title}.", retro.framing), expression: "PassionateExpertise".into(), broll: Some(format!("Video wall: {} - {title}", retro.framing)) },
        Beat { speaker: AnchorRole::Counterpart, text: match &retro.anniversary { Some(when) => format!("A look back, since it was {when}."), None => "A look back, for everyone who wasn't there the first time.".into() }, expression: "DryWit".into(), broll: None },
    ];
    for (index, fact) in retro.facts.iter().take(8).enumerate() {
        let speaker = if index % 2 == 0 { AnchorRole::Enthusiast } else { AnchorRole::Counterpart };
        beats.push(Beat { speaker, text: fact.clone(), expression: "ClearJournalistic".into(), broll: None });
    }
    beats.push(Beat { speaker: AnchorRole::Counterpart, text: format!("Those details come from {}.", join_names(&retro.sources)), expression: "SeriousCritical".into(), broll: Some("Lower third: retrospective sources".into()) });
    beats.push(Beat { speaker: AnchorRole::Enthusiast, text: "History class dismissed. Back to the desk.".into(), expression: "WarmConfident".into(), broll: None });
    beats
}

fn join_names(names: &[String]) -> String {
    match names {
        [] => "published sources".into(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

fn cold_open_beats(all_briefs: &[StoryBrief]) -> Vec<Beat> {
    let briefs: Vec<StoryBrief> = all_briefs.iter().filter(|b| !b.is_retrospective()).cloned().collect();
    let retros: Vec<&StoryBrief> = all_briefs.iter().filter(|b| b.is_retrospective()).collect();
    let teasers = briefs
        .iter()
        .take(3)
        .map(|b| b.headline.trim_end_matches('.').to_string())
        .collect::<Vec<_>>();
    let tease = match teasers.as_slice() {
        [] => "the latest from across the animation world".to_string(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first}, plus {}", rest.join(", and ")),
    };
    vec![
        Beat { speaker: AnchorRole::Enthusiast, text: format!("Welcome to {}. I'm Marcus.", atlas_core::SHOW_TITLE), expression: "WarmConfident".into(), broll: None },
        Beat { speaker: AnchorRole::Counterpart, text: "And I'm Elena. We read the animation news so you don't have to, and we check it twice.".into(), expression: "DryWit".into(), broll: None },
        Beat { speaker: AnchorRole::Enthusiast, text: format!("Tonight: {tease}."), expression: "IntenseExuberance".into(), broll: Some("Video wall: episode rundown".into()) },
        Beat {
            speaker: AnchorRole::Counterpart,
            text: match (briefs.len(), retros.len()) {
                (1, 0) => "One story, sourced and checked. Let's get into it.".to_string(),
                (n, 0) => format!("{n} stories, every one sourced. Let's get into it."),
                (n, r) => format!(
                    "{n} {}, every one sourced, and then {} from animation history: {}. Let's get into it.",
                    if n == 1 { "news story" } else { "news stories" },
                    if r == 1 { "a look back".to_string() } else { format!("{r} looks back") },
                    join_names(&retros.iter().map(|b| b.headline.clone()).collect::<Vec<_>>())
                ),
            },
            expression: "ClearJournalistic".into(),
            broll: None,
        },
    ]
}

fn closing_beats() -> Vec<Beat> {
    vec![
        Beat { speaker: AnchorRole::Enthusiast, text: format!("That's the show. Thanks for spending part of your day with {}.", atlas_core::SHOW_TITLE), expression: "WarmConfident".into(), broll: None },
        Beat { speaker: AnchorRole::Counterpart, text: "Every source we used tonight is linked in the description, so you can check our work.".into(), expression: "ClearJournalistic".into(), broll: Some("End card: sources".into()) },
        Beat { speaker: AnchorRole::Enthusiast, text: format!("This program was produced by {} with AI-assisted writing, voices, and animation, reviewed against published reporting.", atlas_core::COMPANY_NAME), expression: "NeutralObservant".into(), broll: Some("Transparency disclosure card".into()) },
        Beat { speaker: AnchorRole::Duo, text: "See you next time.".into(), expression: "Smile".into(), broll: None },
    ]
}

fn camera_for(speaker: AnchorRole, position: usize, broll: bool) -> StudioCameraCut {
    match (speaker, position, broll) {
        (_, 0, _) | (AnchorRole::Duo, _, _) => StudioCameraCut::WideTwoShot,
        (_, _, true) => StudioCameraCut::SplitScreenAnalysis,
        (AnchorRole::Enthusiast, _, _) => StudioCameraCut::MarcusCloseUp,
        (AnchorRole::Counterpart, _, _) => StudioCameraCut::ElenaCloseUp,
    }
}

pub struct NewsroomWriter {
    pronunciation: PronunciationDictionary,
}

impl Default for NewsroomWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl NewsroomWriter {
    pub fn new() -> Self {
        Self {
            pronunciation: PronunciationDictionary::new(),
        }
    }

    fn segment(&self, number: usize, title: &str, category: &str, beats: Vec<Beat>) -> NewsSegmentScript {
        let lines = beats
            .into_iter()
            .enumerate()
            .map(|(position, beat)| {
                let partner = if beat.speaker == AnchorRole::Enthusiast { "AttentiveListening" } else { "EngagedNod" };
                ScriptDialogueLine {
                    speaker: beat.speaker,
                    camera_cut: camera_for(beat.speaker, position, beat.broll.is_some()),
                    phonetic_text: self.pronunciation.phonetically_prepare(&beat.text),
                    estimated_seconds: estimate_seconds(&beat.text),
                    spoken_text: beat.text,
                    anchor_expression: beat.expression,
                    partner_reaction_expression: partner.into(),
                    broll_visual_cue: beat.broll,
                }
            })
            .collect::<Vec<_>>();
        let actual = lines.iter().map(|line| line.estimated_seconds).sum();
        NewsSegmentScript {
            segment_number: number,
            segment_title: title.to_string(),
            segment_category: category.to_string(),
            target_duration_secs: actual,
            actual_duration_secs: actual,
            lines,
        }
    }

    fn assemble(&self, title: &str, date: &str, briefs: &[StoryBrief], story_beats: Vec<Vec<Beat>>, target: f64) -> MasterEpisodeScript {
        let mut segments = vec![self.segment(1, "Cold Open & Rundown", "Introduction", cold_open_beats(briefs))];
        for (index, (brief, beats)) in briefs.iter().zip(story_beats).enumerate() {
            let title = match &brief.retrospective {
                Some(retro) => format!("{}: {}", retro.framing, brief.headline),
                None => brief.headline.clone(),
            };
            segments.push(self.segment(index + 2, &title, &brief.category, beats));
        }
        let closing_number = segments.len() + 1;
        segments.push(self.segment(closing_number, "Sign-off & Transparent Credits", "Closing", closing_beats()));
        MasterEpisodeScript {
            show_title: atlas_core::SHOW_TITLE.into(),
            episode_title: title.into(),
            episode_date: date.into(),
            target_duration_secs: target,
            total_duration_secs: segments.iter().map(|s| s.actual_duration_secs).sum(),
            segments,
        }
    }

    pub fn template_episode(&self, title: &str, date: &str, briefs: &[StoryBrief], policy: &EpisodeDurationPolicy) -> DraftedEpisode {
        let beats = briefs.iter().enumerate().map(|(i, b)| template_beats(i, b)).collect();
        DraftedEpisode {
            script: self.assemble(title, date, briefs, beats, policy.target_seconds),
            segment_methods: vec![ScriptMethod::Template; briefs.len()],
            director_errors: Vec::new(),
        }
    }

    pub async fn director_episode(
        &self,
        director: &DirectorClient,
        title: &str,
        date: &str,
        briefs: &[StoryBrief],
        policy: &EpisodeDurationPolicy,
    ) -> DraftedEpisode {
        let retros = briefs.iter().filter(|b| b.is_retrospective()).count();
        let words = segment_words(briefs.len() - retros, retros, policy);
        let mut tasks = tokio::task::JoinSet::new();
        for (index, brief) in briefs.iter().cloned().enumerate() {
            let director = director.clone();
            let target = if brief.is_retrospective() { words.retro } else { words.news };
            tasks.spawn(async move { (index, director_story(&director, &brief, target).await) });
        }
        let mut drafted: Vec<Option<AtlasResult<Vec<Beat>>>> = (0..briefs.len()).map(|_| None).collect();
        while let Some(joined) = tasks.join_next().await {
            if let Ok((index, result)) = joined {
                drafted[index] = Some(result);
            }
        }
        let mut methods = Vec::with_capacity(briefs.len());
        let mut errors = Vec::new();
        let beats = drafted
            .into_iter()
            .enumerate()
            .map(|(index, result)| match result {
                Some(Ok(beats)) => {
                    methods.push(ScriptMethod::Director);
                    beats
                }
                other => {
                    let reason = match other {
                        Some(Err(error)) => error.to_string(),
                        _ => "director task did not complete".to_string(),
                    };
                    errors.push(format!("{}: {reason}", briefs[index].headline));
                    methods.push(ScriptMethod::Template);
                    template_beats(index, &briefs[index])
                }
            })
            .collect();
        DraftedEpisode {
            script: self.assemble(title, date, briefs, beats, policy.target_seconds),
            segment_methods: methods,
            director_errors: errors,
        }
    }
}

fn template_beats(index: usize, brief: &StoryBrief) -> Vec<Beat> {
    if brief.is_retrospective() {
        template_retro_beats(brief)
    } else {
        template_story_beats(index, brief)
    }
}

fn words_budget(stories: usize, policy: &EpisodeDurationPolicy) -> usize {
    let bookends_seconds = BOOKENDS_SECONDS;
    let per_story_seconds = (policy.target_seconds - bookends_seconds) / stories.max(1) as f64;
    ((per_story_seconds * WORDS_PER_SECOND) as usize).clamp(MIN_WORDS_PER_STORY, MAX_WORDS_PER_STORY)
}

fn retro_prompt(brief: &StoryBrief, retro: &RetroBrief, words: usize) -> String {
    let facts = retro.facts.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n");
    format!(
        "Write one RETROSPECTIVE segment of {show}, a two-anchor animation news show by {company}.\n\
         Anchors: Marcus (The Enthusiast: energetic, animation history buff, playful hyperbole) and \
         Elena (The Sharp Counterpart: witty, deadpan, grounded).\n\
         This is NOT breaking news. Open by framing it on air as \"{framing}\"{when}, and make clear it is a look back.\n\
         Style: Weekend Update desk banter: crisp setups, dry punchlines from Elena, Marcus's history-buff \
         enthusiasm, quick back-and-forth; jokes are about the facts, never instead of them.\n\
         Topic: {topic}\nVerified facts (each confirmed by Wikipedia and at least one other reputable source):\n{facts}\n\n\
         Rules: use ONLY the facts above; never invent dates, numbers, names, quotes, awards, or box office figures. \
         Say once, naturally, that the details come from {sources}. Opinions must be clearly the anchors' opinions. \
         Write about {words} words in 10 to 24 alternating lines, starting with Marcus.\n\
         Return JSON: {{\"lines\": [{{\"speaker\": \"marcus\" or \"elena\", \"text\": \"...\", \"expression\": \"one word\", \"broll\": \"short visual cue or null\"}}]}}",
        show = atlas_core::SHOW_TITLE,
        company = atlas_core::COMPANY_NAME,
        framing = retro.framing,
        when = retro.anniversary.as_ref().map(|a| format!(" ({a})")).unwrap_or_default(),
        topic = brief.headline,
        sources = join_names(&retro.sources),
    )
}

fn director_prompt(brief: &StoryBrief, words: usize) -> String {
    if let Some(retro) = &brief.retrospective {
        return retro_prompt(brief, retro, words);
    }
    let outlets = if brief.outlets.is_empty() { "unconfirmed".to_string() } else { brief.outlets.join(" and ") };
    format!(
        "Write one segment of {show}, a two-anchor animation news show by {company}.\n\
         Anchors: Marcus (The Enthusiast: energetic, animation history buff, playful hyperbole) and \
         Elena (The Sharp Counterpart: witty, deadpan, grounded, asks the hard question).\n\
         Headline: {headline}\nCategory: {category}\nRegion: {region}\nStudio or organization: {org}\nReported by: {outlets}\n\
         {official}{original}Facts: {facts}\n\n\
         Rules: use ONLY the facts above; never invent dates, numbers, names, quotes, or box office figures. \
         Credit the source once, the way it is written under Reported by (for example \"via Natalie, Japan\"); \
         if a studio announced it on its official channel, say so. Keep every name, title and accent exactly as written. Opinions must be clearly framed as the anchors' opinions. \
         Write about {words} words in 10 to 20 alternating lines, starting with Marcus. Keep it warm, funny, and accurate.\n\
         Return JSON: {{\"lines\": [{{\"speaker\": \"marcus\" or \"elena\", \"text\": \"...\", \"expression\": \"one word\", \"broll\": \"short visual cue or null\"}}]}}",
        show = atlas_core::SHOW_TITLE,
        company = atlas_core::COMPANY_NAME,
        headline = brief.headline,
        category = brief.category,
        region = if brief.region.is_empty() { "US" } else { brief.region.as_str() },
        official = brief.official_announcement.as_ref().map(|o| format!("Official source: {o}.\n")).unwrap_or_default(),
        original = brief.original_headline.as_ref().map(|h| format!("Original headline (translated above): {h}\n")).unwrap_or_default(),
        org = if brief.organization.is_empty() { "not named" } else { &brief.organization },
        facts = facts(&brief.summary),
    )
}

async fn director_story(director: &DirectorClient, brief: &StoryBrief, words: usize) -> AtlasResult<Vec<Beat>> {
    let value = director
        .complete_json(
            "You are the head writer for an animation news broadcast. Return valid JSON only.",
            &director_prompt(brief, words),
            0.6,
        )
        .await?;
    let segment: DirectorSegment = serde_json::from_value(value)
        .map_err(|error| AtlasError::Config(format!("Director segment shape invalid: {error}")))?;
    validate_director_segment(segment, words)
}

fn validate_director_segment(segment: DirectorSegment, target_words: usize) -> AtlasResult<Vec<Beat>> {
    if !(6..=30).contains(&segment.lines.len()) {
        return Err(AtlasError::Config(format!("Director wrote {} lines; expected 6 to 30", segment.lines.len())));
    }
    let mut beats = Vec::with_capacity(segment.lines.len());
    for line in segment.lines {
        let speaker = match line.speaker.trim().to_lowercase().as_str() {
            "marcus" => AnchorRole::Enthusiast,
            "elena" => AnchorRole::Counterpart,
            other => return Err(AtlasError::Config(format!("Unknown speaker '{other}'"))),
        };
        let text = line.text.trim().to_string();
        if text.is_empty() || word_count(&text) > 120 {
            return Err(AtlasError::Config("Director line empty or longer than 120 words".into()));
        }
        beats.push(Beat {
            speaker,
            text,
            expression: line.expression.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| "Engaged".into()),
            broll: line.broll.filter(|b| !b.trim().is_empty() && b.trim() != "null"),
        });
    }
    if !beats.iter().any(|b| b.speaker == AnchorRole::Enthusiast) || !beats.iter().any(|b| b.speaker == AnchorRole::Counterpart) {
        return Err(AtlasError::Config("Director segment must include both anchors".into()));
    }
    let total = beats.iter().map(|b| word_count(&b.text)).sum::<usize>();
    if total * 2 < target_words {
        return Err(AtlasError::Config(format!("Director segment too short: {total} of ~{target_words} words")));
    }
    Ok(beats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(headline: &str, outlets: &[&str]) -> StoryBrief {
        StoryBrief {
            headline: headline.into(),
            summary: "MAPPA confirmed production of the sequel. A new staff list was also announced. (Source: Anime News Network)".into(),
            category: "Anime".into(),
            organization: "MAPPA".into(),
            outlets: outlets.iter().map(|o| o.to_string()).collect(),
            region: String::new(),
            official_announcement: None,
            original_headline: None,
            retrospective: None,
        }
    }

    fn retro(title: &str) -> StoryBrief {
        StoryBrief {
            headline: title.into(),
            summary: String::new(),
            category: "Retrospective".into(),
            organization: String::new(),
            outlets: vec!["Wikipedia".into(), "the Library of Congress".into()],
            region: String::new(),
            official_announcement: None,
            original_headline: None,
            retrospective: Some(RetroBrief {
                framing: "This Week in Animation History".into(),
                anniversary: Some("released October 18, 1967".into()),
                facts: vec![
                    "The Jungle Book is a 1967 American animated musical adventure film produced by Walt Disney Productions.".into(),
                    "It was the last film produced by Walt Disney, who died during its production.".into(),
                    "The film was released on October 18, 1967.".into(),
                ],
                sources: vec!["Wikipedia".into(), "the Library of Congress".into()],
            }),
        }
    }

    #[test]
    fn retrospectives_fill_thin_news_days() {
        let policy = EpisodeDurationPolicy::default();
        assert_eq!(retros_needed(1, &policy), 4);
        assert_eq!(retros_needed(3, &policy), 2);
        assert_eq!(retros_needed(6, &policy), 0);
        let thirty = EpisodeDurationPolicy { min_seconds: 1500.0, max_seconds: 2100.0, target_seconds: 1800.0 };
        assert_eq!(retros_needed(4, &thirty), 4); // 1680 s fill: 4 news x 200 s + 75 s bookends + 4 retros
        assert_eq!(retros_needed(1, &thirty), 7);
        assert_eq!(retros_needed_with_feature(4, 300.0, &thirty), 3);
        assert_eq!(segment_words(2, 2, &policy), SegmentWords { news: NEWS_WORDS_WITH_RETROS, retro: RETRO_WORDS });
        let briefs = vec![brief("MAPPA Confirms Chainsaw Man Season 2", &["Anime News Network", "Cartoon Brew"]), retro("The Jungle Book (1967)")];
        let script = NewsroomWriter::new().template_episode("EP-009", "2026-10-18", &briefs, &policy).script;
        assert_eq!(script.segments[2].segment_title, "This Week in Animation History: The Jungle Book (1967)");
        let open = script.segments[0].lines.iter().map(|l| l.spoken_text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(open.contains("1 news story"), "{open}");
        assert!(open.contains("a look back"));
        let text = script.segments[2].lines.iter().map(|l| l.spoken_text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(text.contains("not breaking news"));
        assert!(text.contains("Wikipedia and the Library of Congress"));
        let prompt = director_prompt(&briefs[1], 600);
        assert!(prompt.contains("NOT breaking news") && prompt.contains("Weekend Update") && prompt.contains("October 18, 1967"));
    }

    #[test]
    fn template_episode_is_built_from_the_stories() {
        let briefs = vec![
            brief("MAPPA Confirms Chainsaw Man Season 2", &["Anime News Network", "Cartoon Brew"]),
            brief("Toei Unveils New Film", &["Crunchyroll News"]),
        ];
        let drafted = NewsroomWriter::new().template_episode("EP-002", "2026-10-17", &briefs, &EpisodeDurationPolicy::default());
        let script = drafted.script;
        assert_eq!(script.segments.len(), 4);
        assert_eq!(script.segments[1].segment_title, "MAPPA Confirms Chainsaw Man Season 2");
        let story_text = script.segments[1].lines.iter().map(|l| l.spoken_text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(story_text.contains("MAPPA confirmed production of the sequel."));
        assert!(story_text.contains("confirmed by two outlets, via Anime News Network and via Cartoon Brew"));
        assert!(!story_text.contains("(Source:"));
        let second = script.segments[2].lines.iter().map(|l| l.spoken_text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(second.contains("single outlet, via Crunchyroll News"));
        assert!(script.segments.iter().flat_map(|s| &s.lines).all(|l| !l.spoken_text.contains("Segment discussion beat")));
        let summed: f64 = script.segments.iter().map(|s| s.actual_duration_secs).sum();
        assert!((script.total_duration_secs - summed).abs() < 1e-9);
        assert_eq!(drafted.segment_methods, vec![ScriptMethod::Template; 2]);
    }

    #[test]
    fn director_segments_are_validated() {
        let line = |speaker: &str, words: usize| DirectorLine {
            speaker: speaker.into(),
            text: vec!["word"; words].join(" "),
            expression: None,
            broll: Some("null".into()),
        };
        let good = DirectorSegment { lines: (0..10).map(|i| line(if i % 2 == 0 { "Marcus" } else { "elena" }, 25)).collect() };
        let beats = validate_director_segment(good, 300).unwrap();
        assert_eq!(beats.len(), 10);
        assert!(beats[0].broll.is_none());

        let one_anchor = DirectorSegment { lines: (0..8).map(|_| line("marcus", 30)).collect() };
        assert!(validate_director_segment(one_anchor, 300).is_err());
        let too_short = DirectorSegment { lines: (0..8).map(|i| line(if i % 2 == 0 { "marcus" } else { "elena" }, 5)).collect() };
        assert!(validate_director_segment(too_short, 300).is_err());
        let stranger = DirectorSegment { lines: vec![line("marcus", 30), line("guest", 30), line("elena", 30), line("marcus", 30), line("elena", 30), line("marcus", 30)] };
        assert!(validate_director_segment(stranger, 100).is_err());
    }

    #[test]
    fn word_budget_targets_policy_runtime() {
        let policy = EpisodeDurationPolicy::default();
        assert_eq!(words_budget(8, &policy), 404);
        assert_eq!(words_budget(1, &policy), MAX_WORDS_PER_STORY);
        let three_story_seconds = 3.0 * words_budget(3, &policy) as f64 / WORDS_PER_SECOND + 75.0;
        assert!(three_story_seconds >= policy.min_seconds, "{three_story_seconds}");
        assert_eq!(words_budget(40, &policy), MIN_WORDS_PER_STORY);
    }

    #[test]
    fn sourcing_credits_regions_and_official_announcements() {
        let japan = brief("Chainsaw Man Assassins Arc announced", &["Natalie, Japan", "Anime News Network"]);
        assert_eq!(sourcing(&japan), "And this one is confirmed by two outlets, via Natalie, Japan and via Anime News Network.");
        let mut official = brief("Toy Story 5 trailer", &["Pixar's official YouTube channel"]);
        official.official_announcement = Some("Pixar announced it on its official YouTube channel".into());
        assert_eq!(sourcing(&official), "Pixar announced it on its official YouTube channel, so this one comes straight from the source.");
        official.outlets.push("Cartoon Brew".into());
        assert_eq!(sourcing(&official), "Pixar announced it on its official YouTube channel, and Cartoon Brew reported it too.");
    }
}
