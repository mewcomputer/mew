use super::*;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mew_agent::AgentEvent;
use mew_hooks::{NopDispatcher, PermissionDecision, PermissionMode};
use mew_message::{Part, ToolState};
use mew_provider::{EventStream, Provider, ProviderError, ProviderEvent, Request};
use mew_provider_fake::FakeProvider;

/// Replay scripts at the provider boundary while retaining real request state.
struct ProjectProvider {
    scripts: Mutex<VecDeque<Vec<ProviderEvent>>>,
    requests: Mutex<Vec<Request>>,
}

impl ProjectProvider {
    fn new(scripts: Vec<Vec<ProviderEvent>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into()),
            requests: Mutex::new(vec![]),
        })
    }
}

#[async_trait::async_trait]
impl Provider for ProjectProvider {
    fn name(&self) -> &str {
        "project-test"
    }
    async fn stream(&self, request: Request) -> Result<EventStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected provider request");
        FakeProvider::new(script).stream(Request::default()).await
    }
}

fn tool_turn(name: &str, id: &str, input: serde_json::Value) -> Vec<Vec<ProviderEvent>> {
    vec![
        FakeProvider::tool_call(name, id, input),
        FakeProvider::text_response("done"),
    ]
}

fn project(marker: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("nested")).unwrap();
    std::fs::write(root.join("input.txt"), marker).unwrap();
    std::fs::write(
        root.join("AGENTS.md"),
        format!("---\nmew: true\n---\ncontext-{marker}: {{{{ cwd }}}}\n"),
    )
    .unwrap();
    for (folder, file, content) in [
        (
            "skills/cwd-test",
            "SKILL.md",
            format!(
                "---\nname: cwd-test\ndescription: project skill\ntemplate: true\n---\nskill-{marker}: {{{{ cwd }}}}\n"
            ),
        ),
        (
            "personas/cwd-test",
            "PERSONA.md",
            format!(
                "---\nname: cwd-test\ndescription: project persona\nmew:\n  template: true\n---\npersona-{marker}: {{{{ cwd }}}}\n"
            ),
        ),
        (
            "agents",
            "cwd-test.md",
            format!(
                "---\nname: cwd-test\ndescription: project child\ntools: [read, bash]\ntemplate: true\n---\nchild-{marker}: {{{{ cwd }}}}\n"
            ),
        ),
    ] {
        let path = root.join(".mew").join(folder);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(file), content).unwrap();
    }
    (directory, root)
}

fn build(
    root: &Path,
    provider: Arc<ProjectProvider>,
    cfg: &Config,
    writer: Option<SessionWriter>,
    id: Option<SessionId>,
) -> Agent {
    build_session_agent(
        cfg,
        None,
        provider,
        "fake",
        "fake",
        false,
        PermissionMode::Standard,
        writer,
        id,
        Some(root.to_path_buf()),
        Arc::new(NopDispatcher),
        None,
        &[],
    )
    .unwrap()
}

fn config() -> Config {
    Config {
        default_persona: "none".into(),
        plan_path: "PLAN.md".into(),
        ..Default::default()
    }
}

async fn run(agent: &Agent) -> usize {
    let mut workspace_requests = 0;
    let mut events = agent.run("do the requested work".into());
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(15), events.recv())
        .await
        .expect("session tool or permission response stalled")
    {
        match event {
            AgentEvent::WorkspacePermissionRequest { tx, .. } => {
                workspace_requests += 1;
                tx.send(PermissionDecision::AllowOnce).unwrap();
            }
            AgentEvent::PermissionRequest { tx, .. }
            | AgentEvent::SubagentPermissionRequest { tx, .. } => {
                tx.send(PermissionDecision::AllowOnce).unwrap();
            }
            AgentEvent::Error(error) => panic!("unexpected runtime error: {error}"),
            _ => {}
        }
    }
    workspace_requests
}

