# CDN Studio Handoff (Blender v3 set)

Updated 2026-10-18, 6:16 PM MDT. Read this first if you are picking up the studio work.

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
| Debate Corner | DONE, committed (docs+manifest); blend saved | 17_DEBATE_CORNER; renders polish\dfinal\DEBATE_* |
| Crew lockers (CC0 Innerscene LOCK 3, 3 units = 18 lockers, 3 doors ajar + crew items) | DONE, committed; blend saved | BTS_Crew_Lockers (east crew wall by crafty); render polish\dfinal\REVIEW_LOCKERS.png |

## Remaining queue and acceptance criteria

0. **Shelf dressing (Eli: "the shelves are too empty").**
   - Densely dress every shelf (set, crew and kitchen) with CC0 or credited CC BY items: books, binders, trophies, original toy figurines, VHS/DVD cases, boxes, film cans, cables and mugs, plus jars and cookware in the kitchen.
   - Use collection instances and vary rotation and scale. No plants. Save often.
   - Deliver before and after renders.
1. **Debate Corner: DONE.**
   - Copy `dfinal` renders to `/workspace/news-intro/polish/debate_corner_{wide,single_elena,single_marcus,question_screen}.png`.
   - Add a docs section and a CLAUDE.md line, then commit.
   - Accept when:
     - c_dchk shows FORBIDDEN {} and SETCAM_CREW_LEAK [].
     - Collisions are internal only: podium parts, props resting on podiums, adjoining backdrop panels.
     - verify_studio passes.
     - The manifest has debate_* cameras, the `debate` group, the debate_question and debate_score screens, and the STAND_MARKER_Debate_Elena/Marcus markers.
2. **Lockers: DONE.**
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

## Crash note, 2026-10-18 6:07 PM MDT
- Event log: Application Hang 1002. Blender stopped responding while a 96-sample full-studio BTS_WIDE render ran in the GUI thread, and Windows closed it.
- Nothing was lost: the 5:49 PM save holds every build.
- Relaunched as PID 30716.
- To avoid a repeat:
  - Render only at low settings, and never while importing.
  - Import heavy assets one at a time and save after each.
  - Decimate meshes and keep textures at 2K or below.
  - Prefer collection instances.

