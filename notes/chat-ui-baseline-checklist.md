# Chat UI baseline checklist

Date: 2026-10-27
Status: packaged chat/browser UI proof verified; full migration acceptance pending

This checklist implements the baseline step of the
[migration plan](cef-ui-migration-plan.md). The current milestone is the familiar
React, Tailwind and shadcn chat shell with a tabbed right pane, its new-tab
launcher, and a real external browser page beside a working daemon conversation
in the packaged app. Stop at that UI milestone before expanding the migration.

The tables distinguish code and test sources inspected from acceptance actually
performed. A cited test is existing coverage to reuse or extend, not a claim
that it was run for this checklist. Every unchecked acceptance case remains
pending in the replacement UI. Mockup message echoes, terminal output and browser
previews do not satisfy these cases.

## Current milestone

- [x] Inventory the baseline, owning layers, existing coverage and remaining
  acceptance cases below.
- [x] Rebuild the familiar frame and composer in the existing `mew-web-ui`
  package using React, Tailwind and shadcn controls.
- [x] Keep the conversation/composer visible beside a collapsible
  right pane with a single tab strip and new-tab launcher.
- [x] Load the built UI in the packaged desktop host and complete a real daemon
  turn while an external CEF browser tab remains usable beside it.
- [ ] Verify focus, native editing, IME, menus/dialogs, pane bounds, browser
  identity, theme application and owned-process shutdown in that packaged app.

### UI proof performed on 2026-10-27

The packaged debug app ran the compiled React assets without Vite. An isolated
daemon with the deterministic fake provider completed real WebSocket turns and
streamed assistant text beside a retained native CEF page. The external page
accepted text input; navigation updated its tab title. The conversation remained
visible while opening and closing the workbench and selecting tabs. The native
window was also inspected at two sizes.

The summary belongs to the chat pane. It appears automatically at that pane's
right edge when the pane is at least 1040 CSS pixels wide, including beside an
open workbench. In narrower chat panes its control remains in the chat header.
It shows current-chat change totals, subagent status, background processes and
pinned sources; its entries select the existing detail surfaces. Sources
currently reflect pinned files, not a complete attachment/source index.

All 161 React tests, the production TypeScript/Vite build and 334 desktop tests
passed. Desktop tests used isolated configuration and a single test thread to
avoid shared native/global-state races. The real hidden AppKit harness verified
editing routing, Undo/Redo menu targets and visible accessibility-root
composition. The palette test and generated-theme check passed. Native editing
was also exercised in the actual packaged browser and composer.

IME, full native shortcut/menu acceptance, live installed-theme acceptance and
measured resource use remain pending. The
packaged accessibility tree still does not expose the CEF document through the
computer-use interface, despite the passing composition harness. The opt-in
shell currently supports local loopback connections; saved remote profiles,
durable UI view-state import and the rest of the baseline remain migration work.
The existing native desktop stays the default. This is the stopping point for
the requested UI milestone, not full replacement acceptance.

### Packaged polish verification on 2026-10-27

The UI now fills the former native titlebar row. Trusted app header drag regions
use macOS window dragging and exclude interactive controls; external pages do
not receive the drag overlay. Inspected expanded and collapsed navigation, 900px
and wider split windows, light/dark themes, browser input and restored browser
content after Escape dismisses a menu. The browser retained its input through
pane resizing and tab switches. Pointer resizing was exercised in the split
view; keyboard divider resizing also verified the summary appearing inside the
chat pane and tucking away as that pane narrows. The compact summary remains in
the chat header when the workbench is open.

All 193 React tests, the production build, 334 isolated serial desktop tests,
20 native CEF library tests and the real AppKit editing/accessibility/drag
harness passed. The generated-theme check and debug packaging passed. The
existing Vite large-chunk and Rust dependency future-compatibility warnings
remain. The same isolated daemon and deterministic fake provider were used;
this pass does not establish production provider or full migration parity.

Photos are in the task's visualization folder:
`mew-polish-07-narrow-header.png`, `mew-polish-09-browser-summary-light.png`,
`mew-polish-10-collapsed.png`, `mew-polish-12-summary-compact.png` and
`mew-polish-14-summary-in-chat.png`, `mew-polish-15-chat-summary-browser.png`
and `mew-polish-16-chat-popup-browser.png`. Earlier wide-summary photos show the
superseded page-edge placement; photos 14-16 show the corrected chat-pane layout.
The native browser remains visible beside a compact summary that does not
overlap it. Overlapping tool menus and modal dialogs still hide native content
until dismissal; bounds and modal regressions cover this distinction.

