use super::*;
use crate::news_voice::VoicedLine;
use atlas_storyboard::two_anchor_script::{AnchorRole, MasterEpisodeScript};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const FPS: u32 = 30;
const WRAP_CHARS: usize = 54;
const MAX_CAPTION_LINES: usize = 4;
const TAIL_SECONDS: f32 = 1.0;

#[derive(Deserialize)]
struct VoiceTimeline {
    lines: Vec<VoicedLine>,
}

pub(crate) struct Animatic {
    pub(crate) path: PathBuf,
    pub(crate) duration_seconds: f32,
}

struct Caption {
    start: f32,
    end: f32,
    speaker: AnchorRole,
    text: String,
}

struct Title {
    start: f32,
    end: f32,
    text: String,
}

fn wrap(text: &str, width: usize, max_lines: usize) -> String {
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

fn speaker_style(speaker: AnchorRole) -> (&'static str, &'static str) {
    match speaker {
        AnchorRole::Enthusiast => ("MARCUS · The Enthusiast", "0x5aa9e6"),
        AnchorRole::Counterpart => ("ELENA · The Sharp Counterpart", "0xf2a65a"),
        AnchorRole::Duo => ("MARCUS & ELENA", "0x9bd18b"),
    }
}

fn plan(script: &MasterEpisodeScript, timeline: &[VoicedLine]) -> (Vec<Title>, Vec<Caption>) {
    let spoken = script
        .segments
        .iter()
        .flat_map(|segment| {
            segment
                .lines
                .iter()
                .enumerate()
                .map(move |(index, line)| ((segment.segment_number, index), line.spoken_text.clone()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let captions = timeline
        .iter()
        .map(|line| Caption {
            start: line.start_seconds,
            end: line.start_seconds + line.duration_seconds,
            speaker: line.speaker,
            text: spoken
                .get(&(line.segment_number, line.line_index))
                .cloned()
                .unwrap_or_else(|| line.text.clone()),
        })
        .collect::<Vec<_>>();
    let titles = script
        .segments
        .iter()
        .filter_map(|segment| {
            let lines = timeline.iter().filter(|line| line.segment_number == segment.segment_number);
            let start = lines.clone().map(|line| line.start_seconds).reduce(f32::min)?;
            let end = lines.map(|line| line.start_seconds + line.duration_seconds).reduce(f32::max)?;
            Some(Title { start, end, text: segment.segment_title.clone() })
        })
        .collect();
    (titles, captions)
}

fn font_option() -> String {
    std::env::var("ATLAS_PREVIEW_FONT")
        .ok()
        .filter(|font| !font.trim().is_empty() && font.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-'))
        .map(|font| format!(":font='{font}'"))
        .unwrap_or_default()
}

fn filter_graph(titles: &[Title], captions: &[Caption], duration: f32, text_dir: &Path) -> anyhow::Result<String> {
    std::fs::create_dir_all(text_dir)?;
    let font = font_option();
    let window = |start: f32, end: f32| format!("enable='between(t,{start:.3},{end:.3})'");
    let mut filters = vec![
        format!("color=c=0x14202b:s={WIDTH}x{HEIGHT}:r={FPS}:d={duration:.3}"),
        "drawbox=x=0:y=0:w=iw:h=64:color=0x0b131b:t=fill".to_string(),
        format!("drawtext=text='{}'{font}:fontcolor=0xf2a65a:fontsize=30:x=32:y=17", atlas_core::SHOW_TITLE.to_uppercase()),
        format!("drawtext=text='PREVIEW - NOT FOR BROADCAST'{font}:fontcolor=white@0.55:fontsize=18:x=w-tw-32:y=24"),
        "drawbox=x=64:y=440:w=iw-128:h=2:color=white@0.15:t=fill".to_string(),
    ];
    for (index, title) in titles.iter().enumerate() {
        let file = format!("title_{:02}.txt", index + 1);
        std::fs::write(text_dir.join(&file), wrap(&title.text, 40, 2))?;
        filters.push(format!(
            "drawtext=textfile='text/{file}':expansion=none{font}:fontcolor=white:fontsize=44:line_spacing=12:x=64:y=150:{}",
            window(title.start, title.end)
        ));
    }
    for (index, caption) in captions.iter().enumerate() {
        let (name, color) = speaker_style(caption.speaker);
        let file = format!("line_{:04}.txt", index + 1);
        std::fs::write(text_dir.join(&file), wrap(&caption.text, WRAP_CHARS, MAX_CAPTION_LINES))?;
        let shown = window(caption.start, caption.end);
        filters.push(format!("drawbox=x=64:y=470:w=8:h=190:color={color}:t=fill:{shown}"));
        filters.push(format!("drawtext=text='{name}'{font}:fontcolor={color}:fontsize=26:x=92:y=472:{shown}"));
        filters.push(format!(
            "drawtext=textfile='text/{file}':expansion=none{font}:fontcolor=white:fontsize=30:line_spacing=10:x=92:y=516:{shown}"
        ));
    }
    Ok(format!("{}[v]", filters.join(",\n")))
}

pub(crate) async fn render_animatic(episode_dir: &Path, script: &MasterEpisodeScript) -> anyhow::Result<Animatic> {
    let timeline: VoiceTimeline = serde_json::from_slice(&std::fs::read(episode_dir.join("voice_timeline.json"))?)?;
    if timeline.lines.is_empty() {
        anyhow::bail!("voice timeline has no lines");
    }
    let narration = episode_dir.join("audio").join("episode_narration.wav");
    if !narration.is_file() {
        anyhow::bail!("missing {}", narration.display());
    }
    let (titles, captions) = plan(script, &timeline.lines);
    let duration = captions.iter().map(|c| c.end).fold(0.0_f32, f32::max) + TAIL_SECONDS;
    let preview_dir = episode_dir.join("preview");
    let graph = filter_graph(&titles, &captions, duration, &preview_dir.join("text"))?;
    std::fs::write(preview_dir.join("graph.txt"), graph)?;
    let output = preview_dir.join("animatic.mp4");
    let result = tokio::process::Command::new("ffmpeg")
        .current_dir(&preview_dir)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-filter_complex_script", "graph.txt", "-i"])
        .arg(&narration)
        .args(["-map", "[v]", "-map", "0:a", "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "160k", "-t", &format!("{duration:.3}"), "animatic.mp4"])
        .kill_on_drop(true)
        .output()
        .await?;
    if !result.status.success() || !output.is_file() {
        anyhow::bail!("FFmpeg animatic render failed: {}", String::from_utf8_lossy(&result.stderr).trim());
    }
    Ok(Animatic { path: output, duration_seconds: duration })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_truncates_captions() {
        assert_eq!(wrap("one two three four", 9, 4), "one two\nthree\nfour");
        assert_eq!(wrap("aaaa bbbb cccc dddd", 4, 2), "aaaa\nbbbb…");
        assert_eq!(wrap("", 10, 2), "");
    }

    #[test]
    fn filter_graph_uses_text_files_and_timed_windows() {
        let dir = std::env::temp_dir().join(format!("atlas_preview_{}", uuid::Uuid::new_v4().simple()));
        let titles = vec![Title { start: 0.0, end: 5.0, text: "MAPPA: 50% \"Bigger\" Season".into() }];
        let captions = vec![
            Caption { start: 0.0, end: 2.5, speaker: AnchorRole::Enthusiast, text: "It's 100% happening: really!".into() },
            Caption { start: 2.5, end: 5.0, speaker: AnchorRole::Counterpart, text: "Is it, though?".into() },
        ];
        let graph = filter_graph(&titles, &captions, 6.0, &dir).unwrap();
        assert!(graph.starts_with("color=c=0x14202b:s=1280x720:r=30:d=6.000"));
        assert!(graph.ends_with("[v]"));
        assert!(graph.contains("textfile='text/line_0001.txt':expansion=none"));
        assert!(graph.contains("enable='between(t,2.500,5.000)'"));
        assert!(graph.contains("ELENA · The Sharp Counterpart"));
        assert!(!graph.contains("100%"));
        assert_eq!(std::fs::read_to_string(dir.join("line_0001.txt")).unwrap(), "It's 100% happening: really!");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
