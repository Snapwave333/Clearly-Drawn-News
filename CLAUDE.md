# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

ATLAS STUDIO: a Rust workspace whose single binary, `atlas` (crate `atlas-cli`), drives an autonomous video pipeline for the "Clearly Drawn" channel. Two production lines share it:

- **Whiteboard explainers / Shorts** (`atlas produce`, `atlas publish`, `atlas daemon`): topic → storyboard → Kokoro TTS narration → Blender Grease Pencil frames → FFmpeg assembly → policy preflight → YouTube / TikTok upload.
- **Clearly Drawn News** (`atlas news ...`): a 15–30 minute, 16:9, two-anchor (Marcus / Elena) animation-news broadcast with a Telegram human-approval gate and an Unreal Engine 5.8 virtual studio.

The project was developed on Windows (`C:\Users\chrom\Desktop\Marker\atlas-studio`); the Unreal project (`AtlasStudioOffice`), external tools (`tools/`) and `artifacts/` live outside this repo or are gitignored. The Rust workspace builds and its tests pass on Linux too.

## Commands

```bash
cargo build --release                         # target/release/atlas(.exe)
cargo check
cargo test --workspace                        # ~65 unit tests, no external tools needed
cargo test -p atlas-policy                    # one crate
cargo test -p atlas-policy copyright          # tests whose name matches a filter
cargo clippy --workspace -- -D warnings       # the bar the audit held the code to

atlas doctor                                  # probe OpenCode, Blender, UE, FFmpeg, Kokoro, ArtCraft tools
atlas --data-dir <dir> ...                    # override the SQLite/artifact data dir (default: ProjectDirs com/atlas-studio/atlas)
atlas news research [--json]                  # live RSS wire + cross-outlet fact check
atlas news slate                              # episode slate (stories + rapid-fire bulletins)
atlas news telegram                           # send slate to Telegram and wait for a decision
atlas news produce [--publish --privacy private --shorts] [--target-minutes 22] [--resolution uhd|fhd] [--encoder x264|x265|nvenc-h264|nvenc-hevc] [--studio-plate f] [--music f]
atlas news produce --episode EP-003           # resume an approved episode (reuses selection, script, TTS cache)
atlas news copyright --episode EP-003         # re-scan an episode's rights_manifest.json
atlas news publish --episode EP-003 --privacy private [--shorts]

cd office-ui && npm install && npm start      # Electron + Three.js office visualizer (npm run dev for Vite HMR)
docker compose -f docker-compose.memory.yml up -d   # Neo4j + Graphiti backing `atlas memory`
```

All tests are inline `#[cfg(test)]` modules in each crate's `src/` (no `tests/` dirs). The root `src/lib.rs` is an unused `cargo new` stub, not part of the workspace members.

## Architecture

- **`atlas-cli/src/main.rs` (~4.5k lines) holds nearly all orchestration.** Each subcommand is a `Commands`/`*Action` clap enum variant dispatched from `main()` to an `async fn cmd_*`. The whiteboard vertical slice is `cmd_produce`; the newsroom is `cmd_news`; `cmd_daemon` loops `run_daemon_cycle` until a `daemon/paused` file appears in the data dir. Library crates are mostly typed building blocks the CLI wires together.
- **Data dir is the integration point.** `Store::open(data_dir)` (SQLite WAL, `atlas-store`) plus subfolders the CLI writes: `renders/<video_id>`, `vectors/`, `policy/`, `publish/ledger.json`, `payroll/ledger.json`, `daemon/`. `cmd_news` does not use the store.
- **Crate roles** (dependency direction: everything depends on `atlas-core`; `atlas-store` → `atlas-agent-bus` → `atlas-memory`):
  - `atlas-core`: shared types and enums (`Project`, `ProjectState`, `ToolKind`, `PublishPlatform`), plus the news show constants, `BroadcastFormat`, `EpisodeDurationPolicy` (900–1800 s, ~1320 s target) and the 17-stage `NewsProductionStage` state machine.
  - `atlas-blender`, `atlas-kokoro`, `atlas-ffmpeg`: generate scripts / shell out to the external tools. Blender output is rejected on a Python traceback even with exit code 0.
  - `atlas-analytics`: YouTube OAuth + Data/Analytics API, TikTok via the Upload-Post API, publish ledger/cadence, retention and recommendations.
  - `atlas-policy`: `PolicyGuardian` pre-publish checks, `copyright.rs` (rights registry, 4-factor fair use, trailer audio muted), `telegram.rs` (approval gate with Approve / Reject / Replace / Request More Research).
  - `atlas-media`: stock media client, `news_desk.rs`, `fact_checker.rs` (two-source corroboration, official-entity requirement).
  - `atlas-storyboard`: `DirectorClient` (LLM creative director via `ATLAS_DIRECTOR_URL` / `ATLAS_DIRECTOR_API_KEY` or `NVIDIA_API_KEY` / `ATLAS_DIRECTOR_MODEL`), `two_anchor_script.rs` (8-segment episode), `pronunciation.rs`.
  - `atlas-memory`: Shared Context MCP (working memory) + Graphiti/Neo4j (knowledge graph) with a promoter.
  - `atlas-opencode`: reuses NVIDIA model access through the `opencode` CLI.
