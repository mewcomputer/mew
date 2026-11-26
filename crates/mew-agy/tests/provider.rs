#![cfg(unix)]
use futures::StreamExt;
use mew_agy::{
    process::{IsolationBoundary, LaunchSpec},
    provider::{AgyProvider, Config},
};
use mew_message::{
    FilePart, Finish, Message, Part, PartBase, Role, TextPart, Time, ToolImage, ToolResultPart,
    ToolState, ToolStateCompleted, ToolTime,
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
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
sys.stdin.readline()
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'ALLOW'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':5,'result':{'stopReason':'end_turn'}}),flush=True)
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
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
sys.stdin.readline()
print(json.dumps({'jsonrpc':'2.0','id':5,'error':{'code':-32001,'message':'quota exhausted secret-canary'}}),flush=True)
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

#[tokio::test]
async fn provider_reports_auth_error_pointing_to_login() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json
json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':1,'result':{'protocolVersion':1}}),flush=True)
json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':2,'result':{}}),flush=True)
json.loads(sys.stdin.readline())
print(json.dumps({'jsonrpc':'2.0','id':3,'error':{'code':-32000,'message':'Authentication required'}}),flush=True)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
    let events: Vec<_> = provider.stream(request()).await.unwrap().collect().await;
    let error = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Error(error) => Some(error.message.as_str()),
            _ => None,
        })
        .expect("provider must report auth error");
    assert!(error.contains("mew auth login antigravity"), "{error}");
}

const TOOL_SCRIPT: &str = r#"import sys,json,urllib.request
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
line=sys.stdin.readline()
with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
def post(body,sid=None):
 h={'Content-Type':'application/json'}
 if sid: h['mcp-session-id']=sid
 with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers, r.read()
_,_=post({'jsonrpc':'2.0','id':0,'method':'server/discover','params':{}})
h,b=post({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'fake','version':'1'}}})
sid=h['mcp-session-id']
post({'jsonrpc':'2.0','method':'notifications/initialized'},sid)
print(json.dumps({'jsonrpc':'2.0','id':'p1','method':'session/request_permission','params':{'sessionId':'s1','toolCall':{'name':'call_mcp_tool','args':{'ServerName':'mew','ToolName':'echo'}},'options':[{'optionId':'allow','kind':'allow_once'},{'optionId':'deny','kind':'reject_once'}]}}),flush=True)
perm=json.loads(sys.stdin.readline())
assert perm['result']['outcome']=={'outcome':'selected','optionId':'allow'}
h,b=post({'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'echo','arguments':{'text':'hello'}}},sid)
r=json.loads(b)
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':r['result']['content'][0]['text']}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':5,'result':{'stopReason':'end_turn'}}),flush=True)
"#;

#[tokio::test]
async fn long_mcp_name_is_aliased_for_agy_and_restored_for_mew() {
    let parent = tempfile::tempdir().unwrap();
    let script = TOOL_SCRIPT.replace("'echo'", "'mew_tool_0'");
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
    let script = TOOL_SCRIPT.replace("'echo'", "'mew_tool_0_1'");
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
async fn abandon_pending_cleans_up_handoff_without_cancelling_turn_token_and_allows_next_turn() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json,urllib.request
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
prompt_msg=json.loads(sys.stdin.readline())
prompt_text='\n'.join(b.get('text','') for b in prompt_msg['params']['prompt'] if isinstance(b,dict))
if 'after max turns' in prompt_text:
 print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'resumed cleanly'}}}}),flush=True)
 print(json.dumps({'jsonrpc':'2.0','id':5,'result':{'stopReason':'end_turn'}}),flush=True)
else:
 with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
 def post(body,sid=None):
  h={'Content-Type':'application/json'}
  if sid: h['mcp-session-id']=sid
  with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers, r.read()
 h,_=post({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'fake','version':'1'}}})
 sid=h['mcp-session-id']
 post({'jsonrpc':'2.0','method':'notifications/initialized'},sid)
 post({'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'echo','arguments':{'text':'hello'}}},sid)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
    let mut req = request();
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

    provider.abandon_pending(req.session_id.unwrap()).await;
    assert!(
        !req.cancel_token.is_cancelled(),
        "abandoning an open handoff on clean turn exit must not cancel the caller turn token"
    );
    let leftover = std::fs::read_dir(parent.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("mew-agy-"))
        .count();
    assert_eq!(
        leftover, 0,
        "abandoned process workspace must be cleaned up"
    );

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
    req.messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
    req.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id,
        })],
    ));
    req.messages.push(msg(
        Role::User,
        vec![Part::Text(TextPart {
            base: base(),
            text: "after max turns".into(),
            synthetic: false,
        })],
    ));
    assert!(
        provider.replay_budget(&req).await.unwrap().is_some(),
        "next user turn after abandon must measure full replay"
    );
    let output: Vec<_> = provider.stream(req).await.unwrap().collect().await;
    assert!(output.iter().any(
        |e| matches!(e, ProviderEvent::PartDelta { field: "text", delta, .. } if delta == "resumed cleanly")
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
async fn replay_budget_caps_text_block_below_silent_truncation_limit() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), ""));
    let mut req = request();
    if let Part::Text(text) = &mut req.messages[0].parts[0] {
        text.text = "quotes \" slash \\ newline\n control \0 unicode Ω".into();
    }
    let prompt = mew_agy::transcript::prompt_blocks(&req).unwrap();
    let expected_text = prompt[0]["text"].as_str().unwrap().len();
    let budget = provider.replay_budget(&req).await.unwrap().unwrap();
    assert_eq!(
        budget,
        (expected_text, mew_agy::transcript::MAX_REPLAY_TEXT)
    );

    let mut oversized = request();
    if let Part::Text(text) = &mut oversized.messages[0].parts[0] {
        text.text = "x".repeat(mew_agy::transcript::MAX_REPLAY_TEXT + 16 * 1024);
    }
    let over_budget = provider.replay_budget(&oversized).await.unwrap().unwrap();
    assert_eq!(over_budget.1, mew_agy::transcript::MAX_REPLAY_TEXT);
    assert!(over_budget.0 > over_budget.1);
}

