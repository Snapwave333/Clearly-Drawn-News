use super::*;

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub(crate) struct FreesoundSound {
    id: u64,
    name: String,
    duration: f64,
    preview_url: Option<String>,
}

pub(crate) struct FreesoundClient {
    api_key: Option<String>,
    cache_dir: PathBuf,
    client: reqwest::Client,
}

impl FreesoundClient {
    fn new(data_dir: &Path) -> Self {
        let key = std::env::var("FREESOUND_API_KEY").ok().or_else(|| {
            let key_file = data_dir.join("freesound_key.txt");
            std::fs::read_to_string(key_file).ok().map(|s| s.trim().to_string())
        }).or_else(|| {
            let creds_file = PathBuf::from("freesound_credentials.json");
            std::fs::read_to_string(creds_file).ok().and_then(|text| {
                serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| {
                    v.get("api_key").or_else(|| v.get("token")).and_then(|t| t.as_str()).map(|s| s.to_string())
                })
            })
        });

        let cache_dir = data_dir.join("audio").join("freesound_cache");
        let _ = std::fs::create_dir_all(&cache_dir);

        Self {
            api_key: key,
            cache_dir,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(12))
                .build()
                .unwrap_or_default(),
        }
    }

    fn has_key(&self) -> bool {
        self.api_key.as_ref().is_some_and(|k| !k.is_empty())
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<FreesoundSound>> {
        let key = self.api_key.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Freesound API key not configured. Set FREESOUND_API_KEY env var or save in freesound_credentials.json")
        })?;
        let encoded_query = query.replace(' ', "+");
        let url = format!(
            "https://freesound.org/apiv2/search/text/?query={encoded_query}&token={key}&fields=id,name,previews,duration&filter=duration:[0.1+TO+6.0]&page_size={}",
            limit.clamp(1, 15)
        );

        let resp = self.client.get(&url).send().await?.error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        let results = json.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();

        let mut sounds = Vec::new();
        for r in results {
            if let (Some(id), Some(name)) = (r.get("id").and_then(|v| v.as_u64()), r.get("name").and_then(|v| v.as_str())) {
                let preview_url = r.get("previews")
                    .and_then(|p| p.get("preview-hq-mp3").or_else(|| p.get("preview-lq-mp3")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let duration = r.get("duration").and_then(|v| v.as_f64()).unwrap_or(1.0);
                sounds.push(FreesoundSound {
                    id,
                    name: name.to_string(),
                    duration,
                    preview_url,
                });
            }
        }
        Ok(sounds)
    }

    async fn download(&self, sound_id: u64, preview_url: Option<&str>, target_name: Option<&str>) -> anyhow::Result<PathBuf> {
        let filename = target_name
            .map(|n| format!("{n}.wav"))
            .unwrap_or_else(|| format!("freesound_{sound_id}.wav"));
        let target_path = self.cache_dir.join(&filename);
        if target_path.is_file() {
            return Ok(target_path);
        }

        let url = if let Some(u) = preview_url {
            u.to_string()
        } else {
            let key = self.api_key.as_deref().ok_or_else(|| anyhow::anyhow!("Missing Freesound API key"))?;
            let info_url = format!("https://freesound.org/apiv2/sounds/{}/?token={key}", sound_id);
            let resp = self.client.get(&info_url).send().await?.error_for_status()?;
            let info: serde_json::Value = resp.json().await?;
            info.get("previews")
                .and_then(|p| p.get("preview-hq-mp3").or_else(|| p.get("preview-lq-mp3")))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("No preview audio available for Freesound ID {sound_id}"))?
                .to_string()
        };

        let temp_mp3 = self.cache_dir.join(format!("temp_{sound_id}.mp3"));
        let bytes = self.client.get(&url).send().await?.error_for_status()?.bytes().await?;
        tokio::fs::write(&temp_mp3, &bytes).await?;

        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(&temp_mp3)
            .args(["-ar", "44100", "-ac", "2", "-c:a", "pcm_s16le"])
            .arg(&target_path)
            .status()
            .await;
        let _ = tokio::fs::remove_file(&temp_mp3).await;

        if matches!(status, Ok(s) if s.success()) && target_path.is_file() {
            Ok(target_path)
        } else {
            anyhow::bail!("FFmpeg failed converting Freesound {sound_id} preview to WAV");
        }
    }

    async fn ensure_cached_effect(&self, semantic_name: &str, search_query: &str) -> Option<PathBuf> {
        let cached = self.cache_dir.join(format!("{semantic_name}.wav"));
        if cached.is_file() {
            return Some(cached);
        }
        if !self.has_key() {
            return None;
        }
        match self.search(search_query, 3).await {
            Ok(sounds) if !sounds.is_empty() => {
                let first = &sounds[0];
                match self.download(first.id, first.preview_url.as_deref(), Some(semantic_name)).await {
                    Ok(path) => {
                        println!("   ✓ Freesound.org cached: {semantic_name} ({})", first.name);
                        Some(path)
                    }
                    Err(e) => {
                        eprintln!("Freesound download warning for {semantic_name}: {e}");
                        None
                    }
                }
            }
            Ok(_) => None,
            Err(e) => {
                eprintln!("Freesound query warning for {semantic_name}: {e}");
                None
            }
        }
    }
}

