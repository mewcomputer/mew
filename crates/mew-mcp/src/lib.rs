use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, info};

pub const MCP_DISCOVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const STDIO_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const TOOL_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

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
    async fn call_dispatched(
        &self,
        method: &str,
        params: Option<Value>,
        dispatched: Arc<AtomicBool>,
    ) -> Result<Value, McpError> {
        dispatched.store(true, Ordering::Release);
        self.call(method, params).await
    }
    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError>;
    fn set_protocol_version(&self, _version: &str) {
        // Default no-op for transports that don't need version negotiation.
    }
    fn is_broken(&self) -> bool {
        false
    }
    async fn retire(&self) {}
    async fn close(&self) -> Result<(), McpError>;
}

// ---------------------------------------------------------------------------
// HTTP transport (streamable HTTP)
// ---------------------------------------------------------------------------

/// Parse an SSE (Server-Sent Events) response from an MCP server.
async fn parse_sse_response(
    transport: &HttpTransport,
    mut resp: reqwest::Response,
    id: u64,
) -> Result<Value, McpError> {
    let mut buffer = Vec::new();
    let mut data_buffer = String::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| McpError::Transport(format!("read sse body: {}", e.without_url())))?
    {
        buffer.extend_from_slice(&chunk);
        if buffer.len() + data_buffer.len() > MAX_MESSAGE_BYTES {
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
                if let Some(result) = transport.handle_sse_data(&data_buffer, id).await? {
                    return Ok(result);
                }
                data_buffer.clear();
            }
        }
        buffer.drain(..consumed);
    }
    if !data_buffer.is_empty() {
        if let Some(result) = transport.handle_sse_data(&data_buffer, id).await? {
            return Ok(result);
        }
    }

    Err(McpError::Protocol("no data in SSE response".into()))
}

fn server_request_reply(message: &Value) -> Option<Value> {
    let method = message.get("method")?.as_str();
    let id = message.get("id")?;
    Some(if method == Some("ping") {
        serde_json::json!({"jsonrpc":"2.0", "id":id, "result":{}})
    } else {
        serde_json::json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601,"message":"method not supported"}})
    })
}

/// Try to parse accumulated SSE data as a JSON-RPC response. Returns
/// `Some(Ok(...))` for a valid response, `Some(Err(...))` for a fatal error,
/// or `None` to skip this event (e.g. it was a progress notification).
fn try_parse_sse_data(parsed: Value, id: u64) -> Option<Result<Value, McpError>> {
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
    close_lock: tokio::sync::Mutex<Option<String>>,
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
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 10 {
                        attempt.error("too many MCP redirects")
                    } else if attempt
                        .previous()
                        .first()
                        .is_some_and(|original| original.origin() == attempt.url().origin())
                    {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .default_headers(defaults)
                .build()?,
            url,
            request_id: AtomicU64::new(1),
            protocol_version: StdMutex::new(MAX_PROTOCOL_VERSION.into()),
            session_id: StdMutex::new(None),
            close_lock: tokio::sync::Mutex::new(None),
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

    async fn handle_sse_data(&self, data: &str, id: u64) -> Result<Option<Value>, McpError> {
        let message: Value = match serde_json::from_str(data) {
            Ok(message) => message,
            Err(_) => return Ok(None),
        };
        if let Some(reply) = server_request_reply(&message) {
            let body =
                serde_json::to_vec(&reply).map_err(|e| McpError::Transport(e.to_string()))?;
            let response = self.build_request(body).send().await?;
            if !response.status().is_success() {
                return Err(McpError::Transport(format!(
                    "server request reply failed: HTTP {}",
                    response.status()
                )));
            }
            return Ok(None);
        }
        try_parse_sse_data(message, id).transpose()
    }

    fn take_close_request(&self) -> Option<reqwest::RequestBuilder> {
        let id = self.session_id.lock().unwrap().take()?;
        let version = self.protocol_version.lock().unwrap().clone();
        Some(
            self.client
                .delete(&self.url)
                .header("MCP-Session-Id", id)
                .header("MCP-Protocol-Version", version),
        )
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        if let (Some(request), Ok(runtime)) = (
            self.take_close_request(),
            tokio::runtime::Handle::try_current(),
        ) {
            runtime.spawn(async move {
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(3), request.send()).await;
            });
        }
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
                return parse_sse_response(self, resp, id).await;
            }

            if resp
                .content_length()
                .is_some_and(|length| length > MAX_MESSAGE_BYTES as u64)
            {
                return Err(McpError::Protocol("HTTP response too large".into()));
            }
            let mut resp = resp;
            let mut data = Vec::new();
            while let Some(chunk) = resp.chunk().await? {
                if chunk.len() > MAX_MESSAGE_BYTES.saturating_sub(data.len()) {
                    return Err(McpError::Protocol("HTTP response too large".into()));
                }
                data.extend_from_slice(&chunk);
            }
            let response: JsonRpcResponse = serde_json::from_slice(&data)
                .map_err(|e| McpError::Transport(format!("parse response: {e}")))?;

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

    async fn close(&self) -> Result<(), McpError> {
        let mut previous_error = self.close_lock.lock().await;
        if let Some(request) = self.take_close_request() {
            let result = async {
                let response =
                    tokio::time::timeout(std::time::Duration::from_secs(3), request.send())
                        .await
                        .map_err(|_| McpError::Transport("session close timed out".into()))??;
                if !response.status().is_success()
                    && response.status() != reqwest::StatusCode::METHOD_NOT_ALLOWED
                {
                    return Err(McpError::Transport(format!(
                        "session close failed: HTTP {}",
                        response.status()
                    )));
                }
                Ok(())
            }
            .await;
            *previous_error = result.as_ref().err().map(ToString::to_string);
            return result;
        }
        match previous_error.as_ref() {
            Some(error) => Err(McpError::Transport(error.clone())),
            None => Ok(()),
        }
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
    pending: Arc<StdMutex<Option<StdioPending>>>,
    reader_task: tokio::task::JoinHandle<()>,
    call_lock: Arc<tokio::sync::Mutex<()>>,
    request_id: AtomicU64,
    broken: Arc<AtomicBool>,
    retired: AtomicBool,
}

type StdioPending = (
    u64,
    tokio::sync::oneshot::Sender<Result<JsonRpcResponse, McpError>>,
);

async fn read_stdio(
    stdout: tokio::process::ChildStdout,
    stdin: Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>,
    pending: Arc<StdMutex<Option<StdioPending>>>,
    broken: Arc<AtomicBool>,
) {
    let result: Result<(), McpError> = async {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            let count = (&mut reader)
                .take(MAX_MESSAGE_BYTES as u64 + 1)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|error| McpError::Transport(format!("read stdout: {error}")))?;
            if count == 0 {
                return Err(McpError::Transport("server closed stdout".into()));
            }
            if line.len() > MAX_MESSAGE_BYTES || line.last() != Some(&b'\n') {
                return Err(McpError::Protocol(
                    "stdio response too large or incomplete".into(),
                ));
            }
            let message: Value = serde_json::from_slice(&line)
                .map_err(|error| McpError::Transport(format!("parse response: {error}")))?;
            if message.get("method").is_some() {
                if let Some(reply) = server_request_reply(&message) {
                    let mut body = serde_json::to_vec(&reply)
                        .map_err(|error| McpError::Transport(error.to_string()))?;
                    body.push(b'\n');
                    tokio::time::timeout(STDIO_WRITE_TIMEOUT, async {
                        let mut guard = stdin.lock().await;
                        let stdin = guard
                            .as_mut()
                            .ok_or_else(|| McpError::Transport("stdin closed".into()))?;
                        stdin
                            .write_all(&body)
                            .await
                            .map_err(|error| McpError::Transport(format!("write stdin: {error}")))
                    })
                    .await
                    .map_err(|_| McpError::Transport("write stdin timed out".into()))??;
                }
                continue;
            }
            let response: JsonRpcResponse = serde_json::from_value(message)
                .map_err(|error| McpError::Transport(format!("parse response: {error}")))?;
            let sender = {
                let mut pending = pending.lock().unwrap();
                if pending
                    .as_ref()
                    .is_some_and(|(id, _)| response.id == Some(*id))
                {
                    pending.take().map(|(_, sender)| sender)
                } else {
                    None
                }
            };
            if let Some(sender) = sender {
                let _ = sender.send(Ok(response));
            }
        }
    }
    .await;
    let sender = {
        let mut pending = pending.lock().unwrap();
        broken.store(true, Ordering::Release);
        pending.take().map(|(_, sender)| sender)
    };
    if let (Some(sender), Err(error)) = (sender, result) {
        let _ = sender.send(Err(error));
    }
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

        let stdin = Arc::new(tokio::sync::Mutex::new(Some(stdin)));
        let pending = Arc::new(StdMutex::new(None));
        let broken = Arc::new(AtomicBool::new(false));
        let reader_task = tokio::spawn(read_stdio(
            stdout,
            stdin.clone(),
            pending.clone(),
            broken.clone(),
        ));
        Ok(Self {
            child: tokio::sync::Mutex::new(child),
            #[cfg(unix)]
            process_group,
            stdin,
            pending,
            reader_task,
            call_lock: Arc::new(tokio::sync::Mutex::new(())),
            request_id: AtomicU64::new(1),
            broken,
            retired: AtomicBool::new(false),
        })
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        self.reader_task.abort();
        #[cfg(unix)]
        if let Some(pid) = self
            .process_group
            .filter(|_| !self.retired.swap(true, Ordering::AcqRel))
        {
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
    broken: Arc<AtomicBool>,
    pending: Arc<StdMutex<Option<StdioPending>>>,
}