#[tokio::test]
async fn stream_and_auxiliary_reject_oversized_text_replay_before_spawning() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(
        config(parent.path()),
        fixture(parent.path(), "raise SystemExit(1)"),
    );

    let mut req = request();
    if let Part::Text(text) = &mut req.messages[0].parts[0] {
        text.text = "x".repeat(mew_agy::transcript::MAX_REPLAY_TEXT + 1024);
    }
    let stream_err = match provider.stream(req.clone()).await {
        Ok(_) => panic!("oversized text replay should be rejected before spawning"),
        Err(error) => error,
    };
    assert!(stream_err.to_string().contains("text limit"));

    req.session_id = None;
    req.tools.clear();
    let aux_err = match provider.stream_auxiliary(req).await {
        Ok(_) => panic!("oversized auxiliary text replay should be rejected before spawning"),
        Err(error) => error,
    };
    assert!(aux_err.to_string().contains("text limit"));
}

#[tokio::test]
async fn replay_budget_counts_nested_json_framing_when_attachments_dominate() {
    let parent = tempfile::tempdir().unwrap();
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), ""));
    let mut req = request();
    req.messages[0].parts.push(Part::File(FilePart {
        base: base(),
        mime: "image/png".into(),
        filename: Some("large.png".into()),
        url: format!("data:image/png;base64,{}", "AAAA".repeat(6_000)),
    }));
    let prompt = mew_agy::transcript::prompt_blocks(&req).unwrap();
    let mut wire = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "session/prompt",
        "params": {
            "sessionId": "00000000-0000-0000-0000-000000000000",
            "prompt": prompt,
        }
    }))
    .unwrap();
    wire.push(b'\n');
    let budget = provider.replay_budget(&req).await.unwrap().unwrap();
    assert_eq!(budget, (wire.len(), 16 * 1024 * 1024));
}

#[tokio::test]
async fn oversized_tool_output_can_be_retried_and_replayed() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json,urllib.request
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
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
 print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':result['content'][0]['text']}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':5,'result':{'stopReason':'end_turn'}}),flush=True)
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
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
sys.stdin.readline()
for i in range(100):
 print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'item'}}}}),flush=True)
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

#[tokio::test]
async fn user_image_and_resource_attachments_reach_acp_session_prompt() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
prompt_msg=json.loads(sys.stdin.readline())
blocks=prompt_msg['params']['prompt']
assert blocks[1]=={'type':'image','mimeType':'image/png','data':'iVBORw0KGgo='}, blocks
assert blocks[2]['type']=='resource' and blocks[2]['resource']['text']=='note body', blocks
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'IMG-OK'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':5,'result':{'stopReason':'end_turn'}}),flush=True)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
    let mut req = request();
    req.messages[0].parts.push(Part::File(FilePart {
        base: base(),
        mime: "image/png".into(),
        filename: Some("pixel.png".into()),
        url: "data:image/png;base64,iVBORw0KGgo=".into(),
    }));
    req.messages[0].parts.push(Part::File(FilePart {
        base: base(),
        mime: "text/plain".into(),
        filename: Some("note.txt".into()),
        url: "data:text/plain,note body".into(),
    }));
    let events: Vec<_> = provider.stream(req).await.unwrap().collect().await;
    assert!(events.iter().any(
        |e| matches!(e, ProviderEvent::PartDelta { field: "text", delta, .. } if delta == "IMG-OK")
    ));
}

