# GPUI UX comparison with Codex, 2026-10-24

This review records the initial source snapshot. The sidebar implementation
follow-up at the end records changes made after the user supplied a Codex screenshot.

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

## Sidebar implementation follow-up

The user supplied an installed Codex screenshot and chose to consolidate chat
navigation into the sidebar. The native shell now uses a continuous sidebar and
titlebar surface, a workspace/chat title in place of the conversation tab strip,
New chat above the workspace list, and search/filter controls revealed on demand.
Idle workspace-root chats use compact rows; path and age remain available in
tooltips and accessible labels, while nested directories and attention states
retain visible detail. Selected highlighting spans the full rounded row.

New chat inherits the selected workspace. Selecting a chat reveals a collapsed
workspace, reselecting an attached chat retains loaded content, and closing a
chat preserves its draft for reopening from the sidebar. Obsolete conversation
tab state, title synchronization, and numeric tab shortcuts are removed. Browser
page ownership, per-chat drafts, and retained transcript/workbench state remain
in their existing layers. The resize short-circuit identified above is fixed.

Actual GPUI entity and painted-geometry regression tests cover these behaviors.
The user screenshot establishes the Codex reference; native mew appearance and
motion still require a live pass because computer use remains unavailable.

## Pane layout implementation follow-up

The user clarified that the reference calls for three full-height panes, with a
larger work area beside the conversation. The earlier sidebar styling pass did
not establish that structure. The shell now gives navigation, conversation and
work content their own aligned top headers and continuous resize boundaries.
There is no global header spanning their content.

Navigation remains the sole chat switcher. Conversation width starts at 480 px;
the work pane fills the rest of the window and receives additional space when
the window grows. The divider changes the conversation width and retains that
preference per chat. Older saved layouts adopt the new conversation default.
The 420 px conversation and 360 px work minima include divider accounting.
Navigation width also clamps when the window narrows, preserving chat usability.

The work pane uses horizontal Browser, Changes, Files and Activity tabs instead
of a vertical icon rail. Changes lets the diff fill the document area, with
changed files beside it in wide panes and above it in narrower panes. The
terminal sits below work content in split view and below the conversation when
work is folded. Existing draft, browser-page, request and terminal ownership
remain in place.

GPUI tests exercise actual pane/header geometry, divider dragging, wide-window
growth, tab clicks, terminal docking, narrow collapse, binary/text diff layout
and reduced-motion pane toggles. Config tests cover legacy layout loading and
new sizing persistence. Native inspection was retried and still fails with the
OS permission error above; these tests do not establish live appearance,
VoiceOver behavior or motion quality.

## Tab workspace implementation follow-up

The user's second screenshot refines the work pane into a browser-style tab
workspace. The fixed category strip is replaced by closable tabs and a plus
control. New tab provides URL entry and a launcher for existing Changes,
Terminal, Files and Activity tools. Tool views reuse an open tab; browser pages
remain independent, including URL drafts, titles, errors and native lifetimes.
Transcript links open a separate browser tab. File-tree actions retain their
existing external-open behavior.

The terminal fills its work tab and keeps the narrow-window fallback below the
conversation. Chat navigation remains in the left pane. Command-T opens a work
tab, and Command-W closes work content within its focus scope while retaining
chat-close behavior in the conversation. Tab order, selection and browser target
are retained per chat; new chats start with a fresh tab workspace.

CEF ownership now includes the tab identity. The agent's browser target follows
the last selected browser/new-tab page for that chat. Closing a page revokes
only its target and controller. Asynchronous registration and revocation replies
are drained before reactivating the same page identity, preventing an old
registration failure from marking a reselected page unavailable. Late page
callbacks cannot overwrite another tab's metadata or address draft.

GPUI tests cover opening/closing tools, keyboard routing, overflow, full-pane
terminal geometry, hidden-field editing, per-chat restoration and browser target
selection. Browser-host tests cover independent same-session pages and stale
callbacks. The real-daemon browser test passes with isolated headless Chrome and
the bundled agent-browser executable. These checks still leave live native
appearance, CEF embedding, VoiceOver and motion unverified because native
computer use remains blocked by the OS permission error.

## Native verification follow-up

