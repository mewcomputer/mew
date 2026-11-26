//! Subprocess end-to-end test for the mew web harness.
//!
//! Spawns the actual `mew` and `mew-web` binaries as subprocesses and
//! verifies the full round-trip over a real WebSocket connection:
//!
//!   client  --ws-->  mew-web (bridge)  --ws+unix-->  mew daemon (fake provider)
//!
//! If either binary isn't built, the test skips with a clear message
//! so CI can run `just ci` after `cargo build`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use mew_protocol::{ClientMessage, ServerMessage};
use tempfile::{NamedTempFile, TempDir};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{
    client::{ClientRequestBuilder, IntoClientRequest},
    http::{header, StatusCode},
    Error as WebSocketError, Message,
};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<TcpStream>;

/// Locate the CLI next to the bridge Cargo built for this test profile.
fn binary_paths() -> Option<(PathBuf, PathBuf)> {
    let mew_web = PathBuf::from(env!("CARGO_BIN_EXE_mew-web"));
    let mew = mew_web.with_file_name(format!("mew{}", std::env::consts::EXE_SUFFIX));
    mew.is_file().then_some((mew, mew_web))
}

/// Bind a TCP listener on an OS-assigned port, read the port, and close.
async fn free_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr.port())
}

/// Spawn a child process. Returns a guard that kills the child on drop.
struct ChildGuard {
    child: Child,
    stderr: NamedTempFile,
}

impl ChildGuard {
    fn spawn(command: &mut Command) -> Result<Self> {
        let stderr = NamedTempFile::new()?;
        let child = command
            .stdout(Stdio::null())
            .stderr(stderr.reopen()?)
            .spawn()?;
        Ok(Self { child, stderr })
    }

    fn ensure_running(&mut self, name: &str) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            let stderr = std::fs::read_to_string(self.stderr.path())?;
            anyhow::bail!("{name} exited before becoming ready ({status}): {stderr}");
        }
        Ok(())
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn `mew daemon` with `--fake-provider` listening on a Unix socket.
/// Returns the guard, the socket path, and the tempdir that holds it.
fn spawn_fake_daemon(
    mew_bin: &PathBuf,
    socket_path: &std::path::Path,
    home_dir: &std::path::Path,
) -> Result<ChildGuard> {
    ChildGuard::spawn(
        Command::new(mew_bin)
            .current_dir(home_dir)
            .arg("daemon")
            .arg("--fake-provider")
            .arg("--socket")
            .arg(socket_path)
            .arg("--pidfile")
            .arg(home_dir.join("mew.pid"))
            .env("MEW_CONFIG_DIR", home_dir.join("config"))
            .env("MEW_DATA_DIR", home_dir.join("data"))
            .env("MEW_SESSION_DIR", home_dir.join("sessions"))
            .env("MEW_CACHE_DIR", home_dir.join("catalog-cache"))
            .env_remove("MEW_CONFIG")
            .env_remove("MEW_DEFAULT_MODEL"),
    )
    .context("spawn mew daemon")
}

/// Spawn `mew-web` pointed at the fake daemon. `--spawn false` makes the
/// bridge skip its own daemon-spawn (we already have one running).
fn spawn_bridge(
    mew_web_bin: &PathBuf,
    daemon_socket: &std::path::Path,
    tcp_port: u16,
) -> Result<ChildGuard> {
    ChildGuard::spawn(
        Command::new(mew_web_bin)
            .arg("--port")
            .arg(format!("127.0.0.1:{tcp_port}"))
            .arg("--daemon-socket")
            .arg(daemon_socket)
            .arg("--spawn")
            .arg("false"),
    )
    .context("spawn mew-web")
}

/// Wait for the daemon socket to appear, up to `timeout` total.
async fn wait_for_unix_socket(
    path: &std::path::Path,
    child: &mut ChildGuard,
    timeout: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        child.ensure_running("daemon")?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let stderr = std::fs::read_to_string(child.stderr.path())?;
    anyhow::bail!("timed out waiting for daemon socket at {path:?}; stderr: {stderr}");
}

/// Wait for the TCP port to accept connections, up to `timeout` total.
async fn wait_for_tcp_port(
    addr: SocketAddr,
    child: &mut ChildGuard,
    timeout: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(s) = TcpStream::connect(addr).await {
            drop(s);
            return Ok(());
        }
        child.ensure_running("bridge")?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let stderr = std::fs::read_to_string(child.stderr.path())?;
    anyhow::bail!("timed out waiting for tcp port {addr}; stderr: {stderr}");
}