#[tokio::test]
async fn tool_result_images_and_foreign_permission_denial_work_end_to_end() {
    let parent = tempfile::tempdir().unwrap();
    let script = r#"import sys,json,urllib.request
for i,r in [(1,{'protocolVersion':1}),(2,{}),(3,{'sessionId':'s1'}),(4,{})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':r}),flush=True)
sys.stdin.readline()
print(json.dumps({'jsonrpc':'2.0','id':'deny-1','method':'session/request_permission','params':{'sessionId':'s1','toolCall':{'name':'run_command','args':{'CommandLine':'id'}},'options':[{'optionId':'allow','kind':'allow_once'},{'optionId':'deny','kind':'reject_once'}]}}),flush=True)
denied=json.loads(sys.stdin.readline())
assert denied['result']['outcome']=={'outcome':'selected','optionId':'deny'}, denied
print(json.dumps({'jsonrpc':'2.0','id':'allow-mew-typo','method':'session/request_permission','params':{'sessionId':'s1','toolCall':{'name':'call_mcp_tool','args':{'ServerName':'mew','ToolName':'read_file'}},'options':[{'optionId':'allow','kind':'allow_once'},{'optionId':'deny','kind':'reject_once'}]}}),flush=True)
allowed=json.loads(sys.stdin.readline())
assert allowed['result']['outcome']=={'outcome':'selected','optionId':'allow'}, allowed
with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
def post(body,sid=None):
 h={'Content-Type':'application/json'}
 if sid: h['mcp-session-id']=sid
 with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers, r.read()
h,_=post({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'fake','version':'1'}}})
sid=h['mcp-session-id']
post({'jsonrpc':'2.0','method':'notifications/initialized'},sid)
_,bad_b=post({'jsonrpc':'2.0','id':99,'method':'tools/call','params':{'name':'read_file','arguments':{}}},sid)
bad=json.loads(bad_b)
assert bad['error']['code']==-32602 and 'unknown tool' in bad['error']['message'], bad
_,b=post({'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'echo','arguments':{'text':'shot'}}},sid)
res=json.loads(b)['result']
assert res['content'][1]=={'type':'image','data':'c2hvdA==','mimeType':'image/png'}, res
print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'TOOL-IMG-OK'}}}}),flush=True)
print(json.dumps({'jsonrpc':'2.0','id':5,'result':{'stopReason':'end_turn'}}),flush=True)
"#;
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), script));
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
    let mut done = call.clone();
    done.state = ToolState::Completed(ToolStateCompleted {
        input: json!({"text": "shot"}),
        output: "here is the image".into(),
        metadata: None,
        diff: None,
        images: vec![ToolImage {
            mime: "image/png".into(),
            data: "c2hvdA==".into(),
        }],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    let mut continuation = req;
    continuation
        .messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
    continuation.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id,
        })],
    ));
    let output: Vec<_> = provider.stream(continuation).await.unwrap().collect().await;
    assert!(output.iter().any(|e| matches!(
        e,
        ProviderEvent::PartDelta { field: "text", delta, .. } if delta == "TOOL-IMG-OK"
    )));
}

