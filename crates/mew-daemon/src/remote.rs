//! State and authorization primitives shared by the daemon's remote modes.
//!
//! Transport-specific code should depend on these types instead of treating a
//! remote peer as an unrestricted local daemon client.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use mew_protocol::RemoteScope;

pub type RemoteFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Live lifecycle and invite controller for remote iroh access on a running daemon.
pub trait RemoteAccessController: Send + Sync {
    fn status(&self) -> mew_protocol::RemoteAccessInfo;
    fn set_enabled(&self, enabled: bool) -> RemoteFuture<'_, mew_protocol::RemoteAccessInfo>;
    fn create_pairing_invite(
        &self,
        scope: RemoteScope,
        ttl_secs: u64,
    ) -> RemoteFuture<'_, (String, String, u64)>;
}

#[cfg(feature = "iroh")]
struct RunningIroh {
    shutdown_tx: tokio::sync::watch::Sender<Option<i32>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

#[cfg(feature = "iroh")]
#[derive(Default)]
struct IrohControllerState {
    generation: u64,
    running: Option<RunningIroh>,
}

#[cfg(feature = "iroh")]
pub struct IrohRemoteController {
    session_manager: Arc<crate::session::SessionManager>,
    groups_store: Arc<crate::groups::GroupsStore>,
    thinking_setter: Option<crate::ThinkingSetter>,
    auto_summary_enabled: tokio::sync::watch::Sender<bool>,
    store: Arc<RemoteAccessStore>,
    secret_key: iroh::SecretKey,
    node_id: String,
    allowlist_path: PathBuf,
    remote_mode: RemoteMode,
    enabled: Arc<std::sync::atomic::AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
    state: Arc<tokio::sync::Mutex<IrohControllerState>>,
}

#[cfg(feature = "iroh")]
impl IrohRemoteController {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_manager: Arc<crate::session::SessionManager>,
        groups_store: Arc<crate::groups::GroupsStore>,
        thinking_setter: Option<crate::ThinkingSetter>,
        auto_summary_enabled: tokio::sync::watch::Sender<bool>,
        store: Arc<RemoteAccessStore>,
        secret_key: iroh::SecretKey,
        allowlist_path: PathBuf,
        remote_mode: RemoteMode,
    ) -> Self {
        let node_id = secret_key.public().to_string();
        Self {
            session_manager,
            groups_store,
            thinking_setter,
            auto_summary_enabled,
            store,
            secret_key,
            node_id,
            allowlist_path,
            remote_mode,
            enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_error: Arc::new(Mutex::new(None)),
            state: Arc::new(tokio::sync::Mutex::new(IrohControllerState::default())),
        }
    }

    /// Register an already-started external `run_iroh` listener (e.g. from `mew daemon --remote`).
    pub fn mark_running(&self, shutdown_tx: tokio::sync::watch::Sender<Option<i32>>) {
        self.enabled
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut guard) = self.state.try_lock() {
            guard.generation = guard.generation.wrapping_add(1);
            guard.running = Some(RunningIroh {
                shutdown_tx,
                handle: None,
            });
        }
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

#[cfg(feature = "iroh")]
impl RemoteAccessController for IrohRemoteController {
    fn status(&self) -> mew_protocol::RemoteAccessInfo {
        let enabled = self.enabled.load(std::sync::atomic::Ordering::Acquire)
            || self.store.snapshot().enabled;
        mew_protocol::RemoteAccessInfo {
            supported: true,
            enabled,
            node_id: Some(self.node_id.clone()),
        }
    }

