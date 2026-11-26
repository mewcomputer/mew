use super::*;
use crate::request_tests::{connect, flush, send};

#[tokio::test]
async fn initial_model_list_returns_other_models_before_delayed_discovery() {
    let root = tempfile::tempdir().unwrap();
    let signal = Arc::new(tokio::sync::Notify::new());
    let complete = signal.clone();
    let lister: ModelLister = Arc::new(move || {
        let complete = complete.clone();
        let immediate = mew_protocol::ModelInfo {
            id: "other/example".into(),
            provider: "other".into(),
            model: "example".into(),
            display_name: None,
            description: None,
            thinking_variants: Vec::new(),
            thinking_budget: None,
            context_window: None,
        };
        let current = vec![immediate.clone()];
        let update = Box::pin(async move {
            complete.notified().await;
            vec![
                immediate,
                mew_protocol::ModelInfo {
                    id: "antigravity/example".into(),
                    provider: "antigravity".into(),
                    model: "example".into(),
                    display_name: None,
                    description: None,
                    thinking_variants: Vec::new(),
                    thinking_budget: None,
                    context_window: None,
                },
            ]
        });
        (current, Some(update))
    });
    let server = policy_server(root.path().to_path_buf()).with_model_management(
        Arc::new(|_, provider, model| Ok((provider.into(), model.into()))),
        lister,
    );
    let mut socket = connect(&server).await;
    send(&mut socket, ClientMessage::ListModels).await;
    let messages = flush(&mut socket).await;
    assert!(messages.iter().any(|message| matches!(message, ServerMessage::ModelList { models } if models.len() == 1 && models[0].id == "other/example")));
    signal.notify_one();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = frame else {
        panic!("expected model list")
    };
    let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
    assert!(
        matches!(message, ServerMessage::ModelList { models } if models.len() == 2 && models[1].id == "antigravity/example")
    );
}

