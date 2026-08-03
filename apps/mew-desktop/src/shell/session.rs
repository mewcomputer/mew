use super::*;

impl DesktopShell {
    pub(super) fn new_conversation(&mut self, cx: &mut Context<Self>) {
        self.begin_new_conversation(None, cx);
    }

    pub(super) fn new_conversation_in_group(&mut self, group_id: String, cx: &mut Context<Self>) {
        self.begin_new_conversation(Some(group_id), cx);
    }

    pub(super) fn begin_new_conversation(
        &mut self,
        group_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.capture_session_view_state();
        self.close_terminal();
        self.open_new_tab();
        self.reset_unbound_tab_view();
        self.pending_model = None;
        self.awaiting_model_switch = None;
        self.pending_prompt = None;
        self.pending_session_request = true;
        self.pending_session_target = None;
        let command = match group_id {
            Some(group_id) => self.model.ui.new_conversation_in_group(None, group_id),
            None => self.model.ui.new_conversation(None),
        };
        self.composer_selection = 0..0;
        self.composer_selection_reversed = false;
        self.composer_marked_range = None;
        self.composer_is_selecting = false;
        self.restart_composer_cursor_blink(cx);
        self.send_command(command);
        cx.notify();
    }

    pub(super) fn open_browser_panel(&mut self, cx: &mut Context<Self>) {
        let was_open = self.browser_panel_open;
        self.auxiliary_view = AuxiliaryView::Browser;
        self.capture_session_view_state();
        self.browser_panel_open = true;
        self.browser_error = None;
        self.browser_url_focus_requested = true;
        if self.browser_url.is_empty() {
            self.browser_url = self
                .model
                .ui
                .browser
                .url
                .clone()
                .unwrap_or_else(|| DEFAULT_BROWSER_URL.to_owned());
        }
        if !was_open && self.browser_portal.is_some() {
            self.start_browser_pump(cx);
        }
        self.capture_session_view_state();
        cx.notify();
    }

    pub(super) fn close_browser_panel(&mut self, cx: &mut Context<Self>) {
        self.browser_panel_open = false;
        self.browser_native_rect = None;
        self.browser_pump_epoch = self.browser_pump_epoch.wrapping_add(1);
        self.browser_pump = None;
        self.browser_url_focus_requested = false;
        if let Some(portal) = self.browser_portal.as_ref() {
            let _ = portal.close(BROWSER_OWNER);
        }
        self.capture_session_view_state();
        cx.notify();
    }

    fn reset_browser_for_unattached_tab(&mut self) {
        self.browser_panel_open = false;
        self.browser_native_rect = None;
        self.browser_pump_epoch = self.browser_pump_epoch.wrapping_add(1);
        self.browser_pump = None;
        self.browser_initialization_pending = false;
        self.browser_url_focus_requested = false;
        self.browser_url.clear();
        self.browser_title.clear();
        self.browser_error = None;
        if let Some(portal) = self.browser_portal.as_ref() {
            let _ = portal.close(BROWSER_OWNER);
        }
    }

    fn reset_unbound_tab_view(&mut self) {
        self.pending_session_request = false;
        self.pending_session_target = None;
        self.model.attached_session = None;
        self.model.ui.selected_session = None;
        self.model.ui.transcript.clear();
        self.reset_browser_for_unattached_tab();
        self.markdown_cache.clear();
        self.tool_text_lists.borrow_mut().clear();
        self.tool_text_cache.borrow_mut().clear();
        self.transcript_rows.clear();
        self.transcript_list.reset(0);
        self.expanded_chat_parts.clear();
        self.transcript_selection = None;
        self.transcript_selection_anchor = None;
        self.transcript_selected_text = None;
        self.transcript_is_selecting = false;
        self.clear_review();
    }

    pub(super) fn retry_browser(&mut self, cx: &mut Context<Self>) {
        if let Some(portal) = self.browser_portal.take() {
            let _ = portal.close(BROWSER_OWNER);
        }
        self.browser_initialization_pending = false;
        self.browser_native_rect = None;
        self.browser_error = None;
        self.browser_url_focus_requested = false;
        cx.notify();
    }

    pub(super) fn navigate_browser_to(&mut self, cx: &mut Context<Self>) {
        let Some(url) = normalize_browser_url(&self.browser_url) else {
            self.browser_error = Some("enter a valid http(s) URL".into());
            cx.notify();
            return;
        };
        self.browser_url = url.clone();
        self.browser_error = None;
        if let Some(portal) = self.browser_portal.as_ref() {
            if let Err(error) = portal.navigate(BROWSER_OWNER, &url) {
                self.browser_error = Some(error.to_string());
            }
        }
        self.capture_session_view_state();
        cx.notify();
    }

    fn start_browser_pump(&mut self, cx: &mut Context<Self>) {
        if self.browser_pump.is_none() {
            let shell = cx.entity().downgrade();
            self.browser_pump = Some(cx.new(|cx| BrowserPumpView::new(shell, cx)));
        }
    }

