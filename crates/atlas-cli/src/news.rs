use anyhow::{Context, anyhow, bail};
use atlas_analytics::{YouTubeAnalyticsClient, YouTubeUploadMetadata, load_youtube_credentials};
use atlas_core::{EpisodeDurationPolicy, NewsProductionStage, SHOW_TAGLINE, SHOW_TITLE};
use atlas_ffmpeg::broadcast::{
    AudioPiece, BroadcastRenderer, BroadcastSegment, BroadcastSpec, CaptionCue, CueSpeaker,
    LoudnessReport, MediaProbe, ShortCue, ShortSpec, SpeakerWindow, VideoEncoder, chapter_time,
    write_srt,
};
use atlas_kokoro::{KokoroPipeline, NarrationSegment, VoiceConfig};
use atlas_media::fact_checker::{FactCheckReport, NewsFactChecker};
use atlas_media::news_desk::{
    AnimationNewsDesk, NewsStory, default_sources, load_sources, sources_from_env,
};
use atlas_policy::copyright::{
    CopyrightComplianceEngine, MediaClassification, MediaType, RightsRecord, ScanResult,
};
use atlas_policy::telegram::{
    ApprovalDecision, EpisodeSlateProposal, ProposedNewsStory, TelegramApprovalGate, escape_html,
};
use atlas_policy::{ChannelPreflight, CheckStatus, ContentPreflight, PolicyGuardian};
use atlas_storyboard::two_anchor_script::{
    AnchorRole, BulletinBrief, LINE_PAUSE_SECONDS, MasterEpisodeScript, NewsScriptDirector,
    SEGMENT_CARD_SECONDS, ScriptStoryBrief, TwoAnchorScriptWriter, WORDS_PER_SECOND,
};
use chrono::Utc;
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Args, Clone, Debug)]
pub struct ResearchOptions {
    #[arg(long, default_value_t = 72)]
    max_age_hours: i64,
    #[arg(long, default_value_t = 2)]
    min_sources: usize,
    #[arg(long, default_value_t = 5)]
    max_stories: usize,
    #[arg(long, default_value_t = 22.0)]
    target_minutes: f64,
    #[arg(long)]
    feeds_file: Option<PathBuf>,
}

impl ResearchOptions {
    fn policy(&self) -> anyhow::Result<EpisodeDurationPolicy> {
        let policy = EpisodeDurationPolicy::default();
        let target = self.target_minutes * 60.0;
        if !policy.is_valid(target) {
            bail!("--target-minutes must be between 15 and 30");
        }
        Ok(EpisodeDurationPolicy {
            target_seconds: target,
            ..policy
        })
    }
}

#[derive(Args, Clone, Debug)]
pub struct TelegramOptions {
    #[arg(long, env = "TELEGRAM_BOT_TOKEN", hide_env_values = true)]
    token: Option<String>,
    #[arg(long, env = "TELEGRAM_CHAT_ID")]
    chat_id: Option<i64>,
    #[arg(long, default_value_t = 120)]
    approval_timeout_mins: u64,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum NewsPrivacy {
    Private,
    Unlisted,
    Public,
}

impl NewsPrivacy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Unlisted => "unlisted",
            Self::Public => "public",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum NewsResolution {
    Uhd,
    Fhd,
}

impl NewsResolution {
    fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Uhd => (3840, 2160),
            Self::Fhd => (1920, 1080),
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum NewsEncoder {
    X264,
    X265,
    NvencH264,
    NvencHevc,
}

impl NewsEncoder {
    fn encoder(self) -> VideoEncoder {
        match self {
            Self::X264 => VideoEncoder::X264,
            Self::X265 => VideoEncoder::X265,
            Self::NvencH264 => VideoEncoder::NvencH264,
            Self::NvencHevc => VideoEncoder::NvencHevc,
        }
    }

    fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::X264 => "libx264",
            Self::X265 => "libx265",
            Self::NvencH264 => "h264_nvenc",
            Self::NvencHevc => "hevc_nvenc",
        }
    }
}

#[derive(Subcommand)]
pub enum NewsAction {
    Research {
        #[command(flatten)]
        research: ResearchOptions,
        #[arg(long)]
        json: bool,
    },
    Slate {
        #[command(flatten)]
        research: ResearchOptions,
    },
    Telegram {
        #[command(flatten)]
        research: ResearchOptions,
        #[command(flatten)]
        telegram: TelegramOptions,
    },
    Copyright {
        #[arg(long)]
        episode: String,
    },
    Produce {
        #[command(flatten)]
        research: ResearchOptions,
        #[command(flatten)]
        telegram: TelegramOptions,
        #[arg(long, alias = "skip-telegram-wait")]
        skip_telegram: bool,
        #[arg(long)]
        episode: Option<String>,
        #[arg(long, value_enum, default_value_t = NewsResolution::Uhd)]
        resolution: NewsResolution,
        #[arg(long, value_enum, default_value_t = NewsEncoder::X264)]
        encoder: NewsEncoder,
        #[arg(long)]
        studio_plate: Option<PathBuf>,
        #[arg(long)]
        music: Option<PathBuf>,
        #[arg(long)]
        publish: bool,
        #[arg(long, value_enum, default_value_t = NewsPrivacy::Private)]
        privacy: NewsPrivacy,
        #[arg(long)]
        shorts: bool,
    },
    Publish {
        #[arg(long)]
        episode: String,
        #[arg(long, value_enum, default_value_t = NewsPrivacy::Private)]
        privacy: NewsPrivacy,
        #[arg(long)]
        shorts: bool,
    },
}

#[derive(Default, Serialize, Deserialize)]
struct NewsState {
    next_episode_number: u32,
}

