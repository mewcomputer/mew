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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use mew_agent::{Agent, AgentEvent, GoalDecision};
use mew_protocol::{
    ClientKind, ClientMessage, Question, QuestionOption, RemoteScope, ServerMessage, SessionState,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::accept_async;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tungstenite::Message;

const HISTORY_PAGE_MAX_BYTES: usize = 8 * 1024 * 1024;

struct BrowserRegistration {
    page_instance: String,
    session: Arc<session::Session>,
}

pub mod browser;
#[cfg(test)]
mod browser_tests;
pub mod client;
pub mod files;
pub mod groups;
pub mod projects;
pub mod remote;
pub mod session;
pub mod terminal;
mod workspace;

#[cfg(feature = "iroh")]
pub mod iroh_transport;

pub use client::DaemonClient;
pub use session::{AttachError, Session, SessionManager};

pub(crate) fn workspace_roots() -> Vec<PathBuf> {
    mew_config::load()
        .map(|config| config.workspace.roots)
        .unwrap_or_default()
}

/// Parameters passed to the agent-builder closure.
pub struct AgentBuildParams {
    pub session_id: String,
    pub writer: mew_session::Writer,
    pub cwd: Option<std::path::PathBuf>,
    /// Complete saved runtime choices; None uses startup defaults for legacy sessions.
    pub policy: Option<mew_session::SessionPolicy>,
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
    /// Workspace pin preferences (projects.json).
    pub project_pins: Arc<projects::ProjectPinsStore>,
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
        let project_pins = Arc::new(projects::ProjectPinsStore::from_session_dir(
            session_dir.clone(),
        ));
        let session_manager = Arc::new(SessionManager::new(
            builder.clone(),
            session_dir.clone(),
            switcher,
            lister,
            project_pins.clone(),
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
            project_pins,
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
            self.project_pins.clone(),
        ));
        self
    }

    /// Enable thinking variant switching.
    pub fn with_thinking_setter(mut self, setter: ThinkingSetter) -> Self {
        self.thinking_setter = Some(setter);
        self
    }

    /// Clone the listener-facing handles while keeping the same session
    /// manager. Used when local and remote transports run concurrently.
    pub fn clone_for_listener(&self) -> Self {
        Self {
            builder: self.builder.clone(),
            model_switcher: self.model_switcher.clone(),
            model_lister: self.model_lister.clone(),
            thinking_setter: self.thinking_setter.clone(),
            session_manager: self.session_manager.clone(),
            groups_store: self.groups_store.clone(),
            project_pins: self.project_pins.clone(),
            auto_summary_enabled: self.auto_summary_enabled.clone(),
        }
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

fn remote_scope_label(scope: RemoteScope) -> &'static str {
    match scope {
        RemoteScope::Observe => "observe",
        RemoteScope::Collaborate => "collaborate",
        RemoteScope::Control => "control",
    }
}

fn remote_message_allowed(scope: RemoteScope, message: &ClientMessage) -> bool {
    if scope == RemoteScope::Control {
        return matches!(
            message,
            ClientMessage::RemoteHello { .. }
                | ClientMessage::NewSession { .. }
                | ClientMessage::NewSessionInGroup { .. }
                | ClientMessage::AttachSession { .. }
                | ClientMessage::LoadSessionHistory { .. }
                | ClientMessage::ListSessions
                | ClientMessage::DeleteSession { .. }
                | ClientMessage::RenameSession { .. }
                | ClientMessage::SetAutoTitle { .. }
                | ClientMessage::SetAutoSummary { .. }
                | ClientMessage::Prompt { .. }
                | ClientMessage::Cancel
                | ClientMessage::PermissionResponse { .. }
                | ClientMessage::AskUserResponse { .. }
                | ClientMessage::GoalResponse { .. }
                | ClientMessage::SlashCommand { .. }
                | ClientMessage::ListModels
                | ClientMessage::SwitchModel { .. }
                | ClientMessage::SetThinkingVariant { .. }
                | ClientMessage::SetPermissionMode { .. }
                | ClientMessage::YieldControl { .. }
                | ClientMessage::CreateGroup { .. }
                | ClientMessage::UpdateGroup { .. }
                | ClientMessage::DeleteGroup { .. }
                | ClientMessage::AssignSessionGroup { .. }
                | ClientMessage::ArchiveSession { .. }
                | ClientMessage::PinSession { .. }
                | ClientMessage::ListDir { .. }
                | ClientMessage::ListFilesystemDir { .. }
                | ClientMessage::ReadFilePreview { .. }
                | ClientMessage::GitStatus { .. }
                | ClientMessage::ReadSessionDiff { .. }
                | ClientMessage::WatchWorkspace { .. }
                | ClientMessage::OpenPath { .. }
                | ClientMessage::UnflagFile { .. }
                | ClientMessage::ListPersonas
                | ClientMessage::SwitchPersona { .. }
                | ClientMessage::ListProjects
                | ClientMessage::PinProject { .. }
                | ClientMessage::RegenerateTitle { .. }
                | ClientMessage::Ping
                | ClientMessage::BrowserOpen { .. }
                | ClientMessage::BrowserSnapshot { .. }
                | ClientMessage::BrowserScreenshot { .. }
                | ClientMessage::BrowserClick { .. }
                | ClientMessage::BrowserFill { .. }
                | ClientMessage::BrowserPress { .. }
                | ClientMessage::BrowserClose { .. }
                | ClientMessage::TerminalOpen { .. }
                | ClientMessage::TerminalInput { .. }
                | ClientMessage::TerminalResize { .. }
                | ClientMessage::TerminalClose { .. }
        );
    }

    let observe = matches!(
        message,
        ClientMessage::RemoteHello { .. }
            | ClientMessage::AttachSession { .. }
            | ClientMessage::LoadSessionHistory { .. }
            | ClientMessage::ListSessions
            | ClientMessage::ListProjects
            | ClientMessage::ListModels
            | ClientMessage::Ping
            | ClientMessage::ListDir { .. }
            | ClientMessage::ListFilesystemDir { .. }
            | ClientMessage::ReadFilePreview { .. }
            | ClientMessage::GitStatus { .. }
            | ClientMessage::ReadSessionDiff { .. }
            | ClientMessage::WatchWorkspace { .. }
    );
    if observe {
        return true;
    }

    scope == RemoteScope::Collaborate
        && matches!(
            message,
            ClientMessage::Prompt { .. }
                | ClientMessage::Cancel
                | ClientMessage::PermissionResponse { .. }
                | ClientMessage::AskUserResponse { .. }
                | ClientMessage::YieldControl { .. }
        )
}

/// Select the newest messages before `before` without constructing a wire
/// frame large enough to trip the WebSocket frame limit. The returned cursor
/// is the index to pass for the next request.
fn history_page(
    messages: &[mew_message::Message],
    before: usize,
    limit: usize,
) -> (Vec<mew_message::Message>, Option<usize>) {
    let end = before.min(messages.len());
    let limit = limit.max(1);
    let mut start = end;
    let mut bytes = 0usize;
    while start > 0 && end - start < limit {
        let message = &messages[start - 1];
        let message_bytes = serde_json::to_vec(message).map_or(0, |encoded| encoded.len());
        if start < end && bytes.saturating_add(message_bytes) > HISTORY_PAGE_MAX_BYTES {
            break;
        }
        bytes = bytes.saturating_add(message_bytes);
        start -= 1;
    }
    (messages[start..end].to_vec(), (start > 0).then_some(start))
}

async fn detach_connection_client(
    session_manager: &Arc<SessionManager>,
    session: Arc<Session>,
    client_id: u64,
) {
    session
        .broadcast(ServerMessage::ClientDetached { client_id })
        .await;
    if !session.detach_client(client_id).await {
        return;
    }

    let session = Arc::downgrade(&session);
    let manager = session_manager.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let Some(session) = session.upgrade() else {
                break;
            };
            if manager.unload_detached(&session).await {
                break;
            }
        }
    });
}

/// Poll a session workspace and notify its clients when filesystem entries
/// change. This intentionally keeps the watcher dependency-free: daemon
/// sessions can live on network filesystems where native watcher backends are
/// not consistently available.
async fn watch_workspace(session: Arc<Session>, root: PathBuf, cancel: CancellationToken) {
    let mut previous = tokio::task::spawn_blocking({
        let root = root.clone();
        move || workspace_snapshot(&root)
    })
    .await
    .unwrap_or_default();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {
                let next = tokio::task::spawn_blocking({
                    let root = root.clone();
                    move || workspace_snapshot(&root)
                })
                .await
                .unwrap_or_default();
                let changed = changed_workspace_paths(&previous, &next);
                previous = next;
                if !changed.is_empty() {
                    session.broadcast(ServerMessage::FsChanged { session_id: session.id.clone(), paths: changed }).await;
                }
            }
        }
    }
}

struct ReviewResponseBudget(usize);

impl std::io::Write for ReviewResponseBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::Error::other("review response exceeds size limit"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(PartialEq, Eq)]
enum WorkspaceStamp {
    File(u64, u32, u64),
    Head(Vec<u8>),
}

fn workspace_file_stamp(metadata: &std::fs::Metadata) -> WorkspaceStamp {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .unwrap_or_default();
    WorkspaceStamp::File(modified.as_secs(), modified.subsec_nanos(), metadata.len())
}

