# mew: miscellaneous features spec

Implementation spec for the non-jj, non-ADE features: cost surfacing,
notifications, scheduled tasks, `mew doctor`, and visibility for
flagged-files/secrets. Grounded in `mewcomputer/mew` @ main (2026-07-01).
Written to be executable by an agent with minimal extra design.

Each feature is independent; suggested order is the document order (cost →
notifications → doctor → scheduled → visibility), cheapest-first.

---

## 1. Cost & usage surfacing

### What exists already (do not rebuild)

- `ProviderEvent::MessageEnd { finish, usage: Tokens, cost: f64 }` — the
  provider layer already computes per-message cost from catalog `Pricing`
  (`input/output/cache_read/cache_write` per-token rates).
- The wire mirror `ProviderEventWire` reaches frontends via
  `ServerMessage::Provider`.

So the work is: aggregate, persist, and render. No pricing math.

### Aggregation & persistence

- In the agent (or the daemon's event translation — prefer the **daemon**,
  since subagent child sessions also stream through it and should roll up),
  accumulate per session:

```rust
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: f64,            // summed MessageEnd.cost
    pub turns: u32,
}
```

- Persist on `mew_session::Meta` as `usage: Option<SessionUsage>`
  (serde-default, skip-if-none — same pattern as `summary`). Write on turn
  end, not per message.
- **Subagent rollup**: child sessions have `parent_session_id` on `Meta`.
  When a child's turn ends, also add its delta into the parent's in-memory
  usage and include it in the parent's next persist. Keep a
  `own_cost` vs `total_cost` distinction if trivial; otherwise just total.
- **Daily rollup**: no separate store needed — compute from session metas
  on demand (`created_at`/`last_message_at` bucket by day). Only add a
  dedicated ledger file if listing all metas gets slow (hundreds of
  sessions is fine to scan).

### Protocol

```rust
// SessionInfo: add
pub usage: Option<SessionUsageWire>,   // mirror struct in mew-protocol

// ServerMessage: add (broadcast at turn end, throttled to once per turn)
SessionUsageChanged { session_id: String, usage: SessionUsageWire },

// ClientMessage: add
GetUsageSummary { days: u32 },
// ServerMessage reply
UsageSummary { days: Vec<DayUsage> },  // { date: "2026-07-01", cost, input_tokens, output_tokens, sessions }
```

### UI

- Session rail row: cost badge next to the ± badge, formatted `$0.43`
  (two decimals; `<$0.01` renders as `<1¢`). Tooltip with token breakdown.
- `status-footer.tsx`: running total for the **current** session, updating
  on `SessionUsageChanged`.
- `settings.tsx`: a "Usage" section rendering `UsageSummary` for 30 days —
  simple table or bar list, no charting dependency needed.
- TUI: session-info line gains cost; defer anything fancier.

### Tests

- Fake provider (`mew-provider-fake`) emits `MessageEnd` with known
  usage/cost; assert `Meta.usage` after two turns equals the sum.
- Subagent rollup test: parent spawns child via existing subagent runner
  with fake provider; assert parent total includes child.
- Protocol round-trips for the new variants (existing test pattern).

---

## 2. Notifications for background sessions

### Problem

With parallel sessions, a non-focused session blocked on
`PermissionRequest` / `AskUser`, or finishing a long turn, is invisible.
Worst case: silently blocked on approval for 20 minutes.

### Design: daemon emits, frontends decide

The daemon already broadcasts everything needed; the gap is that clients
attached to session A don't hear session B's events. Two pieces:

**(a) Cross-session event channel.** Add a lightweight global broadcast that
every connected client receives regardless of attachment:

```rust
// ServerMessage
SessionAlert {
    session_id: String,
    title: String,            // session title for display
    kind: AlertKind,          // TurnComplete | TurnFailed | PermissionNeeded | InputNeeded
    detail: Option<String>,   // e.g. tool name for PermissionNeeded
}
```

Emit points (all in the daemon where `AgentEvent`s are translated):
- `PermissionRequest` / `WorkspacePermissionRequest` surfaced → `PermissionNeeded`.
- `AskUser` tool start → `InputNeeded`.
- Turn end → `TurnComplete` (only if turn duration ≥ configurable threshold,
  default 30s — don't spam for quick turns).
- `AgentEvent::Error` terminal → `TurnFailed`.

Send `SessionAlert` to **all** clients; each client suppresses alerts for
the session it's currently viewing (client-side, since the daemon doesn't
know focus).

