use async_trait::async_trait;
use mew_message::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

// Type aliases for PluginHost callbacks to keep clippy's type_complexity happy.
type NotifyFn = Arc<dyn Fn(String) + Send + Sync>;
type ConfigReadFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
type LogFn = Arc<dyn Fn(String) + Send + Sync>;
type StorageReadFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
type StorageWriteFn = Arc<dyn Fn(&str, &str) + Send + Sync>;
type StorageDeleteFn = Arc<dyn Fn(&str) + Send + Sync>;
type SetUiFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatParams {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub call_id: String,
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A dynamically registered tool from a plugin.
pub struct ToolRegistration {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: Value,
    /// Called when the model invokes this tool. Returns the tool's output text.
    /// The boxed closure is Send + Sync so it can be shared across threads.
    pub execute: Box<dyn Fn(Value) -> String + Send + Sync>,
}

/// A dynamically registered slash command from a plugin (m10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommandDef {
    pub name: String,
    pub description: String,
    pub handler_id: String,
}

/// Host handle plugins receive during init. Provides session-context access.
#[derive(Clone)]
pub struct PluginHost {
    /// Pushes a non-modal alert to the TUI (supports CC Notification hook in m11).
    pub notify: NotifyFn,
    /// Read-only access to a safe subset of config values (known keys only).
    pub config_read: ConfigReadFn,
    /// Restricted log channel (prefixed with plugin name, rate-limited).
    pub log: LogFn,
    /// Read a per-plugin persistent value. Key is namespaced to the plugin.
    pub storage_read: StorageReadFn,
    /// Write a per-plugin persistent value (stored to disk).
    pub storage_write: StorageWriteFn,
    /// Delete a per-plugin persistent value.
    pub storage_delete: StorageDeleteFn,
    /// Push named text content to the TUI for rendering. The TUI exposes plugin
    /// data via a designated area beside the input prompt.
    /// Keys are plugin-namespaced: the format is `"plugin-name/key"`.
    /// The TUI reads `app.plugin_ui` during render.
    pub set_ui: SetUiFn,
}

#[async_trait]
pub trait Dispatcher: Send + Sync {
    /// Lifecycle: called at session startup with the host handle.
    async fn init(&self, host: &PluginHost);

    /// Lifecycle: called at session shutdown.
    async fn shutdown(&self);

    /// Dynamic tool registration: called at startup after init and on plugin
    /// reload. Returns additional tools to merge into the agent's tool registry.
    async fn on_register_tools(&self) -> Vec<ToolRegistration>;

    /// Dynamic slash command registration (m10): returns slash commands to
    /// merge into the TUI's command registry.
    async fn on_register_slash_commands(&self) -> Vec<SlashCommandDef>;

    /// Execute a previously registered slash command (m10). Called by the TUI
    /// when the user invokes a plugin-registered slash command. Returns the
    /// result text to display, or None to fall through to the model.
    async fn execute_slash_command(&self, command: &str, args: &str) -> Option<String>;

    /// Observe-only hook. Errors are logged, never propagated.
    async fn on_event(&self, ev: &dyn Any);

    /// Turn-grain observer: fires after each assistant turn completes
    /// (tool results pushed, about to loop back or terminate). Errors logged,
    /// never propagated. Fire-and-forget — must not block the turn loop.
    async fn on_turn_end(&self, messages: &[Message]);

    /// Mutation hooks. Each returns the (possibly modified) value.
    /// Errors fall back to the input unchanged and are logged.
    async fn on_chat_message(&self, msg: Message) -> Message;
    async fn on_chat_params(&self, p: ChatParams) -> ChatParams;
    async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap;
    /// Called when the system prompt is assembled, before it's sent to the
    /// model. Plugins may prepend, append, or replace sections. Called every
    /// turn (system prompt is rebuilt from scratch each turn).
    async fn on_system_prompt(&self, prompt: String) -> String;
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
    async fn init(&self, _: &PluginHost) {}
    async fn shutdown(&self) {}
    async fn on_register_tools(&self) -> Vec<ToolRegistration> {
        vec![]
    }
    async fn on_register_slash_commands(&self) -> Vec<SlashCommandDef> {
        vec![]
    }
    async fn execute_slash_command(&self, _command: &str, _args: &str) -> Option<String> {
        None
    }
    async fn on_event(&self, _ev: &dyn Any) {}
    async fn on_turn_end(&self, _messages: &[Message]) {}
    async fn on_chat_message(&self, msg: Message) -> Message {
        msg
    }
    async fn on_chat_params(&self, p: ChatParams) -> ChatParams {
        p
    }
    async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap {
        h
    }
    async fn on_system_prompt(&self, prompt: String) -> String {
        prompt
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_nop_init_shutdown() {
        let host = PluginHost {
            notify: Arc::new(|_| {}),
            config_read: Arc::new(|_| None),
            log: Arc::new(|_| {}),
            storage_read: Arc::new(|_| None),
            storage_write: Arc::new(|_, _| {}),
            storage_delete: Arc::new(|_| {}),
            set_ui: Arc::new(|_, _| {}),
        };
        let nop = NopDispatcher;
        nop.init(&host).await;
        nop.shutdown().await;
    }

    #[tokio::test]
    async fn test_nop_on_register_slash_commands_returns_empty() {
        assert!(NopDispatcher.on_register_slash_commands().await.is_empty());
    }

    #[tokio::test]
    async fn test_nop_on_turn_end_does_not_panic() {
        NopDispatcher.on_turn_end(&[]).await;
    }

    #[tokio::test]
    async fn test_nop_on_system_prompt_passthrough() {
        let prompt = "system instructions".to_string();
        let result = NopDispatcher.on_system_prompt(prompt.clone()).await;
        assert_eq!(result, prompt);
    }

    #[tokio::test]
    async fn test_nop_execute_slash_command_returns_none() {
        assert!(NopDispatcher
            .execute_slash_command("/buddy", "pet")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_nop_on_event_does_not_panic() {
        NopDispatcher.on_event(&"event").await;
    }

    #[tokio::test]
    async fn test_slash_command_def_serialization() {
        let cmd = SlashCommandDef {
            name: "/buddy".into(),
            description: "pet companion".into(),
            handler_id: "buddy-handler".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: SlashCommandDef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "/buddy");
        assert_eq!(parsed.description, "pet companion");
        assert_eq!(parsed.handler_id, "buddy-handler");
    }
}
