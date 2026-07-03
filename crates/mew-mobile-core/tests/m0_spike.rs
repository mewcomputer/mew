//! M0 spike test: mobile core connects to daemon over iroh, sends Ping, gets Pong.
//!
//! This test de-risks the two iOS unknowns:
//! 1. iroh cross-compiles for iOS targets (verified by `cargo check --target`)
//! 2. WS-client-over-QUIC handshake + Ping/Pong round-trip works
//!
//! The test runs on the host (not the simulator) but exercises the exact
//! code path that would run on the phone: MobileCore connects to a daemon
//! endpoint over iroh, does the WebSocket upgrade over the QUIC stream,
//! sends a Ping, and verifies it receives a Pong with the daemon version.

#![cfg(feature = "test-harness")]

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
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<IrohStream>;

fn fake_builder() -> mew_daemon::AgentBuilder {
    Arc::new(|params: mew_daemon::AgentBuildParams| {
        use mew_message::SessionId;
        let provider = Arc::new(FakeProvider::new(FakeProvider::text_response(
            "hello from mobile test",
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

async fn send(ws: &mut Ws, msg: ClientMessage) {
    let json = mew_protocol::encode_json(&msg).expect("encode");
    ws.send(Message::Text(json)).await.expect("send frame");
}

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

/// Full M0 round-trip: connect → Ping → Pong → NewSession → SessionReady →
/// Prompt → streaming PartStart + MessageEnd. This exercises the exact path
/// a mobile client would take.
#[tokio::test]
async fn mobile_core_ping_pong_round_trip() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let session_dir = dir.path().join("sessions");
    let allowlist_path = dir.path().join("authorized_nodes.json");

    let allowlist = Arc::new(NodeIdAllowlist::new(allowlist_path));

    let server = DaemonServer::with_session_dir(fake_builder(), session_dir.clone());
    let handler = MewIrohHandler {
        allowlist: allowlist.clone(),
        session_manager: server.session_manager.clone(),
        groups_store: server.groups_store.clone(),
        thinking_setter: None,
    };

    // Start the daemon endpoint.
    let daemon_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .alpns(vec![MEW_ALPN.to_vec()])
        .bind()
        .await?;
    let _ = tokio::time::timeout(Duration::from_secs(15), daemon_endpoint.online()).await;

    let router = iroh::protocol::Router::builder(daemon_endpoint.clone())
        .accept(MEW_ALPN, handler)
        .spawn();

    // Start the "mobile" client endpoint.
    let client_endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .bind()
        .await?;
    let _ = tokio::time::timeout(Duration::from_secs(15), client_endpoint.online()).await;

    // Pre-authorize the client.
    allowlist.add(&client_endpoint.id().to_string())?;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect — this is the path MobileCore::connect() takes.
    let daemon_addr = daemon_endpoint.addr();
    let conn = client_endpoint
        .connect(daemon_addr, MEW_ALPN)
        .await
        .expect("connect to daemon");

    // Open bi stream — must write immediately (Note #3: open_bi is lazy).
    let (send_stream, recv_stream) = conn.open_bi().await.expect("open bi stream");
    let iroh_stream = IrohStream::new(send_stream, recv_stream);

    // WS upgrade over QUIC — this is the de-risk path for M0.
    let req = ClientRequestBuilder::new("ws://daemon.mew/".parse().unwrap())
        .with_header("Host", "daemon.mew")
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let (mut ws, _resp) = client_async(req, iroh_stream)
        .await
        .expect("websocket handshake over iroh");

    // 1. Ping → Pong (version handshake)
    send(&mut ws, ClientMessage::Ping).await;
    let msgs = recv_until(&mut ws, |m| matches!(m, ServerMessage::Pong { .. })).await;
    match &msgs[0] {
        ServerMessage::Pong { version } => {
            println!("✓ Ping/Pong: daemon version = {version}");
            assert!(!version.is_empty(), "version should be non-empty");
        }
        _ => unreachable!(),
    }

    // 2. NewSession with ClientKind::Mobile → SessionReady
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: ClientKind::Mobile,
        },
    )
    .await;
    let msgs = recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;
    let session_id = match &msgs[0] {
        ServerMessage::SessionReady { session_id, .. } => session_id.clone(),
        _ => unreachable!(),
    };
    println!("✓ NewSession (Mobile): session_id = {session_id}");

    // 3. Prompt → streaming response
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "hello from mobile core".into(),
            attachments: vec![],
        },
    )
    .await;
    let msgs = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;
    let has_part_start = msgs.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: ProviderEventWire::PartStart { .. }
            }
        )
    });
    assert!(has_part_start, "should have received PartStart");
    println!("✓ Prompt → streaming response (PartStart + MessageEnd)");

    println!("\n✓✓✓ M0 round-trip complete: Ping/Pong + NewSession(Mobile) + Prompt/stream");

    let _ = router.shutdown().await;
    let _ = client_endpoint.close().await;
    Ok(())
}
