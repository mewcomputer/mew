use std::net::{IpAddr, Ipv4Addr, TcpListener as StdListener};
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_tungstenite::{accept_hdr_async, connect_async_with_config};
use tungstenite::{
    handshake::server::{ErrorResponse, Request, Response},
    http::{header::ORIGIN, StatusCode, Uri},
    protocol::WebSocketConfig,
};

/// A loopback relay for a desktop host's embedded, privileged UI.
pub struct DesktopRelay {
    listener: StdListener,
    origin: String,
    route: String,
    daemon_url: String,
    url: String,
}

impl DesktopRelay {
    pub fn bind(daemon_url: String, origin: &str) -> Result<Self> {
        let endpoint: Uri = daemon_url
            .parse()
            .context("invalid desktop daemon endpoint")?;
        ensure!(
            endpoint.scheme_str() == Some("ws")
                && endpoint.host().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }),
            "the desktop UI requires a loopback daemon endpoint"
        );

        let listener = StdListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let route = format!("/{}/{}", ulid::Ulid::new(), ulid::Ulid::new());
        let url = format!("ws://{}{route}", listener.local_addr()?);
        Ok(Self {
            listener,
            origin: origin.to_owned(),
            route,
            daemon_url,
            url,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub async fn run(self) -> Result<()> {
        let listener = TcpListener::from_std(self.listener)?;
        let mut peers = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _) = accepted?;
                    let origin = self.origin.clone();
                    let route = self.route.clone();
                    let daemon_url = self.daemon_url.clone();
                    peers.spawn(async move {
                        relay(stream, &origin, &route, &daemon_url).await;
                    });
                }
                _ = peers.join_next(), if !peers.is_empty() => {}
            }
        }
    }
}

#[allow(clippy::result_large_err)] // The WebSocket handshake fixes the callback error type.
async fn relay(stream: TcpStream, origin: &str, route: &str, daemon_url: &str) {
    let authorize = |request: &Request, response: Response| {
        let mut origins = request.headers().get_all(ORIGIN).iter();
        if request
            .uri()
            .path_and_query()
            .is_some_and(|value| value.as_str() == route)
            && origins.next().is_some_and(|value| value == origin)
            && origins.next().is_none()
        {
            Ok(response)
        } else {
            let mut denied = ErrorResponse::new(None);
            *denied.status_mut() = StatusCode::FORBIDDEN;
            Err(denied)
        }
    };
    let Ok(Ok(browser)) =
        tokio::time::timeout(Duration::from_secs(5), accept_hdr_async(stream, authorize)).await
    else {
        return;
    };
    let config = WebSocketConfig {
        max_frame_size: None,
        ..WebSocketConfig::default()
    };
    let Ok(Ok((daemon, _))) = tokio::time::timeout(
        Duration::from_secs(5),
        connect_async_with_config(daemon_url, Some(config), false),
    )
    .await
    else {
        return;
    };

    let (mut browser_tx, mut browser_rx) = browser.split();
    let (mut daemon_tx, mut daemon_rx) = daemon.split();
    // Drive both directions concurrently so a slow renderer cannot prevent
    // client commands from making progress toward the daemon.
    tokio::select! {
        _ = async {
            while let Some(Ok(frame)) = browser_rx.next().await {
                if daemon_tx.send(frame).await.is_err() { break; }
            }
        } => {}
        _ = async {
            while let Some(Ok(frame)) = daemon_rx.next().await {
                if browser_tx.send(frame).await.is_err() { break; }
            }
        } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::connect_async;
    use tungstenite::{client::IntoClientRequest, Message};

    #[tokio::test]
    async fn only_the_configured_origin_with_the_bootstrap_route_can_relay() {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let origin = "mew://app";
        let relay =
            DesktopRelay::bind(format!("ws://{}", upstream.local_addr().unwrap()), origin).unwrap();
        let url = relay.url().to_owned();
        let base = url.trim_end_matches(&relay.route);
        let _relay = tokio::spawn(relay.run());

        for (address, header) in [(base, origin), (url.as_str(), "http://localhost")] {
            let mut request = address.into_client_request().unwrap();
            request
                .headers_mut()
                .insert(ORIGIN, header.parse().unwrap());
            assert!(matches!(
                connect_async(request).await,
                Err(tungstenite::Error::Http(response)) if response.status() == StatusCode::FORBIDDEN
            ));
        }
        let echo = tokio::spawn(async move {
            let (stream, _) = upstream.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let received = socket.next().await.unwrap().unwrap();
            socket.send(received).await.unwrap();
        });
        let mut request = url.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert(ORIGIN, origin.parse().unwrap());
        let (mut browser, _) = connect_async(request).await.unwrap();
        browser.send(Message::text("hello")).await.unwrap();
        assert_eq!(
            browser.next().await.unwrap().unwrap(),
            Message::text("hello")
        );
        echo.await.unwrap();
    }
}
