# CURRENT.md — mew Fixes + Right Rail Redesign Progress

## 2026-07-06: Restored Accidentally Removed Features

A large batch commit (`7623b4d3` / `742f4bdc`) accidentally removed several features from the compiled surface while leaving dependent code (mew-mobile-core, iOS app, web client) still referencing them. All features have been restored from git history.

### Protocol (`mew-protocol`)
- `ClientMessage::Ping` and `ClientMessage::ListProjects` added back
- `ServerMessage::Pong { version }` and `ServerMessage::ProjectList { projects }` added back
- `ClientKind::Mobile` variant added back
- `ModelInfo.context_window: Option<i64>` field added back (serde default + skip-if-none)
- `ProjectInfo` struct added back (path, display_name, session_count, last_used_at)
- Roundtrip tests for all new variants + updated exhaustive tag tests (74 tests pass)

### Daemon (`mew-daemon`)
- `pub mod iroh_transport;` declaration added (file was already on disk, fully implemented)
- `check_socket_liveness()` restored (3 unit tests pass)
- `list_projects()` async helper restored — walks session_dir, reads meta.json, dedupes by canonical path, sorts by recency
- `ClientMessage::Ping` → `ServerMessage::Pong { version }` handler restored
- `ClientMessage::ListProjects` → `ServerMessage::ProjectList` handler restored
- `NewSession { cwd }` validation restored (must exist and be a directory)
- `handle_connection` made `pub` with `auto_summary_enabled: Arc<AtomicBool>` 5th param
- `DaemonServer.auto_summary_enabled` field added; idle_summary_task now spawned daemon-wide (not per-connection)
- 3 new e2e tests: ping/pong, list_projects, bad cwd validation (15 e2e tests pass)

### Iroh transport + `mew pair` (`mew` + `mew-daemon`)
- `iroh = "1"` and `qrcode = "0.14"` added to workspace deps
- `uniffi = "0.32"` added to workspace deps
- `mew-daemon` `iroh` feature now enables `dep:iroh`
- `mew` crate gets `iroh` feature (`dep:iroh` + `mew-daemon/iroh`) and `qrcode` dep
- `mew pair` subcommand restored — prints NodeId + ASCII QR code, enters pairing mode
- `--iroh` flag on `mew daemon` restored — runs daemon via iroh P2P transport
- `run_daemon_iroh()` and `pair_cmd()` functions restored
- `check_socket_liveness()` called before `daemonize()` (only when binding Unix socket)
- `build_daemon_server()` extracted as shared helper used by both `run_daemon` and `run_daemon_iroh`
- `mew-mobile-core` added to workspace members (was missing); `tool_sensitivity` field references fixed (set to `None` since the field was removed from `ToolCallPart`)

### Web client project picker (`mew-web-client` + `mew-web-ui`)
- `ProjectInfo` interface, `list_projects` ClientMessage, `project_list` ServerMessage, `project-list` event, `listProjects()` method added to web client
- `context_window` field added to `ModelInfo` interface
- Store: `projects`/`projectsLoading` state, `onProjectList` handler, bridge registration, `reset()` cleanup
- `ProjectPickerModal` component added to session-rail (recent projects list + free-text path input)
- 9 web UI unit tests pass

### Justfile
- `ios-core` recipe restored — builds both iOS targets, generates Swift bindings via uniffi-bindgen, creates XCFramework

### Verification
- `cargo clippy --all --features iroh -- -D warnings` — zero warnings
- `cargo test -p mew-protocol` — 74 tests pass
- `cargo test -p mew-daemon` (lib + e2e + tcp + concurrency) — all pass
- `pnpm --filter @mew/web-client build` — clean
- `pnpm --filter mew-web-ui test` — 9 unit tests pass (e2e requires running daemon)
- `mew pair --help` — shows subcommand

## 2026-07-02: Web UI Fixes Plan (6 Phases Complete)

### Phase 1: Crashes and alert correctness
- **1.1**: Moved `useState` above early return in `TodoRailPanel` (hooks crash fix)
- **1.2**: Added `SessionManager::broadcast_all()` — permission/input alerts now reach ALL sessions' clients via `forward_events` diverting `SessionAlert` messages
- **1.3**: Hoisted turn-end alert broadcast out of the usage block — failed turns before any `MessageEnd` now produce `TurnFailed` alerts
- **1.4**: `Notification.requestPermission()` called when permission is "default" (was never requested)
- **1.5**: Created `lib/router-ref.ts` with `navigateToSession()` — notification clicks use router-ref instead of broken `window.location.hash`
- **1.6**: Added `clearAlertsForSession(sessionId)` + `dismissAlert(sessionId, timestamp)` to store. `syncTitleBadge()` helper updates `document.title`. Called from `useSessionAttach` on session switch.
- **1.7**: Moved composer focus from ⌘K to ⌘L. Added ⌘N for new session in `__root.tsx`.
- **1.8**: Command palette theme toggle now uses `useTheme()` context + cycles through `THEMES` instead of writing `data-theme` directly.

### Phase 2: Flagged files lifecycle
- **2.1**: `reset()` now clears `flaggedFiles`, `dirListing`, `dirListingPath`, `filePreview`, `gitStatus`
- **2.2**: `flagged-files-changed` bridge handler guards with `data.session_id === sessionId`
- **2.3**: `AttachSession` handler replays current flagged-files set via `FlaggedFilesChanged`
- **2.4**: Shared `flag_mode_label()` helper in `mew-tools/flag_important.rs` — all three emit sites (agent, daemon unflag, attach replay) use the same function

### Phase 3: Session rail truthfulness
- **3.1**: `list()` checks `session.is_running` and reports `Running` state
- **3.2**: Added `SessionMetaChanged` broadcast for archive/pin/assign-group — all clients update their rail. Removed no-op `broadcast_groups` from archive.
- **3.3**: Alert titles use `session.display_title()` (custom title > summary > id) instead of raw ULID

### Phase 4: /wiki turn management refactor
- Extracted `run_turn()` helper encapsulating: cancel token, is_running, activity broadcasts, forward_events, meta/usage updates, alert broadcast
- Both Prompt handler and /wiki handler route through `run_turn()`
- Title generation stays Prompt-only (gated on `!had_error`)
- /wiki now reports failure in SlashResult text

### Phase 5: Tests
- Vitest store tests: alert lifecycle (push/clear/dismiss), flagged files (set/reset), session meta changes, attention changes, activity/usage updates (9 tests, all passing)

### Phase 6: "Needs you" ordering
- Added `pending_permissions`/`pending_questions` to `SessionInfo`
- `SessionAttentionChanged` broadcast on permission/ask-user create + resolve
- `statePriority()` tier 0 = needs attention (amber pulsing dot), excludes current session
- Session rail sorts needs-attention above running above active above idle


## 2026-07-05: Fix provider retry stuck at "retrying (1/4)"

The provider retry loop had three bugs causing it to get stuck or behave
incorrectly when the provider failed:

