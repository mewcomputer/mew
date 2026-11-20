use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use mew_hooks::Dispatcher;
use mew_message::{SessionId, ToolState, TurnManifest};
use mew_provider::Provider;
use mew_subagents::{
    ChildGuidanceError, ModelResolver, SubagentDef, SubagentError, SubagentEvent, SubagentOutcome,
    SubagentResult, SubagentRunOptions, SubagentRunner,
};
use mew_tools::Tool;

use crate::child_guidance::ChildGuidance;

/// If the given tool state has completed successfully, return the output
/// string. Used by the runner to pull the final answer out of an `exit_tool`
/// call's completed state.
fn extract_completed_output(state: &ToolState) -> Option<String> {
    if let ToolState::Completed(s) = state {
        return Some(s.output.clone());
    }
    None
}

/// Extract per-turn manifests from a child agent's assistant messages.
/// Each assistant message may carry a `TurnManifest` on its `AssistantMeta`;
/// we collect all of them in order. Takes `&Agent` (shared reference) —
/// since `agent.messages` is `Arc<Mutex<...>>`, the borrow is cheap and
/// the agent remains owned by the caller.
/// Result of checking a subagent's final text against its output schema.
/// `InvalidSchema` means the def's schema itself won't compile — a def bug
/// that a corrective turn can never fix — while `InvalidOutput` is a fixable
/// output problem.
enum SchemaCheck {
    Valid,
    InvalidOutput(String),
    InvalidSchema(String),
}

/// Validate a subagent's final text against its output schema: the text must
/// parse as JSON (a surrounding ``` fence is tolerated) and satisfy the
/// schema. Returns the validation error(s) on failure.
fn validate_against_schema(schema: &serde_json::Value, text: &str) -> SchemaCheck {
    let candidate = strip_json_fence(text);
    let instance: serde_json::Value = match serde_json::from_str(candidate) {
        Ok(v) => v,
        Err(e) => return SchemaCheck::InvalidOutput(format!("output is not valid JSON: {e}")),
    };
    let validator = match jsonschema::validator_for(schema) {
        Ok(v) => v,
        Err(e) => {
            return SchemaCheck::InvalidSchema(format!("def's output_schema is invalid: {e}"));
        }
    };
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        SchemaCheck::Valid
    } else {
        SchemaCheck::InvalidOutput(errors.join("; "))
    }
}

/// Strip a single surrounding ``` fence (with optional `json` tag) so models
/// that insist on fencing still validate.
fn strip_json_fence(text: &str) -> &str {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            let inner = &rest[..end];
            let inner = inner
                .strip_prefix("json")
                .map(|s| s.trim_start_matches('\n'))
                .unwrap_or(inner);
            return inner.trim();
        }
    }
    t
}

async fn extract_manifests(agent: &crate::Agent) -> Vec<TurnManifest> {
    let messages = agent.messages.lock().await;
    messages
        .iter()
        .filter_map(|m| m.assistant.as_ref())
        .filter_map(|meta| meta.manifest.clone())
        .collect()
}

#[derive(Default)]
pub(crate) struct ActiveChildren {
    state: Mutex<ChildRegistry>,
}

type ChildKey = (SessionId, SessionId);

#[derive(Default)]
struct ChildRegistry {
    active: HashMap<ChildKey, Arc<ChildGuidance>>,
    // Keep unresolved receipts queryable after the child ends, including
    // a transcript write still draining after cancellation.
    retired: VecDeque<(ChildKey, Arc<ChildGuidance>)>,
}

struct ActiveChild<'a> {
    registry: &'a ActiveChildren,
    key: ChildKey,
}

impl Drop for ActiveChild<'_> {
    fn drop(&mut self) {
        let mut state = self.registry.state.lock().unwrap();
        let Some(guidance) = state.active.remove(&self.key) else {
            return;
        };
        if guidance.close() {
            const MAX_RETIRED_CHILDREN: usize = 32;
            if state.retired.len() == MAX_RETIRED_CHILDREN {
                state.retired.pop_front();
            }
            state.retired.push_back((self.key, guidance));
        }
    }
}

/// Simple subagent runner that spawns a child agent for each invocation.
#[derive(Clone)]
pub struct SimpleRunner {
    default_provider: Arc<dyn Provider>,
    default_provider_id: String,
    default_model_id: String,
    code_mode: mew_config::CodeModeConfig,
    codex_code_mode_models: Arc<std::collections::HashSet<String>>,
    secrets: Arc<mew_tools::SecretSet>,
    tools: HashMap<String, Arc<dyn Tool>>,
    dispatcher: Arc<dyn Dispatcher>,
    cwd: PathBuf,
    workspace_roots: Vec<PathBuf>,
    permission_engine: Option<Arc<mew_config::permissions::PermissionEngine>>,
    classifier_provider: Option<Arc<dyn Provider>>,
    classifier_model: Option<String>,
    /// Optional resolver for per-subagent model overrides. When a subagent def
    /// has `model: "provider/model"`, the runner asks the resolver for a
    /// provider for that model. If no resolver is configured, the override is
    /// ignored and the default provider is used.
    model_resolver: Option<Arc<dyn ModelResolver>>,
    /// Optional override for where subagent session files are written.
    /// `None` means use the global `mew_session::session_dir()`. Session
    /// builders preserve the parent writer's root here.
    session_root: Option<PathBuf>,
    /// Subagent definitions, handed to children that are allowed to spawn
    /// (`can_spawn: true` within the depth cap) so they can nest further.
    defs: Vec<SubagentDef>,
    /// Session's nesting cap, propagated to children that can spawn.
    max_subagent_depth: u32,
    /// Session's concurrency cap, propagated to children that can spawn.
    max_concurrent_subagents: u32,
    /// Fallback wall-clock cap for defs that don't set `max_duration_secs`.
    default_max_duration_secs: u64,
    active_children: Arc<ActiveChildren>,
}

impl SimpleRunner {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Vec<Arc<dyn Tool>>,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> Self {
        let tools_map = tools
            .into_iter()
            .map(|t| (t.name().to_string(), t))
            .collect();
        Self {
            default_provider: provider,
            default_provider_id: String::new(),
            default_model_id: String::new(),
            code_mode: mew_config::CodeModeConfig::default(),
            codex_code_mode_models: Arc::default(),
            secrets: Arc::new(mew_tools::SecretSet::default()),
            tools: tools_map,
            dispatcher,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            workspace_roots: Vec::new(),
            permission_engine: None,
            classifier_provider: None,
            classifier_model: None,
            model_resolver: None,
            session_root: None,
            defs: Vec::new(),
            max_subagent_depth: 1,
            max_concurrent_subagents: 4,
            default_max_duration_secs: mew_subagents::DEFAULT_MAX_DURATION_SECS,
            active_children: Arc::default(),
        }
    }

    /// Keep active children addressable when a parent replaces its runner.
    pub fn with_agent_registry(mut self, agent: &mut crate::Agent) -> Self {
        self.code_mode = agent.code_mode.clone();
        self.codex_code_mode_models = agent.codex_code_mode_models.clone();
        self.secrets = agent.secrets.clone();
        self.default_provider_id = agent.provider_id.clone();
        self.default_model_id = agent.model_id.clone();
        self.active_children = agent
            .child_guidance_registry
            .get_or_insert_with(|| self.active_children.clone())
            .clone();
        self
    }

    /// Builder method to attach a model resolver for per-subagent overrides.
    pub fn with_model_resolver(mut self, resolver: Arc<dyn ModelResolver>) -> Self {
        self.model_resolver = Some(resolver);
        self
    }

    /// Builder method to attach the subagent definitions and orchestration
    /// limits used when a child is allowed to spawn further subagents.
    pub fn with_spawn_policy(
        mut self,
        defs: Vec<SubagentDef>,
        max_subagent_depth: u32,
        max_concurrent_subagents: u32,
        default_max_duration_secs: u64,
    ) -> Self {
        self.defs = defs;
        self.max_subagent_depth = max_subagent_depth;
        self.max_concurrent_subagents = max_concurrent_subagents;
        self.default_max_duration_secs = default_max_duration_secs;
        self
    }

    /// Builder method to override the root directory for subagent session
    /// files, preserving the parent session's storage location.
    pub fn with_session_root(mut self, root: PathBuf) -> Self {
        self.session_root = Some(root);
        self
    }

    pub fn with_workspace(
        mut self,
        cwd: PathBuf,
        roots: Vec<PathBuf>,
        permission_engine: Option<Arc<mew_config::permissions::PermissionEngine>>,
    ) -> Self {
        self.cwd = cwd;
        self.workspace_roots = roots;
        self.permission_engine = permission_engine;
        self
    }

    pub fn with_classifier(
        mut self,
        provider: Option<Arc<dyn Provider>>,
        model: Option<String>,
    ) -> Self {
        self.classifier_provider = provider;
        self.classifier_model = model;
        self
    }

    /// Whether a child of `def` spawned from `parent_depth` may itself
    /// spawn subagents: the def must opt in, spawning from the child's
    /// depth must stay within the cap, and there must be defs to spawn.
    fn child_can_spawn(&self, def: &SubagentDef, parent_depth: u32) -> bool {
        def.can_spawn && parent_depth + 2 <= self.max_subagent_depth && !self.defs.is_empty()
    }

    /// Tool set for a child agent: the def's allowlist (or everything), with
    /// the spawn tools stripped unless the child may itself spawn.
    fn child_tools(&self, def: &SubagentDef, parent_depth: u32) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = if let Some(ref allowed) = def.tools {
            allowed
                .iter()
                .filter_map(|name| self.tools.get(name).cloned())
                .collect()
        } else {
            self.tools.values().cloned().collect()
        };
        if !self.child_can_spawn(def, parent_depth) {
            tools.retain(|t| t.name() != "subagent_start" && t.name() != "subagent_wait");
        }
        tools
    }

    /// Resolve which provider to use for this subagent invocation. A
    /// call-time `model` override beats the def's `model`; either beats the
    /// default provider. A literal value of `"micro"`, `"deci"`, or `"nano"`
    /// is resolved by the `ModelResolver` against the active router's tiers.
    async fn resolve_provider(
        &self,
        def: &SubagentDef,
        model: Option<&str>,
    ) -> (Arc<dyn Provider>, String, String) {
        let default = || {
            (
                self.default_provider.clone(),
                self.default_provider_id.clone(),
                self.default_model_id.clone(),
            )
        };
        let model = model.or(def.model.as_deref());

        let Some(model) = model else {
            return default();
        };

        let Some(ref resolver) = self.model_resolver else {
            tracing::warn!(
                subagent = %def.name,
                model = %model,
                "subagent has model override but no resolver configured; using default provider"
            );
            return default();
        };
        match resolver.resolve(model).await {
            Ok(p) => {
                let resolved = resolver.resolved_model(model);
                let (provider, model) = resolved
                    .split_once('/')
                    .unwrap_or((&self.default_provider_id, &resolved));
                (p, provider.to_owned(), model.to_owned())
            }
            Err(e) => {
                tracing::warn!(
                    subagent = %def.name,
                    model = %model,
                    error = %e,
                    "failed to resolve subagent model override; using default provider"
                );
                default()
            }
        }
    }
}

