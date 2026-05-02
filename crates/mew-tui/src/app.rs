use mew_agent::AgentEvent;
use mew_message::{Message, Part, PartId, Role, ToolState};
use mew_provider::ProviderEvent;
use std::collections::HashMap;

/// Minimum terminal width to show the sidebar.
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
/// Width of the sidebar in columns.
pub const SIDEBAR_WIDTH: u16 = 32;

/// The application's main state.
pub struct App {
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Input buffer.
    pub input: String,
    /// Cursor position in the input buffer (byte offset).
    pub cursor: usize,
    /// Scroll offset in the chat area (number of lines from bottom).
    pub scroll: u16,
    /// Current UI mode.
    pub mode: Mode,
    /// Status information.
    pub status: Status,
    /// Pending permission request, if any.
    pub permission: Option<PermissionState>,
    /// Map of part_id -> display state for tool calls.
    pub tool_states: HashMap<PartId, ToolDisplayState>,
    /// Input history (previous prompts).
    pub history: Vec<String>,
    /// Current history index when navigating.
    pub history_index: Option<usize>,
    /// Whether the agent is currently streaming.
    pub streaming: bool,
    /// Whether to exit the application.
    pub should_quit: bool,
    /// Context files loaded for this session.
    pub context_files: Vec<String>,
    /// Available tool names.
    pub tools: Vec<String>,
    /// Available models (id -> description) for the palette.
    pub models: Vec<(String, String)>,
    /// Active picker, if any.
    pub picker: Option<PickerState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    PermissionPrompt,
    SlashCommand,
    CommandPalette,
}

/// A single item in the command palette.
#[derive(Debug, Clone)]
pub struct PickerItem {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// State for the cmdk-style command palette.
#[derive(Debug)]
pub struct PickerState {
    /// What the picker is selecting (e.g. "command", "model").
    pub kind: String,
    pub items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
    pub cursor: usize,
}

impl PickerState {
    pub fn filtered(&self) -> Vec<&PickerItem> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|i| {
                i.label.to_lowercase().contains(&f)
                    || i.description.to_lowercase().contains(&f)
            })
            .collect()
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        let filtered = self.filtered();
        filtered.get(self.selected).copied()
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub model: String,
    pub provider: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: f64,
    pub session_id: String,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            model: String::new(),
            provider: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            session_id: String::new(),
        }
    }
}

