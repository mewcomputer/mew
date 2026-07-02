use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    #[error("transport: {0}")]
    Transport(String),
    #[error("json-rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("initialization failed: {0}")]
    Init(String),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
}

// ---------------------------------------------------------------------------
// Transport abstraction
// ---------------------------------------------------------------------------

#[async_trait]
trait Transport: Send + Sync {
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, McpError>;
    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError>;
    #[allow(dead_code)]
    async fn cancel(&self, _request_id: u64) -> Result<(), McpError> {
        Ok(())
    }
    fn set_protocol_version(&self, _version: &str) {
        // Default no-op for transports that don't need version negotiation.
    }
    async fn close(&mut self) -> Result<(), McpError>;
}

// ---------------------------------------------------------------------------
// HTTP transport (streamable HTTP)
// ---------------------------------------------------------------------------

/// Parse an SSE (Server-Sent Events) response from an MCP server.
async fn parse_sse_response(resp: reqwest::Response) -> Result<Value, McpError> {
    let body = resp
        .text()
        .await
        .map_err(|e| McpError::Transport(format!("read sse body: {e}")))?;

    // Accumulate multi-line data chunks.
    let mut data_buffer = String::new();

    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if !data_buffer.is_empty() {
                data_buffer.push('\n');
            }
            data_buffer.push_str(data);
        } else if line.is_empty() && !data_buffer.is_empty() {
            // End of SSE event: try to parse the accumulated data.
            if let Some(result) = try_parse_sse_data(&data_buffer) {
                return result;
            }
            data_buffer.clear();
        }
    }

    // Handle last event if no empty line terminated it.
    if !data_buffer.is_empty() {
        if let Some(result) = try_parse_sse_data(&data_buffer) {
            return result;
        }
    }

    Err(McpError::Protocol("no data in SSE response".into()))
}

