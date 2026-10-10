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
            cancelled: false,
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
            cancelled: false,
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
            cancelled: false,
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

#[tokio::test]
async fn delayed_todo_events_publish_canonical_state_instead_of_old_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let server = test_server(dir.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    session
        .attach_client(tx, mew_protocol::ClientKind::Unknown)
        .await;
    let mut old = mew_agent::TodoList::new();
    old.create(vec![("old content".into(), vec![])]);
    let todos = session.agent.lock().await.todos.clone();
    let mut latest = old.clone();
    latest
        .update(
            1,
            Some("confirmed edit".into()),
            Some(mew_agent::TodoStatus::Blocked),
        )
        .unwrap();
    *todos.lock().await = latest;
    let messages = translate_event(
        AgentEvent::TodosUpdated { todos: old.items },
        &session,
        &server.session_manager,
    )
    .await;
    assert!(
        messages.is_empty(),
        "todo publication must remain inside its canonical lock"
    );
    let ServerMessage::TodosUpdated { todos } = rx.recv().await.unwrap() else {
        panic!("expected todo broadcast")
    };
    assert_eq!(todos[0].content, "confirmed edit");
    assert_eq!(todos[0].status, "blocked");
}

#[tokio::test]
async fn child_question_replays_and_resolves_only_its_original_channel() {
    for mode in 0..3 {
        let cancelled = mode != 0;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let mut script = mew_provider_fake::FakeProvider::tool_call(
            "ask_user_question",
            "child-ask",
            serde_json::json!({
                "questions": [{"prompt":"which file?", "options":[{"label":"source", "description":"implementation"}, {"label":"test", "description":"regression"}]}]
            }),
        );
        script.pop();
        script.extend(mew_provider_fake::FakeProvider::tool_call(
            "exit_tool",
            "child-exit",
            serde_json::json!({"final_answer":"child done"}),
        ));
        let child_provider = Arc::new(mew_provider_fake::FakeProvider::new(script));
        let runner = Arc::new(
            mew_agent::runner::SimpleRunner::new(
                child_provider,
                vec![Arc::new(mew_tools::tools::exit_tool::ExitTool)],
                Arc::new(mew_hooks::NopDispatcher),
            )
            .with_session_root(root.clone()),
        );
        let server = DaemonServer::with_session_dir(
            Arc::new(move |params| {
                let mut agent = Agent::new(
                    Arc::new(mew_provider_fake::FakeProvider::new(vec![])),
                    Arc::new(mew_hooks::NopDispatcher),
                    Some(params.writer),
                    vec![],
                    None,
                );
                agent.subagent_defs = vec![mew_subagents::SubagentDef {
                    name: "worker".into(),
                    description: "worker".into(),
                    model: None,
                    tools: None,
                    max_turns: Some(10),
                    max_duration_secs: Some(60),
                    body: String::new(),
                    path: PathBuf::new(),
                    template: false,
                    can_spawn: false,
                    output_schema: None,
                }];
                agent.subagent_runner = Some(runner.clone());
                Ok((agent, None, None))
            }),
            root.clone(),
        );
        let session = server.session_manager.create(None).await.unwrap();
        let (tx, mut rx) = mpsc::channel(64);
        let task = session
            .agent
            .lock()
            .await
            .start_subagent("worker", "ask", None, None, &tx)
            .await
            .unwrap();
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = rx.recv().await.unwrap();
                if matches!(event, AgentEvent::SubagentAskUser { .. }) {
                    break publish(&server, &session, event)
                        .await
                        .into_iter()
                        .find(|message| request_id(message).is_some())
                        .unwrap();
                }
            }
        })
        .await
        .unwrap();
        let ServerMessage::AskUserRequest {
            subagent: Some(source),
            questions,
            ..
        } = &request
        else {
            panic!("missing child identity: {request:?}");
        };
        assert_eq!(source.parent_call_id, task);
        assert_eq!(source.name, "worker");
        assert_eq!(questions[0].options[1].description, "regression");
        let child_id = source.child_session_id.clone();
        let (parent_tx, mut parent_rx) = oneshot::channel();
        publish(
            &server,
            &session,
            AgentEvent::AskUser {
                call_id: "parent-ask".into(),
                questions: vec![mew_agent::AskUserQuestion {
                    prompt: "parent?".into(),
                    options: vec![],
                }],
                tx: parent_tx,
            },
        )
        .await;
        let mut socket = connect(&server).await;
        send(
            &mut socket,
            ClientMessage::AttachSession {
                session_id: session.id.clone(),
                client_kind: ClientKind::Tui,
            },
        )
        .await;
        let replay = flush(&mut socket).await;
        assert!(replay
            .iter()
            .any(|message| serde_json::to_value(message).unwrap()
                == serde_json::to_value(&request).unwrap()));
        let id = request_id(&request).unwrap().to_owned();
        if mode == 2 {
            assert!(session.agent.lock().await.cancel_subagent(&task).await);
        } else {
            send(
                &mut socket,
                ClientMessage::AskUserResponse {
                    request_id: id.clone(),
                    answers: vec!["test".into()],
                    cancelled,
                },
            )
            .await;
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                if matches!(event, AgentEvent::SubagentEnd { .. }) {
                    publish(&server, &session, event).await;
                    break;
                }
            }
        })
        .await
        .unwrap();
        let resolved = flush(&mut socket).await;
        assert!(resolved.iter().any(|message| matches!(message, ServerMessage::RequestResolved { request_id } if request_id == &id)), "missing resolution for mode {mode}: {resolved:?}");
        assert_eq!(session.pending_questions_count().await, 1);
        assert!(matches!(
            parent_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let parent_id = session.agent.lock().await.session_id.to_string();
        let messages = mew_session::Reader::load_subagent_from(&root, &parent_id, &child_id)
            .await
            .unwrap();
        assert!(messages
            .iter()
            .flat_map(|message| &message.parts)
            .any(|part| match part {
                mew_message::Part::ToolCall(call) if call.call_id == "child-ask" =>
                    match &call.state {
                        mew_message::ToolState::Completed(state) =>
                            !cancelled && state.output.contains("A (selected): test"),
                        mew_message::ToolState::Error(state) =>
                            cancelled && state.error.contains("cancelled"),
                        _ => false,
                    },
                _ => false,
            }));
    }
}

