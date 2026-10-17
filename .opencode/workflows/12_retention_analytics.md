# Multi-Platform Retention Analytics & Audience Optimization

## Objective
Ingests first-party audience retention, watch time, and engagement curves across YouTube and TikTok (@focogaming) to discover pacing drop-offs, hook strength, and produce algorithmic optimization tasks for the production team.

## Metrics Architecture

### 1. Retention Curve Sampling
- **Normalized Elapsed Ratio ($0.0 \to 1.0$):** Sampled along 100 points or key time intervals.
- **Hook Analysis:**
  - **Long-form (YouTube):** First 30 seconds retention against channel baseline.
  - **Short-form (Shorts / TikTok):** First 3 seconds retention against target threshold ($\ge 70.0\%$).
- **Drop-off Cliff Detection:** Identifies any contiguous point interval with $\Delta \text{retention} > 15\%$, mapping to the exact timecode and script scene.
- **Completion & Loop Rate:** Retention at $100\%$ video duration (target $\ge 35.0\%$ for virality on Shorts & TikTok).

### 2. Multi-Platform Schema (`platform_metrics`)
- Durably stored in SQLite database (`atlas.db`):
  - `youtube_metrics`: YouTube long-form and Shorts analytics.
  - `platform_metrics`: Multi-platform store partitioned by `(video_id, platform)`.
  - `retention_curves`: High-resolution retention samples with timestamps.

## CLI Commands

```bash
# Ingest or simulate live metrics snapshot
atlas analytics fetch <video_id> --sample --platform youtube --format long
atlas analytics fetch <video_id> --sample --platform tiktok --format short

# Display audience retention curve, hook rate, and completion
atlas analytics retention <video_id> --platform youtube
atlas analytics retention <video_id> --platform tiktok

# Dedicated TikTok channel command
atlas tiktok analytics --video-id <video_id> --sample

# Generate automated recommendations from retention cliffs
atlas analytics recommend <video_id> --platform tiktok --duration-seconds 30

# Channel-wide analytics dashboard
atlas analytics dashboard
```

## Agent Collaboration Bus
1. **`analytics_engineer`:** Fetches consented metrics and records normalized `VideoMetrics` into `AnalyticsStore`.
2. **`analytics_dashboard`:** Displays retention curves, cliff heatmaps, and baseline comparisons.
3. **`thumbnail_optimizer`:** Uses CTR vs channel baseline to trigger thumbnail revisions and A/B experiments.
4. **`motion_director` / `lead_writer`:** Receives revision tasks for pacing cliffs identified in retention analysis.
