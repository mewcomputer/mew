# Chat UI migration plan

Date: 2026-10-27
Status: Electron UI milestone implemented; full migration acceptance pending

Move the desktop presentation to the packaged Chromium runtime and rebuild the
web interface around the approved chat mockup. Preserve the Rust daemon, agent
runtime, providers, tools, session storage and theme system. Direct messaging to
subagents is required in the first version and includes runtime work.

The first implementation milestone is a packaged app running the new chat shell
beside a real browser page. That establishes whether the host composition works
before investing in the rest of the presentation.

## Product scope

- The default interface is the familiar chat layout. The notifications display
  uses the compact chat list from the mockup. Preserve project/group navigation
  and existing filters as optional ways to organize the same chats.
- The conversation and composer remain visible beside a resizable right pane.
  The pane can collapse, expand and restore its previous width.
- The right pane contains document tabs. Its new-tab launcher offers tools,
  artifacts and an address/search field. There is one tab strip, including for
  individual files and browser pages.
- Keep subagents grouped under their parent chat. Opening one shows its own
  conversation and composer in the right pane. Ordinary side chats may also
  open there without being represented as delegated agents.
- Tab selection and view state belong to a connection profile and chat.
  Collapsing the pane retains its tabs. Navigation leaves running work alone.
- Closing a chat stops its work and descendants, then archives it. Restoring
  makes history available without automatically resuming work. Closing a
  subagent conversation tab only closes its view; stopping that child is an
  explicit action. Closing a terminal or browser tab disposes its owned resource.
- Preserve built-in and installed themes, system appearance, the selected light
  and dark palettes, semantic colors and applicable font preferences.

The mockup establishes layout and interaction direction. Its sample terminal,
browser pages and local message echoes must be replaced by real capabilities.
Canvas layouts and topic lanes are outside this migration.

## Current host decision

The browser composition comparison and packaged UI checks selected Electron for
the opt-in desktop shell. `apps/mew-electron` owns the window, menus and retained
DOM guests; `mew desktop-host` reuses the Rust supervisor, embedded assets and
theme loader. The shared React presentation connects to the local daemon through
`MewClient`. Native GPUI/CEF remains available as the default during migration.
See the [Electron implementation and verification record](../apps/mew-electron/README.md)
for build commands, trust boundaries and keyboard-focus preflight. Browser
automation now requires fresh session confirmation and the daemon's advertised
focus capability. The first UI milestone is complete; the baseline release
requirements below remain in force, including direct subagent messaging.

## Initial CEF architecture (superseded host choice)

The following describes the initial hosting plan. The Electron decision above
supersedes its GPUI/CEF ownership and transport-binding choices; the shared
presentation, runtime boundaries and baseline requirements still apply.

Keep React, TypeScript and Vite as the presentation toolkit. Replace the current
screen composition and revise state ownership within `mew-web-ui`; retain
useful accessible controls, tested pure helpers and existing build tooling.
This retires the current interface without introducing another frontend package
or requiring a framework migration at the same time.

```text
shared React presentation and client state
  browser: MewClient -> existing web bridge -> daemon
  desktop: MewClient -> native transport binding -> local/iroh transport -> daemon

thin Rust desktop host
  native window, menus, appearance and file dialogs
  daemon supervisor and connection profiles
  trusted app page and retained external CEF pages
  native preferences and installed theme resolution

daemon -> session -> agent -> provider/tools
```

Use the existing `MewClient` socket-factory boundary for desktop transport.
Reuse the local and iroh Rust transport implementations below that binding.
The TypeScript client owns the web/desktop protocol projection and reconnect
workflow. Avoid running a second `ClientEngine` projection for the same desktop
conversation. The Rust client core continues serving its existing clients;
its receipt/reconnect scenarios provide behavioral references for the new UI.

