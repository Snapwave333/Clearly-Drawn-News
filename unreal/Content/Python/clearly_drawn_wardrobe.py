import json
import os
import re
import sys

import unreal

MIC_FOLDER = "/Game/ClearlyDrawn/Wardrobe"
SLOT_COMPONENTS = {"torso": "torso", "legs": "legs", "feet": "feet"}
NON_CLOTHING = {"body", "face", "head", "groom", "hair", "eyebrows", "eyelashes", "beard", "mustache", "peachfuzz"}
BODY_TYPE = re.compile(r"^([mf])_(srt|med|tal)_(nrw|ovw|unw)")
SLOT_ALIASES = {"torso": ["top"], "legs": ["btm"], "feet": ["shs"]}
PRIMARY_PARAMS = ("color", "tint", "albedo", "diffuse", "base")
SECONDARY_PARAMS = ("secondary", "accent", "trim", "detail")


def parse_args(argv):
    apply = "--apply" in argv
    spec = os.environ.get("CLEARLY_DRAWN_WARDROBE")
    if "--spec" in argv:
        spec = argv[argv.index("--spec") + 1]
    return apply, spec


def locate_spec(explicit):
    here = os.path.dirname(os.path.abspath(__file__))
    candidates = [explicit] if explicit else []
    candidates += [
        os.path.join(here, "wardrobe.json"),
        os.path.join(here, "..", "..", "wardrobe.json"),
        os.path.join(unreal.Paths.project_dir(), "wardrobe.json"),
    ]
    for path in candidates:
        if path and os.path.isfile(path):
            return os.path.abspath(path)
    raise FileNotFoundError("wardrobe.json not found; pass --spec PATH or set CLEARLY_DRAWN_WARDROBE")


def tokens(name):
    spaced = re.sub(r"([a-z])([A-Z])", r"\1 \2", name)
    return [t for t in re.split(r"[^a-z0-9]+", spaced.lower()) if t]


def body_type(name):
    match = BODY_TYPE.match(name.lower())
    return match.group(0) if match else None


def is_banned(name_tokens, banned):
    for token in name_tokens:
        for word in banned:
            if token == word or (len(word) >= 4 and token.startswith(word)):
                return True
    return False


def skeletal_mesh_assets():
    registry = unreal.AssetRegistryHelpers.get_asset_registry()
    try:
        assets = registry.get_assets_by_class(unreal.TopLevelAssetPath("/Script/Engine", "SkeletalMesh"), True)
    except (AttributeError, TypeError):
        assets = registry.get_assets_by_class("SkeletalMesh", True)
    return [(str(a.asset_name), "{}.{}".format(a.package_name, a.asset_name)) for a in assets]


def classify_wardrobe(spec):
    slot_keywords = spec["slot_keywords"]
    banned = spec["banned_keywords"]
    catalog = {slot: [] for slot in slot_keywords}
    rejected = []
    for name, path in skeletal_mesh_assets():
        name_tokens = tokens(name)
        if NON_CLOTHING.intersection(name_tokens):
            continue
        for slot, keywords in slot_keywords.items():
            if not set(keywords + SLOT_ALIASES.get(slot, [])).intersection(name_tokens):
                continue
            if is_banned(name_tokens, banned):
                rejected.append(path)
            else:
                catalog[slot].append({"name": name, "path": path, "tokens": name_tokens, "body_type": body_type(name)})
            break
    return catalog, rejected


def pick_mesh(candidates, prefer, wearer_body_type):
    best, best_score = None, 0
    for item in candidates:
        if wearer_body_type and item["body_type"] and item["body_type"] != wearer_body_type:
            continue
        score = 0
        for rank, word in enumerate(prefer):
            if any(t.startswith(word) for t in item["tokens"]):
                score += (len(prefer) - rank) * 10
        if wearer_body_type and item["body_type"] == wearer_body_type:
            score += 5
        if score > best_score:
            best, best_score = item, score
    return best


def srgb_to_linear(hex_color):
    value = hex_color.lstrip("#")
    channels = [int(value[i:i + 2], 16) / 255.0 for i in (0, 2, 4)]
    linear = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in channels]
    return unreal.LinearColor(linear[0], linear[1], linear[2], 1.0)


def metahuman_actors():
    subsystem = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    found = []
    for actor in subsystem.get_all_level_actors():
        components = {c.get_name().lower(): c for c in actor.get_components_by_class(unreal.SkeletalMeshComponent)}
        if "body" in components and "face" in components:
            found.append((actor, components))
    return found


def component_mesh(component):
    for prop in ("skeletal_mesh_asset", "skeletal_mesh"):
        try:
            return component.get_editor_property(prop)
        except Exception:
            continue
    return None


