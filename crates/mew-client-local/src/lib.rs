//! Local WebSocket transport for clients connecting to an app-owned daemon.

use futures::{SinkExt, StreamExt};
use mew_client_core::{
    decode_server_message_lenient, encode_client_message, ClientConnection, ClientTransport,
    TransportError,
};
use mew_protocol::ClientMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tungstenite::Message;

#[derive(Debug, Clone)]
pub struct LocalWebSocketTransport {
    url: String,
}

impl LocalWebSocketTransport {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

#[async_trait::async_trait]
impl ClientTransport for LocalWebSocketTransport {
    async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
        let (socket, _) = connect_async(&self.url)
            .await
            .map_err(|error| TransportError::Other(format!("connect to {}: {error}", self.url)))?;
        Ok(Box::new(LocalWebSocketConnection {
            socket,
            closed: false,
        }))
    }
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct LocalWebSocketConnection {
    socket: Socket,
    closed: bool,
}

#[async_trait::async_trait]
impl ClientConnection for LocalWebSocketConnection {
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

    async fn receive(&mut self) -> Result<Option<mew_protocol::ServerMessage>, TransportError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use mew_protocol::ServerMessage;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[tokio::test]
    async fn local_transport_round_trips_typed_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let Some(Ok(Message::Text(text))) = socket.next().await else {
                panic!("expected client message");
            };
            let message: ClientMessage = mew_protocol::decode_json(text.as_ref()).unwrap();
            assert!(matches!(message, ClientMessage::Ping));
            let response = mew_protocol::encode_json(&ServerMessage::Pong {
                version: "test".into(),
            })
            .unwrap();
            socket.send(Message::Text(response)).await.unwrap();
        });

        let transport = LocalWebSocketTransport::new(format!("ws://{address}"));
        let mut connection = transport.connect().await.unwrap();
        connection.send(ClientMessage::Ping).await.unwrap();
        assert!(matches!(
            connection.receive().await.unwrap(),
            Some(ServerMessage::Pong { version }) if version == "test"
        ));
        server.await.unwrap();
    }
}
