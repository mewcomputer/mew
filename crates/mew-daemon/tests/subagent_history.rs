use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use mew_agent::Agent;
use mew_daemon::{client::DaemonClient, DaemonServer};
use mew_hooks::NopDispatcher;
use mew_message::{
    Message as RecordedMessage, Part, PartBase, Role, TextPart, Time, ToolCallPart, ToolState,
    ToolStateCompleted, ToolTime,
};
use mew_protocol::{ClientMessage, ServerMessage};
use mew_provider_fake::FakeProvider;
use tokio::io::AsyncWriteExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn recorded_message(child: ulid::Ulid) -> RecordedMessage {
    let id = ulid::Ulid::new();
    let base = || PartBase {
        id: ulid::Ulid::new(),
        message_id: id,
        session_id: child,
    };
    RecordedMessage {
        id,
        session_id: child,
        role: Role::Assistant,
        time: Time {
            created: 1,
            completed: Some(2),
        },
        assistant: None,
        parts: vec![
            Part::Text(TextPart {
                base: base(),
                text: "found the answer 日本語".into(),
                synthetic: false,
            }),
            Part::ToolCall(ToolCallPart {
                base: base(),
                tool_name: "bash".into(),
                call_id: "read-1".into(),
                raw_input: String::new(),
                state: ToolState::Completed(ToolStateCompleted {
                    input: serde_json::json!({"command":"cat notes.txt"}),
                    output: "recorded tool output".into(),
                    metadata: None,
                    diff: None,
                    images: vec![],
                    time: ToolTime {
                        start: 1,
                        end: Some(2),
                    },
                }),
            }),
        ],
    }
}

