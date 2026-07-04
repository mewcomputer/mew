use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use mew_hooks::Dispatcher;
use mew_message::{Message, Part, PartBase, Role, SessionId, TextPart, Time};
use mew_provider::{Provider, ReasoningConfig};
use mew_subagents::{SubagentDef, SubagentRunOptions, SubagentRunner};
use mew_tools::tools::flag_important::FlaggedFile;
use mew_tools::SecretSet;
use mew_tools::Tool;
use ulid::Ulid;

use crate::{AgentEvent, SessionWriter};

/// Session-scoped classifier decision cache. Keyed by
/// `(tool_name, serialized_input)`; values are the classifier's parsed
/// decisions. Cleared on `/clear`.
pub type ClassifierCache = std::sync::Mutex<
    std::collections::HashMap<(String, String), mew_prompts::classifier::ClassifierDecision>,
>;

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

/// Lifecycle state of a background shell job.
#[derive(Debug, Clone)]
pub enum ShellJobState {
    Running,
    Completed { exit_code: i32 },
    Failed { reason: String },
    Cancelled,
}

impl ShellJobState {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// A background shell command tracked as a job. The process runs in a
/// background task that reads stdout/stderr into `output` and updates
/// `state` on exit. Callers use `done` to wait for completion.
pub struct ShellJob {
    pub id: String,
    pub command: String,
    pub started_at: i64,
    pub cancel: tokio_util::sync::CancellationToken,
    /// Accumulated stdout + stderr (interleaved). Updated by the
    /// background reader task.
    pub output: Arc<tokio::sync::Mutex<String>>,
    /// Current state. `Running` until the process exits or is cancelled.
    pub state: Arc<tokio::sync::Mutex<ShellJobState>>,
    /// Notified once when the job reaches a terminal state. Used by
    /// `job_block` to wait without polling.
    pub done: Arc<tokio::sync::Notify>,
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
    /// Truncates long reasoning traces and forces a tool call on the
    /// next turn. See `ReasoningTruncator` for the rationale.
    pub reasoning_truncator: crate::reasoning_truncator::ReasoningTruncator,
    /// Master switch for the truncation behaviour. When false, the
    /// truncator is a no-op (its threshold is ignored and its
    /// force-tool flag is never consumed).
    pub reasoning_truncation_enabled: bool,
    /// Default value for `params.max_tokens` when neither the user
    /// nor a dispatcher plugin has set one. 0 = no override (let
    /// the provider pick its own default, e.g. 4096 for Anthropic).
    /// Stored as `i64` to accommodate future heuristic-derived
    /// values; the cast to the wire-level `i32` is saturating — see
    /// the call site in `turn.rs` (clamp happens there, not in the
    /// setter).
    pub default_max_output_tokens: i64,
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
    /// Background shell jobs: job_id → job. Populated by `shell_background`,
    /// drained by `job_block` / `job_cancel`.
    pub shell_jobs: Arc<tokio::sync::Mutex<HashMap<String, ShellJob>>>,
    /// Directories the agent is allowed to read/write within.
    pub workspace_roots: Vec<PathBuf>,
    /// The working directory for this agent/session. Defaults to the process
    /// cwd in `Agent::new`. Daemon sessions override it per-session so
    /// multiple projects can be served by one daemon. All file tools,
    /// the permission engine, and template context read from this.
    pub cwd: PathBuf,
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
    /// Snapshot store shared with `read` and `edit_hashline` so tags and
    /// seen-line provenance survive across tool calls in a session.
    pub snapshot_store: Arc<dyn mew_hashline::SnapshotStore>,
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
    /// Template context shared with the `Skill` tool so templated skills
    /// can render with current model/persona/session info. Updated by
    /// `apply_persona` and `set_model_info`.
    pub template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>>,
    /// Project-local variables from `.mew/project_vars.yaml`. Accessible as
    /// `project_vars` in templates.
    pub project_vars: std::collections::HashMap<String, String>,
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
    /// Current persona name, shared with the `switch_persona` tool so it
    /// can look up transition rules for the *active* persona before
    /// queuing a switch. Updated by `apply_persona` and `clear_persona`.
    pub current_persona_name: Arc<tokio::sync::RwLock<Option<String>>>,
    /// Active persona name (for display/status). `None` = no persona.
    pub persona_name: Option<String>,
    /// Active persona's autonomous hint for the Auto/Auto+ permission
    /// classifier. Injected into the classifier prompt to steer decisions
    /// (e.g. "this persona is read-only; be strict about shell commands").
    /// `None` = no hint (classifier uses its default prompt).
    pub autonomous_hint: Option<String>,
    /// Active persona's transition rules. Controls which personas this
    /// one can switch to and whether confirmation is required. `None` =
    /// unrestricted.
    pub persona_transitions: Option<mew_personas::TransitionRules>,
    /// Active persona's fallback models. Tried in order if the primary
    /// model's provider returns a stream error. `None` or empty = no
    /// fallbacks.
    pub fallback_models: Option<Vec<String>>,
    /// Current model ID (e.g. "deepseek-v4-flash"). Set by the main loop
    /// when building or switching the provider. Used in template context.
    pub model_id: String,
    /// Current provider ID (e.g. "deepseek"). Derived from the provider
    /// name. Used in template context.
    pub provider_id: String,
    /// Provider used by the Auto permission mode to classify tool calls
    /// (the small LLM that decides allow / deny / escalate when the user
    /// has selected Auto mode). `None` → Auto mode falls through to the
    /// user modal at runtime.
    pub classifier_provider: Option<Arc<dyn Provider>>,
    /// Model id to use when calling the classifier provider. Each provider
    /// has a default model if unset; the CLI / config can pin a specific
    /// one (e.g. `gpt-4o-mini` for cost or a local model for privacy).
    pub classifier_model: Option<String>,
    /// Session-scoped cache of classifier decisions, keyed by
    /// `(tool_name, serialized_input)`. Cleared on `/clear`. Subagents
    /// share this cache via the agent reference — there's no per-subagent
    /// classifier provider today.
    pub classifier_cache: Option<Arc<ClassifierCache>>,
    /// Path to the plan file (e.g. `PLAN.md`). When set and the file exists,
    /// it's auto-flagged as important at the start of each turn so it
    /// survives context compaction without the model having to remember.
    /// Set from `config.toml: plan_path = "PLAN.md"`.
    pub plan_path: Option<PathBuf>,
    /// Optional callback that builds a new provider for a given
    /// `provider/model` string. Used by the turn loop to try fallback
    /// models when the primary provider returns a stream error. The
    /// main loop sets this via `set_provider_builder`; when `None`,
    /// stream errors are fatal (the existing behavior).
    pub provider_builder: Option<Arc<ProviderBuilder>>,
    /// Optional persistent shell session shared with the `bash` tool.
    /// When `Some`, bash commands run in a long-lived shell process so
    /// `cd`, `export`, and other state survive across calls. When
    /// `None`, each bash call spawns a fresh process (the existing
    /// behavior). Set via `set_shell_session`.
    pub shell_session: Option<mew_tools::tools::shell_session::SharedShellSession>,
}

/// Builds a new provider for a `provider/model` pair. Used by the turn
/// loop to retry with fallback models when the primary provider fails.
pub type ProviderBuilderFn =
    Box<dyn Fn(&str) -> Result<Arc<dyn mew_provider::Provider>, String> + Send + Sync>;

/// Wrapper around the boxed builder so it can be cloned via `Arc`.
pub struct ProviderBuilder(pub ProviderBuilderFn);

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
            // Defaults to enabled with the 5k-token threshold recommended
            // in the publish that motivated this feature. Use
            // `set_reasoning_truncation_threshold(0)` to disable.
            reasoning_truncator: crate::reasoning_truncator::ReasoningTruncator::default(),
            reasoning_truncation_enabled: true,
            default_max_output_tokens: 0,
            subagent_runner: None,
            subagent_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            shell_jobs: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            workspace_roots: Vec::new(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            workspace_allowances: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            force_compact: Arc::new(tokio::sync::Mutex::new(false)),
            flagged_files: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            secrets: Arc::new(SecretSet::default()),
            snapshot_store: Arc::new(mew_hashline::InMemorySnapshotStore::new()),
            todos: Arc::new(tokio::sync::Mutex::new(crate::TodoList::new())),
            todos_path: None,
            reasoning: None,
            persona_prompt: None,
            active_tool_names: None,
            denied_tool_names: HashSet::new(),
            skill_filter: Arc::new(tokio::sync::RwLock::new(None)),
            template_ctx: Arc::new(tokio::sync::RwLock::new(None)),
            project_vars: std::collections::HashMap::new(),
            skills: Vec::new(),
            base_system: String::new(),
            personas: Vec::new(),
            pending_persona_switch: Arc::new(tokio::sync::Mutex::new(None)),
            current_persona_name: Arc::new(tokio::sync::RwLock::new(None)),
            persona_name: None,
            autonomous_hint: None,
            persona_transitions: None,
            fallback_models: None,
            model_id: String::new(),
            provider_id: String::new(),
            classifier_provider: None,
            classifier_model: None,
            classifier_cache: None,
            plan_path: None,
            provider_builder: None,
            shell_session: None,
        }
    }

    /// Set the current model and provider IDs. Used for template context
    /// (so persona/skill/subagent templates can reference `model_id` and
    /// `provider_id`). The main loop calls this after constructing the agent
    /// and whenever the model is switched.
    pub fn set_model_info(&mut self, model_id: &str, provider_id: &str) {
        self.model_id = model_id.to_string();
        self.provider_id = provider_id.to_string();
    }

    pub fn set_permission_engine(
        &mut self,
        engine: Arc<mew_config::permissions::PermissionEngine>,
    ) {
        self.permission_engine = Some(engine);
    }

    /// Switch the runtime permission mode. Called by the `/permissions` slash
    /// command and the `--dangerously-skip-permissions` CLI flag (via the
    /// engine's initial mode). Cheap; takes effect on the next tool call.
    pub fn set_permission_mode(&self, mode: mew_hooks::PermissionMode) {
        if let Some(ref engine) = self.permission_engine {
            engine.set_mode(mode);
        }
    }

    /// Current permission mode (Standard by default if no engine is set).
    pub fn permission_mode(&self) -> mew_hooks::PermissionMode {
        self.permission_engine
            .as_ref()
            .map(|e| e.mode())
            .unwrap_or(mew_hooks::PermissionMode::Standard)
    }

    /// Clear the session-scoped classifier cache. Called on `/clear` and
    /// on `clear_context()` so a fresh context gets fresh classifier
    /// decisions (the old ones were made under different context).
    pub fn clear_classifier_cache(&self) {
        if let Some(ref cache) = self.classifier_cache {
            cache.lock().expect("classifier cache poisoned").clear();
        }
    }

    /// Build a serializable snapshot of the agent's current state. Used
    /// for the per-turn state dump at `<session_dir>/mew.state.json`.
    /// Returned synchronously — no I/O; the caller writes the file.
    pub fn state_snapshot(&self) -> serde_json::Value {
        let msg_count = self.messages.try_lock().map(|m| m.len()).unwrap_or(0);
        let flagged_count = self.flagged_files.try_lock().map(|f| f.len()).unwrap_or(0);
        let todo_count = self.todos.try_lock().map(|t| t.items.len()).unwrap_or(0);
        let classifier_cache_size = self
            .classifier_cache
            .as_ref()
            .and_then(|c| c.try_lock().ok())
            .map(|c| c.len())
            .unwrap_or(0);

        serde_json::json!({
            "session_id": self.session_id.to_string(),
            "permission_mode": self.permission_mode().id(),
            "active_persona": self.persona_name,
            "plan_path": self.plan_path,
            "classifier": {
                "provider": self.classifier_provider.as_ref().map(|p| p.name().to_string()),
                "model": self.classifier_model,
                "cache_size": classifier_cache_size,
            },
            "message_count": msg_count,
            "flagged_files": flagged_count,
            "todo_count": todo_count,
            "tools_registered": self.tools.len(),
        })
    }

    /// Write the state snapshot to `<session_dir>/mew.state.json` for
    /// external introspection (e.g. `cat` from another terminal). No-op if
    /// there's no active session. Atomic write: serializes to a temp file
    /// then renames, so other processes never see a half-written state.
    pub async fn dump_state_to(&self, session_dir: &std::path::Path) {
        let snapshot = self.state_snapshot();
        let path = session_dir.join("mew.state.json");
        let tmp = session_dir.join(".mew.state.json.tmp");
        let bytes = match serde_json::to_vec_pretty(&snapshot) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize state snapshot");
                return;
            }
        };
        if let Err(e) = tokio::fs::write(&tmp, &bytes).await {
            tracing::warn!(error = %e, path = %tmp.display(), "failed to write state dump temp");
            return;
        }
        if let Err(e) = tokio::fs::rename(&tmp, &path).await {
            tracing::warn!(error = %e, "failed to rename state dump temp");
        }
    }

    /// Set the provider (and optional model id) used by Auto mode to classify
    /// tool calls. If a provider isn't set when Auto mode is active,
    /// `classify_permission` returns `None` and the call falls through to
    /// the user modal — the safe default.
    ///
    /// Also initializes the session-scoped classifier cache. Subagents share
    /// the parent's cache via the agent reference (no per-subagent provider
    /// today).
    pub fn set_classifier_provider(&mut self, provider: Arc<dyn Provider>, model: Option<String>) {
        self.classifier_provider = Some(provider);
        self.classifier_model = model;
        self.classifier_cache = Some(Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::new(),
        )));
    }

    /// Set the plan file path. When set and the file exists, it's
    /// auto-flagged as important at the start of each turn so it survives
    /// context compaction.
    pub fn set_plan_path(&mut self, path: impl Into<PathBuf>) {
        self.plan_path = Some(path.into());
    }

    /// Call the configured classifier LLM to decide whether `tool_call`
    /// should be allowed. Returns `None` if no classifier provider is
    /// configured, the call fails, or the response can't be parsed —
    /// callers should treat `None` as "escalate to the user."
    pub async fn classify_permission(
        &self,
        tool_call: &mew_hooks::ToolCall,
    ) -> Option<mew_prompts::classifier::ClassifierDecision> {
        let provider = self.classifier_provider.as_ref()?;
        let sensitivity = self
            .tools
            .get(&tool_call.tool_name)
            .map(|t| t.sensitivity())
            .unwrap_or(mew_tools::Sensitivity::ReadOnly);

        // Check the session-scoped cache first. Key on (tool_name, serialized
        // input) — exact match required. No TTL: decisions don't change
        // within a session; `/clear` empties the cache.
        let cache_key = (
            tool_call.tool_name.clone(),
            serde_json::to_string(&tool_call.input).unwrap_or_default(),
        );
        if let Some(ref cache) = self.classifier_cache {
            if let Some(cached) = cache
                .lock()
                .expect("classifier cache poisoned")
                .get(&cache_key)
                .copied()
            {
                tracing::debug!(tool = %tool_call.tool_name, "classifier cache hit");
                return Some(cached);
            }
        }

        let cwd_str = Some(self.cwd.to_string_lossy().to_string());

        let model = self
            .classifier_model
            .clone()
            .unwrap_or_else(|| provider.name().to_string());

        let prompt = mew_prompts::classifier::permission_decision(
            &tool_call.tool_name,
            &tool_call.input,
            sensitivity_label(sensitivity),
            cwd_str.as_deref(),
            None,
            self.autonomous_hint.as_deref(),
        );

        let request = mew_provider::Request {
            model,
            messages: vec![mew_message::Message {
                id: ulid::Ulid::new(),
                session_id: self.session_id,
                role: mew_message::Role::User,
                parts: vec![mew_message::Part::Text(mew_message::TextPart {
                    base: mew_message::PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: self.session_id,
                    },
                    text: prompt,
                    synthetic: true,
                })],
                time: mew_message::Time {
                    created: chrono::Utc::now().timestamp_millis(),
                    completed: None,
                },
                assistant: None,
            }],
            tools: Vec::new(),
            system: String::new(),
            reasoning: None,
            params: Some(mew_provider::ChatParams {
                temperature: Some(0.0),
                max_tokens: Some(8),
                ..Default::default()
            }),
            headers: http::HeaderMap::new(),
        };

        let stream = match provider.stream(request).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "classifier provider error; escalating to user");
                return None;
            }
        };

        // Collect text from the response stream.
        use futures::StreamExt;
        let mut text = String::new();
        let mut stream = stream;
        while let Some(event) = stream.next().await {
            if let mew_provider::ProviderEvent::PartDelta { delta, .. } = event {
                text.push_str(&delta);
            }
        }

        let decision = mew_prompts::classifier::ClassifierDecision::parse(&text);
        if decision.is_none() {
            tracing::warn!(
                classifier_text = %text,
                "classifier response could not be parsed; escalating to user"
            );
        }
        // Cache successful decisions so repeated tool calls with the same
        // input don't hit the classifier. Only successful parses are cached
        // (None means "couldn't parse" — let the next call try again).
        if let Some(d) = decision {
            if let Some(ref cache) = self.classifier_cache {
                cache
                    .lock()
                    .expect("classifier cache poisoned")
                    .insert(cache_key, d);
            }
        }
        decision
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

    /// Share the current-persona-name slot with the `switch_persona` tool
    /// so it can look up transition rules for the active persona. The
    /// agent keeps this in sync via `apply_persona` / `clear_persona`.
    pub fn set_current_persona_name(&mut self, current: Arc<tokio::sync::RwLock<Option<String>>>) {
        // Preserve the current value if the agent already has a persona active.
        if let Some(ref name) = self.persona_name {
            if let Ok(mut g) = current.try_write() {
                *g = Some(name.clone());
            }
        }
        self.current_persona_name = current;
    }

    /// Set the provider builder callback used by the turn loop to try
    /// fallback models when the primary provider returns a stream error.
    /// When not set, stream errors are fatal (the existing behavior).
    /// The builder receives a `provider/model` string and returns a
    /// new provider, or an error message.
    pub fn set_provider_builder(&mut self, builder: ProviderBuilderFn) {
        self.provider_builder = Some(Arc::new(ProviderBuilder(builder)));
    }

    /// Set the persistent shell session. When set, the `bash` tool uses
    /// it instead of spawning a fresh process for each command. This
    /// means `cd`, `export`, and other state survive across calls.
    pub fn set_shell_session(
        &mut self,
        session: mew_tools::tools::shell_session::SharedShellSession,
    ) {
        self.shell_session = Some(session);
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
        if let Ok(mut g) = self.current_persona_name.try_write() {
            *g = Some(persona.name.clone());
        }
        self.autonomous_hint = persona.config.autonomous_hint.clone();
        self.persona_transitions = persona.config.transitions.clone();
        self.fallback_models = persona.config.fallback_models.clone();

        // Compute tool filters before rendering the prompt so a templated
        // persona body can reference the effective toolset.
        self.active_tool_names = persona.config.tools.as_ref().map(|tools| {
            let expanded = expand_tool_tags(tools, &self.tools);
            expanded.into_iter().collect::<HashSet<_>>()
        });
        self.denied_tool_names = persona
            .config
            .tools_deny
            .as_ref()
            .map(|d| d.iter().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();

        // Build the template context once: used for rendering the persona
        // body (if templated) and shared with the Skill tool for templated
        // skills.
        let tool_names: Vec<String> = self.tools.keys().cloned().collect();
        let (tools, denied_tools) = mew_prompts::template::TemplateContext::compute_tools(
            &tool_names,
            &self.active_tool_names,
            &self.denied_tool_names,
        );
        let ctx = mew_prompts::template::TemplateContext {
            supports_vision: self.supports_vision,
            persona_name: persona.name.clone(),
            model_id: self.model_id.clone(),
            provider_id: self.provider_id.clone(),
            session_id: self.session_id.to_string(),
            cwd: self.cwd.to_string_lossy().to_string(),
            current_date: mew_prompts::template::TemplateContext::today(),
            tools,
            denied_tools,
            skills: self.skills.iter().map(|s| s.name.clone()).collect(),
            project_vars: self.project_vars.clone(),
            ..Default::default()
        };

        self.persona_prompt = if persona.body.is_empty() {
            None
        } else if persona.config.template == Some(true) {
            Some(mew_prompts::persona::render_with_context(
                &persona.body,
                &ctx,
            ))
        } else {
            Some(persona.body.clone())
        };
        // Update the shared template context so templated skills can render
        // with the same model/persona/session info the persona used.
        if persona.config.template == Some(true) {
            if let Ok(mut g) = self.template_ctx.try_write() {
                *g = Some(ctx);
            }
        } else if let Ok(mut g) = self.template_ctx.try_write() {
            *g = None;
        }
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
        if let Ok(mut g) = self.current_persona_name.try_write() {
            *g = None;
        }
        self.persona_prompt = None;
        self.active_tool_names = None;
        self.denied_tool_names.clear();
        self.autonomous_hint = None;
        self.persona_transitions = None;
        self.fallback_models = None;
        if let Ok(mut g) = self.skill_filter.try_write() {
            *g = None;
        }
        self.rebuild_system();
    }

    pub fn set_reasoning(&mut self, config: Option<ReasoningConfig>) {
        self.reasoning = config;
    }

    /// Set the approximate-token threshold above which reasoning traces
    /// are truncated. Pass `0` to disable truncation entirely.
    pub fn set_reasoning_truncation_threshold(&mut self, threshold: u32) {
        self.reasoning_truncator.threshold = threshold;
    }

    /// Enable or disable the reasoning-truncation behaviour entirely.
    pub fn set_reasoning_truncation_enabled(&mut self, enabled: bool) {
        self.reasoning_truncation_enabled = enabled;
    }

    /// Set the default `max_output_tokens` used when no plugin
    /// dispatches one. `value < 0` is clamped to 0 (which disables
    /// the override — let the provider pick). Values exceeding
    /// `i32::MAX` are stored verbatim and saturated to `i32::MAX` at
    /// the `turn.rs` call site.
    pub fn set_default_max_output_tokens(&mut self, value: i64) {
        self.default_max_output_tokens = if value < 0 { 0 } else { value };
    }

    /// Walk `msg`'s parts; for each `ReasoningPart`, ask the truncator
    /// whether the text exceeds its threshold and (if so) replace it
    /// in place with the truncated form. Returns `true` if any part
    /// was truncated — the caller is responsible for forging an
    /// acknowledgement message and marking the truncator.
    pub fn maybe_truncate_reasoning_in_place(&mut self, msg: &mut mew_message::Message) -> bool {
        let mut truncated_any = false;
        for part in msg.parts.iter_mut() {
            if let mew_message::Part::Reasoning(rp) = part {
                if let Some(new_text) = self.reasoning_truncator.maybe_truncate(&rp.text) {
                    rp.text = new_text;
                    truncated_any = true;
                }
            }
        }
        truncated_any
    }

    /// Consume the truncator's "force tool call next" flag. Returns
    /// `true` if the next model request should set `tool_choice: required`.
    pub fn take_force_tool_choice(&mut self) -> bool {
        self.reasoning_truncator.take_force_tool_choice()
    }

    pub async fn load_messages(&self, messages: Vec<Message>) {
        *self.messages.lock().await = messages;
    }

    /// Return a clone of this agent's persisted session metadata, if any.
    pub async fn session_meta(&self) -> Option<mew_session::Meta> {
        if let Some(session) = &self.session {
            let guard = session.lock().await;
            Some(guard.meta().clone())
        } else {
            None
        }
    }

    pub async fn force_compact(&self) {
        *self.force_compact.lock().await = true;
    }

    /// Clear the in-memory conversation context and append a clear marker to
    /// the session log. The session file is the immutable event log and keeps
    /// everything; only what the model sees this turn is reset. Resume
    /// reconstructs forward from the marker.
    ///
    /// Permission caches are tied to the *session* lifetime (the JSONL log),
    /// not the *context* (what the model sees this turn), so they survive
    /// `/clear`. Both `permission_engine.session_allows` (the HashSet backing
    /// the `AllowSession` keypress in the permission modal) and
    /// `workspace_allowances` (the set of directories outside workspace
    /// roots the user has granted access to) persist. This is deliberate:
    /// a "session" is the JSONL log; a "context" is the visible turn.
    /// Clearing the latter doesn't invalidate the user's prior grants
    /// within the former. Pinned by
    /// `tests::test_clear_context_preserves_permission_caches`.
    pub async fn clear_context(&self) {
        self.messages.lock().await.clear();
        // Clear the classifier cache too — a fresh context should get
        // fresh classifier decisions, not stale ones from before the clear.
        self.clear_classifier_cache();

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
        self.run_with_parts(prompt, vec![], None)
    }

    /// Start a turn with an optional per-turn cancellation token. If no token
    /// is supplied, a fresh one is created. Shared-session callers pass a token
    /// so that cancelling one turn does not poison all future turns.
    pub fn run_with_parts(
        &self,
        prompt: String,
        attachments: Vec<Part>,
        cancel_token: Option<CancellationToken>,
    ) -> mpsc::Receiver<AgentEvent> {
        // Auto-flag the plan file as important if it exists. This guarantees
        // the plan survives context compaction without the model having to
        // remember to call flag_important every turn.
        if let Some(ref plan_path) = self.plan_path {
            let path = if plan_path.is_absolute() {
                plan_path.clone()
            } else {
                self.cwd.join(plan_path)
            };
            if path.exists() {
                let path = path.clone();
                let flagged = self.flagged_files.clone();
                tokio::spawn(async move {
                    let mut guard = flagged.lock().await;
                    let already = guard.iter().any(|f| f.path == path);
                    if !already {
                        guard.push(FlaggedFile {
                            path: path.clone(),
                            mode: mew_tools::tools::flag_important::FlagMode::Included,
                        });
                        tracing::debug!(plan = %path.display(), "auto-flagged plan file as important");
                    }
                });
            }
        }

        // Write a state snapshot to <session_dir>/mew.state.json for
        // external introspection. Spawned as a background task so it
        // doesn't block the turn loop.
        if let Some(ref session) = self.session {
            if let Ok(w) = session.try_lock() {
                let dir = w.path().parent().map(|p| p.to_path_buf());
                if let Some(dir) = dir {
                    let agent = self.clone();
                    tokio::spawn(async move {
                        agent.dump_state_to(&dir).await;
                    });
                }
            }
        }

        let (tx, rx) = mpsc::channel(256);
        let mut agent = self.clone();
        agent.cancel_token = cancel_token.unwrap_or_default();

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
        model: Option<&str>,
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
        let model = model.map(|s| s.to_string());
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
                .run(SubagentRunOptions {
                    def: &def,
                    prompt,
                    parent_call_id: call_id,
                    parent_session_id,
                    event_tx,
                    cancel,
                    model,
                })
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

    // -----------------------------------------------------------------
    // Background shell jobs
    // -----------------------------------------------------------------

    /// Launch a shell command in the background. Returns a job_id that can
    /// be used with `shell_job_status`, `shell_job_block`, and
    /// `cancel_shell_job`.
    pub async fn start_shell_job(&self, command: &str, cwd: &std::path::Path) -> String {
        let id = ulid::Ulid::new().to_string();
        let cancel = self.cancel_token.child_token();
        let output = Arc::new(tokio::sync::Mutex::new(String::new()));
        let state = Arc::new(tokio::sync::Mutex::new(ShellJobState::Running));
        let done = Arc::new(tokio::sync::Notify::new());

        let job = ShellJob {
            id: id.clone(),
            command: command.to_string(),
            started_at: chrono::Utc::now().timestamp_millis(),
            cancel: cancel.clone(),
            output: output.clone(),
            state: state.clone(),
            done: done.clone(),
        };

        // Spawn the background runner.
        let _job_id = id.clone();
        let cmd = command.to_string();
        let cwd = cwd.to_path_buf();
        tokio::spawn(async move {
            run_shell_job(&cmd, &cwd, &output, &state, &done, &cancel).await;
        });

        self.shell_jobs.lock().await.insert(id.clone(), job);
        id
    }

    /// Get the current state and accumulated output of a shell job.
    pub async fn shell_job_status(&self, job_id: &str) -> Option<(ShellJobState, String)> {
        let jobs = self.shell_jobs.lock().await;
        let job = jobs.get(job_id)?;
        let state = job.state.lock().await.clone();
        let output = job.output.lock().await.clone();
        Some((state, output))
    }

    /// Wait for a shell job to reach a terminal state (up to `timeout_secs`).
    /// Returns the final state and full output, or `None` if the job doesn't
    /// exist. If the timeout fires while still running, returns the current
    /// state (Running) and partial output.
    pub async fn shell_job_block(
        &self,
        job_id: &str,
        timeout_secs: u64,
    ) -> Option<(ShellJobState, String)> {
        let done = {
            let jobs = self.shell_jobs.lock().await;
            jobs.get(job_id)?.done.clone()
        };

        // Wait for the done signal or timeout.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            done.notified(),
        )
        .await;

        // Return whatever state we're in.
        self.shell_job_status(job_id).await
    }

    /// Cancel a running shell job by killing the process. Returns true if
    /// the job existed and was running.
    pub async fn cancel_shell_job(&self, job_id: &str) -> bool {
        let jobs = self.shell_jobs.lock().await;
        let Some(job) = jobs.get(job_id) else {
            return false;
        };
        let state = job.state.lock().await;
        if state.is_terminal() {
            return false;
        }
        drop(state);
        job.cancel.cancel();
        true
    }
}

