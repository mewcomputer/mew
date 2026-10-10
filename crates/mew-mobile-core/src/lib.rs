//! mew-mobile-core: the Rust core for mobile clients (iOS/Android).
//!
//! Owns the iroh endpoint, manages per-daemon connections, decodes the
//! mew wire protocol, assembles session state, and emits typed events to
//! the platform layer via `CoreListener`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use iroh::{Endpoint, SecretKey};
use tokio::sync::mpsc;
use tokio_tungstenite::client_async_with_config;
use tokio_tungstenite::tungstenite::{
    client::ClientRequestBuilder, protocol::WebSocketConfig, Message,
};
use tracing::{info, warn};

use mew_message::{Part, ProviderEventWire};
use mew_protocol::{ClientMessage, ServerMessage};

pub mod codec;
pub mod events;
pub mod registry;
pub mod state;

pub use codec::decode_server_message_lenient;
pub use events::{CoreEvent, CoreListener, DaemonStatus, Decision, TodoItem};
pub use registry::{DaemonEntry, DaemonId, DaemonRegistry};
pub use state::{DaemonSnapshot, SessionState};

/// The ALPN used by the mew daemon's iroh listener.
pub const MEW_ALPN: &[u8] = b"mew/wire/0";

uniffi::setup_scaffolding!();

