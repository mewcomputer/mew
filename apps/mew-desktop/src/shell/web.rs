use super::*;
use mew_cef_host::embed::{AppRequestHandler, AppResponse, CefPageController};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as sync_channel, Mutex};

mod commands;
use commands::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    daemon_url: String,
    #[serde(flatten)]
    preferences: ThemePreferences,
    themes: Vec<HostTheme>,
}

#[derive(Serialize)]
struct HostTheme {
    id: String,
    mode: &'static str,
    css: String,
    #[serde(skip)]
    background: gpui::Rgba,
    #[serde(skip)]
    foreground: gpui::Rgba,
    #[serde(skip)]
    chrome: gpui::Rgba,
}

impl Bootstrap {
    fn load(daemon_url: String) -> Self {
        let state = mew_config::load_state().unwrap_or_default();
        let themes = Theme::list_available()
            .into_iter()
            .map(|id| {
                let theme = Theme::load(&id);
                HostTheme {
                    id,
                    mode: if theme.mode == mew_tui::theme::ThemeMode::Light {
                        "light"
                    } else {
                        "dark"
                    },
                    css: theme.css_variables(),
                    background: theme_rgb(&theme, "background"),
                    foreground: theme_rgb(&theme, "foreground"),
                    chrome: theme_rgb(&theme, "status_bar.background"),
                }
            })
            .collect();
        Self {
            daemon_url,
            preferences: ThemePreferences {
                theme_mode: match state.desktop_theme_mode.as_str() {
                    "light" => ThemeMode::Light,
                    "dark" => ThemeMode::Dark,
                    _ => ThemeMode::System,
                },
                light_theme: if state.desktop_light_theme.is_empty() {
                    "codex-light".into()
                } else {
                    state.desktop_light_theme
                },
                dark_theme: if state.desktop_dark_theme.is_empty() {
                    "codex-dark".into()
                } else {
                    state.desktop_dark_theme
                },
            },
            themes,
        }
    }

    fn palette(&self, appearance: WindowAppearance) -> Option<&HostTheme> {
        let light = match self.preferences.theme_mode {
            ThemeMode::Light => true,
            ThemeMode::Dark => false,
            ThemeMode::System => matches!(
                appearance,
                WindowAppearance::Light | WindowAppearance::VibrantLight
            ),
        };
        let id = if light {
            &self.preferences.light_theme
        } else {
            &self.preferences.dark_theme
        };
        self.themes
            .iter()
            .find(|theme| &theme.id == id)
            .or_else(|| self.themes.iter().find(|theme| theme.id == "dark"))
    }
}

struct AssetServer {
    url: String,
    handle: tokio::runtime::Handle,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AssetServer {
    fn start() -> Result<Self> {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}/", listener.local_addr()?);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let handle = runtime.handle().clone();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new().name("mew-ui-assets".into()).spawn(move || {
            runtime.block_on(async move {
                let result = async {
                    let listener = tokio::net::TcpListener::from_std(listener)?;
                    mew_web_bridge::serve_assets(listener).await
                };
                tokio::select! {
                    result = result => if let Err(error) = result { tracing::error!(%error, "desktop UI asset server stopped"); },
                    _ = stopped => {}
                }
            });
        })?;
        Ok(Self {
            url,
            handle,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                tracing::warn!("desktop UI asset thread panicked");
            }
        }
    }
}

impl Drop for AssetServer {
    fn drop(&mut self) {
        self.stop();
    }
}

struct NativeViews {
    parent_view: usize,
    app: CefPageController,
    portal: BrowserPortal,
    viewport: BrowserRect,
    page_rects: BTreeMap<BrowserOwner, (ViewportRect, bool)>,
    requested_urls: BTreeMap<BrowserOwner, String>,
}

impl NativeViews {
    fn layout(&mut self, bounds: Bounds<Pixels>, window_size: gpui::Size<Pixels>) {
        self.viewport = browser_native_rect(bounds, window_size, true);
        self.app
            .set_rect_on_main_thread(mew_cef_host::embed::BrowserRect {
                x: self.viewport.x,
                y: self.viewport.y,
                width: self.viewport.width,
                height: self.viewport.height,
                visible: self.viewport.visible,
            });
        for (owner, (rect, visible)) in &self.page_rects {
            let _ = self
                .portal
                .set_rect(owner, rect.in_viewport(self.viewport, *visible));
        }
    }

