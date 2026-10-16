# Voice, audio and timing synchronization

## Pipeline
1. **SYNTHESIZE** — Kokoro reads source-locked short utterances and pronunciation dictionary
2. **ALIGN** — produce scene timings and update storyboard duration
3. **MIX** — editor balances speech and cleared SFX/music
4. **CONFORM** — video editor creates exact scene map and final audio/video
5. **CAPTION** — accessibility agent validates content and placement
6. **TECHQA** — ffprobe + frame sampling + loudness checks

## Rules
Never compress narration mechanically to compensate for wrong scene length. Narration should sound purposeful and remain intelligible.

## Exit evidence
A completed workflow emits immutable artifact hashes, review gate statuses, agent/event trace IDs, and clear failure records. Do not substitute a successful LLM response for a verified native asset, fetched metric or upload confirmation.
