---
title: Subagents
description: Delegate bounded tasks to child agents with isolated context.
---

Subagents are child agents spawned by the main agent for focused work. Each
subagent runs with its own conversation context, tool list, and cancellation
token, then returns its final result to the parent agent as a tool output.

Use a subagent when a subtask would fill the main conversation with search
output, logs, or intermediate file reads.

## Built-in subagents

Mew includes three built-in subagents:

| Name | Purpose |
|------|---------|
| `researcher` | Investigate the codebase with `read`, `glob`, and `grep` |
| `plan-reviewer` | Review an implementation plan before execution |
| `coder` | Execute implementation edits and checks |

User-defined subagents override built-ins with the same name. The parent agent
picks a subagent based on the task and each subagent's description.

## How subagents work

1. The parent agent calls `subagent_start` with a subagent name and prompt.
2. `SubagentRunner` spawns a child `Agent` with a fresh session, the subagent's
   system prompt, and its tool list.
3. The child runs until it produces a final result, hits a limit, or is
   cancelled.
4. Mew returns the result to the parent agent as a tool result.

By default, `subagent_start` blocks until the child finishes. Passing
`async: true` starts the subagent in the background and returns a task ID
immediately; the parent collects its output later with `subagent_wait`. Up to
four subagents can run concurrently per session by default (configured via
`[orchestration] max_concurrent_subagents`).

### What the parent sees

When a subagent stops, the parent receives a `SubagentResult` with one of three
outcomes:

| Outcome | Description |
|---------|-------------|
| `Complete` | Final answer text, turn count, and flags indicating whether a turn or time cap triggered |
| `Cancelled` | Stopped before completion |
| `Error` | Stopped due to a provider or tool error |

When a child hits its turn or time cap, Mew flags the result as possibly
incomplete so the parent can verify before relying on it.

### Communication and approvals

During a run, progress reports stream to attached clients for UI status display
without entering the parent agent's model context. Tool and workspace permission
requests from child agents keep their reply channels so approvals in the UI
unblock the waiting child. Questions from `ask_user_question` also forward to
attached clients.

## Defining custom subagents

Create `<name>.md` in `.mew/agents/`:

```markdown
---
name: my-reviewer
description: Reviews code changes for bugs and style issues.
tools:
  - read
  - grep
  - glob
max_turns: 50
---

You are a code reviewer. Focus on:
- Logic errors and edge cases
- Security issues
- Style consistency

Reference exact file paths and line numbers.
```

### Frontmatter fields

Frontmatter is optional; without it, Mew uses the filename stem as `name` and
an empty description. When frontmatter is present, `name` must be 1 to 64
characters matching `^[a-z0-9]+(-[a-z0-9]+)*$` and must match the `.md`
filename stem (otherwise the loader skips the file).

