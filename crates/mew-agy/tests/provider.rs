#![cfg(unix)]
use futures::StreamExt;
use mew_agy::{
    process::{IsolationBoundary, LaunchSpec},
    provider::{AgyProvider, Config},
};
use mew_message::{
    Finish, Message, Part, PartBase, Role, TextPart, Time, ToolResultPart, ToolState,
    ToolStateCompleted, ToolTime,
};
use mew_provider::{Provider, ProviderEvent, Request, ToolDef};
use serde_json::json;
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use ulid::Ulid;

struct FixtureBoundary {
    script: PathBuf,
}
impl IsolationBoundary for FixtureBoundary {
    fn wrap(&self, mut spec: LaunchSpec, _: &Path) -> io::Result<LaunchSpec> {
        spec.program = PathBuf::from("/usr/bin/python3");
        spec.args = vec!["-u".into(), self.script.as_os_str().into()];
        Ok(spec)
    }
}
struct Refuse;
impl IsolationBoundary for Refuse {
    fn wrap(&self, _: LaunchSpec, _: &Path) -> io::Result<LaunchSpec> {
        Err(io::Error::other("isolation unavailable"))
    }
}
fn config(parent: &Path) -> Config {
    Config {
        executable: PathBuf::from("/usr/bin/python3"),
        workspace_parent: parent.to_owned(),
        environment: vec![],
        lifetime: Duration::from_secs(20),
    }
}
fn msg(role: Role, parts: Vec<Part>) -> Message {
    Message {
        id: Ulid::new(),
        session_id: Ulid::new(),
        role,
        parts,
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    }
}
fn base() -> PartBase {
    PartBase {
        id: Ulid::new(),
        message_id: Ulid::new(),
        session_id: Ulid::new(),
    }
}
fn request() -> Request {
    Request {
        session_id: Some(Ulid::new()),
        model: "model".into(),
        messages: vec![msg(
            Role::User,
            vec![Part::Text(TextPart {
                base: base(),
                text: "hello".into(),
                synthetic: false,
            })],
        )],
        tools: vec![ToolDef {
            name: "echo".into(),
            description: "echo".into(),
            schema: json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
        }],
        ..Default::default()
    }
}
fn fixture(dir: &Path, script: &str) -> FixtureBoundary {
    let path = dir.join("fake.py");
    std::fs::write(&path, script).unwrap();
    FixtureBoundary { script: path }
}

#[tokio::test]
async fn auxiliary_sampling_hints_are_advisory_but_regular_turn_controls_are_rejected() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json
sys.stdin.readline()
print(json.dumps({'event':'step_update','step_update':{'step_type':'agent_response','text_delta':'ALLOW'}}),flush=True)
print(json.dumps({'event':'result','result':{'status':'SUCCESS'}}),flush=True)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
    let mut auxiliary = request();
    auxiliary.session_id = None;
    auxiliary.tools.clear();
    auxiliary.params = Some(mew_provider::ChatParams {
        temperature: Some(0.0),
        max_tokens: Some(8),
        ..Default::default()
    });
    let mut disabled = serde_json::Map::new();
    disabled.insert("type".into(), "disabled".into());
    auxiliary.reasoning = Some(mew_provider::ReasoningConfig { params: disabled });
    assert!(provider.stream(auxiliary.clone()).await.is_err());
    let events: Vec<_> = provider
        .stream_auxiliary(auxiliary.clone())
        .await
        .unwrap()
        .collect()
        .await;
    assert!(events.iter().any(|event| matches!(event, ProviderEvent::PartDelta { field: "text", delta, .. } if delta == "ALLOW")));
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            ..
        })
    ));

    auxiliary.params.as_mut().unwrap().top_p = Some(0.5);
    assert!(provider.stream_auxiliary(auxiliary.clone()).await.is_err());
    auxiliary.params.as_mut().unwrap().top_p = None;
    auxiliary.session_id = Some(Ulid::new());
    assert!(provider.stream_auxiliary(auxiliary).await.is_err());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires a signed-in local AGY installation and network"]
