//! Integration test: iroh peer connects to daemon, sends protocol messages.
//!
//! Spins up two iroh endpoints on the same machine using the `Minimal` preset
//! (no relay servers needed — direct localhost connections). The daemon side
//! uses `MewIrohHandler` with an allowlist that pre-adds the client's NodeId.
//! The client connects, does a WebSocket upgrade over the QUIC stream, and
//! exchanges `ClientMessage` / `ServerMessage` just like the TCP/Unix e2e tests.

#![cfg(feature = "iroh")]

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use mew_daemon::iroh_transport::{IrohStream, MewIrohHandler, NodeIdAllowlist, MEW_ALPN};
use mew_daemon::DaemonServer;
use mew_hooks::NopDispatcher;
use mew_message::ProviderEventWire;
use mew_protocol::{ClientKind, ClientMessage, ServerMessage};
use mew_provider_fake::FakeProvider;
use tempfile::TempDir;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<IrohStream>;

/// Build a fake-provider agent builder (same as e2e.rs).
fn fake_builder() -> mew_daemon::AgentBuilder {
    Arc::new(|params: mew_daemon::AgentBuildParams| {
        use mew_message::SessionId;
        let provider = Arc::new(FakeProvider::new(FakeProvider::text_response(
            "hello from iroh test",
        )));
        let dispatcher = Arc::new(NopDispatcher);
        let session_id: Option<SessionId> = params
            .session_id
            .strip_prefix("sess_")
            .and_then(|s| ulid::Ulid::from_string(s).ok());
        Ok((
            {
                let mut a = mew_agent::Agent::new(
                    provider,
                    dispatcher,
                    Some(params.writer),
                    Vec::new(),
                    session_id,
                );
                a.set_model_info("fake", "fake");
                a
            },
            Some("fake".to_string()),
            Some("fake".to_string()),
        ))
    })
}

/// Send a client message as a JSON text frame.
async fn send(ws: &mut Ws, msg: ClientMessage) {
    let json = mew_protocol::encode_json(&msg).expect("encode");
    ws.send(Message::Text(json)).await.expect("send frame");
}

/// Receive server messages until the predicate returns true, or timeout.
async fn recv_until<F>(ws: &mut Ws, mut pred: F) -> Vec<ServerMessage>
where
    F: FnMut(&ServerMessage) -> bool,
{
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            panic!(
                "recv_until timed out; collected {} messages: {:?}",
                out.len(),
                out
            );
        }
        let remaining = deadline - now;
        let next = tokio::time::timeout(remaining, ws.next())
            .await
            .expect("recv timed out")
            .expect("ws stream ended")
            .expect("ws recv error");
        let text = match next {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => panic!("ws closed by peer"),
            other => panic!("unexpected frame: {:?}", other),
        };
        let msg: ServerMessage = mew_protocol::decode_json(&text).expect("decode server message");
        out.push(msg.clone());
        if pred(&msg) {
            return out;
        }
    }
}

