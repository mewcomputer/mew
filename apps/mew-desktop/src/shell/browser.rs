use super::*;

pub(super) fn replace_browser_event_value(current: &mut String, next: String) -> bool {
    if *current == next {
        return false;
    }
    *current = next;
    true
}

impl DesktopShell {
    pub(super) fn open_browser_panel(&mut self, cx: &mut Context<Self>) {
        self.auxiliary_view = AuxiliaryView::Browser;
        self.capture_session_view_state();
        self.browser_panel_open = true;
        self.browser_url_focus_requested = true;
        self.sync_browser_portal_to_view_state();
        self.capture_session_view_state();
        cx.notify();
    }

    pub(super) fn close_browser_panel(&mut self, cx: &mut Context<Self>) {
        self.browser_panel_open = false;
        self.browser_native_rect = None;
        self.browser_url_focus_requested = false;
        if let Some(portal) = self.browser_portal.as_ref() {
            portal.hide_all();
        }
        self.capture_session_view_state();
        cx.notify();
    }

    pub(super) fn reset_browser_for_unattached_tab(&mut self) {
        self.browser_panel_open = false;
        self.browser_native_rect = None;
        self.browser_url_focus_requested = false;
        self.browser_url.clear();
        self.browser_title.clear();
        self.browser_error = None;
        if let Some(portal) = self.browser_portal.as_ref() {
            portal.hide_all();
        }
    }

    pub(super) fn reset_browser_connection(&mut self) {
        self.browser_confirmed_sessions.clear();
        self.browser_registration_attempts.clear();
        self.browser_initialization_pending = None;
        self.browser_native_rect = None;
        if let Some(portal) = &self.browser_portal {
            portal.hide_all();
        }
    }

    pub(super) fn current_browser_owner(&self) -> Option<BrowserOwner> {
        let profile = self.connection_profile.as_ref()?;
        if !browser_profile_is_local(profile)
            || self.model.connection != crate::model::ShellConnection::Connected
            || !self.model.session_is_ready()
        {
            return None;
        }
        let session_id = self.model.attached_session.as_ref()?;
        self.browser_confirmed_sessions
            .contains(session_id)
            .then(|| BrowserOwner {
                connection_profile: profile.endpoint_label(),
                session_id: session_id.clone(),
            })
    }

    pub(super) fn browser_owner_is_confirmed(&self, owner: &BrowserOwner) -> bool {
        self.model.connection == crate::model::ShellConnection::Connected
            && self.connection_profile.as_ref().is_some_and(|profile| {
                browser_profile_is_local(profile)
                    && profile.endpoint_label() == owner.connection_profile
            })
            && self.browser_confirmed_sessions.contains(&owner.session_id)
    }

    pub(super) fn retry_browser(&mut self, cx: &mut Context<Self>) {
        if let Some(owner) = self.current_browser_owner() {
            if self
                .browser_initialization_pending
                .as_ref()
                .is_some_and(|page| page.owner == owner)
            {
                self.browser_initialization_pending = None;
            }
            if let Some(page) = self
                .browser_portal
                .as_ref()
                .and_then(|portal| portal.page_state(&owner))
                .map(|state| state.page)
            {
                self.send_command(ClientMessage::UnregisterBrowserPage {
                    session_id: owner.session_id.clone(),
                    page_instance: page.page_instance.clone(),
                });
                if let Some(portal) = &mut self.browser_portal {
                    portal.close(&page);
                }
            }
            self.browser_registration_attempts.remove(&owner);
        }
        self.browser_native_rect = None;
        self.browser_url = DEFAULT_BROWSER_URL.into();
        self.browser_title.clear();
        self.browser_error = None;
        cx.notify();
    }

    pub(super) fn navigate_browser_to(&mut self, cx: &mut Context<Self>) {
        let result = normalize_browser_url(&self.browser_url)
            .ok_or_else(|| "enter a valid http(s) URL".to_owned())
            .and_then(|url| {
                let owner = self
                    .current_browser_owner()
                    .ok_or_else(|| "attach a local session to use the browser".to_owned())?;
                let portal = self
                    .browser_portal
                    .as_ref()
                    .ok_or_else(|| "native browser is starting".to_owned())?;
                portal
                    .navigate(&owner, &url)
                    .map_err(|error| error.to_string())?;
                self.browser_url = url;
                Ok(())
            });
        self.browser_error = result.err();
        self.capture_session_view_state();
        cx.notify();
    }

