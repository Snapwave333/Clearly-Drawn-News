# ATLAS STUDIO
## Autonomous YouTube Educational Video Production — Master Requirements & Agent Architecture

**Version:** 2.0 (full replacement of ATLAS Academy)  
**Date:** 2026-10-15  
**Status:** Build-ready product specification and implementation plan  
**Target:** Windows 11; autonomous, headless-first production with an optional premium desktop control room

> **Core mission:** Build an autonomous, multi-agent YouTube production studio that discovers worthwhile trending subjects, researches them, writes original and accurate explanations, designs distinctive whiteboard illustrations, animates full scenes with Blender and Unreal Engine, records narration through Kokoro TTS, edits and quality-checks finished videos, makes thumbnails and metadata, and optionally uploads/schedules videos to YouTube. The outcome should feel like a carefully art-directed educational production—not a slide deck, generic stock montage, or repetitive AI-content factory.
>
> **AI disclosure:** All videos include transparent AI-production credits. Use YouTube's altered/synthetic-content disclosure setting whenever the video meets the platform's disclosure criteria; end credits alone are not a substitute for a required platform disclosure.

---

# 1. REDEFINE THE PRODUCT

ATLAS STUDIO is **not a school or a course builder**. Remove the old 1–10 level academy workflow, learner mastery system, student exams, skill trees, and NotebookLM-style student chat dashboard. Retain source-backed research and reusable knowledge notebooks as **internal production tools**, not the customer-facing product.

The user operates a **YouTube channel powered by a coordinated production team of AI agents**. It should run from discovery through upload without manually writing prompts, moving files, opening editing tools, or assembling timelines. Users may override topics, art direction, scheduling, scripts, and publication permissions, but unattended operation is the target.

A typical film answers one compelling question about something currently trending:
- “Why is everybody talking about this breakthrough?”
- “What actually happens inside the machine everyone is watching?”
- “Why is this scientific idea suddenly relevant?”
- “What caused the recent event, according to evidence available now?”
- “How does this technology, discovery, piece of history, or trend really work?”

**Output:** An illustrated, researched, narrated, edited, captioned, sourced YouTube video (usually 5–12 minutes), plus 16:9 thumbnail, accurate title/description, chapter markers, transcript, subtitle file, source list, chapter illustrations, production manifest, credits, and optional Shorts cutdowns.

**Brand promise:** Visually authored, conceptually original, genuinely educational. Do not promise an impossibility such as guaranteed human-like appearance or guaranteed monetization. Enforce measurable creative and editorial standards instead.

## 1.1 Operating modes

| Mode | What happens |
|---|---|
| Autonomous | Discover → decide → produce → audit → publish according to previously configured permission and channel schedule. |
| Review before publish | Everything is automatic through a private draft; a person approves the final upload or public release. |
| Topic on demand | User enters a topic/URL; skip discovery but run the same research-to-video workflow. |
| Research-first | Generate a cited treatment and script; hold renders until approved. |
| Visual production only | Start from supplied script/audio; storyboard, animate, compose, export. |
| Batch series | Build related episodes with reusable art direction but unique storytelling and animation. |

**Default:** Review before publish. Fully unattended publication is explicitly enabled in settings with a connected and authorized YouTube channel. “Fully autonomous” means no per-video operator intervention *after opt-in* if all quality gates pass; never circumvent failed gates or missing permissions.

# 2. TECHNOLOGY AND INSTALLED TOOL INTEGRATION

Assume the following programs are already installed locally; **discover, inspect and reuse them before proposing installation, cloning or downloading**:

| Tool | Production responsibility | Headless interface |
|---|---|---|
| NVIDIA NIM free hosted models | Agent reasoning, research synthesis, scripts, shot decisions, QA | Existing OpenCode credential/provider or supported local API adapter |
| OpenCode | Existing model configuration, active credential source, optional agent bridge | Supported OpenCode CLI/server/API; version-detect v1/v2 |
| Blender | Main animation engine: drawn strokes, Grease Pencil, 2.5D diagrams, 3D explanatory objects, camera, lighting, rendering | `blender --background ... --python ...`; inspect installed version |
| Unreal Engine | Selective real-time/cinematic 3D examples, simulations, environment shots, complex physical/spatial demonstrations | Version-matched Unreal Editor command-line, Python scripts, Sequencer/Movie Render Queue |
| VectorCraft | Original vectors, storyboard assets, clean diagrams, SVG illustrations, charts | `vectorcraft-cli` run/headless/MCP |
| PhotoCraft | Referenced photos, image cleanup, editorial compositing, thumbnails | `photocraft-cli` run/batch/MCP |
| EffectCraft | Motion typography, motion infographics, overlays, supplemental compositing | `effectcraft-cli` render/MCP/control channel |
| Kokoro TTS | Primary voiceover generation | Detect existing executable, Python bindings, API or local server |
| DuckDuckGo | Broad web discovery and research seed searches | Rate-limited search adapter; treat as discovery, not sole verification |
| FFmpeg/ffprobe | Final editorial assembly, mixes, subtitle mux, probing, thumbnail frames | Discover executable; install only if missing and user allows |

**Practical distinction:** VectorCraft, PhotoCraft and EffectCraft are **creation/editing/rendering engines**, not general-purpose text-to-image generation models. Agents must author source geometry, documents, compositions, timelines and edit commands. Do not silently pretend they can produce original content from natural-language text without a scene/command-construction layer.

Use Unreal only when it materially strengthens an explanation. Whiteboard line art remains the dominant art form; gratuitous 3D fly-throughs make the result less coherent.

## 2.1 Technical architecture

- **Coordinator/worker runtime:** Rust (Tokio, serde, SQLite, process supervisor). Implement deterministic state machine, deadlines, resource locks, task queues, event bus, and recoverable artifacts. Independent local processes may speak typed JSON over localhost/stdin/stdout; no hidden dependence on the GUI.
- **Desktop control room:** Angular + TypeScript + Tauri 2, optionally launched against the headless service. Light and dark themes, never generic template-looking cards and gradients.
- **Storage:** SQLite in WAL mode for jobs, decisions, assets, messages, evaluations, model/usage telemetry; on-disk content-addressed artifact store; full-text source search with SQLite FTS5, optional vector index for research RAG.
- **Transport:** Persistent job state in SQLite with typed event ledger; localhost WebSocket or HTTP stream for live events; agent messages on a logical bus; never make volatile in-memory chat the only shared state.
- **Rendering:** Blender and Unreal isolated as worker processes, serialized or resource-locked by available VRAM; bounded CPU/RAM/GPU use; job-specific temp directories.
- **Video timeline:** An explicit edit decision list (EDL) or JSON scene timeline with frame-accurate timing; use FFmpeg for final conform and mix; allow EffectCraft precomps where useful.
- **Models:** Dynamically discover current NVIDIA NIM endpoint/model list, capabilities, context limits, free availability and rate-limit response. Route tasks by measured ability, cost, latency and health, not hard-coded fashionable model names.
- **Platforms:** Windows 11 first. Project paths must be Unicode-safe and handle Windows spaces. No Unix-specific assumptions in the core service.

