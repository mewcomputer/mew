# Design: Process-per-session isolation

## Goal

Give each daemon session its own OS process so that:

1. **Crash isolation** — a panicking tool or bad provider call kills only that session, not the daemon or other sessions.
2. **Workspace isolation** — the OS (not just the agent's `workspace_roots` enforcement) prevents a session from touching files outside its workspace.
3. **Multi-user / shared-env safety** — different sessions can run as different users, so one person's agent can't write another person's files.
4. **Resource limits** — per-session CPU/memory limits via cgroups or container limits.

This is the architecture Ed describes: fork + copy-on-write so session workers share the binary and read-only pages, with per-process isolation for mutable state and filesystem access.

## The core tension: fork() vs fork()+exec()

### fork() without exec() (COW, what Polytoken does)

```
Supervisor process (tokio multi-threaded runtime)
  │
  ├─ fork() ──→ Child process (only calling thread survives)
  │              │
  │              ├─ Kernel marks all pages COW (shared until written)
  │              ├─ Binary, loaded libs, config, catalog → shared read-only
  │              ├─ Provider connection pool, MCP clients → STALE (must recreate)
  │              ├─ Tokio runtime threads → GONE (must start fresh runtime)
  │              └─ Starts single-threaded tokio runtime, creates new connections
  │
  └─ Supervisor routes frames to child over inherited socketpair
```

**Pros:**
- Instant fork — no re-loading the binary, re-parsing config, re-fetching the catalog
- Read-only state (config, catalog, tool definitions, personas, skills) is shared via COW — no memory duplication until a session mutates something
- This is why Polytoken forks: the binary + all the setup work is amortized across sessions

**Cons:**
- **tokio + fork is dangerous.** Only the calling thread survives fork. Tokio's worker threads, IO driver thread, blocking pool — all gone. Any thread-local state (including Rust's `std::rng`, allocator state, TLS in libraries) is in an undefined state.
- Signal handlers inherited from the parent can fire in the child with no handler context.
- File descriptors are inherited — the child must close every FD it doesn't need (the supervisor's listener, other sessions' sockets, MCP server pipes, etc.) or risk FD leaks and subtle corruption.
- TLS connection state, HTTP/2 connection pools — all in an undefined state post-fork. Must be recreated.

**How to make it safe:**
- Fork from a **single-threaded context** — before the multi-threaded runtime starts, or from a dedicated fork-thread that has no tokio runtime attached.
- In the child: immediately close all inherited FDs except the socketpair, start a **fresh single-threaded tokio runtime**, create new provider/MCP connections.
- The COW benefit is real but bounded: the child shares read-only pages (binary, libs, config struct, catalog) but must allocate its own runtime, connection pools, and Agent state. For a Rust binary that's ~30-50 MB of shared read-only pages vs ~5-10 MB of per-child mutable state.

### fork()+exec() (safe, no COW)

```
Supervisor process
  │
  ├─ spawn("mew", "session-worker", "--fd", N) 
  │    ├─ Fresh process image — no inherited tokio state
  │    ├─ Owns its own runtime from the start
  │    ├─ Re-loads config, catalog, personas from disk (fast — already cached)
  │    └─ Connects to supervisor over the pre-opened socketpair FD
  │
  └─ Supervisor routes frames to child
```

**Pros:**
- No tokio+fork danger. Each worker is a clean process from the start.
- No FD leak risk — the child starts with only the FDs you explicitly pass.
- Simpler to reason about. The bin_e2e test already does this (spawns `mew daemon` as a subprocess).
- Works with `seccomp`, `chroot`, `setuid` — all applied in the worker's startup before the runtime starts.

**Cons:**
- Startup cost: re-loading the binary (~20-50ms), re-parsing config, re-fetching catalog (mitigated by disk cache — the catalog has a 24h cache).
- No COW memory sharing. Each worker has its own copy of the binary in memory (though the OS may share text pages via the page cache).

### Recommendation: fork()+exec() for v1, fork()-only as a future optimization