    pub(super) fn start_browser_pump(&mut self, cx: &mut Context<Self>) {
        if self.browser_pump.is_none() {
            let shell = cx.entity().downgrade();
            self.browser_pump = Some(cx.new(|cx| BrowserPumpView::new(shell, cx)));
        }
    }

    pub(super) fn ensure_browser_portal(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(owner) = self.current_browser_owner() else {
            if self.browser_panel_open
                && self
                    .connection_profile
                    .as_ref()
                    .is_some_and(|profile| !browser_profile_is_local(profile))
            {
                self.browser_error = Some("native browser tools require a local daemon connection; remote profiles are unsupported".into());
            }
            return;
        };
        if !browser_initialization_is_needed(
            true,
            self.browser_portal
                .as_ref()
                .is_some_and(|portal| portal.contains_page(&owner)),
            self.browser_initialization_pending.is_some(),
            self.browser_error.is_some(),
        ) {
            return;
        }
        let Some(parent_view) = parent_view_handle(window) else {
            self.browser_error = Some("native browser is unavailable on this platform".into());
            return;
        };
        let page = BrowserPage::new(owner);
        self.browser_initialization_pending = Some(page.clone());
        let shell = cx.entity().downgrade();
        cx.defer(move |cx| {
            if let Some(shell) = shell.upgrade() {
                shell.update(cx, |shell, cx| {
                    shell.initialize_browser_portal(parent_view, page, cx)
                });
            }
        });
    }

