---
title: Architecture
description: Internal architecture and crate structure for mew contributors.
---

mew is a three-layer pipeline: **TUI → Agent → Provider**.

## Crate map

```
mew (binary)
  ├─ mew-tui        event loop, ratatui UI, App state
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
  │    │    per-connection session, ID-paired permission/ask requests via mew-protocol.
  │    ├─ mew-protocol  Wire message types + JSON codec (ClientMessage/ServerMessage).
  │    ├─ mew-web-bridge  TCP+WS listener that relays browser WS connections to the
  │    │    daemon's Unix socket. Also serves the chat UI's static assets.
  │    └─ mew-skills    skill discovery + loading from .mew/skills, .opencode/skills, etc.
  └─ mew-context   discovers AGENTS.md / CLAUDE.md files from cwd up to git root
```

## Shared crates

| Crate | Purpose |
|-------|---------|
| `mew-message` | Canonical message/part types (Message, Part, Role, Finish, ToolState) |
| `mew-config` | config.toml + credential resolution + permission rule engine |
| `mew-hooks` | Dispatcher trait: pluggable hooks for observability, mutation, permissions |
| `mew-session` | JSONL session persistence (`~/.local/share/mew/sessions/<id>.jsonl`) |
| `mew-catalog` | models.dev catalog with 24-hour cache (pricing, shapes, context windows) |
| `ratatui-mdstream` | Streaming markdown to ratatui Lines with syntect highlighting |

## Event flow

1. User presses Enter → `Action::Submit` → `agent.run(prompt)` returns `mpsc::Receiver<AgentEvent>`
2. `forward_agent_events` pumps that receiver into the TUI's main event channel
3. TUI `recv()` loop processes events → `app.handle_agent_event()` updates App state → `draw()`
4. Inside the agent: `turn_loop` streams provider events, collects tool calls from
   `MessageEnd(ToolUse)`, executes them sequentially, then loops back for the next LLM turn

The drain loop after each primary event coalesces Agent/Tick events but caps agent
events at 4 per frame during streaming so text appears incrementally.

## Message model

`Message` contains a `Vec<Part>`. Parts are `Text | Reasoning | ToolCall | ToolResult | Error`.
`ToolCallPart` carries a `ToolState` enum (`Pending → Running → Completed | Error`) updated
live as the tool executes.

The TUI's `app.messages` is the **display** store. The agent's `self.messages` is the
**API history** store. They're separate. In the TUI, all parts from a multi-turn agentic
loop (text → tool calls → follow-up text) are merged into one assistant message display entry.

## Streaming markdown

`app.md_stream` / `app.md_state` track the currently-streaming text part. Only the
**last** `Part::Text` in the active message uses `render_streaming(md_state)`. Earlier
text parts (preceding tool calls) use the static cached path. On `MessageEnd`, the stream
is finalized and `pending_md_rerender` triggers a full re-render from `tp.text` on the
next frame.
