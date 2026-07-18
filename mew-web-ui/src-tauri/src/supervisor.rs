use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri_plugin_shell::{process::CommandChild, ShellExt};
use tungstenite::{client, Message};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const DESKTOP_DAEMON_URL: &str = "MEW_DESKTOP_DAEMON_URL";
const DESKTOP_DAEMON_BINARY: &str = "MEW_DESKTOP_DAEMON_BINARY";
const DESKTOP_DAEMON_PORT: &str = "MEW_DESKTOP_DAEMON_PORT";
const DEFAULT_DAEMON_PORT: u16 = 25566;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Owns the daemon launched for this desktop application instance.
///
/// The browser frontend still uses `mew-web-bridge`. The desktop shell talks
/// to a loopback TCP daemon directly, which keeps the host free of protocol
/// and agent logic while avoiding a second bridge process.
pub struct DaemonSupervisor {
    app: AppHandle,
    daemon: Mutex<Option<ReadyDaemon>>,
}

impl DaemonSupervisor {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            app: app.clone(),
            daemon: Mutex::new(None),
        }
    }

    fn start(&self) -> Result<ReadyDaemon> {
        if let Some(url) = configured_daemon_url()? {
            return Ok(ReadyDaemon {
                websocket_url: url,
                child: None,
            });
        }

        let address = daemon_address()?;
        let url = websocket_url(address);

        if probe_daemon(&url).is_ok() {
            tracing::info!(%url, "attached to an existing mew daemon");
            return Ok(ReadyDaemon {
                websocket_url: url,
                child: None,
            });
        }

        if TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok() {
            bail!(
                "daemon rendezvous port {address} is already in use, but it is not a mew daemon; \
                 set MEW_DESKTOP_DAEMON_URL to attach explicitly or stop the process"
            );
        }

        if let Some((child, binary)) = spawn_configured_daemon(address)? {
            tracing::info!(binary = %binary.display(), %address, "configured mew daemon is ready");
            return Ok(ReadyDaemon {
                websocket_url: url,
                child: Some(OwnedDaemon::Process(child)),
            });
        }

        if let Some((child, binary)) = spawn_bundled_daemon(address)? {
            tracing::info!(binary = %binary.display(), %address, "bundled mew daemon is ready");
            return Ok(ReadyDaemon {
                websocket_url: url,
                child: Some(OwnedDaemon::Process(child)),
            });
        }

        if let Some(child) = spawn_sidecar(&self.app, address)? {
            tracing::info!(%address, "bundled mew daemon sidecar is ready");
            return Ok(ReadyDaemon {
                websocket_url: url,
                child: Some(OwnedDaemon::Sidecar(child)),
            });
        }

        let (child, binary) = spawn_daemon(address)?;
        tracing::info!(binary = %binary.display(), %address, "desktop daemon is ready");

        Ok(ReadyDaemon {
            websocket_url: url,
            child: Some(OwnedDaemon::Process(child)),
        })
    }

    pub fn websocket_url(&self) -> Result<String> {
        let mut daemon_slot = self.daemon.lock().expect("daemon mutex poisoned");
        if let Some(daemon) = daemon_slot.as_ref() {
            return Ok(daemon.websocket_url.clone());
        }

        let daemon = self.start()?;
        let url = daemon.websocket_url.clone();
        *daemon_slot = Some(daemon);
        Ok(url)
    }
}

impl Drop for DaemonSupervisor {
    fn drop(&mut self) {
        let Ok(mut daemon) = self.daemon.lock() else {
            return;
        };
        let Some(daemon) = daemon.take() else {
            return;
        };

        if let Some(child) = daemon.child {
            child.kill();
        }
    }
}

struct ReadyDaemon {
    websocket_url: String,
    child: Option<OwnedDaemon>,
}

enum OwnedDaemon {
    Sidecar(CommandChild),
    Process(Child),
}

