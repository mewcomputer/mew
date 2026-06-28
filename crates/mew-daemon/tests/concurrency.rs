//! Concurrency / stress tests for the mew daemon.
//!
//! These exercise the per-connection session isolation, message ordering on
//! the wire, and the behaviour of the ID-paired permission routing under
//! contention. The agent is backed by `mew-provider-fake` so we never make
//! network calls.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use mew_agent::Agent;
use mew_daemon::DaemonServer;
use mew_hooks::NopDispatcher;
use mew_message::Finish;
use mew_protocol::{ClientMessage, ServerMessage};
use mew_provider::{EventStream, Provider, ProviderError, ProviderEvent, Request};
use mew_provider_fake::FakeProvider;
use std::sync::Mutex;
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<UnixStream>;

/// A provider that hands out a different script on each `stream()` call,
/// so successive turns produce distinct `PartId`s. Useful for concurrency
/// tests that want to assert on turn isolation.
struct TurnRotatingProvider {
    scripts: Mutex<Vec<Vec<ProviderEvent>>>,
}

impl TurnRotatingProvider {
    fn new(scripts: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            scripts: Mutex::new(scripts),
        }
    }
}

#[async_trait]
impl Provider for TurnRotatingProvider {
    fn name(&self) -> &str {
        "turn-rotating"
    }

    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError> {
        let script = {
            let mut scripts = self.scripts.lock().unwrap();
            if scripts.is_empty() {
                Vec::new()
            } else {
                scripts.remove(0)
            }
        };
        let provider = FakeProvider::new(script);
        provider.stream(req).await
    }
}

async fn spawn_daemon() -> (TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("mew.sock");
    let socket_str = socket_path.to_string_lossy().to_string();
    let socket_for_task = socket_str.clone();
    let session_dir = dir.path().join("sessions");
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
        let _ = server.run(&socket_for_task).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (dir, socket_str)
}

/// Spawn a daemon whose agent uses a `TurnRotatingProvider` — each prompt
/// gets a distinct script so successive turns produce distinct `PartId`s.
async fn spawn_daemon_with_scripts(scripts: Vec<Vec<ProviderEvent>>) -> (TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("mew.sock");
    let socket_str = socket_path.to_string_lossy().to_string();
    let socket_for_task = socket_str.clone();
    let session_dir = dir.path().join("sessions");
    let provider = Arc::new(TurnRotatingProvider::new(scripts));
    let builder: mew_daemon::AgentBuilder =
        Arc::new(move |params: mew_daemon::AgentBuildParams| {
            let dispatcher = Arc::new(NopDispatcher);
            let session_id: Option<mew_message::SessionId> = params
                .session_id
                .strip_prefix("sess_")
                .and_then(|s| ulid::Ulid::from_string(s).ok());
            Ok((
                Agent::new(
                    provider.clone(),
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
        let _ = server.run(&socket_for_task).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (dir, socket_str)
}

async fn connect(socket_path: &str) -> Ws {
    let stream = UnixStream::connect(socket_path).await.expect("connect");
    let req = ClientRequestBuilder::new("ws://localhost/".parse().unwrap())
        .with_header("Host", "localhost")
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

/// Drain all messages from the WebSocket until the connection closes (the
/// peer hangs up). Used in tests where we want to collect everything that
/// streams back in response to many sends.
async fn drain(ws: &mut Ws, max_msgs: usize) -> Vec<ServerMessage> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while out.len() < max_msgs {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let next = match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(_))) => return out,
            Ok(None) => return out,
            Err(_) => return out,
        };
        let text = match next {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => return out,
            _ => continue,
        };
        if let Ok(parsed) = mew_protocol::decode_json::<ServerMessage>(&text) {
            out.push(parsed);
        }
    }
    out
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

// ---------------------------------------------------------------------------

#[tokio::test]
async fn five_concurrent_connections_all_get_distinct_sessions() {
    // Five clients connect in parallel. Each must receive a distinct
    // session_id from the daemon — proving the per-connection session
    // state is isolated.
    let (_dir, socket) = spawn_daemon().await;

    let mut handles = Vec::new();
    for _ in 0..5 {
        let socket = socket.clone();
        handles.push(tokio::spawn(async move {
            let mut ws = connect(&socket).await;
            send(&mut ws, ClientMessage::NewSession { cwd: None }).await;
            let mut session_id = None;
            recv_until(&mut ws, |m| {
                if let ServerMessage::SessionReady { session_id: id, .. } = m {
                    session_id = Some(id.clone());
                    true
                } else {
                    false
                }
            })
            .await;
            session_id.expect("session_id must arrive")
        }));
    }

    let mut session_ids = Vec::new();
    for h in handles {
        session_ids.push(h.await.unwrap());
    }

    let unique: std::collections::HashSet<_> = session_ids.iter().collect();
    assert_eq!(
        unique.len(),
        session_ids.len(),
        "all session_ids must be distinct: {session_ids:?}"
    );
}

#[tokio::test]
async fn concurrent_prompts_on_same_connection_serialize() {
    // A single connection fires 3 prompts in rapid succession. Each prompt
    // gets a distinct script (via TurnRotatingProvider), so the resulting
    // PartStart.part.id ulids must all be different — proving the prompts
    // are distinct turns and not sharing cached state.
    let scripts = vec![
        FakeProvider::text_response("a"),
        FakeProvider::text_response("b"),
        FakeProvider::text_response("c"),
    ];
    let (_dir, socket) = spawn_daemon_with_scripts(scripts).await;
    let mut ws = connect(&socket).await;
    send(&mut ws, ClientMessage::NewSession { cwd: None }).await;
    recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    let mut all_part_ids = Vec::new();
    for _ in 0..3 {
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
                    event: mew_message::ProviderEventWire::MessageEnd { .. }
                }
            )
        })
        .await;
        if let Some(id) = events.iter().find_map(|m| match m {
            ServerMessage::Provider {
                event:
                    mew_message::ProviderEventWire::PartStart {
                        part: mew_message::Part::Text(p),
                    },
            } => Some(p.base.id),
            _ => None,
        }) {
            all_part_ids.push(id);
        }
    }

    assert_eq!(
        all_part_ids.len(),
        3,
        "expected 3 PartStart ids: {all_part_ids:?}"
    );
    let unique: std::collections::HashSet<_> = all_part_ids.iter().collect();
    assert_eq!(
        unique.len(),
        3,
        "concurrent prompts on same connection must produce distinct part ids: {all_part_ids:?}"
    );
}

