//! Async transport boundary. Concrete WebSocket and iroh adapters live above
//! this crate; tests use the in-memory implementation below.

use async_trait::async_trait;
use mew_protocol::{ClientMessage, ServerMessage};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("transport is closed")]
    Closed,
    #[error("transport error: {0}")]
    Other(String),
}

#[async_trait]
pub trait ClientTransport: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError>;
}

#[async_trait]
pub trait ClientConnection: Send {
    async fn send(&mut self, message: ClientMessage) -> Result<(), TransportError>;
    async fn receive(&mut self) -> Result<Option<ServerMessage>, TransportError>;
    async fn close(&mut self) -> Result<(), TransportError>;
}

#[derive(Clone, Default)]
pub struct InMemoryTransport {
    state: Arc<Mutex<InMemoryState>>,
}

#[derive(Default)]
struct InMemoryState {
    inbound: VecDeque<ServerMessage>,
    outbound: Vec<ClientMessage>,
    closed: bool,
}

impl InMemoryTransport {
    pub fn push_server_message(&self, message: ServerMessage) {
        self.state
            .lock()
            .expect("in-memory transport lock")
            .inbound
            .push_back(message);
    }

    pub fn sent_messages(&self) -> Vec<ClientMessage> {
        self.state
            .lock()
            .expect("in-memory transport lock")
            .outbound
            .clone()
    }
}

pub struct InMemoryConnection {
    state: Arc<Mutex<InMemoryState>>,
}

#[async_trait]
impl ClientTransport for InMemoryTransport {
    async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
        if self.state.lock().expect("in-memory transport lock").closed {
            return Err(TransportError::Closed);
        }
        Ok(Box::new(InMemoryConnection {
            state: Arc::clone(&self.state),
        }))
    }
}

#[async_trait]
impl ClientConnection for InMemoryConnection {
    async fn send(&mut self, message: ClientMessage) -> Result<(), TransportError> {
        let mut state = self.state.lock().expect("in-memory transport lock");
        if state.closed {
            return Err(TransportError::Closed);
        }
        state.outbound.push(message);
        Ok(())
    }

    async fn receive(&mut self) -> Result<Option<ServerMessage>, TransportError> {
        let mut state = self.state.lock().expect("in-memory transport lock");
        if state.closed {
            return Err(TransportError::Closed);
        }
        Ok(state.inbound.pop_front())
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.state.lock().expect("in-memory transport lock").closed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_transport_round_trips_commands_and_events() {
        let transport = InMemoryTransport::default();
        transport.push_server_message(ServerMessage::Pong {
            version: "test".into(),
        });
        let mut connection = transport.connect().await.unwrap();

        connection.send(ClientMessage::Ping).await.unwrap();
        assert!(matches!(
            connection.receive().await.unwrap(),
            Some(ServerMessage::Pong { version }) if version == "test"
        ));
        assert!(matches!(
            transport.sent_messages().as_slice(),
            [ClientMessage::Ping]
        ));

        connection.close().await.unwrap();
        assert!(matches!(
            connection.receive().await,
            Err(TransportError::Closed)
        ));
    }
}
