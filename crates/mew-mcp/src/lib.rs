use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcNotification<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("configuration: {0}")]
    Config(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("json-rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("initialization failed: {0}")]
    Init(String),
    #[error("http: {0}")]
    Http(reqwest::Error),
}

impl From<reqwest::Error> for McpError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
}

// ---------------------------------------------------------------------------
// Transport abstraction
// ---------------------------------------------------------------------------

#[async_trait]
trait Transport: Send + Sync {
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, McpError>;
    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError>;
    fn set_protocol_version(&self, _version: &str) {
        // Default no-op for transports that don't need version negotiation.
    }
    async fn close(&mut self) -> Result<(), McpError>;
}

// ---------------------------------------------------------------------------
// HTTP transport (streamable HTTP)
// ---------------------------------------------------------------------------

/// Parse an SSE (Server-Sent Events) response from an MCP server.
async fn parse_sse_response(mut resp: reqwest::Response, id: u64) -> Result<Value, McpError> {
    let mut buffer = Vec::new();
    let mut data_buffer = String::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| McpError::Transport(format!("read sse body: {}", e.without_url())))?
    {
        buffer.extend_from_slice(&chunk);
        if buffer.len() + data_buffer.len() > 16 * 1024 * 1024 {
            return Err(McpError::Protocol("SSE response too large".into()));
        }
        let mut consumed = 0;
        while let Some(offset) = buffer[consumed..].iter().position(|byte| *byte == b'\n') {
            let end = consumed + offset;
            let line = &buffer[consumed..end];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            consumed = end + 1;
            let line = std::str::from_utf8(line)
                .map_err(|e| McpError::Protocol(format!("invalid SSE text: {e}")))?;
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.strip_prefix(' ').unwrap_or(data);
                if data == "[DONE]" {
                    continue;
                }
                if !data_buffer.is_empty() {
                    data_buffer.push('\n');
                }
                data_buffer.push_str(data);
            } else if line.is_empty() && !data_buffer.is_empty() {
                if let Some(result) = try_parse_sse_data(&data_buffer, id) {
                    return result;
                }
                data_buffer.clear();
            }
        }
        buffer.drain(..consumed);
    }
    if !data_buffer.is_empty() {
        if let Some(result) = try_parse_sse_data(&data_buffer, id) {
            return result;
        }
    }

    Err(McpError::Protocol("no data in SSE response".into()))
}

/// Try to parse accumulated SSE data as a JSON-RPC response. Returns
/// `Some(Ok(...))` for a valid response, `Some(Err(...))` for a fatal error,
/// or `None` to skip this event (e.g. it was a progress notification).
fn try_parse_sse_data(data: &str, id: u64) -> Option<Result<Value, McpError>> {
    let parsed: serde_json::Value = serde_json::from_str(data).ok()?;

    // Check if it's a notification (has method but no id/result).
    if parsed.get("method").is_some() && parsed.get("id").is_none() {
        // Notification (e.g. progress) — skip, keep looking for response.
        debug!("sse notification: {}", parsed["method"]);
        return None;
    }

    // Parse as response.
    let response: JsonRpcResponse = match serde_json::from_value(parsed) {
        Ok(r) => r,
        Err(_) => return None,
    };

    if response.id != Some(id) {
        return None;
    }

    if let Some(err) = response.error {
        return Some(Err(McpError::Rpc {
            code: err.code,
            message: err.message,
        }));
    }

    response.result.map(Ok)
}

/// Maximum MCP protocol version this client supports. Servers reporting
/// a version at or below this are accepted (the protocol is backward-compatible).
/// Servers reporting a newer version are rejected since we may not understand
/// new protocol features.
const MAX_PROTOCOL_VERSION: &str = "2025-11-25";

/// Returns true if the server's protocol version is acceptable (≤ our max).
fn version_acceptable(server_version: &str) -> bool {
    server_version <= MAX_PROTOCOL_VERSION
}

struct HttpTransport {
    client: reqwest::Client,
    url: String,
    request_id: AtomicU64,
    /// Negotiated MCP protocol version from the initialize response.
    protocol_version: StdMutex<String>,
    /// Optional session ID returned by the server.
    session_id: StdMutex<Option<String>>,
}

impl HttpTransport {
    fn new(
        url: String,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<Self, McpError> {
        let mut defaults = reqwest::header::HeaderMap::new();
        for (name, value) in headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| McpError::Config("invalid HTTP header name".into()))?;
            if matches!(
                name.as_str(),
                "content-type" | "accept" | "mcp-protocol-version" | "mcp-session-id"
            ) {
                return Err(McpError::Config(format!(
                    "reserved MCP HTTP header: {name}"
                )));
            }
            let mut value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| McpError::Config(format!("invalid value for HTTP header {name}")))?;
            value.set_sensitive(true);
            defaults.insert(name, value);
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .default_headers(defaults)
                .build()?,
            url: url.trim_end_matches('/').to_string(),
            request_id: AtomicU64::new(1),
            protocol_version: StdMutex::new(MAX_PROTOCOL_VERSION.into()),
            session_id: StdMutex::new(None),
        })
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    fn build_request(&self, body: Vec<u8>) -> reqwest::RequestBuilder {
        let mut builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(body);

        let ver = self.protocol_version.lock().unwrap();
        builder = builder.header("MCP-Protocol-Version", ver.as_str());
        let sid = self.session_id.lock().unwrap();
        if let Some(ref id) = *sid {
            builder = builder.header("MCP-Session-Id", id.as_str());
        }

        builder
    }

    /// Update headers from an HTTP response (session id, protocol version).
    fn update_from_response(&self, resp: &reqwest::Response) {
        if let Some(val) = resp.headers().get("mcp-session-id") {
            if let Ok(id) = val.to_str() {
                *self.session_id.lock().unwrap() = Some(id.to_string());
            }
        }
        if let Some(val) = resp.headers().get("mcp-protocol-version") {
            if let Ok(ver) = val.to_str() {
                self.set_protocol_version_inherent(ver);
            }
        }
    }

    /// Set the negotiated protocol version (called during initialize).
    /// This is an inherent method used by update_from_response.
    fn set_protocol_version_inherent(&self, version: &str) {
        *self.protocol_version.lock().unwrap() = version.to_string();
    }
}

