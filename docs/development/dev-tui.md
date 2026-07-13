---
title: TUI Architecture
description: How the ratatui frontend works, in standalone mode and when connected to the daemon.
---

The mew TUI is a ratatui application that can run in two modes:

- **Standalone**: it builds the `Agent` directly and drives the provider loop
  itself. This is the default `mew chat` path.
- **Daemon client**: it connects to `mew-daemon` over WebSocket and receives
  `AgentEvent`s over the wire. This is `mew chat --connect <url>`.

This doc covers the TUI's event loop, display state, streaming markdown, and
how a keystroke becomes a provider stream. For the daemon and session model,
see [Architecture](/docs/development/dev-architecture/). For recording captures
against a live daemon, see [Recording real-provider TUI captures](/docs/development/dev-tui-capture/).

## The pipeline

```
Keyboard → crossterm EventStream → EventLoop (mpsc::channel(256))
                                      │
                                      ├─ Event::Input(crossterm::Event)
                                      ├─ Event::Agent(AgentEvent)
                                      ├─ Event::Tick (60fps)
                                      └─ Event::Quit
                                            │
                  handle_input_event() → Action::Submit(text)
                                            │
                  agent.run_with_parts(prompt, attachments, token)
                      returns mpsc::Receiver<AgentEvent>
                                            │
                  event_loop.forward_agent_events(agent_rx)
                      spawns tokio task pumping AgentEvent → EventLoop
                                            │
                  app.handle_agent_event(event) → draw()
```

In daemon mode the same pipeline runs, but the `AgentEvent`s arrive through
`mew-daemon/src/client.rs` instead of a local `Agent`.

## The event loop

`EventLoop` (`events.rs`) is a thin wrapper around `mpsc::channel(256)`:

```rust
pub struct EventLoop {
    tx: mpsc::Sender<Event>,
}
```

Three tokio tasks feed events into the channel:

- **Crossterm reader**: reads keyboard/mouse events and forwards as `Event::Input`.
- **Tick generator**: fires every 16ms (60fps) as `Event::Tick`. Skipped
  when idle (see `tick_interval_ms` for adaptive polling).
- **Agent forwarder**: per-prompt. `forward_agent_events` spawns a task
  that pumps `mpsc::Receiver<AgentEvent>` into `Event::Agent`:

```rust
pub fn forward_agent_events(&self, mut agent_rx: Receiver<AgentEvent>) {
    let tx = self.tx.clone();
    tokio::spawn(async move {
        while let Some(event) = agent_rx.recv().await {
            if tx.send(Event::Agent(event)).await.is_err() { break; }
        }
    });
}
```

## The main loop

The TUI main loop (`run_tui` in `main.rs`):

1. **Render**: `terminal.draw(|f| mew_tui::ui::draw(f, &mut app))`. Skipped
   when idle: `if !last_event_was_tick || app.needs_redraw()`.
2. **Wait for event**: `event_rx.recv().await`.
3. **Process**: match on `Event::Input` / `Event::Agent` / `Event::Tick` / `Event::Quit`.
4. **Drain loop**: after processing the first event, coalesces rapid events
   via `try_recv()`. Capped at `STREAMING_DRAIN_LIMIT = 4` agent events per
   frame so streaming text appears incrementally instead of all at once.

## How a keystroke becomes a provider stream

1. User types text and presses Enter. Crossterm fires `Event::Key(Enter)`.
2. `handle_input_event` calls `app.submit_input()`, returns `Action::Submit(text)`.
3. The main loop calls `agent.run_with_parts(prompt, attachments, token)`.
4. `run_with_parts` spawns a tokio task running `run_loop`, which calls
   `turn_loop`. Returns `mpsc::Receiver<AgentEvent>` immediately.
5. `event_loop.forward_agent_events(agent_rx)` pumps the receiver into the
   main event channel.
6. Each `AgentEvent::Provider(pe)` updates App state via `handle_agent_event`:
   `PartStart` creates a new part, `PartDelta` appends text, `MessageEnd`
   finalizes the stream.
7. `draw()` renders the updated state.

In daemon mode steps 3–5 are replaced by sending `ClientMessage::Prompt` over
WebSocket and receiving `ServerMessage`s that are translated back into
`AgentEvent`s by `DaemonClient`.

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
