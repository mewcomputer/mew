# Progress — 2026-06-29

## Polytoken inspirations: templating, skills, includes, web_fetch

Implemented P0 and P1 items from `docs/polytoken_inspirations.md`.

### Shared template context (`crates/mew-prompts/src/template.rs`)

New module with a `TemplateContext` struct and `render()` function shared
by personas, skills, and subagents. Variables:

- `supports_vision`, `persona_name`, `subagent_name`, `skill_name`
- `model_id`, `provider_id`, `model_variant` (thinking variant)
- `session_id`, `cwd`, `current_date`
- `tools`, `denied_tools` (effective tool list)
- `skills` (available skill names), `mcp_servers` (connected server names)
- `project_vars` (project-local variables from `.mew/project_vars.yaml`)

Template functions:

- `transclude("mew://path")` — inline built-in VFS resource
- `has_tool(name)` — check if a tool is available
- `has_skill(name)` — check if a skill is available
- `has_mcp(name)` — check if an MCP server is connected
- `is_model_variant(variant)` — check provider family ("anthropic",
  "openai", "deepseek", "z-ai", "umans", "opencode")

48 tests pass.

### Template skills (opt-in)

`mew-skills`: `template: bool` field on `Skill` and frontmatter. When
true, body is rendered through minijinja before returning from the skill
tool. Shared `template_ctx` Arc between agent and skill tool.

### Template subagents (opt-in)

`mew-subagents`: `template: bool` field on `SubagentDef` and frontmatter.
Runner renders body through minijinja with `subagent_name`, `model_id`,
`provider_id`, `session_id`, `cwd`, `current_date`, `tools`.

### `@file` static includes in AGENTS.md

`mew-context`: `expand_includes()` preprocessor. `@path/to/file` lines
inlined as literal text. Confined to file's subtree (`../` rejected).
5 tests.

### `polytoken:` frontmatter alias

`mew-personas`: `polytoken:` accepted as alias for `mew:`. `mew:` wins
when both present. 2 tests.

### Project variables file

`mew-context`: `load_project_vars(cwd)` loads `.mew/project_vars.yaml`
(or `.opencode/`, `.claude/`, `.agents/` prefixes). Walked cwd to git
root, first match wins. Returns `HashMap<String, String>` accessible as
`project_vars` in templates. 4 tests.

Agent gains `project_vars` field, loaded at all construction sites.
`apply_persona` includes skills list and project_vars in the template
context.

### `web_fetch` tool

`mew-tools/src/tools/web_fetch.rs`: fetches a URL via reqwest, converts
HTML to markdown via `htmd` crate. Content truncated at 128KB. ReadOnly
sensitivity. Registered in `build_tools()`.

### Blocked on model tier refactor

The model tier refactor (replacing `model_small`/`model_big` with
`micro`/`deci` on `ProviderConfig`, changing router config) is in
progress. Errors in `mew/src/main.rs` reference `router`, `turn_threshold`,
and mismatched types on `ProviderConfig`. My changes don't touch these
fields and should compile once the refactor lands.

Verified clean: `mew-prompts` (48 tests), `mew-context` (20 tests),
`mew-personas` (18 tests), all pass clippy `-D warnings`.

## Docs site expansion — fill out thin pages, add missing pages

**Problem**: The docs site was a sketch. Pages were accurate but skeletal:
no field references, no examples, missing pages for features users hit daily.

**New pages** (3):
- `site/src/content/docs/sessions.md` — session lifecycle, storage format,
  `/sessions` `/resume` `/rewind` `/clear` `/compact`. Verified against
  `mew-session` crate, `app.rs` slash handlers, and `main.rs` dispatch.
- `site/src/content/docs/permissions.md` — full permission mode reference,
  declarative rules, workspace sandboxing (agent layer + escape tier),
  secret file guard, secret redaction, auto classifier config. Verified
  against `mew-config/src/permissions.rs` and `lib.rs`.
- `site/src/content/docs/tools.md` — user-facing tool reference with every
  built-in tool, sensitivity, and description. Verified against `mew-tools`
  and `build_tools()` in `main.rs`.

**Expanded pages** (12):
- `installation.md` — cargo install, build from source, install recipes,
  subcommand reference, binary list.
- `quick-start.md` — TUI layout diagram with ASCII art, orientation for
  what each area does, cost review step.
- `configuration.md` — full field reference for every config section,
  env var table, state.toml fields, credential resolution order.
- `slash-commands.md` — added `/rewind` (no-arg variant), cross-references
  to Sessions and Permissions pages.
- `providers.md` — credential env var per provider, router decision logic,
  thinking variant meaning, catalog overrides.
- `permissions.md` — (new, see above)
- `personas.md` — planner/builder workflow, template variables
  (`supports_vision`, `persona_name`, `tools`, `denied_tools`), transclusion,
  worked template example. Verified against `mew-personas` and `mew-prompts`.
- `skills.md` — when to use a skill vs context file, how the agent decides
  to load, writing effective descriptions, frontmatter reference. Removed
  inaccurate "built-in skills" section (those were Polytoken-specific).
- `subagents.md` — what the parent sees on completion (`SubagentResult`
  variants), async vs blocking start, turn/time limits, session nesting.
- `plugins.md` — concrete Python plugin example with full JSON-RPC
  protocol, host function reference table, discovery paths, key rules.
- `mcp-servers.md` — common servers table, troubleshooting, qualified tool
  names, permission rules for MCP tools.
- `web-ui.md` — feature comparison table (TUI vs web), connection lifecycle,
  reconnection behavior.
- `context-files.md` — how context is used (per-turn injection), when to use
  skills instead, what to include.
- `tips-and-tricks.md` — prompting patterns section (scope, investigate
  first, subagents for research, planner/builder workflow), permission
  workflow guidance.

**Sidebar** (`astro.config.mjs`):
- Added Sessions to Getting Started.
- Added Permissions and Tools to Using mew.

**Build**: all 25 pages build cleanly (`pnpm astro build`). Pre-existing
TypeScript errors in `TerminalDemo.tsx` are unrelated to docs content.

## TUI UX improvements — terminal title, /thinking, Ctrl+P variant cycle — COMPLETE

**Terminal title** (`mew-tui/src/title.rs`):
- Sets terminal tab title to `mew — thinking…` while streaming, `mew` when idle.
- Uses xterm OSC 0 escape sequence on stderr. Restored to `mew` on exit.
- Called from both TUI main loops (daemon-client + local) after each draw.

**`/thinking` slash command**:
- `/thinking high`, `/thinking max`, `/thinking off` — resolves variant via
  catalog's `resolve_reasoning()` and calls `agent.set_reasoning()`.
- Added `SlashResult::SetThinkingVariant(String)` variant.
- Added `/thinking` to builtin slash command list (autofills in autocomplete).
- Handled in local TUI main loop, drain path, and daemon-client path.

**Thinking variant picker** (Ctrl+P command palette):
- Added "Thinking Variant" entry to the command palette.
- `open_thinking_variant_picker()` shows Off/high/max/thinking options.
- Enter applies the selected variant via `Action::SetThinkingVariant`.
- **Ctrl+P cycling**: when the thinking variant picker is open, pressing
  Ctrl+P cycles to the next variant and applies it immediately.
- Added `Action::SetThinkingVariant(String)` to the events Action enum.

**Esc-cancel verification**: already working correctly — first Esc shows
"esc again to stop agent" in status bar, second Esc cancels immediately,
2s timeout, cleared on stream end/error. No changes needed.

