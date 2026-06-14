//! ACP integration for mew.
//!
//! Two modes:
//!   client — spawns an ACP agent subprocess, handles prompt turns
//!   server — exposes mew's agent core as an ACP service over stdio

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, info};

use mew_message::Part;

// ---------------------------------------------------------------------------
// Transport trait
// ---------------------------------------------------------------------------

/// Abstract transport for ACP message framing.
///
/// Implementations provide a line-delimited reader and writer. m6 ships
/// StdioTransport; m8 can add iroh/tcp without touching protocol code.
pub trait Transport: Send + 'static {
    type Reader: tokio::io::AsyncBufRead + Unpin + Send;
    type Writer: tokio::io::AsyncWrite + Unpin + Send;

    fn split(self) -> (Self::Reader, Self::Writer);
}

/// Stdio transport for server mode (stdin/stdout).
pub struct StdioTransport {
    _private: (),
}

impl StdioTransport {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Transport for StdioTransport {
    type Reader = BufReader<tokio::io::Stdin>;
    type Writer = BufWriter<tokio::io::Stdout>;

    fn split(self) -> (Self::Reader, Self::Writer) {
        (
            BufReader::new(tokio::io::stdin()),
            BufWriter::new(tokio::io::stdout()),
        )
    }
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}

/// Transport for tests: wraps a duplex pair.
#[cfg(test)]
struct DuplexTransport<
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
> {
    reader: R,
    writer: W,
}

#[cfg(test)]
impl<
        R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    > Transport for DuplexTransport<R, W>
{
    type Reader = R;
    type Writer = W;

    fn split(self) -> (Self::Reader, Self::Writer) {
        (self.reader, self.writer)
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct Request {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Notification {
    jsonrpc: &'static str,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
}

// ---------------------------------------------------------------------------
// ACP Client — spawns an agent, runs prompt turns, translates to AgentEvent
// ---------------------------------------------------------------------------

/// Connects to an external ACP agent over stdio or a transport.
pub struct AcpClient {
    _child: Option<Child>,
    reader: Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncBufRead + Unpin + Send>>>,
    writer: Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncWrite + Unpin + Send>>>,
    next_id: AtomicU64,
    session_id: String,
}

impl AcpClient {
    /// Spawns an ACP agent, initializes the connection, and creates a session.
    pub async fn connect(command: &str, args: &[String], cwd: &str) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("spawn acp agent")?;

        let stdout = child.stdout.take().context("stdout")?;
        let stdin = child.stdin.take().context("stdin")?;
        let reader = Arc::new(tokio::sync::Mutex::new(
            Box::new(BufReader::new(stdout)) as _
        ));
        let writer = Arc::new(tokio::sync::Mutex::new(Box::new(stdin) as _));

        let mut client = Self {
            _child: Some(child),
            reader,
            writer,
            next_id: AtomicU64::new(1),
            session_id: String::new(),
        };

        Self::init_session(&mut client, cwd).await?;
        Ok(client)
    }

    pub async fn from_transport(
        reader: impl tokio::io::AsyncBufRead + Unpin + Send + 'static,
        writer: impl tokio::io::AsyncWrite + Unpin + Send + 'static,
    ) -> Result<Self> {
        let reader = Arc::new(tokio::sync::Mutex::new(Box::new(reader) as _));
        let writer = Arc::new(tokio::sync::Mutex::new(Box::new(writer) as _));

        let mut client = Self {
            _child: None,
            reader,
            writer,
            next_id: AtomicU64::new(1),
            session_id: String::new(),
        };

        Self::init_session(&mut client, "").await?;
        Ok(client)
    }

    async fn init_session(client: &mut Self, cwd: &str) -> Result<()> {
        let _ = client
            .call_rpc(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true },
                        "terminal": true
                    },
                    "clientInfo": {
                        "name": "mew",
                        "title": "mew",
                        "version": "0.1.0"
                    }
                })),
                |_| {},
            )
            .await?;
        info!("acp agent initialized");

        let session_id = client
            .call_rpc(
                "session/new",
                Some(serde_json::json!({
                    "cwd": cwd,
                    "mcpServers": []
                })),
                |_| {},
            )
            .await?
            .context("create session")?;

        client.session_id = session_id;
        Ok(())
    }

    /// Run a single prompt turn. Returns a receiver of agent events that the
    /// TUI can drain, matching the `Agent::run_with_parts` interface.
    pub async fn run_turn(&mut self, text: &str) -> Result<mpsc::Receiver<mew_agent::AgentEvent>> {
        let (ev_tx, ev_rx) = mpsc::channel(256);

        let params = serde_json::json!({
            "sessionId": self.session_id,
            "prompt": [
                { "type": "text", "text": text }
            ]
        });

        let reader = self.reader.clone();
        let writer_arc = self.writer.clone();
        let mut writer = writer_arc.lock().await;
        let next_id = &self.next_id;

        let id = next_id.fetch_add(1, Ordering::SeqCst);
        let req = Request {
            jsonrpc: "2.0",
            id,
            method: "session/prompt".to_string(),
            params: Some(params),
        };
        let line = serde_json::to_string(&req)?;
        debug!("acp → {}", &line[..line.len().min(200)]);
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        drop(writer);

        tokio::spawn(async move {
            let mut r = reader.lock().await;
            loop {
                let mut line_buf = String::new();
                if r.read_line(&mut line_buf).await.is_err() {
                    break;
                }
                let line = line_buf.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                debug!("acp ← {}", &line[..line.len().min(200)]);
                let msg: RawMessage = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if msg.id == Some(id) || (msg.id.is_none() && msg.result.is_some()) {
                    let stop_reason = msg
                        .result
                        .and_then(|v| {
                            v.get("stopReason")
                                .and_then(|s| s.as_str().map(String::from))
                        })
                        .unwrap_or_else(|| "end_turn".to_string());
                    let finish = match stop_reason.as_str() {
                        "cancelled" => mew_message::Finish::Error,
                        "max_tokens" => mew_message::Finish::Length,
                        _ => mew_message::Finish::Stop,
                    };
                    let _ = ev_tx
                        .send(mew_agent::AgentEvent::Provider(
                            mew_provider::ProviderEvent::MessageEnd {
                                finish,
                                usage: mew_message::Tokens::default(),
                                cost: 0.0,
                            },
                        ))
                        .await;
                    break;
                }

                let method = msg.method.as_deref().unwrap_or("");
                if method == "session/request_permission" {
                    handle_client_permission_request(&msg, &ev_tx, &writer_arc).await;
                    continue;
                }

                translate_notification(&msg, &ev_tx);
            }
        });

        Ok(ev_rx)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Send a session/cancel notification to the agent.
    pub async fn cancel(&self) -> Result<()> {
        let notif = Notification {
            jsonrpc: "2.0",
            method: "session/cancel".to_string(),
            params: Some(serde_json::json!({
                "sessionId": self.session_id
            })),
        };
        let line = serde_json::to_string(&notif)?;
        debug!("acp → cancel");
        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes()).await?;
        w.write_all(b"\n").await?;
        w.flush().await?;
        Ok(())
    }

    /// Send a JSON-RPC request and read lines until the response arrives.
    /// Each intermediate notification is passed to `on_notification`.
    /// Returns the `stopReason` extracted from the response result (for
    /// session/prompt) or the full raw result as a JSON string.
    async fn call_rpc<F>(
        &mut self,
        method: &str,
        params: Option<Value>,
        mut on_notification: F,
    ) -> Result<Option<String>>
    where
        F: FnMut(&RawMessage),
    {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = Request {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&req)?;
        debug!("acp → {}", &line[..line.len().min(200)]);
        {
            let mut w = self.writer.lock().await;
            w.write_all(line.as_bytes()).await?;
            w.write_all(b"\n").await?;
            w.flush().await?;
        }

        let mut r = self.reader.lock().await;
        loop {
            let mut line_buf = String::new();
            r.read_line(&mut line_buf).await?;
            let line = line_buf.trim().to_string();
            if line.is_empty() {
                continue;
            }
            debug!("acp ← {}", &line[..line.len().min(200)]);

            let msg: RawMessage = match serde_json::from_str(&line) {
                Ok(m) => m,
                Err(e) => {
                    debug!("skip unparseable line: {e}");
                    continue;
                }
            };

            if msg.id == Some(id) || (msg.id.is_none() && msg.result.is_some()) {
                if let Some(ref err) = msg.error {
                    anyhow::bail!("rpc error: {:?}", err);
                }
                return Ok(msg.result.and_then(|v| {
                    v.get("stopReason")
                        .and_then(|s| s.as_str().map(String::from))
                }));
            }

            on_notification(&msg);
        }
    }
}