/// Background runner for a shell job. Reads stdout/stderr into `output`,
/// updates `state` on exit, and notifies `done`.
async fn run_shell_job(
    command: &str,
    cwd: &std::path::Path,
    output: &Arc<tokio::sync::Mutex<String>>,
    state: &Arc<tokio::sync::Mutex<ShellJobState>>,
    done: &Arc<tokio::sync::Notify>,
    cancel: &tokio_util::sync::CancellationToken,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.current_dir(cwd);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            *state.lock().await = ShellJobState::Failed {
                reason: format!("spawn failed: {}", e),
            };
            done.notify_waiters();
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Reader tasks for stdout and stderr.
    if let Some(stdout) = stdout {
        let output = output.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let mut out = output.lock().await;
                out.push_str(&line);
                out.push('\n');
            }
        });
    }
    if let Some(stderr) = stderr {
        let output = output.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let mut out = output.lock().await;
                out.push_str(&line);
                out.push('\n');
            }
        });
    }

    // Wait for exit or cancellation.
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            *state.lock().await = ShellJobState::Cancelled;
        }
        status = child.wait() => {
            match status {
                Ok(s) => {
                    *state.lock().await = ShellJobState::Completed {
                        exit_code: s.code().unwrap_or(-1),
                    };
                }
                Err(e) => {
                    *state.lock().await = ShellJobState::Failed {
                        reason: format!("wait failed: {}", e),
                    };
                }
            }
        }
    }

    done.notify_waiters();
}

