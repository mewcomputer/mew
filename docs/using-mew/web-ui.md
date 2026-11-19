---
title: Web UI
description: Browser-based chat interface for mew.
---

mew includes a web UI that connects to the daemon over WebSocket. It
provides a browser-based alternative to the TUI with the same agent
capabilities.

## Starting the web UI

Build the browser bridge from a [source checkout](/docs/getting-started/installation/).
The CLI nightly includes `mew`. The separate `mew-web` executable is built by
`just build-web`.

```sh
just build-web
./target/debug/mew-web
```

Then open `http://localhost:9847` in your browser. The `mew-web` bridge
serves the built React app and relays WebSocket connections to the daemon.
The bridge accepts browser connections from its own local origin. Use the
bridge URL for browser chat and the daemon's loopback TCP endpoint for native clients.

For development with hot reload:

```sh
just dev-web
```

This starts Vite with HMR and the bridge, which starts a daemon when the local
socket is idle. Use `--open` to
launch the browser automatically:

```sh
just dev-web --open
```

## What you see

- **Chat** shows streamed Markdown, tool output, and expandable tool cards.
- **Model picker** lets you switch models and choose thinking settings.
- **Session rail** lists conversations. Start a new chat in a recent workspace
  or select a workspace by path.
- **Composer** sends with Enter. Use Shift+Enter for a newline.
- **Tool permissions and questions** appear as permission prompts and inline
  question cards.
- **Conversation details** include status, todos, and workers as they become
  available.

Use Cmd+K on macOS or Ctrl+K on Linux to open the command palette.

## Architecture

```
Browser → mew-web (bridge) → mew daemon (Unix socket)
```

The browser stack consists of a Rust bridge and two pnpm workspace packages.

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

The browser retries a dropped connection with exponential backoff, up to
30 seconds between attempts. It reattaches to the current session and loads
the newest messages. Select **Load earlier messages** at the top of the chat
for older history.

The home page remembers your last opened session in local storage. To open a
specific conversation, use its `/session/<session-id>` URL from the same
running bridge. Multiple tabs can attach to one session, and handling a
permission request or question in one authorized client resolves it for the
others.

See [Daemon Protocol](/docs/development/dev-protocol/) for the wire-level details,
and [Web UI Development](/docs/development/dev-web/) for the developer guide.