#[tokio::test]
async fn persistent_session_reuses_process_and_sends_delta_prompt_across_turns() {
    let parent = tempfile::tempdir().unwrap();
    let count_path = parent.path().join("spawns.txt");
    let log_path = parent.path().join("prompts.jsonl");
    let script = format!(
        r#"import sys,json,urllib.request
count_path={:?}
log_path={:?}
with open(count_path,'a') as f: f.write('1\n')
def read_msg():
 line=sys.stdin.readline()
 if not line: sys.exit(0)
 return json.loads(line)
for i,r in [(1,{{'protocolVersion':1}}),(2,{{}}),(3,{{'sessionId':'s-persist'}}),(4,{{}})]:
 m=read_msg()
 print(json.dumps({{'jsonrpc':'2.0','id':m['id'],'result':r}}),flush=True)
with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
def post(body,sid=None):
 h={{'Content-Type':'application/json'}}
 if sid: h['mcp-session-id']=sid
 with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers, r.read()
# Turn 1: full replay prompt, triggers MCP tool call -> handoff -> end_turn
p1=read_msg()
with open(log_path,'a') as f: f.write(json.dumps({{'turn':1,'method':p1['method'],'prompt':p1['params']['prompt']}})+'\n')
h,_=post({{'jsonrpc':'2.0','id':1,'method':'initialize','params':{{'protocolVersion':'2025-11-25','capabilities':{{}},'clientInfo':{{'name':'fake','version':'1'}}}}}})
sid=h['mcp-session-id']
post({{'jsonrpc':'2.0','method':'notifications/initialized'}},sid)
_,b=post({{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{{'name':'echo','arguments':{{'text':'big'}}}}}},sid)
res=json.loads(b)['result']
print(json.dumps({{'jsonrpc':'2.0','method':'session/update','params':{{'sessionId':'s-persist','update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':'t1-ok'}}}}}}}}),flush=True)
print(json.dumps({{'jsonrpc':'2.0','id':p1['id'],'result':{{'stopReason':'end_turn'}}}}),flush=True)
# Turn 2: model switched -> set_model then delta prompt on SAME process
m2=read_msg()
if m2['method']=='session/close': sys.exit(0)
assert m2['method']=='session/set_model' and m2['params']['modelId']=='model-b', m2
print(json.dumps({{'jsonrpc':'2.0','id':m2['id'],'result':{{}}}}),flush=True)
p2=read_msg()
with open(log_path,'a') as f: f.write(json.dumps({{'turn':2,'method':p2['method'],'prompt':p2['params']['prompt']}})+'\n')
print(json.dumps({{'jsonrpc':'2.0','method':'session/update','params':{{'sessionId':'s-persist','update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':'t2-ok'}}}}}}}}),flush=True)
print(json.dumps({{'jsonrpc':'2.0','id':p2['id'],'result':{{'stopReason':'end_turn'}}}}),flush=True)
# Turn 3: attachment follow-up on SAME process
p3=read_msg()
with open(log_path,'a') as f: f.write(json.dumps({{'turn':3,'method':p3['method'],'prompt':p3['params']['prompt']}})+'\n')
print(json.dumps({{'jsonrpc':'2.0','method':'session/update','params':{{'sessionId':'s-persist','update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':'t3-ok'}}}}}}}}),flush=True)
print(json.dumps({{'jsonrpc':'2.0','id':p3['id'],'result':{{'stopReason':'end_turn'}}}}),flush=True)
# Expect session/close when system prompt changes on Turn 4
c=read_msg()
"#,
        count_path, log_path
    );
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), &script));
    let mut req = request();
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    let call = events
        .iter()
        .find_map(|e| match e {
            ProviderEvent::PartStart {
                part: Part::ToolCall(c),
            } => Some(c.clone()),
            _ => None,
        })
        .expect("tool call");
    let huge_output = "x".repeat(420 * 1024);
    let mut done = call.clone();
    done.state = ToolState::Completed(ToolStateCompleted {
        input: json!({"text": "big"}),
        output: huge_output.clone(),
        metadata: None,
        diff: None,
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    req.messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
    req.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id,
        })],
    ));
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    assert!(events.iter().any(
        |e| matches!(e, ProviderEvent::PartDelta { field: "text", delta, .. } if delta == "t1-ok")
    ));
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            ..
        })
    ));

    // Turn 2: model switch + short follow-up must keep same ACP session & tiny delta budget!
    req.model = "model-b".into();
    req.messages.push(msg(
        Role::Assistant,
        vec![Part::Text(TextPart {
            base: base(),
            text: "t1-ok".into(),
            synthetic: false,
        })],
    ));
    req.messages.push(msg(
        Role::User,
        vec![Part::Text(TextPart {
            base: base(),
            text: "follow-up 2".into(),
            synthetic: false,
        })],
    ));
    let (budget, limit) = provider.replay_budget(&req).await.unwrap().unwrap();
    assert_eq!(limit, mew_agy::transcript::MAX_REPLAY_TEXT);
    assert!(
        budget < 1024,
        "delta budget should ignore 420KB tool output from Turn 1, got {budget}"
    );
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    assert!(events.iter().any(
        |e| matches!(e, ProviderEvent::PartDelta { field: "text", delta, .. } if delta == "t2-ok")
    ));

    // Turn 3: follow-up with attachment on same live process
    req.messages.push(msg(
        Role::Assistant,
        vec![Part::Text(TextPart {
            base: base(),
            text: "t2-ok".into(),
            synthetic: false,
        })],
    ));
    req.messages.push(msg(
        Role::User,
        vec![
            Part::Text(TextPart {
                base: base(),
                text: "follow-up 3".into(),
                synthetic: false,
            }),
            Part::File(FilePart {
                base: base(),
                mime: "image/png".into(),
                filename: Some("p.png".into()),
                url: "data:image/png;base64,iVBORw0KGgo=".into(),
            }),
        ],
    ));
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    assert!(events.iter().any(
        |e| matches!(e, ProviderEvent::PartDelta { field: "text", delta, .. } if delta == "t3-ok")
    ));

    // Only 1 process spawned across Turns 1..=3!
    let spawns = std::fs::read_to_string(&count_path)
        .unwrap()
        .lines()
        .count();
    assert_eq!(spawns, 1, "Turns 1..=3 must reuse a single AGY process");

    let logged: Vec<serde_json::Value> = std::fs::read_to_string(&log_path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(logged.len(), 3);
    assert_eq!(logged[1]["prompt"][0]["text"], "follow-up 2");
    assert_eq!(logged[2]["prompt"][0]["text"], "follow-up 3");
    assert_eq!(logged[2]["prompt"][1]["type"], "image");

    // Turn 4: changed system contract cannot reuse or fit 420KB replay -> fresh session rejects >384KB
    req.system = "changed rules".into();
    req.messages.push(msg(
        Role::Assistant,
        vec![Part::Text(TextPart {
            base: base(),
            text: "t3-ok".into(),
            synthetic: false,
        })],
    ));
    req.messages.push(msg(
        Role::User,
        vec![Part::Text(TextPart {
            base: base(),
            text: "follow-up 4".into(),
            synthetic: false,
        })],
    ));
    assert!(provider.replay_budget(&req).await.unwrap().unwrap().0 > limit);
    assert!(provider.stream(req).await.is_err());
}

#[tokio::test]
async fn persistent_session_resumes_after_idle_exit_and_after_abandon_pending() {
    // Part A: clean EOF while idle -> session/resume -> fallback to session/new on error
    let parent = tempfile::tempdir().unwrap();
    let methods_path = parent.path().join("methods.jsonl");
    let script = format!(
        r#"import sys,json
methods_path={:?}
def record(obj):
 with open(methods_path,'a') as f: f.write(json.dumps(obj)+'\n')
def read_msg():
 line=sys.stdin.readline()
 if not line: sys.exit(0)
 m=json.loads(line)
 record({{'id':m.get('id'),'method':m.get('method'),'params':m.get('params')}})
 return m
m1=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m1['id'],'result':{{'protocolVersion':1}}}}),flush=True)
m2=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m2['id'],'result':{{}}}}),flush=True)
m3=read_msg()
if m3['method']=='session/new':
 print(json.dumps({{'jsonrpc':'2.0','id':m3['id'],'result':{{'sessionId':'s-orig'}}}}),flush=True)
 m4=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m4['id'],'result':{{}}}}),flush=True)
 m5=read_msg()
 text='fallback-ok' if 'mew_transcript_v1' in m5['params']['prompt'][0]['text'] else 'turn1-ok'
 print(json.dumps({{'jsonrpc':'2.0','method':'session/update','params':{{'sessionId':'s-orig','update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':text}}}}}}}}),flush=True)
 print(json.dumps({{'jsonrpc':'2.0','id':m5['id'],'result':{{'stopReason':'end_turn'}}}}),flush=True)
 sys.exit(0)