**Root causes & fixes:**

1. **No HTTP timeout** (`mew-provider-openai`, `mew-provider-anthropic`):
   Both providers used `reqwest::Client::new()` which has no timeout. If
   the server hung (accepted TCP but never responded), `execute(req).await`
   blocked forever — the RetryWait event was sent to the channel but the
   retry loop never advanced to the next attempt. Added
   `.timeout(300s).connect_timeout(30s)` to the client builder.

2. **Network errors not retried** (`mew-provider-openai`,
   `mew-provider-anthropic`): The `?` on `self.client.execute(req).await?`
   propagated connection-level errors (DNS failure, connection refused,
   TLS error, timeout) immediately, bypassing the retry loop entirely.
   Wrapped the execute call in a `match` — `Err(e)` now goes through
   `RetryPolicy::should_retry_network_error()` with the same exponential
   backoff as 429s.

3. **Stale retry status in TUI** (`mew-tui/src/app.rs`):
   `AgentEvent::Error` and `ProviderEvent::PartStart` did not clear
   `retry_status`, so the "retrying (1/4)" message persisted in the status
   bar even after the error was shown or content started flowing. Added
   `self.retry_status = None` to both handlers.

4. **Hardcoded `max_attempts: 4`** (`mew-provider-openai`,
   `mew-provider-anthropic`): The RetryWait event always reported
   `max_attempts: 4` regardless of the actual retry policy. For 5xx errors
   (which only allow 1 retry), this was misleading. Added
   `RetryPolicy::max_attempts_for(status_code)` which returns the correct
   max based on the status code.

**New `RetryPolicy` methods** (`mew-provider/src/lib.rs`):
- `should_retry_network_error(attempt)` — same exponential backoff as 429,
  retryable up to `max_retries`.
- `max_attempts_for(status_code)` — returns the correct max attempts for
  display: `max_retries` for 429/network errors, 1 for 5xx.

**Tests added** (`mew-provider/src/lib.rs`):
- `test_retry_network_error_backoff` — verifies exponential backoff and
  max_retries cutoff for network errors.
- `test_max_attempts_for` — verifies correct max_attempts for 429, 5xx
  with/without retry_5xx, and network errors.

## 2026-07-05: Docs update — sync with recent iOS + daemon commits

Audited the last ~40 commits (all iOS chat + daemon permission replay) against
the docs and fixed drift across five files:

**`docs/using-mew/ios-app.md`**:
- Chat view: replaced stale "basic formatting" / "collapsed reasoning" with
  SwiftStreamingMarkdown, full theme coverage, incremental streaming,
  de-jittered autoscroll, consecutive same-tool batching.
- Added file browser (+ button, folder navigation, file-to-composer), liquid
  glass chatbar, working indicator.
- Permission/ask sheets now noted as un-dismissable while pending.
- Connection lifecycle: added permission/ask replay on attach.
- Limitations: removed "No file browser" and "Basic markdown" (both shipped).
  Replaced with the one real remaining gap (no syntax highlighting in code
  blocks).

**`docs/development/dev-protocol.md`**:
- Client message table: was 12 entries, now covers all variants organized
  into sections (session lifecycle, turn interaction, model & mode, groups,
  projects & file service, other). Added DeleteSession, RenameSession,
  SetAutoTitle, SetAutoSummary, SetPermissionMode, YieldControl, all group
  ops, ListDir, ReadFilePreview, GitStatus, WatchWorkspace, OpenPath,
  UnflagFile, Ping.
- Server message table: similarly expanded. Added UserMessage, ErrorEvent,
  WorkspacePermissionRequest, SubagentPermissionRequest, ClientAttached/
  Detached, ControlYielded, PermissionModeChanged, GroupList, GroupsChanged,
  DirListing, FilePreview, GitStatusResult, FsChanged, SessionSummaryChanged,
  SessionMetaChanged, SessionAttentionChanged, SessionActivityChanged,
  SessionStatsChanged, SessionUsageChanged, SessionAlert,
  FlaggedFilesChanged, Pong.
- Session struct: updated to match current code (PendingRequest, ClientKind
  in clients tuple, title_generated, is_running, session_dir). Added
  explanation of PendingRequest and ClientKind.
- Connection lifecycle: added permission/ask replay + flagged-files replay
  on attach, drain_pending on last-client disconnect.
- AgentEvent translation: added WorkspacePermissionRequest and
  SubagentPermissionRequest.
- Added File service section with DirEntry struct.

**`docs/development/dev-mobile.md`**:
- Added File browser section (list_dir method, DirListing CoreEvent,
  DirEntry record, iOS + button integration).
- Added CI section documenting the ios-ci job (cargo check for both targets,
  UniFFI bindings drift guard).

**`docs/development/dev-architecture.md`**:
- Crate map: added mew-mobile-core and mew-ios.
- AgentEvent table: added WorkspacePermissionRequest, SubagentPermissionRequest.
- Channel-bearing variants note: expanded to include all four permission
  variants + PendingRequest explanation.
- State ownership table: updated pending requests row (PendingRequest),
  added iOS AppStore, is_running, ClientKind rows.
- Lifecycle: added permission/ask + flagged-files replay on attach,
  drain_pending on disconnect.
- Multi-client section: mentions ClientKind, iOS app, permission replay.

**`docs/development/dev-testing.md`**:
- Protocol test count: 63 → 70.
- Added CI jobs section documenting rust-ci, web-ci, and ios-ci jobs.

## 2026-07-05: Thread tool sensitivity through wire protocol (iOS + web)

Previously the iOS app used a client-side `toolSensitivity(for:)` map that
hard-coded tool names → sensitivity tiers. The web UI had a similar
`inferSensitivity(toolName)` heuristic. Both required manual updates when new
tools were added, and MCP tools were never handled correctly.

**Now**: `ToolCallPart` carries an `Option<String>` `sensitivity` field stamped
by the agent from the tool registry at `PartStart` time. The field flows
through `ProviderEventWire` → daemon → clients automatically. Both clients
read the wire field and fall back to `.dangerous` (never grouped) when absent.

### Rust changes
- `crates/mew-message/src/lib.rs`: Added `sensitivity: Option<String>` to
  `ToolCallPart` (serde `default` + `skip_serializing_if = "Option::is_none"`
  for backward compat with old sessions). Added `#[allow(clippy::large_enum_variant)]`
  on `Part`, `ProviderEventWire`, and `ProviderEvent`.
- `crates/mew-agent/src/events.rs`: `handle_provider_event` PartStart handler
  stamps `sensitivity` from `self.tools` registry using `sensitivity_label()`.
- `crates/mew-agent/src/agent.rs`: Made `sensitivity_label()` `pub(crate)`.
- `crates/mew-agent/src/tools.rs`: All 11 `PartUpdated` event constructions
  propagate `tc.sensitivity.clone()`.
