use super::*;

pub(super) const BUNDLED_MONO_REGULAR: &[u8] =
    include_bytes!("../../../../crates/mew-raster/assets/IoskeleyMono-Regular.ttf");
pub(super) const BUNDLED_MONO_MEDIUM: &[u8] =
    include_bytes!("../../../../crates/mew-raster/assets/IoskeleyMono-Medium.ttf");

pub(super) enum DesktopClientEvent {
    Connected,
    Updated {
        events: Vec<ClientEvent>,
        state: Box<ClientState>,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DesktopConnectionProfile {
    LocalWebSocket {
        url: String,
    },
    RemoteIroh {
        node_id: String,
        pairing_token: Option<String>,
        device_name: String,
    },
}

impl DesktopConnectionProfile {
    pub(super) fn endpoint_label(&self) -> String {
        match self {
            Self::LocalWebSocket { url } => url.clone(),
            Self::RemoteIroh { node_id, .. } => format!("iroh://{node_id}"),
        }
    }
}

pub(super) struct DesktopShell {
    pub(super) model: ShellModel,
    pub(super) theme: Theme,
    pub(super) theme_mode: DesktopThemeMode,
    pub(super) light_theme: String,
    pub(super) dark_theme: String,
    pub(super) open_tabs: Vec<OpenConversationTab>,
    pub(super) active_tab: Option<usize>,
    pub(super) tab_scroll_handle: gpui::ScrollHandle,
    pub(super) layout: ShellLayoutState,
    pub(super) sidebar_rows: Vec<SidebarRow>,
    pub(super) sidebar_list: gpui::ListState,
    pub(super) collapsed_groups: BTreeSet<String>,
    pub(super) session_view_states: BTreeMap<String, SessionViewState>,
    pub(super) session_menu_session: Option<String>,
    pub(super) hovered_group: Option<String>,
    pub(super) hovered_session: Option<String>,
    pub(super) drag_over_group: Option<String>,
    pub(super) rename_session_id: Option<String>,
    pub(super) rename_draft: String,
    pub(super) rename_focus_handle: FocusHandle,
    pub(super) popover_focus_handle: FocusHandle,
    pub(super) rename_selection: Range<usize>,
    pub(super) rename_selection_reversed: bool,
    pub(super) rename_marked_range: Option<Range<usize>>,
    pub(super) sidebar_animation_id: u64,
    pub(super) workbench_animation_id: u64,
    pub(super) terminal_animation_id: u64,
    pub(super) workbench_width: f32,
    pub(super) auxiliary_view: AuxiliaryView,
    pub(super) transcript_list: gpui::ListState,
    pub(super) transcript_rows: Vec<TranscriptRenderRow>,
    pub(super) transcript_rows_append_only: bool,
    pub(super) markdown_cache: Vec<Vec<CachedMarkdown>>,
    pub(super) tool_text_lists: RefCell<BTreeMap<String, gpui::ListState>>,
    pub(super) tool_text_cache: RefCell<BTreeMap<String, ToolTextCache>>,
    pub(super) transcript_text_registry: Rc<RefCell<Vec<TranscriptTextEntry>>>,
    pub(super) transcript_selection: Option<TranscriptSelection>,
    pub(super) transcript_selection_anchor: Option<TranscriptSelectionPoint>,
    pub(super) transcript_is_selecting: bool,
    pub(super) transcript_selected_text: Option<String>,
    pub(super) review_diffs: Vec<FileDiff>,
    pub(super) review_lines: Vec<DiffLine>,
    pub(super) review_selected_file: Option<usize>,
    pub(super) review_line_list: gpui::ListState,
    pub(super) review_signature: Option<String>,
    pub(super) review_loading: bool,
    pub(super) review_error: Option<String>,
    pub(super) input_animation_id: u64,
    pub(super) composer_focus_handle: FocusHandle,
    pub(super) composer_selection: Range<usize>,
    pub(super) composer_selection_reversed: bool,
    pub(super) composer_marked_range: Option<Range<usize>>,
    pub(super) composer_is_selecting: bool,
    pub(super) composer_bounds: Option<Bounds<Pixels>>,
    pub(super) browser_url_focus_handle: FocusHandle,
    pub(super) browser_url_focus_requested: bool,
    pub(super) browser_url_selection: Range<usize>,
    pub(super) browser_url_selection_reversed: bool,
    pub(super) browser_url_marked_range: Option<Range<usize>>,
    pub(super) browser_url_bounds: Option<Bounds<Pixels>>,
    pub(super) browser_native_rect: Option<BrowserRect>,
    pub(super) model_picker_bounds: Option<Bounds<Pixels>>,
    pub(super) persona_picker_bounds: Option<Bounds<Pixels>>,
    pub(super) permission_picker_bounds: Option<Bounds<Pixels>>,
    pub(super) thinking_picker_bounds: Option<Bounds<Pixels>>,
    pub(super) slash_menu_dismissed: bool,
    pub(super) slash_menu_index: usize,
    pub(super) mention_menu_dismissed: bool,
    pub(super) mention_menu_index: usize,
    pub(super) file_tree_entries: BTreeMap<String, Vec<DirEntry>>,
    pub(super) file_tree_expanded: BTreeSet<String>,
    pub(super) file_tree_pending: BTreeSet<String>,
    pub(super) watching_workspace_session: Option<String>,
    pub(super) prompt_history: PromptHistory,
    pub(super) plan_feedback_request: Option<String>,
    pub(super) composer_cursor_visible: bool,
    pub(super) composer_blink_epoch: usize,
    pub(super) streaming_render_scheduled: bool,
    pub(super) expanded_chat_parts: BTreeSet<String>,
    pub(super) pending_prompt: Option<String>,
    pub(super) pending_attachments: Vec<Attachment>,
    pub(super) attachments: Vec<Attachment>,
    pub(super) attachment_error: Option<String>,
    pub(super) pending_model: Option<(String, String)>,
    pub(super) awaiting_model_switch: Option<(String, String)>,
    pub(super) pending_session_request: bool,
    pub(super) pending_session_target: Option<String>,
    pub(super) model_picker_open: bool,
    pub(super) persona_picker_open: bool,
    pub(super) permission_picker_open: bool,
    pub(super) thinking_picker_open: bool,
    pub(super) terminal_font_picker_open: bool,
    pub(super) terminal_font_family: String,
    pub(super) terminal_view: Entity<TerminalView>,
    pub(super) terminal_id: Option<String>,
    pub(super) terminal_status: String,
    pub(super) browser_portal: Option<BrowserPortal>,
    pub(super) browser_panel_open: bool,
    pub(super) browser_initialization_pending: bool,
    pub(super) browser_url: String,
    pub(super) browser_title: String,
    pub(super) browser_error: Option<String>,
    pub(super) browser_pump_epoch: u64,
    pub(super) browser_pump: Option<Entity<BrowserPumpView>>,
    pub(super) settings_open: bool,
    pub(super) settings_page: SettingsPage,
    pub(super) connection_picker_open: bool,
    pub(super) remote_profiles: Vec<mew_config::DesktopRemoteProfile>,
    pub(super) connection_profile_selection: Option<String>,
    pub(super) command_tx: Option<UnboundedSender<ClientMessage>>,
    pub(super) connection_profile: Option<DesktopConnectionProfile>,
    pub(super) client_stop_tx: Option<oneshot::Sender<()>>,
    pub(super) client_thread: Option<std::thread::JoinHandle<()>>,
    pub(super) _terminal_subscription: Subscription,
    pub(super) _composer_focus_subscription: Subscription,
    pub(super) _composer_blur_subscription: Subscription,
    pub(super) _app_quit_subscription: Subscription,
    pub(super) quitting: bool,
    pub(super) _appearance_subscription: Subscription,
    pub(super) _bounds_subscription: Subscription,
    pub(super) _supervisor: Option<DesktopSupervisor>,
}

/// Pumps native CEF work without invalidating the entire shell at a fixed
/// interval. The browser child is native, so the view itself has no pixels;
/// its entity is only a narrow GPUI invalidation boundary.
pub(super) struct BrowserPumpView {
    shell: WeakEntity<DesktopShell>,
}

impl BrowserPumpView {
    pub(super) fn new(shell: WeakEntity<DesktopShell>, cx: &mut Context<Self>) -> Self {
        let shell_for_task = shell.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
            let Some(this) = this.upgrade() else {
                break;
            };
            let keep_pumping = this.update(cx, |pump, cx| {
                let Some(shell) = pump.shell.upgrade() else {
                    return false;
                };
                shell.update(cx, |shell, cx| {
                    if !shell.browser_panel_open || shell.browser_portal.is_none() {
                        return false;
                    }
                    if let Some(portal) = shell.browser_portal.as_ref() {
                        portal.pump();
                    }
                    if shell.apply_browser_events(cx) {
                        cx.notify();
                    }
                    true
                })
            });
            if !keep_pumping {
                break;
            }
        })
        .detach();
        Self {
            shell: shell_for_task,
        }
    }
}