Native inspection became available after reverting the recent Codex/OpenCode
configuration changes. A disposable app bundle with isolated settings, sessions,
CEF cache and a real fake-provider daemon now exercises the implemented panes.
The live mew comparison uses the user's Codex screenshots. Direct inspection of
the Codex app is denied by the inspection tool's safety policy.

The native pass reproduced and fixed four gaps:

- Transcript rows touched the conversation dividers because pinned GPUI lists
  apply vertical padding only. Horizontal gutters now belong to the list's parent.
- Target replies lost their owning CEF JSON Value before nested dictionaries
  were read. Keeping that owner alive restores exact native target registration.
- Closing a browser page used CEF's default close handler, which closes the
  containing GPUI window. The handler now queues removal of only its child view;
  the retired target disappears and sibling pages survive.
- A fresh pinned agent-browser session created a separate Chromium blank page.
  Registration now selects and persists the existing target before pinning it,
  checks the returned target and persistence warnings, and disconnects without
  reconnecting during cleanup.

The real native smoke confirms aligned full-height headers, message gutters,
sidebar collapse/restore, new-chat navigation with an unsent draft restored,
Files and full-pane Terminal launch, independent browser inputs across tab/chat
switches, and restored browser navigation after restarting on a tool tab. Blank
page replacement keeps the app alive and removes the retired CDP target. No
extra Chromium window appeared with the corrected registration flow.

The desktop, CEF host and tools suites pass, along with the opt-in native CEF
parser regression, real-CLI page-isolation regression and real-daemon browser
tests. The CLI regression compares actual page-target sets before registration,
after browser operations and after closing owned pages. Native divider dragging
could not be established with the inspection tool; GPUI pointer/geometry tests
cover it. VoiceOver, physical IME and motion quality remain unverified.

## Active conversation review after activity grouping, 2026-10-24

Reviewed commit `da98960` with two independent assessments and a fresh native
pass in the isolated QA app. Compared the supplied Codex screenshots, including
the compact activity rows, inline working marker and edited-files handoff.
Direct inspection of Codex remains denied by the computer-use safety policy.
This establishes a screenshot comparison, not current Codex interaction parity.

The native checks covered the 480 px conversation pane, the wider centered
reading column, grouped command disclosure and an eight-line unsent draft.
The draft was cleared afterward. No product code was changed for this review.
The Impeccable HTML/DOM detector does not support Rust GPUI; the second
assessment instead inventoried the actual render, control and input paths.
No automated detector results or performance measurements are claimed.

### Main assessment

The three-pane ownership, shared reading column and grouped completed activity
now give the chat a coherent structure. The remaining roughness is concentrated
in the editor, content interactions and disclosure hierarchy. Another broad
layout rewrite is not supported by these findings.

The visual treatment does not show the gradient/glass/card-grid patterns the
design detector targets. The more relevant risk is a log-like reading rhythm:
single tools still expose completion chrome, and expanded details prioritize
raw input JSON over the result a reader usually wants.

### Priorities

| Priority | Finding and evidence | Next change |
| --- | --- | --- |
| P1 | **Long drafts overlap controls.** Native paste of eight explicit lines retained all eight in the accessible field value, but painted text through the toolbar/footer and left the end out of view. The composer caps its allocated height, paints from the top and has no caret-following scroll offset. | Implement a bounded scrolling editor, clip text/caret to the editing viewport and keep the active selection/caret visible. Test explicit lines and wrapped text before further cosmetic work. |
| P2 | **Useful content lacks direct reuse actions.** Code fences and tool output use bare text elements rather than the selectable transcript renderer, and expose no copy control. Native inspection confirms the absent actions; clipboard routing was not exercised. | Add selection/copy for code and command results, with a contextual copy button. Validate focus and selection behavior after writing in the composer. |
| P2 | **Markdown loses structure.** Source parsing strips ordered-list markers and nesting; rendering always adds bullets. Tables remain pipe-delimited text rows. Every virtual code chunk receives another rounded card and padding. | Preserve list markers/depth and table cells, and make virtual code chunks read as one continuous block. These are content-fidelity fixes, independent of the comparison. |
| P2 | **Activity disclosure remains uneven.** Groups use one muted row; single completions still show a tool name, green completed label and a second command/path line. Native group inspection requires opening the group and the call, then shows input JSON before output. Errors remain inside collapsed details. | Give singleton completions the same quiet treatment. Put the command and result first, move raw input to secondary details and show an error excerpt immediately. |
| P2 | **Running feedback changes abruptly.** Source shows a bordered Working card before assistant output, then removes it once an assistant item exists, even when the turn is still running. The Codex reference shows a small Working-for marker. | Keep a stable, small turn-status row near the live tail. Reserve attention cards for requests and failures. This running-state finding is source-supported, not a fresh native streaming reproduction. |
| P2 | **Composer administration is still prominent.** Permission, persona and model controls sit beside attachment/send; a separate footer repeats workspace context and always exposes Yield. Native inspection confirms the persistent controls in both widths. | Keep frequently changed choices close to writing and move persona/secondary session controls into a menu. Reveal collaboration controls when another client makes them useful. |
| P3 | **Edited work has a weak handoff.** The Codex reference includes filenames, change totals and View changes. Mew exposes review through a header icon and individual tool diffs. | Consider a compact edited-files result that opens the existing Changes tab. This is a product opportunity, not a current correctness defect; Undo is a separate capability and is not implied. |

