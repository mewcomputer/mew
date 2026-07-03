//! Client for connecting to a mew daemon.
//!
//! The `DaemonClient` connects to a daemon's WebSocket, sends commands
//! (NewSession, Prompt, Cancel, PermissionResponse), and translates
//! `ServerMessage`s back into `AgentEvent`s — the same `AgentEvent` the
//! TUI already knows how to handle.
//!
//! ## Channel bridging
//!
//! `AgentEvent` variants like `PermissionRequest` carry a `oneshot::Sender`
//! so the TUI can respond. On the client side, we reconstruct these by
//! creating a fresh `oneshot::channel` for each incoming wire request.
//! The `Receiver` goes into the `AgentEvent` (handed to the TUI); the
//! `Sender` is awaited by a background task that forwards the decision
//! back to the daemon as a `ClientMessage::PermissionResponse`.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use mew_agent::{
    AgentEvent, AskUserQuestion, QuestionOption as AgentQuestionOption, Todo, TodoStatus,
};
use mew_hooks::{PermissionDecision, ToolCall as HookToolCall};
use mew_protocol::{ClientMessage, PermissionDecision as WirePermissionDecision, ServerMessage};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async;
use tracing::warn;
use tungstenite::Message;

/// Shared state for pending request/response pairs.
struct ClientState {
    /// `request_id → receiver` for permission decisions.
    /// The receiver is awaited by a spawned task that forwards the
    /// decision back to the daemon.
    pending_permissions: Mutex<HashMap<u64, oneshot::Receiver<PermissionDecision>>>,
    /// `request_id → receiver` for ask-user answers.
    pending_ask_user: Mutex<HashMap<u64, oneshot::Receiver<Vec<String>>>>,
    /// The current event sender (set by `prompt()`, cleared when the
    /// receiver is dropped). The background reader uses this to forward
    /// translated AgentEvents.
    event_tx: Mutex<Option<mpsc::Sender<AgentEvent>>>,
    /// Outgoing message channel (JSON strings to the WebSocket).
    ws_out: mpsc::Sender<String>,
    /// Session ID set when `SessionReady` arrives.
    session_id: Mutex<Option<String>>,
}

/// A client connected to a mew daemon.
///
/// Exposes `prompt(text) -> mpsc::Receiver<AgentEvent>` — the same
/// interface as `Agent::run()`. The TUI can swap `agent.run(prompt)`
/// for `client.prompt(prompt)` without touching the event loop.
pub struct DaemonClient {
    state: Arc<ClientState>,
}

impl DaemonClient {
    /// Connect to a daemon at the given WebSocket URL.
    pub async fn connect(url: &str) -> Result<Self> {
        let (ws_stream, _response) = connect_async(url).await.context("connect to daemon")?;

        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(64);

        // Forward outgoing JSON to the WebSocket.
        tokio::spawn(async move {
            while let Some(json) = outgoing_rx.recv().await {
                if ws_tx.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        });

        let state = Arc::new(ClientState {
            pending_permissions: Mutex::new(HashMap::new()),
            pending_ask_user: Mutex::new(HashMap::new()),
            event_tx: Mutex::new(None),
            ws_out: outgoing_tx.clone(),
            session_id: Mutex::new(None),
        });

        let state_clone = state.clone();

        // Background reader: translate ServerMessage → AgentEvent.
        tokio::spawn(async move {
            while let Some(msg) = ws_rx.next().await {
                let text = match msg {
                    Ok(Message::Text(t)) => t.to_string(),
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(e) => {
                        warn!(error = %e, "daemon client: websocket read error");
                        break;
                    }
                };

                let server_msg: ServerMessage = match mew_protocol::decode_json(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(error = %e, "daemon client: decode failed");
                        continue;
                    }
                };

                // SessionReady is not an AgentEvent — the caller handles it
                // via new_session(). Skip forwarding for those. But we do
                // capture the session ID for later use.
                if let ServerMessage::SessionReady { ref session_id, .. } = server_msg {
                    *state_clone.session_id.lock().await = Some(session_id.clone());
                    continue;
                }

                let events = translate_server_message(&server_msg, &state_clone).await;

                let event_tx = {
                    let guard = state_clone.event_tx.lock().await;
                    guard.clone()
                };

                if let Some(event_tx) = event_tx {
                    for event in events {
                        if event_tx.send(event).await.is_err() {
                            // Receiver dropped — turn ended.
                            break;
                        }
                    }
                } else {
                    warn!("daemon client: event without active prompt");
                }
            }
        });

        Ok(Self { state })
    }

