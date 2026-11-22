---
title: Tips & Tricks
description: Power-user shortcuts and workflows for Mew's terminal client.
---

These shortcuts and workflows apply to the terminal client.

## Input editing

- **Multi-line input**: press `Shift+Enter`, `Alt+Enter`, or `Ctrl+J` to insert
  a newline without sending (for code snippets or structured prompts).
- **Undo and redo**: press `Ctrl+Z` or `Ctrl+_` to undo, and `Ctrl+Shift+Z` to
  redo. Rapid typing groups into one undo step, separated by cursor movement or
  kill/yank commands.
- **History recall**: press `Ctrl+P` / `Ctrl+N` to browse previous and next
  prompts, restoring your unsent draft when returning past the newest entry.
- **History search**: press `Ctrl+R` to search backward through input history
  and `Ctrl+S` to search forward. Type to filter and press `Enter` to insert.
- **Word navigation**: jump word boundaries with `Alt+B` / `Alt+F` or
  `Alt+Left` / `Alt+Right`. Kill words backward or forward with `Alt+Backspace`
  / `Alt+D`, or kill whitespace-delimited tokens (such as file paths) with
  `Ctrl+W`.
- **Line editing**: jump to start or end with `Ctrl+A` / `Ctrl+E`, kill to line
  end with `Ctrl+K`, kill to line start with `Ctrl+U` or `Cmd+Backspace`, and
  transpose characters with `Ctrl+T`.
- **Kill ring**: paste the latest kill with `Ctrl+Y`, then press `Alt+Y` to cycle
  older kills. All text fields share the kill ring while keeping separate undo
  stacks.
- **Image paste**: paste an image from the system clipboard with `Ctrl+V`,
  `Ctrl+Shift+V`, or `Cmd+V`.

## Chat interaction

- **Copy text**: click and drag to select text (toggle mouse capture with
  `/mouse`), which copies automatically on mouse release, or press
  `Ctrl+Shift+C`. Copied text strips terminal padding and line wraps while
  keeping paragraph breaks and indentation.
- **Scroll**: use the mouse wheel, `PageUp` / `PageDown` (10 lines), `Up` /
  `Down` with an empty composer (1 line), scrollbar clicks, `Ctrl+Home` /
  `Ctrl+End`, or `Ctrl+L` to jump to the bottom and re-enable auto-scroll.
- **Cancel streaming**: press `Esc` twice. The first `Esc` displays `"esc again
  to stop agent"` in the status bar; the second cancels immediately.
- **Steer or replace with queued messages**: press `Ctrl+Up` to inject the
  oldest queued message as guidance into the running turn, or press `Up` twice
  with an empty composer while streaming to cancel the turn and send the oldest
  queued message immediately.
- **Expand blocks**: press `Ctrl+Alt+T` for reasoning blocks, `Ctrl+O` for bash
  output, Python cells, and nested tool details, and `Ctrl+G` for tool-call
  batches and compaction blocks.
- **Help overlay**: press `?` with an empty composer to toggle the shortcuts
  sheet.

## Session management

- **Open views**: press `Ctrl+Alt+O` to list conversations open in this
  terminal window, or `Tab` from that picker for global session history. Switch
  between open views with `Ctrl+Alt+Left` / `Ctrl+Alt+Right`, and start a new
  conversation in the current project with `Ctrl+Alt+Enter`.
- **Close views**: press `Ctrl+Alt+W` to close the current view or the selected
  row in a conversation picker. Drafts and queued follow-ups stay alive for the
  TUI process while daemon tasks keep running; closing the last view exits.
- **Notifications**: the attention bell (`Ctrl+Alt+N`) tracks conversations open
  in this window that have pending questions, approvals, failures, or unread
  results. Completed results stay unread until viewed in a focused terminal
  (or interacted with when focus reporting is unavailable).
- **History and context**: run `/sessions` or `/resume <id>` to switch
  conversations (see [Sessions](/docs/getting-started/sessions/)). Run `/clear`
  to reset model context while keeping `"Allow session"` tool approvals, or
  `/compact` to compact history immediately.

## Models and personas

- **Quick switch**: run `/model deepseek-v4-flash` to switch directly or `/model`
  to open the picker. Press `Right` in the model picker (with an empty filter)
  to jump straight to that model's thinking variants.
- **Thinking variants**: run `/thinking high` or `/thinking off`, or select
  **Thinking Variant** from `Ctrl+Space` and cycle with `Ctrl+Alt+P`.
- **Cycle personas**: press `Shift+Tab` (forward) or `Ctrl+Shift+Tab`
  (backward) with an empty composer.
- **Cost tracking**: run `/cost` for session token counts and estimated cost.

## `@`-mentions

Type `@` in the composer to open a unified picker listing files, models,
skills, and subagents; keep typing to filter. Selecting a file inserts an
`@path` reference whose contents Mew attaches when sending the prompt. Selecting
a model, skill, or subagent inserts `@model:<id>`, `@skill:<name>`, or
`@subagent:<name>`.

## Terminal title and logs

Mew updates the terminal title with session status and background counts, such as
`mew · running · Native browser lifecycle · ~/code/mew`. Daemon and TUI logs
redirect to `$TMPDIR/mew-<pid>.log` (from the system temp directory) so they
stay out of the terminal UI.

## Workflows

- **Scope prompts explicitly**: name target files or directories and specify
  boundaries (for example, `"fix the pagination off-by-one in src/api/list.rs;
  don't change the database schema"`).
- **Investigate before editing**: ask for a read-only explanation first when
  working in unfamiliar code, or delegate multi-file scans to `researcher` (see
  [Subagents](/docs/using-mew/subagents/)).
- **Plan then build**: switch to `/persona planner` to draft a plan under
  `plans/`, review it, then switch to `/persona builder` to implement it (see
  [Personas](/docs/using-mew/personas/)).
- **Step permission modes safely**: inspect early edits in `standard` mode,
  switch to `/permissions permissive` or `/permissions auto` for repetitive
  safe batches, and return to `/permissions standard` afterward (see
  [Permissions](/docs/using-mew/permissions/)).