Summary motion verification: the summary now uses a 180ms slide/fade with
transform-based chat-column repositioning. Divider resizing and initial layout
remain immediate. Reduced motion disables the movement and cancels active
column animations on a live preference change. Regressions cover focus in both
layout directions, repinning an open popover and motion cancellation. All 194
React tests and the production build pass. Packaged smoke retained browser
input through pane close/reopen; `mew-motion-01-compact.jpg`,
`mew-motion-02-expanded.jpg` and `mew-motion-03-summary-browser.jpg` record the
resulting layouts. Temporary slow inspection timing was restored to 180ms.

Workbench motion verification: browser and tool panes now open and close over
the existing 260ms panel token. Their actual resizable geometry animates, with
retained contents through exit and immediate logical/input deactivation. Native
browser controls, focus and accessibility restore on reopening the same page.
Reduced motion, live preference changes, reversal and StrictMode initialization
have regression coverage. Pointer divider resizing passed in the browser
harness; native keyboard resizing, chat draft retention, browser form retention,
editing after reopen and Files close/reopen passed in the packaged app. Native
window dragging was unavailable through the automation API during this pass.
All 201 React tests, 335 isolated serial desktop tests, 21 CEF library tests,
11 browser-host tests, the real AppKit harness, production build and debug
packaging passed. Photos `mew-pane-01-closed.jpg`,
`mew-pane-04-tool-reopened.jpg` and `mew-pane-06-chat-browser.jpg` record the
verified layouts.

The complete baseline remains the replacement's release bar. Direct subagent
messaging is required for v1, but its runtime/controller work and the remaining
workbench tools are beyond this first UI milestone. Keep the existing desktop
available for comparison until the full baseline is accepted. Controls must not
report success for a capability that has not been connected yet.

## Running the UI milestone

Install workspace dependencies with `just install-js` on a first checkout or
after lockfile changes. Use `just desktop-dev --web-ui` to build and run the
packaged debug app with the React interface. `just desktop-dev` retains the
existing native interface as the default.

`just desktop-build` produces the release bundle. Launch its React interface
with:

```sh
target/release/bundle/macos/mew.app/Contents/MacOS/mew-desktop --web-ui
```

The [desktop recipes](../justfile) build the TypeScript client, then the React UI,
before compiling the Rust host. The [app entry point](../apps/mew-desktop/src/main.rs)
selects the interface through `--web-ui`. The
[embedded asset server](../crates/mew-web-bridge/src/lib.rs) compiles
`mew-web-ui/dist` into the binary; the
[packaging script](../scripts/package-desktop-native.sh) copies that binary and
its native runtime resources into the app bundle. These paths require no Vite
server or separate loose UI asset installation.

## Host, transport and lifecycle

