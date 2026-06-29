---
title: Web UI
description: Browser-based chat interface for mew.
---

mew includes a web UI that connects to the daemon over WebSocket. It
provides a browser-based alternative to the TUI with the same features.

## Starting the web UI

```sh
mew daemon --port 127.0.0.1:9847
mew-web
```

Then open `http://localhost:9847` in your browser. The `mew-web` bridge
serves the built React app and relays WebSocket connections to the daemon.

## Features

- **Streaming markdown** with syntax-highlighted code blocks (Shiki)
- **Tool call cards** with inline output and expand/collapse
- **Model picker** with thinking variant selection
- **Session list**: switch between sessions
- **Subagent panel**: shows running/completed subagents
- **Todo list**: renders the agent's todo list
- **Ask-user prompts**: interactive question cards
- **Permission prompts**: approve/deny tool calls
- **Reconnect with backoff**: automatically reconnects on disconnect
- **Light/dark theme**: system, light, or dark

## Architecture

```
Browser → mew-web (bridge) → mew daemon (Unix socket)
```

- **`mew-web-bridge`**: TCP+WS listener that relays browser connections to
  the daemon's Unix socket. Also serves the chat UI's static assets.
- **`mew-web-client`**: TypeScript client library for the mew wire protocol.
  Reusable for Discord/iOS/web frontends.
- **`mew-web-ui`**: React + TanStack Router app. Builds to ESM with `.d.ts`.

## Development

For hot-reload development:

```sh
just dev-ui
```

This starts Vite with HMR and auto-spawns the daemon. Use `--open` to
launch the browser automatically.