async fn official_agy_responds_through_local_sandbox() {
    use mew_agy::{proxy::EgressProxy, sandbox::SeatbeltBoundary};
    let binary = PathBuf::from(std::env::var_os("MEW_AGY_TEST_BIN").expect("agy binary"));
    let workspace = tempfile::tempdir().unwrap();
    let proxy = EgressProxy::start().await.unwrap();
    let boundary = SeatbeltBoundary::new(proxy.port()).unwrap();
    let provider = AgyProvider::new(
        Config {
            executable: binary,
            workspace_parent: workspace.path().to_owned(),
            environment: vec![],
            lifetime: Duration::from_secs(120),
        },
        boundary,
    );
    let mut req = request();
    req.model = std::env::var("MEW_AGY_TEST_MODEL").expect("test model");
    req.tools.clear();
    let events: Vec<_> = tokio::time::timeout(Duration::from_secs(120), async {
        provider.stream(req).await.unwrap().collect().await
    })
    .await
    .expect("AGY prompt timed out");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::PartDelta { field: "text", .. })),
        "no text: {events:?}"
    );
    assert!(
        matches!(
            events.last(),
            Some(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            })
        ),
        "no successful completion: {events:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires signed-in local AGY, network, and explicit unsandboxed execution"]
async fn official_agy_responds_with_existing_local_account() {
    use mew_agy::process::UnsandboxedBoundary;
    let binary = PathBuf::from(std::env::var_os("MEW_AGY_TEST_BIN").expect("agy binary"));
    let workspace = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(
        Config {
            executable: binary,
            workspace_parent: workspace.path().to_owned(),
            environment: vec![],
            lifetime: Duration::from_secs(180),
        },
        UnsandboxedBoundary,
    );
    let mut req = request();
    req.model = std::env::var("MEW_AGY_TEST_MODEL").expect("test model");
    req.tools.clear();
    let events: Vec<_> = tokio::time::timeout(Duration::from_secs(180), async {
        provider.stream(req).await.unwrap().collect().await
    })
    .await
    .expect("AGY prompt timed out");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::PartDelta { field: "text", .. })),
        "no text: {events:?}"
    );
    assert!(
        matches!(
            events.last(),
            Some(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            })
        ),
        "no successful completion: {events:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires signed-in local AGY, network, and explicit unsandboxed execution"]
async fn official_agy_calls_client_tool_and_receives_its_result() {
    use mew_agy::process::UnsandboxedBoundary;
    let binary = PathBuf::from(std::env::var_os("MEW_AGY_TEST_BIN").expect("agy binary"));
    let workspace = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(
        Config {
            executable: binary,
            workspace_parent: workspace.path().to_owned(),
            environment: vec![],
            lifetime: Duration::from_secs(180),
        },
        UnsandboxedBoundary,
    );
    let mut req = request();
    req.model = std::env::var("MEW_AGY_TEST_MODEL").expect("test model");
    if let Part::Text(text) = &mut req.messages[0].parts[0] {
        text.text = "Call the echo tool with text 'mew-bridge-canary'. After receiving its result, report the tool result.".into();
    }
    let events: Vec<_> = tokio::time::timeout(Duration::from_secs(120), async {
        provider.stream(req.clone()).await.unwrap().collect().await
    })
    .await
    .expect("AGY tool call timed out");
    let call = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::PartStart {
                part: Part::ToolCall(call),
            } => Some(call.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("AGY did not call the client tool: {events:?}"));
    assert_eq!(call.tool_name, "echo");
    let input = match &call.state {
        ToolState::Pending(state) => state.input.clone(),
        _ => panic!("unexpected call state"),
    };
    assert_eq!(input["text"], "mew-bridge-canary");
    let mut parts = Vec::new();
    let mut text = String::new();
    for event in &events {
        if let ProviderEvent::PartDelta {
            field: "text",
            delta,
            ..
        } = event
        {
            text.push_str(delta);
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(TextPart {
            base: base(),
            text,
            synthetic: false,
        }));
    }
    let mut done = call.clone();
    done.state = ToolState::Completed(ToolStateCompleted {
        input,
        output: "mew-tool-result-canary".into(),
        metadata: None,
        diff: None,
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    parts.push(Part::ToolCall(done));
    let mut continuation = req;
    continuation.messages.push(msg(Role::Assistant, parts));
    continuation.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id,
        })],
    ));
    let final_events: Vec<_> = tokio::time::timeout(Duration::from_secs(120), async {
        provider.stream(continuation).await.unwrap().collect().await
    })
    .await
    .expect("AGY tool continuation timed out");
    let text: String = final_events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::PartDelta {
                field: "text",
                delta,
                ..
            } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        text.contains("mew-tool-result-canary"),
        "tool result not reflected in final response"
    );
    assert!(matches!(
        final_events.last(),
        Some(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            ..
        })
    ));
}

