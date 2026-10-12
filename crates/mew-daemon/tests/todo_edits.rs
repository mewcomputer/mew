use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use mew_agent::{Agent, Todo, TodoList, TodoStatus};
use mew_daemon::{client::DaemonClient, DaemonServer};
use mew_hooks::NopDispatcher;
use mew_protocol::{ClientMessage, ServerMessage, TodoEdit};
use mew_provider_fake::FakeProvider;
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn edit(todo: &Todo, content: &str, status: &str) -> TodoEdit {
    TodoEdit {
        todo_id: todo.id,
        expected_content: todo.content.clone(),
        expected_status: todo.status.as_str().into(),
        content: content.into(),
        status: status.into(),
    }
}

async fn wire_peer() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    (listener, url)
}

async fn accept_peer(
    listener: tokio::net::TcpListener,
) -> tokio_tungstenite::WebSocketStream<tokio::net::TcpStream> {
    let (socket, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
    let Message::Text(text) = ws.next().await.unwrap().unwrap() else {
        panic!("expected ping")
    };
    assert!(matches!(
        mew_protocol::decode_json::<ClientMessage>(&text).unwrap(),
        ClientMessage::Ping
    ));
    ws.send(Message::Text(
        mew_protocol::encode_json(&ServerMessage::Pong {
            version: "test".into(),
            protocol_version: mew_protocol::PROTOCOL_VERSION,
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    ws
}

#[tokio::test]
async fn todo_client_accepts_only_the_matching_acknowledgement() {
    let (listener, url) = wire_peer().await;
    let peer = tokio::spawn(async move {
        let mut ws = accept_peer(listener).await;
        let Message::Text(text) = ws.next().await.unwrap().unwrap() else {
            panic!("expected edit")
        };
        let ClientMessage::UpdateTodo {
            session_id,
            request_id,
            ..
        } = mew_protocol::decode_json(&text).unwrap()
        else {
            panic!("expected update_todo")
        };
        for (session, request) in [
            ("other-session", request_id.as_str()),
            (session_id.as_str(), "other-request"),
        ] {
            ws.send(Message::Text(
                mew_protocol::encode_json(&ServerMessage::TodoUpdateError {
                    session_id: session.into(),
                    request_id: request.into(),
                    message: "wrong acknowledgement".into(),
                })
                .unwrap(),
            ))
            .await
            .unwrap();
        }
        ws.send(Message::Text(
            mew_protocol::encode_json(&ServerMessage::TodoUpdated {
                session_id,
                request_id,
                todos: vec![mew_protocol::Todo {
                    id: 7,
                    content: "authoritative".into(),
                    status: "blocked".into(),
                    depends_on: vec![2],
                }],
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        ws.close(None).await.unwrap();
    });
    let (client, _notifications) = DaemonClient::connect(&url).await.unwrap();
    let updated = client
        .update_todo("session", TodoEdit::default())
        .await
        .unwrap();
    assert_eq!(
        updated,
        vec![Todo {
            id: 7,
            content: "authoritative".into(),
            status: TodoStatus::Blocked,
            depends_on: vec![2]
        }]
    );
    peer.await.unwrap();
}

#[tokio::test]
async fn todo_client_reports_disconnect_before_confirmation() {
    let (listener, url) = wire_peer().await;
    let peer = tokio::spawn(async move {
        let mut ws = accept_peer(listener).await;
        assert!(matches!(
            ws.next().await.unwrap().unwrap(),
            Message::Text(_)
        ));
        ws.close(None).await.unwrap();
    });
    let (client, _notifications) = DaemonClient::connect(&url).await.unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        client.update_todo("session", TodoEdit::default()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("disconnected"), "{error}");
    peer.await.unwrap();
}

#[tokio::test]
async fn todo_client_bounds_unsupported_legacy_requests_without_consuming_unrelated_errors() {
    let (listener, url) = wire_peer().await;
    let peer = tokio::spawn(async move {
        let mut ws = accept_peer(listener).await;
        assert!(matches!(
            ws.next().await.unwrap().unwrap(),
            Message::Text(_)
        ));
        ws.send(Message::Text(
            mew_protocol::encode_json(&ServerMessage::Error {
                message: "unknown variant update_todo".into(),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
        while ws.next().await.is_some() {}
    });
    let (client, mut notifications) = DaemonClient::connect(&url).await.unwrap();
    let pending = client.update_todo("session", TodoEdit::default());
    let (result, notification) = tokio::join!(pending, notifications.recv());
    assert!(
        matches!(notification, Some(ServerMessage::Error { message }) if message.contains("unknown variant"))
    );
    let error = result.unwrap_err();
    assert!(error.to_string().contains("not confirmed"), "{error}");
    assert!(error.to_string().contains("update or reconnect"), "{error}");
    peer.abort();
}

#[tokio::test]
async fn todo_edits_are_acknowledged_persisted_and_broadcast_while_a_turn_is_running() {
    let dir = tempfile::tempdir().unwrap();
    let server = DaemonServer::with_session_dir(
        Arc::new(|params| {
            Ok((
                Agent::new(
                    Arc::new(FakeProvider::new(FakeProvider::text_response("hi"))),
                    Arc::new(NopDispatcher),
                    Some(params.writer),
                    vec![],
                    None,
                ),
                None,
                None,
            ))
        }),
        dir.path().join("sessions"),
    );
    let session = server.session_manager.create(None).await.unwrap();
    let path = dir.path().join("todos.json");
    let originals = {
        let mut agent = session.agent.lock().await;
        agent.todos_path = Some(path.clone());
        let mut list = agent.todos.lock().await;
        list.create(vec![
            ("first".into(), vec![]),
            ("dependent".into(), vec![1]),
        ]);
        list.save(&path).await.unwrap();
        list.items.clone()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let task = tokio::spawn(async move { server.run_tcp(addr).await });
    let url = format!("ws://{addr}");
    let (client, mut notifications) = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match DaemonClient::connect(&url).await {
                Ok(client) => break client,
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .unwrap();
    client.attach_session(&session.id).await.unwrap();
    while !matches!(
        notifications.recv().await,
        Some(ServerMessage::SessionReady { .. })
    ) {}
    let (mut observer, _) = connect_async(&url).await.unwrap();
    observer
        .send(Message::Text(
            mew_protocol::encode_json(&ClientMessage::AttachSession {
                session_id: session.id.clone(),
                client_kind: mew_protocol::ClientKind::Unknown,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    let mut confirmed = false;
    loop {
        let Message::Text(text) = observer.next().await.unwrap().unwrap() else {
            continue;
        };
        match mew_protocol::decode_json::<ServerMessage>(&text).unwrap() {
            ServerMessage::SessionReady { .. } => confirmed = true,
            ServerMessage::TodosUpdated { todos } => {
                assert!(
                    confirmed,
                    "the initial todo list follows session confirmation"
                );
                assert_eq!(todos[0].content, originals[0].content);
                assert_eq!(todos[1].depends_on, originals[1].depends_on);
                break;
            }
            _ => {}
        }
    }
    // Editing must not wait for the agent's full turn lock.
    let turn = session.turn_lock.lock().await;
    *session.is_running.lock().await = true;
    let updated = tokio::time::timeout(
        Duration::from_secs(1),
        client.update_todo(&session.id, edit(&originals[0], "updated", "in_progress")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(updated[0].content, "updated");
    assert_eq!(updated[0].status, TodoStatus::InProgress);
    assert_eq!(updated[1], originals[1]);
    assert_eq!(TodoList::load(&path).await.unwrap().items, updated);
    let broadcast = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let Message::Text(text) = observer.next().await.unwrap().unwrap() else {
                continue;
            };
            let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
            if let ServerMessage::TodosUpdated { todos } = message {
                break todos;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(broadcast[0].content, "updated");
    let stale = client
        .update_todo(
            &session.id,
            edit(&originals[0], "stale overwrite", "pending"),
        )
        .await
        .unwrap_err();
    assert!(stale.to_string().contains("changed"), "{stale}");
    let invalid = client
        .update_todo(&session.id, edit(&originals[1], "dependent", "done"))
        .await
        .unwrap_err();
    assert!(invalid.to_string().contains("depends"), "{invalid}");
    assert!(client
        .update_todo(
            "another-session",
            edit(&updated[0], "wrong task", "pending")
        )
        .await
        .is_err());
    assert!(client
        .update_todo(&session.id, edit(&updated[0], " ", "pending"))
        .await
        .is_err());
    assert!(client
        .update_todo(&session.id, edit(&updated[0], "bad status", "invalid"))
        .await
        .is_err());
    let mut missing = edit(&updated[0], "missing", "pending");
    missing.todo_id = usize::MAX;
    let error = client.update_todo(&session.id, missing).await.unwrap_err();
    assert!(error.to_string().contains("no longer exists"), "{error}");
    let mut stale_status = edit(&updated[0], &updated[0].content, "blocked");
    stale_status.expected_status = "pending".into();
    let error = client
        .update_todo(&session.id, stale_status)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("changed"), "{error}");
    let mut bad_original = edit(&updated[0], "invalid original", "blocked");
    bad_original.expected_status = "invalid".into();
    let error = client
        .update_todo(&session.id, bad_original)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("invalid original"), "{error}");
    assert_eq!(session.agent.lock().await.todos.lock().await.items, updated);
    assert_eq!(TodoList::load(&path).await.unwrap().items, updated);
    drop(turn);
    let completed = client
        .update_todo(&session.id, edit(&updated[0], "completed first", "done"))
        .await
        .unwrap();
    let completed = client
        .update_todo(
            &session.id,
            edit(&completed[1], "completed dependent", "done"),
        )
        .await
        .unwrap();
    assert_eq!(completed[0].status, TodoStatus::Done);
    assert_eq!(completed[1].status, TodoStatus::Done);
    assert_eq!(completed[1].depends_on, vec![originals[0].id]);
    assert_eq!(TodoList::load(&path).await.unwrap().items, completed);
    session.agent.lock().await.todos_path = None;
    let error = client
        .update_todo(
            &session.id,
            edit(&completed[0], "no persistence", "blocked"),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cannot persist"), "{error}");
    session.agent.lock().await.todos_path = Some(dir.path().to_path_buf());
    let failed = client
        .update_todo(&session.id, edit(&completed[0], "unsaved", "blocked"))
        .await
        .unwrap_err();
    assert!(failed.to_string().contains("persist"), "{failed}");
    assert_eq!(
        session.agent.lock().await.todos.lock().await.items,
        completed
    );
    assert_eq!(TodoList::load(&path).await.unwrap().items, completed);
    task.abort();
}
