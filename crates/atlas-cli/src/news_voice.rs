use super::*;
use atlas_storyboard::two_anchor_script::{AnchorRole, MasterEpisodeScript};

const DEFAULT_MARCUS_VOICE: &str = "am_michael";
const DEFAULT_ELENA_VOICE: &str = "af_bella";
const LINE_SPEED: f32 = 1.0;

pub(crate) struct AnchorVoices {
    marcus: String,
    elena: String,
}

impl AnchorVoices {
    pub(crate) fn from_env() -> Self {
        let pick = |name: &str, fallback: &str| {
            std::env::var(name)
                .ok()
                .filter(|voice| !voice.trim().is_empty())
                .unwrap_or_else(|| fallback.to_string())
        };
        Self {
            marcus: pick("ATLAS_MARCUS_VOICE", DEFAULT_MARCUS_VOICE),
            elena: pick("ATLAS_ELENA_VOICE", DEFAULT_ELENA_VOICE),
        }
    }

    fn for_speaker(&self, speaker: AnchorRole) -> &str {
        match speaker {
            AnchorRole::Counterpart => &self.elena,
            AnchorRole::Enthusiast | AnchorRole::Duo => &self.marcus,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct VoicedLine {
    pub(crate) segment_number: usize,
    pub(crate) line_index: usize,
    pub(crate) speaker: AnchorRole,
    pub(crate) voice: String,
    pub(crate) text: String,
    pub(crate) file: PathBuf,
    pub(crate) start_seconds: f32,
    pub(crate) duration_seconds: f32,
}

pub(crate) enum VoiceOutcome {
    Voiced {
        narration: PathBuf,
        total_seconds: f32,
        lines: usize,
        loudness: atlas_ffmpeg::LoudnessReport,
    },
    Skipped(String),
}

impl VoiceOutcome {
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Voiced { total_seconds, lines, loudness, .. } => format!(
                "Anchor voices synthesized: {lines} lines, {:.1} minutes of narration at {:.1} LUFS / {:.1} dBTP",
                total_seconds / 60.0,
                loudness.output_lufs,
                loudness.output_true_peak_dbtp
            ),
            Self::Skipped(reason) => format!("Anchor voices skipped: {reason}"),
        }
    }

    pub(crate) fn is_voiced(&self) -> bool {
        matches!(self, Self::Voiced { .. })
    }
}

struct PlannedLine {
    segment_number: usize,
    line_index: usize,
    speaker: AnchorRole,
    segment: NarrationSegment,
}

fn plan_lines(script: &MasterEpisodeScript, voices: &AnchorVoices) -> Vec<PlannedLine> {
    script
        .segments
        .iter()
        .flat_map(|segment| {
            segment.lines.iter().enumerate().map(move |(index, line)| (segment.segment_number, index, line))
        })
        .filter(|(_, _, line)| !strip_stage_tags(&line.spoken_text).is_empty())
        .enumerate()
        .map(|(order, (segment_number, line_index, line))| {
            let phonetic = strip_stage_tags(&line.phonetic_text);
            let text = if phonetic.is_empty() { strip_stage_tags(&line.spoken_text) } else { phonetic };
            PlannedLine {
                segment_number,
                line_index,
                speaker: line.speaker,
                segment: NarrationSegment {
                    segment_id: format!("news_{:04}", order + 1),
                    text,
                    voice: voices.for_speaker(line.speaker).to_string(),
                    speed: LINE_SPEED,
                    pitch: 1.0,
                    emphasis_words: Vec::new(),
                    pause_after_ms: 0,
                    pronunciation_hints: Vec::new(),
                },
            }
        })
        .collect()
}

/// Removes camera and visual direction tags such as `[BANTER]`, `[REACTION:elena]`, or
/// `[VISUAL: cards/ghibli.png]` (any square-bracket tag) so they are never read aloud, then tidies
/// the spacing left behind. An unclosed `[` is kept as ordinary text.
pub(crate) fn strip_stage_tags(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        match rest[open..].find(']') {
            Some(close) => {
                kept.push_str(&rest[..open]);
                kept.push(' ');
                rest = &rest[open + close + 1..];
            }
            None => break,
        }
    }
    kept.push_str(rest);
    let mut tidy = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    for mark in [",", ".", "!", "?", ";", ":"] {
        tidy = tidy.replace(&format!(" {mark}"), mark);
    }
    tidy
}

