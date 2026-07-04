# mew web + iOS multi-workspace plan

Context: companion to `mew-daemon-multi-workspace-plan.md`. Once the daemon honors per-session cwd, the clients need a way to pick a project when creating a session and to organize the rail by project. Depends on the daemon plan landing first (sessions created with a cwd must actually operate there).

Status: planned, not started.

---

## Current state (audited 2026-07-03)

Already working:
- Web rail groups sessions by `deriveWorkspaceName(s.cwd)` and shows a workspace chip per session (`mew-web-ui/src/components/session-rail.tsx:105,220,311`).
- TS client `newSession(cwd)` accepts a cwd and sends it (`mew-web-client/src/index.ts`); the three call sites (`routes/__root.tsx:47`, `routes/index.tsx:18`, `session-rail.tsx:52`) all pass nothing.
- Mobile core `new_session(id, cwd)` plumbs cwd over the wire (`crates/mew-mobile-core/src/lib.rs:389`); iOS `AppStore.newSession()` passes `cwd: nil` (`mew-ios/mew/AppStore.swift:206`).
- `SessionInfo.cwd` is on the wire for every session list.

Gaps:
- No project discovery: all file-service messages (`ListDir`, `ReadFilePreview`, `GitStatus`, `OpenPath`) require a `session_id` and are scoped to that session's cwd (`crates/mew-protocol/src/lib.rs:160-181`). A client cannot browse or enumerate directories before it has a session there — chicken-and-egg.
- No cwd validation: the daemon accepts any string on `NewSession` and persists it (`crates/mew-daemon/src/lib.rs:358-379`). A bad path creates a broken session.
- Mobile core drops cwd: the `SessionList` → `SessionSummary` mapping omits `s.cwd` (`crates/mew-mobile-core/src/lib.rs:746-755`), so Swift never sees it.
- No new-session-with-project UI on either client.

Design choice — project discovery is "recent projects + free-text path", not filesystem browsing:
- `ListProjects` returns deduped cwds from session metas the daemon already has, plus configured `workspace.roots`. Cheap, useful, and adds no new security surface (these paths are already in every `SessionList` response).
- Free-text path entry covers brand-new directories, with daemon-side validation at `NewSession` time.
- Unscoped remote filesystem browsing (tree picker from the phone) is deliberately out of scope: it is a real security-boundary expansion for iroh clients and the recent-projects flow covers the common case. Revisit if it hurts.

---

## Expected state

- Protocol: `ListProjects` client message → `ProjectList` server message: `{ path, display_name, session_count, last_used_at }` per project, derived from session metas + `workspace.roots`, deduped, most-recent first. `NewSession` with a cwd that doesn't exist or isn't a directory fails with a clear error message instead of creating the session.
- Web: "new session" opens a small picker — recent projects list + free-text path input; each workspace group header in the rail gets a "+" that creates a session directly in that project's cwd. First-run auto-create (no sessions yet) keeps today's behavior: no cwd, daemon default.
- iOS: `SessionSummary` carries `cwd`; the rail groups by project the way the web rail does; "new session" presents the same recent-projects picker (sheet), with per-group "+" shortcut. Free-text path entry included but secondary (phones don't type paths happily).
- Both clients render a clear error state when `NewSession` is rejected for a bad path.

## Work items

Order: 1 → 2 and 3 in either order (both depend only on 1).

### 1. daemon + protocol: ListProjects and NewSession validation

- `mew-protocol`: add `ClientMessage::ListProjects` and `ServerMessage::ProjectList { projects: Vec<ProjectInfo> }` with `ProjectInfo { path, display_name, session_count, last_used_at }`. Roundtrip serde tests like the existing message tests.
- `mew-daemon`: handler walks session metas in `session_dir` (cwd, last_message_at), merges `workspace.roots`, dedupes canonicalized paths, sorts by recency. `NewSession`: if `cwd` is `Some`, validate it exists and is a directory before creating; reject with the protocol's error response otherwise (match how `AttachSession` reports failure).
- Tests: daemon-level — sessions in two tempdirs → `ListProjects` returns both with counts; `NewSession` with a bogus path errors and creates nothing on disk.

### 2. web

- `mew-web-client`: `listProjects()` method + `ProjectInfo` type; wire types for the two new messages. Unit tests alongside the existing client tests.
- `mew-web-ui`:
  - new-session flow: replace the bare `client.newSession()` in `session-rail.tsx` with a picker popover (recent projects from `listProjects()`, free-text path input at the bottom). `routes/__root.tsx` / `routes/index.tsx` first-run auto-create stays cwd-less.
  - workspace group headers get a "+" → `newSession(groupCwd)` directly, no picker.
  - error toast/state for rejected paths.

### 3. iOS

- `mew-mobile-core`: add `cwd: Option<String>` to `events::SessionSummary` and map `s.cwd` in the `SessionList` handler (`lib.rs:746`); add `list_projects(id: DaemonId)` sending `ListProjects`; new `CoreEvent::ProjectList`; handle `ServerMessage::ProjectList`. Regenerate the UniFFI bindings (`mew-ios/MewMobileCore/Sources/MewMobileCore/mew_mobile_core.swift`).
- `mew-ios`:
  - `AppStore`: `projectLists: [String: [ProjectInfo]]`, `newSession(cwd:)` variant, handle the new event.
  - `SessionRailView`: group sessions by `cwd` (same last-path-component derivation as web); per-group "+"; "new session" presents a picker sheet listing recent projects with a path text field.
  - error surface for rejected paths (alert or inline).
- Note: uncommitted iOS work is in flight on this branch's working tree (fonts, QR scanner, etc.) — coordinate before touching shared files like `AppStore.swift` / `SessionRailView.swift`.

### 4. docs

`docs/using-mew/web-ui.md` + `docs/using-mew/ios-app.md`: creating sessions in a project, how recent projects are derived. `docs/development/dev-protocol.md`: the two new messages. CURRENT.md entry.

## Risks / watch items

- Path privacy over iroh: `ListProjects` exposes directory paths to any paired device — same data already present in `SessionList`, so no new exposure, but worth remembering if per-device scoping ever becomes a thing.
- Canonicalization for dedupe happens on the daemon (case-insensitive APFS, symlinks); clients treat paths as opaque strings and only derive display names.
- `display_name` collisions (two projects named `api`): clients disambiguate with a parent-dir suffix when names collide, daemon just sends paths.
