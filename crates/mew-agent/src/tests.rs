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

/// A provider that captures every request's full state (messages + params)
/// and replays a list of scripts in order. Each `stream()` call pops the
/// next script; once the queue is empty, the provider replays the last
/// one forever (useful when the agent loops more turns than scripts
/// you provided). Captures `Request` for inspection.
struct CapturingProvider {
    captured: StdMutex<Vec<Request>>,
    scripts: StdMutex<Vec<Vec<mew_provider::ProviderEvent>>>,
}

impl CapturingProvider {
    fn new(scripts: Vec<Vec<mew_provider::ProviderEvent>>) -> Self {
        Self {
            captured: StdMutex::new(Vec::new()),
            scripts: StdMutex::new(scripts),
        }
    }
}

#[async_trait]
impl Provider for CapturingProvider {
    fn name(&self) -> &str {
        "capturing"
    }

    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError> {
        self.captured.lock().unwrap().push(req.clone());
        let script = {
            let mut scripts = self.scripts.lock().unwrap();
            if scripts.is_empty() {
                Vec::new()
            } else {
                scripts.remove(0)
            }
        };
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
            metadata: None,
            file_delta: None,
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
async fn test_clear_context_preserves_permission_caches() {
    // /clear resets the visible context (messages) and writes a synthetic
    // marker to the session log. It does NOT wipe the in-memory permission
    // caches — `session_allows` and `workspace_allowances` are tied to the
    // session lifetime (the JSONL log), not the visible context. This test
    // pins that decision so future refactors don't quietly change it.
    let mut agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );

    let engine = std::sync::Arc::new(mew_config::permissions::PermissionEngine::new(vec![]));
    agent.set_permission_engine(engine.clone());

    // 1. `session_allows` survives `/clear`.
    engine.add_session_allow("write").await;
    let write_input = serde_json::json!({"path": "foo.rs"});
    let before = engine
        .check(
            "write",
            &write_input,
            Sensitivity::Mutating,
            std::path::Path::new("."),
        )
        .await;
    assert_eq!(
        before,
        mew_hooks::PermissionDecision::AllowOnce,
        "session-allow should hit before /clear"
    );

    agent.clear_context().await;
    assert_eq!(
        agent.messages.lock().await.len(),
        0,
        "messages should be empty after /clear"
    );

    let after = engine
        .check(
            "write",
            &write_input,
            Sensitivity::Mutating,
            std::path::Path::new("."),
        )
        .await;
    assert_eq!(
        after,
        mew_hooks::PermissionDecision::AllowOnce,
        "session-allow MUST survive /clear — tied to session lifetime, not context"
    );

    // 2. `workspace_allowances` survives `/clear` (same lifetime argument).
    let dir = std::path::PathBuf::from("/tmp/outside-workspace");
    agent.workspace_allowances.lock().await.insert(dir.clone());
    assert!(agent.workspace_allowances.lock().await.contains(&dir));

    agent.clear_context().await;

    assert!(
        agent.workspace_allowances.lock().await.contains(&dir),
        "workspace_allowances MUST survive /clear — tied to session lifetime, not context"
    );
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
    let provider = std::sync::Arc::new(CapturingProvider::new(vec![script]));

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
    let request_msgs = &captured[0].messages;
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

    // Pass the agent's own cancel token so cancelling it propagates to the
    // turn. (run_with_parts creates a fresh token when passed None, which
    // would not be reachable from the agent's permanent cancel_token.)
    let mut rx = agent.run_with_parts("hi".into(), vec![], Some(agent.cancel_token.clone()));
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
// Workspace-escape tier integration
//
// The escape tier sits between deny rules and the Permissive short-circuit.
// A bash command that reads a path outside `workspace_roots` must escalate
// to `Prompt` even in Permissive mode (which would otherwise auto-allow
// Mutating tools). This regression covers that path end-to-end through
// `agent.run()` rather than only via the engine's unit tests.
// ------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_escape_tier_prompts_in_permissive_mode() {
    use mew_config::permissions::PermissionEngine;
    use mew_hooks::PermissionMode;
    use mew_tools::tools::bash::Bash;

    let dir = tempfile::tempdir().unwrap();
    let workspace_root = dir.path().to_path_buf();

    // Provider emits a bash tool call whose command reads a path outside
    // the workspace, then a final text response.
    let bash_call = FakeProvider::tool_call(
        "bash",
        "c1",
        serde_json::json!({"command": "cat /etc/passwd"}),
    );
    let done = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![bash_call, done]));

    // Build the engine with the workspace root + Permissive mode. Permissive
    // would normally auto-allow Mutating tools, but the escape tier must
    // short-circuit to Prompt *before* the Permissive branch fires.
    let engine = PermissionEngine::new(vec![])
        .with_workspace_roots(vec![workspace_root.clone()], workspace_root.clone());
    let engine = engine.with_mode(PermissionMode::Permissive);

    let mut agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(Bash)],
        None,
    );
    agent.set_permission_engine(std::sync::Arc::new(engine));

    let mut rx = agent.run("read /etc/passwd".into());
    let mut got_prompt = false;
    let mut tool_started = false;

    while let Some(ev) = rx.recv().await {
        let mut should_break = false;
        match ev {
            AgentEvent::PermissionRequest { call, tx } => {
                got_prompt = true;
                assert_eq!(call.tool_name, "bash");
                // Approve so the tool runs and the turn can complete.
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            AgentEvent::ToolStart { .. } => tool_started = true,
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }) if got_prompt && tool_started => should_break = true,
            _ => {}
        }
        if should_break {
            break;
        }
    }

    assert!(
        got_prompt,
        "escape tier must escalate to PermissionRequest even in Permissive mode"
    );
    assert!(
        tool_started,
        "after approval the bash tool must still execute"
    );
}

