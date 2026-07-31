//! Deterministic daemon-message reducer used by platform clients.

use mew_message::{AssistantMeta, Message, Part, PartId, ProviderEventWire, Role, Time};
use mew_protocol::{
    ClientMessage, ModelInfo, PermissionDecision, Question, RemoteScope, ServerMessage,
    SessionInfo, SessionUsageWire,
};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Backoff { attempt: u32, error: String },
}

#[derive(Debug, Clone)]
pub struct ClientSession {
    pub session_id: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub permission_mode: Option<String>,
    pub messages: Vec<Message>,
    pub running: bool,
    pub usage: SessionUsageWire,
    pub pending_actions: Vec<PendingAction>,
    pub last_sent_prompt: Option<String>,
    pub streaming_part_id: Option<PartId>,
    pub streaming_text: String,
}

impl ClientSession {
    fn new(session_id: String) -> Self {
        Self {
            session_id,
            cwd: None,
            model: None,
            provider: None,
            permission_mode: None,
            messages: Vec::new(),
            running: false,
            usage: SessionUsageWire::default(),
            pending_actions: Vec::new(),
            last_sent_prompt: None,
            streaming_part_id: None,
            streaming_text: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ActionKind {
    Permission {
        tool_name: String,
        input: serde_json::Value,
    },
    WorkspacePermission {
        path: String,
    },
    AskUser {
        call_id: String,
        questions: Vec<Question>,
    },
    PlanApproval {
        call_id: String,
        plan_path: String,
        plan_markdown: String,
        persona: String,
    },
    GoalApproval {
        call_id: String,
        objective: String,
    },
    SubagentPermission {
        parent_call_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone)]
pub struct PendingAction {
    pub request_id: String,
    pub kind: ActionKind,
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    ConnectionChanged(ConnectionStatus),
    RemoteReady(RemoteScope),
    SessionReady {
        session_id: String,
    },
    SessionListChanged,
    SessionHistoryLoaded {
        session_id: String,
    },
    MessageChanged {
        session_id: String,
    },
    TextDelta {
        session_id: String,
        part_id: PartId,
        delta: String,
    },
    TurnEnded {
        session_id: String,
        cost: f64,
        input_tokens: u32,
        output_tokens: u32,
    },
    RequiredActionChanged {
        session_id: String,
        request_id: String,
    },
    RequestResolved {
        request_id: String,
    },
    Error(String),
}

#[derive(Debug, Default)]
pub struct ClientState {
    pub connection: Option<ConnectionStatus>,
    pub remote_scope: Option<RemoteScope>,
    pub daemon_version: Option<String>,
    pub attached_session: Option<String>,
    pub sessions: BTreeMap<String, ClientSession>,
    pub session_list: Vec<SessionInfo>,
    pub models: Vec<ModelInfo>,
    pub current_model: Option<String>,
    pub current_provider: Option<String>,
    pub thinking_variant: Option<String>,
    pub permission_mode: Option<String>,
    pub current_persona: Option<String>,
}

impl ClientState {
    pub fn set_connection_status(&mut self, status: ConnectionStatus) -> ClientEvent {
        self.connection = Some(status.clone());
        ClientEvent::ConnectionChanged(status)
    }

    pub fn session(&self, session_id: &str) -> Option<&ClientSession> {
        self.sessions.get(session_id)
    }

    pub fn session_mut(&mut self, session_id: &str) -> &mut ClientSession {
        self.sessions
            .entry(session_id.to_string())
            .or_insert_with(|| ClientSession::new(session_id.to_string()))
    }

    /// Record a prompt before sending it. A later UserMessage echo is matched
    /// and consumed instead of duplicating the message in the transcript.
    pub fn record_prompt(&mut self, session_id: &str, text: String) {
        let session = self.session_mut(session_id);
        session.last_sent_prompt = Some(text.clone());
        let session_id_ulid = Ulid::from_string(session_id).unwrap_or_else(|_| Ulid::new());
        let message_id = Ulid::new();
        let part_id = Ulid::new();
        session.messages.push(Message {
            id: message_id,
            session_id: session_id_ulid,
            role: Role::User,
            parts: vec![Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: part_id,
                    message_id,
                    session_id: session_id_ulid,
                },
                text,
                synthetic: false,
            })],
            time: Time {
                created: now_millis(),
                completed: None,
            },
            assistant: None,
        });
    }