fn speaker_slug(speaker: AnchorRole) -> &'static str {
    match speaker {
        AnchorRole::Enthusiast => "marcus",
        AnchorRole::Counterpart => "elena",
        AnchorRole::Duo => "duo",
    }
}

fn build_timeline(planned: &[PlannedLine], durations: &[f32], lines_dir: &Path) -> Vec<VoicedLine> {
    let mut start = 0.0_f32;
    planned
        .iter()
        .zip(durations)
        .map(|(line, duration)| {
            let voiced = VoicedLine {
                segment_number: line.segment_number,
                line_index: line.line_index,
                speaker: line.speaker,
                voice: line.segment.voice.clone(),
                text: line.segment.text.clone(),
                file: lines_dir.join(format!(
                    "{:02}_{:03}_{}.wav",
                    line.segment_number,
                    line.line_index + 1,
                    speaker_slug(line.speaker)
                )),
                start_seconds: start,
                duration_seconds: *duration,
            };
            start += duration;
            voiced
        })
        .collect()
}

/// Which TTS renders the anchors. `ATLAS_TTS_ENGINE=voicebox` uses the local Voicebox app (profiles
/// `CDN Marcus` / `CDN Elena`) when its server answers; otherwise, and by default, Kokoro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TtsEngine {
    Kokoro,
    Voicebox,
}

pub(crate) fn tts_engine(lookup: impl Fn(&str) -> Option<String>) -> TtsEngine {
    match lookup("ATLAS_TTS_ENGINE").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("voicebox") => TtsEngine::Voicebox,
        _ => TtsEngine::Kokoro,
    }
}

const DEFAULT_VOICEBOX_URL: &str = "http://127.0.0.1:17493";
const DEFAULT_VOICEBOX_CLI: &str = r"C:\Users\chrom\Tools\voicebox-cli\vbcli.py";

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| fallback.to_string())
}

fn voicebox_cli() -> PathBuf {
    let configured = PathBuf::from(env_or("ATLAS_VOICEBOX_CLI", DEFAULT_VOICEBOX_CLI));
    if configured.is_file() {
        configured
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../integrations/voicebox-cli/vbcli.py")
    }
}

/// `SPEAKER: text` cast for vb-batch: Marcus and the duo lines use `CDN Marcus`, Elena uses `CDN Elena`.
fn voicebox_cast() -> [(String, String); 3] {
    let marcus = env_or("ATLAS_VOICEBOX_MARCUS", "CDN Marcus");
    let elena = env_or("ATLAS_VOICEBOX_ELENA", "CDN Elena");
    [("marcus".into(), marcus.clone()), ("elena".into(), elena), ("duo".into(), marcus)]
}

struct RenderedLines {
    files: Vec<PathBuf>,
    durations: Vec<f32>,
    combined: PathBuf,
    voices: serde_json::Value,
}