/// Build the `<available_skills>` block for the system prompt. **Moved to
/// `mew_prompts::skills::build_xml`.** Kept here as a thin re-export so the
/// existing call site at `rebuild_system()` keeps working.
fn build_skills_xml(skills: &[&mew_skills::Skill]) -> String {
    mew_prompts::skills::build_xml(skills)
}

/// Expand persona tool-list tag entries into actual tool names.
///
/// Supported tags:
/// - `tag!ALL` — every registered tool
/// - `tag!ALL_MCP` — every `mcp__*` tool
/// - `mcp__<server>` — every tool from the named MCP server (e.g. `mcp__filesystem`)
///
/// Plain tool names (e.g. `read`, `bash`) pass through unchanged. Tag
/// expansion is additive: the result is the union of all expanded tags
/// plus any plain names. Duplicates are deduplicated.
fn expand_tool_tags(
    tools: &[String],
    registry: &HashMap<String, Arc<dyn mew_tools::Tool>>,
) -> Vec<String> {
    let mut result: HashSet<String> = HashSet::new();
    let all_names: Vec<&String> = registry.keys().collect();

    for entry in tools {
        match entry.as_str() {
            "tag!ALL" => {
                for name in &all_names {
                    result.insert((*name).clone());
                }
            }
            "tag!ALL_MCP" => {
                for name in &all_names {
                    if name.starts_with("mcp__") {
                        result.insert((*name).clone());
                    }
                }
            }
            tag if tag.starts_with("mcp__") && tag.matches("__").count() == 1 => {
                // `mcp__<server>` — expand to all tools from that server.
                // Tool names are `mcp__<server>__<tool>`, so match the prefix.
                let prefix = format!("{}__", tag);
                for name in &all_names {
                    if name.starts_with(&prefix) {
                        result.insert((*name).clone());
                    }
                }
            }
            _ => {
                result.insert(entry.clone());
            }
        }
    }

    result.into_iter().collect()
}

/// Map a tool's `Sensitivity` to the label string the classifier prompt
/// expects. Kept here (next to `Agent::classify_permission`) so the
/// classifier call site doesn't have to know about `mew_tools` directly.
fn sensitivity_label(s: mew_tools::Sensitivity) -> &'static str {
    match s {
        mew_tools::Sensitivity::ReadOnly => "ReadOnly",
        mew_tools::Sensitivity::Mutating => "Mutating",
        mew_tools::Sensitivity::Dangerous => "Dangerous",
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
            metadata: None,
            file_delta: None,
        })
    }
}