Source anchors:

- Composer layout, shaping and painting: `shell/composer.rs`,
  `ComposerElement::{request_layout, prepaint, paint}`; editor bounds and toolbar:
  `shell/chat_render.rs`, `render_center`.
- Content selection/actions and code/table rendering: `shell/chat.rs`,
  `render_inline`, `render_markdown_block`, `transcript_mouse_down`,
  `shell_key_down`; tool input/output: `shell/chat_render.rs`, `render_tool_text`.
- Markdown structure: `markdown.rs`, `parse_block`, `parse_list_items`,
  `virtualize_block` and `split_lines`.
- Activity/status: `shell/chat_render.rs`, `render_tool_part`,
  `render_tool_group`, `render_transcript_attention`; `shell/helpers.rs`,
  `transcript_attention`.

### Smaller findings and unverified candidates

- Empty send remains visually enabled while whitespace submission returns
  silently. Disable/dim it consistently with actual availability.
- The 15 px/24 px prose setting is comfortable in the wide reading column.
  The narrower pane feels heavier than the supplied Codex stream, but this pass
  found no evidence that normal prose is accidentally assigned bold weight.
  Keep typography changes behind the editor/content fixes and verify them with
  equal-width, equal-content captures.
- Source suggests that selecting prose after focusing the composer may leave
  Cmd-C routed to the editor. A focused behavior test is needed before calling
  this a reproduced clipboard bug.
- Source-file links currently enter the browser URL path rather than a
  file-aware handoff. Test absolute paths, relative paths and line references
  before choosing an editor or external-open behavior.
- Physical IME, VoiceOver, actual long-running provider turns and Codex's own
  disclosure/copy interactions were not exercised in this pass.

### Design health and audience checks

These are provisional design judgments for the active conversation, not measured
usability outcomes or a whole-application accessibility score.

| Heuristic | Score / 4 | Basis |
| --- | --- | --- |
| Visibility of system status | 2 | Running label is conditional on transcript shape. |
| Match with user language | 3 | Activity phrases are clearer; Yield and raw JSON need context. |
| User control and freedom | 2 | Long-draft editing and content reuse have gaps. |
| Consistency and standards | 2 | Singleton/group activity and markdown fidelity differ. |
| Error prevention | 2 | Draft ownership is retained; empty-send availability is misleading. |
| Recognition over recall | 3 | Pane identity and disclosures are visible; result review is indirect. |
| Flexibility and efficiency | 2 | Keyboard disclosure works; code/output reuse is missing. |
| Aesthetic and minimalist design | 3 | Shared column is sound; composer/footer chrome can be quieter. |
| Error recovery | 2 | Failure labels are visible, but the useful error is initially hidden. |
| Contextual help | 1 | Active-chat controls offer little explanation of collaboration and raw tool details. |
| **Total** | **22 / 40** | **Focused behavior and hierarchy work remains.** |

Cognitive-load checklist: two of eight areas need attention, minimal choices in
the persistent composer and progressive disclosure inside tool inspection.
The main reading focus, chunking, grouping, visual priority, independent writing
decision and retained workspace context are materially improved. This is a
moderate-load judgment; the number of controls alone does not establish that
users must make every choice on each prompt.

