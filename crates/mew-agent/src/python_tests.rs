use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::python::{PythonOptions, PythonSession, PythonWorker, PythonWorkerCache};

fn idle_agent(timeout: Duration) -> crate::Agent {
    session_agent(PythonSession::with_idle_timeout(timeout))
}

fn session_agent(session: PythonSession) -> crate::Agent {
    let mut agent = crate::Agent::new(
        Arc::new(mew_provider_fake::FakeProvider::new(vec![])),
        Arc::new(mew_hooks::NopDispatcher),
        None,
        vec![],
        None,
    );
    agent.code_mode.isolated = false;
    agent.set_python_session(session);
    agent
}

#[tokio::test]
async fn python_idle_cache_evicts_the_least_recently_used_worker() {
    let cache = PythonWorkerCache::new(3);
    let first = session_agent(cache.session());
    let second = session_agent(cache.session());
    let third = session_agent(cache.session());
    let fourth = session_agent(cache.session());
    for (agent, value) in [(&first, 1), (&second, 2), (&third, 3)] {
        execute(agent, &format!("saved={value}")).await.unwrap();
    }
    assert_eq!(execute(&first, "text(saved)").await.unwrap().output, "1\n");
    execute(&fourth, "saved=4").await.unwrap();
    let error = execute(&second, "text(saved)")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("capacity"), "{error}");
    assert!(error.contains("state") && error.contains("lost"), "{error}");
    for (agent, value) in [(&first, "1\n"), (&third, "3\n"), (&fourth, "4\n")] {
        assert_eq!(execute(agent, "text(saved)").await.unwrap().output, value);
    }
    assert_eq!(
        execute(&second, "text('saved' in globals())")
            .await
            .unwrap()
            .output,
        "false\n"
    );
}

#[tokio::test]
async fn zero_idle_capacity_stops_worker_after_each_call() {
    let cache = PythonWorkerCache::new(0);
    let agent = session_agent(cache.session());
    assert_eq!(
        execute(&agent, "saved=42\ntext(saved)")
            .await
            .unwrap()
            .output,
        "42\n"
    );
    let error = execute(&agent, "text(saved)")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("capacity") && error.contains("lost"),
        "{error}"
    );
    assert_eq!(
        execute(&agent, "text('saved' in globals())")
            .await
            .unwrap()
            .output,
        "false\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn idle_capacity_eviction_terminates_the_worker_process() {
    let cache = PythonWorkerCache::new(1);
    let first = session_agent(cache.session());
    let second = session_agent(cache.session());
    let pid = execute(&first, "import os\ntext(os.getpid())")
        .await
        .unwrap()
        .output
        .trim()
        .parse()
        .unwrap();
    execute(&second, "text('second')").await.unwrap();
    wait_for_process_exit(pid).await;
    assert!(execute(&first, "text('unreachable')")
        .await
        .unwrap_err()
        .to_string()
        .contains("capacity"));
}

#[tokio::test]
async fn repeated_idle_eviction_stops_each_old_reaper() {
    let cache = PythonWorkerCache::new(1);
    let mut agents = Vec::new();
    let mut previous: Option<tokio::task::AbortHandle> = None;
    for _ in 0..8 {
        let session = cache.session();
        let agent = session_agent(session.clone());
        execute(&agent, "text('ready')").await.unwrap();
        if let Some(previous) = previous.take() {
            tokio::time::timeout(Duration::from_secs(2), async {
                while !previous.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("evicted worker left its idle timer running");
        }
        previous = session.idle_reaper().await;
        assert!(previous.is_some());
        agents.push(agent);
    }
    drop(agents);
    let last = previous.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !last.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the session left its idle timer running");
}

#[tokio::test]
async fn idle_capacity_does_not_evict_a_worker_awaiting_approval() {
    let dir = tempfile::tempdir().unwrap();
    let cache = PythonWorkerCache::new(1);
    let mut first = crate::Agent::new(
        Arc::new(mew_provider_fake::FakeProvider::new(vec![])),
        Arc::new(mew_hooks::NopDispatcher),
        None,
        vec![Arc::new(mew_tools::tools::write::Write)],
        None,
    );
    first.code_mode.isolated = false;
    first.cwd = dir.path().to_path_buf();
    first.workspace_roots = vec![dir.path().to_path_buf()];
    first.set_python_session(cache.session());
    execute(&first, "saved=42").await.unwrap();
    let first = Arc::new(first);
    let (events, mut receiver) = tokio::sync::mpsc::channel(32);
    let running = first.clone();
    let call = tokio::spawn(async move {
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "test".into(),
            Default::default(),
            tokio::sync::mpsc::channel(1).0,
        );
        running.execute_python(&ctx, json!({"source":"tools.write({'path':'approved.txt','content':'ok'})\ntext(saved)"}), &events).await
    });
    let approval = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(crate::AgentEvent::PermissionRequest { tx, .. }) = receiver.recv().await {
                break tx;
            }
        }
    })
    .await
    .expect("nested tool did not request permission");
    let second = session_agent(cache.session());
    execute(&second, "saved=2").await.unwrap();
    approval
        .send(mew_hooks::PermissionDecision::AllowOnce)
        .unwrap();
    assert_eq!(call.await.unwrap().unwrap().output, "42\n");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("approved.txt")).unwrap(),
        "ok"
    );
    assert!(execute(&second, "text(saved)")
        .await
        .unwrap_err()
        .to_string()
        .contains("capacity"));
    assert_eq!(execute(&first, "text(saved)").await.unwrap().output, "42\n");
}

