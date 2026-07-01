---
title: Session handover
description: Target design for handing a mew session from one frontend to another.
---

This doc describes the target design for handing a mew session from one
frontend to another. The motivating case is a user chatting in the TUI that is
connected to the daemon and wanting to continue the same conversation in the
web UI.

## Current state

The daemon already supports multiple clients attaching to the same session. See
`docs/development/dev-architecture.md` and
`docs/development/dev-protocol.md` for how `AttachSession`,
`SessionHistory`, and broadcast events work today.

The missing pieces are all on the client side and in the protocol around
presence and lifecycle:

- The TUI in daemon mode cannot attach to an existing session; it always calls
  `NewSession`.
- There is no way for a frontend to discover or advertise that it is viewing a
  session.
- The last client to detach cancels the in-flight turn and unloads the session
  from memory, which makes handover feel fragile.

## Target experience

A user running `mew chat --connect ws://...` should be able to say `/web` (or
use a keybinding) and get a short URL or QR code that opens the web UI already
attached to the current session. The TUI can then either close or stay open as
a read-only observer. The conversation continues uninterrupted.

The reverse should also work: a web UI session can be picked up by the TUI via
`mew chat --connect ws://... --attach <session-id>`.

## Required protocol additions

### 1. Client presence events

The daemon should tell every attached client when another client joins or
leaves the session. This lets frontends show "also viewing on web" or dim the
input when another client has focus.

Suggested `ServerMessage` variants:

```rust
ClientAttached { client_id: u64, client_kind: ClientKind },
ClientDetached { client_id: u64 },
```

`ClientKind` is an enum such as `Tui`, `Web`, `Cli`, or `Unknown`. Clients
report their kind during the handshake or in `NewSession` / `AttachSession`.

### 2. Handover intent

A frontend can announce that it is yielding control. This is advisory; the
daemon does not enforce it. Other clients can use it to update their UI, for
example by switching from active input to observer mode.

```rust
// Client → Server
YieldControl {},

// Server → Client (broadcast)
ControlYielded { client_id: u64, to_client_id: Option<u64> },
```

### 3. Keep-warm detach

When the last client detaches, the daemon currently cancels the turn and
removes the session from `active`. For handover to feel seamless, the daemon
should keep the session warm for a short grace period (e.g. 30-60 seconds)
while waiting for the new frontend to attach.

Behavior:

- If no client reattaches before the grace period expires, cancel and unload as
  today.
- If a client attaches during the grace period, resume broadcasting without
  cancelling the turn.
- The grace period should be configurable and opt-in per session or globally.

### 4. TUI attach support

`mew chat --connect <url>` should accept an optional `--attach <session-id>`
flag that sends `AttachSession` instead of `NewSession`. The TUI should then
support `/resume <session-id>` in daemon mode by sending `AttachSession` and
replacing its local display store with the returned `SessionHistory`.

### 5. Session URL scheme

The web bridge and bundled UI should support a session id in the URL:

```
http://localhost:9847/session/<session-id>
```

Opening that route calls `AttachSession` for the given id. If the session does
not exist, the UI shows a clear error and offers to start a new session.

For local handover from the TUI, a short URL or QR code can point to:

```
http://localhost:9847/handover?t=<token>&s=<session-id>
```

The token is optional and can be used to authorize the handover if the web
bridge ever runs on a shared interface. For local-only operation it can be
omitted.

## UX guidelines

- **Input ownership is advisory, not enforced.** Multiple clients can send
  prompts to the same session, but the daemon serializes turns via `turn_lock`.
  The UI should make it obvious who is "driving" without blocking the user.
- **Observer mode.** A frontend that has yielded control should still display
  streaming events and tool progress. It should not hide or disable the input
  completely, but it should de-emphasize it.
- **Grace period feedback.** If the user closes the TUI and opens the web UI,
  a brief "reconnecting..." state is acceptable while the session is warm.
- **No surprises.** Handover should never duplicate a user message. The web UI
  already deduplicates `UserMessage` events; this should remain the rule.

## Open questions

- Should the daemon persist tool execution state across unload, or is it
  acceptable to cancel and let the LLM retry on the next turn?
- Should handover carry ephemeral state such as the current streaming markdown
  buffer, or should the new client rebuild it from `SessionHistory`?
- Should there be a permission gate when a new client attaches to an active
  session that has pending permission or ask-user requests?

## Related code

- `crates/mew-daemon/src/session.rs` — `SessionManager`, `Session::attach_client`,
  `Session::broadcast`, last-client detach logic.
- `crates/mew-daemon/src/lib.rs` — `handle_connection`, `translate_event`.
- `crates/mew-daemon/src/client.rs` — `DaemonClient`; needs `attach_session` and
  `list_sessions` APIs.
- `crates/mew-protocol/src/lib.rs` — `ClientMessage` / `ServerMessage` definitions.
- `mew-web-client/src/index.ts` and `mew-web-ui/src/stores/session.ts` — web
  client attach and deduplication.
- `crates/mew-tui/src/app.rs` and `crates/mew/src/main.rs` — TUI daemon mode and
  slash command handling.