async fn receipt(
    socket: &mut TestSocket,
    request: &str,
) -> (
    mew_protocol::PromptStatus,
    Option<String>,
    Vec<ServerMessage>,
) {
    let mut messages = Vec::new();
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
            if let ServerMessage::PromptReceipt {
                request_id,
                status,
                message,
                ..
            } = &message
            {
                if request_id == request {
                    return (*status, message.clone(), messages);
                }
            }
            messages.push(message);
        }
    }
}

#[tokio::test]
async fn cancelling_before_the_agent_lock_is_released_prevents_provider_work() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingProvider {
        provider: mew_provider_fake::FakeProvider,
        requests: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl mew_provider::Provider for RecordingProvider {
        fn name(&self) -> &str {
            "fake"
        }

        async fn stream(
            &self,
            request: mew_provider::Request,
        ) -> Result<mew_provider::EventStream, mew_provider::ProviderError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            self.provider.stream(request).await
        }
    }

    for command in ["submit", "prompt", "wiki"] {
        let directory = tempfile::tempdir().unwrap();
        let provider = Arc::new(RecordingProvider {
            provider: mew_provider_fake::FakeProvider::new(
                mew_provider_fake::FakeProvider::text_response("must not run"),
            ),
            requests: AtomicUsize::new(0),
        });
        let session_provider = provider.clone();
        let server = DaemonServer::with_session_dir(
            Arc::new(move |params| {
                let agent = Agent::new(
                    session_provider.clone(),
                    Arc::new(mew_hooks::NopDispatcher),
                    Some(params.writer),
                    vec![],
                    None,
                );
                Ok((agent, None, None))
            }),
            directory.path().to_path_buf(),
        );
        let session = server.session_manager.create(None).await.unwrap();
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
        let agent = session.agent.lock().await;
        send(
            &mut socket,
            match command {
                "submit" => ClientMessage::SubmitPrompt {
                    session_id: session.id.clone(),
                    request_id: "cancel-before-start".into(),
                    text: "must not run".into(),
                    attachments: vec![],
                },
                "prompt" => ClientMessage::Prompt {
                    text: "must not run".into(),
                    attachments: vec![],
                },
                _ => ClientMessage::SlashCommand {
                    command: "/wiki".into(),
                },
            },
        )
        .await;
        if command == "submit" {
            assert_eq!(
                receipt(&mut socket, "cancel-before-start").await.0,
                mew_protocol::PromptStatus::Accepted
            );
        } else {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while !*session.is_running.lock().await {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("turn must register before waiting for the agent lock");
        }
        send(&mut socket, ClientMessage::Cancel).await;
        flush(&mut socket).await;
        assert_eq!(provider.requests.load(Ordering::SeqCst), 0);
        drop(agent);

        let mut messages = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut messages = Vec::new();
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = frame {
                    let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
                    let idle = matches!(&message, ServerMessage::SessionActivityChanged {
                        session_id, activity: SessionState::Idle,
                    } if session_id == &session.id);
                    messages.push(message);
                    if idle {
                        break messages;
                    }
                }
            }
        })
        .await
        .expect("cancelled turn must return to idle");
        let _finished = session.turn_lock.lock().await;
        messages.extend(flush(&mut socket).await);
        if command == "wiki" {
            assert!(messages.iter().any(|message| matches!(
                message,
                ServerMessage::SlashResult { text } if text == "wiki generation cancelled"
            )));
        }
        assert_eq!(provider.requests.load(Ordering::SeqCst), 0, "{command}");
        assert!(!messages.iter().any(|message| matches!(
            message,
            ServerMessage::Provider { .. } | ServerMessage::ToolStart { .. }
        )));
        assert!(!*session.is_running.lock().await);
        assert!(session.current_turn_cancel.lock().await.is_none());
        if command == "submit" {
            let history = mew_session::Reader::load_from(directory.path(), &session.id)
                .await
                .unwrap();
            assert_eq!(history, *session.agent.lock().await.messages.lock().await);
            assert_eq!(history.len(), 1, "accepted input must remain durable");
            assert_eq!(history[0].role, mew_message::Role::User);
            assert!(matches!(
                &history[0].parts[0],
                mew_message::Part::Text(part) if part.text == "must not run"
            ));
        }
    }
}

