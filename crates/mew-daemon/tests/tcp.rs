//! TCP/WS listener tests for `mew-daemon`.
//!
//! The Unix-socket path is covered in `e2e.rs` and `concurrency.rs`. These
//! tests prove the TCP listener behaves equivalently: same wire protocol,
//! same per-connection isolation, same forwarding semantics.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use mew_agent::Agent;
use mew_daemon::DaemonServer;
use mew_hooks::NopDispatcher;
use mew_message::Finish;
use mew_protocol::{ClientMessage, ServerMessage};
use mew_provider_fake::FakeProvider;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<TcpStream>;

async fn spawn_daemon_tcp() -> SocketAddr {
    // Bind a kernel-assigned port (port 0) so multiple test runs don't
    // collide.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // release; the daemon re-binds

    let dir = tempfile::tempdir().expect("tempdir");
    let session_dir = dir.path().join("sessions");
    // Leak the TempDir so it outlives this function — the daemon task holds
    // no reference to it, but the session files must persist for the test
    // duration. The process exits after the test anyway.
    std::mem::forget(dir);
    let builder: mew_daemon::AgentBuilder = Arc::new(|params: mew_daemon::AgentBuildParams| {
        let provider = Arc::new(FakeProvider::new(FakeProvider::text_response("hi")));
        let dispatcher = Arc::new(NopDispatcher);
        let session_id: Option<mew_message::SessionId> = params
            .session_id
            .strip_prefix("sess_")
            .and_then(|s| ulid::Ulid::from_string(s).ok());
        Ok((
            Agent::new(
                provider,
                dispatcher,
                Some(params.writer),
                Vec::new(),
                session_id,
            ),
            None,
            None,
        ))
    });
    let server = DaemonServer::with_session_dir(builder, session_dir);
    tokio::spawn(async move {
        let _ = server.run_tcp(addr).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    addr
}

async fn connect_tcp(addr: SocketAddr) -> Ws {
    let stream = TcpStream::connect(addr).await.expect("tcp connect");
    let req = ClientRequestBuilder::new(format!("ws://{addr}/").parse().unwrap())
        .with_header("Host", format!("{addr}"))
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let (ws, _) = client_async(req, stream).await.expect("ws handshake");
    ws
}

async fn send(ws: &mut Ws, msg: ClientMessage) {
    let json = mew_protocol::encode_json(&msg).expect("encode");
    ws.send(Message::Text(json)).await.expect("send");
}

async fn recv_until<F>(ws: &mut Ws, mut pred: F) -> Vec<ServerMessage>
where
    F: FnMut(&ServerMessage) -> bool,
{
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            panic!("recv_until timed out; collected {out:?}");
        }
        let remaining = deadline - now;
        let next = tokio::time::timeout(remaining, ws.next())
            .await
            .expect("recv timed out")
            .expect("ws ended")
            .expect("ws err");
        let text = match next {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => return out,
            _ => continue,
        };
        let parsed: ServerMessage = mew_protocol::decode_json(&text).expect("decode");
        let done = pred(&parsed);
        out.push(parsed);
        if done {
            return out;
        }
    }
}

#[tokio::test]
async fn tcp_listener_serves_session_ready() {
    let addr = spawn_daemon_tcp().await;
    let mut ws = connect_tcp(addr).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;

    let msg = recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;
    assert!(matches!(
        msg.last().unwrap(),
        ServerMessage::SessionReady { .. }
    ));
}

