---
title: Web UI Development
description: How to develop and extend the mew web UI.
---

The web UI is a React app that talks to the daemon over WebSocket via
the `mew-web-client` TypeScript library.

## Getting started

```sh
just build-web        # installs dependencies and builds both binaries
just dev-web          # starts the bridge and Vite with HMR
just dev-web --open   # also launches the browser
```

This auto-spawns `mew daemon` in the background and starts the Vite dev
server. Hot module replacement works for all React components.

## Architecture

```
Browser → mew-web (bridge) → mew daemon (Unix socket)
```

The browser stack consists of a Rust bridge and two pnpm workspace packages.

- **`mew-web-bridge`** (Rust): TCP+WS listener that relays browser WebSocket
  connections to the daemon's Unix socket. Serves the built React app from
  `mew-web-ui/dist/` as static assets via `include_dir`. SPA fallback for
  client-side routing. Auto-spawns `mew daemon` if not running.
- **`mew-web-client`** (TypeScript): Typed client for the wire protocol.
  Builds to ESM with `.d.ts` types. The `MewClient` class manages the
  WebSocket connection, dispatches events to typed listeners, and provides
  promise-based methods for request/response patterns.
- **`mew-web-ui`** (TypeScript/React): React app with TanStack Router.
  Vite build to `dist/`. Uses Zustand for state management.

## The TypeScript client

`MewClient` (`mew-web-client/src/index.ts`):

```typescript
import { MewClient } from "@mew/web-client";

const client = new MewClient("ws://localhost:9847/");
await client.connect();
await client.ping();
const sessionId = await client.newSession();

// Promise-based methods (wait for the matching server response):
const models = await client.listModels();
const model = models[0];
if (model) await client.switchModel(model.provider, model.model);

// Event-based (streaming):
client.on("provider", (ev) => { /* handle PartStart/PartDelta/etc */ });
client.on("tool-start", ({ call_id }) => { /* ... */ });
client.on("model-switched", (data) => { /* ... */ });

await client.submitPrompt(sessionId, "Summarize this project.");
```

The application calls `ping()` after connection to discover daemon capabilities.
Create or attach a session before submitting a prompt. Scoped submissions
resolve once the daemon acknowledges acceptance. Watch provider events for
streamed output.

Subscribe to `client.on("permission-request", (request, respond) => { … })`
to handle permission requests and call `respond` with the user's decision.

## The Zustand store

`src/stores/session.ts` defines `SessionState` and its actions. The store
tracks connection and session state, assembles streaming messages and
history pages, and holds model settings, todos, and pending requests.

## The bridge function

`getClient()` in `src/lib/client.ts` creates a shared `MewClient` and calls
`bridgeClientToStore(client)`. The bridge maps client events to actions on
`useSessionStore`. Permission responders and queued ask-user actions retain
request IDs so replies reach the right pending action.

## Routing and reconnect

`src/routes/__root.tsx` mounts the workspace and calls `useMewConnection`.
The connection hook retries with exponential backoff capped at 30 seconds and
checks the daemon with `ping()`. The session route's `useSessionAttach` hook
reattaches after reconnect and surfaces attachment errors.

The confirmed session ID is saved to `localStorage`. The home route tries the
saved session, and `/session/<id>` opens a specific conversation directly.
History is paginated. Attaching loads the newest page, and older messages load
when requested.

## Key components

| Component or route | Purpose |
|-----------|---------|
| `routes/__root.tsx` | Workspace layout, connection hook, global navigation |
| `routes/session.$sessionId.tsx` | Session attachment and conversation layout |
| `components/virtual-chat-surface.tsx` | Virtualized message list and streaming text |
| `components/input-area.tsx` | Prompt input and slash command suggestions |
| `components/model-pill.tsx` | Model picker with thinking variant selection |
| `components/session-rail.tsx` | Session list |
| `components/right-rail.tsx` | Tabbed workspace tools |
| `components/permission-toast.tsx` | Tool permission requests |
| `components/ask-user-card.tsx` | Interactive questions |

## Building

```sh
just build-web  # builds Rust + TS, embeds dist into mew-web-bridge
```

The built `mew-web` binary serves the React app from embedded assets at
runtime. Vite hashes asset filenames, so files are served dynamically by
path lookup rather than hardcoded `include_bytes!`.

`mew-web` listens on loopback. It accepts WebSocket requests for its bound
address and port, including the `localhost` alias, and checks browser origins
against the same address and any explicit loopback origins. `just dev-web`
allows the Vite server on port 5173. If you run the bridge and Vite separately,
pass `--allow-origin http://localhost:5173` to the bridge and open Vite at that
address. Repeat `--allow-origin` for each additional local development origin.

## Adding a new wire event end-to-end

1. **Protocol**: Add `ClientMessage` or `ServerMessage` variant + roundtrip
   test in `mew-protocol/src/lib.rs`
2. **Daemon**: Handle in `handle_connection` (`mew-daemon/src/lib.rs`).
   Update `translate_server_message` in `mew-daemon/src/client.rs` if needed.
3. **TS client**: Add the event type to the `MewClientEvents` interface,
   add the dispatch case in `handleMessage`, and add any request method.
4. **Store**: Add state field + action to `SessionState`, initialize in the
   store, wire in `bridgeClientToStore`.
5. **Component**: Render the new state in a React component and exercise the
   flow with the relevant client or UI tests.
