use mew_agent::{AgentEvent, AskUserQuestion};
use mew_message::{Message, MessageId, Part, PartId, Role, ToolState};
use mew_provider::ProviderEvent;
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use mdstream;

/// Minimum terminal width to show the sidebar.
pub const SIDEBAR_MIN_WIDTH: u16 = 120;
/// Width of the sidebar in columns.
pub const SIDEBAR_WIDTH: u16 = 32;
/// Number of items visible in the picker list at once.
pub const PICKER_VISIBLE_ITEMS: usize = 8;
/// The spinner advances one frame every N ticks. At 16ms/tick, N=5 gives
/// ~80ms per frame — a ~800ms full cycle for 10 frames. Feels calm but alive.
const SPINNER_TICK_DIVISOR: u8 = 5;

/// A single slash command definition.
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
}

/// Result of handling a slash command.
#[derive(Debug)]
pub enum SlashResult {
    /// Command unknown; fall through to the model.
    Continue,
    Quit,
    Clear,
    /// Display a message in the chat pane.
    Message(String),
    /// Switch to a different model.
    SwitchModel(String),
    /// Switch to a persona by name, or clear with "default".
    SwitchPersona(String),
    /// Open a confirm modal for switching to the named persona. The actual
    /// switch only happens after the user confirms. Used by `/persona
    /// <name>` when the target differs from the currently active persona.
    /// Clearing the persona (via "default" / "none") bypasses the modal.
    PersonaSwitchConfirm(String),
    /// Open the model picker.
    OpenModelPicker,
    /// Resume a previous session by ID.
    ResumeSession(String),
    /// Toggle mouse capture on/off for native text selection.
    ToggleMouseCapture,
    /// Force context compaction.
    Compact,
    /// Show the session todo list.
    Todo,
    /// Rewind to keep only the first N messages.
    Rewind(usize),
    /// A plugin-registered slash command that needs dispatcher execution.
    PluginCommand {
        name: String,
        args: String,
    },
    /// Open the permission-mode picker (Standard / Dangerous!).
    PermissionModeMenu,
    /// Apply a permission-mode selection directly (used by `/permissions
    /// <mode>` and by the picker).
    SetPermissionMode(mew_hooks::PermissionMode),
    /// Set or clear the thinking variant (used by `/thinking <variant>`
    /// and `/thinking off`).
    SetThinkingVariant(String),
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
    /// Pending ask_user_question prompt, if any.
    pub user_question: Option<UserQuestionState>,
    /// Pending persona switch awaiting user confirmation. Set by the
    /// `/persona <name>` slash command when the target differs from the
    /// currently active persona; consumed by the confirm modal.
    pub persona_switch_confirm: Option<PersonaSwitchConfirmState>,
    /// Persona switch queued by the `switch_persona` tool and drained at
    /// end of turn. The TUI receives `AgentEvent::PersonaSwitchRequested`
    /// and stashes the name here; the main loop polls and applies the
    /// switch. We use a side-channel instead of returning an Action from
    /// `handle_agent_event` because that signature is shared with several
    /// other call sites that don't need to return.
    pub pending_persona_switch_apply: Option<String>,
    /// Current snapshot of the session todo list, for the sidebar pane.
    /// Populated on startup and refreshed via `AgentEvent::TodosUpdated`.
    pub todos: Vec<mew_agent::Todo>,
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
    /// Spinner frame index for the "thinking" indicator. Advances on each
    /// `tick()` while `streaming` is true.
    pub spinner_frame: usize,
    /// Sub-tick counter used to slow down the spinner. The spinner advances
    /// every `SPINNER_TICK_DIVISOR` ticks so it doesn't spin too fast.
    spinner_sub_tick: u8,
    /// Whether to exit the application.
    pub should_quit: bool,
    /// Context files loaded for this session.
    pub context_files: Vec<String>,
    /// Available tool names.
    pub tools: Vec<String>,
    /// Available personas (name, description).
    pub personas: Vec<(String, String)>,
    /// Active persona name, if any.
    pub active_persona: Option<String>,
    /// Active persona's explicit accent color (hex string like "#ff8800").
    /// `None` = use the deterministic color generated from `active_persona`.
    pub active_persona_color: Option<String>,
    /// Current permission mode (Standard / Dangerous!). Mirrors the agent's
    /// runtime mode so the TUI can render the picker state, the status-line
    /// badge, and alert the user when it changes.
    pub permission_mode: mew_hooks::PermissionMode,
    /// MCP server status: (name, connected, tool_count)
    pub mcp_status: Vec<(String, bool, usize)>,
    /// Available subagent names and descriptions for @-mention.
    pub subagent_names: Vec<(String, String)>,
    /// Sidebar section collapsed state: section name → collapsed.
    pub sidebar_collapsed: std::collections::HashMap<String, bool>,
    /// y-positions of sidebar section headers from last render (for click detection).
    pub sidebar_header_rows: Vec<(u16, String)>,
    /// Sidebar area rect from last render.
    pub sidebar_rect: Rect,
    /// Available models (id -> description) for the palette.
    pub models: Vec<(String, String)>,
    /// Active picker, if any.
    pub picker: Option<PickerState>,
    /// Selected slash command suggestion index.
    pub slash_selected: usize,
    /// Scroll offset for slash autocomplete.
    pub slash_scroll: usize,
    /// Whether bash output is expanded (shows all lines vs last 10).
    pub bash_expanded: bool,
    /// Set of reasoning part IDs that are expanded. The currently-streaming
    /// reasoning block is auto-added; user-toggled blocks persist here.
    pub reasoning_expanded: HashSet<PartId>,
    /// PartId of the reasoning block currently being streamed (auto-expanded).
    pub reasoning_active_id: Option<PartId>,
    /// When the current reasoning block started, for elapsed display.
    pub reasoning_started_at: Option<std::time::Instant>,
    /// Elapsed time of the last completed reasoning block.
    pub reasoning_elapsed: Option<std::time::Duration>,
    /// Visual rows (indices into chat_rows) of reasoning block headers,
    /// populated during render so mouse clicks can toggle individual blocks.
    pub reasoning_header_rows: Vec<(PartId, usize)>,
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
    /// Whether mouse capture (scroll/clicks) is enabled instead of native selection.
    pub mouse_capture: bool,
    /// Chat area position from last render, for mouse click mapping.
    pub chat_area: Rect,
    /// Input area position from last render, for mouse click mapping.
    pub input_area: Rect,
    /// Start row (visual row in chat_rows) of drag selection.
    pub sel_anchor_row: Option<usize>,
    /// Start column (byte offset within the anchor row) of drag selection.
    pub sel_anchor_col: Option<usize>,
    /// End row of drag selection.
    pub sel_end_row: Option<usize>,
    /// End column of drag selection.
    pub sel_end_col: Option<usize>,
    /// Text of each rendered visual row from the last frame, for selection copy.
    pub chat_rows: Vec<String>,
    /// Temporary alert text to display (e.g. "copied 42 chars").
    pub alert: Option<(String, Instant)>,
    /// Toast queue: small notifications that stack in the bottom-right.
    /// Each toast has a 3s TTL. Up to 3 visible at once; older ones
    /// are dropped. `set_alert` pushes to this queue (and also sets
    /// `alert` for backward compat with the existing render path).
    pub toasts: Vec<(String, Instant)>,
    /// Plugin UI content pushed via PluginHost::set_ui. Keys are namespaced
    /// as "plugin-name/key". Rendered beside the input prompt.
    pub plugin_ui: std::collections::HashMap<String, String>,
    /// Slash commands registered by plugins via on_register_slash_commands.
    pub dynamic_slash_commands: Vec<SlashCommand>,
    /// When the companion speech bubble was set (for auto-dismiss).
    pub companion_bubble_since: Option<Instant>,
    /// Marquee offset for the status bar when pills overflow the width.
    pub status_ticker_offset: usize,
    pub status_ticker_at: Option<Instant>,
    /// Cached git branch for the status pill (resolved lazily on first draw).
    pub git_branch: Option<String>,
    pub git_branch_resolved: bool,
    /// Shortened cwd (home → `~`) for the status pill.
    pub short_cwd: String,
    /// Running subagent tasks for sidebar display.
    pub subagents: Vec<SubagentState>,
    /// Background shell jobs, kept in sync via `AgentEvent::JobUpdate`.
    /// Each entry represents either a running or recently-finished job.
    pub background_jobs: Vec<BackgroundJobState>,
    /// Undo stack for the input editor. Each entry is a snapshot of the
    /// input buffer + cursor before a mutation. Capped at 100 entries.
    pub undo_stack: Vec<(String, usize)>,
    /// Redo stack for the input editor. Populated on undo, cleared on
    /// any new mutation.
    pub redo_stack: Vec<(String, usize)>,
    /// Timestamp of the last undo-push. Used for coalescing consecutive
    /// same-type edits within a 500ms window into one undo entry.
    pub last_undo_push: Option<Instant>,
    /// Search query for Ctrl+R history search.
    pub history_search_query: String,
    /// Index into filtered history results for Ctrl+R search.
    pub history_search_index: Option<usize>,
    /// Saved input before entering history search (restored on cancel).
    pub history_search_saved: Option<(String, usize)>,
    /// Pending large paste awaiting user confirmation. When set, the TUI
    /// shows "paste N chars? [y/N]" and only inserts on 'y'.
    pub pending_paste: Option<String>,
}