- `crates/mew-provider-openai/src/lib.rs`, `crates/mew-provider-anthropic/src/lib.rs`,
  `crates/mew-provider-fake/src/lib.rs`: All `ToolCallPart` construction sites
  set `sensitivity: None` (stamped later by the agent).
- `crates/mew-mobile-core/src/state.rs`: Added `tool_sensitivity: Option<String>`
  to the UniFFI `MessagePart` record, populated from `tcp.sensitivity` in
  `apply_provider_event`.
- `crates/mew-mobile-core/src/lib.rs`: Session reload path carries
  `tool_sensitivity` through.
- UniFFI Swift bindings regenerated via `just ios-core`.

### iOS changes (`mew-ios/mew/MessageItemView.swift`)
- Replaced `toolSensitivity(for name: String?)` map with
  `toolSensitivity(for part: MessagePart)` that reads `part.toolSensitivity`.
- Grouping logic unchanged: readonly batched, modifiable batched, dangerous
  always separate. Mixed-tool summary "2× read · 1× grep".
- Spacing: 8→4pt between part groups, 6→4pt within tool rows (from the prior
  change, still applies).

### Web changes
- `mew-web-client/src/index.ts`: Added `sensitivity?: string` to the
  `tool_call` Part type.
- `mew-web-ui/src/stores/session.ts`: Added `sensitivity?: string` to the
  `MessagePart` tool-call variant. Threading through both
  `wirePartToMessagePart` and the streaming `part_start` handler.
- `mew-web-ui/src/components/tool-call-card.tsx`: Replaced
  `inferSensitivity(toolName)` with `partSensitivity(part)` reading the wire field.
- `mew-web-ui/src/components/message-item.tsx`: `groupParts` now groups by
  sensitivity tier (readonly batched, modifiable batched, dangerous always
  individual). Updated `ToolCallGroup` summary to show per-tool counts
  "2× read · 1× grep".

### Build status
- Rust: `cargo clippy --all -- -D warnings` → zero warnings.
- iOS: `xcodebuild -scheme mew -destination 'iPhone 17'` → BUILD SUCCEEDED.
- Web client: `pnpm build` → clean.
- Web UI: `pnpm build` → clean, 9 vitest tests pass (1 e2e test file fails,
  expected — needs running daemon).

## 2026-07-04: iOS tool call grouping by sensitivity tier + spacing fix

Changed how tool calls are grouped in the iOS chat UI (`mew-ios/mew/MessageItemView.swift`):

**Before:** consecutive calls to the *same* tool name were collapsed (`Read ×4`). Different tools in a row were separate rows with 8pt gaps.

**After:** consecutive tool calls are grouped by sensitivity tier:
- **ReadOnly** tools (read, grep, glob, echo, etc.) batch together → one row showing a mixed summary like `2× read · 1× grep`
- **Modifiable** tools (write, edit_hashline, edit_str_replace, etc.) batch together
- **Dangerous** tools (bash, shell_background, shell_monitor, unknown/MCP tools) are always shown individually

The grouping uses a client-side `toolSensitivity(for:)` map derived from the Rust `Tool::sensitivity()` impls. Unknown tools default to `.dangerous` so MCP tools are never silently grouped.

**Spacing:** reduced vertical gaps between part groups (8→4pt) and within tool rows (6→4pt) to eliminate the visible gap between consecutive tool calls.

Build: `xcodebuild -scheme mew -destination 'iPhone 17'` → BUILD SUCCEEDED.

## 2026-07-04: Iroh/WebSocket transport parity fix

The iroh p2p transport (`run_iroh`) was missing two things the Unix-socket and TCP transports had:

1. **`idle_summary_task` never spawned** — auto-summary of idle sessions silently didn't run over iroh. Added the spawn in `run_iroh`, matching `run()` and `run_tcp()`.
2. **`auto_summary_enabled` was disconnected** — `run_iroh` created a fresh `Arc<AtomicBool>` inside the handler instead of using the server's shared flag. `SetAutoSummary` client messages updated the server's flag but the iroh handler read a different one. Now `run_iroh` accepts the server's `auto_summary_enabled` as a parameter.
3. **iroh test didn't compile** — `auto_summary_enabled` field was added to `MewIrohHandler` but the test struct literals were never updated.

Changes:
- `crates/mew-daemon/src/iroh_transport.rs`: `run_iroh` gains `auto_summary_enabled` param, spawns `idle_summary_task`, passes the shared flag to the handler. Removed stale TODO comment.
- `crates/mew/src/main.rs`: `run_daemon_iroh` passes `server.auto_summary_enabled`.
- `crates/mew-daemon/tests/iroh.rs`: Both `MewIrohHandler` constructions updated with the field.

All daemon tests pass (e2e: 12, tcp: 5, concurrency: 6, iroh: 2). Clippy clean with `-D warnings`.

## 2026-07-03: Daemon multi-workspace plan (Complete)

Implemented the full plan from `notes/mew-daemon-multi-workspace-plan.md`:

### Commit 1: Socket liveness guard
- `check_socket_liveness()` in `mew-daemon/src/lib.rs` — before binding, tries connecting to the existing socket. If connect succeeds, bails "daemon already running". If connection refused, stale — remove and bind.
- Called before `daemonize()` in `main.rs` so the error reaches the terminal.
- 3 unit tests (live socket rejected, stale socket removed, missing socket ok).

### Commit 2: `Agent.cwd` field
- Added `pub cwd: PathBuf` to `Agent`, defaulted to `current_dir()` in `Agent::new`.
- Replaced all 7 runtime `current_dir()` call sites in mew-agent:
  - `tools.rs:542` — `ToolCtx.cwd` (file tool path resolution)
  - `tools.rs:276` — permission engine cwd per tool call
  - `tools.rs:1394/1496` — `shell_background` / `shell_monitor` fallback cwd
  - `agent.rs:726` — `TemplateContext.cwd` (system prompt template var)
  - `agent.rs:949` — `plan_path` resolution
  - `agent.rs:498` — cwd sent to Auto/Auto+ classifier
  - `runner.rs:184` — subagent template ctx
- `SimpleRunner::with_cwd(cwd, workspace_roots)` — child agents inherit parent cwd + workspace roots.
- All 95 existing mew-agent tests pass.

### Commit 3: Thread cwd through daemon build path
- `build_session_agent` takes `session_cwd: &Path` parameter.
- All loaders (skills, personas, subagents, context files), permission engine, shell session, workspace_roots, and `agent.cwd` use the session cwd.
- Daemon builder closure passes `params.cwd.unwrap_or(current_dir())`.
- Fixed resume bug: `session.rs` was passing `cwd: None` on attach even though `meta.cwd` was available. Now passes `meta.cwd`.
- Subagent runner wired with `.with_cwd(agent.cwd.clone(), agent.workspace_roots.clone())` at all 3 call sites.

### Commit 4: TUI daemon client sends cwd
- `client.rs` `new_session()` now sends `current_dir()` instead of `None`.

