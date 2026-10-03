//! Session lifetime through real daemon WebSocket connections. Only the provider
//! boundary is deterministic; persistence, requests, cancellation, and reaping
//! use the production implementation.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use mew_agent::Agent;
use mew_daemon::{DaemonServer, Session};
use mew_hooks::NopDispatcher;
use mew_protocol::{ClientKind, ClientMessage, ServerMessage, SessionState};
use mew_provider_fake::FakeProvider;
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type Ws = WebSocketStream<DuplexStream>;

fn daemon(ask_user: bool) -> (tempfile::TempDir, DaemonServer) {
    let directory = tempfile::tempdir().unwrap();
    let builder: mew_daemon::AgentBuilder = Arc::new(move |params| {
        let script = if ask_user {
            FakeProvider::tool_call(
                "ask_user_question",
                "question-call",
                serde_json::json!({"questions": [{"prompt": "which route?", "options": [
                    {"label": "a", "description": "first route"},
                    {"label": "b", "description": "second route"}
                ]}]}),
            )
        } else {
            FakeProvider::text_response("finished")
        };
        let session_id = params
            .session_id
            .strip_prefix("sess_")
            .and_then(|id| ulid::Ulid::from_string(id).ok());
        let agent = Agent::new(
            Arc::new(FakeProvider::new(script)),
            Arc::new(NopDispatcher),
            Some(params.writer),
            vec![Arc::new(mew_tools::tools::ask_user::AskUser)],
            session_id,
        );
        Ok((agent, Some("fake".into()), Some("fake".into())))
    });
    let server = DaemonServer::with_session_dir(builder, directory.path().join("sessions"));
    (directory, server)
}

async fn connect(server: &DaemonServer) -> (Ws, JoinHandle<()>) {
    let (client, stream) = tokio::io::duplex(64 * 1024);
    let manager = server.session_manager.clone();
    let groups = server.groups_store.clone();
    let task = tokio::spawn(async move {
        mew_daemon::handle_connection(
            stream,
            manager,
            groups,
            None,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await
        .unwrap();
    });
    let (mut socket, _) = tokio_tungstenite::client_async("ws://localhost/", client)
        .await
        .unwrap();
    send(&mut socket, ClientMessage::SetAutoTitle { enabled: false }).await;
    (socket, task)
}

async fn send(socket: &mut Ws, command: ClientMessage) {
    socket
        .send(Message::Text(mew_protocol::encode_json(&command).unwrap()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Ws, matches: impl Fn(&ServerMessage) -> bool) -> ServerMessage {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            let message: ServerMessage =
                mew_protocol::decode_json(&frame.into_text().unwrap()).unwrap();
            if let ServerMessage::Error { message } = &message {
                assert_eq!(message, "aborted", "unexpected daemon error: {message}");
            }
            if matches(&message) {
                return message;
            }
        }
    })
    .await
    .expect("daemon response timed out")
}

