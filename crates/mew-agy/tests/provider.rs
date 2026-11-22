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