    fn dispatch(&self, name: &str, detail: serde_json::Value) {
        let script = format!(
            "window.dispatchEvent(new CustomEvent({},{{detail:{detail}}}));",
            serde_json::to_string(name).unwrap()
        );
        self.app.execute_javascript_on_main_thread(&script);
    }

    fn browser_event(&mut self, event: BrowserEvent) {
        let (page, kind) = match event {
            BrowserEvent::Ready { page } => (page, "ready"),
            BrowserEvent::Closed { page } => (page, "closed"),
            BrowserEvent::Error { page } => (page, "error"),
            BrowserEvent::AddressChanged { page } | BrowserEvent::TitleChanged { page } => {
                (page, "changed")
            }
        };
        let state = self.portal.page_state(&page.owner);
        let url = state
            .as_ref()
            .map(|state| state.url.as_str())
            .unwrap_or_default();
        let url = if url.is_empty() || url == "about:blank" {
            self.requested_urls
                .get(&page.owner)
                .map(String::as_str)
                .unwrap_or(url)
        } else {
            if let Some(requested) = self.requested_urls.get_mut(&page.owner) {
                if requested != url {
                    requested.clear();
                    requested.push_str(url);
                }
            }
            url
        };
        let ready = state.as_ref().and_then(|state| state.ready.as_ref());
        self.dispatch("mew:browser-event", serde_json::json!({
            "kind":kind,"sessionId":page.owner.session_id,"tabId":page.owner.tab_id,"pageInstance":page.page_instance,
            "url":url,
            "title":state.as_ref().map(|state| state.title.as_str()).unwrap_or_default(),
            "cdpEndpoint":ready.map(|ready| ready.cdp_endpoint.as_str()),"targetId":ready.map(|ready| ready.target_id.as_str()),
            "message":state.as_ref().and_then(|state| state.error.as_deref()),
        }));
    }
}

struct WebShell {
    assets: Option<AssetServer>,
    supervisor: Option<DesktopSupervisor>,
    bootstrap: Arc<Mutex<Bootstrap>>,
    native: Option<Rc<RefCell<NativeViews>>>,
    requests: sync_channel::Receiver<(HostCommand, AppResponse)>,
    sender: sync_channel::SyncSender<(HostCommand, AppResponse)>,
    app_error: Arc<Mutex<Option<String>>>,
    theme_changed: Arc<AtomicBool>,
    window_bounds: Bounds<Pixels>,
    error: Option<String>,
    quitting: bool,
    _subscriptions: Vec<Subscription>,
}

