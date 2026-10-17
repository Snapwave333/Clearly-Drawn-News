pub mod broadcast;

use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};

/// Edit Decision List (EDL) entry for frame-accurate timeline assembly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdlEntry {
    pub entry_id: String,
    pub source_type: SourceType,
    pub source_path: PathBuf,
    pub source_in_frame: u32,
    pub source_out_frame: u32,
    pub record_in_frame: u32,
    pub record_out_frame: u32,
    pub transition: Option<Transition>,
    pub audio_sync_offset_ms: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Video,
    Audio,
    StillImage,
    Caption,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transition {
    pub transition_type: TransitionType,
    pub duration_frames: u32,
    pub parameters: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionType {
    Cut,
    Crossfade,
    Morph,
    FadeToColor,
    Slide,
}

/// Complete EDL for a video project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditDecisionList {
    pub edl_id: String,
    pub project_id: String,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub entries: Vec<EdlEntry>,
    pub audio_tracks: Vec<AudioTrack>,
    pub caption_tracks: Vec<CaptionTrack>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioTrack {
    pub track_id: String,
    pub source_path: PathBuf,
    pub start_frame: u32,
    pub volume_db: f32,
    pub fade_in_frames: u32,
    pub fade_out_frames: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptionTrack {
    pub track_id: String,
    pub format: CaptionFormat,
    pub source_path: PathBuf,
    pub language: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionFormat {
    #[default]
    Srt,
    Vtt,
    Ass,
}

/// FFmpeg assembly pipeline - converts EDL to final video.
pub struct FfmpegAssembler {
    ffmpeg_exe: PathBuf,
    ffprobe_exe: PathBuf,
}

impl FfmpegAssembler {
    pub fn new(
        ffmpeg_exe: impl Into<PathBuf>,
        ffprobe_exe: impl Into<PathBuf>,
        workspace_dir: impl Into<PathBuf>,
    ) -> Self {
        let _ = workspace_dir.into();
        Self {
            ffmpeg_exe: ffmpeg_exe.into(),
            ffprobe_exe: ffprobe_exe.into(),
        }
    }

    /// Assemble final video from EDL.
    pub async fn assemble(
        &self,
        edl: &EditDecisionList,
        output_path: &Path,
    ) -> AtlasResult<AssemblyOutput> {
        info!("Assembling video from EDL: {} entries", edl.entries.len());

        // Build filter complex
        let filter_complex = self.build_filter_complex(edl)?;

        let mut input_args = Vec::new();
        for entry in &edl.entries {
            if matches!(entry.source_type, SourceType::Video) {
                input_args.push("-framerate".to_string());
                input_args.push(edl.fps.to_string());
                input_args.push("-start_number".to_string());
                input_args.push("1".to_string());
            } else if matches!(entry.source_type, SourceType::StillImage) {
                input_args.push("-loop".to_string());
                input_args.push("1".to_string());
            }
            input_args.push("-i".to_string());
            input_args.push(entry.source_path.to_string_lossy().to_string());
        }

        // Add audio tracks
        for track in &edl.audio_tracks {
            input_args.push("-i".to_string());
            input_args.push(track.source_path.to_string_lossy().to_string());
        }

        // Build output arguments
        let mut output_args = vec![
            "-y".to_string(),
            "-filter_complex".to_string(),
            filter_complex,
            "-map".to_string(),
            "[vout]".to_string(),
            "-map".to_string(),
            "[aout]".to_string(),
            "-c:v".to_string(),
            "libx264".to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-preset".to_string(),
            "medium".to_string(),
            "-crf".to_string(),
            "18".to_string(),
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            "192k".to_string(),
            "-movflags".to_string(),
            "+faststart".to_string(),
            "-r".to_string(),
            edl.fps.to_string(),
            "-s".to_string(),
            format!("{}x{}", edl.width, edl.height),
        ];

        // Add color space metadata
        output_args.extend([
            "-color_primaries".to_string(),
            "bt709".to_string(),
            "-color_trc".to_string(),
            "bt709".to_string(),
            "-colorspace".to_string(),
            "bt709".to_string(),
        ]);

        output_args.push("-shortest".to_string());
        output_args.push(output_path.to_string_lossy().to_string());

        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut all_args = input_args;
        all_args.extend(output_args);

        info!(
            "Running FFmpeg with {} inputs",
            edl.entries.len() + edl.audio_tracks.len()
        );

        let output = Command::new(&self.ffmpeg_exe)
            .args(&all_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AtlasError::Render(format!("Failed to start FFmpeg: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("FFmpeg assembly failed: {}", stderr);
            return Err(AtlasError::Render(format!(
                "FFmpeg exited with {}: {}",
                output.status, stderr
            )));
        }

        // Verify output
        let metadata = self.probe_output(output_path).await?;

        // Generate caption files
        let caption_files = self
            .generate_captions(edl, output_path.parent().unwrap())
            .await?;

        Ok(AssemblyOutput {
            output_file: output_path.to_path_buf(),
            duration_seconds: metadata.duration_seconds,
            frame_count: metadata.frame_count,
            bitrate_kbps: metadata.bitrate_kbps,
            width: metadata.width,
            height: metadata.height,
            caption_files,
        })
    }

    fn build_filter_complex(&self, edl: &EditDecisionList) -> AtlasResult<String> {
        if edl.fps == 0 || edl.width == 0 || edl.height == 0 {
            return Err(AtlasError::Render(
                "EDL frame rate and dimensions must be greater than zero".into(),
            ));
        }

        // Strict 16:9 widescreen validation (reject portrait / vertical aspect ratios)
        if edl.height > edl.width {
            return Err(AtlasError::Render(format!(
                "Portrait aspect ratio detected ({}x{}). Clearly Drawn News strictly requires native 16:9 widescreen.",
                edl.width, edl.height
            )));
        }
        let aspect = edl.width as f64 / edl.height as f64;
        let diff = (aspect - (16.0 / 9.0)).abs();
        if diff > 0.05 {
            return Err(AtlasError::Render(format!(
                "Non-16:9 aspect ratio detected (ratio: {:.3}). Must be 16:9 (e.g., 3840x2160 or 1920x1080).",
                aspect
            )));
        }

        let mut filters = Vec::new();
        let mut video_inputs = Vec::new();
        let mut audio_inputs = Vec::new();

        for (i, entry) in edl.entries.iter().enumerate() {
            match entry.source_type {
                SourceType::Video => {
                    if entry.source_out_frame <= entry.source_in_frame {
                        return Err(AtlasError::Render(format!(
                            "EDL entry {} has an invalid source range",
                            entry.entry_id
                        )));
                    }
                    let duration =
                        (entry.source_out_frame - entry.source_in_frame) as f32 / edl.fps as f32;
                    filters.push(format!(
                        "[{i}:v]fps={fps},scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:({width}-iw)/2:({height}-ih)/2,setsar=1,trim=duration={duration:.3},setpts=PTS-STARTPTS[v{i}]",
                        fps = edl.fps,
                        width = edl.width,
                        height = edl.height,
                    ));
                    video_inputs.push(format!("[v{i}]"));
                }
                SourceType::StillImage => {
                    let duration =
                        (entry.record_out_frame - entry.record_in_frame) as f32 / edl.fps as f32;
                    filters.push(format!(
                        "[{i}:v]scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:({width}-iw)/2:({height}-ih)/2,setsar=1,trim=duration={duration:.3},setpts=PTS-STARTPTS[v{i}]",
                        width = edl.width,
                        height = edl.height,
                    ));
                    video_inputs.push(format!("[v{i}]"));
                }
                _ => {}
            }
        }

        if video_inputs.is_empty() {
            return Err(AtlasError::Render("EDL contains no video entries".into()));
        }
        if video_inputs.len() == 1 {
            filters.push(format!("{}null[vout]", video_inputs[0]));
        } else {
            let concat_inputs = video_inputs.join("");
            filters.push(format!(
                "{}concat=n={}:v=1:a=0[vout]",
                concat_inputs,
                video_inputs.len()
            ));
        }

        let timeline_duration = edl
            .entries
            .iter()
            .map(|entry| entry.record_out_frame)
            .max()
            .unwrap_or(0) as f32
            / edl.fps as f32;
        let audio_offset = edl.entries.len();
        for (i, track) in edl.audio_tracks.iter().enumerate() {
            let input_idx = audio_offset + i;
            let start_time = track.start_frame as f32 / edl.fps as f32;
            let delay_ms = (start_time * 1000.0).round() as u64;
            let mut chain = format!(
                "[{input_idx}:a]adelay={delay_ms}:all=1,volume={}dB",
                track.volume_db
            );
            if track.fade_in_frames > 0 {
                let fade_dur = track.fade_in_frames as f32 / edl.fps as f32;
                chain.push_str(&format!(",afade=t=in:st={start_time:.3}:d={fade_dur:.3}"));
            }
            if track.fade_out_frames > 0 {
                let fade_dur = track.fade_out_frames as f32 / edl.fps as f32;
                let fade_start = (timeline_duration - fade_dur).max(start_time);
                chain.push_str(&format!(",afade=t=out:st={fade_start:.3}:d={fade_dur:.3}"));
            }
            chain.push_str(&format!(
                ",apad,atrim=duration={timeline_duration:.3}[a{i}]"
            ));
            filters.push(chain);
            audio_inputs.push(format!("[a{i}]"));
        }

        if audio_inputs.is_empty() {
            filters.push(format!(
                "anullsrc=r=48000:cl=stereo,atrim=duration={timeline_duration:.3}[aout]"
            ));
        } else if audio_inputs.len() == 1 {
            filters.push(format!(
                "{}loudnorm=I=-16:TP=-1.5:LRA=11[aout]",
                audio_inputs[0]
            ));
        } else {
            let amix_inputs = audio_inputs.join("");
            filters.push(format!(
                "{}amix=inputs={}:duration=longest:dropout_transition=0,atrim=duration={timeline_duration:.3},loudnorm=I=-16:TP=-1.5:LRA=11[aout]",
                amix_inputs,
                audio_inputs.len()
            ));
        }

        Ok(filters.join(";"))
    }

    /// Probe output file for verification.
    async fn probe_output(&self, file: &Path) -> AtlasResult<OutputMetadata> {
        let output = Command::new(&self.ffprobe_exe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration,bit_rate:stream=width,height,r_frame_rate,nb_frames",
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
        let bitrate = json["format"]["bit_rate"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let frame_count = json["streams"][0]["nb_frames"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let width = json["streams"][0]["width"].as_u64().unwrap_or(0) as u32;
        let height = json["streams"][0]["height"].as_u64().unwrap_or(0) as u32;

        Ok(OutputMetadata {
            duration_seconds: duration,
            frame_count,
            bitrate_kbps: bitrate / 1000,
            width,
            height,
        })
    }

    /// Generate caption files (SRT/VTT) from caption tracks.
    async fn generate_captions(
        &self,
        edl: &EditDecisionList,
        output_dir: &Path,
    ) -> AtlasResult<Vec<PathBuf>> {
        let mut caption_files = Vec::new();

        for track in &edl.caption_tracks {
            let output_file = output_dir.join(format!(
                "{}.{}",
                track.track_id,
                track.format.to_string().to_lowercase()
            ));

            // For now, copy the source caption file
            tokio::fs::copy(&track.source_path, &output_file)
                .await
                .map_err(|e| AtlasError::Render(format!("Failed to copy caption file: {}", e)))?;

            caption_files.push(output_file);
        }

        Ok(caption_files)
    }

    /// Create a simple EDL for testing.
    pub fn create_test_edl(project_id: &str) -> EditDecisionList {
        EditDecisionList {
            edl_id: format!("edl_{}", Uuid::new_v4().simple()),
            project_id: project_id.to_string(),
            fps: 30,
            width: 1920,
            height: 1080,
            entries: vec![
                EdlEntry {
                    entry_id: "shot_001".into(),
                    source_type: SourceType::Video,
                    source_path: PathBuf::from("renders/test_proj/shot_001/%04d.png"),
                    source_in_frame: 0,
                    source_out_frame: 90,
                    record_in_frame: 0,
                    record_out_frame: 90,
                    transition: None,
                    audio_sync_offset_ms: 0,
                },
                EdlEntry {
                    entry_id: "shot_002".into(),
                    source_type: SourceType::Video,
                    source_path: PathBuf::from("renders/test_proj/shot_002/%04d.png"),
                    source_in_frame: 0,
                    source_out_frame: 60,
                    record_in_frame: 90,
                    record_out_frame: 150,
                    transition: Some(Transition {
                        transition_type: TransitionType::Crossfade,
                        duration_frames: 15,
                        parameters: serde_json::json!({}),
                    }),
                    audio_sync_offset_ms: 0,
                },
            ],
            audio_tracks: vec![AudioTrack {
                track_id: "narration".into(),
                source_path: PathBuf::from("audio/combined/track_001.wav"),
                start_frame: 0,
                volume_db: 0.0,
                fade_in_frames: 15,
                fade_out_frames: 15,
            }],
            caption_tracks: vec![CaptionTrack {
                track_id: "captions_en".into(),
                format: CaptionFormat::Srt,
                source_path: PathBuf::from("captions/track_001.srt"),
                language: "en".into(),
            }],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssemblyOutput {
    pub output_file: PathBuf,
    pub duration_seconds: f32,
    pub frame_count: u32,
    pub bitrate_kbps: u32,
    pub width: u32,
    pub height: u32,
    pub caption_files: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputMetadata {
    pub duration_seconds: f32,
    pub frame_count: u32,
    pub bitrate_kbps: u32,
    pub width: u32,
    pub height: u32,
}

use uuid::Uuid;

impl std::fmt::Display for CaptionFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptionFormat::Srt => write!(f, "srt"),
            CaptionFormat::Vtt => write!(f, "vtt"),
            CaptionFormat::Ass => write!(f, "ass"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn builds_filter_complex() {
        let dir = tempdir().unwrap();
        let assembler = FfmpegAssembler::new("ffmpeg", "ffprobe", dir.path());
        let edl = FfmpegAssembler::create_test_edl("test_proj");
        let filter = assembler.build_filter_complex(&edl).unwrap();

        eprintln!("FILTER: {}", filter);

        assert!(filter.contains("trim"));
        assert!(filter.contains("setpts"));
        assert!(filter.contains("scale"));
        assert!(filter.contains("concat"));
        // amix may not appear if only one audio input
        assert!(filter.contains("loudnorm") || filter.contains("amix"));
    }

    #[test]
    fn edl_serialization() {
        let edl = FfmpegAssembler::create_test_edl("test_proj");
        let json = serde_json::to_string(&edl).unwrap();
        let parsed: EditDecisionList = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.audio_tracks.len(), 1);
    }
}
