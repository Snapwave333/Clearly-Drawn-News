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
    for material in bpy.data.materials:
        if material in found:
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
    image = load_media(resolve(path, manifest))
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


def plan_episode(episode_dir, kind="desk", manifest=None):
    """Turn an ATLAS episode package (script.json + voice_timeline.json) into camera-role cuts in frames."""
    manifest = manifest or load_manifest()
    fps = manifest["render"]["fps"]
    with open(os.path.join(episode_dir, "script.json"), "r", encoding="utf-8") as handle:
        script = json.load(handle)
    with open(os.path.join(episode_dir, "voice_timeline.json"), "r", encoding="utf-8") as handle:
        timeline = json.load(handle)
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
    for anchor, spec in manifest["anchors"].items():
        check("anchor {} armature {}".format(anchor, spec["armature"]), spec["armature"] in bpy.data.objects)
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
    return report