    fn set_enabled(&self, enabled: bool) -> RemoteFuture<'_, mew_protocol::RemoteAccessInfo> {
        Box::pin(async move {
            let mut guard = self.state.lock().await;
            if enabled {
                if guard.running.as_ref().is_some_and(|running| {
                    running
                        .handle
                        .as_ref()
                        .is_none_or(|handle| !handle.is_finished())
                }) {
                    return Ok(self.status());
                }
                guard.running = None;
                *self.last_error.lock().expect("iroh error mutex poisoned") = None;
                guard.generation = guard.generation.wrapping_add(1);
                let generation = guard.generation;
                let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(None::<i32>);

                let session_manager = self.session_manager.clone();
                let groups_store = self.groups_store.clone();
                let thinking_setter = self.thinking_setter.clone();
                let auto_summary_enabled = self.auto_summary_enabled.clone();
                let store = self.store.clone();
                let secret_key = self.secret_key.clone();
                let allowlist_path = self.allowlist_path.clone();
                let remote_mode = self.remote_mode;
                let enabled_flag = self.enabled.clone();
                let last_error = self.last_error.clone();
                let state = self.state.clone();

                let handle = tokio::spawn(async move {
                    let result = crate::iroh_transport::run_iroh(
                        session_manager.clone(),
                        groups_store,
                        thinking_setter,
                        auto_summary_enabled,
                        crate::iroh_transport::IrohListenerConfig {
                            allowlist_path,
                            secret_key,
                            remote_scope: Some(RemoteScope::Control),
                            remote_token: None,
                            remote_store: Some(store.clone()),
                            remote_mode,
                            shutdown: Some(shutdown_rx),
                        },
                    )
                    .await;
                    enabled_flag.store(false, std::sync::atomic::Ordering::Release);
                    let _ = store.set_hosting(false, remote_mode, None);
                    if let Err(error) = result {
                        tracing::error!(%error, "iroh remote listener stopped");
                        *last_error.lock().expect("iroh error mutex poisoned") =
                            Some(format!("{error:#}"));
                    }
                    if let Ok(mut guard) = state.try_lock() {
                        if guard.generation == generation {
                            guard.running = None;
                        }
                    }
                    session_manager.settings_changed.send_replace(());
                });
                guard.running = Some(RunningIroh {
                    shutdown_tx,
                    handle: Some(handle),
                });

                let deadline = tokio::time::Instant::now() + Duration::from_secs(16);
                loop {
                    if self.store.snapshot().enabled {
                        self.enabled
                            .store(true, std::sync::atomic::Ordering::Release);
                        return Ok(self.status());
                    }
                    if guard.running.as_ref().is_some_and(|running| {
                        running
                            .handle
                            .as_ref()
                            .is_some_and(|handle| handle.is_finished())
                    }) {
                        if let Some(mut running) = guard.running.take() {
                            if let Some(handle) = running.handle.take() {
                                let _ = handle.await;
                            }
                        }
                        self.enabled
                            .store(false, std::sync::atomic::Ordering::Release);
                        let message = self
                            .last_error
                            .lock()
                            .expect("iroh error mutex poisoned")
                            .take()
                            .unwrap_or_else(|| "iroh listener stopped unexpectedly".into());
                        anyhow::bail!(message);
                    }
                    if tokio::time::Instant::now() >= deadline {
                        if let Some(mut running) = guard.running.take() {
                            let _ = running.shutdown_tx.send(Some(0));
                            if let Some(handle) = running.handle.take() {
                                handle.abort();
                            }
                        }
                        self.enabled
                            .store(false, std::sync::atomic::Ordering::Release);
                        anyhow::bail!("timed out starting iroh remote listener");
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            } else {
                if let Some(mut running) = guard.running.take() {
                    let _ = running.shutdown_tx.send(Some(0));
                    if let Some(mut handle) = running.handle.take() {
                        if tokio::time::timeout(Duration::from_secs(3), &mut handle)
                            .await
                            .is_err()
                        {
                            handle.abort();
                        }
                    }
                }
                self.enabled
                    .store(false, std::sync::atomic::Ordering::Release);
                let store = self.store.clone();
                let mode = self.remote_mode;
                tokio::task::spawn_blocking(move || store.set_hosting(false, mode, None)).await??;
                Ok(self.status())
            }
        })
    }

    fn create_pairing_invite(
        &self,
        scope: RemoteScope,
        ttl_secs: u64,
    ) -> RemoteFuture<'_, (String, String, u64)> {
        Box::pin(async move {
            let store = self.store.clone();
            let node_id = self.node_id.clone();
            let active = self.enabled.load(std::sync::atomic::Ordering::Acquire)
                || self.store.snapshot().enabled;
            anyhow::ensure!(
                active,
                "remote access is not active; enable remote access before creating a pairing invite"
            );
            let ttl = ttl_secs.clamp(30, 600);
            let now = unix_now();
            let token = tokio::task::spawn_blocking(move || store.create_pairing(scope, now, ttl))
                .await??;
            let payload = format!("computer.mew.mew://{node_id}?token={token}");
            Ok((node_id, payload, now.saturating_add(ttl)))
        })
    }
}

/// How the remote endpoint is being hosted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteMode {
    /// A deliberately long-lived endpoint, typically on a VPS or home server.
    #[default]
    Daemon,
    /// An endpoint owned by the desktop app and stopped with its daemon.
    Desktop,
}