#[tokio::test]
async fn test_workspace_escape_tier_disabled_when_roots_empty() {
    // The escape tier is opt-in: empty `workspace_roots` disables it, so
    // `cat /etc/passwd` falls through to the normal permission flow.
    use mew_config::permissions::PermissionEngine;
    use mew_hooks::PermissionMode;
    use mew_tools::tools::bash::Bash;

    let bash_call = FakeProvider::tool_call(
        "bash",
        "c1",
        serde_json::json!({"command": "cat /etc/passwd"}),
    );
    let done = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![bash_call, done]));

    // Permissive mode + no workspace_roots: bash is Dangerous, but Permissive
    // for Mutating tools... actually bash is Dangerous. Permissive still
    // prompts on Dangerous. Use Standard mode + an allow rule to confirm
    // the escape tier is the only thing that escalates.
    let engine = PermissionEngine::new(vec![mew_config::permissions::PermissionRule {
        tool: "bash".into(),
        decision: mew_config::permissions::RuleDecision::Allow,
        r#match: mew_config::permissions::MatchConditions::default(),
    }])
    .with_mode(PermissionMode::Standard);

    let mut agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(Bash)],
        None,
    );
    agent.set_permission_engine(std::sync::Arc::new(engine));

    let mut rx = agent.run("read /etc/passwd".into());
    let mut got_prompt = false;
    let mut tool_started = false;

    while let Some(ev) = rx.recv().await {
        let mut should_break = false;
        match ev {
            AgentEvent::PermissionRequest { call: _, tx } => {
                got_prompt = true;
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            AgentEvent::ToolStart { .. } => tool_started = true,
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::Stop,
                ..
            }) if got_prompt && tool_started => should_break = true,
            _ => {}
        }
        if should_break {
            break;
        }
    }

    assert!(
        !got_prompt,
        "with empty workspace_roots and an Allow rule, no PermissionRequest should fire"
    );
    assert!(tool_started, "tool must still execute after the allow rule");
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

// ------------------------------------------------------------------
// apply_todo_op: the tool-handler dispatch layer
// ------------------------------------------------------------------

#[tokio::test]
async fn test_todo_create_through_handler() {
    let mut list = crate::TodoList::new();
    let note = crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({
            "todos": [
                { "content": "write tests" },
                { "content": "ship it", "depends_on": [1] }
            ]
        }),
        &mut list,
    )
    .expect("create should succeed");
    assert!(note.contains("created #1, #2"));
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.get(2).unwrap().depends_on, vec![1]);
}

#[tokio::test]
async fn test_todo_create_drops_unknown_deps_with_note() {
    let mut list = crate::TodoList::new();
    let note = crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({ "todos": [{ "content": "x", "depends_on": [99] }] }),
        &mut list,
    )
    .unwrap();
    assert!(note.contains("dropped"), "{}", note);
    assert!(list.get(1).unwrap().depends_on.is_empty());
}

#[tokio::test]
async fn test_todo_complete_enforces_deps_through_handler() {
    let mut list = crate::TodoList::new();
    crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({ "todos": [{ "content": "base" }, { "content": "child", "depends_on": [1] }] }),
        &mut list,
    )
    .unwrap();
    let err =
        crate::tools::apply_todo_op("todo_complete", &serde_json::json!({ "id": 2 }), &mut list)
            .unwrap_err();
    assert!(err.contains("depends on #1"));
    assert_eq!(list.get(2).unwrap().status, crate::TodoStatus::Pending);
}

#[tokio::test]
async fn test_todo_delete_enforces_dependents_through_handler() {
    let mut list = crate::TodoList::new();
    crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({ "todos": [{ "content": "base" }, { "content": "child", "depends_on": [1] }] }),
        &mut list,
    )
    .unwrap();
    let err =
        crate::tools::apply_todo_op("todo_delete", &serde_json::json!({ "id": 1 }), &mut list)
            .unwrap_err();
    assert!(err.contains("depend on it"));
    assert_eq!(list.items.len(), 2);
}

#[tokio::test]
async fn test_todo_update_status_change() {
    let mut list = crate::TodoList::new();
    crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({ "todos": [{ "content": "task" }] }),
        &mut list,
    )
    .unwrap();
    crate::tools::apply_todo_op(
        "todo_update",
        &serde_json::json!({ "id": 1, "status": "in_progress" }),
        &mut list,
    )
    .unwrap();
    assert_eq!(list.get(1).unwrap().status, crate::TodoStatus::InProgress);
}

#[tokio::test]
async fn test_todo_update_to_done_enforces_deps() {
    let mut list = crate::TodoList::new();
    crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({ "todos": [{ "content": "base" }, { "content": "child", "depends_on": [1] }] }),
        &mut list,
    )
    .unwrap();
    let err = crate::tools::apply_todo_op(
        "todo_update",
        &serde_json::json!({ "id": 2, "status": "done" }),
        &mut list,
    )
    .unwrap_err();
    assert!(err.contains("depends on #1"));
}

#[tokio::test]
async fn test_todo_update_rejects_empty_change() {
    let mut list = crate::TodoList::new();
    crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({ "todos": [{ "content": "task" }] }),
        &mut list,
    )
    .unwrap();
    let err =
        crate::tools::apply_todo_op("todo_update", &serde_json::json!({ "id": 1 }), &mut list)
            .unwrap_err();
    assert!(err.contains("nothing to update"));
}

#[tokio::test]
async fn test_todo_list_returns_render() {
    let mut list = crate::TodoList::new();
    crate::tools::apply_todo_op(
        "todo_create",
        &serde_json::json!({ "todos": [{ "content": "only item" }] }),
        &mut list,
    )
    .unwrap();
    // apply_todo_op returns an empty note for list; the handler appends render.
    let note = crate::tools::apply_todo_op("todo_list", &serde_json::json!({}), &mut list).unwrap();
    assert!(
        note.is_empty(),
        "list op returns empty note; handler adds the render"
    );
}

