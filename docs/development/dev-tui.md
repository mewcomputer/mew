---
title: TUI Architecture
description: How the ratatui frontend handles input, daemon updates, and rendering.
---

The terminal UI is a ratatui client for `mew-daemon`. Plain `mew chat`
starts a daemon on a local TCP port. Use `mew chat --connect <url>` to select a
running daemon and add `--attach <session-id>` to open an existing session.

This doc covers the TUI's event loop, display state, streaming markdown, and
how a keystroke becomes a provider stream. For the daemon and session model,
see [Architecture](/docs/development/dev-architecture/). For recording captures
against a live daemon, see [TUI capture methods](/docs/development/dev-tui-capture/).

## The pipeline

```
Keyboard → crossterm EventStream → EventLoop → Action
  → runtime::dispatch::handle_action → DaemonTarget → TuiClient
  → daemon WebSocket → session → agent → provider

Daemon events → TuiClient and ClientEngine → App → ratatui
```

## The event loop

`crates/mew-tui/src/events.rs` defines `EventLoop`. Its crossterm
reader sends keyboard and mouse events over a bounded channel. A second task
sends ticks every 16 milliseconds. The daemon client actor sends updates over
a separate channel.

## The main loop

`chat_with_daemon` in `crates/mew/src/commands/tui.rs` waits for terminal
input, ticks, and client updates. It applies daemon updates and pending
permission or question requests to `App`. The input drain coalesces rapid
events and queues each action for `runtime::dispatch::handle_action`.
Rendering skips idle ticks when `App` has nothing to redraw.

## How a keystroke becomes a provider stream

1. `handle_input_event` turns a submit keystroke into `Action::Submit`.
2. The dispatcher resolves mentions and attachments and gives the prompt
   a request ID for the confirmed session.
3. `DaemonTarget` passes the scoped submission through `TuiClient`.
4. The daemon accepts the prompt, serializes the turn, and streams events
   back over WebSocket.
5. `TuiClient` uses the shared `mew-client-core::ClientEngine` to project
   those events. The TUI applies the changes to `App` and renders.

## Display store vs API history store

Two separate message stores exist:

- **`app.messages`** (display): what the TUI renders. All parts from a
  multi-turn agentic loop (text, tool calls, follow-up text) merge into
  one assistant message entry. Synthetic messages (alerts, cost reports)
  live here too.
- **`agent.messages`** (API history): what gets sent to the provider. Each
  provider turn produces a separate assistant `Message`. Tool calls and
  results are separate parts. This is the canonical conversation state
  persisted to disk.

The display store is rebuilt from the API history on session resume. The
streaming markdown cache (`rendered_md_cache`) maps message IDs to rendered
ratatui Lines, invalidated when the terminal width changes.

## Streaming markdown

`app.md_stream` / `app.md_state` track the currently-streaming text part:

- The **last** `Part::Text` in the active message uses `render_streaming(md_state)`
  for incremental rendering.
- Earlier text parts (before tool calls in the same message) use the cached
  path: `render_markdown(tp.text, md_width, theme)`.
- On `MessageEnd`, the stream is finalized and `pending_md_rerender` triggers
  a full re-render from `tp.text` on the next frame.
- Cache invalidation: `rendered_md_cache` is cleared when `md_width` changes
  (terminal resize).

## Related crates

| Crate | Purpose | Key Types |
|-------|---------|-----------|
| `mew-tui` | Event loop, ratatui UI, App state | `Event`, `EventLoop`, `App`, `Action` |
| `ratatui-mdstream` | Streaming markdown to ratatui Lines | `MdStream`, `DocumentState` |

See [Architecture](/docs/development/dev-architecture/) for the daemon,
session model, and provider pipeline.