/// Error type for the mobile core.
#[derive(Debug, uniffi::Error)]
pub enum CoreError {
    InvalidSecretKey,
    EndpointBindFailed { reason: String },
    RegistryLoadFailed { reason: String },
    InvalidNodeId { reason: String },
    ParseFailed { reason: String },
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::InvalidSecretKey => write!(f, "invalid secret key (must be 32 bytes)"),
            CoreError::EndpointBindFailed { reason } => {
                write!(f, "failed to bind iroh endpoint: {reason}")
            }
            CoreError::RegistryLoadFailed { reason } => {
                write!(f, "failed to load daemon registry: {reason}")
            }
            CoreError::InvalidNodeId { reason } => write!(f, "invalid NodeId: {reason}"),
            CoreError::ParseFailed { reason } => write!(f, "parse failed: {reason}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// Parse a pairing payload into a dialable NodeId string.
///
/// Accepts:
/// - Raw NodeId (e.g. the string from `endpoint.id().to_string()`)
/// - `mew001:<node_id>` — versioned payload per the accounts plan
///
/// Rejects unknown version prefixes loudly. Returns the NodeId string
/// on success, or an error with a clear message.
///
/// Result of parsing a pairing payload.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DialInfo {
    pub node_id: String,
    pub name: Option<String>,
    pub pairing_token: Option<String>,
}

/// Per spec notes #2 and #11: keep dial-info parsing behind this one
/// function since it's the most likely thing to change post-stage-1.
#[uniffi::export]
pub fn parse_dial_info(payload: String) -> Result<DialInfo, CoreError> {
    parse_dial_info_impl(&payload)
        .map(|(node_id, name, pairing_token)| DialInfo {
            node_id,
            name,
            pairing_token,
        })
        .map_err(|e| CoreError::ParseFailed {
            reason: e.to_string(),
        })
}

fn parse_dial_info_impl(payload: &str) -> Result<(String, Option<String>, Option<String>)> {
    let payload = payload.trim();

    // URL-scheme format: computer.mew.mew://<node_id>
    // This is what `mew pair` puts in the QR code.
    if let Some(rest) = payload.strip_prefix("computer.mew.mew://") {
        if rest.is_empty() {
            return Err(anyhow::anyhow!("computer.mew.mew:// payload is empty"));
        }
        let (node_id, pairing_token) = rest
            .split_once("?token=")
            .map(|(node_id, token)| (node_id, Some(token.to_string())))
            .unwrap_or((rest, None));
        // Validate it's a real NodeId.
        iroh::PublicKey::from_str(node_id)
            .map_err(|e| anyhow::anyhow!("invalid NodeId in URL scheme: {e}"))?;
        return Ok((node_id.to_string(), None, pairing_token));
    }

    // Legacy versioned format: mew001:<node_id> or mew001:{json}
    if let Some(rest) = payload.strip_prefix("mew001:") {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(rest) {
            let node_id = json
                .get("node_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("mew001 payload missing node_id field"))?;
            let name = json.get("name").and_then(|v| v.as_str()).map(String::from);
            let token = json.get("token").and_then(|v| v.as_str()).map(String::from);
            return Ok((node_id.to_string(), name, token));
        }
        if rest.is_empty() {
            return Err(anyhow::anyhow!("mew001 payload is empty"));
        }
        return Ok((rest.to_string(), None, None));
    }

    // Raw NodeId — validate it parses as a PublicKey.
    iroh::PublicKey::from_str(payload)
        .map_err(|e| anyhow::anyhow!("invalid NodeId '{payload}': {e}"))?;
    Ok((payload.to_string(), None, None))
}

use std::str::FromStr;

/// One instance per app launch. Owns the iroh endpoint and manages
/// connections to multiple daemons.
#[derive(uniffi::Object)]
pub struct MobileCore {
    endpoint: Endpoint,
    registry: Mutex<DaemonRegistry>,
    connections: Mutex<HashMap<String, DaemonConnection>>,
    listener: Mutex<Option<Arc<dyn CoreListener>>>,
    /// Handle to the tokio runtime, captured during `new()`.
    /// Used by sync methods (connect, etc.) to spawn background tasks.
    runtime: tokio::runtime::Handle,
}

/// Shared connection state, accessible from both the background task
/// (for updates) and the foreground API (for snapshots).
struct ConnState {
    /// Session requested by the user, retained across reconnects.
    requested_session: Mutex<Option<String>>,
    /// Reject commands queued before a reconnect or attachment change.
    attachment_generation: AtomicU64,
    /// SessionReady confirms this generation, even when reattaching the same ID.
    confirmed_generation: AtomicU64,
    /// Accepted prompts awaiting daemon running activity or rejection.
    pending_submissions: Mutex<HashMap<String, u64>>,
    /// Framework-independent daemon/session state used by desktop and mobile
    /// adapters. The UniFFI projection remains alongside it during migration.
    client_state: Mutex<mew_client_core::ClientState>,
    /// Per-session state assembler, updated by the message loop.
    session_state: Mutex<Option<SessionState>>,
    /// Daemon version from last Pong.
    daemon_version: Mutex<Option<String>>,
    /// Available models from last ListModels.
    models: Mutex<Vec<state::ModelInfo>>,
    /// Session title (from SessionTitleChanged).
    session_title: Mutex<Option<String>>,
    /// Event listener, read per-event so set_listener() after connect works.
    listener: Mutex<Option<Arc<dyn CoreListener>>>,
    /// Current permission mode (lowercase id, e.g. "standard", "dangerous").
    permission_mode: Mutex<Option<String>>,
    /// Current model name.
    current_model: Mutex<Option<String>>,
    /// Current provider name.
    current_provider: Mutex<Option<String>>,
    /// Current thinking variant (None = thinking disabled).
    thinking_variant: Mutex<Option<String>>,
    /// Current persona name (None = default/no persona).
    current_persona: Mutex<Option<String>>,
    /// Available personas from last PersonaList.
    available_personas: Mutex<Vec<crate::events::PersonaInfo>>,
    #[cfg(test)]
    shared_events: Mutex<Vec<mew_client_core::ClientEvent>>,
}

impl ConnState {
    fn new() -> Self {
        Self {
            requested_session: Mutex::new(None),
            attachment_generation: AtomicU64::new(0),
            confirmed_generation: AtomicU64::new(u64::MAX),
            pending_submissions: Mutex::new(HashMap::new()),
            client_state: Mutex::new(mew_client_core::ClientState::default()),
            session_state: Mutex::new(None),
            daemon_version: Mutex::new(None),
            models: Mutex::new(Vec::new()),
            session_title: Mutex::new(None),
            listener: Mutex::new(None),
            permission_mode: Mutex::new(None),
            current_model: Mutex::new(None),
            current_provider: Mutex::new(None),
            thinking_variant: Mutex::new(None),
            current_persona: Mutex::new(None),
            available_personas: Mutex::new(Vec::new()),
            #[cfg(test)]
            shared_events: Mutex::new(Vec::new()),
        }
    }

    fn invalidate_session_commands(&self) {
        self.attachment_generation.fetch_add(1, Ordering::Relaxed);
        self.pending_submissions.lock().unwrap().clear();
    }

    fn release_submission(&self, session_id: &str, generation: u64) {
        let mut pending = self.pending_submissions.lock().unwrap();
        if pending.get(session_id) == Some(&generation) {
            pending.remove(session_id);
        }
    }

    fn push_shared_events(&self, events: impl IntoIterator<Item = mew_client_core::ClientEvent>) {
        #[cfg(test)]
        self.shared_events.lock().unwrap().extend(events);
        #[cfg(not(test))]
        drop(events);
    }

    fn push_shared_event(&self, event: mew_client_core::ClientEvent) {
        #[cfg(test)]
        self.shared_events.lock().unwrap().push(event);
        #[cfg(not(test))]
        drop(event);
    }

    #[cfg(test)]
    fn take_shared_events(&self) -> Vec<mew_client_core::ClientEvent> {
        std::mem::take(&mut *self.shared_events.lock().unwrap())
    }
}

/// Per-daemon connection state. Stored in the connections map keyed by
/// the daemon's node_id string.
struct DaemonConnection {
    /// Channel for sending ClientMessages to the background task, which
    /// forwards them over the WebSocket.
    tx: mpsc::UnboundedSender<QueuedCommand>,
    /// Shared state between the background task and the foreground API.
    state: Arc<ConnState>,
    task: tokio::task::JoinHandle<()>,
}

struct QueuedCommand {
    message: ClientMessage,
    session_id: Option<String>,
    generation: Option<u64>,
}

impl QueuedCommand {
    fn ready(&self, state: &ConnState) -> bool {
        if let Some(session_id) = &self.session_id {
            let client = state.client_state.lock().unwrap();
            let generation = state.attachment_generation.load(Ordering::Relaxed);
            if self.generation != Some(generation)
                || state.confirmed_generation.load(Ordering::Relaxed) != generation
            {
                return false;
            }
            return client.connection == Some(mew_client_core::ConnectionStatus::Connected)
                && client.attached_session.as_ref() == Some(session_id)
                && state.requested_session.lock().unwrap().as_ref() == Some(session_id);
        }
        if let ClientMessage::AttachSession { session_id, .. } = &self.message {
            return state.requested_session.lock().unwrap().as_ref() == Some(session_id);
        }
        true
    }
}

impl DaemonConnection {
    fn send(&self, message: ClientMessage) -> bool {
        let session_scoped = matches!(
            &message,
            ClientMessage::Cancel
                | ClientMessage::PermissionResponse { .. }
                | ClientMessage::AskUserResponse { .. }
                | ClientMessage::GoalResponse { .. }
                | ClientMessage::SwitchModel { .. }
                | ClientMessage::SetPermissionMode { .. }
                | ClientMessage::SlashCommand { .. }
                | ClientMessage::SetThinkingVariant { .. }
                | ClientMessage::ListPersonas
                | ClientMessage::SwitchPersona { .. }
                | ClientMessage::SetAutoTitle { .. }
                | ClientMessage::SetAutoSummary { .. }
        );
        let (session_id, generation) = if session_scoped {
            let client = self.state.client_state.lock().unwrap();
            let session_id = client.attached_session.clone();
            let generation = self.state.attachment_generation.load(Ordering::Relaxed);
            if self.state.confirmed_generation.load(Ordering::Relaxed) != generation
                || client.connection != Some(mew_client_core::ConnectionStatus::Connected)
                || session_id.is_none()
                || session_id != *self.state.requested_session.lock().unwrap()
            {
                return false;
            }
            (session_id, Some(generation))
        } else {
            (None, None)
        };
        self.tx
            .send(QueuedCommand {
                message,
                session_id,
                generation,
            })
            .is_ok()
    }
}

impl Drop for DaemonConnection {
    fn drop(&mut self) {
        *self.state.listener.lock().unwrap() = None;
        self.task.abort();
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl MobileCore {
    /// Create a new core with the given persistent secret key (32 bytes).
    /// The key should be loaded from the platform keychain by the Swift layer.
    /// This is async because it binds the iroh endpoint.
    #[uniffi::constructor]
    pub async fn new(secret_key_bytes: Vec<u8>, data_dir: String) -> Result<Self, CoreError> {
        let secret_key_bytes: [u8; 32] = secret_key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::InvalidSecretKey)?;
        let secret_key = SecretKey::from_bytes(&secret_key_bytes);
        let registry_path = PathBuf::from(data_dir).join("daemons.json");
        let registry =
            DaemonRegistry::load(registry_path).map_err(|e| CoreError::RegistryLoadFailed {
                reason: e.to_string(),
            })?;

        let endpoint = Endpoint::builder(iroh::endpoint::presets::N0)
            .secret_key(secret_key)
            .bind()
            .await
            .map_err(|e| CoreError::EndpointBindFailed {
                reason: e.to_string(),
            })?;

        // Capture the tokio runtime handle so sync methods can spawn tasks.
        let runtime = tokio::runtime::Handle::current();

        Ok(Self {
            endpoint,
            registry: Mutex::new(registry),
            connections: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
            runtime,
        })
    }

    /// This phone's NodeId (public key).
    pub fn node_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// Add a daemon to the registry.
    pub fn add_daemon(&self, node_id: String, name: String) -> DaemonId {
        self.registry.lock().unwrap().add(node_id, name)
    }

    /// Add a daemon from an invite payload and retain its pairing credential
    /// for reconnects.
    pub fn add_daemon_with_token(&self, node_id: String, name: String, token: String) -> DaemonId {
        self.registry
            .lock()
            .unwrap()
            .add_with_token(node_id, name, token)
    }

    /// Remove a daemon from the registry and disconnect.
    pub fn remove_daemon(&self, id: DaemonId) {
        self.registry.lock().unwrap().remove(&id);
        self.disconnect(id);
    }

    /// List all known daemons.
    pub fn list_daemons(&self) -> Vec<DaemonEntry> {
        self.registry.lock().unwrap().list()
    }

    /// Connect to a daemon. Connection progress is reported via events.
    pub fn connect(&self, id: DaemonId) {
        let entry = match self.registry.lock().unwrap().get(&id) {
            Some(e) => e.clone(),
            None => return,
        };

        let mut conns = self.connections.lock().unwrap();
        if conns.contains_key(&id.node_id) {
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel::<QueuedCommand>();
        let conn_state = Arc::new(ConnState::new());

        let endpoint = self.endpoint.clone();
        let daemon_id = id.node_id.clone();
        let node_id = entry.node_id.clone();
        let pairing_token = entry.pairing_token.clone();

        // Sync the listener into ConnState so the background task reads
        // it per-event (supports set_listener after connect).
        {
            let l = self.listener.lock().unwrap().clone();
            *conn_state.listener.lock().unwrap() = l;
        }

        // Spawn the background connection task.
        let conn_state_for_task = conn_state.clone();
        let task = self.runtime.spawn(async move {
            // Reconnect loop with exponential backoff (spec: 1s, 2s, 4s… cap 30s).
            let mut attempt: u32 = 0;
            let mut rx = rx;
            loop {
                let result = connect_and_run(
                    &endpoint,
                    &daemon_id,
                    &node_id,
                    pairing_token.as_deref(),
                    &mut rx,
                    &conn_state_for_task,
                )
                .await;

                // Resolve the outcome into either "stop" or a reason to surface.
                let error = match result {
                    Err(e) => {
                        let msg = e.to_string();
                        warn!(daemon = %daemon_id, error = %msg, attempt, "connection failed");
                        msg
                    }
                    Ok(ConnOutcome::UserDisconnected) => break,
                    Ok(ConnOutcome::Dropped { reason }) => reason,
                };

                // Exponential backoff with jitter.
                let base_secs = 1u64 << attempt.min(5); // 1, 2, 4, 8, 16, 32
                let capped = base_secs.min(30);
                let jitter = rand_jitter() % 500; // 0-499ms jitter
                let delay = Duration::from_millis(capped * 1000 + jitter);

                set_shared_connection_status(&conn_state_for_task, mew_client_core::ConnectionStatus::Backoff { attempt, error: error.clone() });

                if let Some(ref l) = *conn_state_for_task.listener.lock().unwrap() {
                    l.on_event(CoreEvent::DaemonStatusChanged {
                        daemon: daemon_id.clone(),
                        status: DaemonStatus::Backoff {
                            attempt,
                            error: error.clone(),
                        },
                    });
                }

                info!(daemon = %daemon_id, attempt, delay_ms = delay.as_millis(), "reconnecting after backoff");
                tokio::time::sleep(delay).await;
                attempt += 1;
            }

            conn_state_for_task.push_shared_event(
                conn_state_for_task
                    .client_state
                    .lock()
                    .unwrap()
                    .set_connection_status(mew_client_core::ConnectionStatus::Disconnected),
            );
            if let Some(ref l) = *conn_state_for_task.listener.lock().unwrap() {
                l.on_event(CoreEvent::DaemonStatusChanged {
                    daemon: daemon_id.clone(),
                    status: DaemonStatus::Disconnected,
                });
            }
        });
        conns.insert(
            id.node_id,
            DaemonConnection {
                tx,
                state: conn_state,
                task,
            },
        );
    }

    /// Disconnect from a daemon.
    pub fn disconnect(&self, id: DaemonId) {
        let removed = self.connections.lock().unwrap().remove(&id.node_id);
        if let Some(conn) = removed {
            set_shared_connection_status(
                &conn.state,
                mew_client_core::ConnectionStatus::Disconnected,
            );
            drop(conn);
            if let Some(listener) = self.listener.lock().unwrap().clone() {
                listener.on_event(CoreEvent::DaemonStatusChanged {
                    daemon: id.node_id,
                    status: DaemonStatus::Disconnected,
                });
            }
        }
    }

    /// Attach to a session on a daemon.
    pub fn attach(&self, id: DaemonId, session_id: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            conn.state.invalidate_session_commands();
            *conn.state.requested_session.lock().unwrap() = Some(session_id.clone());
            let _ = conn.send(ClientMessage::AttachSession {
                session_id: session_id.clone(),
                client_kind: mew_protocol::ClientKind::Mobile,
            });
        }
    }

    /// Queue a prompt only after the requested session is confirmed.
    /// False leaves the caller's composer and transcript unchanged.
    pub fn prompt(&self, id: DaemonId, text: String) -> bool {
        let conns = self.connections.lock().unwrap();
        let Some(conn) = conns.get(&id.node_id) else {
            return false;
        };
        let mut client = conn.state.client_state.lock().unwrap();
        let Some(session_id) = client.attached_session.clone() else {
            return false;
        };
        let generation = conn.state.attachment_generation.load(Ordering::Relaxed);
        if conn.state.confirmed_generation.load(Ordering::Relaxed) != generation
            || client.connection != Some(mew_client_core::ConnectionStatus::Connected)
            || client
                .session(&session_id)
                .is_some_and(|session| session.running)
            || conn.state.requested_session.lock().unwrap().as_deref() != Some(&session_id)
        {
            return false;
        }
        let mut pending = conn.state.pending_submissions.lock().unwrap();
        if pending.contains_key(&session_id) {
            return false;
        }
        if conn
            .tx
            .send(QueuedCommand {
                message: ClientMessage::Prompt {
                    text: text.clone(),
                    attachments: vec![],
                },
                session_id: Some(session_id.clone()),
                generation: Some(generation),
            })
            .is_err()
        {
            return false;
        }
        pending.insert(session_id.clone(), generation);
        drop(pending);
        client.record_prompt(&session_id, text);
        drop(client);
        sync_mobile_session_from_client_state(&conn.state);
        if let Some(listener) = conn.state.listener.lock().unwrap().clone() {
            listener.on_event(CoreEvent::SessionReloaded {
                daemon: id.node_id,
                session_id,
            });
        }
        true
    }

    /// Cancel the current turn on a daemon.
    pub fn cancel(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::Cancel);
        }
    }

    /// Respond to a permission request.
    pub fn respond_permission(&self, id: DaemonId, request_id: String, decision: Decision) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let wire_decision = match decision {
                Decision::AllowOnce => mew_protocol::PermissionDecision::AllowOnce,
                Decision::AllowSession => mew_protocol::PermissionDecision::AllowSession,
                Decision::Deny => mew_protocol::PermissionDecision::Deny,
            };
            let _ = conn.send(ClientMessage::PermissionResponse {
                request_id,
                decision: wire_decision,
            });
        }
    }

    /// Respond to an ask-user request.
    pub fn respond_ask_user(&self, id: DaemonId, request_id: String, answers: Vec<String>) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::AskUserResponse {
                request_id,
                answers,
                cancelled: false,
            });
        }
    }

    /// Close a question without cancelling the parent turn.
    pub fn cancel_ask_user(&self, id: DaemonId, request_id: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::AskUserResponse {
                request_id,
                answers: vec![],
                cancelled: true,
            });
        }
    }

    /// Respond to a goal proposal from the agent (`propose_goal`).
    pub fn respond_to_goal(&self, id: DaemonId, request_id: String, accepted: bool) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::GoalResponse {
                request_id,
                accepted,
            });
        }
    }

    /// List sessions on a daemon.
    pub fn list_sessions(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::ListSessions);
        }
    }

    /// Create a new session on a daemon.
    pub fn new_session(&self, id: DaemonId, cwd: Option<String>) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            conn.state.invalidate_session_commands();
            *conn.state.requested_session.lock().unwrap() = None;
            let _ = conn.send(ClientMessage::NewSession {
                cwd,
                client_kind: mew_protocol::ClientKind::Mobile,
            });
        }
    }

    /// Request the list of known projects (recent session cwds + workspace.roots).
    /// Response arrives as a `CoreEvent::ProjectList` event.
    pub fn list_projects(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::ListProjects);
        }
    }

    /// List files in a directory on the daemon. The result arrives as a
    /// `CoreEvent::DirListing` event.
    pub fn list_dir(&self, id: DaemonId, session_id: String, path: Option<String>) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::ListDir { session_id, path });
        }
    }

    /// List available models from the daemon.
    pub fn list_models(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::ListModels);
        }
    }

    /// Switch the active session to a different model.
    pub fn switch_model(&self, id: DaemonId, provider: String, model: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::SwitchModel { provider, model });
        }
    }

    /// Set the permission mode for the active session.
    pub fn set_permission_mode(&self, id: DaemonId, mode: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::SetPermissionMode { mode });
        }
    }

    /// Send a slash command to the daemon. Response comes as SlashResult event.
    pub fn slash_command(&self, id: DaemonId, command: String) -> bool {
        let conns = self.connections.lock().unwrap();
        let Some(conn) = conns.get(&id.node_id) else {
            return false;
        };
        conn.send(ClientMessage::SlashCommand { command })
    }

    /// Set the thinking variant for the active session.
    pub fn set_thinking_variant(&self, id: DaemonId, variant: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::SetThinkingVariant { variant });
        }
    }

    /// List available personas for the active session.
    /// Response arrives as `PersonaList` event.
    pub fn list_personas(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::ListPersonas);
        }
    }

    /// Switch the active session to a different persona.
    /// Confirmation arrives as `PersonaSwitched` event.
    pub fn switch_persona(&self, id: DaemonId, name: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::SwitchPersona { name });
        }
    }

    /// Enable or disable auto-generated session titles.
    pub fn set_auto_title(&self, id: DaemonId, enabled: bool) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::SetAutoTitle { enabled });
        }
    }

    /// Enable or disable idle session summaries.
    pub fn set_auto_summary(&self, id: DaemonId, enabled: bool) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::SetAutoSummary { enabled });
        }
    }

    /// Rename a session.
    pub fn rename_session(&self, id: DaemonId, session_id: String, title: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::RenameSession { session_id, title });
        }
    }

    /// Archive or unarchive a session.
    pub fn archive_session(&self, id: DaemonId, session_id: String, archived: bool) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::ArchiveSession {
                session_id,
                archived,
            });
        }
    }

    /// Pin or unpin a session.
    pub fn pin_session(&self, id: DaemonId, session_id: String, pinned: bool) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::PinSession { session_id, pinned });
        }
    }

    /// Delete a session from disk.
    pub fn delete_session(&self, id: DaemonId, session_id: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.send(ClientMessage::DeleteSession { session_id });
        }
    }

    /// Set the event listener. Events are delivered on the tokio runtime.
    /// Also updates existing connections so set_listener works after connect.
    pub fn set_listener(&self, listener: Arc<dyn CoreListener>) {
        *self.listener.lock().unwrap() = Some(listener.clone());
        // Push into all active connection states so the background tasks
        // pick up the new listener on the next event.
        let conns = self.connections.lock().unwrap();
        for conn in conns.values() {
            *conn.state.listener.lock().unwrap() = Some(listener.clone());
        }
    }

    /// Get a full snapshot of a daemon's state.
    pub fn snapshot(&self, id: DaemonId) -> Option<DaemonSnapshot> {
        let conns = self.connections.lock().unwrap();
        let conn = conns.get(&id.node_id)?;
        let attached = conn
            .state
            .client_state
            .lock()
            .unwrap()
            .attached_session
            .clone();
        let session_state = conn.state.session_state.lock().unwrap();
        let models = conn.state.models.lock().unwrap().clone();
        let daemon_version = conn.state.daemon_version.lock().unwrap().clone();
        let title = conn.state.session_title.lock().unwrap().clone();
        let permission_mode = conn.state.permission_mode.lock().unwrap().clone();
        let current_model = conn.state.current_model.lock().unwrap().clone();
        let current_provider = conn.state.current_provider.lock().unwrap().clone();
        let thinking_variant = conn.state.thinking_variant.lock().unwrap().clone();
        let current_persona = conn.state.current_persona.lock().unwrap().clone();
        let available_personas = conn.state.available_personas.lock().unwrap().clone();

        let mut snap = DaemonSnapshot {
            attached_session: attached.clone(),
            models,
            daemon_version,
            permission_mode,
            current_model,
            current_provider,
            thinking_variant,
            current_persona,
            available_personas,
            ..Default::default()
        };
        if let Some(ss) = session_state.as_ref() {
            snap.sessions.push(state::SessionInfo {
                session_id: ss.session_id.clone(),
                title: title.unwrap_or_default(),
                messages: ss.messages.clone(),
                running: ss.running,
                usage_cost: ss.usage_cost,
                pending_permissions: ss.pending_permissions,
                pending_questions: ss.pending_questions,
                input_tokens: ss.input_tokens,
                output_tokens: ss.output_tokens,
                turns: ss.turns,
                todos: ss.todos.clone(),
            });
            let client = conn.state.client_state.lock().unwrap();
            if let Some(session) = client.session(&ss.session_id) {
                for action in &session.pending_actions {
                    use mew_client_core::ActionKind;
                    let permission = match &action.kind {
                        ActionKind::Permission { tool_name, input } => Some((
                            tool_name.clone(),
                            serde_json::to_string_pretty(input).unwrap_or_default(),
                        )),
                        ActionKind::WorkspacePermission { path } => {
                            Some(("workspace_escape".into(), path.clone()))
                        }
                        ActionKind::SubagentPermission {
                            tool_name, input, ..
                        } => Some((
                            format!("subagent:{tool_name}"),
                            serde_json::to_string_pretty(input).unwrap_or_default(),
                        )),
                        ActionKind::AskUser {
                            call_id,
                            questions,
                            subagent,
                        } => {
                            snap.pending_ask_user.push(state::PendingAskUser {
                                subagent: subagent.as_ref().map(Into::into),
                                daemon: id.node_id.clone(),
                                request_id: action.request_id.clone(),
                                session_id: ss.session_id.clone(),
                                call_id: call_id.clone(),
                                questions: questions
                                    .iter()
                                    .map(|question| question.prompt.clone())
                                    .collect(),
                            });
                            None
                        }
                        ActionKind::GoalApproval { .. } => None,
                    };
                    if let Some((tool_name, input)) = permission {
                        snap.pending_permissions.push(state::PendingPermission {
                            daemon: id.node_id.clone(),
                            request_id: action.request_id.clone(),
                            session_id: ss.session_id.clone(),
                            tool_name,
                            input,
                        });
                    }
                }
            }
        }
        Some(snap)
    }
}

