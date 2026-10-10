---
title: TUI task flow assessment
description: October 2026 UX findings and implementation priorities for the TUI and its native companion.
---

The TUI should keep its personality and keyboard efficiency while sharing task
semantics with GPUI. The largest remaining problems concern ownership and live
observation of a conversation. Visual repairs alone cannot address them.

## Scope and evidence

Two independent assessments inspected task flow and rendering. Headless captures
covered 40, 80, 120, and 160 columns. Behavior tests exercised real App state,
crossterm events, and ratatui buffers, including 24-column and smaller edge cases.
This was source and deterministic rendering QA; live cross-client interaction
still needs validation. The HTML/JSX design detector does not apply to ratatui.

The interface has a distinct identity: the cat, companion, theme accents, and
terminal conventions are worth retaining. There is no generic card-grid or
gradient-heavy composition to replace. Progressive tool/reasoning disclosure,
complete resumed history, and waiting for a full turn before releasing queued
work are useful foundations.

## Implemented on October 10

- Raised, opaque modal and autocomplete surfaces use `panel.overlay`, bound to
  the theme's `popover`. Default dark and light popovers differ from the canvas.
- The full frame has a painted canvas. Tool half-block edges inherit that canvas;
  question slots and toast padding are opaque.
- Queued previews and history/paste prompts have reserved rows. Long drafts
  follow the caret through navigation, history recall, clicking, and resizing.
  Rendering and cursor mapping share borrowed grapheme-aware rows. An oversized
  glyph in a one-cell text column has a visible replacement without changing
  the submitted input.
- Help scrolls with keys and the wheel, clamps after resize, and keeps a close
  hint visible. Pickers have contextual titles, navigation hints, and explicit
  empty results that retain the query.
- Centered pickers use one row per choice, a full selected-row highlight, and a
  separate description viewport with page-key navigation. Search follows the
  Unicode caret; selection and search edits reset the details viewport.
- Permission, persona and goal dialogs size to their content, scroll long or
  multiline bodies, and pin actions and close hints. The goal editor owns its
  paste and undo history without modifying the conversation draft.
- Inline autocomplete measures the clipped landing viewport, keeps the choice
  visible through scrolling and resizing, and retains completion/close hints.
  Long slash/reference lists grow to three fifths of the terminal, bounded by
  composer space; short searches shrink to their matches. Centered pickers use
  the available popup height rather than an eight-choice cap. Overflow has a
  proportional scrollbar. Wheeling over slash choices moves their selection;
  wheeling over chat retains chat scrolling. Landing choices cover the hero
  while preserving the composer's position.
- The sidebar appears at 112 columns. It leads with current work or the waiting
  decision, shows remaining/blocked tasks, and scrolls independently. Successful
  and cancelled activity leaves immediately; recent failures retain their
  reason for `tui.sidebar_finished_ttl_secs`. Todo and goal records are preserved.
  Restarting a job or subagent clears stale timing and progress.
- Sidebar task, subagent and job rows open their corresponding details. `/todo`,
  `/agents` and `/jobs` also work below the sidebar width. Finished work leaves
  the sidebar while an already-open detail view retains its final status.
  Activity views retain their session owner and explain when data is unavailable.
  Their raised surfaces have no extra outline. Subagent views read recorded child
  messages and tool arguments/results/errors through a correlated daemon request,
  refresh while open, coalesce repeated refreshes and retain scroll position.
  The initial assignment is visible; an unfinished turn may not yet be saved.
- The todo browser includes completed records and supports description/status
  editing. Saves wait for a correlated daemon acknowledgment; conflicts,
  dependency constraints and persistence failures keep the draft for recovery.
  Permission/question interruptions preserve an unsaved editor. Idle updates and
  attachment snapshots refresh canonical todos without replacing that draft.
  A confirmed session switch clears the old sidebar's activity and todo records.
- Paste inserts in one undo step, normalizes line endings, and targets the
  visible editor. Large-paste confirmation counts characters and preserves slash
  command mode. Short landing screens prioritize the composer and its prompt.
  Rendering and mouse input share composer geometry.
- Question inputs follow the Unicode caret and retain exact multiline answers.
  Long prompts/choices leave room for the answer and cancel hint; review pins
  submit/cancel beneath clipped previews. Read-only popovers hide the chat caret.
  The allow-tool panel uses its raised color without an extra outline.

