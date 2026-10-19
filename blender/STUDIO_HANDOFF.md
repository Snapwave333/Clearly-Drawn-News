# CDN Studio Handoff (Blender v3 set)

Updated 2026-10-18, 6:10 PM MDT. Read this first if you are picking up the studio work.

## Files

- **Live blend:** `AtlasStudioOffice\BroadcastStudio_NewsroomSet_v3_polished.blend`. Build scripts save it after each step. Never overwrite the originals, and never open the live file headless; for checks, copy it to `%TEMP%` first.
- **Manifest:** `AtlasStudioOffice\studio_manifest.json`. This is the single source for cameras, camera_groups, screens, anchors, segments, wardrobe_rotation, craft_services, wall_decor, story_props, kitchen_set, debate_corner and crew_lockers.
- **Pipeline API:** `atlas-studio\blender\studio_api.py`. It provides validate() and outfit_for(anchor, date).
- **Verify:** `atlas-studio\blender\verify_studio.py`. Run it headless on a temp copy. Expected: PASS, 0 failed. Pending are Armature_Elena, Armature_Marcus and Armature_Chef.
- **Docs:** `studio_optimization_report.md`, with the box master copy in both repos.
- **Build scripts:** all in `AtlasStudioOffice\polish\`. Each one is idempotent: it wipes its own collection and rebuilds it.
  - `c_bh.py`: shared helpers.
  - `c_crafty.py`: crafty and decor.
  - `c_kitchen.py`, `c_debate.py`, `c_lockers.py`.
  - Importers: `c_cr1.py` and `c_cr2.py` (gltf/glb into BTS_PROP_MASTERS as PM_<id>).
  - Audits: `c_chk.py`, `c_kchk.py` (any set; set `SETC` / `FORB_EXTRA`), `c_dchk.py`, `c_lchk.py`, and `c_aud2.py` for crew sightlines.
- **Renders:** `run_r.ps1 <tag> <cams> <samples> <pct>` writes PNGs to `polish\<tag>\` and also `<tag>.zip`.

## Relaunching and driving Blender

1. Launch it headed, as a GUI on the user's desktop:
   `"C:\Program Files\Blender Foundation\Blender 5.1\blender.exe" "C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\BroadcastStudio_NewsroomSet_v3_polished.blend" --python "C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\polish\live_listener.py"`
   The listener serves TCP 127.0.0.1:9877. PID 3748 is Eli's own Blender, which holds the original; never touch it.
2. Send a script:
   `& "$d\send.ps1" "$d\<script>.py" <timeoutSec>`, with `$d = AtlasStudioOffice\polish`.
   - The script runs in the GUI session and its stdout comes back.
   - End build scripts with `bpy.ops.wm.save_mainfile()`.
3. Render with `start_render(cam, path, samples, pct)`, which is defined in the listener namespace.
   - It is asynchronous and writes `path.done` when finished.
   - Use `run_r.ps1`, or poll for `.done`.
   - While a render runs, Blender shows "Not Responding" and send.ps1 times out. Wait for `.done` before sending more.
4. The machine is slow. Run long jobs in the background and poll; foreground calls get interrupted.
5. Edit the manifest headless with Blender's Python, because python is not on PATH:
   `blender -b --factory-startup --python polish\upd_manifestN.py`. Use OrderedDict to keep key order.

## Eli's rules

- **No primitives.** Use real downloaded assets (CC0: Poly Haven, ambientCG, Innerscene) or Eli's kit (SM_BS_*). Instance repeats as linked duplicates of PM_ masters.
- **No AI-generated images.** Original PIL art (CDN brand, original characters) is allowed.
- **Asset licences:** check every asset's licence for AI-use restrictions and skip anything marked no-AI. The lockers request said "CC0 only".
  - For CC BY assets (Fab), credit the creator, URL and CC BY 4.0 in the manifest `asset_credits` and in the docs.
- **No real studio or brand logos, and no copyrighted characters or imagery.** Debate topics are plain text only.
- **No plants. No people.** Guests appear only on monitors.
- **MetaHumans:** never call anything a MetaHuman without its Unreal asset path. There are none in the scene yet.
- **Reports:** keep them short (renders plus key notes). Report times in MDT.
- **No sends or posts without approval.**
- **Git:**
  - Commit only to `feature/blender-studio-manifest` in atlas-studio, and only our files.
  - Never commit `crates/*`, `unreal/Content/Python/*`, `vlc-help.txt` or `Cargo.lock`.
  - Never delete user files outside the box without approval. Deleting our own scratch files is fine.
- **Keep sets clean:** on-camera set walls stay clean, and BTS dressing must not be visible in desk, interview, kitchen or debate cameras. Audit after every change.

## Done

| Item | Status | Where |
|---|---|---|
| Craft services + BTS wall decor (+ bible story props) | DONE, committed 0ae58f5 | BTS_Craft_Services, BTS_Wall_Decor; renders crafty_area_*, wall_decor_* |
| Wardrobe rotation (7 outfits per anchor, Friday casual) | DONE, committed 0ae58f5 | manifest wardrobe_rotation; studio_api.outfit_for() |
| Kitchen set with chef marker | DONE, committed ebb6de6 (docs); blend saved | 16_KITCHEN_SET; renders kitchen_set_*.png |
| Debate Corner | BUILT + audited + saved; manifest written; docs/commit in progress | 17_DEBATE_CORNER; renders polish\dfinal\DEBATE_* |
| Crew lockers (CC0 Innerscene LOCK 3, 3 units = 18 lockers, 3 doors ajar + crew items) | BUILT + audited + saved; manifest written | BTS_Crew_Lockers (east crew wall by crafty); render polish\dfinal\REVIEW_LOCKERS.png |

## Remaining queue and acceptance criteria

1. **Debate Corner: finish.**
   - Copy `dfinal` renders to `/workspace/news-intro/polish/debate_corner_{wide,single_elena,single_marcus,question_screen}.png`.
   - Add a docs section and a CLAUDE.md line, then commit.
   - Accept when:
     - c_dchk shows FORBIDDEN {} and SETCAM_CREW_LEAK [].
     - Collisions are internal only: podium parts, props resting on podiums, adjoining backdrop panels.
     - verify_studio passes.
     - The manifest has debate_* cameras, the `debate` group, the debate_question and debate_score screens, and the STAND_MARKER_Debate_Elena/Marcus markers.
2. **Lockers: finish.**
   - Render `crew_lockers_*.png`, add docs, commit.
   - Accept when c_lchk shows FORBIDDEN {} (desk, interview, kitchen and debate cams), the lockers are visible in BTS_WIDE_CAM and on the walk track, and the only collisions are items resting on shelves.
3. **Fab CC BY 4.0 props.** Source: `AtlasStudioOffice\assets\fab_cc_by\` and CREDITS_CC_BY.md. All are reported "AI use allowed"; confirm that per asset.
   - Unpack the props. Watch disk space and delete the zips after extracting if space is tight.
   - Import them as PM_ masters and instance them at real-world scale with PBR materials:
     - coffee machines and mugs at crafty
     - folding tables
     - teleprompters on the camera pedestals
     - cinema camera and camcorder on crew carts
     - softbox and umbrella lights as crew rigs
     - frying pan in the kitchen
     - office chairs at the crew desks (x ~15, y -6, control room)
   - Lockers conflict: Eli said CC0 only for the lockers, so the Fab lockers.zip needs Eli's OK before it replaces the CC0 lockers.
   - Accept when:
     - The manifest has an `asset_credits` array with creator, URL and "CC BY 4.0" for each asset, and the docs list them too.
     - Audits are clean.
     - A render shows the props.
4. **Remaining fixes and nice-to-haves.**
   - No CC0 fridge or stand mixer: KITCHEN_FRIDGE_SLOT and KITCHEN_MIXER_SLOT are empty markers. Fill them if a licensed asset turns up.
   - Crafty uses an ice cooler in place of a mini fridge.
   - The S-wall show posters (x 5.4–9.0) are now hidden behind the debate backdrop. Move them if they are wanted.
   - Crew marks show in BROADCAST_WIDE_STUDIO_CAM (Mae, Roz, Teddy). This is known and accepted for the wide shot.
5. **MetaHuman import (Eli, GUI).** Nothing is in the scene yet.
   - Build Elena, Marcus and the Chef in MetaHuman Creator, with wardrobe from the manifest wardrobe_rotation ids.
   - Export each to UE, record its Unreal asset path, export FBX or glTF, and import into Blender.
   - Name the rigs Armature_Elena, Armature_Marcus and Armature_Chef, or update `anchors.<role>.object`.
   - Snap the rigs to the SEAT/STAND markers: SEAT_MARKER_*, IV_SEAT_MARKER_*, STAND_MARKER_Kitchen_*, STAND_MARKER_Chef, STAND_MARKER_Debate_*.
   - Re-run verify_studio. The pending items should go to 0.
