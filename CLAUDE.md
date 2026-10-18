# CLAUDE.md — Clearly Drawn Studios (ATLAS engine) Development Guide

## Brand Identity

| Layer | Name | Source of truth |
|-------|------|-----------------|
| Company | Clearly Drawn Studios | `atlas_core::COMPANY_NAME` |
| Company tagline | Every story, clearly drawn. | `atlas_core::COMPANY_TAGLINE` |
| Production engine | ATLAS (`atlas` CLI) | `atlas_core::PRODUCTION_ENGINE` |
| Flagship show | Clearly Drawn News | `atlas_core::SHOW_TITLE` |
| Channel | Clearly Drawn | `atlas_core::CHANNEL_NAME` |
| Office simulation | Clearly Drawn Studios HQ | `office-ui/` |

Use these constants instead of hardcoding brand strings in Rust.

## Quick Start

```bash
cd C:\Users\chrom\Desktop\Marker\atlas-studio
cargo build --release                    # Build optimized binary
.\target\release\atlas.exe doctor        # Verify tool discovery
.\target\release\atlas.exe --help        # Show all commands
cargo test                               # Run the workspace test suite
```

## Project Structure

```
atlas-studio/
├── .opencode/           # 38 skills, 10 workflows, 8 schemas (copied from skill pack)
├── crates/              # 10 Rust crates in Cargo workspace
├── tools/               # Bundled binaries (VectorCraft, PhotoCraft, EffectCraft)
├── artifacts/first-run/  # Verified first-run video package
├── youtube_credentials.json  # Ignored now, but present in Git history
└── Cargo.toml           # Workspace root
```

## Key Commands