#[derive(Default)]
struct TextDeltaBuffer {
    origin: Option<(String, String)>,
    text: String,
}

impl TextDeltaBuffer {
    fn push(
        &mut self,
        daemon: &str,
        session_id: String,
        part_id: String,
        delta: String,
    ) -> Option<CoreEvent> {
        let same_origin = self
            .origin
            .as_ref()
            .is_some_and(|(session, part)| session == &session_id && part == &part_id);
        let previous = if same_origin { None } else { self.take(daemon) };
        if self.origin.is_none() {
            self.origin = Some((session_id, part_id));
        }
        self.text.push_str(&delta);
        previous
    }

    fn take(&mut self, daemon: &str) -> Option<CoreEvent> {
        let (session_id, part_id) = self.origin.take()?;
        if self.text.is_empty() {
            return None;
        }
        Some(CoreEvent::TextDelta {
            daemon: daemon.to_owned(),
            session_id,
            part_id,
            delta: std::mem::take(&mut self.text),
        })
    }
}

/// Outcome of a single `connect_and_run` attempt.
///
/// Carries the reason the connection ended so the reconnect loop can
/// surface it in the `Backoff` status event. `Err` is reserved for
/// failures to establish the connection (connect / open_bi / handshake);
/// mid-stream drops are `Ok(Dropped)` with a descriptive reason.
enum ConnOutcome {
    /// The user (or daemon-removal) closed our outbound channel — stop.
    UserDisconnected,
    /// The connection dropped mid-stream. Retry, surfacing `reason`.
    Dropped { reason: String },
}

fn set_shared_connection_status(
    conn_state: &Arc<ConnState>,
    status: mew_client_core::ConnectionStatus,
) {
    let mut client = conn_state.client_state.lock().unwrap();
    if status != mew_client_core::ConnectionStatus::Connected {
        conn_state.invalidate_session_commands();
        client.attached_session = None;
    }
    let event = client.set_connection_status(status);
    drop(client);
    conn_state.push_shared_event(event);
}

/// Verify compatibility before announcing a connection or attaching a session.
async fn verify_daemon_protocol<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
) -> anyhow::Result<ServerMessage>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(Duration::from_secs(10), async {
        ws.send(Message::Text(mew_protocol::encode_json(&ClientMessage::Ping)?)).await?;
        loop {
            let frame = ws.next().await.context("daemon closed during protocol handshake")??;
            match frame {
                Message::Text(text) => {
                    let message = mew_protocol::decode_json::<ServerMessage>(&text)
                        .context("invalid daemon protocol handshake")?;
                    match &message {
                        ServerMessage::Pong { protocol_version, .. } => {
                            anyhow::ensure!(
                                *protocol_version == mew_protocol::PROTOCOL_VERSION,
                                "incompatible daemon protocol version {protocol_version} (expected {}); update or restart the daemon and app",
                                mew_protocol::PROTOCOL_VERSION,
                            );
                            return Ok(message);
                        }
                        ServerMessage::RemoteReady { .. } => {}
                        ServerMessage::Error { message } => anyhow::bail!("daemon protocol handshake failed: {message}"),
                        _ => anyhow::bail!("unexpected daemon protocol handshake response"),
                    }
                }
                Message::Close(_) => anyhow::bail!("daemon closed during protocol handshake"),
                _ => {}
            }
        }
    }).await.context("daemon protocol handshake timed out")?
}

