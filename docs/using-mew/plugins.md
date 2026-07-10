---
title: Plugins
description: How to use and manage mew plugins.
---

Plugins are external programs that extend mew's capabilities. They can
register tools, add slash commands, modify requests, observe events, and
push UI updates. Plugins are the original extension format — the newer
[extension package](/docs/using-mew/extensions/) format adds manifests,
sandboxing, and consent, but bare plugins continue to work.

## Where plugins live

Plugins are discovered as executable files in these directories:

1. `~/.config/mew/plugins/` (global — loads for every session)
2. `.mew/plugins/` in your project directory (project-local)

Any executable file in these directories is loaded automatically on
startup. Plugins are loaded in alphabetical order by filename.

## Managing plugins

There's no `mew plugin install` command — just copy the executable into
the plugins directory:

```bash
cp my-plugin ~/.config/mew/plugins/
chmod +x ~/.config/mew/plugins/my-plugin
```

To disable a plugin without removing it:

```bash
mew ext disable <name>
```

To re-enable:

```bash
mew ext enable <name>
```

Disabled plugins are tracked in `state.toml` under `disabled_plugins`.

## What plugins can do

- Register dynamic tools that appear in the model's tool list
- Register slash commands that show up in autocomplete and `/help`
- Modify chat parameters (temperature, max_tokens, tool_choice) per turn
- Inject HTTP headers into provider requests
- Modify the system prompt before each turn
- Block or modify tool inputs before execution
- Modify tool outputs after execution
- Auto-approve or deny permissions based on custom rules
- Inject environment variables into bash commands
- Observe all provider events for telemetry and metrics
- Store per-plugin key-value data persisted to disk
- Push toast notifications to the TUI
- Render custom text content beside the input area

## Consent

On first load, bare plugins prompt for full access (all hooks and host
functions). Approve to grant full access, or decline to restrict the
plugin to observe-only mode. In non-interactive mode, new plugins are
auto-restricted to observe-only.

Your choice is persisted and remembered on subsequent loads.

## Plugins vs extension packages

| Feature | Bare plugins | Extension packages |
|---------|-------------|-------------------|
| Manifest | None | `mew-ext.toml` required |
| Sandbox | No | Yes (macOS) |
| Consent | Full-access prompt | Capability-based prompts |
| Install | Manual file copy | `mew ext install` |
| Discovery | `plugins/` dir | `extensions/` dir |

Bare plugins are being superseded by extension packages. A future release
will add deprecation warnings for bare plugins. See
[Extensions](/docs/using-mew/extensions/) for the newer format.

For the JSON-RPC protocol, hook point reference, and host function API,
see [Extension System Internals](/docs/development/dev-extensions/).
