---
title: Web UI
description: Browser-based chat interface for mew.
---

mew includes a web UI that connects to the daemon over WebSocket. It
provides a browser-based alternative to the TUI with the same agent
capabilities.

## Starting the web UI

```sh
mew daemon --port 127.0.0.1:9847
mew-web
```

Then open `http://localhost:9847` in your browser. The `mew-web` bridge
serves the built React app and relays WebSocket connections to the daemon.

For development with hot reload:

```sh
just dev-ui
```

This starts Vite with HMR and auto-spawns the daemon. Use `--open` to
launch the browser automatically:

```sh
just dev-ui -- --open
```

## What you see

The web UI has a layout similar to the TUI but optimized for browser
interaction:

- **Chat surface**: streaming markdown with syntax-highlighted code
  blocks (Shiki). Tool calls appear as cards with inline output and
  expand/collapse.
- **Model picker**: shows available models with thinking variant
  selection. Click to switch models.
- **Title strip**: shows the session title and connection status.
- **Session rail**: lists previous sessions. Click to resume.
- **Input area**: type your prompt. `Cmd/Ctrl+Enter` to send.
- **Status footer**: shows token count, model, and connection state.
- **Todo panel**: renders the agent's todo list when active.
- **Subagent panel**: shows running and completed subagents.
- **Ask-user cards**: interactive question cards when the agent asks.
- **Permission prompts**: approve or deny tool calls inline.

## Features

| Feature | TUI | Web UI |
|---------|-----|--------|
| Streaming markdown | ratatui-mdstream | Shiki syntax highlighting |
| Tool call cards | inline | inline with expand/collapse |
| Model picker | `Ctrl+P` | click model pill |
| Thinking variant | `Ctrl+P` cycle | dropdown |
| Session list | `/sessions` | sidebar rail |
| Subagent panel | sidebar | dedicated panel |
| Todo list | sidebar panel | dedicated panel |
| Ask-user prompts | modal | inline card |
| Permission prompts | modal (`a`/`s`/`d`) | inline buttons |
| Reconnect | n/a | exponential backoff (2s, 4s, 8s, cap 30s) |
| Theme | terminal colors | system, light, dark |

## Architecture

```
Browser → mew-web (bridge) → mew daemon (Unix socket)
```

Three packages in a pnpm workspace:

- **`mew-web-bridge`** (Rust): TCP+WS listener that relays browser
  WebSocket connections to the daemon's Unix socket. Serves the built
  React app from `mew-web-ui/dist/` as static assets. Auto-spawns
  `mew daemon` if not running.
- **`mew-web-client`** (TypeScript): Typed client for the wire protocol.
  Builds to ESM with `.d.ts` types. The `MewClient` class manages the
  WebSocket connection, dispatches events to typed listeners, and
  provides promise-based methods for request/response patterns.
- **`mew-web-ui`** (TypeScript/React): React app with TanStack Router.
  Vite build to `dist/`. Uses Zustand for state management.

The bridge serves the built React app from embedded assets at runtime.
Vite hashes asset filenames, so files are served dynamically by path
lookup rather than hardcoded `include_bytes!`.

## Connection lifecycle

The web UI handles disconnection gracefully:

1. On unexpected WS close, the client retries with exponential backoff
   (2s, 4s, 8s, up to 30s).
2. The session ID is saved to `localStorage`. On reload, the client
   reconnects via `attachSession`, falling back to `newSession` if the
   session is gone.
3. When reconnected, the full message history is replayed.

Multiple browser tabs can attach to the same session. Permission and
ask-user prompts go to all connected clients. Any client can respond.
`RequestResolved` dismisses the prompt across all clients.

See [Daemon Protocol](/docs/dev-protocol/) for the wire-level details,
and [Web UI Development](/docs/dev-web/) for the developer guide.
