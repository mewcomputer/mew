//! Restart-capable transport layer for plugin subprocesses.
//!
//! [`PluginSlot`] wraps a [`PluginProcess`] with restart capability.
//! When a plugin's stdout closes or errors, the reader task spawns a
//! restart-with-backoff sequence instead of disabling the plugin for
//! the session.
//!
//! Tool closures capture a `watch::Receiver<PluginHandles>` so they
//! always see the *current* process's I/O handles — after a restart,
//! the next tool call transparently uses the new process.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::Context;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use mew_hooks::PluginHost;

use crate::runtime::handle_plugin_message;

// ── PluginHandles ──────────────────────────────────────────────────

/// Type alias for the pending-requests map.
/// Each entry is a oneshot sender carrying `Ok(result_string)` on
/// success or `Err(error_message)` when the plugin dies mid-call.
pub(crate) type PendingMap = Arc<AsyncMutex<HashMap<u64, oneshot::Sender<Result<String, String>>>>>;

/// Arc-wrapped I/O handles for a running plugin process.
///
/// All fields are `Arc`, so cloning is cheap (refcount bumps) and the
/// clone is safe to hold across `.await`. Tool closures capture a
/// `watch::Receiver<PluginHandles>` and clone out of `borrow()` before
/// awaiting.
#[derive(Clone)]
pub struct PluginHandles {
    pub writer: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    pub pending: PendingMap,
    pub healthy: Arc<AtomicBool>,
    pub timeout: Duration,
    /// Per-process request ID counter. Fresh on each restart.
    pub next_id: Arc<AtomicU64>,
}

// ── PluginProcess (inner) ──────────────────────────────────────────

/// A running plugin subprocess with multiplexed JSON-RPC transport.
///
/// This is the inner type owned by [`PluginSlot`]. It does NOT own the
/// reader task — that's owned by `PluginSlot` so it can be aborted on
/// restart.
pub(crate) struct PluginProcess {
    pub(crate) name: String,
    pub(crate) _child: Child,
    pub(crate) pending: PendingMap,
    pub(crate) writer: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    pub(crate) next_id: Arc<AtomicU64>,
    pub(crate) timeout: Duration,
    pub(crate) healthy: Arc<AtomicBool>,
}

impl PluginProcess {
    /// Spawn the subprocess and return the process + its handles + stdout.
    /// The caller (PluginSlot) owns the reader task.
    pub(crate) async fn spawn(
        path: &PathBuf,
        timeout: Duration,
    ) -> anyhow::Result<(Self, PluginHandles, tokio::process::ChildStdout)> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mut child = Command::new(path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("spawn plugin")?;

        let stdout = child.stdout.take().context("plugin stdout")?;
        let stdin = child.stdin.take().context("plugin stdin")?;

        let pending: PendingMap = Arc::new(AsyncMutex::new(HashMap::new()));
        let writer = Arc::new(tokio::sync::Mutex::new(Some(stdin)));
        let healthy = Arc::new(AtomicBool::new(true));
        let next_id = Arc::new(AtomicU64::new(1));

        let handles = PluginHandles {
            writer: writer.clone(),
            pending: pending.clone(),
            healthy: healthy.clone(),
            timeout,
            next_id: next_id.clone(),
        };

        let process = Self {
            name,
            _child: child,
            pending,
            writer,
            next_id,
            timeout,
            healthy,
        };

        Ok((process, handles, stdout))
    }

    /// Send a request to the plugin and await the response.
    pub(crate) async fn call(&self, method: &str, params: &Value) -> anyhow::Result<String> {
        if !self.healthy.load(Ordering::Acquire) {
            return Err(anyhow::anyhow!(
                "plugin '{}' is not running; skipping call to '{}'",
                self.name,
                method
            ));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let (tx, rx) = oneshot::channel::<Result<String, String>>();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        {
            let mut w = self.writer.lock().await;
            let Some(w) = w.as_mut() else {
                self.healthy.store(false, Ordering::Release);
                return Err(anyhow::anyhow!(
                    "plugin '{}' stdin closed; skipping call to '{}'",
                    self.name,
                    method
                ));
            };
            w.write_all(line.as_bytes()).await?;
            w.flush().await?;
        }

        let result = tokio::time::timeout(self.timeout, rx)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "plugin '{}' timed out after {:?} on method '{}'",
                    self.name,
                    self.timeout,
                    method
                )
            })?
            .context("plugin response channel closed")?;

        {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
        }

        result.map_err(|e| anyhow::anyhow!(e))
    }
}

// ── ExtensionConnection trait ──────────────────────────────────────