| Feature and owner | Existing evidence inspected | Acceptance still required |
| --- | --- | --- |
| Packaged startup, local daemon attachment/start and ownership. Host: [supervisor](../crates/mew-desktop-supervisor/src/lib.rs), [desktop lifecycle](../apps/mew-desktop/src/shell/lifecycle.rs), [packaging](../scripts/package-desktop-native.sh). | Supervisor tests cover protocol probes, attach-only URLs, reuse, startup cancellation and owned-child shutdown. | [ ] Launch the installed bundle with no dev server; attach to an existing daemon or launch one; show recoverable startup errors; quit reaps owned processes and leaves an independently owned daemon alive. |
| Trusted app document and external pages. Host: [CEF app](../native/cef-host/src/app.rs), [embedding](../native/cef-host/src/embed.rs), [browser host](../crates/mew-browser-host/src/lib.rs). | Retained external pages already use CEF. This does not establish trusted React app binding or sibling-view composition. | [ ] App and external page coexist in one window/runtime; external pages cannot access the app binding or navigate over the privileged app document; menus, dialogs and clipping remain correct over browser content. |
| Version negotiation, connection state and reconnect. Client: [TypeScript client](../mew-web-client/src/index.ts), [Rust engine](../crates/mew-client-core/src/engine.rs), [web host](../mew-web-ui/src/lib/host.ts). | [Client tests](../mew-web-client/src/__tests__/client.test.ts) cover reconnect and ping protocol data; Rust engine rejects incompatible revisions; [host tests](../mew-web-ui/src/__tests__/host.test.ts) cover browser bootstrap. | [ ] Desktop socket binding and browser bridge use the required revision, expose disconnect/incompatibility, recover without mixing sessions and retain recoverable drafts. |
| Saved local/remote connection profiles. Host: [native preferences](../apps/mew-desktop/src/shell/preferences.rs), [local transport](../crates/mew-client-local/src/lib.rs), [iroh transport](../crates/mew-client-iroh/src/lib.rs), [config](../crates/mew-config/src/lib.rs). | Existing transports and profile switching; [iroh integration tests](../crates/mew-daemon/tests/iroh.rs) exercise the remote boundary. | [ ] Import and select saved profiles, reconnect to the correct host, isolate identical session IDs on different profiles and retain scope restrictions; remote profiles never register local browser pages. |
| App menus, shortcuts, file dialogs and clipboard. Host: [platform operations](../apps/mew-desktop/src/shell/platform.rs), [main](../apps/mew-desktop/src/main.rs), [CEF native key tests](../native/cef-host/tests/native_key_equivalents.rs). | Native shell has platform operations and shortcut coverage in [shell tests](../apps/mew-desktop/src/shell/tests.rs). | [ ] Native open/save/attachment dialogs, copy/paste/select-all, undo/redo, pane/tab shortcuts and quit work with focus in app text, browser content and dialogs; renderer shortcuts do not steal native editing. |
| Responsiveness and resource lifecycle. Presentation and host. | [Native shell tests](../apps/mew-desktop/src/shell/tests.rs) cover bounded panes, streaming coalescing and scroll stability; retained-page tests cover hidden-page ownership. | [ ] Measure idle, streaming while typing, resize and repeated tab open/close in the package, including helpers; no lingering owned pages/processes or increasing resource use after disposal; inspect keyboard access and accessibility of the DOM shell. |

## Navigation, chat identity and persisted view state

| Feature and owner | Existing evidence inspected | Acceptance still required |
| --- | --- | --- |
| New/resume chat, title/search, pin and archive discovery. Daemon/session metadata plus [native sidebar](../apps/mew-desktop/src/shell/sidebar.rs), [web session rail](../mew-web-ui/src/components/session-rail.tsx). | [Shell tests](../apps/mew-desktop/src/shell/tests.rs) cover inherited workspace, pin actions, search and archived restoration; [web search tests](../mew-web-ui/src/__tests__/session-search.test.ts) and [rail tests](../mew-web-ui/src/__tests__/session-rail.test.ts) cover title/workspace grouping. | [ ] Notifications-style navigation opens the actual target chat, including while another chat streams; search/rename/pin/restore preserve selection and full titles; new chat uses the selected workspace. |
| Optional projects/groups and filters. Daemon: [projects](../crates/mew-daemon/src/projects.rs), [groups](../crates/mew-daemon/src/groups.rs); session metadata. | [Daemon e2e](../crates/mew-daemon/tests/e2e.rs) includes project lists and persisted new-session group membership; native sidebar renders groups/workspaces. | [ ] Create/select/manage existing organization and filters without making them prerequisites for an ordinary chat; group/project changes survive reload and target the intended chat. |
| Inbox, unread completion and background attention. Presentation: [attention helpers](../mew-web-ui/src/lib/attention.ts), [session store](../mew-web-ui/src/stores/session.ts). | [Attention tests](../mew-web-ui/src/__tests__/attention.test.ts), [store tests](../mew-web-ui/src/__tests__/store.test.ts), and native `sidebar_inbox_is_available_without_search_and_tracks_unread_and_requests`. | [ ] Pending input and unread work remain discoverable outside the selected chat; opening/dismissing an item affects its exact owner and does not clear unrelated attention. |
| Per-profile/chat drafts and attachment ownership. UI: [native drafts](../apps/mew-desktop/src/shell/drafts.rs), replacement workspace state. | Draft tests cover navigation, separate profiles, late new-chat acknowledgements and history recall. The current web input tests cover attachments, not equivalent cross-chat draft ownership. | [ ] Switch chats/profiles during typing, attachment reads and session creation; restore each original draft and attachments; a late acknowledgement never binds a different draft. |
| Pane/tab/scroll preferences and import. UI/config: [native work tabs](../apps/mew-desktop/src/shell/work_tabs.rs), [config](../crates/mew-config/src/lib.rs), [web surface state](../mew-web-ui/src/lib/workspace-surfaces.ts). | [Surface tests](../mew-web-ui/src/__tests__/workspace-surfaces.test.ts) cover width clamps and malformed preferences; native tabs persist order/selection. Existing workspace-wide web tabs need chat ownership. | [ ] Import applicable original preferences without overwriting original data; version new view storage; restore tabs, pane width and logical scroll anchors to their profile/chat, including malformed or missing state. |
| Stop, close/archive and restore. Daemon execution/session lifecycle; UI close action. | [Session lifetime tests](../crates/mew-daemon/tests/session_lifetime.rs) establish that navigation/disconnection preserve work and explicit cancellation ends it. Current archive metadata alone does not implement stop-and-archive. | [ ] Navigation and pane/tab closure preserve ongoing chat work; closing a chat acknowledges stopping its descendants before archiving; restored history is idle until an explicit follow-up; child tab closure alone does not stop that child. |