    pub(super) fn select_auxiliary_view(&mut self, view: AuxiliaryView, cx: &mut Context<Self>) {
        self.auxiliary_view = view;
        self.capture_session_view_state();
        self.sync_workspace_watch();
        if view == AuxiliaryView::Browser {
            self.open_browser_panel(cx);
        } else if self.browser_panel_open {
            self.close_browser_panel(cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn ensure_browser_portal(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !browser_initialization_is_needed(
            self.browser_panel_open,
            self.browser_portal.is_some(),
            self.browser_initialization_pending,
            self.browser_error.is_some(),
        ) {
            return;
        }
        let Some(parent_view) = parent_view_handle(window) else {
            self.browser_error = Some("native browser is unavailable on this platform".into());
            cx.notify();
            return;
        };
        let initial_url = if self.browser_url.is_empty() {
            DEFAULT_BROWSER_URL
        } else {
            self.browser_url.as_str()
        };
        self.browser_initialization_pending = true;
        let initial_url = initial_url.to_owned();
        let shell = cx.entity();
        cx.defer(move |cx| {
            shell.update(cx, |shell, cx| {
                shell.initialize_browser_portal(parent_view, initial_url, cx);
            });
        });
    }

    fn initialize_browser_portal(
        &mut self,
        parent_view: usize,
        initial_url: String,
        cx: &mut Context<Self>,
    ) {
        self.browser_initialization_pending = false;
        if !self.browser_panel_open || self.browser_portal.is_some() {
            return;
        }
        // GPUI's render pass runs on AppKit's main thread. The native CEF
        // message loop must be pumped there, so the render loop below is the
        // scheduler for this child surface.
        let pump_trigger: PumpTrigger = std::sync::Arc::new(|| {});
        let portal = match BrowserPortal::initialize(parent_view, &initial_url, pump_trigger) {
            Ok(Some(portal)) => portal,
            Ok(None) => {
                self.browser_error = Some(
                    "native browser is unavailable; install the packaged browser runtime".into(),
                );
                eprintln!("native browser unavailable: packaged browser runtime missing");
                return;
            }
            Err(error) => {
                self.browser_error = Some(format!("could not start native browser: {error:#}"));
                return;
            }
        };
        self.browser_portal = Some(portal);
        self.start_browser_pump(cx);
        if let Some(portal) = self.browser_portal.as_ref() {
            let _ = portal.set_visible(BROWSER_OWNER, true);
            if !initial_url.is_empty() {
                let _ = portal.navigate(BROWSER_OWNER, &initial_url);
            }
        }
        cx.notify();
    }

    pub(super) fn apply_browser_events(&mut self, _cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        let events = self
            .browser_portal
            .as_ref()
            .map(BrowserPortal::drain_events)
            .unwrap_or_default();
        for event in events {
            match event {
                BrowserEvent::AddressChanged { owner, url }
                    if owner.as_deref().is_none_or(|owner| owner == BROWSER_OWNER) =>
                {
                    self.browser_url = url;
                    self.browser_error = None;
                    changed = true;
                }
                BrowserEvent::TitleChanged { owner, title, .. }
                    if owner.as_deref().is_none_or(|owner| owner == BROWSER_OWNER) =>
                {
                    self.browser_title = title;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    pub(super) fn update_browser_bounds(&mut self, bounds: Bounds<Pixels>, window: &Window) {
        let Some(portal) = self.browser_portal.as_ref() else {
            return;
        };
        let rect = browser_native_rect(
            bounds,
            window.bounds().size,
            self.browser_panel_open && !self.layout.workbench_collapsed,
        );
        if self.browser_native_rect == Some(rect) {
            return;
        }
        if let Err(error) = portal.set_rect(BROWSER_OWNER, rect) {
            self.browser_error = Some(error.to_string());
        } else {
            self.browser_native_rect = Some(rect);
        }
    }

    pub(super) fn attach_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.capture_session_view_state();
        if let Some(command) = self.model.ui.select_session(&session_id) {
            self.restore_session_view_state(&session_id, cx);
            self.clear_pending_session_view();
            self.transcript_selection = None;
            self.transcript_selection_anchor = None;
            self.transcript_selected_text = None;
            self.transcript_is_selecting = false;
            self.pending_session_request = true;
            self.pending_session_target = Some(session_id.clone());
            if self.model.attached_session.as_deref() != Some(session_id.as_str()) {
                self.close_terminal();
            }
            self.open_session_tab(&session_id);
            self.send_command(command);
            cx.notify();
        }
    }

    fn clear_pending_session_view(&mut self) {
        self.model.attached_session = None;
        self.model.last_error = None;
        self.model.ui.clear_attached_session_projection();
        self.markdown_cache.clear();
        self.tool_text_lists.borrow_mut().clear();
        self.tool_text_cache.borrow_mut().clear();
        self.transcript_rows.clear();
        self.transcript_list.reset(0);
        self.expanded_chat_parts.clear();
        self.clear_review();
    }

    pub(super) fn conversation_title(&self, session_id: &str) -> String {
        self.model
            .ui
            .conversations
            .iter()
            .find(|conversation| conversation.session_id == session_id)
            .map(|conversation| conversation.title.clone())
            .unwrap_or_else(|| "New conversation".into())
    }

    pub(super) fn refresh_open_tab_titles(&mut self) {
        for tab in &mut self.open_tabs {
            refresh_tab_title(tab, &self.model.ui.conversations);
        }
    }

    pub(super) fn open_session_tab(&mut self, session_id: &str) {
        let title = self.conversation_title(session_id);
        self.active_tab = Some(open_tab_for_session(
            &mut self.open_tabs,
            self.active_tab,
            OpenConversationTab {
                session_id: Some(session_id.to_owned()),
                title,
            },
        ));
    }

    pub(super) fn bind_new_tab_to_session(&mut self, session_id: &str) {
        self.open_session_tab(session_id);
    }

    pub(super) fn open_new_tab(&mut self) {
        if let Some(index) = self
            .open_tabs
            .iter()
            .position(|tab| tab.session_id.is_none())
        {
            self.active_tab = Some(index);
            return;
        }
        self.open_tabs.push(OpenConversationTab {
            session_id: None,
            title: "New conversation".into(),
        });
        self.active_tab = Some(self.open_tabs.len() - 1);
    }

    pub(super) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.open_tabs.len() {
            return;
        }
        self.open_tabs.remove(index);
        if self.open_tabs.is_empty() {
            self.active_tab = None;
            self.reset_unbound_tab_view();
        } else {
            let next_index = active_tab_after_close(self.active_tab, index, self.open_tabs.len())
                .unwrap_or_default();
            self.active_tab = Some(next_index);
            if let Some(session_id) = self.open_tabs[next_index].session_id.clone() {
                self.attach_session(session_id, cx);
            } else {
                self.reset_unbound_tab_view();
            }
        }
        cx.notify();
    }

    pub(super) fn persist_layout(&self) {
        let Ok(mut state) = mew_config::load_state() else {
            tracing::warn!("desktop: could not load state while saving layout");
            return;
        };
        self.layout.write_state(&mut state);
        state.desktop_session_views = self
            .session_view_states
            .iter()
            .map(|(session_id, view_state)| (session_id.clone(), view_state.to_persisted()))
            .collect();
        state.terminal_font = self.terminal_font_family.clone();
        if let Err(error) = mew_config::save_state(&state) {
            tracing::warn!(%error, "desktop: could not persist layout");
        }
    }

    pub(super) fn persist_window_bounds(&self, window: &Window) {
        let bounds = window.bounds();
        let Ok(mut state) = mew_config::load_state() else {
            tracing::warn!("desktop: could not load state while saving window bounds");
            return;
        };
        state.desktop_window = Some(mew_config::DesktopWindowState {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        });
        if let Err(error) = mew_config::save_state(&state) {
            tracing::warn!(%error, "desktop: could not persist window bounds");
        }
    }

    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.layout.sidebar_collapsed = !self.layout.sidebar_collapsed;
        self.sidebar_animation_id = self.sidebar_animation_id.wrapping_add(1);
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn toggle_workbench(&mut self, cx: &mut Context<Self>) {
        self.layout.workbench_collapsed = !self.layout.workbench_collapsed;
        self.workbench_animation_id = self.workbench_animation_id.wrapping_add(1);
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn toggle_terminal(&mut self, cx: &mut Context<Self>) {
        self.layout.terminal_collapsed = !self.layout.terminal_collapsed;
        self.terminal_animation_id = self.terminal_animation_id.wrapping_add(1);
        self.capture_session_view_state();
        if let (Some(command_tx), Some(terminal_id)) = (&self.command_tx, &self.terminal_id) {
            let _ = command_tx.send(ClientMessage::TerminalResize {
                terminal_id: terminal_id.clone(),
                rows: if self.layout.terminal_collapsed {
                    1
                } else {
                    TERMINAL_EXPANDED_ROWS
                },
                cols: TERMINAL_COLS,
            });
        }
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn close_terminal(&mut self) {
        if let Some(terminal_id) = self.terminal_id.take() {
            self.send_command(ClientMessage::TerminalClose { terminal_id });
        }
        self.terminal_status = "closed".into();
    }

    pub(super) fn toggle_changes(&mut self, cx: &mut Context<Self>) {
        self.layout.changes_expanded = !self.layout.changes_expanded;
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn toggle_local(&mut self, cx: &mut Context<Self>) {
        self.layout.local_expanded = !self.layout.local_expanded;
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn toggle_activity(&mut self, cx: &mut Context<Self>) {
        self.layout.activity_expanded = !self.layout.activity_expanded;
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        self.close_shell_popovers();
        cx.notify();
    }

    pub(super) fn close_shell_popovers(&mut self) -> bool {
        let was_open = self.model_picker_open
            || self.persona_picker_open
            || self.permission_picker_open
            || self.thinking_picker_open
            || self.terminal_font_picker_open
            || self.connection_picker_open;
        self.model_picker_open = false;
        self.persona_picker_open = false;
        self.permission_picker_open = false;
        self.thinking_picker_open = false;
        self.terminal_font_picker_open = false;
        self.connection_picker_open = false;
        was_open
    }

    pub(super) fn select_settings_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.settings_open = true;
        self.settings_page = page;
        self.close_shell_popovers();
        cx.notify();
    }

    pub(super) fn dispatch_shell_command(&mut self, command: ShellCommand, cx: &mut Context<Self>) {
        match command {
            ShellCommand::ToggleSidebar => self.toggle_sidebar(cx),
            ShellCommand::ToggleTerminal => self.toggle_terminal(cx),
            ShellCommand::ToggleWorkbench => self.toggle_workbench(cx),
            ShellCommand::NewConversation => self.new_conversation(cx),
            ShellCommand::CloseActiveTab => {
                if let Some(index) = self.active_tab {
                    self.close_tab(index, cx);
                }
            }
            ShellCommand::SelectTab(index) => self.select_tab(index, cx),
        }
    }

    pub(super) fn action_new_conversation(
        &mut self,
        _: &NewConversation,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_shell_command(ShellCommand::NewConversation, cx);
    }

    pub(super) fn action_close_conversation(
        &mut self,
        _: &CloseConversation,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_shell_command(ShellCommand::CloseActiveTab, cx);
    }

    pub(super) fn action_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_shell_command(ShellCommand::ToggleSidebar, cx);
    }

    pub(super) fn action_toggle_terminal(
        &mut self,
        _: &ToggleTerminal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_shell_command(ShellCommand::ToggleTerminal, cx);
    }

    pub(super) fn action_toggle_workbench(
        &mut self,
        _: &ToggleWorkbench,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_shell_command(ShellCommand::ToggleWorkbench, cx);
    }

    pub(super) fn action_dismiss_popovers(
        &mut self,
        _: &DismissPopovers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_shell_popovers() {
            window.focus(&self.composer_focus_handle, cx);
            cx.notify();
        }
    }

    pub(super) fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.open_tabs.get(index).cloned() else {
            return;
        };
        self.active_tab = Some(index);
        if let Some(session_id) = tab.session_id {
            self.attach_session(session_id, cx);
        } else {
            self.reset_unbound_tab_view();
            cx.notify();
        }
    }

    pub(super) fn send_command(&self, command: ClientMessage) {
        if let Some(command_tx) = &self.command_tx {
            if let Err(error) = command_tx.send(command) {
                tracing::warn!(error = %error, "desktop: command channel closed");
            }
        }
    }

    pub(super) fn yield_control(&mut self, cx: &mut Context<Self>) {
        self.send_command(ClientMessage::YieldControl {});
        cx.notify();
    }

    pub(super) fn create_group(&mut self, cx: &mut Context<Self>) {
        let next_number = self.model.ui.groups.len() + 1;
        self.send_command(ClientMessage::CreateGroup {
            name: format!("Group {next_number}"),
            color: None,
        });
        cx.notify();
    }

    pub(super) fn delete_group(&mut self, group_id: String, cx: &mut Context<Self>) {
        self.send_command(ClientMessage::DeleteGroup { group_id });
        self.session_menu_session = None;
        cx.notify();
    }

    pub(super) fn capture_session_view_state(&mut self) {
        let Some(session_id) = self
            .model
            .ui
            .selected_session
            .clone()
            .or_else(|| self.model.attached_session.clone())
        else {
            return;
        };
        self.session_view_states
            .insert(session_id, self.current_session_view_state());
    }

    fn current_session_view_state(&self) -> SessionViewState {
        SessionViewState {
            layout: self.layout,
            auxiliary_view: self.auxiliary_view,
            workbench_width: self.workbench_width,
            expanded_chat_parts: self.expanded_chat_parts.clone(),
            browser_panel_open: self.browser_panel_open,
            browser_url: self.browser_url.clone(),
            browser_title: self.browser_title.clone(),
        }
    }

    fn ensure_session_view_state(&mut self, session_id: &str) {
        let current_view_state = self.current_session_view_state();
        self.session_view_states
            .entry(session_id.to_owned())
            .or_insert(current_view_state);
    }

    fn restore_session_view_state(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let current_view_state = self.current_session_view_state();
        let view_state = session_view_state_or_current(
            &mut self.session_view_states,
            session_id,
            current_view_state,
        );
        self.layout = view_state.layout;
        self.auxiliary_view = view_state.auxiliary_view;
        self.workbench_width = view_state.workbench_width;
        self.expanded_chat_parts = view_state.expanded_chat_parts;
        self.browser_panel_open = view_state.browser_panel_open;
        self.browser_url = view_state.browser_url;
        self.browser_title = view_state.browser_title;
        self.browser_error = None;
        self.browser_initialization_pending = false;
        self.sync_browser_portal_to_view_state();
        if self.browser_panel_open && self.browser_portal.is_some() {
            self.start_browser_pump(cx);
        }
    }

    fn sync_browser_portal_to_view_state(&mut self) {
        let Some(portal) = self.browser_portal.as_ref() else {
            return;
        };
        if !self.browser_panel_open {
            if let Err(error) = portal.set_visible(BROWSER_OWNER, false) {
                self.browser_error = Some(error.to_string());
            }
            return;
        }
        if let Err(error) = portal.set_visible(BROWSER_OWNER, true) {
            self.browser_error = Some(error.to_string());
            return;
        }
        if browser_url_is_navigable(&self.browser_url) {
            if let Err(error) = portal.navigate(BROWSER_OWNER, &self.browser_url) {
                self.browser_error = Some(error.to_string());
            }
        }
    }

    pub(super) fn rebuild_sidebar_rows(&mut self) {
        self.sidebar_rows = build_sidebar_rows(
            &self.model.ui.conversations,
            &self.model.ui.groups,
            &self.collapsed_groups,
        );
    }

    pub(super) fn toggle_group(&mut self, group_id: String, cx: &mut Context<Self>) {
        if !self.collapsed_groups.insert(group_id.clone()) {
            self.collapsed_groups.remove(&group_id);
        }
        self.rebuild_sidebar_rows();
        cx.notify();
    }

    pub(super) fn toggle_session_menu(&mut self, session_id: String, cx: &mut Context<Self>) {
        let row_index = self.sidebar_rows.iter().position(|row| {
            matches!(row, SidebarRow::Session(conversation) if conversation.session_id == session_id)
        });
        if self.session_menu_session.as_deref() == Some(session_id.as_str()) {
            self.session_menu_session = None;
        } else {
            self.session_menu_session = Some(session_id);
        }
        if let Some(row_index) = row_index {
            self.sidebar_list.remeasure_items(row_index..row_index + 1);
        }
        cx.notify();
    }

    pub(super) fn archive_session(
        &mut self,
        session_id: String,
        archived: bool,
        cx: &mut Context<Self>,
    ) {
        self.send_command(ClientMessage::ArchiveSession {
            session_id,
            archived,
        });
        self.session_menu_session = None;
        cx.notify();
    }

    pub(super) fn pin_session(&mut self, session_id: String, pinned: bool, cx: &mut Context<Self>) {
        self.send_command(ClientMessage::PinSession { session_id, pinned });
        self.session_menu_session = None;
        cx.notify();
    }

    pub(super) fn begin_rename(
        &mut self,
        session_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = self.conversation_title(&session_id);
        self.rename_session_id = Some(session_id);
        self.rename_draft = title;
        let end = self.rename_draft.len();
        self.rename_selection = 0..end;
        self.rename_selection_reversed = false;
        self.rename_marked_range = None;
        self.session_menu_session = None;
        self.release_browser_focus();
        window.focus(&self.rename_focus_handle, cx);
        let row_index = self.sidebar_rows.iter().position(|row| {
            matches!(row, SidebarRow::Session(conversation)
                if Some(&conversation.session_id) == self.rename_session_id.as_ref())
        });
        if let Some(row_index) = row_index {
            self.sidebar_list.remeasure_items(row_index..row_index + 1);
        }
        cx.notify();
    }

    pub(super) fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.rename_session_id = None;
        self.rename_draft.clear();
        cx.notify();
    }

    pub(super) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.rename_session_id.take() else {
            return;
        };
        let title = self.rename_draft.trim().to_owned();
        self.rename_draft.clear();
        if !title.is_empty() && title != self.conversation_title(&session_id) {
            self.send_command(ClientMessage::RenameSession { session_id, title });
        }
        cx.notify();
    }

    pub(super) fn rename_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        if key == "enter" {
            self.commit_rename(cx);
            cx.stop_propagation();
        } else if key == "escape" {
            self.cancel_rename(cx);
            cx.stop_propagation();
        } else if key == "a" && event.keystroke.modifiers.platform {
            self.rename_selection = 0..self.rename_draft.len();
            self.rename_selection_reversed = false;
            cx.notify();
            cx.stop_propagation();
        } else if key == "backspace" {
            self.rename_backspace(window, cx);
            cx.stop_propagation();
        } else if key == "delete" {
            self.rename_delete(window, cx);
            cx.stop_propagation();
        } else if key == "left" {
            self.rename_move_left(cx);
            cx.stop_propagation();
        } else if key == "right" {
            self.rename_move_right(cx);
            cx.stop_propagation();
        } else if !event.keystroke.modifiers.platform
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
        {
            if let Some(text) = event.keystroke.key_char.as_deref() {
                self.replace_rename_text(None, text, cx);
                cx.stop_propagation();
            }
        }
    }

    pub(super) fn rename_mouse_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release_browser_focus();
        window.focus(&self.rename_focus_handle, cx);
        let end = self.rename_draft.len();
        self.rename_selection = 0..end;
        self.rename_selection_reversed = false;
        cx.notify();
    }

    pub(super) fn rename_cursor_offset(&self) -> usize {
        if self.rename_selection_reversed {
            self.rename_selection.start
        } else {
            self.rename_selection.end
        }
    }

    pub(super) fn rename_backspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename_selection.is_empty() {
            let cursor = self.rename_cursor_offset();
            let start = previous_utf8_boundary(&self.rename_draft, cursor);
            if start == cursor {
                window.play_system_bell();
                return;
            }
            self.rename_selection = start..cursor;
        }
        self.replace_rename_text(None, "", cx);
    }

    pub(super) fn rename_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename_selection.is_empty() {
            let cursor = self.rename_cursor_offset();
            let end = next_utf8_boundary(&self.rename_draft, cursor);
            if end == cursor {
                window.play_system_bell();
                return;
            }
            self.rename_selection = cursor..end;
        }
        self.replace_rename_text(None, "", cx);
    }

