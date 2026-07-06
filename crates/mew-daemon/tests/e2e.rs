//! End-to-end tests for the mew daemon.
//!
//! These tests spawn a real `DaemonServer` on a temp Unix socket, connect a
//! WebSocket client over the Unix stream, and exercise the full
//! `ClientMessage` ↔ `ServerMessage` protocol. The agent is backed by
//! `mew-provider-fake` so we never make network calls.
//!
//! The tests use a raw WebSocket client over a `UnixStream` rather than the
//! production `DaemonClient` so we can assert on the wire shape directly and
//! so connection plumbing isn't tested through itself.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use mew_agent::{Agent, AgentEvent};
use mew_daemon::DaemonServer;
use mew_hooks::NopDispatcher;
use mew_message::{Finish, PartId, TextPart, ToolCallPart, ToolState, ToolStatePending, ToolTime};
use mew_protocol::{ClientMessage, ServerMessage};
use mew_provider_fake::FakeProvider;
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<UnixStream>;

/// Spawn a daemon bound to a temp Unix socket. The server runs in a
/// background task until the `TempDir` (and the socket path inside it) is
/// dropped. Each connection builds a fresh agent via `agent_factory`.
async fn spawn_daemon<F>(agent_factory: F) -> (TempDir, String)
where
    F: Fn(mew_daemon::AgentBuildParams) -> Result<(Agent, Option<String>, Option<String>)>
        + Send
        + Sync
        + 'static,
{
    let dir = tempfile::tempdir().expect("create tempdir");
    let socket_path = dir.path().join("mew.sock");
    let socket_str = socket_path.to_string_lossy().to_string();
    let socket_for_task = socket_str.clone();
    let session_dir = dir.path().join("sessions");
    let builder: mew_daemon::AgentBuilder = Arc::new(agent_factory);
    let server = DaemonServer::with_session_dir(builder, session_dir);
    tokio::spawn(async move {
        let _ = server.run(&socket_for_task).await;
    });
    // Give the listener a moment to bind. The OS-level accept race is
    // short; a tiny sleep is more reliable than polling.
    tokio::time::sleep(Duration::from_millis(50)).await;
    (dir, socket_str)
}

/// Connect a WebSocket client to the daemon over its Unix socket.
async fn connect(socket_path: &str) -> Ws {
    let stream = UnixStream::connect(socket_path)
        .await
        .expect("connect to daemon unix socket");
    let req = ClientRequestBuilder::new("ws://localhost/".parse().unwrap())
        .with_header("Host", "localhost")
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let (ws, _resp) = client_async(req, stream)
        .await
        .expect("websocket handshake");
    ws
}

/// Send a client message as a JSON text frame.
async fn send(ws: &mut Ws, msg: ClientMessage) {
    let json = mew_protocol::encode_json(&msg).expect("encode");
    ws.send(Message::Text(json)).await.expect("send frame");
}

/// Receive server messages until the predicate returns true for one, or
/// until the timeout fires (test fails).
async fn recv_until<F>(ws: &mut Ws, mut pred: F) -> Vec<ServerMessage>
where
    F: FnMut(&ServerMessage) -> bool,
{
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            panic!(
                "recv_until timed out; collected {} messages so far: {:?}",
                out.len(),
                out
            );
        }
        let remaining = deadline - now;
        let next = tokio::time::timeout(remaining, ws.next())
            .await
            .expect("recv_until timed out")
            .expect("ws stream ended")
            .expect("ws recv error");
        let text = match next {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => panic!("ws closed by peer"),
            other => panic!("unexpected non-text frame: {:?}", other),
        };
        let parsed: ServerMessage = mew_protocol::decode_json(&text).expect("decode");
        let done = pred(&parsed);
        out.push(parsed);
        if done {
            return out;
        }
    }
}

/// Recv exactly one matching message, discarding prior messages.
async fn recv_one_matching<F>(ws: &mut Ws, pred: F) -> ServerMessage
where
    F: Fn(&ServerMessage) -> bool,
{
    let collected = recv_until(ws, pred).await;
    collected.into_iter().last().expect("at least one match")
}