impl Render for BrowserPumpView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_0()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DesktopThemeMode {
    System,
    Light,
    Dark,
}

impl DesktopThemeMode {
    pub(super) fn parse(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

impl Drop for DesktopShell {
    fn drop(&mut self) {
        self.prepare_for_quit();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpenConversationTab {
    pub(super) session_id: Option<String>,
    pub(super) title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SessionViewState {
    pub(super) layout: ShellLayoutState,
    pub(super) auxiliary_view: AuxiliaryView,
    pub(super) workbench_width: f32,
    pub(super) expanded_chat_parts: BTreeSet<String>,
    pub(super) browser_panel_open: bool,
    pub(super) browser_url: String,
    pub(super) browser_title: String,
}

impl SessionViewState {
    pub(super) fn from_persisted(state: &mew_config::DesktopSessionViewState) -> Self {
        Self {
            layout: ShellLayoutState {
                sidebar_collapsed: state.sidebar_collapsed,
                workbench_collapsed: state.workbench_collapsed,
                terminal_collapsed: state.terminal_collapsed,
                changes_expanded: state.changes_expanded,
                local_expanded: state.local_expanded,
                activity_expanded: state.activity_expanded,
            },
            auxiliary_view: match state.auxiliary_view.as_str() {
                "browser" => AuxiliaryView::Browser,
                "local" => AuxiliaryView::Local,
                "activity" => AuxiliaryView::Activity,
                _ => AuxiliaryView::Changes,
            },
            workbench_width: state
                .workbench_width
                .clamp(WORKBENCH_MIN_WIDTH, WORKBENCH_MAX_WIDTH),
            expanded_chat_parts: state.expanded_chat_parts.iter().cloned().collect(),
            browser_panel_open: state.browser_panel_open,
            browser_url: state.browser_url.clone(),
            browser_title: state.browser_title.clone(),
        }
    }

    pub(super) fn to_persisted(&self) -> mew_config::DesktopSessionViewState {
        mew_config::DesktopSessionViewState {
            sidebar_collapsed: self.layout.sidebar_collapsed,
            workbench_collapsed: self.layout.workbench_collapsed,
            terminal_collapsed: self.layout.terminal_collapsed,
            changes_expanded: self.layout.changes_expanded,
            local_expanded: self.layout.local_expanded,
            activity_expanded: self.layout.activity_expanded,
            auxiliary_view: match self.auxiliary_view {
                AuxiliaryView::Browser => "browser",
                AuxiliaryView::Changes => "changes",
                AuxiliaryView::Local => "local",
                AuxiliaryView::Activity => "activity",
            }
            .into(),
            workbench_width: self.workbench_width,
            expanded_chat_parts: self.expanded_chat_parts.iter().cloned().collect(),
            browser_panel_open: self.browser_panel_open,
            browser_url: self.browser_url.clone(),
            browser_title: self.browser_title.clone(),
        }
    }
}

pub(super) struct ToolTextCache {
    pub(super) source_identity: usize,
    pub(super) source_len: usize,
    pub(super) lines: Arc<Vec<SharedString>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TranscriptSelectionPoint {
    pub(super) message_index: usize,
    pub(super) block_index: usize,
    pub(super) offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TranscriptSelection {
    pub(super) start: TranscriptSelectionPoint,
    pub(super) end: TranscriptSelectionPoint,
}

#[derive(Clone)]
pub(super) struct TranscriptTextEntry {
    pub(super) message_index: usize,
    pub(super) block_index: usize,
    pub(super) text: String,
    pub(super) layout: gpui::TextLayout,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SidebarRow {
    Toolbar,
    Group {
        id: String,
        name: String,
        color: Option<String>,
        count: usize,
        collapsed: bool,
    },
    Session(ConversationItem),
}

/// Pseudo-group id in the sidebar listing conversations without a group.
pub(super) const UNGROUPED_GROUP_ID: &str = "__ungrouped__";
/// Pseudo-group id for the collapsed-by-default archived sessions section.
pub(super) const ARCHIVED_GROUP_ID: &str = "__archived__";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuxiliaryView {
    Browser,
    Changes,
    Local,
    Activity,
}

pub(super) struct WorkbenchResizeDrag;

pub(super) struct WorkbenchResizePreview;

impl Render for WorkbenchResizePreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(1.)).bg(gpui::transparent_black())
    }
}

#[derive(Clone)]
pub(super) struct SessionDrag {
    pub(super) session_id: String,
    pub(super) title: SharedString,
    pub(super) surface: gpui::Rgba,
    pub(super) foreground: gpui::Rgba,
    pub(super) position: Point<Pixels>,
}

impl SessionDrag {
    pub(super) fn new(
        session_id: String,
        title: SharedString,
        surface: gpui::Rgba,
        foreground: gpui::Rgba,
    ) -> Self {
        Self {
            session_id,
            title,
            surface,
            foreground,
            position: Point::default(),
        }
    }

    pub(super) fn positioned(mut self, position: Point<Pixels>) -> Self {
        self.position = position;
        self
    }
}

impl Render for SessionDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .pl(self.position.x - px(60.))
            .pt(self.position.y - px(16.))
            .child(
                div()
                    .max_w(px(180.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .bg(self.surface)
                    .shadow_md()
                    .text_xs()
                    .text_color(self.foreground)
                    .child(self.title.clone()),
            )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ShellLayoutState {
    pub(super) sidebar_collapsed: bool,
    pub(super) workbench_collapsed: bool,
    pub(super) terminal_collapsed: bool,
    pub(super) changes_expanded: bool,
    pub(super) local_expanded: bool,
    pub(super) activity_expanded: bool,
}

impl ShellLayoutState {
    pub(super) fn from_state(state: &mew_config::State) -> Self {
        let collapsed = |key: &str| state.sidebar_collapsed.get(key).copied().unwrap_or(false);
        Self {
            sidebar_collapsed: collapsed("shell.sidebar"),
            workbench_collapsed: collapsed("shell.workbench"),
            terminal_collapsed: collapsed("shell.terminal"),
            changes_expanded: !collapsed("shell.changes"),
            local_expanded: !collapsed("shell.local"),
            activity_expanded: !collapsed("shell.activity"),
        }
    }

    pub(super) fn write_state(self, state: &mut mew_config::State) {
        let entries = [
            ("shell.sidebar", self.sidebar_collapsed),
            ("shell.workbench", self.workbench_collapsed),
            ("shell.terminal", self.terminal_collapsed),
            ("shell.changes", !self.changes_expanded),
            ("shell.local", !self.local_expanded),
            ("shell.activity", !self.activity_expanded),
        ];
        for (key, value) in entries {
            if value {
                state.sidebar_collapsed.insert(key.into(), true);
            } else {
                state.sidebar_collapsed.remove(key);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsPage {
    General,
    Terminal,
    Workspace,
    Connection,
}

impl SettingsPage {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Terminal => "terminal",
            Self::Workspace => "workspace",
            Self::Connection => "connection",
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Terminal => "Terminal",
            Self::Workspace => "Workspace",
            Self::Connection => "Connection",
        }
    }

    pub(super) fn description(self) -> &'static str {
        match self {
            Self::General => "mew desktop preferences",
            Self::Terminal => "terminal appearance and behavior",
            Self::Workspace => "panel visibility and layout",
            Self::Connection => "daemon and remote workspace status",
        }
    }
}

/// Slash commands the daemon executes server-side
/// (`crates/mew-daemon/src/lib.rs` `ClientMessage::SlashCommand`). The desktop
/// autocompletes these in the composer and routes exact submissions to
/// `ClientMessage::SlashCommand` instead of `ClientMessage::Prompt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SlashCommandDef {
    pub(super) name: &'static str,
    pub(super) description: &'static str,
}

pub(super) const SLASH_COMMANDS: &[SlashCommandDef] = &[
    SlashCommandDef {
        name: "/clear",
        description: "Clear the current session",
    },
    SlashCommandDef {
        name: "/compact",
        description: "Compact message history",
    },
    SlashCommandDef {
        name: "/goal",
        description: "Set or manage the session goal",
    },
    SlashCommandDef {
        name: "/wiki",
        description: "Generate the repository wiki",
    },
];

/// Permission modes accepted by `ClientMessage::SetPermissionMode`
/// (ids match `mew_hooks::PermissionMode::id`; labels follow the TUI picker).
pub(super) const PERMISSION_MODES: &[(&str, &str, &str)] = &[
    (
        "standard",
        "Standard",
        "Prompts for mutating and dangerous tools",
    ),
    (
        "permissive",
        "Permissive",
        "Auto-allows mutating tools; still prompts for shell commands",
    ),
    (
        "auto",
        "Auto",
        "Routes tool calls through a classifier model",
    ),
    (
        "auto_plus",
        "Auto+",
        "Classifier cannot escalate; uncertainty denies",
    ),
    (
        "dangerous",
        "Dangerous!",
        "Every tool auto-runs; overrides deny and ask rules",
    ),
];

pub(super) const PROMPT_HISTORY_LIMIT: usize = 100;

/// In-memory prompt history for Up/Down recall in the composer. Entries are
/// oldest → newest; consecutive duplicates are collapsed and recall stashes
/// the in-progress composer text so Down past the newest entry restores it.
#[derive(Clone, Debug, Default)]
pub(super) struct PromptHistory {
    entries: Vec<String>,
    recall_index: Option<usize>,
    stash: String,
}

impl PromptHistory {
    pub(super) fn record(&mut self, text: &str) {
        let text = text.trim();
        self.reset_recall();
        if text.is_empty() {
            return;
        }
        if self.entries.last().is_some_and(|last| last == text) {
            return;
        }
        self.entries.push(text.to_owned());
        if self.entries.len() > PROMPT_HISTORY_LIMIT {
            let overflow = self.entries.len() - PROMPT_HISTORY_LIMIT;
            self.entries.drain(..overflow);
        }
    }

    pub(super) fn reset_recall(&mut self) {
        self.recall_index = None;
        self.stash.clear();
    }

    pub(super) fn is_recalling(&self) -> bool {
        self.recall_index.is_some()
    }

    pub(super) fn recall_older(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let index = match self.recall_index {
            None => {
                self.stash = current.to_owned();
                self.entries.len() - 1
            }
            Some(index) => index.saturating_sub(1),
        };
        self.recall_index = Some(index);
        self.entries.get(index).cloned()
    }

    pub(super) fn recall_newer(&mut self) -> Option<String> {
        let index = self.recall_index?;
        if index + 1 < self.entries.len() {
            self.recall_index = Some(index + 1);
            self.entries.get(index + 1).cloned()
        } else {
            self.recall_index = None;
            Some(std::mem::take(&mut self.stash))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShellCommand {
    ToggleSidebar,
    ToggleTerminal,
    ToggleWorkbench,
    NewConversation,
    CloseActiveTab,
    SelectTab(usize),
}

pub(super) const SIDEBAR_COLLAPSED_WIDTH: f32 = 0.;
pub(super) const SIDEBAR_EXPANDED_WIDTH: f32 = 264.;
pub(super) const WORKBENCH_COLLAPSED_WIDTH: f32 = 0.;
pub(super) const WORKBENCH_EXPANDED_WIDTH: f32 = 360.;
pub(super) const SHELL_GUTTER: f32 = 8.;
pub(super) const SHELL_SURFACE_RADIUS: f32 = 14.;
pub(super) const CHAT_CONTENT_MAX_WIDTH: f32 = 760.;
pub(super) const CHAT_MIN_WIDTH: f32 = 420.;
pub(super) const WORKBENCH_MIN_WIDTH: f32 = 280.;
pub(super) const WORKBENCH_MAX_WIDTH: f32 = 960.;
pub(super) const TERMINAL_EXPANDED_ROWS: u16 = 7;
pub(super) const TERMINAL_COLS: u16 = 80;
pub(super) const TERMINAL_EXPANDED_HEIGHT: f32 = 190.;
pub(super) const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
pub(super) const BROWSER_OWNER: &str = "native-browser";
pub(super) const DEFAULT_BROWSER_URL: &str = "about:blank";
pub(super) const TERMINAL_FONT_CHOICES: &[(&str, &str)] = &[
    (DEFAULT_FONT_FAMILY, "Bundled mono"),
    ("SF Mono", "macOS mono"),
    ("Menlo", "macOS fallback"),
];

pub(super) struct CachedMarkdown {
    pub(super) source: String,
    pub(super) source_identity: usize,
    pub(super) source_len: usize,
    pub(super) render_blocks: Vec<MarkdownRenderBlock>,
    pub(super) streaming: Option<StreamingMarkdown>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct TranscriptRenderRow {
    pub(super) message_index: usize,
    pub(super) part_index: usize,
    pub(super) block_index: usize,
}
