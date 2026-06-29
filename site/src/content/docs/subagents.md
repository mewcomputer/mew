---
title: Subagents
description: Delegate tasks to child agents that run in the background.
---

Subagents are child agents spawned by the main agent to work on bounded
tasks in the background. They have their own conversation context, tool
access, and cancellation token.

## Built-in subagents

mew ships with three built-in subagent definitions:

| Name | Purpose |
|------|---------|
| `researcher` | Investigate research questions against the local codebase or internet |
| `reviewer` | Review plans and code for issues |
| `coder` | Execute implementation tasks |

User-defined subagents override built-ins by name.

## Defining custom subagents

Create a `SUBAGENT.md` file in `.mew/subagents/<name>/`:

```markdown
---
name: my-reviewer
description: Reviews code changes for bugs and style issues.
tools:
  - read
  - grep
  - glob
---

You are a code reviewer. Focus on:
- Logic errors and edge cases
- Security issues
- Style consistency

Be direct and specific. Reference file paths and line numbers.
```

Discovery paths (walked cwd to git root):

1. `.mew/subagents/<name>/SUBAGENT.md`
2. `.opencode/subagents/<name>/SUBAGENT.md`
3. `.claude/subagents/<name>/SUBAGENT.md`
4. `.agents/subagents/<name>/SUBAGENT.md`

## How subagents work

1. The agent calls the `start_subagent` tool with a name and prompt.
2. `SubagentRunner` spawns a child `Agent` with a fresh conversation,
   the subagent's system prompt, and the subagent's tool allowlist.
3. The child agent runs to completion (or cancellation).
4. Progress updates flow up via `AgentEvent::SubagentStatus`.
5. The result is returned to the parent agent as a tool result.

## Display names

Each running subagent gets a human-friendly display name (e.g. "Curie",
"Rosalind", "Turing") shown in the sidebar. Names are picked from a pool
of 25 entries via `pick_display_name(seed)`, which hashes the subagent's
fresh `SessionId` with splitmix64. Deterministic per session, no `rand`
dependency.

## Depth limiting

Subagent nesting is capped at `max_subagent_depth` (default 3). Top-level
sessions are depth 0, their direct subagents are depth 1, and so on. A
subagent at the depth cap cannot spawn further subagents.

## Cancellation

Press `x` (when the input is empty) to cancel the most recently started
subagent. The sidebar shows running subagents with their elapsed time and
last progress message (`↳ message`).

## Sidebar display

The sidebar shows:

```
▸ Curie (researcher)  3s ↳ scanning the repo
▸ Turing (coder)     12s ↳ writing tests
```

The display name, subagent type, elapsed time, and last progress message
are all visible. Completed subagents show a status dot (green = completed,
red = failed, yellow = cancelled).