#[tokio::test]
async fn disconnect_cancels_pending_model_list_response() {
    struct NotifyDrop(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for NotifyDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let sender = Arc::new(std::sync::Mutex::new(Some(dropped_tx)));
    let lister: ModelLister = Arc::new({
        let started = started.clone();
        move || {
            let guard = NotifyDrop(sender.lock().unwrap().take());
            let started = started.clone();
            let update = Box::pin(async move {
                let _guard = guard;
                started.notify_one();
                std::future::pending::<Vec<mew_protocol::ModelInfo>>().await
            });
            (Vec::new(), Some(update))
        }
    });
    let server = policy_server(root.path().to_path_buf()).with_model_management(
        Arc::new(|_, provider, model| Ok((provider.into(), model.into()))),
        lister,
    );
    let mut socket = connect(&server).await;
    send(&mut socket, ClientMessage::ListModels).await;
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    drop(socket);
    tokio::time::timeout(std::time::Duration::from_secs(3), dropped_rx)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn newer_model_list_request_cancels_stale_update() {
    let root = tempfile::tempdir().unwrap();
    let signal = Arc::new(tokio::sync::Notify::new());
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lister: ModelLister = Arc::new({
        let signal = signal.clone();
        move || {
            let current = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if current == 0 {
                let signal = signal.clone();
                (
                    Vec::new(),
                    Some(Box::pin(async move {
                        signal.notified().await;
                        vec![mew_protocol::ModelInfo {
                            id: "stale/model".into(),
                            provider: "stale".into(),
                            model: "model".into(),
                            display_name: None,
                            description: None,
                            thinking_variants: Vec::new(),
                            thinking_budget: None,
                            context_window: None,
                        }]
                    })),
                )
            } else {
                (
                    vec![mew_protocol::ModelInfo {
                        id: "new/model".into(),
                        provider: "new".into(),
                        model: "model".into(),
                        display_name: None,
                        description: None,
                        thinking_variants: Vec::new(),
                        thinking_budget: None,
                        context_window: None,
                    }],
                    None,
                )
            }
        }
    });
    let server = policy_server(root.path().to_path_buf()).with_model_management(
        Arc::new(|_, provider, model| Ok((provider.into(), model.into()))),
        lister,
    );
    let mut socket = connect(&server).await;
    send(&mut socket, ClientMessage::ListModels).await;
    assert!(flush(&mut socket).await.iter().any(
        |message| matches!(message, ServerMessage::ModelList { models } if models.is_empty())
    ));
    send(&mut socket, ClientMessage::ListModels).await;
    let second = flush(&mut socket).await;
    assert!(second.iter().any(|message| matches!(message, ServerMessage::ModelList { models } if models.len() == 1 && models[0].id == "new/model")));
    signal.notify_one();
    assert!(!flush(&mut socket)
        .await
        .iter()
        .any(|message| matches!(message, ServerMessage::ModelList { .. })));
}

fn policy_server(root: PathBuf) -> DaemonServer {
    let builder: AgentBuilder = Arc::new(|params| {
        let policy = params.policy.unwrap_or(mew_session::SessionPolicy {
            model: Some("initial".into()),
            provider: Some("fake".into()),
            persona: Some("builder".into()),
            thinking_variant: Some("high".into()),
            permission_mode: "standard".into(),
        });
        let todos_path = params.writer.path().parent().unwrap().join("todos.json");
        let mut agent = Agent::new(
            Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
            Arc::new(mew_hooks::NopDispatcher),
            Some(params.writer),
            Vec::new(),
            None,
        );
        agent.todos_path = Some(todos_path);
        agent.set_model_info(
            policy.model.as_deref().unwrap_or_default(),
            policy.provider.as_deref().unwrap_or_default(),
        );
        let mode = mew_hooks::PermissionMode::from_id(&policy.permission_mode)
            .ok_or_else(|| anyhow::anyhow!("invalid saved permission mode"))?;
        agent.set_permission_engine(Arc::new(
            mew_config::permissions::PermissionEngine::new(Vec::new()).with_mode(mode),
        ));
        let mut personas = mew_personas::builtin_defaults();
        let mut pinned = personas
            .iter()
            .find(|p| p.name == "planner")
            .unwrap()
            .clone();
        pinned.name = "broken-pin".into();
        pinned.config.model = Some("fake/broken".into());
        pinned.config.skills = Some(Vec::new());
        personas.push(pinned);
        let mut inherited = personas
            .iter()
            .find(|p| p.name == "planner")
            .unwrap()
            .clone();
        inherited.name = "inherited-pin".into();
        inherited.config.model = Some("chosen".into());
        personas.push(inherited);
        agent.set_personas(personas.clone());
        if let Some(name) = policy.persona.as_deref() {
            agent.apply_persona(personas.iter().find(|p| p.name == name).unwrap());
        }
        agent.set_active_thinking_variant(policy.thinking_variant);
        Ok((agent, policy.model, policy.provider))
    });
    DaemonServer::with_session_dir(builder, root)
        .with_model_management(
            Arc::new(|agent, provider, model| {
                if model == "broken" {
                    anyhow::bail!("provider construction failed")
                }
                agent.set_model_info(model, provider);
                agent.set_active_thinking_variant(Some("high".into()));
                Ok((provider.into(), model.into()))
            }),
            Arc::new(|| (Vec::new(), None)),
        )
        .with_thinking_setter(Arc::new(|agent, _, variant| {
            let variant = (!variant.is_empty()).then(|| variant.to_owned());
            agent.set_active_thinking_variant(variant.clone());
            Ok(variant)
        }))
}

async fn next_setting(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
) -> Vec<ServerMessage> {
    let mut messages = Vec::new();
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Message::Text(text) = frame else { continue };
        let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
        let done = matches!(
            message,
            ServerMessage::PermissionModeChanged { .. }
                | ServerMessage::ThinkingVariantChanged { .. }
                | ServerMessage::PersonaSwitched { .. }
                | ServerMessage::Error { .. }
        );
        messages.push(message);
        if done {
            return messages;
        }
    }
}

#[tokio::test]
async fn confirmed_sessions_publish_persisted_todos_and_empty_new_snapshots() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    {
        let agent = session.agent.lock().await;
        let mut todos = agent.todos.lock().await;
        todos.create(vec![
            ("stored task".into(), vec![]),
            ("dependent task".into(), vec![1]),
        ]);
        todos
            .update(1, None, Some(mew_agent::TodoStatus::Blocked))
            .unwrap();
        todos
            .save(agent.todos_path.as_ref().unwrap())
            .await
            .unwrap();
    }
    drop(session);
    server.session_manager.remove(&id).await;
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: id.clone(),
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    let messages = flush(&mut socket).await;
    let ready = messages.iter().position(|message| matches!(message, ServerMessage::SessionReady { session_id, .. } if session_id == &id)).unwrap();
    let snapshot = messages
        .iter()
        .position(|message| matches!(message, ServerMessage::TodosUpdated { .. }))
        .expect("attach must publish canonical todos");
    assert!(
        snapshot > ready,
        "session confirmation must precede its todo snapshot"
    );
    let ServerMessage::TodosUpdated { todos } = &messages[snapshot] else {
        unreachable!()
    };
    assert_eq!(todos.len(), 2);
    assert_eq!(todos[0].content, "stored task");
    assert_eq!(todos[0].status, "blocked");
    assert_eq!(todos[1].depends_on, vec![1]);

    send(
        &mut socket,
        ClientMessage::NewSession {
            cwd: None,
            scratch: false,
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    let messages = flush(&mut socket).await;
    let ready = messages.iter().position(|message| matches!(message, ServerMessage::SessionReady { session_id, .. } if session_id != &id)).unwrap();
    let snapshot = messages
        .iter()
        .position(|message| matches!(message, ServerMessage::TodosUpdated { .. }))
        .expect("a new session must clear the previous todo snapshot");
    assert!(snapshot > ready);
    assert!(
        matches!(&messages[snapshot], ServerMessage::TodosUpdated { todos } if todos.is_empty())
    );
}

#[tokio::test]
async fn effective_policy_and_todos_survive_runtime_reload() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    flush(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::SwitchPersona {
            name: "planner".into(),
        },
    )
    .await;
    next_setting(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::SwitchModel {
            provider: "fake".into(),
            model: "chosen".into(),
        },
    )
    .await;
    next_setting(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::SetThinkingVariant {
            variant: String::new(),
        },
    )
    .await;
    next_setting(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::SetPermissionMode {
            mode: "permissive".into(),
        },
    )
    .await;
    next_setting(&mut socket).await;
    let todo_ids = {
        let agent = session.agent.lock().await;
        let mut todos = agent.todos.lock().await;
        let (first, dropped) = todos.create(vec![("read project".into(), Vec::new())]);
        assert!(dropped.iter().all(Vec::is_empty));
        let (second, dropped) = todos.create(vec![("finish task".into(), vec![first[0].id])]);
        assert!(dropped.iter().all(Vec::is_empty));
        todos
            .save(agent.todos_path.as_ref().unwrap())
            .await
            .unwrap();
        (first[0].id, second[0].id)
    };
    let saved = mew_session::SessionPolicy::read(root.path(), &id)
        .await
        .unwrap()
        .expect("effective policy must be persisted");
    assert_eq!(saved.model.as_deref(), Some("chosen"));
    assert_eq!(saved.persona.as_deref(), Some("planner"));
    assert_eq!(saved.thinking_variant, None);
    assert_eq!(saved.permission_mode, "permissive");
    // Switch the socket away before simulating eviction of the old runtime.
    send(
        &mut socket,
        ClientMessage::NewSession {
            cwd: None,
            scratch: false,
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    assert!(flush(&mut socket).await.iter().any(|message| {
        matches!(message, ServerMessage::SessionReady { session_id, .. } if session_id != &id)
    }));
    let previous = Arc::downgrade(&session);
    drop(session);
    server.session_manager.remove(&id).await;
    assert!(previous.upgrade().is_none());
    let restored = server.session_manager.attach(&id).await.unwrap();
    let agent = restored.agent.lock().await;
    assert_eq!(agent.model_id, "chosen");
    assert_eq!(agent.persona_name.as_deref(), Some("planner"));
    assert_eq!(agent.active_thinking_variant(), None);
    assert_eq!(
        agent.permission_mode(),
        mew_hooks::PermissionMode::Permissive
    );
    let mut todos = agent.todos.lock().await;
    assert!(todos.render().contains("finish task"));
    assert_eq!(todos.get(todo_ids.1).unwrap().depends_on, vec![todo_ids.0]);
    assert!(todos.complete(todo_ids.1).is_err());
    todos.complete(todo_ids.0).unwrap();
    todos.complete(todo_ids.1).unwrap();
}

#[tokio::test]
async fn failed_persona_pin_keeps_current_model_persona_and_skill_filter() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
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
    let before = session.agent.lock().await.skill_filter.read().await.clone();
    send(
        &mut socket,
        ClientMessage::SwitchPersona {
            name: "broken-pin".into(),
        },
    )
    .await;
    let messages = next_setting(&mut socket).await;
    assert!(messages.iter().any(|m| matches!(m, ServerMessage::Error { message } if message.contains("provider construction failed"))));
    assert!(!messages
        .iter()
        .any(|m| matches!(m, ServerMessage::PersonaSwitched { .. })));
    let agent = session.agent.lock().await;
    assert_eq!(agent.persona_name.as_deref(), Some("builder"));
    assert_eq!(agent.model_id, "initial");
    assert_eq!(*agent.skill_filter.read().await, before);
}

#[tokio::test]
async fn failed_policy_write_does_not_apply_permission_mode() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
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
    let policy_path = root.path().join(&session.id).join("policy.json");
    if policy_path.exists() {
        tokio::fs::remove_file(&policy_path).await.unwrap();
    }
    tokio::fs::create_dir(&policy_path).await.unwrap();
    send(
        &mut socket,
        ClientMessage::SetPermissionMode {
            mode: "permissive".into(),
        },
    )
    .await;
    let messages = next_setting(&mut socket).await;
    assert!(messages
        .iter()
        .any(|m| matches!(m, ServerMessage::Error { .. })));
    assert_eq!(
        session.agent.lock().await.permission_mode(),
        mew_hooks::PermissionMode::Standard
    );
}

#[tokio::test]
async fn malformed_policy_prevents_silent_resume_with_defaults() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    drop(session);
    server.session_manager.remove(&id).await;
    tokio::fs::write(root.path().join(&id).join("policy.json"), b"{invalid")
        .await
        .unwrap();
    assert!(matches!(
        server.session_manager.attach(&id).await,
        Err(AttachError::BuildAgent(_))
    ));
}