async fn send(ws: &mut Ws, msg: ClientMessage) -> Result<()> {
    let json = mew_protocol::encode_json(&msg)?;
    ws.send(Message::Text(json)).await?;
    Ok(())
}

/// Recv messages until `pred` returns true or timeout fires.
async fn recv_until<F>(ws: &mut Ws, mut pred: F, timeout: Duration) -> Result<Vec<ServerMessage>>
where
    F: FnMut(&ServerMessage) -> bool,
{
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let next = match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(None) => {
                anyhow::bail!("ws stream ended before predicate matched; collected {out:?}")
            }
            Err(_) => anyhow::bail!("recv timed out; collected {out:?}"),
        };
        let text = match next {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => anyhow::bail!("ws closed by peer; collected {out:?}"),
            _ => continue,
        };
        let parsed: ServerMessage = mew_protocol::decode_json(&text)?;
        let done = pred(&parsed);
        out.push(parsed);
        if done {
            return Ok(out);
        }
    }
    anyhow::bail!("recv_until timed out; collected {out:?}");
}

#[tokio::test]
async fn daemon_startup_failure_reports_child_stderr() -> Result<()> {
    let tmp = TempDir::new()?;
    let mut child = ChildGuard::spawn(
        Command::new("/bin/sh")
            .arg("-c")
            .arg("printf 'startup failure fixture\\n' >&2; exit 7"),
    )?;
    let error = wait_for_unix_socket(
        &tmp.path().join("missing.sock"),
        &mut child,
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("startup failure fixture"),
        "{error:#}"
    );
    assert!(error.to_string().contains("exit status: 7"), "{error:#}");
    Ok(())
}

#[tokio::test]
async fn bridge_startup_timeout_reports_stderr_while_child_is_alive() -> Result<()> {
    let mut child = ChildGuard::spawn(
        Command::new("/bin/sh")
            .arg("-c")
            .arg("printf 'startup stalled fixture\\n' >&2; read held")
            .stdin(Stdio::piped()),
    )?;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if std::fs::read_to_string(child.stderr.path())?.contains("startup stalled fixture") {
                return Ok::<_, anyhow::Error>(());
            }
            child.ensure_running("fixture")?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let addr = SocketAddr::from(([127, 0, 0, 1], free_port().await?));
    let error = wait_for_tcp_port(addr, &mut child, Duration::from_millis(50))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("timed out waiting for tcp port"),
        "{error:#}"
    );
    assert!(
        error.to_string().contains("startup stalled fixture"),
        "{error:#}"
    );
    assert!(child.child.try_wait()?.is_none());
    Ok(())
}

