use super::*;

pub(super) struct BrowserRegistration {
    pub(super) page_instance: String,
    pub(super) pending_replies: u8,
    pub(super) retiring: bool,
}

pub(super) fn replace_browser_event_value(current: &mut String, next: String) -> bool {
    if *current == next {
        return false;
    }
    *current = next;
    true
}

impl DesktopShell {
    pub(super) fn reset_browser_for_new_conversation(&mut self) {
        self.browser_panel_open = false;
        self.browser_native_rect = None;
        self.browser_url_focus_requested = false;
        self.browser_url.clear();
        self.browser_title.clear();
        self.browser_error = None;
        self.work_tabs = WorkTabs::default();
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
        self.browser_owner_for_session(session_id)
    }

    fn browser_tabs_for_session(&self, session_id: &str) -> Option<&WorkTabs> {
        if self.model.ui.selected_session.as_deref() == Some(session_id) {
            Some(&self.work_tabs)
        } else {
            self.session_view_states
                .get(session_id)
                .map(|state| &state.work_tabs)
        }
    }

    pub(super) fn browser_owner_for_session(&self, session_id: &str) -> Option<BrowserOwner> {
        let profile = self.connection_profile.as_ref()?;
        let tab = self.browser_tabs_for_session(session_id)?.browser_tab()?;
        self.browser_confirmed_sessions
            .contains(session_id)
            .then(|| BrowserOwner {
                connection_profile: profile.endpoint_label(),
                session_id: session_id.to_owned(),
                tab_id: tab.id.clone(),
            })
    }

    pub(super) fn browser_owner_is_confirmed(&self, owner: &BrowserOwner) -> bool {
        self.model.connection == crate::model::ShellConnection::Connected
            && self.connection_profile.as_ref().is_some_and(|profile| {
                browser_profile_is_local(profile)
                    && profile.endpoint_label() == owner.connection_profile
            })
            && self.browser_confirmed_sessions.contains(&owner.session_id)
            && self
                .browser_tabs_for_session(&owner.session_id)
                .is_some_and(|tabs| {
                    tabs.tabs
                        .iter()
                        .any(|tab| tab.id == owner.tab_id && tab.kind.is_browser())
                })
    }