#[tokio::test]
async fn concurrent_model_and_mode_updates_preserve_both_effective_choices() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
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
    send(
        &mut socket,
        ClientMessage::SwitchModel {
            provider: "fake".into(),
            model: "chosen".into(),
        },
    )
    .await;
    send(
        &mut socket,
        ClientMessage::SetPermissionMode {
            mode: "permissive".into(),
        },
    )
    .await;
    next_setting(&mut socket).await;
    next_setting(&mut socket).await;
    let policy = mew_session::SessionPolicy::read(root.path(), &session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.model.as_deref(), Some("chosen"));
    assert_eq!(policy.permission_mode, "permissive");
}

#[tokio::test]
async fn thinking_changes_reach_other_attached_viewers() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let mut first = connect(&server).await;
    let mut second = connect(&server).await;
    for socket in [&mut first, &mut second] {
        send(
            socket,
            ClientMessage::AttachSession {
                session_id: session.id.clone(),
                client_kind: ClientKind::Unknown,
            },
        )
        .await;
        flush(socket).await;
    }
    send(
        &mut first,
        ClientMessage::SetThinkingVariant {
            variant: String::new(),
        },
    )
    .await;
    next_setting(&mut first).await;
    let messages = next_setting(&mut second).await;
    assert!(messages
        .iter()
        .any(|m| matches!(m, ServerMessage::ThinkingVariantChanged { variant: None })));
}

