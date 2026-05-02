use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use futures::StreamExt;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;
use ulid::Ulid;

use mew_hooks::{
    ChatParams, Dispatcher, PermissionDecision, ToolCall as HookToolCall, ToolOutput,
};
use mew_message::{
    AssistantMeta, ErrorKind, Message, MessageError, Part, PartBase, PartId, Role, SessionId,
    TextPart, Time, Tokens, ToolCallPart, ToolResultPart, ToolState, ToolStateCompleted,
    ToolStateError, ToolStateRunning, ToolTime,
};
use mew_provider::{Provider, ProviderEvent, Request, ToolDef};
use mew_session::Writer as SessionWriterInner;
use mew_tools::{Sensitivity, Tool, ToolCtx, ToolProgress};

/// Alias for the interior-mutability wrapper required by the async agent loop.
pub type SessionWriter = Arc<Mutex<SessionWriterInner>>;

/// Events emitted by the agent core to the TUI.
pub enum AgentEvent {
    /// A raw provider event.
    Provider(ProviderEvent),
    /// Request user approval for a tool call.
    PermissionRequest {
        call: HookToolCall,
        tx: oneshot::Sender<PermissionDecision>,
    },
    /// A tool execution has started.
    ToolStart { call_id: String },
    /// A tool execution has finished.
    ToolEnd { call_id: String, success: bool },
    /// A part's content or state has changed (e.g. tool-call state transition).
    PartUpdated { part_id: PartId, part: Part },
    /// A terminal error occurred.
    Error(String),
}

impl std::fmt::Debug for AgentEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentEvent::Provider(ev) => f.debug_tuple("Provider").field(ev).finish(),
            AgentEvent::PermissionRequest { call, .. } => f
                .debug_struct("PermissionRequest")
                .field("call", call)
                .finish(),
            AgentEvent::ToolStart { call_id } => {
                f.debug_struct("ToolStart").field("call_id", call_id).finish()
            }
            AgentEvent::ToolEnd { call_id, success } => f
                .debug_struct("ToolEnd")
                .field("call_id", call_id)
                .field("success", success)
                .finish(),
            AgentEvent::PartUpdated { part_id, part } => f
                .debug_struct("PartUpdated")
                .field("part_id", part_id)
                .field("part", part)
                .finish(),
            AgentEvent::Error(msg) => f.debug_tuple("Error").field(msg).finish(),
        }
    }
}

/// The core conversation loop.
#[derive(Clone)]
pub struct Agent {
    pub provider: Arc<dyn Provider>,
    pub dispatcher: Arc<dyn Dispatcher>,
    pub session: Option<SessionWriter>,
    pub tools: HashMap<String, Arc<dyn Tool>>,
    pub messages: Arc<Mutex<Vec<Message>>>,
    pub session_id: SessionId,
    pub system: String,
    pub cancel_token: CancellationToken,
    pub permission_engine: Option<Arc<mew_config::permissions::PermissionEngine>>,
}

impl Agent {
    /// Creates a new agent.
    pub fn new(
        provider: Arc<dyn Provider>,
        dispatcher: Arc<dyn Dispatcher>,
        session: Option<SessionWriterInner>,
        tools: Vec<Arc<dyn Tool>>,
        session_id: Option<SessionId>,
    ) -> Self {
        let mut tools_map = HashMap::new();
        for tool in tools {
            tools_map.insert(tool.name().to_string(), tool);
        }

        Self {
            provider,
            dispatcher,
            session: session.map(|w| Arc::new(Mutex::new(w))),
            tools: tools_map,
            messages: Arc::new(Mutex::new(Vec::new())),
            session_id: session_id.unwrap_or_else(Ulid::new),
            system: String::new(),
            cancel_token: CancellationToken::new(),
            permission_engine: None,
        }
    }

    /// Attach a permission engine for rule-based permission checks.
    pub fn set_permission_engine(&mut self, engine: Arc<mew_config::permissions::PermissionEngine>) {
        self.permission_engine = Some(engine);
    }

    /// Sets the system prompt prepended to every provider request.
    pub fn set_system(&mut self, system: String) {
        self.system = system;
    }