`just ci` fully green (fmt + clippy + all tests + web build + e2e).

---

## Thinking variant + session title — full protocol/daemon/TS wiring — COMPLETE

Wired thinking variant switching and session title broadcasting end-to-end
through the protocol, daemon, TS client, and web UI store.

**Protocol** (`mew-protocol`):
- `ClientMessage::SetThinkingVariant { variant: String }`
- `ServerMessage::ThinkingVariantChanged { variant: Option<String> }`
- `ServerMessage::SessionTitleChanged { session_id, title }`
- `ThinkingVariantInfo { name }` + `thinking_variants` field on `ModelInfo`
- 7 new roundtrip tests

**Daemon** (`mew-daemon`):
- `ThinkingSetter` type, `DaemonServer::with_thinking_setter()`
- `SetThinkingVariant` handler in `handle_connection`
- Client translation handles new `ServerMessage` variants

**main.rs**: lister populates `thinking_variants` from catalog; thinking
setter closure wired to `resolve_reasoning()` + `agent.set_reasoning()`.

**TS client** (`mew-web-client`): `ThinkingVariantInfo` type,
`setThinkingVariant()` method, `thinking-variant-changed` +
`session-title-changed` events.

**TS store** (`mew-web-ui`): `currentThinkingVariant`, `sessionTitles`
state + actions. Bridge wires both events. `ModelPill.tsx` and
`TitleStrip.tsx` now compile and work end-to-end.

---

## Daemon concurrency test fix — COMPLETE

Fixed 2 pre-existing test failures in `mew-daemon/tests/concurrency.rs`
(`prompt_during_in_flight_turn_is_serialized` and
`concurrent_prompts_on_same_connection_serialize`).

**Root cause:** The `TurnRotatingProvider` (test-only) pops a script from
its `Vec` on each `provider.stream()` call. But `spawn_title_generation`
in `lib.rs` also calls `provider.stream()` directly — consuming a script
meant for a test prompt. With 3 scripts for 3 prompts, the first prompt
consumed 2 (response + title generation), leaving only 1 for the second
prompt, and nothing for the third — causing a 5s timeout (empty script
= no events = no `MessageEnd` = hang).

**Fix:** When scripts are exhausted, `TurnRotatingProvider` now falls
back to `FakeProvider::text_response("(no script)")` instead of an empty
`Vec`, ensuring a valid event sequence with `MessageEnd` so the stream
always terminates cleanly.

All workspace tests now pass (was: 2 failures in concurrency.rs).

---

## CI + release workflows — COMPLETE

Added three GitHub Actions workflows:

- **`.github/workflows/ci.yml`** — Full CI gate on push/PR to main:
  - Rust CI matrix (macOS + Ubuntu): `cargo fmt --check`, `cargo clippy
    --all -- -D warnings`, `cargo test --all`.
  - Web CI: pnpm install, build `@mew/web-client` (tsc), typecheck +
    build `mew-web-ui`, run web-client tests.
- **`.github/workflows/release.yml`** — Tagged releases: on `v*` tag
  push, cross-compiles release binaries for `aarch64-apple-darwin`,
  `x86_64-apple-darwin`, and `x86_64-unknown-linux-gnu`, packages them
  as `.tar.gz`, creates a GitHub Release with auto-generated notes.
- **`.github/workflows/nightly.yml`** — Nightly builds: daily cron
  builds the same three targets from `main`, updates a rolling
  `nightly` pre-release tag.

---

## TUI chat — trailing separator after last message — COMPLETE

The empty separator line after every message was also added after the
last message, so scrolling to `max_scroll` showed a blank line at the
bottom instead of the actual last content. Fixed by checking `is_last`
(already computed for streaming detection) and skipping the separator
for the final message.

---

## TUI input wrapping — text truncated at right edge — COMPLETE

Same class of bug as the chat scrolling fix: the input renderer wrapped
text to `content_width` (the content area width after the 1-cell border
on each side), but each visual row has a 2-character prefix (`"> "` or
spinner) or indent (`"  "`). So text wrapped at `content_width` columns
but only `content_width - 2` columns were visible — the right 2
characters of each wrapped row were truncated and never shown.

**Fix:** Subtract the 2-char prefix/indent from the wrap width at every
call site that passes `content_width` to `input_visual_line_count`,
`cursor_visual_row_col`, `visual_to_byte_offset`, `cursor_visual_up`,
or `cursor_visual_down`. The wrap width is now
`area.width.saturating_sub(2).saturating_sub(2)` (border + prefix).

**Files changed:**
- `ui/input.rs` — `draw_input`: compute `text_width` from
  `content_width - 2`, use it for `input_visual_line_count`,
  `cursor_visual_row_col`, and `wrap_w`.
- `ui/mod.rs` — pre-layout height calculation: subtract 2 more for
  prefix.
- `ui/welcome.rs` — welcome screen input height: subtract 2 more.
- `events.rs` — mouse click-to-position, Up/Down visual line
  navigation: subtract 2 more from `content_width`.

Build, clippy, 130 tests pass.

---

# Progress — 2026-06-29

## omp.sh inspiration doc — COMPLETE

Researched `omp` (oh-my-pi / `omp.sh`) and produced a comprehensive
compare-and-contrast document cataloging improvements mew could adopt.

**Doc:** `docs/omp_inspirations.md`
- Executive summary of the three highest-leverage themes.
- Feature catalog organized by domain, each with:
  - What omp does
  - mew's current state
  - Pro-adoption and anti-adoption arguments
  - Verdict / effort assessment
- Prioritized shortlist (P0–P3) for roadmap planning.
- Appendix linking omp sources and relevant mew source files.

**Key themes identified:**
1. Move core tools in-process (grep, bash, file cache) instead of forking external binaries.
2. Improve the edit format to be model-friendly and stale-file safe.
3. Add durable cross-session memory and runtime rule/guardrail injection.

**Highest-priority quick wins flagged:** in-process `grep`, shell completions,
and safer `edit` anchoring. Debugger/LSP integrations were included but
down-ranked as heavier bets per your feedback.

---

# Progress — 2026-06-29

## Polytoken inspiration doc — COMPLETE

Researched Polytoken docs and compared them to mew's templates, personas,
skills, subagents, tools, and VFS. Noted that this repo already contains
`.polytoken/permissions.local.yaml`, so the team is already exploring
Polytoken.

**Doc:** `docs/polytoken_inspirations.md`
- Compare-and-contrast catalog organized by templates, facets/personas,
  skills, subagents, tools, project context, VFS, and project variables.
- Each entry includes pro-adoption and anti-adoption arguments.
- Prioritized shortlist (P0–P3) for roadmap planning.

**Key themes identified:**
1. Extend MiniJinja templating from personas to skills, subagents, and
   AGENTS.md (opt-in).
2. Expand template context variables and functions (`model_variant`,
   `has_tool`, `source_control`, `project_vars`).
3. Formalize the planner/builder workflow with plan-specific tools
   (`write_plan`, `edit_plan`, `handoff_plan`).
4. Add `web_fetch` as a smaller first step toward web capabilities.

**Highest-priority quick wins:** accept `polytoken:` frontmatter alias,
opt-in skill/subagent templating, expand persona template variables, and
project variables file.

---

# Progress — 2026-06-27

## B1 web UI polish — COMPLETE

Finished all five B1 polish items for `mew-web-ui`.