## Conversation and composer

| Feature and owner | Existing evidence inspected | Acceptance still required |
| --- | --- | --- |
| History, streamed text, reasoning, tool states and compaction. UI: [chat surface](../mew-web-ui/src/components/chat-surface.tsx), [message item](../mew-web-ui/src/components/message-item.tsx), [session store](../mew-web-ui/src/stores/session.ts); daemon canonical history. | [Message tests](../mew-web-ui/src/__tests__/message-item.test.tsx), store tests and [Rust UI model](../crates/mew-ui-model/src/lib.rs) cover projection; [daemon e2e](../crates/mew-daemon/tests/e2e.rs) covers stream events. | [ ] Real history and live turns preserve part order, failure causes, tool disclosure and compaction boundaries without duplicate text or unrelated-chat updates; completed tool details remain inspectable. |
| Markdown/code/tables, links, selection and copying. UI: [markdown](../mew-web-ui/src/components/markdown-body.tsx), [code blocks](../mew-web-ui/src/components/code-block.tsx), [inspector](../mew-web-ui/src/components/message-inspector.tsx). | Native shell tests cover full code/tool copying, Unicode selection, offscreen selections, table alignment and reading-order copy; [inspector tests](../mew-web-ui/src/__tests__/message-inspector.test.tsx) cover details. | [ ] Selection and copy preserve exact Unicode, newlines and complete code/tool content; wide code/tables scroll inside their surface; links open the intended document/browser tab beside chat. |
| History paging, scroll anchors and streaming follow. UI: [virtual chat surface](../mew-web-ui/src/components/virtual-chat-surface.tsx), client history cursor. | Client history-page tests and native tests cover prepend anchors, offscreen completion and turn spacing. | [ ] Prepend older history without moving the visible passage; keep reading position while tools or streaming update elsewhere; follow active output only when appropriate; restore per-chat anchors. |
| Model/provider, persona, effort and permissions. Daemon policy; UI composer controls. | [Policy tests](../crates/mew-daemon/src/policy_tests.rs) cover persistence, failed writes and concurrent updates; [persona pill tests](../mew-web-ui/src/__tests__/persona-pill.test.tsx) and client tests cover confirmed selection. | [ ] Catalog labels/search and allowed effort options use actual identities; apply only confirmed policy; rejected changes preserve current policy; restore effective settings and persona accents after reload. |
| Prompt submission, receipts and retry. Client/daemon: [wire protocol](../crates/mew-protocol/src/lib.rs), [request tests](../crates/mew-daemon/src/request_tests.rs), Rust client core reference. | [Receipt wire tests](../crates/mew-protocol/tests/prompt_receipts.rs) and daemon tests cover owner scope, duplicates and reconnect. Existing TS `prompt` still uses the legacy command. | [ ] Keep text/attachments recoverable until acceptance; correlate session/request/connection; reconnect reconciles the original receipt; a busy/rejected send remains recoverable; an unknown receipt after daemon restart stays visibly uncertain rather than automatically resubmitting. |
| Attachments: chooser, paste, drop, MIME and removal. UI: [input area](../mew-web-ui/src/components/input-area.tsx); host file/clipboard operations. | [Input tests](../mew-web-ui/src/__tests__/input-area.test.tsx) cover data URLs, pasted/dropped files and double-submit prevention; native tests cover conservative MIME and image names. | [ ] Real files/images survive async reads and navigation; removing an attachment prevents its submission; failed reads are visible; the accepted prompt owns exactly the intended attachments. |
| Mentions, slash commands, prompt history and running-turn guidance. UI composer plus daemon/client commands. | Native shell tests cover mention boundaries, file matching, slash routing and prompt-history stash; client tests cover slash commands and `guide`. | [ ] Keyboard/pointer pickers insert correct context; full commands reach the daemon; guidance targets the active intended turn; ordinary text is not misinterpreted; history recall restores the original draft. |
| Native editing, focus and IME. DOM composer plus native host. | Native tests cover UTF-8 boundaries and draft undo grouping; web input tests do not establish packaged IME correctness. | [ ] Compose CJK text without premature send, use dead keys/emoji, selection replacement, undo/redo, multiline paste and Enter/Shift+Enter; focus moves between chat, menus, address field and external browser without losing edits. |