#[tokio::test]
async fn new_session_reports_active_thinking_variant() {
    // A daemon whose agent has a preset thinking variant must surface it as
    // `ThinkingVariantChanged` right after `SessionReady`, so the TUI never
    // shows "no thinking" while the model is actually thinking.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let dir = tempfile::tempdir().expect("tempdir");
    let session_dir = dir.path().join("sessions");
    std::mem::forget(dir);
    let builder: mew_daemon::AgentBuilder = Arc::new(|params: mew_daemon::AgentBuildParams| {
        let provider = Arc::new(FakeProvider::new(FakeProvider::text_response("hi")));
        let dispatcher = Arc::new(NopDispatcher);
        let mut agent = Agent::new(provider, dispatcher, Some(params.writer), Vec::new(), None);
        agent.set_active_thinking_variant(Some("high".into()));
        Ok((agent, None, None))
    });
    let server = DaemonServer::with_session_dir(builder, session_dir);
    tokio::spawn(async move {
        let _ = server.run_tcp(addr).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut ws = connect_tcp(addr).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;

    let msgs = recv_until(
        &mut ws,
        |m| matches!(m, ServerMessage::ThinkingVariantChanged { variant } if variant.as_deref() == Some("high")),
    )
    .await;
    assert!(
        msgs.iter()
            .any(|m| matches!(m, ServerMessage::SessionReady { .. })),
        "SessionReady must precede ThinkingVariantChanged, got: {msgs:?}"
    );
    assert!(matches!(
        msgs.last().unwrap(),
        ServerMessage::ThinkingVariantChanged { variant } if variant.as_deref() == Some("high")
    ));
}

#[tokio::test]
async fn tcp_listener_streams_text_response() {
    let addr = spawn_daemon_tcp().await;
    let mut ws = connect_tcp(addr).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "hi".into(),
            attachments: vec![],
        },
    )
    .await;
    let events = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;

    let reassembled: String = events
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartDelta { delta, .. },
            } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reassembled, "hi");
}

#[tokio::test]
async fn tcp_listener_handles_invalid_json_with_server_error() {
    let addr = spawn_daemon_tcp().await;
    let mut ws = connect_tcp(addr).await;
    ws.send(Message::Text(r#"{"not":"a valid message"}"#.into()))
        .await
        .expect("send raw");

    let err = recv_until(&mut ws, |m| matches!(m, ServerMessage::Error { .. })).await;
    let last = err.last().unwrap();
    match last {
        ServerMessage::Error { message } => {
            assert!(message.contains("invalid message"), "{message:?}");
        }
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn tcp_listener_supports_concurrent_connections() {
    let addr = spawn_daemon_tcp().await;

    let mut a = connect_tcp(addr).await;
    let mut b = connect_tcp(addr).await;

    send(
        &mut a,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    send(
        &mut b,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;

    let sa = recv_until(&mut a, |m| matches!(m, ServerMessage::SessionReady { .. })).await;
    let sb = recv_until(&mut b, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    let id_a = match sa.last().unwrap() {
        ServerMessage::SessionReady { session_id, .. } => session_id.clone(),
        _ => unreachable!(),
    };
    let id_b = match sb.last().unwrap() {
        ServerMessage::SessionReady { session_id, .. } => session_id.clone(),
        _ => unreachable!(),
    };
    assert_ne!(
        id_a, id_b,
        "concurrent TCP connections get distinct session_ids"
    );
}

#[tokio::test]
async fn tcp_listener_accepts_tool_call_shaped_response() {
    // Spawn a daemon whose fake replays a tool-call shape, not a text shape.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let dir = tempfile::tempdir().expect("tempdir");
    let session_dir = dir.path().join("sessions");
    std::mem::forget(dir);
    let builder: mew_daemon::AgentBuilder = Arc::new(|params: mew_daemon::AgentBuildParams| {
        let provider = Arc::new(FakeProvider::new(FakeProvider::tool_call(
            "bash",
            "c1",
            serde_json::json!({"command": "ls"}),
        )));
        let dispatcher = Arc::new(NopDispatcher);
        let session_id: Option<mew_message::SessionId> = params
            .session_id
            .strip_prefix("sess_")
            .and_then(|s| ulid::Ulid::from_string(s).ok());
        Ok((
            Agent::new(
                provider,
                dispatcher,
                Some(params.writer),
                Vec::new(),
                session_id,
            ),
            None,
            None,
        ))
    });
    let server = DaemonServer::with_session_dir(builder, session_dir);
    tokio::spawn(async move {
        let _ = server.run_tcp(addr).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut ws = connect_tcp(addr).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "x".into(),
            attachments: vec![],
        },
    )
    .await;

    let events = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd {
                    finish: Finish::ToolUse,
                    ..
                }
            }
        )
    })
    .await;

    let saw_tool_call = events.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::ToolCall(tc),
                }
            } if tc.tool_name == "bash"
        )
    });
    assert!(
        saw_tool_call,
        "TCP listener must relay PartStart(ToolCall) identically to Unix path: {events:?}"
    );
}
