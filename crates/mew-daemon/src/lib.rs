//! mew daemon — a standalone agent server.
//!
//! The daemon owns the agent loop and streams events to connected frontends
//! over WebSocket. Frontends (TUI, Discord bot, iOS app) connect and send
//! commands; the daemon runs the agent and streams `ServerMessage`s back.
//!
//! ## Wire format
//!
//! JSON over WebSocket text frames. The schema is defined in `mew-protocol`.
//!
//! ## Session model (minimal slice)
//!
//! One connection = one session. The frontend sends `NewSession`, then
//! `Prompt` messages. The daemon streams `ServerMessage` events back.
//! Multi-session support comes later.
//!
//! ## AgentEvent → ServerMessage translation
//!
//! `AgentEvent` has four variants that carry `oneshot::Sender` channels
//! (PermissionRequest, WorkspacePermissionRequest, AskUser,
//! SubagentPermissionRequest). These can't be serialized. The daemon
//! translates them into ID-paired wire requests: it stashes the `oneshot`
//! in a pending-requests map keyed by a fresh ID, sends a `ServerMessage`
//! with that ID, and when the frontend responds with the same ID the
//! daemon retrieves the `oneshot` and sends the decision back.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use mew_agent::{Agent, AgentEvent};
use mew_protocol::{ClientMessage, Question, QuestionOption, ServerMessage};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tungstenite::Message;

pub mod client;
pub mod session;

pub use client::DaemonClient;
pub use session::{AttachError, Session, SessionManager};

/// Parameters passed to the agent-builder closure.
pub struct AgentBuildParams {
    pub session_id: String,
    pub writer: mew_session::Writer,
    pub cwd: Option<std::path::PathBuf>,
}

/// Type alias for the agent-builder closure. The daemon calls this once
/// per session to create an `Agent` backed by the supplied `Writer`.
/// Returns the agent plus the current model/provider display IDs (so the
/// frontend can show them in SessionReady).
pub type AgentBuilder =
    Arc<dyn Fn(AgentBuildParams) -> Result<(Agent, Option<String>, Option<String>)> + Send + Sync>;

/// Switch the model on an existing agent. Returns the new (provider, model)
/// display IDs on success. The closure receives `(provider_id, model_id)`
/// and rebuilds the provider in-place on the agent.
pub type ModelSwitcher =
    Arc<dyn Fn(&mut Agent, &str, &str) -> Result<(String, String)> + Send + Sync>;

/// List available models. Returns `Vec<ModelInfo>` for the picker UI.
pub type ModelLister = Arc<dyn Fn() -> Vec<mew_protocol::ModelInfo> + Send + Sync>;

/// Set the thinking/reasoning variant on an agent. Takes the agent,
/// current model ID, and variant name (or empty/"none" to disable).
/// Returns the resolved variant name, or None if thinking was disabled.
pub type ThinkingSetter =
    Arc<dyn Fn(&mut Agent, &str, &str) -> Result<Option<String>> + Send + Sync>;

/// The daemon server. Binds a Unix socket and accepts WebSocket connections.
pub struct DaemonServer {
    /// Public so callers (e.g. `mew daemon` main) can clone it for dual
    /// Unix+TCP listeners sharing a single agent-builder closure.
    pub builder: AgentBuilder,
    /// Optional model switcher for `SwitchModel` client messages.
    pub model_switcher: Option<ModelSwitcher>,
    /// Optional model lister for `ListModels` client messages.
    pub model_lister: Option<ModelLister>,
    /// Optional thinking variant setter for `SetThinkingVariant` client messages.
    pub thinking_setter: Option<ThinkingSetter>,
    /// Owns active sessions and resume-from-disk.
    pub session_manager: Arc<SessionManager>,
}

impl DaemonServer {
    /// Create a new daemon. `builder` is called once per session to construct
    /// an `Agent` with all its tools, provider, etc.
    pub fn new(builder: AgentBuilder) -> Self {
        let session_dir = mew_session::session_dir();
        Self::with_session_dir_inner(builder, session_dir, None, None)
    }