async fn output(agent: &Agent, call_id: &str) -> String {
    let messages = agent.messages.lock().await;
    messages
        .iter()
        .flat_map(|message| &message.parts)
        .find_map(|part| match part {
            Part::ToolCall(call) if call.call_id == call_id => {
                let ToolState::Completed(state) = &call.state else {
                    panic!("tool failed: {:?}", call.state)
                };
                Some(state.output.clone())
            }
            _ => None,
        })
        .expect("missing tool call")
}

#[tokio::test]
async fn relative_tools_and_plan_files_stay_in_each_session_project() {
    let process_cwd = std::env::current_dir().unwrap();
    let (_first, first) = project("first");
    let (_second, second) = project("second");
    let mut agents = Vec::new();
    for root in [&first, &second] {
        let scripts = [
            tool_turn("read", "read", serde_json::json!({"path":"input.txt"})),
            tool_turn(
                "write",
                "write",
                serde_json::json!({"path":"output.txt", "content":root.display().to_string()}),
            ),
            tool_turn(
                "write_plan",
                "plan",
                serde_json::json!({"content":"# scoped plan"}),
            ),
        ]
        .concat();
        agents.push(build(
            root,
            ProjectProvider::new(scripts),
            &config(),
            None,
            None,
        ));
    }
    // Check reads before any mutation so a broken cwd cannot write into the repository.
    run(&agents[0]).await;
    run(&agents[1]).await;
    assert!(output(&agents[0], "read").await.contains("first"));
    assert!(output(&agents[1], "read").await.contains("second"));
    for (agent, root) in agents.iter().zip([&first, &second]) {
        run(agent).await;
        run(agent).await;
        assert_eq!(
            std::fs::read_to_string(root.join("output.txt")).unwrap(),
            root.display().to_string()
        );
        assert_eq!(
            std::fs::read_to_string(root.join("PLAN.md")).unwrap(),
            "# scoped plan"
        );
        assert_eq!(agent.resolved_plan_path().unwrap(), root.join("PLAN.md"));
    }
    assert_eq!(std::env::current_dir().unwrap(), process_cwd);
}

#[tokio::test]
async fn explicit_workspace_roots_still_restrict_session_relative_tools() {
    let (_first, first) = project("restricted");
    let (_second, second) = project("allowed");
    let provider = ProjectProvider::new(
        [
            tool_turn("read", "denied", serde_json::json!({"path":"input.txt"})),
            tool_turn(
                "read",
                "allowed",
                serde_json::json!({"path":second.join("input.txt")}),
            ),
        ]
        .concat(),
    );
    let mut cfg = config();
    cfg.workspace.roots = vec![second.clone()];
    let agent = build(&first, provider, &cfg, None, None);
    let mut events = agent.run("read input.txt".into());
    let mut workspace_requests = 0;
    while let Some(event) = events.recv().await {
        match event {
            AgentEvent::WorkspacePermissionRequest { path, tx } => {
                assert_eq!(path, first);
                workspace_requests += 1;
                tx.send(PermissionDecision::Deny).unwrap();
            }
            AgentEvent::PermissionRequest { tx, .. } => {
                tx.send(PermissionDecision::AllowOnce).unwrap();
            }
            AgentEvent::Error(error) => panic!("unexpected runtime error: {error}"),
            _ => {}
        }
    }
    assert_eq!(workspace_requests, 1);
    assert!(agent.messages.lock().await.iter().flat_map(|message| &message.parts).any(|part| {
        matches!(part, Part::ToolCall(call) if call.call_id == "denied" && matches!(&call.state, ToolState::Error(state) if state.error.contains("denied")))
    }));
    assert_eq!(run(&agent).await, 0);
    assert!(output(&agent, "allowed").await.contains("allowed"));
}

