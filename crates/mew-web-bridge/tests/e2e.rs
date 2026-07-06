//! End-to-end tests for `mew-web-bridge`.
//!
//! These spawn a real `mew-daemon` on a Unix socket, the bridge on a TCP
//! port, and a test client on top of TCP. The bridge must relay frames
//! faithfully in both directions without reading or interpreting them.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use mew_agent::Agent;
use mew_daemon::DaemonServer;
use mew_hooks::NopDispatcher;
use mew_message::Finish;
use mew_protocol::{ClientMessage, ServerMessage};
use mew_provider_fake::FakeProvider;
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<TcpStream>;

struct TestStack {
    _dir: TempDir,
    _socket: String,
    tcp_addr: SocketAddr,
}

async fn spawn_stack() -> TestStack {
    // Daemon on a Unix socket.
    let dir = TempDir::new().expect("tempdir");
    let socket = dir.path().join("mew.sock");
    let socket_str = socket.to_string_lossy().to_string();
    let session_dir = dir.path().join("sessions");
    let builder: mew_daemon::AgentBuilder = Arc::new(|_| {
        let provider = Arc::new(FakeProvider::new(FakeProvider::text_response("hi")));
        let dispatcher = Arc::new(NopDispatcher);
        Ok((
            Agent::new(provider, dispatcher, None, Vec::new(), None),
            None,
            None,
        ))
    });
    let server = DaemonServer::with_session_dir(builder, session_dir);
    let socket_for_daemon = socket_str.clone();
    tokio::spawn(async move {
        let _ = server.run(&socket_for_daemon).await;
    });

    // Bridge on a kernel-assigned TCP port. Since the bridge binary is
    // spawned via a subprocess elsewhere (e.g. demo), for these tests we
    // replicate the bridge logic inline: a tiny TcpListener that accepts,
    // detects an Upgrade request, and proxies.
    let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = tcp_listener.local_addr().unwrap();

    let socket_str_for_bridge = socket_str.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = match tcp_listener.accept().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let socket_str = socket_str_for_bridge.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_bridge_conn(stream, &socket_str).await {
                    tracing::warn!(error = %e, "bridge conn error");
                }
            });
        }
    });

    // Give both servers a moment to bind.
    tokio::time::sleep(Duration::from_millis(50)).await;

    TestStack {
        _dir: dir,
        _socket: socket_str,
        tcp_addr,
    }
}

