use super::*;

pub(crate) struct ProduceOptions<'a> {
    pub(crate) topic_id: &'a str,
    pub(crate) reuse_rendered: bool,
    pub(crate) format: VideoFormat,
    pub(crate) publish: bool,
    pub(crate) privacy: PublishPrivacy,
    pub(crate) platform: PublishPlatformCli,
    pub(crate) tiktok_user: &'a str,
    pub(crate) tiktok_draft: bool,
}

pub(crate) async fn cmd_produce(store: &Store, options: ProduceOptions<'_>) -> anyhow::Result<()> {
    let ProduceOptions {
        topic_id,
        reuse_rendered,
        format,
        publish,
        privacy,
        platform,
        tiktok_user,
        tiktok_draft,
    } = options;
    let mut project = store
        .get_project(topic_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {}", topic_id))?;
    let profile = format.profile();
    let render_project_id = format!("{}_{}", project.video_id, profile.slug);

    println!("═══ {} Vertical Slice ═══", atlas_core::COMPANY_NAME);
    println!("Project: {} — {}", project.video_id, project.topic.title);
    println!(
        "Format: {} ({}x{}, {} shots)",
        profile.slug, profile.width, profile.height, profile.shot_count
    );
    println!();

    let mut timer = StageTimer::new();
    timer.begin("storyboard");
    println!("📝 Stage 1: Generating storyboard from script...");
    project.state = ProjectState::Scripting;
    store.insert_project(&project)?;
    let project_dir = store
        .data_dir()
        .join("projects")
        .join(&project.video_id)
        .join(profile.slug);
    let long_storyboard_path = store
        .data_dir()
        .join("projects")
        .join(&project.video_id)
        .join("long")
        .join("storyboard.json");
    let long_plan = if matches!(format, VideoFormat::Short) {
        if long_storyboard_path.is_file() {
            std::fs::read(&long_storyboard_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<StoryboardPlan>(&bytes).ok())
        } else {
            None
        }
    } else {
        None
    };
    let planned = planned_video(&project.topic.title, format, long_plan.as_ref()).await?;
    let beats = planned.beats;
    let script = beats
        .iter()
        .map(VideoBeat::narration)
        .collect::<Vec<_>>()
        .join(" ");
    std::fs::create_dir_all(&project_dir)?;
    std::fs::write(project_dir.join("script.txt"), &script)?;
    if let Some(plan) = planned.generated_plan {
        std::fs::write(
            project_dir.join("storyboard.json"),
            serde_json::to_vec_pretty(&plan)?,
        )?;
    }
    println!("   Script: {} chars", script.len());

    timer.begin("narration_and_broll");
    println!("🎙️  Stage 2: Synthesizing narration and importing B-roll in parallel...");
    project.state = ProjectState::AudioBuild;
    store.insert_project(&project)?;
    let data_dir = store.data_dir().to_path_buf();
    let voice_config = VoiceConfig::default();
    let kokoro_pipeline = KokoroPipeline::new("kokoro-tts", voice_config, &data_dir)
        .with_parallelism(env_parallelism("ATLAS_TTS_JOBS"));
    let segments = beats
        .iter()
        .enumerate()
        .map(|(i, beat)| NarrationSegment {
            segment_id: format!("seg_{:03}", i + 1),
            text: beat.narration().to_string(),
            voice: "af_heart".to_string(),
            speed: 1.0,
            pitch: 1.0,
            emphasis_words: Vec::new(),
            pause_after_ms: 0,
            pronunciation_hints: Vec::new(),
        })
        .collect::<Vec<_>>();
    let broll_queries = beats
        .iter()
        .map(|beat| beat.broll_query().to_string())
        .collect::<Vec<_>>();
    let (narration, broll) = tokio::join!(
        kokoro_pipeline.synthesize_track(segments.clone(), render_project_id.clone()),
        import_broll(&project.video_id, format, &data_dir, &broll_queries),
    );
    let narration = narration?;
    let combined_audio = narration
        .combined_file
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Kokoro did not return a combined narration file"))?;
    println!("   ✓ Timed narration: {}", combined_audio.display());
    println!("   ✓ {} attributed media assets available", broll.len());

    timer.begin("shot_planning");
    println!("🎬 Stage 3: Creating lip-synced shot specifications...");
    project.state = ProjectState::Storyboarding;
    store.insert_project(&project)?;

    let vectors_dir = data_dir.join("vectors").join(&project.video_id);
    std::fs::create_dir_all(&vectors_dir)?;
    let mut shot_specs = Vec::new();
    for (index, (beat, audio)) in beats.iter().zip(&narration.segments).enumerate() {
        let mut spec = create_test_shot_spec(&render_project_id, &format!("shot_{:03}", index + 1));
        spec.duration_frames = ((audio.duration_seconds + 0.4) * 30.0).ceil() as u32;
        spec.scene_template = beat.scene().to_string();
        spec.visual_elements.truncate(1);
        spec.visual_elements[0].properties["text"] = serde_json::json!(beat.title());
        spec.visual_elements[0].properties["font_size"] = serde_json::json!(20.0);
        spec.visual_elements[0].hold_frames = spec.duration_frames;
        spec.mascot = Some(MascotSpec {
            name: "Luma".into(),
            emotion: beat.emotion(),
            gesture: beat.gesture(),
            lip_sync: lip_sync_cues(audio),
            morph: beat.morph(),
        });
        if index == 0 {
            spec.background_media = broll
                .iter()
                .find(|media| matches!(media.kind, MediaKind::Image))
                .map(|media| media.file_path.clone());
        }
        if index == 3 {
            spec.background_media = broll
                .iter()
                .find(|media| matches!(media.kind, MediaKind::Video))
                .map(|media| media.file_path.clone());
        }
        if matches!(format, VideoFormat::Short) {
            spec.camera.ortho_scale = Some(10.0);
            spec.visual_elements[0].transform.position[1] = 3.2;
        } else {
            spec.visual_elements[0].transform.position[1] = 1.6;
        }
        beat.apply_visuals(&mut spec, matches!(format, VideoFormat::Short));
        for (term_idx, term) in beat.focal_terms().iter().enumerate() {
            if let Some(svg_path) = generate_vector_diagram(term, term_idx, &vectors_dir) {
                if let Some(elem) = spec.visual_elements.iter_mut().find(|el| el.element_id == format!("focus_{term_idx}")) {
                    elem.properties["svg_path"] = serde_json::json!(svg_path.to_string_lossy());
                }
            }
        }
        shot_specs.push(spec);
    }
    println!("   {} shots planned", shot_specs.len());

    let total_timeline_duration = shot_specs
        .iter()
        .map(|spec| spec.duration_frames as f64 / spec.fps as f64)
        .sum::<f64>();
    validate_format_duration(format, total_timeline_duration)
        .map_err(|error| anyhow::anyhow!("Pre-render plan QA: {error}"))?;
    println!("   ✓ Planned runtime {:.1}s passes {} QA", total_timeline_duration, profile.slug);

    timer.begin("render_and_score");
    let render_parallelism = env_parallelism("ATLAS_RENDER_JOBS");
    println!(
        "🎨 Stage 4: Rendering with Blender ({} parallel job{}) while scoring with AIWaves...",
        render_parallelism,
        if render_parallelism == 1 { "" } else { "s" }
    );
    project.state = ProjectState::AssetBuild;
    store.insert_project(&project)?;

    let blender_exe = "blender";
    let blender_pipeline = BlenderPipeline::new(blender_exe, &data_dir);

    let mut render_jobs = Vec::new();
    for spec in &shot_specs {
        let frames_dir = data_dir
            .join("renders")
            .join(&render_project_id)
            .join(&spec.shot_id);
        let reusable_frames = if reuse_rendered && frames_dir.is_dir() {
            std::fs::read_dir(&frames_dir)?
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
                .count()
        } else {
            0
        };
        if reusable_frames == spec.duration_frames as usize {
            println!("   ✓ {} {} verified cached frames", spec.shot_id, reusable_frames);
            continue;
        }
        render_jobs.push(atlas_blender::RenderJob {
            job_id: format!("job_{}", spec.shot_id),
            shot_spec: spec.clone(),
            quality: RenderQuality::Final,
            output_dir: frames_dir,
            status: atlas_blender::RenderStatus::Queued,
            output_size: Some([profile.width, profile.height]),
        });
    }

    let (render_result, (original_score, original_sfx)) = tokio::join!(
        render_shots(&blender_pipeline, render_jobs, render_parallelism),
        generate_aiwaves_audio(
            &project.video_id,
            &project.topic.title,
            &project_dir,
            &data_dir,
            total_timeline_duration,
            &shot_specs,
        ),
    );
    render_result?;

    timer.begin("assembly_and_qa");
    println!("🔧 Stage 5: Assembling final video with FFmpeg...");
    project.state = ProjectState::EditAndMix;
    store.insert_project(&project)?;

    let ffmpeg_exe = "ffmpeg";
    let ffprobe_exe = "ffprobe";
    let ffmpeg_assembler = FfmpegAssembler::new(ffmpeg_exe, ffprobe_exe, &data_dir);

    let caption_dir = data_dir
        .join("captions")
        .join(&project.video_id)
        .join(profile.slug);
    std::fs::create_dir_all(&caption_dir)?;
    let caption_path = caption_dir.join("en.srt");
    write_captions(&caption_path, &segments, &narration)?;

    let mut audio_tracks = vec![AudioTrack {
        track_id: "narration".into(),
        source_path: combined_audio.clone(),
        start_frame: 0,
        volume_db: 0.0,
        fade_in_frames: 15,
        fade_out_frames: 15,
    }];
    if let Some(score_path) = original_score {
        println!("   ✓ Original score synthesized: {}", score_path.display());
        audio_tracks.push(AudioTrack {
            track_id: "original_score".into(),
            source_path: score_path,
            start_frame: 0,
            volume_db: -20.0,
            fade_in_frames: 45,
            fade_out_frames: 60,
        });
    }
    if let Some(sfx_path) = original_sfx {
        println!("   ✓ Procedural sound effects synthesized: {}", sfx_path.display());
        audio_tracks.push(AudioTrack {
            track_id: "original_sfx".into(),
            source_path: sfx_path,
            start_frame: 0,
            volume_db: -12.0,
            fade_in_frames: 5,
            fade_out_frames: 10,
        });
    }

    let edl = EditDecisionList {
        edl_id: format!("edl_{}", uuid::Uuid::new_v4().simple()),
        project_id: project.video_id.clone(),
        fps: 30,
        width: profile.width,
        height: profile.height,
        entries: shot_specs
            .iter()
            .scan(0_u32, |record_frame, spec| {
                let index = spec
                    .shot_id
                    .trim_start_matches("shot_")
                    .parse::<u32>()
                    .unwrap_or(1)
                    - 1;
                let shot_id = format!("shot_{:03}", index + 1);
                let entry = EdlEntry {
                    entry_id: shot_id.clone(),
                    source_type: SourceType::Video,
                    source_path: data_dir
                        .join("renders")
                        .join(&render_project_id)
                        .join(&shot_id)
                        .join("%04d.png"),
                    source_in_frame: 0,
                    source_out_frame: spec.duration_frames,
                    record_in_frame: *record_frame,
                    record_out_frame: *record_frame + spec.duration_frames,
                    transition: None,
                    audio_sync_offset_ms: 0,
                };
                *record_frame += spec.duration_frames;
                Some(entry)
            })
            .collect(),
        audio_tracks,
        caption_tracks: vec![CaptionTrack {
            track_id: "captions_en".into(),
            format: CaptionFormat::Srt,
            source_path: caption_path.clone(),
            language: "en".into(),
        }],
    };

    let output_video = data_dir
        .join("exports")
        .join(&project.video_id)
        .join(profile.slug)
        .join("final.mp4");
    std::fs::create_dir_all(
        data_dir
            .join("exports")
            .join(&project.video_id)
            .join(profile.slug),
    )?;

    let assembly = ffmpeg_assembler.assemble(&edl, &output_video).await?;
    let export_dir = output_video
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid export path"))?;
    if let Some(effectcraft_mov) = generate_motion_graphic_overlay(export_dir, assembly.duration_seconds, profile.width, profile.height).await {
        println!("✨ Applying EffectCraft motion overlay: {}", effectcraft_mov.display());
        let composited = output_video.with_extension("composited.mp4");
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-i"]).arg(&output_video)
            .args(["-i"]).arg(&effectcraft_mov)
            .args(["-filter_complex", "[0:v][1:v]overlay=0:main_h-overlay_h:enable='between(t,1,5)'[v]", "-map", "[v]", "-map", "0:a?", "-c:v", "libx264", "-c:a", "copy"])
            .arg(&composited)
            .status()
            .await;
        if matches!(status, Ok(s) if s.success()) && composited.is_file() {
            let _ = std::fs::copy(&composited, &output_video);
            let _ = std::fs::remove_file(composited);
            let _ = std::fs::remove_file(effectcraft_mov);
            println!("   ✓ Motion graphic overlay composited into master video");
        }
    }
    if assembly.duration_seconds < 5.0 || assembly.frame_count < 150 {
        anyhow::bail!(
            "Release QA rejected output: {:.2}s and {} frames",
            assembly.duration_seconds,
            assembly.frame_count
        );
    }
    validate_format_duration(format, f64::from(assembly.duration_seconds))?;
    if assembly.width != profile.width || assembly.height != profile.height {
        anyhow::bail!(
            "Release QA rejected {}x{} output; expected {}x{}",
            assembly.width,
            assembly.height,
            profile.width,
            profile.height
        );
    }
    let output_size = std::fs::metadata(&output_video)?.len();
    if output_size < 100_000 {
        anyhow::bail!(
            "Release QA rejected undersized output: {} bytes",
            output_size
        );
    }
    std::fs::copy(
        data_dir
            .join("renders")
            .join(&render_project_id)
            .join("shot_001")
            .join(format!(
                "{:04}.png",
                (shot_specs[0].duration_frames / 2).max(1)
            )),
        export_dir.join("thumbnail.png"),
    )?;
    add_vector_thumbnail_title(
        &export_dir.join("thumbnail.png"),
        &project.topic.title,
        profile.width,
        profile.height,
    )
    .await;
    optimize_thumbnail(
        &export_dir.join("thumbnail.png"),
        profile.width,
        profile.height,
    )
    .await;
    let title = match format {
        VideoFormat::Long => format!("{} | {}", project.topic.title, atlas_core::COMPANY_NAME),
        VideoFormat::Short => format!("{} | #Shorts", project.topic.title),
    };
    let metadata = serde_json::json!({
        "title": title,
        "description": format!("A concise whiteboard explanation produced by {}. Narration and visuals were generated with AI-assisted production tools. Educational demonstration; sources should be reviewed before reuse.", atlas_core::COMPANY_NAME),
        "transcript": script,
        "contains_synthetic_media": true,
        "credits": [atlas_core::COMPANY_NAME, "Blender", "Kokoro TTS", "FFmpeg"],
        "broll_attribution": broll,
        "format": profile.slug,
        "width": profile.width,
        "height": profile.height,
        "duration_seconds": assembly.duration_seconds,
        "frame_count": assembly.frame_count,
        "size_bytes": output_size,
        "derived_from": if matches!(format, VideoFormat::Short) {
            Some(long_storyboard_path.to_string_lossy().to_string())
        } else {
            None
        }
    });
    std::fs::write(
        export_dir.join("youtube_metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!(
        "   ✓ Final video: {} ({:.2}s, {} frames, {} bytes)",
        output_video.display(),
        assembly.duration_seconds,
        assembly.frame_count,
        output_size
    );

    timer.begin("package");
    println!("✅ Stage 6: Quality check & package...");
    project.state = ProjectState::ReleaseQa;
    store.insert_project(&project)?;

    let timings = timer.finish();
    std::fs::write(
        export_dir.join("stage_timings.json"),
        serde_json::to_vec_pretty(&timings)?,
    )?;

    println!();
    println!("═══ Production Complete ═══");
    println!("Output: {}", output_video.display());
    println!(
        "Renders: {}",
        data_dir.join("renders").join(&project.video_id).display()
    );
    println!("Audio:   {}", combined_audio.display());
    println!("Captions: {}", caption_path.display());
    println!();
    println!("Stage timings ({:.1}s total):", timings.total_seconds);
    for stage in &timings.stages {
        let marker = if timings.slowest.as_deref() == Some(stage.stage) { "  ← slowest" } else { "" };
        println!("  {:22} {:>8.1}s{}", stage.stage, stage.seconds, marker);
    }
    println!();
    println!("Next steps:");
    println!("  atlas analytics fetch {} --sample", project.video_id);
    println!("  atlas analytics recommend {}", project.video_id);
    println!("  atlas analytics optimize {}", project.video_id);

    project.publish.contains_synthetic_media = Some(true);
    project.publish.credits_present = true;
    project.state = ProjectState::ReadyToPublish;
    store.insert_project(&project)?;
    if publish {
        cmd_publish(
            store,
            PublishOptions {
                topic_id,
                format,
                privacy,
                platform,
                tiktok_user,
                tiktok_draft,
                cleanup_after_upload: true,
            },
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn optimize_thumbnail(path: &Path, _width: u32, _height: u32) {
    if let Some(photocraft) = bundled_cli("photocraft") {
        let optimized = path.with_extension("photocraft.png");
        let status = tokio::process::Command::new(photocraft)
            .args(["convert"])
            .arg(path)
            .arg(&optimized)
            .args(["--format", "png"])
            .status()
            .await;
        if matches!(status, Ok(status) if status.success()) && optimized.is_file() {
            if let Err(error) = std::fs::copy(&optimized, path) {
                eprintln!("PhotoCraft thumbnail copy failed: {error}");
            }
            if let Err(error) = std::fs::remove_file(&optimized) {
                eprintln!("PhotoCraft temporary thumbnail cleanup failed: {error}");
            }
        }
    }
}

pub(crate) async fn add_vector_thumbnail_title(path: &Path, title: &str, width: u32, height: u32) {
    let Some(vectorcraft) = bundled_cli("vectorcraft") else {
        return;
    };
    let escaped = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let svg_path = path.with_extension("title.svg");
    let overlay_path = path.with_extension("title.png");
    let composed_path = path.with_extension("composed.png");
    let font_size = (width as f32 * 0.038).clamp(38.0, 76.0);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\"><rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" fill=\"none\"/><rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{}\" fill=\"#101522\" fill-opacity=\"0.88\"/><text x=\"{}\" y=\"{}\" fill=\"#fff4c2\" font-family=\"Arial\" font-size=\"{font_size}\" font-weight=\"700\">{escaped}</text></svg>",
        (height as f32 * 0.16) as u32,
        (width as f32 * 0.04) as u32,
        (height as f32 * 0.105) as u32,
    );
    if std::fs::write(&svg_path, svg).is_err() {
        return;
    }
    let converted = tokio::process::Command::new(vectorcraft)
        .args(["convert"])
        .arg(&svg_path)
        .arg(&overlay_path)
        .status()
        .await;
    if !matches!(converted, Ok(status) if status.success()) || !overlay_path.is_file() {
        let _ = std::fs::remove_file(&svg_path);
        return;
    }
    let composed = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(path)
        .arg("-i")
        .arg(&overlay_path)
        .args(["-filter_complex", "overlay=0:0", "-frames:v", "1"])
        .arg(&composed_path)
        .status()
        .await;
    if matches!(composed, Ok(status) if status.success()) && composed_path.is_file() {
        if let Err(error) = std::fs::copy(&composed_path, path) {
            eprintln!("VectorCraft title overlay copy failed: {error}");
        }
    }
    for temporary in [&svg_path, &overlay_path, &composed_path] {
        if let Err(error) = std::fs::remove_file(temporary)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Creative CLI temporary cleanup failed: {error}");
        }
    }
}

pub(crate) async fn generate_motion_graphic_overlay(
    output_dir: &Path,
    _duration_seconds: f32,
    _width: u32,
    _height: u32,
) -> Option<PathBuf> {
    let overlay_path = output_dir.join("effectcraft_overlay.mov");
    if overlay_path.is_file() {
        return Some(overlay_path);
    }
    let effectcraft = bundled_cli("effectcraft")?;
    let status = tokio::process::Command::new(effectcraft)
        .arg("--demo")
        .arg("render")
        .args(["--comp", "Lower Third", "--format", "prores", "--channels", "rgba", "--start", "0", "--end", "2", "--out"])
        .arg(&overlay_path)
        .status()
        .await;
    if matches!(status, Ok(s) if s.success()) && overlay_path.is_file() {
        Some(overlay_path)
    } else {
        None
    }
}