/// Transport interface that the extension broker (W4) consumes.
///
/// `PluginSlot` is the concrete implementation for stdio-spawned
/// subprocesses. A socket-attached transport (Phase 2) will implement
/// the same trait.
//
// TODO(W4): This trait has no implementors yet — it will be implemented
// by the broker when W4 is built. Marked #[doc(hidden)] to avoid
// implying it's ready for use.
#[doc(hidden)]
#[async_trait::async_trait]
pub trait ExtensionConnection: Send + Sync {
    /// Request/response call to the extension.
    async fn call(&self, method: &str, params: &Value) -> anyhow::Result<String>;
    /// Fire-and-forget notification to the extension.
    async fn notify(&self, method: &str, params: &Value);
    /// Whether the extension process is alive and accepting calls.
    fn is_healthy(&self) -> bool;
    /// Graceful shutdown.
    async fn shutdown(&self);
    /// Receiver for the current I/O handles (for tool closures).
    fn handles(&self) -> watch::Receiver<PluginHandles>;
    /// Extension name (for logging, sorting, config lookup).
    fn name(&self) -> &str;
}

// ── PluginSlot ─────────────────────────────────────────────────────

/// A restartable plugin slot.
///
/// Owns the process, reader task, and a `watch` channel for passing
/// updated handles to tool closures after a restart. Stored as
/// `Arc<PluginSlot>` in the dispatcher's `Vec`.
pub struct PluginSlot {
    name: String,
    path: PathBuf,
    timeout: Duration,
    host: PluginHost,
    /// The current process. `None` during the restart window.
    /// `std::sync::Mutex` — lock is held briefly, never across `.await`.
    process: StdMutex<Option<PluginProcess>>,
    /// Watch channel: sender updates handles on restart, receiver is
    /// captured by tool closures.
    handles_tx: watch::Sender<PluginHandles>,
    /// Reader task handle — aborted on restart.
    reader: StdMutex<Option<JoinHandle<()>>>,
    /// Backoff schedule for restarts.
    restart_attempts: StdMutex<u32>,
    /// Guard against concurrent restart: only one `do_restart_with_backoff`
    /// may run at a time. Set to `true` at entry, reset on success or
    /// exhaustion.
    restarting: AtomicBool,
}

/// Backoff schedule: 200ms, 5s, 30s, then give up.
/// First attempt is fast to minimize the gate-enforcement gap.
const BACKOFF_SCHEDULE: &[Duration] = &[
    Duration::from_millis(200),
    Duration::from_secs(5),
    Duration::from_secs(30),
];

impl PluginSlot {
    /// Spawn a plugin as a restartable slot.
    pub async fn spawn(
        path: PathBuf,
        host: PluginHost,
        timeout: Duration,
    ) -> anyhow::Result<Arc<Self>> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        info!("starting plugin: {} ({})", name, path.display());

        let (process, handles, stdout) = PluginProcess::spawn(&path, timeout).await?;

        // Create the watch channel with the initial handles.
        let (handles_tx, _handles_rx) = watch::channel(handles.clone());

        let slot = Arc::new(Self {
            name: name.clone(),
            path,
            timeout,
            host,
            process: StdMutex::new(Some(process)),
            handles_tx,
            reader: StdMutex::new(None),
            restart_attempts: StdMutex::new(0),
            restarting: AtomicBool::new(false),
        });

        // Spawn the reader task.
        let reader_handle = spawn_reader_task(slot.clone(), stdout, handles, name);
        *slot.reader.lock().unwrap() = Some(reader_handle);

