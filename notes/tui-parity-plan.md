# TUI Daemon-Parity Implementation Plan

> Produced 2026-07-05 from a three-way frontend parity audit (see CURRENT.md entry of the same date).
> Companion plans: `web-parity-plan.md`, `ios-parity-plan.md`. This plan needs **no protocol changes**;
> its foundation is a TUI/daemon-client transport fix (Item 0).

## Research findings (verify/correct the audit first)

Key facts established by reading the code — an implementer should not need to re-derive these:

**The central architectural gap the audit missed.** The TUI never sees a `ServerMessage`. `mew-daemon/src/client.rs::DaemonClient` translates `ServerMessage → AgentEvent` and pipes them through a **prompt-scoped** channel: `event_tx` is only set inside `prompt()` (client.rs:179) and is `None` between turns. Worse, `translate_server_message` (client.rs:490-521) **explicitly drops** every session-management message — `SessionList`, `SessionHistory`, `SessionAlert`, `SessionAttentionChanged`, `SessionTitleChanged`, `SessionSummaryChanged`, `SessionStatsChanged`, `FlaggedFilesChanged`, `SessionMetaChanged`, `GroupList/GroupsChanged`, `ProjectList`, `SessionUsageChanged`, `ModelList` — all map to an empty `Vec`. So **almost every feature in the audit is blocked on a missing transport**, not just missing UI. This is Item 0 below and must land first.

**Audit corrections:**
- **Item 4 is partly wrong.** `/resume <id>` *is* wired in daemon mode — `chat_with_daemon` (main.rs:2195-2206) calls `client.attach_session()`. The real bug is that the daemon replies with `SessionHistory`, which the client **drops** (client.rs:493), so attaching produces a blank chat with no replay. The fix is rendering `SessionHistory`, not adding `/resume`.
- **Item 8 is partly present.** `AgentEvent::FileDelta` and `AgentEvent::FlaggedFilesChanged` already exist and already reach `App::handle_agent_event` (app.rs:2438-2444) — but are deliberate no-ops. And their wire forms (`SessionStatsChanged`, `FlaggedFilesChanged`) are in the dropped set, so in daemon mode they never even become `AgentEvent`s.
- **Item 6 is unblocked and mode-independent.** `ToolCallPart` already carries `sensitivity: Option<String>` ("readonly"/"mutating"/"dangerous") in the display store (mew-message/src/lib.rs:173). Batching is a pure `chat.rs` rendering change that works identically in local and daemon mode.
- **Items 1/2/3/5/7** are genuinely absent as the audit states.

