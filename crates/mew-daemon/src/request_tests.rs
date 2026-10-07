use super::*;
use tokio::sync::oneshot;
use tokio_tungstenite::{client_async, WebSocketStream};

type TestSocket = WebSocketStream<tokio::io::DuplexStream>;

fn test_server(path: PathBuf) -> DaemonServer {
    DaemonServer::with_session_dir(
        Arc::new(|params| {
            let agent = Agent::new(
                Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                Arc::new(mew_hooks::NopDispatcher),
                Some(params.writer),
                Vec::new(),
                None,
            );
            Ok((agent, None, None))
        }),
        path,
    )
}

pub(super) async fn connect(server: &DaemonServer) -> TestSocket {
    let (client, stream) = tokio::io::duplex(256 * 1024);
    let manager = server.session_manager.clone();
    let groups = server.groups_store.clone();
    let auto_summary = server.auto_summary_enabled.clone();
    let thinking_setter = server.thinking_setter.clone();
    tokio::spawn(async move {
        handle_connection(stream, manager, groups, thinking_setter, auto_summary)
            .await
            .unwrap();
    });
    client_async("ws://localhost/", client).await.unwrap().0
}

pub(super) async fn send(socket: &mut TestSocket, message: ClientMessage) {
    socket
        .send(Message::Text(mew_protocol::encode_json(&message).unwrap()))
        .await
        .unwrap();
}

// Ping is handled after previous commands and shares their ordered outbound queue.
pub(super) async fn flush(socket: &mut TestSocket) -> Vec<ServerMessage> {
    send(socket, ClientMessage::Ping).await;
    let mut messages = Vec::new();
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .expect("daemon reply timed out")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
            if matches!(message, ServerMessage::Pong { .. }) {
                return messages;
            }
            messages.push(message);
        }
    }
}

async fn publish(
    server: &DaemonServer,
    session: &Arc<Session>,
    event: AgentEvent,
) -> Vec<ServerMessage> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let (client_id, _) = session.attach_client(sender, ClientKind::Unknown).await;
    for message in translate_event(event, session, &server.session_manager).await {
        session.broadcast(message).await;
    }
    let mut messages = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        messages.push(message);
    }
    session.detach_client(client_id).await;
    messages
}

fn request_id(message: &ServerMessage) -> Option<&str> {
    match message {
        ServerMessage::PermissionRequest { request_id, .. }
        | ServerMessage::WorkspacePermissionRequest { request_id, .. }
        | ServerMessage::SubagentPermissionRequest { request_id, .. }
        | ServerMessage::AskUserRequest { request_id, .. }
        | ServerMessage::GoalProposed { request_id, .. } => Some(request_id),
        _ => None,
    }
}

fn hook_call() -> mew_hooks::ToolCall {
    mew_hooks::ToolCall {
        tool_name: "write".into(),
        call_id: "write-1".into(),
        input: serde_json::json!({"path": "answer.txt", "content": "hello"}),
    }
}