impl WebShell {
    fn new(
        startup: Result<(AssetServer, DesktopSupervisor, Bootstrap)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, requests) = sync_channel::sync_channel(64);
        let (assets, supervisor, bootstrap, error) = match startup {
            Ok((assets, supervisor, bootstrap)) => {
                (Some(assets), Some(supervisor), bootstrap, None)
            }
            Err(error) => (
                None,
                None,
                Bootstrap::load(String::new()),
                Some(error.to_string()),
            ),
        };
        let _subscriptions = vec![
            cx.on_app_quit(|shell, _| {
                shell.prepare_for_quit();
                async {}
            }),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            cx.observe_window_bounds(window, |shell, window, _| {
                shell.window_bounds = window.bounds()
            }),
        ];
        let shell = Self {
            assets,
            supervisor,
            bootstrap: Arc::new(Mutex::new(bootstrap)),
            native: None,
            requests,
            sender,
            app_error: Arc::new(Mutex::new(None)),
            theme_changed: Arc::new(AtomicBool::new(false)),
            window_bounds: window.bounds(),
            error,
            quitting: false,
            _subscriptions,
        };
        if let Some(parent) = parent_view_handle(window).filter(|_| shell.error.is_none()) {
            let weak = cx.entity().downgrade();
            cx.defer(move |cx| {
                if let Some(shell) = weak.upgrade() {
                    shell.update(cx, |shell, cx| shell.initialize(parent, cx));
                }
            });
        }
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let Some(shell) = weak.upgrade() else { break };
            if shell.read_with(cx, |shell, _| shell.native.is_some() && !shell.quitting) {
                // CEF can validate native menus, which re-enters the GPUI app.
                mew_cef_host::embed::CefEmbedController::do_message_loop_work();
            }
            if !shell.update(cx, |shell, cx| shell.pump(cx)) {
                break;
            }
        })
        .detach();
        shell
    }

    fn initialize(&mut self, parent_view: usize, cx: &mut Context<Self>) {
        let result = (|| -> Result<NativeViews> {
            let portal = BrowserPortal::initialize()?
                .ok_or_else(|| anyhow::anyhow!("packaged CEF runtime is unavailable"))?;
            let assets = self
                .assets
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("UI assets are unavailable"))?;
            let sender = self.sender.clone();
            let bootstrap = self.bootstrap.clone();
            let runtime = assets.handle.clone();
            let theme_changed = self.theme_changed.clone();
            let handler: AppRequestHandler = Arc::new(move |request, reply| {
                let command = match HostCommand::parse(request) {
                    Ok(command) => command,
                    Err(error) => {
                        reply.failure(400, &error);
                        return;
                    }
                };
                match command {
                    HostCommand::Bootstrap {} => match bootstrap
                        .lock()
                        .ok()
                        .and_then(|bootstrap| serde_json::to_string(&*bootstrap).ok())
                    {
                        Some(value) => reply.success(&value),
                        None => reply.failure(500, "could not load desktop startup settings"),
                    },
                    HostCommand::SetTheme { preferences } => {
                        let bootstrap = bootstrap.clone();
                        let theme_changed = theme_changed.clone();
                        runtime.spawn_blocking(move || {
                            let result = (|| -> Result<()> {
                                let mut bootstrap = bootstrap
                                    .lock()
                                    .map_err(|_| anyhow::anyhow!("theme state is unavailable"))?;
                                for id in [&preferences.light_theme, &preferences.dark_theme] {
                                    anyhow::ensure!(
                                        bootstrap.themes.iter().any(|theme| &theme.id == id),
                                        "unknown theme: {id}"
                                    );
                                }
                                let mut state = mew_config::load_state()?;
                                preferences.apply_to(&mut state);
                                mew_config::save_state(&state)?;
                                bootstrap.preferences = preferences;
                                theme_changed.store(true, Ordering::Release);
                                Ok(())
                            })();
                            match result {
                                Ok(()) => reply.success("{}"),
                                Err(error) => reply.failure(500, &error.to_string()),
                            }
                        });
                    }
                    command => {
                        if let Err(
                            sync_channel::TrySendError::Full((_, reply))
                            | sync_channel::TrySendError::Disconnected((_, reply)),
                        ) = sender.try_send((command, reply))
                        {
                            reply.failure(503, "desktop host is busy; retry the operation");
                        }
                    }
                }
            });
            let error_state = self.app_error.clone();
            let app = portal.create_app_page(
                parent_view,
                &assets.url,
                Arc::new(move |event| {
                    if let mew_cef_host::embed::BrowserEvent::Error { message } = event {
                        if let Ok(mut error) = error_state.lock() {
                            *error = Some(message);
                        }
                    }
                }),
                handler,
            )?;
            Ok(NativeViews {
                parent_view,
                app,
                portal,
                viewport: BrowserRect {
                    x: 0.,
                    y: 32.,
                    width: 0.,
                    height: 0.,
                    visible: false,
                },
                page_rects: BTreeMap::new(),
                requested_urls: BTreeMap::new(),
            })
        })();
        match result {
            Ok(native) => self.native = Some(Rc::new(RefCell::new(native))),
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn pump(&mut self, cx: &mut Context<Self>) -> bool {
        if self.quitting {
            return false;
        }
        if self.theme_changed.swap(false, Ordering::AcqRel) {
            cx.notify();
        }
        if let Some(native) = self.native.clone() {
            for (command, reply) in self.requests.try_iter() {
                let result = self.apply_browser_command(&mut native.borrow_mut(), command);
                match result {
                    Ok(()) => reply.success("{}"),
                    Err(error) => reply.failure(400, &error.to_string()),
                }
            }
            let events = native.borrow_mut().portal.drain_events();
            for event in events {
                native.borrow_mut().browser_event(event);
            }
        }
        if let Some(error) = self
            .app_error
            .lock()
            .ok()
            .and_then(|mut error| error.take())
        {
            if let Some(native) = &self.native {
                let native = native.borrow();
                native.portal.hide_all();
                native.app.set_visible_on_main_thread(false);
            }
            self.error = Some(error);
            cx.notify();
        }
        true
    }

    fn apply_browser_command(&self, native: &mut NativeViews, command: HostCommand) -> Result<()> {
        let profile = self
            .bootstrap
            .lock()
            .map_err(|_| anyhow::anyhow!("desktop connection is unavailable"))?
            .daemon_url
            .clone();
        match command {
            HostCommand::BrowserUpdate {
                session_id,
                tab_id,
                url,
                rect,
                visible,
            } => {
                let owner = BrowserOwner {
                    connection_profile: profile,
                    session_id,
                    tab_id,
                };
                if !visible && !native.portal.contains_page(&owner) {
                    return Ok(());
                }
                if !native.portal.contains_page(&owner) {
                    native
                        .portal
                        .create_page(native.parent_view, BrowserPage::new(owner.clone()))?;
                }
                if native.requested_urls.get(&owner) != Some(&url) {
                    native.portal.navigate(&owner, &url)?;
                    native.requested_urls.insert(owner.clone(), url);
                }
                native.page_rects.insert(owner.clone(), (rect, visible));
                native
                    .portal
                    .set_rect(&owner, rect.in_viewport(native.viewport, visible))?;
            }
            HostCommand::BrowserClose { session_id, tab_id } => {
                let owner = BrowserOwner {
                    connection_profile: profile,
                    session_id,
                    tab_id,
                };
                native.page_rects.remove(&owner);
                native.requested_urls.remove(&owner);
                if let Some(page) = native.portal.page_state(&owner).map(|state| state.page) {
                    native.portal.close(&page);
                    native.dispatch("mew:browser-event", serde_json::json!({"kind":"closed","sessionId":page.owner.session_id,"tabId":page.owner.tab_id,"pageInstance":page.page_instance,"url":"","title":""}));
                }
            }
            _ => anyhow::bail!("unsupported browser operation"),
        }
        Ok(())
    }

    fn menu(&self, action: &str) {
        if let Some(native) = &self.native {
            let native = native.borrow();
            native.app.focus_on_main_thread();
            native.dispatch("mew:menu-action", serde_json::json!(action));
        }
    }

    fn prepare_for_quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        if let Some(native) = self.native.as_ref() {
            native.borrow_mut().portal.prepare_for_process_exit();
        }
        if let Some(assets) = self.assets.as_mut() {
            // Finish pending preference writes before saving window state.
            assets.stop();
        }
        let bounds = self.window_bounds;
        if let Ok(mut state) = mew_config::load_state() {
            state.desktop_window = Some(mew_config::DesktopWindowState {
                x: bounds.origin.x.into(),
                y: bounds.origin.y.into(),
                width: bounds.size.width.into(),
                height: bounds.size.height.into(),
            });
            if let Err(error) = mew_config::save_state(&state) {
                tracing::warn!(%error, "could not save desktop window bounds");
            }
        }
        if let Some(supervisor) = self.supervisor.as_mut() {
            if let Err(error) = supervisor.shutdown() {
                tracing::warn!(%error, "could not stop desktop-owned daemon");
            }
        }
    }
}