    /// Use a custom session directory instead of the global default. Used by
    /// tests that need to isolate sessions to a tempdir.
    pub fn with_session_dir(builder: AgentBuilder, session_dir: PathBuf) -> Self {
        Self::with_session_dir_inner(builder, session_dir, None, None)
    }

    fn with_session_dir_inner(
        builder: AgentBuilder,
        session_dir: PathBuf,
        switcher: Option<ModelSwitcher>,
        lister: Option<ModelLister>,
    ) -> Self {
        let session_manager = Arc::new(SessionManager::new(
            builder.clone(),
            session_dir,
            switcher,
            lister,
        ));
        Self {
            builder,
            model_switcher: None,
            model_lister: None,
            thinking_setter: None,
            session_manager,
        }
    }

    /// Enable model switching by supplying a switcher + lister.
    pub fn with_model_management(mut self, switcher: ModelSwitcher, lister: ModelLister) -> Self {
        self.model_switcher = Some(switcher.clone());
        self.model_lister = Some(lister.clone());
        // Rebuild the session manager with the same session dir, now carrying
        // the switcher/lister.
        let session_dir = self.session_manager.session_dir.clone();
        self.session_manager = Arc::new(SessionManager::new(
            self.builder.clone(),
            session_dir,
            Some(switcher),
            Some(lister),
        ));
        self
    }

    /// Enable thinking variant switching.
    pub fn with_thinking_setter(mut self, setter: ThinkingSetter) -> Self {
        self.thinking_setter = Some(setter);
        self
    }

