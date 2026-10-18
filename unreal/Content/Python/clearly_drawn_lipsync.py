import json
import os

import unreal

FACE_RIG = os.environ.get("CLEARLY_DRAWN_FACE_RIG", "/Game/MetaHumans/Common/Face/Face_ControlBoard_CtrlRig")
JAW_CONTROL = os.environ.get("CLEARLY_DRAWN_JAW_CONTROL", "CTRL_C_jaw")
JAW_SCALE = float(os.environ.get("CLEARLY_DRAWN_JAW_SCALE", "0.6"))
KEY_THRESHOLD = 0.04


def keyframes(values, threshold=KEY_THRESHOLD):
    if not values:
        return []
    keys = [(0, values[0])]
    for index in range(1, len(values) - 1):
        previous, current, following = values[index - 1], values[index], values[index + 1]
        extremum = (current > previous and current >= following) or (current < previous and current <= following)
        if extremum or abs(current - keys[-1][1]) >= threshold:
            keys.append((index, current))
    if len(values) > 1:
        keys.append((len(values) - 1, values[-1]))
    return keys


def face_rig_class():
    name = FACE_RIG.rsplit("/", 1)[-1]
    rig = unreal.load_object(None, "{}.{}_C".format(FACE_RIG, name))
    if rig is None:
        raise RuntimeError("Face control rig not found at {}".format(FACE_RIG))
    return rig


def anchor_faces():
    subsystem = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    faces = {}
    for actor in subsystem.get_all_level_actors():
        label = actor.get_actor_label().lower()
        for anchor in ("marcus", "elena"):
            if anchor in label and anchor not in faces:
                for component in actor.get_components_by_class(unreal.SkeletalMeshComponent):
                    if component.get_name().lower() == "face":
                        faces[anchor] = (actor, component)
    return faces


def rig_for_binding(sequence, binding):
    for proxy in unreal.ControlRigSequencerLibrary.get_control_rigs(sequence):
        if proxy.proxy.get_id() == binding.get_id():
            return proxy.control_rig
    return None


def apply(sequence, episode_dir):
    path = os.path.join(episode_dir, "lipsync.json")
    if not os.path.isfile(path):
        return {"applied": False, "reason": "lipsync.json not found"}
    with open(path, "r", encoding="utf-8") as handle:
        data = json.load(handle)
    world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
    rig_class = face_rig_class()
    faces = anchor_faces()
    result = {"applied": False, "control": JAW_CONTROL, "scale": JAW_SCALE, "anchors": {}}
    for anchor, values in data["tracks"].items():
        if anchor not in faces:
            result["anchors"][anchor] = "no actor labelled '{}' with a Face component".format(anchor)
            continue
        try:
            _, face = faces[anchor]
            binding = sequence.add_possessable(face)
            unreal.ControlRigSequencerLibrary.find_or_create_control_rig_track(world, sequence, rig_class, binding)
            rig = rig_for_binding(sequence, binding)
            if rig is None:
                raise RuntimeError("control rig track was not created")
            keys = keyframes(values)
            for frame, value in keys:
                unreal.ControlRigSequencerLibrary.set_local_control_rig_vector2d(
                    sequence, rig, JAW_CONTROL, unreal.FrameNumber(frame), unreal.Vector2D(0.0, value * JAW_SCALE), True
                )
            result["anchors"][anchor] = "{} jaw keys".format(len(keys))
            result["applied"] = True
        except Exception as error:
            result["anchors"][anchor] = "{}: {}".format(type(error).__name__, error)
    return result