**Subagent visualization:**
- `SubagentPanel.tsx` — renders running/completed/cancelled/failed subagents with colored status dots, display name, and last progress message (`↳ message`).
- Store: `subagents: Map<string, SubagentInfo>` state, `onSubagentStart`/`onSubagentStatus`/`onSubagentEnd` actions. Bridge wires the three events.

**AskUserRequest rendering:**
- `AskUserCard.tsx` — renders pending questions with selectable options or free-text input. Submit button sends answers back via `respondToAskUser`.
- Store: `pendingAskUser: PendingAskUser[]` state, `onAskUserRequest`/`resolveAskUser` actions. `askUserResponders` side-channel map (like `permissionResponders`).
- TS client fix: `ask-user-request` event now includes a `respond` callback (matching the `permission-request` pattern). Added `respondToAskUser()` method.

**Todo list rendering:**
- `TodoPanel.tsx` — renders the agent's todo list with status icons (✅🔄⬜⛔), dependency labels, and progress count (done/total + active).
- Store: `todos: TodoItem[]` state, `onTodosUpdated` action that maps wire `Todo[]` → `TodoItem[]`. Bridge wires `todos-updated`.

**Reconnect with backoff:**
- `App.tsx` — exponential backoff reconnection (2s, 4s, 8s, ... cap 30s). On unexpected WS close, retries automatically and re-attaches to the same session via `attachSession`. `intentionalDisconnectRef` prevents retries during unmount. `reconnectTimerRef` is cleared on cleanup.
- TS client fix: `close` event handler now clears `ws` and `openPromise` so `connect()` can be called again after a disconnect.

**Playwright e2e:**
- `playwright.config.ts` + `e2e/chat.spec.ts` — 3 browser-level tests: page loads, send prompt + see streaming text, session list drawer opens. Spawns `mew daemon --fake-provider` + `mew-web` bridge as subprocesses, uses a temp dir for the socket. All 3 pass.

All wired into `App.tsx` layout: `ChatSurface → TodoPanel → SubagentPanel → AskUserCard → InputArea`.

Verified: `pnpm exec tsc --noEmit` clean, `pnpm build` succeeds, `npx playwright test` 3/3 pass.

## Daemonization (A1) — COMPLETE

Implemented `mew daemon --background` using the standard double-fork + setsid daemonization pattern. The daemon can now detach from the terminal and survive logout.

**What changed:**

- **`main.rs` restructured**: Split into a sync `main()` (parses CLI, handles `--stop`/`--background` before tokio starts) and `async_main(cli, daemonized)` (the runtime body). This was critical: `dup2` calls in `daemonize()` must run *before* tokio creates its internal FDs, or the runtime panics with "Bad file descriptor".
- **`daemonize()` function**: Redirects stdio to `/dev/null` (or `--log` file) *before* forking, then double-forks (first fork detaches from parent, `setsid()` creates a new session, second fork ensures the child can never re-acquire a terminal). Writes PID to pidfile. Re-inits tracing to write to the redirected stderr.
- **`--background` flag**: Detaches the daemon from the terminal.
- **`--log <path>` flag**: Redirects logs to a file instead of `/dev/null`.
- **`--pidfile <path>` flag**: Overrides the default PID file path (`$XDG_RUNTIME_DIR/mew.pid` or `/tmp/mew.pid`).
- **`--stop` flag**: Reads the PID from the pidfile and sends SIGTERM. Removes the pidfile after.
- **SIGTERM handling in `DaemonServer`**: Both `run()` and `run_tcp()` now use `tokio::select!` with a SIGTERM handler, so the daemon shuts down gracefully on `--stop` or `kill`.
- **Tracing init dedup**: `daemonize()` inits tracing for the daemon child; `async_main` skips re-init when `daemonized == true` (avoids "global default trace dispatcher already set" panic).
- **`nix` crate**: Added as a workspace + `mew` dependency for `fork`, `setsid`, `dup2`, `kill`.

**Verified live:**
```
$ mew daemon --fake-provider --socket /tmp/mew.sock --pidfile /tmp/mew.pid --log /tmp/mew.log --background
$ # daemon runs in background, survives terminal close
$ mew daemon --stop --pidfile /tmp/mew.pid
daemon (PID 12345) stopped
```

fmt clean, clippy clean, all 717 tests pass.

## Shared sessions Phase 1 — COMPLETE

Finished implementing live shared sessions per `SHARED_SESSIONS_SPEC.md` Phase 1. The workspace now builds clean, all 717 tests pass, and `just ci` is green.

**What was done this session (resume from mid-implementation):**

- **`main.rs` builder closure**: Updated both the fake-provider and real-provider builder closures to accept `AgentBuildParams { session_id, writer, cwd }`. The `build_session_agent` helper now takes `writer` and `session_id` params and threads them into `Agent::new`. The closures parse the `sess_<ULID>` string into a `SessionId` (Ulid). The `mew` binary builds clean.
- **`DaemonServer::with_session_dir`**: New constructor that lets tests isolate sessions to a temp dir instead of the global `~/.local/share/mew/sessions`. `with_model_management` now preserves the custom session dir instead of resetting to the global default. `SessionManager::create` now uses `Writer::open_at(&self.session_dir, …)` instead of `Writer::open(…)` so it respects the configured dir.
- **Daemon tests updated** (`e2e.rs`, `concurrency.rs`, `tcp.rs`): All builder closures updated to the `AgentBuildParams` signature and use `DaemonServer::with_session_dir` with a temp `sessions/` subdir. Session IDs are parsed to `SessionId` for the `Agent::new` call.
- **Web-bridge e2e tests updated**: Two builder closures fixed to the new tuple-return shape + `with_session_dir`; `SessionReady` patterns use `..` for the new optional `model`/`provider` fields.
- **Unused imports cleaned**: Removed `HashMap`, `PermissionDecision`, `oneshot`, `Mutex` from `mew-daemon/src/lib.rs`; removed unused `anyhow::Result` from `tcp.rs`.
- **Protocol round-trip tests** (+10 new, 47 → 57): `AttachSession`, `ListSessions`, `SessionState` (lowercase serialization), `SessionInfo` (full + optional-field-skip), `RequestResolved`, `SessionCleared`, `SessionList` (multi-entry), `SessionHistory` (with a real `Message` + empty list). Added `PartialEq, Eq` derives to `SessionState` for `assert_eq!` support.
- **Web UI session list**: New `SessionListDrawer` component (slide-in drawer with backdrop, Escape-to-close, new-session button, attach-to-session). `TopBar` now has a sessions button showing the current session ID. `App.tsx` wires the drawer, persists `sessionId` to `localStorage`, and reconnects via `attachSession` on reload (falling back to `newSession` if the session is gone).
- **Store updates**: `availableSessions`, `sessionsLoading` state; `onSessionHistory` (maps wire `Message[]` → `ChatMessage[]` via `wirePartToMessagePart`), `onSessionCleared` (wipes messages + counters), `setAvailableSessions`, `setSessionsLoading`. Bridge wires `session-list`, `session-history`, `session-cleared`, and `request-resolved` events.
- **Cancellation test fix**: `test_cancellation_during_stream` in `mew-agent/src/tests.rs` was failing because `run()` now creates a fresh per-turn `CancellationToken` (the shared-sessions change), so cancelling the agent's permanent token didn't reach the turn. Fixed by calling `run_with_parts` with `Some(agent.cancel_token.clone())`.
- **Drive-by clippy fixes**: `unwrap_or_else(CancellationToken::new)` → `unwrap_or_default()` in `agent.rs`; `#[allow(clippy::too_many_arguments)]` on `build_session_agent` (now 8 args).