    /// Run the daemon, listening on the given Unix socket path.
    /// Blocks until the listener is closed, a signal (SIGINT/SIGTERM) is
    /// received, or an unrecoverable error occurs.
    pub async fn run(self, socket_path: &str) -> Result<()> {
        // Remove stale socket.
        let _ = std::fs::remove_file(socket_path);

        let listener = UnixListener::bind(socket_path)
            .with_context(|| format!("bind socket {}", socket_path))?;
        info!(socket = socket_path, "mew daemon listening");

        let session_manager = self.session_manager.clone();
        let thinking_setter = self.thinking_setter.clone();
        let mut id_counter = 0u64;

        let mut sig_term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("install SIGTERM handler")?;

        loop {
            tokio::select! {
                _ = sig_term.recv() => {
                    info!("received SIGTERM, shutting down");
                    break;
                }
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            id_counter += 1;
                            let conn_id = id_counter;
                            let session_manager = session_manager.clone();
                            let thinking_setter = thinking_setter.clone();
                            tokio::spawn(async move {
                                info!(conn_id, "connection accepted");
                                if let Err(e) = handle_connection(stream, session_manager, thinking_setter).await {
                                    if !e.to_string().contains("connection reset") {
                                        warn!(conn_id, error = %e, "connection ended with error");
                                    }
                                }
                                info!(conn_id, "connection closed");
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "accept failed");
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Run the daemon, listening on the given TCP address (e.g.
    /// `127.0.0.1:9847`). Browser-based frontends connect to this. Same
    /// per-connection semantics as `run()` over a Unix socket.
    pub async fn run_tcp(self, addr: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind tcp {addr}"))?;
        info!(%addr, "mew daemon listening (tcp)");

        let session_manager = self.session_manager.clone();
        let thinking_setter = self.thinking_setter.clone();
        let mut id_counter = 0u64;

        let mut sig_term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("install SIGTERM handler")?;

        loop {
            tokio::select! {
                _ = sig_term.recv() => {
                    info!("received SIGTERM, shutting down");
                    break;
                }
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer)) => {
                            id_counter += 1;
                            let conn_id = id_counter;
                            let session_manager = session_manager.clone();
                            let thinking_setter = thinking_setter.clone();
                            tokio::spawn(async move {
                                info!(conn_id, %peer, "connection accepted (tcp)");
                                if let Err(e) = handle_connection(stream, session_manager, thinking_setter).await {
                                    if !e.to_string().contains("connection reset") {
                                        warn!(conn_id, error = %e, "connection ended with error");
                                    }
                                }
                                info!(conn_id, "connection closed");
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "accept failed (tcp)");
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Per-connection handler
// ---------------------------------------------------------------------------

async fn handle_connection<S>(
    stream: S,
    session_manager: Arc<SessionManager>,
    thinking_setter: Option<ThinkingSetter>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ws_stream = accept_async(stream).await.context("websocket handshake")?;
    let (ws_tx, mut ws_rx) = ws_stream.split();

    let (client_tx, mut client_rx): (
        mpsc::UnboundedSender<ServerMessage>,
        mpsc::UnboundedReceiver<ServerMessage>,
    ) = mpsc::unbounded_channel();
    let mut attached_session: Option<Arc<Session>> = None;
    let mut client_id: Option<u64> = None;
    let mut auto_title_enabled = true;

    // Shared flag for the idle-summary background task.
    let auto_summary_enabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

    // Spawn the idle-summary background task. It periodically checks active
    // sessions and generates a summary for sessions that have been idle
    // for a while.
    {
        let auto_summary_enabled = auto_summary_enabled.clone();
        let session_manager = session_manager.clone();
        tokio::spawn(async move {
            idle_summary_task(session_manager, auto_summary_enabled).await;
        });
    }

    // Spawn a writer task that owns the WebSocket sink.
    let mut ws_tx = ws_tx;
    let writer = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if send_msg(&mut ws_tx, msg).await.is_err() {
                break;
            }
        }
        let _ = ws_tx.close().await;
    });

    let reply = |msg: ServerMessage| {
        let _ = client_tx.send(msg);
    };

    while let Some(msg) = ws_rx.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                if !e.to_string().contains("connection reset") {
                    warn!(error = %e, "websocket read error");
                }
                break;
            }
        };

        let text = match msg.to_text() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let client_msg = match mew_protocol::decode_json::<ClientMessage>(text) {
            Ok(cm) => cm,
            Err(e) => {
                reply(ServerMessage::Error {
                    message: format!("invalid message: {e}"),
                });
                continue;
            }
        };

        match client_msg {
            ClientMessage::NewSession { cwd } => {
                match session_manager.create(cwd.map(PathBuf::from)).await {
                    Ok(session) => {
                        let (cid, _) = session.attach_client(client_tx.clone()).await;
                        client_id = Some(cid);
                        attached_session = Some(session.clone());
                        let (model, provider) = {
                            (
                                session.model.lock().await.clone(),
                                session.provider.lock().await.clone(),
                            )
                        };
                        reply(ServerMessage::SessionReady {
                            session_id: session.id.clone(),
                            model,
                            provider,
                        });
                    }
                    Err(e) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to create session: {e}"),
                        });
                    }
                }
            }
            ClientMessage::AttachSession { session_id } => {
                match session_manager.attach(&session_id).await {
                    Ok(session) => {
                        let (cid, _was_first) = session.attach_client(client_tx.clone()).await;
                        client_id = Some(cid);
                        attached_session = Some(session.clone());

                        let (model, provider) = {
                            (
                                session.model.lock().await.clone(),
                                session.provider.lock().await.clone(),
                            )
                        };
                        reply(ServerMessage::SessionReady {
                            session_id: session_id.clone(),
                            model,
                            provider,
                        });

                        // Always send the current message history on attach.
                        // The client may be switching between sessions on the
                        // same connection, so it needs the full history.
                        let messages = {
                            let agent = session.agent.lock().await;
                            let msgs = agent.messages.lock().await.clone();
                            msgs
                        };
                        reply(ServerMessage::SessionHistory { messages });
                    }
                    Err(AttachError::NotFound) => {
                        reply(ServerMessage::Error {
                            message: "session not found".into(),
                        });
                    }
                    Err(AttachError::NotTopLevel) => {
                        reply(ServerMessage::Error {
                            message: "cannot attach to subagent session".into(),
                        });
                    }
                    Err(AttachError::BuildAgent(e)) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to resume session: {e}"),
                        });
                    }
                }
            }
            ClientMessage::ListSessions => {
                let sessions = session_manager.list().await;
                reply(ServerMessage::SessionList { sessions });
            }
            ClientMessage::DeleteSession { session_id } => {
                // Remove from active sessions if present.
                session_manager.remove(&session_id).await;
                // Delete from disk.
                let session_dir = mew_session::session_dir();
                let dir = session_dir.join(&session_id);
                if dir.exists() {
                    match tokio::fs::remove_dir_all(&dir).await {
                        Ok(_) => {
                            tracing::info!(session_id = %session_id, "deleted session");
                        }
                        Err(e) => {
                            tracing::warn!(session_id = %session_id, error = %e, "failed to delete session dir");
                        }
                    }
                }
                // If the deleted session was the current one, navigate home.
                if attached_session.as_ref().map(|s| s.id.as_str()) == Some(session_id.as_str()) {
                    attached_session = None;
                    client_id = None;
                }
            }
            ClientMessage::RenameSession { session_id, title } => {
                // Persist to disk for both active and idle sessions.
                let dir = mew_session::session_dir();
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    let _ = meta.set_custom_title(&dir, title.clone()).await;
                }
                // Broadcast to all clients.
                session_manager.broadcast_title(&session_id, title).await;
            }
            ClientMessage::SetAutoTitle { enabled } => {
                auto_title_enabled = enabled;
                tracing::info!(auto_title = enabled, "auto-title setting changed");
            }
            ClientMessage::SetAutoSummary { enabled } => {
                auto_summary_enabled.store(enabled, std::sync::atomic::Ordering::Relaxed);
                tracing::info!(auto_summary = enabled, "auto-summary setting changed");
            }
            ClientMessage::Prompt { text, .. } => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::Error {
                        message: "no session — send NewSession or AttachSession first".into(),
                    });
                    continue;
                };
                // Broadcast the user message to all attached clients.
                // The sending client deduplicates by matching text content.
                session
                    .broadcast(ServerMessage::UserMessage { text: text.clone() })
                    .await;
                let client_tx = client_tx.clone();
                tokio::spawn(async move {
                    let _guard = session.turn_lock.lock().await;
                    let has_turn: bool = session.current_turn_cancel.lock().await.is_some();
                    if has_turn {
                        let _ = client_tx.send(ServerMessage::Error {
                            message: "turn in progress".into(),
                        });
                        return;
                    }
                    let token = CancellationToken::new();
                    *session.current_turn_cancel.lock().await = Some(token.clone());
                    let agent = session.agent.lock().await.clone();
                    let prompt_text = text.clone();
                    let auto_title = auto_title_enabled;
                    let rx = agent.run_with_parts(text, vec![], Some(token));
                    forward_events(rx, session.clone()).await;
                    *session.current_turn_cancel.lock().await = None;

                    // Generate a session title from the first user message
                    // if we haven't already. Uses a lightweight LLM call;
                    // falls back to text truncation on error. Skipped if the
                    // user has disabled auto-title generation.
                    if auto_title {
                        let mut generated = session.title_generated.lock().await;
                        if !*generated {
                            *generated = true;
                            drop(generated);

                            let title = generate_session_title(&agent, &prompt_text).await;
                            session
                                .broadcast(ServerMessage::SessionTitleChanged {
                                    session_id: session.id.clone(),
                                    title,
                                })
                                .await;
                        }
                    }
                });
            }
            ClientMessage::Cancel => {
                if let Some(session) = &attached_session {
                    session.cancel_turn().await;
                }
            }
            ClientMessage::PermissionResponse {
                request_id,
                decision,
            } => {
                if let Some(session) = &attached_session {
                    let tx = session.pending_permissions.lock().await.remove(&request_id);
                    if let Some(tx) = tx {
                        let _ = tx.send(decision.into());
                    }
                    session
                        .broadcast(ServerMessage::RequestResolved { request_id })
                        .await;
                }
            }
            ClientMessage::AskUserResponse {
                request_id,
                answers,
            } => {
                if let Some(session) = &attached_session {
                    let tx = session.pending_ask_user.lock().await.remove(&request_id);
                    if let Some(tx) = tx {
                        let _ = tx.send(answers);
                    }
                    session
                        .broadcast(ServerMessage::RequestResolved { request_id })
                        .await;
                }
            }
            ClientMessage::SlashCommand { command } => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::Error {
                        message: "no session".into(),
                    });
                    continue;
                };
                let client_tx = client_tx.clone();
                tokio::spawn(async move {
                    let _guard = session.turn_lock.lock().await;
                    let result = {
                        let agent = session.agent.lock().await;
                        let (cmd, _arg) = match command.split_once(' ') {
                            Some((c, a)) => (c, Some(a)),
                            None => (command.as_str(), None),
                        };
                        match cmd {
                            "/clear" => {
                                agent.clear_context().await;
                                session.broadcast(ServerMessage::SessionCleared).await;
                                Some("context cleared".to_string())
                            }
                            "/compact" => {
                                agent.force_compact().await;
                                Some("compaction done".to_string())
                            }
                            _ => None,
                        }
                    };
                    if let Some(text) = result {
                        let _ = client_tx.send(ServerMessage::SlashResult { text });
                    }
                });
            }
            ClientMessage::ListModels => {
                let models = match &session_manager.lister {
                    Some(lister) => lister(),
                    None => Vec::new(),
                };
                reply(ServerMessage::ModelList { models });
            }
            ClientMessage::SwitchModel { provider, model } => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::Error {
                        message: "no session".into(),
                    });
                    continue;
                };
                let client_tx = client_tx.clone();
                let switcher = session_manager.switcher.clone();
                tokio::spawn(async move {
                    let _guard = session.turn_lock.lock().await;
                    let result = match &switcher {
                        Some(switcher) => {
                            let mut agent = session.agent.lock().await;
                            switcher(&mut agent, &provider, &model)
                        }
                        None => Err(anyhow::anyhow!("model switching not available")),
                    };
                    match result {
                        Ok((new_provider, new_model)) => {
                            info!(provider = %new_provider, model = %new_model, "model switched");
                            *session.provider.lock().await = Some(new_provider.clone());
                            *session.model.lock().await = Some(new_model.clone());
                            session
                                .broadcast(ServerMessage::ModelSwitched {
                                    provider: new_provider,
                                    model: new_model,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = client_tx.send(ServerMessage::Error {
                                message: format!("failed to switch model: {e}"),
                            });
                        }
                    }
                });
            }
            ClientMessage::SetThinkingVariant { variant } => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::Error {
                        message: "no session".into(),
                    });
                    continue;
                };
                let client_tx = client_tx.clone();
                let thinking_setter = thinking_setter.clone();
                tokio::spawn(async move {
                    let _guard = session.turn_lock.lock().await;
                    let result = match &thinking_setter {
                        Some(setter) => {
                            let model = session.model.lock().await.clone().unwrap_or_default();
                            let mut agent = session.agent.lock().await;
                            setter(&mut agent, &model, &variant)
                        }
                        None => Err(anyhow::anyhow!("thinking variant switching not available")),
                    };
                    match result {
                        Ok(resolved) => {
                            let _ = client_tx
                                .send(ServerMessage::ThinkingVariantChanged { variant: resolved });
                        }
                        Err(e) => {
                            let _ = client_tx.send(ServerMessage::Error {
                                message: format!("failed to set thinking variant: {e}"),
                            });
                        }
                    }
                });
            }
        }
    }

    // Cleanup
    if let (Some(session), Some(cid)) = (&attached_session, client_id) {
        let was_last = session.detach_client(cid).await;
        if was_last {
            let has_turn: bool = session.current_turn_cancel.lock().await.is_some();
            if has_turn {
                session.cancel_turn().await;
            }
            session_manager.remove(&session.id).await;
        }
    }
    drop(client_tx);
    writer.await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Event forwarding: AgentEvent → ServerMessage
