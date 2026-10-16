# ATLAS STUDIO — Autonomous YouTube Educational Video Production

## Project Status: Milestone 1 Complete (Vertical Slice)

**Last Updated:** 2026-10-16  
**Branch:** master  
**Working Directory:** `C:\Users\chrom\Desktop\Marker\atlas-studio`

---

## Architecture Overview

```
atlas-studio/
├── .opencode/                    # Skill pack (38 skills, 10 workflows, 8 schemas)
│   ├── config/                   # Agent registry, defaults, editorial rules
│   ├── skills/                   # 38 agent skills (trend-scout → publisher)
│   ├── workflows/                # 10 orchestrated workflow guides
│   ├── schemas/                  # 8 JSON Schema contracts
│   ├── scripts/                  # Python reference implementations
│   └── reference/                # Specs, bibles, API reality docs
├── crates/                       # Rust workspace (10 crates)
│   ├── atlas-core/               # Shared types, errors, schemas
│   ├── atlas-doctor/             # Tool discovery (OpenCode, Blender, UE, FFmpeg, ArtCraft, Kokoro)
│   ├── atlas-opencode/           # NVIDIA provider reuse via OpenCode CLI
│   ├── atlas-store/              # SQLite WAL persistence
│   ├── atlas-agent-bus/          # Typed message bus with leases, retries
│   ├── atlas-analytics/          # Retention curves, CTR, optimization loop
│   ├── atlas-blender/            # Grease Pencil stroke-reveal render pipeline
│   ├── atlas-kokoro/             # Sentence-segment TTS with retakes
│   ├── atlas-ffmpeg/             # EDL → filter_complex → MP4 + captions
│   └── atlas-cli/                # Unified CLI entrypoint
├── tools/                        # Bundled binaries (verified SHA256)
│   ├── vectorcraft/              # v0.11.0
│   ├── photocraft/               # v0.9.0
│   └── effectcraft/              # v0.1.1
├── youtube_credentials.json      # Google OAuth client secret (local only)
└── Cargo.toml                    # Workspace root
```

---

## Implemented Crates (All Tests Passing: 20/20)

| Crate | Purpose | Key Features |
|-------|---------|--------------|
| `atlas-core` | Shared types | Project states, agent messages, tool kinds, render presets |
| `atlas-doctor` | Tool discovery | Probes OpenCode, Blender 5.1, UE 5.8, FFmpeg, Kokoro, VectorCraft/PhotoCraft/EffectCraft |
| `atlas-opencode` | NVIDIA provider reuse | Parses `opencode providers list`, `opencode models nvidia` via supported CLI |
| `atlas-store` | SQLite WAL | Projects, artifacts, agent messages, YouTube metrics, retention curves |
| `atlas-agent-bus` | Message bus | Leases, heartbeats, exponential backoff retries, idempotency keys |
| `atlas-analytics` | Retention & optimization | Retention curves, CTR, hook analysis, cliff detection, `OptimizationLoop` → agent messages |
| `atlas-blender` | Render pipeline | Grease Pencil stroke-reveal script gen, preview/final quality, frame collection |
| `atlas-kokoro` | TTS synthesis | Sentence-segment with retakes, word timestamps, FFmpeg concat, pronunciation hints |
| `atlas-ffmpeg` | EDL assembly | Filter complex builder, concat + amix, caption tracks, SRT/VTT output |
| `atlas-cli` | Unified CLI | `doctor`, `config`, `models`, `projects`, `topics`, `produce`, `analytics` |

---

## Verified End-to-End Vertical Slice

```bash
# 1. Create topic
atlas topics create "How do lithium-ion batteries actually work?"
# → Creates project vid_20261015_0000, seeds agent message

# 2. Full production (mock frames/audio)
atlas produce --topic vid_20261015_0000
# Stage 1: Script → Stage 2: Storyboard (2 shots)
# Stage 3: Blender renders (90+90 frames) → Stage 4: Kokoro TTS (6 segments)
# Stage 5: FFmpeg assembly → Stage 6: QA & package
# Output: exports/vid_20261015_0000/final.mp4 + captions + audio

# 3. Analytics (sample data)
atlas analytics fetch vid_20261015_0000 --sample
atlas analytics retention vid_20261015_0000
atlas analytics recommend vid_20261015_0000
atlas analytics optimize vid_20261015_0000
# → Generates REVISION_REQUIRED agent messages from retention cliffs

# 4. YouTube OAuth (real credentials)
atlas analytics auth                    # Auto-loads youtube_credentials.json
atlas analytics auth-complete --code <CODE>
# → Refresh token stored in Windows Credential Manager
```

---

## YouTube Integration

- **Credentials:** `youtube_credentials.json` (Google OAuth installed app, local only)
- **Scopes:** `youtube.readonly`, `yt-analytics.readonly`, `youtube.upload`
- **Token Storage:** Windows Credential Manager via `keyring` crate (never in config/files)
- **Refresh Token:** Persisted securely, auto-refreshed on API calls
- **Auth Flow:** `atlas analytics auth` → browser → `atlas analytics auth-complete --code <CODE>`

---

## Next Milestones (Per Spec)

| Milestone | Target | Status |
|-----------|--------|--------|
| **0** System probe & integration | Tool discovery, OpenCode NVIDIA reuse | ✅ Complete |
| **1** Single-video vertical slice | Script → render → audio → MP4 | ✅ Complete (mock) |
| **2** Agent bus & auto revisions | Typed handoffs, revision loops | ✅ Core bus done |
| **3** Full craft pipeline | PhotoCraft, EffectCraft, Unreal, 6-10 min story | 🔄 Pending real renders |
| **4** Automatic trend editorial | Google Trends RSS, YouTube charts, ranking | 🔄 Pending |
| **5** Publication automation | OAuth, private upload, scheduling, UI | 🔄 OAuth done |
| **6** Unattended operations | Quality thresholds, GPU queue, batch ops | 🔄 Pending |

---

## Running Tests

```bash
cargo test                    # All 20 tests pass
cargo test -p atlas-analytics # 5 tests
cargo test -p atlas-blender   # 2 tests
cargo test -p atlas-ffmpeg    # 2 tests
cargo test -p atlas-kokoro    # 2 tests
cargo test -p atlas-agent-bus # 2 tests
cargo test -p atlas-opencode  # 1 test
```

---

## Build & Run

```bash
cargo build --release         # Optimized binary at target/release/atlas.exe
.\target\release\atlas.exe doctor
.\target\release\atlas.exe config show --redact
.\target\release\atlas.exe models probe
```

---

## Security Notes

- ✅ No secrets in repo (`.gitignore` excludes target/, youtube_credentials.json not tracked)
- ✅ OAuth refresh tokens in Windows Credential Manager only
- ✅ NVIDIA API key never extracted; reused via OpenCode CLI
- ✅ SQLite WAL mode, no plaintext credentials in DB
- ✅ Idempotent uploads, crash recovery via durable state

---

## Key Files for Development

| File | Purpose |
|------|---------|
| `Cargo.toml` | Workspace root, dependencies |
| `.opencode/skills/*/SKILL.md` | 38 agent skill definitions |
| `.opencode/workflows/*.md` | 10 workflow guides |
| `.opencode/schemas/*.schema.json` | 8 JSON Schema contracts |
| `crates/atlas-cli/src/main.rs` | CLI command implementations |
| `crates/atlas-analytics/src/youtube.rs` | YouTube OAuth + Analytics API |
| `crates/atlas-kokoro/src/lib.rs` | Kokoro TTS + credentials loading |
| `youtube_credentials.json` | Google OAuth client secret (local only) |