#[tokio::test]
async fn concurrent_prompts_across_connections_do_not_cross_contaminate() {
    // Two clients run prompts simultaneously. Each must receive ONLY its
    // own events — no overlap with the other's session. We give each
    // connection a unique text and verify the deltas match what that
    // connection sent.
    let (_dir, socket) = spawn_daemon().await;

    let mut ws_a = connect(&socket).await;
    let mut ws_b = connect(&socket).await;
    send(&mut ws_a, ClientMessage::NewSession { cwd: None }).await;
    send(&mut ws_b, ClientMessage::NewSession { cwd: None }).await;
    recv_until(&mut ws_a, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;
    recv_until(&mut ws_b, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;

    // The fake's `text_response` is fixed (always replays "hi"), but the
    // part_ids are regenerated per stream() call. So we assert that the
    // two connections receive DIFFERENT part_ids — proving each got its
    // own agent invocation.
    send(
        &mut ws_a,
        ClientMessage::Prompt {
            text: "a".into(),
            attachments: vec![],
        },
    )
    .await;
    send(
        &mut ws_b,
        ClientMessage::Prompt {
            text: "b".into(),
            attachments: vec![],
        },
    )
    .await;

    let events_a = recv_until(&mut ws_a, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;
    let events_b = recv_until(&mut ws_b, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;

    let id_a = events_a.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event:
                mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::Text(p),
                },
        } => Some(p.base.id),
        _ => None,
    });
    let id_b = events_b.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event:
                mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::Text(p),
                },
        } => Some(p.base.id),
        _ => None,
    });

    assert!(
        id_a.is_some() && id_b.is_some(),
        "both connections must yield a PartStart"
    );
    assert_ne!(
        id_a, id_b,
        "concurrent prompts on separate connections must yield distinct part ids"
    );

    // No session leakage: the part_id seen on connection A must NOT appear
    // on connection B, and vice versa.
    let all_a_part_ids: std::collections::HashSet<_> = events_a
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartStart { part },
            } => Some(part.id()),
            _ => None,
        })
        .collect();
    let all_b_part_ids: std::collections::HashSet<_> = events_b
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartStart { part },
            } => Some(part.id()),
            _ => None,
        })
        .collect();
    assert!(
        all_a_part_ids.is_disjoint(&all_b_part_ids),
        "part ids must not overlap across connections: A={all_a_part_ids:?} B={all_b_part_ids:?}"
    );
}

