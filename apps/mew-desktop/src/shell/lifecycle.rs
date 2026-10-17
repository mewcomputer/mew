use super::*;

pub(super) fn transcript_is_scrolled_away(
    is_scrolled: bool,
    visible_end: usize,
    count: usize,
    is_following_tail: bool,
) -> bool {
    is_scrolled && visible_end < count && !is_following_tail
}

impl DesktopShell {
    fn schedule_streaming_render(&mut self, cx: &mut Context<Self>) {
        if self.streaming_render_scheduled {
            return;
        }
        self.streaming_render_scheduled = true;
        let shell = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            if let Some(shell) = shell.upgrade() {
                shell.update(cx, |shell, cx| {
                    shell.streaming_render_scheduled = false;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn reload_theme(&mut self, appearance: WindowAppearance, cx: &mut Context<Self>) {
        let theme_name = desktop_theme_name(
            self.theme_mode,
            appearance,
            &self.light_theme,
            &self.dark_theme,
        );
        self.theme = Theme::load(theme_name);
        let theme = self.theme.clone();
        self.terminal_view.update(cx, |view, cx| {
            view.set_theme_colors(
                theme_rgb(&theme, "text.body"),
                theme_rgb(&theme, "panel.background"),
                cx,
            );
        });
        cx.notify();
    }

    pub(super) fn new(
        endpoint: Option<DaemonEndpoint>,
        supervisor: Option<DesktopSupervisor>,
        startup_error: Option<String>,
        remote_profile: Option<DesktopConnectionProfile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let endpoint_url = endpoint.map(|endpoint| endpoint.websocket_url);
        let persisted_state = mew_config::load_state().unwrap_or_else(|error| {
            tracing::warn!(%error, "desktop: could not load persisted layout");
            mew_config::State::default()
        });
        let mut persisted_layout = ShellLayoutState::from_state(&persisted_state);
        let collapsed_groups = persisted_state
            .desktop_collapsed_groups
            .iter()
            .cloned()
            .collect();
        let expanded_workspaces = BTreeSet::new();
        let workspace_open_destinations = detect_workspace_open_destinations();
        let sidebar_width = persisted_state
            .desktop_sidebar_width
            .unwrap_or(SIDEBAR_EXPANDED_WIDTH)
            .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        let window_width = f32::from(window.bounds().size.width);
        if !persisted_layout.sidebar_collapsed
            && !workbench_fits_window(window_width, sidebar_width)
        {
            persisted_layout.workbench_collapsed = true;
        }
        let session_view_states = persisted_state
            .desktop_session_views
            .iter()
            .map(|(session_id, state)| {
                (session_id.clone(), SessionViewState::from_persisted(state))
            })
            .collect();
        let mut remote_profiles = persisted_state.desktop_remote_profiles.clone();
        let connection_profile_selection = remote_profile
            .as_ref()
            .and_then(|profile| match profile {
                DesktopConnectionProfile::RemoteIroh { node_id, .. } => Some(node_id.clone()),
                DesktopConnectionProfile::LocalWebSocket { .. } => None,
            })
            .or_else(|| persisted_state.desktop_active_remote_profile.clone());
        if let Some(DesktopConnectionProfile::RemoteIroh {
            node_id,
            device_name,
            ..
        }) = remote_profile.as_ref()
        {
            if !remote_profiles
                .iter()
                .any(|profile| profile.node_id == *node_id)
            {
                remote_profiles.push(mew_config::DesktopRemoteProfile {
                    name: "remote daemon".into(),
                    node_id: node_id.clone(),
                    device_name: device_name.clone(),
                });
            }
        }
        let connection_profile = remote_profile.or_else(|| {
            endpoint_url
                .clone()
                .map(|url| DesktopConnectionProfile::LocalWebSocket { url })
        });
        let client_profile = connection_profile.clone();
        let mut model = ShellModel::starting(
            connection_profile
                .as_ref()
                .map(DesktopConnectionProfile::endpoint_label),
        );
        let (command_tx, client_stop_tx, client_thread) = if let Some(profile) = connection_profile
        {
            let (command_tx, stop_tx, thread) = Self::start_client(profile, 0, cx);
            (Some(command_tx), Some(stop_tx), thread)
        } else {
            if let Some(error) = startup_error {
                model.fail(error);
            }
            (None, None, None)
        };
        let theme_mode = DesktopThemeMode::parse(&persisted_state.desktop_theme_mode);
        let light_theme = if persisted_state.desktop_light_theme.is_empty() {
            "light".to_owned()
        } else {
            persisted_state.desktop_light_theme.clone()
        };
        let dark_theme = if persisted_state.desktop_dark_theme.is_empty() {
            "dark".to_owned()
        } else {
            persisted_state.desktop_dark_theme.clone()
        };
        let theme = Theme::load(desktop_theme_name(
            theme_mode,
            window.appearance(),
            &light_theme,
            &dark_theme,
        ));
        let terminal_font_family = if persisted_state.terminal_font.is_empty() {
            DEFAULT_FONT_FAMILY.to_owned()
        } else {
            persisted_state.terminal_font.clone()
        };
        if let Err(error) = cx.text_system().add_fonts(vec![
            Cow::Borrowed(BUNDLED_MONO_REGULAR),
            Cow::Borrowed(BUNDLED_MONO_MEDIUM),
        ]) {
            tracing::warn!(%error, "desktop: could not register bundled terminal font");
        }
        let appearance_subscription = cx.observe_window_appearance(window, |shell, window, cx| {
            shell.reload_theme(window.appearance(), cx);
        });
        let bounds_subscription = cx.observe_window_bounds(window, |shell, window, cx| {
            let changed = shell.update_window_width(f32::from(window.bounds().size.width));
            if changed {
                shell.persist_layout();
                cx.notify();
            }
            shell.persist_window_bounds(window);
        });
        let terminal_view = cx.new(TerminalView::new_remote);
        terminal_view.update(cx, |view, cx| {
            view.set_theme_colors(
                theme_rgb(&theme, "text.body"),
                theme_rgb(&theme, "panel.background"),
                cx,
            );
            view.set_font_family(terminal_font_family.clone(), cx);
        });
        let composer_focus_handle = cx.focus_handle();
        let browser_url_focus_handle = cx.focus_handle();
        let rename_focus_handle = cx.focus_handle();
        let sidebar_search_focus_handle = cx.focus_handle().tab_stop(true);
        let model_picker_focus_handle = cx.focus_handle().tab_stop(true);
        let sidebar_focus_handle = cx.focus_handle().tab_stop(true);
        let popover_focus_handle = cx.focus_handle();
        let composer_focus_subscription =
            cx.on_focus(&composer_focus_handle, window, |shell, _window, cx| {
                shell.restart_composer_cursor_blink(cx)
            });
        let composer_blur_subscription =
            cx.on_blur(&composer_focus_handle, window, |shell, _window, cx| {
                shell.stop_composer_cursor_blink(cx);
            });
        let app_quit_subscription = cx.on_app_quit(|shell, _cx| {
            shell.prepare_for_quit();
            async {}
        });
        let terminal_subscription =
            cx.subscribe(&terminal_view, |shell, _terminal, event, _cx| match event {
                TerminalEvent::Input(bytes) => {
                    if let Some(terminal_id) = &shell.terminal_id {
                        shell.send_command(ClientMessage::TerminalInput {
                            terminal_id: terminal_id.clone(),
                            bytes: bytes.clone(),
                        });
                    }
                }
                TerminalEvent::Resize { cols, rows } => {
                    if let Some(terminal_id) = &shell.terminal_id {
                        shell.send_command(ClientMessage::TerminalResize {
                            terminal_id: terminal_id.clone(),
                            rows: *rows,
                            cols: *cols,
                        });
                    }
                }
            });
        let transcript_list = gpui::ListState::new(0, gpui::ListAlignment::Bottom, px(128.));
        transcript_list.set_follow_mode(gpui::FollowMode::Tail);
        let shell_ref = cx.entity().downgrade();
        transcript_list.set_scroll_handler(move |event, _window, cx| {
            let is_scrolled = event.is_scrolled;
            let scrolled_away = transcript_is_scrolled_away(
                is_scrolled,
                event.visible_range.end,
                event.count,
                event.is_following_tail,
            );
            let reached_history_top =
                is_scrolled && event.count > 0 && event.visible_range.start == 0;
            let shell_ref = shell_ref.clone();
            cx.defer(move |cx| {
                let _ = shell_ref.update(cx, |shell, _cx| {
                    // ListState already invalidates the containing view for the scroll repaint.
                    // Keep this deferred work limited to state and history updates so a wheel
                    // tick does not enqueue a second full-shell render.
                    if shell.transcript_scrolled_away != scrolled_away {
                        shell.transcript_scrolled_away = scrolled_away;
                        shell.capture_session_view_state();
                    }
                    if reached_history_top {
                        shell.request_older_history();
                    }
                });
            });
        });
        let frame_probe = DesktopFrameProbe::from_env();
        let draft_profile = client_profile
            .as_ref()
            .map(DesktopConnectionProfile::endpoint_label)
            .unwrap_or_default();
        Self {
            model,
            theme,
            theme_mode,
            light_theme,
            dark_theme,
            open_tabs: Vec::new(),
            active_tab: None,
            tab_scroll_handle: gpui::ScrollHandle::new(),
            layout: persisted_layout,
            sidebar_rows: Vec::new(),
            sidebar_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(40.)),
            sidebar_width,
            collapsed_groups,
            expanded_workspaces,
            workspace_open_menu: None,
            workspace_open_destinations,
            remembered_editor: persisted_state.desktop_external_editor,
            sidebar_search: String::new(),
            sidebar_filter: SidebarFilter::All,
            sidebar_search_focus_handle,
            sidebar_focus_handle,
            sidebar_keyboard_session: None,
            sidebar_search_selection: 0..0,
            sidebar_search_selection_reversed: false,
            sidebar_search_marked_range: None,
            model_picker_query: String::new(),
            model_picker_focus_handle,
            model_picker_selection: 0..0,
            model_picker_selection_reversed: false,
            model_picker_marked_range: None,
            model_picker_filtered_indices: Vec::new(),
            model_picker_rows: Vec::new(),
            recent_models: persisted_state.recent_models,
            thinking_effort_drag_index: None,
            thinking_effort_drag_position: None,
            thinking_effort_track_bounds: None,
            thinking_effort_animation_id: 0,
            thinking_effort_animation_from: None,
            pending_group_deletion: None,
            session_view_states,
            session_menu_session: None,
            hovered_group: None,
            drag_over_group: None,
            rename_session_id: None,
            rename_draft: String::new(),
            rename_focus_handle,
            popover_focus_handle,
            rename_selection: 0..0,
            rename_selection_reversed: false,
            rename_marked_range: None,
            sidebar_animation_id: 0,
            workbench_animation_id: 0,
            terminal_animation_id: 0,
            window_width,
            workbench_width: WORKBENCH_EXPANDED_WIDTH,
            auxiliary_view: AuxiliaryView::Changes,
            transcript_list,
            transcript_scrolled_away: false,
            transcript_scrollbar_grab_offset: None,
            history_before: None,
            history_loading: false,
            pending_transcript_scroll_anchor: None,
            transcript_rows: Vec::new(),
            transcript_rows_append_only: false,
            transcript_rows_prepend_count: None,
            markdown_cache: Vec::new(),
            tool_text_lists: RefCell::new(BTreeMap::new()),
            tool_text_cache: RefCell::new(BTreeMap::new()),
            transcript_text_registry: Rc::new(RefCell::new(Vec::new())),
            transcript_selection: None,
            transcript_selection_anchor: None,
            transcript_is_selecting: false,
            transcript_link_candidate: None,
            transcript_selected_text: None,
            review_diffs: Vec::new(),
            review_lines: Vec::new(),
            review_selected_file: None,
            review_line_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(2048.)),
            review_requests: review::ReviewRequests::default(),
            review_loading: false,
            review_error: None,
            input_animation_id: 0,
            composer_focus_handle,
            composer_selection: 0..0,
            composer_selection_reversed: false,
            composer_marked_range: None,
            composer_is_selecting: false,
            composer_bounds: None,
            browser_url_focus_handle,
            browser_url_focus_requested: false,
            browser_url_selection: 0..0,
            browser_url_selection_reversed: false,
            browser_url_marked_range: None,
            browser_url_bounds: None,
            browser_native_rect: None,
            model_picker_bounds: None,
            persona_picker_bounds: None,
            permission_picker_bounds: None,
            slash_menu_dismissed: false,
            slash_menu_index: 0,
            mention_menu_dismissed: false,
            mention_menu_index: 0,
            file_tree_entries: BTreeMap::new(),
            file_tree_expanded: BTreeSet::new(),
            file_tree_pending: BTreeSet::new(),
            watching_workspace_session: None,
            prompt_history: PromptHistory::default(),
            composer_cursor_visible: true,
            composer_blink_epoch: 0,
            streaming_render_scheduled: false,
            expanded_chat_parts: BTreeSet::new(),
            task_drafts: TaskDrafts::new(DraftOwner::new_task(&draft_profile)),
            initial_auto_attach: true,
            attachments: Vec::new(),
            attachment_error: None,
            pending_model: None,
            awaiting_model_switch: None,
            session_requests: SessionRequests::default(),
            model_picker_open: false,
            persona_picker_open: false,
            permission_picker_open: false,
            terminal_font_picker_open: false,
            terminal_font_family,
            terminal_view,
            terminal_id: None,
            terminal_status: "closed".into(),
            browser_portal: None,
            browser_panel_open: false,
            browser_initialization_pending: None,
            browser_url: String::new(),
            browser_title: String::new(),
            browser_error: None,
            browser_confirmed_sessions: BTreeSet::new(),
            browser_registration_attempts: BTreeMap::new(),
            browser_pump: None,
            settings_open: false,
            settings_page: SettingsPage::General,
            connection_picker_open: false,
            remote_profiles,
            connection_profile_selection,
            command_tx,
            connection_profile: client_profile,
            client_stop_tx,
            client_thread,
            _terminal_subscription: terminal_subscription,
            _composer_focus_subscription: composer_focus_subscription,
            _composer_blur_subscription: composer_blur_subscription,
            _app_quit_subscription: app_quit_subscription,
            quitting: false,
            _appearance_subscription: appearance_subscription,
            _bounds_subscription: bounds_subscription,
            _supervisor: supervisor,
            supervisor_retry_pending: false,
            supervisor_startup_cancel: None,
            frame_probe,
        }
    }

    pub(super) fn prepare_for_quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        if let Some(cancellation) = self.supervisor_startup_cancel.take() {
            cancellation.cancel();
        }
        self.capture_session_view_state();
        self.persist_layout();
        self.close_terminal();
        self.stop_client();
        self.browser_panel_open = false;
        if let Some(portal) = self.browser_portal.as_mut() {
            portal.prepare_for_process_exit();
        }
        if let Some(supervisor) = self._supervisor.as_mut() {
            if let Err(error) = supervisor.shutdown() {
                tracing::warn!(%error, "desktop: could not stop daemon during app shutdown");
            }
        }
    }

    pub(super) fn retry_connection(&mut self, cx: &mut Context<Self>) {
        if self.quitting || self.supervisor_retry_pending {
            return;
        }

        self.stop_client();
        let profile = self.connection_profile.clone();
        if should_refresh_supervisor(profile.as_ref(), self._supervisor.as_ref()) {
            let Some(supervisor) = self._supervisor.as_ref() else {
                self.model.fail("no daemon connection is configured");
                cx.notify();
                return;
            };
            let cancellation = supervisor.startup_cancellation();
            self.supervisor_startup_cancel = Some(cancellation.clone());
            self.supervisor_retry_pending = true;
            self.model.connection = crate::model::ShellConnection::Starting;
            self.model.last_error = None;
            let generation = self.session_requests.generation();
            let profile_selection = self.connection_profile_selection.clone();
            let result_rx = match spawn_supervisor_retry(
                &mut self._supervisor,
                cancellation,
                std::thread::Builder::new().name("mew-desktop-supervisor".into()),
            ) {
                Ok(receiver) => receiver,
                Err(error) => {
                    self.supervisor_startup_cancel.take();
                    self.supervisor_retry_pending = false;
                    self.model
                        .fail(format!("could not start daemon connection worker: {error}"));
                    cx.notify();
                    return;
                }
            };

            cx.spawn(async move |this, cx| {
                let result = result_rx.await;
                let _ = this.update(cx, |shell, cx| {
                    shell.supervisor_retry_pending = false;
                    shell.supervisor_startup_cancel.take();
                    if shell.quitting {
                        return;
                    }
                    let is_current = shell.session_requests.accepts_source(generation)
                        && shell.connection_profile == profile
                        && shell.connection_profile_selection == profile_selection;
                    let (supervisor, result) = match result {
                        Ok(result) => result,
                        Err(error) => {
                            if is_current {
                                shell
                                    .model
                                    .fail(format!("daemon connection worker stopped: {error}"));
                                cx.notify();
                            }
                            return;
                        }
                    };
                    shell._supervisor = Some(supervisor);
                    if !is_current {
                        return;
                    }
                    match result {
                        Ok(endpoint) => shell.reconnect_with_profile(
                            DesktopConnectionProfile::LocalWebSocket {
                                url: endpoint.websocket_url,
                            },
                            cx,
                        ),
                        Err(error) => {
                            shell.model.fail(error.to_string());
                            cx.notify();
                        }
                    }
                });
            })
            .detach();
            cx.notify();
            return;
        }

        let Some(profile) = profile else {
            self.model.fail("no daemon connection is configured");
            cx.notify();
            return;
        };
        self.reconnect_with_profile(profile, cx);
    }

    fn reconnect_with_profile(
        &mut self,
        profile: DesktopConnectionProfile,
        cx: &mut Context<Self>,
    ) {
        let selected_session = self.model.ui.selected_session.clone();
        self.connection_profile = Some(profile.clone());
        self.switch_draft_owner(selected_session.as_deref());
        let composer = std::mem::take(&mut self.model.ui.composer);
        self.model = ShellModel::starting(Some(profile.endpoint_label()));
        self.model.ui.composer = composer;
        self.model.ui.selected_session = selected_session.clone();
        let (command_tx, stop_tx, thread) =
            Self::start_client(profile, self.session_requests.generation(), cx);
        self.command_tx = Some(command_tx);
        self.client_stop_tx = Some(stop_tx);
        self.client_thread = thread;
        cx.notify();
    }

    fn stop_client(&mut self) {
        self.session_requests.reset_connection();
        self.reset_browser_connection();
        if let Some(stop_tx) = self.client_stop_tx.take() {
            let _ = stop_tx.send(());
        }
        self.command_tx.take();
        if let Some(thread) = self.client_thread.take() {
            if thread.join().is_err() {
                tracing::warn!("desktop: client thread panicked during client shutdown");
            }
        }
    }

    pub(super) fn start_client(
        profile: DesktopConnectionProfile,
        generation: usize,
        cx: &mut Context<Self>,
    ) -> (
        UnboundedSender<ClientMessage>,
        oneshot::Sender<()>,
        Option<std::thread::JoinHandle<()>>,
    ) {
        let (command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded();
        let thread_event_tx = event_tx.clone();
        let thread = std::thread::Builder::new()
            .name("mew-desktop-client".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = thread_event_tx
                            .unbounded_send(DesktopClientEvent::Failed(error.to_string()));
                        return;
                    }
                };
                runtime.block_on(Self::run_client(
                    profile,
                    &mut command_rx,
                    stop_rx,
                    thread_event_tx,
                ));
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                let _ = event_tx.unbounded_send(DesktopClientEvent::Failed(error.to_string()));
                None
            }
        };