async fn voicebox_lines(episode_dir: &Path, planned: &[PlannedLine]) -> anyhow::Result<RenderedLines> {
    let base = env_or("VOICEBOX_URL", DEFAULT_VOICEBOX_URL);
    let alive = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?
        .get(format!("{}/health", base.trim_end_matches('/')))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false);
    if !alive {
        anyhow::bail!("Voicebox app is not running at {base}");
    }
    let out_dir = episode_dir.join("audio").join("voicebox");
    std::fs::create_dir_all(&out_dir)?;
    let script = out_dir.join("script.json");
    let rows = planned
        .iter()
        .map(|line| serde_json::json!({ "speaker": speaker_slug(line.speaker), "text": line.segment.text }))
        .collect::<Vec<_>>();
    std::fs::write(&script, serde_json::to_vec_pretty(&rows)?)?;
    let python = env_or("ATLAS_PYTHON", if cfg!(windows) { "py" } else { "python3" });
    let mut command = tokio::process::Command::new(python);
    command.arg(voicebox_cli()).arg("batch").arg(&script).arg("-o").arg(&out_dir);
    let cast = voicebox_cast();
    for (speaker, profile) in &cast {
        command.arg("--cast").arg(format!("{speaker}={profile}"));
    }
    let output = command.args(["--gap", "0", "--concat"]).env("VOICEBOX_URL", &base).kill_on_drop(true).output().await?;
    if !output.status.success() {
        anyhow::bail!("vb-batch failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(out_dir.join("manifest.json"))?)?;
    let lines = manifest["lines"].as_array().cloned().unwrap_or_default();
    if lines.len() != planned.len() {
        anyhow::bail!("vb-batch voiced {} of {} lines", lines.len(), planned.len());
    }
    let combined = out_dir.join(manifest["full_mix"].as_str().unwrap_or("full.wav"));
    if !combined.is_file() {
        anyhow::bail!("vb-batch did not write the combined track");
    }
    Ok(RenderedLines {
        files: lines.iter().map(|l| out_dir.join(l["file"].as_str().unwrap_or_default())).collect(),
        durations: lines.iter().map(|l| l["duration"].as_f64().unwrap_or(0.0) as f32).collect(),
        combined,
        voices: serde_json::json!({ "engine": "voicebox", "marcus": cast[0].1, "elena": cast[1].1 }),
    })
}

async fn kokoro_lines(data_dir: &Path, episode_id: &str, planned: &[PlannedLine], voices: &AnchorVoices) -> anyhow::Result<RenderedLines> {
    let pipeline = KokoroPipeline::new("kokoro-tts", VoiceConfig::default(), data_dir)
        .with_parallelism(env_parallelism("ATLAS_TTS_JOBS"));
    let track = pipeline
        .synthesize_track(
            planned.iter().map(|line| line.segment.clone()).collect(),
            format!("news_{episode_id}"),
        )
        .await?;
    let combined = track
        .combined_file
        .ok_or_else(|| anyhow::anyhow!("Kokoro did not produce a combined narration track"))?;
    Ok(RenderedLines {
        files: track.segments.iter().map(|audio| audio.file_path.clone()).collect(),
        durations: track.segments.iter().map(|audio| audio.duration_seconds).collect(),
        combined,
        voices: serde_json::json!({ "engine": "kokoro", "marcus": voices.marcus, "elena": voices.elena }),
    })
}

pub(crate) async fn voice_episode(
    data_dir: &Path,
    episode_dir: &Path,
    episode_id: &str,
    script: &MasterEpisodeScript,
) -> anyhow::Result<VoiceOutcome> {
    let (ffmpeg, ffprobe) = tokio::join!(atlas_doctor::probe_ffmpeg(), atlas_doctor::probe_ffprobe());
    let missing = [ffmpeg, ffprobe]
        .into_iter()
        .filter(|probe| !probe.status.is_available())
        .map(|probe| probe.kind.to_string())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Ok(VoiceOutcome::Skipped(format!("{} not available (run `atlas doctor`)", missing.join(", "))));
    }

    let voices = AnchorVoices::from_env();
    let planned = plan_lines(script, &voices);
    if planned.is_empty() {
        return Ok(VoiceOutcome::Skipped("script has no spoken lines".into()));
    }

    let mut rendered = None;
    if tts_engine(|name| std::env::var(name).ok()) == TtsEngine::Voicebox {
        match voicebox_lines(episode_dir, &planned).await {
            Ok(lines) => rendered = Some(lines),
            Err(error) => eprintln!("  Voicebox unavailable, falling back to Kokoro: {error}"),
        }
    }
    let rendered = match rendered {
        Some(lines) => lines,
        None => {
            let kokoro = atlas_doctor::probe_kokoro().await;
            if !kokoro.status.is_available() {
                return Ok(VoiceOutcome::Skipped(format!("{} not available (run `atlas doctor`)", kokoro.kind)));
            }
            kokoro_lines(data_dir, episode_id, &planned, &voices).await?
        }
    };

    let audio_dir = episode_dir.join("audio");
    let lines_dir = audio_dir.join("lines");
    std::fs::create_dir_all(&lines_dir)?;
    let timeline = build_timeline(&planned, &rendered.durations, &lines_dir);
    for (line, file) in timeline.iter().zip(&rendered.files) {
        std::fs::copy(file, &line.file)?;
    }
    let narration = audio_dir.join("episode_narration.wav");
    let loudness = atlas_ffmpeg::normalize_loudness(
        "ffmpeg",
        &rendered.combined,
        &narration,
        atlas_ffmpeg::LoudnessTarget::default(),
    )
    .await?;
    let total_seconds = rendered.durations.iter().sum::<f32>();
    std::fs::write(
        episode_dir.join("voice_timeline.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "narration": narration,
            "total_seconds": total_seconds,
            "voices": rendered.voices,
            "loudness": loudness,
            "lines": timeline,
        }))?,
    )?;
    let lipsync = crate::news_lipsync::write_lipsync(episode_dir, &timeline, total_seconds).await?;
    println!("  Lip-sync curves: {}", lipsync.display());
    Ok(VoiceOutcome::Voiced {
        narration,
        total_seconds,
        lines: timeline.len(),
        loudness,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::EpisodeDurationPolicy;
    use atlas_storyboard::story_script::{NewsroomWriter, StoryBrief};

    fn script() -> MasterEpisodeScript {
        let brief = StoryBrief {
            headline: "Studio Ghibli Restores Archive".into(),
            summary: "Studio Ghibli announced a cel restoration project. (Source: Anime News Network)".into(),
            category: "Anime".into(),
            organization: "Studio Ghibli".into(),
            outlets: vec!["Anime News Network".into()],
        };
        NewsroomWriter::new()
            .template_episode("EP-009", "2026-10-17", &[brief], &EpisodeDurationPolicy::default())
            .script
    }

    #[test]
    fn lines_get_each_anchors_voice_in_script_order() {
        let voices = AnchorVoices { marcus: "am_michael".into(), elena: "af_bella".into() };
        let script = script();
        let planned = plan_lines(&script, &voices);
        let total_lines = script.segments.iter().map(|s| s.lines.len()).sum::<usize>();
        assert_eq!(planned.len(), total_lines);
        assert_eq!(planned[0].segment.segment_id, "news_0001");
        for line in &planned {
            let expected = if line.speaker == AnchorRole::Counterpart { "af_bella" } else { "am_michael" };
            assert_eq!(line.segment.voice, expected);
        }
        assert!(planned.iter().any(|l| l.segment.text.contains("JIB-lee")));
    }

    #[test]
    fn kokoro_is_the_default_voice_engine() {
        assert_eq!(tts_engine(|_| None), TtsEngine::Kokoro);
        assert_eq!(tts_engine(|_| Some("Voicebox".into())), TtsEngine::Voicebox);
        assert_eq!(tts_engine(|_| Some("other".into())), TtsEngine::Kokoro);
        let cast = voicebox_cast();
        assert_eq!(cast[2].0, "duo");
    }

    #[test]
    fn stage_tags_are_never_spoken() {
        assert_eq!(strip_stage_tags("[BANTER] Thrilled is a strong word, Marcus."), "Thrilled is a strong word, Marcus.");
        assert_eq!(strip_stage_tags("Even the dust is hand-painted [REACTION:marcus]."), "Even the dust is hand-painted.");
        assert_eq!(
            strip_stage_tags("Ghibli opened a lab. [VISUAL: news_intro/assets/story_cards/card 1.png] [SETUP]  It restores cels."),
            "Ghibli opened a lab. It restores cels."
        );
        assert_eq!(strip_stage_tags("[WALK]"), "");
        assert_eq!(strip_stage_tags("A dangling [bracket stays"), "A dangling [bracket stays");
        assert_eq!(strip_stage_tags("No tags here."), "No tags here.");
    }

    #[test]
    fn tagged_lines_reach_kokoro_clean_and_tag_only_lines_are_skipped() {
        let voices = AnchorVoices { marcus: "m".into(), elena: "e".into() };
        let mut script = script();
        let first = &mut script.segments[0].lines[0];
        first.spoken_text = format!("[BANTER] {} [REACTION:elena]", first.spoken_text);
        first.phonetic_text = format!("[VISUAL: a.png] {}", first.phonetic_text);
        let mut tag_only = script.segments[0].lines[0].clone();
        tag_only.spoken_text = "[WALK]".into();
        tag_only.phonetic_text = String::new();
        script.segments[0].lines.push(tag_only);
        let total_lines = script.segments.iter().map(|s| s.lines.len()).sum::<usize>();
        let planned = plan_lines(&script, &voices);
        assert_eq!(planned.len(), total_lines - 1);
        assert!(planned.iter().all(|l| !l.segment.text.contains('[') && !l.segment.text.contains(']')));
        assert!(!planned[0].segment.text.is_empty());
    }

    #[test]
    fn timeline_accumulates_line_durations() {
        let voices = AnchorVoices { marcus: "m".into(), elena: "e".into() };
        let planned = plan_lines(&script(), &voices);
        let durations = vec![2.0_f32; planned.len()];
        let timeline = build_timeline(&planned, &durations, Path::new("/ep/audio/lines"));
        assert_eq!(timeline[0].start_seconds, 0.0);
        assert_eq!(timeline[3].start_seconds, 6.0);
        assert_eq!(timeline[0].file, Path::new("/ep/audio/lines/01_001_marcus.wav"));
        assert!(timeline[1].file.to_string_lossy().ends_with("_elena.wav"));
    }
}
