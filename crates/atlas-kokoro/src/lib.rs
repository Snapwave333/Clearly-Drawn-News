use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tracing::{info, warn};

/// A single narration segment (sentence or beat) for TTS synthesis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NarrationSegment {
    pub segment_id: String,
    pub text: String,
    pub voice: String,
    pub speed: f32,
    pub pitch: f32,
    pub emphasis_words: Vec<String>,
    pub pause_after_ms: u32,
    pub pronunciation_hints: Vec<PronunciationHint>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PronunciationHint {
    pub word: String,
    pub phonemes: String,
}

/// Synthesized audio segment with timing metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioSegment {
    pub segment_id: String,
    pub file_path: PathBuf,
    pub duration_seconds: f32,
    pub sample_rate: u32,
    pub channels: u16,
    pub word_timestamps: Vec<WordTimestamp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WordTimestamp {
    pub word: String,
    pub start_seconds: f32,
    pub end_seconds: f32,
}

/// Complete narration with all segments combined.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NarrationTrack {
    pub track_id: String,
    pub segments: Vec<AudioSegment>,
    pub total_duration_seconds: f32,
    pub sample_rate: u32,
    pub combined_file: Option<PathBuf>,
}

/// Internal YouTube OAuth credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YouTubeCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

/// Voice configuration for a channel/project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceConfig {
    pub voice_id: String,
    pub default_speed: f32,
    pub default_pitch: f32,
    pub language: String,
}

/// Kokoro TTS synthesis pipeline.
#[derive(Clone)]
pub struct KokoroPipeline {
    kokoro_exe: PathBuf,
    voice_config: VoiceConfig,
    workspace_dir: PathBuf,
    parallelism: usize,
}

