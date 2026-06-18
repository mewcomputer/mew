use async_trait::async_trait;
use std::sync::Mutex as StdMutex;

use mew_hooks::NopDispatcher;
use mew_hooks::ToolOutput;
use mew_message::{
    Finish, Message, Part, PartBase, Role, TextPart, Time, Tokens, ToolCallPart, ToolState,
    ToolStatePending, ToolTime,
};
use mew_provider::{EventStream, Provider, ProviderError, ProviderEvent, Request};
use mew_provider_fake::FakeProvider;
use mew_tools::tools::flag_important::{FlagMode, FlaggedFile};
use mew_tools::{Sensitivity, Tool, ToolCtx};

use crate::Agent;
use crate::AgentEvent;

// ------------------------------------------------------------------
// Fakes
// ------------------------------------------------------------------

/// A fake provider that returns a different script on each call.
struct StatefulFakeProvider {
    scripts: StdMutex<Vec<Vec<mew_provider::ProviderEvent>>>,
}

impl StatefulFakeProvider {
    fn new(scripts: Vec<Vec<mew_provider::ProviderEvent>>) -> Self {
        Self {
            scripts: StdMutex::new(scripts),
        }
    }
}

#[async_trait]
impl Provider for StatefulFakeProvider {
    fn name(&self) -> &str {
        "stateful-fake"
    }

    async fn stream(&self, _req: Request) -> Result<EventStream, ProviderError> {
        let script = self.scripts.lock().unwrap().remove(0);
        let stream = futures::stream::iter(script);
        Ok(Box::pin(stream))
    }
}

/// A provider that captures every request's messages and replays a fixed
/// script. Used to assert what the agent actually sent to the model.
struct CapturingProvider {
    captured: StdMutex<Vec<Vec<Message>>>,
    script: Vec<mew_provider::ProviderEvent>,
}

#[async_trait]
impl Provider for CapturingProvider {
    fn name(&self) -> &str {
        "capturing"
    }

    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError> {
        self.captured.lock().unwrap().push(req.messages.clone());
        let script = self.script.clone();
        Ok(Box::pin(futures::stream::iter(script)))
    }
}

struct EchoTool {
    schema: serde_json::Value,
    sensitivity: Sensitivity,
}

impl EchoTool {
    fn mutating() -> Self {
        Self {
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                }
            }),
            sensitivity: Sensitivity::Mutating,
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echoes input"
    }
    fn schema(&self) -> &serde_json::Value {
        &self.schema
    }
    fn sensitivity(&self) -> Sensitivity {
        self.sensitivity
    }
    async fn execute(
        &self,
        _ctx: ToolCtx,
        input: serde_json::Value,
    ) -> Result<ToolOutput, mew_tools::ToolError> {
        Ok(ToolOutput {
            output: input.to_string(),
            error: String::new(),
            diff: None,
        })
    }
}

// ------------------------------------------------------------------
// Unit tests
// ------------------------------------------------------------------

#[tokio::test]
async fn test_set_system() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    // Agent is not mutable because set_system takes &mut self.
    // We need to create it as mutable.
    let mut agent = agent;
    agent.set_system("you are a cat".into());
    assert_eq!(agent.system, "you are a cat");
}

#[tokio::test]
async fn test_clear_context_empties_messages() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let msg = Message {
        id: ulid::Ulid::new(),
        session_id: agent.session_id,
        role: Role::User,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: agent.session_id,
            },
            text: "hello".into(),
            synthetic: false,
        })],
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    };
    agent.load_messages(vec![msg]).await;
    assert_eq!(agent.messages.lock().await.len(), 1);

    agent.clear_context().await;
    assert_eq!(agent.messages.lock().await.len(), 0);
}

#[tokio::test]
async fn test_clear_context_writes_marker_to_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let session_id = format!("clear-test-{}", ulid::Ulid::new());
    let writer = mew_session::Writer::open_at(tmp.path(), &session_id)
        .await
        .expect("open session");
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        Some(writer),
        vec![],
        None,
    );

    agent.clear_context().await;

    let msgs = mew_session::Reader::load_from(tmp.path(), &session_id)
        .await
        .expect("load session");
    assert_eq!(msgs.len(), 1, "exactly the clear marker should be on disk");
    assert_eq!(msgs[0].role, Role::User);
    let is_synthetic_marker = msgs[0].parts.iter().any(|p| match p {
        Part::Text(tp) => tp.synthetic,
        _ => false,
    });
    assert!(
        is_synthetic_marker,
        "marker should carry a synthetic text part"
    );
}