#[tokio::test]
async fn failed_model_policy_write_restores_owned_and_shared_prompt_state() {
    let root = tempfile::tempdir().unwrap();
    let mut server = policy_server(root.path().to_path_buf());
    server = server.with_model_management(
        Arc::new(|agent, provider, model| {
            agent.set_model_info(model, provider);
            let persona = agent
                .personas
                .iter()
                .find(|p| p.name == "broken-pin")
                .unwrap()
                .clone();
            agent.apply_persona(&persona);
            Ok((provider.into(), model.into()))
        }),
        Arc::new(|| (Vec::new(), None)),
    );
    let session = server.session_manager.create(None).await.unwrap();
    let before_filter = session.agent.lock().await.skill_filter.read().await.clone();
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
    let path = root.path().join(&session.id).join("policy.json");
    tokio::fs::remove_file(&path).await.unwrap();
    tokio::fs::create_dir(&path).await.unwrap();
    send(
        &mut socket,
        ClientMessage::SwitchModel {
            provider: "fake".into(),
            model: "chosen".into(),
        },
    )
    .await;
    let messages = next_setting(&mut socket).await;
    assert!(messages
        .iter()
        .any(|m| matches!(m, ServerMessage::Error { .. })));
    assert!(!messages
        .iter()
        .any(|m| matches!(m, ServerMessage::ModelSwitched { .. })));
    let agent = session.agent.lock().await;
    assert_eq!(agent.model_id, "initial");
    assert_eq!(agent.persona_name.as_deref(), Some("builder"));
    assert_eq!(*agent.skill_filter.read().await, before_filter);
    assert_eq!(session.model.lock().await.as_deref(), Some("initial"));
}

#[tokio::test]
async fn invalid_persisted_todos_fail_resume_visibly() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    drop(session);
    server.session_manager.remove(&id).await;
    tokio::fs::write(root.path().join(&id).join("todos.json"), b"{invalid")
        .await
        .unwrap();
    assert!(matches!(
        server.session_manager.attach(&id).await,
        Err(AttachError::BuildAgent(_))
    ));
}

#[tokio::test]
async fn idle_session_list_reports_saved_effective_model_and_provider() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    {
        let agent = session.agent.lock().await;
        let message = mew_message::Message {
            id: ulid::Ulid::new(),
            session_id: agent.session_id,
            role: mew_message::Role::User,
            parts: Vec::new(),
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        agent
            .session
            .as_ref()
            .unwrap()
            .lock()
            .await
            .write_message(&message)
            .await
            .unwrap();
    }
    let mut policy = mew_session::SessionPolicy::read(root.path(), &id)
        .await
        .unwrap()
        .unwrap();
    policy.model = Some("chosen".into());
    policy.provider = Some("chosen-provider".into());
    policy.write(root.path(), &id).await.unwrap();
    server.session_manager.remove(&id).await;
    let sessions = server.session_manager.list().await;
    let listed = sessions.iter().find(|s| s.session_id == id).unwrap();
    assert_eq!(listed.model.as_deref(), Some("chosen"));
    assert_eq!(listed.provider.as_deref(), Some("chosen-provider"));
}

