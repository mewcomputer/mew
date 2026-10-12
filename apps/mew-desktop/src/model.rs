use mew_client_core::{ClientEvent, ClientState, ConnectionStatus};
use mew_ui_model::UiModel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellConnection {
    Starting,
    Connecting,
    Connected,
    Reconnecting,
    Error(String),
}

impl ShellConnection {
    pub fn label(&self) -> &str {
        match self {
            Self::Starting => "starting",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Error(_) => "connection failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShellModel {
    pub ui: UiModel,
    pub endpoint: Option<String>,
    pub connection: ShellConnection,
    pub attached_session: Option<String>,
    pub last_error: Option<String>,
}

impl ShellModel {
    pub fn starting(endpoint: Option<String>) -> Self {
        Self {
            ui: UiModel::default(),
            endpoint,
            connection: ShellConnection::Starting,
            attached_session: None,
            last_error: None,
        }
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.connection = ShellConnection::Error(error.clone());
        self.last_error = Some(error);
    }

    pub fn connected(&mut self) {
        self.connection = ShellConnection::Connected;
        self.last_error = None;
    }

    pub fn finish_prompt_submission(&mut self, accepted: bool) {
        if accepted {
            self.ui.composer.clear();
            self.last_error = None;
        } else {
            self.last_error = Some("Could not queue the prompt. Your draft is saved; reconnect the client to continue.".into());
        }
    }

    pub fn prepare_attachment(&mut self, reconnect_required: bool) {
        self.attached_session = None;
        if !reconnect_required {
            self.last_error = None;
        }
        self.ui.clear_attached_session_projection();
    }

    pub fn sync_client_metadata(&mut self, state: &ClientState) {
        self.attached_session = state.attached_session.clone();
        self.ui.sync_client_metadata(state);
    }

    pub fn sync_client_metadata_for_selection(
        &mut self,
        state: &ClientState,
        selection: Option<&str>,
    ) {
        self.sync_client_metadata(state);
        if selection.is_none() || self.attached_session.as_deref() != selection {
            self.attached_session = None;
            self.ui.clear_attached_session_projection();
        }
        self.ui.selected_session = selection.map(str::to_owned);
    }

    pub fn sync_client_transcript(&mut self, state: &ClientState) {
        self.ui.sync_client_transcript(state);
    }

    pub fn append_transcript_delta(&mut self, session_id: &str, delta: &str) -> bool {
        self.ui.append_transcript_delta(session_id, delta)
    }

    pub fn append_tool_progress(&mut self, session_id: &str, call_id: &str, chunk: &str) -> bool {
        self.ui.append_tool_progress(session_id, call_id, chunk)
    }

    pub fn session_is_ready(&self) -> bool {
        matches!(
            (
                self.attached_session.as_deref(),
                self.ui.selected_session.as_deref()
            ),
            (Some(attached), Some(selected)) if attached == selected
        )
    }

    pub fn apply_events(&mut self, events: &[ClientEvent], reconnect_required: bool) {
        for event in events {
            match event {
                ClientEvent::ConnectionChanged(status) => {
                    self.connection = match status {
                        ConnectionStatus::Connecting => ShellConnection::Connecting,
                        ConnectionStatus::Connected => {
                            self.last_error = None;
                            ShellConnection::Connected
                        }
                        ConnectionStatus::Backoff { .. } => ShellConnection::Reconnecting,
                        ConnectionStatus::Disconnected => {
                            self.last_error = Some("daemon connection closed".into());
                            ShellConnection::Error("daemon connection closed".into())
                        }
                    };
                }
                ClientEvent::SessionReady { session_id } => {
                    if !reconnect_required {
                        self.last_error = None;
                    }
                    self.ui.selected_session = Some(session_id.clone());
                }
                ClientEvent::SessionHistoryLoaded { session_id }
                | ClientEvent::SessionHistoryPageLoaded { session_id, .. } => {
                    self.ui.selected_session = Some(session_id.clone());
                }
                ClientEvent::PermissionModeChanged { mode } => {
                    self.ui.permission_mode = Some(mode.clone());
                }
                ClientEvent::SessionListChanged => {}
                ClientEvent::Error(message) => self.last_error = Some(message.clone()),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigating_with_an_ambiguous_attachment_preserves_its_recovery_message() {
        let mut model = ShellModel::starting(None);
        model.connected();
        model.attached_session = Some("a".into());
        model.ui.selected_session = Some("b".into());
        model.ui.composer = "b draft".into();
        let mut state = ClientState::default();
        for message in [
            mew_protocol::ServerMessage::Error {
                message: "attachment failed".into(),
            },
            mew_protocol::ServerMessage::SessionReady {
                session_id: "a".into(),
                cwd: None,
                model: None,
                provider: None,
                permission_mode: None,
            },
        ] {
            let events = state.apply_server_message(message);
            model.apply_events(&events, true);
            model.sync_client_metadata_for_selection(&state.ui_metadata_snapshot(), Some("b"));
        }
        model.prepare_attachment(true);
        assert_eq!(model.last_error.as_deref(), Some("attachment failed"));
        assert_eq!(model.ui.composer, "b draft");
        assert_eq!(model.ui.selected_session.as_deref(), Some("b"));
        assert!(!model.session_is_ready());
        model.prepare_attachment(false);
        assert!(model.last_error.is_none());
    }

    #[test]
    fn explicit_unbound_selection_survives_attached_daemon_activity_and_history() {
        use mew_protocol::{ServerMessage, SessionState};
        let mut client = ClientState::default();
        client.apply_server_message(ServerMessage::SessionReady {
            session_id: "closed-task".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
        let mut model = ShellModel::starting(None);
        model.ui.composer = "new task draft".into();
        for event in [
            ServerMessage::SessionActivityChanged {
                session_id: "closed-task".into(),
                activity: SessionState::Running,
            },
            ServerMessage::SessionTitleChanged {
                session_id: "closed-task".into(),
                title: "background work".into(),
            },
        ] {
            let events = client.apply_server_message(event);
            model.apply_events(&events, false);
            model.sync_client_metadata_for_selection(&client.ui_metadata_snapshot(), None);
            assert!(model.ui.selected_session.is_none());
            assert!(model.attached_session.is_none());
            assert!(!model.session_is_ready());
            assert!(!model.ui.running);
            assert_eq!(model.ui.composer, "new task draft");
        }
        model.apply_events(
            &[ClientEvent::SessionHistoryLoaded {
                session_id: "closed-task".into(),
            }],
            false,
        );
        model.sync_client_metadata_for_selection(&client.ui_metadata_snapshot(), None);
        assert!(model.ui.selected_session.is_none());
        assert!(!model.session_is_ready());
    }

    #[test]
    fn background_completion_is_acknowledged_only_by_its_attachment_or_next_turn() {
        use mew_protocol::{AlertKind, ServerMessage, SessionState};

        let mut client = ClientState::default();
        client.apply_server_message(ServerMessage::SessionList {
            sessions: serde_json::from_value(serde_json::json!([
                { "session_id": "background", "state": "idle", "created_at": 0, "client_count": 0 },
                { "session_id": "selected", "state": "active", "created_at": 0, "client_count": 1 }
            ]))
            .unwrap(),
        });
        let ready = |session_id: &str| ServerMessage::SessionReady {
            session_id: session_id.into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        };
        let complete = |session_id: &str| ServerMessage::SessionAlert {
            session_id: session_id.into(),
            title: "work".into(),
            kind: AlertKind::TurnComplete,
            detail: None,
        };
        client.apply_server_message(ready("selected"));
        let mut model = ShellModel::starting(None);
        let events = client.apply_server_message(complete("background"));
        model.apply_events(&events, false);
        model.sync_client_metadata(&client.ui_metadata_snapshot());
        assert!(model.ui.conversations[0].unread_completion);
        assert!(!model.ui.conversations[1].unread_completion);
        assert_eq!(model.attached_session.as_deref(), Some("selected"));

        client.apply_server_message(ready("selected"));
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(model.ui.conversations[0].unread_completion);
        client.apply_server_message(ready("background"));
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(!model.ui.conversations[0].unread_completion);
        client.apply_server_message(complete("background"));
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(!model.ui.conversations[0].unread_completion);

        client.apply_server_message(ready("selected"));
        client.apply_server_message(complete("background"));
        client.apply_server_message(ServerMessage::SessionActivityChanged {
            session_id: "selected".into(),
            activity: SessionState::Running,
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(model.ui.conversations[0].unread_completion);
        client.apply_server_message(ServerMessage::SessionActivityChanged {
            session_id: "background".into(),
            activity: SessionState::Running,
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(!model.ui.conversations[0].unread_completion);

        client.apply_server_message(complete("background"));
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(model.ui.conversations[0].unread_completion);
        let other_daemon = ClientState {
            session_list: client.session_list.clone(),
            ..ClientState::default()
        };
        model.sync_client_metadata(&other_daemon.ui_snapshot());
        assert!(model
            .ui
            .conversations
            .iter()
            .all(|item| !item.unread_completion));
    }

    #[test]
    fn background_failure_keeps_its_session_and_survives_stale_session_metadata() {
        use mew_protocol::{AlertKind, ServerMessage, SessionInfo};

        let sessions: Vec<SessionInfo> = serde_json::from_value(serde_json::json!([
            { "session_id": "background", "state": "idle", "created_at": 0, "client_count": 0 },
            { "session_id": "selected", "state": "active", "created_at": 0, "client_count": 1 }
        ]))
        .unwrap();
        let mut client = ClientState::default();
        client.apply_server_message(ServerMessage::SessionList {
            sessions: sessions.clone(),
        });
        client.apply_server_message(ServerMessage::SessionReady {
            session_id: "selected".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
        let mut model = ShellModel::starting(None);
        let events = client.apply_server_message(ServerMessage::SessionAlert {
            session_id: "background".into(),
            title: "background work".into(),
            kind: AlertKind::TurnFailed,
            detail: Some("provider unavailable".into()),
        });
        assert!(events
            .iter()
            .any(|event| matches!(event, ClientEvent::SessionListChanged)));
        model.apply_events(&events, false);
        model.sync_client_metadata(&client.ui_snapshot());
        assert_eq!(model.attached_session.as_deref(), Some("selected"));
        assert!(model.ui.conversations[0].last_turn_failed);
        assert_eq!(
            model.ui.conversations[0].failure_detail.as_deref(),
            Some("provider unavailable")
        );
        assert!(!model.ui.conversations[1].last_turn_failed);
        assert!(model.last_error.is_none());

        client.apply_server_message(ServerMessage::SessionList { sessions });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(model.ui.conversations[0].last_turn_failed);
        assert!(!model.ui.conversations[1].last_turn_failed);

        client.apply_server_message(ServerMessage::SessionAlert {
            session_id: "background".into(),
            title: "background work".into(),
            kind: AlertKind::TurnComplete,
            detail: None,
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(!model.ui.conversations[0].last_turn_failed);
        assert!(model.ui.conversations[0].failure_detail.is_none());
        let mut stale_sessions = client.session_list.clone();
        stale_sessions[0].last_turn_failed = true;
        client.apply_server_message(ServerMessage::SessionList {
            sessions: stale_sessions,
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(!model.ui.conversations[0].last_turn_failed);
    }

    #[test]
    fn retry_failure_detail_belongs_to_the_selected_session() {
        use mew_protocol::{AlertKind, ServerMessage, SessionState};

        let mut client = ClientState::default();
        client.apply_server_message(ServerMessage::SessionList {
            sessions: serde_json::from_value(serde_json::json!([
                { "session_id": "a", "state": "idle", "created_at": 0, "client_count": 0 },
                { "session_id": "b", "state": "active", "created_at": 0, "client_count": 1 }
            ]))
            .unwrap(),
        });
        client.apply_server_message(ServerMessage::SessionReady {
            session_id: "b".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
        let mut model = ShellModel::starting(None);
        let events = client.apply_server_message(ServerMessage::ErrorEvent {
            message: "b's provider failed".into(),
        });
        model.apply_events(&events, false);
        client.apply_server_message(ServerMessage::SessionAlert {
            session_id: "b".into(),
            title: "b".into(),
            kind: AlertKind::TurnFailed,
            detail: None,
        });
        client.apply_server_message(ServerMessage::SessionAlert {
            session_id: "a".into(),
            title: "a".into(),
            kind: AlertKind::TurnFailed,
            detail: None,
        });
        let events = client.apply_server_message(ServerMessage::SessionReady {
            session_id: "a".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
        model.apply_events(&events, false);
        model.sync_client_metadata(&client.ui_snapshot());
        assert_eq!(model.ui.selected_session.as_deref(), Some("a"));
        assert!(model.ui.conversations[0].failure_detail.is_none());
        assert_eq!(
            model.ui.conversations[1].failure_detail.as_deref(),
            Some("b's provider failed")
        );
        assert!(model.last_error.is_none());
        client.apply_server_message(ServerMessage::SessionActivityChanged {
            session_id: "b".into(),
            activity: SessionState::Running,
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(!model.ui.conversations[1].last_turn_failed);
        assert!(model.ui.conversations[1].failure_detail.is_none());
    }

    #[test]
    fn composer_stays_busy_between_provider_messages_until_daemon_activity_ends() {
        use mew_message::{Finish, ProviderEventWire};
        use mew_protocol::{ServerMessage, SessionState};

        let mut client = ClientState::default();
        client.apply_server_message(ServerMessage::SessionReady {
            session_id: "session-1".into(),
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
        });
        let mut model = ShellModel::starting(None);
        client.apply_server_message(ServerMessage::SessionActivityChanged {
            session_id: "session-1".into(),
            activity: SessionState::Running,
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(model.ui.running);

        client.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::MessageEnd {
                finish: Finish::ToolUse,
                usage: Default::default(),
                cost: 0.0,
                manifest: None,
            },
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(model.ui.running);

        client.apply_server_message(ServerMessage::SessionActivityChanged {
            session_id: "session-1".into(),
            activity: SessionState::Active,
        });
        model.sync_client_metadata(&client.ui_snapshot());
        assert!(!model.ui.running);
    }

    #[test]
    fn shell_model_tracks_connection_and_session_attachment() {
        let mut model = ShellModel::starting(Some("ws://127.0.0.1:25566".into()));
        model.apply_events(
            &[
                ClientEvent::ConnectionChanged(ConnectionStatus::Connected),
                ClientEvent::SessionReady {
                    session_id: "session-1".into(),
                },
            ],
            false,
        );
        model.sync_client_metadata(&ClientState {
            attached_session: Some("session-1".into()),
            ..ClientState::default()
        });

        assert_eq!(model.connection, ShellConnection::Connected);
        assert_eq!(model.ui.selected_session.as_deref(), Some("session-1"));
        assert_eq!(model.attached_session.as_deref(), Some("session-1"));
    }

    #[test]
    fn shell_model_surfaces_reconnect_and_error_states() {
        let mut model = ShellModel::starting(None);
        model.apply_events(
            &[ClientEvent::ConnectionChanged(ConnectionStatus::Backoff {
                attempt: 2,
                error: "closed".into(),
            })],
            false,
        );
        assert_eq!(model.connection, ShellConnection::Reconnecting);

        model.apply_events(&[ClientEvent::Error("bad handshake".into())], false);
        assert_eq!(model.connection, ShellConnection::Reconnecting);
        assert_eq!(model.last_error.as_deref(), Some("bad handshake"));
    }

    #[test]
    fn daemon_errors_do_not_masquerade_as_connection_failures() {
        let mut model = ShellModel::starting(None);
        model.connected();
        model.apply_events(
            &[ClientEvent::Error("provider is not configured".into())],
            false,
        );

        assert_eq!(model.connection, ShellConnection::Connected);
        assert_eq!(
            model.last_error.as_deref(),
            Some("provider is not configured")
        );
    }

    #[test]
    fn permission_mode_change_updates_the_composer_projection() {
        let mut model = ShellModel::starting(None);
        model.apply_events(
            &[ClientEvent::PermissionModeChanged {
                mode: "permissive".into(),
            }],
            false,
        );

        assert_eq!(model.ui.permission_mode.as_deref(), Some("permissive"));
    }

    #[test]
    fn disconnected_state_surfaces_a_recoverable_error() {
        let mut model = ShellModel::starting(None);
        model.apply_events(
            &[ClientEvent::ConnectionChanged(
                ConnectionStatus::Disconnected,
            )],
            false,
        );

        assert_eq!(model.connection.label(), "connection failed");
        assert_eq!(
            model.last_error.as_deref(),
            Some("daemon connection closed")
        );
    }

    #[test]
    fn session_selection_is_not_protocol_readiness() {
        let mut model = ShellModel::starting(None);
        model.ui.selected_session = Some("session-1".into());
        assert!(!model.session_is_ready());

        model.attached_session = Some("session-2".into());
        assert!(!model.session_is_ready());

        model.attached_session = Some("session-1".into());
        assert!(model.session_is_ready());
    }

    #[test]
    fn pending_attach_hides_the_previous_session_projection() {
        let mut model = ShellModel::starting(None);
        model.attached_session = Some("session-a".into());
        model.ui.selected_session = Some("session-b".into());
        model
            .ui
            .pending_actions
            .push(mew_client_core::PendingAction {
                request_id: "permission-1".into(),
                kind: mew_client_core::ActionKind::WorkspacePermission {
                    path: "/tmp/project".into(),
                },
            });
        model.ui.set_composer("stale");

        model.sync_client_metadata_for_selection(
            &ClientState {
                attached_session: Some("session-a".into()),
                ..ClientState::default()
            },
            Some("session-b"),
        );

        assert_eq!(model.attached_session, None);
        assert!(model.ui.pending_actions.is_empty());
        assert!(model.ui.transcript.is_empty());
        assert!(!model.session_is_ready());
    }
}
