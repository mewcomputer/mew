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

    pub fn sync_client_metadata(&mut self, state: &ClientState) {
        self.attached_session = state.attached_session.clone();
        self.ui.sync_client_metadata(state);
    }

    pub fn sync_client_metadata_while_attaching(
        &mut self,
        state: &ClientState,
        target_session: &str,
    ) {
        self.sync_client_metadata(state);
        if self.attached_session.as_deref() != Some(target_session) {
            self.attached_session = None;
            self.ui.clear_attached_session_projection();
        }
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

    pub fn apply_events(&mut self, events: &[ClientEvent]) {
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
                ClientEvent::SessionReady { session_id }
                | ClientEvent::SessionHistoryLoaded { session_id } => {
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
    fn shell_model_tracks_connection_and_session_attachment() {
        let mut model = ShellModel::starting(Some("ws://127.0.0.1:25566".into()));
        model.apply_events(&[
            ClientEvent::ConnectionChanged(ConnectionStatus::Connected),
            ClientEvent::SessionReady {
                session_id: "session-1".into(),
            },
        ]);
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
        model.apply_events(&[ClientEvent::ConnectionChanged(ConnectionStatus::Backoff {
            attempt: 2,
            error: "closed".into(),
        })]);
        assert_eq!(model.connection, ShellConnection::Reconnecting);

        model.apply_events(&[ClientEvent::Error("bad handshake".into())]);
        assert_eq!(model.connection, ShellConnection::Reconnecting);
        assert_eq!(model.last_error.as_deref(), Some("bad handshake"));
    }

    #[test]
    fn daemon_errors_do_not_masquerade_as_connection_failures() {
        let mut model = ShellModel::starting(None);
        model.connected();
        model.apply_events(&[ClientEvent::Error("provider is not configured".into())]);

        assert_eq!(model.connection, ShellConnection::Connected);
        assert_eq!(
            model.last_error.as_deref(),
            Some("provider is not configured")
        );
    }

    #[test]
    fn permission_mode_change_updates_the_composer_projection() {
        let mut model = ShellModel::starting(None);
        model.apply_events(&[ClientEvent::PermissionModeChanged {
            mode: "permissive".into(),
        }]);

        assert_eq!(model.ui.permission_mode.as_deref(), Some("permissive"));
    }

    #[test]
    fn disconnected_state_surfaces_a_recoverable_error() {
        let mut model = ShellModel::starting(None);
        model.apply_events(&[ClientEvent::ConnectionChanged(
            ConnectionStatus::Disconnected,
        )]);

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

        model.sync_client_metadata_while_attaching(
            &ClientState {
                attached_session: Some("session-a".into()),
                ..ClientState::default()
            },
            "session-b",
        );

        assert_eq!(model.attached_session, None);
        assert!(model.ui.pending_actions.is_empty());
        assert!(model.ui.transcript.is_empty());
        assert!(!model.session_is_ready());
    }
}