**(b) OS notification delivery.**
- Web UI: the Notifications API. Request permission lazily (first time a
  background session exists, show a small enable prompt — never on load).
  Clicking the notification navigates to `/session/$sessionId` (TanStack
  route exists). Fall back to a badge count on the rail + document title
  `(1) mew` when permission denied. Note: browser notifications from a
  `localhost` http origin work in Chrome/Firefox; no service worker needed
  for simple `new Notification(...)` while a tab is open.
- TUI: terminal bell + a status-line indicator; optionally OSC 777 / OSC 9
  notification escape (kitty/wezterm/iTerm support it) behind a config flag
  `[notifications] terminal_osc = true`.
- Optional later: a `notify_command = "..."` config hook the daemon shells
  with `$MEW_ALERT_KIND` / `$MEW_SESSION_TITLE` env — covers
  ntfy/telegram/whatever without mew growing integrations.

### Config

```toml
[notifications]
enabled = true
turn_complete_min_secs = 30
terminal_osc = false
notify_command = ""       # optional external hook
```

### Tests

- Daemon unit: fake agent emits PermissionRequest → assert `SessionAlert`
  broadcast to a second connected client.
- Threshold test: short turn emits no TurnComplete alert; long (mocked
  clock) does.

---

## 3. `mew doctor`

### Behavior

`mew doctor` (new subcommand in `crates/mew` CLI parsing) runs read-only
checks and prints a pass/warn/fail table. Exit code 0 if no fails, 1
otherwise (CI-friendly). `--json` flag for machine output.

### Checks (each a small function returning `CheckResult { name, status, detail, fix_hint }`)

1. **Config parse**: load `config.toml`; on error show the serde message +
   path. Warn on unknown top-level keys (serde-ignored today — collect via
   a second pass with `toml::Value` and diff against known keys).
2. **Credentials**: for each configured provider, check credential presence
   (env var or credentials store — reuse `mew-config` resolution). Do NOT
   print secrets; print source ("env ANTHROPIC_API_KEY" / "missing").
3. **Provider connectivity** (behind `--network`, off by default): cheapest
   possible authenticated call per provider (models list endpoint where the
   adapter supports it; otherwise skip with "no probe available").
4. **Catalog**: cache file exists, age < 24h, parses; model count > 0.
   `default_model` resolves in the catalog.
5. **MCP servers**: for each entry in `mcp.json`/config, attempt connect +
   `initialize` with a 5s timeout (reuse `McpClient`); report tool count or
   the error. This is the highest-value check — MCP misconfig is the top
   support burden in every harness.
6. **Daemon**: socket path exists and accepts a connection; report version
   from a `Ping`/hello if the protocol has one (add a trivial
   `ClientMessage::Ping` / `ServerMessage::Pong { version }` if absent —
   useful beyond doctor for version-skew warnings).
7. **VCS** (after jj spec lands): `jj`/`git` on PATH, versions, workspace
   detection result for cwd.
8. **Skills/personas/plugins**: directories scanned, count discovered,
   surface load errors that are currently only logged.
9. **Permissions sanity**: warn if mode is fully permissive
   (`permissive` short-circuit on) — one-line "you are running unguarded".

### Implementation notes

- Lives in `crates/mew/src/doctor.rs`; checks that need internals import
  from the relevant crates (config, catalog, mcp) — all are already
  workspace deps of the binary or can be added.
- Table rendering: plain manual padding or `comfy-table`; keep deps minimal.
- Every check individually timeboxed (5s) and panics caught
  (`catch_unwind` around each) — doctor must never hang or crash.

### Tests

- Golden tests with fixture config dirs (good config, bad toml, missing
  credential, dead MCP endpoint via a socket that accepts-then-closes).

---

## 4. Scheduled / recurring tasks

### Scope for v1

Cron-like entries that spawn a session with a saved prompt at schedule time,
run it headless in a chosen permission mode, and rely on notifications (§2)
+ the session rail for results. No retry logic, no dependencies between
jobs, no output routing beyond the session itself.

### Config

Prefer a dedicated file so the agent/users can edit it without touching main
config: `~/.mew/schedules.toml` (also honor `[schedules]` inline in
config.toml if trivial; otherwise document the file only).

```toml
[[schedule]]
name = "morning-triage"
cron = "0 8 * * 1-5"          # standard 5-field cron, local time
workspace = "/home/me/proj"    # session cwd
prompt = "Pull main, run the test suite, summarize failures into TRIAGE.md."
model = ""                     # empty = daemon default
persona = ""                   # optional
permission_mode = "standard"   # never allow "permissive" from a schedule? see guardrails
enabled = true
```

### Implementation