**Verification:** `cargo fmt --check` clean, `cargo clippy --all -- -D warnings` clean, `cargo test --all` — 717 tests pass, `just ci` green (including `just e2e` subprocess test).

## Shared sessions Phase 1 — IN PROGRESS (stopped mid-implementation)

Started implementing live shared sessions per `SHARED_SESSIONS_SPEC.md` Phase 1. Work is partially complete; the daemon lib compiles but the full workspace does not yet build because `main.rs`'s builder closure and the daemon tests still use the old signature.

**Done:**

- **Protocol** (`crates/mew-protocol/src/lib.rs`): added `ClientMessage::AttachSession`, `ClientMessage::ListSessions`, `ServerMessage::SessionList`, `ServerMessage::SessionHistory`, `ServerMessage::RequestResolved`, `ServerMessage::SessionCleared`, plus `SessionInfo` / `SessionState` structs. Compiles clean.
- **TS client** (`mew-web-client/src/index.ts`): mirrored all new wire types (`Message`, `Role`, `Finish`, `Tokens`, `AssistantMeta`, `Time`, `SessionInfo`, `SessionState`), added `attachSession()` / `listSessions()` methods, new events (`session-list`, `session-history`, `request-resolved`, `session-cleared`), and dispatch cases. `tsc --noEmit` clean.
- **Agent API** (`crates/mew-agent/src/agent.rs`): `run_with_parts` now takes `Option<CancellationToken>`; the per-turn clone assigns it so one `Cancel` no longer poisons all future turns. Added `session_meta()` accessor. Updated the two TUI callers in `main.rs` to pass `None`. `mew-agent` compiles clean.
- **Daemon session module** (`crates/mew-daemon/src/session.rs`, new): `SessionManager` (create/attach/list/remove) and `Session` (broadcast via per-client `mpsc::UnboundedSender`, turn serialization, per-turn cancel, pending-request maps, detach-by-id). Resume-from-disk with `Meta::depth != 0` filtering and per-session loading lock. Added `mew-session` + `tokio-util` deps.
- **Daemon connection handler** (`crates/mew-daemon/src/lib.rs`): rewrote `handle_connection` to use `SessionManager`, spawn a per-connection writer task, broadcast events, route permission/ask-user responses from any client, cancel+drain on last-client disconnect. `mew-daemon` lib compiles (with 3 unused-import warnings to clean up). Updated `DaemonClient`'s `translate_server_message` catch-all for the new variants.

**Not done (resume points):**

- `crates/mew/src/main.rs` builder closure at ~L1171/L1195 still has the old `move || { ... }` signature; needs to accept `AgentBuildParams { session_id, writer, cwd }` and use the supplied `Writer` instead of calling `Agent::new(..., None, ...)` with no session. `build_session_agent` needs to thread the writer through. Until this is fixed the workspace won't build.
- `crates/mew-daemon/tests/` (e2e.rs, concurrency.rs) construct `DaemonServer`/builder with the old no-arg closure; they'll need updating to the new `AgentBuildParams` signature and a fake provider path.
- Clean up unused imports in `mew-daemon/src/lib.rs` (`HashMap`, `PermissionDecision`, `oneshot`, `Mutex`).
- Add protocol round-trip tests for the new variants (pattern matches existing tests in `crates/mew-protocol/src/lib.rs`).
- Web UI: session list drawer + attach button + render `SessionHistory` / handle `SessionCleared`. Store updates for the new events.
- Run `just ci` green.

`todo #69` is set back to `pending` so the next session can pick it up.

## Shared sessions spec — reviewed and ready

Wrote and twice reviewed `SHARED_SESSIONS_SPEC.md` for live shared sessions across daemon clients.

First reviewer pass found critical/high issues around the broadcast mechanism (`SplitSink` cannot be shared), sync/async `AgentBuilder` mismatch, permanent agent-level `cancel_token`, disconnect-during-turn deadlock on pending oneshots, missing `RequestResolved`, `SwitchModel` scoping, `AttachSession` TOCTOU race, and a few smaller gaps.

Revised the spec to address all of the above and ran a second reviewer pass. Remaining medium finding (per-turn cancellation must cascade to subagents/shell jobs) was fixed by specifying that `start_subagent` / `start_shell_job` accept a parent `CancellationToken`. Minor cleanups (`SessionState` enum, `SessionHistory` sent only on disk resume, corrupt meta handling, `SessionCleared` broadcast) were also incorporated.

Key final design:

- Daemon owns sessions via `SessionManager`; connections attach to sessions.
- `Session` broadcasts via per-client `mpsc::UnboundedSender<ServerMessage>`.
- One turn at a time; fresh `CancellationToken` per turn.
- Permission/ask-user requests go to all clients; any client can respond; `RequestResolved` dismisses modals everywhere.
- Idle sessions resume from disk via `Agent::load_messages`.
- `SessionCleared` keeps `/clear` consistent across clients.

Spec is approved for Phase 1 implementation.

## Web UI plan update — session branching preserved for later

Updated `WEB_UI_PLAN.md` per feedback: moved **session branching / threading** and **multi-model comparison** out of Phase 4 into a new **"Preserved for future exploration"** section. They stay tracked but are not assigned to a phase now. Renumbered Phase 4 accordingly (4.1 session sharing, 4.2 command palette, 4.3 PWA + mobile, 4.4 per-hunk change review). No implementation changes; planning only.

## Chat surface polish — complete pass

Finished Phase 1 chat surface polish for `mew-web-ui`:

- **Input shortcuts** (`InputArea.tsx`): changed send shortcut from plain `Enter` to `Cmd/Ctrl + Enter`. Plain `Enter` now inserts a newline. Updated placeholder and send-button tooltip.
- **Shiki syntax highlighting**:
  - Added `shiki` + `@shikijs/langs`.
  - Created `src/lib/shiki.ts` with lazy highlighter using `shiki/core`, `engine/oniguruma`, 14 common languages, and `github-dark`/`github-light` themes.
  - Created `src/components/CodeBlock.tsx` and `src/components/MarkdownBody.tsx`. Streaming text renders plain markdown; finalized text gets highlighted code blocks.
  - Updated `MessageItem.tsx` to use `MarkdownBody`.
- **Shared `CopyButton` + message copy**: extracted `CopyButton.tsx`, added a hover-to-reveal copy button on assistant messages (copies all text parts).
- **Tool output inline** (`ToolCallCard.tsx`): removed the need to expand a tool card to see output. A compact output panel is visible once the tool has output; full input/output still available via expand.
- **Error boundaries**: added `ErrorBoundary.tsx`, wrapped the whole app in `App.tsx`, and wrapped each `MessageItem` in `ChatSurface.tsx` so a single bad message can't crash the app.
- **Light/dark theme support**:
  - Created `src/lib/theme.tsx` with `ThemeProvider` and `useTheme`. Supports `light`/`dark`/`system`, persisted to `localStorage`, reacts to `prefers-color-scheme` changes.
  - Wrapped app in `ThemeProvider` (`main.tsx`).
  - Added `ThemeToggle` to `TopBar`.
  - Updated `CodeBlock` to use the resolved theme and removed the hardcoded `#0d1117` background.
- Verified: `pnpm exec tsc --noEmit` clean, `pnpm build` succeeds.

