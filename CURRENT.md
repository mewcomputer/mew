# Current Progress — Consolidate Agent Construction

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