## Remaining priorities

| Priority | Problem and consequence | Next implementation |
|---|---|---|
| P1 | The global follow-up queue can outlive a session switch. Closing the old prompt stream produces an unscoped completion, which can release task A's queued instruction through task B. | Conversation-owned drafts and queues; session-scoped events; serialized attachment. Test A to B switching with a real daemon. |
| P1 | Ordinary streaming events have no persistent recipient unless the TUI initiated a prompt. Attaching to work started in GPUI loses live progress. Idle disconnects also lack visible recovery. | Persistent task observation, explicit connection lifecycle, and submission acceptance that preserves rejected drafts. |
| P1 | The TUI ignores confirmed session cwd and still uses its launch directory for mentions, indexing, and footer context. | Project context derived from the attached session, with index invalidation on changes. |
| P1 | Attention lacks a reliable destination. Session lists do not seed attention state; the flat picker omits required-action counts and useful conversation summaries. | Project-grouped task navigation with running/attention filters, archived recovery, and retained outcomes. |
| P2 | Changes are a noninteractive path list; session-diff replies have no TUI consumer. Some advertised commands are unsupported in the daemon path. | Direct review entry point using session/request correlation; implement commands against authoritative state or explain their availability. |

Tool-output truncation still needs a clear way to inspect the omitted content.
Subagent detail views include recorded child transcripts. Background job detail
views still need transport support for full job output. Child `ask_user` questions
remain unforwarded, and parent/child follow-up messaging is not implemented.
These follow the ownership and connection repairs. The sidebar reflects events
received by the current adapter; passive progress from another client remains
limited by its transport. Changes remains the cumulative session delta summary.

## Architecture choice

The recommended direction is to reuse `mew-client-core` and `mew-client-local`
for canonical task state and persistent transport, retaining the TUI editor and
renderer. That choice is awaiting user input; it has not been implemented.

The core already carries session-scoped events, confirmed cwd, transcript,
attention, projects, and required actions. It needs job updates and retry details
before migration. Native attachment coordination and drafts currently belong to
the native shell, so adopting the core alone will not fix ownership.

Migrate in tested steps: task-owned drafts and attachment, missing shared-core
events, a persistent TUI driver and submission acceptance, then navigation and
review projections. Replace the old adapter rather than maintaining a second
socket or parallel canonical transcript. Daemon-connected capture must migrate
with the interactive TUI.

## Design assessment

These provisional scores describe the remaining source-level risks after the
visual/input and activity slices. They are judgment, not measured usability results.

| Heuristic | Score / 4 | Evidence |
|---|---:|---|
| Visibility of status | 2 | Passive progress and idle connection failures remain incomplete. |
| Domain language | 3 | Familiar terminal commands; task/session terminology still varies. |
| User control | 2 | Efficient editing, but switching lacks owned drafts and reliable recovery. |
| Consistency | 2 | TUI and GPUI have different task observation paths. |
| Error prevention | 1 | A queued instruction can cross task ownership. |
| Recognition | 2 | Picker labels improved; finding waiting work still requires recall. |
| Efficiency | 3 | Keyboard editing and progressive disclosure work well. |
| Minimalism | 3 | Strong identity; command ordering still needs work. |
| Error recovery | 1 | Idle disconnects and rejected submission recovery remain weak. |
| Help | 3 | All sections are reachable; advertised unsupported commands still confuse. |
| Total | 22 / 40 | Task reliability limits the experience. |

Cognitive load is moderate, with three observed risks: remembering task state
across screens, an ungrouped conversation list, and equally weighted command
choices. A multitasking developer can miss waiting work or misroute a follow-up.
A terminal power user sharing a task with GPUI can miss progress and reference
the wrong project. Completion should leave a persistent outcome and a clear
review/follow-up action; a transient toast is insufficient.

## Screenshot gallery

The local gallery is `notes/capture/tui-20261010/index.html`. It contains 16
actual-renderer captures at 128 columns by 40 rows, rasterized to 1280 by 800
pixels with approximately 16 px text. It covers the sidebar, subagent/job
details, todo browsing/editing/errors, long paste, question input/review and
existing popovers. Narrow-terminal behavior remains covered by buffer tests and
separate captures. The capture directory is ignored by Git.