The COW benefit is real but the complexity of safe fork-without-exec in a tokio application is high. For a first implementation, `fork()+exec()` gives us 90% of the value (crash isolation, workspace isolation, multi-user) with 10% of the risk. The ~50ms startup latency is acceptable for session creation — sessions are not created at high frequency.

If startup latency becomes a problem later, we can add a fork-only fast path where:
- The supervisor keeps a warm process pool (pre-forked children with no runtime yet)
- On `NewSession`, send an init message to a warm child, which then starts its runtime

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  mew daemon (supervisor)                            │
│                                                     │
│  ├─ WebSocket listener (Unix socket + TCP)          │
│  ├─ Connection router                               │
│  │    maps session_id → WorkerHandle                │
│  │    maps client_id → (session_id, ws_sink)        │
│  ├─ SessionManager                                  │
│  │    ├─ active: HashMap<SessionId, WorkerHandle>   │
│  │    ├─ loading: per-session locks (as today)       │
│  │    └─ session_dir: PathBuf                        │
│  └─ Per-client writer tasks (as today)              │
│                                                     │
│  WorkerHandle {                                     │
│    session_id: String,                              │
│    child: tokio::process::Child,                    │
│    socket: UnixStream (or pipe pair),                │
│    clients: Vec<client_id>,                          │
│  }                                                  │
└──────────────┬──────────────────────────────────────┘
               │ socketpair / pipe
               │ (same ServerMessage/ClientMessage protocol)
               ▼
