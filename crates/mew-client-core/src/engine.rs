//! A small headless client driver that connects a transport to the reducer.

use crate::{ClientConnection, ClientEvent, ClientState, ClientTransport, TransportError};
use mew_protocol::{ClientMessage, ServerMessage, PROTOCOL_VERSION};
use std::time::Duration;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

pub struct ClientEngine {
    state: ClientState,
    connection: Box<dyn ClientConnection>,
    session_list_pending: bool,
    session_list_dirty: bool,
}

impl ClientEngine {
    pub async fn connect(transport: &dyn ClientTransport) -> Result<Self, TransportError> {
        let mut connection = transport.connect().await?;
        let mut state = ClientState::default();
        let handshake = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            connection.send(ClientMessage::Ping).await?;
            loop {
                match connection.receive().await? {
                    Some(ServerMessage::Pong { protocol_version, version }) => {
                        if protocol_version != PROTOCOL_VERSION {
                            return Err(TransportError::Other(format!(
                                "incompatible daemon protocol {protocol_version} (daemon {version}); \
                                 this app requires {PROTOCOL_VERSION}"
                            )));
                        }
                        state.apply_server_message(ServerMessage::Pong { protocol_version, version });
                        return Ok(());
                    }
                    Some(message @ ServerMessage::RemoteReady { .. }) => {
                        state.apply_server_message(message);
                    }
                    Some(_) => return Err(TransportError::Other(
                        "daemon sent session data before completing the protocol handshake".into(),
                    )),
                    None => return Err(TransportError::Closed),
                }
            }
        }).await;
        let result = handshake.unwrap_or_else(|_| {
            Err(TransportError::Other(
                "daemon protocol handshake timed out".into(),
            ))
        });
        if let Err(error) = result {
            let _ = tokio::time::timeout(Duration::from_millis(250), connection.close()).await;
            return Err(TransportError::Other(format!(
                "daemon protocol handshake failed: {error}; restart or update the daemon and app to connect"
            )));
        }
        state.set_connection_status(crate::ConnectionStatus::Connected);
        Ok(Self {
            state,
            connection,
            session_list_pending: false,
            session_list_dirty: false,
        })
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
        if matches!(message, ClientMessage::ListSessions) {
            if self.session_list_pending {
                self.session_list_dirty = true;
                return Ok(());
            }
            self.connection.send(message).await?;
            self.session_list_pending = true;
            Ok(())
        } else {
            self.connection.send(message).await
        }
    }

    pub async fn send_prompt(&mut self, text: String) -> Result<(), TransportError> {
        let session_id = self.state.attached_session.clone().ok_or_else(|| {
            TransportError::Other("cannot send a prompt without an attached session".into())
        })?;
        self.send(ClientMessage::Prompt {
            text: text.clone(),
            attachments: Vec::new(),
        })
        .await?;
        self.state.record_prompt(&session_id, text);
        Ok(())
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
            Some(message) => {
                let personas = matches!(&message, ServerMessage::SessionReady { .. });
                match &message {
                    ServerMessage::SessionList { .. } => self.session_list_pending = false,
                    ServerMessage::SessionReady { .. }
                    | ServerMessage::SessionMetaChanged { .. } => self.session_list_dirty = true,
                    ServerMessage::SessionActivityChanged {
                        session_id,
                        activity,
                    } => {
                        let running = *activity == mew_protocol::SessionState::Running;
                        if self
                            .state
                            .session(session_id)
                            .is_none_or(|session| session.running != running)
                            || !self
                                .state
                                .session_list
                                .iter()
                                .any(|session| session.session_id == *session_id)
                        {
                            self.session_list_dirty = true;
                        }
                    }
                    _ => {}
                }
                let events = self.state.apply_server_message(message);
                if self.session_list_dirty && !self.session_list_pending {
                    self.send(ClientMessage::ListSessions).await?;
                    self.session_list_dirty = false;
                }
                if personas {
                    self.send(ClientMessage::ListPersonas).await?;
                }
                Ok(events)
            }
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
    async fn connection_rejects_legacy_daemon_before_sending_session_commands() {
        let transport = InMemoryTransport::default();
        transport.push_server_message(
            serde_json::from_str(r#"{"type":"pong","version":"0.2.0"}"#).unwrap(),
        );

        let result = ClientEngine::connect(&transport).await;

        let error = result.err().expect("legacy daemon must be rejected");
        assert!(error.to_string().contains("protocol"));
        assert!(error.to_string().contains("restart"));
        assert!(matches!(
            transport.sent_messages().as_slice(),
            [ClientMessage::Ping]
        ));
    }

    #[tokio::test]
    async fn previous_native_schema_cannot_activate_a_client() {
        let transport = InMemoryTransport::default();
        transport.push_server_message(ServerMessage::Pong {
            version: "previous-native".into(),
            protocol_version: 1,
        });
        let error = ClientEngine::connect(&transport)
            .await
            .err()
            .expect("previous native schema must be rejected");
        assert!(error.to_string().contains("protocol"));
        assert!(matches!(
            transport.sent_messages().as_slice(),
            [ClientMessage::Ping]
        ));
    }

    #[tokio::test]
    async fn headless_engine_handles_prompt_stream_and_required_action() {
        let transport = InMemoryTransport::default();
        transport.push_server_message(ServerMessage::Pong {
            version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
        });
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
            [ClientMessage::Ping, ClientMessage::ListSessions, ClientMessage::ListPersonas, ClientMessage::Prompt { text, .. }] if text == "hello"
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
            [ClientEvent::ProviderMessageEnded {
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

        transport.push_server_message(ServerMessage::Pong {
            version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
        });
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
    async fn history_pages_replace_then_prepend_and_update_cursor() {
        let transport = InMemoryTransport::default();
        transport.push_server_message(ServerMessage::Pong {
            version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
        });
        let mut engine = ClientEngine::connect(&transport).await.unwrap();
        transport.push_server_message(ServerMessage::SessionReady {
            session_id: SESSION_ID.into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
        engine.receive().await.unwrap();

        let newest = mew_message::Message {
            id: mew_message::MessageId::new(),
            session_id: mew_message::SessionId::new(),
            role: mew_message::Role::User,
            parts: Vec::new(),
            time: mew_message::Time {
                created: 2,
                completed: None,
            },
            assistant: None,
        };
        let oldest = mew_message::Message {
            id: mew_message::MessageId::new(),
            session_id: mew_message::SessionId::new(),
            role: mew_message::Role::Assistant,
            parts: Vec::new(),
            time: mew_message::Time {
                created: 1,
                completed: None,
            },
            assistant: None,
        };
        transport.push_server_message(ServerMessage::SessionHistoryPage {
            session_id: SESSION_ID.into(),
            messages: vec![newest.clone()],
            next_cursor: Some(1),
            replace: true,
        });
        let events = engine.receive().await.unwrap();
        assert!(matches!(
            events.as_slice(),
            [ClientEvent::SessionHistoryPageLoaded { session_id, replace: true }]
                if session_id == SESSION_ID
        ));
        assert_eq!(
            engine.state().session(SESSION_ID).unwrap().history_before,
            Some(1)
        );

        transport.push_server_message(ServerMessage::SessionHistoryPage {
            session_id: SESSION_ID.into(),
            messages: vec![oldest.clone()],
            next_cursor: None,
            replace: false,
        });
        engine.receive().await.unwrap();
        let session = engine.state().session(SESSION_ID).unwrap();
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].id, oldest.id);
        assert_eq!(session.messages[1].id, newest.id);
        assert_eq!(session.history_before, None);
    }

    #[tokio::test]
    async fn engine_sends_session_management_and_workspace_messages() {
        let transport = InMemoryTransport::default();
        transport.push_server_message(ServerMessage::Pong {
            version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
        });
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
        assert!(matches!(sent[0], ClientMessage::Ping));
        let sent = &sent[1..];
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
