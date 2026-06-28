//! mew-web — a small TCP/WS bridge + static UI server.
//!
//! Bridges browser WebSocket connections to a local `mew daemon` (which
//! speaks the same wire protocol over a Unix domain socket), and serves the
//! chat UI's static assets over HTTP on the same port.
//!
//! Architecture:
//!
//!   Browser  ──ws://127.0.0.1:9847/──▶  mew-web  ──ws+unix://...──▶  mew daemon
//!                                       │
//!                                       └─ HTTP /, /main.js ──▶ embedded assets
//!
//! The bridge is a pure relay: it doesn't read or interpret the wire
//! protocol, it just forwards frames in both directions. Each browser
//! connection opens a fresh daemon connection.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use futures::{SinkExt, StreamExt};
use include_dir::{include_dir, Dir};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::{client_async, WebSocketStream};
use tracing::{error, info, warn};

/// The built React app, embedded at compile time from `mew-web-ui/dist/`.
/// Run `pnpm --filter mew-web-ui build` (or `just build-web`) to regenerate.
/// Vite hashes asset filenames, so we serve them dynamically by path.
static UI_DIST: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../mew-web-ui/dist");

#[derive(Parser, Debug)]
#[command(name = "mew-web", about = "WebSocket bridge + chat UI for mew daemon")]
struct Args {
    /// TCP address to listen on for browser connections.
    #[arg(long, default_value = "127.0.0.1:9847")]
    port: SocketAddr,

    /// Unix socket path the daemon is listening on.
    /// Same default as `mew daemon`.
    #[arg(long)]
    daemon_socket: Option<PathBuf>,

    /// If true (default), spawn `mew daemon` via this command if it's not
    /// already running on the expected socket. Pass `--spawn false` to
    /// disable — useful when you've started the daemon yourself.
    #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
    spawn: bool,
}

fn default_daemon_socket() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join("mew.sock"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/mew.sock"))
}

/// Per-connection handler. Routes between static UI and WS proxy.
///
/// We can't read the HTTP request first and then hand the stream to
/// tungstenite — tungstenite expects to read the request itself. So
/// instead we use a BufReader and `fill_buf()` to peek at the request
/// line + headers without consuming them, then decide:
///   - If WS upgrade: hand the BufReader to `accept_async` (which reads
///     the same bytes from the buffer).
///   - Otherwise: read more from the BufReader (request body) and serve
///     the static UI.
async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    daemon_socket: Arc<String>,
) -> Result<()> {
    let mut buf: BufReader<TcpStream> = BufReader::with_capacity(8192, stream);

    // Peek at the buffered bytes without consuming them. Cap at 8 KiB
    // so we don't read forever on a malicious client.
    let peeked = match peek_request(&mut buf).await? {
        Some(s) => s,
        None => return Ok(()),
    };

    let mut lines = peeked.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let mut wants_ws = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let lk = k.trim().to_ascii_lowercase();
            match lk.as_str() {
                "upgrade" => {
                    if v.trim().eq_ignore_ascii_case("websocket") {
                        wants_ws = true;
                    }
                }
                "connection" => {
                    if v.to_ascii_lowercase()
                        .split(',')
                        .any(|c| c.trim().eq_ignore_ascii_case("upgrade"))
                    {
                        // already set if Upgrade header matched
                    }
                }
                _ => {}
            }
        }
    }

    if method == "GET" && wants_ws {
        // Hand the BufReader (still containing the request bytes) to
        // tungstenite — it will read from the cursor position, which is
        // still at the start of the request.
        let browser_ws = tokio_tungstenite::accept_async(buf)
            .await
            .context("server-side WS handshake")?;
        info!(%peer, "browser ws upgraded; connecting to daemon");
        let daemon_ws = connect_to_daemon(&daemon_socket).await?;
        return proxy(browser_ws, daemon_ws).await;
    }

    // Plain HTTP — drain the BufReader's still-buffered peeked bytes via
    // `consume`, unwrap the stream, and serve a static response. For
    // GETs there's no body to read past the headers.
    let peeked_len = peeked.len().min(8192);
    buf.consume(peeked_len);
    let stream = buf.into_inner();
    serve_http(stream, method, path).await
}