pub(crate) async fn cmd_freesound(store: &Store, action: FreesoundAction) -> anyhow::Result<()> {
    let client = FreesoundClient::new(store.data_dir());
    match action {
        FreesoundAction::Status => {
            println!("Freesound.org Integration Status");
            println!("================================");
            if client.has_key() {
                println!("API Key:       Configured (Active)");
            } else {
                println!("API Key:       Not Configured");
                println!("  To activate: set FREESOUND_API_KEY environment variable or save key in freesound_credentials.json");
            }
            println!("Cache Directory: {}", client.cache_dir.display());
            let cached_count = std::fs::read_dir(&client.cache_dir)
                .ok()
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|e| e.path().extension().is_some_and(|ext| ext == "wav"))
                        .count()
                })
                .unwrap_or(0);
            println!("Cached SFX:    {} sound file(s)", cached_count);
        }
        FreesoundAction::Search { query, limit } => {
            println!("Searching Freesound.org for: '{}' (limit {})", query, limit);
            println!("------------------------------------------------------------");
            let results = client.search(&query, limit).await?;
            if results.is_empty() {
                println!("No sounds found matching query.");
            } else {
                for sound in results {
                    println!("[ID: {:>8}] {:.2}s  {}", sound.id, sound.duration, sound.name);
                    if let Some(preview) = sound.preview_url {
                        println!("             Preview: {}", preview);
                    }
                }
            }
        }
        FreesoundAction::Download { sound_id, name } => {
            println!("Downloading Freesound sound #{}...", sound_id);
            let path = client.download(sound_id, None, name.as_deref()).await?;
            println!("Successfully saved to: {}", path.display());
        }
    }
    Ok(())
}