/// Connect to a daemon over iroh and run the message loop.
///
/// This function owns the WebSocket connection and processes incoming
/// `ServerMessage`s, translating them to `CoreEvent`s and updating
/// `SessionState` in the shared `ConnState`.
async fn connect_and_run(
    endpoint: &Endpoint,
    daemon_id: &str,
    node_id: &str,
    pairing_token: Option<&str>,
    rx: &mut mpsc::UnboundedReceiver<QueuedCommand>,
    conn_state: &Arc<ConnState>,
) -> Result<ConnOutcome> {
    let emit_event = |event: CoreEvent| {
        if let Some(ref l) = *conn_state.listener.lock().unwrap() {
            l.on_event(event);
        }
    };

    set_shared_connection_status(conn_state, mew_client_core::ConnectionStatus::Connecting);
    emit_event(CoreEvent::DaemonStatusChanged {
        daemon: daemon_id.to_string(),
        status: DaemonStatus::Connecting,
    });

    // Parse the node_id into a PublicKey for connecting.
    let public_key: iroh::PublicKey = node_id
        .parse()
        .context("parse daemon NodeId as PublicKey")?;
    let endpoint_addr = iroh::EndpointAddr::new(public_key);

    let conn = endpoint
        .connect(endpoint_addr, MEW_ALPN)
        .await
        .context("connect to daemon over iroh")?;

    // Open a bidirectional stream. Must write immediately (spec note #3).
    let (send_stream, recv_stream) = conn.open_bi().await.context("open bi stream")?;
    let iroh_stream = IrohStreamWrapper::new(send_stream, recv_stream);

    // WebSocket upgrade over the QUIC stream.
    let req = ClientRequestBuilder::new("ws://daemon.mew/".parse().unwrap())
        .with_header("Host", "daemon.mew")
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let config = WebSocketConfig {
        max_frame_size: None,
        ..WebSocketConfig::default()
    };
    let (mut ws, _resp) = client_async_with_config(req, iroh_stream, Some(config))
        .await
        .context("websocket handshake over iroh")?;

    // Iroh connections must complete the daemon's transport-level handshake
    // before sending normal protocol messages. Existing paired mobile nodes
    // authenticate through the daemon's trusted-node record, so they do not
    // need a user token here.
    let hello = mew_protocol::encode_json(&ClientMessage::RemoteHello {
        token: pairing_token.map(str::to_owned),
        device_name: "mew mobile".to_string(),
    })?;
    ws.send(Message::Text(hello)).await?;

    let pong = verify_daemon_protocol(&mut ws).await?;
    set_shared_connection_status(conn_state, mew_client_core::ConnectionStatus::Connected);
    emit_event(CoreEvent::DaemonStatusChanged {
        daemon: daemon_id.to_string(),
        status: DaemonStatus::Connected,
    });
    for event in translate_message(&pong, conn_state, &mut None, daemon_id) {
        emit_event(event);
    }

    // If we have a previously-attached session, re-send AttachSession so the
    // daemon replays the newest history page (spec: reconnect re-attach).
    let reattach = conn_state.requested_session.lock().unwrap().clone();
    if let Some(session_id) = reattach {
        let attach_msg = mew_protocol::encode_json(&ClientMessage::AttachSession {
            session_id,
            client_kind: mew_protocol::ClientKind::Mobile,
        })?;
        ws.send(Message::Text(attach_msg)).await?;
    }

    let mut last_sent_prompt: Option<String> = None;
    let mut user_disconnected = false;
    // Reason the connection dropped (set on every non-user break). Surfaced
    // via ConnOutcome::Dropped so the reconnect loop can show it in Backoff.
    let mut drop_reason: Option<String> = None;

    // TextDelta coalescing buffer (spec note #8).
    let mut delta_buffer = TextDeltaBuffer::default();
    let mut last_flush = tokio::time::Instant::now();
    const FLUSH_INTERVAL: Duration = Duration::from_millis(16);

    loop {
        tokio::select! {
            // Forward client messages to the daemon.
            msg = rx.recv() => {
                match msg {
                    Some(queued) => {
                        if !queued.ready(conn_state) {
                            if let (Some(session_id), Some(generation)) = (&queued.session_id, queued.generation) {
                                if matches!(queued.message, ClientMessage::Prompt { .. }) { conn_state.release_submission(session_id, generation); }
                            }
                            if let Some(session_id) = queued.session_id {
                                emit_event(CoreEvent::Alert { daemon: daemon_id.to_string(), session_id, kind: "error".into(), title: "Message was not sent".into(), detail: Some("The session connection changed. Reattach and retry.".into()) });
                            }
                            continue;
                        }
                        let client_msg = queued.message;
                        // Track last sent prompt for UserMessage dedup.
                        if let ClientMessage::Prompt { text, .. } = &client_msg {
                            last_sent_prompt = Some(text.clone());
                        }
                        let json = mew_protocol::encode_json(&client_msg)?;
                        if ws.send(Message::Text(json)).await.is_err() {
                            if let (Some(session_id), Some(generation)) = (&queued.session_id, queued.generation) {
                                if matches!(client_msg, ClientMessage::Prompt { .. }) { conn_state.release_submission(session_id, generation); }
                            }
                            if let Some(session_id) = queued.session_id {
                                emit_event(CoreEvent::Alert { daemon: daemon_id.to_string(), session_id, kind: "error".into(), title: "Message delivery interrupted".into(), detail: Some("Check the session history after reconnecting before retrying.".into()) });
                            }
                            drop_reason = Some("failed to send message to daemon".to_string());
                            break;
                        }
                    }
                    None => { user_disconnected = true; break; }, // Channel closed — daemon removed.
                }
            }
            // Receive server messages.
            msg = ws.next() => {
                let ws_msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        warn!(daemon = %daemon_id, error = %e, "ws error");
                        drop_reason = Some(format!("connection error: {e}"));
                        break;
                    }
                    None => {
                        info!(daemon = %daemon_id, "ws stream ended");
                        drop_reason = Some("connection closed".to_string());
                        break;
                    }
                };

                let text = match ws_msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => {
                        info!(daemon = %daemon_id, "connection closed by daemon");
                        drop_reason = Some("closed by daemon".to_string());
                        break;
                    }
                    _ => continue,
                };

                // Lenient decode (spec note #7).
                let server_msg = match decode_server_message_lenient(&text) {
                    Ok(Some(msg)) => msg,
                    Ok(None) => continue, // Dropped unknown frame.
                    Err(e) => {
                        warn!(daemon = %daemon_id, error = %e, "invalid JSON frame");
                        continue;
                    }
                };

                // Reduce ServerMessage into the shared client state first.
                // CoreEvent remains a compatibility projection for mobile.
                let events = translate_message(
                    &server_msg,
                    conn_state,
                    &mut last_sent_prompt,
                    daemon_id,
                );

                if !events.iter().any(|e| matches!(e, CoreEvent::TextDelta { .. })) {
                    if let Some(event) = delta_buffer.take(daemon_id) {
                        emit_event(event);
                    }
                }
                for event in events {
                    if let CoreEvent::TextDelta { session_id, part_id, delta, .. } = event {
                        if let Some(previous) = delta_buffer.push(daemon_id, session_id, part_id, delta) {
                            emit_event(previous);
                        }
                        if last_flush.elapsed() >= FLUSH_INTERVAL {
                            if let Some(event) = delta_buffer.take(daemon_id) {
                                emit_event(event);
                            }
                            last_flush = tokio::time::Instant::now();
                        }
                    } else {
                        emit_event(event);
                    }
                }
            }
        }
    }

    if let Some(event) = delta_buffer.take(daemon_id) {
        emit_event(event);
    }

    Ok(if user_disconnected {
        ConnOutcome::UserDisconnected
    } else {
        ConnOutcome::Dropped {
            reason: drop_reason.unwrap_or_else(|| "unknown reason".to_string()),
        }
    })
}