impl Drop for WebShell {
    fn drop(&mut self) {
        self.prepare_for_quit();
    }
}

impl Render for WebShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let native = self.native.clone();
        let (background, foreground, chrome) = self
            .bootstrap
            .lock()
            .ok()
            .and_then(|bootstrap| {
                bootstrap
                    .palette(window.appearance())
                    .map(|theme| (theme.background, theme.foreground, theme.chrome))
            })
            .unwrap_or_default();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .on_action(cx.listener(|shell, _: &NewConversation, _, _| shell.menu("new-chat")))
            .on_action(cx.listener(|shell, _: &NewWorkTab, _, _| shell.menu("new-tab")))
            .on_action(cx.listener(|shell, _: &CloseWorkTab, _, _| shell.menu("close-tab")))
            .on_action(cx.listener(|shell, _: &ToggleTerminal, _, _| shell.menu("terminal")))
            .on_action(cx.listener(|shell, _: &ToggleSidebar, _, _| shell.menu("toggle-sidebar")))
            .on_action(
                cx.listener(|shell, _: &ToggleWorkbench, _, _| shell.menu("toggle-workbench")),
            )
            .on_action(cx.listener(|shell, _: &FocusSidebarSearch, _, _| shell.menu("search")))
            .child(
                div()
                    .h(px(32.))
                    .flex_shrink_0()
                    .bg(chrome)
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                        window.start_window_move()
                    }),
            )
            .child(if let Some(error) = &self.error {
                div()
                    .p(px(24.))
                    .text_color(foreground)
                    .child(format!("could not open mew's web interface: {error}"))
                    .into_any_element()
            } else {
                canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        if let Some(native) = &native {
                            native.borrow_mut().layout(bounds, window.bounds().size);
                        }
                    },
                )
                .flex_1()
                .min_h_0()
                .w_full()
                .into_any_element()
            })
    }
}

