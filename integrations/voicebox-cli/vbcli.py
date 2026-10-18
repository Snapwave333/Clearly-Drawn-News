#!/usr/bin/env python3
"""Voicebox CLI - thin wrappers over the local Voicebox REST API (v0.5.0).

Every endpoint used here was checked against jamiepine/voicebox v0.5.0
backend/routes/*.py. Stdlib only (Python 3.9+). ffmpeg is used only for --speed.

Server: http://127.0.0.1:17493 (override with VOICEBOX_URL). The desktop app
(C:\\Users\\chrom\\Tools\\voicebox\\voicebox.exe) starts the server.
"""
import argparse, json, os, re, shutil, subprocess, sys, time, uuid, wave
import urllib.request, urllib.error, urllib.parse
from pathlib import Path

BASE = os.environ.get("VOICEBOX_URL", "http://127.0.0.1:17493").rstrip("/")
APP_EXE = os.environ.get("VOICEBOX_APP", r"C:\Users\chrom\Tools\voicebox\voicebox.exe")
LANGS = "zh en ja ko de fr ru pt es it he ar da el fi hi ms nl no pl sv sw tr".split()
ENGINES = ["qwen", "qwen_custom_voice", "luxtts", "chatterbox", "chatterbox_turbo", "tada", "kokoro"]
CLONING_ENGINES = ["qwen", "luxtts", "chatterbox", "chatterbox_turbo", "tada"]


class VBError(Exception):
    pass


# ---------------------------------------------------------------- HTTP helpers
def _req(method, path, body=None, headers=None, timeout=60, raw=False):
    url = BASE + path
    data = None
    hdrs = dict(headers or {})
    if body is not None and not isinstance(body, (bytes, bytearray)):
        data = json.dumps(body).encode()
        hdrs.setdefault("Content-Type", "application/json")
    elif body is not None:
        data = bytes(body)
    r = urllib.request.Request(url, data=data, method=method, headers=hdrs)
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            payload = resp.read()
            if raw:
                return payload, resp.headers
            return json.loads(payload) if payload else None
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", "replace")
        try:
            detail = json.loads(detail).get("detail", detail)
        except Exception:
            pass
        raise VBError(f"{method} {path} -> HTTP {e.code}: {detail}")
    except urllib.error.URLError as e:
        raise VBError(f"Voicebox server not reachable at {BASE} ({e.reason}). Start the app: vb-status --start")


def _multipart(fields, files):
    """fields: {name: str}; files: {name: Path}. Returns (body, content_type)."""
    boundary = "----vbcli" + uuid.uuid4().hex
    out = bytearray()
    for k, v in fields.items():
        out += f"--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n".encode()
        out += str(v).encode() + b"\r\n"
    for k, p in files.items():
        p = Path(p)
        out += (f"--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"; filename=\"{p.name}\"\r\n"
                f"Content-Type: application/octet-stream\r\n\r\n").encode()
        out += p.read_bytes() + b"\r\n"
    out += f"--{boundary}--\r\n".encode()
    return bytes(out), f"multipart/form-data; boundary={boundary}"


def _post_form(path, fields, files, timeout=600):
    body, ctype = _multipart(fields, files)
    return _req("POST", path, body, {"Content-Type": ctype}, timeout=timeout)


def server_up():
    try:
        return _req("GET", "/", timeout=3)
    except VBError:
        return None


# ---------------------------------------------------------------- profiles
def list_profiles():
    return _req("GET", "/profiles")


def resolve_profile(name_or_id):
    profs = list_profiles()
    for p in profs:
        if p["id"] == name_or_id:
            return p
    low = name_or_id.lower()
    hits = [p for p in profs if p["name"].lower() == low]
    if not hits:
        names = ", ".join(p["name"] for p in profs) or "(none)"
        raise VBError(f"No voice profile named '{name_or_id}'. Profiles: {names}")
    return hits[0]


def _default_engine(profile, engine):
    if engine:
        return engine
    return profile.get("default_engine") or profile.get("preset_engine") or "qwen"


def _wav_duration(path):
    try:
        with wave.open(str(path), "rb") as w:
            return round(w.getnframes() / float(w.getframerate()), 3)
    except Exception:
        return None