// ---------------------------------------------------------------------------
// ACP Server — exposes mew's agent as an ACP service over stdio
// ---------------------------------------------------------------------------

use mew_agent::Agent as AgentCore;

/// Run an ACP server on stdin/stdout, using the provided agent.
/// Reads JSON-RPC requests, runs the agent, and streams updates back.
pub async fn run_server(agent: AgentCore) -> Result<()> {
    run_server_on(agent, StdioTransport::new()).await
}

pub async fn run_server_on<T: Transport>(agent: AgentCore, transport: T) -> Result<()> {
    let (reader, writer) = transport.split();
    let mut lines = BufReader::new(reader).lines();
    let mut stdout = BufWriter::new(writer);

    let mut session_id: Option<String> = None;
    let mut perm_request_id: u64 = 900_000;
    let agent = Arc::new(agent);

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        debug!("acp server ← {}", &line[..line.len().min(200)]);

        let msg: RawMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                debug!("skip unparseable: {e}");
                continue;
            }
        };

        let method = msg.method.as_deref().unwrap_or("");
        let resp_id = msg.id.unwrap_or(0);

        match method {
            "initialize" => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": resp_id,
                    "result": {
                        "protocolVersion": 1,
                        "agentCapabilities": {
                            "loadSession": false,
                            "promptCapabilities": {
                                "image": true
                            },
                            "sessionCapabilities": {}
                        },
                        "agentInfo": {
                            "name": "mew",
                            "title": "mew",
                            "version": "0.1.0"
                        }
                    }
                });
                send_line(&mut stdout, &resp).await?;
            }
            "session/new" => {
                let sid = format!("sess_{}", ulid::Ulid::new());
                session_id = Some(sid.clone());
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": resp_id,
                    "result": {
                        "sessionId": sid
                    }
                });
                send_line(&mut stdout, &resp).await?;

                let notif = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": sid,
                        "update": {
                            "sessionUpdate": "available_commands_update",
                            "availableCommands": available_commands()
                        }
                    }
                });
                send_line(&mut stdout, &notif).await?;
            }
            "session/prompt" => {
                let Some(ref sid) = session_id else {
                    send_error(&mut stdout, resp_id, -32602, "no session").await?;
                    continue;
                };

                // Extract text from prompt params.
                let text = msg
                    .params
                    .as_ref()
                    .and_then(|p| p.get("prompt"))
                    .and_then(|p| p.get(0))
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                info!("acp server: prompt \"{}\"", &text[..text.len().min(80)]);

                let (cmd, _arg) = match text.split_once(' ') {
                    Some((c, a)) => (c, Some(a)),
                    None => (text, None),
                };
                if let Some(response_text) = handle_slash(cmd, &agent).await {
                    let chunk = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": sid,
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": { "type": "text", "text": response_text }
                            }
                        }
                    });
                    send_line(&mut stdout, &chunk).await?;
                    let reply = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": resp_id,
                        "result": { "stopReason": "end_turn" }
                    });
                    send_line(&mut stdout, &reply).await?;
                    continue;
                }

                let agent = agent.clone();
                let sid = sid.clone();

                // Run the agent and stream events back.
                let rx = agent.run(text.to_string());
                tokio::pin!(rx);

                let mut stop_reason = "end_turn".to_string();
                while let Some(event) = rx.recv().await {
                    match event {
                        mew_agent::AgentEvent::Provider(pe) => match pe {
                            mew_provider::ProviderEvent::PartDelta { field, delta, .. } => {
                                if field == "text" || field.is_empty() {
                                    let notif = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "method": "session/update",
                                        "params": {
                                            "sessionId": sid,
                                            "update": {
                                                "sessionUpdate": "agent_message_chunk",
                                                "content": {
                                                    "type": "text",
                                                    "text": delta
                                                }
                                            }
                                        }
                                    });
                                    send_line(&mut stdout, &notif).await?;
                                }
                            }
                            mew_provider::ProviderEvent::MessageEnd { finish, .. } => {
                                stop_reason = match finish {
                                    mew_message::Finish::Stop => "end_turn".into(),
                                    mew_message::Finish::Length => "max_tokens".into(),
                                    mew_message::Finish::ToolUse => "end_turn".into(),
                                    mew_message::Finish::Error => "refusal".into(),
                                };
                            }
                            mew_provider::ProviderEvent::RetryWait {
                                attempt,
                                max_attempts,
                                delay_secs,
                                reason,
                            } => {
                                info!(
                                    "acp server retry {}/{}: {} in {}s",
                                    attempt, max_attempts, reason, delay_secs
                                );
                            }
                            _ => {}
                        },
                        mew_agent::AgentEvent::ToolStart { call_id } => {
                            let notif = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "session/update",
                                "params": {
                                    "sessionId": sid,
                                    "update": {
                                        "sessionUpdate": "tool_call",
                                        "toolCallId": call_id,
                                        "title": call_id,
                                        "status": "in_progress"
                                    }
                                }
                            });
                            send_line(&mut stdout, &notif).await?;
                        }
                        mew_agent::AgentEvent::ToolEnd { call_id, success } => {
                            let notif = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "session/update",
                                "params": {
                                    "sessionId": sid,
                                    "update": {
                                        "sessionUpdate": "tool_call_update",
                                        "toolCallId": call_id,
                                        "status": if success {
                                            "completed"
                                        } else {
                                            "failed"
                                        }
                                    }
                                }
                            });
                            send_line(&mut stdout, &notif).await?;
                        }
                        mew_agent::AgentEvent::Error(msg) => {
                            let notif = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "session/update",
                                "params": {
                                    "sessionId": sid,
                                    "update": {
                                        "sessionUpdate": "agent_message_chunk",
                                        "content": {
                                            "type": "text",
                                            "text": format!("[mew] {msg}")
                                        }
                                    }
                                }
                            });
                            send_line(&mut stdout, &notif).await?;
                        }
                        mew_agent::AgentEvent::PermissionRequest { call, tx } => {
                            let call_id = call.call_id.clone();
                            let tool_name = call.tool_name.clone();
                            let input = call.input.clone();
                            let pid = perm_request_id;
                            perm_request_id += 1;

                            let req = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": pid,
                                "method": "session/request_permission",
                                "params": {
                                    "sessionId": sid,
                                    "toolCall": {
                                        "toolCallId": call_id,
                                        "title": format!("{} permission", tool_name),
                                        "rawInput": input,
                                    },
                                    "options": [
                                        { "optionId": "allow-once", "name": "Allow once", "kind": "allow_once" },
                                        { "optionId": "allow-session", "name": "Allow for session", "kind": "allow_always" },
                                        { "optionId": "reject-once", "name": "Reject", "kind": "reject_once" }
                                    ]
                                }
                            });
                            send_line(&mut stdout, &req).await?;

                            let decision =
                                read_permission_response(&mut lines, &mut stdout, pid, &call_id)
                                    .await;
                            let _ = tx.send(decision);
                        }
                        _ => {}
                    }
                }

                // Send the final prompt response.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": resp_id,
                    "result": {
                        "stopReason": stop_reason
                    }
                });
                send_line(&mut stdout, &resp).await?;
            }
            "session/cancel" => {
                if let Some(ref sid) = session_id {
                    info!("acp server: cancel session {sid}");
                    agent.cancel_token.cancel();
                }
                // No response needed for notification, but if it's a request, respond OK.
                if msg.id.is_some() {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": resp_id,
                        "result": {}
                    });
                    send_line(&mut stdout, &resp).await?;
                }
            }
            _ => {
                debug!("acp server: unknown method {}", method);
            }
        }
    }

    Ok(())
}

