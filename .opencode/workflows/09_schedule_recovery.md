# Unattended operations and recoverable state

## Pipeline
1. **PLAN** — Producer schedule from topic freshness, available model and render budget
2. **LEASE** — Dispatcher grants worker leases with hash-idempotency keys
3. **RENDERLOCK** — single heavy local GPU job at a time on mid-range laptop unless proven safe
4. **MONITOR** — health check for process exit, OAuth, NIM quota, disk and missing frames
5. **RETRY** — targeted bounded retries, preserving good artifacts
6. **PAUSE** — operator pause/resume/steer/cancel honored at safe boundaries
7. **SLEEP** — idle gracefully when no viable topic or API quota is available

## Rules
No invented time estimates. Handle Windows process boundaries and file-path quoting. Verify stdout/stderr are sanitized.

## Exit evidence
A completed workflow emits immutable artifact hashes, review gate statuses, agent/event trace IDs, and clear failure records. Do not substitute a successful LLM response for a verified native asset, fetched metric or upload confirmation.