impl OwnedDaemon {
    fn kill(self) {
        match self {
            Self::Sidecar(child) => {
                let _ = child.kill();
            }
            Self::Process(mut child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn configured_daemon_url() -> Result<Option<String>> {
    let Ok(url) = env::var(DESKTOP_DAEMON_URL) else {
        return Ok(None);
    };

    if is_websocket_url(&url) {
        return Ok(Some(url));
    }

    Err(anyhow!(
        "{DESKTOP_DAEMON_URL} must start with ws:// or wss://"
    ))
}

fn daemon_address() -> Result<SocketAddr> {
    let port = env::var(DESKTOP_DAEMON_PORT)
        .ok()
        .map(|value| parse_daemon_port(&value))
        .transpose()?
        .unwrap_or(DEFAULT_DAEMON_PORT);
    Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
}

fn parse_daemon_port(value: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .with_context(|| format!("{DESKTOP_DAEMON_PORT} must be a valid TCP port"))?;
    if port == 0 {
        bail!("{DESKTOP_DAEMON_PORT} cannot be 0");
    }
    Ok(port)
}

fn spawn_sidecar(app: &AppHandle, address: SocketAddr) -> Result<Option<CommandChild>> {
    let command = match app.shell().sidecar("mew") {
        Ok(command) => command,
        Err(error) => {
            tracing::debug!(%error, "bundled mew daemon sidecar is unavailable");
            return Ok(None);
        }
    };
    let port_arg = format!("127.0.0.1:{}", address.port());
    let (events, child) = command
        .args(["daemon", "--port", &port_arg])
        .spawn()
        .context("spawn the bundled mew daemon sidecar")?;
    drop(events);

    if wait_for_daemon(address, STARTUP_TIMEOUT) {
        return Ok(Some(child));
    }

    let _ = child.kill();
    bail!("bundled mew daemon sidecar did not bind {address}")
}

fn spawn_configured_daemon(address: SocketAddr) -> Result<Option<(Child, PathBuf)>> {
    let Ok(binary) = env::var(DESKTOP_DAEMON_BINARY) else {
        return Ok(None);
    };
    let binary = PathBuf::from(binary);
    let child = spawn_daemon_binary(address, &binary)?;
    Ok(Some((child, binary)))
}

fn spawn_bundled_daemon(address: SocketAddr) -> Result<Option<(Child, PathBuf)>> {
    let Some(binary) = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|directory| directory.join("mew")))
        .filter(|binary| binary.is_file())
    else {
        return Ok(None);
    };

    let child = spawn_daemon_binary(address, &binary)?;
    Ok(Some((child, binary)))
}

fn spawn_daemon(address: SocketAddr) -> Result<(Child, PathBuf)> {
    let mut last_error = None;
    let port = address.port().to_string();
    let port_arg = format!("127.0.0.1:{port}");

    for binary in daemon_binary_candidates() {
        let mut command = Command::new(&binary);
        command
            .args(["daemon", "--port", &port_arg])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        close_inherited_descriptors(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                last_error = Some(format!("{}: {error}", binary.display()));
                continue;
            }
        };

        if wait_for_daemon(address, STARTUP_TIMEOUT) {
            return Ok((child, binary));
        }

        let _ = child.kill();
        let _ = child.wait();
        last_error = Some(format!("{} did not bind {address}", binary.display()));
    }

    bail!(
        "could not start the mew daemon; {DESKTOP_DAEMON_BINARY} may point to a binary, or install `mew` on PATH ({})",
        last_error.unwrap_or_else(|| "no candidate binaries found".to_owned())
    )
}

fn daemon_binary_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(binary) = env::var(DESKTOP_DAEMON_BINARY) {
        candidates.push(PathBuf::from(binary));
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("../../../target/debug/mew"));
    candidates.push(manifest_dir.join("../../../target/release/mew"));
    candidates.push(PathBuf::from("mew"));

    candidates.dedup();
    candidates
}

fn spawn_daemon_binary(address: SocketAddr, binary: &Path) -> Result<Child> {
    let port_arg = format!("127.0.0.1:{}", address.port());
    let mut command = Command::new(binary);
    command
        .args(["daemon", "--port", &port_arg])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    close_inherited_descriptors(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn configured daemon {}", binary.display()))?;

    if wait_for_daemon(address, STARTUP_TIMEOUT) {
        return Ok(child);
    }

    let _ = child.kill();
    let _ = child.wait();
    bail!(
        "configured daemon {} did not bind {address}",
        binary.display()
    )
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
    if !url.starts_with("ws://") {
        bail!("only loopback ws:// daemon URLs can be probed");
    }

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
    let response: serde_json::Value =
        serde_json::from_str(text.as_ref()).context("decode daemon health check")?;
    if response.get("type").and_then(serde_json::Value::as_str) != Some("pong") {
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
        .to_socket_addrs()
        .context("resolve daemon host")?
        .next()
        .ok_or_else(|| anyhow!("daemon URL did not resolve to an address"))
}

fn websocket_url(address: SocketAddr) -> String {
    format!("ws://{address}")
}

fn is_websocket_url(url: &str) -> bool {
    url.starts_with("ws://") || url.starts_with("wss://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_uses_loopback_address() {
        let address = "127.0.0.1:43127".parse().unwrap();

        assert_eq!(websocket_url(address), "ws://127.0.0.1:43127");
    }

    #[test]
    fn external_url_accepts_websocket_schemes() {
        assert!(is_websocket_url("ws://127.0.0.1:9847"));
        assert!(is_websocket_url("wss://example.test/socket"));
    }

    #[test]
    fn external_url_rejects_non_websocket_schemes() {
        assert!(!is_websocket_url("http://127.0.0.1:9847"));
        assert!(!is_websocket_url("127.0.0.1:9847"));
    }

    #[test]
    fn parse_daemon_port_uses_explicit_port() {
        assert_eq!(parse_daemon_port("25567").unwrap(), 25567);
    }

    #[test]
    fn parse_daemon_port_rejects_zero_and_invalid_values() {
        assert!(parse_daemon_port("0").is_err());
        assert!(parse_daemon_port("nope").is_err());
    }

    #[test]
    fn websocket_address_ignores_path() {
        assert_eq!(
            websocket_address("ws://127.0.0.1:43127/ws").unwrap(),
            "127.0.0.1:43127".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn wait_for_daemon_does_not_treat_a_bound_listener_as_ready() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        assert!(!wait_for_daemon(
            address,
            std::time::Duration::from_millis(100)
        ));
    }
}