    /// Starts a single turn and returns a channel of agent events.
    pub fn run(&self, prompt: String) -> mpsc::Receiver<AgentEvent> {
        let (tx, rx) = mpsc::channel(256);
        let agent = self.clone();

        tokio::spawn(async move {
            if let Err(e) = agent.run_loop(prompt, tx).await {
                tracing::error!("agent loop ended with error: {}", e);
            }
        });

        rx
    }

    // ------------------------------------------------------------------
    // Internal async loop
    // ------------------------------------------------------------------

    async fn run_loop(
        &self,
        prompt: String,
        ev_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let msg_id = Ulid::new();
        let user_msg = Message {
            id: msg_id,
            session_id: self.session_id,
            role: Role::User,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: msg_id,
                    session_id: self.session_id,
                },
                text: prompt,
                synthetic: false,
            })],
            time: Time {
                created: Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        };

        self.append_message(user_msg).await;
        self.turn_loop(ev_tx).await
    }

    async fn turn_loop(
        &self,
        ev_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        loop {
            let tool_defs: Vec<ToolDef> = self
                .tools
                .values()
                .map(|t| ToolDef {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    schema: t.schema().clone(),
                })
                .collect();

            let messages = self.messages.lock().await.clone();

            let req = Request {
                model: String::new(),
                messages,
                tools: tool_defs,
                system: self.system.clone(),
            };

            let _ = self
                .dispatcher
                .on_chat_params(ChatParams {
                    temperature: None,
                    top_p: None,
                    max_tokens: None,
                })
                .await;
            let _ = self
                .dispatcher
                .on_chat_headers(http::HeaderMap::new())
                .await;

            let mut stream = match self.provider.stream(req).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = ev_tx
                        .send(AgentEvent::Error(format!("provider stream: {}", e)))
                        .await;
                    return Err(Box::new(e));
                }
            };

            let mut assistant_msg: Option<Message> = None;

            // Stream provider events until the stream ends or we are cancelled.
            loop {
                tokio::select! {
                    biased;
                    _ = self.cancel_token.cancelled() => {
                        if let Some(ref mut msg) = assistant_msg {
                            let now = Utc::now().timestamp_millis();
                            msg.time.completed = Some(now);
                            msg.assistant = Some(AssistantMeta {
                                provider_id: String::new(),
                                model_id: String::new(),
                                cost: 0.0,
                                tokens: Tokens::default(),
                                finish: None,
                                error: Some(MessageError {
                                    kind: ErrorKind::Aborted,
                                    message: "aborted".into(),
                                }),
                            });
                            self.append_message(msg.clone()).await;
                        }
                        let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                        return Ok(());
                    }
                    ev = stream.next() => {
                        match ev {
                            None => break,
                            Some(ev) => {
                                self.handle_provider_event(
                                    &ev,
                                    &mut assistant_msg,
                                    &ev_tx,
                                ).await;
                                if matches!(ev, ProviderEvent::Error(_)) {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }

            // Stream ended naturally.
            if assistant_msg.is_none() {
                let _ = ev_tx
                    .send(AgentEvent::Error(
                        "no assistant message received".into(),
                    ))
                    .await;
                return Ok(());
            }

            if self.cancel_token.is_cancelled() {
                if let Some(ref mut msg) = assistant_msg {
                    let now = Utc::now().timestamp_millis();
                    msg.time.completed = Some(now);
                    msg.assistant = Some(AssistantMeta {
                        provider_id: String::new(),
                        model_id: String::new(),
                        cost: 0.0,
                        tokens: Tokens::default(),
                        finish: None,
                        error: Some(MessageError {
                            kind: ErrorKind::Aborted,
                            message: "aborted".into(),
                        }),
                    });
                    self.append_message(msg.clone()).await;
                }
                let _ = ev_tx.send(AgentEvent::Error("aborted".into())).await;
                return Ok(());
            }

            let pending = self.pending_tool_calls(assistant_msg.as_ref().unwrap());
            if pending.is_empty() {
                return Ok(());
            }

            // Execute pending tool calls.
            let mut result_parts: Vec<Part> = Vec::with_capacity(pending.len());
            for tc in pending {
                let call_id = tc.call_id.clone();
                let part_id = tc.base.id;

                // Mark as running.
                let running_state = ToolState::Running(ToolStateRunning {
                    input: tc.input().clone(),
                    output: String::new(),
                    time: ToolTime {
                        start: Utc::now().timestamp_millis(),
                        end: None,
                    },
                });

                if let Some(ref mut msg) = assistant_msg {
                    self.update_tool_call(msg, part_id, running_state.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::PartUpdated {
                        part_id,
                        part: Part::ToolCall(ToolCallPart {
                            base: tc.base.clone(),
                            tool_name: tc.tool_name.clone(),
                            call_id: tc.call_id.clone(),
                            state: running_state,
                            raw_input: tc.raw_input.clone(),
                        }),
                    })
                    .await;
                let _ = ev_tx
                    .send(AgentEvent::ToolStart {
                        call_id: call_id.clone(),
                    })
                    .await;

                let hook_call = HookToolCall {
                    tool_name: tc.tool_name.clone(),
                    call_id: tc.call_id.clone(),
                    input: tc.input().clone(),
                };

                // Permission check.
                let sensitivity = self
                    .tools
                    .get(&tc.tool_name)
                    .map(|t| t.sensitivity())
                    .unwrap_or(Sensitivity::Dangerous);
                let default_decision = if let Some(ref engine) = self.permission_engine {
                    engine
                        .check(&tc.tool_name, &hook_call.input, sensitivity)
                        .await
                } else {
                    match sensitivity {
                        Sensitivity::ReadOnly => PermissionDecision::AllowOnce,
                        _ => PermissionDecision::Prompt,
                    }
                };
                let decision = self
                    .dispatcher
                    .on_permission_ask(&hook_call, default_decision)
                    .await;

                let decision = if decision == PermissionDecision::Prompt {
                    let (perm_tx, perm_rx) = oneshot::channel();
                    let _ = ev_tx
                        .send(AgentEvent::PermissionRequest {
                            call: hook_call.clone(),
                            tx: perm_tx,
                        })
                        .await;
                    match perm_rx.await {
                        Ok(d) => d,
                        Err(_) => PermissionDecision::Deny,
                    }
                } else {
                    decision
                };

                if decision == PermissionDecision::AllowSession {
                    if let Some(ref engine) = self.permission_engine {
                        engine.add_session_allow(&tc.tool_name).await;
                    }
                }

                if decision == PermissionDecision::Deny {
                    let error_state = ToolState::Error(ToolStateError {
                        input: hook_call.input.clone(),
                        error: "permission denied".into(),
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    });
                    if let Some(ref mut msg) = assistant_msg {
                        self.update_tool_call(msg, part_id, error_state.clone());
                    }
                    let _ = ev_tx
                        .send(AgentEvent::PartUpdated {
                            part_id,
                            part: Part::ToolCall(ToolCallPart {
                                base: tc.base.clone(),
                                tool_name: tc.tool_name.clone(),
                                call_id: tc.call_id.clone(),
                                state: error_state,
                                raw_input: tc.raw_input.clone(),
                            }),
                        })
                        .await;
                    let _ = ev_tx
                        .send(AgentEvent::ToolEnd {
                            call_id: call_id.clone(),
                            success: false,
                        })
                        .await;
                    result_parts.push(Part::ToolResult(ToolResultPart {
                        base: PartBase {
                            id: Ulid::new(),
                            message_id: assistant_msg.as_ref().unwrap().id,
                            session_id: self.session_id,
                        },
                        call_id: tc.call_id.clone(),
                    }));
                    continue;
                }

                let tool = match self.tools.get(&tc.tool_name) {
                    Some(t) => t,
                    None => {
                        let error_state = ToolState::Error(ToolStateError {
                            input: hook_call.input.clone(),
                            error: format!("unknown tool {:?}", tc.tool_name),
                            time: ToolTime {
                                start: Utc::now().timestamp_millis(),
                                end: Some(Utc::now().timestamp_millis()),
                            },
                        });
                        if let Some(ref mut msg) = assistant_msg {
                            self.update_tool_call(msg, part_id, error_state.clone());
                        }
                        let _ = ev_tx
                            .send(AgentEvent::PartUpdated {
                                part_id,
                                part: Part::ToolCall(ToolCallPart {
                                    base: tc.base.clone(),
                                    tool_name: tc.tool_name.clone(),
                                    call_id: tc.call_id.clone(),
                                    state: error_state,
                                    raw_input: tc.raw_input.clone(),
                                }),
                            })
                            .await;
                        let _ = ev_tx
                            .send(AgentEvent::ToolEnd {
                                call_id: call_id.clone(),
                                success: false,
                            })
                            .await;
                        result_parts.push(Part::ToolResult(ToolResultPart {
                            base: PartBase {
                                id: Ulid::new(),
                                message_id: assistant_msg.as_ref().unwrap().id,
                                session_id: self.session_id,
                            },
                            call_id: tc.call_id.clone(),
                        }));
                        continue;
                    }
                };

                let input = if hook_call.input.is_null() && !tc.raw_input.is_empty() {
                    serde_json::from_str(&tc.raw_input).unwrap_or_else(|_| hook_call.input.clone())
                } else {
                    hook_call.input.clone()
                };
                let input = self
                    .dispatcher
                    .on_tool_execute_before(&hook_call, input)
                    .await;

                let (progress_tx, mut progress_rx) = mpsc::channel::<ToolProgress>(16);
                // Drain progress so the channel doesn't back-pressure the tool.
                tokio::spawn(async move {
                    while progress_rx.recv().await.is_some() {}
                });

                let ctx = ToolCtx {
                    session_id: self.session_id,
                    call_id: tc.call_id.clone(),
                    cancel: self.cancel_token.child_token(),
                    progress_tx,
                    cwd: std::env::current_dir()
                        .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                };

                tracing::info!(tool = %tc.tool_name, call_id = %call_id, input = %input, "executing tool");
                let exec_result = tool.execute(ctx, input.clone()).await;
                let tool_output = match exec_result {
                    Ok(out) => {
                        tracing::info!(tool = %tc.tool_name, call_id = %call_id, output = %out.output, error = %out.error, "tool executed successfully");
                        out
                    }
                    Err(e) => {
                        tracing::warn!(tool = %tc.tool_name, call_id = %call_id, error = %e, "tool execution failed");
                        ToolOutput {
                            output: String::new(),
                            error: e.to_string(),
                        }
                    }
                };

                // Update hook_call with parsed input so hooks and final state are correct.
                let hook_call = HookToolCall {
                    tool_name: hook_call.tool_name,
                    call_id: hook_call.call_id,
                    input: input.clone(),
                };

                let output = self
                    .dispatcher
                    .on_tool_execute_after(&hook_call, tool_output)
                    .await;

                tracing::info!(tool = %tc.tool_name, call_id = %call_id, success = %output.error.is_empty(), "tool finished");
                let (success, final_state) = if !output.error.is_empty() {
                    (
                        false,
                        ToolState::Error(ToolStateError {
                            input: input.clone(),
                            error: output.error.clone(),
                            time: ToolTime {
                                start: Utc::now().timestamp_millis(),
                                end: Some(Utc::now().timestamp_millis()),
                            },
                        }),
                    )
                } else {
                    (
                        true,
                        ToolState::Completed(ToolStateCompleted {
                            input: input.clone(),
                            output: output.output.clone(),
                            metadata: None,
                            time: ToolTime {
                                start: Utc::now().timestamp_millis(),
                                end: Some(Utc::now().timestamp_millis()),
                            },
                        }),
                    )
                };

                if let Some(ref mut msg) = assistant_msg {
                    self.update_tool_call(msg, part_id, final_state.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::PartUpdated {
                        part_id,
                            part: Part::ToolCall(ToolCallPart {
                                base: tc.base.clone(),
                                tool_name: tc.tool_name.clone(),
                                call_id: tc.call_id.clone(),
                                state: final_state,
                                raw_input: tc.raw_input.clone(),
                            }),
                    })
                    .await;
                let _ = ev_tx
                    .send(AgentEvent::ToolEnd {
                        call_id: call_id.clone(),
                        success,
                    })
                    .await;

                result_parts.push(Part::ToolResult(ToolResultPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id: assistant_msg.as_ref().unwrap().id,
                        session_id: self.session_id,
                    },
                    call_id: tc.call_id.clone(),
                }));
            }

            // Sync updated assistant message (with tool state transitions)
            // back into self.messages so the next request has the correct state.
            if let Some(ref msg) = assistant_msg {
                tracing::debug!(msg_id = %msg.id, "syncing assistant message to store");
                let mut messages = self.messages.lock().await;
                for m in messages.iter_mut() {
                    if m.id == msg.id {
                        *m = msg.clone();
                        break;
                    }
                }
            }

            let result_msg = Message {
                id: Ulid::new(),
                session_id: self.session_id,
                role: Role::User,
                parts: result_parts,
                time: Time {
                    created: Utc::now().timestamp_millis(),
                    completed: None,
                },
                assistant: None,
            };
            self.append_message(result_msg).await;
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    async fn append_message(&self, msg: Message) {
        let mut messages = self.messages.lock().await;
        messages.push(msg.clone());
        drop(messages);

        if let Some(session) = &self.session {
            let mut session = session.lock().await;
            if let Err(e) = session.write_message(&msg).await {
                tracing::error!("session write failed: {}", e);
            }
        }
    }

    fn start_assistant_message(&self) -> Message {
        Message {
            id: Ulid::new(),
            session_id: self.session_id,
            role: Role::Assistant,
            parts: Vec::new(),
            time: Time {
                created: Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: Some(AssistantMeta {
                provider_id: String::new(),
                model_id: String::new(),
                cost: 0.0,
                tokens: Tokens::default(),
                finish: None,
                error: None,
            }),
        }
    }

    async fn handle_provider_event(
        &self,
        ev: &ProviderEvent,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
    ) {
        match ev {
            ProviderEvent::PartStart { part } => {
                if assistant_msg.is_none() {
                    *assistant_msg = Some(self.start_assistant_message());
                }
                if let Some(ref mut msg) = assistant_msg {
                    msg.parts.push(part.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::PartStart {
                        part: part.clone(),
                    }))
                    .await;
            }
            ProviderEvent::PartDelta {
                part_id,
                field,
                delta,
            } => {
                if let Some(ref mut msg) = assistant_msg {
                    self.apply_delta(msg, *part_id, field, delta);
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::PartDelta {
                        part_id: *part_id,
                        field,
                        delta: delta.clone(),
                    }))
                    .await;
            }
            ProviderEvent::PartEnd { part_id } => {
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::PartEnd {
                        part_id: *part_id,
                    }))
                    .await;
            }
            ProviderEvent::MessageEnd {
                finish,
                usage,
                cost,
            } => {
                if let Some(ref mut msg) = assistant_msg {
                    let now = Utc::now().timestamp_millis();
                    msg.time.completed = Some(now);
                    if let Some(ref mut meta) = msg.assistant {
                        meta.finish = Some(*finish);
                        meta.tokens = *usage;
                        meta.cost = *cost;
                    }
                    self.append_message(msg.clone()).await;
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::MessageEnd {
                        finish: *finish,
                        usage: *usage,
                        cost: *cost,
                    }))
                    .await;
            }
            ProviderEvent::Error(err) => {
                if let Some(ref mut msg) = assistant_msg {
                    let now = Utc::now().timestamp_millis();
                    msg.time.completed = Some(now);
                    if let Some(ref mut meta) = msg.assistant {
                        meta.error = Some(err.clone());
                    }
                    self.append_message(msg.clone()).await;
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::Error(err.clone())))
                    .await;
                let _ = ev_tx
                    .send(AgentEvent::Error(err.message.clone()))
                    .await;
            }
        }
    }

    /// Applies a text delta to the matching part in a message.
    pub fn apply_delta(&self, msg: &mut Message, part_id: PartId, field: &str, delta: &str) {
        for part in &mut msg.parts {
            if part.id() != part_id {
                continue;
            }
            match part {
                Part::Text(ref mut p) => {
                    if field == "text" || field.is_empty() {
                        p.text.push_str(delta);
                    }
                }
                Part::Reasoning(ref mut p) => {
                    if field == "text" || field.is_empty() {
                        p.text.push_str(delta);
                    }
                }
                Part::ToolCall(ref mut p) => {
                    match field {
                        "arguments" => p.raw_input.push_str(delta),
                        "call_id" => p.call_id.push_str(delta),
                        "tool_name" => p.tool_name.push_str(delta),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn pending_tool_calls(&self, msg: &Message) -> Vec<ToolCallPart> {
        msg.parts
            .iter()
            .filter_map(|p| {
                if let Part::ToolCall(tc) = p {
                    match &tc.state {
                        ToolState::Pending(_) | ToolState::Running(_) => Some(tc.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    fn update_tool_call(&self, msg: &mut Message, part_id: PartId, state: ToolState) {
        for part in &mut msg.parts {
            if let Part::ToolCall(ref mut tc) = part {
                if tc.base.id == part_id {
                    tc.state = state;
                    return;
                }
            }
        }
    }
}

// Helper trait to extract input from any ToolState variant.
trait ToolInput {
    fn input(&self) -> &serde_json::Value;
}

impl ToolInput for ToolCallPart {
    fn input(&self) -> &serde_json::Value {
        match &self.state {
            ToolState::Pending(s) => &s.input,
            ToolState::Running(s) => &s.input,
            ToolState::Completed(s) => &s.input,
            ToolState::Error(s) => &s.input,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mew_hooks::NopDispatcher;
    use mew_message::{
        Finish, PartBase, ReasoningPart, ToolStatePending, ToolTime,
    };
    use mew_provider::{EventStream, ProviderError, Request};
    use mew_provider_fake::FakeProvider;
    use std::sync::Mutex as StdMutex;

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
            })
        }
    }

    // ------------------------------------------------------------------
    // Unit tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_system() {
        let agent = Agent::new(
            Arc::new(FakeProvider::new(vec![])),
            Arc::new(NopDispatcher),
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
            Arc::new(FakeProvider::new(vec![])),
            Arc::new(NopDispatcher),
            None,
            vec![],
            None,
        );
        let mut msg = Message {
            id: Ulid::new(),
            session_id: agent.session_id,
            role: Role::Assistant,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
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
        let agent = Agent::new(
            Arc::new(FakeProvider::new(vec![])),
            Arc::new(NopDispatcher),
            None,
            vec![],
            None,
        );
        let mut msg = Message {
            id: Ulid::new(),
            session_id: agent.session_id,
            role: Role::Assistant,
            parts: vec![Part::Reasoning(ReasoningPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id: Ulid::new(),
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
            Arc::new(FakeProvider::new(vec![])),
            Arc::new(NopDispatcher),
            None,
            vec![],
            None,
        );
        let session_id = agent.session_id;
        let msg_id = Ulid::new();
        let now = Utc::now().timestamp_millis();

        let pending_part = ToolCallPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: msg_id,
                session_id,
            },
            tool_name: "echo".into(),
            call_id: "c1".into(),
            state: ToolState::Pending(ToolStatePending {
                input: serde_json::Value::Null,
                time: ToolTime { start: now, end: None },
            }),
            raw_input: String::new(),
        };
        let completed_part = ToolCallPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: msg_id,
                session_id,
            },
            tool_name: "echo".into(),
            call_id: "c2".into(),
            state: ToolState::Completed(mew_message::ToolStateCompleted {
                input: serde_json::Value::Null,
                output: "done".into(),
                metadata: None,
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
        let agent = Agent::new(
            Arc::new(FakeProvider::new(vec![])),
            Arc::new(NopDispatcher),
            None,
            vec![],
            None,
        );
        let session_id = agent.session_id;
        let msg_id = Ulid::new();
        let part_id = Ulid::new();
        let now = Utc::now().timestamp_millis();

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
                    time: ToolTime { start: now, end: None },
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
            time: ToolTime { start: now, end: None },
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
        let provider = Arc::new(FakeProvider::new(script));
        let agent = Agent::new(provider, Arc::new(NopDispatcher), None, vec![], None);

        let mut rx = agent.run("hi".into());
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }

        // Should see PartStart, some PartDeltas, PartEnd, MessageEnd
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::PartStart { .. })))
        );
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::PartDelta { .. })))
        );
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::PartEnd { .. })))
        );
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Provider(ProviderEvent::MessageEnd { .. })))
        );

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
        let provider = Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
        let agent = Agent::new(
            provider,
            Arc::new(NopDispatcher),
            None,
            vec![Arc::new(EchoTool::mutating())],
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
                    let _ = tx.send(PermissionDecision::AllowOnce);
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
        let provider = Arc::new(StatefulFakeProvider::new(vec![script1, script2]));
        let agent = Agent::new(
            provider,
            Arc::new(NopDispatcher),
            None,
            vec![Arc::new(EchoTool::mutating())],
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
                    let _ = tx.send(PermissionDecision::Deny);
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
        let provider = Arc::new(FakeProvider::new(script));
        let agent = Agent::new(provider, Arc::new(NopDispatcher), None, vec![], None);

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
}