async fn execute(
    agent: &crate::Agent,
    source: &str,
) -> Result<mew_hooks::ToolOutput, mew_tools::ToolError> {
    let ctx = mew_tools::ToolCtx::new(
        Arc::new(mew_tools::ToolCtxShared::default()),
        "test".into(),
        Default::default(),
        tokio::sync::mpsc::channel(1).0,
    );
    let (events, _receiver) = tokio::sync::mpsc::channel(32);
    agent
        .execute_python(&ctx, json!({"source":source}), &events)
        .await
}

#[tokio::test]
async fn unrestricted_python_uses_each_sessions_workspace() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let mut first = idle_agent(Duration::from_secs(60));
    let mut second = idle_agent(Duration::from_secs(60));
    first.cwd = first_dir.path().to_path_buf();
    second.cwd = second_dir.path().to_path_buf();

    for (agent, directory) in [(&first, &first_dir), (&second, &second_dir)] {
        let output = execute(agent, "import os\ntext(os.getcwd())")
            .await
            .unwrap()
            .output;
        assert_eq!(
            std::fs::canonicalize(output.trim()).unwrap(),
            std::fs::canonicalize(directory.path()).unwrap()
        );
    }
}

#[tokio::test]
async fn python_redacts_secrets_from_results_and_errors() {
    let agent = idle_agent(Duration::from_secs(60));
    let secrets = mew_tools::SecretSet {
        words: vec!["synthetic-secret".into()],
        globs: vec![],
    };
    let ctx = mew_tools::ToolCtx::new(
        Arc::new(mew_tools::ToolCtxShared {
            secrets: Arc::new(secrets),
            ..Default::default()
        }),
        "test".into(),
        Default::default(),
        tokio::sync::mpsc::channel(1).0,
    );
    let (events, _receiver) = tokio::sync::mpsc::channel(32);
    for source in [
        "text('synthetic-secret')",
        "print('synthetic-secret')\nraise ValueError('synthetic-secret')",
    ] {
        let result = agent
            .execute_python(&ctx, json!({"source":source}), &events)
            .await
            .unwrap();
        let combined = format!("{}{}", result.output, result.error);
        assert!(
            !combined.contains("synthetic-secret"),
            "secret leaked in {combined}"
        );
        assert!(combined.contains("[REDACTED]"), "{combined}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn python_redacts_secrets_from_worker_failure_diagnostics() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("python-fails");
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf synthetic-secret >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let secrets = mew_tools::SecretSet {
        words: vec!["synthetic-secret".into()],
        globs: vec![],
    };
    let ctx = mew_tools::ToolCtx::new(
        Arc::new(mew_tools::ToolCtxShared {
            secrets: Arc::new(secrets),
            ..Default::default()
        }),
        "test".into(),
        Default::default(),
        tokio::sync::mpsc::channel(1).0,
    );
    for startup in [true, false] {
        let mut agent = idle_agent(Duration::from_secs(60));
        if startup {
            agent.code_mode.python = Some(executable.clone());
        }
        let source = "import os\nos.write(2, b'synthetic-secret')\nos._exit(1)";
        let (events, _receiver) = tokio::sync::mpsc::channel(32);
        let error = agent
            .execute_python(&ctx, json!({"source":source}), &events)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            !error.contains("synthetic-secret"),
            "secret leaked in {error}"
        );
        assert!(error.contains("[REDACTED]"), "{error}");
    }
}

#[tokio::test]
async fn malformed_python_images_are_redacted_and_reset_the_worker() {
    let agent = idle_agent(Duration::from_secs(60));
    let secrets = mew_tools::SecretSet {
        words: vec!["synthetic-secret".into()],
        globs: vec![],
    };
    let ctx = mew_tools::ToolCtx::new(
        Arc::new(mew_tools::ToolCtxShared {
            secrets: Arc::new(secrets),
            ..Default::default()
        }),
        "test".into(),
        Default::default(),
        tokio::sync::mpsc::channel(1).0,
    );
    let source = "saved=42\nimport __main__, json\n__main__.protocol.write(json.dumps({'type':'result','cell':__main__.active_cell,'images':'synthetic-secret','output':''})+'\\n')\n__main__.protocol.flush()";
    let (events, _receiver) = tokio::sync::mpsc::channel(32);
    let error = agent
        .execute_python(&ctx, json!({"source":source}), &events)
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("synthetic-secret"), "{error}");
    assert!(
        error.contains("[REDACTED]") && error.contains("state was lost"),
        "{error}"
    );
    assert_eq!(
        execute(&agent, "text('saved' in globals())")
            .await
            .unwrap()
            .output,
        "false\n"
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn restricting_persona_restarts_python_in_isolation() {
    let secret = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(secret.path(), "private fixture").unwrap();
    let source = format!(
        "try:\n text(open({:?}).read())\nexcept OSError:\n text('denied')",
        secret.path().to_string_lossy()
    );
    let mut agent = idle_agent(Duration::from_secs(60));
    assert_eq!(
        execute(&agent, &source).await.unwrap().output,
        "private fixture\n"
    );

    agent.active_tool_names = Some(std::collections::HashSet::from(["read".into()]));
    let error = execute(&agent, "text('must not run')")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("isolation") && error.contains("lost"),
        "{error}"
    );
    assert_eq!(execute(&agent, &source).await.unwrap().output, "denied\n");

    agent.active_tool_names = None;
    let error = execute(&agent, "text('must not run')")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("isolation") && error.contains("lost"),
        "{error}"
    );
    assert_eq!(
        execute(&agent, &source).await.unwrap().output,
        "private fixture\n"
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn persona_denylist_automatically_isolates_python() {
    let secret = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(secret.path(), "private fixture").unwrap();
    let mut agent = idle_agent(Duration::from_secs(60));
    agent.denied_tool_names.insert("bash".into());
    let source = format!(
        "try:\n text(open({:?}).read())\nexcept OSError:\n text('denied')",
        secret.path().to_string_lossy()
    );
    assert_eq!(execute(&agent, &source).await.unwrap().output, "denied\n");
}

#[cfg(unix)]
#[tokio::test]
async fn disabling_python_stops_an_existing_unrestricted_worker() {
    let mut agent = idle_agent(Duration::from_secs(60));
    let pid = execute(&agent, "import os\ntext(os.getpid())")
        .await
        .unwrap()
        .output
        .trim()
        .parse()
        .unwrap();
    agent.apply_persona(&mew_personas::Persona {
        name: "no tools".into(),
        description: String::new(),
        body: String::new(),
        path: Default::default(),
        config: mew_personas::PersonaConfig {
            tools: Some(vec![]),
            ..Default::default()
        },
    });
    wait_for_process_exit(pid).await;
}

#[cfg(unix)]
#[tokio::test]
async fn reattaching_a_restricted_session_stops_the_old_unrestricted_worker() {
    let session = PythonSession::with_idle_timeout(Duration::from_secs(60));
    let first = session_agent(session.clone());
    let pid = execute(&first, "import os\ntext(os.getpid())")
        .await
        .unwrap()
        .output
        .trim()
        .parse()
        .unwrap();
    let mut resumed = idle_agent(Duration::from_secs(60));
    resumed.apply_persona(&mew_personas::Persona {
        name: "no tools".into(),
        description: String::new(),
        body: String::new(),
        path: Default::default(),
        config: mew_personas::PersonaConfig {
            tools: Some(vec![]),
            ..Default::default()
        },
    });
    resumed.set_python_session(session);
    wait_for_process_exit(pid).await;
}

fn outer_approval_agent(session: PythonSession) -> crate::Agent {
    let provider = Arc::new(mew_provider_fake::FakeProvider::new(
        mew_provider_fake::FakeProvider::tool_call(
            "python",
            "outer",
            json!({"source":"text(saved)"}),
        ),
    ));
    let mut agent = crate::Agent::new(
        provider,
        Arc::new(mew_hooks::NopDispatcher),
        None,
        vec![],
        None,
    );
    agent.code_mode.mode = Some(mew_config::CodeMode::Full);
    agent.code_mode.isolated = false;
    agent.max_turns = Some(1);
    agent.set_python_session(session);
    agent
}

async fn pending_outer_approval(
    agent: &crate::Agent,
    cancel: tokio_util::sync::CancellationToken,
) -> (
    tokio::sync::mpsc::Receiver<crate::AgentEvent>,
    tokio::sync::oneshot::Sender<mew_hooks::PermissionDecision>,
) {
    let mut events = agent.run_with_parts("continue".into(), vec![], Some(cancel));
    let approval = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(crate::AgentEvent::PermissionRequest { call, tx }) = events.recv().await {
                assert_eq!(call.tool_name, "python");
                break tx;
            }
        }
    })
    .await
    .expect("Python did not request permission");
    (events, approval)
}

