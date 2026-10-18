# voicebox-cli

Command-line wrappers for the local [Voicebox](https://github.com/jamiepine/voicebox) voice studio
(MIT, v0.5.0). Stdlib-only Python (`vbcli.py`) plus one `.cmd` shim per tool.

- App: `C:\Users\chrom\Tools\voicebox\voicebox.exe` (per-user NSIS install, v0.5.0)
- API: `http://127.0.0.1:17493` (override with `VOICEBOX_URL`); interactive docs at `/docs`
- Data: `%APPDATA%\sh.voicebox.app` (profiles DB, samples, generations)
- The server only runs while the app is open (`vb-status --start` launches it; first boot ~2 min).

| Tool | What it does | API |
| --- | --- | --- |
| `vb-status [--start] [--json]` | Is the server up, version, CPU/CUDA, models on disk | `GET /`, `/health`, `/models/status`, `/profiles` |
| `vb-voices [--presets kokoro\|qwen_custom_voice]` | List voice profiles, or an engine's preset voices | `GET /profiles`, `/profiles/presets/{engine}` |
| `vb-clone NAME --sample a.wav --text "..." [--engine chatterbox]` | Clone a voice from 2-30 s samples (repeat `--sample`/`--text`; `--texts-file`; `--transcribe` uses Whisper) | `POST /profiles`, `POST /profiles/{id}/samples` |
| `vb-clone NAME --preset kokoro:am_michael` | Preset (non-cloned) profile | `POST /profiles` |
| `vb-say -v VOICE -o out.wav "text" \| --file f.txt` | TTS to WAV. `--engine --language --instruct --seed --model-size --speed 1.1 --effects-preset Radio --effects-json --history` | `POST /generate/stream` (or `/generate` + `/history/{id}` + `/audio/{id}` with `--history`) |
| `vb-batch script.txt -o dir --cast MARCUS="CDN Marcus" [--concat] [--gap 0.25] [--resume]` | `SPEAKER: text` lines (or JSON `[{speaker,text}]`) to per-line WAVs + `manifest.json` (start/end/duration per line). Strips every `[bracket]` tag (`[BANTER]`, `[REACTION:x]`, `[VISUAL: ...]`, `[WALK]`); tag-only lines are skipped | same as `vb-say` |
| `vb-export VOICE [-o x.zip]` / `vb-export --generation ID [--zip]` | Back up a cloned profile (zip) or pull a generation's WAV | `GET /profiles/{id}/export`, `/history/{id}/export-audio`, `/history/{id}/export` |
| `vb-import file.voicebox.zip` | Restore a profile zip | `POST /profiles/import` |
| `vb-transcribe audio.wav [--model turbo]` | Whisper speech-to-text | `POST /transcribe` (form field `file`) |
| `vb-models [--download NAME] [--progress NAME]` | Model status / trigger download | `/models/status`, `POST /models/download`, `/models/progress/{name}` |
| `vb-effects [--available]` | Effects presets (Robotic, Radio, Echo Chamber, Deep Voice) | `/effects/presets`, `/effects/available` |
| `vb-delete VOICE --yes` | Delete a profile | `DELETE /profiles/{id}` |

Notes
- `--speed` is applied after generation with ffmpeg `atempo` (pitch preserved); the Voicebox API itself has no speed parameter.
- Preset (Kokoro) profiles can't be exported - export is for cloned profiles with samples.
- `/generate/stream` refuses if the engine's model isn't downloaded; `vb-say`/`vb-batch` then fall back to `/generate`, which downloads it first.
- Imported profiles come back with default engine `qwen`; pass `--engine` when using them.
- `[laugh]`, `[sigh]` etc. are only spoken by `chatterbox_turbo`; keep them with `--keep-paralinguistic`.
- Engines that clone: `qwen`, `luxtts`, `chatterbox`, `chatterbox_turbo`, `tada`. Presets: `kokoro`, `qwen_custom_voice`.

Installed profiles: `CDN Marcus` (kokoro:am_michael) and `CDN Elena` (kokoro:af_bella), matching `ATLAS_MARCUS_VOICE` / `ATLAS_ELENA_VOICE`.