pub(crate) async fn generate_aiwaves_audio(
    project_id: &str,
    topic: &str,
    project_dir: &Path,
    data_dir: &Path,
    duration: f64,
    shot_specs: &[atlas_blender::ShotSpec],
) -> (Option<PathBuf>, Option<PathBuf>) {
    let aiwaves_dir = PathBuf::from(
        std::env::var("AIWAVES_DIR").unwrap_or_else(|_| r"C:\Users\chrom\Desktop\aiwaves".to_string()),
    );
    let audio_dir = project_dir.join("audio");
    let _ = tokio::fs::create_dir_all(&audio_dir).await;

    let score_path = audio_dir.join(format!("aiwaves_score_{project_id}.wav"));
    let sfx_path = audio_dir.join(format!("aiwaves_sfx_{project_id}.wav"));

    let freesound = FreesoundClient::new(data_dir);
    if freesound.has_key() {
        println!("🎵 Freesound.org active: syncing authentic whiteboard sound effects...");
    } else {
        println!("🎵 Freesound.org: using local cache and AIWaves sound library (set FREESOUND_API_KEY to search online)");
    }

    // 1. Generate unique, topic-attuned original musical score
    let score_res = if score_path.is_file() {
        Some(score_path.clone())
    } else {
        let hash: u64 = topic
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let root_freqs = [130.81, 146.83, 164.81, 174.61, 196.00, 220.00];
        let root = root_freqs[(hash % root_freqs.len() as u64) as usize];
        let fifth = root * 1.5;
        let ninth = root * 2.25;
        let sub = root * 0.5;
        let safe_dur = duration.max(5.0);
        let fade_out_start = (safe_dur - 2.5).max(1.0);

        let aiwaves_master = aiwaves_dir
            .join("fixtures")
            .join("longing")
            .join("audio")
            .join("master.wav");
        let status = if aiwaves_master.is_file() {
            let filter = format!(
                "[0:a]lowpass=f=600,volume=0.18,atrim=duration={safe_dur:.2}[a0];[1:a]lowpass=f=420,volume=0.25[a1];[a0][a1]amix=inputs=2:duration=first:dropout_transition=0,afade=t=in:st=0:d=2.0,afade=t=out:st={fade_out_start:.2}:d=2.5"
            );
            tokio::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error"])
                .args(["-stream_loop", "-1", "-i"])
                .arg(&aiwaves_master)
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    &format!(
                        "aevalsrc=0.03*sin(2*PI*{root:.2}*t)+0.02*sin(2*PI*{fifth:.2}*t)+0.015*sin(2*PI*{ninth:.2}*t)+0.025*sin(2*PI*{sub:.2}*t):d={safe_dur:.2}:s=44100"
                    ),
                ])
                .args(["-filter_complex", &filter, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"])
                .arg(&score_path)
                .status()
                .await
        } else {
            let filter = format!(
                "aevalsrc=0.04*sin(2*PI*{root:.2}*t)+0.03*sin(2*PI*{fifth:.2}*t)+0.02*sin(2*PI*{ninth:.2}*t)+0.035*sin(2*PI*{sub:.2}*t):d={safe_dur:.2}:s=44100,lowpass=f=450,afade=t=in:st=0:d=2.0,afade=t=out:st={fade_out_start:.2}:d=2.5"
            );
            tokio::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", &filter, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"])
                .arg(&score_path)
                .status()
                .await
        };
        if matches!(status, Ok(s) if s.success()) && score_path.is_file() {
            Some(score_path)
        } else {
            None
        }
    };

    // 2. Generate original synchronized procedural sound effects (SFX)
    let sfx_res = if sfx_path.is_file() {
        Some(sfx_path.clone())
    } else {
        let safe_dur = duration.max(5.0);

        // Pre-fetch or query freesound effects if available
        let marker_draw = freesound.ensure_cached_effect("marker_draw", "marker whiteboard drawing").await;
        let eraser_swoosh = freesound.ensure_cached_effect("whiteboard_swoosh", "whiteboard eraser whoosh").await;

        let intro_sfx = aiwaves_dir.join("media").join("intro").join("intro_sfx.wav");
        let pan_notes = [
            aiwaves_dir.join("panNoteC.wav"),
            aiwaves_dir.join("panNoteD.wav"),
            aiwaves_dir.join("panNoteE.wav"),
            aiwaves_dir.join("panNoteG.wav"),
            aiwaves_dir.join("panNoteA.wav"),
        ];

        let mut cmd = tokio::process::Command::new("ffmpeg");
        cmd.args(["-y", "-loglevel", "error"]);

        let mut inputs_count = 0;
        let mut filter_parts = Vec::new();

        // Opening sound effect (intro reveal)
        if intro_sfx.is_file() {
            cmd.arg("-i").arg(&intro_sfx);
            filter_parts.push(format!("[{inputs_count}:a]volume=0.35,atrim=duration=4[s{inputs_count}]"));
            inputs_count += 1;
        }

        // Shot boundary chimes & marker sound effects
        let mut current_frame = 0u32;
        for (i, spec) in shot_specs.iter().enumerate() {
            if i > 0 {
                let delay_ms = (current_frame as f64 / spec.fps as f64 * 1000.0).round() as u64;

                // Alternate between Freesound swoosh and AIWaves melodic chimes
                if let Some(swoosh) = &eraser_swoosh {
                    if i % 2 == 1 {
                        cmd.arg("-i").arg(swoosh);
                        filter_parts.push(format!("[{inputs_count}:a]adelay={delay_ms}:all=1,volume=0.28,atrim=duration=2.5[s{inputs_count}]"));
                        inputs_count += 1;
                    }
                }

                let note_idx = (i - 1) % pan_notes.len();
                let note_path = &pan_notes[note_idx];
                if note_path.is_file() {
                    cmd.arg("-i").arg(note_path);
                    filter_parts.push(format!("[{inputs_count}:a]adelay={delay_ms}:all=1,volume=0.32,atrim=duration=3[s{inputs_count}]"));
                    inputs_count += 1;
                }
            }
            current_frame += spec.duration_frames;
        }

        // Add marker stroke Foley at start of first shot
        if let Some(draw) = &marker_draw {
            cmd.arg("-i").arg(draw);
            filter_parts.push(format!("[{inputs_count}:a]adelay=400:all=1,volume=0.22,atrim=duration=3.5[s{inputs_count}]"));
            inputs_count += 1;
        }

        if inputs_count > 0 {
            let stream_tags = (0..inputs_count).map(|idx| format!("[s{idx}]")).collect::<String>();
            let complex = format!(
                "{};{stream_tags}amix=inputs={inputs_count}:duration=first:dropout_transition=0,atrim=duration={safe_dur:.2}",
                filter_parts.join(";")
            );
            cmd.args(["-filter_complex", &complex, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"]).arg(&sfx_path);
            let status = cmd.status().await;
            if matches!(status, Ok(s) if s.success()) && sfx_path.is_file() {
                Some(sfx_path)
            } else {
                None
            }
        } else {
            let complex = format!(
                "anoisesrc=d={safe_dur:.2}:c=white:r=44100,bandpass=f=1100:w=500,volume=0.18,afade=t=in:st=0:d=0.08,afade=t=out:st=0.12:d=0.20"
            );
            let status = tokio::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", &complex, "-t", &format!("{safe_dur:.2}"), "-c:a", "pcm_s16le"])
                .arg(&sfx_path)
                .status()
                .await;
            if matches!(status, Ok(s) if s.success()) && sfx_path.is_file() {
                Some(sfx_path)
            } else {
                None
            }
        }
    };

    (score_res, sfx_res)
}