/// Peek the request line + headers from the BufReader. Returns the
/// first up-to-8-KiB of the stream's buffered bytes without consuming
/// them. Returns `None` if EOF was hit before any bytes arrived.
async fn peek_request<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<String>> {
    let buf = reader.fill_buf().await?;
    if buf.is_empty() {
        return Ok(None);
    }
    // Take up to 8 KiB; this is more than enough for a single GET.
    let n = buf.len().min(8192);
    let s = std::str::from_utf8(&buf[..n])
        .context("request bytes are not utf-8")?
        .to_string();
    Ok(Some(s))
}

/// Serve a static HTTP response for a non-WS GET request, pulling files
/// from the embedded `mew-web-ui/dist/` directory. SPA fallback: any
/// unknown path serves `index.html` so client-side routing works.
async fn serve_http(mut stream: TcpStream, method: &str, path: &str) -> Result<()> {
    if method != "GET" {
        let body = "Method Not Allowed";
        let resp = format!(
            "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(resp.as_bytes()).await?;
        stream.shutdown().await?;
        return Ok(());
    }

    // Strip query string, normalize path.
    let clean_path = path.split('?').next().unwrap_or("/");
    // Trim leading '/' and the optional leading slash.
    let rel = clean_path.trim_start_matches('/');

    // Try to find the file in the embedded dist directory.
    let (bytes, mime) = if rel.is_empty() {
        // "/" → index.html
        (index_html_bytes(), "text/html; charset=utf-8")
    } else if let Some(file) = UI_DIST.get_file(rel) {
        (file.contents(), mime_type(rel))
    } else {
        // SPA fallback: serve index.html for unknown paths so client-side
        // routing (TanStack Router) handles them.
        (index_html_bytes(), "text/html; charset=utf-8")
    };

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nCache-Control: no-cache\r\n\r\n",
        bytes.len(),
        mime
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.shutdown().await?;
    Ok(())
}

/// Get the bytes of `index.html` from the embedded dist directory.
fn index_html_bytes() -> &'static [u8] {
    UI_DIST
        .get_file("index.html")
        .map(|f| f.contents())
        .unwrap_or(FALLBACK_INDEX_HTML)
}

/// Map a file extension to a MIME type. Covers all the asset types Vite
/// produces for a React app (JS, CSS, fonts, SVG, images, source maps).
fn mime_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "map" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Minimal fallback if index.html is somehow missing from the dist.
const FALLBACK_INDEX_HTML: &[u8] = b"<!DOCTYPE html><html><body><h1>mew-web</h1><p>UI not built. Run <code>just build-web</code>.</p></body></html>";

/// Connect to the daemon over a Unix socket and perform the client-side
/// WebSocket handshake.
async fn connect_to_daemon(socket_path: &str) -> Result<WebSocketStream<UnixStream>> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect to daemon unix socket {socket_path}"))?;
    let req = ClientRequestBuilder::new("ws://localhost/".parse().unwrap())
        .with_header("Host", "localhost")
        .with_header("Connection", "Upgrade")
        .with_header("Upgrade", "websocket")
        .with_header("Sec-WebSocket-Version", "13")
        .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    let (ws, _resp) = client_async(req, stream)
        .await
        .context("client-side WS handshake to daemon")?;
    Ok(ws)
}

/// Bidirectional frame relay between the browser WS and the daemon WS.
/// Returns when either side closes or errors. The browser stream type is
/// generic so the caller can hand us either a raw TcpStream or a
/// BufReader<TcpStream> (the latter when we peeked the HTTP request).
async fn proxy<S>(browser: WebSocketStream<S>, daemon: WebSocketStream<UnixStream>) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut b_tx, mut b_rx) = browser.split();
    let (mut d_tx, mut d_rx) = daemon.split();

    let b_to_d = async {
        while let Some(msg) = b_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    warn!(error = %e, "browser->daemon read error");
                    break;
                }
            };
            if d_tx.send(msg).await.is_err() {
                break;
            }
        }
    };
    let d_to_b = async {
        while let Some(msg) = d_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    warn!(error = %e, "daemon->browser read error");
                    break;
                }
            };
            if b_tx.send(msg).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        _ = b_to_d => {}
        _ = d_to_b => {}
    }
    Ok(())
}

