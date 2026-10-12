use super::*;
use mew_hooks::PermissionMode;
use mew_session::SessionPolicy;

struct Credential(String);

impl Drop for Credential {
    fn drop(&mut self) {
        std::env::remove_var(&self.0);
    }
}

fn fixture() -> (mew_config::Config, mew_catalog::Catalog, Credential) {
    let reference = format!("POLICY_TEST_{}", ulid::Ulid::new());
    let key = format!("MEW_CRED_{reference}");
    std::env::set_var(&key, "local-fixture-token");
    let provider = serde_json::from_value(serde_json::json!({
        "shape":"openai", "base_url":"http://127.0.0.1:1", "credential_ref":reference,
    }))
    .unwrap();
    let cfg = mew_config::Config {
        default_model: "first".into(),
        default_persona: "builder".into(),
        providers: std::collections::HashMap::from([("fixture".into(), provider)]),
        ..Default::default()
    };
    let mut catalog = mew_catalog::Catalog::empty();
    for (id, vision, context, max, cost, retention) in [
        ("first", true, 120_000, 30_000, 1.0, Some(3600)),
        ("second", false, 60_000, 4096, 9.0, None),
    ] {
        let model: mew_catalog::Model = serde_json::from_value(serde_json::json!({
            "id":id,"provider":"fixture","vision":vision,"context_window":context,
            "max_output":max,"pricing":{"input":cost,"output":cost+1.0},
            "prompt_cache_retention_secs":retention,
            "thinking_variants":[{"name":"high","params":{"reasoning_effort":"high"}},
                                 {"name":"low","params":{"reasoning_effort":"low"}}],
        }))
        .unwrap();
        catalog.models.insert(id.into(), model.clone());
        catalog
            .providers
            .entry("fixture".into())
            .or_default()
            .insert(id.into(), model);
    }
    (cfg, catalog, Credential(key))
}

fn saved() -> SessionPolicy {
    SessionPolicy {
        provider: Some("fixture".into()),
        model: Some("second".into()),
        persona: Some("planner".into()),
        thinking_variant: Some("low".into()),
        permission_mode: "permissive".into(),
    }
}

async fn params(root: &Path, policy: Option<SessionPolicy>) -> mew_daemon::AgentBuildParams {
    let id = format!("sess_{}", ulid::Ulid::new());
    mew_daemon::AgentBuildParams {
        writer: mew_session::Writer::open_at(&root.join("sessions"), &id)
            .await
            .unwrap(),
        session_id: id,
        cwd: Some(root.to_path_buf()),
        policy,
    }
}

#[tokio::test]
async fn real_builder_restores_effective_policy_and_explicit_thinking_off() {
    let directory = tempfile::tempdir().unwrap();
    let (cfg, cat, _credential) = fixture();
    let server = build_daemon_server_with_config(
        cfg,
        Some(cat),
        false,
        "fixture",
        Some("first".into()),
        false,
        PermissionMode::Standard,
    )
    .unwrap();
    for variant in [Some("low"), None] {
        let mut policy = saved();
        policy.thinking_variant = variant.map(str::to_owned);
        let (agent, model, provider) =
            (server.builder)(params(directory.path(), Some(policy)).await).unwrap();
        assert_eq!(
            (model.as_deref(), provider.as_deref()),
            (Some("second"), Some("fixture"))
        );
        assert_eq!(
            (agent.model_id.as_str(), agent.provider_id.as_str()),
            ("second", "fixture")
        );
        assert_eq!(agent.persona_name.as_deref(), Some("planner"));
        assert_eq!(agent.permission_mode(), PermissionMode::Permissive);
        assert_eq!(agent.active_thinking_variant(), variant);
        assert_eq!(agent.reasoning.is_some(), variant.is_some());
        assert_eq!(agent.context_window, 60_000);
        assert_eq!(agent.default_max_output_tokens, 4096);
        assert_eq!(agent.input_price, 9.0);
        assert!(!agent.supports_vision);
    }
}

