use mew_agent::AgentEvent;
use mew_message::{Message, MessageId, Part, PartId, Role, ToolState};
use mew_provider::ProviderEvent;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use mdstream;

/// Minimum terminal width to show the sidebar.
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
/// Width of the sidebar in columns.
pub const SIDEBAR_WIDTH: u16 = 32;
/// Number of items visible in the picker list at once.
pub const PICKER_VISIBLE_ITEMS: usize = 8;

/// A single slash command definition.
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
}

/// Result of handling a slash command.
pub enum SlashResult {
    /// Command unknown; fall through to the model.
    Continue,
    Quit,
    Clear,
    /// Display a message in the chat pane.
    Message(String),
    /// Switch to a different model.
    SwitchModel(String),
    /// Open the model picker.
    OpenModelPicker,
    /// Resume a previous session by ID.
    ResumeSession(String),
    /// Force context compaction.
    Compact,
}

/// The application's main state.
pub struct App {
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Input buffer.
    pub input: String,
    /// Cursor position in the input buffer (byte offset).
    pub cursor: usize,
    /// Scroll offset in the chat area (number of lines from top).
    pub scroll: u16,
    /// Whether to auto-scroll to the bottom when content changes.
    pub auto_scroll: bool,
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
    /// Draft input saved when history navigation starts.
    pub history_draft: Option<String>,
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
    /// Selected slash command suggestion index.
    pub slash_selected: usize,
    /// Whether bash output is expanded (shows all lines vs last 10).
    pub bash_expanded: bool,
    /// Message ID pending markdown re-render after streaming completes.
    pub pending_md_rerender: Option<mew_message::MessageId>,
    /// Incremental markdown render state.
    pub md_state: mdstream::DocumentState,
    /// Active markdown stream (set during streaming, taken on completion).
    pub md_stream: Option<mdstream::MdStream>,
    /// Cached rendered markdown lines by message ID.
    pub rendered_md_cache: HashMap<MessageId, (u16, String, Rc<Vec<ratatui::text::Line<'static>>>)>,
    /// Last render width for cache invalidation.
    pub last_md_width: u16,
    /// Max scroll offset from the most recent render, used to re-attach auto-scroll.
    pub max_scroll: u16,
    /// Set on the first Esc press while streaming; second Esc within the window cancels.
    pub esc_cancel_pending: Option<Instant>,
    /// Set on the first Ctrl-c press while streaming; second Ctrl-c within 1s exits.
    pub ctrl_c_quit_pending: Option<Instant>,
    /// Current retry status for display in the status line.
    pub retry_status: Option<String>,
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
    /// First visible item index in the filtered list.
    pub scroll: usize,
}

impl PickerState {
    pub fn filtered(&self) -> Vec<&PickerItem> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|i| {
                i.label.to_lowercase().contains(&f) || i.description.to_lowercase().contains(&f)
            })
            .collect()
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        let filtered = self.filtered();
        filtered.get(self.selected).copied()
    }

    /// Ensure scroll keeps selected item in view for the given visible count.
    pub fn adjust_scroll(&mut self, visible_count: usize) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible_count {
            self.scroll = self.selected.saturating_sub(visible_count - 1);
        }
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
    pub context_window: u32,
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
            context_window: 0,
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
    Completed {
        output: String,
        diff: Option<String>,
    },
    Error(String),
}

