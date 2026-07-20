---
title: Configuration
description: Configure mew providers, credentials, permissions, and workspace.
---

mew is configured through `config.toml`, environment variables, and a
runtime `state.toml`. This page covers every configurable field.

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

1. Built-in provider defaults (`opencode-zen`, `opencode-go`, `z-ai`,
   `deepseek`, `umans`)
2. `config.toml` (overrides built-in providers, adds custom providers)
3. Environment variables with `MEW_` prefix (`MEW_DEFAULT_MODEL`,
   `MEW_WORKSPACE__ROOTS`, `__` = nested path)

A `.env` file in your working directory is loaded at startup before
anything else, so `RUST_LOG` and `MEW_CRED_*` vars work from there.

## Full config reference

### Top-level fields

```toml
default_model = "deepseek-v4-flash"
default_persona = "builder"
plan_path = "PLAN.md"

[providers.deepseek]
# ... see below

[workspace]
roots = ["~/code"]

[permissions]
# ... see Permissions

[secrets]
# ... see below

[[models]]
# ... see below
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_model` | string | (empty) | Model ID used when no `--model` flag or state entry exists |
| `default_persona` | string | `"builder"` | Persona loaded at startup. `"none"` or `"default"` starts without one |
| `plan_path` | string | `"PLAN.md"` | Where the planner persona writes and builder reads. Relative or absolute |
| `providers` | table | (built-in) | Provider definitions keyed by provider ID |
| `models` | array | `[]` | Custom model entries that override or extend the catalog |
| `permissions` | table | (empty) | Permission rules and classifier config. See [Permissions](/docs/using-mew/permissions/) |
| `secrets` | table | (empty) | Secret files and words for redaction |
| `workspace` | table | (empty) | Workspace roots for sandboxing |

:::note
Personas and skills accept `polytoken:` as an alias for `mew:` in their
frontmatter. This is for compatibility with personas authored for
Polytoken. See [Personas](/docs/using-mew/personas/) for details.
:::

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
| `shape` | yes | Adapter protocol: `"openai"` or `"anthropic"` |
| `base_url` | yes | API endpoint URL |
| `credential_ref` | yes | Credential name (resolved via env, keyring, or credentials.json) |
| `kind` | no | `"direct"` (default) or `"router"` |
| `small` | no | Router: small model ID for simple turns |
| `big` | no | Router: big model ID for complex turns |
| `disable_hashline` | no | If `true`, do not register the `edit_hashline` tool for this provider (default: `false`) |

Router providers wrap two models behind one entry:

```toml
[providers.smart]
shape = "openai"
kind = "router"
small = "deepseek/deepseek-v4-flash"
big = "z-ai/glm-4.5-air"
credential_ref = "deepseek"
```

See [Providers](/docs/using-mew/providers/) for the full router behavior and
built-in provider list.

### Custom models

Override or extend the models.dev catalog with `[[models]]` entries:

```toml
[[models]]
id = "custom-llama"
provider = "my-provider"
shape = "openai"
context_window = 32768

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
| `thinking_variants` | array | Named thinking variants with provider-specific params |

### Secrets

Files listed under `secrets.files` are guarded: reading them forces a
permission prompt. Their contents are also redacted from all tool output.

```toml
[[secrets.files]]
paths = [".env", "secrets.toml", "*.key"]

[[secrets.words]]
words = ["my-api-key-value", "sk-1234567890"]
```

Pattern-based redaction (API keys, tokens) is always on. File-based and
word-based redaction add to this. See [Permissions](/docs/using-mew/permissions/)
for how the secret-file guard interacts with permission rules.

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
| `MEW_WORKSPACE__ROOTS` | Override workspace roots (`__` = nested path, comma-separated) |
| `MEW_SESSION_DIR` | Override session storage directory |
| `MEW_CONFIG_DIR` | Override config directory |
| `MEW_PERMISSIVE` | Start in permissive mode |
| `MEW_AUTO` | Start in auto mode |
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

MCP servers are configured in `mcp.json` (not `config.toml`). The code
checks these locations in order:

1. `mcp.json`
2. `.mcp.json`
3. `.mew/mcp.json`
4. `.mew/.mcp.json`

See [MCP Servers](/docs/using-mew/mcp-servers/) for the format and transport
options.
