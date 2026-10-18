use super::*;
use crate::news_voice::VoicedLine;
use atlas_storyboard::two_anchor_script::AnchorRole;
use std::collections::BTreeMap;

pub(crate) const LIPSYNC_FPS: u32 = 30;
const ANALYSIS_RATE: u32 = 16_000;
const NOISE_FLOOR: f32 = 0.02;
const ATTACK: f32 = 0.6;
const RELEASE: f32 = 0.25;

pub(crate) fn envelope(samples: &[f32], sample_rate: u32, fps: u32) -> Vec<f32> {
    let window = (sample_rate / fps).max(1) as usize;
    let rms = samples
        .chunks(window)
        .map(|chunk| (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt())
        .collect::<Vec<_>>();
    let peak = rms.iter().copied().fold(0.0_f32, f32::max);
    if peak <= f32::EPSILON {
        return vec![0.0; rms.len()];
    }
    let mut smoothed = 0.0_f32;
    rms.into_iter()
        .map(|value| {
            let level = ((value / peak - NOISE_FLOOR) / (1.0 - NOISE_FLOOR)).clamp(0.0, 1.0).sqrt();
            let rate = if level > smoothed { ATTACK } else { RELEASE };
            smoothed += (level - smoothed) * rate;
            (smoothed * 100.0).round() / 100.0
        })
        .collect()
}

async fn decode_mono(path: &Path) -> anyhow::Result<Vec<f32>> {
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-ac", "1", "-ar", &ANALYSIS_RATE.to_string(), "-"])
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!("FFmpeg could not decode {}: {}", path.display(), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn speaker_track(speaker: AnchorRole) -> &'static [&'static str] {
    match speaker {
        AnchorRole::Enthusiast => &["marcus"],
        AnchorRole::Counterpart => &["elena"],
        AnchorRole::Duo => &["marcus", "elena"],
    }
}

pub(crate) fn place(tracks: &mut BTreeMap<&'static str, Vec<f32>>, speaker: AnchorRole, start_frame: usize, values: &[f32]) {
    for name in speaker_track(speaker) {
        let track = tracks.entry(name).or_default();
        if track.len() < start_frame + values.len() {
            track.resize(start_frame + values.len(), 0.0);
        }
        for (offset, value) in values.iter().enumerate() {
            let slot = &mut track[start_frame + offset];
            *slot = slot.max(*value);
        }
    }
}

pub(crate) async fn write_lipsync(episode_dir: &Path, lines: &[VoicedLine], total_seconds: f32) -> anyhow::Result<PathBuf> {
    let total_frames = (total_seconds * LIPSYNC_FPS as f32).ceil() as usize + LIPSYNC_FPS as usize;
    let mut tracks: BTreeMap<&'static str, Vec<f32>> = BTreeMap::new();
    tracks.insert("marcus", vec![0.0; total_frames]);
    tracks.insert("elena", vec![0.0; total_frames]);
    for line in lines {
        let samples = decode_mono(&line.file).await?;
        let values = envelope(&samples, ANALYSIS_RATE, LIPSYNC_FPS);
        let start_frame = (line.start_seconds * LIPSYNC_FPS as f32).round() as usize;
        place(&mut tracks, line.speaker, start_frame, &values);
    }
    let path = episode_dir.join("lipsync.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "fps": LIPSYNC_FPS,
            "control": "jaw_open",
            "tracks": tracks,
        }))?,
    )?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_opens_on_speech_and_closes_in_silence() {
        let rate = 16_000;
        let mut samples = vec![0.0_f32; rate as usize / 2];
        samples.extend((0..rate as usize / 2).map(|i| (i as f32 * 0.2).sin() * 0.5));
        samples.extend(vec![0.0_f32; rate as usize / 2]);
        let values = envelope(&samples, rate, 30);
        assert_eq!(values.len(), 46);
        assert!(values[..14].iter().all(|v| *v == 0.0));
        assert!(values[20..29].iter().all(|v| *v > 0.9), "{values:?}");
        assert!(values[45] < 0.05);
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(envelope(&[0.0; 1000], rate, 30).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn duo_lines_move_both_anchors() {
        let mut tracks = BTreeMap::new();
        place(&mut tracks, AnchorRole::Counterpart, 2, &[0.5, 0.7]);
        place(&mut tracks, AnchorRole::Duo, 3, &[0.4, 0.9]);
        assert_eq!(tracks["elena"], vec![0.0, 0.0, 0.5, 0.7, 0.9]);
        assert_eq!(tracks["marcus"], vec![0.0, 0.0, 0.0, 0.4, 0.9]);
    }
}
