//! Embedded React UI assets shared by the web bridge and desktop host.

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tracing::warn;

mod desktop;
pub use desktop::DesktopRelay;

/// The built React app, or an empty directory when its assets are unavailable.
static UI_DIST: Dir<'static> = include_dir!("$MEW_WEB_UI_DIST");

const MAX_HEADER_BYTES: usize = 8192;

/// Serve only embedded UI assets on the supplied listener.
///
/// Dropping this future closes the listener and aborts its in-flight requests.
/// Daemon connections and native operations belong to the caller's transport.
pub async fn serve_assets(listener: TcpListener) -> Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted.context("accept UI asset connection")?;
                connections.spawn(async move {
                    if let Err(error) = serve_asset_connection(stream).await {
                        warn!(%peer, %error, "UI asset request failed");
                    }
                });
            }
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result {
                    warn!(%error, "UI asset request task failed");
                }
            }
        }
    }
}

async fn serve_asset_connection(mut stream: TcpStream) -> Result<()> {
    // One extra byte distinguishes a complete maximum-sized header from an
    // oversized request without growing the per-connection buffer.
    let mut request = [0; MAX_HEADER_BYTES + 1];
    let mut length = 0;
    let header_end = loop {
        let read = stream.read(&mut request[length..]).await?;
        if read == 0 {
            return if length == 0 {
                Ok(())
            } else {
                serve_error(stream, "400 Bad Request", "Bad Request").await
            };
        }
        let search_start = length.saturating_sub(3);
        length += read;
        if let Some(end) = request[search_start..length]
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .map(|offset| search_start + offset + 4)
        {
            if end <= MAX_HEADER_BYTES {
                break end;
            }
        }
        if length > MAX_HEADER_BYTES {
            return serve_error(
                stream,
                "431 Request Header Fields Too Large",
                "Request Header Fields Too Large",
            )
            .await;
        }
    };
    let Ok(headers) = std::str::from_utf8(&request[..header_end]) else {
        return serve_error(stream, "400 Bad Request", "Bad Request").await;
    };
    let mut parts = headers
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let (Some(method), Some(path), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return serve_error(stream, "400 Bad Request", "Bad Request").await;
    };
    if !path.starts_with('/') || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return serve_error(stream, "400 Bad Request", "Bad Request").await;
    }
    serve_http(stream, method, path).await
}

async fn serve_error(mut stream: TcpStream, status: &str, body: &str) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

/// Serve a static HTTP response for a non-WS GET request, pulling files
/// from the embedded `mew-web-ui/dist/` directory. SPA fallback: any
/// unknown path serves `index.html` so client-side routing works.
pub async fn serve_http(mut stream: TcpStream, method: &str, path: &str) -> Result<()> {
    if method != "GET" {
        return serve_error(stream, "405 Method Not Allowed", "Method Not Allowed").await;
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
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
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
