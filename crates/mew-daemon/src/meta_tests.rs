use super::*;
use crate::request_tests::{connect, flush, send};

#[test]
fn recovered_metadata_keeps_usage_recorded_during_an_outage() {
    let mut meta = mew_session::Meta::new("test");
    meta.usage = Some(mew_session::SessionUsage {
        input_tokens: 12,
        output_tokens: 3,
        turns: 1,
        ..Default::default()
    });
    meta.context_tokens = Some(12);
    let cached = mew_protocol::SessionUsageWire {
        input_tokens: 42,
        output_tokens: 8,
        turns: 1,
        ..Default::default()
    };
    reconcile_usage(&mut meta, Some(&cached), Some(30));
    let usage = meta.usage.unwrap();
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.turns),
        (42, 8, 1)
    );
    assert_eq!(meta.context_tokens, Some(30));
}

#[test]
fn recovered_metadata_does_not_restore_a_cleared_context_meter() {
    let mut meta = mew_session::Meta::new("test");
    meta.context_tokens = Some(42);
    reconcile_usage(&mut meta, None, None);
    assert_eq!(meta.context_tokens, None);
}

#[tokio::test]
async fn metadata_edits_and_cumulative_usage_survive_follow_up_turns() {
    let directory = tempfile::tempdir().unwrap();
    let mut script = mew_provider_fake::FakeProvider::text_response("finished");
    let Some(mew_provider::ProviderEvent::MessageEnd { usage, .. }) = script.last_mut() else {
        panic!("expected a usage report");
    };
    usage.input = 12;
    usage.output = 3;
    let provider = Arc::new(mew_provider_fake::FakeProvider::new(script));
    let server = DaemonServer::with_session_dir(
        Arc::new(move |params| {
            let agent = Agent::new(
                provider.clone(),
                Arc::new(mew_hooks::NopDispatcher),
                Some(params.writer),
                Vec::new(),
                None,
            );
            Ok((agent, None, None))
        }),
        directory.path().to_path_buf(),
    );
    let mut socket = connect(&server).await;
    let session = server.session_manager.create(None).await.unwrap();
    send(&mut socket, ClientMessage::SetAutoTitle { enabled: false }).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;

    for text in ["first turn", "second turn"] {
        send(
            &mut socket,
            ClientMessage::RenameSession {
                session_id: session.id.clone(),
                title: "Title kept across turns".into(),
            },
        )
        .await;
        send(
            &mut socket,
            ClientMessage::PinSession {
                session_id: session.id.clone(),
                pinned: true,
            },
        )
        .await;
        send(
            &mut socket,
            ClientMessage::Prompt {
                text: text.into(),
                attachments: Vec::new(),
            },
        )
        .await;
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                let reply =
                    mew_protocol::decode_json::<ServerMessage>(frame.to_text().unwrap()).unwrap();
                if matches!(
                    reply,
                    ServerMessage::SessionAlert {
                        kind: mew_protocol::AlertKind::TurnComplete,
                        ..
                    }
                ) {
                    break;
                }
                assert!(
                    !matches!(
                        reply,
                        ServerMessage::Error { .. } | ServerMessage::ErrorEvent { .. }
                    ),
                    "{reply:?}"
                );
            }
        })
        .await
        .expect("turn never completed");
    }

    // Clients rebuild their title slot from this snapshot after a restart, so
    // the persisted title has to travel with the list.
    send(&mut socket, ClientMessage::ListSessions).await;
    let listed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            let reply =
                mew_protocol::decode_json::<ServerMessage>(frame.to_text().unwrap()).unwrap();
            if let ServerMessage::SessionList { sessions } = reply {
                break sessions;
            }
        }
    })
    .await
    .expect("session list never arrived");
    let listed = listed
        .into_iter()
        .find(|info| info.session_id == session.id)
        .expect("session missing from the list");
    assert_eq!(listed.title.as_deref(), Some("Title kept across turns"));

    let meta = mew_session::Meta::read(directory.path(), &session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        meta.custom_title.as_deref(),
        Some("Title kept across turns")
    );
    assert!(meta.pinned);
    let usage = meta.usage.unwrap();
    assert_eq!(usage.turns, 2);
    assert_eq!(usage.input_tokens, 24);
    assert_eq!(usage.output_tokens, 6);
    assert_eq!(meta.context_tokens, Some(12));
}

#[tokio::test]
async fn independent_daemons_cannot_load_the_same_live_writer() {
    use crate::request_tests::test_server;

    let directory = tempfile::tempdir().unwrap();
    let first = test_server(directory.path().to_path_buf());
    let second = test_server(directory.path().to_path_buf());
    let session = first.session_manager.create(None).await.unwrap();
    let id = session.id.clone();

    let failed = second.session_manager.attach(&id).await;
    assert!(
        matches!(failed, Err(error) if error.to_string().contains("another daemon")),
        "a live writer must be exclusive across independent daemons"
    );
    let other = second.session_manager.create(None).await.unwrap();
    assert_ne!(other.id, id);

    first.session_manager.remove(&id).await;
    let failed = second.session_manager.attach(&id).await;
    assert!(matches!(failed, Err(error) if error.to_string().contains("another daemon")));
    drop(session);

    let restored = second.session_manager.attach(&id).await.unwrap();
    assert_eq!(restored.id, id);
}

#[tokio::test]
async fn deletion_cannot_remove_a_conversation_with_another_daemon_writer() {
    use crate::request_tests::test_server;

    let directory = tempfile::tempdir().unwrap();
    let first = test_server(directory.path().to_path_buf());
    let second = test_server(directory.path().to_path_buf());
    let session = first.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    let path = directory.path().join(&id);

    let failed = second.session_manager.delete(&id, None, None).await;
    assert!(matches!(failed, Err(error) if error.to_string().contains("another daemon")));
    assert!(path.is_dir());

    first.session_manager.remove(&id).await;
    drop(session);
    second
        .session_manager
        .delete(&id, None, None)
        .await
        .unwrap();
    assert!(!path.exists());
    assert!(matches!(
        second.session_manager.attach(&id).await,
        Err(AttachError::NotFound)
    ));
}
