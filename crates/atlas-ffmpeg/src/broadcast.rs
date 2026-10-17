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
        let graph = build_filtergraph(spec, &regular, &bold, &txt_dir).await?;
        tokio::fs::write(spec.work_dir.join("filtergraph.txt"), graph).await?;

        let preset = std::env::var("ATLAS_NEWS_X264_PRESET").unwrap_or_else(|_| "veryfast".into());
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
                format!("color=c=0x0b1020:s={}x{}:r={}", spec.width, spec.height, spec.fps),
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
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            preset,
            "-crf".into(),
            "20".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-g".into(),
            (spec.fps * 2).to_string(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
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
        let graph = format!(
            "color=c=0x0b1020:s=1280x720:d=1,drawbox=x=0:y=0:w=iw:h=120:color=0x111a33:t=fill,drawbox=x=0:y=120:w=iw:h=8:color={BRAND_ACCENT}:t=fill,drawbox=x=0:y=660:w=iw:h=60:color={BRAND_ACCENT}:t=fill,{},{},{}[out]",
            drawtext(&bold, "txt/thumb_show.txt", "48", "36", 54.0, "white", None),
            drawtext(&regular, "txt/thumb_label.txt", "48", "672", 30.0, "white", None),
            drawtext(&bold, "txt/thumb_headline.txt", "48", "175", 70.0, "white", None).replace(":expansion=none", ":line_spacing=14:expansion=none"),
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

async fn build_filtergraph(spec: &BroadcastSpec, regular: &str, bold: &str, txt_dir: &Path) -> AtlasResult<String> {
    let k = spec.width as f64 / 1920.0;
    let px = |v: f64| v * k;
    let w = spec.width as f64;
    let mut filters: Vec<String> = Vec::new();
    let write = |name: String, body: String| async move {
        tokio::fs::write(txt_dir.join(&name), body).await.map(|_| format!("txt/{name}"))
    };

    if spec.background_plate.is_some() {
        filters.push(format!(
            "scale={}:{}:force_original_aspect_ratio=increase,crop={}:{},fps={},setsar=1",
            spec.width, spec.height, spec.width, spec.height, spec.fps
        ));
    }
    filters.push("format=yuv420p".into());

    filters.push(drawbox(0.0, 0.0, "iw", px(96.0), "0x111a33", "fill", None));
    filters.push(drawbox(0.0, px(96.0), "iw", px(6.0), BRAND_ACCENT, "fill", None));
    let show = write("show.txt".into(), spec.show_title.to_uppercase()).await?;
    filters.push(drawtext(bold, &show, &format!("{:.0}", px(48.0)), &format!("{:.0}", px(24.0)), px(46.0), "white", None));
    let label = write("label.txt".into(), spec.episode_label.clone()).await?;
    filters.push(drawtext(regular, &label, &format!("w-tw-{:.0}", px(48.0)), &format!("{:.0}", px(34.0)), px(30.0), "0xc9d3ff", None));

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

    if spec.background_plate.is_none() {
        for (x, letter, name, role, accent, on) in [
            (left_x, "M", "MARCUS", "THE ENTHUSIAST", MARCUS_ACCENT, marcus_on.as_deref()),
            (right_x, "E", "ELENA", "THE SHARP COUNTERPART", ELENA_ACCENT, elena_on.as_deref()),
        ] {
            filters.push(drawbox(x, panel_y, &format!("{panel_w:.0}"), panel_h, "0x1b2547", "fill", None));
            let letter_file = write(format!("letter_{letter}.txt"), letter.into()).await?;
            filters.push(drawtext(
                bold,
                &letter_file,
                &format!("{x:.0}+({panel_w:.0}-tw)/2"),
                &format!("{:.0}", px(190.0)),
                px(280.0),
                accent,
                None,
            ));
            let name_file = write(format!("name_{letter}.txt"), name.into()).await?;
            filters.push(drawtext(bold, &name_file, &format!("{x:.0}+({panel_w:.0}-tw)/2"), &format!("{:.0}", px(640.0)), px(44.0), "white", None));
            let role_file = write(format!("role_{letter}.txt"), role.into()).await?;
            filters.push(drawtext(regular, &role_file, &format!("{x:.0}+({panel_w:.0}-tw)/2"), &format!("{:.0}", px(700.0)), px(24.0), "0xaab4dd", None));
            if let Some(expr) = on {
                filters.push(drawbox(x, panel_y, &format!("{panel_w:.0}"), panel_h, accent, &format!("{:.0}", px(8.0)), Some(expr)));
            }
        }
    }

    filters.push(drawbox(wall_x, panel_y, &format!("{wall_w:.0}"), panel_h, "0x0f1630@0.92", "fill", None));
    filters.push(drawbox(wall_x, panel_y, &format!("{wall_w:.0}"), panel_h, "0x3a4a8a", &format!("{:.0}", px(4.0)), None));
    filters.push(drawbox(left_x, px(800.0), &format!("{:.0}", w - 2.0 * left_x), px(130.0), "0x111a33@0.94", "fill", None));
    filters.push(drawbox(left_x, px(800.0), &format!("{:.0}", px(12.0)), px(130.0), BRAND_ACCENT, "fill", None));

    let wrap_width = ((wall_w / px(24.0)) as usize).max(16);
    for (i, seg) in spec.segments.iter().enumerate() {
        let window = format!("between(t,{:.2},{:.2})", seg.start, seg.end);
        let cat = write(format!("seg{i:02}_cat.txt"), seg.category.to_uppercase()).await?;
        filters.push(drawtext(bold, &cat, &format!("{:.0}", wall_x + px(36.0)), &format!("{:.0}", px(170.0)), px(28.0), BRAND_ACCENT, Some(&window)));
        let head = write(format!("seg{i:02}_head.txt"), wrap_text(&seg.headline, wrap_width, 6)).await?;
        filters.push(
            drawtext(bold, &head, &format!("{:.0}", wall_x + px(36.0)), &format!("{:.0}", px(230.0)), px(44.0), "white", Some(&window))
                .replace(":expansion=none", &format!(":line_spacing={:.0}:expansion=none", px(12.0))),
        );
        if !seg.attribution.is_empty() {
            let attr = write(format!("seg{i:02}_attr.txt"), wrap_text(&seg.attribution, (wrap_width as f64 * 1.6) as usize, 2)).await?;
            filters.push(drawtext(regular, &attr, &format!("{:.0}", wall_x + px(36.0)), &format!("{:.0}", px(680.0)), px(24.0), "0xaab4dd", Some(&window)));
        }
        let title = write(format!("seg{i:02}_title.txt"), wrap_text(&seg.title, 90, 1)).await?;
        filters.push(drawtext(regular, &title, &format!("{:.0}", px(100.0)), &format!("{:.0}", px(880.0)), px(28.0), "0xc9d3ff", Some(&window)));

        let card_window = format!("between(t,{:.2},{:.2})", seg.start, seg.start + seg.card_seconds);
        filters.push(drawbox(0.0, 0.0, "iw", spec.height as f64, "0x0b1020@0.97", "fill", Some(&card_window)));
        filters.push(drawbox(0.0, spec.height as f64 / 2.0 + px(70.0), "iw", px(6.0), BRAND_ACCENT, "fill", Some(&card_window)));
        let card_cat = write(format!("seg{i:02}_cardcat.txt"), seg.category.to_uppercase()).await?;
        filters.push(drawtext(bold, &card_cat, "(w-tw)/2", &format!("h/2-{:.0}", px(150.0)), px(36.0), BRAND_ACCENT, Some(&card_window)));
        let card_title = write(format!("seg{i:02}_cardtitle.txt"), wrap_text(&seg.title, 40, 2)).await?;
        filters.push(
            drawtext(bold, &card_title, "(w-tw)/2", &format!("h/2-{:.0}", px(80.0)), px(64.0), "white", Some(&card_window))
                .replace(":expansion=none", &format!(":line_spacing={:.0}:expansion=none", px(10.0))),
        );
    }

    for (speaker, label_text, color, on) in [
        ("marcus", "MARCUS  ·  The Enthusiast", MARCUS_ACCENT, marcus_on.as_deref()),
        ("elena", "ELENA  ·  The Sharp Counterpart", ELENA_ACCENT, elena_on.as_deref()),
    ] {
        if let Some(expr) = on {
            let file = write(format!("lower_{speaker}.txt"), label_text.into()).await?;
            filters.push(drawtext(bold, &file, &format!("{:.0}", px(100.0)), &format!("{:.0}", px(818.0)), px(42.0), color, Some(expr)));
        }
    }

    filters.push(drawbox(0.0, px(980.0), "iw", px(70.0), BRAND_ACCENT, "fill", None));
    let ticker = write("ticker.txt".into(), spec.ticker.replace('\n', " ")).await?;
    let ticker_label = write("ticker_label.txt".into(), "HEADLINES".into()).await?;
    let speed = px(150.0);
    filters.push(format!(
        "drawtext=fontfile={bold}:textfile={ticker}:x='{left:.0}+w-mod(t*{speed:.0},w+tw)':y={y:.0}:fontsize={size:.0}:fontcolor=white:expansion=none",
        left = px(0.0),
        y = px(995.0),
        size = px(36.0),
    ));
    filters.push(drawbox(0.0, px(980.0), &format!("{:.0}", px(260.0)), px(70.0), "0x111a33", "fill", None));
    filters.push(drawtext(bold, &ticker_label, &format!("{:.0}", px(36.0)), &format!("{:.0}", px(997.0)), px(34.0), "white", None));

    let mut graph = format!("[0:v]{}[base]", filters.join(",\n"));
    if spec.background_plate.is_none() && (marcus_on.is_some() || elena_on.is_some()) {
        let wave_w = (panel_w - px(80.0)).round() as u32 / 2 * 2;
        let wave_h = px(110.0).round() as u32 / 2 * 2;
        let wave_y = px(500.0);
        graph.push_str(&format!(
            ";\n[1:a]aformat=channel_layouts=mono,showwaves=s={wave_w}x{wave_h}:mode=cline:rate={fps}:colors=white,format=rgba,colorchannelmixer=aa=0.85,split=2[wm][we]",
            fps = spec.fps
        ));
        let mut last = "base".to_string();
        for (tag, x, on) in [("wm", left_x + px(40.0), marcus_on.as_deref()), ("we", right_x + px(40.0), elena_on.as_deref())] {
            let next = format!("{last}_{tag}");
            match on {
                Some(expr) => graph.push_str(&format!(";\n[{last}][{tag}]overlay=x={x:.0}:y={wave_y:.0}:enable='{expr}'[{next}]")),
                None => graph.push_str(&format!(";\n[{tag}]nullsink;\n[{last}]null[{next}]")),
            }
            last = next;
        }
        graph.push_str(&format!(";\n[{last}]null[vout]"));
    } else {
        graph.push_str(";\n[base]null[vout]");
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
    fn wraps_and_truncates() {
        assert_eq!(wrap_text("one two three four", 9, 5), "one two\nthree\nfour");
        assert_eq!(wrap_text("one two three four", 9, 2), "one two\nthree…");
        assert_eq!(chapter_time(65.4), "1:05");
    }
}
