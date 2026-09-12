# GPUI workspace sidebar cleanup plan

Status: ready for execution.

Audited: 2026-09-11.

Primary surface: `apps/mew-desktop`.

Related prior plan: `notes/mew-clients-multi-workspace-plan.md`. That plan
correctly identified daemon project discovery and workspace-grouped client
rails. Its protocol, daemon discovery, and shared client reducer work have
largely landed. This plan starts from the current implementation and must not
reimplement those pieces under new names.

## Goal

Make the native GPUI sidebar a stable, project-first navigation surface. A
user should be able to identify a workspace, resume a recent task, start a new
task in that workspace, and reach secondary actions without expanding or
reflowing list rows.

The sidebar should feel quiet and direct:

- workspace roots provide the primary hierarchy;
- task titles carry most of the information;
- state appears only when it needs attention;
- controls respond immediately and stay spatially anchored;
- daemon-owned state remains consistent across clients;
- local presentation state remains local to the desktop app.

This work uses the interaction principles in the `apple-design` skill without
copying Apple visual tokens. Priorities are predictable hierarchy, immediate
feedback, stable geometry, keyboard access, restraint, and user agency.

## Confirmed product decisions

1. The sidebar is organized by workspace root, called a **workspace** in UI
   copy. Existing protocol types may retain the `ProjectInfo` name to avoid an
   unnecessary compatibility rename.
2. Existing arbitrary session groups remain stored and supported by the
   daemon, but GPUI stops presenting group creation, deletion, drag-and-drop,
   and assignment in the primary sidebar. Do not delete `groups.json`, clear
   `group_id`, or remove group protocol messages.
3. A future concept may group workspaces, for example `Work -> Product A,
   Product B`. That is a separate entity from session groups and is out of
   scope here.
4. Each workspace initially shows five recent unpinned, non-archived tasks.
   Define this as a named internal constant. Do not add a user preference yet.
5. `Show more` reveals the remaining active tasks for that workspace.
6. Pinned tasks stay inside their workspace, sort before unpinned tasks, and do
   not consume the five-task allowance.
7. Workspaces can be pinned independently. Pinned workspace state belongs to
   the daemon so connected clients agree on ordering.
8. Search covers every session, including archived sessions. Text relevance is
   primary and recency is a bounded secondary boost. Search bypasses the
   five-task limit and temporarily expands matching workspaces.
9. Normal navigation excludes archived sessions. Archived sessions remain
   reachable through a separate sidebar entry and through search.
10. Sessions without a resolvable workspace appear in an `Other` section at
    the bottom.
11. Configured workspaces with no sessions remain visible and can start a new
    task.
12. A workspace row keeps its header compact:

    ```text
    [ folder name ][ kebab ][ new task ]
    ```

    The kebab opens a stable, anchored destination menu containing editor,
    platform, copy-path, and pin actions. GitHub destinations are deliberately
    deferred, but the menu model must allow them to be added later.

## Explicit non-goals

- Do not delete or migrate arbitrary session-group data.
- Do not build workspace collections.
- Do not add a sidebar organization-mode preference.
- Do not add a configurable recent-task count.
- Do not redesign the transcript, composer, workbench, terminal, settings, or
  browser portal except where sidebar focus and dismissal behavior touches
  them.
- Do not implement GitHub remote parsing or GitHub navigation in this change.
- Do not redesign theme tokens or manually edit generated theme assets.
- Do not infer worktree behavior. Treat worktrees according to the canonical
  workspace path returned by the daemon; document observed behavior during
  implementation if it is surprising.

## Current state

### Already available

- `SessionInfo.cwd` is sent for every session and projected to
  `ConversationItem.cwd`.
- `ClientMessage::ListProjects` and `ServerMessage::ProjectList` exist in
  `crates/mew-protocol/src/lib.rs`.
- `ProjectInfo` already carries `path`, `display_name`, `session_count`, and
  `last_used_at`.
- `crates/mew-daemon/src/lib.rs::list_projects` merges configured
  `workspace.roots` with session metadata and sorts by recent activity.
- `mew-client-core::ClientState` stores `projects` and emits
  `ClientEvent::ProjectListChanged`.