pub fn scope_allows_prompt(scope: RemoteScope) -> bool {
    matches!(scope, RemoteScope::Collaborate | RemoteScope::Control)
}

pub fn scope_allows_control(scope: RemoteScope) -> bool {
    matches!(scope, RemoteScope::Control)
}

/// A device that has completed pairing with a remote endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    pub node_id: String,
    pub scope: RemoteScope,
    pub paired_at: u64,
    #[serde(default)]
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub revoked_at: Option<u64>,
}

impl RemoteDevice {
    pub fn is_active_at(&self, now: u64) -> bool {
        self.revoked_at.is_none() && self.expires_at.map(|at| at > now).unwrap_or(true)
    }
}

/// Persisted configuration for either remote hosting mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAccessState {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: RemoteMode,
    #[serde(default)]
    pub endpoint_id: Option<String>,
    #[serde(default)]
    pub devices: Vec<RemoteDevice>,
    #[serde(default)]
    pub pairings: Vec<RemotePairing>,
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// A short-lived pairing credential. Only the digest is persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePairing {
    pub id: String,
    pub token_sha256: String,
    pub scope: RemoteScope,
    pub created_at: u64,
    pub expires_at: u64,
}

/// Persistence boundary for remote access state. The raw pairing token exists
/// only in the invite payload and is never written to disk or logs.
pub struct RemoteAccessStore {
    state: Mutex<RemoteAccessState>,
    path: PathBuf,
}