def _apply_speed(path, speed):
    if not speed or abs(speed - 1.0) < 1e-3:
        return
    ff = shutil.which("ffmpeg")
    if not ff:
        raise VBError("--speed needs ffmpeg on PATH (Voicebox's API has no speed parameter)")
    filt, s = [], speed
    while s > 2.0:
        filt.append("atempo=2.0"); s /= 2.0
    while s < 0.5:
        filt.append("atempo=0.5"); s /= 0.5
    filt.append(f"atempo={s:.4f}")
    tmp = Path(str(path) + ".tmp.wav")
    subprocess.run([ff, "-y", "-loglevel", "error", "-i", str(path), "-filter:a", ",".join(filt), str(tmp)], check=True)
    os.replace(tmp, path)


def synth(profile, text, out, engine=None, language=None, instruct=None, seed=None, model_size=None,
          speed=1.0, effects=None, normalize=True, save_history=False, timeout=1800):
    """Generate one WAV. Default: POST /generate/stream (returns WAV bytes, not saved in app history).
    save_history=True: POST /generate, poll GET /history/{id}, download GET /audio/{id}."""
    engine = _default_engine(profile, engine)
    body = {"profile_id": profile["id"], "text": text, "language": language or profile.get("language") or "en",
            "engine": engine, "normalize": normalize}
    if instruct:
        body["instruct"] = instruct
    if seed is not None:
        body["seed"] = seed
    if model_size:
        body["model_size"] = model_size
    if effects is not None:
        body["effects_chain"] = effects
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    gen_id = None
    if save_history:
        gen = _req("POST", "/generate", body, timeout=120)
        gen_id = gen["id"]
        t0 = time.time()
        while True:
            h = _req("GET", f"/history/{gen_id}")
            if h["status"] == "completed":
                break
            if h["status"] == "failed":
                raise VBError(f"generation failed: {h.get('error')}")
            if time.time() - t0 > timeout:
                raise VBError("timed out waiting for generation")
            time.sleep(1.5)
        data, _ = _req("GET", f"/audio/{gen_id}", raw=True, timeout=120)
    else:
        try:
            data, _ = _req("POST", "/generate/stream", body, raw=True, timeout=timeout)
        except VBError as e:
            if "not downloaded" not in str(e):
                raise
            # /generate/stream won't download models; /generate queues the download + generation.
            print("model not on disk yet - using /generate so Voicebox downloads it (can take minutes)...", file=sys.stderr)
            return synth(profile, text, out, engine, language, instruct, seed, model_size, speed, effects,
                         normalize, True, timeout)
    out.write_bytes(data)
    _apply_speed(out, speed)
    return {"path": str(out), "duration": _wav_duration(out), "engine": engine, "generation_id": gen_id}


def strip_tags(text):
    """Same rule as atlas-studio studio_api.strip_tags / news_voice.rs strip_stage_tags."""
    text = re.sub(r"\[[^\]]*\]", " ", text or "")
    return re.sub(r"\s+([,.!?;:])", r"\1", " ".join(text.split()))


# Chatterbox Turbo speaks these natively; keep them when --keep-paralinguistic is set.
PARALINGUISTIC = {"laugh", "chuckle", "gasp", "cough", "sigh", "groan", "sniff", "shush", "clear throat",
                  "angry", "crying", "dramatic", "fear", "happy", "narration", "sarcastic", "surprised",
                  "whispering", "advertisement"}


def clean_line(text, keep_para=False):
    if not keep_para:
        return strip_tags(text)
    text = re.sub(r"\[([^\]]*)\]", lambda m: m.group(0) if m.group(1).strip().lower() in PARALINGUISTIC else " ", text)
    return re.sub(r"\s+([,.!?;:])", r"\1", " ".join(text.split()))