/// Minimal inline bridge: peek HTTP, route to daemon Unix WS, proxy frames.
/// Mirrors `crates/mew-web-bridge/src/main.rs::handle_connection`.
async fn handle_bridge_conn(stream: TcpStream, daemon_socket: &str) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut buf: BufReader<TcpStream> = BufReader::with_capacity(8192, stream);

    // Peek request line + headers without consuming.
    let peeked = buf.fill_buf().await?;
    if peeked.is_empty() {
        return Ok(());
    }
    let n = peeked.len().min(8192);
    let s = std::str::from_utf8(&peeked[..n])?.to_string();

    let mut lines = s.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or("");
    let _path = parts.next().unwrap_or("");

    let mut wants_ws = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let lk = k.trim().to_ascii_lowercase();
            if lk == "upgrade" && v.trim().eq_ignore_ascii_case("websocket") {
                wants_ws = true;
            }
        }
    }

    if !wants_ws {
        return Ok(());
    }

    // Hand the BufReader (still containing the request bytes) to tungstenite.
    let browser_ws = tokio_tungstenite::accept_async(buf).await?;

    // Connect to daemon.
    let daemon_stream = tokio::net::UnixStream::connect(daemon_socket).await?;
    let req = ClientRequestBuilder::new("ws://localhost/".parse().unwrap())
        .with_header("Host", "localhost")
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let (daemon_ws, _) = client_async(req, daemon_stream).await?;

    // Bidirectional proxy.
    let (mut b_tx, mut b_rx) = browser_ws.split();
    let (mut d_tx, mut d_rx) = daemon_ws.split();
    let b_to_d = async {
        while let Some(msg) = b_rx.next().await {
            if let Ok(m) = msg {
                if d_tx.send(m).await.is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    };
    let d_to_b = async {
        while let Some(msg) = d_rx.next().await {
            if let Ok(m) = msg {
                if b_tx.send(m).await.is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    };
    tokio::select! {
        _ = b_to_d => {}
        _ = d_to_b => {}
    }
    Ok(())
}

async fn connect_browser(addr: SocketAddr) -> Ws {
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
async fn bridge_relays_session_ready_to_browser() {
    let stack = spawn_stack().await;
    let mut browser = connect_browser(stack.tcp_addr).await;
    send(
        &mut browser,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;

    let msgs = recv_until(&mut browser, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;
    let last = msgs.last().unwrap();
    assert!(matches!(last, ServerMessage::SessionReady { .. }));
}

#[tokio::test]
async fn bridge_relays_streaming_text_from_daemon() {
    let stack = spawn_stack().await;
    let mut browser = connect_browser(stack.tcp_addr).await;
    send(
        &mut browser,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_until(&mut browser, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;

    send(
        &mut browser,
        ClientMessage::Prompt {
            text: "hi".into(),
            attachments: vec![],
        },
    )
    .await;
    let events = recv_until(&mut browser, |m| {
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
async fn bridge_proxies_concurrent_browser_sessions_independently() {
    let stack = spawn_stack().await;
    let mut a = connect_browser(stack.tcp_addr).await;
    let mut b = connect_browser(stack.tcp_addr).await;
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
    assert_ne!(id_a, id_b);
}

#[tokio::test]
async fn bridge_relays_invalid_json_as_server_error() {
    let stack = spawn_stack().await;
    let mut browser = connect_browser(stack.tcp_addr).await;
    // Skip NewSession; send raw garbage.
    browser
        .send(Message::Text(r#"{"not":"valid"}"#.into()))
        .await
        .expect("send raw");
    let msgs = recv_until(&mut browser, |m| matches!(m, ServerMessage::Error { .. })).await;
    let last = msgs.last().unwrap();
    match last {
        ServerMessage::Error { message } => assert!(message.contains("invalid message")),
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn bridge_handles_tool_use_finish_end_to_end() {
    // Spawn a stack whose daemon uses a tool-call script.
    let dir = TempDir::new().expect("tempdir");
    let socket: PathBuf = dir.path().join("mew.sock");
    let socket_str = socket.to_string_lossy().to_string();
    let session_dir = dir.path().join("sessions");
    let builder: mew_daemon::AgentBuilder = Arc::new(|_| {
        let provider = Arc::new(FakeProvider::new(FakeProvider::tool_call(
            "bash",
            "c1",
            serde_json::json!({"command": "ls"}),
        )));
        let dispatcher = Arc::new(NopDispatcher);
        Ok((
            Agent::new(provider, dispatcher, None, Vec::new(), None),
            None,
            None,
        ))
    });
    let server = DaemonServer::with_session_dir(builder, session_dir);
    let socket_for_daemon = socket_str.clone();
    tokio::spawn(async move {
        let _ = server.run(&socket_for_daemon).await;
    });

    let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = tcp_listener.local_addr().unwrap();
    let socket_str_for_bridge = socket_str.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = match tcp_listener.accept().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let socket_str = socket_str_for_bridge.clone();
            tokio::spawn(async move {
                let _ = handle_bridge_conn(stream, &socket_str).await;
            });
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut browser = connect_browser(tcp_addr).await;
    send(
        &mut browser,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_until(&mut browser, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;
    send(
        &mut browser,
        ClientMessage::Prompt {
            text: "x".into(),
            attachments: vec![],
        },
    )
    .await;
    let events = recv_until(&mut browser, |m| {
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
    let saw = events.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::ToolCall(tc),
                }
            } if tc.tool_name == "bash"
        )
    });
    assert!(saw, "expected PartStart(ToolCall) bash: {events:?}");
}
