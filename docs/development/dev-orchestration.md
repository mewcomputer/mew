---
title: Tasks and orchestration
description: How todos, goals, subagent runs, and orchestration limits work.
---

Mew can track session todos with dependencies, propose multi-turn goals, and
delegate subtasks to child subagents. See [Subagents](/docs/using-mew/subagents/)
for user configuration and examples.

## Todos and goals

`mew-agent::todos` creates, updates, completes, and deletes session todos via
the `todo_*` tools; users inspect and edit them with `/todo`. Todos persist in `<session>/todos.json` across
compaction and session resume. Dependencies gate `todo_complete` without
scheduling execution order.

Session goals use `propose_goal`, `complete_goal`, and `block_goal`. A proposed
goal blocks until the user accepts or rejects it (`GoalProposed` / `GoalResponse`).

## Run and collect subagents

`subagent_start` spawns a child agent in its own isolated session. Synchronous
runs return the final `exit_tool` payload (or final message) directly; passing
`async: true` returns a task ID so the parent can keep working and collect one
(`task_id`), several (`task_ids`), or all (`all: true`) tasks with
`subagent_wait`. Subagents report mid-run status through `progress_update`.

Pass `todo_id` to `subagent_start` to link a task to a todo. When a subagent
definition sets `output_schema`, the runner validates the final JSON output and
grants one corrective turn on failure.

Attached clients receive progress and completion events and can read recorded
child transcripts. Supported web clients can also queue guidance for a running
child (`GuideSubagent`) and verify delivery after reconnect
(`ReadSubagentMessageReceipt`).

## Runtime limits

`[orchestration]` defaults in `config.toml`:

| Setting | Default | Effect |
|---------|---------|--------|
| `max_concurrent_subagents` | `4` | Active subagents per session (`0` disables the cap) |
| `max_subagent_depth` | `1` | Maximum nesting depth (requires `can_spawn: true` on child definitions) |
| `default_max_duration_secs` | `600` | Wall-clock deadline when a definition omits `max_duration_secs` |
| `leak_reminder` | `true` | Remind the model about uncollected async tasks at turn end |
| `leak_reminder_max` | `2` | Maximum reminder continuations per user turn |

## Persistence

`crates/mew-agent/src/subagent_registry.rs` saves task metadata in
`<session>/subagent_tasks.json`, alongside child session transcripts under
`<session>/subagents/<child-id>/`. On daemon restart or session resume, the
registry marks interrupted runs as orphaned and recovers saved results where
available.

See [Architecture](/docs/development/dev-architecture/) for session ownership
and [Daemon Protocol](/docs/development/dev-protocol/) for wire events.
