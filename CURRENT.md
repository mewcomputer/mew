# 2026-08-03 — filter non-text-output models from the picker

`alibaba-token-plan` (and the whole catalog) carried image/video/audio
generation models (qwen-image-2.0, wan2.7-image, happyhorse-1.1-t2v, …) that
mew's chat adapters can't consume. Added a `text_output` flag to
`mew_catalog::Model`, parsed from models.dev `modalities.output` (defaults to
true when absent, so unknown/legacy models stay selectable; umans/codex
parsers set true since those endpoints only serve chat models).

- `crates/mew-catalog/src/lib.rs`: `Model.text_output: bool` + serde default;
  parsed in `parse_models_dev_model`; `text_output: true` in the umans and
  codex parsers; new test `test_parse_models_dev_text_output` (text model,
  image model, missing-modalities default). 50 tests green.
- `crates/mew/src/commands/daemon.rs`: model lister skips `!m.text_output`
  (feeds the picker in daemon mode → TUI/web/desktop via `ModelInfo`).
- `crates/mew/src/setup/providers.rs`: `discover_models` (local TUI) drops
  catalog-identified non-text models but keeps models the catalog doesn't
  know; `build_custom_model` marks custom models text-capable except when
  merging an existing image-only catalog model.

Filtered 182 models across 26 providers (alibaba: 7 of 22 per region variant).
`cargo test -p mew-catalog` and `-p mew` green; clippy clean on the three
touched crates (pre-existing mew-tui `question_mark` errors untouched).

# 2026-08-03 — verify Alibaba Token Plan setup, fix local config override

Verified the Alibaba Token Plan wiring end to end (config defaults, resolve
chain, prompt family, models.dev catalog + cached catalog, daemon lister
exact-id match; all targeted tests green).

Found a local-machine footgun: `~/.config/mew/config.toml` had a
`[providers.alibaba-token-plan]` section with empty `shape`/`base_url`/
`credential_ref` strings. The layered config merge lets file values override
built-in defaults, so this wiped the built-in provider config: the provider
still looked "available" (credential found via ref fallback) and showed models
in the picker, but `build_direct_provider` failed with `unsupported shape  for
provider alibaba-token-plan`. Removed the empty section (backup:
`~/.config/mew/config.toml.bak.*`); `mew config show` now resolves the
built-in defaults for both providers. Note: `alibaba-token-plan-cn` still has
no credential stored (env/credentials.json/keyring), so it stays unavailable
until one is added.

# 2026-08-03 — add Alibaba Token Plan built-in providers

Added `alibaba-token-plan` and `alibaba-token-plan-cn` as built-in providers.

- `crates/mew-config/src/lib.rs`: two new entries in `Config::default()`
  (shape `openai`, base URLs from models.dev, distinct `credential_ref`s);
  tests `test_default_alibaba_token_plan_provider` /
  `test_default_alibaba_token_plan_cn_provider` plus presence assertions in
  `test_load_default_when_missing`.
- `crates/mew/src/setup/providers.rs`: both ids added to the
  `resolve_provider` fallback chain so they are auto-selected like the other
  built-in credential providers.
- `crates/mew-prompts/src/template.rs`: both ids recognized as the `openai`
  family in `is_model_variant` (system prompt shape); new test
  `test_render_is_model_variant_openai_family_alibaba`.
- `docs/using-mew/providers.md`: built-in provider table rows.

No catalog changes needed — models.dev already lists both providers and the
24h-cached catalog supplies model metadata; the daemon model lister matches
them by exact provider id (`catalog_provider_matches`).

# 2026-08-02 — desktop session management, @-mentions, file tree, inline diffs (parity plan phase 4 batch C)

Batch C covers session organization in the sidebar, composer file references,
a workbench file tree, and tool-call diffs in the transcript.

- **Session rename/pin/archive UI** (`apps/mew-desktop/src/shell/`): a
  per-session context menu (Rename, Pin/Unpin, Archive/Unarchive, plus the
  existing group options) replaces the old group-only picker. Rename edits
  inline in the sidebar row through a third `TextInputTarget::Rename` wired
  into the existing ComposerElement/EntityInputHandler machinery
  (`rename_focus_handle`, `rename_draft`/`selection`/`marked_range`,
  `rename_key_down`/`rename_mouse_down`, `replace_rename_text`/
  `replace_rename_and_mark`); Enter commits, Esc cancels, click-away commits.
  Pinned rows get a pin marker icon.
- **Archived sidebar section**: `build_sidebar_rows` (pure function, tested)
  adds a collapsible "Archived" section (`__archived__` key) below the
  ungrouped/named-group sections; archived sessions no longer appear in their
  group sections. `ConversationItem` gained `pinned: bool` in
  `crates/mew-ui-model` (projected from `SessionInfo`).
- **@-mentions in the composer**: typing a whitespace-delimited `@token` opens
  a picker over the workspace listings already accumulated in shell state from
  `ListDir` responses (no fuzzy filesystem walker — prefix/substring filter,
  case-insensitive, capped at 8 rows). Up/Down/Tab navigate, Enter/click
  completes the path into the composer text, Esc dismisses; keys are handled
  in `composer_key_down` before the slash menu. The overlay is priority 5 in
  `render_picker_overlays`. Pure helpers `mention_query_at_cursor` and
  `filter_mention_candidates` are unit-tested.
- **Clipboard image paste**: `cmd-v` in the "Composer" context runs
  `composer_paste`; `gpui::ClipboardEntry::Image` entries are written to
  `temp_dir()/mew-paste/mew-paste-{ms}.{ext}` (`save_clipboard_image` in
  `session_data.rs`) and attached via the existing attachment flow, while text
  falls back to normal insertion.
- **File tree in workbench Local view**: the Local panel renders a tree built
  from `ListDir` responses; expanding a directory sends a listing request,
  tracked in `file_tree_pending` so `DirListing` responses are distinguished
  from `FsChanged` pushes (content comparison would infinite-loop). The
  workspace watch subscription (`WatchWorkspace`) is enabled only while the
  Local view is active (`sync_workspace_watch`), and `FileTreeChanged` updates
  refresh the tree. Clicking a file sends `OpenPath`. Pure helper
  `collect_file_tree_rows` is unit-tested.
- **Inline tool diffs**: `TranscriptPart::ToolCall` gained
  `diff: Option<String>` in mew-ui-model, projected from the wire's
  `ToolStateCompleted.diff`. `render_tool_part` renders the unified diff
  (`render_tool_diff`, 40-line cap, `+` lines green / `-` lines red / context
  muted).
- **Skipped: OS notifications** (task 4b). The vendored gpui (zed rev
  ae394f3) exposes `App::show_system_notification` but only test platforms
  implement it — the macOS platform is a no-op — and `mew-client-core`'s
  reducer does not project `SessionAlert` into `ClientEvent` at all. Wiring
  real notifications needs either a gpui platform change or a different
  notification crate; left out of this batch.

Verification:

- `cargo test -p mew-desktop` — 78 passed, 0 failed (new tests cover mention
  query/candidates, file-tree row collection, clipboard image naming, and the
  archived sidebar section).
- `cargo test -p mew-ui-model` — 10 passed, 0 failed (includes
  `projects_pinned_flag_and_tool_diffs`).
- `cargo clippy -p mew-desktop --no-deps -- -D warnings` — clean.
- `cargo clippy -p mew-ui-model --no-deps -- -D warnings` — clean.

# 2026-08-02 — desktop composer parity: slash menu, history, pickers, plan feedback (parity plan phase 4 batch B)

Batch B brings the desktop composer and session controls up to TUI parity:
slash-command autocomplete, prompt-history recall, permission-mode and
thinking-variant pickers, and plan-approval feedback. A previous attempt left
the app not compiling (undefined `PromptHistory`, missing `DesktopShell`
initializers); its `types.rs` field layout was kept and completed.

- **Slash autocomplete** (`apps/mew-desktop/src/shell/`): typing `/` at the
  composer start opens a filtered dropdown (daemon-handled commands only:
  `/clear`, `/compact`, `/goal`, `/wiki`, mirroring the daemon's
  `ClientMessage::SlashCommand` handler). Enter/Tab completes the highlighted
  entry into the composer, Esc dismisses until the text no longer starts with
  `/`, Up/Down navigates, and clicking a row completes it. A fully typed
  command submits directly on Enter. On submit, a recognized command line is
  routed to `ClientMessage::SlashCommand` instead of `ClientMessage::Prompt`
  (the daemon does not parse slash commands out of prompt text). The menu is a
  deferred anchored overlay above the composer, reusing the picker overlay
  pattern.
- **Prompt history**: new in-memory `PromptHistory` store (cap 100,
  consecutive-duplicate collapse, stash-and-restore recall). Up/Down recalls
  previous prompts only for a single-line composer with no active selection;
  Down past the newest entry restores the in-progress draft. Edits and
  submissions reset the recall position.
- **Permission mode**: new pill in the composer footer shows the current mode
  (from `UiModel.permission_mode`, already reduced from
  `ServerMessage::PermissionModeChanged`) and opens a picker listing the five
  wire modes with TUI-picker descriptions; choosing one sends
  `ClientMessage::SetPermissionMode`.
- **Thinking variants**: when the current model reports `thinking_variants`,
  a sibling pill opens a variant picker ("off" plus each variant) that sends
  `ClientMessage::SetThinkingVariant` (empty string disables). Current variant
  display via `ThinkingVariantChanged` was already plumbed through
  client-core/ui-model; no protocol or client-core changes were needed.
- **Plan approval feedback**: the plan card's "request changes" button now
  focuses the composer as a feedback input (placeholder switches to "Plan
  feedback… (Enter to send, Esc to cancel)", card shows a hint and cancel
  button); submitting sends `PlanApprovalResponse { approved: false,
  feedback: Some(..) }` instead of the hardcoded `None`. Esc cancels feedback
  mode.
- **Plan-mode indicator**: skipped — the protocol has no plan-mode wire
  signal; plan approval only exists as an `ActionKind::PlanApproval` pending
  action.
- **Multi-question AskUser inputs**: deferred — answers currently route
  through the single composer (one line per question); per-question inputs
  would need N new focusable `EntityInputHandler` targets, a much larger
  refactor.
- **Icons**: added `TablerIcon::Bulb` and `TablerIcon::ShieldLock` following
  the existing inline-SVG pattern.

Verification:

- `cargo test -p mew-desktop` — 72 passed, 0 failed (new logic tests:
  `PromptHistory` recall/stash/cap, slash filter and daemon routing,
  permission-mode labels, thinking-variant lookup).
- `cargo clippy -p mew-desktop --no-deps -- -D warnings` — clean.
- `mew-ui-model` untouched (permission/thinking state was already projected);
  its tests not required.

# 2026-08-02 — desktop activity, usage, and presence parity (parity plan phase 4 batch A)

Phase 4 batch A wires the phase-3 client-core state through the desktop app:
activity view, usage readout, and presence/yield, matching TUI sidebar and web
status-footer semantics.

- **UiModel projection** (`crates/mew-ui-model`): new additive fields
  `subagents: Vec<SubagentEntry>`, `todos: Vec<ActivityTodo>`,
  `usage: Option<UsageSummary>`, `presence: Vec<PresenceEntry>`, and
  `control_yielded_by: Option<u64>`, synced in `sync_client_metadata`. Session
  activity is only refreshed when the snapshot actually contains the attached
  session (metadata-only snapshots omit it), so later metadata events cannot
  wipe the activity view. `ActivityTodo`/`UsageSummary` are new presentation
  types (the wire `Todo`/`SessionUsageWire` lack `PartialEq`); todo status is
  parsed into `ActivityTodoStatus`. `select_session`/`new_conversation*` clear
  activity so a previous session's items never bleed into a new one.
- **Event flow** (`apps/mew-desktop`): new
  `client_event_requires_session_snapshot` helper marks
  `SubagentsChanged`/`TodosChanged`/`UsageChanged`; the client thread now
  requests `engine.ui_snapshot()` for those events so the session-scoped
  activity state reaches the shell (they stay out of the transcript-snapshot
  path, so markdown caches are not invalidated). `PresenceChanged` flows
  through the existing metadata snapshot. New `DesktopShell::yield_control`
  sends `ClientMessage::YieldControl` through the command channel.
- **Activity view**: the workbench Activity tab now renders live data instead
  of placeholder text — a Subagents section (status dot, display name, latest
  status message) and a Todos section (`done/total` header, per-status marker
  and colors mirroring `crates/mew-tui/src/ui/sidebar.rs`). The disclosure
  header summarizes ("2 subagents", "3/5 todos", "Needs input"); empty state
  keeps the previous placeholder copy.
- **Usage/cost + presence/yield**: the composer footer ("checkout-context")
  gained a right-aligned status area: `in/out/$cost` metrics for the selected
  session (hidden while usage is zero), an "N clients" presence chip, and a
  Yield button. When `control_yielded_by` is set the button is replaced by a
  "control yielded" indicator, mirroring the web status footer.