#[async_trait]
impl SubagentRunner for SimpleRunner {
    async fn guide_child(
        &self,
        parent_session_id: SessionId,
        child_session_id: SessionId,
        request_id: &str,
        text: String,
    ) -> Result<(), ChildGuidanceError> {
        if text.trim().is_empty() {
            return Err(ChildGuidanceError::Rejected(
                "Child guidance is empty.".into(),
            ));
        }
        if request_id.is_empty() || request_id.len() > 128 {
            return Err(ChildGuidanceError::Rejected(
                "Invalid child guidance request ID.".into(),
            ));
        }
        let guidance = self
            .active_children
            .state
            .lock()
            .unwrap()
            .active
            .get(&(parent_session_id, child_session_id))
            .cloned()
            .ok_or(ChildGuidanceError::NotRunning)?;
        guidance.guide(request_id, text).await
    }

    fn child_guidance_receipt(
        &self,
        parent_session_id: SessionId,
        child_session_id: SessionId,
        request_id: &str,
    ) -> Option<Result<(), ChildGuidanceError>> {
        let state = self.active_children.state.lock().unwrap();
        let key = (parent_session_id, child_session_id);
        state
            .active
            .get(&key)
            .or_else(|| {
                state
                    .retired
                    .iter()
                    .find_map(|(child_key, guidance)| (*child_key == key).then_some(guidance))
            })?
            .receipt(request_id)
    }