For a coding power user, the major friction is pasting a substantial prompt,
reusing command/code output and finding the changed files. For a returning TUI
user, persistent Yield/persona controls make the chat feel more administrative
than necessary. For a keyboard/assistive-technology user, disclosure activation
is available, but editor scrolling and copy/focus behavior need direct coverage;
VoiceOver conclusions would be premature.

Recommended order: fix editor overflow and validate copy/selection first, then
restore markdown/content fidelity, then refine activity/progress and composer
hierarchy. Treat the edited-files handoff as a separate product decision. Finish
with a spacing/type pass once those behaviors are stable.

## Content interactions follow-up, 2026-10-24

Resolved the direct reuse gap with selectable code and tool input/output/diffs,
plus controls that copy complete fences/sections rather than visible chunks or
preview lines. A failing behavior test confirmed the suspected stale composer
Copy focus; transcript selection now owns its typed Copy route. Six focused
tests cover complete sources, Unicode, offscreen chunks and real pointer/focus
behavior. Native QA confirms code/result buttons and partial selection copying
after draft editing, with readable selection in both theme modes. This does not
resolve list/table fidelity, repeated code cards or source-file link routing.

## Draft editing follow-up, 2026-10-24

Resolved the overflowing-draft defect with complete shaping, measured wrapping
and an internally scrolling, clipped editor. The caret, selection, pointer
mapping and native candidate geometry share the scroll offset. Vertical keys
edit visual rows before history recall and preserve their horizontal position
through short lines and soft wraps. Unchanged composition text can move its
selection without waiting for another paint.

The 247-test desktop suite and strict lint/build gates pass. Actual GPUI tests
cover narrow/wide wrapping, caret clipping and revelation, manual scroll,
another field owning native focus and immediate IME geometry. Native QA confirms
an eight-line draft stays within its viewport and supports wheel scrolling and
Shift-arrow selection. Captures became stale during the later wrapped/pane-width
check, so visual verification of that transition remains open. Physical IME and
VoiceOver remain unverified. The remaining markdown, activity/progress and
composer hierarchy findings keep their existing priority.

## Continuous code follow-up, 2026-10-24

Resolved repeated code cards and artificial line breaks at virtual boundaries.
Code chunks now end at physical newlines, share one surface and horizontal
position, and expose one header/copy control per fence. A GPUI regression
reproduced a 66 px seam before the fix; it now matches normal 20 px leading.
Narrow/wide geometry, source fidelity, adjacent fences, streaming scroll state
and existing copy behavior pass. The native 32-line fixture confirms continuous
spacing and complete copy, with readable syntax in both themes.

## Lists and tables follow-up, 2026-10-24

Resolved flattened list markers/depth and pipe-delimited table rendering. Lists
retain ordered starts, nested markers and hanging wrapped text. Tables retain
cells, alignment and a single header across virtual rows, wrap within shared
columns and scroll horizontally inside the reading column. Inline formatting,
links, escaped pipes and code-span pipes survive the projection. Structured
selection uses parent block identities and byte offsets; copied table rows have
tabs between cells.

Parser and actual GPUI tests cover source structure, streamed/final parity,
narrow/wide geometry, left/center/right placement, cross-cell selection and
shared scrolling across virtual table chunks. All 261 desktop tests and the
lint/build gates pass. Native split/wide QA confirms ordered/nested markers,
aligned wrapping tables and Unicode row copy in dark/light themes. The requested
three markdown gaps are addressed. VoiceOver and physical IME remain unverified;
full nested block content and interactive task checkboxes remain outside this
pass. Activity/progress and composer hierarchy keep their existing priority.

## Activity and progress follow-up, 2026-10-24

Single completed tools now match grouped activity with one muted action row,
including the command or path, and no completed badge. Long and multiline
commands remain one visual line with complete accessible labels. Expansion,
keyboard disclosure and complete input/output/diff copying remain available.
Collapsed failures expose the first useful error line. Expanded details retain
their existing input/output ordering; changing that inspection flow remains open.

The bordered Working card is replaced by a small status line above the composer
with a gentle muted pulse. It stays visible through partial assistant replies
and tool activity, and works when running metadata precedes history. Stop lives
in the composer. Required actions replace the working line; partial failures
retain a retry card with the original prompt.