| Field | Required | Description |
|-------|----------|-------------|
| `name` | When frontmatter is present | Subagent identifier matching the filename stem |
| `description` | No | Tells the parent agent when to delegate to this subagent |
| `tools` | No | Tool allowlist. Omitting `tools` inherits parent tools except `subagent_start` and `subagent_wait` (unless `can_spawn: true` within depth limits) |
| `model` | No | Pin `provider/model` or a tier (`nano`, `micro`, `deci`) from a [router provider](/docs/using-mew/providers/#router-provider) |
| `max_turns` | No | Maximum turns before stopping (default: `500`) |
| `max_duration_secs` | No | Maximum runtime in seconds (default: `600`) |
| `template` | No | Render the body with minijinja when `true` |
| `can_spawn` | No | Allow nested subagent spawning within depth limits (default: `false`) |
| `output_schema` | No | JSON Schema (YAML map or relative `@file.json`) enforced on the final answer |

With `output_schema`, the child should emit plain JSON through `exit_tool`. On
schema validation failure, the runner grants one corrective turn; if that retry
still fails, it returns the output with a `schema_invalid` warning.

### System prompt composition

Every subagent receives the compiled subagent prompt
(`mew://system_prompts/subagent`), which pulls in the base prompt and adds the
subagent instructions for `exit_tool`, progress updates, narration, and context
limits. Mew appends the subagent's own markdown body after that shared prompt and
renders the body through minijinja when `template: true`. Keep custom subagent
bodies focused on role-specific instructions.

### Templated subagents

When `template: true` is set, minijinja renders the subagent body with
`subagent_name`, `model_id`, `provider_id`, `session_id`, `cwd`, `current_date`,
and `tools`. See [Personas](/docs/using-mew/personas/#templates) for the full
template variable and function reference.

```markdown
---
name: context-aware-coder
description: Writes code with awareness of available tools.
template: true
---

You are a coder. Available tools: {{ tools | join(", ") }}.
{% if has_tool("bash") %}You can run commands.{% endif %}
```

### Discovery paths

Mew scans project directories (cwd up to Git root), extension `[provides]`
subagent directories (in `mew run`), and global directories. Earlier matches win:

**Project paths** (cwd up to Git root):

1. `.mew/agents/<name>.md`
2. `.opencode/agents/<name>.md`
3. `.claude/agents/<name>.md`
4. `.agents/<name>.md`

**Extension paths:**

5. Extension `[provides]` agent directories (in `mew run`)

**Global paths:**

6. `~/.config/mew/agents/<name>.md`
7. `~/.config/opencode/agents/<name>.md`
8. `~/.claude/agents/<name>.md`
9. `~/.agents/agents/<name>.md`

## Limits and depth

- **Turn cap** (`max_turns`, default `500`): stops the child after that many
  turns and marks the result as possibly incomplete.
- **Time cap** (`max_duration_secs`, default `600`): spans model requests,
  tools, and schema correction; on expiry, the runner cancels the child and
  gives it a short grace period to emit partial results. Override the default
  globally via `[orchestration] default_max_duration_secs` (per-subagent
  `max_duration_secs` takes precedence).
- **Depth limit**: nested spawning requires `can_spawn: true` and
  `subagent_start` in `tools`, capped by `[orchestration] max_subagent_depth`
  (default `1`, where top-level sessions are depth `0`).

## Display and sidebar inspection

Each running subagent gets a deterministic display name (such as `Curie`,
`Turing`, or `Lovelace`) derived from its session ID and appears in the TUI
sidebar with a status icon and elapsed time:

```text
Subagents (2)
▸ Curie (researcher)  3s
    scanning the repo
▸ Turing (coder)     12s
    writing tests
```

Status icons indicate `▸` running, `✓` done, `✗` failed, and `⊘` cancelled.
Completed and cancelled entries leave the sidebar right away; recent failures
keep their error message for `tui.sidebar_finished_ttl_secs`.

Click a subagent row or run `/agents` to inspect the subagent's assignment,
messages, reasoning, tool inputs, results, and errors. Open viewers refresh as
turns save (`r` refreshes manually, arrows and page keys scroll, and `Esc`
closes). Daemon-backed TUI clients do not support cancelling subagents with `x`.

## Guide a running subagent

In the shared React Web UI and Electron shell, open the **Agents** pane in a
parent conversation and select a subagent to read its transcript. While the
subagent is running, type a message and click **Send guidance**. `Guidance
saved.` confirms the daemon queued the message for the subagent's next model
request; use refresh to load new turns.

If the connection drops before delivery confirmation, the client keeps your
draft and checks its receipt after reconnect, reusing the original request ID so
the subagent receives the message at most once. To assign more work after a
subagent finishes, ask the parent agent to spawn a new subagent.

## Subagent sessions

Mew writes each subagent's transcript under its parent session:

```text
sessions/<parent-id>/subagents/<child-id>/session.jsonl
```

The parent's `meta.json` lists child session IDs. Child transcripts can be
viewed after completion, though child workers cannot be resumed individually.
Resuming a parent session marks any previously active subagents as orphaned and
recovers their final results when available. See
[Sessions](/docs/getting-started/sessions/) for session storage layout.
