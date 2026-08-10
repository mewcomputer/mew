# AGENTS.md

Project guidance for coding agents working in this repository.

## Working rules

- Preserve unrelated edits in the worktree. Inspect `git status` before editing and keep changes scoped to the request.
- Follow the surrounding Rust or TypeScript style. Prefer small, reviewable changes and existing helpers over new abstractions.
- Write behavior tests with implementation changes. Run the narrowest relevant tests first, then the broader gate when practical.
- Update `CURRENT.md` with an append-only dated entry after each meaningful chunk of work. Do this after verification, not as a substitute for it.
- Do not manually edit generated theme outputs. Change the manifest and run the theme codegen command.
- When adding shadcn components, use `npx shadcn@latest add <component>` so the CLI owns installation and local component scaffolding.

## Commands

Rust and repository checks:

```bash
cargo build -p mew
cargo test --all
cargo test -p mew-agent
cargo test test_text_turn
cargo clippy --all -- -D warnings
cargo fmt
just ci                         # full CI gate: fmt, clippy, arch, themes, Rust/JS tests, e2e
just arch-check
just theme-codegen-check
just test-v
just deps
just tidy
MEW_RECORD=1 cargo test -p mew-provider-openai
just install
```

Web, capture, docs, and mobile workflows:

```bash
just install-js                 # first checkout or lockfile changes
just test-js
just build-js
just build-web
just lint-all                   # Rust clippy + web-client TypeScript check
just dev-web                    # Vite UI + bridge; accepts --open and bridge flags
just dev-ui                     # Vite UI only
just desktop-dev                # Tauri desktop shell + bundled debug daemon + HMR
just desktop-build              # release Tauri bundle + architecture-specific daemon sidecar
just site-dev
just e2e                        # builds web assets and runs subprocess bridge tests
just ios-core
just ios-test
mew tui-capture --help
```

`mew-web-bridge` embeds `mew-web-ui/dist`, so rebuild the UI before testing a
production bridge. `mew tui-capture` is deterministic in local harness mode;
daemon-connected capture requires a running daemon and real credentials. MP4
capture also requires `ffmpeg`.

## Repository shape

The Cargo workspace is defined in `Cargo.toml`. The important boundaries are:

```text
frontends and entry points
  crates/mew             CLI and application wiring
  crates/mew-tui         ratatui app, event loop, display state, headless harness
  crates/mew-daemon      session-owning WebSocket daemon
  crates/mew-web-bridge  browser TCP/WS bridge and embedded static UI server
  crates/mew-mobile-core UniFFI mobile core and iroh client

agent runtime
  crates/mew-agent       conversation state, turn loop, compaction, tool execution
  crates/mew-provider    Provider trait, requests, streaming events, auth helpers
  crates/mew-provider-openai       OpenAI-compatible chat/SSE adapter
  crates/mew-provider-anthropic    Anthropic-compatible adapter
  crates/mew-provider-responses    OpenAI Responses and Codex OAuth adapter
  crates/mew-provider-router       tiered provider resolution for task/subagent use
  crates/mew-provider-fake          deterministic test/demo provider
  crates/mew-tools        built-in tools and tool context
  crates/mew-mcp          MCP stdio/HTTP clients and MCP tool wrappers
  crates/mew-subagents    definitions, loaders, runners, child sessions
  crates/mew-personas     persona definitions, loaders, transitions, model/tool policy
  crates/mew-skills       SKILL.md discovery and loading
  crates/mew-context      AGENTS.md/CLAUDE.md/project context loading
  crates/mew-prompts      embedded prompt resources and template rendering
  crates/mew-hooks        Dispatcher trait, hook types, plugin host
  crates/mew-hooks-runtime subprocess plugin transport
  crates/mew-ext-broker   extension manifests, capabilities, consent, tokens, sandboxing

shared data and infrastructure
  crates/mew-message      canonical messages, parts, tool state, turn manifests
  crates/mew-protocol     JSON WebSocket wire types and command registry
  crates/mew-session       JSONL history, metadata, groups, and session readers/writers
  crates/mew-config        config/state loading, credentials, permission engine, shell parsing
  crates/mew-catalog       model metadata, pricing, thinking variants, cache
  crates/mew-hashline      hash-anchored file patch format and recovery
  crates/mew-harness       shared filesystem discovery helpers
  crates/mew-raster        deterministic ratatui Buffer to PNG/MP4 rasterization support
  crates/ratatui-mdstream  streaming markdown, wrapping, syntax highlighting, and tables

other frontends
  apps/mew-desktop         native GPUI desktop client and app-owned daemon supervisor
  mew-web-client            TypeScript wire-protocol client
  mew-web-ui                React/Vite browser frontend
  native/cef-host           macOS CEF helper used by the native desktop browser portal
  mew-ios                   Swift app using mew-mobile-core
  site                      Astro documentation/marketing site
```