# ---------------------------------------------------------------- commands
def cmd_status(a):
    info = server_up()
    if not info and a.start:
        if not Path(APP_EXE).exists():
            raise VBError(f"Voicebox app not found at {APP_EXE}")
        subprocess.Popen([APP_EXE], close_fds=True)
        print("Starting Voicebox app (server takes ~1-2 min on first launch)...", file=sys.stderr)
        t0 = time.time()
        while time.time() - t0 < a.wait and not info:
            time.sleep(3)
            info = server_up()
    if not info:
        print(json.dumps({"up": False, "url": BASE}) if a.json else f"DOWN  {BASE}  (run: vb-status --start)")
        return 1
    health = _req("GET", "/health")
    models = _req("GET", "/models/status")["models"]
    ready = [m["model_name"] for m in models if m.get("downloaded")]
    res = {"up": True, "url": BASE, "version": info.get("version"), "health": health,
           "downloaded_models": ready, "profiles": len(list_profiles())}
    if a.json:
        print(json.dumps(res, indent=2))
    else:
        gpu = health.get("gpu_type") or "none"
        print(f"UP  {BASE}  Voicebox {res['version']}  backend={health.get('backend_type')}/{health.get('backend_variant')}  gpu={gpu}")
        print(f"models on disk: {', '.join(ready) or '(none)'}")
        print(f"voice profiles: {res['profiles']}")
        if health.get("gpu_compatibility_warning"):
            print("warning:", health["gpu_compatibility_warning"])
    return 0


def cmd_voices(a):
    if a.presets:
        res = _req("GET", f"/profiles/presets/{a.presets}")
        if a.json:
            print(json.dumps(res, indent=2)); return 0
        for v in res["voices"]:
            print(f"{v['voice_id']:<22} {v['name']:<14} {v['gender']:<7} {v['language']}")
        return 0
    profs = list_profiles()
    if a.json:
        print(json.dumps(profs, indent=2)); return 0
    if not profs:
        print("(no voice profiles yet - create one with vb-clone)")
    for p in profs:
        kind = p.get("voice_type", "cloned")
        extra = f"{p.get('preset_engine')}:{p.get('preset_voice_id')}" if kind == "preset" else f"{p.get('sample_count', 0)} samples"
        print(f"{p['name']:<24} {p['id']}  {kind:<7} {extra:<24} engine={_default_engine(p, None)} lang={p['language']}")
    return 0


def cmd_clone(a):
    if a.preset:
        eng, _, vid = a.preset.partition(":")
        if not vid:
            raise VBError("--preset must look like kokoro:am_michael or qwen_custom_voice:<speaker>")
        body = {"name": a.name, "language": a.language, "voice_type": "preset", "preset_engine": eng,
                "preset_voice_id": vid, "default_engine": eng}
    else:
        if not a.sample:
            raise VBError("give at least one --sample WAV (2-30 s each), or use --preset engine:voice")
        body = {"name": a.name, "language": a.language, "voice_type": "cloned", "default_engine": a.engine}
    if a.description:
        body["description"] = a.description
    if a.personality:
        body["personality"] = a.personality
    prof = _req("POST", "/profiles", body)
    print(f"created profile {prof['name']} ({prof['id']})", file=sys.stderr)
    texts = list(a.text or [])
    if a.texts_file:
        texts += [l.strip() for l in Path(a.texts_file).read_text(encoding="utf-8").splitlines() if l.strip()]
    for i, s in enumerate(a.sample or []):
        ref = texts[i] if i < len(texts) else None
        if not ref and a.transcribe:
            ref = _req_transcribe(Path(s), a.language, a.whisper_model)["text"].strip()
            print(f"  transcribed {Path(s).name}: {ref!r}", file=sys.stderr)
        if not ref:
            raise VBError(f"sample {s} needs its transcript: pass --text (one per sample), --texts-file, or --transcribe")
        smp = _post_form(f"/profiles/{prof['id']}/samples", {"reference_text": ref}, {"file": Path(s)})
        print(f"  added sample {Path(s).name} ({smp['id']})", file=sys.stderr)
    print(json.dumps(_req("GET", f"/profiles/{prof['id']}"), indent=2) if a.json else prof["id"])
    return 0


def _req_transcribe(path, language=None, model=None):
    fields = {}
    if language:
        fields["language"] = language
    if model:
        fields["model"] = model
    return _post_form("/transcribe", fields, {"file": path}, timeout=1800)