## Catalog parser + model picker fixes

Two issues fixed after initial model picker implementation:

- **Catalog parser** (`mew-catalog`): The models.dev API returns a nested
  format `{ "provider_id": { "models": { "model_id": {...} } } }` but the
  parser only handled `{"models": [...]}` and `[...]` formats. The first
  parser path was silently returning 0 models (serde default on missing
  `models` field). Added a third parser path that:
  - Detects the nested format by checking for `models` sub-objects
  - Maps models.dev field names to our `Model` struct (`limit.context` →
    `context_window`, `limit.output` → `max_output`, `modalities.input`
    contains "image" → `vision`, `cost` → `pricing`)
  - Sets `provider` from the top-level key
  - Result: **2674 models** parsed (was 0)

- **Model lister filter** (`main.rs`): Now only shows models from providers
  with credentials (`provider_has_credential`). Previously showed all
  configured providers' models even without credentials.

- **Typography plugin** (`mew-web-ui`): Installed `@tailwindcss/typography`
  and added to tailwind config. The `prose` class was referenced in
  `MessageItem.tsx` but the plugin wasn't installed — markdown rendered
  with default browser styles (dark text on dark background). CSS bundle
  grew from 14KB to 34KB confirming the plugin is included.

- **Empty-object parse fix**: `parse(b"{}")` now returns an empty catalog
  instead of falling through to the array parser (which would error).

Verified: 12 models in picker (7 opencode-go + 5 umans), clippy clean,
33 catalog tests pass.

---

## Model picker (end-to-end: protocol → daemon → TS client → UI)

Added model switching to the web UI. Four layers changed:

- **Protocol** (`mew-protocol`): New `ClientMessage::ListModels` and
  `SwitchModel { provider, model }`. New `ServerMessage::ModelList { models }`
  and `ModelSwitched { provider, model }`. New `ModelInfo` struct
  (id, provider, model, description).
- **Daemon** (`mew-daemon`): `DaemonServer` gains optional `model_switcher`
  and `model_lister` closures. `with_model_management()` builder method.
  Connection handler dispatches `ListModels` (calls lister) and `SwitchModel`
  (calls switcher, rebuilds provider on the agent, updates vision/context/
  max_output from catalog). `DaemonClient::translate_server_message` handles
  the new server messages (returns empty Vec — not AgentEvents).
- **main.rs**: Wires the switcher/lister for non-fake-provider daemons.
  Lister reads from the catalog (sync, no network). Switcher calls
  `build_provider` + updates agent metadata.
- **TS client** (`mew-web-client`): New `ModelInfo` type, `listModels()` and
  `switchModel()` methods. New `model-list` and `model-switched` events.
- **Store** (`mew-web-ui`): `availableModels`, `currentModel`,
  `currentProvider` state. Bridge wires `model-list` → `setAvailableModels`
  and `model-switched` → `setCurrentModel`.
- **UI**: `ModelPicker` component — searchable dropdown grouped by provider,
  shows active model with checkmark, fetches on connect. Sits in the TopBar.

Verified: daemon returns 5 models from catalog, ListModels round-trips
through WS, clippy clean, all 47 protocol + 5 daemon tests pass.

---

## Bridge serves React app from `mew-web-ui/dist/`

The `mew-web-bridge` binary now serves the built React app instead of the
old vanilla-JS prototype. Changes:

- **`include_dir` embedding**: `mew-web-ui/dist/` is embedded at compile
  time via `include_dir!("$CARGO_MANIFEST_DIR/../../mew-web-ui/dist")`.
  Vite hashes asset filenames (`index-BNDoaku_.js`), so files are served
  dynamically by path lookup rather than hardcoded `include_bytes!`.
- **MIME type mapping**: full extension table (JS, CSS, SVG, fonts, images,
  source maps). Correct `Content-Type` headers verified via curl.
- **SPA fallback**: unknown GET paths serve `index.html` so TanStack Router
  client-side routing works.
- **Removed old assets**: `INDEX_HTML`, `MAIN_JS`, `STYLE_CSS` constants and
  the `_unused_message_import` function deleted. The old vanilla JS files
  were already moved to `mew-web-ui-legacy/`.
- **Justfile updates**: `build-web` now runs `pnpm --filter mew-web-ui build`.
  New recipes: `dev-ui` (Vite dev server with HMR) and `clean-web` (purge
  dist + Vite cache).

Verified end-to-end: `GET /` serves index.html, `GET /assets/*.js` and
`*.css` serve with correct MIME types, SPA fallback works, e2e test passes,
clippy clean.

---

# Progress — 2026-06-25

## UX fixes: tool error messages + bash output (committed)

- **Bash partial output on timeout**: timeout now returns `Ok(ToolOutput)` with partial stdout/stderr + `"timeout after Ns (partial output shown)"` instead of opaque `Err("timeout")`. The model can see what the command produced before it was killed.
- **Bash stdout/stderr separation**: stdout and stderr collected separately. Final output shows stdout first, then `--- stderr ---` separator, then stderr lines. The model can distinguish error output from normal output.
- **Edit error context**: `"old_string not found"` now includes file path, line count, first/last line snippets, and a recovery hint. Ambiguous match includes path + "include more context" hint.
- **Read error messages**: all errors now include the file path (stat, read, too-large, binary) instead of raw io::Error strings.
- **Permission denied reason**: tracked `deny_reason` at every Deny source in the agent tool loop. Error now says `"permission denied: deny rule or workspace-escape tier"`, `"permission denied: user denied"`, `"permission denied: classifier denied"`, etc. instead of bare `"permission denied"`.

## Daemon architecture: minimal vertical slice (in progress)

Started the daemon split — separating the agent loop into a standalone server process that frontends (TUI, Discord bot, iOS app) connect to over WebSocket.

### Created crates

- **`mew-protocol`** (`crates/mew-protocol/`): Wire message types for daemon↔frontend communication. `ClientMessage` (NewSession, Prompt, Cancel, PermissionResponse, AskUserResponse, SlashCommand) and `ServerMessage` (SessionReady, Provider, ToolStart/End, PartUpdated, PermissionRequest, AskUserRequest, Subagent events, TodosUpdated, SlashResult, etc.). Tagged enums with `serde(tag = "type")`. JSON codec helpers. Conversion functions from `ProviderEvent` and `SubagentOutcome` to wire types.

- **`mew-daemon`** (`crates/mew-daemon/`): WebSocket server over Unix socket. `DaemonServer::new(builder)` takes an agent-builder closure (so main.rs owns the agent setup). Accept loop spawns a task per connection. Each connection creates a session, handles client messages, streams `ServerMessage`s. Channel-bearing `AgentEvent` variants (PermissionRequest, AskUser, etc.) translated to ID-paired wire requests with `oneshot::Sender` stashed in a pending-requests map.

- **`mew-message`**: Added `ProviderEventWire` — serializable mirror of `ProviderEvent` using `String` instead of `&'static str` for the `field` parameter.

### main.rs changes

- Added `Daemon` subcommand: `mew daemon [--socket <path>] [--provider <id>] [--model <id>] [--raw] [-P/-A/--auto-plus/-D]`. Default socket: `$XDG_RUNTIME_DIR/mew.sock` or `/tmp/mew.sock`.
- Extracted `build_session_agent()` helper from `run_acp_server` — shared between ACP server and daemon. Builds full agent (provider, tools, MCP, personas, skills, subagents, context files, pricing, permission engine).
- `run_daemon()` — loads config + catalog, wraps `build_session_agent` in a closure, starts `DaemonServer`.