fn workspace_snapshot(root: &std::path::Path) -> std::collections::HashMap<String, WorkspaceStamp> {
    let mut snapshot = std::collections::HashMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if matches!(name.as_str(), ".git" | "target" | "node_modules" | ".venv") {
                    continue;
                }
                pending.push(path);
                continue;
            }
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            snapshot.insert(relative, workspace_file_stamp(&metadata));
        }
    }
    // Git resolves symbolic/packed refs and linked worktrees without walking
    // repository objects. A commit can change review's base without rewriting
    // any workspace file; index-only operations also need to invalidate review.
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(root)
        .output()
    {
        if output.status.success() {
            snapshot.insert(".git/HEAD".into(), WorkspaceStamp::Head(output.stdout));
        }
    }
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-path", "index"])
        .current_dir(root)
        .output()
    {
        if output.status.success() {
            if let Ok(path) = std::str::from_utf8(&output.stdout) {
                if let Ok(metadata) = std::fs::metadata(path.strip_suffix('\n').unwrap_or(path)) {
                    snapshot.insert(".git/index".into(), workspace_file_stamp(&metadata));
                }
            }
        }
    }
    snapshot
}

fn changed_workspace_paths(
    previous: &std::collections::HashMap<String, WorkspaceStamp>,
    next: &std::collections::HashMap<String, WorkspaceStamp>,
) -> Vec<String> {
    let mut changed = previous
        .keys()
        .chain(next.keys())
        .filter(|path| previous.get(*path) != next.get(*path))
        .cloned()
        .collect::<Vec<_>>();
    changed.sort();
    changed.dedup();
    changed
}

pub async fn handle_connection<S>(
    stream: S,
    session_manager: Arc<SessionManager>,
    groups_store: Arc<groups::GroupsStore>,
    thinking_setter: Option<ThinkingSetter>,
    auto_summary_enabled: Arc<std::sync::atomic::AtomicBool>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    handle_connection_with_scope(
        stream,
        session_manager,
        groups_store,
        thinking_setter,
        auto_summary_enabled,
        None,
        None,
        false,
    )
    .await
}

/// Handle a connection with an optional daemon-enforced remote scope.
///
/// The local transports pass `None`. Iroh remote transports must pass a scope
/// so a connected peer cannot turn a transport-level identity check into full
/// daemon authority by sending handcrafted protocol messages.
#[allow(clippy::too_many_arguments)]
pub async fn handle_connection_with_scope<S>(
    stream: S,
    session_manager: Arc<SessionManager>,
    groups_store: Arc<groups::GroupsStore>,
    thinking_setter: Option<ThinkingSetter>,
    auto_summary_enabled: Arc<std::sync::atomic::AtomicBool>,
    remote_scope: Option<RemoteScope>,
    remote_token: Option<&str>,
    legacy_iroh: bool,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let authorizer = match (remote_scope, remote_token) {
        (Some(scope), Some(expected)) => {
            let expected = expected.to_owned();
            Some(Arc::new(move |token: Option<&str>, _device_name: &str| {
                Ok((token == Some(expected.as_str())).then_some(scope))
            }) as RemoteAuthorizer)
        }
        _ => None,
    };
    handle_connection_with_authorizer(
        stream,
        session_manager,
        groups_store,
        thinking_setter,
        auto_summary_enabled,
        remote_scope,
        authorizer,
        legacy_iroh,
    )
    .await
}

/// Resolve a remote connection's scope from its first protocol message.
pub type RemoteAuthorizer =
    Arc<dyn Fn(Option<&str>, &str) -> Result<Option<RemoteScope>> + Send + Sync>;