        info!("plugin started: {}", slot.name);
        Ok(slot)
    }

    /// Get a receiver for the current handles. Tool closures call this
    /// once and use `borrow()` on each invocation.
    pub fn handles_receiver(&self) -> watch::Receiver<PluginHandles> {
        self.handles_tx.subscribe()
    }

    /// Alias for `handles_receiver` — the name the Dispatcher expects.
    pub fn handles(&self) -> watch::Receiver<PluginHandles> {
        self.handles_receiver()
    }

    /// Whether the current process is healthy.
    pub fn is_healthy(&self) -> bool {
        let process = self.process.lock().unwrap();
        match &*process {
            Some(p) => p.healthy.load(Ordering::Acquire),
            None => false,
        }
    }

    /// Initiate an external restart (e.g. from `mew ext dev` file-watch).
    /// The caller must pass the `Arc<PluginSlot>` so the spawned task
    /// can keep the slot alive across the backoff loop.
    pub fn restart(self: &Arc<Self>) {
        let slot = Arc::clone(self);
        let name = self.name.clone();
        tokio::spawn(async move {
            warn!("plugin {}: external restart requested", name);
            do_restart_with_backoff(slot).await;
        });
    }

    /// Perform the actual restart: abort old reader, drop old process,
    /// spawn new process, send new handles.
    async fn do_restart(slot: &Arc<Self>) -> anyhow::Result<()> {
        // Abort old reader task if still alive.
        {
            let mut reader = slot.reader.lock().unwrap();
            if let Some(handle) = reader.take() {
                handle.abort();
            }
        }

        // Drop old process (kills child via kill_on_drop).
        // Drain its pending requests first — but we can't hold the
        // std Mutex guard across .await, so take the process out,
        // drop the guard, then drain.
        let old_process: Option<PluginProcess> = {
            let mut guard = slot.process.lock().unwrap();
            guard.take()
        };

        if let Some(old) = old_process {
            // Mark old as unhealthy.
            old.healthy.store(false, Ordering::Release);
            // Drain pending — fail any in-flight requests.
            let pending = old.pending.clone();
            let mut pending_guard = pending.lock().await;
            for (_id, tx) in pending_guard.drain() {
                let _ = tx.send(Err("plugin died".to_string()));
            }
            // old drops here — kills child via kill_on_drop
        }

        // Spawn new process.
        let (new_process, new_handles, stdout) =
            PluginProcess::spawn(&slot.path, slot.timeout).await?;

        // Send new handles through the watch channel.
        let _ = slot.handles_tx.send(new_handles.clone());

        // Store new process.
        *slot.process.lock().unwrap() = Some(new_process);

        // Spawn new reader task.
        let reader_handle = spawn_reader_task(slot.clone(), stdout, new_handles, slot.name.clone());
        *slot.reader.lock().unwrap() = Some(reader_handle);

        info!("plugin {} restarted", slot.name);
        Ok(())
    }

    /// Call a method on the current process.
    pub async fn call(&self, method: &str, params: &Value) -> anyhow::Result<String> {
        // Clone the Arc to the process's call data via the watch receiver.
        // This avoids holding the std Mutex across .await.
        let receiver = self.handles_tx.subscribe();
        let handles = receiver.borrow().clone();

        if !handles.healthy.load(Ordering::Acquire) {
            return Err(anyhow::anyhow!(
                "plugin '{}' is not running; skipping call to '{}'",
                self.name,
                method
            ));
        }

        // Use the handles directly — they're Arc-wrapped, safe across await.
        call_via_handles(&self.name, method, params, &handles).await
    }

    /// Notify the current process (fire-and-forget).
    pub async fn notify(&self, method: &str, params: &Value) {
        let receiver = self.handles_tx.subscribe();
        let handles = receiver.borrow().clone();

        if !handles.healthy.load(Ordering::Acquire) {
            return;
        }

        notify_via_handles(&self.name, method, params, &handles).await;
    }

    /// Graceful shutdown.
    pub async fn shutdown(&self) {
        let process = self.process.lock().unwrap().take();
        if let Some(p) = process {
            let _ = p.call("shutdown", &serde_json::json!({})).await;
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Run a request/response call via cloned handles (no Mutex guard held).
pub async fn call_via_handles(
    name: &str,
    method: &str,
    params: &Value,
    handles: &PluginHandles,
) -> anyhow::Result<String> {
    if !handles.healthy.load(Ordering::Acquire) {
        return Err(anyhow::anyhow!(
            "plugin '{}' is not running; skipping call to '{}'",
            name,
            method
        ));
    }

    let id = handles.next_id.fetch_add(1, Ordering::Relaxed);
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": id,
    });

    let (tx, rx) = oneshot::channel::<Result<String, String>>();
    {
        let mut pending = handles.pending.lock().await;
        pending.insert(id, tx);
    }

    let mut line = serde_json::to_string(&request)?;
    line.push('\n');
    {
        let mut w = handles.writer.lock().await;
        let Some(w) = w.as_mut() else {
            handles.healthy.store(false, Ordering::Release);
            return Err(anyhow::anyhow!(
                "plugin '{}' stdin closed; skipping call to '{}'",
                name,
                method
            ));
        };
        w.write_all(line.as_bytes()).await?;
        w.flush().await?;
    }

    let result = tokio::time::timeout(handles.timeout, rx)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "plugin '{}' timed out after {:?} on method '{}'",
                name,
                handles.timeout,
                method
            )
        })?
        .context("plugin response channel closed")?;

    {
        let mut pending = handles.pending.lock().await;
        pending.remove(&id);
    }

    result.map_err(|e| anyhow::anyhow!(e))
}

