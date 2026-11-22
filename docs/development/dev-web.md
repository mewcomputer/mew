---
title: Web UI Development
description: How to develop and extend the Mew web UI.
---

`mew-web-ui` is a React application communicating with `mew-daemon` over
WebSocket through `@mew/web-client`. The same compiled React bundle also powers
the opt-in Electron shell (`apps/mew-electron`) via `mew desktop-host`
(`DesktopRelay` in `crates/mew-web-bridge/src/desktop.rs`).

## Getting started

```sh
just build-web        # install JS deps, build UI, and compile mew-web
just dev-web          # start mew-web bridge (:9847) and Vite HMR (:5173)
just dev-web --open   # also open http://127.0.0.1:5173/ in your browser
```

`just dev-web` auto-spawns `mew daemon` when the local Unix socket is idle and
enables hot module replacement across React components.

## Architecture

```text
Browser -> mew-web (bridge) -> mew daemon (Unix socket)
```

- **`mew-web-bridge`** (Rust): TCP and WebSocket server relaying browser
  WebSockets to the daemon's Unix socket, auto-spawning `mew daemon` when idle,
  and serving Vite-hashed static assets embedded from `mew-web-ui/dist/` via
  `include_dir` (with SPA fallback routing).
- **`@mew/web-client`** (`mew-web-client/`, TypeScript): ESM package with `.d.ts`
  declarations exporting `MewClient` for typed events and request/response
  methods.
- **`mew-web-ui`** (`mew-web-ui/`, TypeScript/React): TanStack Router and
  Zustand frontend.

## The TypeScript client

`MewClient` (`mew-web-client/src/index.ts`):

```typescript
import { MewClient } from "@mew/web-client";

const client = new MewClient("ws://localhost:9847/");
await client.connect();
await client.ping();
const sessionId = await client.newSession();

const models = await client.listModels();
const model = models[0];
if (model) await client.switchModel(model.provider, model.model);

client.on("provider", (ev) => { /* handle PartStart/PartDelta/etc. */ });
client.on("tool-start", ({ call_id }) => { /* ... */ });
client.on("model-switched", (data) => { /* ... */ });

await client.submitPrompt(sessionId, "Summarize this project.");
```

Call `ping()` after connecting to inspect daemon version and capabilities, then
create or attach to a session before submitting prompts. `submitPrompt` resolves
once the daemon returns an accepted `PromptReceipt`. Handle
`client.on("permission-request", (request, respond) => { ... })` by calling
`respond` with the user's decision.

## Zustand store and client bridge

- `src/stores/session.ts` defines `SessionState`, tracking connection status,
  paginated transcripts, streaming parts, model settings, todos, and pending
  prompts.
- `src/lib/client.ts` (`getClient()`) creates the singleton `MewClient` and
  wires `bridgeClientToStore(client)` to Zustand actions; components and store
  modules also import `src/lib/client-ref.ts` to avoid circular dependencies.

## Routing and reconnect

`src/routes/__root.tsx` mounts the layout and runs `useMewConnection`, which
reconnects with exponential backoff (up to 30 seconds) and checks daemon health
with `ping()`. `src/routes/session.$sessionId.tsx` runs `useSessionAttach` to
reattach and restore paginated history (`Load earlier messages` fetches older
pages); the root route remembers the latest session ID in `localStorage`.

## Key components

| Component or route | Purpose |
|--------------------|---------|
| `routes/__root.tsx` | Workspace layout, connection hook, global shortcuts |
| `routes/session.$sessionId.tsx` | Session attachment and conversation view |
| `components/virtual-chat-surface.tsx` | Virtualized message list and streaming text |
| `components/input-area.tsx` | Prompt composer and slash autocomplete |
| `components/model-pill.tsx` | Model and thinking-variant picker |
| `components/session-rail.tsx` | Session list and creation |
| `components/right-rail.tsx` | Tabbed workspace tools |
| `components/permission-toast.tsx` | Tool approval prompt |
| `components/ask-user-card.tsx` | Interactive question prompt |

## Building and origins

```sh
just build-web  # build TypeScript packages and embed dist/ into mew-web
```

`mew-web` binds to loopback and accepts WebSocket connections whose `Origin`
matches its bound address (including `localhost`) or an explicit `--allow-origin`
flag. `just dev-web` passes `--allow-origin` for Vite on port `5173`; pass
`--allow-origin http://localhost:5173` when running Vite and the bridge
separately.

## Adding a new wire event end-to-end

1. **Protocol**: add the `ClientMessage` or `ServerMessage` variant and
   roundtrip test in `crates/mew-protocol/src/lib.rs`.
2. **Daemon**: handle it in `handle_connection` (`crates/mew-daemon/src/lib.rs`),
   and update `mew-client-core` (and `translate_server_message` in
   `crates/mew-daemon/src/client.rs` if applicable).
3. **TypeScript client**: add the type to `MewClientEvents`, dispatch it in
   `handleMessage`, and add any request method in `mew-web-client/src/index.ts`.
4. **Store**: add the state field and action to `SessionState` and wire it in
   `bridgeClientToStore` (`mew-web-ui/src/stores/session.ts`).
5. **Component and tests**: render the state in `mew-web-ui` and add client/UI
   tests.