#[tokio::test]
async fn test_flagged_files_re_injected_after_compaction() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plan_path = tmp.path().join("plan.md");
    std::fs::write(&plan_path, "# THE PLAN\nstep 1: do the thing").unwrap();

    let script = FakeProvider::text_response("ok");
    let provider = std::sync::Arc::new(CapturingProvider {
        captured: StdMutex::new(Vec::new()),
        script,
    });

    let mut agent = Agent::new(
        provider.clone(),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    // Tiny context window so compaction triggers; keep only the most recent
    // turn so older history is dropped and the flagged file is the surviving
    // signal.
    agent.context_window = 10;
    agent.keep_turns = 1;

    // Pre-populate history so compaction has something to compact away.
    let old_msg = Message {
        id: ulid::Ulid::new(),
        session_id: agent.session_id,
        role: Role::User,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: agent.session_id,
            },
            text: "old conversation that should be compacted away".into(),
            synthetic: false,
        })],
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    };
    agent.load_messages(vec![old_msg]).await;

    // Flag the plan file as important in Included mode.
    agent.flagged_files.lock().await.push(FlaggedFile {
        path: plan_path.clone(),
        mode: FlagMode::Included,
    });

    agent.force_compact().await;

    let mut rx = agent.run("next prompt".into());
    while rx.recv().await.is_some() {}

    let captured = provider.captured.lock().unwrap();
    assert!(
        !captured.is_empty(),
        "provider should have received at least one request"
    );
    let request_msgs = &captured[0];
    let has_flagged_content = request_msgs.iter().any(|m| {
        m.parts.iter().any(|p| match p {
            Part::Text(tp) => tp.text.contains("THE PLAN"),
            _ => false,
        })
    });
    assert!(
        has_flagged_content,
        "flagged file content should be re-injected into the request after compaction"
    );
}

#[test]
fn test_apply_delta_text() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut msg = Message {
        id: ulid::Ulid::new(),
        session_id: agent.session_id,
        role: Role::Assistant,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: agent.session_id,
            },
            text: "hello".into(),
            synthetic: false,
        })],
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    };
    let part_id = msg.parts[0].id();
    agent.apply_delta(&mut msg, part_id, "text", " world");
    assert_eq!(
        match &msg.parts[0] {
            Part::Text(p) => &p.text,
            _ => panic!("expected text"),
        },
        "hello world"
    );
}

#[test]
fn test_apply_delta_reasoning() {
    use mew_message::ReasoningPart;

    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut msg = Message {
        id: ulid::Ulid::new(),
        session_id: agent.session_id,
        role: Role::Assistant,
        parts: vec![Part::Reasoning(ReasoningPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: agent.session_id,
            },
            text: "think".into(),
            signature: None,
        })],
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    };
    let part_id = msg.parts[0].id();
    agent.apply_delta(&mut msg, part_id, "text", "ing");
    assert_eq!(
        match &msg.parts[0] {
            Part::Reasoning(p) => &p.text,
            _ => panic!("expected reasoning"),
        },
        "thinking"
    );
}

#[test]
fn test_pending_tool_calls() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let session_id = agent.session_id;
    let msg_id = ulid::Ulid::new();
    let now = chrono::Utc::now().timestamp_millis();

    let pending_part = ToolCallPart {
        base: PartBase {
            id: ulid::Ulid::new(),
            message_id: msg_id,
            session_id,
        },
        tool_name: "echo".into(),
        call_id: "c1".into(),
        state: ToolState::Pending(ToolStatePending {
            input: serde_json::Value::Null,
            time: ToolTime {
                start: now,
                end: None,
            },
        }),
        raw_input: String::new(),
    };
    let completed_part = ToolCallPart {
        base: PartBase {
            id: ulid::Ulid::new(),
            message_id: msg_id,
            session_id,
        },
        tool_name: "echo".into(),
        call_id: "c2".into(),
        state: ToolState::Completed(mew_message::ToolStateCompleted {
            input: serde_json::Value::Null,
            output: "done".into(),
            metadata: None,
            diff: None,
            time: ToolTime {
                start: now,
                end: Some(now),
            },
        }),
        raw_input: String::new(),
    };

    let msg = Message {
        id: msg_id,
        session_id,
        role: Role::Assistant,
        parts: vec![
            Part::ToolCall(pending_part.clone()),
            Part::ToolCall(completed_part),
        ],
        time: Time {
            created: now,
            completed: None,
        },
        assistant: None,
    };

    let pending = agent.pending_tool_calls(&msg);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].call_id, "c1");
}