## Runtime architecture

The canonical path is:

```text
frontend → transport → daemon → session → agent → provider
```

The built-in TUI is daemon-only: local mode was sunset. The `mew` chat command
spawns a `mew daemon` on a loopback TCP port (if none is running) and connects
via `runtime::daemon::DaemonTarget` + `DaemonClient`. The daemon owns the
`Agent`; the TUI receives the same logical `AgentEvent` stream after wire
translation. Because the daemon serializes turns (`turn_lock` +
`current_turn_cancel`), concurrent-turn races are impossible. The deprecated
`runtime::local::LocalTarget` was removed.

The web path is `browser → mew-web-bridge → daemon`. The bridge relays WebSocket
frames to the daemon's Unix socket and serves the compiled React assets. The
mobile path uses `mew-mobile-core` over iroh and the same protocol model.

The desktop path is `apps/mew-desktop (GPUI) → transport → daemon`. The native
client uses a shared loopback rendezvous port (`25566` by default, overridable
with `MEW_DESKTOP_DAEMON_PORT`), performs a WebSocket health check, and attaches
to an existing daemon without owning it. If no healthy daemon is present, the
supervisor starts `mew daemon --port 127.0.0.1:<port>`, waits for a
protocol-level health check, and owns that child for the app lifetime.
`MEW_DESKTOP_DAEMON_URL` is attach-only; `MEW_DESKTOP_DAEMON_BINARY` selects an
external executable for the app-owned launch path. Remote profiles use the
same native client model over iroh.

The browser frontend remains a separate React/Vite web client. Native desktop
packaging is handled by `scripts/package-desktop-native.sh`; the native browser
portal uses `native/cef-host` behind `mew-browser-host` and does not share the
web client’s host bridge.

`SessionManager` owns active sessions and reloads idle top-level sessions from
the path returned by `mew_session::session_dir()` (`MEW_SESSION_DIR` overrides
it). A session serializes turns with a lock and
broadcasts provider, tool, permission, question, subagent, and metadata events
to all attached clients. Permission and question requests are paired by request
ID on the wire. The daemon is single-user and has no general authentication or
multi-tenant isolation; do not expose an unauthenticated TCP listener publicly.

### Agent turn loop

`Agent::run_with_parts` returns an `mpsc::Receiver<AgentEvent>` immediately and
runs the turn asynchronously. `turn_loop` streams provider events, appends the
assistant message, collects tool calls at `MessageEnd`, executes pending tools
sequentially, and loops back to the provider when tool results require another
turn. Compaction, hooks, permission checks, subagents, shell jobs, and session
persistence all sit around this loop.

`mew_message::Message` is the API/history model. Its parts are `Text`,
`Reasoning`, `File`, `ToolCall`, `ToolResult`, and `Compaction`. Tool calls carry
`ToolState` transitions of `Pending → Running → Completed | Error`.

The TUI's `App` owns a separate display store. It merges all parts from a
multi-turn agentic exchange into the visible assistant entry. Streaming markdown
uses `app.md_stream` and `app.md_state`; only the last active text part uses the
incremental renderer, while completed or earlier parts use the cache.

### TUI event flow

1. Crossterm input becomes a `mew_tui::events::Action`.
2. `EventLoop::forward_agent_events` pumps agent events into the TUI channel.
3. The main loop updates `App` state and draws.
4. The drain coalesces ticks and queued actions, processes at most four agent
   events per frame while streaming, then replays actions through the runtime
   dispatcher.

### Runtime invariants

These are enforced by `just arch-check` and by the deny lint in
`crates/mew/src/runtime/dispatch.rs`:

1. Match `Action` and `SlashResult` only in `runtime/dispatch.rs`. Constructors,
   tests, and `App::handle_slash` are fine; execution belongs in
   `handle_action`.
2. The event drain never interprets actions. It coalesces events, caps streaming
   work, queues every produced action, and replays the queue after draining.
3. Push display messages through `app.push_message`, `app.push_synthetic_message`,
   or `app.push_user`. Direct `app.messages.push(...)` skips dirty-state updates.
4. A new command needs a `CommandTarget` method, a dispatch arm, and a test in
   the same change. `LocalTarget` and `DaemonTarget` are both implemented today.
5. `Unsupported` is the only sanctioned backend no-op. Returning it renders a
   visible alert; never swallow an unsupported command.

### Adding providers, tools, and protocol features

To add a provider, implement `Provider` in a `mew-provider-*` crate and add the
adapter selection in `crates/mew/src/setup/providers.rs`. Current configured
adapter shapes are `openai`, `anthropic`, and `responses`; keep provider-specific
wire details inside the adapter. Add fixture or adapter tests before changing
the shared `ProviderEvent` model.

To add a built-in tool, implement `mew_tools::Tool` with a name, description,
JSON schema, sensitivity, and async `execute`, then register it in
`crates/mew/src/setup/agent.rs::build_tools`. `ReadOnly`, `Mutating`, and
`Dangerous` sensitivity feed the permission engine. MCP tools and approved
plugin tools are registered through their own integration paths.

To add a daemon feature, update `ClientMessage`/`ServerMessage` in
`mew-protocol`, the daemon handler and event translators, `DaemonClient`, the
TypeScript client, and the web store/UI as applicable. Add JSON roundtrip and
end-to-end coverage. Keep session-management messages separate from the
streaming `AgentEvent` translation path.

## Loadable project resources

`mew-context` walks from the current directory to the git root and loads files
from most general to most specific. At each level, `AGENTS.md` wins over
`CLAUDE.md`; `.mew/AGENTS.md` and `.mew/wiki.md` are additive. Global context
uses `~/.config/mew/AGENTS.md`, then `~/.claude/CLAUDE.md` as fallback.

Skills and personas use project and global locations under `.mew`, `.opencode`,
`.claude`, and `.agents`, with project-local definitions taking precedence over
global definitions and earlier duplicates winning. Skills are directories
containing `SKILL.md`; personas are directories containing `PERSONA.md`; custom
subagents are markdown definitions in the corresponding agent locations. Built-in
skills and the `planner`/`builder` personas are added when not overridden.

Persona frontmatter supports `name`, `description`, and a `mew:` or `polytoken:`
block for model pinning, tool/skill allowlists, denied tools, templates,
transitions, fallback models, and an accent color. Persona bodies are injected
into a freshly rebuilt system prompt each turn.

Subagent definitions can pin a fully qualified `provider/model` or a router tier
(`nano`, `micro`, `deci`), restrict tools, and set `max_turns` or
`max_duration_secs`. They can also opt into nested spawning with
`can_spawn: true` (subject to `orchestration.max_subagent_depth`) and require a
typed final output with `output_schema` (YAML map or `@path` JSON file; the
runner validates the child's final output as JSON and grants one corrective
turn on failure). Session-level orchestration guardrails (concurrency cap,
leak reminders) live under `[orchestration]` in config.toml; see
`docs/development/dev-orchestration.md`. The runner gives active tasks a
deterministic human display name and emits progress through
`AgentEvent::SubagentStatus`.

### Namespace references in input

In the TUI, skills, models, and subagents can be referenced inline in the chat
input using `@namespace:value` syntax:

- `@skill:name` (e.g. `@skill:clarify`) — inlines the skill body into the
  model-facing prompt. This is the primary skill-reference form.
- `@model:provider/model` (e.g. `@model:openai/gpt-4o`) — inlines a model
  reference marker, useful for indicating which model a subagent or tool call
  should use.
- `@subagent:name` (e.g. `@subagent:researcher`) — inlines the subagent's
  description into the prompt, giving the model context about available
  subagents.