def _load_effects(a):
    if getattr(a, "effects_preset", None):
        for p in _req("GET", "/effects/presets"):
            if p["name"].lower() == a.effects_preset.lower() or p["id"] == a.effects_preset:
                return p["effects_chain"]
        raise VBError(f"no effects preset '{a.effects_preset}' (see vb-effects)")
    if getattr(a, "effects_json", None):
        return json.loads(Path(a.effects_json).read_text(encoding="utf-8"))
    return None


def cmd_say(a):
    text = Path(a.file).read_text(encoding="utf-8") if a.file else " ".join(a.text or [])
    if not text.strip():
        raise VBError("no text: pass words, or --file script.txt")
    if not a.keep_tags:
        text = clean_line(text, a.keep_paralinguistic)
    prof = resolve_profile(a.voice)
    res = synth(prof, text, a.out, engine=a.engine, language=a.language, instruct=a.instruct, seed=a.seed,
                model_size=a.model_size, speed=a.speed, effects=_load_effects(a), save_history=a.history)
    print(json.dumps(res, indent=2) if a.json else f"{res['path']}  ({res['duration']} s, {res['engine']})")
    return 0


LINE_RE = re.compile(r"^\s*([A-Za-z0-9_ .'\-]{1,40}?)\s*:\s*(.+)$")


def parse_script(path):
    p = Path(path)
    if p.suffix.lower() == ".json":
        data = json.loads(p.read_text(encoding="utf-8"))
        rows = data.get("lines", data) if isinstance(data, dict) else data
        return [(r["speaker"], r.get("text") or r.get("spoken_text", "")) for r in rows]
    rows = []
    for raw in p.read_text(encoding="utf-8").splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        m = LINE_RE.match(raw)
        if m:
            rows.append((m.group(1).strip(), m.group(2)))
        elif not strip_tags(raw):
            continue  # standalone stage tag line ([BANTER], [WALK]) - not spoken, not merged
        elif rows:  # continuation line
            rows[-1] = (rows[-1][0], rows[-1][1] + " " + raw.strip())
    return rows