#[test]
fn test_update_tool_call() {
    use mew_message::ToolStateRunning;

    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let session_id = agent.session_id;
    let msg_id = ulid::Ulid::new();
    let part_id = ulid::Ulid::new();
    let now = chrono::Utc::now().timestamp_millis();

    let mut msg = Message {
        id: msg_id,
        session_id,
        role: Role::Assistant,
        parts: vec![Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: part_id,
                message_id: msg_id,
                session_id,
            },
            tool_name: "echo".into(),
            call_id: "c1".into(),
            state: ToolState::Pending(ToolStatePending {
                input: serde_json::Value::Null,
                time: ToolTime {
                    start: now,
                    end: None,
                },
            }),
            raw_input: String::new(),
        })],
        time: Time {
            created: now,
            completed: None,
        },
        assistant: None,
    };

    let new_state = ToolState::Running(ToolStateRunning {
        input: serde_json::Value::Null,
        output: String::new(),
        time: ToolTime {
            start: now,
            end: None,
        },
    });
    agent.update_tool_call(&mut msg, part_id, new_state.clone());

    assert!(
        matches!(&msg.parts[0], Part::ToolCall(tc) if tc.base.id == part_id && matches!(tc.state, ToolState::Running(_)))
    );
}

// ------------------------------------------------------------------
// Integration tests
// ------------------------------------------------------------------

#[tokio::test]
async fn test_text_turn() {
    let script = FakeProvider::text_response("hello world");
    let provider = std::sync::Arc::new(FakeProvider::new(script));
    let agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );

    let mut rx = agent.run("hi".into());
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }

    // Should see PartStart, some PartDeltas, PartEnd, MessageEnd
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::PartStart { .. }))));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::PartDelta { .. }))));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::PartEnd { .. }))));
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::MessageEnd { .. }))));

    // Messages should contain user + assistant
    let msgs = agent.messages.lock().await;
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, Role::User);
    assert_eq!(msgs[1].role, Role::Assistant);
}

#[tokio::test]
async fn test_tool_turn_allowed() {
    let script1 = FakeProvider::tool_call("echo", "c1", serde_json::json!({"input": "hi"}));
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
    let agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );

    let mut rx = agent.run("call echo".into());
    let mut got_permission = false;
    let mut got_tool_start = false;
    let mut got_tool_end = false;

    while let Some(ev) = rx.recv().await {
        let mut should_break = false;
        match ev {
            AgentEvent::PermissionRequest { call, tx } => {
                got_permission = true;
                assert_eq!(call.tool_name, "echo");
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            AgentEvent::ToolStart { call_id } => {
                got_tool_start = true;
                assert_eq!(call_id, "c1");
            }
            AgentEvent::ToolEnd { call_id, success } => {
                got_tool_end = true;
                assert_eq!(call_id, "c1");
                assert!(success);
            }
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }) if got_tool_end => should_break = true,
            _ => {}
        }
        if should_break {
            break;
        }
    }

    assert!(got_permission);
    assert!(got_tool_start);
    assert!(got_tool_end);

    // There should be 4 messages: user, assistant (tool call), user (tool result), assistant (text)
    let msgs = agent.messages.lock().await;
    assert_eq!(msgs.len(), 4);
    assert!(matches!(&msgs[1].parts[0], Part::ToolCall(_)));
    assert!(matches!(&msgs[2].parts[0], Part::ToolResult(_)));
}