### Decisions

- **Wire format**: JSON over WebSocket text frames (MessagePack as binary frames later — same schema, different codec).
- **ACP**: Will be dropped entirely. `mew-acp` and `mew-acp-iroh` crates still exist but will be removed in a follow-up.
- **First slice scope**: `mew daemon` → Unix socket → one session → stream AgentEvents. Multi-session, TUI client mode, and ACP removal come next.

### Next steps

1. Add TUI daemon client mode (`mew chat --connect ws://unix:/tmp/mew.sock`) — WebSocket client that translates ServerMessages back to AgentEvents.
2. Remove ACP crates.
3. Session registry for multi-session support.
4. MessagePack codec.

---

# Progress — 2026-06-26

## ACP removed

Step 5 of `DAEMON_PLAN.md` is done. `mew-acp` and `mew-acp-iroh` are gone, and ACP is no longer accessible from the binary.

### What changed

- **Crates deleted**: `crates/mew-acp/` and `crates/mew-acp-iroh/`.
- **Workspace `Cargo.toml`**: dropped both crates from `members`, removed their path deps, and removed `iroh` / `opaque-ke` / `chacha20poly1305` (kept `rand` — the TUI spinner still uses it).
- **`crates/mew/Cargo.toml`**: removed `mew-acp` dep.
- **`crates/mew-message/src/lib.rs`**: removed `ErrorKind::AcpProtocol` (and it from the roundtrip test loop).
- **`crates/mew/src/main.rs`**:
  - Removed the `Acp { ... }` Commands variant and its `Some(Commands::Acp { ... })` match arm.
  - Removed the `--acp-agent` flag on `Chat` and the `chat_with_acp(...)` dispatch branch.
  - Deleted `chat_with_acp` and `run_acp_server` (~280 lines), replaced with a tombstone comment.
  - Updated the `build_session_agent` doc comment — it's now documented as used by `run_daemon` only (the TUI's `--connect` mode goes through the daemon side).
- **Docs**: `CLAUDE.md` (architecture + ACP section), `POLYTOKEN_PARITY.md` (deferred → in-progress status for daemon/TUI split), `DAEMON_PLAN.md` already said "dropped entirely" — no change needed there.

### Drive-by fixes (not ACP, but blocked the build)

The tree had two pre-existing bugs in the daemon-client mode that surfaced once ACP came out:

- `mew_tui::events::Action::Compact` doesn't exist — the existing `SlashCommand(text)` arm already handles `/compact`. Removed a redundant `Action::Compact` branch I'd added.
- `mew_tui::App::push_synthetic_message` was private but called from `handle_slash_result_local` in `main.rs`. Made it `pub`.

### Verification

- `cargo build -p mew` — clean.
- `cargo clippy -p mew -- -D warnings` — clean.
- `cargo test --all` — 294 tests passed, 0 failed.

### What's left in the daemon plan

- **Step 6 (e2e test)**: daemon up → TUI connects → prompt streams → permission prompt → cancel. Worth doing before declaring the slice shippable.
- **Follow-up slice**: session registry, MessagePack codec, TCP listener, config hot-reload.

---

# Progress — 2026-06-26 (later)

## Testing push — Tier 1, 2, 3 all shipped

Following up on the ACP removal: stood up a comprehensive test suite across three tiers. Test count went from **294 → 667** (+373 new tests). All passing, CI gate green.

### Tier 1 — Foundation

- **`mew-provider-fake`** (+13 tests): text_response/tool_call script shape, part_id consistency, streaming semantics, empty script, multi-byte text round-trip.
- **`mew-protocol`** (+43 tests): exhaustive roundtrip for every ClientMessage and ServerMessage variant, nested structures (SubagentStart/TodosUpdated/AskUserRequest), negative tests (malformed JSON, missing fields, wrong types, unknown tags), PermissionDecision ↔ hooks conversions, ProviderEvent ↔ wire converters.
- **`mew-daemon/tests/e2e.rs`** (+12 tests, new file): full daemon lifecycle over real Unix sockets — SessionReady, prompt streaming, tool-call finish, prompt without session, invalid JSON, /clear + /compact slash commands, cancel mid-stream, sequential prompts, concurrent connections, fresh-agent-per-connection.

### Tier 2 — Behavior

- **`mew-tools/tests/integration.rs`** (+8 tests, new file): composed tool scenarios — write→read round-trip, write→edit→bash cat cross-tool verification, glob→grep composition, bash nonzero exit surfaces code, glob no-match returns empty not error, read offset/limit pagination, grep extension filter, edit preserves filename for glob.
- **mew-agent escape-tier integration** (+2 tests): `cat /etc/passwd` against `workspace_roots = [tempdir]` escalates to `Prompt` even in Permissive mode; with empty roots and an Allow rule, no prompt fires. Covers the escape tier end-to-end through `agent.run()` rather than just unit.
- **mew-tools regression tests** (+4 tests): edit "not found" includes first/last line snippets + recovery hint; edit "ambiguous" suggests more context; read errors (missing + binary) include the file path. Pins down the UX fixes from 2026-06-25 so a refactor can't silently regress them.

### Tier 3 — Polish

- **`mew-daemon/tests/concurrency.rs`** (+6 tests, new file): 5 concurrent connections get distinct sessions, sequential prompts on one connection produce distinct part IDs (via custom `TurnRotatingProvider`), concurrent cross-connection prompts have disjoint part ID sets, prompt during in-flight turn serializes, rapid-fire Cancel doesn't crash, slash command during stream doesn't block.
- **`mew-session/tests/roundtrip.rs`** (+6 tests): JSONL write/load, empty session loads empty, meta persists (model + subagent_name), multi-session independence, unknown session errors, reopen appends without truncating.
- **`mew-personas/tests/discovery.rs`** (+8 tests): single + multi persona discovery, model pin + tool allowlist, tools_deny, markdown fence preservation, invalid name rejected, template flag parsed.
- **`mew-subagents/tests/loader.rs`** (+9 tests): user defs picked up, built-in defaults included unless overridden, user override replaces built-in, tool allowlist parses, empty dir yields built-ins, display-name pool ≥ 10 unique entries, deterministic per seed, distribution varies across seeds, all picked names come from the pool.

### Plumbing changes