#[tokio::test]
async fn explicit_default_persona_clears_runtime_and_survives_reload() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
    let session = server.session_manager.create(None).await.unwrap();
    let id = session.id.clone();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    flush(&mut socket).await;
    send(
        &mut socket,
        ClientMessage::SwitchPersona {
            name: "default".into(),
        },
    )
    .await;
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Message::Text(text) = frame else { continue };
        let message: ServerMessage = mew_protocol::decode_json(&text).unwrap();
        match message {
            ServerMessage::PersonaList { personas } => {
                assert!(!personas.iter().any(|p| p.active));
                break;
            }
            ServerMessage::Error { message } => panic!("persona clear failed: {message}"),
            _ => {}
        }
    }
    assert_eq!(session.agent.lock().await.persona_name, None);
    // Switch the socket away before simulating eviction of the old runtime.
    send(
        &mut socket,
        ClientMessage::NewSession {
            cwd: None,
            scratch: false,
            client_kind: ClientKind::Tui,
        },
    )
    .await;
    assert!(flush(&mut socket).await.iter().any(|message| {
        matches!(message, ServerMessage::SessionReady { session_id, .. } if session_id != &id)
    }));
    let previous = Arc::downgrade(&session);
    drop(session);
    server.session_manager.remove(&id).await;
    assert!(previous.upgrade().is_none());
    let restored = server.session_manager.attach(&id).await.unwrap();
    assert_eq!(restored.agent.lock().await.persona_name, None);
}

#[tokio::test]
async fn bare_persona_pin_inherits_the_current_provider() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().to_path_buf());
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
    send(
        &mut socket,
        ClientMessage::SwitchPersona {
            name: "inherited-pin".into(),
        },
    )
    .await;
    let messages = next_setting(&mut socket).await;
    assert!(!messages
        .iter()
        .any(|m| matches!(m, ServerMessage::Error { .. })));
    let messages = next_setting(&mut socket).await;
    assert!(messages
        .iter()
        .any(|m| matches!(m, ServerMessage::PersonaSwitched { name } if name == "inherited-pin")));
    let agent = session.agent.lock().await;
    assert_eq!(agent.model_id, "chosen");
    assert_eq!(agent.provider_id, "fake");
}

#[tokio::test]
async fn new_chat_defaults_survive_restart_without_changing_existing_chats() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let original = server.session_manager.create(None).await.unwrap();
    let mut defaults = original.policy(&*original.agent.lock().await).await;
    defaults.model = Some("chosen".into());
    defaults.persona = None;
    defaults.thinking_variant = None;
    defaults.permission_mode = "permissive".into();

    server
        .session_manager
        .set_session_defaults(Some(defaults.clone()))
        .await
        .unwrap();
    let restarted = policy_server(root.path().into());
    assert_eq!(
        restarted
            .session_manager
            .read_session_defaults()
            .await
            .unwrap(),
        Some(defaults.clone())
    );
    let created = restarted.session_manager.create(None).await.unwrap();
    assert_eq!(created.policy(&*created.agent.lock().await).await, defaults);
    assert_eq!(original.agent.lock().await.model_id, "initial");
    let id = original.id.clone();
    drop(original);
    drop(server);
    let restored = restarted.session_manager.attach(&id).await.unwrap();
    assert_eq!(restored.agent.lock().await.model_id, "initial");

    restarted
        .session_manager
        .set_session_defaults(None)
        .await
        .unwrap();
    assert_eq!(
        restarted
            .session_manager
            .read_session_defaults()
            .await
            .unwrap(),
        None
    );
    let reset = restarted.session_manager.create(None).await.unwrap();
    assert_eq!(reset.agent.lock().await.model_id, "initial");
    assert_eq!(created.agent.lock().await.model_id, "chosen");
}

#[tokio::test]
async fn malformed_defaults_fail_new_chats_visibly_but_can_be_reset() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    tokio::fs::write(root.path().join("new-chat-defaults.json"), b"{invalid")
        .await
        .unwrap();
    assert!(server
        .session_manager
        .read_session_defaults()
        .await
        .is_err());
    assert!(server.session_manager.create(None).await.is_err());
    server
        .session_manager
        .set_session_defaults(None)
        .await
        .unwrap();
    assert!(server.session_manager.create(None).await.is_ok());
}

#[tokio::test]
async fn failed_defaults_write_preserves_the_confirmed_defaults() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let session = server.session_manager.create(None).await.unwrap();
    let policy = session.policy(&*session.agent.lock().await).await;
    server
        .session_manager
        .set_session_defaults(Some(policy.clone()))
        .await
        .unwrap();
    tokio::fs::create_dir(root.path().join("new-chat-defaults.json.tmp"))
        .await
        .unwrap();
    let mut updated = policy.clone();
    updated.permission_mode = "permissive".into();
    assert!(server
        .session_manager
        .set_session_defaults(Some(updated))
        .await
        .is_err());
    assert_eq!(
        server
            .session_manager
            .read_session_defaults()
            .await
            .unwrap(),
        Some(policy)
    );
}

