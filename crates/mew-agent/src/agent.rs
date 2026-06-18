use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use mew_hooks::Dispatcher;
use mew_message::{Message, Part, SessionId};
use mew_provider::{Provider, ReasoningConfig};
use mew_subagents::{SubagentDef, SubagentRunner};
use mew_tools::Tool;

use crate::{AgentEvent, SessionWriter};

/// Status of a background subagent task.
pub struct SubagentTask {
    pub name: String,
    pub started_at: i64,
    pub result_rx: Option<
        tokio::sync::oneshot::Receiver<
            Result<mew_subagents::SubagentResult, mew_subagents::SubagentError>,
        >,
    >,
    /// Per-task cancellation token. A child of the agent's own cancel_token,
    /// so cancelling the agent cascades to running subagents, but cancelling
    /// a single subagent does not affect siblings or the agent.
    pub cancel: tokio_util::sync::CancellationToken,
    /// Session id of the subagent's own session file, set when the runner
    /// reports its `Started` event. Used for the session pop-in feature.
    pub child_session_id: Arc<tokio::sync::Mutex<Option<String>>>,
}

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
    pub max_turns: Option<u32>,
    /// Maximum allowed nesting depth for subagent spawning. Top-level sessions
    /// are depth 0; a subagent of a depth-N session would be depth N+1. The
    /// spawn is rejected if N+1 exceeds this cap.
    pub max_subagent_depth: u32,
    pub subagent_defs: Vec<SubagentDef>,
    pub subagent_runner: Option<Arc<dyn SubagentRunner>>,
    /// Background subagent tasks: task_id → task.
    pub subagent_tasks: Arc<tokio::sync::Mutex<HashMap<String, SubagentTask>>>,
    /// Directories the agent is allowed to read/write within.
    pub workspace_roots: Vec<PathBuf>,
    /// Additional directories approved for this session.
    pub workspace_allowances: Arc<tokio::sync::Mutex<HashSet<PathBuf>>>,
    pub(crate) force_compact: Arc<tokio::sync::Mutex<bool>>,
    /// Current reasoning/thinking configuration, if any.
    pub reasoning: Option<ReasoningConfig>,
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
            session_id: session_id.unwrap_or_default(),
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
            max_turns: None,
            max_subagent_depth: 3,
            subagent_defs: Vec::new(),
            subagent_runner: None,
            subagent_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            workspace_roots: Vec::new(),
            workspace_allowances: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            force_compact: Arc::new(tokio::sync::Mutex::new(false)),
            reasoning: None,
        }
    }

    pub fn set_permission_engine(
        &mut self,
        engine: Arc<mew_config::permissions::PermissionEngine>,
    ) {
        self.permission_engine = Some(engine);
    }

    pub fn set_system(&mut self, system: String) {
        self.system = system;
    }

    pub fn set_reasoning(&mut self, config: Option<ReasoningConfig>) {
        self.reasoning = config;
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

    pub fn run_with_parts(
        &self,
        prompt: String,
        attachments: Vec<Part>,
    ) -> mpsc::Receiver<AgentEvent> {
        let (tx, rx) = mpsc::channel(256);
        let agent = self.clone();

        tokio::spawn(async move {
            if let Err(e) = agent.run_loop(prompt, attachments, tx).await {
                tracing::error!("agent loop ended with error: {}", e);
            }
        });

        rx
    }

    /// Spawn a subagent in the background. Returns a task ID immediately.
    pub async fn start_subagent(
        &self,
        name: &str,
        prompt: &str,
        ev_tx: &mpsc::Sender<AgentEvent>,
    ) -> Result<String, String> {
        let def = self
            .subagent_defs
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| format!("unknown subagent: {}", name))?
            .clone();

        // Depth cap: parent's depth + 1 must not exceed max_subagent_depth.
        // Top-level sessions are depth 0; their direct subagents are depth 1.
        let parent_depth = if let Some(session) = &self.session {
            session.lock().await.meta().depth
        } else {
            0
        };
        if parent_depth + 1 > self.max_subagent_depth {
            return Err(format!(
                "subagent nesting depth exceeded (parent depth {}, max {})",
                parent_depth, self.max_subagent_depth
            ));
        }

        let runner = self
            .subagent_runner
            .clone()
            .ok_or_else(|| "no subagent runner configured".to_string())?;

        let task_id = format!("sa_{}", ulid::Ulid::new());
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        // Per-task child of the agent's cancel token. Cancelling the agent
        // cascades to all subagents; cancelling a single subagent only stops
        // that one.
        let task_cancel = self.cancel_token.child_token();
        let cancel = task_cancel.clone();
        let prompt = prompt.to_string();
        let call_id = task_id.clone();
        let name_clone = name.to_string();
        let ev_tx_clone = ev_tx.clone();
        let parent_session_id = self.session_id;
        let child_session_id_slot = Arc::new(tokio::sync::Mutex::new(None::<String>));
        let child_id_for_pump = child_session_id_slot.clone();

        tokio::spawn(async move {
            let (event_tx, mut event_rx) = mpsc::channel(256);
            let pump_cid = call_id.clone();
            let pump_child_id = child_id_for_pump;

            // Pump events to parent in a background task. The first event
            // (Started) carries the real child session id; we record it for
            // future pop-in and use it in the AgentEvent translation.
            let pump = tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    let agent_event = match event {
                        mew_subagents::SubagentEvent::Started {
                            child_session_id,
                            display_name,
                        } => {
                            *pump_child_id.lock().await = Some(child_session_id.clone());
                            AgentEvent::SubagentStart {
                                parent_call_id: pump_cid.clone(),
                                name: name_clone.clone(),
                                child_session_id,
                                display_name,
                            }
                        }
                        mew_subagents::SubagentEvent::Finished {
                            child_session_id,
                            outcome,
                        } => AgentEvent::SubagentEnd {
                            parent_call_id: pump_cid.clone(),
                            child_session_id,
                            outcome,
                        },
                        mew_subagents::SubagentEvent::TextDelta { text } => {
                            AgentEvent::ToolProgress {
                                call_id: pump_cid.clone(),
                                chunk: text,
                            }
                        }
                        _ => continue,
                    };
                    let _ = ev_tx_clone.send(agent_event).await;
                }
            });

            let result = runner
                .run(&def, prompt, call_id, parent_session_id, event_tx, cancel)
                .await;

            let _ = pump.await;
            let _ = result_tx.send(result);
        });

        let mut tasks = self.subagent_tasks.lock().await;
        tasks.insert(
            task_id.clone(),
            SubagentTask {
                name: name.to_string(),
                started_at: chrono::Utc::now().timestamp_millis(),
                result_rx: Some(result_rx),
                cancel: task_cancel,
                child_session_id: child_session_id_slot,
            },
        );

        Ok(task_id)
    }

    /// Cancel a running subagent task. Returns true if the task was running
    /// and got cancelled. The task's `wait_subagent` will resolve with
    /// `SubagentResult::Cancelled` once the runner observes the cancellation.
    pub async fn cancel_subagent(&self, task_id: &str) -> bool {
        let tasks = self.subagent_tasks.lock().await;
        if let Some(task) = tasks.get(task_id) {
            task.cancel.cancel();
            true
        } else {
            false
        }
    }

    /// Wait for a background subagent to complete. Returns the structured result.
    pub async fn wait_subagent(
        &self,
        task_id: &str,
    ) -> Result<mew_subagents::SubagentResult, String> {
        let mut tasks = self.subagent_tasks.lock().await;
        let mut task = tasks
            .remove(task_id)
            .ok_or_else(|| format!("unknown subagent task: {}", task_id))?;

        let result_rx = task
            .result_rx
            .take()
            .ok_or_else(|| format!("task {} already awaited", task_id))?;

        drop(tasks);

        let result = result_rx
            .await
            .map_err(|_| "subagent task cancelled".to_string())?;

        result.map_err(|e| format!("subagent error: {}", e))
    }

    /// List running subagent tasks (name, task_id, elapsed_ms).
    pub async fn list_subagents(&self) -> Vec<(String, String, i64)> {
        let tasks = self.subagent_tasks.lock().await;
        let now = chrono::Utc::now().timestamp_millis();
        tasks
            .iter()
            .map(|(id, t)| (t.name.clone(), id.clone(), now - t.started_at))
            .collect()
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
