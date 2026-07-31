//! Tauri-free daemon lifecycle management for native desktop clients.
//!
//! The supervisor owns only processes it starts. Explicit endpoints and
//! already-running daemons are attach-only and are never killed on shutdown.

use anyhow::{bail, Context, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tungstenite::{client, Message};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonMode {
    LocalOwned,
    LocalExisting,
    RemoteWebSocket,
}

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub explicit_url: Option<String>,
    pub daemon_binary: Option<PathBuf>,
    pub local_port: u16,
    pub remote_enabled: bool,
    pub startup_timeout: Duration,
    pub log_dir: Option<PathBuf>,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            explicit_url: None,
            daemon_binary: None,
            local_port: 0,
            remote_enabled: false,
            startup_timeout: STARTUP_TIMEOUT,
            log_dir: None,
        }
    }
}

impl SupervisorConfig {
    pub fn from_env() -> Result<Self> {
        let local_port = std::env::var("MEW_DESKTOP_DAEMON_PORT")
            .ok()
            .map(|value| parse_port(&value))
            .transpose()?
            .unwrap_or(0);
        Ok(Self {
            explicit_url: std::env::var("MEW_DESKTOP_DAEMON_URL")
                .ok()
                .map(|url| validate_websocket_url(&url).map(|_| url))
                .transpose()?,
            daemon_binary: std::env::var_os("MEW_DESKTOP_DAEMON_BINARY").map(PathBuf::from),
            local_port,
            remote_enabled: std::env::var("MEW_DESKTOP_REMOTE")
                .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            ..Self::default()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub websocket_url: String,
    pub mode: DaemonMode,
}

pub struct DesktopSupervisor {
    config: SupervisorConfig,
    endpoint: Option<DaemonEndpoint>,
    child: Option<Child>,
}

impl DesktopSupervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        Self {
            config,
            endpoint: None,
            child: None,
        }
    }

    pub fn endpoint(&self) -> Option<&DaemonEndpoint> {
        self.endpoint.as_ref()
    }

    /// Adopt a daemon launched by a host-specific process manager.
    ///
    /// The supervisor records the endpoint but does not take ownership of the
    /// process behind it. This is used by desktop shells that launch a
    /// packaged sidecar through their platform integration.
    pub fn attach_existing(&mut self, url: String) -> Result<DaemonEndpoint> {
        validate_websocket_url(&url)?;
        let endpoint = DaemonEndpoint {
            mode: if url.starts_with("wss://") {
                DaemonMode::RemoteWebSocket
            } else {
                DaemonMode::LocalExisting
            },
            websocket_url: url,
        };
        self.endpoint = Some(endpoint.clone());
        Ok(endpoint)
    }

    pub fn set_remote_enabled(&mut self, enabled: bool) -> Result<DaemonEndpoint> {
        if self.config.remote_enabled == enabled {
            return self.connect_or_launch();
        }
        if enabled
            && self
                .endpoint
                .as_ref()
                .is_some_and(|endpoint| endpoint.mode == DaemonMode::LocalExisting)
        {
            bail!("desktop remote access requires an app-owned daemon");
        }
        self.shutdown()?;
        self.config.remote_enabled = enabled;
        self.connect_or_launch()
    }

    pub fn connect_or_launch(&mut self) -> Result<DaemonEndpoint> {
        if let Some(endpoint) = &self.endpoint {
            return Ok(endpoint.clone());
        }

        if let Some(url) = &self.config.explicit_url {
            if self.config.remote_enabled {
                bail!(
                    "MEW_DESKTOP_DAEMON_URL is attach-only and cannot enable desktop remote mode"
                );
            }
            let endpoint = DaemonEndpoint {
                websocket_url: url.clone(),
                mode: if url.starts_with("wss://") {
                    DaemonMode::RemoteWebSocket
                } else {
                    DaemonMode::LocalExisting
                },
            };
            self.endpoint = Some(endpoint.clone());
            return Ok(endpoint);
        }

        let address = allocate_local_address(self.config.local_port)?;
        let url = websocket_url(address);
        if probe_daemon(&url).is_ok() {
            let endpoint = DaemonEndpoint {
                websocket_url: url,
                mode: DaemonMode::LocalExisting,
            };
            self.endpoint = Some(endpoint.clone());
            return Ok(endpoint);
        }

        if TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok() {
            bail!(
                "daemon address {address} is already in use, but it is not a mew daemon; "
                    .to_string()
                    + "set MEW_DESKTOP_DAEMON_URL to attach explicitly or stop the process"
            );
        }

        let binary =
            self.config.daemon_binary.as_deref().ok_or_else(|| {
                anyhow::anyhow!("no daemon binary configured for app-owned launch")
            })?;
        let child = spawn_daemon(
            address,
            binary,
            self.config.remote_enabled,
            self.config.log_dir.as_deref(),
            self.config.startup_timeout,
        )?;
        self.child = Some(child);
        let endpoint = DaemonEndpoint {
            websocket_url: url,
            mode: DaemonMode::LocalOwned,
        };
        self.endpoint = Some(endpoint.clone());
        Ok(endpoint)
    }

    pub fn restart(&mut self) -> Result<DaemonEndpoint> {
        self.shutdown()?;
        self.connect_or_launch()
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.endpoint = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

impl Drop for DesktopSupervisor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn allocate_local_address(port: u16) -> Result<SocketAddr> {
    if port != 0 {
        return Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?)
}

fn spawn_daemon(
    address: SocketAddr,
    binary: &Path,
    remote_enabled: bool,
    log_dir: Option<&Path>,
    startup_timeout: Duration,
) -> Result<Child> {
    let mut command = Command::new(binary);
    let port_arg = format!("127.0.0.1:{}", address.port());
    command.args(["daemon", "--port", &port_arg]);
    if remote_enabled {
        command.arg("--remote");
        command.env("MEW_REMOTE_MODE", "desktop");
    }
    command.stdin(Stdio::null());
    if let Some(log_dir) = log_dir {
        std::fs::create_dir_all(log_dir)?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("desktop-daemon.log"))?;
        command.stdout(Stdio::from(log.try_clone()?));
        command.stderr(Stdio::from(log));
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    close_inherited_descriptors(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn configured daemon {}", binary.display()))?;
    if wait_for_daemon(address, startup_timeout) {
        return Ok(child);
    }
    let _ = child.kill();
    let _ = child.wait();
    bail!(
        "configured daemon {} did not bind {address}",
        binary.display()
    )
}

fn wait_for_daemon(address: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if probe_daemon(&websocket_url(address)).is_ok() {
            return true;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    false
}

fn probe_daemon(url: &str) -> Result<()> {
    validate_websocket_url(url)?;
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
        .ok_or_else(|| anyhow::anyhow!("daemon URL must start with ws:// or wss://"))?
        .split('/')
        .next()
        .filter(|authority| !authority.is_empty())
        .ok_or_else(|| anyhow::anyhow!("daemon URL is missing a host"))?;
    authority
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("daemon URL did not resolve to an address"))
}

fn websocket_url(address: SocketAddr) -> String {
    format!("ws://{address}")
}

fn validate_websocket_url(url: &str) -> Result<()> {
    if url.starts_with("ws://") || url.starts_with("wss://") {
        return Ok(());
    }
    bail!("daemon URL must start with ws:// or wss://")
}

fn parse_port(value: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .with_context(|| "MEW_DESKTOP_DAEMON_PORT must be a valid TCP port")?;
    if port == 0 {
        bail!("MEW_DESKTOP_DAEMON_PORT must be 1-65535 when explicitly set")
    }
    Ok(port)
}

fn close_inherited_descriptors(command: &mut Command) {
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            let limit = libc::sysconf(libc::_SC_OPEN_MAX);
            let limit = if limit > 3 { limit } else { 1024 };
            for fd in 3..limit {
                libc::close(fd as libc::c_int);
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_websocket_endpoint_is_attach_only() {
        let mut supervisor = DesktopSupervisor::new(SupervisorConfig {
            explicit_url: Some("ws://127.0.0.1:43210/ws".into()),
            ..SupervisorConfig::default()
        });
        let endpoint = supervisor.connect_or_launch().unwrap();
        assert_eq!(endpoint.websocket_url, "ws://127.0.0.1:43210/ws");
        assert_eq!(endpoint.mode, DaemonMode::LocalExisting);
        supervisor.shutdown().unwrap();
    }

    #[test]
    fn host_launched_endpoint_can_be_adopted_without_process_ownership() {
        let mut supervisor = DesktopSupervisor::new(SupervisorConfig::default());

        let endpoint = supervisor
            .attach_existing("ws://127.0.0.1:43210".into())
            .unwrap();

        assert_eq!(endpoint.mode, DaemonMode::LocalExisting);
        assert_eq!(supervisor.endpoint(), Some(&endpoint));
    }

    #[test]
    fn explicit_remote_endpoint_cannot_enable_local_remote_mode() {
        let mut supervisor = DesktopSupervisor::new(SupervisorConfig {
            explicit_url: Some("ws://127.0.0.1:43210".into()),
            remote_enabled: true,
            ..SupervisorConfig::default()
        });
        assert!(supervisor.connect_or_launch().is_err());
    }

    #[test]
    fn zero_port_allocates_loopback_port() {
        let address = allocate_local_address(0).unwrap();
        assert_eq!(address.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_ne!(address.port(), 0);
    }

    #[test]
    fn invalid_ports_and_urls_are_rejected() {
        assert!(parse_port("0").is_err());
        assert!(parse_port("65536").is_err());
        assert!(validate_websocket_url("http://127.0.0.1:1").is_err());
        assert!(websocket_address("ws://").is_err());
    }
}