| Command | Purpose |
|---------|---------|
| `atlas doctor` | Probe all tools (OpenCode, Blender, UE, FFmpeg, Kokoro, ArtCraft) |
| `atlas config show --redact` | Show config with secrets hidden |
| `atlas models probe` | List NVIDIA models via OpenCode |
| `atlas topics create "Title"` | Create new project/topic |
| `atlas produce --topic <id>` | Render real Blender frames, synthesize Kokoro narration, and assemble MP4 |
| `atlas produce --topic <id> --reuse-rendered` | Reassemble from a verified existing frame cache |
| `ATLAS_RENDER_JOBS=2 atlas produce --topic <id>` | Render up to N Blender shots in parallel (default 1) |
| `ATLAS_TTS_JOBS=2 atlas produce --topic <id>` | Synthesize up to N Kokoro narration segments in parallel (default 1) |
| `atlas news research` | Live RSS/Atom wire from Anime News Network, Cartoon Brew, Animation Magazine, and Crunchyroll News; stories from 7 days, cross-outlet corroboration, rumor flagging; falls back to the curated wire (labelled) when feeds are unreachable |
| `atlas news health [--runs 20]` | Feed reliability (success rate, average items, failure streak, last error) and fact-check pass rates from `news/wire_health.json`, which every wire run appends to (last 200 kept); flags feeds failing 3+ runs in a row; such feeds are skipped for 6 hours after their last failed attempt, then retried (skips are not counted as failures) |
| `NVIDIA_API_KEY=... atlas news produce` | Director (LLM) writes each live story's Marcus/Elena dialogue from verified facts only, in parallel; invalid segments fall back to templates. Without a key, template segments run about 1 minute per story and production halts if under 15 minutes. Provenance in `script_provenance.json` |
| `atlas news produce --skip-telegram-wait` | Clearly Drawn News episode package in `news/EP-###/` (fact check, slate, approval, script, rights manifest, timings, status) |
| `ATLAS_MARCUS_VOICE` / `ATLAS_ELENA_VOICE` | Kokoro voices for the anchors (defaults `am_michael` / `af_bella`); `atlas news produce` writes `audio/episode_narration.wav` (two-pass loudnorm to -16 LUFS / -1.5 dBTP, 48 kHz), per-line `audio/lines/*.wav`, and `voice_timeline.json` for Unreal lip sync, and reports runtime from the synthesized audio |
| `atlas news animatic --episode EP-001` | Re-render `preview/animatic.mp4` (1280x720 H.264/AAC review cut: show header, segment titles, speaker lower thirds, on-air captions over the normalized narration; marked not for broadcast). `atlas news produce` renders it automatically after voicing; `ATLAS_PREVIEW_FONT` picks the font |
| `TELEGRAM_BOT_TOKEN=... atlas news produce` | Same, gated on Telegram editorial approval; the token is never stored in code |
| `ATLAS_UE_CMD` / `ATLAS_UE_PROJECT` / `ATLAS_UE_MAP` | Phase 6 of `atlas news produce`: launches the Unreal editor (windowed `UnrealEditor.exe` next to `-Cmd` when present, or `ATLAS_UE_EDITOR`) with `unreal/Content/Python/clearly_drawn_produce.py`, which loads the map, runs `setup_newsroom_studio.py` if present, dresses the anchors, builds `LS_EP_###`, renders PNG frames with Movie Render Queue, writes `render/handoff.json`, and quits; ATLAS then muxes frames + narration into `final/episode.mp4`. `ATLAS_RENDER_PRESET` (`fast` 1 sample default for an RTX 4070, `balanced` 4, `quality` 8), `ATLAS_RENDER_WIDTH`/`HEIGHT`/`FPS` (1920/1080/30), `CLEARLY_DRAWN_TEMPORAL_SAMPLES` override; `status.json` records render minutes, real-time factor, and estimated episodes/day, `CLEARLY_DRAWN_RUN_SETUP=0`, `CLEARLY_DRAWN_KEEP_EDITOR_OPEN=1`, `ATLAS_UNREAL_SCRIPTS`. Skipped (with reason) when the editor, project, or voices are missing. Lip sync: ATLAS writes `lipsync.json` (per-anchor 30 fps jaw-open curves from each line's audio envelope; duo lines move both) and `clearly_drawn_lipsync.py` keys `CTRL_C_jaw` on each anchor's MetaHuman face control rig (`CLEARLY_DRAWN_FACE_RIG`, `CLEARLY_DRAWN_JAW_CONTROL`, `CLEARLY_DRAWN_JAW_SCALE`=0.6, `CLEARLY_DRAWN_LIPSYNC=0` to disable); result in `render/handoff.json` |
| `py "<repo>/unreal/Content/Python/clearly_drawn_wardrobe.py"` | Unreal Editor Python console: dry run listing which dress-code clothing each MetaHuman would get |
| `py "<repo>/unreal/Content/Python/clearly_drawn_wardrobe.py" --apply` | Dress MetaHumans (Torso/Legs/Feet) per `unreal/wardrobe.json`; report in `Saved/ClearlyDrawn/wardrobe_report.json` |
| `py "<repo>/unreal/Content/Python/clearly_drawn_newsroom_sequence.py" <episode_dir> [--apply]` | Unreal Editor: build `LS_EP_###` Level Sequence from an episode package (per-anchor VO tracks from `voice_timeline.json`, camera cuts from `script.json` mapped to CineCameras via `unreal/newsroom_cameras.json`); dry run reports matched cameras |
| `atlas cache stats` | Disk usage of narration, render, B-roll, and vector caches |
| `atlas cache prune --older-than-days 30 --dry-run` | Preview or delete stale cache entries (exports are never touched); the daemon runs this weekly with 30-day retention |
| `atlas daemon start` | Autonomous loop; failed productions back off 30m, then 1h, and are quarantined after the 3rd failure in `daemon/production_failures.json` (delete an entry to retry); skips production while `atlas doctor` reports required tools missing |
| `atlas daemon start` (news) | Produces Clearly Drawn News at `ATLAS_NEWS_HOURS` in the PC's local time (default `12,20` = 2 episodes/day; `0,6,12,18` for 4; `off` to disable). Each slot runs once, only with Telegram approval (`TELEGRAM_BOT_TOKEN`, optional `TELEGRAM_CHAT_ID`) or explicit `ATLAS_NEWS_AUTO_APPROVE=1`; failures back off per slot; `daemon/news_last_run` holds the last slot |
| `news/aired_stories.json` | Live-wire stories aired in the last 7 days (by id, source URL, or headline) are skipped so daily episodes never repeat news |
| `stage_timings.json` | Written next to each `final.mp4`; per-stage seconds plus the slowest stage |
| `audio/segments/<hash>.wav` | Narration cache keyed by text, voice, speed, and language; delete to force re-synthesis |
| `media/<id>/<format>/attribution.json` | B-roll cache; reused when both image and video files are present |
| `atlas publish --topic <id> --platform tiktok` | Publish whiteboard Short directly to TikTok (@focogaming) |
| `atlas publish --topic <id> --platform both` | Publish multi-platform (YouTube + TikTok) |
| `atlas tiktok info --user focogaming` | Inspect TikTok account connection via Upload-Post |
| `atlas tiktok status --request-id <id>` | Query TikTok upload job processing status |
| `atlas tiktok upload --topic <id>` | Direct upload rendered Short to TikTok (@focogaming) |
| `atlas analytics fetch <id> --sample` | Load sample metrics |
| `atlas analytics recommend <id>` | Generate optimization recommendations |
| `atlas analytics auth` | Start YouTube OAuth (auto-loads credentials) |
| `atlas analytics auth-complete --code <CODE>` | Complete OAuth |
| `.\Launch_Unreal_3D_Office.bat` | 1-Click Launch Standalone Unreal Engine 5.8 Virtual Spectator View Window |
| `cd office-ui && npm start` | Launch 24/7 Electron Three.js Office Visualizer & Tray app |

## Blender Broadcast Studio (CDN)

The CDN broadcast studio (`AtlasStudioOffice\BroadcastStudio_NewsroomSet_v3_polished.blend`) is reused by every Blender
shoot through one manifest and one helper. Never hardcode studio camera, screen, anchor, or file names.

| Piece | Path |
|-------|------|
| Manifest (source of truth) | `C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\studio_manifest.json` (override: `ATLAS_STUDIO_MANIFEST`) |
| Helper | `blender/studio_api.py`: `open_studio(copy_to=)`, `use_camera(role)`, `camera_for_cut(cut, kind)`, `anchor(role)`, `set_guest_feed(path)`, `set_brand_screens()`, `setup_shot(kind)`, `plan_episode(dir, kind)`, `apply_render_preset(name)` |
| Dry-run test | `blender -b <copy>.blend --python blender/verify_studio.py -- --report out.json` (`py blender/verify_studio.py` for static checks) |
| Docs | `docs/BLENDER_STUDIO.md`, `AtlasStudioOffice\PIPELINE_STUDIO_README.md` (Mermaid pipeline flow and camera map) |

Camera roles: `desk_hero`, `desk_wide`, `desk_closeup_elena`, `desk_closeup_marcus`, `desk_ots_graphics`,
`interview_two_shot`, `interview_reverse`, `interview_monitor`, `bts_wide`, `bts_handheld`, `throwback_walk_wide`, `throwback_walk_track`.
Anchor roles: `anchor_female` = Elena, `anchor_male` = Marcus (currently Renderpeople Claudia/Eric models, not
MetaHumans; MetaHuman exports will replace them under the same roles).

Desk news defaults to the `weekend_update` camera style (`plan_desk`): `desk_wide` only for open/close, `desk_ots_graphics` on story setups, the reader's locked-off eye-level single for the story, `desk_hero` for banter and handoffs, and a quick cutaway to the other anchor's single on `[REACTION:elena]` / `[REACTION:marcus]`. Anchors alternate stories; cuts land on line boundaries. Script tags (`[BANTER]`, `[SETUP]`, `[REACTION:x]`, `[VISUAL: path]`, `[WALK]`) are stripped before Kokoro by `strip_stage_tags` in `crates/atlas-cli/src/news_voice.rs` (any `[bracket]` tag; tag-only lines are skipped); `studio_api.strip_tags` is the Python equivalent. Preview captions in `news_preview.rs` still use raw `spoken_text`. Desk logo slot: material `CDN_DESK_EMBLEM` (pending rename from `Unreal_Medallion_Mat`).

Story visuals: `set_story_visual("elena"|"marcus", path)` puts slides, photos, clips or lower thirds on the monitor behind the reader (Elena `SM_BS_Screen_38` left, Marcus `SM_BS_Screen_41` right); `[VISUAL: path]` is auto-assigned per story, else the CDN brand loop. `blender/story_cards.py` makes brand-kit headline cards (Midnight/Signal Cyan, Inter), lower thirds, and the brand loop. Thursday Throwback uses `plan_throwback` (desk open, walk on the `throwback_walk_*` cameras, then interview cameras).

Rules:
- Interviews (Thursday Throwback, weekly, 30 min max) use both anchors in the interview corner and only interview cameras. Never the desk.
- Guests (public-domain cartoons) appear only on the `guest_feed` monitor, never in the studio and never on desk monitors.
- No primitives: downloaded assets only.
- Never edit the original .blend files; headless jobs open a copy, never the file open in a visible Blender.
- Run `verify_studio.py` after any studio change; rename a name only together with `studio_manifest.json`.

`atlas news produce` Phase 6 still films in Unreal; selecting the Blender studio as the Phase 6 engine is not wired yet.

## Voice Backend Option: Voicebox

Local [Voicebox](https://github.com/jamiepine/voicebox) v0.5.0 (MIT) is installed at `C:\Users\chrom\Tools\voicebox` (API `http://127.0.0.1:17493`, only while the app is open; `vb-status --start`). It is an optional backend beside Kokoro for the CDN anchors (`CDN Marcus` = `kokoro:am_michael`, `CDN Elena` = `kokoro:af_bella`) and for Thursday Throwback guests (`TT <Character>` profiles: clone from public-domain clips with `vb-clone --engine chatterbox`, or an original voice for silent-era characters). CLI tools `vb-status`, `vb-voices`, `vb-clone`, `vb-say`, `vb-batch` (per-line WAVs + `manifest.json`, strips `[bracket]` stage tags like `strip_tags`), `vb-export`, `vb-import`, `vb-transcribe`, `vb-models`, `vb-effects`, `vb-delete` live in `C:\Users\chrom\Tools\voicebox-cli` (source mirror: `integrations/voicebox-cli/`). `atlas news produce` does not call Voicebox yet. See `docs/VOICEBOX.md`.

## Legacy: Office Simulation

The UE office simulation (`docs/OFFICE_SIMULATION.md`, `office-ui/`, `Launch_Unreal_3D_Office.bat`,
`AtlasStudioOffice\Content\Python\office_runtime_bridge.py`, `setup_spectator_office.py`, the 38-agent MetaHuman crowd)
is legacy overhead. Don't build new pipeline work on it or add dependencies to it, and don't delete it without asking.

## Crate Overview

| Crate | Module | Responsibility |
|-------|--------|----------------|
| `atlas-core` | `lib.rs` | Shared types: `Project`, `AgentMessage`, `ToolKind`, `ToolStatus`, `RenderPreset`, `PublishPlatform` |
| `atlas-doctor` | `lib.rs` | `probe_all()`, `has_minimal_pipeline()` |
| `atlas-opencode` | `lib.rs` | `OpenCodeClient`, `NvidiaStatus`, credential parsing |
| `atlas-store` | `lib.rs` | `Store::open()`, `insert_project()`, `insert_message()`, `conn()` |
| `atlas-agent-bus` | `lib.rs` | `AgentBus`, leases, heartbeats, retries |
| `atlas-analytics` | `lib.rs` + `models/recommend/optimization/youtube.rs/tiktok.rs` | `VideoMetrics`, `YouTubeAnalyticsClient`, `TikTokClient` (Upload-Post), `PublishLedger` |
| `atlas-blender` | `lib.rs` | `BlenderPipeline`, `ShotSpec`, `RenderJob`, Grease Pencil script generation |
| `atlas-kokoro` | `lib.rs` | `KokoroPipeline`, `NarrationSegment`, `load_youtube_credentials()` |
| `atlas-ffmpeg` | `lib.rs` | `FfmpegAssembler`, `EditDecisionList`, filter_complex builder |
| `atlas-cli` | `main.rs` + `produce/publish/production/broll/planning/audio/analytics/memory/daemon/cache/news*.rs` | CLI definitions and dispatch in `main.rs`; `cmd_produce()` and thumbnails in `produce.rs`; `cmd_publish()`/`cmd_tiktok()` in `publish.rs`; stage timing, render fan-out, and duration QA in `production.rs`; B-roll import in `broll.rs`; daemon loop and failure ledger in `daemon.rs`; cache tooling in `cache.rs`; beat planning, lip-sync cues, and captions in `planning.rs`; Freesound and AIWaves scoring in `audio.rs`; `analytics.rs` and `memory.rs` commands; newsroom pipeline in `news.rs`, `news_voice.rs`, and `news_health.rs` |

## Development Workflow

### Adding a New CLI Command
1. Add enum variant in `Commands` or subcommand enum in `crates/atlas-cli/src/main.rs`
2. Add handler function `async fn cmd_xxx(...)`
3. Wire in `main()` match arm
4. Add any new dependencies to `crates/atlas-cli/Cargo.toml`

### Adding a New Agent Skill
1. Create `.opencode/skills/skill-name/SKILL.md` following existing patterns
2. Add to `.opencode/config/agents.json` if it's an active agent

### Running Tests for Specific Crate
```bash
cargo test -p atlas-analytics
cargo test -p atlas-blender
cargo test -p atlas-ffmpeg
cargo test -p atlas-kokoro
cargo test -p atlas-agent-bus
cargo test -p atlas-opencode
cargo test -p atlas-doctor
cargo test -p atlas-store
```

## YouTube OAuth

Credentials file: `youtube_credentials.json` (local only, in .gitignore)

```bash
atlas analytics auth                    # Opens browser with auth URL
# Sign in, grant permissions, copy code from redirect URL
atlas analytics auth-complete --code <CODE>
# Refresh token stored in Windows Credential Manager (keyring)
```

Scopes: `youtube.readonly`, `yt-analytics.readonly`, `youtube.upload`

## Building

```bash
cargo check              # Fast type-check
cargo build              # Debug build
cargo build --release    # Optimized (target/release/atlas.exe)
```

## Code Style

- Rust 2024 edition, rust-version 1.85
- `anyhow::Result` for errors, `thiserror` for crate errors
- `tracing` for logging (use `info!`, `warn!`, `debug!`)
- Serde for serialization, `serde_json` for JSON
- SQLite via `rusqlite` with WAL mode
- No secrets in code/config — use OS credential store (keyring)

## Key Files to Know

| File | Why |
|------|-----|
| `crates/atlas-cli/src/main.rs` | CLI definitions and command dispatch |
| `crates/atlas-cli/src/produce.rs` | Whiteboard vertical slice (`cmd_produce`) |
| `crates/atlas-cli/src/news.rs` | Clearly Drawn News episode pipeline |
| `blender/studio_api.py` | Blender CDN studio helper; reads `AtlasStudioOffice\studio_manifest.json` |
| `blender/story_cards.py` | Brand-kit headline cards, lower thirds, CDN brand loop |
| `docs/BLENDER_STUDIO.md` | Blender studio reuse, camera roles, rules, Mermaid diagrams |
| `.github/workflows/ci.yml` | CI: strict clippy, tests (with FFmpeg), Unreal script checks, office UI build |
| `crates/atlas-analytics/src/youtube.rs` | YouTube OAuth + Analytics API |
| `crates/atlas-kokoro/src/lib.rs` | Kokoro TTS + credentials loading |
| `crates/atlas-blender/src/lib.rs` | Blender script generation |
| `crates/atlas-ffmpeg/src/lib.rs` | EDL → filter_complex assembly |
| `crates/atlas-agent-bus/src/lib.rs` | Message bus, leases, retries |
| `.opencode/skills/*/SKILL.md` | Agent definitions (38 skills) |
| `.opencode/workflows/*.md` | 10 workflow guides |

## Common Issues

| Issue | Fix |
|-------|-----|
| `atlas doctor` shows tools missing | Ensure tools in PATH or `tools/` subdirectory |
| OAuth fails | Check `youtube_credentials.json` exists and redirect URI matches (`http://localhost`) |
| `cargo test` fails | Run `cargo clean && cargo test` |
| Blender/Kokoro/FFmpeg not found | Install in PATH or place in `tools/` |

## Git

```bash
git status
git add -A
git commit -m "message"
git push
```

**Never commit:** `target/`, `youtube_credentials.json`, `*.db`, `*.log`, IDE files

`youtube_credentials.json` exists in the initial Git commit. Rotate the OAuth client and remove the tracked copy only as an explicitly authorized security cleanup.