- **Drive-by lint fix**: `DesktopClientEvent::Updated.state` is now
  `Box<ClientState>`; the enum had grown past `clippy::large_enum_variant`
  (pre-existing, surfaced by phase 3's `ClientState` growth).

Verification:
- `cargo test -p mew-desktop` — 66 passed, 0 failed, including new tests for
  session-snapshot event classification and usage label formatting.
- `cargo test -p mew-ui-model` — 8 passed, 0 failed, including a new
  projection test (activity/presence/usage sync, metadata-snapshot
  preservation, clear-on-session-switch).
- `cargo clippy -p mew-desktop --no-deps -- -D warnings` — clean.
- `cargo clippy -p mew-ui-model --no-deps -- -D warnings` — clean.
- `cargo fmt -p mew-desktop -p mew-ui-model -- --check` — clean.

# 2026-08-02 — client-core parity items (parity plan phase 3)

Phase 3 of the desktop parity plan: `mew-client-core`'s reducer no longer
silently drops the server messages the desktop app needs, and the engine
gained the matching outbound send methods. Desktop UI wiring is a later
phase.

- **Subagent status**: `SubagentStart`/`SubagentStatus`/`SubagentEnd` now
  reduce into a per-session `subagents: Vec<SubagentEntry>` (task id,
  name/display name, latest status message, active tool), keyed by parent
  call id; `SubagentStatus` upserts so late-attaching clients still see
  active tasks. Emits `ClientEvent::SubagentsChanged`.
- **Todos**: `TodosUpdated` stores the items on the attached session and
  emits `ClientEvent::TodosChanged`.
- **Presence/yield**: `ClientAttached`/`ClientDetached`/`ControlYielded`
  track `presence: Vec<PresenceEntry>` and `control_yielded_by` on
  `ClientState` and emit `ClientEvent::PresenceChanged`. New
  `ClientEngine::yield_control()` sends `ClientMessage::YieldControl`.
- **Usage/cost**: `SessionUsageChanged` now emits
  `ClientEvent::UsageChanged { session_id }` in addition to accumulating
  `SessionUsageWire`; new `ClientState::usage(session_id)` accessor.
  `TurnEnded` shape unchanged.
- **Rename/archive/pin**: new `ClientEngine::rename_session`,
  `archive_session`, and `pin_session` send methods.
- **File tree**: `DirListing` stores `dir_listing_path`/`dir_listing` on
  `ClientState`; `DirListing` and `FsChanged` emit
  `ClientEvent::FileTreeChanged` (on `FsChanged` the client re-requests
  `ListDir`, mirroring the web store). New
  `ClientEngine::watch_workspace(session_id, enabled)` subscription send.
  `FilesystemDirListing` (pre-session cwd browser) remains unhandled.

Verification:
- `cargo test -p mew-client-core` — 23 passed, 0 failed, including new
  reducer tests (subagent lifecycle/upsert, todos, presence/yield, usage,
  dir listing/fs-changed) and an engine test for the five new send methods.
- `cargo clippy -p mew-client-core --no-deps -- -D warnings` — clean.
- `cargo check -p mew-client-local -p mew-client-iroh -p mew-mobile-core
  -p mew-ui-model` — clean (new enum variants are additive; downstream
  matches use wildcards).

# 2026-08-02 — TUI parity items 8, 5, 2 (parity plan phase 2)

Second phase of the GPUI↔TUI parity catch-up plan — the TUI's three
remaining parity-plan items:

- **Item 8 — Changes section**: `App` now tracks `change_stats` (fed by
  local `AgentEvent::FileDelta`, daemon `SessionList` sync, and
  `SessionStatsChanged` totals) and `flagged_files` (local + daemon
  `FlaggedFilesChanged`). The sidebar renders a "Changes (+A −R)" section
  with flagged (⚑) and changed files when non-empty. New `/unflag <path>`
  command removes a flag via `CommandTarget::unflag_file` (daemon wire
  message + local agent set). `SessionReady` now also syncs
  `status.session_id` so per-session reducer scoping works.
- **Item 5 — project picker**: the in-flight `ListProjects`/`ProjectList`
  protocol gained a `DaemonClient::list_projects` method; `/project`
  (daemon mode) requests the list and opens the new project picker when
  `ProjectList` arrives; selecting a row fires
  `Action::NewSessionInProject` → `CommandTarget::new_session_in`.
- **Item 2 — archive/pin/groups**: `GroupList`/`GroupsChanged` reduce into
  `app.groups`; the sessions rail renders grouped sessions under group
  headers (sorted by group order) with ungrouped sessions flat, and shows
  a "P" marker for pinned sessions. Session picker keybinds ^A/^P toggle
  archived/pinned on the highlighted row (picker hint updated), wired via
  new `CommandTarget::archive_session/pin_session`. New `/rename <title>`
  command renames the active session via `CommandTarget::rename_session`.
  All three use the previously dead `DaemonClient` methods.

Verification:
- `cargo test --all` — full workspace green (exit 0), including new
  reducer tests (change stats, flagged files, SessionList sync), slash
  parsing tests, and dispatch tests (unflag, project flow, session meta
  actions).
- `cargo clippy -p mew-tui -p mew --tests --no-deps -- -D warnings` and
  `just arch-check` — clean.
- Known pre-existing issues (not from this work): `mew-daemon` fails
  `-D warnings` clippy (`handle_connection_with_scope`, 8 args) from
  uncommitted in-flight changes; `test_paste_clipboard_image_no_tool_error`
  is environment-flaky (fails when the OS clipboard contains an image and
  a clipboard tool exists, which makes paste legitimately succeed).

# 2026-08-02 — TUI daemon-mode wiring fixes (parity plan phase 1)

First phase of the GPUI↔TUI parity catch-up plan. Five TUI wiring fixes:

- Tool-call batch expansion is now reachable: collapsed batch rows record
  their visual row (`App::tool_batch_header_rows`) so mouse clicks toggle
  expansion, an expanded batch gets a "▾ N tool calls" collapse header, and
  Ctrl-G toggles the most recent batch (help overlay updated).
- Daemon-mode model picker works: `App::apply_daemon_notification` now has a
  `ServerMessage::ModelList` arm that populates `app.models` and
  `app.thinking_variants` and refreshes the active model's context window
  (previously the message was forwarded on the notify channel but dropped).
- `/sessions` and bare `/resume` open the daemon session picker
  (`SlashResult::OpenSessionPicker`) when `daemon_mode` is set, instead of
  the disk picker; `/resume <id>` in daemon mode attaches instead of
  requiring the session JSONL on local disk.
- `/autotitle on|off` and `/autosummary on|off` now work in daemon mode via
  new `CommandTarget::set_auto_title/set_auto_summary` (previously dead
  commands that always replied "only available in daemon mode").
- `/yield` sends `ClientMessage::YieldControl` in daemon mode via
  `CommandTarget::yield_control`; `/web` removed from the builtin command
  list (it had no handler and fell through to the model as a prompt).

Verification:
- `cargo test -p mew-tui -p mew` — 162 + 126 + dispatch-table tests pass,
  including new tests: ModelList reducer arm, daemon session picker slash
  parsing, autotitle/autosummary/yield target calls, unsupported→alert path.
- `just arch-check` — clean.
- `cargo clippy -p mew-tui --tests` and `cargo clippy -p mew --tests
  --no-deps` — clean. Full `cargo clippy -p mew` is blocked by a
  pre-existing `too_many_arguments` error in `mew-daemon`
  (`handle_connection_with_scope`, 8 args) from uncommitted in-flight
  worktree changes, unrelated to this work.

# 2026-08-01 — Put the native top bar on the base surface

The desktop shell header now uses the shared `background` token, matching the
base work area instead of the raised panel surface. Its horizontal tab fade
edges use the same token, while selected tabs and the connection popover keep
their existing card and panel surfaces.

Verification:
- `cargo check -p mew-desktop` and scoped clippy pass cleanly.
- `just desktop-build` — optimized native bundle packaged successfully.
- formatting and `git diff --check` pass.

# 2026-08-01 — Populate native pickers, theme preferences, and browser URLs

The native shell now receives explicit model and persona catalog events, and
the local fake daemon publishes the built-in `builder` and `planner` personas
plus a usable `fake/fake` model. Model and persona menus are detached,
trigger-anchored overlays with menu semantics instead of in-tree expansion.

Desktop appearance preferences now persist a `system`, `light`, or `dark`
mode with independently selected light and dark theme variants. Theme variant
chips expose selected state to accessibility clients. The browser toolbar has
an editable URL field with selection-aware text editing, safe `http`/`https`
normalization, explicit navigation, and one-shot focus when the browser panel
opens. Return-key handling accepts both native key spellings.

Verification:
- `cargo test -p mew-client-core -p mew-desktop -p mew-config` — 16, 60,
  and 121 tests pass.
- focused theme and browser URL tests pass.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant`,
  `cargo fmt --all -- --check`, and `git diff --check` are clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- Computer Use confirmed the populated model/persona menus and native theme
  controls; the URL field rendered and accepted text in the packaged shell.

# 2026-08-01 — Let persona options size to their content

Persona options no longer use a fixed row height. Each option has a 48px
minimum target and grows naturally when its description wraps. The menu uses a
bounded scroll surface for larger persona sets, while the existing window-aware
placement keeps the popup usable near either edge.

Verification:
- `cargo test -p mew-desktop` — 59 tests pass.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant` — clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- the latest packaged app rendered the native session shell and flexible
  composer controls; Computer Use lost its window handle before the popup
  click on this relaunch, so no click result is claimed here.

# 2026-08-01 — Keep streaming tool rows cacheable

Tool transcript rows now borrow their model strings while rendering instead of
cloning tool input and output on every redraw. Their line/list caches live
behind narrow interior-mutable boundaries, so unrelated shell redraws do not
invalidate the cached output. Markdown cache reuse also accepts equal content
from a new allocation, and non-markdown parts no longer allocate debug strings
just to populate a markdown cache.

The persona picker now shares the model picker’s trigger-relative placement,
with both popovers positioned above their actual composer controls and clamped
to the window margins. Persona rows use a taller two-line layout so their
descriptions stay inside the rounded option surfaces.

Verification:
- `cargo test -p mew-desktop` — 59 tests pass.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant` and
  `git diff --check` are clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- the rebuilt app rendered the real session, composer, session rail, and
  workbench; after attaching the daemon on the default port, Computer Use
  opened both popovers and confirmed their above-trigger placement. The
  persona picker exposed both `builder` and `planner` with their descriptions
  fully contained in the option rows.

# 2026-08-01 — Align the model picker to its trigger

The model picker now captures the composer control's actual bounds and places
the popup directly above that control with an 8px gap. It still clamps to the
window edges, and the popup height is shared with its list layout so the
anchor remains accurate as the model count changes.

Verification:
- `cargo test -p mew-desktop` — 56 tests pass.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant` and
  `git diff --check` are clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- Computer Use on the rebuilt bundle confirms the popup is adjacent to the
  model button instead of floating above the composer container.

# 2026-08-01 — Make daemon restart and native recovery complete

The native GPUI app now quits when its last window closes, so closing the
desktop window also reaps an app-owned daemon instead of leaving the process
behind in AppKit's event loop. The connection recovery action can restart an
app-owned daemon, reconnect the client, and reattach the previously selected
session. Empty metadata-only protocol updates now invalidate the shell, which
keeps the model catalog picker live after startup and reconnect. Session rows
show compact relative timestamps beside their `~/…` path, and empty model or
persona values use actionable picker labels.

Verification:
- `cargo test --all` — all workspace tests and doctests pass.
- focused desktop/UI/supervisor/browser-host tests — 55, 7, 6, and 2 pass.
- scoped clippy, `just arch-check`, `just theme-codegen-check`, `cargo fmt --all`,
  and `git diff --check` are clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- fresh packaged launch loaded sessions, model options, and the builder persona;
  killing the daemon exposed the retry action, which restarted and reattached;
  closing the final window left neither the desktop process nor daemon running.

# 2026-08-01 — Bound responsive workbench and restore browser sessions

The expanded workbench wrapper now uses the same available-width cap as its
content, so a stored width cannot overflow the chat surface after a narrow
window resize. Browser state received while attaching a session is used as the
initial per-session view state when no local override exists, and restoring an
open browser restarts its native pump. Browser events are captured back into
the selected session view state. Removed a duplicate transcript-selection
reset while attaching a session.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host` — 52 and 2 tests pass.
- `cargo clippy -p mew-desktop -p mew-ui-model -p mew-desktop-supervisor -p mew-browser-host -- -D warnings -A clippy::large-enum-variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-08-01 — Make native client teardown cancellable

The native desktop client now retains its transport thread and a dedicated
stop signal. App quit cancels connection startup, in-flight startup sends, and
the receive loop before joining the thread, so a stalled daemon connection
cannot outlive the GPUI shell. The browser workbench state is also captured
per session, including visibility, URL, and title, and restored without
leaking the previous session's native page. The native browser host accepts
the safe `about:blank` default when restoring that state.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host` — 50 and 2 tests pass,
  including cancellation tests for pending and connected client transports.
- `cargo clippy -p mew-desktop -p mew-ui-model -p mew-desktop-supervisor -p mew-browser-host -- -D warnings -A clippy::large-enum-variant` — clean.
- `cargo fmt --all` — clean.

# 2026-08-01 — Make stalled turns retryable

The conversation attention card now offers the same retry action for a turn
that ended without an assistant response as it does for an explicit failed
turn. Running turns retain their cancel action, and pending required actions
still take precedence over the generic attention card.

Verification:
- focused desktop/UI/supervisor/browser-host tests pass: 47, 7, 6, and 2.
- scoped clippy, `cargo fmt --all`, `git diff --check`, and `just desktop-build` are clean.
- the rebuilt native bundle reconnects to the restarted daemon on `127.0.0.1:49721`.

# 2026-08-01 — Finish the native shell performance and overlay pass

The native GPUI shell now keeps large transcript subtrees bounded during
scrolling. Expanded thinking is represented as one outer virtualized row per
markdown block, and tool input/output panes use independent virtualized line
lists with preserved scroll state. The connection-profile and terminal-font
menus now paint as deferred overlays, with the connection menu positioned above
its trigger. Tool-list state is discarded when the visible session is replaced
so long-lived tabs do not retain stale output lists.

The markdown and interaction paths also received small totality and resilience
fixes: malformed or unsupported parts return safely, content-sized user rows
remain right-aligned, and the existing theme-backed syntax/monospace rendering
continues to work for wrapped lists and code.

Verification:
- `cargo test -p mew-desktop -p mew-ui-model -p mew-desktop-supervisor -p mew-browser-host` — 47, 7, 6, and 2 tests pass.
- `cargo clippy -p mew-desktop -p mew-ui-model -p mew-desktop-supervisor -p mew-browser-host -- -D warnings -A clippy::large-enum-variant` — clean; the repository-wide `large_enum_variant` lint remains in the unrelated `mew-message` dependency when the allowance is omitted.
- `just desktop-build` — optimized native bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.
- Fresh packaged native launch connected to the restarted daemon on `127.0.0.1:49721`; daemon accepted the new client as `conn_id=5`, and Computer Use captured the final frame and accessibility tree. Direct Computer Use clicks on this GPUI window did not open the model picker, so picker activation still needs a more reliable native interaction harness.

# 2026-07-31 — Add open-conversation tabs and collapsible shell regions

## Summary

The native shell now has an initial Codex-inspired workbench composition. Open
conversations appear as tabs, while the session library remains in the left
rail. The left rail, right workbench, terminal strip, and workbench sections
can collapse independently, with rounded controls and surfaces throughout.

## Changes

- Added open conversation tab state with active-tab selection, close behavior,
  new-conversation tabs, and selected-session synchronization.
- Added collapsible left rail and right workbench rails.
- Added disclosure state for Spaces, Sessions, Changes, Local checkout, and
  Activity sections.
- Added a collapsible terminal strip and keyboard shortcuts for rail/terminal
  toggles, tab switching, and closing the active tab.
- Removed user/assistant name labels from transcript rows while preserving
  alignment and message styling.
- Added rounded tab, rail, workbench, terminal, and control surfaces using the
  existing theme schema.

## Verification

- `cargo check -p mew-desktop` — clean.
- `cargo test -p mew-desktop -p mew-ui-model -p mew-client-core` — 23 tests pass.
- `cargo clippy -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- Connected native shell visually inspected after rebuild against an
  independently running daemon on `127.0.0.1:53015`.
- Computer-use accessibility is unavailable for the unbundled debug binary;
  interaction coverage still needs a native app bundle or GPUI visual test
  harness.

# 2026-07-31 — Reduce native shell stream update churn

## Summary

The native GPUI shell now keeps streaming updates incremental. Per-delta state
snapshots omit inactive sessions and message history, while the visible
assistant transcript appends deltas directly. GPUI notifications are coalesced
when several daemon frames arrive together, and historical markdown does not
create enter-animation elements.

## Changes

- Added metadata-only and attached-session-only client snapshots.
- Added incremental assistant transcript delta projection with a behavior test.
- Rebuilt markdown and transcript rows only for message-boundary/history events
  or actual text changes.
- Batched queued desktop client events into one shell update and notification.
- Kept word/code-line entrance motion for the newest message and the composer
  submit animation, while rendering historical content statically.

## Verification

- `cargo test -p mew-client-core -p mew-client-local -p mew-ui-model -p mew-desktop` — 24 tests pass.
- `cargo clippy -p mew-desktop -p mew-client-core -p mew-ui-model --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo build -p mew-desktop`, `cargo fmt --all`, and `git diff --check` — clean.
- Rebuilt native shell launched against `ws://127.0.0.1:53015` and rendered the restored session successfully.

# 2026-07-31 — Render markdown and virtualize native shell lists

## Summary

The native GPUI shell now renders conversation content as markdown and keeps
large UI collections bounded during layout. Streamed words and code lines get
stable enter animations, while submitting a prompt retriggers a small composer
response animation.

## Changes

- Added a GPUI-friendly markdown projection on top of `mdstream` for headings,
  emphasis, links, lists, quotes, code fences, tables, and thematic breaks.
- Resolved markdown colors and text treatments through the existing theme
  schema.
- Rendered markdown blocks as variable-height virtual transcript rows, so a
  single large assistant response does not become one unbounded element tree.
- Virtualized sessions, model choices, and review files with GPUI uniform lists.
- Added stable per-word and per-code-line enter animation IDs, plus a composer
  submit animation hook that respects GPUI reduced-motion handling.

## Verification

- `cargo test -p mew-desktop -p mew-client-core -p mew-client-local -p mew-ui-model` — 21 tests pass.
- `cargo clippy -p mew-desktop -p mew-ui-model --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo build -p mew-desktop -p mew` — clean.
- `cargo fmt --all`, `git diff --check`, and live native GPUI smoke test against `127.0.0.1:25578` — clean.

# 2026-07-30 — Load sessions and expose model selection

## Summary

The native shell now bootstraps the session and model state it needs to be
useful. It requests the daemon's model catalog, restores the first available
non-archived session with its history, and exposes a model picker that works
for both attached sessions and new conversations.

## Changes

- Requested `ListModels` alongside the initial ping and session list.
- Projected model/provider identity and model metadata into `mew-ui-model`.
- Auto-attached the first non-archived session after startup so its history and
  current model load through the normal protocol path.
- Added a model picker with daemon-backed `SwitchModel` commands.
- Ordered new-session model selection, model-switch confirmation, and pending
  prompt submission so the first prompt uses the selected model.
- Added equality derives to the shared model catalog types for UI state tests.

## Verification

- `cargo test -p mew-client-core -p mew-client-local -p mew-ui-model -p mew-desktop` — 17 tests pass.
- `cargo clippy -p mew-ui-model -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Turn the composer into a real chat input

Reworked the desktop composer from a one-line painted label into a proper
multiline GPUI text surface. It now has wrapped text layout, a visible editing
area with room for multiple lines, a separate model/persona/action row, and a
caret positioned against the wrapped text. Existing platform text input,
selection, attachments, submit, and cancel behavior remain in place.

Verification:
- `cargo test -p mew-desktop` — all 25 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt native bundle — composer has a taller editing surface and a separate control row.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Split the desktop shell into focused modules

Reduced `apps/mew-desktop/src/main.rs` from roughly 6,000 lines to a small
entry point. The shell now has separate modules for lifecycle and transport,
session state, composer/input, chat rendering, sidebar and topbar rendering,
settings, workbench panels, platform setup, shared helpers/types, and tests.
The refactor preserves the existing behavior and keeps the shell tests under a
dedicated test module.

Verification:
- `cargo test -p mew-desktop` — all 25 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Stabilize conversation switching and shell motion

Session switching now targets the requested session so a stale pending
new-session response cannot replace an existing conversation. Existing
conversations open in their own tabs while draft tabs remain intact. The
terminal fades and slides into place, draggable areas expose hover grips, the
auxiliary workbench can expand to 720 px, and settings remains accessible
without occupying the conversation tab strip. Removed the desktop-attached
browser system notice while preserving the existing browser capability.

Verification:
- `cargo test -p mew-desktop -p mew-daemon -p mew-ui-model -p mew-session` — all focused tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — settings tab removed, session/group controls and workbench grip visible.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Package the native CEF helper topology

The native macOS bundle now includes the process-specific CEF helper app
bundles used for renderer, GPU, plugin, alerts, and base browser processes.
The GPUI launcher also exports the packaged framework path before starting
CEF, allowing those helpers to load the sibling framework instead of
crashing during startup.

Verification:
- `just desktop-build` — native bundle built and packaged successfully.
- isolated native smoke launch — renderer helper processes started from the packaged bundle without the previous `library_loader` panic.
- `cargo test -p mew-browser-host -p mew-client-core -p mew-ui-model -p mew-desktop` — 35 tests pass.
- focused clippy, `cargo fmt --all`, and `git diff --check` — clean.

The remaining browser QA is a visual unlocked-session check of the page pixels
and input/focus behavior. The current locked-session run can verify process
startup but cannot provide a reliable compositor screenshot or CDP response.

# 2026-07-30 — Distinguish daemon errors from connection failures

## Summary

Submitting a prompt could surface the generic “connection failed” status even
when the WebSocket was still connected. The native shell now keeps daemon and
provider errors separate from transport state and shows the server's actual
message below the composer.

## Changes

- Added `ShellModel.last_error` for actionable daemon and transport details.
- Kept `ClientEvent::Error` from changing a healthy connection into a failure.
- Added a regression test for provider/session errors being reported without
  masquerading as socket failures.

## Verification

- `cargo test -p mew-client-core -p mew-client-local -p mew-ui-model -p mew-desktop` — 17 tests pass.
- `cargo clippy -p mew-ui-model -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-30 — Wire the native composer to daemon prompts

## Summary

The GPUI composer is now an actual focused text surface. It accepts platform
text input through `ElementInputHandler`, keeps cursor positions correct across
Unicode and UTF-16 callbacks, and submits prompts with Enter. A prompt typed
before the first session exists is held until the daemon reports the new
session ready, then sent through the existing client engine.

## Changes

- Added a native composer element with focus tracking, cursor painting, and
  platform text-input registration.
- Added prompt submission for attached sessions and first-session bootstrap.
- Added Unicode cursor offset coverage for the GPUI text-input boundary.

## Verification

- `cargo test -p mew-ui-model -p mew-desktop` — 6 tests pass.
- `cargo clippy -p mew-ui-model -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.
- Native launch smoke check — composer surface rendered without a reactor panic; debug shell processes were stopped after verification.

# 2026-07-30 — Establish the Codex-style native shell composition

## Summary

The native GPUI window now has the first screenshot-aligned shell composition:
a single conversation workspace with a left orientation rail, a dominant
conversation column, and an independently visible right review/workbench
region. The shell has no tabs and keeps empty states explicit when the daemon
has no sessions or workspace changes.

## Changes

- Replaced the provisional root render with a model-driven top bar, session
  rail, conversation surface, and optional review pane.
- Opened the review pane by default so workspace state remains visible beside
  the conversation, matching the intended coding-workbench workflow.
- Added stable scroll and interaction IDs, constrained flex parents, and
  theme-schema colors throughout the new regions.
- Kept the composer and terminal strip as visible layout surfaces until their
  native input and terminal behavior are implemented in the next slice.

## Verification

- `cargo test -p mew-ui-model -p mew-desktop` — 5 tests pass.
- `cargo clippy -p mew-ui-model -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.
- `MEW_DESKTOP_DAEMON_URL=ws://127.0.0.1:9 cargo run -p mew-desktop` — native window launched without a reactor panic; visual smoke check confirmed the shell and connection-failure state.

# 2026-07-30 — Isolate Tokio transport from the GPUI executor

## Summary

The native shell no longer polls Tokio WebSocket futures on GPUI's executor.
The desktop client runs its protocol engine inside a dedicated current-thread
Tokio runtime and sends typed state snapshots back to the GPUI entity. This
removes the `no reactor running` launch panic and makes connection shutdown an
observable error instead of a busy loop.

## Changes

- Added a Tokio runtime/thread boundary around the local WebSocket client.
- Added a futures channel for typed client events and cloned client snapshots
  delivered to the GPUI task.
- Made `ClientState` cloneable for the UI projection boundary.
- Treat a closed transport as `TransportError::Closed` in `ClientEngine`.

## Verification

- `cargo test -p mew-client-core -p mew-desktop -p mew-ui-model` — 14 tests pass.
- `cargo clippy -p mew-client-core -p mew-client-local -p mew-ui-model -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `MEW_DESKTOP_DAEMON_URL=ws://127.0.0.1:9 cargo run -p mew-desktop` — launched without a reactor panic and was stopped after the connection-failure smoke test.

# 2026-07-30 — Add framework-independent conversation navigation model

## Summary

The native shell now projects daemon session metadata and assembled messages
into a UI model that GPUI can render without owning protocol state. The shell
can display conversation rows, attach to an existing session, request a new
session, and render the selected transcript. Its theme follows the window's
system light/dark appearance while continuing to resolve colors from the
existing mew theme schema.

## Changes

- Added `mew-ui-model` for conversation summaries, transcript items, selection,
  new-session commands, and composer command validation.
- Added stable IDs and constrained scroll layout for the transcript and
  interactive conversation rows.
- Added appearance observation so the GPUI shell reloads the matching `dark`
  or `light` theme from the shared manifest.
- Added a command channel between GPUI actions and the client engine for
  attach/new-session requests.
- Applied the GPUI reference guidance on `cx.notify()`, stable scroll IDs,
  `min_h_0()`, and entity-owned state updates.

## Verification

- `cargo test -p mew-ui-model -p mew-desktop` — 5 tests pass.
- `cargo clippy -p mew-ui-model -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` — clean.

# 2026-07-30 — Allow native desktop session commands on local daemon transport

## Summary

The native shell can now create and attach sessions through the local TCP
daemon without being rejected as a legacy iroh client. The daemon restriction
for desktop clients remains limited to the legacy iroh transport.

## Changes

- Added an explicit legacy-iroh transport flag to the daemon connection path.
- Added an end-to-end regression test for `ClientKind::Desktop` over the local
  transport.
- Kept prompt submission gated on confirmed daemon attachment, so a prompt
  waits for `SessionReady`/history instead of racing the session command.

## Verification

- `cargo test -p mew-daemon --test e2e` — 17 tests pass.
- `cargo test -p mew-client-core -p mew-client-local -p mew-ui-model -p mew-desktop` — 18 tests pass.
- `cargo check -p mew-daemon --features iroh` — clean.
- Focused clippy, `cargo fmt --all`, and `git diff --check` — clean.
- Live native GPUI smoke test against `127.0.0.1:25577` — sessions, transcript, model catalog, and connected state loaded without the previous daemon error.

# 2026-07-30 — Bootstrap the native GPUI desktop shell

## Summary

The native desktop migration now has a compilable GPUI application boundary
that owns the shared desktop supervisor and connects to the daemon through a
framework-independent local WebSocket transport. The first shell keeps the
conversation surface dominant, reserves the left rail for workspace and
session orientation, and consumes the existing TUI theme schema for its
colors.

## Changes

- Added `mew-client-local` for typed WebSocket transport over the existing
  daemon protocol.
- Added `apps/mew-desktop` with a native GPUI window, application menu, daemon
  startup ownership, connection state, and session count projection.
- Pinned GPUI to the upstream revision used by this initial shell slice.
- Kept the right workbench collapsed until conversation and workspace actions
  have a real model behind them.

## Verification

- `cargo test -p mew-client-local` — 1 test passes.
- `cargo test -p mew-desktop` — 2 tests pass.
- `cargo fmt --all -- --check` — clean.
- `cargo clippy -p mew-client-local -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- Broad clippy remains blocked by the pre-existing `mew-message` large enum
  warnings.

# 2026-07-30 — Route Tauri daemon lifecycle through shared supervisor

## Summary

The existing Tauri shell now delegates daemon endpoint selection, health
checks, and configured or packaged process ownership to the Tauri-free desktop
supervisor. Tauri retains only the platform-specific sidecar fallback and its
existing CEF integration.

## Changes

- Added endpoint adoption for a host-launched sidecar without transferring
  process ownership to the shared supervisor.
- Added shared remote-mode restart handling.
- Replaced the duplicated Tauri daemon lifecycle implementation with a thin
  adapter around `mew-desktop-supervisor`.
- Preserved the fixed local rendezvous port, remote-state persistence, daemon
  logs, sidecar launch, and CEF/browser code.

## Verification

- `cargo test -p mew-desktop-supervisor` — 5 tests pass.
- `cargo check --manifest-path mew-web-ui/src-tauri/Cargo.toml` — clean.
- `cargo clippy --manifest-path mew-web-ui/src-tauri/Cargo.toml -- -D warnings` — clean.

# 2026-07-30 — Introduce typed mobile client events

## Summary

Mobile daemon connections now retain the framework-independent client events
produced by the shared reducer. The existing `CoreEvent` listener remains as a
compatibility projection while native platform consumers move to the shared
event vocabulary.

## Changes

- Added a typed event inbox to each mobile daemon connection.
- Reduced every incoming server message through `ClientState`, including
  provider frames, before updating the mobile projection.
- Routed connecting, connected, backoff, and disconnected states through the
  shared connection reducer.
- Added coverage for session-ready and required-action events entering the
  typed event stream.

## Verification

- `cargo test -p mew-client-core` — 9 tests pass.
- `cargo test -p mew-mobile-core --lib` — 26 tests pass.
- `cargo test -p mew-mobile-core --features test-harness --test m0_spike -- --nocapture` — 1 test passes.
- `cargo test -p mew-mobile-core --features test-harness --test m1_integration -- --nocapture` — 2 tests pass.
- `cargo clippy -p mew-client-core -p mew-mobile-core --all-targets -- -D warnings` — clean.

# 2026-07-30 — Introduce shared daemon-wide mobile state shadow

## Summary

Mobile daemon translation now reduces non-streaming protocol messages into the
framework-independent client state and synchronizes the existing UniFFI
projection from it. Provider streams continue through the already migrated
shared session reducer so the FFI event contract stays stable.

## Changes

- Added shared client state to each mobile daemon connection.
- Reduced session readiness, history, user messages, required actions, models,
  personas, mode changes, and other daemon metadata through `ClientState`.
- Added explicit synchronization in both directions around provider events so
  reconnect and streaming state remain coherent during the adapter transition.
- Fixed `SessionReady` to record the attached session for new sessions.
- Added tests proving shared state is populated by mobile translation.

## Verification

- `cargo test -p mew-client-core` — 9 tests pass.
- `cargo test -p mew-mobile-core --lib` — 25 tests pass.
- `cargo test -p mew-mobile-core --features test-harness --test m1_integration -- --nocapture` — 2 tests pass.
- `cargo clippy -p mew-client-core -p mew-mobile-core --all-targets -- -D warnings` — clean.

# 2026-07-30 — Route mobile session projection through shared core

## Summary

The mobile session adapter now maintains canonical `mew-client-core` session
state and projects it into the existing UniFFI records, preserving mobile
metadata and optimistic prompt behavior.

## Changes

- Added canonical prompt recording and provider-event projection helpers to
  `mew-client-core`.
- Added a shared session field to mobile `SessionState` and synchronized text,
  reasoning, tool, usage, and manifest data into the mobile representation.
- Preserved manifest model identifiers and optimistic prompt echo deduplication.
- Updated stale iroh test fixtures for the current remote-auth handler shape.

## Verification

- `cargo test -p mew-mobile-core --lib` — 24 tests pass.
- `cargo test -p mew-client-core` — 8 tests pass.
- `cargo test -p mew-mobile-core --features test-harness --test m0_spike -- --nocapture` — pass.
- `cargo test -p mew-mobile-core --features test-harness --test m1_integration -- --nocapture` — 2 tests pass.
- `cargo clippy -p mew-mobile-core -p mew-client-core --all-targets -- -D warnings` — clean.

# 2026-07-30 — Add Tauri-free desktop supervisor boundary

## Summary

Added the first native-desktop daemon lifecycle boundary without importing
Tauri, CEF, or frontend types into the supervisor.

## Changes

- Added `crates/mew-desktop-supervisor` to the workspace.
- Added attach-only handling for explicit WebSocket endpoints.
- Added app-owned local daemon launch with ephemeral loopback ports, optional
  logging, health probing, restart, and shutdown ownership.
- Rejected ambiguous remote configuration where an explicit endpoint is mixed
  with app-owned remote mode.
- Kept Tauri sidecar and CEF integration in the existing adapter until it can
  consume this contract safely.

## Verification

- `cargo test -p mew-desktop-supervisor` — 4 tests pass.
- `cargo clippy -p mew-desktop-supervisor --all-targets -- -D warnings` — clean.
- `git diff --check` — clean.

# 2026-07-30 — Add headless client lifecycle and reconnect coverage

## Summary

Connected the shared client reducer to an async transport driver so the core
now proves a complete headless conversation lifecycle.

## Changes

- Added `ClientEngine` for typed send, receive, prompt, and close operations.
- Made the in-memory transport support independent reconnects.
- Added a lifecycle test covering session attach, prompt echo handling,
  streaming text, required actions, turn usage, command sending, disconnect,
  and session-history restoration.

## Verification

- `cargo test -p mew-client-core` — 8 tests pass.
- `cargo clippy -p mew-client-core --all-targets -- -D warnings` — clean.

# 2026-07-30 — Start framework-independent desktop client core

## Summary

Started the Tauri-to-GPUI migration with a client-core seam that is independent
of GPUI, UniFFI, and platform services.

## Changes

- Added `crates/mew-client-core` to the workspace.
- Added a tolerant server-message codec and typed client-message encoder.
- Added a transport contract with an in-memory implementation for deterministic
  tests.
- Added a reducer for session history, streaming provider parts, usage,
  optimistic prompt echo handling, model/session metadata, and required-action
  tracking.
- Updated `mew-mobile-core` to consume the shared codec while preserving its
  public compatibility module.

## Verification

- `cargo test -p mew-client-core` — 7 tests pass.
- `cargo test -p mew-mobile-core --lib` — 24 tests pass.
- `cargo clippy -p mew-client-core -p mew-mobile-core --all-targets -- -D warnings` — clean.
- `cargo fmt --all` — clean.

# 2026-07-30 — Re-fix Kimi K3 tool-call response mismatch (jobblock:80)

## Summary

The previous fix changed `content: null` to `content: ""` for assistant messages with `tool_calls`, but Kimi (and other OpenAI-compatible providers) was still rejecting follow-up requests with:

```text
an assistant message with 'toolcalls' must be followed by tool messages
responding to each 'toolcallid'. The following toolcallids did not have
response messages: jobblock:80
```

Two issues could leave the assistant message without a matching tool response:

1. **Cancelled tool turns**: if a user cancelled while a tool was running, the agent aborted the turn without appending the tool-result message, leaving a broken conversation history for the next request.
2. **Ordering bug in the OpenAI adapter**: if a user message somehow carried both text and a `ToolResult`, the adapter emitted the user text *before* the `role: tool` messages, violating the rule that tool messages must immediately follow the assistant message that issued the tool calls.

## Changes

- `crates/mew-agent/src/turn.rs`
  - When a turn is cancelled after the assistant message has been appended but before/while tools finish, the agent now still appends a matching `ToolResult` message for every pending tool call.
  - Unprocessed tool calls are marked as errored, and the updated assistant message is synced back into the store instead of being appended a second time.

- `crates/mew-provider-openai/src/lib.rs`
  - In user messages that contain both `ToolResult` parts and text/image content, emit the `role: tool` messages first, then the user message. This keeps the tool responses immediately after the assistant message even if a message also carries text.
  - Added `test_build_wire_message_tool_results_before_user_text` to lock in the ordering.

- `crates/mew-agent/src/tests.rs`
  - Added `test_cancelled_tool_turn_appends_tool_results` to verify that a cancelled turn leaves a valid history (user, assistant tool-call, tool result).

## Verification

- `cargo test -p mew-agent` — 142 tests pass.
- `cargo test -p mew-provider-openai` — 9 tests pass.
- `cargo test --all` — all pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.
- `just arch-check` — pass.

# 2026-07-26 — Fix duplicated thinking duration across reasoning blocks

## Summary

The TUI was showing the same elapsed thinking time for every reasoning block in the transcript. The elapsed duration was stored in a single global `Option<Duration>` on `App`, so once any reasoning block finished, all collapsed reasoning headers rendered that same duration.

## Changes

- `crates/mew-tui/src/app/mod.rs`
  - Replaced `pub reasoning_elapsed: Option<Duration>` with `HashMap<PartId, Duration>` so each reasoning block keeps its own elapsed time.
  - Added `record_reasoning_elapsed(collapse: bool)` helper that records the active reasoning block's duration and optionally collapses it.
  - Finalize the active reasoning block when a new reasoning part starts, a text/toolcall part starts, the reasoning part ends (`PartEnd`), or the message ends (`MessageEnd`).
  - Only collapse the reasoning block when the model moves on to a text or toolcall part, preserving the existing behavior where a final reasoning block stays expanded.

- `crates/mew-tui/src/ui/chat.rs`
  - Rendering now looks up the elapsed duration for each reasoning part by its `PartId` instead of reading a global value.
  - Added `test_reasoning_headers_use_per_part_elapsed` to verify that two reasoning blocks in the same message show their own recorded durations.

## Verification

- `cargo test -p mew-tui` — 159 unit tests + 5 golden tests pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.

# 2026-07-26 — Fix Kimi K3 (OpenAI adapter) tool-call validation errors

## Summary

Kimi K3 was rejecting follow-up requests after a tool call with:

```text
an assistant message with 'toolcalls' must be followed by tool messages
responding to each 'toolcallid'. The following toolcallids did not have
response messages: bash:14
```

The OpenAI adapter was sending assistant messages with `content: null` and empty `reasoning`/`reasoning_content` fields. Some OpenAI-compatible providers (including Kimi) reject that combination when `tool_calls` is present, causing the server to fail validation before it reaches the tool response message.

## Changes

- `crates/mew-provider-openai/src/lib.rs`
  - For assistant messages with empty content, emit `""` instead of `null`.
  - Omit `reasoning` and `reasoning_content` fields when the reasoning string is empty; only echo them back when non-empty reasoning was actually streamed.
  - Added `test_build_wire_message_tool_result_pair` to verify that an assistant message with a tool call and a user message with the matching tool result produce a valid request body.

## Verification

- `cargo test -p mew-provider-openai` — 8 tests pass.
- `cargo test -p mew-agent` — 141 tests pass.
- `cargo clippy --all -- -D warnings` — clean.

# 2026-07-26 — Fix mew-prompts transclude and make it render nested content

## Summary

Four mew-prompts tests were failing because `transclude(...)` returned raw file contents without rendering the nested Jinja directives inside them. The system prompt (`base.md`) was recently split into provider-specific partials via `{{ transclude(...) }}`, but the `transclude` function only inserted the raw text. This left literal `{{ transclude(...) }}` directives in the rendered system prompt and broke tests that expected text from the rendered partials.

## Changes

- `crates/mew-prompts/src/template.rs`
  - Changed the `transclude` function to take the current minijinja `State` and render the included content against a clone of the original template context, so nested directives resolve recursively.
  - If the included content fails to render, it falls back to the raw content with a warning.

- `crates/mew-prompts/src/persona.rs` and `crates/mew-prompts/src/template.rs`
  - Updated the four transclude tests to assert on text that actually exists in the current rendered system prompt (`"Treat the current prompt context as authoritative"`).

## Verification

- `cargo test -p mew-prompts` — 52 tests pass (was 48 passing + 4 failing).
- `cargo test --all` — all pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.
- `just arch-check` — pass.

# 2026-07-26 — Fix startup crash on unknown provider and make config loading consistent

## Summary

`mew` was crashing on startup with:

```text
Error: build provider

Caused by:
    unknown provider zhipuai-coding-plan
```

The provider resolution in `async_main` was using a config loaded in the top-level startup path, while `chat_cmd` and `run_cmd` reloaded config independently before building the provider. If the two loads ever diverged (e.g. an environment-only provider present during resolution but missing at build time), the resolved provider could be passed to `build_provider` and fail as unknown. The error message also gave no hint about where the config was loaded from or how to fix it.

## Changes

- `crates/mew/src/main.rs`
  - Pass the already-loaded `Config` from `async_main` into `chat_cmd` and `run_cmd` instead of letting them reload it.

- `crates/mew/src/commands/tui.rs`
  - `chat_cmd` now accepts `cfg: mew_config::Config` and uses it directly (no second `mew_config::load()`).

- `crates/mew/src/commands/run.rs`
  - `run_cmd` now accepts `cfg: mew_config::Config` and uses it directly.

- `crates/mew/src/setup/providers.rs`
  - Improved the `build_provider` "unknown provider" error to include the config file path, the list of available providers, and a pointer to `state.toml` so the user can clear stale persisted state.

## Verification

- `cargo test --all` — all pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.
- `just arch-check` — pass.

# 2026-07-25 — Add Gleam language support to code highlighter and hashline block resolver

## Summary

Added Gleam (`.gleam`) support to two systems:
1. **`ratatui-mdstream`** — ```` ```gleam ```` code fences are now syntax-highlighted. `two-face` 0.5 does not bundle a Gleam syntax definition, so a custom `gleam.sublime-syntax` is embedded and merged into the `SyntaxSet` at init time via `SyntaxSet::into_builder()` + `SyntaxDefinition::load_from_str()`.
2. **`mew-hashline`** — `SWAP.BLK` / `DEL.BLK` / `INS.BLK.POST` block resolution now works for `.gleam` files via the `tree-sitter-gleam` grammar.

Changes:
- `crates/mew-hashline/Cargo.toml` — added `tree-sitter-gleam = "1.0"` dependency.
- `crates/mew-hashline/src/block.rs` — registered `("gleam", tree_sitter_gleam::LANGUAGE.into())` in `build_language_table()`; added `resolve_gleam_function_block` test.
- `crates/ratatui-mdstream/resources/gleam.sublime-syntax` — new TextMate-style syntax definition covering Gleam keywords, types, strings, numbers, comments, constants, operators, and function calls. Written from scratch (not copied from any GPL source).
- `crates/ratatui-mdstream/src/highlight/syntect.rs` — `syntax_set()` now extends two-face's `SyntaxSet` with the embedded Gleam definition; added `test_gleam_syntax_available` and `test_gleam_highlighting_produces_styles` tests.

Verification:
- `cargo test -p mew-hashline` — 57 passed (including new Gleam block test).
- `cargo test -p ratatui-mdstream` — 22 passed (including 2 new Gleam tests).
- `cargo test --all` — all pass, 0 failures.
- `cargo clippy --all -- -D warnings` — clean.
- `just arch-check` — pass.
- `just theme-codegen-check` — pass.
- `cargo fmt -- --check` — clean on changed files.

Notes:
- `two-face` 0.5 confirmed (via test program) to NOT include Gleam, hence the custom syntax file.
- The sublime-syntax uses standard TextMate scope names (`keyword.control`, `storage.type`, `constant.numeric`, `entity.name.function`, etc.) that the existing `theme.tmTheme` already maps colors for.

# 2026-07-19 — Switch Kimi to OpenAI adapter + fix reasoning_content deserialization

## Summary

Kimi K3 thinking was silently dropped. Two issues, both fixed:

1. **Kimi was wired to the Anthropic adapter**, but Kimi's Anthropic-compatible
   endpoint doesn't reliably stream thinking content blocks. Moonshot's OpenAI
   surface is their primary, fully-tested API. Switched the `kimi` provider
   shape from `"anthropic"` to `"openai"`. Base URL unchanged
   (`https://api.kimi.com/coding/v1`); the OpenAI adapter appends
   `/chat/completions`, which matches.

2. **The OpenAI adapter only deserialized `reasoning` in streaming deltas**,
   but Kimi K3 (and DeepSeek) emit reasoning under `reasoning_content`. Added
   `#[serde(alias = "reasoning_content")]` to the `Delta.reasoning` field so
   both field names are accepted. The outbound path already wrote both names.

## Changes

- `crates/mew-config/src/lib.rs`: kimi provider `shape` → `"openai"`, comment
  updated, test assertion updated.
- `crates/mew/src/setup/providers.rs`: removed `"kimi"` from the anthropic arm
  of `provider_name_to_shape` (falls through to default `"openai"`), shape test
  and fallback description updated.
- `crates/mew-provider-openai/src/lib.rs`: `#[serde(alias = "reasoning_content")]`
  on `Delta.reasoning`. New test `test_fixture_reasoning_content_alias` with
  fixture `src/testdata/reasoning-content.sse` verifying reasoning is captured
  from `reasoning_content` deltas.

## Additional fix: model picker Right-arrow now switches the model

When pressing Right on a model in the picker to open the thinking variant
picker, the agent's active model wasn't switched. So `set_thinking` resolved
the variant against the *old* model, failing with "unknown thinking variant
for model". Now Right fires a `SwitchModel` action alongside opening the
variant picker; since actions are processed sequentially, the model switch
completes before the user selects a variant.

- `crates/mew-tui/src/events.rs`: `handle_picker_key` Right handler returns
  `Action::SwitchModel(id)` instead of `None`.

## Additional feature: "Recent Models" section in model picker

The model picker now shows a "Recent" section at the top with up to 6
previously used models, followed by an "All Models" section with the full
list. Recent models are persisted in `state.toml` and survive restarts.

- `crates/mew-config/src/lib.rs`: added `recent_models: Vec<String>` to `State`.
- `crates/mew-tui/src/app/mod.rs`: added `recent_models` field to `App`,
  `header` field to `PickerItem` (with `Default` derive), `move_selection()`
  on `PickerState` to skip headers, and updated `filtered()` to hide headers
  when a filter is active.
- `crates/mew-tui/src/app/pickers.rs`: `open_model_picker` prepends recent
  models with section headers; `picker_up`/`picker_down` use `move_selection`.
- `crates/mew-tui/src/ui/overlays.rs`: section headers render as muted, dimmed,
  non-selectable lines.
- `crates/mew/src/runtime/dispatch.rs`: `handle_switch_model` records the
  switched model in `recent_models` (move to front, dedupe, cap at 6) and
  persists to state.
- `crates/mew/src/commands/tui.rs`: loads `recent_models` from state at
  startup in both daemon and standalone modes.
- 6 new tests covering recent section rendering, empty state, unknown model
  filtering, header-skipping navigation, and filter behavior.

## Verification

- `cargo test -p mew-provider-openai` — 7/7 pass (including new test).
- `cargo test -p mew-config test_default_kimi_provider` — pass.
- `cargo test -p mew setup::providers::tests` — 45/45 pass.
- `cargo test -p mew-tui` — 156/156 pass + 5 golden tests pass.
- `cargo clippy -p mew-provider-openai -p mew-config -p mew-tui` — clean.
- Pre-existing clippy dead-code error in `crates/mew/src/commands/daemon.rs`
  (`remote_invite_payload`) is unrelated to this change.

---

# 2026-07-18 — Surface real credential diagnostics instead of bare "get credential"

## Summary

Running `mew` with no credential env vars but a `credentials.json` present
failed with an unhelpful two-line error:

```
Error: build provider

Caused by:
    get credential
```

Root cause: `build_direct_provider` called `get_credential(...).ok()`,
discarding the rich `CredentialNotFound` error that `get_credential` returns
(an error that already names the exact env var, the keyring command, and the
`credentials.json` path). Each API-key shape arm then replaced the swallowed
error with a bare `.context("get credential")?`, producing a causeless
"get credential" string with no diagnostic. The user couldn't tell whether
the lookup missed the key, read a malformed file, or referenced the wrong
`credential_ref`.

Fix: stop swallowing the error. `get_credential` now returns the `Result`
directly into the match. The `openai` and `anthropic` arms propagate it with
`?`, so the full `CredentialNotFound` message reaches the user. The
`responses` arm keeps the OAuth-first behavior (a missing API key is
non-fatal there) but captures the credential error message and appends it to
the final "no credentials for codex" error when every auth path fails.

## Changes

- `crates/mew/src/setup/providers.rs`: `build_direct_provider` holds the
  `Result<String, ConfigError>` instead of `.ok()`; `openai`/`anthropic`
  arms use `?` to propagate the cause; `responses` arm captures the error
  message for the all-paths-failed diagnostic.
- Added two tests guarding the regression for both `openai` and `anthropic`
  shapes: `build_provider_missing_credential_surfaces_diagnostic` and
  `build_provider_missing_credential_anthropic_shape_surfaces_diagnostic`.
  They assert the error chain contains the env var name, "credentials.json",
  and "credential not found" — the swallowed-error regression produced none
  of these.

## Verification

- `cargo test -p mew --bin mew setup::providers::tests` — 44 passed.
- `cargo clippy -p mew --all-targets -- -D warnings` — clean.
- `cargo fmt -p mew -- --check` — clean.
- `cargo test -p mew-config` — 118 passed (no downstream breakage).

## k3 reasoning verification

Traced the full Kimi K3 thinking path end-to-end and confirmed it is correct:

1. Catalog (`mew-catalog`) produces variants `low`/`high`/`max` for any model
   id containing `k3`, each with `params: {"reasoning_effort": <effort>}`.
   `default_thinking` selects `high`.
2. `resolve_reasoning` returns the variant's params as a `ReasoningConfig`.
3. The agent clones it into the `Request.reasoning` field each turn.
4. The Anthropic adapter's `build_request_body` iterates `reasoning.params`
   and inserts each key at the top level of the JSON body — so
   `reasoning_effort` lands top-level, which is where Kimi's API reads it.
5. The `thinking.budget_tokens` bump does not fire for k3 (no such key), so
   no Anthropic-style thinking block is injected.

Added `test_anthropic_adapter_forwards_reasoning_effort_top_level` in
`mew-provider-anthropic` to lock in steps 4–5: asserts `reasoning_effort`
appears top-level in the wire body and that no `thinking` object is injected.

## Kimi tool-call ID sanitization

Kimi's Anthropic-compatible API emits tool-call IDs containing spaces and
colons (e.g. `"handoff plan:29"` — tool name + space + colon + counter).
Anthropic's `tool_use` ID format rules reject these on the next-turn replay,
producing:

```
an assistant message with 'toolcalls' must be followed by tool messages
responding to each 'toolcallid'. The following toolcallids did not have
response messages: handoff plan:29
```

The API generates the non-conformant ID itself, then rejects its own ID when
we replay it. Both mew adapters previously took the provider's call ID
verbatim with no sanitization.

Fix: in the Anthropic adapter's `tool_use` ingest arm, replace every incoming
`content_block.id` with a fresh `toolu_`-prefixed ULID before storing it in
`ToolCallPart.call_id`. The fresh ID round-trips consistently — the agent
matches tool results to calls by `call_id`, and `build_request_body`
serializes both `tool_use.id` and `tool_result.tool_use_id` from that same
field, so the pair always matches.

The OpenAI adapter is unaffected (Kimi uses the `anthropic` shape).

### Changes

- `crates/mew-provider-anthropic/src/lib.rs`: `tool_use` ingest arm now
  assigns `call_id: format!("toolu_{}", ulid::Ulid::new())` instead of
  copying `event.content_block.id`. The `ContentBlock.id` field is still
  deserialized (for raw-dump mode) but marked `#[allow(dead_code)]`.
- `crates/mew-provider-anthropic/src/testdata/tool-call-nonconformant-id.sse`:
  fixture with a `"handoff plan:29"` tool-call ID.
- `test_fixture_tool_call_nonconformant_id_is_sanitized`: asserts the
  resulting `call_id` is `toolu_`-prefixed and contains no spaces or colons.

### Verification

- `cargo test -p mew-provider-anthropic` — 16 passed.
- `cargo clippy -p mew-provider-anthropic --all-targets -- -D warnings` —
  clean.
- `cargo fmt -p mew-provider-anthropic -- --check` — clean.
- `cargo build -p mew` — clean (one pre-existing unrelated warning in
  `daemon.rs::remote_invite_payload`).

## Shift+Tab / Ctrl+Shift+Tab persona cycling

Added keyboard cycling for personas in the TUI:

- **Shift+Tab** — cycle forward through loaded personas, wrapping through
  "default" (no persona) at the end.
- **Ctrl+Shift+Tab** — cycle backward.

Terminals deliver Shift+Tab as `BackTab` and Ctrl+Shift+Tab as `BackTab` with
the `CONTROL` modifier. The normal-mode key handler maps these to
`Action::CyclePersona(+1)` / `Action::CyclePersona(-1)`, which dispatches
through `handle_cycle_persona` → `handle_switch_persona` — reusing the
existing switch path so model pinning, accent color, and the synthetic
display message all fire identically to `/persona <name>`.

The keybinding is suppressed when the input box has text (BackTab is a no-op
there anyway) and in slash-command mode (where Tab does completion). When no
personas are loaded, it sets an "no personas loaded" alert.

### Changes

- `crates/mew-tui/src/events.rs`: `CyclePersona(i32)` Action variant;
  `BackTab` handler in `handle_normal_key`.
- `crates/mew/src/runtime/dispatch.rs`: `handle_cycle_persona` computes the
  next persona from `app.personas` + `app.active_persona` and dispatches
  through `handle_switch_persona`.
- `crates/mew-tui/src/harness.rs`: `parse_key` now supports `shift+` prefix
  and maps `shift+tab` / `ctrl+shift+tab` to `BackTab` for test input.
- `crates/mew/src/dispatch_table_tests.rs`: `CyclePersona` arm in the
  variant table; four new tests covering forward, wrap-to-default,
  backward-from-default, and empty-list alert.
- `crates/mew-tui/src/ui/overlays.rs`: help overlay entry for Shift+Tab.

### Verification

- `cargo test -p mew --bin mew dispatch_table_tests` — 12 passed.
- `cargo test -p mew-tui` — 5 passed + 1 doc test.
- `cargo clippy -p mew-tui --all-targets -- -D warnings` — clean.
- `cargo fmt -p mew -p mew-tui -- --check` — clean.

## Fix: Right key not opening thinking variant picker from model picker

Pressing Right in the model picker to open the thinking variant picker was
silently failing. Two bugs:

1. **Key mismatch (standalone mode):** The model picker uses `provider/model`
   format IDs (e.g. `opencode-zen/claude-sonnet-4-6`), but the
   `thinking_variants` HashMap is keyed by the bare model id (e.g.
   `claude-sonnet-4-6`). The Right-key handler's `contains_key(&selected.id)`
   always missed because of the provider prefix.

2. **Daemon mode never populated:** `ModelList` messages from the daemon were
   forwarded to the notification channel but never parsed into `app.models`
   or `app.thinking_variants`. The model picker was empty in daemon mode.

### Changes

- `crates/mew-tui/src/app/pickers.rs`: Added `open_thinking_variant_picker_for`
  which accepts an optional model id (in `provider/model` format), strips the
  provider prefix via `rsplit('/')`, and looks up that model's variants — not
  the current active model's. The old `open_thinking_variant_picker` delegates
  to it with `None` (uses `self.status.model`).
- `crates/mew-tui/src/events.rs`: Right-key handler now strips the provider
  prefix before the `contains_key` check, and passes the selected model id to
  `open_thinking_variant_picker_for` so the picker shows variants for the
  highlighted model, not the current one.
- `crates/mew-tui/src/app/mod.rs`: `apply_daemon_notification` now handles
  `ModelList` — populates `app.models` (picker items) and
  `app.thinking_variants` (keyed by bare model id from `ModelInfo.model`).
- `crates/mew-daemon/src/client.rs`: Added `list_models()` method.
- `crates/mew/src/commands/tui.rs`: Daemon-mode TUI now calls
  `client.list_models()` on startup.
- Tests: `test_thinking_variant_picker_strips_provider_prefix` and
  `test_thinking_variant_picker_for_bare_model_id`.

### Verification

- `cargo test -p mew-tui` — 7 passed (5 existing + 2 new).
- `cargo test -p mew-daemon` — 5 passed.
- `cargo test -p mew-protocol` — 106 passed.
- `cargo clippy -p mew-tui --all-targets -- -D warnings` — clean.
- `cargo fmt -p mew-tui -- --check` — clean.

# 2026-07-17 — Fix dev CEF Mach-port rendezvous by anchoring a main bundle

## Summary

The Tauri dev app spawned CEF helpers that died in a loop with
`bootstrap_look_up org.cef.framework.MachPortRendezvousServer.<pid>: Unknown
service name` followed by `Network service crashed or was terminated,
restarting service`. Root cause: Chromium names the browser's Mach rendezvous
server `<main bundle id>.MachPortRendezvousServer.<pid>` and helpers derive
the same name from their own main bundle. The unbundled dev executable has no
bundle identifier (the server registered as `.MachPortRendezvousServer.<pid>`,
verified with `bootstrap_look_up` probes), while helpers resolve the CEF
framework identifier `org.cef.framework`, so every lookup missed and each
helper terminated on startup.

Fix: point CEF at a real bundle for dev. `scripts/prepare-cef.mjs` now writes
a synthetic `src-tauri/target/debug/mew.app` (Info.plist with the
`ai.mew.mew` identifier) next to the dev executable, and the CEF host sets
`main_bundle_path` to it plus an explicit `framework_dir_path`. CEF appends
both as command-line switches and propagates them to every helper, so browser
and helpers agree on one rendezvous name. Verified in the dev app: browser
registers `ai.mew.mew.MachPortRendezvousServer.<pid>`, helpers stay alive,
CDP answers on 9223, zero rendezvous errors.

## Changes

- `native/cef-host/src/embed.rs`: set `Settings.main_bundle_path` (new
  `main_bundle_path()` resolver: `MEW_CEF_MAIN_BUNDLE_PATH` env override,
  nothing when running packaged, else exe-adjacent `mew.app`) and
  `Settings.framework_dir_path` (parent dir of the resolved framework).
- `mew-web-ui/scripts/prepare-cef.mjs`: write the synthetic development
  bundle `src-tauri/target/debug/mew.app/Contents/Info.plist` on every run.
- `mew-web-ui/src-tauri/src/lib.rs`: also print the CEF fallback reason to
  stderr; the desktop binary installs no tracing subscriber, so
  `tracing::warn` made CEF init failures invisible.
- `native/cef-host/README.md`: document the rendezvous/bundle relationship
  and the new `MEW_CEF_MAIN_BUNDLE_PATH` override; correct the dev prepare
  description (copies by default, `--link` symlinks).

## Notes

- Packaged (release) runs are unchanged: the resolver returns nothing when
  the executable lives inside a real `.app`, so CEF derives the bundle from
  the executable location as before.
- `cargo clippy --all-targets -- -D warnings` previously failed on five
  pre-existing lints in `native/cef-host`; these are now fixed (collapsed
  nested `if let` into let-chains, moved a `pub use` ahead of the test
  module), and the crate and `src-tauri` are both clippy-clean.
- Helpers no longer strictly need the `binaries/Resources` copy for ICU/pak
  files since `framework_dir_path` is propagated, but the copy is kept as a
  fallback.
- Force-killing the dev app leaves CEF's `SingletonLock`/`SingletonSocket`
  in the cache profile, which can make the next launch silently skip CEF
  initialization (WKWebView fallback). The new stderr message surfaces that
  failure; clearing `~/Library/Application Support/ai.mew.mew/cef-desktop-cache`
  recovers.
- Production sandbox/helper `.app` layout under `Contents/Frameworks` remains
  separate hardening, as before.

# 2026-07-17 — Add Kimi (Moonshot AI) provider

## Summary

Added Kimi as a built-in provider using the Anthropic-compatible endpoint.
Kimi serves three models: `k3` (Kimi K3, thinking-capable), `kimi-for-coding`
(Kimi K2.7 Code), and `kimi-for-coding-highspeed` (Kimi K2.7 Code HighSpeed).
K3 supports low/high/max thinking effort via top-level `reasoning_effort`.

## Changes

- `crates/mew-config/src/lib.rs`: Added `kimi` to `Config::default()` with
  `shape = "anthropic"`, `base_url = "https://api.kimi.com/coding/v1"`,
  `credential_ref = "kimi"`. Added `test_default_kimi_provider` and updated
  `test_load_default_when_missing`.
- `crates/mew/src/setup/providers.rs`: Added `"kimi"` to the anthropic arm of
  `provider_name_to_shape()`. Added kimi/k3 to the `discover_models()`
  fallback list gated on credential presence. Updated
  `provider_name_to_shape_known` test.
- `crates/mew-catalog/src/lib.rs`: Added k3 thinking variants (low/high/max
  via `reasoning_effort`) in `builtin_thinking_variants()` before the `kimi`
  catch-all that returns empty.

## Notes

- The `mew` binary does not compile due to a pre-existing error in
  `crates/mew/src/commands/tui_capture.rs:1142` — a non-exhaustive match on
  `ServerMessage` missing `BrowserSnapshot`/`BrowserScreenshot`/`BrowserState`
  variants from in-progress browser work. Not caused by these changes.
- `mew-config` and `mew-catalog` compile and pass all tests (37/37 catalog,
  2/2 config kimi tests).

# 2026-07-14 — Rework mew theming to a flat, aliased token table

## Summary

Replaced the hardcoded `ThemeTokens` struct with a flat, aliased token table
loaded from a single shared manifest. Built a `theme_codegen` binary that emits
the web UI CSS, Rust default/overrides, and a syntect `.tmTheme` from the same
manifest. Migrated TUI color usage to `Theme::resolve`, updated
`ratatui-mdstream` to consume the shared theme and generated syntect theme, and
migrated all 21 web UI themes into the manifest.

## Changes

- `crates/mew-tui/resources/theme_manifest.json`: new single source of truth
  with base tokens, aliases, and 21 selectable themes as sparse overrides.
- `crates/mew-tui/src/bin/theme_codegen.rs`: codegen binary producing
  `mew-web-ui/src/generated-themes.css`,
  `crates/mew-tui/src/theme_generated.rs`, and
  `crates/ratatui-mdstream/resources/theme.tmTheme`.
- `crates/mew-tui/src/theme.rs`: `Theme` backed by `HashMap<String, Color>`,
  `resolve`, `ansi`, `with_persona_accent`, `css_variables`, and manifest
  validation for custom theme files.
- `crates/mew-tui/src/ui/*.rs`, `settings.rs`: migrated `Color::` usage to token
  lookups.
- `crates/ratatui-mdstream/src/*.rs`: consumes the shared token table and the
  generated `.tmTheme` instead of hardcoded colors / `base16-ocean.dark`.
- `mew-web-ui/src/index.css`: imports `generated-themes.css` before the
  `@theme inline` mapping; hand-written theme blocks removed.
- `crates/mew/src/cli.rs` + `commands/theme.rs`: added `mew theme export-css`
  command.
- `crates/mew/tests/theme_install.rs`: integration tests for validation and
  install behavior.
- `docs/THEMING.md`: new design and vocabulary reference.
- `justfile`: `theme-codegen` and `theme-codegen-check` recipes wired into
  `just ci`.

## UI polish follow-up

- `crates/mew-tui/src/ui/input.rs`: input bar now uses `muted` token for its
  background instead of `status_bar.background`.
- `crates/mew-tui/src/ui/mod.rs`: removed the 1-cell divider line between chat
  and input; separation now comes from the muted input background.
- `crates/mew-tui/src/ui/status.rs`: removed the now-unused `draw_divider`
  function.
- Golden frames in `crates/mew-tui/tests/golden/` updated to reflect the removed
  divider line.
- `crates/mew-tui/src/app/mod.rs`: status-bar marquee tick interval halved
  from 300ms to 150ms, so overflow pill text scrolls roughly twice as fast.

## Verification

- `cargo clippy --all -- -D warnings` — clean
- `cargo test -p mew-tui` — passes after golden update
- Visual inspection via `mew tui-capture`: input bar shows muted background and
  no divider line.
- `just theme-codegen-check` — generated files up-to-date
- `pnpm build` in `mew-web-ui` — succeeds

---

# 2026-07-14 — Fix slow rasterization by caching the font system

## Summary

Root-caused the slow daemon capture: `mew_raster::rasterize` was creating a
fresh `cosmic_text::FontSystem` for every single frame. Font system creation is
expensive, so captures were taking ~1s per frame and producing very short
videos. Added a reusable `Rasterizer` that caches the font system and updated all
call sites to use it. Also switched shaping from `Advanced` to `Basic` and split
the capture timing log into draw vs rasterize time.

## Changes

- `crates/mew-raster/src/lib.rs`:
  - New `Rasterizer` struct that owns the `FontSystem` and `SwashCache`.
  - `Rasterizer::new()` builds the font system once with the bundled
    IoskeleyMono fonts.
  - `Rasterizer::rasterize()` and `Rasterizer::to_png()` are the cached
    equivalents of the old free functions.
  - The old free `rasterize()` and `to_png()` functions remain as convenience
    wrappers that create a one-off `Rasterizer` for callers that only need a
    single frame.
  - Switched `Shaping::Advanced` to `Shaping::Basic` for faster monospace text
    rendering.

- `crates/mew/src/commands/tui_capture.rs`:
  - `DaemonBackend` now owns a `Rasterizer` and uses it for both frame capture
    and screenshot PNG encoding.
  - Per-frame log now reports `draw_ms` and `rasterize_ms` separately.

- `crates/mew-tui/src/harness.rs`:
  - `LocalBackend` now owns a `Rasterizer` and uses it for frame capture and
    screenshots.

## CI Gate

- `cargo build` — clean
- `cargo test -p mew-raster` — 10 tests + 1 doc-test pass
- `cargo test -p mew-tui harness` — 15 tests pass
- `cargo test -p mew tui_capture` — 5 tests pass
- `cargo clippy -p mew-raster -- -D warnings` — clean
- `cargo clippy -p mew-tui -p mew -- -D warnings` — blocked by an unrelated
  `theme.rs` clippy warning (`manual_strip`) that is not part of these changes

---

# 2026-07-13 — Real-time streaming, flushing, tracing, and fast typing in daemon `tui-capture`

## Summary

Made daemon-connected `mew tui-capture --connect` record thinking and streaming
output progressively, keep pace with the real stream, print output incrementally
instead of buffering it all until exit, added tracing logs at the major flow
points, and fixed the long-prompt typing stall by stopping per-keystroke frame
rasterization.

## Changes

- `crates/mew/src/commands/tui_capture.rs`:
  - `DaemonBackend::send_text` no longer rasterizes a frame after every
    character; it types the whole prompt, then captures one frame. This fixes
    the "really long time" at `send_text: typing prompt` for long prompts.
  - `DaemonBackend::type_str` uses the same fast path (one frame after the full
    string).
  - `DaemonBackend::wait_turn` applies a streaming drain limit of 4
    `AgentEvent`s per frame, the same limit used by the live TUI drain loop.
  - `wait_turn` no longer sleeps after every batch when more events are already
    available, so it keeps pace with the actual stream instead of artificially
    slowing down the capture.
  - Drawing/capturing only happens when a 60 fps frame is due; the sleep is now
    reserved for idle waits when no events are ready.
  - `send_text` skips its 5 ms sleep once `app.streaming` is true.
  - `run_script_daemon` now prints each verb's output immediately and flushes
    stdout, rather than accumulating the entire script's output and printing it
    all at the end.
  - Added `tracing` logs: command start/finish, daemon connect/session ready,
    each script verb, `send`/`wait_turn` lifecycle, frame capture counts,
    per-frame capture timing, typing timing, and failure paths.
  - Added `agent_event_name` and `server_message_type` helpers so trace logs can
    record which event variant arrived without exposing channel payloads.

- `crates/mew-tui/src/harness.rs`:
  - `LocalBackend::type_str` also stops per-character captures and captures one
    frame after the full string, keeping harness mode consistent.

## Notes

- A reported symptom (UI shows "thought for 41.2s" but video is only 10s)
  pointed to slow frame rasterization. This was root-caused and fixed in the
  following entry by caching the `mew_raster` font system.

---

# 2026-07-13 — Daemon-connected `mew tui-capture --connect`

## Summary

Added a real-daemon capture path to `mew tui-capture`. Scripts can now drive
a live mew daemon (e.g. `mew daemon --fake-provider`) headlessly, capturing
true chat/turn behavior — streaming, tool calls, subagents, session rail —
with the same rasterized PNG/text output pipeline.

## Changes

- `crates/mew-tui/src/harness.rs`:
  - New `Backend` trait so the verb interpreter can drive pluggable backends.
  - Existing behavior moved into `LocalBackend`.
  - `Harness` is now generic over `Backend`, defaulting to `LocalBackend`; the
    public API (`h.app`, `h.actions`, `run_script`, etc.) is preserved.
  - Local-only verbs (`say`, `error`, `settings`, `settings_config`) now
    produce a clear error when used with a non-local backend.
  - `parse_key` made public for reuse by the daemon interpreter.

- `crates/mew/src/commands/tui_capture.rs`:
  - New `DaemonBackend` that connects to a mew daemon via `DaemonClient`,
    creates a session, and pumps `AgentEvent`s / `ServerMessage`s into a
    headless `App` + `TestBackend`.
  - New async-aware script verbs for daemon mode: `send`, `wait_turn`, `expect`,
    `screenshot_dir`.
  - New `run_script_daemon` / `run_interactive_daemon` paths.
  - Screenshot/MP4 encoding work in daemon mode the same way as harness mode.

- `crates/mew/src/cli.rs` and `crates/mew/src/main.rs`:
  - Added `--connect <url>` to `TuiCapture`.
  - Made `tui_capture::run` async and wired `.await`.

- `crates/mew/Cargo.toml`:
  - Added `mew-raster`, `tiny-skia`, `png` dependencies for the daemon backend.

- `.mew/skills/tui-capture/SKILL.md`:
  - Documented `--connect` and the daemon-mode verbs.
  - Added a daemon capture example and updated the comparison table.

## Tests

- `test_daemon_capture_fake_provider_end_to_end` — starts an in-process daemon
  with `FakeProvider`, runs `send` / `wait_turn` / `expect`, and verifies the
  response appears in the output.
- `test_daemon_capture_expect_fails_when_missing` — verifies `expect` reports a
  clear error when the expected text is absent.
- `test_daemon_capture_screenshot_dir_writes_png` — verifies numbered PNGs are
  written when `screenshot_dir` is set in daemon mode.

## CI Gate

- `cargo clippy -p mew -p mew-tui -p mew-daemon -- -D warnings` — clean
- `cargo test -p mew -p mew-tui` — 154+ tests pass (mew-tui harness tests +
  new daemon capture tests)
- `cargo test --all` — one pre-existing `mew-daemon` concurrency test
  (`slash_command_during_in_flight_turn_does_not_block_stream`) is failing
  independently of these changes; all other tests pass.

---


# 2026-07-13 — Real-provider `mew tui-capture --connect` improvements

## Summary

Improved daemon-connected `mew tui-capture` while recording a real `umans`
demo: streaming frames are now captured during `wait_turn`, the TUI status bar
shows the real model/provider, and a dev doc was added.

## Changes

- `crates/mew/src/commands/tui_capture.rs`:
  - `DaemonBackend::wait_turn` now draws and captures a frame roughly every
    100 ms while `app.streaming` is true, so recorded MP4s show the response
    appearing progressively instead of jumping straight to the final frame.
  - `DaemonBackend::connect` reads `model`/`provider` from
    `ServerMessage::SessionReady` and sets `app.status.model`/`provider`, so the
    status bar shows the real backend (e.g. `umans/umans-coder`) instead of
    `mewd/daemon`.
  - Replaced `wait_for_session_id` with `wait_for_session_ready`.

- `crates/mew-daemon/src/client.rs`:
  - `SessionReady` is now forwarded to `notify_tx` so callers can read the
    active model/provider from it.

- `docs/development/dev-tui-capture.md`:
  - New dev doc explaining how to record real-provider captures with
    `mew tui-capture --connect`, including daemon setup, script verbs, tips,
    and troubleshooting.

- `docs/development/dev-tui.md`:
  - Added a cross-reference to the new dev-tui-capture doc.

## Captures produced

- `notes/capture/umans-optimized-reverse-binary-string-demo.mp4`
- `notes/capture/umans-optimized-reverse-binary-string-final.png`

Script used:

```text
send "What's the optimised way of reversing a binary string in Rust?"
wait_turn 120000
expect "reverse"
pause 4000
screenshot /tmp/umans-optimized-final.png
```

Command:

```bash
export MEW_CRED_UMANS=$(grep MEW_CRED_UMANS .env | cut -d= -f2-)
./target/debug/mew daemon --provider umans --model umans/umans-coder \
  --port 127.0.0.1:0 --background --log /tmp/mew-capture.log
./target/debug/mew tui-capture \
  --script /tmp/capture-umans-optimized.txt \
  --connect ws://127.0.0.1:<port> \
  --mp4 notes/capture/umans-optimized-reverse-binary-string-demo.mp4 \
  --width 100 --height 30
./target/debug/mew daemon --stop
```

## CI Gate

- `cargo clippy -p mew -p mew-daemon -- -D warnings` — clean
- `cargo test -p mew tui_capture` — 5 tests pass

---

# 2026-07-13 — Settings overlay capture verb

## Summary

Wired the in-app settings overlay into the TUI harness so it can be captured
deterministically with `mew tui-capture`.

## Changes

- `crates/mew-tui/src/harness.rs`:
  - New `settings` verb — opens the settings overlay with the default config.
  - New `settings_config <path>` verb — opens the overlay loaded from a controlled
    TOML file, making captures independent of the user's real `config.toml`.
  - Updated `help` output to list the new verbs.
  - Added 4 tests covering both verbs, missing-path handling, and help text.
- `.mew/skills/tui-capture/SKILL.md`:
  - Documented `settings` and `settings_config` in the verb table.
  - Added a settings-overlay example script.

## Captures produced

Ran:

```bash
MEW_CRED_Z_AI=fake ./target/debug/mew tui-capture \
  --script notes/settings-capture-zai.txt --width 100 --height 30 \
  > notes/images/settings-capture-zai-output.txt
```

- `notes/images/settings-capture-zai-output.txt` — text snapshots of each frame
- `notes/images/settings-only-zai.png` — settings overlay open
- `notes/images/settings-accounts-only-zai.png` — Accounts category selected
- `notes/images/settings-zai-details.png` — z-ai account details panel

Generated files live in `notes/images/`, which is gitignored.

## CI Gate

- `cargo clippy -p mew-tui -- -D warnings` — clean
- `cargo test -p mew-tui harness` — 15 tests pass
- `cargo fmt` — clean

## Overview

Implemented the full "mew films itself" plan from `notes/mew-tui-self-capture-plan.md`:
the agent can now capture screenshots and record videos of mew's own TUI — both
as human-facing artifacts (demo mp4s/gifs) and as agent-facing feedback (PNG images
for VLM inspection).

## Phase 0 — vhs skill (done)

Created `.mew/skills/tui-capture/SKILL.md` teaching the agent to use charm vhs to
record the real mew binary in a pty. Verified end-to-end: built mew, started
fake-provider daemon, ran a .tape file through vhs producing valid mp4 + png.

## Phase 1 — buffer→png rasterizer (done)

New crate `crates/mew-raster`:
- `rasterize(buf: &Buffer, opts: &RasterOptions) -> Pixmap` — converts ratatui Buffer to pixels
- `to_png(buf: &Buffer, opts: &RasterOptions) -> Vec<u8>` — encodes as PNG bytes
- Bundles IoskeleyMono-Regular.ttf + IoskeleyMono-Medium.ttf via `include_bytes!`
- Full color mapping: ratatui Color enum (named, Rgb, Indexed 256-color) → RGB
- Style modifiers: BOLD (bold font weight), REVERSED (swap fg/bg), UNDERLINED
- RasterOptions with scale (default 2× = 16×32px cells), bg/fg colors
- 10 tests covering dimensions, scaling, colors, reversed modifier, PNG validity

Harness integration (`crates/mew-tui/src/harness.rs`):
- `Harness::screenshot(path)` method — renders current frame to PNG
- `screenshot <path>` verb in the script format

## Phase 2 — video from the harness (done)

Frame recording on `Harness`:
- `start_recording()` / `stop_recording()` — capture frames after each verb
- `capture_frame()` — rasterize current buffer into Pixmap (no-op when not recording)
- `duplicate_last_frame(count)` — clone last frame N times for timing
- `encode_mp4(path, fps)` — write numbered PNGs to temp dir, shell to ffmpeg
- Automatic frame capture: `type_str` emits one frame per keystroke, `say` emits
  one frame per 8-char delta chunk (streaming animation)

New script verbs: `start_recording`, `stop_recording`, `pause <ms>`, `record <path> [fps]`

5 new tests. All 12 harness tests + golden tests pass.

## Phase 3 — expose to the agent (done)

New `mew tui-capture` subcommand (`crates/mew/src/commands/tui_capture.rs` + CLI variant):
- `--script <path>` — reads a harness script file
- `--mp4 <path>` — auto-wraps script in recording/encoding
- `--fps <n>` — framerate (default 30)
- `--width` / `--height` — terminal dimensions (default 80×24)
- No provider needed — doesn't trigger state health check

Updated `.mew/skills/tui-capture/SKILL.md` with both methods documented:
`mew tui-capture` (deterministic) and `vhs` (glamour shots), with comparison table.

## Phase 4 — a/b legibility experiment (done)

Rendered the same TUI frame three ways: vhs screenshot, rasterizer PNG, text dump.
Sent to two VLMs (umans/umans-coder and minimax/MiniMax-M3).

Results:
- Both methods legible to VLMs (both models could read all text)
- Rasterizer scored higher on average (8.0 vs 7.0 across both models)
- Rasterizer wins on determinism (vhs captured wrong frame on first attempt)
- vhs wins on visual fidelity (real terminal chrome)
- Text dump remains most reliable for structural analysis

## cosmic-text renderer upgrade (done)

Replaced ab_glyph with cosmic-text for glyph rendering:
- Uses `cosmic-text`'s `Buffer::draw()` API with `swash` for proper glyph shaping
- Fixes: box-drawing chars (solid divider lines instead of broken/dotted), multi-cell
  graphemes, bold/italic via cosmic-text Attrs system
- VLM comparison after upgrade: minimax legibility jumped from 7/10 → 9.5/10
- Divider line artifact (faint dotted rules) eliminated — confirmed by both VLMs

## Files created/modified

New files:
- `crates/mew-raster/Cargo.toml`
- `crates/mew-raster/src/lib.rs`
- `crates/mew-raster/assets/IoskeleyMono-Regular.ttf`
- `crates/mew-raster/assets/IoskeleyMono-Medium.ttf`
- `crates/mew/src/commands/tui_capture.rs`
- `.mew/skills/tui-capture/SKILL.md`

Modified files:
- `Cargo.toml` — added `mew-raster` to workspace members
- `crates/mew-tui/Cargo.toml` — added `mew-raster`, `tiny-skia`, `png`, `tempfile` deps
- `crates/mew-tui/src/harness.rs` — screenshot verb, frame recording, video encoding
- `crates/mew/src/cli.rs` — `TuiCapture` subcommand
- `crates/mew/src/main.rs` — dispatch arm
- `crates/mew/src/commands/mod.rs` — module declaration

## CI Gate

- `cargo clippy -p mew-raster -p mew-tui -p mew -- -D warnings` — clean
- `cargo test -p mew-raster` — 10 tests + 1 doctest pass
- `cargo test -p mew-tui` — 12 harness tests + 5 golden tests + 1 doctest pass
- End-to-end: `mew tui-capture --script capture.txt --mp4 demo.mp4` — valid MP4 produced

---

# 2026-07-12 — Context Window Inspector: Steps 7, 8, 9

## Step 8 — Calibration Harness (done)

Added `#[cfg(test)]` calibration module at the end of `crates/mew-agent/src/manifest.rs`.
Uses direct `assert_eq!` equality assertions against known-exact token counts
for `cl100k_base` and `o200k_base` encodings. No `DriftEntry` struct or ratio
tracking.

- 6 tests: `test_calibration_cl100k_short_string`, `test_calibration_cl100k_long_text`,
  `test_calibration_o200k_short_string`, `test_calibration_o200k_code_snippet`,
  `test_calibration_json_schema_text`, `test_model_encoding_routing`.
- Independently verified baseline: "Hello world" = 2 tokens for cl100k_base.
- JSON fixture distinguishes cl100k (36) from o200k (38) for routing tests.

## Step 9 — Subagent Manifests (done)

Threaded manifests from child agent → `SubagentResult::Complete` →
`ToolStateCompleted.metadata` → wire protocol → web client/store.

### Changes by file

- `crates/mew-subagents/src/lib.rs` — Added `manifests: Vec<TurnManifest>` to
  `SubagentResult::Complete` and `SubagentEvent::Finished`.
- `crates/mew-agent/src/runner.rs` — Added `extract_manifests` helper.
  Threaded manifests through Cancelled, Error, and Completed paths (both
  `SubagentEvent::Finished` and `SubagentResult::Complete`).
- `crates/mew-agent/src/tools.rs` — Updated 3 destructure sites
  (`execute_subagent_call`, `execute_subagent_start`, `execute_subagent_wait`)
  to capture manifests and store on `ToolStateCompleted.metadata` as JSON.
- `crates/mew-agent/src/manifest.rs` — Added `tool_call_label` helper and
  subagent label detection (`"subagent: {name}"` instead of `"tool: subagent"`).
  Updated `part_label_kind` and `build_part_segment`.
- `crates/mew-agent/src/lib.rs` — Added `manifests: Vec<TurnManifest>` to
  `AgentEvent::SubagentEnd`. Updated manual Debug impl (uses `..` to omit).
- `crates/mew-agent/src/agent.rs` — Threaded manifests through async subagent
  pump's `SubagentEvent::Finished` → `AgentEvent::SubagentEnd`.
- `crates/mew-protocol/src/lib.rs` — Added `manifests: Vec<TurnManifest>` with
  `#[serde(default)]` to `ServerMessage::SubagentEnd`. Added round-trip tests.
- `crates/mew-daemon/src/lib.rs` — Updated `translate_event` to pass manifests
  from `AgentEvent::SubagentEnd` to `ServerMessage::SubagentEnd`.
- `crates/mew-daemon/src/client.rs` — Updated `ServerMessage::SubagentEnd`
  handler to thread manifests to `AgentEvent::SubagentEnd`.
- `mew-web-client/src/index.ts` — Added `manifests?: TurnManifest[]` to
  `subagent-end` event type and wire `ServerMessage` type.
- `mew-web-ui/src/stores/session.ts` — Added `manifests: TurnManifest[]` to
  `SubagentInfo`, populated in `onSubagentEnd`, initialized in `onSubagentStart`.

### Tests added

- `test_manifest_labels_subagent_tool_calls` (mew-agent manifest)
- `subagent_end_manifests_roundtrip`, `subagent_end_no_manifests_roundtrip` (mew-protocol)

## Step 7 — Mobile Manifests (done)

### Changes by file

- `crates/mew-mobile-core/src/state.rs` — Added `MobileTurnManifest`,
  `MobileSegment`, `MobileSegmentKind`, `MobileAssistantMeta` UniFFI records.
  Added `to_mobile_manifest` conversion function. Extended `ChatMessage` with
  `assistant_meta: Option<MobileAssistantMeta>`. Updated `apply_provider_event`
  to extract manifest from `MessageEnd` and attach to last assistant message.
  Added `last_manifest` to `SessionState`. Updated all `ChatMessage` construction
  sites with `assistant_meta`.
- `crates/mew-mobile-core/src/events.rs` — Added `manifest` field to
  `CoreEvent::TurnEnded`. Added `CoreEvent::SubagentEnd` variant with
  `parent_call_id`, `child_session_id`, `outcome`, `manifests`.
- `crates/mew-mobile-core/src/lib.rs` — Updated `translate_message` to extract
  manifest from `MessageEnd` and emit in `CoreEvent::TurnEnded`. Extracted
  `SubagentEnd` from the no-op match group into its own arm that emits
  `CoreEvent::SubagentEnd`. Fixed `SessionHistory` handler to populate
  `assistant_meta` from `msg.assistant`.

### Tests added

- `test_to_mobile_manifest_preserves_fields` — verifies field mapping + source_id drop
- `test_message_end_extracts_manifest` — verifies manifest extraction + attachment to assistant message
- `test_chat_message_has_assistant_meta` — verifies ChatMessage field
- `test_message_end_no_manifest` — verifies graceful handling when manifest is None

## CI Gate

- `cargo fmt --check` — clean
- `cargo clippy -p mew-agent -p mew-subagents -p mew-protocol -p mew-mobile-core -p mew-daemon -- -D warnings` — clean
- `cargo test -p mew-agent -p mew-subagents -p mew-protocol -p mew-mobile-core` — 284 tests pass
- `pnpm build` (web-client) — clean
- `pnpm build` (web-ui) — clean
- `pnpm test` (web-ui) — 50 tests pass
- `just ios-core` (Swift binding regen) — manual step (AC.12), not run

## 2026-07-13 — mew-raster: ~190x faster frame rasterization

`Rasterizer::rasterize` went from ~410ms to ~2.2ms warm per frame (160x48, scale 2, release), measured with the new `cargo run -p mew-raster --release --example bench`.

What changed in `crates/mew-raster/src/lib.rs`:
- Replaced per-pixel `tiny_skia::fill_rect` calls (one per glyph pixel via cosmic-text's `draw()` callback) with direct blends into the pixmap buffer (`blit_glyph`). This alone: 410ms → 92ms.
- Discovered the remaining cost was cosmic-text shaping+layout per row per frame (was hidden — lazy layout inside `draw()`). Replaced per-line shaping with a per-symbol shape cache: each unique (symbol, bold, italic) is shaped once, then frames are pure mask blitting at cell origins. 92ms → 2.2ms. Also removes per-cell String allocs and per-row TextBuffer creation.
- Backgrounds: single `pixels_mut().fill()` for the canvas + merged same-bg cell spans written as row slices.
- PNG encode: `png::Compression::Fast` (104ms → 11ms per screenshot; files ~40% larger, fine for capture).
- New shared `mew_raster::encode_frames_mp4` pipes raw RGBA to ffmpeg (`-f rawvideo`) instead of writing a temp PNG per frame; both `mew-tui/src/harness.rs` and `mew/src/commands/tui_capture.rs` `encode_mp4` impls now delegate to it. Dropped the now-unused `png` dep from both crates.

Verified: mew-raster tests (10), harness tests (15), tui_capture daemon tests (5), plus e2e smoke: harness script → mp4 (ffprobe-valid h264) and PNG screenshot visually checked (bold, bg spans, box-drawing, colors all correct).

Note: capture perf logging in `wait_turn` (`rasterize_ms`) should now read ~2ms; if it's still slow, check for a debug build — the blend loops are heavily penalized without optimization.

## 2026-07-13 — mew-raster: fix tofu glyphs and block-element seams

Follow-up to the rasterizer rewrite; user screenshot showed the welcome-screen cat rendering as tofu boxes and the block "mew" wordmark with grid seams.

- Tofu: `Shaping::Basic` (`shape_skip` in cosmic-text) never font-falls-back for `Family::Monospace`, so kana/CJK punctuation missing from IoskeleyMono rendered as .notdef. Pre-existing bug, not a regression. Switched `shape_symbol` to `Shaping::Advanced` — per-glyph system-font fallback (Hiragino Sans for kana on macOS). Cost is once per unique symbol thanks to the shape cache; warm frame time unchanged (2.2ms).
- Seams: block elements (U+2580–U+259F) from the font don't cover the full cell (advance ≈15.6px < 16px cell), so per-cell placement left background lines between cells. Added `draw_block_element` — geometric fills in cell-eighths (halves, eighth-blocks, shades with alpha, quadrants), same approach as terminal emulators. Blocks now tile edge-to-edge.
- New tests: full block fills entire cell incl. corners, adjacent blocks tile without seams, half block covers exactly its half. 13 mew-raster tests pass; harness (15) and tui_capture (5) still green; welcome screen visually verified (cat + seamless wordmark).

## 2026-07-13 — tui-capture: animate spinner in daemon-mode recordings

The input-bar spinner was static in captured videos: `app.tick()` (which advances `spinner_frame` every 5th tick) is driven by the EventLoop's 60fps tick task, which the headless `DaemonBackend` never spawns. `wait_turn` now calls `app.tick()` on its own 16ms cadence, matching the real TUI's animation rate. Local harness mode is unaffected by design (turns are synchronous; `pause` duplicates frames).

Test: `test_spinner_advances_while_streaming` — long fake-provider response (~500ms stream), asserts `spinner_frame` advanced during `wait_turn`. Written failing-first, passes after the fix; all 6 tui_capture tests green.

## 2026-07-17 — Tauri desktop shell scaffold

- Added `mew-web-ui/src-tauri`, a thin Tauri 2 shell around the existing React/Vite app.
- Kept the shell outside the root Cargo workspace so desktop-only dependencies do not enter the core workspace checks.
- Added `just desktop-dev` and `just desktop-build`, plus Tauri-aware Vite settings for fixed-port dev/HMR and WebKit-compatible production builds.
- Generated the initial app icons from `mew-web-ui/public/favicon.svg`.
- Daemon supervision and the desktop host/runtime adapter are intentionally left for the next slice.

Verified: `cargo check --manifest-path mew-web-ui/src-tauri/Cargo.toml`, `pnpm --filter mew-web-ui build`, `pnpm --filter mew-web-ui exec tauri info`, and `pnpm --filter mew-web-ui desktop:build`.

## 2026-07-17 — Tauri daemon supervision and shared host bootstrap

- Added `DaemonSupervisor` to the Tauri host. It supports an explicit
  `MEW_DESKTOP_DAEMON_URL`, otherwise reserves a loopback port, launches
  `mew daemon --port ...`, waits for TCP readiness, and kills the child when
  the desktop process exits.
- Added `mew-web-ui/src/lib/host.ts`. Browser mode still derives `/ws` from the
  current origin; Tauri mode resolves the daemon URL through the native
  `daemon_ws_url` command before mounting the same React tree.
- Updated daemon startup so a stale persisted provider/model state cannot block
  a non-interactive desktop-owned daemon with an interactive healing prompt.
- Release sidecar packaging remains a follow-up. The current host discovers
  `mew` from `MEW_DESKTOP_DAEMON_BINARY`, a sibling/debug/release path, or
  `PATH`.

Verified: `pnpm --filter mew-web-ui test` (52 tests), `pnpm --filter mew-web-ui build`,
desktop-host Rust tests (4), desktop-host clippy, `cargo test -p mew` (105 tests),
and a Tauri dev smoke with a real loopback daemon and established WebSocket
connection. `cargo fmt --all -- --check` still reports unrelated pre-existing
formatting differences in several dirty files; the changed desktop and daemon
files are formatted.

## 2026-07-17 — Tauri sidecar packaging and daemon ownership

- Added the Tauri shell plugin and `bundle.externalBin` configuration for an
  architecture-specific `mew` sidecar.
- Added `mew-web-ui/scripts/build-sidecar.mjs`; `desktop:dev` prepares a debug
  sidecar and `desktop:build` prepares a release sidecar before Tauri runs.
- Made ownership explicit: `MEW_DESKTOP_DAEMON_URL` attaches without owning or
  killing the process; bundled sidecars and binaries launched through
  `MEW_DESKTOP_DAEMON_BINARY` are owned by the current desktop process.
- Removed the release fallback that could mistake the Tauri executable itself
  for the `mew` daemon, and corrected the repository target paths used by the
  development fallback.

Verified: sidecar unit tests, browser tests (52), Tauri dev with a bundled debug
sidecar, explicit attach to an existing daemon on port 25566, clean child
shutdown in both modes, and the packaged release executable. The release app
bundle contains `Contents/MacOS/mew` alongside the desktop host.

## 2026-07-17 — Desktop daemon rendezvous and adversarial UX pass

- Reworked the Tauri supervisor around a shared loopback rendezvous port
  (25566 by default, with MEW_DESKTOP_DAEMON_PORT as an override). It performs
  a real WebSocket ping/pong check before launching anything, attaches to an
  existing mew daemon without owning it, rejects occupied non-mew ports, and
  lazily starts owned sidecars/processes only when the frontend requests the
  endpoint.
- Made native host bootstrap failures recoverable in the UI with a clear error
  message and retry action. The frontend connection manager now reports
  connection errors, retries with bounded backoff, and recovers after both
  pre-open failures and established socket closes.
- Removed automatic session creation from the home route. Reopening the last
  session is explicit and failure leaves a useful new-session path instead of
  silently replacing the session.
- Added workspace context to the header, session search, clickable recent
  sessions, a labeled view dropdown for timeline/workspace/grouped modes, and
  mobile dock spacing that keeps the composer, status footer, permission toast,
  and bottom navigation from overlapping.
- Added shared press/focus behavior and reduced-motion handling, replaced the
  reasoning bounce indicator, and added labels to icon-only controls.

Verified: web UI tests (52), web UI production build, web-client tests (12),
Tauri host tests (7), Tauri host clippy, and a real ping/pong probe against the
existing daemon on 127.0.0.1:25566.

## 2026-07-17 — Desktop release verification

Verified the updated host and shared React app through
pnpm --filter mew-web-ui desktop:build. The macOS app and aarch64 DMG both
bundled successfully with the architecture-specific mew sidecar.

## 2026-07-17 — Final daemon lifecycle and UX verification

- Corrected the interim daemon notes above: desktop startup now uses the shared
  25566 rendezvous port, protocol-level WebSocket ping/pong health checks, lazy
  startup, attach-without-ownership, and an explicit occupied-port failure.
- Added retry coverage for both pre-open failures and errors emitted after an
  established WebSocket connection.
- Kept the final frontend pass focused on recoverable states, session discovery,
  workspace context, labeled view switching, mobile dock spacing, keyboard
  focus, reduced motion, and icon-only control labels.

Verified: web UI tests (54), web-client tests (13), web UI production build,
Tauri host tests (7), Tauri host clippy, Tauri formatting, scoped diff checks,
and the packaged macOS app plus aarch64 DMG from the release build.

## 2026-07-17 — Session rail overlap and radius restoration

- Reserved the hover-action column inside each session title row so regenerate,
  pin, archive, and grouping controls cannot cover session text.
- Restored the shared `--radius` CSS token at `0.625rem`; the theme generator
  emits color variables only, so this dimension belongs in the web base layer.
- Added a regression test for the reserved session-action space and explicit
  cleanup between rail renders.

Verified: web UI tests (55), the production web build, generated CSS containing
the radius token and rounded utilities, and scoped diff checks.

## 2026-07-17 — Interactive activity panel and workspace lifecycle pass

- Rebuilt the Activity sheet around six visible icon-and-label sections, with
  actionable-first opening, wrapped mobile tabs, richer empty states, pinned
  context, stable question resolution, and rounded desktop/mobile treatment.
- Prevented Files and Changes from probing sessions without a workspace; they
  now explain the missing context instead of injecting daemon errors into chat.
- Made browser-created sessions inherit and persist the daemon launch cwd,
  exposed it through `SessionReady`, kept writer metadata synchronized, and
  threaded it into resumed agent construction so chat tools, file browsing, and
  git status share one workspace.
- Serialized overlapping client session lifecycle requests. This prevents an
  old failed attach from rejecting a new-session request and fixes the live
  `session not found` route handoff.
- Limited the default session timeline to the 40 newest sessions while keeping
  search access to older history, replaced raw ids with useful titles, and
  added accessible labels to session/group actions.

Verified: interactive browser smoke tests for fresh-session creation, Activity,
Files, Changes, recovery, and workspace-backed git status; web UI tests (64),
web-client tests (14), session metadata regression coverage, bridge e2e, daemon
and workspace builds, production web build, and scoped diff checks.

## 2026-07-17 — Keyboard-first session and project search

- Added the shadcn Command primitive via `npx shadcn@latest add command --yes
  --overwrite` and moved the existing palette onto the shared UI component.
- Expanded cmd/ctrl+k into a searchable workspace launcher for actions, sessions,
  projects, titles, summaries, first-message content, loaded current-session
  content, folders, and dates.
- Added `project:`, `folder:`, `before:`, and `after:` query operators, with
  project results opening a fresh session in that directory.
- Added focused search tests covering content, path, combined filters, inclusive
  dates, and project matching. The index tolerates both seconds and milliseconds
  from older and newer daemon metadata.

Verified: browser interaction for cmd/ctrl+k, content search, no-match state, and
project results; web UI tests (68) and production web build.

## 2026-07-17 — Attention-first notification hierarchy

- Added a shared attention taxonomy with explicit labels: `Permissions needed`,
  `Question · needs input`, and `Turn failed`.
- Session ordering now puts attention ahead of recency, running state, and pin
  state in the timeline, grouped view, and workspace folders. The current
  session no longer gets an exception.
- Added a persistent `Needs attention` queue at the top of Activity, with
  session-level badges and a header indicator. Queue items navigate directly to
  the session that needs action.
- Stopped treating ordinary turn completion as an in-app notification. A
  successful follow-up clears the previous failure alert, while permission and
  question items remain driven by their live pending counts.
- Added regression coverage for precedence, explicit labels, queue rendering,
  current-session ordering, and alert lifecycle cleanup.

Verified: interactive Activity-panel smoke test, web UI tests (73), production
web build, and scoped diff checks.

## 2026-07-17 — Browser-use vertical slice

- Added daemon-owned browser commands backed by the installed native
  `agent-browser` CLI. Browser sessions are keyed to mew sessions and support
  HTTP(S) navigation, accessibility snapshots, screenshots, click, fill, key
  press, and close.
- Extended the Rust and TypeScript wire clients with browser state, snapshot,
  and screenshot messages.
- Added a Browser tab to the Activity rail with URL navigation, semantic text
  inspection, screenshot capture, and basic element actions.

Verified: `agent-browser` opened and inspected example.com, Rust daemon and
protocol tests (101 protocol tests, 3 daemon tests), web-client build, and
production web build.

## 2026-07-17 — macOS CEF authoritative-browser proof of concept

- Added `native/cef-host`, a standalone Rust `cef-rs` host that creates a
  visible Chromium window and exposes loopback CDP for `agent-browser`.
- Added the macOS helper target and CEF bundle metadata required to produce a
  real `.app` with the framework and helper app bundles.
- Added a stable per-user CEF cache path and defaulted the unsigned development
  host to Chromium's mock keychain to avoid repeated macOS Keychain prompts.
  `MEW_CEF_USE_SYSTEM_KEYCHAIN=1` opts back into real browser Keychain storage.

Verified: native all-targets check, official CEF bundle generation, launch of
the bundled app, CDP `/json/version`, accessibility snapshot, and an
`agent-browser --cdp 9223` click against the same visible page.

## 2026-07-17 — Tauri native sibling integration

- Exposed `mew-cef-host` as a reusable macOS embedding library. It creates a
  CEF child `NSView` inside the Tauri content view, keeps bounds and visibility
  on the main thread, and uses Tauri's external message-pump callback.
- Added Tauri commands for CEF availability, bounds, navigation, visibility,
  and close/hide behavior. React now positions the native surface over the
  Browser panel viewport while retaining the existing WKWebView controls and
  text snapshot fallback.
- Routed desktop daemon `agent-browser` calls to the same CEF CDP endpoint and
  removed the incompatible persistent agent-browser session flag in that mode.
- Added CEF framework preparation for development links and release copies,
  macOS framework bundling, CEF helper-process dispatch, mock-keychain defaults,
  and graceful fallback when the native sibling is unavailable.

Verified: native all-targets check, Tauri cargo check, web UI tests (73), web
production build, daemon CDP argument tests, Tauri debug `.app` bundle build,
and standalone CEF CDP control with `agent-browser`. The bundled Tauri process
reaches CEF DevTools, but its GPU subprocess is still unstable in this local
environment even though embedded mode requests software rendering; GPU remains
opt-in for experiments via `MEW_CEF_ENABLE_GPU=1`. macOS sandbox/helper
hardening is still a follow-up before release signing.

## 2026-07-17 — CEF reopen lifecycle hardening

- Fixed helper startup ordering so the CEF command-line wrapper is never
  constructed before `libcef` is loaded.
- Packaged and selected the dedicated `mew-cef-host-helper` for Tauri CEF
  subprocesses. The helper now resolves both nested CEF helper bundles and the
  flat `Contents/MacOS` layout used by Tauri.
- Removed the inline message-loop call from `CefInitialize`, seeded the first
  external-pump turn after Tauri re-enters its event loop, isolated the embedded
  cache from the standalone host, and close/release the browser before
  `CefShutdown`.
- Split native browser layout and visibility effects so tab unmounts and
  reopenings cannot enqueue stale hide/show work against the native view.

Verified: native helper and embedding tests, Tauri cargo check, web UI tests
(73), frontend build, Tauri debug `.app` bundle build, CEF page startup, and
two launches against the same cache with the page available at the CDP target.

## 2026-07-17 — Daemon sidecar rebuild

- Added the three browser message variants to the TUI capture message-name
  formatter so the daemon binary remains exhaustive as the wire protocol
  evolves.
- Rebuilt the debug daemon sidecar and bundled debug `.app` after the CEF
  lifecycle changes.

Verified: `cargo test -p mew --bin mew tui_capture` (6 passed),
`pnpm desktop:prepare:dev`, and `pnpm tauri build --debug --bundles app
--no-sign`.

## 2026-07-17 — Workspace surfaces design direction

- Established the frontend direction as a Codex-style desktop workspace with
  two independent surfaces: a default-on pinned summary for project/session
  orientation and a separately toggled workbench for activity, browser,
  changes, and review.
- Captured the design context in `.impeccable.md`: the product should feel
  fast, focused, and alive, with attention surfaced before ordinary session
  navigation and keyboard-first, low-latency interactions.
- Identified the main implementation constraint: the native CEF browser must
  remain mounted while switching workbench tabs so its visible page and CDP
  target survive tab changes.

## 2026-07-17 — Independent workspace surfaces implementation

- Added persisted workspace-surface state with a pinned summary defaulting on
  and a workbench defaulting off. `⌘B` controls the summary and `⌘⇧B` controls
  the workbench independently.
- Added a root-level `WorkspaceFrame` so the desktop workbench is a dock beside
  the chat surface. Mobile keeps the existing sheet behavior.
- Reworked the activity rail into top-level Activity, Browser, Changes, and
  Review tabs. Browser lazy-mounts on first use and stays mounted while tabs
  switch, while CEF visibility follows the active surface.
- Added a first local working-tree Review surface and removed duplicate Changes
  navigation from the Activity sub-tabs.

Verified: web UI tests (80), production web build, and `git diff --check`.
The in-app browser smoke pass could not start because this sandbox rejects
binding the local Vite development port.

## 2026-07-17 — Tauri CEF dev preparation fix

- Isolated the opaque Tauri `os error 2`: sidecars were present; the failing
  resource was the CEF framework path and dev preparation was linking it.
- Updated the dev CEF preparation command to copy a real framework directory,
  matching Tauri's macOS framework resource copier.
- Confirmed `cargo check --manifest-path mew-web-ui/src-tauri/Cargo.toml` and
  `just desktop-dev` reach a successful `mew-desktop` build and launch.

## 2026-07-17 — CEF development runtime assets

- Added explicit CEF resource and locale paths for the embedded browser.
- Prepared `icudtl.dat`, CEF resources, and GPU libraries for the unbundled
  macOS debug executable.
- Updated the helper loader to use `MEW_CEF_FRAMEWORK_PATH` when running
  outside the packaged `.app` helper layout.
- The ICU and missing-library errors are gone. Remaining dev-only output is
  CEF Mach port rendezvous noise from the unbundled helper/runtime layout.

Verified: `cargo check --manifest-path native/cef-host/Cargo.toml`,
`cargo fmt --all -- --check`, and desktop launch through `just desktop-dev`.

## 2026-07-17 — CEF diagnostic cleanup

- Kept the confirmed synthetic bundle/rendezvous fix and external-pump
  backstop.
- Removed the confirmed no-op occlusion switches and reverted the diagnostic
  800×600 initial bounds to 1×1.
- Removed inert `was_hidden`/`was_resized` host notifications from the embed
  path after verification showed they do not affect this CEF build.
- Added the dev helper framework/resource loading path and development CEF
  asset preparation needed by the unbundled macOS executable.

Verified: native and Tauri cargo checks, cargo formatting, and diff hygiene.

## 2026-07-18 — Codex-style browser workbench tabs

- Added a tested browser-tab reducer with add, select, update, and close
  behavior that always leaves one usable new-tab surface.
- Added a compact tab strip inside the Browser workbench with explicit close
  and new-tab controls, hostname labels, and `⌘T`/`⌘W` shortcuts.
- Kept the browser surface mounted while switching workbench modes and made
  the native CEF visibility follow both the active workbench mode and the
  active browser tab.
- Exercised the interaction in the local app: opened the workbench, created a
  second tab, navigated it to example.com, and switched back to the new tab.

Verified: browser-tab and RightRail tests (12 passed), TypeScript build,
production web build, and `git diff --check`. The local smoke app still reports
the pre-existing daemon connection outage, but the workbench remains usable.

## 2026-07-18 — Packaged Tauri native smoke test

- Built and launched the debug `.app` bundle with the real macOS bundle
  identity, including the bundled CEF framework and helper.
- Confirmed the clean packaged launch reaches the native event loop and starts
  the CEF helper processes without the earlier ICU/resource errors.
- The embedded CEF target still advertises a DevTools page but has no renderer
  child; `Page.enable` times out. This is the remaining native browser blocker
  before the browser workbench can be exercised visually in the packaged app.
- The Computer Use accessibility probe could not retrieve the native AX tree in
  this environment, so no source changes were made from this smoke pass.

Verified: Tauri debug bundle, clean native launch, process/helper inspection,
and `git diff --check`. `desktop:verify:cef` reaches the DevTools endpoint but
fails at `Page.enable` timeout.

## 2026-07-18 — CEF renderer startup unblocked

- Switched embedded browser creation to CEF's asynchronous API and retained the
  browser handle from `on_after_created`, removing the create-time race.
- Added the macOS helper app bundle layout CEF expects under
  `Contents/Frameworks`: renderer, GPU, plugin, alerts, and base helpers named
  from the Tauri executable (`mew-desktop Helper*.app`).
- Kept the flat helper for fallback/dev preparation, but let packaged CEF use
  the nested helper apps and propagate the bundled framework path to children.
- Re-sign the generated app after adding nested helpers so the debug and
  release bundle workflows remain verifiable on macOS.

Verified: web tests (86), native and Tauri cargo checks/clippy, a rebuilt and
launched packaged `.app`, live renderer helper processes, `codesign --verify
--deep --strict`, and all 7 `desktop:verify:cef` checks including a compositor
screenshot.

## 2026-07-18 — Shared native browser session verification

- Launched the packaged Tauri app and attached `agent-browser --cdp` to the
  same CEF target used by the visible app surface.
- Navigated from `https://example.com/` to `https://example.org/` through the
  agent path and captured a screenshot, confirming the user-facing renderer
and agent control path share one browser session.

## 2026-07-18 — Tauri dev framework preparation fix

- Restored the normal `desktop:dev` CEF preparation step to copy the framework
  instead of symlinking it.
- Tauri's macOS build script walks and recopies the framework, and its symlink
  failure surfaced only as `No such file or directory (os error 2)` during the
  custom build command.

Verified: `desktop:prepare:dev`, `desktop:prepare:cef:dev`, `tauri dev`, live
CEF renderer helper processes, and `agent-browser --cdp 9223` reading the
embedded page title and URL.

## 2026-07-18 — Browser protocol mismatch recovery

- Made both desktop preparation paths rebuild `@mew/web-client` before Vite
  or Tauri starts, so the generated SDK distribution cannot lag behind its
  browser protocol source.
- Added SDK coverage for browser response dispatch.
- Made the Browser workbench listen for browser-scoped daemon errors, clear
  its loading state, and show the protocol error inline instead of appearing
  frozen.
- The existing daemon on the shared port was started from an older binary and
  cannot decode the browser variants. Restart that daemon after browser
  protocol changes, or use `MEW_DESKTOP_DAEMON_PORT` to attach to a fresh
  instance during development.

Verified: 15 web-client tests, 87 web UI tests, production web build, and
`git diff --check`.

## 2026-07-18 — Workbench tab restructuring plan

- Decided to use the existing shadcn/Radix Tabs primitive for accessible tab
  semantics and keyboard behavior, with a custom document-strip presentation.
- Defined a unified workbench tab registry for browser pages, terminals, files,
  changes, reviews, and activity. The pinned summary remains independent.
- Recorded the staged architecture and migration plan in
  `docs/development/workbench-tabs.md`.

Verified: local shadcn Tabs implementation review, official shadcn Tabs
reference review, and `git diff --check`.

## 2026-07-18 — Resizable workbench decision

- Chose shadcn's Resizable panel composition for the conversation/workbench
  split, with a draggable and keyboard-adjustable divider.
- The workbench will collapse to zero when closed and restore its last usable
  width when reopened; mobile keeps the existing sheet behavior.
- Recorded the CLI convention for new shadcn components in `AGENTS.md`:
  `npx shadcn@latest add <component>`.

Verified: local dependency/component inventory, official shadcn Resizable
reference review, and `git diff --check`.

## 2026-07-18 — Resizable workbench shell implemented

- Added shadcn's `resizable` component through the CLI and adapted its wrapper
  to the installed `react-resizable-panels` v4 exports (`Group` and
  `Separator`).
- Replaced the fixed desktop workbench width with a keyboard- and pointer-
  adjustable conversation/workbench split.
- Persisted the workbench width, restored it after collapse/reopen, and kept
  the mobile sheet path unchanged.
- Added reducer coverage for width clamping and visibility synchronization.

Verified: 89 web UI tests, production web build, and `git diff --check`.

## 2026-07-18 — Unified workbench tabs implemented

- Added a persistent workbench tab registry for Activity, browser pages,
  terminal/job output, files, Changes, and Review.
- Migrated the old single `workbenchTab` preference into the new registry while
  keeping the pinned summary independent from the workbench.
- Replaced the fixed workbench mode buttons and nested browser tabs with one
  shared shadcn/Radix Tabs strip, close controls, a surface picker, and
  Codex-style Cmd/Ctrl+T, Cmd/Ctrl+W, and Cmd/Ctrl+1–9 shortcuts.
- Promoted browser URL/title state into top-level tabs and kept native CEF
  navigation scoped to the active browser tab. Terminal copy is deliberately
  labeled as background job output until the daemon has PTY support.
- Added reducer, migration, persistence, accessibility, browser-tab, and
  RightRail interaction coverage.

Verified: 95 web UI tests, TypeScript check, production web build, and
`git diff --check`.

## 2026-07-18 — Browser lifecycle and tab routing hardening

- Added optional `tab_id` fields to browser commands and responses, plus a
  typed `browser_error` response so late failures cannot overwrite the active
  browser tab.
- Updated the daemon and TypeScript client to echo browser tab identity while
  retaining decode compatibility with older messages that omit it.
- Filtered browser events in the workbench by tab identity, scoped browser
  close requests, and cleared loading state for every typed browser error.
- Kept browser surfaces mounted through tab switches so snapshots and errors
  survive selection changes, while inactive panels remain hidden from the
  accessibility tree and cannot trigger navigation.
- Added owner-gated native CEF bounds, visibility, navigation, and cleanup
  calls. Stale queued callbacks cannot hide or release a newly active tab, and
  failed main-thread scheduling releases only the owner that claimed it.
- Documented phase 2 as in progress. The remaining lifecycle work is a live
  unlocked-screen CEF soak covering repeated tab switches, close/reopen, and
  agent-browser CDP control.

Verified: 103 protocol tests, 15 web-client tests, 101 web UI tests, web-client
build, UI TypeScript check, production UI build, `cargo check -p mew-daemon -p
mew`, 10 Tauri shell tests, `cargo fmt --all -- --check`, and `git diff
--check`.

## 2026-07-18 — Desktop browser soak and lifecycle hardening

- Ran the live browser soak against the Tauri app: 12 rapid switches between
  two browser tabs stayed on the expected URL, close/reopen preserved the
  workbench, and a newly created tab navigated successfully.
- Verified `agent-browser --cdp 9223` against the packaged CEF page. Native CEF
  address/title events now update the active React tab, so CDP navigation and
  the visible tab strip share one authority.
- Removed the Tauri host's inherited browser-CDP environment override so the
  daemon cannot be configured to launch a second browser session. Directly
  spawned daemon children close inherited descriptors; a fresh packaged launch
  now leaves the daemon on 25566 without a copied listener on 9223. The desktop
  shell also uses Tauri's single-instance plugin to focus the existing app
  instead of creating a competing host.
- Added a web-host no-op listener regression test for native CEF events.

Verified: 102 web UI tests, 15 web-client tests, 103 protocol tests, web-client
build, UI TypeScript check, production UI build, packaged `pnpm desktop:build`,
`pnpm desktop:verify:cef` with all 7 checks passing, 10 Tauri shell tests,
`cargo check -p mew-daemon -p mew`, `cargo fmt --all -- --check`, and
`git diff --check`.

## 2026-07-18 — Desktop browser authority and shutdown fixes

- Restored the desktop browser transport contract: when CEF is available, the
  daemon receives `MEW_BROWSER_CDP_PORT` and `agent-browser` targets the same
  visible CEF page used by the workbench.
- Added an explicit CEF pump lifecycle with a stop token, callback guards, and
  a joined worker so queued message-loop work cannot run after `libcef`
  shutdown.
- Added URL-aware native CEF event filtering, native title URL context, and
  reducer-action routing for controlled workbench state so rapid navigation
  events cannot overwrite newer tab state.
- Closed inherited file descriptors inside daemon startup as well as the
  direct desktop spawn path, covering the shell-sidecar fallback.
- Removed the unused duplicate browser-tab registry and the obsolete native
  close helper.

Verified: 101 web UI tests, UI TypeScript check, production UI build, 103
protocol tests, daemon and Tauri checks, packaged `pnpm desktop:build`, fresh
packaged launch/relaunch, `pnpm desktop:verify:cef` with all 7 checks passing,
`cargo fmt --all -- --check`, and `git diff --check`.

## 2026-07-18 — UI motion and surface polish

- Added shared motion tokens and custom easing curves for press feedback,
  menus, drawers, panels, and reduced-motion behavior in the web UI.
- Replaced generic shadcn animation utilities on dialogs, alert dialogs,
  sheets, tooltips, dropdowns, and selects with explicit property-scoped
  opacity/transform transitions and origin-aware popover behavior.
- Added consistent press feedback to the shared Button primitive and the
  highest-frequency raw controls, plus restrained entry motion for activity,
  attention, permission, connection, and plan-request surfaces.
- Added rounded desktop conversation/workbench surfaces and a clearer resize
  handle treatment. Native CEF visibility remains discrete and is not CSS
  transformed or faded.
- Reduced streaming-adjacent motion noise by keeping tab selection and token
  streaming immediate and collapsing reasoning activity to one live indicator.

Verified: 101 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Browser workbench chrome cleanup

- Removed the generic Workbench title/subtitle when a browser tab is active so
  the surface reads like an actual browser window.
- Restyled the shared workbench tab strip into compact browser chrome with
  rounded active tabs, while keeping Activity, Files, Changes, Review, and
  Terminal tabs in the same registry.
- Made the URL and page-action rows browser-like, centered the page identity,
  and let the native browser viewport fill the remaining surface without a
  nested card inset.
- Kept the generic workbench summary header for non-browser surfaces.

Verified: 101 web UI tests, TypeScript build, production UI build, and
`git diff --check`.

## 2026-07-18 — CEF navigation pump re-entrancy fix

- Diagnosed the Google navigation freeze from a live debug process: the CEF
  renderer helpers stayed alive, but `curl http://127.0.0.1:9223/json/version`
  connected and then hung. A macOS process sample showed CEF recursively
  entering `do_message_loop_work` through Tauri's inline
  `run_on_main_thread`, while CEF already held its browser-process mutex.
- Added a pump gate so nested CEF turns are skipped, and dispatch on-demand
  pump callbacks from a worker before handing them back to the Tauri main
  thread. This prevents callbacks raised during native view visibility or
  navigation work from re-entering CEF synchronously.
- Kept the 30 ms backstop for callbacks coalesced during an active turn and
  added a regression test for the pump gate.

Verified: 12 Tauri host tests, cargo formatting check, and `git diff --check`.
The running debug process must be restarted to load this fix.

## 2026-07-18 — iOS motion and surface parity

- Added native motion tokens for press feedback, disclosures, and surface
  changes, using SwiftUI springs/snappy timing instead of generic easing.
- Added a shared press style for frequent controls, with a restrained 0.97
  touch-down scale and reduced-motion support.
- Applied the motion vocabulary to connection banners, retry states, todo and
  tool disclosures, scrolling, typing indicators, and daemon activity pulses.
- Made custom fonts Dynamic Type-aware and corrected the MiSans runtime font
  name used by the UIKit fallback configuration.
- Standardized the main message and todo panel geometry around continuous,
  rounded surfaces while preserving native sheets and navigation.

Verified: arm64 iOS Simulator `xcodebuild` succeeded with
`CODE_SIGNING_ALLOWED=NO`, and `git diff --check` passed. The default
multi-architecture simulator build remains blocked by the checked-in
`mew_mobile_core.xcframework` missing an x86_64 simulator slice.

## 2026-07-18 — Native Tauri workbench menu boundary

- Added a macOS AppKit `NSMenu` path for the workbench `+` menu. It is
  anchored to the React control and renders above the native CEF child view,
  while the regular web host keeps the existing HTML menu and does not expose
  a built-in browser.
- Kept workbench tab state and surface selection in React. Native menu actions
  return through a Tauri event, so Browser, Terminal, Files, Changes, and
  Review continue to share the same tab registry.
- Added native-menu lifecycle commands, close events, and a web-host no-op
  test. Failed native menu setup falls back to the HTML menu.
- Hardened the CEF/Tauri boundary by installing the two CEF macOS application
  selectors onto Tauri's existing `TaoApp` instead of replacing its
  `NSApplication` class. CEF child views now start hidden until an active tab
  claims and sizes them.

Verified: 102 web UI tests, UI TypeScript check, native CEF tests, 12 Tauri
host tests, `cargo fmt --all -- --check`, and `git diff --check`. A debug
desktop launch stayed alive after the selector and initial-visibility fixes;
the native popup click still needs a clean single-instance desktop smoke run.

## 2026-07-18 — Browser connection lifecycle guard

- Fixed restored browser tabs throwing `not connected` during the first render
  while the websocket client was still connecting.
- Browser daemon commands now wait for the shared connection state, and the
  browser controls stay disabled while disconnected instead of surfacing a
  React error boundary.
- Native macOS CEF navigation now takes precedence once CEF availability is
  resolved, so a disconnected daemon does not block the embedded browser.
- Added a regression test for restored browser tabs during daemon disconnect.

Verified: 103 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`. A desktop smoke launch was blocked by an existing Vite
process already listening on port 5173; the active Tauri host did stay alive
and spawned a daemon on port 25566.

## 2026-07-18 — Browser navigation deduplication guard

- Kept restored-tab recovery tied to connection changes without making normal
  URL submission navigate twice.
- Added an assertion covering the single-send web browser open path.

Verified: 103 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Chat rendering stability and overflow guard

- End-anchored the virtualized conversation and batched resize observations so
  measured message heights stop pulling the viewport around during streaming.
- Limited live-store subscriptions to the currently streaming message and
  memoized message rows, reducing token-by-token rerenders across the visible
  conversation.
- Added stable scrollbar space and min-width/overflow boundaries across chat,
  markdown, code, reasoning, and tool surfaces so the conversation cannot
  create page-level horizontal scrolling.
- Added regression coverage for completed rows staying stable during live
  streaming updates.

Verified: 105 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — macOS XDG-style config directory

- Changed macOS global configuration storage to `~/.config/mew`, matching the
  Linux-style location used by other CLI tools.
- Kept the change non-migrating: existing Application Support data remains in
  place and is no longer selected by the default resolver.
- Updated configuration and session path documentation and added a macOS path
  regression test.

Verified: 118 `mew-config` tests, Rust formatting, and `git diff --check`.

## 2026-07-18 — Files root navigation

- Fixed the Files surface sending `/` to the daemon when navigating back to
  the workspace root.
- Kept root navigation represented as an omitted relative path, preserving the
  daemon’s absolute-path safety check.
- Added regression coverage for parent and join path handling.

Verified: 107 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Workbench surface picker

- Replaced the web add-tab dropdown with a searchable shadcn command picker
  showing every workbench surface, icon, description, and shortcut.
- Preserved the native macOS picker for the CEF host so the menu stays above
  the native browser surface, while adding the same complete option set.
- Added jsdom layout shims and picker coverage for keyboard-oriented cmdk
  behavior.

Verified: 105 web UI tests, UI TypeScript check, production UI build, Rust
formatting, and `git diff --check`.

## 2026-07-18 — Browser omnibox and native tools menu

- Combined the browser URL and page chrome into one omnibox-style row with a
  single submit affordance and loading state.
- Moved snapshots, screenshots, selector interaction, and hide-page controls
  into a secondary Browser tools surface so the browser viewport keeps its
  height.
- Added an AppKit browser-tools menu for macOS, routing native menu actions to
  the active CEF tab so the controls stay above the native browser view.
- Kept the web popover fallback for non-native browser rendering and covered
  the new one-row and tools interactions.

Verified: 105 web UI tests, UI TypeScript check, production UI build, Tauri
`cargo check`, Rust formatting, and `git diff --check`.

## 2026-07-18 — Composer surface simplification

- Flattened the composer into one bounded surface, keeping the message field,
  attachment state, persona/model controls, and send/cancel action in a single
  visual hierarchy inspired by the Codex composer.
- Kept session telemetry in the separate status footer so connection and token
  information does not compete with message composition.
- Added a regression test ensuring the composer controls remain inside the
  primary surface.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Floating dock scroll containment fix

- Restored flex-column containment around the session chat after moving the
  composer to the full-surface overlay.
- This gives the virtualized and fallback scroll containers a bounded height
  again, preserving chat scrolling while keeping the dock independently
  positioned.

Verified: 106 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Unified modular workbench tabs

- Promoted Plan, Agents, Questions, and Jobs into first-class workbench tabs
  alongside Browser, Terminal, Files, Changes, and Review.
- Removed the nested Activity tablist and its duplicate tab state, leaving one
  modular tab interface for every right-rail surface.
- Kept core activity tabs pinned, migrated persisted legacy Activity tabs to
  Plan, and preserved actionable-tab selection when the workbench opens.
- Updated tab, persistence, and right-rail regression coverage.

Verified: 107 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Workbench header removal

- Removed the redundant Workbench title/subtitle header so the tab strip is the
  first visible workbench surface.
- Moved the close action into the tab row, preserving access without restoring
  another explanatory panel.

Verified: 106 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Composer containing-block correction

- Restored explicit parent-relative height on the workspace row and both
  conversation insets so the bottom dock resolves against the full app surface
  instead of a content-sized containing block.
- Kept the earlier full-surface fade and bottom dock positioning intact.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Full-surface composer anchoring

- Re-anchored the floating composer to the complete session surface instead of
  the chat column, so it reaches the actual bottom edge beneath the footer.
- Added a non-interactive bottom fade that eases the underlying chat and status
  details into the page background without adding another panel row.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Floating composer treatment

- Removed the full-width composer panel border and background so the chat
  surface continues behind it.
- Kept the composer itself elevated with a focused border and restrained
  shadow, preserving the single-surface hierarchy and all existing controls.

Verified: 106 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Positioned floating composer

- Moved the composer into an absolute bottom dock inside the session surface,
  removing it from normal chat layout flow while keeping the status footer
  independent below it.
- Added transparent pointer-through space around the dock so only the composer
  and pending interaction cards capture input.
- Added bottom scroll clearance to both virtualized and fallback chat surfaces,
  keeping the latest message visible above the floating dock.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Workspace shell sizing correction

- Changed the conversation inset from viewport-relative `h-screen` sizing to
  parent-relative `h-full` sizing in both mobile and desktop layouts.
- Preserved the rounded inset surface while keeping it inside the enclosing
  `h-svh` workspace frame, preventing the inner shell from becoming taller
  than the outer app surface.

Verified: 105 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — On-demand workbench tabs

- Removed Agents and Jobs from the default workbench so the rail opens as an
  intentional empty tool area until the pinned summary exists.
- Kept Agents and Jobs available as explicit optional tabs through both the web
  and native macOS add-tab menus.
- Migrated the previous pinned Agents/Jobs defaults out of persisted state and
  added an empty-workbench affordance instead of rendering a blank surface.

Verified: 105 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Main request flow and focused workbench tabs

- Kept Plan approval and Questions in the main session interface, including
  desktop AskUser rendering beside the composer.
- Reduced the workbench to Agents, Jobs, Browser, Terminal, Files, Changes,
  and Review. Browser retains its own internal browser-tab model.
- Migrated persisted Activity, Plan, and Questions rail tabs out of the active
  tab model while preserving Agents and Jobs as pinned core tabs.
- Moved pinned file context into the Files surface and removed duplicate plan,
  question, and cross-session attention panels from the workbench.

Verified: 105 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Read-only Files workbench

- Replaced the narrow Files inspector with a lightweight editor workbench:
  resizable explorer, lazy directory expansion, file filtering, and internal
  read-only document tabs.
- Kept workspace paths relative at the UI boundary and preserved the existing
  external-editor action.
- Added focused behavior coverage for directory expansion, relative file
  opening, filtering, empty states, and document tabs.
- Kept dotfiles hidden until the daemon preview path is routed through the
  secret-file permission checks.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Bound file viewer code width

- Constrained the inner Shiki `<pre>` element, which was still able to impose
  its intrinsic width on the surrounding workbench.
- Made wrapped lines the default for file previews so long JSON and generated
  files cannot create a horizontal layout jump. The toggle remains available
  for intentional horizontal inspection inside the viewer.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Isolate unwrapped file code

- Prevented the highlighted surface and nested token code from contributing
  intrinsic width to the surrounding flex layout.
- Moved unwrapped horizontal scrolling to the highlighted `<pre>` itself so
  disabling wrapping cannot move the workbench.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Flush file preview surface

- Removed the stacked horizontal insets around file contents so the code
  surface reaches the workbench edges.
- Kept chat code block spacing unchanged and retained a small inset for the
  truncated-preview notice.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Fill file preview height

- Let the file code surface use the available editor height instead of ending
  at the last line of a short preview.
- Kept the viewer scrollable and left chat code blocks content-sized.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Tighten file code leading

- Reduced the file viewer line box from 1.55 to 1.35 and applied it directly
  to each highlighted line so dense source files stay compact.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Restore live response streaming

- Fixed provider part appends to replace the assistant message and parts
  immutably instead of mutating an object already held by memoized message
  rows.
- Kept newly started empty text parts renderable so their live stream buffer
  is visible before the completed text is committed.
- Added a regression test for text streaming that begins after reasoning.

Verified: 111 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Restore configured provider models

- Matched catalog provider aliases such as `opencode`/`opencode-zen` and
  `zai`/`z-ai` when building the daemon model list.
- Kept the configured provider id in the picker and switch request so the
  selected model uses the correct endpoint.
- Added coverage for the alias matching boundary.

Verified: targeted Rust provider test, `cargo fmt --all`, and
`git diff --check`.

## 2026-07-18 — Bound file viewer code width

- Constrained the inner Shiki `<pre>` element, which was still able to impose
  its intrinsic width on the surrounding workbench.
- Made wrapped lines the default for file previews so long JSON and generated
  files cannot create a horizontal layout jump. The toggle remains available
  for intentional horizontal inspection inside the viewer.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Bounded file viewer

- Separated file previews from chat code blocks so the editor uses a compact,
  full-height viewer rather than a card that can dictate the workbench width.
- Added editor-style line numbers and preserved syntax highlighting without
  changing message code-block rendering.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — File viewer line wrapping

- Added a per-file Wrap toggle for long lines.
- Kept horizontal overflow contained by default for code readability, with
  wrapped mode available for prose and configuration files.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — File viewer overflow containment

- Added min-width and overflow boundaries across the nested explorer/editor
  flex surfaces so long highlighted files cannot push the entire workbench
  horizontally.
- Kept horizontal scrolling local to the file viewer when wrapping is off.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Nested file pane sizing

- Forced the editor column and highlighted code surface onto shrinkable flex
  bases so files such as large JSON manifests cannot move the whole workbench
  horizontally.
- Kept overflow local to the viewer while preserving the Wrap toggle.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Restore file viewer width

- Corrected the containment pass so vertical editor children retain full
  width while the surrounding horizontal flex boundaries remain shrinkable.
- Restored visible file content without reintroducing workbench-wide overflow.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Bound file viewer code width

- Constrained the inner Shiki `<pre>` element, which was still able to impose
  its intrinsic width on the surrounding workbench.
- Made wrapped lines the default for file previews so long JSON and generated
  files cannot create a horizontal layout jump. The toggle remains available
  for intentional horizontal inspection inside the viewer.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Add desktop install and daemon cleanup recipes

- Added `just desktop-install` to build and copy the release app into
  `/Applications/mew.app` on macOS.
- Added `just stop-all-daemons` to stop mew bridges and daemon processes on
  the development ports, including stale desktop sidecars.

Verified: justfile dry-run checks and `git diff --check`.

## 2026-07-18 — Add in-app workspace folder browser

- Replaced the native directory dialog with a shared web/Tauri folder browser
  in the new-session project picker.
- Added a daemon filesystem-listing request that starts at the user’s home
  directory, returns directories only, and rejects protected macOS locations
  such as `/System`, `/Library`, `/private`, and `/Volumes`.
- Kept manual path entry for deliberate paths outside the picker’s normal
  browsing boundary.

Verified: protocol and daemon checks, web-client build, UI TypeScript check,
and `git diff --check`.

## 2026-07-18 — Ignore filesystem browsing in TUI capture

- Added the new filesystem directory listing to the TUI capture message
  naming table as an explicit no-op. Folder browsing is owned by the web and
  desktop surfaces and does not change TUI state.

Verified: `cargo check -p mew`, format check, and `git diff --check`.

## 2026-07-18 — Harden folder picker review findings

- Fixed the trailing comma in the Tauri capability manifest.
- Added filesystem boundary tests, protocol round-trip coverage, and a picker
  interaction test covering folder navigation and the parent-folder action.
- Added an in-app up-arrow control that stays within the picker’s home root.

Verified: daemon and protocol tests, picker tests, UI TypeScript check, JSON
validation, and `git diff --check`.

## 2026-07-18 — Recover from unavailable remembered sessions

- Kept the unavailable-session explanation visible while restoring the
  start-new-session action on the home route.
- A stale or daemon-mismatched remembered session can no longer strand the
  production app before the project/folder flow is reachable.

Verified: 112 UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Persist desktop daemon logs

- Stopped discarding stdout and stderr from daemons launched by the Tauri
  host.
- Release and configured daemon launches now append to
  `~/.config/mew/logs/desktop-daemon.log` for provider/config/session startup
  diagnostics.

Verified: Tauri cargo check, Rust formatting, and `git diff --check`.

## 2026-07-19 — Preserve full daemon provider errors

- Session creation and resume failures now log the complete anyhow error chain
  and return it to the frontend, instead of collapsing failures to only
  `build provider`.

Verified: daemon cargo check, Tauri cargo check, formatting, and
`git diff --check`.

## 2026-07-19 — Avoid blocking on an uncredentialed remembered provider

- Provider credential lookup now tries the configured credential reference,
  then the provider id for aliased providers such as `opencode-go`.
- Implicit daemon startup falls back to another configured provider with valid
  credentials when the remembered provider is unavailable.
- Explicit provider selections continue to return a clear credential error.

Verified: provider-resolution tests, `cargo check -p mew`, and
`git diff --check`.

## 2026-07-19 — Gate in-app browser tools to desktop sessions

- Added a distinct desktop client kind and made the shared React client
  advertise it only when running inside Tauri.
- Added explicit browser tools backed by the existing `agent-browser`/CDP
  session, with execution-time capability checks and untrusted-page output
  labeling.
- Browser protocol commands now reject TUI, web, CLI, and mobile clients.
- Desktop attachment enables browser tools for the session; detaching the
  final desktop client disables them again.
- Added a persisted system-level capability notice when desktop browser
  access is introduced to an existing session. Notices are deduplicated by a
  capability marker in the persisted message history.
- Added a `System` message role and provider translation for OpenAI,
  Anthropic, and Responses adapters. TUI rendering hides these notices.

Verified: focused Rust tests for message, protocol, tools, agent, daemon
library, and TUI packages; web-client TypeScript build; formatting and
`git diff --check`. The first combined test attempt hit the machine disk
limit while compiling daemon integration tests, so generated Cargo artifacts
were cleaned before the focused rerun.

## 2026-07-19 — Load the bundled web fonts

- Added `@font-face` declarations for the bundled MiSans, Banga, and
  IoskeleyMono assets.
- Pointed Tailwind, body text, headings, code, and typeset typography at those
  real families instead of unbundled placeholder names.
- Added the missing typeset heading fallback token.

Verified: production UI build, UI TypeScript check, and `git diff --check`.

## 2026-07-19 — Add iOS-matched font preferences to web and desktop

- Added the iOS font choices to web settings: System, Mi Sans, Junicode, and
  OFL Goudy.
- Mi Sans remains the default; the selection persists in local storage and
  applies immediately through the shared typography variables.
- Added the matching Junicode and OFL Goudy assets to the web bundle.

Verified: 112 UI tests, production UI build, UI TypeScript check, and
`git diff --check`.

## 2026-07-19 — Add explicit remote daemon and desktop remote modes

- Added a navigable settings surface with a dedicated Remote access section
  and a prominent warning describing remote filesystem, command, session, and
  relay exposure.
- Added `mew daemon --remote`, which keeps the local Unix/TCP listener and
  runs an authenticated iroh listener beside it. The existing `--iroh` path
  remains available for mobile compatibility.
- Added remote protocol authentication, explicit `RemoteScope` enforcement,
  `client_kind=remote` validation, pairing-token loading, and restrictive
  permissions for persisted pairing material.
- Added desktop supervisor support for enabling/disabling app-lifetime remote
  access and shipped the iroh feature in desktop sidecar builds.

Verified: `cargo clippy -p mew-daemon -p mew --features iroh -- -D warnings`,
focused daemon scope tests, iroh integration-test compilation, Tauri host
check, UI TypeScript check, 112 UI tests, production UI build, and
`git diff --check`.

## 2026-07-19 — Harden remote pairing and settings lifecycle

- Reused the existing `RemoteAccessStore` as the pairing boundary instead of
  introducing a second raw-token store. Pairing tokens are short-lived,
  single-use, and persisted only as SHA-256 digests.
- Remote iroh connections can now resolve their granted scope from a pairing
  token or an already paired device. The daemon rejects protocol messages
  until the remote handshake succeeds.
- `mew pair` now creates an invite without binding a second iroh endpoint, so it
  works while `mew daemon --remote` is already running.
- The settings toggle is wired to the Tauri supervisor and refuses to enable
  desktop remote access when the app is attached to an externally owned
  daemon.

Verified: `cargo check -p mew --features iroh`, `cargo test -p mew-daemon`,
`cargo clippy -p mew-daemon -p mew --features iroh -- -D warnings`, Tauri
`cargo check`, UI TypeScript check, focused UI tests, and `git diff --check`.

The final smoke pass also completed `cargo test --manifest-path
mew-web-ui/src-tauri/Cargo.toml supervisor --quiet`, the full web production
build, and `cargo test -p mew-protocol`.

## 2026-07-19 — Close remote lifecycle and protocol review gaps

- Removed the obsolete raw remote-token persistence path. Pairing now has one
  state store, one-time hashed invites, device metadata, expiry, revocation,
  and explicit hosting state.
- Added protocol roundtrip coverage for the remote handshake and made the
  desktop supervisor restore its prior setting if a remote-mode restart fails.
- Desktop-launched remote daemons now persist desktop ownership mode, while
  the CLI remains the long-lived daemon mode. `--iroh` and `--remote` are
  mutually exclusive and their user-facing guidance is distinct.

Verified: focused daemon, iroh, protocol, mobile-core, Tauri, web-client, and
React UI tests/builds; formatting and `git diff --check`.

Final review follow-up:

- Legacy allowlisted iroh clients can no longer claim the desktop-only client
  kind, and control-scope authorization is an explicit current-message list
  that fails closed for future protocol additions.
- Remote state mutations use a short-lived cross-process lock and refresh from
  disk before read-modify-write operations. Pairing readiness, listener failure
  cleanup, and private key/allowlist permission hardening are covered in the
  implementation path.

Verified: daemon checks, iroh integration tests, protocol tests, Tauri check,
formatting, and `git diff --check` after the final review fixes.

## 2026-07-19 — Fix live remote pairing invites

- Included the one-time pairing token in the QR URL consumed by mobile and
  remote clients.
- Refreshed the daemon's file-backed access state during authorization so a
  `mew pair` run can authorize a live `--remote` daemon without a restart.
- Removed the legacy iroh allowlist bypass from explicit remote mode; it now
  requires a pairing token or an active paired device.
- Added regression coverage for invite payloads and cross-process pairing
  refresh.

Verified: focused iroh, daemon, and CLI tests; Tauri check; formatting; and
`git diff --check`.

## 2026-07-19 — Repair daemon listener structure and observe scope

- Restored the missing close for the Unix daemon accept loop after the remote
  capability guard was misplaced during the lifecycle pass.
- Kept observe-only remote access read-only by rejecting `NewSession` and added
  a regression test for that boundary.

Verified: daemon remote tests, iroh integration tests, daemon and CLI clippy,
Tauri check, TypeScript, 112 React tests, and `git diff --check`.

# 2026-07-19 — Invert CEF/WKWebView layering, delete AppKit menus

## Summary

The embedded CEF browser used to paint above the Tauri WKWebView, so any HTML
overlay (settings dialog, cmd+k palette, surface picker, browser-tools
popover) was clipped by the browser rect, and the two menus that had to sit
above the browser were AppKit `NSMenu`s. Implemented PLAN.md: CEF now sits
permanently below a transparent WKWebView, the React app leaves a transparent
hole where the CEF viewport lives, and the native menu paths are deleted in
favor of the existing HTML surfaces on every host.

## What changed

- `mew-web-ui/src-tauri/src/native_layering.rs` (new): `NativeLayeringGuard`
  (pure, unit-tested — orders each CEF native view handle exactly once,
  re-orders when CEF recreates the view) and `order_cef_below_webview`, which
  resolves the WKWebView via Tauri's `with_webview` and the content view via
  `ns_view()`, verifies they share a superview, and calls
  `addSubview_positioned_relativeTo(cef, Below, wkwebview)`. Failures are
  logged, never fatal.
- `native/cef-host/src/embed.rs`: added `CefEmbedController::native_view_handle()`
  (macOS impl + non-macOS stub returning 0).
- `mew-web-ui/src-tauri/src/lib.rs`: ordering is folded into
  `cef_browser_set_rect` with `visible: true` (the owner-claim moment). The
  guard decision and `mark_ordered` both run outside AppKit calls; the guard
  pointer is captured by the main-thread closure as a raw address of the
  managed `CefEmbedState` field, which outlives scheduled callbacks. Deleted
  the 6 `native_*_menu_*` commands, their impl shims, and
  `NativeMenuRectPayload`; removed `mod native_workbench_menu;` and deleted
  `native_workbench_menu.rs`.
- `mew-web-ui/src-tauri/tauri.conf.json`: `"transparent": true` on the main
  window and `"macOSPrivateApi": true` under `app` (required by Tauri for
  macOS webview transparency; blocks Mac App Store distribution, which is
  fine since mew ships directly). `Cargo.toml` gained the matching
  `macos-private-api` tauri feature; `objc2-app-kit` features trimmed to just
  `NSView` (NSMenu/NSMenuItem/NSResponder went away with the menus).
- Frontend: `host.ts` sets `data-host="desktop"|"web"` on `<html>` during
  `initializeHost()` and lost all 8 native-menu wrappers plus the two event
  types. `index.css` makes `body` transparent only under `[data-host="desktop"]`.
- `browser-panel.tsx`: viewport div is `bg-transparent` once CEF reports
  available (muted placeholder otherwise), the closed-browser empty state
  carries its own `bg-muted/20`, and a comment marks the hole as untouchable.
  The native browser-tools path (state, listener effect, button ref) is gone;
  the tools button always toggles the HTML popover.
- `right-rail.tsx`: the "+" button always opens the `CommandDialog` surface
  picker; native menu state/listener/ref deleted.
- `__tests__/host.test.ts`: dropped the native-menu no-op test, added a
  `data-host` attribute test. `right-rail.test.tsx` needed no changes — the
  picker was already the tested path.

## Verification

- `cargo test` on `mew-web-ui/src-tauri`: 15 passed (3 new layering-guard
  tests). `mew-cef-host`: 1 passed. Clippy `-D warnings` clean on both;
  `cargo fmt --check` clean.
- `pnpm --filter mew-web-ui test`: 112/112. `pnpm --filter mew-web-ui build`
  and `tsc --noEmit` clean.
- Not yet done: the live desktop smoke (`just desktop-dev`; hit-testing
  through the transparent hole is the open risk — if WKWebView swallows
  clicks over the hole, the fallback is dynamic ordering for the browser
  region, per PLAN.md risk #1) and `pnpm desktop:verify:cef`. If a desktop
  flash-through shows during resize, paint the NSWindow background to the
  theme color (PLAN.md step 2 contingency).

# 2026-07-19 — Fix inverted transparency: opaque chrome, truly see-through WKWebView

## Summary

First pass at the layering inversion got the transparency backwards: the
body was made transparent so genuine gaps in the page chrome (margins around
the floating session rail and inset panels) showed the desktop, while the
CEF hole itself stayed opaque white because wry's `transparent` flag only
sets the private `drawsBackground` config key — it never sets
`opaque = false` on the WKWebView or clears its layer background (verified
in wry 0.55.1 `wkwebview/mod.rs`; the `setOpaque(false)` +
`setBackgroundColor` path only runs for `background_color`, not
`transparent`).

Fix, both directions:

- `native_layering.rs::make_webview_transparent` (called at the start of
  `initialize_cef`) uses `with_webview` to set `setOpaque(false)` via
  msg_send (WKWebView responds to it; not a public NSView property) and
  clears the layer background color. Now the view composites nothing where
  the page doesn't paint, so the CEF view below shows through the hole.
- CSS: removed the `[data-host="desktop"] body { background: transparent }`
  rule; the body keeps `var(--background)`. The sidebar wrapper in
  `ui/sidebar.tsx` gained an explicit `bg-background` since it was relying
  on the body paint behind it. Only the `browser-panel` viewport hole is
  transparent, so only the CEF rect shows through.

The `data-host` attribute stays (harmless, tested hook for future
host-specific CSS).

## Verification

- mew-desktop: 15/15 tests, clippy `-D warnings` clean, fmt clean.
- mew-web-ui: 112/112 vitest, `tsc --noEmit` clean.
- Still owed: live `just desktop-dev` smoke — the hole should now show the
  CEF page, the rest of the window should be fully opaque, and clicks in the
  hole must reach CEF (PLAN.md risk #1).

# 2026-07-19 — WKWebView transparency take 3: drawsBackground on the view instance

## Summary

After the opaque-chrome fix, the window background looked right but the CEF
hole still painted the webview's default background. Cause: wry only sets
`drawsBackground = NO` on the WKWebViewConfiguration at construction; on the
live view the property that matters is the same private KVC key set on the
WKWebView *instance* (exactly what wry's own `set_background_color` runtime
path does in `wkwebview/mod.rs`: "On the webview instance (vs config) for
runtime changes"). The previous `setOpaque(false)` msg_send was a no-op.

`make_webview_transparent` now mirrors wry's runtime path: cast the handle to
`WKWebView`, `setValue:forKey:` `drawsBackground = NO` on the instance,
`setUnderPageBackgroundColor(clearColor)`, and clear the layer background
color. Added `objc2-web-kit` 0.3 and the `NSColor`/`NSString` features to the
desktop crate for this.

## Verification

- mew-desktop: 15/15 tests, clippy `-D warnings` clean, fmt clean.
- Still owed: live `just desktop-dev` smoke — hole should finally show the
  CEF page; clicks in the hole must reach CEF (PLAN.md risk #1).

# 2026-07-19 — Re-assert CEF layering on every visible claim

## Summary

Instance-level `drawsBackground = NO` at setup still wasn't enough: WebKit
re-enables background drawing as the page renders, so the hole went opaque
again after the first paint. The layering pass is now a steady-state
re-assertion instead of a one-shot:

- `native_layering::ensure_cef_layering` (renamed from
  `order_cef_below_webview`) re-applies webview transparency AND the
  CEF-below-WKWebView ordering on every call; all AppKit calls inside are
  idempotent.
- `cef_browser_set_rect` with `visible: true` now runs it on every claim
  (React's ResizeObserver fires these continuously while the browser tab is
  visible), not just once per CEF view handle. The `NativeLayeringGuard` is
  retained only as a record of which handles have been seen (and its unit
  tests); it no longer gates the pass.
- `make_webview_transparent` at setup stays as the initial pass before the
  first claim.

## Verification

- mew-desktop: 15/15 tests, clippy `-D warnings` clean, fmt clean.
- Still owed: live smoke — hole should stay transparent while the page
  renders; clicks in the hole must reach CEF.

# 2026-07-19 — Layering inversion abandoned: CEF stays on top (WebKit composites opaque)

## Summary

The PLAN.md goal — CEF below a transparent WKWebView with a transparent React
hole — is **not viable on this WebKit build**. Proven via live hierarchy
dumps: CEF reorders below the WKWebView correctly (`drawsBackground=0`, layer
bg null, CEF at subview [0]), yet the browser stays invisible until the
WKWebView is moved below CEF — at which point CEF composites fine. The
WebContent process paints an opaque background into the webview's remote
layer tree, and no AppKit-level transparency on the view punches through it.
`setDrawsBackground:` is gone from this WebKit (unrecognized selector), and
`setValue:forKey: drawsBackground` only writes wry's KVO subclass mirror.

What shipped instead (the stable baseline):

- `native_layering.rs`: stripped to `ensure_cef_on_top` — asserts CEF is the
  content view's topmost subview on every visible claim (CEF adds new views
  on top; nothing reorders it back). Guard + tests retained.
- `embed.rs`: browser is now created with `window_info.hidden = 1` so no
  window flashes before React claims it (kept from the underlay experiment —
  harmless, and CEF composites on top regardless).
- Reverted everything transparency-related: `tauri.conf.json` back to no
  `transparent`/`macOSPrivateApi`, Cargo back to no `macos-private-api`
  feature, `objc2-app-kit` trimmed to just `NSView`, dropped the
  `objc2`/`objc2-foundation`/`objc2-web-kit` deps. Frontend viewport hole,
  `data-host` hook, CSS transparency, sidebar-wrapper change all reverted.
- The AppKit-menu deletion from the earlier step stands (HTML popover +
  CommandDialog are the only menu implementations).

## Verified

- mew-desktop: 15/15 tests. mew-cef-host: 1/1. Clippy `-D warnings` both.
  fmt both.
- mew-web-ui: 111/111 vitest, `tsc --noEmit`, build. All green.
- App launches clean (the earlier panic was objc2's bool→BOOL encode check
  aborting across the FFI boundary; gone with the transparency code).

## Still open (the real overlay problem)

HTML overlays that overlap the browser rect (browser-tools popover, surface
picker, cmd-K palette, settings, sheets, toasts) are still under CEF. The
working approach, when prioritized: a native click-through NSView shield
above CEF that React drives via IPC with overlay rects/open state. Simpler
intermediate: hide the CEF surface while full-window modals (settings,
palette) are open, since only the tools popover + surface picker overlap the
browser during normal use.

# 2026-01-21 — hidden `mew funfact` easter egg

Implemented the hidden `mew funfact` CLI subcommand per PLAN.md.

Files touched:
- `crates/mew/src/cli.rs` — added `Commands::Funfact` with `#[command(hide = true)]`.
- `crates/mew/src/commands/funfact.rs` — new module with 8 fun facts, `pick_fact()` using `SystemTime`, and `funfact_cmd()`; includes tests for output membership, CLI parsing, and hidden help.
- `crates/mew/src/commands/mod.rs` — registered `pub mod funfact`.
- `crates/mew/src/main.rs` — imported `funfact_cmd` and dispatched the `Funfact` variant.

Verification:
- `cargo build -p mew` succeeded.
- `cargo run -p mew -- funfact` printed a random fact.
- `cargo run -p mew -- --help` does not list `funfact`.
- `cargo test -p mew` passed (118 unit tests + 3 integration tests).
- `cargo clippy -p mew` only reports the pre-existing `remote_invite_payload` dead-code warning; our new code is clean.

Notes:
- No new dependencies were added.
- The command uses `SystemTime` for low-quality randomness as specified.
- The `None` default arm was already placed in the middle of the match in `main.rs`, so the `Funfact` arm was inserted right before it there rather than at the end of the match.

# 2026-07-24 — hashline test + fun facts refresh

Tested the hashline patch editor on the hidden `mew funfact` easter egg.

Changes:
- `crates/mew/src/commands/funfact.rs` — replaced the frisbee fact with a technical one about the first computer bug, and added three more technical facts (ARPANET `LOGIN`, IPv6 address size, `sudo` etymology). Used `edit_hashline` for the patches; the first insert landed after the closing `];`, so a second patch corrected the array structure.

Verification:
- `cargo test -p mew-hashline` — 56 passed.
- `cargo test -p mew funfact` — 3 passed (membership, CLI parsing, hidden help).
- `cargo run -p mew -- funfact` — printed one of the new technical facts.

# 2026-07-31 — Animate native sidebar collapse

Added a quick 220ms ease-out width transition for the native sidebar. The
transition uses GPUI's animation wrapper, so the app's reduced-motion setting
still produces a static state rather than scheduling animation frames.

Verification:
- `cargo fmt --all` — clean.
- `cargo check -p mew-desktop` — clean.
- `cargo test -p mew-desktop` — 10 tests pass, including both collapse
  direction endpoint checks.
- `cargo build -p mew-desktop` — clean.
- Fresh native expanded-state screenshot inspected after rebuilding the debug
  binary. Computer-use accessibility cannot target this unbundled debug app,
  so the live toggle interaction still needs a bundled app or GPUI harness.

# 2026-07-31 — Bring required actions into the native shell

The shared client reducer's pending permission, workspace-access, subagent,
question, plan, and goal requests now cross the native presentation boundary.
The GPUI shell renders them as inline action cards with typed protocol
responses. Ask-user requests use the composer, with one answer per line.

Changes:
- Projected selected-session pending actions into `mew-ui-model`.
- Added action response controls for permission, plan, and goal requests.
- Added workspace and subagent permission presentation.
- Routed composer submission to `AskUserResponse` while a question request is
  pending.
- Requested full client snapshots for action arrival and resolution so the UI
  does not lose pending-action state during metadata updates.
- Added protocol equality derives needed by deterministic action projection.

Verification:
- `cargo test -p mew-desktop -p mew-ui-model -p mew-client-core` — 25 tests pass.
- `cargo clippy -p mew-desktop -p mew-ui-model -p mew-client-core --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo build -p mew-desktop` — clean.
- Fresh native connected-shell screenshot inspected after rebuild.
- Computer-use accessibility still cannot target the unbundled debug binary;
  action-card click-through needs a bundled app or GPUI harness.

# 2026-07-31 — Add the framework-independent native diff model

Added `mew-diff`, a small permissive Rust crate for native review surfaces.
It computes stable line-numbered hunks from old/new file contents using the
existing `similar` dependency, including context windows, additions,
deletions, renamed paths, empty files, and per-file counts.

Verification:
- `cargo test -p mew-diff` — 3 tests pass.
- `cargo clippy -p mew-diff --all-targets -- -D warnings` — clean.
- `cargo check -p mew-diff` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

The GPUI workbench is not wired to this model yet. The daemon currently sends
changed-path statistics rather than old/new file contents, so the next slice
must add a local repository loader with explicit stale/binary handling before
rendering actual hunks.

Notes:
- The only compiler warning is the pre-existing `remote_invite_payload` dead-code warning in `crates/mew/src/commands/daemon.rs`, unrelated to this work.

# 2026-07-31 — Connect the native workbench to local diffs

Connected the GPUI review workbench to the selected conversation's workspace.
The native shell now loads tracked and untracked files against `HEAD` on a
worker thread, reports binary and load errors explicitly, derives live file
status and line counts, and shows the selected file through a virtualized diff
line list. The daemon remains responsible for session-level change metadata;
the desktop client owns local file inspection so the protocol does not need to
carry full file contents.

Verification:
- `cargo fmt --all` — clean.
- `cargo check -p mew-desktop -p mew-ui-model -p mew-client-core -p mew-diff` — clean.
- `cargo test -p mew-desktop -p mew-ui-model -p mew-client-core -p mew-diff` — 30 tests pass.
- `cargo clippy -p mew-desktop -p mew-ui-model -p mew-client-core -p mew-diff --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `git diff --check` — clean.
- `cargo build -p mew-desktop` — clean.

The unbundled debug window launched successfully, but this macOS session's
screen-capture path returned a display shield instead of pixels for the native
window. Window geometry was still confirmed through CoreGraphics; interaction
coverage for selecting a file and opening its diff remains pending a bundled
debug target or GPUI harness that the computer-use driver can attach to.

# 2026-07-31 — Complete native turn controls and persona selection

Projected the shared client's active-turn state and persona catalog into the
native UI model. The GPUI composer now prevents accidental duplicate prompts
while a turn is running, provides a stop control that sends `Cancel`, and
preserves the question-answer path for unresolved ask-user actions. The model
picker is now paired with a native persona picker backed by `ListPersonas` and
`SwitchPersona`, including an explicit default option.

Verification:
- `cargo test -p mew-ui-model -p mew-desktop` — 15 tests pass.
- `cargo clippy -p mew-desktop -p mew-ui-model -p mew-client-core -p mew-diff --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo build -p mew-desktop` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

The rebuilt debug app creates its native 1116×713 window and attaches to the
daemon. ScreenCaptureKit and `screencapture` are both blocked by this session's
display shield, so the live stop/persona click path still needs a captureable
bundled target or GPUI harness.

# 2026-07-31 — Add native file attachments

Added a real GPUI file-drop path to the composer. Dropped files are checked for
regular-file status and a 10 MB limit, deduplicated, shown as removable chips,
and sent as protocol `Attachment` values with conservative MIME hints. Pending
prompts retain their attachments while a new session is being created, so the
native flow does not lose them during session readiness.

Verification:
- `cargo test -p mew-desktop -p mew-ui-model -p mew-client-core -p mew-diff -p mew-protocol` — 137 tests pass.
- `cargo clippy -p mew-desktop -p mew-ui-model -p mew-client-core -p mew-diff --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo build -p mew-desktop` — clean; native window geometry confirmed at 1116×713.
- `cargo fmt --all` and `git diff --check` — clean.

The next parity cut remains typed command routing and a real terminal/PTY;
file-drop click-through is still subject to the macOS display shield noted
above.

# 2026-07-31 — Centralize native shell commands

Added a typed `ShellCommand` seam and one dispatcher for the native shell's
keyboard shortcuts and panel/conversation controls. Sidebar, workbench,
terminal, and new-conversation actions now share the same routing path;
platform-modifier and composer-focus behavior is covered by direct tests.

Verification:
- `cargo test -p mew-desktop` — 12 tests pass.
- `cargo clippy -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo build -p mew-desktop` — clean; native window geometry confirmed at 1116×713.
- `cargo fmt --all` and `git diff --check` — clean.

The command seam is ready to absorb command-palette and application-menu
bindings. The remaining major desktop boundary is still a real PTY surface;
the current terminal strip is intentionally only a shell placeholder.

# 2026-07-31 — Bootstrap a native PTY terminal surface

Added the framework-independent `mew-pty` crate and connected it to the native
GPUI terminal strip. The shell now owns a real pseudo-terminal with worker
thread input/output, resizing, termination, bounded output retention, and
basic control-sequence cleanup. The existing 220 ms `ease_out_quint` sidebar
transition remains the visual model for quick, non-instant panel motion.

This is an intermediate native bootstrap. Daemon-backed PTY ownership and a
full terminal grid with ANSI state, scrollback, copy, and search remain the
next terminal parity seam; the current surface displays sanitized PTY text.

Verification:
- `cargo test -p mew-desktop -p mew-pty -p mew-ui-model -p mew-client-core -p mew-diff -p mew-protocol` — 141 tests pass.
- `cargo clippy -p mew-desktop -p mew-pty -p mew-ui-model -p mew-client-core -p mew-diff --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all -- --check` and `git diff --check` — clean.
- `cargo build -p mew-desktop` — clean; native window geometry confirmed at 1116×713.

The macOS display shield still prevents screenshot pixels and computer-use
click-through for the unbundled debug GPUI window.

# 2026-07-31 — Move native terminal ownership into the daemon

Added a daemon-backed terminal protocol with open, input, resize, and close
commands plus opened, raw output, exited, and error events. The daemon now
starts one PTY in the attached session workspace, owns its process lifecycle,
and streams bytes over the existing WebSocket connection. The GPUI client
opens the terminal after `SessionReady`, routes keyboard input through the
shared client reducer, and closes the terminal when changing sessions or
shutting down.

This keeps shell process ownership with the daemon while leaving terminal
emulation in the client. The current GPUI view still sanitizes raw output into
a bounded text buffer; ANSI state, scrollback, copy/search, and pixel-aware
resize remain the next terminal-grid pass.

Verification:
- `cargo test -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon --test e2e --lib` — 156 tests pass, including the daemon WebSocket PTY e2e.
- `cargo test -p mew-desktop` — 13 tests pass.
- `cargo clippy -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant -A clippy::items_after_test_module -A clippy::too_many_arguments` — clean after suppressing two pre-existing daemon lints.
- `cargo build -p mew-desktop`, `cargo fmt --all`, and `git diff --check` — clean.

# 2026-07-31 — Make native GPUI the default desktop workflow

Changed `desktop-dev`, `desktop-build`, and `desktop-install` to use the
native GPUI client. The previous Tauri workflows remain available under
explicit `*-tauri` recipes while the cutover is validated. Added a macOS
bundle script and `Info.plist`; the resulting app places `mew` beside
`mew-desktop` so the existing supervisor can launch the app-owned daemon.

The terminal viewport now measures its actual GPUI layout bounds during
prepaint, derives rows and columns from the active text metrics, resizes the
client grid, and sends a matching daemon resize only when dimensions change.
Adding terminal protocol variants also required updating the TUI capture
message-type table, which is now exhaustive again.

Verification:
- `just desktop-build` — release binaries built and `target/release/bundle/macos/mew.app` packaged successfully.
- Bundle smoke checks — both Mach-O executables are present and executable; `Info.plist` identifies `com.mew.desktop`; native links are system frameworks only.
- `cargo test -p mew` — 120 tests pass.
- `cargo test -p mew-desktop` — 13 tests pass.
- `cargo test -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon --test e2e --lib` — 160 tests pass.
- `cargo clippy -p mew -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant -A clippy::items_after_test_module -A clippy::too_many_arguments` — clean after suppressing two pre-existing daemon lints.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Animate the workbench rail

The right workbench now uses the same 220ms eased width transition as the
sidebar when it moves between its 360px expanded rail and 56px collapsed
rail. The transition is wrapped in an overflow-hidden container so the
content does not flash outside the rail while it moves.

Verification:
- `cargo test -p mew-desktop` — 14 tests pass, including both rail transition endpoint tests.
- `cargo clippy -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Smoke-test the native launch path

Launched the debug GPUI client with its app-owned debug daemon, observed no
startup panic, and stopped it cleanly. Rebuilt the release package after the
layout persistence change; the macOS bundle still contains executable native
client and daemon binaries and passes `plutil -lint`.

# 2026-07-31 — Persist the native window frame

Added `DesktopWindowState` to the shared configuration state. The GPUI client
restores a valid saved frame at startup and observes live window-bound changes
to save the frame back to the same state file used for layout preferences.
Invalid or undersized persisted frames are ignored in favor of the centered
default.

Verification:
- `cargo test -p mew-config -p mew-desktop` — 121 config tests and 15 desktop tests pass.
- `cargo clippy -p mew-config -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

An isolated live launch with `XDG_CONFIG_HOME` and `MEW_SESSION_DIR` pointed
at a temporary directory also completed without a startup panic and left no
new daemon process behind after shutdown.

# 2026-07-31 — Persist native shell layout

The GPUI shell now reads and writes rail and disclosure state through the
existing `mew_config::State.sidebar_collapsed` map. Sidebar, workbench,
terminal, and section toggles persist across launches, and shutdown writes a
final snapshot without introducing a desktop-only preferences file.

Verification:
- `cargo test -p mew-desktop` — 15 tests pass, including shared-state layout round-trip coverage.
- `cargo clippy -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Expose terminal search and copy controls

Added a compact terminal search field with match counts, a copy-all action
using the native GPUI clipboard, and scrollback history hinting in the
terminal toolbar. Removed the unused terminal-tab plus affordance and named
the compact grid dimensions so the daemon and client use the same expanded
viewport size.

Verification:
- `cargo test -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon --test e2e --lib` — 160 tests pass.
- `cargo test -p mew-desktop` — 12 tests pass.
- `cargo clippy -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant -A clippy::items_after_test_module -A clippy::too_many_arguments` — clean after suppressing two pre-existing daemon lints.
- `cargo build -p mew-desktop`, `cargo fmt --all`, and `git diff --check` — clean.

# 2026-07-31 — Add a stateful native terminal grid

Added `TerminalGrid` to the framework-independent PTY crate. It preserves
ANSI parser state across output chunks, tracks cursor movement, erase and
scroll regions, SGR colors/styles, UTF-8 fragments, bounded scrollback, and
resize behavior. The GPUI terminal now renders only the bounded viewport as
styled rows and supports wheel scrolling; copy-range and search operate on
the same logical line model.

The remaining terminal polish is wiring visible search/copy affordances and
deriving rows/columns from the actual terminal body bounds instead of the
current compact fixed grid dimensions.

Verification:
- `cargo test -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon --test e2e --lib` — 160 tests pass, including the daemon WebSocket PTY e2e.
- `cargo test -p mew-desktop` — 12 tests pass.
- `cargo clippy -p mew-protocol -p mew-pty -p mew-client-core -p mew-daemon -p mew-desktop --all-targets -- -D warnings -A clippy::large_enum_variant -A clippy::items_after_test_module -A clippy::too_many_arguments` — clean after suppressing two pre-existing daemon lints.
- `cargo build -p mew-desktop`, `cargo fmt --all`, and `git diff --check` — clean.

# 2026-07-31 — Verify native rail motion visually

Rebuilt and launched the bundled GPUI app separately from the installed Tauri
bundle. Live interaction confirmed that the left sidebar transitions between
its expanded and compact rails, and the right workbench transitions between
its full and compact rails while clipping its contents during the move. The
220ms eased motion reads as quick but perceptible at the current shell scale.

Verification:
- `just desktop-build` — release binaries built and the native macOS bundle packaged successfully.
- `cargo test -p mew-desktop` — 15 tests pass, including both rail endpoint tests.
- `cargo test -p mew-config`, `cargo fmt --all -- --check`, and `git diff --cached --check` — clean.

# 2026-07-31 — Add the native browser portal boundary

Added the Tauri-free `mew-browser-host` boundary around the existing CEF
child-view controller. The native shell now owns browser visibility,
navigation, typed address/title events, panel geometry, and owner checks while
the shared client core projects daemon browser state into the UI model. The
native package now carries the CEF framework and helper alongside the GPUI
client and daemon.

The live bundle reaches CEF context initialization, creates a native child
view, pumps CEF from the AppKit main thread, and exposes an `https://example.com/`
CDP page target when launched with an isolated cache and debug port. The
visible child surface remained blank during this pass, so the browser portal
stays in progress pending the final pixel/compositing fix.

Verification:
- `cargo test -p mew-browser-host -p mew-client-core -p mew-ui-model -p mew-desktop` — 35 tests pass.
- focused clippy for those packages — clean with the repository’s documented baseline lint suppressions.
- `just desktop-build` — native macOS bundle packaged with arm64 GPUI client, daemon, CEF framework, and helper.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Decouple native browser packaging from the Tauri shell

Updated the native macOS packager to accept `MEW_CEF_FRAMEWORK_SOURCE`,
`CEF_PATH`, and `MEW_CEF_HELPER_PATH` before falling back to the existing local
asset cache. The GPUI bundle can now be assembled from explicitly owned CEF
inputs without requiring the Tauri build workflow, while retaining a useful
fallback for the current checkout.

Verification:
- `bash -n scripts/package-desktop-native.sh` — clean.
- `just desktop-build` — release binaries and native app bundle built successfully.
- packaging with explicit framework/helper environment paths — succeeded.
- `git diff --check` and `git diff --cached --check` — clean.

Live visual verification remains paused because the macOS session is locked.

# 2026-07-31 — Complete the native browser focus boundary

Added `focus(owner)` to the Tauri-free browser portal and kept the CEF host
handle private. The GPUI shell requests focus once after the browser receives
its first visible measured rectangle, so opening the browser produces a usable
native child without repeatedly stealing focus from the composer or terminal.
The native package now builds its own CEF helper target as part of
`desktop-build`; the old Tauri-produced helper is only a compatibility
fallback.

Verification:
- focused browser/desktop tests — 18 tests pass.
- focused clippy for CEF, browser host, and desktop — clean.
- `just desktop-build` — native client, daemon, helper, framework, and helper app bundles packaged successfully.
- packaged helper checksums match the workspace-built helper binary.
- `cargo fmt --all` and `git diff --check` — clean.

Live pixel and keyboard verification remains pending until the macOS session is
unlocked.

# 2026-07-31 — Restore markdown wrapping inside native transcript rows

Made the native markdown layout use a definite, shrinkable transcript width so
paragraphs, headings, bullets, quotes, code blocks, and table rows wrap within
the conversation column. List content now lives in a flex-shrinkable column,
and hard-wrapped continuation lines in list items are preserved as one logical
bullet instead of being dropped by the parser.

Verification:
- `cargo test -p mew-desktop` — 21 tests pass, including long and hard-wrapped list cases.
- focused desktop clippy — clean with the existing documented baseline suppressions.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt native bundle — long bullets wrap in the transcript with aligned continuation lines.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Compact native chrome and Tabler controls

Moved the native shell into a transparent macOS titlebar so compact conversation
tabs sit inline with the traffic lights. Removed the redundant product label
from that row, kept new conversation creation in the sidebar, and replaced
visible unicode controls with a small local Tabler SVG asset layer. The same
icons now cover rail/workbench toggles, disclosures, model/persona controls,
terminal actions, browser actions, and composer actions. All colors continue to
come from the shared theme schema.

Verification:
- `cargo test -p mew-desktop` — 19 tests pass, including the icon asset test.
- focused desktop clippy — clean with the repository's documented baseline suppressions.
- `just desktop-build` — native release bundle and CEF helper package successfully.
- live inspection of the packaged `com.mew.desktop` bundle — tabs, sidebar collapse,
  workbench collapse, tab selection, and icon rendering all exercised successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Bundle and persist native terminal typography

The native GPUI terminal now embeds the existing `Ioskeley Mono` regular and
medium TTF assets into GPUI's text system. The terminal resolves that family
by its actual registered name, measures cells against the selected family, and
persists the choice in shared config state. A compact terminal toolbar picker
offers the bundled mono, SF Mono, and Menlo, with the bundled family as the
default.

Verification:
- `cargo test -p mew-config -p mew-desktop` — 121 config tests and 17 desktop tests pass.
- focused desktop clippy — clean with documented baseline suppressions.
- `just desktop-build` — native release bundle and CEF helper package successfully.
- `cargo fmt --all` and `git diff --check` — clean.

Live visual interaction remains pending until the macOS session is unlocked.

# 2026-07-31 — Bound transcript markdown work per virtualized row

Improved native transcript scrolling by bounding oversized markdown blocks into
small render rows, replacing per-word GPUI elements with one styled text layout
per visible row, and reducing transcript overdraw from 2048 to 512 pixels. The
list now remeasures only changed message ranges. Append-only streaming updates
splice new rows and preserve the current scroll position instead of resetting
the entire list.

Verification:
- `cargo test -p mew-desktop -p mew-config` — 121 config tests and 18 desktop tests pass.
- focused desktop clippy — clean with documented baseline suppressions.
- `just desktop-build` — native release bundle and CEF helper package successfully.
- `cargo fmt --all` and `git diff --check` — clean.

Live scroll profiling remains pending until the macOS session is unlocked.

# 2026-07-31 — Move native terminal rendering to libghostty

Replaced the desktop shell's custom terminal grid renderer with a vendored
`libghostty-vt` terminal view based on the existing Artemis GPUI integration.
The daemon still owns the PTY and sends raw terminal bytes over the existing
protocol. The native client now feeds those bytes into a separate GPUI entity
with libghostty-managed scrollback, viewport scrolling, dirty snapshots, and
cached rows. Terminal output batches no longer invalidate the whole shell.
The default foreground and panel background continue to come from the shared
theme schema.

Verification:
- `cargo test -p mew-desktop` — 17 tests pass.
- `cargo clippy -p mew-desktop --all-targets -- -D warnings ...` — clean.
- `just desktop-build` — native release bundle and CEF helper package successfully.
- `cargo fmt --all` and `git diff --check` — clean.

The standalone `libghostty-vt-sys` test binary still hits an Apple linker
alignment error from the upstream Zig-produced static archives; the native
release desktop bundle links and packages successfully.

# 2026-07-31 — Keep streaming updates on the lightweight path

Avoided cloning the full native UI metadata projection for every streamed text
delta and terminal output event. Those high-frequency events now update their
own render models directly; session, action, browser, and connection events
still request the full metadata sync they need.

Verification:
- `cargo test -p mew-desktop` — 17 tests pass, including the event classification regression test.
- focused desktop clippy with the repository baseline suppressions — clean.
- `cargo fmt --all` — clean.

# 2026-07-31 — Make native CEF development self-contained

The GPUI debug workflow now builds the CEF helper itself and discovers the
framework from the native environment (`MEW_CEF_FRAMEWORK_SOURCE`, `CEF_PATH`,
the standard local CEF cache, or the packaged app) before starting the client.
This removes the former requirement to run Tauri’s CEF preparation scripts just
to exercise the native browser during development.

Verification:
- `cargo test -p mew-desktop` — 16 tests pass.
- focused desktop clippy with the repository baseline suppressions — clean.
- `just desktop-build` — native release client, daemon, helper, and browser bundle package successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Extract the iroh desktop transport

Added `mew-client-iroh`, a framework-independent implementation of the shared
client transport contract. It dials a daemon by NodeId over the existing
`mew/wire/0` ALPN, upgrades the QUIC stream to the daemon’s JSON WebSocket
protocol, sends the authenticated `RemoteHello`, and exposes normal typed
messages to `mew-client-core`. This keeps remote transport concerns out of the
GPUI view layer and gives the native client the same transport seam as local
WebSocket connections.

Verification:
- `cargo test -p mew-client-iroh` — adapter validation passes.
- `cargo clippy -p mew-client-iroh --all-targets -- -D warnings` — clean.
- `cargo fmt --all` and `git diff --check` — clean.

The native shell still needs a user-facing remote connection profile before
this adapter becomes selectable from the desktop UI.

# 2026-07-31 — Wire native iroh profiles into the desktop client

The GPUI client can now select an explicit remote profile through
`MEW_DESKTOP_IROH_NODE_ID`, with optional `MEW_DESKTOP_IROH_TOKEN` and
`MEW_DESKTOP_IROH_DEVICE_NAME`. When present, the app skips local daemon
supervision, creates an app-owned iroh endpoint, and runs the same
`ClientEngine` against `mew-client-iroh`. Local WebSocket supervision remains
the default path.

Verification:
- `cargo test -p mew-desktop -p mew-client-iroh` — 17 tests pass.
- focused clippy for both packages — clean with documented baseline suppressions.
- `just desktop-build` — release app and native CEF helper package successfully with iroh linked into the desktop binary.
- `cargo fmt --all`, `git diff --check`, and staged diff checks — clean.

The next native connection slice is a profile picker and persistence layer;
environment selection is intentionally the first integration seam.

The environment profile is now wired through the actual GPUI client startup:
when `MEW_DESKTOP_IROH_NODE_ID` is present, local supervisor startup is skipped,
an iroh endpoint is created on the client runtime thread, and the native shell
uses `IrohTransport` with the same command/event loop as local WebSocket.

Verification:
- `cargo test -p mew-desktop -p mew-client-iroh` — 17 tests pass.
- focused clippy for both packages — clean with documented baseline suppressions.
- `just desktop-build` — release app and native CEF helper package successfully.
- `cargo fmt --all`, `git diff --check`, and staged diff checks — clean.

# 2026-07-31 — Persist native connection profiles

Added native desktop remote-profile persistence to the shared config state.
Profiles retain only a display name, daemon NodeId, and device name; pairing
tokens remain outside the state file. The GPUI topbar now opens a compact
profile picker for the local daemon and saved remote identities. Choosing a
profile records it for the next launch, while the current session remains
stable and clearly reports that restart is required.

Verification:
- `cargo test -p mew-config -p mew-desktop` — 137 tests pass.
- focused clippy for config and desktop — clean with documented baseline suppressions.
- `just desktop-build` — native release bundle and CEF helper package successfully.
- `cargo fmt --all`, `git diff --check`, and staged diff checks — clean.

# 2026-07-31 — Keep mobile protocol translation compatible

Added explicit no-op handling for the native terminal server events in the
legacy mobile `CoreEvent` projection. The shared client reducer still receives
and stores those events for newer clients; mobile simply does not expose a
terminal surface yet.

Verification:
- `cargo test -p mew-mobile-core` — 26 tests pass.
- `cargo test --all` — workspace tests and doc tests pass.
- `cargo fmt --all`, `git diff --check`, and staged diff checks — clean.

# 2026-07-31 — Remove the legacy Tauri desktop shell

Completed the desktop cutover. Removed `mew-web-ui/src-tauri`, its generated
CEF/sidecar preparation scripts, Tauri package dependencies, and explicit
legacy desktop recipes. The React application remains available as a web
client, but its host and browser surfaces now use only the daemon WebSocket
protocol. Native desktop packaging owns CEF discovery and no longer falls back
to assets from the deleted shell.

Verification:
- `pnpm --filter mew-web-ui test` — 107 tests pass.
- `pnpm --filter mew-web-ui build` — production web client builds successfully.
- `pnpm --filter @mew/web-client test` — 16 tests pass.
- `pnpm --filter @mew/web-client build` — TypeScript client builds successfully.
- `cargo test --all` — workspace tests and doc tests pass.
- `just desktop-build` — native GPUI release bundle and CEF helper packages successfully.
- `just arch-check`, `just theme-codegen-check`, `cargo fmt --all -- --check`, and `git diff --check` — clean.

Live pixel and keyboard verification remains pending until the macOS session is
unlocked.

# 2026-07-31 — Make user transcript turns content-sized

Updated native transcript rows so assistant content can remain full-width while
user turns use a right-aligned, shrink-to-content flex item capped at 650px.
User rows also get a small separation so adjacent short prompts do not fuse into
one continuous card.

Verification:
- `cargo test -p mew-desktop` — 21 tests pass.
- focused desktop clippy — clean with the existing documented baseline suppressions.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Inset the conversation and remove collapsed rails

Updated the native chrome so active tab icons use the theme accent text token,
the conversation surface sits inside an 8px inset rounded shell, and both
navigation sidebars animate to zero width instead of settling on icon rails.

Verification:
- `cargo test -p mew-desktop` — 21 tests pass, including zero-width collapse targets.
- focused desktop clippy — clean with the existing documented baseline suppressions.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — inset center and cyan active tab icon visible.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Inset the full native workbench

Moved the native shell onto a darker theme-backed outer frame with a shared 8px
gutter. The navigation rail, conversation/workspace, and review workbench now
render as independent rounded surfaces, and the terminal is its own card below
the conversation rather than a divider inside it.

Verification:
- `cargo test -p mew-desktop` — 21 tests pass.
- focused desktop clippy — clean with the existing documented baseline suppressions.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — inset shell and separate terminal surface visible.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Tighten the tab-to-shell edge

Removed the top body gutter beneath the tab bar while preserving the horizontal
and bottom insets, so the rounded work surfaces meet the tab row cleanly.

Verification:
- `cargo test -p mew-desktop` — 21 tests pass.
- focused desktop clippy — clean with the existing documented baseline suppressions.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — panels now start directly below the tab bar.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Add the native settings page

Added an in-app settings surface rather than opening a second native window.
The sidebar footer and collapsed rail now expose a settings control, and the
page includes appearance status, terminal font selection, workspace layout
toggles, daemon connection status, and a back control. Settings content scrolls
for smaller windows and uses the shared theme tokens and Tabler icon assets.

Verification:
- `cargo test -p mew-desktop` — 21 tests pass.
- focused desktop clippy — clean with the existing documented baseline suppressions.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — settings gear visible in the native sidebar footer.
- `cargo fmt --all` and `git diff --check` — clean.

Note: direct Computer Use clicks against the native bundle remained unavailable
because the local accessibility bridge reported `noWindowsAvailable`; the page
was still verified through the native bundle screenshot and compile/test path.

# 2026-07-31 — DeepSeek V4 Flash over the Responses API

`deepseek/deepseek-v4-flash` now uses the OpenAI Responses transport
(`POST /v1/responses`) while the rest of the DeepSeek lineup stays on chat
completions. models.dev has no per-model transport field, so the override is
hardcoded in `hardcoded_shape_override` (alongside the existing opencode-go
minimax precedent) and wins over both provider config and catalog shape. The
responses arm's auth is gated with `responses_uses_codex_oauth` so stored Codex
OAuth tokens are only ever sent to OpenAI; deepseek authenticates with its own
API key.

Verification:
- `cargo test -p mew` — 121 tests pass, including 4 new override/auth-gate tests.
- `cargo clippy -p mew --all-targets` — clean for `mew`; two pre-existing
  `too_many_arguments` warnings remain in the uncommitted mew-daemon work.
- `cargo fmt -p mew -- --check` — clean.

# 2026-07-31 — Split settings into pages and a top-bar tab

Turned settings into a multi-page surface with General, Terminal, Workspace,
and Connection categories. Added a persistent Settings tab beside conversation
tabs, kept the settings content vertically scrollable, and retained the
sidebar entry and back navigation.

Verification:
- `cargo test -p mew-desktop` — 22 tests pass, including settings navigation metadata.
- focused desktop clippy — clean with the existing documented baseline suppressions.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — Settings tab visible in the native top bar.
- `cargo fmt --all` and `git diff --check` — clean.

Note: direct Computer Use clicks against the native bundle remained unavailable
because the local accessibility bridge reported `noWindowsAvailable`.

# 2026-07-31 — Replace local workspace rail with daemon-backed groups

Removed the local-workspace card from the native sidebar and replaced the
spaces/session split with one virtualized row stream. The stream now contains a
groups toolbar, collapsible daemon groups, an ungrouped fallback, and session
rows. Session rows can move a session to any loaded group or back to no group;
the plus action creates a numbered group through the daemon. Group state and
session membership now flow through the shared client reducer and UI model.
Archive and pin broadcasts preserve existing group membership.

Verification:
- `cargo test -p mew-client-core -p mew-ui-model -p mew-daemon -p mew-desktop` — all focused tests pass, including group reducer coverage.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — single `GROUPS` rail and `All sessions` fallback visible; no local workspace card.
- `cargo fmt --all` and `git diff --check` — clean.

Note: full focused clippy with `-D warnings` remains blocked by existing
`large_enum_variant` and `too_many_arguments` findings in the native migration
and daemon code; no new warning was reported by the normal test/build checks.

# 2026-07-31 — Fix grouped sidebar row overlap

Replaced the grouped sidebar's `uniform_list` with GPUI's variable-height
virtual `list`. The previous implementation mixed toolbar, group, session, and
picker heights inside a uniform list, causing session text to paint over later
rows. The list now measures visible rows independently and remeasures a session
when its group picker opens.

Verification:
- `cargo test -p mew-desktop` — 22 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — the sidebar renders without row overlap.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Make session rows single-line

Reduced session rows to a compact one-line layout. Titles now use the smaller
text token and truncate with an ellipsis; status and the group control share
the same line, keeping the virtual list dense and height-stable.

Verification:
- `cargo test -p mew-desktop` — 22 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — compact one-line sidebar treatment visible.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Constrain title truncation and pad the rail scroll surface

Made the session title's width budget explicit with hidden overflow, nowrap,
and ellipsis styling. Added padding to the virtual sidebar list itself so the
scrollable content owns its inset rather than relying on row margins.

Verification:
- `cargo test -p mew-desktop` — 22 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — long titles truncate cleanly and the rail content is padded.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Guarantee visible session title ellipses

Added whitespace normalization and a compact 24-character title cap with a
literal ellipsis, alongside the flex width and overflow constraints. Status
and group controls are now explicitly non-shrinking so the title owns the
remaining width budget.

Verification:
- `cargo test -p mew-desktop` — 23 tests pass, including title normalization coverage.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — native rail rebuilt successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Show session paths beneath width-truncated titles

Removed the idle label and folder glyph from session rows. Titles now use the
actual flex width for truncation, while each session's working directory is
shown as a muted, smaller second line. The group action remains available from
a neutral dots control.

Verification:
- `cargo test -p mew-desktop` — 22 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt native rail — bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Render session paths relative to home

Session working directories now display from the home prefix, so paths such as
`/Users/natalie/code/mew` render as `~/code/mew`. The home directory itself
renders as `~`, while paths outside home remain absolute.

Verification:
- `cargo test -p mew-desktop` — 23 tests pass, including home-relative path coverage.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt native bundle — rail remains clean in the fresh shell state.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Hover session actions over compact titles

Session titles now normalize whitespace and cap at 28 characters with a literal
ellipsis, keeping the sidebar readable at narrow widths. The group action is
right-aligned over the title and appears only while its row is hovered, with a
theme-derived fade preserving the title’s visual width. The action remains
clickable and opens the existing group picker.

Verification:
- `cargo test -p mew-desktop` — 24 tests pass, including title truncation coverage.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — ellipses are visible, the hover dots appear over the title, and clicking them opens the group picker.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Center chat content at a readable measure

The transcript and composer now share a 760 px maximum content width and are
centered within the conversation surface. The surrounding conversation shell
and terminal remain full-width, preserving the native workbench layout while
giving the chat a calmer reading measure.

Verification:
- `cargo test -p mew-desktop` — 24 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — transcript and composer are centered at the shared measure, with the terminal still spanning the center pane.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Add an auxiliary bar and hideable terminal surface

The right workbench now behaves like a compact auxiliary bar with one active
view at a time for Browser, Changes, Local, and Activity. A draggable divider
between chat and the auxiliary bar resizes it within 280–560 px. The terminal
toggle moved into the top-right shell header, and a collapsed terminal now
removes its entire surface from layout rather than leaving a strip behind.

Verification:
- `cargo test -p mew-desktop` — 25 tests pass, including divider width bounds.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — auxiliary tabs render in the right rail and the collapsed terminal leaves no visible region.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Move auxiliary navigation to a vertical rail

Moved the auxiliary view switcher from the workbench header into a slim
vertical rail on the workbench’s left edge, matching the quieter VS Code-style
auxiliary bar direction. Reduced the active panel’s main horizontal insets by
half so the new rail does not make the content feel cramped.

Verification:
- `cargo test -p mew-desktop` — 25 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — vertical auxiliary rail and tighter panel insets are visible while the centered chat measure remains unchanged.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Clarify session hierarchy in the navigation rail

Renamed the rail header to `SESSIONS` and made its plus action start a new
conversation. Real groups and `Ungrouped` own the session rows directly, with
increased indentation for the child rows and no extra pseudo-group or filter
level.

Verification:
- `cargo test -p mew-desktop` — 25 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — session header, group container, and indented sessions are clear.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Add grouped-session creation and drag/drop

Added a group action beside the session controls, a per-group new-session
action, and GPUI drag sources/drop targets for moving sessions between groups.
New sessions created from a group use a dedicated protocol message so the
daemon validates the group and persists membership before attaching the client.
Existing group-picker actions remain available as a precise fallback.

Verification:
- focused daemon, desktop, UI-model, protocol, and session tests pass, including an end-to-end grouped-session metadata test.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt bundle — group creation and per-group session affordances remain compact and readable.
- live drag smoke test — an ungrouped session can be dropped onto `Ungrouped` without disturbing the rail.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Restore native composer deletion actions

Bound composer-scoped backspace and delete actions through GPUI's keymap while
keeping text and IME replacement on `EntityInputHandler`. Deletion now walks
UTF-8 character boundaries, so multi-byte characters are removed as complete
characters.

Verification:
- `cargo test -p mew-desktop` — all 26 tests pass, including UTF-8 deletion boundary coverage.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt native bundle — the composer renders as a focused, multiline editing surface with its control row separated below.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Align composer editing with GPUI's native input model

Reworked the composer around the same split used by Zed's GPUI text input:
key actions own cursor movement, selection, deletion, and navigation, while
`EntityInputHandler` owns text and IME replacement. Added grapheme-safe
boundaries, marked-text state, selection-aware replacement, shift selection,
cmd-a, home/end, and selection colors from the shared theme schema.

Verification:
- `cargo test -p mew-desktop` — all 26 tests pass, including combining-mark and emoji boundary coverage.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt native bundle — the composer remains centered, multiline, and visually consistent with the existing shell.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Add composer selection and caret motion polish

Added mouse-driven text selection, drag extension, shift-click extension, and
selection-aware caret rendering. The caret now uses a cancellable 500 ms blink
epoch that restarts after editing or moving the selection, following the same
focus-sensitive approach used by Zed's editor.

Verification:
- `cargo test -p mew-desktop` — all 26 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- visual inspection of the rebuilt native bundle — the composer remains correctly laid out after the input interaction changes.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Show the focused empty composer caret

Made the insertion caret represent focus rather than existing content. The
empty composer now displays a blinking caret at position zero beside its
placeholder, and focus events restart the blink cycle.

Verification:
- `cargo test -p mew-desktop` — all 26 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Add native transcript text selection

Evaluated `gpui-component` against the desktop shell's pinned Zed GPUI
revision. The published package depends on the crates.io `gpui 0.2.2`, so it
would introduce an incompatible second GPUI dependency rather than compose
with the native shell. Kept the dependency out of the workspace and adopted
the useful selection pattern directly instead.

Markdown inline text in virtualized transcript rows now supports mouse drag
selection, selection-aware syntax highlighting, platform copy, and shared
`selection.foreground` / `selection.background` theme tokens. Selection is
cleared when switching or creating conversations. The implementation is
deliberately narrow for this first slice: it selects within one rendered inline
block, leaving cross-row selection for a later selection registry pass.

Verification:
- `cargo test -p mew-desktop` — all 27 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.
- `cargo clippy -p mew-desktop -- -D warnings` reaches unrelated existing
  `mew-message` large-enum-variant errors outside this change.

# 2026-07-31 — Extend transcript selection across rendered rows

Moved transcript selection toward gpui-component's window-level model by
registering the visible markdown text layouts and resolving drag movement from
the transcript surface. Selection can now span adjacent rendered markdown
blocks in either direction, and copy joins the selected visible blocks with
line breaks. The registry remains bounded by the existing virtualized list, so
off-screen transcript rows are not eagerly laid out.

Reviewed the remaining gpui-component patterns as well:

- its `InputState` still assumes the component `Root` and global action/theme
  registries, so the custom composer remains the safer fit for mew's daemon,
  model, attachment, and shared-theme behavior;
- its resizable panel group is useful as a general constraint model, but the
  desktop shell already has a single persisted workbench split with GPUI drag
  events and explicit min/max bounds;
- its virtual list and text view use `measure_all` to stabilize unknown heights,
  which is inappropriate for streamed chat at the current performance target;
- its global theme would duplicate `theme_manifest.json`, so no second theme
  registry was introduced.

Verification:
- `cargo test -p mew-desktop` — all 28 tests pass, including bidirectional
  cross-row selection range coverage.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Tune caret and selection contrast

Made the composer insertion caret explicitly white and changed composer and
transcript selection fills to use the shared `selection.background` theme
token at 28% opacity. The existing selection foreground token remains in use,
so this keeps selection colors controlled by the current theme schema without
adding a second set of UI colors.

Verification:
- `cargo test -p mew-desktop` — all 28 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Add syntax coloring to fenced code

Reused the existing `ratatui-mdstream` Syntect/two-face assets for native
markdown code fences. Token runs are converted into GPUI range highlights for
colors, bold, italic, and underline, while code language labels remain shown
only on the first virtualized line. Inline code keeps the mono face without
syntax coloring because it has no language context.

Verification:
- `cargo test -p mew-desktop` — all 29 tests pass, including syntax-color
  coverage for Rust fences.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — opened a Rust code conversation in the fresh
  native bundle and visually confirmed colored fenced code.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Render reasoning and tool parts in chat

Preserved typed transcript parts through the native UI model instead of
flattening assistant messages into plain text. The desktop renderer now shows
thinking blocks, tool calls with queued/running/done/failed states, inputs,
outputs, errors, attachments, and compaction markers. Thinking and tool rows
share a compact disclosure treatment, so long execution details can be
collapsed without leaving the chat surface. Tool progress now updates the
running tool part in the client reducer and reuses the existing virtualized
transcript list.

The composer caret now uses the shared `foreground` theme token, and its text
selection uses the shared selection background at 18% opacity. Transcript
selection remains at 28% opacity because it sits directly on the chat surface.

Verification:
- `cargo test -p mew-client-core -p mew-ui-model -p mew-desktop` — all 48
  focused tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — opened a tool-rich session and toggled both
  thinking and tool disclosures.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Use the bundled mono face for code

Applied the bundled `Ioskeley Mono` family to inline markdown code spans,
fenced code blocks, and tool input/output. Prose continues to inherit the
normal UI font, while code now matches the terminal typography.

Verification:
- `cargo test -p mew-desktop` — all 28 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-07-31 — Make conversation tabs horizontally scrollable

The top conversation strip now keeps tab widths stable inside a tracked
horizontal scroller, so the connection and panel controls remain pinned while
many sessions are open. Active tabs are brought into view automatically.
Session metadata also refreshes tab titles after a session becomes ready,
replacing the temporary “New conversation” fallback when the real title
arrives.

Verification:
- `cargo test -p mew-desktop` — all 30 tests pass, including fallback-title
  refresh coverage.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — opened many session tabs in the fresh native
  bundle and confirmed the strip keeps the right-side controls visible.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-08-01 — Keep session identifiers stable across history loading

The daemon’s ready event uses the persisted `sess_<ulid>` identifier, while
history messages carry the underlying bare ULID. The client reducer now keeps
the ready identifier when attaching history, preventing one logical session
from appearing as both the original tab and a fallback “New conversation” tab.
Opening an existing session also removes any orphan placeholder tabs.

Verification:
- `cargo test -p mew-client-core -p mew-desktop` — 15 client-core and 31
  desktop tests pass, including identifier and placeholder regression cases.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — fresh native launch shows one session tab for the
  attached conversation.
- `cargo fmt --all` and `git diff --check` — clean.

# 2026-08-01 — Clarify native conversation state and shell affordances

The native transcript projection now hides persisted desktop capability notices
and exposes failed-turn state for short sessions. Thinking and tool parts are
collapsed by default, with human-facing tool categories, concise action
summaries, readable status labels, and formatted input/output on expansion.

The composer now has a native multi-file picker, a visible focus treatment,
larger model and persona targets, and accessibility metadata. The main shell,
conversation tabs, session rail, groups, auxiliary panels, settings controls,
and browser error state now expose native roles and labels. Browser startup
failures include a retry action, and active connection, workbench, terminal, and
auxiliary icons use semantic theme colors.

Verification:
- `cargo test -p mew-ui-model -p mew-desktop` — all 34 desktop and 7
  ui-model tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — verified AX labels, composer focus, attachment
  picker open/cancel, browser failure/retry, and tool-rich transcript layout.

# 2026-08-01 — Finish the native visual audit pass

The native shell now gives the tab strip subtle edge cues when conversations
continue offscreen, while keeping the connection and panel controls pinned.
The final visual pass also confirmed the settings navigation, collapsible
tool/thinking output, browser recovery state, terminal toggle, and multi-tab
session behavior in the packaged GPUI app.

Verification:
- `cargo test -p mew-ui-model -p mew-desktop` — all 34 desktop and 7
  ui-model tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — opened seven real conversation tabs, inspected
  the settings pages, retried the browser error, and showed/hidden the full
  terminal surface through the top-bar toggle.
- `cargo fmt --package mew-desktop --package mew-ui-model` and
  `git diff --check` — clean.

# 2026-08-01 — Refine native shell hierarchy and density

The second design pass gives assistant prose more breathing room, tones down
tool cards, and keeps the centered conversation composition intentional. The
session rail now shows real workspace paths as a second line when the daemon
provides them, without inventing repeated fallback labels for older sessions.
Browser startup errors now use concise recovery copy while preserving the
retry action, and settings has a wider, more stable two-column layout.

Verification:
- `cargo test -p mew-desktop -p mew-ui-model` — all 34 desktop and 7
  ui-model tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — inspected the fresh rail, settings layout,
  browser recovery state, short-session state, terminal, and workbench.
- `cargo fmt --package mew-desktop` and `git diff --check` — clean.

# 2026-08-01 — Harden review-critical native shell paths

Markdown virtualization now advances oversized chunk boundaries to valid UTF-8
character boundaries, so long Unicode transcript lines cannot panic during
rendering. Native cleanup is registered with GPUI's app-quit hook, stopping the
terminal client, browser portal, and app-owned daemon before GPUI clears its
windows; the existing entity drop path is now idempotent.

Composer focus no longer disables platform shell shortcuts. Failed last turns
now expose a retry action that resends the latest user prompt when the session
is ready.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host -p mew-desktop-supervisor` —
  all 38 desktop, 2 browser-host, and 5 supervisor tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- fresh native bundle screenshot — shell renders with the centered composer,
  sessions rail, and workbench; the computer-use AX tree could not reliably
  invoke the app menu's Quit action, so graceful quit remains source- and
  unit-tested but not visually confirmed in this harness.
- `cargo fmt --package mew-desktop -p mew-browser-host -p mew-desktop-supervisor`
  and `git diff --check` — clean.

# 2026-08-01 — Defer native browser initialization out of render

The browser portal no longer calls into CEF synchronously from
`DesktopShell::render`. Initialization is scheduled through GPUI's deferred
effect queue, guarded by a pending flag, and latched on failure until the user
presses retry. This avoids repeated CEF entry during frame traversal and keeps
the existing browser error card actionable.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host -p mew-cef-host` — all 39
  desktop, 2 browser-host, and CEF-host tests pass.
- `just desktop-build` and `git diff --check` — native release bundle packaged
  successfully and the diff is clean.
- fresh native bundle — shell rendered correctly; the computer-use coordinate
  click failed before reaching the Browser tab, so browser-open behavior still
  needs a direct manual interaction check.

# 2026-08-01 — Make session rows fill the navigation rail

Session rows now claim the full sidebar width before laying out their title,
truncation, hover action, and selection background. This prevents rows with
short titles from ending early and keeps the rail’s interaction surface
consistent.

Verification:
- `cargo test -p mew-desktop` — all 34 tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — fresh native launch shows full-width selected
  session rows with title truncation intact.
- `cargo fmt --package mew-desktop` and `git diff --check` — clean.

# 2026-08-01 — Move collapsed rails off-canvas

Collapsed sessions and workbench rails now animate their painted surfaces past
the window edge while their layout widths shrink to zero. Reopening either rail
slides it back from the same edge, and the workbench uses its current resized
width as the travel distance.

Verification:
- `cargo test -p mew-desktop -p mew-ui-model` — all 36 desktop and 7
  ui-model tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — collapsed and restored both rails in the fresh
  native bundle; no panel surface or border remained in either collapsed state.
- `cargo fmt --package mew-desktop` and `git diff --check` — clean.

# 2026-08-01 — Finish session interaction state and tool containment

Session management now has an explicit delete action for user-created groups,
drag-and-drop placement, and an accessible group picker with a clear
“remove from group” option. Group deletion is persisted and removes member
assignments through the daemon store.

Tool input and output blocks now have independent scroll viewports, keeping
long terminal results inside their cards instead of covering later transcript
content. Embedded browser focus now releases both CEF focus and the macOS
window’s first responder before the composer takes focus.

Conversation view state is scoped to the conversation currently shown in the
UI, including the sessions rail, workbench, terminal, auxiliary panel, width,
and expanded chat parts. This also fixes state leakage when switching between
already-open conversation tabs.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host -p mew-daemon -p mew-ui-model`
  — all focused Rust tests pass, including 36 desktop, 18 daemon, 19 daemon
  e2e, 7 ui-model, and browser-host tests.
- `just desktop-build` — native release bundle packaged successfully.
- computer-use smoke test — moved a real session into a group, opened the
  placement picker, removed the session, deleted the temporary group, and
  verified independent sidebar/workbench state across two open tabs.
- `cargo fmt --package mew-desktop -p mew-browser-host -p mew-daemon` and
  `git diff --check` — clean.

# 2026-08-01 — Make workbench sizing viewport-aware

The right workbench resize limit now follows the available shell width instead
of a fixed 720px cap. It leaves at least 420px for the centered conversation
area after accounting for the left sessions rail, while retaining the
workbench’s minimum usable width and adapting when the rail is collapsed.

Verification:
- `cargo test -p mew-desktop` — all 36 desktop tests pass, including the
  viewport-aware resize bounds.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --package mew-desktop` and `git diff --check` — clean.

# 2026-08-01 — Make conversation attention states explicit

Running turns, failed turns, and turns that ended without an assistant reply
now render as distinct inline attention cards. Running turns expose cancel,
failed turns expose retry and preserve the daemon error detail, and pending
permission/question actions suppress the generic working state so the user
sees one clear next step. Daemon and connection errors are now contained in a
dismissible card above the composer instead of appearing as an unstructured
red line.

Verification:
- `cargo test -p mew-desktop` — all 41 desktop tests pass, including the
  attention-state selection matrix.
- `just desktop-build` — native release bundle packaged successfully.
- `cargo fmt --package mew-desktop` and `git diff --check` — clean.

# 2026-08-01 — Harden CEF app-exit teardown and native commands

CEF teardown now distinguishes ordinary browser shutdown from GPUI app exit.
During app exit, the embedded browser and pump are stopped without calling
`CefShutdown` from inside GPUI’s `on_app_quit` borrow. The controller is
disarmed so entity drop cannot repeat the shutdown call. This avoids the
re-entrant `NSApplication terminate:` path identified in the review while
preserving normal CEF shutdown outside app exit.

The native menu bar now includes File and View menus for new/close conversation
and sidebar, terminal, and workbench toggles. Their actions are registered on
the shell root and the core shortcuts are bound globally, including while the
composer has focus.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host -p mew-cef-host` — all 41
  desktop, 2 browser-host, and 1 CEF-host tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- fresh rebuilt native bundle — rendered the populated transcript and
  centered composer without a new crash report; Computer Use could capture
  the window but could not complete a coordinate action because the target
  window became unavailable.
- `cargo fmt --package mew-desktop -p mew-browser-host -p mew-cef-host` and
  `git diff --check` — clean.

# 2026-08-01 — Remove stale workspace placeholders from the shell

The composer footer and Local workbench now show the selected session’s actual
working directory, rendered relative to the user’s home directory, and stop
inventing a checkout name or branch. Activity now reports the real session
state (`Needs input`, `Working`, or `No active tasks`) with matching empty
copy. A fresh browser panel starts at `about:blank` instead of a placeholder
external site.

When no named groups exist, the sessions rail places conversations directly
under its `SESSIONS` header. The `No group` placement action remains available
when named groups do exist.

Verification:
- `cargo test -p mew-desktop` — all 43 desktop tests pass, including the
  selected-path and group-section behavior tests.
- `just desktop-build` and `git diff --check` — native release bundle packaged
  successfully and the diff is clean.
- fresh rebuilt native bundle — visually confirmed the honest empty composer
  footer and File/View menus. The new bundle remained in its `starting` state
  with an empty session list during this run, so populated-session visual
  comparison remains pending; the unrelated `/Applications/mew.app` instance
  was left untouched.

# 2026-08-01 — Verify native connection timing before changing transport code

The rebuilt native client was traced through a fresh launch rather than
changing the WebSocket protocol speculatively. The app-owned daemon accepted
the client upgrade and answered a real protocol `ping`; the client reached
`connected`, emitted its initial event, and the GPUI relay received the
session/model updates. The earlier `starting` frame was a short-lived launch
observation, not a reproducible transport failure. Temporary diagnostic
prints were removed before the clean build.

Verification:
- `cargo test -p mew-desktop` — all 43 desktop tests pass.
- `just desktop-build` — native release bundle packaged successfully.
- `git diff --check` — clean.
- fresh native Computer Use capture after a longer startup window — connected
  state, populated sessions, model/persona controls, transcript, and changes
  workbench rendered successfully.

# 2026-08-01 — Harden process-exit browser teardown and real-session tabs

The app-quit path no longer asks embedded CEF to close a browser while GPUI is
inside its `will_terminate` borrow. It stops the browser pump and disarms the
CEF controller, leaving process-exit reclamation to the operating system. A
real session tab is also inserted directly instead of being created through a
transient `New conversation` placeholder, with regression coverage for both
fresh opens and active-placeholder reuse.

Verification:
- `cargo test -p mew-desktop` — all 45 desktop tests pass.
- `cargo test -p mew-browser-host` — all 2 browser-host tests pass.
- `git diff --check` — clean.

# 2026-08-01 — Correct variable-height sidebar rows and attachment identity

The virtualized sessions list no longer seeds every row with a fixed 52px
height even though toolbar, group, one-line, two-line, and picker rows have
different sizes. GPUI can now measure visible rows and converge their actual
scroll geometry. The session title row also uses GPUI's complete truncate
helper with space reserved for the hover move control, and the attachment
trigger no longer has a duplicate element id.

Verification:
- `cargo test -p mew-desktop` — all 45 desktop tests pass.
- `cargo test -p mew-browser-host` — all 2 browser-host tests pass.
- `cargo test -p mew-desktop-supervisor` — all 5 supervisor tests pass.
- `git diff --check` — clean.

# 2026-08-01 — Native bundle verification for the review pass

The clean packaged bundle was launched both through the native visual harness
and directly from its bundled executable. The direct bundle launch reached the
daemon-backed `connected` state, and the native capture showed the populated
sessions rail, compact top tab, right-aligned user message, collapsible
thinking/tool content, monospace highlighted code, centered composer, and
changes workbench. The Computer Use screenshot service could capture the app,
but its click and scroll actions returned `noWindowsAvailable`, so interaction
QA remains open for a harness-enabled run.

Verification:
- `just desktop-build` — clean release bundle packaged at
  `target/release/bundle/macos/mew.app`.
- `cargo test -p mew-desktop` — all 45 desktop tests pass.
- `git diff --check` — clean.

# 2026-08-01 — Complete native lifecycle and interaction review pass

The review fixes now have regression coverage for idempotent app-quit cleanup,
owned daemon reaping, browser focus handoff, duplicate-session-tab prevention,
variable-height sidebar measurement, and attachment identity. A fresh bundled
launch was exercised through the native accessibility surface: existing
sessions opened as real tabs without a placeholder, both sidebars moved fully
offscreen when collapsed, the terminal left no residual strip when hidden, the
composer accepted input after the browser panel was opened and closed, and the
embedded workbench remained available through its auxiliary tabs.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host -p mew-desktop-supervisor` —
  45 desktop, 2 browser-host, and 6 supervisor tests pass.
- `just desktop-build` — clean native release bundle packaged successfully.
- native Computer Use interaction pass — populated bundle, session tabs,
  collapsed surfaces, composer focus recovery, browser panel, and terminal
  visibility checked with screenshots.
- `git diff --check` — clean.

# 2026-08-01 — Anchor preference menus above their triggers

The model and persona selectors now render as trigger-local dropdowns above
their actual controls, with measured list heights that keep the composer
geometry stable. Their paint is deferred at priority 2, following GPUI's Zed
popover pattern, so the menus stay above transcript and workbench scroll
surfaces while retaining the same themed panel treatment.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host` — 45 desktop and 2
  browser-host tests pass.
- `cargo test -p mew-desktop-supervisor` — all 6 supervisor tests pass.
- `just desktop-build` — clean native release bundle packaged successfully.
- `git diff --check` — clean.
- native Computer Use — trigger-local placement was visually captured before
  the final deferred paint-order adjustment; the final clean launch remained
  in daemon-starting state, so repeated-toggle interaction is still a manual
  follow-up.

# 2026-08-01 — Restore metadata-only catalogs and composer-safe shortcuts

The desktop client now synchronizes metadata snapshots even when a protocol
message reduces to no display events. This covers `ModelList` and
`PersonaList`, so the composer receives the daemon's real catalog instead of
falling back to the synthetic `default` persona. The shell key dispatcher also
handles command-modified `b`, `j`, `n`, and `w` while the composer is focused,
alongside the existing command-number tab selection.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host -p mew-desktop-supervisor` —
  46 desktop, 2 browser-host, and 6 supervisor tests pass.
- `just desktop-build` — clean native release bundle packaged successfully.
- direct protocol probe against a restarted daemon — reported active `builder`
  and available `planner` personas.
- native Computer Use screenshot — packaged shell displayed the active
  `builder` persona after the metadata synchronization fix.
- `git diff --check` — clean.
# 2026-08-01 — Let model picker rows size to their content

Model picker options now use natural-height rows with a bounded five-row
viewport, matching the persona picker so long model IDs and descriptions no
longer collide. Scrolling is isolated to the options viewport so the anchored
popup does not reflow the desktop shell.

Verification:
- `cargo test -p mew-desktop` — 62 tests pass.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant` — clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- Computer Use confirmed long model rows remain readable, the model list scrolls, and the shell keeps its full width while the popup is open.
# 2026-08-01 — Remove horizontal borders from the workbench rail

The right workbench shell now keeps only its vertical divider edges. Removing
the outer top and bottom borders lets it read as a continuous contextual rail
instead of a separate card, while preserving its rounded clipping and inner
panel dividers.

Verification:
- `cargo test -p mew-desktop` — 62 tests pass.
- `cargo fmt --all -- --check` and `git diff --check` — clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- Computer Use confirmed the horizontal workbench strokes are gone in the live native frame.
# 2026-08-01 — Preserve view state while switching sessions

Session view snapshots now inherit the active layout on first visit instead of
falling back to startup defaults. Session selection restores the saved state
before attachment, while late `SessionReady` events only seed missing entries,
so they cannot undo a sidebar, workbench, terminal, auxiliary-panel, or
collapsible-content change made during the transition.

Verification:
- `cargo test -p mew-desktop` — 64 tests pass.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant` — clean.
- `cargo fmt --all -- --check` and `git diff --check` — clean.
- `just desktop-build` — optimized native bundle packaged successfully.
- Computer Use confirmed the native shell launches with its persisted layout and the session-switch path remains stable.

# 2026-08-01 — Restore GPUI focus after native browser interaction

The embedded CEF browser no longer claims focus during every visible layout
pass, which was racing the GPUI URL field and leaving the native child as the
first responder. Browser and URL-field interactions now explicitly release
CEF focus, while the macOS host restores the GPUI parent view as the window's
first responder. The shell also releases browser focus on GPUI mouse-down so
composer, picker, and other controls can recover input consistently.

Verification:
- `cargo test -p mew-desktop` — 64 tests pass.
- `cargo test -p mew-browser-host` — 2 tests pass.
- `cargo check -p mew-cef-host` and clippy for all three affected packages — clean.
- `cargo fmt --all -- --check` and `git diff --check` — clean.
- `just desktop-build` — optimized native bundle packaged successfully after clearing only the generated debug profile to recover disk space.
- Computer Use launched the rebuilt bundle and confirmed the native browser panel renders in the live shell.

# 2026-08-01 — Resolve CEF assets from native app bundles

Native CEF startup now prefers the framework inside the current app bundle or
the standard sibling bundle output before falling back to an exported CEF
distribution. This prevents Chromium from searching beside an unbundled
`target/<profile>/mew-desktop` for `libGLESv2.dylib`. The desktop development
recipe now packages and launches a debug `.app` so its helper processes use the
same bundle layout as release builds.

Verification:
- `cargo test -p mew-desktop -p mew-browser-host -p mew-cef-host` — 64, 2,
  and 1 tests pass.
- scoped clippy, `cargo fmt --all -- --check`, `git diff --check`, and
  `bash -n scripts/package-desktop-native.sh` — clean.
- `just desktop-build` — optimized native release bundle packaged successfully.
- Computer Use opened the browser in both the debug development bundle and
  the release bundle; no `libGLESv2.dylib` lookup error appeared, and helper
  processes resolved framework, resources, and locales from the app bundle.

# 2026-08-01 — Remove duplicate browser labels

The browser workbench no longer repeats its name in the launcher row and the
browser toolbar. The browser icon, URL controls, state affordance, and
accessibility labels remain intact, while the auxiliary rail stays compact and
icon-only.

Verification:
- `cargo test -p mew-desktop` — 64 tests pass.
- `cargo fmt --all -- --check` and `git diff --check` — clean.
- Computer Use opened the native browser panel and visually confirmed the
  duplicate labels are gone.

# 2026-08-02 — daemon/client parity correctness pass

The daemon and shared client state now handle the review findings around
session switching, metadata delivery, remote scopes, workspace watching, and
background persistence. Switching sessions detaches the old client
registration and resets session-scoped presence. Collaborate remotes cannot
open or drive terminals, while goal responses remain available to control
remotes. Reducers now retain title, summary, activity, stats, attention,
flagged-file, project, slash-result, and filesystem events. Group, archive, and
pin broadcasts preserve the other metadata fields, and configured workspace
roots appear in project listings.

Daemon client sends now propagate WebSocket errors to their callers. Auto
titles and summaries use the manager's session directory and persist their
results; summaries avoid duplicate in-flight provider work. Workspace watches
emit filesystem changes, and terminal output has a bounded intermediary
buffer so a slow frontend cannot grow PTY output without limit.

Verification:
- `cargo test --all` — passed.
- `cargo test -p mew-daemon --lib`, `cargo test -p mew-client-core --lib`,
  `cargo test -p mew-ui-model --lib`, and `cargo test -p mew --bin mew` — passed
  after final formatting.
- Targeted clippy for `mew-daemon`, `mew-client-core`, `mew-ui-model`, and the
  `mew` binary — clean.
- The workspace-wide clippy gate remains blocked by existing
  `clippy::large_enum_variant` errors in `crates/mew-message/src/lib.rs`.

# 2026-08-02 — native shell UX review pass

The native shell received a focused UX and accessibility pass. Composer input
now grows with wrapped content within a bounded range, while markdown code,
tool output, and diffs preserve whitespace and scroll horizontally. Picker
descriptions are width-safe, popovers receive keyboard focus and dismiss with
Escape, and focus returns to the composer. The top bar now uses the base
background token, and connection/profile menus use the panel surface instead
of an alert-colored surface.

Verification:
- `cargo test -p mew-desktop` — 80 tests pass.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant` — clean.
- `just arch-check`, `just theme-codegen-check`, and `git diff --check` — clean.
- Rebuilt the debug native bundle and used Computer Use to verify the model
  picker dismisses with Escape, the sidebar animates fully offscreen, and the
  terminal collapses to no visible surface before restoring correctly.

# 2026-08-02 — float required-action prompts above the composer

Pending permission, workspace-access, question, plan, and goal cards now live
in the deferred overlay layer instead of normal composer flow. They anchor to
the measured outer composer box, stay above the omnibox without shifting chat
layout, cap their own scroll height, and enter with a short opacity/translation
transition plus a panel shadow. This removes the visible composer jump and
keeps prompt rendering isolated from transcript layout.

Verification:
- `cargo test -p mew-desktop` — 81 tests pass, including the composer overlay
  anchor geometry regression test.
- `cargo clippy -p mew-desktop -- -D warnings -A clippy::large-enum-variant` — clean.
- `just arch-check`, `just theme-codegen-check`, and `git diff --check` — clean.
- Rebuilt and launched the native debug bundle; the normal composer and terminal
  layout remained stable after the overlay change.

# 2026-08-02 — bound desktop streaming and browser invalidation work

The native browser pump no longer starts before a portal exists, and successful
browser pumping runs from a small lifetime entity instead of invalidating the
whole shell every 30ms. Tool output now emits a dedicated incremental client
event and updates only the matching running tool call. Markdown row replacement
is range-scoped, text-layout measurement is cached by source identity, model and
persona menus use GPUI uniform lists, and transcript entry animation is limited
to the final visible row.

Verification:
- `cargo test --all` — passed across the workspace.
- `cargo test -p mew-ui-model -p mew-client-core` — 11 and 26 tests pass.
- `cargo test -p mew-desktop` — 82 tests pass.
- Scoped clippy with the existing `mew-message` large-enum lint allowed,
  `just arch-check`, `just theme-codegen-check`, and `git diff --check` pass.
- Rebuilt and launched the native debug bundle. Computer Use confirmed model
  and persona picker contents render, and browser-open CPU stayed around 3–5%
  versus the earlier ~83% runaway profile in the same unavailable-browser case.

# 2026-08-03: native shell correctness and virtualization pass

The desktop shell now gates workspace browsing and review data on the attached
session's real readiness and working directory, scopes directory listings by
session, and clears the previous transcript while a session attach is pending.
Closing tabs keeps the active tab stable, placeholder tabs are consolidated,
and per-session layout, auxiliary panel, browser, and expanded-part state is
persisted and restored.

Browser URL editing clamps stale UTF-8 ranges, native browser ownership clears
queued events on release/reclaim, and browser bounds are only sent when they
change. Streaming text/tool events are coalesced to a frame-sized interval,
cursor blinking stops on composer blur, and incremental markdown state avoids
replaying the entire stream parser for every delta. Activity, subagent, todo,
file-tree, model, persona, and transcript surfaces use bounded or virtualized
rows where applicable.

Verification:
- `cargo test --all`: passed.
- `cargo clippy -p mew-desktop -p mew-client-core -p mew-browser-host -p
  mew-config -p mew-ui-model -p mew-mobile-core --all-targets -- -D warnings
  -A clippy::large-enum-variant`: clean.
- `just arch-check`, `just theme-codegen-check`, `cargo fmt --all -- --check`,
  and `git diff --check`: clean.
- Rebuilt and launched the native debug bundle. The visual smoke pass showed
  real conversation tabs, centered bounded chat, and the workbench fully
  offscreen when collapsed. Browser interaction could not be completed because
  the packaged browser helper is unavailable in this environment.

# 2026-08-03: sort sidebar sessions by recent activity

Sidebar session rows now sort newest-first by the daemon's `last_message_at`
timestamp within each existing group, with sessions lacking a timestamp last.
Equal timestamps use the session id as a deterministic tie-breaker, while group
order and archived/ungrouped sections remain unchanged.

Verification:
- `cargo test -p mew-desktop --no-default-features`: 89 tests passed.
- `cargo clippy -p mew-desktop --all-targets -- -D warnings
  -A clippy::large-enum-variant`: clean.
- `cargo fmt --all` and `git diff --check`: clean.

# 2026-08-03: apply permission modes without waiting for active turns

Permission mode changes now update the agent's atomic permission engine and
broadcast immediately instead of waiting behind the session turn mutex. The
desktop client receives a targeted mode event, so changing to `permissive`
does not trigger a broad metadata refresh. Transcript selection now uses the
same measured `StyledText` layout that GPUI paints, avoiding the
`measurement has not been performed` panic during mouse or accessibility
events.

Verification:
- `cargo test -p mew-daemon --test e2e`: 22 tests passed, including active-turn
  permission-mode and connection-liveness regressions.
- `cargo test -p mew-client-core`: 27 tests passed.
- `cargo test -p mew-desktop --no-default-features`: 90 tests passed.
- Rebuilt and launched the native debug bundle. Computer Use selected
  `permissive`, confirmed the label changed, and exercised transcript
  interaction without the previous panic.
- `cargo build -p mew-desktop` and `git diff --check`: clean. The combined
  `mew` build remains blocked by pre-existing dirty `mew-catalog` errors about
  `thinking_budget`.

# 2026-08-03: render visible list bullets in TUI markdown

The ratatui-mdstream list renderer stripped item markers and styled the
bullet slot as two blank spaces, so unordered lists rendered without any
glyph and ordered lists lost their numbers. `render_list` now emits a
`•` for bullet items, keeps the source number for ordered items, and
preserves nested indentation; wrapped continuation lines align under the
item text for both marker shapes. Also dropped an unused `Theme` import
from the mdstream demo example.

Verification:
- `cargo test -p ratatui-mdstream`: 27 tests passed, including new
  bullet, numbering, wrap-alignment, and nested-list coverage.
- `cargo test -p mew-tui --test golden_test`: 5 frames unchanged.
- `cargo clippy -p ratatui-mdstream --all-targets -- -D warnings`: clean.
