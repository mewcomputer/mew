---
title: Web UI
description: Browser-based chat interface for Mew.
---

Mew provides a browser frontend (`mew-web-ui`) connected to the daemon over
WebSocket via the `mew-web` bridge.

## Starting the web UI

Build and run the bridge from a [source checkout](/docs/getting-started/installation/)
(CLI nightlies include `mew` only; `just build-web` builds `mew-web`):

```sh
just build-web
./target/debug/mew-web
```

Open `http://localhost:9847` in your browser. `mew-web` serves the embedded
React bundle and relays WebSocket traffic to the local daemon (accepting
same-origin browser connections; native clients connect directly to the daemon's
loopback TCP port).

For hot-reload development:

```sh
just dev-web
```

`just dev-web` launches both `mew-web` on port `9847` and Vite's HMR dev server
on `http://127.0.0.1:5173/`, auto-spawning a daemon when the local socket is
idle. Open `http://127.0.0.1:5173/` (or pass `--open` to launch it
automatically).

## Interface

- **Chat**: streamed Markdown, tool output, and expandable tool cards.
- **Model picker**: switch models and choose thinking variants.
- **Session rail**: browse sessions, create chats in recent or custom paths,
  and filter conversations.
- **Composer**: press `Enter` to send and `Shift+Enter` for a newline.
- **Approvals and questions**: tool permission prompts and inline question
  cards.
- **Conversation details**: live status, todos, and subagent transcripts.

Press `Cmd+K` (macOS) or `Ctrl+K` (Linux/Windows) to open the command palette,
and `Cmd+N` / `Ctrl+N` to start a new chat.

## Architecture

```text
Browser -> mew-web (bridge) -> mew daemon (Unix socket)
```

- **`mew-web-bridge`** (`mew-web`, Rust): TCP and WebSocket server that relays
  browser connections to the daemon's Unix socket, auto-spawns `mew daemon`
  when idle, and serves Vite-hashed React assets embedded from `mew-web-ui/dist/`
  at compile time.
- **`@mew/web-client`** (`mew-web-client/`, TypeScript): ESM package with `.d.ts`
  types exporting `MewClient`, which opens the WebSocket connection, dispatches
  incoming events, and matches request/response calls.
- **`mew-web-ui`** (`mew-web-ui/`, TypeScript/React): Vite React app using
  TanStack Router and Zustand.

## Connection lifecycle

On disconnect, the browser retries with exponential backoff up to 30 seconds,
reattaches to the active session, and fetches recent messages. Click **Load
earlier messages** at the top of the transcript to page backward.

The root route restores your last opened session from `localStorage`, or open
any session directly at `/session/<session-id>` on the running bridge. Multiple
tabs can attach to the same session, and answering a permission or question in
one client clears it on all attached clients.

See [Daemon Protocol](/docs/development/dev-protocol/) for wire messages and
[Web UI Development](/docs/development/dev-web/) for contributor details.
