//! A small headless client driver that connects a transport to the reducer.

use crate::{ClientConnection, ClientEvent, ClientState, ClientTransport, TransportError};
use mew_protocol::ClientMessage;

pub struct ClientEngine {
    state: ClientState,
    connection: Box<dyn ClientConnection>,
}

impl ClientEngine {
    pub async fn connect(transport: &dyn ClientTransport) -> Result<Self, TransportError> {
        let connection = transport.connect().await?;
        let mut state = ClientState::default();
        state.set_connection_status(crate::ConnectionStatus::Connected);
        Ok(Self { state, connection })
    }

    pub fn state(&self) -> &ClientState {
        &self.state
    }

    pub fn ui_snapshot(&self) -> ClientState {
        self.state.ui_snapshot()
    }

    pub fn ui_metadata_snapshot(&self) -> ClientState {
        self.state.ui_metadata_snapshot()
    }

    pub fn state_mut(&mut self) -> &mut ClientState {
        &mut self.state
    }

    pub async fn send(&mut self, message: ClientMessage) -> Result<(), TransportError> {
        self.connection.send(message).await
    }

    pub async fn send_prompt(&mut self, text: String) -> Result<(), TransportError> {
        let session_id = self.state.attached_session.clone().ok_or_else(|| {
            TransportError::Other("cannot send a prompt without an attached session".into())
        })?;
        self.state.record_prompt(&session_id, text.clone());
        self.send(ClientMessage::Prompt {
            text,
            attachments: Vec::new(),
        })
        .await
    }

    pub async fn yield_control(&mut self) -> Result<(), TransportError> {
        self.send(ClientMessage::YieldControl {}).await
    }

    pub async fn rename_session(
        &mut self,
        session_id: String,
        title: String,
    ) -> Result<(), TransportError> {
        self.send(ClientMessage::RenameSession { session_id, title })
            .await
    }

    pub async fn archive_session(
        &mut self,
        session_id: String,
        archived: bool,
    ) -> Result<(), TransportError> {
        self.send(ClientMessage::ArchiveSession {
            session_id,
            archived,
        })
        .await
    }

    pub async fn pin_session(
        &mut self,
        session_id: String,
        pinned: bool,
    ) -> Result<(), TransportError> {
        self.send(ClientMessage::PinSession { session_id, pinned })
            .await
    }

    /// Subscribe (or unsubscribe) to filesystem change notifications for a
    /// session's workspace. Changes arrive as `ServerMessage::FsChanged`.
    pub async fn watch_workspace(
        &mut self,
        session_id: String,
        enabled: bool,
    ) -> Result<(), TransportError> {
        self.send(ClientMessage::WatchWorkspace {
            session_id,
            enabled,
        })
        .await
    }

    pub async fn receive(&mut self) -> Result<Vec<ClientEvent>, TransportError> {
        match self.connection.receive().await? {
            Some(message) => Ok(self.state.apply_server_message(message)),
            None => {
                self.state
                    .set_connection_status(crate::ConnectionStatus::Disconnected);
                Err(TransportError::Closed)
            }
        }
    }

    pub async fn close(&mut self) -> Result<(), TransportError> {
        self.state
            .set_connection_status(crate::ConnectionStatus::Disconnected);
        self.connection.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionKind, ClientEvent, InMemoryTransport};
    use mew_message::{Finish, Part, PartBase, ProviderEventWire, TextPart, Tokens};
    use mew_protocol::{ClientKind, ServerMessage};
    use ulid::Ulid;

    const SESSION_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

