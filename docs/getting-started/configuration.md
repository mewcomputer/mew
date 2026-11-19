---
title: Configuration
description: Configure mew providers, credentials, permissions, and workspace.
---

Configure Mew with `config.toml` and environment variables. Mew saves
runtime preferences in `state.toml`. Run `mew config show` to inspect
the effective configuration, or `mew config edit` to edit the file.

## File locations

| File | Path | Purpose |
|------|------|---------|
| `config.toml` | `~/.config/mew/config.toml` | Main config: providers, permissions, secrets, workspace |
| `state.toml` | Same directory | Runtime state: last model, sidebar state, disabled plugins |
| `credentials.json` | Same directory | Credential fallback (keyring and env vars preferred) |
| `.env` | Working directory | Loaded at startup via dotenvy (for `RUST_LOG`, `MEW_CRED_*`) |

Override the config directory by setting `MEW_CONFIG_DIR` or
`XDG_CONFIG_HOME`.

## Config sources (later wins)

1. Built-in defaults (see [Providers](/docs/using-mew/providers/) for provider IDs)
2. `config.toml` (overrides built-in providers, adds custom providers)
3. Environment variables with the `MEW_` prefix. A double underscore
   denotes a nested key, as in `MEW_TUI__THEME`. Set lists such as
   `workspace.roots` in `config.toml`.

The CLI loads a `.env` file at process startup. A daemon started earlier
keeps its own environment when you open or resume a conversation.

## Configuration reference

### Top-level fields