- Session pin, archive, title, activity, attention, and cwd metadata already
  reach the shared UI model.
- GPUI already has sidebar search, keyboard traversal, resizing, persisted
  collapse state, and a virtualized list.

### Remaining gaps

- GPUI startup requests `Ping`, `ListSessions`, and `ListModels`, but not
  `ListProjects`.
- `UiModel` does not expose the project list even though `ClientState` has it.
- GPUI ignores the semantic meaning of `ProjectListChanged` and builds rows
  from daemon session groups instead.
- Project discovery counts exact canonical cwd values. It does not expose one
  canonical workspace membership key per session, so clients would have to
  repeat path-containment logic.
- `ProjectInfo` has no pinned state and the protocol has no workspace-pin
  mutation.
- The session overflow menu expands inside a virtualized row and changes its
  measured height. It also embeds the complete group-assignment list.
- A normal task row can show title, path, time, status, pin, drag handle, and
  overflow control at once. Workspace context will make most of that metadata
  redundant.
- `OpenPath` is session-scoped, accepts a relative path, and launches the OS
  default application on the daemon host. It does not implement local desktop
  editor selection or opening an empty workspace.

## Target information architecture

```text
sidebar toolbar
  new task
  search

pinned workspace
  pinned task
  recent task
  recent task
  show more

recent workspace
  recent task
  recent task

other                         only when needed
  task without workspace

archived                      global entry, collapsed/out of the main flow
settings
```

Workspace order:

1. pinned workspaces;
2. unpinned workspaces by most recent non-archived task activity;
3. deterministic case-insensitive display-name/path tie-breakers;
4. `Other` last.

Task order within a workspace:

1. pinned, non-archived tasks by recency;
2. unpinned, non-archived tasks by recency;
3. stable session-id tie-breaker when timestamps match or are absent.

The selected task and any task that is running, failed, or awaiting input must
remain visible even when it falls beyond the five-task limit. These forced
rows do not change the stored expanded/collapsed state.

## Canonical data model and ownership

### Workspace membership

Do not implement path grouping in `sidebar.rs`.

Add a daemon-owned canonical workspace key to session metadata returned on the
wire. A suggested field is:

```rust
pub workspace_path: Option<String>
```

on `SessionInfo`, mirrored by `ConversationItem`. Keep `cwd` because it is the
session's operational working directory; `workspace_path` is the grouping
identity.

Resolve membership in one pure daemon helper:

1. Canonicalize the session cwd when possible; retain a normalized absolute
   fallback when canonicalization fails.
2. Canonicalize configured `workspace.roots` using the same helper.
3. If one or more configured roots contain the cwd, choose the longest matching
   root.
4. Otherwise use the canonical cwd itself as the workspace path.
5. A missing or unusable cwd produces `None` and therefore the `Other` section.

Use this helper both when building `SessionInfo` and when building
`ProjectInfo`. `ProjectInfo.path` and `SessionInfo.workspace_path` must use the
same canonical representation. This is the invariant that lets every client
group by equality and prevents frontend drift.

If loading config per `SessionManager::list` call is undesirable, load the
canonical configured roots into daemon-owned state and pass them through the
existing builder. Do not introduce a second client-specific workspace map.

### Workspace pins

Extend `ProjectInfo` with a defaulted `pinned: bool` field and add an explicit
mutation such as:

```rust
ClientMessage::PinProject { path: String, pinned: bool }
```

Keep persisted project preferences beside daemon session state, using an
atomic sidecar pattern like `groups.json`. A small `projects.json` containing
canonical pinned paths is sufficient. Do not put daemon-owned pins in
`mew_config::State`: the native desktop also writes that file for window and
layout state, and independent read-modify-write paths would create a lost
update race.

On mutation:

- normalize and validate the path against the daemon's known project list;
- persist atomically;
- rebuild the project list;
- broadcast the updated `ProjectList` to all attached clients;
- return a visible protocol error on failure rather than optimistically
  diverging.

Collapse state and `Show more` state remain desktop presentation state:

- collapsed workspace paths persist in `mew_config::State`, replacing the GPUI
  use of `desktop_collapsed_groups` with a clearly named workspace field;
- expanded-through-`Show more` paths may remain in memory for the app lifetime;
- migrate no group data and leave the old field readable for compatibility.

