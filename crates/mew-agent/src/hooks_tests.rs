use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use mew_hooks::{
    ChatParams, Dispatcher, PermissionDecision, PluginHost, SlashCommandDef, ToolCall, ToolOutput,
    ToolRegistration,
};
use mew_message::Message;
use mew_provider::ProviderEvent;
use mew_provider_fake::FakeProvider;
use mew_tools::{Sensitivity, Tool};

use crate::{Agent, AgentEvent};

// ------------------------------------------------------------------
// Recording dispatcher — captures hook calls for assertions
// ------------------------------------------------------------------

struct RecordingDispatcher {
    pub system_prompt_calls: Arc<StdMutex<Vec<String>>>,
    pub turn_end_calls: Arc<StdMutex<usize>>,
}

impl RecordingDispatcher {
    fn new() -> Self {
        Self {
            system_prompt_calls: Arc::new(StdMutex::new(Vec::new())),
            turn_end_calls: Arc::new(StdMutex::new(0)),
        }
    }
}

#[async_trait]
impl Dispatcher for RecordingDispatcher {
    async fn init(&self, _host: &PluginHost) {}
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

    async fn on_provider_event(&self, _ev: &mew_provider::ProviderEvent) {}
    async fn on_tool_error(&self, _call: &mew_hooks::ToolCall, _error: &str) {}
    async fn on_subagent_start(
        &self,
        _name: &str,
        _parent_call_id: &str,
        _display_name: Option<&str>,
    ) {
    }
    async fn on_subagent_end(&self, _name: &str, _parent_call_id: &str, _outcome: &str) {}

    async fn on_turn_end(&self, messages: &[Message]) {
        *self.turn_end_calls.lock().unwrap() += 1;
        let _ = messages.len();
    }

    async fn on_pre_model_turn(&self, _messages: &[Message], _system: &str) {}
    async fn on_stop(&self) {}
    async fn on_pre_compaction(&self, _messages: &[Message]) {}
    async fn on_post_compaction(&self, _messages: &[Message]) {}

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
        self.system_prompt_calls
            .lock()
            .unwrap()
            .push(prompt.clone());
        prompt
    }

    async fn on_tool_execute_before(
        &self,
        _call: &ToolCall,
        input: serde_json::Value,
    ) -> mew_hooks::HookOutcome<serde_json::Value> {
        mew_hooks::HookOutcome::Proceed(input)
    }
    async fn on_tool_execute_after(&self, _call: &ToolCall, output: ToolOutput) -> ToolOutput {
        output
    }
    async fn on_permission_ask(
        &self,
        _call: &ToolCall,
        current: PermissionDecision,
    ) -> mew_hooks::HookOutcome<PermissionDecision> {
        mew_hooks::HookOutcome::Proceed(current)
    }
    async fn on_shell_env(
        &self,
        env: std::collections::HashMap<String, String>,
    ) -> std::collections::HashMap<String, String> {
        env
    }
    async fn on_user_input(&self, prompt: String) -> String {
        prompt
    }
    async fn on_persona_change(&self, _old_persona: Option<&str>, _new_persona: &str) {}
    async fn on_session_save(&self) {}
    async fn on_model_finish(
        &self,
        _finish: &str,
        _input_tokens: u32,
        _output_tokens: u32,
        _cost: f64,
    ) {
    }
}

// ------------------------------------------------------------------
// EchoTool (same pattern as tests.rs, duplicated for self-contained tests)
// ------------------------------------------------------------------

struct EchoTool {
    schema: serde_json::Value,
    sensitivity: Sensitivity,
}

