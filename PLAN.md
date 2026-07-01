# mew — forward plan

## What's done

Everything in the original m0–m11 roadmap, the daemon slice, shared sessions Phase 1, the web UI MVP, and the testing push. See `CURRENT.md` for implementation history. The daemon runs, the web UI connects and chats, sessions persist and resume, the protocol is exhaustively tested (717 tests).

## What's next

Three tracks, independently schedulable:

---

## Track A: Daemon lifecycle

### A1: Daemonization (survive logout) — DONE

Right now `mew daemon` runs in the foreground — tied to the terminal. Close the terminal, daemon dies. For "start it once, connect whenever" usage, the daemon needs to detach from the login session.

**Mechanism:** Double-fork + `setsid()` — the standard Unix daemonization pattern (not COW fork, just process lifecycle):

```
mew daemon --background
  └─ fork() → parent exits
       └─ child: setsid() (new session, no controlling terminal)
            └─ fork() → second child = the real daemon
                        (survives logout, ignores SIGHUP)
```

This is ~30 lines using the `daemonize` crate or raw `libc` calls. No architecture change — the daemon code is identical, just the process lifecycle differs.

**Scope:**
- `--background` flag on `mew daemon`
- Write PID to `$XDG_RUNTIME_DIR/mew.pid` (or `--pidfile`)
- Redirect stdio to `/dev/null` (or a log file via `--log`)
- SIGHUP → reload config (not kill)
- `mew daemon --stop` reads the PID file and sends SIGTERM

**Done when:** `mew daemon --background` survives closing the terminal; `mew daemon --stop` cleanly shuts it down.

### A2: Process-per-session isolation (future)

Currently all sessions share one process. A panicking tool or bad provider call kills the entire daemon and all sessions. Process-per-session gives crash isolation + OS-level workspace sandboxing.

This is a significant architecture change — see `FORK_SESSIONS.md` for the full design. The short version:

- Supervisor process is a thin relay (no Agent, no credentials, no tools)
- Each session is a `mew session-worker` subprocess owning one Agent
- Wire protocol unchanged — supervisor↔worker uses the same JSON over a socketpair
- Isolation tiers: Tier 0 (crash isolation only), Tier 1 (`chroot`), Tier 2 (`setuid`), Tier 4 (Docker)

**Recommendation:** `fork()+exec()` (not fork-only). The COW benefit for mew is negligible — the shared read-only state (~530 KB config+catalog) takes ~2ms to reload from disk, and the binary text pages are shared by the OS page cache regardless. The live state (provider pools, tokio runtime, MCP connections) must be recreated either way.

**Done when:** A crash in one session's tool execution doesn't affect other sessions; `--isolation chroot` confines a session to its workspace.

---

## Track B: Web UI

### B1: Finish Phase 1 polish (MVP chat) — DONE

Phase 1 is mostly done — streaming text, tool calls, permissions, model picker, session list drawer, themes, syntax highlighting all work. Remaining gaps:

- **Subagent visualization**: `SubagentStart`/`SubagentStatus`/`SubagentEnd` events arrive but aren't rendered. Need a sidebar or inline card showing running subagents with their progress messages.
- **AskUserRequest rendering**: the wire type exists and is bridged to the store, but no UI component renders the question card. Need an inline form with options.
- **Todo list rendering**: `TodosUpdated` events arrive but aren't displayed. Need a checklist panel.
- **Reconnect with backoff**: the client connects once on mount; if the WS drops, it doesn't retry. Need exponential backoff + re-attach to the same session.
- **Playwright e2e**: browser-level test that loads the page, sends a prompt, sees streaming text. Catches the catastrophic regressions (bridge not serving dist, MIME types wrong, WS proxy broken).

**Done when:** all daemon events have a UI representation; a WS drop reconnects automatically; a Playwright test guards the load→prompt→stream path.

### B1.5: Artifact rendering

Let the model present rich content (HTML, SVG, Mermaid, formatted markdown) via a `present_artifact` tool call. Tool-based — no new `Part` variant, no protocol changes. The `ToolStateCompleted.metadata` field (already on the wire) carries the artifact data. Web UI renders it in a sandboxed panel; TUI shows the text output. See `ARTIFACTS.md` for the full design.

### B2: Phase 2 — Workspace awareness

- **Live diff view**: show file changes as the agent makes them, with accept/reject per hunk
- **File tree**: sidebar showing the workspace, with modified files highlighted
- **REST endpoints**: the daemon needs HTTP endpoints (not just WS) for file listing, diff retrieval, session metadata — or the bridge proxies these

### B3: Phase 3 — Agent orchestration

- **Subagent graph**: visualize the parent→child subagent tree with live status
- **Multi-model comparison**: run the same prompt through two models side by side (requires session branching — preserved for future exploration)

### B4: Phase 4 — Polish

- **Command palette** (Cmd+K): quick actions, session switching, model switching
- **PWA + mobile**: service worker, responsive layout, install prompt
- **Per-hunk change review**: elevated from Phase 2 — review and accept/reject individual code changes inline

---

## Track C: Agent robustness

### C1: Subagent + job resume

When the daemon restarts (or a session resumes from disk), background subagent tasks and shell jobs are lost. They should **Report as failed**: mark the task as failed in history so the model knows

**Done when:** a resumed session shows completed/failed background tasks rather than silently losing them.

### C2: Config hot-reload

Editing `config.toml` requires restarting the daemon. With the daemonization work (A1), restarts are cheaper but still disruptive. Options:
- **SIGHUP**: supervisor re-reads config, applies what it can (workspace roots, permission rules) without restarting sessions
- **Worker-level**: since workers are fresh processes (after A2), they always read the latest config on spawn

### C3: Permission cache persistence

`AllowSession` grants are in-memory only. Resuming a session from disk loses them, so the user re-approves tools they already allowed. Persist the permission cache per session (in `meta.json` or a sidecar file).

**Done when:** a resumed session remembers `AllowSession` decisions from before the disconnect.

---

## Explicit non-goals

- **MessagePack codec** — JSON is fine for the wire protocol. MessagePack saves bandwidth but adds a binary dependency to the TS client. Not worth it until bandwidth is a measured bottleneck.
- **Session branching / threading** — forking a session at a point in history to explore two paths. Interesting but no active design.
- **Discord/iOS frontends** — the protocol supports them, but no work planned until the web UI is solid.
- **Plan/execute workflow** — personas shipped standalone with planner/builder personas. No separate plan-mode UI planned.