#[tokio::test]
async fn test_todo_create_missing_todos_errors() {
    let mut list = crate::TodoList::new();
    let err =
        crate::tools::apply_todo_op("todo_create", &serde_json::json!({}), &mut list).unwrap_err();
    assert!(err.contains("todos"));
}

#[tokio::test]
async fn test_todo_complete_missing_id_errors() {
    let mut list = crate::TodoList::new();
    let err = crate::tools::apply_todo_op("todo_complete", &serde_json::json!({}), &mut list)
        .unwrap_err();
    assert!(err.contains("id"));
}

// ------------------------------------------------------------------
// Persona tests
// ------------------------------------------------------------------

#[test]
fn test_apply_persona_sets_prompt_and_tool_filter() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    let persona = mew_personas::Persona {
        name: "researcher".into(),
        description: "read-only".into(),
        body: "You are a researcher.".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: None,
            tools: Some(vec!["read".into(), "grep".into(), "glob".into()]),
            ..Default::default()
        },
    };
    agent.apply_persona(&persona);
    assert_eq!(agent.persona_name.as_deref(), Some("researcher"));
    assert_eq!(
        agent.persona_prompt.as_deref(),
        Some("You are a researcher.")
    );
    let active = agent.active_tool_names.as_ref().unwrap();
    assert!(active.contains("read"));
    assert!(active.contains("grep"));
    assert!(!active.contains("bash"));
}

#[test]
fn test_apply_persona_with_model_pin_returns_model() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    let persona = mew_personas::Persona {
        name: "executor".into(),
        description: "writes code".into(),
        body: "You execute.".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: Some("z-ai/glm-4.5-air".into()),
            tools: None,
            ..Default::default()
        },
    };
    let pinned = agent.apply_persona(&persona);
    assert_eq!(pinned.as_deref(), Some("z-ai/glm-4.5-air"));
    assert!(agent.active_tool_names.is_none());
}

#[test]
fn test_clear_persona_resets_state() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    let persona = mew_personas::Persona {
        name: "researcher".into(),
        description: "".into(),
        body: "body".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: None,
            tools: Some(vec!["read".into()]),
            ..Default::default()
        },
    };
    agent.apply_persona(&persona);
    assert!(agent.persona_name.is_some());
    assert!(agent.active_tool_names.is_some());

    agent.clear_persona();
    assert!(agent.persona_name.is_none());
    assert!(agent.persona_prompt.is_none());
    assert!(agent.active_tool_names.is_none());
}

#[test]
fn test_apply_persona_with_tools_deny() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    let persona = mew_personas::Persona {
        name: "researcher".into(),
        description: "read-only".into(),
        body: "body".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: None,
            tools: None, // allowlist is None = all tools
            tools_deny: Some(vec!["bash".into(), "write".into()]),
            skills: None,
            ..Default::default()
        },
    };
    agent.apply_persona(&persona);
    // Denylist is populated even though the allowlist is None.
    assert!(agent.active_tool_names.is_none());
    assert!(agent.denied_tool_names.contains("bash"));
    assert!(agent.denied_tool_names.contains("write"));
    assert!(!agent.denied_tool_names.contains("read"));
}

#[test]
fn test_clear_persona_resets_denylist() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    let persona = mew_personas::Persona {
        name: "p".into(),
        description: "".into(),
        body: "".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: None,
            tools: None,
            tools_deny: Some(vec!["bash".into()]),
            skills: None,
            ..Default::default()
        },
    };
    agent.apply_persona(&persona);
    assert!(!agent.denied_tool_names.is_empty());
    agent.clear_persona();
    assert!(agent.denied_tool_names.is_empty());
}

#[tokio::test]
async fn test_apply_persona_with_skills_filter_updates_shared_arc() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    let persona = mew_personas::Persona {
        name: "reviewer".into(),
        description: "code review".into(),
        body: "".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: None,
            tools: None,
            tools_deny: None,
            skills: Some(vec!["git-release".into(), "code-review".into()]),
            ..Default::default()
        },
    };
    agent.apply_persona(&persona);
    // The shared filter Arc reflects the persona's allowlist.
    let guard = agent.skill_filter.read().await;
    let set = guard.as_ref().expect("filter should be set");
    assert!(set.contains("git-release"));
    assert!(set.contains("code-review"));
    assert!(!set.contains("unrelated-skill"));
}

#[test]
fn test_set_skills_rebuilds_system_with_filter() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    agent.set_system("base prompt.".into());
    let skills = vec![
        mew_skills::Skill {
            name: "git-release".into(),
            description: "Create a release".into(),
            body: "...".into(),
            path: std::path::PathBuf::new(),
            template: false,
        },
        mew_skills::Skill {
            name: "code-review".into(),
            description: "Review code".into(),
            path: std::path::PathBuf::new(),
            body: "...".into(),
            template: false,
        },
    ];
    agent.set_skills(skills);
    // No filter yet — both skills appear in the system prompt.
    assert!(agent.system.contains("git-release"));
    assert!(agent.system.contains("code-review"));
}

#[test]
fn test_apply_persona_template_renders() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    agent.supports_vision = true;

    let persona = mew_personas::Persona {
        name: "templated".into(),
        description: "uses template vars".into(),
        body: "You are {{ persona_name }}. {% if supports_vision %}You can see images.{% else %}No vision.{% endif %}".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: None,
            tools: None,
            tools_deny: None,
            skills: None,
            template: Some(true),
            ..Default::default()
        },
    };
    agent.apply_persona(&persona);
    let prompt = agent.persona_prompt.expect("prompt should be set");
    assert!(prompt.contains("You are templated."));
    assert!(prompt.contains("You can see images."));
    assert!(!prompt.contains("supports_vision"));
    assert!(!prompt.contains("{%"));
}

