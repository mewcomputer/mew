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
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{client::ClientRequestBuilder, Message};
use tokio_tungstenite::{client_async, WebSocketStream};

type Ws = WebSocketStream<TcpStream>;

/// Locate the binaries. `cargo build` puts them in `target/debug/`
/// relative to CARGO_MANIFEST_DIR's parent (the workspace root).
fn binary_paths() -> Option<(PathBuf, PathBuf)> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent()?.parent()?;
    let target = if cfg!(debug_assertions) {
        workspace_root.join("target").join("debug")
    } else {
        workspace_root.join("target").join("release")
    };
    let mew = target.join("mew");
    let mew_web = target.join(if cfg!(windows) {
        "mew-web.exe"
    } else {
        "mew-web"
    });
    if mew.exists() && mew_web.exists() {
        Some((mew, mew_web))
    } else {
        None
    }
}

/// Bind a TCP listener on an OS-assigned port, read the port, and close.
async fn free_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    drop(listener);
    Ok(addr.port())
}

/// Spawn a child process. Returns a guard that kills the child on drop.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn `mew daemon` with `--fake-provider` listening on a Unix socket.
/// Returns the guard, the socket path, and the tempdir that holds it.
fn spawn_fake_daemon(mew_bin: &PathBuf, socket_path: &std::path::Path) -> Result<ChildGuard> {
    let child = Command::new(mew_bin)
        .arg("daemon")
        .arg("--fake-provider")
        .arg("--socket")
        .arg(socket_path)
        // Make sure the daemon doesn't try to read the user's real
        // config — empty env is enough; with no MEW_* env and a missing
        // config file, load() returns defaults.
        .env_remove("MEW_CONFIG")
        .env_remove("MEW_DEFAULT_MODEL")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn mew daemon")?;
    Ok(ChildGuard(child))
}

/// Spawn `mew-web` pointed at the fake daemon. `--spawn false` makes the
/// bridge skip its own daemon-spawn (we already have one running).
fn spawn_bridge(
    mew_web_bin: &PathBuf,
    daemon_socket: &std::path::Path,
    tcp_port: u16,
) -> Result<ChildGuard> {
    let child = Command::new(mew_web_bin)
        .arg("--port")
        .arg(format!("127.0.0.1:{tcp_port}"))
        .arg("--daemon-socket")
        .arg(daemon_socket)
        .arg("--spawn")
        .arg("false")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn mew-web")?;
    Ok(ChildGuard(child))
}

/// Wait for the daemon socket to appear, up to `timeout` total.
async fn wait_for_unix_socket(path: &std::path::Path, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("timed out waiting for daemon socket at {:?}", path);
}

/// Wait for the TCP port to accept connections, up to `timeout` total.
async fn wait_for_tcp_port(addr: SocketAddr, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(s) = TcpStream::connect(addr).await {
            drop(s);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("timed out waiting for tcp port {addr}");
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
async fn bin_e2e_daemon_plus_bridge_full_round_trip() -> Result<()> {
    let Some((mew_bin, mew_web_bin)) = binary_paths() else {
        eprintln!("skipping: binaries not built. Run `cargo build` (or `just build`) and re-run.");
        return Ok(());
    };

    // Pick a free TCP port for the bridge, set up a tempdir for the
    // daemon's Unix socket.
    let bridge_port = free_port().await?;
    let bridge_addr: SocketAddr = format!("127.0.0.1:{bridge_port}").parse()?;
    let tmp = TempDir::new()?;
    let socket_path = tmp.path().join("mew.sock");

    // Spawn the daemon first. It must bind the socket before the
    // bridge can connect.
    let _daemon = spawn_fake_daemon(&mew_bin, &socket_path)?;
    wait_for_unix_socket(&socket_path, Duration::from_secs(5)).await?;

    // Spawn the bridge. `--spawn false` skips the bridge's own
    // daemon-spawn (we already have one running).
    let _bridge = spawn_bridge(&mew_web_bin, &socket_path, bridge_port)?;
    wait_for_tcp_port(bridge_addr, Duration::from_secs(5)).await?;

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
