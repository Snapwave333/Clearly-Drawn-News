import json
import os
import runpy
import sys
import traceback

import unreal

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import clearly_drawn_lipsync as lipsync
import clearly_drawn_newsroom_sequence as newsroom
import clearly_drawn_wardrobe as wardrobe

STATE = {}
RENDER_PRESETS = {"fast": (1, 1), "balanced": (1, 4), "quality": (1, 8)}


def env(name, default=None):
    value = os.environ.get(name, "").strip()
    return value or default


def write_handoff(payload):
    path = env("CLEARLY_DRAWN_HANDOFF")
    if not path:
        return
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2)


def finish(payload):
    write_handoff(payload)
    unreal.log("Clearly Drawn Studios production handoff: {}".format(json.dumps(payload)))
    if env("CLEARLY_DRAWN_KEEP_EDITOR_OPEN") != "1":
        unreal.SystemLibrary.quit_editor()


def run_studio_setup(report):
    setup = os.path.join(unreal.Paths.project_content_dir(), "Python", "setup_newsroom_studio.py")
    if env("CLEARLY_DRAWN_RUN_SETUP", "1") == "1" and os.path.isfile(setup):
        runpy.run_path(setup, run_name="__main__")
        report["studio_setup"] = setup


def lipsync_status():
    if hasattr(unreal, "MetaHumanPerformance"):
        return "MetaHuman Animator is installed; audio-driven facial animation is not automated by this script yet"
    return "MetaHuman Animator not found; anchors will not lip-sync"


def configure_job(sequence_path, map_path, frames_dir, width, height):
    subsystem = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
    queue = subsystem.get_queue()
    for existing in list(queue.get_jobs()):
        queue.delete_job(existing)
    job = queue.allocate_new_job(unreal.MoviePipelineExecutorJob)
    job.set_editor_property("sequence", unreal.SoftObjectPath(sequence_path))
    job.set_editor_property("map", unreal.SoftObjectPath(map_path))
    job.set_editor_property("job_name", os.path.basename(sequence_path.split(".")[0]))
    config = job.get_configuration()
    output = config.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
    output.set_editor_property("output_directory", unreal.DirectoryPath(frames_dir))
    output.set_editor_property("output_resolution", unreal.IntPoint(width, height))
    output.set_editor_property("file_name_format", "frame.{frame_number}")
    output.set_editor_property("zero_pad_frame_numbers", 6)
    output.set_editor_property("override_existing_output", True)
    config.find_or_add_setting_by_class(unreal.MoviePipelineDeferredPassBase)
    config.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_PNG)
    spatial, temporal = RENDER_PRESETS.get(env("CLEARLY_DRAWN_RENDER_PRESET", "fast"), RENDER_PRESETS["fast"])
    anti_aliasing = config.find_or_add_setting_by_class(unreal.MoviePipelineAntiAliasingSetting)
    anti_aliasing.set_editor_property("spatial_sample_count", int(env("CLEARLY_DRAWN_SPATIAL_SAMPLES", str(spatial))))
    anti_aliasing.set_editor_property("temporal_sample_count", int(env("CLEARLY_DRAWN_TEMPORAL_SAMPLES", str(temporal))))
    return subsystem


def on_render_finished(executor, success):
    report = STATE.get("report", {})
    report["render_success"] = bool(success)
    report["ok"] = bool(success)
    if not success:
        report["error"] = "Movie Render Queue reported a failed render; see the Unreal log"
    finish(report)


def main():
    report = {"ok": False}
    STATE["report"] = report
    try:
        episode_dir = env("CLEARLY_DRAWN_EPISODE")
        frames_dir = env("CLEARLY_DRAWN_FRAMES")
        if not episode_dir or not frames_dir:
            raise RuntimeError("CLEARLY_DRAWN_EPISODE and CLEARLY_DRAWN_FRAMES must be set")
        width = int(env("CLEARLY_DRAWN_WIDTH", "1920"))
        height = int(env("CLEARLY_DRAWN_HEIGHT", "1080"))

        map_path = env("CLEARLY_DRAWN_MAP")
        if map_path:
            unreal.EditorLoadingAndSavingUtils.load_map(map_path)
        else:
            world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
            map_path = world.get_path_name().split(".")[0]
        report["map"] = map_path

        run_studio_setup(report)
        report["wardrobe"] = [
            {"actor": a["actor"], "look": a["look"], "slots": a["slots"]}
            for a in wardrobe.dress(True, wardrobe.locate_spec(None))["actors"]
        ]
        sequence_report = newsroom.build(episode_dir, True, newsroom.locate_config(None))
        report["sequence"] = sequence_report["sequence"]
        report["cameras"] = sequence_report["cameras"]
        report["unmatched_cameras"] = sequence_report["unmatched_cameras"]
        report["duration_seconds"] = sequence_report["duration_seconds"]
        sequence_asset = unreal.EditorAssetLibrary.load_asset(report["sequence"].split(".")[0])
        lip = lipsync.apply(sequence_asset, episode_dir) if env("CLEARLY_DRAWN_LIPSYNC", "1") == "1" else {"applied": False, "reason": "disabled"}
        report["lipsync_detail"] = lip
        report["lipsync"] = (
            "procedural jaw animation: " + ", ".join("{} {}".format(k, v) for k, v in lip.get("anchors", {}).items())
            if lip.get("applied")
            else "not applied ({}); {}".format(lip.get("reason") or lip.get("anchors"), lipsync_status())
        )
        unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)

        os.makedirs(frames_dir, exist_ok=True)
        report["frames_dir"] = frames_dir
        report["resolution"] = [width, height]
        report["render_preset"] = env("CLEARLY_DRAWN_RENDER_PRESET", "fast")
        subsystem = configure_job(report["sequence"], map_path, frames_dir, width, height)
        executor = subsystem.render_queue_with_executor(unreal.MoviePipelinePIEExecutor)
        STATE["executor"] = executor
        executor.on_executor_finished_delegate.add_callable_unique(on_render_finished)
        unreal.log("Clearly Drawn Studios: Movie Render Queue started for {}".format(report["sequence"]))
    except Exception as error:
        report["error"] = "{}: {}".format(type(error).__name__, error)
        report["traceback"] = traceback.format_exc()
        finish(report)


main()
