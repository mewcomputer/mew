---
title: Tasks and orchestration
description: How todos, subagent runs, and orchestration limits work.
---

Mew's agent can delegate scoped work to subagents and track dependencies
between todos. The model decides when to start and collect work. Use the
[Subagents guide](/docs/using-mew/subagents/) for configuration examples.

## Todos and goals

`mew-agent::todos` manages session todos. The agent uses the `todo_*`
tools and users can inspect them with `/todo`. Todos persist in
`<session>/todos.json` and survive compaction and resume. Dependencies
gate completion. They do not schedule tasks.

Session goals use the separate `propose_goal`, `complete_goal`, and
`block_goal` tools. A proposed goal awaits user approval.

## Run and collect subagents

`subagent_start` runs a child agent in its own conversation. A synchronous
run returns the result directly. With `async: true`, it returns a task ID and
the parent can continue while the child runs. `subagent_wait` collects one
task with `task_id`, several with `task_ids`, or all outstanding tasks with
`all: true`.

Pass `todo_id` when starting a task to record its link to a todo. Collecting
the result gives the parent context for updating that todo. An optional
`output_schema` in the subagent definition validates the completed JSON
result. If validation fails, the runner gives the child one corrective turn.

The runner emits progress and completion events. Attached clients can
read a child's recorded transcript. Supported clients can also send guidance
to a running child and confirm delivery after reconnecting.

## Runtime limits

The `[orchestration]` section uses the following defaults.

| Setting | Default | Effect |
|---------|---------|--------|
| `max_concurrent_subagents` | 4 | Active runs per session. Use 0 to disable this cap. |
| `max_subagent_depth` | 1 | Maximum nesting. Deeper nesting also requires `can_spawn: true` on the child definition. |
| `default_max_duration_secs` | 600 | Wall-clock limit unless the definition sets `max_duration_secs`. |
| `leak_reminder` | `true` | Remind the model about uncollected tasks at turn end. |
| `leak_reminder_max` | 2 | Maximum reminder loop-backs per user turn. |

## Persistence

`mew-agent/src/subagent_registry.rs` persists outstanding task metadata
in `<session>/subagent_tasks.json`. The child transcript is stored with the
session. After a restart, the registry can surface interrupted work and
recover recorded results. Running execution state lives in memory.

See [Architecture](/docs/development/dev-architecture/) for the daemon
session model and [Daemon Protocol](/docs/development/dev-protocol/) for
frontend events.
