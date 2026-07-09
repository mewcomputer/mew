# Current Progress — Consolidate Agent Construction

## 2026-07-09 — Phase 2a: Manifest Parser + Discovery (in progress)

**Status: IN PROGRESS**

### Completed
- **Phase 1 ✅** — Manifest parser: `parse_manifest()` + `validate_manifest()` in manifest.rs. Added `toml` dep. 3 tests (parse valid, parse invalid, validate denylist).
- **Phase 2 ✅** — Extension discovery: `discovery.rs` with `DiscoveredExtension`, `ExtensionScope`, `discover_extensions(cwd)`. Scans `~/.config/mew/extensions/` and `.mew/extensions/`. Dedup: project beats global. 4 tests.

### Next
- **Phase 3** — SpawnSpec enum + broker integration (manifest-based extensions get scoped capabilities)
- **Phase 4** — Loader changes (load_markdown_dirs_with_extra) + [provides] integration
- **Phase 5** — `mew ext` CLI (list/enable/disable/remove/doctor)
- **Phase 6** — Integration tests + verify

---

## Previous: W4 + W5 (Phase 1 complete)

**Status: COMPLETE ✅**

Implemented the `ExtensionBroker` that implements `mew_hooks::Dispatcher`, routing hook calls to extension processes with capability enforcement, concurrency, timeouts, audit logging, and event delivery. Replaces `SubprocessDispatcher` as the runtime's `Dispatcher` impl.

### What was done

**Phase 1 — Move routing logic into the broker:**
- Created `crates/mew-ext-broker/src/broker.rs` with `ExtensionBroker` struct + full `Dispatcher` impl (all 26 methods)
- `ExtensionBroker::from_dirs_filtered_with_config()` — same signature as `SubprocessDispatcher`'s, creates `Principal::extension()` with `CapabilitySet::legacy_full()` per slot
- Routing helpers (`should_fire`, `notify_all_filtered`, `pipe_json_filtered`, `pipe_json_raw`, `detect_outcome`) moved into the broker
- `build_dispatcher` in `setup/agent.rs` switched from `SubprocessDispatcher` to `ExtensionBroker`
- `SubprocessDispatcher` left as dead code for rollback safety
- `call_via_handles` made `pub` in transport.rs for broker consumption

**Phase 2 — Capability enforcement:**
- `hook_capability(HookId) -> Option<Capability>` maps each hook to its required capability
- `check_capability()` checks `Principal.has_capability()` + `should_fire()` before routing
- Legacy extensions get `CapabilitySet::legacy_full()` (all caps) — no-ops for them, active for future manifest-based extensions

**Phase 3 — Gate audit logging:**
- Created `crates/mew-ext-broker/src/audit_log.rs` with `AuditLog` (Mutex<BufWriter<File>> + PathBuf)
- `on_tool_execute_before` and `on_permission_ask` write `GateAuditEntry` per extension
- `set_session_id()` method for audit session context
- `audit_entries()` public accessor for tests/future CLI

**Phase 4 — Event queues (scaffolding):**
- Created `crates/mew-ext-broker/src/event_queue.rs` with `EventQueue` (bounded mpsc, drop-oldest, Lagged)
- Not wired — Phase 2 activates it when socket transport lands

**Phase 5 — Collision-rejecting registration:**
- `registered_tools`/`registered_commands` as `Mutex<HashMap<String, String>>` (interior mutability)
- Duplicate tool/command names from different extensions are skipped with a warning
- Same-extension re-registration allowed (restart case)

**Phase 6 — Tests:**
- Created `conflicting-plugin.rs` example binary (registers `sample-echo`, transforms `on-system-prompt` with `[conflicting-plugin]`)
- 6 integration tests: e2e_hook_delivery, noop_equivalence, collision_rejection, gate_audit, last_writer_wins, capability_enforcement
- 35 unit tests (capabilities, audit_log, event_queue, manifest, principal)
- All 41 tests pass, clippy clean, fmt clean

