"""Reusable access to the CDN broadcast studio for Blender pipeline scripts.

Every name (cameras, screens, anchors, brand files, render presets) comes from
AtlasStudioOffice/studio_manifest.json, so shoots never hardcode the studio.

    import sys
    sys.path.insert(0, r"C:\\Users\\chrom\\Desktop\\Marker\\atlas-studio\\blender")
    import studio_api as studio

    studio.open_studio(copy_to=r"C:\\temp\\EP-012\\studio.blend")   # never the live file
    studio.setup_shot("interview", guest_feed=r"C:\\...\\felix_take3.mp4")
    studio.apply_render_preset("fast")

Set ATLAS_STUDIO_MANIFEST to point at another manifest.
"""
import json
import os
import re
import shutil

try:
    import bpy
except ImportError:  # manifest helpers still work outside Blender
    bpy = None

DEFAULT_MANIFEST = r"C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\studio_manifest.json"
VIDEO_EXTENSIONS = {".mp4", ".mov", ".mkv", ".avi", ".webm", ".m4v"}
_CACHE = {}


def manifest_path():
    return os.environ.get("ATLAS_STUDIO_MANIFEST", "").strip() or DEFAULT_MANIFEST


def load_manifest(path=None, reload=False):
    path = os.path.abspath(path or manifest_path())
    if reload or path not in _CACHE:
        with open(path, "r", encoding="utf-8") as handle:
            data = json.load(handle)
        data["_path"] = path
        data["root"] = data.get("root") or os.path.dirname(path)
        _CACHE[path] = data
    return _CACHE[path]


def resolve(relative, manifest=None):
    manifest = manifest or load_manifest()
    if not relative:
        return None
    return os.path.normpath(relative if os.path.isabs(relative) else os.path.join(manifest["root"], relative))


def studio_blend(manifest=None):
    manifest = manifest or load_manifest()
    return resolve(manifest["blend"]["canonical"], manifest)


def brand_asset(key, manifest=None):
    manifest = manifest or load_manifest()
    if key not in manifest["brand_kit"]:
        raise KeyError("unknown brand asset '{}'; known: {}".format(key, ", ".join(sorted(manifest["brand_kit"]))))
    return resolve(manifest["brand_kit"][key], manifest)


def _require_bpy():
    if bpy is None:
        raise RuntimeError("studio_api scene functions must run inside Blender")


def open_studio(copy_to=None, manifest=None):
    """Open the canonical studio. Pass copy_to to work on a private copy (required for headless jobs)."""
    _require_bpy()
    manifest = manifest or load_manifest()
    source = studio_blend(manifest)
    target = source
    if copy_to:
        os.makedirs(os.path.dirname(os.path.abspath(copy_to)), exist_ok=True)
        shutil.copy2(source, copy_to)
        target = copy_to
    elif manifest["blend"].get("copy_before_use") and bpy.app.background:
        raise RuntimeError("headless jobs must pass copy_to; the live studio file may be open in another Blender")
    bpy.ops.wm.open_mainfile(filepath=target)
    return target


# ---------------------------------------------------------------- cameras

def camera_roles(group=None, manifest=None):
    manifest = manifest or load_manifest()
    if group:
        return list(manifest["camera_groups"][group])
    return list(manifest["cameras"])


def camera_name(role, manifest=None):
    manifest = manifest or load_manifest()
    if role not in manifest["cameras"]:
        raise KeyError("unknown camera role '{}'; known: {}".format(role, ", ".join(manifest["cameras"])))
    return manifest["cameras"][role]


def camera_group(role, manifest=None):
    manifest = manifest or load_manifest()
    for group, roles in manifest["camera_groups"].items():
        if role in roles:
            return group
    return None


def camera_for_cut(cut, kind="desk", manifest=None):
    """Map a script.json camera_cut (e.g. 'elena_close_up') to a camera role for this segment kind."""
    manifest = manifest or load_manifest()
    table = manifest["script_cut_map"][kind]
    return table.get(cut) or manifest["default_camera"][kind]


def camera(role, manifest=None):
    _require_bpy()
    name = camera_name(role, manifest)
    obj = bpy.data.objects.get(name)
    if obj is None or obj.type != "CAMERA":
        raise LookupError("camera '{}' for role '{}' is not in this .blend".format(name, role))
    return obj


