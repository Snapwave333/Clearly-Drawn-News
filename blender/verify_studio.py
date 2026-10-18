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