impl KokoroPipeline {
    pub fn new(
        kokoro_exe: impl Into<PathBuf>,
        voice_config: VoiceConfig,
        workspace_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            kokoro_exe: kokoro_exe.into(),
            voice_config,
            workspace_dir: workspace_dir.into(),
            parallelism: 1,
        }
    }

    pub fn with_parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism.max(1);
        self
    }

    /// Synthesize a single narration segment.
    pub async fn synthesize_segment(
        &self,
        segment: &NarrationSegment,
    ) -> AtlasResult<AudioSegment> {
        let output_dir = self.workspace_dir.join("audio").join("segments");
        tokio::fs::create_dir_all(&output_dir).await?;

        let cache_key = self.segment_cache_key(segment);
        let output_file = output_dir.join(format!("{cache_key}.wav"));
        let cached = match output_file.is_file() {
            true => self
                .probe_audio(&output_file)
                .await
                .ok()
                .filter(|metadata| metadata.duration_seconds > 0.0),
            false => None,
        };
        let metadata = match cached {
            Some(metadata) => {
                info!("Reusing cached narration for segment {}", segment.segment_id);
                metadata
            }
            None => {
                self.run_kokoro(segment, &output_dir, &cache_key, &output_file)
                    .await?;
                self.probe_audio(&output_file).await?
            }
        };

        let word_timestamps = self
            .generate_word_timestamps(segment, metadata.duration_seconds)
            .await?;

        Ok(AudioSegment {
            segment_id: segment.segment_id.clone(),
            file_path: output_file,
            duration_seconds: metadata.duration_seconds,
            sample_rate: metadata.sample_rate,
            channels: metadata.channels,
            word_timestamps,
        })
    }

    fn segment_cache_key(&self, segment: &NarrationSegment) -> String {
        let mut hasher = Sha256::new();
        for part in [
            segment.text.as_str(),
            segment.voice.as_str(),
            &segment.speed.to_string(),
            self.voice_config.language.as_str(),
        ] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        hex::encode(&hasher.finalize()[..16])
    }

    async fn run_kokoro(
        &self,
        segment: &NarrationSegment,
        output_dir: &Path,
        cache_key: &str,
        output_file: &Path,
    ) -> AtlasResult<()> {
        let attempt = Uuid::new_v4().simple();
        let partial_file = output_dir.join(format!("{cache_key}.{attempt}.partial.wav"));
        let input_file = output_dir.join(format!("{cache_key}.{attempt}.txt"));
        tokio::fs::write(&input_file, &segment.text).await?;
        let model_dir = directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".kokoro"))
            .ok_or_else(|| {
                AtlasError::Render("Unable to resolve the Kokoro model directory".into())
            })?;
        let model_file = model_dir.join("kokoro-v1.0.onnx");
        let voices_file = model_dir.join("voices-v1.0.bin");
        if !model_file.is_file() || !voices_file.is_file() {
            return Err(AtlasError::Render(format!(
                "Kokoro model files are missing from {}",
                model_dir.display()
            )));
        }

        let args = vec![
            input_file.to_string_lossy().to_string(),
            partial_file.to_string_lossy().to_string(),
            "--speed".to_string(),
            segment.speed.to_string(),
            "--lang".to_string(),
            self.voice_config.language.clone(),
            "--voice".to_string(),
            segment.voice.clone(),
            "--format".to_string(),
            "wav".to_string(),
            "--model".to_string(),
            model_file.to_string_lossy().to_string(),
            "--voices".to_string(),
            voices_file.to_string_lossy().to_string(),
        ];

        info!(
            "Synthesizing segment {}: '{}'",
            segment.segment_id,
            segment.text.chars().take(50).collect::<String>()
        );

        let output = Command::new(&self.kokoro_exe)
            .args(&args)
            // kokoro-tts is a Python 3.11 script whose deps (librosa) can live in the 3.11 user site. A Store-Python
            // parent (the `py` launcher) exports PYTHONUSERBASE for 3.13, which hides that site, so drop it here.
            // UTF-8 stdout keeps its progress spinner from crashing on cp1252 consoles.
            .env_remove("PYTHONUSERBASE")
            .env("PYTHONIOENCODING", "utf-8")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AtlasError::Render(format!("Failed to start Kokoro: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Kokoro synthesis failed: {}", stderr);
            return Err(AtlasError::Render(format!(
                "Kokoro exited with {}: {}",
                output.status, stderr
            )));
        }

        let _ = tokio::fs::remove_file(&input_file).await;
        if !partial_file.is_file() {
            return Err(AtlasError::Render(format!(
                "Kokoro completed without creating {}",
                partial_file.display()
            )));
        }
        tokio::fs::rename(&partial_file, output_file).await?;
        Ok(())
    }

    /// Synthesize all segments for a narration track.
    pub async fn synthesize_track(
        &self,
        segments: Vec<NarrationSegment>,
        track_id: String,
    ) -> AtlasResult<NarrationTrack> {
        let pauses = segments
            .iter()
            .map(|segment| segment.pause_after_ms as f32 / 1000.0)
            .sum::<f32>();
        let audio_segments = self.synthesize_all(segments).await?;
        let total_duration = audio_segments
            .iter()
            .map(|audio| audio.duration_seconds)
            .sum::<f32>()
            + pauses;

        let combined_file = self.combine_segments(&audio_segments, &track_id).await?;

        let sample_rate = audio_segments
            .first()
            .map_or(24000, |segment| segment.sample_rate);
        Ok(NarrationTrack {
            track_id,
            segments: audio_segments,
            total_duration_seconds: total_duration,
            sample_rate,
            combined_file: Some(combined_file),
        })
    }

    async fn synthesize_all(&self, segments: Vec<NarrationSegment>) -> AtlasResult<Vec<AudioSegment>> {
        if self.parallelism == 1 {
            let mut audio_segments = Vec::with_capacity(segments.len());
            for segment in &segments {
                audio_segments.push(self.synthesize_segment(segment).await?);
            }
            return Ok(audio_segments);
        }
        let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(self.parallelism));
        let mut tasks = tokio::task::JoinSet::new();
        let count = segments.len();
        for (index, segment) in segments.into_iter().enumerate() {
            let pipeline = self.clone();
            let permits = permits.clone();
            tasks.spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .map_err(|error| AtlasError::Render(error.to_string()))?;
                pipeline
                    .synthesize_segment(&segment)
                    .await
                    .map(|audio| (index, audio))
            });
        }
        let mut ordered: Vec<Option<AudioSegment>> = (0..count).map(|_| None).collect();
        while let Some(joined) = tasks.join_next().await {
            let joined = joined.map_err(|error| AtlasError::Render(error.to_string()))?;
            match joined {
                Ok((index, audio)) => ordered[index] = Some(audio),
                Err(error) => {
                    tasks.abort_all();
                    return Err(error);
                }
            }
        }
        Ok(ordered.into_iter().flatten().collect())
    }

    /// Combine audio segments with pauses into a single file.
    async fn combine_segments(
        &self,
        segments: &[AudioSegment],
        track_id: &str,
    ) -> AtlasResult<PathBuf> {
        let output_dir = self.workspace_dir.join("audio").join("combined");
        tokio::fs::create_dir_all(&output_dir).await?;
        let output_file = output_dir.join(format!("{}.wav", track_id));

        let mut filter_parts = Vec::new();
        let mut input_args = Vec::new();

        for (i, seg) in segments.iter().enumerate() {
            input_args.push("-i".to_string());
            input_args.push(seg.file_path.to_string_lossy().to_string());
            filter_parts.push(format!("[{}:a]", i));
        }

        let filter_complex = format!(
            "{}concat=n={}:v=0:a=1[out]",
            filter_parts.join(""),
            segments.len()
        );

        let mut args: Vec<String> = vec!["-y".to_string()];
        args.extend(input_args);
        args.extend([
            "-filter_complex".to_string(),
            filter_complex,
            "-map".to_string(),
            "[out]".to_string(),
            output_file.to_string_lossy().to_string(),
        ]);

        let output = Command::new("ffmpeg")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AtlasError::Render(format!("Failed to start FFmpeg: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AtlasError::Render(format!(
                "FFmpeg concat failed: {}",
                stderr
            )));
        }

        Ok(output_file)
    }

    /// Probe audio file for metadata using ffprobe.
    async fn probe_audio(&self, file: &Path) -> AtlasResult<AudioMetadata> {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=sample_rate,channels",
                "-of",
                "json",
                file.to_string_lossy().as_ref(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AtlasError::Render(format!("Failed to start ffprobe: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| AtlasError::Render(format!("Invalid ffprobe JSON: {}", e)))?;

        let duration = json["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        let sample_rate = json["streams"][0]["sample_rate"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(22050);
        let channels = json["streams"][0]["channels"].as_u64().unwrap_or(1) as u16;

        Ok(AudioMetadata {
            duration_seconds: duration,
            sample_rate,
            channels,
        })
    }

    async fn generate_word_timestamps(
        &self,
        segment: &NarrationSegment,
        total_duration: f32,
    ) -> AtlasResult<Vec<WordTimestamp>> {
        let words: Vec<&str> = segment.text.split_whitespace().collect();
        let word_count = words.len().max(1);
        let total_weight: f32 = words
            .iter()
            .map(|word| (word.len() as f32 / 5.0).clamp(0.5, 2.0))
            .sum();

        let mut timestamps = Vec::new();
        let mut current_time = 0.0f32;

        for word in words {
            let word_duration = total_duration * (word.len() as f32 / 5.0).clamp(0.5, 2.0)
                / total_weight.max(word_count as f32 * 0.5);
            timestamps.push(WordTimestamp {
                word: word.to_string(),
                start_seconds: current_time,
                end_seconds: current_time + word_duration,
            });
            current_time += word_duration;
        }

        Ok(timestamps)
    }

    /// Request a retake for a specific segment with modified parameters.
    pub async fn retake_segment(
        &self,
        original: &NarrationSegment,
        modifications: RetakeModifications,
    ) -> AtlasResult<AudioSegment> {
        let mut modified = original.clone();
        modified.segment_id = format!("{}_retake_{}", original.segment_id, Uuid::new_v4().simple());

        if let Some(speed) = modifications.speed {
            modified.speed = speed;
        }
        if let Some(pitch) = modifications.pitch {
            modified.pitch = pitch;
        }
        if let Some(emphasis) = modifications.emphasis_words {
            modified.emphasis_words = emphasis;
        }
        if let Some(pause) = modifications.pause_after_ms {
            modified.pause_after_ms = pause;
        }
        if let Some(hints) = modifications.pronunciation_hints {
            modified.pronunciation_hints = hints;
        }

        self.synthesize_segment(&modified).await
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AudioMetadata {
    pub duration_seconds: f32,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RetakeModifications {
    pub speed: Option<f32>,
    pub pitch: Option<f32>,
    pub emphasis_words: Option<Vec<String>>,
    pub pause_after_ms: Option<u32>,
    pub pronunciation_hints: Option<Vec<PronunciationHint>>,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            voice_id: "af_heart".to_string(),
            default_speed: 1.0,
            default_pitch: 1.0,
            language: "en-us".to_string(),
        }
    }
}

use uuid::Uuid;

/// Helper to split script text into narration segments.
pub fn split_script_into_segments(script: &str, voice_id: &str) -> Vec<NarrationSegment> {
    let sentences: Vec<&str> = script
        .split(['.', '?', '!'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    sentences
        .into_iter()
        .enumerate()
        .map(|(i, text)| NarrationSegment {
            segment_id: format!("seg_{:03}", i + 1),
            text: format!("{}.", text),
            voice: voice_id.to_string(),
            speed: 1.0,
            pitch: 1.0,
            emphasis_words: Vec::new(),
            pause_after_ms: 300,
            pronunciation_hints: Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segment_cache_key_tracks_synthesis_inputs() {
        let pipeline = KokoroPipeline::new("kokoro-tts", VoiceConfig::default(), "/tmp/atlas");
        let segment = split_script_into_segments("Hello world.", "af_heart").remove(0);
        let mut renumbered = segment.clone();
        renumbered.segment_id = "seg_999".into();
        let mut faster = segment.clone();
        faster.speed = 1.2;
        let mut other_voice = segment.clone();
        other_voice.voice = "am_adam".into();

        let key = pipeline.segment_cache_key(&segment);
        assert_eq!(key, pipeline.segment_cache_key(&renumbered));
        assert_ne!(key, pipeline.segment_cache_key(&faster));
        assert_ne!(key, pipeline.segment_cache_key(&other_voice));
    }

    #[test]
    fn splits_script_into_segments() {
        let script = "Hello world. This is a test. How are you?";
        let segments = split_script_into_segments(script, "af_heart");

        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "Hello world.");
        assert_eq!(segments[1].text, "This is a test.");
        // Note: split function normalizes punctuation to period
        assert_eq!(segments[2].text, "How are you.");
        assert!(segments[0].segment_id.contains("seg_001"));
    }

    #[test]
    fn narration_segment_serialization() {
        let segment = NarrationSegment {
            segment_id: "seg_001".into(),
            text: "Hello world.".into(),
            voice: "af_heart".into(),
            speed: 1.0,
            pitch: 1.0,
            emphasis_words: vec!["world".into()],
            pause_after_ms: 300,
            pronunciation_hints: vec![],
        };

        let json = serde_json::to_string(&segment).unwrap();
        let parsed: NarrationSegment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, "Hello world.");
        assert_eq!(parsed.emphasis_words, vec!["world"]);
    }
}