def use_camera(role, manifest=None):
    obj = camera(role, manifest)
    bpy.context.scene.camera = obj
    return obj


# ---------------------------------------------------------------- anchors

def anchor_specs(manifest=None):
    manifest = manifest or load_manifest()
    return {role: spec for role, spec in manifest["anchors"].items() if not role.startswith("_")}


def anchor(role, manifest=None):
    """Return the scene object for 'anchor_female' (Elena) or 'anchor_male' (Marcus)."""
    _require_bpy()
    specs = anchor_specs(manifest)
    if role not in specs:
        raise KeyError("unknown anchor role '{}'; known: {}".format(role, ", ".join(specs)))
    obj = bpy.data.objects.get(specs[role]["object"])
    if obj is None:
        raise LookupError("anchor '{}' object '{}' is not in this .blend".format(role, specs[role]["object"]))
    return obj


# ---------------------------------------------------------------- screens

def screen_materials(role, manifest=None):
    _require_bpy()
    manifest = manifest or load_manifest()
    spec = manifest["screens"][role]
    found = []
    for name in spec.get("materials", []):
        material = bpy.data.materials.get(name)
        if material is not None:
            found.append(material)
    prefix = spec.get("material_prefix")
    tag = spec.get("tag")
    excluded = set(spec.get("exclude_materials", []))
    for material in bpy.data.materials:
        if material in found or material.name in excluded:
            continue
        if (prefix and material.name.startswith(prefix)) or (tag and material.get("polish_v3") == tag):
            found.append(material)
    return found


def _image_node(material, node_name):
    if not material.use_nodes:
        return None
    nodes = material.node_tree.nodes
    node = nodes.get(node_name) if node_name else None
    return node or next((n for n in nodes if n.type == "TEX_IMAGE"), None)


def load_media(path):
    _require_bpy()
    if not os.path.isfile(path):
        raise FileNotFoundError(path)
    image = bpy.data.images.load(path, check_existing=True)
    if os.path.splitext(path)[1].lower() in VIDEO_EXTENSIONS:
        image.source = "MOVIE"
    return image


def set_screen_media(role, path, manifest=None):
    """Swap the image or video on every material in a screen role. Returns the material names changed."""
    manifest = manifest or load_manifest()
    spec = manifest["screens"][role]
    path = resolve(path, manifest)
    if role in manifest.get("story_visuals", {}).get("desk_screen_roles", []) and is_guest_media(path, manifest):
        raise ValueError("guest feeds never go on desk monitors ({} -> {})".format(path, role))
    image = load_media(path)
    changed = []
    for material in screen_materials(role, manifest):
        node = _image_node(material, spec.get("image_node"))
        if node is None:
            continue
        node.image = image
        if image.source == "MOVIE":
            node.image_user.frame_duration = image.frame_duration
            node.image_user.frame_start = 1
            node.image_user.use_auto_refresh = True
            node.image_user.use_cyclic = True
        material["brand_image"] = image.filepath
        changed.append(material.name)
    if not changed:
        raise LookupError("no materials found for screen role '{}'".format(role))
    return changed


def set_guest_feed(path, manifest=None):
    return set_screen_media("guest_feed", path, manifest)


def set_brand_screens(images=None, manifest=None):
    """Put brand graphics on the studio screens. images maps role -> path; defaults come from the manifest."""
    manifest = manifest or load_manifest()
    images = images or {}
    result = {}
    for role, spec in manifest["screens"].items():
        if role == "guest_feed":
            continue
        path = images.get(role) or spec.get("default_image")
        if path:
            result[role] = set_screen_media(role, path, manifest)
    return result


# ---------------------------------------------------------------- shots and rendering