#[tokio::test]
async fn scoped_prompt_rejects_wrong_attachment_and_busy_session_without_echoing_user_text() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let a = server.session_manager.create(None).await.unwrap();
    let b = server.session_manager.create(None).await.unwrap();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: b.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;
    for (target, request) in [(&a.id, "wrong-owner"), (&b.id, "busy")] {
        let guard = b.turn_lock.lock().await;
        *b.is_running.lock().await = true;
        send(
            &mut socket,
            ClientMessage::SubmitPrompt {
                session_id: target.clone(),
                request_id: request.into(),
                text: "must stay in draft".into(),
                attachments: Vec::new(),
            },
        )
        .await;
        let (status, _, messages) = receipt(&mut socket, request).await;
        assert_eq!(status, mew_protocol::PromptStatus::Rejected);
        assert!(!messages
            .iter()
            .any(|m| matches!(m, ServerMessage::UserMessage { .. })));
        drop(guard);
        *b.is_running.lock().await = false;
    }
    assert!(a.agent.lock().await.messages.lock().await.is_empty());
    assert!(b.agent.lock().await.messages.lock().await.is_empty());
}

#[tokio::test]
async fn accepted_prompt_receipt_survives_reconnect_and_duplicate_submission_runs_once() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
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
    let prompt = || ClientMessage::SubmitPrompt {
        session_id: session.id.clone(),
        request_id: "accepted-once".into(),
        text: "hello".into(),
        attachments: Vec::new(),
    };
    send(&mut socket, prompt()).await;
    assert_eq!(
        receipt(&mut socket, "accepted-once").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    socket.close(None).await.unwrap();
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
    send(
        &mut socket,
        ClientMessage::ReadPromptReceipt {
            session_id: session.id.clone(),
            request_id: "accepted-once".into(),
        },
    )
    .await;
    assert_eq!(
        receipt(&mut socket, "accepted-once").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    send(&mut socket, prompt()).await;
    assert_eq!(
        receipt(&mut socket, "accepted-once").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    let _finished = session.turn_lock.lock().await;
    let agent = session.agent.lock().await;
    assert_eq!(
        agent
            .messages
            .lock()
            .await
            .iter()
            .filter(|m| m.role == mew_message::Role::User)
            .count(),
        1
    );
    drop(agent);
    drop(_finished);
    send(
        &mut socket,
        ClientMessage::ReadPromptReceipt {
            session_id: session.id.clone(),
            request_id: "never-sent".into(),
        },
    )
    .await;
    assert_eq!(
        receipt(&mut socket, "never-sent").await.0,
        mew_protocol::PromptStatus::Unknown
    );
}

#[tokio::test]
async fn follow_up_waits_for_a_finished_turn_to_release_its_guard() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
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
    let finishing = session.turn_lock.lock().await;
    assert!(!*session.is_running.lock().await);
    send(
        &mut socket,
        ClientMessage::SubmitPrompt {
            session_id: session.id.clone(),
            request_id: "finishing-follow-up".into(),
            text: "follow-up".into(),
            attachments: vec![],
        },
    )
    .await;
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(30),
        receipt(&mut socket, "finishing-follow-up")
    )
    .await
    .is_err());
    drop(finishing);
    assert_eq!(
        receipt(&mut socket, "finishing-follow-up").await.0,
        mew_protocol::PromptStatus::Accepted
    );
}