/// Try to parse accumulated SSE data as a JSON-RPC response. Returns
/// `Some(Ok(...))` for a valid response, `Some(Err(...))` for a fatal error,
/// or `None` to skip this event (e.g. it was a progress notification).
fn try_parse_sse_data(data: &str) -> Option<Result<Value, McpError>> {
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
    fn new(url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.trim_end_matches('/').to_string(),
            request_id: AtomicU64::new(1),
            protocol_version: StdMutex::new(MAX_PROTOCOL_VERSION.into()),
            session_id: StdMutex::new(None),
        }
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
            let body = resp.text().await.unwrap_or_default();
            return Err(McpError::Transport(format!("HTTP {status}: {body}")));
        }

        // Handle SSE (text/event-stream) responses.
        if content_type.starts_with("text/event-stream") {
            return parse_sse_response(resp).await;
        }

        let response: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| McpError::Transport(format!("parse response: {e}")))?;

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

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let body = serde_json::to_vec(&request).map_err(|e| McpError::Transport(e.to_string()))?;

        let resp = self.build_request(body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(McpError::Transport(format!(
                "notification failed: HTTP {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn cancel(&self, request_id: u64) -> Result<(), McpError> {
        self.notify(
            "notifications/cancelled",
            Some(serde_json::json!({
                "requestId": request_id,
                "reason": "cancelled by client"
            })),
        )
        .await
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
    stdin: tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
    stdout: tokio::sync::Mutex<BufReader<tokio::process::ChildStdout>>,
    request_id: AtomicU64,
}

impl StdioTransport {
    fn new(mut child: tokio::process::Child) -> Result<Self, McpError> {
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
            stdin: tokio::sync::Mutex::new(Some(stdin)),
            stdout: tokio::sync::Mutex::new(BufReader::new(stdout)),
            request_id: AtomicU64::new(1),
        })
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
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

        // Write to stdin.
        {
            let mut guard = self.stdin.lock().await;
            let stdin = guard
                .as_mut()
                .ok_or_else(|| McpError::Transport("stdin closed".into()))?;
            stdin
                .write_all(body.as_bytes())
                .await
                .map_err(|e| McpError::Transport(format!("write stdin: {e}")))?;
        }

        // Read response line from stdout.
        let response_line = {
            let mut stdout = self.stdout.lock().await;
            let mut line = String::new();
            stdout
                .read_line(&mut line)
                .await
                .map_err(|e| McpError::Transport(format!("read stdout: {e}")))?;
            if line.is_empty() {
                return Err(McpError::Transport("server closed stdout".into()));
            }
            line
        };

        let response: JsonRpcResponse = serde_json::from_str(&response_line).map_err(|e| {
            McpError::Transport(format!("parse response: {e}\nraw: {response_line:.100}"))
        })?;

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

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

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

    async fn cancel(&self, request_id: u64) -> Result<(), McpError> {
        self.notify(
            "notifications/cancelled",
            Some(serde_json::json!({
                "requestId": request_id,
                "reason": "cancelled by client"
            })),
        )
        .await
    }

    async fn close(&mut self) -> Result<(), McpError> {
        // Graceful: close stdin to signal EOF, then wait for exit.
        self.stdin.lock().await.take();
        let mut child = self.child.lock().await;
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

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    /// Server name (e.g. "context7")
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
        let transport = HttpTransport::new(url.to_string());
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
        let child = tokio::process::Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
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

    /// Perform the MCP handshake: initialize → get capabilities → send initialized.
    async fn initialize(&mut self) -> Result<(), McpError> {
        info!(server = %self.name, "initializing MCP server");

        let result = self
            .transport
            .call(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": MAX_PROTOCOL_VERSION,
                    "capabilities": {
                        "roots": { "listChanged": true }
                    },
                    "clientInfo": {
                        "name": "mew",
                        "title": "mew",
                        "version": "0.1.0"
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
        let _ = self
            .transport
            .notify("notifications/initialized", None)
            .await;
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

            info!(server = %self.name, page_count = page.len(), cursor = ?cursor, "discovered MCP tool page");
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

        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok(ToolCallResult { text, is_error })
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
        format!("mcp__{}__{}", server_name, self.name)
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
        // Return the qualified name — the agent registry uses this as key.
        // (We need to store it because `name()` returns a reference.)
        // We'll use a leased pattern: store the name in the struct.
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
        _ctx: mew_tools::ToolCtx,
        input: Value,
    ) -> Result<mew_hooks::ToolOutput, mew_tools::ToolError> {
        match self.client.call_tool(&self.tool_name, input).await {
            Ok(result) => Ok(mew_hooks::ToolOutput {
                output: if result.is_error {
                    String::new()
                } else {
                    result.text.clone()
                },
                error: if result.is_error {
                    result.text
                } else {
                    String::new()
                },
                diff: None,
                ..Default::default()
            }),
            Err(e) => Ok(mew_hooks::ToolOutput {
                output: String::new(),
                error: e.to_string(),
                diff: None,
                metadata: None,
                file_delta: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live integration test — connect, handshake, list tools, call one.
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_context7_live() {
        let _ = tracing_subscriber::fmt::try_init();

        let url = "https://mcp.context7.com/mcp";
        eprintln!("connecting to {url}...");

        let client = McpClient::connect_http("context7", url)
            .await
            .expect("connect");

        eprintln!("connected, listing tools...");
        let tools = client.list_tools().await.expect("list tools");

        eprintln!("found {} tools:", tools.len());
        for t in &tools {
            eprintln!("  {} — {}", t.name, t.description);
        }

        assert!(!tools.is_empty(), "expected at least one tool");

        if let Some(first) = tools.first() {
            eprintln!("\ncalling tool: {}", first.name);
            let args = if first.name.contains("resolve") || first.name.contains("get") {
                serde_json::json!({"query": "rust reqwest"})
            } else {
                serde_json::json!({})
            };
            match client.call_tool(&first.name, args).await {
                Ok(r) => eprintln!(
                    "tool output (first 500): {}",
                    &r.text[..r.text.len().min(500)]
                ),
                Err(e) => eprintln!("tool error (may be expected): {}", e),
            }
        }
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

        let mut client = McpClient::connect_stdio(
            "filesystem",
            "npx",
            &[
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
                allowed.clone(),
            ],
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
        match client
            .call_tool(
                "write_file",
                serde_json::json!({
                    "path": &test_path,
                    "content": "hello world from mew mcp test\n"
                }),
            )
            .await
        {
            Ok(r) => eprintln!("write: {}", r.text),
            Err(e) => eprintln!("write error: {e}"),
        }

        eprintln!("\nreading test file...");
        match client
            .call_tool("read_text_file", serde_json::json!({"path": &test_path}))
            .await
        {
            Ok(r) => {
                eprintln!("read: {}", r.text);
                assert!(r.text.contains("hello world"), "file content mismatch");
            }
            Err(e) => eprintln!("read error: {e}"),
        }
    }
}
