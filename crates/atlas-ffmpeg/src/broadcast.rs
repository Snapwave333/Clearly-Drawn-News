use atlas_core::{AtlasError, AtlasResult};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueSpeaker {
    Marcus,
    Elena,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    X264,
    X265,
    NvencH264,
    NvencHevc,
}

impl VideoEncoder {
    pub fn codec_name(self) -> &'static str {
        match self {
            Self::X264 | Self::NvencH264 => "h264",
            Self::X265 | Self::NvencHevc => "hevc",
        }
    }

    pub fn args(self, fps: u32, height: u32) -> Vec<String> {
        let gop = (fps * 2).to_string();
        let uhd = height >= 2160;
        let mut args: Vec<String> = match self {
            Self::X264 => vec![
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                std::env::var("ATLAS_NEWS_X264_PRESET").unwrap_or_else(|_| "slow".into()),
                "-tune".into(),
                "animation".into(),
                "-crf".into(),
                std::env::var("ATLAS_NEWS_CRF").unwrap_or_else(|_| "16".into()),
                "-profile:v".into(),
                "high".into(),
                "-level:v".into(),
                if uhd { "5.2".into() } else { "4.2".into() },
                "-x264-params".into(),
                "aq-mode=3:aq-strength=0.9:psy-rd=1.0,0.15:bframes=2:ref=4".into(),
            ],
            Self::X265 => vec![
                "-c:v".into(),
                "libx265".into(),
                "-preset".into(),
                std::env::var("ATLAS_NEWS_X265_PRESET").unwrap_or_else(|_| "medium".into()),
                "-crf".into(),
                std::env::var("ATLAS_NEWS_CRF").unwrap_or_else(|_| "17".into()),
                "-tag:v".into(),
                "hvc1".into(),
                "-x265-params".into(),
                "aq-mode=3:bframes=2:log-level=error".into(),
            ],
            Self::NvencH264 | Self::NvencHevc => vec![
                "-c:v".into(),
                if self == Self::NvencH264 { "h264_nvenc".into() } else { "hevc_nvenc".into() },
                "-preset".into(),
                "p7".into(),
                "-tune".into(),
                "hq".into(),
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                std::env::var("ATLAS_NEWS_CQ").unwrap_or_else(|_| "17".into()),
                "-b:v".into(),
                "0".into(),
                "-maxrate".into(),
                if uhd { "90M".into() } else { "30M".into() },
                "-bufsize".into(),
                if uhd { "180M".into() } else { "60M".into() },
                "-spatial-aq".into(),
                "1".into(),
                "-temporal-aq".into(),
                "1".into(),
                "-bf".into(),
                "2".into(),
            ],
        };
        if self == Self::NvencHevc {
            args.extend(["-tag:v".into(), "hvc1".into()]);
        }
        args.extend([
            "-g".into(),
            gop,
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-colorspace".into(),
            "bt709".into(),
            "-color_primaries".into(),
            "bt709".into(),
            "-color_trc".into(),
            "bt709".into(),
            "-color_range".into(),
            "tv".into(),
        ]);
        args
    }
}

