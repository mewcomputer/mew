use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use mew_hooks::Dispatcher;
use mew_message::{Message, Part, PartBase, Role, SessionId, TextPart, Time};
use mew_provider::{Provider, ReasoningConfig};
use mew_subagents::{SubagentDef, SubagentRunner};
use mew_tools::tools::flag_important::FlaggedFile;
use mew_tools::SecretSet;
use mew_tools::Tool;
use ulid::Ulid;

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
    /// Files flagged as important for the session. These survive context
    /// compaction: `Included` files are re-injected as text, `Referenced`
    /// files get a pointer note. Shared with the `flag_important` tool.
    pub flagged_files: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>>,
    /// Secret words and file globs to redact from tool output. Shared (via
    /// `Arc`) with each `ToolCtx` built for a tool call.
    pub secrets: Arc<SecretSet>,
    /// Session-lived, dependency-enforced todo list. Survives compaction (it's
    /// agent state, not message history) and resume (persisted to
    /// `todos_path`).
    pub todos: Arc<tokio::sync::Mutex<crate::TodoList>>,
    /// Where to persist `todos`. `None` when there's no session (tests,
    /// non-interactive runs that skip the writer).
    pub todos_path: Option<std::path::PathBuf>,
    /// Current reasoning/thinking configuration, if any.
    pub reasoning: Option<ReasoningConfig>,
    /// Active persona's system-prompt body. Prepended to `self.system` in
    /// the turn loop. `None` when no persona is active (default behavior).
    pub persona_prompt: Option<String>,
    /// Active persona's tool allow-list. `None` = all tools (subject to
    /// `denied_tool_names`); `Some(set)` = only tools whose name is in the
    /// set are sent to the provider.
    pub active_tool_names: Option<HashSet<String>>,
    /// Active persona's tool denylist. Always applied, even when
    /// `active_tool_names` is `None` (so "all tools except X" works).
    /// Empty by default.
    pub denied_tool_names: HashSet<String>,
    /// Active persona's skill allow-list. `None` = all skills; `Some(set)` =
    /// only skills whose name is in the set. Shared with the `Skill` tool
    /// via `Arc` so the tool can gate its `execute` and the agent can
    /// rebuild the system prompt's skills listing.
    pub skill_filter: Arc<tokio::sync::RwLock<Option<HashSet<String>>>>,
    /// All skills discovered at startup. Used by `rebuild_system` to render
    /// the `<available_skills>` block in the system prompt, filtered by the
    /// current `skill_filter`.
    pub skills: Vec<mew_skills::Skill>,
    /// Base system prompt (without the skills XML appended). Stored so
    /// `rebuild_system` can re-derive `self.system` after a persona change
    /// without losing the original context prompt.
    pub base_system: String,
    /// All personas discovered at startup. Used by the turn loop when a
    /// `switch_persona` tool call queued a switch: the agent looks up the
    /// persona by name at end of turn and emits `PersonaSwitchRequested`
    /// for the main loop to apply.
    pub personas: Vec<mew_personas::Persona>,
    /// Pending persona switch queued by the `switch_persona` tool. Shared
    /// with the tool via `Arc<Mutex<...>>` so the tool can set it during
    /// `execute` and the turn loop can drain it at end of turn. The
    /// actual switch (model pin, provider rebuild) is the main loop's
    /// responsibility, triggered by the `PersonaSwitchRequested` event.
    pub pending_persona_switch: Arc<tokio::sync::Mutex<Option<String>>>,
    /// Active persona name (for display/status). `None` = no persona.
    pub persona_name: Option<String>,
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
            flagged_files: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            secrets: Arc::new(SecretSet::default()),
            todos: Arc::new(tokio::sync::Mutex::new(crate::TodoList::new())),
            todos_path: None,
            reasoning: None,
            persona_prompt: None,
            active_tool_names: None,
            denied_tool_names: HashSet::new(),
            skill_filter: Arc::new(tokio::sync::RwLock::new(None)),
            skills: Vec::new(),
            base_system: String::new(),
            personas: Vec::new(),
            pending_persona_switch: Arc::new(tokio::sync::Mutex::new(None)),
            persona_name: None,
        }
    }

    pub fn set_permission_engine(
        &mut self,
        engine: Arc<mew_config::permissions::PermissionEngine>,
    ) {
        self.permission_engine = Some(engine);
    }

    pub fn set_system(&mut self, system: String) {
        self.base_system = system.clone();
        self.system = system;
    }

    /// Discover tools registered by plugins (via `on_register_tools`) and
    /// merge them into the agent's tool registry. Idempotent — calling
    /// twice won't double-register tools. Plugin tools default to
    /// `Sensitivity::Mutating` since their effects are unknown to the host.
    pub async fn register_plugin_tools(&mut self) {
        let registrations = self.dispatcher.on_register_tools().await;
        for reg in registrations {
            if self.tools.contains_key(&reg.name) {
                tracing::warn!(
                    plugin_tool = %reg.name,
                    "plugin tried to register a tool that already exists; skipping"
                );
                continue;
            }
            let tool: Arc<dyn mew_tools::Tool> = Arc::new(PluginTool::new(reg));
            self.tools.insert(tool.name().to_string(), tool);
        }
    }

    /// Set the discovered skills. Triggers a system-prompt rebuild so the
    /// `<available_skills>` block reflects the active persona's filter.
    pub fn set_skills(&mut self, skills: Vec<mew_skills::Skill>) {
        self.skills = skills;
        self.rebuild_system();
    }

    /// Set the discovered personas. Stored for the turn loop to look up
    /// a target persona by name when a `switch_persona` tool call is
    /// drained at end of turn.
    pub fn set_personas(&mut self, personas: Vec<mew_personas::Persona>) {
        self.personas = personas;
    }

    /// Provide a shared `pending_persona_switch` slot. Should be the same
    /// `Arc` passed to the `SwitchPersona` tool so the tool can queue a
    /// switch and the turn loop can drain it.
    pub fn set_pending_persona_switch(&mut self, slot: Arc<tokio::sync::Mutex<Option<String>>>) {
        self.pending_persona_switch = slot;
    }

    /// Rebuild `self.system` from `self.base_system` + the current skills
    /// listing (filtered by the active persona's skill allow-list). Called
    /// automatically by `set_skills`, `apply_persona`, and `clear_persona`.
    /// Exposed for callers that mutate filters out of band.
    pub fn rebuild_system(&mut self) {
        let filter = self.skill_filter.try_read().ok().and_then(|g| g.clone());
        let allowed = filter.as_ref();
        let visible: Vec<&mew_skills::Skill> = self
            .skills
            .iter()
            .filter(|s| allowed.is_none_or(|set| set.contains(&s.name)))
            .collect();
        let mut out = self.base_system.clone();
        if !visible.is_empty() {
            out.push_str(&build_skills_xml(&visible));
        }
        self.system = out;
    }

    /// Activate a persona. Sets the persona prompt (prepended to `self.system`
    /// in the turn loop), filters the tool set (allowlist + denylist), gates
    /// the skills the model can see/load, and records the persona name for
    /// display.
    ///
    /// Returns the persona's pinned model (if any) so the caller can rebuild
    /// the provider.
    pub fn apply_persona(&mut self, persona: &mew_personas::Persona) -> Option<String> {
        self.persona_name = Some(persona.name.clone());

        // Compute tool filters before rendering the prompt so a templated
        // persona body can reference the effective toolset.
        self.active_tool_names = persona
            .config
            .tools
            .as_ref()
            .map(|tools| tools.iter().cloned().collect::<HashSet<_>>());
        self.denied_tool_names = persona
            .config
            .tools_deny
            .as_ref()
            .map(|d| d.iter().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();

        self.persona_prompt = if persona.body.is_empty() {
            None
        } else if persona.config.template == Some(true) {
            Some(render_persona_template(
                &persona.body,
                &persona.name,
                self.supports_vision,
                &self.active_tool_names,
                &self.tools,
                &self.denied_tool_names,
            ))
        } else {
            Some(persona.body.clone())
        };
        // Block in-line to update the shared skill filter. The Skill tool
        // shares the same Arc and reads it on every execute; the system
        // prompt rebuild below picks up the new filter too.
        if let Some(skills) = persona.config.skills.as_ref() {
            let allow = skills.iter().cloned().collect::<HashSet<_>>();
            if let Ok(mut g) = self.skill_filter.try_write() {
                *g = Some(allow);
            }
        } else if let Ok(mut g) = self.skill_filter.try_write() {
            *g = None;
        }
        self.rebuild_system();
        persona.config.model.clone()
    }

    /// Clear the active persona, restoring full tool access, the denylist
    /// default (empty), and the unfiltered skills list.
    pub fn clear_persona(&mut self) {
        self.persona_name = None;
        self.persona_prompt = None;
        self.active_tool_names = None;
        self.denied_tool_names.clear();
        if let Ok(mut g) = self.skill_filter.try_write() {
            *g = None;
        }
        self.rebuild_system();
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

    /// Clear the in-memory conversation context and append a clear marker to
    /// the session log. The session file is the immutable event log and keeps
    /// everything; only what the model sees this turn is reset. Resume
    /// reconstructs forward from the marker.
    pub async fn clear_context(&self) {
        self.messages.lock().await.clear();

        if let Some(session) = &self.session {
            let now = chrono::Utc::now().timestamp_millis();
            let msg_id = Ulid::new();
            let marker = Message {
                id: msg_id,
                session_id: self.session_id,
                role: Role::User,
                parts: vec![Part::Text(TextPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id: msg_id,
                        session_id: self.session_id,
                    },
                    text: "Context cleared. The model starts fresh; prior turns are no longer visible to it.".into(),
                    synthetic: true,
                })],
                time: Time {
                    created: now,
                    completed: None,
                },
                assistant: None,
            };
            if let Err(e) = session.lock().await.write_message(&marker).await {
                tracing::warn!(error = %e, "failed to write clear marker to session");
            }
        }
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

/// Render a persona body as a minijinja template, exposing the agent's
/// current capabilities so persona authors can write conditional system
/// prompts (e.g. "you can see images" only when `supports_vision` is true).
///
/// Variables available in the template:
/// - `supports_vision` (bool)
/// - `persona_name` (string)
/// - `tools` (list of active tool names)
/// - `denied_tools` (list of explicitly denied tool names)
///
/// Falls back to the raw body on any render error (with a warning) so a
/// typo in the template never bricks the persona.
fn render_persona_template(
    body: &str,
    persona_name: &str,
    supports_vision: bool,
    active_tool_names: &Option<HashSet<String>>,
    all_tools: &HashMap<String, Arc<dyn Tool>>,
    denied_tool_names: &HashSet<String>,
) -> String {
    use minijinja::context;

    // Compute the effective tool list the model will see this turn:
    // start from the full registry, apply the allowlist if set, then
    // subtract the denylist.
    let effective: Vec<String> = all_tools
        .keys()
        .filter(|name| {
            let allowed = active_tool_names
                .as_ref()
                .is_none_or(|set| set.contains(*name));
            allowed && !denied_tool_names.contains(*name)
        })
        .cloned()
        .collect();

    let denied: Vec<String> = denied_tool_names.iter().cloned().collect();

    let ctx = context! {
        supports_vision => supports_vision,
        persona_name => persona_name,
        tools => effective,
        denied_tools => denied,
    };

    minijinja::Environment::new()
        .render_str(body, ctx)
        .unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                persona = %persona_name,
                "persona template render failed, falling back to raw body"
            );
            body.to_string()
        })
}

