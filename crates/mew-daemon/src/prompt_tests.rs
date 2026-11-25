use super::*;
use crate::request_tests::{connect, flush, receipt, send};

fn server(root: &std::path::Path) -> DaemonServer {
    DaemonServer::with_session_dir(
        Arc::new(|params| {
            let mut agent = Agent::new(
                Arc::new(mew_provider_fake::FakeProvider::new(
                    mew_provider_fake::FakeProvider::text_response("done"),
                )),
                Arc::new(mew_hooks::NopDispatcher),
                Some(params.writer),
                Vec::new(),
                None,
            );
            agent.supports_vision = false;
            Ok((agent, None, None))
        }),
        root.to_path_buf(),
    )
}

fn submit(session_id: &str, request_id: &str) -> ClientMessage {
    ClientMessage::SubmitPrompt {
        session_id: session_id.into(),
        request_id: request_id.into(),
        text: "keep this input".into(),
        attachments: Vec::new(),
    }
}

async fn attach(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
    id: &str,
) {
    send(socket, ClientMessage::SetAutoTitle { enabled: false }).await;
    send(
        socket,
        ClientMessage::AttachSession {
            session_id: id.into(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(socket).await;
}

#[tokio::test]
async fn accepted_prompt_is_persisted_before_reply_and_deduplicated_after_reload() {
    let directory = tempfile::tempdir().unwrap();
    let original = server(directory.path());
    let session = original.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    let mut socket = connect(&original).await;
    attach(&mut socket, &id).await;

    send(&mut socket, submit(&id, "persisted")).await;
    assert_eq!(
        receipt(&mut socket, "persisted").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    let persisted = mew_session::Reader::load_from(directory.path(), &id)
        .await
        .unwrap();
    assert!(matches!(
        persisted.first().map(|message| message.parts.as_slice()),
        Some([mew_message::Part::Text(text)]) if text.text == "keep this input"
    ));
    let finished = session.turn_lock.lock().await;
    drop(finished);
    send(
        &mut socket,
        ClientMessage::NewSession {
            cwd: None,
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    assert!(flush(&mut socket).await.iter().any(|message| {
        matches!(message, ServerMessage::SessionReady { session_id, .. } if session_id != &id)
    }));
    original.session_manager.remove(&id).await;
    drop(session);
    socket.close(None).await.unwrap();

    let restarted = server(directory.path());
    let mut socket = connect(&restarted).await;
    send(
        &mut socket,
        ClientMessage::ReadPromptReceipt {
            session_id: id.clone(),
            request_id: "persisted".into(),
        },
    )
    .await;
    assert_eq!(
        receipt(&mut socket, "persisted").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    attach(&mut socket, &id).await;
    send(&mut socket, submit(&id, "persisted")).await;
    assert_eq!(
        receipt(&mut socket, "persisted").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    let restored = restarted.session_manager.attach(&id).await.unwrap();
    let _finished = restored.turn_lock.lock().await;
    let transcript = mew_session::Reader::load_from(directory.path(), &id)
        .await
        .unwrap();
    assert_eq!(
        transcript
            .iter()
            .filter(|message| message.role == mew_message::Role::User)
            .count(),
        1
    );
}

#[tokio::test]
async fn unsupported_attachment_rejects_submission_before_user_message() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let mut socket = connect(&server).await;
    let session = server.session_manager.create(None).await.unwrap();
    attach(&mut socket, &session.id).await;

    send(
        &mut socket,
        ClientMessage::SubmitPrompt {
            session_id: session.id.clone(),
            request_id: "unsupported-image".into(),
            text: "my draft".into(),
            attachments: vec![mew_protocol::Attachment {
                path: "diagram.png".into(),
                mime: Some("image/png".into()),
            }],
        },
    )
    .await;

    let (status, message, mut events) = receipt(&mut socket, "unsupported-image").await;
    assert_eq!(status, mew_protocol::PromptStatus::Rejected);
    assert!(message.unwrap().contains("image attachments not supported"));
    events.extend(flush(&mut socket).await);
    assert!(!events
        .iter()
        .any(|event| matches!(event, ServerMessage::UserMessage { .. })));
    assert!(
        mew_session::Reader::load_from(directory.path(), &session.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn unwritable_transcript_rejects_submission_and_leaves_turn_idle() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let mut socket = connect(&server).await;
    let session = server.session_manager.create(None).await.unwrap();
    attach(&mut socket, &session.id).await;
    let path = directory.path().join(&session.id).join("session.jsonl");
    let saved = path.with_extension("saved");
    tokio::fs::rename(&path, &saved).await.unwrap();
    tokio::fs::create_dir(&path).await.unwrap();

    send(&mut socket, submit(&session.id, "write-failed")).await;

    let (status, message, mut events) = receipt(&mut socket, "write-failed").await;
    assert_eq!(status, mew_protocol::PromptStatus::Rejected);
    assert!(message.unwrap().contains("could not save prompt"));
    events.extend(flush(&mut socket).await);
    assert!(!events
        .iter()
        .any(|event| matches!(event, ServerMessage::UserMessage { .. })));
    assert!(!*session.is_running.lock().await);
    assert!(session.current_turn_cancel.lock().await.is_none());
    assert!(tokio::fs::read(&saved).await.unwrap().is_empty());
}

#[tokio::test]
async fn failed_clear_reports_the_error_without_broadcasting_a_reset() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(directory.path());
    let session = server.session_manager.create(None).await.unwrap();
    let mut socket = connect(&server).await;
    attach(&mut socket, &session.id).await;
    send(&mut socket, submit(&session.id, "before-clear")).await;
    assert_eq!(
        receipt(&mut socket, "before-clear").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    let finished = session.turn_lock.lock().await;
    let before = session.agent.lock().await.messages.lock().await.clone();
    drop(finished);
    assert_eq!(before.len(), 2);
    let path = directory.path().join(&session.id).join("session.jsonl");
    let saved = path.with_extension("saved");
    tokio::fs::rename(&path, &saved).await.unwrap();
    tokio::fs::create_dir(&path).await.unwrap();

    send(
        &mut socket,
        ClientMessage::SlashCommand {
            command: "/clear".into(),
        },
    )
    .await;
    let (text, cleared) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut cleared = false;
        loop {
            let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
            match mew_protocol::decode_json::<ServerMessage>(&frame).unwrap() {
                ServerMessage::SlashResult { text } => break (text, cleared),
                ServerMessage::SessionCleared => cleared = true,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(text.contains("Could not clear context"), "{text}");
    assert!(!cleared);
    assert_eq!(*session.agent.lock().await.messages.lock().await, before);
}

#[tokio::test]
async fn queued_messages_sync_across_clients_promote_to_steer_and_remove() {
    let directory = tempfile::Builder::new()
        .prefix("mew-queue-")
        .tempdir_in("/tmp")
        .unwrap();
    let server = server(directory.path());
    let session = server.session_manager.create(None).await.unwrap();
    let id = session.id.clone();

    *session.is_running.lock().await = true;

    let mut socket_a = connect(&server).await;
    let mut socket_b = connect(&server).await;
    attach(&mut socket_a, &id).await;
    attach(&mut socket_b, &id).await;

    send(
        &mut socket_a,
        ClientMessage::EnqueueMessage {
            session_id: id.clone(),
            request_id: "req-1".into(),
            text: "first follow-up".into(),
            attachments: Vec::new(),
            mode: mew_protocol::QueueMode::FollowUp,
        },
    )
    .await;
    send(
        &mut socket_a,
        ClientMessage::EnqueueMessage {
            session_id: id.clone(),
            request_id: "req-2".into(),
            text: "second follow-up".into(),
            attachments: Vec::new(),
            mode: mew_protocol::QueueMode::FollowUp,
        },
    )
    .await;

    let _ = flush(&mut socket_a).await;
    let b_updates = flush(&mut socket_b).await;
    let last_items = b_updates
        .iter()
        .rev()
        .find_map(|msg| match msg {
            ServerMessage::SessionQueueChanged { session_id, items } if session_id == &id => {
                Some(items.clone())
            }
            _ => None,
        })
        .expect("second client should receive synced queue update");
    assert_eq!(last_items.len(), 2);
    assert_eq!(last_items[0].text, "first follow-up");
    assert_eq!(last_items[1].text, "second follow-up");

    // A newly attached client receives the queued stack immediately on attach.
    let mut socket_c = connect(&server).await;
    send(
        &mut socket_c,
        ClientMessage::AttachSession {
            session_id: id.clone(),
            client_kind: ClientKind::Web,
        },
    )
    .await;
    let c_replay = flush(&mut socket_c).await;
    assert!(c_replay.iter().any(|msg| matches!(
        msg,
        ServerMessage::SessionQueueChanged { session_id, items }
            if session_id == &id && items.len() == 2
    )));

    // Remove second item.
    send(
        &mut socket_a,
        ClientMessage::RemoveQueuedMessage {
            session_id: id.clone(),
            id: last_items[1].id.clone(),
        },
    )
    .await;
    let after_remove = flush(&mut socket_a).await;
    assert!(after_remove.iter().any(|msg| matches!(
        msg,
        ServerMessage::SessionQueueChanged { session_id, items }
            if session_id == &id && items.len() == 1 && items[0].text == "first follow-up"
    )));

    // Promote remaining item to steer while busy.
    send(
        &mut socket_a,
        ClientMessage::UpdateQueuedMessage {
            session_id: id.clone(),
            id: last_items[0].id.clone(),
            text: None,
            mode: Some(mew_protocol::QueueMode::Steer),
        },
    )
    .await;
    let _ = flush(&mut socket_a).await;
    let b_steer = flush(&mut socket_b).await;
    assert!(b_steer.iter().any(|msg| matches!(
        msg,
        ServerMessage::SessionQueueChanged { session_id, items }
            if session_id == &id && items.is_empty()
    )));
    assert!(b_steer.iter().any(|msg| matches!(
        msg,
        ServerMessage::UserMessage { text, request_id: None, steered: true } if text == "first follow-up"
    )));
}

#[tokio::test]
async fn finish_prompt_turn_drains_queued_follow_ups_sequentially_unless_cancelled() {
    let directory = tempfile::Builder::new()
        .prefix("mew-drain-")
        .tempdir_in("/tmp")
        .unwrap();
    let server = server(directory.path());
    let session = server.session_manager.create(None).await.unwrap();
    let mut socket = connect(&server).await;
    attach(&mut socket, &session.id).await;

    session
        .queued_messages
        .lock()
        .await
        .push_back(mew_protocol::QueuedMessageWire {
            id: "q-next".into(),
            text: "queued follow-up".into(),
            attachments: Vec::new(),
            mode: mew_protocol::QueueMode::FollowUp,
            created_at: 1,
        });

    let guard = session.turn_lock.lock().await;
    let token = register_turn(&session).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        finish_prompt_turn(
            &session,
            &server.session_manager,
            "initial prompt".into(),
            PromptTurn::New {
                text: "initial prompt".into(),
                attachments: Vec::new(),
            },
            false,
            guard,
            token,
        ),
    )
    .await
    .expect("finish_prompt_turn turn1+drain timed out");

    let drained = flush(&mut socket).await;
    assert!(drained.iter().any(|msg| matches!(
        msg,
        ServerMessage::SessionQueueChanged { session_id, items }
            if session_id == &session.id && items.is_empty()
    )));
    assert!(drained.iter().any(|msg| matches!(
        msg,
        ServerMessage::UserMessage { text, request_id: None, steered: false } if text == "queued follow-up"
    )));
    let messages = session.agent.lock().await.messages.lock().await.clone();
    assert_eq!(messages.len(), 4);

    // Cancelled turns stop auto-drain and preserve remaining queued items.
    session
        .queued_messages
        .lock()
        .await
        .push_back(mew_protocol::QueuedMessageWire {
            id: "q-keep".into(),
            text: "stay queued on cancel".into(),
            attachments: Vec::new(),
            mode: mew_protocol::QueueMode::FollowUp,
            created_at: 2,
        });
    let guard = session.turn_lock.lock().await;
    let token = register_turn(&session).await;
    token.cancel();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        finish_prompt_turn(
            &session,
            &server.session_manager,
            "cancelled turn".into(),
            PromptTurn::New {
                text: "cancelled turn".into(),
                attachments: Vec::new(),
            },
            false,
            guard,
            token,
        ),
    )
    .await
    .expect("finish_prompt_turn cancelled timed out");
    assert_eq!(session.queued_messages.lock().await.len(), 1);
}