### Shared presentation model

Project `ClientState.projects` into `UiModel.projects`. Add `workspace_path` to
`ConversationItem`. Keep `group_id` in `ConversationItem` while other clients
still use it, but GPUI's new row builder must not consult it.

The desktop client must request `ListProjects` during initial connection. A
reconnect repeats the same bootstrap. Track whether a project response has
arrived if needed to distinguish “loading” from a valid empty list.

## Sidebar row model

Replace the GPUI-specific `SidebarRow::Group` hierarchy with explicit row
semantics. Exact names may follow surrounding style, but the model should
represent at least:

```rust
enum SidebarRow {
    Toolbar,
    WorkspaceHeader { /* path, label, pinned, collapsed, state */ },
    Session(ConversationItem),
    ShowMore { workspace_path: String, hidden_count: usize },
    OtherHeader { /* ... */ },
    ArchivedEntry { count: usize },
    EmptyState,
}
```

Build rows in a pure helper with no GPUI elements. Inputs should include
projects, conversations, local collapse/expansion state, selected session, and
search query. Keep ordering, visibility, and scoring testable without opening
a window.

Avoid cloning the entire conversation and group collections on every rendered
row. Compute the row projection once when relevant metadata or query state
changes. Rows should own only the data necessary to render or use lightweight
shared data where the surrounding code already has a pattern.

## Search behavior

Search remains global and keyboard-first. It matches normalized:

- task title;
- session ID;
- workspace display name and path.

Do not add a fuzzy-search dependency in this pass. Use deterministic match
tiers, for example:

1. exact title match;
2. title prefix or word-prefix match;
3. title substring match;
4. session ID or workspace path substring match.

Within a text tier, apply a small bounded recency bucket before the raw
timestamp. One acceptable scale is activity within 7 days, 30 days, 90 days,
or older/unknown. The bucket must never promote a weaker text match above a
stronger one. Accept `now` as an input to the pure scoring function so tests do
not depend on wall-clock time.

During search:

- show every matching session, including archived sessions;
- group results under workspace headers;
- bypass collapse and the five-task cap;
- annotate archived results quietly;
- include a workspace with all of its matching sessions when the workspace
  name/path itself matches;
- show one concise no-results state;
- clearing search restores the prior collapse and `Show more` state.

## Interaction specification

### Workspace header

- Clicking the disclosure region expands or collapses the workspace.
- The whole header must not ambiguously perform both selection and disclosure.
- Show compact trailing actions on pointer hover and keyboard focus.
- `+` creates a new session with that workspace path using the existing
  `NewSession { cwd }` flow.
- Pin/unpin is available from the workspace context menu.
- Right-click and the overflow action open the same menu.
- Menus use an anchored/deferred overlay and never change virtualized row
  height.
- Press feedback begins on mouse-down. No interaction waits for a decorative
  animation to finish.

### Workspace action menu

- The workspace name remains the expand/collapse target.
- A single kebab button opens the destination menu without changing row height.
- Selecting an editor opens the workspace and makes that editor the next
  remembered target.
- The menu is divided into installed editor destinations and platform actions
  such as Reveal in Finder, Open in Terminal, and Copy Path.
- Do not show unavailable applications.
- When no remembered editor is available, choose a deterministic installed
  fallback or the OS default and update the label/icon accordingly.
- Store the external-editor preference in the desktop state model with a
  defaulted serialized field and a roundtrip test.
- Workspace opening is a local desktop action. Hide or clearly disable it for
  remote daemon profiles because the remote workspace path is not a valid path
  on the desktop host. Do not silently launch an application on the daemon
  host.
- Keep destination representation extensible enough for a future GitHub item,
  but add no GitHub-specific enum variant or dead UI now.

Use platform-specific code behind a small desktop boundary. On macOS, invoke
system tools with argument arrays rather than a shell string. Application
detection and invocation failures must produce a visible, dismissible error.

### Task row

- Render a single-line title by default.
- Do not repeat cwd or workspace name beneath every task.
- Render a semantic state indicator only for running, failed, or needs-input.
- Keep selected state restrained but unmistakable.
- A task pin may be shown quietly, but pinned placement should usually make an
  extra icon unnecessary.