def set_component_mesh(component, mesh):
    for prop in ("skeletal_mesh_asset", "skeletal_mesh"):
        try:
            component.set_editor_property(prop, mesh)
            return
        except Exception:
            continue
    raise RuntimeError("Unable to assign skeletal mesh on {}".format(component.get_name()))


def follow_body(component, body):
    for method in ("set_leader_pose_component", "set_master_pose_component"):
        if hasattr(component, method):
            getattr(component, method)(body)
            return


def tinted_instance(parent, label, slot, index, colors):
    lib = unreal.MaterialEditingLibrary
    names = [str(n) for n in lib.get_vector_parameter_names(parent)]
    primary = [n for n in names if any(k in n.lower() for k in PRIMARY_PARAMS) and not any(k in n.lower() for k in SECONDARY_PARAMS)]
    secondary = [n for n in names if any(k in n.lower() for k in SECONDARY_PARAMS)]
    if not primary:
        return None
    asset_name = "MI_{}_{}_{}".format(re.sub(r"[^A-Za-z0-9]", "", label), slot, index)
    asset_path = "{}/{}".format(MIC_FOLDER, asset_name)
    if unreal.EditorAssetLibrary.does_asset_exist(asset_path):
        mic = unreal.EditorAssetLibrary.load_asset(asset_path)
    else:
        mic = unreal.AssetToolsHelpers.get_asset_tools().create_asset(
            asset_name, MIC_FOLDER, unreal.MaterialInstanceConstant, unreal.MaterialInstanceConstantFactoryNew()
        )
    lib.set_material_instance_parent(mic, parent)
    for name in primary:
        lib.set_material_instance_vector_parameter_value(mic, name, srgb_to_linear(colors["primary"]))
    if colors.get("secondary"):
        for name in secondary:
            lib.set_material_instance_vector_parameter_value(mic, name, srgb_to_linear(colors["secondary"]))
    unreal.EditorAssetLibrary.save_loaded_asset(mic)
    return mic


def look_for(actor, index, spec):
    label = actor.get_actor_label().lower()
    for character in spec["characters"]:
        if any(m in label for m in character["match"]):
            return character
    looks = spec["staff_looks"]
    return looks[index % len(looks)]


def dress(apply, spec_path):
    with open(spec_path, "r", encoding="utf-8") as handle:
        spec = json.load(handle)
    catalog, rejected = classify_wardrobe(spec)
    report = {
        "spec": spec_path,
        "applied": apply,
        "catalog": {slot: [i["path"] for i in items] for slot, items in catalog.items()},
        "rejected_by_dress_code": rejected,
        "actors": [],
    }
    staff_index = 0
    for actor, components in metahuman_actors():
        label = actor.get_actor_label()
        look = look_for(actor, staff_index, spec)
        if look in spec["staff_looks"]:
            staff_index += 1
        body_mesh = component_mesh(components["body"])
        wearer = body_type(body_mesh.get_name()) if body_mesh else None
        entry = {"actor": label, "look": look["look"], "body_type": wearer, "slots": {}}
        if apply:
            actor.modify()
        for slot, component_name in SLOT_COMPONENTS.items():
            component = components.get(component_name)
            choice = pick_mesh(catalog.get(slot, []), look[slot]["prefer"], wearer)
            if component is None or choice is None:
                entry["slots"][slot] = "no matching component or dress-code clothing found"
                continue
            entry["slots"][slot] = choice["path"]
            if not apply:
                continue
            mesh = unreal.EditorAssetLibrary.load_asset(choice["path"])
            component.modify()
            set_component_mesh(component, mesh)
            follow_body(component, components["body"])
            for i, parent in enumerate(component.get_materials()):
                if parent is None:
                    continue
                mic = tinted_instance(parent, label, slot, i, look[slot])
                if mic is not None:
                    component.set_material(i, mic)
        report["actors"].append(entry)

    out_dir = os.path.join(unreal.Paths.project_saved_dir(), "ClearlyDrawn")
    os.makedirs(out_dir, exist_ok=True)
    out_path = os.path.join(out_dir, "wardrobe_report.json")
    with open(out_path, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)

    mode = "APPLIED" if apply else "DRY RUN (pass --apply to dress)"
    unreal.log("Clearly Drawn Studios wardrobe {}: {} MetaHumans, report at {}".format(mode, len(report["actors"]), out_path))
    for entry in report["actors"]:
        unreal.log("  {} -> {}: {}".format(entry["actor"], entry["look"], entry["slots"]))
    if rejected:
        unreal.log_warning("Excluded by dress code: {}".format(", ".join(rejected)))
    return report


if __name__ == "__main__":
    apply_changes, explicit_spec = parse_args(sys.argv[1:])
    dress(apply_changes, locate_spec(explicit_spec))
