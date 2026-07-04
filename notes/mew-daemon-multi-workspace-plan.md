# mew daemon multi-workspace plan

Context: one daemon should serve sessions across multiple project directories at once — two terminals in two repos, or a phone picking any project on the machine. Today the daemon is a machine-global singleton whose per-project behavior (workspace roots, skills, personas, context files, tool cwd) is frozen at launch from `std::env::current_dir()`. The wire protocol already carries a per-session cwd (`NewSession { cwd }`, `SessionInfo.cwd`, session meta); the daemon persists it and the file browser honors it, but the agent build path drops it on the floor.

Status: planned, not started. Companion plan for the client side: web + iOS support (separate note).

Decisions already made:
- Single multi-workspace daemon, not daemon-per-project. Pairing once (iroh) and one session rail across projects is the model.
- Project `.env` is not applied per-session. Process env comes from the daemon's launch dir, unchanged; documented as a limitation. Per-session env injection via `on_shell_env` is a later feature.
- Socket-stealing fix folds into this branch as its own commit.
- `Agent` gains a `cwd` field defaulted to `current_dir()` in `Agent::new` (small diff; TUI paths unchanged) rather than a required constructor arg.

---

## Current state (audited 2026-07-03)

Already per-session, no work needed:
- Protocol: `NewSession { cwd }`, `SessionInfo.cwd`, meta persistence (`meta.set_cwd`).
- Daemon file browser (`crates/mew-daemon/src/files.rs`): resolves and scopes everything against `session_cwd`, including `open` with `.current_dir(&cwd)`. This is the pattern the rest follows.
- Shell session: per-agent (`shared_session` is an `Arc<Mutex>` wrapper, not a singleton).
- Permission engine: per-agent instance, constructor already takes `default_cwd`.

Per-agent but fed process cwd by the builder (`build_session_agent`, `crates/mew/src/main.rs:1389-1504`):
skills loader, personas loader, subagents loader, context loader (CLAUDE.md/AGENTS.md walk), project vars, shell session cwd, workspace_roots default, permission engine `default_cwd` (`build_permission_engine`, main.rs:1038).

Process-global inside mew-agent core (no cwd on `Agent`, so these call `std::env::current_dir()` at runtime):
- `tools.rs:542` — `ToolCtx.cwd`; every file tool resolves relative paths via `ctx.cwd.join(path)`
- `tools.rs:276` — permission-engine cwd per tool call (deliberately mirrors ToolCtx)
- `tools.rs:1394`, `tools.rs:1496` — `shell_background` / `shell_monitor` fallback cwd
- `agent.rs:726` — `TemplateContext.cwd` (system prompt template var)
- `agent.rs:943` — `plan_path` resolution (`PLAN.md` joined onto process cwd)
- `agent.rs:498` — cwd sent to the Auto/Auto+ permission classifier
- `runner.rs:184` — subagent template ctx; child agents also get no cwd and no workspace_roots

Bugs found during the audit:
- Resume drops cwd: `crates/mew-daemon/src/session.rs:279` passes `cwd: None` to the builder on attach even though `meta.cwd` was just read.
- Socket stealing: daemon startup unconditionally `remove_file`s the socket path (`crates/mew-daemon/src/lib.rs:174`); a second daemon at the default path silently steals the first one's socket.
- `mew chat --connect` never sends its cwd: `crates/mew-daemon/src/client.rs:142` hardcodes `cwd: None`. The web client already sends cwd.

Out of scope:
- MCP in the daemon (`connect_mcp_servers` is TUI/run-path only today; its config discovery is cwd-based and per-session MCP connections are their own project).
- Per-session `.env` injection.
- Per-project daemon sockets, pidfile hardening beyond the socket guard.

---

## Expected state

- `Agent` has `cwd: PathBuf`; all seven core call sites read it. Subagents inherit the parent's cwd and workspace roots.
- `build_session_agent` takes a cwd and feeds the loaders, permission engine, shell session, workspace_roots default, and `agent.cwd` from it. Per-session skills/personas/context: two sessions in two repos each see their own `.mew/` and CLAUDE.md.
- Daemon builder passes `params.cwd.unwrap_or(current_dir)`; sessions created without a cwd keep today's behavior (daemon's launch dir).
- Attach/resume passes `meta.cwd` through, so cwd survives daemon restarts and eviction.
- `mew chat --connect` sends its `current_dir()` on `new_session`.
- Second daemon at an in-use socket path errors out instead of stealing it.
- Global config `workspace.roots`, if set, applies as-is to every session; if empty, defaults to the session cwd (same shape as today, right cwd).
- TUI and `mew run` behavior identical (process cwd is the project dir; the default field value covers them).

## Commits

Branch: `daemon-multi-workspace`. Tests first in each commit.

### 1. socket liveness guard

`crates/mew-daemon/src/lib.rs:172`. Before `remove_file`, try connecting to the existing socket: connect succeeds → bail "daemon already running at <path>"; connection refused → stale, remove and bind. The check must also run before `daemonize()` in `main()` so the error reaches the terminal — small shared helper.

Test: bind a listener on a temp socket path, assert a second `run()` errors.

### 2. `Agent.cwd` field (mew-agent)

Add `pub cwd: PathBuf` to `Agent`, defaulted to `current_dir()` in `Agent::new`. Replace the seven runtime call sites (list above). `SimpleRunner` gains `with_cwd`: sets child `agent.cwd`, child `workspace_roots`, and the runner template ctx.

Tests (fake-provider harness, `mew-agent/src/tests.rs`): a tool call with a relative path resolves against `agent.cwd` (tempdir), not process cwd; a spawned subagent inherits the parent cwd.

### 3. thread cwd through the daemon build path

`build_session_agent` takes `cwd: PathBuf`; loaders/engine/shell/workspace_roots/subagent runner use it; sets `agent.cwd`. Daemon builder passes `params.cwd.unwrap_or(current_dir)`. Fix resume: `session.rs:279` passes `meta.cwd`. TUI/run paths pass `current_dir()` explicitly.

Tests: daemon-level — create a session with `cwd = tempdir`, drive a fake-provider tool call, assert it operates in the tempdir; attach after eviction and assert cwd survives.

### 4. TUI daemon client sends cwd

`crates/mew-daemon/src/client.rs:140` — `new_session` sends `current_dir()`. Protocol-level test.

### 5. docs

CLAUDE.md daemon notes: sessions are per-cwd; project `.env` does not apply to daemon sessions; `mcp.json` not yet wired in daemon mode. CURRENT.md entry.

## Risks / watch items

- One daemon process executes tools across multiple workspace sandboxes: the permission engine's workspace-roots escape tier must be per-session (it is — per-agent instance — but verify `default_cwd` is the session's, commit 3).
- `Agent::new` defaulting cwd to `current_dir()` means a forgotten call site degrades silently to today's behavior rather than failing loudly. Accepted for diff size; the daemon tests in commit 3 are the guard.
- Skills/personas/context loading moves from once-per-daemon to once-per-session-build. Loaders are cheap directory walks; fine at current scale, revisit if session creation gets hot.