async fn outer_output(
    events: &mut tokio::sync::mpsc::Receiver<crate::AgentEvent>,
) -> Result<String, String> {
    let mut result = None;
    let mut ended = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.recv().await {
            if let crate::AgentEvent::PartUpdated {
                part: mew_message::Part::ToolCall(call),
                ..
            } = event
            {
                if call.call_id == "outer" {
                    match call.state {
                        mew_message::ToolState::Completed(state) => result = Some(Ok(state.output)),
                        mew_message::ToolState::Error(state) => result = Some(Err(state.error)),
                        _ => {}
                    }
                }
            } else if let crate::AgentEvent::ToolEnd { call_id, .. } = event {
                ended |= call_id == "outer";
            }
        }
    })
    .await
    .expect("turn did not finish");
    assert!(ended, "Python tool did not emit ToolEnd");
    result.expect("missing final Python result")
}

#[tokio::test]
async fn outer_python_approval_protects_worker_from_capacity_eviction() {
    let cache = PythonWorkerCache::new(1);
    let first = outer_approval_agent(cache.session());
    execute(&first, "saved=42").await.unwrap();
    let (mut events, approval) = pending_outer_approval(&first, Default::default()).await;
    let second = session_agent(cache.session());
    execute(&second, "saved=2").await.unwrap();
    approval
        .send(mew_hooks::PermissionDecision::AllowOnce)
        .unwrap();
    assert_eq!(outer_output(&mut events).await.unwrap(), "42\n");
    assert!(execute(&second, "text(saved)")
        .await
        .unwrap_err()
        .to_string()
        .contains("capacity"));
}