def apply_render_preset(name=None, manifest=None):
    _require_bpy()
    manifest = manifest or load_manifest()
    render = manifest["render"]
    name = name or os.environ.get("ATLAS_RENDER_PRESET") or render["default_preset"]
    preset = render["presets"][name]
    scene = bpy.context.scene
    scene.render.engine = render["engine"]
    scene.render.resolution_x, scene.render.resolution_y = render["resolution"]
    scene.render.resolution_percentage = preset["resolution_percentage"]
    scene.render.fps = render["fps"]
    scene.render.image_settings.file_format = render["file_format"]
    scene.cycles.samples = preset["samples"]
    scene.cycles.use_denoising = preset.get("denoise", True)
    try:
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = render["compute_device_type"]
        prefs.get_devices()
        for device in prefs.devices:
            device.use = device.type != "CPU"
        scene.cycles.device = render["device"]
    except Exception as error:  # CPU fallback keeps headless checks working
        print("studio_api: GPU setup skipped ({})".format(error))
    return name


def set_output(frames_dir, manifest=None):
    _require_bpy()
    manifest = manifest or load_manifest()
    os.makedirs(frames_dir, exist_ok=True)
    bpy.context.scene.render.filepath = os.path.join(frames_dir, manifest["render"]["frame_name"])
    return bpy.context.scene.render.filepath


def setup_shot(kind, camera_role=None, guest_feed=None, preset=None, manifest=None):
    """Prepare a desk, interview, or bts shot. Interviews always use the interview set, never the desk."""
    manifest = manifest or load_manifest()
    if kind not in manifest["camera_groups"]:
        raise ValueError("unknown shot kind '{}'".format(kind))
    role = camera_role or manifest["default_camera"][kind]
    group = camera_group(role, manifest)
    if group != kind:
        raise ValueError("camera role '{}' belongs to '{}' shots, not '{}'".format(role, group, kind))
    if guest_feed and kind != "interview":
        raise ValueError("guests only appear on the interview guest monitor")
    obj = use_camera(role, manifest)
    if guest_feed:
        set_guest_feed(guest_feed, manifest)
    if preset:
        apply_render_preset(preset, manifest)
    return obj.name


def load_episode(episode_dir):
    with open(os.path.join(episode_dir, "script.json"), "r", encoding="utf-8") as handle:
        script = json.load(handle)
    with open(os.path.join(episode_dir, "voice_timeline.json"), "r", encoding="utf-8") as handle:
        timeline = json.load(handle)
    return script, timeline


def plan_episode(episode_dir, kind="desk", style=None, manifest=None):
    """Turn an ATLAS episode package into camera-role cuts in frames.

    Desk segments default to the manifest's default_camera_style (weekend_update); interview
    segments keep the script_cut_map. Returns the list of cuts; use plan_desk() for the full plan.
    """
    manifest = manifest or load_manifest()
    style = style or manifest.get("default_camera_style", {}).get(kind, "script_cut_map")
    if style in manifest.get("camera_styles", {}):
        return plan_desk(episode_dir, style, manifest)["cuts"]
    script, timeline = load_episode(episode_dir)
    return plan_by_cut_map(script, timeline, kind, manifest)


def plan_by_cut_map(script, timeline, kind="desk", manifest=None):
    manifest = manifest or load_manifest()
    fps = manifest["render"]["fps"]
    cuts_by_line = {}
    for segment in script["segments"]:
        for index, line in enumerate(segment["lines"]):
            cuts_by_line[(segment["segment_number"], index)] = line.get("camera_cut")
    cuts = []
    for line in timeline["lines"]:
        start = int(round(line["start_seconds"] * fps))
        end = max(start + 1, int(round((line["start_seconds"] + line["duration_seconds"]) * fps)))
        role = camera_for_cut(cuts_by_line.get((line["segment_number"], line["line_index"])), kind, manifest)
        if cuts and cuts[-1]["role"] == role:
            cuts[-1]["end"] = end
        else:
            cuts.append({"role": role, "camera": camera_name(role, manifest), "start": start, "end": end})
    return cuts


# ---------------------------------------------------------------- story visuals (desk monitors)