    pub(super) fn rename_move_left(&mut self, cx: &mut Context<Self>) {
        let offset = if self.rename_selection.is_empty() {
            previous_utf8_boundary(&self.rename_draft, self.rename_cursor_offset())
        } else {
            self.rename_selection.start
        };
        self.rename_selection = offset..offset;
        self.rename_selection_reversed = false;
        cx.notify();
    }

    pub(super) fn rename_move_right(&mut self, cx: &mut Context<Self>) {
        let offset = if self.rename_selection.is_empty() {
            next_utf8_boundary(&self.rename_draft, self.rename_cursor_offset())
        } else {
            self.rename_selection.end
        };
        self.rename_selection = offset..offset;
        self.rename_selection_reversed = false;
        cx.notify();
    }

    pub(super) fn replace_rename_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        let current = self.rename_draft.clone();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(&current, range.start)
                    ..byte_offset_for_utf16(&current, range.end)
            })
            .or_else(|| self.rename_marked_range.clone())
            .unwrap_or_else(|| self.rename_selection.clone());
        let mut updated = current;
        updated.replace_range(range.clone(), replacement);
        let cursor = range.start + replacement.len();
        self.rename_selection = cursor..cursor;
        self.rename_selection_reversed = false;
        self.rename_marked_range = None;
        self.rename_draft = updated;
        cx.notify();
    }

    pub(super) fn replace_rename_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        let current = self.rename_draft.clone();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(&current, range.start)
                    ..byte_offset_for_utf16(&current, range.end)
            })
            .or_else(|| self.rename_marked_range.clone())
            .unwrap_or_else(|| self.rename_selection.clone());
        let mut updated = current;
        updated.replace_range(range.clone(), replacement);
        let replacement_end = range.start + replacement.len();
        self.rename_marked_range =
            (!replacement.is_empty()).then_some(range.start..replacement_end);
        self.rename_selection = new_selected_range_utf16
            .map(|new_range| {
                let start = byte_offset_for_utf16(replacement, new_range.start);
                let end = byte_offset_for_utf16(replacement, new_range.end);
                range.start + start..range.start + end
            })
            .unwrap_or(replacement_end..replacement_end);
        self.rename_selection_reversed = false;
        self.rename_draft = updated;
        cx.notify();
    }

    pub(super) fn assign_session_group(
        &mut self,
        session_id: String,
        group_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.send_command(ClientMessage::AssignSessionGroup {
            session_id,
            group_id,
            position: None,
        });
        self.session_menu_session = None;
        self.drag_over_group = None;
        cx.notify();
    }

    pub(super) fn apply_client_update(
        &mut self,
        events: &[ClientEvent],
        state: &ClientState,
        cx: &mut Context<Self>,
    ) {
        for event in events {
            match event {
                ClientEvent::TerminalOpened { terminal_id } => {
                    self.terminal_id = Some(terminal_id.clone());
                    self.terminal_view.update(cx, |view, cx| view.clear(cx));
                    self.terminal_status = "running".into();
                }
                ClientEvent::TerminalOutput { terminal_id, bytes }
                    if self.terminal_id.as_deref() == Some(terminal_id.as_str()) =>
                {
                    self.terminal_view
                        .update(cx, |view, cx| view.ingest(bytes, cx));
                    self.terminal_status = "running".into();
                }
                ClientEvent::TerminalExited {
                    terminal_id,
                    status,
                } if self.terminal_id.as_deref() == Some(terminal_id.as_str()) => {
                    self.terminal_status = status.clone();
                }
                ClientEvent::TerminalError {
                    terminal_id,
                    message,
                } if terminal_id
                    .as_deref()
                    .is_none_or(|id| self.terminal_id.as_deref() == Some(id)) =>
                {
                    self.terminal_status = message.clone();
                }
                ClientEvent::ToolProgress {
                    session_id,
                    call_id,
                    chunk,
                } => {
                    self.model.append_tool_progress(session_id, call_id, chunk);
                }
                ClientEvent::BrowserStateChanged {
                    open, url, title, ..
                } => {
                    self.browser_panel_open = *open;
                    if let Some(url) = url {
                        self.browser_url = url.clone();
                    }
                    if let Some(title) = title {
                        self.browser_title = title.clone();
                    }
                    self.browser_error = None;
                }
                ClientEvent::BrowserSnapshot { url, title, .. } => {
                    self.browser_panel_open = true;
                    self.browser_url = url.clone();
                    self.browser_title = title.clone();
                    self.browser_error = None;
                }
                ClientEvent::BrowserScreenshot { url, .. } => {
                    self.browser_panel_open = true;
                    self.browser_url = url.clone();
                }
                ClientEvent::BrowserError { message, .. } => {
                    self.browser_error = Some(message.clone());
                }
                _ => {}
            }
        }
        let ready_session_id = events.iter().find_map(|event| match event {
            ClientEvent::SessionReady { session_id } => Some(session_id.as_str()),
            _ => None,
        });
        let has_browser_event = events.iter().any(|event| {
            matches!(
                event,
                ClientEvent::BrowserStateChanged { .. }
                    | ClientEvent::BrowserSnapshot { .. }
                    | ClientEvent::BrowserScreenshot { .. }
                    | ClientEvent::BrowserError { .. }
            )
        });
        self.model.apply_events(events);
        if let Some(session_id) = ready_session_id {
            if should_track_session_ready(
                self.model.ui.selected_session.as_deref(),
                self.pending_session_target.as_deref(),
                session_id,
            ) {
                self.ensure_session_view_state(session_id);
            }
        }
        if client_update_requires_metadata_sync(events) {
            if let Some(target) = self.pending_session_target.as_deref() {
                self.model
                    .sync_client_metadata_while_attaching(state, target);
            } else {
                self.model.sync_client_metadata(state);
            }
            self.rebuild_sidebar_rows();
        }
        if events
            .iter()
            .any(|event| matches!(event, ClientEvent::FileTreeChanged))
        {
            self.apply_file_tree_update(state);
        }
        self.sync_workspace_watch();
        if has_browser_event {
            self.capture_session_view_state();
        }
        if events.iter().any(|event| {
            matches!(
                event,
                ClientEvent::SessionListChanged
                    | ClientEvent::SessionMetaChanged { .. }
                    | ClientEvent::SessionReady { .. }
                    | ClientEvent::SessionHistoryLoaded { .. }
            )
        }) {
            self.refresh_open_tab_titles();
        }
        if let Some(target) = self.pending_session_target.as_deref() {
            self.model.ui.selected_session = Some(target.to_owned());
        }
        if events.iter().any(|event| {
            matches!(
                event,
                ClientEvent::SessionReady { .. }
                    | ClientEvent::SessionHistoryLoaded { .. }
                    | ClientEvent::MessageChanged { .. }
                    | ClientEvent::FlaggedFilesChanged { .. }
                    | ClientEvent::SessionMetaChanged { .. }
            )
        }) {
            self.refresh_review(cx);
        }
        if let Some(session_id) = self.model.ui.selected_session.clone() {
            if !self
                .open_tabs
                .iter()
                .any(|tab| tab.session_id.as_deref() == Some(session_id.as_str()))
            {
                if self.pending_session_request && self.pending_session_target.is_none() {
                    self.bind_new_tab_to_session(&session_id);
                } else {
                    self.open_session_tab(&session_id);
                }
            }
        }
        if events.iter().any(client_event_is_text_delta) {
            for event in events {
                if let ClientEvent::TextDelta {
                    session_id, delta, ..
                } = event
                {
                    self.model.append_transcript_delta(session_id, delta);
                }
            }
            self.sync_markdown_cache();
        } else if self.model.session_is_ready()
            && events.iter().any(client_event_requires_transcript_snapshot)
        {
            self.model.sync_client_transcript(state);
            self.markdown_cache.clear();
            self.tool_text_lists.borrow_mut().clear();
            self.tool_text_cache.borrow_mut().clear();
            self.sync_markdown_cache();
        }
        let session_list_changed = events
            .iter()
            .any(|event| matches!(event, ClientEvent::SessionListChanged));
        if session_list_changed
            && !self.pending_session_request
            && self.model.ui.selected_session.is_none()
        {
            let session_id = self
                .model
                .ui
                .conversations
                .iter()
                .find(|conversation| !conversation.archived)
                .map(|conversation| conversation.session_id.clone());
            if let Some(session_id) = session_id {
                if let Some(command) = self.model.ui.select_session(&session_id) {
                    self.restore_session_view_state(&session_id, cx);
                    self.pending_session_request = true;
                    self.pending_session_target = Some(session_id);
                    self.send_command(command);
                }
            }
        }
        let session_ready = ready_session_id.is_some_and(|session_id| {
            self.pending_session_request
                && self
                    .pending_session_target
                    .as_deref()
                    .is_none_or(|target| target == session_id)
        });
        if session_ready {
            self.terminal_view.update(cx, |view, cx| view.clear(cx));
            self.terminal_status = "opening".into();
            self.send_command(ClientMessage::TerminalOpen {
                rows: TERMINAL_EXPANDED_ROWS,
                cols: TERMINAL_COLS,
            });
            self.send_command(ClientMessage::ListPersonas);
            self.pending_session_request = false;
            self.pending_session_target = None;
            if let Some((provider, model)) = self.pending_model.take() {
                self.awaiting_model_switch = Some((provider.clone(), model.clone()));
                self.send_command(ClientMessage::SwitchModel { provider, model });
            }
        }
        if let Some((provider, model)) = &self.awaiting_model_switch {
            let switch_completed = state.current_provider.as_deref() == Some(provider)
                && state.current_model.as_deref() == Some(model);
            if switch_completed {
                self.awaiting_model_switch = None;
            }
        }
        if self.awaiting_model_switch.is_none()
            && (session_ready || self.model.ui.selected_session.is_some())
        {
            self.send_pending_prompt();
        }
    }
}