```toml
default_model = "deepseek-v4-flash"
default_persona = "builder"
plan_path = "plans/"

[workspace]
roots = ["~/code"]

[tui]
sidebar_finished_ttl_secs = 180
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_model` | string | (empty) | Model ID used when no `--model` flag or state entry exists |
| `default_persona` | string | `"builder"` | Persona loaded at startup. `"none"` or `"default"` starts without one |
| `plan_path` | string | `"plans/"` | Each `write_plan` call creates a new file such as `plans/vale-nori.md`. Use the returned path with `edit_plan`. Set a different relative or absolute path to use a fixed file |
| `providers` | table | (built-in) | Provider definitions keyed by provider ID |
| `models` | array | `[]` | Custom model entries that override or extend the catalog |
| `code_mode` | table | (off) | Run tools through a persistent Python session. See [Python code mode](#python-code-mode) |
| `permissions` | table | (empty) | Permission rules and classifier config. See [Permissions](/docs/using-mew/permissions/) |
| `secrets` | table | (empty) | Secret files and words for redaction |
| `workspace` | table | (empty) | Workspace roots for file access and shell permission checks |
| `plugins` | table | (empty) | Configuration by plugin name. See [Plugins](/docs/using-mew/plugins/) |
| `tui` | table | (built-in) | Terminal theme and sidebar settings. See [Theming](/docs/using-mew/theming/) |
| `orchestration` | table | (built-in) | Subagent limits and reminders. See [Subagents](/docs/using-mew/subagents/) |

Set `tui.sidebar_finished_ttl_secs` to change how long failed activity
stays visible in the terminal sidebar. It defaults to 180 seconds.
A value of 0 hides failed activity immediately.

:::note
Personas and skills accept `polytoken:` as an alias for `mew:` in their
frontmatter. This is for compatibility with personas authored for
Polytoken. See [Personas](/docs/using-mew/personas/) for details.
:::

### Python code mode

Code mode gives the model a single `python` tool. Python runs in a persistent
session and calls Mew's available tools through `tools`. The default is `off`,
which keeps the existing tool interface. Choose a default and optional
provider-qualified overrides in `config.toml`:

```toml
[code_mode]
mode = "off"       # "off" or "full"
isolated = true    # default
# python = "/path/to/venv/bin/python"

[code_mode.models]
"codex/gpt-6-luna" = "full"
"opencode-go/hy4-flash-free" = "off"
```

Set `mode = "full"` to enable it for every model, then use `"off"` overrides
for exceptions. `python` defaults to `python3` on the daemon's `PATH`. Use a
virtual environment's Python executable to make its installed packages
available. The interpreter and its installed packages must be trusted.

Python variables persist across calls in the same session, including when an
idle session is unloaded and reattached. They do not survive daemon restarts,
session deletion, cancellation, a worker crash, or a ten-minute period without
a worker response. Approval waits do not count toward that timeout. Subagents
have separate Python sessions.

The model can call tools with `tools.read({"path": "file.txt"})`, inspect
their definitions in `ALL_TOOLS`, and use `text(value)` to return output.
`emit_image(result["images"][0])` returns a tool's image to a vision-capable
model. Calls from Python threads are serialized through the tool dispatcher.
Nested tool calls use the same permission checks as direct calls.
Mew does not yet register MCP servers; see [MCP Servers](/docs/using-mew/mcp-servers/).

With `isolated = true`, Mew runs Python in a macOS Seatbelt sandbox or a Linux
Bubblewrap sandbox. Linux requires `bwrap` and working unprivileged user
namespaces. The worker can read the selected Python environment and system
libraries and use a private scratch directory. Other workspace files and the
network are inaccessible directly.
Access through Mew tools remains governed by their permissions. This uses the
host kernel and does not impose CPU or memory quotas. If isolation cannot start,
the call fails instead of running without it.

Some scientific packages fall back to serial execution when the sandbox
restricts subprocesses or synchronization primitives.

Setting `isolated = false` lets Python access the host directly, including files,
network, and subprocesses, without passing those operations through Mew's tool
permissions. Mew marks each unrestricted Python call as dangerous for the
permission engine.

### Provider configuration

Each provider is a key under `[providers]`:

```toml
[providers.deepseek]
shape = "openai"
base_url = "https://api.deepseek.com/v1"
credential_ref = "deepseek"
```

| Field | Required | Description |
|-------|----------|-------------|
| `shape` | yes | Adapter protocol (`"openai"`, `"anthropic"`, or `"responses"`) |
| `base_url` | direct API providers | API endpoint URL |
| `credential_ref` | direct providers using API keys | Credential name. See [Credentials](#credentials) |
| `kind` | no | `"direct"` (default) or `"router"` |
| `nano` | no | Least expensive router model |
| `micro` | router | General-purpose router model |
| `deci` | router | Most capable router model |
| `disable_hashline` | no | If `true`, do not register the `edit_hashline` tool for this provider (default: `false`) |

Router providers group model tiers for subagents and the permission
classifier. Choose a direct provider for chat. A tier uses
`provider/model` syntax:

```toml
[providers.router]
shape = "openai"
kind = "router"
micro = "deepseek/deepseek-v4-flash"
deci = "deepseek/deepseek-v4-pro"
```

Existing router configurations can use `small` and `big` as fallbacks
for `micro` and `deci`. See [Providers](/docs/using-mew/providers/) for routing
behavior and built-in providers.

### Custom models

Override or extend the models.dev catalog with `[[models]]` entries:

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
| `provider` | string | Provider ID that serves this model |
| `shape` | string | Adapter shape (optional, inferred from provider if omitted) |
| `context_window` | integer | Context window in tokens |
| `prompt_cache_retention_secs` | integer | Known prompt-cache retention in seconds; omit when unknown |
| `thinking_variants` | array | Named thinking variants with provider-specific params |
| `thinking_budget` | table | Token budget range for models that accept a thinking budget |
| `responses_lite` | boolean | Use the Responses Lite transport for compatible models |
| `merge` | boolean | Keep catalog values for omitted fields when overriding an existing model (default: `false`) |

### Secrets

Add file patterns under `secrets.files` to guard reads and hide matching
paths from search tools. Add literal values under `secrets.words` to redact
them from supported tool output.

```toml
[[secrets.files]]
paths = [".env", "secrets.toml", "*.key"]

[[secrets.words]]
values = ["example-secret-value"]
```

See [Permissions](/docs/using-mew/permissions/) for how the secret-file
guard interacts with permission rules and modes.

### Workspace

```toml
[workspace]
roots = ["~/code/myproject", "~/code/shared"]
```

Directories the agent is allowed to touch. Defaults to the current
directory when empty. Feeds two enforcement layers:

1. **Agent layer**: path-based tools (read, write, edit, glob, grep)
   reject paths outside the roots before running.
2. **Escape tier**: bash commands with path arguments outside the roots
   escalate from auto-allow to a prompt.

Empty roots disable the escape tier. See
[Permissions](/docs/using-mew/permissions/) for the full sandboxing details.

## Credentials

Credential resolution for `credential_ref` follows this order:

1. **Env var**: `MEW_CRED_<REF_UPPERCASED>` (hyphens become underscores).
   For `credential_ref = "deepseek"`, set `MEW_CRED_DEEPSEEK`.
2. **System keyring**: `mew` service, account = ref name
3. **credentials.json**: `{"deepseek": "sk-..."}` in the config directory

Env vars are the fastest path for development. Keyring is best for
persistent setups. `credentials.json` works when neither is available.

## Environment variables

| Variable | Description |
|----------|-------------|
| `MEW_CRED_<NAME>` | Set a credential value (hyphens in name become underscores) |
| `MEW_DEFAULT_MODEL` | Override `default_model` |
| `MEW_SESSION_DIR` | Override session storage directory |
| `MEW_DATA_DIR` | Override persistent data directory (XDG default: `~/.local/share/mew`) |
| `MEW_CACHE_DIR` | Override cache directory (XDG default: `~/.cache/mew`) |
| `MEW_CONFIG_DIR` | Override config directory |
| `MEW_PERMISSIVE` | Start in permissive mode |
| `MEW_AUTO` | Start in auto mode |
| `MEW_AUTO_PLUS` | Start in auto+ mode |
| `MEW_DANGEROUS` | Start in dangerous mode (skip all prompts) |
| `RUST_LOG` | Log level (loaded from `.env` before tracing init) |

## State persistence

`state.toml` stores runtime state between sessions:

| Field | Description |
|-------|-------------|
| `last_model` | Last-used model ID |
| `last_provider` | Last-used provider ID |
| `sidebar_collapsed` | Sidebar section collapse state |
| `disabled_plugins` | Plugins the user has disabled |

CLI `--provider` and `--model` flags override state, which overrides
the built-in default. The last-used model and provider are saved back to
`state.toml` whenever you switch.

## MCP servers

MCP support is experimental library functionality. See
[MCP Servers](/docs/using-mew/mcp-servers/) for the library and transport
configuration.
