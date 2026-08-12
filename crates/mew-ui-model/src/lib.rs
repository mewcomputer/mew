//! Framework-independent presentation state for native mew clients.

use mew_client_core::{
    BrowserState, ClientState, ConnectionStatus, PendingAction, PresenceEntry, SubagentEntry,
};
use mew_message::{Message, Part, Role, ToolState};
use mew_protocol::{
    ClientKind, ClientMessage, FlaggedFileWire, GroupInfo, ModelInfo, PersonaInfo, SessionInfo,
    SessionState, SessionUsageWire, Todo,
};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationItem {
    pub session_id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub last_message_at: Option<i64>,
    pub state: SessionState,
    pub last_turn_failed: bool,
    pub needs_attention: bool,
    pub archived: bool,
    pub pinned: bool,
    pub group_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptItem {
    pub role: TranscriptRole,
    pub text: String,
    pub parts: Vec<TranscriptPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptPart {
    Text(String),
    Reasoning(String),
    File(String),
    ToolCall {
        tool_name: String,
        call_id: String,
        status: ToolStatus,
        input: String,
        output: Option<String>,
        error: Option<String>,
        /// Unified diff produced by an edit/write tool, when the tool
        /// reported one.
        diff: Option<String>,
    },
    Compaction {
        auto: bool,
        summary: Option<String>,
        removed_count: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Running,
    Completed,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewSummary {
    pub files: Vec<String>,
    pub added: u64,
    pub removed: u64,
}

/// Presentation status for one todo item, parsed from the wire string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityTodoStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
    Unknown,
}

/// One todo item in the selected session's activity list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityTodo {
    pub id: usize,
    pub content: String,
    pub status: ActivityTodoStatus,
}

/// Accumulated token and cost usage for the selected session.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
    pub turns: u32,
}

impl UsageSummary {
    pub fn is_empty(&self) -> bool {
        self.turns == 0 && self.input_tokens == 0 && self.output_tokens == 0
    }
}

impl From<&Todo> for ActivityTodo {
    fn from(todo: &Todo) -> Self {
        Self {
            id: todo.id,
            content: todo.content.clone(),
            status: match todo.status.as_str() {
                "pending" => ActivityTodoStatus::Pending,
                "in_progress" => ActivityTodoStatus::InProgress,
                "done" => ActivityTodoStatus::Done,
                "blocked" => ActivityTodoStatus::Blocked,
                _ => ActivityTodoStatus::Unknown,
            },
        }
    }
}

impl From<&SessionUsageWire> for UsageSummary {
    fn from(usage: &SessionUsageWire) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost: usage.cost,
            turns: usage.turns,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiModel {
    pub connection: Option<ConnectionStatus>,
    pub conversations: Vec<ConversationItem>,
    pub groups: Vec<GroupInfo>,
    pub selected_session: Option<String>,
    pub transcript: Vec<TranscriptItem>,
    pub models: Vec<ModelInfo>,
    pub composer: String,
    pub current_provider: Option<String>,
    pub model: Option<String>,
    pub thinking_variant: Option<String>,
    pub current_persona: Option<String>,
    pub personas: Vec<PersonaInfo>,
    pub permission_mode: Option<String>,
    pub running: bool,
    pub pending_actions: Vec<PendingAction>,
    pub workbench_open: bool,
    pub review: ReviewSummary,
    pub flagged_files: Vec<FlaggedFileWire>,
    pub browser: BrowserState,
    pub subagents: Vec<SubagentEntry>,
    pub todos: Vec<ActivityTodo>,
    pub usage: Option<UsageSummary>,
    pub presence: Vec<PresenceEntry>,
    pub control_yielded_by: Option<u64>,
}

impl Default for UiModel {
    fn default() -> Self {
        Self {
            connection: None,
            conversations: Vec::new(),
            groups: Vec::new(),
            selected_session: None,
            transcript: Vec::new(),
            models: Vec::new(),
            composer: String::new(),
            current_provider: None,
            model: None,
            thinking_variant: None,
            current_persona: None,
            personas: Vec::new(),
            permission_mode: None,
            running: false,
            pending_actions: Vec::new(),
            workbench_open: true,
            review: ReviewSummary::default(),
            flagged_files: Vec::new(),
            browser: BrowserState::default(),
            subagents: Vec::new(),
            todos: Vec::new(),
            usage: None,
            presence: Vec::new(),
            control_yielded_by: None,
        }
    }
}

impl UiModel {
    pub fn sync_from_client(&mut self, state: &ClientState) {
        self.sync_client_metadata(state);
        self.sync_client_transcript(state);
    }

    pub fn sync_client_metadata(&mut self, state: &ClientState) {
        self.connection = state.connection.clone();
        self.current_provider = state.current_provider.clone();
        self.model = state.current_model.clone();
        self.thinking_variant = state.thinking_variant.clone();
        self.current_persona = state.current_persona.clone();
        self.personas = state.personas.clone();
        self.permission_mode = state.permission_mode.clone();
        self.running = state
            .attached_session
            .as_deref()
            .and_then(|session_id| state.session(session_id))
            .is_some_and(|session| session.running);
        self.models = state.models.clone();
        self.pending_actions = state
            .attached_session
            .as_deref()
            .and_then(|session_id| state.session(session_id))
            .map(|session| session.pending_actions.clone())
            .unwrap_or_default();
        self.browser = state.browser.clone();
        self.presence = state.presence.clone();
        self.control_yielded_by = state.control_yielded_by;
        // Metadata-only snapshots omit session state entirely, so keep the last
        // known activity projection unless the attached session is present.
        if let Some(session) = state
            .attached_session
            .as_deref()
            .and_then(|session_id| state.session(session_id))
        {
            self.subagents = session.subagents.clone();
            self.todos = session.todos.iter().map(ActivityTodo::from).collect();
            self.usage = state.usage(&session.session_id).map(UsageSummary::from);
            self.flagged_files = session.flagged_files.clone();
        } else {
            self.flagged_files.clear();
        }
        self.conversations = state
            .session_list
            .iter()
            .map(|session| {
                let mut conversation = ConversationItem::from(session);
                if let Some(title) = state.session_titles.get(&session.session_id) {
                    conversation.title = title.clone();
                }
                conversation
            })
            .collect();
        self.groups = state.groups.clone();

        if let Some(attached_session) = &state.attached_session {
            self.selected_session = Some(attached_session.clone());
        } else if self.selected_session.as_ref().is_some_and(|selected| {
            !state
                .session_list
                .iter()
                .any(|item| item.session_id == *selected)
        }) {
            self.selected_session = None;
        }

        self.review = self
            .selected_session
            .as_deref()
            .and_then(|selected| {
                state
                    .session_list
                    .iter()
                    .find(|session| session.session_id == selected)
            })
            .and_then(|session| session.change_stats.as_ref())
            .map(|stats| ReviewSummary {
                files: stats.files.clone(),
                added: stats.added,
                removed: stats.removed,
            })
            .unwrap_or_default();
    }

    pub fn sync_client_transcript(&mut self, state: &ClientState) {
        self.transcript = self
            .selected_session
            .as_deref()
            .and_then(|session_id| state.session(session_id))
            .map(|session| {
                session
                    .messages
                    .iter()
                    .filter_map(TranscriptItem::from_message)
                    .collect()
            })
            .unwrap_or_default();
    }

    pub fn append_transcript_delta(&mut self, session_id: &str, delta: &str) -> bool {
        if self.selected_session.as_deref() != Some(session_id) {
            return false;
        }
        let Some(item) = self
            .transcript
            .iter_mut()
            .rev()
            .find(|item| item.role == TranscriptRole::Assistant)
        else {
            return false;
        };
        item.text.push_str(delta);
        if let Some(part) = item
            .parts
            .iter_mut()
            .rev()
            .find(|part| matches!(part, TranscriptPart::Text(_) | TranscriptPart::Reasoning(_)))
        {
            let (TranscriptPart::Text(text) | TranscriptPart::Reasoning(text)) = part else {
                return true;
            };
            text.push_str(delta);
        }
        true
    }

    pub fn append_tool_progress(&mut self, session_id: &str, call_id: &str, chunk: &str) -> bool {
        if self.selected_session.as_deref() != Some(session_id) {
            return false;
        }
        let Some(TranscriptPart::ToolCall {
            call_id: existing_call_id,
            output,
            status: ToolStatus::Running,
            ..
        }) = self
            .transcript
            .iter_mut()
            .rev()
            .flat_map(|item| item.parts.iter_mut().rev())
            .find(|part| {
                matches!(
                    part,
                    TranscriptPart::ToolCall { call_id: existing_call_id, .. }
                        if existing_call_id == call_id
                )
            })
        else {
            return false;
        };
        debug_assert_eq!(existing_call_id, call_id);
        output.get_or_insert_with(String::new).push_str(chunk);
        true
    }

    pub fn select_session(&mut self, session_id: &str) -> Option<ClientMessage> {
        if !self
            .conversations
            .iter()
            .any(|conversation| conversation.session_id == session_id && !conversation.archived)
        {
            return None;
        }
        self.selected_session = Some(session_id.to_owned());
        self.clear_activity();
        Some(ClientMessage::AttachSession {
            session_id: session_id.to_owned(),
            client_kind: ClientKind::Desktop,
        })
    }

    pub fn new_conversation(&mut self, cwd: Option<String>) -> ClientMessage {
        self.selected_session = None;
        self.transcript.clear();
        self.clear_activity();
        ClientMessage::NewSession {
            cwd,
            client_kind: ClientKind::Desktop,
        }
    }

    /// Clear state that belongs to the previously attached session while a
    /// new session is still being hydrated.
    pub fn clear_attached_session_projection(&mut self) {
        self.transcript.clear();
        self.pending_actions.clear();
        self.running = false;
        self.clear_activity();
        self.review = ReviewSummary::default();
        self.flagged_files.clear();
        self.browser = BrowserState::default();
        self.presence.clear();
        self.control_yielded_by = None;
    }

    pub fn new_conversation_in_group(
        &mut self,
        cwd: Option<String>,
        group_id: String,
    ) -> ClientMessage {
        self.selected_session = None;
        self.transcript.clear();
        self.clear_activity();
        ClientMessage::NewSessionInGroup {
            cwd,
            group_id,
            client_kind: ClientKind::Desktop,
        }
    }

    pub fn set_composer(&mut self, text: impl Into<String>) {
        self.composer = text.into();
    }

    fn clear_activity(&mut self) {
        self.subagents.clear();
        self.todos.clear();
        self.usage = None;
    }

    pub fn take_prompt(&mut self) -> Option<String> {
        if self.composer.trim().is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.composer))
    }
}