struct HttpCancellation(Option<reqwest::RequestBuilder>);

impl Drop for HttpCancellation {
    fn drop(&mut self) {
        if let (Some(request), Ok(runtime)) = (self.0.take(), tokio::runtime::Handle::try_current())
        {
            runtime.spawn(async move {
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(3), request.send()).await;
            });
        }
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        let id = self.next_id();
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let body = serde_json::to_vec(&request).map_err(|e| McpError::Transport(e.to_string()))?;

        debug!(%method, id, "mcp http request");
        let cancellation = serde_json::json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":id,"reason":"cancelled by client"}});
        let cancellation =
            serde_json::to_vec(&cancellation).map_err(|e| McpError::Transport(e.to_string()))?;
        let mut cancellation = HttpCancellation(Some(self.build_request(cancellation)));
        let result = async {
            let resp = self.build_request(body).send().await?;
            self.update_from_response(&resp);

            let status = resp.status();
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            if !status.is_success() {
                return Err(McpError::Transport(format!("HTTP {status}")));
            }

            // Handle SSE (text/event-stream) responses.
            if content_type.starts_with("text/event-stream") {
                return parse_sse_response(resp, id).await;
            }

            let response: JsonRpcResponse = resp
                .json()
                .await
                .map_err(|e| McpError::Transport(format!("parse response: {}", e.without_url())))?;

            if response.id != Some(id) {
                return Err(McpError::Protocol(
                    "HTTP response ID does not match the request".into(),
                ));
            }

            if let Some(err) = response.error {
                return Err(McpError::Rpc {
                    code: err.code,
                    message: err.message,
                });
            }

            response
                .result
                .ok_or_else(|| McpError::Protocol("missing result in response".into()))
        }
        .await;
        cancellation.0 = None;
        result
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let request = JsonRpcNotification {
            jsonrpc: "2.0",
            method,
            params,
        };

        let body = serde_json::to_vec(&request).map_err(|e| McpError::Transport(e.to_string()))?;

        let resp = self.build_request(body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(McpError::Transport(format!(
                "notification failed: HTTP {status}"
            )));
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<(), McpError> {
        Ok(())
    }

    fn set_protocol_version(&self, version: &str) {
        self.set_protocol_version_inherent(version);
    }
}

// ---------------------------------------------------------------------------
// stdio transport (subprocess)
// ---------------------------------------------------------------------------

struct StdioTransport {
    child: tokio::sync::Mutex<tokio::process::Child>,
    #[cfg(unix)]
    process_group: Option<i32>,
    stdin: Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>,
    stdout: tokio::sync::Mutex<StdioOutput>,
    call_lock: Arc<tokio::sync::Mutex<()>>,
    request_id: AtomicU64,
}

struct StdioOutput {
    reader: BufReader<tokio::process::ChildStdout>,
    line: Vec<u8>,
}

impl StdioTransport {
    fn new(mut child: tokio::process::Child) -> Result<Self, McpError> {
        #[cfg(unix)]
        let process_group = child.id().and_then(|id| i32::try_from(id).ok());
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("child has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("child has no stdout".into()))?;

        Ok(Self {
            child: tokio::sync::Mutex::new(child),
            #[cfg(unix)]
            process_group,
            stdin: Arc::new(tokio::sync::Mutex::new(Some(stdin))),
            stdout: tokio::sync::Mutex::new(StdioOutput {
                reader: BufReader::new(stdout),
                line: Vec::new(),
            }),
            call_lock: Arc::new(tokio::sync::Mutex::new(())),
            request_id: AtomicU64::new(1),
        })
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.process_group {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

struct StdioCancellation {
    stdin: Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>,
    id: u64,
    armed: bool,
    call_guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl Drop for StdioCancellation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let stdin = self.stdin.clone();
        let call_guard = self.call_guard.take();
        let message = serde_json::json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":self.id,"reason":"cancelled by client"}}).to_string() + "\n";
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _call_guard = call_guard;
                if let Some(stdin) = stdin.lock().await.as_mut() {
                    let _ = stdin.write_all(message.as_bytes()).await;
                }
            });
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        let call_guard = self.call_lock.clone().lock_owned().await;
        let id = self.next_id();
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let mut body =
            serde_json::to_string(&request).map_err(|e| McpError::Transport(e.to_string()))?;
        body.push('\n');

        debug!(%method, id, "mcp stdio request");

        // Finish a frame even if the caller is cancelled during a write.
        let mut guard = self.stdin.clone().lock_owned().await;
        let mut cancellation = StdioCancellation {
            stdin: self.stdin.clone(),
            id,
            armed: true,
            call_guard: Some(call_guard),
        };
        tokio::spawn(async move {
            let stdin = guard
                .as_mut()
                .ok_or_else(|| McpError::Transport("stdin closed".into()))?;
            stdin
                .write_all(body.as_bytes())
                .await
                .map_err(|e| McpError::Transport(format!("write stdin: {e}")))
        })
        .await
        .map_err(|error| McpError::Transport(format!("write task: {error}")))??;

        let mut stdout = self.stdout.lock().await;
        loop {
            let StdioOutput { reader, line } = &mut *stdout;
            let remaining = (16 * 1024 * 1024 + 1 - line.len()) as u64;
            let count = reader
                .take(remaining)
                .read_until(b'\n', line)
                .await
                .map_err(|e| McpError::Transport(format!("read stdout: {e}")))?;
            if count == 0 {
                return Err(McpError::Transport("server closed stdout".into()));
            }
            if line.len() > 16 * 1024 * 1024 || line.last() != Some(&b'\n') {
                line.clear();
                return Err(McpError::Protocol(
                    "stdio response too large or incomplete".into(),
                ));
            }
            let message = serde_json::from_slice(line);
            line.clear();
            let message: Value =
                message.map_err(|e| McpError::Transport(format!("parse response: {e}")))?;
            if message.get("method").is_some() {
                if let Some(request_id) = message.get("id") {
                    let reply = serde_json::json!({"jsonrpc":"2.0", "id":request_id, "error":{"code":-32601,"message":"method not supported"}});
                    let mut body = serde_json::to_vec(&reply)
                        .map_err(|e| McpError::Transport(e.to_string()))?;
                    body.push(b'\n');
                    let mut guard = self.stdin.lock().await;
                    let stdin = guard
                        .as_mut()
                        .ok_or_else(|| McpError::Transport("stdin closed".into()))?;
                    stdin
                        .write_all(&body)
                        .await
                        .map_err(|e| McpError::Transport(format!("write stdin: {e}")))?;
                }
                continue;
            }
            let response: JsonRpcResponse = serde_json::from_value(message)
                .map_err(|e| McpError::Transport(format!("parse response: {e}")))?;
            if response.id != Some(id) {
                continue;
            }
            cancellation.armed = false;
            if let Some(err) = response.error {
                return Err(McpError::Rpc {
                    code: err.code,
                    message: err.message,
                });
            }
            return response
                .result
                .ok_or_else(|| McpError::Protocol("missing result in stdio response".into()));
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let request = JsonRpcNotification {
            jsonrpc: "2.0",
            method,
            params,
        };

        let mut body =
            serde_json::to_string(&request).map_err(|e| McpError::Transport(e.to_string()))?;
        body.push('\n');

        let mut guard = self.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| McpError::Transport("stdin closed".into()))?;
        stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| McpError::Transport(format!("write stdin: {e}")))?;
        Ok(())
    }