Initially retain GPUI as the native window owner while CEF renders the app
interface. Move remaining responsibilities out of `DesktopShell` into the
existing supervisor, browser host and narrow native binding as they are needed.
Whether the final window owner still uses GPUI is decided after the packaged
host milestone; it must not create a second set of application views.

CEF supports a native shell around a web interface and asynchronous renderer
bindings. Those capabilities do not prove that this repository's embedded-view
composition handles focus, overlays and shutdown correctly. Verify those in the
first milestone. See the [CEF integration documentation](https://chromiumembedded.github.io/cef/general_usage).

### Ownership and trust

The daemon remains authoritative for execution, permissions, session metadata,
policy and subagent lifecycle. The UI owns drafts, selection, expansion,
scroll anchors and pane layout. The native host owns platform resources.

The trusted app page receives only the native operations actually consumed by
the interface. External browser pages receive no app binding. Keep them in
separate page identities and request contexts from the app page, and prevent
external navigation from replacing the privileged app document.

Preserve the existing browser owner tuple: connection profile, parent session,
tab and page instance. Register only the intended external page with the daemon,
with its exact CDP endpoint and target. Retain registration/revocation ordering,
stale-callback rejection and local-only browser registration.

Use one CEF runtime with an app page per window and pages for browser tabs.
Reuse the current retained-page integration before considering a different
rendering path. The app page and an external page must coexist correctly under
menus, dialogs, clipping and resize. A plain iframe cannot substitute for the
existing browser capability.

## Existing capabilities and required changes

| Area | Existing foundation | Migration requirement |
| --- | --- | --- |
| Native startup | `mew-desktop-supervisor`, local WebSocket and iroh transports | Reuse attach/start/ownership behavior and saved remote profiles through the host. |
| Client reliability | Rust client core has scoped prompt receipts; TS client still sends legacy `prompt` | Add revision negotiation, submission receipts, reconciliation and attachment ownership to the TS path before replacing daily use. |
| Chat navigation | Session metadata, groups, project lists, pin/archive and history APIs | Rebuild notifications navigation and preserve other organization controls. Keep background attention visible. |
| Conversation | Existing markdown, tool state, reasoning, compaction, history paging and inspectors | Rebuild the transcript with selection, copying, links, stable paging anchors and streaming updates. |
| Composer | Model/persona/effort/permission policy, attachments, mentions, commands and guidance | Preserve the complete interaction inventory, including keyboard editing, Unicode/IME and per-chat drafts. |
| Required actions | Request IDs, question sources, permission decisions, goal proposals and resolved events | Route every response to its original owner; show pending and resolved state across navigation/reconnect. |
| Progress and jobs | Todos and edits, goal state, subagent progress, background jobs and output reads | Preserve their controls and inspectable output in the summary/activity surfaces without treating job logs as a PTY. |
| Files and changes | Directory/preview/watch APIs and Rust `ReadSessionDiff` | Expose missing TS messages and use independent file/diff tabs. Existing file previews remain read-only with external-editor access. |
| Terminal | Daemon already provides PTY input/output/resize/close; native renderer uses Ghostty | Add TS PTY support and a real web terminal surface. Preserve native behavior through tests against actual PTYs. |
| Browser | Retained native CEF pages with exact target registration | Connect DOM pane bounds and focus to the native host; preserve page identity when changing chats and tools. |
| Subagents | Nested transcripts, progress, question routing and cancellation | Add live child conversation subscriptions, direct messages and follow-up runs. Existing history reads alone are insufficient. |
| Themes and settings | Shared manifest, Rust loader/CSS export, installed themes and native preferences | Reuse resolution and storage; add a theme projection for desktop and browser. Replace the built-in-only web picker behavior. |
| Session and device settings | New-chat defaults, title/summary preferences and authorized devices | Preserve confirmed policy/default changes and device revocation under their existing authority checks. |
| Close and archive | `ArchiveSession` currently changes metadata | Implement acknowledged stop-and-archive behavior that includes descendants, including a child running after its parent's turn ended. |

The terminal protocol currently owns one shell per daemon connection. Persisting
a terminal tab does not mean the shell survives reconnect or app exit. Preserve
that distinction and expose exited/disconnected states. Start with one terminal
per chat connection; any retained connections must be tied to live terminal
resources and disposed explicitly. Do not require multiple PTYs on one connection
or durable terminal reattachment to complete UI parity.

## State and compatibility

Use the existing packages and boundaries:

- `mew-web-client/src/index.ts`: wire types, correlated commands and events,
  capability negotiation and connection lifecycle. Keep presentation out.
- `mew-web-ui/src/stores/session.ts`: daemon projection keyed by confirmed
  connection/session identity. Replace the single selected-session assumptions
  where they would mix parent, child or background chat data.
- A workspace state module under `mew-web-ui/src/stores`: drafts and view state
  shared by navigation, conversation and workbench. Key unsent new-chat drafts
  separately until session creation is confirmed.
- Rebuild the existing workspace frame, session rail, chat surface, input area
  and right rail in their current component layer. Add launcher and subagent
  conversation components where those surfaces become real.
- `apps/mew-desktop`, `native/cef-host` and `mew-browser-host`: native hosting
  and platform resources. Keep agent execution in the daemon/runtime crates.

Every async operation needs its actual owner: connection generation, session,
request and, where applicable, tab/page or child/run. Reuse existing identities.
Extend wire messages only where their current shape cannot distinguish replies.
Do not retarget queued commands when the selected chat changes.

Import desktop theme preferences, recent models, saved profiles, pane state,
work tabs and logical transcript anchors through `mew-config`. Import browser
local preferences in the browser context. Keep versioned new UI view storage
separate from authoritative session history, and leave the original persisted
view data available during comparison. Preserve common preference fields
through the existing config API rather than duplicating path resolution.

Prompt text and attachments remain recoverable until acceptance. Reconnect
reconciles the original request ID before retrying. An unknown receipt after a
daemon restart must remain visibly uncertain; automatic resubmission must not
invent an exactly-once guarantee that the daemon does not provide.

## Theme contract

`crates/mew-tui/resources/theme_manifest.json` remains the source of truth. Keep
the current built-in palettes, inheritance, aliases and installed JSON format.
Use the existing Rust theme loader and CSS export for custom theme resolution;
do not copy the mockup's embedded palettes into application code.

The native binding supplies resolved theme data and appearance changes. The
browser bridge supplies the equivalent installed-theme inventory and resolved
data from its host environment. Browser and desktop use the same application
mapping, with existing preference ownership preserved. Project-local theme
lookup must use an explicit context rather than accidentally following a helper
process's working directory.

Apply semantic tokens to chrome, menus, chat, markdown, tool output, syntax,
selection and terminal. Preserve persona accents. Use explicit theme mode for
appearance rather than inferring it from preview swatches. Update already
rendered code and idle terminals immediately when appearance changes.

Keep theme changes in the manifest and run `just theme-codegen`; never manually
edit generated CSS, Rust tables or TextMate output. Add fixture coverage for a
custom theme, inheritance/aliases, invalid input, missing preferences and live
system light/dark changes. Built-in theme inventory and generated metadata must
stay in sync. See the [shared theme guide](../docs/using-mew/theming.md).

## Direct subagent conversations

This is a release requirement, with lifecycle work ahead of its UI. The current
runner creates a fresh child per invocation and reports a one-shot result.
Outstanding-task persistence recovers orphaned results; it does not currently
reattach a live child after a daemon restart.

Add a parent-owned child conversation controller around the existing runner and
agent execution. Keep the child session identity across follow-up runs and
persist the effective definition/policy needed to reopen its nested transcript.
Active execution may remain in memory; completed children should be reopened
on demand rather than retaining every agent instance indefinitely.

Preserve the runner's tool restrictions, hooks, schema checks and orchestration
limits when continuing a child. Persist the information required to reproduce
its effective policy; a human follow-up must not reset a concurrency or depth
guardrail by being presented as an unrelated top-level chat.

Proposed messaging behavior:

- A message to a running child is acknowledged and queued for its next provider
  request, using the existing guidance mechanism. If that run ends first, the
  controller starts the follow-up run so an accepted message is not stranded.
- A message to a finished or stopped child starts a new serialized run in that
  same child conversation. Preserve its effective model, tool restrictions,
  workspace and permissions; use a fresh run cancellation token.
- Identify commands and receipts by parent session, child session and request.
  Identify lifecycle/result events by child and run so a late completion cannot
  end a newer follow-up.
- Keep already-collected parent tool results final. A new child run creates a
  new outcome and informs the parent that its delegated work changed. Define
  how `subagent_wait` observes that run without replacing the prior result.
- Route child permissions/questions through the existing parent ownership
  chain. Human messages must not broaden the child's capabilities.
- Parent stop/archive reaches all live descendant controllers, including when
  only a child is running. An explicit child stop leaves siblings running.
  Closing the child tab does not cancel it.

Expose the commands and events through protocol, daemon, Rust client and TS
client together. Reject unrelated child IDs, stale runs and insufficient remote
scope. Preserve drafts on rejected/uncertain submissions. Test delivery at the
boundary where a child completes, double submission, parent collection races,
parent cancellation and nested child ownership.

Persist resume metadata for newly created children and define migration for
older children. When a legacy transcript lacks enough policy information for a
safe resume, show that specific limitation and retain its history. Do not
silently create an unrelated agent or guess a more permissive policy.

## Delivery sequence

### 1 Establish the acceptance baseline

Create the feature checklist from the matrix above, including settings and
native shortcuts not shown in the mockup. Capture representative existing
conversations and native interactions in isolated data directories. Write the
first failure cases before changing their implementations.

Exit: every baseline capability has an owning layer and an acceptance scenario;
prototype-only behavior is explicitly identified. Record which cases already
pass instead of treating all of the web UI as unusable.

### 2 Prove the packaged host

Boot a trusted app page in the current native window with the daemon supervisor,
native binding and one external CEF page beside a minimal working chat. Exercise
file dialogs, clipboard, native editing, IME, appearance and remote transport.
Keep this implementation on the eventual path to the product.

Measure the existing software-rendering default against the available GPU path
for the full app surface. Choose the host configuration from that packaged test,
rather than assuming the browser portal's current defaults fit the new workload.

Exit: the packaged app can send/receive a real daemon turn, resize the browser
pane, open a menu over it, switch focus, close one page and quit cleanly. Agent
browser tools must target the visible external page. If the existing native
embedding fails these cases, resolve the host composition before expanding the
interface or committing to removal of GPUI as the window owner.

### 3 Close protocol gaps and implement child messaging

Bring the TS client up to the required revision and expose receipts, terminal,
review, project and browser registration messages. Add Rust-produced JSON
fixtures consumed by TS tests so drift is detected. Implement the subagent
conversation controller, message receipts, subscriptions and resume metadata.
Implement stop-and-archive at the daemon boundary.

Exit: real daemon/client tests pass for reconnect, cross-chat routing, required
actions, PTY I/O, direct child messages and parent/child cancellation. Display
protocol incompatibility explicitly. Existing TUI/mobile paths remain compatible
or negotiate the required revision; no commands are silently swallowed.

### 4 Build the chat interface and theme integration

Rebuild the notifications navigation, conversation, composer and context
summary around the agreed layout. Restore project/group controls without
requiring them for ordinary chats. Connect the shared theme projection and
native preference import before adding isolated color styles.

Exit: real history, streaming, selections, paged scroll anchors, drafts,
attachments, policy changes and required actions work through navigation and
reconnect. All built-in themes and installed custom themes cover the same
surfaces. Composer keyboard/IME behavior has native QA as well as browser tests.

### 5 Complete the tabbed workbench

Implement the new-tab launcher, pane sizing, independent documents, changes,
activity, browser pages and ordinary/subagent side conversations. Reuse the
existing file and diff readers. Keep files read-only until editing and conflict
handling are separately scoped.

Use a maintained web terminal such as [xterm.js](https://xtermjs.org/docs/) over
the existing PTY protocol, subject to the terminal acceptance cases. Preserve
fonts, selection, input, resize, Unicode, ANSI colors, interrupts and exit state;
background job output remains a separate surface. Introduce the terminal
dependency only with its real consumer.

Exit: each surface operates beside the main chat, restores to its correct owner,
and disposes resources correctly. Browser pages keep their identity while hidden.
Child follow-ups stream into their own conversation and update parent status.
Workbench tools are real capabilities in the packaged app.

### 6 Qualify daily use and switch the default

Exercise the complete desktop and browser flows, remote profiles and preference
imports. Keep the current GPUI app available for comparison until these checks
pass. The delivered architecture has one desktop/browser presentation; the old
application views are retired only after the replacement passes acceptance.

Exit: the migration checklist passes, original sessions/configuration remain
usable, packaged shutdown leaves no owned processes behind, and the new app
is suitable for daily use. CLI/TUI and mobile remain independent products.

## Verification and resource use

Use behavior tests at each changed boundary, then focused integration tests with
real local daemon connections, PTYs and filesystem data. A deterministic provider
is appropriate for repeatable UI/transport scenarios; it does not replace a real
provider smoke check where streaming/provider behavior is the subject of a test.

Run UI Vitest in CI and the full repository gate; the current `web-ci` job and
`just test-all` run client tests without the UI suite. Add browser acceptance
coverage and packaged macOS smoke coverage. Pure reducers cannot establish
native IME, accessibility, browser embedding or process cleanup correctness.

Check the changed Rust crates, TS client/UI tests and builds, architecture rules
and generated themes. Before switching the default, run `just ci`, the UI suite,
desktop build/package checks and native acceptance on supported macOS targets.

Measure one representative workload before and after: idle app, large transcript
streaming while typing, pane resize, several active chats, browser tabs and
repeated tab open/close. Include CEF helpers in memory/process measurements.
Keep completed transcript work stable, render visible history, avoid broadcasting
each token through unrelated views, and load heavy tools on demand. Native
platform tests remain necessary even though the main presentation uses the DOM.

Do not choose arbitrary RAM ceilings before measuring the working host. Investigate
resource growth, lingering pages/processes and input stalls as concrete failures.

## Source map

- [Native browser integration](../native/cef-host/README.md) and
  [retained page ownership](../crates/mew-browser-host/src/lib.rs).
- [Desktop supervisor](../crates/mew-desktop-supervisor/src/lib.rs),
  [local transport](../crates/mew-client-local/src/lib.rs) and
  [iroh transport](../crates/mew-client-iroh/src/lib.rs).
- [Wire protocol](../crates/mew-protocol/src/lib.rs),
  [TypeScript client](../mew-web-client/src/index.ts) and
  [Rust client engine](../crates/mew-client-core/src/engine.rs).
- [Web host bootstrap](../mew-web-ui/src/lib/host.ts),
  [workspace persistence](../mew-web-ui/src/lib/workspace-surfaces.ts) and
  [current theme picker](../mew-web-ui/src/lib/theme.tsx).
- [Native preferences](../apps/mew-desktop/src/shell/preferences.rs) and
  [stored desktop view state](../crates/mew-config/src/lib.rs).
- [Child runner](../crates/mew-agent/src/runner.rs),
  [task registry](../crates/mew-agent/src/subagent_registry.rs) and
  [parent session ownership](../crates/mew-daemon/src/session.rs).

Earlier workbench and GPUI plans describe useful implementation history. Their
fixed picker, workspace-wide tab storage and old browser-host assumptions do not
override the chat-scoped tab/launcher direction above.