#[derive(Clone, Debug)]
pub enum AudioPiece {
    Voice(Vec<PathBuf>),
    Silence(f64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BroadcastSegment {
    pub title: String,
    pub category: String,
    pub headline: String,
    pub attribution: String,
    pub start: f64,
    pub end: f64,
    pub card_seconds: f64,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpeakerWindow {
    pub speaker: CueSpeaker,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptionCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct BroadcastSpec {
    pub show_title: String,
    pub episode_label: String,
    pub segments: Vec<BroadcastSegment>,
    pub speakers: Vec<SpeakerWindow>,
    pub ticker: String,
    pub audio: PathBuf,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub font_regular: PathBuf,
    pub font_bold: PathBuf,
    pub background_plate: Option<PathBuf>,
    pub encoder: VideoEncoder,
    pub work_dir: PathBuf,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShortCue {
    pub start: f64,
    pub end: f64,
    pub speaker: CueSpeaker,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ShortSpec {
    pub show_title: String,
    pub title: String,
    pub category: String,
    pub attribution: String,
    pub audio: PathBuf,
    pub clip_start: f64,
    pub duration: f64,
    pub cues: Vec<ShortCue>,
    pub font_regular: PathBuf,
    pub font_bold: PathBuf,
    pub encoder: VideoEncoder,
    pub work_dir: PathBuf,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LoudnessReport {
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    pub loudness_range_lu: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MediaProbe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: String,
    pub audio_codec: String,
    pub audio_channels: u32,
    pub size_bytes: u64,
}

pub struct BroadcastRenderer {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

const MARCUS_ACCENT: &str = "0xff9f1c";
const ELENA_ACCENT: &str = "0x2ec4b6";
const BRAND_ACCENT: &str = "0xff4f5e";

impl BroadcastRenderer {
    pub fn new(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }

    pub async fn has_filter(&self, name: &str) -> bool {
        let Ok(output) = Command::new(&self.ffmpeg)
            .args(["-hide_banner", "-filters"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
        else {
            return false;
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
    }

    pub async fn encoder_works(&self, name: &str) -> bool {
        Command::new(&self.ffmpeg)
            .args([
                "-hide_banner", "-nostdin", "-v", "error", "-f", "lavfi", "-i", "color=c=black:s=1280x720:r=30:d=0.2",
                "-frames:v", "3", "-c:v", name, "-f", "null", "-",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn run_ffmpeg(&self, args: &[String], cwd: Option<&Path>) -> AtlasResult<String> {
        let mut command = Command::new(&self.ffmpeg);
        command
            .arg("-hide_banner")
            .arg("-nostdin")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        let output = command
            .output()
            .await
            .map_err(|e| AtlasError::Render(format!("failed to start {}: {e}", self.ffmpeg.display())))?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            let tail: Vec<&str> = stderr.lines().rev().take(25).collect();
            return Err(AtlasError::Render(format!(
                "ffmpeg exited with {}: {}",
                output.status,
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            )));
        }
        Ok(stderr)
    }

    pub async fn probe(&self, path: &Path) -> AtlasResult<MediaProbe> {
        let output = Command::new(&self.ffprobe)
            .args(["-v", "error", "-show_format", "-show_streams", "-of", "json"])
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| AtlasError::Render(format!("failed to start ffprobe: {e}")))?;
        if !output.status.success() {
            return Err(AtlasError::Render(format!(
                "ffprobe failed for {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let mut probe = MediaProbe {
            duration: json["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0),
            size_bytes: json["format"]["size"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0),
            ..Default::default()
        };
        for stream in json["streams"].as_array().cloned().unwrap_or_default() {
            match stream["codec_type"].as_str() {
                Some("video") if probe.video_codec.is_empty() => {
                    probe.video_codec = stream["codec_name"].as_str().unwrap_or_default().to_string();
                    probe.width = stream["width"].as_u64().unwrap_or(0) as u32;
                    probe.height = stream["height"].as_u64().unwrap_or(0) as u32;
                    probe.fps = parse_rate(stream["avg_frame_rate"].as_str().unwrap_or("0/1"));
                }
                Some("audio") if probe.audio_codec.is_empty() => {
                    probe.audio_codec = stream["codec_name"].as_str().unwrap_or_default().to_string();
                    probe.audio_channels = stream["channels"].as_u64().unwrap_or(0) as u32;
                }
                _ => {}
            }
        }
        Ok(probe)
    }

    pub async fn concat_dialogue(&self, pieces: &[AudioPiece], work_dir: &Path, output: &Path) -> AtlasResult<Vec<f64>> {
        let norm_dir = work_dir.join("norm");
        tokio::fs::create_dir_all(&norm_dir).await?;
        let mut list = String::new();
        let mut durations = Vec::with_capacity(pieces.len());
        for (index, piece) in pieces.iter().enumerate() {
            let target = norm_dir.join(format!("{index:05}.wav"));
            let mut args: Vec<String> = vec!["-y".into()];
            match piece {
                AudioPiece::Silence(seconds) => {
                    args.extend([
                        "-f".into(),
                        "lavfi".into(),
                        "-i".into(),
                        "anullsrc=r=48000:cl=mono".into(),
                        "-t".into(),
                        format!("{seconds:.3}"),
                    ]);
                }
                AudioPiece::Voice(paths) if paths.len() > 1 => {
                    for p in paths {
                        args.extend(["-i".into(), p.to_string_lossy().to_string()]);
                    }
                    let inputs: String = (0..paths.len()).map(|i| format!("[{i}:a]")).collect();
                    args.extend([
                        "-filter_complex".into(),
                        format!("{inputs}amix=inputs={}:duration=longest:normalize=0,volume=0.7[a]", paths.len()),
                        "-map".into(),
                        "[a]".into(),
                    ]);
                }
                AudioPiece::Voice(paths) => {
                    let first = paths
                        .first()
                        .ok_or_else(|| AtlasError::Render("voice piece without audio".into()))?;
                    args.extend(["-i".into(), first.to_string_lossy().to_string()]);
                }
            }
            args.extend([
                "-ar".into(),
                "48000".into(),
                "-ac".into(),
                "1".into(),
                "-c:a".into(),
                "pcm_s16le".into(),
                target.to_string_lossy().to_string(),
            ]);
            self.run_ffmpeg(&args, None).await?;
            durations.push(self.probe(&target).await?.duration);
            writeln!(list, "file 'norm/{index:05}.wav'").ok();
        }
        let list_path = work_dir.join("dialogue_concat.txt");
        tokio::fs::write(&list_path, list).await?;
        self.run_ffmpeg(
            &[
                "-y".into(),
                "-f".into(),
                "concat".into(),
                "-safe".into(),
                "0".into(),
                "-i".into(),
                "dialogue_concat.txt".into(),
                "-c".into(),
                "copy".into(),
                output.to_string_lossy().to_string(),
            ],
            Some(work_dir),
        )
        .await?;
        Ok(durations)
    }

    pub async fn master_audio(
        &self,
        dialogue: &Path,
        music: Option<&Path>,
        output: &Path,
        target_lufs: f64,
        true_peak: f64,
    ) -> AtlasResult<LoudnessReport> {
        let duration = self.probe(dialogue).await?.duration;
        let mut inputs: Vec<String> = vec!["-i".into(), dialogue.to_string_lossy().to_string()];
        let premix = match music {
            Some(path) => {
                inputs.extend([
                    "-stream_loop".into(),
                    "-1".into(),
                    "-i".into(),
                    path.to_string_lossy().to_string(),
                ]);
                format!(
                    "[1:a]aformat=sample_rates=48000:channel_layouts=mono,volume=0.12,atrim=0:{duration:.3},afade=t=out:st={:.3}:d=3[m];[0:a][m]amix=inputs=2:duration=first:normalize=0",
                    (duration - 3.0).max(0.0)
                )
            }
            None => "[0:a]anull".to_string(),
        };
        let measure_filter = format!("{premix},loudnorm=I={target_lufs}:TP={true_peak}:LRA=11:print_format=json[out]");
        let mut args: Vec<String> = vec!["-y".into()];
        args.extend(inputs.clone());
        args.extend(["-filter_complex".into(), measure_filter, "-map".into(), "[out]".into(), "-f".into(), "null".into(), "-".into()]);
        let stderr = self.run_ffmpeg(&args, None).await?;
        let measured = parse_loudnorm_json(&stderr)
            .ok_or_else(|| AtlasError::Render("loudnorm measurement produced no JSON".into()))?;
        let apply_filter = format!(
            "{premix},loudnorm=I={target_lufs}:TP={true_peak}:LRA=11:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true,aresample=48000[out]",
            measured["input_i"].as_str().unwrap_or("-24"),
            measured["input_tp"].as_str().unwrap_or("-2"),
            measured["input_lra"].as_str().unwrap_or("7"),
            measured["input_thresh"].as_str().unwrap_or("-34"),
            measured["target_offset"].as_str().unwrap_or("0"),
        );
        let mut args: Vec<String> = vec!["-y".into()];
        args.extend(inputs);
        args.extend([
            "-filter_complex".into(),
            apply_filter,
            "-map".into(),
            "[out]".into(),
            "-ac".into(),
            "2".into(),
            "-c:a".into(),
            "pcm_s16le".into(),
            output.to_string_lossy().to_string(),
        ]);
        self.run_ffmpeg(&args, None).await?;
        self.measure_loudness(output).await
    }

    pub async fn measure_loudness(&self, path: &Path) -> AtlasResult<LoudnessReport> {
        let stderr = self
            .run_ffmpeg(
                &[
                    "-i".into(),
                    path.to_string_lossy().to_string(),
                    "-map".into(),
                    "0:a:0".into(),
                    "-af".into(),
                    "ebur128=peak=true".into(),
                    "-f".into(),
                    "null".into(),
                    "-".into(),
                ],
                None,
            )
            .await?;
        parse_ebur128_summary(&stderr)
            .ok_or_else(|| AtlasError::Render("ebur128 produced no loudness summary".into()))
    }

    pub async fn render(&self, spec: &BroadcastSpec) -> AtlasResult<()> {
        let txt_dir = spec.work_dir.join("txt");
        tokio::fs::create_dir_all(&txt_dir).await?;
        let regular = copy_font(&spec.font_regular, &spec.work_dir, "font_regular").await?;
        let bold = copy_font(&spec.font_bold, &spec.work_dir, "font_bold").await?;
        let animated_background = spec.background_plate.is_none() && self.has_filter("gradients").await;
        let graph = build_filtergraph(spec, &regular, &bold, &txt_dir).await?;
        tokio::fs::write(spec.work_dir.join("filtergraph.txt"), graph).await?;

        let mut args: Vec<String> = vec!["-y".into()];
        match &spec.background_plate {
            Some(plate) if is_image(plate) => args.extend([
                "-loop".into(),
                "1".into(),
                "-framerate".into(),
                spec.fps.to_string(),
                "-i".into(),
                plate.to_string_lossy().to_string(),
            ]),
            Some(plate) => args.extend([
                "-stream_loop".into(),
                "-1".into(),
                "-i".into(),
                plate.to_string_lossy().to_string(),
            ]),
            None => args.extend([
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                background_source(animated_background, spec.fps),
            ]),
        }
        args.extend([
            "-i".into(),
            spec.audio.to_string_lossy().to_string(),
            "-filter_complex_script".into(),
            "filtergraph.txt".into(),
            "-map".into(),
            "[vout]".into(),
            "-map".into(),
            "1:a".into(),
            "-t".into(),
            format!("{:.3}", spec.duration),
            "-r".into(),
            spec.fps.to_string(),
        ]);
        args.extend(spec.encoder.args(spec.fps, spec.height));
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "384k".into(),
            "-ar".into(),
            "48000".into(),
            "-ac".into(),
            "2".into(),
            "-movflags".into(),
            "+faststart".into(),
            spec.output.to_string_lossy().to_string(),
        ]);
        self.run_ffmpeg(&args, Some(&spec.work_dir)).await?;
        Ok(())
    }

    pub async fn render_thumbnail(
        &self,
        show_title: &str,
        episode_label: &str,
        headline: &str,
        fonts: (&Path, &Path),
        work_dir: &Path,
        output: &Path,
    ) -> AtlasResult<()> {
        tokio::fs::create_dir_all(work_dir.join("txt")).await?;
        let regular = copy_font(fonts.0, work_dir, "font_regular").await?;
        let bold = copy_font(fonts.1, work_dir, "font_bold").await?;
        tokio::fs::write(work_dir.join("txt/thumb_show.txt"), show_title.to_uppercase()).await?;
        tokio::fs::write(work_dir.join("txt/thumb_label.txt"), episode_label).await?;
        tokio::fs::write(work_dir.join("txt/thumb_headline.txt"), wrap_text(headline, 26, 4)).await?;
        let background = if self.has_filter("gradients").await {
            "gradients=s=960x540:r=1:c0=0x0a0f24:c1=0x1b1446:c2=0x08243a:nb_colors=3:x0=0:y0=0:x1=960:y1=540:d=1,vignette=PI/5,scale=1920:1080:flags=bicubic"
        } else {
            "color=c=0x0b1020:s=1920x1080:d=1"
        };
        let shadow = ":shadowcolor=black@0.65:shadowx=4:shadowy=4:expansion=none";
        let graph = format!(
            "{background},format=yuv444p,drawbox=x=0:y=0:w=iw:h=170:color=0x111a33@0.92:t=fill,drawbox=x=0:y=170:w=iw:h=12:color={BRAND_ACCENT}:t=fill,drawbox=x=0:y=980:w=iw:h=100:color={BRAND_ACCENT}:t=fill,drawbox=x=72:y=250:w=18:h=620:color={BRAND_ACCENT}:t=fill,{},{},{},format=yuv420p[out]",
            drawtext(&bold, "txt/thumb_show.txt", "72", "52", 78.0, "white", None).replace(":expansion=none", shadow),
            drawtext(&regular, "txt/thumb_label.txt", "72", "1003", 46.0, "white", None),
            drawtext(&bold, "txt/thumb_headline.txt", "130", "250", 108.0, "white", None).replace(":expansion=none", &format!(":line_spacing=22{shadow}")),
        );
        tokio::fs::write(work_dir.join("thumbnail_graph.txt"), graph).await?;
        self.run_ffmpeg(
            &[
                "-y".into(),
                "-filter_complex_script".into(),
                "thumbnail_graph.txt".into(),
                "-map".into(),
                "[out]".into(),
                "-frames:v".into(),
                "1".into(),
                output.to_string_lossy().to_string(),
            ],
            Some(work_dir),
        )
        .await?;
        Ok(())
    }
}

impl BroadcastRenderer {
    pub async fn render_short(&self, spec: &ShortSpec) -> AtlasResult<()> {
        let txt_dir = spec.work_dir.join("txt");
        tokio::fs::create_dir_all(&txt_dir).await?;
        let regular = copy_font(&spec.font_regular, &spec.work_dir, "font_regular").await?;
        let bold = copy_font(&spec.font_bold, &spec.work_dir, "font_bold").await?;
        let animated = self.has_filter("gradients").await;
        let graph = build_short_graph(spec, &regular, &bold, &txt_dir).await?;
        tokio::fs::write(spec.work_dir.join("short_graph.txt"), graph).await?;
        let background = if animated {
            "gradients=s=540x960:r=30:c0=0x0a0f24:c1=0x1b1446:c2=0x08243a:nb_colors=3:x0=0:y0=0:x1=540:y1=960:speed=0.006".to_string()
        } else {
            "color=c=0x0b1020:s=540x960:r=30".to_string()
        };
        let mut args: Vec<String> = vec![
            "-y".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            background,
            "-ss".into(),
            format!("{:.3}", spec.clip_start),
            "-t".into(),
            format!("{:.3}", spec.duration),
            "-i".into(),
            spec.audio.to_string_lossy().to_string(),
            "-filter_complex_script".into(),
            "short_graph.txt".into(),
            "-map".into(),
            "[vout]".into(),
            "-map".into(),
            "[aout]".into(),
            "-t".into(),
            format!("{:.3}", spec.duration),
            "-r".into(),
            "30".into(),
        ];
        args.extend(spec.encoder.args(30, 1920));
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "256k".into(),
            "-ar".into(),
            "48000".into(),
            "-ac".into(),
            "2".into(),
            "-movflags".into(),
            "+faststart".into(),
            spec.output.to_string_lossy().to_string(),
        ]);
        self.run_ffmpeg(&args, Some(&spec.work_dir)).await?;
        Ok(())
    }
}

pub fn caption_phrases(text: &str, max_words: usize, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        let len: usize = current.iter().map(|w| w.chars().count() + 1).sum::<usize>() + word.chars().count();
        if !current.is_empty() && (current.len() >= max_words || len > max_chars) {
            chunks.push(current.join(" "));
            current.clear();
        }
        current.push(word);
        if word.ends_with(['.', '!', '?']) || (word.ends_with([',', ';', ':']) && current.len() >= 2) {
            chunks.push(current.join(" "));
            current.clear();
        }
    }
    if !current.is_empty() {
        chunks.push(current.join(" "));
    }
    chunks
}

async fn build_short_graph(spec: &ShortSpec, regular: &str, bold: &str, txt_dir: &Path) -> AtlasResult<String> {
    let write = |name: String, body: String| async move {
        tokio::fs::write(txt_dir.join(&name), body).await.map(|_| format!("txt/{name}"))
    };
    let text = |font, file: String, x: &str, y: &str, size: f64, color| Text {
        font,
        file,
        x: x.to_string(),
        y: y.to_string(),
        size,
        color,
        enable: None,
        alpha: None,
        shadow: 3.0,
        line_spacing: 0.0,
    };
    let dur = spec.duration;
    let mut f: Vec<String> = vec![
        "vignette=angle=PI/5,scale=1080:1920:flags=bicubic:out_color_matrix=bt709:out_range=tv,setsar=1,format=yuv444p".into(),
        drawbox(0.0, 0.0, "iw", 150.0, "0x0b1124@0.92", "fill", None),
        drawbox(0.0, 150.0, "iw", 8.0, BRAND_ACCENT, "fill", None),
    ];
    let show = write("s_show.txt".into(), spec.show_title.to_uppercase()).await?;
    f.push(text(bold, show, "60", "52", 50.0, "white").build());
    let cat = write("s_cat.txt".into(), spec.category.to_uppercase()).await?;
    f.push(text(bold, cat, "60", "212", 34.0, BRAND_ACCENT).build());

    f.push(drawbox(60.0 + 12.0, 270.0 + 12.0, "960", 400.0, "black@0.35", "fill", None));
    f.push(drawbox(60.0, 270.0, "960", 400.0, "0x0d1430@0.92", "fill", None));
    f.push(drawbox(60.0, 270.0, "14", 400.0, BRAND_ACCENT, "fill", None));
    let title = write("s_title.txt".into(), wrap_text(&spec.title, 20, 4)).await?;
    let mut t = text(bold, title, "110", "310", 74.0, "white");
    t.line_spacing = 14.0;
    t.shadow = 4.0;
    t.alpha = Some(fade_in(0.0, 0.4));
    f.push(t.build());

    let windows: Vec<SpeakerWindow> = spec
        .cues
        .iter()
        .map(|c| SpeakerWindow { speaker: c.speaker, start: c.start, end: c.end })
        .collect();
    let marcus_on = enable_expr(&merge_windows(&windows, CueSpeaker::Marcus));
    let elena_on = enable_expr(&merge_windows(&windows, CueSpeaker::Elena));
    f.push(drawbox(60.0 + 12.0, 730.0 + 12.0, "960", 440.0, "black@0.35", "fill", None));
    f.push(drawbox(60.0, 730.0, "960", 440.0, "0x16203f@0.9", "fill", None));
    for (letter, name, accent, on) in [
        ("M", "MARCUS  ·  The Enthusiast", MARCUS_ACCENT, marcus_on.as_deref()),
        ("E", "ELENA  ·  The Sharp Counterpart", ELENA_ACCENT, elena_on.as_deref()),
    ] {
        let Some(expr) = on else { continue };
        f.push(drawbox(60.0, 730.0, "960", 12.0, accent, "fill", Some(expr)));
        let letter_file = write(format!("s_letter_{letter}.txt"), letter.into()).await?;
        let mut t = text(bold, letter_file, "(w-tw)/2", "770", 230.0, accent);
        t.enable = Some(expr.to_string());
        t.shadow = 6.0;
        f.push(t.build());
        let name_file = write(format!("s_name_{letter}.txt"), name.into()).await?;
        let mut t = text(bold, name_file, "(w-tw)/2", "1110", 38.0, "white");
        t.enable = Some(expr.to_string());
        f.push(t.build());
    }

    let mut index = 0;
    for cue in &spec.cues {
        let phrases = caption_phrases(&cue.text, 4, 22);
        let total: usize = phrases.iter().map(|p| p.chars().count()).sum::<usize>().max(1);
        let mut at = cue.start;
        for phrase in phrases {
            let end = (at + (cue.end - cue.start) * phrase.chars().count() as f64 / total as f64).min(cue.end);
            let file = write(format!("s_cap{index:03}.txt"), wrap_text(&phrase, 16, 2)).await?;
            let color = match cue.speaker {
                CueSpeaker::Marcus => "0xffe3b8",
                CueSpeaker::Elena => "0xc8fff8",
                CueSpeaker::Both => "white",
            };
            let mut t = text(bold, file, "(w-tw)/2", "1330-th/2", 92.0, color);
            t.enable = Some(format!("between(t,{at:.2},{end:.2})"));
            t.line_spacing = 10.0;
            t.shadow = 0.0;
            f.push(t.build().replace(":expansion=none", ":borderw=8:bordercolor=black@0.9:expansion=none"));
            at = end;
            index += 1;
        }
    }

    if !spec.attribution.is_empty() {
        let attr = write("s_attr.txt".into(), wrap_text(&spec.attribution, 44, 2)).await?;
        f.push(text(regular, attr, "60", "1610", 30.0, "0xaab4dd").build());
    }
    let cta_start = (dur - 4.0).max(0.0);
    let cta_window = format!("gte(t,{cta_start:.2})");
    f.push(drawbox(60.0, 1700.0, "960", 120.0, BRAND_ACCENT, "fill", Some(&cta_window)));
    let cta = write("s_cta.txt".into(), "FULL EPISODE ON THE CHANNEL".into()).await?;
    let mut t = text(bold, cta, "(w-tw)/2", "1740", 44.0, "white");
    t.enable = Some(cta_window);
    t.alpha = Some(fade_in(cta_start, 0.3));
    t.shadow = 0.0;
    f.push(t.build());

    let mut graph = format!("[0:v]{}[base]", f.join(",\n"));
    graph.push_str(&format!(
        ";\n[1:a]afade=t=in:d=0.12,afade=t=out:st={:.3}:d=0.35,asplit=3[aout][am][ae]",
        (dur - 0.35).max(0.0)
    ));
    let mut last = "base".to_string();
    for (tag, src, color, on) in [("wm", "am", MARCUS_ACCENT, marcus_on.as_deref()), ("we", "ae", ELENA_ACCENT, elena_on.as_deref())] {
        match on {
            Some(expr) => {
                graph.push_str(&format!(
                    ";\n[{src}]aformat=channel_layouts=mono,showwaves=s=840x110:mode=cline:rate=30:colors={color}:draw=full,format=rgba[{tag}];\n[{last}][{tag}]overlay=x=120:y=990:format=yuv444:enable='{expr}'[{last}_{tag}]"
                ));
                last = format!("{last}_{tag}");
            }
            None => graph.push_str(&format!(";\n[{src}]anullsink")),
        }
    }
    graph.push_str(&format!(";\n[{last}]format=yuv420p[vout]"));
    Ok(graph)
}

fn background_source(animated: bool, fps: u32) -> String {
    if animated {
        format!("gradients=s=960x540:r={fps}:c0=0x0a0f24:c1=0x1b1446:c2=0x08243a:nb_colors=3:x0=0:y0=0:x1=960:y1=540:speed=0.004")
    } else {
        format!("color=c=0x0b1020:s=960x540:r={fps}")
    }
}

fn is_image(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "webp")
    )
}

async fn copy_font(source: &Path, work_dir: &Path, stem: &str) -> AtlasResult<String> {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("ttf");
    let name = format!("{stem}.{ext}");
    tokio::fs::copy(source, work_dir.join(&name))
        .await
        .map_err(|e| AtlasError::Render(format!("font {} unavailable: {e}", source.display())))?;
    Ok(name)
}

fn parse_rate(rate: &str) -> f64 {
    match rate.split_once('/') {
        Some((n, d)) => {
            let n: f64 = n.parse().unwrap_or(0.0);
            let d: f64 = d.parse().unwrap_or(1.0);
            if d == 0.0 { 0.0 } else { n / d }
        }
        None => rate.parse().unwrap_or(0.0),
    }
}

fn parse_loudnorm_json(stderr: &str) -> Option<serde_json::Value> {
    let start = stderr.rfind("{\n")?;
    let end = stderr[start..].find('}')? + start + 1;
    serde_json::from_str(&stderr[start..end]).ok()
}

pub fn parse_ebur128_summary(stderr: &str) -> Option<LoudnessReport> {
    let summary = &stderr[stderr.rfind("Summary:")?..];
    let value_after = |label: &str| -> Option<f64> {
        let line = summary.lines().find(|l| l.trim_start().starts_with(label))?;
        line.split_once(':')?.1.split_whitespace().next()?.parse().ok()
    };
    Some(LoudnessReport {
        integrated_lufs: value_after("I:")?,
        loudness_range_lu: value_after("LRA:")?,
        true_peak_dbtp: value_after("Peak:")?,
    })
}

pub fn wrap_text(text: &str, width: usize, max_lines: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    lines.join("\n")
}

fn drawtext(font: &str, textfile: &str, x: &str, y: &str, size: f64, color: &str, enable: Option<&str>) -> String {
    let mut f = format!(
        "drawtext=fontfile={font}:textfile={textfile}:x={x}:y={y}:fontsize={size:.0}:fontcolor={color}:expansion=none"
    );
    if let Some(e) = enable {
        f.push_str(&format!(":enable='{e}'"));
    }
    f
}

fn drawbox(x: f64, y: f64, w: &str, h: f64, color: &str, thickness: &str, enable: Option<&str>) -> String {
    let mut f = format!("drawbox=x={x:.0}:y={y:.0}:w={w}:h={h:.0}:color={color}:t={thickness}");
    if let Some(e) = enable {
        f.push_str(&format!(":enable='{e}'"));
    }
    f
}

pub fn merge_windows(windows: &[SpeakerWindow], speaker: CueSpeaker) -> Vec<(f64, f64)> {
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for w in windows.iter().filter(|w| w.speaker == speaker || w.speaker == CueSpeaker::Both) {
        match merged.last_mut() {
            Some(last) if w.start - last.1 < 0.6 => last.1 = last.1.max(w.end),
            _ => merged.push((w.start, w.end)),
        }
    }
    merged
}

fn enable_expr(windows: &[(f64, f64)]) -> Option<String> {
    if windows.is_empty() {
        return None;
    }
    Some(
        windows
            .iter()
            .map(|(s, e)| format!("between(t,{s:.2},{e:.2})"))
            .collect::<Vec<_>>()
            .join("+"),
    )
}

struct Text<'a> {
    font: &'a str,
    file: String,
    x: String,
    y: String,
    size: f64,
    color: &'a str,
    enable: Option<String>,
    alpha: Option<String>,
    shadow: f64,
    line_spacing: f64,
}

impl Text<'_> {
    fn build(&self) -> String {
        let mut f = format!(
            "drawtext=fontfile={}:textfile={}:x='{}':y='{}':fontsize={:.0}:fontcolor={}:expansion=none",
            self.font, self.file, self.x, self.y, self.size, self.color
        );
        if self.line_spacing > 0.0 {
            f.push_str(&format!(":line_spacing={:.0}", self.line_spacing));
        }
        if self.shadow > 0.0 {
            f.push_str(&format!(":shadowcolor=black@0.6:shadowx={0:.0}:shadowy={0:.0}", self.shadow));
        }
        if let Some(a) = &self.alpha {
            f.push_str(&format!(":alpha='{a}'"));
        }
        if let Some(e) = &self.enable {
            f.push_str(&format!(":enable='{e}'"));
        }
        f
    }
}

fn fade_in(start: f64, seconds: f64) -> String {
    format!("min(1,max(0,(t-{start:.2})/{seconds:.2}))")
}

fn fade_in_out(start: f64, end: f64, seconds: f64) -> String {
    format!("min(1,max(0,min((t-{start:.2})/{seconds:.2},({end:.2}-t)/{seconds:.2})))")
}

async fn build_filtergraph(spec: &BroadcastSpec, regular: &str, bold: &str, txt_dir: &Path) -> AtlasResult<String> {
    let k = spec.width as f64 / 1920.0;
    let px = |v: f64| v * k;
    let n = |v: f64| format!("{:.0}", v * k);
    let w = spec.width as f64;
    let h = spec.height as f64;
    let mut filters: Vec<String> = Vec::new();
    let write = |name: String, body: String| async move {
        tokio::fs::write(txt_dir.join(&name), body).await.map(|_| format!("txt/{name}"))
    };
    let text = |font, file: String, x: String, y: String, size: f64, color| Text {
        font,
        file,
        x,
        y,
        size: size * k,
        color,
        enable: None,
        alpha: None,
        shadow: 2.0 * k,
        line_spacing: 0.0,
    };

    if spec.background_plate.is_some() {
        filters.push(format!(
            "scale={}:{}:force_original_aspect_ratio=increase:flags=lanczos:out_color_matrix=bt709:out_range=tv,crop={}:{},fps={},setsar=1,format=yuv444p",
            spec.width, spec.height, spec.width, spec.height, spec.fps
        ));
    } else {
        filters.push(format!(
            "vignette=angle=PI/5,scale={}:{}:flags=bicubic:out_color_matrix=bt709:out_range=tv,setsar=1,format=yuv444p",
            spec.width, spec.height
        ));
    }

    filters.push(drawbox(0.0, 0.0, "iw", px(96.0), "0x0b1124@0.92", "fill", None));
    filters.push(drawbox(0.0, px(96.0), "iw", px(6.0), BRAND_ACCENT, "fill", None));
    let show = write("show.txt".into(), spec.show_title.to_uppercase()).await?;
    filters.push(text(bold, show, n(48.0), n(24.0), 46.0, "white").build());
    let pill_w = px(150.0);
    let pill_x = w - px(48.0) - pill_w;
    filters.push(drawbox(pill_x, px(22.0), &format!("{pill_w:.0}"), px(52.0), BRAND_ACCENT, "fill", None));
    let live = write("live.txt".into(), "● LIVE".into()).await?;
    let mut live_text = text(bold, live, format!("{pill_x:.0}+({pill_w:.0}-tw)/2"), n(32.0), 30.0, "white");
    live_text.alpha = Some("0.65+0.35*sin(2*PI*t/1.6)".into());
    live_text.shadow = 0.0;
    filters.push(live_text.build());
    let label = write("label.txt".into(), spec.episode_label.clone()).await?;
    filters.push(text(regular, label, format!("{:.0}-tw", pill_x - px(28.0)), n(34.0), 30.0, "0xc9d3ff").build());

    let marcus = merge_windows(&spec.speakers, CueSpeaker::Marcus);
    let elena = merge_windows(&spec.speakers, CueSpeaker::Elena);
    let marcus_on = enable_expr(&marcus);
    let elena_on = enable_expr(&elena);

    let panel_y = px(140.0);
    let panel_h = px(620.0);
    let panel_w = px(500.0);
    let left_x = px(60.0);
    let right_x = w - px(60.0) - panel_w;
    let wall_x = px(600.0);
    let wall_w = w - 2.0 * px(600.0);
    let shadow_off = px(14.0);

    if spec.background_plate.is_none() {
        for (x, letter, name, role, accent, on) in [
            (left_x, "M", "MARCUS", "THE ENTHUSIAST", MARCUS_ACCENT, marcus_on.as_deref()),
            (right_x, "E", "ELENA", "THE SHARP COUNTERPART", ELENA_ACCENT, elena_on.as_deref()),
        ] {
            filters.push(drawbox(x + shadow_off, panel_y + shadow_off, &format!("{panel_w:.0}"), panel_h, "black@0.35", "fill", None));
            filters.push(drawbox(x, panel_y, &format!("{panel_w:.0}"), panel_h, "0x16203f@0.88", "fill", None));
            filters.push(drawbox(x, panel_y, &format!("{panel_w:.0}"), px(10.0), accent, "fill", None));
            let center = format!("{x:.0}+({panel_w:.0}-tw)/2");
            let letter_file = write(format!("letter_{letter}.txt"), letter.into()).await?;
            let mut big = text(bold, letter_file, center.clone(), n(190.0), 280.0, accent);
            big.shadow = px(6.0);
            filters.push(big.build());
            let name_file = write(format!("name_{letter}.txt"), name.into()).await?;
            filters.push(text(bold, name_file, center.clone(), n(640.0), 44.0, "white").build());
            let role_file = write(format!("role_{letter}.txt"), role.into()).await?;
            filters.push(text(regular, role_file, center, n(700.0), 24.0, "0xaab4dd").build());
            if let Some(expr) = on {
                filters.push(drawbox(x, panel_y, &format!("{panel_w:.0}"), panel_h, accent, &n(8.0), Some(expr)));
            }
        }
    }

    filters.push(drawbox(wall_x + shadow_off, panel_y + shadow_off, &format!("{wall_w:.0}"), panel_h, "black@0.35", "fill", None));
    filters.push(drawbox(wall_x, panel_y, &format!("{wall_w:.0}"), panel_h, "0x0d1430@0.9", "fill", None));
    filters.push(drawbox(wall_x, panel_y, &format!("{wall_w:.0}"), panel_h, "0x3a4a8a", &n(4.0), None));
    let lower_w = w - 2.0 * left_x;
    filters.push(drawbox(left_x + shadow_off, px(800.0) + shadow_off, &format!("{lower_w:.0}"), px(130.0), "black@0.35", "fill", None));
    filters.push(drawbox(left_x, px(800.0), &format!("{lower_w:.0}"), px(130.0), "0x0b1124@0.94", "fill", None));
    filters.push(drawbox(left_x, px(800.0), &n(12.0), px(130.0), BRAND_ACCENT, "fill", None));

    let inner_w = wall_w - px(72.0);
    let wrap_width = ((inner_w / px(44.0 * 0.62)) as usize).max(12);
    let notes_width = ((inner_w / px(30.0 * 0.56)) as usize).max(16);
    for (i, seg) in spec.segments.iter().enumerate() {
        let window = format!("between(t,{:.2},{:.2})", seg.start, seg.end);
        let after_card = seg.start + seg.card_seconds;
        let cat = write(format!("seg{i:02}_cat.txt"), seg.category.to_uppercase()).await?;
        let mut t = text(bold, cat, format!("{:.0}", wall_x + px(36.0)), n(172.0), 28.0, BRAND_ACCENT);
        t.enable = Some(window.clone());
        filters.push(t.build());
        let head_text = wrap_text(&seg.headline, wrap_width, 4);
        let head_lines = head_text.lines().count() as f64;
        let head = write(format!("seg{i:02}_head.txt"), head_text).await?;
        let mut t = text(bold, head, format!("{:.0}", wall_x + px(36.0)), n(232.0), 44.0, "white");
        t.enable = Some(window.clone());
        t.alpha = Some(fade_in(after_card, 0.6));
        t.line_spacing = px(12.0);
        filters.push(t.build());
        if !seg.notes.is_empty() {
            let notes_y = 232.0 + head_lines * 56.0 + 36.0;
            let max_lines = ((640.0 - notes_y) / 42.0).floor().max(1.0) as usize;
            filters.push(drawbox(wall_x + px(36.0), px(notes_y - 18.0), &n(80.0), px(4.0), BRAND_ACCENT, "fill", Some(&window)));
            let notes = write(format!("seg{i:02}_notes.txt"), wrap_text(&seg.notes, notes_width, max_lines)).await?;
            let mut t = text(regular, notes, format!("{:.0}", wall_x + px(36.0)), n(notes_y), 30.0, "0xdfe4ff");
            t.enable = Some(window.clone());
            t.alpha = Some(fade_in(after_card + 0.5, 0.6));
            t.line_spacing = px(10.0);
            t.shadow = 0.0;
            filters.push(t.build());
        }
        if !seg.attribution.is_empty() {
            let attr = write(format!("seg{i:02}_attr.txt"), wrap_text(&seg.attribution, (notes_width as f64 * 1.2) as usize, 2)).await?;
            let mut t = text(regular, attr, format!("{:.0}", wall_x + px(36.0)), n(680.0), 24.0, "0xaab4dd");
            t.enable = Some(window.clone());
            t.alpha = Some(fade_in(after_card + 0.3, 0.6));
            filters.push(t.build());
        }
        let title = write(format!("seg{i:02}_title.txt"), wrap_text(&seg.title, 90, 1)).await?;
        let mut t = text(regular, title, n(100.0), n(880.0), 28.0, "0xc9d3ff");
        t.enable = Some(window);
        filters.push(t.build());

        let card_end = seg.start + seg.card_seconds;
        let card_window = format!("between(t,{:.2},{:.2})", seg.start, card_end);
        filters.push(drawbox(0.0, 0.0, "iw", h, "0x0a0f24@0.96", "fill", Some(&card_window)));
        let card_title_text = wrap_text(&seg.title, 40, 2);
        let card_lines = card_title_text.lines().count() as f64;
        filters.push(drawbox(w * 0.35, h / 2.0 - px(80.0) + px(card_lines * 74.0 + 30.0), &format!("{:.0}", w * 0.3), px(6.0), BRAND_ACCENT, "fill", Some(&card_window)));
        let card_cat = write(format!("seg{i:02}_cardcat.txt"), seg.category.to_uppercase()).await?;
        let mut t = text(bold, card_cat, "(w-tw)/2".into(), format!("h/2-{}", n(150.0)), 36.0, BRAND_ACCENT);
        t.enable = Some(card_window.clone());
        t.alpha = Some(fade_in_out(seg.start, card_end, 0.35));
        filters.push(t.build());
        let card_title = write(format!("seg{i:02}_cardtitle.txt"), card_title_text).await?;
        let mut t = text(bold, card_title, "(w-tw)/2".into(), format!("h/2-{}", n(80.0)), 64.0, "white");
        t.enable = Some(card_window);
        t.alpha = Some(fade_in_out(seg.start + 0.15, card_end, 0.35));
        t.line_spacing = px(10.0);
        t.shadow = px(4.0);
        filters.push(t.build());
    }

    for (speaker, label_text, color, on) in [
        ("marcus", "MARCUS  ·  The Enthusiast", MARCUS_ACCENT, marcus_on.as_deref()),
        ("elena", "ELENA  ·  The Sharp Counterpart", ELENA_ACCENT, elena_on.as_deref()),
    ] {
        if let Some(expr) = on {
            let file = write(format!("lower_{speaker}.txt"), label_text.into()).await?;
            let mut t = text(bold, file, n(100.0), n(818.0), 42.0, color);
            t.enable = Some(expr.to_string());
            filters.push(t.build());
        }
    }

    filters.push(drawbox(0.0, px(980.0), "iw", px(70.0), BRAND_ACCENT, "fill", None));
    let ticker = write("ticker.txt".into(), spec.ticker.replace('\n', " ")).await?;
    let ticker_label = write("ticker_label.txt".into(), "HEADLINES".into()).await?;
    let mut t = text(bold, ticker, format!("w-mod(t*{:.0},w+tw)", px(150.0)), n(995.0), 36.0, "white");
    t.shadow = 0.0;
    filters.push(t.build());
    filters.push(drawbox(0.0, px(980.0), &n(300.0), px(70.0), "0x0b1124", "fill", None));
    let mut t = text(bold, ticker_label, n(36.0), n(997.0), 34.0, "white");
    t.shadow = 0.0;
    filters.push(t.build());

    let mut graph = format!("[0:v]{}[base]", filters.join(",\n"));
    if spec.background_plate.is_none() && (marcus_on.is_some() || elena_on.is_some()) {
        let wave_w = (panel_w - px(80.0)).round() as u32 / 2 * 2;
        let wave_h = px(110.0).round() as u32 / 2 * 2;
        let wave_y = px(500.0);
        let fps = spec.fps;
        graph.push_str(";\n[1:a]aformat=channel_layouts=mono,asplit=2[am][ae]");
        for (tag, color) in [("wm", MARCUS_ACCENT), ("we", ELENA_ACCENT)] {
            let src = if tag == "wm" { "am" } else { "ae" };
            graph.push_str(&format!(
                ";\n[{src}]showwaves=s={wave_w}x{wave_h}:mode=cline:rate={fps}:colors={color}:draw=full,format=rgba,colorchannelmixer=aa=0.9[{tag}]"
            ));
        }
        let mut last = "base".to_string();
        for (tag, x, on) in [("wm", left_x + px(40.0), marcus_on.as_deref()), ("we", right_x + px(40.0), elena_on.as_deref())] {
            let next = format!("{last}_{tag}");
            match on {
                Some(expr) => graph.push_str(&format!(";\n[{last}][{tag}]overlay=x={x:.0}:y={wave_y:.0}:format=yuv444:enable='{expr}'[{next}]")),
                None => graph.push_str(&format!(";\n[{tag}]nullsink;\n[{last}]null[{next}]")),
            }
            last = next;
        }
        graph.push_str(&format!(";\n[{last}]format=yuv420p[vout]"));
    } else {
        graph.push_str(";\n[base]format=yuv420p[vout]");
    }
    Ok(graph)
}

pub fn write_srt(cues: &[CaptionCue]) -> String {
    let mut out = String::new();
    let mut index = 1;
    for cue in cues {
        let chunks = caption_chunks(&cue.text, 84);
        let total_chars: usize = chunks.iter().map(|c| c.chars().count()).sum::<usize>().max(1);
        let span = (cue.end - cue.start).max(0.1);
        let mut t = cue.start;
        for chunk in chunks {
            let share = span * chunk.chars().count() as f64 / total_chars as f64;
            let end = (t + share).min(cue.end);
            writeln!(out, "{index}\n{} --> {}\n{}\n", srt_time(t), srt_time(end), wrap_text(&chunk, 42, 2)).ok();
            index += 1;
            t = end;
        }
    }
    out
}

fn caption_chunks(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

pub fn srt_time(seconds: f64) -> String {
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
}

pub fn chapter_time(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_splits_long_lines_and_keeps_timing() {
        let text = "word ".repeat(40);
        let srt = write_srt(&[CaptionCue { start: 1.0, end: 11.0, text }]);
        assert!(srt.starts_with("1\n00:00:01,000 --> "));
        assert!(srt.contains("\n3\n"));
        assert!(srt.trim_end().lines().any(|l| l.ends_with("00:00:11,000")));
    }

    #[test]
    fn merges_adjacent_speaker_windows() {
        let windows = vec![
            SpeakerWindow { speaker: CueSpeaker::Marcus, start: 0.0, end: 2.0 },
            SpeakerWindow { speaker: CueSpeaker::Marcus, start: 2.3, end: 4.0 },
            SpeakerWindow { speaker: CueSpeaker::Elena, start: 4.3, end: 6.0 },
            SpeakerWindow { speaker: CueSpeaker::Both, start: 6.3, end: 7.0 },
        ];
        assert_eq!(merge_windows(&windows, CueSpeaker::Marcus), vec![(0.0, 4.0), (6.3, 7.0)]);
        assert_eq!(merge_windows(&windows, CueSpeaker::Elena), vec![(4.3, 7.0)]);
    }

    #[test]
    fn parses_ebur128_summary() {
        let stderr = "[Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -14.2 LUFS\n    Threshold: -24.5 LUFS\n\n  Loudness range:\n    LRA:         4.1 LU\n\n  True peak:\n    Peak:       -1.6 dBFS";
        let report = parse_ebur128_summary(stderr).unwrap();
        assert_eq!(report.integrated_lufs, -14.2);
        assert_eq!(report.loudness_range_lu, 4.1);
        assert_eq!(report.true_peak_dbtp, -1.6);
    }

    #[test]
    fn caption_phrases_break_on_punctuation_and_length() {
        assert_eq!(
            caption_phrases("MAPPA just did it, again. This is a very long sentence indeed", 4, 22),
            vec!["MAPPA just did it,", "again.", "This is a very", "long sentence indeed"]
        );
    }

    #[test]
    fn wraps_and_truncates() {
        assert_eq!(wrap_text("one two three four", 9, 5), "one two\nthree\nfour");
        assert_eq!(wrap_text("one two three four", 9, 2), "one two\nthree…");
        assert_eq!(chapter_time(65.4), "1:05");
    }
}