    /// Create a new session on the daemon.
    pub async fn new_session(&self) -> Result<()> {
        let msg = ClientMessage::NewSession {
            cwd: None,
            client_kind: mew_protocol::ClientKind::Tui,
        };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .context("send NewSession")?;
        Ok(())
    }

    /// Attach to an existing session on the daemon.
    pub async fn attach_session(&self, session_id: &str) -> Result<()> {
        let msg = ClientMessage::AttachSession {
            session_id: session_id.to_string(),
            client_kind: mew_protocol::ClientKind::Tui,
        };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .context("send AttachSession")?;
        Ok(())
    }

    /// Send a prompt and return an `AgentEvent` stream — same interface as
    /// `Agent::run()`.
    pub async fn prompt(&self, text: String) -> mpsc::Receiver<AgentEvent> {
        let (event_tx, event_rx) = mpsc::channel(256);

        // Set the event sender so the background reader forwards events.
        *self.state.event_tx.lock().await = Some(event_tx);

        // Send the prompt message.
        let msg = ClientMessage::Prompt {
            text,
            attachments: vec![],
        };
        let json = mew_protocol::encode_json(&msg).unwrap_or_default();
        let _ = self.state.ws_out.send(json).await;

        event_rx
    }

    /// Cancel the current turn.
    pub async fn cancel(&self) {
        let msg = ClientMessage::Cancel;
        let json = mew_protocol::encode_json(&msg).unwrap_or_default();
        let _ = self.state.ws_out.send(json).await;
    }

    /// Send a raw protocol message (for messages not covered by a dedicated
    /// method, e.g. `YieldControl`).
    pub async fn send_raw(&self, json: &str) -> Result<()> {
        self.state
            .ws_out
            .send(json.to_string())
            .await
            .context("send_raw")?;
        Ok(())
    }

    /// Send a slash command to the daemon.
    pub async fn slash_command(&self, command: String) {
        let msg = ClientMessage::SlashCommand { command };
        let json = mew_protocol::encode_json(&msg).unwrap_or_default();
        let _ = self.state.ws_out.send(json).await;
    }

    /// Returns the current session ID, if known (set after `SessionReady`).
    pub async fn session_id(&self) -> Option<String> {
        self.state.session_id.lock().await.clone()
    }

    /// Clear the event sender (called when the turn ends).
    pub async fn clear_event_channel(&self) {
        *self.state.event_tx.lock().await = None;
    }
}