#[tokio::test]
async fn test_tool_turn_denied() {
    let script1 = FakeProvider::tool_call("echo", "c1", serde_json::json!({"input": "hi"}));
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
    let agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );

    let mut rx = agent.run("call echo".into());
    let mut got_permission = false;
    let mut got_tool_start = false;
    let mut got_tool_end = false;

    while let Some(ev) = rx.recv().await {
        let mut should_break = false;
        match ev {
            AgentEvent::PermissionRequest { call, tx } => {
                got_permission = true;
                assert_eq!(call.tool_name, "echo");
                let _ = tx.send(mew_hooks::PermissionDecision::Deny);
            }
            AgentEvent::ToolStart { call_id } => {
                got_tool_start = true;
                assert_eq!(call_id, "c1");
            }
            AgentEvent::ToolEnd { call_id, success } => {
                got_tool_end = true;
                assert_eq!(call_id, "c1");
                assert!(!success);
            }
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }) if got_tool_end => should_break = true,
            _ => {}
        }
        if should_break {
            break;
        }
    }

    assert!(got_permission);
    assert!(got_tool_start);
    assert!(got_tool_end);
}

#[tokio::test]
async fn test_cancellation_during_stream() {
    let script = FakeProvider::text_response("a very long response that takes time");
    let provider = std::sync::Arc::new(FakeProvider::new(script));
    let agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );

    let mut rx = agent.run("hi".into());
    // Cancel immediately.
    agent.cancel_token.cancel();

    let mut got_error = false;
    while let Some(ev) = rx.recv().await {
        if matches!(ev, AgentEvent::Error(ref msg) if msg == "aborted") {
            got_error = true;
            break;
        }
    }

    assert!(got_error);

    // At minimum the user message should be persisted.
    let msgs = agent.messages.lock().await;
    assert!(!msgs.is_empty());
    assert_eq!(msgs[0].role, Role::User);
}

#[tokio::test]
async fn test_permission_engine_allow_rule() {
    let script1 = FakeProvider::tool_call("echo", "c1", serde_json::json!({"input": "hi"}));
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script1, script2]));

    let mut agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );
    agent.set_permission_engine(std::sync::Arc::new(
        mew_config::permissions::PermissionEngine::new(vec![]),
    ));

    let mut rx = agent.run("call two echos".into());
    let mut permissions_prompted = 0;

    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::PermissionRequest { tx, .. } => {
                permissions_prompted += 1;
                let decision = if permissions_prompted == 1 {
                    mew_hooks::PermissionDecision::AllowSession
                } else {
                    mew_hooks::PermissionDecision::AllowOnce
                };
                let _ = tx.send(decision);
            }
            AgentEvent::ToolEnd { .. } => {}
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }) => break,
            _ => {}
        }
    }

    assert_eq!(
        permissions_prompted, 1,
        "AllowSession should skip second prompt within same turn"
    );
}

#[tokio::test]
async fn test_multi_tool_call_turn() {
    // Provider returns two tool calls in one turn
    let script1 = vec![
        mew_provider::ProviderEvent::PartStart {
            part: Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: "echo".into(),
                call_id: "c1".into(),
                state: ToolState::Pending(ToolStatePending {
                    input: serde_json::json!({"input": "first"}),
                    time: ToolTime {
                        start: 0,
                        end: None,
                    },
                }),
                raw_input: String::new(),
            }),
        },
        mew_provider::ProviderEvent::PartEnd {
            part_id: ulid::Ulid::new(),
        },
        mew_provider::ProviderEvent::PartStart {
            part: Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: "echo".into(),
                call_id: "c2".into(),
                state: ToolState::Pending(ToolStatePending {
                    input: serde_json::json!({"input": "second"}),
                    time: ToolTime {
                        start: 0,
                        end: None,
                    },
                }),
                raw_input: String::new(),
            }),
        },
        mew_provider::ProviderEvent::PartEnd {
            part_id: ulid::Ulid::new(),
        },
        mew_provider::ProviderEvent::MessageEnd {
            finish: Finish::ToolUse,
            usage: Tokens::default(),
            cost: 0.0,
        },
    ];
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script1, script2]));

    let agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );

    let mut rx = agent.run("call two echos".into());
    let mut tool_ends = 0;

    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::PermissionRequest { tx, .. } => {
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            AgentEvent::ToolEnd { .. } => {
                tool_ends += 1;
            }
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }) => break,
            _ => {}
        }
    }

    assert_eq!(tool_ends, 2, "should execute both tools");

    let msgs = agent.messages.lock().await;
    assert_eq!(msgs.len(), 4); // user, assistant (2 tools), user (2 results), assistant
}

