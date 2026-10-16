# End-to-end production DAG

## Pipeline
1. **DISCOVER** — trend-scout + trend-analyst find dated educational opportunity; generate approved charter
2. **RESEARCH** — research-lead → fact-checker → subject-editor; source-backed claim ledger
3. **WRITE** — lead-writer → script-editor → independent fact-checker
4. **DESIGN** — creative-director → storyboard-artist → motion-director; preserve exact beat IDs
5. **ASSETS** — vector-illustrator and photo-editor independent asset generation
6. **VOICE** — kokoro-narrator → audio-editor produce timed stems; timing changes invalidate boards
7. **RENDER** — blender-animator; unreal-cinematographer only with justification; effectcraft overlays
8. **EDIT** — video-editor conform and produce exact published scene timeline; accessibility-editor captions
9. **QUALITY** — independent quality-supervisor + rights-disclosure validate gates
10. **PUBLISH** — publisher private draft by default, public only with existing channel opt-in and all gates
11. **LEARN** — analytics-ingestion after data matures → retention-analyst → experiments → recursive optimizer

## Rules
Use typed task DAG with hash-based caches and state transitions. Parallelize illustration and narrated audio only after script lock; isolate GPU-heavy Blender/Unreal. Failure loops return to the **earliest** invalid artifact, invalidate dependent stages and cap revisions at 3. Never auto-publish while gate is pending.

## Exit evidence
A completed workflow emits immutable artifact hashes, review gate statuses, agent/event trace IDs, and clear failure records. Do not substitute a successful LLM response for a verified native asset, fetched metric or upload confirmation.