elif m3['method']=='session/resume':
 # Reject resume on turn 3 (`reject-resume` trigger), accept otherwise
 print(json.dumps({{'jsonrpc':'2.0','id':m3['id'],'error':{{'code':-32004,'message':'gone'}}}}),flush=True) if 'reject' in open(methods_path).read() else None
 # Wait: check turn number from recorded methods
"#,
        methods_path
    );
    let _ = script;
    // Simpler deterministic script tracking spawn count via spawns.txt:
    let spawns_path = parent.path().join("spawns.txt");
    let script = format!(
        r#"import sys,json
spawns_path={:?}
methods_path={:?}
with open(spawns_path,'a') as f: f.write('1\n')
spawn=len(open(spawns_path).readlines())
def record(obj):
 with open(methods_path,'a') as f: f.write(json.dumps(obj)+'\n')
def read_msg():
 line=sys.stdin.readline()
 if not line: sys.exit(0)
 m=json.loads(line)
 record({{'spawn':spawn,'id':m.get('id'),'method':m.get('method'),'params':m.get('params')}})
 return m
m1=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m1['id'],'result':{{'protocolVersion':1}}}}),flush=True)
m2=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m2['id'],'result':{{}}}}),flush=True)
while True:
 m=read_msg()
 meth=m['method']
 if meth=='session/new':
  print(json.dumps({{'jsonrpc':'2.0','id':m['id'],'result':{{'sessionId':f's-{{spawn}}'}}}}),flush=True)
 elif meth=='session/resume':
  if spawn==3:
   print(json.dumps({{'jsonrpc':'2.0','id':m['id'],'error':{{'code':-32004,'message':'expired'}}}}),flush=True)
  else:
   print(json.dumps({{'jsonrpc':'2.0','id':m['id'],'result':{{'sessionId':m['params']['sessionId']}}}}),flush=True)
 elif meth=='session/set_model':
  print(json.dumps({{'jsonrpc':'2.0','id':m['id'],'result':{{}}}}),flush=True)
 elif meth=='session/prompt':
  sid=m['params']['sessionId']
  prompt_txt=m['params']['prompt'][0]['text']
  reply=f'spawn={{spawn}};sid={{sid}};txt={{prompt_txt}}'
  print(json.dumps({{'jsonrpc':'2.0','method':'session/update','params':{{'sessionId':sid,'update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':reply}}}}}}}}),flush=True)
  print(json.dumps({{'jsonrpc':'2.0','id':m['id'],'result':{{'stopReason':'end_turn'}}}}),flush=True)
  # Spawn 1 & 2 exit immediately after end_turn to force session/resume on next turn
  if spawn in (1,2): sys.exit(0)
"#,
        spawns_path, methods_path
    );
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), &script));
    let mut req = request();
    let e1: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    assert!(e1.iter().any(
        |e| matches!(e, ProviderEvent::PartDelta { field: "text", delta, .. } if delta.contains("spawn=1;sid=s-1"))
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Turn 2: Spawn 1 exited -> Spawn 2 uses session/resume('s-1') + delta prompt!
    req.messages.push(msg(
        Role::Assistant,
        vec![Part::Text(TextPart {
            base: base(),
            text: e1
                .iter()
                .filter_map(|e| match e {
                    ProviderEvent::PartDelta {
                        field: "text",
                        delta,
                        ..
                    } => Some(delta.as_str()),
                    _ => None,
                })
                .collect(),
            synthetic: false,
        })],
    ));
    req.messages.push(msg(
        Role::User,
        vec![Part::Text(TextPart {
            base: base(),
            text: "turn2-delta".into(),
            synthetic: false,
        })],
    ));
    let e2: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    let t2: String = e2
        .iter()
        .filter_map(|e| match e {
            ProviderEvent::PartDelta {
                field: "text",
                delta,
                ..
            } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(t2, "spawn=2;sid=s-1;txt=turn2-delta");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Turn 3: Spawn 2 exited, Spawn 3 rejects session/resume -> falls back in-process to session/new + full replay!
    req.messages.push(msg(
        Role::Assistant,
        vec![Part::Text(TextPart {
            base: base(),
            text: t2,
            synthetic: false,
        })],
    ));
    req.messages.push(msg(
        Role::User,
        vec![Part::Text(TextPart {
            base: base(),
            text: "turn3-fallback".into(),
            synthetic: false,
        })],
    ));
    let e3: Vec<_> = provider.stream(req).await.unwrap().collect().await;
    let t3: String = e3
        .iter()
        .filter_map(|e| match e {
            ProviderEvent::PartDelta {
                field: "text",
                delta,
                ..
            } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    let recorded: Vec<serde_json::Value> = std::fs::read_to_string(&methods_path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        recorded.iter().any(|m| m["spawn"] == 2
            && m["method"] == "session/resume"
            && m["params"]["sessionId"] == "s-1"),
        "expected Turn 2 to send session/resume for s-1: {recorded:?}"
    );
    assert!(
        recorded
            .iter()
            .any(|m| m["spawn"] == 3 && m["method"] == "session/new"),
        "expected Turn 3 to fall back to session/new after resume error: {recorded:?}"
    );
    assert!(
        t3.starts_with("spawn=3;sid=s-3;txt=") && t3.contains("mew_transcript_v1"),
        "expected fallback full replay on s-3, got: {t3}"
    );

    // Part B: abandon_pending retains continuity and resumes via session/resume + delta transcript
    let methods_b = parent.path().join("methods_b.jsonl");
    let script_b = format!(
        r#"import sys,json,urllib.request
methods_b={:?}
def record(obj):
 with open(methods_b,'a') as f: f.write(json.dumps(obj)+'\n')
def read_msg():
 line=sys.stdin.readline()
 if not line: sys.exit(0)
 m=json.loads(line)
 record({{'id':m.get('id'),'method':m.get('method'),'params':m.get('params')}})
 return m
m1=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m1['id'],'result':{{'protocolVersion':1}}}}),flush=True)
m2=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m2['id'],'result':{{}}}}),flush=True)
m3=read_msg()
if m3['method']=='session/new':
 print(json.dumps({{'jsonrpc':'2.0','id':m3['id'],'result':{{'sessionId':'s-abandon'}}}}),flush=True)
 m4=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m4['id'],'result':{{}}}}),flush=True)
 read_msg()
 with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
 def post(body,sid=None):
  h={{'Content-Type':'application/json'}}
  if sid: h['mcp-session-id']=sid
  with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers, r.read()
 h,_=post({{'jsonrpc':'2.0','id':1,'method':'initialize','params':{{'protocolVersion':'2025-11-25','capabilities':{{}},'clientInfo':{{'name':'fake','version':'1'}}}}}})
 sid=h['mcp-session-id']
 post({{'jsonrpc':'2.0','method':'notifications/initialized'}},sid)
 post({{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{{'name':'echo','arguments':{{'text':'hello'}}}}}},sid)