/// A running or completed subagent task shown in the sidebar.
#[derive(Debug, Clone)]
pub struct SubagentState {
    pub task_id: String,
    pub name: String,
    pub started_at: Instant,
    pub status: SubagentStatus,
    /// Most recent status message from the subagent (via `progress_update`).
    /// `None` until the subagent reports its first status.
    pub last_progress: Option<String>,
    /// Human-friendly per-run name (e.g. "Curie"), picked by the runner
    /// at spawn time. `None` if the runner didn't set one.
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
    /// Modal showing the diff between the active persona and a target
    /// persona. The user must confirm or cancel before any switch happens.
    PersonaSwitchConfirm,
    /// Keyboard shortcuts overlay. Toggled by pressing `?`. Shows all
    /// available shortcuts grouped by category.
    Help,
    /// Reverse search through input history. Entered via Ctrl+R.
    /// Shows a search prompt; typing filters history. Enter inserts
    /// the match. Esc cancels.
    HistorySearch,
    /// Confirmation prompt for a large paste (>2000 chars). Shows
    /// "paste N chars? [y/N]". 'y' inserts, any other key cancels.
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
    /// What the picker is selecting (e.g. "command", "model").
    pub kind: String,
    pub items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
    pub cursor: usize,
    /// First visible item index in the filtered list.
    pub scroll: usize,
    /// How many items fit in the visible area (updated by draw_picker).
    pub visible_items: usize,
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
    /// Channel sender for the decision.
    pub tx: Option<tokio::sync::oneshot::Sender<mew_hooks::PermissionDecision>>,
    /// Currently selected option (0=allow once, 1=session, 2=deny).
    pub selected: usize,
}

/// A pending persona switch awaiting user confirmation. Populated by the
/// `/persona <name>` slash command when the target differs from the active
/// persona, and consumed (applied or discarded) by the confirm modal.
#[derive(Debug, Clone)]
pub struct PersonaSwitchConfirmState {
    /// Snapshot of the persona the user requested.
    pub target: PersonaSummary,
    /// Snapshot of the currently active persona, if any.
    pub current: Option<PersonaSummary>,
    /// Index of the focused button (0 = confirm, 1 = cancel).
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
    /// Allowlist of tool names. `None` = all tools (subject to deny).
    pub tools: Option<Vec<String>>,
    /// Denylist of tool names. `None` or empty = no denials.
    pub tools_deny: Option<Vec<String>>,
    /// Allowlist of skill names. `None` = all skills.
    pub skills: Option<Vec<String>>,
    /// Explicit accent color as hex string (e.g. "#ff8800"). `None` = use
    /// the deterministic color generated from the persona name.
    pub color: Option<String>,
}

