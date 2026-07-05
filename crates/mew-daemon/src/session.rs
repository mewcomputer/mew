//! Session manager and shared session state for the daemon.
//!
//! A session is owned by the daemon, not by a connection. Multiple clients can
//! attach to the same session; events are broadcast to all attached clients.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use mew_agent::Agent;
use mew_hooks::PermissionDecision;
use mew_protocol::{ServerMessage, SessionInfo, SessionState};

use crate::{AgentBuildParams, AgentBuilder, ModelLister, ModelSwitcher};

/// Error returned when attaching to a session fails.
#[derive(Debug)]
pub enum AttachError {
    NotFound,
    NotTopLevel,
    BuildAgent(anyhow::Error),
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachError::NotFound => write!(f, "session not found"),
            AttachError::NotTopLevel => write!(f, "only top-level sessions can be attached"),
            AttachError::BuildAgent(e) => write!(f, "failed to build agent: {e}"),
        }
    }
}

impl std::error::Error for AttachError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AttachError::BuildAgent(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for AttachError {
    fn from(e: anyhow::Error) -> Self {
        AttachError::BuildAgent(e)
    }
}

/// One active session. Shared between all attached client connections.
pub struct Session {
    pub id: String,
    /// The single agent for this session. Cloned for each turn; mutated in
    /// place for model switches.
    pub agent: Mutex<Agent>,
    /// Ensures only one turn or model switch runs at a time.
    pub turn_lock: Mutex<()>,
    /// Broadcast targets: (client_id, sender, kind) per attached client.
    pub clients: Mutex<
        Vec<(
            u64,
            mpsc::UnboundedSender<ServerMessage>,
            mew_protocol::ClientKind,
        )>,
    >,
    /// Pending permission / workspace-permission / subagent-permission requests.
    pub pending_permissions: Mutex<HashMap<u64, PendingRequest<PermissionDecision>>>,
    /// Pending ask-user requests.
    pub pending_ask_user: Mutex<HashMap<u64, PendingRequest<Vec<String>>>>,
    /// Monotonically increasing IDs for both clients and permission requests.
    pub next_id: AtomicU64,
    /// Token for the turn currently in progress, if any.
    pub current_turn_cancel: Mutex<Option<CancellationToken>>,
    /// Current model/provider display IDs for SessionReady.
    pub model: Mutex<Option<String>>,
    pub provider: Mutex<Option<String>>,
    /// Whether a title has been generated for this session.
    pub title_generated: Mutex<bool>,
    /// True when a turn is in progress. Used for the session-rail running indicator.
    pub is_running: Mutex<bool>,
    /// Sessions root directory (for meta persistence).
    pub session_dir: PathBuf,
}

/// A request awaiting a client response. Keeps the wire payload so it can be
/// replayed to a client that attaches while the request is still outstanding,
/// alongside the channel that delivers the response back to the agent.
pub struct PendingRequest<T> {
    pub payload: mew_protocol::ServerMessage,
    pub responder: oneshot::Sender<T>,
}

impl Session {
    pub fn new(
        id: String,
        agent: Agent,
        model: Option<String>,
        provider: Option<String>,
        session_dir: PathBuf,
    ) -> Self {
        Self {
            id,
            agent: Mutex::new(agent),
            turn_lock: Mutex::new(()),
            clients: Mutex::new(Vec::new()),
            pending_permissions: Mutex::new(HashMap::new()),
            pending_ask_user: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            current_turn_cancel: Mutex::new(None),
            model: Mutex::new(model),
            provider: Mutex::new(provider),
            title_generated: Mutex::new(false),
            is_running: Mutex::new(false),
            session_dir,
        }
    }

    /// Get a display title for this session (custom title > summary > id).
    pub async fn display_title(&self) -> String {
        let agent = self.agent.lock().await;
        if let Some(meta) = agent.session_meta().await {
            if let Some(title) = &meta.custom_title {
                return title.clone();
            }
            if let Some(summary) = &meta.summary {
                return summary.clone();
            }
        }
        self.id.clone()
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn next_request_id(&self) -> u64 {
        self.next_id()
    }

    /// Attach a client sender. Returns (client_id, was_first_client).
    pub async fn attach_client(
        &self,
        sender: mpsc::UnboundedSender<ServerMessage>,
        client_kind: mew_protocol::ClientKind,
    ) -> (u64, bool) {
        let mut clients = self.clients.lock().await;
        let was_first = clients.is_empty();
        let client_id = self.next_id();
        clients.push((client_id, sender, client_kind));
        (client_id, was_first)
    }

    /// Detach a client by id. Returns true if this was the last client.
    pub async fn detach_client(&self, client_id: u64) -> bool {
        let mut clients = self.clients.lock().await;
        clients.retain(|(id, _, _)| *id != client_id);
        clients.is_empty()
    }

    pub async fn client_count(&self) -> usize {
        self.clients.lock().await.len()
    }

    /// Broadcast a message to all attached clients. Removes any sender that fails.
    pub async fn broadcast(&self, msg: ServerMessage) {
        let mut clients = self.clients.lock().await;
        clients.retain(|(_, sender, _)| sender.send(msg.clone()).is_ok());
    }

    /// Cancel the current turn, if any. Also drains pending requests so the
    /// agent loop unblocks if it was waiting on a permission/ask-user oneshot.
    pub async fn cancel_turn(&self) {
        if let Some(token) = self.current_turn_cancel.lock().await.take() {
            token.cancel();
        }
        self.drain_pending().await;
    }

    /// Drop all pending oneshot senders. Call when the last client detaches
    /// mid-turn so the agent loop unblocks.
    pub async fn drain_pending(&self) {
        let mut perms = self.pending_permissions.lock().await;
        perms.clear();
        let mut asks = self.pending_ask_user.lock().await;
        asks.clear();
    }
}

/// Owns all active sessions.
pub struct SessionManager {
    pub(crate) builder: AgentBuilder,
    pub(crate) switcher: Option<ModelSwitcher>,
    pub(crate) lister: Option<ModelLister>,
    pub(crate) session_dir: PathBuf,
    pub(crate) active: Mutex<HashMap<String, Arc<Session>>>,
    loading: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SessionManager {
    pub fn new(
        builder: AgentBuilder,
        session_dir: PathBuf,
        switcher: Option<ModelSwitcher>,
        lister: Option<ModelLister>,
    ) -> Self {
        Self {
            builder,
            switcher,
            lister,
            session_dir,
            active: Mutex::new(HashMap::new()),
            loading: Mutex::new(HashMap::new()),
        }
    }

    /// Create a brand-new session.
    pub async fn create(&self, cwd: Option<PathBuf>) -> Result<Arc<Session>> {
        let session_id = format!("sess_{}", ulid::Ulid::new());
        let mut meta = mew_session::Meta::new(&session_id);
        if let Some(ref c) = cwd {
            meta.cwd = Some(c.display().to_string());
        }
        let writer = mew_session::Writer::open_at_with_meta(&self.session_dir, &session_id, meta)
            .await
            .context("open session writer")?;
        let (agent, model, provider) = (self.builder)(AgentBuildParams {
            session_id: session_id.clone(),
            writer,
            cwd,
        })?;
        let session = Arc::new(Session::new(
            session_id.clone(),
            agent,
            model.clone(),
            provider.clone(),
            self.session_dir.clone(),
        ));
        self.active.lock().await.insert(session_id, session.clone());
        Ok(session)
    }

    /// Attach to an active session, or resume an idle session from disk.
    pub async fn attach(&self, session_id: &str) -> Result<Arc<Session>, AttachError> {
        // Fast path: already active.
        if let Some(session) = self.active.lock().await.get(session_id).cloned() {
            return Ok(session);
        }

        // Acquire a per-session load lock to prevent TOCTOU races.
        let load_lock = {
            let mut loading = self.loading.lock().await;
            loading
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = load_lock.lock().await;

        // Double-check after acquiring the lock.
        if let Some(session) = self.active.lock().await.get(session_id).cloned() {
            return Ok(session);
        }

        // Read meta to check depth and created_at.
        let meta = match mew_session::Meta::read(&self.session_dir, session_id).await {
            Ok(Some(m)) => m,
            Ok(None) => return Err(AttachError::NotFound),
            Err(e) => {
                tracing::warn!(session_id, error = %e, "failed to read session meta; treating as not found");
                return Err(AttachError::NotFound);
            }
        };

        if meta.depth != 0 {
            return Err(AttachError::NotTopLevel);
        }

        // Preserve cwd across resume — the session's cwd must survive
        // daemon restarts and eviction (plan: "Attach/resume passes meta.cwd").
        let session_cwd = meta.cwd.as_deref().map(std::path::PathBuf::from);

        let writer = mew_session::Writer::open_at_with_meta(&self.session_dir, session_id, meta)
            .await
            .context("open session writer for resume")
            .map_err(AttachError::BuildAgent)?;

        let (agent, model, provider) = (self.builder)(AgentBuildParams {
            session_id: session_id.to_string(),
            writer,
            cwd: session_cwd,
        })
        .context("build agent for resume")
        .map_err(AttachError::BuildAgent)?;

        let messages = mew_session::Reader::load_from(&self.session_dir, session_id)
            .await
            .context("load session history")
            .map_err(AttachError::BuildAgent)?;
        agent.load_messages(messages).await;

        let session = Arc::new(Session::new(
            session_id.to_string(),
            agent,
            model.clone(),
            provider.clone(),
            self.session_dir.clone(),
        ));
        self.active
            .lock()
            .await
            .insert(session_id.to_string(), session.clone());
        Ok(session)
    }

    /// Remove a session from active memory.
    pub async fn remove(&self, session_id: &str) {
        self.active.lock().await.remove(session_id);
        self.loading.lock().await.remove(session_id);
    }

    /// List active and idle top-level sessions.
    pub async fn list(&self) -> Vec<SessionInfo> {
        let mut infos = Vec::new();
        let active = self.active.lock().await;

        // Active sessions.
        for (id, session) in active.iter() {
            let agent = session.agent.lock().await;
            let meta = agent.session_meta().await;
            // Skip empty sessions (no messages yet).
            let message_count = agent.messages.try_lock().map(|m| m.len()).unwrap_or(0);
            if message_count == 0 {
                continue;
            }
            let (model, provider) = {
                (
                    session.model.lock().await.clone(),
                    session.provider.lock().await.clone(),
                )
            };
            let created_at = meta.as_ref().map(|m| m.created_at).unwrap_or(0);
            let last_message_at = meta
                .as_ref()
                .and_then(|m| m.last_message_at)
                .or_else(|| meta.as_ref().map(|m| m.created_at));
            let summary = meta.as_ref().and_then(|m| m.summary.clone());
            infos.push(SessionInfo {
                session_id: id.clone(),
                state: if *session.is_running.lock().await {
                    SessionState::Running
                } else {
                    SessionState::Active
                },
                model,
                provider,
                created_at,
                last_message_at,
                summary,
                client_count: session.client_count().await,
                cwd: meta.as_ref().and_then(|m| m.cwd.clone()),
                last_turn_failed: meta.as_ref().map(|m| m.last_turn_failed).unwrap_or(false),
                archived: meta.as_ref().map(|m| m.archived).unwrap_or(false),
                pinned: meta.as_ref().map(|m| m.pinned).unwrap_or(false),
                group_id: meta.as_ref().and_then(|m| m.group_id.clone()),
                change_stats: meta.as_ref().and_then(|m| m.change_stats.clone()),
                usage: meta.as_ref().and_then(|m| m.usage.as_ref().map(Into::into)),
                pending_permissions: session.pending_permissions.lock().await.len() as u32,
                pending_questions: session.pending_ask_user.lock().await.len() as u32,
            });
        }
        drop(active);

        // Idle sessions from disk.
        if let Ok(entries) = tokio::fs::read_dir(&self.session_dir).await {
            let mut entries = entries;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let id = match path.file_name().and_then(|s| s.to_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };
                if self.active.lock().await.contains_key(&id) {
                    continue;
                }
                match mew_session::Meta::read(&self.session_dir, &id).await {
                    Ok(Some(meta)) if meta.depth == 0 => {
                        // Auto-delete sessions with no messages.
                        let jsonl_path = path.join("session.jsonl");
                        let has_messages = match tokio::fs::metadata(&jsonl_path).await {
                            Ok(m) => m.len() > 0,
                            Err(_) => false,
                        };
                        if !has_messages {
                            tracing::debug!(session_id = %id, "auto-deleting empty session");
                            let _ = tokio::fs::remove_dir_all(&path).await;
                            continue;
                        }
                        infos.push(SessionInfo {
                            session_id: id.clone(),
                            state: SessionState::Idle,
                            model: meta.model.clone(),
                            provider: None,
                            created_at: meta.created_at,
                            last_message_at: meta.last_message_at.or(Some(meta.created_at)),
                            summary: meta.summary.clone(),
                            client_count: 0,
                            cwd: meta.cwd.clone(),
                            last_turn_failed: meta.last_turn_failed,
                            archived: meta.archived,
                            pinned: meta.pinned,
                            group_id: meta.group_id.clone(),
                            change_stats: meta.change_stats.clone(),
                            usage: meta.usage.as_ref().map(Into::into),
                            pending_permissions: 0,
                            pending_questions: 0,
                        });
                    }
                    _ => continue,
                }
            }
        }

        infos
    }

    /// Broadcast a title change for a session. If the session is active,
    /// sends to all attached clients. If only on disk, notifies via
    /// disk meta (frontend will pick it up on next list_sessions).
    pub async fn broadcast_title(&self, session_id: &str, title: String) {
        let active = self.active.lock().await;
        if let Some(session) = active.get(session_id) {
            session
                .broadcast(ServerMessage::SessionTitleChanged {
                    session_id: session_id.to_string(),
                    title,
                })
                .await;
        }
        // If the session is idle/on-disk only, the frontend will get the
        // title when it calls list_sessions (we don't return titles in
        // SessionInfo yet, so this is best-effort).
    }

    /// Broadcast a session activity change to all clients of that session
    /// and all other active sessions (for the rail).
    pub async fn broadcast_activity(&self, session_id: &str, activity: SessionState) {
        let active = self.active.lock().await;
        let msg = ServerMessage::SessionActivityChanged {
            session_id: session_id.to_string(),
            activity,
        };
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Broadcast a session stats change to all clients.
    pub async fn broadcast_stats(
        &self,
        session_id: &str,
        added: u64,
        removed: u64,
        files_changed: u64,
    ) {
        let active = self.active.lock().await;
        let msg = ServerMessage::SessionStatsChanged {
            session_id: session_id.to_string(),
            added,
            removed,
            files_changed,
        };
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Broadcast a groups-changed notification to all active sessions.
    pub async fn broadcast_groups(&self, groups: Vec<mew_protocol::GroupInfo>) {
        let active = self.active.lock().await;
        let msg = ServerMessage::GroupsChanged { groups };
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Broadcast a message to ALL active sessions' clients.
    /// Used for cross-session alerts (permission needed, turn complete, etc.)
    pub async fn broadcast_all(&self, msg: ServerMessage) {
        let active = self.active.lock().await;
        for session in active.values() {
            session.broadcast(msg.clone()).await;
        }
    }

    /// Get the cwd for a session (from active agent's meta or disk).
    pub async fn session_cwd(&self, session_id: &str) -> Option<PathBuf> {
        let active = self.active.lock().await;
        if let Some(session) = active.get(session_id) {
            let agent = session.agent.lock().await;
            let meta = agent.session_meta().await;
            drop(agent);
            drop(active);
            return meta.and_then(|m| m.cwd.map(PathBuf::from));
        }
        drop(active);
        match mew_session::Meta::read(&self.session_dir, session_id).await {
            Ok(Some(meta)) => meta.cwd.map(PathBuf::from),
            _ => None,
        }
    }
}
