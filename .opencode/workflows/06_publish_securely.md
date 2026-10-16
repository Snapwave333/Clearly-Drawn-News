# Secure YouTube upload and transparent disclosure

## Pipeline
1. **RIGHTS** — check sources, third-party media permissions, credits
2. **FACTS** — independent fact check final script and graphic text
3. **MEDIA** — technical and humanlike sampled quality checks
4. **METADATA** — accurate title, distinct thumbnail, source URLs and chapters
5. **DISCLOSE** — end credits always + synthetic-media field when required
6. **AUTH** — separate YouTube OAuth and channel-specific upload opt-in
7. **UPLOAD** — resumable upload, draft/private default, recording video_id for dedupe
8. **VERIFY** — read-back metadata/captions/visibility and reconcile API restrictions

## Rules
No repeat upload when network status unknown. If permissions absent, stop at package exported locally. Do not mark public until API actually confirms.

## Exit evidence
A completed workflow emits immutable artifact hashes, review gate statuses, agent/event trace IDs, and clear failure records. Do not substitute a successful LLM response for a verified native asset, fetched metric or upload confirmation.
