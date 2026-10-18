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
        .filter(|(_, _, line)| !line.spoken_text.trim().is_empty())
        .enumerate()
        .map(|(order, (segment_number, line_index, line))| {
            let text = if line.phonetic_text.trim().is_empty() {
                line.spoken_text.clone()
            } else {
                line.phonetic_text.clone()
            };
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

pub(crate) async fn voice_episode(
    data_dir: &Path,
    episode_dir: &Path,
    episode_id: &str,
    script: &MasterEpisodeScript,
) -> anyhow::Result<VoiceOutcome> {
    let (kokoro, ffmpeg, ffprobe) = tokio::join!(
        atlas_doctor::probe_kokoro(),
        atlas_doctor::probe_ffmpeg(),
        atlas_doctor::probe_ffprobe(),
    );
    let missing = [kokoro, ffmpeg, ffprobe]
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
    let pipeline = KokoroPipeline::new("kokoro-tts", VoiceConfig::default(), data_dir)
        .with_parallelism(env_parallelism("ATLAS_TTS_JOBS"));
    let track = pipeline
        .synthesize_track(
            planned.iter().map(|line| line.segment.clone()).collect(),
            format!("news_{episode_id}"),
        )
        .await?;

    let audio_dir = episode_dir.join("audio");
    let lines_dir = audio_dir.join("lines");
    std::fs::create_dir_all(&lines_dir)?;
    let durations = track.segments.iter().map(|audio| audio.duration_seconds).collect::<Vec<_>>();
    let timeline = build_timeline(&planned, &durations, &lines_dir);
    for (line, audio) in timeline.iter().zip(&track.segments) {
        std::fs::copy(&audio.file_path, &line.file)?;
    }
    let combined = track
        .combined_file
        .ok_or_else(|| anyhow::anyhow!("Kokoro did not produce a combined narration track"))?;
    let narration = audio_dir.join("episode_narration.wav");
    let loudness = atlas_ffmpeg::normalize_loudness(
        "ffmpeg",
        &combined,
        &narration,
        atlas_ffmpeg::LoudnessTarget::default(),
    )
    .await?;
    let total_seconds = durations.iter().sum::<f32>();
    std::fs::write(
        episode_dir.join("voice_timeline.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "narration": narration,
            "total_seconds": total_seconds,
            "voices": { "marcus": voices.marcus, "elena": voices.elena },
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
