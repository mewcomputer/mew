use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkTabKind {
    NewTab,
    Browser,
    Changes,
    Files,
    Terminal,
    Activity,
}

impl WorkTabKind {
    pub(super) fn is_browser(self) -> bool {
        matches!(self, Self::NewTab | Self::Browser)
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::NewTab => "new_tab",
            Self::Browser => "browser",
            Self::Changes => "changes",
            Self::Files => "files",
            Self::Terminal => "terminal",
            Self::Activity => "activity",
        }
    }

    fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "new_tab" => Self::NewTab,
            "browser" => Self::Browser,
            "changes" => Self::Changes,
            "files" | "local" => Self::Files,
            "terminal" => Self::Terminal,
            "activity" => Self::Activity,
            _ => return None,
        })
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NewTab => "New tab",
            Self::Browser => "Browser",
            Self::Changes => "Changes",
            Self::Files => "Files",
            Self::Terminal => "Terminal",
            Self::Activity => "Activity",
        }
    }

    pub(super) fn icon(self) -> TablerIcon {
        match self {
            Self::NewTab | Self::Browser => TablerIcon::Browser,
            Self::Changes => TablerIcon::FileCode,
            Self::Files => TablerIcon::Folder,
            Self::Terminal => TablerIcon::Terminal2,
            Self::Activity => TablerIcon::GitBranch,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkTab {
    pub(super) id: String,
    pub(super) kind: WorkTabKind,
    pub(super) url: String,
    pub(super) title: String,
    pub(super) error: Option<String>,
    pub(super) initialization_failed: bool,
    pub(super) native_url: String,
    pub(super) native_error: Option<String>,
}

impl WorkTab {
    fn new(kind: WorkTabKind) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            kind,
            url: String::new(),
            title: String::new(),
            error: None,
            initialization_failed: false,
            native_url: DEFAULT_BROWSER_URL.into(),
            native_error: None,
        }
    }

    pub(super) fn label(&self) -> &str {
        if self.kind == WorkTabKind::Browser {
            if !self.title.is_empty() {
                return &self.title;
            }
            if !self.url.is_empty() && self.url != DEFAULT_BROWSER_URL {
                return &self.url;
            }
        }
        self.kind.label()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkTabs {
    pub(super) tabs: Vec<WorkTab>,
    pub(super) active: usize,
    pub(super) browser_tab_id: Option<String>,
}

impl Default for WorkTabs {
    fn default() -> Self {
        let tab = WorkTab::new(WorkTabKind::NewTab);
        Self {
            browser_tab_id: Some(tab.id.clone()),
            tabs: vec![tab],
            active: 0,
        }
    }
}

impl WorkTabs {
    pub(super) fn active(&self) -> &WorkTab {
        &self.tabs[self.active]
    }
    pub(super) fn active_mut(&mut self) -> &mut WorkTab {
        &mut self.tabs[self.active]
    }

    pub(super) fn browser_tab(&self) -> Option<&WorkTab> {
        let id = self.browser_tab_id.as_deref()?;
        self.tabs
            .iter()
            .find(|tab| tab.id == id && tab.kind.is_browser())
    }

    pub(super) fn select(&mut self, id: &str) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        self.active = index;
        self.repair_browser_target();
        true
    }

    pub(super) fn new_tab(&mut self) -> String {
        let tab = WorkTab::new(WorkTabKind::NewTab);
        let id = tab.id.clone();
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.repair_browser_target();
        id
    }

    pub(super) fn open_tool(&mut self, kind: WorkTabKind) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.kind == kind) {
            self.active = index;
        } else if self.active().kind == WorkTabKind::NewTab {
            self.active_mut().kind = kind;
            self.active_mut().url.clear();
            self.active_mut().title.clear();
            self.active_mut().error = None;
        } else {
            self.tabs.push(WorkTab::new(kind));
            self.active = self.tabs.len() - 1;
        }
        self.repair_browser_target();
    }

    pub(super) fn close(&mut self, id: &str) -> Option<WorkTab> {
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
        let closed = self.tabs.remove(index);
        if self.tabs.is_empty() {
            *self = Self::default();
        } else {
            if index < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len() - 1);
            self.repair_browser_target();
        }
        Some(closed)
    }

    fn repair_browser_target(&mut self) {
        if self.active().kind.is_browser() {
            self.browser_tab_id = Some(self.active().id.clone());
        } else if self.browser_tab().is_none() {
            self.browser_tab_id = self
                .tabs
                .iter()
                .rev()
                .find(|tab| tab.kind.is_browser())
                .map(|tab| tab.id.clone());
        }
    }

    pub(super) fn from_persisted(
        saved: Option<&mew_config::DesktopWorkTabs>,
        legacy_view: &str,
        terminal_open: bool,
    ) -> Self {
        let Some(saved) = saved else {
            let mut tabs = Self::default();
            let kind = WorkTabKind::parse(legacy_view).unwrap_or(WorkTabKind::Changes);
            tabs.open_tool(kind);
            let selected = tabs.active().id.clone();
            if terminal_open {
                tabs.open_tool(WorkTabKind::Terminal);
            }
            tabs.select(&selected);
            return tabs;
        };
        let mut ids = BTreeSet::new();
        let tabs: Vec<_> = saved
            .tabs
            .iter()
            .filter_map(|tab| {
                let kind = WorkTabKind::parse(&tab.kind)?;
                if tab.id.is_empty() || !ids.insert(tab.id.clone()) {
                    return None;
                }
                Some(WorkTab {
                    id: tab.id.clone(),
                    kind,
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                    error: None,
                    initialization_failed: false,
                    native_url: DEFAULT_BROWSER_URL.into(),
                    native_error: None,
                })
            })
            .collect();
        if tabs.is_empty() {
            return Self::default();
        }
        let active = tabs
            .iter()
            .position(|tab| tab.id == saved.active_tab_id)
            .unwrap_or(0);
        let mut tabs = Self {
            tabs,
            active,
            browser_tab_id: saved.browser_tab_id.clone(),
        };
        tabs.repair_browser_target();
        tabs
    }

    pub(super) fn to_persisted(&self) -> mew_config::DesktopWorkTabs {
        mew_config::DesktopWorkTabs {
            tabs: self
                .tabs
                .iter()
                .map(|tab| mew_config::DesktopWorkTab {
                    id: tab.id.clone(),
                    kind: tab.kind.key().into(),
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                })
                .collect(),
            active_tab_id: self.active().id.clone(),
            browser_tab_id: self.browser_tab_id.clone(),
        }
    }

    pub(super) fn apply_browser_state(
        &mut self,
        state: &mew_browser_host::BrowserPageState,
    ) -> bool {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == state.page.owner.tab_id && tab.kind.is_browser())
        else {
            return false;
        };
        let mut changed = false;
        let address_changed = tab.native_url != state.url;
        if address_changed {
            tab.native_url.clone_from(&state.url);
            if state.url != DEFAULT_BROWSER_URL || tab.kind == WorkTabKind::Browser {
                tab.url.clone_from(&state.url);
                tab.kind = WorkTabKind::Browser;
                changed = true;
            }
        }
        changed |= browser::replace_browser_event_value(&mut tab.title, state.title.clone());
        let error = if state.closed {
            Some("This browser page was closed. Retry to reopen it.".into())
        } else {
            state.error.clone()
        };
        if address_changed || tab.native_error != error {
            tab.native_error.clone_from(&error);
            tab.error = error;
            changed = true;
        }
        changed
    }
}