fn refresh_tab_title(tab: &mut OpenConversationTab, conversations: &[ConversationItem]) {
    let Some(session_id) = tab.session_id.as_deref() else {
        return;
    };
    let Some(conversation) = conversations
        .iter()
        .find(|conversation| conversation.session_id == session_id)
    else {
        return;
    };
    tab.title.clone_from(&conversation.title);
}

fn remove_unbound_tabs(tabs: &mut Vec<OpenConversationTab>) {
    tabs.retain(|tab| tab.session_id.is_some());
}

fn active_tab_after_close(
    active_tab: Option<usize>,
    removed_index: usize,
    remaining_len: usize,
) -> Option<usize> {
    let last_index = remaining_len.checked_sub(1)?;
    let current_index = active_tab.unwrap_or(removed_index);
    let adjusted_index = if current_index > removed_index {
        current_index - 1
    } else {
        current_index
    };
    Some(adjusted_index.min(last_index))
}

fn open_tab_for_session(
    tabs: &mut Vec<OpenConversationTab>,
    active_tab: Option<usize>,
    tab: OpenConversationTab,
) -> usize {
    let Some(session_id) = tab.session_id.as_deref() else {
        return active_tab.unwrap_or_else(|| {
            tabs.push(tab);
            tabs.len() - 1
        });
    };

    if tabs
        .iter()
        .any(|existing| existing.session_id.as_deref() == Some(session_id))
    {
        remove_unbound_tabs(tabs);
        let Some(index) = tabs
            .iter()
            .position(|existing| existing.session_id.as_deref() == Some(session_id))
        else {
            tabs.push(tab);
            return tabs.len() - 1;
        };
        tabs[index].title = tab.title;
        return index;
    }

    let index = active_tab.filter(|index| {
        tabs.get(*index)
            .is_some_and(|existing| existing.session_id.is_none())
    });
    if let Some(index) = index {
        tabs[index] = tab;
        index
    } else {
        tabs.push(tab);
        tabs.len() - 1
    }
}