impl Drop for StdioCancellation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.as_ref().is_some_and(|(id, _)| *id == self.id) {
                pending.take();
            }
        }
        let stdin = self.stdin.clone();
        let call_guard = self.call_guard.take();
        let broken = self.broken.clone();
        if broken.swap(true, Ordering::AcqRel) {
            return;
        }
        let message = serde_json::json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":self.id,"reason":"cancelled by client"}}).to_string() + "\n";
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _call_guard = call_guard;
                let result = tokio::time::timeout(STDIO_WRITE_TIMEOUT, async {
                    let mut guard = stdin.lock().await;
                    let stdin = guard
                        .as_mut()
                        .ok_or_else(|| std::io::Error::other("stdin closed"))?;
                    stdin.write_all(message.as_bytes()).await
                })
                .await;
                if !matches!(result, Ok(Ok(()))) {
                    broken.store(true, Ordering::Release);
                }
            });
        }
    }
}

impl StdioTransport {
    async fn call_marked(
        &self,
        method: &str,
        params: Option<Value>,
        dispatched: Option<Arc<AtomicBool>>,
    ) -> Result<Value, McpError> {
        let call_guard = self.call_lock.clone().lock_owned().await;
        if self.is_broken() {
            return Err(McpError::Transport("stdio connection is unusable".into()));
        }
        if let Some(dispatched) = dispatched {
            dispatched.store(true, Ordering::Release);
        }
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

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if self.is_broken() {
                return Err(McpError::Transport("stdio connection is unusable".into()));
            }
            *pending = Some((id, response_tx));
        }
        // Finish a frame even if the caller is cancelled during a write.
        let mut guard = self.stdin.clone().lock_owned().await;
        let mut cancellation = StdioCancellation {
            stdin: self.stdin.clone(),
            id,
            armed: true,
            call_guard: Some(call_guard),
            broken: self.broken.clone(),
            pending: self.pending.clone(),
        };
        let broken = self.broken.clone();
        tokio::spawn(async move {
            let stdin = guard
                .as_mut()
                .ok_or_else(|| McpError::Transport("stdin closed".into()))?;
            let result =
                tokio::time::timeout(STDIO_WRITE_TIMEOUT, stdin.write_all(body.as_bytes())).await;
            if !matches!(result, Ok(Ok(()))) {
                broken.store(true, Ordering::Release);
            }
            match result {
                Ok(result) => result.map_err(|e| McpError::Transport(format!("write stdin: {e}"))),
                Err(_) => Err(McpError::Transport("write stdin timed out".into())),
            }
        })
        .await
        .map_err(|error| McpError::Transport(format!("write task: {error}")))??;

