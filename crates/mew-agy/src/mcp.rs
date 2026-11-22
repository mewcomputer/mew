//! A bounded loopback MCP subset for forwarding tool calls to Mew.
//!
//! Supports initialization, initialized notifications, tool listing and calls.
//! This is not a complete MCP server and does not provide OS isolation.
//! Each server accepts one initialization for one AGY process. A lost
//! initialization response or a reconnect requiring a new session cannot be
//! resumed. Duplicate suppression does not survive a server/process restart.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rand::{rngs::OsRng, RngCore};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

const MAX_BODY: usize = 1_048_576;
const MAX_SCHEMA: usize = 65_536;
const MAX_DESCRIPTION: usize = 8192;
const MAX_TOOLS: usize = 256;
const MAX_IDS: usize = 4096;
const MAX_CONNECTIONS: usize = 64;
type Body = Full<Bytes>;

struct NoExternalSchemas;
impl jsonschema::Retrieve for NoExternalSchemas {
    fn retrieve(
        &self,
        _uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("external schema references are disabled".into())
    }
}

pub struct Tool {
    name: String,
    description: String,
    schema: Value,
    validator: jsonschema::Validator,
}

impl Tool {
    pub fn new(name: &str, description: &str, schema: Value) -> Result<Self, String> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        {
            return Err("invalid tool name".into());
        }
        if description.len() > MAX_DESCRIPTION || json_size(&schema, MAX_SCHEMA).is_err() {
            return Err("tool definition too large".into());
        }
        let validator = jsonschema::options()
            .with_retriever(NoExternalSchemas)
            .build(&schema)
            .map_err(|e| format!("invalid schema: {e}"))?;
        Ok(Self {
            name: name.into(),
            description: description.into(),
            schema,
            validator,
        })
    }
}

#[derive(Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub cancellation: CancellationToken,
}

struct Session {
    initialized: bool,
    ids: HashSet<String>,
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Session>,
    pending: HashMap<String, oneshot::Sender<Value>>,
    requests: HashMap<(String, String), String>,
    quiesced: bool,
}

struct State {
    path: String,
    tools: HashMap<String, Tool>,
    registry: Mutex<Registry>,
    calls: mpsc::Sender<ToolCall>,
    slots: Arc<Semaphore>,
}

pub struct McpServer {
    url: String,
    state: Arc<State>,
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl McpServer {
    pub async fn bind(
        tools: Vec<Tool>,
        max_pending: usize,
    ) -> Result<(Self, mpsc::Receiver<ToolCall>), String> {
        if max_pending == 0 || max_pending > 1024 {
            return Err("invalid pending limit".into());
        }
        if tools.len() > MAX_TOOLS {
            return Err("tool limit exceeded".into());
        }
        let mut total_tool_bytes = 0_usize;
        let mut map = HashMap::new();
        for tool in tools {
            total_tool_bytes = total_tool_bytes
                .saturating_add(tool.name.len())
                .saturating_add(tool.description.len())
                .saturating_add(
                    json_size(&tool.schema, MAX_SCHEMA).map_err(|_| "tool schema too large")?,
                )
                .saturating_add(128);
            if total_tool_bytes > MAX_BODY {
                return Err("tool catalog too large".into());
            }
            if map.insert(tool.name.clone(), tool).is_some() {
                return Err("duplicate tool name".into());
            }
        }
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .map_err(|e| e.to_string())?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let path = format!("/{}", random_id()?);
        let (tx, rx) = mpsc::channel(max_pending);
        let state = Arc::new(State {
            path: path.clone(),
            tools: map,
            registry: Mutex::new(Registry::default()),
            calls: tx,
            slots: Arc::new(Semaphore::new(max_pending)),
        });
        let owned = state.clone();
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            let connections = Arc::new(Semaphore::new(MAX_CONNECTIONS));
            let mut jobs = JoinSet::new();
            loop {
                tokio::select! {
                    _ = stopped.cancelled() => break,
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        let Ok(permit) = connections.clone().try_acquire_owned() else { continue };
                        let state = owned.clone();
                        jobs.spawn(async move {
                            let _permit = permit;
                            let service = service_fn(move |req| handle(req, state.clone()));
                            let _ = tokio::time::timeout(Duration::from_secs(630), hyper::server::conn::http1::Builder::new().keep_alive(false).serve_connection(TokioIo::new(socket), service)).await;
                        });
                    }
                    Some(_) = jobs.join_next(), if !jobs.is_empty() => {}
                }
            }
            jobs.shutdown().await;
        });
        Ok((
            Self {
                url: format!("http://{addr}{path}"),
                state,
                stop,
                task: Some(task),
            },
            rx,
        ))
    }
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Stop accepting requests and wait until all connections have closed.
    pub async fn shutdown(&mut self) -> std::io::Result<()> {
        self.stop.cancel();
        if let Some(task) = self.task.as_mut() {
            task.await.map_err(std::io::Error::other)?;
            self.task.take();
        }
        Ok(())
    }

    /// Stop admitting tool calls and report whether all existing calls settled.
    pub fn quiesce(&self) -> bool {
        let mut registry = self
            .state
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.quiesced = true;
        registry.requests.is_empty()
    }
    pub fn resolve(&self, id: &str, result: Value) -> Result<(), &'static str> {
        let sender = self
            .state
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .remove(id)
            .ok_or("unknown call")?;
        if json_size(&result, MAX_BODY).is_err() {
            return Err("tool result too large");
        }
        sender.send(result).map_err(|_| "call cancelled")
    }
    pub fn is_cancelled(&self, id: &str) -> bool {
        !self
            .state
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .contains_key(id)
    }
}
impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