#[tokio::test]
async fn iroh_peer_connects_and_exchanges_protocol() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let session_dir = dir.path().join("sessions");
    let allowlist_path = dir.path().join("authorized_nodes.json");

    // Create the allowlist — we'll add the client's NodeId after creating
    // the client endpoint.
    let allowlist = Arc::new(NodeIdAllowlist::new(allowlist_path));

    // Build the daemon server.
    let server = DaemonServer::with_session_dir(fake_builder(), session_dir.clone());
    let handler = MewIrohHandler {
        allowlist: allowlist.clone(),
        session_manager: server.session_manager.clone(),
        groups_store: server.groups_store.clone(),
        thinking_setter: None,
    };

    // Create the daemon (accept) endpoint.
    // Use N0 preset (default relay + discovery) so the endpoints can find each other.
    let daemon_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .alpns(vec![MEW_ALPN.to_vec()])
        .bind()
        .await?;
    // Wait for endpoint to come online (relays + direct addresses).
    let _ = tokio::time::timeout(Duration::from_secs(15), daemon_endpoint.online()).await;

    let router = iroh::protocol::Router::builder(daemon_endpoint.clone())
        .accept(MEW_ALPN, handler)
        .spawn();

    // Create the client endpoint.
    let client_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .bind()
        .await?;
    let _ = tokio::time::timeout(Duration::from_secs(15), client_endpoint.online()).await;
    let client_node_id = client_endpoint.id();

    // Pre-authorize the client in the allowlist.
    allowlist.add(&client_node_id.to_string())?;

    // Give the daemon endpoint a moment to be ready.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client connects to the daemon.
    let daemon_addr = daemon_endpoint.addr();
    let conn = client_endpoint
        .connect(daemon_addr, MEW_ALPN)
        .await
        .expect("connect to daemon");

    // Open a bidirectional stream.
    let (send_stream, recv_stream) = conn.open_bi().await.expect("open bi stream");
    let iroh_stream = IrohStream::new(send_stream, recv_stream);

    // WebSocket upgrade over the QUIC stream.
    let req = ClientRequestBuilder::new("ws://iroh/".parse().unwrap())
        .with_header("Host", "iroh")
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let (mut ws, _resp) = client_async(req, iroh_stream)
        .await
        .expect("websocket handshake over iroh");

    // Send NewSession.
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: ClientKind::Unknown,
        },
    )
    .await;

    // Expect SessionReady.
    let msgs = recv_until(&mut ws, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;
    let session_id = match &msgs[0] {
        ServerMessage::SessionReady { session_id, .. } => session_id.clone(),
        _ => unreachable!(),
    };
    assert!(!session_id.is_empty(), "session id should be non-empty");

    // Send a prompt.
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "hello from iroh test".into(),
            attachments: vec![],
        },
    )
    .await;

    // Collect streaming messages until we get a MessageEnd.
    let msgs = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;

    // Verify we got at least one PartStart before the MessageEnd.
    let has_text = msgs.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: ProviderEventWire::PartStart { .. }
            }
        )
    });
    assert!(has_text, "should have received at least one part start");

    // Clean up.
    let _ = router.shutdown().await;
    let _ = client_endpoint.close().await;

    Ok(())
}

#[tokio::test]
async fn iroh_unauthorized_peer_is_rejected() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let session_dir = dir.path().join("sessions");
    let allowlist_path = dir.path().join("authorized_nodes.json");

    // Empty allowlist — nobody is authorized.
    let allowlist = Arc::new(NodeIdAllowlist::new(allowlist_path));

    let server = DaemonServer::with_session_dir(fake_builder(), session_dir.clone());
    let handler = MewIrohHandler {
        allowlist: allowlist.clone(),
        session_manager: server.session_manager.clone(),
        groups_store: server.groups_store.clone(),
        thinking_setter: None,
    };

    let daemon_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .alpns(vec![MEW_ALPN.to_vec()])
        .bind()
        .await?;
    let _ = tokio::time::timeout(Duration::from_secs(15), daemon_endpoint.online()).await;

    let router = iroh::protocol::Router::builder(daemon_endpoint.clone())
        .accept(MEW_ALPN, handler)
        .spawn();

    // Create a client endpoint NOT in the allowlist.
    let client_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .bind()
        .await?;
    let _ = tokio::time::timeout(Duration::from_secs(15), client_endpoint.online()).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    let daemon_addr = daemon_endpoint.addr();
    // The QUIC connection may succeed (handshake completes) but the handler
    // closes it immediately. We verify the connection is rejected by attempting
    // to open a stream and checking it fails or the connection is closed.
    let conn = client_endpoint
        .connect(daemon_addr, MEW_ALPN)
        .await
        .expect("QUIC connect should succeed");

    // The handler will close the connection with error code 1.
    // Wait for the connection to close.
    let closed = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    assert!(
        closed.is_ok(),
        "connection should be closed by daemon (rejected)"
    );

    let _ = router.shutdown().await;
    let _ = client_endpoint.close().await;

    Ok(())
}

// Keep the TempDir alive for the duration of the test.
#[allow(dead_code)]
fn _ensure_tempdir_type() -> TempDir {
    tempfile::tempdir().unwrap()
}