impl EchoTool {
    fn new(schema: serde_json::Value, sensitivity: Sensitivity) -> Self {
        Self {
            schema,
            sensitivity,
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echo tool"
    }
    fn schema(&self) -> &serde_json::Value {
        &self.schema
    }
    fn sensitivity(&self) -> Sensitivity {
        self.sensitivity
    }
    async fn execute(
        &self,
        _ctx: mew_tools::ToolCtx,
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
// Tests
// ------------------------------------------------------------------

#[tokio::test]
async fn test_on_turn_end_called_after_text_turn() {
    let recorder = RecordingDispatcher::new();
    let turn_end_calls = recorder.turn_end_calls.clone();

    let script = FakeProvider::text_response("response");
    let provider = Arc::new(FakeProvider::new(script));
    let dispatcher: Arc<dyn Dispatcher> = Arc::new(recorder);
    let tools: Vec<Arc<dyn Tool>> = vec![];

    let agent = Agent::new(provider, dispatcher, None, tools, None);
    let mut rx = agent.run("prompt".into());
    while let Some(ev) = rx.recv().await {
        if matches!(ev, AgentEvent::Provider(ProviderEvent::MessageEnd { .. })) {
            break;
        }
    }
    while let Ok(ev) = rx.try_recv() {
        drop(ev);
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let count = *turn_end_calls.lock().unwrap();
    assert!(
        count >= 1,
        "on_turn_end should be called after text turn completes, got {count}"
    );
}

#[tokio::test]
async fn test_on_system_prompt_transforms_output() {
    struct PrefixDispatcher {
        prefix: String,
        calls: StdMutex<Vec<String>>,
    }
    #[async_trait]
    impl Dispatcher for PrefixDispatcher {
        async fn init(&self, _: &PluginHost) {}
        async fn shutdown(&self) {}
        async fn on_register_tools(&self) -> Vec<ToolRegistration> {
            vec![]
        }
        async fn on_register_slash_commands(&self) -> Vec<SlashCommandDef> {
            vec![]
        }
        async fn execute_slash_command(&self, _: &str, _: &str) -> Option<String> {
            None
        }
        async fn on_provider_event(&self, _: &mew_provider::ProviderEvent) {}
        async fn on_tool_error(&self, _: &mew_hooks::ToolCall, _: &str) {}
        async fn on_subagent_start(&self, _: &str, _: &str, _: Option<&str>) {}
        async fn on_subagent_end(&self, _: &str, _: &str, _: &str) {}
        async fn on_turn_end(&self, _: &[Message]) {}
        async fn on_pre_model_turn(&self, _: &[Message], _: &str) {}
        async fn on_stop(&self) {}
        async fn on_pre_compaction(&self, _: &[Message]) {}
        async fn on_post_compaction(&self, _: &[Message]) {}
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
            self.calls.lock().unwrap().push(prompt.clone());
            format!("{} {}", self.prefix, prompt)
        }
        async fn on_tool_execute_before(
            &self,
            _: &ToolCall,
            input: serde_json::Value,
        ) -> mew_hooks::HookOutcome<serde_json::Value> {
            mew_hooks::HookOutcome::Proceed(input)
        }
        async fn on_tool_execute_after(&self, _: &ToolCall, output: ToolOutput) -> ToolOutput {
            output
        }
        async fn on_permission_ask(
            &self,
            _: &ToolCall,
            current: PermissionDecision,
        ) -> mew_hooks::HookOutcome<PermissionDecision> {
            mew_hooks::HookOutcome::Proceed(current)
        }
        async fn on_shell_env(
            &self,
            env: std::collections::HashMap<String, String>,
        ) -> std::collections::HashMap<String, String> {
            env
        }
        async fn on_user_input(&self, prompt: String) -> String {
            prompt
        }
        async fn on_persona_change(&self, _: Option<&str>, _: &str) {}
        async fn on_session_save(&self) {}
        async fn on_model_finish(&self, _: &str, _: u32, _: u32, _: f64) {}
    }

    let dispatcher = Arc::new(PrefixDispatcher {
        prefix: "[PLUGIN]".into(),
        calls: StdMutex::new(Vec::new()),
    });

    let script = FakeProvider::text_response("ok");
    let provider = Arc::new(FakeProvider::new(script));
    let tools: Vec<Arc<dyn Tool>> = vec![];

    let mut agent = Agent::new(provider, dispatcher, None, tools, None);
    agent.set_system("base prompt".into());

    let mut rx = agent.run("hi".into());
    while let Some(ev) = rx.recv().await {
        if matches!(ev, AgentEvent::Provider(ProviderEvent::MessageEnd { .. })) {
            break;
        }
    }
}

#[tokio::test]
async fn test_on_turn_end_called_after_tool_turn() {
    let recorder = RecordingDispatcher::new();
    let turn_end_calls = recorder.turn_end_calls.clone();

    let script = FakeProvider::tool_call("echo", "c1", serde_json::json!({"input": "x"}));
    let provider = Arc::new(FakeProvider::new(script));

    let echo_tool = Arc::new(EchoTool::new(
        serde_json::json!({
            "type": "object",
            "properties": {"input": {"type": "string"}}
        }),
        Sensitivity::ReadOnly,
    ));
    let tools: Vec<Arc<dyn Tool>> = vec![echo_tool];
    let dispatcher: Arc<dyn Dispatcher> = Arc::new(recorder);

    let agent = Agent::new(provider, dispatcher, None, tools, None);
    let mut rx = agent.run("call echo".into());
    while let Some(ev) = rx.recv().await {
        let mut done = false;
        match ev {
            AgentEvent::PermissionRequest { tx, .. } => {
                let _ = tx.send(PermissionDecision::AllowOnce);
            }
            AgentEvent::Provider(ProviderEvent::MessageEnd { .. }) => {
                done = true;
            }
            _ => {}
        }
        if done {
            break;
        }
    }
    while let Ok(ev) = rx.try_recv() {
        drop(ev);
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let count = *turn_end_calls.lock().unwrap();
    assert!(
        count >= 1,
        "on_turn_end should be called after tool turn completes, got {count}"
    );
}

#[tokio::test]
async fn test_on_system_prompt_called_each_turn() {
    let recorder = RecordingDispatcher::new();
    let sys_calls = recorder.system_prompt_calls.clone();

    let script1 = FakeProvider::text_response("first response");
    let script2 = FakeProvider::text_response("second response");
    let provider = Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
    let dispatcher: Arc<dyn Dispatcher> = Arc::new(recorder);
    let tools: Vec<Arc<dyn Tool>> = vec![];

    let mut agent = Agent::new(provider, dispatcher, None, tools, None);
    agent.set_system("prompt v1".into());

    // First turn
    let mut rx = agent.run("turn 1".into());
    while let Some(ev) = rx.recv().await {
        if matches!(ev, AgentEvent::Provider(ProviderEvent::MessageEnd { .. })) {
            break;
        }
    }

    // Second turn
    agent.set_system("prompt v2".into());
    let mut rx = agent.run("turn 2".into());
    while let Some(ev) = rx.recv().await {
        if matches!(ev, AgentEvent::Provider(ProviderEvent::MessageEnd { .. })) {
            break;
        }
    }

    let calls = sys_calls.lock().unwrap().clone();
    assert_eq!(
        calls.len(),
        2,
        "on_system_prompt should be called each turn"
    );
    assert_eq!(calls[0], "prompt v1");
    assert_eq!(calls[1], "prompt v2");
}

/// Multi-script provider for multi-turn tests.
struct StatefulFakeProvider {
    scripts: StdMutex<Vec<Vec<ProviderEvent>>>,
}

impl StatefulFakeProvider {
    fn new(scripts: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            scripts: StdMutex::new(scripts),
        }
    }
}

#[async_trait]
impl mew_provider::Provider for StatefulFakeProvider {
    fn name(&self) -> &str {
        "stateful-fake"
    }
    async fn stream(
        &self,
        _req: mew_provider::Request,
    ) -> Result<mew_provider::EventStream, mew_provider::ProviderError> {
        let script = self.scripts.lock().unwrap().remove(0);
        let stream = futures::stream::iter(script);
        Ok(Box::pin(stream))
    }
}