# 3. OPENCode API KEY: REQUIRED EXISTING CREDENTIAL REUSE

**Mandatory:** Use the **same NVIDIA NIM API key currently active in OpenCode**. Do not make the user create a second NVIDIA account or retype the same secret. Never generate a new key or silently replace the active provider.

Implementation requirements:

1. Discover which `opencode` executable and version are actually installed. Read provider metadata/configuration using **documented** OpenCode interfaces. Identify the current NVIDIA provider/account; support an OpenCode account switch without reinstalling ATLAS.
2. **Preferred solution:** Invoke the configured OpenCode local backend/SDK or CLI as the model-inference broker **if** its supported interface can serve the required operations with the active provider. OpenCode owns credentials; ATLAS receives responses but not the raw token. Protect loopback server with supported authentication and minimum permissions.
3. **Version-aware fallback:** OpenCode v1 describes credentials in its user data directory's `auth.json`; OpenCode v2 documents saved credentials in its local SQLite database and an active-account concept. **Do not assume v2 still uses `auth.json` or directly edit the v2 database.** Prefer supported interfaces; if no supported credential forwarding mechanism exists, detect available authorized environment-provider configuration. Report incompatibility clearly rather than quietly extracting/decrypting secrets.
4. If direct NVIDIA endpoint access is genuinely required, offer a one-time, explicitly approved local integration that refers to the same credential via OS-protected secret storage or a user-authorized handle. Never put plaintext tokens in project files, app logs, command-line arguments, screenshots, video manifests, AI-agent tool prompts, or exported diagnostics.
5. Secret access belongs only to the broker process. Worker agents request model operations via the broker and receive no keys.
6. Respect provider quotas, `429` and `Retry-After`, network failure, concurrent request ceilings, and shared limits with OpenCode. Cache research summaries and intermediate artifacts. Favor fewer high-quality calls over swarms burning through free quota.
7. Show model status, routing choices, API connectivity and provider-limit health in the UI **without displaying secret material**.
8. Acceptance: after authorized initial setup, changing the active OpenCode NVIDIA account is reflected by ATLAS without manually entering a second secret. If OpenCode is unreachable or not configured for NVIDIA, show a specific actionable error and do not claim automatic credential reuse succeeded.