    async fn run(&self, opts: SubagentRunOptions<'_>) -> Result<SubagentResult, SubagentError> {
        let def = opts.def;
        let prompt = opts.prompt;
        let parent_session_id = opts.parent_session_id;
        let event_tx = opts.event_tx;
        let cancel = opts.cancel;
        let model = opts.model.as_deref();
        let session_id = SessionId::new();

        let child_can_spawn = self.child_can_spawn(def, opts.parent_depth);
        let tools = self.child_tools(def, opts.parent_depth);

        // Open a subagent session file nested under the parent. If this fails
        // the subagent still runs, but we surface the failure via
        // `session_unavailable` in the result so the user can see it instead
        // of silently getting a subagent with no transcript.
        let open_result = match &self.session_root {
            Some(root) => {
                mew_session::Writer::open_subagent_at(
                    root,
                    &parent_session_id.to_string(),
                    &session_id.to_string(),
                    &def.name,
                )
                .await
            }
            None => {
                mew_session::Writer::open_subagent(
                    &parent_session_id.to_string(),
                    &session_id.to_string(),
                    &def.name,
                )
                .await
            }
        };
        let (session_writer, session_unavailable) = match open_result {
            Ok(w) => (Some(w), false),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    parent_session_id = %parent_session_id,
                    subagent = %def.name,
                    "could not open subagent session; running without transcript"
                );
                (None, true)
            }
        };

        let (provider, provider_id, model_id) = self.resolve_provider(def, model).await;
        let mut agent = crate::Agent::new(
            provider,
            self.dispatcher.clone(),
            session_writer,
            tools,
            Some(session_id),
        );

        agent.code_mode = self.code_mode.clone();
        agent.active_tool_names = def
            .tools
            .as_ref()
            .map(|names| names.iter().cloned().collect());
        agent.codex_code_mode_models = self.codex_code_mode_models.clone();
        agent.secrets = self.secrets.clone();
        agent.set_model_info(&model_id, &provider_id);

        agent.cwd = self.cwd.clone();
        agent.workspace_roots = self.workspace_roots.clone();
        agent.permission_engine = self.permission_engine.clone();
        if let Some(provider) = &self.classifier_provider {
            agent.set_classifier_provider(provider.clone(), self.classifier_model.clone());
        }
        agent.set_shell_session(mew_tools::tools::shell_session::shared_session(
            self.cwd.clone(),
        ));

        // Wire nesting when the policy allows it: the child gets a runner of
        // its own, the defs it may invoke, and the session's limits.
        if child_can_spawn {
            agent.subagent_runner = Some(Arc::new(self.clone()));
            agent.subagent_defs = self.defs.clone();
            agent.max_subagent_depth = self.max_subagent_depth;
            agent.max_concurrent_subagents = self.max_concurrent_subagents;
        }

        // Compose the system prompt: the shared subagent base (base prompt +
        // subagent contract) always renders so every def inherits the
        // exit_tool contract regardless of its own template flag; the def's
        // body is appended and rendered only when `template: true`.
        let mut tool_names: Vec<String> = agent.tools.keys().cloned().collect();
        tool_names.sort_unstable();
        let ctx = mew_prompts::template::TemplateContext {
            subagent_name: def.name.clone(),
            model_id: agent.model_id.clone(),
            provider_id: agent.provider_id.clone(),
            session_id: session_id.to_string(),
            cwd: self.cwd.to_string_lossy().into_owned(),
            current_date: mew_prompts::template::TemplateContext::today(),
            tools: tool_names,
            supports_vision: agent.supports_vision,
            available_skills: mew_prompts::template::SkillInfo::from_skills(&agent.skills),
            ..Default::default()
        };
        let body = if def.template {
            mew_prompts::template::render(&def.body, &ctx)
        } else {
            def.body.clone()
        };
        let base = mew_prompts::vfs::read_builtin("system_prompts/subagent").unwrap_or("");
        let mut system = if base.is_empty() {
            String::new()
        } else {
            mew_prompts::template::render(base, &ctx)
        };
        if !body.is_empty() {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(&body);
        }
        if !system.is_empty() {
            agent.set_system(system);
        }

        // Set max_turns if specified, else apply the built-in default.
        // The turn count is the secondary safeguard; the wall-clock cap is
        // the primary one (catches stuck subagents in tight tool loops).
        let turn_limit = def.max_turns.unwrap_or(mew_subagents::DEFAULT_MAX_TURNS);
        agent.max_turns = Some(turn_limit);

        let max_duration_secs = def
            .max_duration_secs
            .unwrap_or(self.default_max_duration_secs);

        let key = (parent_session_id, session_id);
        let guidance = Arc::new(ChildGuidance::default());
        agent.child_guidance = Some(guidance.clone());
        self.active_children
            .state
            .lock()
            .unwrap()
            .active
            .insert(key, guidance);
        let active = ActiveChild {
            registry: &self.active_children,
            key,
        };

        let display_name = mew_subagents::pick_display_name(session_id.0);
        let _ = event_tx
            .send(SubagentEvent::Started {
                child_session_id: session_id.to_string(),
                display_name: Some(display_name.to_string()),
            })
            .await;

        // Cancel this child without cancelling its parent when its deadline expires.
        let child_cancel = cancel.child_token();
        let mut rx = agent.run_with_parts(prompt, vec![], Some(child_cancel.clone()));
        let mut result_text = String::new();
        let mut message_open = false;
        let mut text_parts = HashSet::new();
        let mut turns_used: u32 = 0;
        let mut last_error: Option<String> = None;
        let max_duration = std::time::Duration::from_secs(max_duration_secs);
        // Live copy of every tool-call part the subagent has emitted. We
        // track these so we can (a) parse streamed arguments into `input`
        // ourselves (mirroring the agent's reconcile) and (b) read the
        // input for `progress_update` parts at PartEnd to emit a
        // SubagentEvent::Progress.
        let mut tool_parts: std::collections::HashMap<
            mew_message::PartId,
            mew_message::ToolCallPart,
        > = std::collections::HashMap::new();
        // Capture the explicit exit result while draining the rest of the
        // turn so terminal state is persisted before reporting Finished.
        let mut exit_answer: Option<String> = None;
        // Output-schema enforcement: on the first invalid final output the
        // child gets exactly one corrective turn; a second failure returns
        // the raw text flagged schema_invalid.
        let mut correction_used = false;
        let mut schema_invalid = false;
        let mut schema_error: Option<String> = None;

        let finished = tokio::time::timeout(max_duration, async {
            'runs: loop {
                while let Some(event) = rx.recv().await {
                    match event {
                        crate::AgentEvent::ChildGuidanceContinuation => result_text.clear(),
                        crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartStart {
                            part,
                        }) => {
                            if !message_open {
                                result_text.clear();
                                text_parts.clear();
                                message_open = true;
                            }
                            match part {
                                mew_message::Part::Text(text) => {
                                    text_parts.insert(text.base.id);
                                    result_text.push_str(&text.text);
                                }
                                mew_message::Part::ToolCall(tc) => {
                                    tool_parts.insert(tc.base.id, tc.clone());
                                    let _ = event_tx
                                        .send(SubagentEvent::ToolStart {
                                            call_id: tc.call_id,
                                            tool_name: tc.tool_name,
                                        })
                                        .await;
                                }
                                _ => {}
                            }
                        }
                        crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartDelta {
                            part_id,
                            field: "arguments",
                            delta,
                        }) => {
                            if let Some(tc) = tool_parts.get_mut(&part_id) {
                                tc.raw_input.push_str(&delta);
                            }
                        }
                        crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartDelta {
                            part_id,
                            field,
                            delta,
                        }) if matches!(field, "text" | "") && text_parts.contains(&part_id) => {
                            result_text.push_str(&delta);
                            let _ = event_tx
                                .send(SubagentEvent::TextDelta { text: delta })
                                .await;
                        }
                        crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartEnd {
                            part_id,
                        }) => {
                            // Reconcile streamed arguments into `state.input` and
                            // surface `progress_update` messages.
                            if let Some(tc) = tool_parts.get_mut(&part_id) {
                                if !tc.raw_input.is_empty() {
                                    if let Ok(value) =
                                        serde_json::from_str::<serde_json::Value>(&tc.raw_input)
                                    {
                                        tc.state.set_input(value);
                                    }
                                }
                                if tc.tool_name == "progress_update" {
                                    let message = tc
                                        .state
                                        .input()
                                        .get("message")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if !message.is_empty() {
                                        let _ = event_tx
                                            .send(SubagentEvent::Progress {
                                                call_id: tc.call_id.clone(),
                                                tool_name: tc.tool_name.clone(),
                                                message,
                                            })
                                            .await;
                                    }
                                }
                            }
                        }
                        crate::AgentEvent::Provider(mew_provider::ProviderEvent::MessageEnd {
                            ..
                        }) => {
                            message_open = false;
                            turns_used += 1;
                        }
                        crate::AgentEvent::ToolEnd { call_id, success } => {
                            let _ = event_tx
                                .send(SubagentEvent::ToolEnd {
                                    call_id: call_id.clone(),
                                    success,
                                })
                                .await;
                        }
                        crate::AgentEvent::PartUpdated {
                            part: mew_message::Part::ToolCall(tc),
                            ..
                        } => {
                            if tc.tool_name == "progress_update"
                                && !tool_parts.contains_key(&tc.base.id)
                                && matches!(tc.state, mew_message::ToolState::Completed(_))
                            {
                                if let Some(message) = tc
                                    .state
                                    .input()
                                    .get("message")
                                    .and_then(|v| v.as_str())
                                    .filter(|message| !message.is_empty())
                                {
                                    let _ = event_tx
                                        .send(SubagentEvent::Progress {
                                            call_id: tc.call_id.clone(),
                                            tool_name: tc.tool_name.clone(),
                                            message: message.to_owned(),
                                        })
                                        .await;
                                }
                            }
                            // Keep our copy of the part in sync with the agent's.
                            // This catches the Completed state for exit_tool; for
                            // progress_update we already handled it at PartEnd.
                            tool_parts.insert(tc.base.id, tc.clone());
                            if tc.tool_name == "exit_tool" {
                                if let Some(answer) = extract_completed_output(&tc.state) {
                                    exit_answer = Some(answer);
                                }
                            }
                        }
                        crate::AgentEvent::AskUser {
                            call_id,
                            questions,
                            tx,
                        } => {
                            let _ = event_tx
                                .send(SubagentEvent::AskUser {
                                    source: mew_message::SubagentQuestionSource {
                                        parent_call_id: opts.parent_call_id.clone(),
                                        child_session_id: session_id.to_string(),
                                        name: def.name.clone(),
                                        display_name: Some(display_name.to_string()),
                                    },
                                    call_id,
                                    questions,
                                    tx,
                                })
                                .await;
                        }
                        crate::AgentEvent::SubagentAskUser {
                            source,
                            call_id,
                            questions,
                            tx,
                        } => {
                            let _ = event_tx
                                .send(SubagentEvent::AskUser {
                                    source,
                                    call_id,
                                    questions,
                                    tx,
                                })
                                .await;
                        }
                        crate::AgentEvent::WorkspacePermissionRequest { path, tx } => {
                            let _ = event_tx
                                .send(SubagentEvent::WorkspacePermissionRequest { path, tx })
                                .await;
                        }
                        crate::AgentEvent::PermissionRequest { call, tx }
                        | crate::AgentEvent::SubagentPermissionRequest { call, tx, .. } => {
                            let _ = event_tx
                                .send(SubagentEvent::PermissionRequest {
                                    tool_name: call.tool_name,
                                    call_id: call.call_id,
                                    input: call.input,
                                    tx,
                                })
                                .await;
                        }
                        crate::AgentEvent::Error(msg) => {
                            last_error = Some(msg);
                        }
                        _ => {}
                    }
                }

                if cancel.is_cancelled() {
                    break 'runs;
                }
                // The agent closes the stream after persisting terminal tool results.
                let effective_text = exit_answer.as_deref().unwrap_or(&result_text);
                let remaining_turns = turn_limit.saturating_sub(turns_used);
                let can_correct = !correction_used && last_error.is_none() && remaining_turns > 0;
                if let Some(schema) = &def.output_schema {
                    match validate_against_schema(schema, effective_text) {
                        SchemaCheck::Valid => {}
                        SchemaCheck::InvalidOutput(err) if can_correct => {
                            correction_used = true;
                            result_text.clear();
                            text_parts.clear();
                            message_open = false;
                            exit_answer = None;
                            let correction = format!(
                                "Your final output did not satisfy the required output schema.\n\
                                 Validation error: {err}\n\n\
                                 Reply with only a JSON value that validates against this \
                                 schema — no prose, no code fences:\n{}",
                                serde_json::to_string_pretty(schema).unwrap_or_default()
                            );
                            agent.max_turns = Some(remaining_turns);
                            rx = agent.run_with_parts(
                                correction,
                                vec![],
                                Some(child_cancel.clone()),
                            );
                            continue 'runs;
                        }
                        SchemaCheck::InvalidSchema(err) => {
                            // A broken def schema can never validate; flag it
                            // without burning a corrective turn.
                            schema_invalid = true;
                            schema_error = Some(err);
                        }
                        SchemaCheck::InvalidOutput(_) => {
                            schema_invalid = true;
                        }
                    }
                }
                break 'runs;
            }
        })
        .await;
        if finished.is_err() {
            child_cancel.cancel();
            // Consume buffered events while the agent saves its partial reply
            // and closes pending tools. A stuck dependency gets a bounded grace period.
            if tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while rx.recv().await.is_some() {}
            })
            .await
            .is_err()
            {
                tracing::warn!(child_session_id = %session_id, "subagent did not stop after its deadline");
            }
            last_error = Some(format!(
                "subagent exceeded max_duration_secs ({max_duration_secs}s)"
            ));
        }

        drop(active);

        if let Some(answer) = exit_answer {
            result_text = answer;
        }

        if cancel.is_cancelled() {
            let manifests = extract_manifests(&agent).await;
            let _ = event_tx
                .send(SubagentEvent::Finished {
                    child_session_id: session_id.to_string(),
                    outcome: SubagentOutcome::Cancelled,
                    manifests,
                })
                .await;
            return Ok(SubagentResult::Cancelled);
        }

        if let Some(reason) = last_error {
            let manifests = extract_manifests(&agent).await;
            let _ = event_tx
                .send(SubagentEvent::Finished {
                    child_session_id: session_id.to_string(),
                    outcome: SubagentOutcome::Failed {
                        reason: reason.clone(),
                    },
                    manifests,
                })
                .await;
            return Ok(SubagentResult::Error { reason });
        }

        let hit_turn_limit = turns_used >= turn_limit;

        // Apply the schema warning before the Finished event so both
        // observers of the run agree on the final output.
        if schema_invalid {
            match schema_error {
                Some(err) => result_text.insert_str(
                    0,
                    &format!(
                        "warning: schema_invalid: def's output_schema is invalid ({err}); \
                         returning raw text\n\n"
                    ),
                ),
                None => result_text.insert_str(
                    0,
                    "warning: schema_invalid: final output did not satisfy the required \
                     output schema after one corrective turn; returning raw text\n\n",
                ),
            }
        }

        let child_manifests = extract_manifests(&agent).await;

        let _ = event_tx
            .send(SubagentEvent::Finished {
                child_session_id: session_id.to_string(),
                outcome: SubagentOutcome::Completed,
                manifests: child_manifests.clone(),
            })
            .await;

        Ok(SubagentResult::Complete {
            text: result_text,
            turns_used,
            hit_turn_limit,
            hit_time_limit: false,
            session_unavailable,
            manifests: child_manifests,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the time/turn limit behavior of `SimpleRunner`.
    //!
    //! These exercise the wall-clock and turn-count caps independently and
    //! together. The runner is what enforces both — the agent's own
    //! `max_turns` is a secondary check; the wall-clock is the primary
    //! safeguard against a subagent in a tight tool loop.

    use super::*;
    use async_trait::async_trait;
    use futures::{stream, StreamExt};
    use mew_provider::{EventStream, ProviderError, ProviderEvent};
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::time::sleep;

    /// A scripted provider that yields each event in order, optionally
    /// sleeping before the next one. Lets the test control wall-clock pace
    /// of the agent's stream so the runner's time-based cap can fire
    /// deterministically.
    struct ScriptedProvider {
        script: StdMutex<Vec<(Duration, Vec<mew_provider::ProviderEvent>)>>,
        requests: StdMutex<Vec<mew_provider::Request>>,
    }

    impl ScriptedProvider {
        fn new(script: Vec<(Duration, Vec<mew_provider::ProviderEvent>)>) -> Self {
            Self {
                script: StdMutex::new(script),
                requests: StdMutex::new(Vec::new()),
            }
        }

        /// System prompts of every request made to this provider, in order.
        fn captured_systems(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|req| req.system.clone())
                .collect()
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn stream(&self, req: mew_provider::Request) -> Result<EventStream, ProviderError> {
            self.requests.lock().unwrap().push(req);
            // Atomically consume the script for this call. The first
            // `stream()` invocation gets the events; subsequent calls
            // (i.e. the agent's later turns) get an empty stream, so the
            // agent's `turn_loop` exits naturally after one turn. This
            // matches the shape tests need: drive a single turn with a
            // known script, then let the loop end.
            //
            // The previous behavior (replay the script on every call)
            // happened to work for single-turn tests but produced
            // surprising "stuck in a loop" failures for tests that emit
            // `SubagentEvent::Progress` etc. — the same call was emitted
            // once per replay.
            let script = {
                let mut guard = self.script.lock().unwrap();
                if guard.is_empty() {
                    Vec::new()
                } else {
                    std::mem::take(&mut *guard)
                }
            };
            let s = stream::unfold(script.into_iter(), |mut iter| async move {
                match iter.next() {
                    Some((delay, events)) => {
                        if !delay.is_zero() {
                            sleep(delay).await;
                        }
                        Some((stream::iter(events), iter))
                    }
                    None => None,
                }
            })
            .flatten();
            Ok(Box::pin(s))
        }
    }

    /// Build a single-turn text script: PartStart → text delta → PartEnd →
    /// MessageEnd. The text delta content is irrelevant for these tests.
    fn turn(label: &str) -> Vec<mew_provider::ProviderEvent> {
        let part_id = mew_message::PartId::new();
        let message_id = mew_message::MessageId::new();
        let session_id = mew_message::SessionId::new();
        vec![
            mew_provider::ProviderEvent::PartStart {
                part: mew_message::Part::Text(mew_message::TextPart {
                    base: mew_message::PartBase {
                        id: part_id,
                        message_id,
                        session_id,
                    },
                    text: String::new(),
                    synthetic: false,
                }),
            },
            mew_provider::ProviderEvent::PartDelta {
                part_id,
                field: "text",
                delta: label.to_string(),
            },
            mew_provider::ProviderEvent::PartEnd { part_id },
            mew_provider::ProviderEvent::MessageEnd {
                finish: mew_message::Finish::Stop,
                usage: mew_message::Tokens::default(),
                cost: 0.0,
            },
        ]
    }

    fn make_def(max_turns: Option<u32>, max_duration_secs: Option<u64>) -> SubagentDef {
        SubagentDef {
            name: "test-agent".into(),
            description: "test".into(),
            model: None,
            tools: None,
            max_turns,
            max_duration_secs,
            body: String::new(),
            path: PathBuf::from("(test)"),
            template: false,
            can_spawn: false,
            output_schema: None,
        }
    }

    /// Build a script that calls `exit_tool` with a given final answer in
    /// turn 0, followed by a turn-1 text response. The second turn should
    /// never be reached if `exit_tool` is wired correctly.
    fn exit_tool_script(call_id: &str, final_answer: &str) -> Vec<mew_provider::ProviderEvent> {
        let part_id = mew_message::PartId::new();
        let message_id = mew_message::MessageId::new();
        let session_id = mew_message::SessionId::new();
        let now = chrono::Utc::now().timestamp_millis();
        vec![
            mew_provider::ProviderEvent::PartStart {
                part: mew_message::Part::ToolCall(mew_message::ToolCallPart {
                    base: mew_message::PartBase {
                        id: part_id,
                        message_id,
                        session_id,
                    },
                    tool_name: "exit_tool".into(),
                    call_id: call_id.into(),
                    state: mew_message::ToolState::Pending(mew_message::ToolStatePending {
                        input: serde_json::json!({"final_answer": final_answer}),
                        time: mew_message::ToolTime {
                            start: now,
                            end: None,
                        },
                    }),
                    raw_input: String::new(),
                }),
            },
            mew_provider::ProviderEvent::PartEnd { part_id },
            mew_provider::ProviderEvent::MessageEnd {
                finish: mew_message::Finish::ToolUse,
                usage: mew_message::Tokens::default(),
                cost: 0.0,
            },
        ]
    }

    fn exit_tool() -> Arc<dyn mew_tools::Tool> {
        Arc::new(mew_tools::tools::exit_tool::ExitTool)
    }

    fn tool_turn(label: &str) -> Vec<mew_provider::ProviderEvent> {
        let mut events = turn(label);
        events.truncate(events.len() - 1);
        events.extend(mew_provider_fake::FakeProvider::tool_call(
            "echo",
            label,
            serde_json::json!({"input": label}),
        ));
        events
    }

    struct ControlledProvider {
        requests: tokio::sync::mpsc::Sender<mew_provider::Request>,
        responses: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<ProviderEvent>>>,
    }

    #[async_trait]
    impl Provider for ControlledProvider {
        fn name(&self) -> &str {
            "controlled"
        }

        async fn stream(
            &self,
            request: mew_provider::Request,
        ) -> Result<EventStream, ProviderError> {
            self.requests
                .send(request)
                .await
                .map_err(|error| ProviderError::Message(error.to_string()))?;
            let events =
                self.responses.lock().await.recv().await.ok_or_else(|| {
                    ProviderError::Message("fixture response stream closed".into())
                })?;
            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[tokio::test]
    async fn child_guidance_is_durable_at_its_next_request_and_retry_is_idempotent() {
        for (boundary, previous_answer) in [
            (tool_turn("continue"), "continue"),
            (turn("clean stop"), "clean stop"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let parent = SessionId::new();
            let (requests, mut incoming) = tokio::sync::mpsc::channel(2);
            let (responses, reply) = tokio::sync::mpsc::channel(2);
            let provider = Arc::new(ControlledProvider {
                requests,
                responses: tokio::sync::Mutex::new(reply),
            });
            let mut parent_agent = crate::Agent::new(
                provider.clone(),
                Arc::new(mew_hooks::NopDispatcher),
                None,
                vec![],
                Some(parent),
            );
            let runner = SimpleRunner::new(
                provider.clone(),
                vec![Arc::new(mew_tools::tools::echo::Echo)],
                Arc::new(mew_hooks::NopDispatcher),
            )
            .with_agent_registry(&mut parent_agent)
            .with_session_root(directory.path().to_path_buf());
            let def = make_def(Some(5), Some(30));
            let (event_tx, mut events) = tokio::sync::mpsc::channel(16);

            let child = runner.run(SubagentRunOptions {
                def: &def,
                prompt: "start task".into(),
                parent_call_id: "child_call".into(),
                parent_session_id: parent,
                event_tx,
                cancel: Default::default(),
                model: None,
                parent_depth: 0,
            });
            let parent_interaction = async {
                let Some(SubagentEvent::Started {
                    child_session_id, ..
                }) = events.recv().await
                else {
                    panic!("expected child start");
                };
                let child_id: SessionId = child_session_id.parse().unwrap();
                incoming.recv().await.expect("initial provider request");

                let wrong_parent = runner
                    .guide_child(
                        SessionId::new(),
                        child_id,
                        "wrong-parent",
                        "ignore me".into(),
                    )
                    .await;
                assert!(wrong_parent.is_err());
                assert!(runner
                    .guide_child(parent, child_id, "empty", " ".into())
                    .await
                    .is_err());

                let replacement =
                    SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
                        .with_agent_registry(&mut parent_agent);
                let mut guidance = Box::pin(replacement.guide_child(
                    parent,
                    child_id,
                    "guide-1",
                    "Keep the API stable.".into(),
                ));
                assert!(futures::poll!(&mut guidance).is_pending());
                let mut duplicate = Box::pin(runner.guide_child(
                    parent,
                    child_id,
                    "guide-1",
                    "Do not replace the original guidance".into(),
                ));
                assert!(futures::poll!(&mut duplicate).is_pending());
                assert!(runner
                    .guide_child(parent, child_id, "guide-2", "another guide".into())
                    .await
                    .is_err());
                let child_root = directory.path().join(parent.to_string()).join("subagents");
                assert!(!mew_session::Reader::has_prompt_receipt(
                    &child_root,
                    &child_session_id,
                    "guide-1"
                )
                .await
                .unwrap());

                responses.send(boundary).await.unwrap();
                let next = incoming.recv().await.expect("tool continuation");
                guidance.await.unwrap();
                duplicate.await.unwrap();
                assert!(runner
                    .guide_child(parent, child_id, "guide-1", "same request again".into())
                    .await
                    .is_ok());
                let last = next.messages.last().unwrap();
                assert!(last.parts.iter().any(|part| {
                    matches!(part, mew_message::Part::Text(text) if text.text == "Keep the API stable.")
                }));
                let raw = mew_session::Reader::load_subagent_from(
                    directory.path(),
                    &parent.to_string(),
                    &child_session_id,
                )
                .await
                .unwrap();
                assert_eq!(
                    raw.iter().filter(|message| message.id == last.id).count(),
                    1
                );
                assert!(raw.iter().flat_map(|message| &message.parts).any(|part| {
                    matches!(part, mew_message::Part::Text(text) if text.text == previous_answer)
                }));
                assert!(mew_session::Reader::has_prompt_receipt(
                    &child_root,
                    &child_session_id,
                    "guide-1"
                )
                .await
                .unwrap());

                responses.send(turn("finished")).await.unwrap();
                while events.recv().await.is_some() {}
                assert!(runner
                    .guide_child(parent, child_id, "late", "too late".into())
                    .await
                    .is_err());
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::join!(child, parent_interaction)
            })
            .await
            .unwrap();
            assert!(matches!(
                result.unwrap(),
                SubagentResult::Complete { turns_used: 2, text, .. } if text == "finished"
            ));
        }
    }

    #[tokio::test]
    async fn a_child_finishing_before_its_next_request_rejects_pending_guidance() {
        let directory = tempfile::tempdir().unwrap();
        let parent = SessionId::new();
        let (requests, mut incoming) = tokio::sync::mpsc::channel(2);
        let (responses, reply) = tokio::sync::mpsc::channel(2);
        let provider = Arc::new(ControlledProvider {
            requests,
            responses: tokio::sync::Mutex::new(reply),
        });
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
            .with_session_root(directory.path().to_path_buf());
        let def = make_def(Some(1), Some(30));
        let (event_tx, mut events) = tokio::sync::mpsc::channel(16);
        let child = runner.run(SubagentRunOptions {
            def: &def,
            prompt: "start task".into(),
            parent_call_id: "child_call".into(),
            parent_session_id: parent,
            event_tx,
            cancel: Default::default(),
            model: None,
            parent_depth: 0,
        });
        let parent_interaction = async {
            let Some(SubagentEvent::Started {
                child_session_id, ..
            }) = events.recv().await
            else {
                panic!("expected child start");
            };
            incoming.recv().await.expect("initial provider request");
            let mut guidance = Box::pin(runner.guide_child(
                parent,
                child_session_id.parse().unwrap(),
                "late-guide",
                "Keep this as a draft".into(),
            ));
            assert!(futures::poll!(&mut guidance).is_pending());
            responses.send(turn("finished")).await.unwrap();
            assert!(matches!(
                guidance.await, Err(ChildGuidanceError::Rejected(message)) if message.contains("ended before")
            ));
            let child_root = directory.path().join(parent.to_string()).join("subagents");
            assert!(!mew_session::Reader::has_prompt_receipt(
                &child_root,
                &child_session_id,
                "late-guide",
            )
            .await
            .unwrap());
            assert!(
                incoming.try_recv().is_err(),
                "guidance must not start another request"
            );
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(child, parent_interaction)
        })
        .await
        .unwrap();
        assert!(matches!(
            result.unwrap(),
            SubagentResult::Complete { turns_used: 1, .. }
        ));
    }

    #[tokio::test]
    async fn builtin_agents_use_configured_timeout_and_honor_per_agent_override() {
        let root = tempfile::tempdir().unwrap();
        for mut def in mew_subagents::Loader::builtin_defaults() {
            for (override_secs, expected_timeout) in [(None, true), (Some(60), false)] {
                if let Some(secs) = override_secs {
                    def.max_duration_secs = Some(secs);
                }
                let provider = Arc::new(ScriptedProvider::new(vec![(
                    Duration::from_millis(20),
                    turn("done"),
                )]));
                let runner =
                    SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
                        .with_session_root(root.path().to_path_buf())
                        .with_spawn_policy(vec![], 1, 4, 0);
                let (tx, mut rx) = tokio::sync::mpsc::channel(64);
                let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
                let result = runner
                    .run(SubagentRunOptions {
                        def: &def,
                        prompt: "prompt".into(),
                        parent_call_id: "call".into(),
                        parent_session_id: SessionId::new(),
                        event_tx: tx,
                        cancel: tokio_util::sync::CancellationToken::new(),
                        model: None,
                        parent_depth: 0,
                    })
                    .await
                    .unwrap();
                drain.abort();
                if expected_timeout {
                    assert!(
                        matches!(&result, SubagentResult::Error { reason } if reason.contains("max_duration_secs")),
                        "{} override={override_secs:?}: {result:?}",
                        def.name,
                    );
                } else {
                    assert!(
                        matches!(&result, SubagentResult::Complete { .. }),
                        "{} override={override_secs:?}: {result:?}",
                        def.name,
                    );
                }
            }
        }
    }

    #[test]
    fn configured_default_matches_runner_default() {
        assert_eq!(
            mew_config::OrchestrationConfig::default().default_max_duration_secs,
            600
        );
        assert_eq!(mew_subagents::DEFAULT_MAX_DURATION_SECS, 600);
    }

    #[tokio::test]
    async fn test_turn_limit_trips_before_time_limit() {
        // 5 turns available, generous time budget.
        let scripts = vec![
            tool_turn("t0"),
            tool_turn("t1"),
            turn("t2"),
            turn("t3"),
            turn("t4"),
        ];
        let def = make_def(Some(3), Some(60));
        let (result, provider) = run_provider_turns(scripts, def).await;
        assert_eq!(*provider.calls.lock().unwrap(), 3);
        match result {
            SubagentResult::Complete {
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                ..
            } => {
                assert_eq!(turns_used, 3);
                assert!(hit_turn_limit, "should have hit the turn cap");
                assert!(!hit_time_limit, "should not have hit the time cap");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn deadline_stops_a_stalled_stream_and_persists_partial_output_before_failure() {
        let directory = tempfile::tempdir().unwrap();
        let parent_id = SessionId::new();
        let mut early_events = turn("partial output");
        let terminal_events = early_events.split_off(2);
        let provider = Arc::new(ScriptedProvider::new(vec![
            (Duration::ZERO, early_events),
            (Duration::from_secs(30), terminal_events),
        ]));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
            .with_session_root(directory.path().to_path_buf());
        let def = make_def(Some(1000), Some(1));
        let (event_tx, mut events) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            runner.run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: parent_id,
                event_tx,
                cancel: cancel.clone(),
                model: None,
                parent_depth: 0,
            }),
        )
        .await
        .expect("the child deadline must expire before a provider completion event")
        .unwrap();

        assert!(
            !cancel.is_cancelled(),
            "child timeout must leave its parent running"
        );
        let SubagentResult::Error { reason } = result else {
            panic!("a timed-out child must fail, got {result:?}");
        };
        assert!(reason.contains("max_duration_secs"));
        let mut finished = None;
        while let Some(event) = events.recv().await {
            if let SubagentEvent::Finished {
                child_session_id,
                outcome,
                ..
            } = event
            {
                assert!(finished.replace((child_session_id, outcome)).is_none());
            }
        }
        let (child_id, outcome) = finished.expect("a terminal event");
        assert!(
            matches!(outcome, SubagentOutcome::Failed { reason: event_reason } if event_reason == reason)
        );
        let history = mew_session::Reader::load_subagent_from(
            directory.path(),
            &parent_id.to_string(),
            &child_id,
        )
        .await
        .unwrap();
        assert!(
            history.iter().any(|message| {
                message.assistant.as_ref().and_then(|meta| meta.error.as_ref())
                .is_some_and(|error| error.kind == mew_message::ErrorKind::Aborted)
            && message.parts.iter().any(|part| {
                matches!(part, mew_message::Part::Text(text) if text.text == "partial output")
            })
            }),
            "the cancelled partial response must be on disk before publishing the result"
        );
    }

    #[tokio::test]
    async fn test_neither_limit_trips_on_short_run() {
        // 2 turns, plenty of both budgets.
        let scripts = vec![tool_turn("t0"), turn("t1")];
        let def = make_def(Some(500), Some(60));
        let (result, _) = run_provider_turns(scripts, def).await;
        match result {
            SubagentResult::Complete {
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                text,
                ..
            } => {
                assert_eq!(turns_used, 2);
                assert!(!hit_turn_limit);
                assert!(!hit_time_limit);
                assert_eq!(text, "t1");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_defaults_apply_when_def_omits_both_caps() {
        // No caps on the def. The runner should apply built-in defaults
        // (500 turns, 600s) — neither will trip on a short script.
        let scripts = vec![tool_turn("t0"), turn("t1")];
        let def = make_def(None, None);
        let (result, _) = run_provider_turns(scripts, def).await;
        match result {
            SubagentResult::Complete {
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                ..
            } => {
                assert_eq!(turns_used, 2);
                assert!(!hit_turn_limit);
                assert!(!hit_time_limit);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    /// End-to-end: running a subagent through `SimpleRunner` should write
    /// the subagent's `session.jsonl` and `meta.json` to disk, AND update
    /// the parent's `meta.json` to register the child. This is the bug
    /// that hid the cause of `session 01KV48M8R0CAN7BXB1RG75EWCD` —
    /// the subagent ran but left no transcript.
    #[tokio::test]
    async fn test_subagent_writes_session_and_updates_parent_meta() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();

        // Set up the parent's session dir + meta by hand (avoids touching
        // the global session_dir).
        let parent_id = ulid::Ulid::new().to_string();
        let parent_session_id = ulid::Ulid::from_string(&parent_id).expect("valid ulid");
        let parent_dir = root.join(&parent_id);
        std::fs::create_dir_all(&parent_dir).expect("mkdir parent");
        let meta = mew_session::Meta::new(&parent_id);
        let meta_path = parent_dir.join("meta.json");
        std::fs::write(&meta_path, serde_json::to_vec_pretty(&meta).unwrap())
            .expect("write parent meta");

        let script: Vec<_> = (0..1)
            .map(|i| (Duration::from_millis(0), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(Some(10), Some(60));

        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
            .with_session_root(root.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id,
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .expect("runner ok");
        drain.abort();
        assert!(matches!(result, SubagentResult::Complete { .. }));

        // The subagent's session directory should exist with both files.
        let subagents_root = root.join(&parent_id).join("subagents");
        assert!(
            subagents_root.exists(),
            "subagents/ dir should exist under parent"
        );
        let mut child_dirs: Vec<_> = std::fs::read_dir(&subagents_root)
            .expect("read subagents")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            child_dirs.len(),
            1,
            "expected exactly one child dir, got {}",
            child_dirs.len()
        );
        let child_dir = child_dirs.pop().unwrap().path();
        assert!(
            child_dir.join("meta.json").exists(),
            "child meta.json missing"
        );
        assert!(
            child_dir.join("session.jsonl").exists(),
            "child session.jsonl missing"
        );

        // And the parent's meta.json should now reference the child.
        let parent_meta_path = root.join(&parent_id).join("meta.json");
        let parent_meta_bytes = std::fs::read(&parent_meta_path).expect("read parent meta");
        let parent_meta: mew_session::Meta =
            serde_json::from_slice(&parent_meta_bytes).expect("parse parent meta");
        assert_eq!(parent_meta.children_session_ids.len(), 1);
        let child_id = child_dir.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(parent_meta.children_session_ids[0], child_id);
    }

    /// When the subagent session file cannot be opened, the runner should
    /// still complete the run and surface `session_unavailable = true` in
    /// the result so the user sees a "transcript could not be written"
    /// warning rather than silently losing the transcript.
    #[tokio::test]
    async fn test_session_unavailable_when_cannot_open() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Place a regular file at <tmp>/sessions, so the runner's
        // `create_dir_all(<root>/<parent_id>)` will fail because the
        // parent component is not a directory.
        let blocker = tmp.path().join("sessions");
        std::fs::write(&blocker, b"not a directory").expect("write blocker");

        let script: Vec<_> = (0..1)
            .map(|i| (Duration::from_millis(0), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(Some(10), Some(60));
        let parent_session_id = ulid::Ulid::new();

        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
            .with_session_root(blocker.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id,
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .expect("runner ok");
        drain.abort();

        match result {
            SubagentResult::Complete {
                session_unavailable,
                ..
            } => {
                assert!(
                    session_unavailable,
                    "session_unavailable should be true when open fails"
                );
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    /// When the subagent calls `exit_tool(final_answer)` the runner should
    /// break the loop, set `result_text` to the final answer, and not
    /// continue with subsequent turns.
    #[tokio::test]
    async fn test_exit_tool_short_circuits_with_final_answer() {
        let script = vec![(
            Duration::from_millis(0),
            exit_tool_script("c1", "the answer is 42"),
        )];
        let def = make_def(Some(10), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(
            provider,
            vec![exit_tool()],
            Arc::new(mew_hooks::NopDispatcher),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.abort();

        match result {
            SubagentResult::Complete {
                text,
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                ..
            } => {
                assert_eq!(text, "the answer is 42");
                assert_eq!(turns_used, 1, "should have stopped after one turn");
                assert!(!hit_turn_limit);
                assert!(!hit_time_limit);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn code_mode_subagent_returns_the_nested_exit_result() {
        check_code_mode_subagent(false).await;
        check_code_mode_subagent(true).await;
    }

    #[tokio::test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    async fn tool_restricted_subagent_isolates_python() {
        let directory = tempfile::tempdir().unwrap();
        let secret = directory.path().join("private.txt");
        std::fs::write(&secret, "private fixture").unwrap();
        let source = format!("try:\n result=open({:?}).read()\nexcept OSError:\n result='denied'\ntools.exit_tool({{'final_answer':result}})", secret.to_string_lossy());
        let provider = Arc::new(ScriptedProvider::new(vec![(
            Duration::ZERO,
            mew_provider_fake::FakeProvider::tool_call(
                "python",
                "py",
                serde_json::json!({"source":source}),
            ),
        )]));
        let tools = vec![exit_tool()];
        let mut parent = crate::Agent::new(
            provider.clone(),
            Arc::new(mew_hooks::NopDispatcher),
            None,
            tools.clone(),
            None,
        );
        parent.code_mode.mode = Some(mew_config::CodeMode::Full);
        parent.code_mode.isolated = false;
        let runner = SimpleRunner::new(provider, tools, Arc::new(mew_hooks::NopDispatcher))
            .with_agent_registry(&mut parent)
            .with_session_root(directory.path().to_path_buf());
        let def = SubagentDef {
            tools: Some(vec!["exit_tool".into()]),
            ..make_def(Some(10), Some(60))
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let SubagentEvent::PermissionRequest { tx, .. } = event {
                    let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
                }
            }
        });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "parent".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel: Default::default(),
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.await.unwrap();
        let SubagentResult::Complete { text, .. } = result else {
            panic!("expected completed child");
        };
        assert_eq!(text, "denied");
    }

    #[tokio::test]
    async fn code_mode_subagent_forwards_progress_to_parent() {
        let directory = tempfile::tempdir().unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![(
            Duration::ZERO,
            mew_provider_fake::FakeProvider::tool_call(
                "python",
                "py",
                serde_json::json!({"source":"tools.progress_update({'message':'scanning files'})\ntools.exit_tool({'final_answer':'done'})"}),
            ),
        )]));
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(mew_tools::tools::progress_update::ProgressUpdate),
            exit_tool(),
        ];
        let mut parent = crate::Agent::new(
            provider.clone(),
            Arc::new(mew_hooks::NopDispatcher),
            None,
            tools.clone(),
            None,
        );
        parent.code_mode.mode = Some(mew_config::CodeMode::Full);
        parent.code_mode.isolated = false;
        let runner = SimpleRunner::new(provider, tools, Arc::new(mew_hooks::NopDispatcher))
            .with_agent_registry(&mut parent)
            .with_session_root(directory.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move {
            let mut messages = Vec::new();
            while let Some(event) = rx.recv().await {
                match event {
                    SubagentEvent::PermissionRequest { tx, .. } => {
                        let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
                    }
                    SubagentEvent::Progress { message, .. } => messages.push(message),
                    _ => {}
                }
            }
            messages
        });
        runner
            .run(SubagentRunOptions {
                def: &make_def(Some(10), Some(60)),
                prompt: "prompt".into(),
                parent_call_id: "parent".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel: Default::default(),
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        assert_eq!(drain.await.unwrap(), ["scanning files"]);
    }

    #[tokio::test]
    async fn subagent_tool_results_redact_parent_secrets() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("data.txt"), "token=secret-value").unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![(
            Duration::ZERO,
            mew_provider_fake::FakeProvider::tool_call(
                "python",
                "py",
                serde_json::json!({"source":"r = tools.read({'path':'data.txt'})\ntools.exit_tool({'final_answer':str(r)})"}),
            ),
        )]));
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(mew_tools::tools::read::Read), exit_tool()];
        let mut parent = crate::Agent::new(
            provider.clone(),
            Arc::new(mew_hooks::NopDispatcher),
            None,
            tools.clone(),
            None,
        );
        parent.code_mode.mode = Some(mew_config::CodeMode::Full);
        parent.code_mode.isolated = false;
        parent.secrets = Arc::new(mew_tools::SecretSet {
            words: vec!["secret-value".into()],
            globs: vec![],
        });
        let runner = SimpleRunner::new(provider, tools, Arc::new(mew_hooks::NopDispatcher))
            .with_agent_registry(&mut parent)
            .with_workspace(
                directory.path().to_path_buf(),
                vec![directory.path().to_path_buf()],
                None,
            )
            .with_session_root(directory.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let SubagentEvent::PermissionRequest { tx, .. } = event {
                    let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
                }
            }
        });
        let result = runner
            .run(SubagentRunOptions {
                def: &make_def(Some(10), Some(60)),
                prompt: "prompt".into(),
                parent_call_id: "parent".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel: Default::default(),
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.await.unwrap();
        let SubagentResult::Complete { text, .. } = result else {
            panic!("expected completed child");
        };
        assert!(text.contains("[REDACTED]"), "{text}");
        assert!(!text.contains("secret-value"), "{text}");
    }

    async fn check_code_mode_subagent(from_catalog: bool) {
        let directory = tempfile::tempdir().unwrap();
        let unwanted = directory.path().join("after-exit");
        let source = format!("tools.exit_tool({{'final_answer':'the answer is 42'}})\nopen({:?}, 'w').write('wrong')", unwanted.to_string_lossy());
        let script = vec![(
            Duration::from_millis(0),
            mew_provider_fake::FakeProvider::tool_call(
                "python",
                "py",
                serde_json::json!({"source":source}),
            ),
        )];
        let def = make_def(Some(10), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let mut parent = crate::Agent::new(
            provider.clone(),
            Arc::new(mew_hooks::NopDispatcher),
            None,
            vec![exit_tool()],
            None,
        );
        parent.code_mode.isolated = false;
        parent.set_model_info("gpt-6-luna", "codex");
        if from_catalog {
            parent.codex_code_mode_models =
                Arc::new(std::collections::HashSet::from(["gpt-6-luna".to_string()]));
        } else {
            parent.code_mode.mode = Some(mew_config::CodeMode::Full);
        }
        let runner = SimpleRunner::new(
            provider,
            vec![exit_tool()],
            Arc::new(mew_hooks::NopDispatcher),
        )
        .with_agent_registry(&mut parent)
        .with_session_root(directory.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let drain = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let mew_subagents::SubagentEvent::PermissionRequest { tx, .. } = event {
                    let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
                }
            }
        });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "parent".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel: tokio_util::sync::CancellationToken::new(),
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.abort();
        assert!(!unwanted.exists(), "code after exit_tool must not run");
        match result {
            SubagentResult::Complete {
                text, turns_used, ..
            } => {
                assert_eq!(text, "the answer is 42");
                assert_eq!(turns_used, 1);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    /// Every subagent system prompt is the shared subagent base (base prompt
    /// + subagent contract) composed with the def's own body. The base always
    /// renders through minijinja; the body honors the def's `template` flag.
    #[tokio::test]
    async fn test_system_prompt_composes_subagent_base_and_body() {
        let script = vec![(Duration::from_millis(0), turn("done"))];
        let mut def = make_def(Some(10), Some(60));
        def.body = "ROLE_MARKER investigate the thing".into();
        def.template = true;
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner =
            SimpleRunner::new(provider.clone(), vec![], Arc::new(mew_hooks::NopDispatcher));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.abort();

        let systems = provider.captured_systems();
        assert!(!systems.is_empty(), "provider should have seen a request");
        let system = &systems[0];
        assert!(
            system.contains("focused mew subagent named test-agent"),
            "system prompt must include the rendered subagent base, got: {system}"
        );
        assert!(
            system.contains("ROLE_MARKER investigate the thing"),
            "system prompt must include the def body"
        );
        assert!(
            system.contains("The parent receives only that message")
                && system.contains("including any requested JSON"),
            "the fallback contract must require complete final results"
        );
        assert!(
            !system.contains("{{ "),
            "base must render, not stay raw template syntax"
        );
    }

    /// A def with `template: false` keeps its body verbatim (template syntax
    /// preserved), but the shared subagent base around it still renders.
    #[tokio::test]
    async fn test_template_false_body_verbatim_base_still_renders() {
        let script = vec![(Duration::from_millis(0), turn("done"))];
        let mut def = make_def(Some(10), Some(60));
        def.body = "Raw {{ subagent_name }} instructions".into();
        def.template = false;
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner =
            SimpleRunner::new(provider.clone(), vec![], Arc::new(mew_hooks::NopDispatcher));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.abort();

        let systems = provider.captured_systems();
        let system = &systems[0];
        assert!(
            system.contains("Raw {{ subagent_name }} instructions"),
            "non-templated body must stay verbatim, got: {system}"
        );
        assert!(
            system.contains("focused mew subagent named test-agent"),
            "shared subagent base must render even for non-templated defs"
        );
    }

    /// A def with an empty body still gets the subagent base as its system
    /// prompt: the exit_tool contract and base prompt are structural, not
    /// something a body-less def should lose.
    #[tokio::test]
    async fn test_empty_body_still_gets_subagent_base() {
        let script = vec![(Duration::from_millis(0), turn("done"))];
        let def = make_def(Some(10), Some(60));
        assert!(def.body.is_empty());
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner =
            SimpleRunner::new(provider.clone(), vec![], Arc::new(mew_hooks::NopDispatcher));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.abort();

        let systems = provider.captured_systems();
        let system = &systems[0];
        assert!(
            system.contains("focused mew subagent named test-agent"),
            "empty-bodied def must still get the subagent base, got: {system}"
        );
    }

    /// The runner should forward every tool call as a `SubagentEvent::ToolStart`
    /// so the parent (and any UI watching the channel) can see what the
    /// subagent is doing. Previously this was unwired — the parent's
    /// `ToolStart`/`ToolEnd` pump received events that the runner never
    /// actually sent. This test guards the wiring for a tool that does
    /// not short-circuit the run (progress_update).
    #[tokio::test]
    async fn test_subagent_tool_start_end_events_emitted() {
        let progress_part_id = mew_message::PartId::new();
        let progress_mid = mew_message::MessageId::new();
        let progress_sid = mew_message::SessionId::new();
        let now = chrono::Utc::now().timestamp_millis();

        let script: Vec<(Duration, Vec<mew_provider::ProviderEvent>)> = vec![(
            Duration::from_millis(0),
            vec![
                mew_provider::ProviderEvent::PartStart {
                    part: mew_message::Part::ToolCall(mew_message::ToolCallPart {
                        base: mew_message::PartBase {
                            id: progress_part_id,
                            message_id: progress_mid,
                            session_id: progress_sid,
                        },
                        tool_name: "progress_update".into(),
                        call_id: "p1".into(),
                        state: mew_message::ToolState::Pending(mew_message::ToolStatePending {
                            input: serde_json::json!({"message": "starting work"}),
                            time: mew_message::ToolTime {
                                start: now,
                                end: None,
                            },
                        }),
                        raw_input: String::new(),
                    }),
                },
                mew_provider::ProviderEvent::PartEnd {
                    part_id: progress_part_id,
                },
                mew_provider::ProviderEvent::MessageEnd {
                    finish: mew_message::Finish::ToolUse,
                    usage: mew_message::Tokens::default(),
                    cost: 0.0,
                },
            ],
        )];
        let def = make_def(Some(10), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(
            provider,
            vec![Arc::new(mew_tools::tools::progress_update::ProgressUpdate)],
            Arc::new(mew_hooks::NopDispatcher),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move {
            let mut events: Vec<SubagentEvent> = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drop(result);
        let events = drain.await.unwrap();

        let tool_starts: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::ToolStart { call_id, tool_name } => {
                    Some((call_id.clone(), tool_name.clone()))
                }
                _ => None,
            })
            .collect();
        let tool_ends: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::ToolEnd { call_id, success } => Some((call_id.clone(), *success)),
                _ => None,
            })
            .collect();

        assert!(
            tool_starts
                .iter()
                .any(|(_, name)| name == "progress_update"),
            "expected a ToolStart for progress_update, got {tool_starts:?}"
        );
        assert!(
            tool_ends.iter().any(|(id, _)| id == "p1"),
            "expected a ToolEnd for p1, got {tool_ends:?}"
        );
    }

    /// When the subagent calls `progress_update(message: "...")`, the
    /// runner should emit a `SubagentEvent::Progress` carrying that
    /// message so the parent (and the TUI's sidebar) can show what the
    /// subagent is working on.
    #[tokio::test]
    async fn test_progress_update_emits_subagent_progress_event() {
        let progress_part_id = mew_message::PartId::new();
        let progress_mid = mew_message::MessageId::new();
        let progress_sid = mew_message::SessionId::new();
        let now = chrono::Utc::now().timestamp_millis();

        let script: Vec<(Duration, Vec<mew_provider::ProviderEvent>)> = vec![(
            Duration::from_millis(0),
            vec![
                mew_provider::ProviderEvent::PartStart {
                    part: mew_message::Part::ToolCall(mew_message::ToolCallPart {
                        base: mew_message::PartBase {
                            id: progress_part_id,
                            message_id: progress_mid,
                            session_id: progress_sid,
                        },
                        tool_name: "progress_update".into(),
                        call_id: "p1".into(),
                        state: mew_message::ToolState::Pending(mew_message::ToolStatePending {
                            input: serde_json::json!({"message": "scanning the repo"}),
                            time: mew_message::ToolTime {
                                start: now,
                                end: None,
                            },
                        }),
                        raw_input: String::new(),
                    }),
                },
                mew_provider::ProviderEvent::PartEnd {
                    part_id: progress_part_id,
                },
                mew_provider::ProviderEvent::MessageEnd {
                    finish: mew_message::Finish::ToolUse,
                    usage: mew_message::Tokens::default(),
                    cost: 0.0,
                },
            ],
        )];
        let def = make_def(Some(10), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(
            provider,
            vec![Arc::new(mew_tools::tools::progress_update::ProgressUpdate)],
            Arc::new(mew_hooks::NopDispatcher),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move {
            let mut events: Vec<SubagentEvent> = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drop(result);
        let events = drain.await.unwrap();

        let progress: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::Progress { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            progress,
            vec!["scanning the repo".to_string()],
            "expected one Progress event with the message"
        );
    }

    /// The runner should pick a human-friendly display name from
    /// `mew_subagents::DISPLAY_NAMES` and emit it on `SubagentEvent::Started`.
    /// The same def can be spawned multiple times; each run should get its
    /// own (potentially-colliding) name.
    #[tokio::test]
    async fn test_started_event_includes_display_name() {
        let script = vec![(Duration::from_millis(0), turn("hi"))];
        let def = make_def(Some(2), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move {
            let mut events: Vec<SubagentEvent> = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        });
        let _ = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await;
        let events = drain.await.unwrap();

        let started: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::Started {
                    child_session_id,
                    display_name,
                } => Some((child_session_id.clone(), display_name.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(started.len(), 1, "expected exactly one Started event");
        let (_, display_name) = &started[0];
        let name = display_name
            .as_deref()
            .expect("Started should carry a display_name");
        assert!(
            mew_subagents::DISPLAY_NAMES.contains(&name),
            "display_name {name:?} should be from DISPLAY_NAMES"
        );
    }

    // --------------------------------------------------------------
    // Spawn-policy tests (can_spawn + depth cap)
    // --------------------------------------------------------------

    /// A runner whose tool map includes the spawn tools, so tests can
    /// observe whether a given child keeps or loses them.
    fn policy_runner(defs: Vec<SubagentDef>, max_depth: u32) -> SimpleRunner {
        let defs_arc = Arc::new(defs.clone());
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(mew_tools::tools::subagent_start::SubagentStart::new(
                defs_arc,
            )),
            Arc::new(mew_tools::tools::subagent_wait::SubagentWait::new()),
            exit_tool(),
        ];
        let provider = Arc::new(ScriptedProvider::new(vec![]));
        SimpleRunner::new(provider, tools, Arc::new(mew_hooks::NopDispatcher))
            .with_spawn_policy(defs, max_depth, 4, 300)
    }

    fn tool_names(tools: Vec<Arc<dyn Tool>>) -> Vec<String> {
        let mut names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn child_tools_strip_spawn_tools_when_def_does_not_opt_in() {
        let def = make_def(None, None); // can_spawn: false
        let runner = policy_runner(vec![def.clone()], 3);
        let names = tool_names(runner.child_tools(&def, 0));
        assert!(!names.contains(&"subagent_start".to_string()));
        assert!(!names.contains(&"subagent_wait".to_string()));
        assert!(names.contains(&"exit_tool".to_string()));
    }

    #[test]
    fn child_tools_keep_spawn_tools_when_opted_in_and_within_depth() {
        let def = SubagentDef {
            can_spawn: true,
            ..make_def(None, None)
        };
        let runner = policy_runner(vec![def.clone()], 3);
        let names = tool_names(runner.child_tools(&def, 0));
        assert!(names.contains(&"subagent_start".to_string()));
        assert!(names.contains(&"subagent_wait".to_string()));
    }

    #[test]
    fn child_tools_strip_spawn_tools_at_depth_cap() {
        let def = SubagentDef {
            can_spawn: true,
            ..make_def(None, None)
        };
        let runner = policy_runner(vec![def.clone()], 2);
        // Child of a depth-1 session sits at depth 2 == cap; it cannot spawn.
        let names = tool_names(runner.child_tools(&def, 1));
        assert!(!names.contains(&"subagent_start".to_string()));
        // ...but a depth-0 session's child (depth 1 < cap 2) can.
        let names = tool_names(runner.child_tools(&def, 0));
        assert!(names.contains(&"subagent_start".to_string()));
    }

    #[test]
    fn child_tools_strip_spawn_tools_when_no_defs_loaded() {
        let def = SubagentDef {
            can_spawn: true,
            ..make_def(None, None)
        };
        let runner = policy_runner(vec![], 3);
        let names = tool_names(runner.child_tools(&def, 0));
        assert!(!names.contains(&"subagent_start".to_string()));
    }

    #[test]
    fn child_tools_respect_def_allowlist_over_spawn_policy() {
        // Even with can_spawn, an explicit allowlist that omits the spawn
        // tools wins.
        let def = SubagentDef {
            can_spawn: true,
            tools: Some(vec!["exit_tool".into()]),
            ..make_def(None, None)
        };
        let runner = policy_runner(vec![def.clone()], 3);
        let names = tool_names(runner.child_tools(&def, 0));
        assert_eq!(names, vec!["exit_tool".to_string()]);
    }

    // --------------------------------------------------------------
    // Output-schema tests
    // --------------------------------------------------------------

    /// A provider that pops one script per `stream()` call, so tests can
    /// script the initial run AND the corrective turn. Counts calls.
    struct SequentialProvider {
        scripts: StdMutex<Vec<Vec<mew_provider::ProviderEvent>>>,
        calls: StdMutex<usize>,
    }

    impl SequentialProvider {
        fn new(scripts: Vec<Vec<mew_provider::ProviderEvent>>) -> Self {
            Self {
                scripts: StdMutex::new(scripts),
                calls: StdMutex::new(0),
            }
        }
    }

    #[async_trait]
    impl Provider for SequentialProvider {
        fn name(&self) -> &str {
            "sequential"
        }

        async fn stream(&self, _req: mew_provider::Request) -> Result<EventStream, ProviderError> {
            *self.calls.lock().unwrap() += 1;
            let script = {
                let mut guard = self.scripts.lock().unwrap();
                if guard.is_empty() {
                    Vec::new()
                } else {
                    guard.remove(0)
                }
            };
            Ok(Box::pin(stream::iter(script)))
        }
    }

    fn typed_def(schema: serde_json::Value) -> SubagentDef {
        SubagentDef {
            output_schema: Some(schema),
            ..make_def(Some(10), Some(60))
        }
    }

    async fn run_provider_turns(
        scripts: Vec<Vec<mew_provider::ProviderEvent>>,
        def: SubagentDef,
    ) -> (SubagentResult, Arc<SequentialProvider>) {
        let provider = Arc::new(SequentialProvider::new(scripts));
        // These fixtures exercise outcomes rather than transcript storage. Use a
        // non-directory session root to keep the full turn-cap run in memory.
        let no_transcripts = tempfile::NamedTempFile::new().unwrap();
        let runner = SimpleRunner::new(
            provider.clone(),
            vec![
                Arc::new(mew_tools::tools::echo::Echo),
                Arc::new(mew_tools::tools::progress_update::ProgressUpdate),
                exit_tool(),
            ],
            Arc::new(mew_hooks::NopDispatcher),
        )
        .with_session_root(no_transcripts.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
                parent_depth: 0,
            })
            .await
            .unwrap();
        drain.abort();
        (result, provider)
    }

    #[tokio::test]
    async fn final_json_is_validated_without_earlier_narration() {
        let mut progress = turn("checking the data");
        progress.pop();
        progress.extend(mew_provider_fake::FakeProvider::tool_call(
            "progress_update",
            "progress",
            serde_json::json!({"message": "checking"}),
        ));
        let def = typed_def(serde_json::json!({"type": "object", "required": ["answer"]}));
        let (result, provider) =
            run_provider_turns(vec![progress, turn("{\"answer\":42}")], def).await;
        assert!(
            matches!(result, SubagentResult::Complete { text, .. } if text == "{\"answer\":42}")
        );
        assert_eq!(
            *provider.calls.lock().unwrap(),
            2,
            "valid final JSON must not trigger a correction"
        );
    }

    #[tokio::test]
    async fn exit_tool_returns_its_json_without_narration() {
        let mut progress = turn("checking the data");
        progress.pop();
        progress.extend(mew_provider_fake::FakeProvider::tool_call(
            "progress_update",
            "progress",
            serde_json::json!({"message": "checking"}),
        ));
        let mut finishing = turn("ready to report");
        finishing.pop();
        finishing.extend(exit_tool_script("exit", "{\"answer\":42}"));
        let def = typed_def(serde_json::json!({"type": "object", "required": ["answer"]}));
        let (result, provider) = run_provider_turns(vec![progress, finishing], def).await;
        assert!(
            matches!(result, SubagentResult::Complete { text, .. } if text == "{\"answer\":42}")
        );
        assert_eq!(*provider.calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn final_message_includes_every_text_part_and_initial_content() {
        let mut first = turn(" first");
        if let mew_provider::ProviderEvent::PartStart {
            part: mew_message::Part::Text(text),
        } = &mut first[0]
        {
            text.text = "initial".into();
        } else {
            panic!("expected a text part");
        }
        first.pop();
        let mut second = turn(" second");
        if let mew_provider::ProviderEvent::PartDelta { field, .. } = &mut second[1] {
            *field = "";
        }
        first.extend(second);
        let (result, _) = run_provider_turns(vec![first], make_def(Some(10), Some(60))).await;
        assert!(
            matches!(result, SubagentResult::Complete { text, .. } if text == "initial first second")
        );
    }

    #[tokio::test]
    async fn final_reasoning_is_not_returned_as_answer() {
        let reasoning_id = mew_message::PartId::new();
        let mut events = vec![
            mew_provider::ProviderEvent::PartStart {
                part: mew_message::Part::Reasoning(mew_message::ReasoningPart {
                    base: mew_message::PartBase {
                        id: reasoning_id,
                        message_id: mew_message::MessageId::new(),
                        session_id: mew_message::SessionId::new(),
                    },
                    text: String::new(),
                    signature: None,
                    encrypted_content: None,
                    provider_item_id: None,
                }),
            },
            mew_provider::ProviderEvent::PartDelta {
                part_id: reasoning_id,
                field: "text",
                delta: "private reasoning".into(),
            },
            mew_provider::ProviderEvent::PartEnd {
                part_id: reasoning_id,
            },
        ];
        events.extend(turn("final answer"));
        let (result, _) = run_provider_turns(vec![events], make_def(Some(10), Some(60))).await;
        assert!(matches!(result, SubagentResult::Complete { text, .. } if text == "final answer"));
    }

    #[tokio::test]
    async fn tool_only_last_message_does_not_reuse_earlier_narration() {
        let mut first = turn("earlier status");
        first.pop();
        first.extend(mew_provider_fake::FakeProvider::tool_call(
            "progress_update",
            "first",
            serde_json::json!({"message": "working"}),
        ));
        let last = mew_provider_fake::FakeProvider::tool_call(
            "progress_update",
            "last",
            serde_json::json!({"message": "still working"}),
        );
        let (result, _) = run_provider_turns(vec![first, last], make_def(Some(2), Some(60))).await;
        assert!(
            matches!(result, SubagentResult::Complete { text, hit_turn_limit: true, .. } if text.is_empty())
        );
    }

    #[tokio::test]
    async fn empty_final_message_does_not_reuse_earlier_narration() {
        let mut first = turn("earlier status");
        first.pop();
        first.extend(mew_provider_fake::FakeProvider::tool_call(
            "progress_update",
            "progress",
            serde_json::json!({"message": "working"}),
        ));
        let last = vec![mew_provider::ProviderEvent::MessageEnd {
            finish: mew_message::Finish::Stop,
            usage: mew_message::Tokens::default(),
            cost: 0.0,
        }];
        let (result, _) = run_provider_turns(vec![first, last], make_def(Some(10), Some(60))).await;
        assert!(
            matches!(&result, SubagentResult::Error { reason } if reason == "no assistant message received"),
            "unexpected result: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_output_schema_valid_first_try_passes_untouched() {
        let def = typed_def(serde_json::json!({"type": "object", "required": ["answer"]}));
        let (result, provider) = run_provider_turns(vec![turn("{\"answer\": 42}")], def).await;
        match result {
            SubagentResult::Complete { text, .. } => {
                assert_eq!(text, "{\"answer\": 42}");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert_eq!(
            *provider.calls.lock().unwrap(),
            1,
            "no corrective turn expected"
        );
    }

    #[tokio::test]
    async fn test_output_schema_corrective_turn_fixes_output() {
        let def = typed_def(serde_json::json!({"type": "object", "required": ["answer"]}));
        let (result, provider) =
            run_provider_turns(vec![turn("not json at all"), turn("{\"answer\": 42}")], def).await;
        match result {
            SubagentResult::Complete { text, .. } => {
                assert_eq!(text, "{\"answer\": 42}");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert_eq!(
            *provider.calls.lock().unwrap(),
            2,
            "exactly one corrective turn"
        );
    }

    #[tokio::test]
    async fn test_output_schema_second_failure_returns_raw_with_warning() {
        let def = typed_def(serde_json::json!({"type": "object", "required": ["answer"]}));
        let (result, provider) =
            run_provider_turns(vec![turn("junk one"), turn("junk two")], def).await;
        match result {
            SubagentResult::Complete { text, .. } => {
                assert!(
                    text.starts_with("warning: schema_invalid"),
                    "expected schema_invalid warning: {text}"
                );
                assert!(
                    text.contains("junk two"),
                    "raw text must be returned: {text}"
                );
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert_eq!(
            *provider.calls.lock().unwrap(),
            2,
            "only one corrective turn allowed"
        );
    }

    #[tokio::test]
    async fn test_output_schema_tolerates_json_fence() {
        let def = typed_def(serde_json::json!({"type": "object", "required": ["answer"]}));
        let fenced = "```json\n{\"answer\": 1}\n```";
        let (result, provider) = run_provider_turns(vec![turn(fenced)], def).await;
        match result {
            SubagentResult::Complete { text, .. } => {
                assert!(!text.starts_with("warning: schema_invalid"));
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert_eq!(*provider.calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_broken_output_schema_skips_corrective_turn() {
        // A schema that won't compile can never validate; the runner must
        // not burn a corrective turn on it.
        let def = typed_def(serde_json::json!({"type": "nonsense"}));
        let (result, provider) = run_provider_turns(vec![turn("{\"answer\": 1}")], def).await;
        match result {
            SubagentResult::Complete { text, .. } => {
                assert!(
                    text.starts_with("warning: schema_invalid"),
                    "expected schema_invalid warning: {text}"
                );
                assert!(
                    text.contains("output_schema is invalid"),
                    "expected schema error detail: {text}"
                );
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert_eq!(
            *provider.calls.lock().unwrap(),
            1,
            "broken schema must not trigger a corrective turn"
        );
    }

    #[tokio::test]
    async fn test_default_turn_cap_prevents_corrective_turn() {
        // def without max_turns: exhausting the DEFAULT_MAX_TURNS budget must
        // suppress the corrective turn (no budget left to correct with).
        let scripts = (1..mew_subagents::DEFAULT_MAX_TURNS)
            .map(|i| tool_turn(&format!("turn {i}")))
            .chain(std::iter::once(turn("not valid JSON")))
            .collect();
        let def = SubagentDef {
            output_schema: Some(serde_json::json!({"type": "object", "required": ["answer"]})),
            ..make_def(None, Some(60))
        };
        let (result, provider) = run_provider_turns(scripts, def).await;
        match result {
            SubagentResult::Complete { text, .. } => {
                assert!(
                    text.starts_with("warning: schema_invalid"),
                    "expected schema_invalid warning: {text}"
                );
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert_eq!(
            *provider.calls.lock().unwrap(),
            mew_subagents::DEFAULT_MAX_TURNS as usize,
            "exhausted turn budget must not grant a corrective turn"
        );
    }

    #[tokio::test]
    async fn exit_tool_can_finish_on_the_last_allowed_response() {
        let responses = vec![exit_tool_script("exit", "final answer")];
        let (result, provider) = run_provider_turns(responses, make_def(Some(1), Some(60))).await;

        assert!(matches!(result, SubagentResult::Complete {
                text, turns_used: 1, hit_turn_limit: true, ..
            } if text == "final answer"));
        assert_eq!(*provider.calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn schema_correction_uses_only_the_remaining_turn_budget() {
        let def = SubagentDef {
            max_turns: Some(2),
            ..typed_def(serde_json::json!({"type": "object", "required": ["answer"]}))
        };
        let responses = vec![
            turn("invalid initial answer"),
            tool_turn("still working"),
            turn(r#"{"answer":42}"#),
        ];
        let (result, provider) = run_provider_turns(responses, def).await;

        assert!(matches!(result, SubagentResult::Complete {
                text, turns_used: 2, hit_turn_limit: true, ..
            } if text.starts_with("warning: schema_invalid")));
        assert_eq!(*provider.calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn empty_final_text_is_a_successful_empty_answer() {
        let responses = vec![tool_turn("earlier narration"), turn("")];
        let (result, _) = run_provider_turns(responses, make_def(Some(3), Some(60))).await;

        assert!(
            matches!(result, SubagentResult::Complete { text, turns_used: 2, .. } if text.is_empty())
        );
    }

    #[tokio::test]
    async fn provider_failure_at_the_turn_cap_is_reported_as_failure() {
        let mut response = turn("partial text");
        if let Some(mew_provider::ProviderEvent::MessageEnd { finish, .. }) = response.last_mut() {
            *finish = mew_message::Finish::Error;
        }
        let (result, _) = run_provider_turns(vec![response], make_def(Some(1), Some(60))).await;
        assert!(
            matches!(result, SubagentResult::Error { reason } if reason.contains("failed response"))
        );
    }

    #[test]
    fn test_validate_against_schema_pure() {
        use SchemaCheck::*;
        let schema = serde_json::json!({"type": "object", "required": ["answer"]});
        assert!(matches!(
            validate_against_schema(&schema, "{\"answer\": 1}"),
            Valid
        ));
        assert!(matches!(
            validate_against_schema(&schema, "not json"),
            InvalidOutput(e) if e.contains("not valid JSON")
        ));
        assert!(matches!(
            validate_against_schema(&schema, "{\"other\": 1}"),
            InvalidOutput(_)
        ));
        assert!(matches!(
            validate_against_schema(&schema, "[1, 2]"),
            InvalidOutput(_)
        ));
        // A schema that won't compile is InvalidSchema, not InvalidOutput.
        assert!(matches!(
            validate_against_schema(&serde_json::json!({"type": "nonsense"}), "{}"),
            InvalidSchema(_)
        ));
    }
}