def is_guest_media(path, manifest=None):
    """True for guest (cartoon) media: anything under the casting dirs or what the guest TV is showing now."""
    manifest = manifest or load_manifest()
    full = os.path.normcase(os.path.abspath(path))
    for folder in manifest.get("story_visuals", {}).get("guest_media_dirs", []):
        root = os.path.normcase(os.path.abspath(resolve(folder, manifest))) + os.sep
        if full.startswith(root):
            return True
    if bpy is not None:
        guest = manifest["screens"]["guest_feed"]
        default = os.path.normcase(os.path.abspath(resolve(guest.get("default_image"), manifest) or ""))
        material = bpy.data.materials.get(guest["materials"][0])
        node = _image_node(material, guest.get("image_node")) if material else None
        if node is not None and node.image is not None:
            showing = os.path.normcase(os.path.abspath(bpy.path.abspath(node.image.filepath)))
            if showing == full and showing != default:
                return True
    return False


def anchor_role(name, manifest=None):
    """Accept 'anchor_female', 'elena', 'Counterpart', etc. and return the anchor role (None for duo)."""
    manifest = manifest or load_manifest()
    key = str(name or "").strip().lower()
    for role, spec in anchor_specs(manifest).items():
        if key in (role, spec["name"].lower(), spec.get("script_speaker", "")):
            return role
    return None


def story_monitor(anchor, manifest=None):
    manifest = manifest or load_manifest()
    role = anchor_role(anchor, manifest)
    if role is None:
        if anchor in manifest["screens"]:
            return anchor
        raise KeyError("no story monitor for '{}'".format(anchor))
    return manifest["story_visuals"]["slots"][role]


def story_fallback(manifest=None):
    manifest = manifest or load_manifest()
    visuals = manifest["story_visuals"]
    loop = resolve(visuals["fallback"], manifest)
    return loop if os.path.isfile(loop) else resolve(visuals["fallback_still"], manifest)


def set_story_visual(slot, image_or_video=None, manifest=None):
    """Show a slide, photo, clip or lower-third card on the monitor behind an anchor.

    slot: 'elena' / 'marcus' / 'anchor_female' / 'anchor_male' or a story monitor screen role.
    image_or_video None uses the CDN brand loop. Guest feeds are refused.
    """
    manifest = manifest or load_manifest()
    role = story_monitor(slot, manifest)
    return set_screen_media(role, image_or_video or story_fallback(manifest), manifest)


# ---------------------------------------------------------------- weekend_update desk style

TAG_PATTERN = re.compile(r"\[(BANTER|SETUP|WALK|REACTION\s*:\s*(\w+)|VISUAL\s*:\s*([^\]]+))\]", re.IGNORECASE)


def parse_tags(*texts):
    tags = {"banter": False, "setup": False, "walk": False, "reaction": None, "visual": None}
    for text in texts:
        if isinstance(text, (list, tuple)):
            text = " ".join("[{}]".format(t.strip("[]")) for t in text)
        for match in TAG_PATTERN.finditer(text or ""):
            word = match.group(1).upper()
            if word in ("BANTER", "SETUP", "WALK"):
                tags[word.lower()] = True
            elif word.startswith("REACTION"):
                tags["reaction"] = match.group(2).lower()
            elif word.startswith("VISUAL"):
                tags["visual"] = match.group(3).strip()
    return tags


def strip_tags(text):
    """Remove camera tags so they are never spoken by TTS or shown in captions."""
    return re.sub(r"\s{2,}", " ", TAG_PATTERN.sub("", text or "")).strip()


def _line_tags(line):
    return parse_tags(line.get("tags"), line.get("spoken_text"), line.get("broll_visual_cue"))


def _timeline_rows(script, timeline, fps):
    lines = {}
    for segment in script["segments"]:
        for index, line in enumerate(segment["lines"]):
            lines[(segment["segment_number"], index)] = line
    rows = []
    for item in timeline["lines"]:
        line = lines.get((item["segment_number"], item["line_index"]), {})
        start = int(round(item["start_seconds"] * fps))
        end = max(start + 1, int(round((item["start_seconds"] + item["duration_seconds"]) * fps)))
        rows.append({
            "segment": item["segment_number"],
            "line": item["line_index"],
            "speaker": item.get("speaker") or line.get("speaker"),
            "cut": line.get("camera_cut"),
            "tags": _line_tags(line),
            "start": start,
            "end": end,
        })
    return rows