Official implementation references: [OpenCode providers](https://dev.opencode.ai/docs/providers), [OpenCode v2 provider accounts](https://dev.opencode.ai/v2/docs/cli/providers/), [OpenCode local server](https://dev.opencode.ai/docs/server/), [NVIDIA NIM catalog](https://build.nvidia.com/models).

# 4. THE AUTONOMOUS AGENT PRODUCTION COMPANY

Build an actual **communicating agent team**, not a list of prompt personas running sequentially. Each agent has a responsibility, typed tools, input/output contract, shared evidence access, objective success criteria, a work inbox, and explicit escalation policies. Agents issue requests and reviews to one another through the message bus. They can revisit earlier stages, challenge unsupported assertions, and demand revision of shots rather than blindly accepting preceding output.

## 4.1 Agent roster

| Agent | Responsibilities | Key handoffs |
|---|---|---|
| 01 Executive Producer | Scheduling, strategic topic acceptance, budget, priority, conflict resolution | Assigns projects; resolves blocks |
| 02 Trend Scout | Monitors trend feeds, detects developing interest and audience questions | Candidates → Trend Analyst |
| 03 Trend Analyst | Ranks timely topics and checks longevity/relevance/competition | Scorecard → Executive Producer |
| 04 Research Lead | Builds cited dossier from independent reputable sources | Dossier → Fact Checker/Writer |
| 05 Fact Checker | Checks claim-source support, dates, disputed narratives and uncertainty | Findings → Writer/Producer |
| 06 Subject Editor | Ensures explanations are accurate for the specific domain | Notes → Writer/Storyboard |
| 07 Creative Director | Concept, visual metaphor, artistic consistency and originality | Style brief → every creative role |
| 08 Lead Writer | Original narrative, hook, script structure, explanations, transitions | Script → Editor/Storyboard |
| 09 Script Editor | Cuts padding, improves clarity and rhythm, catches AI-sounding prose | Approved rewrite → Storyboard |
| 10 Storyboard Artist | Converts script to shots, framing, reveal beats, board notes | Shot plan → Illustrator/Animator |
| 11 Illustrator | VectorCraft art, clean diagrams, shapes, arrows, labels, icon language | SVG/project files → Animation |
| 12 Motion Director | Chooses purposeful animation, timing, reveals and scene transitions | Animation brief → Blender/Unreal |
| 13 Blender Animator | Builds Grease Pencil/2.5D/3D whiteboard scenes and renders | Scene files/frames → Compositor |
| 14 Unreal Cinematographer | Creates UE demo scenes only where spatial realism is necessary | Sequencer shots/frames → Compositor |
| 15 Voice Director | Kokoro voice style, pronunciation, pacing, retakes, timing | Narration + timing → Editor |
| 16 Audio Editor | Cleans speech, mixes original/licensed sound and music, normalizes | Audio stems → Editor |
| 17 Video Editor | Edits scenes to narration, captions, camera cuts, pacing, final conform | Candidate MP4 → Quality Team |
| 18 Accessibility Editor | Captions, readable overlays, comprehension, flashing and contrast | Fix list → Creative roles |
| 19 Quality Supervisor | Automated and semantic visual/audio/fact/novelty inspections | Pass/fail; sends rework tickets |
| 20 Publisher/Analyst | Metadata, thumbnail tests, credits, YouTube upload, performance analysis | Published ID/analytics → Producer |

**Auditor independence:** Fact Checker and Quality Supervisor must not simply re-evaluate their own generated output using an identical prompt. Use separate agent contexts, deterministic validators, and independent source or frame inspection. An extra model family is valuable if one is available under quota, but deterministic tests still matter.

## 4.2 Communication protocol

All handoffs pass through a persistent, queryable message system:

```json
{
  "schema_version": 1,
  "message_id": "msg_...",
  "project_id": "vid_...",
  "from": "fact_checker",
  "to": "lead_writer",
  "type": "REVISION_REQUIRED",
  "task_id": "script_scene_04",
  "artifact_ids": ["script_v3", "dossier_v2"],
  "blocking": true,
  "priority": "high",
  "feedback": [{
    "claim_id": "claim_27",
    "issue": "Source describes an estimate as if it were measured",
    "required_change": "Label as estimate and use the correct dated range"
  }],
  "created_at": "ISO-8601 timestamp",
  "deadline_at": "ISO-8601 timestamp"
}
```

Required event types: `TASK_CREATED`, `TASK_ACCEPTED`, `ARTIFACT_READY`, `REVIEW_REQUESTED`, `REVIEW_PASSED`, `REVISION_REQUIRED`, `NEEDS_RESEARCH`, `RENDER_QUEUED`, `RENDER_FAILED`, `QUALITY_FAILED`, `QUALITY_PASSED`, `APPROVAL_REQUESTED`, `UPLOAD_STARTED`, `PUBLISHED`, `HALTED`, `HEARTBEAT`.

Rules:
- Each artifact is immutable/versioned; a revision produces a new artifact hash and lineage link.
- Agents cite artifact IDs, not chat history. Shared production memory is authoritative for task data, not for factual claims absent sources.
- A blocking review prevents downstream publication. Non-blocking suggestions are ranked by value/cost.
- Detect deadlocks, circular review loops, dropped worker processes and contradictory agent requests. Escalate to Producer after bounded revisions.
- Require acknowledgement/timeouts/retries with idempotency keys to prevent duplicate renders/uploads.
- Preserve an event log and decision rationale, with tool outputs sanitized of secrets.
- Workers claim leases and send heartbeats; abandoned tasks are recoverable on process restart.
- Allow parallel work when dependencies permit: illustration and narration can start once the approved script and board exist.

## 4.3 Example multi-agent discussion

```
[Trend Scout]   "Battery recycling spike: 3 independent signals, active 18h."
[Trend Analyst] "Educational opportunity: explain hydrometallurgy, not headline recap."
[Research Lead] "Primary engineering references sourced; one viral claim unsupported."
[Fact Checker]  "Reject the viral efficiency percentage. Request revised wording."
[Lead Writer]   "New hook: What actually happens to old lithium-ion batteries?"
[Creative Dir.] "Recurring visual motif: ink-drawn battery cross-section."
[Storyboard]    "Scene 3 needs a nested assembly reveal and liquid-flow demonstration."
[Blender]       "Animating traced cell components and animated fluid paths."
[Unreal]        "Decline Unreal shot—adds no learning value here."
[Audio]         "Voice timing says scene 3 needs +42 frames."
[Quality]       "Diagram labels overlap at 1080p; revision required."
[Publisher]     "Passed all gates. Prepared upload with links and disclosures."
```

The team should be able to challenge a suggestion (“decline Unreal”) rather than execute gratuitous work.

# 5. TREND INTELLIGENCE: DISCOVER WHAT IS WORTH TEACHING

**Core requirement:** Produce videos about *what people currently want to understand*, not random generic encyclopedia summaries.

## 5.1 Discovery sources

1. **Google Trends Trending Now RSS** for geographic/category/time-specific rising queries; include published/retrieved timestamps.
2. **YouTube Data API** for `videos.list?chart=mostPopular&regionCode=...`, and selective keyword/topic searches. Popular-video charts are not a direct measure of search demand; label accordingly. Use only authorized quotas.
3. **DuckDuckGo** for discovery, audience phrasing, recent reports, and source finding; treat its search surfaces as unofficial and failure-prone. Cache and rate-limit.
4. **Trusted source feeds** for science/technology/public affairs/culture depending on channel scope, preferably official institutions and publishers with RSS/API terms permitting access.
5. **Optional user inputs:** subscribed topic list, bookmarked web pages, creator notes, manual idea pitches, previous published videos and audience questions.

Never claim exact platform-wide trend counts without a reputable measured source. Never hallucinate “trending right now.” Every idea card shows where/when the signal was observed.

Reference: [Google Trends RSS export](https://support.google.com/trends/answer/3076011), [YouTube mostPopular chart](https://developers.google.com/youtube/v3/docs/videos/list).

## 5.2 Trend scoring

Score every candidate 0–100 using configurable weighted factors:

| Signal | Default weight |
|---|---:|
| Current interest and momentum, multi-source where available | 25 |
| Can be explained visually and educationally | 20 |
| Strong, verifiable sources exist | 20 |
| Fresh angle/low overlap with existing uploads | 15 |
| Expected retention: one clear question and satisfying answer | 10 |
| Feasible to produce to house quality with current render budget | 10 |

Reject candidates that are unverified rumors, duplicates, obvious clickbait without educational substance, inaccessible evidence, or require realistic reenactments prone to misleading viewers. No “breaking news” guarantee: important developing stories may need verification time before production.

Trend dossier fields: `trend_id`, `candidate_questions[]`, `region`, `observed_at`, `signal_sources[]`, `momentum_evidence`, `canonical_topic`, `recommended_angle`, `freshness_deadline`, `evidence_risk`, `render_cost_estimate`, `score`, `rejection_reason`.

## 5.3 Editorial schedule

- Scheduler discovers trends at a configurable cadence (default every 6 hours, bounded by provider/API limits).
- Produce one candidate at a time initially; advance frequency only after stable quality and render performance.
- Balance 70% timely explainers / 30% evergreen-with-current-hook by default; configurable.
- Limit same-theme consecutive videos, celebrity fixation, near-duplicate titles and shallow “top 10 facts” content.
- When trend expires before release, re-evaluate facts, title, timeliness, and whether to shelve the video.
- Create channel series/recurring educational pillars without reusing scripts or shallowly swapping nouns.

# 6. RESEARCH AND FACTUAL RELIABILITY PIPELINE

The script may be original, but **facts must be traceable**.

```
Trend signal → question formulation → source discovery → source snapshot
→ claim extraction → independent verification → research dossier
→ educational outline → script with claim IDs → fact-check → production
→ final claim-to-timecode audit → description citations.
```

For each claim store `claim_id, exact wording, evidence passage or dataset, canonical URL, source owner, publication date, retrieval date, confidence, verification status, script timecode`. Store source excerpts where permitted with provenance, not full copyrighted works by default.

Editorial standards:
- Prefer primary documents, peer-reviewed studies, official technical docs, original reporting, standards and reputable data.
- Verify important claims across independent credible sources when possible; independent means different provenance, not six sites copying one press release.
- Flag speculation, extrapolation, preliminary findings, contested statistics, and dates explicitly in narration/text.
- Use DuckDuckGo for **discovery**, not as automatic proof of any assertion.
- No invented quotes, screenshots, citations, numbers, experts or event footage. No synthetic on-screen facsimile presented as authentic evidence.
- If evidence changes during production, revise the script and invalidate affected downstream scenes; do not silently upload stale assertions.
- Include concise source citations in description and an optional on-screen/source-panel interlude for key findings.
- Keep internal source notebook and searchable claim graph for each film.
- Store URLs and dates; surface failed retrieval rather than letting the model fabricate missing text.

# 7. SIGNATURE VISUAL LANGUAGE: THE LIVING WHITEBOARD

**Desired result:** Imagine a premium educational illustrator working at cinematic scale: concepts are drawn in front of the viewer, morph intelligently, and become spatial animated models when needed. Think *editorial motion design and explanatory illustration*, not the cliché whiteboard-video preset that drags a stock hand across clip art.

## 7.1 Art direction Bible

- Whiteboard is a **visual metaphor**, not necessarily a literal wall and fake arm.
- Base environments: clean warm-white/off-white paper, lightly tactile drafting surfaces, occasional dark inversion as an intentional chapter transition.
- Default stroke: near-black ink, continuous pressure-consistent lines, selected roughening for human character, legible at 1080p and 4K.
- Disciplined accents: cyan `#21C7CF`, fuchsia `#E34E9F`, copper `#B97550`, with substantial empty space. No arbitrary rainbow icon clutter or default bright yellow.
- Bespoke illustrations per concept. Maintain a reusable shape language without reusing the same scenes in every video.
- Original layout grammar: asymmetric but balanced compositions, anchoring diagrams, consistent baselines, controlled hierarchy, and carefully spaced typography.
- Handcrafted illusions through stroke order, timing offsets, animated cross-sections, mechanical unfolding, morphing diagrams, 2.5D depth, parallax and selective 3D.
- Show only one main idea at a time. Visual elements enter when narration needs them and exit when no longer relevant.
- Camera: controlled dolly/pan/zoom/reframe, not incessant movement. Cuts should land on meaningful conceptual beats.
- Typography: custom wordmarks/titles and two complementary type families; support full Unicode; text vector-rendered whenever possible.
- No generic AI imagery, mismatched faces, fake documentary footage, gibberish diagrams, repetitive stock wipes, endless slow zooms, random particle clouds, or text walls.
- Avoid pretending a procedural handwriting animation is literally human-drawn. The authoring standard is excellent motion design, not deception.

## 7.2 Shot classes

| Shot class | Best engine | Visual grammar |
|---|---|---|
| Ink-drawn process explanation | Blender Grease Pencil / curves | Stroke reveal + sequential labels + arrows |
| Causal chain / timeline | VectorCraft + Blender | Connected frames transform when cause advances |
| Graph, chart, measured data | VectorCraft + Blender | Plot accurate values from actual dataset; annotation reveals |
| Cross-section of object | Blender | Technical outline peels away into depth, snap back to flat diagram |
| Spatial/geometric concept | Blender or Unreal | 2D sketch becomes 3D camera-orbit proof then flattens |
| Dynamic system/simulation | Unreal only if justified | Stylized white materials and ink edges, physically explained action |
| Big conceptual reveal | Blender + EffectCraft | High-impact but restrained edit and morph |
| Credits / sources | VectorCraft + EffectCraft | Clear readable typography, provenance and AI tooling |

## 7.3 Originality safeguards

- Creative Director assigns a unique per-episode **visual thesis** (a central shape/metaphor derived from the topic).
- Storyboard artist identifies 3–5 bespoke visual moments that could not be reused for another unrelated episode.
- Check scene thumbnails and script embeddings against previously published episodes; flag high similarity and duplicated hooks/sections.
- Require a real explanation payoff in the first ~30 seconds and at least one surprising evidence-backed illustration.
- No slide-only videos; minimum animation coverage can be configured, but purposeful static pauses remain allowed for comprehension.
- Require narrative specificity, novel scene choreography, topic-relevant diagrams and varied pacing, not gimmicky randomness.
- A final Quality Supervisor rubric grades originality, instructional clarity, legibility, motion quality, audio quality and factual integrity; low-scoring videos are revised or withheld.

# 8. BLENDER CLI: PRIMARY ANIMATION FACTORY

The engine must generate reproducible Blender Python scripts or structured scene definitions from approved shot specs. Do not let the language model write ad hoc destructive terminal strings directly.

## 8.1 Workflow

1. `ShotSpec` contains concept, citation/claim IDs, on-screen text, geometry, transitions, camera, narration markers, scene duration, render engine/format.
2. Layout composer creates vector assets with VectorCraft, optionally SVG/curve import into Blender.
3. Blender script builds Grease Pencil drawing data or curve-based strokes, shape layers, mesh props, shader materials, camera rig, timeline keys and compositor effects. Pick implementation based on actual installed Blender API version; Grease Pencil changed across Blender versions.
4. Generate **very fast preview** first: simplified materials, low sampling, proxy resolution, short frames.
5. Motion Director and visual QA evaluate generated preview/frames; send revisions through the bus.
6. Render frame sequence (PNG or EXR as needed) using background jobs. Store frame metadata, project source and asset attribution.
7. Allow frame range rerender without invalidating whole film.

Example commands (paths illustrative; validate against local version):

```powershell
# Generate scene from a version-controlled script and shot-spec input
blender --background --python .\pipeline\blender\build_shot.py -- --spec .\jobs\vid_001\shot_003.json

# Render approved scene as an image sequence
blender --background .\jobs\vid_001\shot_003.blend --render-output .\jobs\vid_001\frames\shot_003_ --render-format PNG --render-anim
```

Required Blender capabilities: Grease Pencil/curve drawing reveal; bevel-to-ink strokes; SVG paths; graph axes; vector math; reusable cameras; geometry nodes where helpful; stylized outlines; 2.5D layers; smooth morph/interpolation; object cutaways; transparent render passes; orthographic camera; procedural scene templates; no dependency on mouse clicks/UI.

Official reference: [Blender command-line arguments](https://docs.blender.org/manual/en/latest/advanced/command_line/arguments.html).

# 9. UNREAL ENGINE CLI: SELECTIVE CINEMATIC/REAL-TIME PIPELINE

Unreal is a **purposeful supporting engine**. Use it for spatial reconstructions, visually complex demonstrations, controlled real-time simulations, and shots that cannot efficiently be conveyed in Blender's whiteboard approach. Keep consistent non-photorealistic art direction using white/ink materials, outline passes and concise fuchsia/cyan annotations.

Requirements:
- Detect installed Unreal executable(s), engine version(s), valid project `.uproject`, enabled Python Editor Script Plugin, Sequencer and Movie Render Queue plugins. Support version-specific scripts instead of assuming every command line flag works in every engine version.
- Build/modify assets, actor placement, materials, lights, cameras and sequences by Unreal Python when possible.
- Use **UnrealEditor-Cmd.exe** with suitable automation scripts; use Sequencer/Movie Render Queue command-line or custom Python executor for renders; output frame sequences and metadata.
- Keep all UE project modifications in a generated per-video content folder or throwaway project; never overwrite unrelated user's UE assets.
- Optimize for available VRAM; verify actual render memory usage before selecting 4K path tracing, Nanite or Lumen. Fall back to stylized standard rendering, 1080p or Blender when the 8 GB-class GPU cannot finish reliably.
- Asset audits ensure physical/technical models are correctly represented and labeled as illustrations—not evidence or captured footage.

Version-matched official guidance: [Unreal Movie Render Queue command-line rendering](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-command-line-rendering-with-move-render-queue-in-unreal-engine).

# 10. CREATIVE AUXILIARIES: VECTORCRAFT, PHOTOCRAFT, EFFECTCRAFT

Agent adapters shall use the **already installed headless binaries and MCP services** as supported by their current versions. Probe commands and capabilities at startup.

- **VectorCraft:** custom technical art, charts, pictograms, animated SVG components, title cards, character silhouettes, graphing coordinates, branded symbols, whiteboard illustration libraries; preserve edit-friendly vector originals.
- **PhotoCraft:** source photo cleanup, correction, documentary reference cutouts, lawful edits, texture prep, thumbnail composites. Never invent manipulations that convert an illustration into apparent event evidence.
- **EffectCraft:** motion graphics for animated titles, overlays, chapter cards, kinetic type, animated infographics, credits, scene composites. Leverage its JSON command/MCP workflow and render queue.

Illustrative public CLI paths from upstream docs (verify installed versions first):

```powershell
vectorcraft-cli --help
photocraft-cli --help
effectcraft-cli --help

# Agent connections may use their native stdio MCP modes
vectorcraft-cli mcp
photocraft-cli mcp
effectcraft-cli mcp
```

References: [VectorCraft](https://github.com/storytold/vectorcraft), [PhotoCraft](https://github.com/storytold/photocraft), [EffectCraft](https://getartcraft.com/apps/effectcraft) and [EffectCraft source](https://github.com/storytold/effectcraft).

# 11. KOKORO VOICE DIRECTION AND FULL AUDIO PRODUCTION

Kokoro must produce narration that sounds specifically directed for each film, rather than uninterrupted robotic paragraph reading.

- Version-detect the installed Kokoro backend and voice inventory. Do not assume the exact service or CLI name.
- Voice Director creates a pronunciation dictionary for proper nouns/technical terms, breath groups, pace markers, emphasis and pause markers.
- Synthesize **sentence/beat segments**, store source text and output per segment, and allow isolated retakes. Recombine sample-accurately with minimal clicks/pops.
- Use voice choices consistent with channel identity, and avoid exaggerated affect, pseudo-human filler and synthetic conversational dialogue unless the script calls for it.
- Generate alignment data (word or sentence timestamps) using TTS metadata or an optional local speech aligner; verify observed duration with ffprobe.
- Build a sound design language: subtle pencil/stroke cues, whiteboard clicks, paper movement, restrained sound effects, occasional memorable musical motifs. No stock whoosh every two seconds.
- Only use music/sounds the channel owns, creates, or is licensed to publish. Persist rights metadata and require a valid usage record.
- Quality checks: clipping, long silences, gaps, mouthful pacing, abrupt timbral changes between chunks, pronunciation of names, peak level, integrated loudness targets configured for output, caption sync.
- Produce narration-only and mixed audio stems for future corrections.

# 12. FILM STORYTELLING AND VIDEO EDITING

Standard episode structure (adapt to story, **not** a rigid repetitive template):

1. **Hook** — an accurate, concrete question/problem and vivid custom visual rather than an exaggerated clickbait promise.
2. **Context** — why the viewer should care now; date-sensitive trends clearly dated.
3. **Mechanism** — progressive visual explanation; pictures reveal one cause/effect at a time.
4. **Complication** — exceptions, what people commonly misunderstand, what remains unknown.
5. **Payoff** — resolve the hook with an instructive reveal or real-world implication.
6. **Clean conclusion** — concise takeaway, source/credit signoff; no padding.

Default runtime 6–10 minutes, configurable 3–20. Prefer information density and good pacing to filling a target duration. Shorts may be independently storyboarded from the researched episode, not just mindless recrops.

Script requirements: narration text, pronunciation notes, claims and citations, per-scene learning goals, exact on-screen text, rough timecodes, planned animation reveal, emotional beats, optional sound cues, asset requirements and editorial notes.

Assembly requirements: frame-accurate 16:9 1920×1080 30 fps delivery preset (4K selectable), loudness specification, safe margins, correct aspect ratio, color management, H.264/AAC MP4 default or user-chosen output, thumbnails at YouTube-supported dimensions, chapter times, `.srt`/`.vtt`, detailed source description. Preserve original project and per-shot exports for correction.

Keep transitions motivated: morphological connections are preferred over arbitrary crossfades and wipe presets. Every illustrative claim that uses real numeric data must render numerically accurate labels and axes.

# 13. UNCOMPROMISING QUALITY CONTROL

**There is no single reliable 'AI slop detector'.** Quality must be evaluated via a combination of measurable checks, editorial reviews and sample inspection. Zero tolerance for fabricated evidence, broken video, or missing credits. Perceived originality must be earned through craftsmanship.

## 13.1 Six mandatory gates

| Gate | Minimum expectation | Failure action |
|---|---|---|
| Research | Primary sources for major factual claims; dates/uncertainty preserved | Re-research/rewrite or reject topic |
| Script | Clear hook, original structure, no filler, coherent narrative, claims linked to sources | Script-editor rewrite |
| Storyboard | Every shot has a teaching purpose; original visual thesis; readable design | Regenerate boards, new visual metaphor |
| Animation | No collisions, pop-ins, broken strokes, bad interpolation, frame corruption | Rerender affected frames/shot |
| Audio | Speech audible/consistent, clean joins, sync, no clipping or dead air | Retake segment/remix |
| Release | Correct credits, disclosures, captions, references, rights and output specs | Withhold upload |

## 13.2 Automated inspection

- ffprobe verifies codec, length, FPS, audio presence, channel layout, frame dimensions and stream corruption where detectable.
- Frame sampler takes storyboard keyframes plus periodic samples, creates contact sheets and uses vision review where a capable model is available. The auditor checks diagram correctness, legibility, clipping, unusual overlaps, text misspellings, inconsistent object continuity, unexplained sudden style changes and visual repetition.
- Deterministic checks validate contrast, minimum text size at delivery resolution, safe margins, animated stroke continuity, transparent-background artifacts and enough hold time for labels.
- Audio tests check silence intervals, abrupt amplitude changes, clipping, relative music/narration level and caption timing.
- Semantic script review checks claim completeness and whether animation conveys the actual explanation rather than decorative movement.
- Duplicate detection compares transcript, structure, diagram hashes and shot embeddings to existing uploads.
- License/disclosure check verifies every externally sourced visual/audio asset's provenance and the presence of required end-credit and description language.
- Automated preview renders **must be inspected before full-resolution rendering**. A failing scene cannot be ignored merely because a deadline is close.

### Quality scoring

Score each category 0–5: research integrity, narrative coherence, teaching clarity, visual originality, animation craft, audio craft, pacing, typography/accessibility, metadata honesty. Require minimum 4 in every category and overall average ≥4.4, plus all hard checks passing, before automatic publish. These scores are a proposed internal rubric, not objective measurements of artistic quality. Allow manual adjustment of thresholds but log all overrides.

Track time spent per shot, revision count, cause of failure and quality improvement over time. Do not optimize for upload volume at the expense of quality.

# 14. YOUTUBE PUBLISHING + TRANSPARENT AI CREDITS

The system can produce a **publication package** with no YouTube login. To upload, the user must separately authorize their YouTube channel using official OAuth; never ask for a Google password and do not repurpose OpenCode/NVIDIA credentials as YouTube credentials.

Use official YouTube Data API (`videos.insert`) for uploads with private/unlisted/public/scheduled settings supported by the API. Check API-project verification/privacy restrictions, scopes, rate limits and upload processing before claiming success. Save YouTube video ID and publication URL only after confirmation.

Include:
- Title: specific, truthful, education-led, not sensational misrepresentation.
- Description: precise episode summary, chapter markers, important dated sources, permitted links, AI production note, rights/attribution where required.
- Thumbnail: custom VectorCraft/PhotoCraft concept image with high readability, no fabricated photographic proof.
- Captions: upload/attach when supported and authorized, plus downloadable VTT/SRT locally.
- Credits: short, readable animated credits card naming AI-assisted production and major tools used.

**Suggested on-screen credits:**

> ATLAS STUDIO — AI-assisted educational production. Research, writing, illustrations, animation, narration and editing were coordinated with AI agents. Visuals were created with Blender, Unreal Engine, VectorCraft, PhotoCraft and EffectCraft; narration with Kokoro. Sources and corrections: see description.

Only list tools actually used in that episode. The description can give a more detailed breakdown, and the archive should contain a complete **per-video provenance manifest**.

**Platform disclosure:** As of the reference documentation consulted, YouTube requires disclosure of realistically altered/synthetic content in certain cases, and its upload API exposes `status.containsSyntheticMedia`. Rendered whiteboard animation and production assistance do not automatically trigger every disclosure case. The publisher must evaluate current YouTube guidance before each upload; when requirements apply, set the platform disclosure field *in addition to* end credits. Always show the user what disclosure choice was made. No concealment of AI production.

**Monetization:** No promises. YouTube has stated that original AI-assisted storytelling can be eligible, while repetitive or mass-produced inauthentic content can be ineligible. Produce original educational value and maintain auditable provenance. Publishing success does not imply monetization eligibility.

References: [YouTube video upload API](https://developers.google.com/youtube/v3/docs/videos/insert), [disclosing GenAI content](https://support.google.com/youtube/answer/14328491), [YouTube creator clarification about inauthentic content](https://support.google.com/youtube/thread/356734251).

# 15. AUTONOMOUS EXECUTION STATES AND SCHEDULER

A video is a durable project with the following states:

```
IDEA_DISCOVERED
  -> TOPIC_APPROVED
  -> RESEARCHING
  -> EVIDENCE_APPROVED
  -> SCRIPTING
  -> SCRIPT_APPROVED
  -> STORYBOARDING
  -> STORYBOARD_APPROVED
  -> AUDIO_BUILD
  -> ASSET_BUILD
  -> PREVIEW_RENDER
  -> PREVIEW_QA
  -> FINAL_RENDER
  -> EDIT_AND_MIX
  -> RELEASE_QA
  -> READY_TO_PUBLISH
  -> UPLOADING
  -> UPLOADED_PROCESSING
  -> PUBLISHED
  -> METRICS_REVIEW
```

Every state may enter `REVISION`, `BLOCKED`, `PAUSED`, `CANCELED`, `FAILED_RETRYABLE` or `FAILED_TERMINAL` with reason and next action. User can pause/resume any queue, re-render a selected shot, force a fresh trend scan, or hold publication.

**Resource orchestration:**
- GPU lock with resource estimate; default only one Blender/Unreal heavy render at a time on RTX 4070 8 GB-class systems, unless measurements show concurrency is safe.
- CPU workers for sourcing, vector drawing, metadata and audio may run concurrently under configured caps.
- Dependency scheduler tracks required inputs. Never start final render on an unapproved storyboard or outdated script hash.
- Every command has explicit working directory, validated arguments, timeout, cancellation, sandboxed file roots, resource limits and structured stdout/stderr capture.
- Content-addressable caching avoids rerendering unchanged shots.
- On power loss, restart, OpenCode outage, or NVIDIA free-tier throttling, resume from last verified artifact rather than starting over.
- Uploads are idempotent: query saved video ID and processing state before any retry that could create a duplicate video.
- Support overnight or scheduled rendering with user-defined quiet hours and max GPU utilization.

# 16. CONTROL ROOM UI: PREMIUM, NOT VIBE-CODED

Build the **optional** desktop UI as a professional creative-production console. The headless CLI must remain 100% capable independently.

**Visual language:** polished editorial software, refined typography, black/near-black mode and warm-white light mode, well-managed spacing, dense functional views where needed, limited accent colors (cyan/fuchsia), no lazy endless cards, glowing orb mascots, arbitrary gradients, emoji-only navigation or fake metrics.

## Core screens

1. **Studio Overview:** current production stage, next topic, publishing calendar, health of agents, model quota state, rendering queue.
2. **Trend Radar:** evidence-backed candidates, source citations, freshness, ranking, competition/redundancy notes; approve/reject/ban topics.
3. **Production Board:** Kanban/list/timeline for each episode; critical path, parallel agent work, durable artifacts and revision loops.
4. **Writers' Room:** sourced research dossier, claim graph, editable narration, comments threaded to agent decisions.
5. **Storyboard Wall:** contact sheet with shot numbering, reveal timing and generated previews; compare revisions.
6. **Live Render Monitor:** Blender/Unreal tasks, frames, GPU use, errors, ability to pause, retry or cancel shot-level jobs.
7. **Edit Review:** native playback with script, captions, source claims and timeline annotations synced to timecode.
8. **Release Desk:** title, thumbnail, metadata, end credits, disclosure, YouTube authorization and publish queue.
9. **Agent Activity:** readable inter-agent messages with artifacts, status and reason for rejection; filter noise by severity.
10. **Settings:** dark/light/system themes, installed paths, OpenCode integration, model routing, audience/channel goals, render presets, auto-publish permissions.

## Quality-of-life requirements

Global search; command palette; keyboard shortcuts; autosave; undo/version history; adjustable dock panels; drag-and-drop references; queue pause/resume; task-specific notifications; focus mode; accessible text sizing; motion reduction; export/backup; multiple channels/profiles; per-episode budget limits; visual diff of storyboards; source quick-open; episode audit bundle download; scheduled publishing and blackout dates; offline library of produced assets; plugin/CLI diagnostics; safe errors with exact remediation.

Light/dark modes must have independently tuned contrast and surfaces, not merely inverted CSS variables. No yellow accent as a default.

# 17. CLI-FIRST AUTOMATION INTERFACE

Provide a single entrypoint with stable human-readable and JSON outputs. Commands are conceptual until implementation; do **not** pretend they already exist.

```powershell
atlas doctor                         # discover OpenCode, NVIDIA, Blender, Unreal, ArtCraft, Kokoro
atlas config show --redact           # secrets always redacted
atlas models probe                   # test model routes and current capacity
atlas agents status                  # agent health, current work, messages
atlas trends scan --region US        # trend research + scored topic cards
atlas topics create "How do batteries get recycled?"
atlas produce --topic <topic-id>     # execute full production workflow
atlas projects list --json
atlas projects inspect <video-id> --json
atlas storyboard preview <video-id>
atlas render shot <video-id> <shot-id> --quality preview
atlas render resume <video-id>
atlas qa run <video-id> --all
atlas export <video-id> --preset youtube-1080p
atlas publish <video-id> --privacy private
atlas publish schedule <video-id> --at <ISO-8601>
atlas daemon start                   # unattended scheduling/production worker
atlas daemon pause
```

Default automation policy is review-before-publish unless an enabled channel profile has explicitly allowed autonomous publication. Headless daemon must have Windows startup/service strategy, safe shutdown, logs and crash recovery. Expose REST/OpenAPI and event-stream interfaces for UI/other agents.

# 18. DATA CONTRACTS AND PROJECT LAYOUT

Representative shared video schema:

```json
{
  "video_id": "vid_20261015_0001",
  "topic": {
    "title": "How do batteries get recycled?",
    "trend_sources": [],
    "observed_at": "2026-10-15T00:00:00Z",
    "region": "US"
  },
  "state": "PREVIEW_RENDER",
  "channel_profile": "primary",
  "visual_thesis": "a traced battery unfolded into looping material flows",
  "script_artifact_id": "script_v4",
  "research_artifact_id": "dossier_v2",
  "storyboard_artifact_id": "boards_v3",
  "shots": [{
    "shot_id": "s003",
    "engine": "blender",
    "scene_template": "technical_unfold",
    "duration_frames": 270,
    "fps": 30,
    "claim_ids": ["c12", "c13"],
    "narration_segment_ids": ["n06", "n07"],
    "output_artifact_id": null,
    "status": "QUEUED"
  }],
  "publish": {
    "mode": "approval_required",
    "contains_synthetic_media": null,
    "credits_present": false,
    "youtube_video_id": null
  }
}
```

Suggested project tree:

```text
atlas-studio/
  apps/control-room-angular/
  apps/desktop-tauri/
  crates/core/
  crates/orchestrator/
  crates/agent-bus/
  crates/ai-broker-opencode/
  crates/trend-discovery/
  crates/research-rag/
  crates/claim-audit/
  crates/shot-schema/
  crates/render-scheduler/
  crates/qa/
  crates/youtube-publisher/
  workers/vectorcraft/
  workers/photocraft/
  workers/effectcraft/
  workers/blender/
  workers/unreal/
  workers/kokoro/
  workers/ffmpeg/
  prompts/<agent-name>/system.md
  prompts/<agent-name>/output.schema.json
  styles/whiteboard-design-system/
  templates/blender/
  templates/unreal/
  tests/fixtures/
  docs/architecture/
  docs/production-bible/
  data/atlas.db
  data/artifacts/<project-id>/<asset-sha>/
  data/exports/<project-id>/
```

# 19. REALISTIC MILESTONES — BUILD IT IN THE RIGHT ORDER

**Milestone 0 — system probe and integration contract**
- Discover installed executables/versions, OpenCode active NVIDIA account via supported method, available models and quotas, Unreal project support, Blender CLI, Kokoro and ArtCraft headless capabilities.
- Create safe shell execution layer and failure reports.
- **Pass:** test calls/render/synthesis with no exposed credentials.

**Milestone 1 — real single-video vertical slice**
- User supplies topic; DuckDuckGo research; cited 2-minute script; storyboard; VectorCraft diagrams; Blender CLI animation; Kokoro narration; FFmpeg assembly; credits; local MP4.
- **Pass:** an actual watchable, narrated, edited, sourced video. No mock demo replacing the workflow.

**Milestone 2 — agent bus and automatic revisions**
- Add producer/research/writer/artist/animator/fact/quality roles; persistent handoffs, feedback loops, crash recovery, revision tracking.
- **Pass:** failed scene is independently detected and rerendered without human glue code.

**Milestone 3 — full craft pipeline**
- Integrate PhotoCraft, EffectCraft, Unreal selective shots, 6–10-minute story structure, original thumbnails, captioning, consistent style and scene-level preview inspection.
- **Pass:** multiple complete episodes of unrelated topics have clearly distinct original visual theses and no technical QA defects.

**Milestone 4 — automatic trend editorial desk**
- Google Trends RSS, YouTube chart metadata, DuckDuckGo, ranking, topic novelty, script/source freshness review.
- **Pass:** autonomously proposes and produces a genuinely trend-linked video with auditable evidence of when/where it trended.

**Milestone 5 — publication automation and desktop experience**
- OAuth connection, private upload, status checks, metadata/credits/AI disclosure, approval flow, scheduling, Angular/Tauri control room.
- **Pass:** verified private upload, public/scheduled only on explicit auto-publish permission; no duplicate publication on retries.

**Milestone 6 — unattended hardened operations**
- Measured quality thresholds, efficient GPU queue, robust quotas, batch operation, post-release analytics, drift/duplicate prevention and recovery.
- **Pass:** multiple end-to-end episodes can move from trend scan to finished publication unattended; failed gate halts the episode, logs cause and continues with a different valid candidate where appropriate.

# 20. NON-NEGOTIABLE ACCEPTANCE TESTS

1. New user can run `atlas doctor` and see accurate local tool availability and version, including **OpenCode's current NVIDIA account** without typing a duplicate key.
2. Secret scanning confirms the NVIDIA key is absent from SQL logs, exported manifests, scripts, command lines, agent transcripts, project archives and rendered frames.
3. A new verified trend produces a story with source links, observation timestamps, original explanatory angle and correct referenced dates.
4. A 2–3-minute vertical-slice video actually renders from source to MP4 via headless Blender and Kokoro.
5. The agents send review/rework messages with typed artifacts; no shared state depends exclusively on chat context.
6. A deliberately wrong numeric diagram causes QA failure and a targeted fix; audio/scene revisions do not require full project regeneration.
7. A Blender worker crash does not corrupt other scenes or lose the project; restarting resumes from durable state.
8. Unreal is **optional per shot**; Blender-only production works if Unreal is absent, busy or memory constrained.
9. Each published video includes actual source links, AI-assisted production credits, and disclosure metadata when applicable.
10. Originality checker rejects obviously reused scripts/scenes; at least three unique meaningful illustrations per episode.
11. Exports have synced narration, readable text, verified dimensions/framerate/audio, generated subtitles and attractive custom thumbnail.
12. Full production is possible with UI closed through a headless daemon; user can optionally view the event stream with a light/dark control room.
13. Default publishing never makes an unapproved video public; explicit auto-publish mode can release only QA-passed videos under the selected account.
14. No unsupported claims of monetization, research certainty, human authorship, or guaranteed invisibility of AI assistance.
15. Free-tier API exhaustion results in clean pause/queuing/resume, not false completion, runaway retries or new chargeable credentials.

# 21. CODING AGENT DIRECTIVE

> Build **ATLAS STUDIO** as a real autonomous, headless-first, agentic YouTube educational filmmaking pipeline. **Do not build the old ATLAS Academy.** The studio must discover current trending topics, research primary references, fact-check every substantive claim, create original whiteboard-illustration-style teaching scripts and storyboards, animate through the locally installed Blender CLI as the primary renderer, selectively add Unreal Engine cinematic/simulation shots, and integrate the already-installed VectorCraft/PhotoCraft/EffectCraft CLI tools and Kokoro TTS. A persistent AI-agent team must communicate through an audited typed message bus and revise each other's work. The primary inference provider is free NVIDIA NIM models, reusing the active NVIDIA provider credential already configured inside OpenCode through supported interfaces without making the user re-enter it or exposing it in logs. Build frame-accurate video/audio editing, robust source provenance, objective preview/release QA, polished thumbnails, captions, readable AI-production end credits, and the appropriate YouTube altered/synthetic media disclosures. Support autonomous YouTube scheduling and publication only after channel OAuth and an explicit opt-in. Deliver a functioning Windows CLI/daemon first, then a premium light/dark Rust+Angular/Tauri production control room. Use strict quality gates to reject repetitive, shallow or unverified output. No placeholder production features, fake renders, simulated success statuses, or architecture-only completion claims.

---

# 22. SOURCE/INTEGRATION REFERENCE INDEX

- Blender CLI manual: https://docs.blender.org/manual/en/latest/advanced/command_line/arguments.html
- Unreal Engine MRQ CLI: https://dev.epicgames.com/documentation/en-us/unreal-engine/using-command-line-rendering-with-move-render-queue-in-unreal-engine
- OpenCode provider credentials (legacy): https://dev.opencode.ai/docs/providers
- OpenCode v2 provider storage/accounts: https://dev.opencode.ai/v2/docs/cli/providers/
- OpenCode local server: https://dev.opencode.ai/docs/server/
- NVIDIA NIM model catalog: https://build.nvidia.com/models
- NVIDIA NIM quickstart: https://docs.api.nvidia.com/nim/docs/api-quickstart
- Google Trends official RSS export: https://support.google.com/trends/answer/3076011
- YouTube `videos.list` trending-popular chart: https://developers.google.com/youtube/v3/docs/videos/list
- YouTube `videos.insert`: https://developers.google.com/youtube/v3/docs/videos/insert
- YouTube disclosure help: https://support.google.com/youtube/answer/14328491
- YouTube creator clarification on inauthentic content: https://support.google.com/youtube/thread/356734251
- VectorCraft: https://github.com/storytold/vectorcraft
- PhotoCraft: https://github.com/storytold/photocraft
- EffectCraft: https://github.com/storytold/effectcraft

**End of specification.**
