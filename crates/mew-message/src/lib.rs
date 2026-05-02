use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub type MessageId = Ulid;
pub type SessionId = Ulid;
pub type PartId = Ulid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    pub role: Role,
    pub parts: Vec<Part>,
    pub time: Time,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant: Option<AssistantMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Time {
    pub created: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u32,
    pub output: u32,
    pub reasoning: u32,
    pub cache_read: u32,
    pub cache_write: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Finish {
    Stop,
    Length,
    ToolUse,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    ProviderAuth,
    ProviderRateLimit,
    ProviderOverload,
    ProviderApi,
    ContextOverflow,
    Aborted,
    ToolExec,
    ToolTimeout,
    McpTransport,
    AcpProtocol,
    Network,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartBase {
    pub id: Ulid,
    pub message_id: MessageId,
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub text: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub synthetic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePart {
    #[serde(flatten)]
    pub base: PartBase,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub tool_name: String,
    pub call_id: String,
    pub state: ToolState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPart {
    #[serde(flatten)]
    pub base: PartBase,
    pub auto: bool,
    #[serde(default)]
    pub overflow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_start_id: Option<MessageId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    pub fn output(&self) -> Option<&str> {
        match self {
            ToolState::Running(s) => Some(&s.output),
            ToolState::Completed(s) => Some(&s.output),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatePending {
    pub input: serde_json::Value,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStateRunning {
    pub input: serde_json::Value,
    #[serde(default)]
    pub output: String,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStateCompleted {
    pub input: serde_json::Value,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStateError {
    pub input: serde_json::Value,
    pub error: String,
    pub time: ToolTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTime {
    pub start: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}
