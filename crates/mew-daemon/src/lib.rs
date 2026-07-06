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
use mew_protocol::{ClientMessage, Question, QuestionOption, ServerMessage, SessionState};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tungstenite::Message;

pub mod client;
pub mod files;
pub mod groups;
pub mod session;

#[cfg(feature = "iroh")]
pub mod iroh_transport;

#[cfg(feature = "iroh")]
pub use iroh_transport::{
    default_allowlist_path, default_secret_key_path, load_or_create_secret_key, run_iroh,
    IrohStream, MewIrohHandler, NodeIdAllowlist, MEW_ALPN,
};

pub use client::DaemonClient;
pub use session::{AttachError, PendingRequest, Session, SessionManager};

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
    /// Session groups sidecar store (groups.json).
    pub groups_store: Arc<groups::GroupsStore>,
    /// Daemon-wide flag for auto-summary. Shared across all connections
    /// so the idle-summary task is spawned once, not per-connection.
    pub auto_summary_enabled: Arc<std::sync::atomic::AtomicBool>,
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
            session_dir.clone(),
            switcher,
            lister,
        ));
        let groups_state = {
            let path = session_dir.join("groups.json");
            std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<groups::GroupsState>(&bytes).ok())
                .unwrap_or_default()
        };
        let groups_store = Arc::new(groups::GroupsStore::from_state(groups_state, session_dir));
        Self {
            builder,
            model_switcher: None,
            model_lister: None,
            thinking_setter: None,
            session_manager,
            groups_store,
            auto_summary_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
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

    /// Adopt the shared session state (`session_manager`, `groups_store`,
    /// and `auto_summary_enabled`) from `other`.
    ///
    /// Because these fields are all `Arc`s, copying them shares the same
    /// underlying state instead of creating fresh copies. This is what the
    /// dual-listener (Unix + TCP) setup uses so that both listeners see the
    /// same sessions and only one `idle_summary_task` scans them all.
    pub fn share_session_state(mut self, other: &DaemonServer) -> Self {
        self.session_manager = Arc::clone(&other.session_manager);
        self.groups_store = Arc::clone(&other.groups_store);
        self.auto_summary_enabled = Arc::clone(&other.auto_summary_enabled);
        self
    }

    /// Run the daemon, listening on the given Unix socket path.
    /// Blocks until the listener is closed, a signal (SIGINT/SIGTERM) is
    /// received, or an unrecoverable error occurs.
    pub async fn run(self, socket_path: &str) -> Result<()> {
        // Check if the socket is already live; bail if so, remove if stale.
        check_socket_liveness(socket_path)?;

        let listener = UnixListener::bind(socket_path)
            .with_context(|| format!("bind socket {}", socket_path))?;
        info!(socket = socket_path, "mew daemon listening");

        let session_manager = self.session_manager.clone();
        let groups_store = self.groups_store.clone();
        let thinking_setter = self.thinking_setter.clone();
        let auto_summary_enabled = self.auto_summary_enabled.clone();

        // Spawn the idle-summary task once (daemon-wide, not per-connection).
        {
            let sm = session_manager.clone();
            let flag = auto_summary_enabled.clone();
            tokio::spawn(async move {
                idle_summary_task(sm, flag).await;
            });
        }

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
                            let groups_store = groups_store.clone();
                            let thinking_setter = thinking_setter.clone();
                            let auto_summary_enabled = auto_summary_enabled.clone();
                            tokio::spawn(async move {
                                info!(conn_id, "connection accepted");
                                if let Err(e) = handle_connection(stream, session_manager, groups_store, thinking_setter, auto_summary_enabled).await {
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
        let groups_store = self.groups_store.clone();
        let thinking_setter = self.thinking_setter.clone();
        let auto_summary_enabled = self.auto_summary_enabled.clone();

        // Spawn the idle-summary task once (daemon-wide, not per-connection).
        {
            let sm = session_manager.clone();
            let flag = auto_summary_enabled.clone();
            tokio::spawn(async move {
                idle_summary_task(sm, flag).await;
            });
        }

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
                            let groups_store = groups_store.clone();
                            let thinking_setter = thinking_setter.clone();
                            let auto_summary_enabled = auto_summary_enabled.clone();
                            tokio::spawn(async move {
                                info!(conn_id, %peer, "connection accepted (tcp)");
                                if let Err(e) = handle_connection(stream, session_manager, groups_store, thinking_setter, auto_summary_enabled).await {
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
    groups_store: Arc<groups::GroupsStore>,
    thinking_setter: Option<ThinkingSetter>,
    auto_summary_enabled: Arc<std::sync::atomic::AtomicBool>,
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

    // auto_summary_enabled is passed in from the daemon (daemon-wide task
    // is spawned once in run()/run_tcp(), not per-connection).

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
            ClientMessage::NewSession { cwd, client_kind } => {
                // Validate cwd if provided: must exist and be a directory.
                if let Some(ref cwd_str) = cwd {
                    let cwd_path = PathBuf::from(cwd_str);
                    if !cwd_path.exists() {
                        reply(ServerMessage::Error {
                            message: format!("session cwd does not exist: {cwd_str}"),
                        });
                        continue;
                    }
                    if !cwd_path.is_dir() {
                        reply(ServerMessage::Error {
                            message: format!("session cwd is not a directory: {cwd_str}"),
                        });
                        continue;
                    }
                }
                match session_manager.create(cwd.clone().map(PathBuf::from)).await {
                    Ok(session) => {
                        let (cid, was_first) =
                            session.attach_client(client_tx.clone(), client_kind).await;
                        client_id = Some(cid);
                        attached_session = Some(session.clone());
                        let session_id = session.id.clone();
                        let (model, provider, permission_mode) = {
                            let agent = session.agent.lock().await;
                            (
                                session.model.lock().await.clone(),
                                session.provider.lock().await.clone(),
                                Some(agent.permission_mode().id().to_string()),
                            )
                        };
                        // cwd is already persisted in meta at create time;
                        // no need to set it again here.
                        reply(ServerMessage::SessionReady {
                            session_id,
                            model,
                            provider,
                            permission_mode,
                        });
                        // Notify other clients that a new client joined.
                        if !was_first {
                            session
                                .broadcast(ServerMessage::ClientAttached {
                                    client_id: cid,
                                    client_kind,
                                })
                                .await;
                        }
                    }
                    Err(e) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to create session: {e}"),
                        });
                    }
                }
            }
            ClientMessage::AttachSession {
                session_id,
                client_kind,
            } => {
                match session_manager.attach(&session_id).await {
                    Ok(session) => {
                        let (cid, was_first) =
                            session.attach_client(client_tx.clone(), client_kind).await;
                        client_id = Some(cid);
                        attached_session = Some(session.clone());

                        let (model, provider, permission_mode) = {
                            let agent = session.agent.lock().await;
                            (
                                session.model.lock().await.clone(),
                                session.provider.lock().await.clone(),
                                Some(agent.permission_mode().id().to_string()),
                            )
                        };
                        reply(ServerMessage::SessionReady {
                            session_id: session_id.clone(),
                            model,
                            provider,
                            permission_mode,
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

                        // Replay any outstanding permission / ask-user requests
                        // so a client attaching while the agent is blocked can
                        // answer them (the payloads aren't in the history).
                        {
                            let perms = session.pending_permissions.lock().await;
                            for pending in perms.values() {
                                reply(pending.payload.clone());
                            }
                        }
                        {
                            let asks = session.pending_ask_user.lock().await;
                            for pending in asks.values() {
                                reply(pending.payload.clone());
                            }
                        }

                        // Replay current flagged-files set so the UI has it
                        // immediately after attach.
                        {
                            let agent = session.agent.lock().await;
                            let files: Vec<mew_protocol::FlaggedFileWire> = agent
                                .flagged_files
                                .lock()
                                .await
                                .iter()
                                .map(|f| mew_protocol::FlaggedFileWire {
                                    path: f.path.display().to_string(),
                                    reason: Some(
                                        mew_tools::tools::flag_important::flag_mode_label(f.mode)
                                            .to_string(),
                                    ),
                                })
                                .collect();
                            drop(agent);
                            if !files.is_empty() {
                                reply(ServerMessage::FlaggedFilesChanged {
                                    session_id: session_id.clone(),
                                    files,
                                });
                            }
                        }

                        // Notify other clients that a new client joined.
                        if !was_first {
                            session
                                .broadcast(ServerMessage::ClientAttached {
                                    client_id: cid,
                                    client_kind,
                                })
                                .await;
                        }
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
                let groups = groups_store.list().await;
                reply(ServerMessage::GroupList { groups });
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
                let session_mgr = session_manager.clone();
                tokio::spawn(async move {
                    let _guard = session.turn_lock.lock().await;
                    let has_turn: bool = session.current_turn_cancel.lock().await.is_some();
                    if has_turn {
                        let _ = client_tx.send(ServerMessage::Error {
                            message: "turn in progress".into(),
                        });
                        return;
                    }
                    let agent = session.agent.lock().await.clone();
                    let prompt_text = text.clone();
                    let auto_title = auto_title_enabled;
                    let had_error = run_turn(&session, &session_mgr, &agent, text).await;

                    // Generate a session title from the first user message
                    // if we haven't already. Uses a lightweight LLM call;
                    // falls back to text truncation on error. Skipped if the
                    // user has disabled auto-title generation.
                    if auto_title && !had_error {
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
            ClientMessage::Ping => {
                reply(ServerMessage::Pong {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                });
            }
            ClientMessage::ListProjects => {
                let projects = list_projects(&session_manager).await;
                reply(ServerMessage::ProjectList { projects });
            }
            ClientMessage::PermissionResponse {
                request_id,
                decision,
            } => {
                if let Some(session) = &attached_session {
                    let pending = session.pending_permissions.lock().await.remove(&request_id);
                    if let Some(pending) = pending {
                        let _ = pending.responder.send(decision.into());
                    }
                    session
                        .broadcast(ServerMessage::RequestResolved { request_id })
                        .await;
                    // Broadcast updated attention count.
                    let perm_count = session.pending_permissions.lock().await.len() as u32;
                    let q_count = session.pending_ask_user.lock().await.len() as u32;
                    session_manager
                        .broadcast_all(ServerMessage::SessionAttentionChanged {
                            session_id: session.id.clone(),
                            pending_permissions: perm_count,
                            pending_questions: q_count,
                        })
                        .await;
                }
            }
            ClientMessage::AskUserResponse {
                request_id,
                answers,
            } => {
                if let Some(session) = &attached_session {
                    let pending = session.pending_ask_user.lock().await.remove(&request_id);
                    if let Some(pending) = pending {
                        let _ = pending.responder.send(answers);
                    }
                    session
                        .broadcast(ServerMessage::RequestResolved { request_id })
                        .await;
                    // Broadcast updated attention count.
                    let perm_count = session.pending_permissions.lock().await.len() as u32;
                    let q_count = session.pending_ask_user.lock().await.len() as u32;
                    session_manager
                        .broadcast_all(ServerMessage::SessionAttentionChanged {
                            session_id: session.id.clone(),
                            pending_permissions: perm_count,
                            pending_questions: q_count,
                        })
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
                let session_manager_clone = session_manager.clone();
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
                            "/wiki" => {
                                let _ = client_tx.send(ServerMessage::SlashResult {
                                    text: "Generating wiki… (this may take a moment)".to_string(),
                                });
                                let wiki_prompt = "You are generating a repository wiki. \
                                    Analyze the codebase structure using your read, glob, and grep tools. \
                                    Then write a file at .mew/wiki.md with:\n\
                                    1. YAML frontmatter with `generated_at` (ISO timestamp) and `git_head` (run `git rev-parse HEAD` via bash)\n\
                                    2. A markdown document covering:\n\
                                    - Project overview (what this codebase is)\n\
                                    - Directory structure (key directories and their responsibilities)\n\
                                    - Build & test commands\n\
                                    - Configuration conventions\n\
                                    - Key architectural patterns\n\n\
                                    Keep it concise (under 200 lines). Use the write tool to create .mew/wiki.md.";
                                drop(agent);
                                let wiki_agent = session.agent.lock().await.clone();
                                let had_error = run_turn(
                                    &session,
                                    &session_manager_clone,
                                    &wiki_agent,
                                    wiki_prompt.to_string(),
                                )
                                .await;
                                if had_error {
                                    Some("wiki generation failed".to_string())
                                } else {
                                    Some("wiki generated at .mew/wiki.md".to_string())
                                }
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
            ClientMessage::SetPermissionMode { mode } => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::Error {
                        message: "no session".into(),
                    });
                    continue;
                };
                let session = session.clone();
                tokio::spawn(async move {
                    let _guard = session.turn_lock.lock().await;
                    let parsed = mew_hooks::PermissionMode::from_id(&mode);
                    match parsed {
                        Some(m) => {
                            {
                                let agent = session.agent.lock().await;
                                agent.set_permission_mode(m);
                            }
                            // Broadcast to all attached clients.
                            session
                                .broadcast(ServerMessage::PermissionModeChanged {
                                    mode: m.id().to_string(),
                                })
                                .await;
                        }
                        None => {
                            session
                                .broadcast(ServerMessage::Error {
                                    message: format!("unknown permission mode: {mode}"),
                                })
                                .await;
                        }
                    }
                });
            }
            ClientMessage::YieldControl {} => {
                let Some(session) = attached_session.clone() else {
                    continue;
                };
                let Some(cid) = client_id else {
                    continue;
                };
                session
                    .broadcast(ServerMessage::ControlYielded { client_id: cid })
                    .await;
            }

            // -- Phase 2: groups & archive --
            ClientMessage::CreateGroup { name, color } => {
                match groups_store.create_group(name, color).await {
                    Ok(groups) => {
                        session_manager.broadcast_groups(groups).await;
                    }
                    Err(e) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to create group: {e}"),
                        });
                    }
                }
            }
            ClientMessage::UpdateGroup {
                group_id,
                name,
                color,
                order,
            } => {
                match groups_store
                    .update_group(&group_id, name, color.map(Some), order)
                    .await
                {
                    Ok(groups) => {
                        session_manager.broadcast_groups(groups).await;
                    }
                    Err(e) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to update group: {e}"),
                        });
                    }
                }
            }
            ClientMessage::DeleteGroup { group_id } => {
                match groups_store.delete_group(&group_id).await {
                    Ok(groups) => {
                        let dir = session_manager.session_dir.clone();
                        if let Ok(entries) = tokio::fs::read_dir(&dir).await {
                            let mut entries = entries;
                            while let Ok(Some(entry)) = entries.next_entry().await {
                                if let Some(id) = entry.file_name().to_str() {
                                    if let Ok(Some(mut meta)) =
                                        mew_session::Meta::read(&dir, id).await
                                    {
                                        if meta.group_id.as_deref() == Some(&group_id) {
                                            let _ = meta.set_group_id(&dir, None).await;
                                        }
                                    }
                                }
                            }
                        }
                        session_manager.broadcast_groups(groups).await;
                    }
                    Err(e) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to delete group: {e}"),
                        });
                    }
                }
            }
            ClientMessage::AssignSessionGroup {
                session_id,
                group_id,
                position: _,
            } => {
                let dir = session_manager.session_dir.clone();
                let mut meta_group_id = None;
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    let _ = meta.set_group_id(&dir, group_id.clone()).await;
                    meta_group_id = meta.group_id.clone();
                }
                match groups_store.assign_session(&session_id, group_id).await {
                    Ok(groups) => {
                        session_manager.broadcast_groups(groups).await;
                        session_manager
                            .broadcast_all(ServerMessage::SessionMetaChanged {
                                session_id: session_id.clone(),
                                archived: None,
                                pinned: None,
                                group_id: meta_group_id,
                            })
                            .await;
                    }
                    Err(e) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to assign session: {e}"),
                        });
                    }
                }
            }
            ClientMessage::ArchiveSession {
                session_id,
                archived,
            } => {
                let dir = session_manager.session_dir.clone();
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    let _ = meta.set_archived(&dir, archived).await;
                }
                session_manager
                    .broadcast_all(ServerMessage::SessionMetaChanged {
                        session_id: session_id.clone(),
                        archived: Some(archived),
                        pinned: None,
                        group_id: None,
                    })
                    .await;
            }
            ClientMessage::PinSession { session_id, pinned } => {
                let dir = session_manager.session_dir.clone();
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    let _ = meta.set_pinned(&dir, pinned).await;
                }
                session_manager
                    .broadcast_all(ServerMessage::SessionMetaChanged {
                        session_id: session_id.clone(),
                        archived: None,
                        pinned: Some(pinned),
                        group_id: None,
                    })
                    .await;
            }

            // -- Phase 3: File service --
            ClientMessage::ListDir { session_id, path } => {
                match crate::files::handle_list_dir(&session_manager, &session_id, path).await {
                    Ok(listing) => reply(listing),
                    Err(e) => reply(ServerMessage::Error {
                        message: format!("list_dir: {e}"),
                    }),
                }
            }
            ClientMessage::ReadFilePreview {
                session_id,
                path,
                max_bytes,
            } => {
                match crate::files::handle_read_preview(
                    &session_manager,
                    &session_id,
                    &path,
                    max_bytes,
                )
                .await
                {
                    Ok(preview) => reply(preview),
                    Err(e) => reply(ServerMessage::Error {
                        message: format!("read_file_preview: {e}"),
                    }),
                }
            }
            ClientMessage::GitStatus { session_id } => {
                match crate::files::handle_git_status(&session_manager, &session_id).await {
                    Ok(result) => reply(result),
                    Err(e) => reply(ServerMessage::Error {
                        message: format!("git_status: {e}"),
                    }),
                }
            }
            ClientMessage::WatchWorkspace { .. } => {
                // Watcher not implemented in v1; acknowledge silently.
            }
            ClientMessage::OpenPath { session_id, path } => {
                match crate::files::handle_open_path(&session_manager, &session_id, &path).await {
                    Ok(()) => {}
                    Err(e) => reply(ServerMessage::Error {
                        message: format!("open_path: {e}"),
                    }),
                }
            }

            // -- Flagged files --
            ClientMessage::UnflagFile { session_id, path } => {
                // Remove from the agent's flagged_files set if the session is active.
                let active = session_manager.active.lock().await;
                if let Some(session) = active.get(&session_id).cloned() {
                    drop(active);
                    let agent = session.agent.lock().await;
                    let mut guard = agent.flagged_files.lock().await;
                    guard.retain(|f| f.path.display().to_string() != path);
                    let files: Vec<mew_protocol::FlaggedFileWire> = guard
                        .iter()
                        .map(|f| mew_protocol::FlaggedFileWire {
                            path: f.path.display().to_string(),
                            reason: Some(
                                mew_tools::tools::flag_important::flag_mode_label(f.mode)
                                    .to_string(),
                            ),
                        })
                        .collect();
                    drop(guard);
                    drop(agent);
                    session
                        .broadcast(ServerMessage::FlaggedFilesChanged {
                            session_id: session_id.clone(),
                            files,
                        })
                        .await;
                } else {
                    drop(active);
                }
            }
        }
    }
    if let (Some(session), Some(cid)) = (&attached_session, client_id) {
        // Broadcast departure to remaining clients.
        session
            .broadcast(ServerMessage::ClientDetached { client_id: cid })
            .await;
        let was_last = session.detach_client(cid).await;
        if was_last {
            // Keep-warm: spawn a grace-period timer. If no client reattaches
            // before it fires, cancel the turn and unload the session.
            let session_id = session.id.clone();
            let session_clone = session.clone();
            let sm = session_manager.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                // Check if the session still has no clients.
                if session_clone.client_count().await == 0 {
                    let has_turn: bool = session_clone.current_turn_cancel.lock().await.is_some();
                    if has_turn {
                        session_clone.cancel_turn().await;
                    }
                    sm.remove(&session_id).await;
                }
            });
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
/// Run an agent turn with full turn management: cancel token, is_running,
/// activity broadcasts, forward_events, meta updates, usage broadcast, and
/// session alert. Returns `had_error`.
///
/// Does NOT do: user message broadcast, title generation — those are
/// Prompt-handler-specific.
async fn run_turn(
    session: &Arc<Session>,
    session_mgr: &Arc<SessionManager>,
    agent: &Agent,
    prompt_text: String,
) -> bool {
    let token = CancellationToken::new();
    *session.current_turn_cancel.lock().await = Some(token.clone());
    *session.is_running.lock().await = true;
    session_mgr
        .broadcast_activity(&session.id, SessionState::Running)
        .await;

    let rx = agent.run_with_parts(prompt_text, vec![], Some(token));
    let had_error = forward_events(rx, session.clone(), session_mgr.clone()).await;

    *session.current_turn_cancel.lock().await = None;
    *session.is_running.lock().await = false;

    // Update last_turn_failed + increment turn count.
    let dir = session_mgr.session_dir.clone();
    let usage_wire = {
        let agent = session.agent.lock().await;
        if let Some(mut meta) = agent.session_meta().await {
            let _ = meta.set_last_turn_failed(&dir, had_error).await;
            if let Some(u) = meta.usage.as_mut() {
                u.add_turn();
                let wire = mew_protocol::SessionUsageWire::from(&*u);
                let _ = meta.set_usage(&dir, wire.clone().into()).await;
                Some(wire)
            } else {
                None
            }
        } else {
            None
        }
    };

    // Broadcast idle state.
    session_mgr
        .broadcast_activity(&session.id, SessionState::Idle)
        .await;

    // Broadcast usage (only if we have data).
    if let Some(u) = &usage_wire {
        session_mgr
            .broadcast_all(ServerMessage::SessionUsageChanged {
                session_id: session.id.clone(),
                usage: u.clone(),
            })
            .await;
    }

    // Session alert: TurnComplete or TurnFailed.
    session_mgr
        .broadcast_all(ServerMessage::SessionAlert {
            session_id: session.id.clone(),
            title: session.display_title().await,
            kind: if had_error {
                mew_protocol::AlertKind::TurnFailed
            } else {
                mew_protocol::AlertKind::TurnComplete
            },
            detail: None,
        })
        .await;

    had_error
}

