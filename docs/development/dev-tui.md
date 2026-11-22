---
title: TUI Architecture
description: How the ratatui frontend handles input, daemon updates, and rendering.
---

The terminal UI is a ratatui client for `mew-daemon`. Plain `mew chat` connects
to an existing local daemon or starts one on a loopback TCP port. Pass
`--connect <url>` to attach to a specific daemon and `--attach <session-id>` to
resume an existing session.

For the shared daemon and provider pipeline, see
[Architecture](/docs/development/dev-architecture/). For headless TUI captures,
see [TUI capture methods](/docs/development/dev-tui-capture/).

## The pipeline

```text
Keyboard -> crossterm EventStream -> EventLoop -> Action
  -> runtime::dispatch::handle_action -> DaemonTarget -> TuiClient
  -> daemon WebSocket -> session -> agent -> provider

Daemon events -> TuiClient + ClientEngine -> App -> ratatui
```

## Event loop and main loop

`EventLoop` (`crates/mew-tui/src/events.rs`) reads keyboard and mouse events
from crossterm over a bounded channel, fires 16ms ticks on a second task, and
receives daemon updates from `TuiClient` on a separate channel.

`chat_with_daemon` (`crates/mew/src/commands/tui.rs`) selects across terminal
input, ticks, and client updates. Each iteration applies incoming state updates
and pending permission or question prompts to `App`, drains and coalesces rapid
events, dispatches queued actions through `runtime::dispatch::handle_action`,
and redraws only when `App` is dirty.

## From keystroke to provider stream

1. `handle_input_event` converts a submit keystroke into `Action::Submit`.
2. `runtime::dispatch` resolves `@` references and attachments, assigning a
   request ID bound to the confirmed session.
3. `DaemonTarget` sends `SubmitPrompt` through `TuiClient`.
4. The daemon acknowledges acceptance, acquires `turn_lock`, and streams
   `ServerMessage` events back over WebSocket.
5. `TuiClient` feeds wire events into `mew_client_core::ClientEngine`, then
   projects updates onto `App` for rendering.

## Display store and API history

`App` and `Agent` maintain distinct message stores:

- **`app.messages` (display store)**: merges all text, reasoning, and tool
  parts across a multi-turn agentic loop into a single visible assistant entry,
  alongside synthetic local notices (alerts, `/cost` output).
- **`agent.messages` (API history)**: canonical turn-by-turn messages stored in
  `session.jsonl` and sent to providers, where each model call produces its own
  assistant `Message`.

On resume, `ClientEngine` rebuilds display messages from daemon history.

## Streaming markdown

`crates/mew-tui/src/ui/chat.rs` renders assistant text in two paths using
`app.md_stream`, `app.md_state`, `app.md_render_cache`, and
`app.rendered_md_cache` (keyed by `mew_message::PartId`):

- **Active streaming text part**: committed Markdown blocks cache into
  `app.md_render_cache` via `md_render_cache.extend(app.md_state.committed(), ...)`,
  while the trailing pending block renders incrementally with
  `ratatui_mdstream::render::render_pending_with_metadata`.
- **Earlier or completed text parts**: expand em dashes and render via
  `ratatui_mdstream::render_markdown_with_metadata`, stored in
  `app.rendered_md_cache` keyed by `(PartId, md_width, text)`.
- **Finalization and resize**: `MessageEnd` sets `pending_md_rerender` to
  trigger a full re-render on the next frame; changing `md_width` clears the
  caches.

## Related crates

| Crate | Purpose | Key types |
|-------|---------|-----------|
| `mew-tui` | Event loop, ratatui views, `App` state | `Event`, `EventLoop`, `App`, `Action` |
| `mew-client-core` | Shared session projection and reconnect core | `ClientEngine`, `ClientState` |
| `ratatui-mdstream` | Streaming Markdown to ratatui `Line`s | `MdStream`, `DocumentState`, `RenderCache` |
