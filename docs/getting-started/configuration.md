---
title: Configuration
description: Configure Mew providers, credentials, permissions, and workspace.
---

Configure Mew with `config.toml` and environment variables. Mew stores runtime
preferences in `state.toml`. Run `mew config show` to print the effective
configuration, `mew config edit` to open the file in your editor, or
`mew config editor` to edit settings in the interactive TUI.

## File locations

| File | Path | Purpose |
|------|------|---------|
| `config.toml` | `~/.config/mew/config.toml` | Providers, permissions, secrets, and workspace settings |
| `state.toml` | Same directory | Runtime state (last model, theme, collapsed sections, disabled plugins) |
| `credentials.json` | Same directory | Plaintext credential fallback when keyring and env vars are unset |
| `.env` | Working directory or parent | Loaded at startup via `dotenvy` (for `RUST_LOG`, `MEW_CRED_*`) |

Override the config directory with `MEW_CONFIG_DIR` or `XDG_CONFIG_HOME`.

## Config sources (later wins)

1. Built-in defaults (see [Providers](/docs/using-mew/providers/) for provider IDs).
2. `config.toml` (overrides built-in providers and adds custom settings).
3. `MEW_`-prefixed environment variables. Use `__` for nested keys, such as
   `MEW_TUI__THEME`. Define list fields such as `workspace.roots` in
   `config.toml`.

The CLI loads `.env` at process startup. A running daemon keeps its startup
environment when you open or resume a conversation.

## Configuration reference

### Top-level fields

