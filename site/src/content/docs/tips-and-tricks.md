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
  `/resume <id>` resumes one.
- **Rewind**: `/rewind <n>` truncates to the first N messages. Use
  `/rewind` with no args to see a list with snippets.
- **Clear**: `/clear` wipes the conversation entirely (persists to disk).
  `/compact` forces context compaction on the next turn.

## Model switching

- **Quick switch**: `/model deepseek-v4-flash` switches without leaving
  the keyboard. `/model` alone opens the picker.
- **Thinking variants**: `/thinking high` sets the reasoning effort.
  `/thinking off` disables it. Or use `Ctrl+P` → "Thinking Variant" and
  press `Ctrl+P` repeatedly to cycle through options.
- **Cost tracking**: `/cost` shows accumulated token counts and estimated
  cost for the session.

## @-mentions

- Type `@` in the input to open the file picker. Selecting a file inserts
  an `@path` mention that gets resolved and added to the agent's context
  before the prompt is sent.
- `@` followed by a subagent name (shown with `[subagent]` tag) inserts
  a subagent reference instead of a file path.

## Terminal title

When streaming, the terminal tab title shows `mew — thinking…`. When idle,
it shows `mew`. This helps you tell when a response is done from another
tab. Logs are redirected to `/tmp/mew-<pid>.log` so they don't corrupt the
TUI display.
