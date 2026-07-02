# mew × ADE: integration plan

Bringing ZCode's ADE tooling and task/file management into the mew harness
(frontend → daemon → agent → provider architecture).

Sources: [ADE Tools](https://zcode.z.ai/en/docs/ADE-tools),
[Task & File Management](https://zcode.z.ai/en/docs/task-management),
`mewcomputer/mew` @ main (2026-07-01).

---

## 0. Gap analysis — what mew already has vs what zcode ships

| zcode feature | mew today | gap |
|---|---|---|
| Task list w/ title, relative time, status dots | `SessionInfo` (id, state, model, timestamps, summary) + `session-rail.tsx` | no per-session workspace, no unread/failed status, no `+/-` counts |
| `+/-` line counts per task | nothing — `ToolEnd` only carries `success` | need diff-stat accumulation per session |
| Task views: Grouped / Workspace / Timeline | flat list only | need `cwd`/workspace on `Meta` + `SessionInfo`; grouping is UI + one persisted field |
| Task groups (colors, drag-drop, rename) | none | new persisted structure (daemon-side, not per-client) |
| Archive + auto-archive | `DeleteSession` only | new `archived` flag + retention policy |
| Command Center (⌘K) | none in `mew-web-ui` | pure frontend + a few existing protocol messages |
| Quick Actions in search box | none | subsumed by command palette |
| Workspace file tree + git status + "changed only" filter | none | new protocol surface + daemon file/git service |
| Drag file into chat / Add to Chat | `Attachment { path, mime }` already exists on `Prompt` | wiring only |
| Repo Wiki | `mew-context` discovers AGENTS.md but doesn't generate | new agent-driven generation task, cached artifact |
| Terminal panel (⌘J) | none in web UI (TUI users have a shell) | optional; needs PTY-over-WS |
| Browser element context | none | optional, frontend-heavy |
| SSH remote workspace | none | out of scope for v1 (mew daemon can already run remotely; document that instead) |
| Goal mode / progress checklist | `todo_*` tools + `todo-panel.tsx` already exist | mostly done; surface per-task progress in the rail |

The plan below is ordered so each phase ships standalone value and the
protocol changes accrete cleanly (all new `ServerMessage` / `ClientMessage`
variants are additive, so old clients keep working — serde `deny_unknown_fields`
is not set, keep it that way).

---

## Phase 1 — session metadata: workspace, status, diff stats

This is the foundation. Everything zcode's sidebar shows hangs off three bits
of metadata mew doesn't track yet.

### 1.1 `cwd` / workspace on sessions

`ClientMessage::NewSession` already accepts `cwd: Option<String>`, but it's
not persisted or echoed back.

- `mew-session::Meta`: add `pub cwd: Option<String>` (serde default, skip-if-none —
  same pattern as `custom_title`). Old JSONL metas deserialize fine.
- `mew-protocol::SessionInfo`: add `cwd: Option<String>`.
- Daemon: stamp cwd at session creation; include it in `ListSessions` responses.
- Derive a display "workspace name" client-side: last path component of cwd,
  fall back to `~`. Don't store the display name; derive it.

### 1.2 Session status beyond Active/Idle

zcode's dots are: running, unread, failed.

- **running**: you have this — a session with an in-flight turn. Expose it:
  extend `SessionState` (or add a parallel `activity: Option<Activity>` field
  to `SessionInfo` to avoid breaking the enum) with `Running`, plus
  broadcast `ServerMessage::SessionActivityChanged { session_id, activity }`
  when a turn starts/ends. Frontends currently only learn about the session
  they're attached to; the rail needs cross-session signals.
- **failed**: set when a turn ends via provider error / tool hard-failure.
  Store `last_turn_failed: bool` on `Meta`, clear on next successful turn.
- **unread**: purely client-side. `mew-web-ui/src/stores/session.ts` tracks
  `lastSeenMessageAt` per session in localStorage; compare against
  `last_message_at` from `SessionInfo`. Don't put unread in the daemon —
  it's per-client state and you support multi-client attach.

### 1.3 Per-session `+/-` diff stats

Two viable designs; recommend (a):

**(a) Accumulate at the tool layer (recommended).** `Write`/`Edit` (and the
hashline patcher) know exactly what changed. Add to `ToolOutput` an optional
`file_delta: Option<FileDelta { path, added, removed }>` (or a side-channel on
`ToolCtx` that the agent drains after each call). The agent aggregates into a
per-session `ChangeStats { added: u64, removed: u64, files: HashSet<PathBuf> }`,
persists it on `Meta`, and the daemon broadcasts
`ServerMessage::SessionStatsChanged { session_id, added, removed, files_changed }`
after each tool round.

- Counting: for `Write` on a new file, added = line count; for `Edit`/hashline,
  the patcher already computes line-level ops (you have `similar` in-tree from
  the 3-way merge work — reuse it for `Write`-over-existing-file diffs).
- Bash escapes this (agent runs `sed`/`git apply` in shell). Accept the
  undercount for v1, or…

**(b) Git-based**: run `git diff --numstat` against a baseline ref per session.
Accurate for everything including bash edits, but requires a git repo, a
baseline concept (what if the user commits mid-session?), and polling. Save
this for the file-tree phase (1.3a numbers + git status markers together
cover zcode's UX).

### 1.4 UI

`session-rail.tsx`: render workspace grouping headers, status dot, relative
time, and `+734 −7` badge per row. All data now arrives via `SessionList` +
the two new broadcast messages.

**Estimated protocol additions**: 2 `SessionInfo` fields, 2 `ServerMessage`
variants, 1 `Meta` field-set. No breaking changes.

---

## Phase 2 — task organization: views, groups, archive

### 2.1 Where groups live

Groups must be daemon-side (multi-client, and the TUI should eventually see
them too), but they're not per-session data — don't stuff them into `Meta`.
Add a small sidecar store owned by the daemon:

```
~/.mew/sessions/groups.json
{
  "groups": [
    { "id": "grp_x1", "name": "gomoku-ai", "color": "blue", "order": 0 }
  ],
  "membership": { "sess_abc": "grp_x1", ... },
  "order": { "grp_x1": ["sess_abc", "sess_def"] }
}
```

Load at daemon start, write atomically on mutation (temp file + rename — same
discipline as your JSONL writer).

### 2.2 Protocol

```rust
// ClientMessage
CreateGroup { name: String, color: Option<String> },
UpdateGroup { group_id: String, name: Option<String>, color: Option<String>, order: Option<u32> },
DeleteGroup { group_id: String },            // ungroup-and-delete semantics: members survive
AssignSessionGroup { session_id: String, group_id: Option<String>, position: Option<u32> },
ArchiveSession { session_id: String, archived: bool },

// ServerMessage
GroupList { groups: Vec<GroupInfo> },        // include in the ListSessions reply flow
GroupsChanged { groups: Vec<GroupInfo> },    // broadcast after any mutation
```

`SessionInfo` gains `group_id: Option<String>` and `archived: bool`
(`archived` also lands on `Meta` so it survives daemon restarts).

### 2.3 Views + archive semantics

- **Grouped / Workspace / Timeline** are pure client-side projections of the
  same `SessionList`: grouped = `group_id`, workspace = `cwd` (phase 1),
  timeline = sort by `last_message_at` desc. Persist the chosen view + sort in
  localStorage.
- **Archive**: `ListSessions` returns everything; archived rows are filtered
  into a separate view with Restore (`archived: false`) and Delete (existing
  `DeleteSession`). Auto-archive matching zcode's rule (completed, no pin,
  older than retention window) is a daemon-side sweep on startup + daily
  timer; make the window configurable in `config.toml`
  (`[sessions] auto_archive_days = 30`, 0 = off). Skip "pinned" for v1 or add
  `pinned: bool` to `Meta` at the same time — it's one more field and zcode's
  rule references it.

### 2.4 UI

`session-rail.tsx` grows: view switcher menu, group headers with color dot +
collapse triangle, drag-and-drop (dnd-kit is the usual choice with your React
stack), right-click context menu (move to group / archive / rename — rename
already exists). Archived view is a new route or a rail mode.

---

## Phase 3 — workspace file tree, git status, add-to-chat

This is the biggest new surface and the one that most changes mew's shape:
the daemon becomes a (read-only, scoped) file server for frontends.

### 3.1 Daemon file service

New module `mew-daemon/src/files.rs` (or a small `mew-files` crate if the TUI
will share it):

```rust
// ClientMessage
ListDir { session_id: String, path: Option<String> },   // relative to session cwd
ReadFilePreview { session_id: String, path: String, max_bytes: Option<u64> },
GitStatus { session_id: String },
WatchWorkspace { session_id: String, enabled: bool },

// ServerMessage
DirListing { path: String, entries: Vec<DirEntry> },
FilePreview { path: String, content: String, truncated: bool, language: Option<String> },
GitStatusResult { entries: Vec<GitEntry> },   // { path, status: Added|Modified|Deleted|Renamed }
FsChanged { paths: Vec<String> },             // debounced watcher events
```

Implementation notes:

- **Scope enforcement**: reuse the agent's `workspace_roots` machinery —
  every path is canonicalized and must sit under the session cwd. This is a
  security boundary, not a convenience; the web bridge is on TCP.
- **Git status**: shell out to `git status --porcelain=v2 -z` (or `gix` if you
  want to stay pure-Rust; porcelain parsing is ~40 lines and zero new deps).
  Directory aggregation (zcode shows changed-state rolled up to folders) is a
  client-side reduce over the entry list.
- **Watcher**: `notify` crate, debounced 300–500ms, only while at least one
  client has `WatchWorkspace` on. Emit `FsChanged` and let the client re-query
  git status lazily. Don't stream full listings on every event.
- **"Show changed files only"**: client-side filter over `GitStatusResult` —
  no protocol needed.

### 3.2 Add-to-chat / drag into chat

The plumbing already exists: `Prompt.attachments: Vec<Attachment>`. Work:

- File tree row context menu → "Add to Chat" pushes an `Attachment { path }`
  chip into `input-area.tsx`.
- Drag-and-drop from the tree into the input does the same.
- Agent side: verify text attachments are inlined into the turn (if
  attachments are currently image-only, extend the prompt assembly to read
  text files through the same read tool path so line numbers / hashline
  snapshots stay consistent).
- zcode's killer combo — "changed files only" + add-to-chat for self-review —
  falls out for free: add a one-click "review changes" button that attaches
  all `GitStatusResult` paths and pre-fills a review prompt. Cheap, high value.

### 3.3 UI

New `file-tree.tsx` in the rail (toggle between tasks ⇄ files, like zcode's
"View Files" / "Back to Tasks"), fuzzy filter box, git badges, single-click
preview (reuse `code-block.tsx` + `markdown-body.tsx` in the right rail or a
preview pane), double-click → OS open (needs a daemon `OpenPath` message that
shells `open`/`xdg-open` — gate it behind the permission engine).

---

## Phase 4 — command center (⌘K) + quick actions

Almost entirely frontend; ship it early if you want a quick win — it has no
protocol dependencies beyond what exists.

- `command-palette.tsx` using `cmdk` (the standard React lib for this).
- Command sources, in priority order:
  1. **Actions**: new task, toggle theme, open settings, switch persona
     (`SwitchPersona` path exists), change model (`ListModels` exists),
     toggle right rail, archive current session, cancel turn.
  2. **Sessions**: fuzzy over `SessionList` titles + summaries → navigate.
  3. **Files** (after phase 3): fuzzy over a cached workspace listing →
     preview / add to chat.
- Show keybindings on the right of each row; register the same bindings
  globally (⌘K palette, ⌘J terminal-later, ⌘N new task — mirror your TUI
  keymap where sensible so muscle memory transfers).
- The zcode "search box doubles as quick actions" pattern: make the rail's
  search input focus just open the palette pre-filtered to sessions. One
  component, two entry points.

---

## Phase 5 — repo wiki

zcode's Repo Wiki = agent-generated orientation doc, cached, regenerable.
mew already has the pieces: subagents + `mew-context`.

- **Generation**: a built-in command (slash command or palette action)
  `wiki: generate` that spawns a subagent (you have `mew-subagents` and the
  child-session machinery — `parent_session_id`/`depth` are already on `Meta`)
  with a fixed prompt: map directory responsibilities, build/test entry
  points, config conventions. Restrict its tools to read/glob/grep + one
  write to the output path.
- **Storage**: `.mew/wiki.md` in the workspace (alongside where skills live).
  Add a staleness hint: record the git HEAD sha at generation time in
  frontmatter; the UI shows "regenerate?" when HEAD moved.
- **Consumption**, two-fold:
  1. UI: entry at the top of the file tree, renders in the preview pane.
  2. Agent: `mew-context::ContextResolver` optionally injects `.mew/wiki.md`
     (or a summary of it) into system context, config-gated — this is where
     the feature pays for itself, not just the human-readable doc.

---

## Phase 6 (optional / later) — terminal, browser context, remote

- **Terminal panel**: PTY over the existing WS (new message pair
  `TermInput`/`TermOutput` + `portable-pty` daemon-side, xterm.js in the web
  UI). Meaningful chunk of work; the permission story needs thought since it
  bypasses the tool sensitivity system entirely — probably gate on the same
  tier as unrestricted bash.
- **Browser element context**: an element-picker bookmarklet/extension that
  posts `{ title, url, selector, text, html_summary }` to the web UI, which
  wraps it as a structured attachment. Frontend-only; no daemon changes. Low
  priority unless you do frontend work in mew often.
- **SSH remote workspace**: don't build zcode's SSH client. mew's
  architecture already covers this — run `mew-daemon` on the remote box and
  point the web bridge / TUI at it. Ship docs + a `mew connect ssh://` helper
  that tunnels the unix socket (`ssh -L`) instead. Same outcome, ~1% of the
  code.

---

## Sequencing & effort sketch

| Phase | Depends on | Rough size |
|---|---|---|
| 1. session metadata (cwd, status, diff stats) | — | M — protocol + Meta + agent stat accumulation + rail rendering |
| 4. command palette | — (better after 1) | S — frontend only |
| 2. groups / views / archive | 1 | M — sidecar store + protocol + heavy rail UI (dnd) |
| 3. file tree + git + add-to-chat | 1 | L — new daemon surface, watcher, security review |
| 5. repo wiki | subagents (have) | S/M — mostly prompt + plumbing |
| 6. terminal / browser ctx / ssh docs | 3-ish | L / S / S |

Suggested order: **1 → 4 → 2 → 3 → 5**, with 6 opportunistic.

## Cross-cutting notes

- **Protocol discipline**: every addition above is a new variant or an
  optional field — keep serde defaults so TUI ↔ daemon ↔ web version skew
  stays survivable. You already do this well in `SessionInfo`.
- **TUI parity**: phases 1–2 map cleanly onto the TUI session picker (status
  dots → glyphs, groups → sections). Phase 3's tree could reuse the same
  `ListDir`/`GitStatus` messages in a ratatui tree widget later — a reason to
  put the file service behind protocol messages rather than a web-bridge-only
  HTTP endpoint.
- **Testing**: protocol round-trip tests exist as a pattern
  (`mew-protocol` test module) — extend for every new variant. For diff stats,
  golden tests on the hashline patcher's line-op → delta mapping. For the file
  service, path-escape tests (symlinks out of workspace, `..` traversal) are
  the important ones.
- **Config**: new knobs under `[sessions]` (auto_archive_days, pinned) and
  `[workspace]` (watcher on/off, preview max bytes, wiki injection).
