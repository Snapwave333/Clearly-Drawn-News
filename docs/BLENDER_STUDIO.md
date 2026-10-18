# Blender broadcast studio (CDN)

Shoots in the Blender CDN broadcast studio go through two files in this repo and one manifest next to the set:

| File | Purpose |
|------|---------|
| `C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\studio_manifest.json` | Single source of truth (canonical .blend, camera roles, screen slots, anchor roles, brand kit, render presets, rules). Override with `ATLAS_STUDIO_MANIFEST` |
| `blender/studio_api.py` | Helpers for Blender scripts: `open_studio(copy_to=)`, `camera`/`use_camera(role)`, `camera_for_cut(cut, kind)`, `anchor(role)`, `set_guest_feed(path)`, `set_screen_media(role, path)`, `set_brand_screens()`, `setup_shot(kind)`, `plan_episode(episode_dir, kind)`, `apply_render_preset(name)`, `set_output(dir)`, `validate()` |
| `blender/verify_studio.py` | Dry-run test. `python blender/verify_studio.py` runs static checks; `blender -b <copy>.blend --python blender/verify_studio.py -- --report out.json` checks every name in the scene. Missing scene names are `PENDING`, broken data is `FAILED` |

Full usage guide: `AtlasStudioOffice\PIPELINE_STUDIO_README.md`.

## Rules

- Camera roles: `desk_hero`, `desk_wide`, `desk_closeup_elena`, `desk_closeup_marcus`, `desk_ots_graphics`,
  `interview_two_shot`, `interview_reverse`, `interview_monitor`, `bts_wide`, `bts_handheld`.
- `script.json` `camera_cut` values (`StudioCameraCut`, snake_case) map to roles through `script_cut_map.desk` or
  `script_cut_map.interview`.
- Interviews (Thursday Throwback, weekly, 30 min max) use both anchors in the interview corner and only interview
  cameras; `setup_shot` rejects desk cameras for interviews.
- Guests (public-domain cartoons) appear only on the `guest_feed` monitor, never in the studio.
- Anchors are addressed by role: `anchor_female` (Elena), `anchor_male` (Marcus). Today they are Renderpeople models
  (Claudia, Eric); MetaHuman exports will replace them without changing the roles.
- No primitives: downloaded assets only. Originals are never edited; headless jobs open a copy of the canonical .blend.
- The office simulation (`docs/OFFICE_SIMULATION.md`) is legacy and is not part of this pipeline.

## Relation to `atlas news produce`

Phase 6 still films in Unreal (`DEFAULT_UE_PROJECT` in `crates/atlas-cli/src/news.rs`, cameras from
`unreal/newsroom_cameras.json`). The Blender studio produces frames in the same `render/frames/frame.NNNNNN.png`
layout and can reuse the same episode package through `plan_episode`. Selecting Blender as the Phase 6 engine is
not wired yet.

## Pipeline flow

```mermaid
flowchart TD
    A["atlas news produce<br/>wire, fact check, approval"] --> B["script.json<br/>camera_cut per line"]
    B --> C["Kokoro voices<br/>voice_timeline.json, lipsync.json"]
    C --> U["Phase 6 today: Unreal<br/>AtlasStudioOffice.uproject + clearly_drawn_produce.py<br/>cameras via unreal/newsroom_cameras.json"]
    C --> M["studio_manifest.json"]
    M --> S["studio_api.py (inside Blender)"]
    S --> K["open_studio(copy_to=episode/render/studio.blend)"]
    K --> P["plan_episode(): cuts by camera role<br/>desk or interview table"]
    P --> Q["setup_shot(), set_guest_feed(), set_brand_screens()<br/>apply_render_preset()"]
    Q --> F["render/frames/frame.######.png"]
    U --> F
    F --> G["FFmpeg mux with episode_narration.wav<br/>final/episode.mp4"]
    V["verify_studio.py on a copy"] -. checks names .-> M
```

## Studio layout and camera map

```mermaid
flowchart LR
    subgraph SHELL["09_STUDIO_SHELL_BTS: grid, pedestals, teleprompter, control-room glass"]
        BTSW(["bts_wide<br/>BTS_WIDE_CAM"])
        BTSH(["bts_handheld<br/>BTS_HANDHELD_CAM"])
        subgraph DESK["Desk set: 04_NEWS_DESK, 05_STAGE_SET, 06_BACKDROP_WALLS"]
            WALL["backdrop_brand<br/>BRAND_LEDWALL_MAIN"]
            SIDE["side_screens<br/>BRAND_SCREEN_*"]
            ANCH["anchor_female Elena + anchor_male Marcus<br/>03_ANCHORS"]
            LOGO["desk_logo"]
            HERO(["desk_hero"])
            WIDE(["desk_wide"])
            CUE(["desk_closeup_elena"])
            CUM(["desk_closeup_marcus"])
            OTS(["desk_ots_graphics"])
        end
        subgraph IV["08_INTERVIEW_SET: Thursday Throwback, both anchors, never the desk"]
            TV["guest_feed TV<br/>GUEST_FEED_SCREEN"]
            SKY["night skyline + interview_backdrop"]
            CH["two anchor chairs, table, lamp"]
            TWO(["interview_two_shot"])
            REV(["interview_reverse"])
            MON(["interview_monitor"])
        end
    end
    HERO --> ANCH
    WIDE --> WALL
    CUE --> ANCH
    CUM --> ANCH
    OTS --> SIDE
    TWO --> CH
    TWO --> TV
    REV --> CH
    MON --> TV
    BTSW --> DESK
    BTSH --> IV
```

