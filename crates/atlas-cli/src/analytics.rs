use super::*;

pub(crate) async fn cmd_analytics(store: &Store, action: AnalyticsAction) -> anyhow::Result<()> {
    match action {
        AnalyticsAction::Auth {
            client_id,
            client_secret,
            redirect_uri,
        } => {
            // Load from credentials file if not provided
            let (client_id, client_secret, redirect_uri) = match (
                client_id,
                client_secret,
                redirect_uri,
            ) {
                (Some(cid), Some(csec), ruri) => (
                    cid,
                    csec,
                    ruri.unwrap_or_else(|| "http://localhost".to_string()),
                ),
                _ => {
                    let creds_file = PathBuf::from("youtube_credentials.json");
                    if creds_file.exists() {
                        let file_creds = load_youtube_credentials(&creds_file).await?;
                        let creds: YouTubeCredentials = file_creds;
                        (creds.client_id, creds.client_secret, creds.redirect_uri)
                    } else {
                        anyhow::bail!(
                            "YouTube credentials not provided and youtube_credentials.json not found"
                        );
                    }
                }
            };

            let creds = YouTubeCredentials {
                client_id,
                client_secret,
                redirect_uri,
            };
            let url = authorization_url(&creds);
            println!("Visit this URL in a browser signed into your YouTube channel:");
            println!("{}", url);
            println!();
            println!("After authorization, run:");
            println!("  atlas analytics auth-complete --code <CODE>");
        }
        AnalyticsAction::AuthComplete {
            client_id,
            client_secret,
            code,
            redirect_uri,
        } => {
            // Load from credentials file if not provided
            let (client_id, client_secret, redirect_uri) = match (
                client_id,
                client_secret,
                redirect_uri,
            ) {
                (Some(cid), Some(csec), ruri) => (
                    cid,
                    csec,
                    ruri.unwrap_or_else(|| "http://localhost".to_string()),
                ),
                _ => {
                    let creds_file = PathBuf::from("youtube_credentials.json");
                    if creds_file.exists() {
                        let file_creds = load_youtube_credentials(&creds_file).await?;
                        let creds: YouTubeCredentials = file_creds;
                        (creds.client_id, creds.client_secret, creds.redirect_uri)
                    } else {
                        anyhow::bail!(
                            "YouTube credentials not provided and youtube_credentials.json not found"
                        );
                    }
                }
            };

            let creds = YouTubeCredentials {
                client_id,
                client_secret,
                redirect_uri,
            };
            let client = YouTubeAnalyticsClient::new(creds);
            let token = client.exchange_code(&code).await?;
            println!(
                "YouTube OAuth complete. Access token expires at {:?}",
                token.expires_at
            );
            println!("Refresh token stored securely.");
        }
        AnalyticsAction::Fetch {
            video_id,
            sample,
            platform,
            format,
        } => {
            let project = store
                .get_project(&video_id)?
                .ok_or_else(|| anyhow::anyhow!("project not found: {}", video_id))?;
            let youtube_id = project
                .publish
                .youtube_video_id
                .clone()
                .unwrap_or_else(|| "sample_video_id".to_string());

            if sample {
                let metrics = if format.eq_ignore_ascii_case("short")
                    || platform.eq_ignore_ascii_case("tiktok")
                {
                    atlas_analytics::save_sample_short_metrics(store, &video_id, &platform)?
                } else {
                    save_sample_metrics(store, &video_id, &youtube_id)?
                };
                println!(
                    "Saved sample metrics for {} on {} ({})",
                    video_id, platform, format
                );
                println!("{}", serde_json::to_string_pretty(&metrics)?);
            } else {
                println!(
                    "Live {} Analytics fetch requires authenticated credentials.",
                    platform
                );
                println!(
                    "Use --sample to store a demo snapshot, or run `atlas analytics auth` first."
                );
            }
        }
        AnalyticsAction::Retention { video_id, platform } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics
                .load_platform_metrics(&video_id, &platform)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no metrics found for {} on {}. Run `atlas analytics fetch {} --sample --platform {}` first.",
                        video_id,
                        platform,
                        video_id,
                        platform
                    )
                })?;
            println!(
                "Retention curve for {} [{} | {}]:",
                video_id, metrics.platform, metrics.format
            );
            println!("{:<8} {:<12} {:<12}", "Time", "Elapsed", "Retention");
            for p in &metrics.retention_curve {
                let mm = p.elapsed_seconds / 60;
                let ss = p.elapsed_seconds % 60;
                println!(
                    "{:02}:{:02}    {:>5.1}%       {:>5.1}%",
                    mm,
                    ss,
                    p.elapsed_ratio * 100.0,
                    p.retention_ratio * 100.0
                );
            }
            if let Some(hook) = metrics.short_hook_retention() {
                println!("\n0-3s Short Hook Rate: {:.1}%", hook * 100.0);
            }
            if let Some(comp) = metrics.completion_rate() {
                println!("Full Video Completion: {:.1}%", comp * 100.0);
            }
        }
        AnalyticsAction::Recommend {
            video_id,
            duration_seconds,
            platform,
        } => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics
                .load_platform_metrics(&video_id, &platform)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no metrics found for {} on {}. Run `atlas analytics fetch {} --sample --platform {}` first.",
                        video_id,
                        platform,
                        video_id,
                        platform
                    )
                })?;

            // Use all stored metrics as the channel baseline.
            let all = analytics.list_metrics(100)?;
            let baseline = compute_channel_baseline(&all);
            let performance = compare_to_baseline(&metrics, &baseline);

            let engine = RecommendationEngine::new();
            let recs = engine.rank(engine.analyze(&performance, duration_seconds));

            println!(
                "Optimization recommendations for {} ({}):",
                video_id, platform
            );
            for r in &recs {
                println!("[{:?}] {:?} — {}", r.priority, r.category, r.issue);
                println!("  Evidence: {}", r.evidence);
                println!("  Action:   {}", r.suggested_action);
                if let Some(tc) = &r.affected_timecodes {
                    println!("  Timecodes: {}", tc.join(", "));
                }
                println!();
            }
            if recs.is_empty() {
                println!("No major issues detected. Video is performing at or above baseline.");
            }
        }
        AnalyticsAction::Optimize {
            video_id,
            duration_seconds,
        } => {
            let loop_ = OptimizationLoop::new();
            let ids = loop_.generate_revision_tasks(store, &video_id, duration_seconds)?;
            if ids.is_empty() {
                println!("No optimization tasks generated; no metrics or no issues detected.");
            } else {
                println!(
                    "Generated {} optimization task(s) from analytics:",
                    ids.len()
                );
                for id in ids {
                    println!("  - {}", id);
                }
            }
        }
        AnalyticsAction::Dashboard => {
            let analytics = AnalyticsStore::new(store);
            let metrics = analytics.list_metrics(50)?;
            let baseline = compute_channel_baseline(&metrics);
            println!("{} — Analytics Dashboard", atlas_core::COMPANY_NAME);
            println!("==================================");
            println!("Videos tracked: {}", metrics.len());
            println!(
                "Channel baseline — avg view duration: {:.1}s, avg % viewed: {:.1}%, avg CTR: {:.2}%",
                baseline.average_view_duration_seconds,
                baseline.average_percentage_viewed * 100.0,
                baseline.average_ctr * 100.0
            );
            println!();
            println!(
                "{:<24} {:<16} {:<12} {:<12} {:<12}",
                "VIDEO", "YOUTUBE ID", "VIEWS", "AVG DUR", "CTR"
            );
            for m in metrics {
                println!(
                    "{:<24} {:<16} {:<12} {:<12} {:<12}",
                    m.video_id,
                    m.youtube_video_id,
                    m.views.map(|v| v.to_string()).unwrap_or_default(),
                    m.average_view_duration_seconds
                        .map(|d| format!("{:.1}s", d))
                        .unwrap_or_default(),
                    m.impressions_click_through_rate
                        .map(|c| format!("{:.2}%", c * 100.0))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}