- `mew-daemon/Cargo.toml`: added `mew-provider-fake`, `tempfile`, `async-trait` as dev-deps.
- `mew-tools/Cargo.toml`: added `test-utils` feature so `ToolCtx::test_new` is available to integration tests.
- `mew-tools/src/lib.rs`: `test_new` now `#[cfg(any(test, feature = "test-utils"))]`.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo test --all` — 667 tests, 0 failures.

### Foundation for the web harness

The Tier 1 e2e + concurrency tests validate the protocol and server surface that a future web frontend will sit on top of. The protocol itself (`mew-protocol`) is now exhaustively covered, so a JS/TS client implementing against `ClientMessage`/`ServerMessage` has a stable wire contract pinned down by tests. Next obvious work for the web harness UI: a thin WebSocket client library + a basic HTTP→WebSocket adapter so a browser frontend can connect to `mew daemon`.


---

# Progress — 2026-06-26 (web harness UI)

## Web harness UI: A → B → C → D end-to-end

Started the web harness path: a browser-based chat UI that talks to the mew daemon. Built all four layers from the original plan. The system is now runnable end-to-end via the new `mew-web` binary.

### Layer A — Daemon TCP listener

- `crates/mew-daemon/src/lib.rs`: `DaemonServer` is now generic over the connection stream (was Unix-only). Added `run_tcp(addr)` that binds a TCP `SocketAddr` and accepts WebSocket connections. `AgentBuilder` changed from `Box<dyn Fn>` to `Arc<dyn Fn>` so two listeners can share one closure.
- `crates/mew/src/main.rs`: `mew daemon --port 127.0.0.1:9847` flag added. `--socket` and `--port` are independent — pass both for dual listeners, just `--port` for TCP-only.
- `crates/mew-daemon/tests/tcp.rs`: 5 tests covering TCP path (SessionReady, streaming text, invalid JSON, concurrent connections, tool-call finish).

### Layer B — `mew-web-bridge`

- New crate `crates/mew-web-bridge/` with binary `mew-web`. Pure byte relay — no protocol awareness.
  - Listens on TCP+WS at `--port` (default `127.0.0.1:9847`).
  - On WS upgrade: opens a fresh connection to the daemon's Unix socket, proxies frames in both directions.
  - On plain GET: serves embedded static assets (`/`, `/main.js`, `/style.css`).
  - HTTP routing uses `BufReader::fill_buf()` peek-then-hand-off-to-`accept_async` so tungstenite's handshake reads from the same buffer (without this, the bridge hangs waiting for bytes already consumed).
  - Auto-spawns `mew daemon` via subprocess if not already running.
- `crates/mew-web-bridge/tests/e2e.rs`: 5 end-to-end tests (session relay, streaming text relay, invalid JSON as server error, concurrent sessions independence, tool-call finish end-to-end).

### Layer C — `mew-web-client` (TypeScript)

- New top-level directory `mew-web-client/` (not a Cargo crate — it's a TypeScript package).
- `package.json` + `tsconfig.json` — strict TS, ES2022, generates `.d.ts` for consumers.
- `src/index.ts` — typed `MewClient` class with:
  - `connect()`, `disconnect()`, `newSession(cwd?)`, `prompt(text, attachments?)`, `cancel()`, `slashCommand(cmd)`, `respondToPermission(id, decision)`.
  - `on(event, cb)` typed event handlers for every `ServerMessage` variant (provider, tool-start/end, permission-request, subagent-start/status/end, todos-updated, slash-result, etc.).
  - Pluggable `socketFactory` so Node users can inject `ws`, browsers use native WebSocket.
- `src/__tests__/client.test.ts`: 10 tests using an in-memory `MockWebSocket` (no daemon needed). Covers connect, session handshake, prompt/cancel/slash, permission-request round-trip, event dispatch, off() unsubscribe, malformed JSON, error routing. All 10 pass via `node --experimental-strip-types --test`.

### Layer D — Minimal chat UI

- `mew-web-ui/` at the workspace root: `index.html`, `style.css`, `main.js`. Vanilla JS, no framework, no build step.
- The HTML is the chat shell; the JS speaks the wire protocol directly and renders streaming text, tool calls, permission dialogs, slash results. ~250 lines.
- Embedded into the bridge via `include_bytes!` so `mew-web` is a single static binary.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo test --all` — 677 tests pass.
- `cd mew-web-client && node --experimental-strip-types --test src/__tests__/client.test.ts` — 10 tests pass.
- Manual smoke (out of band, but reproducible): `cargo run -p mew-web -- --port 127.0.0.1:9847`, open `http://127.0.0.1:9847/` in a browser, type a prompt, see streaming text.

### What's still TBD for the web harness

- The static UI is hand-rolled JS. A natural next pass is to switch `main.js` to import the compiled `mew-web-client` (would also exercise the library from a browser context).
- Auth: anyone with network access to the bridge can drive the daemon. Add a token check on the bridge side, or front the bridge with a reverse proxy.
- TLS: bridge currently listens plain. Add `--tls-cert`/`--tls-key` flags and pass through `wss://`.
- Session resume across reloads: persist `session_id` to localStorage so a refresh reconnects to the same daemon session.

---

# Progress — 2026-06-26 (reasoning truncation)

## Reasoning truncation: stop open-model "But.. Wait.. Actually.." loops

Added `mew-agent::ReasoningTruncator` — a small piece of logic in the turn loop that breaks the overthinking loop GLM-class models fall into. Inspired by a TerminalBench write-up: when a single reasoning trace exceeds a threshold, truncate the text in place, forge a short assistant acknowledgement into history, and force the next request to use `tool_choice: required`. Provider-agnostic (no stream mutation) and works against every adapter mew supports today.

### What changed

- **`mew-hooks` + `mew-provider`**: new `ToolChoice` enum (Auto / Required / None_) and a `tool_choice` field on `Request` and `ChatParams`. OpenAI adapter serializes it as `"auto"|"required"|"none"`; Anthropic as `"auto"|"any"|"none"`. Fake provider ignores it.
- **`mew-agent`**: new `ReasoningTruncator` struct (`crates/mew-agent/src/reasoning_truncator.rs`). Default threshold is **5k tokens** (set as `DEFAULT_REASONING_TRUNCATION_THRESHOLD`). Threshold = 0 disables. Master switch via `Agent::set_reasoning_truncation_enabled(false)`.
- **Token counting is approximate** — `len(chars).div_ceil(4)`. Documented as a soft cap, not a precise tokenizer.
- **Forged acknowledgement** is injected as a synthetic `Part::Text` with `synthetic: true`, text: `"I've been thinking too long. Acknowledging overthinking — committing to my next action now and stopping further deliberation."` Exposed as `mew_agent::TRUNCATION_ACK_TEXT`.
- **Wiring** in `turn.rs`: after each MessageEnd, the agent walks the assistant message's `ReasoningPart`s, calls `maybe_truncate_reasoning_in_place` (truncates any over-threshold), and if any was truncated, appends the ack and sets `force_tool_choice_next`. At the start of the next request, `take_force_tool_choice()` is read into `req.params.tool_choice`.

### Tests added