async fn try_spawn_daemon(socket_path: &str) -> Result<()> {
    // Best-effort: spawn `mew daemon` if it's not already running. We try
    // three locations in order:
    //   1. The directory containing *this* binary (so `cargo run` uses the
    //      same build, not a stale `~/.cargo/bin/mew`).
    //   2. `mew` on PATH (for installed binaries).
    //   3. `target/debug/mew` relative to CWD (a common dev layout).
    //
    // If all fail, log and continue — the user may have started the daemon
    // themselves.
    info!(socket = socket_path, "spawning mew daemon");

    let candidates = daemon_binary_candidates();
    let mut last_err = None;
    for bin in &candidates {
        let mut cmd = tokio::process::Command::new(bin);
        cmd.arg("daemon").arg("--socket").arg(socket_path);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());
        match cmd.spawn() {
            Ok(mut child) => {
                // Give the daemon a moment to bind.
                let bound = wait_for_socket(socket_path, 40).await;
                if bound {
                    info!(socket = socket_path, "daemon is listening");
                    return Ok(());
                }
                // Daemon spawned but didn't bind — grab its stderr to find out why.
                let stderr = match child.try_wait() {
                    Ok(Some(status)) => {
                        let out = child
                            .wait_with_output()
                            .await
                            .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                            .unwrap_or_default();
                        format!("exited {status}: {out}")
                    }
                    _ => String::from("still running but socket not created"),
                };
                warn!(binary = %bin.display(), socket = socket_path, stderr = %stderr, "daemon spawned but did not bind");
                last_err = Some(stderr);
            }
            Err(e) => {
                warn!(binary = %bin.display(), error = %e, "failed to spawn mew daemon");
                last_err = Some(e.to_string());
            }
        }
    }
    warn!(
        socket = socket_path,
        error = last_err.unwrap_or_else(|| "no mew binary found".to_string()),
        "could not spawn mew daemon — start it manually: mew daemon"
    );
    Ok(())
}

/// Return candidate paths for the `mew` binary, in priority order:
/// 1. Sibling of the current executable (e.g. `target/debug/mew`).
/// 2. `mew` on PATH.
/// 3. `target/debug/mew` and `target/release/mew` relative to CWD.
fn daemon_binary_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();

    // 1. Next to the bridge binary itself.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("mew"));
        }
    }

    // 2. Bare `mew` — resolves via PATH at spawn time.
    candidates.push(std::path::PathBuf::from("mew"));

    // 3. Common dev paths relative to CWD.
    candidates.push(std::path::PathBuf::from("target/debug/mew"));
    candidates.push(std::path::PathBuf::from("target/release/mew"));

    candidates
}

/// Poll for the daemon socket to appear, up to `attempts` × 100ms.
async fn wait_for_socket(socket_path: &str, attempts: usize) -> bool {
    for _ in 0..attempts {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if tokio::net::UnixStream::connect(socket_path).await.is_ok() {
            return true;
        }
    }
    false
}

async fn daemon_is_running(socket_path: &str) -> bool {
    tokio::net::UnixStream::connect(socket_path).await.is_ok()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,mew_web=debug")),
        )
        .init();

    let args = Args::parse();
    let daemon_socket = args
        .daemon_socket
        .clone()
        .unwrap_or_else(default_daemon_socket);
    let daemon_socket_str = daemon_socket.to_string_lossy().to_string();

    if args.spawn && !daemon_is_running(&daemon_socket_str).await {
        try_spawn_daemon(&daemon_socket_str).await?;
    }

    let listener = TcpListener::bind(args.port)
        .await
        .with_context(|| format!("bind {}", args.port))?;
    info!(
        port = %args.port,
        daemon = %daemon_socket_str,
        "mew-web listening — open http://{}/ in a browser",
        args.port
    );

    let daemon_socket = Arc::new(daemon_socket_str);
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                error!(error = %e, "accept failed");
                continue;
            }
        };
        let daemon_socket = Arc::clone(&daemon_socket);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer, daemon_socket).await {
                warn!(%peer, error = %e, "connection ended with error");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Embedded UI assets
// ---------------------------------------------------------------------------
//
// The built React app is embedded via `include_dir!` (see UI_DIST above).
// The dist is produced by `pnpm --filter mew-web-ui build` (a.k.a. `just
// build-web`). Vite hashes asset filenames (e.g. `index-BNDoaku_.js`), so
// we serve them dynamically by path lookup rather than hardcoded constants.