#[tokio::test]
async fn daemon_defaults_commands_capture_only_the_attached_chat_and_confirm_persistence() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let session = server.session_manager.create(None).await.unwrap();
    let mut socket = connect(&server).await;
    send(
        &mut socket,
        ClientMessage::ReadDaemonSettings {
            request_id: "read".into(),
        },
    )
    .await;
    let replies = flush(&mut socket).await;
    assert!(
        matches!(&replies[..], [ServerMessage::DaemonSettings { request_id: Some(id), defaults: None, devices: None, .. }] if id == "read")
    );
    send(
        &mut socket,
        ClientMessage::SaveSessionDefaults {
            session_id: session.id.clone(),
            request_id: "wrong".into(),
        },
    )
    .await;
    assert!(
        matches!(&flush(&mut socket).await[..], [ServerMessage::DaemonSettingsFailed { request_id, .. }] if request_id == "wrong")
    );
    assert_eq!(
        server
            .session_manager
            .read_session_defaults()
            .await
            .unwrap(),
        None
    );
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
        ClientMessage::SaveSessionDefaults {
            session_id: session.id.clone(),
            request_id: "save".into(),
        },
    )
    .await;
    let replies = flush(&mut socket).await;
    assert!(replies.iter().any(|reply| matches!(reply, ServerMessage::DaemonSettings { request_id: Some(id), defaults: Some(defaults), .. } if id == "save" && defaults.model.as_deref() == Some("initial"))));
    assert!(server
        .session_manager
        .read_session_defaults()
        .await
        .unwrap()
        .is_some());
    send(
        &mut socket,
        ClientMessage::ResetSessionDefaults {
            request_id: "reset".into(),
        },
    )
    .await;
    let replies = flush(&mut socket).await;
    assert!(replies.iter().any(|reply| matches!(reply, ServerMessage::DaemonSettings { request_id: Some(id), defaults: None, .. } if id == "reset")));
}

async fn paired_connection(
    server: &DaemonServer,
    store: Arc<remote::RemoteAccessStore>,
    peer: &str,
    scope: RemoteScope,
) -> tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream> {
    paired_connection_task(server, store, peer, scope, 256 * 1024)
        .await
        .0
}

async fn paired_connection_task(
    server: &DaemonServer,
    store: Arc<remote::RemoteAccessStore>,
    peer: &str,
    scope: RemoteScope,
    capacity: usize,
) -> (
    tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
    tokio::task::JoinHandle<()>,
) {
    let token = store.create_pairing(scope, remote::unix_now(), 60).unwrap();
    let peer = peer.to_string();
    let node_id = peer.clone();
    let authorizer: RemoteAuthorizer = Arc::new(move |token, name| {
        store.authorize_peer(token, &node_id, name, remote::unix_now())
    });
    let (client, stream) = tokio::io::duplex(capacity);
    let manager = server.session_manager.clone();
    let groups = server.groups_store.clone();
    let summary = server.auto_summary_enabled.clone();
    let task = tokio::spawn(async move {
        handle_connection_with_authorizer(
            stream,
            manager,
            groups,
            None,
            summary,
            Some(scope),
            Some(authorizer),
            false,
            Some(peer),
        )
        .await
        .unwrap();
    });
    let mut socket = tokio_tungstenite::client_async("ws://localhost/", client)
        .await
        .unwrap()
        .0;
    send(
        &mut socket,
        ClientMessage::RemoteHello {
            token: Some(token),
            device_name: "Phone".into(),
        },
    )
    .await;
    assert!(flush(&mut socket)
        .await
        .iter()
        .any(|m| matches!(m, ServerMessage::RemoteReady { .. })));
    (socket, task)
}

#[tokio::test]
async fn a_phone_over_the_remote_transport_keeps_its_own_client_kind() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let session = server.session_manager.create(None).await.unwrap();
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;

    send(
        &mut phone,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Mobile,
        },
    )
    .await;
    let replies = flush(&mut phone).await;

    assert!(
        replies
            .iter()
            .any(|m| matches!(m, ServerMessage::SessionReady { .. })),
        "a phone must attach over the remote transport, got {replies:?}"
    );
    assert!(
        !replies
            .iter()
            .any(|m| matches!(m, ServerMessage::Error { .. })),
        "the client kind must not be policed by the transport, got {replies:?}"
    );
    let presence = session.client_presence_except(u64::MAX).await;
    assert_eq!(presence.len(), 1);
    assert_eq!(presence[0].1, ClientKind::Mobile);
}

