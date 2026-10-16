# Installation into an existing coding/agent harness

ATLAS is a **skills specification pack**, not an automatically registered live tool/plugin. Adapt to installed agents. Do not claim this creates a functioning autonomous YouTube channel by itself.

## Recommended deployment
1. Clone/copy `skills/*/SKILL.md` into the harness's supported skill path; or point the harness's skill loader at this directory. Each has readable YAML frontmatter, role, prerequisites and task contracts.
2. Load `config/agents.json` to register the agent roster, routing and review independence. Instantiate workers via the runtime's message bus.
3. Validate messages and reports against `schemas/*.schema.json` at process boundaries. Use immutable files and SQLite WAL ledger as the durable source of truth.
4. Run local discovery (`opencode --version`, `blender --version`, and each installed `*-cli --help`) and probe Unreal Engine path and Kokoro implementation. Capture findings in adapter config; never assume commands by filename alone.
5. Set up YouTube OAuth app separately for `yt-analytics.readonly` / `youtube.readonly`; add `youtube.upload` or youtube scope ONLY when publishing is authorized.
6. Run deterministic sample analysis: `python scripts/atlas_retention.py --report examples/retention_api_report.json --timeline examples/scene_timeline.json --out reports/sample_retention_review.json`
7. Run `python -m unittest discover -s tests -v` and `python scripts/atlas_gate.py examples/episode_manifest.json` before any active automation.
8. Connect workflow stages in order: `workflows/01_...` to `09_...`; start with review-before-publish. Enable unattended public posting only after user consent and verified gate behavior.

## Integration boundaries
- Native Rust/Tokio + SQLite worker runtime not implemented in this pack; build it using the contracts.
- Blender / Unreal / ArtCraft application invocations are prescribed skill behaviors; local CLI versions must be detected and adapters coded by implementation agent.
- Scripts are **reference utilities** for API collection with a supplied OAuth token, normalized retention analysis, editorial experiment proposals and publish-gate checking. They do **not** upload videos or run all agents.
- If skills loader demands a different frontmatter format, translate skill manifests, but preserve the skill semantics and quality requirements.

## Sample commands
```powershell
python .\scripts\atlas_retention.py --report .\examples\retention_api_report.json --timeline .\examples\scene_timeline.json --out .\reports\example.json
python .\scripts\atlas_experiment.py --review .\reports\example.json --out .\reports\example_experiment.json
python .\scripts\atlas_gate.py .\examples\episode_manifest.json
python -m unittest discover -s tests -v
```
