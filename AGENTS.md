# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Commands

```bash
cargo build -p mew                  # build the binary
cargo test --all                    # run all tests
cargo test -p mew-agent             # run tests for one crate
cargo test test_text_turn           # run a single test by name
cargo clippy --all -- -D warnings   # lint (CI requires zero warnings)
cargo fmt                           # format
just ci                             # fmt + clippy + test (full CI gate)
just run --model deepseek-v4-flash "hello"   # run a prompt
just test-v                         # run tests with verbose output
just deps                           # show crate dependency tree
just tidy                           # update lockfile dependencies
just record                         # record a new provider test fixture (set MEW_RECORD=1)
just install                        # install to ~/.cargo/bin
```

## Architecture

Three-layer pipeline: **TUI → Agent → Provider**

```
mew (binary)
  ├─ mew-tui        event loop, ratatui UI, App state
  │    │              ui/ (draw orchestration) → chat, sidebar, input, status, overlays, welcome
  │    ├─ mew-agent conversation state, tool execution loop, AgentEvent channel
  │    │    ├─ mew-provider     Provider trait + ProviderEvent stream
  │    │    │    ├─ mew-provider-openai
  │    │    │    ├─ mew-provider-anthropic
  │    │    │    ├─ mew-provider-fake (tests only)
  │    │    │    └─ mew-provider-router  splits traffic between cheap + capable models
  │    │    ├─ mew-tools        Tool trait + built-ins (Bash, Read, Write, Edit, Glob, Grep, Echo, ExitTool, ProgressUpdate)
  │    │    │    └─ mew-mcp     McpTool wraps remote MCP servers as Tool impls
  │    │    ├─ mew-subagents    SubagentDef, SubagentRunner, child agent spawning
  │    │    ├─ mew-personas     switchable system prompts with model pinning + tool allowlists
  │    │    ├─ mew-hooks-runtime  subprocess-based Dispatcher: loads external plugins
  │    ├─ mew-daemon    WebSocket server. Owns the agent loop;
  │    │    per-connection session, ID-paired permission/ask requests via `mew-protocol`.
  │    │    Listens on Unix socket AND/OR TCP (--port flag) for browser frontends.
  │    ├─ mew-protocol  Wire message types + JSON codec (ClientMessage/ServerMessage).
  │    │    Future: MessagePack over binary frames, same schema.
  │    ├─ mew-web-bridge  TCP+WS listener that relays browser WS connections to the
  │    │    daemon's Unix socket. Also serves the chat UI's static assets.
  │    └─ mew-skills    skill discovery + loading from .mew/skills, .opencode/skills, etc.
  └─ mew-context   discovers AGENTS.md / AGENTS.md files from cwd up to git root

Non-Rust frontends:
  mew-web-client (TypeScript)  Typed client for the mew wire protocol. Used by the
    bundled chat UI and reusable for Discord/iOS/web frontends. Builds to
    ESM with `.d.ts` types via `tsc`.

Shared crates (data types, traits, and utilities used across layers):
  mew-message   canonical message/part types (Message, Part, Role, Finish, ToolState…)
  mew-config    config.toml + credential resolution + permission rule engine
  mew-hooks     Dispatcher trait — pluggable hooks for observability, mutation, permissions
  mew-session   JSONL session persistence (~/.local/share/mew/sessions/<id>.jsonl)
  mew-catalog   models.dev catalog with 24-hour cache (pricing, shapes, context windows)
  ratatui-mdstream  streaming markdown → ratatui Lines with syntect highlighting
```

### Event flow

1. User presses Enter → `Action::Submit` → `agent.run(prompt)` returns `mpsc::Receiver<AgentEvent>`
2. `forward_agent_events` pumps that receiver into the TUI's main event channel
3. TUI `recv()` loop processes events → `app.handle_agent_event()` updates App state → `draw()`
4. Inside the agent: `turn_loop` streams provider events, collects tool calls from `MessageEnd(ToolUse)`, executes them sequentially, then loops back for the next LLM turn

The drain loop after each primary event coalesces Agent/Tick events but caps agent events at 4 per frame during streaming so text appears incrementally.

### Message model

`Message` contains a `Vec<Part>`. Parts are `Text | Reasoning | ToolCall | ToolResult | Error`. `ToolCallPart` carries a `ToolState` enum (`Pending → Running → Completed | Error`) updated live as the tool executes. `app.tool_states: HashMap<PartId, ToolDisplayState>` mirrors this for the TUI.

The TUI's `app.messages` is the **display** store. The agent's `self.messages` is the **API history** store. They're separate. In the TUI, all parts from a multi-turn agentic loop (text → tool calls → follow-up text) are merged into one assistant message display entry.

### Streaming markdown

`app.md_stream` / `app.md_state` track the currently-streaming text part. Only the **last** `Part::Text` in the active message uses `render_streaming(md_state)`; earlier text parts (preceding tool calls) use the static cached path. On `MessageEnd`, the stream is finalized and `pending_md_rerender` triggers a full re-render from `tp.text` on the next frame.