### Acceptance Criteria
- AC.1 ✅ — `cargo build -p mew` compiles with `ExtensionBroker`
- AC.2 ✅ — Existing `SubprocessDispatcher` tests pass (11 + 5)
- AC.3 ✅ — `test_e2e_hook_delivery` passes
- AC.4 ✅ — `test_noop_equivalence` passes
- AC.5 ✅ — `test_collision_rejection` passes
- AC.6 ✅ — `test_gate_audit` passes
- AC.7 ✅ — `test_capability_enforcement` passes (HooksGate + sub-scope non-implication)
- AC.8 ✅ — clippy clean, fmt clean
- AC.9 ✅ — `test_last_writer_wins` passes (sample-plugin wins, alphabetically last)
- AC.10 (stretch) — Not implemented (Phase 7 lifecycle hardening deferred)

### Files
- New: `crates/mew-ext-broker/src/broker.rs`, `audit_log.rs`, `event_queue.rs`
- New: `crates/mew-ext-broker/tests/broker_integration.rs`
- New: `crates/mew-hooks-runtime/examples/conflicting-plugin.rs`
- Modified: `crates/mew-ext-broker/Cargo.toml`, `src/lib.rs`, `src/capabilities.rs`
- Modified: `crates/mew-hooks-runtime/src/lib.rs`, `src/transport.rs`
- Modified: `crates/mew/src/setup/agent.rs`, `crates/mew/Cargo.toml`

---

## Previous: Consolidate Agent Construction

## Status: COMPLETE ✅

## What was done

Eliminated ~420 lines of triplicated agent-construction code by making `run_tui` and `build_and_run` delegate to `build_session_agent`, and extracting shared helpers.

### Phase 1 ✅ — build_session_agent accepts dispatcher
- Added `dispatcher: Arc<dyn Dispatcher>` and `todos_path: Option<PathBuf>` params
- Kept sync (daemon AgentBuilder closure is sync)
- Updated daemon.rs call site to pass `NopDispatcher` + `None`

### Phase 2 ✅ — make_provider_builder helper
- Extracted to `setup/providers.rs`
- Returns `Box<dyn Fn(&str) -> Result<Arc<dyn Provider>, String> + Send + Sync>`
- Replaced 3 inline closure sites (agent.rs, chat.rs x2)

### Phase 3 ✅ — wire_subagents helper
- Extracted to `setup/agent.rs`
- Called inside `build_session_agent` (for daemon path)
- Called again by `run_tui`/`build_and_run` after `register_plugin_tools` (refresh with plugin tools)
- Replaced 3 inline blocks (agent.rs, chat.rs x2)

### Phase 4 ✅ — run_tui delegates to build_session_agent
- Replaced ~130 lines of inlined construction with single call
- TUI-specific steps remain: dispatcher construction, MCP status, sidebar, App state

### Phase 5 ✅ — build_and_run delegates to build_session_agent
- Replaced ~80 lines of inlined construction with single call
- Dropped unused MCP tool loading (was only keeping clients alive, never read)

### Phase 6 ✅ — Verification
- All 8 mew tests pass
- All 137 mew-tui tests pass
- clippy clean, fmt clean, arch-check passes

## Acceptance Criteria
- AC.1 ✅ — Agent::new count in chat.rs = 0
- AC.2 ✅ — Same (both run_tui and build_and_run delegate)
- AC.3 ✅ — set_provider_builder = 0 inline closures (all use make_provider_builder)
- AC.4 ✅ — SubagentStart::new count in chat.rs = 0 (only in wire_subagents)
- AC.5 ✅ — build_session_agent is sync, register_plugin_tools called by callers
- AC.6 ✅ — No behavior change (all tests pass)
- AC.7 ⚠️ — chat.rs at 1148 lines (target was <1000, but remaining code is non-duplicated)

## 2026-07-08 — Heal corrupted state.toml on startup

**Problem:** `mew` crashed with `unknown provider t` when `state.toml` had
stale `last_provider = "t"` / `last_model = "t"` values (likely written by
an earlier partial run during refactoring). Resolvers trusted state blindly.

**Fix (two layers):**

1. **Resilient read** — `setup::providers::resolve_provider` /
   `resolve_model_opt` now validate persisted state against `cfg.providers`
   before using it. Falls back to the built-in default when the persisted
   value is unknown, so a corrupted state file doesn't crash startup.

