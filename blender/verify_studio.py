"""Dry-run check that every studio_manifest.json name resolves in the studio .blend.

Static checks (brand files, cut maps) run anywhere:
    python verify_studio.py
Scene checks run in Blender on a COPY of the studio (never the live file):
    blender -b <copy>.blend --python verify_studio.py -- [--manifest PATH] [--report OUT.json]

Names missing from the scene are reported as pending (the studio may still be
in progress); only broken manifest data or missing files fail. Nothing is saved.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import studio_api as studio


def parse_args(argv):
    argv = argv[argv.index("--") + 1:] if "--" in argv else argv[1:]
    options = {"manifest": None, "report": None}
    for flag in ("--manifest", "--report"):
        if flag in argv:
            options[flag[2:]] = argv[argv.index(flag) + 1]
    return options


def exercise_api(manifest, report):
    """Run the helpers against the open copy without saving, recording what worked."""
    def attempt(label, func):
        try:
            func()
            report["ok"].append("api " + label)
        except LookupError as error:
            report["pending"].append("api {}: {}".format(label, error))
        except Exception as error:
            report["failed"].append("api {}: {}: {}".format(label, type(error).__name__, error))

    attempt("setup_shot desk", lambda: studio.setup_shot("desk", manifest=manifest))
    attempt("setup_shot interview + guest feed", lambda: studio.setup_shot(
        "interview", guest_feed=manifest["screens"]["guest_feed"]["default_image"], manifest=manifest))
    attempt("set_brand_screens defaults", lambda: studio.set_brand_screens(manifest=manifest))
    attempt("apply_render_preset preview", lambda: studio.apply_render_preset("preview", manifest))

    def interview_at_desk_is_rejected():
        try:
            studio.setup_shot("interview", camera_role="desk_hero", manifest=manifest)
        except ValueError:
            return
        raise AssertionError("an interview was allowed on a desk camera")

    def guest_at_desk_is_rejected():
        try:
            studio.setup_shot("desk", guest_feed="x.png", manifest=manifest)
        except ValueError:
            return
        raise AssertionError("a guest feed was allowed on a desk shot")

    attempt("rule: interviews never use desk cameras", interview_at_desk_is_rejected)
    attempt("rule: guests only on the interview monitor", guest_at_desk_is_rejected)


def sample_episode(folder):
    """Tiny synthetic episode: open, an Elena story with a visual and punchline, a Marcus story with banter, close."""
    os.makedirs(folder, exist_ok=True)
    plan = [
        (1, "Open", "open", [("Duo", "Good evening.", "wide_two_shot")]),
        (2, "Story A", "news", [("Counterpart", "Setup. [VISUAL: news_intro/assets/brand_screens/brand_led_worldmap.png]", "elena_close_up"),
                                 ("Counterpart", "Detail.", "elena_close_up"),
                                 ("Counterpart", "Punchline. [REACTION:marcus]", "elena_close_up")]),
        (3, "Story B", "news", [("Enthusiast", "Setup.", "marcus_close_up"), ("Enthusiast", "Detail.", "marcus_close_up"),
                                 ("Counterpart", "[BANTER] Sure, Marcus.", "wide_two_shot")]),
        (4, "Close", "close", [("Duo", "Good night.", "wide_two_shot")]),
    ]
    segments, lines, clock = [], [], 0.0
    for number, title, category, rows in plan:
        script_lines = []
        for index, (speaker, text, cut) in enumerate(rows):
            script_lines.append({"speaker": speaker, "camera_cut": cut, "spoken_text": text, "broll_visual_cue": None})
            lines.append({"segment_number": number, "line_index": index, "speaker": speaker, "file": "x.wav",
                          "start_seconds": clock, "duration_seconds": 4.0})
            clock += 4.2
        segments.append({"segment_number": number, "segment_title": title, "segment_category": category, "lines": script_lines})
    with open(os.path.join(folder, "script.json"), "w", encoding="utf-8") as handle:
        json.dump({"segments": segments}, handle)
    with open(os.path.join(folder, "voice_timeline.json"), "w", encoding="utf-8") as handle:
        json.dump({"lines": lines}, handle)
    return folder


def exercise_styles(manifest, report, folder):
    def attempt(label, func):
        try:
            func()
            report["ok"].append("style " + label)
        except LookupError as error:
            report["pending"].append("style {}: {}".format(label, error))
        except Exception as error:
            report["failed"].append("style {}: {}: {}".format(label, type(error).__name__, error))

    episode = sample_episode(folder)
    plan = studio.plan_desk(episode, manifest=manifest)

    def weekend_update_shape():
        beats = [c["beat"] for c in plan["cuts"]]
        roles = [c["role"] for c in plan["cuts"]]
        assert beats[0] == "OPEN" and beats[-1] == "CLOSE", beats
        assert "desk_wide" not in roles[1:-1], "wide used outside the open and close"
        assert {"SETUP", "STORY", "BANTER", "REACTION"} <= set(beats), beats
        assert [s["reader"] for s in plan["stories"]] == ["anchor_female", "anchor_male"]
        assert all(a["end"] == b["start"] for a, b in zip(plan["cuts"], plan["cuts"][1:]))
        assert studio.plan_episode(episode, manifest=manifest)[0]["role"] == "desk_wide", "weekend_update is not the desk default"
        assert studio.plan_episode(episode, kind="interview", manifest=manifest)[0]["role"].startswith("interview_")

    attempt("weekend_update plan (open/setup/story/banter/reaction, alternating readers)", weekend_update_shape)
    if studio.bpy is None:
        return

    def visuals_follow_reader():
        for story in plan["stories"]:
            studio.apply_story_visual(story, manifest)
        studio.set_story_visual("marcus", None, manifest)

    def guest_never_on_desk():
        casting = studio.resolve(manifest["story_visuals"]["guest_media_dirs"][0], manifest)
        clip = next((os.path.join(casting, f) for f in os.listdir(casting)), None) if os.path.isdir(casting) else None
        if clip is None:
            raise LookupError("no casting media to test with")
        try:
            studio.set_story_visual("elena", clip, manifest)
        except ValueError:
            return
        raise AssertionError("guest media was allowed on a desk monitor")

    attempt("set_story_visual on both reader monitors (+ brand loop fallback)", visuals_follow_reader)
    attempt("rule: guest feeds never on desk monitors", guest_never_on_desk)
    attempt("apply_cuts timeline markers", lambda: studio.apply_cuts(plan["cuts"], manifest))

    def throwback():
        tb = studio.plan_throwback(episode, manifest=manifest)
        phases = [c["beat"] for c in tb["cuts"]]
        assert phases[0] == "DESK_OPEN" and "WALK" in phases and phases[-1] == "INTERVIEW", phases
        if tb["pending_cameras"]:
            raise LookupError("throwback cameras pending: " + ", ".join(tb["pending_cameras"]))

    attempt("thursday_throwback sequence (desk open -> walk -> interview)", throwback)


def main():
    options = parse_args(sys.argv)
    manifest = studio.load_manifest(options["manifest"])
    report = studio.validate(manifest)
    report["manifest"] = manifest["_path"]
    report["mode"] = "static"
    if studio.bpy is not None:
        report["mode"] = "blender"
        report["blend_opened"] = studio.bpy.data.filepath
        if os.path.normcase(os.path.abspath(studio.bpy.data.filepath or "")) == os.path.normcase(studio.studio_blend(manifest)):
            report["failed"].append("verify must run on a copy, not the live studio file")
        else:
            exercise_api(manifest, report)
    exercise_styles(manifest, report, os.path.join(os.environ.get("TEMP", "/tmp"), "studio_verify_episode"))
    for section in ("ok", "pending", "failed"):
        print("[{}] {}".format(section.upper(), len(report[section])))
        if section != "ok":
            for item in report[section]:
                print("   - " + item)
    if options["report"]:
        with open(options["report"], "w", encoding="utf-8") as handle:
            json.dump(report, handle, indent=2)
    print("STUDIO_VERIFY {}".format("FAIL" if report["failed"] else "PASS"))
    sys.exit(1 if report["failed"] else 0)


if __name__ == "__main__":
    main()
