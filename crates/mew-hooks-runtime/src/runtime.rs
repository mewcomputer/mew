//! Subprocess-based `Dispatcher` implementation.
//!
//! Each plugin is a subprocess. Communication uses newline-delimited JSON-RPC 2.0
//! over stdin/stdout with unique request IDs for multiplexing.
//!
//! Host → Plugin  (hook calls):
//!   {"jsonrpc":"2.0","method":"hook-name","params":{...},"id":1}
//!   Plugin → {"jsonrpc":"2.0","result":"...","id":1}
//!
//! Plugin → Host  (host function calls):
//!   {"jsonrpc":"2.0","method":"host-set-ui","params":{"key":"x","value":"y"},"id":2}
//!   Host → {"jsonrpc":"2.0","result":"ok","id":2}
//!
//! A spawned reader task on plugin stdout distinguishes responses (by matching
//! pending request IDs) from plugin-initiated requests (by their `method` field).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, warn};

use mew_hooks::{
    BoxFuture, ChatParams, Dispatcher, PermissionDecision, PluginHost, SlashCommandDef, ToolCall,
    ToolOutput, ToolRegistration,
};
use mew_message::Message;

use crate::loader::PluginLoader;
use crate::transport::{call_via_handles, PluginHandles, PluginSlot};

// handle_plugin_message and handle_host_request remain in this file
// (pub(crate)) and are called from transport.rs's reader task.

/// Handle one JSON message from a plugin's stdout.
///
/// Messages with an `id` matching a pending request are routed as responses.
/// Messages with a `method` field are plugin→host requests; they are handled
/// and a response is written back to the plugin's stdin.
pub(crate) async fn handle_plugin_message(
    name: &str,
    msg: &Value,
    pending: &crate::transport::PendingMap,
    writer: &tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
    host: &PluginHost,
) {
    // If this is a response to one of our requests.
    if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
        if let Some(sender) = pending.lock().await.remove(&id) {
            let result_str = msg
                .get("result")
                .map(|v| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_default();
            let _ = sender.send(Ok(result_str));
            return;
        }
        // If we have an id but no matching pending request, it's a stale
        // response or a plugin-initiated request with an id.
    }

    // If this is a plugin→host request.
    if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
        let id = msg.get("id").and_then(|v| v.as_u64());
        let result = handle_host_request(name, method, msg.get("params"), host);
        if let Some(id) = id {
            // Write response back to the plugin.
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "result": result,
                "id": id,
            });
            let mut line = serde_json::to_string(&response).unwrap_or_default();
            line.push('\n');
            let mut w = writer.lock().await;
            if let Some(w) = w.as_mut() {
                if let Err(e) = w.write_all(line.as_bytes()).await {
                    error!("plugin {} failed to write host response: {}", name, e);
                }
                let _ = w.flush().await;
            }
        }
        return;
    }

    // Neither a response nor a request — log and ignore.
    warn!("plugin {} sent unrecognized message: {}", name, msg);
}

