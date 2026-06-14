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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tracing::{error, info, warn};

use mew_hooks::{
    ChatParams, Dispatcher, PermissionDecision, PluginHost, SlashCommandDef, ToolCall, ToolOutput,
    ToolRegistration,
};
use mew_message::Message;

use crate::loader::PluginLoader;

/// A running plugin subprocess with multiplexed JSON-RPC transport.
struct PluginProcess {
    name: String,
    /// The process handle, kept alive so the subprocess isn't killed.
    _child: Child,
    /// Pending host→plugin requests keyed by request id.
    pending: Arc<AsyncMutex<HashMap<u64, oneshot::Sender<String>>>>,
    writer: Arc<AsyncMutex<tokio::process::ChildStdin>>,
    next_id: AtomicU64,
    timeout: Duration,
}

impl PluginProcess {
    /// Spawn a plugin subprocess and start the stdout reader task.
    async fn spawn(
        path: &PathBuf,
        host: PluginHost,
    ) -> anyhow::Result<(Self, tokio::task::JoinHandle<()>)> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mut child = Command::new(path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("spawn plugin")?;

        let stdout = child.stdout.take().context("plugin stdout")?;
        let stdin = child.stdin.take().context("plugin stdin")?;

        let pending: Arc<AsyncMutex<HashMap<u64, oneshot::Sender<String>>>> =
            Arc::new(AsyncMutex::new(HashMap::new()));
        let writer = Arc::new(AsyncMutex::new(stdin));

        // Spawn the reader task.
        let reader_pending = pending.clone();
        let reader_writer = writer.clone();
        let reader_name = name.clone();
        let reader_host = host;
        let reader_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        info!("plugin {} stdout closed", reader_name);
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Value>(trimmed) {
                            Ok(msg) => {
                                handle_plugin_message(
                                    &reader_name,
                                    &msg,
                                    &reader_pending,
                                    &reader_writer,
                                    &reader_host,
                                )
                                .await;
                            }
                            Err(e) => {
                                warn!("plugin {} sent unparseable json: {}", reader_name, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("plugin {} stdout read error: {}", reader_name, e);
                        break;
                    }
                }
            }
        });

        let process = Self {
            name,
            _child: child,
            pending,
            writer,
            next_id: AtomicU64::new(1),
            timeout: Duration::from_secs(5),
        };

        Ok((process, reader_handle))
    }

    /// Send a request to the plugin and await the response.
    async fn call(&self, method: &str, params: &Value) -> anyhow::Result<String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        {
            let mut w = self.writer.lock().await;
            w.write_all(line.as_bytes()).await?;
            w.flush().await?;
        }

        let result = tokio::time::timeout(self.timeout, rx)
            .await
            .context("plugin response timed out")?
            .context("plugin response channel closed")?;

        // Remove the pending entry (in case the reader task didn't).
        {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
        }

        Ok(result)
    }

    /// Send a notification to the plugin (no response expected).
    async fn notify(&self, method: &str, params: &Value) {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let mut line = match serde_json::to_string(&request) {
            Ok(l) => l,
            Err(e) => {
                error!(
                    "plugin {} failed to serialize {} notification: {}",
                    self.name, method, e
                );
                return;
            }
        };
        line.push('\n');

        let mut w = self.writer.lock().await;
        if let Err(e) = w.write_all(line.as_bytes()).await {
            error!(
                "plugin {} failed to send {} notification: {}",
                self.name, method, e
            );
        }
        if let Err(e) = w.flush().await {
            error!(
                "plugin {} failed to flush {} notification: {}",
                self.name, method, e
            );
        }
    }
}

/// Handle one JSON message from a plugin's stdout.
///
/// Messages with an `id` matching a pending request are routed as responses.
/// Messages with a `method` field are plugin→host requests; they are handled
/// and a response is written back to the plugin's stdin.
async fn handle_plugin_message(
    name: &str,
    msg: &Value,
    pending: &AsyncMutex<HashMap<u64, oneshot::Sender<String>>>,
    writer: &AsyncMutex<tokio::process::ChildStdin>,
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
            let _ = sender.send(result_str);
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
            if let Err(e) = w.write_all(line.as_bytes()).await {
                error!("plugin {} failed to write host response: {}", name, e);
            }
            let _ = w.flush().await;
        }
        return;
    }

    // Neither a response nor a request — log and ignore.
    warn!("plugin {} sent unrecognized message: {}", name, msg);
}

/// Handle a plugin→host function call.
fn handle_host_request(
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
    plugins: Vec<PluginProcess>,
    /// Reader task handles — kept alive so stdout keeps being read.
    #[allow(dead_code)]
    reader_handles: Vec<tokio::task::JoinHandle<()>>,
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

    pub async fn from_dirs_filtered(
        dirs: Vec<PathBuf>,
        host: PluginHost,
        disabled: &[String],
    ) -> anyhow::Result<Self> {
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
        let mut processes = Vec::new();
        let mut reader_handles = Vec::new();

        for path in &plugin_paths {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            info!("starting plugin: {} ({})", name, path.display());

            match PluginProcess::spawn(path, host.as_ref().clone()).await {
                Ok((process, reader_handle)) => {
                    info!("plugin started: {}", name);
                    processes.push(process);
                    reader_handles.push(reader_handle);
                }
                Err(e) => {
                    warn!("failed to start plugin {}: {}", path.display(), e);
                }
            }
        }

        // Sort alphabetically by name for deterministic hook ordering.
        processes.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Self {
            plugins: processes,
            reader_handles,
        })
    }

    async fn pipe_json<T, F, G>(&self, method: &str, initial: &str, _default: F, parse: G) -> T
    where
        F: Fn() -> T,
        G: Fn(&str) -> T,
    {
        let mut current = initial.to_string();
        for plugin in &self.plugins {
            let params = serde_json::json!({
                "value": current,
            });
            match plugin.call(method, &params).await {
                Ok(result) => current = result,
                Err(e) => error!("plugin {} {}() failed: {}", plugin.name, method, e),
            }
        }
        parse(&current)
    }

    /// Fire-and-forget notification to all plugins.
    async fn notify_all(&self, method: String, params: Value) {
        for plugin in &self.plugins {
            plugin.notify(&method, &params).await;
        }
    }
}

