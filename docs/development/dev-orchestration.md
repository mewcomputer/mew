# Tasks and orchestration: current state and improvement paths

Status: assessment + proposal, no implementation yet.

## What exists today

### Task tracking (sufficient)

`mew-agent::todos` is a session-lived todo list with real semantics:

- `pending / in_progress / done / blocked` statuses
- `depends_on` edges; completion is refused until dependencies are done
- persisted to `<session>/todos.json`, survives compaction and resume
- shared surface: the model drives it via `todo_*` tools, the user via `/todo`

As a task list this is done. The limitation is that the dependency graph is
passive: it gates *marking* work done, it does not *execute* work.

### Orchestration primitives (good primitives, no system)

Subagents (`mew-subagents` + `mew-agent::runner`):

- Markdown defs with frontmatter: model pin or router tier (`nano`/`micro`/`deci`),
  tool allowlist, `max_turns`, `max_duration_secs`, templated bodies
- Each run gets a child agent with its own session file, cancellation token,
  turn manifests, and `SubagentStatus` events for the UI
- Tool surface: `subagent_start` (sync by default, blocks and returns the
  result inline; `async=true` returns a `task_id`), `subagent_wait`, plus
  cancel and list operations on the agent

Adjacent machinery that already exists and matters for orchestration:

- goals: `propose_goal` / `complete_goal` / `block_goal` with user approval
  and turn-loop continuation
- personas with model/tool policy and transitions
- hooks: dispatcher observation/mutation points including `on_subagent_start`
- background shell jobs using the same task_id/wait pattern as subagents

## The gaps

Ordered by how much they limit real orchestration today.

1. **Orchestration is model-driven, not runtime-driven.** The model manually
   sequences `subagent_start`/`subagent_wait` and tracks task_ids in its own
   context. The todo dependency graph is never scheduled. Under compaction or
   a long fan-out, the orchestration plan lives in the most fragile place it
   can: the model's rolling context.

2. **No fan-in primitive.** There is no `wait_all` / `wait_any`. The model must
   remember every task_id it spawned and wait each one. A forgotten task_id
   means orphaned work running until its turn/time cap, with no leak reporting
   at turn end.

3. **Unbounded fan-out.** No concurrency cap, admission control, or cost/token
   ceiling across a parallel batch. Nothing stops a runaway plan from spawning
   dozens of children.

4. **Orchestration state is in-memory.** `Agent::subagent_tasks` is an
   `Arc<Mutex<HashMap>>`. Todos survive session resume; running tasks and the
   task registry do not. A resumed session cannot re-attach to or reap work it
   started.

5. **Todos and subagents are disconnected.** There is no link between todo #3
   and the task executing it. The dependency data and the execution data exist
   in two separate structures that never join. The UI reflects this: sidebar
   subagent list and todo list are independent.

6. **Free-text handoffs.** `SubagentResult::Complete.text` is unstructured.
   No output schema, no validation, no retry-on-malformed-result loop. The
   parent model is the only consumer and the only validator.

7. **Depth-1 only.** The runner is constructed before `subagent_start` is
   registered into the parent's tool map, so children never receive it.
   This is a sane default guardrail, but it rules out orchestrator-subagent
   patterns (a subagent that manages its own fan-out) even where a def would
   explicitly opt in.

## Verdict

Sufficient as a delegation primitive: spawn helpers, cap them, cancel them,
collect text. Not sufficient as orchestration: there is no scheduler, no
durable orchestration state, no fan-in, and no typed contracts between
coordinator and workers. Every gap above is currently papered over by model
discipline and prompt hints, which degrades exactly when orchestration gets
long or parallel enough to be worth having.

## Improvement paths

These are independent and roughly ordered cheapest-first. None requires
rethinking what exists; they compose.

### A. Runtime-side fan-in (small, high value)

- `subagent_wait` accepts an array of task_ids, or a `wait_all: true` mode
  that drains every outstanding task and returns results keyed by task_id.
- At turn end, the agent reports still-running tasks to the model (a synthetic
  reminder, or a warning if any task has been running past a threshold). This
  kills the orphaned-work failure mode without a scheduler.

### B. Orchestration guardrails (small)

- Configurable concurrency cap on concurrent subagent runs per session
  (`orchestration.max_concurrent_subagents`, default ~4). `subagent_start`
  past the cap returns a structured error telling the model to wait first.
- Depth limit stays at 1 unless a subagent def explicitly opts into receiving
  `subagent_start` (frontmatter `can_spawn: true`), at which point depth is
  enforced (e.g. max 2) rather than left open.

### C. Join todos to execution (medium)