#[tokio::test]
async fn history_pages_preserve_messages_before_compaction_and_live_updates() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let session_id = session.agent.lock().await.session_id;
    let mut expected = Vec::new();
    for index in 0..mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT + 2 {
        let message_id = mew_message::MessageId::new();
        expected.push(mew_message::Message {
            id: message_id,
            session_id,
            role: mew_message::Role::User,
            parts: vec![mew_message::Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: mew_message::PartId::new(),
                    message_id,
                    session_id,
                },
                text: format!("message {index}"),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: index as i64,
                completed: None,
            },
            assistant: None,
        });
    }
    let tail_start_id = expected.last().unwrap().id;
    let mut marker = expected.last().unwrap().clone();
    marker.id = mew_message::MessageId::new();
    marker.role = mew_message::Role::System;
    marker.parts = vec![mew_message::Part::Compaction(mew_message::CompactionPart {
        base: mew_message::PartBase {
            id: mew_message::PartId::new(),
            message_id: marker.id,
            session_id: marker.session_id,
        },
        auto: true,
        overflow: false,
        tail_start_id: Some(tail_start_id),
        summary: None,
        removed_count: Some(expected.len() as u32 - 1),
    })];
    expected.push(marker);
    let persisted = expected.clone();
    {
        let agent = session.agent.lock().await;
        let mut writer = agent.session.as_ref().unwrap().lock().await;
        for message in &expected {
            writer.write_message(message).await.unwrap();
        }
        drop(writer);
        agent.load_messages(expected.clone()).await;
        let mut context = agent.messages.lock().await;
        assert_eq!(
            context.len(),
            2,
            "the model retains only the compacted tail"
        );
        let retained = context.last_mut().unwrap();
        retained.parts.clear();
        retained.time.completed = Some(100);
        expected[mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT + 1] = retained.clone();
        let mut live = retained.clone();
        live.id = mew_message::MessageId::new();
        live.time.created = 101;
        context.push(live.clone());
        expected.push(live);
    }

    for expected in [expected, persisted] {
        for client_kind in [ClientKind::Tui, ClientKind::Desktop] {
            let mut socket = connect(&server).await;
            send(
                &mut socket,
                ClientMessage::AttachSession {
                    session_id: session.id.clone(),
                    client_kind,
                },
            )
            .await;
            let mut recovered = Vec::new();
            let mut replace_expected = true;
            loop {
                let reply = flush(&mut socket).await;
                let pages = reply
                    .into_iter()
                    .filter_map(|message| match message {
                        ServerMessage::SessionHistoryPage {
                            messages,
                            next_cursor,
                            replace,
                            ..
                        } => Some((messages, next_cursor, replace)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    pages.len(),
                    if client_kind == ClientKind::Tui {
                        expected
                            .len()
                            .div_ceil(mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT)
                    } else {
                        1
                    }
                );
                let mut cursor = None;
                for (messages, next_cursor, replace) in pages {
                    assert_eq!(replace, replace_expected);
                    if replace_expected {
                        assert_eq!(messages.len(), mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT);
                    }
                    replace_expected = false;
                    recovered.splice(..0, messages);
                    cursor = next_cursor;
                }
                let Some(before) = cursor else {
                    break;
                };
                assert_eq!(client_kind, ClientKind::Desktop);
                send(
                    &mut socket,
                    ClientMessage::LoadSessionHistory {
                        session_id: session.id.clone(),
                        before,
                        limit: mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT,
                    },
                )
                .await;
            }

            assert_eq!(recovered, expected);
        }
        server.session_manager.remove(&session.id).await;
    }
}

#[tokio::test]
async fn history_read_errors_are_reported_on_attach_and_older_page_requests() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let (tx, rx) = oneshot::channel();
    let pending = publish(
        &server,
        &session,
        AgentEvent::PermissionRequest {
            call: hook_call(),
            tx,
        },
    )
    .await;
    let permission_id = pending.iter().find_map(request_id).unwrap().to_owned();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;
    tokio::fs::write(
        directory.path().join(&session.id).join("session.jsonl"),
        "invalid JSON\n",
    )
    .await
    .unwrap();

    send(
        &mut socket,
        ClientMessage::LoadSessionHistory {
            session_id: session.id.clone(),
            before: 1,
            limit: mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT,
        },
    )
    .await;
    let page_reply = flush(&mut socket).await;
    let mut attaching = connect(&server).await;
    send(
        &mut attaching,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    let attach_reply = flush(&mut attaching).await;
    assert!(attach_reply
        .iter()
        .any(|message| request_id(message) == Some(permission_id.as_str())));

    for reply in [page_reply, attach_reply] {
        assert!(reply.iter().any(|message| matches!(message,
            ServerMessage::Error { message } if message.contains("failed to load session history")
        )));
        assert!(!reply
            .iter()
            .any(|message| matches!(message, ServerMessage::SessionHistoryPage { .. })));
    }
    send(
        &mut attaching,
        ClientMessage::PermissionResponse {
            request_id: permission_id,
            decision: mew_protocol::PermissionDecision::AllowOnce,
        },
    )
    .await;
    flush(&mut attaching).await;
    assert!(matches!(
        rx.await.unwrap(),
        mew_hooks::PermissionDecision::AllowOnce
    ));
}

#[tokio::test]
async fn attach_recovers_each_required_action_and_original_response_channel() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let mut requests = Vec::new();
    let mut permission_receivers = Vec::new();
    for kind in 0..3 {
        let (tx, rx) = oneshot::channel();
        permission_receivers.push(rx);
        let event = match kind {
            0 => AgentEvent::PermissionRequest {
                call: hook_call(),
                tx,
            },
            1 => AgentEvent::WorkspacePermissionRequest {
                path: PathBuf::from("/outside/workspace"),
                tx,
            },
            _ => AgentEvent::SubagentPermissionRequest {
                parent_call_id: "parent-1".into(),
                call: hook_call(),
                tx,
            },
        };
        let messages = publish(&server, &session, event).await;
        assert!(messages
            .iter()
            .any(|message| matches!(message, ServerMessage::SessionAlert { .. })));
        assert!(messages.iter().any(|message| matches!(message, ServerMessage::SessionAttentionChanged { pending_permissions, .. } if *pending_permissions == kind + 1)));
        requests.push(
            messages
                .into_iter()
                .find(|message| request_id(message).is_some())
                .unwrap(),
        );
    }
    let (ask_tx, ask_rx) = oneshot::channel();
    let messages = publish(
        &server,
        &session,
        AgentEvent::AskUser {
            call_id: "ask-1".into(),
            questions: vec![mew_agent::AskUserQuestion {
                prompt: "which project?".into(),
                options: Vec::new(),
            }],
            tx: ask_tx,
        },
    )
    .await;
    requests.push(
        messages
            .into_iter()
            .find(|message| request_id(message).is_some())
            .unwrap(),
    );
    let (goal_tx, goal_rx) = oneshot::channel();
    let messages = publish(
        &server,
        &session,
        AgentEvent::GoalProposed {
            call_id: "goal-1".into(),
            objective: "finish the project".into(),
            tx: goal_tx,
        },
    )
    .await;
    requests.push(
        messages
            .into_iter()
            .find(|message| request_id(message).is_some())
            .unwrap(),
    );

    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Desktop,
        },
    )
    .await;
    let replay = flush(&mut socket).await;
    let history_index = replay
        .iter()
        .position(|message| matches!(message, ServerMessage::SessionHistoryPage { .. }))
        .unwrap();
    for request in &requests {
        let recovered = replay
            .iter()
            .position(|message| request_id(message) == request_id(request))
            .expect("pending request missing from attach replay");
        assert!(
            recovered > history_index,
            "actions must follow the attached history snapshot"
        );
        assert_eq!(
            serde_json::to_value(&replay[recovered]).unwrap(),
            serde_json::to_value(request).unwrap()
        );
    }
    for (request, receiver) in requests[..3].iter().zip(permission_receivers) {
        send(
            &mut socket,
            ClientMessage::PermissionResponse {
                request_id: request_id(request).unwrap().into(),
                decision: mew_protocol::PermissionDecision::AllowOnce,
            },
        )
        .await;
        flush(&mut socket).await;
        assert!(matches!(
            receiver.await.unwrap(),
            mew_hooks::PermissionDecision::AllowOnce
        ));
    }
    send(
        &mut socket,
        ClientMessage::AskUserResponse {
            request_id: request_id(&requests[3]).unwrap().into(),
            answers: vec!["project-a".into()],
        },
    )
    .await;
    flush(&mut socket).await;
    assert_eq!(ask_rx.await.unwrap(), vec!["project-a"]);
    send(
        &mut socket,
        ClientMessage::GoalResponse {
            request_id: request_id(&requests[4]).unwrap().into(),
            accepted: false,
        },
    )
    .await;
    flush(&mut socket).await;
    assert!(matches!(goal_rx.await.unwrap(), GoalDecision::Rejected));
    assert_eq!(session.pending_questions_count().await, 0);
}

