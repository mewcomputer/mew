use super::*;

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
        self.terminal_view.update(cx, |view, _cx| {
            view.set_theme_colors(
                theme_rgb(&theme, "text.body"),
                theme_rgb(&theme, "panel.background"),
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
        let persisted_layout = ShellLayoutState::from_state(&persisted_state);
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
            let (command_tx, stop_tx, thread) = Self::start_client(profile, cx);
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
        let bounds_subscription = cx.observe_window_bounds(window, |shell, window, _cx| {
            shell.persist_window_bounds(window);
        });
        let terminal_view = cx.new(TerminalView::new_remote);
        terminal_view.update(cx, |view, cx| {
            view.set_theme_colors(
                theme_rgb(&theme, "text.body"),
                theme_rgb(&theme, "panel.background"),
            );
            view.set_font_family(terminal_font_family.clone(), cx);
        });
        let composer_focus_handle = cx.focus_handle();
        let browser_url_focus_handle = cx.focus_handle();
        let rename_focus_handle = cx.focus_handle();
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
            collapsed_groups: BTreeSet::new(),
            session_view_states,
            session_menu_session: None,
            hovered_group: None,
            hovered_session: None,
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
            workbench_width: WORKBENCH_EXPANDED_WIDTH,
            auxiliary_view: AuxiliaryView::Changes,
            transcript_list: gpui::ListState::new(0, gpui::ListAlignment::Bottom, px(128.)),
            transcript_rows: Vec::new(),
            transcript_rows_append_only: false,
            markdown_cache: Vec::new(),
            tool_text_lists: RefCell::new(BTreeMap::new()),
            tool_text_cache: RefCell::new(BTreeMap::new()),
            transcript_text_registry: Rc::new(RefCell::new(Vec::new())),
            transcript_selection: None,
            transcript_selection_anchor: None,
            transcript_is_selecting: false,
            transcript_selected_text: None,
            review_diffs: Vec::new(),
            review_lines: Vec::new(),
            review_selected_file: None,
            review_line_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(2048.)),
            review_signature: None,
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
            thinking_picker_bounds: None,
            slash_menu_dismissed: false,
            slash_menu_index: 0,
            mention_menu_dismissed: false,
            mention_menu_index: 0,
            file_tree_entries: BTreeMap::new(),
            file_tree_expanded: BTreeSet::new(),
            file_tree_pending: BTreeSet::new(),
            watching_workspace_session: None,
            prompt_history: PromptHistory::default(),
            plan_feedback_request: None,
            composer_cursor_visible: true,
            composer_blink_epoch: 0,
            streaming_render_scheduled: false,
            expanded_chat_parts: BTreeSet::new(),
            pending_prompt: None,
            pending_attachments: Vec::new(),
            attachments: Vec::new(),
            attachment_error: None,
            pending_model: None,
            awaiting_model_switch: None,
            pending_session_request: false,
            pending_session_target: None,
            model_picker_open: false,
            persona_picker_open: false,
            permission_picker_open: false,
            thinking_picker_open: false,
            terminal_font_picker_open: false,
            terminal_font_family,
            terminal_view,
            terminal_id: None,
            terminal_status: "closed".into(),
            browser_portal: None,
            browser_panel_open: false,
            browser_initialization_pending: false,
            browser_url: String::new(),
            browser_title: String::new(),
            browser_error: None,
            browser_pump_epoch: 0,
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
        }
    }

    pub(super) fn prepare_for_quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
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
        if self.quitting {
            return;
        }

        self.stop_client();
        let mut profile = self.connection_profile.clone();
        let should_restart_owned_daemon = self
            ._supervisor
            .as_ref()
            .and_then(|supervisor| supervisor.endpoint())
            .is_some_and(|endpoint| endpoint.mode == DaemonMode::LocalOwned);

        if profile.is_none() || should_restart_owned_daemon {
            let Some(supervisor) = self._supervisor.as_mut() else {
                self.model.fail("no daemon connection is configured");
                cx.notify();
                return;
            };
            let endpoint = if should_restart_owned_daemon {
                supervisor.restart()
            } else {
                supervisor.connect_or_launch()
            };
            match endpoint {
                Ok(endpoint) => {
                    profile = Some(DesktopConnectionProfile::LocalWebSocket {
                        url: endpoint.websocket_url,
                    });
                }
                Err(error) => {
                    self.model.fail(error.to_string());
                    cx.notify();
                    return;
                }
            }
        }

        let Some(profile) = profile else {
            self.model.fail("no daemon connection is configured");
            cx.notify();
            return;
        };
        let selected_session = self.model.ui.selected_session.clone();
        self.connection_profile = Some(profile.clone());
        self.model = ShellModel::starting(Some(profile.endpoint_label()));
        self.model.ui.selected_session = selected_session.clone();
        self.pending_session_request = selected_session.is_some();
        self.pending_session_target = selected_session;
        let (command_tx, stop_tx, thread) = Self::start_client(profile, cx);
        self.command_tx = Some(command_tx);
        self.client_stop_tx = Some(stop_tx);
        self.client_thread = thread;
        cx.notify();
    }

    fn stop_client(&mut self) {
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
                    let mut needs_shell_render = false;
                    let mut has_streaming_update = false;
                    for event in batch {
                        match event {
                            DesktopClientEvent::Connected => {
                                shell.model.connected();
                                if let Some(session_id) = shell.pending_session_target.clone() {
                                    shell.send_command(ClientMessage::AttachSession {
                                        session_id,
                                        client_kind: mew_protocol::ClientKind::Desktop,
                                    });
                                }
                                needs_shell_render = true;
                            }
                            DesktopClientEvent::Updated { events, state } => {
                                if client_events_are_streaming_only(&events) {
                                    has_streaming_update = true;
                                } else {
                                    needs_shell_render |=
                                        client_events_require_shell_render(&events);
                                }
                                shell.apply_client_update(&events, &state, cx);
                            }
                            DesktopClientEvent::Failed(error) => {
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
                    if let Err(error) = engine.send(command).await {
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

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use futures::future::pending;
    use mew_client_core::{ClientConnection, TransportError};

    struct PendingTransport;

    #[async_trait::async_trait]
    impl ClientTransport for PendingTransport {
        async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
            pending().await
        }
    }

    struct PendingConnection;

    #[async_trait::async_trait]
    impl ClientConnection for PendingConnection {
        async fn send(&mut self, _message: ClientMessage) -> Result<(), TransportError> {
            Ok(())
        }

        async fn receive(&mut self) -> Result<Option<mew_protocol::ServerMessage>, TransportError> {
            pending().await
        }

        async fn close(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    struct ReadyTransport;

    #[async_trait::async_trait]
    impl ClientTransport for ReadyTransport {
        async fn connect(&self) -> Result<Box<dyn ClientConnection>, TransportError> {
            Ok(Box::new(PendingConnection))
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
        let (_command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded();
        let task = tokio::spawn(async move {
            DesktopShell::run_client_with_transport(
                Box::new(ReadyTransport),
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
        stop_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("connected client should stop")
            .unwrap();
    }
}