/// A pending `ask_user_question` prompt. Shown as a one-question-per-page
/// inline overlay replacing the input box, with multiple-choice options
/// and an implicit "type your own" final option.
#[derive(Debug)]
pub struct UserQuestionState {
    pub call_id: String,
    pub questions: Vec<AskUserQuestion>,
    /// One entry per question; populated as the user commits each page.
    pub answers: Vec<String>,
    /// Index of the question currently being shown.
    pub page: usize,
    /// Index of the highlighted row in the current question (0..=options+1).
    /// The final index is the implicit "type your own" freeform option.
    pub selected: usize,
    /// In-progress text when `selected` points at the freeform row.
    pub freeform_text: String,
    /// True when the user has committed all questions and is reviewing.
    pub review: bool,
    /// Selected action on the review page: 0 = Submit, 1 = Cancel.
    pub review_selected: usize,
    pub tx: Option<tokio::sync::oneshot::Sender<Vec<String>>>,
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
            user_question: None,
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
            picker: None,
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
            rendered_md_cache: HashMap::new(),
            last_md_width: 0,
            max_scroll: 0,
            esc_cancel_pending: None,
            ctrl_c_quit_pending: None,
            retry_status: None,
            mouse_capture: true,
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
                id: "thinking-variant".into(),
                label: "Thinking Variant".into(),
                description: "Set reasoning effort (high, max, off)".into(),
            },
            PickerItem {
                id: "settings".into(),
                label: "Settings".into(),
                description: "Configure mew (providers, plugins)".into(),
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
            visible_items: PICKER_VISIBLE_ITEMS,
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
            visible_items: PICKER_VISIBLE_ITEMS,
        });
    }

    /// Open the permission-mode picker (Standard / Permissive / Auto /
    /// Auto+ / Dangerous!). Marks the currently active mode with a
    /// trailing "● active" suffix so the user can see what's currently
    /// in effect before selecting.
    pub fn open_permission_mode_picker(&mut self) {
        let active = self.permission_mode;
        let marker = |m: mew_hooks::PermissionMode| -> &'static str {
            if m == active {
                " ● active"
            } else {
                ""
            }
        };
        let items = vec![
            PickerItem {
                id: mew_hooks::PermissionMode::Standard.id().into(),
                label: format!("Standard{}", marker(mew_hooks::PermissionMode::Standard)),
                description: "Prompts for Mutating/Dangerous tools. Default.".into(),
            },
            PickerItem {
                id: mew_hooks::PermissionMode::Permissive.id().into(),
                label: format!(
                    "Permissive{}",
                    marker(mew_hooks::PermissionMode::Permissive)
                ),
                description: "Auto-allows Mutating tools (write/edit/etc.). \
                              Still prompts for bash and respects your deny rules, \
                              ask rules, and secret-file guard."
                    .into(),
            },
            PickerItem {
                id: mew_hooks::PermissionMode::Auto.id().into(),
                label: format!("Auto{}", marker(mew_hooks::PermissionMode::Auto)),
                description: "Routes every tool call through a small/cheap LLM \
                              classifier. Classifier returns allow/deny/escalate; \
                              escalate falls back to the user modal. Skip the \
                              prompts — let the model decide. Requires a \
                              classifier provider to be configured."
                    .into(),
            },
            PickerItem {
                id: mew_hooks::PermissionMode::AutoPlus.id().into(),
                label: format!("Auto+{}", marker(mew_hooks::PermissionMode::AutoPlus)),
                description: "Like Auto, but the classifier CANNOT escalate. \
                              Escalate or any classifier failure → Deny (fail \
                              closed). Hands-off but uncertainty means no. \
                              Use when you trust the model more than your \
                              own attention but don't want a provider outage \
                              to silently run destructive tools."
                    .into(),
            },
            PickerItem {
                id: mew_hooks::PermissionMode::Dangerous.id().into(),
                label: format!("Dangerous!{}", marker(mew_hooks::PermissionMode::Dangerous)),
                description: "Every tool auto-runs. Overrides deny rules, ask rules, \
                              secret-file guard, bash decomposition. Pure bypass — \
                              you've said \"don't ask me anything, even the things I \
                              said don't do.\" Output redaction still applies."
                    .into(),
            },
        ];
        // Pre-select the active mode so Enter on an unchanged picker is a no-op.
        let pre_selected = match active {
            mew_hooks::PermissionMode::Standard => 0,
            mew_hooks::PermissionMode::Permissive => 1,
            mew_hooks::PermissionMode::Auto => 2,
            mew_hooks::PermissionMode::AutoPlus => 3,
            mew_hooks::PermissionMode::Dangerous => 4,
        };
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "permission_mode".into(),
            items,
            filter: String::new(),
            selected: pre_selected,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
        });
    }

    /// Close the picker and return to normal mode.
    pub fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
    }

    /// Open a thinking variant picker. Shows available variants for the
    /// current model, plus an "Off" option.
    pub fn open_thinking_variant_picker(&mut self) {
        let mut items = vec![PickerItem {
            id: "off".into(),
            label: "Off".into(),
            description: "Disable thinking/reasoning".into(),
        }];
        // Variant names come from the model list; we stored them as
        // "provider/model" → thinking_variants in the models vec.
        // The models vec stores (id, description) pairs. We need to find
        // the current model's variants. Since we don't have direct access
        // to the catalog here, we rely on the daemon/main loop having
        // populated app.models with variant info encoded in the description.
        // For now, use common variant names as static options.
        for name in &["high", "max", "thinking"] {
            items.push(PickerItem {
                id: name.to_string(),
                label: name.to_string(),
                description: format!("{} thinking effort", name),
            });
        }
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "thinking_variant".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
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

    /// Push the current input state to the undo stack. Called before every
    /// input mutation. Coalesces consecutive edits within 500ms into one
    /// undo entry so Ctrl+Z undoes word-by-word, not char-by-char.
    pub fn push_undo(&mut self) {
        let now = Instant::now();
        let should_coalesce = self
            .last_undo_push
            .map(|t| now.duration_since(t) < Duration::from_millis(500))
            .unwrap_or(false);
        if !should_coalesce {
            self.undo_stack.push((self.input.clone(), self.cursor));
            if self.undo_stack.len() > 100 {
                self.undo_stack.remove(0);
            }
        }
        self.last_undo_push = Some(now);
        // Any new mutation clears the redo stack.
        self.redo_stack.clear();
    }

    /// Undo the last input mutation. Restores the previous state and pushes
    /// the current state to the redo stack.
    pub fn undo(&mut self) {
        if let Some((prev_input, prev_cursor)) = self.undo_stack.pop() {
            self.redo_stack.push((self.input.clone(), self.cursor));
            self.input = prev_input;
            self.cursor = prev_cursor;
            self.last_undo_push = None;
        }
    }

    /// Redo the last undone input mutation.
    pub fn redo(&mut self) {
        if let Some((next_input, next_cursor)) = self.redo_stack.pop() {
            self.undo_stack.push((self.input.clone(), self.cursor));
            self.input = next_input;
            self.cursor = next_cursor;
            self.last_undo_push = None;
        }
    }

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
                p.adjust_scroll();
            }
        }
    }

    /// Move picker selection up.
    pub fn picker_up(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.selected = p.selected.saturating_sub(1);
            p.adjust_scroll();
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
    /// Also includes subagent names if they match the prefix.
    pub fn open_file_picker(&mut self, prefix: &str) {
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut items: Vec<PickerItem> = Vec::new();

        let prefix_lower = prefix.to_lowercase();

        for (name, description) in &self.subagent_names {
            if name.to_lowercase().contains(&prefix_lower) {
                items.push(PickerItem {
                    id: name.clone(),
                    label: format!("@{} [subagent]", name),
                    description: description.clone(),
                });
            }
        }

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
            if !rel.to_lowercase().contains(&prefix_lower) {
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
            visible_items: PICKER_VISIBLE_ITEMS,
        });
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.push_undo();
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Insert a newline at cursor position.
    pub fn insert_newline(&mut self) {
        self.push_undo();
        self.input.insert(self.cursor, '\n');
        self.cursor += 1;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.push_undo();
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
            self.push_undo();
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

    /// Move the cursor up one visual line (respecting word wrap). Returns
    /// true if the cursor moved (i.e. we weren't on the first visual line).
    /// Returns false if already on the first visual line — the caller can
    /// then fall through to history navigation.
    pub fn cursor_visual_up(&mut self, content_width: u16) -> bool {
        let (row, col) = self.cursor_visual_row_col(content_width);
        if row == 0 {
            return false;
        }
        // Move to the same column on the previous visual row.
        let target_row = row - 1;
        if let Some(offset) = self.visual_to_byte_offset_opt(target_row, col, content_width) {
            self.cursor = offset;
        }
        true
    }

    /// Move the cursor down one visual line. Returns true if the cursor
    /// moved (i.e. we weren't on the last visual line). Returns false if
    /// already on the last visual line — the caller can fall through to
    /// history navigation.
    pub fn cursor_visual_down(&mut self, content_width: u16) -> bool {
        let (row, col) = self.cursor_visual_row_col(content_width);
        let total = self.input_visual_line_count(content_width);
        if row >= total.saturating_sub(1) {
            return false;
        }
        let target_row = row + 1;
        if let Some(offset) = self.visual_to_byte_offset_opt(target_row, col, content_width) {
            self.cursor = offset;
        }
        true
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

    /// Number of lines in the input.
    pub fn input_line_count(&self) -> usize {
        self.input.lines().count()
    }

    /// Visual row count for the input when wrapped to `content_width`
    /// columns (the width available for text, after the border and any
    /// prefix). Each logical line contributes `ceil(display_width / width)`
    /// rows, minimum 1.
    pub fn input_visual_line_count(&self, content_width: u16) -> usize {
        let w = content_width.max(1) as usize;
        self.input
            .split('\n')
            .map(|line| {
                let dw = unicode_width::UnicodeWidthStr::width(line);
                dw.div_ceil(w).max(1)
            })
            .sum()
    }

    /// Return (visual_row, visual_col) for the cursor when the input is
    /// wrapped to `content_width` columns. `visual_row` is the index into
    /// the wrapped visual grid; `visual_col` is the column within that row.
    pub fn cursor_visual_row_col(&self, content_width: u16) -> (usize, usize) {
        let w = content_width.max(1) as usize;
        let (logical_line, col_in_line) = self.cursor_line_col();
        let mut visual_row = 0;
        for (li, line) in self.input.split('\n').enumerate() {
            if li == logical_line {
                let dw = unicode_width::UnicodeWidthStr::width(line);
                let col_clamped = col_in_line.min(dw);
                let row_in_line = col_clamped.checked_div(w).unwrap_or(0);
                return (visual_row + row_in_line, col_clamped - row_in_line * w);
            }
            let dw = unicode_width::UnicodeWidthStr::width(line);
            let rows = if w == 0 { 1 } else { dw.div_ceil(w).max(1) };
            visual_row += rows;
        }
        (visual_row, 0)
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
        // Re-attach auto-scroll so the user sees the response.
        self.auto_scroll = true;
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

    pub fn push_synthetic_message(&mut self, text: String) {
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

    /// Rewind the display to keep only the first `n` messages. Cleans up
    /// tool states and markdown cache for removed messages.
    pub fn rewind_to(&mut self, n: usize) {
        if n >= self.messages.len() {
            return;
        }
        self.messages.truncate(n);
        self.rendered_md_cache
            .retain(|id, _| self.messages.iter().any(|m| m.id == *id));
        self.pending_md_rerender = None;
    }

    /// Toggle bash output expansion.
    pub fn toggle_bash_expanded(&mut self) {
        self.bash_expanded = !self.bash_expanded;
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

    /// Available built-in slash commands.
    pub fn builtin_slash_commands() -> Vec<SlashCommand> {
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
                name: "/todo".into(),
                description: "show the session todo list".into(),
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
                name: "/thinking".into(),
                description: "set thinking variant (e.g. /thinking high)".into(),
            },
            SlashCommand {
                name: "/persona".into(),
                description: "switch persona (e.g. /persona researcher)".into(),
            },
            SlashCommand {
                name: "/permissions".into(),
                description: "switch permission mode (Standard or Dangerous!)".into(),
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
            SlashCommand {
                name: "/rewind".into(),
                description: "rewind to an earlier point (e.g. /rewind 3)".into(),
            },
            SlashCommand {
                name: "/mouse".into(),
                description: "toggle mouse capture for text selection".into(),
            },
        ]
    }

    /// All slash commands including dynamic ones from plugins.
    pub fn all_slash_commands(&self) -> Vec<SlashCommand> {
        let mut cmds = Self::builtin_slash_commands();
        cmds.extend(self.dynamic_slash_commands.clone());
        cmds
    }

    /// Register dynamic slash commands from a plugin.
    pub fn add_dynamic_slash_commands(&mut self, cmds: Vec<SlashCommand>) {
        self.dynamic_slash_commands.extend(cmds);
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
            "/todo" => SlashResult::Todo,
            "/cost" => SlashResult::Message(self.build_cost_report()),
            "/help" => SlashResult::Message(self.build_help()),
            "/model" => {
                if let Some(id) = arg {
                    SlashResult::SwitchModel(id.to_string())
                } else {
                    SlashResult::OpenModelPicker
                }
            }
            "/thinking" => {
                if let Some(variant) = arg {
                    SlashResult::SetThinkingVariant(variant.trim().to_string())
                } else {
                    SlashResult::Message(
                        "usage: /thinking <variant> — e.g. /thinking high, /thinking max, /thinking off".into(),
                    )
                }
            }
            "/persona" => {
                if let Some(name) = arg {
                    // Clearing the persona is idempotent and unambiguous; do
                    // it directly. Switching to the currently active persona
                    // is a no-op. Any other switch goes through the confirm
                    // modal so the user sees the model/toolset diff.
                    if name == "default" || name == "none" {
                        SlashResult::SwitchPersona(name.to_string())
                    } else if self.active_persona.as_deref() == Some(name) {
                        SlashResult::Message(format!("persona '{name}' is already active"))
                    } else {
                        SlashResult::PersonaSwitchConfirm(name.to_string())
                    }
                } else {
                    let mut out = String::from("available personas:\n");
                    if self.personas.is_empty() {
                        out.push_str("  (none — create .mew/personas/<name>/PERSONA.md)");
                    } else {
                        for (name, desc) in &self.personas {
                            let active = if self.active_persona.as_deref() == Some(name.as_str()) {
                                " *"
                            } else {
                                ""
                            };
                            out.push_str(&format!("  {} — {}{}\n", name, desc, active));
                        }
                    }
                    SlashResult::Message(out)
                }
            }
            "/permissions" => {
                // `/permissions`        → open the picker
                // `/permissions standard|permissive|auto|auto_plus|dangerous` → switch directly
                if let Some(arg) = arg {
                    let mode = arg.trim();
                    match mew_hooks::PermissionMode::from_id(mode) {
                        Some(m) => SlashResult::SetPermissionMode(m),
                        None => SlashResult::Message(format!(
                            "unknown permission mode '{mode}'; expected 'standard', 'permissive', 'auto', 'auto_plus', or 'dangerous'"
                        )),
                    }
                } else {
                    SlashResult::PermissionModeMenu
                }
            }
            "/sessions" => SlashResult::Message(self.build_sessions_list()),
            "/mouse" | "/m" => SlashResult::ToggleMouseCapture,
            "/resume" => {
                if let Some(id) = arg {
                    SlashResult::ResumeSession(id.to_string())
                } else {
                    SlashResult::Message("usage: /resume <session-id>".into())
                }
            }
            "/rewind" => {
                if let Some(n_str) = arg {
                    match n_str.parse::<usize>() {
                        Ok(n) => SlashResult::Rewind(n),
                        Err(_) => SlashResult::Message(
                            "usage: /rewind <n> — n is the number of messages to keep".into(),
                        ),
                    }
                } else {
                    SlashResult::Message(self.build_rewind_list())
                }
            }
            _ => {
                // Check dynamic/plugin commands.
                if let Some(dyn_cmd) = self.dynamic_slash_commands.iter().find(|c| c.name == cmd) {
                    let args = arg.unwrap_or("").to_string();
                    SlashResult::PluginCommand {
                        name: dyn_cmd.name.clone(),
                        args,
                    }
                } else {
                    SlashResult::Continue
                }
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

    fn build_help(&self) -> String {
        let mut out = String::from("commands:\n");
        for cmd in self.all_slash_commands() {
            out.push_str(&format!("  {:12}  {}\n", cmd.name, cmd.description));
        }
        out.trim_end().to_string()
    }

    fn build_rewind_list(&self) -> String {
        if self.messages.is_empty() {
            return "no messages to rewind.".into();
        }
        let mut out = String::from("messages (keep 0..n with /rewind <n>):\n");
        let total = self.messages.len();
        let start = total.saturating_sub(15);
        for (i, msg) in self.messages.iter().enumerate() {
            if i < start {
                continue;
            }
            let role = match msg.role {
                mew_message::Role::User => "user",
                mew_message::Role::Assistant => "asst",
            };
            let snippet = msg
                .parts
                .iter()
                .find_map(|p| match p {
                    mew_message::Part::Text(tp) => Some(tp.text.as_str()),
                    _ => None,
                })
                .unwrap_or("(no text)")
                .chars()
                .take(60)
                .collect::<String>();
            out.push_str(&format!("  [{}] {} {:<60}\n", i, role, snippet));
        }
        out.push_str(&format!("\ntotal: {} messages", total));
        out
    }

    fn build_sessions_list(&self) -> String {
        use std::time::UNIX_EPOCH;
        let dir = mew_session::session_dir();
        let mut out = String::from("sessions:\n");
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                // Top-level sessions are folders containing `session.jsonl`.
                // Subagent sessions are nested under `<parent>/subagents/<id>/`
                // and intentionally hidden from the top-level list.
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
                    out.push_str(&format!("  {}  ({} bytes)\n", name, size));
                }
                if folders.is_empty() {
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
        let all = self.all_slash_commands();
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
        // Visible count matches the inner area in draw_slash_autocomplete:
        // cap to 5 total, minus 2 padding = 3.
        let visible: usize = 3;
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

/// Find the byte offset into `s` that corresponds to the given display column.
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

    #[test]
    fn test_builtin_slash_commands_has_core_commands() {
        let cmds = App::builtin_slash_commands();
        let names: Vec<&str> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"/help"));
        assert!(names.contains(&"/clear"));
        assert!(names.contains(&"/quit"));
        assert!(names.contains(&"/permissions"));
    }

    #[test]
    fn test_permissions_slash_with_no_arg_opens_picker() {
        let app = App::new();
        let result = app.handle_slash("/permissions");
        assert!(matches!(result, SlashResult::PermissionModeMenu));
    }

    #[test]
    fn test_permissions_slash_with_dangerous_arg() {
        let app = App::new();
        let result = app.handle_slash("/permissions dangerous");
        assert!(matches!(
            result,
            SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Dangerous)
        ));
    }

    #[test]
    fn test_permissions_slash_with_standard_arg() {
        let app = App::new();
        let result = app.handle_slash("/permissions standard");
        assert!(matches!(
            result,
            SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Standard)
        ));
    }

    #[test]
    fn test_permissions_slash_with_permissive_arg() {
        let app = App::new();
        let result = app.handle_slash("/permissions permissive");
        assert!(matches!(
            result,
            SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Permissive)
        ));
    }

    #[test]
    fn test_permissions_slash_with_auto_arg() {
        let app = App::new();
        let result = app.handle_slash("/permissions auto");
        assert!(matches!(
            result,
            SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Auto)
        ));
    }

    #[test]
    fn test_permissions_slash_with_auto_plus_arg() {
        let app = App::new();
        let result = app.handle_slash("/permissions auto_plus");
        assert!(matches!(
            result,
            SlashResult::SetPermissionMode(mew_hooks::PermissionMode::AutoPlus)
        ));
    }

    #[test]
    fn test_permission_mode_picker_has_five_items() {
        let mut app = App::new();
        app.open_permission_mode_picker();
        let picker = app.picker.as_ref().unwrap();
        assert_eq!(picker.items.len(), 5, "picker should show all five modes");
        let ids: Vec<&str> = picker.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["standard", "permissive", "auto", "auto_plus", "dangerous",],
            "modes ordered from most-restrictive to least"
        );
    }

    #[test]
    fn test_permission_mode_picker_preselects_autoplus() {
        let mut app = App::new();
        app.permission_mode = mew_hooks::PermissionMode::AutoPlus;
        app.open_permission_mode_picker();
        let picker = app.picker.as_ref().unwrap();
        assert_eq!(
            picker.selected, 3,
            "AutoPlus should pre-select index 3 (between Auto and Dangerous!)"
        );
        let autoplus = picker.items.iter().find(|i| i.id == "auto_plus").unwrap();
        assert!(
            autoplus.label.contains("● active"),
            "active marker on Auto+ row: {:?}",
            autoplus.label
        );
    }

    #[test]
    fn test_permission_mode_picker_has_three_items() {
        // Back-compat shim — kept as a 3-item check to make the picker
        // expansion to Auto visible. The new comprehensive test is
        // `test_permission_mode_picker_has_four_items` below.
        let mut app = App::new();
        app.open_permission_mode_picker();
        let picker = app.picker.as_ref().unwrap();
        assert!(
            picker.items.len() >= 3,
            "picker should show at least three modes"
        );
    }

    #[test]
    fn test_permission_mode_picker_has_four_items() {
        // Back-compat shim — kept so a refactor that drops AutoPlus still
        // catches the regression. The comprehensive test is
        // `test_permission_mode_picker_has_five_items` below.
        let mut app = App::new();
        app.open_permission_mode_picker();
        let picker = app.picker.as_ref().unwrap();
        assert!(
            picker.items.len() >= 4,
            "picker should show at least four modes"
        );
    }

    #[test]
    fn test_permission_mode_picker_preselects_permissive() {
        let mut app = App::new();
        app.permission_mode = mew_hooks::PermissionMode::Permissive;
        app.open_permission_mode_picker();
        let picker = app.picker.as_ref().unwrap();
        assert_eq!(
            picker.selected, 1,
            "Permissive should pre-select index 1 (middle item)"
        );
        let permissive = picker.items.iter().find(|i| i.id == "permissive").unwrap();
        assert!(
            permissive.label.contains("● active"),
            "active marker on Permissive row: {:?}",
            permissive.label
        );
    }

    #[test]
    fn test_permissions_slash_with_unknown_arg_errors() {
        let app = App::new();
        let result = app.handle_slash("/permissions banana");
        assert!(matches!(result, SlashResult::Message(_)));
    }

    #[test]
    fn test_permission_mode_picker_marks_active_mode() {
        let mut app = App::new();
        app.permission_mode = mew_hooks::PermissionMode::Dangerous;
        app.open_permission_mode_picker();
        let picker = app.picker.as_ref().expect("picker opened");
        assert_eq!(picker.kind, "permission_mode");
        let dangerous = picker
            .items
            .iter()
            .find(|i| i.id == "dangerous")
            .expect("dangerous item");
        assert!(
            dangerous.label.contains("● active"),
            "active mode should be marked: {:?}",
            dangerous.label
        );
        let standard = picker.items.iter().find(|i| i.id == "standard").unwrap();
        assert!(!standard.label.contains("● active"));
    }

    #[test]
    fn test_permission_mode_picker_preselects_active() {
        let mut app = App::new();
        app.permission_mode = mew_hooks::PermissionMode::Dangerous;
        app.open_permission_mode_picker();
        let picker = app.picker.as_ref().unwrap();
        assert_eq!(
            picker.selected, 4,
            "Dangerous index should be 4 (fifth item in slider with Auto and Auto+)"
        );
    }

    #[test]
    fn test_all_slash_commands_includes_dynamic() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/buddy".into(),
            description: "pet companion".into(),
        }]);
        let all = app.all_slash_commands();
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"/buddy"));
        assert!(names.contains(&"/help"));
    }

    #[test]
    fn test_all_slash_commands_no_dynamic_still_has_builtins() {
        let app = App::new();
        let all = app.all_slash_commands();
        let builtins = App::builtin_slash_commands();
        assert_eq!(all.len(), builtins.len());
    }

    #[test]
    fn test_handle_slash_routes_dynamic_command() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/buddy".into(),
            description: "pet companion".into(),
        }]);
        let result = app.handle_slash("/buddy pet");
        match result {
            SlashResult::PluginCommand { name, args } => {
                assert_eq!(name, "/buddy");
                assert_eq!(args, "pet");
            }
            _ => panic!("expected PluginCommand, got {:?}", result),
        }
    }

    #[test]
    fn test_handle_slash_unknown_dynamic_command_continues() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/buddy".into(),
            description: "pet companion".into(),
        }]);
        let result = app.handle_slash("/nonexistent");
        assert!(matches!(result, SlashResult::Continue));
    }

    #[test]
    fn test_handle_slash_dynamic_command_without_args() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/stats".into(),
            description: "show stats".into(),
        }]);
        let result = app.handle_slash("/stats");
        match result {
            SlashResult::PluginCommand { name, args } => {
                assert_eq!(name, "/stats");
                assert_eq!(args, "");
            }
            _ => panic!("expected PluginCommand"),
        }
    }

    #[test]
    fn test_add_dynamic_slash_commands_accumulates() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/foo".into(),
            description: "first".into(),
        }]);
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/bar".into(),
            description: "second".into(),
        }]);
        let all = app.all_slash_commands();
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"/foo"));
        assert!(names.contains(&"/bar"));
    }

    #[test]
    fn test_filtered_slash_commands_includes_dynamic() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/buddy".into(),
            description: "pet companion".into(),
        }]);
        app.input = "/bud".to_string();
        let filtered = app.filtered_slash_commands();
        assert!(!filtered.is_empty());
        assert!(filtered.iter().any(|c| c.name == "/buddy"));
    }

    #[test]
    fn test_plugin_ui_starts_empty() {
        let app = App::new();
        assert!(app.plugin_ui.is_empty());
    }

    #[test]
    fn test_plugin_ui_can_store_values() {
        let mut app = App::new();
        app.plugin_ui
            .insert("buddy/sprite".into(), "(\u{b7}>".into());
        assert_eq!(app.plugin_ui.get("buddy/sprite").unwrap(), "(\u{b7}>");
    }

    #[test]
    fn test_filtered_slash_commands_no_match_returns_empty() {
        let app = App::new();
        // Set input to a prefix that matches nothing
        let mut app = app;
        app.input = "/zzz".to_string();
        let filtered = app.filtered_slash_commands();
        assert!(filtered.is_empty(), "no commands should match /zzz");
    }

    #[test]
    fn test_filtered_slash_commands_non_slash_no_results() {
        let mut app = App::new();
        app.input = "hello".to_string();
        let filtered = app.filtered_slash_commands();
        assert!(filtered.is_empty(), "non-slash input returns empty");
    }

    #[test]
    fn test_build_help_includes_dynamic_commands() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![SlashCommand {
            name: "/buddy".into(),
            description: "pet companion".into(),
        }]);
        let help = app.build_help();
        assert!(help.contains("/buddy"), "help should list /buddy: {help}");
        assert!(
            help.contains("pet companion"),
            "help should include description"
        );
    }

    #[test]
    fn test_build_help_no_dynamic_commands_works() {
        let app = App::new();
        let help = app.build_help();
        assert!(help.contains("/help"), "help should list built-in commands");
        assert!(help.contains("/clear"), "help should list /clear");
    }

    #[test]
    fn test_dynamic_slash_commands_uses_all_slash_for_autocomplete() {
        let mut app = App::new();
        app.add_dynamic_slash_commands(vec![
            SlashCommand {
                name: "/buddy".into(),
                description: "buddy".into(),
            },
            SlashCommand {
                name: "/stats".into(),
                description: "stats".into(),
            },
        ]);
        app.input = "/bu".to_string();
        let filtered = app.filtered_slash_commands();
        assert!(!filtered.is_empty());
        let names: Vec<&str> = filtered.iter().map(|c| c.name.as_str()).collect();
        assert!(
            !names.contains(&"/stats"),
            "/stats should not match /bu prefix"
        );
    }

    #[test]
    fn test_subagent_status_event_stores_progress() {
        use mew_agent::AgentEvent;
        let mut app = App::new();

        // Subagent starts.
        app.handle_agent_event(AgentEvent::SubagentStart {
            parent_call_id: "task-1".into(),
            name: "researcher".into(),
            child_session_id: "child-1".into(),
            display_name: Some("Curie".into()),
        });
        assert_eq!(app.subagents.len(), 1);
        assert_eq!(app.subagents[0].display_name.as_deref(), Some("Curie"));
        assert!(app.subagents[0].last_progress.is_none());

        // Subagent reports its first status.
        app.handle_agent_event(AgentEvent::SubagentStatus {
            parent_call_id: "task-1".into(),
            tool_name: "progress_update".into(),
            message: "scanning the repo".into(),
        });
        assert_eq!(
            app.subagents[0].last_progress.as_deref(),
            Some("scanning the repo")
        );

        // A second status replaces the first.
        app.handle_agent_event(AgentEvent::SubagentStatus {
            parent_call_id: "task-1".into(),
            tool_name: "progress_update".into(),
            message: "writing the report".into(),
        });
        assert_eq!(
            app.subagents[0].last_progress.as_deref(),
            Some("writing the report")
        );

        // A status for an unknown subagent is ignored.
        app.handle_agent_event(AgentEvent::SubagentStatus {
            parent_call_id: "no-such-task".into(),
            tool_name: "progress_update".into(),
            message: "ignored".into(),
        });
        assert_eq!(
            app.subagents[0].last_progress.as_deref(),
            Some("writing the report")
        );
    }

    #[test]
    fn test_subagent_start_without_display_name_falls_back() {
        use mew_agent::AgentEvent;
        let mut app = App::new();

        // Older callers (or callers that opt out) may not set a
        // display_name. The state should still record the entry, with
        // display_name == None, and the sidebar's "fall back to def
        // name" path takes over.
        app.handle_agent_event(AgentEvent::SubagentStart {
            parent_call_id: "task-1".into(),
            name: "researcher".into(),
            child_session_id: "child-1".into(),
            display_name: None,
        });
        assert_eq!(app.subagents.len(), 1);
        assert_eq!(app.subagents[0].name, "researcher");
        assert!(app.subagents[0].display_name.is_none());
    }

    #[test]
    fn test_ask_user_event_stores_state_and_sets_mode() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, _rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![
                AskUserQuestion {
                    prompt: "which branch?".into(),
                    options: vec![
                        QuestionOption {
                            label: "main".into(),
                            description: "production".into(),
                        },
                        QuestionOption {
                            label: "dev".into(),
                            description: "".into(),
                        },
                    ],
                },
                AskUserQuestion {
                    prompt: "confirm?".into(),
                    options: vec![
                        QuestionOption {
                            label: "yes".into(),
                            description: "".into(),
                        },
                        QuestionOption {
                            label: "no".into(),
                            description: "".into(),
                        },
                    ],
                },
            ],
            tx,
        });
        assert_eq!(app.mode, Mode::UserQuestion);
        let uq = app.user_question.as_ref().expect("question stored");
        assert_eq!(uq.questions.len(), 2);
        assert_eq!(uq.questions[0].prompt, "which branch?");
        assert_eq!(uq.questions[0].options[0].label, "main");
        assert!(uq.answers.iter().all(|a| a.is_empty()));
        assert_eq!(uq.page, 0);
        assert_eq!(uq.selected, 0);
        assert!(!uq.review);
    }

    #[test]
    fn test_single_question_picks_option_and_submits() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![AskUserQuestion {
                prompt: "branch?".into(),
                options: vec![
                    QuestionOption {
                        label: "main".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "dev".into(),
                        description: "".into(),
                    },
                ],
            }],
            tx,
        });
        // Move highlight to "dev" and confirm. Single question auto-submits.
        app.user_question_select_next();
        app.user_question_confirm();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.user_question.is_none());
        let answers = rx.try_recv().expect("answers sent");
        assert_eq!(answers, vec!["dev"]);
    }

    #[test]
    fn test_multi_question_goes_to_review_before_submit() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![
                AskUserQuestion {
                    prompt: "branch?".into(),
                    options: vec![
                        QuestionOption {
                            label: "main".into(),
                            description: "".into(),
                        },
                        QuestionOption {
                            label: "dev".into(),
                            description: "".into(),
                        },
                    ],
                },
                AskUserQuestion {
                    prompt: "scope?".into(),
                    options: vec![
                        QuestionOption {
                            label: "minimal".into(),
                            description: "".into(),
                        },
                        QuestionOption {
                            label: "wide".into(),
                            description: "".into(),
                        },
                    ],
                },
            ],
            tx,
        });
        // First question: pick "dev" (next from 0), confirm.
        app.user_question_select_next();
        app.user_question_confirm();
        let uq = app.user_question.as_ref().expect("still active");
        assert_eq!(uq.page, 1);
        assert_eq!(uq.selected, 0);
        assert!(!uq.review);
        // Second question: pick "wide" (next twice), confirm. Multi-question
        // should now go to the review page rather than submit.
        app.user_question_select_next();
        app.user_question_confirm();
        let uq = app.user_question.as_ref().expect("still active");
        assert!(uq.review, "should be on the review page");
        assert_eq!(uq.review_selected, 0);
        assert_eq!(uq.answers, vec!["dev", "wide"]);
        assert!(rx.try_recv().is_err(), "should not have submitted yet");
        // Confirm Submit on the review page.
        app.user_question_confirm();
        assert_eq!(app.mode, Mode::Normal);
        let answers = rx.try_recv().expect("answers sent");
        assert_eq!(answers, vec!["dev", "wide"]);
    }

    #[test]
    fn test_freeform_text_commits_via_typing() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![AskUserQuestion {
                prompt: "branch?".into(),
                options: vec![
                    QuestionOption {
                        label: "main".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "dev".into(),
                        description: "".into(),
                    },
                ],
            }],
            tx,
        });
        // Two options + freeform = 3 rows. Jump to row 3.
        app.user_question_jump(3);
        app.user_question_type_char('f');
        app.user_question_type_char('o');
        app.user_question_type_char('o');
        app.user_question_backspace();
        app.user_question_confirm();
        let answers = rx.try_recv().expect("answers sent");
        assert_eq!(answers, vec!["fo"]);
    }

    #[test]
    fn test_freeform_does_not_advance_with_empty_text() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, _rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![AskUserQuestion {
                prompt: "branch?".into(),
                options: vec![
                    QuestionOption {
                        label: "main".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "dev".into(),
                        description: "".into(),
                    },
                ],
            }],
            tx,
        });
        app.user_question_jump(3);
        app.user_question_confirm();
        // Should still be on the same page; nothing sent.
        let uq = app.user_question.as_ref().expect("still active");
        assert_eq!(uq.page, 0);
        assert_eq!(uq.selected, 2);
    }

    #[test]
    fn test_review_cancel_drops_state() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![
                AskUserQuestion {
                    prompt: "a".into(),
                    options: vec![
                        QuestionOption {
                            label: "x".into(),
                            description: "".into(),
                        },
                        QuestionOption {
                            label: "y".into(),
                            description: "".into(),
                        },
                    ],
                },
                AskUserQuestion {
                    prompt: "b".into(),
                    options: vec![
                        QuestionOption {
                            label: "x".into(),
                            description: "".into(),
                        },
                        QuestionOption {
                            label: "y".into(),
                            description: "".into(),
                        },
                    ],
                },
            ],
            tx,
        });
        app.user_question_confirm();
        app.user_question_confirm();
        // Now on the review page. Move to Cancel and confirm.
        app.user_question_review_next();
        app.user_question_confirm();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.user_question.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_typing_only_affects_freeform_row() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, _rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![AskUserQuestion {
                prompt: "branch?".into(),
                options: vec![
                    QuestionOption {
                        label: "main".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "dev".into(),
                        description: "".into(),
                    },
                ],
            }],
            tx,
        });
        // With the first option highlighted, typing should be a no-op.
        app.user_question_type_char('z');
        let uq = app.user_question.as_ref().unwrap();
        assert!(uq.freeform_text.is_empty());
    }

    #[test]
    fn test_cancel_user_question_drops_without_sending() {
        use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
        let mut app = App::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
        app.handle_agent_event(AgentEvent::AskUser {
            call_id: "c1".into(),
            questions: vec![AskUserQuestion {
                prompt: "q".into(),
                options: vec![
                    QuestionOption {
                        label: "a".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "b".into(),
                        description: "".into(),
                    },
                ],
            }],
            tx,
        });
        assert_eq!(app.mode, Mode::UserQuestion);
        app.cancel_user_question();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.user_question.is_none());
        // Sender was dropped without sending → the receiver sees a disconnect,
        // which the agent handler turns into a "cancelled" tool result.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_todos_updated_event_stores_snapshot() {
        use mew_agent::{AgentEvent, Todo, TodoStatus};
        let mut app = App::new();
        assert!(app.todos.is_empty());
        app.handle_agent_event(AgentEvent::TodosUpdated {
            todos: vec![
                Todo {
                    id: 1,
                    content: "write tests".into(),
                    status: TodoStatus::Done,
                    depends_on: vec![],
                },
                Todo {
                    id: 2,
                    content: "ship".into(),
                    status: TodoStatus::InProgress,
                    depends_on: vec![1],
                },
            ],
        });
        assert_eq!(app.todos.len(), 2);
        assert_eq!(app.todos[0].id, 1);
        assert_eq!(app.todos[1].status, TodoStatus::InProgress);
    }

    #[test]
    fn test_insert_mention_replaces_trigger_at() {
        // The '@' that opens the picker is already in the input; the picked
        // mention carries its own '@'. Without replacement you get '@@path'.
        let mut app = App::new();
        app.input = "@".to_string();
        app.cursor = 1;
        app.insert_mention("@src/main.rs");
        assert_eq!(app.input, "@src/main.rs");
        assert_eq!(app.cursor, "@src/main.rs".len());

        // Mid-sentence: the trigger '@' is still the last char when picked.
        app.input = "see @".to_string();
        app.cursor = app.input.len();
        app.insert_mention("@lib.rs ");
        assert_eq!(app.input, "see @lib.rs ");
    }

    #[test]
    fn test_input_visual_line_count_no_wrap() {
        let mut app = App::new();
        app.input = "short\nlines".to_string();
        assert_eq!(app.input_visual_line_count(80), 2);
    }

    #[test]
    fn test_input_visual_line_count_wraps_long_line() {
        let mut app = App::new();
        app.input = "a".repeat(25);
        // 25 chars at width 10 -> ceil(25/10) = 3 rows
        assert_eq!(app.input_visual_line_count(10), 3);
    }

    #[test]
    fn test_input_visual_line_count_empty_line_counts_as_one() {
        let mut app = App::new();
        app.input = "short\n\n".to_string() + &"a".repeat(25);
        // 1 + 1 + 3 = 5 rows at width 10
        assert_eq!(app.input_visual_line_count(10), 5);
    }

    #[test]
    fn test_cursor_visual_row_col_no_wrap() {
        let mut app = App::new();
        app.input = "hello\nworld".to_string();
        app.cursor = 8; // byte 8 is 'r' in "world" -> line 1, col 2
        assert_eq!(app.cursor_visual_row_col(80), (1, 2));
    }

    #[test]
    fn test_cursor_visual_row_col_wraps() {
        let mut app = App::new();
        app.input = "a".repeat(25);
        app.cursor = 22; // char 22, which is on visual row 2 (chars 20-29), col 2
        assert_eq!(app.cursor_visual_row_col(10), (2, 2));
    }

    #[test]
    fn test_visual_to_byte_offset_first_row() {
        let mut app = App::new();
        app.input = "hello world".to_string();
        // visual row 0, visual col 6 -> byte offset 6 (the 'w')
        assert_eq!(app.visual_to_byte_offset(0, 6, 80), 6);
    }

    #[test]
    fn test_visual_to_byte_offset_wrapped_row() {
        let mut app = App::new();
        app.input = "a".repeat(25);
        // visual row 1, visual col 5 -> char 15 -> byte 15
        assert_eq!(app.visual_to_byte_offset(1, 5, 10), 15);
    }

    #[test]
    fn test_visual_to_byte_offset_past_end() {
        let mut app = App::new();
        app.input = "hi".to_string();
        // visual row 5 is past the end -> return input.len()
        assert_eq!(app.visual_to_byte_offset(5, 0, 80), 2);
    }

    #[test]
    fn test_persona_slash_command_with_name_returns_confirm() {
        let mut app = App::new();
        app.personas = vec![("researcher".into(), "read-only".into())];
        let result = app.handle_slash("/persona researcher");
        // Real switches go through the confirm modal so the user sees
        // the model/toolset diff before applying.
        assert!(matches!(
            result,
            crate::app::SlashResult::PersonaSwitchConfirm(ref n) if n == "researcher"
        ));
    }

    #[test]
    fn test_persona_slash_command_default_returns_direct_clear() {
        let app = App::new();
        // "default" / "none" bypass the confirm modal — they're idempotent.
        let result = app.handle_slash("/persona default");
        assert!(matches!(result, crate::app::SlashResult::SwitchPersona(ref n) if n == "default"));
    }

    #[test]
    fn test_persona_slash_command_same_as_active_returns_message() {
        let mut app = App::new();
        app.personas = vec![("researcher".into(), "read-only".into())];
        app.active_persona = Some("researcher".into());
        let result = app.handle_slash("/persona researcher");
        // Switching to the active persona is a no-op; the slash handler
        // returns an info message rather than opening the confirm modal.
        assert!(matches!(result, crate::app::SlashResult::Message(_)));
    }

    #[test]
    fn test_persona_slash_command_no_arg_lists() {
        let mut app = App::new();
        app.personas = vec![
            ("researcher".into(), "read-only".into()),
            ("executor".into(), "writes code".into()),
        ];
        app.active_persona = Some("researcher".into());
        let result = app.handle_slash("/persona");
        match result {
            crate::app::SlashResult::Message(msg) => {
                assert!(msg.contains("researcher"));
                assert!(msg.contains("executor"));
                assert!(msg.contains("read-only"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn test_persona_slash_command_empty_personas() {
        let app = App::new();
        let result = app.handle_slash("/persona");
        match result {
            crate::app::SlashResult::Message(msg) => {
                assert!(msg.contains("none"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn test_rewind_slash_command_returns_rewind() {
        let app = App::new();
        let result = app.handle_slash("/rewind 2");
        assert!(matches!(result, SlashResult::Rewind(2)));
    }

    #[test]
    fn test_rewind_slash_command_invalid_arg() {
        let app = App::new();
        let result = app.handle_slash("/rewind abc");
        match result {
            SlashResult::Message(msg) => assert!(msg.contains("usage")),
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn test_rewind_slash_command_no_arg_lists() {
        let mut app = App::new();
        // Add a message so the list isn't empty.
        let msg = mew_message::Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: mew_message::Role::User,
            parts: vec![mew_message::Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                text: "hello world".into(),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        app.messages.push(msg);
        let result = app.handle_slash("/rewind");
        match result {
            SlashResult::Message(msg) => {
                assert!(msg.contains("hello world"));
                assert!(msg.contains("total: 1"));
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn test_rewind_slash_empty_messages() {
        let app = App::new();
        let result = app.handle_slash("/rewind");
        match result {
            SlashResult::Message(msg) => assert!(msg.contains("no messages")),
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn test_rewind_to_truncates_messages() {
        let mut app = App::new();
        for i in 0..5 {
            let id = ulid::Ulid::new();
            let msg = mew_message::Message {
                id,
                session_id: ulid::Ulid::new(),
                role: mew_message::Role::User,
                parts: vec![mew_message::Part::Text(mew_message::TextPart {
                    base: mew_message::PartBase {
                        id: ulid::Ulid::new(),
                        message_id: id,
                        session_id: ulid::Ulid::new(),
                    },
                    text: format!("msg {}", i),
                    synthetic: false,
                })],
                time: mew_message::Time {
                    created: 0,
                    completed: None,
                },
                assistant: None,
            };
            app.messages.push(msg);
            app.rendered_md_cache
                .insert(id, (80, format!("msg {}", i), std::rc::Rc::new(vec![])));
        }
        assert_eq!(app.messages.len(), 5);
        assert_eq!(app.rendered_md_cache.len(), 5);

        app.rewind_to(2);
        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.rendered_md_cache.len(), 2);
    }

    #[test]
    fn test_rewind_to_noop_when_n_too_large() {
        let mut app = App::new();
        app.messages.push(mew_message::Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: mew_message::Role::User,
            parts: vec![],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        });
        app.rewind_to(10);
        assert_eq!(app.messages.len(), 1);
    }

    #[test]
    fn test_undo_redo_basic() {
        let mut app = App::new();
        assert!(app.input.is_empty());

        // Type "hello" — coalesces into one undo entry.
        app.insert_char('h');
        app.insert_char('e');
        app.insert_char('l');
        app.insert_char('l');
        app.insert_char('o');
        assert_eq!(app.input, "hello");

        // Undo restores to empty (coalesced entry).
        app.undo();
        assert_eq!(app.input, "");
        assert!(app.undo_stack.is_empty());

        // Redo restores "hello".
        app.redo();
        assert_eq!(app.input, "hello");
    }

    #[test]
    fn test_undo_after_backspace() {
        let mut app = App::new();
        app.insert_char('a');
        app.insert_char('b');
        // Wait past the coalesce window so backspace is its own entry.
        std::thread::sleep(std::time::Duration::from_millis(600));
        app.backspace(); // removes 'b'
        assert_eq!(app.input, "a");

        app.undo();
        assert_eq!(app.input, "ab");
    }

    #[test]
    fn test_redo_cleared_on_new_edit() {
        let mut app = App::new();
        app.insert_char('x');
        std::thread::sleep(std::time::Duration::from_millis(600));
        app.insert_char('y');
        app.undo();
        app.undo();
        assert_eq!(app.input, "");
        assert!(!app.redo_stack.is_empty());

        // New edit clears redo stack.
        app.insert_char('z');
        assert!(app.redo_stack.is_empty());
        assert_eq!(app.input, "z");
    }

    #[test]
    fn test_undo_paste_single_entry() {
        let mut app = App::new();
        // Simulate a paste by pushing undo once then inserting multiple chars.
        app.push_undo();
        for c in "pasted".chars() {
            app.input.insert(app.cursor, c);
            app.cursor += c.len_utf8();
        }
        assert_eq!(app.input, "pasted");
        assert_eq!(app.undo_stack.len(), 1); // single entry, not 6

        app.undo();
        assert_eq!(app.input, "");
    }

    #[test]
    fn test_toast_queue_pushes_and_expires() {
        let mut app = App::new();
        assert!(app.toasts.is_empty());

        app.set_alert("copied 42 chars");
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].0, "copied 42 chars");

        app.set_alert("model switched");
        assert_eq!(app.toasts.len(), 2);

        app.set_alert("third toast");
        app.set_alert("fourth toast");
        // Cap at 3 visible.
        assert_eq!(app.toasts.len(), 3);
        assert_eq!(app.toasts[0].0, "model switched"); // oldest dropped

        // Expiry: simulate passage of time by manually setting old timestamps.
        let old = Instant::now() - Duration::from_secs(5);
        for toast in &mut app.toasts {
            toast.1 = old;
        }
        app.clear_expired_alerts();
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn test_history_search_finds_matches() {
        let mut app = App::new();
        app.history.push("cargo build".into());
        app.history.push("cargo test".into());
        app.history.push("git status".into());

        app.start_history_search();
        assert_eq!(app.mode, Mode::HistorySearch);

        // Search for "cargo" → 2 matches (newest first).
        for c in "cargo".chars() {
            app.history_search_query.push(c);
        }
        app.history_search_index = Some(0);
        let matches = app.history_search_matches();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], "cargo test"); // newest first

        // Current match should be "cargo test".
        assert_eq!(
            app.history_search_current_match(),
            Some("cargo test".to_string())
        );

        // Confirm: input should be set to the match.
        app.history_search_confirm();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.input, "cargo test");
    }

    #[test]
    fn test_history_search_cancel_restores() {
        let mut app = App::new();
        app.input = "partial text".into();
        app.cursor = app.input.len();

        app.start_history_search();
        assert_eq!(app.mode, Mode::HistorySearch);

        // Cancel restores saved input.
        app.history_search_cancel();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.input, "partial text");
    }

    #[test]
    fn test_history_search_no_match() {
        let mut app = App::new();
        app.history.push("hello world".into());

        app.start_history_search();
        app.history_search_query = "xyz".into();
        app.history_search_index = Some(0);

        let matches = app.history_search_matches();
        assert!(matches.is_empty());
        assert_eq!(app.history_search_current_match(), None);
    }
}
