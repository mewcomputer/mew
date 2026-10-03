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
//! The `Sender` goes into the `AgentEvent` (handed to the TUI); the
//! `Receiver` is awaited by a background task that forwards the decision
//! back to the daemon as a `ClientMessage::PermissionResponse`.
//! Required actions use the persistent notification channel, including when
//! the TUI attaches to a turn initiated by another client.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use mew_agent::{
    AgentEvent, AskUserQuestion, GoalDecision, QuestionOption as AgentQuestionOption, Todo,
    TodoStatus,
};
use mew_hooks::{PermissionDecision, ToolCall as HookToolCall};
use mew_message::Part;
use mew_protocol::{ClientMessage, PermissionDecision as WirePermissionDecision, ServerMessage};
use mew_provider::ProviderEvent;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async_with_config;
use tracing::warn;
use tungstenite::{protocol::WebSocketConfig, Message};

/// Shared state for pending request/response pairs.
struct ClientState {
    /// Buffered required actions, with consumed entries retained until the
    /// daemon resolves them so replay never creates a second response sender.
    required_actions: std::sync::Mutex<HashMap<String, Option<AgentEvent>>>,
    /// `request_id → receiver` for permission decisions.
    /// The receiver is awaited by a spawned task that forwards the
    /// decision back to the daemon.
    pending_permissions: Mutex<HashMap<String, oneshot::Receiver<PermissionDecision>>>,
    /// `request_id → receiver` for ask-user answers.
    pending_ask_user: Mutex<HashMap<String, oneshot::Receiver<Vec<String>>>>,
    /// `request_id → receiver` for goal-proposal decisions.
    pending_goal_proposals: Mutex<HashMap<String, oneshot::Receiver<GoalDecision>>>,
    /// The current event sender, set by `prompt()` and closed when the
    /// attached session reports Idle or the connection closes. The reader
    /// forwards all turn events before closing it.
    event_tx: Mutex<Option<mpsc::Sender<AgentEvent>>>,
    /// Persistent notification channel — always alive, unlike `event_tx`
    /// which is prompt-scoped. Session-management `ServerMessage`s
    /// (SessionList, SessionTitleChanged, etc.) and required actions arrive as raw
    /// wire types so the TUI can maintain a live session rail, alerts,
    /// and metadata without being in the middle of a turn.
    notify_tx: mpsc::Sender<ServerMessage>,
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
    ///
    /// Returns the `DaemonClient` and a `Receiver<ServerMessage>` for
    /// session-management notifications. The notification channel is
    /// always alive (unlike the prompt-scoped event channel) and receives
    /// raw `ServerMessage`s for session state and required actions. After
    /// applying a notification, callers consume its reconstructed action with
    /// `take_required_action`. Drain this channel in the main event loop.
    pub async fn connect(url: &str) -> Result<(Self, mpsc::Receiver<ServerMessage>)> {
        let config = WebSocketConfig {
            max_frame_size: None,
            ..WebSocketConfig::default()
        };
        let (mut ws_stream, _response) = connect_async_with_config(url, Some(config), false)
            .await
            .context("connect to daemon")?;

        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            ws_stream
                .send(Message::Text(mew_protocol::encode_json(&ClientMessage::Ping)?))
                .await?;
            loop {
                let frame = ws_stream.next().await.context("daemon closed during handshake")??;
                match frame {
                    Message::Text(text) => {
                        let message: ServerMessage = mew_protocol::decode_json(&text)
                            .context("invalid daemon protocol handshake")?;
                        match message {
                            ServerMessage::Pong { protocol_version, .. }
                                if protocol_version == mew_protocol::PROTOCOL_VERSION => return Ok(()),
                            _ => anyhow::bail!("incompatible daemon protocol; restart or update the daemon and client"),
                        }
                    }
                    Message::Close(_) => anyhow::bail!("daemon closed during protocol handshake"),
                    _ => {}
                }
            }
        })
        .await
        .context("daemon protocol handshake timed out; restart or update the daemon")??;

        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<String>(64);

        // Forward outgoing JSON to the WebSocket.
        let outgoing_task = tokio::spawn(async move {
            while let Some(json) = outgoing_rx.recv().await {
                if ws_tx.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        });

        let (notify_tx, notify_rx) = mpsc::channel::<ServerMessage>(128);

        let state = Arc::new(ClientState {
            required_actions: std::sync::Mutex::new(HashMap::new()),
            pending_permissions: Mutex::new(HashMap::new()),
            pending_ask_user: Mutex::new(HashMap::new()),
            pending_goal_proposals: Mutex::new(HashMap::new()),
            event_tx: Mutex::new(None),
            notify_tx,
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
                // capture the session ID for later use, and still push the
                // message to notify_tx so callers can read model/provider.
                if let ServerMessage::SessionReady { ref session_id, .. } = server_msg {
                    let mut current_session = state_clone.session_id.lock().await;
                    if current_session.as_ref() != Some(session_id) {
                        *state_clone.event_tx.lock().await = None;
                    }
                    *current_session = Some(session_id.clone());
                    drop(current_session);
                    state_clone.required_actions.lock().unwrap().clear();
                    let _ = state_clone.notify_tx.send(server_msg.clone()).await;
                    continue;
                }

                if let Some(request_id) = required_action_id(&server_msg) {
                    if !state_clone
                        .required_actions
                        .lock()
                        .unwrap()
                        .contains_key(request_id)
                    {
                        let action = translate_server_message(&server_msg, &state_clone)
                            .await
                            .into_iter()
                            .next();
                        state_clone
                            .required_actions
                            .lock()
                            .unwrap()
                            .insert(request_id.to_owned(), action);
                        let _ = state_clone.notify_tx.send(server_msg).await;
                    }
                    continue;
                }
                if let ServerMessage::RequestResolved { ref request_id } = server_msg {
                    state_clone
                        .required_actions
                        .lock()
                        .unwrap()
                        .remove(request_id);
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
                } else if !events.is_empty() {
                    warn!("daemon client: event without active prompt");
                }
            }
            outgoing_task.abort();
            let _ = outgoing_task.await;
            if let Some(tx) = state_clone.event_tx.lock().await.take() {
                let _ = tx
                    .send(AgentEvent::Error("daemon connection closed".into()))
                    .await;
            }
        });

        Ok((Self { state }, notify_rx))
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
    /// `Agent::run()`. Attachments (file paths) are forwarded to the daemon.
    pub async fn prompt(
        &self,
        text: String,
        attachments: Vec<mew_protocol::Attachment>,
    ) -> mpsc::Receiver<AgentEvent> {
        let (event_tx, event_rx) = mpsc::channel(256);
        if self.state.session_id.lock().await.is_none() {
            let _ = event_tx
                .send(AgentEvent::Error("no daemon session attached".into()))
                .await;
            return event_rx;
        }

        // Set the event sender so the background reader forwards events.
        *self.state.event_tx.lock().await = Some(event_tx);

        // Send the prompt message.
        let msg = ClientMessage::Prompt { text, attachments };
        let json = mew_protocol::encode_json(&msg).unwrap_or_default();
        if self.state.ws_out.send(json).await.is_err() {
            if let Some(tx) = self.state.event_tx.lock().await.take() {
                let _ = tx
                    .send(AgentEvent::Error("daemon connection closed".into()))
                    .await;
            }
        }

        event_rx
    }

    /// Cancel the current turn.
    pub async fn cancel(&self) {
        let msg = ClientMessage::Cancel;
        let json = mew_protocol::encode_json(&msg).unwrap_or_default();
        let _ = self.state.ws_out.send(json).await;
    }

    /// Inject a short user message as guidance into the running turn's next
    /// provider request (even a tool-call continuation). If no turn is running,
    /// the guidance is picked up by the next turn.
    pub async fn guide(&self, text: String) {
        let msg = ClientMessage::Guide { text };
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
    pub async fn slash_command(&self, command: String) -> Result<()> {
        let msg = ClientMessage::SlashCommand { command };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Request the daemon's session list. The response arrives as
    /// `ServerMessage::SessionList` on the notification channel.
    pub async fn list_sessions(&self) -> Result<()> {
        let msg = ClientMessage::ListSessions;
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Request the list of available models from the daemon. The response
    /// arrives as `ServerMessage::ModelList`, which populates the TUI's
    /// model picker and thinking-variants map.
    pub async fn list_models(&self) -> Result<()> {
        let msg = ClientMessage::ListModels;
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Request the daemon's known project directories. The response arrives
    /// as `ServerMessage::ProjectList` on the notification channel.
    pub async fn list_projects(&self) -> Result<()> {
        let msg = ClientMessage::ListProjects;
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Create a new session in the given cwd.
    pub async fn new_session_in(&self, cwd: &str) -> Result<()> {
        let msg = ClientMessage::NewSession {
            cwd: Some(cwd.to_string()),
            client_kind: mew_protocol::ClientKind::Tui,
        };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Archive or unarchive a session.
    pub async fn archive_session(&self, session_id: &str, archived: bool) -> Result<()> {
        let msg = ClientMessage::ArchiveSession {
            session_id: session_id.to_string(),
            archived,
        };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Pin or unpin a session.
    pub async fn pin_session(&self, session_id: &str, pinned: bool) -> Result<()> {
        let msg = ClientMessage::PinSession {
            session_id: session_id.to_string(),
            pinned,
        };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Toggle auto-title for the current session.
    pub async fn set_auto_title(&self, enabled: bool) -> Result<()> {
        let msg = ClientMessage::SetAutoTitle { enabled };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Toggle auto-summary for the current session.
    pub async fn set_auto_summary(&self, enabled: bool) -> Result<()> {
        let msg = ClientMessage::SetAutoSummary { enabled };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Rename a session.
    pub async fn rename_session(&self, session_id: &str, title: &str) -> Result<()> {
        let msg = ClientMessage::RenameSession {
            session_id: session_id.to_string(),
            title: title.to_string(),
        };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Unflag a file in a session.
    pub async fn unflag_file(&self, session_id: &str, path: &str) -> Result<()> {
        let msg = ClientMessage::UnflagFile {
            session_id: session_id.to_string(),
            path: path.to_string(),
        };
        let json = mew_protocol::encode_json(&msg)?;
        self.state
            .ws_out
            .send(json)
            .await
            .map_err(|e| anyhow::anyhow!("daemon send error: {}", e))
    }

    /// Returns the current session ID, if known (set after `SessionReady`).
    pub async fn session_id(&self) -> Option<String> {
        self.state.session_id.lock().await.clone()
    }

    /// Take a required action from its ordered notification. Requests remain
    /// available when observing a session without initiating its active turn.
    pub fn take_required_action(&self, message: &ServerMessage) -> Option<(String, AgentEvent)> {
        let request_id = required_action_id(message)?;
        let event = self
            .state
            .required_actions
            .lock()
            .unwrap()
            .get_mut(request_id)?
            .take()?;
        Some((request_id.to_owned(), event))
    }

    /// Clear the event sender (called when the turn ends).
    pub async fn clear_event_channel(&self) {
        *self.state.event_tx.lock().await = None;
    }
}

fn required_action_id(message: &ServerMessage) -> Option<&str> {
    match message {
        ServerMessage::PermissionRequest { request_id, .. }
        | ServerMessage::WorkspacePermissionRequest { request_id, .. }
        | ServerMessage::SubagentPermissionRequest { request_id, .. }
        | ServerMessage::AskUserRequest { request_id, .. }
        | ServerMessage::GoalProposed { request_id, .. } => Some(request_id),
        _ => None,
    }
}

/// Translate a `ServerMessage` into zero or more `AgentEvent`s.
/// Channel-bearing wire messages get fresh `oneshot::channel`s; the
/// `Sender` goes into the `AgentEvent`, and a spawned task forwards
/// the response back to the daemon.
async fn translate_server_message(
    msg: &ServerMessage,
    state: &Arc<ClientState>,
) -> Vec<AgentEvent> {
    match msg {
        ServerMessage::RemoteReady { .. } => Vec::new(),
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
                .insert(request_id.clone(), rx);

            spawn_permission_forwarder(request_id.clone(), state);

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
                .insert(request_id.clone(), rx);

            spawn_permission_forwarder(request_id.clone(), state);

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
            state
                .pending_ask_user
                .lock()
                .await
                .insert(request_id.clone(), rx);

            let request_id = request_id.clone();
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

        ServerMessage::GoalProposed {
            request_id,
            call_id,
            objective,
        } => {
            let (tx, rx) = oneshot::channel();
            state
                .pending_goal_proposals
                .lock()
                .await
                .insert(request_id.clone(), rx);

            let request_id = request_id.clone();
            let state = state.clone();
            tokio::spawn(async move {
                let rx = {
                    let mut guard = state.pending_goal_proposals.lock().await;
                    guard.remove(&request_id)
                };
                if let Some(rx) = rx {
                    if let Ok(decision) = rx.await {
                        let accepted = matches!(decision, GoalDecision::Accepted);
                        let msg = ClientMessage::GoalResponse {
                            request_id,
                            accepted,
                        };
                        if let Ok(json) = mew_protocol::encode_json(&msg) {
                            let _ = state.ws_out.send(json).await;
                        }
                    }
                }
            });

            vec![AgentEvent::GoalProposed {
                call_id: call_id.clone(),
                objective: objective.clone(),
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
            manifests,
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
                manifests: manifests.clone(),
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
                .insert(request_id.clone(), rx);

            spawn_permission_forwarder(request_id.clone(), state);

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
            // Slash results come back as synthetic text messages — NOT errors.
            // Emit as a complete text part so it renders in the chat pane.
            let part_id = mew_message::PartId::new();
            let msg_id = mew_message::MessageId::new();
            let session_id = mew_message::SessionId::new();
            vec![
                AgentEvent::Provider(ProviderEvent::PartStart {
                    part: Part::Text(mew_message::TextPart {
                        base: mew_message::PartBase {
                            id: part_id,
                            message_id: msg_id,
                            session_id,
                        },
                        text: text.clone(),
                        synthetic: true,
                    }),
                }),
                AgentEvent::Provider(ProviderEvent::PartEnd { part_id }),
                AgentEvent::Provider(ProviderEvent::MessageEnd {
                    finish: mew_message::Finish::Stop,
                    usage: mew_message::Tokens::default(),
                    cost: 0.0,
                }),
            ]
        }

        ServerMessage::SessionActivityChanged {
            session_id,
            activity,
        } => {
            // Idle is emitted only after the agent's full event stream drains,
            // including tools and continuations. Closing this prompt-scoped
            // channel lets the TUI finish after its buffered events, in order.
            if *activity == mew_protocol::SessionState::Idle
                && state.session_id.lock().await.as_ref() == Some(session_id)
            {
                *state.event_tx.lock().await = None;
            }
            let _ = state.notify_tx.send(msg.clone()).await;
            Vec::new()
        }

        ServerMessage::BrowserPageRegistration { .. }
        | ServerMessage::ModelList { .. }
        | ServerMessage::ModelSwitched { .. }
        | ServerMessage::RequestResolved { .. }
        | ServerMessage::SessionCleared
        | ServerMessage::ThinkingVariantChanged { .. }
        | ServerMessage::PermissionModeChanged { .. }
        | ServerMessage::ClientAttached { .. }
        | ServerMessage::ClientDetached { .. }
        | ServerMessage::ControlYielded { .. }
        | ServerMessage::SessionStatsChanged { .. }
        | ServerMessage::GroupList { .. }
        | ServerMessage::GroupsChanged { .. }
        | ServerMessage::DirListing { .. }
        | ServerMessage::FilePreview { .. }
        | ServerMessage::GitStatusResult { .. }
        | ServerMessage::SessionDiff { .. }
        | ServerMessage::SessionDiffError { .. }
        | ServerMessage::FsChanged { .. }
        | ServerMessage::SessionUsageChanged { .. }
        | ServerMessage::SessionMetaChanged { .. }
        | ServerMessage::SessionAttentionChanged { .. }
        | ServerMessage::TerminalOpened { .. }
        | ServerMessage::TerminalOutput { .. }
        | ServerMessage::TerminalExited { .. }
        | ServerMessage::TerminalError { .. } => {
            // Forward to the notification channel for the TUI's daemon
            // session rail / alerts / metadata. These don't map to
            // AgentEvents.
            let _ = state.notify_tx.send(msg.clone()).await;
            Vec::new()
        }

        ServerMessage::BrowserSnapshot { .. }
        | ServerMessage::BrowserScreenshot { .. }
        | ServerMessage::BrowserState { .. }
        | ServerMessage::BrowserError { .. } => {
            let _ = state.notify_tx.send(msg.clone()).await;
            Vec::new()
        }

        // These carry data the TUI needs (session list, history, titles,
        // alerts, flagged files). Forward to notify AND produce no
        // AgentEvent — the TUI handles them via the reducer.
        ServerMessage::SessionList { .. }
        | ServerMessage::SessionHistory { .. }
        | ServerMessage::SessionHistoryPage { .. }
        | ServerMessage::SessionTitleChanged { .. }
        | ServerMessage::SessionSummaryChanged { .. }
        | ServerMessage::SessionAlert { .. }
        | ServerMessage::FlaggedFilesChanged { .. }
        | ServerMessage::PersonaList { .. }
        | ServerMessage::PersonaSwitched { .. }
        | ServerMessage::Pong { .. }
        | ServerMessage::ProjectList { .. }
        | ServerMessage::FilesystemDirListing { .. } => {
            let _ = state.notify_tx.send(msg.clone()).await;
            Vec::new()
        }
    }
}

/// Spawn a task that waits for the TUI's permission decision and forwards
/// it back to the daemon.
fn spawn_permission_forwarder(request_id: String, state: &Arc<ClientState>) {
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
            manifest: _,
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn accept_current_handshake(
        ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) {
        let frame = ws.next().await.unwrap().unwrap();
        assert!(
            matches!(frame, Message::Text(text) if matches!(mew_protocol::decode_json::<ClientMessage>(&text).unwrap(), ClientMessage::Ping))
        );
        let pong = ServerMessage::Pong {
            version: "test".into(),
            protocol_version: mew_protocol::PROTOCOL_VERSION,
        };
        ws.send(Message::Text(mew_protocol::encode_json(&pong).unwrap()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn required_actions_are_available_without_prompt_and_not_duplicated_in_prompt_stream() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            for active_prompt in [false, true] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let server = tokio::spawn(async move {
                    let (socket, _) = listener.accept().await.unwrap();
                    let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                    accept_current_handshake(&mut ws).await;
                    ws.send(Message::Text(mew_protocol::encode_json(&ServerMessage::SessionReady {
                        session_id: "active".into(), cwd: None, model: None, provider: None, permission_mode: None,
                    }).unwrap())).await.unwrap();
                    if active_prompt {
                        assert!(matches!(ws.next().await.unwrap().unwrap(), Message::Text(text) if text.contains("prompt")));
                    }
                    let requests = [
                        ServerMessage::PermissionRequest { request_id: "p".into(), tool_name: "bash".into(), input: serde_json::json!({"command":"pwd"}) },
                        ServerMessage::WorkspacePermissionRequest { request_id: "w".into(), path: "/elsewhere".into() },
                        ServerMessage::SubagentPermissionRequest { request_id: "s".into(), parent_call_id: "parent".into(), tool_name: "write".into(), input: serde_json::json!({}) },
                        ServerMessage::AskUserRequest { request_id: "q".into(), call_id: "call-q".into(), questions: vec![mew_protocol::Question { prompt: "pick".into(), options: vec![] }] },
                        ServerMessage::GoalProposed { request_id: "g".into(), call_id: "call-g".into(), objective: "finish".into() },
                    ];
                    for request in requests {
                        ws.send(Message::Text(mew_protocol::encode_json(&request).unwrap())).await.unwrap();
                        let Message::Text(response) = ws.next().await.unwrap().unwrap() else { panic!("expected response") };
                        let response: ClientMessage = mew_protocol::decode_json(&response).unwrap();
                        let request_id = match response {
                            ClientMessage::PermissionResponse { request_id, decision } => {
                                assert_eq!(decision, WirePermissionDecision::AllowOnce); request_id
                            }
                            ClientMessage::AskUserResponse { request_id, answers } => {
                                assert_eq!(answers, vec!["answer"]); request_id
                            }
                            ClientMessage::GoalResponse { request_id, accepted } => {
                                assert!(accepted); request_id
                            }
                            other => panic!("unexpected response: {other:?}"),
                        };
                        assert_eq!(Some(request_id.as_str()), required_action_id(&request));
                        ws.send(Message::Text(mew_protocol::encode_json(&ServerMessage::RequestResolved { request_id }).unwrap())).await.unwrap();
                    }
                    ws.send(Message::Text(mew_protocol::encode_json(&ServerMessage::SessionActivityChanged {
                        session_id: "active".into(), activity: mew_protocol::SessionState::Idle,
                    }).unwrap())).await.unwrap();
                    while ws.next().await.is_some() {}
                });
                let (client, mut notifications) = DaemonClient::connect(&format!("ws://{addr}")).await.unwrap();
                assert!(matches!(notifications.recv().await, Some(ServerMessage::SessionReady { .. })));
                let mut prompt = if active_prompt { Some(client.prompt("go".into(), vec![]).await) } else { None };
                for expected_id in ["p", "w", "s", "q", "g"] {
                    let request = notifications.recv().await.unwrap();
                    let (request_id, action) = client.take_required_action(&request).unwrap();
                    assert_eq!(request_id, expected_id);
                    assert!(client.take_required_action(&request).is_none());
                    match action {
                        AgentEvent::PermissionRequest { tx, .. } | AgentEvent::WorkspacePermissionRequest { tx, .. } | AgentEvent::SubagentPermissionRequest { tx, .. } => { tx.send(PermissionDecision::AllowOnce).unwrap(); }
                        AgentEvent::AskUser { tx, .. } => { tx.send(vec!["answer".into()]).unwrap(); }
                        AgentEvent::GoalProposed { tx, .. } => { tx.send(GoalDecision::Accepted).unwrap(); }
                        other => panic!("unexpected action: {other:?}"),
                    }
                    assert!(matches!(notifications.recv().await, Some(ServerMessage::RequestResolved { request_id }) if request_id == expected_id));
                }
                assert!(matches!(notifications.recv().await, Some(ServerMessage::SessionActivityChanged { .. })));
                if let Some(prompt) = prompt.as_mut() { assert!(prompt.recv().await.is_none()); }
                server.abort();
            }
        }).await.expect("required-action client sequence timed out");
    }

    #[tokio::test]
    async fn resolving_another_clients_request_does_not_send_a_decision_or_cancel() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                accept_current_handshake(&mut ws).await;
                for message in [
                    ServerMessage::SessionReady {
                        session_id: "active".into(),
                        cwd: None,
                        model: None,
                        provider: None,
                        permission_mode: None,
                    },
                    ServerMessage::AskUserRequest {
                        request_id: "q".into(),
                        call_id: "c".into(),
                        questions: vec![],
                    },
                ] {
                    ws.send(Message::Text(mew_protocol::encode_json(&message).unwrap()))
                        .await
                        .unwrap();
                }
                let Message::Text(ping) = ws.next().await.unwrap().unwrap() else {
                    panic!("expected ping")
                };
                assert!(matches!(
                    mew_protocol::decode_json::<ClientMessage>(&ping).unwrap(),
                    ClientMessage::Ping
                ));
                ws.send(Message::Text(
                    mew_protocol::encode_json(&ServerMessage::RequestResolved {
                        request_id: "q".into(),
                    })
                    .unwrap(),
                ))
                .await
                .unwrap();
                let Message::Text(ping) = ws.next().await.unwrap().unwrap() else {
                    panic!("expected ping")
                };
                assert!(matches!(
                    mew_protocol::decode_json::<ClientMessage>(&ping).unwrap(),
                    ClientMessage::Ping
                ));
            });
            let (client, mut notifications) = DaemonClient::connect(&format!("ws://{address}"))
                .await
                .unwrap();
            assert!(matches!(
                notifications.recv().await,
                Some(ServerMessage::SessionReady { .. })
            ));
            let request = notifications.recv().await.unwrap();
            let (_, event) = client.take_required_action(&request).unwrap();
            let ping = mew_protocol::encode_json(&ClientMessage::Ping).unwrap();
            client.send_raw(&ping).await.unwrap();
            assert!(matches!(
                notifications.recv().await,
                Some(ServerMessage::RequestResolved { .. })
            ));
            drop(event);
            client.send_raw(&ping).await.unwrap();
            server.await.unwrap();
        })
        .await
        .expect("external request resolution timed out");
    }

    #[tokio::test]
    async fn connection_rejects_legacy_daemon_before_attaching() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let message = ws.next().await.unwrap().unwrap();
            assert!(matches!(message, Message::Text(text) if text.contains("ping")));
            ws.send(Message::Text(r#"{"type":"pong","version":"0.2.0"}"#.into()))
                .await
                .unwrap();
            while ws.next().await.is_some() {}
        });
        let result = DaemonClient::connect(&format!("ws://{address}")).await;
        let error = result.err().expect("legacy daemon must be rejected");
        assert!(error.to_string().contains("protocol"));
        assert!(error.to_string().contains("restart"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn prompt_stream_closes_only_when_attached_turn_is_idle() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (finish_tx, finish_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                accept_current_handshake(&mut ws).await;
                let ready = ServerMessage::SessionReady {
                    session_id: "active".into(),
                    cwd: None,
                    provider: Some("codex".into()),
                    model: Some("test".into()),
                    permission_mode: None,
                };
                ws.send(Message::Text(mew_protocol::encode_json(&ready).unwrap()))
                    .await
                    .unwrap();
                ws.next().await.unwrap().unwrap(); // Prompt
                for msg in [
                    ServerMessage::Provider {
                        event: mew_message::ProviderEventWire::MessageEnd {
                            finish: mew_message::Finish::Stop,
                            usage: mew_message::Tokens::default(),
                            cost: 0.0,
                            manifest: None,
                        },
                    },
                    ServerMessage::ToolStart {
                        call_id: "tool".into(),
                    },
                    ServerMessage::SessionActivityChanged {
                        session_id: "other".into(),
                        activity: mew_protocol::SessionState::Idle,
                    },
                ] {
                    ws.send(Message::Text(mew_protocol::encode_json(&msg).unwrap()))
                        .await
                        .unwrap();
                }
                finish_rx.await.unwrap();
                let idle = ServerMessage::SessionActivityChanged {
                    session_id: "active".into(),
                    activity: mew_protocol::SessionState::Idle,
                };
                ws.send(Message::Text(mew_protocol::encode_json(&idle).unwrap()))
                    .await
                    .unwrap();
                // Wait for the test to close the connection so websocket closure
                // cannot accidentally stand in for the Idle notification.
                while ws.next().await.is_some() {}
            });
            let (client, mut notifications) = DaemonClient::connect(&format!("ws://{addr}"))
                .await
                .unwrap();
            assert!(matches!(
                notifications.recv().await,
                Some(ServerMessage::SessionReady { .. })
            ));
            let mut events = client.prompt("hello".into(), Vec::new()).await;
            assert!(matches!(
                events.recv().await,
                Some(AgentEvent::Provider(ProviderEvent::MessageEnd { .. }))
            ));
            assert!(matches!(
                events.recv().await,
                Some(AgentEvent::ToolStart { .. })
            ));
            assert!(matches!(
                notifications.recv().await,
                Some(ServerMessage::SessionActivityChanged { .. })
            ));
            assert!(matches!(
                events.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
            finish_tx.send(()).unwrap();
            assert!(events.recv().await.is_none());
            server.abort();
        })
        .await
        .expect("prompt stream did not finish");
    }

    #[tokio::test]
    async fn disconnected_prompt_stream_reports_error_and_closes() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                accept_current_handshake(&mut ws).await;
                let ready = ServerMessage::SessionReady {
                    session_id: "active".into(),
                    cwd: None,
                    provider: None,
                    model: None,
                    permission_mode: None,
                };
                ws.send(Message::Text(mew_protocol::encode_json(&ready).unwrap()))
                    .await
                    .unwrap();
                ws.next().await.unwrap().unwrap();
                ws.close(None).await.unwrap();
            });
            let (client, mut notifications) = DaemonClient::connect(&format!("ws://{addr}"))
                .await
                .unwrap();
            notifications.recv().await.unwrap();
            let mut events = client.prompt("hello".into(), Vec::new()).await;
            assert!(matches!(events.recv().await, Some(AgentEvent::Error(_))));
            assert!(events.recv().await.is_none());
            // A queued follow-up must also terminate if the connection is gone.
            let mut events = client.prompt("next".into(), Vec::new()).await;
            assert!(matches!(events.recv().await, Some(AgentEvent::Error(_))));
            assert!(events.recv().await.is_none());
            server.await.unwrap();
        })
        .await
        .expect("disconnected prompt stream did not finish");
    }
}
