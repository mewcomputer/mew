---
title: Plugins
description: Manage installed Mew plugins and inspect the experimental hook runtime.
---

Plugins are standalone executable programs that communicate with Mew over
stdin/stdout JSON-RPC. They predate [extension packages](/docs/using-mew/extensions/),
which add `mew-ext.toml` manifests, macOS Seatbelt sandboxing, and capability
consent.

The chat daemon and `mew run` currently wire a no-op hook dispatcher, so
managing executables with `mew ext` does not load plugins during chat sessions
or one-shot runs. The capabilities and consent sections below describe the
experimental hook runtime.

## Discovery

The experimental loader scans executable files alphabetically in:

1. `~/.config/mew/plugins/` (global)
2. `.mew/plugins/` (project-local)

Compiled dynamic library files (`.wasm`, `.dylib`, `.so`, `.dll`) are ignored.

## Managing plugins

Place an executable in `~/.config/mew/plugins/` (or `.mew/plugins/`) to include
it in `mew ext list`:

```sh
mkdir -p ~/.config/mew/plugins
cp my-plugin ~/.config/mew/plugins/
chmod +x ~/.config/mew/plugins/my-plugin
```

Disable or re-enable a plugin without deleting it:

```sh
mew ext disable <name>
mew ext enable <name>
```

Mew records disabled plugin names in `state.toml` under `disabled_plugins`.

## Experimental hook capabilities

When the hook runtime is wired, plugins can:

- Register dynamic `Mutating` tools and custom slash commands.
- Modify chat parameters (`temperature`, `max_tokens`, `top_p`, `tool_choice`),
  HTTP headers, and system prompts before turns.
- Block or rewrite tool inputs, outputs, permission decisions, and shell
  environment variables.
- Observe provider events for telemetry.
- Persist per-plugin key-value storage to disk.
- Push TUI toast notifications or custom text beside the input box.

## Consent

On first load, a bare plugin prompts once for full access (all hooks and host
functions); declining restricts it to observe-only mode. In non-interactive runs,
new plugins default to observe-only. Mew persists consent choices across runs.

See [Extensions](/docs/using-mew/extensions/) for a comparison between bare
plugins and extension packages, and [Extension System Internals](/docs/development/dev-extensions/)
for the JSON-RPC protocol and hook reference.
