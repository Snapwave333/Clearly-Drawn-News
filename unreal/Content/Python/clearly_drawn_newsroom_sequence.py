import json
import os
import sys

import unreal


def parse_args(argv):
    if not argv or argv[0].startswith("--"):
        raise SystemExit("usage: clearly_drawn_newsroom_sequence.py <episode_dir> [--apply] [--config PATH]")
    config = None
    if "--config" in argv:
        config = argv[argv.index("--config") + 1]
    return argv[0], "--apply" in argv, config


def locate_config(explicit):
    here = os.path.dirname(os.path.abspath(__file__))
    for path in [explicit, os.path.join(here, "newsroom_cameras.json"), os.path.join(here, "..", "..", "newsroom_cameras.json")]:
        if path and os.path.isfile(path):
            return os.path.abspath(path)
    raise FileNotFoundError("newsroom_cameras.json not found; pass --config PATH")


def load_json(path):
    with open(path, "r", encoding="utf-8") as handle:
        return json.load(handle)


def to_frame(seconds, fps):
    return int(round(seconds * fps))


def plan_timeline(script, timeline, fps, fallback_camera):
    cameras = {}
    for segment in script["segments"]:
        for index, line in enumerate(segment["lines"]):
            cameras[(segment["segment_number"], index)] = line.get("camera_cut") or fallback_camera
    clips = []
    cuts = []
    for line in timeline["lines"]:
        start = to_frame(line["start_seconds"], fps)
        end = max(start + 1, to_frame(line["start_seconds"] + line["duration_seconds"], fps))
        camera = cameras.get((line["segment_number"], line["line_index"]), fallback_camera)
        clips.append({"file": line["file"], "speaker": line["speaker"], "start": start, "end": end})
        if cuts and cuts[-1]["camera"] == camera and cuts[-1]["end"] >= start:
            cuts[-1]["end"] = end
        else:
            if cuts:
                cuts[-1]["end"] = start
            cuts.append({"camera": camera, "start": start, "end": end})
    return clips, cuts


def camera_actors():
    subsystem = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
    return [actor for actor in subsystem.get_all_level_actors() if isinstance(actor, unreal.CineCameraActor)]


def match_cameras(config, actors):
    labels = [(actor, actor.get_actor_label().lower()) for actor in actors]
    resolved = {}
    for cut, keywords in config["cameras"].items():
        for keyword in keywords:
            found = next((actor for actor, label in labels if keyword in label), None)
            if found is not None:
                resolved[cut] = found
                break
    fallback = resolved.get(config["fallback_camera"]) or (actors[0] if actors else None)
    return resolved, fallback


def import_audio(files, destination):
    tasks = []
    for path in files:
        task = unreal.AssetImportTask()
        task.set_editor_property("filename", path)
        task.set_editor_property("destination_path", destination)
        task.set_editor_property("automated", True)
        task.set_editor_property("replace_existing", True)
        task.set_editor_property("save", True)
        tasks.append(task)
    unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks(tasks)
    sounds = {}
    for path, task in zip(files, tasks):
        imported = [p for p in task.get_editor_property("imported_object_paths")]
        if imported:
            sounds[path] = unreal.EditorAssetLibrary.load_asset(imported[0])
    return sounds


def add_track(sequence, track_class):
    for method in ("add_track", "add_master_track"):
        if hasattr(sequence, method):
            return getattr(sequence, method)(track_class)
    raise RuntimeError("This Unreal version cannot add sequence tracks from Python")


def binding_id(sequence, binding):
    if hasattr(unreal.MovieSceneSequenceExtensions, "get_portable_binding_id"):
        return unreal.MovieSceneSequenceExtensions.get_portable_binding_id(sequence, sequence, binding)
    return sequence.make_binding_id(binding, unreal.MovieSceneObjectBindingSpace.LOCAL)


