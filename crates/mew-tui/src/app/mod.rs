use mew_agent::{AgentEvent, AskUserQuestion};
use mew_message::{Message, Part, PartId, Role, ToolState};
use mew_provider::ProviderEvent;
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use mdstream;

pub const SIDEBAR_MIN_WIDTH: u16 = 120;
pub const SIDEBAR_WIDTH: u16 = 32;
pub const PICKER_VISIBLE_ITEMS: usize = 8;
const SPINNER_TICK_DIVISOR: u8 = 5;

/// A single slash command definition.
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
}

/// Result of handling a slash command.
#[derive(Debug, strum::EnumIter)]
pub enum SlashResult {
    Continue,
    Quit,
    Clear,
    Message(String),
    SwitchModel(String),
    SwitchPersona(String),
    PersonaSwitchConfirm(String),
    OpenModelPicker,
    ResumeSession(String),
    ToggleMouseCapture,
    Compact,
    Todo,
    Rewind(usize),
    PluginCommand { name: String, args: String },
    PermissionModeMenu,
    SetPermissionMode(mew_hooks::PermissionMode),
    SetThinkingVariant(String),
    SetTheme(String),
    OpenThinkingVariantPicker,
    OpenCommandPalette,
    OpenThemePicker,
    OpenPersonaPicker,
    OpenRewindPicker,
    OpenSessionPickerFromDisk,
    OpenHelp,
}

pub struct App {
    pub theme: crate::theme::Theme,
    pub(crate) messages: Vec<Message>,
    pub input: String,
    pub cursor: usize,
    pub scroll: u16,
    pub auto_scroll: bool,
    pub mode: Mode,
    pub status: Status,
    pub permission: Option<PermissionState>,
    pub user_question: Option<UserQuestionState>,
    pub plan_approval: Option<PlanApprovalState>,
    pub persona_switch_confirm: Option<PersonaSwitchConfirmState>,
    pub pending_persona_switch_apply: Option<String>,
    pub todos: Vec<mew_agent::Todo>,
    pub tool_states: HashMap<PartId, ToolDisplayState>,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub history_draft: Option<String>,
    pub streaming: bool,
    pub spinner_frame: usize,
    spinner_sub_tick: u8,
    pub should_quit: bool,
    pub context_files: Vec<String>,
    pub tools: Vec<String>,
    pub personas: Vec<(String, String)>,
    pub active_persona: Option<String>,
    pub active_persona_color: Option<String>,
    pub permission_mode: mew_hooks::PermissionMode,
    pub mcp_status: Vec<(String, bool, usize)>,
    pub subagent_names: Vec<(String, String)>,
    pub sidebar_collapsed: std::collections::HashMap<String, bool>,
    pub sidebar_header_rows: Vec<(u16, String)>,
    pub sidebar_rect: Rect,
    pub models: Vec<(String, String)>,
    pub thinking_variants: HashMap<String, Vec<String>>,
    pub active_thinking_variant: Option<String>,
    pub picker: Option<PickerState>,
    pub settings: Option<crate::settings::SettingsState>,
    pub slash_selected: usize,
    pub slash_scroll: usize,
    pub bash_expanded: bool,
    pub reasoning_expanded: HashSet<PartId>,
    pub reasoning_active_id: Option<PartId>,
    pub reasoning_started_at: Option<std::time::Instant>,
    pub reasoning_elapsed: Option<std::time::Duration>,
    pub reasoning_header_rows: Vec<(PartId, usize)>,
    pub pending_md_rerender: Option<mew_message::MessageId>,
    pub md_state: mdstream::DocumentState,
    pub md_stream: Option<mdstream::MdStream>,
    pub md_render_cache: ratatui_mdstream::cache::RenderCache,
    pub rendered_md_cache:
        HashMap<mew_message::PartId, (u16, String, Rc<Vec<ratatui::text::Line<'static>>>)>,
    pub last_md_width: u16,
    pub max_scroll: u16,
    pub rendered_chat: Option<RenderedChat>,
    pub chat_dirty: Option<u64>,
    pub rendered_chat_width: u16,
    pub esc_cancel_pending: Option<Instant>,
    pub ctrl_c_quit_pending: Option<Instant>,
    pub retry_status: Option<String>,
    pub mouse_capture: bool,
    pub pending_mouse_toggle: bool,
    pub chat_area: Rect,
    pub input_area: Rect,
    pub sel_anchor_row: Option<usize>,
    pub sel_anchor_col: Option<usize>,
    pub sel_end_row: Option<usize>,
    pub sel_end_col: Option<usize>,
    pub chat_rows: Vec<String>,
    pub alert: Option<(String, Instant)>,
    pub toasts: Vec<(String, Instant)>,
    pub plugin_ui: std::collections::HashMap<String, String>,
    pub dynamic_slash_commands: Vec<SlashCommand>,
    pub companion_bubble_since: Option<Instant>,
    pub status_ticker_offset: usize,
    pub status_ticker_at: Option<Instant>,
    pub git_branch: Option<String>,
    pub git_branch_resolved: bool,
    pub short_cwd: String,
    pub subagents: Vec<SubagentState>,
    pub background_jobs: Vec<BackgroundJobState>,
    pub undo_stack: Vec<(String, usize)>,
    pub redo_stack: Vec<(String, usize)>,
    pub last_undo_push: Option<Instant>,
    pub history_search_query: String,
    pub history_search_index: Option<usize>,
    pub history_search_saved: Option<(String, usize)>,
    pub pending_paste: Option<String>,

    // ── Daemon-mode state ──────────────────────────────────────────
    // All fields below are daemon-only. In local mode (`run_tui`), they
    // stay at their Default values and are never populated. `daemon_mode`
    // is the single gate — UI surfaces check it before rendering.
    pub daemon_mode: bool,
    pub daemon_sessions: Vec<mew_protocol::SessionInfo>,
    pub session_titles: std::collections::HashMap<String, String>,
    pub session_summaries: std::collections::HashMap<String, String>,
    pub session_attention: std::collections::HashMap<String, (u32, u32)>,
    pub auto_title: bool,
    pub auto_summary: bool,
    pub tool_batch_expanded: std::collections::HashSet<mew_message::PartId>,
    /// Test-only instrumentation: counts how many times `ensure_chat_rendered`
    /// rebuilds the chat (the `!cache_ok` branch). Used by `test_daemon_coalescing`
    /// (AC.13) to assert the 4-agent-event cap is respected.
    /// Renamed from `render_count` doc — the test that uses this field
    /// is now `test_render_cache_batches_deltas`.
    #[cfg(test)]
    pub render_count: u32,
}

/// Cached result of building the chat `Text` for one transcript state.
///
/// `lines` holds every rendered line (already indented, em-dash-fixed,
/// selection applied, and pre-wrapped to ≤ chat width so each entry is
/// exactly one visual row — letting us slice the visible window without
/// ratatui's O(scroll.y) skip). `chat_rows` is the plain-text mirror used
/// by mouse selection. `max_scroll` is cached so `scroll_up`/`scroll_down`
/// can clamp without rebuilding. `dirty_gen` is the `chat_dirty` generation
/// this cache was built from, so a stale cache is detected in O(1).
#[derive(Clone)]
pub struct RenderedChat {
    pub lines: Vec<ratatui::text::Line<'static>>,
    pub chat_rows: Vec<String>,
    pub total_wrapped: u16,
    pub max_scroll: u16,
    pub dirty_gen: u64,
}

pub struct BuiltChat {
    pub lines: Vec<ratatui::text::Line<'static>>,
    pub chat_rows: Vec<String>,
}

/// A running or completed subagent task shown in the sidebar.
#[derive(Debug, Clone)]
pub struct SubagentState {
    pub task_id: String,
    pub name: String,
    pub started_at: Instant,
    pub status: SubagentStatus,
    pub last_progress: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed { reason: String },
    Cancelled,
}

/// A background shell job shown in the sidebar.
#[derive(Debug, Clone)]
pub struct BackgroundJobState {
    pub job_id: String,
    pub command: String,
    pub started_at: Instant,
    pub status: BackgroundJobStatus,
}

#[derive(Debug, Clone)]
pub enum BackgroundJobStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    PermissionPrompt,
    SlashCommand,
    CommandPalette,
    Settings,
    UserQuestion,
    PlanApproval,
    PersonaSwitchConfirm,
    Help,
    HistorySearch,
    PasteConfirm,
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
    pub kind: String,
    pub items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
    pub cursor: usize,
    pub scroll: usize,
    pub visible_items: usize,
    pub hint: Option<String>,
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