#[test]
fn test_apply_persona_without_template_is_verbatim() {
    let agent = Agent::new(
        std::sync::Arc::new(FakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![],
        None,
    );
    let mut agent = agent;
    let persona = mew_personas::Persona {
        name: "plain".into(),
        description: "".into(),
        body: "Hello {{ name }}".into(),
        path: std::path::PathBuf::new(),
        config: mew_personas::PersonaConfig {
            model: None,
            tools: None,
            tools_deny: None,
            skills: None,
            template: None, // verbatim — no rendering
            ..Default::default()
        },
    };
    agent.apply_persona(&persona);
    let prompt = agent.persona_prompt.expect("prompt should be set");
    // Template syntax is preserved literally when template is not enabled.
    assert_eq!(prompt, "Hello {{ name }}");
}

// --------------------------------------------------------------------------
// Reasoning truncation integration
// --------------------------------------------------------------------------
//
// These exercise the full agent.run() pipeline with a fake provider that
// emits a long ReasoningPart. They assert:
//   - the assistant message's reasoning text is truncated in self.messages
//   - a forged acknowledgement message is appended
//   - the next model request has tool_choice = Required
//   - threshold = 0 disables the behaviour entirely

/// Build a ProviderEvent script that emits one big ReasoningPart followed
/// by a single ToolCall and MessageEnd(ToolUse). Used to drive the
/// truncation path.
fn long_reasoning_then_tool_call_script(
    reasoning_chars: usize,
) -> Vec<mew_provider::ProviderEvent> {
    use mew_message::{
        Part, PartBase, ReasoningPart, ToolCallPart, ToolState, ToolStatePending, ToolTime,
    };
    let part_id = ulid::Ulid::new();
    let message_id = ulid::Ulid::new();
    let session_id = ulid::Ulid::new();
    let reasoning_text = "x".repeat(reasoning_chars);

    let mut events = vec![mew_provider::ProviderEvent::PartStart {
        part: Part::Reasoning(ReasoningPart {
            base: PartBase {
                id: part_id,
                message_id,
                session_id,
            },
            text: reasoning_text,
            signature: None,
        }),
    }];
    events.push(mew_provider::ProviderEvent::PartEnd { part_id });

    // Then a tool call (so the turn has something to do after truncation).
    let tc_id = ulid::Ulid::new();
    let tc_msg_id = ulid::Ulid::new();
    let tc_sess_id = ulid::Ulid::new();
    let now = chrono::Utc::now().timestamp_millis();
    events.push(mew_provider::ProviderEvent::PartStart {
        part: Part::ToolCall(ToolCallPart {
            base: PartBase {
                id: tc_id,
                message_id: tc_msg_id,
                session_id: tc_sess_id,
            },
            tool_name: "echo".into(),
            call_id: "call_1".into(),
            state: ToolState::Pending(ToolStatePending {
                input: serde_json::json!({"input": "hi"}),
                time: ToolTime {
                    start: now,
                    end: None,
                },
            }),
            raw_input: String::new(),
        }),
    });
    events.push(mew_provider::ProviderEvent::PartEnd { part_id: tc_id });
    events.push(mew_provider::ProviderEvent::MessageEnd {
        finish: Finish::ToolUse,
        usage: Tokens::default(),
        cost: 0.0,
    });
    events
}

#[tokio::test]
async fn test_long_reasoning_truncates_and_forges_ack() {
    let script1 = long_reasoning_then_tool_call_script(20_000); // ~5k tokens at 4 chars/tok
    let script2 = FakeProvider::text_response("done");
    // CapturingProvider pops scripts in order — first turn gets the
    // long-reasoning + tool-call, second turn gets a plain text
    // response (so the agent's turn loop terminates).
    let provider = std::sync::Arc::new(CapturingProvider::new(vec![script1, script2]));
    let mut agent = Agent::new(
        provider.clone(),
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );
    agent.set_reasoning_truncation_threshold(1000); // ~1000-token cap

    let mut rx = agent.run("think a lot".into());

    // Drain: PermissionRequest (EchoTool is mutating) → send AllowOnce →
    // MessageEnd → loop continues for the post-tool turn → second
    // MessageEnd. We pull events until we see two MessageEnds.
    let mut message_ends = 0;
    while let Some(ev) = rx.recv().await {
        if let AgentEvent::Provider(ProviderEvent::MessageEnd { .. }) = &ev {
            message_ends += 1;
            if message_ends >= 2 {
                break;
            }
        }
        if let AgentEvent::PermissionRequest { tx, .. } = ev {
            let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
        }
    }

    // The assistant message in self.messages must have truncated reasoning.
    // Capture the truncator flag BEFORE acquiring the messages lock — the
    // flag accessor borrows agent mutably while the lock guard borrows
    // immutably, and we'd hit a borrow-checker conflict otherwise.
    // The truncator's "force tool" flag has been consumed by the second
    // turn already (that's the point — it set tool_choice on the next
    // request). Verify via the captured requests: the second request
    // should have tool_choice = Some(Required).
    // Scope the sync lock so it's released before the async `agent.messages`
    // lock below (clippy: await-holding-lock). The assert borrows `captured`
    // only inside this block.
    {
        let captured = provider.captured.lock().unwrap();
        assert!(
            captured.len() >= 2,
            "agent should have made at least 2 model requests, got {}",
            captured.len()
        );
        let second_req = &captured[1];
        let tc = second_req
            .params
            .as_ref()
            .and_then(|p| p.tool_choice)
            .expect("second request must have tool_choice set (the truncator's force flag)");
        assert!(
            matches!(tc, mew_provider::ToolChoice::Required),
            "second request tool_choice must be Required, got {:?}",
            tc
        );
    }

    let msgs = agent.messages.lock().await;
    let assistant = msgs
        .iter()
        .find(|m| {
            m.role == Role::Assistant && m.parts.iter().any(|p| matches!(p, Part::Reasoning(_)))
        })
        .expect("assistant message with reasoning should exist");
    let reasoning_text = assistant
        .parts
        .iter()
        .find_map(|p| match p {
            Part::Reasoning(rp) => Some(rp.text.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        reasoning_text.contains("truncated at ~1000 tokens"),
        "reasoning should be truncated with marker: len={}, head={:?}",
        reasoning_text.len(),
        &reasoning_text[..80.min(reasoning_text.len())]
    );
    assert!(
        reasoning_text.len() < 20_000,
        "truncated reasoning should be smaller than the original 20k chars, got {}",
        reasoning_text.len()
    );

    // The forged acknowledgement message must be present in history.
    let has_ack = msgs.iter().any(|m| {
        m.role == Role::Assistant
            && m.parts.iter().any(
                |p| matches!(p, Part::Text(tp) if tp.text.contains("Acknowledging overthinking")),
            )
    });
    assert!(
        has_ack,
        "forged acknowledgement assistant message must be in history"
    );
}

#[tokio::test]
async fn test_short_reasoning_does_not_trigger_truncation() {
    // ~200 tokens — well under the default 5k threshold.
    let script1 = long_reasoning_then_tool_call_script(800);
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
    let mut agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );
    // Default threshold (5000). Don't change it.

    let mut rx = agent.run("hi".into());
    let mut message_ends = 0;
    while let Some(ev) = rx.recv().await {
        if let AgentEvent::Provider(ProviderEvent::MessageEnd { .. }) = &ev {
            message_ends += 1;
            if message_ends >= 2 {
                break;
            }
        }
        if let AgentEvent::PermissionRequest { tx, .. } = ev {
            let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
        }
    }

    let force_tool_flag = agent.take_force_tool_choice();

    // Reasoning must be intact (not truncated).
    let msgs = agent.messages.lock().await;
    let reasoning_text = msgs
        .iter()
        .flat_map(|m| m.parts.iter())
        .find_map(|p| match p {
            Part::Reasoning(rp) => Some(rp.text.clone()),
            _ => None,
        })
        .expect("reasoning part should exist");
    assert!(
        !reasoning_text.contains("truncated at ~"),
        "short reasoning must not be truncated, got len={}",
        reasoning_text.len()
    );
    assert_eq!(reasoning_text.len(), 800);

    // No acknowledgement was forged.
    let has_ack = msgs.iter().any(|m| {
        m.role == Role::Assistant
            && m.parts.iter().any(
                |p| matches!(p, Part::Text(tp) if tp.text.contains("Acknowledging overthinking")),
            )
    });
    assert!(
        !has_ack,
        "no acknowledgement should be forged for short reasoning"
    );

    // No force-tool flag.
    assert!(!force_tool_flag);
}

#[tokio::test]
async fn test_truncation_disabled_when_threshold_zero() {
    // Long reasoning but threshold = 0 disables truncation.
    let script1 = long_reasoning_then_tool_call_script(20_000);
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
    let mut agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );
    agent.set_reasoning_truncation_threshold(0);

    let mut rx = agent.run("hi".into());
    let mut message_ends = 0;
    while let Some(ev) = rx.recv().await {
        if let AgentEvent::Provider(ProviderEvent::MessageEnd { .. }) = &ev {
            message_ends += 1;
            if message_ends >= 2 {
                break;
            }
        }
        if let AgentEvent::PermissionRequest { tx, .. } = ev {
            let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
        }
    }

    let force_tool_flag = agent.take_force_tool_choice();
    let msgs = agent.messages.lock().await;
    let reasoning_text = msgs
        .iter()
        .flat_map(|m| m.parts.iter())
        .find_map(|p| match p {
            Part::Reasoning(rp) => Some(rp.text.clone()),
            _ => None,
        })
        .expect("reasoning should exist");
    assert_eq!(
        reasoning_text.len(),
        20_000,
        "threshold=0 disables truncation; reasoning must be intact"
    );
    assert!(!force_tool_flag);
}

#[tokio::test]
async fn test_set_reasoning_truncation_disabled_master_switch() {
    // Master switch off overrides a high threshold.
    let script1 = long_reasoning_then_tool_call_script(20_000);
    let script2 = FakeProvider::text_response("done");
    let provider = std::sync::Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
    let mut agent = Agent::new(
        provider,
        std::sync::Arc::new(NopDispatcher),
        None,
        vec![std::sync::Arc::new(EchoTool::mutating())],
        None,
    );
    agent.set_reasoning_truncation_threshold(100);
    agent.set_reasoning_truncation_enabled(false);

    let mut rx = agent.run("hi".into());
    let mut message_ends = 0;
    while let Some(ev) = rx.recv().await {
        if let AgentEvent::Provider(ProviderEvent::MessageEnd { .. }) = &ev {
            message_ends += 1;
            if message_ends >= 2 {
                break;
            }
        }
        if let AgentEvent::PermissionRequest { tx, .. } = ev {
            let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
        }
    }

    let msgs = agent.messages.lock().await;
    let reasoning_text = msgs
        .iter()
        .flat_map(|m| m.parts.iter())
        .find_map(|p| match p {
            Part::Reasoning(rp) => Some(rp.text.clone()),
            _ => None,
        })
        .expect("reasoning should exist");
    assert_eq!(
        reasoning_text.len(),
        20_000,
        "master switch off must skip truncation"
    );
}

// --------------------------------------------------------------------------
// Default `max_output_tokens` integration
// --------------------------------------------------------------------------
//
// These exercise the agent's default-max-output wiring: the catalog
// provides a value (capped at 32K), the turn loop injects it into the
// request unless the dispatcher/plugin overrides, and a user-set
// `set_default_max_output_tokens` overrides everything.

#[tokio::test]
async fn test_set_default_max_output_tokens_basic_setter() {
    let mut agent = Agent::new(
        std::sync::Arc::new(StatefulFakeProvider::new(vec![
            FakeProvider::text_response("ok"),
        ])),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        None,
    );
    assert_eq!(agent.default_max_output_tokens, 0);
    agent.set_default_max_output_tokens(8192);
    assert_eq!(agent.default_max_output_tokens, 8192);
}

#[tokio::test]
async fn test_set_default_max_output_tokens_clamps_negative_to_zero() {
    let mut agent = Agent::new(
        std::sync::Arc::new(StatefulFakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        None,
    );
    agent.set_default_max_output_tokens(-1);
    assert_eq!(
        agent.default_max_output_tokens, 0,
        "negative must clamp to 0"
    );

    agent.set_default_max_output_tokens(-1_000_000);
    assert_eq!(agent.default_max_output_tokens, 0);
}

#[tokio::test]
async fn test_set_default_max_output_tokens_saturates_huge_via_field() {
    // The setter itself doesn't saturate (we want to keep the full
    // value for diagnostics / future fields); saturation happens at
    // the turn.rs call site. Verify the field holds the full i64 value.
    let mut agent = Agent::new(
        std::sync::Arc::new(StatefulFakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        None,
    );
    agent.set_default_max_output_tokens(i64::MAX);
    assert_eq!(agent.default_max_output_tokens, i64::MAX);
}

#[tokio::test]
async fn test_set_default_max_output_tokens_zero_disables_override() {
    // Setter 0 → request's max_tokens is None (no override).
    let script = FakeProvider::text_response("ok");
    let provider = std::sync::Arc::new(CapturingProvider::new(vec![script]));
    let mut agent = Agent::new(
        provider.clone(),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        None,
    );
    agent.set_default_max_output_tokens(0);

    let mut rx = agent.run("hi".into());
    while rx.recv().await.is_some() {}

    let captured = provider.captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert!(
        captured[0]
            .params
            .as_ref()
            .and_then(|p| p.max_tokens)
            .is_none(),
        "setter(0) must disable override — request.max_tokens should be None"
    );
}

#[tokio::test]
async fn test_turn_loop_injects_default_max_output_into_request() {
    let script = FakeProvider::text_response("ok");
    let provider = std::sync::Arc::new(CapturingProvider::new(vec![script]));
    let mut agent = Agent::new(
        provider.clone(),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        None,
    );
    agent.set_default_max_output_tokens(16384);

    let mut rx = agent.run("hi".into());
    while rx.recv().await.is_some() {}

    let captured = provider.captured.lock().unwrap();
    let max = captured[0]
        .params
        .as_ref()
        .and_then(|p| p.max_tokens)
        .expect("default must be injected when dispatcher returns None");
    assert_eq!(max, 16384, "agent's default must reach the request");
}

#[tokio::test]
async fn test_user_dispatcher_max_tokens_wins_over_default() {
    // Build a dispatcher that overrides max_tokens. The agent's default
    // must NOT win. We construct a wrapper that holds a NopDispatcher
    // and overrides on_chat_params / on_chat_headers; the rest of the
    // 23 trait methods are pass-throughs.
    use mew_hooks::ChatParams;

    struct OverrideDispatcher {
        inner: NopDispatcher,
    }

    async fn pass_through(d: &OverrideDispatcher) {
        // Suppress the unused field warning on `inner` for these tests
        // — we use it for the field access in every method.
        let _ = &d.inner;
    }
    let _ = pass_through; // function exists only to suppress lint

    #[async_trait::async_trait]
    impl mew_hooks::Dispatcher for OverrideDispatcher {
        async fn on_chat_params(&self, _params: ChatParams) -> ChatParams {
            ChatParams {
                temperature: None,
                top_p: None,
                max_tokens: Some(1234),
                tool_choice: None,
            }
        }
        async fn on_chat_headers(&self, headers: http::HeaderMap) -> http::HeaderMap {
            headers
        }
        async fn init(&self, _: &mew_hooks::PluginHost) {}
        async fn shutdown(&self) {}
        async fn on_register_tools(&self) -> Vec<mew_hooks::ToolRegistration> {
            self.inner.on_register_tools().await
        }
        async fn on_register_slash_commands(&self) -> Vec<mew_hooks::SlashCommandDef> {
            self.inner.on_register_slash_commands().await
        }
        async fn execute_slash_command(&self, c: &str, a: &str) -> Option<String> {
            self.inner.execute_slash_command(c, a).await
        }
        async fn on_provider_event(&self, ev: &mew_provider::ProviderEvent) {
            self.inner.on_provider_event(ev).await
        }
        async fn on_tool_error(&self, c: &mew_hooks::ToolCall, e: &str) {
            self.inner.on_tool_error(c, e).await
        }
        async fn on_subagent_start(&self, n: &str, p: &str, d: Option<&str>) {
            self.inner.on_subagent_start(n, p, d).await
        }
        async fn on_subagent_end(&self, n: &str, p: &str, o: &str) {
            self.inner.on_subagent_end(n, p, o).await
        }
        async fn on_turn_end(&self, m: &[mew_message::Message]) {
            self.inner.on_turn_end(m).await
        }
        async fn on_pre_model_turn(&self, m: &[mew_message::Message], s: &str) {
            self.inner.on_pre_model_turn(m, s).await
        }
        async fn on_stop(&self) {
            self.inner.on_stop().await
        }
        async fn on_pre_compaction(&self, m: &[mew_message::Message]) {
            self.inner.on_pre_compaction(m).await
        }
        async fn on_post_compaction(&self, m: &[mew_message::Message]) {
            self.inner.on_post_compaction(m).await
        }
        async fn on_chat_message(&self, msg: mew_message::Message) -> mew_message::Message {
            self.inner.on_chat_message(msg).await
        }
        async fn on_system_prompt(&self, p: String) -> String {
            self.inner.on_system_prompt(p).await
        }
        async fn on_tool_execute_before(
            &self,
            c: &mew_hooks::ToolCall,
            v: serde_json::Value,
        ) -> mew_hooks::HookOutcome<serde_json::Value> {
            self.inner.on_tool_execute_before(c, v).await
        }
        async fn on_tool_execute_after(
            &self,
            c: &mew_hooks::ToolCall,
            o: mew_hooks::ToolOutput,
        ) -> mew_hooks::ToolOutput {
            self.inner.on_tool_execute_after(c, o).await
        }
        async fn on_permission_ask(
            &self,
            c: &mew_hooks::ToolCall,
            d: mew_hooks::PermissionDecision,
        ) -> mew_hooks::HookOutcome<mew_hooks::PermissionDecision> {
            self.inner.on_permission_ask(c, d).await
        }
        async fn on_shell_env(
            &self,
            e: std::collections::HashMap<String, String>,
        ) -> std::collections::HashMap<String, String> {
            self.inner.on_shell_env(e).await
        }
        async fn on_user_input(&self, p: String) -> String {
            self.inner.on_user_input(p).await
        }
        async fn on_persona_change(&self, o: Option<&str>, n: &str) {
            self.inner.on_persona_change(o, n).await
        }
        async fn on_session_save(&self) {
            self.inner.on_session_save().await
        }
        async fn on_model_finish(&self, f: &str, i: u32, o: u32, c: f64) {
            self.inner.on_model_finish(f, i, o, c).await
        }
    }

    let script = FakeProvider::text_response("ok");
    let provider = std::sync::Arc::new(CapturingProvider::new(vec![script]));
    let dispatcher = std::sync::Arc::new(OverrideDispatcher {
        inner: NopDispatcher,
    });
    let mut agent = Agent::new(provider.clone(), dispatcher, None, Vec::new(), None);
    agent.set_default_max_output_tokens(16384);

    let mut rx = agent.run("hi".into());
    while rx.recv().await.is_some() {}

    let captured = provider.captured.lock().unwrap();
    let max = captured[0]
        .params
        .as_ref()
        .and_then(|p| p.max_tokens)
        .expect("dispatcher must have set max_tokens");
    assert_eq!(
        max, 1234,
        "dispatcher's explicit max_tokens must win over agent default"
    );
}

#[tokio::test]
async fn test_dispatcher_some_zero_is_honored_at_request_level() {
    // Some(0) is a valid request-level override. The Anthropic
    // adapter's wire floor is its problem (see Anthropic tests); the
    // agent itself must NOT silently convert Some(0) → None.
    use mew_hooks::ChatParams;

    struct ZeroDispatcher {
        inner: NopDispatcher,
    }

    #[async_trait::async_trait]
    impl mew_hooks::Dispatcher for ZeroDispatcher {
        async fn on_chat_params(&self, _: ChatParams) -> ChatParams {
            ChatParams {
                temperature: None,
                top_p: None,
                max_tokens: Some(0),
                tool_choice: None,
            }
        }
        async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap {
            h
        }
        async fn init(&self, _: &mew_hooks::PluginHost) {}
        async fn shutdown(&self) {}
        async fn on_register_tools(&self) -> Vec<mew_hooks::ToolRegistration> {
            self.inner.on_register_tools().await
        }
        async fn on_register_slash_commands(&self) -> Vec<mew_hooks::SlashCommandDef> {
            self.inner.on_register_slash_commands().await
        }
        async fn execute_slash_command(&self, c: &str, a: &str) -> Option<String> {
            self.inner.execute_slash_command(c, a).await
        }
        async fn on_provider_event(&self, ev: &mew_provider::ProviderEvent) {
            self.inner.on_provider_event(ev).await
        }
        async fn on_tool_error(&self, c: &mew_hooks::ToolCall, e: &str) {
            self.inner.on_tool_error(c, e).await
        }
        async fn on_subagent_start(&self, n: &str, p: &str, d: Option<&str>) {
            self.inner.on_subagent_start(n, p, d).await
        }
        async fn on_subagent_end(&self, n: &str, p: &str, o: &str) {
            self.inner.on_subagent_end(n, p, o).await
        }
        async fn on_turn_end(&self, m: &[mew_message::Message]) {
            self.inner.on_turn_end(m).await
        }
        async fn on_pre_model_turn(&self, m: &[mew_message::Message], s: &str) {
            self.inner.on_pre_model_turn(m, s).await
        }
        async fn on_stop(&self) {
            self.inner.on_stop().await
        }
        async fn on_pre_compaction(&self, m: &[mew_message::Message]) {
            self.inner.on_pre_compaction(m).await
        }
        async fn on_post_compaction(&self, m: &[mew_message::Message]) {
            self.inner.on_post_compaction(m).await
        }
        async fn on_chat_message(&self, msg: mew_message::Message) -> mew_message::Message {
            self.inner.on_chat_message(msg).await
        }
        async fn on_system_prompt(&self, p: String) -> String {
            self.inner.on_system_prompt(p).await
        }
        async fn on_tool_execute_before(
            &self,
            c: &mew_hooks::ToolCall,
            v: serde_json::Value,
        ) -> mew_hooks::HookOutcome<serde_json::Value> {
            self.inner.on_tool_execute_before(c, v).await
        }
        async fn on_tool_execute_after(
            &self,
            c: &mew_hooks::ToolCall,
            o: mew_hooks::ToolOutput,
        ) -> mew_hooks::ToolOutput {
            self.inner.on_tool_execute_after(c, o).await
        }
        async fn on_permission_ask(
            &self,
            c: &mew_hooks::ToolCall,
            d: mew_hooks::PermissionDecision,
        ) -> mew_hooks::HookOutcome<mew_hooks::PermissionDecision> {
            self.inner.on_permission_ask(c, d).await
        }
        async fn on_shell_env(
            &self,
            e: std::collections::HashMap<String, String>,
        ) -> std::collections::HashMap<String, String> {
            self.inner.on_shell_env(e).await
        }
        async fn on_user_input(&self, p: String) -> String {
            self.inner.on_user_input(p).await
        }
        async fn on_persona_change(&self, o: Option<&str>, n: &str) {
            self.inner.on_persona_change(o, n).await
        }
        async fn on_session_save(&self) {
            self.inner.on_session_save().await
        }
        async fn on_model_finish(&self, f: &str, i: u32, o: u32, c: f64) {
            self.inner.on_model_finish(f, i, o, c).await
        }
    }

    let script = FakeProvider::text_response("ok");
    let provider = std::sync::Arc::new(CapturingProvider::new(vec![script]));
    let dispatcher = std::sync::Arc::new(ZeroDispatcher {
        inner: NopDispatcher,
    });
    let mut agent = Agent::new(provider.clone(), dispatcher, None, Vec::new(), None);
    agent.set_default_max_output_tokens(16384); // would normally win

    let mut rx = agent.run("hi".into());
    while rx.recv().await.is_some() {}

    let captured = provider.captured.lock().unwrap();
    let max = captured[0]
        .params
        .as_ref()
        .and_then(|p| p.max_tokens)
        .expect("dispatcher's Some(0) must reach the request");
    assert_eq!(
        max, 0,
        "Some(0) must be honored verbatim, not converted to None"
    );
}

#[tokio::test]
async fn test_default_max_output_saturates_to_i32_in_request() {
    // Setter stores i64, turn.rs clamps to i32. Verify a value > i32::MAX
    // saturates rather than panics or overflows.
    let script = FakeProvider::text_response("ok");
    let provider = std::sync::Arc::new(CapturingProvider::new(vec![script]));
    let mut agent = Agent::new(
        provider.clone(),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        None,
    );
    agent.set_default_max_output_tokens(i64::MAX);

    let mut rx = agent.run("hi".into());
    while rx.recv().await.is_some() {}

    let captured = provider.captured.lock().unwrap();
    let max = captured[0]
        .params
        .as_ref()
        .and_then(|p| p.max_tokens)
        .expect("huge default must be injected (saturated)");
    assert_eq!(
        max,
        i32::MAX,
        "huge default must saturate to i32::MAX in the request"
    );
}

#[tokio::test]
async fn test_default_max_output_caps_at_32k_logic_directly() {
    // This is a tiny unit-style test: the 32K cap is the
    // build_session_agent logic (`raw_max_output.min(32_768)`), not a
    // field invariant. The agent's setter doesn't apply the cap (it
    // stores whatever the user passed). Verify both halves of the
    // contract via a helper function we don't have to call through
    // build_session_agent here.
    let mut agent = Agent::new(
        std::sync::Arc::new(StatefulFakeProvider::new(vec![])),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        None,
    );
    // Simulate the cap that build_session_agent would apply to a
    // catalog value of 128K.
    let raw: i64 = 128_000;
    let capped = raw.min(32_768);
    agent.set_default_max_output_tokens(capped);
    assert_eq!(agent.default_max_output_tokens, 32_768);
}

// --------------------------------------------------------------------------
// Fallback models
// --------------------------------------------------------------------------
//
// When the primary provider returns a stream error and the active persona
// has `fallback_models` configured, the turn loop tries each fallback in
// order. The first that succeeds is used for the rest of the turn.

/// A provider that always returns an error on `stream`.
struct ErroringProvider;

#[async_trait::async_trait]
impl Provider for ErroringProvider {
    fn name(&self) -> &str {
        "erroring"
    }
    async fn stream(&self, _req: Request) -> Result<EventStream, ProviderError> {
        Err(ProviderError::Message("primary provider down".into()))
    }
}

#[tokio::test]
async fn test_fallback_model_retries_on_stream_error() {
    use mew_message::SessionId;

    // Primary provider always errors; the fallback succeeds with a text
    // response.
    let fallback_provider = std::sync::Arc::new(FakeProvider::new(FakeProvider::text_response(
        "fallback response",
    )));
    let fallback_clone = fallback_provider.clone();

    let mut agent = Agent::new(
        std::sync::Arc::new(ErroringProvider),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        Some(SessionId::default()),
    );
    // Configure fallback models + the provider builder.
    agent.fallback_models = Some(vec!["z-ai/glm-4.5-air".to_string()]);
    agent.set_provider_builder(Box::new(move |_model_str: &str| {
        Ok(fallback_clone.clone() as std::sync::Arc<dyn Provider>)
    }));

    let mut rx = agent.run("hello".into());
    let mut got_text = String::new();
    let mut had_error = false;
    while let Some(ev) = rx.recv().await {
        match ev {
            crate::AgentEvent::Provider(ProviderEvent::PartDelta { delta, .. }) => {
                got_text.push_str(&delta);
            }
            crate::AgentEvent::Error(msg) if msg.contains("trying fallback") => {
                had_error = true;
            }
            crate::AgentEvent::Provider(ProviderEvent::MessageEnd { .. }) => break,
            crate::AgentEvent::Error(_) => {}
            _ => {}
        }
    }
    assert!(
        had_error,
        "should have emitted a 'trying fallback' error event"
    );
    assert!(
        got_text.contains("fallback response"),
        "should have received the fallback response, got: {got_text}"
    );
}

#[tokio::test]
async fn test_no_fallback_models_means_fatal_error() {
    use mew_message::SessionId;

    let agent = Agent::new(
        std::sync::Arc::new(ErroringProvider),
        std::sync::Arc::new(NopDispatcher),
        None,
        Vec::new(),
        Some(SessionId::default()),
    );
    // No fallback_models configured — the error should be fatal.
    let mut rx = agent.run("hello".into());
    let mut got_fatal = false;
    while let Some(ev) = rx.recv().await {
        if let crate::AgentEvent::Error(msg) = ev {
            if msg.contains("provider stream") {
                got_fatal = true;
            }
        }
    }
    assert!(
        got_fatal,
        "should have emitted a fatal 'provider stream' error"
    );
}