// ---------------------------------------------------------------------------

type WsSink<S> = futures::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>;

/// Read AgentEvents from the receiver and broadcast them to every client
/// attached to the session. This consumes events until the receiver is drained
/// (i.e. the agent turn has ended).
async fn forward_events(mut rx: tokio::sync::mpsc::Receiver<AgentEvent>, session: Arc<Session>) {
    while let Some(event) = rx.recv().await {
        let msgs = translate_event(event, &session).await;
        for msg in msgs {
            session.broadcast(msg).await;
        }
    }
}

/// Generate a short session title using the LLM.
/// Falls back to text truncation if the provider call fails.
async fn generate_session_title(agent: &Agent, prompt_text: &str) -> String {
    use mew_message::{Message, Part, PartBase, Role, TextPart, Time};
    use mew_provider::{ChatParams, ProviderEvent, ReasoningConfig, Request};
    use std::time::{SystemTime, UNIX_EPOCH};
    use ulid::Ulid;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let system = "You generate concise session titles. Respond with ONLY a 3-5 word title. No quotes, no punctuation at the end, no explanation.".to_string();

    let user_msg = Message {
        id: Ulid::new(),
        session_id: Ulid::new(),
        role: Role::User,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: Ulid::new(),
                message_id: Ulid::new(),
                session_id: Ulid::new(),
            },
            text: format!(
                "Generate a 3-5 word title for a session that starts with this message:\n\n{}",
                prompt_text.chars().take(500).collect::<String>()
            ),
            synthetic: false,
        })],
        time: Time {
            created: now,
            completed: None,
        },
        assistant: None,
    };

    // Explicitly disable thinking/reasoning for the title generation call.
    let mut reasoning_params = serde_json::Map::new();
    reasoning_params.insert("type".into(), "disabled".into());

    let req = Request {
        model: String::new(),
        messages: vec![user_msg],
        tools: vec![],
        system,
        reasoning: Some(ReasoningConfig {
            params: reasoning_params,
        }),
        params: Some(ChatParams {
            temperature: Some(0.3),
            max_tokens: Some(30),
            ..Default::default()
        }),
        headers: Default::default(),
    };

    match agent.provider.stream(req).await {
        Ok(mut stream) => {
            let mut title = String::new();
            let mut current_part_is_text = false;
            while let Some(event) = futures::StreamExt::next(&mut stream).await {
                match event {
                    ProviderEvent::PartStart { part } => {
                        current_part_is_text = matches!(part, Part::Text(_));
                    }
                    ProviderEvent::PartDelta { delta, .. } => {
                        if current_part_is_text {
                            title.push_str(&delta);
                        }
                    }
                    ProviderEvent::MessageEnd { .. } => break,
                    _ => {}
                }
            }
            let trimmed = title.trim().trim_matches('"').to_string();
            if trimmed.is_empty() {
                derive_session_title(prompt_text)
            } else {
                trimmed.chars().take(80).collect()
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "title generation failed, using fallback");
            derive_session_title(prompt_text)
        }
    }
}