        cx.spawn(async move |this, cx| {
            while let Some(event) = event_rx.next().await {
                let mut batch = vec![event];
                while let Ok(event) = event_rx.try_recv() {
                    batch.push(event);
                }
                this.update(cx, |shell, cx| {
                    if !shell.session_requests.accepts_source(generation) {
                        return;
                    }
                    let mut needs_shell_render = false;
                    let mut has_streaming_update = false;
                    for event in batch {
                        match event {
                            DesktopClientEvent::Connected => {
                                shell.reset_browser_connection();
                                shell.model.connected();
                                if let Some(session_id) =
                                    shell.task_drafts.owner().session_id.clone()
                                {
                                    shell.switch_draft_owner(Some(&session_id));
                                    shell.dispatch_attachment(&session_id);
                                }
                                needs_shell_render = true;
                            }
                            DesktopClientEvent::Updated { mut events, state } => {
                                if client_events_are_streaming_only(&events) {
                                    has_streaming_update = true;
                                } else {
                                    needs_shell_render |=
                                        client_events_require_shell_render(&events);
                                }
                                shell.apply_client_update(&mut events, &state, cx);
                            }
                            DesktopClientEvent::Failed(error) => {
                                shell.reset_browser_connection();
                                shell.model.fail(error);
                                needs_shell_render = true;
                            }
                        }
                    }
                    if needs_shell_render {
                        cx.notify();
                    } else if has_streaming_update {
                        shell.schedule_streaming_render(cx);
                    }
                })
                .ok();
            }
        })
        .detach();
        (command_tx, stop_tx, thread)
    }

    pub(super) async fn run_client(
        profile: DesktopConnectionProfile,
        command_rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
        stop_rx: oneshot::Receiver<()>,
        event_tx: mpsc::UnboundedSender<DesktopClientEvent>,
    ) {
        let transport: Box<dyn ClientTransport> = match profile {
            DesktopConnectionProfile::LocalWebSocket { url } => {
                Box::new(LocalWebSocketTransport::new(url))
            }
            DesktopConnectionProfile::RemoteIroh {
                node_id,
                pairing_token,
                device_name,
            } => {
                let endpoint = match iroh::Endpoint::builder(iroh::endpoint::presets::N0)
                    .bind()
                    .await
                {
                    Ok(endpoint) => endpoint,
                    Err(error) => {
                        let _ = event_tx.unbounded_send(DesktopClientEvent::Failed(format!(
                            "could not start iroh endpoint: {error}"
                        )));
                        return;
                    }
                };
                Box::new(IrohTransport::new(
                    endpoint,
                    node_id,
                    pairing_token,
                    device_name,
                ))
            }
        };
        Self::run_client_with_transport(transport, command_rx, stop_rx, event_tx).await;
    }

    pub(super) async fn run_client_with_transport(
        transport: Box<dyn ClientTransport>,
        command_rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
        mut stop_rx: oneshot::Receiver<()>,
        event_tx: mpsc::UnboundedSender<DesktopClientEvent>,
    ) {
        let mut engine = tokio::select! {
            result = ClientEngine::connect(transport.as_ref()) => match result {
                Ok(engine) => engine,
                Err(error) => {
                    let _ = event_tx.unbounded_send(DesktopClientEvent::Failed(error.to_string()));
                    return;
                }
            },
            _ = &mut stop_rx => return,
        };
        if event_tx
            .unbounded_send(DesktopClientEvent::Connected)
            .is_err()
        {
            return;
        }

        for message in [
            ClientMessage::Ping,
            ClientMessage::ListSessions,
            ClientMessage::ListProjects,
            ClientMessage::ListModels,
        ] {
            let result = tokio::select! {
                result = engine.send(message) => result,
                _ = &mut stop_rx => return,
            };
            if let Err(error) = result {
                let _ = event_tx.unbounded_send(DesktopClientEvent::Failed(error.to_string()));
                return;
            }
        }

        loop {
            tokio::select! {
                _ = &mut stop_rx => return,
                command = command_rx.recv() => {
                    let Some(command) = command else { return; };
                    let result = tokio::select! {
                        result = engine.send(command) => result,
                        _ = &mut stop_rx => return,
                    };
                    if let Err(error) = result {
                        let _ = event_tx.unbounded_send(DesktopClientEvent::Failed(error.to_string()));
                        return;
                    }
                }
                result = engine.receive() => match result {
                    Ok(events) => {
                        let state = if events.iter().any(|event| {
                            client_event_requires_transcript_snapshot(event)
                                || client_event_requires_session_snapshot(event)
                        }) {
                            engine.ui_snapshot()
                        } else {
                            engine.ui_metadata_snapshot()
                        };
                        if event_tx.unbounded_send(DesktopClientEvent::Updated {
                            events,
                            state: Box::new(state),
                        }).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = event_tx.unbounded_send(DesktopClientEvent::Failed(error.to_string()));
                        return;
                    }
                }
            }
        }
    }
}