/// Build an agent backed by a `FakeProvider` with no tools — just enough to
/// stream text and respond to slash commands. The `script_fn` lets each test
/// inject a specific event script per connection.
fn make_text_agent_factory(
    script_fn: Arc<dyn Fn() -> Vec<mew_provider::ProviderEvent> + Send + Sync>,
) -> impl Fn(mew_daemon::AgentBuildParams) -> Result<(Agent, Option<String>, Option<String>)>
       + Send
       + Sync
       + 'static {
    move |params: mew_daemon::AgentBuildParams| {
        let script = script_fn();
        let provider = Arc::new(FakeProvider::new(script));
        let dispatcher = Arc::new(NopDispatcher);
        let session_id: Option<mew_message::SessionId> = params
            .session_id
            .strip_prefix("sess_")
            .and_then(|s| ulid::Ulid::from_string(s).ok());
        let agent = Agent::new(
            provider,
            dispatcher,
            Some(params.writer),
            Vec::new(),
            session_id,
        );
        Ok((agent, None, None))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn new_session_returns_session_ready() {
    let script = Arc::new(|| FakeProvider::text_response("hello"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;

    let msg = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    let session_id = match msg {
        ServerMessage::SessionReady { session_id, .. } => session_id,
        _ => unreachable!(),
    };
    assert!(
        session_id.starts_with("sess_"),
        "session_id should start with sess_, got {session_id:?}"
    );
}

#[tokio::test]
async fn prompt_streams_text_response_events() {
    let script = Arc::new(|| FakeProvider::text_response("hello world"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "hi".into(),
            attachments: vec![],
        },
    )
    .await;

    // Collect until MessageEnd lands, then sanity-check the event shape.
    let events = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;

    // The fake emits PartStart + PartDelta* + PartEnd + MessageEnd.
    let has_part_start = events.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartStart { .. }
            }
        )
    });
    let has_deltas = events.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartDelta { .. }
            }
        )
    });
    let has_message_end = events.iter().any(|m| {
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
    assert!(has_part_start, "expected PartStart in stream: {events:?}");
    assert!(has_deltas, "expected PartDelta in stream: {events:?}");
    assert!(
        has_message_end,
        "expected MessageEnd(Stop) in stream: {events:?}"
    );

    // The deltas should reassemble to "hello world".
    let reassembled: String = events
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartDelta { delta, .. },
            } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reassembled, "hello world");
}

