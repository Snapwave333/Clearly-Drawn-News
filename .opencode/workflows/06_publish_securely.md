# Secure Multi-Platform Publishing (YouTube & TikTok) and Transparent Disclosure

## Pipeline
1. **RIGHTS** — check sources, third-party media permissions, credits.
2. **FACTS** — independent fact check final script and graphic text.
3. **MEDIA** — technical and humanlike sampled quality checks (16:9 for YouTube Long, 9:16 for YouTube Shorts and TikTok).
4. **METADATA** — accurate title, distinct thumbnail, source URLs, chapters, and educational hashtags.
5. **DISCLOSE** — end credits always + mandatory synthetic-media disclosure (`contains_synthetic_media: true` on YouTube, `tiktok_is_ai_generated: true` on TikTok).
6. **AUTH** — separate YouTube OAuth credentials and Upload-Post TikTok API key (`UPLOAD_POST_API_KEY`).
7. **UPLOAD**:
   - **YouTube:** Resumable upload via Data API with captions and thumbnail.
   - **TikTok (@focogaming):** Idempotent upload via Upload-Post MCP/REST with SHA-256 fingerprinting.
8. **VERIFY** — read-back metadata/captions/visibility on YouTube; poll Upload-Post request status on TikTok until `completed` and record published video URLs.

## Rules
- No repeat upload when network status unknown.
- If permissions absent, stop at package exported locally.
- Do not mark public until API confirms.
- TikTok cadence strictly limited to 15 posts per rolling 24 hours.

## Exit evidence
A completed workflow emits immutable artifact hashes in `data/publish/ledger.json`, review gate statuses, agent/event trace IDs, and clear failure records. Do not substitute a successful LLM response for a verified native asset, fetched metric, or upload confirmation.
