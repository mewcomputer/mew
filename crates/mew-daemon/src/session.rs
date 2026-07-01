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
    pub pending_permissions: Mutex<HashMap<u64, oneshot::Sender<PermissionDecision>>>,
    /// Pending ask-user requests.
    pub pending_ask_user: Mutex<HashMap<u64, oneshot::Sender<Vec<String>>>>,
    /// Monotonically increasing IDs for both clients and permission requests.
    pub next_id: AtomicU64,
    /// Token for the turn currently in progress, if any.
    pub current_turn_cancel: Mutex<Option<CancellationToken>>,
    /// Current model/provider display IDs for SessionReady.
    pub model: Mutex<Option<String>>,
    pub provider: Mutex<Option<String>>,
    /// Whether a title has been generated for this session.
    pub title_generated: Mutex<bool>,
}

impl Session {
    pub fn new(id: String, agent: Agent, model: Option<String>, provider: Option<String>) -> Self {
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
        }
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
        let writer = mew_session::Writer::open_at(&self.session_dir, &session_id)
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

        let writer = mew_session::Writer::open_at_with_meta(&self.session_dir, session_id, meta)
            .await
            .context("open session writer for resume")
            .map_err(AttachError::BuildAgent)?;

        let (agent, model, provider) = (self.builder)(AgentBuildParams {
            session_id: session_id.to_string(),
            writer,
            cwd: None,
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
                state: SessionState::Active,
                model,
                provider,
                created_at,
                last_message_at,
                summary,
                client_count: session.client_count().await,
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
}
