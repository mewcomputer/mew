//! Extension broker — implements `Dispatcher` by routing hook calls to
//! extension processes with capability enforcement, concurrency, timeouts,
//! and audit logging.
//!
//! The broker sits between `mew-agent` (which calls `Dispatcher` methods)
//! and `mew-hooks-runtime`'s transport layer (`PluginSlot` /
//! `call_via_handles`). It owns `Vec<Arc<PluginSlot>>` and implements
//! `Dispatcher` by routing each hook call through one of four strategies:
//!
//! - **observe-event**: fire-and-forget notification to all eligible extensions
//! - **mutate-pipe**: concurrent `join_all`, last-writer-wins by alphabetical name
//! - **gate-audit**: like mutate-pipe but also writes a `GateAuditEntry`
//! - **registration**: collision-rejecting tool/command registration

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tracing::{debug, error, info, warn};

use mew_hooks::{
    BoxFuture, ChatParams, Dispatcher, HookId, HookOutcome, PermissionDecision, PluginHost,
    SlashCommandDef, ToolCall, ToolOutput, ToolRegistration,
};
use mew_message::Message;

use mew_hooks_runtime::{call_via_handles, PluginHandles, PluginLoader, PluginSlot};

use crate::audit::GateOutcome;
use crate::audit_log::AuditLog;
use crate::capabilities::{Capability, CapabilitySet};
use crate::principal::Principal;

// ── ExtensionBroker ─────────────────────────────────────────────────

/// The extension broker — a `Dispatcher` impl that routes hook calls
/// to extension subprocesses with capability enforcement and audit logging.
///
/// Replaces `SubprocessDispatcher` as the runtime's `Dispatcher`. The
/// `SubprocessDispatcher` type stays as dead code for rollback safety;
/// a follow-up PR removes it.
pub struct ExtensionBroker {
    /// Extension slots, sorted alphabetically by name.
    slots: Vec<Arc<PluginSlot>>,
    /// Per-plugin hook configuration from config.toml.
    configs: HashMap<String, mew_hooks::PluginHookConfig>,
    /// One principal per slot (capability set for enforcement).
    principals: Vec<Principal>,
    /// Gate audit log writer.
    audit: AuditLog,
    /// Session ID for audit entries (set by the agent setup path).
    session_id: Option<String>,
    /// Registered tool names → extension name (for collision detection).
    registered_tools: std::sync::Mutex<HashMap<String, String>>,
    /// Registered command names → extension name (for collision detection).
    registered_commands: std::sync::Mutex<HashMap<String, String>>,
}