#[tokio::test]
async fn outer_python_approval_protects_worker_from_idle_expiry() {
    let first = outer_approval_agent(PythonSession::with_idle_timeout(Duration::from_secs(60)));
    execute(&first, "saved=42").await.unwrap();
    let (mut events, approval) = pending_outer_approval(&first, Default::default()).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    approval
        .send(mew_hooks::PermissionDecision::AllowOnce)
        .unwrap();
    assert_eq!(outer_output(&mut events).await.unwrap(), "42\n");
}

#[tokio::test]
async fn denied_or_cancelled_outer_python_approval_releases_worker() {
    for cancel_request in [false, true] {
        let cache = PythonWorkerCache::new(1);
        let first = outer_approval_agent(cache.session());
        execute(&first, "saved=42").await.unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (mut events, mut approval) = pending_outer_approval(&first, cancel.clone()).await;
        if cancel_request {
            cancel.cancel();
            tokio::time::timeout(Duration::from_secs(2), approval.closed())
                .await
                .expect("cancel did not release approval");
        } else {
            approval.send(mew_hooks::PermissionDecision::Deny).unwrap();
        }
        let _ = outer_output(&mut events).await;
        let second = session_agent(cache.session());
        execute(&second, "saved=2").await.unwrap();
        assert!(execute(&first, "text(saved)")
            .await
            .unwrap_err()
            .to_string()
            .contains("capacity"));
    }
}

#[tokio::test]
async fn cancelling_python_call_waiting_for_session_lock_finishes_tool_state() {
    let dir = tempfile::tempdir().unwrap();
    let cache = PythonWorkerCache::new(1);
    let session = cache.session();
    let mut first = crate::Agent::new(
        Arc::new(mew_provider_fake::FakeProvider::new(vec![])),
        Arc::new(mew_hooks::NopDispatcher),
        None,
        vec![Arc::new(mew_tools::tools::write::Write)],
        None,
    );
    first.code_mode.isolated = false;
    first.cwd = dir.path().to_path_buf();
    first.workspace_roots = vec![dir.path().to_path_buf()];
    first.set_python_session(session.clone());
    execute(&first, "saved=42").await.unwrap();
    let first = Arc::new(first);
    let (events, mut receiver) = tokio::sync::mpsc::channel(32);
    let active = first.clone();
    let first_call = tokio::spawn(async move {
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "test".into(),
            Default::default(),
            tokio::sync::mpsc::channel(1).0,
        );
        active.execute_python(&ctx, json!({"source":"tools.write({'path':'approved.txt','content':'ok'})\ntext(saved)"}), &events).await
    });
    let approval = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(crate::AgentEvent::PermissionRequest { tx, .. }) = receiver.recv().await {
                break tx;
            }
        }
    })
    .await
    .expect("nested tool did not request permission");
    let queued = outer_approval_agent(session);
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut queued_events = queued.run_with_parts("continue".into(), vec![], Some(cancel.clone()));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(crate::AgentEvent::ToolStart { call_id }) = queued_events.recv().await {
                if call_id == "outer" {
                    break;
                }
            }
        }
    })
    .await
    .expect("queued Python call did not start");
    cancel.cancel();
    let error = outer_output(&mut queued_events).await.unwrap_err();
    assert!(error.contains("cancel"), "{error}");
    approval
        .send(mew_hooks::PermissionDecision::AllowOnce)
        .unwrap();
    assert_eq!(first_call.await.unwrap().unwrap().output, "42\n");
    assert_eq!(execute(&first, "text(saved)").await.unwrap().output, "42\n");
}

#[tokio::test]
async fn python_cancelled_before_dispatch_keeps_state_and_does_not_run_code() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("must-not-exist");
    let agent = idle_agent(Duration::from_secs(60));
    execute(&agent, "saved=42").await.unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let ctx = mew_tools::ToolCtx::new(
        Arc::new(mew_tools::ToolCtxShared::default()),
        "test".into(),
        cancel,
        tokio::sync::mpsc::channel(1).0,
    );
    let (events, _receiver) = tokio::sync::mpsc::channel(32);
    let source = format!("open({:?}, 'w').write('wrong')", marker.to_string_lossy());
    assert!(agent
        .execute_python(&ctx, json!({"source":source}), &events)
        .await
        .is_err());
    assert!(!marker.exists(), "cancelled code ran");
    assert_eq!(execute(&agent, "text(saved)").await.unwrap().output, "42\n");
}

#[tokio::test]
async fn cancelled_python_call_does_not_consume_idle_state_loss_notice() {
    let agent = idle_agent(Duration::from_secs(60));
    execute(&agent, "saved=42").await.unwrap();
    tokio::time::pause();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(61)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let ctx = mew_tools::ToolCtx::new(
        Arc::new(mew_tools::ToolCtxShared::default()),
        "test".into(),
        cancel,
        tokio::sync::mpsc::channel(1).0,
    );
    let (events, _receiver) = tokio::sync::mpsc::channel(32);
    let error = agent
        .execute_python(&ctx, json!({"source":"text(saved)"}), &events)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("cancel"), "{error}");
    let error = execute(&agent, "text(saved)")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("idle") && error.contains("lost"), "{error}");
}