async fn send_line(
    w: &mut (impl tokio::io::AsyncWrite + Unpin),
    val: &serde_json::Value,
) -> Result<()> {
    let line = serde_json::to_string(val)?;
    debug!("acp server → {}", &line[..line.len().min(200)]);
    w.write_all(line.as_bytes()).await?;
    w.write_all(b"\n").await?;
    w.flush().await?;
    Ok(())
}

async fn send_error(
    w: &mut (impl tokio::io::AsyncWrite + Unpin),
    id: u64,
    code: i64,
    message: &str,
) -> Result<()> {
    let err = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    });
    send_line(w, &err).await
}

async fn read_permission_response<
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
>(
    _lines: &mut Lines<R>,
    _stdout: &mut W,
    _perm_request_id: u64,
    _call_id: &str,
) -> mew_hooks::PermissionDecision {
    loop {
        let line = match _lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) | Err(_) => break mew_hooks::PermissionDecision::Deny,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        debug!(
            "acp server ← permission response: {}",
            &line[..line.len().min(200)]
        );
        let msg: RawMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if msg.id != Some(_perm_request_id) {
            debug!(
                "acp server: ignoring message with wrong id while waiting for permission response"
            );
            continue;
        }

        if msg.error.is_some() {
            break mew_hooks::PermissionDecision::Deny;
        }

        let outcome = msg
            .result
            .as_ref()
            .and_then(|r| r.get("outcome"))
            .and_then(|o| o.get("outcome"))
            .and_then(|o| o.as_str())
            .unwrap_or("cancelled");

        if outcome == "cancelled" {
            break mew_hooks::PermissionDecision::Deny;
        }

        let option_id = msg
            .result
            .as_ref()
            .and_then(|r| r.get("outcome"))
            .and_then(|o| o.get("optionId"))
            .and_then(|o| o.as_str())
            .unwrap_or("reject-once");

        break if option_id.starts_with("allow") {
            if option_id.contains("always") || option_id.contains("session") {
                mew_hooks::PermissionDecision::AllowSession
            } else {
                mew_hooks::PermissionDecision::AllowOnce
            }
        } else {
            mew_hooks::PermissionDecision::Deny
        };
    }
}

