---
title: Plugins
description: Extend mew with external plugins via the subprocess dispatcher.
---

mew has a plugin system that lets external programs hook into the agent
lifecycle. Plugins are standalone executables that communicate with mew
via stdin/stdout using newline-delimited JSON-RPC 2.0.

## How it works

The `Dispatcher` trait in `mew-hooks` exposes hook points covering the
full agent lifecycle. `NopDispatcher` is the default (all passthrough).
`mew-hooks-runtime` provides `SubprocessDispatcher`, which spawns plugins
as subprocesses and relays hook calls to them.

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

## What plugins can do

- **Register dynamic tools** that appear in the model's tool list
- **Register slash commands** that show up in autocomplete and `/help`
- **Modify chat params** (temperature, max_tokens, tool_choice) per turn
- **Inject HTTP headers** into provider requests
- **Modify the system prompt** before each turn
- **Block or modify tool inputs** before execution
- **Modify tool outputs** after execution
- **Auto-approve or deny permissions** based on custom rules
- **Inject environment variables** into bash commands
- **Observe** all provider events for telemetry/metrics
- **Store per-plugin key-value data** (persisted to disk)
- **Push notifications** to the TUI (toast alerts)
- **Render custom UI** beside the input area

## Plugin capabilities

`PluginHost` gives plugins access to:

- **Config read**: restricted subset of `config.toml` (plugin section only)
- **Key-value storage**: per-plugin, persisted to disk
- **Notify channel**: push toast notifications to the TUI
- **set_ui**: render custom text content beside the input area

## Writing a plugin

A plugin is any executable that reads JSON-RPC 2.0 messages from stdin
and writes responses to stdout. Each message is one JSON object per line.

The protocol follows the standard JSON-RPC 2.0 format: `method` + `params`
for requests, `result` or `error` for responses.

## Configuring plugins

Plugins are discovered from the standard plugin directories. The settings
page (`Ctrl+P` → "Settings") shows discovered plugins and lets you enable
or disable them.

## Permission modes

Permission prompts can be automated or bypassed entirely. See
[Permissions](/docs/permissions/) for the full mode reference, rules,
workspace sandboxing, and secret redaction.