def cmd_batch(a):
    cast = {}
    if a.cast_json:
        cast.update({k.lower(): v for k, v in json.loads(Path(a.cast_json).read_text(encoding="utf-8")).items()})
    for c in a.cast or []:
        k, _, v = c.partition("=")
        cast[k.strip().lower()] = v.strip()
    rows = parse_script(a.script)
    out = Path(a.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    effects = _load_effects(a)
    cache, manifest, t = {}, [], 0.0
    for i, (speaker, raw) in enumerate(rows, 1):
        text = clean_line(raw, a.keep_paralinguistic)
        if not text:
            continue  # tag-only line ([BANTER], [VISUAL: ...]) - nothing to speak
        vname = cast.get(speaker.lower(), speaker)
        if vname not in cache:
            cache[vname] = resolve_profile(vname)
        prof = cache[vname]
        slug = re.sub(r"[^a-z0-9]+", "_", speaker.lower()).strip("_") or "line"
        wav = out / f"{i:03d}_{slug}.wav"
        if a.resume and wav.exists():
            res = {"path": str(wav), "duration": _wav_duration(wav), "engine": _default_engine(prof, a.engine), "generation_id": None}
        else:
            res = synth(prof, text, wav, engine=a.engine, language=a.language, seed=a.seed, speed=a.speed,
                        effects=effects, save_history=a.history)
        dur = res["duration"] or 0.0
        manifest.append({"index": i, "speaker": speaker, "profile": prof["name"], "profile_id": prof["id"],
                         "engine": res["engine"], "raw_text": raw, "text": text, "file": wav.name,
                         "duration": dur, "start": round(t, 3), "end": round(t + dur, 3)})
        print(f"[{i}/{len(rows)}] {speaker}: {wav.name} ({dur:.2f} s)", file=sys.stderr)
        t += dur + a.gap
    mpath = out / "manifest.json"
    doc = {"tool": "vb-batch", "voicebox_url": BASE, "script": str(Path(a.script).resolve()), "gap": a.gap,
           "total_duration": round(max(t - a.gap, 0), 3), "lines": manifest}
    if a.concat and manifest:
        cpath = out / "full.wav"
        _concat([out / m["file"] for m in manifest], cpath, a.gap)
        doc["full_mix"] = cpath.name
    mpath.write_text(json.dumps(doc, indent=2), encoding="utf-8")
    print(str(mpath))
    return 0


def _concat(paths, dest, gap):
    params, frames = None, []
    for p in paths:
        with wave.open(str(p), "rb") as w:
            if params is None:
                params = w.getparams()
            elif (w.getframerate(), w.getnchannels(), w.getsampwidth()) != (params.framerate, params.nchannels, params.sampwidth):
                raise VBError(f"{p} has a different WAV format; can't concat without ffmpeg")
            frames.append(w.readframes(w.getnframes()))
    silence = b"\x00" * int(params.framerate * gap) * params.nchannels * params.sampwidth
    with wave.open(str(dest), "wb") as w:
        w.setparams(params)
        w.writeframes(silence.join(frames))


def cmd_export(a):
    if a.generation:
        data, _ = _req("GET", f"/history/{a.generation}/export-audio" if not a.zip else f"/history/{a.generation}/export", raw=True)
        dest = Path(a.out or (f"{a.generation}.zip" if a.zip else f"{a.generation}.wav"))
    else:
        prof = resolve_profile(a.voice)
        data, _ = _req("GET", f"/profiles/{prof['id']}/export", raw=True, timeout=300)
        safe = re.sub(r"[^A-Za-z0-9 _-]", "", prof["name"]).strip() or "profile"
        dest = Path(a.out or f"profile-{safe}.voicebox.zip")
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    print(str(dest))
    return 0


def cmd_import(a):
    prof = _post_form("/profiles/import", {}, {"file": Path(a.zip)})
    print(f"{prof['name']}  {prof['id']}")
    return 0


def cmd_transcribe(a):
    res = _req_transcribe(Path(a.audio), a.language, a.model)
    print(json.dumps(res, indent=2) if a.json else res["text"])
    return 0


def cmd_models(a):
    if a.download:
        print(json.dumps(_req("POST", "/models/download", {"model_name": a.download})))
        return 0
    if a.progress:
        print(json.dumps(_req("GET", f"/models/progress/{a.progress}"), indent=2))
        return 0
    ms = _req("GET", "/models/status")["models"]
    if a.json:
        print(json.dumps(ms, indent=2)); return 0
    for m in ms:
        size = f"{m['size_mb']:.0f} MB" if m.get("size_mb") else ""
        state = "loaded" if m.get("loaded") else ("downloaded" if m.get("downloaded") else ("downloading" if m.get("downloading") else "-"))
        print(f"{m['model_name']:<24} {state:<11} {size:>9}  {m['display_name']}")
    return 0


def cmd_effects(a):
    if a.available:
        print(json.dumps(_req("GET", "/effects/available"), indent=2)); return 0
    for p in _req("GET", "/effects/presets"):
        chain = ", ".join(e["type"] for e in p["effects_chain"])
        print(f"{p['name']:<18} {'builtin' if p.get('is_builtin') else 'custom ':<8} {chain}")
    return 0


def cmd_delete(a):
    prof = resolve_profile(a.voice)
    if not a.yes:
        raise VBError(f"refusing to delete '{prof['name']}' without --yes")
    print(json.dumps(_req("DELETE", f"/profiles/{prof['id']}")))
    return 0


# ---------------------------------------------------------------- argparse
def build_parser(prog):
    ap = argparse.ArgumentParser(prog=prog, description="Voicebox local API CLI")
    sub = ap.add_subparsers(dest="cmd", required=True)

    def gen_opts(p):
        p.add_argument("--engine", choices=ENGINES, help="TTS engine (default: profile's engine)")
        p.add_argument("--language", choices=LANGS)
        p.add_argument("--seed", type=int)
        p.add_argument("--speed", type=float, default=1.0, help="tempo multiplier via ffmpeg atempo (pitch kept)")
        p.add_argument("--effects-preset", help="name of a Voicebox effects preset, e.g. Radio")
        p.add_argument("--effects-json", help="JSON file with an effects_chain list")
        p.add_argument("--history", action="store_true", help="save in the app's history (POST /generate) instead of /generate/stream")
        p.add_argument("--keep-paralinguistic", action="store_true", help="keep [laugh]/[sigh]... tags (Chatterbox Turbo only)")

    p = sub.add_parser("status"); p.add_argument("--start", action="store_true"); p.add_argument("--wait", type=int, default=180)
    p.add_argument("--json", action="store_true"); p.set_defaults(fn=cmd_status)

    p = sub.add_parser("voices"); p.add_argument("--presets", metavar="ENGINE", help="list preset voices: kokoro | qwen_custom_voice")
    p.add_argument("--json", action="store_true"); p.set_defaults(fn=cmd_voices)

    p = sub.add_parser("clone"); p.add_argument("name"); p.add_argument("--sample", action="append", help="WAV/MP3 2-30 s; repeatable")
    p.add_argument("--text", action="append", help="transcript of each sample, in order")
    p.add_argument("--texts-file", help="one transcript per line, in sample order")
    p.add_argument("--transcribe", action="store_true", help="transcribe samples with Whisper (needs a whisper model)")
    p.add_argument("--whisper-model", help="base|small|medium|large|turbo")
    p.add_argument("--engine", default="qwen", choices=CLONING_ENGINES, help="default engine for this cloned voice")
    p.add_argument("--preset", help="make a preset profile instead, e.g. kokoro:am_michael")
    p.add_argument("--language", default="en", choices=LANGS); p.add_argument("--description"); p.add_argument("--personality")
    p.add_argument("--json", action="store_true"); p.set_defaults(fn=cmd_clone)

    p = sub.add_parser("say"); p.add_argument("text", nargs="*"); p.add_argument("--file"); p.add_argument("-v", "--voice", required=True)
    p.add_argument("-o", "--out", default="say.wav"); p.add_argument("--instruct", help="delivery note (Qwen engines)")
    p.add_argument("--model-size", choices=["1.7B", "0.6B", "1B", "3B"]); p.add_argument("--keep-tags", action="store_true")
    gen_opts(p); p.add_argument("--json", action="store_true"); p.set_defaults(fn=cmd_say)

    p = sub.add_parser("batch"); p.add_argument("script", help="'speaker: text' lines (.txt) or JSON [{speaker,text}]")
    p.add_argument("-o", "--out-dir", default="vb_batch"); p.add_argument("--cast", action="append", help="speaker=Profile; repeatable")
    p.add_argument("--cast-json", help='{"MARCUS": "Marcus", ...}'); p.add_argument("--gap", type=float, default=0.25)
    p.add_argument("--concat", action="store_true", help="also write full.wav"); p.add_argument("--resume", action="store_true")
    gen_opts(p); p.set_defaults(fn=cmd_batch)

    p = sub.add_parser("export"); p.add_argument("voice", nargs="?"); p.add_argument("--generation", help="generation id instead of a profile")
    p.add_argument("--zip", action="store_true", help="with --generation: full zip instead of WAV"); p.add_argument("-o", "--out")
    p.set_defaults(fn=cmd_export)

    p = sub.add_parser("import"); p.add_argument("zip"); p.set_defaults(fn=cmd_import)
    p = sub.add_parser("transcribe"); p.add_argument("audio"); p.add_argument("--language"); p.add_argument("--model")
    p.add_argument("--json", action="store_true"); p.set_defaults(fn=cmd_transcribe)
    p = sub.add_parser("models"); p.add_argument("--download", metavar="MODEL"); p.add_argument("--progress", metavar="MODEL")
    p.add_argument("--json", action="store_true"); p.set_defaults(fn=cmd_models)
    p = sub.add_parser("effects"); p.add_argument("--available", action="store_true"); p.set_defaults(fn=cmd_effects)
    p = sub.add_parser("delete"); p.add_argument("voice"); p.add_argument("--yes", action="store_true"); p.set_defaults(fn=cmd_delete)
    return ap


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    # vb-<cmd>.cmd shims call: vbcli.py <cmd> ...
    a = build_parser("vb").parse_args(argv)
    if a.cmd == "export" and not a.generation and not a.voice:
        print("vb-export: give a profile name, or --generation <id>", file=sys.stderr); return 2
    try:
        return a.fn(a) or 0
    except VBError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