    /// Ensure scroll keeps selected item in view.
    pub fn adjust_scroll(&mut self) {
        let visible = self.visible_items;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected.saturating_sub(visible - 1);
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
    pub tx: Option<tokio::sync::oneshot::Sender<mew_hooks::PermissionDecision>>,
    pub selected: usize,
}

/// A pending persona switch awaiting user confirmation. Populated by the
/// `/persona <name>` slash command when the target differs from the active
/// persona, and consumed (applied or discarded) by the confirm modal.
#[derive(Debug, Clone)]
pub struct PersonaSwitchConfirmState {
    pub target: PersonaSummary,
    pub current: Option<PersonaSummary>,
    pub selected: usize,
}

/// Display-friendly snapshot of a persona's identity + restrictions. Built
/// by the main loop when a `/persona` switch is requested, and rendered by
/// the confirm modal. Decoupled from `mew_personas::Persona` so the tui
/// crate doesn't need to depend on the full persona type.
#[derive(Debug, Clone, Default)]
pub struct PersonaSummary {
    pub name: String,
    pub description: String,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub tools_deny: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub color: Option<String>,
}

/// A pending `ask_user_question` prompt. Shown as a one-question-per-page
/// inline overlay replacing the input box, with multiple-choice options
/// and an implicit "type your own" final option.
#[derive(Debug)]
pub struct UserQuestionState {
    pub call_id: String,
    pub questions: Vec<AskUserQuestion>,
    pub answers: Vec<String>,
    pub page: usize,
    pub selected: usize,
    pub freeform_text: String,
    pub review: bool,
    pub review_selected: usize,
    pub tx: Option<tokio::sync::oneshot::Sender<Vec<String>>>,
}

/// A pending `handoff_plan` approval. Shown as a large centered modal with
/// the full plan rendered as markdown, an approve / request-changes toggle,
/// and a feedback editor for the request-changes path.
#[derive(Debug)]
pub struct PlanApprovalState {
    pub call_id: String,
    pub plan_path: String,
    pub persona: String,
    pub plan_markdown: String,
    pub scroll: u16,
    /// 0 = approve, 1 = request changes.
    pub selected: usize,
    /// True while the user is typing feedback for the request-changes path.
    pub editing_feedback: bool,
    pub feedback: String,
    pub tx: Option<tokio::sync::oneshot::Sender<mew_agent::PlanDecision>>,
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
            theme: crate::theme::Theme::dark(),
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            scroll: 0,
            auto_scroll: true,
            mode: Mode::Normal,
            status: Status::default(),
            permission: None,
            user_question: None,
            plan_approval: None,
            persona_switch_confirm: None,
            pending_persona_switch_apply: None,
            todos: Vec::new(),
            tool_states: HashMap::new(),
            history: Vec::new(),
            history_index: None,
            history_draft: None,
            streaming: false,
            spinner_frame: 0,
            spinner_sub_tick: 0,
            should_quit: false,
            context_files: Vec::new(),
            tools: Vec::new(),
            personas: Vec::new(),
            active_persona: None,
            active_persona_color: None,
            permission_mode: Default::default(),
            mcp_status: Vec::new(),
            subagent_names: Vec::new(),
            sidebar_collapsed: std::collections::HashMap::new(),
            sidebar_header_rows: Vec::new(),
            sidebar_rect: Rect::default(),
            models: Vec::new(),
            thinking_variants: HashMap::new(),
            active_thinking_variant: None,
            picker: None,
            settings: None,
            slash_selected: 0,
            slash_scroll: 0,
            bash_expanded: false,
            reasoning_expanded: HashSet::new(),
            reasoning_active_id: None,
            reasoning_started_at: None,
            reasoning_elapsed: None,
            reasoning_header_rows: Vec::new(),
            pending_md_rerender: None,
            md_state: mdstream::DocumentState::new(),
            md_stream: None,
            md_render_cache: ratatui_mdstream::cache::RenderCache::new(),
            rendered_md_cache: HashMap::new(),
            last_md_width: 0,
            max_scroll: 0,
            rendered_chat: None,
            chat_dirty: None,
            rendered_chat_width: 0,
            esc_cancel_pending: None,
            ctrl_c_quit_pending: None,
            retry_status: None,
            mouse_capture: true,
            pending_mouse_toggle: false,
            chat_area: Rect::ZERO,
            input_area: Rect::ZERO,
            sel_anchor_row: None,
            sel_anchor_col: None,
            sel_end_row: None,
            sel_end_col: None,
            chat_rows: Vec::new(),
            alert: None,
            toasts: Vec::new(),
            plugin_ui: std::collections::HashMap::new(),
            dynamic_slash_commands: Vec::new(),
            companion_bubble_since: None,
            status_ticker_offset: 0,
            status_ticker_at: None,
            git_branch: None,
            git_branch_resolved: false,
            short_cwd: short_cwd(),
            subagents: Vec::new(),
            background_jobs: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_undo_push: None,
            history_search_query: String::new(),
            history_search_index: None,
            history_search_saved: None,
            pending_paste: None,
            daemon_mode: false,
            daemon_sessions: Vec::new(),
            session_titles: std::collections::HashMap::new(),
            session_summaries: std::collections::HashMap::new(),
            session_attention: std::collections::HashMap::new(),
            auto_title: false,
            auto_summary: false,
            tool_batch_expanded: std::collections::HashSet::new(),
            #[cfg(test)]
            render_count: 0,
        }
    }

    /// Process a daemon `ServerMessage` notification. Only called in daemon
    /// mode (when `daemon_mode` is true). Updates the daemon-scoped state
    /// fields; UI surfaces read those fields during `draw()`.
    pub fn apply_daemon_notification(&mut self, msg: &mew_protocol::ServerMessage) {
        use mew_protocol::ServerMessage;
        match msg {
            ServerMessage::SessionList { sessions } => {
                self.daemon_sessions = sessions.clone();
            }
            ServerMessage::SessionTitleChanged { session_id, title } => {
                self.session_titles
                    .insert(session_id.clone(), title.clone());
            }
            ServerMessage::SessionSummaryChanged {
                session_id,
                summary,
            } => {
                self.session_summaries
                    .insert(session_id.clone(), summary.clone());
            }
            ServerMessage::SessionAttentionChanged {
                session_id,
                pending_permissions,
                pending_questions,
            } => {
                self.session_attention.insert(
                    session_id.clone(),
                    (*pending_permissions, *pending_questions),
                );
            }
            ServerMessage::SessionAlert {
                title,
                kind,
                detail,
                session_id,
            } => {
                // Show a toast for the alert. If it's for a non-active
                // session, prefix with the session title.
                let prefix = if session_id != &self.status.session_id {
                    let name = self
                        .session_titles
                        .get(session_id)
                        .cloned()
                        .unwrap_or_else(|| session_id.chars().take(8).collect::<String>());
                    format!("[{}] ", name)
                } else {
                    String::new()
                };
                let kind_label = match kind {
                    mew_protocol::AlertKind::PermissionNeeded => "⚠ ",
                    mew_protocol::AlertKind::InputNeeded => "? ",
                    mew_protocol::AlertKind::TurnComplete => "✓ ",
                    mew_protocol::AlertKind::TurnFailed => "✗ ",
                };
                let detail_str = detail
                    .as_deref()
                    .filter(|d| !d.is_empty())
                    .map(|d| format!(": {}", d))
                    .unwrap_or_default();
                self.set_alert(format!("{}{}{}{}", prefix, kind_label, title, detail_str));
            }
            ServerMessage::SessionHistory { messages, .. } => {
                // Replay: replace the current chat with the session's
                // history. This is the /resume path — the daemon sends
                // the full message list on attach.
                self.messages.clear();
                self.md_stream = None;
                self.md_state = mdstream::DocumentState::new();
                self.md_render_cache.invalidate();
                for msg in messages {
                    self.messages.push(msg.clone());
                }
                self.auto_scroll = true;
                self.pending_md_rerender = self.messages.last().map(|m| m.id);
                self.mark_chat_dirty();
            }
            ServerMessage::ModelSwitched {
                provider, model, ..
            } => {
                self.status.provider = provider.clone();
                self.status.model = model.clone();
            }
            ServerMessage::ThinkingVariantChanged { variant, .. } => {
                self.active_thinking_variant = variant.clone();
            }
            ServerMessage::PermissionModeChanged { mode, .. } => {
                if let Some(pm) = mew_hooks::PermissionMode::from_id(mode) {
                    self.permission_mode = pm;
                }
            }
            ServerMessage::SessionMetaChanged { session_id, .. } => {
                // A session's metadata changed (pinned, archived, etc.).
                // The next SessionList will refresh the rail.
                let _ = session_id;
            }
            ServerMessage::FlaggedFilesChanged { files, .. } => {
                // Store flagged files for the Changes sidebar section.
                // (Item 8 will render these; for now just log.)
                let _ = files;
            }
            _ => {
                // Other notifications (ClientAttached, FsChanged, etc.)
                // are not yet consumed by the TUI.
            }
        }
    }

    /// Open a session picker that reads from disk (standalone mode).
    /// Reuses kind: "session" so the existing AttachSession dispatch works.
    pub fn open_session_picker_from_disk(&mut self) {
        use std::time::UNIX_EPOCH;
        let dir = mew_session::session_dir();
        let mut items: Vec<PickerItem> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut folders: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter(|e| e.path().join("session.jsonl").exists())
                .collect();
            folders.sort_by_key(|e| {
                e.path()
                    .join("session.jsonl")
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH)
            });
            for entry in folders.iter().rev().take(20) {
                let name = entry.file_name().to_string_lossy().to_string();
                let size = entry
                    .path()
                    .join("session.jsonl")
                    .metadata()
                    .map(|m| m.len())
                    .unwrap_or(0);
                items.push(PickerItem {
                    id: name.clone(),
                    label: name,
                    description: format!("{} bytes", size),
                });
            }
        }
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "session".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    /// Show a temporary alert that auto-clears after 3 seconds. Pushes
    /// to the toast queue (bottom-right, stacks up to 3). Also sets
    /// `alert` for backward compat with the existing centered render.
    pub fn set_alert(&mut self, text: impl Into<String>) {
        let text = text.into();
        let now = Instant::now();
        self.alert = Some((text.clone(), now));
        self.toasts.push((text, now));
        // Keep at most 3 toasts.
        while self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    /// Mark companion bubble as just-updated (for auto-dismiss timer).
    pub fn touch_companion_bubble(&mut self) {
        self.companion_bubble_since = Some(Instant::now());
    }

    /// Duration the companion bubble stays visible: max(3s, text_len * 0.075s).
    pub fn companion_bubble_ttl(text: &str) -> Duration {
        let char_ttl = Duration::from_secs_f64(text.len() as f64 * 0.075);
        char_ttl.max(Duration::from_secs(3))
    }

    /// Clear expired alerts (older than 3 seconds) and expired toasts.
    pub fn clear_expired_alerts(&mut self) {
        if let Some((_, at)) = &self.alert {
            if at.elapsed() > Duration::from_secs(3) {
                self.alert = None;
            }
        }
        // Expire toasts older than 3 seconds.
        self.toasts
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(3));
    }

    // -- Input undo/redo --

    // -- Ctrl+R history search --

    /// Start a reverse history search. Saves current input state.
    pub fn start_history_search(&mut self) {
        self.history_search_saved = Some((self.input.clone(), self.cursor));
        self.history_search_query.clear();
        self.history_search_index = None;
        self.mode = Mode::HistorySearch;
    }

    /// Filtered history entries matching the current search query (newest first).
    pub fn history_search_matches(&self) -> Vec<String> {
        if self.history_search_query.is_empty() {
            return self.history.iter().rev().cloned().collect();
        }
        let q = self.history_search_query.to_lowercase();
        self.history
            .iter()
            .rev()
            .filter(|h| h.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    /// The current match text, if any.
    pub fn history_search_current_match(&self) -> Option<String> {
        let matches = self.history_search_matches();
        let idx = self.history_search_index?;
        matches.get(idx).cloned()
    }

    /// Cycle to the next (older) match.
    pub fn history_search_next(&mut self) {
        let count = self.history_search_matches().len();
        if count == 0 {
            return;
        }
        let idx = match self.history_search_index {
            Some(i) if i + 1 < count => i + 1,
            _ => 0,
        };
        self.history_search_index = Some(idx);
    }

    /// Cycle to the previous (newer) match.
    pub fn history_search_prev(&mut self) {
        let count = self.history_search_matches().len();
        if count == 0 {
            return;
        }
        let idx = match self.history_search_index {
            Some(0) | None => count - 1,
            Some(i) => i - 1,
        };
        self.history_search_index = Some(idx);
    }

    /// Confirm the search: insert the current match into the input and
    /// return to Normal mode.
    pub fn history_search_confirm(&mut self) {
        if let Some(match_text) = self.history_search_current_match() {
            self.input = match_text;
            self.cursor = self.input.len();
        }
        self.history_search_query.clear();
        self.history_search_index = None;
        self.history_search_saved = None;
        self.mode = Mode::Normal;
    }

    /// Cancel the search: restore the saved input state.
    pub fn history_search_cancel(&mut self) {
        if let Some((saved_input, saved_cursor)) = self.history_search_saved.take() {
            self.input = saved_input;
            self.cursor = saved_cursor;
        }
        self.history_search_query.clear();
        self.history_search_index = None;
        self.mode = Mode::Normal;
    }

    /// Whether the UI needs a redraw on this tick. Returns false when the
    /// app is idle (not streaming, no animations, no expiring alerts, no
    /// spinner, no status marquee). Input events and agent events always
    /// trigger a draw regardless of this flag — the main loop only consults
    /// it on `Event::Tick`.
    ///
    /// The conditions here mirror what `tick()` mutates: if `tick()` would
    /// change any visible state, we need a redraw. If `tick()` is a no-op,
    /// we can skip the draw and let the terminal stay as-is.
    pub fn needs_redraw(&self) -> bool {
        // Streaming text: always redraw (the spinner advances + text
        // may have new deltas from the drain loop).
        if self.streaming {
            return true;
        }
        // Any tool running (not streaming but a tool is executing): the
        // spinner should animate. We check tool_states for any Running
        // entry.
        if self
            .tool_states
            .values()
            .any(|s| matches!(s, ToolDisplayState::Running))
        {
            return true;
        }
        // Alert or toasts visible and may expire.
        if self.alert.is_some() || !self.toasts.is_empty() {
            return true;
        }
        // Companion bubble visible (has a TTL → may expire).
        if self.companion_bubble_since.is_some() {
            return true;
        }
        // Status marquee scrolling.
        if self.status_ticker_at.is_some() {
            return true;
        }
        // Esc-cancel or Ctrl-C-quit pending hint (has a TTL).
        if self.esc_cancel_pending.is_some() || self.ctrl_c_quit_pending.is_some() {
            return true;
        }
        // Any modal/picker open (always interactive, always redraw).
        if !matches!(self.mode, Mode::Normal | Mode::Settings) {
            return true;
        }
        false
    }

    /// Return the selected text accounting for row and column ranges.
    /// `sel_anchor_col` / `sel_end_col` are display columns; convert to byte offsets.
    pub fn selected_text(&self) -> String {
        let (ar, ac, er, ec) = match (
            self.sel_anchor_row,
            self.sel_anchor_col,
            self.sel_end_row,
            self.sel_end_col,
        ) {
            (Some(ar), Some(ac), Some(er), Some(ec)) => (ar, ac, er, ec),
            _ => return String::new(),
        };
        let lo = ar.min(er);
        let hi = ar.max(er).min(self.chat_rows.len().saturating_sub(1));
        if lo >= self.chat_rows.len() {
            return String::new();
        }
        let (lo_col, hi_col) = if ar < er {
            (ac, ec)
        } else if ar > er {
            (ec, ac)
        } else {
            (ac.min(ec), ac.max(ec))
        };
        let mut parts: Vec<String> = Vec::new();
        for (row, line) in self
            .chat_rows
            .iter()
            .enumerate()
            .skip(lo)
            .take(hi.saturating_sub(lo) + 1)
        {
            let byte_lo = byte_at_display_offset(line, lo_col);
            let byte_hi = byte_at_display_offset(line, hi_col);
            if row == lo && row == hi {
                if byte_lo < byte_hi {
                    parts.push(line[byte_lo..byte_hi].to_string());
                }
            } else if row == lo {
                parts.push(line[byte_lo..].to_string());
            } else if row == hi {
                parts.push(line[..byte_hi].to_string());
            } else {
                parts.push(line.to_string());
            }
        }

        // For multi-line selections, strip the common leading whitespace
        // prefix (the TUI's left margin / padding that the user didn't
        // intend to copy). Single-line selections are left verbatim since
        // the user explicitly positioned the column cursor.
        if parts.len() > 1 {
            let common_ws = parts
                .iter()
                .map(|p| p.chars().take_while(|c| c.is_whitespace()).count())
                .min()
                .unwrap_or(0);
            if common_ws > 0 {
                for part in &mut parts {
                    *part = part.chars().skip(common_ws).collect();
                }
            }
        }

        // Strip carriage returns that leak from terminal rendering.
        let result = parts.join("\n");
        result.replace('\r', "")
    }

    pub fn clear_selection(&mut self) {
        self.sel_anchor_row = None;
        self.sel_anchor_col = None;
        self.sel_end_row = None;
        self.sel_end_col = None;
        self.mark_chat_dirty();
    }

    /// Toggle a sidebar section's collapsed state by name ("context", "tools", "mcp").
    pub fn toggle_sidebar_section(&mut self, section: &str) {
        let collapsed = self
            .sidebar_collapsed
            .entry(section.to_string())
            .or_insert(false);
        *collapsed = !*collapsed;
    }

    /// Open the persona-switch confirm modal for the given target. The caller
    /// is responsible for actually applying the switch after the user
    /// confirms (see `take_confirmed_persona_switch`).
    pub fn request_persona_switch_confirm(
        &mut self,
        target: PersonaSummary,
        current: Option<PersonaSummary>,
    ) {
        self.persona_switch_confirm = Some(PersonaSwitchConfirmState {
            target,
            current,
            selected: 0, // default focus is the confirm button
        });
        self.mode = Mode::PersonaSwitchConfirm;
    }

    /// If the user confirmed a persona switch, return the target name and
    /// clear the confirm state. Returns `None` if there is no pending switch
    /// or the user picked cancel.
    pub fn take_confirmed_persona_switch(&mut self) -> Option<String> {
        let state = self.persona_switch_confirm.take()?;
        self.mode = Mode::Normal;
        if state.selected == 0 {
            Some(state.target.name)
        } else {
            None
        }
    }

    /// Move the focus between the two buttons in the confirm modal.
    pub fn persona_confirm_focus(&mut self, delta: i32) {
        if let Some(ref mut state) = self.persona_switch_confirm {
            let n = 2; // 0 = confirm, 1 = cancel
            let cur = state.selected as i32;
            let next = (cur + delta).rem_euclid(n) as usize;
            state.selected = next;
        }
    }

    /// Like `visual_to_byte_offset` but returns `None` instead of clamping
    /// to the last valid position when the target row is out of range.
    /// Used by `cursor_visual_up` / `cursor_visual_down` which need to
    /// detect edge cases.
    fn visual_to_byte_offset_opt(
        &self,
        visual_row: usize,
        visual_col: usize,
        content_width: u16,
    ) -> Option<usize> {
        let w = content_width.max(1) as usize;
        let mut current_row = 0;
        for (line_idx, line) in self.input.split('\n').enumerate() {
            let dw = unicode_width::UnicodeWidthStr::width(line);
            let rows_in_line = if w == 0 { 1 } else { dw.div_ceil(w).max(1) };
            if current_row + rows_in_line > visual_row {
                // Target is within this logical line.
                let row_in_line = visual_row - current_row;
                let target_byte_col = (row_in_line * w) + visual_col;
                // Find the byte offset at this display column.
                let mut display_col = 0;
                let mut byte_offset = 0;
                for (byte_idx, ch) in line.char_indices() {
                    if display_col >= target_byte_col {
                        break;
                    }
                    let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    display_col += ch_w;
                    byte_offset = byte_idx + ch.len_utf8();
                }
                // Compute the start of this logical line in the full input.
                let line_start = self
                    .input
                    .split('\n')
                    .take(line_idx)
                    .map(|l| l.len() + 1) // +1 for the '\n'
                    .sum::<usize>();
                return Some(line_start + byte_offset);
            }
            current_row += rows_in_line;
        }
        None
    }

    /// Map a (visual_row, visual_col) in the wrapped input grid to a byte
    /// offset in `self.input`. Used by mouse click handling to position the
    /// cursor at the clicked cell.
    pub fn visual_to_byte_offset(
        &self,
        visual_row: usize,
        visual_col: usize,
        content_width: u16,
    ) -> usize {
        let w = content_width.max(1) as usize;
        let mut current_visual_row = 0usize;
        let mut byte_offset = 0usize;
        for line in self.input.split('\n') {
            let dw = unicode_width::UnicodeWidthStr::width(line);
            let rows = if w == 0 { 1 } else { dw.div_ceil(w).max(1) };
            if visual_row < current_visual_row + rows {
                let row_in_line = visual_row - current_visual_row;
                let start_col = row_in_line * w;
                let col_clamped = visual_col.min(dw.saturating_sub(start_col));
                let target_col = start_col + col_clamped;
                let target_byte = byte_at_display_offset(line, target_col);
                return byte_offset + target_byte;
            }
            current_visual_row += rows;
            byte_offset += line.len() + 1; // +1 for the '\n'
        }
        self.input.len()
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

    /// Mark the cached chat render as stale so the next draw rebuilds it.
    /// Call from every mutator that changes what `draw_chat` would produce.
    /// Scroll mutators must NOT call this — they re-render from cache.
    pub fn mark_chat_dirty(&mut self) {
        self.chat_dirty = Some(self.chat_dirty.unwrap_or(0).wrapping_add(1));
    }

    /// Ensure `rendered_chat` is up to date for the current `chat_dirty`
    /// generation and width. Rebuilds only when stale — idle scroll frames
    /// (which don't bump `chat_dirty`) skip the rebuild and stay O(visible).
    /// `area_height` is needed to compute `max_scroll` so scroll mutators
    /// can clamp without rebuilding.
    pub fn ensure_chat_rendered(&mut self, md_width: u16, chat_width: u16, area_height: u16) {
        if md_width == 0 || chat_width == 0 {
            return;
        }
        let dirty = self.chat_dirty;
        let width_ok = self.rendered_chat_width == chat_width;
        let cache_ok = matches!(
            (&self.rendered_chat, &dirty, width_ok),
            (Some(_), Some(_), true)
        ) && self.rendered_chat.as_ref().map(|c| c.dirty_gen) == dirty;
        if !cache_ok {
            #[cfg(test)]
            {
                self.render_count += 1;
            }
            let built = crate::ui::chat::build_chat_lines(self, md_width, chat_width);
            let total_lines = built.lines.len() as u16;
            let max_scroll = total_lines.saturating_sub(area_height);
            self.max_scroll = max_scroll;
            // Publish the plain-text mirror for mouse selection + the
            // companion overlay. Only on rebuild — idle scroll frames skip
            // this clone, keeping them O(visible).
            self.chat_rows = built.chat_rows.clone();
            self.rendered_chat = Some(RenderedChat {
                lines: built.lines,
                chat_rows: built.chat_rows,
                total_wrapped: total_lines,
                max_scroll,
                dirty_gen: dirty.unwrap_or(0),
            });
            self.rendered_chat_width = chat_width;
            // Re-attach auto-scroll to the new bottom. While streaming we
            // always pin to the bottom; otherwise only if already anchored.
            if self.auto_scroll {
                self.scroll = max_scroll;
            }
        } else if let Some(ref mut rc) = self.rendered_chat {
            // Width and content are unchanged, but area_height may have
            // changed (resize). Recompute max_scroll cheaply.
            let max_scroll = rc.total_wrapped.saturating_sub(area_height);
            if rc.max_scroll != max_scroll {
                rc.max_scroll = max_scroll;
                self.max_scroll = max_scroll;
                if self.auto_scroll {
                    self.scroll = max_scroll;
                }
            }
        }
    }

    /// Conversation messages (read-only accessor).
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Push a message onto the display store and mark the chat dirty so the
    /// next render picks it up. All external message pushes should go through
    /// this method (or `push_synthetic_message` / `push_user`) — never
    /// `app.messages.push(...)` directly, which skips the dirty mark.
    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
        self.mark_chat_dirty();
    }

    /// Construct and push a user message from display text + attachments.
    pub fn push_user(&mut self, display: String, attachments: Vec<Part>) {
        let msg_id = ulid::Ulid::new();
        let mut parts = vec![Part::Text(mew_message::TextPart {
            base: mew_message::PartBase {
                id: ulid::Ulid::new(),
                message_id: msg_id,
                session_id: ulid::Ulid::new(),
            },
            text: display,
            synthetic: false,
        })];
        parts.extend(attachments);
        self.push_message(Message {
            id: msg_id,
            session_id: ulid::Ulid::new(),
            role: Role::User,
            parts,
            time: mew_message::Time {
                created: chrono::Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        });
    }

    /// Push a synthetic assistant message (e.g. `/cost` output, system
    /// alerts). Marks the chat dirty so it renders on the next draw.
    pub fn push_synthetic_message(&mut self, text: String) {
        let msg_id = ulid::Ulid::new();
        self.push_message(Message {
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
        self.md_render_cache.invalidate();
        self.pending_md_rerender = None;
        self.mark_chat_dirty();
    }

    /// Rewind the display to keep only the first `n` messages. Cleans up
    /// tool states and markdown cache for removed messages.
    pub fn rewind_to(&mut self, n: usize) {
        if n >= self.messages.len() {
            return;
        }
        self.messages.truncate(n);
        // Retain cache entries whose PartId belongs to a surviving message.
        let surviving: std::collections::HashSet<mew_message::PartId> = self
            .messages
            .iter()
            .flat_map(|m| m.parts.iter().map(|p| p.id()))
            .collect();
        self.rendered_md_cache
            .retain(|id, _| surviving.contains(id));
        self.pending_md_rerender = None;
        self.mark_chat_dirty();
    }

    /// Toggle bash output expansion.
    pub fn toggle_bash_expanded(&mut self) {
        self.bash_expanded = !self.bash_expanded;
        self.mark_chat_dirty();
    }

    /// Toggle reasoning/thinking block expansion.
    /// Toggles the last reasoning block in the chat. If none are visible,
    /// falls back to toggling all known reasoning blocks.
    pub fn toggle_reasoning_expanded(&mut self) {
        if let Some(&(id, _)) = self.reasoning_header_rows.last() {
            if self.reasoning_expanded.contains(&id) {
                self.reasoning_expanded.remove(&id);
            } else {
                self.reasoning_expanded.insert(id);
            }
            self.mark_chat_dirty();
        }
    }

    /// Insert an @-mention into the input, replacing the trigger `@` that
    /// opened the picker. Without this, the trigger `@` (already in the
    /// input) plus the mention's leading `@` produces `@@path`.
    pub fn insert_mention(&mut self, mention: &str) {
        if self.input.ends_with('@') {
            self.input.pop();
            self.cursor = self.cursor.saturating_sub(1);
        }
        self.input.push_str(mention);
        self.cursor += mention.len();
    }

    /// Return the task id of the most recently started running subagent, if
    /// any. Used by the `x` key to cancel the most recent in-flight subagent.
    pub fn most_recent_running_subagent(&self) -> Option<String> {
        self.subagents
            .iter()
            .rev()
            .find(|sa| matches!(sa.status, SubagentStatus::Running))
            .map(|sa| sa.task_id.clone())
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
        // Advance the status-bar marquee when pills overflow the width.
        // Only tick if the marquee is active (set when overflow is detected
        // during render). When not overflowing, `status_ticker_at` stays
        // `None` and this is a no-op.
        if let Some(last) = self.status_ticker_at {
            if last.elapsed() > Duration::from_millis(300) {
                self.status_ticker_offset = self.status_ticker_offset.wrapping_add(1);
                self.status_ticker_at = Some(Instant::now());
            }
        }
        // Advance the thinking spinner only while the agent is streaming.
        // Throttle to every Nth tick so the spinner doesn't spin too fast.
        if self.streaming {
            self.spinner_sub_tick = self.spinner_sub_tick.wrapping_add(1);
            if self.spinner_sub_tick >= SPINNER_TICK_DIVISOR {
                self.spinner_sub_tick = 0;
                self.spinner_frame = self.spinner_frame.wrapping_add(1);
            }
        }
        // Expire alerts.
        self.clear_expired_alerts();
        // Expire companion bubble.
        if let Some(since) = self.companion_bubble_since {
            // The TTL depends on the bubble text length, which we don't
            // have here. Use a conservative 10s upper bound; the render
            // path re-checks with the actual TTL.
            if since.elapsed() > Duration::from_secs(10) {
                self.companion_bubble_since = None;
            }
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

    /// Cycle to next slash command suggestion.
    pub fn slash_next(&mut self) {
        let filtered = self.filtered_slash_commands();
        if !filtered.is_empty() {
            self.slash_selected = (self.slash_selected + 1) % filtered.len();
            self.adjust_slash_scroll();
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
            self.adjust_slash_scroll();
        }
    }

    fn adjust_slash_scroll(&mut self) {
        let filtered = self.filtered_slash_commands();
        if filtered.is_empty() {
            return;
        }
        // Visible count: min(half the chat area height, 12). Falls back to
        // 3 when chat_area hasn't been populated yet (e.g. before first draw).
        let visible: usize = self.chat_area.height.checked_div(2).unwrap_or(3).min(12) as usize;
        let visible = visible.max(1);
        if self.slash_selected < self.slash_scroll {
            self.slash_scroll = self.slash_selected;
        } else if self.slash_selected >= self.slash_scroll + visible {
            self.slash_scroll = self.slash_selected.saturating_sub(visible - 1);
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
                // Auto-expand reasoning while streaming, auto-collapse when
                // the model moves on to text or a tool call.
                match &part {
                    Part::Reasoning(rp) => {
                        self.reasoning_started_at = Some(std::time::Instant::now());
                        self.reasoning_elapsed = None;
                        self.reasoning_active_id = Some(rp.base.id);
                        self.reasoning_expanded.insert(rp.base.id);
                    }
                    Part::Text(_) | Part::ToolCall(_) => {
                        if let Some(start) = self.reasoning_started_at.take() {
                            self.reasoning_elapsed = Some(start.elapsed());
                        }
                        if let Some(id) = self.reasoning_active_id.take() {
                            self.reasoning_expanded.remove(&id);
                        }
                    }
                    _ => {}
                }
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
                            self.md_render_cache.invalidate();
                        }
                        msg.parts.push(part);
                        self.mark_chat_dirty();
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
                self.md_render_cache.invalidate();
                self.mark_chat_dirty();
            }
            AgentEvent::Provider(ProviderEvent::PartDelta { part_id, delta, .. }) => {
                let mut is_text_delta = false;
                if let Some(msg) = self.messages.last_mut() {
                    if msg.role == Role::Assistant {
                        for part in &mut msg.parts {
                            if part.id() == part_id {
                                append_to_part(part, &delta);
                                is_text_delta = matches!(part, Part::Text(_));
                                break;
                            }
                        }
                    }
                }
                // Feed the delta into the incremental markdown stream only for
                // the text part it tracks. Reasoning deltas render separately
                // (dimmed) and must not pollute the stream — otherwise the
                // reasoning text appears twice: once as normal markdown here,
                // once via the Part::Reasoning render.
                if is_text_delta {
                    if let Some(ref mut stream) = self.md_stream {
                        let update = stream.append(&delta);
                        self.md_state.apply(update);
                    }
                }
                self.mark_chat_dirty();
            }
            AgentEvent::Provider(ProviderEvent::PartEnd { .. }) => {
                self.mark_chat_dirty();
            }
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
                self.mark_chat_dirty();
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
            AgentEvent::AskUser {
                call_id,
                questions,
                tx,
            } => {
                let answers = vec![String::new(); questions.len()];
                self.mode = Mode::UserQuestion;
                self.user_question = Some(UserQuestionState {
                    call_id,
                    questions,
                    answers,
                    page: 0,
                    selected: 0,
                    freeform_text: String::new(),
                    review: false,
                    review_selected: 0,
                    tx: Some(tx),
                });
            }
            AgentEvent::PlanApprovalRequest {
                call_id,
                plan_path,
                plan_markdown,
                persona,
                tx,
            } => {
                self.mode = Mode::PlanApproval;
                self.plan_approval = Some(PlanApprovalState {
                    call_id,
                    plan_path,
                    persona,
                    plan_markdown,
                    scroll: 0,
                    selected: 0,
                    editing_feedback: false,
                    feedback: String::new(),
                    tx: Some(tx),
                });
            }
            AgentEvent::TodosUpdated { todos } => {
                self.todos = todos;
            }
            AgentEvent::PersonaSwitchRequested { name } => {
                // Stash the requested name so the main loop can pick it up
                // and apply the switch. The main loop owns the agent
                // reference and the provider-builder plumbing, so it has
                // to be the one to do the actual swap.
                self.pending_persona_switch_apply = Some(name);
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
                self.mark_chat_dirty();
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
                self.mark_chat_dirty();
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
                self.mark_chat_dirty();
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
                self.mark_chat_dirty();
            }
            AgentEvent::Error(msg) => {
                self.streaming = false;
                self.esc_cancel_pending = None;
                self.ctrl_c_quit_pending = None;
                self.push_synthetic_message(format!("Error: {}", msg));
            }
            AgentEvent::SubagentStart {
                parent_call_id,
                name,
                display_name,
                ..
            } => {
                self.subagents.push(SubagentState {
                    task_id: parent_call_id.clone(),
                    name: name.clone(),
                    started_at: Instant::now(),
                    status: SubagentStatus::Running,
                    last_progress: None,
                    display_name: display_name.clone(),
                });
            }
            AgentEvent::SubagentProgress { .. } => {}
            AgentEvent::SubagentStatus {
                parent_call_id,
                message,
                ..
            } => {
                if let Some(sa) = self
                    .subagents
                    .iter_mut()
                    .find(|s| s.task_id == parent_call_id)
                {
                    sa.last_progress = Some(message);
                }
            }
            AgentEvent::SubagentEnd {
                parent_call_id,
                outcome,
                ..
            } => {
                if let Some(sa) = self
                    .subagents
                    .iter_mut()
                    .find(|s| s.task_id == parent_call_id)
                {
                    sa.status = match outcome {
                        mew_agent::SubagentOutcome::Completed => SubagentStatus::Completed,
                        mew_agent::SubagentOutcome::Cancelled => SubagentStatus::Cancelled,
                        mew_agent::SubagentOutcome::Failed { reason } => {
                            SubagentStatus::Failed { reason }
                        }
                    };
                }
            }
            AgentEvent::SubagentPermissionRequest { call, tx, .. } => {
                self.permission = Some(crate::app::PermissionState {
                    tool_name: call.tool_name.clone(),
                    call_id: call.call_id.clone(),
                    input: call.input.clone(),
                    tx: Some(tx),
                    selected: 0,
                });
                self.mode = crate::app::Mode::PermissionPrompt;
            }
            AgentEvent::WorkspacePermissionRequest { path, tx } => {
                self.permission = Some(crate::app::PermissionState {
                    tool_name: "workspace".into(),
                    call_id: String::new(),
                    input: serde_json::json!({"path": path.to_string_lossy()}),
                    tx: Some(tx),
                    selected: 0,
                });
                self.mode = crate::app::Mode::PermissionPrompt;
            }
            AgentEvent::JobUpdate {
                job_id,
                command,
                state,
            } => {
                let status = match state.as_str() {
                    "running" => BackgroundJobStatus::Running,
                    "completed" => BackgroundJobStatus::Completed,
                    "failed" => BackgroundJobStatus::Failed,
                    // "cancelled" or any unrecognized value.
                    _ => BackgroundJobStatus::Cancelled,
                };
                if let Some(job) = self.background_jobs.iter_mut().find(|j| j.job_id == job_id) {
                    // Existing entry: transition its state. Preserve the
                    // original started_at so the elapsed counter is stable.
                    job.status = status;
                } else {
                    // First update we've seen for this job.
                    self.background_jobs.push(BackgroundJobState {
                        job_id,
                        command,
                        started_at: Instant::now(),
                        status,
                    });
                }
            }
            AgentEvent::FileDelta { .. } => {
                // File delta accumulation is handled by the daemon for web
                // clients. The TUI doesn't show per-session diff stats yet.
            }
            AgentEvent::FlaggedFilesChanged { .. } => {
                // Flagged files visibility is handled by web frontends.
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

    /// Number of selectable rows on the current question page: author-supplied
    /// options + 1 implicit freeform row.
    fn user_question_row_count(uq: &UserQuestionState) -> usize {
        uq.questions
            .get(uq.page)
            .map(|q| q.options.len() + 1)
            .unwrap_or(1)
    }

    /// Move the highlight down by one row on the current page (wraps).
    pub fn user_question_select_next(&mut self) {
        if let Some(ref mut uq) = self.user_question {
            if uq.review {
                uq.review_selected = (uq.review_selected + 1) % 2;
            } else {
                let n = Self::user_question_row_count(uq);
                uq.selected = (uq.selected + 1) % n;
            }
        }
    }

    /// Move the highlight up by one row on the current page (wraps).
    pub fn user_question_select_prev(&mut self) {
        if let Some(ref mut uq) = self.user_question {
            if uq.review {
                uq.review_selected = if uq.review_selected == 0 { 1 } else { 0 };
            } else {
                let n = Self::user_question_row_count(uq);
                uq.selected = if uq.selected == 0 {
                    n - 1
                } else {
                    uq.selected - 1
                };
            }
        }
    }

    /// Move the highlight right on the review page (Submit / Cancel only).
    pub fn user_question_review_next(&mut self) {
        if let Some(ref mut uq) = self.user_question {
            if uq.review {
                uq.review_selected = (uq.review_selected + 1) % 2;
            }
        }
    }

    /// Move the highlight left on the review page.
    pub fn user_question_review_prev(&mut self) {
        if let Some(ref mut uq) = self.user_question {
            if uq.review {
                uq.review_selected = if uq.review_selected == 0 { 1 } else { 0 };
            }
        }
    }

    /// Jump directly to row N (1-indexed, for digit shortcuts).
    pub fn user_question_jump(&mut self, n: usize) {
        if let Some(ref mut uq) = self.user_question {
            let rows = Self::user_question_row_count(uq);
            if n >= 1 && n <= rows {
                uq.selected = n - 1;
            }
        }
    }

    /// Append a character to the freeform text when the freeform row is
    /// selected. No-op otherwise.
    pub fn user_question_type_char(&mut self, c: char) {
        if let Some(ref mut uq) = self.user_question {
            if !uq.review {
                let freeform_index = uq
                    .questions
                    .get(uq.page)
                    .map(|q| q.options.len())
                    .unwrap_or(0);
                if uq.selected == freeform_index {
                    uq.freeform_text.push(c);
                }
            }
        }
    }

    /// Delete the last character from the freeform text when the freeform row
    /// is selected. No-op otherwise.
    pub fn user_question_backspace(&mut self) {
        if let Some(ref mut uq) = self.user_question {
            if !uq.review {
                let freeform_index = uq
                    .questions
                    .get(uq.page)
                    .map(|q| q.options.len())
                    .unwrap_or(0);
                if uq.selected == freeform_index {
                    uq.freeform_text.pop();
                }
            }
        }
    }

    /// Commit the current selection. On the question page this saves the
    /// answer and advances (or goes to review for multi-question calls). On
    /// the review page this activates the highlighted action.
    pub fn user_question_confirm(&mut self) {
        enum Next {
            Advance,
            Submit,
        }
        let next = if let Some(ref mut uq) = self.user_question {
            if uq.review {
                match uq.review_selected {
                    0 => Next::Submit,
                    _ => {
                        // Cancel: drop without sending.
                        self.user_question = None;
                        self.mode = Mode::Normal;
                        return;
                    }
                }
            } else {
                let question = match uq.questions.get(uq.page) {
                    Some(q) => q,
                    None => return,
                };
                let answer = if uq.selected < question.options.len() {
                    question.options[uq.selected].label.clone()
                } else if !uq.freeform_text.is_empty() {
                    std::mem::take(&mut uq.freeform_text)
                } else {
                    // Freeform row picked but no text entered — ignore.
                    return;
                };
                uq.answers[uq.page] = answer;
                uq.freeform_text.clear();
                if uq.page + 1 < uq.questions.len() {
                    uq.page += 1;
                    uq.selected = 0;
                    Next::Advance
                } else if uq.questions.len() > 1 {
                    uq.review = true;
                    uq.review_selected = 0;
                    Next::Advance
                } else {
                    Next::Submit
                }
            }
        } else {
            return;
        };

        if matches!(next, Next::Submit) {
            self.submit_user_question();
        }
    }

    /// Final submit: send the answers and clear state.
    pub fn submit_user_question(&mut self) {
        if let Some(uq) = self.user_question.take() {
            if let Some(tx) = uq.tx {
                let _ = tx.send(uq.answers);
            }
        }
        self.mode = Mode::Normal;
    }

    /// Cancel the question prompt. Dropping `tx` without sending makes the
    /// agent's `rx.await` return `Err`, which the handler turns into a
    /// cancelled tool result.
    pub fn cancel_user_question(&mut self) {
        self.user_question = None;
        self.mode = Mode::Normal;
    }

    pub fn plan_approval_scroll_down(&mut self, lines: u16) {
        if let Some(pa) = self.plan_approval.as_mut() {
            pa.scroll = pa.scroll.saturating_add(lines);
        }
    }

    pub fn plan_approval_scroll_up(&mut self, lines: u16) {
        if let Some(pa) = self.plan_approval.as_mut() {
            pa.scroll = pa.scroll.saturating_sub(lines);
        }
    }

    /// Toggle between the approve (0) and request-changes (1) options.
    pub fn plan_approval_toggle(&mut self) {
        if let Some(pa) = self.plan_approval.as_mut() {
            if !pa.editing_feedback {
                pa.selected = if pa.selected == 0 { 1 } else { 0 };
            }
        }
    }

    pub fn plan_approval_type_char(&mut self, c: char) {
        if let Some(pa) = self.plan_approval.as_mut() {
            if pa.editing_feedback {
                pa.feedback.push(c);
            }
        }
    }

    pub fn plan_approval_backspace(&mut self) {
        if let Some(pa) = self.plan_approval.as_mut() {
            if pa.editing_feedback {
                pa.feedback.pop();
            }
        }
    }

    /// Confirm the current selection. Approve sends `Approved`. Request-changes
    /// enters the feedback editor first; once feedback is present a second
    /// confirm sends `ChangesRequested`.
    pub fn plan_approval_confirm(&mut self) {
        let ready = match self.plan_approval.as_mut() {
            Some(pa) if pa.selected == 1 && !pa.editing_feedback => {
                pa.editing_feedback = true;
                false
            }
            Some(pa) if pa.selected == 1 && pa.feedback.trim().is_empty() => {
                // Stay in the editor until there's something to send.
                false
            }
            Some(_) => true,
            None => false,
        };
        if !ready {
            return;
        }
        if let Some(pa) = self.plan_approval.take() {
            if let Some(tx) = pa.tx {
                let decision = if pa.selected == 0 {
                    mew_agent::PlanDecision::Approved
                } else {
                    mew_agent::PlanDecision::ChangesRequested(pa.feedback)
                };
                let _ = tx.send(decision);
            }
        }
        self.mode = Mode::Normal;
    }

    /// Cancel the plan approval. Dropping `tx` without sending makes the
    /// agent's `rx.await` return `Err`, which the handler turns into a
    /// cancelled tool result.
    pub fn cancel_plan_approval(&mut self) {
        self.plan_approval = None;
        self.mode = Mode::Normal;
    }
}

fn append_to_part(part: &mut Part, delta: &str) {
    match part {
        Part::Text(tp) => tp.text.push_str(delta),
        Part::Reasoning(rp) => rp.text.push_str(delta),
        _ => {}
    }
}

/// Current dir with `$HOME` collapsed to `~`, for the status pill.
fn short_cwd() -> String {
    let cwd = match std::env::current_dir() {
        Ok(p) => p.display().to_string(),
        Err(_) => return String::new(),
    };
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy().into_owned();
        if let Some(rest) = cwd.strip_prefix(&home) {
            return format!("~{}", rest);
        }
    }
    cwd
}

/// Walk up from cwd to find a `.git/HEAD` and parse the branch name.
/// Returns `None` outside a repo. Used for the status pill.
pub fn current_git_branch() -> Option<String> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let head = dir.join(".git").join("HEAD");
        if let Ok(content) = std::fs::read_to_string(&head) {
            let content = content.trim();
            if let Some(branch) = content.strip_prefix("ref: refs/heads/") {
                return Some(branch.to_string());
            }
            // Detached HEAD: content is a commit hash.
            if content.len() >= 7 {
                return Some(content[..7].to_string());
            }
            return Some(content.to_string());
        }
        dir = dir.parent()?.to_path_buf();
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn byte_at_display_offset(s: &str, target_col: usize) -> usize {
    let mut col = 0usize;
    for (i, ch) in s.char_indices() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if col + w > target_col {
            return i;
        }
        col += w;
    }
    s.len()
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
mod tests;

pub mod input;
pub mod pickers;
pub mod slash;