/// Translate a `ServerMessage` into zero or more `CoreEvent`s, updating
/// the shared `ConnState` as needed.
fn translate_message(
    msg: &ServerMessage,
    conn_state: &Arc<ConnState>,
    last_sent_prompt: &mut Option<String>,
    daemon_id: &str,
) -> Vec<CoreEvent> {
    use mew_protocol::ServerMessage;
    let d = daemon_id.to_string();
    let mut events = Vec::new();
    if matches!(msg, ServerMessage::SessionReady { .. }) {
        conn_state
            .confirmed_generation
            .store(u64::MAX, Ordering::Relaxed);
    }

    let shared_events = conn_state
        .client_state
        .lock()
        .unwrap()
        .apply_server_message(msg.clone());
    conn_state.push_shared_events(shared_events);
    match msg {
        ServerMessage::SessionActivityChanged {
            session_id,
            activity: mew_protocol::SessionState::Running,
        }
        | ServerMessage::SessionAlert {
            session_id,
            kind: mew_protocol::AlertKind::TurnComplete | mew_protocol::AlertKind::TurnFailed,
            ..
        } => {
            conn_state
                .pending_submissions
                .lock()
                .unwrap()
                .remove(session_id);
        }
        ServerMessage::Error { .. } | ServerMessage::ErrorEvent { .. } => {
            if let Some(session_id) = conn_state
                .client_state
                .lock()
                .unwrap()
                .attached_session
                .as_deref()
            {
                conn_state
                    .pending_submissions
                    .lock()
                    .unwrap()
                    .remove(session_id);
            }
        }
        _ => {}
    }

    // Project the canonical reduction before reading per-message metadata.
    if matches!(msg, ServerMessage::Provider { .. }) {
        sync_mobile_session_from_client_state(conn_state);
    }

    match msg {
        ServerMessage::Pong { version, .. } => {
            *conn_state.daemon_version.lock().unwrap() = Some(version.clone());
            events.push(CoreEvent::DaemonVersion {
                daemon: d,
                version: version.clone(),
            });
        }

        ServerMessage::SessionReady {
            session_id,
            model,
            provider,
            permission_mode,
            ..
        } => {
            let mut requested = conn_state.requested_session.lock().unwrap();
            if requested.is_none() {
                *requested = Some(session_id.clone());
            }
            drop(requested);
            conn_state
                .pending_submissions
                .lock()
                .unwrap()
                .remove(session_id);
            *conn_state.session_state.lock().unwrap() = Some(SessionState::new(session_id.clone()));
            // Capture current model/provider/permission_mode from SessionReady.
            *conn_state.current_model.lock().unwrap() = model.clone();
            *conn_state.current_provider.lock().unwrap() = provider.clone();
            *conn_state.permission_mode.lock().unwrap() = permission_mode.clone();
            if let Some(pm) = &permission_mode {
                if let Some(ss) = conn_state.session_state.lock().unwrap().as_mut() {
                    ss.permission_mode = Some(pm.clone());
                }
            }
            events.push(CoreEvent::SessionReady {
                daemon: d.clone(),
                session_id: session_id.clone(),
            });
            events.push(CoreEvent::SessionReloaded {
                daemon: d.clone(),
                session_id: session_id.clone(),
            });
            // Emit mode/model events so Swift seeds UI immediately.
            if let Some(mode) = permission_mode {
                events.push(CoreEvent::PermissionModeChanged {
                    daemon: d.clone(),
                    mode: mode.clone(),
                });
            }
            if let (Some(p), Some(m)) = (provider, model) {
                events.push(CoreEvent::ModelSwitched {
                    daemon: d.clone(),
                    provider: p.clone(),
                    model: m.clone(),
                });
            }
        }

        ServerMessage::SessionList { sessions } => {
            let summaries: Vec<_> = sessions
                .iter()
                .map(|s| events::SessionSummary {
                    session_id: s.session_id.clone(),
                    title: s.summary.clone().unwrap_or_else(|| s.session_id.clone()),
                    state: format!("{:?}", s.state).to_lowercase(),
                    archived: s.archived,
                    pinned: s.pinned,
                    pending_permissions: s.pending_permissions,
                    pending_questions: s.pending_questions,
                    usage_cost: s.usage.as_ref().map(|u| u.cost).unwrap_or(0.0),
                    cwd: s.cwd.clone(),
                    model: s.model.clone(),
                    provider: s.provider.clone(),
                    created_at: s.created_at,
                    last_message_at: s.last_message_at,
                    last_turn_failed: s.last_turn_failed,
                    group_id: s.group_id.clone(),
                    input_tokens: s.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
                    output_tokens: s.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
                    turns: s.usage.as_ref().map(|u| u.turns).unwrap_or(0),
                    context_tokens: s.context_tokens,
                })
                .collect();
            events.push(CoreEvent::SessionList {
                daemon: d,
                sessions: summaries,
            });
        }

        ServerMessage::ProjectList { projects } => {
            let infos: Vec<_> = projects
                .iter()
                .map(|p| events::ProjectInfo {
                    path: p.path.clone(),
                    display_name: p.display_name.clone(),
                    session_count: p.session_count,
                    last_used_at: p.last_used_at,
                })
                .collect();
            events.push(CoreEvent::ProjectList {
                daemon: d,
                projects: infos,
            });
        }

        ServerMessage::DirListing {
            session_id,
            path,
            entries,
        } => {
            let entries: Vec<events::DirEntry> = entries
                .iter()
                .map(|e| events::DirEntry {
                    name: e.name.clone(),
                    is_dir: e.is_dir,
                    size: e.size,
                })
                .collect();
            events.push(CoreEvent::DirListing {
                daemon: d,
                session_id: session_id.clone(),
                path: path.clone(),
                entries,
            });
        }

        ServerMessage::SessionHistory { .. } => {
            let session_id = conn_state
                .session_state
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.session_id.clone());
            if let Some(session_id) = session_id {
                events.push(CoreEvent::SessionReloaded {
                    daemon: d,
                    session_id,
                });
            }
        }

        ServerMessage::SessionHistoryPage { session_id, .. } => {
            let attached = conn_state
                .client_state
                .lock()
                .unwrap()
                .attached_session
                .clone();
            if attached.as_deref() == Some(session_id) {
                events.push(CoreEvent::SessionReloaded {
                    daemon: d,
                    session_id: session_id.clone(),
                });
            }
        }

        ServerMessage::SessionActivityChanged {
            session_id,
            activity,
        } => {
            events.push(CoreEvent::SessionActivityChanged {
                daemon: d,
                session_id: session_id.clone(),
                running: *activity == mew_protocol::SessionState::Running,
            });
        }

        ServerMessage::Provider { event } => {
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut() {
                match event {
                    ProviderEventWire::PartStart {
                        part: Part::Text(tp),
                    } => {
                        events.push(CoreEvent::TextDelta {
                            daemon: d,
                            session_id: ss.session_id.clone(),
                            part_id: tp.base.id.to_string(),
                            delta: tp.text.clone(),
                        });
                    }
                    ProviderEventWire::PartDelta { part_id, delta, .. } => {
                        events.push(CoreEvent::TextDelta {
                            daemon: d,
                            session_id: ss.session_id.clone(),
                            part_id: part_id.to_string(),
                            delta: delta.clone(),
                        });
                    }
                    ProviderEventWire::PartEnd { part_id } => {
                        events.push(CoreEvent::PartUpdated {
                            daemon: d,
                            session_id: ss.session_id.clone(),
                            part_id: part_id.to_string(),
                            part_kind: "text".into(),
                            state: Some("completed".into()),
                        });
                    }
                    ProviderEventWire::MessageEnd {
                        usage,
                        cost,
                        finish,
                        ..
                    } => {
                        let failed = *finish == mew_message::Finish::Error;
                        let manifest = ss.last_manifest.take();
                        events.push(CoreEvent::ProviderMessageEnded {
                            daemon: d,
                            session_id: ss.session_id.clone(),
                            input_tokens: usage.input as u64,
                            output_tokens: usage.output as u64,
                            cost: *cost,
                            failed,
                            manifest,
                        });
                    }
                    _ => {}
                }
            }
        }

        ServerMessage::PartUpdated { part_id, part } => {
            // Authoritative replacement (spec note #10).
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut() {
                let pid = part_id.to_string();
                for msg in ss.messages.iter_mut() {
                    if let Some(p) = msg.parts.iter_mut().find(|p| p.id == pid) {
                        match part {
                            Part::Text(tp) => {
                                p.text = Some(tp.text.clone());
                                p.kind = state::PartKind::Text;
                            }
                            Part::ToolCall(tcp) => {
                                let (input_str, output, error, time_start, time_end) =
                                    state::tool_state_fields(&tcp.state);
                                p.tool_name = Some(tcp.tool_name.clone());
                                p.tool_state = Some(format!("{:?}", tcp.state).to_lowercase());
                                p.tool_input = input_str;
                                p.tool_output = output;
                                p.tool_error = error;
                                p.tool_call_id = Some(tcp.call_id.clone());
                                p.tool_time_start = time_start;
                                p.tool_time_end = time_end;
                                p.kind = state::PartKind::ToolCall;
                            }
                            Part::Reasoning(rp) => {
                                p.text = Some(rp.text.clone());
                                p.kind = state::PartKind::Reasoning;
                            }
                            _ => {}
                        }
                        break;
                    }
                }
                events.push(CoreEvent::PartUpdated {
                    daemon: d,
                    session_id: ss.session_id.clone(),
                    part_id: pid,
                    part_kind: match part {
                        Part::Text(_) => "text",
                        Part::ToolCall(_) => "tool_call",
                        Part::Reasoning(_) => "reasoning",
                        _ => "other",
                    }
                    .into(),
                    state: None,
                });
            }
        }

        ServerMessage::UserMessage { text, .. } => {
            // Dedup: daemon echoes our prompt back (spec note #9).
            if let Some(ref last) = last_sent_prompt {
                if last == text {
                    *last_sent_prompt = None;
                    return events; // Drop the echo.
                }
            }
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut() {
                ss.messages.push(state::ChatMessage {
                    id: ulid::Ulid::new().to_string(),
                    role: "user".into(),
                    parts: vec![state::MessagePart {
                        id: ulid::Ulid::new().to_string(),
                        kind: state::PartKind::Text,
                        text: Some(text.clone()),
                        tool_name: None,
                        tool_state: None,
                        tool_input: None,
                        tool_output: None,
                        tool_error: None,
                        tool_call_id: None,
                        tool_time_start: None,
                        tool_time_end: None,
                        tool_sensitivity: None,
                    }],
                    assistant_meta: None,
                });
            }
        }

        ServerMessage::PermissionRequest {
            request_id,
            tool_name,
            input,
        } => {
            let input_str = serde_json::to_string_pretty(input).unwrap_or_default();
            let ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            events.push(CoreEvent::PermissionRequested {
                daemon: d,
                session_id,
                request_id: request_id.clone(),
                tool_name: tool_name.clone(),
                input: input_str,
            });
        }

        ServerMessage::AskUserRequest {
            request_id,
            call_id,
            questions,
            subagent,
        } => {
            let ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            events.push(CoreEvent::AskUserRequested {
                subagent: subagent.as_ref().map(Into::into),
                daemon: d,
                session_id,
                request_id: request_id.clone(),
                call_id: call_id.clone(),
                questions: questions.iter().map(|q| q.prompt.clone()).collect(),
            });
        }

        ServerMessage::GoalProposed {
            request_id,
            call_id,
            objective,
        } => {
            let ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            events.push(CoreEvent::GoalProposed {
                daemon: d,
                session_id,
                request_id: request_id.clone(),
                call_id: call_id.clone(),
                objective: objective.clone(),
            });
        }

        ServerMessage::RequestResolved { request_id } => {
            events.push(CoreEvent::RequestResolved {
                daemon: d,
                request_id: request_id.clone(),
            });
        }

        ServerMessage::SessionAlert {
            session_id,
            title,
            kind,
            detail,
        } => {
            events.push(CoreEvent::Alert {
                daemon: d,
                session_id: session_id.clone(),
                kind: format!("{:?}", kind).to_lowercase(),
                title: title.clone(),
                detail: detail.clone(),
            });
        }

        ServerMessage::SessionAttentionChanged {
            session_id,
            pending_permissions,
            pending_questions,
        } => {
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut().filter(|ss| ss.session_id == *session_id) {
                ss.pending_permissions = *pending_permissions;
                ss.pending_questions = *pending_questions;
            }
            events.push(CoreEvent::AttentionChanged {
                daemon: d,
                session_id: session_id.clone(),
                pending_permissions: *pending_permissions,
                pending_questions: *pending_questions,
            });
        }

        ServerMessage::ModelList { models } => {
            let summaries = models
                .iter()
                .map(|m| events::ModelSummary {
                    id: m.id.clone(),
                    provider: m.provider.clone(),
                    model: m.model.clone(),
                    description: m.description.clone(),
                    context_window: m.context_window,
                    thinking_variants: m.thinking_variants.iter().map(|v| v.name.clone()).collect(),
                })
                .collect();
            // Store models in shared state for snapshot.
            *conn_state.models.lock().unwrap() = models
                .iter()
                .map(|m| state::ModelInfo {
                    id: m.id.clone(),
                    provider: m.provider.clone(),
                    model: m.model.clone(),
                    context_window: m.context_window,
                    thinking_variants: m.thinking_variants.iter().map(|v| v.name.clone()).collect(),
                })
                .collect();
            events.push(CoreEvent::ModelList {
                daemon: d,
                models: summaries,
            });
        }

        ServerMessage::SlashResult { text } => {
            let session_id = conn_state
                .session_state
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            events.push(CoreEvent::SlashResult {
                daemon: d,
                session_id,
                text: text.clone(),
            });
        }

        ServerMessage::SessionCleared => {
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut() {
                ss.messages.clear();
            }
        }

        ServerMessage::SessionTitleChanged { session_id, title } => {
            *conn_state.session_title.lock().unwrap() = Some(title.clone());
            // SessionList doesn't carry titles per-session in real-time,
            // so this event is the authority for the attached session.
            let _ = session_id; // available if needed for multi-session later
        }

        ServerMessage::TodosUpdated { todos } => {
            let session_id = conn_state
                .session_state
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();

            // Map protocol Todo → mobile-core TodoItem (usize → u64).
            let todo_items: Vec<events::TodoItem> = todos
                .iter()
                .map(|t| events::TodoItem {
                    id: t.id as u64,
                    content: t.content.clone(),
                    status: t.status.clone(),
                    depends_on: t.depends_on.iter().map(|&d| d as u64).collect(),
                })
                .collect();

            // Store in SessionState.
            if let Some(ss) = conn_state.session_state.lock().unwrap().as_mut() {
                ss.todos = todo_items.clone();
            }

            events.push(CoreEvent::TodosUpdated {
                daemon: d,
                session_id,
                todos: todo_items,
            });
        }

        // Pass through events that need permission handling but no state assembly.
        ServerMessage::WorkspacePermissionRequest { request_id, path } => {
            let ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            events.push(CoreEvent::PermissionRequested {
                daemon: d,
                session_id,
                request_id: request_id.clone(),
                tool_name: "workspace_escape".into(),
                input: path.clone(),
            });
        }

        ServerMessage::SubagentPermissionRequest {
            request_id,
            tool_name,
            input,
            ..
        } => {
            let ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            let input_str = serde_json::to_string_pretty(input).unwrap_or_default();
            events.push(CoreEvent::PermissionRequested {
                daemon: d,
                session_id,
                request_id: request_id.clone(),
                tool_name: format!("subagent:{tool_name}"),
                input: input_str,
            });
        }

        ServerMessage::PermissionModeChanged { mode } => {
            *conn_state.permission_mode.lock().unwrap() = Some(mode.clone());
            if let Some(ss) = conn_state.session_state.lock().unwrap().as_mut() {
                ss.permission_mode = Some(mode.clone());
            }
            events.push(CoreEvent::PermissionModeChanged {
                daemon: d,
                mode: mode.clone(),
            });
        }

        ServerMessage::ModelSwitched { provider, model } => {
            *conn_state.current_model.lock().unwrap() = Some(model.clone());
            *conn_state.current_provider.lock().unwrap() = Some(provider.clone());
            events.push(CoreEvent::ModelSwitched {
                daemon: d,
                provider: provider.clone(),
                model: model.clone(),
            });
        }

        ServerMessage::ThinkingVariantChanged { variant } => {
            *conn_state.thinking_variant.lock().unwrap() = variant.clone();
            if let Some(ss) = conn_state.session_state.lock().unwrap().as_mut() {
                ss.thinking_variant = variant.clone();
            }
            events.push(CoreEvent::ThinkingVariantChanged {
                daemon: d,
                variant: variant.clone(),
            });
        }

        // Explicit arms for unhandled variants — no silent catch-all.
        // New ServerMessage variants will cause a compile error here,
        // forcing a decision about how to handle them.
        ServerMessage::Error { message } => {
            events.push(CoreEvent::Alert {
                daemon: d.clone(),
                session_id: conn_state
                    .client_state
                    .lock()
                    .unwrap()
                    .attached_session
                    .clone()
                    .unwrap_or_default(),
                kind: "error".into(),
                title: "Error".into(),
                detail: Some(message.clone()),
            });
        }
        ServerMessage::ErrorEvent { message } => {
            events.push(CoreEvent::Alert {
                daemon: d.clone(),
                session_id: conn_state
                    .client_state
                    .lock()
                    .unwrap()
                    .attached_session
                    .clone()
                    .unwrap_or_default(),
                kind: "error".into(),
                title: "Error".into(),
                detail: Some(message.clone()),
            });
        }

        ServerMessage::PersonaList { personas } => {
            let mapped: Vec<crate::events::PersonaInfo> = personas
                .iter()
                .map(|p| crate::events::PersonaInfo {
                    name: p.name.clone(),
                    description: p.description.clone(),
                    color: p.color.clone(),
                    active: p.active,
                })
                .collect();
            *conn_state.available_personas.lock().unwrap() = mapped.clone();
            // Sync current_persona from the server's active flag — the
            // daemon knows which persona is active, and PersonaList may
            // arrive before any PersonaSwitched event.
            let active_persona = mapped.iter().find(|p| p.active).map(|p| p.name.clone());
            *conn_state.current_persona.lock().unwrap() = active_persona.clone();
            if let Some(ss) = conn_state.session_state.lock().unwrap().as_mut() {
                ss.available_personas = mapped.clone();
                ss.current_persona = active_persona;
            }
            events.push(CoreEvent::PersonaList {
                daemon: d.clone(),
                personas: mapped,
            });
        }

        ServerMessage::PersonaSwitched { name } => {
            *conn_state.current_persona.lock().unwrap() = Some(name.clone());
            if let Some(ss) = conn_state.session_state.lock().unwrap().as_mut() {
                ss.current_persona = Some(name.clone());
            }
            events.push(CoreEvent::PersonaSwitched {
                daemon: d.clone(),
                name: name.clone(),
            });
        }

        // These variants are not yet translated to CoreEvent — they are
        // intentionally no-ops for now. Adding a new ServerMessage variant
        // here forces a compile-time decision.
        ServerMessage::SubagentEnd {
            parent_call_id,
            child_session_id,
            outcome,
            manifests,
        } => {
            let outcome_str = match outcome {
                mew_protocol::SubagentOutcome::Completed => "completed",
                mew_protocol::SubagentOutcome::Cancelled => "cancelled",
                mew_protocol::SubagentOutcome::Failed { .. } => "failed",
            };
            let mobile_manifests = manifests
                .iter()
                .map(crate::state::to_mobile_manifest)
                .collect();
            events.push(CoreEvent::SubagentEnd {
                daemon: d,
                parent_call_id: parent_call_id.clone(),
                child_session_id: child_session_id.clone(),
                outcome: outcome_str.to_string(),
                manifests: mobile_manifests,
            });
        }
        ServerMessage::ToolStart { .. }
        | ServerMessage::ToolEnd { .. }
        | ServerMessage::ToolProgress { .. }
        | ServerMessage::SubagentStart { .. }
        | ServerMessage::SubagentStatus { .. }
        | ServerMessage::JobUpdate { .. }
        | ServerMessage::SessionActivitySnapshot { .. }
        | ServerMessage::ClientAttached { .. }
        | ServerMessage::ClientDetached { .. }
        | ServerMessage::ControlYielded { .. }
        | ServerMessage::SessionSummaryChanged { .. }
        | ServerMessage::SessionStatsChanged { .. }
        | ServerMessage::SessionUsageChanged { .. }
        | ServerMessage::GroupList { .. }
        | ServerMessage::GroupsChanged { .. }
        | ServerMessage::FilePreview { .. }
        | ServerMessage::FilesystemDirListing { .. }
        | ServerMessage::GitStatusResult { .. }
        | ServerMessage::SessionDiff { .. }
        | ServerMessage::SessionDiffError { .. }
        | ServerMessage::SubagentHistory { .. }
        | ServerMessage::JobOutput { .. }
        | ServerMessage::JobOutputError { .. }
        | ServerMessage::TodoUpdated { .. }
        | ServerMessage::TodoUpdateError { .. }
        | ServerMessage::BrowserPageRegistration { .. }
        | ServerMessage::FsChanged { .. }
        | ServerMessage::SessionMetaChanged { .. }
        | ServerMessage::FlaggedFilesChanged { .. }
        | ServerMessage::RemoteReady { .. }
        | ServerMessage::BrowserSnapshot { .. }
        | ServerMessage::BrowserScreenshot { .. }
        | ServerMessage::BrowserState { .. }
        | ServerMessage::BrowserError { .. }
        | ServerMessage::TerminalOpened { .. }
        | ServerMessage::TerminalOutput { .. }
        | ServerMessage::TerminalExited { .. }
        | ServerMessage::TerminalError { .. }
        | ServerMessage::PromptReceipt { .. } => {}
    }

    if !matches!(msg, ServerMessage::Provider { .. }) {
        sync_mobile_session_from_client_state(conn_state);
    }
    if matches!(msg, ServerMessage::SessionReady { .. }) {
        conn_state.confirmed_generation.store(
            conn_state.attachment_generation.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
    events
}

fn sync_mobile_session_from_client_state(conn_state: &Arc<ConnState>) {
    let Some(session_id) = conn_state
        .client_state
        .lock()
        .unwrap()
        .attached_session
        .clone()
    else {
        return;
    };
    let shared = conn_state
        .client_state
        .lock()
        .unwrap()
        .session(&session_id)
        .cloned();
    let Some(shared) = shared else {
        return;
    };
    if let Some(session) = conn_state.session_state.lock().unwrap().as_mut() {
        session.shared = shared;
        session.sync_from_shared();
    }
}

/// Minimal stream wrapper for iroh QUIC streams.
struct IrohStreamWrapper {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
}

impl IrohStreamWrapper {
    fn new(send: iroh::endpoint::SendStream, recv: iroh::endpoint::RecvStream) -> Self {
        Self { send, recv }
    }
}

impl tokio::io::AsyncRead for IrohStreamWrapper {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        tokio::io::AsyncRead::poll_read(std::pin::Pin::new(&mut this.recv), cx, buf)
    }
}

