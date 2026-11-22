use mew_agy::proxy::EgressProxy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(target_os = "macos")]
#[tokio::test]
async fn proxy_reserves_the_same_ipv6_loopback_port() {
    let proxy = EgressProxy::start().await.unwrap();
    assert!(
        tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, proxy.port()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn proxy_rejects_non_google_and_local_destinations() {
    let proxy = EgressProxy::start().await.unwrap();
    for target in [
        "example.com:443",
        "127.0.0.1:443",
        "google.com:80",
        "oauth2.googleapis.com:80",
        "oauth2.googleapis.com.evil.example:443",
        "oauth2.googleapis.com@evil.example:443",
        "oauth2.googleapis.com:443:443",
    ] {
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        socket
            .write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 403"), "{target}");
    }
}

#[tokio::test]
async fn proxy_rejects_http_forward_requests() {
    let proxy = EgressProxy::start().await.unwrap();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .unwrap();
    client
        .write_all(
            b"GET http://oauth2.googleapis.com/ HTTP/1.1\r\nHost: oauth2.googleapis.com\r\n\r\n",
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 403"));
}

#[tokio::test]
async fn shutdown_closes_idle_clients_and_releases_listener() {
    let mut proxy = EgressProxy::start().await.unwrap();
    let port = proxy.port();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    client.write_all(b"CONNECT").await.unwrap();
    proxy.shutdown().await.unwrap();
    let mut byte = [0];
    match client.read(&mut byte).await {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        result => panic!("connection remained open: {result:?}"),
    }
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires network access to Google"]
async fn allowed_google_destination_accepts_connect() {
    let proxy = EgressProxy::start().await.unwrap();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .unwrap();
    client.write_all(b"CONNECT oauth2.googleapis.com:443 HTTP/1.1\r\nHost: oauth2.googleapis.com:443\r\n\r\n").await.unwrap();
    let mut response = [0; 256];
    let count = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        client.read(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(response[..count].starts_with(b"HTTP/1.1 200"));
}

#[tokio::test]
#[ignore = "requires network access to Google"]
async fn play_api_required_by_agy_authentication_accepts_connect() {
    let proxy = EgressProxy::start().await.unwrap();
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .unwrap();
    client
        .write_all(b"CONNECT play.googleapis.com:443 HTTP/1.1\r\nHost: play.googleapis.com\r\n\r\n")
        .await
        .unwrap();
    let mut response = [0; 256];
    let count = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        client.read(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(response[..count].starts_with(b"HTTP/1.1 200"));
}
