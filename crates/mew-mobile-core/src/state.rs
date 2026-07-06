//! Session state assembly for the mobile core.
//!
//! Ports the web store's part-assembly logic: Provider events → parts →
//! messages, tool call states, pending requests, session usage.
//! `PartUpdated` is authoritative — when it arrives for a part built from
//! accumulated deltas, replace the accumulated state wholesale.

/// A snapshot of a daemon's state — the full mirror that Swift pulls
/// after reconnect or attach instead of replaying deltas.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct DaemonSnapshot {
    pub sessions: Vec<SessionInfo>,
    pub attached_session: Option<String>,
    pub pending_permissions: Vec<PendingPermission>,
    pub pending_ask_user: Vec<PendingAskUser>,
    pub models: Vec<ModelInfo>,
    pub daemon_version: Option<String>,
}

/// A session in the snapshot.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SessionInfo {
    pub session_id: String,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    pub running: bool,
    pub usage_cost: f64,
    pub pending_permissions: u32,
    pub pending_questions: u32,
}

/// A chat message in the snapshot.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub parts: Vec<MessagePart>,
}

/// A part of a message.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MessagePart {
    pub id: String,
    pub kind: PartKind,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_state: Option<String>,
    pub tool_input: Option<String>,
    pub tool_output: Option<String>,
    pub tool_error: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_time_start: Option<i64>,
    pub tool_time_end: Option<i64>,
    /// Sensitivity tier ("ReadOnly", "Mutating", "Dangerous") stamped by the
    /// agent from the tool registry. None for non-tool parts or old sessions.
    pub tool_sensitivity: Option<String>,
}

/// What kind of part this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PartKind {
    Text,
    Reasoning,
    ToolCall,
    Error,
}

/// A pending permission request.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PendingPermission {
    pub request_id: u64,
    pub session_id: String,
    pub tool_name: String,
    pub input: String,
}

/// A pending ask-user request.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PendingAskUser {
    pub request_id: u64,
    pub session_id: String,
    pub call_id: String,
    pub questions: Vec<String>,
}

/// Model info in the snapshot.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub context_window: Option<i64>,
}

/// Per-session state tracker. Assembles parts from provider events.
pub struct SessionState {
    pub session_id: String,
    pub messages: Vec<ChatMessage>,
    pub running: bool,
    pub usage_cost: f64,
    pub pending_permissions: u32,
    pub pending_questions: u32,
    /// The currently streaming text part ID and accumulated text.
    pub streaming_part_id: Option<String>,
    pub streaming_text: String,
    /// Dedup: the last prompt text we sent, to drop the echoed UserMessage.
    pub last_sent_prompt: Option<String>,
}

impl SessionState {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            messages: Vec::new(),
            running: false,
            usage_cost: 0.0,
            pending_permissions: 0,
            pending_questions: 0,
            streaming_part_id: None,
            streaming_text: String::new(),
            last_sent_prompt: None,
        }
    }

    /// Apply a provider event wire to the session state.
    /// Returns true if the event was consumed.
    pub fn apply_provider_event(&mut self, event: &mew_message::ProviderEventWire) -> bool {
        use mew_message::{Part, ProviderEventWire};
        match event {
            ProviderEventWire::PartStart { part } => {
                // A new part means a turn is in progress.
                self.running = true;
                let (part_id, msg_part) = match part {
                    Part::Text(tp) => {
                        let id = tp.base.id.to_string();
                        self.streaming_part_id = Some(id.clone());
                        self.streaming_text = tp.text.clone();
                        (
                            id,
                            MessagePart {
                                id: tp.base.id.to_string(),
                                kind: PartKind::Text,
                                text: Some(tp.text.clone()),
                                tool_name: None,
                                tool_state: None,
                                tool_input: None,
                                tool_output: None,
                                tool_error: None,
                                tool_call_id: None,
                                tool_time_start: None,
                                tool_time_end: None,
                                tool_sensitivity: None,
                            },
                        )
                    }
                    Part::Reasoning(rp) => (
                        rp.base.id.to_string(),
                        MessagePart {
                            id: rp.base.id.to_string(),
                            kind: PartKind::Reasoning,
                            text: Some(rp.text.clone()),
                            tool_name: None,
                            tool_state: None,
                            tool_input: None,
                            tool_output: None,
                            tool_error: None,
                            tool_call_id: None,
                            tool_time_start: None,
                            tool_time_end: None,
                            tool_sensitivity: None,
                        },
                    ),
                    Part::ToolCall(tcp) => {
                        let (input_str, output, error, time_start, time_end) =
                            tool_state_fields(&tcp.state);
                        (
                            tcp.base.id.to_string(),
                            MessagePart {
                                id: tcp.base.id.to_string(),
                                kind: PartKind::ToolCall,
                                text: None,
                                tool_name: Some(tcp.tool_name.clone()),
                                tool_state: Some(format!("{:?}", tcp.state).to_lowercase()),
                                tool_input: input_str,
                                tool_output: output,
                                tool_error: error,
                                tool_call_id: Some(tcp.call_id.clone()),
                                tool_time_start: time_start,
                                tool_time_end: time_end,
                                tool_sensitivity: None,
                            },
                        )
                    }
                    _ => return false,
                };

                // Ensure there's a current assistant message.
                if self.messages.is_empty()
                    || self.messages.last().map(|m| m.role.as_str()) != Some("assistant")
                {
                    self.messages.push(ChatMessage {
                        id: simple_uuid(),
                        role: "assistant".into(),
                        parts: vec![],
                    });
                }
                self.messages.last_mut().unwrap().parts.push(msg_part);
                let _ = part_id;
                true
            }
            ProviderEventWire::PartDelta { part_id, delta, .. } => {
                let pid_str = part_id.to_string();
                if self.streaming_part_id.as_deref() == Some(&pid_str) {
                    self.streaming_text.push_str(delta);
                    if let Some(msg) = self.messages.last_mut() {
                        if let Some(part) = msg.parts.iter_mut().find(|p| p.id == pid_str) {
                            part.text = Some(self.streaming_text.clone());
                        }
                    }
                    true
                } else {
                    false
                }
            }
            ProviderEventWire::PartEnd { part_id } => {
                let pid_str = part_id.to_string();
                if self.streaming_part_id.as_deref() == Some(&pid_str) {
                    self.streaming_part_id = None;
                    self.streaming_text.clear();
                }
                true
            }
            ProviderEventWire::MessageEnd { cost, .. } => {
                self.running = false;
                self.usage_cost += cost;
                self.streaming_part_id = None;
                self.streaming_text.clear();
                true
            }
            _ => false,
        }
    }
}