#[tokio::test]
async fn prompt_without_new_session_returns_error() {
    let script = Arc::new(|| FakeProvider::text_response("anything"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    // Skip NewSession; go straight to Prompt.
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "hi".into(),
            attachments: vec![],
        },
    )
    .await;

    let err = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::Error { .. })).await;
    match err {
        ServerMessage::Error { message } => {
            assert!(
                message.contains("no session"),
                "expected 'no session' error, got {message:?}"
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn invalid_json_returns_server_error() {
    let script = Arc::new(|| FakeProvider::text_response("x"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    // Send raw garbage — not even a valid ClientMessage envelope.
    ws.send(Message::Text(r#"{"not":"a valid client message"}"#.into()))
        .await
        .expect("send raw frame");

    let err = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::Error { .. })).await;
    match err {
        ServerMessage::Error { message } => {
            assert!(
                message.contains("invalid message"),
                "expected 'invalid message' error, got {message:?}"
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn slash_command_clear_returns_slash_result() {
    let script = Arc::new(|| FakeProvider::text_response(""));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::SlashCommand {
            command: "/clear".into(),
        },
    )
    .await;
    let r = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SlashResult { .. })).await;
    match r {
        ServerMessage::SlashResult { text } => {
            assert!(
                text.contains("cleared"),
                "expected slash result mentioning cleared, got {text:?}"
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn slash_command_compact_returns_slash_result() {
    let script = Arc::new(|| FakeProvider::text_response(""));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::SlashCommand {
            command: "/compact".into(),
        },
    )
    .await;
    let r = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SlashResult { .. })).await;
    match r {
        ServerMessage::SlashResult { text } => {
            assert!(
                text.contains("compaction"),
                "expected slash result mentioning compaction, got {text:?}"
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn cancel_during_stream_does_not_panic() {
    // The fake provider streams its script quickly. Cancel after the first
    // event and verify the daemon survives (no panic, connection stays
    // open enough to send another message).
    let script = Arc::new(|| FakeProvider::text_response("a long stream of text content"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "hi".into(),
            attachments: vec![],
        },
    )
    .await;

    // Immediately cancel — the fake has 10ms between events, so we should
    // catch the stream mid-flight.
    send(&mut ws, ClientMessage::Cancel).await;

    // The connection should stay open; sending another slash command
    // should still produce a SlashResult. This proves the daemon survived.
    send(
        &mut ws,
        ClientMessage::SlashCommand {
            command: "/clear".into(),
        },
    )
    .await;
    let r = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SlashResult { .. })).await;
    assert!(matches!(r, ServerMessage::SlashResult { .. }));
}

#[tokio::test]
async fn tool_call_response_emits_tool_use_finish() {
    // The FakeProvider's `tool_call` helper produces a single ToolCall
    // followed by MessageEnd(ToolUse). The daemon must faithfully relay.
    let script = Arc::new(|| {
        FakeProvider::tool_call("bash", "call_1", serde_json::json!({"command": "ls"}))
    });
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "list files".into(),
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

    // We should see at least one PartStart(ToolCall) carrying our tool name.
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
        "expected PartStart(ToolCall) with tool_name=bash: {events:?}"
    );
}

#[tokio::test]
async fn multiple_sequential_prompts_each_get_session() {
    // One connection, one session, two prompts back to back. The daemon's
    // session state must reset per agent.run() — neither prompt should
    // contaminate the other.
    let script = Arc::new(|| FakeProvider::text_response("second"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    // First prompt: drain until MessageEnd.
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "first".into(),
            attachments: vec![],
        },
    )
    .await;
    let _ = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;

    // Second prompt: drain until next MessageEnd.
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "second".into(),
            attachments: vec![],
        },
    )
    .await;
    let events2 = recv_until(&mut ws, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;
    let reassembled: String = events2
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartDelta { delta, .. },
            } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reassembled, "second");
}

#[tokio::test]
async fn concurrent_connections_are_independent() {
    // Two clients connect simultaneously, each runs its own session. The
    // sessions must not share event streams.
    let script = Arc::new(|| FakeProvider::text_response("hi"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws_a = connect(&socket).await;
    let mut ws_b = connect(&socket).await;

    send(
        &mut ws_a,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    send(
        &mut ws_b,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;

    let sa = recv_one_matching(&mut ws_a, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;
    let sb = recv_one_matching(&mut ws_b, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;

    let id_a = match sa {
        ServerMessage::SessionReady { session_id, .. } => session_id,
        _ => unreachable!(),
    };
    let id_b = match sb {
        ServerMessage::SessionReady { session_id, .. } => session_id,
        _ => unreachable!(),
    };
    assert_ne!(
        id_a, id_b,
        "two concurrent sessions should have distinct session_ids"
    );
}

#[tokio::test]
async fn part_id_consistent_across_part_start_and_part_end() {
    // Verify the streaming pipeline preserves PartId across the start
    // and end of a TextPart. (PartUpdated isn't on the wire for the
    // fake-driven path; pinning the PartStart.id == PartEnd.id contract.)
    let script = Arc::new(|| FakeProvider::text_response("ping"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;
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
    let part_id_from_start = events.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event:
                mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::Text(TextPart { base, .. }),
                },
        } => Some(base.id),
        _ => None,
    });
    let part_id_from_end = events.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event: mew_message::ProviderEventWire::PartEnd { part_id },
        } => Some(*part_id),
        _ => None,
    });
    if let (Some(s), Some(e)) = (part_id_from_start, part_id_from_end) {
        assert_eq!(s, e, "PartStart.id must match the corresponding PartEnd.id");
    }
}

#[tokio::test]
async fn fresh_agent_per_connection() {
    // A fresh agent is built per connection. Send the same prompt to two
    // connections; each must produce its own event sequence (rather than
    // sharing a single agent). The provider's PartStart ulid should differ.
    let script = Arc::new(|| FakeProvider::text_response("x"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws1 = connect(&socket).await;
    send(
        &mut ws1,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws1, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;
    send(
        &mut ws1,
        ClientMessage::Prompt {
            text: "x".into(),
            attachments: vec![],
        },
    )
    .await;
    let e1 = recv_until(&mut ws1, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;
    let part1 = e1.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event:
                mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::Text(TextPart { base, .. }),
                },
        } => Some(base.id),
        _ => None,
    });

    drop(ws1);

    let mut ws2 = connect(&socket).await;
    send(
        &mut ws2,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws2, |m| {
        matches!(m, ServerMessage::SessionReady { .. })
    })
    .await;
    send(
        &mut ws2,
        ClientMessage::Prompt {
            text: "x".into(),
            attachments: vec![],
        },
    )
    .await;
    let e2 = recv_until(&mut ws2, |m| {
        matches!(
            m,
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::MessageEnd { .. }
            }
        )
    })
    .await;
    let part2 = e2.iter().find_map(|m| match m {
        ServerMessage::Provider {
            event:
                mew_message::ProviderEventWire::PartStart {
                    part: mew_message::Part::Text(TextPart { base, .. }),
                },
        } => Some(base.id),
        _ => None,
    });

    assert_ne!(
        part1, part2,
        "fresh connection should yield a new agent with a new part id"
    );
}

// Quiet unused-import warnings for items only used in commented scaffolding.
#[allow(dead_code)]
fn _unused_type_anchors(
    _: AgentEvent,
    _: ToolCallPart,
    _: ToolStatePending,
    _: ToolState,
    _: ToolTime,
    _: PartId,
) {
}

#[tokio::test]
async fn ping_returns_pong_with_version() {
    let script = Arc::new(|| FakeProvider::text_response("ok"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(&mut ws, ClientMessage::Ping).await;

    let msg = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::Pong { .. })).await;
    match msg {
        ServerMessage::Pong { version } => {
            assert!(
                !version.is_empty(),
                "version should be non-empty, got {version:?}"
            );
        }
        _ => unreachable!("expected Pong"),
    }
}

#[tokio::test]
async fn list_projects_returns_project_list() {
    let script = Arc::new(|| FakeProvider::text_response("ok"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    // Create a session with a known cwd so there's at least one project.
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: Some("/tmp".into()),
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;
    recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::SessionReady { .. })).await;

    // Now ask for the project list.
    send(&mut ws, ClientMessage::ListProjects).await;
    let msg = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::ProjectList { .. })).await;
    match msg {
        ServerMessage::ProjectList { projects } => {
            // /tmp should appear in the list (it was used as a session cwd).
            let found = projects.iter().any(|p| p.path == "/tmp");
            assert!(found, "expected /tmp in project list, got: {:?}", projects);
        }
        _ => unreachable!("expected ProjectList"),
    }
}

#[tokio::test]
async fn new_session_with_bad_cwd_returns_error() {
    let script = Arc::new(|| FakeProvider::text_response("ok"));
    let (_dir, socket) = spawn_daemon(make_text_agent_factory(script)).await;

    let mut ws = connect(&socket).await;
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: Some("/nonexistent/path/that/does/not/exist".into()),
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await;

    let msg = recv_one_matching(&mut ws, |m| matches!(m, ServerMessage::Error { .. })).await;
    match msg {
        ServerMessage::Error { message } => {
            assert!(
                message.contains("does not exist"),
                "expected 'does not exist' in error, got: {message}"
            );
        }
        _ => unreachable!("expected Error"),
    }
}