async fn forward_events(
    mut rx: tokio::sync::mpsc::Receiver<AgentEvent>,
    session: Arc<Session>,
    session_mgr: Arc<SessionManager>,
) -> bool {
    let mut had_error = false;
    while let Some(event) = rx.recv().await {
        if matches!(event, AgentEvent::Error(_)) {
            had_error = true;
        }
        let msgs = translate_event(event, &session, &session_mgr).await;
        for msg in msgs {
            // SessionAlert and SessionAttentionChanged go to ALL sessions.
            // Everything else goes to just this session's clients.
            if matches!(
                msg,
                ServerMessage::SessionAlert { .. } | ServerMessage::SessionAttentionChanged { .. }
            ) {
                session_mgr.broadcast_all(msg).await;
            } else {
                session.broadcast(msg).await;
            }
        }
    }
    had_error
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

/// Check if a Unix socket path has a live daemon listening.
///
/// If a connection succeeds, a daemon is already running — bail with an error.
/// If the connection is refused (or the path doesn't exist), the socket is stale
/// (or absent) — remove it and return Ok so the caller can bind.
///
/// This should be called before `daemonize()` in `main()` so the error reaches
/// the terminal, and again inside `run()` as a safety net.
pub fn check_socket_liveness(socket_path: &str) -> Result<()> {
    use std::os::unix::net::UnixStream;
    match UnixStream::connect(socket_path) {
        Ok(_) => anyhow::bail!(
            "a mew daemon is already running at {socket_path}. \
             Stop it first (`mew daemon --stop`) or use a different --socket path."
        ),
        Err(_) => {
            // Connection refused or path doesn't exist — stale or absent.
            let _ = std::fs::remove_file(socket_path);
            Ok(())
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

/// Collect known projects from session metas.
/// Deduped by canonicalized path, sorted by recency (most recent first).
async fn list_projects(
    session_manager: &std::sync::Arc<crate::session::SessionManager>,
) -> Vec<mew_protocol::ProjectInfo> {
    use std::collections::HashMap;
    use std::path::PathBuf;

    let dir = session_manager.session_dir.clone();
    let mut projects: HashMap<PathBuf, mew_protocol::ProjectInfo> = HashMap::new();

    // Walk session dirs and read meta.json for each.
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let Ok(Some(meta)) = mew_session::Meta::read(&dir, &file_name).await else {
                continue;
            };
            let Some(cwd_str) = &meta.cwd else {
                continue;
            };
            let path = PathBuf::from(cwd_str);
            let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());
            let display_name = canonical
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| cwd_str.clone());
            let last_used = meta.last_message_at.or(Some(meta.created_at));
            let entry = projects
                .entry(canonical)
                .or_insert_with(|| mew_protocol::ProjectInfo {
                    path: cwd_str.clone(),
                    display_name: display_name.clone(),
                    session_count: 0,
                    last_used_at: None,
                });
            entry.session_count += 1;
            if let Some(ts) = last_used {
                entry.last_used_at = Some(entry.last_used_at.map(|e| e.max(ts)).unwrap_or(ts));
            }
        }
    }

    // Sort by last_used_at descending (None sorts last).
    let mut result: Vec<_> = projects.into_values().collect();
    result.sort_by(|a, b| {
        b.last_used_at
            .unwrap_or(0)
            .cmp(&a.last_used_at.unwrap_or(0))
    });
    result
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
                    if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &id_clone).await {
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
        time: mew_message::Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    };
    let mut reasoning_params = serde_json::Map::new();
    reasoning_params.insert("type".into(), "disabled".into());
    let req = mew_provider::Request {
        model: String::new(),
        messages: vec![user_msg],
        tools: vec![],
        system:
            "You write concise 1-2 sentence summaries. No preamble, no quotes, no bullet points."
                .to_string(),
        reasoning: Some(ReasoningConfig {
            params: reasoning_params,
        }),
        params: Some(ChatParams {
            temperature: Some(0.3),
            max_tokens: Some(60),
            ..Default::default()
        }),
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
                        if current_part_is_text {
                            summary.push_str(&delta);
                        }
                    }
                    ProviderEvent::MessageEnd { .. } => break,
                    _ => {}
                }
            }
            let trimmed = summary.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(_) => None,
    }
}

