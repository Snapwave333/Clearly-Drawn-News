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
cargo test --workspace                        # ~45 unit tests, no external tools needed
cargo test -p atlas-policy                    # one crate
cargo test -p atlas-policy copyright          # tests whose name matches a filter
cargo clippy --workspace -- -D warnings       # the bar the audit held the code to

atlas doctor                                  # probe OpenCode, Blender, UE, FFmpeg, Kokoro, ArtCraft tools
atlas --data-dir <dir> ...                    # override the SQLite/artifact data dir (default: ProjectDirs com/atlas-studio/atlas)
atlas news research | slate | copyright | telegram | produce [--skip-telegram-wait]

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
- **The news line is partly scaffolding.** `AnimationNewsDesk::fetch_curated_wire` returns a hardcoded story list (no live fetch), and in `news produce` the copyright, mastering and (when UE isn't installed) Unreal phases only print status. The UE step runs `UnrealEditor-Cmd.exe` with `AtlasStudioOffice/Content/Python/setup_newsroom_studio.py` at hardcoded Windows paths. Don't describe these stages as working end to end without checking the code.
- **`.opencode/`** is an agent skill pack for OpenCode (`opencode.json` loads `.opencode/prompts/*RULES.md`): ~40 `skills/*/SKILL.md`, 12 numbered `workflows/`, JSON `schemas/`, Python reference `scripts/`. New agent roles go in `skills/` and `.opencode/config/agents.json`.
- **`AGENTS.md`** describes the UE 5.8 MetaHuman office simulation (`docs/OFFICE_SIMULATION.md`); `AUDIT_2026-10-16.md` records what was verified versus mocked in the whiteboard pipeline. Readiness (`READY_TO_PUBLISH`) must stay gated on verified frames, duration, size, captions, metadata, disclosures and credits.

## Conventions

- Rust 2024, rust-version 1.85; `anyhow::Result` in the CLI, `thiserror` errors in crates (`AtlasError`), `tracing` for logs, `rusqlite` bundled.
- Do not add code comments anywhere in the codebase (owner preference).
- Credentials load from gitignored local files (`youtube_credentials.json`, `youtube_token.json`, `upload_post_credentials.json`, `local.properties`, `freesound_key.txt` in the data dir) or env vars (`UPLOAD_POST_API_KEY`, `TIKTOK_PROFILE`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`, `FREESOUND_API_KEY`, `NVIDIA_API_KEY`, `AIWAVES_DIR`); YouTube refresh tokens go to the OS keyring.
- Publishing defaults to `--privacy private`; public publication requires an explicit user decision.
- `tools/`, `artifacts/`, `*.exe`, `*.db` are gitignored. `main` history was rewritten with `git filter-branch` to strip binaries and credentials.