## Current state (6:16 PM MDT)
- Blender: PID 30716 (headed, listener on 9877). The blend was saved after every build.
- Committed: kitchen, debate and lockers docs, plus this handoff.
- Shelf dressing:
  - IN PROGRESS. The 19 Poly Haven CC0 assets are downloaded and staged at `polish\ph_shelves.zip`, not yet imported. They are book_encyclopedia_set_01, CheeseBox_01, alarm_clock_01, ammo_box, brass_pot_01/02, brass_vase_03/04, ceramic_vase_03, concrete_cat_statue, horse_statue_01, medical_box, rubber_duck_toy, vintage_video_camera, Camera_01, wooden_bowl_02, oil_tin, measuring_tape_01 and spray_paint_bottles.
  - Review cams exist in 12_REVIEW_CAMS: REVIEW_SHELVES_WEST, _NORTH, _EAST and _KITCHEN.
  - "Before" renders go to `polish\shelf_before\`.

## Shelf dressing plan (Eli: "the shelves are too empty")
1. Expand ph_shelves.zip into `news_intro\assets\polyhaven\props\`, then run `c_cr2.py`. It imports any missing PM_ masters; import a few at a time and save.
2. Find the shelf levels by raycasting down through a live rack instance. Use PI_worn_metal_rack.008, rot 0, at (4.4, 5.1, -0.05): its local level z values apply to every worn_metal_rack.
3. Build 4 loadout variants as collections (DRESS_RACK_V1..V4) under BTS_PROP_MASTERS, which is excluded from the view layer. Use linked duplicates of the PM_ meshes laid out in rack-local coordinates.
   - Vary the order, gaps, rotation (±15° plus some 90° turns) and scale (0.9–1.1) so the shelves read as lived-in, not tiled.
   - Contents: books, binders, office_notepads, boxes (cardboard_box_01 at 0.5x, ammo_box, medical_box, metal_toolbox), "film cans" (CheeseBox_01 and russian_food_cans_01 as round-tin stand-ins), cables and tape (medical_tape, measuring_tape_01), mugs and cups (tea_set_01 and the Fab mugs), trophies (brass_goblets, brass_vase_03/04), figurines (horse_statue_01, concrete_cat_statue, rubber_duck_toy; all generic and uncopyrighted), and retro gear (vintage_video_camera, Camera_01, alarm_clock_01).
   - There are no CC0 VHS/DVD case models. Use boxes and binders in their place, or Fab CC BY items if any match, and credit them.
4. Put one collection-instance empty on each of the 18 worn_metal_rack instances, matching their matrix_world and picking a random variant. Do the same for the 2 industrial_storage_carts with their own variant.
5. Kitchen: dress the KITCHEN_TallDrawers top shelves and the back counter with brass pots, ceramic_vase_03 jars, wooden bowls, pot_enamel, the cutting board and tea cups (CC0), plus the Fab frying pan once it is imported.
6. Use low-sample renders only (24 samples, 40%), and never render while importing. Save after each step.
7. Acceptance:
   - Every rack and shelf is visibly full in the after renders (`polish\shelf_after\`, copied to `/workspace/news-intro/polish/shelves_{before,after}_*.png`).
   - No new items are visible in desk, interview, kitchen or debate cams. Run c_kchk with SETC set to the dressing collection and FORB_EXTRA set to the kitchen and debate cams; FORBIDDEN must be {}. Exception: kitchen shelves may appear in kitchen cams.
   - No floating items, and nothing intersecting rack posts.
   - No plants.
   - The blend is saved and committed.

## Fab CC BY 4.0 props: import plan
- **Source:** `AtlasStudioOffice\assets\fab_cc_by\` and CREDITS_CC_BY.md. All 15 are reported "AI use allowed"; re-check each one against CREDITS_CC_BY.md and its Fab page.
- **Files:** studio-softbox-light.zip, studio-umbrella-light.zip, cinema-camera.zip, camcorder_fab.blend, coffee-machine.zip, coffee_machine_mazor_fbx.zip, coffee_machine.blend, cappuccino_mug_blender.zip, coffee_mug.fbx, mug.zip, folding-tables.zip, lockers.zip, teleprompter-dbv2020.zip, frying_pan_blender.zip, office-chair.zip.
- **Process for each asset, one at a time:**
  1. Unzip to `assets\fab_cc_by\<name>\`. Delete the zip only if disk space is tight (about 8 GB free at 6:07 PM).
  2. Import it as a PM_<name> master in BTS_PROP_MASTERS. Append .blend files via bpy.data.libraries.load; import .fbx and .gltf with the importers.
  3. Scale it to real-world size.
  4. Decimate meshes over 100k tris (Decimate modifier, ratio around 0.3, then apply).
  5. Downsize textures over 2K to 2K: image.scale, then save to the asset folder.
  6. Keep Principled PBR materials.
  7. Save, then audit.
- **Placement:**
  - Crafty: coffee machines and mugs, replacing the thermos/airpot duplicates where they match. The folding tables replace the dining_table (crafty).
  - Teleprompters go on the BTS camera pedestals.
  - The cinema camera and camcorder go on the crew carts (industrial_storage_cart, tool cart).
  - Softbox and umbrella lights become crew rigs next to the BTS tripod lights. They are props, so light-link them or keep them unlit.
  - The frying pan replaces brass_pan_01 on the kitchen cooktop.
  - Office chairs go at the crew desks (PI_metal_office_desk around x 15, y -6).
  - Lockers.zip: do NOT replace the CC0 Innerscene lockers without Eli's OK, because Eli said "CC0 only" for the lockers.
- **Credits:** add the manifest `asset_credits` array, one entry per asset: name, creator, url, license "CC BY 4.0", ai_use "allowed", file, used_in. Add a matching docs table so the pipeline can put them in episode description credits.
- **Acceptance:**
  - Every prop is placed and instanced at real-world scale.
  - asset_credits is complete.
  - Audits are clean.
  - A render shows the props (`fab_props_*.png`).
  - Committed.

## Other open items
- The debate wide shot was re-checked: a floating PI_metal_toolbox (left over when the tool cart moved) was moved with the cart and saved.
- verify_studio must still PASS after each step. Expect 3 pending (Armature_Elena, Armature_Marcus, Armature_Chef).