#[tokio::test]
async fn test_permission_engine_session_allow() {
    let script = vec![
        mew_provider::ProviderEvent::PartStart {
            part: Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: "echo".into(),
                call_id: "c1".into(),
                state: ToolState::Pending(ToolStatePending {
                    input: serde_json::json!({"input": "first"}),
                    time: ToolTime {
                        start: 0,
                        end: None,
                    },
                }),
                raw_input: String::new(),
            }),
        },
        mew_provider::ProviderEvent::PartEnd {
            part_id: ulid::Ulid::new(),
        },
        mew_provider::ProviderEvent::PartStart {
            part: Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: "echo".into(),
                call_id: "c2".into(),
                state: ToolState::Pending(ToolStatePending {
                    input: serde_json::json!({"input": "second"}),
                    time: ToolTime {
                        start: 0,
                        end: None,
                    },
                }),
                raw_input: String::new(),
            }),
        },
        mew_provider::ProviderEvent::PartEnd {
            part_id: ulid::Ulid::new(),
        },
        mew_provider::ProviderEvent::MessageEnd {
            finish: Finish::ToolUse,
            usage: Tokens::default(),
            cost: 0.0,
        },
    ];
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script, script2]));

    let mut agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );
    agent.set_permission_engine(std::sync::Arc::new(
        mew_config::permissions::PermissionEngine::new(vec![]),
    ));

    let mut rx = agent.run("call two echos".into());
    let mut permissions_prompted = 0;

    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::PermissionRequest { tx, .. } => {
                permissions_prompted += 1;
                let decision = if permissions_prompted == 1 {
                    mew_hooks::PermissionDecision::AllowSession
                } else {
                    mew_hooks::PermissionDecision::AllowOnce
                };
                let _ = tx.send(decision);
            }
            AgentEvent::ToolEnd { .. } => {}
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }) => break,
            _ => {}
        }
    }

    assert_eq!(
        permissions_prompted, 1,
        "AllowSession should skip second prompt within same turn"
    );
}

// ------------------------------------------------------------------
// Streaming tool-call argument reconciliation
//
// Regression: streamed deltas accumulated into `raw_input`, but
// `state.input` stayed at `Value::Null` for the entire streaming window,
// so the session JSONL, tool execution, and subagent dispatcher all saw a
// null/empty input.
// ------------------------------------------------------------------

#[test]
fn test_reconcile_tool_call_input_parses_streamed_arguments() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let session_id = agent.session_id;
    let msg_id = ulid::Ulid::new();
    let part_id = ulid::Ulid::new();
    let now = chrono::Utc::now().timestamp_millis();

    let mut msg = Message {
        id: msg_id,
        session_id,
        role: Role::Assistant,
        parts: vec![Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: part_id,
                message_id: msg_id,
                session_id,
            },
            tool_name: "subagent_start".into(),
            call_id: "c1".into(),
            state: ToolState::Pending(ToolStatePending {
                input: serde_json::Value::Null,
                time: ToolTime {
                    start: now,
                    end: None,
                },
            }),
            raw_input: r#"{"name":"explore","prompt":"what is the repo vibe?"}"#.into(),
        })],
        time: Time {
            created: now,
            completed: None,
        },
        assistant: None,
    };

    agent.reconcile_tool_call_input(&mut msg, part_id);

    let Part::ToolCall(tc) = &msg.parts[0] else {
        panic!("expected tool call part");
    };
    assert_eq!(
        tc.state.input(),
        &serde_json::json!({"name": "explore", "prompt": "what is the repo vibe?"}),
        "state.input should be parsed from raw_input at PartEnd"
    );
}

