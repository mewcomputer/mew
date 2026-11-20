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
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio_tungstenite::accept_async;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tungstenite::Message;

const HISTORY_PAGE_MAX_BYTES: usize = 8 * 1024 * 1024;

struct BrowserRegistration {
    page_instance: String,
    session: Arc<session::Session>,
}

mod activity;
pub mod browser;
#[cfg(test)]
mod browser_tests;
pub mod client;
pub mod files;
pub mod groups;
mod history;
mod outbox;
pub mod projects;
pub mod remote;
pub mod session;
pub mod terminal;
mod workspace;

#[cfg(feature = "iroh")]
pub mod iroh_transport;

pub use client::DaemonClient;
pub use outbox::{ClientFrames, ClientSender};
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
    /// Daemon-wide summary setting, observed by clients and the idle-summary task.
    pub auto_summary_enabled: watch::Sender<bool>,
    /// Process-wide tools that need an awaited shutdown.
    pub shutdown_tools: Arc<Vec<Arc<dyn mew_tools::Tool>>>,
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
            auto_summary_enabled: watch::channel(true).0,
            shutdown_tools: Arc::new(Vec::new()),
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
            shutdown_tools: self.shutdown_tools.clone(),
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
        let mut sig_int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .context("install SIGINT handler")?;

        loop {
            tokio::select! {
                _ = sig_term.recv() => {
                    info!("received SIGTERM, shutting down");
                    break;
                }
                _ = sig_int.recv() => {
                    info!("received SIGINT, shutting down");
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

    /// Run the native TCP transport on loopback. Browser clients connect
    /// through a host relay that authenticates their origin and connection.
    pub async fn run_tcp(self, addr: SocketAddr) -> Result<()> {
        anyhow::ensure!(
            addr.ip().is_loopback(),
            "the native daemon TCP listener requires a loopback address"
        );
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
        let mut sig_int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .context("install SIGINT handler")?;

        loop {
            tokio::select! {
                _ = sig_term.recv() => {
                    info!("received SIGTERM, shutting down");
                    break;
                }
                _ = sig_int.recv() => {
                    info!("received SIGINT, shutting down");
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
                                let connection = async {
                                    let socket = tokio_tungstenite::accept_hdr_async(stream, native_tcp_handshake)
                                        .await
                                        .context("native websocket handshake")?;
                                    handle_websocket(socket, session_manager, groups_store, thinking_setter, auto_summary_enabled, None, None, false, None).await
                                };
                                if let Err(e) = connection.await {
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

#[allow(clippy::result_large_err)]
fn native_tcp_handshake(
    request: &tungstenite::handshake::server::Request,
    response: tungstenite::handshake::server::Response,
) -> std::result::Result<
    tungstenite::handshake::server::Response,
    tungstenite::handshake::server::ErrorResponse,
> {
    if request
        .headers()
        .contains_key(tungstenite::http::header::ORIGIN)
    {
        let mut denied = tungstenite::handshake::server::ErrorResponse::new(None);
        *denied.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
        return Err(denied);
    }
    Ok(response)
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
                | ClientMessage::DeleteSessionConfirmed { .. }
                | ClientMessage::RenameSession { .. }
                | ClientMessage::ReadAutomationSettings
                | ClientMessage::ReadDaemonSettings { .. }
                | ClientMessage::SaveSessionDefaults { .. }
                | ClientMessage::ResetSessionDefaults { .. }
                | ClientMessage::RevokeDevice { .. }
                | ClientMessage::SetAutoTitle { .. }
                | ClientMessage::SetAutoSummary { .. }
                | ClientMessage::Prompt { .. }
                | ClientMessage::SubmitPrompt { .. }
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
                | ClientMessage::ReadSubagentHistory { .. }
                | ClientMessage::GuideSubagent { .. }
                | ClientMessage::ReadSubagentMessageReceipt { .. }
                | ClientMessage::ReadJobOutput { .. }
                | ClientMessage::ReadPromptReceipt { .. }
                | ClientMessage::UpdateTodo { .. }
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
            | ClientMessage::ReadAutomationSettings
            | ClientMessage::Ping
            | ClientMessage::ListDir { .. }
            | ClientMessage::ListFilesystemDir { .. }
            | ClientMessage::ReadFilePreview { .. }
            | ClientMessage::GitStatus { .. }
            | ClientMessage::ReadSessionDiff { .. }
            | ClientMessage::ReadSubagentHistory { .. }
            | ClientMessage::ReadSubagentMessageReceipt { .. }
            | ClientMessage::ReadJobOutput { .. }
            | ClientMessage::ReadPromptReceipt { .. }
            | ClientMessage::WatchWorkspace { .. }
    );
    if observe {
        return true;
    }

    scope == RemoteScope::Collaborate
        && matches!(
            message,
            ClientMessage::Prompt { .. }
                | ClientMessage::SubmitPrompt { .. }
                | ClientMessage::Cancel
                | ClientMessage::PermissionResponse { .. }
                | ClientMessage::AskUserResponse { .. }
                | ClientMessage::YieldControl { .. }
        )
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
async fn watch_workspace(
    session: std::sync::Weak<Session>,
    root: PathBuf,
    cancel: CancellationToken,
) {
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
                    let Some(session) = session.upgrade() else { break; };
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
    auto_summary_enabled: watch::Sender<bool>,
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
    auto_summary_enabled: watch::Sender<bool>,
    remote_scope: Option<RemoteScope>,
    remote_token: Option<&str>,
    legacy_iroh: bool,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let authorizer = remote_token_authorizer(remote_scope, remote_token);
    handle_connection_with_authorizer(
        stream,
        session_manager,
        groups_store,
        thinking_setter,
        auto_summary_enabled,
        remote_scope,
        authorizer,
        legacy_iroh,
        None,
    )
    .await
}

pub(crate) fn remote_token_authorizer(
    remote_scope: Option<RemoteScope>,
    remote_token: Option<&str>,
) -> Option<RemoteAuthorizer> {
    match (remote_scope, remote_token) {
        (Some(scope), Some(expected)) => {
            let expected = expected.to_owned();
            Some(Arc::new(move |token: Option<&str>, _device_name: &str| {
                Ok((token == Some(expected.as_str())).then_some(scope))
            }) as RemoteAuthorizer)
        }
        _ => None,
    }
}

/// Resolve a remote connection's scope from its first protocol message.
pub type RemoteAuthorizer =
    Arc<dyn Fn(Option<&str>, &str) -> Result<Option<RemoteScope>> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct RemoteAuthorization {
    peer: String,
    devices: remote::RemoteDeviceManager,
    revocation_receipt: Arc<std::sync::Mutex<()>>,
}

impl RemoteAuthorization {
    fn require_control(&self) -> Result<()> {
        anyhow::ensure!(
            self.devices
                .authorized_scope(&self.peer, remote::unix_now())?
                == Some(RemoteScope::Control),
            "this device no longer has full control"
        );
        Ok(())
    }

    async fn scope(&self) -> Result<Option<RemoteScope>> {
        let authorization = self.clone();
        tokio::task::spawn_blocking(move || {
            let _receipt = authorization
                .revocation_receipt
                .lock()
                .expect("revocation receipt lock");
            authorization
                .devices
                .authorized_scope(&authorization.peer, remote::unix_now())
        })
        .await?
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_connection_with_authorizer<S>(
    stream: S,
    session_manager: Arc<SessionManager>,
    groups_store: Arc<groups::GroupsStore>,
    thinking_setter: Option<ThinkingSetter>,
    auto_summary_enabled: watch::Sender<bool>,
    remote_scope: Option<RemoteScope>,
    remote_authorizer: Option<RemoteAuthorizer>,
    legacy_iroh: bool,
    remote_peer: Option<String>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ws_stream = accept_async(stream).await.context("websocket handshake")?;
    handle_websocket(
        ws_stream,
        session_manager,
        groups_store,
        thinking_setter,
        auto_summary_enabled,
        remote_scope,
        remote_authorizer,
        legacy_iroh,
        remote_peer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_websocket<S>(
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    session_manager: Arc<SessionManager>,
    groups_store: Arc<groups::GroupsStore>,
    thinking_setter: Option<ThinkingSetter>,
    auto_summary_enabled: watch::Sender<bool>,
    mut remote_scope: Option<RemoteScope>,
    remote_authorizer: Option<RemoteAuthorizer>,
    legacy_iroh: bool,
    remote_peer: Option<String>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (ws_tx, mut ws_rx) = ws_stream.split();

    let connection_lifetime = CancellationToken::new();
    let (client_tx, ClientFrames(mut client_rx)) =
        ClientSender::channel(connection_lifetime.clone());
    let mut attached_session: Option<Arc<Session>> = None;
    let browser_owner = ulid::Ulid::new().to_string();
    let mut confirmed_desktop_sessions = std::collections::HashSet::new();
    let browser_pages = Arc::new(std::sync::Mutex::new(
        HashMap::<String, BrowserRegistration>::new(),
    ));
    let mut client_id: Option<u64> = None;
    let mut terminal: Option<terminal::TerminalHandle> = None;
    let mut workspace_watch: Option<CancellationToken> = None;
    let guidance_slots = Arc::new(tokio::sync::Semaphore::new(16));
    let mut auto_title_enabled = true;
    let mut summary_updates = auto_summary_enabled.subscribe();
    let mut settings_updates = session_manager.settings_changed.subscribe();
    let mut observes_daemon_settings = false;
    let authorization = remote_peer.and_then(|peer| {
        session_manager
            .remote_devices
            .get()
            .cloned()
            .map(|devices| RemoteAuthorization {
                peer,
                devices,
                revocation_receipt: Arc::new(std::sync::Mutex::new(())),
            })
    });
    let mut authorization_monitor = None;
    let mut remote_ready = remote_scope.is_none() && remote_authorizer.is_none();

    // auto_summary_enabled is passed in from the daemon (daemon-wide task
    // is spawned once in run()/run_tcp(), not per-connection).

    // Spawn a writer task that owns the WebSocket sink.
    let mut ws_tx = ws_tx;
    let writer_lifetime = connection_lifetime.clone();
    let writer = tokio::spawn(async move {
        {
            let write_messages = async {
                loop {
                    let msg = tokio::select! {
                        message = client_rx.recv() => match message { Some(message) => message, None => break },
                        _ = writer_lifetime.cancelled() => {
                            client_rx.close();
                            while let Some(message) = client_rx.recv().await {
                                if send_msg(&mut ws_tx, message).await.is_err() { break; }
                            }
                            break;
                        }
                    };
                    if !matches!(
                        tokio::time::timeout(outbox::WRITE_TIMEOUT, send_msg(&mut ws_tx, msg))
                            .await,
                        Ok(Ok(()))
                    ) {
                        writer_lifetime.cancel();
                        break;
                    }
                }
            };
            tokio::pin!(write_messages);
            tokio::select! {
                _ = &mut write_messages => {}
                _ = writer_lifetime.cancelled() => {
                    // Allow a self-revocation receipt to flush, but never retain a blocked peer.
                    let _ = tokio::time::timeout(std::time::Duration::from_millis(250), &mut write_messages).await;
                }
            }
        }
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), ws_tx.close()).await;
    });

    let reply = |msg: ServerMessage| {
        let _ = client_tx.send(msg);
    };
    let session_failure = |operation, session_id, message: String| {
        reply(ServerMessage::SessionOperationFailed {
            operation,
            session_id,
            message: message.clone(),
        });
        reply(ServerMessage::Error { message });
    };

    enum ConnectionUpdate {
        Frame(std::result::Result<Message, tungstenite::Error>),
        Summary,
        Settings,
    }
    'connection: loop {
        if remote_ready && authorization_monitor.is_none() {
            if let Some(authorization) = authorization.clone() {
                let lifetime = connection_lifetime.clone();
                authorization_monitor = Some(tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        tokio::select! {
                            _ = lifetime.cancelled() => break,
                            _ = interval.tick() => {
                                if !matches!(authorization.scope().await, Ok(Some(_))) {
                                    lifetime.cancel();
                                    break;
                                }
                            }
                        }
                    }
                }));
            }
        }
        let update = tokio::select! {
            // Deliver pending setting changes before the next client command.
            biased;
            _ = connection_lifetime.cancelled() => break,
            changed = settings_updates.changed(), if observes_daemon_settings && remote_ready && remote_scope.is_none_or(|scope| scope == RemoteScope::Control) => {
                if changed.is_err() { break; }
                ConnectionUpdate::Settings
            }
            changed = summary_updates.changed(), if remote_ready => {
                if changed.is_err() { break; }
                ConnectionUpdate::Summary
            }
            message = ws_rx.next() => {
                let Some(message) = message else { break; };
                ConnectionUpdate::Frame(message)
            }
        };
        if remote_ready {
            if let Some(authorization) = &authorization {
                match authorization.scope().await {
                    Ok(Some(scope)) => {
                        if remote_scope.is_some() {
                            remote_scope = Some(scope);
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        tracing::warn!(%error, "could not verify remote authorization");
                        break;
                    }
                }
            }
        }
        let msg = match update {
            ConnectionUpdate::Frame(message) => message,
            ConnectionUpdate::Summary => {
                reply(ServerMessage::AutoSummaryChanged {
                    enabled: *summary_updates.borrow_and_update(),
                });
                continue;
            }
            ConnectionUpdate::Settings => {
                if remote_scope.is_some_and(|scope| scope != RemoteScope::Control) {
                    continue;
                }
                let result = tokio::select! {
                    biased;
                    _ = connection_lifetime.cancelled() => break 'connection,
                    result = daemon_settings_message(&session_manager, None, authorization.clone()) => result,
                };
                match result {
                    Ok(message) => reply(message),
                    Err(error) => tracing::warn!(%error, "could not publish daemon settings"),
                }
                continue;
            }
        };
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
                let message = format!(
                    "remote access scope `{}` does not allow this operation",
                    remote_scope_label(scope)
                );
                match &client_msg {
                    ClientMessage::ReadDaemonSettings { request_id }
                    | ClientMessage::SaveSessionDefaults { request_id, .. }
                    | ClientMessage::ResetSessionDefaults { request_id }
                    | ClientMessage::RevokeDevice { request_id, .. } => {
                        reply(ServerMessage::DaemonSettingsFailed {
                            request_id: request_id.clone(),
                            message,
                        })
                    }
                    ClientMessage::DeleteSessionConfirmed {
                        session_id,
                        request_id,
                    } => {
                        reply(ServerMessage::SessionDeletionResult {
                            session_id: session_id.clone(),
                            request_id: request_id.clone(),
                            error: Some(message),
                        });
                    }
                    ClientMessage::GuideSubagent {
                        session_id,
                        child_session_id,
                        request_id,
                        ..
                    } => {
                        reply(ServerMessage::SubagentMessageReceipt {
                            session_id: session_id.clone(),
                            child_session_id: child_session_id.clone(),
                            request_id: request_id.clone(),
                            status: mew_protocol::PromptStatus::Rejected,
                            message: Some(message),
                        });
                    }
                    _ => reply(ServerMessage::Error { message }),
                }
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
                        session_failure(
                            mew_protocol::SessionOperation::Create,
                            None,
                            format!("session cwd does not exist: {}", cwd_str.display()),
                        );
                        continue;
                    }
                    if !cwd_str.is_dir() {
                        session_failure(
                            mew_protocol::SessionOperation::Create,
                            None,
                            format!("session cwd is not a directory: {}", cwd_str.display()),
                        );
                        continue;
                    }
                }
                if let Some(group_id) = requested_group_id.as_deref() {
                    if !groups_store.contains(group_id).await {
                        session_failure(
                            mew_protocol::SessionOperation::Create,
                            None,
                            "session group not found".into(),
                        );
                        continue;
                    }
                }
                match session_manager.create(cwd.clone()).await {
                    Ok(session) => {
                        if let Some(group_id) = requested_group_id.as_deref() {
                            let group = group_id.to_owned();
                            if let Err(e) = session_manager
                                .update_metadata(&session.id, move |meta| {
                                    meta.group_id = Some(group)
                                })
                                .await
                            {
                                session_failure(
                                    mew_protocol::SessionOperation::Create,
                                    None,
                                    format!("failed to store session group: {e}"),
                                );
                                continue;
                            }
                            if let Err(e) = groups_store
                                .assign_session(&session.id, Some(group_id.to_owned()))
                                .await
                            {
                                session_failure(
                                    mew_protocol::SessionOperation::Create,
                                    None,
                                    format!("failed to assign session group: {e}"),
                                );
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
                        let (cid, was_first) = session
                            .attach_client_ready(
                                client_tx.clone(),
                                client_kind,
                                ServerMessage::SessionReady {
                                    session_id: session_id.clone(),
                                    cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
                                    model,
                                    provider,
                                    permission_mode,
                                },
                            )
                            .await;
                        client_id = Some(cid);
                        attached_session = Some(session.clone());
                        if client_kind == ClientKind::Desktop
                            && !legacy_iroh
                            && remote_scope.is_none()
                            && remote_authorizer.is_none()
                        {
                            confirmed_desktop_sessions.insert(session.id.clone());
                        }
                        let running = session.is_running.lock().await;
                        reply(ServerMessage::SessionActivityChanged {
                            session_id,
                            activity: if *running {
                                SessionState::Running
                            } else {
                                SessionState::Idle
                            },
                        });
                        drop(running);
                        session.send_activity(&client_tx).await;
                        session.send_todos(&client_tx).await;
                        // Report the new session's active thinking variant so
                        // the frontend renders the pill immediately.
                        reply(ServerMessage::ThinkingVariantChanged {
                            variant: thinking_variant,
                        });
                        drop(actions);
                        // Notify other clients that a new client joined.
                        if !was_first {
                            session.notify_client_attached(cid, client_kind).await;
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, error_chain = %format!("{e:#}"), "failed to create session");
                        session_failure(
                            mew_protocol::SessionOperation::Create,
                            None,
                            format!("failed to create session: {e:#}"),
                        );
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
                        // Register and queue history plus the in-flight prefix
                        // before the provider can publish another delta.
                        let provider_replay = session.provider_replay.lock().await;
                        let (cid, was_first) = session
                            .attach_client_ready(
                                client_tx.clone(),
                                client_kind,
                                ServerMessage::SessionReady {
                                    session_id: session_id.clone(),
                                    cwd,
                                    model,
                                    provider,
                                    permission_mode,
                                },
                            )
                            .await;
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
                        let running = session.is_running.lock().await;
                        reply(ServerMessage::SessionActivityChanged {
                            session_id: session_id.clone(),
                            activity: if *running {
                                SessionState::Running
                            } else {
                                SessionState::Idle
                            },
                        });
                        drop(running);
                        session.send_activity(&client_tx).await;
                        session.send_todos(&client_tx).await;
                        // Report the session's current thinking variant so the
                        // frontend renders the pill immediately on attach instead
                        // of showing "no thinking" until the user toggles it.
                        reply(ServerMessage::ThinkingVariantChanged {
                            variant: thinking_variant,
                        });

                        let initial_history = async {
                            let mut history = session.history().await?;
                            let (messages, next_cursor) = history
                                .page(usize::MAX, mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT)
                                .await?;
                            let already_in_history = provider_replay.present_in(&messages);
                            client_tx
                                .send_history_page(ServerMessage::SessionHistoryPage {
                                    session_id: session_id.clone(),
                                    messages,
                                    next_cursor,
                                    replace: true,
                                })
                                .await?;
                            if !already_in_history {
                                provider_replay.send(&client_tx);
                            }
                            Ok::<_, anyhow::Error>(next_cursor.map(|cursor| (history, cursor)))
                        }
                        .await;
                        let older_history = match initial_history {
                            Ok(history) => history,
                            Err(error) => {
                                reply(ServerMessage::Error {
                                    message: format!("failed to load session history: {error:#}"),
                                });
                                provider_replay.send(&client_tx);
                                None
                            }
                        };
                        provider_replay.send_tool_activity(&client_tx);
                        drop(provider_replay);

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
                            session.notify_client_attached(cid, client_kind).await;
                        }

                        // The TUI receives older pages without holding the
                        // publication locks needed by a live turn.
                        if let Some((mut history, mut before)) =
                            older_history.filter(|_| client_kind == ClientKind::Tui)
                        {
                            loop {
                                let (messages, next_cursor) = match history
                                    .page(before, mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT)
                                    .await
                                {
                                    Ok(page) => page,
                                    Err(error) => {
                                        reply(ServerMessage::Error {
                                            message: format!(
                                                "failed to load session history: {error:#}"
                                            ),
                                        });
                                        break;
                                    }
                                };
                                if client_tx
                                    .send_history_page(ServerMessage::SessionHistoryPage {
                                        session_id: session_id.clone(),
                                        messages,
                                        next_cursor,
                                        replace: false,
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                let Some(cursor) = next_cursor else {
                                    break;
                                };
                                before = cursor;
                            }
                        }
                    }
                    Err(AttachError::NotFound) => {
                        session_failure(
                            mew_protocol::SessionOperation::Attach,
                            Some(session_id),
                            "session not found".into(),
                        );
                    }
                    Err(AttachError::NotTopLevel) => {
                        session_failure(
                            mew_protocol::SessionOperation::Attach,
                            Some(session_id),
                            "cannot attach to subagent session".into(),
                        );
                    }
                    Err(error @ AttachError::InUse) => {
                        session_failure(
                            mew_protocol::SessionOperation::Attach,
                            Some(session_id),
                            error.to_string(),
                        );
                    }
                    Err(AttachError::BuildAgent(e)) => {
                        tracing::error!(error = %e, error_chain = %format!("{e:#}"), "failed to resume session");
                        session_failure(
                            mew_protocol::SessionOperation::Attach,
                            Some(session_id),
                            format!("failed to resume session: {e:#}"),
                        );
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
                let page = async {
                    let mut history = session.history().await?;
                    history
                        .page(before, limit.min(mew_protocol::DEFAULT_HISTORY_PAGE_LIMIT))
                        .await
                }
                .await;
                let (messages, next_cursor) = match page {
                    Ok(page) => page,
                    Err(error) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to load session history: {error:#}"),
                        });
                        continue;
                    }
                };
                let _ = client_tx
                    .send_history_page(ServerMessage::SessionHistoryPage {
                        session_id,
                        messages,
                        next_cursor,
                        replace: false,
                    })
                    .await;
            }
            ClientMessage::ReadJobOutput {
                session_id,
                job_id,
                request_id,
                offset,
            } => {
                let result = match attached_session
                    .as_ref()
                    .filter(|session| session.id == session_id)
                {
                    Some(session) => {
                        session
                            .agent
                            .lock()
                            .await
                            .shell_job_output(&job_id, offset)
                            .await
                    }
                    None => Err("output requires the attached session".to_string()),
                };
                reply(match result {
                    Ok(chunk) => ServerMessage::JobOutput {
                        session_id,
                        job_id,
                        request_id,
                        offset,
                        next_offset: chunk.next_offset,
                        output: chunk.output,
                        done: chunk.done,
                        incomplete: chunk.incomplete,
                    },
                    Err(message) => ServerMessage::JobOutputError {
                        session_id,
                        job_id,
                        request_id,
                        message,
                    },
                });
            }
            ClientMessage::ReadSubagentHistory {
                session_id,
                child_session_id,
                request_id,
            } => {
                let result = match attached_session
                    .as_ref()
                    .filter(|parent| parent.id == session_id)
                {
                    Some(parent) => parent.subagent_history(&child_session_id).await,
                    None => Err(anyhow::anyhow!(
                        "history requires the attached parent session"
                    )),
                };
                let (messages, error) = match result {
                    Ok(messages) => (messages, None),
                    Err(error) => (Vec::new(), Some(format!("{error:#}"))),
                };
                reply(ServerMessage::SubagentHistory {
                    session_id,
                    child_session_id,
                    request_id,
                    messages,
                    error,
                });
            }
            ClientMessage::GuideSubagent {
                session_id,
                child_session_id,
                request_id,
                text,
            } => {
                let Ok(slot) = guidance_slots.clone().try_acquire_owned() else {
                    reply(ServerMessage::SubagentMessageReceipt {
                        session_id,
                        child_session_id,
                        request_id,
                        status: mew_protocol::PromptStatus::Unknown,
                        message: Some("Too many pending subagent message requests.".into()),
                    });
                    continue;
                };
                let parent = attached_session
                    .clone()
                    .filter(|parent| parent.id == session_id);
                let client = client_tx.clone();
                let lifetime = connection_lifetime.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    // Closing the socket drops its receipt waiter. A guide
                    // already queued on the child can still be admitted.
                    let (status, message) = tokio::select! {
                        result = subagent_message_status(
                            parent.as_deref(), &child_session_id, &request_id, Some(text),
                        ) => result,
                        _ = lifetime.cancelled() => return,
                    };
                    let _ = client.send(ServerMessage::SubagentMessageReceipt {
                        session_id,
                        child_session_id,
                        request_id,
                        status,
                        message,
                    });
                });
            }
            ClientMessage::ReadSubagentMessageReceipt {
                session_id,
                child_session_id,
                request_id,
            } => {
                let parent = attached_session
                    .as_deref()
                    .filter(|parent| parent.id == session_id);
                let (status, message) =
                    subagent_message_status(parent, &child_session_id, &request_id, None).await;
                reply(ServerMessage::SubagentMessageReceipt {
                    session_id,
                    child_session_id,
                    request_id,
                    status,
                    message,
                });
            }
            ClientMessage::ListSessions => {
                let sessions = session_manager.list().await;
                reply(ServerMessage::SessionList { sessions });
                let groups = groups_store.list().await;
                reply(ServerMessage::GroupList { groups });
            }
            ClientMessage::DeleteSession { ref session_id }
            | ClientMessage::DeleteSessionConfirmed { ref session_id, .. } => {
                let report = |error: Option<String>| {
                    if let ClientMessage::DeleteSessionConfirmed { request_id, .. } = &client_msg {
                        reply(ServerMessage::SessionDeletionResult {
                            session_id: session_id.clone(),
                            request_id: request_id.clone(),
                            error,
                        });
                    } else if let Some(message) = error {
                        reply(ServerMessage::Error { message });
                    }
                };
                let browser_session = browser_pages
                    .lock()
                    .expect("browser registrations lock")
                    .get(session_id)
                    .map(|registration| Arc::downgrade(&registration.session));
                if let Err(error) = session_manager
                    .delete(
                        session_id,
                        attached_session.as_ref(),
                        browser_session.as_ref(),
                    )
                    .await
                {
                    report(Some(format!("could not delete session: {error}")));
                    continue;
                }
                confirmed_desktop_sessions.remove(session_id);
                browser_pages
                    .lock()
                    .expect("browser registrations lock")
                    .remove(session_id);
                tracing::info!(session_id = %session_id, "deleted session");
                // If the deleted session was the current one, navigate home.
                if attached_session
                    .as_ref()
                    .is_some_and(|session| session.id == *session_id)
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
                report(None);
            }
            ClientMessage::RenameSession { session_id, title } => {
                let stored_title = title.clone();
                match session_manager
                    .update_metadata(&session_id, move |meta| {
                        meta.custom_title = Some(stored_title)
                    })
                    .await
                {
                    Ok(_) => session_manager.broadcast_title(&session_id, title).await,
                    Err(error) => reply(ServerMessage::Error {
                        message: format!("could not rename session: {error}"),
                    }),
                }
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
                        let stored_title = title.clone();
                        match session_manager
                            .update_metadata(&session_id, move |meta| {
                                meta.custom_title = Some(stored_title);
                            })
                            .await
                        {
                            Ok(_) => session_manager.broadcast_title(&session_id, title).await,
                            Err(error) => reply(ServerMessage::Error {
                                message: format!("could not save session title: {error}"),
                            }),
                        }
                    }
                    Err(_) => {
                        reply(ServerMessage::Error {
                            message: "session not found".into(),
                        });
                    }
                }
            }
            ClientMessage::ReadAutomationSettings => {
                reply(ServerMessage::AutomationSettings {
                    auto_title_enabled,
                    auto_summary_enabled: *auto_summary_enabled.borrow(),
                });
            }
            message @ (ClientMessage::ReadDaemonSettings { .. }
            | ClientMessage::SaveSessionDefaults { .. }
            | ClientMessage::ResetSessionDefaults { .. }
            | ClientMessage::RevokeDevice { .. }) => {
                observes_daemon_settings = true;
                let request_id = match &message {
                    ClientMessage::ReadDaemonSettings { request_id }
                    | ClientMessage::SaveSessionDefaults { request_id, .. }
                    | ClientMessage::ResetSessionDefaults { request_id }
                    | ClientMessage::RevokeDevice { request_id, .. } => request_id.clone(),
                    _ => unreachable!(),
                };
                let operation = async {
                    match message {
                        ClientMessage::ReadDaemonSettings { .. } => {}
                        ClientMessage::SaveSessionDefaults { session_id, .. } => {
                            let session = attached_session
                                .as_ref()
                                .filter(|session| session.id == session_id)
                                .context("open this conversation before saving its defaults")?;
                            #[cfg(test)]
                            session_manager.settings_operation_started.notify_one();
                            let policy = session.policy(&*session.agent.lock().await).await;
                            session_manager
                                .set_session_defaults_authorized(
                                    Some(policy),
                                    authorization.clone(),
                                )
                                .await?;
                        }
                        ClientMessage::ResetSessionDefaults { .. } => {
                            session_manager
                                .set_session_defaults_authorized(None, authorization.clone())
                                .await?
                        }
                        ClientMessage::RevokeDevice { device_id, .. } => {
                            // Capture the final authorized snapshot before self-revocation removes access.
                            let mut confirmation = daemon_settings_message(
                                &session_manager,
                                Some(request_id.clone()),
                                authorization.clone(),
                            )
                            .await?;
                            let devices = session_manager
                                .remote_devices
                                .get()
                                .cloned()
                                .context("remote device management is unavailable")?;
                            let actor = authorization
                                .as_ref()
                                .map(|authorization| authorization.peer.clone());
                            let receipt = authorization
                                .as_ref()
                                .map(|authorization| authorization.revocation_receipt.clone());
                            let changes = session_manager.settings_changed.clone();
                            let sender = client_tx.clone();
                            #[cfg(test)]
                            let commit_hook = session_manager
                                .revocation_commit_hook
                                .lock()
                                .unwrap()
                                .clone();
                            tokio::task::spawn_blocking(move || -> Result<()> {
                                let _receipt = receipt.as_ref().map(|receipt| {
                                    receipt.lock().expect("revocation receipt lock")
                                });
                                devices.revoke_as(
                                    &device_id,
                                    remote::unix_now(),
                                    actor.as_deref(),
                                )?;
                                #[cfg(test)]
                                if let Some(hook) = commit_hook {
                                    hook();
                                }
                                if let ServerMessage::DaemonSettings {
                                    devices: Some(devices),
                                    ..
                                } = &mut confirmation
                                {
                                    devices.retain(|device| device.id != device_id);
                                }
                                let _ = sender.send(confirmation);
                                changes.send_replace(());
                                Ok(())
                            })
                            .await??;
                            return Ok(None);
                        }
                        _ => unreachable!(),
                    }
                    daemon_settings_message(
                        &session_manager,
                        Some(request_id.clone()),
                        authorization.clone(),
                    )
                    .await
                    .map(Some)
                };
                let result = tokio::select! {
                    biased;
                    _ = connection_lifetime.cancelled() => break 'connection,
                    result = operation => result,
                };
                match result {
                    Ok(Some(message)) => reply(message),
                    Ok(None) => {}
                    Err(error) => reply(ServerMessage::DaemonSettingsFailed {
                        request_id,
                        message: format!("{error:#}"),
                    }),
                }
            }
            ClientMessage::SetAutoTitle { enabled } => {
                auto_title_enabled = enabled;
                tracing::info!(auto_title = enabled, "auto-title setting changed");
                reply(ServerMessage::AutomationSettings {
                    auto_title_enabled,
                    auto_summary_enabled: *auto_summary_enabled.borrow(),
                });
            }
            ClientMessage::SetAutoSummary { enabled } => {
                auto_summary_enabled.send_replace(enabled);
                tracing::info!(auto_summary = enabled, "auto-summary setting changed");
                reply(ServerMessage::AutomationSettings {
                    auto_title_enabled,
                    auto_summary_enabled: *summary_updates.borrow_and_update(),
                });
            }
            ClientMessage::ReadPromptReceipt {
                session_id,
                request_id,
            } => {
                let session = session_manager
                    .active
                    .lock()
                    .await
                    .get(&session_id)
                    .cloned();
                let receipt = match session {
                    Some(session) => session
                        .prompt_receipts
                        .lock()
                        .await
                        .get(&request_id)
                        .cloned(),
                    None if !request_id.is_empty()
                        && request_id.len() <= 128
                        && std::path::Path::new(&session_id)
                            .file_name()
                            .is_some_and(|name| name == session_id.as_str()) =>
                    {
                        match mew_session::Reader::has_prompt_receipt(
                            &session_manager.session_dir,
                            &session_id,
                            &request_id,
                        )
                        .await
                        {
                            Ok(true) => Some((mew_protocol::PromptStatus::Accepted, None)),
                            Ok(false) => None,
                            Err(mew_session::SessionError::Io(error))
                                if error.kind() == std::io::ErrorKind::NotFound =>
                            {
                                None
                            }
                            Err(error) => {
                                tracing::warn!(%session_id, %error, "could not read persisted prompt receipt");
                                None
                            }
                        }
                    }
                    None => None,
                };
                let (status, message) = receipt.unwrap_or((mew_protocol::PromptStatus::Unknown, Some("submission could not be confirmed; inspect the conversation before retrying".into())));
                reply(ServerMessage::PromptReceipt {
                    session_id,
                    request_id,
                    status,
                    message,
                });
            }
            ClientMessage::SubmitPrompt {
                session_id,
                request_id,
                text,
                attachments,
            } => {
                let Some(session) = attached_session
                    .clone()
                    .filter(|session| session.id == session_id)
                else {
                    reply(ServerMessage::PromptReceipt {
                        session_id,
                        request_id,
                        status: mew_protocol::PromptStatus::Rejected,
                        message: Some("the requested conversation is not attached".into()),
                    });
                    continue;
                };
                let client_tx = client_tx.clone();
                let session_mgr = session_manager.clone();
                tokio::spawn(async move {
                    let mut receipts = session.prompt_receipts.lock().await;
                    if let Some((status, message)) = receipts.get(&request_id) {
                        let _ = client_tx.send(ServerMessage::PromptReceipt {
                            session_id,
                            request_id,
                            status: *status,
                            message: message.clone(),
                        });
                        return;
                    }
                    let guard = match session.turn_lock.try_lock() {
                        Ok(guard) => Ok(guard),
                        Err(error) if *session.is_running.lock().await => Err(error),
                        Err(_) => Ok(session.turn_lock.lock().await),
                    };
                    let error = if text.trim().is_empty() {
                        Some("the prompt is empty")
                    } else if request_id.is_empty() || request_id.len() > 128 {
                        Some("invalid submission request ID")
                    } else if guard.is_err() {
                        Some("a turn is already running; your draft is preserved")
                    } else {
                        None
                    };
                    if let Some(error) = error {
                        let message = Some(error.to_owned());
                        if !request_id.is_empty() && request_id.len() <= 128 {
                            receipts.insert(
                                request_id.clone(),
                                (mew_protocol::PromptStatus::Rejected, message.clone()),
                            );
                        }
                        let _ = client_tx.send(ServerMessage::PromptReceipt {
                            session_id,
                            request_id,
                            status: mew_protocol::PromptStatus::Rejected,
                            message,
                        });
                        return;
                    }
                    let guard = guard.expect("validated turn lock");
                    let token = register_turn(&session).await;
                    // Attach registers and queues its history under the same lock.
                    // A new peer sees this message in history or in the live echo.
                    let projection = session.provider_replay.lock().await;
                    let admitted = session
                        .agent
                        .lock()
                        .await
                        .accept_prompt(text.clone(), attachment_parts(attachments), &request_id)
                        .await;
                    let prompt = match admitted {
                        Ok(prompt) => prompt,
                        Err(error) => {
                            *session.current_turn_cancel.lock().await = None;
                            *session.is_running.lock().await = false;
                            let status = match &error {
                                mew_agent::PromptAdmissionError::Persistence(
                                    mew_session::SessionError::TranscriptCommitUnknown { .. },
                                ) => mew_protocol::PromptStatus::Unknown,
                                _ => mew_protocol::PromptStatus::Rejected,
                            };
                            let message = Some(error.to_string());
                            receipts.insert(request_id.clone(), (status, message.clone()));
                            drop(receipts);
                            let _ = client_tx.send(ServerMessage::PromptReceipt {
                                session_id: session_id.clone(),
                                request_id,
                                status,
                                message,
                            });
                            drop(projection);
                            session_mgr
                                .broadcast_activity(&session_id, SessionState::Idle)
                                .await;
                            return;
                        }
                    };
                    receipts.insert(
                        request_id.clone(),
                        (mew_protocol::PromptStatus::Accepted, None),
                    );
                    drop(receipts);
                    let _ = client_tx.send(ServerMessage::PromptReceipt {
                        session_id,
                        request_id: request_id.clone(),
                        status: mew_protocol::PromptStatus::Accepted,
                        message: None,
                    });
                    session
                        .broadcast(ServerMessage::UserMessage {
                            text: text.clone(),
                            request_id: Some(request_id),
                        })
                        .await;
                    drop(projection);
                    finish_prompt_turn(
                        &session,
                        &session_mgr,
                        text,
                        PromptTurn::Accepted(prompt),
                        auto_title_enabled,
                        guard,
                        token,
                    )
                    .await;
                });
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
                    .broadcast(ServerMessage::UserMessage {
                        text: text.clone(),
                        request_id: None,
                    })
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
                    let token = register_turn(&session).await;
                    finish_prompt_turn(
                        &session,
                        &session_mgr,
                        text.clone(),
                        PromptTurn::New { text, attachments },
                        auto_title_enabled,
                        _guard,
                        token,
                    )
                    .await;
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
                session
                    .broadcast(ServerMessage::UserMessage {
                        text,
                        request_id: None,
                    })
                    .await;
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
                    browser_focus_preflight: true,
                    scoped_session_failures: true,
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
                cancelled,
            } => {
                if let Some(session) = &attached_session {
                    let _actions = session.pending_action_lock.lock().await;
                    let request = session.pending_ask_user.lock().await.remove(&request_id);
                    if let Some(request) = request {
                        if !cancelled {
                            let _ = request.sender.send(answers);
                        }
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
                    let (cmd, _arg) = match command.split_once(' ') {
                        Some((c, a)) => (c, Some(a)),
                        None => (command.as_str(), None),
                    };
                    let token = if cmd == "/wiki" {
                        Some(register_turn(&session).await)
                    } else {
                        None
                    };
                    let result = {
                        let agent = session.agent.lock().await;
                        match cmd {
                            "/clear" => match agent.clear_context().await {
                                Ok(()) => {
                                    session.broadcast(ServerMessage::SessionCleared).await;
                                    Some("context cleared".to_string())
                                }
                                Err(error) => Some(format!("Could not clear context. {error}")),
                            },
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
                                let token = token.expect("wiki turn registered");
                                let had_error = run_turn(
                                    &session,
                                    &session_manager_clone,
                                    &wiki_agent,
                                    PromptTurn::New {
                                        text: wiki_prompt.to_string(),
                                        attachments: Vec::new(),
                                    },
                                    token.clone(),
                                )
                                .await;
                                if token.is_cancelled() {
                                    Some("wiki generation cancelled".to_string())
                                } else if had_error {
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
                                let name = entry.file_name();
                                let Some(id) = name.to_str() else {
                                    continue;
                                };
                                if !matches!(mew_session::Meta::read(&dir, id).await,
                                    Ok(Some(meta)) if meta.group_id.as_deref() == Some(&group_id)
                                ) {
                                    continue;
                                }
                                let removed_group = group_id.clone();
                                match session_manager
                                    .update_metadata(id, move |meta| {
                                        if meta.group_id.as_deref() == Some(&removed_group) {
                                            meta.group_id = None;
                                        }
                                    })
                                    .await
                                {
                                    Ok(meta) if meta.group_id.is_none() => {
                                        ungrouped_sessions.push((
                                            id.to_owned(),
                                            meta.archived,
                                            meta.pinned,
                                        ));
                                    }
                                    Ok(_) => {}
                                    Err(error) => {
                                        tracing::warn!(session_id = id, %error, "could not clear deleted group")
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
                let stored_group = group_id.clone();
                let meta = match session_manager
                    .update_metadata(&session_id, move |meta| meta.group_id = stored_group)
                    .await
                {
                    Ok(meta) => meta,
                    Err(error) => {
                        reply(ServerMessage::Error {
                            message: format!("failed to save session group: {error}"),
                        });
                        continue;
                    }
                };
                match groups_store.assign_session(&session_id, group_id).await {
                    Ok(groups) => {
                        session_manager.broadcast_groups(groups).await;
                        session_manager
                            .broadcast_all(ServerMessage::SessionMetaChanged {
                                session_id,
                                archived: meta.archived,
                                pinned: meta.pinned,
                                group_id: meta.group_id,
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
            message @ (ClientMessage::ArchiveSession { .. } | ClientMessage::PinSession { .. }) => {
                let (session_id, archived, pinned) = match message {
                    ClientMessage::ArchiveSession {
                        session_id,
                        archived,
                    } => (session_id, Some(archived), None),
                    ClientMessage::PinSession { session_id, pinned } => {
                        (session_id, None, Some(pinned))
                    }
                    _ => unreachable!(),
                };
                match session_manager
                    .update_metadata(&session_id, move |meta| {
                        if let Some(archived) = archived {
                            meta.archived = archived;
                        }
                        if let Some(pinned) = pinned {
                            meta.pinned = pinned;
                        }
                    })
                    .await
                {
                    Ok(meta) => {
                        session_manager
                            .broadcast_all(ServerMessage::SessionMetaChanged {
                                session_id,
                                archived: meta.archived,
                                pinned: meta.pinned,
                                group_id: meta.group_id,
                            })
                            .await
                    }
                    Err(error) => reply(ServerMessage::Error {
                        message: format!("could not update session metadata: {error}"),
                    }),
                }
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
            ClientMessage::UpdateTodo {
                session_id,
                request_id,
                edit,
            } => {
                let result = match attached_session
                    .as_ref()
                    .filter(|session| session.id == session_id)
                {
                    Some(session) => session.update_todo(edit).await,
                    None => Err(anyhow::anyhow!("todo edits require the attached session")),
                };
                reply(match result {
                    Ok(todos) => ServerMessage::TodoUpdated {
                        session_id,
                        request_id,
                        todos,
                    },
                    Err(error) => ServerMessage::TodoUpdateError {
                        session_id,
                        request_id,
                        message: format!("{error:#}"),
                    },
                });
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
                    tokio::spawn(watch_workspace(Arc::downgrade(&session), root, task_cancel));
                    workspace_watch = Some(cancel);
                }
            }

            // -- Browser --
            ClientMessage::RegisterBrowserPage {
                session_id,
                page_instance,
                cdp_endpoint,
                target_id,
                focus_url,
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
                        focus_url,
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
    if let Some(monitor) = authorization_monitor {
        monitor.abort();
    }
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
    // Retain the transport until the peer acknowledges our close frame. In
    // particular, dropping an iroh connection can discard buffered stream data.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while let Some(Ok(message)) = ws_rx.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    })
    .await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Event forwarding: AgentEvent → ServerMessage
// ---------------------------------------------------------------------------

type WsSink<S> = futures::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>;

async fn daemon_settings_message(
    manager: &SessionManager,
    request_id: Option<String>,
    authorization: Option<RemoteAuthorization>,
) -> Result<ServerMessage> {
    #[cfg(test)]
    manager.settings_operation_started.notify_one();
    let defaults = manager.read_session_defaults().await?;
    let devices = if let Some(devices) = manager.remote_devices.get().cloned() {
        Some(tokio::task::spawn_blocking(move || devices.devices(remote::unix_now())).await??)
    } else {
        None
    };
    if let Some(authorization) = authorization {
        tokio::task::spawn_blocking(move || authorization.require_control()).await??;
    }
    Ok(ServerMessage::DaemonSettings {
        request_id,
        defaults,
        devices,
    })
}

async fn subagent_message_status(
    parent: Option<&Session>,
    child_session_id: &str,
    request_id: &str,
    text: Option<String>,
) -> (mew_protocol::PromptStatus, Option<String>) {
    use mew_protocol::PromptStatus::{Accepted, Rejected, Unknown};
    use mew_subagents::ChildGuidanceError;
    let Some(parent) = parent else {
        return (
            Rejected,
            Some("Attach the parent conversation to message its subagent.".into()),
        );
    };
    if request_id.is_empty() || request_id.len() > 128 {
        return (
            Rejected,
            Some("Invalid subagent message request ID.".into()),
        );
    }
    let child_root = match parent.subagent_transcript_root(child_session_id).await {
        Ok(root) => root,
        Err(error) => return (Rejected, Some(error.to_string())),
    };
    let sending = text.is_some();
    let (runner, parent_id) = {
        let agent = parent.agent.lock().await;
        (agent.subagent_runner.clone(), agent.session_id)
    };
    if let Some(runner) = runner {
        let child_id = ulid::Ulid::from_string(child_session_id).expect("validated child ID");
        let receipt = if let Some(text) = text {
            match runner
                .guide_child(parent_id, child_id, request_id, text)
                .await
            {
                Err(ChildGuidanceError::NotRunning) => {
                    runner.child_guidance_receipt(parent_id, child_id, request_id)
                }
                receipt => Some(receipt),
            }
        } else {
            runner.child_guidance_receipt(parent_id, child_id, request_id)
        };
        match receipt {
            Some(Ok(())) => return (Accepted, None),
            Some(Err(ChildGuidanceError::Rejected(error))) => return (Rejected, Some(error)),
            Some(Err(ChildGuidanceError::Unknown(error))) => return (Unknown, Some(error)),
            Some(Err(ChildGuidanceError::NotRunning)) | None => {}
        }
    }
    match mew_session::Reader::has_prompt_receipt(&child_root, child_session_id, request_id).await {
        Ok(true) => (Accepted, None),
        Ok(false) if sending => (Rejected, Some("This subagent is no longer running.".into())),
        Ok(false) => (
            Unknown,
            Some("The subagent message could not be confirmed.".into()),
        ),
        Err(error) => {
            tracing::warn!(session_id = %parent.id, %child_session_id, %error, "could not read subagent message receipt");
            (
                Unknown,
                Some("The subagent message could not be confirmed.".into()),
            )
        }
    }
}

/// Register cancellation before accepting work or waiting for the agent lock.
async fn register_turn(session: &Session) -> CancellationToken {
    let token = session
        .current_turn_cancel
        .lock()
        .await
        .get_or_insert_with(CancellationToken::new)
        .clone();
    *session.is_running.lock().await = true;
    token
}

enum PromptTurn {
    New {
        text: String,
        attachments: Vec<mew_protocol::Attachment>,
    },
    Accepted(mew_agent::AcceptedPrompt),
}

/// Complete the serialized turn before generating its optional title.
async fn finish_prompt_turn(
    session: &Arc<Session>,
    session_mgr: &Arc<SessionManager>,
    prompt_text: String,
    prompt: PromptTurn,
    auto_title: bool,
    guard: tokio::sync::MutexGuard<'_, ()>,
    token: CancellationToken,
) {
    let agent = session.agent.lock().await.clone();
    let had_error = run_turn(session, session_mgr, &agent, prompt, token.clone()).await;
    drop(guard);

    // Generate a session title from the first user message
    // if we haven't already. Uses a lightweight LLM call;
    // falls back to text truncation on error. Skipped if the
    // user has disabled auto-title generation.
    if auto_title && !had_error && !token.is_cancelled() {
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
            match session_mgr
                .update_metadata(&session.id, move |meta| {
                    meta.custom_title.get_or_insert(title);
                })
                .await
            {
                Ok(meta) => {
                    if let Some(title) = meta.custom_title {
                        session_mgr.broadcast_title(&session.id, title).await;
                    }
                }
                Err(error) => {
                    tracing::warn!(session_id = %session.id, %error, "could not save automatic title")
                }
            }
        }
    }
}

fn attachment_parts(attachments: Vec<mew_protocol::Attachment>) -> Vec<mew_message::Part> {
    attachments
        .into_iter()
        .map(|attachment| {
            let filename = attachment.path.rsplit('/').next().map(str::to_owned);
            mew_message::Part::File(mew_message::FilePart {
                base: mew_message::PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                mime: attachment.mime.unwrap_or_default(),
                filename,
                url: attachment.path,
            })
        })
        .collect()
}

/// Run and persist a turn, then publish its idle state and outcome.
async fn run_turn(
    session: &Arc<Session>,
    session_mgr: &Arc<SessionManager>,
    agent: &Agent,
    prompt: PromptTurn,
    token: CancellationToken,
) -> bool {
    session_mgr
        .broadcast_activity(&session.id, SessionState::Running)
        .await;

    let rx = match prompt {
        PromptTurn::New { text, attachments } => {
            agent.run_with_parts(text, attachment_parts(attachments), Some(token))
        }
        PromptTurn::Accepted(prompt) => agent.run_accepted_prompt(prompt, token),
    };
    let had_error = forward_events(rx, session.clone(), session_mgr.clone()).await;

    *session.current_turn_cancel.lock().await = None;
    *session.is_running.lock().await = false;

    let (usage_wire, context_tokens) = match session_mgr
        .update_metadata(&session.id, move |meta| {
            meta.last_turn_failed = had_error;
            if let Some(usage) = meta.usage.as_mut() {
                usage.add_turn();
            }
        })
        .await
    {
        Ok(meta) => (meta.usage.as_ref().map(Into::into), meta.context_tokens),
        Err(error) => {
            tracing::warn!(session_id = %session.id, %error, "could not save turn metadata");
            (None, None)
        }
    };

    // Broadcast idle state.
    session_mgr
        .broadcast_activity(&session.id, SessionState::Idle)
        .await;

    // Broadcast usage (only if we have data).
    if let Some(usage) = usage_wire {
        session_mgr
            .broadcast_all(ServerMessage::SessionUsageChanged {
                session_id: session.id.clone(),
                usage,
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
    session.provider_replay.lock().await.clear();
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
    enabled: watch::Sender<bool>,
) {
    use std::time::Duration;
    let idle_threshold = Duration::from_secs(600); // 10 minutes
    let check_interval = Duration::from_secs(300); // 5 minutes

    loop {
        tokio::time::sleep(check_interval).await;

        if !*enabled.borrow() {
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
                    let stored_summary = summary.clone();
                    match manager
                        .update_metadata(&id_clone, move |meta| meta.summary = Some(stored_summary))
                        .await
                    {
                        Ok(_) => {
                            manager
                                .broadcast_all(mew_protocol::ServerMessage::SessionSummaryChanged {
                                    session_id: id_clone,
                                    summary,
                                })
                                .await
                        }
                        Err(error) => {
                            tracing::warn!(session_id = %id_clone, %error, "could not save automatic summary")
                        }
                    }
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

fn wire_questions(questions: Vec<mew_agent::AskUserQuestion>) -> Vec<Question> {
    questions
        .into_iter()
        .map(|question| Question {
            prompt: question.prompt,
            options: question
                .options
                .into_iter()
                .map(|option| QuestionOption {
                    label: option.label,
                    description: option.description,
                })
                .collect(),
        })
        .collect()
}

/// Translate a single `AgentEvent` (owned) into zero or more `ServerMessage`s.
/// Channel-bearing events are converted to wire requests with fresh IDs;
/// the `oneshot::Sender` is stashed in the `Session` for later response.
async fn translate_event(
    event: AgentEvent,
    session: &Arc<Session>,
    session_mgr: &SessionManager,
) -> Vec<ServerMessage> {
    match event {
        AgentEvent::Provider(pe) => {
            // Intercept MessageEnd to accumulate usage on Meta and extract manifest.
            if let mew_provider::ProviderEvent::MessageEnd { usage, cost, .. } = &pe {
                let (usage, cost) = (*usage, *cost);
                if let Err(error) = session_mgr
                    .update_metadata(&session.id, move |meta| {
                        meta.usage
                            .get_or_insert_with(mew_session::SessionUsage::default)
                            .add_message(
                                usage.input as u64,
                                usage.output as u64,
                                usage.cache_read as u64,
                                usage.cache_write as u64,
                                cost,
                            );
                        // Keep the last real context reading if the provider did
                        // not report prompt usage on this response.
                        if usage.input > 0 {
                            meta.context_tokens = Some(usage.input as u64);
                        }
                    })
                    .await
                {
                    tracing::warn!(session_id = %session.id, %error, "could not save provider usage");
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
        AgentEvent::NestedToolStart {
            parent_call_id,
            call_id,
            tool_name,
            input,
        } => vec![ServerMessage::NestedToolStart {
            parent_call_id,
            call_id,
            tool_name,
            input,
        }],
        AgentEvent::NestedToolEnd {
            parent_call_id,
            call_id,
            success,
            error,
            output_bytes,
        } => vec![ServerMessage::NestedToolEnd {
            parent_call_id,
            call_id,
            success,
            error,
            output_bytes,
        }],
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
        AgentEvent::SubagentAskUser {
            source,
            call_id,
            questions,
            tx,
        } => {
            let request = ServerMessage::AskUserRequest {
                request_id: session.next_request_id(),
                subagent: Some(source),
                call_id,
                questions: wire_questions(questions),
            };
            publish_pending_request(session, session_mgr, &session.pending_ask_user, tx, request)
                .await
        }
        AgentEvent::AskUser {
            call_id,
            questions,
            tx,
        } => {
            let request = ServerMessage::AskUserRequest {
                subagent: None,
                request_id: session.next_request_id(),
                call_id,
                questions: wire_questions(questions),
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
            let _actions = session.pending_action_lock.lock().await;
            let mut pending = session.pending_ask_user.lock().await;
            let mut resolved = Vec::new();
            pending.retain(|request_id, request| {
                let belongs_to_task = matches!(&request.message,
                    ServerMessage::AskUserRequest { subagent: Some(source), .. }
                        if source.parent_call_id == parent_call_id
                );
                if belongs_to_task && request.sender.is_closed() {
                    resolved.push(request_id.clone());
                    false
                } else {
                    true
                }
            });
            drop(pending);
            if !resolved.is_empty() {
                for request_id in resolved {
                    session
                        .broadcast(ServerMessage::RequestResolved { request_id })
                        .await;
                }
                broadcast_attention(session, session_mgr).await;
            }
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
        AgentEvent::ChildGuidanceContinuation => Vec::new(),
        AgentEvent::TodosUpdated { .. } => {
            session.broadcast_todos().await;
            Vec::new()
        }
        AgentEvent::JobUpdate {
            job_id,
            command,
            state,
        } => {
            session.publish_job_update(job_id, command, state).await;
            Vec::new()
        }
        AgentEvent::FileDelta {
            path,
            added,
            removed,
        } => {
            match session_mgr
                .update_metadata(&session.id, move |meta| {
                    meta.change_stats
                        .get_or_insert_with(mew_session::ChangeStats::default)
                        .apply_delta(&path, added, removed);
                })
                .await
            {
                Ok(meta) => {
                    let stats = meta
                        .change_stats
                        .expect("file delta initialized change stats");
                    vec![ServerMessage::SessionStatsChanged {
                        session_id: session.id.clone(),
                        added: stats.added,
                        removed: stats.removed,
                        files_changed: stats.files.len() as u64,
                    }]
                }
                Err(error) => {
                    tracing::warn!(session_id = %session.id, %error, "could not save file changes");
                    Vec::new()
                }
            }
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
    msg: outbox::ClientFrame,
) -> Result<()> {
    let outbox::ClientFrame {
        json,
        budget: _budget,
    } = msg;
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
mod meta_tests;

#[cfg(test)]
mod prompt_tests;

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

    #[tokio::test]
    async fn history_page_reads_backwards_and_returns_a_cursor() {
        let messages = (0..5).map(test_message).collect::<Vec<_>>();
        let mut history = history::HistorySnapshot::new(
            None,
            Arc::default(),
            Arc::new(tokio::sync::Mutex::new(messages)),
        )
        .await;
        let (page, cursor) = history.page(usize::MAX, 2).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].time.created, 3);
        assert_eq!(page[1].time.created, 4);
        assert_eq!(cursor, Some(3));

        let (older, cursor) = history.page(cursor.unwrap(), 2).await.unwrap();
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
    fn todo_edits_require_remote_control_scope() {
        let message = ClientMessage::UpdateTodo {
            session_id: "session".into(),
            request_id: "edit".into(),
            edit: mew_protocol::TodoEdit::default(),
        };
        assert!(!remote_message_allowed(RemoteScope::Observe, &message));
        assert!(!remote_message_allowed(RemoteScope::Collaborate, &message));
        assert!(remote_message_allowed(RemoteScope::Control, &message));
    }

    #[test]
    fn child_history_is_available_to_each_remote_read_scope() {
        let message = ClientMessage::ReadSubagentHistory {
            session_id: "parent".into(),
            child_session_id: "child".into(),
            request_id: "history".into(),
        };
        for scope in [
            RemoteScope::Observe,
            RemoteScope::Collaborate,
            RemoteScope::Control,
        ] {
            assert!(remote_message_allowed(scope, &message));
        }
    }

    #[test]
    fn guiding_a_child_requires_remote_control_scope() {
        let guide = ClientMessage::GuideSubagent {
            session_id: "parent".into(),
            child_session_id: "child".into(),
            request_id: "guide".into(),
            text: "Review the tests.".into(),
        };
        let receipt = ClientMessage::ReadSubagentMessageReceipt {
            session_id: "parent".into(),
            child_session_id: "child".into(),
            request_id: "guide".into(),
        };
        for scope in [
            RemoteScope::Observe,
            RemoteScope::Collaborate,
            RemoteScope::Control,
        ] {
            assert_eq!(
                remote_message_allowed(scope, &guide),
                scope == RemoteScope::Control
            );
            assert!(remote_message_allowed(scope, &receipt));
        }
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
