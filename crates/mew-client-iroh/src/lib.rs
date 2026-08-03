//! iroh transport for the framework-independent desktop client core.
//!
//! The daemon exposes the same JSON WebSocket protocol over an authenticated
//! iroh QUIC stream. This adapter owns only the transport and handshake; state
//! reduction remains in `mew-client-core`.

use futures::{SinkExt, StreamExt};
use iroh::{Endpoint, EndpointAddr, PublicKey};
use mew_client_core::{
    decode_server_message_lenient, encode_client_message, ClientConnection, ClientTransport,
    TransportError,
};
use mew_protocol::{ClientMessage, ServerMessage};
use tungstenite::{client::ClientRequestBuilder, Message};

pub const MEW_ALPN: &[u8] = b"mew/wire/0";

#[derive(Clone)]
pub struct IrohTransport {
    endpoint: Endpoint,
    daemon_id: String,
    pairing_token: Option<String>,
    device_name: String,
}

impl IrohTransport {
    pub fn new(
        endpoint: Endpoint,
        daemon_id: impl Into<String>,
        pairing_token: Option<String>,
        device_name: impl Into<String>,
    ) -> Self {
        Self {
            endpoint,
            daemon_id: daemon_id.into(),
            pairing_token,
            device_name: device_name.into(),
        }
    }

    pub fn daemon_id(&self) -> &str {
        &self.daemon_id
    }
}

#[async_trait::async_trait]
impl ClientTransport for IrohTransport {
    async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
        let daemon_key = parse_daemon_id(&self.daemon_id)?;
        let connection = self
            .endpoint
            .connect(EndpointAddr::new(daemon_key), MEW_ALPN)
            .await
            .map_err(|error| {
                TransportError::Other(format!("connect to daemon over iroh: {error}"))
            })?;
        let (send, receive) = connection
            .open_bi()
            .await
            .map_err(|error| TransportError::Other(format!("open daemon stream: {error}")))?;
        let mut socket = tokio_tungstenite::client_async(
            ClientRequestBuilder::new(
                "ws://daemon.mew/"
                    .parse()
                    .expect("static iroh WebSocket URL is valid"),
            )
            .with_header("Host", "daemon.mew")
            .with_header("Connection", "Upgrade")
            .with_header("Upgrade", "websocket")
            .with_header("Sec-WebSocket-Version", "13")
            .with_header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
            IrohStream::new(send, receive),
        )
        .await
        .map_err(|error| {
            TransportError::Other(format!("complete iroh WebSocket handshake: {error}"))
        })?
        .0;

        let hello = ClientMessage::RemoteHello {
            token: self.pairing_token.clone(),
            device_name: self.device_name.clone(),
        };
        let hello = encode_client_message(&hello)
            .map_err(|error| TransportError::Other(format!("encode remote handshake: {error}")))?;
        socket
            .send(Message::Text(hello))
            .await
            .map_err(|error| TransportError::Other(format!("send remote handshake: {error}")))?;

        Ok(Box::new(IrohConnection {
            socket,
            closed: false,
        }))
    }
}

fn parse_daemon_id(daemon_id: &str) -> Result<PublicKey, TransportError> {
    daemon_id
        .parse()
        .map_err(|error| TransportError::Other(format!("invalid daemon node id: {error}")))
}

type Socket = tokio_tungstenite::WebSocketStream<IrohStream>;

struct IrohConnection {
    socket: Socket,
    closed: bool,
}

#[async_trait::async_trait]
impl ClientConnection for IrohConnection {
    async fn send(&mut self, message: ClientMessage) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let text = encode_client_message(&message)
            .map_err(|error| TransportError::Other(format!("encode client message: {error}")))?;
        self.socket
            .send(Message::Text(text))
            .await
            .map_err(|error| TransportError::Other(format!("send client message: {error}")))
    }

    async fn receive(&mut self) -> Result<Option<ServerMessage>, TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        loop {
            let Some(message) = self.socket.next().await else {
                self.closed = true;
                return Ok(None);
            };
            let message = message.map_err(|error| {
                TransportError::Other(format!("receive server message: {error}"))
            })?;
            match message {
                Message::Text(text) => match decode_server_message_lenient(text.as_ref()) {
                    Ok(Some(message)) => return Ok(Some(message)),
                    Ok(None) => continue,
                    Err(error) => {
                        return Err(TransportError::Other(format!(
                            "decode server message: {error}"
                        )))
                    }
                },
                Message::Close(_) => {
                    self.closed = true;
                    return Ok(None);
                }
                Message::Ping(payload) => {
                    self.socket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| {
                            TransportError::Other(format!("send websocket pong: {error}"))
                        })?;
                }
                Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
            }
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.socket
            .send(Message::Close(None))
            .await
            .map_err(|error| TransportError::Other(format!("close websocket: {error}")))
    }
}

struct IrohStream {
    send: iroh::endpoint::SendStream,
    receive: iroh::endpoint::RecvStream,
}

impl IrohStream {
    fn new(send: iroh::endpoint::SendStream, receive: iroh::endpoint::RecvStream) -> Self {
        Self { send, receive }
    }
}

impl tokio::io::AsyncRead for IrohStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        tokio::io::AsyncRead::poll_read(std::pin::Pin::new(&mut this.receive), cx, buffer)
    }
}

impl tokio::io::AsyncWrite for IrohStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(&mut this.send), cx, bytes)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        tokio::io::AsyncWrite::poll_flush(std::pin::Pin::new(&mut this.send), cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        tokio::io::AsyncWrite::poll_shutdown(std::pin::Pin::new(&mut this.send), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_daemon_ids_fail_before_network_dial() {
        let error = parse_daemon_id("not-a-node").unwrap_err();
        assert!(matches!(
            error,
            TransportError::Other(message) if message.contains("invalid daemon node id")
        ));
    }
}