#[tokio::test]
async fn idle_python_worker_stops_and_reports_lost_state_before_executing() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("should-not-exist");
    let agent = idle_agent(Duration::from_secs(60));
    let saved = execute(&agent, "saved = 42\ntext(saved)").await.unwrap();
    assert_eq!(saved.output, "42\n");
    let reattached = agent.clone();
    tokio::time::pause();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(61)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();

    let source = format!(
        "open({:?}, 'w').write('unexpected')",
        marker.to_string_lossy()
    );
    let error = execute(&reattached, &source).await.unwrap_err().to_string();
    assert!(error.contains("idle"), "{error}");
    assert!(error.contains("variables"), "{error}");
    assert!(error.contains("scratch"), "{error}");
    assert!(!marker.exists(), "the first call after expiry must not run");
    assert_eq!(
        execute(&reattached, "text('saved' in globals())")
            .await
            .unwrap()
            .output,
        "false\n"
    );
}

#[tokio::test]
async fn python_activity_extends_idle_lifetime() {
    let agent = idle_agent(Duration::from_secs(60));
    assert_eq!(
        execute(&agent, "saved = 42\ntext(saved)")
            .await
            .unwrap()
            .output,
        "42\n"
    );
    tokio::time::pause();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(40)).await;
    tokio::time::resume();
    assert_eq!(
        execute(&agent, "text(saved + 1)").await.unwrap().output,
        "43\n"
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    assert_eq!(execute(&agent, "text(saved)").await.unwrap().output, "42\n");
}

#[tokio::test]
async fn python_does_not_expire_while_waiting_for_tool_approval() {
    let directory = tempfile::tempdir().unwrap();
    let mut agent = crate::Agent::new(
        Arc::new(mew_provider_fake::FakeProvider::new(vec![])),
        Arc::new(mew_hooks::NopDispatcher),
        None,
        vec![Arc::new(mew_tools::tools::write::Write)],
        None,
    );
    agent.code_mode.isolated = false;
    agent.cwd = directory.path().to_path_buf();
    agent.workspace_roots = vec![directory.path().to_path_buf()];
    agent.set_python_session(PythonSession::with_idle_timeout(Duration::from_secs(60)));
    assert_eq!(
        execute(&agent, "saved=42\ntext(saved)")
            .await
            .unwrap()
            .output,
        "42\n"
    );
    let agent = Arc::new(agent);
    let (events, mut receiver) = tokio::sync::mpsc::channel(32);
    let active = agent.clone();
    let call = tokio::spawn(async move {
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "test".into(),
            Default::default(),
            tokio::sync::mpsc::channel(1).0,
        );
        active.execute_python(&ctx, json!({"source":"tools.write({'path':'approved.txt','content':'ok'})\ntext(saved)"}), &events).await
    });
    let approval = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(crate::AgentEvent::PermissionRequest { tx, .. }) = receiver.recv().await {
                break tx;
            }
        }
    })
    .await
    .expect("nested tool did not request permission");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    approval
        .send(mew_hooks::PermissionDecision::AllowOnce)
        .unwrap();
    tokio::time::resume();
    assert_eq!(call.await.unwrap().unwrap().output, "42\n");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("approved.txt")).unwrap(),
        "ok"
    );
    assert_eq!(execute(&agent, "text(saved)").await.unwrap().output, "42\n");
}

#[cfg(unix)]
async fn wait_for_process_exit(pid: i32) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
                Err(nix::errno::Errno::ESRCH) => break,
                Ok(()) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("checking Python process failed: {error}"),
            }
        }
    })
    .await
    .expect("Python worker did not stop");
}

#[cfg(unix)]
#[tokio::test]
async fn idle_expiry_terminates_the_python_process() {
    let agent = idle_agent(Duration::from_secs(60));
    let output = execute(&agent, "import os\ntext(os.getpid())")
        .await
        .unwrap()
        .output;
    let pid = output.trim().parse().unwrap();
    tokio::time::pause();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(61)).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    wait_for_process_exit(pid).await;
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_the_session_stops_its_worker_without_waiting_for_idle_timeout() {
    let agent = idle_agent(Duration::from_secs(3600));
    let output = execute(&agent, "import os\ntext(os.getpid())")
        .await
        .unwrap()
        .output;
    let pid = output.trim().parse().unwrap();
    drop(agent);
    wait_for_process_exit(pid).await;
}