- **News pipeline** (`atlas-cli/src/news.rs`, stages logged with `NewsProductionStage` to `production_log.json`):
  1. `atlas-media::news_desk` fetches RSS/Atom feeds (`default_sources()`, or `ATLAS_NEWS_FEEDS="Outlet|url|anime,..."`, or `--feeds-file` JSON; `file://` works for fixtures), classifies category and organization by keyword.
  2. `fact_checker` drops rumors/reviews and clusters headlines across outlets (`title_tokens`/`same_story`). Stories with ≥ `--min-sources` outlets (default 2) and a known organization are main stories; the rest become attributed single-source bulletins. Fewer than 3 main stories aborts.
  3. Telegram gate (`atlas-policy::telegram`, HTML messages, `TELEGRAM_BOT_TOKEN`/`TELEGRAM_CHAT_ID`, `TELEGRAM_API_BASE` override): Approve / Reject / More Research (re-fetch) / Swap #n (drop a story, re-slate). Mandatory unless `--skip-telegram`.
  4. `atlas-storyboard::two_anchor_script`: cold open, rapid-fire and sign-off are deterministic from real headlines; each story segment is written by `NewsScriptDirector` (OpenAI-compatible chat API: `ATLAS_DIRECTOR_URL`, `ATLAS_DIRECTOR_API_KEY`/`NVIDIA_API_KEY`, `ATLAS_NEWS_SCRIPT_MODEL`) and rejected/retried if word count, speakers, hook length, banned stock phrases or any digit not in the fact sheet fails `validate_story_lines`, or the marked Short fails `validate_short` (90–150 words, no context-dependent opener). The CLI then appends "Still ahead" teases and builds the cold open around the lead story's hook. `--target-minutes` (15–30) sizes the per-story word budget. No LLM key = hard error, never template filler.
  5. Kokoro per line (`ATLAS_KOKORO_EXE`, `ATLAS_KOKORO_MODEL_DIR`, voices `ATLAS_MARCUS_VOICE`/`ATLAS_ELENA_VOICE`, `ATLAS_TTS_JOBS`), cached by voice+text hash in `<data>/news/tts`.
  6. `atlas-ffmpeg::broadcast`: dialogue concat with measured durations → timeline; two-pass loudnorm to -14 LUFS; copyright scan of real file hashes (third-party music/plates need a `<file>.rights.json` sidecar or the render is blocked); single-pass 16:9 render (4K by default; graphics drawn in yuv444p and converted to BT.709 4:2:0 at the end; encoder profiles in `VideoEncoder`, x264 preset/CRF via `ATLAS_NEWS_X264_PRESET`/`ATLAS_NEWS_CRF`) of the graphics studio (anchor panels with voice meters, video wall, lower thirds, ticker, segment cards) via `filter_complex_script` with fonts/text files copied into `work/` so no path escaping is needed; SRT, thumbnail, chapters.
  7. Shorts: each story's marked span is cut from `master.wav` and rendered as a 1080x1920 vertical (`render_short`: hook title, speaker card, phrase-chunked burned-in captions, CTA) into `shorts/`, with `shorts.json` metadata; `--shorts` uploads them after the episode with the episode URL substituted into the description.
  8. QA (`qa_report.json`: 900–1800 s, resolution, 30 fps, h264/aac stereo, loudness) gates `publish`, which runs `PolicyGuardian` and uploads to YouTube (category 1, synthetic media declared).
  Outputs live in `<data>/news/episodes/EP-NNN/`. Unreal is optional input only: pass a rendered plate with `--studio-plate`; nothing drives UE automatically.
- **`.opencode/`** is an agent skill pack for OpenCode (`opencode.json` loads `.opencode/prompts/*RULES.md`): ~40 `skills/*/SKILL.md`, 12 numbered `workflows/`, JSON `schemas/`, Python reference `scripts/`. New agent roles go in `skills/` and `.opencode/config/agents.json`.
- **`AGENTS.md`** describes the UE 5.8 MetaHuman office simulation (`docs/OFFICE_SIMULATION.md`); `AUDIT_2026-10-16.md` records what was verified versus mocked in the whiteboard pipeline. Readiness (`READY_TO_PUBLISH`) must stay gated on verified frames, duration, size, captions, metadata, disclosures and credits.

## Conventions

- Rust 2024, rust-version 1.86; `anyhow::Result` in the CLI, `thiserror` errors in crates (`AtlasError`), `tracing` for logs, `rusqlite` bundled.
- Do not add code comments anywhere in the codebase (owner preference).
- Credentials load from gitignored local files (`youtube_credentials.json`, `youtube_token.json`, `upload_post_credentials.json`, `local.properties`, `freesound_key.txt` in the data dir) or env vars (`UPLOAD_POST_API_KEY`, `TIKTOK_PROFILE`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`, `ATLAS_NEWS_FONT`/`ATLAS_NEWS_FONT_BOLD`, `FREESOUND_API_KEY`, `NVIDIA_API_KEY`, `AIWAVES_DIR`); YouTube refresh tokens go to the OS keyring.
- Publishing defaults to `--privacy private`; public publication requires an explicit user decision.
- `tools/`, `artifacts/`, `*.exe`, `*.db` are gitignored. `main` history was rewritten with `git filter-branch` to strip binaries and credentials.