#[tokio::test]
async fn provider_reports_safe_quota_errors_without_disclosing_cli_details() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json
sys.stdin.readline()
print(json.dumps({'event':'result','result':{'status':'ERROR','error':'quota exhausted secret-canary'}}),flush=True)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
    let events: Vec<_> = provider.stream(request()).await.unwrap().collect().await;
    let error = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Error(error) => Some(&error.message),
            _ => None,
        })
        .expect("provider must report the error");
    assert!(error.contains("quota"));
    assert!(!error.contains("secret-canary"));
}
const TOOL_SCRIPT: &str = r#"import sys,json,urllib.request
line=sys.stdin.readline()
with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
def post(body,sid=None):
 h={'Content-Type':'application/json'}
 if sid: h['mcp-session-id']=sid
 with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers, r.read()
h,b=post({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'fake','version':'1'}}})
sid=h['mcp-session-id']
post({'jsonrpc':'2.0','method':'notifications/initialized'},sid)
h,b=post({'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'echo','arguments':{'text':'hello'}}},sid)
r=json.loads(b)
print(json.dumps({'event':'step_update','step_update':{'step_type':'agent_response','text_delta':r['result']['content'][0]['text']}}),flush=True)
print(json.dumps({'event':'result','result':{'status':'SUCCESS'}}),flush=True)
"#;

#[tokio::test]
async fn long_mcp_name_is_aliased_for_agy_and_restored_for_mew() {
    let parent = tempfile::tempdir().unwrap();
    let script = TOOL_SCRIPT.replace("'name':'echo'", "'name':'mew_tool_0'");
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), &script));
    let mut req = request();
    let name = format!("mcp__server__{}", "long_name_".repeat(10));
    req.tools[0].name = name.clone();
    let events: Vec<_> = provider.stream(req).await.unwrap().collect().await;
    let call = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::PartStart {
                part: Part::ToolCall(call),
            } => Some(call),
            _ => None,
        })
        .expect("tool call");
    assert_eq!(call.tool_name, name);
}

#[tokio::test]
async fn generated_alias_does_not_shadow_a_real_tool_name() {
    let parent = tempfile::tempdir().unwrap();
    let script = TOOL_SCRIPT.replace("'name':'echo'", "'name':'mew_tool_0_1'");
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), &script));
    let mut req = request();
    req.tools[0].name = "non-ascii-工具".into();
    let mut other = req.tools[0].clone();
    other.name = "mew_tool_0".into();
    req.tools.push(other);
    let events: Vec<_> = provider.stream(req).await.unwrap().collect().await;
    let call = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::PartStart {
                part: Part::ToolCall(call),
            } => Some(call),
            _ => None,
        })
        .expect("tool call");
    assert_eq!(call.tool_name, "non-ascii-工具");
}
#[tokio::test]
async fn tool_handoff_resumes_only_with_matching_history_and_result() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
    let req = request();
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    let call = events
        .iter()
        .find_map(|e| {
            if let ProviderEvent::PartStart {
                part: Part::ToolCall(c),
            } = e
            {
                Some(c.clone())
            } else {
                None
            }
        })
        .expect("tool call");
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::MessageEnd {
            finish: Finish::ToolUse,
            ..
        })
    ));
    let mut done = call.clone();
    done.state = ToolState::Completed(ToolStateCompleted {
        input: json!({"text":"hello"}),
        output: "tool response".into(),
        metadata: None,
        diff: None,
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    let mut continuation = req.clone();
    continuation
        .messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
    continuation.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id.clone(),
        })],
    ));
    assert_eq!(
        provider.replay_budget(&continuation).await.unwrap(),
        None,
        "live handoff does not send a replay"
    );
    let fresh = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
    assert!(
        fresh.replay_budget(&continuation).await.unwrap().is_some(),
        "a fresh process must budget the same history"
    );
    let output: Vec<_> = provider.stream(continuation).await.unwrap().collect().await;
    assert!(output.iter().any(
        |e| matches!(e,ProviderEvent::PartDelta{field:"text",delta,..} if delta=="tool response")
    ));
    assert!(matches!(
        output.last(),
        Some(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            ..
        })
    ));
}