impl tokio::io::AsyncWrite for IrohStreamWrapper {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(&mut this.send), cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        tokio::io::AsyncWrite::poll_flush(std::pin::Pin::new(&mut this.send), cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        tokio::io::AsyncWrite::poll_shutdown(std::pin::Pin::new(&mut this.send), cx)
    }
}

/// Simple jitter for backoff — uses process time as entropy.
/// Not cryptographically secure, but fine for connection retry timing.
fn rand_jitter() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_action_frames_leave_attention_counts_authoritative() {
        let conn = Arc::new(ConnState::new());
        let daemon = "node-abc".to_owned();
        let mut prompt = None;
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        translate_message(
            &ServerMessage::SessionReady {
                session_id: id.into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        let permission = ServerMessage::PermissionRequest {
            request_id: "permission".into(),
            tool_name: "shell".into(),
            input: serde_json::json!({}),
        };
        translate_message(&permission, &conn, &mut prompt, &daemon);
        translate_message(&permission, &conn, &mut prompt, &daemon);
        for request in [
            ServerMessage::WorkspacePermissionRequest {
                request_id: "workspace".into(),
                path: "/project".into(),
            },
            ServerMessage::SubagentPermissionRequest {
                request_id: "subagent".into(),
                parent_call_id: "parent".into(),
                tool_name: "shell".into(),
                input: serde_json::json!({}),
            },
            ServerMessage::AskUserRequest {
                subagent: None,
                request_id: "ask".into(),
                call_id: "ask-call".into(),
                questions: vec![],
            },
            ServerMessage::GoalProposed {
                request_id: "goal".into(),
                call_id: "goal-call".into(),
                objective: "finish".into(),
            },
        ] {
            translate_message(&request, &conn, &mut prompt, &daemon);
            translate_message(&request, &conn, &mut prompt, &daemon);
        }
        assert_eq!(
            conn.session_state
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .pending_permissions,
            0
        );
        assert_eq!(
            conn.session_state
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .pending_questions,
            0
        );
        translate_message(
            &ServerMessage::SessionAttentionChanged {
                session_id: id.into(),
                pending_permissions: 1,
                pending_questions: 2,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        translate_message(
            &ServerMessage::SessionAttentionChanged {
                session_id: "other".into(),
                pending_permissions: 7,
                pending_questions: 8,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        translate_message(
            &ServerMessage::RequestResolved {
                request_id: "permission".into(),
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        let guard = conn.session_state.lock().unwrap();
        let session = guard.as_ref().unwrap();
        assert_eq!(session.pending_permissions, 1);
        assert_eq!(session.pending_questions, 2);
    }

    async fn connected_core() -> (
        MobileCore,
        DaemonId,
        Arc<ConnState>,
        mpsc::UnboundedReceiver<QueuedCommand>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let core = MobileCore::new(vec![7; 32], dir.path().to_str().unwrap().into())
            .await
            .unwrap();
        let id = DaemonId {
            node_id: core.node_id(),
        };
        let state = Arc::new(ConnState::new());
        state
            .client_state
            .lock()
            .unwrap()
            .set_connection_status(mew_client_core::ConnectionStatus::Connected);
        let (tx, rx) = mpsc::unbounded_channel();
        core.connections.lock().unwrap().insert(
            id.node_id.clone(),
            DaemonConnection {
                tx,
                state: state.clone(),
                task: tokio::spawn(std::future::pending()),
            },
        );
        translate_message(
            &ServerMessage::SessionReady {
                session_id: "confirmed".into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
            &state,
            &mut None,
            &id.node_id,
        );
        (core, id, state, rx)
    }

    #[tokio::test]
    async fn repeated_connect_preserves_the_live_connection_and_disconnect_releases_it() {
        let dir = tempfile::tempdir().unwrap();
        let core = MobileCore::new(vec![7; 32], dir.path().to_str().unwrap().into())
            .await
            .unwrap();
        let id = core.add_daemon(core.node_id(), "self".into());
        core.connect(id.clone());
        let state = core
            .connections
            .lock()
            .unwrap()
            .get(&id.node_id)
            .unwrap()
            .state
            .clone();
        core.connect(id.clone());
        assert!(Arc::ptr_eq(
            &state,
            &core
                .connections
                .lock()
                .unwrap()
                .get(&id.node_id)
                .unwrap()
                .state
        ));
        let weak = Arc::downgrade(&state);
        drop(state);
        core.disconnect(id);
        tokio::time::timeout(Duration::from_secs(1), async {
            while weak.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("disconnect must cancel connecting and backoff tasks");
    }

    #[tokio::test]
    async fn pending_attachment_cannot_redirect_a_prompt_or_snapshot() {
        let (core, id, _state, mut rx) = connected_core().await;
        core.attach(id.clone(), "requested".into());
        assert!(matches!(
            rx.recv().await,
            Some(QueuedCommand {
                message: ClientMessage::AttachSession { .. },
                ..
            })
        ));
        assert_eq!(
            core.snapshot(id.clone())
                .unwrap()
                .attached_session
                .as_deref(),
            Some("confirmed")
        );
        core.prompt(id.clone(), "must stay in composer".into());
        assert!(
            rx.try_recv().is_err(),
            "no prompt may reach the previous session"
        );
        assert!(core.snapshot(id).unwrap().sessions[0].messages.is_empty());
    }

    #[tokio::test]
    async fn failed_prompt_send_does_not_create_a_phantom_user_message() {
        let (core, id, state, rx) = connected_core().await;
        drop(rx);
        core.prompt(id.clone(), "unsent".into());
        assert!(core.snapshot(id).unwrap().sessions[0].messages.is_empty());
        assert!(state
            .client_state
            .lock()
            .unwrap()
            .session("confirmed")
            .unwrap()
            .last_sent_prompt
            .is_none());
    }

    #[tokio::test]
    async fn accepted_prompt_survives_the_next_canonical_projection() {
        let (core, id, state, mut rx) = connected_core().await;
        core.prompt(id.clone(), "accepted".into());
        assert!(
            matches!(rx.recv().await, Some(QueuedCommand { message: ClientMessage::Prompt { text, .. }, .. }) if text == "accepted")
        );
        translate_message(
            &ServerMessage::SessionActivityChanged {
                session_id: "confirmed".into(),
                activity: mew_protocol::SessionState::Running,
            },
            &state,
            &mut None,
            &id.node_id,
        );
        assert_eq!(core.snapshot(id).unwrap().sessions[0].messages.len(), 1);
    }

    #[tokio::test]
    async fn history_for_another_session_cannot_replace_the_attached_transcript() {
        let (core, id, state, _rx) = connected_core().await;
        core.prompt(id.clone(), "current".into());
        translate_message(
            &ServerMessage::SessionHistoryPage {
                session_id: "other".into(),
                messages: vec![],
                next_cursor: None,
                replace: true,
            },
            &state,
            &mut None,
            &id.node_id,
        );
        assert_eq!(core.snapshot(id).unwrap().sessions[0].messages.len(), 1);
    }

    #[tokio::test]
    async fn queued_prompt_keeps_its_origin_when_requested_attachment_changes() {
        let (core, id, state, mut rx) = connected_core().await;
        assert!(core.prompt(id.clone(), "old session".into()));
        let queued = rx.recv().await.unwrap();
        core.attach(id.clone(), "new session".into());
        assert_eq!(queued.session_id.as_deref(), Some("confirmed"));
        assert!(!queued.ready(&state));
    }

    #[tokio::test]
    async fn reconnect_requires_a_new_session_ack_before_sending() {
        let (core, id, state, _rx) = connected_core().await;
        set_shared_connection_status(&state, mew_client_core::ConnectionStatus::Connecting);
        set_shared_connection_status(&state, mew_client_core::ConnectionStatus::Connected);
        assert!(!core.prompt(id.clone(), "wait for replay".into()));
        assert!(core.snapshot(id).unwrap().attached_session.is_none());
    }

    #[tokio::test]
    async fn repeated_submission_before_running_queues_only_one_prompt() {
        let (core, id, state, mut rx) = connected_core().await;
        assert!(core.prompt(id.clone(), "first".into()));
        assert!(!core.prompt(id.clone(), "second".into()));
        let first = rx.try_recv().unwrap();
        assert!(matches!(first.message, ClientMessage::Prompt { text, .. } if text == "first"));
        assert!(rx.try_recv().is_err());
        let mut echoed = Some("first".into());
        translate_message(
            &ServerMessage::UserMessage {
                text: "first".into(),
                request_id: None,
            },
            &state,
            &mut echoed,
            &id.node_id,
        );
        assert!(!core.prompt(id.clone(), "after echo".into()));
        translate_message(
            &ServerMessage::SessionActivityChanged {
                session_id: "other".into(),
                activity: mew_protocol::SessionState::Running,
            },
            &state,
            &mut None,
            &id.node_id,
        );
        assert!(!core.prompt(id.clone(), "after foreign activity".into()));
        translate_message(
            &ServerMessage::SessionActivityChanged {
                session_id: "confirmed".into(),
                activity: mew_protocol::SessionState::Running,
            },
            &state,
            &mut None,
            &id.node_id,
        );
        assert!(!core.prompt(id.clone(), "while running".into()));
        translate_message(
            &ServerMessage::SessionActivityChanged {
                session_id: "confirmed".into(),
                activity: mew_protocol::SessionState::Idle,
            },
            &state,
            &mut None,
            &id.node_id,
        );
        assert!(core.prompt(id.clone(), "next turn".into()));
    }

    #[tokio::test]
    async fn submission_error_and_reconnect_release_only_current_reservations() {
        let (core, id, state, mut rx) = connected_core().await;
        assert!(core.prompt(id.clone(), "first".into()));
        rx.try_recv().unwrap();
        translate_message(
            &ServerMessage::Error {
                message: "not accepted".into(),
            },
            &state,
            &mut None,
            &id.node_id,
        );
        assert!(core.prompt(id.clone(), "retry".into()));
        let old = rx.try_recv().unwrap();
        set_shared_connection_status(
            &state,
            mew_client_core::ConnectionStatus::Backoff {
                attempt: 0,
                error: "closed".into(),
            },
        );
        set_shared_connection_status(&state, mew_client_core::ConnectionStatus::Connected);
        translate_message(
            &ServerMessage::SessionReady {
                session_id: "confirmed".into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
            &state,
            &mut None,
            &id.node_id,
        );
        assert!(
            !old.ready(&state),
            "reconnect must not resubmit an old queued prompt"
        );
        assert!(core.prompt(id.clone(), "fresh".into()));
    }

    #[test]
    fn coalesced_text_keeps_its_session_when_attachment_changes() {
        let mut buffer = TextDeltaBuffer::default();
        assert!(buffer
            .push("daemon", "first".into(), "part".into(), "old".into())
            .is_none());
        let flushed = buffer
            .push("daemon", "second".into(), "part".into(), "new".into())
            .unwrap();
        assert!(
            matches!(flushed, CoreEvent::TextDelta { session_id, delta, .. } if session_id == "first" && delta == "old")
        );
        assert!(
            matches!(buffer.take("daemon"), Some(CoreEvent::TextDelta { session_id, delta, .. }) if session_id == "second" && delta == "new")
        );
        assert!(buffer.take("daemon").is_none());
    }

    #[tokio::test]
    async fn snapshot_contains_replayed_permissions_and_questions() {
        let dir = tempfile::tempdir().unwrap();
        let core = MobileCore::new(vec![7; 32], dir.path().to_str().unwrap().into())
            .await
            .unwrap();
        let conn = Arc::new(ConnState::new());
        let daemon = "node-abc".to_owned();
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        *conn.requested_session.lock().unwrap() = Some(id.into());
        conn.client_state.lock().unwrap().connection =
            Some(mew_client_core::ConnectionStatus::Connected);
        let (tx, mut outbound) = mpsc::unbounded_channel();
        core.connections.lock().unwrap().insert(
            daemon.clone(),
            DaemonConnection {
                tx,
                state: conn.clone(),
                task: tokio::spawn(std::future::pending()),
            },
        );
        let mut prompt = None;
        translate_message(
            &ServerMessage::SessionReady {
                session_id: id.into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        translate_message(
            &ServerMessage::PermissionRequest {
                request_id: "permission".into(),
                tool_name: "shell".into(),
                input: serde_json::json!({"command":"pwd"}),
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        translate_message(
            &ServerMessage::AskUserRequest {
                subagent: Some(mew_protocol::SubagentQuestionSource {
                    parent_call_id: "task".into(),
                    child_session_id: "child".into(),
                    name: "worker".into(),
                    display_name: Some("Curie".into()),
                }),
                request_id: "ask".into(),
                call_id: "ask-call".into(),
                questions: vec![mew_protocol::Question {
                    prompt: "which file?".into(),
                    options: vec![],
                }],
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        let snapshot = core
            .snapshot(DaemonId {
                node_id: daemon.clone(),
            })
            .unwrap();
        assert_eq!(snapshot.pending_permissions.len(), 1);
        assert_eq!(snapshot.pending_permissions[0].session_id, id);
        assert_eq!(snapshot.pending_permissions[0].request_id, "permission");
        assert_eq!(snapshot.pending_permissions[0].daemon, daemon);
        assert_eq!(snapshot.pending_ask_user.len(), 1);
        assert_eq!(snapshot.pending_ask_user[0].session_id, id);
        assert_eq!(snapshot.pending_ask_user[0].questions, ["which file?"]);
        assert_eq!(snapshot.pending_ask_user[0].daemon, daemon);
        let source = snapshot.pending_ask_user[0].subagent.as_ref().unwrap();
        assert_eq!(source.parent_call_id, "task");
        assert_eq!(source.child_session_id, "child");
        assert_eq!(source.name, "worker");
        assert_eq!(source.display_name.as_deref(), Some("Curie"));
        core.cancel_ask_user(
            DaemonId {
                node_id: daemon.clone(),
            },
            "ask".into(),
        );
        let command = outbound
            .try_recv()
            .expect("confirmed attachment should allow closing its child question");
        assert_eq!(command.session_id.as_deref(), Some(id));
        assert!(
            matches!(command.message, ClientMessage::AskUserResponse { request_id, cancelled: true, .. } if request_id == "ask")
        );
        translate_message(
            &ServerMessage::SessionHistoryPage {
                session_id: id.into(),
                messages: vec![],
                next_cursor: None,
                replace: false,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        assert_eq!(
            core.snapshot(DaemonId {
                node_id: daemon.clone()
            })
            .unwrap()
            .pending_permissions
            .len(),
            1
        );
        let ready = translate_message(
            &ServerMessage::SessionReady {
                session_id: id.into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        assert!(
            matches!(ready.as_slice(), [CoreEvent::SessionReady { daemon: ready_daemon, session_id }, CoreEvent::SessionReloaded { .. }]
            if ready_daemon == &daemon && session_id == id)
        );
        let snapshot = core.snapshot(DaemonId { node_id: daemon }).unwrap();
        assert!(snapshot.pending_permissions.is_empty());
        assert!(snapshot.pending_ask_user.is_empty());
        core.endpoint.close().await;
    }

    #[test]
    fn mobile_provider_projection_reduces_each_frame_once() {
        let conn = Arc::new(ConnState::new());
        let daemon = "node-abc".to_owned();
        let mut prompt = None;
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        translate_message(
            &ServerMessage::SessionReady {
                session_id: id.into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        let message_id = ulid::Ulid::new();
        let part_id = ulid::Ulid::new();
        translate_message(
            &ServerMessage::Provider {
                event: ProviderEventWire::PartStart {
                    part: Part::Text(mew_message::TextPart {
                        base: mew_message::PartBase {
                            id: part_id,
                            message_id,
                            session_id: ulid::Ulid::from_string(id).unwrap(),
                        },
                        text: String::new(),
                        synthetic: false,
                    }),
                },
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        translate_message(
            &ServerMessage::Provider {
                event: ProviderEventWire::PartDelta {
                    part_id,
                    field: "text".into(),
                    delta: "hello".into(),
                },
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        {
            let guard = conn.session_state.lock().unwrap();
            let session = guard.as_ref().unwrap();
            assert_eq!(session.messages[0].parts.len(), 1);
            assert_eq!(session.messages[0].parts[0].text.as_deref(), Some("hello"));
        }
        translate_message(
            &ServerMessage::Provider {
                event: ProviderEventWire::MessageEnd {
                    finish: mew_message::Finish::ToolUse,
                    usage: mew_message::Tokens {
                        input: 7,
                        output: 2,
                        ..Default::default()
                    },
                    cost: 0.01,
                    manifest: None,
                },
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        let guard = conn.session_state.lock().unwrap();
        let session = guard.as_ref().unwrap();
        assert_eq!(session.messages[0].parts.len(), 1);
        assert_eq!(session.messages[0].parts[0].text.as_deref(), Some("hello"));
        assert_eq!(session.input_tokens, 7);
        assert_eq!(session.turns, 1);
    }

    #[test]
    fn mobile_activity_controls_busy_state_across_provider_completion() {
        let conn = Arc::new(ConnState::new());
        let daemon = "node-abc".to_owned();
        let mut prompt = None;
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        translate_message(
            &ServerMessage::SessionReady {
                session_id: id.into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        translate_message(
            &ServerMessage::SessionActivityChanged {
                session_id: id.into(),
                activity: mew_protocol::SessionState::Running,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        assert!(conn.session_state.lock().unwrap().as_ref().unwrap().running);
        translate_message(
            &ServerMessage::Provider {
                event: ProviderEventWire::MessageEnd {
                    finish: mew_message::Finish::ToolUse,
                    usage: Default::default(),
                    cost: 0.0,
                    manifest: None,
                },
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        assert!(conn.session_state.lock().unwrap().as_ref().unwrap().running);
        translate_message(
            &ServerMessage::SessionActivityChanged {
                session_id: "background".into(),
                activity: mew_protocol::SessionState::Active,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        assert!(conn.session_state.lock().unwrap().as_ref().unwrap().running);
        translate_message(
            &ServerMessage::SessionActivityChanged {
                session_id: id.into(),
                activity: mew_protocol::SessionState::Active,
            },
            &conn,
            &mut prompt,
            &daemon,
        );
        assert!(!conn.session_state.lock().unwrap().as_ref().unwrap().running);
    }

    async fn protocol_handshake_result(pong: &str) -> anyhow::Result<ServerMessage> {
        let (client, server) = tokio::io::duplex(4096);
        let mut client = tokio_tungstenite::WebSocketStream::from_raw_socket(
            client,
            tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        let mut server = tokio_tungstenite::WebSocketStream::from_raw_socket(
            server,
            tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let pong = pong.to_owned();
        let peer = tokio::spawn(async move {
            let ping = server.next().await.unwrap().unwrap();
            assert!(
                matches!(ping, Message::Text(ref text) if mew_protocol::decode_json::<ClientMessage>(text).is_ok_and(|msg| matches!(msg, ClientMessage::Ping)))
            );
            let ready = mew_protocol::encode_json(&ServerMessage::RemoteReady {
                scope: mew_protocol::RemoteScope::Control,
            })
            .unwrap();
            server.send(Message::Text(ready)).await.unwrap();
            server.send(Message::Text(pong)).await.unwrap();
        });
        let result = verify_daemon_protocol(&mut client).await;
        peer.await.unwrap();
        result
    }

    #[tokio::test]
    async fn protocol_handshake_accepts_current_daemon_after_remote_ready() {
        let current = mew_protocol::encode_json(&ServerMessage::Pong {
            version: "test".into(),
            protocol_version: mew_protocol::PROTOCOL_VERSION,
        })
        .unwrap();
        let pong = protocol_handshake_result(&current).await.unwrap();
        assert!(matches!(pong, ServerMessage::Pong { .. }));
    }

    #[tokio::test]
    async fn protocol_handshake_rejects_incompatible_daemons() {
        for pong in [
            r#"{"type":"pong","version":"legacy"}"#.to_owned(),
            r#"{"type":"pong","version":"previous-native","protocol_version":1}"#.to_owned(),
            mew_protocol::encode_json(&ServerMessage::Pong {
                version: "future".into(),
                protocol_version: mew_protocol::PROTOCOL_VERSION + 1,
            })
            .unwrap(),
        ] {
            let error = protocol_handshake_result(&pong).await.unwrap_err();
            assert!(error.to_string().contains("protocol"));
            assert!(error.to_string().contains("daemon"));
        }
    }

    #[test]
    fn test_parse_dial_info_raw_node_id() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let (parsed, name, token) = parse_dial_info_impl(&node_id).unwrap();
        assert_eq!(parsed, node_id);
        assert!(name.is_none());
        assert!(token.is_none());
    }

    #[test]
    fn test_parse_dial_info_mew001_plain() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let payload = format!("mew001:{node_id}");
        let (parsed, name, token) = parse_dial_info_impl(&payload).unwrap();
        assert_eq!(parsed, node_id);
        assert!(name.is_none());
        assert!(token.is_none());
    }

    #[test]
    fn test_parse_dial_info_mew001_json() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let payload = format!(r#"mew001:{{"node_id":"{node_id}","name":"Homelab"}}"#);
        let (parsed, name, token) = parse_dial_info_impl(&payload).unwrap();
        assert_eq!(parsed, node_id);
        assert_eq!(name.as_deref(), Some("Homelab"));
        assert!(token.is_none());
    }

    #[test]
    fn test_parse_dial_info_unknown_version_rejected() {
        // mew002: is no longer specially rejected — it falls through to
        // PublicKey parsing and fails there. This is fine.
        let result = parse_dial_info_impl("mew002:something");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dial_info_invalid_node_id() {
        let result = parse_dial_info_impl("not-a-valid-key");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid NodeId"));
    }

    #[test]
    fn test_parse_dial_info_empty_mew001() {
        let result = parse_dial_info_impl("mew001:");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dial_info_url_scheme() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let payload = format!("computer.mew.mew://{node_id}");
        let (parsed, name, token) = parse_dial_info_impl(&payload).unwrap();
        assert_eq!(parsed, node_id);
        assert!(name.is_none());
        assert!(token.is_none());
    }

    #[test]
    fn test_parse_dial_info_url_scheme_extracts_pairing_token() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let payload = format!("computer.mew.mew://{node_id}?token=invite-123");
        let (parsed, name, token) = parse_dial_info_impl(&payload).unwrap();
        assert_eq!(parsed, node_id);
        assert!(name.is_none());
        assert_eq!(token.as_deref(), Some("invite-123"));
    }

    #[test]
    fn test_parse_dial_info_url_scheme_empty() {
        let result = parse_dial_info_impl("computer.mew.mew://");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dial_info_url_scheme_invalid() {
        let result = parse_dial_info_impl("computer.mew.mew://not-a-valid-key");
        assert!(result.is_err());
    }

    #[test]
    fn test_translate_persona_list() {
        let conn = Arc::new(ConnState::new());
        let daemon_id = "node-abc".to_string();
        let mut last_prompt: Option<String> = None;

        let msg = ServerMessage::PersonaList {
            personas: vec![
                mew_protocol::PersonaInfo {
                    name: "code-reviewer".into(),
                    description: "Reviews code".into(),
                    color: Some("#ff0000".into()),
                    active: false,
                },
                mew_protocol::PersonaInfo {
                    name: "explainer".into(),
                    description: "Explains things".into(),
                    color: None,
                    active: true,
                },
            ],
        };

        let events = translate_message(&msg, &conn, &mut last_prompt, &daemon_id);
        assert_eq!(events.len(), 1);
        match &events[0] {
            CoreEvent::PersonaList { daemon, personas } => {
                assert_eq!(*daemon, daemon_id);
                assert_eq!(personas.len(), 2);
                assert_eq!(personas[0].name, "code-reviewer");
                assert_eq!(personas[0].color.as_deref(), Some("#ff0000"));
                assert!(!personas[0].active);
                assert_eq!(personas[1].name, "explainer");
                assert!(personas[1].active);
            }
            other => panic!("expected PersonaList, got {:?}", other),
        }
        // Verify ConnState was updated.
        let stored = conn.available_personas.lock().unwrap();
        assert_eq!(stored.len(), 2);
        // Verify current_persona was synced from the active flag.
        assert_eq!(
            conn.current_persona.lock().unwrap().as_deref(),
            Some("explainer")
        );
        let shared = conn.client_state.lock().unwrap();
        assert_eq!(shared.personas.len(), 2);
        assert_eq!(shared.current_persona.as_deref(), Some("explainer"));
    }

    #[test]
    fn test_translate_reduces_session_and_required_action_into_shared_state() {
        let conn = Arc::new(ConnState::new());
        let daemon_id = "node-abc".to_string();
        let mut last_prompt: Option<String> = None;

        translate_message(
            &ServerMessage::SessionReady {
                session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                cwd: Some("/tmp/project".into()),
                model: Some("fake".into()),
                provider: Some("fake".into()),
                permission_mode: Some("standard".into()),
            },
            &conn,
            &mut last_prompt,
            &daemon_id,
        );
        translate_message(
            &ServerMessage::PermissionRequest {
                request_id: "request-1".into(),
                tool_name: "shell".into(),
                input: serde_json::json!({"command": "pwd"}),
            },
            &conn,
            &mut last_prompt,
            &daemon_id,
        );

        let shared = conn.client_state.lock().unwrap();
        let session = shared
            .session("01ARZ3NDEKTSV4RRFFQ69G5FAV")
            .expect("shared session");
        assert_eq!(
            shared.attached_session.as_deref(),
            Some(session.session_id.as_str())
        );
        assert_eq!(session.pending_actions.len(), 1);
        assert_eq!(session.pending_actions[0].request_id, "request-1");
    }

    #[test]
    fn test_translate_queues_typed_shared_events_for_platform_adapters() {
        let conn = Arc::new(ConnState::new());
        let daemon_id = "node-abc".to_string();
        let mut last_prompt: Option<String> = None;
        let session_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

        translate_message(
            &ServerMessage::SessionReady {
                session_id: session_id.into(),
                cwd: Some("/tmp/project".into()),
                model: Some("fake".into()),
                provider: Some("fake".into()),
                permission_mode: Some("standard".into()),
            },
            &conn,
            &mut last_prompt,
            &daemon_id,
        );
        translate_message(
            &ServerMessage::PermissionRequest {
                request_id: "request-1".into(),
                tool_name: "shell".into(),
                input: serde_json::json!({"command": "pwd"}),
            },
            &conn,
            &mut last_prompt,
            &daemon_id,
        );

        let events = conn.take_shared_events();
        assert!(matches!(
            events.as_slice(),
            [
                mew_client_core::ClientEvent::SessionReady { session_id: ready_id },
                mew_client_core::ClientEvent::RequiredActionChanged {
                    session_id: action_session_id,
                    request_id,
                },
            ] if ready_id == session_id
                && action_session_id == session_id
                && request_id == "request-1"
        ));
    }

    #[test]
    fn test_translate_persona_switched() {
        let conn = Arc::new(ConnState::new());
        let daemon_id = "node-abc".to_string();
        let mut last_prompt: Option<String> = None;

        let msg = ServerMessage::PersonaSwitched {
            name: "explainer".into(),
        };

        let events = translate_message(&msg, &conn, &mut last_prompt, &daemon_id);
        assert_eq!(events.len(), 1);
        match &events[0] {
            CoreEvent::PersonaSwitched { daemon, name } => {
                assert_eq!(name, "explainer");
                assert_eq!(*daemon, daemon_id);
            }
            other => panic!("expected PersonaSwitched, got {:?}", other),
        }
        assert_eq!(
            conn.current_persona.lock().unwrap().as_deref(),
            Some("explainer")
        );
    }
}