impl From<&SessionInfo> for ConversationItem {
    fn from(session: &SessionInfo) -> Self {
        let title = session
            .summary
            .as_deref()
            .or(session.first_message.as_deref())
            .or(session
                .cwd
                .as_deref()
                .and_then(|cwd| Path::new(cwd).file_name().and_then(|name| name.to_str())))
            .unwrap_or("New conversation")
            .to_owned();
        Self {
            session_id: session.session_id.clone(),
            title,
            cwd: session.cwd.clone(),
            last_message_at: session.last_message_at,
            state: session.state,
            last_turn_failed: session.last_turn_failed,
            needs_attention: session.pending_permissions > 0 || session.pending_questions > 0,
            archived: session.archived,
            pinned: session.pinned,
            group_id: session.group_id.clone(),
        }
    }
}

impl TranscriptItem {
    fn from_message(message: &Message) -> Option<Self> {
        let mut text = String::new();
        let mut parts = Vec::new();
        for part in &message.parts {
            if Self::is_internal_system_notice(message, part) {
                continue;
            }
            let (part_text, projected): (Option<String>, Option<TranscriptPart>) = match part {
                Part::Text(part) => (
                    Some(part.text.clone()),
                    Some(TranscriptPart::Text(part.text.clone())),
                ),
                Part::Reasoning(part) => (
                    Some(part.text.clone()),
                    Some(TranscriptPart::Reasoning(part.text.clone())),
                ),
                Part::File(part) => {
                    let label = part.filename.clone().unwrap_or_else(|| part.url.clone());
                    (Some(label.clone()), Some(TranscriptPart::File(label)))
                }
                Part::ToolCall(part) => {
                    let (status, output, error, diff) = match &part.state {
                        ToolState::Pending(_) => (ToolStatus::Pending, None, None, None),
                        ToolState::Running(state) => {
                            (ToolStatus::Running, Some(state.output.clone()), None, None)
                        }
                        ToolState::Completed(state) => (
                            ToolStatus::Completed,
                            Some(state.output.clone()),
                            None,
                            state.diff.clone(),
                        ),
                        ToolState::Error(state) => {
                            (ToolStatus::Error, None, Some(state.error.clone()), None)
                        }
                    };
                    (
                        Some(part.tool_name.clone()),
                        Some(TranscriptPart::ToolCall {
                            tool_name: part.tool_name.clone(),
                            call_id: part.call_id.clone(),
                            status,
                            input: part.state.input().to_string(),
                            output,
                            error,
                            diff,
                        }),
                    )
                }
                Part::ToolResult(_) => (None, None),
                Part::Compaction(part) => (
                    Some("context compacted".into()),
                    Some(TranscriptPart::Compaction {
                        auto: part.auto,
                        summary: part.summary.clone(),
                        removed_count: part.removed_count,
                    }),
                ),
            };
            if let Some(projected) = projected {
                parts.push(projected);
            }
            if let Some(part_text) = part_text {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&part_text);
            }
        }
        (!text.is_empty() || !parts.is_empty()).then_some(Self {
            role: match message.role {
                Role::User => TranscriptRole::User,
                Role::Assistant => TranscriptRole::Assistant,
                Role::System => TranscriptRole::System,
            },
            text,
            parts,
        })
    }

    fn is_internal_system_notice(message: &Message, part: &Part) -> bool {
        if message.role != Role::System {
            return false;
        }
        let Part::Text(part) = part else {
            return false;
        };
        [
            "[capability:desktop_browser_v1]",
            "[capability:desktopbrowser:v1]",
        ]
        .iter()
        .any(|prefix| part.text.starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_client_core::{ActionKind, ConnectionStatus};
    use mew_message::{
        Message, Part, PartBase, ReasoningPart, TextPart, Time, ToolCallPart, ToolState,
        ToolStateCompleted, ToolTime,
    };
    use mew_protocol::{GroupInfo, PersonaInfo, ServerMessage, SessionInfo, SessionState};
    use ulid::Ulid;

    const SESSION_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

    fn session_info() -> SessionInfo {
        SessionInfo {
            session_id: SESSION_ID.into(),
            state: SessionState::Idle,
            model: None,
            provider: None,
            created_at: 0,
            last_message_at: None,
            summary: None,
            client_count: 0,
            cwd: Some("/tmp/mew-project".into()),
            last_turn_failed: false,
            archived: false,
            pinned: false,
            group_id: None,
            change_stats: None,
            usage: None,
            context_tokens: None,
            pending_permissions: 0,
            pending_questions: 0,
            first_message: Some("first prompt".into()),
        }
    }

    #[test]
    fn projects_stable_navigation_and_transcript_from_client_state() {
        let mut state = ClientState {
            connection: Some(ConnectionStatus::Connected),
            session_list: vec![session_info()],
            attached_session: Some(SESSION_ID.into()),
            current_provider: Some("fake".into()),
            current_model: Some("model".into()),
            current_persona: Some("reviewer".into()),
            personas: vec![PersonaInfo {
                name: "reviewer".into(),
                description: "Reviews changes".into(),
                color: None,
                active: true,
            }],
            models: vec![ModelInfo {
                id: "fake/model".into(),
                provider: "fake".into(),
                model: "model".into(),
                description: Some("Test model".into()),
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            }],
            ..ClientState::default()
        };
        let session = state.session_mut(SESSION_ID);
        let message_id = Ulid::new();
        let session_id = Ulid::from_string(SESSION_ID).unwrap();
        session.messages.push(Message {
            id: message_id,
            session_id,
            role: Role::User,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id,
                    session_id,
                },
                text: "hello".into(),
                synthetic: false,
            })],
            time: Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        });

        let mut model = UiModel::default();
        model.sync_from_client(&state);
        assert_eq!(model.selected_session.as_deref(), Some(SESSION_ID));
        assert_eq!(model.conversations[0].title, "first prompt");
        assert_eq!(model.conversations[0].group_id, None);
        assert_eq!(
            model.conversations[0].cwd.as_deref(),
            Some("/tmp/mew-project")
        );
        assert_eq!(model.transcript[0].text, "hello");
        assert_eq!(model.connection, Some(ConnectionStatus::Connected));
        assert_eq!(model.models[0].id, "fake/model");
        assert_eq!(model.current_provider.as_deref(), Some("fake"));
        assert_eq!(model.model.as_deref(), Some("model"));
        assert_eq!(model.current_persona.as_deref(), Some("reviewer"));
        assert_eq!(model.personas[0].name, "reviewer");
        assert!(!model.running);

        state.groups.push(GroupInfo {
            id: "grp_1".into(),
            name: "Project work".into(),
            color: None,
            order: 0,
        });
        state.session_list[0].group_id = Some("grp_1".into());
        state.session_list[0].last_turn_failed = true;
        model.sync_client_metadata(&state);
        assert_eq!(model.groups[0].name, "Project work");
        assert_eq!(model.conversations[0].group_id.as_deref(), Some("grp_1"));
        assert!(model.conversations[0].last_turn_failed);

        state.session_mut(SESSION_ID).running = true;
        model.sync_client_metadata(&state);
        assert!(model.running);
    }

    #[test]
    fn projects_reasoning_and_tool_parts_for_native_rendering() {
        let mut state = ClientState {
            attached_session: Some(SESSION_ID.into()),
            ..ClientState::default()
        };
        let session_id = Ulid::from_string(SESSION_ID).unwrap();
        let message_id = Ulid::new();
        let session = state.session_mut(SESSION_ID);
        session.messages.push(Message {
            id: message_id,
            session_id,
            role: Role::Assistant,
            parts: vec![
                Part::Reasoning(ReasoningPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id,
                        session_id,
                    },
                    text: "inspect the workspace".into(),
                    signature: None,
                    encrypted_content: None,
                }),
                Part::ToolCall(ToolCallPart {
                    base: PartBase {
                        id: Ulid::new(),
                        message_id,
                        session_id,
                    },
                    tool_name: "shell".into(),
                    call_id: "call-1".into(),
                    state: ToolState::Completed(ToolStateCompleted {
                        input: serde_json::json!({"command": "pwd"}),
                        output: "/tmp/mew".into(),
                        metadata: None,
                        diff: None,
                        images: vec![],
                        time: ToolTime {
                            start: 0,
                            end: Some(1),
                        },
                    }),
                    raw_input: String::new(),
                }),
            ],
            time: Time {
                created: 0,
                completed: Some(1),
            },
            assistant: None,
        });

        let mut model = UiModel::default();
        model.sync_from_client(&state);
        assert!(matches!(
            &model.transcript[0].parts[0],
            TranscriptPart::Reasoning(text) if text == "inspect the workspace"
        ));
        assert!(matches!(
            &model.transcript[0].parts[1],
            TranscriptPart::ToolCall {
                tool_name,
                status: ToolStatus::Completed,
                output: Some(output),
                ..
            } if tool_name == "shell" && output == "/tmp/mew"
        ));
    }

    #[test]
    fn hides_synthetic_desktop_capability_notices_from_transcript() {
        let mut state = ClientState {
            attached_session: Some(SESSION_ID.into()),
            ..ClientState::default()
        };
        let session_id = Ulid::from_string(SESSION_ID).unwrap();
        let message_id = Ulid::new();
        state.session_mut(SESSION_ID).messages.push(Message {
            id: message_id,
            session_id,
            role: Role::System,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id,
                    session_id,
                },
                text: "[capability:desktop_browser_v1] The desktop app is now attached.".into(),
                synthetic: true,
            })],
            time: Time {
                created: 0,
                completed: Some(0),
            },
            assistant: None,
        });

        let mut model = UiModel::default();
        model.sync_from_client(&state);

        assert!(model.transcript.is_empty());
    }

    #[test]
    fn navigation_and_composer_emit_safe_commands() {
        let mut model = UiModel {
            conversations: vec![ConversationItem::from(&session_info())],
            ..UiModel::default()
        };
        assert!(matches!(
            model.select_session(SESSION_ID),
            Some(ClientMessage::AttachSession { session_id, .. }) if session_id == SESSION_ID
        ));
        assert!(model.select_session("missing").is_none());

        assert!(model.take_prompt().is_none());
        model.set_composer("  hello  ");
        assert_eq!(model.take_prompt().as_deref(), Some("  hello  "));
        assert_eq!(model.composer, "");
        assert!(matches!(
            model.new_conversation(None),
            ClientMessage::NewSession {
                client_kind: ClientKind::Desktop,
                ..
            }
        ));
        assert!(matches!(
            model.new_conversation_in_group(None, "grp_1".into()),
            ClientMessage::NewSessionInGroup {
                group_id,
                client_kind: ClientKind::Desktop,
                ..
            } if group_id == "grp_1"
        ));
    }

    #[test]
    fn projects_pinned_flag_and_tool_diffs() {
        let mut pinned = session_info();
        pinned.pinned = true;
        let model = UiModel {
            conversations: vec![ConversationItem::from(&pinned)],
            ..UiModel::default()
        };
        assert!(model.conversations[0].pinned);

        let mut state = ClientState {
            attached_session: Some(SESSION_ID.into()),
            ..ClientState::default()
        };
        let session_id = Ulid::from_string(SESSION_ID).unwrap();
        let message_id = Ulid::new();
        state.session_mut(SESSION_ID).messages.push(Message {
            id: message_id,
            session_id,
            role: Role::Assistant,
            parts: vec![Part::ToolCall(ToolCallPart {
                base: PartBase {
                    id: Ulid::new(),
                    message_id,
                    session_id,
                },
                tool_name: "edit".into(),
                call_id: "call-1".into(),
                state: ToolState::Completed(ToolStateCompleted {
                    input: serde_json::json!({"path": "src/main.rs"}),
                    output: "edited".into(),
                    metadata: None,
                    diff: Some("@@ -1 +1 @@\n-old\n+new".into()),
                    images: vec![],
                    time: ToolTime {
                        start: 0,
                        end: Some(1),
                    },
                }),
                raw_input: String::new(),
            })],
            time: Time {
                created: 0,
                completed: Some(1),
            },
            assistant: None,
        });

        let mut model = UiModel::default();
        model.sync_from_client(&state);
        assert!(matches!(
            &model.transcript[0].parts[0],
            TranscriptPart::ToolCall { diff: Some(diff), .. } if diff.contains("+new")
        ));
    }

    #[test]
    fn archived_conversations_remain_in_projection_but_cannot_be_selected() {
        let mut archived = session_info();
        archived.archived = true;
        let mut model = UiModel {
            conversations: vec![ConversationItem::from(&archived)],
            ..UiModel::default()
        };
        assert!(model.select_session(SESSION_ID).is_none());
        assert!(model.conversations[0].archived);
    }

    #[test]
    fn streaming_delta_updates_only_the_selected_assistant_message() {
        let mut model = UiModel {
            selected_session: Some(SESSION_ID.into()),
            transcript: vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "hello".into(),
                parts: vec![TranscriptPart::Text("hello".into())],
            }],
            ..UiModel::default()
        };

        assert!(model.append_transcript_delta(SESSION_ID, " world"));
        assert_eq!(model.transcript[0].text, "hello world");
        assert!(!model.append_transcript_delta("other", "ignored"));
    }

    #[test]
    fn streaming_tool_progress_updates_only_the_matching_call() {
        let mut model = UiModel {
            selected_session: Some(SESSION_ID.into()),
            transcript: vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "shell".into(),
                parts: vec![TranscriptPart::ToolCall {
                    tool_name: "shell".into(),
                    call_id: "call-1".into(),
                    status: ToolStatus::Running,
                    input: "{}".into(),
                    output: Some("before".into()),
                    error: None,
                    diff: None,
                }],
            }],
            ..UiModel::default()
        };

        assert!(model.append_tool_progress(SESSION_ID, "call-1", " after"));
        assert!(matches!(
            &model.transcript[0].parts[0],
            TranscriptPart::ToolCall { output: Some(output), .. } if output == "before after"
        ));
        assert!(!model.append_tool_progress(SESSION_ID, "missing", "ignored"));
        assert!(!model.append_tool_progress("other", "call-1", "ignored"));
    }

    #[test]
    fn projects_activity_presence_and_usage_from_client_state() {
        use mew_client_core::{PresenceEntry, SubagentEntry};

        let mut state = ClientState {
            attached_session: Some(SESSION_ID.into()),
            presence: vec![
                PresenceEntry {
                    client_id: 7,
                    client_kind: ClientKind::Desktop,
                },
                PresenceEntry {
                    client_id: 9,
                    client_kind: ClientKind::Tui,
                },
            ],
            control_yielded_by: Some(9),
            ..ClientState::default()
        };
        let session = state.session_mut(SESSION_ID);
        session.subagents.push(SubagentEntry {
            task_id: "task-1".into(),
            name: "explore".into(),
            display_name: Some("Explore the codebase".into()),
            status: "running".into(),
            tool_name: None,
        });
        session.todos = vec![
            Todo {
                id: 1,
                content: "map the reducer".into(),
                status: "done".into(),
                depends_on: Vec::new(),
            },
            Todo {
                id: 2,
                content: "wire the view".into(),
                status: "in_progress".into(),
                depends_on: Vec::new(),
            },
        ];
        session.usage = SessionUsageWire {
            input_tokens: 1200,
            output_tokens: 340,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: 0.0123,
            turns: 2,
        };

        let mut model = UiModel::default();
        model.sync_client_metadata(&state);
        assert_eq!(model.subagents.len(), 1);
        assert_eq!(
            model.subagents[0].display_name.as_deref(),
            Some("Explore the codebase")
        );
        assert_eq!(model.todos.len(), 2);
        assert_eq!(model.todos[0].status, ActivityTodoStatus::Done);
        assert_eq!(model.todos[1].status, ActivityTodoStatus::InProgress);
        assert_eq!(model.usage.map(|usage| usage.turns), Some(2));
        assert!(!model.usage.is_some_and(|usage| usage.is_empty()));
        assert_eq!(model.presence.len(), 2);
        assert_eq!(model.control_yielded_by, Some(9));

        // Metadata-only snapshots omit session state; the last known activity
        // projection is preserved instead of being cleared.
        let metadata = state.ui_metadata_snapshot();
        model.sync_client_metadata(&metadata);
        assert_eq!(model.subagents.len(), 1);
        assert_eq!(model.todos.len(), 2);
        assert_eq!(model.usage.map(|usage| usage.turns), Some(2));
        assert_eq!(model.presence.len(), 2);
        assert_eq!(model.control_yielded_by, Some(9));

        // Switching conversations drops the previous session's activity.
        model.conversations = vec![ConversationItem::from(&session_info())];
        assert!(model.select_session(SESSION_ID).is_some());
        assert!(model.subagents.is_empty());
        assert!(model.todos.is_empty());
        assert!(model.usage.is_none());
    }

    #[test]
    fn projects_permission_mode_from_client_state() {
        let mut state = ClientState::default();
        state.apply_server_message(ServerMessage::SessionReady {
            session_id: SESSION_ID.into(),
            cwd: Some("/tmp/mew-project".into()),
            model: Some("model".into()),
            provider: Some("provider".into()),
            permission_mode: Some("standard".into()),
        });

        let mut model = UiModel::default();
        model.sync_client_metadata(&state);
        assert_eq!(model.permission_mode.as_deref(), Some("standard"));

        state.apply_server_message(ServerMessage::PermissionModeChanged {
            mode: "permissive".into(),
        });
        model.sync_client_metadata(&state);
        assert_eq!(model.permission_mode.as_deref(), Some("permissive"));
    }

    #[test]
    fn projects_pending_required_actions_for_the_selected_session() {
        let mut state = ClientState::default();
        state.apply_server_message(ServerMessage::SessionReady {
            session_id: SESSION_ID.into(),
            cwd: Some("/tmp/mew-project".into()),
            model: Some("model".into()),
            provider: Some("provider".into()),
            permission_mode: Some("standard".into()),
        });
        state.apply_server_message(ServerMessage::PermissionRequest {
            request_id: "request-1".into(),
            tool_name: "shell".into(),
            input: serde_json::json!({"command": "pwd"}),
        });

        let mut model = UiModel::default();
        model.sync_client_metadata(&state);

        assert!(matches!(
            model.pending_actions.as_slice(),
            [action] if action.request_id == "request-1"
                && matches!(&action.kind, ActionKind::Permission { tool_name, .. } if tool_name == "shell")
        ));
    }
}