#[tokio::test]
async fn receipt_lookup_keeps_its_owner_after_attaching_elsewhere() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let a = server.session_manager.create(None).await.unwrap();
    let b = server.session_manager.create(None).await.unwrap();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: a.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::SubmitPrompt {
            session_id: a.id.clone(),
            request_id: "a-receipt".into(),
            text: "a prompt".into(),
            attachments: vec![],
        },
    )
    .await;
    assert_eq!(
        receipt(&mut socket, "a-receipt").await.0,
        mew_protocol::PromptStatus::Accepted
    );
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: b.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::ReadPromptReceipt {
            session_id: a.id.clone(),
            request_id: "a-receipt".into(),
        },
    )
    .await;
    assert_eq!(
        receipt(&mut socket, "a-receipt").await.0,
        mew_protocol::PromptStatus::Accepted
    );
}

#[tokio::test]
async fn equal_prompt_text_is_broadcast_with_each_exact_submission_id() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
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
    for request in ["first", "second"] {
        send(
            &mut socket,
            ClientMessage::SubmitPrompt {
                session_id: session.id.clone(),
                request_id: request.into(),
                text: "same prompt".into(),
                attachments: Vec::new(),
            },
        )
        .await;
        assert_eq!(
            receipt(&mut socket, request).await.0,
            mew_protocol::PromptStatus::Accepted
        );
        loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let Message::Text(text) = frame else {
                continue;
            };
            let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
            if let ServerMessage::UserMessage { text, request_id } = message {
                assert_eq!(text, "same prompt");
                assert_eq!(request_id.as_deref(), Some(request));
                break;
            }
        }
        let finished = session.turn_lock.lock().await;
        drop(finished);
    }
    assert_eq!(
        session
            .agent
            .lock()
            .await
            .messages
            .lock()
            .await
            .iter()
            .filter(|message| message.role == mew_message::Role::User)
            .count(),
        2
    );
}