11 new tests, all passing:
- 7 unit tests on `ReasoningTruncator` itself: threshold behaviour, flag consumption, default value, marker text, disabled-when-zero.
- 4 integration tests through `agent.run()`:
  - `test_long_reasoning_truncates_and_forges_ack` — long reasoning (20k chars) with 1k threshold gets truncated, ack appears in history, **next request has `tool_choice: Some(Required)`** (verified via `CapturingProvider`).
  - `test_short_reasoning_does_not_trigger_truncation` — 800 chars < 5k default, no truncation, no ack, no force flag.
  - `test_truncation_disabled_when_threshold_zero` — long reasoning + `set_reasoning_truncation_threshold(0)` leaves the trace intact.
  - `test_set_reasoning_truncation_disabled_master_switch` — `set_reasoning_truncation_enabled(false)` overrides a high threshold.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo test --all` — 688 tests pass.

### Open follow-ups

- **Default `max_output_tokens` for large-context models**: separately discussed, not yet built. For models with very large total context (e.g. 400K), the input window = total - max_output. Defaulting max_output to a smaller value (e.g. 32K instead of 128K) would leave more room for context. Tracked for a follow-up.

---

# Progress — 2026-06-26 (default `max_output_tokens`)

Added a per-model cap on `params.max_tokens` so large-context models (e.g. GPT-5-Codex with 400K total / 272K input / 128K max output) don't silently burn 128K of the user's input window on output. The default caps output at 32K (or the model's natural max, whichever is smaller), and is honored uniformly by both OpenAI and Anthropic adapters.

Plan: `MAX_OUTPUT_TOKENS.md`.

### What changed

- **`mew-catalog`**: new `Catalog::max_output(model_id) -> Option<i64>` method. Returns `None` when the model is unknown (no hard-coded fallback — an unknown max_output is meaningfully different from a known 128K).
- **`mew-provider-anthropic`**: `build_request_body` now reads `req.params.max_tokens` as the baseline instead of hard-coding 4096. This was a long-standing gap — the adapter silently ignored plugin-set `max_tokens` even when the dispatcher set them. The thinking-budget bump still applies on top. `Some(0)` is honored verbatim; only the API floor (`max_tokens >= 1`) is enforced explicitly, and only when thinking isn't on.
- **`mew-agent`**: new `Agent::default_max_output_tokens: i64` field (0 = no override) and a saturating setter (`< 0 → 0`, `> i32::MAX → i32::MAX` at the call site in `turn.rs`). `build_session_agent` derives the default from the catalog: `cat.max_output(model_id).map(|v| v.min(32_768)).unwrap_or(0)`. The turn loop injects the default into `ChatParams.max_tokens` when the dispatcher returns `None`; a dispatcher that returns `Some(n)` still wins (the contract is preserved).
- **No Config / env-var wiring** in this slice — the plan marked them optional and the runtime setter is sufficient for the demo. The plan records the precedence (catalog → TOML → env → setter) for a follow-up.

### Tests added

14 new tests, all passing:
- **7 in `mew-agent`**: setter basics, negative-clamp, huge-saturation, zero-disables, default-injected-into-request, override-by-dispatcher (twice — `Some(1234)` wins, `Some(0)` honored). The user-plugin override tests needed 23 hand-written pass-throughs to the embedded `NopDispatcher`; documented inline.
- **5 in `mew-provider-anthropic`**: `params.max_tokens` honored on the wire, thinking-bump uses max of (default, `budget+4096`) — both directions tested (`8_000` budget → default wins, `64_000` → thinking wins), `Some(0)` without thinking → falls back to 4096 (API floor), `Some(0)` with thinking → thinking-bump handles the floor naturally, no-params → 4096 default.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo test --all` — 702 tests pass.

### Notes for the plan

The plan's test #12 had a math error (`30_000 + 4096 = 34_096 > 32_768`, so thinking wins, not the default) — caught by the Anthropic test, fixed in the plan. The corrected plan uses `8_000` budget (default wins) and `64_000` (thinking wins).

---

# Progress — 2026-06-26 (subprocess e2e)

Wired up end-to-end testing that exercises the **actual binaries** (`mew` and `mew-web`) as real subprocesses, not just the library code. The new test in `crates/mew-web-bridge/tests/bin_e2e.rs`:

1. Locates the `mew` and `mew-web` binaries in `target/debug/`.
2. Picks a free TCP port and a tempdir for the daemon's Unix socket.
3. Spawns `mew daemon --fake-provider --socket <tmp>/mew.sock` (new `--fake-provider` flag — see below).
4. Spawns `mew-web --port <free> --daemon-socket <tmp>/mew.sock --spawn false` (the bridge skips its own daemon-spawn because one's already running).
5. Connects a raw `tokio_tungstenite` WebSocket client to the bridge's port.
6. Sends `NewSession` → asserts `SessionReady` arrives.
7. Sends `Prompt` → asserts the streamed `PartDelta` events reassemble to "hello from fake provider" and `MessageEnd(Stop)` arrives.
8. Closes the WS, child processes are killed via `Drop` guard.

The test **gracefully skips** with a clear message if the binaries aren't built (`eprintln!` to stderr, returns `Ok(())`). That keeps `cargo test --all` green even before a fresh build, and `just ci` runs `build-web` first so it always builds.

### What changed

- **`crates/mew/src/main.rs`**: new `--fake-provider` flag on the `Daemon` subcommand. When set, every connection gets a `FakeProvider`-backed agent that streams a fixed "hello from fake provider" text. Bypasses all real-provider setup so the daemon runs without network access. Documented as "for tests, demos, and offline experimentation — do not use in production."
- **`crates/mew-web-bridge/tests/bin_e2e.rs`**: new subprocess e2e test. Includes `ChildGuard` for clean process teardown, `binary_paths()` helper that locates the binaries, and `wait_for_unix_socket`/`wait_for_tcp_port` polling helpers.
- **`justfile`**: new `just e2e` recipe (`build-web && cargo test -p mew-web-bridge --test bin_e2e`). `just ci` now also depends on `e2e` so the subprocess test is part of the gate.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo test --all` — 703 tests pass (was 702 + 1 e2e).
- `just e2e` — builds and runs the subprocess test in ~2s.

### Deferred (per the earlier conversation)

- **TS-side e2e**: not yet wired. Once `mew-web-ui` is a real consumer of `mew-web-client`, a Node-side test that spawns the same binaries and uses the compiled `mew-web-client` library to round-trip would be the natural next step. Tracks with the "we have a decent UI" milestone.

---

# Progress — 2026-06-29

## Router tier refactor — removed `turn_threshold`, fixed build

Finished the cleanup pass on the three-tier router work. Removed the
`turn_threshold` field from the settings UI and fixed two unrelated build
errors that surfaced in `crates/mew/src/main.rs`.

### Changes

- **`crates/mew/src/config_editor.rs`**: removed the `ProviderTurnThreshold`
  field from the router provider settings panel. Router providers now only
  expose `nano`, `micro`, and `deci` in the UI.
- **`crates/mew/src/main.rs`**:
  - Reconstructed the `Router` instance before wrapping it in `Routed`
    (`Router::new(nano, micro, deci)`), fixing the "cannot find value
    `router`" compile error.
  - Passed `&cwd` to `mew_context::Loader::new()` so `cwd` remains available
    for the subsequent `load_project_vars(&cwd)` call, fixing the borrow/move
    error.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo test --all` — all tests pass.

### Notes

The router itself still behaves as implemented: `nano` on the first turn,
`micro` for later non-tool turns, and `deci` whenever tool results are
present in the conversation.

---

# Progress — 2026-06-29

## Router tier refactor — router is now task-only

Made the router a task-only primitive. It can no longer be selected as the
main chat provider, and tool-call turns stay on the user's chosen model.
The `nano`/`micro`/`deci` tiers are still used for subagents and the
permission classifier.

### Changes

- **`crates/mew/src/main.rs`**:
  - Added `find_router_provider(cfg)` to locate the first provider configured
    as a router, preferring one literally named `router`.
  - `maybe_set_classifier_provider()` now wires the classifier to the router's
    `micro` tier whenever any router provider exists, independent of the active
    chat provider.
  - `MainModelResolver` gained a `router_provider_id` field; tier keywords
    (`nano`/`micro`/`deci`) are resolved against the router provider, not the
    active chat provider.
  - `build_provider()` now rejects `kind = "router"` providers with a clear
    error when something tries to use them as the main chat provider.
  - Removed the router-provider branch from `build_provider()`, the recursive
    tier construction, and the special-case display logic that showed the
    `deci` model in the TUI/one-shot status line.
- **`crates/mew/Cargo.toml`**: removed the `mew-provider-router` dependency.
- **`CLAUDE.md`**: updated the router description to reflect the task-only
  design.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo test --all` — all tests pass.