    pub(super) fn save_browser_tab_draft(&mut self) {
        let Some(id) = self.work_tabs.browser_tab_id.as_deref() else {
            return;
        };
        if let Some(tab) = self.work_tabs.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.url.clone_from(&self.browser_url);
            tab.title.clone_from(&self.browser_title);
            tab.error.clone_from(&self.browser_error);
        }
    }

    pub(super) fn retire_browser_tab(&mut self, tab_id: &str) {
        let Some(profile) = self.connection_profile.as_ref() else {
            return;
        };
        let Some(session_id) = self.model.ui.selected_session.as_ref() else {
            return;
        };
        let owner = BrowserOwner {
            connection_profile: profile.endpoint_label(),
            session_id: session_id.clone(),
            tab_id: tab_id.to_owned(),
        };
        if self
            .browser_initialization_pending
            .as_ref()
            .is_some_and(|page| page.owner == owner)
        {
            self.browser_initialization_pending = None;
        }
        self.retire_browser_registration(&owner);
        if let Some(portal) = &mut self.browser_portal {
            if let Some(page) = portal.page_state(&owner).map(|state| state.page) {
                portal.close(&page);
            }
        }
    }

    pub(super) fn sync_browser_target(&mut self) {
        if !self.model.session_is_ready() {
            return;
        }
        let desired = self.current_browser_owner();
        let Some(session_id) = self.model.attached_session.clone() else {
            return;
        };
        let old: Vec<_> = self
            .browser_registration_attempts
            .keys()
            .filter(|owner| owner.session_id == session_id && Some(*owner) != desired.as_ref())
            .cloned()
            .collect();
        for owner in old {
            self.retire_browser_registration(&owner);
        }
        if let Some(page) = desired
            .and_then(|owner| self.browser_portal.as_ref()?.page_state(&owner))
            .map(|state| state.page)
        {
            self.register_ready_browser_page(&page);
        }
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
            self.retire_browser_registration(&owner);
            if let Some(page) = self
                .browser_portal
                .as_ref()
                .and_then(|portal| portal.page_state(&owner))
                .map(|state| state.page)
            {
                if let Some(portal) = &mut self.browser_portal {
                    portal.close(&page);
                }
            }
        }
        self.browser_native_rect = None;
        self.browser_error = None;
        if let Some(id) = self.work_tabs.browser_tab_id.as_deref() {
            if let Some(tab) = self.work_tabs.tabs.iter_mut().find(|tab| tab.id == id) {
                tab.error = None;
                tab.initialization_failed = false;
                tab.native_url = DEFAULT_BROWSER_URL.into();
                tab.native_error = None;
            }
        }
        cx.notify();
    }

    pub(super) fn navigate_browser_to(&mut self, cx: &mut Context<Self>) {
        if self.browser_url.trim().is_empty() {
            return;
        }
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
        if self.browser_error.is_none() {
            self.work_tabs.active_mut().kind = WorkTabKind::Browser;
            self.browser_panel_open = true;
        }
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
            self.work_tabs
                .browser_tab()
                .is_some_and(|tab| tab.initialization_failed),
        ) {
            return;
        }
        let Some(parent_view) = parent_view_handle(window) else {
            self.fail_browser_initialization(
                &owner,
                "native browser is unavailable on this platform".into(),
            );
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
                    self.fail_browser_initialization(
                        &page.owner,
                        "native browser is unavailable; install the packaged browser runtime"
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                Err(error) => {
                    self.fail_browser_initialization(
                        &page.owner,
                        format!("could not start native browser: {error:#}"),
                    );
                    cx.notify();
                    return;
                }
            }
        }
        let owner = page.owner.clone();
        if let Err(error) = self
            .browser_portal
            .as_mut()
            .unwrap()
            .create_page(parent_view, page)
        {
            self.fail_browser_initialization(&owner, error.to_string());
        } else if let Some(url) = self
            .browser_tabs_for_session(&owner.session_id)
            .and_then(|tabs| tabs.tabs.iter().find(|tab| tab.id == owner.tab_id))
            .and_then(|tab| normalize_browser_url(&tab.url))
            .filter(|url| url != DEFAULT_BROWSER_URL)
        {
            if let Some(portal) = &self.browser_portal {
                let _ = portal.navigate(&owner, &url);
            }
        }
        self.start_browser_pump(cx);
        self.sync_browser_portal_to_view_state();
        self.sync_browser_visibility();
        cx.notify();
    }

    fn fail_browser_initialization(&mut self, owner: &BrowserOwner, message: String) {
        let selected = self.model.ui.selected_session.as_deref() == Some(owner.session_id.as_str());
        let tabs = if selected {
            Some(&mut self.work_tabs)
        } else {
            self.session_view_states
                .get_mut(&owner.session_id)
                .map(|state| &mut state.work_tabs)
        };
        if let Some(tab) =
            tabs.and_then(|tabs| tabs.tabs.iter_mut().find(|tab| tab.id == owner.tab_id))
        {
            tab.initialization_failed = true;
            tab.error = Some(message.clone());
        }
        if self.current_browser_owner().as_ref() == Some(owner) {
            self.browser_error = Some(message);
        }
    }

    pub(super) fn register_ready_browser_page(&mut self, page: &BrowserPage) {
        if !self.browser_owner_is_confirmed(&page.owner)
            || self
                .browser_owner_for_session(&page.owner.session_id)
                .as_ref()
                != Some(&page.owner)
        {
            return;
        }
        let old: Vec<_> = self
            .browser_registration_attempts
            .keys()
            .filter(|owner| owner.session_id == page.owner.session_id && **owner != page.owner)
            .cloned()
            .collect();
        for owner in old {
            self.retire_browser_registration(&owner);
        }
        if self
            .browser_registration_attempts
            .iter()
            .any(|(owner, registration)| {
                owner.session_id == page.owner.session_id && registration.retiring
            })
        {
            return;
        }
        if let Some(registration) = self.browser_registration_attempts.get(&page.owner) {
            if registration.page_instance != page.page_instance {
                self.retire_browser_registration(&page.owner);
            }
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
        self.browser_registration_attempts.insert(
            page.owner.clone(),
            BrowserRegistration {
                page_instance: page.page_instance.clone(),
                pending_replies: 1,
                retiring: false,
            },
        );
        self.send_command(ClientMessage::RegisterBrowserPage {
            session_id: page.owner.session_id.clone(),
            page_instance: page.page_instance.clone(),
            cdp_endpoint: ready.cdp_endpoint,
            target_id: ready.target_id,
        });
    }

    fn retire_browser_registration(&mut self, owner: &BrowserOwner) {
        let Some(registration) = self.browser_registration_attempts.get_mut(owner) else {
            return;
        };
        if registration.retiring {
            return;
        }
        registration.retiring = true;
        // Both replies carry the same page identity. Drain registration and
        // revocation before reusing it, so a late failure cannot mark a new activation.
        registration.pending_replies += 1;
        let page_instance = registration.page_instance.clone();
        self.send_command(ClientMessage::UnregisterBrowserPage {
            session_id: owner.session_id.clone(),
            page_instance,
        });
    }

    pub(super) fn complete_browser_registration(
        &mut self,
        session_id: &str,
        instance: &str,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some((owner, registration)) =
            self.browser_registration_attempts
                .iter_mut()
                .find(|(owner, registration)| {
                    owner.session_id == session_id && registration.page_instance == instance
                })
        else {
            return;
        };
        if registration.pending_replies == 0 {
            return;
        }
        registration.pending_replies -= 1;
        let owner = owner.clone();
        let retiring = registration.retiring;
        let complete = registration.pending_replies == 0;
        if retiring && complete {
            self.browser_registration_attempts.remove(&owner);
        }
        if !retiring && self.browser_owner_is_confirmed(&owner) {
            if let Some(portal) = &self.browser_portal {
                if let Some(page) = portal
                    .page_for_instance(instance)
                    .filter(|page| page.owner == owner)
                {
                    portal.set_error(&page, error.clone());
                }
            }
            if self.current_browser_owner().as_ref() == Some(&owner) {
                self.browser_error = error;
                cx.notify();
            }
        }
        if complete {
            if let Some(page) = self
                .browser_owner_for_session(session_id)
                .and_then(|owner| self.browser_portal.as_ref()?.page_state(&owner))
                .map(|state| state.page)
            {
                self.register_ready_browser_page(&page);
            }
        }
    }

    pub(super) fn apply_browser_events(&mut self, _cx: &mut Context<Self>) -> bool {
        self.save_browser_tab_draft();
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
                    if self
                        .browser_registration_attempts
                        .get(&page.owner)
                        .is_some_and(|registration| {
                            registration.page_instance == page.page_instance
                        })
                    {
                        self.retire_browser_registration(&page.owner);
                    }
                    page
                }
                BrowserEvent::AddressChanged { page, .. }
                | BrowserEvent::TitleChanged { page, .. }
                | BrowserEvent::Error { page, .. } => page,
            };
            let state = self
                .browser_portal
                .as_ref()
                .and_then(|portal| portal.page_state(&page.owner))
                .filter(|state| state.page == page && self.browser_owner_is_confirmed(&page.owner));
            if let Some(state) = state {
                let selected = self.model.ui.selected_session.as_deref()
                    == Some(page.owner.session_id.as_str());
                let tabs = if selected {
                    Some(&mut self.work_tabs)
                } else {
                    self.session_view_states
                        .get_mut(&page.owner.session_id)
                        .map(|view| &mut view.work_tabs)
                };
                if let Some(tabs) = tabs {
                    changed |= tabs.apply_browser_state(&state) && selected;
                }
                if selected
                    && self.work_tabs.browser_tab_id.as_deref() == Some(page.owner.tab_id.as_str())
                {
                    self.load_browser_tab_chrome();
                }
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
            self.work_tabs.apply_browser_state(&state);
        }
        self.load_browser_tab_chrome();
        self.browser_native_rect = None;
        self.sync_browser_visibility();
    }

    fn load_browser_tab_chrome(&mut self) {
        if let Some(tab) = self.work_tabs.browser_tab() {
            self.browser_url.clone_from(&tab.url);
            self.browser_title.clone_from(&tab.title);
            self.browser_error.clone_from(&tab.error);
        } else {
            self.browser_url.clear();
            self.browser_title.clear();
            self.browser_error = None;
        }
        self.browser_panel_open = self.work_tabs.active().kind == WorkTabKind::Browser;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn returning_to_a_browser_tab_waits_for_retired_registration_replies(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let window =
            cx.add_window(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let shell = window.root(cx).unwrap();
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.command_tx = Some(sender);
            shell.connection_profile = Some(DesktopConnectionProfile::LocalWebSocket { url: "ws://127.0.0.1:25566".into() });
            shell.model.connection = crate::model::ShellConnection::Connected;
            shell.model.ui.selected_session = Some("a".into());
            shell.model.attached_session = Some("a".into());
            shell.browser_confirmed_sessions.insert("a".into());
            let first = shell.work_tabs.active().id.clone();
            let second = shell.work_tabs.new_tab();
            shell.work_tabs.select(&first);
            let owner = shell.current_browser_owner().unwrap();
            shell.browser_registration_attempts.insert(owner.clone(), BrowserRegistration { page_instance: "page-a".into(), pending_replies: 1, retiring: false });
            shell.select_work_tab(&second, cx);
            assert!(shell.browser_registration_attempts[&owner].retiring);
            assert_eq!(shell.browser_registration_attempts[&owner].pending_replies, 2);
            shell.complete_browser_registration("a", "page-a", None, cx);
            shell.select_work_tab(&first, cx);
            assert!(shell.browser_registration_attempts[&owner].retiring);
            shell.complete_browser_registration("a", "page-a", Some("registration superseded".into()), cx);
            assert!(!shell.browser_registration_attempts.contains_key(&owner));
            assert!(shell.browser_error.is_none());
            assert!(matches!(commands.try_recv().unwrap(), ClientMessage::UnregisterBrowserPage { page_instance, .. } if page_instance == "page-a"));
            assert!(commands.try_recv().is_err());
            shell.quitting = true;
        });
    }

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
            tab_id: "browser".into(),
        });
        shell.update(cx, |shell, _| {
            shell.model.connection = crate::model::ShellConnection::Connected;
            shell.connection_profile = Some(profile);
            shell.model.ui.selected_session = Some(page.owner.session_id.clone());
            shell.model.attached_session = Some(page.owner.session_id.clone());
            shell
                .work_tabs
                .active_mut()
                .id
                .clone_from(&page.owner.tab_id);
            shell.work_tabs.browser_tab_id = Some(page.owner.tab_id.clone());
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