### Adding a provider

Implement `Provider` in a new `mew-provider-*` crate. The two shapes already handled are `openai` (SSE, delta-based) and `anthropic` (SSE, content-block events). `build_provider` in `main.rs` maps `shape` strings to adapters; add a new match arm there.

Router providers are a task-only configuration primitive. A provider entry with `kind = "router"` defines three tiers: `nano` (cheapest), `micro` (medium), and `deci` (most capable). The Auto/Auto+ permission classifier automatically uses the router's `micro` tier, and subagents can request a tier by passing `model: "nano"`, `model: "micro"`, `model: "deci"`, or any fully-qualified `provider/model`. Router providers cannot be selected as the main chat provider; the user's chosen model handles all chat turns, including tool-call turns, without escalation.

### Adding a tool

Implement the `Tool` trait (name, description, JSON schema, sensitivity, async execute). Register it in `build_tools()` in `main.rs`. MCP tools register automatically via `McpTool` after `connect_mcp_servers()`.

Tool `sensitivity()` controls the default permission gate: `ReadOnly` → auto-allow, `Mutating` → prompt, `Dangerous` → prompt. The `PermissionEngine` in `mew-config` applies declarative rules before prompting.

### Personas

Switchable system prompts with per-persona model pinning and tool allowlisting. Personas are loaded at startup from `PERSONA.md` files:

- **Search order** (earlier wins on duplicate name): `<cwd→git-root>/.mew/personas/<name>/PERSONA.md`, then `~/.config/mew/personas/<name>/PERSONA.md`
- **Format**: YAML frontmatter (`name`, `description`, optional `mew:` block with `model` and `tools` list) followed by markdown body
- **Model pinning**: a persona can force a specific `provider/model` pair (e.g. `z-ai/glm-4.5-air`), overriding the session default
- **Tool allowlisting**: `tools: []` (no tools), `tools: [read, bash]` (whitelist), or absent (all tools available)
- **Activation**: `mew-personas` discovers and loads them; the TUI exposes switching via the sidebar

The system prompt is rebuilt from scratch every turn, so persona body text is always injected fresh.

### Hooks / Dispatcher

The `mew-hooks::Dispatcher` trait is the plugin architecture. It exposes twenty-one hook points (`HookId` variants) across twenty-six `Dispatcher` trait methods covering the full agent lifecycle: `init`, `shutdown`, `on_register_tools`, `on_register_slash_commands`, `execute_slash_command`, `on_provider_event`, `on_tool_error`, `on_subagent_start`, `on_subagent_end`, `on_turn_end`, `on_pre_model_turn`, `on_stop`, `on_pre_compaction`, `on_post_compaction`, `on_chat_message`, `on_chat_params`, `on_chat_headers`, `on_system_prompt`, `on_tool_execute_before`, `on_tool_execute_after`, `on_permission_ask`, `on_shell_env`, `on_user_input`, `on_persona_change`, `on_session_save`, and `on_model_finish`.

`NopDispatcher` is the default (all passthrough/no-op). `mew-hooks-runtime` provides `SubprocessDispatcher`, which spawns plugins as subprocesses communicating via stdin/stdout newline-delimited JSON-RPC 2.0. Plugins can register dynamic tools (`ToolRegistration`), slash commands (`SlashCommandDef`), and hook into any lifecycle event. The `PluginHost` handle gives plugins access to a restricted config subset, key-value storage (per-plugin, persisted to disk), a notify channel to the TUI, and a `set_ui` function for rendering custom content beside the input area.

The upgrade path to a Wasmtime component-model runtime keeps the same `Dispatcher` trait — only the transport changes.

### Subagent run display

The sidebar shows a human-friendly name for every running subagent (e.g. `▸ Curie (researcher)  3s ↳ scanning the repo`). The display name is picked by the runner at spawn time from `mew_subagents::DISPLAY_NAMES` (25 entries) via `pick_display_name(seed: u128)`, which hashes the subagent's fresh `SessionId` with splitmix64 — deterministic, no `rand` dep, decent distribution.

To add a new name: append it to `DISPLAY_NAMES` in `crates/mew-subagents/src/lib.rs`. To plug a user-configurable pool in later, the function already takes `(seed, list)`-shaped args.

`AgentEvent::SubagentStatus { parent_call_id, tool_name, message }` carries the subagent's `progress_update(message: "...")` content from the runner up to the TUI's `SubagentState.last_progress`, which the sidebar renders on a sub-line with a `↳` indent. To add a new subagent-controlled UI affordance, the pattern is: runner emits a new `SubagentEvent` variant → pump translates to a new `AgentEvent` variant → `App::handle_agent_event` stores it in `SubagentState` → sidebar renders it.

## Configuration

Config file: `~/.config/mew/config.toml` (macOS: `~/Library/Application Support/ai.mew.mew/config.toml`)