        let response = response_rx
            .await
            .map_err(|_| McpError::Transport("stdio response reader stopped".into()))??;
        cancellation.armed = false;
        if let Some(err) = response.error {
            return Err(McpError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        response
            .result
            .ok_or_else(|| McpError::Protocol("missing result in stdio response".into()))
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        self.call_marked(method, params, None).await
    }

    async fn call_dispatched(
        &self,
        method: &str,
        params: Option<Value>,
        dispatched: Arc<AtomicBool>,
    ) -> Result<Value, McpError> {
        self.call_marked(method, params, Some(dispatched)).await
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

    async fn close(&self) -> Result<(), McpError> {
        self.stdin.lock().await.take();
        let mut child = self.child.lock().await;
        #[cfg(unix)]
        if let Some(pid) = self
            .process_group
            .filter(|_| !self.retired.swap(true, Ordering::AcqRel))
        {
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

    fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    async fn retire(&self) {
        self.broken.store(true, Ordering::Release);
        let first = !self.retired.swap(true, Ordering::AcqRel);
        #[cfg(unix)]
        if let Some(pid) = self.process_group.filter(|_| first) {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
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
    connect_tools_until(configs, cwd, std::future::pending()).await
}

/// Discover tools until shutdown is requested, then stop unfinished servers.
pub async fn connect_tools_until(
    configs: Vec<McpServerConfig>,
    cwd: &std::path::Path,
    shutdown: impl std::future::Future<Output = ()>,
) -> Vec<Arc<dyn mew_tools::Tool>> {
    connect_tools_with_timeout(configs, cwd, shutdown, MCP_DISCOVERY_TIMEOUT).await
}

async fn connect_tools_with_timeout(
    configs: Vec<McpServerConfig>,
    cwd: &std::path::Path,
    shutdown: impl std::future::Future<Output = ()>,
    timeout: std::time::Duration,
) -> Vec<Arc<dyn mew_tools::Tool>> {
    tokio::pin!(shutdown);
    let mut tasks = tokio::task::JoinSet::new();
    let (cancel, _) = tokio::sync::watch::channel(false);
    for config in configs {
        if config.disabled {
            continue;
        }
        let cwd = cwd.to_path_buf();
        let mut task_shutdown = cancel.subscribe();
        tasks.spawn(async move {
            let name = config.name.clone();
            let deadline = tokio::time::Instant::now() + timeout;
            let mut init_shutdown = task_shutdown.clone();
            let result = match McpClient::connect_configured_until(
                &config, &cwd,
                async move { let _ = init_shutdown.wait_for(|shutdown| *shutdown).await; },
                Some(deadline),
            ).await {
                Ok(client) => {
                    let client = Arc::new(client);
                    let listing = tokio::select! {
                        biased;
                        _ = task_shutdown.wait_for(|shutdown| *shutdown) => Ok(Err(McpError::Transport("server startup cancelled".into()))),
                        result = tokio::time::timeout_at(deadline, client.list_tools()) => result,
                    };
                    match listing {
                        Ok(Ok(definitions)) => Ok(Ok((
                            Arc::new(ManagedClient::configured(client, config, cwd)),
                            definitions,
                        ))),
                        result => {
                            let _ = client.shutdown().await;
                            match result {
                                Ok(Err(error)) => Ok(Err(error)),
                                Err(error) => Err(error),
                                Ok(Ok(_)) => unreachable!(),
                            }
                        }
                    }
                }
                Err(error) => Ok(Err(error)),
            };
            (name, result)
        });
    }
    let mut connected: Vec<(String, Arc<ManagedClient>, Vec<McpToolDef>)> = Vec::new();
    loop {
        let result = tokio::select! {
            _ = &mut shutdown => {
                cancel.send_replace(true);
                let mut closing = tokio::task::JoinSet::new();
                for (_, client, _) in connected {
                    closing.spawn(async move { let _ = client.shutdown().await; });
                }
                while let Some(result) = tasks.join_next().await {
                    if let Ok((_, Ok(Ok((client, _))))) = result {
                        closing.spawn(async move { let _ = client.shutdown().await; });
                    }
                }
                while closing.join_next().await.is_some() {}
                return Vec::new();
            }
            result = tasks.join_next() => result,
        };
        let Some(result) = result else { break };
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
    let mut unused = tokio::task::JoinSet::new();
    for (name, client, mut definitions) in connected {
        let previous_count = tools.len();
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
            tools.push(Arc::new(McpTool::with_client(
                &name,
                &definition,
                client.clone(),
            )));
        }
        if tools.len() == previous_count {
            unused.spawn(async move {
                let _ = client.shutdown().await;
            });
        }
    }
    while unused.join_next().await.is_some() {}
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

async fn wait_for_deadline(deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    } else {
        std::future::pending::<()>().await;
    }
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
        Self::connect_http_until(name, url, headers, std::future::pending(), None).await
    }

    async fn connect_http_until(
        name: &str,
        url: &str,
        headers: &std::collections::HashMap<String, String>,
        shutdown: impl std::future::Future<Output = ()> + Send,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<Self, McpError> {
        let transport = HttpTransport::new(url.to_string(), headers)?;
        let mut client = Self {
            name: name.to_string(),
            transport: Box::new(transport),
        };
        client.initialize_until(shutdown, deadline).await?;
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
        Self::connect_stdio_until(name, command, args, env, cwd, std::future::pending(), None).await
    }

    async fn connect_stdio_until(
        name: &str,
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
        cwd: Option<&std::path::Path>,
        shutdown: impl std::future::Future<Output = ()> + Send,
        deadline: Option<tokio::time::Instant>,
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
        client
            .initialize_until(shutdown, deadline)
            .await
            .map_err(|error| McpError::Init(error.to_string()))?;
        Ok(client)
    }

    /// Connect using an entry from `mcp.json`.
    pub async fn connect_configured(
        config: &McpServerConfig,
        cwd: &std::path::Path,
    ) -> Result<Self, McpError> {
        Self::connect_configured_until(config, cwd, std::future::pending(), None).await
    }

    async fn connect_configured_until(
        config: &McpServerConfig,
        cwd: &std::path::Path,
        shutdown: impl std::future::Future<Output = ()> + Send,
        deadline: Option<tokio::time::Instant>,
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
                Self::connect_http_until(&config.name, url, &config.headers, shutdown, deadline)
                    .await
            }
            (None | Some("stdio"), None, Some(command)) if config.headers.is_empty() => {
                Self::connect_stdio_until(
                    &config.name,
                    command,
                    &config.args,
                    &config.env,
                    Some(cwd),
                    shutdown,
                    deadline,
                )
                .await
            }
            _ => Err(McpError::Config(format!(
                "server {:?} has an invalid transport configuration",
                config.name
            ))),
        }
    }

    async fn initialize_until(
        &mut self,
        shutdown: impl std::future::Future<Output = ()> + Send,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<(), McpError> {
        let result = tokio::select! {
            biased;
            _ = shutdown => Err(McpError::Transport("server startup cancelled".into())),
            _ = wait_for_deadline(deadline) => Err(McpError::Transport("server startup timed out".into())),
            result = self.initialize() => result,
        };
        if result.is_err() {
            let _ = self.shutdown().await;
        }
        result
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
        self.call_tool_marked(tool_name, arguments, None).await
    }

    async fn call_tool_marked(
        &self,
        tool_name: &str,
        arguments: Value,
        dispatched: Option<Arc<AtomicBool>>,
    ) -> Result<ToolCallResult, McpError> {
        let params = Some(serde_json::json!({"name":tool_name, "arguments":arguments}));
        let result = match dispatched {
            Some(dispatched) => {
                self.transport
                    .call_dispatched("tools/call", params, dispatched)
                    .await?
            }
            None => self.transport.call("tools/call", params).await?,
        };

        let content = result.get("content").and_then(|v| v.as_array());
        let text = content
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let structured_content = result.get("structuredContent").cloned();
        let content = content.cloned().unwrap_or_default();
        let images = content
            .iter()
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
            content,
        })
    }

    /// Shut down the server connection.
    pub async fn shutdown(&self) -> Result<(), McpError> {
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
    pub content: Vec<Value>,
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
    client: Arc<ManagedClient>,
}

/// A server shared by its tools. A failed call is returned to its caller;
/// a later call reconnects rather than replaying a potentially mutating call.
struct ManagedClient {
    current: tokio::sync::RwLock<Arc<McpClient>>,
    reconnect: Option<(McpServerConfig, std::path::PathBuf)>,
    reconnect_lock: tokio::sync::Mutex<()>,
    invalid: AtomicBool,
    closed: tokio::sync::watch::Sender<bool>,
    active_calls: tokio::sync::RwLock<()>,
}

impl ManagedClient {
    fn new(client: Arc<McpClient>) -> Self {
        Self {
            current: tokio::sync::RwLock::new(client),
            reconnect: None,
            reconnect_lock: tokio::sync::Mutex::new(()),
            invalid: AtomicBool::new(false),
            closed: tokio::sync::watch::channel(false).0,
            active_calls: tokio::sync::RwLock::new(()),
        }
    }

    fn configured(
        client: Arc<McpClient>,
        config: McpServerConfig,
        cwd: std::path::PathBuf,
    ) -> Self {
        Self {
            reconnect: Some((config, cwd)),
            ..Self::new(client)
        }
    }

    async fn call_tool_cancellable(
        &self,
        name: &str,
        arguments: Value,
        cancel: impl std::future::Future<Output = ()> + Send,
    ) -> Result<ToolCallResult, McpError> {
        self.call_tool_until(name, arguments, TOOL_CALL_TIMEOUT, cancel)
            .await
    }

    #[cfg(test)]
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolCallResult, McpError> {
        self.call_tool_with_timeout(name, arguments, TOOL_CALL_TIMEOUT)
            .await
    }

    async fn shutdown(&self) -> Result<(), McpError> {
        self.closed.send_replace(true);
        let original = self.current.read().await.clone();
        let close_original = original.shutdown();
        let close_replacement = async {
            let _active = self.active_calls.write().await;
            let _guard = self.reconnect_lock.lock().await;
            let current = self.current.read().await.clone();
            current.shutdown().await
        };
        let (original, replacement) = tokio::join!(close_original, close_replacement);
        original.and(replacement)
    }

    #[cfg(test)]
    async fn call_tool_with_timeout(
        &self,
        name: &str,
        arguments: Value,
        timeout: std::time::Duration,
    ) -> Result<ToolCallResult, McpError> {
        self.call_tool_until(name, arguments, timeout, std::future::pending())
            .await
    }

    async fn call_tool_until(
        &self,
        name: &str,
        arguments: Value,
        timeout: std::time::Duration,
        cancel: impl std::future::Future<Output = ()> + Send,
    ) -> Result<ToolCallResult, McpError> {
        let timed_out = AtomicBool::new(false);
        let cancel = async {
            tokio::select! {
                _ = cancel => {}
                _ = tokio::time::sleep(timeout) => timed_out.store(true, Ordering::Release),
            }
        };
        tokio::pin!(cancel);
        let dispatched = Arc::new(AtomicBool::new(false));
        let selected = StdMutex::new(None::<Arc<McpClient>>);
        let operation = async {
            let _active = tokio::select! {
                biased;
                _ = &mut cancel => return Err(McpError::Transport("MCP tool cancelled".into())),
                active = self.active_calls.read() => active,
            };
            if *self.closed.borrow() {
                return Err(McpError::Transport("MCP connection is shut down".into()));
            }
            let current = self.current.read().await.clone();
            if let Some((config, cwd)) = self
                .reconnect
                .as_ref()
                .filter(|_| self.invalid.load(Ordering::Acquire) || current.transport.is_broken())
            {
                let _guard = tokio::select! {
                    biased;
                    _ = &mut cancel => return Err(McpError::Transport("MCP tool cancelled".into())),
                    guard = self.reconnect_lock.lock() => guard,
                };
                if *self.closed.borrow() {
                    return Err(McpError::Transport("MCP connection is shut down".into()));
                }
                let current = self.current.read().await.clone();
                if self.invalid.load(Ordering::Acquire) || current.transport.is_broken() {
                    current.transport.retire().await;
                    let mut reconnect_shutdown = self.closed.subscribe();
                    let client = McpClient::connect_configured_until(
                        config,
                        cwd,
                        async {
                            tokio::select! {
                                _ = &mut cancel => {}
                                _ = reconnect_shutdown.wait_for(|closed| *closed) => {}
                            }
                        },
                        Some(tokio::time::Instant::now() + MCP_DISCOVERY_TIMEOUT),
                    )
                    .await?;
                    if *self.closed.borrow() {
                        let _ = client.shutdown().await;
                        return Err(McpError::Transport("MCP connection is shut down".into()));
                    }
                    let mut current = self.current.write().await;
                    *current = Arc::new(client);
                    self.invalid.store(false, Ordering::Release);
                }
            }
            let client = self.current.read().await.clone();
            if *self.closed.borrow() {
                return Err(McpError::Transport("MCP connection is shut down".into()));
            }
            *selected.lock().unwrap() = Some(client.clone());
            let mut closed = self.closed.subscribe();
            tokio::select! {
                biased;
                _ = &mut cancel => Err(McpError::Transport("MCP tool cancelled".into())),
                _ = closed.wait_for(|closed| *closed) => Err(McpError::Transport("MCP connection is shut down".into())),
                result = client.call_tool_marked(name, arguments, Some(dispatched.clone())) => result,
            }
        };
        let result = operation.await;
        let expired = timed_out.load(Ordering::Acquire);
        let invalidate = dispatched.load(Ordering::Acquire)
            && matches!(
                &result,
                Err(McpError::Transport(_) | McpError::Http(_) | McpError::Protocol(_))
            );
        let result = if expired {
            Err(McpError::Transport("tool call timed out".into()))
        } else {
            result
        };
        let selected = selected.into_inner().unwrap();
        if let Some(client) = selected.filter(|_| invalidate) {
            let current = self.current.read().await;
            if Arc::ptr_eq(&current, &client) {
                self.invalid.store(true, Ordering::Release);
            }
        }
        result
    }
}

impl McpTool {
    pub fn new(server_name: &str, def: &McpToolDef, client: Arc<McpClient>) -> Self {
        Self::with_client(server_name, def, Arc::new(ManagedClient::new(client)))
    }

    fn with_client(server_name: &str, def: &McpToolDef, client: Arc<ManagedClient>) -> Self {
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
        mew_tools::Sensitivity::Dangerous
    }

    async fn shutdown(&self) -> Result<(), mew_tools::ToolError> {
        self.client
            .shutdown()
            .await
            .map_err(|error| mew_tools::ToolError::Execution(error.to_string()))
    }

    async fn execute(
        &self,
        ctx: mew_tools::ToolCtx,
        input: Value,
    ) -> Result<mew_hooks::ToolOutput, mew_tools::ToolError> {
        let cancel = ctx.cancel.clone();
        let result = self
            .client
            .call_tool_cancellable(&self.tool_name, input, async move {
                cancel.cancelled().await;
            })
            .await;
        if ctx.cancel.is_cancelled() {
            return Err(mew_tools::ToolError::Execution("MCP tool cancelled".into()));
        }
        match result {
            Ok(mut result) => {
                if result.content.iter().any(|item| {
                    matches!(
                        item.get("type").and_then(Value::as_str),
                        Some("resource" | "resource_link")
                    )
                }) {
                    result.text = result
                        .content
                        .iter_mut()
                        .filter_map(|item| match item.get("type").and_then(Value::as_str) {
                            Some("text") => {
                                item.get("text").and_then(Value::as_str).map(str::to_owned)
                            }
                            Some("resource" | "resource_link") => {
                                redact_structured(item, &ctx.secrets);
                                serde_json::to_string(item).ok()
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                }
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
                    (
                        String::new(),
                        if text.is_empty() {
                            "MCP tool returned an error".into()
                        } else {
                            text
                        },
                    )
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
        assert_eq!(tools[0].sensitivity(), mew_tools::Sensitivity::Dangerous);
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

    #[tokio::test]
    async fn configured_stdio_server_reconnects_after_exiting_without_replaying_call() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("attempts");
        let server = r#"
import json, os, pathlib, sys
marker = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{'tools':{}},'serverInfo':{'name':'test','version':'1'}}
    elif req['method'] == 'tools/list':
        result = {'tools':[{'name':'write','inputSchema':{'type':'object'}}]}
    else:
        marker.write_text(marker.read_text() + 'call\n' if marker.exists() else 'call\n')
        if len(marker.read_text().splitlines()) == 1: os._exit(0)
        result = {'content':[{'type':'text','text':'completed'}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec![
                "-u".into(),
                "-c".into(),
                server.into(),
                marker.to_string_lossy().into(),
            ],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let tools = connect_tools(vec![config], directory.path()).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = || {
            mew_tools::ToolCtx::new(
                Arc::new(mew_tools::ToolCtxShared::default()),
                "call".into(),
                Default::default(),
                tx.clone(),
            )
        };
        let first = tools[0]
            .execute(ctx(), serde_json::json!({}))
            .await
            .unwrap();
        assert!(!first.error.is_empty());
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "call\n");
        let second = tools[0]
            .execute(ctx(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(second.output, "completed", "{}", second.error);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "call\ncall\n");
    }

    #[tokio::test]
    async fn cancelling_a_blocked_stdio_write_does_not_wedge_other_calls() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("stalled");
        let server = r#"
import json, pathlib, sys, time
marker = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{'tools':{}},'serverInfo':{'name':'test','version':'1'}}
    elif req['method'] == 'tools/list':
        result = {'tools':[{'name':'echo','inputSchema':{'type':'object'}}]}
        print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
        if not marker.exists():
            sys.stdin.buffer.read(1)
            marker.write_text('stalled')
            time.sleep(30)
        continue
    else:
        result = {'content':[{'type':'text','text':'recovered'}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec![
                "-u".into(),
                "-c".into(),
                server.into(),
                marker.to_string_lossy().into(),
            ],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let tools = connect_tools(vec![config], directory.path()).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "slow".into(),
            Default::default(),
            tx.clone(),
        );
        let cancel = ctx.cancel.clone();
        let tool = tools[0].clone();
        let slow = tokio::spawn(async move {
            tool.execute(ctx, serde_json::json!({"value":"x".repeat(2_000_000)}))
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !marker.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("server did not begin reading the request");
        cancel.cancel();
        assert!(slow.await.unwrap().is_err());
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "next".into(),
            Default::default(),
            tx,
        );
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tools[0].execute(ctx, serde_json::json!({})),
        )
        .await
        .expect("other calls were wedged")
        .unwrap();
        assert_eq!(result.output, "recovered", "{}", result.error);
    }

    #[tokio::test]
    async fn cancelling_an_unanswered_stdio_call_reconnects_on_the_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("request-read");
        let server = r#"
import json, pathlib, sys, time
marker = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    elif req['method'] == 'tools/list':
        result = {'tools':[{'name':'echo','inputSchema':{'type':'object'}}]}
    elif not marker.exists():
        marker.write_text('read')
        time.sleep(30)
        continue
    else:
        result = {'content':[{'type':'text','text':'recovered'}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec![
                "-u".into(),
                "-c".into(),
                server.into(),
                marker.to_string_lossy().into(),
            ],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let tools = connect_tools(vec![config], dir.path()).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "first".into(),
            Default::default(),
            tx.clone(),
        );
        let cancel = ctx.cancel.clone();
        let tool = tools[0].clone();
        let pending = tokio::spawn(async move { tool.execute(ctx, serde_json::json!({})).await });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !marker.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("server did not consume the call");
        cancel.cancel();
        assert!(pending.await.unwrap().is_err());
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared::default()),
            "next".into(),
            Default::default(),
            tx,
        );
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tools[0].execute(ctx, serde_json::json!({})),
        )
        .await
        .expect("next call was wedged")
        .unwrap();
        assert_eq!(output.output, "recovered", "{}", output.error);
    }

    #[tokio::test]
    async fn queued_stdio_call_does_not_timeout_or_retire_an_active_call() {
        let dir = tempfile::tempdir().unwrap();
        let started = dir.path().join("started");
        let release = dir.path().join("release");
        let server = r#"
import json, pathlib, sys, time
started, release = map(pathlib.Path, sys.argv[1:])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    else:
        value = req['params']['arguments']['value']
        if value == 'first':
            started.write_text('started')
            while not release.exists(): time.sleep(0.01)
        result = {'content':[{'type':'text','text':value}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec![
                "-u".into(),
                "-c".into(),
                server.into(),
                started.to_string_lossy().into(),
                release.to_string_lossy().into(),
            ],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let client = Arc::new(
            McpClient::connect_configured(&config, dir.path())
                .await
                .unwrap(),
        );
        let managed = Arc::new(ManagedClient::configured(
            client,
            config,
            dir.path().to_path_buf(),
        ));
        let mut first = {
            let managed = managed.clone();
            tokio::spawn(async move {
                managed
                    .call_tool_with_timeout(
                        "echo",
                        serde_json::json!({"value":"first"}),
                        std::time::Duration::from_secs(3),
                    )
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !started.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("first request did not start");
        let queued = {
            let managed = managed.clone();
            tokio::spawn(async move {
                managed
                    .call_tool_with_timeout(
                        "echo",
                        serde_json::json!({"value":"second"}),
                        std::time::Duration::from_millis(100),
                    )
                    .await
            })
        };
        assert!(
            queued.await.unwrap().is_err(),
            "the queued call should respect its total deadline"
        );
        let third = {
            let managed = managed.clone();
            tokio::spawn(async move {
                managed
                    .call_tool_with_timeout(
                        "echo",
                        serde_json::json!({"value":"third"}),
                        std::time::Duration::from_secs(3),
                    )
                    .await
            })
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(250), &mut first)
                .await
                .is_err(),
            "a queued timeout killed the active call"
        );
        std::fs::write(release, "go").unwrap();
        assert_eq!(first.await.unwrap().unwrap().text, "first");
        assert_eq!(third.await.unwrap().unwrap().text, "third");
    }

    #[tokio::test]
    async fn stdio_call_deadline_includes_reconnect_and_later_call_can_recover() {
        let dir = tempfile::tempdir().unwrap();
        let failed = dir.path().join("failed");
        let waiting = dir.path().join("waiting");
        let release = dir.path().join("release");
        let server = r#"
import json, pathlib, sys, time
failed, waiting, release = map(pathlib.Path, sys.argv[1:])
if failed.exists() and not release.exists():
    waiting.write_text('waiting')
    while not release.exists(): time.sleep(0.01)
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    elif not failed.exists():
        failed.write_text('failed')
        print('invalid-json',flush=True)
        continue
    else:
        result = {'content':[{'type':'text','text':'recovered'}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec![
                "-u".into(),
                "-c".into(),
                server.into(),
                failed.to_string_lossy().into(),
                waiting.to_string_lossy().into(),
                release.to_string_lossy().into(),
            ],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let client = Arc::new(
            McpClient::connect_configured(&config, dir.path())
                .await
                .unwrap(),
        );
        let managed = ManagedClient::configured(client, config, dir.path().to_path_buf());
        assert!(managed
            .call_tool("echo", serde_json::json!({}))
            .await
            .is_err());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            managed.call_tool_with_timeout(
                "echo",
                serde_json::json!({}),
                std::time::Duration::from_secs(2),
            ),
        )
        .await
        .expect("reconnect ignored the call deadline");
        assert!(result.is_err());
        assert!(waiting.exists(), "the replacement server never started");
        std::fs::write(release, "go").unwrap();
        assert_eq!(
            managed
                .call_tool("echo", serde_json::json!({}))
                .await
                .unwrap()
                .text,
            "recovered"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn broken_stdio_process_releases_exclusive_resources_before_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("server.lock");
        let marker = dir.path().join("first-call");
        let server = r#"
import fcntl, json, pathlib, sys, time
lock = open(sys.argv[1], 'w')
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
marker = pathlib.Path(sys.argv[2])
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    elif req['method'] == 'tools/list':
        result = {'tools':[{'name':'echo','inputSchema':{'type':'object'}}]}
    elif not marker.exists():
        marker.write_text('called')
        print('not json',flush=True)
        time.sleep(30)
        continue
    else:
        result = {'content':[{'type':'text','text':'recovered'}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec![
                "-u".into(),
                "-c".into(),
                server.into(),
                lock.to_string_lossy().into(),
                marker.to_string_lossy().into(),
            ],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let tools = connect_tools(vec![config], dir.path()).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = || {
            mew_tools::ToolCtx::new(
                Arc::new(mew_tools::ToolCtxShared::default()),
                "call".into(),
                Default::default(),
                tx.clone(),
            )
        };
        let first = tools[0]
            .execute(ctx(), serde_json::json!({}))
            .await
            .unwrap();
        assert!(!first.error.is_empty());
        let next = tools[0]
            .execute(ctx(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(next.output, "recovered", "{}", next.error);
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
        let complete = client
            .call_tool("echo", serde_json::json!({"value":"slow"}))
            .await
            .unwrap();
        assert_eq!(
            complete.text, "slow",
            "a complete split response must be reconstructed"
        );
        let directory = tempfile::tempdir().unwrap();
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec!["-u".into(), "-c".into(), server.into()],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let client = Arc::new(
            McpClient::connect_configured(&config, directory.path())
                .await
                .unwrap(),
        );
        let client = ManagedClient::configured(client, config, directory.path().to_path_buf());
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
    async fn blocked_server_request_reply_retires_stdio_connection() {
        let server = r#"
import json, sys, time
for line in sys.stdin:
    req = json.loads(line)
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
        print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
    elif req['method'] == 'tools/call':
        print(json.dumps({'jsonrpc':'2.0','id':'x' * 1_000_000,'method':'unsupported'}),flush=True)
        time.sleep(30)
"#;
        let client = McpClient::connect_stdio(
            "test",
            "python3",
            &["-u".into(), "-c".into(), server.into()],
        )
        .await
        .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            client.call_tool("test", serde_json::json!({})),
        )
        .await
        .expect("server request reply blocked indefinitely");
        assert!(result.is_err());
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.call_tool("test", serde_json::json!({})),
        )
        .await
        .expect("broken connection blocked the next call");
        assert!(next.is_err());
    }

    #[tokio::test]
    async fn cancelling_partial_server_request_reply_retires_stdio_connection() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("reply-started");
        let server = r#"
import json, pathlib, sys, time
for line in sys.stdin:
    req = json.loads(line)
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
        print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
    elif req['method'] == 'tools/call':
        print(json.dumps({'jsonrpc':'2.0','id':'x' * 1_000_000,'method':'unsupported'}),flush=True)
        sys.stdin.buffer.read(1)
        pathlib.Path(sys.argv[1]).write_text('started')
        time.sleep(30)
"#;
        let client = Arc::new(
            McpClient::connect_stdio(
                "test",
                "python3",
                &[
                    "-u".into(),
                    "-c".into(),
                    server.into(),
                    marker.to_string_lossy().into(),
                ],
            )
            .await
            .unwrap(),
        );
        let pending = {
            let client = client.clone();
            tokio::spawn(async move { client.call_tool("test", serde_json::json!({})).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !marker.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("server did not consume part of the reply");
        pending.abort();
        assert!(matches!(pending.await, Err(error) if error.is_cancelled()));
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            client.call_tool("test", serde_json::json!({})),
        )
        .await
        .expect("partially written reply wedged the connection");
        assert!(next.is_err());
    }

    #[tokio::test]
    async fn http_rejects_cross_origin_redirects_with_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination_address = destination.local_addr().unwrap();
        let source_address = source.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = source.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{destination_address}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let forwarded = tokio::spawn(async move {
            match tokio::time::timeout(std::time::Duration::from_millis(500), destination.accept())
                .await
            {
                Ok(Ok((mut stream, _))) => {
                    let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    true
                }
                _ => false,
            }
        });
        let headers =
            std::collections::HashMap::from([("X-Api-Key".into(), "synthetic-secret".into())]);
        let transport =
            HttpTransport::new(format!("http://{source_address}/mcp"), &headers).unwrap();
        let result = transport
            .call("test", Some(serde_json::json!({"secret":"synthetic-body"})))
            .await;
        assert!(result.is_err());
        assert!(
            !forwarded.await.unwrap(),
            "redirect forwarded a request to another origin"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_preserves_the_configured_endpoint_path() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.starts_with("POST /mcp/ HTTP/1.1"), "{request}");
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let transport =
            HttpTransport::new(format!("http://{address}/mcp/"), &Default::default()).unwrap();
        transport.call("test", None).await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_allows_same_origin_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(first.read(&mut request).await.unwrap() > 0);
            first.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{address}/mcp/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            let (mut second, _) =
                tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let count = second.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_lowercase();
            assert!(request.starts_with("post /mcp/ http/1.1"), "{request}");
            assert!(request.contains("x-api-key: synthetic-secret"), "{request}");
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
            second.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let headers =
            std::collections::HashMap::from([("X-Api-Key".into(), "synthetic-secret".into())]);
        let transport = HttpTransport::new(format!("http://{address}/mcp"), &headers).unwrap();
        assert_eq!(transport.call("test", None).await.unwrap()["ok"], true);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn stdio_rejects_malformed_response_without_waiting_for_timeout() {
        let server = r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
        print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
    else:
        print('42', flush=True)
"#;
        let client = McpClient::connect_stdio(
            "test",
            "python3",
            &["-u".into(), "-c".into(), server.into()],
        )
        .await
        .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.call_tool("test", serde_json::json!({})),
        )
        .await
        .expect("malformed response left the call waiting");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stdio_replies_to_ping_while_idle() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("ping-replied");
        let server = r#"
import json, pathlib, sys
marker = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    req = json.loads(line)
    if req.get('method') == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
        print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
    elif req.get('method') == 'notifications/initialized':
        print(json.dumps({'jsonrpc':'2.0','id':'idle-ping','method':'ping'}),flush=True)
        reply = json.loads(sys.stdin.readline())
        if reply == {'jsonrpc':'2.0','id':'idle-ping','result':{}}: marker.write_text('received')
"#;
        let _client = McpClient::connect_stdio(
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
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !marker.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("idle server did not receive its ping response");
    }

    #[tokio::test]
    async fn stdio_responds_to_server_ping_during_a_tool_call() {
        let server = r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{},'serverInfo':{'name':'test','version':'1'}}
    else:
        print(json.dumps({'jsonrpc':'2.0','id':'server-ping','method':'ping'}), flush=True)
        reply = json.loads(sys.stdin.readline())
        result = {'content':[{'type':'text','text':json.dumps(reply)}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let client = McpClient::connect_stdio(
            "test",
            "python3",
            &["-u".into(), "-c".into(), server.into()],
        )
        .await
        .unwrap();
        let result = client
            .call_tool("test", serde_json::json!({}))
            .await
            .unwrap();
        let reply: Value = serde_json::from_str(&result.text).unwrap();
        assert_eq!(
            reply,
            serde_json::json!({"jsonrpc":"2.0","id":"server-ping","result":{}})
        );
    }

    #[tokio::test]
    async fn mcp_embedded_resources_redact_secrets_before_json_encoding() {
        let directory = tempfile::tempdir().unwrap();
        let server = r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{'tools':{}},'serverInfo':{'name':'test','version':'1'}}
    elif req['method'] == 'tools/list':
        result = {'tools':[{'name':'test','inputSchema':{'type':'object'}}]}
    else:
        result = {'content':[{'type':'resource','resource':{'uri':'file:///fixture','text':'key"\nsecret'}}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec!["-u".into(), "-c".into(), server.into()],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let tools = connect_tools(vec![config], directory.path()).await;
        let secrets = mew_tools::SecretSet {
            words: vec!["key\"\nsecret".into()],
            globs: vec![],
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = mew_tools::ToolCtx::new(
            Arc::new(mew_tools::ToolCtxShared {
                secrets: Arc::new(secrets),
                ..Default::default()
            }),
            "call".into(),
            Default::default(),
            tx,
        );
        let result = tools[0].execute(ctx, serde_json::json!({})).await.unwrap();
        let resource: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(resource["resource"]["text"], "[REDACTED]");
    }

    #[tokio::test]
    async fn mcp_tool_preserves_resource_content_and_empty_errors() {
        let directory = tempfile::tempdir().unwrap();
        let server = r#"
import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    if req['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25','capabilities':{'tools':{}},'serverInfo':{'name':'test','version':'1'}}
    elif req['method'] == 'tools/list':
        result = {'tools':[{'name':'test','inputSchema':{'type':'object'}}]}
    elif req['params']['arguments'].get('error'):
        result = {'isError':True,'content':[]}
    else:
        result = {'content':[{'type':'resource','resource':{'uri':'file:///fixture','mimeType':'text/plain','text':'embedded contents'}},{'type':'resource_link','uri':'file:///linked','name':'linked file'}]}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}),flush=True)
"#;
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: None,
            command: Some("python3".into()),
            args: vec!["-u".into(), "-c".into(), server.into()],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let tools = connect_tools(vec![config], directory.path()).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let shared = Arc::new(mew_tools::ToolCtxShared::default());
        let ctx = || {
            mew_tools::ToolCtx::new(
                shared.clone(),
                "call".into(),
                Default::default(),
                tx.clone(),
            )
        };
        let result = tools[0]
            .execute(ctx(), serde_json::json!({"error":true}))
            .await
            .unwrap();
        assert!(
            !result.error.is_empty(),
            "server error was reported as success"
        );
        let result = tools[0]
            .execute(ctx(), serde_json::json!({}))
            .await
            .unwrap();
        assert!(
            result.output.contains("embedded contents"),
            "{}",
            result.output
        );
        assert!(
            result.output.contains("file:///linked"),
            "{}",
            result.output
        );
    }

    #[tokio::test]
    async fn http_replies_to_server_ping_before_the_tool_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            let ping = "data: {\"jsonrpc\":\"2.0\",\"id\":\"server-ping\",\"method\":\"ping\"}\n\n";
            stream
                .write_all(format!("{:X}\r\n{ping}\r\n", ping.len()).as_bytes())
                .await
                .unwrap();
            let response =
                tokio::time::timeout(std::time::Duration::from_millis(500), listener.accept())
                    .await;
            let mut reply = String::new();
            if let Ok(Ok((mut incoming, _))) = response {
                let count = incoming.read(&mut request).await.unwrap();
                reply = String::from_utf8_lossy(&request[..count]).into_owned();
                incoming
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            }
            let result = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
            stream
                .write_all(format!("{:X}\r\n{result}\r\n0\r\n\r\n", result.len()).as_bytes())
                .await
                .unwrap();
            reply
        });
        let transport =
            HttpTransport::new(format!("http://{address}/mcp"), &Default::default()).unwrap();
        assert_eq!(transport.call("test", None).await.unwrap()["ok"], true);
        let reply = server.await.unwrap();
        let body = reply
            .split_once("\r\n\r\n")
            .expect("missing ping response")
            .1;
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            serde_json::json!({"jsonrpc":"2.0","id":"server-ping","result":{}})
        );
    }

    #[tokio::test]
    async fn cancelling_discovery_awaits_cleanup_of_an_initialized_session() {
        check_discovery_cleanup(false).await;
    }

    #[tokio::test]
    async fn timed_out_discovery_awaits_cleanup_of_an_initialized_session() {
        check_discovery_cleanup(true).await;
    }

    async fn check_discovery_cleanup(timeout: bool) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (listing_tx, listing_rx) = tokio::sync::oneshot::channel();
        let (close_tx, close_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut data = [0; 4096];
            let (mut init, _) = listener.accept().await.unwrap();
            assert!(init.read(&mut data).await.unwrap() > 0);
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1"}}}"#;
            init.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: startup\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            let (mut initialized, _) = listener.accept().await.unwrap();
            assert!(initialized.read(&mut data).await.unwrap() > 0);
            initialized
                .write_all(
                    b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            let (mut listing, _) = listener.accept().await.unwrap();
            assert!(listing.read(&mut data).await.unwrap() > 0);
            listing_tx.send(()).unwrap();
            loop {
                let (mut incoming, _) = listener.accept().await.unwrap();
                let count = incoming.read(&mut data).await.unwrap();
                let request = String::from_utf8_lossy(&data[..count]).to_lowercase();
                if request.starts_with("delete ") {
                    assert!(request.contains("mcp-session-id: startup"));
                    close_tx.send(()).unwrap();
                    release_rx.await.unwrap();
                    incoming
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                    break;
                }
                incoming
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            }
            drop(listing);
        });
        let directory = tempfile::tempdir().unwrap();
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: Some(format!("http://{address}/mcp")),
            command: None,
            args: vec![],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let path = directory.path().to_path_buf();
        let mut discovery = tokio::spawn(async move {
            connect_tools_with_timeout(
                vec![config],
                &path,
                async {
                    let _ = cancel_rx.await;
                },
                if timeout {
                    std::time::Duration::from_millis(300)
                } else {
                    MCP_DISCOVERY_TIMEOUT
                },
            )
            .await
        });
        listing_rx.await.unwrap();
        if !timeout {
            cancel_tx.send(()).unwrap();
        }
        close_rx.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut discovery)
                .await
                .is_err(),
            "discovery returned before cleanup completed"
        );
        release_tx.send(()).unwrap();
        assert!(discovery.await.unwrap().is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn discovery_closes_sessions_without_usable_tools_before_returning() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for result in [
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"failed"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"bad space","inputSchema":null}]}}"#,
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let mut request = [0; 4096];
                let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1"}}}"#;
                for (body, extra) in [
                    (init, "MCP-Session-Id: session-1\r\n"),
                    ("", ""),
                    (result, ""),
                ] {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    assert!(stream.read(&mut request).await.unwrap() > 0);
                    let status = if body.is_empty() {
                        "202 Accepted"
                    } else {
                        "200 OK"
                    };
                    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                }
                let (mut close, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                        .await
                        .expect("session was not closed")
                        .unwrap();
                let count = close.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..count]).to_lowercase();
                assert!(request.starts_with("delete /mcp http/1.1"), "{request}");
                started_tx.send(()).unwrap();
                release_rx.await.unwrap();
                close
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            });
            let directory = tempfile::tempdir().unwrap();
            let config = McpServerConfig {
                name: "test".into(),
                type_: None,
                url: Some(format!("http://{address}/mcp")),
                command: None,
                args: vec![],
                env: Default::default(),
                headers: Default::default(),
                disabled: false,
            };
            let path = directory.path().to_path_buf();
            let mut discovery =
                tokio::spawn(async move { connect_tools(vec![config], &path).await });
            started_rx.await.unwrap();
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut discovery)
                    .await
                    .is_err(),
                "discovery returned before closing its unused session"
            );
            release_tx.send(()).unwrap();
            assert!(discovery.await.unwrap().is_empty());
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn shutting_down_cancels_an_in_flight_http_tool_call() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut init, _) = listener.accept().await.unwrap();
            let mut data = [0; 4096];
            assert!(init.read(&mut data).await.unwrap() > 0);
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
            init.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: active\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            let (mut call, _) = listener.accept().await.unwrap();
            assert!(call.read(&mut data).await.unwrap() > 0);
            started_tx.send(()).unwrap();
            loop {
                let (mut incoming, _) = listener.accept().await.unwrap();
                let count = incoming.read(&mut data).await.unwrap();
                let request = String::from_utf8_lossy(&data[..count]).to_lowercase();
                incoming
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                if request.starts_with("delete ") {
                    break;
                }
            }
            let _ = release_rx.await;
            drop(call);
        });
        let transport =
            HttpTransport::new(format!("http://{address}/mcp"), &Default::default()).unwrap();
        transport.call("initialize", None).await.unwrap();
        let client = Arc::new(McpClient {
            name: "test".into(),
            transport: Box::new(transport),
        });
        let managed = Arc::new(ManagedClient::new(client));
        let call = {
            let managed = managed.clone();
            tokio::spawn(async move { managed.call_tool("test", serde_json::json!({})).await })
        };
        started_rx.await.unwrap();
        managed.shutdown().await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), call)
            .await
            .expect("shutdown left the tool call running")
            .unwrap();
        assert!(result.is_err());
        release_tx.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn shutting_down_waits_for_and_closes_a_reconnecting_session() {
        check_reconnecting_session_cleanup(0).await;
    }

    #[tokio::test]
    async fn cancelling_a_reconnect_waits_for_session_cleanup() {
        check_reconnecting_session_cleanup(1).await;
    }

    #[tokio::test]
    async fn timing_out_a_reconnect_waits_for_session_cleanup() {
        check_reconnecting_session_cleanup(2).await;
    }

    async fn check_reconnecting_session_cleanup(mode: u8) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (close_tx, close_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut init, _) = listener.accept().await.unwrap();
            let mut data = [0; 4096];
            assert!(init.read(&mut data).await.unwrap() > 0);
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}"#;
            init.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: reconnected\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            let (mut notification, _) = listener.accept().await.unwrap();
            assert!(notification.read(&mut data).await.unwrap() > 0);
            started_tx.send(()).unwrap();
            let (mut close, _) =
                tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                    .await
                    .expect("reconnected session was not closed")
                    .unwrap();
            let count = close.read(&mut data).await.unwrap();
            let request = String::from_utf8_lossy(&data[..count]).to_lowercase();
            assert!(request.starts_with("delete /mcp http/1.1"), "{request}");
            assert!(request.contains("mcp-session-id: reconnected"), "{request}");
            close_tx.send(()).unwrap();
            release_rx.await.unwrap();
            close
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let url = format!("http://{address}/mcp");
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: Some(url.clone()),
            command: None,
            args: vec![],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let original = Arc::new(McpClient {
            name: "test".into(),
            transport: Box::new(HttpTransport::new(url, &Default::default()).unwrap()),
        });
        let managed = Arc::new(ManagedClient::configured(
            original,
            config,
            directory.path().to_path_buf(),
        ));
        managed.invalid.store(true, Ordering::Release);
        let definition = McpToolDef {
            name: "test".into(),
            description: String::new(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let tool = Arc::new(McpTool::with_client("test", &definition, managed.clone()));
        let (call, cancel) = {
            let tool = tool.clone();
            let (tx, _rx) = tokio::sync::mpsc::channel(1);
            let ctx = mew_tools::ToolCtx::new(
                Arc::new(mew_tools::ToolCtxShared::default()),
                "call".into(),
                Default::default(),
                tx,
            );
            let cancel = ctx.cancel.clone();
            let call = tokio::spawn(async move {
                if mode == 2 {
                    managed
                        .call_tool_with_timeout(
                            "test",
                            serde_json::json!({}),
                            std::time::Duration::from_millis(300),
                        )
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                } else {
                    mew_tools::Tool::execute(tool.as_ref(), ctx, serde_json::json!({}))
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|output| {
                            if output.error.is_empty() {
                                Ok(())
                            } else {
                                Err(output.error)
                            }
                        })
                }
            });
            (call, cancel)
        };
        started_rx.await.unwrap();
        if mode != 0 {
            let mut call = call;
            if mode == 1 {
                cancel.cancel();
            }
            close_rx.await.unwrap();
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut call)
                    .await
                    .is_err(),
                "call returned before reconnect cleanup completed"
            );
            release_tx.send(()).unwrap();
            let error = call.await.unwrap().unwrap_err();
            if mode == 2 {
                assert!(error.contains("timed out"), "{error}");
            }
        } else {
            let mut shutdown =
                tokio::spawn(async move { mew_tools::Tool::shutdown(tool.as_ref()).await });
            close_rx.await.unwrap();
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut shutdown)
                    .await
                    .is_err(),
                "shutdown returned before reconnect finished"
            );
            release_tx.send(()).unwrap();
            shutdown.await.unwrap().unwrap();
            assert!(call.await.unwrap().is_err());
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_http_shutdowns_wait_for_the_same_session_close() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for success in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let mut data = [0; 4096];
                let (mut init, _) = listener.accept().await.unwrap();
                assert!(init.read(&mut data).await.unwrap() > 0);
                let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
                init.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: session-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                let (mut close, _) = listener.accept().await.unwrap();
                let count = close.read(&mut data).await.unwrap();
                assert!(String::from_utf8_lossy(&data[..count]).starts_with("DELETE /mcp "));
                started_tx.send(()).unwrap();
                release_rx.await.unwrap();
                let status = if success {
                    "200 OK"
                } else {
                    "500 Internal Server Error"
                };
                close
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            });
            let transport = Arc::new(
                HttpTransport::new(format!("http://{address}/mcp"), &Default::default()).unwrap(),
            );
            transport.call("initialize", None).await.unwrap();
            let first = {
                let transport = transport.clone();
                tokio::spawn(async move { transport.close().await })
            };
            started_rx.await.unwrap();
            let mut second = {
                let transport = transport.clone();
                tokio::spawn(async move { transport.close().await })
            };
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut second)
                    .await
                    .is_err(),
                "second close returned before the session closed"
            );
            release_tx.send(()).unwrap();
            assert_eq!(first.await.unwrap().is_ok(), success);
            assert_eq!(second.await.unwrap().is_ok(), success);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn shutting_down_an_mcp_tool_closes_the_shared_http_session() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: session-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                    .await
                    .expect("tool shutdown did not close the session")
                    .unwrap();
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_lowercase();
            assert!(request.starts_with("delete /mcp http/1.1"), "{request}");
            assert!(request.contains("mcp-session-id: session-1"), "{request}");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let transport =
            HttpTransport::new(format!("http://{address}/mcp"), &Default::default()).unwrap();
        transport.call("test", None).await.unwrap();
        let client = Arc::new(McpClient {
            name: "test".into(),
            transport: Box::new(transport),
        });
        let definition = McpToolDef {
            name: "test".into(),
            description: String::new(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let tool = McpTool::new("test", &definition, client);
        mew_tools::Tool::shutdown(&tool).await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn closing_or_dropping_http_transport_closes_the_server_session() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for explicit in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).await.unwrap() > 0);
                let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: session-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                let (mut stream, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                        .await
                        .expect("session not closed")
                        .unwrap();
                let count = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..count]).to_lowercase();
                assert!(request.starts_with("delete /mcp http/1.1"), "{request}");
                assert!(request.contains("mcp-session-id: session-1"), "{request}");
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            });
            let transport =
                HttpTransport::new(format!("http://{address}/mcp"), &Default::default()).unwrap();
            transport.call("test", None).await.unwrap();
            if explicit {
                transport.close().await.unwrap();
            }
            drop(transport);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn http_sse_returns_the_matching_response_before_the_stream_closes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
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
    async fn http_json_response_rejects_oversized_content_length_without_waiting_for_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16777217\r\n\r\n").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });
        let transport =
            HttpTransport::new(format!("http://{address}"), &Default::default()).unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            transport.call("test", None),
        )
        .await
        .expect("waited for oversized body")
        .unwrap_err()
        .to_string();
        assert!(error.contains("too large"), "{error}");
        server.abort();
    }

    #[tokio::test]
    async fn http_json_response_rejects_oversized_chunked_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            let chunk = vec![b'x'; 64 * 1024];
            for _ in 0..257 {
                if stream.write_all(b"10000\r\n").await.is_err()
                    || stream.write_all(&chunk).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        });
        let transport =
            HttpTransport::new(format!("http://{address}"), &Default::default()).unwrap();
        let error = transport.call("test", None).await.unwrap_err().to_string();
        assert!(error.contains("too large"), "{error}");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn configured_http_server_reconnects_expired_session_without_replaying_call() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut tool_calls = 0;
            let mut sessions = 0;
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let (headers, request) = loop {
                    let mut buffer = [0; 4096];
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if data.len() >= end + 4 + length {
                            break (
                                headers,
                                serde_json::from_slice::<Value>(&data[end + 4..end + 4 + length])
                                    .unwrap(),
                            );
                        }
                    }
                };
                let method = request["method"].as_str().unwrap();
                let (status, body, session) = match method {
                    "initialize" => {
                        assert!(!headers.contains("mcp-session-id:"));
                        sessions += 1;
                        ("200 OK", serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"test","version":"1"}}}).to_string(), format!("MCP-Session-Id: session-{sessions}\r\n"))
                    }
                    "notifications/initialized" => ("202 Accepted", String::new(), String::new()),
                    "tools/list" => ("200 OK", serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"tools":[{"name":"write","inputSchema":{"type":"object"}}]}}).to_string(), String::new()),
                    "tools/call" => {
                        tool_calls += 1;
                        if tool_calls == 1 {
                            assert!(headers.contains("mcp-session-id: session-1"));
                            ("404 Not Found", String::new(), String::new())
                        } else {
                            assert!(headers.contains("mcp-session-id: session-2"));
                            ("200 OK", serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"content":[{"type":"text","text":"completed"}]}}).to_string(), String::new())
                        }
                    }
                    _ => panic!("unexpected method {method}"),
                };
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{session}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                if tool_calls == 2 {
                    return (sessions, tool_calls);
                }
            }
        });
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: Some(format!("http://{address}")),
            command: None,
            args: vec![],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let directory = tempfile::tempdir().unwrap();
        let tools = connect_tools(vec![config], directory.path()).await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let ctx = || {
            mew_tools::ToolCtx::new(
                Arc::new(mew_tools::ToolCtxShared::default()),
                "call".into(),
                Default::default(),
                tx.clone(),
            )
        };
        let first = tools[0]
            .execute(ctx(), serde_json::json!({}))
            .await
            .unwrap();
        assert!(!first.error.is_empty());
        let second = tools[0]
            .execute(ctx(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(second.output, "completed", "{}", second.error);
        assert_eq!(server.await.unwrap(), (2, 2));
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
                assert!(stream.read(&mut request).await.unwrap() > 0);
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

    #[tokio::test]
    async fn timed_out_http_tool_notifies_server_and_reconnects_without_replay() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut calls = 0;
            let mut sessions = 0;
            let mut cancelled = 0;
            let mut pending = None;
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let request: Value = loop {
                    let mut buffer = [0; 4096];
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
                            break serde_json::from_slice(&data[end + 4..end + 4 + length])
                                .unwrap();
                        }
                    }
                };
                let (status, body, session) = match request["method"].as_str().unwrap() {
                    "initialize" => {
                        sessions += 1;
                        ("200 OK", serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}).to_string(), format!("MCP-Session-Id: session-{sessions}\r\n"))
                    }
                    "notifications/initialized" => ("202 Accepted", String::new(), String::new()),
                    "notifications/cancelled" => {
                        cancelled += 1;
                        drop(pending.take());
                        ("202 Accepted", String::new(), String::new())
                    }
                    "tools/call" => {
                        calls += 1;
                        if calls == 1 {
                            pending = Some(stream);
                            continue;
                        }
                        ("200 OK", serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"content":[{"type":"text","text":"recovered"}]}}).to_string(), String::new())
                    }
                    method => panic!("unexpected method {method}"),
                };
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{session}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                if sessions == 2 && calls == 2 && cancelled == 1 {
                    return (sessions, calls, cancelled);
                }
            }
        });
        let config = McpServerConfig {
            name: "test".into(),
            type_: None,
            url: Some(format!("http://{address}")),
            command: None,
            args: vec![],
            env: Default::default(),
            headers: Default::default(),
            disabled: false,
        };
        let directory = tempfile::tempdir().unwrap();
        let client = Arc::new(
            McpClient::connect_configured(&config, directory.path())
                .await
                .unwrap(),
        );
        let managed = ManagedClient::configured(client, config, directory.path().to_path_buf());
        let result = managed
            .call_tool_with_timeout(
                "test",
                serde_json::json!({}),
                std::time::Duration::from_millis(100),
            )
            .await;
        assert!(matches!(result, Err(McpError::Transport(_))));
        let result = managed
            .call_tool("test", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result.text, "recovered");
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(3), server)
                .await
                .unwrap()
                .unwrap(),
            (2, 2, 1)
        );
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
