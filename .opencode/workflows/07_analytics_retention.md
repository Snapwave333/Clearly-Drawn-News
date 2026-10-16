# Audience-retention ingestion and shot mapping

## Pipeline
1. **POLL** — after ~48h query official YouTube Analytics on owning channel
2. **EXTRACT** — one video ID each; collect normalized time ratios and watch metrics
3. **NORMALIZE** — use column headers, mature-data status and raw metric origin
4. **JOIN** — map 100 normalized points to exact published scene times
5. **ANNOTATE** — attach hypotheses around hook, dense diagrams, proof and chapter transitions
6. **REVIEW** — analyst states likely confounding and sample-size limitations
7. **PROPOSE** — submit evidence-backed controlled future-video trial

## Rules
A missing curve is not 0. Audience watch ratio may exceed 1. Check available quotas and OAuth scopes. Never scrape Studio-only retention or impression data.

## Exit evidence
A completed workflow emits immutable artifact hashes, review gate statuses, agent/event trace IDs, and clear failure records. Do not substitute a successful LLM response for a verified native asset, fetched metric or upload confirmation.