async fn worker(isolated: bool) -> PythonWorker {
    PythonWorker::start(&PythonOptions {
        executable: PathBuf::from("python3"),
        isolated,
        cwd: std::env::current_dir().unwrap(),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn python_start_reports_when_the_interpreter_exits_immediately() {
    let result = PythonWorker::start(&PythonOptions {
        executable: PathBuf::from("/usr/bin/false"),
        isolated: false,
        cwd: std::env::current_dir().unwrap(),
    })
    .await;
    assert!(result.is_err());
}

async fn run(worker: &mut PythonWorker, source: &str) -> Value {
    worker
        .send(&json!({"type":"exec", "cell":"test-cell", "source":source, "tools":[]}))
        .await
        .unwrap();
    let result = worker.receive().await.unwrap();
    assert_eq!(result["cell"], "test-cell");
    result
}

#[tokio::test]
async fn python_variables_survive_calls_and_exceptions_but_not_other_sessions() {
    let mut first = worker(false).await;
    let mut second = worker(false).await;
    assert_eq!(
        run(&mut first, "saved = 41\ntext(saved)").await["output"],
        "41\n"
    );
    assert_eq!(
        run(&mut first, "raise ValueError('bad')").await["type"],
        "error"
    );
    assert_eq!(run(&mut first, "text(saved + 1)").await["output"], "42\n");
    assert_eq!(
        run(&mut second, "text('saved' in globals())").await["output"],
        "false\n"
    );
}

#[tokio::test]
async fn python_tools_send_arguments_and_receive_the_host_result() {
    let mut worker = worker(false).await;
    worker
        .send(&json!({
            "type":"exec",
            "cell":"tool-cell",
            "source":"result = tools.read({'path':'file.txt'})\ntext(result['output'])",
            "tools":[{"name":"read","tool_name":"read"}]
        }))
        .await
        .unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    assert_eq!(call["cell"], "tool-cell");
    assert_eq!(call["name"], "read");
    assert_eq!(call["input"], json!({"path":"file.txt"}));
    worker.send(&json!({"type":"tool_result","cell":"tool-cell","id":call["id"],"result":{"output":"hello","error":""}})).await.unwrap();
    let result = worker.receive().await.unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["cell"], "tool-cell");
    assert_eq!(result["output"], "hello\n");
}

#[tokio::test]
async fn python_output_is_bounded_and_later_calls_still_work() {
    let mut worker = worker(false).await;
    let result = run(&mut worker, "print('x' * 1000000)").await;
    let output = result["output"].as_str().unwrap();
    assert!(output.len() < 150_000);
    assert!(output.contains("[output truncated]"));
    assert_eq!(
        run(&mut worker, "text('still running')").await["output"],
        "still running\n"
    );
}

#[tokio::test]
async fn python_tool_calls_from_threads_receive_their_own_results() {
    let mut worker = worker(false).await;
    worker.send(&json!({
        "type":"exec", "cell":"threads",
        "source":"from concurrent.futures import ThreadPoolExecutor\ndef run(x): return tools.echo({'input':x})['output']\nwith ThreadPoolExecutor(max_workers=2) as pool:\n text(sorted(pool.map(run, ['one','two'])))",
        "tools":[{"name":"echo","tool_name":"echo"}]
    })).await.unwrap();
    for _ in 0..2 {
        let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(call["type"], "tool");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let response = json!({"type":"tool_result","cell":"threads","id":call["id"],"result":{"output":call["input"]["input"],"error":""}});
        worker.send(&response).await.unwrap();
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["output"], "[\"one\",\"two\"]\n");
}

#[tokio::test]
async fn python_can_reuse_a_thread_pool_for_tool_calls_across_cells() {
    let mut worker = worker(false).await;
    let tools = json!([{"name":"echo","tool_name":"echo"}]);
    for (cell, source, expected) in [
        ("first", "from concurrent.futures import ThreadPoolExecutor\npool=ThreadPoolExecutor(max_workers=1)\ntext(pool.submit(tools.echo, {'input':'one'}).result()['output'])", "one\n"),
        ("second", "text(pool.submit(tools.echo, {'input':'two'}).result()['output'])\npool.shutdown()", "two\n"),
    ] {
        worker.send(&json!({"type":"exec","cell":cell,"tools":tools,"source":source})).await.unwrap();
        let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive()).await.unwrap().unwrap();
        assert_eq!(call["type"], "tool");
        assert_eq!(call["cell"], cell);
        worker.send(&json!({"type":"tool_result","cell":cell,"id":call["id"],"result":{"output":expected.trim(),"error":""}})).await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive()).await.unwrap().unwrap();
        assert_eq!(result["type"], "result", "{result}");
        assert_eq!(result["output"], expected);
    }
}

#[tokio::test]
async fn python_waits_for_nested_pool_tasks_that_call_tools() {
    let mut worker = worker(false).await;
    worker.send(&json!({"type":"exec","cell":"first","tools":[{"name":"echo","tool_name":"echo"}],"source":"from concurrent.futures import ThreadPoolExecutor\nimport threading\npool=ThreadPoolExecutor(max_workers=2)\nstarted=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef child():\n return tools.echo({'input':'two'})\ndef parent():\n tools.echo(Arguments(input='one'))\n return pool.submit(child).result()\nfuture=pool.submit(parent)\nassert started.wait(2)\ntext('submitted')"})).await.unwrap();
    for expected in ["one", "two"] {
        let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(call["type"], "tool", "{call}");
        assert_eq!(call["input"]["input"], expected);
        worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":expected,"error":""}})).await.unwrap();
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result", "{result}");
    assert_eq!(result["output"], "submitted\n");
    assert_eq!(
        run(
            &mut worker,
            "text(future.result()['output'])\npool.shutdown()"
        )
        .await["output"],
        "two\n"
    );
}

#[tokio::test]
async fn python_resets_when_queued_pool_work_outlives_its_cell() {
    let mut worker = worker(false).await;
    let tools = json!([{"name":"echo","tool_name":"echo"}]);
    worker.send(&json!({"type":"exec","cell":"first","tools":tools,"source":"from concurrent.futures import ThreadPoolExecutor\nimport threading\npool=ThreadPoolExecutor(max_workers=1)\nstarted=threading.Event()\nrelease=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef later():\n release.wait()\n return tools.echo({'input':'late'})\nfirst=pool.submit(tools.echo, Arguments(input='one'))\nassert started.wait(2)\nlate=pool.submit(later)\ntext('submitted')"})).await.unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"one","error":""}})).await.unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first["type"], "error", "{first}");
    assert_eq!(first["reset"], true);
    assert!(
        first["error"].as_str().unwrap().contains("background"),
        "{first}"
    );
}