#[tokio::test]
async fn only_full_control_connections_can_manage_daemon_defaults_or_devices() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    for (peer, scope) in [
        ("reader", RemoteScope::Observe),
        ("collaborator", RemoteScope::Collaborate),
    ] {
        let mut socket = paired_connection(&server, store.clone(), peer, scope).await;
        for message in [
            ClientMessage::ReadDaemonSettings {
                request_id: "denied".into(),
            },
            ClientMessage::SaveSessionDefaults {
                session_id: "unused".into(),
                request_id: "denied".into(),
            },
            ClientMessage::ResetSessionDefaults {
                request_id: "denied".into(),
            },
            ClientMessage::RevokeDevice {
                device_id: "unused".into(),
                request_id: "denied".into(),
            },
            ClientMessage::SetWebSearchSettings {
                enabled: Some(false),
                provider: None,
                request_id: "denied".into(),
            },
        ] {
            send(&mut socket, message).await;
            let replies = flush(&mut socket).await;
            assert!(
                matches!(&replies[..], [ServerMessage::DaemonSettingsFailed { request_id, message }] if request_id == "denied" && message.contains("does not allow"))
            );
        }
    }
    let mut controller =
        paired_connection(&server, store, "controller", RemoteScope::Control).await;
    send(
        &mut controller,
        ClientMessage::ReadDaemonSettings {
            request_id: "allowed".into(),
        },
    )
    .await;
    assert!(flush(&mut controller).await.iter().any(|reply| matches!(reply, ServerMessage::DaemonSettings { request_id: Some(id), devices: Some(devices), .. } if id == "allowed" && devices.len() == 3)));
}

#[tokio::test]
async fn revoking_a_device_closes_its_idle_socket_and_rejects_reauthentication() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;
    send(
        &mut phone,
        ClientMessage::ReadDaemonSettings {
            request_id: "subscribe".into(),
        },
    )
    .await;
    flush(&mut phone).await;
    let device_id = store.snapshot().devices[0].id.clone();
    let mut admin = connect(&server).await;
    send(
        &mut admin,
        ClientMessage::RevokeDevice {
            device_id,
            request_id: "revoke".into(),
        },
    )
    .await;
    assert!(flush(&mut admin).await.iter().any(|reply| matches!(reply, ServerMessage::DaemonSettings { request_id: Some(id), devices: Some(devices), .. } if id == "revoke" && devices.is_empty())));
    let closed = tokio::time::timeout(std::time::Duration::from_secs(3), phone.next())
        .await
        .unwrap();
    assert!(matches!(closed, None | Some(Ok(Message::Close(_)))));
    assert_eq!(
        store.authorize_device("phone", remote::unix_now()).unwrap(),
        None
    );
}

#[tokio::test]
async fn a_revoked_device_cannot_reset_defaults_before_the_idle_check_runs() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let session = server.session_manager.create(None).await.unwrap();
    let policy = session.policy(&*session.agent.lock().await).await;
    server
        .session_manager
        .set_session_defaults(Some(policy.clone()))
        .await
        .unwrap();
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;
    let id = store.snapshot().devices[0].id.clone();
    store.revoke(&id, remote::unix_now()).unwrap();
    let _ = phone
        .send(Message::Text(
            mew_protocol::encode_json(&ClientMessage::ResetSessionDefaults {
                request_id: "revoked".into(),
            })
            .unwrap(),
        ))
        .await;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(3), phone.next())
        .await
        .unwrap();
    assert!(matches!(closed, None | Some(Ok(Message::Close(_)))));
    assert_eq!(
        server
            .session_manager
            .read_session_defaults()
            .await
            .unwrap(),
        Some(policy)
    );
}

#[tokio::test]
async fn self_revocation_delivers_confirmation_before_closing_the_socket() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;
    let id = store.snapshot().devices[0].id.clone();
    send(
        &mut phone,
        ClientMessage::RevokeDevice {
            device_id: id,
            request_id: "self".into(),
        },
    )
    .await;
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), phone.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = message else {
        panic!("revocation must be acknowledged before close");
    };
    assert!(
        matches!(mew_protocol::decode_json::<ServerMessage>(&text).unwrap(), ServerMessage::DaemonSettings { request_id: Some(id), devices: Some(devices), .. } if id == "self" && devices.is_empty())
    );
}