impl std::fmt::Debug for RemoteAccessStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteAccessStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl RemoteAccessStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        let state = if path.exists() {
            let bytes = std::fs::read(&path)
                .with_context(|| format!("read remote access state {}", path.display()))?;
            serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                tracing::warn!(%error, path = %path.display(), "remote access state is invalid; starting disabled");
                RemoteAccessState::default()
            })
        } else {
            RemoteAccessState::default()
        };
        Ok(Self {
            state: Mutex::new(state),
            path,
        })
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            state: Mutex::new(RemoteAccessState::default()),
            path,
        }
    }

    /// Refresh state written by another process, such as `mew pair` while a
    /// daemon is already running. Pairing state is intentionally file-backed
    /// so the short-lived CLI command does not need to reach into the daemon.
    fn refresh_from_disk(&self) -> Result<()> {
        if !self.path.exists() {
            *self.state.lock().expect("remote state mutex poisoned") = RemoteAccessState::default();
            return Ok(());
        }
        let bytes = std::fs::read(&self.path)
            .with_context(|| format!("read remote access state {}", self.path.display()))?;
        let state = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode remote access state {}", self.path.display()))?;
        *self.state.lock().expect("remote state mutex poisoned") = state;
        Ok(())
    }

    pub fn snapshot(&self) -> RemoteAccessState {
        self.state
            .lock()
            .expect("remote state mutex poisoned")
            .clone()
    }

    pub fn read_state(&self) -> Result<RemoteAccessState> {
        let _lock = self.acquire_file_lock()?;
        self.refresh_from_disk()?;
        Ok(self.snapshot())
    }

    pub fn set_hosting(
        &self,
        enabled: bool,
        mode: RemoteMode,
        endpoint_id: Option<String>,
    ) -> Result<()> {
        let _lock = self.acquire_file_lock()?;
        self.refresh_from_disk()?;
        let mut state = self.state.lock().expect("remote state mutex poisoned");
        state.enabled = enabled;
        state.mode = mode;
        state.endpoint_id = endpoint_id;
        self.save_locked(&state)
    }

    pub fn revoke(&self, device_id: &str, now: u64) -> Result<bool> {
        self.revoke_as(device_id, now, None)
    }

    pub(crate) fn revoke_as(&self, device_id: &str, now: u64, actor: Option<&str>) -> Result<bool> {
        let _lock = self.acquire_file_lock()?;
        self.refresh_from_disk()?;
        let mut state = self.state.lock().expect("remote state mutex poisoned");
        if let Some(actor) = actor {
            anyhow::ensure!(
                state.enabled
                    && state
                        .active_devices(now)
                        .any(|device| device.node_id == actor
                            && device.scope == RemoteScope::Control),
                "this device no longer has full control"
            );
        }
        let mut next = state.clone();
        let changed = next.revoke(device_id, now);
        if changed {
            self.save_locked(&next)?;
            *state = next;
        }
        Ok(changed)
    }

    pub fn revoke_all(&self, now: u64) -> Result<()> {
        let _lock = self.acquire_file_lock()?;
        self.refresh_from_disk()?;
        let mut state = self.state.lock().expect("remote state mutex poisoned");
        state.revoke_all(now);
        self.save_locked(&state)
    }

    pub fn create_pairing(&self, scope: RemoteScope, now: u64, ttl_secs: u64) -> Result<String> {
        let _lock = self.acquire_file_lock()?;
        self.refresh_from_disk()?;
        let token = format!("mew_{}", ulid::Ulid::new());
        let pairing = RemotePairing {
            id: format!("pair_{}", ulid::Ulid::new()),
            token_sha256: digest_token(&token),
            scope,
            created_at: now,
            expires_at: now.saturating_add(ttl_secs),
        };
        let mut state = self.state.lock().expect("remote state mutex poisoned");
        state.pairings.retain(|entry| entry.expires_at > now);
        state.pairings.push(pairing);
        self.save_locked(&state)?;
        Ok(token)
    }

    pub fn authorize_token(
        &self,
        token: &str,
        node_id: &str,
        device_name: &str,
        now: u64,
    ) -> Result<Option<RemoteScope>> {
        let _lock = self.acquire_file_lock()?;
        self.refresh_from_disk()?;
        let digest = digest_token(token);
        let mut state = self.state.lock().expect("remote state mutex poisoned");
        let Some(index) = state
            .pairings
            .iter()
            .position(|entry| entry.expires_at > now && entry.token_sha256 == digest)
        else {
            state.pairings.retain(|entry| entry.expires_at > now);
            self.save_locked(&state)?;
            return Ok(None);
        };
        let pairing = state.pairings.remove(index);
        let device = RemoteDevice {
            id: format!("device_{}", ulid::Ulid::new()),
            name: device_name.trim().chars().take(80).collect(),
            node_id: node_id.to_owned(),
            scope: pairing.scope,
            paired_at: now,
            expires_at: None,
            revoked_at: None,
        };
        let scope = device.scope;
        state
            .devices
            .retain(|existing| existing.node_id != node_id || existing.revoked_at.is_some());
        state.devices.push(device);
        self.save_locked(&state)?;
        Ok(Some(scope))
    }

    pub fn authorize_device(&self, node_id: &str, now: u64) -> Result<Option<RemoteScope>> {
        let _lock = self.acquire_file_lock()?;
        self.refresh_from_disk()?;
        Ok(self
            .state
            .lock()
            .expect("remote state mutex poisoned")
            .active_devices(now)
            .find(|device| device.node_id == node_id)
            .map(|device| device.scope))
    }

    fn save_locked(&self, state: &RemoteAccessState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    fn acquire_file_lock(&self) -> Result<RemoteFileLock> {
        acquire_file_lock(&self.path)
    }
}