// ---------------------------------------------------------------------------
// Client-side permission handling
// ---------------------------------------------------------------------------

async fn handle_client_permission_request(
    msg: &RawMessage,
    ev_tx: &mpsc::Sender<mew_agent::AgentEvent>,
    writer: &Arc<tokio::sync::Mutex<Box<dyn tokio::io::AsyncWrite + Unpin + Send>>>,
) {
    let request_id = msg.id.unwrap_or(0);
    let tool_call = msg.params.as_ref().and_then(|p| p.get("toolCall"));

    let call_id = tool_call
        .and_then(|tc| tc.get("toolCallId"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tool_name = tool_call
        .and_then(|tc| tc.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let input = tool_call
        .and_then(|tc| tc.get("rawInput"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let (perm_tx, perm_rx) = tokio::sync::oneshot::channel();

    let hook_call = mew_hooks::ToolCall {
        tool_name: tool_name.clone(),
        call_id: call_id.clone(),
        input,
    };

    let _ = ev_tx
        .send(mew_agent::AgentEvent::PermissionRequest {
            call: hook_call,
            tx: perm_tx,
        })
        .await;

    let writer = writer.clone();
    tokio::spawn(async move {
        let decision = match perm_rx.await {
            Ok(d) => d,
            Err(_) => mew_hooks::PermissionDecision::Deny,
        };

        let option_id = match decision {
            mew_hooks::PermissionDecision::AllowOnce => "allow-once",
            mew_hooks::PermissionDecision::AllowSession => "allow-session",
            mew_hooks::PermissionDecision::Deny => "reject-once",
            mew_hooks::PermissionDecision::Prompt => "reject-once",
        };

        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "outcome": {
                    "outcome": "selected",
                    "optionId": option_id
                }
            }
        });
        let mut w = writer.lock().await;
        if let Ok(line) = serde_json::to_string(&resp) {
            let _ = w.write_all(line.as_bytes()).await;
            let _ = w.write_all(b"\n").await;
            let _ = w.flush().await;
        }
    });
}

// ---------------------------------------------------------------------------
// Notification translation (client side: ACP → AgentEvent)
// ---------------------------------------------------------------------------

/// Track state across one notification stream for a single turn.
struct TurnState {
    msg_id: ulid::Ulid,
    session_id: ulid::Ulid,
    started: bool,
}

fn translate_notification(msg: &RawMessage, ev_tx: &mpsc::Sender<mew_agent::AgentEvent>) {
    thread_local! {
        static TURN_STATE: std::cell::RefCell<Option<TurnState>> = const { std::cell::RefCell::new(None) };
    }

    TURN_STATE.with(|ts| {
        let mut ts = ts.borrow_mut();
        let state = ts.get_or_insert_with(|| TurnState {
            msg_id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            started: false,
        });

        if msg.method.as_deref() != Some("session/update") {
            return;
        }
        let Some(ref params) = msg.params else { return };
        let Some(update) = params.get("update") else {
            return;
        };
        let update_type = update
            .get("sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match update_type {
            "agent_message_chunk" => {
                if let Some(content) = update.get("content") {
                    if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
                        // Send PartStart on the first chunk, then PartDelta on subsequent.
                        if !state.started {
                            let _ = ev_tx.try_send(mew_agent::AgentEvent::Provider(
                                mew_provider::ProviderEvent::PartStart {
                                    part: Part::Text(mew_message::TextPart {
                                        base: mew_message::PartBase {
                                            id: state.msg_id,
                                            message_id: state.msg_id,
                                            session_id: state.session_id,
                                        },
                                        text: text.to_string(),
                                        synthetic: false,
                                    }),
                                },
                            ));
                            state.started = true;
                        } else {
                            let _ = ev_tx.try_send(mew_agent::AgentEvent::Provider(
                                mew_provider::ProviderEvent::PartDelta {
                                    part_id: state.msg_id,
                                    field: "text",
                                    delta: text.to_string(),
                                },
                            ));
                        }
                    }
                }
            }
            "tool_call" => {
                let id = update
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let _ = ev_tx.try_send(mew_agent::AgentEvent::ToolStart {
                    call_id: id.to_string(),
                });
            }
            "tool_call_update" => {
                let id = update
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if status == "completed" || status == "failed" {
                    let _ = ev_tx.try_send(mew_agent::AgentEvent::ToolEnd {
                        call_id: id.to_string(),
                        success: status == "completed",
                    });
                }
            }
            _ => {}
        }
    });
}

// ---------------------------------------------------------------------------
// Slash command definitions and handlers (server-side)
// ---------------------------------------------------------------------------

async fn handle_slash(cmd: &str, agent: &mew_agent::Agent) -> Option<String> {
    match cmd {
        "/compact" => {
            agent.force_compact().await;
            Some("[mew] compaction will run on next turn\n".into())
        }
        "/clear" => {
            agent.load_messages(vec![]).await;
            Some("[mew] conversation cleared\n".into())
        }
        "/cost" => Some(build_cost_report(agent).await),
        "/help" => Some(build_help_text()),
        _ => None,
    }
}

fn available_commands() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "compact",
            "description": "force context compaction on next turn",
        }),
        serde_json::json!({
            "name": "clear",
            "description": "clear conversation history",
        }),
        serde_json::json!({
            "name": "cost",
            "description": "show session cost breakdown",
        }),
        serde_json::json!({
            "name": "help",
            "description": "show available slash commands",
        }),
    ]
}