    async fn close(&mut self) -> Result<(), McpError> {
        self.stdin.lock().await.take();
        let mut child = self.child.lock().await;
        #[cfg(unix)]
        if let Some(pid) = self.process_group.take() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let wait = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
        if wait.is_err() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MCP server config (from user configuration)
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize)]
pub struct McpServerConfig {
    /// Server name (e.g. "context7")
    #[serde(default)]
    pub name: String,
    /// Transport type: "stdio" (default) or "http"
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
    /// HTTP URL (streamable HTTP transport)
    #[serde(default)]
    pub url: Option<String>,
    /// stdio command
    #[serde(default)]
    pub command: Option<String>,
    /// stdio arguments
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
}

/// Load global and project `mcp.json` files; project entries take precedence.
pub fn load_servers(
    cwd: &std::path::Path,
    config_dir: &std::path::Path,
) -> Result<Vec<McpServerConfig>, McpError> {
    #[derive(Deserialize)]
    struct ConfigFile {
        #[serde(rename = "mcpServers", default)]
        servers: std::collections::BTreeMap<String, McpServerConfig>,
    }

    let root = mew_harness::find_git_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let project_file = mew_harness::paths_between(&root, cwd)
        .into_iter()
        .rev()
        .map(|path| path.join("mcp.json"))
        .find(|path| path.is_file());
    let mut merged = std::collections::BTreeMap::new();
    for path in std::iter::once(config_dir.join("mcp.json")).chain(project_file) {
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(McpError::Config(format!("{}: {error}", path.display()))),
        };
        let config: ConfigFile = serde_json::from_slice(&data)
            .map_err(|error| McpError::Config(format!("{}: {error}", path.display())))?;
        merged.extend(config.servers);
    }
    Ok(merged
        .into_iter()
        .filter_map(|(name, mut server)| {
            server.name = name;
            (!server.disabled).then_some(server)
        })
        .collect())
}

/// Connect configured servers and expose their tools through Mew's tool interface.
pub async fn connect_tools(
    configs: Vec<McpServerConfig>,
    cwd: &std::path::Path,
) -> Vec<Arc<dyn mew_tools::Tool>> {
    let mut tasks = tokio::task::JoinSet::new();
    for config in configs {
        if config.disabled {
            continue;
        }
        let cwd = cwd.to_path_buf();
        tasks.spawn(async move {
            let name = config.name.clone();
            let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                let client = Arc::new(McpClient::connect_configured(&config, &cwd).await?);
                let definitions = client.list_tools().await?;
                Ok::<_, McpError>((client, definitions))
            })
            .await;
            (name, result)
        });
    }
    let mut connected = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let (name, result) = match result {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(%error, "MCP connection task failed");
                continue;
            }
        };
        let (client, definitions) = match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let reason = match error {
                    McpError::Config(_) => "invalid configuration",
                    McpError::Transport(_) | McpError::Http(_) => "transport error",
                    McpError::Rpc { .. } => "server rejected request",
                    McpError::Protocol(_) => "protocol error",
                    McpError::Init(_) => "initialization error",
                };
                tracing::warn!(server = %name, reason, "MCP server unavailable");
                continue;
            }
            Err(_) => {
                tracing::warn!(server = %name, "MCP server startup timed out");
                continue;
            }
        };
        connected.push((name, client, definitions));
    }
    connected.sort_by(|a, b| a.0.cmp(&b.0));
    let mut tools: Vec<Arc<dyn mew_tools::Tool>> = Vec::new();
    let mut names = std::collections::HashSet::new();
    for (name, client, mut definitions) in connected {
        definitions.sort_by(|a, b| a.name.cmp(&b.name));
        for definition in definitions {
            let qualified = definition.qualified_name(&name);
            if name.is_empty()
                || definition.name.is_empty()
                || qualified.len() > 64
                || !qualified
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                || !definition.input_schema.is_object()
            {
                tracing::warn!(server = %name, tool = %definition.name, "skipping MCP tool with an unsupported name or schema");
                continue;
            }
            if !names.insert(qualified.clone()) {
                tracing::warn!(tool = %qualified, "skipping duplicate MCP tool");
                continue;
            }
            tools.push(Arc::new(McpTool::new(&name, &definition, client.clone())));
        }
    }
    tools.sort_by(|a, b| a.name().cmp(b.name()));
    tools
}

