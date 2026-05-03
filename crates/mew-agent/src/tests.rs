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
