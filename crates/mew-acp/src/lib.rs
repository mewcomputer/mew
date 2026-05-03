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
    pub async fn run_turn(
        &mut self,
        text: &str,
    ) -> Result<mpsc::Receiver<mew_agent::AgentEvent>> {
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
// Notification translation
// ---------------------------------------------------------------------------

fn translate_notification(
    msg: &RawMessage,
    ev_tx: &mpsc::Sender<mew_agent::AgentEvent>,
) {
    if msg.method.as_deref() != Some("session/update") {
        return;
    }
    let Some(ref params) = msg.params else { return };
    let Some(update) = params.get("update") else { return };
    let update_type = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match update_type {
        "agent_message_chunk" => {
            if let Some(content) = update.get("content") {
                if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
                    let _ = ev_tx.try_send(mew_agent::AgentEvent::Provider(
                        mew_provider::ProviderEvent::PartStart {
                            part: Part::Text(mew_message::TextPart {
                                base: mew_message::PartBase {
                                    id: ulid::Ulid::new(),
                                    message_id: ulid::Ulid::new(),
                                    session_id: ulid::Ulid::new(),
                                },
                                text: text.to_string(),
                                synthetic: false,
                            }),
                        },
                    ));
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
}