pub(crate) fn acquire_file_lock(path: &Path) -> Result<RemoteFileLock> {
    let lock_path = path.with_extension("json.lock");
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for _ in 0..100 {
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
        {
            Ok(_) => return Ok(RemoteFileLock { path: lock_path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("timed out waiting for remote access state lock")
}

pub(crate) struct RemoteFileLock {
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum RemoteDeviceManager {
    Scoped(Arc<RemoteAccessStore>),
    #[cfg(feature = "iroh")]
    Legacy(Arc<crate::iroh_transport::NodeIdAllowlist>),
}

impl RemoteDeviceManager {
    pub fn devices(&self, now: u64) -> Result<Vec<mew_protocol::PairedDeviceInfo>> {
        match self {
            Self::Scoped(store) => Ok(store
                .read_state()?
                .active_devices(now)
                .map(|device| mew_protocol::PairedDeviceInfo {
                    id: device.id.clone(),
                    name: device.name.clone(),
                    node_id: device.node_id.clone(),
                    scope: Some(device.scope),
                    paired_at: Some(device.paired_at),
                })
                .collect()),
            #[cfg(feature = "iroh")]
            Self::Legacy(store) => Ok(store
                .read_nodes()?
                .into_iter()
                .map(|id| mew_protocol::PairedDeviceInfo {
                    id: id.clone(),
                    name: String::new(),
                    node_id: id,
                    scope: None,
                    paired_at: None,
                })
                .collect()),
        }
    }

    pub fn revoke(&self, id: &str, now: u64) -> Result<()> {
        self.revoke_as(id, now, None)
    }

    pub(crate) fn revoke_as(&self, id: &str, now: u64, actor: Option<&str>) -> Result<()> {
        match self {
            Self::Scoped(store) => {
                anyhow::ensure!(
                    store.revoke_as(id, now, actor)?,
                    "device is no longer paired"
                );
            }
            #[cfg(feature = "iroh")]
            Self::Legacy(store) => store.remove_as(id, actor)?,
        }
        Ok(())
    }

    pub fn authorized_scope(&self, node_id: &str, now: u64) -> Result<Option<RemoteScope>> {
        match self {
            Self::Scoped(store) => {
                let state = store.read_state()?;
                if !state.enabled {
                    return Ok(None);
                }
                let scope = state
                    .active_devices(now)
                    .find(|device| device.node_id == node_id)
                    .map(|device| device.scope);
                Ok(scope)
            }
            #[cfg(feature = "iroh")]
            Self::Legacy(store) => Ok(store
                .read_nodes()?
                .iter()
                .any(|id| id == node_id)
                .then_some(RemoteScope::Control)),
        }
    }
}

impl Drop for RemoteFileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn digest_token(token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(token.as_bytes());
    format!("{:x}", digest.finalize())
}

pub fn default_state_path() -> PathBuf {
    let session_dir = mew_session::session_dir();
    session_dir
        .parent()
        .map(|parent| parent.join("remote.json"))
        .unwrap_or_else(|| Path::new("remote.json").to_path_buf())
}

impl RemoteAccessState {
    pub fn active_devices(&self, now: u64) -> impl Iterator<Item = &RemoteDevice> {
        self.devices
            .iter()
            .filter(move |device| device.is_active_at(now))
    }

    pub fn revoke(&mut self, id: &str, now: u64) -> bool {
        let Some(device) = self.devices.iter_mut().find(|device| device.id == id) else {
            return false;
        };
        device.revoked_at = Some(now);
        true
    }

    pub fn revoke_all(&mut self, now: u64) {
        for device in &mut self.devices {
            device.revoked_at = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(scope: RemoteScope) -> RemoteDevice {
        RemoteDevice {
            id: "device-1".into(),
            name: "laptop".into(),
            node_id: "node-1".into(),
            scope,
            paired_at: 10,
            expires_at: Some(100),
            revoked_at: None,
        }
    }

    #[test]
    fn scopes_have_explicit_capabilities() {
        assert!(!scope_allows_prompt(RemoteScope::Observe));
        assert!(scope_allows_prompt(RemoteScope::Collaborate));
        assert!(!scope_allows_control(RemoteScope::Collaborate));
        assert!(scope_allows_control(RemoteScope::Control));
    }

    #[test]
    fn expired_and_revoked_devices_are_not_active() {
        let mut state = RemoteAccessState {
            enabled: true,
            devices: vec![device(RemoteScope::Observe)],
            ..Default::default()
        };
        assert_eq!(state.active_devices(99).count(), 1);
        assert_eq!(state.active_devices(100).count(), 0);
        assert!(state.revoke("device-1", 20));
        assert_eq!(state.active_devices(20).count(), 0);
        assert!(!state.revoke("missing", 20));
    }

    #[test]
    fn remote_state_roundtrips_with_safe_defaults() {
        let state = RemoteAccessState {
            enabled: true,
            mode: RemoteMode::Desktop,
            endpoint_id: Some("node-1".into()),
            devices: vec![device(RemoteScope::Collaborate)],
            ..Default::default()
        };
        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: RemoteAccessState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, state);

        let legacy: RemoteAccessState = serde_json::from_str("{}").unwrap();
        assert!(!legacy.enabled);
        assert_eq!(legacy.mode, RemoteMode::Daemon);
    }

    #[test]
    fn pairing_tokens_are_single_use_and_expire() {
        let dir = tempfile::tempdir().unwrap();
        let store = RemoteAccessStore::new(dir.path().join("remote.json"));
        let token = store
            .create_pairing(RemoteScope::Collaborate, 100, 60)
            .unwrap();

        assert_eq!(
            store
                .authorize_token(&token, "node-1", "laptop", 159)
                .unwrap(),
            Some(RemoteScope::Collaborate)
        );
        assert_eq!(
            store
                .authorize_token(&token, "node-2", "replay", 159)
                .unwrap(),
            None
        );
    }

    #[test]
    fn expired_pairing_is_rejected_without_persisting_the_raw_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.json");
        let store = RemoteAccessStore::new(path.clone());
        let token = store.create_pairing(RemoteScope::Observe, 100, 10).unwrap();
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(!contents.contains(&token));
        assert_eq!(
            store
                .authorize_token(&token, "node-1", "late", 110)
                .unwrap(),
            None
        );
    }

    #[test]
    fn authorization_refreshes_pairings_created_by_another_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.json");
        let daemon_store = RemoteAccessStore::load(path.clone()).unwrap();
        let cli_store = RemoteAccessStore::new(path);
        let token = cli_store
            .create_pairing(RemoteScope::Control, 100, 60)
            .unwrap();

        assert_eq!(
            daemon_store
                .authorize_token(&token, "node-1", "laptop", 101)
                .unwrap(),
            Some(RemoteScope::Control)
        );
    }

    #[test]
    fn device_management_reads_external_pairings_and_revokes_persistently_without_exposing_invites()
    {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("remote.json");
        let daemon_store = Arc::new(RemoteAccessStore::load(path.clone()).unwrap());
        let cli = RemoteAccessStore::new(path);
        cli.set_hosting(true, RemoteMode::Daemon, None).unwrap();
        let token = cli.create_pairing(RemoteScope::Control, 100, 60).unwrap();
        cli.authorize_token(&token, "phone", "My phone", 101)
            .unwrap();
        let manager = RemoteDeviceManager::Scoped(daemon_store);
        let devices = manager.devices(102).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "My phone");
        assert!(!serde_json::to_string(&devices).unwrap().contains(&token));
        assert_eq!(
            manager.authorized_scope("phone", 102).unwrap(),
            Some(RemoteScope::Control)
        );
        manager.revoke(&devices[0].id, 103).unwrap();
        assert_eq!(cli.authorize_device("phone", 104).unwrap(), None);
        assert!(manager.devices(104).unwrap().is_empty());
        assert_eq!(manager.authorized_scope("phone", 104).unwrap(), None);
    }

    #[test]
    fn failed_device_revocation_keeps_the_authorized_device() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("remote.json");
        let store = RemoteAccessStore::new(path);
        let token = store.create_pairing(RemoteScope::Control, 100, 60).unwrap();
        store
            .authorize_token(&token, "phone", "My phone", 101)
            .unwrap();
        let device = store.snapshot().devices[0].id.clone();
        std::fs::create_dir(root.path().join("remote.json.tmp")).unwrap();
        assert!(store.revoke(&device, 102).is_err());
        assert_eq!(store.snapshot().active_devices(103).count(), 1);
    }

    #[test]
    fn removing_the_remote_state_file_invalidates_cached_authorization() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("remote.json");
        let store = Arc::new(RemoteAccessStore::new(path.clone()));
        store.set_hosting(true, RemoteMode::Daemon, None).unwrap();
        let token = store.create_pairing(RemoteScope::Control, 100, 60).unwrap();
        store
            .authorize_token(&token, "phone", "Phone", 101)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        let manager = RemoteDeviceManager::Scoped(store);
        assert_eq!(manager.authorized_scope("phone", 102).unwrap(), None);
    }
}