┌─────────────────────────────────────────────────────┐
│  mew session-worker (child process)                  │
│                                                     │
│  ├─ Own tokio runtime (fresh)                       │
│  ├─ Agent (provider, tools, MCP, session writer)    │
│  ├─ Reads ClientMessages from socketpair             │
│  ├─ Runs agent turns, executes tools                 │
│  ├─ Streams ServerMessages back through socketpair   │
│  ├─ Owns its own MCP server subprocesses             │
│  └─ [optional] chroot / setuid / seccomp / cgroup    │
└─────────────────────────────────────────────────────┘
```

### Why the supervisor is a relay, not a participant

The supervisor does **not** build or own an `Agent`. It only:

1. Accepts WebSocket connections.
2. Routes `ClientMessage`s from a WebSocket to the right worker's socketpair.
3. Routes `ServerMessage`s from a worker's socketpair to all attached clients' WebSockets.
4. Manages worker lifecycle (spawn, kill, track).
5. Scans disk for idle sessions (for `ListSessions`).

This means the supervisor has **no provider credentials, no MCP connections, no tool execution**. It's a thin router. A compromised session worker can't escalate to the supervisor's privileges because the supervisor never had agent-level capabilities.

### The protocol is unchanged

The wire protocol between client and daemon is already `ClientMessage`/`ServerMessage`. The supervisor↔worker channel uses **the same protocol** over a Unix socketpair instead of a WebSocket. This means:

- The worker is essentially a mini-daemon that reads from a Unix socket instead of accepting WebSocket connections.
- `forward_events` in the worker writes `ServerMessage`s to the socketpair; the supervisor reads them and broadcasts to clients.
- Permission requests flow through identically: worker sends `PermissionRequest`, supervisor broadcasts to all clients, any client responds, supervisor routes the response back.

The only addition is an internal `SessionInit` message the supervisor sends to the worker on spawn, carrying the session ID, cwd, model, provider, and config path.

## Session lifecycle

### NewSession

1. Supervisor generates a `session_id` (ULID).
2. Supervisor opens a `socketpair()` (or creates a `UnixStream` pair).
3. Supervisor spawns `mew session-worker --fd <N> --session-id <id> --session-dir <dir> [--cwd <dir>] [--provider <p>] [--model <m>]`.
4. The child inherits one end of the socketpair (FD N).
5. Child starts its tokio runtime, opens a session `Writer`, builds an `Agent` (provider, tools, MCP, personas, skills — same as `build_session_agent` today).
6. Child sends `SessionReady { session_id, model, provider }` on the socketpair.
7. Supervisor registers the `WorkerHandle` and routes the client to it.

### AttachSession

1. Supervisor checks `active` for an existing `WorkerHandle`.
2. If found: attach the client to that worker's socketpair. No new process needed.
3. If not found (idle session on disk):
   - Acquire per-session loading lock (as today).
   - Spawn a new worker with `--session-id <id> --resume`.
   - Worker loads history from disk via `Reader::load_from` + `Agent::load_messages`.
   - Worker sends `SessionHistory` (if it loaded from disk) + `SessionReady`.
   - Supervisor routes the client.

### ListSessions

1. Supervisor queries each active `WorkerHandle` for metadata (model, provider, client_count) via an internal `QueryMeta` message, or caches this from the initial `SessionReady`.
2. Supervisor scans `session_dir` for idle sessions (as today).
3. Returns merged list.

### Last-client disconnect

1. Supervisor removes the client from the worker's client list.
2. If no clients remain:
   - **Phase 1 (immediate kill):** send a shutdown signal to the worker, wait for it to exit, reap the child. Session is persisted to disk by the worker before exit.
   - **Phase 2 (grace period):** keep the worker alive for N seconds; if a new client attaches, cancel the timer. Otherwise kill.
3. If a turn is in progress: send `Cancel` to the worker, wait for the turn to drain, then kill.

### Crash recovery

If a worker process exits unexpectedly:
1. Supervisor detects `Child::wait()` returning (or the socketpair closing).
2. Supervisor removes the `WorkerHandle` from `active`.
3. The session is still on disk (the worker was appending to the JSONL).
4. A subsequent `AttachSession` spawns a fresh worker that resumes from disk.
5. Any attached clients receive a `ServerMessage::Error { message: "session worker crashed" }` and can reconnect.

This is a major improvement over the in-process model: a panic in a tool or provider today kills the entire daemon and all sessions. With process isolation, only the crashed session dies.

## Isolation tiers

The design supports progressive isolation, selectable per-session or globally:

| Tier | Mechanism | What it prevents | Complexity |
|------|-----------|-----------------|------------|
| 0 — None | `fork()+exec()` only | Crash isolation | Low (just spawn) |
| 1 — Workspace | `chroot` to workspace root after fork | All FS access outside workspace | Medium |
| 2 — User | `setuid` to a dedicated `mew-session` user | Writing to the daemon user's files | Medium |
| 3 — Sandbox | `seccomp` filter (deny `mount`, `ptrace`, raw sockets) | Escaping via kernel syscalls | High |
| 4 — Container | Docker/Podman with volume mounts | Full isolation + resource limits | High (infra) |

Tier 0 is the default and costs nothing beyond the subprocess spawn. Tiers 1-2 are achievable in pure Rust with `nix` crate calls after fork, before exec. Tier 4 requires Docker/Podman installed and configured.

Ed's recommendation (run daemons as a different user, sessions as yet another user) maps to Tier 2. His Docker jailing idea maps to Tier 4.

## Hard parts

### MCP servers

MCP servers are subprocesses spawned by the agent (via `connect_mcp_servers`). In the worker process, these are spawned fresh — the worker owns its own MCP server subprocesses. This is correct: MCP servers should be scoped to the session, not shared across sessions.

The supervisor never spawns MCP servers. This means the supervisor doesn't need `mcp.json` or any MCP configuration — it's purely a router.

### Provider connections

HTTP connection pools (reqwest/hyper) don't survive fork or exec. Each worker creates its own provider connection. This is fine — providers are stateless HTTP endpoints, and connection pooling is per-process.

Credentials: the worker needs API keys. Options:
- **Pass via env vars** (supervisor sets them on the child's environment). Simple, but keys are visible in `/proc/<pid>/environ`.
- **Pass via a pipe** (supervisor sends credentials in the `SessionInit` message). More secure, but the supervisor holds credentials.
- **Worker reads its own config** (loads `config.toml` + keyring independently). Simplest, but means every worker does config loading.

Recommended: worker reads its own config (option 3). The config is already on disk, and config loading is fast (~1ms). This keeps the supervisor credential-free.

### tokio + fork (only if doing fork-only)

If we later want the COW fast path (fork without exec):
1. Supervisor runs its accept loop on a **single-threaded runtime**.
2. On `NewSession`, call `fork()` from the main thread (no tokio context).
3. In the child: close all FDs except the socketpair, call `tokio::runtime::Builder::new_current_thread().enable_all().build()`, start a fresh runtime.
4. The child inherits the config/catalog via COW (read-only, never mutated) but creates fresh provider/MCP/Agent state.

The risk: any C library that uses threads internally (e.g., OpenSSL via rustls, some database drivers) may be in an undefined state post-fork. Rust's `std` allocators are generally fork-safe, but TLS backends are not. Using `rustls` (pure Rust) instead of `native-tls` (OpenSSL) makes this safer.

### Graceful shutdown

When the supervisor receives SIGTERM/SIGINT:
1. Stop accepting new connections.
2. Send a `Shutdown` message to each worker.
3. Each worker finishes its current turn (or cancels after a timeout), flushes its session writer, and exits.
4. Supervisor waits for all children (with a timeout), then force-kills stragglers.
5. Supervisor exits.

### Broadcast latency

In the in-process model, `session.broadcast()` is a `try_send` on an `mpsc::UnboundedSender` — nanoseconds. In the process model, every event goes:

```
worker → socketpair write → supervisor read → WebSocket send to each client
```

This adds ~10-50µs per event (Unix socket round-trip). For streaming text deltas (which fire every ~10ms), this is negligible. For high-frequency tool progress updates, it could add up, but those are already rate-limited.

## What changes in the codebase

### New: `mew session-worker` subcommand

```rust
// A stripped-down daemon that reads from a Unix socket FD instead of
// accepting WebSocket connections. Owns one Agent, one session.
Commands::SessionWorker {
    fd: i32,              // socketpair FD
    session_id: String,
    session_dir: PathBuf,
    cwd: Option<PathBuf>,
    resume: bool,         // load from disk if true
    provider: String,
    model: String,
}
```

The worker:
1. Takes the FD from `--fd`.
2. Converts it to a `UnixStream`.
3. Reads `ClientMessage`s, writes `ServerMessage`s (same JSON codec).
4. Builds an `Agent` via `build_session_agent` (reuses the existing function).
5. Runs the same `handle_connection` logic — but over a Unix stream instead of a WebSocket.

The existing `handle_connection` is already generic over `S: AsyncRead + AsyncWrite + Unpin + Send`. It works over both WebSocket and raw Unix streams. The only change is the handshake: the worker skips `accept_async` (no WebSocket handshake) and reads/writes JSON directly.

Actually — to keep the WebSocket protocol end-to-end (so the supervisor can use the same `send_msg`/`decode` code), the worker could do a WebSocket handshake over the Unix socketpair. But that's unnecessary complexity. Simpler: the supervisor reads `ServerMessage` JSON from the socketpair and writes `ClientMessage` JSON to it. The WebSocket layer is only between client and supervisor.

### `SessionManager` changes

```rust
pub struct SessionManager {
    session_dir: PathBuf,
    active: Mutex<HashMap<String, WorkerHandle>>,
    loading: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    // Config for spawning workers
    worker_config: WorkerSpawnConfig,
}