#[tokio::test]
async fn persistent_shell_cwd_and_exports_are_isolated_between_sessions() {
    let process_cwd = std::env::current_dir().unwrap();
    let (_first, first) = project("first");
    let (_second, second) = project("second");
    let mut agents = Vec::new();
    for (marker, root) in [("first", &first), ("second", &second)] {
        let scripts = [
            tool_turn("bash", "initial", serde_json::json!({"command":"pwd"})),
            tool_turn("bash", "change", serde_json::json!({"command":format!("export MEW_SESSION_CWD_TEST={marker}; cd nested; pwd")})),
            tool_turn("bash", "persisted", serde_json::json!({"command":"printf '%s:%s\\n' \"$MEW_SESSION_CWD_TEST\" \"$PWD\""})),
        ].concat();
        agents.push(build(
            root,
            ProjectProvider::new(scripts),
            &config(),
            None,
            None,
        ));
    }
    run(&agents[0]).await;
    run(&agents[1]).await;
    assert_eq!(
        shell_stdout(&output(&agents[0], "initial").await),
        first.display().to_string()
    );
    assert_eq!(
        shell_stdout(&output(&agents[1], "initial").await),
        second.display().to_string()
    );
    for agent in &agents {
        run(agent).await;
    }
    for agent in &agents {
        run(agent).await;
    }
    for ((marker, root), agent) in [("first", &first), ("second", &second)]
        .into_iter()
        .zip(&agents)
    {
        assert_eq!(
            shell_stdout(&output(agent, "change").await),
            root.join("nested").display().to_string()
        );
        assert_eq!(
            shell_stdout(&output(agent, "persisted").await),
            format!("{marker}:{}", root.join("nested").display())
        );
    }
    assert_eq!(std::env::current_dir().unwrap(), process_cwd);
}

#[tokio::test]
async fn background_and_monitored_shells_resolve_cwd_from_each_session() {
    let (_first, first) = project("first-jobs");
    let (_second, second) = project("second-jobs");
    for root in [&first, &second] {
        let provider = ProjectProvider::new(
            [
                tool_turn(
                    "shell_background",
                    "background",
                    serde_json::json!({"command":"pwd"}),
                ),
                tool_turn(
                    "shell_monitor",
                    "monitor",
                    serde_json::json!({"command":"pwd", "cwd":"nested", "timeout_secs":10}),
                ),
            ]
            .concat(),
        );
        let agent = build(root, provider, &config(), None, None);
        run(&agent).await;
        let job = agent.shell_jobs.lock().await.keys().next().unwrap().clone();
        let (state, result) = agent.shell_job_block(&job, 10).await.unwrap();
        assert_eq!(state.as_str(), "completed");
        assert_eq!(result.trim(), root.display().to_string());
        run(&agent).await;
        assert!(output(&agent, "monitor")
            .await
            .contains(&root.join("nested").display().to_string()));
    }
}

#[tokio::test]
async fn context_discovery_and_templates_use_the_selected_project() {
    let (_first, first) = project("first");
    let (_second, second) = project("second");
    for (marker, root) in [("first", first), ("second", second)] {
        let provider = ProjectProvider::new(tool_turn(
            "skill",
            "skill",
            serde_json::json!({"name":"cwd-test"}),
        ));
        let mut agent = build(&root, provider, &config(), None, None);
        assert!(agent
            .system
            .contains(&format!("context-{marker}: {}", root.display())));
        assert!(agent
            .subagent_defs
            .iter()
            .any(|def| def.name == "cwd-test" && def.body.contains(marker)));
        let persona = agent
            .personas
            .iter()
            .find(|persona| persona.name == "cwd-test")
            .unwrap()
            .clone();
        agent.apply_persona(&persona);
        assert!(agent
            .persona_prompt
            .as_ref()
            .unwrap()
            .contains(&format!("persona-{marker}: {}", root.display())));
        run(&agent).await;
        assert!(output(&agent, "skill")
            .await
            .contains(&format!("skill-{marker}: {}", root.display())));
    }
}