fn build_help_text() -> String {
    let mut out = String::from("[mew] available commands:\n");
    for cmd in available_commands() {
        let name = cmd["name"].as_str().unwrap_or("");
        let desc = cmd["description"].as_str().unwrap_or("");
        out.push_str(&format!("  /{name} — {desc}\n"));
    }
    out
}

async fn build_cost_report(agent: &mew_agent::Agent) -> String {
    let messages = agent.messages.lock().await;
    let mut total = 0f64;
    let mut turns: Vec<(usize, f64)> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if let Some(ref meta) = msg.assistant {
            if meta.cost > 0.0 {
                total += meta.cost;
                turns.push((i, meta.cost));
            }
        }
    }
    let mut report = format!("[mew] session cost: ${total:.4}\n");
    if turns.is_empty() {
        report.push_str("no recorded costs yet");
    } else {
        report.push_str("per-turn breakdown:\n");
        for (idx, cost) in turns {
            report.push_str(&format!("  turn {idx}: ${cost:.4}\n"));
        }
    }
    report
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = Request {
            jsonrpc: "2.0",
            id: 1,
            method: "initialize".to_string(),
            params: Some(serde_json::json!({"protocolVersion": 1})),
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(line.contains("\"id\":1"));
        assert!(line.contains("\"method\":\"initialize\""));
        assert!(line.contains("\"protocolVersion\":1"));
    }

    #[test]
    fn test_raw_message_parsing() {
        let line = r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#;
        let msg: RawMessage = serde_json::from_str(line).unwrap();
        assert_eq!(msg.id, Some(1));
        assert_eq!(
            msg.result.and_then(|v| v
                .get("stopReason")
                .and_then(|s| s.as_str().map(String::from))),
            Some("end_turn".to_string())
        );
    }

    #[test]
    fn test_raw_message_notification() {
        let line = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}}"#;
        let msg: RawMessage = serde_json::from_str(line).unwrap();
        assert_eq!(msg.method, Some("session/update".to_string()));
        assert_eq!(msg.id, None);
    }

    #[test]
    fn test_translate_text_chunk() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let msg = RawMessage {
            id: None,
            method: Some("session/update".to_string()),
            params: Some(serde_json::json!({
                "sessionId": "s1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "type": "text",
                        "text": "hello world"
                    }
                }
            })),
            result: None,
            error: None,
        };
        translate_notification(&msg, &tx);
        drop(tx);
        // Verify there's an event in the channel.
        let events: Vec<_> = rx.blocking_recv().into_iter().collect();
        // Note: translate_notification uses try_send which may fail silently.
        // The channel has capacity 1, so it should succeed.
        assert!(!events.is_empty());
    }

    #[test]
    fn test_translate_tool_call() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let msg = RawMessage {
            id: None,
            method: Some("session/update".to_string()),
            params: Some(serde_json::json!({
                "sessionId": "s1",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call_1",
                    "title": "reading file",
                    "kind": "read",
                    "status": "pending"
                }
            })),
            result: None,
            error: None,
        };
        translate_notification(&msg, &tx);
        drop(tx);
        let events: Vec<_> = rx.blocking_recv().into_iter().collect();
        assert!(!events.is_empty());
    }

    #[test]
    fn test_translate_tool_call_update() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let msg = RawMessage {
            id: None,
            method: Some("session/update".to_string()),
            params: Some(serde_json::json!({
                "sessionId": "s1",
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_1",
                    "status": "completed"
                }
            })),
            result: None,
            error: None,
        };
        translate_notification(&msg, &tx);
        drop(tx);
        let events: Vec<_> = rx.blocking_recv().into_iter().collect();
        assert!(!events.is_empty());
    }

    #[test]
    fn test_translate_unknown_ignored() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let msg = RawMessage {
            id: None,
            method: Some("session/update".to_string()),
            params: Some(serde_json::json!({
                "sessionId": "s1",
                "update": {
                    "sessionUpdate": "unknown_type"
                }
            })),
            result: None,
            error: None,
        };
        translate_notification(&msg, &tx);
        // Unknown types should not panic and not send events.
    }

    #[tokio::test]
    async fn test_server_initialize_and_session() {
        use serde_json::Value;
        use std::sync::Arc;
        let provider = Arc::new(mew_provider_fake::FakeProvider::new(
            mew_provider_fake::FakeProvider::text_response("hello"),
        ));
        let dispatcher = Arc::new(mew_hooks::NopDispatcher);
        let agent = mew_agent::Agent::new(provider, dispatcher, None, vec![], None);
        let (cr, sw) = tokio::io::duplex(4096);
        let (sr, cw) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = run_server_on(
                agent,
                DuplexTransport {
                    reader: BufReader::new(sr),
                    writer: sw,
                },
            )
            .await;
        });
        let mut reader = BufReader::new(cr);
        let mut writer = tokio::io::BufWriter::new(cw);
        let init = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}});
        send_line(&mut writer, &init).await.unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(resp["id"], 1);
        let new_sess = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}});
        send_line(&mut writer, &new_sess).await.unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let sid = serde_json::from_str::<Value>(line.trim()).unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_string();
        let prompt = serde_json::json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":sid,"prompt":[{"type":"text","text":"hi"}]}});
        send_line(&mut writer, &prompt).await.unwrap();
        let got_response = loop {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let msg: Value = serde_json::from_str(line.trim()).unwrap();
            if msg.get("id").and_then(|v| v.as_u64()) == Some(3) {
                break true;
            }
        };
        assert!(got_response);
    }

    #[tokio::test]
    async fn test_server_sends_available_commands_after_session_new() {
        use serde_json::Value;
        use std::sync::Arc;

        let provider = Arc::new(mew_provider_fake::FakeProvider::new(
            mew_provider_fake::FakeProvider::text_response("ok"),
        ));
        let dispatcher = Arc::new(mew_hooks::NopDispatcher);
        let agent = mew_agent::Agent::new(provider, dispatcher, None, vec![], None);
        let (cr, sw) = tokio::io::duplex(4096);
        let (sr, cw) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = run_server_on(
                agent,
                DuplexTransport {
                    reader: BufReader::new(sr),
                    writer: sw,
                },
            )
            .await;
        });
        let mut reader = BufReader::new(cr);
        let mut writer = tokio::io::BufWriter::new(cw);

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await
        .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
        )
        .await
        .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let _sess_resp: Value = serde_json::from_str(line.trim()).unwrap();

        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let notif: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(notif["method"], "session/update");
        let update = &notif["params"]["update"];
        assert_eq!(update["sessionUpdate"], "available_commands_update");
        let cmds = update["availableCommands"].as_array().unwrap();
        assert!(!cmds.is_empty());
        let names: Vec<&str> = cmds.iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"compact"));
        assert!(names.contains(&"clear"));
        assert!(names.contains(&"help"));
        assert!(names.contains(&"cost"));
    }

    #[tokio::test]
    async fn test_server_slash_commands() {
        use serde_json::Value;
        use std::sync::Arc;

        let provider = Arc::new(mew_provider_fake::FakeProvider::new(
            mew_provider_fake::FakeProvider::text_response("ok"),
        ));
        let dispatcher = Arc::new(mew_hooks::NopDispatcher);
        let agent = mew_agent::Agent::new(provider, dispatcher, None, vec![], None);
        let (cr, sw) = tokio::io::duplex(4096);
        let (sr, cw) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = run_server_on(
                agent,
                DuplexTransport {
                    reader: BufReader::new(sr),
                    writer: sw,
                },
            )
            .await;
        });
        let mut reader = BufReader::new(cr);
        let mut writer = tokio::io::BufWriter::new(cw);

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await
        .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
        )
        .await
        .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let sid = serde_json::from_str::<Value>(line.trim()).unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_string();
        line.clear();
        reader.read_line(&mut line).await.unwrap();

        let help_prompt = serde_json::json!({
            "jsonrpc": "2.0", "id": 10,
            "method": "session/prompt",
            "params": { "sessionId": sid, "prompt": [{ "type": "text", "text": "/help" }] }
        });
        send_line(&mut writer, &help_prompt).await.unwrap();

        let mut got_chunk = false;
        loop {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let msg: Value = serde_json::from_str(line.trim()).unwrap();
            if msg.get("id").and_then(|v| v.as_u64()) == Some(10) {
                assert_eq!(msg["result"]["stopReason"], "end_turn");
                break;
            }
            if msg["method"] == "session/update" {
                let text = msg["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or("");
                if text.contains("/compact") && text.contains("/help") {
                    got_chunk = true;
                }
            }
        }
        assert!(got_chunk, "expected help text in notification");
    }

    #[tokio::test]
    async fn test_server_multi_turn_with_tool_call() {
        use async_trait::async_trait;
        use mew_hooks::ToolOutput;
        use mew_provider::{EventStream, Provider, ProviderError, ProviderEvent, Request};
        use mew_tools::Tool;
        use mew_tools::{Sensitivity, ToolCtx, ToolError};
        use serde_json::Value;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        struct TestEcho;

        impl TestEcho {
            fn schema_value() -> &'static serde_json::Value {
                static SCHEMA: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::json!({
                            "type": "object",
                            "properties": { "message": { "type": "string" } },
                            "required": ["message"]
                        })
                    });
                &SCHEMA
            }
        }

        #[async_trait]
        impl Tool for TestEcho {
            fn name(&self) -> &str {
                "echo"
            }
            fn description(&self) -> &str {
                "echo input back"
            }
            fn schema(&self) -> &serde_json::Value {
                Self::schema_value()
            }
            fn sensitivity(&self) -> Sensitivity {
                Sensitivity::ReadOnly
            }
            async fn execute(
                &self,
                _ctx: ToolCtx,
                input: serde_json::Value,
            ) -> Result<ToolOutput, ToolError> {
                let msg = input["message"].as_str().unwrap_or("");
                Ok(ToolOutput {
                    output: msg.to_string(),
                    error: String::new(),
                    diff: None,
                })
            }
        }

        struct TwoPhaseProvider {
            call_count: AtomicU32,
            tool_script: Vec<ProviderEvent>,
            text_script: Vec<ProviderEvent>,
        }

        #[async_trait]
        impl Provider for TwoPhaseProvider {
            fn name(&self) -> &str {
                "twophase"
            }
            async fn stream(&self, _req: Request) -> Result<EventStream, ProviderError> {
                let n = self.call_count.fetch_add(1, Ordering::SeqCst);
                let script = if n == 0 {
                    self.tool_script.clone()
                } else {
                    self.text_script.clone()
                };
                let stream = futures::stream::unfold(
                    script.into_iter(),
                    |mut iter: std::vec::IntoIter<ProviderEvent>| async move {
                        match iter.next() {
                            Some(ev) => {
                                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                                Some((ev, iter))
                            }
                            None => None,
                        }
                    },
                );
                Ok(Box::pin(stream))
            }
        }

        let tool_script = mew_provider_fake::FakeProvider::tool_call(
            "echo",
            "call_1",
            serde_json::json!({"message": "hi"}),
        );
        let text_script = mew_provider_fake::FakeProvider::text_response("done");

        let provider = Arc::new(TwoPhaseProvider {
            call_count: AtomicU32::new(0),
            tool_script,
            text_script,
        });
        let dispatcher = Arc::new(mew_hooks::NopDispatcher);
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(TestEcho)];
        let agent = mew_agent::Agent::new(provider, dispatcher, None, tools, None);

        let (cr, sw) = tokio::io::duplex(8192);
        let (sr, cw) = tokio::io::duplex(8192);
        tokio::spawn(async move {
            let _ = run_server_on(
                agent,
                DuplexTransport {
                    reader: BufReader::new(sr),
                    writer: sw,
                },
            )
            .await;
        });
        let mut reader = BufReader::new(cr);
        let mut writer = tokio::io::BufWriter::new(cw);

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await
        .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
        )
        .await
        .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let sid = serde_json::from_str::<Value>(line.trim()).unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_string();
        line.clear();
        reader.read_line(&mut line).await.unwrap();

        let prompt = serde_json::json!({
            "jsonrpc": "2.0", "id": 10,
            "method": "session/prompt",
            "params": { "sessionId": sid, "prompt": [{ "type": "text", "text": "use echo" }] }
        });
        send_line(&mut writer, &prompt).await.unwrap();

        let mut got_tool_start = false;
        let mut got_tool_end = false;
        let mut got_text = false;
        loop {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let msg: Value = serde_json::from_str(line.trim()).unwrap();
            if msg.get("id").and_then(|v| v.as_u64()) == Some(10) {
                assert_eq!(msg["result"]["stopReason"], "end_turn");
                break;
            }
            if msg["method"] != "session/update" {
                continue;
            }
            let update_type = msg["params"]["update"]["sessionUpdate"]
                .as_str()
                .unwrap_or("");
            match update_type {
                "tool_call" => {
                    got_tool_start = true;
                }
                "tool_call_update" => {
                    let status = msg["params"]["update"]["status"].as_str().unwrap_or("");
                    if status == "completed" {
                        got_tool_end = true;
                    }
                }
                "agent_message_chunk" => {
                    let text = msg["params"]["update"]["content"]["text"]
                        .as_str()
                        .unwrap_or("");
                    if !text.is_empty() {
                        got_text = true;
                    }
                }
                _ => {}
            }
        }
        assert!(got_tool_start, "expected tool_call notification");
        assert!(got_tool_end, "expected tool_call_update completed");
        assert!(got_text, "expected text after tool execution");
    }

    #[tokio::test]
    async fn test_server_permission_request_flow() {
        use async_trait::async_trait;
        use mew_hooks::ToolOutput;
        use mew_provider::{EventStream, Provider, ProviderError, ProviderEvent, Request};
        use mew_tools::{Sensitivity, Tool, ToolCtx, ToolError};
        use serde_json::Value;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        struct TestWrite;

        impl TestWrite {
            fn schema_value() -> &'static serde_json::Value {
                static SCHEMA: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::json!({
                            "type": "object",
                            "properties": { "path": { "type": "string" } },
                            "required": ["path"]
                        })
                    });
                &SCHEMA
            }
        }

        #[async_trait]
        impl Tool for TestWrite {
            fn name(&self) -> &str {
                "test_write"
            }
            fn description(&self) -> &str {
                "a mutating tool for testing"
            }
            fn schema(&self) -> &serde_json::Value {
                Self::schema_value()
            }
            fn sensitivity(&self) -> Sensitivity {
                Sensitivity::Mutating
            }
            async fn execute(
                &self,
                _ctx: ToolCtx,
                input: serde_json::Value,
            ) -> Result<ToolOutput, ToolError> {
                let path = input["path"].as_str().unwrap_or("");
                Ok(ToolOutput {
                    output: format!("wrote {path}"),
                    error: String::new(),
                    diff: None,
                })
            }
        }

        struct TwoPhaseProvider {
            call_count: AtomicU32,
            tool_script: Vec<ProviderEvent>,
            text_script: Vec<ProviderEvent>,
        }

        #[async_trait]
        impl Provider for TwoPhaseProvider {
            fn name(&self) -> &str {
                "twophase"
            }
            async fn stream(&self, _req: Request) -> Result<EventStream, ProviderError> {
                let n = self.call_count.fetch_add(1, Ordering::SeqCst);
                let script = if n == 0 {
                    self.tool_script.clone()
                } else {
                    self.text_script.clone()
                };
                let stream = futures::stream::unfold(
                    script.into_iter(),
                    |mut iter: std::vec::IntoIter<ProviderEvent>| async move {
                        match iter.next() {
                            Some(ev) => {
                                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                                Some((ev, iter))
                            }
                            None => None,
                        }
                    },
                );
                Ok(Box::pin(stream))
            }
        }

        let tool_script = mew_provider_fake::FakeProvider::tool_call(
            "test_write",
            "perm_1",
            serde_json::json!({"path": "/tmp/test"}),
        );
        let text_script = mew_provider_fake::FakeProvider::text_response("written");

        let provider = Arc::new(TwoPhaseProvider {
            call_count: AtomicU32::new(0),
            tool_script,
            text_script,
        });
        let dispatcher = Arc::new(mew_hooks::NopDispatcher);
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(TestWrite)];
        let agent = mew_agent::Agent::new(provider, dispatcher, None, tools, None);

        let (cr, sw) = tokio::io::duplex(8192);
        let (sr, cw) = tokio::io::duplex(8192);
        tokio::spawn(async move {
            let _ = run_server_on(
                agent,
                DuplexTransport {
                    reader: BufReader::new(sr),
                    writer: sw,
                },
            )
            .await;
        });
        let mut reader = BufReader::new(cr);
        let mut writer = tokio::io::BufWriter::new(cw);

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await
        .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        send_line(
            &mut writer,
            &serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}),
        )
        .await
        .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let sid = serde_json::from_str::<Value>(line.trim()).unwrap()["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_string();
        line.clear();
        reader.read_line(&mut line).await.unwrap();

        let prompt = serde_json::json!({
            "jsonrpc": "2.0", "id": 10,
            "method": "session/prompt",
            "params": { "sessionId": sid, "prompt": [{ "type": "text", "text": "write file" }] }
        });
        send_line(&mut writer, &prompt).await.unwrap();

        let mut got_permission_request = false;
        let mut got_tool_completed = false;
        let mut got_text = false;
        loop {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let msg: Value = serde_json::from_str(line.trim()).unwrap();

            if msg.get("id").and_then(|v| v.as_u64()) == Some(10) {
                assert_eq!(msg["result"]["stopReason"], "end_turn");
                break;
            }

            let method = msg["method"].as_str().unwrap_or("");
            if method == "session/request_permission" {
                got_permission_request = true;
                let perm_id = msg["id"].as_u64().unwrap();
                assert_eq!(msg["params"]["toolCall"]["toolCallId"], "perm_1");

                let perm_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": perm_id,
                    "result": {
                        "outcome": {
                            "outcome": "selected",
                            "optionId": "allow-once"
                        }
                    }
                });
                send_line(&mut writer, &perm_resp).await.unwrap();
                continue;
            }

            if method != "session/update" {
                continue;
            }
            let update_type = msg["params"]["update"]["sessionUpdate"]
                .as_str()
                .unwrap_or("");

            match update_type {
                "tool_call_update" => {
                    let status = msg["params"]["update"]["status"].as_str().unwrap_or("");
                    if status == "completed" {
                        got_tool_completed = true;
                    }
                }
                "agent_message_chunk" => {
                    let text = msg["params"]["update"]["content"]["text"]
                        .as_str()
                        .unwrap_or("");
                    if !text.is_empty() {
                        got_text = true;
                    }
                }
                _ => {}
            }
        }
        assert!(
            got_permission_request,
            "expected session/request_permission"
        );
        assert!(got_tool_completed, "expected tool completed after approval");
        assert!(got_text, "expected text after tool execution");
    }
}

