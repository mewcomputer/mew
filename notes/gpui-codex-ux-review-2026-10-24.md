# GPUI UX comparison with Codex, 2026-10-24

This is a provisional comparison. Native computer use failed with an operating
system permission error, so neither installed app could be inspected live.
Mew findings below come from the current source and behavior-test inspection.
Codex observations come from the interactive Codex workspace example in the
[official desktop documentation](https://learn.chatgpt.com/docs/app), including
conversation-only, split, and artifact views. That example establishes a useful
reference, but cannot establish the behavior of the installed Codex build.

Two independent assessments examined mew's design and control/state inventory.
The user identified crowded layout and starting/switching projects or chats as
the primary concerns. This report prioritizes those concerns. Product code was
not changed and tests were not run.

## Main diagnosis

Mew exposes many supporting capabilities before making the current task and
working location sufficiently clear. Conversation, changes, terminal, open-chat
tabs, session settings, and connection state all claim space in the starting
composition. The effect is likely a crowded working environment with weak task
orientation. This remains a source-supported diagnosis pending native inspection.

The useful lesson from the Codex example is a hierarchy: a clear way to start,
recognizable project/chat navigation, a dominant conversation, and outputs that
can be opened alongside it or given the full content area. Mew already has the
underlying capabilities and careful state ownership needed to support that
hierarchy.

## Direct reference comparison

| Area | Observed Codex documentation example | Current mew source | Implication |
| --- | --- | --- | --- |
| Conversation space | Chat can occupy the content area; opening a document creates a split; full document view is also available. | Fresh saved-state fallback expands sidebar, Changes workbench, and terminal together. | Auxiliary surfaces claim space before they are useful. The example does not prove Codex's first-run defaults. |
| Starting work | New chat is near the top of the navigation, above project and recent-chat lists. | New conversation is at the sidebar bottom. The global action supplies no explicit working directory; a 22 px plus beside an existing workspace supplies its path. | The primary path hides the working-location decision. |
| Chat navigation | Projects and recent chats form distinct, labeled navigation groups. | Workspace-grouped tasks coexist with a global strip of title-only chat tabs. The working path appears as muted text below the composer. | Project context is stronger in the sidebar than in the selected tab, especially when names repeat or the sidebar closes. This is a usability risk, not an observed wrong-session submission. |
| Composer | Attachment, approval mode, model, and send appear together; no persona or Yield control is visible in the example. | Attachment, permissions, persona, model/effort, and send/stop share one unwrapped row; Yield appears below unless control was already yielded. | More session configuration competes with writing a message. |
| Empty send | The documentation example exposes Send as disabled with an empty composer. | The idle send control stays filled and clickable; empty submission returns silently. | Visual availability and actual behavior disagree. |
| Outputs | The example opens named artifacts through content tabs and switches between split and full views. | Browser, Changes, Local, and Activity share four auxiliary icon tabs inside the workbench. | Review whether panel navigation is understandable from the task, rather than copying Codex's tabs wholesale. |

## Priority findings

### 1. [P1] Give the conversation the default space

With no saved collapse state, sidebar, workbench, and terminal are expanded.
The initial window is 1240 by 760 pixels. The sidebar takes 264 pixels, the
workbench 360 pixels, and the terminal 190 pixels of height. Accounting for
explicit shell gaps and the divider leaves approximately 570 pixels of center
width before composer padding. This is arithmetic from source, not a measured
native frame. Default workbench content is Changes, even when no changes exist.

Start with navigation and conversation. Reveal terminal and review when the
user requests them or a relevant task opens them. Keep existing panes and their
retained state. Reduce separate enclosing borders where they do not communicate
a meaningful boundary. Verify the resulting composition before adjusting fonts
or colors.

Evidence: [layout defaults](../apps/mew-desktop/src/shell/types.rs#L723),
[dimensions](../apps/mew-desktop/src/shell/types.rs#L927),
[initial window](../apps/mew-desktop/src/shell/platform.rs#L239),
[shell composition](../apps/mew-desktop/src/shell/render.rs#L57), and
[default Changes view](../apps/mew-desktop/src/shell/lifecycle.rs#L313).
Suggested commands: `/distill`, `/layout`.

### 2. [P1] Make the working location part of starting a chat

The global New conversation action calls `begin_new_conversation(None, None)`.
The explicit workspace path is available through the small plus beside an
existing workspace. The current path below the composer is display text, not a
location chooser. Users must discover the less prominent path to control where
the new task runs. This report does not assert what directory the daemon chooses
for `None`, or that the chosen directory is wrong.

Place New chat with the main navigation. Make its selected workspace visible
and changeable before the first send. Preserve the workspace-specific shortcut,
but give both entry points one understandable creation flow. Starting in a new
repository should be as discoverable as starting another task in a known one.

Evidence: [creation paths](../apps/mew-desktop/src/shell/session.rs#L4),
[workspace plus](../apps/mew-desktop/src/shell/sidebar.rs#L355),
[global entry placement](../apps/mew-desktop/src/shell/sidebar.rs#L1554), and
[checkout text](../apps/mew-desktop/src/shell/chat_render.rs#L2033).
Suggested commands: `/shape`, `/clarify`.

### 3. [P2] Keep project identity legible while switching chats

Chat tabs are limited to 200 pixels and render a message icon, truncated title,
and close control. They do not include workspace identity. The sidebar supplies
workspace grouping, while the muted footer supplies the current checkout path.
This splits orientation across distant locations. Similar chat titles in
different repositories make that arrangement particularly weak.

Keep one stable, legible project/chat context near the conversation. Establish
distinct roles for the sidebar and open-chat tabs: finding work versus returning
to open work. Keep the existing draft, scroll, and auxiliary-state restoration.
Per-chat pane restoration can change the composition during navigation, so test
whether those transitions preserve orientation before deciding to change their
ownership. Do not remove tabs or rewrite navigation without a separate design
decision.

Evidence: [tab content](../apps/mew-desktop/src/shell/topbar.rs#L120),
[workspace grouping](../apps/mew-desktop/src/shell/sidebar.rs#L245),
[checkout text](../apps/mew-desktop/src/shell/chat_render.rs#L2033), and
[view restoration](../apps/mew-desktop/src/shell/session.rs#L653).
Suggested commands: `/layout`, `/clarify`.

### 4. [P1] Correct the resize transition before evaluating narrow layouts

`update_window_width` returns
`width_changed || self.collapse_workbench_if_needed()`. On a meaningful width
change, Rust short-circuits the expression and skips the collapse check. The
resize observer then persists and redraws. The workbench rendering formula can
shrink an expanded pane to 36 pixels at the 720 pixel minimum window width with
the default sidebar. Existing helper tests cover that arithmetic, but do not
exercise the width-update transition. The fit calculation also omits shell gaps
and divider width, so its nominal chat minimum is approximate.

Run the collapse check on every width update and account for actual shell
spacing. Verify a wide-to-narrow-to-wide sequence with the sidebar and review
open. The control-flow defect is confirmed from source; its visible manifestation
has not been reproduced in the native app.

Evidence: [short-circuit](../apps/mew-desktop/src/shell/session.rs#L287),
[observer](../apps/mew-desktop/src/shell/lifecycle.rs#L167),
[width arithmetic](../apps/mew-desktop/src/shell/helpers.rs#L180), and
[existing narrow helper tests](../apps/mew-desktop/src/shell/tests.rs#L920).
Suggested command: `/adapt`.

### 5. [P2] Reduce routine message configuration

The composer exposes five controls, with three session settings sharing its
single horizontal row. Yield remains visible even without another client,
because its visibility depends on whether control has already been yielded.
The problem is their relevance and grouping, not an arbitrary rule that five
controls are always too many.

Keep message entry, attachments, and send prominent. Preserve easy access to
model/effort; place less frequent session choices together. Show handoff controls
when another client makes them useful. Verify long labels and narrow widths
against the complete composer, since the existing narrow input test draws only
the text-input element.

Evidence: [composer controls](../apps/mew-desktop/src/shell/chat_render.rs#L1731),
[model and effort label](../apps/mew-desktop/src/shell/chat_render.rs#L1367),
[Yield visibility](../apps/mew-desktop/src/shell/chat_render.rs#L2097), and
[input-only narrow test](../apps/mew-desktop/src/shell/composer.rs#L1890).
Suggested commands: `/distill`, `/adapt`.

## Secondary interaction findings

- During a normal running turn, Enter returns silently while the composer still
  describes Enter as send. The send button becomes cancel. A drafted follow-up
  needs an explicit supported action or a clear saved-draft state. Do not assume
  runtime support for steering or queueing. See
  [submission guard](../apps/mew-desktop/src/shell/chat.rs#L641) and
  [input description](../apps/mew-desktop/src/shell/chat_render.rs#L1696).
- Multiple clarification questions hide their suggested options and require one
  answer per composer line. Submission pads missing answers with empty strings.
  Give each question its own answer control. See
  [question rendering](../apps/mew-desktop/src/shell/chat_render.rs#L269) and
  [positional answers](../apps/mew-desktop/src/shell/chat.rs#L680).
- Permission cards expose a tool name and compact serialized JSON. Use readable
  action summaries, formatted commands/paths, and clear permission scope. See
  [permission card](../apps/mew-desktop/src/shell/chat_render.rs#L170).
- Close-tab targets are 18 pixels and workspace actions are 22 pixels. Icon-only
  topbar/composer actions lack hover tooltips. Native hit testing, readability,
  and pointer use still need inspection.
- Review-file rows, terminal controls, and attachment-removal controls do not
  consistently use the role/label/focus pattern found on primary controls.
  These are source-based accessibility risks, not verified VoiceOver failures.

## What to retain

- Workspace grouping, bounded recent tasks, search, and Show more already form a
  useful navigation base. There is no justification here for another sidebar
  architecture or a shell rewrite.
- Per-session drafts, attachments, transcript position, and auxiliary state have
  careful ownership and behavior-test coverage. Improve their presentation while
  retaining those protections.
- Keyboard navigation, native text input, Unicode/IME handling, virtualization,
  and tool/reasoning disclosure provide a substantial foundation.

## Heuristic assessment

These are tentative source-only scores, where 4 is excellent. Visual treatment
and in-app help are unscored. There is no defensible overall score out of 40 yet.

| Heuristic | Score | Evidence or limit |
| --- | --- | --- |
| System status | 3/4 | Working/retry/reconnect surfaces exist; running submission is silent. |
| Real-world language | 2/4 | Readable tool summaries coexist with raw permission JSON and client/daemon terms. |
| User control | 2/4 | Cancellation and draft protection exist; follow-up and question interaction are weak. |
| Consistency | 2/4 | The same composer changes between message entry and positional question answers. |
| Error prevention | 2/4 | Strong draft ownership; working-location choice and answer mapping remain unclear. |
| Recognition | 2/4 | Workspace grouping helps; location choice and title-only tabs split context. |
| Efficiency | 3/4 | Search, shortcuts, keyboard navigation, and retained task state. |
| Aesthetic/minimalist design | Unscored | Native visual inspection unavailable. |
| Error recovery | 3/4 | Retry/reconnect and preserved work; internal error language remains in places. |
| Help/documentation | Unscored | In-app help not comprehensively inspected. |

The cognitive-load checklist has three source-supported failures: progressive
disclosure in the default layout, one-at-a-time answering, and question-to-line
working-memory demands. This is a moderate source assessment. Native single
focus and visual hierarchy remain unscored. Control counts alone do not establish
overload, and secondary controls are not all alternatives at one decision point.

No claim that the app looks AI-generated is justified without native images.
The markup-based `impeccable` detector is inapplicable to the Rust GPUI views;
the independent deterministic inventory examined controls, sizes, defaults,
conditions, and test scope instead. No automated visual-detector result is claimed.

## Persona checks

- **Power user:** can search and use chat shortcuts, but title-only tabs make
  similar tasks harder to distinguish across projects. A follow-up drafted
  during a run has no visible send action.
- **First-time user:** must find New conversation below the task list, discover
  a workspace-specific plus to choose context, and understand supporting panes
  before they contain useful work.
- **Keyboard/screen-reader user:** many main controls have labels and visible
  focus styling, but the pattern is uneven in review and terminal controls.
  Actual traversal, announcements, contrast, and VoiceOver remain unverified.

## Recommended sequence

Reflecting the user's priorities:

1. `/distill` and `/layout`: establish a conversation-first composition with
   supporting panes closed until useful. Keep the existing implementation.
2. `/shape` and `/clarify`: design one clear new-chat flow with a visible working
   location, and clarify project identity when switching among open chats.
3. `/adapt`: fix resize collapse, account for shell spacing, and verify the full
   composer at narrow widths.
4. `/polish`: refine targets, tooltips, typography, and boundaries after the
   composition and navigation work in the native app.

These are recommendations, not implemented changes. Fonts, colors, motion,
content density, and the final visual score require live comparison.

## Native comparison still required

Compare both installed apps at the same window size with real local data:

1. A new chat in a selected repository, before sending.
2. Two projects with similarly named chats, switching between them while a
   draft and scroll position are retained.
3. A running task with a drafted follow-up, then its completed result.
4. Changes opened beside the conversation, followed by narrow/wide resizing.
5. A permission request and a multi-question clarification.

Native inspection is currently blocked by computer-use error `-10005`, reporting
`failed to resolve feature override precedence: Operation not permitted`.
Screenshots of the installed apps can establish layout, but interaction and
motion still require working native access. The official example cannot replace
this comparison.
