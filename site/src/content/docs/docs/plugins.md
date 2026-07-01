---
title: Plugins
description: Extend mew with external plugins via the subprocess dispatcher.
---

Plugins are external programs that hook into the agent lifecycle. They
can register tools, add slash commands, modify requests, observe events,
and push UI updates. A plugin is any executable that communicates over
stdin/stdout using newline-delimited JSON-RPC 2.0.

## How it works

The `Dispatcher` trait in `mew-hooks` defines hook points covering the
full agent lifecycle. `NopDispatcher` is the default (all passthrough).
`mew-hooks-runtime` provides `SubprocessDispatcher`, which spawns each
plugin as a subprocess and relays hook calls to it over JSON-RPC.

The host sends hook calls to the plugin. The plugin responds with a
result. The plugin can also call host functions (notify, storage, UI)
using the same protocol in reverse.

```
Host (mew)                          Plugin (subprocess)
    │                                      │
    ├── hook call ──────────────────────→  │  {"jsonrpc":"2.0","method":"on_chat_message","params":{...},"id":1}
    │                                      │
    │  ←────────────────────────────────── result
    │  {"jsonrpc":"2.0","result":{...},"id":1}
    │                                      │
    │  ←────────────────────────────────── host function call
    │  {"jsonrpc":"2.0","method":"host-notify","params":{"message":"hi"},"id":2}
    │                                      │
    ├── host result ─────────────────────→  │  {"jsonrpc":"2.0","result":"ok","id":2}
```

## Discovery

Plugins are discovered as executables in these directories:

1. `~/.config/mew/plugins/` (global)
2. `.mew/plugins/` in the current working directory (project)

Any executable file in these directories is loaded as a plugin. Plugins
are sorted alphabetically by filename for deterministic hook ordering.

## Hook points

Plugins can hook into any of these lifecycle events:

| Hook | When it fires | Can mutate? |
|------|---------------|-------------|
| `init` | Plugin loaded at startup | No |
| `shutdown` | Agent stopping | No |
| `on_register_tools` | Startup, before first turn | Registers dynamic tools |
| `on_register_slash_commands` | Startup, before first turn | Registers slash commands |
| `execute_slash_command` | User runs a plugin-registered command | Returns text output |
| `on_provider_event` | Every provider streaming event | No (observation only) |
| `on_tool_error` | A tool returned an error | No |
| `on_turn_end` | A turn finished | No |
| `on_chat_message` | Before each message is sent to the provider | Yes (returns modified message) |
| `on_chat_params` | Before building the request | Yes (temperature, max_tokens, tool_choice) |
| `on_chat_headers` | Before sending the request | Yes (HTTP headers) |
| `on_system_prompt` | After system prompt is assembled | Yes (returns modified prompt) |
| `on_tool_execute_before` | Before a tool runs | Yes (can block or modify input) |
| `on_tool_execute_after` | After a tool runs | Yes (can modify output) |
| `on_permission_ask` | When a permission prompt is shown | Yes (can auto-allow/deny) |
| `on_shell_env` | Before bash runs | Yes (environment variables) |
| `on_pre_compaction` | Before context compaction | No |
| `on_post_compaction` | After context compaction | No |
| `on_subagent_start` | A subagent was spawned | No |
| `on_subagent_end` | A subagent finished | No |
| `on_stop` | Agent is shutting down | No |

## Host functions

Plugins can call back into the host using these methods:

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `host-notify` | `message` | `"ok"` | Push a toast notification to the TUI |
| `host-log` | `message` | `"ok"` | Write to the mew log |
| `host-config-read` | `key` | config value | Read a key from the plugin's config section |
| `host-storage-read` | `key` | stored value | Read from per-plugin persistent storage |
| `host-storage-write` | `key`, `value` | `"ok"` | Write to per-plugin persistent storage |
| `host-storage-delete` | `key` | `"ok"` | Delete from persistent storage |
| `host-set-ui` | `key`, `value` | `"ok"` | Render custom text beside the input area |

Storage is per-plugin and persisted to disk. Config reads are scoped to
the plugin's section in `config.toml`.

## Writing a plugin

A plugin reads JSON-RPC 2.0 messages from stdin and writes responses to
stdout. Each message is one JSON object per line. Here's a minimal plugin
in Python that logs every turn:

```python
#!/usr/bin/env python3
import sys, json

def handle(method, params, msg_id):
    if method == "init":
        return {"status": "ready"}
    elif method == "on_turn_end":
        # Log turn completion
        turns = params.get("turn_count", "?")
        print(f"turn {turns} done", file=sys.stderr)
        return None  # no mutation
    elif method == "on_register_slash_commands":
        return [{"name": "/ping", "description": "Test plugin"}]
    elif method == "execute_slash_command":
        if params.get("command") == "/ping":
            return "pong"
        return None
    return None

for line in sys.stdin:
    try:
        msg = json.loads(line)
        result = handle(msg.get("method"), msg.get("params", {}), msg.get("id"))
        if "id" in msg:
            response = {"jsonrpc": "2.0", "id": msg["id"]}
            if result is not None:
                response["result"] = result
            else:
                response["result"] = None
            print(json.dumps(response), flush=True)
    except Exception as e:
        if "id" in msg:
            print(json.dumps({
                "jsonrpc": "2.0",
                "id": msg["id"],
                "error": {"code": -32603, "message": str(e)}
            }), flush=True)
```

Save it as `~/.config/mew/plugins/turn-logger`, make it executable
(`chmod +x`), and mew will discover and load it on next start.

### Key rules

- **One JSON object per line.** No pretty-printing, no multiline JSON.
- **Always flush stdout.** The host reads line by line. Unflushed output
  blocks the protocol.
- **Respond to every request with an `id`.** Use the same `id` you
  received. For notification hooks (no `id`), no response is needed.
- **Return `null` for pass-through.** When a mutating hook doesn't want
  to change anything, return `null` (or `None`).
- **Write logs to stderr.** stdout is reserved for the JSON-RPC protocol.
  stderr goes to the mew log file.

## Configuring plugins

Plugins are auto-discovered from the plugin directories. No config file
entry is needed. Disabled plugins are tracked in `state.toml` under
`disabled_plugins`.

## What plugins can do

- Register dynamic tools that appear in the model's tool list
- Register slash commands that show up in autocomplete and `/help`
- Modify chat params (temperature, max_tokens, tool_choice) per turn
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