/// Fallback: derive a short session title from the first user message.
/// Truncates to 60 chars, collapses whitespace, strips newlines.
fn derive_session_title(text: &str) -> String {
    let cleaned: String = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
    if cleaned.len() == 60 && text.len() > 60 {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}

/// Background task that generates AI summaries for sessions that have
/// been idle for >10 minutes. Runs every 5 minutes.
async fn idle_summary_task(
    session_manager: std::sync::Arc<crate::session::SessionManager>,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::time::Duration;
    let idle_threshold = Duration::from_secs(600); // 10 minutes
    let check_interval = Duration::from_secs(300); // 5 minutes

    loop {
        tokio::time::sleep(check_interval).await;

        if !enabled.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }

        // Collect idle sessions (id, Arc) without holding the active lock.
        let candidates: Vec<(String, Arc<Session>)> = {
            let active = session_manager.active.lock().await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let mut to_process: Vec<(String, Arc<Session>)> = Vec::new();
            for (id, session) in active.iter() {
                let last = {
                    let agent = session.agent.lock().await;
                    let msgs = agent.messages.lock().await;
                    msgs.last().map(|m| m.time.created).unwrap_or(0)
                };
                if now - last >= idle_threshold.as_millis() as i64 {
                    to_process.push((id.clone(), session.clone()));
                }
            }
            to_process
        };

        for (id, session) in candidates {
            // Check if already has a summary.
            let dir = mew_session::session_dir();
            let has_summary = mew_session::Meta::read(&dir, &id)
                .await
                .ok()
                .flatten()
                .and_then(|m| m.summary)
                .is_some();
            if has_summary {
                continue;
            }

            let session_clone = session.clone();
            let id_clone = id.clone();
            tokio::spawn(async move {
                if let Some(summary) = {
                    let agent_guard = session_clone.agent.lock().await;
                    let agent_ref: &Agent = &agent_guard;
                    generate_session_summary(agent_ref).await
                } {
                    if let Ok(Some(mut meta)) =
                        mew_session::Meta::read(&dir, &id_clone).await
                    {
                        let _ = meta.set_summary(&dir, summary.clone()).await;
                    }
                    session_clone
                        .broadcast(mew_protocol::ServerMessage::SessionSummaryChanged {
                            session_id: id_clone.clone(),
                            summary,
                        })
                        .await;
                }
            });
        }
    }
}