impl ExtensionBroker {
    /// Default timeout: reads `MEW_PLUGIN_TIMEOUT_MS` env var, falls back to 5s.
    pub fn default_timeout() -> Duration {
        std::env::var("MEW_PLUGIN_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_secs(5))
    }

    /// Construct from plugin discovery dirs, host, disabled list, configs, timeout.
    /// Creates a `Principal::extension` with `CapabilitySet::legacy_full()`
    /// for each bare executable — matching current behavior exactly.
    pub async fn from_dirs_filtered_with_config(
        dirs: Vec<PathBuf>,
        host: PluginHost,
        disabled: &[String],
        configs: HashMap<String, mew_hooks::PluginHookConfig>,
        global_timeout: Duration,
    ) -> anyhow::Result<Self> {
        // Validate plugin configs.
        for (name, cfg) in &configs {
            cfg.validate(name);
        }

        let loader = PluginLoader::new(dirs);
        let plugin_paths: Vec<PathBuf> = loader
            .discover_executables()
            .into_iter()
            .filter(|path| {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                !disabled.contains(&name)
            })
            .collect();

        let host = Arc::new(host);
        let mut pairs: Vec<(Arc<PluginSlot>, Principal)> = Vec::new();

        for path in &plugin_paths {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let plugin_timeout = configs
                .get(&name)
                .and_then(|c| c.timeout_ms)
                .map(Duration::from_millis)
                .unwrap_or(global_timeout);

            match PluginSlot::spawn(path.clone(), host.as_ref().clone(), plugin_timeout).await {
                Ok(slot) => {
                    let principal =
                        Principal::extension(name.clone(), CapabilitySet::legacy_full());
                    pairs.push((slot, principal));
                }
                Err(e) => {
                    warn!("failed to start plugin {}: {}", path.display(), e);
                }
            }
        }

        // Sort alphabetically by slot name for deterministic hook ordering.
        pairs.sort_by(|a, b| a.0.name().cmp(b.0.name()));
        let (slots, principals): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();

        let audit = AuditLog::with_audit_dir(
            directories::ProjectDirs::from("ai", "mew", "mew")
                .map(|d| d.data_dir().join("extensions").join("audit"))
                .unwrap_or_else(|| PathBuf::from(".")),
        );

        Ok(Self {
            slots,
            configs,
            principals,
            audit,
            session_id: None,
            registered_tools: std::sync::Mutex::new(HashMap::new()),
            registered_commands: std::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Set the session ID for audit log entries.
    pub fn set_session_id(&mut self, id: String) {
        self.session_id = Some(id);
    }

    /// Read audit entries for a given extension (for tests and `mew ext audit`).
    pub fn audit_entries(&self, extension_name: &str) -> Vec<crate::audit::GateAuditEntry> {
        self.audit.read_all(extension_name)
    }

    /// True if `plugin_name` should receive `hook` for the given `subject`.
    fn should_fire(&self, plugin_name: &str, hook: &str, subject: Option<&str>) -> bool {
        match self.configs.get(plugin_name) {
            None => true,
            Some(cfg) => {
                if cfg.is_hook_disabled(hook) {
                    return false;
                }
                match subject {
                    Some(s) => cfg.matches(hook, s),
                    None => true,
                }
            }
        }
    }

    /// Map a `HookId` to its required capability.
    /// Returns `None` for lifecycle hooks (init/shutdown).
    pub fn hook_capability(hook: HookId) -> Option<Capability> {
        match hook {
            // observe-event
            HookId::ProviderEvent
            | HookId::ToolError
            | HookId::SubagentStart
            | HookId::SubagentEnd
            | HookId::TurnEnd
            | HookId::PreModelTurn
            | HookId::Stop
            | HookId::PreCompaction
            | HookId::PostCompaction
            | HookId::PersonaChange
            | HookId::SessionSave
            | HookId::ModelFinish => Some(Capability::HooksObserve),

            // mutate-pipe
            HookId::SystemPrompt
            | HookId::UserInput
            | HookId::ChatMessage
            | HookId::ToolExecuteAfter => Some(Capability::HooksMutate),

            HookId::ChatParams => Some(Capability::HooksMutateChatParams),
            HookId::ChatHeaders => Some(Capability::HooksMutateHeaders),
            HookId::ShellEnv => Some(Capability::HooksMutateShellEnv),

            // gate-audit
            HookId::ToolExecuteBefore | HookId::PermissionAsk => Some(Capability::HooksGate),
            // registration — handled separately, not via capability check
            // lifecycle — no capability required
        }
    }

    /// Check if slot at `idx` has the capability for `hook` and should fire.
    fn check_capability(&self, idx: usize, hook: HookId, subject: Option<&str>) -> bool {
        let slot = &self.slots[idx];
        if !slot.is_healthy() {
            return false;
        }
        if !self.should_fire(slot.name(), hook.as_config(), subject) {
            return false;
        }
        match Self::hook_capability(hook) {
            None => true, // lifecycle hooks
            Some(required) => {
                let has = self.principals[idx].has_capability(&required);
                if !has {
                    debug!(
                        "extension {} lacks capability {} for hook {}",
                        slot.name(),
                        required.id(),
                        hook.as_config()
                    );
                }
                has
            }
        }
    }

    /// Fire-and-forget notification to all eligible extensions.
    async fn notify_all_filtered(&self, hook: HookId, params: Value, subject: Option<&str>) {
        let wire = hook.as_wire();
        for (idx, slot) in self.slots.iter().enumerate() {
            if !self.check_capability(idx, hook, subject) {
                continue;
            }
            slot.notify(wire, &params).await;
        }
    }

    /// Pipe a value through extensions, filtering by hook + subject.
    /// All eligible extensions run in parallel; last alphabetical
    /// non-error response wins.
    async fn pipe_json_filtered<T, F, G>(
        &self,
        hook: HookId,
        initial: &str,
        subject: Option<&str>,
        _default: F,
        parse: G,
    ) -> T
    where
        F: Fn() -> T,
        G: Fn(&str) -> T,
    {
        let wire = hook.as_wire().to_string();
        let params = serde_json::json!({ "value": initial });

        let mut candidates: Vec<(String, PluginHandles)> = Vec::new();
        for (idx, slot) in self.slots.iter().enumerate() {
            if !self.check_capability(idx, hook, subject) {
                continue;
            }
            let receiver = slot.handles();
            let handles = receiver.borrow().clone();
            candidates.push((slot.name().to_string(), handles));
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0));

        let results = futures::future::join_all(candidates.iter().map(|(name, handles)| {
            let params = params.clone();
            let method = wire.clone();
            let name = name.clone();
            let handles = handles.clone();
            async move {
                let result = call_via_handles(&name, &method, &params, &handles).await;
                (name, result)
            }
        }))
        .await;

        let mut last: Option<String> = None;
        for (name, result) in results {
            match result {
                Ok(s) => last = Some(s),
                Err(e) => error!("plugin {} {}() failed: {}", name, wire, e),
            }
        }
        match last {
            Some(v) => parse(&v),
            None => parse(initial),
        }
    }

    /// Like `pipe_json_filtered` but returns the raw last response.
    /// Used by gate hooks so they can detect Block/Suppress markers.
    async fn pipe_json_raw(
        &self,
        hook: HookId,
        initial: &str,
        subject: Option<&str>,
    ) -> Vec<(String, Option<String>)> {
        let wire = hook.as_wire().to_string();
        let params = serde_json::json!({ "value": initial });

        let mut candidates: Vec<(String, PluginHandles)> = Vec::new();
        for (idx, slot) in self.slots.iter().enumerate() {
            if !self.check_capability(idx, hook, subject) {
                continue;
            }
            let receiver = slot.handles();
            let handles = receiver.borrow().clone();
            candidates.push((slot.name().to_string(), handles));
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0));

        let results = futures::future::join_all(candidates.iter().map(|(name, handles)| {
            let params = params.clone();
            let method = wire.clone();
            let name = name.clone();
            let handles = handles.clone();
            async move {
                let result = call_via_handles(&name, &method, &params, &handles).await;
                (name, result)
            }
        }))
        .await;

        results
            .into_iter()
            .map(|(name, result)| match result {
                Ok(s) => (name, Some(s)),
                Err(e) => {
                    error!("plugin {} {}() failed: {}", name, wire, e);
                    (name, None)
                }
            })
            .collect()
    }

    /// Check whether a raw plugin response indicates Block or Suppress.
    fn detect_outcome(raw: &str) -> Option<HookOutcome<()>> {
        let trimmed = raw.trim();
        let lower = trimmed.to_lowercase();
        if lower == "suppress" {
            Some(HookOutcome::Suppress)
        } else if let Some(reason) = lower.strip_prefix("block") {
            let reason = reason.trim_start_matches(':').trim();
            if reason.is_empty() {
                Some(HookOutcome::Block("blocked by plugin".into()))
            } else {
                Some(HookOutcome::Block(reason.into()))
            }
        } else {
            None
        }
    }
}

// ── Dispatcher impl ─────────────────────────────────────────────────

#[async_trait]
impl Dispatcher for ExtensionBroker {
    async fn init(&self, _host: &PluginHost) {
        for slot in &self.slots {
            if let Err(e) = slot.call("init", &serde_json::json!({})).await {
                error!("plugin {} init failed: {}", slot.name(), e);
            } else {
                info!("plugin {} initialised", slot.name());
            }
        }
    }

    async fn shutdown(&self) {
        for slot in &self.slots {
            slot.shutdown().await;
        }
    }

    async fn on_provider_event(&self, ev: &mew_provider::ProviderEvent) {
        let json = serde_json::to_string(ev).unwrap_or_default();
        let params = serde_json::json!({
            "event": Value::String(json),
        });
        self.notify_all_filtered(HookId::ProviderEvent, params, None)
            .await;
    }

    async fn on_tool_error(&self, call: &ToolCall, error: &str) {
        let params = serde_json::json!({
            "tool_name": &call.tool_name,
            "call_id": &call.call_id,
            "error": error,
        });
        self.notify_all_filtered(HookId::ToolError, params, Some(&call.tool_name))
            .await;
    }

    async fn on_subagent_start(
        &self,
        name: &str,
        parent_call_id: &str,
        display_name: Option<&str>,
    ) {
        let params = serde_json::json!({
            "name": name,
            "parent_call_id": parent_call_id,
            "display_name": display_name,
        });
        self.notify_all_filtered(HookId::SubagentStart, params, None)
            .await;
    }

    async fn on_subagent_end(&self, name: &str, parent_call_id: &str, outcome: &str) {
        let params = serde_json::json!({
            "name": name,
            "parent_call_id": parent_call_id,
            "outcome": outcome,
        });
        self.notify_all_filtered(HookId::SubagentEnd, params, None)
            .await;
    }

    async fn on_turn_end(&self, messages: &[Message]) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
        });
        self.notify_all_filtered(HookId::TurnEnd, params, None)
            .await;
    }

    async fn on_pre_model_turn(&self, messages: &[Message], system: &str) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
            "system": system,
        });
        self.notify_all_filtered(HookId::PreModelTurn, params, None)
            .await;
    }

    async fn on_stop(&self) {
        self.notify_all_filtered(HookId::Stop, serde_json::json!({}), None)
            .await;
    }

    async fn on_pre_compaction(&self, messages: &[Message]) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
        });
        self.notify_all_filtered(HookId::PreCompaction, params, None)
            .await;
    }

    async fn on_post_compaction(&self, messages: &[Message]) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
        });
        self.notify_all_filtered(HookId::PostCompaction, params, None)
            .await;
    }

    async fn on_system_prompt(&self, prompt: String) -> String {
        self.pipe_json_filtered(
            HookId::SystemPrompt,
            &prompt,
            None,
            || prompt.clone(),
            |s| s.to_string(),
        )
        .await
    }

    async fn on_chat_message(&self, msg: Message) -> Message {
        let json = serde_json::to_string(&msg).unwrap_or_default();
        self.pipe_json_filtered(
            HookId::ChatMessage,
            &json,
            None,
            || msg.clone(),
            |s| serde_json::from_str(s).unwrap_or(msg.clone()),
        )
        .await
    }

    async fn on_chat_params(&self, p: ChatParams) -> ChatParams {
        let json = serde_json::to_value(&p).unwrap_or_default().to_string();
        self.pipe_json_filtered(
            HookId::ChatParams,
            &json,
            None,
            || p.clone(),
            |s| serde_json::from_str(s).unwrap_or(p.clone()),
        )
        .await
    }

    async fn on_chat_headers(&self, h: http::HeaderMap) -> http::HeaderMap {
        let pairs: Vec<(String, String)> = h
            .iter()
            .map(|(n, v)| (n.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let json = serde_json::to_string(&pairs).unwrap_or_default();
        let result: Vec<(String, String)> = self
            .pipe_json_filtered(
                HookId::ChatHeaders,
                &json,
                None,
                || pairs.clone(),
                |s| serde_json::from_str(s).unwrap_or(pairs.clone()),
            )
            .await;
        let mut headers = http::HeaderMap::new();
        for (name, value) in &result {
            if let (Ok(n), Ok(v)) = (
                http::HeaderName::from_bytes(name.as_bytes()),
                http::HeaderValue::from_str(value),
            ) {
                headers.insert(n, v);
            }
        }
        headers
    }

    async fn on_tool_execute_before(&self, call: &ToolCall, input: Value) -> HookOutcome<Value> {
        let json = input.to_string();
        let results = self
            .pipe_json_raw(HookId::ToolExecuteBefore, &json, Some(&call.tool_name))
            .await;

        let mut last: Option<String> = None;
        for (ext_name, raw_opt) in &results {
            let raw = match raw_opt {
                Some(r) => r,
                None => continue,
            };

            // Audit the gate decision.
            let outcome = match Self::detect_outcome(raw) {
                Some(HookOutcome::Proceed(_)) => GateOutcome::Proceed,
                Some(HookOutcome::Block(_)) => GateOutcome::Block,
                Some(HookOutcome::Suppress) => GateOutcome::Suppress,
                None => {
                    // Check if the value was mutated (different from input).
                    if raw.trim() != json.trim() {
                        GateOutcome::Mutated
                    } else {
                        GateOutcome::Proceed
                    }
                }
            };
            self.audit.log(crate::audit::GateAuditEntry::new(
                ext_name.clone(),
                self.session_id.clone().unwrap_or_else(|| "unknown".into()),
                &call.tool_name,
                format!("sha256:{:x}", sha2::Sha256::digest(json.as_bytes())),
                outcome,
            ));

            last = Some(raw.clone());
        }

        match last {
            Some(raw) => {
                if let Some(outcome) = Self::detect_outcome(&raw) {
                    return match outcome {
                        HookOutcome::Proceed(_) => HookOutcome::Proceed(input),
                        HookOutcome::Block(r) => HookOutcome::Block(r),
                        HookOutcome::Suppress => HookOutcome::Suppress,
                    };
                }
                let v = serde_json::from_str(&raw).unwrap_or(input);
                HookOutcome::Proceed(v)
            }
            None => HookOutcome::Proceed(input),
        }
    }

    async fn on_tool_execute_after(&self, call: &ToolCall, output: ToolOutput) -> ToolOutput {
        let json = serde_json::to_string(&output).unwrap_or_default();
        self.pipe_json_filtered(
            HookId::ToolExecuteAfter,
            &json,
            Some(&call.tool_name),
            || output.clone(),
            |s| serde_json::from_str(s).unwrap_or(output.clone()),
        )
        .await
    }

    async fn on_permission_ask(
        &self,
        call: &ToolCall,
        current: PermissionDecision,
    ) -> HookOutcome<PermissionDecision> {
        let dec_str = format!("{:?}", current);
        let results = self
            .pipe_json_raw(HookId::PermissionAsk, &dec_str, Some(&call.tool_name))
            .await;

        let mut last: Option<String> = None;
        for (ext_name, raw_opt) in &results {
            let raw = match raw_opt {
                Some(r) => r,
                None => continue,
            };

            // Audit the gate decision.
            let outcome = match Self::detect_outcome(raw) {
                Some(HookOutcome::Proceed(_)) => GateOutcome::Proceed,
                Some(HookOutcome::Block(_)) => GateOutcome::Block,
                Some(HookOutcome::Suppress) => GateOutcome::Suppress,
                None => GateOutcome::Proceed,
            };
            self.audit.log(crate::audit::GateAuditEntry::new(
                ext_name.clone(),
                self.session_id.clone().unwrap_or_else(|| "unknown".into()),
                &call.tool_name,
                format!("sha256:{:x}", sha2::Sha256::digest(dec_str.as_bytes())),
                outcome,
            ));

            last = Some(raw.clone());
        }

        match last {
            Some(raw) => {
                if let Some(outcome) = Self::detect_outcome(&raw) {
                    return match outcome {
                        HookOutcome::Proceed(_) => HookOutcome::Proceed(current),
                        HookOutcome::Block(r) => HookOutcome::Block(r),
                        HookOutcome::Suppress => HookOutcome::Suppress,
                    };
                }
                let v = match raw.trim() {
                    "AllowOnce" => PermissionDecision::AllowOnce,
                    "AllowSession" => PermissionDecision::AllowSession,
                    "Deny" => PermissionDecision::Deny,
                    _ => current,
                };
                HookOutcome::Proceed(v)
            }
            None => HookOutcome::Proceed(current),
        }
    }

    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String> {
        let json = serde_json::to_string(&env).unwrap_or_default();
        self.pipe_json_filtered(
            HookId::ShellEnv,
            &json,
            None,
            || env.clone(),
            |s| serde_json::from_str(s).unwrap_or(env.clone()),
        )
        .await
    }

    async fn on_user_input(&self, prompt: String) -> String {
        self.pipe_json_filtered(
            HookId::UserInput,
            &prompt,
            None,
            || prompt.clone(),
            |s| s.to_string(),
        )
        .await
    }

    async fn on_persona_change(&self, old_persona: Option<&str>, new_persona: &str) {
        let params = serde_json::json!({
            "old_persona": old_persona,
            "new_persona": new_persona,
        });
        self.notify_all_filtered(HookId::PersonaChange, params, None)
            .await;
    }

    async fn on_session_save(&self) {
        self.notify_all_filtered(HookId::SessionSave, serde_json::json!({}), None)
            .await;
    }

    async fn on_model_finish(
        &self,
        finish: &str,
        input_tokens: u32,
        output_tokens: u32,
        cost: f64,
    ) {
        let params = serde_json::json!({
            "finish": finish,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cost": cost,
        });
        self.notify_all_filtered(HookId::ModelFinish, params, None)
            .await;
    }

    async fn on_register_tools(&self) -> Vec<ToolRegistration> {
        let mut all: Vec<ToolRegistration> = Vec::new();
        for slot in &self.slots {
            if !slot.is_healthy() {
                continue;
            }
            match slot.call("on-register-tools", &serde_json::json!({})).await {
                Ok(json_str) => {
                    let defs: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
                        Ok(v) => v,
                        Err(e) => {
                            error!(
                                "plugin {} on-register-tools response invalid: {}",
                                slot.name(),
                                e
                            );
                            continue;
                        }
                    };
                    for def in defs {
                        let name = match def.get("name").and_then(|v| v.as_str()) {
                            Some(n) => n.to_string(),
                            None => continue,
                        };
                        let description = def
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let schema = def
                            .get("input_schema")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        let plugin_name = slot.name().to_string();
                        let tool_name = name.clone();

                        // Collision detection: skip if another extension
                        // already registered this tool name.
                        {
                            let mut reg = self.registered_tools.lock().unwrap();
                            if let Some(existing) = reg.get(&name) {
                                if existing != &plugin_name {
                                    warn!(
                                        "tool '{}' from extension '{}' conflicts with extension '{}'; skipping",
                                        name, plugin_name, existing
                                    );
                                    continue;
                                }
                            }
                            reg.insert(name.clone(), plugin_name.clone());
                        }

                        let handles_receiver = slot.handles();
                        let execute: Box<dyn Fn(Value) -> BoxFuture<String> + Send + Sync> =
                            Box::new(move |input: Value| {
                                let tool_name = tool_name.clone();
                                let plugin_name = plugin_name.clone();
                                let receiver = handles_receiver.clone();
                                Box::pin(async move {
                                    let handles = receiver.borrow().clone();
                                    if !handles.healthy.load(Ordering::Acquire) {
                                        return format!("plugin '{}' is not running", plugin_name);
                                    }
                                    let id = handles.next_id.fetch_add(1, Ordering::Relaxed);
                                    let (tx, rx) = tokio::sync::oneshot::channel();
                                    {
                                        let mut p = handles.pending.lock().await;
                                        p.insert(id, tx);
                                    }
                                    let request = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "method": "call-tool",
                                        "params": {
                                            "name": tool_name,
                                            "input": input,
                                        },
                                        "id": id,
                                    });
                                    let mut line =
                                        serde_json::to_string(&request).unwrap_or_default();
                                    line.push('\n');
                                    {
                                        let mut w = handles.writer.lock().await;
                                        if let Some(w) = w.as_mut() {
                                            let _ = tokio::io::AsyncWriteExt::write_all(
                                                w,
                                                line.as_bytes(),
                                            )
                                            .await;
                                            let _ = tokio::io::AsyncWriteExt::flush(w).await;
                                        }
                                    }
                                    match tokio::time::timeout(handles.timeout, rx).await {
                                        Ok(Ok(Ok(result))) => {
                                            serde_json::from_str::<String>(&result)
                                                .unwrap_or(result)
                                        }
                                        Ok(Ok(Err(e))) => format!(
                                            "plugin '{}' tool '{}' error: {}",
                                            plugin_name, tool_name, e
                                        ),
                                        _ => format!(
                                            "plugin '{}' tool '{}' timed out",
                                            plugin_name, tool_name
                                        ),
                                    }
                                })
                            });
                        all.push(ToolRegistration {
                            name,
                            description,
                            input_schema: schema,
                            execute,
                        });
                    }
                }
                Err(e) => {
                    debug!("plugin {} on-register-tools: {}", slot.name(), e);
                }
            }
        }
        all
    }

    async fn on_register_slash_commands(&self) -> Vec<SlashCommandDef> {
        let mut all: Vec<SlashCommandDef> = Vec::new();
        for slot in &self.slots {
            match slot
                .call("on-register-slash-commands", &serde_json::json!({}))
                .await
            {
                Ok(json_str) => {
                    if let Ok(cmds) = serde_json::from_str::<Vec<SlashCommandDef>>(&json_str) {
                        for cmd in cmds {
                            // Collision detection.
                            {
                                let mut reg = self.registered_commands.lock().unwrap();
                                if let Some(existing) = reg.get(&cmd.name) {
                                    if existing != slot.name() {
                                        warn!(
                                            "command '{}' from extension '{}' conflicts with extension '{}'; skipping",
                                            cmd.name, slot.name(), existing
                                        );
                                        continue;
                                    }
                                }
                                reg.insert(cmd.name.clone(), slot.name().to_string());
                            }
                            all.push(cmd);
                        }
                    } else {
                        warn!(
                            "plugin {} returned invalid slash commands: {}",
                            slot.name(),
                            json_str
                        );
                    }
                }
                Err(e) => {
                    debug!(
                        "plugin {} does not support slash commands: {}",
                        slot.name(),
                        e
                    );
                }
            }
        }
        all
    }

    async fn execute_slash_command(&self, command: &str, args: &str) -> Option<String> {
        for slot in &self.slots {
            let params = serde_json::json!({
                "command": command,
                "args": args,
            });
            match slot.call("execute-slash-command", &params).await {
                Ok(result) => return Some(result),
                Err(e) => {
                    debug!(
                        "plugin {} does not handle slash command '{}': {}",
                        slot.name(),
                        command,
                        e
                    );
                }
            }
        }
        None
    }
}

use sha2::Digest;