**No protocol changes are required for any item.** Every `ClientMessage`/`ServerMessage` needed already exists in `mew-protocol/src/lib.rs`. The only cross-crate change is adding `mew-protocol` as a dependency of `mew-tui` (today it isn't — mew-tui deps are mew-message/agent/provider/session/hooks), or defining thin mirror structs. Recommendation: add the dep; these are plain serde data types.

**The daemon-vs-local split risk.** `run_tui` (local, main.rs:2522) and `chat_with_daemon` (daemon, main.rs:2073) are two separate large event loops with duplicated `match event {…}` bodies. All shared UI lives in `mew-tui`. Therefore every new daemon-only surface must be **gated behind a new `app.daemon_mode: bool`** (App has no such flag today) so local mode hides it, and any new `App` state must be initialized in both loops. New daemon commands are sent from `chat_with_daemon` only; `handle_slash_result_local` (main.rs:2263) is where daemon-mode slash fallbacks live.

**Reusable patterns confirmed:**
- Collapsible **sidebar sections** with click-to-collapse: `ui/sidebar.rs` (Context/Todos/Tools/MCP), driven by `app.sidebar_collapsed: HashMap<String,bool>` and `sidebar_header_rows`.
- **Pickers/overlays**: `PickerState`/`PickerItem` (app.rs:322-341), opened via `open_model_picker`/`open_permission_mode_picker`, rendered by `overlays::draw_picker`, with select→`Action` dispatch keyed by `PickerState.kind` in events.rs:918-952.
- **Toasts**: `app.set_alert()` pushes to `app.toasts` (3s TTL, stacks bottom-right) — exactly the surface for cross-session alerts.
- **Status pills**: `ui/status.rs::build_pills` — bracketed colored badges; "Future pills slot in here" comment at line 133.
- **Slash commands**: `handle_slash` returns `SlashResult`; new variants flow through `run_tui` (local) and `handle_slash_result_local` (daemon).

---

## Ordering (by dependency, then value)

```
Item 0  Daemon notification transport ............ FOUNDATION (blocks 1,2,3,7,8; fixes 4)   [L]
Item 6  Tool-call batching ....................... QUICK WIN, independent of everything     [M]
Item 4  SessionHistory replay .................... small once Item 0 lands                  [S]
Item 1  Live session rail + switcher ............. highest value; needs 0                   [L]
Item 3  Cross-session alerts/attention ........... cheap once 0+1 land                      [S]
Item 7  Auto-title/summary + title display ....... cheap once 0+1 land                      [S]
Item 5  Project picker + new-session-in-cwd ...... needs 0                                  [M]
Item 8  Change stats + flagged files ............. needs 0                                  [M]
Item 2  Archive/pin/groups ....................... needs 0+1                                [M]
```

---

## Item 0 — Daemon notification transport (FOUNDATION) — L

The one prerequisite. Give the TUI a persistent, always-on channel for session-management messages that bypasses the prompt-scoped `event_tx`.

**`crates/mew-daemon/src/client.rs`:**
- Add a persistent broadcast channel to `ClientState`: `notify_tx: mpsc::Sender<ServerMessage>` created at `connect()` time (never cleared). In the background reader (the `while let Some(msg) = ws_rx.next()` loop, ~line 88), after decoding, send the **raw `ServerMessage`** for the session-management variants onto `notify_tx` *in addition to / instead of* the `event_tx` translation. Concretely: keep the existing `translate_server_message` path for turn events; for the currently-dropped set (client.rs:490-521) plus `SessionHistory`, forward the raw `ServerMessage` to `notify_tx`. This decouples them from `event_tx` being set.
- Expose `pub fn take_notifications(&self) -> mpsc::Receiver<ServerMessage>` (or hand back the `Receiver` from `connect`).
- Add thin request methods (each just encodes a `ClientMessage` and sends via `ws_out`, mirroring `slash_command()`): `list_sessions()`, `list_projects()`, `new_session_in(cwd)`, `archive_session(id, bool)`, `pin_session(id, bool)`, `set_auto_title(bool)`, `set_auto_summary(bool)`, `rename_session(id, title)`, `unflag_file(id, path)`. (`send_raw` already exists as a fallback, but typed methods keep `main.rs` clean.)

**`crates/mew-tui`** — add `mew-protocol` to `Cargo.toml` and new `App` state (app.rs), all `Default`-empty so local mode is unaffected:
- `pub daemon_mode: bool` (set true only in `chat_with_daemon`).
- `pub daemon_sessions: Vec<mew_protocol::SessionInfo>`
- `pub daemon_projects: Vec<mew_protocol::ProjectInfo>`
- `pub daemon_groups: Vec<mew_protocol::GroupInfo>`
- `pub session_titles: HashMap<String,String>`, `pub session_summaries: HashMap<String,String>`
- `pub session_attention: HashMap<String,(u32,u32)>` (perms, questions)
- `pub session_stats: HashMap<String, mew_session::ChangeStats>` (reuse mew-session type already in deps)
- `pub flagged_files: Vec<mew_protocol::FlaggedFileWire>`
- `pub auto_title: bool`, `pub auto_summary: bool`
- One reducer method `App::apply_daemon_notification(&mut self, msg: &ServerMessage)` that pattern-matches and updates the above (e.g. `SessionList → daemon_sessions`, `SessionTitleChanged → session_titles.insert`, `SessionAlert → self.set_alert(...)`, `SessionHistory → replace messages`). Keeping this in `App` (not `main.rs`) keeps `main.rs` thin and makes it unit-testable.

**`crates/mew/src/main.rs::chat_with_daemon`:**
- After connect, `let mut notify_rx = client.take_notifications();` and set `app.daemon_mode = true`.
- Change the event loop's `event_rx.recv().await` into a `tokio::select!` over `event_rx.recv()` and `notify_rx.recv()`. On a notification, call `app.apply_daemon_notification(&msg)`. (Handle `SessionHistory` here or in the reducer — see Item 4.)
- On startup, fire `client.list_sessions()` so the rail is populated immediately.

**Local mode:** untouched. `run_tui` never sets `daemon_mode`, never creates a notify channel; all new state stays empty; all new sidebar/picker surfaces are gated on `app.daemon_mode`.

**Risk/watch:** `App` gaining a `mew-protocol` dep is the only new coupling — acceptable (pure data). Do not add these as `AgentEvent` variants; that would leak daemon concepts into the local agent and force `mew-agent` changes.

---

## Item 6 — Tool-call batching — M — QUICK WIN (do in parallel with Item 0)

Independent of the daemon. Collapse consecutive `Part::ToolCall` entries with the same `sensitivity` into one summary row, matching web/iOS.

**`crates/mew-tui/src/ui/chat.rs`** (the `Part::ToolCall(tc)` arm at ~line 324): before rendering, group runs of adjacent tool-call parts sharing `tc.sensitivity`. Render a single collapsed summary line (e.g. `▸ 4 readonly tool calls (Read, Grep, Glob…)`) with the existing state glyph logic from `tool_call_label_and_color` (chat.rs:804), and expand to individual blocks when the group has ≤1 call, any call is `Running`/`Error`, or the user toggles it.
- Reuse the existing expand/collapse idiom already used for reasoning blocks (`reasoning_expanded: HashSet<PartId>`, `reasoning_header_rows`): add `tool_batch_expanded: HashSet<PartId>` keyed on the batch's first `PartId`, plus a `tool_batch_header_rows` for click-toggle in `events.rs` mouse handling.

**Local + daemon:** identical; both paths feed `Part::ToolCall` into the same `chat.rs`. No mode gating.

**Watch:** `chat.rs` is 1000 lines and does streaming/selection row bookkeeping (`chat_rows`, selection offsets). The grouping must keep `chat_rows` line accounting correct. Keep the default expanded for the active/streaming batch so streaming still appears incrementally.

---

## Item 4 — SessionHistory replay in daemon mode — S (needs Item 0)

`/resume <id>` already calls `attach_session`; only replay is missing.

- In Item 0's reducer, handle `ServerMessage::SessionHistory { messages }`: `app.clear_messages()`, push each `mew_message::Message`, set `app.status.session_id`, `auto_scroll = true`, `scroll = max_scroll` — mirroring the local `ResumeSession` replay in `run_tui` (main.rs:3178-3200).
- `chat_with_daemon` already handles `/resume <id>` (main.rs:2195). Once Item 1 lands, resume also becomes selecting a row in the session picker.
- Also translate the `AttachSession` path so the newly-attached session's `SessionReady`/title update `status`.

**Local mode:** unchanged (uses its own `mew_session::Reader::load` path).

---

## Item 1 — Live daemon session rail + in-app switching — L (needs Item 0)

Two surfaces, both reusing existing patterns.

**(a) Sidebar "Sessions" section** — `crates/mew-tui/src/ui/sidebar.rs`. Add a collapsible section (copy the Todos/Tools block structure exactly: arrow glyph, `sidebar_header_rows.push`, `sidebar_collapsed.get("sessions")`) rendered **only when `app.daemon_mode`**. For each `app.daemon_sessions` entry show: state glyph (running `▶` yellow / active `●` green / idle `○` gray from `SessionState`), title (`session_titles` or `SessionInfo.summary` or short id), an attention badge when `session_attention` shows pending perms/questions (reuse a colored `[!]`/`[?]` marker), and cost from `SessionInfo.usage.cost`. Mark the active session (`app.status.session_id`) with a `▸`/bold. Register header rows for click-collapse like the other sections.

**(b) Session switcher picker** — reuse `PickerState`. Add `App::open_session_picker()` building `PickerItem`s from `daemon_sessions` (id, `label = state glyph + title + cost`, `description = cwd + last-active`), with `kind = "session"`. Add a `/sessions` handling branch in `chat_with_daemon` that, in daemon mode, calls `client.list_sessions()` then `app.open_session_picker()` (instead of falling through to the local disk list). Dispatch: in `events.rs:918` add `else if kind == "session" { Some(Action::AttachSession(id)) }`; add `Action::AttachSession(String)` to the `Action` enum (events.rs:994); handle it in `chat_with_daemon` by calling `client.attach_session(&id)` (replay arrives via Item 4).

**Local mode:** `/sessions` keeps its current `build_sessions_list` disk behavior (the daemon branch is gated on `daemon_mode`); the sidebar section is hidden.

**Watch:** `Action::AttachSession` must be a no-op with an alert in `run_tui` (local) since there's no daemon — add the arm to keep the shared `Action` enum exhaustive.

---

## Item 3 — Cross-session alerts + attention — S (needs Item 0, pairs with Item 1)

Pure reducer + rail wiring, no new UI paradigm.

- `SessionAlert { title, kind, detail, session_id }` → `app.set_alert(...)` (existing toast queue). Color/prefix by `AlertKind` (PermissionNeeded/InputNeeded loud, TurnComplete quiet). If the alert is for a non-active session, prefix with the session title.
- `SessionAttentionChanged { pending_permissions, pending_questions }` → update `app.session_attention`; the Item 1 rail renders the badge. Also feed a global count into a **status pill** (`build_pills`, status.rs:133 "Future pills slot in here"): e.g. `[2 need you]` amber when any other session needs attention.

**Local mode:** no daemon → map never populated, no toasts, no pill. Hidden.

---

## Item 7 — Auto-title/summary toggle + title display — S (needs Item 0)

- `SessionTitleChanged`/`SessionSummaryChanged` → reducer updates `session_titles`/`session_summaries` (already added in Item 0). Consumed by the Item 1 rail and by the status line.
- Show the **active** session's title in `ui/status.rs` next to (or replacing) the raw `session_id`, and/or as a pill.
- Toggles: add `/autotitle` and `/autosummary` slash commands. In `handle_slash` (app.rs:1800) return `SlashResult::Message` for local mode ("not available"), but in `chat_with_daemon` intercept them (like `/web`, `/yield`) and call `client.set_auto_title(bool)` / `client.set_auto_summary(bool)`, toggling `app.auto_title`/`app.auto_summary` and emitting a toast. No `SlashResult` variant needed if handled directly in the daemon match arm — cleaner.

**Local mode:** titles are not generated by the daemon; status shows session_id as today. Toggles reply "daemon only" via the existing alert pattern.

---

## Item 5 — Project picker + start-session-in-cwd — M (needs Item 0)

- Add `/project` (or `/new`) slash command. In `chat_with_daemon`, intercept it (like `/web`): call `client.list_projects()`, then on the resulting `ProjectList` notification, `app.open_project_picker()` — a `PickerState` with `kind = "project"`, items from `daemon_projects` (`id = path`, `label = display_name`, `description = session_count + last_used`). Add a synthetic "＋ current directory" and "＋ enter path…" item.
- Dispatch: `events.rs` add `else if kind == "project" { Some(Action::NewSessionInProject(path)) }`; add `Action::NewSessionInProject(String)`; handle in `chat_with_daemon` by `client.new_session_in(path)` (sends `NewSession { cwd }`), then clear the chat for the fresh session.
- "Enter path…" reuses the picker filter text as a free-form cwd.

**Local mode:** local `run_tui` binds one cwd at launch; make `/project` reply "available in daemon mode only" and the `Action` arm a no-op-with-alert.

**Watch:** `NewSession { cwd }` is already what `DaemonClient::new_session()` sends (client.rs:142); factor a `new_session_in(cwd)` variant so the picker can pass an arbitrary path.

---

## Item 8 — Change stats + flagged files — M (needs Item 0)

- Reducer handles `SessionStatsChanged { added, removed, files_changed }` → `session_stats` per id; `FlaggedFilesChanged { files }` → `app.flagged_files`.
- New collapsible **"Changes" sidebar section** (same pattern as Item 1), daemon-gated: show `+added / −removed / N files` for the active session and a list of flagged files (path + reason). This is the TUI analog of web's Changes tab + pinned-context list.
- Optional: a `+X/−Y` diff pill in `build_pills`.
- Unflag: within the section, a keybinding / `/unflag <path>` → `client.unflag_file(id, path)`.

**Local mode:** `AgentEvent::FileDelta` and `AgentEvent::FlaggedFilesChanged` already arrive but are no-ops (app.rs:2438-2444). Optionally make the local handlers accumulate into `session_stats`/`flagged_files` too so the Changes section works locally without a daemon — low cost, and removes a daemon-only gate for this one section. Recommended.

---

## Item 2 — Archive / pin / groups — M (needs Items 0 + 1)

Least valuable, do last; builds entirely on the session rail.

- **Archive/pin**: actions on the Item 1 session picker/rail. Add key handling in the `kind == "session"` picker branch (e.g. `a` archive, `p` pin) → `Action::ArchiveSession(id,bool)` / `Action::PinSession(id,bool)` → `client.archive_session`/`pin_session`. Or slash `/archive <id>` / `/pin <id>`. `SessionInfo.archived/pinned` already flow via `SessionList`/`SessionMetaChanged` (handled in the Item 0 reducer). Render pinned with a `*` marker and hide archived by default with a filter toggle.
- **Groups**: `GroupList`/`GroupsChanged` → `daemon_groups` (Item 0). Render the Sessions section grouped by `SessionInfo.group_id` under group-name subheaders (reuse the divider + header line idiom). Group CRUD (`CreateGroup`/`AssignSessionGroup`) via slash commands or picker actions — defer the editor UI; start read-only grouping + assign-via-command.

**Local mode:** entirely hidden (no daemon sessions).

---

## Cross-cutting notes

- **No protocol changes.** All wire types exist. Only new code is: `mew-tui` gains a `mew-protocol` dep; `DaemonClient` gains a notification channel + typed request methods; `App` gains daemon-scoped state + a reducer; `events.rs` gains a few `Action` variants and picker `kind` branches; `sidebar.rs`/`status.rs`/`chat.rs` gain gated sections.
- **`Action` enum exhaustiveness** is the main footgun for the local/daemon split: every new `Action` (`AttachSession`, `NewSessionInProject`, `ArchiveSession`, `PinSession`) must be handled in **both** `run_tui` and `chat_with_daemon` match arms (no-op-with-alert in local).
- **Init parity:** new `App` fields are `Default`-empty, so `run_tui` needs no changes; only `chat_with_daemon` sets `daemon_mode = true` and wires the notify channel.
- **Testing:** the `App::apply_daemon_notification` reducer and `chat.rs` batching grouping are both unit-testable without a live daemon; add tests there. `mew-protocol` already round-trips every message.

## Critical files

- `crates/mew-daemon/src/client.rs`
- `crates/mew/src/main.rs`
- `crates/mew-tui/src/app.rs`
- `crates/mew-tui/src/events.rs`
- `crates/mew-tui/src/ui/sidebar.rs` (plus `ui/chat.rs` for Item 6, `ui/status.rs` for pills)