#[tokio::test]
async fn activity_attach_snapshot_restores_current_work_and_removes_missed_completions() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let a = server.session_manager.create(None).await.unwrap();
    let b = server.session_manager.create(None).await.unwrap();
    a.broadcast(ServerMessage::SubagentStart {
        parent_call_id: "task".into(),
        child_session_id: "child".into(),
        name: "worker".into(),
        display_name: Some("worker one".into()),
    })
    .await;
    a.broadcast(ServerMessage::SubagentStatus {
        parent_call_id: "task".into(),
        tool_name: "shell".into(),
        message: "checking".into(),
    })
    .await;
    a.broadcast(ServerMessage::JobUpdate {
        job_id: "job".into(),
        command: "sleep 1".into(),
        state: "running".into(),
    })
    .await;
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: a.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    let first = flush(&mut socket).await;
    let (subagents, jobs) = first
        .iter()
        .find_map(|message| match message {
            ServerMessage::SessionActivitySnapshot {
                session_id,
                subagents,
                jobs,
            } if session_id == &a.id => Some((subagents, jobs)),
            _ => None,
        })
        .expect("first attach must restore active work");
    assert_eq!(subagents[0].child_session_id.as_deref(), Some("child"));
    assert_eq!(subagents[0].status, "checking");
    assert_eq!(jobs[0].state, "running");
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: b.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;
    a.broadcast(ServerMessage::SubagentEnd {
        parent_call_id: "task".into(),
        child_session_id: "child".into(),
        outcome: mew_protocol::SubagentOutcome::Completed,
        manifests: Vec::new(),
    })
    .await;
    a.broadcast(ServerMessage::JobUpdate {
        job_id: "job".into(),
        command: "sleep 1".into(),
        state: "completed".into(),
    })
    .await;
    socket.close(None).await.unwrap();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: a.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    let replay = flush(&mut socket).await;
    let (subagents, jobs) = replay
        .iter()
        .find_map(|message| match message {
            ServerMessage::SessionActivitySnapshot {
                session_id,
                subagents,
                jobs,
            } if session_id == &a.id => Some((subagents, jobs)),
            _ => None,
        })
        .expect("reconnect must restore authoritative activity");
    assert!(subagents.is_empty());
    assert_eq!(jobs[0].state, "completed");
}

#[tokio::test]
async fn job_output_reads_only_attached_session_and_preserves_offsets() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let parent = server.session_manager.create(None).await.unwrap();
    let other = server.session_manager.create(None).await.unwrap();
    let job_id = {
        let agent = parent.agent.lock().await;
        let id = agent
            .start_shell_job("printf '日本語\\nfinal\\n'", directory.path())
            .await;
        assert!(agent.shell_job_block(&id, 3).await.unwrap().0.is_terminal());
        id
    };
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: parent.id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    flush(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::ReadJobOutput {
            session_id: parent.id.clone(),
            job_id: job_id.clone(),
            request_id: "first".into(),
            offset: 0,
        },
    )
    .await;
    let messages = flush(&mut socket).await;
    assert!(messages.iter().any(|message| matches!(message, ServerMessage::JobOutput { session_id, job_id: actual, request_id, offset: 0, next_offset: 16, output, done: true, incomplete: false } if session_id == &parent.id && actual == &job_id && request_id == "first" && output == "日本語\nfinal\n")), "{messages:?}");
    for (session_id, id, offset) in [
        (other.id.clone(), job_id.clone(), 0),
        (parent.id.clone(), "missing".into(), 0),
        (parent.id.clone(), job_id.clone(), 1),
        (parent.id.clone(), job_id.clone(), 17),
    ] {
        send(
            &mut socket,
            ClientMessage::ReadJobOutput {
                session_id: session_id.clone(),
                job_id: id.clone(),
                request_id: "error".into(),
                offset,
            },
        )
        .await;
        let messages = flush(&mut socket).await;
        assert!(messages.iter().any(|message| matches!(message, ServerMessage::JobOutputError { session_id: actual, job_id, request_id, message } if actual == &session_id && job_id == &id && request_id == "error" && !message.is_empty())), "{messages:?}");
    }
    send(
        &mut socket,
        ClientMessage::ReadJobOutput {
            session_id: parent.id.clone(),
            job_id,
            request_id: "end".into(),
            offset: 16,
        },
    )
    .await;
    assert!(flush(&mut socket).await.iter().any(|message| matches!(message, ServerMessage::JobOutput { request_id, offset: 16, next_offset: 16, output, done: true, .. } if request_id == "end" && output.is_empty())));
}

