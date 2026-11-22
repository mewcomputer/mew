---
title: TUI UX and task flow
description: Design standards, surfaces, and session-scoped task flow in the terminal UI.
---

The terminal UI pairs ratatui conventions with session-scoped daemon state.
See [TUI Architecture](/docs/development/dev-tui/) for the runtime pipeline and
[Session handover](/docs/development/handover/) for multi-client attachment.

## Surface and input standards

- **Opaque surfaces and theme tokens**: modals, inline autocomplete menus, and
  picker overlays use `panel.overlay` (mapped to `popover` in
  `theme_manifest.json`). Tool half-block borders, question slots, and toast
  padding paint against explicit canvas colors.
- **Grapheme-aware editing**: the composer, picker filters, goal editor, todo
  editor, and question inputs share borrowed grapheme-aware row rendering and
  hit testing. Paste operations normalize line endings in one undo entry, and
  oversized glyphs render visible replacements without altering submitted text.
- **Bounded autocomplete and pickers**: inline slash and `@` menus expand up to
  three fifths of the viewport while preserving composer space. Centered
  pickers display one item per row with a separate scrollable details pane
  (`PageUp` / `PageDown`) and proportional scrollbars.
- **Content-sized dialogs**: permission prompts, persona confirmations, goal
  editors, and question inputs size to their content, scroll long bodies, and
  keep action and close hints pinned.

## Sidebar and detail viewers

- **112-column sidebar**: appears when width is at least 112 columns (`80 + 32`),
  leading with active work or pending decisions, followed by todos, subagents,
  background jobs, file changes, and environment sources. Completed or cancelled
  activity leaves immediately; recent failures remain for
  `tui.sidebar_finished_ttl_secs`.
- **Activity and review modals**: clicking sidebar rows or running `/todo`,
  `/agents`, `/jobs`, `/tools`, or `/review` opens dedicated detail views that
  retain session ownership, coalesce refreshes, and preserve scroll position.
  - `/todo`: inline editing and creation backed by correlated daemon updates.
  - `/agents`: reads recorded child transcripts and tool calls through
    `ReadSubagentHistory`.
  - `/jobs`: pages captured shell output through `ReadJobOutput`.
  - `/review`: loads workspace file changes via `ReadSessionDiff`.

## Session-scoped ownership

The TUI uses `TuiClient` (`crates/mew/src/runtime/client.rs`) on top of
`mew_client_core::ClientEngine` so conversation ownership survives switching
and reconnects:

- **Per-session drafts and queues**: `App.session_drafts` keeps separate
  composer input, cursor state, undo history, attachments, and queued follow-ups
  for each conversation (`crates/mew-tui/src/app/drafts.rs`).
- **Acknowledged submissions**: prompts send `SubmitPrompt` with a request ID
  and check `ReadPromptReceipt` on reconnect so uncertain submissions are never
  replayed automatically.
- **Forwarded child questions**: child `ask_user_question` prompts carry
  `AskUserRequest.subagent` through `TuiClient` so users can answer or cancel
  subagent questions directly.

## October 2026 baseline summary

An October 2026 UX assessment identified cross-session queue leakage, missing
persistent daemon observation, dropped passive progress, missing job/diff
viewers, and unforwarded child questions. Those gaps were closed by moving
the TUI onto `TuiClient` + `mew-client-core`, scoping drafts per session in
`drafts.rs`, and wiring `ReadSessionDiff`, `ReadJobOutput`, and child
`AskUserRequest` handling. Visual layouts remain covered by
`crates/mew-tui/tests/window_golden.rs` and `ux_capture` fixtures (see
[TUI capture methods](/docs/development/dev-tui-capture/)).
