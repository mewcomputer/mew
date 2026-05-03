use async_trait::async_trait;
use mew_message::Message;
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ChatParams {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool_name: String,
    pub call_id: String,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub output: String,
    pub error: String,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
    Prompt,
}

#[async_trait]
pub trait Dispatcher: Send + Sync {
    /// Observe-only hook. Errors are logged, never propagated.
    /// The concrete type is typically `mew_agent::AgentEvent`.
    /// Using `&dyn Any` avoids a circular dependency between `mew-hooks` and `mew-agent`.
    async fn on_event(&self, ev: &dyn Any);

    /// Mutating hooks. Each returns the (possibly modified) value.
    /// Errors fall back to the input unchanged and are logged.
    async fn on_chat_message(&self, msg: Message) -> Message;
    async fn on_chat_params(&self, p: ChatParams) -> ChatParams;
    async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap;
    async fn on_tool_execute_before(&self, call: &ToolCall, input: Value) -> Value;
    async fn on_tool_execute_after(&self, call: &ToolCall, output: ToolOutput) -> ToolOutput;
    async fn on_permission_ask(
        &self,
        call: &ToolCall,
        current: PermissionDecision,
    ) -> PermissionDecision;
    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String>;
}

pub struct NopDispatcher;

#[async_trait]
impl Dispatcher for NopDispatcher {
    async fn on_event(&self, _ev: &dyn Any) {}
    async fn on_chat_message(&self, msg: Message) -> Message {
        msg
    }
    async fn on_chat_params(&self, p: ChatParams) -> ChatParams {
        p
    }
    async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap {
        h
    }
    async fn on_tool_execute_before(&self, _call: &ToolCall, input: Value) -> Value {
        input
    }
    async fn on_tool_execute_after(&self, _call: &ToolCall, output: ToolOutput) -> ToolOutput {
        output
    }
    async fn on_permission_ask(
        &self,
        _call: &ToolCall,
        current: PermissionDecision,
    ) -> PermissionDecision {
        current
    }
    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String> {
        env
    }
}