def _append_cut(cuts, role, start, end, beat, manifest, **extra):
    if end <= start:
        return
    if cuts and cuts[-1]["role"] == role and cuts[-1]["end"] >= start and not extra.get("keep_separate"):
        cuts[-1]["end"] = end
        return
    extra.pop("keep_separate", None)
    cut = {"role": role, "camera": camera_name(role, manifest), "start": start, "end": end, "beat": beat}
    cut.update(extra)
    cuts.append(cut)


def close_gaps(cuts):
    """Hold each shot until the next cut so pauses between lines never cut on their own."""
    for current, following in zip(cuts, cuts[1:]):
        current["end"] = following["start"]
    return cuts


def plan_weekend_update(script, timeline, manifest=None, style_name="weekend_update"):
    """SNL Weekend Update coverage: anchors alternate stories, banter on the two-shot, reactions on punchlines."""
    manifest = manifest or load_manifest()
    style = manifest["camera_styles"][style_name]
    fps = manifest["render"]["fps"]
    rows = _timeline_rows(script, timeline, fps)
    order = [style["first_reader"]] + [r for r in anchor_specs(manifest) if r != style["first_reader"]]
    segments = []
    for row in rows:
        if not segments or segments[-1]["segment"] != row["segment"]:
            segments.append({"segment": row["segment"], "rows": []})
        segments[-1]["rows"].append(row)

    titles = {s["segment_number"]: s.get("segment_title", "") for s in script["segments"]}
    plan = {"style": style_name, "cuts": [], "stories": [], "banter_requests": [], "speaker_mismatches": []}
    cuts = plan["cuts"]
    first_row, last_row = rows[0] if rows else None, rows[-1] if rows else None
    story_number = 0
    story_segments = [s for s in segments if any(anchor_role(r["speaker"], manifest) for r in s["rows"]
                                                  if r is not first_row and r is not last_row)]
    for index, story in enumerate(segments):
        speakers = [anchor_role(r["speaker"], manifest) for r in story["rows"]]
        counts = {role: speakers.count(role) for role in order}
        if story not in story_segments:
            for row in story["rows"]:
                beat = "OPEN" if row is first_row else "CLOSE" if row is last_row else "BANTER"
                role = style["open_close"] if beat != "BANTER" else style["banter"]
                _append_cut(cuts, role, row["start"], row["end"], beat, manifest, segment=row["segment"], line=row["line"])
            continue
        planned = order[story_number % len(order)] if style.get("alternate_anchors_per_story", True) else None
        story_number += 1
        actual = max(order, key=lambda role: counts[role]) if any(counts.values()) else planned
        if planned and counts.get(planned, 0) < counts.get(actual, 0):
            plan["speaker_mismatches"].append({"segment": story["segment"], "planned_reader": planned, "script_reader": actual})
        reader = actual or planned
        visual = next((r["tags"]["visual"] for r in story["rows"] if r["tags"]["visual"]), None)
        visual_path = resolve(visual, manifest) if visual else None
        plan["stories"].append({
            "segment": story["segment"],
            "title": titles.get(story["segment"], ""),
            "reader": reader,
            "planned_reader": planned,
            "monitor": story_monitor(reader, manifest),
            "visual": visual_path or story_fallback(manifest),
            "visual_is_fallback": visual_path is None,
            "start": story["rows"][0]["start"],
            "end": story["rows"][-1]["end"],
        })
        first_reader_line = next((r for r, who in zip(story["rows"], speakers) if who == reader), None)
        reader_lines = sum(1 for who in speakers if who == reader)
        has_banter = False
        for position, (row, who) in enumerate(zip(story["rows"], speakers)):
            tags = row["tags"]
            if row is first_row or row is last_row:
                role, beat = style["open_close"], "OPEN" if row is first_row else "CLOSE"
            elif tags["banter"] or who is None or who != reader:
                role, beat = style["banter"], "BANTER"
                has_banter = True
            elif tags["setup"] or (row is first_reader_line and reader_lines > 1):
                role, beat = style["story_setup"], "SETUP"
            else:
                role, beat = style["story_read"][reader], "STORY"
            is_last = position == len(story["rows"]) - 1
            if (is_last and beat == "STORY" and style.get("insert_handoff_beats") and reader_lines >= 3
                    and story is not story_segments[-1]):
                role, beat = style["handoff"], "HANDOFF"
            reaction = tags["reaction"]
            if reaction and anchor_role(reaction, manifest):
                span = row["end"] - row["start"]
                hold = min(int(round(style["reaction_seconds"] * fps)), int(span * style["reaction_max_fraction"]))
                _append_cut(cuts, role, row["start"], row["end"] - hold, beat, manifest, segment=row["segment"], line=row["line"])
                _append_cut(cuts, style["reaction"][reaction], row["end"] - hold, row["end"], "REACTION", manifest,
                            segment=row["segment"], line=row["line"], keep_separate=True)
            else:
                _append_cut(cuts, role, row["start"], row["end"], beat, manifest, segment=row["segment"], line=row["line"])
        if not has_banter and story is not story_segments[-1]:
            plan["banter_requests"].append({"after_segment": story["segment"], "tag": "[BANTER]"})
    close_gaps(cuts)
    return plan