#[test]
fn test_reconcile_tool_call_input_leaves_other_parts_alone() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let session_id = agent.session_id;
    let msg_id = ulid::Ulid::new();
    let target_id = ulid::Ulid::new();
    let other_id = ulid::Ulid::new();
    let now = chrono::Utc::now().timestamp_millis();

    let mut msg = Message {
        id: msg_id,
        session_id,
        role: Role::Assistant,
        parts: vec![
            Part::Text(mew_message::TextPart {
                base: PartBase {
                    id: other_id,
                    message_id: msg_id,
                    session_id,
                },
                text: "hello".into(),
                synthetic: false,
            }),
            Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: target_id,
                    message_id: msg_id,
                    session_id,
                },
                tool_name: "echo".into(),
                call_id: "c1".into(),
                state: ToolState::Pending(ToolStatePending {
                    input: serde_json::Value::Null,
                    time: ToolTime {
                        start: now,
                        end: None,
                    },
                }),
                raw_input: r#"{"input":"hi"}"#.into(),
            }),
        ],
        time: Time {
            created: now,
            completed: None,
        },
        assistant: None,
    };

    let phantom = ulid::Ulid::new();
    agent.reconcile_tool_call_input(&mut msg, phantom);

    let Part::ToolCall(tc) = &msg.parts[1] else {
        panic!("expected tool call part");
    };
    assert_eq!(
        tc.state.input(),
        &serde_json::Value::Null,
        "unrelated reconcile call should not touch other parts"
    );

    agent.reconcile_tool_call_input(&mut msg, target_id);
    let Part::ToolCall(tc) = &msg.parts[1] else {
        panic!("expected tool call part");
    };
    assert_eq!(tc.state.input(), &serde_json::json!({"input": "hi"}));
}

#[test]
fn test_reconcile_tool_call_input_no_op_on_empty_raw_input() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let session_id = agent.session_id;
    let msg_id = ulid::Ulid::new();
    let part_id = ulid::Ulid::new();
    let now = chrono::Utc::now().timestamp_millis();

    let mut msg = Message {
        id: msg_id,
        session_id,
        role: Role::Assistant,
        parts: vec![Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: part_id,
                message_id: msg_id,
                session_id,
            },
            tool_name: "echo".into(),
            call_id: "c1".into(),
            state: ToolState::Pending(ToolStatePending {
                input: serde_json::Value::Null,
                time: ToolTime {
                    start: now,
                    end: None,
                },
            }),
            raw_input: String::new(),
        })],
        time: Time {
            created: now,
            completed: None,
        },
        assistant: None,
    };

    agent.reconcile_tool_call_input(&mut msg, part_id);

    let Part::ToolCall(tc) = &msg.parts[0] else {
        panic!("expected tool call part");
    };
    assert_eq!(
        tc.state.input(),
        &serde_json::Value::Null,
        "empty raw_input should not overwrite a Null state with anything else"
    );
}

#[tokio::test]
async fn test_streaming_tool_call_appends_with_parsed_input() {
    let part_id = ulid::Ulid::new();
    let script = vec![
        mew_provider::ProviderEvent::PartStart {
            part: Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: part_id,
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                tool_name: "echo".into(),
                call_id: "c1".into(),
                state: ToolState::Pending(ToolStatePending {
                    input: serde_json::Value::Null,
                    time: ToolTime {
                        start: 0,
                        end: None,
                    },
                }),
                raw_input: String::new(),
            }),
        },
        mew_provider::ProviderEvent::PartDelta {
            part_id,
            field: "arguments",
            delta: "{\"input\":\"".into(),
        },
        mew_provider::ProviderEvent::PartDelta {
            part_id,
            field: "arguments",
            delta: "hel".into(),
        },
        mew_provider::ProviderEvent::PartDelta {
            part_id,
            field: "arguments",
            delta: "lo\"}".into(),
        },
        mew_provider::ProviderEvent::PartEnd { part_id },
        mew_provider::ProviderEvent::MessageEnd {
            finish: Finish::ToolUse,
            usage: Tokens::default(),
            cost: 0.0,
        },
    ];
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script, script2]));

    let agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );

    let mut rx = agent.run("call echo".into());
    while let Some(ev) = rx.recv().await {
        if let AgentEvent::Provider(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            ..
        }) = ev
        {
            break;
        }
    }

    let msgs = agent.messages.lock().await;
    let assistant = msgs
        .iter()
        .find(|m| m.role == Role::Assistant)
        .expect("expected an assistant message");
    let tool_call = assistant
        .parts
        .iter()
        .find_map(|p| match p {
            Part::ToolCall(tc) => Some(tc),
            _ => None,
        })
        .expect("expected a tool call part");
    assert_eq!(
        tool_call.state.input(),
        &serde_json::json!({"input": "hello"}),
        "after stream ends, state.input must be the parsed arguments, not Null"
    );
    assert_eq!(
        tool_call.raw_input, r#"{"input":"hello"}"#,
        "raw_input should retain the full streamed payload for debugging"
    );
}
