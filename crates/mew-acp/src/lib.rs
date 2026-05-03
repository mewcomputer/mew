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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, info};

use mew_message::Part;

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

/// Connects to an external ACP agent over stdio.
pub struct AcpClient {
    _child: Child,
    reader: Arc<tokio::sync::Mutex<BufReader<tokio::process::ChildStdout>>>,
    writer: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
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
        let reader = Arc::new(tokio::sync::Mutex::new(BufReader::new(stdout)));
        let writer = Arc::new(tokio::sync::Mutex::new(stdin));

        let mut client = Self {
            _child: child,
            reader,
            writer,
            next_id: AtomicU64::new(1),
            session_id: String::new(),
        };

        // Initialize
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

        // Create session
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
        Ok(client)
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
        let mut writer = self.writer.lock().await;
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
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::BufWriter::new(stdout);

    let mut session_id: Option<String> = None;
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

                let agent = agent.clone();
                let sid = sid.clone();

                // Run the agent and stream events back.
                let rx = agent.run(text.to_string());
                tokio::pin!(rx);

                let mut stop_reason = "end_turn".to_string();
                while let Some(event) = rx.recv().await {
                    match &event {
                        mew_agent::AgentEvent::Provider(pe) => match pe {
                            // PartDelta carries the actual streaming text content.
                            mew_provider::ProviderEvent::PartDelta { field, delta, .. } => {
                                if *field == "text" || field.is_empty() {
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
                                        "status": if *success {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mew_agent::AgentEvent;
    use mew_message::Part;
    use mew_provider::ProviderEvent;

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
}