impl App {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            scroll: 0,
            auto_scroll: true,
            mode: Mode::Normal,
            status: Status::default(),
            permission: None,
            tool_states: HashMap::new(),
            history: Vec::new(),
            history_index: None,
            history_draft: None,
            streaming: false,
            should_quit: false,
            context_files: Vec::new(),
            tools: Vec::new(),
            models: Vec::new(),
            picker: None,
            slash_selected: 0,
            bash_expanded: false,
            pending_md_rerender: None,
            md_state: mdstream::DocumentState::new(),
            md_stream: None,
            rendered_md_cache: HashMap::new(),
            last_md_width: 0,
            max_scroll: 0,
            esc_cancel_pending: None,
            ctrl_c_quit_pending: None,
            retry_status: None,
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
            scroll: 0,
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
            scroll: 0,
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
                p.adjust_scroll(PICKER_VISIBLE_ITEMS);
            }
        }
    }

    /// Move picker selection up.
    pub fn picker_up(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.selected = p.selected.saturating_sub(1);
            p.adjust_scroll(PICKER_VISIBLE_ITEMS);
        }
    }

    /// Move picker cursor left.
    pub fn picker_cursor_left(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.cursor = p.filter[..p.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
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

    /// Open the file picker for @-mentions, filtered by a prefix.
    pub fn open_file_picker(&mut self, prefix: &str) {
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut items: Vec<PickerItem> = Vec::new();

        let walker = ignore::WalkBuilder::new(&cwd)
            .max_depth(Some(4))
            .hidden(false)
            .git_ignore(true)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.len() > 1_048_576 {
                    continue;
                }
            }
            let rel = path
                .strip_prefix(&cwd)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            if !rel.to_lowercase().contains(&prefix.to_lowercase()) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            items.push(PickerItem {
                id: rel.clone(),
                label: format!("@{}", rel),
                description: if size > 1024 {
                    format!("{} KB", size / 1024)
                } else {
                    format!("{} B", size)
                },
            });
        }

        items.sort_by_key(|i| i.label.len());
        items.truncate(50);

        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "file".into(),
            items,
            filter: prefix.to_string(),
            selected: 0,
            cursor: prefix.len(),
            scroll: 0,
        });
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert a newline at cursor position.
    pub fn insert_newline(&mut self) {
        self.input.insert(self.cursor, '\n');
        self.cursor += 1;
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
                .map(|(i, _)| i)
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
        // Move to start of current line.
        let before = &self.input[..self.cursor];
        if let Some(ln_pos) = before.rfind('\n') {
            self.cursor = ln_pos + 1;
        } else {
            self.cursor = 0;
        }
    }

    /// Move cursor to end of current line.
    pub fn cursor_end(&mut self) {
        if let Some(ln_pos) = self.input[self.cursor..].find('\n') {
            self.cursor += ln_pos;
        } else {
            self.cursor = self.input.len();
        }
    }

    /// Number of lines in the input.
    pub fn input_line_count(&self) -> usize {
        self.input.lines().count()
    }

    /// Return (line index, byte offset within that line) for the cursor.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.input[..self.cursor];
        let line = before.lines().count().saturating_sub(1);
        let col = if let Some(ln_pos) = before.rfind('\n') {
            self.cursor - ln_pos - 1
        } else {
            self.cursor
        };
        (line, col)
    }

    /// Move cursor to the previous word boundary.
    pub fn cursor_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = &self.input[..self.cursor];
        let chars: Vec<(usize, char)> = before.char_indices().collect();
        if chars.len() < 2 {
            self.cursor = 0;
            return;
        }

        // Start from the character before cursor.
        let mut i = chars.len() - 1;
        let start_is_word = chars[i].1.is_alphanumeric();

        // Skip word chars if we started on one, or skip non-word chars if we started on one.
        while i > 0 && chars[i].1.is_alphanumeric() == start_is_word {
            i -= 1;
        }

        // If we ended on a different kind, land on the boundary.
        if chars[i].1.is_alphanumeric() != start_is_word {
            self.cursor = chars[i + 1].0;
        } else {
            self.cursor = chars[i].0;
        }
    }

    /// Move cursor to the next word boundary.
    pub fn cursor_word_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let after: Vec<(usize, char)> = self.input[self.cursor..].char_indices().collect();
        if after.is_empty() {
            return;
        }

        let start_is_word = after[0].1.is_alphanumeric();
        let mut i = 0;

        // Skip chars of the same kind.
        while i + 1 < after.len() && after[i + 1].1.is_alphanumeric() == start_is_word {
            i += 1;
        }

        self.cursor = self.cursor + after[i].0 + after[i].1.len_utf8();
    }

    /// Delete from cursor back to the previous word boundary.
    pub fn delete_word_left(&mut self) {
        let old_cursor = self.cursor;
        self.cursor_word_left();
        self.input.replace_range(self.cursor..old_cursor, "");
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
        self.mode = Mode::Normal;
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
            self.history_draft = Some(self.input.clone());
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
                self.input = self.history_draft.take().unwrap_or_default();
                self.cursor = self.input.len();
                return;
            }
            None => return,
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    /// Scroll chat up (show older messages). Disables auto-scroll.
    pub fn scroll_up(&mut self, amount: u16) {
        if self.auto_scroll {
            // Anchor to the actual bottom before subtracting so the first
            // scroll event is immediately visible even if the render hasn't
            // updated app.scroll yet.
            self.scroll = self.max_scroll;
        }
        self.auto_scroll = false;
        self.scroll = self.scroll.saturating_sub(amount);
    }

    /// Scroll chat down (show newer messages). Re-attaches auto-scroll at bottom.
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_add(amount);
        if self.scroll >= self.max_scroll {
            self.auto_scroll = true;
        }
    }

    fn push_synthetic_message(&mut self, text: String) {
        let msg_id = ulid::Ulid::new();
        self.messages.push(Message {
            id: msg_id,
            session_id: ulid::Ulid::new(),
            role: Role::Assistant,
            parts: vec![Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: ulid::Ulid::new(),
                    message_id: msg_id,
                    session_id: ulid::Ulid::new(),
                },
                text,
                synthetic: true,
            })],
            time: mew_message::Time {
                created: chrono::Utc::now().timestamp_millis(),
                completed: Some(chrono::Utc::now().timestamp_millis()),
            },
            assistant: None,
        });
    }

    /// Clear the chat display and all associated render state.
    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.tool_states.clear();
        self.rendered_md_cache.clear();
        self.pending_md_rerender = None;
    }

    /// Toggle bash output expansion.
    pub fn toggle_bash_expanded(&mut self) {
        self.bash_expanded = !self.bash_expanded;
    }

    /// Expire the pending-cancel hint and pending-quit hint.
    pub fn tick(&mut self) {
        if let Some(since) = self.esc_cancel_pending {
            if since.elapsed() > Duration::from_secs(2) {
                self.esc_cancel_pending = None;
                self.ctrl_c_quit_pending = None;
            }
        }
        if let Some(since) = self.ctrl_c_quit_pending {
            if since.elapsed() > Duration::from_secs(1) {
                self.ctrl_c_quit_pending = None;
            }
        }
    }

    /// Available slash commands (single source of truth for autocomplete and handling).
    pub fn slash_commands() -> Vec<SlashCommand> {
        vec![
            SlashCommand {
                name: "/clear".into(),
                description: "clear chat".into(),
            },
            SlashCommand {
                name: "/compact".into(),
                description: "force context compaction".into(),
            },
            SlashCommand {
                name: "/cost".into(),
                description: "show cost breakdown".into(),
            },
            SlashCommand {
                name: "/help".into(),
                description: "show available commands".into(),
            },
            SlashCommand {
                name: "/model".into(),
                description: "switch model (e.g. /model deepseek-v4-flash)".into(),
            },
            SlashCommand {
                name: "/quit".into(),
                description: "exit mew".into(),
            },
            SlashCommand {
                name: "/sessions".into(),
                description: "list previous sessions".into(),
            },
            SlashCommand {
                name: "/resume".into(),
                description: "resume a session (e.g. /resume <id>)".into(),
            },
        ]
    }

    /// Handle a slash command, returning what the caller should do.
    pub fn handle_slash(&self, input: &str) -> SlashResult {
        let (cmd, arg) = match input.split_once(' ') {
            Some((c, a)) => (c, Some(a)),
            None => (input, None),
        };
        match cmd {
            "/quit" | "/q" => SlashResult::Quit,
            "/clear" => SlashResult::Clear,
            "/compact" => SlashResult::Compact,
            "/cost" => SlashResult::Message(self.build_cost_report()),
            "/help" => SlashResult::Message(self.build_help()),
            "/model" => {
                if let Some(id) = arg {
                    SlashResult::SwitchModel(id.to_string())
                } else {
                    SlashResult::OpenModelPicker
                }
            }
            "/sessions" => SlashResult::Message(self.build_sessions_list()),
            "/resume" => {
                if let Some(id) = arg {
                    SlashResult::ResumeSession(id.to_string())
                } else {
                    SlashResult::Message("usage: /resume <session-id>".into())
                }
            }
            _ => SlashResult::Continue,
        }
    }

    fn build_cost_report(&self) -> String {
        let mut total = 0f64;
        let mut turns: Vec<(usize, f64)> = Vec::new();
        for (i, msg) in self.messages.iter().enumerate() {
            if let Some(ref meta) = msg.assistant {
                if meta.cost > 0.0 {
                    total += meta.cost;
                    turns.push((i, meta.cost));
                }
            }
        }
        let mut report = format!("session cost: ${:.4}\n", total);
        if turns.is_empty() {
            report.push_str("no recorded costs yet");
        } else {
            report.push_str("per-turn breakdown:\n");
            for (idx, cost) in turns {
                report.push_str(&format!("  turn {}: ${:.4}\n", idx, cost));
            }
        }
        report
    }

    fn build_help(&self) -> String {
        let mut out = String::from("commands:\n");
        for cmd in Self::slash_commands() {
            out.push_str(&format!("  {:12}  {}\n", cmd.name, cmd.description));
        }
        out.trim_end().to_string()
    }

    fn build_sessions_list(&self) -> String {
        use std::time::UNIX_EPOCH;
        let dir = mew_session::session_dir();
        let mut out = String::from("sessions:\n");
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                let mut files: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_name()
                            .to_str()
                            .map(|n| n.ends_with(".jsonl"))
                            .unwrap_or(false)
                    })
                    .collect();
                files.sort_by_key(|e| {
                    e.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH)
                });
                for entry in files.iter().rev().take(20) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let id = name.strip_suffix(".jsonl").unwrap_or(&name);
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push_str(&format!("  {}  ({} bytes)\n", id, size));
                }
                if files.is_empty() {
                    out.push_str("  (no sessions found)\n");
                }
            }
            Err(_) => {
                out.push_str("  (unable to read sessions directory)\n");
            }
        }
        out.trim_end().to_string()
    }

    /// Filter slash commands matching the current input.
    pub fn filtered_slash_commands(&self) -> Vec<SlashCommand> {
        let all = Self::slash_commands();
        if !self.input.starts_with('/') {
            return Vec::new();
        }
        let prefix = self.input.to_lowercase();
        all.into_iter()
            .filter(|c| c.name.to_lowercase().starts_with(&prefix))
            .collect()
    }

    /// Cycle to next slash command suggestion.
    pub fn slash_next(&mut self) {
        let filtered = self.filtered_slash_commands();
        if !filtered.is_empty() {
            self.slash_selected = (self.slash_selected + 1) % filtered.len();
        }
    }

    /// Cycle to previous slash command suggestion.
    pub fn slash_prev(&mut self) {
        let filtered = self.filtered_slash_commands();
        if !filtered.is_empty() {
            self.slash_selected = if self.slash_selected == 0 {
                filtered.len() - 1
            } else {
                self.slash_selected - 1
            };
        }
    }

    /// Apply the selected slash command completion.
    pub fn apply_slash_completion(&mut self) {
        let filtered = self.filtered_slash_commands();
        if let Some(cmd) = filtered.get(self.slash_selected) {
            self.input = format!("{} ", cmd.name);
            self.cursor = self.input.len();
        }
    }

    /// Process an agent event and update state.
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Provider(ProviderEvent::PartStart { part }) => {
                // Append part to the last assistant message, or create one.
                if let Some(msg) = self.messages.last_mut() {
                    if msg.role == Role::Assistant {
                        // When a new text part starts (e.g. after tool execution in a
                        // multi-turn loop), reset the streaming state so incremental
                        // deltas are rendered for this part, not silently dropped.
                        if matches!(part, Part::Text(_)) {
                            self.md_stream =
                                Some(mdstream::MdStream::new(mdstream::Options::default()));
                            self.md_state = mdstream::DocumentState::new();
                        }
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
                // Start incremental markdown stream for this message.
                self.md_stream = Some(mdstream::MdStream::new(mdstream::Options::default()));
                self.md_state = mdstream::DocumentState::new();
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
                // Feed delta into the incremental markdown stream.
                if let Some(ref mut stream) = self.md_stream {
                    let update = stream.append(&delta);
                    self.md_state.apply(update);
                }
            }
            AgentEvent::Provider(ProviderEvent::PartEnd { .. }) => {}
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish,
                usage,
                cost,
            }) => {
                // Finalize the incremental markdown stream.
                if let Some(mut stream) = self.md_stream.take() {
                    let update: mdstream::Update = stream.finalize();
                    self.md_state.apply(update);
                }
                // Mark the last message for re-render on next frame.
                if let Some(msg) = self.messages.last() {
                    self.pending_md_rerender = Some(msg.id);
                }
                // Invalidate cache so next render re-highlights from scratch.
                // ToolUse means the agent is still working - it will execute tools
                // and request another stream. Stay in streaming mode.
                if finish != mew_message::Finish::ToolUse {
                    self.streaming = false;
                    self.esc_cancel_pending = None;
                    self.ctrl_c_quit_pending = None;
                    self.retry_status = None;
                }
                self.status.input_tokens += usage.input;
                self.status.output_tokens += usage.output;
                self.status.cost += cost;
                if let Some(msg) = self.messages.last_mut() {
                    msg.time.completed = Some(chrono::Utc::now().timestamp_millis());
                }
            }
            AgentEvent::Provider(ProviderEvent::RetryWait {
                attempt,
                max_attempts,
                delay_secs,
                reason,
            }) => {
                self.retry_status = Some(format!(
                    "retrying ({}/{}): {} in {}s",
                    attempt, max_attempts, reason, delay_secs
                ));
            }
            AgentEvent::Provider(ProviderEvent::Error(err)) => {
                self.streaming = false;
                self.esc_cancel_pending = None;
                self.ctrl_c_quit_pending = None;
                self.retry_status = None;
                self.push_synthetic_message(format!("Provider error: {}", err.message));
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
                                self.tool_states
                                    .insert(tc.base.id, ToolDisplayState::Running);
                                break;
                            }
                        }
                    }
                }
            }
            AgentEvent::ToolProgress { call_id, chunk } => {
                // Append chunk to the running tool call's output.
                for msg in self.messages.iter_mut().rev() {
                    for part in &mut msg.parts {
                        if let Part::ToolCall(tc) = part {
                            if tc.call_id == call_id {
                                if let ToolState::Running(ref mut running) = tc.state {
                                    if !running.output.is_empty() {
                                        running.output.push('\n');
                                    }
                                    running.output.push_str(&chunk);
                                }
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
                                // Only set a default state if PartUpdated hasn't
                                // already populated it (which carries the diff).
                                if let std::collections::hash_map::Entry::Vacant(e) =
                                    self.tool_states.entry(tc.base.id)
                                {
                                    let state = if success {
                                        ToolDisplayState::Completed {
                                            output: String::new(),
                                            diff: None,
                                        }
                                    } else {
                                        ToolDisplayState::Error(String::new())
                                    };
                                    e.insert(state);
                                }
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
                        ToolState::Completed(c) => ToolDisplayState::Completed {
                            output: c.output.clone(),
                            diff: c.diff.clone(),
                        },
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
                self.esc_cancel_pending = None;
                self.ctrl_c_quit_pending = None;
                self.push_synthetic_message(format!("Error: {}", msg));
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
            perm.selected = if perm.selected == 0 {
                2
            } else {
                perm.selected - 1
            };
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

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract `@path` file mentions from input text.
/// Returns a list of path strings (without the `@` prefix).
pub fn parse_file_mentions(text: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    for word in text.split_whitespace() {
        if let Some(path) = word.strip_prefix('@') {
            // Strip trailing punctuation.
            let path = path.trim_end_matches(|c: char| c.is_ascii_punctuation());
            if !path.is_empty() {
                mentions.push(path.to_string());
            }
        }
    }
    mentions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_file_mentions_basic() {
        let text = "fix the bug in @src/main.rs";
        let mentions = parse_file_mentions(text);
        assert_eq!(mentions, vec!["src/main.rs"]);
    }

    #[test]
    fn test_parse_file_mentions_multiple() {
        let text = "compare @a.txt and @b.txt";
        let mentions = parse_file_mentions(text);
        assert_eq!(mentions, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn test_parse_file_mentions_with_punctuation() {
        let text = "check @README.md, then @Cargo.toml.";
        let mentions = parse_file_mentions(text);
        assert_eq!(mentions, vec!["README.md", "Cargo.toml"]);
    }

    #[test]
    fn test_parse_file_mentions_none() {
        let text = "no mentions here";
        let mentions = parse_file_mentions(text);
        assert!(mentions.is_empty());
    }
}