/// Run a fire-and-forget notification via cloned handles.
pub(crate) async fn notify_via_handles(
    name: &str,
    method: &str,
    params: &Value,
    handles: &PluginHandles,
) {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });

    if !handles.healthy.load(Ordering::Acquire) {
        return;
    }

    let mut line = match serde_json::to_string(&request) {
        Ok(l) => l,
        Err(e) => {
            error!(
                "plugin {} failed to serialize {} notification: {}",
                name, method, e
            );
            return;
        }
    };
    line.push('\n');

    let mut w = handles.writer.lock().await;
    let Some(w) = w.as_mut() else {
        return;
    };
    if let Err(e) = w.write_all(line.as_bytes()).await {
        error!(
            "plugin {} failed to send {} notification: {}",
            name, method, e
        );
    }
    if let Err(e) = w.flush().await {
        error!(
            "plugin {} failed to flush {} notification: {}",
            name, method, e
        );
    }
}

/// Spawn the reader task for a plugin process. The reader monitors stdout,
/// routes responses and host requests, and on death spawns a restart task.
fn spawn_reader_task(
    slot: Arc<PluginSlot>,
    stdout: tokio::process::ChildStdout,
    handles: PluginHandles,
    name: String,
) -> JoinHandle<()> {
    let reader_pending = handles.pending.clone();
    let reader_writer = handles.writer.clone();
    let reader_healthy = handles.healthy.clone();
    let reader_host = slot.host.clone();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        let death_reason: String = loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break "stdout closed".into(),
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Value>(trimmed) {
                        Ok(msg) => {
                            handle_plugin_message(
                                &name,
                                &msg,
                                &reader_pending,
                                &reader_writer,
                                &reader_host,
                            )
                            .await;
                        }
                        Err(e) => {
                            warn!("plugin {} sent unparseable json: {}", name, e);
                        }
                    }
                }
                Err(e) => break format!("read error: {}", e),
            }
        };

        // Plugin died. Mark unhealthy.
        reader_healthy.store(false, Ordering::Release);
        // Drop the writer so call() returns immediately.
        {
            let mut w = reader_writer.lock().await;
            *w = None;
        }
        // Drain in-flight requests.
        let mut pending = reader_pending.lock().await;
        for (_id, tx) in pending.drain() {
            let _ = tx.send(Err("plugin died".to_string()));
        }
        drop(pending);

        error!("plugin {} died: {}", name, death_reason);

        // Spawn a dedicated restart task (not inline — the reader is done).
        let slot_for_restart = slot.clone();
        tokio::spawn(async move {
            do_restart_with_backoff(slot_for_restart).await;
        });
    })
}

/// Restart a plugin with exponential backoff (200ms, 5s, 30s, 3 attempts).
/// On exhaustion, notifies the user and leaves the slot dead.
async fn do_restart_with_backoff(slot: Arc<PluginSlot>) {
    // Guard against concurrent restart: only one do_restart_with_backoff
    // may run at a time. If another restart is already in progress, bail.
    if slot
        .restarting
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        warn!(
            "plugin {}: restart already in progress, skipping duplicate",
            slot.name
        );
        return;
    }

    loop {
        let attempts = {
            let mut a = slot.restart_attempts.lock().unwrap();
            let current = *a;
            *a += 1;
            current
        };

        if attempts >= BACKOFF_SCHEDULE.len() as u32 {
            error!(
                "plugin {} exhausted restart attempts ({}); disabled",
                slot.name, attempts
            );
            (slot.host.notify)(format!(
                "plugin '{}' exhausted restart attempts; disabled for this session",
                slot.name
            ));
            slot.restarting.store(false, Ordering::Release);
            return;
        }

        let backoff = BACKOFF_SCHEDULE[attempts as usize];
        info!(
            "plugin {} restart attempt {} in {:?}",
            slot.name,
            attempts + 1,
            backoff
        );
        tokio::time::sleep(backoff).await;

        match PluginSlot::do_restart(&slot).await {
            Ok(()) => {
                // Reset attempts on success.
                *slot.restart_attempts.lock().unwrap() = 0;
                slot.restarting.store(false, Ordering::Release);
                (slot.host.notify)(format!("plugin '{}' restarted", slot.name));
                return;
            }
            Err(e) => {
                error!(
                    "plugin {} restart attempt {} failed: {}",
                    slot.name,
                    attempts + 1,
                    e
                );
                // Loop to try again with next backoff.
            }
        }
    }
}

// ExtensionConnection is implemented by the broker (W4). PluginSlot
// exposes the same surface via inherent methods; the trait impl will
// be added when the broker is built, to avoid method-name ambiguity
// between inherent and trait methods during W1.