fn session_view_state_or_current(
    states: &mut BTreeMap<String, SessionViewState>,
    session_id: &str,
    current: SessionViewState,
) -> SessionViewState {
    states
        .entry(session_id.to_owned())
        .or_insert(current)
        .clone()
}

fn should_track_session_ready(
    selected_session: Option<&str>,
    pending_session_target: Option<&str>,
    ready_session_id: &str,
) -> bool {
    selected_session == Some(ready_session_id)
        && pending_session_target.is_none_or(|target| target == ready_session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_fallback_tab_title_after_session_metadata_arrives() {
        let mut tab = OpenConversationTab {
            session_id: Some("session-1".into()),
            title: "New conversation".into(),
        };
        let conversations = [ConversationItem {
            session_id: "session-1".into(),
            title: "A real session".into(),
            cwd: None,
            last_message_at: None,
            state: mew_protocol::SessionState::Idle,
            last_turn_failed: false,
            needs_attention: false,
            archived: false,
            pinned: false,
            group_id: None,
        }];

        refresh_tab_title(&mut tab, &conversations);

        assert_eq!(tab.title, "A real session");
    }

    #[test]
    fn first_visit_inherits_current_session_view_state() {
        let current = SessionViewState {
            layout: ShellLayoutState {
                sidebar_collapsed: true,
                workbench_collapsed: false,
                terminal_collapsed: true,
                changes_expanded: false,
                local_expanded: true,
                activity_expanded: false,
            },
            auxiliary_view: AuxiliaryView::Activity,
            workbench_width: 420.,
            expanded_chat_parts: BTreeSet::from(["tool-1".into()]),
            browser_panel_open: true,
            browser_url: "https://example.com".into(),
            browser_title: "Example".into(),
        };
        let mut states = BTreeMap::new();

        let restored = session_view_state_or_current(&mut states, "session-1", current.clone());

        assert_eq!(restored, current);
        assert_eq!(states.get("session-1"), Some(&current));
    }

    #[test]
    fn stale_session_ready_events_cannot_replace_active_view_state() {
        assert!(should_track_session_ready(
            Some("session-1"),
            Some("session-1"),
            "session-1"
        ));
        assert!(should_track_session_ready(
            Some("session-1"),
            None,
            "session-1"
        ));
        assert!(!should_track_session_ready(
            Some("session-1"),
            Some("session-1"),
            "session-2"
        ));
        assert!(!should_track_session_ready(
            Some("session-1"),
            Some("session-2"),
            "session-1"
        ));
    }

    #[test]
    fn removes_placeholder_tabs_when_opening_an_existing_session() {
        let mut tabs = vec![
            OpenConversationTab {
                session_id: Some("session-1".into()),
                title: "A real session".into(),
            },
            OpenConversationTab {
                session_id: None,
                title: "New conversation".into(),
            },
        ];

        remove_unbound_tabs(&mut tabs);

        assert_eq!(
            tabs,
            vec![OpenConversationTab {
                session_id: Some("session-1".into()),
                title: "A real session".into(),
            }]
        );
    }

    #[test]
    fn opening_a_session_does_not_require_a_placeholder_tab() {
        let mut tabs = Vec::new();
        let session_tab = OpenConversationTab {
            session_id: Some("session-1".into()),
            title: "A real session".into(),
        };

        let index = open_tab_for_session(&mut tabs, None, session_tab.clone());

        assert_eq!(index, 0);
        assert_eq!(tabs, vec![session_tab]);
        assert!(tabs.iter().all(|tab| tab.session_id.is_some()));
    }

    #[test]
    fn opening_a_session_reuses_only_the_active_placeholder() {
        let mut tabs = vec![
            OpenConversationTab {
                session_id: None,
                title: "New conversation".into(),
            },
            OpenConversationTab {
                session_id: None,
                title: "New conversation".into(),
            },
        ];
        let session_tab = OpenConversationTab {
            session_id: Some("session-1".into()),
            title: "A real session".into(),
        };

        let index = open_tab_for_session(&mut tabs, Some(1), session_tab.clone());

        assert_eq!(index, 1);
        assert_eq!(
            tabs,
            vec![
                OpenConversationTab {
                    session_id: None,
                    title: "New conversation".into(),
                },
                session_tab,
            ]
        );
    }

    #[test]
    fn closing_a_tab_adjusts_the_active_index_for_the_removed_tab() {
        assert_eq!(active_tab_after_close(Some(2), 0, 2), Some(1));
        assert_eq!(active_tab_after_close(Some(2), 1, 2), Some(1));
        assert_eq!(active_tab_after_close(Some(0), 1, 2), Some(0));
        assert_eq!(active_tab_after_close(Some(1), 1, 2), Some(1));
        assert_eq!(active_tab_after_close(None, 1, 2), Some(1));
        assert_eq!(active_tab_after_close(Some(1), 1, 0), None);
    }
}
