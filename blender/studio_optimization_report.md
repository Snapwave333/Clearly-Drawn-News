# CDN studio optimization report (v3)

Live file: `AtlasStudioOffice/BroadcastStudio_NewsroomSet_v3_polished.blend`. It was edited in the visible Blender through the live listener, and the originals were not touched. Renders are in `/workspace/news-intro/polish/` (agent box) and `AtlasStudioOffice/polish/` (PC).

## Round 2026-10-18: Thursday Throwback walk and the 360° studio

### Walk (collection `11_THROWBACK_WALK`)
- Frames 1-1050 at 30 fps (35 s, timed to the bible's Throwback walk beats, e.g. S1-20261022-PM-W at 35 s): chat at the desk 1-180, walk 180-900, sit and settle 900-1050. The anchors start in the desk seats (`SEAT_MARKER_*`), walk behind the desk, step off the east edge of the riser, go through the gap between `SM_BS_Stage_Base197` and `SM_BS_Stage_Base173`, and sit in the interview chairs (`IV_SEAT_MARKER_*`).
- Objects: `WALK_ROOT_Elena` / `WALK_ROOT_Marcus` (animated roots; parent the MetaHuman rigs here), `WALK_PATH_Throwback` (guide curve, not rendered) and `WALK_FOCUS` (midpoint of the two walkers at head height; used as the camera track and DOF target).
- Cameras: `THROWBACK_WALK_TRACK_CAM` (26 mm steadicam that leads, then follows, f/2.8) and `THROWBACK_WALK_WIDE_CAM` (18 mm crane reveal). Manifest roles: `throwback_walk_track` and `throwback_walk_wide` (group `walk`).
- Walk-zone light: `WALK_Ambient_Doorway`, `WALK_Ambient_Corridor`, `WALK_Ambient_IVZone_S` and `WALK_Accent_Cyan_Floor`, all light-linked away from `GUEST_FEED_SCREEN`.

### Blocking (collection `13_BLOCKING_MARKS`)
- These are empty marks (no rigs yet), one per bible crew character, each tagged with `character`/`role` custom props and keyframed across the 1050-frame walk. `CREW_MARK_Roz_Vanterpool` is the floor director, `CREW_MARK_Teddy_Abara` operates pedestal 13/14, and `CREW_MARK_Gus_Ferreira` is grip on the left pedestal and tripod light. `CREW_MARK_Priya_Nair` (producer) stands behind the interview two-shot camera, `CREW_MARK_Desmond_Kaye` (TD) is in the control room, `CREW_MARK_Mae_Lindqvist` (desk PA) goes from the desk to the BTS crates, and `CREW_MARK_Hollis_StandIn` hides behind the crates.
- Sightline audit: rays from the desk and interview cameras to proxy points at 0.3, 1.0 and 1.7 m on every mark, sampled every 50 frames. Mae's late mark leaked into `INTERVIEW_ANCHOR_OTS_CAM` and was moved to (8.6,-6.6)→(9.5,-6.8). No crew appears in `INTERVIEW_MONITOR_CAM`.
- Anchors: desk seats `SEAT_MARKER_*`, walk `WALK_ROOT_*`, interview `IV_SEAT_MARKER_*`.

### Interview skyline
- `IV_SKYLINE_BACKDROP` now uses `skyline_night_golden_bay_defocus.jpg`, a crop of the Poly Haven HDRI golden_bay (CC0): a generic waterfront office block at blue hour, defocused, darkened and cooled. The Shanghai Bund image is gone.

### 360° shell (`09_STUDIO_SHELL_BTS` sub-collections)
- `BTS_Perimeter_Walls`: 100 linked duplicates of the kit wall `SM_BS_Stage_Base172`, two rows (8 m tall) around a 24 x 18.5 m room, finished in `BTS_Studio_Wall_Black` (Poly Haven smooth_concrete normal). On the east wall there is a control-room window (`BTS_ControlRoom_Glass`).
- `BTS_Ceiling`: 8 linked duplicates of the kit ceiling `SM_BS_Stage_Base17`.
- `BTS_Control_Room`: desks, laptops, chairs, racks and a practical light behind the glass.
- `BTS_Grid_Fixtures`: 21 instanced pendant fixtures hung from the grid.
- `BTS_Crew_Area`, `BTS_Gear_Storage`, `BTS_Floor_Gear`: about 75 collection instances. Includes office desks, laptops, notepads, clipboards, binders, thermoses, chairs, stools, metal racks, storage and tool carts, tool chests, a generator, a hand truck, a ladder, military crates, plastic crates, cardboard boxes, power boxes, a wall clock and extra cable runs.
- `BTS_Camera_Gear`: linked duplicates of the kit pedestal camera (interview pedestal) and three tripod spotlights.
- `BTS_Work_Lights`: warm work lights plus `BTS_House_*` area house lights, all light-linked away from the guest screen.
- Instancing: every prop is an empty pointing at a master collection `PM_<asset>` inside `BTS_PROP_MASTERS`, which is excluded from the view layer. Walls, ceiling, pedestals and cable runs are linked duplicates that share mesh data.
- Sightline audit: rays from every desk and interview camera to each prop. Three props showed in `INTERVIEW_TWO_SHOT_CAM` / `INTERVIEW_MONITOR_CAM` and were moved; the hero shots are now clean.
- Review cameras (`12_REVIEW_CAMS`): `REVIEW_CORNER_SW/SE/NE/NW` and `REVIEW_TOP`.

### Floors
- Sets: `SET_FLOOR_DarkSatinWalnut` (Poly Haven dark_wooden_planks; satin finish, roughness 0.36-0.55, low specular) on the desk riser `SM_BS_Stage_Base216`. The interview-zone mask inside `MI_Studio_Floor_DarkSealedConcrete` now uses the same planks instead of the rug.
- Studio/BTS: `MI_Studio_Floor_DarkSealedConcrete` now uses Poly Haven concrete_floor_worn_001, layered with the earlier anti-tiling noise.

### Fixes
- The stray bar behind `SM_BS_Screen_41` had two causes. First, `Backdrop_Wall_Fill_3` overlapped `Backdrop_Wall_Fill_2` by 0.09 rad, so a duplicate wall strip sat inside the LED wall. It is now rotated -3.403 → -3.48 rad, matching the kit's 0.40 rad spacing. Second, its dark end trim (`MI_BS_TrimTexture.058`) showed between the LED wall and the wood. The trim slots on Fill_1/2/3 now use `BRAND_LEDWALL_MAIN`. Verified with cropped renders from the main and wide cameras.
- Backdrop glare hotspot: `Sconce_4` and `Sconce_5` sat on the LED wall. `Sconce_4/5` and the `SM_BS_WallLamp_44` fixture are deleted, and `BRAND_LEDWALL_MAIN` roughness is now 0.85.
- Wall sconces `SM_BS_WallLamp_7/11`: dark bronze body, dim warm glass (emission 0.45); `Sconce_0-3` set to 35 W warm.
- Monitors: `SM_BS_Screen_41` is mirrored from `SM_BS_Screen_38` about x = 0.176, and both are pushed 0.72 m off the curved walls so neither cuts into a wall.
- Removed: all plants (10 objects) and `IV_vase_ceramic_vase_01`; orphans purged.
- Rear faces of the interview back walls (`MI_CDN_LEDWall_Guest`) and the backdrop/skyline materials render as black studio wall.
- Closeups: `BROADCAST_ELENA_CLOSEUP_CAM` / `BROADCAST_MARCUS_CLOSEUP_CAM` are off-axis 40 mm eye-level singles with the speaker's monitor (`SM_BS_Screen_38` / `_41`) over the shoulder.
- Desk logo: object `CDN_Desk_Emblem` (`studio_slot=desk_logo`), materials `CDN_DESK_EMBLEM` / `CDN_DESK_EMBLEM_INK`.

### Still open
- MetaHumans: the scene has none. The project holds only unassembled MetaHuman Creator characters: `/Game/MetaHumans/Ada/MHC_Ada`, `/Game/MetaHumans/Maria/MHC_Maria`, `/Game/MetaHumans/BaseMale/MHC_Base_Male` and `/Game/MetaHumans/Skotukeda/Skotukeda5`, all with `can_build_meta_human = False`. Ada and Maria are presets, which the bible rules out for the anchors. A headless `TryAddObjectToEdit` + `export_dcc` crashed twice: the NullRHI run crashed inside MetaHumanCoreTechLib, and the GPU run ran out of memory ("paging file is too small", 361 MiB allocation). The original builds need the UE 5.8 editor GUI (MetaHuman Creator plus a signed-in Epic account for Auto-Rig and texture synthesis).
- There is no CC0 teleprompter, sandbag or gaffer-tape-mark asset yet.
- Kit overlap pairs that remain (designed joins, not visible): `SM_BS_Panel_8`/`SM_BS_Stage_Base196`, `SM_BS_Panel_01_119`/`SM_BS_Stage_Base189_192`, `SM_BS_Panel_152`/`SM_BS_Stage_Base133`.

### Asset sources (all CC0, Poly Haven)
Models: metal_office_desk, worn_metal_rack, industrial_storage_cart, tool_cart, power_box_01, cardboard_box_01, plastic_crate_01/03, metal_toolbox, clipboard, office_notepads, stationery_supplies, classic_laptop, binder_notebook, plastic_thermos, metal_stool_01, mounted_fluorescent_lights, hanging_industrial_lamp, hand_truck, ladder_sectioned_01, security_camera_01, wall_clock, desk_lamp_arm_01, dining_chair_02, bar_chair_round_01, portable_generator, metal_tool_chest, industrial_pastic_container, old_military_crate, plus the earlier modern_arm_chair_01, side_table_01, caged_hanging_light, modular_electric_cables, plastic_crate_02, wooden_crate_02.
Textures: dark_wooden_planks, concrete_floor_worn_001, smooth_concrete_floor, black_walnut_veneer_01, marble_01, poly_wool_herringbone. HDRI: tv_studio. Skyline photo: see earlier rounds.

## Round: craft services, wall decor, wardrobe (2026-10-18)

### Craft services (`09_STUDIO_SHELL_BTS/BTS_Craft_Services`)
- The crew snack and coffee station sits at (11.1, -5.45) on the studio concrete, east of the interview zone and off every set. A grid sightline search picked the spot.
- It shows in `BTS_WIDE_CAM` and `BTS_HANDHELD_CAM`, and in `THROWBACK_WALK_TRACK_CAM` from frame ~750 to 1050. The audit at frames 1 and 1050 found it in none of the desk closeups, main, OTS, wide or the three interview cameras.
- Contents, all Poly Haven CC0, as linked duplicates of `PM_*` masters:
  - Coffee: folding table and cloth (dining_table, re-tinted with the rough_linen texture), airpots (plastic_thermos, modified_thermos x2), a vintage_electric_kettle for hot water, and tea_set_01 cups.
  - Snacks: wooden_bowl_01 fruit bowl (apples, lemon, lime, kiwi, bananas), wicker_basket_01/02 snack bins, croissants, buns, carrot cake and long_life_food cans.
  - Water: plastic_crate_02 water cases with plastic_bottle_gallon jugs, plus cardboard_box_01 cases.
  - Bins and cooler: plastic_container as the ice cooler, and metal_trash_can (one plain, one blue `CRAFTY_Recycling_Blue`) next to a trashbag.
  - Signs: a standing_chalkboard_01 menu and a taped CRAFTY sign (fancy_picture_frame_01 canvas mesh with original art).
  - Light: `CRAFTY_Practical`, a warm area light linked to `LL_EXCLUDE_GUEST_SCREEN`, plus a pendant.
- Gap: Poly Haven and ambientCG have no CC0 mini fridge, and Sketchfab CC0 downloads need a login, so the cooler stands in for it.

### Wall decor (`09_STUDIO_SHELL_BTS/BTS_Wall_Decor`)
- South wall, crew area:
  - Corkboard call sheet (cork texture from ambientCG Cork001).
  - 5 time-zone clocks with name plates (STUDIO / NEW YORK / LOS ANGELES / LONDON / TOKYO) and a dartboard.
- South wall, centre: a cartoon poster wall with 6 original CDN cartoon posters (Telly, Mic Molly, Sparky the Satellite, The Tally Twins, Captain Cable, Slate) and 2 banners. Frames are hanging_picture_frame_01/02/03, with the glass swapped for `DECOR_Glass_Clear`.
- South wall, east: 4 CDN show posters.
- Safety and ON AIR:
  - Fire extinguisher (korean_fire_extinguisher_01) with its sign, a fire_alarm, and EXIT, cable-caution, NO FOOD ON SET and QUIET signs.
  - An emissive ON AIR light with a red glow light on the east wall by the control room.
- Acoustic panels: 36 foam panels on the west wall and the west half of the north wall. They are linked copies of Eli's kit slab `SM_BS_Desk_Mid_01` with the ambientCG AcousticFoam001 texture, in charcoal and midnight. The upper row sits at z 5.1 to clear the wall's top-row trim.
- All art is original, made with PIL: CDN brand, original characters, no third-party logos or characters. The art and its generator scripts are in `news_intro/assets/decor`.
- Collision sweep (world BVH): 0 overlaps after the fixes. The fixes were: the panel scale, the cartoon wall moved west out of `INTERVIEW_ANCHOR_OTS_CAM`, the upper-row height, the fruit layout, the table rotated so its drape faces the walk, and the cooler and chalkboard moved clear of `BTS_Pedestal_IV_13/14`.

### Story props planted (bible clues, hints only)
- C13: a TIDY nightly report on the corkboard (14 scheduled, 13 removed, 1 "restored by: UNKNOWN").
- C03: Roz's floor log with its MISSING column.
- C11: a blank CREW badge, and the call sheet's Desk PA line left blank.
- Hollis: a blank Stand-in line with a pencil "?", and a doodle of a featureless figure saying "hi".
- C01: a polaroid of 16 round floor marks.
- C09: a potluck sheet on the crafty chalkboard, with Mae's line blank and faint pencil ovals.
- C12: a thermos and cups at crafty.

### Anchor wardrobe (`studio_manifest.json` -> `wardrobe_rotation`)
- Each anchor has 7 outfit ids, one per weekday, picked by the episode date's weekday, so the AM and PM videos match. The pipeline uses `studio_api.outfit_for(anchor, episode_date)`.
- Friday is casual (Elena: knit sweater and jeans; Marcus: denim jacket and knit, no tie). The other six days are polished anchor wear.
- Rules: solid fabrics only (no moire), no cyan/teal or chroma colours, and warm jewel and earth tones against the walnut.
- These will be built as MetaHuman Creator wardrobe (Outfit) items when Eli builds Elena and Marcus.
