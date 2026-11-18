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