#[tokio::test]
async fn child_history_reads_recorded_revisions_without_switching_or_waiting_for_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let server = DaemonServer::with_session_dir(
        Arc::new(|params| {
            let session_id = params
                .session_id
                .strip_prefix("sess_")
                .and_then(|id| ulid::Ulid::from_string(id).ok());
            Ok((
                Agent::new(
                    Arc::new(FakeProvider::new(FakeProvider::text_response(
                        "parent reply",
                    ))),
                    Arc::new(NopDispatcher),
                    Some(params.writer),
                    vec![],
                    session_id,
                ),
                None,
                None,
            ))
        }),
        dir.path().to_path_buf(),
    );
    let parent = server.session_manager.create(None).await.unwrap();
    let recorded_parent = parent.agent.lock().await.session_id.to_string();
    let child = ulid::Ulid::new();
    let child_id = child.to_string();
    let mut writer = mew_session::Writer::open_subagent_at(
        dir.path(),
        &recorded_parent,
        &child_id,
        "researcher",
    )
    .await
    .unwrap();
    let original = recorded_message(child);
    writer.write_message(&original).await.unwrap();
    let mut latest = original.clone();
    let Part::Text(text) = &mut latest.parts[0] else {
        unreachable!()
    };
    text.text = "latest recorded answer 日本語".into();
    writer.write_message(&latest).await.unwrap();
    let path = writer.path().clone();
    writer.close().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let task = tokio::spawn(async move { server.run_tcp(address).await });
    let url = format!("ws://{address}");
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
    client.attach_session(&parent.id).await.unwrap();
    loop {
        if matches!(
            notifications.recv().await,
            Some(ServerMessage::TodosUpdated { .. })
        ) {
            break;
        }
    }
    // A second attached client must not receive request-local transcript replies.
    let (mut observer, _) = connect_async(&url).await.unwrap();
    observer
        .send(Message::Text(
            mew_protocol::encode_json(&ClientMessage::AttachSession {
                session_id: parent.id.clone(),
                client_kind: mew_protocol::ClientKind::Unknown,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    observer
        .send(Message::Text(
            mew_protocol::encode_json(&ClientMessage::Ping).unwrap(),
        ))
        .await
        .unwrap();
    loop {
        let Message::Text(text) = observer.next().await.unwrap().unwrap() else {
            continue;
        };
        if matches!(
            mew_protocol::decode_json::<ServerMessage>(&text).unwrap(),
            ServerMessage::Pong { .. }
        ) {
            break;
        }
    }
    let turn = parent.turn_lock.lock().await;
    let mut parent_events = client.prompt("parent work".into(), vec![]).await;
    client
        .read_subagent_history(&parent.id, &child_id, "history-1")
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(message @ ServerMessage::SubagentHistory { .. }) =
                notifications.recv().await
            {
                break message;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(response, ServerMessage::SubagentHistory { session_id, child_session_id, request_id, messages, error } if session_id == parent.id && child_session_id == child_id && request_id == "history-1" && messages == vec![latest.clone()] && error.is_none())
    );
    assert_eq!(
        client.session_id().await.as_deref(),
        Some(parent.id.as_str())
    );
    drop(turn);
    tokio::time::timeout(Duration::from_secs(1), async {
        while parent_events.recv().await.is_some() {}
    })
    .await
    .unwrap();
    observer
        .send(Message::Text(
            mew_protocol::encode_json(&ClientMessage::Ping).unwrap(),
        ))
        .await
        .unwrap();
    loop {
        let Message::Text(text) = observer.next().await.unwrap().unwrap() else {
            continue;
        };
        let message = mew_protocol::decode_json::<ServerMessage>(&text).unwrap();
        assert!(!matches!(message, ServerMessage::SubagentHistory { .. }));
        if matches!(message, ServerMessage::Pong { .. }) {
            break;
        }
    }

    // A live append may end in the middle of a UTF-8 code point or JSON object.
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .await
        .unwrap();
    file.write_all(b"{\"id\":\"").await.unwrap();
    file.flush().await.unwrap();
    client
        .read_subagent_history(&parent.id, &child_id, "partial")
        .await
        .unwrap();
    loop {
        if let Some(ServerMessage::SubagentHistory {
            request_id,
            messages,
            error,
            ..
        }) = notifications.recv().await
        {
            assert_eq!(request_id, "partial");
            assert_eq!(messages, vec![latest.clone()]);
            assert!(error.is_none(), "{error:?}");
            break;
        }
    }
    let mut complete = serde_json::to_vec(&latest).unwrap();
    complete.push(b'\n');
    let unicode = complete
        .windows("日本語".len())
        .position(|bytes| bytes == "日本語".as_bytes())
        .unwrap();
    let mut snapshot = complete.clone();
    snapshot.extend_from_slice(&complete[..unicode + 1]);
    tokio::fs::write(&path, snapshot).await.unwrap();
    client
        .read_subagent_history(&parent.id, &child_id, "partial-unicode")
        .await
        .unwrap();
    loop {
        if let Some(ServerMessage::SubagentHistory {
            messages, error, ..
        }) = notifications.recv().await
        {
            assert_eq!(messages, vec![latest.clone()]);
            assert!(error.is_none(), "{error:?}");
            break;
        }
    }
    let canonical_child = ulid::Ulid::new();
    let canonical_id = canonical_child.to_string();
    let mut writer =
        mew_session::Writer::open_subagent_at(dir.path(), &parent.id, &canonical_id, "legacy")
            .await
            .unwrap();
    let canonical = recorded_message(canonical_child);
    writer.write_message(&canonical).await.unwrap();
    writer.close().await.unwrap();
    client
        .read_subagent_history(&parent.id, &canonical_id, "canonical")
        .await
        .unwrap();
    loop {
        if let Some(ServerMessage::SubagentHistory {
            child_session_id,
            messages,
            error,
            ..
        }) = notifications.recv().await
        {
            assert_eq!(child_session_id, canonical_id);
            assert_eq!(messages, vec![canonical]);
            assert!(error.is_none(), "{error:?}");
            break;
        }
    }
    task.abort();
}

#[tokio::test]
async fn child_history_rejects_unowned_ids_missing_records_and_malformed_complete_lines() {
    let dir = tempfile::tempdir().unwrap();
    let server = DaemonServer::with_session_dir(
        Arc::new(|params| {
            Ok((
                Agent::new(
                    Arc::new(FakeProvider::new(vec![])),
                    Arc::new(NopDispatcher),
                    Some(params.writer),
                    vec![],
                    None,
                ),
                None,
                None,
            ))
        }),
        dir.path().to_path_buf(),
    );
    let parent = server.session_manager.create(None).await.unwrap();
    let recorded_parent = parent.agent.lock().await.session_id.to_string();
    let child_id = ulid::Ulid::new().to_string();
    let writer = mew_session::Writer::open_subagent_at(
        dir.path(),
        &recorded_parent,
        &child_id,
        "researcher",
    )
    .await
    .unwrap();
    let path = writer.path().clone();
    writer.close().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let task = tokio::spawn(async move { server.run_tcp(address).await });
    let url = format!("ws://{address}");
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
    client.attach_session(&parent.id).await.unwrap();
    loop {
        if matches!(
            notifications.recv().await,
            Some(ServerMessage::TodosUpdated { .. })
        ) {
            break;
        }
    }
    let foreign_child = ulid::Ulid::new().to_string();
    let other_parent = ulid::Ulid::new().to_string();
    mew_session::Writer::open_subagent_at(dir.path(), &other_parent, &foreign_child, "foreign")
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
    for (session, child, expected) in [
        ("another-parent", child_id.as_str(), "attached parent"),
        (parent.id.as_str(), "../../outside", "invalid child"),
        (
            parent.id.as_str(),
            "00000000000000000000000000",
            "registered child",
        ),
        (
            parent.id.as_str(),
            foreign_child.as_str(),
            "registered child",
        ),
    ] {
        client
            .read_subagent_history(session, child, "invalid")
            .await
            .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(message @ ServerMessage::SubagentHistory { .. }) =
                    notifications.recv().await
                {
                    break message;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(response, ServerMessage::SubagentHistory { session_id, child_session_id, request_id, messages, error: Some(error) } if session_id == session && child_session_id == child && request_id == "invalid" && messages.is_empty() && error.contains(expected))
        );
    }
    for (data, expected) in [
        (None, "read child transcript"),
        (
            Some(b"not valid JSON\n".as_slice()),
            "invalid child transcript record",
        ),
        (
            Some(b"not valid JSON".as_slice()),
            "invalid child transcript record",
        ),
        (
            Some(b"not valid JSON\xe7".as_slice()),
            "invalid child transcript record",
        ),
    ] {
        match data {
            Some(data) => tokio::fs::write(&path, data).await.unwrap(),
            None => tokio::fs::remove_file(&path).await.unwrap(),
        }
        client
            .read_subagent_history(&parent.id, &child_id, "invalid-source")
            .await
            .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(message @ ServerMessage::SubagentHistory { .. }) =
                    notifications.recv().await
                {
                    break message;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(response, ServerMessage::SubagentHistory { messages, error: Some(error), .. } if messages.is_empty() && error.contains(expected))
        );
    }
    task.abort();
}