## Required actions, progress and child conversations

| Feature and owner | Existing evidence inspected | Acceptance still required |
| --- | --- | --- |
| Permissions and user questions. Daemon request identities; [permission UI](../mew-web-ui/src/components/permission-toast.tsx), [question UI](../mew-web-ui/src/components/ask-user-card.tsx), session store. | [Child question wire tests](../crates/mew-protocol/tests/child_questions.rs), [client child-question tests](../crates/mew-client-core/tests/child_questions.rs) and web store tests cover source identity, replay and exact resolution. | [ ] Respond in a background/child chat after navigation/reconnect; resolve only that request; retain rejected/unsent answers and show confirmed completion; stale callbacks cannot answer another request. |
| Goal proposals and active goal state. Daemon/client plus [goal card](../mew-web-ui/src/components/goal-card.tsx). | Client tests accept/reject by request ID; [goal-card tests](../mew-web-ui/src/__tests__/goal-card.test.tsx) retain a decision when no client can send it. | [ ] Accept/reject with original ownership, expose current goal state, recover pending proposals after reconnect and avoid optimistic disappearance before delivery is possible. |
| Todos and edits. Daemon/client plus [todo panel](../mew-web-ui/src/components/todo-panel.tsx). | [Todo integration tests](../crates/mew-daemon/tests/todo_edits.rs) cover acknowledgement, persistence and live edits; client tests cover timeout, disconnect and unrelated errors. | [ ] Show authoritative todos, send edits with session/request identity, retain unconfirmed edits visibly, and restore persisted state without silently swallowing unsupported commands. |
| Background jobs and activity/output reads. Daemon [activity](../crates/mew-daemon/src/activity.rs), client/store and activity surface. | [Activity parity](../crates/mew-client-core/tests/activity_parity.rs), [job output tests](../crates/mew-client-core/tests/job_output.rs), [attach activity](../crates/mew-daemon/tests/attach_activity.rs), web client/store tests. | [ ] Running/exited job state and bounded output chunks retain task, request and byte offsets after reconnect; no output leaks between jobs/chats; job logs remain distinct from interactive PTY state. |
| Grouped subagents, progress, nested history and source questions. Runtime/controller, daemon, right-pane child view. | [Child history integration](../crates/mew-daemon/tests/subagent_history.rs) reads owned nested transcripts without switching parent; client tests preserve viewer identity. | [ ] Expand grouped children without cluttering navigation; open independent child history/status beside parent, track exact child/run and route required actions without replacing main history. |
| Direct child messaging and follow-up runs, required for v1. [Runner](../crates/mew-agent/src/runner.rs), [registry](../crates/mew-agent/src/subagent_registry.rs), [parent session](../crates/mew-daemon/src/session.rs). | Existing one-shot runner and history reads do not establish resumable direct conversations. The migration plan specifies controller/receipt work. | [ ] Queue guidance to a running child and follow up in the same finished/stopped child conversation; preserve effective policy/guardrails, parent ownership and receipts; test completion races, duplicate sends and parent/child stop; legacy children with insufficient resume metadata show a specific limitation. Outside the current UI milestone. |

## Tabbed right pane and tools

