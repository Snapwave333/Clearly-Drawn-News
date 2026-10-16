# ATLAS STUDIO Analytics Dashboard UX — Functional Design

## Layout
Desktop Tauri + Angular app: persistent navigation `Overview | Episodes | Retention Lab | Trends | Experiments | Render Queue | Publishing | Settings`; main central work surface; secondary inspector; compact bottom worker/event console. Light/dark systems with consistent typography, visual hierarchy, and data table fallbacks. The analytics dashboard must look like a rigorous editorial tool, not a casino/gamified metrics panel.

## Overview
- Time range selector: 7 / 28 / 90 days / authorized custom range.
- Genuine owner-authorized metrics: video views, watch minutes, average view duration, average view percentage, net subscribers from gains/losses if source supports, likes/shares when accessible.
- Time-series comparisons label time windows and report fetch times. Defer profitability claims unless monetization reports are authorized.

## Retention Lab interaction
- Episode dropdown with publication date, length, duration, privacy state, data maturity and last data refresh.
- Two distinct chart modes: absolute audience watch ratio and relative retention performance. X-axis normalized elapsed video position from official API. Pixel-perfect player scrub to the corresponding bucket's range, with uncertainty visualized.
- On curve hover: actual `elapsedVideoTimeRatio`, exact bucket end, audienceWatchRatio (allow > 1), relativeRetentionPerformance, nearby scene IDs and data source.
- Scene-band overlay: `Hook`, `Context`, `Demonstration`, `Evidence`, `Resolution`, `Credits`; labels derived from actual published metadata, not inferred viewer identities.
- Detail panel: scene clip/frame(s) preview, source claim list, narration quote, geometric object density, actual production treatment, inspector comments and revision proposal draft.
- Action bar: `Propose experiment`, `Dismiss`, `Open source proof`, `Inspect storyboard`. These create **editorial tickets**, not direct public-video edits.
- No data: explicit choices `Processing`, `No rows returned`, `OAuth required`, `Insufficient data`, `Source mismatch`, `API error`.

## Experiments
- Track proposal, preregistered variable, existing baseline, proposed variant, comparable episode IDs, maturity date, independent reviewer, confidence status, decision, rollback link.
- Before/after plots are explicitly observation-only unless valid experiment/randomization exists.
- Never display invented proprietary "viral score", "viewer happiness", "hook genius" or Studio-only CTR as fetched measurements.

## Channel-level analysis
- Track topic portfolio and production output (first-party application data), official metric time series, and interpret differences without merging data into restricted derived channel scoring.
- Policy checks: owner-auth refresh, API field support, API quotas, data deletion/refresh. Connect via OAuth separately from OpenCode NIM account.

## Technical transport
Backend websocket notifications: `analytics.fetch_started`, `analytics.fetch_failed`, `analytics.video_updated`, `analytics.retention_ready`, `editorial.review_ready`, `experiment.decision_ready`. Send a view-model containing exact raw report source refs and analysis annotations. No direct secret/OAuth token in Angular client.

## Test cases
- Valid 100-sample retention row set and matching scene timeline.
- Ratio >1 retains correct plot scale and tooltip.
- Less than two points gives data unavailable rather than 0.
- Missing `relativeRetentionPerformance` still renders supported absolute metric.
- Video ID mismatch blocks attribution and audit logs reason.
- Changing channel invalidates cache authorization.
- Light/dark and reduced-motion UI remains readable and keyboard accessible.
