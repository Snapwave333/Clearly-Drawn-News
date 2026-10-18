use super::*;

pub(crate) struct PublishOptions<'a> {
    pub(crate) topic_id: &'a str,
    pub(crate) format: VideoFormat,
    pub(crate) privacy: PublishPrivacy,
    pub(crate) platform: PublishPlatformCli,
    pub(crate) tiktok_user: &'a str,
    pub(crate) tiktok_draft: bool,
    pub(crate) cleanup_after_upload: bool,
}

#[derive(Deserialize)]
pub(crate) struct VideoPackageMetadata {
    pub(crate) title: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) transcript: String,
    pub(crate) contains_synthetic_media: bool,
    pub(crate) duration_seconds: f64,
    #[serde(default)]
    pub(crate) broll_attribution: Vec<ImportedMedia>,
}

pub(crate) async fn cmd_publish(store: &Store, options: PublishOptions<'_>) -> anyhow::Result<()> {
    let PublishOptions {
        topic_id,
        format,
        privacy,
        platform,
        tiktok_user,
        tiktok_draft,
        cleanup_after_upload,
    } = options;
    let mut project = store
        .get_project(topic_id)?
        .ok_or_else(|| anyhow::anyhow!("project not found: {}", topic_id))?;
    let profile = format.profile();
    let export_dir = store
        .data_dir()
        .join("exports")
        .join(&project.video_id)
        .join(profile.slug);
    let video_path = export_dir.join("final.mp4");
    let thumbnail_path = export_dir.join("thumbnail.png");
    let caption_path = if export_dir.join("captions_en.srt").is_file() {
        export_dir.join("captions_en.srt")
    } else {
        export_dir.join("captions.srt")
    };
    if !video_path.is_file() {
        anyhow::bail!("final video not found: {}", video_path.display());
    }

    let publish_youtube = matches!(
        platform,
        PublishPlatformCli::YouTube | PublishPlatformCli::Both
    );
    let publish_tiktok = matches!(
        platform,
        PublishPlatformCli::TikTok | PublishPlatformCli::Both
    );

    if publish_youtube {
        for path in [&thumbnail_path, &caption_path] {
            if !path.is_file() {
                anyhow::bail!("publish package is incomplete: {}", path.display());
            }
        }
    }

    let package: VideoPackageMetadata =
        serde_json::from_slice(&std::fs::read(export_dir.join("youtube_metadata.json"))?)?;
    let ledger_path = store.data_dir().join("publish").join("ledger.json");
    let mut ledger = PublishLedger::load(&ledger_path)?;
    let fingerprint = media_fingerprint(&video_path)?;
    let published_format = match format {
        VideoFormat::Long => PublishedFormat::Long,
        VideoFormat::Short => PublishedFormat::Short,
    };

    if publish_youtube {
        let already_uploaded = project.publish.youtube_video_ids.contains_key(profile.slug);
        let publish_decision = if matches!(privacy, PublishPrivacy::Public) && !already_uploaded {
            ledger.decide(
                &PublishCadencePolicy::default(),
                published_format,
                &fingerprint,
                Utc::now(),
            )
        } else {
            PublishDecision::Allow
        };
        let unique_media = already_uploaded
            || !matches!(&publish_decision, PublishDecision::RejectDuplicate { .. });
        let credentials = load_youtube_credentials("youtube_credentials.json").await?;
        let client = YouTubeAnalyticsClient::new(credentials);
        if !client.is_authenticated()? {
            anyhow::bail!("YouTube is not authenticated; run `atlas analytics auth` first");
        }
        let channel = client.fetch_channel_status().await?;
        let broll_expected = !package.broll_attribution.is_empty();
        let broll_attribution_present = package.broll_attribution.iter().all(|media| {
            !media.provider.trim().is_empty()
                && !media.source_url.trim().is_empty()
                && !media.creator.trim().is_empty()
        });
        let guardian = PolicyGuardian::new(store.data_dir().join("policy"))?;
        let browser_review = guardian.channel_review()?;
        let report = guardian
            .vet(
                &ContentPreflight {
                    title: package.title.clone(),
                    description: package.description.clone(),
                    transcript: package.transcript.clone(),
                    contains_synthetic_media: package.contains_synthetic_media,
                    captions_present: caption_path.is_file(),
                    thumbnail_present: thumbnail_path.is_file(),
                    broll_expected,
                    broll_attribution_present,
                    unique_media,
                },
                &ChannelPreflight {
                    channel_id: channel.channel_id,
                    available: channel.available,
                    linked: channel.is_linked,
                    long_uploads_status: channel.long_uploads_status,
                    privacy_status: channel.privacy_status,
                    browser_review,
                    require_browser_review: true,
                },
                &format!("{}_{}", project.video_id, profile.slug),
            )
            .await?;
        println!(
            "Policy Guardian (YouTube): {}/{} checks passed",
            report
                .checks
                .iter()
                .filter(|check| check.status == CheckStatus::Pass)
                .count(),
            report.checks.len()
        );
        if !report.publish_allowed {
            project.state = ProjectState::ReadyToPublish;
            store.insert_project(&project)?;
            let blocked = report
                .checks
                .iter()
                .filter(|check| check.status != CheckStatus::Pass)
                .map(|check| format!("{}: {}", check.name, check.detail))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("Policy Guardian blocked publication: {blocked}");
        }
        match publish_decision {
            PublishDecision::Allow => {}
            PublishDecision::Defer { retry_at, reason } => {
                project.state = ProjectState::ReadyToPublish;
                store.insert_project(&project)?;
                println!("Public YouTube upload deferred until {retry_at}: {reason}");
                return Ok(());
            }
            PublishDecision::RejectDuplicate { existing_video_id } => {
                anyhow::bail!(
                    "Publication rejected: identical media already exists as YouTube video {existing_video_id}"
                );
            }
        }

        let upload_metadata = YouTubeUploadMetadata {
            title: package.title.clone(),
            description: package.description.clone(),
            tags: vec![
                "education".into(),
                "science".into(),
                "whiteboard animation".into(),
                profile.slug.into(),
            ],
            privacy_status: privacy.as_youtube_value().into(),
            category_id: "27".into(),
            contains_synthetic_media: package.contains_synthetic_media,
        };

        project.state = ProjectState::Uploading;
        store.insert_project(&project)?;
        let result = if let Some(existing_id) = project.publish.youtube_video_ids.get(profile.slug)
        {
            atlas_analytics::YouTubeUploadResult {
                video_id: existing_id.clone(),
                url: format!("https://youtu.be/{existing_id}"),
            }
        } else {
            match client.upload_video(&video_path, &upload_metadata).await {
                Ok(result) => result,
                Err(error) => {
                    project.state = ProjectState::ReadyToPublish;
                    store.insert_project(&project)?;
                    return Err(error.into());
                }
            }
        };

        project.publish.youtube_video_id = Some(result.video_id.clone());
        project
            .publish
            .youtube_video_ids
            .insert(profile.slug.into(), result.video_id.clone());
        project.state = ProjectState::UploadedProcessing;
        store.insert_project(&project)?;
        client
            .upload_thumbnail(&result.video_id, &thumbnail_path)
            .await?;
        if let Err(caption_err) = client
            .upload_captions(&result.video_id, &caption_path, "en-US")
            .await
        {
            eprintln!("Warning: caption upload failed ({caption_err}), proceeding with published video.");
        }
        project.state = ProjectState::Published;
        store.insert_project(&project)?;
        if matches!(privacy, PublishPrivacy::Public) {
            ledger.record(PublishedItem {
                project_id: project.video_id.clone(),
                youtube_video_id: result.video_id.clone(),
                format: published_format,
                published_at: Utc::now(),
                content_fingerprint: fingerprint.clone(),
                platform: "youtube".into(),
                tiktok_post_id: None,
            });
            ledger.save(&ledger_path)?;
        }
        println!(
            "Published {} video to YouTube: {}",
            profile.slug, result.url
        );
    }

    if publish_tiktok {
        println!(
            "Configuring TikTok publishing for @{} via Upload-Post...",
            tiktok_user
        );
        let tiktok_creds = load_tiktok_credentials("upload_post_credentials.json").await?;
        let tiktok_client = TikTokClient::new(tiktok_creds);

        let mut tiktok_already_uploaded =
            project.publish.tiktok_post_ids.contains_key(profile.slug);
        if !tiktok_already_uploaded
            && let Some(request_id) = project.publish.tiktok_request_id.clone()
        {
            let pending = tiktok_client.get_upload_status(&request_id).await?;
            if pending.status == "completed" && pending.success {
                if let Some(post_id) = pending.post_id.clone() {
                    project.publish.tiktok_post_id = Some(post_id.clone());
                    project
                        .publish
                        .tiktok_post_ids
                        .insert(profile.slug.into(), post_id.clone());
                    project.publish.tiktok_request_id = None;
                    project.state = ProjectState::Published;
                    store.insert_project(&project)?;
                    tiktok_already_uploaded = true;
                } else {
                    println!(
                        "TikTok completed request {request_id} without a post id; retaining media for verification."
                    );
                    return Ok(());
                }
            } else if pending.status == "failed" || !pending.success {
                project.publish.tiktok_request_id = None;
                project.state = ProjectState::ReadyToPublish;
                store.insert_project(&project)?;
                anyhow::bail!(
                    "TikTok request {request_id} failed: {}",
                    pending
                        .error_message
                        .unwrap_or_else(|| "unknown TikTok processing error".into())
                );
            } else {
                println!(
                    "TikTok request {request_id} is still processing; retaining media for retry."
                );
                return Ok(());
            }
        }
        let tiktok_decision = if matches!(privacy, PublishPrivacy::Public)
            && !tiktok_draft
            && !tiktok_already_uploaded
        {
            ledger.decide_tiktok(&PublishCadencePolicy::default(), &fingerprint, Utc::now())
        } else {
            PublishDecision::Allow
        };
        let tiktok_unique = !matches!(&tiktok_decision, PublishDecision::RejectDuplicate { .. });

        match tiktok_decision {
            PublishDecision::Allow => {}
            PublishDecision::Defer { retry_at, reason } => {
                project.state = ProjectState::ReadyToPublish;
                store.insert_project(&project)?;
                println!("Public TikTok upload deferred until {retry_at}: {reason}");
                return Ok(());
            }
            PublishDecision::RejectDuplicate { existing_video_id } => {
                anyhow::bail!(
                    "TikTok publication rejected: identical media already exists as TikTok post {existing_video_id}"
                );
            }
        }

        let tiktok_post_mode = if tiktok_draft {
            TikTokPostMode::MediaUpload
        } else {
            TikTokPostMode::DirectPost
        };
        let tiktok_privacy_level = if tiktok_draft {
            TikTokPrivacyLevel::SelfOnly
        } else {
            privacy.as_tiktok_value()
        };

        let caption = format!(
            "{} #education #science #whiteboardanimation #focogaming",
            package.title
        );
        let general_checks = content_checks(&ContentPreflight {
            title: package.title.clone(),
            description: package.description.clone(),
            transcript: package.transcript.clone(),
            contains_synthetic_media: package.contains_synthetic_media,
            captions_present: caption_path.is_file(),
            thumbnail_present: thumbnail_path.is_file(),
            broll_expected: !package.broll_attribution.is_empty(),
            broll_attribution_present: package.broll_attribution.iter().all(|media| {
                !media.provider.trim().is_empty()
                    && !media.source_url.trim().is_empty()
                    && !media.creator.trim().is_empty()
            }),
            unique_media: tiktok_unique,
        });
        let community_guidelines_compliant = general_checks
            .iter()
            .all(|check| check.status == CheckStatus::Pass);
        let tiktok_checks = tiktok_content_checks(&TikTokPreflight {
            title: package.title.clone(),
            description: caption.clone(),
            duration_seconds: package.duration_seconds,
            is_original: true,
            contains_synthetic_media: package.contains_synthetic_media,
            aigc_label_enabled: true,
            commercial_music_cleared: true,
            community_guidelines_compliant,
            account_warning_visible: false,
            account_strike_visible: false,
        });
        let tiktok_blocked = tiktok_checks.iter().any(|check| {
            check.status != CheckStatus::Pass && check.name != "tiktok_monetization_duration"
        });
        println!(
            "Policy Guardian (TikTok): {}/{} required checks passed",
            tiktok_checks
                .iter()
                .filter(|check| check.status == CheckStatus::Pass)
                .count(),
            tiktok_checks.len()
        );
        if tiktok_blocked {
            let blocked = tiktok_checks
                .iter()
                .filter(|check| {
                    check.status != CheckStatus::Pass
                        && check.name != "tiktok_monetization_duration"
                })
                .map(|check| format!("{}: {}", check.name, check.detail))
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("Policy Guardian blocked TikTok publication: {blocked}");
        }
        let tiktok_metadata = TikTokUploadMetadata {
            title: caption,
            user: tiktok_user.to_string(),
            privacy_level: tiktok_privacy_level,
            post_mode: tiktok_post_mode,
            is_ai_generated: true, // Transparent AI disclosure for whiteboard animations
            disable_duet: false,
            disable_stitch: false,
            disable_comment: false,
        };

        project.state = ProjectState::Uploading;
        store.insert_project(&project)?;

        println!(
            "Uploading {} video to TikTok (@{}) via Upload-Post...",
            profile.slug, tiktok_user
        );
        let status = if tiktok_already_uploaded {
            TikTokUploadStatus {
                request_id: String::new(),
                status: "completed".into(),
                completed: 1,
                total: 1,
                success: true,
                post_id: project.publish.tiktok_post_ids.get(profile.slug).cloned(),
                url: None,
                error_message: None,
            }
        } else {
            let res = match tiktok_client
                .upload_video(&video_path, &tiktok_metadata)
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    project.state = ProjectState::ReadyToPublish;
                    store.insert_project(&project)?;
                    return Err(e.into());
                }
            };

            println!("TikTok upload accepted with request_id: {}", res.request_id);
            project.publish.tiktok_request_id = Some(res.request_id.clone());
            project.state = ProjectState::UploadedProcessing;
            store.insert_project(&project)?;

            println!("Polling TikTok processing status...");
            match tiktok_client
                .poll_until_complete(&res.request_id, std::time::Duration::from_secs(45))
                .await
            {
                Ok(status) => status,
                Err(_) => {
                    println!(
                        "Upload continues in the background. Track it with: atlas tiktok status --request-id {}",
                        res.request_id
                    );
                    return Ok(());
                }
            }
        };

        if !status.success {
            project.publish.tiktok_request_id = None;
            project.state = ProjectState::ReadyToPublish;
            store.insert_project(&project)?;
            anyhow::bail!(
                "TikTok processing failed: {}",
                status
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "unknown TikTok processing error".into())
            );
        }

        if let Some(post_id) = &status.post_id {
            project.publish.tiktok_post_id = Some(post_id.clone());
            project
                .publish
                .tiktok_post_ids
                .insert(profile.slug.into(), post_id.clone());
            project.publish.tiktok_request_id = None;
        }
        project.state = ProjectState::Published;
        store.insert_project(&project)?;

        if matches!(privacy, PublishPrivacy::Public)
            && !tiktok_draft
            && !ledger
                .items
                .iter()
                .any(|item| item.platform == "tiktok" && item.content_fingerprint == fingerprint)
        {
            ledger.record(PublishedItem {
                project_id: project.video_id.clone(),
                youtube_video_id: String::new(),
                format: published_format,
                published_at: Utc::now(),
                content_fingerprint: fingerprint.clone(),
                platform: "tiktok".into(),
                tiktok_post_id: status.post_id.clone(),
            });
            ledger.save(&ledger_path)?;
        }

        let post_url = status
            .url
            .unwrap_or_else(|| format!("https://www.tiktok.com/@{}", tiktok_user));
        println!(
            "Published {} video to TikTok (@{}): {}",
            profile.slug, tiktok_user, post_url
        );
    }

    if !publish_youtube && !publish_tiktok {
        println!("No platform selected for publication.");
    } else if cleanup_after_upload {
        cleanup_uploaded_media([video_path.as_path()]);
        if publish_youtube && thumbnail_path.is_file() {
            cleanup_uploaded_media([thumbnail_path.as_path()]);
        }
    }

    Ok(())
}

