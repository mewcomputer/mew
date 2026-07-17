use cef::*;
use std::cell::RefCell;

wrap_window_delegate! {
    struct BrowserWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1280, height: 800 }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let Some(window) = window else { return };
            let browser_view_slot = self.browser_view.borrow();
            let Some(browser_view) = browser_view_slot.as_ref() else { return };
            let mut view = View::from(browser_view);
            window.add_child_view(Some(&mut view));
            window.show();
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            *self.browser_view.borrow_mut() = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let browser_view_slot = self.browser_view.borrow();
            let Some(browser_view) = browser_view_slot.as_ref() else { return 1 };
            let Some(browser) = browser_view.browser() else { return 1 };
            browser.host().map(|host| host.try_close_browser()).unwrap_or(1)
        }
    }
}

wrap_browser_view_delegate! {
    struct MewBrowserViewDelegate {
        runtime_style: RuntimeStyle,
    }

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            _is_devtools: i32,
        ) -> i32 {
            let Some(popup_browser_view) = popup_browser_view else { return 0 };
            let mut delegate = BrowserWindowDelegate::new(RefCell::new(Some((*popup_browser_view).clone())));
            window_create_top_level(Some(&mut delegate));
            1
        }

        fn browser_runtime_style(&self) -> RuntimeStyle {
            self.runtime_style
        }
    }
}

wrap_client! {
    struct BrowserClient;

    impl Client {}
}

wrap_app! {
    pub struct MewCefApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            #[cfg(target_os = "macos")]
            if std::env::var("MEW_CEF_USE_SYSTEM_KEYCHAIN").as_deref() != Ok("1") {
                let switch = CefString::from("use-mock-keychain");
                if let Some(command_line) = command_line {
                    command_line.append_switch(Some(&switch));
                }
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(MewBrowserProcessHandler::new(RefCell::new(None)))
        }
    }
}

wrap_browser_process_handler! {
    struct MewBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            let url = std::env::var("MEW_CEF_URL").unwrap_or_else(|_| "https://example.com".into());
            let url = CefString::from(url.as_str());
            let settings = BrowserSettings::default();
            let client = BrowserClient::new();
            *self.client.borrow_mut() = Some(client.clone());
            let mut client = Some(client);
            let mut browser_delegate = MewBrowserViewDelegate::new(RuntimeStyle::ALLOY);
            let browser_view = browser_view_create(
                client.as_mut(),
                Some(&url),
                Some(&settings),
                None,
                None,
                Some(&mut browser_delegate),
            );
            let Some(browser_view) = browser_view else {
                panic!("CEF failed to create the browser view");
            };
            let mut window_delegate = BrowserWindowDelegate::new(RefCell::new(Some(browser_view)));
            window_create_top_level(Some(&mut window_delegate));
        }
    }
}