elif m3['method']=='session/resume':
 print(json.dumps({{'jsonrpc':'2.0','id':m3['id'],'result':{{'sessionId':m3['params']['sessionId']}}}}),flush=True)
 m4=read_msg(); print(json.dumps({{'jsonrpc':'2.0','id':m4['id'],'result':{{}}}}),flush=True)
 m5=read_msg()
 txt=m5['params']['prompt'][0]['text']
 print(json.dumps({{'jsonrpc':'2.0','method':'session/update','params':{{'sessionId':'s-abandon','update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':txt}}}}}}}}),flush=True)
 print(json.dumps({{'jsonrpc':'2.0','id':m5['id'],'result':{{'stopReason':'end_turn'}}}}),flush=True)
"#,
        methods_b
    );
    let provider_b = AgyProvider::new(config(parent.path()), fixture(parent.path(), &script_b));
    let mut req_b = request();
    let e_handoff: Vec<_> = provider_b
        .stream(req_b.clone())
        .await
        .unwrap()
        .collect()
        .await;
    let call = e_handoff
        .iter()
        .find_map(|e| match e {
            ProviderEvent::PartStart {
                part: Part::ToolCall(c),
            } => Some(c.clone()),
            _ => None,
        })
        .expect("tool call");
    provider_b.abandon_pending(req_b.session_id.unwrap()).await;

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
    req_b
        .messages
        .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
    req_b.messages.push(msg(
        Role::User,
        vec![Part::ToolResult(ToolResultPart {
            base: base(),
            call_id: call.call_id,
        })],
    ));
    req_b.messages.push(msg(
        Role::User,
        vec![Part::Text(TextPart {
            base: base(),
            text: "after abandon".into(),
            synthetic: false,
        })],
    ));
    let e_resumed: Vec<_> = provider_b.stream(req_b).await.unwrap().collect().await;
    let t_resumed: String = e_resumed
        .iter()
        .filter_map(|e| match e {
            ProviderEvent::PartDelta {
                field: "text",
                delta,
                ..
            } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        t_resumed.contains("mew_transcript_delta_v1") && t_resumed.contains("after abandon"),
        "expected delta transcript on resumed session after abandon, got: {t_resumed}"
    );
    let recorded_b: Vec<serde_json::Value> = std::fs::read_to_string(&methods_b)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        recorded_b
            .iter()
            .any(|m| m["method"] == "session/resume" && m["params"]["sessionId"] == "s-abandon"),
        "expected session/resume('s-abandon') after abandon_pending: {recorded_b:?}"
    );
}