/// Build the `<available_skills>` block for the system prompt from a
/// slice of skill references. Renders one `<skill>` element per skill with
/// its name and description, XML-escaped.
fn build_skills_xml(skills: &[&mew_skills::Skill]) -> String {
    let mut buf = String::from("<available_skills>\n");
    for skill in skills {
        buf.push_str(&format!(
            "  <skill>\n    <name>{}</name>\n    <description>{}</description>\n  </skill>\n",
            escape_xml(&skill.name),
            escape_xml(&skill.description),
        ));
    }
    buf.push_str("</available_skills>\n");
    buf
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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

/// Wraps a `ToolRegistration` from a plugin into a `mew_tools::Tool` impl
/// so it can be stored alongside built-in tools in the agent's registry.
/// The `execute` closure is `Fn + Send + Sync` (not async) per the
/// `ToolRegistration` contract, so plugin tools run synchronously on the
/// dispatcher caller's task. Plugins needing async work should spawn their
/// own runtime.
struct PluginTool {
    reg: mew_hooks::ToolRegistration,
    schema: serde_json::Value,
}

impl PluginTool {
    fn new(reg: mew_hooks::ToolRegistration) -> Self {
        let schema = reg.input_schema.clone();
        Self { reg, schema }
    }
}

#[async_trait::async_trait]
impl mew_tools::Tool for PluginTool {
    fn name(&self) -> &str {
        &self.reg.name
    }

    fn description(&self) -> &str {
        &self.reg.description
    }

    fn schema(&self) -> &serde_json::Value {
        &self.schema
    }

    fn sensitivity(&self) -> mew_tools::Sensitivity {
        // Plugin tools default to Mutating: the host can't see what they do
        // under the hood, so treat them like an untrusted write tool.
        mew_tools::Sensitivity::Mutating
    }

    async fn execute(
        &self,
        _ctx: mew_tools::ToolCtx,
        input: serde_json::Value,
    ) -> Result<mew_hooks::ToolOutput, mew_tools::ToolError> {
        let result = (self.reg.execute)(input).await;
        Ok(mew_hooks::ToolOutput {
            output: result,
            error: String::new(),
            diff: None,
        })
    }
}