#[tokio::test]
async fn wrong_kind_and_duplicate_responses_do_not_resolve_pending_actions() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let (tx, rx) = oneshot::channel();
    let messages = publish(
        &server,
        &session,
        AgentEvent::AskUser {
            call_id: "ask-1".into(),
            questions: Vec::new(),
            tx,
        },
    )
    .await;
    let id = messages.iter().find_map(request_id).unwrap().to_owned();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    flush(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::PermissionResponse {
            request_id: id.clone(),
            decision: mew_protocol::PermissionDecision::AllowOnce,
        },
    )
    .await;
    let messages = flush(&mut socket).await;
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, ServerMessage::RequestResolved { .. })),
        "wrong-kind response dismissed a live question"
    );
    assert_eq!(session.pending_questions_count().await, 1);
    send(
        &mut socket,
        ClientMessage::AskUserResponse {
            request_id: id.clone(),
            answers: vec!["answer".into()],
        },
    )
    .await;
    let messages = flush(&mut socket).await;
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ServerMessage::RequestResolved { .. }))
            .count(),
        1
    );
    assert_eq!(rx.await.unwrap(), vec!["answer"]);
    send(
        &mut socket,
        ClientMessage::AskUserResponse {
            request_id: id,
            answers: Vec::new(),
        },
    )
    .await;
    let messages = flush(&mut socket).await;
    assert!(!messages
        .iter()
        .any(|message| matches!(message, ServerMessage::RequestResolved { .. })));
}