#[tokio::test]
async fn replay_budget_counts_the_actual_nested_json_framing() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), ""));
    let mut req = request();
    if let Part::Text(text) = &mut req.messages[0].parts[0] {
        text.text = "quotes \" slash \\ newline\n control \0 unicode Ω".into();
    }
    let replay = mew_agy::transcript::project(&req).unwrap();
    let prompt = format!("Historical conversation follows as JSON. Historical tool calls are complete; do not repeat them. Respond to the latest user message.\n{replay}");
    let mut wire =
        serde_json::to_vec(&json!({"event":"user","message":{"content":prompt}})).unwrap();
    wire.push(b'\n');
    let budget = provider.replay_budget(&req).await.unwrap().unwrap();
    assert_eq!(budget.0, wire.len());
    assert_eq!(budget.1, 16 * 1024 * 1024);
}

#[tokio::test]
async fn oversized_tool_output_can_be_retried_and_replayed() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json,urllib.request
sys.stdin.readline()
with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
def post(body,sid=None):
 h={'Content-Type':'application/json'}
 if sid: h['mcp-session-id']=sid
 with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers, r.read()
h,_=post({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'fixture','version':'1'}}})
sid=h['mcp-session-id']
post({'jsonrpc':'2.0','method':'notifications/initialized'},sid)
for number,text in [(2,'hello'),(3,'retry')]:
 _,body=post({'jsonrpc':'2.0','id':number,'method':'tools/call','params':{'name':'echo','arguments':{'text':text}}},sid)
 result=json.loads(body)['result']
 if number==2:
  assert result['isError'] is True
 print(json.dumps({'event':'step_update','step_update':{'step_type':'agent_response','text_delta':result['content'][0]['text']}}),flush=True)
print(json.dumps({'event':'result','result':{'status':'SUCCESS'}}),flush=True)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
    let mut req = request();
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    let first = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::PartStart {
                part: Part::ToolCall(call),
            } => Some(call.clone()),
            _ => None,
        })
        .expect("first tool call");
    let mut complete = first.clone();
    complete.state = ToolState::Completed(ToolStateCompleted {
        input: json!({"text":"hello"}),
        output: "x".repeat(1_048_577),
        metadata: None,
        diff: None,
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    req.messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(complete)]));
    req.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: first.call_id,
        })],
    ));
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    let recovery: String = events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::PartDelta {
                field: "text",
                delta,
                ..
            } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert!(recovery.contains("offset") && recovery.contains("limit"));
    let second = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::PartStart {
                part: Part::ToolCall(call),
            } => Some(call.clone()),
            _ => None,
        })
        .expect("retry tool call");
    let mut complete = second.clone();
    complete.state = ToolState::Completed(ToolStateCompleted {
        input: json!({"text":"retry"}),
        output: "small result".into(),
        metadata: None,
        diff: None,
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    req.messages.push(msg(
        Role::Assistant,
        vec![
            Part::Text(TextPart {
                base: base(),
                text: recovery,
                synthetic: false,
            }),
            Part::ToolCall(complete),
        ],
    ));
    req.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: second.call_id,
        })],
    ));
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    assert!(events.iter().any(|event| matches!(event, ProviderEvent::PartDelta { field:"text", delta, .. } if delta == "small result")));
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            ..
        })
    ));

    // The same original large result must also remain replayable on the next turn.
    req.messages.push(msg(
        Role::Assistant,
        vec![Part::Text(TextPart {
            base: base(),
            text: "small result".into(),
            synthetic: false,
        })],
    ));
    req.messages.push(msg(
        Role::User,
        vec![Part::Text(TextPart {
            base: base(),
            text: "continue".into(),
            synthetic: false,
        })],
    ));
    let events: Vec<_> = provider.stream(req).await.unwrap().collect().await;
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::PartStart {
            part: Part::ToolCall(_)
        }
    )));
}
#[tokio::test]
async fn unavailable_isolation_fails_before_launch() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), Refuse);
    assert!(provider.stream(request()).await.is_err());
}
#[tokio::test]
async fn auxiliary_requests_cannot_emit_unrecoverable_tool_calls() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
    let mut req = request();
    req.session_id = None;
    assert!(provider.stream(req).await.is_err());
}

#[tokio::test]
async fn mismatched_continuation_poisoned_session_cannot_replay_an_action() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
    let req = request();
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    let call = events
        .iter()
        .find_map(|e| {
            if let ProviderEvent::PartStart {
                part: Part::ToolCall(c),
            } = e
            {
                Some(c.clone())
            } else {
                None
            }
        })
        .unwrap();
    let mut done = call.clone();
    done.state = ToolState::Completed(ToolStateCompleted {
        input: json!({"text":"hello"}),
        output: "done".into(),
        metadata: None,
        diff: None,
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    let mut good = req.clone();
    good.messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
    good.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id,
        })],
    ));
    let mut bad = good.clone();
    if let Part::ToolResult(result) = &mut bad.messages.last_mut().unwrap().parts[0] {
        result.call_id = "orphan".into();
    };
    assert!(provider.stream(bad).await.is_err());
    assert!(
        provider.stream(good).await.is_err(),
        "the failed handoff must not replay as a new process"
    );
}