struct PendingGuard {
    state: Arc<State>,
    id: String,
    request_key: (String, String),
    cancellation: CancellationToken,
    completed: bool,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.cancellation.cancel();
        }
        let mut reg = self
            .state
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reg.pending.remove(&self.id);
        reg.requests.remove(&self.request_key);
    }
}

async fn handle(req: Request<Incoming>, state: Arc<State>) -> Result<Response<Body>, Infallible> {
    Ok(handle_inner(req, state).await)
}
async fn handle_inner(req: Request<Incoming>, state: Arc<State>) -> Response<Body> {
    if req.uri().path() != state.path || req.uri().query().is_some() {
        return response(StatusCode::NOT_FOUND, "");
    }
    if req.headers().contains_key("origin") {
        return response(StatusCode::FORBIDDEN, "");
    }
    if req.method() != Method::POST {
        return response(StatusCode::METHOD_NOT_ALLOWED, "");
    }
    let session = req
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = match tokio::time::timeout(
        Duration::from_secs(15),
        Limited::new(req.into_body(), MAX_BODY).collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(_)) => return response(StatusCode::PAYLOAD_TOO_LARGE, ""),
        Err(_) => return response(StatusCode::REQUEST_TIMEOUT, ""),
    };
    let Ok(rpc) = serde_json::from_slice::<Value>(&body) else {
        return rpc_error(Value::Null, -32700, "parse error");
    };
    let Some(obj) = rpc.as_object() else {
        return rpc_error(Value::Null, -32600, "invalid request");
    };
    let id = obj.get("id").cloned();
    if obj.get("jsonrpc") != Some(&json!("2.0"))
        || !obj.get("method").is_some_and(Value::is_string)
        || id
            .as_ref()
            .is_some_and(|id| !(id.is_string() || id.is_i64() || id.is_u64()))
    {
        return rpc_error(Value::Null, -32600, "invalid request");
    }
    let method = obj["method"].as_str().unwrap();
    if method == "initialize" {
        let Some(id) = id else {
            return rpc_error(Value::Null, -32600, "id required");
        };
        let version = rpc
            .pointer("/params/protocolVersion")
            .and_then(Value::as_str);
        if session.is_some()
            || !matches!(
                version,
                Some("2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25")
            )
        {
            return rpc_error(id, -32602, "unsupported initialization");
        }
        let Ok(sid) = random_id() else {
            return response(StatusCode::INTERNAL_SERVER_ERROR, "");
        };
        let mut reg = state.registry.lock().unwrap_or_else(|e| e.into_inner());
        if !reg.sessions.is_empty() {
            return rpc_error(id, -32000, "session already initialized");
        }
        reg.sessions.insert(
            sid.clone(),
            Session {
                initialized: false,
                ids: HashSet::new(),
            },
        );
        drop(reg);
        let mut resp = rpc_result(
            id,
            json!({"protocolVersion":version.unwrap(),"capabilities":{"tools":{}},"serverInfo":{"name":"mew","version":"1"}}),
        );
        if let Ok(value) = sid.parse() {
            resp.headers_mut().insert("mcp-session-id", value);
        }
        return resp;
    }
    let Some(sid) = session else {
        return rpc_error(id.unwrap_or(Value::Null), -32600, "session required");
    };
    if method == "notifications/initialized" && id.is_none() {
        let mut reg = state.registry.lock().unwrap_or_else(|e| e.into_inner());
        return if let Some(s) = reg.sessions.get_mut(&sid) {
            s.initialized = true;
            response(StatusCode::ACCEPTED, "")
        } else {
            response(StatusCode::NOT_FOUND, "")
        };
    }
    if method == "notifications/cancelled" && id.is_none() {
        let Some(request_id) = rpc
            .pointer("/params/requestId")
            .filter(|id| id.is_string() || id.is_i64() || id.is_u64())
        else {
            return response(StatusCode::BAD_REQUEST, "");
        };
        let mut reg = state.registry.lock().unwrap_or_else(|e| e.into_inner());
        if !reg.sessions.get(&sid).is_some_and(|s| s.initialized) {
            return response(StatusCode::NOT_FOUND, "");
        }
        if let Some(call_id) = reg.requests.get(&(sid, request_id.to_string())).cloned() {
            reg.pending.remove(&call_id);
        }
        return response(StatusCode::ACCEPTED, "");
    }
    let Some(id) = id else {
        return response(StatusCode::BAD_REQUEST, "");
    };
    {
        let mut reg = state.registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = reg.sessions.get_mut(&sid) else {
            return rpc_error(id, -32600, "unknown session");
        };
        if !s.initialized {
            return rpc_error(id, -32600, "session not initialized");
        }
        if s.ids.len() >= MAX_IDS || !s.ids.insert(id.to_string()) {
            return rpc_error(id, -32600, "duplicate id or session capacity exceeded");
        }
    }
    if method == "tools/list" {
        let mut names: Vec<_> = state.tools.values().collect();
        names.sort_by(|a, b| a.name.cmp(&b.name));
        let tools: Vec<_> = names
            .into_iter()
            .map(|t| json!({"name":t.name,"description":t.description,"inputSchema":t.schema}))
            .collect();
        return rpc_result(id, json!({"tools":tools}));
    }
    if method != "tools/call" {
        return rpc_error(id, -32601, "method not found");
    }
    let Some(params) = obj.get("params").and_then(Value::as_object) else {
        return rpc_error(id, -32602, "invalid parameters");
    };
    if params
        .keys()
        .any(|k| k != "name" && k != "arguments" && k != "_meta")
        || params.get("_meta").is_some_and(|value| !value.is_object())
    {
        return rpc_error(id, -32602, "invalid parameters");
    }
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(id, -32602, "invalid tool name");
    };
    let Some(tool) = state.tools.get(name) else {
        return rpc_error(id, -32602, "unknown tool");
    };
    let Some(arguments) = params.get("arguments").filter(|v| v.is_object()) else {
        return rpc_error(id, -32602, "invalid arguments");
    };
    if !tool.validator.is_valid(arguments) {
        return rpc_error(id, -32602, "invalid arguments");
    }
    let Ok(permit) = state.slots.clone().try_acquire_owned() else {
        return rpc_error(id, -32000, "tool capacity exceeded");
    };
    let Ok(call_id) = random_id() else {
        return response(StatusCode::INTERNAL_SERVER_ERROR, "");
    };
    let (tx, rx) = oneshot::channel();
    let request_key = (sid, id.to_string());
    {
        let mut reg = state.registry.lock().unwrap_or_else(|e| e.into_inner());
        if reg.quiesced {
            return rpc_error(id, -32000, "server is no longer accepting tool calls");
        }
        reg.pending.insert(call_id.clone(), tx);
        reg.requests.insert(request_key.clone(), call_id.clone());
    }
    let cancellation = CancellationToken::new();
    let mut guard = PendingGuard {
        state: state.clone(),
        id: call_id.clone(),
        request_key,
        cancellation: cancellation.clone(),
        completed: false,
    };
    if state
        .calls
        .try_send(ToolCall {
            id: call_id,
            name: name.into(),
            arguments: arguments.clone(),
            cancellation,
        })
        .is_err()
    {
        return rpc_error(id, -32000, "tool queue unavailable");
    }
    let answer = tokio::time::timeout(Duration::from_secs(600), rx).await;
    guard.completed = matches!(answer, Ok(Ok(_)));
    drop(guard);
    drop(permit);
    match answer {
        Ok(Ok(value)) => rpc_result(id, value),
        _ => rpc_error(id, -32000, "tool call cancelled or timed out"),
    }
}
fn random_id() -> Result<String, String> {
    let mut bytes = [0_u8; 24];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| "random source unavailable")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn response(status: StatusCode, text: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::copy_from_slice(text.as_bytes())))
        .unwrap()
}
fn rpc_result(id: Value, value: Value) -> Response<Body> {
    json_response(json!({"jsonrpc":"2.0","id":id,"result":value}))
}
fn rpc_error(id: Value, code: i32, message: &str) -> Response<Body> {
    json_response(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}
fn json_response(value: Value) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}

struct SizeLimit {
    size: usize,
    limit: usize,
}
impl std::io::Write for SizeLimit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.size = self
            .size
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("JSON size exceeded"))?;
        if self.size > self.limit {
            return Err(std::io::Error::other("JSON size exceeded"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn json_size(value: &Value, limit: usize) -> Result<usize, serde_json::Error> {
    let mut writer = SizeLimit { size: 0, limit };
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.size)
}
