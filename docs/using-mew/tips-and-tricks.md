---
title: Tips & Tricks
description: Power-user features and workflows for mew.
---

## Input editing

- **Multi-line input**: press `Alt+Enter` to insert a newline without
  submitting. Useful for writing code snippets or structured prompts.
- **Undo/redo**: `Ctrl+Z` undoes the last edit. `Ctrl+Shift+Z` or `Ctrl+Y`
  redoes. Edits within a 500ms window coalesce into one undo entry.
- **History search**: `Ctrl+R` searches backward through input history.
  `Ctrl+S` searches forward. Type to filter, Enter to insert.
- **Word navigation**: `Alt+Left` / `Alt+Right` jump word boundaries.
  `Alt+Backspace` deletes a word backward.
- **Line editing**: `Ctrl+A` / `Ctrl+E` jump to start / end. `Ctrl+K`
  kills to end of line. `Ctrl+U` clears everything.

## Chat interaction

- **Copy text**: `Ctrl+Shift+C` copies selected text to the clipboard.
  Click and drag to select (when mouse capture is on, toggle with `/mouse`).
- **Scroll**: mouse wheel, `PageUp`/`PageDown` (10 lines), or click the
  scrollbar. `Ctrl+Home` / `Ctrl+End` jump to top / bottom. `Ctrl+L`
  re-attaches auto-scroll to the bottom.
- **Cancel streaming**: press `Esc` twice. The first `Esc` shows a hint in
  the status bar ("esc again to stop agent"). The second cancels immediately.
- **Toggle reasoning**: `Ctrl+T` expands/collapses reasoning/thinking blocks.
  `Ctrl+O` expands/collapses bash output.

## Session management

- **History**: `/sessions` lists previous sessions with timestamps.
  `/resume <id>` resumes one. See [Sessions](/docs/getting-started/sessions/).
- **Rewind**: `/rewind <n>` truncates to the first N messages. Use
  `/rewind` with no args to see a list with snippets.
- **Clear**: `/clear` wipes the conversation context (persists to disk).
  `/compact` forces context compaction on the next turn.

Clearing preserves permission grants. If you approved a tool for the
session, that approval survives `/clear`.

## Model switching

- **Quick switch**: `/model deepseek-v4-flash` switches without leaving
  the keyboard. `/model` alone opens the picker.
- **Thinking variants**: `/thinking high` sets the reasoning effort.
  `/thinking off` disables it. Or use `Ctrl+P` and select "Thinking
  Variant", then press `Ctrl+P` repeatedly to cycle through options.
- **Cost tracking**: `/cost` shows accumulated token counts and estimated
  cost for the session.

## @-mentions

- Type `@` in the input to open the reference picker. It lists files,
  models, skills, and subagents in one list; typing after the `@` narrows
  the same list. Selecting a file inserts an `@path` mention that gets
  resolved and added to the agent's context before the prompt is sent.
  Selecting a model, skill, or subagent inserts the matching
  `@model:`, `@skill:`, or `@subagent:` reference.

## Terminal title

When streaming, the terminal tab title shows `mew - thinking...`. When
idle, it shows `mew`. This helps you tell when a response is done from
another tab. Logs are redirected to `/tmp/mew-<pid>.log` so they don't
corrupt the TUI display.

## Prompting patterns

### Be specific about scope

The agent has filesystem access and tools. Tell it what to focus on:

```
Fix the off-by-one in the pagination logic in src/api/list.rs.
Don't touch the database layer.
```

### Ask for investigation first

For unfamiliar code, ask the agent to investigate before making changes:

```
Find where the session timeout is configured and explain how it works.
Don't change anything yet.
```

### Use subagents for heavy research

If a task involves scanning many files, delegate it so the results don't
fill your main context:

```
Use the researcher subagent to find all places we construct SQL queries
and report back which ones use parameterized queries.
```

See [Subagents](/docs/using-mew/subagents/) for the built-in options.

### Break work into phases

Use the planner/builder workflow for complex tasks:

1. `/persona planner` to investigate and write a plan
2. Review the plan in `PLAN.md`
3. `/persona builder` to execute it

See [Personas](/docs/using-mew/personas/) for details.

## Permission workflow

If you're doing repetitive work that needs many tool approvals:

1. Start in `standard` mode to review each tool call.
2. Once you trust the pattern, switch to `permissive` with
   `/permissions permissive` to auto-allow writes and edits.
3. For full automation, use `/permissions auto` to let the classifier
   decide each call.

Switch back to `standard` when you're done:

```
/permissions standard
```

See [Permissions](/docs/using-mew/permissions/) for the full mode reference.
