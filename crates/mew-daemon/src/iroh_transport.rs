//! Iroh p2p transport for the mew daemon.
//!
//! This module is only compiled when the `iroh` feature is enabled.
//! It provides:
//! - `IrohStream` — wraps iroh's `(SendStream, RecvStream)` into a single
//!   `AsyncRead + AsyncWrite` type that `handle_connection` can consume.
//! - `NodeIdAllowlist` — persistent allowlist of trusted peer NodeIds.
//! - `MewIrohHandler` — `ProtocolHandler` that authenticates connections
//!   against the allowlist and dispatches to `handle_connection`.
//! - `DaemonServer::run_iroh` — binds an iroh endpoint and serves connections.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, SecretKey};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{info, warn};

use crate::{groups::GroupsStore, handle_connection, session::SessionManager, ThinkingSetter};

/// ALPN protocol identifier for mew wire protocol over iroh.
pub const MEW_ALPN: &[u8] = b"mew/wire/0";

/// Wraps an iroh bidirectional stream pair into a single type implementing
/// both `AsyncRead` and `AsyncWrite`, so it can be passed to `handle_connection`.
pub struct IrohStream {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
}

impl IrohStream {
    pub fn new(send: iroh::endpoint::SendStream, recv: iroh::endpoint::RecvStream) -> Self {
        Self { send, recv }
    }
}

impl AsyncRead for IrohStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        AsyncRead::poll_read(std::pin::Pin::new(&mut this.recv), cx, buf)
    }
}

impl AsyncWrite for IrohStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        AsyncWrite::poll_write(std::pin::Pin::new(&mut this.send), cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        AsyncWrite::poll_flush(std::pin::Pin::new(&mut this.send), cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        AsyncWrite::poll_shutdown(std::pin::Pin::new(&mut this.send), cx)
    }
}

/// Persistent allowlist of trusted iroh peer NodeIds.
///
/// Stored as a JSON sidecar file (default: `~/.config/mew/authorized_nodes.json`).
/// Managed through the pairing flow — users do not edit it by hand.
#[derive(Debug)]
pub struct NodeIdAllowlist {
    nodes: Mutex<Vec<String>>,
    path: PathBuf,
}

impl NodeIdAllowlist {
    /// Load the allowlist from the given path, or create an empty one if
    /// the file doesn't exist.
    pub fn load(path: PathBuf) -> Result<Self> {
        let nodes = if path.exists() {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("read allowlist {}", path.display()))?;
            match serde_json::from_slice::<Vec<String>>(&bytes) {
                Ok(nodes) => nodes,
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "allowlist file is corrupted, starting with empty allowlist");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        Ok(Self {
            nodes: Mutex::new(nodes),
            path,
        })
    }

    /// Create an empty allowlist with the given path (for tests).
    pub fn new(path: PathBuf) -> Self {
        Self {
            nodes: Mutex::new(Vec::new()),
            path,
        }
    }

    /// Check if a NodeId string is in the allowlist.
    pub fn contains(&self, node_id: &str) -> bool {
        self.nodes.lock().unwrap().contains(&node_id.to_string())
    }

    /// Add a NodeId to the allowlist and persist to disk.
    pub fn add(&self, node_id: &str) -> Result<()> {
        let mut nodes = self.nodes.lock().unwrap();
        let id = node_id.to_string();
        if !nodes.contains(&id) {
            nodes.push(id);
            self.save_locked(&nodes)?;
        }
        Ok(())
    }

    /// Remove a NodeId from the allowlist and persist to disk.
    pub fn remove(&self, node_id: &str) -> Result<()> {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.retain(|n| n != node_id);
        self.save_locked(&nodes)?;
        Ok(())
    }

    /// List all trusted NodeIds.
    pub fn list(&self) -> Vec<String> {
        self.nodes.lock().unwrap().clone()
    }