    #[tokio::test]
    async fn headless_engine_handles_prompt_stream_and_required_action() {
        let transport = InMemoryTransport::default();
        let mut engine = ClientEngine::connect(&transport).await.unwrap();
        assert!(matches!(
            engine.state().connection,
            Some(crate::ConnectionStatus::Connected)
        ));

        transport.push_server_message(ServerMessage::SessionReady {
            session_id: SESSION_ID.into(),
            cwd: Some("/tmp/project".into()),
            model: Some("fake".into()),
            provider: Some("fake".into()),
            permission_mode: Some("standard".into()),
        });
        let events = engine.receive().await.unwrap();
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SessionReady { session_id }] if session_id == SESSION_ID
        ));

        engine.send_prompt("hello".into()).await.unwrap();
        assert!(matches!(
            transport.sent_messages().as_slice(),
            [ClientMessage::Prompt { text, .. }] if text == "hello"
        ));

        let session_id = Ulid::from_string(SESSION_ID).unwrap();
        let message_id = Ulid::new();
        let part_id = Ulid::new();
        transport.push_server_message(ServerMessage::Provider {
            event: ProviderEventWire::PartStart {
                part: Part::Text(TextPart {
                    base: PartBase {
                        id: part_id,
                        message_id,
                        session_id,
                    },
                    text: "hi".into(),
                    synthetic: false,
                }),
            },
        });
        engine.receive().await.unwrap();
        transport.push_server_message(ServerMessage::Provider {
            event: ProviderEventWire::PartDelta {
                part_id,
                field: "text".into(),
                delta: " there".into(),
            },
        });
        let events = engine.receive().await.unwrap();
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::TextDelta { delta, .. }] if delta == " there"
        ));

        transport.push_server_message(ServerMessage::PermissionRequest {
            request_id: "request-1".into(),
            tool_name: "shell".into(),
            input: serde_json::json!({"command": "pwd"}),
        });
        let events = engine.receive().await.unwrap();
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::RequiredActionChanged { request_id, .. }] if request_id == "request-1"
        ));
        assert!(matches!(
            &engine.state().session(SESSION_ID).unwrap().pending_actions[0].kind,
            ActionKind::Permission { tool_name, .. } if tool_name == "shell"
        ));

        transport.push_server_message(ServerMessage::Provider {
            event: ProviderEventWire::MessageEnd {
                finish: Finish::Stop,
                usage: Tokens {
                    input: 3,
                    output: 2,
                    ..Tokens::default()
                },
                cost: 0.01,
                manifest: None,
            },
        });
        let events = engine.receive().await.unwrap();
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::TurnEnded {
                input_tokens: 3,
                output_tokens: 2,
                ..
            }]
        ));

        engine
            .send(ClientMessage::NewSession {
                cwd: None,
                client_kind: ClientKind::Desktop,
            })
            .await
            .unwrap();
        assert!(matches!(
            transport.sent_messages().last(),
            Some(ClientMessage::NewSession {
                client_kind: ClientKind::Desktop,
                ..
            })
        ));

        let history = engine.state().session(SESSION_ID).unwrap().messages.clone();
        engine.close().await.unwrap();

        let mut reconnected = ClientEngine::connect(&transport).await.unwrap();
        transport.push_server_message(ServerMessage::SessionReady {
            session_id: SESSION_ID.into(),
            cwd: Some("/tmp/project".into()),
            model: Some("fake".into()),
            provider: Some("fake".into()),
            permission_mode: Some("standard".into()),
        });
        reconnected.receive().await.unwrap();
        transport.push_server_message(ServerMessage::SessionHistory { messages: history });
        let events = reconnected.receive().await.unwrap();
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SessionHistoryLoaded { session_id }] if session_id == SESSION_ID
        ));
        assert_eq!(
            reconnected
                .state()
                .session(SESSION_ID)
                .unwrap()
                .messages
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn engine_sends_session_management_and_workspace_messages() {
        let transport = InMemoryTransport::default();
        let mut engine = ClientEngine::connect(&transport).await.unwrap();

        engine.yield_control().await.unwrap();
        engine
            .rename_session("sess-1".into(), "renamed".into())
            .await
            .unwrap();
        engine.archive_session("sess-1".into(), true).await.unwrap();
        engine.pin_session("sess-1".into(), false).await.unwrap();
        engine.watch_workspace("sess-1".into(), true).await.unwrap();

        let sent = transport.sent_messages();
        assert!(matches!(sent[0], ClientMessage::YieldControl {}));
        assert!(matches!(
            &sent[1],
            ClientMessage::RenameSession { session_id, title }
                if session_id == "sess-1" && title == "renamed"
        ));
        assert!(matches!(
            &sent[2],
            ClientMessage::ArchiveSession { session_id, archived: true }
                if session_id == "sess-1"
        ));
        assert!(matches!(
            &sent[3],
            ClientMessage::PinSession { session_id, pinned: false }
                if session_id == "sess-1"
        ));
        assert!(matches!(
            &sent[4],
            ClientMessage::WatchWorkspace { session_id, enabled: true }
                if session_id == "sess-1"
        ));
    }
}