| Feature and owner | Existing evidence inspected | Acceptance still required |
| --- | --- | --- |
| New-tab launcher, one strip, tab selection/close and pane resize/collapse. UI workspace state and host page bounds. | [Native work-tab tests](../apps/mew-desktop/src/shell/work_tabs.rs) cover independent browser IDs, launcher replacement, neighbor selection, shortcuts and retained-input folding. [Web tab tests](../mew-web-ui/src/__tests__/workbench-tabs.test.ts) cover older fixed/core tabs, which need replacement. | [ ] Tool/address launcher opens or selects the correct tab alongside chat; each tab is closable; selected tab and + remain reachable with many tabs; collapse retains state and disables hidden native input; resize keeps chat usable and restores width. |
| Browser navigation and load state. Host retained external CEF page plus UI toolbar. | [Native shell tests](../apps/mew-desktop/src/shell/tests.rs) cover URL normalization and recoverable errors; [web browser lifecycle tests](../mew-web-ui/src/__tests__/browser-lifecycle.test.ts) reject stale tab events. | [ ] Enter a URL/search, navigate back/forward/reload, display title/loading/failure, use real page input and preserve address edits when background metadata arrives; closing one tab leaves others live. |
| Exact browser tool target and retained lifecycle. [Browser host](../crates/mew-browser-host/src/lib.rs), [desktop browser integration](../apps/mew-desktop/src/shell/browser.rs), [daemon registration](../crates/mew-daemon/src/browser.rs). | [Host tests](../crates/mew-browser-host/src/tests.rs) cover profile/session/tab/page-generation identity, hiding, recreation and stale callbacks; [daemon browser tests](../crates/mew-daemon/src/browser_tests.rs) cover local authorization and revoke on disconnect/delete. | [ ] Register only the intended external page with its ready CDP endpoint/target; switching browser tabs revokes the prior target in order; hidden pages retain identity; late callbacks/replies cannot revive closed/replaced pages; chat/profile switches never retarget a command to another page. |
| Files, directory/watch, preview and external editor. [Daemon files](../crates/mew-daemon/src/files.rs), [file workbench](../mew-web-ui/src/components/file-workbench.tsx), [file tree](../mew-web-ui/src/components/file-tree.tsx). | [File workbench tests](../mew-web-ui/src/__tests__/file-workbench.test.tsx) and [tree tests](../mew-web-ui/src/__tests__/file-tree.test.ts) cover existing previews/tree behavior. | [ ] Open independent file tabs in the selected workspace, retain read-only behavior, show actual reload/errors and open external editor where supported; watcher updates do not overwrite another file/chat. |
| Changes/diff and workspace refresh. [Daemon workspace](../crates/mew-daemon/src/workspace.rs), [native review](../apps/mew-desktop/src/shell/review.rs), protocol `ReadSessionDiff`. | [Review tests](../crates/mew-daemon/src/review_tests.rs) cover selected workspace, same-count edits, HEAD/index/worktree invalidation and identified errors. | [ ] Expose TS command and render actual requested-session diffs beside chat; distinguish repository/read failure from no changes; refresh external changes without stale response replacement. |
| Interactive PTY, terminal fonts/colors/input/selection. [Daemon terminal](../crates/mew-daemon/src/terminal.rs), [native terminal](../apps/mew-desktop/src/terminal/mod.rs), replacement web terminal. | Daemon terminal unit tests and `desktop_terminal_is_owned_by_daemon_and_streams_bytes` in [daemon e2e](../crates/mew-daemon/tests/e2e.rs) exercise real PTYs. | [ ] Real shell I/O, ANSI/Unicode, resize, selection/copy, interrupts, font/theme updates and exit/disconnect states; one PTY per connection; tab close disposes owned shell. Restored tab metadata must not imply shell reattachment after reconnect/app exit. |
| Ordinary side chats, child tabs and artifacts. UI/right-pane documents and daemon history. | Mockup demonstrates the composition; existing history/child-history and file readers supply partial foundations. No mockup echo proves an independent side conversation. | [ ] Main and side chat retain independent history/drafts/requests; artifacts open their actual content with source identity; child tabs keep parent relationship and explicit stop semantics. Beyond the browser/chat host proof. |

## Themes, settings and compatibility