#[tokio::test]
async fn allows_readonly_spill_path_scopes_strictly_to_active_session_step_outputs() {
    let parent = tempfile::tempdir().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    let active_step_dir = app_data
        .path()
        .join("brain/s-spill/.system_generated/steps/5");
    let root_step_dir = app_data
        .path()
        .join("brain/root-conv-42/.system_generated/steps/9");
    let other_step_dir = app_data
        .path()
        .join("brain/other-conv/.system_generated/steps/5");
    let active_logs_dir = app_data.path().join("brain/s-spill/.system_generated/logs");
    let non_digit_step_dir = app_data
        .path()
        .join("brain/s-spill/.system_generated/steps/five");
    for dir in [
        &active_step_dir,
        &root_step_dir,
        &other_step_dir,
        &active_logs_dir,
        &non_digit_step_dir,
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let active_spill = active_step_dir.join("output.txt");
    let root_spill = root_step_dir.join("output.txt");
    let other_spill = other_step_dir.join("output.txt");
    let transcript_log = active_logs_dir.join("transcript.jsonl");
    let non_output_file = active_step_dir.join("notes.txt");
    let non_digit_spill = non_digit_step_dir.join("output.txt");
    let external_secret = parent.path().join("secret.txt");
    for file in [
        &active_spill,
        &root_spill,
        &other_spill,
        &transcript_log,
        &non_output_file,
        &non_digit_spill,
        &external_secret,
    ] {
        std::fs::write(file, "data\n").unwrap();
    }
    let symlink_spill = app_data
        .path()
        .join("brain/s-spill/.system_generated/steps/6");
    std::fs::create_dir_all(&symlink_spill).unwrap();
    let symlink_target = symlink_spill.join("output.txt");
    std::os::unix::fs::symlink(&external_secret, &symlink_target).unwrap();

    let script = format!(
        r#"import sys,json,urllib.request
for i,r in [(1,{{'protocolVersion':1}}),(2,{{}}),(3,{{'sessionId':'s-spill'}}),(4,{{}})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({{'jsonrpc':'2.0','id':i,'result':r}}),flush=True)
p1=json.loads(sys.stdin.readline())
with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
def post(body,sid=None):
 h={{'Content-Type':'application/json'}}
 if sid: h['mcp-session-id']=sid
 with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers,r.read()
h,_=post({{'jsonrpc':'2.0','id':1,'method':'initialize','params':{{'protocolVersion':'2025-11-25','capabilities':{{}},'clientInfo':{{'name':'fake','version':'1'}}}}}})
sid=h['mcp-session-id']
post({{'jsonrpc':'2.0','method':'notifications/initialized'}},sid)
post({{'jsonrpc':'2.0','id':2,'method':'tools/call','params':{{'name':'read','arguments':{{'path':{:?}}},'_meta':{{'antigravity.google/root_conversation_id':'root-conv-42'}}}}}},sid)
"#,
        active_spill
    );
    let mut cfg = config(parent.path());
    cfg.environment
        .push(("ANTIGRAVITY_APP_DATA_DIR".into(), app_data.path().into()));
    let provider = AgyProvider::new(cfg, fixture(parent.path(), &script));
    let mut req = request();
    req.tools = vec![ToolDef {
        name: "read".into(),
        description: "read".into(),
        schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
    }];
    let mew_sid = req.session_id.unwrap();
    let events: Vec<_> = provider.stream(req.clone()).await.unwrap().collect().await;
    assert!(events.iter().any(|e| matches!(
        e,
        ProviderEvent::MessageEnd {
            finish: Finish::ToolUse,
            ..
        }
    )));

    assert!(
        provider
            .allows_readonly_spill_path(mew_sid, &active_spill)
            .await
    );
    assert!(
        provider
            .allows_readonly_spill_path(mew_sid, &root_spill)
            .await
    );
    assert!(
        !provider
            .allows_readonly_spill_path(mew_sid, &other_spill)
            .await
    );
    assert!(
        !provider
            .allows_readonly_spill_path(mew_sid, &transcript_log)
            .await
    );
    assert!(
        !provider
            .allows_readonly_spill_path(mew_sid, &non_output_file)
            .await
    );
    assert!(
        !provider
            .allows_readonly_spill_path(mew_sid, &non_digit_spill)
            .await
    );
    assert!(
        !provider
            .allows_readonly_spill_path(mew_sid, &symlink_target)
            .await
    );
    let traversal =
        active_step_dir.join("../../../other-conv/.system_generated/steps/5/output.txt");
    assert!(
        !provider
            .allows_readonly_spill_path(mew_sid, &traversal)
            .await
    );
    assert!(
        !provider
            .allows_readonly_spill_path(Ulid::new(), &active_spill)
            .await
    );
}

#[tokio::test]
async fn unpaginated_spill_file_read_is_bounded_with_next_offset_and_workspace_reads_stay_intact() {
    let parent = tempfile::tempdir().unwrap();
    let spill_path = "/tmp/fake-brain/s-cap/.system_generated/steps/5/output.txt";
    let script = format!(
        r#"import sys,json,urllib.request
for i,r in [(1,{{'protocolVersion':1}}),(2,{{}}),(3,{{'sessionId':'s-cap'}}),(4,{{}})]:
 json.loads(sys.stdin.readline())
 print(json.dumps({{'jsonrpc':'2.0','id':i,'result':r}}),flush=True)
sys.stdin.readline()
with open('.agents/mcp_config.json') as f: url=json.load(f)['mcpServers']['mew']['serverUrl']
def post(body,sid=None):
 h={{'Content-Type':'application/json'}}
 if sid: h['mcp-session-id']=sid
 with urllib.request.urlopen(urllib.request.Request(url,data=json.dumps(body).encode(),headers=h)) as r: return r.headers,r.read()
h,_=post({{'jsonrpc':'2.0','id':1,'method':'initialize','params':{{'protocolVersion':'2025-11-25','capabilities':{{}},'clientInfo':{{'name':'fake','version':'1'}}}}}})
sid=h['mcp-session-id']
post({{'jsonrpc':'2.0','method':'notifications/initialized'}},sid)
records=[]
for idx, args in enumerate([
    {{'path':{spill_path:?}}},
    {{'path':{spill_path:?},'offset':120,'limit':20}},
    {{'path':{spill_path:?}}},
    {{'path':{spill_path:?}}},
    {{'path':'src/lib.rs'}},
], start=2):
 _,body=post({{'jsonrpc':'2.0','id':idx,'method':'tools/call','params':{{'name':'read','arguments':args}}}},sid)
 result=json.loads(body)['result']
 records.append({{'id':idx,'len':len(result['content'][0]['text']),'isError':result['isError'],'text':result['content'][0]['text']}})
for rec in records:
 print(json.dumps({{'jsonrpc':'2.0','method':'session/update','params':{{'sessionId':'s-cap','update':{{'sessionUpdate':'agent_message_chunk','content':{{'type':'text','text':json.dumps(rec) + '\n'}}}}}}}}),flush=True)
print(json.dumps({{'jsonrpc':'2.0','id':5,'result':{{'stopReason':'end_turn'}}}}),flush=True)
"#
    );
    let provider = AgyProvider::new(config(parent.path()), fixture(parent.path(), &script));
    let mut req = request();
    req.tools = vec![ToolDef {
        name: "read".into(),
        description: "read".into(),
        schema: json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}},"required":["path"]}),
    }];

    let numbered_20kb = (1..=450)
        .map(|i| format!("{i}:line content padding to exceed eight kilobytes easily {i:04}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(numbered_20kb.len() > 20_000);
    let small_slice = (121..=140)
        .map(|i| format!("{i}:short line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let offloaded = format!(
        "[Output offloaded: 800 lines / 60000 bytes saved to /tmp/fake.txt — use read tool with offset/limit to inspect]\n\n[output.txt#abc]\n{}\n\n[... middle omitted (600 lines / 45000 bytes) ...]\n\n790:tail line",
        (1..=450)
            .map(|i| format!("{i}:head content block padded to exceed eight kilobytes {i:04}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(offloaded.len() > 20_000);
    let huge_single_line = format!("1:{}🦀{}", "a".repeat(12_000), "b".repeat(5_000));
    let workspace_20kb = (1..=450)
        .map(|i| format!("{i}:workspace line {i:04} padded for twenty kilobytes"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(workspace_20kb.len() > 20_000);

    let payloads = [
        (json!({"path": spill_path}), numbered_20kb),
        (
            json!({"path": spill_path, "offset": 120, "limit": 20}),
            small_slice.clone(),
        ),
        (json!({"path": spill_path}), offloaded),
        (json!({"path": spill_path}), huge_single_line),
        (json!({"path": "src/lib.rs"}), workspace_20kb.clone()),
    ];

    let mut cur_req = req;
    let mut final_text = String::new();
    for (input_args, output_text) in payloads {
        let events: Vec<_> = provider
            .stream(cur_req.clone())
            .await
            .unwrap()
            .collect()
            .await;
        for event in &events {
            if let ProviderEvent::PartDelta {
                field: "text",
                delta,
                ..
            } = event
            {
                final_text.push_str(delta);
            }
        }
        let call = events
            .iter()
            .find_map(|event| match event {
                ProviderEvent::PartStart {
                    part: Part::ToolCall(call),
                } => Some(call.clone()),
                _ => None,
            })
            .expect("tool call");
        let mut done = call.clone();
        done.state = ToolState::Completed(ToolStateCompleted {
            input: input_args,
            output: output_text,
            metadata: None,
            diff: None,
            images: vec![],
            time: ToolTime {
                start: 0,
                end: Some(1),
            },
        });
        cur_req
            .messages
            .push(msg(Role::Assistant, vec![Part::ToolCall(done)]));
        cur_req.messages.push(msg(
            Role::User,
            vec![Part::ToolResult(ToolResultPart {
                base: base(),
                call_id: call.call_id,
            })],
        ));
    }
    let last_events: Vec<_> = provider.stream(cur_req).await.unwrap().collect().await;
    for event in &last_events {
        if let ProviderEvent::PartDelta {
            field: "text",
            delta,
            ..
        } = event
        {
            final_text.push_str(delta);
        }
    }

    let records: Vec<serde_json::Value> = final_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 5);

    // 1. Unpaginated 20 KB spill read is capped <= 8_000 bytes and gives exact next offset.
    assert_eq!(records[0]["isError"], false);
    assert!(records[0]["len"].as_u64().unwrap() <= 8_000);
    let r0_text = records[0]["text"].as_str().unwrap();
    let (r0_head, _) = r0_text.split_once("\n\n[mew-agy:").unwrap();
    let last_no: usize = r0_head
        .lines()
        .last()
        .unwrap()
        .split_once(':')
        .unwrap()
        .0
        .parse()
        .unwrap();
    assert!(
        r0_text.contains(&format!("offset={last_no}")),
        "expected offset={last_no} in {r0_text}"
    );

    // 2. Small paginated spill read is returned verbatim.
    assert_eq!(records[1]["text"], small_slice);

    // 3. Offloaded spill read omits tail and caps <= 8_000 bytes.
    assert!(records[2]["len"].as_u64().unwrap() <= 8_000);
    assert!(!records[2]["text"]
        .as_str()
        .unwrap()
        .contains("790:tail line"));
    assert!(records[2]["text"].as_str().unwrap().contains("offset="));

    // 4. Single ultra-long line spill read caps cleanly at <= 8_000 bytes and points to offset=1.
    assert!(records[3]["len"].as_u64().unwrap() <= 8_000);
    assert!(records[3]["text"].as_str().unwrap().contains("offset=1"));

    // 5. Normal workspace file read > 20 KB is NOT truncated by the 8 KB spill cap.
    assert_eq!(records[4]["text"], workspace_20kb);
}
