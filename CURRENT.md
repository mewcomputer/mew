# CURRENT.md — mew Fixes + Right Rail Redesign Progress

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
