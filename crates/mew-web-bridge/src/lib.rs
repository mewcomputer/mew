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

async fn serve_error<S: AsyncWriteExt + Unpin>(
    mut stream: S,
    status: &str,
    body: &str,
) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

/// Serve a static HTTP response for a non-WS GET/OPTIONS request, pulling files
/// from the embedded `mew-web-ui/dist/` directory or session artifacts.
pub async fn serve_http(stream: TcpStream, method: &str, path: &str) -> Result<()> {
    serve_http_with_session_dir(stream, method, path, &mew_session::session_dir()).await
}

/// Like [`serve_http`], with an explicit session root directory for `/artifacts/*`.
pub async fn serve_http_with_session_dir(
    mut stream: TcpStream,
    method: &str,
    path: &str,
    session_dir: &std::path::Path,
) -> Result<()> {
    let clean_path = path.split('?').next().unwrap_or("/");
    if let Some(artifact_rel) = clean_path.strip_prefix("/artifacts/") {
        if method == "OPTIONS" {
            let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nConnection: close\r\n\r\n";
            stream.write_all(resp.as_bytes()).await?;
            stream.shutdown().await?;
            return Ok(());
        }
        if method != "GET" {
            return serve_error(stream, "405 Method Not Allowed", "Method Not Allowed").await;
        }
        return serve_artifact(stream, clean_path, artifact_rel, session_dir).await;
    }

    if method != "GET" {
        return serve_error(stream, "405 Method Not Allowed", "Method Not Allowed").await;
    }

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

    let cors = if rel.starts_with("artifact-runtime/") {
        "Access-Control-Allow-Origin: *\r\n"
    } else {
        ""
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\n{}Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
        bytes.len(),
        mime,
        cors
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

fn decode_uri_segment(seg: &str) -> Option<String> {
    let bytes = seg.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = *bytes.get(i + 1)?;
            let lo = *bytes.get(i + 2)?;
            let hex = [hi, lo];
            let hex_str = std::str::from_utf8(&hex).ok()?;
            let val = u8::from_str_radix(hex_str, 16).ok()?;
            out.push(val);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn is_safe_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg != "."
        && seg != ".."
        && !seg.contains('/')
        && !seg.contains('\\')
        && !seg.contains('\0')
}

fn is_valid_blob_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

fn resolve_manifest_blob<'a>(
    manifest: &'a serde_json::Value,
    rel_key: &str,
) -> Option<(&'a str, String)> {
    let files = manifest.get("files")?.as_object()?;
    let key = if rel_key.is_empty() {
        "index.html".to_string()
    } else if files.contains_key(rel_key) {
        rel_key.to_string()
    } else {
        let dir_index = format!("{rel_key}/index.html");
        if files.contains_key(&dir_index) {
            dir_index
        } else {
            return None;
        }
    };
    let hash = files.get(&key)?.get("hash")?.as_str()?;
    Some((hash, key))
}

async fn serve_artifact<S: AsyncWriteExt + Unpin>(
    mut stream: S,
    clean_path: &str,
    artifact_rel: &str,
    session_dir: &std::path::Path,
) -> Result<()> {
    let mut parts = artifact_rel.splitn(3, '/');
    let (Some(raw_session_id), Some(raw_artifact_id)) = (parts.next(), parts.next()) else {
        return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
    };
    let (Some(session_id), Some(artifact_id)) = (
        decode_uri_segment(raw_session_id),
        decode_uri_segment(raw_artifact_id),
    ) else {
        return serve_error(stream, "400 Bad Request", "Invalid Artifact Path").await;
    };
    if !is_safe_segment(&session_id) || !is_safe_segment(&artifact_id) {
        return serve_error(stream, "400 Bad Request", "Invalid Artifact Path").await;
    }

    let rel_file = parts.next().unwrap_or("");
    let artifacts_root = session_dir.join(&session_id).join("artifacts");
    let artifact_root = artifacts_root.join(&artifact_id);
    let Ok(canonical_root) = tokio::fs::canonicalize(&artifact_root).await else {
        return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
    };

    let mut rel_segments = Vec::new();
    for raw_seg in rel_file.split('/') {
        if raw_seg.is_empty() || raw_seg == "." {
            continue;
        }
        let Some(seg) = decode_uri_segment(raw_seg) else {
            return serve_error(stream, "400 Bad Request", "Invalid Artifact Path").await;
        };
        if !is_safe_segment(&seg) {
            return serve_error(stream, "400 Bad Request", "Invalid Artifact Path").await;
        }
        rel_segments.push(seg);
    }

    let manifest_path = canonical_root.join("manifest.json");
    let (canonical_file, mime_key) = if let Ok(manifest_bytes) =
        tokio::fs::read(&manifest_path).await
    {
        let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&manifest_bytes) else {
            return serve_error(
                stream,
                "500 Internal Server Error",
                "Invalid Artifact Manifest",
            )
            .await;
        };
        let rel_key = rel_segments.join("/");
        let Some((hash, resolved_key)) = resolve_manifest_blob(&manifest, &rel_key) else {
            return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
        };
        if !rel_key.is_empty()
            && resolved_key == format!("{rel_key}/index.html")
            && !clean_path.ends_with('/')
        {
            let resp = format!(
                "HTTP/1.1 302 Found\r\nLocation: {clean_path}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(resp.as_bytes()).await?;
            stream.shutdown().await?;
            return Ok(());
        }
        if !is_valid_blob_hash(hash) {
            return serve_error(stream, "500 Internal Server Error", "Invalid Artifact Blob").await;
        }
        let blobs_root = artifacts_root.join("blobs");
        let Ok(canonical_blobs) = tokio::fs::canonicalize(&blobs_root).await else {
            return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
        };
        let Ok(canonical_blob) = tokio::fs::canonicalize(canonical_blobs.join(hash)).await else {
            return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
        };
        if !canonical_blob.starts_with(&canonical_blobs) {
            return serve_error(stream, "403 Forbidden", "Forbidden").await;
        }
        (canonical_blob, resolved_key)
    } else {
        let mut candidate = canonical_root.clone();
        for seg in &rel_segments {
            candidate.push(seg);
        }
        let is_dir = tokio::fs::metadata(&candidate)
            .await
            .is_ok_and(|meta| meta.is_dir());
        if is_dir {
            if !clean_path.ends_with('/') {
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {clean_path}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(resp.as_bytes()).await?;
                stream.shutdown().await?;
                return Ok(());
            }
            candidate.push("index.html");
        }
        let Ok(canonical_file) = tokio::fs::canonicalize(&candidate).await else {
            return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
        };
        if !canonical_file.starts_with(&canonical_root) {
            return serve_error(stream, "403 Forbidden", "Forbidden").await;
        }
        let key = canonical_file.to_string_lossy().into_owned();
        (canonical_file, key)
    };

    let Ok(mut file) = tokio::fs::File::open(&canonical_file).await else {
        return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
    };
    let Ok(metadata) = file.metadata().await else {
        return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
    };
    if !metadata.is_file() {
        return serve_error(stream, "404 Not Found", "Artifact Not Found").await;
    }

    let mime = mime_type(&mime_key);
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {}\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: sandbox allow-scripts allow-downloads allow-forms;\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        metadata.len(),
        mime
    );
    stream.write_all(resp.as_bytes()).await?;
    tokio::io::copy(&mut file, &mut stream).await?;
    stream.shutdown().await?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn run_serve_artifact(rel: &str, session_dir: &std::path::Path) -> String {
        let (server_stream, mut client_stream) = tokio::io::duplex(64 * 1024);
        let dir = session_dir.to_path_buf();
        let rel_owned = rel.to_string();
        let server = tokio::spawn(async move {
            let clean = format!("/artifacts/{rel_owned}");
            serve_artifact(server_stream, &clean, &rel_owned, &dir)
                .await
                .unwrap();
        });
        let mut buf = Vec::new();
        client_stream.read_to_end(&mut buf).await.unwrap();
        server.await.unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[tokio::test]
    async fn serves_artifact_with_sandbox_csp_and_blocks_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let art_dir = dir.path().join("sess_1").join("artifacts").join("art_1");
        std::fs::create_dir_all(&art_dir).unwrap();
        std::fs::write(art_dir.join("index.html"), "<h1>Sign Maker</h1>").unwrap();

        let ok = run_serve_artifact("sess_1/art_1/index.html", dir.path()).await;
        assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(ok.contains(
            "Content-Security-Policy: sandbox allow-scripts allow-downloads allow-forms;"
        ));
        assert!(ok.contains("Access-Control-Allow-Origin: *"));
        assert!(ok.contains("X-Content-Type-Options: nosniff"));
        assert!(ok.ends_with("<h1>Sign Maker</h1>"));

        // Trailing-slash redirect on directory without trailing '/'.
        let redir = run_serve_artifact("sess_1/art_1", dir.path()).await;
        assert!(redir.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(redir.contains("Location: /artifacts/sess_1/art_1/\r\n"));

        // Directory with trailing '/' serves index.html.
        let dir_ok = run_serve_artifact("sess_1/art_1/", dir.path()).await;
        assert!(dir_ok.starts_with("HTTP/1.1 200 OK\r\n"));

        // Percent-encoded traversal is blocked.
        let bad = run_serve_artifact("sess_1/art_1/%2e%2e/secret.txt", dir.path()).await;
        assert!(bad.starts_with("HTTP/1.1 400 Bad Request\r\n"));
    }

    #[tokio::test]
    async fn serves_manifest_blob_backed_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("sess_m").join("artifacts").join("blobs");
        let art_dir = dir.path().join("sess_m").join("artifacts").join("art_m");
        tokio::fs::create_dir_all(&blobs).await.unwrap();
        tokio::fs::create_dir_all(&art_dir).await.unwrap();
        let hash = "a".repeat(64);
        tokio::fs::write(blobs.join(&hash), "<h1>Blob</h1>")
            .await
            .unwrap();
        let manifest = serde_json::json!({
            "version": 1,
            "files": {
                "index.html": { "hash": hash, "size": 13 }
            }
        });
        tokio::fs::write(
            art_dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .await
        .unwrap();

        let ok = run_serve_artifact("sess_m/art_m/index.html", dir.path()).await;
        assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(ok.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(ok.ends_with("<h1>Blob</h1>"));
    }
}
