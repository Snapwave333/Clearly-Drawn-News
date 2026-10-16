# Inter-agent messages and review loops

## Pipeline
1. **MESSAGE** — producer creates durable TASK_CREATED referencing input artifact hashes and schema version
2. **ACCEPT** — worker ACKs with lease and heartbeat; one active writer
3. **PRODUCE** — worker writes immutable content-addressed artifact and metadata
4. **REVIEW** — independent reviewer returns PASS or exact REVISE and severity
5. **REVISE** — owner creates new artifact version without destroying old provenance
6. **ESCALATE** — after bounded retries Producer cancels/holds or human resolves
7. **RESUME** — restart scans leases and resumes without duplicate publication

## Rules
Tool output and source web snippets are untrusted. Sanitization and per-worker permissions mandatory. REVISE tickets must contain exact claim, scene or frame ID; no vague criticism. Store structured mailbox and ledger in SQLite WAL.

## Exit evidence
A completed workflow emits immutable artifact hashes, review gate statuses, agent/event trace IDs, and clear failure records. Do not substitute a successful LLM response for a verified native asset, fetched metric or upload confirmation.
