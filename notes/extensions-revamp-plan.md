# mew extensions revamp plan

Context: the extension system should clear two bars. Depth: someone can build a zedra-compatible remote host (fs, terminals, git, agent sessions served over iroh/QUIC to mobile clients) as a mew extension, without patching mew. Ergonomics: writing an everyday extension feels like pi (typed SDK, one file, hot reload) or Claude Code (drop a hook command or markdown skill in a directory). References: zedra PROTOCOL_SPECS.md, pi.dev/docs/latest/extensions.

Status: Phase 1 + Phase 2 shipped. Phase 3 (SDK + dev loop) not started. Revised 2026-07-10 after Phase 2 completion.

Decisions already made:
- Runtime model is subprocess + official SDKs. Language-agnostic JSON-RPC processes, DX comes from SDK packages, not from embedding a JS runtime.
- Extensions become privileged clients of the daemon protocol rather than a second parallel protocol. Security comes from per-connection principals and capability enforcement, not from schema count.
- One unified package format bundling all extension surfaces (hooks, tools, skills, commands, personas, subagents, MCP). Existing bare discovery dirs keep working, with a deprecation milestone (below).
- Wasm-compatibility is interface hygiene, not a naming scheme: plain-data payloads, no callbacks in signatures, a single `protocol_version`. No WIT-world naming until a wasm transport is actually scheduled.
- Capability granularity is proportional to risk: coarse grants for low-risk surfaces (register, ui), fine sub-scopes for the sharp ones (gate, mutate, events).
- Extensions run one instance per extension per daemon. Project-local extensions get one instance per project root (not per session). There is no per-session instancing; hook and event payloads carry `session_id` + `cwd` and extensions multiplex.

---

## Current state (audited 2026-07-08)

What exists:
- `mew-hooks::Dispatcher`: ~21 hook points (observe / mutate / gate via `HookOutcome`), `ToolRegistration`, `SlashCommandDef`, `PluginHost` with seven host callbacks. `NopDispatcher` default.
- `mew-hooks-runtime::SubprocessDispatcher`: spawns every executable in `~/.config/mew/plugins/` and `.mew/plugins/`, NDJSON JSON-RPC 2.0 over stdio, alphabetical ordering, per-plugin config (`disabled_hooks`, `matchers`, `timeout_ms`, default 5s). Gate/mutate hooks with a subject run candidates concurrently via `join_all` (max-of-timeouts, last-writer-wins by alphabetical order); this concurrency must be preserved.
- Five separate discovery mechanisms with no shared packaging: plugins, skills, personas, subagents, MCP servers (four candidate paths: `mcp.json`, `.mcp.json`, `.mew/mcp.json`, `.mew/.mcp.json`).
- `mew-protocol`: WS daemon protocol for frontends. Session-scoped and chat-centric. No registration surface, no cross-session subscription, no hook delivery. `request_id` is a monotonic `u64` (session.rs:76); permission responses are routed by id with first-responder-wins among frontends.
- The daemon runs `NopDispatcher` today (commands/daemon.rs); only standalone TUI loads plugins. The revamp inverts this: the broker must run in both.

Codebase fixes folded into this work regardless of design:
- `HookId::UserInput` / `PersonaChange` / `SessionSave` / `ModelFinish` are wired (dispatched from runtime/local.rs and commands/tui.rs) but missing from `HookId::ALL`, so config validation wrongly warns on them. Adding them to `ALL` is warning-only: `validate()` never rejects, and `disabled_hooks`/`matchers` are plain string comparisons that already honor these hook names today (verified: all four dispatch through `pipe_json_filtered`/`notify_all_filtered`). The fix silences a false "possible typo" warning and changes nothing else. Add a test pinning that.
- `SubprocessDispatcher` cannot restart a crashed plugin (TODO at runtime.rs:270-278; crashed plugins are disabled for the session). The broker refactor must make plugin lifecycle restartable — `mew ext dev` and crash recovery both depend on it.
- CLAUDE.md's "twelve hook points" is stale.

Why the zedra test fails today: the plugin protocol is almost entirely host-to-plugin; plugins cannot list sessions, attach, prompt, resume, subscribe to events, or resolve permissions, and they die with one agent's lifecycle. What a zedra server needs from mew is exactly the session/agent surface; terminals, fs, and git it implements itself as an unrestricted process.

Why the pi test fails today: the DX floor is a hand-written JSON-RPC loop; no SDK, no types, no reload, and a seven-function host API vs pi's ctx.

## Design

### Threat model (explicit)

Two independent layers, both driven by the manifest, protecting different things:

1. **Capability layer** (daemon-enforced, airtight): gates mew's data and control surface — sessions, events, hooks, registration, ui. Enforced server-side per method and per scope regardless of what the extension process does.
2. **OS sandbox** (best-effort defense in depth): gates the process's own filesystem/network/exec, which the host API deliberately does not offer. macOS Seatbelt, Linux Landlock + seccomp. Default profile: read/write own package dir + storage dir, no network. `[extension.sandbox]` widens it and every widening appears in the consent prompt. Windows has no OS layer initially; `mew ext list` and the consent prompt display sandbox status (`sandboxed` / `unsandboxed (platform)`) so the user knows which layer is active.

The window where only layer 1 exists is a real gap, so a minimal sandbox (default-deny profile, no widenings honored yet) ships with the manifest phase, not at the end. When full sandboxing lands, previously installed extensions are re-consented once.

Extension-to-extension isolation is part of the model: an extension cannot see another extension's registrations, storage (namespaced today, stays namespaced), hook traffic, or host-API calls. Events delivered to extensions describe agent/session activity, not other extensions' connections.

### Principals and capabilities

Every connection is a principal with a granted capability set. Frontends get the `client` profile (today's surface). Extensions get what their manifest requested and the user approved.

| capability | grants | risk tier |
|---|---|---|
| `storage`, `config:read` | namespaced KV; own config subtree only — the daemon extracts `[extensions.<name>]` and hands over that subtree, never the parsed whole | always granted |
| `ui` | notify + input-area widget + modal prompts. All extension-originated modals are visually attributed ("extension 'x' asks:") so they can't impersonate mew | low |
| `register` | tools + commands + providers. Name collisions are rejected at registration time with an error to the extension and a user-visible warning; no silent overwrite (today's `HashMap::insert` overwrite is fixed in the broker) | low |
| `sessions:read` / `sessions:manage` / `sessions:prompt` | list/history/meta; create/attach/fork/resume; prompt/cancel | medium |
| `permissions:resolve` | answer user-facing permission prompts (the zedra remote-approval case), same power a frontend has | high |
| `events` with `scope: session\|global` and `content: meta\|full` | `meta` = lifecycle events only (session created, turn ended, tool ran, no bodies); `full` = message/tool text. Global+full is the scariest read grant and is called out as such in consent | scope-dependent |
| `hooks:observe` | fire-and-forget hooks | low |
| `hooks:mutate` | the benign mutations: `system_prompt`, `user_input`, `chat_message` | medium |
| `hooks:mutate:headers` / `hooks:mutate:shell_env` / `hooks:mutate:chat_params` | each its own grant with separate consent. Header mutation's primary legitimate use is auth injection for gateways/proxies, so there is no in-capability header denylist — the separate consent is the control. `X-Mew-*` internal headers are reserved and stripped regardless | high |
| `hooks:gate` | approve/deny only: `on_permission_ask`, and `on_tool_execute_before` restricted to Proceed-unchanged/Block/Suppress — input mutation is NOT included | high |
| `hooks:gate:mutate` | `on_tool_execute_before` may rewrite tool input | highest |

Gate scoping and audit:
- `hooks:gate` / `hooks:gate:mutate` are declared per-tool in the manifest (`gate = ["bash", "write"]` or `["*"]`), enforced daemon-side — the existing matcher mechanism becomes a security boundary instead of a config convenience.
- Every gate decision (extension, session, tool, input hash, outcome) is written to an append-only audit log under the daemon's data dir, and `mew ext audit <name>` prints it. No rate limiting in v1; the audit log plus per-tool scope is the control, and a threshold alert can be added later if audit review shows a need.

### Permission routing

- `request_id` becomes an unguessable UUID (replaces the monotonic `u64` for permission/ask pairing).
- Every pending permission request carries its resolver set: the user-facing frontends plus extensions holding `permissions:resolve` (scoped to sessions they can see). `ResolvePermission` from any other principal is rejected and logged. First-responder-wins is kept within the resolver set — that is how answering from phone or TUI already works, and zedra needs exactly that.
- Gate hooks (`on_permission_ask`) remain a separate, earlier mechanism: they run before the request is ever surfaced, under `hooks:gate`, and are audited as above.

### Events: filtering and backpressure

- At handshake, extensions declare event-type subscriptions (e.g. `MessageEnd`, `ToolEnd` — not the `PartDelta` firehose unless asked for) in addition to hook subscriptions. The daemon never serializes an event for a connection that didn't subscribe to its type.
- Event payloads pass through the existing `SecretSet` redaction before delivery.
- Delivery uses a bounded per-connection queue with drop-oldest policy and a `dropped_events` counter surfaced to the extension (a `Lagged { count }` frame, tokio-broadcast style). A slow extension can never stall streaming or grow daemon memory. Hooks are unaffected: they are id-paired request/response with timeouts, not queued events.

### Hook execution semantics

- Concurrency: candidates for the same hook run concurrently (`join_all`), resolution is last-writer-wins in alphabetical extension order — current behavior, kept and now documented as a guarantee.
- Timeouts: observe/mutate keep the 5s default; gate hooks default to 1s. Per-extension `timeout_ms` override stays.
- Failure policy: observe hooks always fail-open silently. Mutate hooks fail-open (original value passes through) but emit a visible TUI warning, not just a log line — "extension 'x' failed on system_prompt; unmodified value used" — because a failed-open security transformation (redaction, header stripping) is a silent regression otherwise. An extension may declare `fail = "closed"` per mutate/gate hook in its manifest to block the action instead. Gate hooks default fail-open (matching today) with the same visible warning; fail-closed is the recommended manifest setting for security gates and the docs say so.

### Unified protocol

`mew-protocol` grows an extension surface alongside the client surface, same tagged-enum schema, same codec:

- Handshake: `ExtensionHello { name, version, protocol_version, requested_capabilities, hook_subscriptions, event_subscriptions }` → `ExtensionReady { granted, protocol_version }`. The daemon rejects protocol-version mismatches with a clear message. The SDK majors in lockstep with the protocol major, and CI carries a small compat matrix (current SDK vs current daemon, previous SDK vs current daemon).
- Hooks are daemon-to-extension id-paired requests over the same connection, replacing `HookId::as_wire` (removed in the same change, with a test asserting no callers remain). `HookOutcome` semantics preserved.
- Host functions (notify, storage, set_ui...) become ordinary extension-to-daemon methods under `ui` / `storage`.
- New methods behind `sessions:*`, `events`, `permissions:resolve` cover the zedra surface: `ListSessions`, `AttachSession`, `NewSession`, `Prompt`, `Cancel`, `SubscribeEvents`, `ResolvePermission`, session history/meta reads.

Transports (two, not three):
- stdio: the daemon (or standalone TUI) spawns manifest extensions and owns their pipes. No token needed; the host is the parent.
- socket: an already-running process attaches to the daemon socket with a token minted at install. Tokens live in the system keyring (same mechanism as provider credentials), with file fallback matching credential resolution today. `mew ext revoke <name>` invalidates immediately (in-memory + persisted denylist checked on every attach); `mew ext rotate-all` re-mints everything. Every socket attach is logged with principal name + token id.

Standalone TUI embeds the same broker in-process and uses only the stdio transport; there is no third transport and no duplicated dispatch path — it is the same library the daemon uses, minus the socket listener. Extensions staying available in standalone mode is a product requirement (standalone is mew's primary mode today; pi and Claude Code both work daemonless). Lifecycle hardening applies to both hosts: a signal handler runs extension `shutdown()` with a grace period before dropping child handles (`kill_on_drop` alone orphans PTYs/sockets on SIGKILL-adjacent exits), and an integration test kills the host and asserts extension PIDs are reaped.

### Manifest and packaging

One installable unit, one manifest. `mew-ext.toml` at package root:

```toml
[extension]
name = "zedra-host"
version = "0.4.0"
description = "serve this machine's mew sessions to zedra mobile clients"

[extension.entry]
run = ["node", "dist/index.js"]     # optional; a package can be purely declarative

[extension.capabilities]
sessions = ["read", "manage", "prompt"]
permissions = ["resolve"]
events = { scope = "global", content = "full", types = ["MessageEnd", "ToolEnd", "PermissionRequest"] }
hooks = { observe = true }

[extension.sandbox]
net = true                          # QUIC listener
fs.read = ["~/.local/share/mew"]    # widenings beyond package dir + storage

[provides]                          # all optional, relative to package root
skills = "skills/"
commands = "commands/"
personas = "personas/"
subagents = "agents/"
mcp = "servers.json"                # explicit file path, loaded directly (not the 4-name probe)
```

- Discovery: `~/.config/mew/extensions/<name>/` (global, daemon-scoped instance) and `.mew/extensions/<name>/` (project-local, one instance per project root, gated on project trust). Bare executables in `plugins/` and bare skill/persona/agent dirs keep working.
- `[provides]` paths feed the same five existing loaders — it is a wrapper over dir-based discovery, not a parallel loader. Dedup precedence on name collision, matching the existing project-beats-global convention: project bare dirs > project packages > global bare dirs > global packages. `mew ext doctor` lists every discovered item with its source path and which duplicate won.
- Sensitive-path denylist: the daemon refuses `fs.read`/`fs.write` widenings into `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config/mew/credentials.json`, and the keyring-adjacent config paths, regardless of consent. Grants outside the package + storage dirs are flagged high-risk in consent.
- Install: `mew ext install <git-url|path>`, `mew ext list|enable|disable|remove|revoke|audit|doctor|dev`. Git and local paths only; registry later.
- Consent is a capability delta, never a raw diff: install and upgrade prompts show newly requested capabilities with one-line plain-language explanations, sensitive ones (`hooks:gate*`, `hooks:mutate:*`, `events` global/full, `permissions:resolve`, `sandbox.net`, fs widenings) highlighted and individually confirmed.
- Legacy bare plugins: on first run after the upgrade, a one-time consent prompt lists what the plugin has been doing (all hooks incl. gate + seven host functions) and asks to re-grant. Approve → legacy profile (unchanged behavior). Decline → observe-only. Silence (non-interactive) → observe-only + warning. After the SDK phase ships, bare plugins log a deprecation warning pointing at `mew ext doctor`.

### SDK and dev loop

- `@mew/ext` TypeScript package (new workspace member next to `mew-web-client`). Shape mirrors pi:

```ts
import type { MewExtension } from "@mew/ext";

export default function (mew: MewExtension) {
  mew.on("tool.executeBefore", async (ev, ctx) => {
    if (ev.toolName === "bash" && ev.input.command.includes("rm -rf"))
      return ctx.block("nope");
  });
  mew.registerTool({ name: "deploy", description: "...", inputSchema, execute });
  mew.registerCommand("stats", { description: "...", handler });
}
```

- Protocol types are generated from the Rust schema (schemars → JSON Schema → TS). Guardrails, since generation is a known drift/build risk: schemars pinned exact, generated TS checked in, codegen runs as `just codegen` (never inside `cargo build`, no Node in the Rust build path), CI asserts regeneration is a no-op. `mew-web-client` migrates onto the generated types when convenient, not as a blocking step. Rationale for generating rather than hand-writing: the surface is about to triple, and sync-by-convention is exactly the bug source the pipeline eliminates; the guardrails cost one CI job.
- `mew ext dev <dir>`: watch + restart + re-handshake (re-registers tools/commands/hooks — the reload story), stderr tailed. Debounced ≥150ms, max 5 restarts per 10s then pause with a visible error, FD-leak covered by an integration test. All of this rides on the restart-capable broker (phase 1 prerequisite).
- Python SDK later; any language can speak raw NDJSON meanwhile.

### Two walkthroughs

**The zedra test (depth).** Manifest requests `sessions:*`, `permissions:resolve`, `events` global/full filtered to end-events, `sandbox.net`. The process handshakes over stdio, opens its iroh endpoint, and implements Register/Connect/AuthProve plus its own terminal (PTY), fs, and git RPCs natively. For zedra's agent surface it maps `AgentList`/`AgentSessions` → `ListSessions` + meta, `AgentResume`/`AiPrompt` → `AttachSession`/`NewSession` + `Prompt`, host events → its event subscription, and remote permission approval → `ResolvePermission`. Nothing in mew knows zedra exists.

**The everyday test (ergonomics).** A "block dangerous bash" gate: a directory with `mew-ext.toml` (`hooks = { gate = ["bash"] }`, entry `["node", "index.ts"]` via the SDK harness) and a ten-line `index.ts` like the snippet above. `mew ext dev .` while writing it; `mew ext install ./` when done; consent shows one highlighted line ("can approve or block bash commands"). No JSON-RPC visible anywhere. This is the acceptance bar for DX, checked in as an in-repo example.

## Phases

1. **Broker + protocol surface.** One phase, one acceptance test: an end-to-end hook delivery through the new path. Restart-capable broker refactor (prerequisite, replaces the runtime.rs:270 TODO), principal + capability enforcement, `ExtensionHello` handshake with `protocol_version`, hooks over the wire (delete `as_wire`), sessions/events/permissions methods, UUID request ids with resolver-set binding, bounded event queues + type filters + redaction, gate audit log, collision-rejecting registration, lifecycle/shutdown hardening (both hosts), `HookId::ALL` fix. Legacy plugins bridge through the broker with the consent-gated legacy profile.
2. **Manifest + CLI + consent + minimal sandbox.** `mew-ext.toml`, discovery + precedence, `provides` feeding existing loaders, capability-delta consent flow, `mew ext install/list/enable/disable/remove/doctor`, socket transport with keyring tokens + `revoke`/`rotate-all`, and the default-deny sandbox profile (package dir + storage, no net) with `[extension.sandbox]` widenings honored on macOS/Linux and clearly reported as absent on Windows. Sandbox ships here, before any SDK exists, so extensions are never developed against an unsandboxed world.
3. **TS SDK + dev loop + docs.** Codegen pipeline with guardrails, `@mew/ext`, `mew ext dev`, rewrite `docs/using-mew/plugins.md` into an extensions guide, three in-repo examples: the everyday bash-gate (DX acceptance), a status widget, the zedra-host skeleton (depth acceptance).
4. **Hardening + deprecation pass.** Sandbox profile completion and re-consent of previously installed extensions, `mew ext audit` UX, deprecation warnings for bare plugins, CLAUDE.md/docs cleanup.
5. **Later, unscheduled.** Wasm component transport (the plain-data/no-callback hygiene from phase 1 is the enabler; WIT bindings get written then, not now), Python SDK, registry.

Each phase lands independently; bare plugins keep working until the phase-4 deprecation warnings, removal in a future major.

### Phase 1+2 completion summary (2026-07-10)

**Phase 1 (shipped):** Broker + protocol surface. `mew-ext-broker` crate with `ExtensionBroker` (Dispatcher impl), `PluginSlot` restart-capable transport, `CapabilitySet` (~17 variants + risk tiers), `ConsentState` with two-phase consent + delta detection, `Principal` model, gate audit logging, collision-rejecting registration, event queue scaffolding. `mew-protocol` extended with extension handshake (`ExtensionHello`/`ExtensionReady`), hook delivery frames, and UUID `request_id` migration. Legacy plugins bridge through the broker.

**Phase 2 (shipped):** Manifest + CLI + consent + sandbox + tokens.
- `mew-ext.toml` manifest parser + validator (path traversal prevention)
- `discover_extensions` + `discover_extensions_from_dirs` (testable)
- `mew ext install` (git clone + local path, `--name`, `--force`, `--dry-run`)
- `mew ext list/enable/disable/remove/doctor` (doctor shows sandbox status)
- macOS Seatbelt sandbox via `sandbox-exec` (default-deny, `file-read-data`/`file-write-data` for pipe I/O, `escape_path` prevents profile injection, ARG_MAX guard)
- Consent resolver with two-phase prompting (batch non-sensitive, individual sensitive), persisted to `consent.json`, clamped via `intersect`, stale sentinel detection
- `[provides]` paths feed existing loaders (skills, personas, subagents)
- Attach token management (`mew ext token/revoke/rotate-all`): keyring + file fallback (0600), `constant_time_eq`, `rotate_all_tokens` partial-failure-safe
- `revoked_extensions` field on `State`
- Slash command aliases (`/models`, `/session`, `/permission`)
- CLI UX: auth selector TTY guard, `mew ext token` secret warning, `install` cleanup-on-failure, no-manifest warning, state.toml corruption doesn't block non-provider subcommands

**Deferred from Phase 2:**
- Daemon socket-attach path (requires daemon to own a broker — separate plan)
- `ExtensionHello` token field (not needed until socket-attach ships)
- Linux Landlock + seccomp sandbox
- Windows sandbox

**Phase 3 (not started):** TS SDK + dev loop + in-repo examples.
**Phase 4 (not started):** Hardening + deprecation pass.
**Phase 5 (unscheduled):** Wasm transport, Python SDK, registry.

## Phase 1 workstream breakdown

Phase 1 is decomposed into 6 workstreams (W0–W5). The structural decision is settled: a **new crate `mew-ext-broker`** holds the capability types (W2) and the broker implementation (W4). This keeps `mew-protocol` (W3) from depending on `mew-hooks-runtime` (which pulls tokio/process). The crate dependency graph:

```
mew-protocol → mew-ext-broker (lightweight types only: CapabilitySet, Principal)
mew-hooks-runtime → mew-ext-broker (broker + transport)
mew (binary) → mew-ext-broker, mew-hooks-runtime, mew-protocol
```

### W0 — Codebase fixes (no design dependency, can start now)

1. **`HookId::ALL` fix** — Add `UserInput`, `PersonaChange`, `SessionSave`, `ModelFinish` to `ALL` in `crates/mew-hooks/src/lib.rs:54-72`. Add a test that iterates all `HookId` variants and asserts membership in `ALL` — use a match-without-wildcard so the build breaks if a variant is added without updating `ALL`.
2. **CLAUDE.md update** — Change "twelve hook points" to "twenty-one hook points (`HookId` variants) across twenty-six `Dispatcher` trait methods." Update the hook name list to include all 21.
3. **Stale TODO** — Update the comment at runtime.rs:270-278 to note the broker refactor (W1) addresses it.

**Files:** `crates/mew-hooks/src/lib.rs`, `CLAUDE.md`
**Tests:** Exhaustiveness test (compile-time, no wildcard arm).
**Dependencies:** None.

### W1 — Restart-capable transport (the prerequisite refactor)

Refactor `PluginProcess` and `SubprocessDispatcher` into a restart-capable transport layer. This is pure refactoring of existing code — no protocol changes, no broker yet. The key insight from review: the transport should expose a clean interface, not the current routing logic.

**Transport interface (new):**
```rust
trait ExtensionTransport: Send + Sync {
    fn spawn(name: &str, config: &TransportConfig) -> impl ExtensionConnection;
}
trait ExtensionConnection {
    async fn call(&self, method: &str, params: Value) -> Result<String>;  // request/response
    async fn notify(&self, method: &str, params: Value);                    // fire-and-forget
    fn is_healthy(&self) -> bool;
    async fn shutdown(&self);
}
```

**9 `self.plugins` iteration sites to refactor** (all in `crates/mew-hooks-runtime/src/runtime.rs`):

| # | Method | Line | Pattern | Refactor approach |
|---|--------|------|---------|-------------------|
| 1 | `with_timeout` | 463 | `&mut self.plugins` | Remove — timeout moves to per-slot config, set at spawn time |
| 2 | `notify_all_filtered` | 586 | `for plugin in &self.plugins` | Iterate slots, lock each, skip None/unhealthy |
| 3 | `pipe_json_filtered` | 614 | `self.plugins.iter().filter(...)` | Collect healthy candidates from slots, sort, `join_all` |
| 4 | `pipe_json_raw` | 655 | `self.plugins.iter().filter(...)` | Same as #3 |
| 5 | `init` | 707 | `for plugin in &self.plugins` | Sequential, lock each slot |
| 6 | `shutdown` | 717 | `for plugin in &self.plugins` | Sequential, lock + shutdown + drop |
| 7 | `on_register_tools` | 1009 | `for plugin in &self.plugins` + **closure capture** | See below |
| 8 | `on_register_slash_commands` | 1119 | `for plugin in &self.plugins` | Sequential, lock each |
| 9 | `execute_slash_command` | 1147 | `for plugin in &self.plugins` | First-match, lock each |

**Critical: `on_register_tools` closure capture (site 7).** Lines 1042-1046 clone `plugin.writer`, `plugin.pending`, `plugin.healthy`, `plugin.timeout` into `Box<dyn Fn>` tool execution closures that live for the agent's lifetime. After a restart, these closures would point at the *old* process's handles — silent stale handles.

**Fix:** Tool closures capture a reference to the `PluginSlot` (an `Arc`), not the internals. On each tool invocation, the closure locks the slot, checks `is_healthy()`, and calls through the current `ExtensionConnection`. If the slot is mid-restart, return a clear error (`ExtensionRestarting`) rather than silently using a stale handle. This adds one `Mutex::lock` per tool call (cheap — uncontended in the common case).

**`PluginSlot` type:**
```rust
struct PluginSlot {
    name: String,
    path: PathBuf,
    config: TransportConfig,
    // None during restart; Some when running
    process: Arc<Mutex<Option<PluginProcess>>>,
    // Shared handle that tool closures capture — always points at the
    // *current* process's internals, updated atomically on restart
    current: Arc<ArcSwap<PluginHandles>>,
}
```

`ArcSwap` (or `tokio::sync::watch`) lets tool closures read the current handles without locking, and `restart_slot` atomically swaps them. This avoids lock contention on the tool-call hot path while ensuring closures always see the live process.

**`restart_slot(name)`:** Drops old `PluginProcess` (kills child via `kill_on_drop`), re-spawns from original path + config, re-wires reader task, atomically swaps `current` handles. Existing tool closures transparently use the new process.

**Files:** `crates/mew-hooks-runtime/src/runtime.rs` (major refactor), new `crates/mew-hooks-runtime/src/transport.rs` (transport trait + slot type)
**Tests:**
- Kill plugin externally → assert slot restarts within N seconds.
- **Restarted plugin receives hooks after restart** (not just "slot is non-None" — verify a hook call reaches the new process and returns a result).
- FD-leak guard: assert handle count doesn't grow across 5 restarts.
- `call()` on a restarting slot returns `ExtensionRestarting`, not panic.
- Tool closure after restart routes to the new process, not stale handles.
**Dependencies:** None (pure refactoring of existing code).

### W2 — Capability types + principal model

New types for the capability system. No runtime behavior change — pure data model. Lives in new crate `mew-ext-broker`.

**Deliverables:**
- `CapabilitySet` — the ~10 capabilities from the plan's table, with risk tiers. Supports `difference(other) -> CapabilityDelta` for consent flow (Phase 2, but the type must support it now).
- `Principal` — identifies a connection (frontend or extension) with its granted capabilities.
- `ExtensionManifest` — parsed `mew-ext.toml` structure (the type, not the parser — parser is Phase 2).
- `GateAuditEntry` — audit log record type (extension, session, tool, input hash, outcome, timestamp).

**Crate:** `mew-ext-broker` (new, lightweight — depends on `serde`, `uuid`, no tokio)
**Tests:**
- Capability set construction, `has_capability`, `is_granted`.
- **Capability delta computation** (`difference()` returns added/removed capabilities).
- Manifest field validation.
**Dependencies:** None (pure types).

### W3 — Protocol extension surface

Split into two sub-workstreams: W3a (additive, no breaking changes) and W3b (breaking `request_id` migration).

#### W3a — New extension protocol messages (additive)

Extending `mew-protocol` with the extension surface — all new variants, no changes to existing types.

**Deliverables in `crates/mew-protocol/src/lib.rs`:**
1. **Extension handshake:** `ExtensionHello { name, version, protocol_version, requested_capabilities, hook_subscriptions, event_subscriptions }` → `ExtensionReady { granted, protocol_version }`.
2. **Hook delivery frames:** `HookRequest { hook_id, params, request_id: String }`, `HookResponse { request_id, outcome }`.
3. **New RPC methods:** `ListSessions`, `AttachSession`, `NewSession`, `Prompt`, `Cancel`, `SubscribeEvents`, `ResolvePermission`, session history/meta reads.
4. **Event types:** `ExtensionEvent` with `Meta`/`Full` variants, `Lagged { count }` frame.
5. **Extension host calls:** `ExtensionHostCall` wrapping notify/storage/set_ui as typed methods.

**Files:** `crates/mew-protocol/src/lib.rs`, `crates/mew-protocol/Cargo.toml` (add dep on `mew-ext-broker`)
**Tests:** Round-trip serialization for every new variant, protocol-version rejection, capability-check simulation.
**Dependencies:** W2 (uses `CapabilitySet`, `Principal` from `mew-ext-broker`).

#### W3b — `request_id` u64 → UUID migration (breaking, cross-language)

Change `request_id` from `u64` to `String` (UUID) in existing permission/ask types. **Blast radius (verified):**

| Crate | Files | Fields/call sites |
|-------|-------|-------------------|
| `mew-protocol` | `lib.rs` | 7 struct fields: `PermissionResponse` (L84), `AskUserResponse` (L90), `PermissionRequest` (L483), `WorkspacePermissionRequest` (L491), `AskUserRequest` (L497), `SubagentPermissionRequest` (L524), `RequestResolved` (L557). Plus validation test at L1713 that asserts `request_id` must be a number — **invert this test**. |
| `mew-daemon` | `session.rs`, `lib.rs`, `client.rs` | `next_request_id() -> u64` (session.rs:133, `AtomicU64` → UUID generator); 4 call sites in lib.rs (L1769/1793/1805/1879); routing at lib.rs:682-691, 706-715 (HashMaps keyed by `u64` → `String`); client.rs `HashMap<u64, oneshot::Sender>` (L36-40), `spawn_permission_forwarder` (L648), routing at L379-436, L518-530. |
| `mew-mobile-core` | `events.rs`, `state.rs`, `lib.rs` | `PendingPermission.request_id` (L77), `PendingAskUser.request_id` (L86), `RequestResolved` (L92); state.rs fields (L79, L88); lib.rs `respond_permission`/`respond_ask_user` take `u64` (L420/436), 10+ match arms (L1191-1242, L1370-1406). |
| `mew-web-client` (TypeScript) | `index.ts`, test file | 20+ references: type defs (`request_id: number` → `string` at L47/360/367/392/518/529), handler functions (L758/764), event forwarding (L1027-1046/1090), test assertions (L165/174). |

**Note:** `mew-mcp` uses `request_id: u64` for its own JSON-RPC id namespace (MCP server protocol, not daemon/frontend). Do NOT touch it — it's a separate id space.

**Files:** `crates/mew-protocol/src/lib.rs`, `crates/mew-daemon/src/session.rs`, `crates/mew-daemon/src/lib.rs`, `crates/mew-daemon/src/client.rs`, `crates/mew-mobile-core/src/events.rs`, `crates/mew-mobile-core/src/state.rs`, `crates/mew-mobile-core/src/lib.rs`, `mew-web-client/src/index.ts`
**Tests:**
- **Old `u64` `request_id` is rejected** by the new codec (invert existing L1713 test).
- Round-trip serialization with UUID `request_id` for all 7 affected types.
- Daemon permission routing with UUID `request_id` (first-responder-wins still works).
**Dependencies:** W3a (additive surface lands first so it can be tested independently).

### W4 — Broker implementation (wiring it together)

The broker implements `Dispatcher` (so `mew-agent` doesn't change), routing hook calls to extensions with capability enforcement, concurrency, timeouts, and audit logging. Built on top of W1's transport interface and W2's capability types.

**Transport boundary:** W1 produces the `ExtensionTransport`/`ExtensionConnection` interface. The broker consumes it — all hook routing logic (`join_all`, `last-writer-wins`, `detect_outcome`, capability checks, audit) lives in the broker, NOT in the transport. This means the current `pipe_json_filtered`/`pipe_json_raw`/`notify_all_filtered`/`detect_outcome` logic moves from `SubprocessDispatcher` into the broker.

**Dispatcher trait → routing strategy partition:**

| Strategy | Dispatcher methods | Behavior |
|----------|-------------------|----------|
| **observe-event** | `on_provider_event`, `on_tool_error`, `on_subagent_start`, `on_subagent_end`, `on_turn_end`, `on_pre_model_turn`, `on_stop`, `on_pre_compaction`, `on_post_compaction`, `on_persona_change`, `on_session_save`, `on_model_finish` | Route via bounded event queue (drop-oldest + Lagged counter). Check `events` capability. No return value needed. |
| **mutate-pipe** | `on_chat_message`, `on_chat_params`, `on_chat_headers`, `on_system_prompt`, `on_shell_env`, `on_user_input` | Concurrent (`join_all`), last-writer-wins by alphabetical extension name. Check `hooks:mutate` (or sub-scope for headers/shell_env/chat_params). Fail-open with visible TUI warning. |
| **gate-audit** | `on_tool_execute_before`, `on_permission_ask` | Concurrent (`join_all`), last-writer-wins. Check `hooks:gate` (or `hooks:gate:mutate`). Per-tool scope enforcement. Audit log every decision. 1s default timeout. |
| **registration** | `on_register_tools`, `on_register_slash_commands`, `execute_slash_command` | Route to extensions with `register` capability. Collision detection (reject, don't overwrite). `execute_slash_command` routes to the extension that registered the command. |
| **lifecycle** | `init`, `shutdown` | `init`: handshake all extensions, collect registrations. `shutdown`: call with grace period (lifecycle hardening), then drop. |

**Files:** New crate `mew-ext-broker` (broker module), `crates/mew-hooks-runtime/src/runtime.rs` (remove `Dispatcher` impl from `SubprocessDispatcher`, it becomes transport-only)
**Tests:**
- **End-to-end hook delivery** (Phase 1 acceptance test): spawn a fake extension, send a hook, assert it receives and responds.
- **Capability enforcement**: extension without `hooks:gate` cannot receive gate hooks.
- **Event backpressure**: slow extension gets `Lagged` frames, doesn't block the turn loop.
- **Gate audit**: every gate decision is logged.
- **Collision rejection**: two extensions registering the same tool name → error.
- **Concurrency**: hook candidates run concurrently (`join_all`), last-writer-wins in alphabetical order — verified by timing (not sequential).
- **No-op equivalence**: broker with no extensions behaves identically to `NopDispatcher`.
- **Lifecycle hardening**: kill the host, assert extension PIDs are reaped (signal handler + grace period + PID-reap test).
**Dependencies:** W1 (transport), W2 (types), W3a (protocol surface). W3b is not blocking — the broker uses UUID `request_id` from W3a's `HookRequest`/`HookResponse` types; the permission routing migration (W3b) can land in parallel.

### W5 — Legacy plugin bridge

Existing bare-executable plugins (discovered via `PluginLoader`) must continue working through the broker with no manifest and no code changes.

**Scope:**
1. **Synthesize manifest**: For bare executables, create an `ExtensionManifest` with a legacy profile (all hooks + seven host functions) — matching current behavior exactly.
2. **Skip handshake**: Legacy plugins don't speak `ExtensionHello`. The broker auto-generates the handshake internally and grants the legacy profile.
3. **Consent gate**: On first run after upgrade, a one-time consent prompt lists what the plugin has been doing. Approve → legacy profile (unchanged behavior). Decline or non-interactive → observe-only + warning.
4. **Consent state**: Persist consent decisions (approved/declined) so the prompt only appears once per plugin.

**Files:** `mew-ext-broker` (legacy bridge module), `crates/mew-hooks-runtime/src/loader.rs` (unchanged — `PluginLoader` keeps discovering bare executables)
**Tests:**
- **Existing bare plugin runs unchanged**: a bare executable discovered via `PluginLoader` runs through the broker with no manifest, receiving all hooks it previously received.
- **Consent decline → observe-only**: declined plugin only receives observe hooks.
- **Consent persisted**: second run doesn't re-prompt.
**Dependencies:** W2 (manifest types), W4 (broker).

### Dependency graph

```
W0 (codebase fixes) ──────────────────────────────────────────→ merge anytime

W1 (restart-capable transport) ────────────────────────────────→ merge after tests

W2 (capability types, new crate) ──────────────────────────────→ merge after tests

W3a (additive protocol) ── depends on W2 ─────────────────────→ merge after tests

W3b (request_id migration) ── depends on W3a ────────────────→ merge after tests (can run in parallel with W4)

W4 (broker impl) ── depends on W1, W2, W3a ───────────────────→ merge after tests
     (W4's broker scaffolding — capability enforcement, event queues, audit log —
      can start once W2 lands, but transport integration depends on W1 completing)

W5 (legacy bridge) ── depends on W2, W4 ──────────────────────→ merge after tests
```

W0, W1, W2 can start in parallel. W3a starts once W2's types are defined. W3b follows W3a. W4's scaffolding can start once W2 lands, but its transport integration requires W1. W5 follows W4.

## Open questions

- Should `events` content=`full` even exist at global scope in v1, or is meta-only enough for zedra (which mostly needs lifecycle + its own prompts' streams)? Leaning: session-scope full + global meta covers zedra if `AttachSession` grants that session's stream; decide when building the skeleton.
- Audit log retention/rotation policy.
- Whether `mew-web-client` fully merges onto generated types in phase 3 or stays hand-written until it hurts.
