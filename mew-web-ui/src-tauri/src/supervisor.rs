use anyhow::{anyhow, bail, Context, Result};
use mew_desktop_supervisor::{DaemonEndpoint, DesktopSupervisor, SupervisorConfig};
use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri_plugin_shell::{process::CommandChild, ShellExt};
use tungstenite::{client, Message};

const DEFAULT_DAEMON_PORT: u16 = 25566;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const DESKTOP_REMOTE_STATE: &str = "desktop-remote.json";

/// Tauri owns only the platform sidecar. Endpoint selection, health checks,
/// configured-process ownership, and restart policy live in the shared
/// Tauri-free supervisor.
pub struct DaemonSupervisor {
    app: AppHandle,
    state: Mutex<SupervisorState>,
}

struct SupervisorState {
    core: DesktopSupervisor,
    sidecar: Option<CommandChild>,
    config_error: Option<String>,
    allow_sidecar_fallback: bool,
    sidecar_port: u16,
    remote_enabled: bool,
}

impl DaemonSupervisor {
    pub fn new(app: &AppHandle) -> Self {
        let remote_enabled = load_remote_enabled().unwrap_or_else(|| {
            env::var("MEW_DESKTOP_REMOTE")
                .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        });
        let sidecar_port = env::var("MEW_DESKTOP_DAEMON_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_DAEMON_PORT);

        let (mut config, config_error) = match SupervisorConfig::from_env() {
            Ok(config) => (config, None),
            Err(error) => (SupervisorConfig::default(), Some(error.to_string())),
        };
        config.remote_enabled = remote_enabled;
        if env::var_os("MEW_DESKTOP_DAEMON_PORT").is_none() {
            config.local_port = DEFAULT_DAEMON_PORT;
        }
        config.log_dir = daemon_log_dir();
        if config.daemon_binary.is_none() {
            config.daemon_binary = packaged_daemon_binary();
        }
        let allow_sidecar_fallback =
            config.daemon_binary.is_none() && config.explicit_url.is_none();

        Self {
            app: app.clone(),
            state: Mutex::new(SupervisorState {
                core: DesktopSupervisor::new(config),
                sidecar: None,
                config_error,
                allow_sidecar_fallback,
                sidecar_port,
                remote_enabled,
            }),
        }
    }

    pub fn websocket_url(&self) -> Result<String> {
        let mut state = self.state.lock().expect("daemon supervisor mutex poisoned");
        ensure_endpoint(&self.app, &mut state)
    }

    pub fn set_remote_enabled(&self, enabled: bool) -> Result<String> {
        let mut state = self.state.lock().expect("daemon supervisor mutex poisoned");
        if state.remote_enabled == enabled {
            return ensure_endpoint(&self.app, &mut state);
        }
        if enabled && state.sidecar.is_none() && state.core.endpoint().is_some() {
            bail!("desktop remote access requires an app-owned daemon");
        }

        let previous_remote = state.remote_enabled;
        kill_sidecar(&mut state.sidecar);
        state.core.shutdown()?;
        state.remote_enabled = enabled;
        let endpoint = match state.core.set_remote_enabled(enabled) {
            Ok(endpoint) => endpoint,
            Err(_error) if state.allow_sidecar_fallback => {
                launch_sidecar(&self.app, &mut state)?;
                let url = sidecar_url(state.sidecar_port);
                state
                    .core
                    .attach_existing(url)?
            }
            Err(error) => {
                state.remote_enabled = previous_remote;
                return Err(error);
            }
        };
        save_remote_enabled(enabled)?;
        Ok(endpoint.websocket_url)
    }

    pub fn remote_enabled(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.remote_enabled)
            .unwrap_or(false)
    }
}