#[tokio::test]
async fn python_does_not_capture_old_thread_output_in_a_later_cell() {
    let mut worker = worker(false).await;
    let first = run(&mut worker, "import threading\nrelease=threading.Event()\ndone=threading.Event()\ndef later():\n release.wait()\n print('old output')\n done.set()\nthreading.Thread(target=later,daemon=True).start()\ntext('first')").await;
    assert_eq!(first["output"], "first\n");
    let second = run(
        &mut worker,
        "release.set()\nassert done.wait(2)\ntext('second')",
    )
    .await;
    assert_eq!(second["output"], "second\n");
}

#[tokio::test]
async fn python_rejects_images_from_a_previous_cell() {
    let mut worker = worker(false).await;
    let first = run(&mut worker, "import threading\nrelease=threading.Event()\ndone=threading.Event()\ndef later():\n release.wait()\n try:\n  emit_image({'mime':'image/png','data':'YWJj'})\n except RuntimeError:\n  pass\n finally:\n  done.set()\nthreading.Thread(target=later,daemon=True).start()").await;
    assert_eq!(first["type"], "result");
    let second = run(
        &mut worker,
        "release.set()\nassert done.wait(2)\ntext('second')",
    )
    .await;
    assert_eq!(second["output"], "second\n");
    assert_eq!(second["images"], json!([]));
}

#[tokio::test]
async fn python_does_not_emit_a_previous_cells_inflight_image() {
    let mut worker = worker(false).await;
    let first = run(&mut worker, "import threading\nstarted=threading.Event()\nrelease=threading.Event()\ndone=threading.Event()\nclass Image(dict):\n def get(self, key, *args):\n  if key == 'mime':\n   started.set()\n   release.wait()\n  return super().get(key, *args)\ndef later():\n try:\n  emit_image(Image(mime='image/png',data='YWJj'))\n finally:\n  done.set()\nthreading.Thread(target=later,daemon=True).start()\nassert started.wait(2)").await;
    assert_eq!(first["type"], "result");
    let second = run(
        &mut worker,
        "release.set()\nassert done.wait(2)\ntext('second')",
    )
    .await;
    assert_eq!(second["output"], "second\n");
    assert_eq!(second["images"], json!([]));
}

#[tokio::test]
async fn python_captures_future_callback_output_in_its_originating_cell() {
    let mut worker = worker(false).await;
    worker.send(&json!({"type":"exec","cell":"first","tools":[{"name":"echo","tool_name":"echo"}],"source":"from concurrent.futures import ThreadPoolExecutor\nimport threading, time\npool=ThreadPoolExecutor(max_workers=1)\nstarted=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\nf=pool.submit(tools.echo, Arguments(input='hi'))\nassert started.wait(2)\nf.add_done_callback(lambda _: (time.sleep(.2), text('callback output')))"})).await.unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"hi","error":""}})).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result", "{result}");
    assert_eq!(result["output"], "callback output\n");
}

#[tokio::test]
async fn python_background_threads_cannot_call_tools_in_a_later_cell() {
    let mut worker = worker(false).await;
    let tools = json!([{"name":"echo","tool_name":"echo"}]);
    worker.send(&json!({
        "type":"exec", "cell":"first", "tools":tools,
        "source":"import threading\nrelease = threading.Event()\ndone = threading.Event()\nresult = []\ndef later():\n release.wait()\n try:\n  tools.echo({'from':'first'})\n  result.append('called')\n except RuntimeError:\n  result.append('rejected')\n finally:\n  done.set()\nthreading.Thread(target=later, daemon=True).start()\nsaved = 42\ntext('ready')"
    })).await.unwrap();
    let first = worker.receive().await.unwrap();
    assert_eq!(first["type"], "result");
    assert_eq!(first["cell"], "first");
    worker.send(&json!({
        "type":"exec", "cell":"second", "tools":tools,
        "source":"release.set()\nassert done.wait(2), 'background thread did not finish'\ntext([result, saved])"
    })).await.unwrap();
    let second = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second["type"], "result",
        "stale thread sent a tool call: {second}"
    );
    assert_eq!(second["cell"], "second");
    assert_eq!(second["output"], "[[\"rejected\"],42]\n");
}

#[tokio::test]
async fn python_waits_for_a_background_tool_call_before_finishing_its_cell() {
    let mut worker = worker(false).await;
    worker.send(&json!({
        "type":"exec", "cell":"first",
        "tools":[{"name":"echo","tool_name":"echo"}],
        "source":"import threading\nstarted = threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef later():\n r = tools.echo(Arguments(value='hello'))\n r = tools.echo({'value':r['output']})\n text('received ' + r['output'])\nthreading.Thread(target=later, daemon=True).start()\nassert started.wait(2), 'background call did not start'\nsaved = 42\ntext('finished')"
    })).await.unwrap();
    let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call["type"], "tool");
    assert_eq!(call["cell"], "first");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), worker.receive())
            .await
            .is_err(),
        "cell finished before its tool call"
    );
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"ok","error":""}})).await.unwrap();
    let second_call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second_call["type"], "tool", "{second_call}");
    assert_eq!(second_call["input"]["value"], "ok");
    worker.send(&json!({"type":"tool_result","cell":"first","id":second_call["id"],"result":{"output":"ok","error":""}})).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["output"], "finished\nreceived ok\n");
    assert_eq!(run(&mut worker, "text(saved)").await["output"], "42\n");
}

