# mew × jj: version control integration spec

Implementation spec for integrating [jujutsu (jj)](https://github.com/jj-vcs/jj)
into mew as the durable undo / review / branching layer. Written to be
executable by an agent without much additional design work. Grounded in
`mewcomputer/mew` @ main (2026-07-01).

Relationship to other docs: this replaces the "checkpoints" idea from the
brainstorm and provides the substrate for **review-before-apply** and the
**diff stats** from the ADE plan (phase 1.3) / **git status** (phase 3.1).

---

## Design principles

1. **jj is optional.** Everything degrades: jj repo → full features;
   git-only repo → status/diff features via git porcelain, no undo/review;
   no VCS → nothing lights up. Never require jj.
2. **Shell out, don't link.** Use the `jj` CLI with `--no-pager` and
   machine-readable output (`-T` templates, `--tool`-free commands). Do not
   depend on `jj-lib`: it's unstable, huge, and version-locks you to their
   release cadence. Pin a minimum version (check `jj --version`, require
   ≥ 0.23 or whatever is current when implementing; verify template syntax
   against that version's docs).
3. **The daemon owns jj, not the agent.** The agent must never run raw `jj`
   commands as part of its normal loop (users can still ask it to via bash).
   All integration lives in the daemon/session layer so it's deterministic
   and can't be prompted away.
4. **One jj change per turn** is the core invariant everything else builds on.

---

## New crate: `mew-vcs`

Add `crates/mew-vcs` to the workspace. Single responsibility: detect and
drive the workspace VCS. No dependency on agent/daemon crates (daemon depends
on it, not vice versa).

```rust
// crates/mew-vcs/src/lib.rs

pub enum VcsKind { Jj, Git, None }

pub struct Vcs {
    kind: VcsKind,
    root: PathBuf,       // workspace root (canonical)
    jj_bin: PathBuf,     // resolved `jj` path, or default "jj"
}

impl Vcs {
    /// Detect by walking up from `cwd`: `.jj/` dir → Jj (jj colocated repos
    /// have both `.jj` and `.git`; `.jj` wins), else `.git` → Git, else None.
    pub fn detect(cwd: &Path) -> Vcs;

    pub fn kind(&self) -> VcsKind;

    // ---- jj-only operations (return Err(Unsupported) on Git/None) ----

    /// `jj new -m <msg>` — start a fresh change on top of @.
    pub fn new_change(&self, message: &str) -> Result<ChangeId>;

    /// `jj describe -m <msg>` — set/replace @'s description.
    pub fn describe(&self, message: &str) -> Result<()>;

    /// Current @ change id: `jj log -r @ --no-graph -T 'change_id.short()'`.
    pub fn current_change(&self) -> Result<ChangeId>;

    /// Latest operation id: `jj op log --limit 1 --no-graph -T 'id.short()'`.
    pub fn current_op(&self) -> Result<OpId>;

    /// `jj op restore <op>` — restore repo (incl. working copy) to an op.
    pub fn op_restore(&self, op: &OpId) -> Result<()>;

    /// `jj squash --from <src> --into <dst>` (approve flow).
    pub fn squash(&self, from: &ChangeId, into: &ChangeId) -> Result<()>;

    /// `jj abandon <change>` (reject flow).
    pub fn abandon(&self, change: &ChangeId) -> Result<()>;

    /// `jj new <parent>` — create sibling change (branch-from flow).
    pub fn new_child_of(&self, parent: &ChangeId, message: &str) -> Result<ChangeId>;

    // ---- works on both jj and git ----

    /// Unified status. jj: `jj status` parse, or `jj diff --summary`.
    /// git: `git status --porcelain=v2 -z`.
    pub fn status(&self) -> Result<Vec<FileStatus>>;   // { path, kind: Added|Modified|Deleted|Renamed }

    /// Diff stat for a range. jj: `jj diff --stat -r <rev>` / git: `git diff --numstat`.
    pub fn diff_stat(&self, rev: Option<&str>) -> Result<DiffStat>;  // { added, removed, files }

    /// Full unified diff for the review UI. jj: `jj diff --git -r <rev>`.
    pub fn diff_unified(&self, rev: Option<&str>, path: Option<&str>) -> Result<String>;
}
```

Implementation notes:

- All commands run with `cwd = self.root`, `--no-pager`, env
  `JJ_CONFIG=/dev/null`? **No** — do NOT nuke user config (they may have
  custom immutable heads etc.), but DO pass `--color=never` and set
  `--quiet` where supported. Capture stderr for error surfacing.
- Every command gets a 10s timeout (`tokio::time::timeout` around
  `tokio::process::Command`). jj is fast; a hang means a locked repo — fail
  the operation, don't block a turn.
- **Concurrency**: jj takes its own repo lock, but mew must serialize its own
  jj invocations per workspace to avoid interleaving turn-boundary ops from
  parallel sessions in the same repo. Put a `tokio::sync::Mutex` in a
  `HashMap<PathBuf /*repo root*/, Arc<Mutex<()>>>` inside the daemon and hold
  it across each multi-command sequence (e.g. describe + snapshot check).
- Unit tests: use `tempfile` + `jj git init` in a fixture; gate the test
  module on `which::which("jj").is_ok()` so CI without jj skips instead of
  fails (`#[ignore]`-by-default + a justfile target `just test-vcs`, or a
  runtime skip with an eprintln).

---

## Feature 1: turn snapshots + undo

### Behavior

- When a session starts in a jj workspace, the daemon records the current op
  id as `session_base_op` on the session (in-memory) and persists per-turn
  boundaries.
- **Before each turn** (in the daemon, where `Prompt` is handled — see
  `mew-daemon/src/session.rs`, wherever the turn lock is taken):
  1. `vcs.current_op()` → store as `turn.op_before`.
  2. If config `vcs.change_per_turn = true` (default) and the working copy
     is not empty of description-worthy context: `jj describe` the current
     @ with a message derived from the prompt (first line, truncated to 72
     chars, prefixed `mew: `), **only if @'s description is empty** — never
     overwrite a user-written description. Check via
     `jj log -r @ --no-graph -T 'description'`.
- **After each turn**: `vcs.current_op()` → `turn.op_after`;
  `vcs.diff_stat(None)` for the stats broadcast (this replaces/feeds the
  ADE-plan phase 1.3 numbers when jj is present — jj's working-copy
  snapshotting sees bash edits too, fixing the undercount).
- jj auto-snapshots the working copy whenever a jj command runs, so simply
  running `current_op` at turn boundaries creates restorable points. No file
  copying by mew, ever.

### Persistence

Extend `mew_session::Meta` (additive, serde-default):

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub vcs_kind: Option<String>,           // "jj" | "git"
```

Add a new JSONL record type in the session log (alongside messages) OR a
sidecar `<session>.turns.json`; prefer a sidecar to keep the message log
schema untouched:

```json
{ "turns": [ { "index": 0, "op_before": "abc123", "op_after": "def456",
               "change_id": "xyz", "added": 471, "removed": 0 } ] }
```

### Protocol

```rust
// ClientMessage
ListTurnSnapshots { session_id: String },
RestoreToTurn { session_id: String, turn_index: u32 },   // op_restore(op_before)

// ServerMessage
TurnSnapshots { session_id: String, turns: Vec<TurnSnapshot> },
RestoreComplete { session_id: String, turn_index: u32 },
```

`RestoreToTurn` semantics — this is the subtle part:

1. Refuse if a turn is in flight (turn lock held) → `Error`.
2. `vcs.op_restore(op_before)` — reverts files.
3. Truncate conversation: the session writer needs a
   `truncate_after_turn(index)` — mew-session is JSONL, so implement as
   rewrite-to-temp + rename, dropping records after the turn boundary. Add
   turn-boundary markers to the JSONL if none exist (a small
   `{"type":"turn","index":N}` record is fine and old readers must skip
   unknown types — verify `Reader` tolerates unknown record types; if not,
   fix that first).
4. Invalidate hashline snapshots for the session
   (`SnapshotStore` — add a `clear()` if absent) so stale-hash detection
   re-baselines against restored content.
5. Broadcast `SessionHistory` replay to attached clients + `RestoreComplete`.

Important: `jj op restore` moves the *whole repo* back, including changes the
user made concurrently in another tool. Guard: before restoring, diff the
current op against `op_after` of the latest mew turn; if they differ (user
edited since), require a `force: bool` flag on `RestoreToTurn` and surface a
confirmation in the UI ("workspace changed since this session's last turn").

### UI

- `session.$sessionId.tsx`: per-turn hover action "restore to before this
  turn" on user messages; confirmation dialog; force-path warning text.
- TUI: a `/undo` slash command (restore to before last turn) — trivial once
  the protocol exists, and mirrors `jj undo` mental model.

---

## Feature 2: review-before-apply (scratch-change mode)

### Model

Instead of a hand-rolled patch-staging overlay, use jj's model directly:

```
main change (user's @)          scratch change (agent works here)
        │                                │
        └── jj new -m "mew review: …" ──▶ @ moves to scratch
                                          agent edits apply FOR REAL here
        approve  = jj squash --from scratch --into main
        reject   = jj abandon scratch  (+ restore op)
        partial  = jj squash -i is interactive → NOT usable; see below
```

This solves read-after-write for free (the agent reads real files), keeps
hashline untouched, and the review diff is just `vcs.diff_unified(scratch)`.

### Mechanics

- New permission mode `review` (alongside existing modes in
  `mew-config/src/permissions.rs` / the permission engine). In review mode:
  - Edit-class tools (`Write`, `Edit`, hashline apply) are auto-allowed
    (they land in the scratch change) — no per-edit prompts.
  - Bash keeps its normal tier (it can do non-file things).
  - On the **first** edit-class tool call of a turn, the daemon (not the
    tool) runs `enter_review(session)`:
    `main_change = vcs.current_change()`; `scratch = vcs.new_change("mew review: <prompt summary>")`.
    Store both on the session.
- Turn ends → daemon computes `diff_unified` + `diff_stat` of scratch and
  broadcasts:

```rust
// ServerMessage
ReviewPending {
    session_id: String,
    change_id: String,
    stat: DiffStat,
    files: Vec<ReviewFile>,   // { path, status, added, removed }
},

// ClientMessage
ReviewDecision {
    session_id: String,
    decision: ReviewVerdict,  // ApproveAll | RejectAll | Files { approve: Vec<String> }
},
ReviewDiff { session_id: String, path: Option<String> },  // fetch unified diff lazily

// ServerMessage
ReviewDiffResult { path: Option<String>, diff: String },
ReviewResolved { session_id: String, applied: bool },
```

- **ApproveAll**: `squash(scratch → main)`, then `jj new` off main so the
  next turn starts clean. Broadcast `ReviewResolved{applied:true}`.
- **RejectAll**: `abandon(scratch)`. Feed a synthetic system note into the
  session ("user rejected the proposed changes") so the agent knows on the
  next turn.
- **Per-file** (the zed-style part): jj's interactive squash won't work
  programmatically. Implement as:
  `jj squash --from scratch --into main -- <approved paths...>` (jj supports
  fileset args on squash — verify against pinned version; if the installed
  version lacks it, fall back to: squash all, then `jj restore --from main- -- <rejected paths>`;
  write a version-gated code path). Then abandon the remainder if empty
  (`jj diff --summary -r scratch` → empty → abandon).
- **Multi-turn reviews**: if the user sends another prompt while a review is
  pending, keep working in the same scratch change and re-emit
  `ReviewPending` at each turn end with cumulative diff. This matches "agent
  touched 14 files across 6 turns" — one review surface, not six.
- **Per-hunk** accept/reject: defer. File-level is v1. Document that hunk
  granularity would use `jj split` with a scripted diff editor
  (`JJ_EDITOR` + `--tool`), which is doable but fiddly — leave a TODO with
  that pointer.

### Guardrails

- Entering review mode requires `VcsKind::Jj`; if mode is `review` but the
  workspace isn't jj, fall back to the `standard` permission mode and emit a
  one-time `ServerMessage::Error`-adjacent notice (add a `Notice { text }`
  variant if none exists) explaining why.
- Crash-safety: on daemon restart, sessions with a stored pending scratch
  change re-emit `ReviewPending` on attach. Store
  `pending_review: Option<{change_id, main_change}>` on `Meta`.
- Flagged files (`agent.flagged_files`) interplay: flags should reference
  the scratch state; nothing to do beyond making sure flag paths are
  workspace-relative already (verify).

### UI

- New `review-panel.tsx` in the right rail: file list with status + per-file
  ± and checkboxes, lazy diff view per file (reuse `code-block.tsx` with a
  diff grammar — shiki/highlight.js both have `diff`), Approve all / Approve
  selected / Reject all buttons, cumulative stat header.
- `input-area.tsx` shows a "review pending" banner; sending another prompt
  is allowed (multi-turn flow above).
- TUI: `/review` opens a list-and-diff pane; approve/reject keybinds. Can
  ship after web.

---

## Feature 3: session ⇄ change mapping & branch-from

(Named "branch-from" — NOT "fork"; `FORK_SESSIONS.md` in-repo is about
process isolation and the terminology collision will confuse contributors.)

- On `NewSession` in a jj workspace with `vcs.change_per_session = true`
  (default false for v1 — turn on after feature 1 is stable): `jj new` a
  change described `mew session: <title>` and record `change_id` on `Meta`.
- **Branch-from-message**: `ClientMessage::BranchSession { session_id, from_turn: u32 }`:
  1. Create the new session (existing machinery), copying JSONL history up
     to `from_turn` (reuse the truncation writer from feature 1, but
     copy-truncate into the new session file instead of rewriting).
  2. Set `parent_session_id` (field exists on `Meta`).
  3. jj side: `new_child_of(parent = change at that turn's op, msg)` — the
     turn sidecar gives you the change/op at that boundary.
  4. Reply `SessionReady` for the new session.
- Compare view (later): `jj diff --from <sess A change> --to <sess B change>`
  exposed as `ReviewDiff`-style message. Don't build UI for this in v1;
  the CLI story ("run `jj diff` yourself") is fine initially.

---

## Config

```toml
[vcs]
enabled = true            # master switch; false = never run jj/git
jj_bin = "jj"             # override binary path
change_per_turn = true    # describe @ at turn start when description empty
change_per_session = false
review_requires_jj = true # if false, review mode silently degrades to standard

[permissions]
# "review" joins the existing mode list
```

Wire into `mew-config::Config` as `pub vcs: VcsConfig` with serde defaults.

---

## Build order & tests

1. `mew-vcs` crate + fixture tests (detect, status, diff_stat on both jj and
   git fixtures; op round-trip: edit → current_op → edit → op_restore →
   assert file content).
2. Turn snapshots: daemon turn-boundary hooks + sidecar persistence +
   `ListTurnSnapshots`. Test: scripted session against `mew-provider-fake`
   producing a Write tool call; assert sidecar ops recorded and stat > 0.
3. `RestoreToTurn` incl. JSONL truncation + hashline snapshot clear +
   force-guard. Test the guard: mutate a file out-of-band between turns,
   assert restore without `force` errors.
4. Review mode: permission-mode plumbing → enter_review → ReviewPending →
   ApproveAll/RejectAll. Per-file path last (version-gate).
5. Branch-from.

Protocol round-trip tests for every new message variant (follow the existing
pattern in `mew-protocol` tests). All new `Meta` fields serde-default with
a deserialize-old-meta test.