2. **Startup heal prompt** — `mew-config` gained `validate_state`,
   `heal_state`, and `backup_state_file`. `main.rs` calls
   `startup_state_health_check` before subcommand dispatch:
   - clean state → no prompt, continue.
   - dirty state + interactive TTY → warn + `[y/N]` prompt. `y` → back up
     to `state.toml.bak.<unix-epoch-seconds>` and heal; `n` → exit 0.
   - dirty state + non-TTY (piped stdin, CI) → exit 2 with a message to
     re-run from a terminal.

**Files touched:**
- `crates/mew-config/src/lib.rs` — `validate_state`, `heal_state`,
  `backup_state_file`, `state_file_path` (+ 10 tests).
- `crates/mew/src/setup/providers.rs` — resolver signature now takes `&Config`,
  new `is_known_model` helper, 6 new tests for the corrupted-state case.
- `crates/mew/src/main.rs` — `prompt_yn`, `startup_state_health_check`,
  load `cfg` early, wired into all four resolve_provider/resolve_model_opt
  call sites (Run / Chat / Daemon / no-subcommand).

**Verification:**
- `cargo test -p mew --bin mew` → 66 passed
- `cargo test -p mew-tui --lib` → 135 passed
- `cargo test -p mew-config` → 116 passed (10 new)
- `cargo clippy -p mew --all-targets -- -D warnings` → clean
- `cargo fmt -p mew -- --check` → clean
- `just arch-check` → passes
- Manual E2E (via `expect`): heal-yes path created
  `state.toml.bak.1783495830` with the original content and rewrote
  `state.toml` keeping only `disabled_plugins = ["buddy"]`. Decline path
  left state unchanged and exited 0. Non-TTY path exited 2.

## 2026-07-08 — Surface connection errors in iOS reconnect UI

**Problem:** When the iOS app couldn't connect to a daemon over iroh
(e.g. pairing failures, relay unreachable, allowlist rejection), it just
showed "Waiting to retry. The daemon will reconnect automatically." with
no diagnostic info. The actual errors were `warn!`'d in the Rust core but
went nowhere on iOS (no `tracing` subscriber installed). Impossible to
diagnose without Console.app.

**Fix:** Threaded the connection failure reason through the event system
so it shows in the UI the user is already looking at.

1. `DaemonStatus::Backoff` gained an `error: String` field (dropped `Copy`
   from the enum since `String` isn't `Copy`). — `events.rs`
2. `connect_and_run` now returns `Result<ConnOutcome>` where `ConnOutcome`
   is `UserDisconnected` (stop) or `Dropped { reason }` (retry with
   reason). Every break point in the message loop sets a `drop_reason`
   ("connection error: {e}", "connection closed", "closed by daemon",
   "failed to send message to daemon"). The reconnect loop binds the
   reason from the match and passes it into the `Backoff` event. — `lib.rs`
3. `SessionRailView.connectingState` now renders the error string in red
   monospaced `.footnote` text below the status description when non-empty.
   Added `statusError` computed property that extracts it from the
   `Backoff` case. Updated all `.backoff` pattern matches in Swift for
   the new 2-field shape. — `SessionRailView.swift`
4. Regenerated Swift bindings + XCFramework via `just ios-core`.

**Files touched:**
- `crates/mew-mobile-core/src/events.rs` — `Backoff { error }`, drop `Copy`.
- `crates/mew-mobile-core/src/lib.rs` — `ConnOutcome` enum,
  `connect_and_run` return type + `drop_reason` tracking, reconnect loop
  error threading.
- `mew-ios/mew/SessionRailView.swift` — `statusError`, error display in
  `connectingState`, pattern match updates.
- `mew-ios/MewMobileCore/Sources/MewMobileCore/mew_mobile_core.swift` —
  regenerated bindings (auto).

**Verification:**
- `cargo build -p mew-mobile-core` → clean (no warnings)
- `cargo clippy -p mew-mobile-core --all-targets -- -D warnings` → clean
- `cargo test -p mew-mobile-core` → 19 passed
- `cargo fmt -p mew-mobile-core -- --check` → clean
- `just ios-core` → framework + bindings rebuilt
- `xcodebuild ... build` (iPhone 17 sim) → BUILD SUCCEEDED