fn spawn_supervisor_retry(
    supervisor: &mut Option<DesktopSupervisor>,
    cancellation: mew_desktop_supervisor::DaemonStartupCancellation,
    builder: std::thread::Builder,
) -> std::io::Result<oneshot::Receiver<(DesktopSupervisor, Result<DaemonEndpoint>)>> {
    let handoff = Arc::new(std::sync::Mutex::new(supervisor.take()));
    let worker_handoff = handoff.clone();
    let (result_tx, result_rx) = oneshot::channel();
    if let Err(error) = builder.spawn(move || {
        let mut supervisor = worker_handoff
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .expect("daemon supervisor is configured");
        let result = supervisor.connect_or_launch_with_cancellation(&cancellation);
        let _ = result_tx.send((supervisor, result));
    }) {
        *supervisor = handoff
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        return Err(error);
    }
    Ok(result_rx)
}

fn should_refresh_supervisor(
    profile: Option<&DesktopConnectionProfile>,
    supervisor: Option<&DesktopSupervisor>,
) -> bool {
    profile.is_none()
        || (supervisor.is_some()
            && matches!(
                profile,
                Some(DesktopConnectionProfile::LocalWebSocket { .. })
            ))
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use futures::future::pending;
    use mew_client_core::{ClientConnection, TransportError};

    struct SupervisorProbe {
        supervisor: DesktopSupervisor,
        ready: std::sync::mpsc::Receiver<()>,
        response: std::sync::mpsc::Sender<mew_protocol::ServerMessage>,
        server: std::thread::JoinHandle<()>,
    }

    fn held_supervisor_probe() -> SupervisorProbe {
        use tokio_tungstenite::tungstenite::{accept, Message};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready_tx, ready) = std::sync::mpsc::channel();
        let (response, response_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = accept(stream).unwrap();
            assert!(matches!(
                mew_protocol::decode_json::<ClientMessage>(
                    socket.read().unwrap().into_text().unwrap().as_ref()
                )
                .unwrap(),
                ClientMessage::Ping
            ));
            ready_tx.send(()).unwrap();
            if let Ok(response) = response_rx.recv_timeout(Duration::from_secs(3)) {
                let _ = socket.send(Message::Text(mew_protocol::encode_json(&response).unwrap()));
            }
        });
        SupervisorProbe {
            supervisor: DesktopSupervisor::new(SupervisorConfig {
                local_port: port,
                ..SupervisorConfig::default()
            }),
            ready,
            response,
            server,
        }
    }

    fn retry_test_shell(
        supervisor: DesktopSupervisor,
        cx: &mut gpui::TestAppContext,
    ) -> Entity<DesktopShell> {
        // Localhost peers and daemon subprocesses wake tasks from real OS threads.
        cx.background_executor.allow_parking();
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            DesktopShell::new(None, Some(supervisor), None, None, window, cx)
        });
        window.root(cx).unwrap()
    }

    fn finish_supervisor_retry(shell: &Entity<DesktopShell>, cx: &mut gpui::TestAppContext) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while shell.read_with(cx, |shell, _| shell.supervisor_retry_pending) {
            assert!(
                std::time::Instant::now() < deadline,
                "supervisor retry should finish"
            );
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn failed_supervisor_worker_spawn_preserves_the_next_connection_attempt() {
        use tokio_tungstenite::tungstenite::{accept, Message};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut supervisor = Some(DesktopSupervisor::new(SupervisorConfig {
            local_port: address.port(),
            ..SupervisorConfig::default()
        }));
        let cancellation = supervisor.as_ref().unwrap().startup_cancellation();

        let failed_spawn = spawn_supervisor_retry(
            &mut supervisor,
            cancellation.clone(),
            std::thread::Builder::new().stack_size(usize::MAX),
        );

        assert!(
            failed_spawn.is_err(),
            "the invalid stack size should fail to spawn"
        );
        assert!(
            supervisor.is_some(),
            "failed spawn should retain daemon configuration"
        );
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = accept(stream).unwrap();
            assert!(matches!(
                mew_protocol::decode_json::<ClientMessage>(
                    socket.read().unwrap().into_text().unwrap().as_ref()
                )
                .unwrap(),
                ClientMessage::Ping
            ));
            socket
                .send(Message::Text(
                    mew_protocol::encode_json(&mew_protocol::ServerMessage::Pong {
                        version: "test".into(),
                        protocol_version: mew_protocol::PROTOCOL_VERSION,
                    })
                    .unwrap(),
                ))
                .unwrap();
        });
        let receiver =
            spawn_supervisor_retry(&mut supervisor, cancellation, std::thread::Builder::new())
                .unwrap();
        let (_supervisor, endpoint) = futures::executor::block_on(receiver).unwrap();
        server.join().unwrap();

        assert_eq!(endpoint.unwrap().websocket_url, format!("ws://{address}"));
    }

    #[gpui::test]
    fn failed_supervisor_retry_keeps_foreground_draft_edits(cx: &mut gpui::TestAppContext) {
        let SupervisorProbe {
            supervisor,
            ready,
            response,
            server,
        } = held_supervisor_probe();
        let shell = retry_test_shell(supervisor, cx);
        shell.update(cx, |shell, cx| {
            shell.model.ui.selected_session = Some("selected-session".into());
            shell.model.ui.composer = "saved draft".into();
            shell.retry_connection(cx);
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        cx.run_until_parked();
        shell.update(cx, |shell, cx| {
            assert!(shell.supervisor_retry_pending);
            assert!(shell.model.last_error.is_none());
            shell.model.ui.composer.push_str(" edited during startup");
            let generation = shell.session_requests.generation();
            shell.retry_connection(cx);
            assert_eq!(shell.session_requests.generation(), generation);
        });
        response
            .send(mew_protocol::ServerMessage::Error {
                message: "daemon unavailable".into(),
            })
            .unwrap();
        server.join().unwrap();
        finish_supervisor_retry(&shell, cx);
        shell.update(cx, |shell, _| {
            assert!(shell
                .model
                .last_error
                .as_ref()
                .unwrap()
                .contains("already in use"));
            assert_eq!(shell.model.ui.composer, "saved draft edited during startup");
            assert_eq!(
                shell.model.ui.selected_session.as_deref(),
                Some("selected-session")
            );
            assert!(shell._supervisor.is_some());
            assert!(shell.command_tx.is_none());
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn supervisor_retry_ignores_completion_after_profile_changes(cx: &mut gpui::TestAppContext) {
        let SupervisorProbe {
            supervisor,
            ready,
            response,
            server,
        } = held_supervisor_probe();
        let shell = retry_test_shell(supervisor, cx);
        shell.update(cx, |shell, cx| shell.retry_connection(cx));
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        shell.update(cx, |shell, _| {
            shell.connection_profile_selection = Some("another-profile".into());
            shell.model.last_error = Some("profile changed".into());
        });
        response
            .send(mew_protocol::ServerMessage::Pong {
                version: "test".into(),
                protocol_version: mew_protocol::PROTOCOL_VERSION,
            })
            .unwrap();
        server.join().unwrap();
        finish_supervisor_retry(&shell, cx);
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.last_error.as_deref(), Some("profile changed"));
            assert!(shell._supervisor.is_some());
            assert!(shell.command_tx.is_none());
            assert!(shell.connection_profile.is_none());
            shell.quitting = true;
        });
    }

    #[cfg(unix)]
    #[gpui::test]
    fn quitting_during_supervisor_startup_reaps_its_child(cx: &mut gpui::TestAppContext) {
        use std::os::unix::fs::PermissionsExt;

        let directory =
            std::env::temp_dir().join(format!("mew-desktop-startup-cancel-{}", ulid::Ulid::new()));
        std::fs::create_dir(&directory).unwrap();
        let binary = directory.join("daemon");
        let pid_file = binary.with_extension("pid");
        std::fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' $$ > \"$0.pid\"\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let supervisor = DesktopSupervisor::new(SupervisorConfig {
            daemon_binary: Some(binary),
            startup_timeout: Duration::from_secs(5),
            ..SupervisorConfig::default()
        });
        let shell = retry_test_shell(supervisor, cx);
        shell.update(cx, |shell, cx| shell.retry_connection(cx));
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let child_pid = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                if pid.trim().parse::<u32>().is_ok() {
                    break pid.trim().to_owned();
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "daemon subprocess should start"
            );
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(1));
        };

        shell.update(cx, |shell, _| {
            assert!(shell.supervisor_retry_pending);
            shell.prepare_for_quit();
        });
        let child_was_reaped = !std::process::Command::new("kill")
            .args(["-0", &child_pid])
            .output()
            .unwrap()
            .status
            .success();
        finish_supervisor_retry(&shell, cx);
        std::fs::remove_dir_all(directory).unwrap();

        assert!(
            child_was_reaped,
            "quitting should reap the child before returning"
        );
        shell.read_with(cx, |shell, _| {
            assert!(shell.quitting);
            assert!(shell._supervisor.is_none());
            assert!(shell.command_tx.is_none());
            assert!(shell.supervisor_startup_cancel.is_none());
        });
    }

    #[test]
    fn supervised_local_retry_is_refreshed_even_after_its_cached_endpoint_is_cleared() {
        let supervisor = DesktopSupervisor::new(SupervisorConfig::default());
        assert!(supervisor.endpoint().is_none());
        let local = DesktopConnectionProfile::LocalWebSocket {
            url: "ws://127.0.0.1:25566".into(),
        };
        assert!(should_refresh_supervisor(Some(&local), Some(&supervisor)));
        assert!(!should_refresh_supervisor(Some(&local), None));
        let remote = DesktopConnectionProfile::RemoteIroh {
            node_id: "remote".into(),
            pairing_token: None,
            device_name: "desktop".into(),
        };
        assert!(!should_refresh_supervisor(Some(&remote), Some(&supervisor)));
    }

    struct PendingTransport;

    #[async_trait::async_trait]
    impl ClientTransport for PendingTransport {
        async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
            pending().await
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_stop_cancels_a_connection_attempt() {
        let (_command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (event_tx, _event_rx) = mpsc::unbounded();
        let task = tokio::spawn(async move {
            DesktopShell::run_client_with_transport(
                Box::new(PendingTransport),
                &mut command_rx,
                stop_rx,
                event_tx,
            )
            .await;
        });

        stop_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("client connection attempt should stop")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_stop_cancels_after_connection_is_ready() {
        use futures::SinkExt;
        use mew_protocol::{ServerMessage, PROTOCOL_VERSION};
        use tokio_tungstenite::{accept_async, tungstenite::Message};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (server_stop_tx, server_stop_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!("expected daemon handshake ping");
            };
            assert!(matches!(
                mew_protocol::decode_json::<ClientMessage>(text.as_ref()).unwrap(),
                ClientMessage::Ping
            ));
            socket
                .send(Message::Text(
                    mew_protocol::encode_json(&ServerMessage::Pong {
                        version: "test".into(),
                        protocol_version: PROTOCOL_VERSION,
                    })
                    .unwrap(),
                ))
                .await
                .unwrap();
            server_stop_rx.await.unwrap();
        });
        let (_command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded();
        let task = tokio::spawn(async move {
            DesktopShell::run_client_with_transport(
                Box::new(LocalWebSocketTransport::new(url)),
                &mut command_rx,
                stop_rx,
                event_tx,
            )
            .await;
        });

        assert!(matches!(
            event_rx.next().await,
            Some(DesktopClientEvent::Connected)
        ));
        assert!(!task.is_finished());
        stop_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("connected client should stop")
            .unwrap();
        server_stop_tx.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_stop_cancels_a_blocked_websocket_command_write() {
        use futures::SinkExt;
        use mew_protocol::{ServerMessage, PROTOCOL_VERSION};
        use tokio::io::AsyncReadExt;
        use tokio_tungstenite::{accept_async, tungstenite::Message};

        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(1024).unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = socket.listen(1).unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = oneshot::channel();
        let (writing_tx, writing_rx) = oneshot::channel();
        let (server_stop_tx, server_stop_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!("expected daemon handshake ping");
            };
            assert!(matches!(
                mew_protocol::decode_json::<ClientMessage>(text.as_ref()).unwrap(),
                ClientMessage::Ping
            ));
            socket
                .send(Message::Text(
                    mew_protocol::encode_json(&ServerMessage::Pong {
                        version: "test".into(),
                        protocol_version: PROTOCOL_VERSION,
                    })
                    .unwrap(),
                ))
                .await
                .unwrap();
            for expected in [
                ClientMessage::Ping,
                ClientMessage::ListSessions,
                ClientMessage::ListProjects,
                ClientMessage::ListModels,
            ] {
                let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                    panic!("expected client startup command");
                };
                let command = mew_protocol::decode_json::<ClientMessage>(text.as_ref()).unwrap();
                assert_eq!(
                    std::mem::discriminant(&command),
                    std::mem::discriminant(&expected)
                );
            }
            ready_tx.send(()).unwrap();

            // Observe the prompt frame beginning, then leave its body unread so the
            // real WebSocket sender reaches socket backpressure.
            socket.get_mut().read_exact(&mut [0; 14]).await.unwrap();
            writing_tx.send(()).unwrap();
            server_stop_rx.await.unwrap();
        });
        let (command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (event_tx, _event_rx) = mpsc::unbounded();
        let mut client = tokio::spawn(async move {
            DesktopShell::run_client_with_transport(
                Box::new(LocalWebSocketTransport::new(url)),
                &mut command_rx,
                stop_rx,
                event_tx,
            )
            .await;
        });
        ready_rx.await.unwrap();
        command_tx
            .send(ClientMessage::Prompt {
                text: "x".repeat(32 * 1024 * 1024),
                attachments: Vec::new(),
            })
            .unwrap();
        writing_rx.await.unwrap();
        assert!(!client.is_finished());

        stop_tx.send(()).unwrap();
        let stopped = tokio::time::timeout(Duration::from_secs(1), &mut client).await;
        server_stop_tx.send(()).unwrap();
        server.await.unwrap();
        if stopped.is_err() {
            client.await.unwrap();
        }
        stopped
            .expect("client should stop while its command write is blocked")
            .unwrap();
    }
}