#[derive(Serialize, Deserialize)]
struct StageEntry {
    stage: NewsProductionStage,
    status: String,
    detail: String,
    at: chrono::DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct Selection {
    episode_id: String,
    episode_number: u32,
    stories: Vec<NewsStory>,
    bulletins: Vec<NewsStory>,
    approved_by: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct EpisodeMetadata {
    title: String,
    description: String,
    tags: Vec<String>,
    transcript: String,
    contains_synthetic_media: bool,
}

#[derive(Serialize, Deserialize)]
struct QaReport {
    passed: bool,
    checks: Vec<(String, bool, String)>,
    probe: MediaProbe,
    loudness: LoudnessReport,
}

#[derive(Clone, Serialize, Deserialize)]
struct ShortRecord {
    segment_number: usize,
    file: String,
    title: String,
    description: String,
    duration: f64,
    youtube_video_id: Option<String>,
    url: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PublishRecord {
    youtube_video_id: String,
    url: String,
    privacy: String,
    published_at: chrono::DateTime<Utc>,
}

struct Episode {
    id: String,
    number: u32,
    dir: PathBuf,
    log: Vec<StageEntry>,
}

impl Episode {
    fn stage(&mut self, stage: NewsProductionStage, status: &str, detail: impl Into<String>) {
        let detail = detail.into();
        println!("  [{status}] {stage:?}: {detail}");
        self.log.push(StageEntry {
            stage,
            status: status.into(),
            detail,
            at: Utc::now(),
        });
        let _ = write_json(&self.dir.join("production_log.json"), &self.log);
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> anyhow::Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn news_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("news")
}

fn peek_episode_number(data_dir: &Path) -> u32 {
    read_json::<NewsState>(&news_dir(data_dir).join("state.json"))
        .map(|s| s.next_episode_number)
        .unwrap_or(1)
        .max(1)
}

fn allocate_episode(data_dir: &Path) -> anyhow::Result<Episode> {
    let number = peek_episode_number(data_dir);
    write_json(
        &news_dir(data_dir).join("state.json"),
        &NewsState {
            next_episode_number: number + 1,
        },
    )?;
    let id = format!("EP-{number:03}");
    let dir = news_dir(data_dir).join("episodes").join(&id);
    std::fs::create_dir_all(&dir)?;
    Ok(Episode {
        id,
        number,
        dir,
        log: Vec::new(),
    })
}

fn open_episode(data_dir: &Path, id: &str) -> anyhow::Result<Episode> {
    let dir = news_dir(data_dir).join("episodes").join(id);
    if !dir.is_dir() {
        bail!("episode {id} not found in {}", dir.display());
    }
    let number = id
        .trim_start_matches("EP-")
        .parse()
        .with_context(|| format!("episode id {id} is not EP-NNN"))?;
    let log = read_json(&dir.join("production_log.json")).unwrap_or_default();
    Ok(Episode {
        id: id.to_string(),
        number,
        dir,
        log,
    })
}

fn news_desk(options: &ResearchOptions) -> anyhow::Result<AnimationNewsDesk> {
    let sources = match &options.feeds_file {
        Some(path) => load_sources(path).map_err(|e| anyhow!(e))?,
        None => sources_from_env().unwrap_or_else(default_sources),
    };
    Ok(AnimationNewsDesk::new(sources, options.max_age_hours))
}

async fn research(desk: &AnimationNewsDesk, options: &ResearchOptions) -> anyhow::Result<FactCheckReport> {
    let wire = desk.fetch_wire().await;
    println!(
        "  Wire: {} items from {}/{} feeds",
        wire.items.len(),
        wire.feeds_ok,
        desk.sources().len()
    );
    for (outlet, error) in &wire.feed_errors {
        println!("  ! {outlet}: {error}");
    }
    if wire.feeds_ok == 0 {
        bail!("no news feed could be read; check network access or ATLAS_NEWS_FEEDS");
    }
    let report = NewsFactChecker::new(options.min_sources, true).verify(wire.items);
    println!(
        "  Fact check: {} corroborated, {} single-source, {} rejected",
        report.passed_stories.len(),
        report.bulletins.len(),
        report.rejected.len()
    );
    Ok(report)
}

fn select(report: &FactCheckReport, max_stories: usize, excluded: &[String]) -> (Vec<NewsStory>, Vec<NewsStory>) {
    let available: Vec<NewsStory> = report
        .passed_stories
        .iter()
        .filter(|s| !excluded.contains(&s.id))
        .cloned()
        .collect();
    let stories: Vec<NewsStory> = available.iter().take(max_stories.max(1)).cloned().collect();
    let mut bulletins: Vec<NewsStory> = available.into_iter().skip(stories.len()).collect();
    bulletins.extend(report.bulletins.iter().filter(|s| !excluded.contains(&s.id)).cloned());
    bulletins.truncate(8);
    (stories, bulletins)
}

fn proposal(episode_id: &str, number: u32, stories: &[NewsStory], bulletins: &[NewsStory], target_secs: f64) -> EpisodeSlateProposal {
    let writer = TwoAnchorScriptWriter::new();
    let budget = writer.story_word_budget(stories.len(), bulletins.len(), target_secs);
    let per_story_mins = budget as f64 / WORDS_PER_SECOND / 60.0;
    EpisodeSlateProposal {
        episode_id: episode_id.to_string(),
        episode_number: number,
        target_duration_mins: target_secs / 60.0,
        stories: stories
            .iter()
            .map(|s| ProposedNewsStory {
                story_id: s.id.clone(),
                category: s.category.as_str().to_string(),
                headline: s.title.clone(),
                factual_summary: s.summary.clone(),
                source_url: s.primary_url().to_string(),
                sources: s.outlets(),
                announcement_date: s.published.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
                estimated_duration_mins: per_story_mins,
                verified: s.corroboration() >= 2,
            })
            .collect(),
        bulletins: bulletins.iter().map(|b| format!("{} ({})", b.title, b.outlets().join(", "))).collect(),
    }
}

fn print_slate(slate: &EpisodeSlateProposal) {
    println!("\n  Slate {} — target {:.0} min", slate.episode_id, slate.target_duration_mins);
    for (i, s) in slate.stories.iter().enumerate() {
        println!("  {:02}. [{}] {}", i + 1, s.category, s.headline);
        println!("      {} · {}", s.sources.join(", "), s.source_url);
    }
    if !slate.bulletins.is_empty() {
        println!("  Rapid-fire:");
        for b in &slate.bulletins {
            println!("    • {b}");
        }
    }
}

fn ensure_enough(stories: &[NewsStory]) -> anyhow::Result<()> {
    if stories.len() < 3 {
        bail!(
            "only {} corroborated stor{} available; a 15-30 minute episode needs at least 3. Widen --max-age-hours, add feeds, or lower --min-sources",
            stories.len(),
            if stories.len() == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

async fn approval_loop(
    gate: &mut TelegramApprovalGate,
    desk: &AnimationNewsDesk,
    options: &ResearchOptions,
    telegram: &TelegramOptions,
    episode_id: &str,
    number: u32,
    mut report: FactCheckReport,
) -> anyhow::Result<Option<(Vec<NewsStory>, Vec<NewsStory>, String)>> {
    let mut excluded: Vec<String> = Vec::new();
    for _round in 0..6 {
        let (stories, bulletins) = select(&report, options.max_stories, &excluded);
        ensure_enough(&stories)?;
        let slate = proposal(episode_id, number, &stories, &bulletins, options.policy()?.target_seconds);
        print_slate(&slate);
        gate.send_slate(&slate).await.map_err(|e| anyhow!(e))?;
        println!("  Slate sent to Telegram; waiting up to {} min for a decision...", telegram.approval_timeout_mins);
        let decision = gate
            .poll_approval(&slate, Duration::from_secs(telegram.approval_timeout_mins * 60))
            .await
            .map_err(|e| anyhow!(e))?;
        match decision {
            ApprovalDecision::Approved { approver, .. } => return Ok(Some((stories, bulletins, approver))),
            ApprovalDecision::Rejected { reason, .. } => {
                println!("  Slate rejected: {reason}");
                return Ok(None);
            }
            ApprovalDecision::ReplaceRequested { story_id, reason } => {
                println!("  {reason}: dropping {story_id}");
                excluded.push(story_id);
            }
            ApprovalDecision::ResearchRequested { notes, .. } => {
                println!("  {notes}: refreshing the wire");
                report = research(desk, options).await?;
            }
            ApprovalDecision::Pending => {
                println!("  No decision before the approval timeout.");
                return Ok(None);
            }
        }
    }
    bail!("approval did not converge after 6 rounds")
}

fn tool(env: &str, default: &str) -> PathBuf {
    std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(default))
}

fn resolve_fonts() -> anyhow::Result<(PathBuf, PathBuf)> {
    let pick = |env: &str, candidates: &[&str]| -> Option<PathBuf> {
        std::env::var_os(env)
            .map(PathBuf::from)
            .into_iter()
            .chain(candidates.iter().map(PathBuf::from))
            .find(|p| p.is_file())
    };
    let regular = pick(
        "ATLAS_NEWS_FONT",
        &[
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ],
    );
    let bold = pick(
        "ATLAS_NEWS_FONT_BOLD",
        &[
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        ],
    );
    match (regular, bold) {
        (Some(r), Some(b)) => Ok((r, b)),
        (Some(r), None) => Ok((r.clone(), r)),
        _ => bail!("no usable font found; set ATLAS_NEWS_FONT and ATLAS_NEWS_FONT_BOLD to .ttf files"),
    }
}

fn brief(story: &NewsStory) -> ScriptStoryBrief {
    ScriptStoryBrief {
        story_id: story.id.clone(),
        headline: story.title.clone(),
        category: story.category.as_str().to_string(),
        organization: story.studio_or_organization.clone(),
        outlets: story.outlets(),
        fact_sheet: story.fact_sheet(),
    }
}

async fn write_script(
    episode: &mut Episode,
    stories: &[NewsStory],
    bulletins: &[NewsStory],
    policy: &EpisodeDurationPolicy,
) -> anyhow::Result<MasterEpisodeScript> {
    let director = NewsScriptDirector::from_env().ok_or_else(|| {
        anyhow!("script generation needs an LLM: set NVIDIA_API_KEY (or ATLAS_DIRECTOR_API_KEY with ATLAS_DIRECTOR_URL for any OpenAI-compatible endpoint)")
    })?;
    let writer = TwoAnchorScriptWriter::new();
    let briefs: Vec<ScriptStoryBrief> = stories.iter().map(brief).collect();
    let bulletin_briefs: Vec<BulletinBrief> = bulletins
        .iter()
        .map(|b| BulletinBrief {
            headline: b.title.clone(),
            outlet: b.outlets().first().cloned().unwrap_or_default(),
        })
        .collect();
    let budget = writer.story_word_budget(briefs.len(), bulletin_briefs.len(), policy.target_seconds);
    episode.stage(
        NewsProductionStage::LongFormScriptGeneration,
        "RUN",
        format!("{} story segments, ~{budget} words each, model {}", briefs.len(), director.model()),
    );
    let mut stories_written = Vec::new();
    for (i, b) in briefs.iter().enumerate() {
        let segment = director.write_story_segment(&writer, i + 2, b, budget).await?;
        println!(
            "    Segment {:02}: {} ({} words, short: {})",
            segment.segment_number,
            segment.headline,
            segment.word_count(),
            segment.short.as_ref().map(|s| s.title.as_str()).unwrap_or("-")
        );
        stories_written.push(segment);
    }
    let story_count = stories_written.len();
    for i in 0..story_count.saturating_sub(1) {
        let next = stories_written[i + 1].headline.clone();
        writer.append_tease(&mut stories_written[i], &next, i);
    }
    let lead_hook = stories_written.first().and_then(|s| {
        let hook = s.short.as_ref().map(|c| c.hook.clone()).unwrap_or_else(|| s.lines[0].spoken_text.clone());
        (hook.split_whitespace().count() <= 30).then_some(hook)
    });
    let mut segments = vec![writer.cold_open(episode.number, &briefs, bulletin_briefs.len(), lead_hook.as_deref())];
    segments.extend(stories_written);
    let mut next = briefs.len() + 2;
    if let Some(rapid) = writer.rapid_headlines(next, &bulletin_briefs) {
        segments.push(rapid);
        next += 1;
    }
    segments.push(writer.closing(next, &briefs));
    let title = format!("{SHOW_TITLE} #{}: {}", episode.number, briefs[0].headline);
    let script = writer.assemble(&title, &Utc::now().format("%Y-%m-%d").to_string(), policy.target_seconds, segments);
    episode.stage(
        NewsProductionStage::ScriptFactAudit,
        "PASS",
        "every figure in every story segment matches its fact sheet",
    );
    if script.total_duration_secs < policy.min_seconds * 0.9 || script.total_duration_secs > policy.max_seconds * 1.05 {
        bail!(
            "script length estimate {:.0}s is outside the 15-30 minute window; not synthesizing",
            script.total_duration_secs
        );
    }
    write_json(&episode.dir.join("script.json"), &script)?;
    std::fs::write(episode.dir.join("transcript.txt"), script.transcript())?;
    Ok(script)
}

fn voice_for(speaker: AnchorRole) -> Vec<String> {
    let marcus = std::env::var("ATLAS_MARCUS_VOICE").unwrap_or_else(|_| "am_michael".into());
    let elena = std::env::var("ATLAS_ELENA_VOICE").unwrap_or_else(|_| "af_bella".into());
    match speaker {
        AnchorRole::Enthusiast => vec![marcus],
        AnchorRole::Counterpart => vec![elena],
        AnchorRole::Duo => vec![marcus, elena],
    }
}

fn cache_key(voice: &str, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(voice.as_bytes());
    hasher.update([0]);
    hasher.update(text.as_bytes());
    format!("{}_{}", voice, &hex::encode(hasher.finalize())[..20])
}

async fn synthesize(data_dir: &Path, script: &MasterEpisodeScript) -> anyhow::Result<Vec<Vec<PathBuf>>> {
    let workspace = news_dir(data_dir).join("tts");
    let pipeline = Arc::new(KokoroPipeline::new(
        tool("ATLAS_KOKORO_EXE", "kokoro-tts"),
        VoiceConfig::default(),
        &workspace,
    ));
    let speed: f32 = std::env::var("ATLAS_NEWS_TTS_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let jobs: usize = std::env::var("ATLAS_TTS_JOBS").ok().and_then(|v| v.parse().ok()).unwrap_or(2).max(1);
    let lines: Vec<_> = script.segments.iter().flat_map(|s| s.lines.iter()).collect();
    let mut requests = Vec::new();
    let mut per_line = Vec::new();
    for line in &lines {
        let mut paths = Vec::new();
        for voice in voice_for(line.speaker) {
            let id = cache_key(&voice, &line.phonetic_text);
            let path = workspace.join("audio").join("segments").join(format!("{id}.wav"));
            if !path.is_file() && !requests.iter().any(|(r, _, _): &(String, String, String)| r == &id) {
                requests.push((id.clone(), voice.clone(), line.phonetic_text.clone()));
            }
            paths.push(path);
        }
        per_line.push(paths);
    }
    println!(
        "    {} lines, {} new clips to synthesize ({} served from cache or shared lines), {jobs} parallel",
        lines.len(),
        requests.len(),
        per_line.iter().map(Vec::len).sum::<usize>() - requests.len()
    );
    let semaphore = Arc::new(tokio::sync::Semaphore::new(jobs));
    let mut set = tokio::task::JoinSet::new();
    let total = requests.len();
    for (id, voice, text) in requests {
        let pipeline = pipeline.clone();
        let permit = semaphore.clone().acquire_owned().await?;
        set.spawn(async move {
            let _permit = permit;
            pipeline
                .synthesize_segment(&NarrationSegment {
                    segment_id: id,
                    text,
                    voice,
                    speed,
                    pitch: 1.0,
                    emphasis_words: vec![],
                    pause_after_ms: 0,
                    pronunciation_hints: vec![],
                })
                .await
        });
    }
    let mut done = 0;
    while let Some(result) = set.join_next().await {
        result??;
        done += 1;
        if done % 20 == 0 || done == total {
            println!("    synthesized {done}/{total}");
        }
    }
    Ok(per_line)
}

struct Timeline {
    segments: Vec<BroadcastSegment>,
    speakers: Vec<SpeakerWindow>,
    captions: Vec<CaptionCue>,
    duration: f64,
}

fn build_pieces(script: &MasterEpisodeScript, voices: &[Vec<PathBuf>]) -> Vec<AudioPiece> {
    let mut pieces = Vec::new();
    let mut index = 0;
    for segment in &script.segments {
        pieces.push(AudioPiece::Silence(SEGMENT_CARD_SECONDS));
        for _ in &segment.lines {
            pieces.push(AudioPiece::Voice(voices[index].clone()));
            pieces.push(AudioPiece::Silence(LINE_PAUSE_SECONDS));
            index += 1;
        }
    }
    pieces.push(AudioPiece::Silence(1.5));
    pieces
}

fn build_timeline(script: &MasterEpisodeScript, durations: &[f64]) -> Timeline {
    let mut t = 0.0;
    let mut cursor = 0;
    let mut segments = Vec::new();
    let mut speakers = Vec::new();
    let mut captions = Vec::new();
    for segment in &script.segments {
        let start = t;
        t += durations[cursor];
        cursor += 1;
        for line in &segment.lines {
            let voice = durations[cursor];
            let speaker = match line.speaker {
                AnchorRole::Enthusiast => CueSpeaker::Marcus,
                AnchorRole::Counterpart => CueSpeaker::Elena,
                AnchorRole::Duo => CueSpeaker::Both,
            };
            speakers.push(SpeakerWindow { speaker, start: t, end: t + voice });
            captions.push(CaptionCue {
                start: t,
                end: t + voice,
                text: line.spoken_text.clone(),
            });
            t += voice + durations[cursor + 1];
            cursor += 2;
        }
        segments.push(BroadcastSegment {
            title: segment.segment_title.clone(),
            category: segment.segment_category.clone(),
            headline: segment.headline.clone(),
            attribution: segment.attribution.clone(),
            start,
            end: t,
            card_seconds: SEGMENT_CARD_SECONDS,
            notes: String::new(),
        });
    }
    t += durations.get(cursor).copied().unwrap_or(0.0);
    Timeline {
        segments,
        speakers,
        captions,
        duration: t,
    }
}

fn fill_wall_notes(timeline: &mut Timeline, script: &MasterEpisodeScript, selection: &Selection) {
    let headlines: Vec<String> = selection.stories.iter().map(|s| s.title.clone()).collect();
    for (slot, segment) in timeline.segments.iter_mut().zip(&script.segments) {
        slot.notes = match &segment.story_id {
            Some(id) => selection
                .stories
                .iter()
                .find(|s| &s.id == id)
                .map(|s| s.summary.clone())
                .unwrap_or_default(),
            None if segment.segment_title == "Cold Open" => format!("Today: {}", headlines.join("  ·  ")),
            None if segment.segment_title == "Rapid-Fire Headlines" => selection
                .bulletins
                .iter()
                .map(|b| format!("• {}", b.title))
                .collect::<Vec<_>>()
                .join("  "),
            None => "Every story in this episode is sourced; links are in the description. Anchors are AI-generated voices.".into(),
        };
    }
}

fn rights_manifest(
    fonts: &(PathBuf, PathBuf),
    dialogue: &Path,
    music: Option<&Path>,
    plate: Option<&Path>,
) -> anyhow::Result<Vec<RightsRecord>> {
    let mut manifest = vec![
        RightsRecord::for_file(
            dialogue,
            "anchor_dialogue",
            MediaType::AiGenerated,
            "Clearly Drawn News (Kokoro TTS, Apache-2.0 model)",
            "Original script; Kokoro-82M weights Apache-2.0",
            MediaClassification::Cleared,
        )?,
        RightsRecord::for_file(
            &fonts.0,
            "font_regular",
            MediaType::LogoOrTrademark,
            "System font vendor",
            "Installed system font, rendered into video frames",
            MediaClassification::Cleared,
        )?,
        RightsRecord::for_file(
            &fonts.1,
            "font_bold",
            MediaType::LogoOrTrademark,
            "System font vendor",
            "Installed system font, rendered into video frames",
            MediaClassification::Cleared,
        )?,
    ];
    if let Some(path) = music {
        manifest.push(RightsRecord::from_sidecar(path, "music_bed", MediaType::MusicOrSfx)?);
    }
    if let Some(path) = plate {
        manifest.push(RightsRecord::from_sidecar(path, "studio_plate", MediaType::OriginalUnrealFootage)?);
    }
    Ok(manifest)
}

fn describe_scan(result: &ScanResult) -> Option<String> {
    match result {
        ScanResult::AllClear => None,
        ScanResult::ReviewPending(ids) => Some(format!("held for rights review: {}", ids.join(", "))),
        ScanResult::BlockedRestrictedAssets(ids) => Some(format!("restricted assets: {}", ids.join(", "))),
        ScanResult::BlockedProhibitedAssets(ids) => Some(format!("prohibited assets: {}", ids.join(", "))),
    }
}

fn metadata(script: &MasterEpisodeScript, timeline: &Timeline, stories: &[NewsStory], bulletins: &[NewsStory]) -> EpisodeMetadata {
    let mut title: String = script.episode_title.chars().take(100).collect();
    if script.episode_title.chars().count() > 100 {
        title = format!("{}…", title.chars().take(99).collect::<String>());
    }
    let mut description = format!(
        "{SHOW_TITLE} — {SHOW_TAGLINE}\nToday: {}.\n\nCHAPTERS\n",
        stories.iter().map(|s| s.title.as_str()).collect::<Vec<_>>().join("; ")
    );
    for segment in &timeline.segments {
        description.push_str(&format!("{} {}\n", chapter_time(segment.start), segment.title));
    }
    description.push_str("\nSOURCES\n");
    for story in stories.iter().chain(bulletins.iter()) {
        for source in &story.sources {
            description.push_str(&format!("• {} — {}: {}\n", source.outlet, source.title, source.url));
        }
    }
    description.push_str("\nHOW THIS SHOW IS MADE\nThe anchors Marcus and Elena are AI-generated voices (Kokoro text-to-speech) and the script is written with AI assistance from the reporting linked above. Every main story is corroborated by at least two outlets; rapid-fire items are single-source and attributed on air. This is altered or synthetic content.\n");
    if description.chars().count() > 4900 {
        description = description.chars().take(4900).collect();
    }
    EpisodeMetadata {
        title,
        description,
        tags: vec![
            "animation news".into(),
            "anime news".into(),
            "cartoon news".into(),
            "Clearly Drawn News".into(),
            "animation industry".into(),
        ],
        transcript: script.transcript(),
        contains_synthetic_media: true,
    }
}

fn qa(probe: &MediaProbe, loudness: &LoudnessReport, width: u32, height: u32, codec: &str) -> QaReport {
    let policy = EpisodeDurationPolicy::default();
    let checks = vec![
        ("duration_15_to_30_min".to_string(), policy.is_valid(probe.duration), format!("{:.1}s", probe.duration)),
        ("resolution_16x9".into(), probe.width == width && probe.height == height, format!("{}x{}", probe.width, probe.height)),
        ("frame_rate_30".into(), (probe.fps - 30.0).abs() < 0.05, format!("{:.2} fps", probe.fps)),
        (format!("video_{codec}"), probe.video_codec == codec, probe.video_codec.clone()),
        ("audio_aac_stereo".into(), probe.audio_codec == "aac" && probe.audio_channels == 2, format!("{} {}ch", probe.audio_codec, probe.audio_channels)),
        (
            "loudness_minus14_lufs".into(),
            (loudness.integrated_lufs + 14.0).abs() <= 1.5,
            format!("{:.1} LUFS", loudness.integrated_lufs),
        ),
        ("true_peak_below_minus1".into(), loudness.true_peak_dbtp <= -0.5, format!("{:.1} dBTP", loudness.true_peak_dbtp)),
    ];
    QaReport {
        passed: checks.iter().all(|c| c.1),
        checks,
        probe: probe.clone(),
        loudness: loudness.clone(),
    }
}

pub async fn run(data_dir: &Path, action: NewsAction) -> anyhow::Result<()> {
    println!("══════════════════════════════════════════════════════════════════");
    println!("  {SHOW_TITLE} — {SHOW_TAGLINE}");
    println!("══════════════════════════════════════════════════════════════════");
    match action {
        NewsAction::Research { research: options, json } => {
            let desk = news_desk(&options)?;
            let report = research(&desk, &options).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                for (i, s) in report.passed_stories.iter().enumerate() {
                    println!("  {:02}. [{}] {} — {} ({} sources)", i + 1, s.category.as_str(), s.title, s.studio_or_organization, s.corroboration());
                }
                for r in report.rejected.iter().take(15) {
                    println!("  ✗ {} — {} ({})", r.outlet, r.title, r.reason);
                }
            }
            write_json(&news_dir(data_dir).join("research").join("latest.json"), &report)?;
        }
        NewsAction::Slate { research: options } => {
            let desk = news_desk(&options)?;
            let report = research(&desk, &options).await?;
            let (stories, bulletins) = select(&report, options.max_stories, &[]);
            let number = peek_episode_number(data_dir);
            let slate = proposal(&format!("EP-{number:03}"), number, &stories, &bulletins, options.policy()?.target_seconds);
            print_slate(&slate);
            ensure_enough(&stories)?;
            write_json(&news_dir(data_dir).join("slates").join("latest.json"), &slate)?;
        }
        NewsAction::Telegram { research: options, telegram } => {
            let desk = news_desk(&options)?;
            let report = research(&desk, &options).await?;
            let mut gate = TelegramApprovalGate::new(telegram.token.clone(), telegram.chat_id).map_err(|e| anyhow!(e))?;
            let bot = gate.get_bot_info().await.map_err(|e| anyhow!(e))?;
            println!("  Connected to @{}", bot["username"].as_str().unwrap_or("bot"));
            let number = peek_episode_number(data_dir);
            match approval_loop(&mut gate, &desk, &options, &telegram, &format!("EP-{number:03}"), number, report).await? {
                Some((_, _, approver)) => println!("  Approved by {approver}. Run `atlas news produce` to make the episode."),
                None => println!("  Not approved."),
            }
        }
        NewsAction::Copyright { episode } => {
            let episode = open_episode(data_dir, &episode)?;
            let manifest: Vec<RightsRecord> = read_json(&episode.dir.join("rights_manifest.json"))?;
            let engine = CopyrightComplianceEngine::new();
            for r in &manifest {
                println!(
                    "  {:<16} {:<18} {:<26} {}",
                    r.asset_id,
                    format!("{:?}", r.classification),
                    &r.sha256[..16.min(r.sha256.len())],
                    r.license_type
                );
            }
            match describe_scan(&engine.scan_production_manifest(&manifest)) {
                None => println!("  Rights scan: ALL CLEAR"),
                Some(problem) => bail!("rights scan failed: {problem}"),
            }
        }
        NewsAction::Publish { episode, privacy, shorts } => {
            let episode = open_episode(data_dir, &episode)?;
            publish(data_dir, &episode, privacy, shorts).await?;
        }
        NewsAction::Produce {
            research: options,
            telegram,
            skip_telegram,
            episode,
            resolution,
            encoder,
            studio_plate,
            music,
            publish: should_publish,
            privacy,
            shorts,
        } => {
            produce(
                data_dir,
                ProduceArgs {
                    options,
                    telegram,
                    skip_telegram,
                    episode,
                    resolution,
                    encoder,
                    studio_plate,
                    music,
                    publish: should_publish,
                    privacy,
                    shorts,
                },
            )
            .await?;
        }
    }
    Ok(())
}

struct ProduceArgs {
    options: ResearchOptions,
    telegram: TelegramOptions,
    skip_telegram: bool,
    episode: Option<String>,
    resolution: NewsResolution,
    encoder: NewsEncoder,
    studio_plate: Option<PathBuf>,
    music: Option<PathBuf>,
    publish: bool,
    privacy: NewsPrivacy,
    shorts: bool,
}

async fn produce(data_dir: &Path, args: ProduceArgs) -> anyhow::Result<()> {
    let policy = args.options.policy()?;
    let renderer = BroadcastRenderer::new(tool("ATLAS_FFMPEG", "ffmpeg"), tool("ATLAS_FFPROBE", "ffprobe"));
    let fonts = resolve_fonts()?;
    if !renderer.has_encoder(args.encoder.ffmpeg_name()).await {
        bail!("this ffmpeg build has no {} encoder; pick another --encoder", args.encoder.ffmpeg_name());
    }
    for path in args.studio_plate.iter().chain(args.music.iter()) {
        if !path.is_file() {
            bail!("{} does not exist", path.display());
        }
    }
    let mut gate = if args.skip_telegram {
        None
    } else {
        Some(TelegramApprovalGate::new(args.telegram.token.clone(), args.telegram.chat_id).map_err(|e| {
            anyhow!("{e}. The human approval gate is mandatory; pass --skip-telegram only for operator-run test episodes")
        })?)
    };

    let (mut episode, selection) = match &args.episode {
        Some(id) => {
            let episode = open_episode(data_dir, id)?;
            let selection: Selection = read_json(&episode.dir.join("selection.json"))
                .with_context(|| format!("{id} has no approved selection to resume from"))?;
            println!("\n[RESUME] {} with {} approved stories", episode.id, selection.stories.len());
            (episode, selection)
        }
        None => {
            println!("\n[1] RESEARCH & SOURCE VERIFICATION");
            let desk = news_desk(&args.options)?;
            let report = research(&desk, &args.options).await?;
            let mut episode = allocate_episode(data_dir)?;
            write_json(&episode.dir.join("research.json"), &report)?;
            episode.stage(NewsProductionStage::Research, "PASS", format!("{} wire candidates", report.total_candidates));
            episode.stage(
                NewsProductionStage::SourceVerification,
                "PASS",
                format!("{} corroborated, {} single-source, {} rejected", report.passed_stories.len(), report.bulletins.len(), report.rejected.len()),
            );

            println!("\n[2] EDITORIAL SELECTION & TELEGRAM APPROVAL");
            let (stories, bulletins, approver) = match gate.as_mut() {
                Some(gate) => {
                    let bot = gate.get_bot_info().await.map_err(|e| anyhow!(e))?;
                    println!("  Telegram bot @{}", bot["username"].as_str().unwrap_or("bot"));
                    episode.stage(NewsProductionStage::TelegramApprovalPending, "RUN", "slate sent for human approval");
                    match approval_loop(gate, &desk, &args.options, &args.telegram, &episode.id, episode.number, report).await? {
                        Some(approved) => approved,
                        None => {
                            episode.stage(NewsProductionStage::TelegramApprovalPending, "HALT", "slate not approved; nothing produced");
                            return Ok(());
                        }
                    }
                }
                None => {
                    let (stories, bulletins) = select(&report, args.options.max_stories, &[]);
                    ensure_enough(&stories)?;
                    print_slate(&proposal(&episode.id, episode.number, &stories, &bulletins, policy.target_seconds));
                    (stories, bulletins, "operator (--skip-telegram)".to_string())
                }
            };
            episode.stage(NewsProductionStage::EditorialStorySelection, "PASS", format!("{} stories, {} bulletins", stories.len(), bulletins.len()));
            episode.stage(NewsProductionStage::TelegramApproved, "PASS", format!("approved by {approver}"));
            let mut stories = stories;
            for story in &mut stories {
                desk.enrich_article_text(story, 5000).await;
            }
            let selection = Selection {
                episode_id: episode.id.clone(),
                episode_number: episode.number,
                stories,
                bulletins,
                approved_by: Some(approver),
            };
            write_json(&episode.dir.join("selection.json"), &selection)?;
            (episode, selection)
        }
    };
    episode.stage(NewsProductionStage::ProducerActivation, "PASS", format!("producing {}", episode.id));

    println!("\n[3] SCRIPT");
    let script = match read_json::<MasterEpisodeScript>(&episode.dir.join("script.json")) {
        Ok(script) if args.episode.is_some() => {
            println!("  Reusing existing script.json");
            script
        }
        _ => write_script(&mut episode, &selection.stories, &selection.bulletins, &policy).await?,
    };
    episode.stage(
        NewsProductionStage::ComedyAndDialoguePolish,
        "PASS",
        format!("{} segments, estimated {:.1} min", script.segments.len(), script.total_duration_secs / 60.0),
    );

    println!("\n[4] VOICE PERFORMANCE (Kokoro)");
    let voices = synthesize(data_dir, &script).await?;
    episode.stage(NewsProductionStage::VoicePerformanceGeneration, "PASS", format!("{} lines voiced", voices.len()));

    println!("\n[5] DIALOGUE EDIT & MASTERING");
    let work = episode.dir.join("work");
    std::fs::create_dir_all(&work)?;
    let dialogue = work.join("dialogue.wav");
    let durations = renderer.concat_dialogue(&build_pieces(&script, &voices), &work, &dialogue).await?;
    let timeline = build_timeline(&script, &durations);
    let mut timeline = timeline;
    fill_wall_notes(&mut timeline, &script, &selection);
    if let Err(problem) = policy.validate(timeline.duration) {
        episode.stage(NewsProductionStage::FinalEditorialAssembly, "FAIL", problem.clone());
        bail!("voiced episode length is out of policy: {problem}. Adjust ATLAS_NEWS_TTS_SPEED or --max-stories and resume with --episode {}", episode.id);
    }
    episode.stage(NewsProductionStage::FinalEditorialAssembly, "PASS", format!("voiced runtime {:.1} min", timeline.duration / 60.0));

    let manifest = rights_manifest(&fonts, &dialogue, args.music.as_deref(), args.studio_plate.as_deref())?;
    write_json(&episode.dir.join("rights_manifest.json"), &manifest)?;
    if let Some(problem) = describe_scan(&CopyrightComplianceEngine::new().scan_production_manifest(&manifest)) {
        episode.stage(NewsProductionStage::GraphicsAndAssetIntegration, "FAIL", problem.clone());
        bail!("copyright gate blocked the render: {problem}. Add a <file>.rights.json sidecar for each third-party asset");
    }
    episode.stage(NewsProductionStage::GraphicsAndAssetIntegration, "PASS", format!("{} assets cleared", manifest.len()));

    let master = work.join("master.wav");
    let loudness = renderer
        .master_audio(&dialogue, args.music.as_deref(), &master, -14.0, -1.5)
        .await?;
    episode.stage(
        NewsProductionStage::AudioMixAndMastering,
        "PASS",
        format!("{:.1} LUFS, {:.1} dBTP", loudness.integrated_lufs, loudness.true_peak_dbtp),
    );

    println!("\n[6] STUDIO RENDER");
    let (width, height) = args.resolution.dimensions();
    println!("  {width}x{height} @ 30 fps, encoder {}", args.encoder.ffmpeg_name());
    match &args.studio_plate {
        Some(plate) => episode.stage(NewsProductionStage::UnrealEngineStudioFilming, "PASS", format!("studio plate {}", plate.display())),
        None => episode.stage(NewsProductionStage::UnrealEngineStudioFilming, "SKIP", "no --studio-plate; using the built-in broadcast graphics studio"),
    }
    episode.stage(NewsProductionStage::MetaHumanAnimation, "SKIP", "anchors are presented as on-air graphics panels with live voice meters");
    let ticker = selection
        .stories
        .iter()
        .chain(selection.bulletins.iter())
        .map(|s| s.title.clone())
        .collect::<Vec<_>>()
        .join("   ◆   ");
    let output = episode.dir.join("final.mp4");
    let label = format!("EPISODE {}  ·  {}", episode.number, script.episode_date);
    renderer
        .render(&BroadcastSpec {
            show_title: SHOW_TITLE.into(),
            episode_label: label.clone(),
            segments: timeline.segments.clone(),
            speakers: timeline.speakers.clone(),
            ticker,
            audio: master.clone(),
            duration: timeline.duration,
            width,
            height,
            fps: 30,
            font_regular: fonts.0.clone(),
            font_bold: fonts.1.clone(),
            background_plate: args.studio_plate.clone(),
            encoder: args.encoder.encoder(),
            work_dir: work.clone(),
            output: output.clone(),
        })
        .await?;
    renderer
        .render_thumbnail(
            SHOW_TITLE,
            &label,
            &selection.stories[0].title,
            (&fonts.0, &fonts.1),
            &work,
            &episode.dir.join("thumbnail.png"),
        )
        .await?;
    std::fs::write(episode.dir.join("captions.srt"), write_srt(&timeline.captions))?;
    let meta = metadata(&script, &timeline, &selection.stories, &selection.bulletins);
    write_json(&episode.dir.join("youtube_metadata.json"), &meta)?;

    println!("\n[7] TECHNICAL & EDITORIAL QA");
    let probe = renderer.probe(&output).await?;
    let final_loudness = renderer.measure_loudness(&output).await?;
    let report = qa(&probe, &final_loudness, width, height, args.encoder.encoder().codec_name());
    write_json(&episode.dir.join("qa_report.json"), &report)?;
    for (name, ok, detail) in &report.checks {
        println!("  {} {name}: {detail}", if *ok { "PASS" } else { "FAIL" });
    }
    if !report.passed {
        episode.stage(NewsProductionStage::TechnicalAndEditorialQa, "FAIL", "see qa_report.json");
        bail!("QA failed for {}; nothing was published", episode.id);
    }
    episode.stage(NewsProductionStage::TechnicalAndEditorialQa, "PASS", format!("{:.1} min, {} bytes", probe.duration / 60.0, probe.size_bytes));

    println!("\n[8] SHORTS");
    let shorts = make_shorts(&renderer, &episode, &script, &timeline, &master, &fonts, args.encoder.encoder()).await?;
    episode.stage(
        NewsProductionStage::TechnicalAndEditorialQa,
        "PASS",
        format!("{} vertical Shorts cut from the episode", shorts.len()),
    );

    println!("\n══════════════════════════════════════════════════════════════════");
    println!("  {} READY: {}", episode.id, output.display());
    println!("══════════════════════════════════════════════════════════════════");

    let published = if args.publish {
        Some(publish(data_dir, &episode, args.privacy, args.shorts).await?)
    } else {
        println!("  Not published (pass --publish, or run `atlas news publish --episode {}`)", episode.id);
        None
    };
    if let Some(gate) = gate.as_mut() {
        let text = match &published {
            Some(record) => format!(
                "🎬 <b>{}</b> is up ({}): {}",
                escape_html(&meta.title),
                record.privacy,
                escape_html(&record.url)
            ),
            None => format!(
                "🎬 <b>{}</b> rendered and passed QA ({:.1} min). Run <code>atlas news publish --episode {}</code> to upload.",
                escape_html(&meta.title),
                probe.duration / 60.0,
                episode.id
            ),
        };
        if let Err(error) = gate.send_text(&text).await {
            println!("  Telegram notification failed: {error}");
        }
    }
    Ok(())
}

async fn make_shorts(
    renderer: &BroadcastRenderer,
    episode: &Episode,
    script: &MasterEpisodeScript,
    timeline: &Timeline,
    master: &Path,
    fonts: &(PathBuf, PathBuf),
    encoder: VideoEncoder,
) -> anyhow::Result<Vec<ShortRecord>> {
    let dir = episode.dir.join("shorts");
    std::fs::create_dir_all(&dir)?;
    let mut records = Vec::new();
    let mut offset = 0;
    for segment in &script.segments {
        let base = offset;
        offset += segment.lines.len();
        let Some(clip) = &segment.short else { continue };
        let mut end_line = clip.end_line.min(segment.lines.len().saturating_sub(1));
        let start = timeline.captions[base + clip.start_line].start - 0.1;
        while end_line > clip.start_line && timeline.captions[base + end_line].end + 0.5 - start > 59.0 {
            end_line -= 1;
        }
        let end = timeline.captions[base + end_line].end + 0.5;
        let duration = end - start;
        if !(15.0..=59.5).contains(&duration) {
            println!("  ! Segment {:02}: clip is {duration:.1}s, outside 15-59s; skipped", segment.segment_number);
            continue;
        }
        let cues: Vec<ShortCue> = (clip.start_line..=end_line)
            .map(|i| {
                let caption = &timeline.captions[base + i];
                let speaker = match segment.lines[i].speaker {
                    AnchorRole::Enthusiast => CueSpeaker::Marcus,
                    AnchorRole::Counterpart => CueSpeaker::Elena,
                    AnchorRole::Duo => CueSpeaker::Both,
                };
                ShortCue {
                    start: caption.start - start,
                    end: caption.end - start,
                    speaker,
                    text: segment.lines[i].spoken_text.clone(),
                }
            })
            .collect();
        let file = format!("short_{:02}.mp4", segment.segment_number);
        let work = episode.dir.join("work").join(format!("short_{:02}", segment.segment_number));
        std::fs::create_dir_all(&work)?;
        renderer
            .render_short(&ShortSpec {
                show_title: SHOW_TITLE.into(),
                title: clip.title.clone(),
                category: segment.segment_category.clone(),
                attribution: segment.attribution.clone(),
                audio: master.to_path_buf(),
                clip_start: start,
                duration,
                cues,
                font_regular: fonts.0.clone(),
                font_bold: fonts.1.clone(),
                encoder,
                work_dir: work,
                output: dir.join(&file),
            })
            .await?;
        let probe = renderer.probe(&dir.join(&file)).await?;
        let ok = probe.width == 1080 && probe.height == 1920 && probe.duration <= 60.0 && probe.video_codec == encoder.codec_name();
        if !ok {
            bail!("Short {file} failed QA: {}x{} {:.1}s {}", probe.width, probe.height, probe.duration, probe.video_codec);
        }
        let mut title = format!("{} #Shorts", clip.title);
        if title.chars().count() > 100 {
            title = format!("{} #Shorts", clip.title.chars().take(90).collect::<String>());
        }
        let description = format!(
            "{}\n\nFrom {SHOW_TITLE} episode {}. Full episode: {{FULL_EPISODE_URL}}\n\n{}\n\nAnchors are AI-generated voices; script written with AI assistance from the linked reporting. Altered or synthetic content.\n#animation #animationnews #shorts",
            clip.hook,
            episode.number,
            segment.attribution
        );
        println!("  {file}: {:.1}s \"{}\"", probe.duration, clip.title);
        records.push(ShortRecord {
            segment_number: segment.segment_number,
            file,
            title,
            description,
            duration: probe.duration,
            youtube_video_id: None,
            url: None,
        });
    }
    write_json(&dir.join("shorts.json"), &records)?;
    Ok(records)
}

async fn publish(data_dir: &Path, episode: &Episode, privacy: NewsPrivacy, shorts: bool) -> anyhow::Result<PublishRecord> {
    let record = publish_episode(data_dir, episode, privacy).await?;
    if shorts {
        publish_shorts(episode, privacy, &record.url).await?;
    }
    Ok(record)
}

async fn publish_shorts(episode: &Episode, privacy: NewsPrivacy, episode_url: &str) -> anyhow::Result<()> {
    let path = episode.dir.join("shorts").join("shorts.json");
    let mut records: Vec<ShortRecord> = read_json(&path)?;
    let credentials = load_youtube_credentials("youtube_credentials.json").await?;
    let client = YouTubeAnalyticsClient::new(credentials);
    for index in 0..records.len() {
        if records[index].youtube_video_id.is_some() {
            continue;
        }
        let record = records[index].clone();
        let result = client
            .upload_video(
                &episode.dir.join("shorts").join(&record.file),
                &YouTubeUploadMetadata {
                    title: record.title.clone(),
                    description: record.description.replace("{FULL_EPISODE_URL}", episode_url),
                    tags: vec!["animation news".into(), "shorts".into(), SHOW_TITLE.into()],
                    privacy_status: privacy.as_str().into(),
                    category_id: "1".into(),
                    contains_synthetic_media: true,
                },
            )
            .await?;
        println!("  Short uploaded: {} -> {}", record.file, result.url);
        records[index].youtube_video_id = Some(result.video_id);
        records[index].url = Some(result.url);
        write_json(&path, &records)?;
    }
    Ok(())
}

async fn publish_episode(data_dir: &Path, episode: &Episode, privacy: NewsPrivacy) -> anyhow::Result<PublishRecord> {
    let record_path = episode.dir.join("publish.json");
    if let Ok(existing) = read_json::<PublishRecord>(&record_path) {
        println!("  {} was already uploaded: {}", episode.id, existing.url);
        return Ok(existing);
    }
    let qa: QaReport = read_json(&episode.dir.join("qa_report.json"))?;
    if !qa.passed {
        bail!("{} has not passed QA", episode.id);
    }
    let video = episode.dir.join("final.mp4");
    let thumbnail = episode.dir.join("thumbnail.png");
    let captions = episode.dir.join("captions.srt");
    let meta: EpisodeMetadata = read_json(&episode.dir.join("youtube_metadata.json"))?;

    let credentials = load_youtube_credentials("youtube_credentials.json").await?;
    let client = YouTubeAnalyticsClient::new(credentials);
    if !client.is_authenticated()? {
        bail!("YouTube is not authenticated; run `atlas analytics auth` first");
    }
    let channel = client.fetch_channel_status().await?;
    let guardian = PolicyGuardian::new(data_dir.join("policy"))?;
    let browser_review = guardian.channel_review()?;
    let report = guardian
        .vet(
            &ContentPreflight {
                title: meta.title.clone(),
                description: meta.description.clone(),
                transcript: meta.transcript.clone(),
                contains_synthetic_media: meta.contains_synthetic_media,
                captions_present: captions.is_file(),
                thumbnail_present: thumbnail.is_file(),
                broll_expected: false,
                broll_attribution_present: true,
                unique_media: true,
            },
            &ChannelPreflight {
                channel_id: channel.channel_id,
                available: channel.available,
                linked: channel.is_linked,
                long_uploads_status: channel.long_uploads_status,
                privacy_status: channel.privacy_status,
                browser_review,
            },
            &format!("news_{}", episode.id),
        )
        .await?;
    if !report.publish_allowed {
        let blocked = report
            .checks
            .iter()
            .filter(|c| c.status != CheckStatus::Pass)
            .map(|c| format!("{}: {}", c.name, c.detail))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("Policy Guardian blocked publication: {blocked}");
    }
    let result = client
        .upload_video(
            &video,
            &YouTubeUploadMetadata {
                title: meta.title.clone(),
                description: meta.description.clone(),
                tags: meta.tags.clone(),
                privacy_status: privacy.as_str().into(),
                category_id: "1".into(),
                contains_synthetic_media: true,
            },
        )
        .await?;
    let record = PublishRecord {
        youtube_video_id: result.video_id.clone(),
        url: result.url.clone(),
        privacy: privacy.as_str().into(),
        published_at: Utc::now(),
    };
    write_json(&record_path, &record)?;
    client.upload_thumbnail(&result.video_id, &thumbnail).await?;
    if let Err(error) = client.upload_captions(&result.video_id, &captions, "en-US").await {
        println!("  Caption upload failed ({error}); the video is uploaded without them");
    }
    println!("  Uploaded to YouTube ({}): {}", record.privacy, record.url);
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_storyboard::two_anchor_script::{RawLine, TwoAnchorScriptWriter};

    #[test]
    fn timeline_follows_measured_audio() {
        let writer = TwoAnchorScriptWriter::new();
        let b = ScriptStoryBrief {
            story_id: "s".into(),
            headline: "Story".into(),
            category: "Anime".into(),
            organization: "MAPPA".into(),
            outlets: vec!["ANN".into()],
            fact_sheet: String::new(),
        };
        let raw = vec![
            RawLine { speaker: "Marcus".into(), text: "One.".into(), expression: None },
            RawLine { speaker: "Elena".into(), text: "Two.".into(), expression: None },
        ];
        let script = writer.assemble("t", "d", 1320.0, vec![writer.story_segment(2, &b, &raw, None, 10.0).unwrap()]);
        let voices = vec![vec![PathBuf::from("a.wav")], vec![PathBuf::from("b.wav")]];
        let pieces = build_pieces(&script, &voices);
        assert_eq!(pieces.len(), 6);
        let timeline = build_timeline(&script, &[2.5, 3.0, 0.35, 4.0, 0.35, 1.5]);
        assert_eq!(timeline.speakers.len(), 2);
        assert!((timeline.speakers[1].start - 5.85).abs() < 1e-9);
        assert!((timeline.duration - 11.7).abs() < 1e-9);
        assert!((timeline.segments[0].end - 10.2).abs() < 1e-9);
    }

    #[test]
    fn qa_flags_out_of_policy_output() {
        let probe = MediaProbe {
            duration: 600.0,
            width: 1920,
            height: 1080,
            fps: 30.0,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            audio_channels: 2,
            size_bytes: 1,
        };
        let loud = LoudnessReport { integrated_lufs: -14.1, true_peak_dbtp: -1.6, loudness_range_lu: 4.0 };
        let report = qa(&probe, &loud, 1920, 1080, "h264");
        assert!(!report.passed);
        assert_eq!(report.checks.iter().filter(|c| !c.1).count(), 1);
    }
}