#[tokio::test]
async fn dropping_before_terminal_tool_event_cancels_the_owned_process() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
    let mut stream = provider.stream(request()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = stream.next().await {
            if matches!(
                event,
                ProviderEvent::PartStart {
                    part: Part::ToolCall(_),
                    ..
                }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    drop(stream);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = std::fs::read_dir(parent.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with("mew-agy-"))
                .count();
            if count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelled_owning_turn_cleans_up_the_process_after_handoff() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
    let req = request();
    let cancel = req.cancel_token.clone();
    let events: Vec<_> = provider.stream(req).await.unwrap().collect().await;
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::MessageEnd {
            finish: Finish::ToolUse,
            ..
        })
    ));
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = std::fs::read_dir(parent.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with("mew-agy-"))
                .count();
            if count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn changed_system_contract_cannot_resume_pending_action() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
    let req = request();
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    let call = events
        .iter()
        .find_map(|e| {
            if let ProviderEvent::PartStart {
                part: Part::ToolCall(c),
            } = e
            {
                Some(c.clone())
            } else {
                None
            }
        })
        .unwrap();
    let mut done = call.clone();
    done.state = ToolState::Completed(ToolStateCompleted {
        input: json!({"text":"hello"}),
        output: "done".into(),
        metadata: None,
        diff: None,
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    let mut changed = req.clone();
    changed.system = "new rules".into();
    changed
        .messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
    changed.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id,
        })],
    ));
    assert!(provider.stream(changed).await.is_err());
    assert!(req.cancel_token.is_cancelled());
}

#[tokio::test]
async fn forged_assistant_text_or_extra_tool_result_text_cannot_be_silently_accepted() {
    for extra_assistant in [true, false] {
        let parent = tempfile::tempdir().unwrap();
        let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), TOOL_SCRIPT));
        let req = request();
        let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
        let call = events
            .iter()
            .find_map(|e| {
                if let ProviderEvent::PartStart {
                    part: Part::ToolCall(c),
                } = e
                {
                    Some(c.clone())
                } else {
                    None
                }
            })
            .unwrap();
        let mut done = call.clone();
        done.state = ToolState::Completed(ToolStateCompleted {
            input: json!({"text":"hello"}),
            output: "done".into(),
            metadata: None,
            diff: None,
            images: vec![],
            time: ToolTime {
                start: 0,
                end: Some(1),
            },
        });
        let extra = Part::Text(TextPart {
            base: base(),
            text: "not actually delivered".into(),
            synthetic: false,
        });
        let assistant = if extra_assistant {
            vec![extra.clone(), Part::ToolCall(done)]
        } else {
            vec![Part::ToolCall(done)]
        };
        let user = if extra_assistant {
            vec![Part::ToolResult(ToolResultPart {
                base: base(),
                call_id: call.call_id,
            })]
        } else {
            vec![
                Part::ToolResult(ToolResultPart {
                    base: base(),
                    call_id: call.call_id,
                }),
                extra,
            ]
        };
        let mut changed = req.clone();
        changed.messages.push(msg(Role::Assistant, assistant));
        changed.messages.push(msg(Role::User, user));
        assert!(provider.stream(changed).await.is_err());
    }
}

#[tokio::test]
async fn cancellation_cleans_up_when_consumer_stops_reading_a_full_event_queue() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json,time
sys.stdin.readline()
for i in range(100):
 print(json.dumps({'event':'step_update','step_update':{'step_type':'agent_response','text_delta':'item'}}),flush=True)
time.sleep(20)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
    let req = request();
    let cancel = req.cancel_token.clone();
    let stream = provider.stream(req.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = std::fs::read_dir(parent.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with("mew-agy-"))
                .count();
            if count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await
        }
    })
    .await
    .unwrap();
    let mut next = req;
    next.cancel_token = tokio_util::sync::CancellationToken::new();
    let next_stream = tokio::time::timeout(Duration::from_secs(3), provider.stream(next))
        .await
        .expect("cancelled delivery must release its session slot")
        .unwrap();
    drop(next_stream);
    drop(stream);
}