#[tokio::test]
async fn python_reports_unfinished_tool_thread_and_requires_a_reset() {
    let mut worker = worker(false).await;
    worker.send(&json!({
        "type":"exec", "cell":"first", "tools":[{"name":"echo","tool_name":"echo"}],
        "source":"import threading\nstarted=threading.Event()\nrelease=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef later():\n tools.echo(Arguments(value='hello'))\n release.wait()\nthreading.Thread(target=later, daemon=True).start()\nassert started.wait(2)"
    })).await.unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"ok","error":""}})).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "error");
    assert_eq!(result["reset"], true);
    assert!(result["error"].as_str().unwrap().contains("background"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn isolated_python_cannot_open_host_files_connect_to_localhost_or_spawn() {
    let secret = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(secret.path(), "untouched").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut worker = worker(true).await;
    let code = format!(
        "import os, socket, subprocess\ntry:\n open({0:?}).read()\n file_ok=True\nexcept OSError:\n file_ok=False\ntry:\n os.symlink({0:?}, os.path.join(os.environ['HOME'], 'link'))\n open(os.path.join(os.environ['HOME'], 'link')).read()\n link_ok=True\nexcept OSError:\n link_ok=False\ntry:\n open({0:?}, 'w').write('changed')\nexcept OSError:\n pass\ntry:\n s=socket.socket()\n s.settimeout(1)\n s.connect(('127.0.0.1', {1}))\n net_ok=True\nexcept OSError:\n net_ok=False\ntry:\n subprocess.run(['/usr/bin/true'], check=True)\n spawn_ok=True\nexcept OSError:\n spawn_ok=False\ntext([file_ok,link_ok,net_ok,spawn_ok])",
        secret.path().to_string_lossy(), address.port()
    );
    assert_eq!(
        run(&mut worker, &code).await["output"],
        "[false,false,false,false]\n"
    );
    assert_eq!(std::fs::read_to_string(secret.path()).unwrap(), "untouched");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn isolated_python_uses_the_selected_virtual_environment() {
    let dir = tempfile::tempdir().unwrap();
    let status = tokio::process::Command::new("python3")
        .args(["-I", "-m", "venv"])
        .arg(dir.path())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let output = tokio::process::Command::new(dir.path().join("bin/python3"))
        .args([
            "-I",
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let packages = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    std::fs::write(packages.join("mew_isolation_test.py"), "value = 42\n").unwrap();
    let mut worker = PythonWorker::start(&PythonOptions {
        executable: dir.path().join("bin/python3"),
        isolated: true,
        cwd: dir.path().to_path_buf(),
    })
    .await
    .unwrap();
    let result = run(&mut worker, "import sys; text(sys.prefix)").await;
    assert_eq!(
        std::fs::canonicalize(result["output"].as_str().unwrap().trim()).unwrap(),
        std::fs::canonicalize(dir.path()).unwrap()
    );
    assert_eq!(
        run(
            &mut worker,
            "import mew_isolation_test; text(mew_isolation_test.value)"
        )
        .await["output"],
        "42\n"
    );
    let secret = dir.path().join("private.txt");
    std::fs::write(&secret, "private").unwrap();
    let secret = std::fs::canonicalize(secret).unwrap();
    let source = format!(
        "try:\n open({:?}).read()\n text('readable')\nexcept OSError:\n text('denied')",
        secret.to_string_lossy()
    );
    assert_eq!(run(&mut worker, &source).await["output"], "denied\n");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_isolated_python_terminates_its_background_processes() {
    use std::io::Read;
    let mut worker = worker(true).await;
    let code = "import os, time\nr, w = os.pipe()\npid = os.fork()\nif pid == 0:\n os.close(r)\n f = open(os.path.join(os.environ['HOME'], 'late'), 'w')\n os.write(w, b'x')\n time.sleep(1.5)\n f.write('escaped')\n f.flush()\n os._exit(0)\nos.close(w)\nos.read(r, 1)\ntext('ready')";
    assert_eq!(run(&mut worker, code).await["output"], "ready\n");
    let mut file = std::fs::File::open(worker.scratch_path().join("late")).unwrap();
    drop(worker);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert!(contents.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_unrestricted_python_terminates_its_background_processes() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("late");
    let mut worker = worker(false).await;
    let code = format!("import os, time\nr, w = os.pipe()\npid = os.fork()\nif pid == 0:\n os.close(r)\n f = open({:?}, 'w')\n os.write(w, b'x')\n time.sleep(1.5)\n f.write('escaped')\n f.flush()\n os._exit(0)\nos.close(w)\nos.read(r, 1)\ntext('ready')", marker.to_string_lossy());
    assert_eq!(run(&mut worker, &code).await["output"], "ready\n");
    drop(worker);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "");
}