pub(crate) async fn cmd_tiktok(store: &Store, action: TikTokAction) -> anyhow::Result<()> {
    match action {
        TikTokAction::Info { user } => {
            let creds = load_tiktok_credentials("upload_post_credentials.json").await?;
            let client = TikTokClient::new(creds);
            println!("Probing TikTok profile connection for @{user}...");
            let info = client.fetch_user_status(&user).await?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        TikTokAction::Status { request_id } => {
            let creds = load_tiktok_credentials("upload_post_credentials.json").await?;
            let client = TikTokClient::new(creds);
            println!("Checking TikTok upload status for request {request_id}...");
            let status = client.get_upload_status(&request_id).await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        TikTokAction::Upload {
            topic,
            format,
            privacy,
            user,
            draft,
        } => {
            cmd_publish(
                store,
                PublishOptions {
                    topic_id: &topic,
                    format,
                    privacy,
                    platform: PublishPlatformCli::TikTok,
                    tiktok_user: &user,
                    tiktok_draft: draft,
                    cleanup_after_upload: true,
                },
            )
            .await?;
        }
        TikTokAction::Analytics { video_id, sample } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = if sample {
                let m = atlas_analytics::save_sample_short_metrics(store, &video_id, "tiktok")?;
                println!("Stored TikTok analytics snapshot for {}", video_id);
                m
            } else {
                analytics.load_platform_metrics(&video_id, "tiktok")?.ok_or_else(|| {
                    anyhow::anyhow!("no TikTok metrics found for {}. Run with --sample to generate or populate snapshot.", video_id)
                })?
            };
            println!(
                "TikTok Analytics & Retention — @focogaming ({})",
                metrics.video_id
            );
            println!("==================================================");
            println!("Views:        {}", metrics.views.unwrap_or(0));
            println!(
                "Avg Duration: {:.1}s ({:.1}% of video)",
                metrics.average_view_duration_seconds.unwrap_or(0.0),
                metrics.average_percentage_viewed.unwrap_or(0.0) * 100.0
            );
            println!(
                "Likes:        {} | Comments: {} | Shares: {}",
                metrics.likes.unwrap_or(0),
                metrics.comments.unwrap_or(0),
                metrics.shares.unwrap_or(0)
            );
            if let Some(hook) = metrics.short_hook_retention() {
                println!("3-Sec Hook:   {:.1}% (target: >= 70%)", hook * 100.0);
            }
            if let Some(comp) = metrics.completion_rate() {
                println!("Completion:   {:.1}% (target: >= 35%)", comp * 100.0);
            }
            println!("\nAudience Retention Curve:");
            println!("{:<8} {:<12} {:<12}", "Time", "Elapsed", "Retention");
            for p in &metrics.retention_curve {
                println!(
                    "{:02}:{:02}    {:>5.1}%       {:>5.1}%",
                    p.elapsed_seconds / 60,
                    p.elapsed_seconds % 60,
                    p.elapsed_ratio * 100.0,
                    p.retention_ratio * 100.0
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn cleanup_uploaded_media<'a>(paths: impl IntoIterator<Item = &'a Path>) {
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => println!("Removed uploaded local media: {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "Uploaded successfully, but local cleanup failed for {}: {}",
                path.display(),
                error
            ),
        }
    }
}