All 266 desktop tests pass, including failing-first geometry and transition
coverage, plus strict Clippy, native build/packaging, formatting, architecture
and generated-theme checks. Isolated native QA confirms quiet completed rows,
visible failure excerpts and output copy in dark/light and split/wide layouts.
Real daemon/fake-provider turns were briefly paused after an actual text delta
to inspect partial replies, then resumed to verify status removal on completion.
This checks native presentation and state transitions, not production provider
latency. VoiceOver and physical IME remain unverified. Composer hierarchy and
the edited-files handoff keep their existing priority.

## Sidebar inbox and chat rows follow-up, 2026-10-24

The supplied sidebar references exposed two remaining presentation problems:
rows grew for status or nested working folders, and a 28-character title cap
discarded content before layout could measure it. Chat rows now keep one 30 px
line and retain complete normalized titles. Age and pin/archive controls share
a fixed trailing slot, so hovering preserves text geometry. Idle dots and read
failure markers are quiet; full title, workspace, working directory and status
are available in the preview and accessible labels. Right-click or Shift-F10
opens further actions. Menu Escape/outside dismissal is directly tested.

The requested notification mode is a sidebar Inbox for unread results and
requests. It has a permanent entry/count and a flat list across workspaces,
prioritizing requests before recent results. Archived chats and read failures
alone do not qualify. Read acknowledgement comes from the shared client; opening
a request leaves it pending. The existing archive count now opens recoverable
chats with Unarchive, and new chats leave either filtered mode.

All 275 desktop tests and strict lint/build gates pass. Native dark/light and
narrower/wider sidebar checks confirm layout, pin/archive/restore and menu
dismissal. Two real daemon/fake-provider background turns exercised unread
arrival, reading and the empty inbox. Actual GPUI hover/timer tests verify the
full-title preview and stable row bounds; native inspection could not reliably
sustain hover. VoiceOver and physical IME remain unverified.

## Narrow composer follow-up, 2026-10-24

The wrapping toolbar made settings and Send land on different rows, with a
reproduced 36 px baseline jump at the 420 px conversation minimum. Settings now
share one fixed toolbar row with 11 px labels, tighter spacing and text-led
persona/model controls. Bounded labels truncate within the row; Send/Stop keep
their full targets and pickers retain measured trigger anchors. Short model
names keep a natural button width beside Send in wider conversations. Footer
labels are smaller while draft typography and internal scrolling are preserved.

Selected values remain explicit, with short placeholders only for unset choices.
The daemon currently supplies model identifiers and descriptions, without a
separate friendly-name field; this pass does not invent names from those IDs.

All 276 desktop tests and lint/build gates pass. Actual GPUI tests cover four
widths, long/short labels, Unicode, Send/Stop and all picker triggers. Native
dark/light checks confirm the narrow/wide layout, dropdown placement and a long
scrolling draft. The broader persona/secondary-control hierarchy finding remains
a separate design decision. VoiceOver and physical IME remain unverified.


## Composer comparison after friendly model names, 2026-10-24

Reviewed commit `2bf41a2`. This pass compares live native mew at 420, 480 and
600 px conversation widths against the user's supplied Codex screenshot. Direct
Codex inspection remains denied by the computer-use tool's safety policy; the
reference establishes visible composition, not current Codex interaction or
behavior at narrower widths. No product code was changed.

Two independent assessments covered visual hierarchy and the control/state
inventory. The Impeccable HTML/JSX detector has no supported native Rust target,
so no detector score or browser overlay is claimed. Native inspection used the
owned QA app, a real daemon, a local catalog and loopback provider endpoints.
No provider turn was submitted.

### Main diagnosis and width comparison

The single-row toolbar now holds its geometry, but selected state loses space
at the minimum width. The Codex reference groups attachment/approval on the left
and model/effort/send on the right. Mew adds a persistent persona choice and a
separate Yield footer. The useful next step is a narrower hierarchy with visible
model/effort state, rather than another reduction in label size.

| Conversation width | Native result with builder and Claude Sonnet 4.5 · high |
| --- | --- |
| 420 px | builder becomes `buil…`; the model remains recognizable but the effort suffix disappears. Controls remain aligned. |
| 480 px | Persona, model and effort fit in full on the same row. |
| 600 px | The same values fit; the model control keeps its natural width. |