| Feature and owner | Existing evidence inspected | Acceptance still required |
| --- | --- | --- |
| Built-in themes and semantic tokens. [Manifest](../crates/mew-tui/resources/theme_manifest.json), [Rust theme loader](../crates/mew-tui/src/theme.rs), [theme guide](../docs/using-mew/theming.md). | Manifest is authoritative for 25 built-ins and generated assets; loader tests cover built-ins, aliases, persona accent and fallback. Generated web CSS already exists. | [ ] Inventory stays synchronized; shadcn/Tailwind chrome, menus, transcript, markdown, code, tool output, selection and terminal use shared semantic tokens; no copied mockup palette; run theme codegen/check when sources change. |
| Installed custom themes, inheritance/aliases and validation. Rust loader/CSS export; native/browser host supplies inventory and resolved data. | Loader tests cover custom overrides, alias cycles and invalid colors. Existing [web theme picker](../mew-web-ui/src/lib/theme.tsx) is built-in-only and infers mode from swatches. | [ ] Resolve installed JSON through the existing loader with explicit project context; reject invalid/unknown/cyclic tokens visibly; validate inherited/aliased fixtures; expose matching inventory and explicit mode in desktop/browser without hardcoded filesystem paths. |
| System appearance, independent light/dark palettes and live updates. Host appearance plus [native preferences](../apps/mew-desktop/src/shell/preferences.rs). | Native `desktop_theme_mode_selects_the_configured_variant` and cached-code color refresh tests establish references. | [ ] Import saved mode/light/dark choices, follow system changes live, keep explicit mode stable and refresh already-rendered code and idle terminal colors immediately. |
| Font and layout preferences. [Native settings](../apps/mew-desktop/src/shell/settings.rs), [web settings](../mew-web-ui/src/routes/settings.tsx), config/view state. | Native terminal font picker, workspace toggles and browser font choices exist; state tests cover roundtrips. | [ ] Preserve applicable selected fonts, pane visibility/width and recent models through restart/import; missing/invalid preferences have defined usable fallbacks. |
| Effective chat policy and new-chat defaults. Daemon policy/defaults commands. | [Policy tests](../crates/mew-daemon/src/policy_tests.rs) cover persistence, failure rollback, new defaults, unchanged existing chats and authority checks. | [ ] Settings show confirmed policy/defaults, capture/reset defaults for the intended chat, preserve old chats and report failed writes without presenting uncommitted values as saved. |
| Title/summary preferences and session management. Protocol/session metadata plus settings. | Protocol provides confirmed preferences; web settings currently keep local title/summary choices; [daemon e2e](../crates/mew-daemon/tests/e2e.rs) covers title generation. | [ ] Preserve the correct preference authority and confirmation, refresh title/summary metadata across viewers and expose existing session-management actions with their actual effects; do not substitute local success for daemon changes. |
| Authorized devices, revocation and remote scope. Daemon [remote](../crates/mew-daemon/src/remote.rs), protocol settings/devices commands. | Policy tests cover full-control requirements, revocation, self-revocation confirmation and scope downgrades during in-flight requests. | [ ] Show authorized devices only to authorized clients; revocation targets the selected device, confirms before self-disconnect and prevents stale commands from disclosing/changing settings. |
| Existing session/config compatibility and unaffected clients. Shared protocol/config/session layers. | Rust/TUI/mobile and old native UI consume the shared daemon; original history and policy formats remain authoritative. | [ ] Migration opens original sessions/settings without destructive conversion; UI storage is separate/versioned; changed protocol negotiates explicitly; TUI/mobile keep their supported behavior. |

## Verification record

2026-10-27: inspected the migration plan, owning source files and the referenced
test definitions to establish this inventory. No application behavior tests,
provider calls or packaged native QA were run while writing this document.
Acceptance boxes above intentionally remain open. Record subsequent verification
with the exact command or native scenario, result and relevant limitation before
checking a case off.

For this milestone, run the changed TS/UI tests and builds, relevant host and
daemon tests, generated-theme checks, and a packaged app smoke test with an
isolated data/config directory. The smoke record must distinguish a real local
daemon turn from a mock echo and an actual retained external CEF page from a
placeholder or iframe. Before the full replacement becomes default, complete
the remaining baseline and repository gates specified by the migration plan.

Recipe verification: `just --dry-run desktop-dev --web-ui`,
`just --dry-run desktop-dev` and `just --dry-run desktop-build` passed. Each
resolves the client build before the UI build and Cargo commands; debug arguments
use the repository's existing positional-argument forwarding convention. This
checks recipe parsing and command order, not a desktop build or packaged smoke.
