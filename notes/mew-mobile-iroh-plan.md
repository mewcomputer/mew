# mew mobile + iroh remote access plan

Context: an iOS client for mew, plus the general remote-access capability it needs. The timing is unusually good: the daemon's wire protocol is already transport-agnostic, the "session rail + alerts" work is exactly the mobile surface, and iroh solves the hard NAT-traversal problem that would otherwise require Tailscale or port forwarding.

Status: planning / stage 1 ready for handoff.

---

## Why the daemon side is almost free

`mew-daemon/src/lib.rs` already has a generic connection handler:

```rust
async fn handle_connection<S>(stream: S, ...)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ws_stream = accept_async(stream).await?;
    // ... JSON protocol over WebSocket ...
}
```

The daemon runs two listeners today: a Unix socket (`run`) and a TCP socket (`run_tcp`). Both hand a single bidirectional stream to the same handler.

An iroh connection is opened by ALPN and yields a bidirectional QUIC stream: `connection.accept_bi().await?` returns `(SendStream, RecvStream)`, where `SendStream` implements `AsyncWrite` and `RecvStream` implements `AsyncRead`. A small wrapper struct that owns both halves and implements `AsyncRead + AsyncWrite` can be passed straight into `handle_connection`.

So the daemon side is roughly: bind an `iroh::Endpoint`, register an ALPN such as `b"mew/wire/0"` on the Router, and in the `ProtocolHandler::accept` implementation authenticate the peer NodeId, wrap the stream, and call `handle_connection`.

Attach and `SessionHistory` replay already handle "client comes and goes," which is the natural phone pattern.

Current iroh version as of this plan: `v0.95.x`. The accept-side Router API is a handful of lines.

### Bonus: web UI over iroh later

iroh compiles to WASM, with browser nodes working relay-only and still e2e encrypted. Once the daemon speaks iroh, the web UI could dial it over iroh too, and the TCP bridge (currently a localhost-trust security boundary) could be retired. That is not a prerequisite for iOS and is intentionally out of scope for the first stages.

---

## The three real gaps

### Gap 1: auth and pairing (the genuine missing piece)

The TCP bridge is localhost-trust. `crates/mew-daemon/src/files.rs` treats the session cwd as a security boundary because the bridge binds to TCP. That model cannot be exposed to the internet.

iroh connections are mutually authenticated by public key. The daemon knows the connecting peer's `NodeId` before any protocol byte is sent. Auth becomes:

- a persistent allowlist of trusted client `NodeId`s, and
- a pairing flow for adding new clients.

This is a stronger security model than today, not a compromise.

Proposed pairing flow:

1. User runs `mew pair` on the daemon machine.
2. The daemon generates a short-lived pairing token, prints a QR code containing the daemon's `NodeId` and relay info, and listens for one approved first connection.
3. Phone scans the QR code; its core dials the daemon.
4. The daemon adds the peer `NodeId` to the allowlist and persists it automatically.
5. Future connections from that NodeId are accepted automatically.

The allowlist is managed through pairing, not by hand-editing a config file.

### Gap 2: the client core

`mew-web-client` and the web UI's Zustand store encode all the protocol knowledge, but the code cannot be reused in Swift. It needs to exist again in a mobile-friendly layer.

Recommended shape: a new `mew-mobile-core` Rust crate.

Responsibilities of `mew-mobile-core`:

- own the iroh endpoint
- reconnect logic with backoff
- encode/decode `mew-protocol` `ClientMessage` / `ServerMessage`
- expose a small app-specific API to Swift via UniFFI

Example surface (approximate):

```rust
fn connect(daemon_node_id: String, token: Option<String>);
fn list_sessions() -> Vec<SessionInfo>;
fn attach(session_id: String);
fn prompt(text: String);
fn respond_to_permission(id: String, decision: PermissionDecision);
fn set_event_callback(callback: Box<dyn Fn(MobileEvent)>);
```

iroh-ffi exists and now has a `v1.0.0` release (SwiftPM + Cocoapods). It could in theory be consumed directly from Swift. The safer pattern is still to bind your own thin core: it keeps iroh's pre-1.0 API churn out of the Swift layer and lets `mew-protocol` own the wire types. Verify iroh-ffi's update cadence before depending on it directly.

### Gap 3: iOS platform reality

This is the actual hard part, and it is independent of iroh.

- iOS kills the QUIC connection roughly 30 seconds after the app backgrounds.
- "Glance at a long-running turn while the app is open" works.
- Push notifications when a background session needs approval require APNs, and APNs requires a server that can send pushes. Pure p2p cannot push.

Options:

- **v1: foreground-only.** `SessionAlert` becomes a local notification while the app is open. The "phone buzzes on the lock screen" use case is explicitly deferred.
- **v2: tiny push relay.** A small server (could be self-hosted) receives a signal from the daemon and forwards an APNs push. The approval response still travels p2p over iroh.