fn ensure_endpoint(app: &AppHandle, state: &mut SupervisorState) -> Result<String> {
    if let Some(endpoint) = state.core.endpoint() {
        return Ok(endpoint.websocket_url.clone());
    }
    if let Some(error) = &state.config_error {
        bail!("{error}");
    }

    match state.core.connect_or_launch() {
        Ok(DaemonEndpoint { websocket_url, .. }) => Ok(websocket_url),
        Err(error) if state.allow_sidecar_fallback => {
            tracing::debug!(%error, "configured desktop daemon unavailable; trying sidecar");
            launch_sidecar(app, state)?;
            Ok(state
                .core
                .attach_existing(sidecar_url(state.sidecar_port))?
                .websocket_url)
        }
        Err(error) => Err(error),
    }
}

fn launch_sidecar(app: &AppHandle, state: &mut SupervisorState) -> Result<()> {
    if state.sidecar.is_some() {
        return Ok(());
    }
    let command = match app.shell().sidecar("mew") {
        Ok(command) => command,
        Err(error) => {
            return Err(error).context("bundled mew daemon sidecar is unavailable");
        }
    };
    let port_arg = format!("127.0.0.1:{}", state.sidecar_port);
    let mut command = command.args(["daemon", "--port", &port_arg]);
    if state.remote_enabled {
        command = command.arg("--remote");
        command = command.env("MEW_REMOTE_MODE", "desktop");
    }
    let (events, child) = command
        .spawn()
        .context("spawn the bundled mew daemon sidecar")?;
    drop(events);
    if wait_for_daemon(sidecar_address(state.sidecar_port), STARTUP_TIMEOUT) {
        state.sidecar = Some(child);
        return Ok(());
    }

    let _ = child.kill();
    bail!("bundled mew daemon sidecar did not bind 127.0.0.1:{}", state.sidecar_port)
}

fn kill_sidecar(sidecar: &mut Option<CommandChild>) {
    if let Some(child) = sidecar.take() {
        let _ = child.kill();
    }
}

fn packaged_daemon_binary() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|directory| directory.join("mew")))
        .filter(|binary| binary.is_file())
}

fn daemon_log_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("mew").join("logs"))
}

fn sidecar_address(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn sidecar_url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}")
}

fn wait_for_daemon(address: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if probe_daemon(&format!("ws://{address}")).is_ok() {
            return true;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    false
}

fn probe_daemon(url: &str) -> Result<()> {
    let address = websocket_address(url)?;
    let stream = TcpStream::connect_timeout(&address, PROBE_TIMEOUT)
        .with_context(|| format!("connect to daemon at {url}"))?;
    stream.set_read_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
    let (mut socket, _) = client(url, stream).context("complete daemon websocket handshake")?;
    socket
        .send(Message::Text(r#"{"type":"ping"}"#.into()))
        .context("send daemon health check")?;
    let response = socket.read().context("read daemon health check")?;
    let Message::Text(text) = response else {
        bail!("daemon health check returned a non-text response");
    };
    let value: serde_json::Value = serde_json::from_str(text.as_ref())?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some("pong") {
        bail!("daemon health check returned an unexpected response");
    }
    Ok(())
}

fn websocket_address(url: &str) -> Result<SocketAddr> {
    let authority = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
        .ok_or_else(|| anyhow!("daemon URL must start with ws:// or wss://"))?
        .split('/')
        .next()
        .filter(|authority| !authority.is_empty())
        .ok_or_else(|| anyhow!("daemon URL is missing a host"))?;
    authority
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("daemon URL did not resolve to an address"))
}

fn remote_state_path() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("mew").join(DESKTOP_REMOTE_STATE))
}

fn load_remote_enabled() -> Option<bool> {
    let path = remote_state_path()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&data)
        .ok()?
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
}

fn save_remote_enabled(enabled: bool) -> Result<()> {
    let Some(path) = remote_state_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::json!({ "enabled": enabled }).to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

impl Drop for DaemonSupervisor {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            kill_sidecar(&mut state.sidecar);
        }
    }
}