/// Handle a plugin→host function call.
pub(crate) fn handle_host_request(
    _name: &str,
    method: &str,
    params: Option<&Value>,
    host: &PluginHost,
) -> String {
    match method {
        "host-notify" => {
            let msg = params
                .and_then(|p| p.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (host.notify)(msg.to_string());
            "ok".to_string()
        }
        "host-config-read" => {
            let key = params
                .and_then(|p| p.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (host.config_read)(key).unwrap_or_default()
        }
        "host-log" => {
            let msg = params
                .and_then(|p| p.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (host.log)(msg.to_string());
            "ok".to_string()
        }
        "host-storage-read" => {
            let key = params
                .and_then(|p| p.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (host.storage_read)(key).unwrap_or_default()
        }
        "host-storage-write" => {
            let key = params
                .and_then(|p| p.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let val = params
                .and_then(|p| p.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (host.storage_write)(key, val);
            "ok".to_string()
        }
        "host-storage-delete" => {
            let key = params
                .and_then(|p| p.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (host.storage_delete)(key);
            "ok".to_string()
        }
        "host-set-ui" => {
            let key = params
                .and_then(|p| p.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let val = params
                .and_then(|p| p.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (host.set_ui)(key, val);
            "ok".to_string()
        }
        other => {
            warn!("plugin {} called unknown host method: {}", _name, other);
            format!("unknown method: {}", other)
        }
    }
}

pub struct SubprocessDispatcher {
    slots: Vec<Arc<PluginSlot>>,
    /// Per-call deadline for each plugin hook invocation. A hung plugin
    /// can't stall the turn loop — the timeout fires, the hook falls back
    /// to its default value, and the error is logged.
    timeout: Duration,
    /// Per-plugin hook configuration from config.toml. Keyed by plugin
    /// name (the executable's stem). Controls which hooks fire, what they
    /// match against, and per-plugin timeout overrides.
    configs: HashMap<String, mew_hooks::PluginHookConfig>,
}

impl SubprocessDispatcher {
    pub async fn from_default_dirs(host: PluginHost) -> anyhow::Result<Self> {
        let dirs = crate::PluginLoader::default_dirs();
        Self::from_dirs(dirs, host).await
    }

    pub async fn from_default_dirs_filtered(
        host: PluginHost,
        disabled: &[String],
    ) -> anyhow::Result<Self> {
        let dirs = crate::PluginLoader::default_dirs();
        Self::from_dirs_filtered(dirs, host, disabled).await
    }

    pub async fn from_dirs(dirs: Vec<PathBuf>, host: PluginHost) -> anyhow::Result<Self> {
        Self::from_dirs_filtered(dirs, host, &[]).await
    }

    /// Resolve the per-call plugin deadline from the `MEW_PLUGIN_TIMEOUT_MS`
    /// env var, falling back to 5 seconds. This is the safety net that
    /// prevents a hung plugin subprocess from blocking the turn loop.
    pub fn default_timeout() -> Duration {
        std::env::var("MEW_PLUGIN_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_secs(5))
    }

    /// Override the per-call deadline. Builder-style for programmatic
    /// callers that want a different value than the env var default.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        // Per-slot timeout is set at spawn time and can't be retroactively
        // changed on a running process. The self.timeout value is used
        // for any future slots spawned by from_dirs* constructors.
        self
    }

    pub async fn from_dirs_filtered(
        dirs: Vec<PathBuf>,
        host: PluginHost,
        disabled: &[String],
    ) -> anyhow::Result<Self> {
        let timeout = Self::default_timeout();
        Self::from_dirs_filtered_with_config(dirs, host, disabled, HashMap::new(), timeout).await
    }

    pub async fn from_dirs_filtered_with_timeout(
        dirs: Vec<PathBuf>,
        host: PluginHost,
        disabled: &[String],
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        Self::from_dirs_filtered_with_config(dirs, host, disabled, HashMap::new(), timeout).await
    }

    /// Full constructor with per-plugin hook configuration from
    /// config.toml. The `configs` map is keyed by plugin name (executable
    /// stem) and controls disabled hooks, matchers, and per-plugin
    /// timeout overrides.
    pub async fn from_dirs_filtered_with_config(
        dirs: Vec<PathBuf>,
        host: PluginHost,
        disabled: &[String],
        configs: HashMap<String, mew_hooks::PluginHookConfig>,
        global_timeout: Duration,
    ) -> anyhow::Result<Self> {
        // Validate plugin configs — warn on unknown hook names so typos
        // don't silently suppress hooks.
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
        let mut slots = Vec::new();

        for path in &plugin_paths {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Use per-plugin timeout if configured, otherwise the global.
            let plugin_timeout = configs
                .get(&name)
                .and_then(|c| c.timeout_ms)
                .map(Duration::from_millis)
                .unwrap_or(global_timeout);

            match PluginSlot::spawn(
                crate::transport::SpawnSpec::Path(path.clone()),
                host.as_ref().clone(),
                plugin_timeout,
            )
            .await
            {
                Ok(slot) => {
                    slots.push(slot);
                }
                Err(e) => {
                    warn!("failed to start plugin {}: {}", path.display(), e);
                }
            }
        }

        // Sort alphabetically by name for deterministic hook ordering.
        slots.sort_by(|a, b| a.name().cmp(b.name()));

        Ok(Self {
            slots,
            timeout: global_timeout,
            configs,
        })
    }

    /// True if `plugin_name` should receive `hook` for the given `subject`.
    /// Consults the per-plugin config for disabled hooks and matchers.
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

    /// Fire-and-forget notification to plugins that should receive `hook`.
    /// `subject` is the tool name for tool hooks, or `None` for non-tool hooks.
    async fn notify_all_filtered(
        &self,
        hook: mew_hooks::HookId,
        params: Value,
        subject: Option<&str>,
    ) {
        let wire = hook.as_wire();
        for slot in &self.slots {
            if !self.should_fire(slot.name(), hook.as_config(), subject) {
                continue;
            }
            if !slot.is_healthy() {
                continue;
            }
            slot.notify(wire, &params).await;
        }
    }

    /// Pipe a value through plugins, filtering by hook name + subject.
    /// Used by mutation hooks that carry a tool name (e.g.
    /// `on_tool_execute_before`). All eligible plugins run in parallel
    /// with the same initial value; the result is from the last
    /// alphabetical non-error response, falling back to `parse(initial)`
    /// if no plugin responds.
    async fn pipe_json_filtered<T, F, G>(
        &self,
        hook: mew_hooks::HookId,
        initial: &str,
        subject: Option<&str>,
        _default: F,
        parse: G,
    ) -> T
    where
        F: Fn() -> T,
        G: Fn(&str) -> T,
    {
        // Collect candidates: clone name + handles from each healthy slot.
        // We must not hold any guard across the join_all await.
        let wire = hook.as_wire().to_string();
        let params = serde_json::json!({ "value": initial });

        let mut candidates: Vec<(String, PluginHandles)> = Vec::new();
        for slot in &self.slots {
            if !self.should_fire(slot.name(), hook.as_config(), subject) {
                continue;
            }
            if !slot.is_healthy() {
                continue;
            }
            let receiver = slot.handles();
            let handles = receiver.borrow().clone();
            candidates.push((slot.name().to_string(), handles));
        }
        // Sort by name for deterministic last-writer-wins resolution.
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

    /// Like `pipe_json_filtered` but returns the raw last plugin response
    /// (or None if all plugins failed / there were no candidates). Used by
    /// the blocking hooks (`on_permission_ask`, `on_tool_execute_before`)
    /// so they can detect Block/Suppress markers before parsing the value.
    async fn pipe_json_raw(
        &self,
        hook: mew_hooks::HookId,
        initial: &str,
        subject: Option<&str>,
    ) -> Option<String> {
        let wire = hook.as_wire().to_string();
        let params = serde_json::json!({ "value": initial });

        let mut candidates: Vec<(String, PluginHandles)> = Vec::new();
        for slot in &self.slots {
            if !self.should_fire(slot.name(), hook.as_config(), subject) {
                continue;
            }
            if !slot.is_healthy() {
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
        last
    }

    /// Check whether a raw plugin response indicates Block or Suppress.
    /// Returns `Some(Outcome)` for recognized markers; `None` for normal
    /// responses that should be parsed as a value.
    fn detect_outcome(raw: &str) -> Option<mew_hooks::HookOutcome<()>> {
        let trimmed = raw.trim();
        let lower = trimmed.to_lowercase();
        if lower == "suppress" {
            Some(mew_hooks::HookOutcome::Suppress)
        } else if let Some(reason) = lower.strip_prefix("block") {
            let reason = reason.trim_start_matches(':').trim();
            if reason.is_empty() {
                Some(mew_hooks::HookOutcome::Block("blocked by plugin".into()))
            } else {
                Some(mew_hooks::HookOutcome::Block(reason.into()))
            }
        } else {
            None
        }
    }
}

#[async_trait]
impl Dispatcher for SubprocessDispatcher {
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
        self.notify_all_filtered(mew_hooks::HookId::ProviderEvent, params, None)
            .await;
    }

    async fn on_tool_error(&self, call: &ToolCall, error: &str) {
        let params = serde_json::json!({
            "tool_name": &call.tool_name,
            "call_id": &call.call_id,
            "error": error,
        });
        self.notify_all_filtered(mew_hooks::HookId::ToolError, params, Some(&call.tool_name))
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
        self.notify_all_filtered(mew_hooks::HookId::SubagentStart, params, None)
            .await;
    }

    async fn on_subagent_end(&self, name: &str, parent_call_id: &str, outcome: &str) {
        let params = serde_json::json!({
            "name": name,
            "parent_call_id": parent_call_id,
            "outcome": outcome,
        });
        self.notify_all_filtered(mew_hooks::HookId::SubagentEnd, params, None)
            .await;
    }

    async fn on_turn_end(&self, messages: &[Message]) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
        });
        self.notify_all_filtered(mew_hooks::HookId::TurnEnd, params, None)
            .await;
    }

    async fn on_pre_model_turn(&self, messages: &[Message], system: &str) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
            "system": system,
        });
        self.notify_all_filtered(mew_hooks::HookId::PreModelTurn, params, None)
            .await;
    }

    async fn on_stop(&self) {
        self.notify_all_filtered(mew_hooks::HookId::Stop, serde_json::json!({}), None)
            .await;
    }

    async fn on_pre_compaction(&self, messages: &[Message]) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
        });
        self.notify_all_filtered(mew_hooks::HookId::PreCompaction, params, None)
            .await;
    }

    async fn on_post_compaction(&self, messages: &[Message]) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
        });
        self.notify_all_filtered(mew_hooks::HookId::PostCompaction, params, None)
            .await;
    }

    async fn on_system_prompt(&self, prompt: String) -> String {
        self.pipe_json_filtered(
            mew_hooks::HookId::SystemPrompt,
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
            mew_hooks::HookId::ChatMessage,
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
            mew_hooks::HookId::ChatParams,
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
                mew_hooks::HookId::ChatHeaders,
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

    async fn on_tool_execute_before(
        &self,
        call: &ToolCall,
        input: Value,
    ) -> mew_hooks::HookOutcome<Value> {
        let json = input.to_string();
        match self
            .pipe_json_raw(
                mew_hooks::HookId::ToolExecuteBefore,
                &json,
                Some(&call.tool_name),
            )
            .await
        {
            Some(raw) => {
                // Check for Block/Suppress markers in the plugin response.
                if let Some(outcome) = Self::detect_outcome(&raw) {
                    return match outcome {
                        mew_hooks::HookOutcome::Proceed(_) => {
                            mew_hooks::HookOutcome::Proceed(input)
                        }
                        mew_hooks::HookOutcome::Block(r) => mew_hooks::HookOutcome::Block(r),
                        mew_hooks::HookOutcome::Suppress => mew_hooks::HookOutcome::Suppress,
                    };
                }
                // Normal response: parse as the modified value.
                let v = serde_json::from_str(&raw).unwrap_or(input);
                mew_hooks::HookOutcome::Proceed(v)
            }
            None => mew_hooks::HookOutcome::Proceed(input),
        }
    }

    async fn on_tool_execute_after(&self, call: &ToolCall, output: ToolOutput) -> ToolOutput {
        let json = serde_json::to_string(&output).unwrap_or_default();
        self.pipe_json_filtered(
            mew_hooks::HookId::ToolExecuteAfter,
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
    ) -> mew_hooks::HookOutcome<PermissionDecision> {
        let dec_str = format!("{:?}", current);
        match self
            .pipe_json_raw(
                mew_hooks::HookId::PermissionAsk,
                &dec_str,
                Some(&call.tool_name),
            )
            .await
        {
            Some(raw) => {
                // Check for Block/Suppress markers in the plugin response.
                if let Some(outcome) = Self::detect_outcome(&raw) {
                    return match outcome {
                        mew_hooks::HookOutcome::Proceed(_) => {
                            mew_hooks::HookOutcome::Proceed(current)
                        }
                        mew_hooks::HookOutcome::Block(r) => mew_hooks::HookOutcome::Block(r),
                        mew_hooks::HookOutcome::Suppress => mew_hooks::HookOutcome::Suppress,
                    };
                }
                // Normal response: parse as a PermissionDecision.
                let v = match raw.trim() {
                    "AllowOnce" => PermissionDecision::AllowOnce,
                    "AllowSession" => PermissionDecision::AllowSession,
                    "Deny" => PermissionDecision::Deny,
                    _ => current,
                };
                mew_hooks::HookOutcome::Proceed(v)
            }
            None => mew_hooks::HookOutcome::Proceed(current),
        }
    }

    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String> {
        let json = serde_json::to_string(&env).unwrap_or_default();
        self.pipe_json_filtered(
            mew_hooks::HookId::ShellEnv,
            &json,
            None,
            || env.clone(),
            |s| serde_json::from_str(s).unwrap_or(env.clone()),
        )
        .await
    }

    async fn on_user_input(&self, prompt: String) -> String {
        self.pipe_json_filtered(
            mew_hooks::HookId::UserInput,
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
        self.notify_all_filtered(mew_hooks::HookId::PersonaChange, params, None)
            .await;
    }

    async fn on_session_save(&self) {
        self.notify_all_filtered(mew_hooks::HookId::SessionSave, serde_json::json!({}), None)
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
        self.notify_all_filtered(mew_hooks::HookId::ModelFinish, params, None)
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
                        // Capture the watch receiver — tool closures always
                        // see the current process's handles, even after restart.
                        let handles_receiver = slot.handles();
                        let execute: Box<dyn Fn(Value) -> BoxFuture<String> + Send + Sync> =
                            Box::new(move |input: Value| {
                                let tool_name = tool_name.clone();
                                let plugin_name = plugin_name.clone();
                                let receiver = handles_receiver.clone();
                                Box::pin(async move {
                                    // borrow() is lock-free, but the Ref
                                    // can't be held across .await — clone out.
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
                    // Plugin doesn't support tool registration — fine.
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
                        all.extend(cmds);
                    } else {
                        warn!(
                            "plugin {} returned invalid slash commands: {}",
                            slot.name(),
                            json_str
                        );
                    }
                }
                Err(e) => {
                    tracing::debug!(
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
                    tracing::debug!(
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