def plan_desk(episode_dir, style="weekend_update", manifest=None):
    manifest = manifest or load_manifest()
    script, timeline = load_episode(episode_dir)
    return plan_weekend_update(script, timeline, manifest, style)


# ---------------------------------------------------------------- Thursday Throwback sequence

def plan_throwback(episode_dir=None, script=None, timeline=None, manifest=None):
    """Desk open, walk to the interview corner, then interview cameras. Returns cuts plus any audio gap to pad."""
    manifest = manifest or load_manifest()
    if episode_dir:
        script, timeline = load_episode(episode_dir)
    fps = manifest["render"]["fps"]
    phases = {p["phase"]: p for p in manifest["sequences"]["thursday_throwback"]["phases"]}
    walk = phases["walk"]
    rows = _timeline_rows(script, timeline, fps)
    categories = {s["segment_number"]: str(s.get("segment_category", "")).lower() for s in script["segments"]}
    segment_ids = sorted({r["segment"] for r in rows})
    open_id = segment_ids[0] if segment_ids else None
    walk_ids = {sid for sid in segment_ids if walk["segment_category"] in categories.get(sid, "")}
    walk_ids |= {r["segment"] for r in rows if r["tags"]["walk"]}
    plan = {"sequence": "thursday_throwback", "cuts": [], "audio_gaps": [], "pending_cameras": []}
    cuts, shift = plan["cuts"], 0

    def walk_block(start, end, segment):
        split = start + max(1, int((end - start) * walk["wide_fraction"]))
        _append_cut(cuts, "throwback_walk_wide", start, split, "WALK", manifest, segment=segment)
        _append_cut(cuts, "throwback_walk_track", split, end, "WALK", manifest, segment=segment)

    for index, row in enumerate(rows):
        start, end = row["start"] + shift, row["end"] + shift
        sid = row["segment"]
        if sid == open_id and sid not in walk_ids:
            _append_cut(cuts, "desk_wide" if index == 0 else "desk_hero", start, end, "DESK_OPEN", manifest, segment=sid)
            next_sid = rows[index + 1]["segment"] if index + 1 < len(rows) else None
            if next_sid != sid and not walk_ids:
                gap = int(round(walk["insert_seconds_if_missing"] * fps))
                walk_block(end, end + gap, None)
                plan["audio_gaps"].append({"at_frame": end, "frames": gap, "seconds": walk["insert_seconds_if_missing"]})
                shift += gap
        elif sid in walk_ids:
            segment_rows = [r for r in rows if r["segment"] == sid]
            if len(segment_rows) == 1:
                walk_block(start, end, sid)
            else:
                role = "throwback_walk_wide" if row is segment_rows[0] else "throwback_walk_track"
                _append_cut(cuts, role, start, end, "WALK", manifest, segment=sid)
        else:
            role = camera_for_cut(row["cut"], "interview", manifest)
            _append_cut(cuts, role, start, end, "INTERVIEW", manifest, segment=sid)
    close_gaps(cuts)
    if bpy is not None:
        plan["pending_cameras"] = sorted({c["camera"] for c in cuts if c["camera"] not in bpy.data.objects})
    return plan


# ---------------------------------------------------------------- applying a plan in Blender