#[test]
fn test_available_commands_format() {
    let cmds = available_commands();
    assert_eq!(cmds.len(), 4);
    for cmd in &cmds {
        assert!(cmd["name"].is_string());
        assert!(cmd["description"].is_string());
    }
    let names: Vec<&str> = cmds.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"compact"));
    assert!(names.contains(&"clear"));
    assert!(names.contains(&"cost"));
    assert!(names.contains(&"help"));
}

#[test]
fn test_build_help_text() {
    let help = build_help_text();
    assert!(help.contains("/compact"));
    assert!(help.contains("/clear"));
    assert!(help.contains("/cost"));
    assert!(help.contains("/help"));
}

#[tokio::test]
async fn test_handle_slash_known_commands() {
    let provider = Arc::new(mew_provider_fake::FakeProvider::new(
        mew_provider_fake::FakeProvider::text_response("ok"),
    ));
    let dispatcher = Arc::new(mew_hooks::NopDispatcher);
    let agent = mew_agent::Agent::new(provider, dispatcher, None, vec![], None);

    assert!(handle_slash("/compact", &agent).await.is_some());
    assert!(handle_slash("/clear", &agent).await.is_some());
    assert!(handle_slash("/cost", &agent).await.is_some());
    assert!(handle_slash("/help", &agent).await.is_some());
    assert!(handle_slash("/unknown", &agent).await.is_none());
    assert!(handle_slash("hello", &agent).await.is_none());
}

#[tokio::test]
async fn test_handle_slash_clear_empties_messages() {
    let provider = Arc::new(mew_provider_fake::FakeProvider::new(
        mew_provider_fake::FakeProvider::text_response("ok"),
    ));
    let dispatcher = Arc::new(mew_hooks::NopDispatcher);
    let agent = mew_agent::Agent::new(provider, dispatcher, None, vec![], None);

    agent.messages.lock().await.push(mew_message::Message {
        id: ulid::Ulid::new(),
        session_id: ulid::Ulid::new(),
        role: mew_message::Role::User,
        parts: vec![mew_message::Part::Text(mew_message::TextPart {
            base: mew_message::PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: "hello".into(),
            synthetic: false,
        })],
        time: mew_message::Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    });
    assert_eq!(agent.messages.lock().await.len(), 1);

    let result = handle_slash("/clear", &agent).await;
    assert!(result.is_some());
    assert!(result.unwrap().contains("cleared"));
    assert!(agent.messages.lock().await.is_empty());
}