### Commit 5: Docs
- CLAUDE.md updated with "Multi-workspace daemon sessions" section.
- This CURRENT.md entry.

### Build status
- Zero clippy warnings across mew-agent, mew-daemon, mew.
- 95 mew-agent tests pass, 3 mew-daemon tests pass.

## 2026-07-03: mew iOS app M3 — SwiftUI app (Complete)

The full iOS app builds and links on the simulator. Files at `mew-ios/mew/`:

### App scaffold
- **`MewApp.swift`** — `@main` entry, creates `AppStore` as `@StateObject`
- **`AppStore.swift`** — `ObservableObject` mirroring the web UI's session store: daemons, session lists, messages, streaming text, pending permissions/ask-user, models, alerts. `CoreListenerBridge` receives UniFFI callbacks on a background thread and dispatches to `@MainActor`
- **`KeychainHelper.swift`** — loads/creates the phone's persistent iroh secret key in the iOS keychain (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` — no iCloud sync)
- **`RootView.swift`** — `NavigationStack` with `Daemons → Sessions → Chat` route
- **`Theme.swift`** — color palette
- **`FormatHelper.swift`** — cost, relative age, session state labels, JSON pretty-printing

### Views (mirrors web UI patterns)
- **`DaemonListView.swift`** — daemon list with status dots, needs-you badges, add daemon sheet (paste NodeId / `mew001:` payload, validates via `parseDialInfo`), context menu remove
- **`SessionRailView.swift`** — per-daemon session list with needs-attention ordering (pending → running → active → idle), state dots, cost badges, swipe actions (pin/archive/delete with confirmation), new session button, archived toggle, disconnected state
- **`ChatView.swift`** — message list with auto-scroll, streaming text bubble, composer (send/cancel), model picker menu, permission sheet + ask-user sheet driven from store pending queues
- **`MessageItemView.swift`** — renders message parts: text (AttributedString markdown), reasoning (collapsible), tool calls (compact row with state indicators, expandable input), errors
- **`SettingsView.swift`** — phone NodeId (copyable), daemon management (rename/remove), app version

### Project setup
- **`project.yml`** — xcodegen config, iOS 17+, depends on `MewMobileCore` SwiftPM package + `SystemConfiguration`, `Security`, `Network` frameworks
- **`mew.xcodeproj`** — generated via `xcodegen generate`
- Build: `xcodebuild -project mew.xcodeproj -scheme mew -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build` → **BUILD SUCCEEDED**

## 2026-07-03: mew-mobile-core M2 — UniFFI bindings + SwiftPM package (Complete)

### M2: bindings + package

- **`uniffi.toml`** in `crates/mew-mobile-core/` — configures `cdylib_name = "mew_mobile_core_ffi"`
- **`uniffi-bindgen` binary** — added `src/bin/uniffi-bindgen.rs` + `cli` feature on `uniffi` dep, so `cargo run -p mew-mobile-core --bin uniffi-bindgen` generates Swift bindings
- **`just ios-core` recipe** — builds both iOS targets (aarch64-apple-ios + aarch64-apple-ios-sim), generates Swift bindings via `uniffi-bindgen`, creates XCFramework from the two `.a` files with a temp headers dir (only FFI headers, not the generated .swift)
- **SwiftPM package** at `mew-ios/MewMobileCore/`:
  - `Package.swift` — binary target (`mew_mobile_coreFFI`, the xcframework) + source target (`MewMobileCore`, the generated Swift bindings)
  - `Sources/MewMobileCore/mew_mobile_core.swift` — UniFFI-generated Swift bindings (all types: `MobileCore`, `CoreEvent`, `CoreListener`, `DaemonId`, `DaemonEntry`, `DaemonSnapshot`, `Decision`, etc.)
  - `Sources/MewMobileCore/mew_mobile_coreFFI.h` + `mew_mobile_coreFFI.modulemap` — C FFI header + module map
  - `XCFramework/mew_mobile_core.xcframework` — universal binary (ios-arm64 + ios-arm64-simulator), gitignored as a build artifact
  - `.gitignore` — excludes XCFramework + SwiftPM build artifacts
- **Key fix**: xcframework headers need `module.modulemap` (not `<name>.modulemap`) for SwiftPM binary targets to import correctly. The justfile recipe handles this with a temp dir.
- **Build verification**: `xcodebuild -scheme MewMobileCore -destination 'generic/platform=iOS Simulator' build` → BUILD SUCCEEDED, zero errors, zero warnings
- **Note**: `swift build` (host) fails because the xcframework only has iOS slices — that's expected. Use `xcodebuild` for iOS verification.

## 2026-07-03: mew-mobile-core M1 — actually complete (bug fixes)

M1 was "compiles and doesn't crash" but had three critical bugs that meant the
core didn't work end-to-end. All fixed:

### Critical fixes
1. **Listener forwarding was completely broken.** `connect()` spawned a forwarder
   task that just logged events instead of calling `listener.on_event()`. Root
   cause: listener was `Box<dyn CoreListener>` which can't be cloned into the
   spawned task. Fix: changed to `Arc<dyn CoreListener>`, switched from
   `callback_interface` to `with_foreign` (UniFFI's Arc-compatible foreign trait),
   clone the Arc into the spawned task, call `on_event` directly. Removed the
   dead `event_tx`/`event_rx` channel and the dead `emit()` method.
2. **`snapshot()` always returned empty state.** `connect_and_run` used a local
   `SessionState` while `snapshot()` read from `DaemonConnection.session_state`
   — two different objects. Fix: introduced `ConnState` (Arc-shared between
   `DaemonConnection` and the background task). `translate_message` now writes
   to `ConnState`, `snapshot()` reads from it. Also stores `daemon_version`,
   `models`, and `session_title` in shared state.
3. **Reconnect didn't re-send `AttachSession`.** After a reconnect cycle,
   `connect_and_run` only sent Ping. Fix: read `conn_state.attached_session`
   at the top of `connect_and_run` and re-send `AttachSession` if set.

### Missing API methods added
- `respond_ask_user`, `list_models`, `switch_model`, `set_permission_mode`,
  `rename_session`, `archive_session`, `pin_session`, `delete_session`
- All are one-liner `conn.tx.send(ClientMessage::...)` calls

### Additional improvements
- `SessionTitleChanged` now stores title in `ConnState` (was TODO)
- `TodosUpdated` now emits `CoreEvent::TodosUpdated` (was in no-op catch-all)
- `ModelList` stores models in `ConnState` for snapshot
- `CoreError` now impls `std::error::Error` (needed for `?` with anyhow)
- Unused `HashMap` import removed from `state.rs`
- `rx` moved via `Option::take()` to survive reconnect loops
- M1 integration test rewritten: uses `Arc<TestListener>` so the test retains
  access to collected events, verifies Connected → Version → Reloaded →
  TurnEnded → TextDelta → snapshot has messages. Uses `multi_thread` tokio
  flavor (required for `tokio::spawn` in `connect()`)

### Build status
- Zero clippy warnings (lib + tests, all features)
- 14 unit tests pass
- M0 spike test passes (~38s)
- M1 integration test passes (~43s) — events verified through listener

## 2026-07-02: iOS app spec written

- New spec: `notes/mew-ios-app-spec.md` — iOS client for multi-daemon access over iroh
- Builds on `notes/mew-mobile-iroh-plan.md` (POC ladder rungs 3–4); daemon iroh Stage 1 (below) is treated as an unmerged prerequisite, nothing mobile-side exists yet
- Key decisions: `mew-mobile-core` Rust crate owns all protocol knowledge (UniFFI → Swift); one iroh connection per daemon mirroring the web client's one-WS model; WS client handshake over QUIC (matches Stage 1's double framing); phone keeps one iroh keypair in the iOS keychain (becomes its device key under the accounts plan); lenient frame decode so daemon protocol additions don't brick older phones; foreground-only v1, push relay deferred
- Only protocol change needed: `ClientKind::Mobile` (additive; cheapest to land while Stage 1 is unmerged)
- Milestones m0–m4, m0 is a cross-compile + WS-over-QUIC spike to de-risk iroh on iOS first