pub struct WorkerHandle {
    pub session_id: String,
    pub child: tokio::process::Child,
    pub socket: UnixStream,  // supervisor's end of the socketpair
    pub model: Option<String>,
    pub provider: Option<String>,
    pub client_count: usize,
}

// create() spawns a subprocess instead of building an in-process Agent
impl SessionManager {
    pub async fn create(&self, cwd: Option<PathBuf>) -> Result<Arc<WorkerHandle>> {
        let session_id = format!("sess_{}", ulid::Ulid::new());
        let (supervisor_sock, worker_sock) = UnixStream::pair()?;
        
        let mut child = tokio::process::Command::new(current_exe()?)
            .arg("session-worker")
            .arg("--fd").arg("3")
            .arg("--session-id").arg(&session_id)
            .arg("--session-dir").arg(&self.session_dir)
            .args(cwd.iter().map(|c| ("--cwd", c)))
            // Pass worker_sock as FD 3
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        
        // ...wait for SessionReady from the worker...
    }
}
```

### `handle_connection` changes

The supervisor's `handle_connection` becomes a relay:

```rust
// For each ClientMessage from the WebSocket:
//   1. Parse it.
//   2. If it's NewSession/AttachSession: use SessionManager.
//   3. Otherwise: forward the raw JSON to the worker's socketpair.

