# CDN: START HERE (master handoff, Oct 18 2026, 6:15 PM MDT)

Any assistant (Claude Code, Cursor, etc.) picking this up: read this file, then the three detailed handoffs below.

## Detailed handoffs
1. `Marker\HANDOFF.md`: pipeline (atlas-cli, news produce, publish, daemon, director/NIM fallback)
2. `Marker\AtlasStudioOffice\STUDIO_HANDOFF.md`: Blender studio set, cameras, manifest, queue + acceptance criteria
3. `Marker\AtlasStudioOffice\casting\voices\VOICE_HANDOFF.md`: anchor voices (Parler design, then Voicebox/Chatterbox), rerun commands
Also: `atlas-studio\docs\NEWS_PIPELINE.md` (Mermaid), `CLAUDE.md`, `AGENTS.md`, `casting\series\CDN_Season1_Bible.md`, `CDN_Chef_Character.md`, `CDN_Season1_beats.csv`.
Repo branch: `feature/blender-studio-manifest` (small commits, keep docs + Mermaid current).

## Eli's rules (never break)
- Never say "yep". Ask ONE question at a time. Short, emoji-anchored, image-led updates.
- Honesty: always give verifiable proof (renders, file paths, logs). Never claim something works unverified.
- People = real Epic MetaHumans only, original builds (no presets like Ada/Maria). Elena is Asian American. Guests only on monitors. Interviews: both anchors in the interview corner.
- No primitives; real assets only: CC0, or CC BY with credit. Skip any "Allows usage with AI: No". The studio has no generative AI. No plants. Dense, finished from every angle. Fictional skyline. Satin set floors, concrete crew floors. Symmetrical monitors. SNL Weekend Update camera style.
- Real news only, 2-source fact check; retrospectives when news is thin; never invented. International (Japan first), translated to English, source credited. Free sources only (RSS, official YouTube/Bluesky/newsrooms). NO X/Twitter (Eli is banned).
- 1 episode/day, ~30 min, 12:00 MDT, plus several TikTok clips. Segments: news desk, Thursday Throwback, cooking (chef Delphine Quill), Debate Corner. Nothing else.
- Cooking: real, credited recipes plus a USDA food-safety gate. Prop agent plus cooking animation director.
- Voices: ORIGINAL (no cloning real people), emotional, exactly 2 consistent anchor voices.
- 7 outfits per anchor, daily rotation, casual Friday.
- Pipeline is self-contained: Claude API credits first, then NVIDIA NIM free models. No assistant-side routines.
- Blender: visible, headed. Never overwrite the original .blend.

## Go-live (ONLY when Eli says go)
```
[Environment]::SetEnvironmentVariable('ATLAS_NEWS_PUBLISH','on','User')
[Environment]::SetEnvironmentVariable('ATLAS_NEWS_TIKTOK','on','User')   # only after TikTok auth
Enable-ScheduledTask -TaskName 'CDN News Daemon'; Start-ScheduledTask -TaskName 'CDN News Daemon'
```
The daemon must self-heal (restart on crash, start on reboot).

## Waiting on Eli
- Build MetaHumans in UE 5.8 MetaHuman Creator (Auto-Rig, signed in): Elena, Marcus, Delphine, 7 crew (Roz, Teddy, Gus, Priya, Desmond, Mae, Hollis).
- TikTok: dev app (Login Kit + Content Posting API Direct Post, scopes user.info.basic,video.publish, redirect http://localhost:3455/callback/), run `atlas news tiktok-auth`, submit for audit (SELF_ONLY until then).
- Approve voices (v3 banter in progress). Confirm what "CDN" stands for. Guest rights_confirmed=yes. Weekends and finale outro questions. Say "go" for publishing.

## In progress at handoff
- Studio: relaunch after a Blender crash (last save 5:49 PM, polished.blend). Import 15 CC BY Fab props from `assets\fab_cc_by\` one at a time (credits in CREDITS_CC_BY.md, add to the manifest asset_credits). Densely fill ALL shelves (Eli: "too empty"). Craft services, wall decor, wardrobe rotation, kitchen set, Debate Corner, 360 renders.
- Pipeline: daemon self-heal, studio-name matching fix, Debate + Cooking segments, recipes + food safety, prop agent, cooking animator, multiple TikTok clips, asset credits in descriptions, full 30-min dry run (no upload).
- Voices: v2 had 4-5 drifting voices and Marcus's "Elena" lines came out in a female voice. v3 = locked references plus a speaker-embedding consistency gate (exactly 2 speakers), fix "Elena" heard as "Alina", open in VLC for Eli.