#[tokio::test]
async fn model_switch_refreshes_project_templates_and_the_child_provider() {
    let (_directory, root) = project("model-switch");
    std::fs::write(
        root.join("AGENTS.md"),
        "---\nmew: true\n---\ncontext-model: {{ model_id }}; vision: {{ supports_vision }}\n",
    )
    .unwrap();
    std::fs::write(root.join(".mew/personas/cwd-test/PERSONA.md"), "---\nname: cwd-test\ndescription: model persona\nmew:\n  template: true\n---\npersona-model: {{ model_id }}; vision: {{ supports_vision }}\n").unwrap();
    std::fs::write(root.join(".mew/skills/cwd-test/SKILL.md"), "---\nname: cwd-test\ndescription: model skill\ntemplate: true\n---\nskill-model: {{ model_id }}\n").unwrap();
    let mut cat = Catalog::empty();
    for (id, vision) in [("first", false), ("second", true)] {
        cat.models.insert(
            id.into(),
            mew_catalog::Model {
                id: id.into(),
                vision,
                ..Default::default()
            },
        );
    }
    let old = ProjectProvider::new(vec![
        FakeProvider::text_response("initial"),
        FakeProvider::text_response("old provider child"),
    ]);
    let next = ProjectProvider::new(vec![
        FakeProvider::tool_call(
            "skill",
            "model-skill",
            serde_json::json!({"name":"cwd-test"}),
        ),
        FakeProvider::tool_call(
            "subagent",
            "model-child",
            serde_json::json!({"name":"cwd-test", "prompt":"finish"}),
        ),
        FakeProvider::text_response("new provider child"),
        FakeProvider::text_response("new provider parent"),
    ]);
    let cfg = Config {
        default_persona: "cwd-test".into(),
        ..config()
    };
    let id = SessionId::new();
    let writer = SessionWriter::open_at(&root.join("sessions"), &id.to_string())
        .await
        .unwrap();
    let mut agent = build_session_agent(
        &cfg,
        Some(&cat),
        old.clone(),
        "fixture",
        "first",
        false,
        PermissionMode::Standard,
        Some(writer),
        Some(id),
        Some(root.clone()),
        Arc::new(NopDispatcher),
        None,
        &[],
    )
    .unwrap();
    assert!(agent
        .persona_prompt
        .as_ref()
        .unwrap()
        .contains("vision: false"));
    assert!(agent.system.contains("context-model: first; vision: false"));
    run(&agent).await;

    switch_session_model(
        &mut agent,
        &cfg,
        Some(&cat),
        next.clone(),
        "fixture",
        "second",
        false,
    );
    run(&agent).await;

    assert_eq!(old.requests.lock().unwrap().len(), 1);
    let requests = next.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[0]
        .system
        .contains("context-model: second; vision: true"));
    assert!(requests[0]
        .system
        .contains("persona-model: second; vision: true"));
    assert!(requests[1].messages.iter().flat_map(|message| &message.parts).any(|part| {
        matches!(part, Part::ToolCall(call) if call.call_id == "model-skill" && call.state.output().is_some_and(|output| output.contains("skill-model: second")))
    }));
    assert!(requests[3].messages.iter().flat_map(|message| &message.parts).any(|part| {
        matches!(part, Part::ToolCall(call) if call.call_id == "model-child" && call.state.output().is_some_and(|output| output.contains("new provider child")))
    }));
}