async fn new_session(socket: &mut Ws, cwd: Option<String>) -> String {
    send(
        socket,
        ClientMessage::NewSession {
            cwd,
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    match receive(socket, |message| {
        matches!(message, ServerMessage::SessionReady { .. })
    })
    .await
    {
        ServerMessage::SessionReady { session_id, .. } => session_id,
        _ => unreachable!(),
    }
}

async fn barrier(socket: &mut Ws) {
    send(socket, ClientMessage::Ping).await;
    receive(socket, |message| {
        matches!(message, ServerMessage::Pong { .. })
    })
    .await;
}

async fn prompt(socket: &mut Ws) {
    send(
        socket,
        ClientMessage::Prompt {
            text: "do the work".into(),
            attachments: vec![],
        },
    )
    .await;
}

async fn expire_grace_period() {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
}

async fn close(mut socket: Ws, task: JoinHandle<()>) {
    socket.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

async fn weak_session(server: &DaemonServer, session_id: &str) -> std::sync::Weak<Session> {
    Arc::downgrade(&server.session_manager.attach(session_id).await.unwrap())
}

#[tokio::test]
async fn switching_conversations_preserves_pending_work_past_the_grace_period() {
    let (_directory, server) = daemon(true);
    let (mut socket, task) = connect(&server).await;
    let first = new_session(&mut socket, None).await;
    prompt(&mut socket).await;
    receive(&mut socket, |message| {
        matches!(message, ServerMessage::AskUserRequest { .. })
    })
    .await;
    let original = weak_session(&server, &first).await;

    new_session(&mut socket, None).await;
    barrier(&mut socket).await;
    expire_grace_period().await;

    let session = original
        .upgrade()
        .expect("running session must remain loaded");
    assert!(*session.is_running.lock().await);
    assert_eq!(session.pending_questions_count().await, 1);
    assert_eq!(session.client_count().await, 0);
    assert!(Arc::ptr_eq(
        &session,
        &server.session_manager.attach(&first).await.unwrap()
    ));
    drop(session);
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: first,
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    receive(&mut socket, |message| {
        matches!(message, ServerMessage::SessionReady { .. })
    })
    .await;
    send(&mut socket, ClientMessage::Cancel).await;
    receive(&mut socket, |message| {
        matches!(
            message,
            ServerMessage::SessionActivityChanged {
                activity: SessionState::Idle,
                ..
            }
        )
    })
    .await;
    close(socket, task).await;
}

#[tokio::test]
async fn closing_the_last_socket_preserves_work_and_explicit_cancel_still_ends_it() {
    let (_directory, server) = daemon(true);
    let (mut socket, task) = connect(&server).await;
    let session_id = new_session(&mut socket, None).await;
    prompt(&mut socket).await;
    receive(&mut socket, |message| {
        matches!(message, ServerMessage::AskUserRequest { .. })
    })
    .await;
    let original = weak_session(&server, &session_id).await;

    close(socket, task).await;
    expire_grace_period().await;

    let session = original
        .upgrade()
        .expect("running session must survive disconnect");
    assert!(*session.is_running.lock().await);
    assert_eq!(session.pending_questions_count().await, 1);
    drop(session);
    let (mut socket, task) = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id,
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    receive(&mut socket, |message| {
        matches!(message, ServerMessage::SessionReady { .. })
    })
    .await;
    send(&mut socket, ClientMessage::Cancel).await;
    receive(&mut socket, |message| {
        matches!(
            message,
            ServerMessage::SessionActivityChanged {
                activity: SessionState::Idle,
                ..
            }
        )
    })
    .await;
    let session = original.upgrade().unwrap();
    assert!(!*session.is_running.lock().await);
    assert_eq!(session.pending_questions_count().await, 0);
    drop(session);
    receive(&mut socket, |message| {
        matches!(
            message,
            ServerMessage::SessionAlert {
                kind: mew_protocol::AlertKind::TurnFailed,
                ..
            }
        )
    })
    .await;
    close(socket, task).await;
    expire_grace_period().await;
    assert!(
        original.upgrade().is_none(),
        "the retained task must unload once cancellation finishes and its viewers leave"
    );
}

#[tokio::test]
async fn idle_session_unloads_after_watch_cancellation_and_restores_saved_history() {
    let (directory, server) = daemon(false);
    let (mut socket, task) = connect(&server).await;
    let session_id = new_session(&mut socket, Some(directory.path().display().to_string())).await;
    send(
        &mut socket,
        ClientMessage::WatchWorkspace {
            session_id: session_id.clone(),
            enabled: true,
        },
    )
    .await;
    prompt(&mut socket).await;
    receive(&mut socket, |message| {
        matches!(
            message,
            ServerMessage::SessionAlert {
                kind: mew_protocol::AlertKind::TurnComplete,
                ..
            }
        )
    })
    .await;
    let original = weak_session(&server, &session_id).await;

    close(socket, task).await;
    expire_grace_period().await;

    assert!(
        original.upgrade().is_none(),
        "idle runtime and cancelled watcher must be released"
    );
    let (mut socket, task) = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id,
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    receive(&mut socket, |message| {
        matches!(message, ServerMessage::SessionReady { .. })
    })
    .await;
    let page = receive(&mut socket, |message| {
        matches!(message, ServerMessage::SessionHistoryPage { .. })
    })
    .await;
    let ServerMessage::SessionHistoryPage { messages, .. } = page else {
        unreachable!()
    };
    assert!(messages
        .iter()
        .flat_map(|message| &message.parts)
        .any(|part| { matches!(part, mew_message::Part::Text(text) if text.text == "finished") }));
    close(socket, task).await;
}

#[tokio::test]
async fn session_lookup_protects_attachment_before_client_registration() {
    let (_directory, server) = daemon(false);
    let (mut socket, task) = connect(&server).await;
    let session_id = new_session(&mut socket, None).await;
    prompt(&mut socket).await;
    receive(&mut socket, |message| {
        matches!(
            message,
            ServerMessage::SessionAlert {
                kind: mew_protocol::AlertKind::TurnComplete,
                ..
            }
        )
    })
    .await;
    new_session(&mut socket, None).await;
    barrier(&mut socket).await;

    let attaching = server.session_manager.attach(&session_id).await.unwrap();
    expire_grace_period().await;
    let current = server.session_manager.attach(&session_id).await.unwrap();
    assert!(
        Arc::ptr_eq(&attaching, &current),
        "client lookup must not return an orphaned runtime"
    );
    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let (client_id, _) = attaching.attach_client(sender, ClientKind::Desktop).await;
    assert_eq!(attaching.client_count().await, 1);
    attaching.detach_client(client_id).await;
    drop(attaching);
    drop(current);
    close(socket, task).await;
}

#[tokio::test]
async fn stale_reaper_cannot_remove_a_reloaded_session_with_the_same_id() {
    let (_directory, server) = daemon(false);
    let (mut socket, task) = connect(&server).await;
    let session_id = new_session(&mut socket, None).await;
    prompt(&mut socket).await;
    receive(&mut socket, |message| {
        matches!(
            message,
            ServerMessage::SessionAlert {
                kind: mew_protocol::AlertKind::TurnComplete,
                ..
            }
        )
    })
    .await;
    new_session(&mut socket, None).await;
    barrier(&mut socket).await;
    server.session_manager.remove(&session_id).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: session_id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    receive(&mut socket, |message| {
        matches!(message, ServerMessage::SessionReady { .. })
    })
    .await;
    let reloaded = weak_session(&server, &session_id).await;

    expire_grace_period().await;

    let existing = reloaded.upgrade().unwrap();
    assert_eq!(existing.client_count().await, 1);
    assert!(
        Arc::ptr_eq(
            &existing,
            &server.session_manager.attach(&session_id).await.unwrap()
        ),
        "old reaper must not remove the new runtime"
    );
    drop(existing);
    close(socket, task).await;
}

#[tokio::test]
async fn session_ready_is_followed_by_authoritative_activity_for_new_running_and_cancelled_tasks() {
    let (_directory, server) = daemon(true);
    let (mut first, first_task) = connect(&server).await;
    let session_id = new_session(&mut first, None).await;
    assert!(
        matches!(
            receive(&mut first, |_| true).await,
            ServerMessage::SessionActivityChanged { session_id: id, activity: SessionState::Idle } if id == session_id
        ),
        "new session must report its idle state immediately after SessionReady"
    );
    prompt(&mut first).await;
    receive(&mut first, |message| {
        matches!(message, ServerMessage::AskUserRequest { .. })
    })
    .await;

    let (mut second, second_task) = connect(&server).await;
    send(
        &mut second,
        ClientMessage::AttachSession {
            session_id: session_id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    receive(&mut second, |message| {
        matches!(message, ServerMessage::SessionReady { .. })
    })
    .await;
    assert!(
        matches!(
            receive(&mut second, |_| true).await,
            ServerMessage::SessionActivityChanged { session_id: id, activity: SessionState::Running } if id == session_id
        ),
        "attach must remain running while a tool waits for user input"
    );
    send(&mut second, ClientMessage::Cancel).await;
    receive(&mut second, |message| {
        matches!(
            message,
            ServerMessage::SessionActivityChanged {
                activity: SessionState::Idle,
                ..
            }
        )
    })
    .await;
    close(first, first_task).await;
    new_session(&mut second, None).await;
    barrier(&mut second).await;

    send(
        &mut second,
        ClientMessage::AttachSession {
            session_id: session_id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    receive(&mut second, |message| {
        matches!(message, ServerMessage::SessionReady { .. })
    })
    .await;
    assert!(
        matches!(
            receive(&mut second, |_| true).await,
            ServerMessage::SessionActivityChanged { session_id: id, activity: SessionState::Idle } if id == session_id
        ),
        "cancelled task must report idle on reattach"
    );
    close(second, second_task).await;
}