impl DesktopShell {
    pub(super) fn action_new_work_tab(
        &mut self,
        _: &NewWorkTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_work_tab(cx);
    }

    pub(super) fn action_close_work_tab(
        &mut self,
        _: &CloseWorkTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_shell_command(ShellCommand::CloseWorkTab, cx);
        window.focus(&self.workbench_focus_handle, cx);
    }

    pub(super) fn new_work_tab(&mut self, cx: &mut Context<Self>) -> bool {
        if !self
            .layout
            .reveal_workbench(self.window_width, self.sidebar_width)
        {
            self.model.last_error = Some("Widen the window to open a work tab.".into());
            cx.notify();
            return false;
        }
        self.save_browser_tab_draft();
        self.work_tabs.new_tab();
        self.browser_url_focus_requested = true;
        self.activate_work_tab(cx);
        true
    }

    pub(super) fn open_work_tool(&mut self, kind: WorkTabKind, cx: &mut Context<Self>) {
        if !self
            .layout
            .reveal_workbench(self.window_width, self.sidebar_width)
            && kind != WorkTabKind::Terminal
        {
            self.model.last_error = Some("Widen the window to open a work tab.".into());
            cx.notify();
            return;
        }
        self.save_browser_tab_draft();
        if self.work_tabs.active().kind == WorkTabKind::NewTab
            && !self.work_tabs.tabs.iter().any(|tab| tab.kind == kind)
        {
            let id = self.work_tabs.active().id.clone();
            self.retire_browser_tab(&id);
        }
        self.work_tabs.open_tool(kind);
        self.activate_work_tab(cx);
    }