Config is loaded via `config-rs` with layered sources (later wins):
1. Built-in provider defaults (`Config::default()`: `opencode-zen`, `opencode-go`, `z-ai`, `deepseek`)
2. `config.toml` (overrides built-in providers, adds custom providers)
3. Environment variables with `MEW_` prefix (`MEW_DEFAULT_MODEL`, `MEW_WORKSPACE__ROOTS`, `__` = nested path)

`.env` in the working directory is loaded via `dotenvy` at startup (before tracing init, so `RUST_LOG` works).

Credential resolution order for `credential_ref`:
1. Env var `MEW_CRED_<REF_UPPERCASED>` (hyphens → underscores)
2. System keyring (`mew` service, account = ref name)
3. `credentials.json` in the config directory

MCP servers are loaded from `mcp.json` in the working directory (Codex format: `{ "mcpServers": { "name": { "command": "...", "args": [...] } } }`).

Last-used model/provider is persisted to `state.toml` and restored on next launch. CLI `--provider`/`--model` flags override state, which overrides the built-in default (`opencode-zen`).

### Workspace sandboxing

`workspace.roots` is a list of directories the agent is allowed to touch. It feeds two layers:

- **Agent layer (read/write/edit/glob/grep)**: `agent.workspace_roots` (defaulted to current_dir when empty) is enforced by `ensure_workspace_path` before any path-based tool runs. This is the existing behavior.
- **Permission-engine escape tier (bash/shell_background/shell_monitor)**: the engine's `with_workspace_roots` (set in `build_permission_engine` from `cfg.workspace.roots`) inspects parsed shell commands. If any path-shaped positional arg resolves outside the configured roots, the decision is escalated from `AllowOnce` to `Prompt`. Deny rules still win. Sits between deny rules and the Permissive short-circuit so user rules and the mode hierarchy both apply. Empty `workspace_roots` disables the escape tier — useful as an opt-out.

Cwd sourcing for the escape tier: `input["cwd"]` (when present, e.g. on `shell_background` / `shell_monitor`) → caller-supplied `cwd` argument to `engine.check()` → engine's `default_cwd` → `Path::new(".")`. A `cat /etc/passwd` always escapes regardless of cwd. `$HOME/...` and `~/...` are conservatively flagged as escapes without trying to resolve them.

### Multi-workspace daemon sessions

A single daemon can serve sessions across multiple project directories. Each session carries its own `cwd` (sent in `NewSession { cwd }` and persisted in session metadata). The agent build path (`build_session_agent`) threads this cwd through:

- `Agent.cwd` field — all file tools, the permission engine, template context, and `plan_path` resolution read from it
- Skills, personas, context files (AGENTS.md/AGENTS.md), and subagent defs are loaded relative to the session cwd
- Shell session cwd and `workspace_roots` default to the session cwd
- Subagents inherit the parent agent's cwd and workspace roots via `SimpleRunner::with_cwd`
- Resume/attach passes `meta.cwd` through, so cwd survives daemon restarts and session eviction

The TUI client (`mew chat --connect`) sends its `current_dir()` as the session cwd. The web client already sends cwd. Sessions created without a cwd fall back to the daemon's launch directory (today's behavior).

**Limitations:** Project `.env` is not applied per-session (process env comes from the daemon's launch dir). `mcp.json` is not yet wired into daemon sessions (MCP config discovery is cwd-based but the daemon doesn't call `connect_mcp_servers`).

## Progress tracking

Save progress frequently to `CURRENT.md`. Treat it as append-only — add a dated section each time you complete a meaningful chunk of work. Summarize what was done, where, and any decisions made. User clears this file periodically.

## Runtime invariants

The `runtime/` module in `crates/mew/src/` is the single dispatch path. These invariants are enforced by `just arch-check` and `#![deny(clippy::wildcard_enum_match_arm)]` in `dispatch.rs`:

1. **Never match `Action` or `SlashResult` outside `runtime/dispatch.rs`.** The `handle_action` function is the only place that pattern-matches on these enums. Adding a variant breaks the build (clippy deny + exhaustive match).

2. **The drain never interprets events.** The drain loop coalesces scrolls/ticks, caps agent events at 4 per frame during streaming, and queues every produced `Action` into a `Vec<Action>`. After the drain exits, the vec is replayed through `handle_action`.

3. **Messages are pushed only through `App` methods.** Use `app.push_message(msg)`, `app.push_synthetic_message(text)`, or `app.push_user(display, attachments)`. Never call `app.messages.push(...)` directly — it skips `mark_chat_dirty()` and causes stale renders. Enforced by `just arch-check` grep.

4. **A new command means a `CommandTarget` method + dispatch arm + test in the same change.** The `CommandTarget` trait in `runtime/target.rs` defines the backend abstraction. `LocalTarget` implements it for standalone mode; `DaemonTarget` (Phase 2) for daemon mode.

5. **`Unsupported` is the only sanctioned way to not implement something.** Returning `Err(Unsupported("reason"))` from a `CommandTarget` method renders a visible alert — never a swallowed keypress.
