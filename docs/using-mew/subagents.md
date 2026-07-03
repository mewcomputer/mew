---
title: Subagents
description: Delegate tasks to child agents that run in the background.
---

Subagents are child agents spawned by the main agent to work on bounded
tasks. Each subagent has its own conversation context, tool access, and
cancellation token. When the subagent finishes, its result returns to the
parent agent as a tool output.

Use a subagent when a side task would flood your main conversation with
search results, logs, or file contents you won't reference again. The
subagent does that work in its own context and returns only the summary.

## Built-in subagents

mew ships with three built-in subagent definitions:

| Name | Purpose |
|------|---------|
| `researcher` | Investigate research questions against the local codebase or internet |
| `reviewer` | Review plans and code for issues |
| `coder` | Execute implementation tasks |

User-defined subagents override built-ins by name. The agent decides which
subagent to use based on the task and the subagent's description.

## How subagents work

1. The agent calls `subagent_start` with a name and prompt.
2. `SubagentRunner` spawns a child `Agent` with a fresh conversation,
   the subagent's system prompt, and the subagent's tool allowlist.
3. The child agent runs to completion (or cancellation).
4. The result returns to the parent agent as a tool result.

By default, `subagent_start` blocks until the subagent finishes and
returns the result directly. Pass `async: true` to start the subagent in
the background and get a task ID immediately. Use `subagent_wait` with
the task ID to collect the result later. This is useful for running
multiple subagents in parallel before combining their results.

### What the parent sees

When a subagent finishes, the parent receives a `SubagentResult`:

| Outcome | Description |
|---------|-------------|
| `Complete` | The subagent produced a final answer. Includes the text, turns used, and flags for turn/time limits hit. |
| `Cancelled` | The subagent was cancelled before completion. |
| `Error` | The subagent failed with an error from the provider or tool layer. |

If the subagent hit its turn or time limit, the result is marked as
possibly incomplete. The parent agent should treat it with appropriate
caution.

Progress updates flow up during the run via `AgentEvent::SubagentStatus`,
so you can see what the subagent is doing in real time.

## Defining custom subagents

Create a `.md` file in `.mew/agents/`:

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

Be direct and specific. Reference file paths and line numbers.
```

### Frontmatter fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Subagent identifier |
| `description` | yes | When the agent should delegate to this subagent |
| `tools` | no | Tool allowlist. Inherits all tools if omitted |
| `model` | no | Pin a `provider/model` pair, or use tier keywords (`nano`, `micro`, `deci`) when the active provider is a router |
| `max_turns` | no | Maximum turns before stopping (default: 500) |
| `max_duration_secs` | no | Wall-clock cap in seconds (default: 300) |
| `template` | no | When `true`, render the body through minijinja before using it as the system prompt |

### Templated subagents

When `template: true` is set, the subagent body is rendered through
minijinja before being used as the system prompt. The context includes
`subagent_name`, `model_id`, `provider_id`, `session_id`, `cwd`,
`current_date`, and `tools`. See [Personas](/docs/using-mew/personas/#templates)
for the full variable reference.

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

Discovery paths (walked cwd to git root, earlier wins):

1. `.mew/agents/*.md`
2. `.opencode/agents/*.md`
3. `.claude/agents/*.md`
4. `.agents/*.md`

## Limits

Subagent runs are bounded by two caps:

- **Turn cap** (`max_turns`, default 500): the subagent stops after this
  many turns, even if it hasn't produced a final answer. The result is
  marked as possibly incomplete.
- **Time cap** (`max_duration_secs`, default 300 seconds / 5 minutes):
  the subagent is cancelled if it exceeds this wall-clock duration.

Both can be overridden per subagent in the frontmatter.

## Depth limiting

Subagent nesting is capped at `max_subagent_depth` (default 3). Top-level
sessions are depth 0, their direct subagents are depth 1, and so on. A
subagent at the depth cap cannot spawn further subagents.

## Cancellation

Press `x` (when the input is empty) to cancel the most recently started
subagent. The sidebar shows running subagents with their elapsed time and
last progress message.

## Display names

Each running subagent gets a human-friendly display name (e.g. "Curie",
"Turing", "Lovelace") shown in the sidebar. Names are picked from a pool
of 25 entries via a splitmix64 hash of the subagent's session ID.
Deterministic per session, no `rand` dependency.

## Sidebar display

The sidebar shows running subagents with their status:

```
Curie (researcher)  3s
  scanning the repo
Turing (coder)     12s
  writing tests
```

The display name, subagent type, and elapsed time are always visible. The
last progress message appears on a sub-line with a indent. Completed
subagents show a status dot:

- Green: completed successfully
- Red: failed
- Yellow: cancelled

## Subagent sessions

Each subagent run gets its own session file, nested under the parent:

```
sessions/<parent-id>/subagents/<child-id>/session.jsonl
```

This means subagent transcripts are persisted and can be resumed. The
parent session's `meta.json` records child session IDs. See
[Sessions](/docs/getting-started/sessions/) for the session storage format.

## Subagents vs personas vs skills

Subagents, personas, and skills all customize behavior and it is easy to
mix them up. A subagent is a separate child agent that returns a summary;
a persona changes how the main agent behaves for the whole session; a
skill is loaded on demand for one procedure. See
[Comparing Features](/docs/using-mew/comparisons/) for the full
breakdown.