#[derive(Debug)]
pub struct PermissionState {
    pub tool_name: String,
    pub call_id: String,
    pub input: serde_json::Value,
    /// Channel sender for the decision.
    pub tx: Option<tokio::sync::oneshot::Sender<mew_hooks::PermissionDecision>>,
    /// Currently selected option (0=allow once, 1=session, 2=deny).
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub enum ToolDisplayState {
    Running,
    Completed(String),
    Error(String),
}

/// How the TUI was launched.
pub enum RunMode {
    /// Interactive chat.
    Interactive,
    /// Single prompt (used by `mew run`).
    Single(String),
}

impl App {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            scroll: 0,
            mode: Mode::Normal,
            status: Status::default(),
            permission: None,
            tool_states: HashMap::new(),
            history: Vec::new(),
            history_index: None,
            streaming: false,
            should_quit: false,
            context_files: Vec::new(),
            tools: Vec::new(),
            models: Vec::new(),
            picker: None,
        }
    }

    /// Open the command palette with a list of commands.
    pub fn open_command_palette(&mut self) {
        let items = vec![
            PickerItem {
                id: "switch-model".into(),
                label: "Switch Model".into(),
                description: "Change the active LLM".into(),
            },
            PickerItem {
                id: "clear".into(),
                label: "Clear Chat".into(),
                description: "Remove all messages from the current session".into(),
            },
            PickerItem {
                id: "quit".into(),
                label: "Quit".into(),
                description: "Exit mew".into(),
            },
        ];
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "command".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
        });
    }

    /// Open a model picker.
    pub fn open_model_picker(&mut self) {
        let items: Vec<PickerItem> = self
            .models
            .iter()
            .map(|(id, desc)| PickerItem {
                id: id.clone(),
                label: id.clone(),
                description: desc.clone(),
            })
            .collect();
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "model".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
        });
    }

    /// Close the picker and return to normal mode.
    pub fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
    }

    /// Insert a character into the picker filter.
    pub fn picker_insert(&mut self, c: char) {
        if let Some(ref mut p) = self.picker {
            p.filter.insert(p.cursor, c);
            p.cursor += c.len_utf8();
            p.selected = 0;
        }
    }

    /// Backspace in the picker filter.
    pub fn picker_backspace(&mut self) {
        if let Some(ref mut p) = self.picker {
            if p.cursor > 0 {
                let prev = p.filter[..p.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                p.filter.remove(prev);
                p.cursor = prev;
                p.selected = p.selected.min(p.filtered().len().saturating_sub(1));
            }
        }
    }

    /// Move picker selection down.
    pub fn picker_down(&mut self) {
        if let Some(ref mut p) = self.picker {
            let count = p.filtered().len();
            if count > 0 {
                p.selected = (p.selected + 1).min(count - 1);
            }
        }
    }

    /// Move picker selection up.
    pub fn picker_up(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.selected = p.selected.saturating_sub(1);
        }
    }

    /// Move picker cursor left.
    pub fn picker_cursor_left(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.cursor = p.filter[..p.cursor]
                .char_indices()
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
        }
    }

    /// Move picker cursor right.
    pub fn picker_cursor_right(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.cursor = p.filter[p.cursor..]
                .chars()
                .next()
                .map(|c| p.cursor + c.len_utf8())
                .unwrap_or(p.filter.len());
        }
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(prev);
            self.cursor = prev;
        }
    }

    /// Delete the character at the cursor.
    pub fn delete_char(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
        }
    }

    /// Move cursor left.
    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
        }
    }

    /// Move cursor right.
    pub fn cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor = self.input[self.cursor..]
                .chars()
                .next()
                .map(|c| self.cursor + c.len_utf8())
                .unwrap_or(self.input.len());
        }
    }

    /// Move cursor to start.
    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end.
    pub fn cursor_end(&mut self) {
        self.cursor = self.input.len();
    }

    /// Submit the current input, returning it if non-empty.
    pub fn submit_input(&mut self) -> Option<String> {
        let text = self.input.trim();
        if text.is_empty() {
            return None;
        }
        let result = text.to_string();
        self.history.push(result.clone());
        self.history_index = None;
        self.input.clear();
        self.cursor = 0;
        Some(result)
    }

    /// Navigate history up.
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            Some(i) if i > 0 => i - 1,
            Some(_) => return,
            None => self.history.len() - 1,
        };
        if self.history_index.is_none() {
            // Save current draft.
            // (simplified: we don't restore draft on history down)
        }
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    /// Navigate history down.
    pub fn history_down(&mut self) {
        let idx = match self.history_index {
            Some(i) if i + 1 < self.history.len() => i + 1,
            Some(_) => {
                self.history_index = None;
                self.input.clear();
                self.cursor = 0;
                return;
            }
            None => return,
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    /// Scroll chat up.
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_add(amount);
    }

    /// Scroll chat down.
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    /// Process an agent event and update state.
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Provider(ProviderEvent::PartStart { part }) => {
                // Append part to the last assistant message, or create one.
                if let Some(msg) = self.messages.last_mut() {
                    if msg.role == Role::Assistant {
                        msg.parts.push(part);
                        return;
                    }
                }
                // No assistant message exists; create one.
                self.messages.push(Message {
                    id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                    role: Role::Assistant,
                    parts: vec![part],
                    time: mew_message::Time {
                        created: chrono::Utc::now().timestamp_millis(),
                        completed: None,
                    },
                    assistant: None,
                });
            }
            AgentEvent::Provider(ProviderEvent::PartDelta { part_id, delta, .. }) => {
                if let Some(msg) = self.messages.last_mut() {
                    if msg.role == Role::Assistant {
                        for part in &mut msg.parts {
                            if part.id() == part_id {
                                append_to_part(part, &delta);
                                break;
                            }
                        }
                    }
                }
            }
            AgentEvent::Provider(ProviderEvent::PartEnd { part_id }) => {
                if let Some(msg) = self.messages.last_mut() {
                    if msg.role == Role::Assistant {
                        for part in &mut msg.parts {
                            if part.id() == part_id {
                                finalize_part(part);
                                break;
                            }
                        }
                    }
                }
            }
            AgentEvent::Provider(ProviderEvent::MessageEnd { usage, cost, .. }) => {
                self.streaming = false;
                self.status.input_tokens += usage.input;
                self.status.output_tokens += usage.output;
                self.status.cost += cost;
                if let Some(msg) = self.messages.last_mut() {
                    msg.time.completed = Some(chrono::Utc::now().timestamp_millis());
                }
            }
            AgentEvent::Provider(ProviderEvent::Error(err)) => {
                self.streaming = false;
                self.messages.push(Message {
                    id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                    role: Role::Assistant,
                    parts: vec![Part::Text(mew_message::TextPart {
                        base: mew_message::PartBase {
                            id: ulid::Ulid::new(),
                            message_id: ulid::Ulid::new(),
                            session_id: ulid::Ulid::new(),
                        },
                        text: format!("Provider error: {}", err.message),
                        synthetic: true,
                    })],
                    time: mew_message::Time {
                        created: chrono::Utc::now().timestamp_millis(),
                        completed: Some(chrono::Utc::now().timestamp_millis()),
                    },
                    assistant: None,
                });
            }
            AgentEvent::PermissionRequest { call, tx } => {
                self.mode = Mode::PermissionPrompt;
                self.permission = Some(PermissionState {
                    tool_name: call.tool_name,
                    call_id: call.call_id,
                    input: call.input,
                    tx: Some(tx),
                    selected: 0,
                });
            }
            AgentEvent::ToolStart { call_id } => {
                // Find the tool call part and mark as running.
                for msg in self.messages.iter_mut().rev() {
                    for part in &mut msg.parts {
                        if let Part::ToolCall(tc) = part {
                            if tc.call_id == call_id {
                                self.tool_states.insert(
                                    tc.base.id,
                                    ToolDisplayState::Running,
                                );
                                break;
                            }
                        }
                    }
                }
            }
            AgentEvent::ToolEnd { call_id, success } => {
                for msg in self.messages.iter_mut().rev() {
                    for part in &mut msg.parts {
                        if let Part::ToolCall(tc) = part {
                            if tc.call_id == call_id {
                                let state = if success {
                                    ToolDisplayState::Completed(String::new())
                                } else {
                                    ToolDisplayState::Error(String::new())
                                };
                                self.tool_states.insert(tc.base.id, state);
                                break;
                            }
                        }
                    }
                }
            }
            AgentEvent::PartUpdated { part_id, part } => {
                if let Part::ToolCall(tc) = &part {
                    let state = match &tc.state {
                        ToolState::Running(_) => ToolDisplayState::Running,
                        ToolState::Completed(c) => ToolDisplayState::Completed(c.output.clone()),
                        ToolState::Error(e) => ToolDisplayState::Error(e.error.clone()),
                        _ => ToolDisplayState::Running,
                    };
                    self.tool_states.insert(part_id, state);
                }
                // Update the part in messages.
                for msg in self.messages.iter_mut().rev() {
                    for p in &mut msg.parts {
                        if p.id() == part_id {
                            *p = part.clone();
                            break;
                        }
                    }
                }
            }
            AgentEvent::Error(msg) => {
                self.streaming = false;
                // Add an error message.
                self.messages.push(Message {
                    id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                    role: Role::Assistant,
                    parts: vec![Part::Text(mew_message::TextPart {
                        base: mew_message::PartBase {
                            id: ulid::Ulid::new(),
                            message_id: ulid::Ulid::new(),
                            session_id: ulid::Ulid::new(),
                        },
                        text: format!("Error: {}", msg),
                        synthetic: true,
                    })],
                    time: mew_message::Time {
                        created: chrono::Utc::now().timestamp_millis(),
                        completed: Some(chrono::Utc::now().timestamp_millis()),
                    },
                    assistant: None,
                });
            }
        }
    }

    /// Send a permission decision.
    pub fn send_permission_decision(&mut self, decision: mew_hooks::PermissionDecision) {
        if let Some(perm) = self.permission.take() {
            if let Some(tx) = perm.tx {
                let _ = tx.send(decision);
            }
        }
        self.mode = Mode::Normal;
    }

    /// Select next permission option.
    pub fn permission_next(&mut self) {
        if let Some(ref mut perm) = self.permission {
            perm.selected = (perm.selected + 1) % 3;
        }
    }

    /// Select previous permission option.
    pub fn permission_prev(&mut self) {
        if let Some(ref mut perm) = self.permission {
            perm.selected = if perm.selected == 0 { 2 } else { perm.selected - 1 };
        }
    }
}

fn append_to_part(part: &mut Part, delta: &str) {
    match part {
        Part::Text(tp) => tp.text.push_str(delta),
        Part::Reasoning(rp) => rp.text.push_str(delta),
        _ => {}
    }
}

fn finalize_part(_part: &mut Part) {
    // Nothing special needed for most parts.
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