pub(crate) fn run() {
    platform::configure_native_cef_environment();
    let startup = (|| -> Result<_> {
        anyhow::ensure!(platform::remote_connection_profile_from_env().is_none(), "the web interface currently requires a local connection; use the native app for remote profiles");
        let mut supervisor = DesktopSupervisor::new(platform::supervisor_config()?);
        let endpoint = supervisor.connect_or_launch()?;
        anyhow::ensure!(
            browser_profile_is_local(&DesktopConnectionProfile::LocalWebSocket {
                url: endpoint.websocket_url.clone(),
            }),
            "the web interface currently requires a loopback daemon; use the native app for remote connections"
        );
        let bootstrap = Bootstrap::load(endpoint.websocket_url);
        Ok((AssetServer::start()?, supervisor, bootstrap))
    })();
    application()
        .with_assets(IconAssets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            platform::set_app_menus(cx);
            cx.bind_keys([gpui::KeyBinding::new("cmd-w", CloseWorkTab, None)]);
            let bounds = platform::persisted_window_bounds()
                .unwrap_or_else(|| Bounds::centered(None, gpui::size(px(1240.), px(800.)), cx));
            if let Err(error) = cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: None,
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(12.), px(10.))),
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(gpui::size(px(720.), px(480.))),
                    ..Default::default()
                },
                move |window, cx| cx.new(|cx| WebShell::new(startup, window, cx)),
            ) {
                tracing::error!(%error, "could not open desktop web interface");
                cx.quit();
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_commands_retain_their_chat_and_tab_identity() {
        let command = HostCommand::parse(r#"{"type":"browser_update","sessionId":"chat-a","tabId":"tab-a","url":"https://example.com/","rect":{"x":400,"y":80,"width":500,"height":400},"visible":true}"#).unwrap();
        assert!(
            matches!(command, HostCommand::BrowserUpdate { session_id, tab_id, visible: true, .. } if session_id == "chat-a" && tab_id == "tab-a")
        );
    }

    #[test]
    fn malformed_or_unowned_browser_commands_are_rejected() {
        for command in [
            r#"{"type":"browser_close","sessionId":"","tabId":"tab"}"#,
            r#"{"type":"browser_close","sessionId":"chat","tabId":""}"#,
            r#"{"type":"bootstrap","unexpected":true}"#,
            r#"{"type":"browser_update","sessionId":"chat","tabId":"tab","url":"https://example.com","rect":{"x":0,"y":0,"width":-1,"height":300},"visible":true}"#,
        ] {
            assert!(HostCommand::parse(command).is_err(), "accepted {command}");
        }
        assert!(HostCommand::parse(&" ".repeat(16_385)).is_err());
    }

    #[test]
    fn browser_bounds_are_clipped_to_the_app_viewport() {
        let viewport = BrowserRect {
            x: 0.,
            y: 32.,
            width: 1000.,
            height: 700.,
            visible: true,
        };
        let rect = ViewportRect {
            x: 850.,
            y: 600.,
            width: 400.,
            height: 300.,
        }
        .in_viewport(viewport, true);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (850., 32., 150., 100.)
        );
        assert!(rect.visible);
        let outside = ViewportRect {
            x: -500.,
            y: 0.,
            width: 200.,
            height: 100.,
        }
        .in_viewport(viewport, true);
        assert!(!outside.visible);
    }

    #[test]
    fn hiding_a_page_accepts_an_empty_viewport() {
        let command = HostCommand::parse(r#"{"type":"browser_update","sessionId":"chat","tabId":"tab","url":"https://example.com","rect":{"x":0,"y":0,"width":0,"height":0},"visible":false}"#).unwrap();
        assert!(matches!(
            command,
            HostCommand::BrowserUpdate { visible: false, .. }
        ));
    }

    #[test]
    fn theme_preferences_preserve_other_native_settings() {
        let mut state = mew_config::State {
            last_model: "retained-model".into(),
            ..Default::default()
        };
        let preferences = ThemePreferences {
            theme_mode: ThemeMode::System,
            light_theme: "light".into(),
            dark_theme: "nord".into(),
        };
        preferences.apply_to(&mut state);
        assert_eq!(state.desktop_theme_mode, "system");
        assert_eq!(state.desktop_light_theme, "light");
        assert_eq!(state.desktop_dark_theme, "nord");
        assert_eq!(state.last_model, "retained-model");
    }
}
