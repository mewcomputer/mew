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
| `researcher` | Investigate the codebase with `read`, `glob`, and `grep` |
| `plan-reviewer` | Review an implementation plan before handoff |
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
the task ID to collect the result later. This is useful for running multiple subagents in parallel. A session can
run up to four subagents at once by default. Configure the cap with
`[orchestration] max_concurrent_subagents`.

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

Progress reports reach attached clients during the run. Child tool and workspace
permission requests retain their reply channels, so approving or denying a
request reaches the waiting child.

### Parent and child communication

The parent supplies the initial assignment. The child has its own context and
returns its final result directly for a blocking start, or when the parent
collects an asynchronous task with `subagent_wait`. Progress reports are UI
status updates; they are not new messages in the parent's model context.

The parent agent can cancel a child. The user can send follow-up guidance
to a running child from Electron. If a child uses `ask_user_question`, its
question is forwarded to the attached client for a reply.

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
| `model` | no | Pin a `provider/model` pair, or use `nano`, `micro`, or `deci` from a configured [task router](/docs/using-mew/providers/#router-provider) |
| `max_turns` | no | Maximum turns before stopping (default: 500) |
| `max_duration_secs` | no | Time limit in seconds (default: 600) |
| `template` | no | When `true`, render the body through minijinja before using it as the system prompt |
| `can_spawn` | no | Permit nested subagents within the configured depth cap. Defaults to `false`. |
| `output_schema` | no | Validate the final answer against a JSON Schema. Use a YAML map or an `@file.json` path relative to the subagent definition. |

With an output schema, ask the child to return plain JSON with `exit_tool`.
The runner gives the child one chance to correct an invalid answer. If
the retry still fails, it returns the answer with a `schema_invalid` warning.

### System prompt composition

Every subagent's system prompt is the shared subagent base followed by the
subagent's body. The base (built-in resource `mew://system_prompts/subagent`)
transcludes the base system prompt and adds the subagent contract: the
`exit_tool` result channel, reasoning narration, progress updates, and the
context budget. It always renders, so every subagent inherits the contract
regardless of its `template` setting. The body is appended after it and is
rendered only when `template: true`; bodies should hold role-specific
instructions only.

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

After project paths, the loader checks `~/.config/mew/agents/`,
`~/.config/opencode/agents/`, `~/.claude/agents/`, and `~/.agents/agents/`
in that order.

## Limits

Subagent runs are bounded by two caps:

- **Turn cap** (`max_turns`, default 500): the subagent stops after this
  many turns, even if it hasn't produced a final answer. The result is
  marked as possibly incomplete.
- **Time cap** (`max_duration_secs`, default 600 seconds / 10 minutes):
  the deadline covers model requests, tools, and schema correction. At the
  deadline the runner cancels the subagent and allows a brief interval to save
  partial output.

Both can be overridden per subagent in the frontmatter. The default time cap can
also be changed with `[orchestration] default_max_duration_secs`; an explicit
per-subagent limit takes precedence.

## Depth limiting

A subagent can spawn children when its definition sets `can_spawn: true`
and its tool allowlist permits `subagent_start`. The session-wide limit
is `[orchestration] max_subagent_depth` (default 1). Top-level sessions are
at depth 0 and their direct children are at depth 1.

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

The display name, subagent type, and elapsed time are visible. The last
progress message appears below the name. Completed and cancelled activity leaves
the sidebar; recent failures retain their reason for the configured lifetime.

Click a subagent or use `/agents` to open its details. The borderless viewer shows
the recorded assignment, child messages, available reasoning, tool arguments,
results and errors. It refreshes saved turns while open; an unfinished turn may
not yet be recorded. Press `r` to refresh manually, use arrow/page keys or the
wheel to scroll, and Escape to close. An open viewer retains the final outcome.
Trace replies cannot switch the parent conversation or replace its transcript.

## Guide a running subagent in Electron

In the parent conversation, open the **Agents** pane and select a subagent to
read its transcript. While the subagent is running, enter a message and choose
**Send guidance**. **Guidance saved** means the message was saved for the
subagent's next model request. Use the refresh button to load its latest saved
messages.

If the connection drops before delivery is confirmed, Electron keeps your draft
and checks for a receipt when it reconnects. Retrying the same text uses the
original request ID so the subagent receives it at most once.

You can read a finished subagent's transcript while it remains listed.
To give it more work, ask the parent to start a new subagent.

## Subagent sessions

Each subagent run gets its own session file, nested under the parent:

```
sessions/<parent-id>/subagents/<child-id>/session.jsonl
```

The parent session's `meta.json` records child session IDs. Recorded transcripts
are readable, but child workers cannot currently be resumed. After a parent
resume, outstanding tasks are reported as orphaned with a best-effort recovery
of their final result. Older transcripts may lack completed tool outputs that
were not recorded at the time; those outputs cannot be reconstructed. See
[Sessions](/docs/getting-started/sessions/) for the session storage format.
