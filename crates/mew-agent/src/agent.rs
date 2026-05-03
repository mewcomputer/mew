use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ulid::Ulid;

use mew_hooks::Dispatcher;
use mew_message::{Message, Part, SessionId};
use mew_provider::Provider;
use mew_tools::Tool;

use crate::{AgentEvent, SessionWriter};

#[derive(Clone)]
pub struct Agent {
    pub provider: Arc<dyn Provider>,
    pub dispatcher: Arc<dyn Dispatcher>,
    pub session: Option<SessionWriter>,
    pub tools: HashMap<String, Arc<dyn Tool>>,
    pub messages: Arc<tokio::sync::Mutex<Vec<Message>>>,
    pub session_id: SessionId,
    pub system: String,
    pub cancel_token: CancellationToken,
    pub permission_engine: Option<Arc<mew_config::permissions::PermissionEngine>>,
    pub supports_vision: bool,
    pub input_price: f64,
    pub output_price: f64,
    pub cache_read_price: f64,
    pub cache_write_price: f64,
    pub reasoning_price: f64,
    pub context_window: u32,
    pub compaction_threshold: f64,
    pub keep_turns: usize,
    pub(crate) force_compact: Arc<tokio::sync::Mutex<bool>>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        dispatcher: Arc<dyn Dispatcher>,
        session: Option<mew_session::Writer>,
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
            session: session.map(|w| Arc::new(tokio::sync::Mutex::new(w))),
            tools: tools_map,
            messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            session_id: session_id.unwrap_or_else(Ulid::new),
            system: String::new(),
            cancel_token: CancellationToken::new(),
            permission_engine: None,
            supports_vision: true,
            input_price: 0.0,
            output_price: 0.0,
            cache_read_price: 0.0,
            cache_write_price: 0.0,
            reasoning_price: 0.0,
            context_window: 0,
            compaction_threshold: 0.95,
            keep_turns: 4,
            force_compact: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }

    pub fn set_permission_engine(&mut self, engine: Arc<mew_config::permissions::PermissionEngine>) {
        self.permission_engine = Some(engine);
    }

    pub fn set_system(&mut self, system: String) {
        self.system = system;
    }

    pub async fn load_messages(&self, messages: Vec<Message>) {
        *self.messages.lock().await = messages;
    }

    pub async fn force_compact(&self) {
        *self.force_compact.lock().await = true;
    }

    pub(crate) fn estimated_tokens(&self, messages: &[Message]) -> u32 {
        let mut chars: usize = self.system.chars().count();
        for msg in messages {
            for part in &msg.parts {
                if let Part::Text(tp) = part {
                    chars += tp.text.chars().count();
                } else if let Part::Reasoning(rp) = part {
                    chars += rp.text.chars().count();
                } else if let Part::ToolCall(tc) = part {
                    if let Some(output) = tc.state.output() {
                        chars += output.chars().count();
                    }
                }
            }
        }
        (chars / 4) as u32
    }

    pub fn run(&self, prompt: String) -> mpsc::Receiver<AgentEvent> {
        self.run_with_parts(prompt, vec![])
    }

    pub fn run_with_parts(&self, prompt: String, attachments: Vec<Part>) -> mpsc::Receiver<AgentEvent> {
        let (tx, rx) = mpsc::channel(256);
        let agent = self.clone();

        tokio::spawn(async move {
            if let Err(e) = agent.run_loop(prompt, attachments, tx).await {
                tracing::error!("agent loop ended with error: {}", e);
            }
        });

        rx
    }
}

pub(crate) trait ToolInput {
    fn input(&self) -> &serde_json::Value;
}

impl ToolInput for mew_message::ToolCallPart {
    fn input(&self) -> &serde_json::Value {
        match &self.state {
            mew_message::ToolState::Pending(s) => &s.input,
            mew_message::ToolState::Running(s) => &s.input,
            mew_message::ToolState::Completed(s) => &s.input,
            mew_message::ToolState::Error(s) => &s.input,
        }
    }
}
