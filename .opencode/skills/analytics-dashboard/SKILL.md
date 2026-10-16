---
name: atlas-analytics-dashboard
description: Render first-party channel analytics, real retention curves and video-annotated timeline reports with exact provenance and unsupported-data states.
version: 1.0.0
role: analytics_dashboard
requires: [youtube_oauth, channel_metric_reports, retention_samples, scene_timeline]
tools: [youtube_analytics_api, angular, rust_sqlite]
---

# YouTube Analytics Control Room

## Mission
Show a precise picture of performance for the **authorized channel only**, and trace genuine observed performance into story/production improvements. This screen must never simulate or fabricate YouTube metrics. Distinguish observed data from internal editorial judgments and planned experiments.

## Inputs
- YouTube Analytics official reports: per-video retention (`elapsedVideoTimeRatio`, `audienceWatchRatio`, `relativeRetentionPerformance`); daily and video views, `estimatedMinutesWatched`, `averageViewDuration`, `averageViewPercentage`, `subscribersGained`, `subscribersLost`, shares/likes when report compatible; traffic sources where supported.
- Exact published master hash, video duration, scene/beat boundaries, narrated lines, feature metadata, video age, observation range, last refreshed time, permissions.
- Historical comparisons where age, format, duration, traffic source and upload cohorts are made explicit.

## Dashboard specification
1. **Overview:** genuine YouTube views, minutes watched, average view duration, net subscribers from available official metrics, daily time series, fresh versus stale indicators. Never replace these metrics with custom opaque scores.
2. **Episode library:** video cards grouped by verified topic family and publishing period. Filter by date, runtime, format, source type and approved experimental variant. Video metadata is refreshed/deleted under API policies.
3. **Retention Lab:** plot absolute watch ratio and relative retention separately, with 100 normalized points when available. Allow rewatches >100% watch ratio; show missing-data diagnostics. Overlay colored scene bands, a playback scrubber and thumbnails sampled from the user's actual film. Highlight transitions by linked scene ID, not invented viewer emotion.
4. **Moment Inspector:** clicking a curve segment opens the exact exported frame interval and script/narration line, sampled raw metrics, time bucket bounds, candidate causes, source provenance and limitations. Useful controls: Mark confusing, Mark effective, Ignore signal, Convert to experiment.
5. **Traffic & cohorts:** show authorized traffic-source report shapes where available and dates/format context before claiming a scene change mattered. Do not invent YouTube Search CTR or impression CTR when unavailable through authorized official API.
6. **Experiments:** hypotheses and single-variable trials with preregistered criteria, guardrail checks, approval state, editor diff, canary exposure and rollback pointer. Show `insufficient evidence` when experiments cannot be evaluated credibly.
7. **Production correlation:** visualize *internal production variables* (shot duration, narration rate, diagram complexity, reveal method, art style) alongside clearly labeled YouTube analytics without merging them into an unapproved YouTube-derived channel score.
8. **Audit trail:** model/tool/adapter version, last refresh, data maturity, missing permission, JSON report provenance and promotion/reversion audit.

## Constraints
- User's YouTube OAuth grant is required for their retention data. The NVIDIA/OpenCode key does not authorize channel data.
- Follow YouTube API policies on refreshing/deleting authorized data and additional permissions needed for complex derived performance metrics.
- Use official YouTube Studio for native thumbnail tests, not invented public API integrations.
- No misleading charts, extrapolated views, fake real-time retention or hidden impressions metrics.
- Default accessibility: keyboard navigation, toggles for high contrast, light/dark, data tables for chart data, reduced motion.

## Outputs / accept conditions
Produce `analytics_dashboard_view.json` with report provenance, precise values and labels, a retention scene-join file, a list of data gaps, and a read-only API surface to the Angular client. Require a second data steward to confirm metric labels and access permissions. If the report is absent, show `not available` with reason rather than zeros.

## Failure recovery
Missing OAuth -> self-contained connect-channel state. Quota or 403 -> show cached report with timestamp and no fake freshness. Missing timeline -> show curve only, disable moment attribution. Renderer failure -> give accessible numeric table.