## 2026-07-02: iroh Stage 1 — daemon listener + pairing (Complete, minus integration test)

### Architecture
- iroh is an **optional Cargo feature** (`--features iroh`) on both `mew-daemon` and `mew` crates, off by default
- iroh v1.0.1 from crates.io (API differs from the plan's v0.95 reference: `Endpoint::builder(N0)`, `endpoint.id()`, `connection.remote_id()` returns `EndpointId` directly, `ProtocolHandler::accept` uses `async fn` returning `Result<(), AcceptError>`)
- `IrohStream` wraps `(SendStream, RecvStream)` as `AsyncRead + AsyncWrite` → passed to existing `handle_connection` (WebSocket upgrade over QUIC — double-framed, intentional for Stage 1)

### Files created/modified
- **`crates/mew-daemon/src/iroh_transport.rs`** (new): `IrohStream`, `NodeIdAllowlist`, `MewIrohHandler` (ProtocolHandler), `run_iroh()`, `default_allowlist_path()`, 4 unit tests
- **`crates/mew-daemon/src/lib.rs`**: `#[cfg(feature = "iroh")] pub mod iroh_transport` + re-exports
- **`crates/mew-daemon/Cargo.toml`**: `iroh` optional dep + `[features] iroh`
- **`crates/mew/Cargo.toml`**: `iroh` optional dep + `[features] iroh = ["dep:iroh", "mew-daemon/iroh"]`
- **`crates/mew/src/main.rs`**: `--iroh` flag on `mew daemon`, `Pair` subcommand, `run_daemon_iroh()`, `pair_cmd()`
- **`Cargo.toml`**: `iroh = "1"` workspace dep

### Review fixes applied
After a plan-reviewer subagent review, fixed:
1. **`endpoint.online()` timeout** — wrapped in 15s timeout to prevent hanging if relays unreachable
2. **Dead code removed** — `enable_pairing_mode()` + `pairing_mode` field on `MewIrohHandler` were unused since `pair_cmd` handles pairing independently
3. **Corrupted allowlist** — now logs a warning instead of silently returning empty Vec
4. **Pairing timeout** — `mew pair` now times out after 120s
5. **Pairing error handling** — `allowlist.add()` failure now properly closes connection with error code instead of `?` skipping cleanup
6. **SIGTERM in pair_cmd** — added handler alongside Ctrl+C
7. **Unused dep** — removed `data-encoding` (was never used)
8. **Test assertion** — `test_iroh_stream_implements_async_traits` now actually calls `_assert_async_read::<IrohStream>()`

### Known remaining (reviewer flagged, deferred)
- **Code duplication**: `run_daemon_iroh` duplicates ~150 lines of `run_daemon`'s server-building code. Extracting a shared `build_daemon_server()` is the right fix but a larger refactor.
- **QR code**: Plan spec mentions ASCII QR code in `mew pair` output. Deferred — NodeId is printed as text for now.
- **Relay URL**: Not printed in pairing output. iroh's N0 preset handles discovery, so NodeId alone should work, but the plan calls for explicit relay URL.

### Integration tests
- `tests/iroh.rs`: Two tests using real iroh endpoints (N0 preset, relay-based):
  1. `iroh_peer_connects_and_exchanges_protocol` — connects, WS upgrade, NewSession + Prompt, verifies streaming PartStart + MessageEnd
  2. `iroh_unauthorized_peer_is_rejected` — unauthorized peer's connection is closed by daemon
- Both pass (66s — relay connection setup takes time)

### Build status
- Default build: zero clippy warnings, all tests pass
- iroh feature build: zero clippy warnings, 4 iroh unit tests + 23 existing tests pass
- mew-mobile-core: zero clippy warnings, 8 unit tests + 1 M0 spike test pass
- iOS cross-compile: `aarch64-apple-ios` + `aarch64-apple-ios-sim` both compile cleanly

### M0: iOS cross-compile spike (Complete)
- **Cross-compilation de-risked**: iroh 1.0.1 + tokio-tungstenite compile for both `aarch64-apple-ios` (device) and `aarch64-apple-ios-sim` (simulator) with zero errors. iroh's ring/rustls crypto backend works on iOS.
- **M0 spike test** (`tests/m0_spike.rs`): Full round-trip — connect over iroh → WS upgrade over QUIC → Ping/Pong → NewSession(Mobile) → Prompt → streaming PartStart + MessageEnd. Passes in ~43s (relay setup time).
- **Reviewer fixes applied**: 
  - Dead `next_id`/`AtomicU64` removed from `DaemonRegistry` (DaemonId is the node_id string)
  - `simple_uuid()` uses `ulid::Ulid::new()` instead of nanosecond timestamp (matches mew-message pattern)
  - Secret key corruption now returns a clear error with recovery instructions (not silent regeneration)
- **Known M1 items** (reviewer flagged, deferred to M1):
  - Wire `connect_and_run` to emit `CoreEvent`s and update `SessionState` (currently just logs)
  - Handle `PartUpdated` in state assembly (spec note #10 — authoritative replacement)
  - Reconnect with exponential backoff + jitter
  - TextDelta coalescing (spec note #8)
  - UserMessage dedup (spec note #9)
  - `parse_dial_info()` function for pairing payload parsing (spec notes #2, #11)

### M1: Event pipeline + state assembly + reconnect (Complete)
All M1 items from the reviewer's deferred list are now implemented:
- **Full event pipeline wired**: `connect_and_run()` translates `ServerMessage` → `SessionState` → `CoreEvent`, emits via `mpsc` channel → `CoreListener`. All 15 event variants covered.
- **PartUpdated authoritative** (spec note #10): replaces accumulated delta state wholesale for tool calls, text, reasoning.
- **TextDelta coalescing** (spec note #8): batches deltas on ~16ms tick before FFI, prevents per-token ObjC bridge round-trips.
- **UserMessage dedup** (spec note #9): drops the first `UserMessage` matching the last sent prompt text.
- **Reconnect with backoff**: exponential (1s, 2s, 4s… cap 30s) + jitter. Emits `DaemonStatusChanged` (Connecting/Connected/Backoff/Disconnected). Clean disconnect stops retry.
- **`parse_dial_info()`**: handles raw NodeId, `mew001:<node_id>`, `mew001:{json}`. Rejects unknown versions. 6 tests.
- **MobileCore async API**: `new()` is async (binds iroh endpoint). Full send methods: `connect`, `disconnect`, `attach`, `prompt`, `cancel`, `respond_permission`, `list_sessions`, `new_session`, `snapshot`.
- **M1 integration test**: `MobileCore::connect()` + `new_session()` + `prompt()` over real iroh, passes in ~44s.
- **16 total tests** (14 unit + 1 M0 spike + 1 M1 integration), zero clippy warnings.

### Daemon-side fixes for mobile (from iOS spec notes)
- **Persistent secret key** (Note #1 fix): `load_or_create_secret_key()` persists the iroh SecretKey to `iroh_secret_key.json` with 0600 permissions. Both `run_iroh` and `pair_cmd` load the same key → NodeId is now stable across restarts, and `mew pair` shows the daemon's real NodeId.
- **`ClientKind::Mobile`** (spec protocol change): added to mew-protocol + re-exported in TS client. One-line additive change.

### mew-mobile-core crate (M1 milestone)
New workspace crate at `crates/mew-mobile-core/`. Implements the iOS spec's core layer:

- **`codec.rs`** — Lenient decoder: tries typed `ServerMessage`, falls back to `serde_json::Value` + type tag logging, drops unknown frames. 4 tests (known decode, unknown drop, bad JSON error, malformed known drop).
- **`events.rs`** — `CoreEvent` enum (app-vocabulary, not wire-vocabulary), `CoreListener` trait, `DaemonStatus`, `SessionSummary`, `ModelSummary`. 15 event variants covering daemon status, session list, text deltas, part updates, turn ended, permission/ask requests, alerts, attention, todos, model list, slash results, version.
- **`registry.rs`** — On-device `DaemonRegistry` JSON store: add/remove/get/touch, atomic writes, persistence. 2 tests (CRUD + persistence).
- **`state.rs`** — `SessionState` part-assembly: `PartStart`/`PartDelta`/`PartEnd`/`MessageEnd` → messages with streaming text accumulation, cost tracking. `PartUpdated` is authoritative. `DaemonSnapshot` for full state mirror. 2 tests (text streaming + cost accumulation).
- **`lib.rs`** — `MobileCore` struct: owns iroh endpoint with phone's persistent secret key, daemon registry, connections map, listener. `connect()` spawns background task that connects over iroh, does WS upgrade, runs message loop with lenient decode. `IrohStreamWrapper` for AsyncRead+AsyncWrite over QUIC streams.

### Code dedup
- Extracted `build_daemon_server()` shared by `run_daemon` and `run_daemon_iroh`. Eliminated ~150 lines of duplication.

### QR code in `mew pair`
- Added `qrcode` crate. `mew pair` now prints an ASCII QR code containing `mew001:<node_id>` (versioned prefix per accounts plan).

### Phase 0.3: Ping/Pong version handshake
- Added `ClientMessage::Ping` and `ServerMessage::Pong { version }` to mew-protocol
- Daemon responds with `env!("CARGO_PKG_VERSION")` on Ping
- Added `ping()` method to `MewClient` (returns `Promise<string>` with daemon version)
- Added `pong` event type to web client
- Added `Pong` to `DaemonClient`'s `translate_server_message` match (no-op for TUI)
- Protocol round-trip test verifies serialization/deserialization
- Note: Phase 0.1 (message-shaped permissions) was already done — the daemon already translates `oneshot::Sender` to ID-paired wire messages (`PermissionRequest { request_id }` / `PermissionResponse { request_id, decision }`). The `oneshot` is internal to the daemon process, never crosses the wire. Remote clients over iroh already use the ID-paired protocol.

## 2026-07-02: Right Rail Redesign (Complete)

### P0.1a: Context window on ModelInfo (Rust + TS)
- Added `context_window: Option<i64>` to `mew-protocol::ModelInfo` (serde skip-if-none)
- Populated from catalog's `m.context_window` in `main.rs` model lister
- Added `context_window?: number` to `mew-web-client/src/index.ts` `ModelInfo`
- Fixed 2 protocol test constructions to include the new field
- Rust + TS both compile cleanly

### P0.1b: Context gauge
- Added `lastInputTokens` to store (from `message_end` `usage.input` — approximates current context fill)
- Reset in `reset()` and `onSessionCleared()`
- `ContextGauge` component in right rail: progress bar (green/yellow/red), `formatTokens` labels, warning at >80%

### P0.2: Alert banner
- `AlertBanner` at top of right rail showing most recent unread alert
- Color-coded by kind (yellow for permission/input, red for failed, green for turn-complete)
- Click navigates to alert's session via `routerRef` + clears alerts for that session
- Dismiss button per-alert; shows "+N more" when multiple alerts
- Only shows alerts not for the current session (suppresses self-alerts)

### Dock: Right rail as docked panel
- Desktop: `<aside>` docked on the right with sidebar styling (border-l, bg-sidebar, text-sidebar-foreground)
- Mobile: Sheet slide-over (unchanged behavior, toggled from FakeHeader Activity button)
- `FakeHeader` only shows Activity button on mobile (desktop rail is always visible)
- Session route wraps content in flex row: `[chat column] [right rail]`

### P1: Activity timeline tab
- New "Activity" tab in right rail
- Flattens recent messages + subagents into a timeline (text/tool-call/subagent/error entries)
- Sorted newest-first, capped at 50 entries
- Icons match state (spinning for running, green for done, red for error)

### P2: Changes panel tab
- New "Changes" tab in right rail
- Shows per-session `ChangeStats` (added/removed/files) from `availableSessions`
- Lists changed files with short name + full path

### Pre-existing fixes (from revert)
- Fixed stray `s` typo in `virtual-chat-surface.tsx` line 44
- Removed `streamingText`/`streamingReasoningText` props from `MessageItem` usage (it reads from store directly)
- Cleaned up unused store selectors in `virtual-chat-surface.tsx`

### Build status
- Rust: zero clippy warnings, all 67 protocol tests pass
- Web client: builds cleanly
- Web UI: builds cleanly, 9 vitest tests pass

## 2026-07-02: Web UI Fixes Plan (6 Phases Complete)

### Phase 1: Crashes and alert correctness
- **1.1**: Moved `useState` above early return in `TodoRailPanel` (hooks crash fix)
- **1.2**: Added `SessionManager::broadcast_all()` — permission/input alerts now reach ALL sessions' clients via `forward_events` diverting `SessionAlert` messages
- **1.3**: Hoisted turn-end alert broadcast out of the usage block — failed turns before any `MessageEnd` now produce `TurnFailed` alerts
- **1.4**: `Notification.requestPermission()` called when permission is "default" (was never requested)
- **1.5**: Created `lib/router-ref.ts` with `navigateToSession()` — notification clicks use router-ref instead of broken `window.location.hash`
- **1.6**: Added `clearAlertsForSession(sessionId)` + `dismissAlert(sessionId, timestamp)` to store. `syncTitleBadge()` helper updates `document.title`. Called from `useSessionAttach` on session switch.
- **1.7**: Moved composer focus from ⌘K to ⌘L. Added ⌘N for new session in `__root.tsx`.
- **1.8**: Command palette theme toggle now uses `useTheme()` context + cycles through `THEMES` instead of writing `data-theme` directly.

### Phase 2: Flagged files lifecycle
- **2.1**: `reset()` now clears `flaggedFiles`, `dirListing`, `dirListingPath`, `filePreview`, `gitStatus`
- **2.2**: `flagged-files-changed` bridge handler guards with `data.session_id === sessionId`
- **2.3**: `AttachSession` handler replays current flagged-files set via `FlaggedFilesChanged`
- **2.4**: Shared `flag_mode_label()` helper in `mew-tools/flag_important.rs` — all three emit sites (agent, daemon unflag, attach replay) use the same function

### Phase 3: Session rail truthfulness
- **3.1**: `list()` checks `session.is_running` and reports `Running` state
- **3.2**: Added `SessionMetaChanged` broadcast for archive/pin/assign-group — all clients update their rail. Removed no-op `broadcast_groups` from archive.
- **3.3**: Alert titles use `session.display_title()` (custom title > summary > id) instead of raw ULID

### Phase 4: /wiki turn management refactor
- Extracted `run_turn()` helper encapsulating: cancel token, is_running, activity broadcasts, forward_events, meta/usage updates, alert broadcast
- Both Prompt handler and /wiki handler route through `run_turn()`
- Title generation stays Prompt-only (gated on `!had_error`)
- /wiki now reports failure in SlashResult text

### Phase 5: Tests
- Vitest store tests: alert lifecycle (push/clear/dismiss), flagged files (set/reset), session meta changes, attention changes, activity/usage updates (9 tests, all passing)

### Phase 6: "Needs you" ordering
- Added `pending_permissions`/`pending_questions` to `SessionInfo`
- `SessionAttentionChanged` broadcast on permission/ask-user create + resolve
- `statePriority()` tier 0 = needs attention (amber pulsing dot), excludes current session
- Session rail sorts needs-attention above running above active above idle

## 2026-07-03: multi-workspace daemon — audit + plans

Audited what blocks one daemon serving sessions across multiple project directories, wrote two plans:

- `notes/mew-daemon-multi-workspace-plan.md` — daemon/agent side. Key findings: `AgentBuildParams.cwd` is ignored by `build_session_agent` (everything keyed on process cwd); seven `std::env::current_dir()` call sites inside mew-agent core (ToolCtx, permission engine, template ctx, plan path, classifier, shell fallbacks, subagent runner); resume drops `meta.cwd` (`session.rs:279`); second daemon steals the socket (`lib.rs:174`); `mew chat --connect` sends `cwd: None` (`client.rs:142`). Plan: `Agent.cwd` field (defaulted, TUI unchanged), thread cwd through builder + daemon + attach, socket liveness guard, 5 commits on `daemon-multi-workspace`.
- `notes/mew-clients-multi-workspace-plan.md` — web + iOS side, depends on the daemon plan. Web rail already groups by `SessionInfo.cwd`; mobile core drops cwd in its `SessionSummary` mapping. Plan: `ListProjects`/`ProjectList` protocol messages (recent cwds from session metas + workspace.roots — no remote fs browsing), `NewSession` cwd validation, project picker UI on both clients.

Decisions: single multi-workspace daemon (not daemon-per-project); project `.env` not applied per-session (documented limitation, `on_shell_env` later); MCP-in-daemon out of scope.

## 2026-07-03 (later): iOS improvement plan

Audited the iOS app + mew-mobile-core against the spec, wrote `notes/mew-ios-improvement-plan.md`. Headline findings: turn lifecycle broken in the core (`running` never set true, cleared mid-turn on `MessageEnd`, `TurnComplete`/`TurnFailed` unhandled); ~11 spec-required ServerMessage variants unhandled (rail staleness, silent errors, no subagent visibility); no scenePhase handling or local notifications (spec m4); markdown rendering inline-only. Spec blocker #1 (unstable daemon NodeId) is fixed in the uncommitted tree via `load_or_create_secret_key`. Plan item 0: land the ~570 uncommitted lines first.

## 2026-07-03 (later): headless TUI harness

Added `mew_tui::harness` — a deterministic, headless driver for the TUI so agents (and tests) can exercise it without a real terminal, provider, or async runtime. Wraps `App` over ratatui's `TestBackend`; feeds synthetic keyboard events and `AgentEvent`s; renders frames to text. Line-based script format (`type`/`key`/`submit`/`say`/`error`/`snapshot`/`size`) via `run_script`, driven by `examples/tui_driver.rs` (`cargo run -p mew-tui --example tui_driver -- <script>`). Sample at `examples/demo.tuiscript`. Zero new deps (reuses mew-message/mew-provider/mew-agent types + ratatui TestBackend). 5 harness unit tests; clippy clean. Deliberately kept out of `main.rs` (a `mew --tui-script` shim can wrap it later) to avoid colliding with the in-flight daemon-multi-workspace work.

## 2026-07-04: Clients multi-workspace plan (Complete)

Implemented the full plan from `notes/mew-clients-multi-workspace-plan.md`:

**Protocol + daemon** (commit c54bd32c):
- New `ClientMessage::ListProjects` (unit variant) + `ServerMessage::ProjectList { projects: Vec<ProjectInfo> }`
- New `ProjectInfo` struct: `path`, `display_name`, `session_count`, `last_used_at`
- Daemon handler walks `session_dir`, reads `meta.json` for each, dedupes by canonicalized path, sorts by recency
- `NewSession { cwd }` now validates: must exist and be a directory. Bad paths return `ServerMessage::Error`
- Added to exhaustive tag tests, new roundtrip test

**Web** (commit 67356567):
- New `ProjectInfo` interface, `project-list` event, `listProjects()` method on MewClient
- Store: `projects`/`projectsLoading` state, `onProjectList` handler, bridge registration
- SessionRail: project picker modal (recent projects list + free-text path input). + button opens picker; selecting a project calls `client.newSession(cwd)`

**iOS** (commit 71fa60ce):
- New `ProjectInfo` UniFFI record, `CoreEvent::ProjectList` variant
- New `list_projects()` method on mobile-core, handler in `translate_message`
- AppStore: `projectLists`/`projectsLoading` state, `fetchProjects()`, `newSession(cwd:)`
- SessionRailView: new `ProjectPickerSheet` opens from + button, lists recent projects + free-text path
- project.yml: added scheme so xcodebuild can build the app

**Docs**:
- `docs/using-mew/web-ui.md`: project picker bullet in "What you see"
- `docs/using-mew/ios-app.md`: project picker behavior in session rail
- `docs/development/dev-protocol.md`: ListProjects/ProjectList tables + Project discovery section + cwd validation note

70 protocol tests pass, all builds clean (Rust + web + iOS).

## 2026-07-05 — frontend parity audit (TUI vs web vs iOS)

Ran a three-way feature inventory (subagent fan-out) of mew-tui, mew-web-ui, and mew-ios against the mew-protocol surface. Headline findings:

- **iOS furthest behind**: no permission-mode control (core method exists, no UI), todos dropped (core discards payload), slash results no-op'd, no subagent panel (core doesn't handle ToolStart/End/Progress or Subagent* events), attachments always empty vec, no personas/thinking controls, alerts collected but unrendered, no push notifications, only cost (no tokens/context) displayed, renameSession wired but no UI.
- **Web mid**: attachment picker is a dead stub (chips never passed to prompt()), persona pill is a hardcoded local-only stub, no input history / paste-image handling, groups view-only (no create/manage UI), file-service client methods (listDir/readFilePreview/gitStatus/watchWorkspace/openPath) all unused, presence/yield tracked but unrendered, retry_wait and job-update events not surfaced, slash suggestions hardcoded to 3.
- **TUI deepest on interaction but missing daemon-era session management**: no live daemon session rail (only /sessions from local disk), no archive/pin/groups/attention badges/cross-session alerts, /resume unavailable in daemon mode, no tool-call batching (web+iOS have it), no project picker for new-session cwd, no auto-title/summary display.
- **Nobody surfaces**: group creation/management UI, client presence (ClientAttached/Detached/ControlYielded rendering), WatchWorkspace/FsChanged, OpenPath, message queueing. Persona switching has no ClientMessage at all (TUI does it in-process only).

## 2026-07-05 — parity plans written (notes/)

Three implementation plans from the parity audit, each verified against code by a planning agent:

- `notes/tui-parity-plan.md` — key discovery: `DaemonClient::translate_server_message` drops ALL session-management ServerMessages and `event_tx` is prompt-scoped, so nearly every TUI gap is blocked on a new persistent notification channel (Item 0). No protocol changes needed. Also: `/resume` in daemon mode is wired but SessionHistory replay is dropped (fix is rendering, not adding the command); tool batching is unblocked today (sensitivity already on ToolCallPart).
- `notes/web-parity-plan.md` — owns the persona protocol design (ListPersonas/SwitchPersona/PersonaList/PersonaSwitched). Key corrections: attachments are dropped at 3 layers incl. the daemon (Prompt handler ignores the field, run_turn passes vec![]); subagent-panel.tsx and settings-modal.tsx are dead code to delete. Waves: quick wins (retry toast, daemon version, jobs tab, input history) → web-only (presence, file tree, group mgmt) → protocol (personas, attachments w/ bridge upload endpoint option A, dynamic slash commands).
- `notes/ios-parity-plan.md` — phases A–G batched around `just ios-core` UniFFI regens. Key discovery: slash commands are WORSE than audited — iOS has no slash_command method at all, so `/foo` goes to the model as prompt text. Phase C (attachments) depends on the web plan's daemon fix; Phase G (personas) blocked on the web plan's protocol addition.

Cross-plan deps: daemon attachment fix (web item 1) gates iOS phase C; persona protocol (web item 2) gates iOS phase G; ListSlashCommands protocol addition (web item 10) shared by both.

## 2026-07-05 — TUI scroll smoothness (render cache + visible-window)

The TUI chat render was O(total transcript) per frame: `draw_chat` rebuilt the entire `Text` + `chat_rows` from scratch every frame, even on idle wheel-scroll. Benchmarked via `crates/mew-tui/examples/draw_bench.rs`: 3.5ms@10msg → 168ms@500msg per frame. On a 240Hz monitor (4.16ms budget) this caused the "very slightly hitchy" scroll the user reported, and would compound as transcripts grow.

Root cause: markdown *parsing* was cached (`rendered_md_cache`), but line construction, indent-prepending, `chat_rows` string building, `wrapped_height`, and ratatui's `Paragraph` scroll-skip were all O(total) per frame. ratatui's `Paragraph::render` with `Wrap` walks `scroll.y` wrapped lines to skip to the visible window (O(scroll position), not O(visible)).

**Fix** (two changes, both in `crates/mew-tui`):

1. **Cache the built transcript** (`app.rs` + `ui/chat.rs`): new `RenderedChat` struct on `App` holds the built `Vec<Line<'static>>` + `chat_rows` + `max_scroll` + `dirty_gen`. `App::ensure_chat_rendered` rebuilds only when `chat_dirty` (a generation counter) bumps or width changes; idle scroll frames skip the rebuild entirely. `mark_chat_dirty()` is called from every chat-affecting mutator: `handle_agent_event` (PartStart/PartDelta/MessageEnd/ToolStart/ToolProgress/ToolEnd/PartUpdated), `push_synthetic_message`, `clear_messages`, `rewind_to`, `toggle_bash_expanded`, `toggle_reasoning_expanded`, `clear_selection`, and selection start/drag in `events.rs`. Scroll mutators deliberately do NOT bump it.

2. **Visible-window render** (`ui/chat.rs::draw_chat`): user and reasoning text are now pre-wrapped to `md_width` at build time (via existing `wrap_text_to_width`, word-aware) so every cached line is ≤ chat width and each `Line` = exactly one visual row. Render slices `lines[scroll.y..scroll.y+height]` into a small `Text` with `scroll((0,0))` — O(visible) regardless of scroll position, killing ratatui's O(scroll.y) skip. `Wrap` kept on as a safety net for rare em-dash overflow during streaming (returns immediately per pre-wrapped line, still O(visible)). Removed dead `wrapped_height` (replaced by `lines.len()`).

**Result** (`draw_bench.rs`): idle scroll frame time is now flat at ~0.32ms regardless of transcript size (was 3.5ms@10 → 168ms@500; now 0.34/0.32/0.31/0.32 across 10/50/200/500). 540× faster at 500 messages. Rebuild path (streaming/tool events) is unchanged at O(total) — acceptable since it only fires when content genuinely changes, and during streaming the screen redraws every frame anyway.

**Tests**: 4 new unit tests in `app::tests` guard the cache invariant — scroll doesn't bump `chat_dirty`; message/selection/expansion mutations do; width change invalidates. All 128 mew-tui tests pass, clippy clean, fmt clean. `draw_bench.rs` kept as a regression benchmark; `paragraph_bench.rs` (diagnostic only) removed.