    pub(super) fn select_work_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.work_tabs.active().id == id {
            return;
        }
        self.release_browser_focus();
        self.save_browser_tab_draft();
        if self.work_tabs.select(id) {
            self.activate_work_tab(cx);
        }
    }

    pub(super) fn close_work_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        self.release_browser_focus();
        self.save_browser_tab_draft();
        self.retire_browser_tab(id);
        if self.work_tabs.close(id).is_some() {
            self.activate_work_tab(cx);
        }
    }

    fn activate_work_tab(&mut self, cx: &mut Context<Self>) {
        let kind = self.work_tabs.active().kind;
        self.browser_panel_open = kind == WorkTabKind::Browser;
        self.layout.terminal_collapsed = kind != WorkTabKind::Terminal;
        self.browser_url_selection = 0..0;
        self.browser_url_selection_reversed = false;
        self.browser_url_marked_range = None;
        self.sync_browser_portal_to_view_state();
        self.sync_browser_target();
        self.sync_workspace_watch();
        if kind == WorkTabKind::Changes {
            self.review_requests.invalidate();
            self.refresh_review(cx);
        } else if kind == WorkTabKind::Files {
            if let Some(session) = self.model.attached_session.clone() {
                self.refresh_file_tree(&session);
            }
        }
        self.capture_session_view_state();
        self.work_tab_scroll.scroll_to_item(self.work_tabs.active);
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn render_work_tab_bar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        let body = theme_rgb(&self.theme, "text.body");
        let panel = theme_rgb(&self.theme, "panel.background");
        let card = theme_rgb(&self.theme, "card");
        let divider = theme_rgb(&self.theme, "divider");
        let tabs = self
            .work_tabs
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let id = tab.id.clone();
                let close_id = id.clone();
                let selected = index == self.work_tabs.active;
                let label: SharedString = tab.label().to_owned().into();
                div()
                    .id(format!("work-tab-{}", tab.id))
                    .debug_selector(move || format!("work-tab-{index}"))
                    .role(Role::Tab)
                    .aria_selected(selected)
                    .aria_label(label.clone())
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .w(px(176.))
                    .h(px(30.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if selected { divider } else { panel })
                    .bg(if selected { card } else { panel })
                    .text_xs()
                    .text_color(if selected { body } else { muted })
                    .cursor_pointer()
                    .hover(|element| element.bg(card))
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.select_work_tab(&id, cx);
                        window.focus(&shell.workbench_focus_handle, cx);
                    }))
                    .child(tabler_icon(
                        tab.kind.icon(),
                        if selected { body } else { muted },
                        px(13.),
                    ))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(
                        div()
                            .id(format!("close-work-tab-{}", tab.id))
                            .debug_selector(move || format!("close-work-tab-{index}"))
                            .role(Role::Button)
                            .aria_label(SharedString::from(format!("Close {}", tab.label())))
                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(20.))
                            .rounded(px(4.))
                            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                            .on_click(cx.listener(move |shell, _, window, cx| {
                                cx.stop_propagation();
                                shell.close_work_tab(&close_id, cx);
                                window.focus(&shell.workbench_focus_handle, cx);
                            }))
                            .child(tabler_icon(TablerIcon::X, muted, px(12.))),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .id("work-header")
            .debug_selector(|| "work-header".into())
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h(px(PANE_HEADER_HEIGHT))
            .border_b_1()
            .border_color(divider)
            .px(px(4.))
            .gap(px(4.))
            .child(
                div()
                    .id("work-tab-strip")
                    .role(Role::TabList)
                    .aria_label("Open work tabs")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(4.))
                    .overflow_x_scroll()
                    .track_scroll(&self.work_tab_scroll)
                    .children(tabs),
            )
            .child(
                div()
                    .id("new-work-tab")
                    .debug_selector(|| "new-work-tab".into())
                    .role(Role::Button)
                    .aria_label("New work tab")
                    .aria_keyshortcuts("Command+T")
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|element| element.bg(card))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.new_work_tab(cx);
                    }))
                    .child(tabler_icon(TablerIcon::Plus, muted, px(14.))),
            )
            .into_any_element()
    }

    pub(super) fn render_new_work_tab(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        let body = theme_rgb(&self.theme, "text.body");
        let tools = [
            WorkTabKind::Changes,
            WorkTabKind::Terminal,
            WorkTabKind::Files,
            WorkTabKind::Activity,
        ]
        .into_iter()
        .map(|kind| {
            div()
                .id(format!("launch-{}", kind.key()))
                .debug_selector(move || format!("launch-{}", kind.key()))
                .role(Role::Button)
                .aria_label(SharedString::from(format!("Open {}", kind.label())))
                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(38.))
                .w(relative(0.48))
                .px(px(12.))
                .rounded(px(6.))
                .bg(theme_rgb(&self.theme, "card"))
                .text_xs()
                .text_color(body)
                .cursor_pointer()
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.open_work_tool(kind, cx);
                    window.focus(&shell.workbench_focus_handle, cx);
                }))
                .child(tabler_icon(kind.icon(), muted, px(14.)))
                .child(kind.label())
                .into_any_element()
        })
        .collect::<Vec<_>>();
        div()
            .id("work-new-tab-page")
            .debug_selector(|| "work-new-tab-page".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.render_browser_toolbar(cx))
            .when_some(self.browser_error.clone(), |element, error| {
                element.child(
                    div()
                        .id("new-tab-error")
                        .debug_selector(|| "new-tab-error".into())
                        .flex()
                        .flex_none()
                        .flex_col()
                        .gap(px(8.))
                        .p(px(12.))
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.warning"))
                        .child(error)
                        .when(self.work_tabs.active().initialization_failed, |element| {
                            element.child(
                                div()
                                    .id("new-tab-retry-browser")
                                    .role(Role::Button)
                                    .aria_label("Retry browser")
                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(|shell, _, _, cx| shell.retry_browser(cx)),
                                    )
                                    .child("Try again"),
                            )
                        }),
                )
            })
            .child(
                div()
                    .id("work-new-tab-tools")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .pt(px(32.))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(640.))
                            .mx_auto()
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .child(div().text_xs().text_color(muted).child("Tools"))
                            .child(div().flex().flex_wrap().gap(px(8.)).children(tools))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child("Enter a URL above to open a browser page."),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_tabs_have_separate_identity_and_keep_their_address_drafts() {
        let mut tabs = WorkTabs::default();
        let first = tabs.active().id.clone();
        tabs.active_mut().url = "https://first.example/draft".into();
        let second = tabs.new_tab();
        tabs.active_mut().url = "https://second.example".into();
        assert_ne!(first, second);
        assert_eq!(tabs.tabs.len(), 2);
        assert!(tabs.select(&first));
        assert_eq!(tabs.active().url, "https://first.example/draft");
        assert_eq!(tabs.browser_tab_id.as_deref(), Some(first.as_str()));
        assert!(!tabs.select("missing"));
        assert_eq!(tabs.active().id, first);
    }

    #[test]
    fn launcher_replaces_a_blank_tab_and_reuses_open_tools() {
        let mut tabs = WorkTabs::default();
        let first = tabs.active().id.clone();
        tabs.open_tool(WorkTabKind::Changes);
        assert_eq!(tabs.active().id, first);
        assert_eq!(tabs.tabs.len(), 1);
        assert!(tabs.browser_tab_id.is_none());
        tabs.new_tab();
        tabs.open_tool(WorkTabKind::Terminal);
        let terminal = tabs.active().id.clone();
        assert_eq!(tabs.tabs.len(), 2);
        tabs.select(&first);
        tabs.open_tool(WorkTabKind::Terminal);
        assert_eq!(tabs.active().id, terminal);
        assert_eq!(tabs.tabs.len(), 2);
    }

    #[test]
    fn closing_tabs_selects_a_neighbor_and_always_leaves_a_usable_launcher() {
        let mut tabs = WorkTabs::default();
        let first = tabs.active().id.clone();
        let second = tabs.new_tab();
        tabs.open_tool(WorkTabKind::Files);
        let third = tabs.new_tab();
        tabs.select(&second);
        tabs.close(&first);
        assert_eq!(tabs.active().id, second);
        tabs.close(&second);
        assert_eq!(tabs.active().id, third);
        tabs.close(&third);
        assert_eq!(tabs.tabs.len(), 1);
        assert_eq!(tabs.active().kind, WorkTabKind::NewTab);
        assert_ne!(tabs.active().id, third);
        assert!(tabs.close("missing").is_none());
    }

    #[test]
    fn persisted_tabs_restore_order_selection_and_browser_target_and_reject_bad_entries() {
        let mut tabs = WorkTabs::default();
        let first = tabs.active().id.clone();
        tabs.active_mut().kind = WorkTabKind::Browser;
        tabs.active_mut().url = "https://first.example".into();
        tabs.new_tab();
        tabs.open_tool(WorkTabKind::Terminal);
        let saved = tabs.to_persisted();
        let restored = WorkTabs::from_persisted(Some(&saved), "changes", false);
        assert_eq!(restored, tabs);

        let mut malformed = saved;
        malformed.tabs.push(malformed.tabs[0].clone());
        malformed.tabs[0].kind = "unknown-tool".into();
        malformed.active_tab_id = "missing".into();
        malformed.browser_tab_id = Some("missing".into());
        let restored = WorkTabs::from_persisted(Some(&malformed), "changes", false);
        assert_eq!(restored.tabs.len(), 2);
        assert!(restored.tabs.iter().any(|tab| tab.id == first));
        assert_eq!(restored.active().kind, WorkTabKind::Terminal);
        assert_eq!(restored.browser_tab_id.as_deref(), Some(first.as_str()));
    }

    #[test]
    fn background_browser_metadata_preserves_an_address_draft_and_ignores_closed_tabs() {
        let mut tabs = WorkTabs::default();
        let first = tabs.active().id.clone();
        let page = BrowserPage::new(BrowserOwner {
            connection_profile: "local".into(),
            session_id: "chat".into(),
            tab_id: first.clone(),
        });
        let mut state = mew_browser_host::BrowserPageState {
            page,
            url: "https://first.example".into(),
            title: "First".into(),
            visible: false,
            closed: false,
            ready: None,
            error: None,
        };
        assert!(tabs.apply_browser_state(&state));
        tabs.active_mut().url = "https://first.example/unsent-address".into();
        tabs.active_mut().error = Some("enter a valid http(s) URL".into());
        let second = tabs.new_tab();
        state.title = "First, finished loading".into();
        assert!(tabs.apply_browser_state(&state));
        assert_eq!(tabs.active().id, second);
        assert!(tabs.active().url.is_empty());
        tabs.select(&first);
        assert_eq!(tabs.active().url, "https://first.example/unsent-address");
        assert_eq!(tabs.active().label(), "First, finished loading");
        assert_eq!(
            tabs.active().error.as_deref(),
            Some("enter a valid http(s) URL")
        );
        tabs.close(&first);
        assert!(!tabs.apply_browser_state(&state));
        assert!(tabs.active().error.is_none());
    }

    #[gpui::test]
    fn transcript_links_open_independent_browser_tabs(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let window =
            cx.add_window(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let shell = window.root(cx).unwrap();
        shell.update(cx, |shell, cx| {
            shell.window_width = 1440.;
            shell.work_tabs.active_mut().kind = WorkTabKind::Browser;
            shell.browser_url = "https://first.example".into();
            shell.open_transcript_link("https://second.example".into(), cx);
            assert_eq!(shell.work_tabs.tabs.len(), 2);
            assert_eq!(shell.work_tabs.tabs[0].url, "https://first.example");
            assert_eq!(shell.work_tabs.active().kind, WorkTabKind::Browser);
            assert_eq!(shell.work_tabs.active().url, "https://second.example");
            assert_eq!(shell.browser_url, "https://second.example");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn invalid_browser_address_keeps_the_launcher_and_shows_recoverable_feedback(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1440.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.workbench_collapsed = false;
            shell.workbench_animation_id = 0;
            shell.browser_url = "file:///private/example".into();
            shell.navigate_browser_to(cx);
            assert_eq!(shell.work_tabs.active().kind, WorkTabKind::NewTab);
            assert!(!shell.work_tabs.active().initialization_failed);
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        assert!(cx.debug_bounds("new-tab-error").is_some());
        assert!(cx.debug_bounds("launch-changes").is_some());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn folding_work_content_disables_its_retained_browser_input(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1440.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.workbench_collapsed = false;
            shell.workbench_animation_id = 0;
            shell.work_tabs.active_mut().kind = WorkTabKind::Browser;
            cx.notify();
        });
        cx.update(|window, cx| {
            let focus = shell.read(cx).browser_url_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| assert!(window.is_action_available(&TextPaste, cx)));
        shell.update(cx, |shell, cx| {
            shell.layout.workbench_collapsed = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| assert!(!window.is_action_available(&TextPaste, cx)));
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn many_work_tabs_keep_the_selected_tab_and_new_tab_button_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1200.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.workbench_animation_id = 0;
            for _ in 0..6 {
                shell.new_work_tab(cx);
            }
            shell.workbench_animation_id = 0;
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let selected = cx.debug_bounds("work-tab-6").unwrap();
        let plus = cx.debug_bounds("new-work-tab").unwrap();
        let pane = cx.debug_bounds("work-pane").unwrap();
        assert!(selected.origin.x >= pane.origin.x);
        assert!(selected.right() <= plus.origin.x);
        assert!(plus.right() <= pane.right());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn work_tab_shortcuts_keep_chat_navigation_and_drafts_separate(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1440.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.workbench_animation_id = 0;
            shell.model.ui.composer = "do not lose this".into();
            cx.notify();
        });
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-t");
        cx.run_until_parked();
        shell.update(cx, |shell, _| assert_eq!(shell.work_tabs.tabs.len(), 2));
        cx.simulate_keystrokes("cmd-w");
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.work_tabs.tabs.len(), 1);
            assert_eq!(shell.model.ui.composer, "do not lose this");
            shell.quitting = true;
        });
    }
}