Local captures: [420 px empty](/private/tmp/mew-composer-comparison-2026-10-24/420-empty.png),
[480 px empty](/private/tmp/mew-composer-comparison-2026-10-24/480-empty.png),
[600 px empty](/private/tmp/mew-composer-comparison-2026-10-24/600-empty.png).
These widths were set in the stopped QA fixture and confirmed visually after
launch. A computer-use divider drag did not change width and is excluded as
resize evidence.

### Priority findings

1. **P1: long attachment names hide removal.** A real file-picker selection with
   a long basename extends beyond the 420 px pane and puts the remove icon out
   of view. The chip has no width constraint or truncating filename; its remove
   div also lacks a button role, accessible label and keyboard focus treatment.
   Bound the chip to the available width, truncate only the filename and reserve
   a fixed, accessible remove target. Suggested command: `/adapt`.
   Evidence: [native attachment](/private/tmp/mew-composer-comparison-2026-10-24/420-long-attachment.png),
   [attachment rendering](/Users/natalie/code/mew/apps/mew-desktop/src/shell/chat_render.rs:2036).

2. **P1: the primary control does not follow submission state.** Native empty
   drafts show a fully available Send button, while whitespace-only submission
   returns silently, including attachment-only drafts. More seriously, source
   permits Enter to answer an AskUser request during a running turn, but the
   visible primary button still cancels that turn. The question card instructs
   users to answer in the composer and has no free-text submit button of its
   own. Derive the primary action, label and disabled state from the same mode
   and eligibility rules as submission. The AskUser discrepancy is confirmed
   in source, not reproduced in a live request during this pass. Suggested
   command: `/clarify`, backed by interaction tests.
   Evidence: [submission](/Users/natalie/code/mew/apps/mew-desktop/src/shell/chat.rs:1100),
   [primary controls](/Users/natalie/code/mew/apps/mew-desktop/src/shell/chat_render.rs:2413),
   [question instructions](/Users/natalie/code/mew/apps/mew-desktop/src/shell/chat_render.rs:298).

3. **P2: ordinary selected values disappear at 420 px.** Model and effort share
   one truncating string, so the effort suffix goes first. Persona competes with
   that string even when its value is the ordinary builder default. Keep effort
   outside the model-name truncation span and move lower-frequency persona
   configuration into secondary settings at narrow pane widths. Preserve a
   visible permission state. Suggested command: `/distill`.
   Evidence: the width captures above and
   [model label](/Users/natalie/code/mew/apps/mew-desktop/src/shell/chat_render.rs:1800).

4. **P2: popup placement uses different dimensions from rendering.** The model
   menu renders at 400 px, but its right-alignment calculation receives 300 px.
   Placement counts only filtered catalog models; actual height also counts
   Recent duplicates and two headers. With one model and one recent, the height
   difference is 192 px before the shared effort panel. Native window snapping
   prevents some overflow but can shift the oversized menu over the composer.
   Use the same row inventory and dimensions for rendering and placement, and
   constrain overall popup height to available window space. Suggested command:
   `/adapt`.
   Evidence: [native picker](/private/tmp/mew-composer-comparison-2026-10-24/420-model-picker.png),
   [placement](/Users/natalie/code/mew/apps/mew-desktop/src/shell/chat_render.rs:2593),
   [rendered size](/Users/natalie/code/mew/apps/mew-desktop/src/shell/preferences.rs:934).

5. **P2: long drafts do not reveal their extent.** All eight pasted lines remain
   intact and caret-following scrolling works. The 96 px editor shows the last
   four 24 px lines, with no scroll thumb, overflow hint or expand affordance.
   Give writers a restrained scroll cue, and consider expansion when they need
   to inspect a long prompt. Additional height should follow available window
   space. Suggested command: `/polish`.
   Evidence: [native long draft](/private/tmp/mew-composer-comparison-2026-10-24/420-long-draft.png),
   [height cap](/Users/natalie/code/mew/apps/mew-desktop/src/shell/composer.rs:1480).

### Smaller observations and strengths