// ---------------------------------------------------------------------------
// MCP client
// ---------------------------------------------------------------------------

/// A connected MCP client, holding one transport.
pub struct McpClient {
    name: String,
    transport: Box<dyn Transport + 'static>,
}

impl McpClient {
    /// Connect to an MCP server over HTTP.
    pub async fn connect_http(name: &str, url: &str) -> Result<Self, McpError> {
        Self::connect_http_with_headers(name, url, &Default::default()).await
    }

    async fn connect_http_with_headers(
        name: &str,
        url: &str,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<Self, McpError> {
        let transport = HttpTransport::new(url.to_string(), headers)?;
        let mut client = Self {
            name: name.to_string(),
            transport: Box::new(transport),
        };
        client.initialize().await?;
        Ok(client)
    }

    /// Connect to an MCP server over stdio (spawn command).
    pub async fn connect_stdio(
        name: &str,
        command: &str,
        args: &[String],
    ) -> Result<Self, McpError> {
        Self::connect_stdio_with_env(name, command, args, &Default::default(), None).await
    }

    async fn connect_stdio_with_env(
        name: &str,
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self, McpError> {
        let mut process = tokio::process::Command::new(command);
        process.args(args).envs(env);
        #[cfg(unix)]
        process.process_group(0);
        if let Some(cwd) = cwd {
            process.current_dir(cwd);
        }
        let child = process
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| McpError::Transport(format!("spawn {}: {}", command, e)))?;

        let transport = StdioTransport::new(child)?;
        let mut client = Self {
            name: name.to_string(),
            transport: Box::new(transport),
        };
        // Ensure cleanup if initialization fails.
        if let Err(e) = client.initialize().await {
            let _ = client.shutdown().await;
            return Err(McpError::Init(e.to_string()));
        }
        Ok(client)
    }

    /// Connect using an entry from `mcp.json`.
    pub async fn connect_configured(
        config: &McpServerConfig,
        cwd: &std::path::Path,
    ) -> Result<Self, McpError> {
        match (
            config.type_.as_deref(),
            config.url.as_deref(),
            config.command.as_deref(),
        ) {
            (
                None | Some("http") | Some("streamable") | Some("streamable-http"),
                Some(url),
                None,
            ) if config.env.is_empty() && config.args.is_empty() => {
                Self::connect_http_with_headers(&config.name, url, &config.headers).await
            }
            (None | Some("stdio"), None, Some(command)) if config.headers.is_empty() => {
                Self::connect_stdio_with_env(
                    &config.name,
                    command,
                    &config.args,
                    &config.env,
                    Some(cwd),
                )
                .await
            }
            _ => Err(McpError::Config(format!(
                "server {:?} has an invalid transport configuration",
                config.name
            ))),
        }
    }

    /// Perform the MCP handshake: initialize → get capabilities → send initialized.
    async fn initialize(&mut self) -> Result<(), McpError> {
        info!(server = %self.name, "initializing MCP server");

        let result = self
            .transport
            .call(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": MAX_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "mew",
                        "title": "mew",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
            )
            .await?;

        info!(server = %self.name, protocol_version = %result["protocolVersion"], "MCP server initialized");

        let server_version = result["protocolVersion"].as_str().unwrap_or("");
        if server_version.is_empty() {
            // Server may have sent the version only in the response header.
            // update_from_response already extracted it if present.
            debug!("no protocolVersion in body, using header value");
        } else if !version_acceptable(server_version) {
            return Err(McpError::Init(format!(
                "unsupported protocol version: {server_version} (max: {})",
                MAX_PROTOCOL_VERSION
            )));
        }

        self.set_negotiated_version(server_version);

        // Send initialized notification (no response expected).
        self.transport
            .notify("notifications/initialized", None)
            .await?;
        Ok(())
    }

    /// List tools available on this server. Handles pagination.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = cursor.as_ref().map(|c| serde_json::json!({ "cursor": c }));

            let result = self.transport.call("tools/list", params).await?;

            let page: Vec<McpToolDef> = result
                .get("tools")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| {
                            Some(McpToolDef {
                                name: t.get("name")?.as_str()?.to_string(),
                                description: t
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                input_schema: t.get("inputSchema").cloned().unwrap_or(Value::Null),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            info!(server = %self.name, page_count = page.len(), "discovered MCP tool page");
            tools.extend(page);

            cursor = result
                .get("nextCursor")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            if cursor.is_none() {
                break;
            }
        }

        info!(server = %self.name, count = tools.len(), "discovered all MCP tools");
        Ok(tools)
    }

    /// Call a tool on the server. Returns (output_text, is_error).
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<ToolCallResult, McpError> {
        let result = self
            .transport
            .call(
                "tools/call",
                Some(serde_json::json!({
                    "name": tool_name,
                    "arguments": arguments,
                })),
            )
            .await?;

        let content = result.get("content").and_then(|v| v.as_array());
        let text = content
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let structured_content = result.get("structuredContent").cloned();
        let images = content
            .into_iter()
            .flatten()
            .filter_map(|item| {
                if item.get("type").and_then(Value::as_str) != Some("image") {
                    return None;
                }
                Some(mew_message::ToolImage {
                    mime: item.get("mimeType")?.as_str()?.to_owned(),
                    data: item.get("data")?.as_str()?.to_owned(),
                })
            })
            .collect();

        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok(ToolCallResult {
            text,
            is_error,
            images,
            structured_content,
        })
    }

    /// Shut down the server connection.
    pub async fn shutdown(&mut self) -> Result<(), McpError> {
        info!(server = %self.name, "shutting down MCP server");
        self.transport.close().await
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn set_negotiated_version(&self, version: &str) {
        self.transport.set_protocol_version(version);
    }
}

/// Result of a tool call.
pub struct ToolCallResult {
    pub text: String,
    pub is_error: bool,
    pub images: Vec<mew_message::ToolImage>,
    pub structured_content: Option<Value>,
}

// ---------------------------------------------------------------------------
// Tool definitions from MCP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl McpToolDef {
    /// Full qualified name: `mcp__<server>__<tool>`
    pub fn qualified_name(&self, server_name: &str) -> String {
        fn component(name: &str) -> String {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        }
        let mut server = component(server_name);
        while server.contains("__") {
            server = server.replace("__", "_");
        }
        let server = server.trim_end_matches('_');
        let server = if server.is_empty() { "server" } else { server };
        format!("mcp__{}__{}", server, component(&self.name))
    }
}

// ---------------------------------------------------------------------------
// Tool implementation (adapts MCP tool to mew Tool trait)
// ---------------------------------------------------------------------------

pub struct McpTool {
    qualified_name: String,
    description: String,
    schema: Value,
    tool_name: String,
    client: Arc<McpClient>,
}

impl McpTool {
    pub fn new(server_name: &str, def: &McpToolDef, client: Arc<McpClient>) -> Self {
        let qualified = def.qualified_name(server_name);
        Self {
            qualified_name: qualified,
            description: def.description.clone(),
            schema: def.input_schema.clone(),
            tool_name: def.name.clone(),
            client,
        }
    }
}

#[async_trait]
impl mew_tools::Tool for McpTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    fn sensitivity(&self) -> mew_tools::Sensitivity {
        mew_tools::Sensitivity::Mutating
    }

    async fn execute(
        &self,
        ctx: mew_tools::ToolCtx,
        input: Value,
    ) -> Result<mew_hooks::ToolOutput, mew_tools::ToolError> {
        let result = tokio::select! {
            _ = ctx.cancel.cancelled() => return Err(mew_tools::ToolError::Execution("MCP tool cancelled".into())),
            result = self.client.call_tool(&self.tool_name, input) => result,
        };
        match result {
            Ok(mut result) => {
                if let Some(value) = &mut result.structured_content {
                    redact_structured(value, &ctx.secrets);
                    if result.text.is_empty() {
                        result.text = serde_json::to_string(value)
                            .map_err(|error| mew_tools::ToolError::Execution(error.to_string()))?;
                    }
                }
                let (text, count) =
                    mew_tools::secrets::redact_secret_words(&result.text, &ctx.secrets);
                let text = mew_tools::secrets::annotate_redaction(text, count);
                let (output, error) = if result.is_error {
                    (String::new(), text)
                } else {
                    (text, String::new())
                };
                Ok(mew_hooks::ToolOutput {
                    output,
                    error,
                    metadata: result
                        .structured_content
                        .map(|value| serde_json::json!({"structured_content":value})),
                    images: if ctx.supports_vision {
                        result.images
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                })
            }
            Err(e) => Ok(mew_hooks::ToolOutput {
                output: String::new(),
                error: mew_tools::secrets::redact_secret_words(&e.to_string(), &ctx.secrets).0,
                diff: None,
                metadata: None,
                file_delta: None,
                images: vec![],
            }),
        }
    }
}

fn redact_structured(value: &mut Value, secrets: &mew_tools::SecretSet) {
    if secrets.words.is_empty() {
        return;
    }
    match value {
        Value::String(text) => *text = mew_tools::secrets::redact_secret_words(text, secrets).0,
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| redact_structured(value, secrets)),
        Value::Object(values) => {
            for (key, mut value) in std::mem::take(values) {
                let key = mew_tools::secrets::redact_secret_words(&key, secrets).0;
                redact_structured(&mut value, secrets);
                values.insert(key, value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_mcp_tool_names_are_valid_for_model_providers() {
        let tool = McpToolDef {
            name: "find.docs".into(),
            description: String::new(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        assert_eq!(tool.qualified_name("docs.v2"), "mcp__docs_v2__find_docs");
        assert_eq!(
            tool.qualified_name("nested__server"),
            "mcp__nested_server__find_docs"
        );
    }

    #[test]
    fn mcp_configuration_merges_project_and_global_servers() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("config");
        let root = dir.path().join("project");
        let nested = root.join("src");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(global.join("mcp.json"), r#"{"mcpServers":{"shared":{"url":"https://global.example/mcp"},"local":{"command":"python3","args":["server.py"],"env":{"TOKEN":"test"}}}}"#).unwrap();
        std::fs::write(root.join("mcp.json"), r#"{"mcpServers":{"shared":{"url":"https://project.example/mcp","headers":{"Authorization":"test"}},"local":{"disabled":true}}}"#).unwrap();
        let servers = load_servers(&nested, &global).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "shared");
        assert_eq!(
            servers[0].url.as_deref(),
            Some("https://project.example/mcp")
        );
        assert_eq!(servers[0].headers["Authorization"], "test");
    }

    #[test]
    fn mcp_configuration_rejects_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mcp.json"), "not json").unwrap();
        let error = load_servers(dir.path(), &dir.path().join("global"))
            .err()
            .expect("invalid configuration should fail");
        assert!(error.to_string().contains("mcp.json"));
    }

    #[tokio::test]
    async fn configured_stdio_server_receives_its_environment_and_working_directory() {
        let directory = tempfile::tempdir().unwrap();
        let server = r#"
import json, os, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    else:
        result = {'content':[{'type':'text','text':os.environ['MEW_TEST_MCP_VALUE'] + ':' + os.getcwd()}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec!["-u".into(), "-c".into(), server.into()],
            env: std::collections::HashMap::from([(
                "MEW_TEST_MCP_VALUE".into(),
                "configured".into(),
            )]),
            headers: Default::default(),
            disabled: false,
        };
        let client = McpClient::connect_configured(&config, directory.path())
            .await
            .unwrap();
        let result = client
            .call_tool("environment", serde_json::json!({}))
            .await
            .unwrap();
        let expected = format!(
            "configured:{}",
            std::fs::canonicalize(directory.path()).unwrap().display()
        );
        assert_eq!(result.text, expected);
    }

    #[tokio::test]
    async fn configured_servers_discover_tools_that_read_real_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("fixture.txt"), "mcp fixture").unwrap();
        let server = r#"
import json, pathlib, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{'tools':{}},'serverInfo':{'name':'test','version':'1'}}
    elif req['method'] == 'tools/list':
        result = {'tools':[{'name':'read_file','description':'Read a file','inputSchema':{'type':'object','properties':{'path':{'type':'string'}},'required':['path']}}]}
    else:
        text = pathlib.Path(req['params']['arguments']['path']).read_text()
        result = {'content':[{'type':'text','text':text}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "files".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec!["-u".into(), "-c".into(), server.into()],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let tools = connect_tools(vec![config], directory.path()).await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "mcp__files__read_file");
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "call".into(),
            Default::default(),
            tx,
        );
        let output = tools[0]
            .execute(ctx, serde_json::json!({"path":"fixture.txt"}))
            .await
            .unwrap();
        assert_eq!(output.output, "mcp fixture");
        assert!(output.error.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_stdio_server_terminates_its_child_processes() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("late");
        let server = r#"
import json, os, sys, time
r, w = os.pipe()
if os.fork() == 0:
    os.close(r)
    f = open(sys.argv[1], 'w')
    os.write(w, b'x')
    time.sleep(1.5)
    f.write('escaped')
    f.flush()
    os._exit(0)
os.close(w)
os.read(r, 1)
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let client = McpClient::connect_stdio(
            "test",
            "python3",
            &[
                "-u".into(),
                "-c".into(),
                server.into(),
                marker.to_string_lossy().into_owned(),
            ],
        )
        .await
        .unwrap();
        drop(client);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "");
    }

    #[tokio::test]
    async fn mcp_tool_preserves_structured_results_and_images() {
        let server = r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    else:
        result = {'content':[{'type':'image','mimeType':'image/png','data':'aGVsbG8='}], 'structuredContent':{'count':42}}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let client = Arc::new(
            McpClient::connect_stdio(
                "test",
                "python3",
                &["-u".into(), "-c".into(), server.into()],
            )
            .await
            .unwrap(),
        );
        let tool = McpTool::new(
            "test",
            &McpToolDef {
                name: "image".into(),
                description: String::new(),
                input_schema: serde_json::json!({"type":"object"}),
            },
            client,
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let shared = mew_tools::ToolCtxShared {
            supports_vision: true,
            ..Default::default()
        };
        let ctx = mew_tools::ToolCtx::new(Arc::new(shared), "call".into(), Default::default(), tx);
        let output = mew_tools::Tool::execute(&tool, ctx, serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(output.output, r#"{"count":42}"#);
        assert_eq!(output.metadata.unwrap()["structured_content"]["count"], 42);
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].mime, "image/png");
        assert_eq!(output.images[0].data, "aGVsbG8=");
    }

    #[tokio::test]
    async fn mcp_tool_redacts_configured_secrets_in_text_and_structured_results() {
        let server = r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    else:
        result = {'content':[{'type':'text','text':'value=secret-token'}], 'structuredContent':{'nested':['secret-token']}}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let client = Arc::new(
            McpClient::connect_stdio(
                "test",
                "python3",
                &["-u".into(), "-c".into(), server.into()],
            )
            .await
            .unwrap(),
        );
        let tool = McpTool::new(
            "test",
            &McpToolDef {
                name: "read".into(),
                description: String::new(),
                input_schema: serde_json::json!({"type":"object"}),
            },
            client,
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let shared = mew_tools::ToolCtxShared {
            secrets: Arc::new(mew_tools::SecretSet {
                words: vec!["secret-token".into()],
                globs: vec![],
            }),
            ..Default::default()
        };
        let ctx = mew_tools::ToolCtx::new(Arc::new(shared), "call".into(), Default::default(), tx);
        let output = mew_tools::Tool::execute(&tool, ctx, serde_json::json!({}))
            .await
            .unwrap();
        assert!(!output.output.contains("secret-token"));
        assert_eq!(
            output.metadata.unwrap()["structured_content"]["nested"][0],
            "[REDACTED]"
        );
    }

    #[tokio::test]
    async fn stdio_calls_ignore_notifications_and_match_their_responses() {
        let server = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    if request['method'] == 'initialize':
        result = {'protocolVersion': '2025-11-25', 'capabilities': {}, 'serverInfo': {'name': 'test', 'version': '1'}}
    elif request['method'] == 'tools/call':
        result = {'content': [{'type': 'text', 'text': request['params']['arguments']['value']}]}
    else:
        result = {'tools': []}
    print(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/message', 'params': {}}), flush=True)
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'] + 1000, 'result': {'content': [{'type':'text', 'text':'stale'}]}}), flush=True)
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
"#;
        let client = Arc::new(
            McpClient::connect_stdio(
                "test",
                "python3",
                &["-u".into(), "-c".into(), server.into()],
            )
            .await
            .unwrap(),
        );
        let first = client.call_tool("echo", serde_json::json!({"value":"first"}));
        let second = client.call_tool("echo", serde_json::json!({"value":"second"}));
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.unwrap().text, "first");
        assert_eq!(second.unwrap().text, "second");
    }

    #[tokio::test]
    async fn initialized_notification_omits_null_params() {
        let server = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if request['method'] == 'notifications/initialized':
        if request.get('params', {}) is None: sys.exit(2)
        continue
    if request['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25', 'capabilities':{'tools':{}}, 'serverInfo':{'name':'test','version':'1'}}
    else:
        result = {'tools':[]}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
"#;
        let client = McpClient::connect_stdio(
            "test",
            "python3",
            &["-u".into(), "-c".into(), server.into()],
        )
        .await
        .unwrap();
        assert!(client.list_tools().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancelled_stdio_request_does_not_corrupt_the_next_response() {
        let server = r#"
import json, sys, time
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    if request['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25', 'capabilities':{}, 'serverInfo':{'name':'test','version':'1'}}
    else:
        value = request['params']['arguments']['value']
        result = {'content':[{'type':'text','text':value}]}
    response = json.dumps({'jsonrpc':'2.0', 'id':request['id'], 'result':result})
    if request['method'] != 'initialize' and value == 'slow':
        sys.stdout.write(response[:20]); sys.stdout.flush()
        time.sleep(0.3)
        response = response[20:]
    print(response, flush=True)
"#;
        let client = McpClient::connect_stdio(
            "test",
            "python3",
            &["-u".into(), "-c".into(), server.into()],
        )
        .await
        .unwrap();
        let slow = client.call_tool("echo", serde_json::json!({"value":"slow"}));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), slow)
                .await
                .is_err()
        );
        let fast = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            client.call_tool("echo", serde_json::json!({"value":"fast"})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(fast.text, "fast");
    }

    #[tokio::test]
    async fn cancelling_an_mcp_tool_returns_without_waiting_for_the_server() {
        use mew_tools::Tool;
        let server = r#"
import json, sys, time
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    if request['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25', 'capabilities':{}, 'serverInfo':{'name':'test','version':'1'}}
    else:
        time.sleep(1)
        result = {'content':[{'type':'text','text':'done'}]}
    print(json.dumps({'jsonrpc':'2.0', 'id':request['id'], 'result':result}), flush=True)
"#;
        let client = Arc::new(
            McpClient::connect_stdio(
                "test",
                "python3",
                &["-u".into(), "-c".into(), server.into()],
            )
            .await
            .unwrap(),
        );
        let tool = McpTool::new(
            "test",
            &McpToolDef {
                name: "slow".into(),
                description: String::new(),
                input_schema: serde_json::json!({"type":"object"}),
            },
            client,
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "call".into(),
            Default::default(),
            tx,
        );
        let cancel = ctx.cancel.clone();
        let task = tokio::spawn(async move { tool.execute(ctx, serde_json::json!({})).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_millis(400), task)
            .await
            .expect("tool did not cancel")
            .unwrap();
        assert!(result.is_err() || !result.unwrap().error.is_empty());
    }

    #[tokio::test]
    async fn cancelling_a_stdio_tool_notifies_the_server() {
        use mew_tools::Tool;
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("cancelled");
        let server = r#"
import json, pathlib, sys
for line in sys.stdin:
    request = json.loads(line)
    if request['method'] == 'notifications/cancelled':
        pathlib.Path(sys.argv[1]).write_text(str(request['params']['requestId']))
    elif request['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25', 'capabilities':{}, 'serverInfo':{'name':'test','version':'1'}}
        print(json.dumps({'jsonrpc':'2.0', 'id':request['id'], 'result':result}), flush=True)
"#;
        let client = Arc::new(
            McpClient::connect_stdio(
                "test",
                "python3",
                &[
                    "-u".into(),
                    "-c".into(),
                    server.into(),
                    marker.to_string_lossy().into_owned(),
                ],
            )
            .await
            .unwrap(),
        );
        let tool = McpTool::new(
            "test",
            &McpToolDef {
                name: "slow".into(),
                description: String::new(),
                input_schema: serde_json::json!({"type":"object"}),
            },
            client.clone(),
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "call".into(),
            Default::default(),
            tx,
        );
        let cancel = ctx.cancel.clone();
        let task = tokio::spawn(async move { tool.execute(ctx, serde_json::json!({})).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();
        assert!(task.await.unwrap().is_err());
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !marker.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("server did not receive cancellation");
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "2");
    }

    #[tokio::test]
    async fn http_sse_returns_the_matching_response_before_the_stream_closes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).await.unwrap();
            let events = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"value\":\"wrong\"}}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"value\":\"right\"}}\n\n";
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream
                .write_all(format!("{:X}\r\n{}\r\n", events.len(), events).as_bytes())
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });
        let transport =
            HttpTransport::new(format!("http://{address}"), &Default::default()).unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            transport.call("test", None),
        )
        .await
        .expect("SSE response waited for stream close")
        .unwrap();
        assert_eq!(result["value"], "right");
        server.abort();
    }

    #[tokio::test]
    async fn http_transport_sends_configured_headers() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let headers = String::from_utf8_lossy(&request[..count]).to_lowercase();
            assert!(headers.contains("authorization: bearer example"));
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        let headers =
            std::collections::HashMap::from([("Authorization".into(), "Bearer example".into())]);
        let transport = HttpTransport::new(format!("http://{address}"), &headers).unwrap();
        assert_eq!(transport.call("test", None).await.unwrap()["ok"], true);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_errors_do_not_expose_server_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                stream.read(&mut request).await.unwrap();
                let body = "credential-in-server-error";
                let reply = format!("HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let transport = HttpTransport::new(
            format!("http://{address}/?token=credential-in-url"),
            &Default::default(),
        )
        .unwrap();
        for error in [
            transport.call("test", None).await.unwrap_err(),
            transport.notify("test", None).await.unwrap_err(),
        ] {
            let message = error.to_string();
            assert!(message.contains("401"));
            assert!(!message.contains("credential"), "{message}");
        }
        server.await.unwrap();
        let error = transport.call("test", None).await.unwrap_err().to_string();
        assert!(!error.contains("credential"), "{error}");
    }

    #[tokio::test]
    async fn cancelling_an_http_call_notifies_the_server() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn read_request(stream: &mut tokio::net::TcpStream) -> Value {
            let mut data = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                data.extend_from_slice(&buffer[..count]);
                if let Some(end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if data.len() >= end + 4 + length {
                        return serde_json::from_slice(&data[end + 4..end + 4 + length]).unwrap();
                    }
                }
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            assert_eq!(read_request(&mut first).await["id"], 1);
            let (mut second, _) = listener.accept().await.unwrap();
            let notification = read_request(&mut second).await;
            second
                .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            notification
        });
        let transport =
            HttpTransport::new(format!("http://{address}"), &Default::default()).unwrap();
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(100),
            transport.call("test", None)
        )
        .await
        .is_err());
        let notification = tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("server did not receive cancellation")
            .unwrap();
        assert_eq!(notification["method"], "notifications/cancelled");
        assert_eq!(notification["params"]["requestId"], 1);
    }

    /// Live integration test — connect, handshake, list tools, call one.
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_context7_live() {
        let _ = tracing_subscriber::fmt::try_init();

        let url = "https://mcp.context7.com/mcp";
        eprintln!("connecting to {url}...");

        let mut headers = std::collections::HashMap::new();
        if let Ok(key) = std::env::var("CONTEXT7_API_KEY") {
            headers.insert("Authorization".to_string(), format!("Bearer {key}"));
        }
        let client = McpClient::connect_http_with_headers("context7", url, &headers)
            .await
            .expect("connect");

        eprintln!("connected, listing tools...");
        let tools = client.list_tools().await.expect("list tools");

        eprintln!("found {} tools:", tools.len());
        for t in &tools {
            eprintln!("  {} — {}", t.name, t.description);
        }

        assert!(!tools.is_empty(), "expected at least one tool");

        let resolver = tools
            .iter()
            .find(|tool| tool.name.contains("resolve"))
            .expect("library resolver");
        let result = client
            .call_tool(
                &resolver.name,
                serde_json::json!({"libraryName":"react", "query":"React useState hook"}),
            )
            .await
            .expect("resolve library");
        assert!(
            !result.is_error,
            "resolver returned an error: {}",
            result.text
        );
        assert!(!result.text.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_grep_app_live() {
        let client = McpClient::connect_http("grep", "https://mcp.grep.app")
            .await
            .expect("connect");
        let tools = client.list_tools().await.expect("list tools");
        let search = tools
            .iter()
            .find(|tool| tool.name.to_lowercase().contains("search"))
            .expect("search tool");
        let result = client
            .call_tool(&search.name, serde_json::json!({"query":"useState("}))
            .await
            .expect("search");
        assert!(!result.is_error, "search failed: {}", result.text);
        assert!(!result.text.is_empty());
    }

    /// Live stdio test — spawns the official MCP filesystem server,
    /// lists tools, and reads a test file. Run with:
    ///
    /// ```sh
    /// cargo test -p mew-mcp -- --ignored --nocapture test_stdio_live
    /// ```
    #[tokio::test]
    #[ignore = "requires npx and network access"]
    async fn test_stdio_live() {
        let _ = tracing_subscriber::fmt::try_init();

        let tmp = tempfile::tempdir().expect("tempdir");
        let allowed = std::fs::canonicalize(tmp.path())
            .expect("canonicalize")
            .to_string_lossy()
            .to_string();
        eprintln!("stdio test dir: {allowed}");

        let env = std::collections::HashMap::from([
            (
                "npm_config_cache".to_string(),
                tmp.path().join("npm-cache").to_string_lossy().into_owned(),
            ),
            (
                "NPM_CONFIG_USERCONFIG".to_string(),
                tmp.path().join("npmrc").to_string_lossy().into_owned(),
            ),
            (
                "NPM_CONFIG_GLOBALCONFIG".to_string(),
                tmp.path()
                    .join("global-npmrc")
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "NPM_CONFIG_UPDATE_NOTIFIER".to_string(),
                "false".to_string(),
            ),
        ]);
        let client = McpClient::connect_stdio_with_env(
            "filesystem",
            "npx",
            &[
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
                allowed.clone(),
            ],
            &env,
            Some(tmp.path()),
        )
        .await
        .expect("connect");

        eprintln!("connected, listing tools...");
        let tools = client.list_tools().await.expect("list tools");
        eprintln!("found {} tools:", tools.len());
        for t in &tools {
            eprintln!("  {} — {}", t.name, t.description);
        }
        assert!(!tools.is_empty(), "expected at least one tool");

        // Write a test file, then read it back.
        let test_path = format!("{}/hello.txt", allowed);
        eprintln!("\nwriting test file at {test_path}...");
        let result = client
            .call_tool(
                "write_file",
                serde_json::json!({
                    "path": &test_path,
                    "content": "hello world from mew mcp test\n"
                }),
            )
            .await
            .expect("write file");
        assert!(!result.is_error, "write failed: {}", result.text);

        eprintln!("\nreading test file...");
        let result = client
            .call_tool("read_text_file", serde_json::json!({"path": &test_path}))
            .await
            .expect("read file");
        assert!(!result.is_error, "read failed: {}", result.text);
        assert!(result.text.contains("hello world"), "file content mismatch");
    }
}
