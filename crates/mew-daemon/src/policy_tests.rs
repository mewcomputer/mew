use super::*;
use crate::request_tests::{connect, flush, send};

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
            Arc::new(Vec::new),
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
    server.session_manager.remove(&id).await;
    let restored = server.session_manager.attach(&id).await.unwrap();
    assert!(!Arc::ptr_eq(&session, &restored));
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
        Arc::new(Vec::new),
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
    server.session_manager.remove(&id).await;
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