    pub fn apply_server_message(&mut self, message: ServerMessage) -> Vec<ClientEvent> {
        match message {
            ServerMessage::RemoteReady { scope } => {
                self.remote_scope = Some(scope);
                vec![ClientEvent::RemoteReady(scope)]
            }
            ServerMessage::Pong { version } => {
                self.daemon_version = Some(version);
                Vec::new()
            }
            ServerMessage::SessionReady {
                session_id,
                cwd,
                model,
                provider,
                permission_mode,
            } => {
                self.attached_session = Some(session_id.clone());
                let session = self.session_mut(&session_id);
                session.cwd = cwd;
                session.model = model.clone();
                session.provider = provider.clone();
                session.permission_mode = permission_mode.clone();
                self.current_model = model;
                self.current_provider = provider;
                self.permission_mode = permission_mode;
                vec![ClientEvent::SessionReady { session_id }]
            }
            ServerMessage::SessionList { sessions } => {
                self.session_list = sessions;
                vec![ClientEvent::SessionListChanged]
            }
            ServerMessage::SessionHistory { messages } => {
                let Some(session_id) = messages
                    .first()
                    .map(|message| message.session_id.to_string())
                    .or_else(|| self.attached_session.clone())
                else {
                    return Vec::new();
                };
                self.attached_session = Some(session_id.clone());
                self.session_mut(&session_id).messages = messages;
                vec![ClientEvent::SessionHistoryLoaded { session_id }]
            }
            ServerMessage::UserMessage { text } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                if session.last_sent_prompt.as_deref() == Some(text.as_str()) {
                    session.last_sent_prompt = None;
                    return Vec::new();
                }
                session.last_sent_prompt = None;
                self.record_prompt(&session_id, text);
                vec![ClientEvent::MessageChanged { session_id }]
            }
            ServerMessage::Provider { event } => self.apply_provider_event(event),
            ServerMessage::PartUpdated { part_id, part } => {
                let Some(session_id) = self.attached_session.clone() else {
                    return Vec::new();
                };
                let session = self.session_mut(&session_id);
                for message in &mut session.messages {
                    if let Some(existing) =
                        message.parts.iter_mut().find(|item| item.id() == part_id)
                    {
                        *existing = part;
                        return vec![ClientEvent::MessageChanged { session_id }];
                    }
                }
                Vec::new()
            }
            ServerMessage::PermissionRequest {
                request_id,
                tool_name,
                input,
            } => self.add_action(ActionKind::Permission { tool_name, input }, request_id),
            ServerMessage::WorkspacePermissionRequest { request_id, path } => {
                self.add_action(ActionKind::WorkspacePermission { path }, request_id)
            }
            ServerMessage::AskUserRequest {
                request_id,
                call_id,
                questions,
            } => self.add_action(ActionKind::AskUser { call_id, questions }, request_id),
            ServerMessage::PlanApprovalRequest {
                request_id,
                call_id,
                plan_path,
                plan_markdown,
                persona,
            } => self.add_action(
                ActionKind::PlanApproval {
                    call_id,
                    plan_path,
                    plan_markdown,
                    persona,
                },
                request_id,
            ),
            ServerMessage::GoalProposed {
                request_id,
                call_id,
                objective,
            } => self.add_action(ActionKind::GoalApproval { call_id, objective }, request_id),
            ServerMessage::SubagentPermissionRequest {
                request_id,
                parent_call_id,
                tool_name,
                input,
            } => self.add_action(
                ActionKind::SubagentPermission {
                    parent_call_id,
                    tool_name,
                    input,
                },
                request_id,
            ),
            ServerMessage::RequestResolved { request_id } => {
                for session in self.sessions.values_mut() {
                    session
                        .pending_actions
                        .retain(|action| action.request_id != request_id);
                }
                vec![ClientEvent::RequestResolved { request_id }]
            }
            ServerMessage::SessionCleared => {
                if let Some(session_id) = self.attached_session.clone() {
                    self.session_mut(&session_id).messages.clear();
                    return vec![ClientEvent::MessageChanged { session_id }];
                }
                Vec::new()
            }
            ServerMessage::SessionUsageChanged { session_id, usage } => {
                self.session_mut(&session_id).usage = usage;
                Vec::new()
            }
            ServerMessage::ModelList { models } => {
                self.models = models;
                Vec::new()
            }
            ServerMessage::ModelSwitched { provider, model } => {
                self.current_provider = Some(provider);
                self.current_model = Some(model);
                Vec::new()
            }
            ServerMessage::ThinkingVariantChanged { variant } => {
                self.thinking_variant = variant;
                Vec::new()
            }
            ServerMessage::PermissionModeChanged { mode } => {
                self.permission_mode = Some(mode);
                Vec::new()
            }
            ServerMessage::PersonaSwitched { name } => {
                self.current_persona = Some(name);
                Vec::new()
            }
            ServerMessage::Error { message } | ServerMessage::ErrorEvent { message } => {
                vec![ClientEvent::Error(message)]
            }
            _ => Vec::new(),
        }
    }

    fn add_action(&mut self, kind: ActionKind, request_id: String) -> Vec<ClientEvent> {
        let Some(session_id) = self.attached_session.clone() else {
            return Vec::new();
        };
        self.session_mut(&session_id)
            .pending_actions
            .push(PendingAction {
                request_id: request_id.clone(),
                kind,
            });
        vec![ClientEvent::RequiredActionChanged {
            session_id,
            request_id,
        }]
    }

    fn apply_provider_event(&mut self, event: ProviderEventWire) -> Vec<ClientEvent> {
        let Some(session_id) = self.attached_session.clone() else {
            return Vec::new();
        };
        let session = self.session_mut(&session_id);
        match event {
            ProviderEventWire::PartStart { part } => {
                session.running = true;
                let part_id = part.id();
                match &part {
                    Part::Text(part) => {
                        session.streaming_part_id = Some(part_id);
                        session.streaming_text = part.text.clone();
                    }
                    Part::Reasoning(part) => {
                        session.streaming_part_id = Some(part_id);
                        session.streaming_text = part.text.clone();
                    }
                    _ => {}
                }
                let message_id = match &part {
                    Part::Text(part) => part.base.message_id,
                    Part::Reasoning(part) => part.base.message_id,
                    Part::File(part) => part.base.message_id,
                    Part::ToolCall(part) => part.base.message_id,
                    Part::ToolResult(part) => part.base.message_id,
                    Part::Compaction(part) => part.base.message_id,
                };
                let has_message = session
                    .messages
                    .last()
                    .map(|message| message.id == message_id && message.role == Role::Assistant)
                    .unwrap_or(false);
                if !has_message {
                    session.messages.push(Message {
                        id: message_id,
                        session_id: Ulid::from_string(&session.session_id)
                            .unwrap_or_else(|_| Ulid::new()),
                        role: Role::Assistant,
                        parts: Vec::new(),
                        time: Time {
                            created: now_millis(),
                            completed: None,
                        },
                        assistant: None,
                    });
                }
                session
                    .messages
                    .last_mut()
                    .expect("assistant message")
                    .parts
                    .push(part);
                vec![ClientEvent::MessageChanged { session_id }]
            }
            ProviderEventWire::PartDelta { part_id, delta, .. } => {
                if session.streaming_part_id != Some(part_id) {
                    return Vec::new();
                }
                session.streaming_text.push_str(&delta);
                for message in session.messages.iter_mut().rev() {
                    if let Some(part) = message.parts.iter_mut().find(|part| part.id() == part_id) {
                        match part {
                            Part::Text(part) => part.text = session.streaming_text.clone(),
                            Part::Reasoning(part) => part.text = session.streaming_text.clone(),
                            _ => {}
                        }
                        break;
                    }
                }
                vec![ClientEvent::TextDelta {
                    session_id,
                    part_id,
                    delta,
                }]
            }
            ProviderEventWire::PartEnd { part_id } => {
                if session.streaming_part_id == Some(part_id) {
                    session.streaming_part_id = None;
                    session.streaming_text.clear();
                }
                Vec::new()
            }
            ProviderEventWire::MessageEnd {
                usage,
                cost,
                finish,
                manifest,
            } => {
                session.running = false;
                session.streaming_part_id = None;
                session.streaming_text.clear();
                session.usage.input_tokens += usage.input as u64;
                session.usage.output_tokens += usage.output as u64;
                session.usage.cost += cost;
                session.usage.turns += 1;
                if let Some(message) = session
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                {
                    let assistant = message.assistant.get_or_insert(AssistantMeta {
                        provider_id: session.provider.clone().unwrap_or_default(),
                        model_id: session.model.clone().unwrap_or_default(),
                        cost: 0.0,
                        tokens: usage,
                        finish: None,
                        error: None,
                        manifest: None,
                    });
                    assistant.cost += cost;
                    assistant.tokens = usage;
                    assistant.finish = Some(finish);
                    assistant.manifest = manifest;
                }
                vec![ClientEvent::TurnEnded {
                    session_id,
                    cost,
                    input_tokens: usage.input,
                    output_tokens: usage.output,
                }]
            }
            ProviderEventWire::Error(error) => vec![ClientEvent::Error(error.message)],
            ProviderEventWire::RetryWait { reason, .. } => vec![ClientEvent::Error(reason)],
        }
    }

    pub fn permission_response(request_id: String, decision: PermissionDecision) -> ClientMessage {
        ClientMessage::PermissionResponse {
            request_id,
            decision,
        }
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{Finish, PartBase, TextPart, Tokens};

    fn session_ready(state: &mut ClientState) {
        state.apply_server_message(ServerMessage::SessionReady {
            session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            cwd: Some("/tmp/project".into()),
            model: Some("gpt-test".into()),
            provider: Some("fake".into()),
            permission_mode: Some("standard".into()),
        });
    }

    #[test]
    fn prompt_echo_is_not_added_twice() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let id = state.attached_session.clone().unwrap();
        state.record_prompt(&id, "hello".into());
        let before = state.session(&id).unwrap().messages.len();

        let events = state.apply_server_message(ServerMessage::UserMessage {
            text: "hello".into(),
        });
        assert!(events.is_empty());
        assert_eq!(state.session(&id).unwrap().messages.len(), before);
    }

    #[test]
    fn provider_stream_updates_message_and_usage() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();
        let session_ulid = Ulid::from_string(&session_id).unwrap();
        let message_id = Ulid::new();
        let part_id = Ulid::new();
        let part = Part::Text(TextPart {
            base: PartBase {
                id: part_id,
                message_id,
                session_id: session_ulid,
            },
            text: "hello".into(),
            synthetic: false,
        });

        state.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::PartStart { part },
        });
        let events = state.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::PartDelta {
                part_id,
                field: "text".into(),
                delta: " world".into(),
            },
        });
        assert!(
            matches!(events.as_slice(), [ClientEvent::TextDelta { delta, .. }] if delta == " world")
        );
        state.apply_server_message(ServerMessage::Provider {
            event: ProviderEventWire::MessageEnd {
                finish: Finish::Stop,
                usage: Tokens {
                    input: 10,
                    output: 4,
                    ..Tokens::default()
                },
                cost: 0.25,
                manifest: None,
            },
        });

        let session = state.session(&session_id).unwrap();
        assert_eq!(session.messages.len(), 1);
        assert!(
            matches!(&session.messages[0].parts[0], Part::Text(part) if part.text == "hello world")
        );
        assert_eq!(session.usage.input_tokens, 10);
        assert_eq!(session.usage.output_tokens, 4);
        assert_eq!(session.usage.turns, 1);
        assert!(!session.running);
    }

    #[test]
    fn required_actions_are_stored_and_resolved() {
        let mut state = ClientState::default();
        session_ready(&mut state);
        let session_id = state.attached_session.clone().unwrap();
        let events = state.apply_server_message(ServerMessage::PermissionRequest {
            request_id: "request-1".into(),
            tool_name: "shell".into(),
            input: serde_json::json!({"command": "pwd"}),
        });
        assert!(
            matches!(events.as_slice(), [ClientEvent::RequiredActionChanged { request_id, .. }] if request_id == "request-1")
        );
        assert_eq!(state.session(&session_id).unwrap().pending_actions.len(), 1);

        state.apply_server_message(ServerMessage::RequestResolved {
            request_id: "request-1".into(),
        });
        assert!(state
            .session(&session_id)
            .unwrap()
            .pending_actions
            .is_empty());
    }
}