The `@namespace:` prefix is extensible to other types in the future. Typing
`@skill:`, `@model:`, or `@subagent:` opens an autocomplete picker listing
available items. Namespace references resolve client-side in the TUI (same as
`@file` mentions); the web UI does not process them and sends them as literal
text. Templated skills fall back to their raw body since the TUI lacks the
template context; the model can still call the `skill` tool for the rendered
version.

## Configuration and security boundaries

Use `mew config path` to locate the platform-specific config directory. It holds
`config.toml`, `state.toml`, credentials fallback data, plugin storage, and
related config state. Session storage, extension consent, model caches, and
installed themes have their own resolver functions and may use separate data
locations. Configuration layers are:

1. Built-in defaults (`opencode-zen`, `opencode-go`, `z-ai`, `deepseek`, `umans`,
   and `codex` provider entries).
2. `config.toml` overrides and additions.
3. `MEW_` environment variables, with `__` for nesting.

`.env` is loaded at process startup. Credential lookup is
`MEW_CRED_<NORMALIZED_REF>`, then the `mew` keyring service, then
`credentials.json` in the config directory. Do not log or commit credentials.

Important config fields include `default_model`, `models`, `default_persona`,
`plan_path`, `workspace.roots`, `permissions`, `secrets`, `plugins`, and
`tui.theme`. `state.toml` persists last provider/model, sidebar state, disabled
plugins, revoked extensions, and the active theme. Use
`mew_session::session_dir()` for session files, `ConsentState::load()` for
extension consent, and `Theme::themes_dir()` for installed themes instead of
hardcoding platform paths.

`workspace.roots` protects path-based tools and also adds an escape check for
shell, background-shell, and shell-monitor commands. Empty roots opt out of the
escape tier and default path tools to the current directory. Secret file globs
force prompts, while configured secret words are redacted from search/tool
output. Permission modes are `standard`, `permissive`, `auto`, `auto_plus`, and
`dangerous`; dangerous mode bypasses prompts and rules, so use it only when
explicitly requested.

Daemon sessions currently carry a `cwd` through the protocol and persist it in
session metadata, but `build_session_agent` still derives its operational cwd
from the daemon process. Treat per-session loading of context, skills, personas,
MCP config, shell state, and workspace roots as incomplete until the builder
accepts and uses `AgentBuildParams.cwd`. Project `.env` and MCP discovery are
also process-startup concerns, not per-session configuration.

### Plugins and extensions

Bare executable plugins are discovered in `~/.config/mew/plugins` and
`.mew/plugins`, then run through newline-delimited JSON-RPC by
`mew-hooks-runtime`. They can register tools and slash commands and observe or
mutate dispatcher hooks. Plugin host storage is namespaced and persisted.

Structured extension packages live in `~/.config/mew/extensions/<name>` or
`.mew/extensions/<name>` and contain `mew-ext.toml`. Project packages beat global
packages. `mew-ext-broker` applies capability-based consent, attach tokens, and
macOS Seatbelt sandboxing. Network is denied by default for sandboxed extensions;
non-macOS platforms currently warn and run without OS sandbox enforcement.

## Themes and generated assets

`crates/mew-tui/resources/theme_manifest.json` is the source of truth for the
TUI and web theme tokens. `theme_codegen` generates:

- `crates/mew-tui/src/theme_generated.rs`
- `mew-web-ui/src/generated-themes.css`
- `crates/ratatui-mdstream/resources/theme.tmTheme`

Run `just theme-codegen` after manifest changes and `just theme-codegen-check`
before committing. Installed theme JSON files are validated against the shared
manifest. See `docs/THEMING.md` for token conventions.

## Verification targets

- Rust behavior: `cargo test --all` and `cargo clippy --all -- -D warnings`.
- Architecture rules and generated themes: `just arch-check` and
  `just theme-codegen-check`.
- TypeScript client: `just test-js`, `just build-js`, and `pnpm --filter
  @mew/web-client exec tsc --noEmit`.
- React UI: `pnpm --filter mew-web-ui test` and
  `pnpm --filter mew-web-ui build`.
- Daemon/bridge subprocess path: `just e2e`.
- TUI appearance: golden frames under `crates/mew-tui/tests/golden/` and
  `mew tui-capture` screenshots.

When a test fails, fix the underlying behavior or document a reproducible
external blocker. Do not remove coverage or weaken a check to make the suite
green.