/// Generate a short summary of the session's conversation.
async fn generate_session_summary(agent: &Agent) -> Option<String> {
    use mew_message::Role;
    use mew_provider::{ChatParams, ProviderEvent, ReasoningConfig};
    let messages = agent.messages.lock().await.clone();
    if messages.is_empty() {
        return None;
    }
    // Build a condensed transcript (first user msg + last few exchanges).
    let mut transcript = String::new();
    transcript.push_str("Summarize this conversation in 1-2 sentences (max 30 words):\n\n");
    for msg in messages.iter().take(20) {
        if let Role::User = msg.role {
            transcript.push_str("User: ");
        } else {
            transcript.push_str("Assistant: ");
        }
        for part in &msg.parts {
            if let mew_message::Part::Text(tp) = part {
                transcript.push_str(&tp.text);
                transcript.push('\n');
            }
        }
        if transcript.len() > 2000 {
            break;
        }
    }
    let user_msg = mew_message::Message {
        id: ulid::Ulid::new(),
        session_id: ulid::Ulid::new(),
        role: Role::User,
        parts: vec![mew_message::Part::Text(mew_message::TextPart {
            base: mew_message::PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: transcript.chars().take(2000).collect(),
            synthetic: false,
        })],
        time: mew_message::Time { created: 0, completed: None },
        assistant: None,
    };
    let mut reasoning_params = serde_json::Map::new();
    reasoning_params.insert("type".into(), "disabled".into());
    let req = mew_provider::Request {
        model: String::new(),
        messages: vec![user_msg],
        tools: vec![],
        system: "You write concise 1-2 sentence summaries. No preamble, no quotes, no bullet points.".to_string(),
        reasoning: Some(ReasoningConfig { params: reasoning_params }),
        params: Some(ChatParams { temperature: Some(0.3), max_tokens: Some(60), ..Default::default() }),
        headers: Default::default(),
    };
    match agent.provider.stream(req).await {
        Ok(mut stream) => {
            let mut summary = String::new();
            let mut current_part_is_text = false;
            while let Some(event) = futures::StreamExt::next(&mut stream).await {
                match event {
                    ProviderEvent::PartStart { part } => {
                        current_part_is_text = matches!(part, mew_message::Part::Text(_));
                    }
                    ProviderEvent::PartDelta { delta, .. } => {
                        if current_part_is_text { summary.push_str(&delta); }
                    }
                    ProviderEvent::MessageEnd { .. } => break,
                    _ => {}
                }
            }
            let trimmed = summary.trim().to_string();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        }
        Err(_) => None,
    }
}