Also: distribution is sideload-with-a-developer-account territory unless you pursue the App Store.

---

## Why the UI surface is small

The recent rail work maps almost directly to a mobile app:

- `SessionAlert` → local/remote notifications
- needs-you ordering → session list sorted by attention
- session usage → cost badge
- the rail itself → the main mobile navigation

A phone client is mostly: a session rail, a chat view, and permission toasts. That is a small subset of the UI, and it already has the most wire support.

---

## Honest alternative

Tailscale + Safari on the existing web UI works today with zero new code. The iroh path buys:

- no VPN dependency,
- real peer auth (NodeId allowlist),
- a path to native UX.

That tradeoff should be explicit when deciding how far to climb the ladder.

---

## POC ladder

Each rung is independently useful; you can stop after any of them.

1. **Daemon iroh listener + pairing.** Add `--iroh` to `mew daemon`, NodeId allowlist, and `mew pair`.
2. **Prove it with a Rust CLI peer.** A second machine dials the daemon over iroh and sends a prompt.
3. **Mobile core + UniFFI.** `mew-mobile-core` with iroh endpoint, reconnect, and protocol encode/decode.
4. **Thin SwiftUI client.** Sessions list, attach, streaming chat, permission toasts.

---

## Stage 1 spec: daemon iroh listener + pairing

This stage is well-defined enough to hand off now.

### New crate or feature?

Add an optional `iroh` feature to `mew-daemon` that pulls in `iroh` and exposes `DaemonServer::run_iroh`. The existing TCP and Unix listeners stay untouched; iroh is supplemental, not a replacement. The feature stays off by default until the dependency cost and compile-time impact are measured.

### Wire format over iroh

For stage 1, reuse the existing WebSocket upgrade: wrap the iroh `(send, recv)` pair as a single `AsyncRead + AsyncWrite` type and pass it to `accept_async`, then into `handle_connection`. This is double-framed (WebSocket inside QUIC) but requires zero protocol changes.

A future cleanup can split framing from protocol logic and send length-prefixed JSON directly over the QUIC stream. That is out of scope for stage 1.

### Stream wrapper

Roughly:

```rust
pub struct IrohStream {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
}

impl AsyncRead for IrohStream { /* delegate to recv */ }
impl AsyncWrite for IrohStream { /* delegate to send */ }
```

### Daemon listener

```rust
pub async fn run_iroh(self, allowlist: NodeIdAllowlist) -> Result<()> {
    let endpoint = Endpoint::builder()
        .alpns(vec![b"mew/wire/0".to_vec()])
        .bind().await?;

    let router = Router::builder(endpoint.clone())
        .accept(b"mew/wire/0".to_vec(), Arc::new(MewIrohHandler {
            allowlist,
            session_manager: self.session_manager.clone(),
            groups_store: self.groups_store.clone(),
            thinking_setter: self.thinking_setter.clone(),
        }))
        .spawn().await?;

    // print NodeId / pairing info
    // wait for shutdown signal
}
```

### Auth and allowlist

- New type: `NodeIdAllowlist`, stored in the daemon's config directory (e.g. `~/.config/mew/authorized_nodes.json`).
- On connect, check `connection.remote_id()` against the allowlist.
- If in pairing mode, the first connection's NodeId is added to the allowlist and pairing mode exits.

### `mew pair` CLI command

- Prints the daemon's NodeId and a relay URL.
- Prints a QR code (ASCII in the terminal; later an actual image for scanning).
- Puts the daemon into pairing mode for a short timeout or until the first connection.

### Config / state

- `iroh.enabled: bool` in `config.toml`? Or purely CLI-driven via `--iroh`?
- `authorized_nodes.json` as a sidecar file, not in `config.toml`, so `config.toml` can be shared without sharing device keys.
- The sidecar is updated automatically by the pairing flow; users do not edit it by hand.

### Test plan

- Unit test the `IrohStream` wrapper with a tokio duplex.
- Daemon integration test using `mew-provider-fake` and two tokio runtimes: one accepts iroh, one connects and sends `AttachSession` + `Prompt`.
- CLI test for `mew pair` output format (NodeId present, QR-like output).

### Done when

A Rust CLI peer on a different network can:

1. scan/ingest the daemon's NodeId,
2. connect over iroh,
3. attach to a session,
4. send a prompt,
5. receive streaming `ServerMessage`s.

---

## Decisions made

1. TCP bridge stays; iroh is supplemental.
2. `iroh` is an optional Cargo feature on `mew-daemon`.
3. Allowlist is managed through the pairing QR flow and persisted to a sidecar; users do not edit it manually.

## Open questions

1. Is stage 1 enough to merge, or should it wait until a CLI peer proves it end-to-end?
2. Do we want to pursue the WASM web-UI-over-iroh path before iOS, after iOS, or never?