#[tokio::test]
async fn saved_model_wins_a_changed_persona_pin_but_new_sessions_honor_the_pin() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".mew/personas/pinned");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("PERSONA.md"), "---\nname: pinned\ndescription: pinned project persona\nmew:\n  model: fixture/second\n---\nproject body\n").unwrap();
    let (mut cfg, cat, _credential) = fixture();
    cfg.default_persona = "pinned".into();
    let server = build_daemon_server_with_config(
        cfg,
        Some(cat),
        false,
        "fixture",
        Some("first".into()),
        false,
        PermissionMode::Standard,
    )
    .unwrap();
    let (agent, model, _) = (server.builder)(params(directory.path(), None).await).unwrap();
    assert_eq!(model.as_deref(), Some("second"));
    assert_eq!(agent.model_id, "second");
    let policy = SessionPolicy {
        model: Some("first".into()),
        persona: Some("pinned".into()),
        ..saved()
    };
    let (agent, model, _) = (server.builder)(params(directory.path(), Some(policy)).await).unwrap();
    assert_eq!(model.as_deref(), Some("first"));
    assert_eq!(agent.persona_name.as_deref(), Some("pinned"));
}

#[tokio::test]
async fn unavailable_saved_policy_is_reported_without_falling_back() {
    let directory = tempfile::tempdir().unwrap();
    let (cfg, cat, _credential) = fixture();
    let server = build_daemon_server_with_config(
        cfg,
        Some(cat),
        false,
        "fixture",
        Some("first".into()),
        false,
        PermissionMode::Standard,
    )
    .unwrap();
    for (field, value) in [
        ("provider", "missing-provider"),
        ("model", ""),
        ("persona", "missing-persona"),
        ("thinking", "not-a-variant"),
        ("mode", "not-a-mode"),
    ] {
        let mut policy = saved();
        match field {
            "provider" => policy.provider = Some(value.into()),
            "model" => policy.model = Some(value.into()),
            "persona" => policy.persona = Some(value.into()),
            "thinking" => policy.thinking_variant = Some(value.into()),
            _ => policy.permission_mode = value.into(),
        }
        let result = (server.builder)(params(directory.path(), Some(policy)).await);
        assert!(result.is_err(), "invalid saved {field} must be visible");
        let error = format!("{:#}", result.err().unwrap());
        let expected = if field == "model" {
            "must not be empty"
        } else {
            value
        };
        assert!(
            error.contains(expected),
            "saved {field} error must identify the unavailable selection: {error}"
        );
    }
}

#[tokio::test]
async fn model_switch_updates_all_runtime_metadata_and_thinking_off_clears_both_fields() {
    let directory = tempfile::tempdir().unwrap();
    let (cfg, cat, _credential) = fixture();
    let server = build_daemon_server_with_config(
        cfg,
        Some(cat),
        false,
        "fixture",
        Some("first".into()),
        false,
        PermissionMode::Standard,
    )
    .unwrap();
    let (mut agent, _, _) = (server.builder)(params(directory.path(), None).await).unwrap();
    (server.model_switcher.as_ref().unwrap())(&mut agent, "fixture", "second").unwrap();
    assert_eq!(
        (agent.model_id.as_str(), agent.provider_id.as_str()),
        ("second", "fixture")
    );
    assert_eq!(agent.context_window, 60_000);
    assert_eq!(agent.default_max_output_tokens, 4096);
    assert_eq!((agent.input_price, agent.output_price), (9.0, 10.0));
    assert_eq!(
        agent.prompt_cache_retention(),
        mew_agent::PromptCacheRetention::Unknown
    );
    assert!(!agent.supports_vision);
    let setter = server.thinking_setter.as_ref().unwrap();
    setter(&mut agent, "second", "low").unwrap();
    setter(&mut agent, "second", "").unwrap();
    assert!(agent.reasoning.is_none());
    assert_eq!(agent.active_thinking_variant(), None);
    (server.model_switcher.as_ref().unwrap())(&mut agent, "fixture", "uncatalogued-custom")
        .unwrap();
    // Unknown catalog IDs use the catalog's conservative context fallback.
    assert_eq!(
        (agent.context_window, agent.default_max_output_tokens),
        (128_000, 0)
    );
    assert_eq!(
        (
            agent.input_price,
            agent.output_price,
            agent.cache_read_price,
            agent.cache_write_price,
            agent.reasoning_price
        ),
        (0.0, 0.0, 0.0, 0.0, 0.0)
    );
    assert_eq!(
        agent.prompt_cache_retention(),
        mew_agent::PromptCacheRetention::Unknown
    );
}