#[tokio::test]
async fn bridge_websocket_accepts_only_the_bound_host_and_trusted_origins() -> Result<()> {
    let port = free_port().await?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let approved_origin = "http://127.0.0.1:5173";
    let tmp = TempDir::new()?;
    let mut bridge = ChildGuard::spawn(
        Command::new(env!("CARGO_BIN_EXE_mew-web"))
            .args(["--port", &addr.to_string(), "--spawn", "false"])
            .arg("--daemon-socket")
            .arg(tmp.path().join("unused.sock"))
            .args(["--allow-origin", approved_origin]),
    )?;
    wait_for_tcp_port(addr, &mut bridge, Duration::from_secs(5)).await?;

    let bound_host = addr.to_string();
    let bound_origin = format!("http://{addr}");
    let localhost = format!("localhost:{port}");
    let localhost_origin = format!("http://{localhost}");
    for (host, origin, should_upgrade) in [
        (bound_host.as_str(), None, true),
        (bound_host.as_str(), Some(bound_origin.as_str()), true),
        (localhost.as_str(), Some(localhost_origin.as_str()), true),
        (bound_host.as_str(), Some(approved_origin), true),
        (bound_host.as_str(), Some("http://localhost:5173"), false),
        (
            bound_host.as_str(),
            Some("https://untrusted.example"),
            false,
        ),
        (bound_host.as_str(), Some("null"), false),
        ("untrusted.example", None, false),
        ("untrusted.example", Some(bound_origin.as_str()), false),
    ] {
        let mut request = format!("ws://{addr}/ws").into_client_request()?;
        request.headers_mut().insert(header::HOST, host.parse()?);
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert(header::ORIGIN, origin.parse()?);
        }
        let stream = TcpStream::connect(addr).await?;
        let result =
            tokio::time::timeout(Duration::from_secs(2), client_async(request, stream)).await?;
        if should_upgrade {
            result.with_context(|| format!("rejected host {host}, origin {origin:?}"))?;
        } else {
            assert!(
                matches!(result, Err(WebSocketError::Http(response)) if response.status() == StatusCode::FORBIDDEN),
                "host {host}, origin {origin:?} should have received HTTP 403"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn bridge_refuses_to_bind_off_loopback() -> Result<()> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_mew-web"));
    command
        .kill_on_drop(true)
        .args(["--port", "0.0.0.0:0", "--spawn", "false"]);
    let result = tokio::time::timeout(Duration::from_secs(2), command.output()).await??;
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("loopback"),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn bin_e2e_daemon_plus_bridge_full_round_trip() -> Result<()> {
    let Some((mew_bin, mew_web_bin)) = binary_paths() else {
        anyhow::ensure!(
            std::env::var_os("MEW_REQUIRE_SUBPROCESS_E2E").is_none(),
            "mew binary missing next to mew-web; run just build-web with the same target directory"
        );
        eprintln!(
            "skipping: mew binary missing; run just build-web to exercise the subprocess gate"
        );
        return Ok(());
    };

    // Pick a free TCP port for the bridge, set up a tempdir for the
    // daemon's Unix socket and an isolated home directory so the
    // daemon doesn't inherit the user's real state.toml.
    let bridge_port = free_port().await?;
    let bridge_addr: SocketAddr = format!("127.0.0.1:{bridge_port}").parse()?;
    let tmp = TempDir::new()?;
    let socket_path = tmp.path().join("mew.sock");
    let home_dir = tmp.path().join("home");
    std::fs::create_dir_all(&home_dir)?;

    // Spawn the daemon first. It must bind the socket before the
    // bridge can connect.
    let mut daemon = spawn_fake_daemon(&mew_bin, &socket_path, &home_dir)?;
    wait_for_unix_socket(&socket_path, &mut daemon, Duration::from_secs(15)).await?;
    assert!(
        !home_dir.join("catalog-cache").exists(),
        "fake daemon must start without accessing real model catalogs"
    );

    // Spawn the bridge. `--spawn false` skips the bridge's own
    // daemon-spawn (we already have one running).
    let mut bridge = spawn_bridge(&mew_web_bin, &socket_path, bridge_port)?;
    wait_for_tcp_port(bridge_addr, &mut bridge, Duration::from_secs(5)).await?;

    // Connect a raw WS client to the bridge.
    let stream = TcpStream::connect(bridge_addr).await?;
    let req = ClientRequestBuilder::new(format!("ws://{bridge_addr}/").parse().unwrap())
        .with_header("Host", format!("{bridge_addr}"))
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let (mut ws, _) = client_async(req, stream).await.context("ws handshake")?;

    // Send NewSession, expect SessionReady.
    send(
        &mut ws,
        ClientMessage::NewSession {
            cwd: None,
            scratch: false,
            client_kind: mew_protocol::ClientKind::Unknown,
        },
    )
    .await?;
    let session_msgs = recv_until(
        &mut ws,
        |m| matches!(m, ServerMessage::SessionReady { .. }),
        Duration::from_secs(5),
    )
    .await?;
    let session_id = match session_msgs.last().unwrap() {
        ServerMessage::SessionReady { session_id, .. } => session_id.clone(),
        _ => unreachable!(),
    };
    assert!(
        session_id.starts_with("sess_"),
        "session_id should start with sess_: {session_id}"
    );

    // Send Prompt, expect streamed text + MessageEnd(Stop). The fake
    // provider responds with "hello from fake provider".
    send(
        &mut ws,
        ClientMessage::Prompt {
            text: "anything".into(),
            attachments: vec![],
        },
    )
    .await?;
    let prompt_msgs = recv_until(
        &mut ws,
        |m| {
            matches!(
                m,
                ServerMessage::Provider {
                    event: mew_message::ProviderEventWire::MessageEnd { .. }
                }
            )
        },
        Duration::from_secs(5),
    )
    .await?;

    // The deltas should reassemble to "hello from fake provider".
    let reassembled: String = prompt_msgs
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Provider {
                event: mew_message::ProviderEventWire::PartDelta { delta, .. },
            } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        reassembled, "hello from fake provider",
        "streamed text should match the fake provider's scripted response"
    );

    // End the session cleanly. A close from our side ends the
    // bridge's per-connection task; the daemon keeps running.
    ws.send(Message::Close(None)).await.ok();

    Ok(())
}