/// Translate a single `AgentEvent` (owned) into zero or more `ServerMessage`s.
/// Channel-bearing events are converted to wire requests with fresh IDs;
/// the `oneshot::Sender` is stashed in the `Session` for later response.
async fn translate_event(
    event: AgentEvent,
    session: &Session,
    _session_mgr: &SessionManager,
) -> Vec<ServerMessage> {
    match event {
        AgentEvent::Provider(pe) => {
            // Intercept MessageEnd to accumulate usage on Meta.
            if let mew_provider::ProviderEvent::MessageEnd { usage, cost, .. } = &pe {
                let dir = session.session_dir.clone();
                let agent = session.agent.lock().await;
                if let Some(mut meta) = agent.session_meta().await {
                    let u = meta
                        .usage
                        .get_or_insert_with(mew_session::SessionUsage::default);
                    u.add_message(
                        usage.input as u64,
                        usage.output as u64,
                        usage.cache_read as u64,
                        usage.cache_write as u64,
                        *cost,
                    );
                    let usage_clone = u.clone();
                    let _ = meta.set_usage(&dir, usage_clone).await;
                }
            }
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
            let request = ServerMessage::PermissionRequest {
                request_id: id,
                tool_name: call.tool_name.clone(),
                input: call.input,
            };
            session.pending_permissions.lock().await.insert(
                id,
                PendingRequest {
                    payload: request.clone(),
                    responder: tx,
                },
            );
            let alert = ServerMessage::SessionAlert {
                session_id: session.id.clone(),
                title: session.display_title().await,
                kind: mew_protocol::AlertKind::PermissionNeeded,
                detail: Some(call.tool_name),
            };
            vec![
                request,
                alert,
                ServerMessage::SessionAttentionChanged {
                    session_id: session.id.clone(),
                    pending_permissions: session.pending_permissions.lock().await.len() as u32,
                    pending_questions: session.pending_ask_user.lock().await.len() as u32,
                },
            ]
        }
        AgentEvent::WorkspacePermissionRequest { path, tx } => {
            let id = session.next_request_id();
            let request = ServerMessage::WorkspacePermissionRequest {
                request_id: id,
                path: path.display().to_string(),
            };
            session.pending_permissions.lock().await.insert(
                id,
                PendingRequest {
                    payload: request.clone(),
                    responder: tx,
                },
            );
            vec![request]
        }
        AgentEvent::AskUser {
            call_id,
            questions,
            tx,
        } => {
            let id = session.next_request_id();
            let request = ServerMessage::AskUserRequest {
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
            };
            session.pending_ask_user.lock().await.insert(
                id,
                PendingRequest {
                    payload: request.clone(),
                    responder: tx,
                },
            );
            vec![
                request,
                ServerMessage::SessionAlert {
                    session_id: session.id.clone(),
                    title: session.display_title().await,
                    kind: mew_protocol::AlertKind::InputNeeded,
                    detail: None,
                },
                ServerMessage::SessionAttentionChanged {
                    session_id: session.id.clone(),
                    pending_permissions: session.pending_permissions.lock().await.len() as u32,
                    pending_questions: session.pending_ask_user.lock().await.len() as u32,
                },
            ]
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
            let request = ServerMessage::SubagentPermissionRequest {
                request_id: id,
                parent_call_id,
                tool_name: call.tool_name,
                input: call.input,
            };
            session.pending_permissions.lock().await.insert(
                id,
                PendingRequest {
                    payload: request.clone(),
                    responder: tx,
                },
            );
            vec![request]
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
        AgentEvent::FileDelta {
            path,
            added,
            removed,
        } => {
            let dir = session.session_dir.clone();
            let session_id = session.id.clone();
            let (total_added, total_removed, files_changed) = {
                let agent = session.agent.lock().await;
                if let Some(mut meta) = agent.session_meta().await {
                    let _ = meta.apply_file_delta(&dir, &path, added, removed).await;
                    let stats = meta.change_stats.as_ref();
                    (
                        stats.map(|s| s.added).unwrap_or(0),
                        stats.map(|s| s.removed).unwrap_or(0),
                        stats.map(|s| s.files.len() as u64).unwrap_or(0),
                    )
                } else {
                    (0u64, 0u64, 0u64)
                }
            };
            vec![ServerMessage::SessionStatsChanged {
                session_id,
                added: total_added,
                removed: total_removed,
                files_changed,
            }]
        }
        AgentEvent::FlaggedFilesChanged { files } => {
            let wire_files: Vec<mew_protocol::FlaggedFileWire> = files
                .into_iter()
                .map(|f| mew_protocol::FlaggedFileWire {
                    path: f.path,
                    reason: f.reason,
                })
                .collect();
            vec![ServerMessage::FlaggedFilesChanged {
                session_id: session.id.clone(),
                files: wire_files,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_liveness_guard_rejects_live_socket() {
        use std::os::unix::net::UnixListener;
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        // Bind a listener to simulate a running daemon.
        let _listener = UnixListener::bind(&socket_path).unwrap();

        // check_socket_liveness should fail — a daemon is already listening.
        let result = check_socket_liveness(socket_path.to_str().unwrap());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("already running"), "unexpected error: {msg}");
    }

    #[test]
    fn test_socket_liveness_guard_removes_stale_socket() {
        // Create a file at the socket path that isn't a listening socket.
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("stale.sock");
        std::fs::write(&socket_path, b"stale").unwrap();

        // check_socket_liveness should succeed — connection will be refused,
        // stale file removed.
        let result = check_socket_liveness(socket_path.to_str().unwrap());
        assert!(result.is_ok(), "stale socket should be removed: {result:?}");
        assert!(!socket_path.exists(), "stale socket file should be removed");
    }

    #[test]
    fn test_socket_liveness_guard_ok_when_no_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("nonexistent.sock");

        let result = check_socket_liveness(socket_path.to_str().unwrap());
        assert!(result.is_ok(), "missing socket should be fine: {result:?}");
    }
}