Yield remains visible with no other clients; the footer's nonshrinking status
area can also accumulate usage and presence text. Make collaboration controls
contextual. The model picker uses 64 px section-header rows and repeats recent
models in All Models, spending substantial height on organization. Compact
headers could expose more choices. Shift+Enter exists in source but is absent
from the input's accessible editing hint.

The stable toolbar, readable catalog names, fixed Send target and shared reading
column are working well. Draft text stays readable at 15 px, and the scrolling
fix preserves all content. No strong ornamental template patterns were found;
the remaining issues concern component behavior and control density.

### Provisional heuristic assessment

These are subjective composer scores based on the evidence above. Cognitive
load is moderate: 2 of 8 checklist items fail, minimal choices and progressive
disclosure. A power user loses effort visibility at 420 px; a keyboard or
assistive-technology user lacks a named attachment-removal action; a long-prompt
writer has no cue for hidden draft content. Physical IME and VoiceOver were not
exercised in this pass.

| Heuristic | Score / 4 | Main evidence |
| --- | --- | --- |
| Visibility of system status | 2 | Effort and draft extent become hidden. |
| Match with the real world | 3 | Catalog names and familiar send/attachment controls. |
| User control and freedom | 2 | Attachment removal becomes unreachable. |
| Consistency and standards | 2 | Enter can answer while the primary button cancels. |
| Error prevention | 2 | Empty Send appears available. |
| Recognition over recall | 2 | Truncated selected settings require opening a picker. |
| Flexibility and efficiency | 3 | Keyboard editing, search and preserved drafts. |
| Aesthetic and minimalist design | 2 | Persistent persona and collaboration controls. |
| Error recovery | 3 | Existing retry surfaces retain the draft. |
| Help and documentation | 2 | Basic input hint omits newline/overflow guidance. |
| **Total** | **23 / 40** | Improvements concentrated in narrow and conditional states. |

Recommended sequence: `/adapt` for attachment and popup geometry, `/clarify` for
primary-action state, `/distill` for narrow settings hierarchy, then `/polish`
for draft overflow cues and final state coverage. Existing composer geometry
tests cover widths and non-overlap, but do not establish attachment removal,
AskUser primary-button semantics, full selected-state legibility or popup fit.
The next implementation should reproduce those cases before changing behavior.

All owned QA processes were stopped, the test draft cleared and the fixture's
420 px width restored. No production app preferences or sessions were changed.


## Composer follow-up: interaction fixes and narrow settings

Implemented the first four findings from the compact-composer review. Attachment
filenames now truncate within the reading column while a named, tabbable remove
button remains visible. The primary action is disabled for empty or whitespace
drafts and pending model switches. A running AskUser request shows an answer
action matching Enter, with the original request identity and response format.

Model-picker rendering and placement share the actual row inventory, width and
height. The popup includes Recent/header rows, fits available window space and
scrolls the model list without shrinking search or effort. Deferred trigger
notifications keep model, persona and permission menus attached during resizing.

The settings row prioritizes permission, model and effort at the minimum pane
width. Persona uses its existing picker through a compact icon below 480 px.
Effort is separate from the truncating name. Measured editor width controls this
change, including animated pane transitions, while wider panes retain persona
text and naturally sized model controls. Draft typography is unchanged.

Verification: twelve new actual-GPUI regressions and all 291 desktop tests pass,
as do strict desktop Clippy, native build/packaging and repository checks.
Native QA used the isolated real daemon/catalog fixture, with no provider turn.
The 420 px dark view retains the full ordinary model/effort label and a visible
attachment remove button; the 480 px light view restores persona text. Native
accessibility reports disabled Send for an empty draft and enables it after
typing. Pointer removal preserves the Unicode draft, and both model and persona
controls open their existing menus.

Evidence: [420 px composer](/private/tmp/mew-composer-fixes-2026-10-24/420-empty.png),
[long attachment](/private/tmp/mew-composer-fixes-2026-10-24/420-attachment.png),
[model picker](/private/tmp/mew-composer-fixes-2026-10-24/420-picker.png),
[480 px light view](/private/tmp/mew-composer-fixes-2026-10-24/480-light.png).

The owned QA app and daemon are stopped, the draft cleared and the fixture's
420 px/system appearance restored. Long-draft overflow cues, contextual Yield
and picker header density remain outside this pass. Physical IME and VoiceOver
were not exercised.