- Pointer hover and keyboard focus reveal one overflow affordance without
  replacing the status indicator or moving the title.
- Right-click and overflow open the same anchored task menu.
- Preserve rename, regenerate title, pin/unpin, and archive/unarchive.
- Remove `Move to group` from the GPUI menu.
- Remove GPUI session dragging if its only destination is an arbitrary group.
- Inline rename may remain if it does not change row height; Escape cancels and
  Enter commits.

### `Show more`

- Render after the fifth visible unpinned task when hidden tasks remain.
- Label may include the hidden count if it stays concise.
- Activation expands only that workspace.
- Once expanded, offer `Show less` at the same spatial location unless the
  workspace becomes short enough that the control is unnecessary.
- Selected and actionable forced-visible rows do not incorrectly reduce the
  hidden count.

### Archived sessions

- Keep one global `Archived` entry near the bottom, with a count.
- Opening it may reuse the sidebar list surface as an archived-only view; it
  must have an obvious Back action and preserve the prior normal-view state.
- Archived search results can be selected directly without first entering the
  archived view.

### Keyboard and accessibility

- Preserve `cmd-k` search focus.
- Up/Down traverses visible task rows; Home/End reach the first/last visible
  task; Enter attaches; Escape returns focus predictably.
- Workspace disclosures, `Show more`, menus, menu items, and workspace kebab actions,
  and Back are reachable by keyboard and expose roles, labels, selected state,
  and expanded state.
- Menu focus stays inside the menu until selection or dismissal. Escape closes
  only the topmost menu and returns focus to its trigger.
- Search result traversal follows ranked visual order.
- Do not make an essential action hover-only. Keyboard focus and context-click
  provide equivalent access.

## Visual behavior

- Preserve existing theme tokens and compact GPUI spacing conventions.
- Use folder/workspace icons as quiet structure, not decorative color.
- Keep task rows at a stable height. Prefer 30-36 logical pixels unless visual
  QA demonstrates a legibility problem.
- Use weight and foreground contrast for workspace hierarchy; avoid uppercase
  section chrome when the folder label already provides structure.
- Avoid counts and timestamps unless they answer an immediate question.
- Use color only for selected, running, warning, or error meaning.
- Anchor menus to their trigger and dismiss them symmetrically.
- Sidebar collapse/expand motion remains interruptible and must start from the
  current presented width. Preserve reduced-motion behavior if the platform
  exposes it; otherwise keep the existing short restrained transition and do
  not add bounce.

## Implementation sequence

Each behavior change follows test-driven development: add a failing behavior
test, make the smallest implementation pass, then refactor while green.

### Phase 1: canonical workspace identity

Primary files:

- `crates/mew-protocol/src/lib.rs`
- `crates/mew-daemon/src/lib.rs`
- `crates/mew-daemon/src/session.rs`
- `crates/mew-client-core/src/reducer.rs`
- `crates/mew-ui-model/src/lib.rs`

Work:

1. Add the canonical workspace resolver as a pure daemon helper.
2. Add the defaulted `workspace_path` field to `SessionInfo`.
3. Populate it for active and idle sessions.
4. Use the same resolver in `list_projects` so project paths and membership
   keys compare directly.
5. Mirror it through `ConversationItem` and add `UiModel.projects`.
6. Keep all new wire fields backward-compatible with serde defaults.

Tests first:

- exact cwd/root match;
- nested cwd chooses the longest configured ancestor;
- symlink/canonical duplicate resolves to one key;
- no configured ancestor falls back to cwd;
- missing cwd maps to `None`;
- active and idle `SessionInfo` use identical rules;
- protocol JSON roundtrip with and without the new field;
- shared reducer and UI projection preserve the workspace key.

### Phase 2: daemon-owned workspace pins

Primary files:

- new focused store module under `crates/mew-daemon/src/`
- `crates/mew-daemon/src/lib.rs`
- `crates/mew-daemon/src/session.rs`
- `crates/mew-protocol/src/lib.rs`
- `crates/mew-client-core/src/reducer.rs`

Work:

1. Add defaulted `ProjectInfo.pinned` and the pin mutation message.
2. Implement atomic daemon-side persistence beside session state.
3. Reject unknown paths and normalize known paths before persistence.
4. Broadcast an updated project list after mutation.
5. Ensure fresh connections receive pinned state through `ListProjects`.

Tests first:

- missing sidecar loads an empty preference set;
- malformed sidecar fails safely without destroying the file;
- pin/unpin persists across store reconstruction;
- canonical aliases do not create duplicate pins;
- unknown project mutation returns an error;
- two attached clients receive the same updated project list;
- old `ProjectInfo` JSON without `pinned` decodes as false.

### Phase 3: desktop bootstrap and pure sidebar projection

Primary files:

- `apps/mew-desktop/src/shell/lifecycle.rs`
- `apps/mew-desktop/src/model.rs`
- `apps/mew-desktop/src/shell/types.rs`
- `apps/mew-desktop/src/shell/helpers.rs`
- `apps/mew-desktop/src/shell/session.rs`
- `apps/mew-desktop/src/shell/tests.rs`
- `crates/mew-config/src/lib.rs`

Work:

1. Request `ListProjects` in the desktop bootstrap sequence.
2. Rebuild the sidebar when project, session, or relevant session metadata
   changes.
3. Introduce the explicit workspace row model and pure builder.
4. Add local collapsed-workspace and expanded-workspace state.
5. Add the five-task cap, forced-visible rules, workspace/task pin ordering,
   `Other`, archived entry, and empty workspaces.
6. Replace persisted GPUI collapsed-group state with collapsed-workspace state
   without touching daemon group state.
7. Add deterministic search ranking and bypass behavior.

Tests first:

- project response is requested during initial connect and reconnect;
- empty configured workspace renders a header;
- sessions group by `workspace_path`, never `group_id`;
- duplicate display names remain distinct and gain a concise parent-path
  disambiguator;
- pinned workspaces sort first;
- pinned tasks remain inside their workspace and do not consume the cap;
- five unpinned tasks render before `Show more`;
- selected/running/failed/needs-input tasks beyond the cap stay visible;
- `Show more` and `Show less` affect one workspace only;
- archived sessions stay out of normal rows;
- missing workspace goes to `Other`;
- search tiers dominate recency buckets;
- recency sorts comparable matches;
- search includes archived tasks and bypasses collapse/caps;
- clearing search restores local disclosure state;
- ordering ties are stable.

### Phase 4: GPUI rendering and interaction cleanup

Primary files:

- `apps/mew-desktop/src/shell/sidebar.rs`
- `apps/mew-desktop/src/shell/session.rs`
- `apps/mew-desktop/src/shell/render.rs`
- `apps/mew-desktop/src/shell/tests.rs`

Work:

1. Render workspace headers and stable-height task rows from the new row model.
2. Remove GPUI group toolbar/actions, group deletion confirmation, group
   assignment UI, group hover state, and group drag/drop state.
3. Keep daemon and shared group support untouched.
4. Replace the inline expanding session menu with an anchored overlay.
5. Add shared right-click/overflow entry points for task and workspace menus.
6. Add archived-only navigation with an obvious return path.
7. Preserve sidebar resizing, selected-session behavior, and focus traversal.

Tests first where GPUI permits direct behavior assertions:

- opening a menu does not change `ListState` item count or row height;
- Escape dismisses the active menu and returns focus;
- task actions target the correct session after sorting/filtering;
- workspace actions target the correct canonical path;
- archived view preserves and restores normal disclosure state;
- keyboard traversal skips headers and `Show more` unless those controls are
  explicitly focused;
- deleting group UI does not remove group protocol handling elsewhere.

### Phase 5: workspace action menu

Primary files:

- focused platform helper under `apps/mew-desktop/src/`
- `apps/mew-desktop/src/shell/sidebar.rs`
- `apps/mew-desktop/src/shell/session.rs`
- `apps/mew-desktop/src/shell/types.rs`
- `crates/mew-config/src/lib.rs`

Work:

1. Model supported local open destinations separately from rendered controls.
2. Detect installed destinations without blocking the GPUI render path.
3. Persist and restore the selected editor.
4. Render the compact kebab action menu on workspace hover and keyboard focus.
5. Implement primary open and the anchored destination menu.
6. Hide/disable local-path actions for remote profiles with explicit copy.
7. Surface launch failures without changing project/session state.

