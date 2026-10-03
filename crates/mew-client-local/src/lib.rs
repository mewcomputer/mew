//! Local WebSocket transport for clients connecting to an app-owned daemon.

use futures::{SinkExt, StreamExt};
use mew_client_core::{
    decode_server_message_lenient, encode_client_message, ClientConnection, ClientTransport,
    TransportError,
};
use mew_protocol::ClientMessage;
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};
use tungstenite::{protocol::WebSocketConfig, Message};

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
        let config = WebSocketConfig {
            max_frame_size: None,
            ..WebSocketConfig::default()
        };
        let (socket, _) = connect_async_with_config(&self.url, Some(config), false)
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
                protocol_version: mew_protocol::PROTOCOL_VERSION,
            })
            .unwrap();
            socket.send(Message::Text(response)).await.unwrap();
        });

        let transport = LocalWebSocketTransport::new(format!("ws://{address}"));
        let mut connection = transport.connect().await.unwrap();
        connection.send(ClientMessage::Ping).await.unwrap();
        assert!(matches!(
            connection.receive().await.unwrap(),
            Some(ServerMessage::Pong { version, .. }) if version == "test"
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn engine_negotiates_revision_after_remote_authorization_before_session_commands() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let ping = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(matches!(
                mew_protocol::decode_json::<ClientMessage>(&ping).unwrap(),
                ClientMessage::Ping
            ));
            for message in [
                ServerMessage::RemoteReady {
                    scope: mew_protocol::RemoteScope::Observe,
                },
                ServerMessage::Pong {
                    version: "test".into(),
                    protocol_version: mew_protocol::PROTOCOL_VERSION,
                },
            ] {
                socket
                    .send(Message::Text(mew_protocol::encode_json(&message).unwrap()))
                    .await
                    .unwrap();
            }
            let command = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(matches!(
                mew_protocol::decode_json::<ClientMessage>(&command).unwrap(),
                ClientMessage::ListSessions {}
            ));
        });
        let transport = LocalWebSocketTransport::new(format!("ws://{address}"));

        let mut engine = mew_client_core::ClientEngine::connect(&transport)
            .await
            .unwrap();
        assert_eq!(
            engine.state().remote_scope,
            Some(mew_protocol::RemoteScope::Observe)
        );
        assert_eq!(engine.state().daemon_version.as_deref(), Some("test"));
        engine.send(ClientMessage::ListSessions {}).await.unwrap();

        server.await.unwrap();
    }

    #[tokio::test]
    async fn engine_closes_incompatible_or_malformed_handshakes_without_sending_session_commands() {
        for response in [
            r#"{"type":"pong","version":"old"}"#.to_string(),
            format!(
                r#"{{"type":"pong","version":"future","protocol_version":{}}}"#,
                mew_protocol::PROTOCOL_VERSION + 1
            ),
            r#"{"type":"pong","version":"broken","protocol_version":"1"}"#.to_string(),
            r#"{"type":"session_ready","session_id":"premature"}"#.to_string(),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                let ping = socket.next().await.unwrap().unwrap().into_text().unwrap();
                assert!(matches!(
                    mew_protocol::decode_json::<ClientMessage>(&ping).unwrap(),
                    ClientMessage::Ping
                ));
                socket.send(Message::Text(response)).await.unwrap();
                assert!(matches!(
                    socket.next().await,
                    Some(Ok(Message::Close(_))) | None
                ));
            });
            let transport = LocalWebSocketTransport::new(format!("ws://{address}"));

            let error = mew_client_core::ClientEngine::connect(&transport)
                .await
                .err()
                .expect("handshake should fail");

            assert!(error.to_string().contains("protocol"));
            assert!(error.to_string().contains("restart or update"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn engine_times_out_when_daemon_never_answers_ping() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let ping = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(matches!(
                mew_protocol::decode_json::<ClientMessage>(&ping).unwrap(),
                ClientMessage::Ping
            ));
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        let transport = LocalWebSocketTransport::new(format!("ws://{address}"));

        let error = mew_client_core::ClientEngine::connect(&transport)
            .await
            .err()
            .expect("handshake should time out");

        assert!(error.to_string().contains("timed out"));
        server.await.unwrap();
    }
}