/// Translate a single `AgentEvent` (owned) into zero or more `ServerMessage`s.
/// Channel-bearing events are converted to wire requests with fresh IDs;
/// the `oneshot::Sender` is stashed in the `Session` for later response.
async fn translate_event(event: AgentEvent, session: &Session) -> Vec<ServerMessage> {
    match event {
        AgentEvent::Provider(pe) => {
            vec![ServerMessage::Provider {
                event: mew_protocol::provider_event_to_wire(&pe),
            }]
        }
        AgentEvent::ToolStart { call_id } => {
            vec![ServerMessage::ToolStart { call_id }]
        }
        AgentEvent::ToolEnd { call_id, success } => {
            vec![ServerMessage::ToolEnd { call_id, success }]
        }
        AgentEvent::PartUpdated { part_id, part } => {
            vec![ServerMessage::PartUpdated { part_id, part }]
        }
        AgentEvent::ToolProgress { call_id, chunk } => {
            vec![ServerMessage::ToolProgress { call_id, chunk }]
        }
        AgentEvent::Error(msg) => {
            vec![ServerMessage::ErrorEvent { message: msg }]
        }
        AgentEvent::PermissionRequest { call, tx } => {
            let id = session.next_request_id();
            session.pending_permissions.lock().await.insert(id, tx);
            vec![ServerMessage::PermissionRequest {
                request_id: id,
                tool_name: call.tool_name,
                input: call.input,
            }]
        }
        AgentEvent::WorkspacePermissionRequest { path, tx } => {
            let id = session.next_request_id();
            session.pending_permissions.lock().await.insert(id, tx);
            vec![ServerMessage::WorkspacePermissionRequest {
                request_id: id,
                path: path.display().to_string(),
            }]
        }
        AgentEvent::AskUser {
            call_id,
            questions,
            tx,
        } => {
            let id = session.next_request_id();
            session.pending_ask_user.lock().await.insert(id, tx);
            vec![ServerMessage::AskUserRequest {
                request_id: id,
                call_id,
                questions: questions
                    .into_iter()
                    .map(|q| Question {
                        prompt: q.prompt,
                        options: q
                            .options
                            .into_iter()
                            .map(|o| QuestionOption {
                                label: o.label,
                                description: o.description,
                            })
                            .collect(),
                    })
                    .collect(),
            }]
        }
        AgentEvent::SubagentStart {
            parent_call_id,
            name,
            child_session_id,
            display_name,
        } => {
            vec![ServerMessage::SubagentStart {
                parent_call_id,
                name,
                child_session_id,
                display_name,
            }]
        }
        AgentEvent::SubagentStatus {
            parent_call_id,
            tool_name,
            message,
        } => {
            vec![ServerMessage::SubagentStatus {
                parent_call_id,
                tool_name,
                message,
            }]
        }
        AgentEvent::SubagentEnd {
            parent_call_id,
            child_session_id,
            outcome,
        } => {
            vec![ServerMessage::SubagentEnd {
                parent_call_id,
                child_session_id,
                outcome: mew_protocol::subagent_outcome_to_wire(&outcome),
            }]
        }
        AgentEvent::SubagentPermissionRequest {
            parent_call_id,
            call,
            tx,
        } => {
            let id = session.next_request_id();
            session.pending_permissions.lock().await.insert(id, tx);
            vec![ServerMessage::SubagentPermissionRequest {
                request_id: id,
                parent_call_id,
                tool_name: call.tool_name,
                input: call.input,
            }]
        }
        AgentEvent::SubagentProgress { .. } => {
            // Recurse into the child event. The boxed child event is
            // unwrapped and translated recursively.
            // For the minimal slice we flatten one level — deep nesting
            // is rare and can be handled later.
            Vec::new()
        }
        AgentEvent::TodosUpdated { todos } => {
            vec![ServerMessage::TodosUpdated {
                todos: todos
                    .into_iter()
                    .map(|t| mew_protocol::Todo {
                        id: t.id,
                        content: t.content,
                        status: t.status.as_str().to_string(),
                        depends_on: t.depends_on,
                    })
                    .collect(),
            }]
        }
        AgentEvent::PersonaSwitchRequested { name } => {
            vec![ServerMessage::PersonaSwitchRequested { name }]
        }
        AgentEvent::JobUpdate {
            job_id,
            command,
            state,
        } => {
            vec![ServerMessage::JobUpdate {
                job_id,
                command,
                state,
            }]
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn send_msg<S: AsyncRead + AsyncWrite + Unpin + Send>(
    ws_tx: &mut WsSink<S>,
    msg: ServerMessage,
) -> Result<()> {
    let json = mew_protocol::encode_json(&msg)?;
    ws_tx.send(Message::Text(json)).await?;
    Ok(())
}
