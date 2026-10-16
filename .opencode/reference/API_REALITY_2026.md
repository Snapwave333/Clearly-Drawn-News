# API reality check — October 2026

## YouTube Analytics
- **Owner authorization required**: YouTube OAuth, independent of NVIDIA NIM/OpenCode. Use YouTube Analytics read-only authorization and YouTube read-only authorization when required by the current `reports.query` contract. No general anonymous access to a channel's retention curves.
- Endpoint: `GET https://youtubeanalytics.googleapis.com/v2/reports` with `ids=channel==MINE`, startDate, endDate, required `metrics`; for video retention set `dimensions=elapsedVideoTimeRatio`, `filters=video==ID` (one video per call). Begin with `audienceWatchRatio,relativeRetentionPerformance`, then capability-check other retention metrics.
- `elapsedVideoTimeRatio` is the **exclusive end** of a normalized video segment. Official dimensional spec describes 100 evenly spaced sample points from 0.01 to 1.0 when data is available; do not claim frame- or second-resolution retention for long videos.
- `audienceWatchRatio` can exceed 1 because viewers can rewatch or skip to a point. It is **not** a unique-person completion percentage. `relativeRetentionPerformance` is a peer-relative measurement, not an absolute watch percentage.
- Audience-retention processing typically takes 1–2 days; an API error, insufficient data, or missing report is not 0% retention. Some metrics, e.g. startedWatching/stoppedWatching/totalSegmentImpressions, are report-capability-dependent; ask for them only when supported, with fallback.
- Base metrics including views, averageViewDuration and averageViewPercentage have their own report shapes. Parse `columnHeaders` by name. Respect support matrices and owner privileges.
- YouTube Studio impression CTR and native thumbnail **Test & Compare** functionality should **not** be claimed to be available in the public Analytics/Data API. Never scrape private Studio endpoints. Native thumbnail tests are operator-facing unless official API support is verified.
- **Derived data restrictions**: YouTube API Developer Policies restrict derived, scored and merged metrics. Do not fabricate a proprietary channel quality score. As of June 2026, an application may be eligible for additional derived-metric and storage allowances under specific audit/acceptance requirements; until approved, use raw metrics and simple permitted arithmetic, and keep production hypotheses/editorial text separate. Verify ongoing authorization (including every 30 days for long-lived authorized stats), revocations, content deletion and 30-day refresh/delete for applicable metadata.
- **Publisher**: `videos.insert`, separate OAuth permission; `status.containsSyntheticMedia` is a supported field. Transparent human-readable credits are ALWAYS present. Use platform disclosure when real-looking synthetic/meaningfully altered depictions trigger it. Note that API-project upload visibility restrictions, authorization rules and limits can change—validate your project's actual responses instead of hard-coded universal claims.

## Tool reality
- OpenCode NVIDIA key should be accessed through supported configured OpenCode inference broker if possible. Detect version/capabilities. No arbitrary reading of credential files or exporting raw secrets. No second key prompt unless an explicitly authorized secure fallback is necessary.
- Blender headless: inspect installed Python and Grease Pencil APIs before generating scripts. Unreal CLI/Sequencer/Movie Render Queue vary by engine version and project plugin configuration. Tool presence ≠ working end-to-end adapter.
- VectorCraft, PhotoCraft and EffectCraft provide command/MCP authoring surfaces. They are **not** prompt-to-image tools. ATLAS must supply geometry, scene graphs and edits.
- Kokoro interface may be bindings/server/executable; detect it at setup. Audio source licensing still applies.
- DuckDuckGo unofficial search bridges can throttle/break; use sources only for discovery and retrieve original records where possible.

## Official references
- Analytics Channel Reports: https://developers.google.com/youtube/analytics/channel_reports
- Analytics Metrics: https://developers.google.com/youtube/analytics/metrics
- Analytics Dimensions: https://developers.google.com/youtube/analytics/dimensions
- Query method and OAuth: https://developers.google.com/youtube/analytics/reference/reports/query
- YouTube retention help: https://support.google.com/youtube/answer/9314415
- Data API Videos: https://developers.google.com/youtube/v3/docs/videos
- Videos Insert: https://developers.google.com/youtube/v3/docs/videos/insert
- API developer policies: https://developers.google.com/youtube/terms/developer-policies
- Derived metrics conditions: https://developers.google.com/youtube/terms/derived-metrics-policy
- AI disclosure policy: https://support.google.com/youtube/answer/14328491
- Blender automation: https://docs.blender.org/manual/en/latest/advanced/command_line/arguments.html
- Unreal scripting and Movie Render Queue: https://dev.epicgames.com/documentation/en-us/unreal-engine/movie-render-pipeline-in-unreal-engine
- VectorCraft: https://github.com/storytold/vectorcraft
- PhotoCraft: https://github.com/storytold/photocraft
- EffectCraft: https://github.com/storytold/effectcraft