```toml
default_model = "deepseek-v4-flash"
default_persona = "builder"
plan_path = "plans/"
max_turns = 250
max_turns = 250

[workspace]
roots = ["~/code"]

[tui]
sidebar_finished_ttl_secs = 180
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_model` | string | `""` | Fallback model ID when no `--model` flag or `state.toml` entry applies |
| `default_persona` | string | `"builder"` | Startup persona. Set `"none"` or `"default"` to start without one |
| `plan_path` | string | `"plans/"` | Directory or fixed file path for `write_plan`. A directory gets generated names such as `plans/vale-nori.md` for use with `edit_plan` |
| `max_turns` | integer | `250` | Assistant turns allowed in one prompt before the agent stops with a turn-limit error. `0` removes the cap |
| `max_turns` | integer | `250` | Assistant turns allowed in one prompt before the agent stops with a turn-limit error. `0` removes the cap |
| `providers` | table | (built-in) | Provider definitions keyed by provider ID |
| `models` | array | `[]` | Custom model entries that override or extend the catalog |
| `code_mode` | table | (model default) | Run tools through a persistent Python session. See [Python code mode](#python-code-mode) |
| `permissions` | table | (empty) | Permission rules and classifier config. See [Permissions](/docs/using-mew/permissions/) |
| `secrets` | table | (empty) | Secret file globs and literal words to redact |
| `workspace` | table | (empty) | Allowed workspace roots for file tools and shell escape checks |
| `plugins` | table | (empty) | Per-plugin configuration keyed by plugin name. See [Plugins](/docs/using-mew/plugins/) |
| `tui` | table | (built-in) | Terminal theme and sidebar settings. See [Theming](/docs/using-mew/theming/) |
| `orchestration` | table | (built-in) | Subagent concurrency limits and reminders. See [Subagents](/docs/using-mew/subagents/) |

Set `tui.sidebar_finished_ttl_secs` to change how long failed activity stays
visible in the terminal sidebar. It defaults to 180 seconds; `0` hides failed
activity immediately.

:::note
Personas and skills accept `polytoken:` as an alias for `mew:` in frontmatter.
See [Personas](/docs/using-mew/personas/) for details.
:::

### Python code mode

Code mode replaces individual tool calls with a persistent `python` tool that
invokes Mew tools through a Python module.

#### Enabling and model defaults

By default, Mew enables code mode for DeepSeek models (case-insensitive `deepseek`
match across providers) and for Codex models whose catalog entry sets
`code_mode_only`. The Codex catalog loads for OAuth users or from a project
catalog override. Other models receive direct tools unless overridden.

```toml
[code_mode]
mode = "off"          # optional: "off" or "full"
isolated = true       # default
max_idle_workers = 3  # retained top-level Python workers per daemon
# python = "/path/to/venv/bin/python"

[code_mode.models]
"codex/gpt-5.6-luna" = "full"
"opencode-go/hy4-flash-free" = "full"
```

Precedence runs from `code_mode.models["provider/model"]`, to `code_mode.mode`,
to the model default. Omit `mode` to keep per-model defaults. Set `python` to a
virtual environment executable to expose its installed packages; Mew runs `python3`
from `PATH` by default. Only point `python` at a trusted interpreter and
environment.

#### Worker lifecycle and limits

- **State persistence**: variables persist across cells in the same session,
  including after an idle session unloads and reattaches while its worker stays
  cached.
- **Idle worker cache**: each daemon retains up to `max_idle_workers` (default
  `3`) idle top-level workers and evicts the least recently used worker when
  full. Running or approval-waiting calls are never evicted, and subagents keep
  separate workers outside this cap. Setting `0` stops each worker when its call
  ends. Restart the daemon after changing `max_idle_workers`.
- **Timeouts and resets**: workers stop after 1 hour of inactivity or a
  10-minute stall without a response (approval waits excluded). Daemon restarts,
  session deletion, cancellation, and worker crashes also drop Python state.
- **Loss notification**: after an idle eviction or timeout on a live daemon, the
  next `python` call reports the lost state before running code. Save durable
  data to workspace files with Mew tools.

#### In-cell API and TUI display

- **Helpers**: call tools with `tools.read({"path": "file.txt"})`, inspect
  definitions in `ALL_TOOLS`, return output with `text(value)`, and forward tool
  images to vision-capable models with `emit_image(result["images"][0])`.
  Configured MCP tools are also exposed under `tools`.
- **Threads**: Python threads call tools synchronously through the dispatcher
  and obey normal permission checks. Join spawned threads and await thread-pool
  tasks before a cell returns; work outliving a cell has later tool calls
  rejected, fails the cell, resets the worker, and discards late output.
- **TUI inspection**: press `Ctrl+O` to toggle Python cell expansion and view
  nested tool arguments. History retains nested call records and byte counts;
  emit values with `text(...)` to keep results in model context.

#### Sandboxing

With `isolated = true` (default), Mew starts Python inside macOS Seatbelt or
Linux Bubblewrap (`bwrap`, requiring unprivileged user namespaces). The sandbox
exposes the Python environment, system libraries, and a private scratch
directory while blocking direct workspace and network access. Tools called from
Python still enforce their normal permissions. Sandboxing relies on the host
kernel without CPU or memory quotas, and calls fail closed if the sandbox
cannot start. Some scientific packages fall back to serial execution when
restricted from spawning subprocesses.

Setting `isolated = false` grants Python direct host access to files, network,
and subprocesses outside Mew's permission engine, so Mew classifies every
unrestricted `python` call as `Dangerous`. Personas and subagents with tool
restrictions always enforce `isolated = true`, and disabling or omitting `python`
turns code mode off for that persona. Changing effective isolation stops the
existing worker after active cells finish and reports lost state on the next
call.

### Provider configuration

Configure each provider under `[providers.<id>]`:

```toml
[providers.deepseek]
shape = "openai"
base_url = "https://api.deepseek.com/v1"
credential_ref = "deepseek"
```

| Field | Required | Description |
|-------|----------|-------------|
| `shape` | Yes | Wire protocol: `"openai"`, `"anthropic"`, or `"responses"` |
| `base_url` | Direct providers | API endpoint URL |
| `credential_ref` | API-key providers | Credential lookup key. See [Credentials](#credentials) |
| `kind` | No | `"direct"` (default) or `"router"` |
| `nano` | No | Smallest router tier model (`provider/model`) |
| `micro` | Router providers | General-purpose router tier model (`provider/model`) |
| `deci` | Router providers | Highest-capability router tier model (`provider/model`) |
| `disable_hashline` | No | Set `true` to omit `edit_hashline` for this provider (default: `false`) |

Router providers map `nano`, `micro`, and `deci` tiers for subagents and the
permission classifier. Pick a direct provider for interactive chat.

```toml
[providers.router]
shape = "openai"
kind = "router"
micro = "deepseek/deepseek-v4-flash"
deci = "deepseek/deepseek-v4-pro"
```

Existing configs using `small` and `big` still work as fallbacks for `micro` and
`deci`. See [Providers](/docs/using-mew/providers/) for built-in providers and
routing rules.

### Custom models

Add `[[models]]` blocks to extend or override the models.dev catalog:

```toml
[[models]]
id = "custom-llama"
provider = "my-provider"
shape = "openai"
context_window = 32768
# Set this only when the provider documents the retention window.
prompt_cache_retention_secs = 14400

[[models.thinking_variants]]
name = "high"
params = { reasoning_effort = "high" }
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Model identifier |
| `provider` | string | Provider ID serving the model |
| `shape` | string | Adapter shape (defaults to the provider's shape) |
| `context_window` | integer | Context window in tokens |
| `prompt_cache_retention_secs` | integer | Documented prompt-cache retention in seconds |
| `thinking_variants` | array | Named thinking variants with provider parameters |
| `thinking_budget` | table | Token budget range for budget-based thinking models |
| `responses_lite` | boolean | Enable the Responses Lite transport for compatible models |
| `merge` | boolean | Keep existing catalog fields when overriding a model (default: `false`) |

### Secrets

Configure `secrets.files` globs to guard matching paths from reads and search
tools, and `secrets.words` values to redact literal secrets from tool output:

```toml
[[secrets.files]]
paths = [".env", "secrets.toml", "*.key"]

[[secrets.words]]
values = ["example-secret-value"]
```

See [Permissions](/docs/using-mew/permissions/) for how secret guards interact
with permission modes.

### Workspace

```toml
[workspace]
roots = ["~/code/myproject", "~/code/shared"]
```

`workspace.roots` restricts which directories the agent may access without
prompting. Empty `roots` default path checks to the current directory and turn
off the shell escape tier. Non-empty `roots` enable two checks:

1. **Path-based tools** (`read`, `write`, `edit_str_replace`, `edit_hashline`,
   `glob`, `grep`): when targeting a path outside `roots`, Mew prompts once for
   the containing directory and caches granted directories for the session.
   Denying the prompt rejects the tool call.
2. **Shell tools** (`bash`, `shell_background`, `shell_monitor`): commands
   referencing paths outside `roots` escalate from auto-allow to a permission
   prompt.

See [Permissions](/docs/using-mew/permissions/) for full rule evaluation order.

## Credentials

Mew resolves `credential_ref` in this order:

1. **Environment variable**: `MEW_CRED_<NORMALIZED_REF>`, uppercased with
   non-alphanumeric characters converted to `_`. For `credential_ref = "z-ai"`,
   set `MEW_CRED_Z_AI`.
2. **System keyring**: service `mew`, account matching `credential_ref`.
3. **`credentials.json`**: JSON map in the config directory, such as
   `{"deepseek": "sk-..."}`.

Use environment variables for temporary sessions or scripts, the system keyring
for persistent setups, and `credentials.json` when no keyring is available.

## Environment variables

| Variable | Description |
|----------|-------------|
| `MEW_CRED_<NAME>` | Credential value (non-alphanumeric characters become `_`) |
| `MEW_DEFAULT_MODEL` | Override `default_model` |
| `MEW_MAX_TURNS` | Override `max_turns` |
| `MEW_MAX_TURNS` | Override `max_turns` |
| `MEW_SESSION_DIR` | Override session storage directory |
| `MEW_DATA_DIR` | Override data directory (default: `~/.local/share/mew`) |
| `MEW_CACHE_DIR` | Override cache directory (default: `~/.cache/mew`) |
| `MEW_CONFIG_DIR` | Override config directory |
| `MEW_PERMISSIVE` | Start `chat`, `run`, or `daemon` in permissive mode |
| `MEW_AUTO` | Start `chat`, `run`, or `daemon` in auto mode |
| `MEW_AUTO_PLUS` | Start `chat`, `run`, or `daemon` in auto+ mode |
| `MEW_DANGEROUS` | Start `chat`, `run`, or `daemon` in dangerous mode |
| `RUST_LOG` | Tracing log filter (loaded from `.env` before logger init) |

Running bare `mew` starts in `standard` mode and ignores the four permission
environment variables; pass them with `mew chat`, `mew run`, or `mew daemon`.

## State persistence

`state.toml` persists CLI, TUI, and desktop UI preferences across restarts:

| Field | Description |
|-------|-------------|
| `last_model` | Last selected model ID |
| `last_provider` | Last selected provider ID |
| `last_thinking_variant` | Last selected thinking variant |
| `recent_models` | Up to six recently used `provider/model` entries |
| `theme` | Active theme name set via `/theme` |
| `sidebar_collapsed` | Collapsed state of terminal sidebar sections |
| `disabled_plugins` | Disabled plugin names |
| `revoked_extensions` | Revoked extension names |

Native desktop layout preferences (`desktop_*` and `terminal_font`) also live
in `state.toml`. Model selection follows `--model`, then `state.toml`
(`last_model`), then `config.toml` (`default_model`), and finally the built-in
default (`deepseek-v4-flash`). Switching models or providers in a session writes
the choice back to `state.toml`.

## MCP servers

Configure stdio and streamable HTTP servers in `mcp.json`. See
[MCP Servers](/docs/using-mew/mcp-servers/) for discovery, permissions, and
timeouts.