#[tokio::test]
async fn child_tools_templates_and_transcripts_inherit_the_parent_project() {
    let (_directory, root) = project("child-project");
    let sessions = root.join("sessions");
    let session_id = SessionId::new();
    let writer = mew_session::Writer::open_at(&sessions, &session_id.to_string())
        .await
        .unwrap();
    let provider = ProjectProvider::new(vec![
        FakeProvider::tool_call(
            "subagent",
            "parent-call",
            serde_json::json!({"name":"cwd-test", "prompt":"read the input and report pwd"}),
        ),
        FakeProvider::tool_call(
            "read",
            "child-read",
            serde_json::json!({"path":"input.txt"}),
        ),
        FakeProvider::tool_call("bash", "child-pwd", serde_json::json!({"command":"pwd"})),
        FakeProvider::text_response("child finished"),
        FakeProvider::text_response("parent finished"),
    ]);
    let mut cfg = config();
    cfg.workspace.roots = vec![root.clone()];
    let agent = build(
        &root,
        provider.clone(),
        &cfg,
        Some(writer),
        Some(session_id),
    );

    run(&agent).await;

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests[1]
        .system
        .contains(&format!("child-child-project: {}", root.display())));
    for (request, call_id, expected) in [
        (&requests[2], "child-read", "child-project".to_string()),
        (&requests[3], "child-pwd", root.display().to_string()),
    ] {
        assert!(request.messages.iter().flat_map(|message| &message.parts).any(|part| {
            matches!(part, Part::ToolCall(call) if call.call_id == call_id && call.state.output().is_some_and(|output| output.contains(&expected)))
        }), "child tool output must come from its parent project");
    }
    drop(requests);
    let children = std::fs::read_dir(sessions.join(session_id.to_string()).join("subagents"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    assert!(children[0].path().join("session.jsonl").exists());
}

#[tokio::test]
async fn child_workspace_escape_reaches_the_parent_permission_surface() {
    let (_project, root) = project("permissions");
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("outside.txt");
    std::fs::write(&path, "outside-approved").unwrap();
    let id = SessionId::new();
    let writer = mew_session::Writer::open_at(&root.join("sessions"), &id.to_string())
        .await
        .unwrap();
    let provider = ProjectProvider::new(vec![
        FakeProvider::tool_call(
            "subagent",
            "parent",
            serde_json::json!({"name":"cwd-test", "prompt":"read the requested file"}),
        ),
        FakeProvider::tool_call("read", "outside", serde_json::json!({"path":path})),
        FakeProvider::text_response("child done"),
        FakeProvider::text_response("parent done"),
    ]);
    let agent = build(&root, provider.clone(), &config(), Some(writer), Some(id));
    assert_eq!(
        run(&agent).await,
        1,
        "child workspace approval must be visible to its parent"
    );
    let requests = provider.requests.lock().unwrap();
    assert!(requests[2].messages.iter().flat_map(|message| &message.parts).any(|part| {
        matches!(part, Part::ToolCall(call) if call.call_id == "outside" && call.state.output().is_some_and(|output| output.contains("outside-approved")))
    }));
}

#[tokio::test]
async fn nested_child_inherits_cwd_and_forwards_permission_requests() {
    let (_project, root) = project("nested");
    std::fs::write(root.join(".mew/agents/branch.md"),
        "---\nname: branch\ndescription: nested parent\ncan_spawn: true\ntools: [subagent]\n---\nspawn cwd-test\n").unwrap();
    let id = SessionId::new();
    let writer = mew_session::Writer::open_at(&root.join("sessions"), &id.to_string())
        .await
        .unwrap();
    let provider = ProjectProvider::new(vec![
        FakeProvider::tool_call(
            "subagent",
            "parent",
            serde_json::json!({"name":"branch", "prompt":"delegate work"}),
        ),
        FakeProvider::tool_call(
            "subagent",
            "nested",
            serde_json::json!({"name":"cwd-test", "prompt":"report pwd"}),
        ),
        FakeProvider::tool_call("bash", "nested-pwd", serde_json::json!({"command":"pwd"})),
        FakeProvider::text_response("leaf done"),
        FakeProvider::text_response("branch done"),
        FakeProvider::text_response("parent done"),
    ]);
    let mut cfg = config();
    cfg.orchestration.max_subagent_depth = 2;
    let agent = build(&root, provider.clone(), &cfg, Some(writer), Some(id));
    run(&agent).await;
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(requests[3].messages.iter().flat_map(|message| &message.parts).any(|part| {
        matches!(part, Part::ToolCall(call) if call.call_id == "nested-pwd" && call.state.output().is_some_and(|output| shell_stdout(output) == root.display().to_string()))
    }), "nested child must use the same project and surface its permission request");
}

#[tokio::test]
async fn child_inherits_auto_plus_classifier_with_an_independent_cache() {
    let (_project, root) = project("auto-child");
    let id = SessionId::new();
    let writer = mew_session::Writer::open_at(&root.join("sessions"), &id.to_string())
        .await
        .unwrap();
    let provider = ProjectProvider::new(vec![
        FakeProvider::tool_call(
            "subagent",
            "parent",
            serde_json::json!({"name":"cwd-test", "prompt":"report pwd"}),
        ),
        FakeProvider::tool_call("bash", "child-pwd", serde_json::json!({"command":"pwd"})),
        FakeProvider::text_response("child done"),
        FakeProvider::text_response("parent done"),
    ]);
    let cfg = config();
    let mut agent = build(&root, provider.clone(), &cfg, Some(writer), Some(id));
    agent.set_permission_mode(PermissionMode::AutoPlus);
    let classifier = ProjectProvider::new(vec![
        FakeProvider::text_response("allow"),
        FakeProvider::text_response("allow"),
    ]);
    agent.set_classifier_provider(classifier.clone(), Some("test-classifier".into()));
    assert_eq!(
        agent
            .classify_permission(&mew_hooks::ToolCall {
                tool_name: "bash".into(),
                call_id: "parent-cache".into(),
                input: serde_json::json!({"command":"pwd"})
            })
            .await,
        Some(mew_prompts::classifier::ClassifierDecision::Allow)
    );
    wire_subagents(
        &mut agent,
        &cfg,
        None,
        "fake",
        false,
        Arc::new(NopDispatcher),
        &[],
    );

    run(&agent).await;

    let requests = provider.requests.lock().unwrap();
    assert!(requests[2].messages.iter().flat_map(|message| &message.parts).any(|part| {
        matches!(part, Part::ToolCall(call) if call.call_id == "child-pwd" && call.state.output().is_some_and(|output| shell_stdout(output) == root.display().to_string()))
    }), "child must classify and execute under the parent's AutoPlus policy");
    let classifier_requests = classifier.requests.lock().unwrap();
    assert_eq!(
        classifier_requests.len(),
        2,
        "child must not reuse the parent's cached permission decision"
    );
    assert_eq!(classifier_requests[1].model, "test-classifier");
    assert!(classifier_requests[1]
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .any(|part| {
            matches!(part, Part::Text(text) if text.text.contains(&root.display().to_string()))
        }));
}

#[tokio::test]
async fn permission_classifier_describes_the_session_project() {
    let (_directory, root) = project("classification");
    let mut agent = build(&root, ProjectProvider::new(vec![]), &config(), None, None);
    let classifier = ProjectProvider::new(vec![FakeProvider::text_response("allow")]);
    agent.set_classifier_provider(classifier.clone(), Some("fake".into()));

    let decision = agent
        .classify_permission(&mew_hooks::ToolCall {
            tool_name: "bash".into(),
            call_id: "classification".into(),
            input: serde_json::json!({"command":"pwd"}),
        })
        .await;

    assert_eq!(
        decision,
        Some(mew_prompts::classifier::ClassifierDecision::Allow)
    );
    let requests = classifier.requests.lock().unwrap();
    assert!(requests[0]
        .messages
        .iter()
        .flat_map(|message| &message.parts)
        .any(|part| {
            matches!(part, Part::Text(text) if text.text.contains(&root.display().to_string()))
        }));
}

fn shell_stdout(output: &str) -> &str {
    let (stdout, stderr) = output
        .split_once("\n--- stderr ---\n")
        .unwrap_or((output, ""));
    // Interactive bash without a terminal emits these startup warnings.
    for line in stderr.lines().filter(|line| line.starts_with("bash:")) {
        assert!(
            line.starts_with("bash: cannot set terminal process group")
                || line == "bash: no job control in this shell",
            "unexpected shell error: {line}"
        );
    }
    stdout.trim()
}