#[tokio::test]
async fn job_output_reserves_partial_utf8_at_the_chunk_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let job_id = {
        let agent = session.agent.lock().await;
        let id = agent
            .start_shell_job(
                "awk 'BEGIN {for(i=0;i<65533;i++) printf \"x\"; printf \"🐱tail\\n\"}'",
                directory.path(),
            )
            .await;
        assert!(agent.shell_job_block(&id, 3).await.unwrap().0.is_terminal());
        id
    };
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
    send(
        &mut socket,
        ClientMessage::ReadJobOutput {
            session_id: session.id.clone(),
            job_id: job_id.clone(),
            request_id: "first".into(),
            offset: 0,
        },
    )
    .await;
    let first = flush(&mut socket).await;
    assert!(first.iter().any(|message|matches!(message,ServerMessage::JobOutput {offset:0,next_offset:65_533,output,done:false,incomplete:false,..} if output==&"x".repeat(65_533))));
    send(
        &mut socket,
        ClientMessage::ReadJobOutput {
            session_id: session.id.clone(),
            job_id,
            request_id: "tail".into(),
            offset: 65_533,
        },
    )
    .await;
    let tail = flush(&mut socket).await;
    assert!(tail.iter().any(|message|matches!(message,ServerMessage::JobOutput {offset:65_533,next_offset:65_542,output,done:true,incomplete:false,..} if output=="🐱tail\n")),"{tail:?}");
}

