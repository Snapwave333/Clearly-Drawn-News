# Voicebox voice backend (CDN anchors + Thursday Throwback guests)

[Voicebox](https://github.com/jamiepine/voicebox) v0.5.0 (MIT) is a local voice studio: zero-shot voice
cloning, 8 TTS engines (Qwen3-TTS, Chatterbox, Chatterbox Turbo, LuxTTS, TADA, Kokoro, Qwen CustomVoice/VoiceDesign),
effects, Whisper STT, a REST API on `127.0.0.1:17493`, and an MCP server at `/mcp`.

It is an **optional** voice backend next to the built-in Kokoro path (`atlas-kokoro`, `news_voice.rs`). The
Rust pipeline does not call it yet; use the CLI to produce line WAVs and the manifest, then feed them to
lip sync / the Blender studio.

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