- `subagent_start` accepts an optional `todo_id`; the agent records the link
  and reflects it in `todo_list` output and the sidebar ("#3 — Curie,
  running"). When a linked task completes, the todo transitions are suggested
  to the model rather than auto-applied.
- This turns the passive dependency graph into something a scheduler (below)
  or the model can act on with far less context bookkeeping.

### D. Durable orchestration state (medium)

- Persist the task registry (`task_id`, def name, todo link, status, child
  session id) next to `todos.json`. On resume, finished-since tasks' results
  can be collected; orphaned running tasks are at least visible and
  cancellable instead of invisible.
- Child session files already exist per run, so this is bookkeeping, not new
  infrastructure.

### E. Typed handoffs (medium, opt-in)

- Subagent defs gain an optional `output_schema` (JSON Schema). The runner
  validates the child's final text (as JSON) against it; on failure the child
  gets one corrective turn with the validation error before the result is
  returned to the parent.
- Keeps free text as the default; schemas only where the caller consumes
  programmatically.

### F. A real scheduler (large, only if A–E prove the need)

A `plan_run`-style tool that takes a todo subgraph (or an explicit step list
with dependencies), executes ready steps in dependency order up to the
concurrency cap, fans in results, and surfaces failures with retry policy.
This is the point where orchestration becomes runtime-driven. Worth building
only after A–E, because they define the semantics (caps, links, schemas,
durability) a scheduler would need anyway.

## Status (2026-08-09)

Paths A–E are implemented on `wip/orchestration-improvements` and reviewed
(two code-reviewer passes; see CURRENT.md for the fix list). What shipped:
`subagent_wait` batch mode + turn-end leak reminder (A); concurrency cap,
`can_spawn` depth policy, `default_max_duration_secs` (B); `todo_id` linking
(C); `<session>/subagent_tasks.json` registry with orphan surfacing (D);
`output_schema` validation with one corrective turn (E). All tunable via the
`[orchestration]` config section. F remains out of scope.

## Design note: subagent cancel and fork tools

Not implemented — this is the agreed direction, written down so the semantics
survive context. The bias is deliberate: wait is the default, and the model
should not be able to casually discard work.

### Rationale: cancel is for wrongness, not slowness

Agents overuse cancel when a subagent is merely slower than expected. The
current design already resists that:

- The model has no cancel tool today. `cancel_subagent` exists only as a
  user-side dispatch command (and the daemon target returns `Unsupported`
  today — half-stubbed).
- The concurrency cap and leak reminder force the model to wait and collect
  before spawning more, which is the common case.
- The model can already distinguish "slow but progressing" from "stalled":
  subagent text streams through `AgentEvent::ToolProgress` and the tool-call
  state updates as the child runs.

So the rule: a bare cancel is the last resort, not the first reaction to
impatience.

### Proposed shapes

1. **`subagent_nudge` / interrupt (soft-first).** Injects a message into the
   live child session ("wrap up — the parent needs your result now"). The
   child keeps its transcript; the run continues to its normal completion.
   Cheap, non-destructive, and covers the "slow" case without killing work.
   Hard cancel stays available as the second resort.
2. **`subagent_cancel` (non-destructive).** Kills the child's cancel token
   (plumbing already exists: `Agent::cancel_subagent`, per-task child tokens).
   Cancelled runs already surface partial results — partial manifests ride on
   `SubagentEnd`, and the registry recovers the child transcript via
   `recover_child_text` — so cancel must return "here is what it produced
   before it stopped", never a silent void.
3. **`subagent_fork` = restart from the child's transcript.** Seed a new child
   session with the old child's messages plus a new prompt. Plumbing already
   exists: child transcripts persist under `<session>/subagents/<child_id>`,
   the registry records `child_session_id`, and `recover_child_text` reads
   them. Fork duplicates context, not processes.
4. **Combine "cancel and refork with what it produced"** as the high-use
   action. A bare cancel with no replacement plan is what gets overused;
   tying cancellation to a continuation makes the cost visible and the
   partial output useful.

### Open question: opt-in like `can_spawn`?

`can_spawn` set the precedent: nesting is opt-in via frontmatter. Decide
whether cancel/fork should be per-def opt-in too (`cancelable: true` /
`forkable: true`), which would also curb overuse by construction, or available
for every subagent. Lean: opt-in, matching `can_spawn`.

### When to build

Not until the wait-first semantics have proven themselves in real use. The
cheap first step when we do build: wire `subagent_nudge` (smallest, pure win,
no destruction) and decide the opt-in question. Cancel/fork ride on the same
tool-schema and def-frontmatter surface as everything else in this doc.

## Recommendation

## Recommendation

Do A + B now; they are small and remove the two worst failure modes (orphaned
work, runaway fan-out). C and D are the real unlock because they move
orchestration state out of the model's context and into durable structures.
E is cheap insurance for programmatic consumers. F should wait until usage
shows A–E aren't enough.
