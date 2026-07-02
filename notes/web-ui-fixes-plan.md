# web ui + daemon fixes — handoff plan

Context: the misc-features work in CURRENT.md (cost surfacing, cross-session
alerts, flagged files, plus the phase 2/3 groups & file-service plumbing)
shipped with a number of bugs, mostly in `mew-web-ui/` and the daemon's alert
path. This plan fixes them. Findings were verified by reading the code on
2026-07-01; line numbers refer to the current working tree.

Ground rules (from the repo's CLAUDE.md files):

- Create a WIP branch off `main` before starting. Commit per-fix, small diffs.
- `just ci` (fmt + clippy + test) must pass. Web UI: `cd mew-web-ui && pnpm build && pnpm test`.
- If you change wire types in `mew-web-client/src/index.ts`, rebuild it
  (`cd mew-web-client && pnpm build`) before building the web UI.
- Note: there is significant uncommitted work in the tree already. Do not
  revert or "clean up" existing modifications; only make the changes below.
  Ask natalie before restructuring anything beyond the listed scope.
- Append a dated section to CURRENT.md when you finish a phase.

Architecture refresher: browser → `@mew/web-client` (`mew-web-client/src/index.ts`)
→ WS → daemon (`crates/mew-daemon/src/lib.rs`). The UI store is
`mew-web-ui/src/stores/session.ts`; `bridgeClientToStore` at the bottom of that
file maps wire events to store actions. `session.broadcast(msg)` on the daemon
sends only to clients attached to *that* session; a browser client is attached
to exactly one session (the one being viewed).

---

## phase 1 — crashes and alert correctness

### 1.1 rules-of-hooks crash in TodoRailPanel

`mew-web-ui/src/components/right-rail.tsx:157-162` — `useState(expandedId)` is
declared *after* the early `return <EmptyState/>` for empty todos. When todos
go 0 → N while the rail is open, React throws (hook count changed).

Fix: move the `useState` above the early return.

Test (vitest, jsdom): render `TodoRailPanel` with `todos={[]}`, re-render with
one todo, assert no throw and the todo renders.

### 1.2 permission/input alerts are never visible

Two facts combine so `PermissionNeeded`/`InputNeeded` alerts are suppressed
100% of the time:

- Daemon: `translate_event` in `crates/mew-daemon/src/lib.rs` returns the
  `SessionAlert` alongside `PermissionRequest`/`AskUserRequest`, and
  `forward_events` sends it via `session.broadcast()` — only to clients of that
  session.
- UI: the `session-alert` handler in `stores/session.ts` (~line 1026) drops
  alerts where `data.session_id === currentSessionId`.

Since you're always attached to the session you're viewing, nobody else ever
receives these alerts. Turn-end alerts already do it right: the prompt handler
loops `for s in active.values() { s.broadcast(...) }`.

Fix: permission/input alerts must reach all sessions' clients.
`translate_event` doesn't have the `SessionManager`, so the cleanest route:
pass `Arc<SessionManager>` into `forward_events` (both call sites already have
it in scope: the Prompt handler binds `session_mgr`, and the `/wiki` handler
can clone it) and have `forward_events` divert `SessionAlert` messages to an
all-sessions broadcast (add a helper like `SessionManager::broadcast_all(msg)`
next to `broadcast_activity` in `crates/mew-daemon/src/session.rs`, and reuse
it for the turn-end loops too). Alternatively return a
`(Vec<ServerMessage>, Vec<ServerMessage>)` split of session-local vs global
messages from `translate_event`. Either is fine; keep it simple.

Keep the UI-side suppression of alerts for the currently-viewed session — that
part is correct (you can see the permission toast already).

### 1.3 turn alerts skipped on early failures

`crates/mew-daemon/src/lib.rs`, Prompt handler: the `TurnComplete`/`TurnFailed`
`SessionAlert` broadcast is nested inside `if let Some(u) = &usage_wire`. A
turn that fails before any `MessageEnd` (provider/auth error) has
`meta.usage == None`, so the failure produces no alert — the case alerts exist
for.

Fix: hoist the alert broadcast out of the usage block. Usage broadcast stays
conditional; the alert is unconditional at turn end.

Test: daemon-side test with the fake provider emitting an error before any
message end; assert a `SessionAlert { kind: TurnFailed }` is broadcast.

### 1.4 OS notification permission is never requested

`stores/session.ts:1039` only checks `Notification.permission === "granted"`;
nothing calls `Notification.requestPermission()`. Fresh origins sit at
`"default"` forever, so notifications never fire (CURRENT.md claims "requests
permission lazily" — that was never implemented).

Fix: in the `session-alert` handler, if `permission === "default"`, call
`Notification.requestPermission()` and show the notification on grant. Note
Safari only honors requestPermission from a user gesture; that's acceptable
degradation for now (the title badge still works). Don't re-request when
`"denied"`.

### 1.5 notification click doesn't navigate

`stores/session.ts:1044-1048` — click handler sets `window.location.hash`, but
the router is history-based (`createRouter` in `main.tsx`, no hash history).
It just appends a fragment.

Fix: the bridge lives outside React, so mirror the existing `client-ref.ts`
pattern — add a module-level navigate ref (e.g. `lib/router-ref.ts`) that
`__root.tsx` populates with `router.navigate`, and use it here. Keep the
`localStorage.setItem(SESSION_ID_KEY, ...)` write, and import `SESSION_ID_KEY`
from `lib/client.ts` instead of the hardcoded `"mew.sessionId"` string (also
hardcoded in `session-rail.tsx` and `command-palette.tsx` — sweep those too).

### 1.6 alerts are write-only; title badge never clears

`alerts` in the store only grows, nothing renders it, and `document.title` is
set to `(N) mew` on alert but never reset.

Fix:
- Add a store action `clearAlertsForSession(sessionId)`; call it from
  `useSessionAttach` (`lib/hooks.ts`) so visiting a session consumes its
  alerts.
- Compute the title in one place: a small `syncTitleBadge()` helper called
  after any alerts mutation, setting `(N) mew` or plain `mew`.
- Minimal visible surface: a bell button in the `SessionRail` header showing
  `alerts.length`, with a dropdown listing alerts (title, kind, relative time)
  that navigates to the session on click. Keep it small — shadcn
  `DropdownMenu`, no new route. Install any new shadcn primitive via
  `pnpm dlx shadcn@latest add <item>` (hard rule in mew-web-ui/CLAUDE.md).

Test: store-level vitest — push alerts for sessions A/B, `clearAlertsForSession("A")`,
assert only B's remain and `document.title` updates.

### 1.7 ⌘K double-binding

`routes/__root.tsx:21-30` binds ⌘K → command palette. `useComposerFocusShortcut`
in `lib/hooks.ts:64-75` (active on the session route) binds ⌘K → focus
composer. Both listeners fire on every press and fight over focus.

Fix (default; natalie can veto): ⌘K keeps the palette, composer focus moves to
⌘L. Update the shortcut hint text if any is displayed. Also: the palette lists
a `⌘N` hint for "New session" but nothing binds ⌘N — either bind it in
`__root.tsx` or drop the hint (binding it is better).

### 1.8 palette theme toggle bypasses ThemeProvider

`components/command-palette.tsx:72-78` writes `data-theme` directly and
persists to `localStorage["mew.theme"]`. The real theme system
(`lib/theme.tsx`) owns the attribute, persists to `"mew-theme-id"`, and will
clobber the toggle on its next render; the palette's key is never read back.
It also assumes theme ids `"light"`/`"dark"` exist in `themes.json`.

Fix: consume the theme context (`useTheme` or equivalent exported from
`lib/theme.tsx`) inside `CommandPalette` and cycle through the ids actually
defined in `themes.json`. Delete the direct DOM/localStorage writes.

---

## phase 2 — flagged files lifecycle

Three compounding bugs make "Pinned Context" wrong across session switches:

### 2.1 stale state across sessions

`reset()` in `stores/session.ts` (~line 874) doesn't clear `flaggedFiles` —
nor `dirListing`, `dirListingPath`, `filePreview`, `gitStatus`. Switching
sessions shows the previous session's pinned files.

Fix: clear all five in `reset()`. (`sessionUsage`, `alerts`, `sessionTitles`,
`groups` are intentionally cross-session — leave them.)

### 2.2 bridge ignores session_id

The `flagged-files-changed` handler (~line 1018) applies `data.files`
unconditionally. Guard with `data.session_id === store.getState().sessionId`.

### 2.3 no replay on attach

Daemon `AttachSession` handler (`crates/mew-daemon/src/lib.rs:399+`) sends
`SessionReady` + `SessionHistory` but never the current flagged set, so a
reload/switch loses the display until the next `flag_important` call.

Fix: after sending `SessionHistory`, read `agent.flagged_files` and `reply()`
a `FlaggedFilesChanged`.

### 2.4 unflag mangles reasons

The `ClientMessage::UnflagFile` arm rebuilds the wire list with
`reason: Some(format!("{:?}", f.mode))`, while the agent's event path
(`translate_event`, `FlaggedFilesChanged` arm) sends `f.reason`. After one
unflag, the remaining chips' reason text changes to Debug-formatted enum
names.

Fix: find where `mew-agent` builds the `FlaggedFilesChanged` event payload
from `flagged_files` (look in `crates/mew-agent/src/tools.rs` / `lib.rs` —
this diff added ~28 lines to each) and extract that mapping into a shared
helper; use it in both the agent emit path, the daemon unflag arm, and the
attach replay from 2.3.

Test: vitest — flagged files set for session A, attach B (reset), assert
empty; flagged-files event for a non-current session id is ignored.

---

## phase 3 — session rail truthfulness

### 3.1 list() never reports "running"

`crates/mew-daemon/src/session.rs:324` hardcodes `SessionState::Active` for
loaded sessions. The new `Session::is_running` mutex is written by the prompt
handler but never read, so a page reload shows a running session without the
pulse until the next activity broadcast.

Fix: in `list()`, set `Running` when `*session.is_running.lock().await`.

### 3.2 archive/pin don't propagate to other clients

`ArchiveSession` broadcasts `GroupsChanged` (which carries no archived flag —
effectively a no-op for this purpose); `PinSession` broadcasts nothing. The web
UI papers over it with optimistic local updates (`session-rail.tsx`
`handleArchive`/`handlePin`) that are never reconciled.

Fix (default): add `ServerMessage::SessionMetaChanged { session_id, archived,
pinned, group_id }` to `crates/mew-protocol/src/lib.rs`, broadcast it to all
sessions from the archive/pin/assign-group arms, mirror in
`mew-web-client/src/index.ts` (type + emit + `MewClientEvents`), rebuild the
client, and add a store handler updating `availableSessions`. Drop the stray
`broadcast_groups` call from the archive arm.

### 3.3 alert titles are ULIDs

Both alert emit sites use `title: session.id.clone()`, so notifications read
"01J8…: turn complete". Use the session's generated title/summary from meta
(fall back to the id). The daemon already has title generation state on the
session; check what's available at those two call sites.

---

## phase 4 — /wiki runs outside turn management

The `/wiki` slash handler (`crates/mew-daemon/src/lib.rs`, SlashCommand arm)
calls `agent.run_with_parts` directly: no `turn_lock`, its cancel token is
never stored in `current_turn_cancel`, `is_running` is never set, no activity
broadcast, and it replies "wiki generated at .mew/wiki.md" even when the turn
errored. A concurrent Prompt would interleave two turns on one agent, and
Cancel can't stop it.

Fix: extract the turn-execution scaffolding from the Prompt handler
(turn_lock → cancel token → is_running/activity broadcasts → forward_events →
turn-end meta/usage/alert) into a helper, and route `/wiki` through it. Return
the `had_error` flag and report failure in the SlashResult text.

This is the riskiest refactor in the plan — the Prompt handler also does
title/summary generation. Keep that Prompt-only. If the extraction gets hairy,
stop and check with natalie rather than duplicating the scaffolding.

---

## phase 5 — tests to lock it in

- vitest (`mew-web-ui/src/`): store tests listed in phases 1-2, plus one for
  `onPartUpdated` state transitions (pending → running → completed) since
  nothing covers it.
- daemon (`crates/mew-daemon`): failed-turn alert test (1.3); attach replays
  flagged files (2.3); `list()` reports Running mid-turn (3.1). Use the
  existing fake provider (`mew-provider-fake`) patterns — check existing
  daemon tests for the harness.

---

## phase 6 — "needs you" ordering in the session rail (feature)

Goal: sessions with a pending permission request or unanswered question sort
to the top of the left rail with an amber indicator, above "running". The rail
becomes a work queue: things blocked on the human first, then things working,
then everything else.

The daemon already knows the answer: `session.pending_permissions` and
`session.pending_ask_user` maps on `Session` (`crates/mew-daemon/src/session.rs`).
Nothing surfaces them cross-session.

### wire

- `SessionInfo` (protocol + `mew-web-client/src/index.ts`) gains
  `pending_permissions: u32` and `pending_questions: u32`. In
  `SessionManager::list()`, fill from the two maps' lengths for active
  sessions; 0 for on-disk sessions.
- New `ServerMessage::SessionAttentionChanged { session_id,
  pending_permissions, pending_questions }`, broadcast to **all** sessions
  (reuse the `broadcast_all` helper from phase 1.2) whenever either map
  changes: on insert in the `PermissionRequest` / `AskUser` arms of
  `translate_event`, and on removal in the response/resolve handlers (permission
  response, ask-user response, and any cancel path that drains pending maps —
  check what Cancel does to them).

### ui

- Store: handler updates the matching `availableSessions` entry; bridge wires
  `session-attention-changed`.
- `session-rail.tsx`: `statePriority()` gets a new tier 0 for
  `pending_permissions + pending_questions > 0` (running becomes 1, active 2,
  idle 3). `StatusDot` shows amber (pulsing) for needs-attention, taking
  precedence over running; add the count next to the dot when > 1.
- The current session should not float to the top just because its toast is
  open — exclude `session_id === currentSessionId` from the attention tier.

### tests

- Daemon: pending permission on session A → `list()` reports
  `pending_permissions: 1`; resolving broadcasts the change with 0.
- Vitest: store sorts a needs-attention session above a running one; current
  session excluded.

---

## explicitly out of scope (do not do without asking)

- The phase-3 file service (`list_dir`/`read_file_preview`/`git_status` +
  `dirListing`/`filePreview`/`gitStatus` store state) has **zero UI
  consumers** and `watch_workspace`/`fs_changed` are no-ops. Leave the
  plumbing alone; building the file-tree panel is a separate project pending
  natalie's call.
- Store immutability refactor: `onProviderEvent`/`onPartUpdated` mutate
  message objects in place inside spread-copied arrays. It works today only
  because `MessageItem` isn't memoized. Worth fixing, but it's a rewrite of
  the hot path — separate branch, separate review.
- `onSessionStatsChanged` fabricates `file_0, file_1…` names to carry a count
  into `ChangeStats.files`; the wire should carry `files_changed: number`.
  Protocol change, low urgency.
- `user-message` dedup can drop identical consecutive messages from another
  client (no origin id on the wire event). Needs a protocol-level client id.
- groups `order` collides after deletes (`create_group` uses `groups.len()`).
  Harmless until group reordering UI exists.
