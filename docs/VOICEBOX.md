# Voicebox voice backend (CDN anchors + Thursday Throwback guests)

[Voicebox](https://github.com/jamiepine/voicebox) v0.5.0 (MIT) is a local voice studio: zero-shot voice
cloning, 8 TTS engines (Qwen3-TTS, Chatterbox, Chatterbox Turbo, LuxTTS, TADA, Kokoro, Qwen CustomVoice/VoiceDesign),
effects, Whisper STT, a REST API on `127.0.0.1:17493`, and an MCP server at `/mcp`.

It is an **optional** voice backend next to the built-in Kokoro path (`atlas-kokoro`, `news_voice.rs`). The
news pipeline uses it when `ATLAS_TTS_ENGINE=voicebox` (default `kokoro`). `atlas news produce` then
checks `VOICEBOX_URL` (default `http://127.0.0.1:17493`), writes `audio/voicebox/script.json`, runs
`py vbcli.py batch ... --cast marcus="CDN Marcus" --cast elena="CDN Elena" --cast duo="CDN Marcus" --gap 0 --concat`
and reads `manifest.json`. If the app is closed or a line fails, the whole episode falls back to Kokoro, so a
missing Voicebox never breaks a run. `voice_timeline.json` records which engine voiced the episode.

| Setting | Default |
| --- | --- |
| `ATLAS_TTS_ENGINE` | `kokoro` (`voicebox` to opt in) |
| `VOICEBOX_URL` | `http://127.0.0.1:17493` |
| `ATLAS_VOICEBOX_CLI` | `C:\Users\chrom\Tools\voicebox-cli\vbcli.py`, then `integrations/voicebox-cli/vbcli.py` |
| `ATLAS_VOICEBOX_MARCUS` / `ATLAS_VOICEBOX_ELENA` | `CDN Marcus` / `CDN Elena` |

You can still run the CLI by hand to produce line WAVs and the manifest for lip sync or the Blender studio.

- App: `C:\Users\chrom\Tools\voicebox\voicebox.exe` - must be open for the API (`vb-status --start`)
- CLI: `C:\Users\chrom\Tools\voicebox-cli\vb-*.cmd` (source mirrored in `integrations/voicebox-cli/`, see its README)

## Who uses which voice

| Role | Profile | How |
| --- | --- | --- |
| Marcus (anchor) | `CDN Marcus` | preset `kokoro:am_michael` (same as `ATLAS_MARCUS_VOICE`) - swap for a clone any time |
| Elena (anchor) | `CDN Elena` | preset `kokoro:af_bella` (same as `ATLAS_ELENA_VOICE`) |
| Thursday Throwback guest, source had a voice | `TT <Character>` | `vb-clone "TT <Character>" --engine chatterbox --sample clip.wav --text "<transcript>"` from a public-domain clip (2-30 s each, several samples is better) |
| Thursday Throwback guest, silent-era source | `TT <Character>` | original voice: preset (`vb-voices --presets kokoro`) or a designed/cloned voice the Character Actor picks; never clone a real living actor |

## Episode flow

```mermaid
flowchart LR
  S[script.txt<br/>SPEAKER: text + stage tags] --> B[vb-batch --cast ...]
  B -->|strip [BANTER] [REACTION] [VISUAL] [WALK]| VB[(Voicebox API :17493)]
  VB --> W[lines/NNN_speaker.wav]
  B --> M[manifest.json<br/>start/end/duration]
  W --> L[lip sync + Blender studio cut]
  M --> L
```

```
vb-batch episode.txt -o news\EP-012\audio\voicebox --cast MARCUS="CDN Marcus" --cast ELENA="CDN Elena" --cast GUEST="TT <Character>" --concat
```

Tag stripping matches `studio_api.strip_tags` and `news_voice.rs strip_stage_tags`.

## Anchor voice consistency gate (required for every episode)
Lesson from the v2 audition (Oct 18): with Chatterbox Turbo clones, two renders sent to one Voicebox server at the same time swapped speaker
conditioning (Marcus lines came out in Elena's voice), and emotion tags such as `[whispering]`/`[sarcastic]`/`[happy]`/`[surprised]` shift timbre.
Rules for any anchor render:
1. **One Voicebox request at a time.** Never run two vb-say/vb-batch jobs against the same server.
2. Fixed seed list per speaker (reference script: Elena 5100+, Marcus 6100+). Only `[laugh]` and `[chuckle]` are allowed as tags.
3. **Gate every take** with `casting\voices\tools\speaker_gate.py` (Resemblyzer d-vectors, CPU). Compare each take to the canonical references
   (`casting\voices\anchors\{elena,marcus}\*_v2_reference.wav`): own-reference cosine >= 0.80, and own minus other >= 0.10. This also proves the
   take matches the speaker in the script. Re-render failing takes with the next seed.
4. Final check: average-linkage cosine clustering (cut 0.25) over all picked lines must give **exactly 2 clusters**, one per anchor.
5. Script sanity: an anchor's own name may appear only in a self-intro ("I'm Marcus"), never as an addressee. Spell Elena "Elayna" in TTS text
   (Whisper heard "Alina").
Reference implementation: `AtlasStudioOffice\casting\audition\banter_v3\render_v3.py` (writes the per-line table to `render_report.json`).
TODO: port this gate into `news_voice.rs` (call speaker_gate.py after vb-batch; fail the voice step if `all_pass` is false).