#[async_trait]
impl Dispatcher for SubprocessDispatcher {
    async fn init(&self, _host: &PluginHost) {
        for plugin in &self.plugins {
            if let Err(e) = plugin.call("init", &serde_json::json!({})).await {
                error!("plugin {} init failed: {}", plugin.name, e);
            } else {
                info!("plugin {} initialised", plugin.name);
            }
        }
    }

    async fn shutdown(&self) {
        for plugin in &self.plugins {
            let _ = plugin.call("shutdown", &serde_json::json!({})).await;
        }
    }

    async fn on_event(&self, _ev: &dyn std::any::Any) {
        // on_event is a no-op in the subprocess runtime.
        // &dyn Any is !Send, so we cannot forward it through async_trait's
        // Send-bound future.
    }

    async fn on_turn_end(&self, messages: &[Message]) {
        let json = serde_json::to_string(messages).unwrap_or_default();
        let params = serde_json::json!({
            "messages": Value::String(json),
        });
        self.notify_all("on-turn-end".to_string(), params).await;
    }

    async fn on_system_prompt(&self, prompt: String) -> String {
        self.pipe_json(
            "on-system-prompt",
            &prompt,
            || prompt.clone(),
            |s| s.to_string(),
        )
        .await
    }

    async fn on_chat_message(&self, msg: Message) -> Message {
        let json = serde_json::to_string(&msg).unwrap_or_default();
        self.pipe_json(
            "on-chat-message",
            &json,
            || msg.clone(),
            |s| serde_json::from_str(s).unwrap_or(msg.clone()),
        )
        .await
    }

    async fn on_chat_params(&self, p: ChatParams) -> ChatParams {
        let json = serde_json::to_value(&p).unwrap_or_default().to_string();
        self.pipe_json(
            "on-chat-params",
            &json,
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
            .pipe_json(
                "on-chat-headers",
                &json,
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

    async fn on_tool_execute_before(&self, _call: &ToolCall, input: Value) -> Value {
        let json = input.to_string();
        self.pipe_json(
            "on-tool-execute-before",
            &json,
            || input.clone(),
            |s| serde_json::from_str(s).unwrap_or(input.clone()),
        )
        .await
    }

    async fn on_tool_execute_after(&self, _call: &ToolCall, output: ToolOutput) -> ToolOutput {
        let json = serde_json::to_string(&output).unwrap_or_default();
        self.pipe_json(
            "on-tool-execute-after",
            &json,
            || output.clone(),
            |s| serde_json::from_str(s).unwrap_or(output.clone()),
        )
        .await
    }

    async fn on_permission_ask(
        &self,
        _call: &ToolCall,
        current: PermissionDecision,
    ) -> PermissionDecision {
        let dec_str = format!("{:?}", current);
        let result = self
            .pipe_json(
                "on-permission-ask",
                &dec_str,
                || current,
                |s| match s {
                    "AllowOnce" => PermissionDecision::AllowOnce,
                    "AllowSession" => PermissionDecision::AllowSession,
                    "Deny" => PermissionDecision::Deny,
                    _ => current,
                },
            )
            .await;
        result
    }

    async fn on_shell_env(&self, env: HashMap<String, String>) -> HashMap<String, String> {
        let json = serde_json::to_string(&env).unwrap_or_default();
        self.pipe_json(
            "on-shell-env",
            &json,
            || env.clone(),
            |s| serde_json::from_str(s).unwrap_or(env.clone()),
        )
        .await
    }

    async fn on_register_tools(&self) -> Vec<ToolRegistration> {
        // Dynamic tool registration for subprocess plugins not supported
        // in this version. Tools should be registered via other mechanisms.
        Vec::new()
    }

    async fn on_register_slash_commands(&self) -> Vec<SlashCommandDef> {
        let mut all: Vec<SlashCommandDef> = Vec::new();
        for plugin in &self.plugins {
            match plugin
                .call("on-register-slash-commands", &serde_json::json!({}))
                .await
            {
                Ok(json_str) => {
                    if let Ok(cmds) = serde_json::from_str::<Vec<SlashCommandDef>>(&json_str) {
                        all.extend(cmds);
                    } else {
                        warn!(
                            "plugin {} returned invalid slash commands: {}",
                            plugin.name, json_str
                        );
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        "plugin {} does not support slash commands: {}",
                        plugin.name,
                        e
                    );
                }
            }
        }
        all
    }

    async fn execute_slash_command(&self, command: &str, args: &str) -> Option<String> {
        for plugin in &self.plugins {
            let params = serde_json::json!({
                "command": command,
                "args": args,
            });
            match plugin.call("execute-slash-command", &params).await {
                Ok(result) => return Some(result),
                Err(e) => {
                    tracing::debug!(
                        "plugin {} does not handle slash command '{}': {}",
                        plugin.name,
                        command,
                        e
                    );
                }
            }
        }
        None
    }
}