#[tokio::test]
async fn cancellation_dismisses_pending_cards_and_replay_has_no_stale_request() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    flush(&mut socket).await;
    let (tx, rx) = oneshot::channel();
    let messages = publish(
        &server,
        &session,
        AgentEvent::GoalProposed {
            call_id: "goal-1".into(),
            objective: "finish".into(),
            tx,
        },
    )
    .await;
    let id = messages.iter().find_map(request_id).unwrap().to_owned();
    let (permission_tx, permission_rx) = oneshot::channel();
    let permissions = publish(
        &server,
        &session,
        AgentEvent::PermissionRequest {
            call: hook_call(),
            tx: permission_tx,
        },
    )
    .await;
    let permission_id = permissions.iter().find_map(request_id).unwrap().to_owned();
    let (question_tx, question_rx) = oneshot::channel();
    let questions = publish(
        &server,
        &session,
        AgentEvent::AskUser {
            call_id: "pending-question".into(),
            questions: Vec::new(),
            tx: question_tx,
        },
    )
    .await;
    let question_id = questions.iter().find_map(request_id).unwrap().to_owned();
    flush(&mut socket).await;
    send(&mut socket, ClientMessage::Cancel).await;
    let messages = flush(&mut socket).await;
    assert!(messages.iter().any(|message| matches!(message, ServerMessage::RequestResolved { request_id } if *request_id == id)));
    for id in [&permission_id, &question_id] {
        assert!(messages.iter().any(|message| matches!(message, ServerMessage::RequestResolved { request_id } if request_id == id)));
    }
    assert!(messages.iter().any(|message| matches!(
        message,
        ServerMessage::SessionAttentionChanged {
            pending_permissions: 0,
            pending_questions: 0,
            ..
        }
    )));
    assert!(rx.await.is_err());
    assert!(permission_rx.await.is_err());
    assert!(question_rx.await.is_err());
    let mut returning = connect(&server).await;
    send(
        &mut returning,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    assert!(!flush(&mut returning)
        .await
        .iter()
        .any(|message| request_id(message).is_some()));
}

#[tokio::test]
async fn closed_request_channels_are_not_published_or_replayed() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let (tx, rx) = oneshot::channel();
    drop(rx);
    let messages = publish(
        &server,
        &session,
        AgentEvent::GoalProposed {
            call_id: "goal-closed".into(),
            objective: "finish".into(),
            tx,
        },
    )
    .await;
    assert!(messages.is_empty(), "closed action was published");
    assert_eq!(session.pending_questions_count().await, 0);
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    assert!(!flush(&mut socket)
        .await
        .iter()
        .any(|message| request_id(message).is_some()));
}

#[tokio::test]
async fn buffered_requests_from_a_cancelled_turn_are_not_republished() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let token = CancellationToken::new();
    *session.current_turn_cancel.lock().await = Some(token.clone());
    session.cancel_turn().await;
    assert!(token.is_cancelled());
    // The tool's event can be buffered before its receiver observes cancellation.
    let (tx, rx) = oneshot::channel();
    let messages = publish(
        &server,
        &session,
        AgentEvent::GoalProposed {
            call_id: "buffered-goal".into(),
            objective: "finish".into(),
            tx,
        },
    )
    .await;
    assert!(
        messages.is_empty(),
        "cancelled turn published a stale action"
    );
    assert!(rx.await.is_err());
    assert_eq!(session.pending_questions_count().await, 0);
}