    fn initialize_browser_portal(
        &mut self,
        parent_view: usize,
        page: BrowserPage,
        cx: &mut Context<Self>,
    ) {
        if !browser_creation_is_current(
            self.browser_initialization_pending.as_ref(),
            &page,
            self.browser_owner_is_confirmed(&page.owner),
            self.quitting,
        ) {
            return;
        }
        self.browser_initialization_pending = None;
        if self.browser_portal.is_none() {
            match BrowserPortal::initialize() {
                Ok(Some(portal)) => self.browser_portal = Some(portal),
                Ok(None) => {
                    self.browser_error = Some(
                        "native browser is unavailable; install the packaged browser runtime"
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                Err(error) => {
                    self.browser_error = Some(format!("could not start native browser: {error:#}"));
                    cx.notify();
                    return;
                }
            }
        }
        if let Err(error) = self
            .browser_portal
            .as_mut()
            .unwrap()
            .create_page(parent_view, page)
        {
            self.browser_error = Some(error.to_string());
        }
        self.start_browser_pump(cx);
        self.sync_browser_portal_to_view_state();
        self.sync_browser_visibility();
        cx.notify();
    }

    pub(super) fn register_ready_browser_page(&mut self, page: &BrowserPage) {
        if !self.browser_owner_is_confirmed(&page.owner)
            || self.browser_registration_attempts.get(&page.owner) == Some(&page.page_instance)
        {
            return;
        }
        let Some(state) = self
            .browser_portal
            .as_ref()
            .and_then(|portal| portal.page_state(&page.owner))
            .filter(|state| state.page == *page && !state.closed)
        else {
            return;
        };
        let Some(ready) = state.ready else {
            return;
        };
        self.browser_registration_attempts
            .insert(page.owner.clone(), page.page_instance.clone());
        self.send_command(ClientMessage::RegisterBrowserPage {
            session_id: page.owner.session_id.clone(),
            page_instance: page.page_instance.clone(),
            cdp_endpoint: ready.cdp_endpoint,
            target_id: ready.target_id,
        });
    }

    pub(super) fn apply_browser_events(&mut self, _cx: &mut Context<Self>) -> bool {
        let events = self
            .browser_portal
            .as_mut()
            .map(BrowserPortal::drain_events)
            .unwrap_or_default();
        let mut changed = false;
        for event in events {
            let page = match event {
                BrowserEvent::Ready { page, .. } => {
                    self.register_ready_browser_page(&page);
                    page
                }
                BrowserEvent::Closed { page } => {
                    if self.browser_owner_is_confirmed(&page.owner) {
                        self.send_command(ClientMessage::UnregisterBrowserPage {
                            session_id: page.owner.session_id.clone(),
                            page_instance: page.page_instance.clone(),
                        });
                    }
                    self.browser_registration_attempts.remove(&page.owner);
                    page
                }
                BrowserEvent::AddressChanged { page, .. }
                | BrowserEvent::TitleChanged { page, .. }
                | BrowserEvent::Error { page, .. } => page,
            };
            if self.current_browser_owner().as_ref() == Some(&page.owner) {
                self.sync_browser_portal_to_view_state();
                changed = true;
            }
        }
        changed
    }

    pub(super) fn sync_browser_visibility(&self) {
        let Some(portal) = &self.browser_portal else {
            return;
        };
        if self.browser_panel_open && !self.layout.workbench_collapsed {
            if let Some(owner) = self.current_browser_owner() {
                if portal.page_is_live(&owner) {
                    let _ = portal.set_visible(&owner, true);
                    return;
                }
            }
        }
        portal.hide_all();
    }

    pub(super) fn update_browser_bounds(&mut self, bounds: Bounds<Pixels>, window: &Window) {
        let (Some(portal), Some(owner)) =
            (self.browser_portal.as_ref(), self.current_browser_owner())
        else {
            return;
        };
        if !portal.page_is_live(&owner) {
            return;
        }
        let rect = browser_native_rect(
            bounds,
            window.bounds().size,
            self.browser_panel_open && !self.layout.workbench_collapsed,
        );
        if self.browser_native_rect == Some(rect) {
            return;
        }
        if let Err(error) = portal.set_rect(&owner, rect) {
            self.browser_error = Some(error.to_string());
        } else {
            self.browser_native_rect = Some(rect);
        }
    }

    pub(super) fn sync_browser_portal_to_view_state(&mut self) {
        let state = self
            .current_browser_owner()
            .and_then(|owner| self.browser_portal.as_ref()?.page_state(&owner));
        if let Some(state) = state {
            replace_browser_event_value(&mut self.browser_url, state.url);
            replace_browser_event_value(&mut self.browser_title, state.title);
            self.browser_error = state.error;
        } else {
            self.browser_url = DEFAULT_BROWSER_URL.into();
            self.browser_title.clear();
        }
        self.browser_native_rect = None;
        self.sync_browser_visibility();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn unavailable_browser_runtime_notifies_the_shell(cx: &mut gpui::TestAppContext) {
        const CHILD: &str = "MEW_TEST_BROWSER_UNAVAILABLE";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "shell::browser::tests::unavailable_browser_runtime_notifies_the_shell",
                ])
                .env(CHILD, "1")
                .env("MEW_CEF_FRAMEWORK_PATH", "/dev/null/mew-missing-cef")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        cx.skip_drawing();
        let window =
            cx.add_window(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let shell = window.root(cx).unwrap();
        let profile = DesktopConnectionProfile::LocalWebSocket {
            url: "ws://127.0.0.1:25566".into(),
        };
        let page = BrowserPage::new(BrowserOwner {
            connection_profile: profile.endpoint_label(),
            session_id: "browser-test".into(),
        });
        shell.update(cx, |shell, _| {
            shell.model.connection = crate::model::ShellConnection::Connected;
            shell.connection_profile = Some(profile);
            shell
                .browser_confirmed_sessions
                .insert(page.owner.session_id.clone());
            shell.browser_initialization_pending = Some(page.clone());
        });
        let notifications = Rc::new(std::cell::Cell::new(0));
        let _subscription = cx.update(|cx| {
            let notifications = notifications.clone();
            cx.observe(&shell, move |_, _| {
                notifications.set(notifications.get() + 1)
            })
        });
        cx.update(|cx| {
            cx.defer(move |cx| {
                shell.update(cx, |shell, cx| {
                    shell.initialize_browser_portal(0, page, cx);
                    assert!(shell.browser_initialization_pending.is_none());
                    assert!(shell
                        .browser_error
                        .as_ref()
                        .unwrap()
                        .contains("unavailable"));
                    shell.quitting = true;
                });
            });
        });
        cx.run_until_parked();
        assert_eq!(
            notifications.get(),
            1,
            "deferred failure must repaint its error"
        );
    }
}
