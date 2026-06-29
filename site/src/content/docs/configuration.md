---
title: Configuration
description: Configure mew providers, credentials, and settings.
---

mew is configured via `~/.config/mew/config.toml` (macOS:
`~/Library/Application Support/computer.mew.mew/config.toml`).

## Config sources (later wins)

1. Built-in provider defaults (`opencode-zen`, `opencode-go`, `z-ai`, `deepseek`, `umans`)
2. `config.toml`
3. Environment variables with `MEW_` prefix (`MEW_DEFAULT_MODEL`,
   `MEW_WORKSPACE__ROOTS`, `__` = nested path)

A `.env` file in your working directory is loaded at startup (via `dotenvy`),
so `RUST_LOG` and `MEW_CRED_*` vars work there.

## Credentials

Credential resolution order for `credential_ref`:

1. Env var `MEW_CRED_<REF_UPPERCASED>` (hyphens → underscores)
2. System keyring (`mew` service, account = ref name)
3. `credentials.json` in the config directory

## Example config

```toml
default_model = "deepseek-v4-flash"

[providers.deepseek]
shape = "openai"
base_url = "https://api.deepseek.com/v1"
credential_ref = "DEEPSEEK"
model = "deepseek-v4-flash"

[workspace]
roots = ["~/code"]
```

## Workspace sandboxing

`workspace.roots` is a list of directories the agent is allowed to touch.
It feeds two layers:

- **Agent layer**: enforced by `ensure_workspace_path` before any path-based tool runs.
- **Escape tier**: shell commands that resolve paths outside the configured
  roots are escalated from `AllowOnce` to `Prompt`.

Empty `workspace_roots` disables the escape tier.

## State persistence

Last-used model/provider is persisted to `state.toml` and restored on next
launch. CLI `--provider`/`--model` flags override state, which overrides the
built-in default.

## MCP servers

MCP servers are loaded from `mcp.json` in the working directory. The code
also checks `.mcp.json`, `.mew/mcp.json`, and `.mew/.mcp.json`:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    }
  }
}
```
