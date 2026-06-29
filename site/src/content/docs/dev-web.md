---
title: Web UI Development
description: How to develop and extend the mew web UI.
---

The web UI is a React app that talks to the daemon over WebSocket via
the `mew-web-client` TypeScript library.

## Getting started

```sh
just dev-ui           # starts Vite dev server with HMR
just dev-ui -- --open  # also launches browser
```

This auto-spawns `mew daemon` in the background and starts the Vite dev
server. Hot module replacement works for all React components.

## Architecture

```
Browser → mew-web (bridge) → mew daemon (Unix socket)
```

Three packages in a pnpm workspace:

- **`mew-web-bridge`** (Rust): TCP+WS listener that relays browser WebSocket
  connections to the daemon's Unix socket. Serves the built React app from
  `mew-web-ui/dist/` as static assets. SPA fallback for client-side routing.
- **`mew-web-client`** (TypeScript): Typed client for the wire protocol.
  Builds to ESM with `.d.ts` types. Reusable for Discord/iOS/web frontends.
- **`mew-web-ui`** (TypeScript/React): React + TanStack Router app. Vite
  build to `dist/`.

## Store pattern

State management uses Zustand. The store lives in `src/stores/session.ts`.
The bridge function (`bridgeClientToStore`) wires client events to store
actions:

```typescript
client.on("model-list", (data) => store.getState().setAvailableModels(data.models));
client.on("model-switched", (data) =>
  store.getState().setCurrentModel(data.provider, data.model),
);
```

To add a new event:

1. Add the event type + dispatch to `mew-web-client/src/index.ts`
2. Add state + action to the `SessionState` interface in `stores/session.ts`
3. Initialize the state value in the store
4. Wire the event in `bridgeClientToStore`
5. Use it in a component

## Key components

| Component | Purpose |
|-----------|---------|
| `App.tsx` | Root layout, reconnect logic, session attach |
| `ChatSurface.tsx` | Message list + streaming text |
| `MessageItem.tsx` | Single message with markdown rendering |
| `InputArea.tsx` | Prompt input with Cmd/Ctrl+Enter to send |
| `ModelPill.tsx` | Model picker with thinking variant selection |
| `TitleStrip.tsx` | Session title + connection status |
| `ToolCallCard.tsx` | Tool call display with inline output |
| `CodeBlock.tsx` | Shiki syntax-highlighted code blocks |
| `MarkdownBody.tsx` | Markdown renderer (streaming + finalized) |

## Building

```sh
just build-web  # builds Rust + TS, embeds dist into mew-web-bridge
```

The built `mew-web` binary serves the React app from embedded assets at
runtime. No separate file serving needed.

## Adding a new wire event end-to-end

1. **Protocol**: Add `ClientMessage` or `ServerMessage` variant + roundtrip test
2. **Daemon**: Handle in `handle_connection`, update `translate_server_message`
3. **TS client**: Add type, method, event dispatch
4. **Store**: Add state, action, bridge wiring
5. **Component**: Render the new state
