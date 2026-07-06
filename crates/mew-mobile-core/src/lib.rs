//! mew-mobile-core: the Rust core for mobile clients (iOS/Android).
//!
//! Owns the iroh endpoint, manages per-daemon connections, decodes the
//! mew wire protocol, assembles session state, and emits typed events to
//! the platform layer via `CoreListener`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use iroh::{Endpoint, SecretKey};
use tokio::sync::mpsc;
use tokio_tungstenite::client_async;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tracing::{info, warn};

use mew_message::{Part, ProviderEventWire};
use mew_protocol::{ClientMessage, ServerMessage};

pub mod codec;
pub mod events;
pub mod registry;
pub mod state;

pub use codec::decode_server_message_lenient;
pub use events::{CoreEvent, CoreListener, DaemonStatus, Decision};
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
}

/// Per spec notes #2 and #11: keep dial-info parsing behind this one
/// function since it's the most likely thing to change post-stage-1.
#[uniffi::export]
pub fn parse_dial_info(payload: String) -> Result<DialInfo, CoreError> {
    parse_dial_info_impl(&payload)
        .map(|(node_id, name)| DialInfo { node_id, name })
        .map_err(|e| CoreError::ParseFailed {
            reason: e.to_string(),
        })
}

fn parse_dial_info_impl(payload: &str) -> Result<(String, Option<String>)> {
    let payload = payload.trim();

    // URL-scheme format: computer.mew.mew://<node_id>
    // This is what `mew pair` puts in the QR code.
    if let Some(rest) = payload.strip_prefix("computer.mew.mew://") {
        if rest.is_empty() {
            return Err(anyhow::anyhow!("computer.mew.mew:// payload is empty"));
        }
        // Validate it's a real NodeId.
        iroh::PublicKey::from_str(rest)
            .map_err(|e| anyhow::anyhow!("invalid NodeId in URL scheme: {e}"))?;
        return Ok((rest.to_string(), None));
    }

    // Legacy versioned format: mew001:<node_id> or mew001:{json}
    if let Some(rest) = payload.strip_prefix("mew001:") {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(rest) {
            let node_id = json
                .get("node_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("mew001 payload missing node_id field"))?;
            let name = json.get("name").and_then(|v| v.as_str()).map(String::from);
            return Ok((node_id.to_string(), name));
        }
        if rest.is_empty() {
            return Err(anyhow::anyhow!("mew001 payload is empty"));
        }
        return Ok((rest.to_string(), None));
    }

    // Raw NodeId — validate it parses as a PublicKey.
    iroh::PublicKey::from_str(payload)
        .map_err(|e| anyhow::anyhow!("invalid NodeId '{payload}': {e}"))?;
    Ok((payload.to_string(), None))
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
    /// The currently attached session ID, if any.
    attached_session: Mutex<Option<String>>,
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
}

impl ConnState {
    fn new() -> Self {
        Self {
            attached_session: Mutex::new(None),
            session_state: Mutex::new(None),
            daemon_version: Mutex::new(None),
            models: Mutex::new(Vec::new()),
            session_title: Mutex::new(None),
            listener: Mutex::new(None),
        }
    }
}

/// Per-daemon connection state. Stored in the connections map keyed by
/// the daemon's node_id string.
struct DaemonConnection {
    /// Channel for sending ClientMessages to the background task, which
    /// forwards them over the WebSocket.
    tx: mpsc::UnboundedSender<ClientMessage>,
    /// Shared state between the background task and the foreground API.
    state: Arc<ConnState>,
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

    /// Remove a daemon from the registry and disconnect.
    pub fn remove_daemon(&self, id: DaemonId) {
        self.registry.lock().unwrap().remove(&id);
        let mut conns = self.connections.lock().unwrap();
        conns.remove(&id.node_id);
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

        let (tx, rx) = mpsc::unbounded_channel::<ClientMessage>();
        let conn_state = Arc::new(ConnState::new());
        let conn = DaemonConnection {
            tx: tx.clone(),
            state: conn_state.clone(),
        };
        self.connections
            .lock()
            .unwrap()
            .insert(id.node_id.clone(), conn);

        let endpoint = self.endpoint.clone();
        let daemon_id = id.node_id.clone();
        let node_id = entry.node_id.clone();

        // Sync the listener into ConnState so the background task reads
        // it per-event (supports set_listener after connect).
        {
            let l = self.listener.lock().unwrap().clone();
            *conn_state.listener.lock().unwrap() = l;
        }

        // Spawn the background connection task.
        let conn_state_for_task = conn_state.clone();
        self.runtime.spawn(async move {
            // Reconnect loop with exponential backoff (spec: 1s, 2s, 4s… cap 30s).
            let mut attempt: u32 = 0;
            let mut rx = rx;
            loop {
                let result = connect_and_run(
                    &endpoint,
                    &daemon_id,
                    &node_id,
                    &mut rx,
                    &conn_state_for_task,
                )
                .await;

                match result {
                    Err(e) => {
                        warn!(daemon = %daemon_id, error = %e, attempt, "connection failed");
                    }
                    Ok(should_stop) => {
                        // Ok(true) = channel closed (user disconnected) — stop.
                        // Ok(false) = connection dropped — retry.
                        if should_stop {
                            break;
                        }
                    }
                }

                // Exponential backoff with jitter.
                let base_secs = 1u64 << attempt.min(5); // 1, 2, 4, 8, 16, 32
                let capped = base_secs.min(30);
                let jitter = rand_jitter() % 500; // 0-499ms jitter
                let delay = Duration::from_millis(capped * 1000 + jitter);

                if let Some(ref l) = *conn_state_for_task.listener.lock().unwrap() {
                    l.on_event(CoreEvent::DaemonStatusChanged {
                        daemon: daemon_id.clone(),
                        status: DaemonStatus::Backoff { attempt },
                    });
                }

                info!(daemon = %daemon_id, attempt, delay_ms = delay.as_millis(), "reconnecting after backoff");
                tokio::time::sleep(delay).await;
                attempt += 1;
            }

            if let Some(ref l) = *conn_state_for_task.listener.lock().unwrap() {
                l.on_event(CoreEvent::DaemonStatusChanged {
                    daemon: daemon_id.clone(),
                    status: DaemonStatus::Disconnected,
                });
            }
        });
    }

    /// Disconnect from a daemon.
    pub fn disconnect(&self, id: DaemonId) {
        let mut conns = self.connections.lock().unwrap();
        conns.remove(&id.node_id);
    }

    /// Attach to a session on a daemon.
    pub fn attach(&self, id: DaemonId, session_id: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            *conn.state.attached_session.lock().unwrap() = Some(session_id.clone());
            let _ = conn.tx.send(ClientMessage::AttachSession {
                session_id,
                client_kind: mew_protocol::ClientKind::Mobile,
            });
        }
    }

    /// Send a prompt to the attached session on a daemon.
    pub fn prompt(&self, id: DaemonId, text: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            // Optimistically add the user message so it renders immediately, and
            // mark it so the daemon's echoed UserMessage is deduped (spec note
            // #9) rather than double-added.
            let session_id = {
                let mut ss_lock = conn.state.session_state.lock().unwrap();
                if let Some(ss) = ss_lock.as_mut() {
                    ss.last_sent_prompt = Some(text.clone());
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
                    });
                    Some(ss.session_id.clone())
                } else {
                    None
                }
            };
            // Refresh the UI so the optimistic message shows without waiting for
            // a turn to start.
            if let Some(sid) = session_id {
                if let Some(ref l) = *conn.state.listener.lock().unwrap() {
                    l.on_event(CoreEvent::SessionReloaded {
                        daemon: id.node_id.clone(),
                        session_id: sid,
                    });
                }
            }
            let _ = conn.tx.send(ClientMessage::Prompt {
                text,
                attachments: vec![],
            });
        }
    }

    /// Cancel the current turn on a daemon.
    pub fn cancel(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::Cancel);
        }
    }

    /// Respond to a permission request.
    pub fn respond_permission(&self, id: DaemonId, request_id: u64, decision: Decision) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let wire_decision = match decision {
                Decision::AllowOnce => mew_protocol::PermissionDecision::AllowOnce,
                Decision::AllowSession => mew_protocol::PermissionDecision::AllowSession,
                Decision::Deny => mew_protocol::PermissionDecision::Deny,
            };
            let _ = conn.tx.send(ClientMessage::PermissionResponse {
                request_id,
                decision: wire_decision,
            });
        }
    }

    /// Respond to an ask-user request.
    pub fn respond_ask_user(&self, id: DaemonId, request_id: u64, answers: Vec<String>) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::AskUserResponse {
                request_id,
                answers,
            });
        }
    }

    /// List sessions on a daemon.
    pub fn list_sessions(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::ListSessions);
        }
    }

    /// Create a new session on a daemon.
    pub fn new_session(&self, id: DaemonId, cwd: Option<String>) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::NewSession {
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
            let _ = conn.tx.send(ClientMessage::ListProjects);
        }
    }

    /// List files in a directory on the daemon. The result arrives as a
    /// `CoreEvent::DirListing` event.
    pub fn list_dir(&self, id: DaemonId, session_id: String, path: Option<String>) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::ListDir { session_id, path });
        }
    }

    /// List available models from the daemon.
    pub fn list_models(&self, id: DaemonId) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::ListModels);
        }
    }

    /// Switch the active session to a different model.
    pub fn switch_model(&self, id: DaemonId, provider: String, model: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::SwitchModel { provider, model });
        }
    }

    /// Set the permission mode for the active session.
    pub fn set_permission_mode(&self, id: DaemonId, mode: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::SetPermissionMode { mode });
        }
    }

    /// Rename a session.
    pub fn rename_session(&self, id: DaemonId, session_id: String, title: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn
                .tx
                .send(ClientMessage::RenameSession { session_id, title });
        }
    }

    /// Archive or unarchive a session.
    pub fn archive_session(&self, id: DaemonId, session_id: String, archived: bool) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::ArchiveSession {
                session_id,
                archived,
            });
        }
    }

    /// Pin or unpin a session.
    pub fn pin_session(&self, id: DaemonId, session_id: String, pinned: bool) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn
                .tx
                .send(ClientMessage::PinSession { session_id, pinned });
        }
    }

    /// Delete a session from disk.
    pub fn delete_session(&self, id: DaemonId, session_id: String) {
        let conns = self.connections.lock().unwrap();
        if let Some(conn) = conns.get(&id.node_id) {
            let _ = conn.tx.send(ClientMessage::DeleteSession { session_id });
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
        let attached = conn.state.attached_session.lock().unwrap().clone();
        let session_state = conn.state.session_state.lock().unwrap();
        let models = conn.state.models.lock().unwrap().clone();
        let daemon_version = conn.state.daemon_version.lock().unwrap().clone();
        let title = conn.state.session_title.lock().unwrap().clone();

        let mut snap = DaemonSnapshot {
            attached_session: attached.clone(),
            models,
            daemon_version,
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
            });
        }
        Some(snap)
    }
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
    rx: &mut mpsc::UnboundedReceiver<ClientMessage>,
    conn_state: &Arc<ConnState>,
) -> Result<bool> {
    let emit_event = |event: CoreEvent| {
        if let Some(ref l) = *conn_state.listener.lock().unwrap() {
            l.on_event(event);
        }
    };

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
    let (mut ws, _resp) = client_async(req, iroh_stream)
        .await
        .context("websocket handshake over iroh")?;

    emit_event(CoreEvent::DaemonStatusChanged {
        daemon: daemon_id.to_string(),
        status: DaemonStatus::Connected,
    });

    // Send Ping to get daemon version.
    let ping = mew_protocol::encode_json(&ClientMessage::Ping)?;
    ws.send(Message::Text(ping)).await?;

    // If we have a previously-attached session, re-send AttachSession
    // so the daemon replays SessionHistory (spec: reconnect re-attach).
    let reattach = conn_state.attached_session.lock().unwrap().clone();
    if let Some(session_id) = reattach {
        let attach_msg = mew_protocol::encode_json(&ClientMessage::AttachSession {
            session_id,
            client_kind: mew_protocol::ClientKind::Mobile,
        })?;
        ws.send(Message::Text(attach_msg)).await?;
    }

    let mut last_sent_prompt: Option<String> = None;
    let mut user_disconnected = false;

    // TextDelta coalescing buffer (spec note #8).
    let mut delta_buffer = String::new();
    let mut delta_part_id: Option<String> = None;
    let mut last_flush = tokio::time::Instant::now();
    const FLUSH_INTERVAL: Duration = Duration::from_millis(16);

    loop {
        tokio::select! {
            // Forward client messages to the daemon.
            msg = rx.recv() => {
                match msg {
                    Some(client_msg) => {
                        // Track last sent prompt for UserMessage dedup.
                        if let ClientMessage::Prompt { text, .. } = &client_msg {
                            last_sent_prompt = Some(text.clone());
                        }
                        let json = mew_protocol::encode_json(&client_msg)?;
                        if ws.send(Message::Text(json)).await.is_err() {
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
                        break;
                    }
                    None => {
                        info!(daemon = %daemon_id, "ws stream ended");
                        break;
                    }
                };

                let text = match ws_msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => {
                        info!(daemon = %daemon_id, "connection closed by daemon");
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

                // Translate ServerMessage → CoreEvent + update ConnState.
                let events = translate_message(
                    &server_msg,
                    conn_state,
                    &mut last_sent_prompt,
                    daemon_id,
                );

                // Flush any pending TextDelta before emitting non-delta events.
                if !delta_buffer.is_empty() && !events.iter().any(|e| matches!(e, CoreEvent::TextDelta { .. })) {
                    if let Some(ref pid) = delta_part_id {
                        let session_id = conn_state.session_state.lock().unwrap()
                            .as_ref().map(|s| s.session_id.clone()).unwrap_or_default();
                        emit_event(CoreEvent::TextDelta {
                            daemon: daemon_id.to_string(),
                            session_id,
                            part_id: pid.clone(),
                            delta: std::mem::take(&mut delta_buffer),
                        });
                    }
                    delta_part_id = None;
                }

                for event in events {
                    // Coalesce TextDelta events (spec note #8).
                    if let CoreEvent::TextDelta { part_id, delta, .. } = &event {
                        if delta_part_id.as_deref() == Some(part_id) {
                            delta_buffer.push_str(delta);
                            if last_flush.elapsed() >= FLUSH_INTERVAL {
                                let session_id = conn_state.session_state.lock().unwrap()
                                    .as_ref().map(|s| s.session_id.clone()).unwrap_or_default();
                                emit_event(CoreEvent::TextDelta {
                                    daemon: daemon_id.to_string(),
                                    session_id,
                                    part_id: part_id.clone(),
                                    delta: std::mem::take(&mut delta_buffer),
                                });
                                last_flush = tokio::time::Instant::now();
                            }
                            continue;
                        } else {
                            // Part changed — flush old, start new.
                            if !delta_buffer.is_empty() {
                                if let Some(pid) = &delta_part_id {
                                    let session_id = conn_state.session_state.lock().unwrap()
                                        .as_ref().map(|s| s.session_id.clone()).unwrap_or_default();
                                    emit_event(CoreEvent::TextDelta {
                                        daemon: daemon_id.to_string(),
                                        session_id,
                                        part_id: pid.clone(),
                                        delta: std::mem::take(&mut delta_buffer),
                                    });
                                }
                            }
                            delta_part_id = Some(part_id.clone());
                            delta_buffer.push_str(delta);
                            last_flush = tokio::time::Instant::now();
                            continue;
                        }
                    }

                    emit_event(event);
                }
            }
        }
    }

    // Flush remaining delta buffer.
    if !delta_buffer.is_empty() {
        if let Some(pid) = &delta_part_id {
            let session_id = conn_state
                .session_state
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            emit_event(CoreEvent::TextDelta {
                daemon: daemon_id.to_string(),
                session_id,
                part_id: pid.clone(),
                delta: std::mem::take(&mut delta_buffer),
            });
        }
    }

    Ok(user_disconnected)
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

    match msg {
        ServerMessage::Pong { version } => {
            *conn_state.daemon_version.lock().unwrap() = Some(version.clone());
            events.push(CoreEvent::DaemonVersion {
                daemon: d,
                version: version.clone(),
            });
        }

        ServerMessage::SessionReady { session_id, .. } => {
            *conn_state.session_state.lock().unwrap() = Some(SessionState::new(session_id.clone()));
            events.push(CoreEvent::SessionReloaded {
                daemon: d,
                session_id: session_id.clone(),
            });
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

        ServerMessage::DirListing { path, entries } => {
            // Need the session_id for the event — use whatever's currently
            // attached on this connection.
            let sid = conn_state
                .attached_session
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
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
                session_id: sid,
                path: path.clone(),
                entries,
            });
        }

        ServerMessage::SessionHistory { messages } => {
            // Rebuild session state from history replay.
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut() {
                ss.messages.clear();
                for msg in messages {
                    let parts: Vec<_> = msg
                        .parts
                        .iter()
                        .map(|p| match p {
                            Part::Text(tp) => state::MessagePart {
                                id: tp.base.id.to_string(),
                                kind: state::PartKind::Text,
                                text: Some(tp.text.clone()),
                                tool_name: None,
                                tool_state: None,
                                tool_input: None,
                                tool_output: None,
                                tool_error: None,
                                tool_call_id: None,
                                tool_time_start: None,
                                tool_time_end: None,
                                tool_sensitivity: None,
                            },
                            Part::Reasoning(rp) => state::MessagePart {
                                id: rp.base.id.to_string(),
                                kind: state::PartKind::Reasoning,
                                text: Some(rp.text.clone()),
                                tool_name: None,
                                tool_state: None,
                                tool_input: None,
                                tool_output: None,
                                tool_error: None,
                                tool_call_id: None,
                                tool_time_start: None,
                                tool_time_end: None,
                                tool_sensitivity: None,
                            },
                            Part::ToolCall(tcp) => {
                                let (input_str, output, error, time_start, time_end) =
                                    state::tool_state_fields(&tcp.state);
                                state::MessagePart {
                                    id: tcp.base.id.to_string(),
                                    kind: state::PartKind::ToolCall,
                                    text: None,
                                    tool_name: Some(tcp.tool_name.clone()),
                                    tool_state: Some(format!("{:?}", tcp.state).to_lowercase()),
                                    tool_input: input_str,
                                    tool_output: output,
                                    tool_error: error,
                                    tool_call_id: Some(tcp.call_id.clone()),
                                    tool_time_start: time_start,
                                    tool_time_end: time_end,
                                    tool_sensitivity: tcp.sensitivity.clone(),
                                }
                            }
                            _ => state::MessagePart {
                                id: ulid::Ulid::new().to_string(),
                                kind: state::PartKind::Error,
                                text: None,
                                tool_name: None,
                                tool_state: None,
                                tool_input: None,
                                tool_output: None,
                                tool_error: None,
                                tool_call_id: None,
                                tool_time_start: None,
                                tool_time_end: None,
                                tool_sensitivity: None,
                            },
                        })
                        .collect();
                    ss.messages.push(state::ChatMessage {
                        id: ulid::Ulid::new().to_string(),
                        role: format!("{:?}", msg.role).to_lowercase(),
                        parts,
                    });
                }
            }
            let session_id = ss_lock.as_ref().map(|s| s.session_id.clone());
            drop(ss_lock);
            if let Some(sid) = session_id {
                events.push(CoreEvent::SessionReloaded {
                    daemon: d,
                    session_id: sid,
                });
            }
        }

        ServerMessage::Provider { event } => {
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut() {
                match event {
                    ProviderEventWire::PartStart { part } => {
                        ss.apply_provider_event(event);
                        if let Part::Text(tp) = part {
                            events.push(CoreEvent::TextDelta {
                                daemon: d,
                                session_id: ss.session_id.clone(),
                                part_id: tp.base.id.to_string(),
                                delta: tp.text.clone(),
                            });
                        }
                    }
                    ProviderEventWire::PartDelta { part_id, delta, .. } => {
                        ss.apply_provider_event(event);
                        events.push(CoreEvent::TextDelta {
                            daemon: d,
                            session_id: ss.session_id.clone(),
                            part_id: part_id.to_string(),
                            delta: delta.clone(),
                        });
                    }
                    ProviderEventWire::PartEnd { part_id } => {
                        ss.apply_provider_event(event);
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
                    } => {
                        ss.apply_provider_event(event);
                        let failed = *finish == mew_message::Finish::Error;
                        events.push(CoreEvent::TurnEnded {
                            daemon: d,
                            session_id: ss.session_id.clone(),
                            input_tokens: usage.input as u64,
                            output_tokens: usage.output as u64,
                            cost: *cost,
                            failed,
                        });
                    }
                    _ => {
                        ss.apply_provider_event(event);
                    }
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

        ServerMessage::UserMessage { text } => {
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
                });
            }
        }

        ServerMessage::PermissionRequest {
            request_id,
            tool_name,
            input,
        } => {
            let input_str = serde_json::to_string_pretty(input).unwrap_or_default();
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            if let Some(ss) = ss_lock.as_mut() {
                ss.pending_permissions += 1;
            }
            events.push(CoreEvent::PermissionRequested {
                daemon: d,
                session_id,
                request_id: *request_id,
                tool_name: tool_name.clone(),
                input: input_str,
            });
        }

        ServerMessage::AskUserRequest {
            request_id,
            call_id,
            questions,
        } => {
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            if let Some(ss) = ss_lock.as_mut() {
                ss.pending_questions += 1;
            }
            events.push(CoreEvent::AskUserRequested {
                daemon: d,
                session_id,
                request_id: *request_id,
                call_id: call_id.clone(),
                questions: questions.iter().map(|q| q.prompt.clone()).collect(),
            });
        }

        ServerMessage::RequestResolved { request_id } => {
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            if let Some(ss) = ss_lock.as_mut() {
                ss.pending_permissions = ss.pending_permissions.saturating_sub(1);
            }
            events.push(CoreEvent::RequestResolved {
                daemon: d,
                request_id: *request_id,
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
            if let Some(ss) = ss_lock.as_mut() {
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

        ServerMessage::TodosUpdated { .. } => {
            let session_id = conn_state
                .session_state
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            events.push(CoreEvent::TodosUpdated {
                daemon: d,
                session_id,
            });
        }

        // Pass through events that need permission handling but no state assembly.
        ServerMessage::WorkspacePermissionRequest { request_id, path } => {
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            if let Some(ss) = ss_lock.as_mut() {
                ss.pending_permissions += 1;
            }
            events.push(CoreEvent::PermissionRequested {
                daemon: d,
                session_id,
                request_id: *request_id,
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
            let mut ss_lock = conn_state.session_state.lock().unwrap();
            let session_id = ss_lock
                .as_ref()
                .map(|s| s.session_id.clone())
                .unwrap_or_default();
            if let Some(ss) = ss_lock.as_mut() {
                ss.pending_permissions += 1;
            }
            let input_str = serde_json::to_string_pretty(input).unwrap_or_default();
            events.push(CoreEvent::PermissionRequested {
                daemon: d,
                session_id,
                request_id: *request_id,
                tool_name: format!("subagent:{tool_name}"),
                input: input_str,
            });
        }

        // Catch-all: log and ignore messages we don't translate.
        _ => {
            warn!(daemon = %daemon_id, msg = ?msg, "unhandled ServerMessage");
        }
    }

    events
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
    fn test_parse_dial_info_raw_node_id() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let (parsed, name) = parse_dial_info_impl(&node_id).unwrap();
        assert_eq!(parsed, node_id);
        assert!(name.is_none());
    }

    #[test]
    fn test_parse_dial_info_mew001_plain() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let payload = format!("mew001:{node_id}");
        let (parsed, name) = parse_dial_info_impl(&payload).unwrap();
        assert_eq!(parsed, node_id);
        assert!(name.is_none());
    }

    #[test]
    fn test_parse_dial_info_mew001_json() {
        let key = iroh::SecretKey::generate();
        let node_id = key.public().to_string();
        let payload = format!(r#"mew001:{{"node_id":"{node_id}","name":"Homelab"}}"#);
        let (parsed, name) = parse_dial_info_impl(&payload).unwrap();
        assert_eq!(parsed, node_id);
        assert_eq!(name.as_deref(), Some("Homelab"));
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
        let (parsed, name) = parse_dial_info_impl(&payload).unwrap();
        assert_eq!(parsed, node_id);
        assert!(name.is_none());
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
}