def apply_cuts(cuts, manifest=None):
    """Bind cuts to timeline markers (Blender's camera switching). Replaces earlier CUT_ markers."""
    _require_bpy()
    scene = bpy.context.scene
    for marker in [m for m in scene.timeline_markers if m.name.startswith("CUT_")]:
        scene.timeline_markers.remove(marker)
    missing = []
    for number, cut in enumerate(cuts):
        obj = bpy.data.objects.get(cut["camera"])
        if obj is None:
            missing.append(cut["camera"])
            continue
        marker = scene.timeline_markers.new("CUT_{:04d}_{}".format(number, cut["beat"]), frame=cut["start"] + 1)
        marker.camera = obj
    if cuts:
        scene.frame_start, scene.frame_end = cuts[0]["start"] + 1, cuts[-1]["end"]
    return sorted(set(missing))


def apply_story_visual(story, manifest=None):
    """Put a planned story's visual on the monitor behind its reader (call before rendering that story's frames)."""
    return set_story_visual(story["monitor"], story["visual"], manifest)


# ---------------------------------------------------------------- validation

def validate(manifest=None):
    """Check every manifest name against the open .blend and the disk. Missing scene names are 'pending'."""
    manifest = manifest or load_manifest()
    report = {"ok": [], "pending": [], "failed": []}

    def check(label, present, failure=False):
        (report["ok"] if present else report["failed" if failure else "pending"]).append(label)

    for key, relative in manifest["brand_kit"].items():
        check("brand_kit.{} -> {}".format(key, relative), os.path.exists(resolve(relative, manifest)), failure=True)
    for role, spec in manifest["screens"].items():
        if spec.get("default_image"):
            check("screens.{}.default_image".format(role), os.path.isfile(resolve(spec["default_image"], manifest)), failure=True)
    for kind, table in manifest["script_cut_map"].items():
        if kind.startswith("_"):
            continue
        for cut, role in table.items():
            check("script_cut_map.{}.{} -> {}".format(kind, cut, role), camera_group(role, manifest) == kind, failure=True)
    check("blend.canonical exists", os.path.isfile(studio_blend(manifest)), failure=True)

    if bpy is None:
        return report
    for key, name in manifest["collections"].items():
        check("collection {} ({})".format(name, key), name in bpy.data.collections)
    for role, name in manifest["cameras"].items():
        obj = bpy.data.objects.get(name)
        check("camera {} ({})".format(name, role), obj is not None and obj.type == "CAMERA")
    for role, spec in anchor_specs(manifest).items():
        check("anchor {} ({}) object {}".format(role, spec["name"], spec["object"]), spec["object"] in bpy.data.objects)
    for role, spec in manifest["screens"].items():
        materials = screen_materials(role, manifest)
        check("screen {} materials ({})".format(role, ", ".join(m.name for m in materials[:4]) or "none"), bool(materials))
        if spec.get("object"):
            check("screen {} object {}".format(role, spec["object"]), spec["object"] in bpy.data.objects)
        if materials and spec.get("image_node"):
            swappable = [m.name for m in materials if _image_node(m, spec["image_node"]) is not None]
            named = [m for m in materials if m.use_nodes and m.node_tree.nodes.get(spec["image_node"])]
            check("screen {} swappable image node on {}/{} ({} named {})".format(
                role, len(swappable), len(materials), len(named), spec["image_node"]), bool(swappable))
    style = manifest.get("camera_styles", {}).get("weekend_update")
    if style:
        low, high = style.get("eye_level_z", [0, 99])
        for role in style.get("locked_off_cameras", []):
            obj = bpy.data.objects.get(camera_name(role, manifest))
            if obj is None:
                continue
            animated = bool(obj.animation_data and obj.animation_data.action)
            check("weekend_update {} locked off".format(role), not animated)
            if role.startswith("desk_closeup"):
                z = obj.matrix_world.translation.z
                check("weekend_update {} near eye level (z={:.2f})".format(role, z), low <= z <= high)
    for role, spec in anchor_specs(manifest).items():
        if spec.get("seat_marker"):
            check("anchor {} seat marker {}".format(role, spec["seat_marker"]), spec["seat_marker"] in bpy.data.objects)
    return report