    fn save_locked(&self, nodes: &[String]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let json = serde_json::to_string_pretty(nodes)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Iroh protocol handler for the mew wire protocol.
///
/// Authenticates incoming connections against the NodeId allowlist,
/// wraps the first bidirectional stream as an `IrohStream`, and passes
/// it to `handle_connection`.
pub struct MewIrohHandler {
    pub allowlist: Arc<NodeIdAllowlist>,
    pub session_manager: Arc<SessionManager>,
    pub groups_store: Arc<GroupsStore>,
    pub thinking_setter: Option<ThinkingSetter>,
    pub auto_summary_enabled: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for MewIrohHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MewIrohHandler")
            .field("allowlist", &self.allowlist)
            .finish_non_exhaustive()
    }
}

impl ProtocolHandler for MewIrohHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote_id = connection.remote_id();
        let id_str = remote_id.to_string();

        if !self.allowlist.contains(&id_str) {
            warn!(peer = %id_str, "rejected iroh connection: not in allowlist");
            connection.close(1u32.into(), b"unauthorized");
            return Ok(());
        }

        info!(peer = %id_str, "iroh connection accepted");

        let (send, recv) = connection
            .accept_bi()
            .await
            .map_err(|e| AcceptError::from_err(e))?;
        let stream = IrohStream::new(send, recv);

        if let Err(e) = handle_connection(
            stream,
            self.session_manager.clone(),
            self.groups_store.clone(),
            self.thinking_setter.clone(),
            // TODO: hook this in properly
            self.auto_summary_enabled.clone(),
        )
        .await
        {
            if !e.to_string().contains("connection reset") {
                warn!(peer = %id_str, error = %e, "iroh connection ended with error");
            }
        }

        info!(peer = %id_str, "iroh connection closed");
        Ok(())
    }
}

/// Run the daemon with an iroh listener.
///
/// Binds an iroh endpoint with the `mew/wire/0` ALPN, prints the NodeId
/// for pairing, and accepts connections until shutdown.
pub async fn run_iroh(
    session_manager: Arc<SessionManager>,
    groups_store: Arc<GroupsStore>,
    thinking_setter: Option<ThinkingSetter>,
    auto_summary_enabled: Arc<std::sync::atomic::AtomicBool>,
    allowlist_path: PathBuf,
    secret_key: SecretKey,
) -> Result<()> {
    let allowlist = Arc::new(NodeIdAllowlist::load(allowlist_path.clone())?);

    let endpoint = Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret_key)
        .alpns(vec![MEW_ALPN.to_vec()])
        .bind()
        .await
        .context("bind iroh endpoint")?;

    // Bring the endpoint online (waits for relay connection).
    // Timeout prevents hanging forever if relays are unreachable.
    info!("connecting to iroh relay servers...");
    tokio::time::timeout(std::time::Duration::from_secs(15), endpoint.online())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "iroh endpoint failed to come online within 15s — check network connectivity"
            )
        })?;

    let node_id = endpoint.id();
    info!(node_id = %node_id, "mew daemon listening (iroh)");

    // Print pairing info to stdout for `mew pair` to capture.
    println!("iroh-node-id:{node_id}");

    // Spawn the idle-summary task once (daemon-wide, not per-connection),
    // matching the Unix/TCP listener behavior.
    {
        let sm = session_manager.clone();
        let flag = auto_summary_enabled.clone();
        tokio::spawn(async move {
            crate::idle_summary_task(sm, flag).await;
        });
    }

    let handler = MewIrohHandler {
        allowlist: allowlist.clone(),
        session_manager,
        groups_store,
        thinking_setter,
        auto_summary_enabled,
    };

    let router = Router::builder(endpoint).accept(MEW_ALPN, handler).spawn();

    // Wait for shutdown signal.
    let mut sig_term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;

    tokio::select! {
        _ = sig_term.recv() => {
            info!("received SIGTERM, shutting down iroh listener");
        }
        _ = tokio::signal::ctrl_c() => {
            info!("received Ctrl+C, shutting down iroh listener");
        }
    }

    let _ = router.shutdown().await;
    Ok(())
}

