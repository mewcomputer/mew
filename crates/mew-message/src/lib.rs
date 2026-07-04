use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub type MessageId = Ulid;
pub type SessionId = Ulid;
pub type PartId = Ulid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    pub role: Role,
    pub parts: Vec<Part>,
    pub time: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant: Option<AssistantMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Time {
    pub created: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMeta {
    pub provider_id: String,
    pub model_id: String,
    pub cost: f64,
    pub tokens: Tokens,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<Finish>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<MessageError>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Tokens {
    pub input: u32,
    pub output: u32,
    pub reasoning: u32,
    pub cache_read: u32,
    pub cache_write: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    #[serde(alias = "Stop")]
    Stop,
    #[serde(alias = "Length")]
    Length,
    #[serde(alias = "ToolUse")]
    ToolUse,
    #[serde(alias = "Error")]
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    #[serde(alias = "ProviderAuth")]
    ProviderAuth,
    #[serde(alias = "ProviderRateLimit")]
    ProviderRateLimit,
    #[serde(alias = "ProviderOverload")]
    ProviderOverload,
    #[serde(alias = "ProviderApi")]
    ProviderApi,
    #[serde(alias = "ContextOverflow")]
    ContextOverflow,
    #[serde(alias = "Aborted")]
    Aborted,
    #[serde(alias = "ToolExec")]
    ToolExec,
    #[serde(alias = "ToolTimeout")]
    ToolTimeout,
    #[serde(alias = "McpTransport")]
    McpTransport,
    #[serde(alias = "Network")]
    Network,
    #[serde(alias = "Unknown")]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text(TextPart),
    Reasoning(ReasoningPart),
    File(FilePart),
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
    Compaction(CompactionPart),
}

impl Part {
    pub fn id(&self) -> PartId {
        match self {
            Part::Text(p) => p.base.id,
            Part::Reasoning(p) => p.base.id,
            Part::File(p) => p.base.id,
            Part::ToolCall(p) => p.base.id,
            Part::ToolResult(p) => p.base.id,
            Part::Compaction(p) => p.base.id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartBase {
    pub id: Ulid,
    pub message_id: MessageId,
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub text: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FilePart {
    #[serde(flatten)]
    pub base: PartBase,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub tool_name: String,
    pub call_id: String,
    pub state: ToolState,
    #[serde(skip)]
    pub raw_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactionPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub auto: bool,
    #[serde(default)]
    pub overflow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_start_id: Option<MessageId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolState {
    Pending(ToolStatePending),
    Running(ToolStateRunning),
    Completed(ToolStateCompleted),
    Error(ToolStateError),
}

impl ToolState {
    pub fn input(&self) -> &serde_json::Value {
        match self {
            ToolState::Pending(s) => &s.input,
            ToolState::Running(s) => &s.input,
            ToolState::Completed(s) => &s.input,
            ToolState::Error(s) => &s.input,
        }
    }

    /// Replace the input on the current variant, leaving other fields
    /// (output, error, time) untouched.
    pub fn set_input(&mut self, value: serde_json::Value) {
        match self {
            ToolState::Pending(s) => s.input = value,
            ToolState::Running(s) => s.input = value,
            ToolState::Completed(s) => s.input = value,
            ToolState::Error(s) => s.input = value,
        }
    }

    pub fn output(&self) -> Option<&str> {
        match self {
            ToolState::Running(s) => Some(&s.output),
            ToolState::Completed(s) => Some(&s.output),
            _ => None,
        }
    }

    /// Returns the error message if the tool is in an error state.
    pub fn error(&self) -> Option<&str> {
        match self {
            ToolState::Error(s) => Some(&s.error),
            _ => None,
        }
    }

    /// Returns the text a provider should send back to the model as the
    /// tool result content: the output on success, the error message on
    /// failure. Never empty when the tool has reached a terminal state.
    pub fn result_content(&self) -> Option<&str> {
        match self {
            ToolState::Completed(s) => Some(&s.output),
            ToolState::Error(s) => Some(&s.error),
            _ => None,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, ToolState::Error(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolStatePending {
    pub input: serde_json::Value,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolStateRunning {
    pub input: serde_json::Value,
    #[serde(default)]
    pub output: String,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolStateCompleted {
    pub input: serde_json::Value,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolStateError {
    pub input: serde_json::Value,
    pub error: String,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolTime {
    pub start: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}

// ---------------------------------------------------------------------------
// ProviderEventWire — serializable mirror of mew_provider::ProviderEvent
// ---------------------------------------------------------------------------

/// A wire-serializable representation of `ProviderEvent`.
///
/// `ProviderEvent` uses `&'static str` for the `field` parameter, which
/// doesn't round-trip through serde. This mirror uses `String` so it
/// serializes cleanly for the daemon protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderEventWire {
    PartStart {
        part: Part,
    },
    PartDelta {
        part_id: PartId,
        field: String,
        delta: String,
    },
    PartEnd {
        part_id: PartId,
    },
    MessageEnd {
        finish: Finish,
        usage: Tokens,
        cost: f64,
    },
    RetryWait {
        attempt: u32,
        max_attempts: u32,
        delay_secs: u64,
        reason: String,
    },
    Error(MessageError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn proptest_message_roundtrip() {
        use proptest::prelude::*;

        proptest!(ProptestConfig::with_cases(100), |(text in ".*", created in 0i64..1_800_000_000_000i64)| {
            let sid = Ulid::new();
            let mid = Ulid::new();
            let m = Message {
                id: mid,
                session_id: sid,
                role: Role::User,
                parts: vec![Part::Text(TextPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id: mid,
                        session_id: sid,
                    },
                    text: text.clone(),
                    synthetic: false,
                })],
                time: Time {
                    created,
                    completed: None,
                },
                assistant: None,
            };
            let s = serde_json::to_string(&m).expect("serialize");
            let rt: Message = serde_json::from_str(&s).expect("deserialize");
            assert_eq!(rt.id, m.id);
            assert_eq!(rt.session_id, m.session_id);
            assert_eq!(rt.role, m.role);
            assert_eq!(rt.time.created, m.time.created);
            assert_eq!(rt.time.completed, m.time.completed);
            assert_eq!(rt.parts.len(), 1);
        });
    }

    fn base(sid: SessionId, mid: MessageId) -> PartBase {
        PartBase {
            id: Ulid::new(),
            message_id: mid,
            session_id: sid,
        }
    }

    fn msg(role: Role, sid: SessionId, parts: Vec<Part>) -> Message {
        let id = Ulid::new();
        Message {
            id,
            session_id: sid,
            role,
            parts,
            time: Time {
                created: 1700000000000,
                completed: None,
            },
            assistant: None,
        }
    }

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        name: &str,
        val: &T,
    ) {
        let s = serde_json::to_string(val).expect("serialize");
        let rt: T = serde_json::from_str(&s).unwrap_or_else(|e| {
            panic!("{name}: deserialize failed: {e}\njson: {s}");
        });
        assert_eq!(&rt, val, "{name}: round-trip mismatch\nserialized: {s}");
    }

    // -----------------------------------------------------------------------
    // Table-driven Part round-trip tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_part_text_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::Text(TextPart {
            base: base(sid, mid),
            text: "hello world".into(),
            synthetic: false,
        });
        roundtrip("text", &p);
    }

    #[test]
    fn test_part_reasoning_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::Reasoning(ReasoningPart {
            base: base(sid, mid),
            text: "let me think...".into(),
            signature: Some("sig123".into()),
        });
        roundtrip("reasoning with sig", &p);
    }

    #[test]
    fn test_part_reasoning_no_signature_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::Reasoning(ReasoningPart {
            base: base(sid, mid),
            text: "thinking".into(),
            signature: None,
        });
        roundtrip("reasoning no sig", &p);
    }

    #[test]
    fn test_part_file_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::File(FilePart {
            base: base(sid, mid),
            mime: "image/png".into(),
            filename: Some("screenshot.png".into()),
            url: "file:///tmp/img.png".into(),
        });
        roundtrip("file", &p);
    }

    #[test]
    fn test_part_file_minimal_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::File(FilePart {
            base: base(sid, mid),
            mime: "text/plain".into(),
            filename: None,
            url: "file:///tmp/foo.txt".into(),
        });
        roundtrip("file minimal", &p);
    }

    #[test]
    fn test_part_tool_call_pending_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::ToolCall(ToolCallPart {
            base: base(sid, mid),
            tool_name: "read".into(),
            call_id: "call_abc".into(),
            state: ToolState::Pending(ToolStatePending {
                input: json!({"path": "/tmp/foo"}),
                time: ToolTime {
                    start: 1700000001000,
                    end: None,
                },
            }),
            raw_input: String::new(),
        });
        roundtrip("tool call pending", &p);
    }

    #[test]
    fn test_part_tool_call_running_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::ToolCall(ToolCallPart {
            base: base(sid, mid),
            tool_name: "bash".into(),
            call_id: "call_def".into(),
            state: ToolState::Running(ToolStateRunning {
                input: json!({"command": "ls"}),
                output: "file1\nfile2\n".into(),
                time: ToolTime {
                    start: 1700000001000,
                    end: None,
                },
            }),
            raw_input: String::new(),
        });
        roundtrip("tool call running", &p);
    }

    #[test]
    fn test_part_tool_call_completed_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::ToolCall(ToolCallPart {
            base: base(sid, mid),
            tool_name: "grep".into(),
            call_id: "call_ghi".into(),
            state: ToolState::Completed(ToolStateCompleted {
                input: json!({"pattern": "foo"}),
                output: "foo.rs:12: foo()\n".into(),
                metadata: Some(json!({"matches": 1})),
                diff: None,
                time: ToolTime {
                    start: 1700000001000,
                    end: Some(1700000002000),
                },
            }),
            raw_input: String::new(),
        });
        roundtrip("tool call completed", &p);
    }

    #[test]
    fn test_part_tool_call_error_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::ToolCall(ToolCallPart {
            base: base(sid, mid),
            tool_name: "bash".into(),
            call_id: "call_jkl".into(),
            state: ToolState::Error(ToolStateError {
                input: json!({"command": "rm"}),
                error: "permission denied".into(),
                time: ToolTime {
                    start: 1700000001000,
                    end: Some(1700000001500),
                },
            }),
            raw_input: String::new(),
        });
        roundtrip("tool call error", &p);
    }

    #[test]
    fn test_part_tool_result_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::ToolResult(ToolResultPart {
            base: base(sid, mid),
            call_id: "call_abc".into(),
        });
        roundtrip("tool result", &p);
    }

    #[test]
    fn test_part_compaction_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let p = Part::Compaction(CompactionPart {
            base: base(sid, mid),
            auto: true,
            overflow: false,
            tail_start_id: Some(Ulid::new()),
        });
        roundtrip("compaction", &p);
    }

    // -----------------------------------------------------------------------
    // Full message round-trip test (multi-part, with reasoning, tool calls)
    // -----------------------------------------------------------------------

    #[test]
    fn test_message_text_only_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let m = Message {
            id: mid,
            session_id: sid,
            role: Role::User,
            parts: vec![Part::Text(TextPart {
                base: base(sid, mid),
                text: "hello".into(),
                synthetic: false,
            })],
            time: Time {
                created: 1700000000000,
                completed: Some(1700000001000),
            },
            assistant: None,
        };
        roundtrip("msg text only", &m);
    }

    #[test]
    fn test_message_assistant_with_tool_calls_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let m = Message {
            id: mid,
            session_id: sid,
            role: Role::Assistant,
            parts: vec![
                Part::Reasoning(ReasoningPart {
                    base: base(sid, mid),
                    text: "need to read the file".into(),
                    signature: Some("sig_xyz".into()),
                }),
                Part::Text(TextPart {
                    base: base(sid, mid),
                    text: "Let me check that file.".into(),
                    synthetic: false,
                }),
                Part::ToolCall(ToolCallPart {
                    base: base(sid, mid),
                    tool_name: "read".into(),
                    call_id: "call_123".into(),
                    state: ToolState::Completed(ToolStateCompleted {
                        input: json!({"path": "/tmp/test.txt"}),
                        output: "file contents".into(),
                        metadata: None,
                        diff: None,
                        time: ToolTime {
                            start: 1700000001000,
                            end: Some(1700000002000),
                        },
                    }),
                    raw_input: String::new(),
                }),
            ],
            time: Time {
                created: 1700000000000,
                completed: Some(1700000002000),
            },
            assistant: Some(AssistantMeta {
                provider_id: "test-provider".into(),
                model_id: "test-model".into(),
                cost: 0.005,
                tokens: Tokens {
                    input: 10,
                    output: 20,
                    reasoning: 5,
                    cache_read: 0,
                    cache_write: 0,
                },
                finish: Some(Finish::ToolUse),
                error: None,
            }),
        };
        roundtrip("msg with tool calls", &m);
    }

    #[test]
    fn test_message_with_error_roundtrip() {
        let sid = Ulid::new();
        let mid = Ulid::new();
        let m = Message {
            id: mid,
            session_id: sid,
            role: Role::Assistant,
            parts: vec![Part::Text(TextPart {
                base: base(sid, mid),
                text: "partial response...".into(),
                synthetic: false,
            })],
            time: Time {
                created: 1700000000000,
                completed: Some(1700000001000),
            },
            assistant: Some(AssistantMeta {
                provider_id: "test".into(),
                model_id: "test".into(),
                cost: 0.0,
                tokens: Tokens::default(),
                finish: None,
                error: Some(MessageError {
                    kind: ErrorKind::Aborted,
                    message: "cancelled".into(),
                }),
            }),
        };
        roundtrip("msg with error", &m);
    }

    // -----------------------------------------------------------------------
    // Role round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_role_user_roundtrip() {
        roundtrip("role user", &Role::User);
    }

    #[test]
    fn test_role_assistant_roundtrip() {
        roundtrip("role assistant", &Role::Assistant);
    }

    // -----------------------------------------------------------------------
    // ErrorKind round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_kinds_roundtrip() {
        let kinds = [
            ErrorKind::ProviderAuth,
            ErrorKind::ProviderRateLimit,
            ErrorKind::ProviderOverload,
            ErrorKind::ProviderApi,
            ErrorKind::ContextOverflow,
            ErrorKind::Aborted,
            ErrorKind::ToolExec,
            ErrorKind::ToolTimeout,
            ErrorKind::McpTransport,
            ErrorKind::Network,
            ErrorKind::Unknown,
        ];
        for kind in &kinds {
            roundtrip(&format!("error kind {:?}", kind), kind);
        }
    }

    // -----------------------------------------------------------------------
    // Known JSON fixture round-trip (ensures existing serialization is stable)
    // -----------------------------------------------------------------------

    #[test]
    fn test_deserialize_known_text_part_json() {
        let json = r#"{
            "type": "text",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "text": "hello world",
            "synthetic": false
        }"#;
        let part: Part = serde_json::from_str(json).expect("deserialize text part");
        match &part {
            Part::Text(tp) => assert_eq!(tp.text, "hello world"),
            _ => panic!("expected Text part"),
        }
    }

    #[test]
    fn test_deserialize_known_tool_call_pending_json() {
        let json = r#"{
            "type": "tool_call",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "tool_name": "read",
            "call_id": "call_123",
            "state": {
                "status": "pending",
                "input": {"path": "/tmp/foo"},
                "time": {"start": 1700000000000}
            }
        }"#;
        let part: Part = serde_json::from_str(json).expect("deserialize tool call pending");
        match &part {
            Part::ToolCall(tc) => {
                assert_eq!(tc.tool_name, "read");
                assert!(matches!(tc.state, ToolState::Pending(_)));
            }
            _ => panic!("expected ToolCall part"),
        }
    }

    #[test]
    fn test_deserialize_known_tool_call_completed_json() {
        let json = r#"{
            "type": "tool_call",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "tool_name": "bash",
            "call_id": "call_456",
            "state": {
                "status": "completed",
                "input": {"command": "ls"},
                "output": "file1\nfile2\n",
                "time": {"start": 1700000000000, "end": 1700000001000}
            }
        }"#;
        let part: Part = serde_json::from_str(json).expect("deserialize tool call completed");
        match &part {
            Part::ToolCall(tc) => {
                assert_eq!(tc.tool_name, "bash");
                assert!(matches!(tc.state, ToolState::Completed(_)));
            }
            _ => panic!("expected ToolCall part"),
        }
    }

    #[test]
    fn test_deserialize_known_tool_call_error_json() {
        let json = r#"{
            "type": "tool_call",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "tool_name": "edit",
            "call_id": "call_789",
            "state": {
                "status": "error",
                "input": {"path": "/tmp/bar"},
                "error": "file not found",
                "time": {"start": 1700000000000, "end": 1700000000500}
            }
        }"#;
        let part: Part = serde_json::from_str(json).expect("deserialize tool call error");
        match &part {
            Part::ToolCall(tc) => {
                assert_eq!(tc.tool_name, "edit");
                assert!(matches!(tc.state, ToolState::Error(_)));
            }
            _ => panic!("expected ToolCall part"),
        }
    }

    #[test]
    fn test_deserialize_tool_result_json() {
        let json = r#"{
            "type": "tool_result",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "call_id": "call_123"
        }"#;
        let part: Part = serde_json::from_str(json).expect("deserialize tool result");
        assert!(matches!(part, Part::ToolResult(_)));
    }

    // -----------------------------------------------------------------------
    // Backward compatibility: old on-disk data has PascalCase Finish/ErrorKind
    // -----------------------------------------------------------------------

    #[test]
    fn test_deserialize_old_pascal_case_finish() {
        for (name, json) in [
            ("Stop", r#""Stop""#),
            ("Length", r#""Length""#),
            ("ToolUse", r#""ToolUse""#),
            ("Error", r#""Error""#),
        ] {
            let f: Finish = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("deserializing old Finish '{name}': {e}"));
            println!("old Finish '{name}' -> {f:?}");
        }
    }

    #[test]
    fn test_deserialize_old_pascal_case_error_kind() {
        for (name, json) in [
            ("ProviderAuth", r#""ProviderAuth""#),
            ("ToolExec", r#""ToolExec""#),
            ("Network", r#""Network""#),
        ] {
            let k: ErrorKind = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("deserializing old ErrorKind '{name}': {e}"));
            println!("old ErrorKind '{name}' -> {k:?}");
        }
    }

    #[test]
    fn test_deserialize_full_old_format_message() {
        // Exact on-disk format from a real session with PascalCase finish.
        let json = r#"{"id":"01KW8ADP3GR61Q5BVYPH4ENW8F","session_id":"01KW8AD5B7X12BN8GNHN19K1DC","role":"assistant","parts":[{"type":"reasoning","id":"01KW8ADP3G54VTAFNDZT78MR3B","message_id":"01KW8ADP3GS7GQ0B5P50W2HT53","session_id":"01KW8ADP3GMWZJ9FK413S764VX","text":"test","signature":""},{"type":"text","id":"01KW8ADREDRW0D5B8DMBRWQ462","message_id":"01KW8ADRED2DNTQ551R18KRQZ2","session_id":"01KW8ADRED41BVWZRJ8HP6NDEK","text":"hey!"}],"time":{"created":1782690797680,"completed":1782690800125},"assistant":{"provider_id":"","model_id":"","cost":0.0,"tokens":{"input":10134,"output":178,"reasoning":0,"cache_read":0,"cache_write":0},"finish":"Stop"}}"#;
        let msg: Message = serde_json::from_str(json)
            .expect("should deserialize old-format message with PascalCase finish");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.parts.len(), 2);
        assert_eq!(msg.assistant.unwrap().finish, Some(Finish::Stop));
    }
}