#[tokio::test]
async fn saved_thinking_off_preserves_a_models_explicit_disable_parameters() {
    let directory = tempfile::tempdir().unwrap();
    let (cfg, cat, _credential) = fixture();
    let server = build_daemon_server_with_config(
        cfg,
        Some(cat),
        false,
        "fixture",
        None,
        false,
        PermissionMode::Standard,
    )
    .unwrap();
    let policy = SessionPolicy {
        model: Some("qwen3.8-max".into()),
        thinking_variant: None,
        persona: None,
        ..saved()
    };
    let (mut agent, _, _) = (server.builder)(params(directory.path(), Some(policy)).await).unwrap();
    assert_eq!(agent.persona_name, None);
    assert_eq!(agent.active_thinking_variant(), None);
    assert_eq!(
        agent.reasoning.as_ref().unwrap().params["enable_thinking"],
        false
    );
    let setter = server.thinking_setter.as_ref().unwrap();
    setter(&mut agent, "qwen3.8-max", "xhigh").unwrap();
    assert!(setter(&mut agent, "qwen3.8-max", "invalid").is_err());
    assert_eq!(agent.active_thinking_variant(), Some("xhigh"));
    setter(&mut agent, "qwen3.8-max", "off").unwrap();
    assert_eq!(agent.active_thinking_variant(), None);
    assert_eq!(
        agent.reasoning.as_ref().unwrap().params["enable_thinking"],
        false
    );
}

#[tokio::test]
async fn fake_builder_and_manager_restore_policy_and_todos_from_actual_storage() {
    let directory = tempfile::tempdir().unwrap();
    let cfg = mew_config::Config::default();
    let configured = build_daemon_server_with_config(
        cfg,
        None,
        true,
        "fake",
        Some("fake".into()),
        false,
        PermissionMode::Standard,
    )
    .unwrap();
    let server = mew_daemon::DaemonServer::with_session_dir(
        configured.builder,
        directory.path().join("sessions"),
    );
    let session = server
        .session_manager
        .create(Some(directory.path().to_path_buf()))
        .await
        .unwrap();
    let id = session.id.clone();
    let path = directory
        .path()
        .join("sessions")
        .join(&id)
        .join("todos.json");
    {
        let agent = session.agent.lock().await;
        assert_eq!(agent.todos_path.as_ref(), Some(&path));
        let mut todos = agent.todos.lock().await;
        todos.create(vec![("restore this task".into(), vec![])]);
        todos.save(&path).await.unwrap();
    }
    let policy = SessionPolicy {
        provider: Some("fake".into()),
        model: Some("fake".into()),
        thinking_variant: None,
        ..saved()
    };
    policy
        .write(&directory.path().join("sessions"), &id)
        .await
        .unwrap();
    server.session_manager.remove(&id).await;
    drop(session);
    let restored = server.session_manager.attach(&id).await.unwrap();
    let agent = restored.agent.lock().await;
    assert_eq!(agent.persona_name.as_deref(), Some("planner"));
    assert_eq!(agent.permission_mode(), PermissionMode::Permissive);
    assert_eq!(
        agent.todos.lock().await.items[0].content,
        "restore this task"
    );
}