#[allow(clippy::too_many_arguments)]
pub async fn handle_connection_with_authorizer<S>(
    stream: S,
    session_manager: Arc<SessionManager>,
    groups_store: Arc<groups::GroupsStore>,
    thinking_setter: Option<ThinkingSetter>,
    auto_summary_enabled: Arc<std::sync::atomic::AtomicBool>,
    mut remote_scope: Option<RemoteScope>,
    remote_authorizer: Option<RemoteAuthorizer>,
    legacy_iroh: bool,
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
    let browser_owner = ulid::Ulid::new().to_string();
    let connection_lifetime = CancellationToken::new();
    let mut confirmed_desktop_sessions = std::collections::HashSet::new();
    let browser_pages = Arc::new(std::sync::Mutex::new(
        HashMap::<String, BrowserRegistration>::new(),
    ));
    let mut client_id: Option<u64> = None;
    let mut terminal: Option<terminal::TerminalHandle> = None;
    let mut workspace_watch: Option<CancellationToken> = None;
    let mut auto_title_enabled = true;
    let mut remote_ready = remote_scope.is_none() && remote_authorizer.is_none();

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

        if remote_scope.is_some() || remote_authorizer.is_some() || legacy_iroh {
            if let ClientMessage::RegisterBrowserPage {
                session_id,
                page_instance,
                ..
            }
            | ClientMessage::UnregisterBrowserPage {
                session_id,
                page_instance,
            } = &client_msg
            {
                reply(ServerMessage::BrowserPageRegistration {
                    session_id: session_id.clone(),
                    page_instance: page_instance.clone(),
                    error: Some("remote connections cannot register a local browser page".into()),
                });
                continue;
            }
        }
        if remote_scope.is_some() || remote_authorizer.is_some() {
            if !remote_ready {
                match &client_msg {
                    ClientMessage::RemoteHello { token, device_name }
                        if !device_name.trim().is_empty() =>
                    {
                        let authorized_scope = if let Some(authorizer) = &remote_authorizer {
                            authorizer(token.as_deref(), device_name)?
                        } else {
                            remote_scope
                        };
                        if let Some(authorized_scope) = authorized_scope {
                            remote_scope = Some(authorized_scope);
                            remote_ready = true;
                            reply(ServerMessage::RemoteReady {
                                scope: authorized_scope,
                            });
                        } else {
                            reply(ServerMessage::Error {
                                message: "remote pairing credentials were rejected".into(),
                            });
                        }
                        continue;
                    }
                    ClientMessage::RemoteHello { .. } => {
                        reply(ServerMessage::Error {
                            message: "remote pairing credentials were rejected".into(),
                        });
                        continue;
                    }
                    _ => {
                        reply(ServerMessage::Error {
                            message: "remote connection must authenticate before use".into(),
                        });
                        continue;
                    }
                }
            }
            let Some(scope) = remote_scope else {
                reply(ServerMessage::Error {
                    message: "remote connection has no granted scope".into(),
                });
                continue;
            };
            if !remote_message_allowed(scope, &client_msg) {
                reply(ServerMessage::Error {
                    message: format!(
                        "remote access scope `{}` does not allow this operation",
                        remote_scope_label(scope)
                    ),
                });
                continue;
            }
            if matches!(
                &client_msg,
                ClientMessage::NewSession { client_kind, .. }
                | ClientMessage::NewSessionInGroup { client_kind, .. }
                | ClientMessage::AttachSession { client_kind, .. }
                    if *client_kind != ClientKind::Remote
            ) {
                reply(ServerMessage::Error {
                    message: "remote connections must identify as client_kind=remote".into(),
                });
                continue;
            }
        } else if legacy_iroh
            && matches!(
                &client_msg,
                ClientMessage::NewSession {
                    client_kind: ClientKind::Desktop,
                    ..
                } | ClientMessage::NewSessionInGroup {
                    client_kind: ClientKind::Desktop,
                    ..
                } | ClientMessage::AttachSession {
                    client_kind: ClientKind::Desktop,
                    ..
                }
            )
        {
            reply(ServerMessage::Error {
                message: "legacy iroh connections cannot use desktop-only capabilities".into(),
            });
            continue;
        }

        let requested_group_id = match &client_msg {
            ClientMessage::NewSessionInGroup { group_id, .. } => Some(group_id.clone()),
            _ => None,
        };

        match client_msg {
            ClientMessage::RemoteHello { .. } if legacy_iroh => {
                reply(ServerMessage::RemoteReady {
                    scope: RemoteScope::Control,
                });
            }
            ClientMessage::RemoteHello { .. } => {
                reply(ServerMessage::Error {
                    message: "remote authentication is only available over iroh".into(),
                });
            }
            ClientMessage::NewSession { cwd, client_kind }
            | ClientMessage::NewSessionInGroup {
                cwd, client_kind, ..
            } => {
                // A web client may not know the host's filesystem cwd. Keep
                // the session useful for workspace tools by binding it to the
                // daemon's launch directory in that case.
                let cwd = cwd
                    .map(PathBuf::from)
                    .or_else(|| std::env::current_dir().ok());
                // Validate cwd if provided: must exist and be a directory.
                if let Some(ref cwd_str) = cwd {
                    if !cwd_str.exists() {
                        reply(ServerMessage::Error {
                            message: format!("session cwd does not exist: {}", cwd_str.display()),
                        });
                        continue;
                    }
                    if !cwd_str.is_dir() {
                        reply(ServerMessage::Error {
                            message: format!(
                                "session cwd is not a directory: {}",
                                cwd_str.display()
                            ),
                        });
                        continue;
                    }
                }
                if let Some(group_id) = requested_group_id.as_deref() {
                    if !groups_store.contains(group_id).await {
                        reply(ServerMessage::Error {
                            message: "session group not found".into(),
                        });
                        continue;
                    }
                }
                match session_manager.create(cwd.clone()).await {
                    Ok(session) => {
                        if let Some(group_id) = requested_group_id.as_deref() {
                            let dir = session_manager.session_dir.clone();
                            let agent = session.agent.lock().await;
                            if let Some(writer) = &agent.session {
                                let _ = writer
                                    .lock()
                                    .await
                                    .set_group_id(&dir, Some(group_id.to_owned()))
                                    .await;
                            }
                            drop(agent);
                            if let Err(e) = groups_store
                                .assign_session(&session.id, Some(group_id.to_owned()))
                                .await
                            {
                                reply(ServerMessage::Error {
                                    message: format!("failed to assign session group: {e}"),
                                });
                                continue;
                            }
                        }
                        if let Some(watch) = workspace_watch.take() {
                            watch.cancel();
                        }
                        if let (Some(previous), Some(previous_id)) =
                            (attached_session.take(), client_id.take())
                        {
                            detach_connection_client(&session_manager, previous, previous_id).await;
                        }
                        if let Some(previous_terminal) = terminal.take() {
                            previous_terminal.close();
                        }
                        let actions = session.pending_action_lock.lock().await;
                        let (cid, was_first) =
                            session.attach_client(client_tx.clone(), client_kind).await;
                        client_id = Some(cid);
                        attached_session = Some(session.clone());
                        if client_kind == ClientKind::Desktop
                            && !legacy_iroh
                            && remote_scope.is_none()
                            && remote_authorizer.is_none()
                        {
                            confirmed_desktop_sessions.insert(session.id.clone());
                        }
                        let session_id = session.id.clone();
                        let (model, provider, permission_mode) = {
                            let agent = session.agent.lock().await;
                            (
                                session.model.lock().await.clone(),
                                session.provider.lock().await.clone(),
                                Some(agent.permission_mode().id().to_string()),
                            )
                        };
                        let thinking_variant = {
                            let agent = session.agent.lock().await;
                            agent.active_thinking_variant().map(str::to_owned)
                        };
                        // Persist cwd on the session meta.
                        if let Some(cwd_path) = &cwd {
                            let dir = session_manager.session_dir.clone();
                            let agent = session.agent.lock().await;
                            if let Some(writer) = &agent.session {
                                let _ = writer
                                    .lock()
                                    .await
                                    .set_cwd(&dir, cwd_path.to_string_lossy().into_owned())
                                    .await;
                            }
                        }
                        let running = session.is_running.lock().await;
                        reply(ServerMessage::SessionReady {
                            session_id: session_id.clone(),
                            cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
                            model,
                            provider,
                            permission_mode,
                        });
                        reply(ServerMessage::SessionActivityChanged {
                            session_id,
                            activity: if *running {
                                SessionState::Running
                            } else {
                                SessionState::Idle
                            },
                        });
                        drop(running);
                        // Report the new session's active thinking variant so
                        // the frontend renders the pill immediately.
                        reply(ServerMessage::ThinkingVariantChanged {
                            variant: thinking_variant,
                        });
                        drop(actions);
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
                        tracing::error!(error = %e, error_chain = %format!("{e:#}"), "failed to create session");
                        reply(ServerMessage::Error {
                            message: format!("failed to create session: {e:#}"),
                        });
                    }
                }
            }
            ClientMessage::AttachSession {
                session_id,
                client_kind,
            } => {
                if attached_session
                    .as_ref()
                    .is_some_and(|session| session.id == session_id)
                {
                    continue;
                }
                match session_manager.attach(&session_id).await {
                    Ok(session) => {
                        if let Some(watch) = workspace_watch.take() {
                            watch.cancel();
                        }
                        if let (Some(previous), Some(previous_id)) =
                            (attached_session.take(), client_id.take())
                        {
                            detach_connection_client(&session_manager, previous, previous_id).await;
                        }
                        if let Some(previous_terminal) = terminal.take() {
                            previous_terminal.close();
                        }
                        let actions = session.pending_action_lock.lock().await;
                        let (cid, was_first) =
                            session.attach_client(client_tx.clone(), client_kind).await;
                        let existing_clients = session.client_presence_except(cid).await;
                        client_id = Some(cid);
                        attached_session = Some(session.clone());
                        if client_kind == ClientKind::Desktop
                            && !legacy_iroh
                            && remote_scope.is_none()
                            && remote_authorizer.is_none()
                        {
                            confirmed_desktop_sessions.insert(session.id.clone());
                        }

                        let (model, provider, permission_mode) = {
                            let agent = session.agent.lock().await;
                            (
                                session.model.lock().await.clone(),
                                session.provider.lock().await.clone(),
                                Some(agent.permission_mode().id().to_string()),
                            )
                        };
                        let thinking_variant = {
                            let agent = session.agent.lock().await;
                            agent.active_thinking_variant().map(str::to_owned)
                        };
                        let cwd = session_manager
                            .session_cwd(&session_id)
                            .await
                            .map(|path| path.to_string_lossy().into_owned());
                        let running = session.is_running.lock().await;
                        reply(ServerMessage::SessionReady {
                            session_id: session_id.clone(),
                            cwd,
                            model,
                            provider,
                            permission_mode,
                        });
                        reply(ServerMessage::SessionActivityChanged {
                            session_id: session_id.clone(),
                            activity: if *running {
                                SessionState::Running
                            } else {
                                SessionState::Idle
                            },
                        });
                        drop(running);
                        // Report the session's current thinking variant so the
                        // frontend renders the pill immediately on attach instead
                        // of showing "no thinking" until the user toggles it.
                        reply(ServerMessage::ThinkingVariantChanged {
                            variant: thinking_variant,
                        });

                        // The TUI receives the whole transcript in bounded frames.
                        // Other clients request older pages through the cursor.
                        match session.history().await {
                            Ok(messages) => {
                                let mut before = messages.len();
                                let mut replace = true;
                                loop {
                                    let (page, next_cursor) = history_page(
                                        &messages,
                                        before,
                                        mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT,
                                    );
                                    reply(ServerMessage::SessionHistoryPage {
                                        session_id: session_id.clone(),
                                        messages: page,
                                        next_cursor,
                                        replace,
                                    });
                                    match next_cursor {
                                        Some(cursor) if client_kind == ClientKind::Tui => {
                                            before = cursor;
                                            replace = false;
                                        }
                                        _ => break,
                                    }
                                }
                            }
                            Err(error) => {
                                reply(ServerMessage::Error {
                                    message: format!("failed to load session history: {error:#}"),
                                });
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

                        let mut permissions = session.pending_permissions.lock().await;
                        let mut questions = session.pending_ask_user.lock().await;
                        let mut goals = session.pending_goal_proposals.lock().await;
                        permissions.retain(|_, request| !request.sender.is_closed());
                        questions.retain(|_, request| !request.sender.is_closed());
                        goals.retain(|_, request| !request.sender.is_closed());
                        for message in permissions
                            .values()
                            .map(|request| &request.message)
                            .chain(questions.values().map(|request| &request.message))
                            .chain(goals.values().map(|request| &request.message))
                        {
                            reply(message.clone());
                        }
                        reply(ServerMessage::SessionAttentionChanged {
                            session_id: session.id.clone(),
                            pending_permissions: permissions.len() as u32,
                            pending_questions: (questions.len() + goals.len()) as u32,
                        });
                        drop(goals);
                        drop(questions);
                        drop(permissions);

                        for (existing_id, existing_kind) in existing_clients {
                            reply(ServerMessage::ClientAttached {
                                client_id: existing_id,
                                client_kind: existing_kind,
                            });
                        }

                        drop(actions);
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
                        tracing::error!(error = %e, error_chain = %format!("{e:#}"), "failed to resume session");
                        reply(ServerMessage::Error {
                            message: format!("failed to resume session: {e:#}"),
                        });
                    }
                }
            }
            ClientMessage::LoadSessionHistory {
                session_id,
                before,
                limit,
            } => {
                let Some(session) = attached_session.as_ref() else {
                    reply(ServerMessage::Error {
                        message: "no session attached".into(),
                    });
                    continue;
                };
                if session.id != session_id {
                    reply(ServerMessage::Error {
                        message: "history request does not match the attached session".into(),
                    });
                    continue;
                }
                let messages = match session.history().await {
                    Ok(messages) => messages,
                    Err(error) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to load session history: {error:#}"),
                        });
                        continue;
                    }
                };
                let (messages, next_cursor) = history_page(
                    &messages,
                    before,
                    limit.min(mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT),
                );
                reply(ServerMessage::SessionHistoryPage {
                    session_id,
                    messages,
                    next_cursor,
                    replace: false,
                });
            }
            ClientMessage::ListSessions => {
                let sessions = session_manager.list().await;
                reply(ServerMessage::SessionList { sessions });
                let groups = groups_store.list().await;
                reply(ServerMessage::GroupList { groups });
            }
            ClientMessage::DeleteSession { session_id } => {
                confirmed_desktop_sessions.remove(&session_id);
                browser_pages
                    .lock()
                    .expect("browser registrations lock")
                    .remove(&session_id);
                // Remove from active sessions if present.
                session_manager.remove(&session_id).await;
                // Delete from disk.
                let dir = session_manager.session_dir.join(&session_id);
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
                if attached_session
                    .as_ref()
                    .is_some_and(|session| session.id == session_id)
                {
                    if let Some(watch) = workspace_watch.take() {
                        watch.cancel();
                    }
                    if let (Some(session), Some(cid)) = (attached_session.take(), client_id.take())
                    {
                        detach_connection_client(&session_manager, session, cid).await;
                    }
                    if let Some(previous_terminal) = terminal.take() {
                        previous_terminal.close();
                    }
                }
            }
            ClientMessage::RenameSession { session_id, title } => {
                // Persist to disk for both active and idle sessions.
                let dir = session_manager.session_dir.clone();
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    let _ = meta.set_custom_title(&dir, title.clone()).await;
                }
                // Broadcast to all clients.
                session_manager.broadcast_title(&session_id, title).await;
            }
            ClientMessage::RegenerateTitle { session_id } => {
                // Attach (loads from disk if idle) so we can access the
                // full message history for title generation.
                match session_manager.attach(&session_id).await {
                    Ok(session) => {
                        let agent = session.agent.lock().await.clone();
                        // Extract the first user message text.
                        let first_user_text = {
                            let msgs = agent.messages.lock().await;
                            msgs.iter()
                                .find(|m| m.role == mew_message::Role::User)
                                .and_then(|m| {
                                    m.parts.iter().find_map(|p| match p {
                                        mew_message::Part::Text(tp)
                                            if !tp.synthetic && !tp.text.trim().is_empty() =>
                                        {
                                            Some(tp.text.clone())
                                        }
                                        _ => None,
                                    })
                                })
                        };
                        let Some(prompt_text) = first_user_text else {
                            reply(ServerMessage::Error {
                                message: "session has no user messages to generate a title from"
                                    .into(),
                            });
                            continue;
                        };
                        // Generate title via LLM (falls back to text truncation).
                        let title = generate_session_title(&agent, &prompt_text).await;
                        // Persist the title to disk.
                        let dir = session.session_dir.clone();
                        if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await
                        {
                            let _ = meta.set_custom_title(&dir, title.clone()).await;
                        }
                        // Broadcast to all clients.
                        session_manager.broadcast_title(&session_id, title).await;
                    }
                    Err(_) => {
                        reply(ServerMessage::Error {
                            message: "session not found".into(),
                        });
                    }
                }
            }
            ClientMessage::SetAutoTitle { enabled } => {
                auto_title_enabled = enabled;
                tracing::info!(auto_title = enabled, "auto-title setting changed");
            }
            ClientMessage::SetAutoSummary { enabled } => {
                auto_summary_enabled.store(enabled, std::sync::atomic::Ordering::Relaxed);
                tracing::info!(auto_summary = enabled, "auto-summary setting changed");
            }
            ClientMessage::Prompt { text, attachments } => {
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
                    let had_error =
                        run_turn(&session, &session_mgr, &agent, text, attachments).await;

                    // Generate a session title from the first user message
                    // if we haven't already. Uses a lightweight LLM call;
                    // falls back to text truncation on error. Skipped if the
                    // user has disabled auto-title generation.
                    if auto_title && !had_error {
                        let has_existing_title = agent
                            .session_meta()
                            .await
                            .and_then(|meta| meta.custom_title)
                            .is_some();
                        let mut generated = session.title_generated.lock().await;
                        if !*generated && !has_existing_title {
                            *generated = true;
                            drop(generated);

                            let title = generate_session_title(&agent, &prompt_text).await;
                            if let Ok(Some(mut meta)) =
                                mew_session::Meta::read(&session.session_dir, &session.id).await
                            {
                                let _ = meta
                                    .set_custom_title(&session.session_dir, title.clone())
                                    .await;
                            }
                            session_mgr.broadcast_title(&session.id, title).await;
                        }
                    }
                });
            }
            ClientMessage::Cancel => {
                if let Some(session) = &attached_session {
                    session.cancel_turn().await;
                    let _actions = session.pending_action_lock.lock().await;
                    broadcast_attention(session, &session_manager).await;
                }
            }
            ClientMessage::Guide { text } => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::Error {
                        message: "no session — send NewSession or AttachSession first".into(),
                    });
                    continue;
                };
                if text.trim().is_empty() {
                    continue;
                }
                // Queue guidance on the session's agent. The running turn's
                // clone shares `pending_guidance`, so it'll be injected into
                // the next provider request; if no turn is running, it's
                // picked up by the next turn.
                let agent = session.agent.lock().await;
                agent.enqueue_guidance(text.clone()).await;
                drop(agent);
                // Broadcast so all attached clients render the guidance.
                session.broadcast(ServerMessage::UserMessage { text }).await;
            }
            ClientMessage::TerminalOpen { rows, cols } => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::TerminalError {
                        terminal_id: None,
                        message: "no session — send NewSession or AttachSession first".into(),
                    });
                    continue;
                };
                if let Some(existing) = terminal.take() {
                    existing.close();
                }
                let cwd = session_manager
                    .session_cwd(&session.id)
                    .await
                    .or_else(|| std::env::current_dir().ok());
                let Some(cwd) = cwd else {
                    reply(ServerMessage::TerminalError {
                        terminal_id: None,
                        message: "could not determine terminal working directory".into(),
                    });
                    continue;
                };
                match terminal::spawn(&cwd, rows, cols) {
                    Ok((handle, events)) => {
                        let terminal_id = handle.id.clone();
                        let (terminal_tx, mut terminal_rx) = mpsc::channel(256);
                        let websocket_tx = client_tx.clone();
                        tokio::spawn(async move {
                            while let Some(message) = terminal_rx.recv().await {
                                if websocket_tx.send(message).is_err() {
                                    break;
                                }
                            }
                        });
                        let output_tx = terminal_tx.clone();
                        let event_terminal_id = terminal_id.clone();
                        tokio::task::spawn_blocking(move || {
                            while let Ok(event) = events.recv() {
                                let message = match event {
                                    terminal::TerminalEvent::Output(bytes) => {
                                        ServerMessage::TerminalOutput {
                                            terminal_id: event_terminal_id.clone(),
                                            bytes,
                                        }
                                    }
                                    terminal::TerminalEvent::Exited(status) => {
                                        ServerMessage::TerminalExited {
                                            terminal_id: event_terminal_id.clone(),
                                            status,
                                        }
                                    }
                                    terminal::TerminalEvent::Failed(message) => {
                                        ServerMessage::TerminalError {
                                            terminal_id: Some(event_terminal_id.clone()),
                                            message,
                                        }
                                    }
                                };
                                if output_tx.blocking_send(message).is_err() {
                                    break;
                                }
                            }
                        });
                        terminal = Some(handle);
                        reply(ServerMessage::TerminalOpened { terminal_id });
                    }
                    Err(error) => reply(ServerMessage::TerminalError {
                        terminal_id: None,
                        message: format!("could not open terminal: {error}"),
                    }),
                }
            }
            ClientMessage::TerminalInput { terminal_id, bytes } => {
                match terminal
                    .as_ref()
                    .filter(|terminal| terminal.id == terminal_id)
                {
                    Some(terminal) => {
                        if let Err(error) = terminal.send_input(bytes) {
                            reply(ServerMessage::TerminalError {
                                terminal_id: Some(terminal_id),
                                message: error.to_string(),
                            });
                        }
                    }
                    None => reply(ServerMessage::TerminalError {
                        terminal_id: Some(terminal_id),
                        message: "terminal is not open".into(),
                    }),
                }
            }
            ClientMessage::TerminalResize {
                terminal_id,
                rows,
                cols,
            } => {
                match terminal
                    .as_ref()
                    .filter(|terminal| terminal.id == terminal_id)
                {
                    Some(terminal) => {
                        if let Err(error) = terminal.resize(rows, cols) {
                            reply(ServerMessage::TerminalError {
                                terminal_id: Some(terminal_id),
                                message: error.to_string(),
                            });
                        }
                    }
                    None => reply(ServerMessage::TerminalError {
                        terminal_id: Some(terminal_id),
                        message: "terminal is not open".into(),
                    }),
                }
            }
            ClientMessage::TerminalClose { terminal_id } => {
                if terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.id == terminal_id)
                {
                    if let Some(terminal) = terminal.take() {
                        terminal.close();
                    }
                }
            }
            ClientMessage::Ping => {
                reply(ServerMessage::Pong {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    protocol_version: mew_protocol::PROTOCOL_VERSION,
                });
            }
            ClientMessage::ListProjects => {
                let projects = list_projects(&session_manager).await;
                reply(ServerMessage::ProjectList { projects });
            }
            ClientMessage::PinProject { path, pinned } => {
                let known = list_projects(&session_manager)
                    .await
                    .into_iter()
                    .any(|project| project.path == path);
                if !known {
                    reply(ServerMessage::Error {
                        message: format!("unknown workspace: {path}"),
                    });
                    continue;
                }
                if let Err(error) = session_manager.project_pins.set_pinned(path, pinned).await {
                    reply(ServerMessage::Error {
                        message: format!("failed to persist workspace pin: {error}"),
                    });
                    continue;
                }
                let projects = list_projects(&session_manager).await;
                session_manager
                    .broadcast_all(ServerMessage::ProjectList { projects })
                    .await;
            }
            ClientMessage::ListFilesystemDir { path } => {
                match crate::files::handle_list_filesystem_dir(path).await {
                    Ok(listing) => reply(listing),
                    Err(e) => reply(ServerMessage::Error {
                        message: format!("list_filesystem_dir: {e}"),
                    }),
                }
            }
            ClientMessage::PermissionResponse {
                request_id,
                decision,
            } => {
                if let Some(session) = &attached_session {
                    let _actions = session.pending_action_lock.lock().await;
                    let request = session.pending_permissions.lock().await.remove(&request_id);
                    if let Some(request) = request {
                        let _ = request.sender.send(decision.into());
                        session
                            .broadcast(ServerMessage::RequestResolved { request_id })
                            .await;
                        broadcast_attention(session, &session_manager).await;
                    }
                }
            }
            ClientMessage::AskUserResponse {
                request_id,
                answers,
            } => {
                if let Some(session) = &attached_session {
                    let _actions = session.pending_action_lock.lock().await;
                    let request = session.pending_ask_user.lock().await.remove(&request_id);
                    if let Some(request) = request {
                        let _ = request.sender.send(answers);
                        session
                            .broadcast(ServerMessage::RequestResolved { request_id })
                            .await;
                        broadcast_attention(session, &session_manager).await;
                    }
                }
            }
            ClientMessage::GoalResponse {
                request_id,
                accepted,
            } => {
                if let Some(session) = &attached_session {
                    let _actions = session.pending_action_lock.lock().await;
                    let request = session
                        .pending_goal_proposals
                        .lock()
                        .await
                        .remove(&request_id);
                    if let Some(request) = request {
                        let decision = if accepted {
                            GoalDecision::Accepted
                        } else {
                            GoalDecision::Rejected
                        };
                        let _ = request.sender.send(decision);
                        session
                            .broadcast(ServerMessage::RequestResolved { request_id })
                            .await;
                        broadcast_attention(session, &session_manager).await;
                    }
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
                            "/goal" => {
                                use mew_agent::{GoalState, GoalStatus};
                                let sub = command.strip_prefix("/goal").unwrap_or("").trim();
                                let mut goal_guard = agent.goal.lock().await;
                                match sub {
                                    "" | "status" => match &*goal_guard {
                                        Some(g) => Some(format!(
                                            "goal: {}\nstatus: {:?}\ncontinuations: {}",
                                            g.objective, g.status, g.continuation_count
                                        )),
                                        None => Some("no active goal".to_string()),
                                    },
                                    "pause" => {
                                        if let Some(g) = goal_guard.as_mut() {
                                            g.status = GoalStatus::Paused;
                                            Some(format!("goal paused: {}", g.objective))
                                        } else {
                                            Some("no active goal to pause".to_string())
                                        }
                                    }
                                    "resume" => {
                                        if let Some(g) = goal_guard.as_mut() {
                                            g.status = GoalStatus::Active;
                                            Some(format!("goal resumed: {}", g.objective))
                                        } else {
                                            Some("no goal to resume".to_string())
                                        }
                                    }
                                    "clear" => {
                                        if goal_guard.take().is_some() {
                                            Some("goal cleared".to_string())
                                        } else {
                                            Some("no goal to clear".to_string())
                                        }
                                    }
                                    "complete" => {
                                        if let Some(g) = goal_guard.as_mut() {
                                            g.status = GoalStatus::Complete;
                                            Some(format!("goal marked complete: {}", g.objective))
                                        } else {
                                            Some("no active goal to complete".to_string())
                                        }
                                    }
                                    other => {
                                        let now = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_millis() as i64)
                                            .unwrap_or(0);
                                        *goal_guard = Some(GoalState {
                                            objective: other.to_string(),
                                            status: GoalStatus::Active,
                                            continuation_count: 0,
                                            started_at: now,
                                        });
                                        Some(format!(
                                            "goal set: {other}\nthe agent will continue working until the goal is complete."
                                        ))
                                    }
                                }
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
                                    vec![],
                                )
                                .await;
                                if had_error {
                                    Some("wiki generation failed".to_string())
                                } else {
                                    Some("wiki generated at .mew/wiki.md".to_string())
                                }
                            }
                            _ => {
                                // Unknown command — check the registry.
                                if mew_protocol::is_known(cmd) {
                                    // Known command but not handled by daemon —
                                    // it's a client-side command. Silently ignore.
                                    None
                                } else {
                                    Some(format!("unknown command {}", cmd))
                                }
                            }
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
                            change_session_model(
                                &session, &mut agent, switcher, &provider, &model, None,
                            )
                            .await
                        }
                        None => Err(anyhow::anyhow!("model switching not available")),
                    };
                    match result {
                        Ok((new_provider, new_model, variant)) => {
                            info!(provider = %new_provider, model = %new_model, "model switched");
                            session
                                .broadcast(ServerMessage::ModelSwitched {
                                    provider: new_provider,
                                    model: new_model,
                                })
                                .await;
                            // Keep the frontend's thinking pill in sync with
                            // the switched-to model's default.
                            session
                                .broadcast(ServerMessage::ThinkingVariantChanged { variant })
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
                            let previous_reasoning = agent.reasoning.clone();
                            let previous_variant = agent.active_thinking_variant.clone();
                            let result = match setter(&mut agent, &model, &variant) {
                                Ok(resolved) => {
                                    agent.set_active_thinking_variant(resolved.clone());
                                    session
                                        .save_policy(&session.policy(&agent).await)
                                        .await
                                        .map(|()| resolved)
                                }
                                Err(error) => Err(error),
                            };
                            if result.is_err() {
                                agent.set_reasoning(previous_reasoning);
                                agent.set_active_thinking_variant(previous_variant);
                            }
                            result
                        }
                        None => Err(anyhow::anyhow!("thinking variant switching not available")),
                    };
                    match result {
                        Ok(resolved) => {
                            session
                                .broadcast(ServerMessage::ThinkingVariantChanged {
                                    variant: resolved,
                                })
                                .await;
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
                    // PermissionEngine stores the mode atomically, so this
                    // setting must not wait behind an active agent turn.
                    let parsed = mew_hooks::PermissionMode::from_id(&mode);
                    match parsed {
                        Some(m) => {
                            let agent = session.agent.lock().await;
                            let mut policy = session.policy(&agent).await;
                            policy.permission_mode = m.id().to_owned();
                            if let Err(error) = session.save_policy(&policy).await {
                                session
                                    .broadcast(ServerMessage::Error {
                                        message: format!("failed to set permission mode: {error}"),
                                    })
                                    .await;
                                return;
                            }
                            agent.set_permission_mode(m);
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
                        let mut ungrouped_sessions = Vec::new();
                        if let Ok(entries) = tokio::fs::read_dir(&dir).await {
                            let mut entries = entries;
                            while let Ok(Some(entry)) = entries.next_entry().await {
                                if let Some(id) = entry.file_name().to_str() {
                                    if let Ok(Some(mut meta)) =
                                        mew_session::Meta::read(&dir, id).await
                                    {
                                        if meta.group_id.as_deref() == Some(&group_id) {
                                            let _ = meta.set_group_id(&dir, None).await;
                                            ungrouped_sessions.push((
                                                id.to_owned(),
                                                meta.archived,
                                                meta.pinned,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        session_manager.broadcast_groups(groups).await;
                        for (session_id, archived, pinned) in ungrouped_sessions {
                            session_manager
                                .broadcast_all(ServerMessage::SessionMetaChanged {
                                    session_id,
                                    archived,
                                    pinned,
                                    group_id: None,
                                })
                                .await;
                        }
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
                let mut meta_archived = false;
                let mut meta_pinned = false;
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    let _ = meta.set_group_id(&dir, group_id.clone()).await;
                    meta_group_id = meta.group_id.clone();
                    meta_archived = meta.archived;
                    meta_pinned = meta.pinned;
                }
                match groups_store.assign_session(&session_id, group_id).await {
                    Ok(groups) => {
                        session_manager.broadcast_groups(groups).await;
                        session_manager
                            .broadcast_all(ServerMessage::SessionMetaChanged {
                                session_id: session_id.clone(),
                                archived: meta_archived,
                                pinned: meta_pinned,
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
                let mut meta_group_id = None;
                let mut meta_pinned = false;
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    meta_group_id = meta.group_id.clone();
                    if meta.set_archived(&dir, archived).await.is_ok() {
                        meta_pinned = meta.pinned;
                    }
                }
                session_manager
                    .broadcast_all(ServerMessage::SessionMetaChanged {
                        session_id: session_id.clone(),
                        archived,
                        pinned: meta_pinned,
                        group_id: meta_group_id,
                    })
                    .await;
            }
            ClientMessage::PinSession { session_id, pinned } => {
                let dir = session_manager.session_dir.clone();
                let mut meta_group_id = None;
                let mut meta_archived = false;
                if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &session_id).await {
                    meta_group_id = meta.group_id.clone();
                    if meta.set_pinned(&dir, pinned).await.is_ok() {
                        meta_archived = meta.archived;
                    }
                }
                session_manager
                    .broadcast_all(ServerMessage::SessionMetaChanged {
                        session_id: session_id.clone(),
                        archived: meta_archived,
                        pinned,
                        group_id: meta_group_id,
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
            ClientMessage::ReadSessionDiff {
                session_id,
                request_id,
            } => {
                let Some(root) = session_manager.session_cwd(&session_id).await else {
                    reply(ServerMessage::SessionDiffError {
                        session_id,
                        request_id,
                        message: "session has no workspace".into(),
                    });
                    continue;
                };
                let client_tx = client_tx.clone();
                tokio::spawn(async move {
                    let result =
                        tokio::task::spawn_blocking(move || mew_diff::load_session_diffs(&root))
                            .await;
                    let message = match result {
                        Ok(Ok(files)) => ServerMessage::SessionDiff {
                            session_id,
                            request_id,
                            files,
                        },
                        Ok(Err(error)) => ServerMessage::SessionDiffError {
                            session_id,
                            request_id,
                            message: error.to_string(),
                        },
                        Err(error) => ServerMessage::SessionDiffError {
                            session_id,
                            request_id,
                            message: format!("could not load review: {error}"),
                        },
                    };
                    let message = match &message {
                        ServerMessage::SessionDiff { session_id, request_id, .. }
                            if serde_json::to_writer(&mut ReviewResponseBudget(HISTORY_PAGE_MAX_BYTES), &message).is_err() => {
                            ServerMessage::SessionDiffError {
                                session_id: session_id.clone(),
                                request_id: request_id.clone(),
                                message: "workspace changes exceed the review response size limit (8 MiB)".into(),
                            }
                        }
                        _ => message,
                    };
                    let _ = client_tx.send(message);
                });
            }
            ClientMessage::WatchWorkspace {
                session_id,
                enabled,
            } => {
                if let Some(watch) = workspace_watch.take() {
                    watch.cancel();
                }
                if enabled {
                    let Some(session) = attached_session
                        .as_ref()
                        .filter(|session| session.id == session_id)
                        .cloned()
                    else {
                        reply(ServerMessage::Error {
                            message: "watch_workspace requires the attached session".into(),
                        });
                        continue;
                    };
                    let Some(root) = session_manager.session_cwd(&session_id).await else {
                        reply(ServerMessage::Error {
                            message: "session has no workspace".into(),
                        });
                        continue;
                    };
                    let cancel = CancellationToken::new();
                    let task_cancel = cancel.clone();
                    tokio::spawn(watch_workspace(session, root, task_cancel));
                    workspace_watch = Some(cancel);
                }
            }

            // -- Browser --
            ClientMessage::RegisterBrowserPage {
                session_id,
                page_instance,
                cdp_endpoint,
                target_id,
            } => {
                if !confirmed_desktop_sessions.contains(&session_id)
                    || legacy_iroh
                    || remote_scope.is_some()
                    || remote_authorizer.is_some()
                {
                    reply(ServerMessage::BrowserPageRegistration {
                        session_id,
                        page_instance,
                        error: Some(
                            "browser registration requires a confirmed local desktop session"
                                .into(),
                        ),
                    });
                    continue;
                }
                let session = match session_manager.attach(&session_id).await {
                    Ok(session) => session,
                    Err(error) => {
                        confirmed_desktop_sessions.remove(&session_id);
                        browser_pages
                            .lock()
                            .expect("browser registrations lock")
                            .remove(&session_id);
                        reply(ServerMessage::BrowserPageRegistration {
                            session_id,
                            page_instance,
                            error: Some(format!("browser session is unavailable: {error}")),
                        });
                        continue;
                    }
                };
                let preparation = session.browser_access.register(
                    &browser_owner,
                    mew_tools::browser::BrowserTarget {
                        page_instance: page_instance.clone(),
                        cdp_endpoint,
                        target_id,
                    },
                );
                if session
                    .browser_access
                    .is_registered(&browser_owner, &page_instance)
                {
                    browser_pages
                        .lock()
                        .expect("browser registrations lock")
                        .insert(
                            session_id.clone(),
                            BrowserRegistration {
                                page_instance: page_instance.clone(),
                                session: session.clone(),
                            },
                        );
                }
                let lifetime = connection_lifetime.clone();
                let client_tx = client_tx.clone();
                let pages = browser_pages.clone();
                let access = session.browser_access.clone();
                let owner = browser_owner.clone();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        biased;
                        _ = lifetime.cancelled() => Err("desktop connection closed".to_owned()),
                        result = preparation => result,
                    };
                    if result.is_err() {
                        let mut pages = pages.lock().expect("browser registrations lock");
                        if pages
                            .get(&session_id)
                            .is_some_and(|registered| registered.page_instance == page_instance)
                            && !access.is_registered(&owner, &page_instance)
                        {
                            pages.remove(&session_id);
                        }
                    }
                    let _ = client_tx.send(ServerMessage::BrowserPageRegistration {
                        session_id,
                        page_instance,
                        error: result.err(),
                    });
                });
            }
            ClientMessage::UnregisterBrowserPage {
                session_id,
                page_instance,
            } => {
                let mut pages = browser_pages.lock().expect("browser registrations lock");
                let removed = pages
                    .get(&session_id)
                    .is_some_and(|registered| registered.page_instance == page_instance);
                if removed {
                    let registered = pages
                        .remove(&session_id)
                        .expect("matching browser registration");
                    registered
                        .session
                        .browser_access
                        .revoke(&browser_owner, &page_instance);
                }
                reply(ServerMessage::BrowserPageRegistration {
                    session_id,
                    page_instance,
                    error: (!removed)
                        .then(|| "browser page generation is no longer registered".into()),
                });
            }
            request @ (ClientMessage::BrowserOpen { .. }
            | ClientMessage::BrowserSnapshot { .. }
            | ClientMessage::BrowserScreenshot { .. }
            | ClientMessage::BrowserClick { .. }
            | ClientMessage::BrowserFill { .. }
            | ClientMessage::BrowserPress { .. }
            | ClientMessage::BrowserClose { .. }) => {
                let access = attached_session
                    .as_ref()
                    .map(|session| session.browser_access.clone())
                    .unwrap_or_default();
                let client_tx = client_tx.clone();
                let lifetime = connection_lifetime.clone();
                let response = crate::browser::respond(&access, request, &lifetime);
                tokio::spawn(async move {
                    let _ = client_tx.send(response.await);
                });
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

            // -- Personas --
            ClientMessage::ListPersonas => {
                let Some(session) = attached_session.clone() else {
                    reply(ServerMessage::Error {
                        message: "no session".into(),
                    });
                    continue;
                };
                let agent = session.agent.lock().await;
                let current = &agent.persona_name;
                let personas: Vec<mew_protocol::PersonaInfo> = agent
                    .personas
                    .iter()
                    .map(|p| mew_protocol::PersonaInfo {
                        name: p.name.clone(),
                        description: p.description.clone(),
                        color: None,
                        active: current.as_deref() == Some(&p.name),
                    })
                    .collect();
                reply(ServerMessage::PersonaList { personas });
            }
            ClientMessage::SwitchPersona { name } => {
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
                    let mut agent = session.agent.lock().await;
                    let persona = agent.personas.iter().find(|p| p.name == name).cloned();
                    if persona.is_none() && matches!(name.as_str(), "default" | "none") {
                        let mut policy = session.policy(&agent).await;
                        policy.persona = None;
                        if let Err(error) = session.save_policy(&policy).await {
                            let _ = client_tx.send(ServerMessage::Error {
                                message: format!("failed to clear persona: {error}"),
                            });
                            return;
                        }
                        agent.clear_persona();
                        let personas = agent
                            .personas
                            .iter()
                            .map(|persona| mew_protocol::PersonaInfo {
                                name: persona.name.clone(),
                                description: persona.description.clone(),
                                color: persona.config.color.clone(),
                                active: false,
                            })
                            .collect();
                        session
                            .broadcast(ServerMessage::PersonaList { personas })
                            .await;
                        return;
                    }
                    match persona {
                        Some(p) => {
                            let result = if let Some(pinned) = &p.config.model {
                                let inherited_provider =
                                    session.provider.lock().await.clone().unwrap_or_default();
                                let (provider, model) = pinned
                                    .split_once('/')
                                    .unwrap_or((&inherited_provider, pinned));
                                match &switcher {
                                    Some(switcher) if !provider.is_empty() && !model.is_empty() => {
                                        change_session_model(
                                            &session,
                                            &mut agent,
                                            switcher,
                                            provider,
                                            model,
                                            Some(&p.name),
                                        )
                                        .await
                                        .map(Some)
                                    }
                                    _ if provider.is_empty() || model.is_empty() => {
                                        Err(anyhow::anyhow!(
                                            "persona model must be provider/model: {pinned}"
                                        ))
                                    }
                                    _ => Err(anyhow::anyhow!("model switching not available")),
                                }
                            } else {
                                let mut policy = session.policy(&agent).await;
                                policy.persona = Some(p.name.clone());
                                session.save_policy(&policy).await.map(|()| None)
                            };
                            let switched = match result {
                                Ok(switched) => switched,
                                Err(error) => {
                                    let _ = client_tx.send(ServerMessage::Error {
                                        message: format!("failed to switch persona: {error}"),
                                    });
                                    return;
                                }
                            };
                            agent.apply_persona(&p);
                            if let Some((provider, model, variant)) = switched {
                                session
                                    .broadcast(ServerMessage::ModelSwitched { provider, model })
                                    .await;
                                session
                                    .broadcast(ServerMessage::ThinkingVariantChanged { variant })
                                    .await;
                            }
                            session
                                .broadcast(ServerMessage::PersonaSwitched { name: p.name })
                                .await;
                        }
                        None => {
                            let _ = client_tx.send(ServerMessage::Error {
                                message: format!("unknown persona: {name}"),
                            });
                        }
                    }
                });
            }
        }
    }
    connection_lifetime.cancel();
    for registered in browser_pages
        .lock()
        .expect("browser registrations lock")
        .drain()
        .map(|(_, registered)| registered)
    {
        registered.session.browser_access.disconnect(&browser_owner);
    }
    if let Some(watch) = workspace_watch.take() {
        watch.cancel();
    }
    if let (Some(session), Some(cid)) = (attached_session.take(), client_id.take()) {
        detach_connection_client(&session_manager, session, cid).await;
    }
    if let Some(terminal) = terminal {
        terminal.close();
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
    attachments: Vec<mew_protocol::Attachment>,
) -> bool {
    let token = CancellationToken::new();
    *session.current_turn_cancel.lock().await = Some(token.clone());
    *session.is_running.lock().await = true;
    session_mgr
        .broadcast_activity(&session.id, SessionState::Running)
        .await;

    // Convert wire attachments to message Parts.
    let parts: Vec<mew_message::Part> = attachments
        .iter()
        .map(|a| {
            let filename = a.path.rsplit('/').next().map(|s| s.to_string());
            mew_message::Part::File(mew_message::FilePart {
                base: mew_message::PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                mime: a.mime.clone().unwrap_or_default(),
                filename,
                url: a.path.clone(),
            })
        })
        .collect();

    let rx = agent.run_with_parts(prompt_text, parts, Some(token));
    let had_error = forward_events(rx, session.clone(), session_mgr.clone()).await;

    *session.current_turn_cancel.lock().await = None;
    *session.is_running.lock().await = false;

    // Update last_turn_failed + increment turn count. Meta is read from
    // disk: the MessageEnd intercepts persist usage and context occupancy
    // there, while the agent's writer keeps a stale in-memory copy.
    let dir = session_mgr.session_dir.clone();
    let (usage_wire, context_tokens) = match mew_session::Meta::read(&dir, &session.id).await {
        Ok(Some(mut meta)) => {
            let _ = meta.set_last_turn_failed(&dir, had_error).await;
            let context_tokens = meta.context_tokens;
            let wire = if let Some(u) = meta.usage.as_mut() {
                u.add_turn();
                let wire = mew_protocol::SessionUsageWire::from(&*u);
                let _ = meta.set_usage(&dir, wire.clone().into()).await;
                Some(wire)
            } else {
                None
            };
            (wire, context_tokens)
        }
        _ => (None, None),
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
                context_tokens,
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
        supports_vision: agent.supports_vision,
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

/// Check whether a daemon is already listening at the given socket path.
///
/// - If a live daemon responds → bail (prevent double-binding).
/// - If the socket file is stale (connection refused) → remove it and return Ok.
/// - If no socket file exists → return Ok.
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

/// Collect known projects from configured workspace roots and session metas.
/// Deduped by canonicalized path, sorted by recency (most recent first).
async fn list_projects(
    session_manager: &std::sync::Arc<crate::session::SessionManager>,
) -> Vec<mew_protocol::ProjectInfo> {
    use std::collections::HashMap;
    use std::path::PathBuf;

    let dir = session_manager.session_dir.clone();
    let mut projects: HashMap<PathBuf, mew_protocol::ProjectInfo> = HashMap::new();

    // Workspace roots are useful projects even before a session has been
    // created for them. Loading config here also keeps this query aligned with
    // the same layered config the agent builder uses.
    let workspace_roots = workspace_roots();
    for root in &workspace_roots {
        let canonical =
            crate::workspace::resolve_workspace_path(Some(root.to_string_lossy().as_ref()), &[])
                .map(PathBuf::from)
                .unwrap_or_else(|| root.clone());
        let display_name = canonical
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| canonical.to_string_lossy().to_string());
        projects
            .entry(canonical.clone())
            .or_insert_with(|| mew_protocol::ProjectInfo {
                path: canonical.to_string_lossy().to_string(),
                display_name,
                session_count: 0,
                last_used_at: None,
                pinned: false,
            });
    }

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
            let canonical =
                crate::workspace::resolve_workspace_path(Some(cwd_str), &workspace_roots)
                    .map(PathBuf::from)
                    .unwrap_or(path.clone());
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
                    pinned: false,
                });
            entry.session_count += 1;
            if let Some(ts) = last_used {
                entry.last_used_at = Some(entry.last_used_at.map(|e| e.max(ts)).unwrap_or(ts));
            }
        }
    }

    // Sort by last_used_at descending (None sorts last).
    let mut result: Vec<_> = projects.into_values().collect();
    for project in &mut result {
        project.pinned = session_manager.project_pins.is_pinned(&project.path).await;
    }
    result.sort_by(|a, b| {
        b.last_used_at
            .unwrap_or(0)
            .cmp(&a.last_used_at.unwrap_or(0))
    });
    result
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

        let sessions: Vec<(String, Arc<Session>)> = {
            let active = session_manager.active.lock().await;
            active
                .iter()
                .map(|(id, session)| (id.clone(), session.clone()))
                .collect()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut candidates = Vec::new();
        for (id, session) in sessions {
            let last = {
                let agent = session.agent.lock().await;
                let msgs = agent.messages.lock().await;
                msgs.last().map(|m| m.time.created).unwrap_or(0)
            };
            if now - last >= idle_threshold.as_millis() as i64 {
                candidates.push((id, session));
            }
        }

        for (id, session) in candidates {
            // Check if already has a summary.
            let dir = session.session_dir.clone();
            let has_summary = mew_session::Meta::read(&dir, &id)
                .await
                .ok()
                .flatten()
                .and_then(|m| m.summary)
                .is_some();
            if has_summary {
                continue;
            }
            if session
                .summary_running
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                continue;
            }

            let session_clone = session.clone();
            let id_clone = id.clone();
            let manager = session_manager.clone();
            tokio::spawn(async move {
                let agent = session_clone.agent.lock().await.clone();
                let summary = generate_session_summary(&agent).await;
                session_clone
                    .summary_running
                    .store(false, std::sync::atomic::Ordering::Release);
                if let Some(summary) = summary {
                    if let Ok(Some(mut meta)) = mew_session::Meta::read(&dir, &id_clone).await {
                        let _ = meta.set_summary(&dir, summary.clone()).await;
                    }
                    manager
                        .broadcast_all(mew_protocol::ServerMessage::SessionSummaryChanged {
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
        supports_vision: agent.supports_vision,
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

async fn change_session_model(
    session: &Session,
    agent: &mut Agent,
    switcher: &ModelSwitcher,
    provider: &str,
    model: &str,
    persona: Option<&str>,
) -> Result<(String, String, Option<String>)> {
    // Skill tools share these locks with the agent. A failed save must restore
    // their contents as well as the provider and other owned agent fields.
    let previous_template = agent.template_ctx.read().await.clone();
    let previous_filter = agent.skill_filter.read().await.clone();
    let previous = agent.clone();
    let result = async {
        let (provider, model) = switcher(agent, provider, model)?;
        let variant = agent.active_thinking_variant().map(str::to_owned);
        let mut policy = session.policy(agent).await;
        policy.provider = Some(provider.clone());
        policy.model = Some(model.clone());
        policy.thinking_variant = variant.clone();
        if let Some(persona) = persona {
            policy.persona = Some(persona.to_owned());
        }
        session.save_policy(&policy).await?;
        *session.provider.lock().await = Some(provider.clone());
        *session.model.lock().await = Some(model.clone());
        Ok((provider, model, variant))
    }
    .await;
    if result.is_err() {
        *agent = previous;
        *agent.template_ctx.write().await = previous_template;
        *agent.skill_filter.write().await = previous_filter;
    }
    result
}

async fn broadcast_attention(session: &Session, manager: &SessionManager) {
    let pending_permissions = session.pending_permissions.lock().await.len() as u32;
    let pending_questions = session.pending_questions_count().await;
    manager
        .broadcast_all(ServerMessage::SessionAttentionChanged {
            session_id: session.id.clone(),
            pending_permissions,
            pending_questions,
        })
        .await;
}

async fn publish_pending_request<T>(
    session: &Session,
    manager: &SessionManager,
    pending: &Mutex<HashMap<String, session::PendingRequest<T>>>,
    sender: oneshot::Sender<T>,
    message: ServerMessage,
) -> Vec<ServerMessage> {
    let _actions = session.pending_action_lock.lock().await;
    if sender.is_closed()
        || session
            .current_turn_cancel
            .lock()
            .await
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    {
        return Vec::new();
    }
    let (request_id, kind, detail) = match &message {
        ServerMessage::PermissionRequest {
            request_id,
            tool_name,
            ..
        }
        | ServerMessage::SubagentPermissionRequest {
            request_id,
            tool_name,
            ..
        } => (
            request_id.clone(),
            mew_protocol::AlertKind::PermissionNeeded,
            Some(tool_name.clone()),
        ),
        ServerMessage::WorkspacePermissionRequest { request_id, path } => (
            request_id.clone(),
            mew_protocol::AlertKind::PermissionNeeded,
            Some(path.clone()),
        ),
        ServerMessage::AskUserRequest { request_id, .. }
        | ServerMessage::GoalProposed { request_id, .. } => (
            request_id.clone(),
            mew_protocol::AlertKind::InputNeeded,
            None,
        ),
        _ => unreachable!("only required actions are published as pending requests"),
    };
    pending.lock().await.insert(
        request_id,
        session::PendingRequest {
            sender,
            message: message.clone(),
        },
    );
    session.broadcast(message).await;
    manager
        .broadcast_all(ServerMessage::SessionAlert {
            session_id: session.id.clone(),
            title: session.display_title().await,
            kind,
            detail,
        })
        .await;
    broadcast_attention(session, manager).await;
    Vec::new()
}

/// Translate a single `AgentEvent` (owned) into zero or more `ServerMessage`s.
/// Channel-bearing events are converted to wire requests with fresh IDs;
/// the `oneshot::Sender` is stashed in the `Session` for later response.
async fn translate_event(
    event: AgentEvent,
    session: &Session,
    session_mgr: &SessionManager,
) -> Vec<ServerMessage> {
    match event {
        AgentEvent::Provider(pe) => {
            // Intercept MessageEnd to accumulate usage on Meta and extract manifest.
            if let mew_provider::ProviderEvent::MessageEnd { usage, cost, .. } = &pe {
                let dir = session.session_dir.clone();

                // Phase 1: accumulate usage on session meta while holding agent lock.
                {
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
                        // The request's prompt size is the session's current
                        // context occupancy. Zero usage marks a provider that
                        // reported nothing; keep the last real reading.
                        if usage.input > 0 {
                            meta.context_tokens = Some(usage.input as u64);
                        }
                        let usage_clone = u.clone();
                        let _ = meta.set_usage(&dir, usage_clone).await;
                    }
                    // agent guard dropped here — do NOT hold it across messages lock.
                }

                // Phase 2: extract manifest from the last *assistant* message.
                // Search backwards instead of relying on .last() being the
                // assistant message — an ack message may have been appended
                // between MessageEnd dispatch and translate_event.
                //
                // Clone the Arc to the messages mutex and drop the agent guard
                // before locking messages — avoids nested async locks.
                let messages_arc = {
                    let agent = session.agent.lock().await;
                    agent.messages.clone()
                };
                let manifest = {
                    let messages = messages_arc.lock().await;
                    messages
                        .iter()
                        .rev()
                        .find_map(|m| m.assistant.as_ref())
                        .and_then(|meta| meta.manifest.clone())
                };

                let mut wire = mew_protocol::provider_event_to_wire(&pe);
                if let mew_message::ProviderEventWire::MessageEnd {
                    manifest: ref mut m,
                    ..
                } = &mut wire
                {
                    *m = manifest;
                }
                return vec![ServerMessage::Provider { event: wire }];
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
            let request = ServerMessage::PermissionRequest {
                request_id: session.next_request_id(),
                tool_name: call.tool_name,
                input: call.input,
            };
            publish_pending_request(
                session,
                session_mgr,
                &session.pending_permissions,
                tx,
                request,
            )
            .await
        }
        AgentEvent::WorkspacePermissionRequest { path, tx } => {
            let request = ServerMessage::WorkspacePermissionRequest {
                request_id: session.next_request_id(),
                path: path.display().to_string(),
            };
            publish_pending_request(
                session,
                session_mgr,
                &session.pending_permissions,
                tx,
                request,
            )
            .await
        }
        AgentEvent::AskUser {
            call_id,
            questions,
            tx,
        } => {
            let request = ServerMessage::AskUserRequest {
                request_id: session.next_request_id(),
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
            publish_pending_request(session, session_mgr, &session.pending_ask_user, tx, request)
                .await
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
            manifests,
        } => {
            vec![ServerMessage::SubagentEnd {
                parent_call_id,
                child_session_id,
                outcome: mew_protocol::subagent_outcome_to_wire(&outcome),
                manifests,
            }]
        }
        AgentEvent::SubagentPermissionRequest {
            parent_call_id,
            call,
            tx,
        } => {
            let request = ServerMessage::SubagentPermissionRequest {
                request_id: session.next_request_id(),
                parent_call_id,
                tool_name: call.tool_name,
                input: call.input,
            };
            publish_pending_request(
                session,
                session_mgr,
                &session.pending_permissions,
                tx,
                request,
            )
            .await
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
        AgentEvent::GoalProposed {
            call_id,
            objective,
            tx,
        } => {
            let request = ServerMessage::GoalProposed {
                request_id: session.next_request_id(),
                call_id,
                objective,
            };
            publish_pending_request(
                session,
                session_mgr,
                &session.pending_goal_proposals,
                tx,
                request,
            )
            .await
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
mod request_tests;

#[cfg(test)]
mod review_tests;

#[cfg(test)]
mod policy_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(created: i64) -> mew_message::Message {
        mew_message::Message {
            id: mew_message::MessageId::new(),
            session_id: mew_message::SessionId::new(),
            role: mew_message::Role::User,
            parts: Vec::new(),
            time: mew_message::Time {
                created,
                completed: None,
            },
            assistant: None,
        }
    }

    #[test]
    fn history_page_reads_backwards_and_returns_a_cursor() {
        let messages = (0..5).map(test_message).collect::<Vec<_>>();
        let (page, cursor) = history_page(&messages, messages.len(), 2);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].time.created, 3);
        assert_eq!(page[1].time.created, 4);
        assert_eq!(cursor, Some(3));

        let (older, cursor) = history_page(&messages, cursor.unwrap(), 2);
        assert_eq!(older.len(), 2);
        assert_eq!(older[0].time.created, 1);
        assert_eq!(older[1].time.created, 2);
        assert_eq!(cursor, Some(1));
    }

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

    #[test]
    fn remote_observe_scope_rejects_turns_and_mutations() {
        assert!(remote_message_allowed(
            RemoteScope::Observe,
            &ClientMessage::ListSessions
        ));
        assert!(!remote_message_allowed(
            RemoteScope::Observe,
            &ClientMessage::NewSession {
                cwd: None,
                client_kind: ClientKind::Remote,
            }
        ));
        assert!(!remote_message_allowed(
            RemoteScope::Observe,
            &ClientMessage::Prompt {
                text: "run tests".into(),
                attachments: Vec::new(),
            }
        ));
        assert!(!remote_message_allowed(
            RemoteScope::Observe,
            &ClientMessage::DeleteSession {
                session_id: "sess_1".into()
            }
        ));
    }

    #[test]
    fn remote_collaborate_scope_can_drive_turns_but_not_mutating_commands() {
        assert!(remote_message_allowed(
            RemoteScope::Collaborate,
            &ClientMessage::Prompt {
                text: "hello".into(),
                attachments: Vec::new(),
            }
        ));
        assert!(remote_message_allowed(
            RemoteScope::Collaborate,
            &ClientMessage::PermissionResponse {
                request_id: "req_1".into(),
                decision: mew_protocol::PermissionDecision::Deny,
            }
        ));
        assert!(!remote_message_allowed(
            RemoteScope::Collaborate,
            &ClientMessage::DeleteSession {
                session_id: "sess_1".into()
            }
        ));
        assert!(!remote_message_allowed(
            RemoteScope::Collaborate,
            &ClientMessage::TerminalOpen { rows: 24, cols: 80 }
        ));
        assert!(!remote_message_allowed(
            RemoteScope::Collaborate,
            &ClientMessage::TerminalInput {
                terminal_id: "terminal-1".into(),
                bytes: b"rm -rf .\n".to_vec(),
            }
        ));
        assert!(!remote_message_allowed(
            RemoteScope::Collaborate,
            &ClientMessage::GoalResponse {
                request_id: "request-1".into(),
                accepted: true,
            }
        ));
    }

    #[test]
    fn remote_control_scope_allows_protocol_commands() {
        assert!(remote_message_allowed(
            RemoteScope::Control,
            &ClientMessage::DeleteSession {
                session_id: "sess_1".into()
            }
        ));
        assert!(remote_message_allowed(
            RemoteScope::Control,
            &ClientMessage::GoalResponse {
                request_id: "request-1".into(),
                accepted: true,
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_snapshot_tracks_symlinks_without_reading_their_targets() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("external-directory");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("outside.txt"), "private").unwrap();
        let link = workspace.path().join("linked-directory");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let previous = workspace_snapshot(workspace.path());
        assert!(previous.contains_key("linked-directory"));
        assert!(!previous.contains_key("linked-directory/outside.txt"));

        std::fs::write(target.join("outside.txt"), "changed outside").unwrap();
        assert!(
            changed_workspace_paths(&previous, &workspace_snapshot(workspace.path())).is_empty()
        );

        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert_eq!(
            changed_workspace_paths(&previous, &workspace_snapshot(workspace.path())),
            vec!["linked-directory"]
        );
    }

    #[test]
    fn workspace_snapshot_reports_added_changed_and_removed_files() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        std::fs::write(&first, "one").unwrap();
        std::fs::write(&second, "two").unwrap();
        let previous = workspace_snapshot(dir.path());

        std::fs::write(&first, "changed").unwrap();
        std::fs::remove_file(&second).unwrap();
        std::fs::write(dir.path().join("third.txt"), "three").unwrap();
        let next = workspace_snapshot(dir.path());

        assert_eq!(
            changed_workspace_paths(&previous, &next),
            vec!["first.txt", "second.txt", "third.txt"]
        );
    }
}
