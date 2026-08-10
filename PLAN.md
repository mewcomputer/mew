# Orchestration improvements: fan-in, guardrails, todo/execution join, durability, typed handoffs

Implements paths A–E from `docs/development/dev-orchestration.md`, staged so each
phase lands independently with tests. F (full scheduler) is deliberately out of
scope.

## New configuration

All phases read from one new config section in `mew-config::Config`
(`crates/mew-config/src/lib.rs`), with `#[serde(default)]` so existing configs
are untouched. `MEW_ORCHESTRATION__*` env overrides come free from the existing
`MEW_` layering.

```toml
[orchestration]
max_concurrent_subagents = 4     # 0 = unlimited (opt out of the cap)
max_subagent_depth = 1           # children also need `can_spawn: true` to nest
default_max_duration_secs = 300  # overrides the built-in per-run wall-clock default
leak_reminder = true             # remind the model about uncollected tasks at turn end
leak_reminder_max = 2            # max reminder loop-backs per user turn
```

Plumbing: `build_session_agent` passes an `OrchestrationConfig` clone into new
`Agent` fields (same pattern as `subagent_runner`/`subagent_defs`). The runner
receives the pieces it needs via `SimpleRunner::with_*` setters.

## Phase 1 — fan-in and leak reporting (doc item A)

1. **`subagent_wait` batch mode** (`crates/mew-tools/src/tools/subagent_wait.rs`,
   `crates/mew-agent/src/tools.rs::execute_subagent_wait`):
   - Schema gains `task_ids: string[]` and `all: boolean` alongside the existing
     `task_id`. Exactly one of the three must be present.
   - Agent-side: wait each task, collect into a single result keyed by task_id:
     `{"<id>": {"status": "complete|cancelled|error", "text": ...}}`. One failed
     task does not fail the batch; per-task status carries it. `all: true` waits
     every outstanding task.
   - Update the `subagent_start` description to mention batch collection.
2. **Turn-end leak reminder** (`crates/mew-agent/src/turn.rs`):
   - After `dispatcher.on_turn_end`, where goal continuation already injects a
     synthetic user message and loops back, add the same move for uncollected
     subagent tasks: if `list_subagents()` is non-empty and `leak_reminder` is
     on, inject a synthetic message listing task_ids, def names, and elapsed
     time, instructing the model to collect or explicitly abandon them.
   - Guard with a per-user-turn counter (`leak_reminder_max`, default 2) so a
     model that keeps spawning instead of collecting cannot loop forever.
     Counter resets on each real user turn.
3. Tests (`mew-agent/src/tests.rs` style): batch wait returns per-task statuses
   including a failed task; `all: true` drains everything; reminder fires once
   with running tasks, does not fire when none, stops after `leak_reminder_max`.

## Phase 2 — guardrails: concurrency cap and explicit depth policy (doc item B)

1. **Concurrency cap** (`crates/mew-agent/src/agent.rs::start_subagent`):
   - Before spawning, count entries in `subagent_tasks`. At or above
     `max_concurrent_subagents` (when > 0), return a structured error:
     "concurrency cap reached (N running); call subagent_wait to collect
     results first". The tool surfaces this as a normal tool error so the model
     can react.
2. **Explicit depth policy** (`mew-subagents`, `mew-agent::runner`,
   `crates/mew/src/setup/agent.rs`):
   - Subagent frontmatter gains `can_spawn: bool` (default false), parsed into
     `SubagentDef`.
   - `SubagentRunOptions` gains `depth: u32` (parents are depth 0). The runner
     includes `subagent_start`/`subagent_wait` in the child's tool map only when
     `def.can_spawn` and `depth + 1 < max_subagent_depth`, and gives the child a
     nested runner + defs when so.
   - Setup ordering fix: register `subagent_start`/`subagent_wait` into
     `agent.tools` *before* constructing `SimpleRunner`, and let the runner
     filter per-def instead of relying on construction order. This replaces the
     current accidental depth-1 enforcement with an explicit, tested policy.
3. Tests: cap rejects the N+1th spawn and succeeds after a wait frees a slot;
   cap 0 = unlimited; a `can_spawn` def at depth 0 gets the tools, a non-
   `can_spawn` def does not, and nothing gets them past `max_subagent_depth`.

## Phase 3 — join todos to execution (doc item C)

1. `subagent_start` schema gains optional `todo_id: integer`; the agent records
   `(todo_id, task_id)` in the task registry entry.
2. `todo_list` output annotates linked todos (`#3 — in progress — Curie,
   running 42s`) and the completion message for a collected task suggests
   marking the linked todo done (suggestion, not auto-transition: the model
   still owns state changes).
3. Linked state shows up in the leak reminder from Phase 1.
4. Sidebar (TUI) shows the todo id next to the subagent entry if the link
   exists — small `mew-tui` change reading data already on the wire.
5. Tests: link recorded and visible in todo_list; suggestion text appears on
   collection; unknown todo_id is rejected with a clear error.

## Phase 4 — durable orchestration state (doc item D)

1. Persist the task registry to `<session>/subagent_tasks.json` (sibling of
   `todos.json`): task_id, def name, linked todo_id, status, child session id,
   started_at. Written on spawn/collect/cancel.
2. On session resume: reload the file; tasks marked running are reclassified as
   `orphaned` (their process died with the session) and surfaced to the model
   in the first turn's context; completed-but-uncollected results are
   recoverable from the child session transcript where possible, else marked
   `lost`.
3. Tests: round-trip persistence; resume surfaces orphaned tasks; collected
   tasks are removed from the file.

## Phase 5 — typed handoffs (doc item E)

1. Subagent frontmatter gains optional `output_schema` (JSON Schema, inline or
   `@path` relative to the def file).
2. Runner validates the child's final message as JSON against the schema; on
   failure the child gets exactly one corrective turn containing the validation
   error. Second failure returns the raw text with a `schema_invalid` warning
   prepended (never silently drops output).
3. `jsonschema` crate for validation (new dependency, one small crate).
4. Tests: valid output passes through untouched; invalid output triggers one
   corrective turn; still-invalid output returns with warning.

## Settings summary (what the user can tune)

| Setting | Default | Phase |
|---|---|---|
| `orchestration.max_concurrent_subagents` | 4 | 2 |
| `orchestration.max_subagent_depth` | 1 | 2 |
| `orchestration.default_max_duration_secs` | 300 | 2 |
| `orchestration.leak_reminder` | true | 1 |
| `orchestration.leak_reminder_max` | 2 | 1 |

Plus two subagent-def frontmatter fields: `can_spawn` (phase 2),
`output_schema` (phase 5).

## Verification

Per phase: `cargo test -p mew-agent -p mew-tools -p mew-subagents -p mew-config`,
then `cargo clippy --all -- -D warnings`, `just arch-check`, and `cargo fmt`
before committing. Each phase is its own commit on a WIP branch.

## Explicitly out of scope

- Path F (runtime scheduler executing the todo dependency graph). Revisit after
  A–E bake.
- Web UI surfacing of todo/subagent links (TUI only in phase 3).
- Cost/token budgeting across fan-outs.