/// Translate a `ServerMessage` into zero or more `AgentEvent`s.
/// Channel-bearing wire messages get fresh `oneshot::channel`s; the
/// `Receiver` goes into the `AgentEvent`, and a spawned task forwards
/// the response back to the daemon.
async fn translate_server_message(
    msg: &ServerMessage,
    state: &Arc<ClientState>,
) -> Vec<AgentEvent> {
    match msg {
        ServerMessage::SessionReady { .. } => Vec::new(),

        ServerMessage::Error { message } => {
            vec![AgentEvent::Error(message.clone())]
        }

        ServerMessage::Provider { event } => {
            // Convert ProviderEventWire back to ProviderEvent.
            // The TUI matches on ProviderEvent variants — we need to
            // reconstruct them. The `field` becomes a &'static str via
            // leak (acceptable for the minimal slice; these are short-lived).
            match wire_to_provider_event(event) {
                Some(pe) => vec![AgentEvent::Provider(pe)],
                None => Vec::new(),
            }
        }

        ServerMessage::UserMessage { .. } => Vec::new(),

        ServerMessage::ToolStart { call_id } => {
            vec![AgentEvent::ToolStart {
                call_id: call_id.clone(),
            }]
        }

        ServerMessage::ToolEnd { call_id, success } => {
            vec![AgentEvent::ToolEnd {
                call_id: call_id.clone(),
                success: *success,
            }]
        }

        ServerMessage::PartUpdated { part_id, part } => {
            vec![AgentEvent::PartUpdated {
                part_id: *part_id,
                part: part.clone(),
            }]
        }

        ServerMessage::ToolProgress { call_id, chunk } => {
            vec![AgentEvent::ToolProgress {
                call_id: call_id.clone(),
                chunk: chunk.clone(),
            }]
        }

        ServerMessage::ErrorEvent { message } => {
            vec![AgentEvent::Error(message.clone())]
        }

        ServerMessage::PermissionRequest {
            request_id,
            tool_name,
            input,
        } => {
            let (tx, rx) = oneshot::channel();
            state
                .pending_permissions
                .lock()
                .await
                .insert(*request_id, rx);

            spawn_permission_forwarder(*request_id, state);

            vec![AgentEvent::PermissionRequest {
                call: HookToolCall {
                    tool_name: tool_name.clone(),
                    call_id: String::new(),
                    input: input.clone(),
                },
                tx,
            }]
        }

        ServerMessage::WorkspacePermissionRequest { request_id, path } => {
            let (tx, rx) = oneshot::channel();
            state
                .pending_permissions
                .lock()
                .await
                .insert(*request_id, rx);

            spawn_permission_forwarder(*request_id, state);

            vec![AgentEvent::WorkspacePermissionRequest {
                path: std::path::PathBuf::from(path),
                tx,
            }]
        }

        ServerMessage::AskUserRequest {
            request_id,
            call_id,
            questions,
        } => {
            let (tx, rx) = oneshot::channel();
            state.pending_ask_user.lock().await.insert(*request_id, rx);

            let request_id = *request_id;
            let state = state.clone();
            tokio::spawn(async move {
                let rx = {
                    let mut guard = state.pending_ask_user.lock().await;
                    guard.remove(&request_id)
                };
                if let Some(rx) = rx {
                    if let Ok(answers) = rx.await {
                        let msg = ClientMessage::AskUserResponse {
                            request_id,
                            answers,
                        };
                        if let Ok(json) = mew_protocol::encode_json(&msg) {
                            let _ = state.ws_out.send(json).await;
                        }
                    }
                }
            });

            vec![AgentEvent::AskUser {
                call_id: call_id.clone(),
                questions: questions
                    .iter()
                    .map(|q| AskUserQuestion {
                        prompt: q.prompt.clone(),
                        options: q
                            .options
                            .iter()
                            .map(|o| AgentQuestionOption {
                                label: o.label.clone(),
                                description: o.description.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
                tx,
            }]
        }

        ServerMessage::SubagentStart {
            parent_call_id,
            name,
            child_session_id,
            display_name,
        } => {
            vec![AgentEvent::SubagentStart {
                parent_call_id: parent_call_id.clone(),
                name: name.clone(),
                child_session_id: child_session_id.clone(),
                display_name: display_name.clone(),
            }]
        }

        ServerMessage::SubagentStatus {
            parent_call_id,
            tool_name,
            message,
        } => {
            vec![AgentEvent::SubagentStatus {
                parent_call_id: parent_call_id.clone(),
                tool_name: tool_name.clone(),
                message: message.clone(),
            }]
        }

        ServerMessage::SubagentEnd {
            parent_call_id,
            child_session_id,
            outcome,
        } => {
            let agent_outcome = match outcome {
                mew_protocol::SubagentOutcome::Completed => {
                    mew_subagents::SubagentOutcome::Completed
                }
                mew_protocol::SubagentOutcome::Cancelled => {
                    mew_subagents::SubagentOutcome::Cancelled
                }
                mew_protocol::SubagentOutcome::Failed { reason } => {
                    mew_subagents::SubagentOutcome::Failed {
                        reason: reason.clone(),
                    }
                }
            };
            vec![AgentEvent::SubagentEnd {
                parent_call_id: parent_call_id.clone(),
                child_session_id: child_session_id.clone(),
                outcome: agent_outcome,
            }]
        }

        ServerMessage::SubagentPermissionRequest {
            request_id,
            parent_call_id,
            tool_name,
            input,
        } => {
            let (tx, rx) = oneshot::channel();
            state
                .pending_permissions
                .lock()
                .await
                .insert(*request_id, rx);

            spawn_permission_forwarder(*request_id, state);

            vec![AgentEvent::SubagentPermissionRequest {
                parent_call_id: parent_call_id.clone(),
                call: HookToolCall {
                    tool_name: tool_name.clone(),
                    call_id: String::new(),
                    input: input.clone(),
                },
                tx,
            }]
        }

        ServerMessage::TodosUpdated { todos } => {
            vec![AgentEvent::TodosUpdated {
                todos: todos
                    .iter()
                    .map(|t| Todo {
                        id: t.id,
                        content: t.content.clone(),
                        status: TodoStatus::parse(&t.status).unwrap_or(TodoStatus::Pending),
                        depends_on: t.depends_on.clone(),
                    })
                    .collect(),
            }]
        }

        ServerMessage::PersonaSwitchRequested { name } => {
            vec![AgentEvent::PersonaSwitchRequested { name: name.clone() }]
        }

        ServerMessage::JobUpdate {
            job_id,
            command,
            state,
        } => {
            vec![AgentEvent::JobUpdate {
                job_id: job_id.clone(),
                command: command.clone(),
                state: state.clone(),
            }]
        }

        ServerMessage::SlashResult { text } => {
            // Slash results come back as synthetic text messages.
            // The TUI will handle this as a provider text event.
            // For now, emit as an Error so it's visible.
            // TODO: emit as a synthetic text AgentEvent.
            vec![AgentEvent::Error(text.clone())]
        }

        ServerMessage::ModelList { .. }
        | ServerMessage::ModelSwitched { .. }
        | ServerMessage::SessionList { .. }
        | ServerMessage::SessionHistory { .. }
        | ServerMessage::RequestResolved { .. }
        | ServerMessage::SessionCleared
        | ServerMessage::ThinkingVariantChanged { .. }
        | ServerMessage::PermissionModeChanged { .. }
        | ServerMessage::ClientAttached { .. }
        | ServerMessage::ClientDetached { .. }
        | ServerMessage::ControlYielded { .. }
        | ServerMessage::SessionTitleChanged { .. }
        | ServerMessage::SessionSummaryChanged { .. }
        | ServerMessage::SessionActivityChanged { .. }
        | ServerMessage::SessionStatsChanged { .. }
        | ServerMessage::GroupList { .. }
        | ServerMessage::GroupsChanged { .. }
        | ServerMessage::DirListing { .. }
        | ServerMessage::FilePreview { .. }
        | ServerMessage::GitStatusResult { .. }
        | ServerMessage::FsChanged { .. }
        | ServerMessage::SessionUsageChanged { .. }
        | ServerMessage::SessionAlert { .. }
        | ServerMessage::FlaggedFilesChanged { .. }
        | ServerMessage::SessionMetaChanged { .. }
        | ServerMessage::SessionAttentionChanged { .. }
        | ServerMessage::Pong { .. } => {
            // These are handled by the DaemonClient directly or are web-UI
            // specific; they don't map to AgentEvents for the TUI.
            Vec::new()
        }
    }
}

/// Spawn a task that waits for the TUI's permission decision and forwards
/// it back to the daemon.
fn spawn_permission_forwarder(request_id: u64, state: &Arc<ClientState>) {
    let state = state.clone();
    tokio::spawn(async move {
        let rx = {
            let mut guard = state.pending_permissions.lock().await;
            guard.remove(&request_id)
        };
        if let Some(rx) = rx {
            if let Ok(decision) = rx.await {
                let wire = WirePermissionDecision::from(decision);
                let msg = ClientMessage::PermissionResponse {
                    request_id,
                    decision: wire,
                };
                if let Ok(json) = mew_protocol::encode_json(&msg) {
                    let _ = state.ws_out.send(json).await;
                }
            }
        }
    });
}

/// Convert a `ProviderEventWire` back to a `ProviderEvent`.
/// The `field: &'static str` is leaked via `Box::leak` — acceptable for
/// short-lived streaming events. A proper fix would change
/// `ProviderEvent` to use `String` or `Cow<'static, str>`.
fn wire_to_provider_event(
    wire: &mew_message::ProviderEventWire,
) -> Option<mew_provider::ProviderEvent> {
    use mew_message::ProviderEventWire;
    use mew_provider::ProviderEvent;

    Some(match wire {
        ProviderEventWire::PartStart { part } => ProviderEvent::PartStart { part: part.clone() },
        ProviderEventWire::PartDelta {
            part_id,
            field,
            delta,
        } => ProviderEvent::PartDelta {
            part_id: *part_id,
            field: Box::leak(field.clone().into_boxed_str()),
            delta: delta.clone(),
        },
        ProviderEventWire::PartEnd { part_id } => ProviderEvent::PartEnd { part_id: *part_id },
        ProviderEventWire::MessageEnd {
            finish,
            usage,
            cost,
        } => ProviderEvent::MessageEnd {
            finish: *finish,
            usage: *usage,
            cost: *cost,
        },
        ProviderEventWire::RetryWait {
            attempt,
            max_attempts,
            delay_secs,
            reason,
        } => ProviderEvent::RetryWait {
            attempt: *attempt,
            max_attempts: *max_attempts,
            delay_secs: *delay_secs,
            reason: reason.clone(),
        },
        ProviderEventWire::Error(e) => ProviderEvent::Error(e.clone()),
    })
}