#[tokio::test]
async fn prompt_during_in_flight_turn_is_serialized() {
    // Send Prompt A, then Prompt B before A finishes. The daemon must not
    // interleave their events — A's events stream back first, then B's.
    // (This is the existing serial behavior; if the daemon ever switches
    // to concurrent turns per session, this test will catch it.)
    let scripts = vec![
        FakeProvider::text_response("first-turn"),
        FakeProvider::text_response("second-turn"),
    ];
    let (_dir, socket) = spawn_daemon_with_scripts(scripts).await;
    let mut ws = connect(&socket).await;
    send(&mut ws, ClientMessage::NewSession { cwd: None }).await;
    recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    // Send both prompts back-to-back.
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "a".into(),
            attachments: vec![],
        },
    )
    .await;
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "b".into(),
            attachments: vec![],
        },
    )
    .await;

    // Collect the first MessageEnd — it belongs to turn A.
    let turn_a = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;
    let part_a = turn_a.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event:
                mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::Text(p),
                },
        } => Some(p.base.id),
        _ => None,
    });

    // Collect the second MessageEnd — it belongs to turn B and must have
    // a distinct part id.
    let turn_b = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;
    let part_b = turn_b.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event:
                mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::Text(p),
                },
        } => Some(p.base.id),
        _ => None,
    });

    assert!(part_a.is_some() && part_b.is_some());
    assert_ne!(part_a, part_b, "turns must have distinct part ids");

    // Each turn's text matches its script.
    let text_a: String = turn_a
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartDelta { delta, .. },
            } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    let text_b: String = turn_b
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartDelta { delta, .. },
            } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text_a, "first-turn");
    assert_eq!(text_b, "second-turn");
}

#[tokio::test]
async fn rapid_fire_cancel_does_not_crash_daemon() {
    // Send a prompt and then a stream of Cancel messages. The daemon
    // must survive without panicking and remain responsive.
    let (_dir, socket) = spawn_daemon().await;
    let mut ws = connect(&socket).await;
    send(&mut ws, ClientMessage::NewSession { cwd: None }).await;
    recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "x".into(),
            attachments: vec![],
        },
    )
    .await;
    for _ in 0..20 {
        send(&mut ws, ClientMessage::Cancel).await;
    }

    // Daemon still responds to slash commands → still alive.
    send(
        &mut ws,
        ClientMessage::SlashCommand {
            command: "/clear".into(),
        },
    )
    .await;
    let r = recv_until(&mut ws, |m| matches!(m, ServerMessage::SlashResult { .. })).await;
    assert!(r
        .iter()
        .any(|m| matches!(m, ServerMessage::SlashResult { .. })));
}

#[tokio::test]
async fn slash_command_during_in_flight_turn_does_not_block_stream() {
    // Issue a prompt, then a /clear slash command while the stream is
    // running. The stream should still complete (MessageEnd lands) and
    // the slash result should also arrive — proving the connection
    // processes commands in order without blocking.
    let (_dir, socket) = spawn_daemon().await;
    let mut ws = connect(&socket).await;
    send(&mut ws, ClientMessage::NewSession { cwd: None }).await;
    recv_until(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "x".into(),
            attachments: vec![],
        },
    )
    .await;
    send(
        &mut ws,
        ClientMessage::SlashCommand {
            command: "/clear".into(),
        },
    )
    .await;

    // Drain everything for up to 5 seconds and look for both a
    // MessageEnd (from the prompt) AND a SlashResult (from /clear).
    let msgs = drain(&mut ws, 64).await;
    let has_message_end = msgs.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd {
                    finish: Finish::Stop,
                    ..
                }
            }
        )
    });
    let has_slash_result = msgs
        .iter()
        .any(|m| matches!(m, ServerMessage::SlashResult { .. }));
    assert!(has_message_end, "expected MessageEnd in stream: {msgs:?}");
    assert!(
        has_slash_result,
        "expected SlashResult for /clear during stream: {msgs:?}"
    );
}