def create_sequence(name, folder, fps, end_frame):
    path = "{}/{}".format(folder, name)
    if unreal.EditorAssetLibrary.does_asset_exist(path):
        unreal.EditorAssetLibrary.delete_asset(path)
    sequence = unreal.AssetToolsHelpers.get_asset_tools().create_asset(
        name, folder, unreal.LevelSequence, unreal.LevelSequenceFactoryNew()
    )
    sequence.set_display_rate(unreal.FrameRate(fps, 1))
    sequence.set_playback_start(0)
    sequence.set_playback_end(end_frame)
    return sequence


def build(episode_dir, apply, config_path):
    config = load_json(config_path)
    fps = int(config["fps"])
    script = load_json(os.path.join(episode_dir, "script.json"))
    timeline = load_json(os.path.join(episode_dir, "voice_timeline.json"))
    episode_id = os.path.basename(os.path.normpath(episode_dir)).replace("-", "_")
    clips, cuts = plan_timeline(script, timeline, fps, config["fallback_camera"])
    end_frame = (clips[-1]["end"] if clips else 0) + to_frame(config["tail_seconds"], fps)
    if cuts:
        cuts[-1]["end"] = end_frame

    resolved, fallback = match_cameras(config, camera_actors())
    report = {
        "episode_dir": episode_dir,
        "applied": apply,
        "fps": fps,
        "duration_seconds": end_frame / float(fps),
        "audio_clips": len(clips),
        "camera_cuts": len(cuts),
        "cameras": {cut: actor.get_actor_label() for cut, actor in resolved.items()},
        "unmatched_cameras": sorted(set(c["camera"] for c in cuts) - set(resolved)),
        "fallback_camera": fallback.get_actor_label() if fallback else None,
    }

    if apply:
        if fallback is None:
            raise RuntimeError("No CineCameraActor found in the open level")
        missing = [clip["file"] for clip in clips if not os.path.isfile(clip["file"])]
        if missing:
            raise FileNotFoundError("Missing narration files, e.g. {}".format(missing[0]))
        folder = config["sequence_folder"]
        sequence = create_sequence("LS_{}".format(episode_id), folder, fps, end_frame)
        sounds = import_audio([clip["file"] for clip in clips], "{}/{}".format(config["audio_folder"], episode_id))
        tracks = {}
        for clip in clips:
            sound = sounds.get(clip["file"])
            if sound is None:
                continue
            track = tracks.get(clip["speaker"])
            if track is None:
                track = add_track(sequence, unreal.MovieSceneAudioTrack)
                track.set_display_name("VO {}".format(clip["speaker"].title()))
                tracks[clip["speaker"]] = track
            section = track.add_section()
            section.set_editor_property("sound", sound)
            section.set_range(clip["start"], clip["end"])
        cut_track = add_track(sequence, unreal.MovieSceneCameraCutTrack)
        bindings = {}
        for cut in cuts:
            actor = resolved.get(cut["camera"], fallback)
            label = actor.get_actor_label()
            if label not in bindings:
                bindings[label] = sequence.add_possessable(actor)
            section = cut_track.add_section()
            section.set_range(cut["start"], cut["end"])
            section.set_camera_binding_id(binding_id(sequence, bindings[label]))
        unreal.EditorAssetLibrary.save_loaded_asset(sequence)
        report["sequence"] = sequence.get_path_name()

    out_dir = os.path.join(unreal.Paths.project_saved_dir(), "ClearlyDrawn")
    os.makedirs(out_dir, exist_ok=True)
    out_path = os.path.join(out_dir, "sequence_report_{}.json".format(episode_id))
    with open(out_path, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
    mode = "BUILT" if apply else "DRY RUN (pass --apply to build)"
    unreal.log("Clearly Drawn News sequence {}: {} clips, {} cuts, {:.1f} min, report at {}".format(
        mode, len(clips), len(cuts), report["duration_seconds"] / 60.0, out_path))
    if report["unmatched_cameras"]:
        unreal.log_warning("Camera cuts using the fallback camera: {}".format(", ".join(report["unmatched_cameras"])))
    return report


if __name__ == "__main__":
    episode, apply_changes, explicit_config = parse_args(sys.argv[1:])
    build(os.path.abspath(episode), apply_changes, locate_config(explicit_config))