/// Default path for the authorized nodes sidecar: alongside session dir.
pub fn default_allowlist_path() -> PathBuf {
    let session_dir = mew_session::session_dir();
    session_dir
        .parent()
        .map(|p| p.join("authorized_nodes.json"))
        .unwrap_or_else(|| PathBuf::from("authorized_nodes.json"))
}

/// Default path for the daemon's persistent iroh secret key.
/// Stored as JSON (the 32-byte key serialized via serde).
pub fn default_secret_key_path() -> PathBuf {
    let session_dir = mew_session::session_dir();
    session_dir
        .parent()
        .map(|p| p.join("iroh_secret_key.json"))
        .unwrap_or_else(|| PathBuf::from("iroh_secret_key.json"))
}

/// Load a persistent `SecretKey` from the given path, or generate a new one
/// and persist it. This ensures the daemon's NodeId stays stable across
/// restarts, which is essential for the mobile client's daemon registry.
///
/// The key is stored as JSON-serialized bytes. It should never be shared
/// or committed to version control.
pub fn load_or_create_secret_key(path: &Path) -> Result<SecretKey> {
    if path.exists() {
        let bytes = std::fs::read(path)
            .with_context(|| format!("read iroh secret key {}", path.display()))?;
        let key: SecretKey = serde_json::from_slice(&bytes).map_err(|e| {
            anyhow::anyhow!(
                "corrupted iroh secret key at {}: {e}. \
                     Delete the file and restart to generate a new key. \
                     WARNING: this will change the daemon's NodeId and \
                     break all paired mobile clients.",
                path.display()
            )
        })?;
        Ok(key)
    } else {
        let key = SecretKey::generate();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let json = serde_json::to_string(&key)?;
        // Write with restrictive permissions (0600 on Unix).
        std::fs::write(path, json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
        }
        info!(path = %path.display(), "generated new iroh secret key");
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_add_contains_remove() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let allowlist = NodeIdAllowlist::new(tmp.path().to_path_buf());

        assert!(!allowlist.contains("abc123"));
        allowlist.add("abc123").unwrap();
        assert!(allowlist.contains("abc123"));
        assert_eq!(allowlist.list(), vec!["abc123"]);

        // Duplicate add is a no-op
        allowlist.add("abc123").unwrap();
        assert_eq!(allowlist.list(), vec!["abc123"]);

        allowlist.add("def456").unwrap();
        assert_eq!(allowlist.list(), vec!["abc123", "def456"]);

        allowlist.remove("abc123").unwrap();
        assert!(!allowlist.contains("abc123"));
        assert_eq!(allowlist.list(), vec!["def456"]);
    }

    #[test]
    fn test_allowlist_persistence() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        // Write
        {
            let allowlist = NodeIdAllowlist::load(path.clone()).unwrap();
            allowlist.add("node1").unwrap();
            allowlist.add("node2").unwrap();
        }

        // Reload
        let allowlist = NodeIdAllowlist::load(path).unwrap();
        assert!(allowlist.contains("node1"));
        assert!(allowlist.contains("node2"));
        assert!(!allowlist.contains("node3"));
    }

    #[test]
    fn test_allowlist_load_missing_file() {
        let path = PathBuf::from("/tmp/mew-test-does-not-exist-12345.json");
        let allowlist = NodeIdAllowlist::load(path).unwrap();
        assert!(allowlist.list().is_empty());
    }

    #[test]
    fn test_iroh_stream_implements_async_traits() {
        // Verify IrohStream implements AsyncRead + AsyncWrite + Unpin at
        // compile time. This catches accidental trait removals.
        fn _assert_async_read<T: AsyncRead + Unpin>() {}
        fn _assert_async_write<T: AsyncWrite + Unpin>() {}
        _assert_async_read::<IrohStream>();
        _assert_async_write::<IrohStream>();
    }
}
