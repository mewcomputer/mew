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
const client = new MewClient();
await client.connect("ws://localhost:9847");

// Promise-based methods (wait for the matching server response):
const models = await client.listModels();
const result = await client.switchModel("deepseek", "deepseek-v4-flash");
const variant = await client.setThinkingVariant("high");

// Event-based (streaming):
client.on("provider", (ev) => { /* handle PartStart/PartDelta/etc */ });
client.on("tool-start", (callId) => { /* ... */ });
client.on("model-switched", (data) => { /* ... */ });
```

Request/response methods use a pattern: register a one-time listener for
the response event, send the request, resolve the promise when the response
arrives.

Permission requests use a callback pattern: `client.onPermissionRequest`
registers a handler that receives a `respond(decision)` callback. The
handler can be async and the response is forwarded automatically.

## The Zustand store

State lives in `src/stores/session.ts`. The `SessionState` interface defines
all state fields and actions:

```typescript
interface SessionState {
  // Connection
  connectionState: ConnectionState;
  sessionId: string | null;

  // Messages
  messages: ChatMessage[];
  streamingPartId: string | null;
  streamingText: string;

  // Model management
  availableModels: ModelInfo[];
  currentModel: string | null;
  currentProvider: string | null;
  currentThinkingVariant: string | null;

  // Session titles
  sessionTitles: Map<string, string>;

  // Actions
  setAvailableModels: (models: ModelInfo[]) => void;
  setCurrentModel: (provider: string, model: string) => void;
  setCurrentThinkingVariant: (variant: string | null) => void;
  onSessionTitleChanged: (sessionId: string, title: string) => void;
  // ... 30+ more actions
}
```

## The bridge function

`bridgeClientToStore` wires client events to store actions:

```typescript
export function bridgeClientToStore(client: MewClient, store: typeof useSessionStore) {
  client.on("session-ready", (data) => store.getState().setSessionId(data.session_id));
  client.on("model-list", (data) => store.getState().setAvailableModels(data.models));
  client.on("model-switched", (data) =>
    store.getState().setCurrentModel(data.provider, data.model));
  client.on("thinking-variant-changed", (data) =>
    store.getState().setCurrentThinkingVariant(data.variant));
  client.on("session-title-changed", (data) =>
    store.getState().onSessionTitleChanged(data.session_id, data.title));
  // ... 20+ more event bindings
}
```

Permission and ask-user requests use side-channel maps
(`permissionResponders`, `askUserResponders`) to pair the UI's response
with the client's callback.

## App.tsx

`App.tsx` handles:

- **Connection lifecycle**: exponential backoff reconnection (2s, 4s, 8s,
  cap 30s). On unexpected WS close, retries automatically and re-attaches
  to the same session.
- **Session persistence**: `sessionId` saved to `localStorage`. On reload,
  reconnects via `attachSession`, falling back to `newSession` if the
  session is gone.
- **Layout tree**: `ChatSurface → TodoPanel → SubagentPanel → AskUserCard → InputArea`

## Key components

| Component | Purpose |
|-----------|---------|
| `App.tsx` | Root layout, reconnect logic, session attach |
| `ChatSurface.tsx` | Message list + streaming text |
| `MessageItem.tsx` | Single message with markdown rendering |
| `InputArea.tsx` | Prompt input (Cmd/Ctrl+Enter to send) |
| `ModelPill.tsx` | Model picker with thinking variant selection |
| `TitleStrip.tsx` | Session title + connection status |
| `ToolCallCard.tsx` | Tool call display with inline output |
| `CodeBlock.tsx` | Shiki syntax-highlighted code blocks |
| `MarkdownBody.tsx` | Markdown renderer (streaming + finalized) |
| `SessionRail.tsx` | Session list sidebar |
| `StatusFooter.tsx` | Token count, model, connection status |

## Building

```sh
just build-web  # builds Rust + TS, embeds dist into mew-web-bridge
```

The built `mew-web` binary serves the React app from embedded assets at
runtime. Vite hashes asset filenames, so files are served dynamically by
path lookup rather than hardcoded `include_bytes!`.

## Adding a new wire event end-to-end

1. **Protocol**: Add `ClientMessage` or `ServerMessage` variant + roundtrip
   test in `mew-protocol/src/lib.rs`
2. **Daemon**: Handle in `handle_connection` (`mew-daemon/src/lib.rs`).
   Update `translate_server_message` in `mew-daemon/src/client.rs` if needed.
3. **TS client**: Add the event type to the `MewClientEvents` interface,
   add the dispatch case in `handleMessage`, and add any request method.
4. **Store**: Add state field + action to `SessionState`, initialize in the
   store, wire in `bridgeClientToStore`.
5. **Component**: Render the new state in a React component.