#[tokio::test]
async fn revocation_cancels_a_save_waiting_for_the_agent_before_it_can_change_defaults() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let session = server.session_manager.create(None).await.unwrap();
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;
    send(
        &mut phone,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Remote,
        },
    )
    .await;
    flush(&mut phone).await;
    let agent = session.agent.lock().await;
    send(
        &mut phone,
        ClientMessage::SaveSessionDefaults {
            session_id: session.id.clone(),
            request_id: "blocked".into(),
        },
    )
    .await;
    server
        .session_manager
        .settings_operation_started
        .notified()
        .await;
    let id = store.snapshot().devices[0].id.clone();
    store.revoke(&id, remote::unix_now()).unwrap();
    let closed = tokio::time::timeout(std::time::Duration::from_secs(2), phone.next()).await;
    drop(agent);
    assert!(matches!(closed, Ok(None | Some(Ok(Message::Close(_))))));
    assert_eq!(
        server
            .session_manager
            .read_session_defaults()
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_scope_downgrade_during_an_agent_wait_cannot_save_defaults_or_disclose_devices() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let session = server.session_manager.create(None).await.unwrap();
    let path = root.path().join("remote.json");
    let store = Arc::new(remote::RemoteAccessStore::new(path.clone()));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;
    send(
        &mut phone,
        ClientMessage::AttachSession {
            session_id: session.id.clone(),
            client_kind: ClientKind::Remote,
        },
    )
    .await;
    flush(&mut phone).await;
    let agent = session.agent.lock().await;
    send(
        &mut phone,
        ClientMessage::SaveSessionDefaults {
            session_id: session.id.clone(),
            request_id: "downgraded".into(),
        },
    )
    .await;
    server
        .session_manager
        .settings_operation_started
        .notified()
        .await;
    let mut updated = store.read_state().unwrap();
    updated.devices[0].scope = RemoteScope::Observe;
    std::fs::write(path, serde_json::to_vec(&updated).unwrap()).unwrap();
    drop(agent);
    let responses = flush(&mut phone).await;
    assert!(responses.iter().any(|response| matches!(response, ServerMessage::DaemonSettingsFailed { request_id, .. } if request_id == "downgraded")));
    assert!(!responses
        .iter()
        .any(|response| matches!(response, ServerMessage::DaemonSettings { .. })));
    assert_eq!(
        server
            .session_manager
            .read_session_defaults()
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn revocation_bounds_a_writer_blocked_by_a_nonreading_peer() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let (mut phone, task) =
        paired_connection_task(&server, store.clone(), "phone", RemoteScope::Control, 64).await;
    send(
        &mut phone,
        ClientMessage::ReadDaemonSettings {
            request_id: "blocked-writer".into(),
        },
    )
    .await;
    server
        .session_manager
        .settings_operation_started
        .notified()
        .await;
    let id = store.snapshot().devices[0].id.clone();
    store.revoke(&id, remote::unix_now()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await
        .expect("a nonreading revoked peer must not retain its writer task")
        .unwrap();
}

#[tokio::test]
async fn a_scope_downgrade_during_a_snapshot_read_cannot_disclose_device_metadata() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let path = root.path().join("remote.json");
    let store = Arc::new(remote::RemoteAccessStore::new(path.clone()));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;
    let defaults = server.session_manager.defaults_lock.lock().await;
    send(
        &mut phone,
        ClientMessage::ReadDaemonSettings {
            request_id: "reading".into(),
        },
    )
    .await;
    server
        .session_manager
        .settings_operation_started
        .notified()
        .await;
    let mut updated = store.read_state().unwrap();
    updated.devices[0].scope = RemoteScope::Observe;
    std::fs::write(path, serde_json::to_vec(&updated).unwrap()).unwrap();
    drop(defaults);
    let replies = flush(&mut phone).await;
    assert!(
        matches!(&replies[..], [ServerMessage::DaemonSettingsFailed { request_id, .. }] if request_id == "reading")
    );
}

#[tokio::test]
async fn self_revocation_receipt_survives_a_monitor_tick_between_commit_and_completion() {
    let root = tempfile::tempdir().unwrap();
    let server = policy_server(root.path().into());
    let store = Arc::new(remote::RemoteAccessStore::new(
        root.path().join("remote.json"),
    ));
    store
        .set_hosting(true, remote::RemoteMode::Daemon, None)
        .unwrap();
    server
        .session_manager
        .remote_devices
        .set(remote::RemoteDeviceManager::Scoped(store.clone()))
        .unwrap();
    let mut phone = paired_connection(&server, store.clone(), "phone", RemoteScope::Control).await;
    let committed = Arc::new(tokio::sync::Notify::new());
    let notify = committed.clone();
    let (release, gate) = std::sync::mpsc::channel();
    let gate = std::sync::Mutex::new(gate);
    *server
        .session_manager
        .revocation_commit_hook
        .lock()
        .unwrap() = Some(Arc::new(move || {
        notify.notify_one();
        let _ = gate
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(4));
    }));
    let id = store.snapshot().devices[0].id.clone();
    send(
        &mut phone,
        ClientMessage::RevokeDevice {
            device_id: id,
            request_id: "self".into(),
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(3), committed.notified())
        .await
        .unwrap();
    // Keep the committed worker paused across at least one authorization-monitor tick.
    let premature =
        tokio::time::timeout(std::time::Duration::from_millis(1500), phone.next()).await;
    let _ = release.send(());
    assert!(
        premature.is_err(),
        "the monitor must wait for the committed receipt before closing"
    );
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), phone.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = frame else {
        panic!("expected revocation receipt");
    };
    assert!(
        matches!(mew_protocol::decode_json::<ServerMessage>(&text).unwrap(), ServerMessage::DaemonSettings { request_id: Some(id), devices: Some(devices), .. } if id == "self" && devices.is_empty())
    );
}
