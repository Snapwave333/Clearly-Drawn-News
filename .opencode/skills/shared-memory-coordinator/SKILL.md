---
name: shared-memory-coordinator
description: Orchestrates dual-tier agent memory using Shared Context MCP for live working state and Graphiti for permanent knowledge recall and promotion.
version: 1.0.0
role: coordinator
tools: [atlas-cli, shared-context-mcp, graphiti-mcp]
requires: [memory_config]
---

# Shared Context + Graphiti Dual-Memory Protocol

## Mission
Ensure flawless coordination across the **OpenCode**, **Hermes**, and **OpenClaw** swarm without token context bloat or cross-session amnesia. 

The architecture strictly decouples:
1. **Short-Term Working Memory (Shared Context MCP):** Real-time task board, heartbeat/lease locks, inter-agent chat bus, and ephemeral scratchpads.
2. **Long-Term Episodic Memory (Graphiti MCP):** Temporal knowledge graph, architectural decisions, verified patterns, resolved pitfalls, and semantic retrieval.
3. **The Promotion Gate:** Automatic or deliberate transition of completed task learnings from Shared Context into Graphiti.

---

## 3-Phase Execution Lifecycle

### Phase 1: Task Priming & Recall (Graphiti Bootstrap)
Before taking action or writing plans:
1. Query Graphiti for existing knowledge graph entities, past decisions, and known gotchas:
   ```bash
   atlas memory recall "<topic or feature query>" --limit 5
   ```
2. Alternatively, preload and bootstrap a new task in one step:
   ```bash
   atlas memory preload --topic "<topic>" --agent "<agent-name>"
   ```
3. Read the injected priming notes from the working task board:
   ```bash
   atlas memory tasks --status in_progress
   ```

### Phase 2: Live Swarm Coordination (Shared Context MCP)
While active on a task:
- Register agent heartbeat and task assignment:
  ```bash
  atlas memory create-task --title "<Title>" --description "<Objective>" --agent "<AgentName>"
  ```
- Post working progress, intermediate tool outputs, and design notes to the task scratchpad:
  ```bash
  atlas memory add-note --task "<task_id>" --note "<Decision or progress note>"
  ```
- **Rule:** Never dump raw intermediate tool logs or conversational scratchpads directly into Graphiti. Keep all high-frequency churn within Shared Context.

### Phase 3: The Promotion Gate (Graphiti Commit)
When a task finishes and passes review:
1. Update status to `completed`:
   ```bash
   atlas memory update-task --task "<task_id>" --status completed
   ```
2. Promote the distilled knowledge payload (verified artifacts, decisions made, pitfalls avoided) into Graphiti:
   ```bash
   atlas memory promote --task "<task_id>" --notes "<Verified takeaway or post-mortem note>"
   ```
3. Or run batch auto-promotion across all completed pipeline items:
   ```bash
   atlas memory auto-promote
   ```
4. Prune expired ephemeral scratchpads older than retention threshold:
   ```bash
   atlas memory prune --days 7
   ```

---

## Swarm Agent Responsibilities

| Agent | Role in Dual Memory | Memory Actions |
| :--- | :--- | :--- |
| **Hermes** | Swarm Planner & Evaluator | Queries Graphiti for precedents; defines tasks in Shared Context; triggers promotion gate upon review. |
| **OpenCode** | Core Builder & Implementer | Claims tasks; posts execution notes & artifact IDs to Shared Context scratchpad; verifies code passes test suite. |
| **OpenClaw** | Research & Data Integrator | Queries external APIs/web; seeds Shared Context with vetted facts; invalidates outdated edges in Graphiti. |

---

## CLI Reference Cheat Sheet

```bash
# Check connectivity and stats for both tiers
atlas memory status

# Query long-term knowledge graph
atlas memory recall "Grease Pencil stroke reveals"

# View active working memory board
atlas memory tasks

# Promote completed task to permanent memory
atlas memory promote --task task_abc123 --notes "Solved LUFS normalization with 2-pass filter"

# Launch 24/7 background bridge daemon
atlas memory daemon --poll-seconds 15
```