- New module `mew-daemon/src/scheduler.rs`. On daemon start: parse
  schedules, compute next-fire per entry with the `croner` crate (pure-Rust
  cron parsing; alternatives: `cron` crate — pick one, pin it), and run a
  single tokio task that sleeps until the earliest next-fire
  (re-computed after each fire; also watch the schedules file with the
  `notify` watcher if phase-3 ADE work already pulled it in, else re-read
  every 60s).
- Firing = exactly the daemon's internal `NewSession` + `Prompt` path with
  a synthetic client: create session with `cwd = workspace`, tag `Meta` with
  `scheduled: Option<String /*schedule name*/>` (serde-default), send the
  prompt. Turn-end / failure flows through the normal notification path
  (§2) — tag scheduled sessions' alerts so users can filter.
- **Missed fires** (daemon was down): on startup, if `last_fired` (persist
  per schedule in `~/.mew/schedules.state.json`) is before the most recent
  scheduled time, fire once (not once per missed slot). Config
  `catch_up = true|false` per entry, default true.
- **Overlap**: if the previous run's session still has a turn in flight
  when the schedule fires again, skip and alert (`TurnFailed`-style with
  detail "skipped: previous run still active").

### Guardrails

- Scheduled sessions must not sit blocked on permission prompts forever:
  if a scheduled session raises `PermissionRequest` and no client responds
  within `schedule_permission_timeout_secs` (default 300), auto-deny and
  let the agent continue/fail gracefully. Emit `PermissionNeeded` alert
  immediately so a human can respond in time.
- Refuse `permission_mode = "permissive"` in schedules unless
  `allow_permissive_schedules = true` is set globally — unattended +
  unguarded is a footgun that deserves explicit opt-in.
- Doctor (§3) gains a check: schedules parse, cron expressions valid,
  workspaces exist.

### Protocol / UI

- `SessionInfo` gains `scheduled: Option<String>`; rail shows a clock glyph
  on those rows.
- v1 management is file-editing only. Add
  `ListSchedules` / `ScheduleList` messages so the UI can at least display
  them read-only in settings; CRUD UI is a later phase.

### Tests

- Scheduler unit: injected clock, assert fire ordering, catch-up-once,
  overlap-skip.
- End-to-end with fake provider: schedule with `cron` in the past +
  catch_up → session exists with prompt as first message.

---

## 5. Flagged files & secrets visibility

### Problem

`agent.flagged_files` (re-injected after compaction) and the secrets
redaction layer (`mew-tools/src/secrets.rs`) both work silently. Users
should see them: trust features that are invisible earn no trust.

### Flagged files

- Protocol:

```rust
// ServerMessage (broadcast when the set changes)
FlaggedFilesChanged { session_id: String, files: Vec<FlaggedFileWire> },  // { path, reason?, flagged_at }
// ClientMessage
UnflagFile { session_id: String, path: String },
```

- Emit from the agent where flags are pushed (there's a single push site in
  `agent.rs` per the compaction test; add an event alongside).
- UI: small "pinned context" section in the right rail listing flagged
  paths with an ✕ to unflag. Clicking a path previews it (ties into ADE
  phase-3 preview if present; otherwise opens nothing yet — still ship the
  list).
- TUI: `/flags` command prints the list; `/unflag <path>`.

### Secrets redaction indicator

- When the redaction layer replaces content in tool output, count
  replacements and attach to the tool result part metadata (a
  `redactions: u32` field on the relevant `Part` variant in `mew-message`,
  serde-default so it's wire-compatible).
- UI: `tool-call-card.tsx` shows a small shield badge "2 secrets redacted"
  when `redactions > 0`. No reveal mechanism — display-only by design.
- Settings: read-only list of configured secret *names* (never values) from
  `SecretsConfig`, so users can confirm what's being caught.

### Tests

- Agent: flag a file, assert `FlaggedFilesChanged` event; unflag round-trip.
- Secrets: tool output containing a configured secret → part carries
  `redactions >= 1` and the value is absent from the wire message (assert on
  serialized ServerMessage).

---

## Cross-cutting

- **Protocol discipline** (same as the other specs): every message is a new
  variant; every `Meta`/`SessionInfo` field is serde-default +
  skip-if-none. Add round-trip tests per variant and one
  deserialize-old-meta test whenever `Meta` grows.
- **Config**: all new sections (`[notifications]`, `[vcs]` from the jj
  spec, schedules file) get doctor checks the moment they exist.
- **Docs**: each feature adds a page under `docs/using-mew/` (the site
  symlinks `/docs`), and dev notes under `docs/development/` only where
  architecture changed (scheduler, alerts channel).