Tests first:

- unavailable remembered editor selects the documented fallback;
- choosing an editor persists it and changes the primary destination;
- command arguments preserve spaces and cannot become shell syntax;
- remote profiles cannot invoke local workspace paths;
- kebab and new-task controls dispatch different actions;
- launch failure becomes visible error state;
- persisted state roundtrips with the new preference absent and present.

## Verification gates

Run narrow tests after each phase:

```bash
cargo test -p mew-protocol
cargo test -p mew-client-core
cargo test -p mew-ui-model
cargo test -p mew-daemon list_projects
cargo test -p mew-daemon --test e2e
cargo test -p mew-desktop --no-default-features
```

Then run the relevant static gates:

```bash
cargo fmt --all -- --check
cargo clippy -p mew-protocol -p mew-client-core -p mew-ui-model -p mew-daemon -p mew-desktop --all-targets -- -D warnings -A clippy::large-enum-variant
just arch-check
just theme-codegen-check
git diff --check
```

Before handoff, run `cargo test --all` and `just ci` when practical. If an
unrelated external failure prevents either gate, record the exact command and
reproducible failure instead of weakening a test.

Visual QA must use a rebuilt native desktop binary against realistic data:

- zero workspaces and zero sessions;
- one empty configured workspace;
- duplicate workspace display names;
- one workspace with 1, 5, 6, and at least 25 sessions;
- several pinned workspaces and tasks;
- long workspace and task names at minimum and maximum sidebar widths;
- active, running, failed, and needs-input tasks beyond the fold;
- global search with strong old matches and weaker recent matches;
- archived search results;
- light and dark themes;
- keyboard-only navigation;
- rapid menu open/close, session switching, search clearing, sidebar collapse,
  and reconnect while updates arrive.

Capture screenshots for the normal, search, menu-open, archived, empty, and
narrow-width states. Slow or repeatedly interrupt sidebar/menu transitions to
look for jumps, stale anchors, focus loss, and input lockout.

## Acceptance criteria

- Every non-archived session appears under exactly one canonical workspace or
  `Other`.
- Workspace grouping and pin ordering survive reconnects and agree across two
  attached clients.
- Arbitrary group data survives unchanged and no group-management control is
  present in the GPUI primary sidebar.
- A workspace shows five recent unpinned tasks by default, plus all pinned and
  forced-visible actionable tasks.
- Search returns all relevant sessions with text relevance stronger than the
  recency boost.
- Task rows do not display redundant workspace paths or routine timestamps.
- Opening any menu causes no list reflow or scroll jump.
- All sidebar actions work with pointer and keyboard input and expose useful
  accessibility labels/state.
- Workspace open actions never confuse local and remote filesystem paths.
- No new user-facing preference exists for organization mode or task limit.
- Relevant tests, clippy, formatting, architecture, generated-theme, and
  visual QA gates pass.

## Execution discipline

- Create a `natb-c/` WIP branch before non-trivial implementation unless the
  current branch is already dedicated to this work.
- Inspect `git status` and coordinate around unrelated edits before every
  phase. Never include unrelated files in a commit.
- Commit by verified behavior slice. Suggested boundaries are canonical
  workspace identity, workspace pins, row projection/search, GPUI rendering,
  and open destinations.
- Update `CURRENT.md` with an append-only dated entry after each verified
  implementation chunk.
- After the feature works and visual smoke testing passes, remove obsolete
  GPUI-only group/render/menu scaffolding, update relevant docs, rerun the
  gates, and ensure the final diff contains no compatibility aliases or dead
  state introduced during migration.

## Deferred follow-ups

- Workspace collections with a hierarchy such as `Work -> Product A`.
- GitHub remote detection and `Open on GitHub` destinations for repository,
  branch, commit, and file context.
- Explicit worktree grouping policy after current support is audited with real
  worktrees.
- A user preference for the recent-task cap, only if observed usage justifies
  its interface and test cost.
- Cross-client adoption of workspace pins and the same grouped rail. The daemon
  and shared-model work in this plan should make that incremental rather than a
  second source of truth.
