use std::net::SocketAddr;
use std::time::Duration;

use mew_web_bridge::serve_assets;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;

struct AssetServer {
    address: SocketAddr,
    task: JoinHandle<anyhow::Result<()>>,
}

impl AssetServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self {
            address: listener.local_addr().unwrap(),
            task: tokio::spawn(serve_assets(listener)),
        }
    }

    async fn request(&self, method: &str, path: &str) -> Response {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        stream
            .write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        response(&mut stream).await
    }
}

impl Drop for AssetServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Response {
    headers: String,
    body: Vec<u8>,
}

async fn response(stream: &mut TcpStream) -> Response {
    let mut bytes = Vec::new();
    timeout(Duration::from_secs(3), stream.read_to_end(&mut bytes))
        .await
        .expect("HTTP response must finish")
        .unwrap();
    let end = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let body = bytes[end + 4..].to_vec();
    let length: usize = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(length, body.len());
    Response { headers, body }
}

#[tokio::test]
async fn index_client_routes_and_query_strings_serve_the_same_app() {
    let server = AssetServer::start().await;
    let index = server.request("GET", "/").await;
    assert!(index.headers.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(index
        .headers
        .contains("Content-Type: text/html; charset=utf-8"));
    assert!(index.headers.contains("Cache-Control: no-cache"));
    assert!(String::from_utf8_lossy(&index.body).contains("<div id=\"root\""));
    for path in [
        "/index.html?theme=dark",
        "/session/recorded-chat?source=inbox",
        "/settings",
    ] {
        let route = server.request("GET", path).await;
        assert_eq!(
            route.body, index.body,
            "route {path} must use the app entry point"
        );
        assert!(route
            .headers
            .contains("Content-Type: text/html; charset=utf-8"));
    }
}

#[tokio::test]
async fn built_javascript_css_svg_and_json_keep_their_mime_and_bytes() {
    let server = AssetServer::start().await;
    let index = server.request("GET", "/").await;
    let html = String::from_utf8(index.body).unwrap();
    for (extension, mime) in [
        (".js", "application/javascript; charset=utf-8"),
        (".css", "text/css; charset=utf-8"),
    ] {
        let path = html
            .split('"')
            .find(|part| part.starts_with("/assets/") && part.ends_with(extension))
            .expect("built index must reference its generated assets");
        let asset = server
            .request("GET", &format!("{path}?version=preview"))
            .await;
        assert!(asset.headers.contains(&format!("Content-Type: {mime}")));
        assert!(!asset.body.is_empty());
        assert!(!asset.body.starts_with(b"<!doctype html>"));
        assert_eq!(server.request("GET", path).await.body, asset.body);
    }
    let icon = server.request("GET", "/favicon.svg").await;
    assert!(icon.headers.contains("Content-Type: image/svg+xml"));
    assert!(String::from_utf8_lossy(&icon.body).contains("<svg"));
    let manifest = server.request("GET", "/manifest.json").await;
    assert!(manifest
        .headers
        .contains("Content-Type: application/json; charset=utf-8"));
    assert!(serde_json::from_slice::<serde_json::Value>(&manifest.body).is_ok());
}

#[tokio::test]
async fn non_get_requests_are_rejected_without_serving_the_app() {
    let server = AssetServer::start().await;
    for method in ["POST", "HEAD", "DELETE"] {
        let reply = server.request(method, "/").await;
        assert!(reply
            .headers
            .starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
        assert_eq!(reply.body, b"Method Not Allowed");
    }
    assert!(server
        .request("GET", "/")
        .await
        .headers
        .starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn fragmented_request_line_and_headers_wait_until_complete() {
    let server = AssetServer::start().await;
    let mut stream = TcpStream::connect(server.address).await.unwrap();
    stream.write_all(b"GET /fav").await.unwrap();
    let mut premature = [0];
    assert!(
        timeout(Duration::from_millis(30), stream.read(&mut premature))
            .await
            .is_err()
    );
    stream
        .write_all(b"icon.svg?theme=dark HTTP/1.1\r\nHost: local")
        .await
        .unwrap();
    assert!(
        timeout(Duration::from_millis(30), stream.read(&mut premature))
            .await
            .is_err()
    );
    stream.write_all(b"host\r\n\r").await.unwrap();
    assert!(
        timeout(Duration::from_millis(30), stream.read(&mut premature))
            .await
            .is_err()
    );
    stream.write_all(b"\n").await.unwrap();
    let reply = response(&mut stream).await;
    assert!(reply.headers.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(reply.headers.contains("Content-Type: image/svg+xml"));
}

#[tokio::test]
async fn request_header_limit_accepts_the_boundary_and_rejects_one_byte_more() {
    let server = AssetServer::start().await;
    let prefix = "GET / HTTP/1.1\r\nHost: localhost\r\nX-Padding: ";
    for (length, status) in [
        (8192, "200 OK"),
        (8193, "431 Request Header Fields Too Large"),
    ] {
        let mut stream = TcpStream::connect(server.address).await.unwrap();
        let padding = "x".repeat(length - prefix.len() - 4);
        stream
            .write_all(format!("{prefix}{padding}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let reply = response(&mut stream).await;
        assert!(reply.headers.starts_with(&format!("HTTP/1.1 {status}\r\n")));
    }
}

#[tokio::test]
async fn malformed_or_truncated_requests_are_rejected_and_do_not_stop_other_clients() {
    let server = AssetServer::start().await;
    for request in [
        b"nonsense\r\n\r\n".as_slice(),
        b"GET / HTTP/1.1 extra\r\n\r\n",
        b"GET / HTTP/2.0\r\n\r\n",
        b"GET relative-path HTTP/1.1\r\n\r\n",
        b"GET / HTTP/1.1\r\nInvalid: \xff\r\n\r\n",
        b"GET / HTTP/1.1\r\nHost: localhost",
    ] {
        let mut stream = TcpStream::connect(server.address).await.unwrap();
        stream.write_all(request).await.unwrap();
        stream.shutdown().await.unwrap();
        assert!(response(&mut stream)
            .await
            .headers
            .starts_with("HTTP/1.1 400 Bad Request\r\n"));
    }
    assert!(server
        .request("GET", "/")
        .await
        .headers
        .starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn cancelling_asset_server_closes_stalled_connections() {
    let server = AssetServer::start().await;
    let mut stalled = TcpStream::connect(server.address).await.unwrap();
    stalled.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
    // A completed request ensures the accept loop has processed the earlier connection.
    assert!(server
        .request("GET", "/")
        .await
        .headers
        .starts_with("HTTP/1.1 200 OK"));
    server.task.abort();
    let mut byte = [0];
    let closed = timeout(Duration::from_secs(3), stalled.read(&mut byte))
        .await
        .expect("cancellation must close stalled request tasks");
    match closed {
        Ok(length) => assert_eq!(length, 0),
        Err(error) => assert!(matches!(error.kind(), std::io::ErrorKind::ConnectionReset)),
    }
}