/// Extracted fields from a `ToolState`.
type ToolFields = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// Extract input/output/error/time fields from a `ToolState`.
pub fn tool_state_fields(state: &mew_message::ToolState) -> ToolFields {
    use mew_message::ToolState;
    match state {
        ToolState::Pending(s) => {
            let input = serde_json::to_string_pretty(&s.input).ok();
            (input, None, None, Some(s.time.start), s.time.end)
        }
        ToolState::Running(s) => {
            let input = serde_json::to_string_pretty(&s.input).ok();
            let output = if s.output.is_empty() {
                None
            } else {
                Some(s.output.clone())
            };
            (input, output, None, Some(s.time.start), s.time.end)
        }
        ToolState::Completed(s) => {
            let input = serde_json::to_string_pretty(&s.input).ok();
            let output = if s.output.is_empty() {
                None
            } else {
                Some(s.output.clone())
            };
            (input, output, None, Some(s.time.start), s.time.end)
        }
        ToolState::Error(s) => {
            let input = serde_json::to_string_pretty(&s.input).ok();
            let error = if s.error.is_empty() {
                None
            } else {
                Some(s.error.clone())
            };
            (input, None, error, Some(s.time.start), s.time.end)
        }
    }
}

fn simple_uuid() -> String {
    ulid::Ulid::new().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_message::{PartBase, TextPart};
    use ulid::Ulid;

    #[test]
    fn test_session_state_text_streaming() {
        let mut state = SessionState::new("sess1".into());

        let part_id = Ulid::new();
        let text_part = TextPart {
            base: PartBase {
                id: part_id,
                message_id: Ulid::new(),
                session_id: Ulid::new(),
            },
            text: "Hello".into(),
            synthetic: false,
        };

        state.apply_provider_event(&mew_message::ProviderEventWire::PartStart {
            part: mew_message::Part::Text(text_part),
        });

        assert_eq!(state.messages.len(), 1);
        assert!(state.streaming_part_id.is_some());
        assert_eq!(state.streaming_text, "Hello");

        state.apply_provider_event(&mew_message::ProviderEventWire::PartDelta {
            part_id,
            field: "text".into(),
            delta: " world".into(),
        });
        assert_eq!(state.streaming_text, "Hello world");

        state.apply_provider_event(&mew_message::ProviderEventWire::PartEnd { part_id });
        assert!(state.streaming_part_id.is_none());
    }

    #[test]
    fn test_message_end_accumulates_cost() {
        let mut state = SessionState::new("sess1".into());
        assert_eq!(state.usage_cost, 0.0);

        state.apply_provider_event(&mew_message::ProviderEventWire::MessageEnd {
            finish: mew_message::Finish::Stop,
            usage: mew_message::Tokens::default(),
            cost: 0.0042,
        });
        assert_eq!(state.usage_cost, 0.0042);
    }
}