#[tokio::test]
async fn idle_shell_jobs_publish_terminal_state_and_reconnect_without_losing_command() {
    for mode in 0..3 {
        let cancelled = mode == 1;
        let directory = tempfile::tempdir().unwrap();
        let command = if cancelled {
            "printf 'ready\\n'; sleep 5 & wait"
        } else if mode == 2 {
            "sleep 0.3; printf 'build failed\\n'; exit 7"
        } else {
            "sleep 0.3; printf 'final tail\\n'"
        };
        let mut script = mew_provider_fake::FakeProvider::tool_call(
            "shell_background",
            "start-job",
            serde_json::json!({"command": command}),
        );
        script.pop();
        script.extend(mew_provider_fake::FakeProvider::tool_call(
            "exit_tool",
            "exit",
            serde_json::json!({"final_answer": "started"}),
        ));
        let provider = Arc::new(mew_provider_fake::FakeProvider::new(script));
        let server = DaemonServer::with_session_dir(
            Arc::new(move |params| {
                let mut agent = Agent::new(
                    provider.clone(),
                    Arc::new(mew_hooks::NopDispatcher),
                    Some(params.writer),
                    vec![Arc::new(mew_tools::tools::exit_tool::ExitTool)],
                    None,
                );
                agent.set_permission_engine(Arc::new(
                    mew_config::permissions::PermissionEngine::new(vec![])
                        .with_mode(mew_hooks::PermissionMode::Dangerous),
                ));
                Ok((agent, None, None))
            }),
            directory.path().to_path_buf(),
        );
        let session = server.session_manager.create(None).await.unwrap();
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
        send(
            &mut socket,
            ClientMessage::Prompt {
                text: "start background work".into(),
                attachments: vec![],
            },
        )
        .await;
        let job_id = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut job_id = None;
            loop {
                if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                    let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
                    match message {
                        ServerMessage::JobUpdate {
                            job_id: id,
                            command: actual,
                            state,
                        } if state == "running" => {
                            assert_eq!(actual, command);
                            job_id = Some(id);
                        }
                        ServerMessage::SessionActivityChanged {
                            activity: SessionState::Idle,
                            ..
                        } => {
                            break job_id
                                .expect("background tool must start before the parent finishes")
                        }
                        _ => {}
                    }
                }
            }
        })
        .await
        .expect("parent turn must finish while the job remains active");
        if cancelled {
            assert!(session.agent.lock().await.cancel_shell_job(&job_id).await);
        }
        let expected = match mode {
            1 => "cancelled",
            2 => "failed",
            _ => "completed",
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                    let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
                    if let ServerMessage::JobUpdate {
                        job_id: id,
                        command: actual,
                        state,
                    } = message
                    {
                        if id == job_id && state == expected {
                            assert_eq!(actual, command);
                            break;
                        }
                    }
                }
            }
        })
        .await
        .expect("idle clients must receive job completion without another turn");
        let output = session
            .agent
            .lock()
            .await
            .shell_job_output(&job_id, 0)
            .await
            .unwrap();
        assert!(output.done);
        if mode == 0 {
            assert_eq!(output.output, "final tail\n");
        }
        if mode == 2 {
            assert_eq!(output.output, "build failed\n");
        }
        let stale = publish(
            &server,
            &session,
            AgentEvent::JobUpdate {
                job_id: job_id.clone(),
                command: String::new(),
                state: "running".into(),
            },
        )
        .await;
        assert!(stale.iter().all(|message| !matches!(message, ServerMessage::JobUpdate {state, ..} if state == "running")), "late status must not regress terminal state: {stale:?}");
        socket.close(None).await.unwrap();
        let mut socket = connect(&server).await;
        send(
            &mut socket,
            ClientMessage::AttachSession {
                session_id: session.id.clone(),
                client_kind: ClientKind::Tui,
            },
        )
        .await;
        let messages = flush(&mut socket).await;
        let jobs = messages
            .iter()
            .find_map(|message| match message {
                ServerMessage::SessionActivitySnapshot { jobs, .. } => Some(jobs),
                _ => None,
            })
            .expect("reconnect must replay jobs");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, job_id);
        assert_eq!(jobs[0].state, expected);
        assert_eq!(jobs[0].command, command);
    }
}

#[tokio::test]
async fn already_finished_job_publication_uses_terminal_source_state() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let job_id = {
        let agent = session.agent.lock().await;
        let id = agent
            .start_shell_job("printf 'done\\n'", directory.path())
            .await;
        assert!(agent.shell_job_block(&id, 3).await.unwrap().0.is_terminal());
        id
    };
    let messages = publish(
        &server,
        &session,
        AgentEvent::JobUpdate {
            job_id,
            command: String::new(),
            state: "running".into(),
        },
    )
    .await;
    assert!(messages.iter().any(|message| matches!(message, ServerMessage::JobUpdate {state, command, ..} if state == "completed" && command == "printf 'done\\n'")), "{messages:?}");
    assert!(messages.iter().all(
        |message| !matches!(message, ServerMessage::JobUpdate {state, ..} if state == "running")
    ));
}

#[tokio::test]
async fn nonzero_shell_exit_is_failed_with_original_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    let server = test_server(directory.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let agent = session.agent.lock().await;
    let id = agent.start_shell_job("exit 7", directory.path()).await;
    let (state, _) = agent.shell_job_block(&id, 3).await.unwrap();
    assert_eq!(state.as_str(), "failed");
    let jobs = agent.shell_jobs.lock().await;
    assert_eq!(
        format!("{:?}", *jobs[&id].state.lock().await),
        "Completed { exit_code: 7 }"
    );
}