// For each message from the worker's socketpair:
//   1. Parse as ServerMessage.
//   2. Broadcast to all attached clients' WebSockets.
```

The supervisor doesn't translate `AgentEvent` → `ServerMessage` anymore — the worker does that (it has the `Agent`). The supervisor just relays.

### What stays the same

- **Wire protocol** — `ClientMessage`/`ServerMessage` unchanged.
- **TS client** — unchanged.
- **Web UI** — unchanged.
- **`build_session_agent`** — reused in the worker.
- **`forward_events`** — reused in the worker (writes to the socketpair instead of a broadcast).
- **Session persistence** — worker owns its `Writer`, same as today.
- **`SessionManager` API** — `create`/`attach`/`list` keep the same signatures; the internals change from in-process `Agent` to subprocess `WorkerHandle`.

## Migration path

### Phase 1: In-process (today, done)

`SessionManager` holds `Arc<Session>` with an in-process `Agent`. Single process, single crash domain. This is what we just shipped.

### Phase 2: Process-per-session (this design)

`SessionManager` holds `WorkerHandle` with a subprocess. Same wire protocol, same UI. The `Session` struct is replaced by `WorkerHandle`. The supervisor is a relay.

Migration is mechanical: `SessionManager::create` changes from "build an Agent" to "spawn a worker"; `handle_connection` changes from "run the agent" to "relay frames to the worker". The worker is a thin wrapper around the existing `build_session_agent` + `handle_connection` logic.

### Phase 3: Isolation tiers

Add `--isolation <tier>` flag. Tier 0 (none) is default. Tier 1 (`chroot`) and Tier 2 (`setuid`) require root or capabilities. Tier 4 (container) requires Docker/Podman.

### Phase 4: COW fast path (optional)

If startup latency matters, add fork-without-exec for workers. Requires single-threaded supervisor runtime + careful FD/thread handling. Only worth it if worker startup > 100ms becomes a problem.

## Open questions

1. **Worker stdout/stderr**: Should the worker's logs go to the supervisor (via the piped stdout/stderr) or to their own log file? Supervisor-piped is simpler for centralized logging but couples log volume to session count.

2. **Resource limits per session**: cgroups require root or systemd integration. For a local dev tool, probably skip. For hosted, essential.

3. **Provider connection cost**: Each worker creates its own provider HTTP client. If you have 20 sessions, that's 20 HTTP connection pools. For a local tool this is fine. For hosted, a shared provider proxy (supervisor holds the pool, workers route through it) might be better — but that re-introduces the supervisor as a participant.

4. **Socket vs WebSocket over the socketpair**: Using raw JSON over the socketpair is simpler but loses the WebSocket framing (which handles large messages, ping/pong, etc.). For the supervisor↔worker channel, messages are small JSON — raw JSON with newline delimiting is sufficient. Alternatively, use `tungstenite` over the Unix socketpair for consistency.

5. **Config hot-reload**: If the user edits `config.toml`, existing workers don't see it. The supervisor could send a `ReloadConfig` message, or workers could watch the file. Or: workers always re-read config on `NewSession`/`AttachSession` (since they're fresh processes in Phase 2, this is automatic).
