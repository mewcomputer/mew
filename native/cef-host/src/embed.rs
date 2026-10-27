//! Embed a windowed CEF browser as a native child of an existing macOS view.

use std::sync::Arc;

pub use crate::app_bridge::{AppRequestHandler, AppResponse};

#[derive(Clone, Debug)]
pub enum BrowserEvent {
    Closed,
    AddressChanged {
        url: String,
    },
    TitleChanged {
        title: String,
        url: String,
    },
    Ready {
        cdp_endpoint: String,
        target_id: String,
    },
    Error {
        message: String,
    },
}

pub type BrowserEventCallback = Arc<dyn Fn(BrowserEvent) + Send + Sync + 'static>;

#[cfg(target_os = "macos")]
mod mac_impl {
    use super::{AppRequestHandler, AppResponse, BrowserEvent, BrowserEventCallback};
    use crate::{
        app_bridge::{APP_URL_KEY, QueryResponse, TrustedAppOrigin, router_config},
        mac,
    };
    use cef::wrapper::message_router::{
        BrowserSideCallback, BrowserSideHandler, BrowserSideRouter, MessageRouterBrowserSide,
        MessageRouterBrowserSideHandlerCallbacks,
    };
    use cef::*;
    use objc2::sel;
    use objc2_app_kit::{NSResponder, NSView};
    use objc2_foundation::{NSObjectNSDelayedPerforming, NSPoint, NSRect, NSSize};
    use std::{
        collections::HashMap,
        ffi::{CString, c_void},
        os::unix::ffi::OsStrExt,
        path::PathBuf,
        sync::{
            Arc, Mutex, Weak,
            atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
        },
    };

    #[derive(Clone, Copy, Debug)]
    pub struct BrowserRect {
        pub x: f64,
        pub y: f64,
        pub width: f64,
        pub height: f64,
        pub visible: bool,
    }

    pub struct CefEmbedController {
        state: Arc<RuntimeState>,
        owns_runtime: bool,
    }

    struct RuntimeState {
        initialized: AtomicBool,
        context_ready: AtomicBool,
        pending_pages: Mutex<Vec<Arc<PageState>>>,
        pages: Mutex<Vec<Weak<PageState>>>,
        cdp_endpoint: String,
    }

    impl RuntimeState {
        fn new(cdp_endpoint: String) -> Self {
            Self {
                initialized: AtomicBool::new(false),
                context_ready: AtomicBool::new(false),
                pending_pages: Mutex::new(Vec::new()),
                pages: Mutex::new(Vec::new()),
                cdp_endpoint,
            }
        }
    }

    pub struct CefPageController {
        state: Arc<PageState>,
    }

    struct PageState {
        parent_view: usize,
        initial_url: String,
        runtime: Arc<RuntimeState>,
        browser: Mutex<Option<Browser>>,
        devtools_registration: Mutex<Option<Registration>>,
        target_request_id: AtomicI32,
        last_rect: Mutex<Option<BrowserRect>>,
        pending_navigation: Mutex<Option<String>>,
        native_view: AtomicUsize,
        interactive: AtomicBool,
        input_shield: Mutex<Option<objc2::rc::Retained<mac::BrowserInputShield>>>,
        closed: AtomicBool,
        browser_event: BrowserEventCallback,
        app: Option<AppPage>,
    }

    struct AppPage {
        origin: TrustedAppOrigin,
        handler: AppRequestHandler,
        router: Arc<BrowserSideRouter>,
        pending: Mutex<HashMap<i64, Weak<QueryResponse>>>,
        drag_view: Mutex<Option<objc2::rc::Retained<mac::AppDragView>>>,
    }

    // CEF objects are only touched from the macOS main thread. Page controllers
    // cross the GPUI portal boundary, which requires Send + Sync.
    unsafe impl Send for PageState {}
    unsafe impl Sync for PageState {}

    impl PageState {
        fn new(
            parent_view: usize,
            initial_url: &str,
            runtime: Arc<RuntimeState>,
            browser_event: BrowserEventCallback,
        ) -> Self {
            Self {
                parent_view,
                initial_url: initial_url.to_owned(),
                runtime,
                browser: Mutex::new(None),
                devtools_registration: Mutex::new(None),
                target_request_id: AtomicI32::new(0),
                last_rect: Mutex::new(None),
                pending_navigation: Mutex::new(None),
                native_view: AtomicUsize::new(0),
                interactive: AtomicBool::new(true),
                input_shield: Mutex::new(None),
                closed: AtomicBool::new(false),
                browser_event,
                app: None,
            }
        }

        fn accepts_callbacks(&self) -> bool {
            !self.closed.load(Ordering::Acquire) && self.runtime.initialized.load(Ordering::Acquire)
        }

        fn emit(&self, event: BrowserEvent) {
            if self.accepts_callbacks() {
                (self.browser_event)(event);
            }
        }

        fn navigate(&self, url: &str) {
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            if self
                .app
                .as_ref()
                .is_some_and(|app| !app.origin.allows_navigation(url))
            {
                return;
            }

            let Ok(mut browser) = self.browser.lock() else {
                return;
            };
            let Some(browser) = browser.as_mut() else {
                if let Ok(mut pending) = self.pending_navigation.lock() {
                    *pending = Some(url.to_owned());
                }
                return;
            };
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }

        fn close(&self) {
            self.closed.store(true, Ordering::Release);
            self.invalidate_app_document();
            if !self.runtime.initialized.load(Ordering::Acquire) {
                return;
            }
            let browser = self
                .browser
                .lock()
                .ok()
                .and_then(|mut browser| browser.take());
            if let Ok(mut registration) = self.devtools_registration.lock() {
                *registration = None;
            }
            if let Some(app) = &self.app {
                app.router.on_before_close(browser.clone());
            }
            if let Some(browser) = browser
                && let Some(host) = browser.host()
            {
                host.close_browser(1);
            }
            self.native_view.store(0, Ordering::Release);
        }

        fn invalidate_app_document(&self) {
            if let Some(app) = &self.app {
                if let Ok(mut pending) = app.pending.lock() {
                    for response in pending
                        .drain()
                        .filter_map(|(_, response)| response.upgrade())
                    {
                        response.cancel();
                    }
                }
                if let Ok(view) = app.drag_view.lock()
                    && let Some(view) = view.as_ref()
                {
                    view.set_regions(&[]);
                }
            }
        }

        fn accepts_app_frame(&self, browser: &Browser, frame: &Frame) -> bool {
            let Some(app) = &self.app else { return false };
            if !self.accepts_callbacks() {
                return false;
            }
            let expected = self
                .browser
                .lock()
                .ok()
                .and_then(|browser| browser.as_ref().map(|browser| browser.identifier()))
                .unwrap_or_default();
            app.origin.allows_frame(
                expected,
                browser.identifier(),
                frame.is_main() != 0,
                &CefString::from(&frame.url()).to_string(),
            )
        }
    }

    struct AppQueryHandler(Weak<PageState>);

    impl BrowserSideHandler for AppQueryHandler {
        fn on_query_str(
            &self,
            browser: Option<Browser>,
            frame: Option<Frame>,
            query_id: i64,
            request: &str,
            persistent: bool,
            callback: Arc<Mutex<dyn BrowserSideCallback>>,
        ) -> bool {
            let (Some(page), Some(browser), Some(frame)) = (self.0.upgrade(), browser, frame)
            else {
                return false;
            };
            if persistent || !page.accepts_app_frame(&browser, &frame) {
                return false;
            }
            let Some(app) = &page.app else { return false };
            let response = AppResponse::new(callback);
            if let Ok(mut pending) = app.pending.lock() {
                pending.retain(|_, response| response.strong_count() > 0);
                pending.insert(query_id, response.downgrade());
            } else {
                response.failure(-1, "app requests are unavailable");
                return true;
            }
            (app.handler)(request, response);
            true
        }

        fn on_query_canceled(
            &self,
            _browser: Option<Browser>,
            _frame: Option<Frame>,
            query_id: i64,
        ) {
            if let Some(page) = self.0.upgrade()
                && let Some(app) = &page.app
                && let Ok(mut pending) = app.pending.lock()
                && let Some(response) = pending
                    .remove(&query_id)
                    .and_then(|response| response.upgrade())
            {
                response.cancel();
            }
        }
    }

    impl CefEmbedController {
        pub fn try_initialize() -> Result<Option<Self>, String> {
            let Some(framework_path) = framework_path() else {
                return Ok(None);
            };

            let framework_display = framework_path.display().to_string();
            let framework_dir = framework_path.parent().map(|path| path.to_path_buf());
            let resources_dir = framework_path
                .parent()
                .map(|path| path.join("Resources"))
                .filter(|path| path.is_dir());
            let locales_dir = resources_dir.as_ref().map(|path| path.join("locales"));
            let framework_path = CString::new(framework_path.as_os_str().as_bytes())
                .map_err(|_| "CEF framework path contains an invalid nul byte".to_owned())?;
            let load_result = unsafe { cef::load_library(Some(&*framework_path.as_ptr().cast())) };
            if load_result != 1 {
                return Err(format!(
                    "failed to load Chromium Embedded Framework from {} (result {load_result})",
                    framework_display
                ));
            }

            let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
            mac::setup_existing_application();

            let args = Box::leak(Box::new(cef::args::Args::new()));
            let Some(command_line) = args.as_cmd_line() else {
                return Err("failed to parse CEF command line arguments".to_owned());
            };
            let process_type = CefString::from("type");
            if command_line.has_switch(Some(&process_type)) != 0 {
                return Err("CEF helper processes must use the bundled helper target".to_owned());
            }

            let configured_port = std::env::var("MEW_CEF_DEBUG_PORT").ok();
            let debug_port = resolve_debug_port(configured_port.as_deref())?;
            let state = Arc::new(RuntimeState::new(format!("http://127.0.0.1:{debug_port}")));
            let app = Box::leak(Box::new(MewCefApp::new(
                state.clone(),
                crate::app_bridge::render_process_handler(),
            )));
            let cache_dir = cache_dir();
            std::fs::create_dir_all(&cache_dir)
                .map_err(|_| "failed to create the CEF cache directory".to_owned())?;
            let cache_dir = CefString::from(cache_dir.to_string_lossy().as_ref());
            let browser_subprocess_path = if helper_app_bundles_available() {
                CefString::default()
            } else {
                helper_path()
                    .map(|path| CefString::from(path.to_string_lossy().as_ref()))
                    .unwrap_or_default()
            };
            let settings = Settings {
                // GPUI owns the running AppKit loop and periodically calls
                // do_message_loop_work. CEF's native pump would register loop
                // observers mid-iteration and see an exit without an entry.
                external_message_pump: 1,
                // The native embedding mode does not have a separate sandbox
                // bootstrap executable yet. Keep the host usable until the
                // packaged helper/sandbox layout is hardened.
                no_sandbox: 1,
                remote_debugging_port: i32::from(debug_port),
                root_cache_path: cache_dir,
                framework_dir_path: framework_dir
                    .as_ref()
                    .map(|path| CefString::from(path.to_string_lossy().as_ref()))
                    .unwrap_or_default(),
                main_bundle_path: main_bundle_path()
                    .map(|path| CefString::from(path.to_string_lossy().as_ref()))
                    .unwrap_or_default(),
                resources_dir_path: resources_dir
                    .as_ref()
                    .map(|path| CefString::from(path.to_string_lossy().as_ref()))
                    .unwrap_or_default(),
                locales_dir_path: locales_dir
                    .as_ref()
                    .map(|path| CefString::from(path.to_string_lossy().as_ref()))
                    .unwrap_or_default(),
                browser_subprocess_path,
                ..Default::default()
            };

            if cef::initialize(
                Some(args.as_main_args()),
                Some(&settings),
                Some(app),
                std::ptr::null_mut(),
            ) != 1
            {
                return Err("CEF initialization failed".to_owned());
            }

            // GPUI periodically pumps CEF on AppKit's main thread. Do not call
            // `do_message_loop_work` inline here: CEF can still be inside its
            // initialization path, and re-entering the loop at this point
            // crashes Chromium on macOS when the app is launched again.
            state.initialized.store(true, Ordering::Release);
            Ok(Some(Self {
                state,
                owns_runtime: true,
            }))
        }

        pub fn create_page(
            &self,
            parent_view: usize,
            url: &str,
            browser_event: BrowserEventCallback,
        ) -> Result<CefPageController, String> {
            if !self.state.initialized.load(Ordering::Acquire) {
                return Err("CEF runtime is not initialized".into());
            }
            let state = Arc::new(PageState::new(
                parent_view,
                url,
                self.state.clone(),
                browser_event,
            ));
            self.schedule_page(state)
        }

        pub fn create_app_page(
            &self,
            parent_view: usize,
            url: &str,
            browser_event: BrowserEventCallback,
            app_request_handler: AppRequestHandler,
        ) -> Result<CefPageController, String> {
            if !self.state.initialized.load(Ordering::Acquire) {
                return Err("CEF runtime is not initialized".into());
            }
            let origin = TrustedAppOrigin::new(url)?;
            let router = BrowserSideRouter::new(router_config());
            let mut state = PageState::new(parent_view, url, self.state.clone(), browser_event);
            state.app = Some(AppPage {
                origin,
                handler: app_request_handler,
                router: router.clone(),
                pending: Mutex::new(HashMap::new()),
                drag_view: Mutex::new(None),
            });
            let state = Arc::new(state);
            if router
                .add_handler(Arc::new(AppQueryHandler(Arc::downgrade(&state))), true)
                .is_none()
            {
                return Err("CEF could not register the app query handler".into());
            }
            self.schedule_page(state)
        }

        fn schedule_page(&self, state: Arc<PageState>) -> Result<CefPageController, String> {
            self.state
                .pages
                .lock()
                .map_err(|_| "CEF pages mutex poisoned")?
                .push(Arc::downgrade(&state));
            if self.state.context_ready.load(Ordering::Acquire) {
                create_native_page(state.clone());
            } else {
                self.state
                    .pending_pages
                    .lock()
                    .map_err(|_| "CEF pending pages mutex poisoned")?
                    .push(state.clone());
            }
            Ok(CefPageController { state })
        }

        pub fn shutdown(&mut self) {
            if self.owns_runtime && self.state.initialized.load(Ordering::Acquire) {
                if let Ok(mut pages) = self.state.pages.lock() {
                    for page in pages.drain(..).filter_map(|page| page.upgrade()) {
                        page.close();
                    }
                }
                if let Ok(mut pages) = self.state.pending_pages.lock() {
                    pages.clear();
                }
                self.state.initialized.store(false, Ordering::Release);
                self.owns_runtime = false;
                cef::shutdown();
            }
        }

        /// GPUI's quit observers run with its application borrow held. Calling
        /// CEF shutdown can re-enter NSApplication termination, so process exit
        /// disarms all pages without invoking CEF and lets the OS reclaim them.
        pub fn prepare_for_process_exit(&mut self) {
            self.state.initialized.store(false, Ordering::Release);
            if let Ok(pages) = self.state.pages.lock() {
                for page in pages.iter().filter_map(Weak::upgrade) {
                    page.invalidate_app_document();
                }
            }
            self.owns_runtime = false;
        }

        pub fn do_message_loop_work() {
            cef::do_message_loop_work();
        }
    }

    impl Drop for CefEmbedController {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    impl CefPageController {
        pub fn set_rect_on_main_thread(&self, rect: BrowserRect) {
            if let Ok(mut last_rect) = self.state.last_rect.lock() {
                *last_rect = Some(rect);
            }
            let handle = self.state.native_view.load(Ordering::Acquire);
            if handle == 0 {
                return;
            }
            apply_rect(handle, rect);
            if let Some(app) = &self.state.app
                && let Ok(view) = app.drag_view.lock()
                && let Some(view) = view.as_ref()
            {
                let parent = unsafe { &*(handle as *const NSView) };
                view.setFrame(parent.bounds());
            }
            if let Ok(shield) = self.state.input_shield.lock()
                && let Some(shield) = shield.as_ref()
            {
                let parent = unsafe { &*(handle as *const NSView) };
                shield.setFrame(parent.bounds());
                shield.set_interactive(self.state.interactive.load(Ordering::Acquire));
            }
        }

        pub fn navigate_on_main_thread(&self, url: &str) {
            self.state.navigate(url);
        }

        /// Dispatch host events only to the trusted application document.
        pub fn execute_javascript_on_main_thread(&self, script: &str) {
            let browser = self
                .state
                .browser
                .lock()
                .ok()
                .and_then(|browser| browser.clone());
            if let Some(browser) = browser
                && let Some(frame) = browser.main_frame()
                && self.state.accepts_app_frame(&browser, &frame)
            {
                frame.execute_java_script(Some(&CefString::from(script)), None, 0);
            }
        }

        pub fn set_visible_on_main_thread(&self, visible: bool) {
            if let Ok(mut last_rect) = self.state.last_rect.lock()
                && let Some(rect) = last_rect.as_mut()
            {
                rect.visible = visible;
            }
            let handle = self.state.native_view.load(Ordering::Acquire);
            if handle == 0 {
                return;
            }
            let view = unsafe { &*(handle as *const NSView) };
            view.setHidden(!visible);
        }

        pub fn focus_on_main_thread(&self) -> bool {
            if !self.state.interactive.load(Ordering::Acquire) {
                return false;
            }
            let Ok(mut browser) = self.state.browser.lock() else {
                return false;
            };
            if let Some(browser) = browser.as_mut()
                && let Some(host) = browser.host()
            {
                host.set_focus(true as _);
                return true;
            }
            false
        }

        pub fn set_interactive_on_main_thread(&self, interactive: bool) {
            self.state.interactive.store(interactive, Ordering::Release);
            if let Ok(shield) = self.state.input_shield.lock()
                && let Some(shield) = shield.as_ref()
            {
                shield.set_interactive(interactive);
            }
            if !interactive
                && let Ok(browser) = self.state.browser.lock()
                && let Some(host) = browser.as_ref().and_then(|browser| browser.host())
            {
                host.set_focus(0);
            }
        }

        pub fn blur_on_main_thread(&self) -> bool {
            let cef_blurred = self
                .state
                .browser
                .lock()
                .ok()
                .and_then(|mut browser| {
                    browser.as_mut().and_then(|browser| {
                        browser.host().map(|host| {
                            host.set_focus(false as _);
                            true
                        })
                    })
                })
                .unwrap_or(false);

            let native_view = self.state.native_view.load(Ordering::Acquire);
            let native_blurred = if native_view == 0 {
                false
            } else {
                let view = unsafe { &*(native_view as *const NSView) };
                view.window()
                    .map(|window| {
                        if let Some(superview) = unsafe { view.superview() } {
                            window.makeFirstResponder(Some(&*superview))
                        } else {
                            window.makeFirstResponder(None::<&NSResponder>)
                        }
                    })
                    .unwrap_or(false)
            };

            cef_blurred || native_blurred
        }

        pub fn close(&self) {
            self.state.close();
        }
    }

    impl Drop for CefPageController {
        fn drop(&mut self) {
            self.close();
        }
    }

    fn create_native_page(state: Arc<PageState>) {
        if !state.accepts_callbacks() {
            return;
        }
        let mut client = Some(MewBrowserClient::new(state.clone()));
        let bounds = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let mut window_info =
            WindowInfo::default().set_as_child(state.parent_view as *mut c_void, &bounds);
        window_info.hidden = 1;
        let settings = BrowserSettings::default();
        let mut extra_info = None;
        let mut request_context = None;
        if state.app.is_some() {
            extra_info = dictionary_value_create();
            if let Some(info) = extra_info.as_mut() {
                info.set_string(
                    Some(&CefString::from(APP_URL_KEY)),
                    Some(&CefString::from(state.initial_url.as_str())),
                );
            }
            request_context =
                request_context_create_context(Some(&RequestContextSettings::default()), None);
            if extra_info.is_none() || request_context.is_none() {
                state.emit(BrowserEvent::Error {
                    message: "CEF could not isolate the app page".into(),
                });
                return;
            }
        }
        if browser_host_create_browser(
            Some(&window_info),
            client.as_mut(),
            Some(&CefString::from(state.initial_url.as_str())),
            Some(&settings),
            extra_info.as_mut(),
            request_context.as_mut(),
        ) == 0
        {
            state.emit(BrowserEvent::Error {
                message: "CEF could not schedule this page".into(),
            });
        }
    }

    pub fn run_subprocess_if_needed() -> bool {
        if !is_helper_process(std::env::args()) {
            return false;
        }

        let Some(framework_path) = framework_path() else {
            return false;
        };
        let Ok(framework_path) = CString::new(framework_path.as_os_str().as_bytes()) else {
            return false;
        };
        if unsafe { cef::load_library(Some(&*framework_path.as_ptr().cast())) } != 1 {
            return false;
        }

        let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

        // Do not construct any CEF wrapper before libcef is loaded. The
        // command-line wrapper calls into the dynamically loaded API and will
        // otherwise dereference an unset function pointer on a normal launch.
        let args = cef::args::Args::new();
        let Some(command_line) = args.as_cmd_line() else {
            return false;
        };
        let process_type = CefString::from("type");
        if command_line.has_switch(Some(&process_type)) == 0 {
            return false;
        }

        let helper_state = Arc::new(RuntimeState::new(String::new()));
        let mut helper_app =
            MewCefApp::new(helper_state, crate::app_bridge::render_process_handler());
        let exit_code = cef::execute_process(
            Some(args.as_main_args()),
            Some(&mut helper_app),
            std::ptr::null_mut(),
        );
        if exit_code < 0 {
            eprintln!("CEF helper process failed to initialize");
        }
        true
    }

    fn is_helper_process<I>(args: I) -> bool
    where
        I: IntoIterator<Item = String>,
    {
        args.into_iter()
            .any(|arg| arg == "--type" || arg.starts_with("--type="))
    }

    fn framework_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("MEW_CEF_FRAMEWORK_PATH") {
            let path = PathBuf::from(path);
            return path.exists().then_some(path);
        }

        let executable = std::env::current_exe().ok()?;
        let contents = executable.parent()?.parent()?;
        let framework = contents
            .join("Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework");
        framework.exists().then_some(framework)
    }

    fn helper_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("MEW_CEF_HELPER_PATH") {
            let path = PathBuf::from(path);
            return path.exists().then_some(path);
        }

        let executable = std::env::current_exe().ok()?;
        let adjacent = executable.parent()?.join("mew-cef-host-helper");
        adjacent.exists().then_some(adjacent)
    }

    fn helper_app_bundles_available() -> bool {
        let Some(app_bundle) = app_bundle_path() else {
            return false;
        };
        let Some(app_name) = std::env::current_exe()
            .ok()
            .and_then(|path| path.file_stem().map(|name| name.to_os_string()))
            .and_then(|name| name.into_string().ok())
        else {
            return false;
        };
        app_bundle
            .join(format!(
                "Contents/Frameworks/{app_name} Helper.app/Contents/MacOS/{app_name} Helper"
            ))
            .is_file()
    }

    fn app_bundle_path() -> Option<PathBuf> {
        if let Some(bundle) = main_bundle_path() {
            return Some(bundle);
        }
        let executable = std::env::current_exe().ok()?;
        let contents = executable.parent()?.parent()?;
        contents.parent().map(PathBuf::from)
    }

    /// Anchor bundle for CEF's macOS bundle lookups.
    ///
    /// Chromium registers the browser's Mach-port rendezvous server under
    /// "<main bundle id>.MachPortRendezvousServer.<pid>" and every helper
    /// resolves that name from its own main bundle. An unbundled browser
    /// executable has no bundle identifier, while helpers resolve the CEF
    /// framework identifier, so the names never match and each helper exits.
    /// Pointing CEF at a real bundle makes both sides agree on one name.
    fn main_bundle_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("MEW_CEF_MAIN_BUNDLE_PATH") {
            let path = PathBuf::from(path);
            return path.is_dir().then_some(path);
        }

        let executable = std::env::current_exe().ok()?;
        if executable.to_string_lossy().contains(".app/Contents/") {
            // Packaged run: CEF derives the main bundle from the executable.
            return None;
        }

        // Development run: the desktop packaging scripts place a bundle
        // either in the profile directory or its standard bundle output
        // directory. Use it so CEF resolves framework-owned dylibs from the
        // app instead of from the bare Rust target directory.
        let profile_dir = executable.parent()?;
        [
            profile_dir.join("bundle/macos/mew.app"),
            profile_dir.join("mew.app"),
        ]
        .into_iter()
        .find(|bundle| bundle.is_dir())
    }

    fn cache_dir() -> PathBuf {
        if let Some(path) = std::env::var_os("MEW_CEF_CACHE_DIR") {
            return path.into();
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/ai.mew.mew/cef-desktop-cache"))
            .unwrap_or_else(|| std::env::temp_dir().join("mew-cef-host"))
    }

    fn resolve_debug_port(configured: Option<&str>) -> Result<u16, String> {
        if let Some(value) = configured {
            return value
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| "MEW_CEF_DEBUG_PORT must be a port between 1 and 65535".into());
        }
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("could not allocate CEF debugging port: {error}"))?;
        listener
            .local_addr()
            .map(|address| address.port())
            .map_err(|error| format!("could not inspect CEF debugging port: {error}"))
    }

    fn apply_rect(handle: usize, rect: BrowserRect) {
        let view = unsafe { &*(handle as *const NSView) };
        // GPUI owns the parent view and can redraw its layer after CEF adds
        // the child. Re-adding the browser to its existing superview keeps
        // the native surface above that layer without involving the UI shell.
        if let Some(superview) = unsafe { view.superview() } {
            superview.addSubview(view);
        }
        view.setFrame(NSRect {
            origin: NSPoint::new(rect.x, rect.y),
            size: NSSize::new(rect.width.max(1.0), rect.height.max(1.0)),
        });
        view.setHidden(!rect.visible);
    }

    wrap_app! {
        struct MewCefApp {
            state: Arc<RuntimeState>,
            renderer: RenderProcessHandler,
        }

        impl App {
            fn on_before_command_line_processing(
                &self,
                _process_type: Option<&CefString>,
                command_line: Option<&mut CommandLine>,
            ) {
                if let Some(command_line) = command_line {
                    if std::env::var("MEW_CEF_USE_SYSTEM_KEYCHAIN").as_deref() != Ok("1") {
                        let switch = CefString::from("use-mock-keychain");
                        command_line.append_switch(Some(&switch));
                    }

                    if std::env::var("MEW_CEF_ENABLE_GPU").as_deref() != Ok("1") {
                        for name in ["disable-gpu", "disable-gpu-compositing", "in-process-gpu"] {
                            let switch = CefString::from(name);
                            command_line.append_switch(Some(&switch));
                        }
                        let renderer = CefString::from("use-angle");
                        let value = CefString::from("swiftshader");
                        command_line.append_switch_with_value(Some(&renderer), Some(&value));
                    }
                }
            }

            fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
                Some(MewBrowserProcessHandler::new(self.state.clone()))
            }

            fn render_process_handler(&self) -> Option<RenderProcessHandler> {
                Some(self.renderer.clone())
            }
        }
    }

    wrap_browser_process_handler! {
        struct MewBrowserProcessHandler {
            state: Arc<RuntimeState>,
        }

        impl BrowserProcessHandler {
            fn on_context_initialized(&self) {
                self.state.context_ready.store(true, Ordering::Release);
                let pending = self.state.pending_pages.lock().map(|mut pages| std::mem::take(&mut *pages))
                    .unwrap_or_default();
                for page in pending {
                    create_native_page(page);
                }
            }

        }
    }

    wrap_client! {
        struct MewBrowserClient {
            state: Arc<PageState>,
        }

        impl Client {
            fn life_span_handler(&self) -> Option<LifeSpanHandler> {
                Some(MewLifeSpanHandler::new(self.state.clone()))
            }

            fn display_handler(&self) -> Option<DisplayHandler> {
                Some(MewDisplayHandler::new(self.state.clone()))
            }

            fn focus_handler(&self) -> Option<FocusHandler> {
                Some(MewFocusHandler::new(self.state.clone()))
            }

            fn request_handler(&self) -> Option<RequestHandler> {
                self.state.app.as_ref().map(|_| MewAppRequestHandler::new(self.state.clone()))
            }

            fn drag_handler(&self) -> Option<DragHandler> {
                self.state.app.as_ref().map(|_| MewAppDragHandler::new(self.state.clone()))
            }

            fn on_process_message_received(
                &self, browser: Option<&mut Browser>, frame: Option<&mut Frame>,
                source_process: ProcessId, message: Option<&mut ProcessMessage>,
            ) -> i32 {
                let Some(app) = &self.state.app else { return 0 };
                if source_process != ProcessId::RENDERER
                    || !browser.as_ref().zip(frame.as_ref()).is_some_and(|(browser, frame)| self.state.accepts_app_frame(browser, frame))
                {
                    return 0;
                }
                i32::from(app.router.on_process_message_received(browser.cloned(), frame.cloned(), source_process, message.cloned()))
            }
        }
    }

    wrap_focus_handler! {
        struct MewFocusHandler {
            state: Arc<PageState>,
        }

        impl FocusHandler {
            fn on_set_focus(&self, _browser: Option<&mut Browser>, _source: FocusSource) -> i32 {
                i32::from(!self.state.interactive.load(Ordering::Acquire)
                    || self.state.closed.load(Ordering::Acquire))
            }
        }
    }

    wrap_drag_handler! {
        struct MewAppDragHandler {
            state: Arc<PageState>,
        }

        impl DragHandler {
            fn on_draggable_regions_changed(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, regions: Option<&[DraggableRegion]>) {
                let (Some(browser), Some(frame)) = (browser, frame) else { return };
                if !self.state.accepts_app_frame(browser, frame) { return; }
                if let Some(app) = &self.state.app
                    && let Ok(view) = app.drag_view.lock()
                    && let Some(view) = view.as_ref()
                {
                    view.set_regions(regions.unwrap_or_default());
                }
            }
        }
    }

    wrap_request_handler! {
        struct MewAppRequestHandler {
            state: Arc<PageState>,
        }

        impl RequestHandler {
            fn on_before_browse(
                &self, browser: Option<&mut Browser>, frame: Option<&mut Frame>,
                request: Option<&mut Request>, _user_gesture: i32, _is_redirect: i32,
            ) -> i32 {
                let Some(app) = &self.state.app else { return 0 };
                if !self.state.accepts_callbacks() {
                    return 1;
                }
                let Some(frame) = frame else { return 1 };
                if frame.is_main() != 0 {
                    let allowed = request.as_ref().is_some_and(|request| {
                        app.origin.allows_navigation(&CefString::from(&request.url()).to_string())
                    });
                    if !allowed {
                        return 1;
                    }
                    self.state.invalidate_app_document();
                }
                app.router.on_before_browse(browser.cloned(), Some(frame.clone()));
                0
            }

            fn on_open_urlfrom_tab(
                &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                _target_url: Option<&CefString>, _target_disposition: WindowOpenDisposition, _user_gesture: i32,
            ) -> i32 {
                1
            }

            fn on_render_process_terminated(
                &self, browser: Option<&mut Browser>, _status: TerminationStatus, _error_code: i32, _error_string: Option<&CefString>,
            ) {
                self.state.invalidate_app_document();
                if let Some(app) = &self.state.app {
                    app.router.on_render_process_terminated(browser.cloned());
                }
                self.state.emit(BrowserEvent::Error { message: "the app renderer stopped".into() });
            }
        }
    }

    wrap_display_handler! {
        struct MewDisplayHandler {
            state: Arc<PageState>,
        }

        impl DisplayHandler {
            fn on_address_change(
                &self,
                _browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                url: Option<&CefString>,
            ) {
                if frame.is_some_and(|frame| frame.is_main() == 0) {
                    return;
                }
                self.state.emit(BrowserEvent::AddressChanged {
                    url: url.map(ToString::to_string).unwrap_or_default(),
                });
            }

            fn on_title_change(
                &self,
                browser: Option<&mut Browser>,
                title: Option<&CefString>,
            ) {
                let url = browser
                    .and_then(|browser| browser.main_frame())
                    .map(|frame| {
                        let url = frame.url();
                        CefString::from(&url).to_string()
                    })
                    .unwrap_or_default();
                self.state.emit(BrowserEvent::TitleChanged {
                    title: title.map(ToString::to_string).unwrap_or_default(),
                    url,
                });
            }
        }
    }

    wrap_life_span_handler! {
        struct MewLifeSpanHandler {
            state: Arc<PageState>,
        }

        impl LifeSpanHandler {
            fn on_before_popup(
                &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: i32,
                target_url: Option<&CefString>, _target_frame_name: Option<&CefString>,
                _target_disposition: WindowOpenDisposition, _user_gesture: i32,
                _popup_features: Option<&PopupFeatures>, _window_info: Option<&mut WindowInfo>,
                _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>,
                _extra_info: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>,
            ) -> i32 {
                if self.state.app.is_some() {
                    return 1;
                }
                // A popup must not replace the opener's native handle or target.
                if self.state.accepts_callbacks()
                    && let Some(units) = target_url.and_then(CefString::as_slice) {
                        let url = String::from_utf16_lossy(units);
                        if url == "about:blank" || url.starts_with("https://") || url.starts_with("http://") {
                            self.state.navigate(&url);
                        } else {
                            self.state.emit(BrowserEvent::Error { message: "browser popup only supports http(s) and about:blank URLs".into() });
                        }
                }
                1
            }

            fn on_after_created(&self, browser: Option<&mut Browser>) {
                let Some(browser) = browser else { return };
                if !self.state.accepts_callbacks() {
                    if let Some(host) = browser.host() { host.close_browser(1); }
                    return;
                }
                *self.state.browser.lock().expect("CEF browser mutex poisoned") =
                    Some(browser.clone());
                let pending_url = self.state.pending_navigation.lock().ok().and_then(|mut pending| pending.take());
                if let Some(url) = pending_url && let Some(frame) = browser.main_frame() {
                    frame.load_url(Some(&CefString::from(url.as_str())));
                }
                if let Some(host) = browser.host() {
                    if self.state.app.is_some() {
                        mac::configure_app_edit_menu();
                        host.set_accessibility_state(State::ENABLED);
                    }
                    let handle = host.window_handle() as usize;
                    self.state.native_view.store(handle, Ordering::Release);
                    // The browser is created before GPUI has had a chance to
                    // claim an active workbench tab. Keep the child hidden
                    // until the first visible bounds update arrives.
                    let view = unsafe { &*(handle as *const NSView) };
                    if let Some(app) = &self.state.app
                        && let Ok(mut drag_view) = app.drag_view.lock()
                    {
                        *drag_view = Some(mac::AppDragView::attach(view));
                    } else if self.state.app.is_none()
                        && let Ok(mut input_shield) = self.state.input_shield.lock()
                    {
                        let shield = mac::BrowserInputShield::attach(view);
                        let interactive = self.state.interactive.load(Ordering::Acquire);
                        shield.set_interactive(interactive);
                        if !interactive { host.set_focus(0); }
                        *input_shield = Some(shield);
                    }
                    mac::register_browser_view(view);
                    view.setHidden(true);
                    if let Ok(last_rect) = self.state.last_rect.lock()
                        && let Some(rect) = *last_rect
                    {
                        apply_rect(handle, rect);
                    }
                    let mut observer = Some(MewTargetObserver::new(self.state.clone()));
                    let registration = host.add_dev_tools_message_observer(observer.as_mut());
                    if registration.is_none() {
                        self.state.emit(BrowserEvent::Error { message: "CEF could not observe this page's target".into() });
                        return;
                    }
                    *self.state.devtools_registration.lock().expect("CEF observer mutex poisoned") = registration;
                    self.state.target_request_id.store(1, Ordering::Release);
                    let request_id = host.execute_dev_tools_method(1, Some(&CefString::from("Target.getTargetInfo")), None);
                    if request_id == 0 {
                        self.state.emit(BrowserEvent::Error { message: "CEF could not request this page's target".into() });
                    }
                }
            }

            fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
                // CEF's default closes the containing GPUI window. Removing
                // only its child view completes browser destruction instead.
                if let Some(host) = browser.and_then(|browser| browser.host()) {
                    let handle = host.window_handle();
                    if !handle.is_null() {
                        let view = unsafe { &*handle.cast::<NSView>() };
                        // The run loop retains the receiver until removal;
                        // defer teardown until CEF's close callback returns.
                        unsafe { view.performSelector_withObject_afterDelay(sel!(removeFromSuperview), None, 0.); }
                    }
                }
                1
            }

            fn on_before_close(&self, browser: Option<&mut Browser>) {
                self.state.invalidate_app_document();
                if let Some(app) = &self.state.app {
                    app.router.on_before_close(browser.cloned());
                }
                self.state.emit(BrowserEvent::Closed);
                self.state.closed.store(true, Ordering::Release);
                self.state.native_view.store(0, Ordering::Release);
                if let Ok(mut browser) = self.state.browser.lock() { *browser = None; }
                if let Ok(mut registration) = self.state.devtools_registration.lock() { *registration = None; }
                if let Ok(mut shield) = self.state.input_shield.lock() { *shield = None; }
            }
        }
    }

    wrap_dev_tools_message_observer! {
        struct MewTargetObserver {
            state: Arc<PageState>,
        }

        impl DevToolsMessageObserver {
            fn on_dev_tools_method_result(
                &self,
                _browser: Option<&mut Browser>,
                message_id: i32,
                success: i32,
                result: Option<&[u8]>,
            ) {
                if message_id == 0 || message_id != self.state.target_request_id.load(Ordering::Acquire)
                    || !self.state.accepts_callbacks() {
                    return;
                }
                let target_id = if success != 0 {
                    parse_json_buffer(result, JsonParserOptions::RFC)
                        .and_then(|value| {
                            // Nested dictionaries remain valid only while their owning Value lives.
                            let dictionary = value.dictionary()?;
                            let target_info = dictionary.dictionary(Some(&CefString::from("targetInfo")))?;
                            let target_id = CefString::from(&target_info.string(Some(&CefString::from("targetId")))).to_string();
                            (!target_id.is_empty()).then_some(target_id)
                        })
                } else { None };
                match target_id {
                    Some(target_id) => self.state.emit(BrowserEvent::Ready {
                        cdp_endpoint: self.state.runtime.cdp_endpoint.clone(),
                        target_id,
                    }),
                    None => self.state.emit(BrowserEvent::Error {
                        message: "CEF could not identify this page's debugging target".into(),
                    }),
                }
            }
        }
    }

    pub use CefEmbedController as Controller;

    #[cfg(test)]
    mod tests {
        use super::{PageState, RuntimeState, is_helper_process};
        use cef::{ImplClient, ImplFocusHandler, ImplLifeSpanHandler};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[test]
        fn default_debugging_uses_a_released_ephemeral_loopback_port() {
            let port = super::resolve_debug_port(None).unwrap();
            assert_ne!(port, 0);
            let listener =
                std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
            assert!(listener.local_addr().unwrap().ip().is_loopback());
        }

        #[test]
        fn explicit_debugging_port_is_validated_without_falling_back() {
            assert_eq!(super::resolve_debug_port(Some("9223")).unwrap(), 9223);
            for value in ["", "0", "-1", "65536", "not-a-port"] {
                let error = super::resolve_debug_port(Some(value)).unwrap_err();
                assert!(error.contains("MEW_CEF_DEBUG_PORT"));
            }
        }

        #[test]
        #[ignore = "requires MEW_CEF_FRAMEWORK_PATH pointing to the native CEF runtime"]
        fn native_target_reply_emits_the_exact_page_identity() {
            use cef::ImplDevToolsMessageObserver;

            let framework = super::framework_path().expect("native CEF runtime path");
            let framework = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(
                framework.as_os_str(),
            ))
            .unwrap();
            assert_eq!(
                unsafe { cef::load_library(Some(&*framework.as_ptr().cast())) },
                1
            );
            let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
            let runtime = Arc::new(RuntimeState::new("http://127.0.0.1:9223".into()));
            runtime.initialized.store(true, Ordering::Release);
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = events.clone();
            let page = Arc::new(PageState::new(
                0,
                "about:blank",
                runtime,
                Arc::new(move |event| captured.lock().unwrap().push(event)),
            ));
            page.target_request_id.store(1, Ordering::Release);
            let observer = super::MewTargetObserver::new(page.clone());
            let reply =
                br#"{"targetInfo":{"targetId":"page-42","type":"page","url":"about:blank"}}"#;
            observer.on_dev_tools_method_result(None, 2, 1, Some(reply));
            assert!(events.lock().unwrap().is_empty());
            observer.on_dev_tools_method_result(None, 1, 1, Some(reply));
            assert!(matches!(
                events.lock().unwrap().as_slice(),
                [super::BrowserEvent::Ready { cdp_endpoint, target_id }]
                if cdp_endpoint == "http://127.0.0.1:9223" && target_id == "page-42"
            ));

            for (success, reply) in [
                (0, Some(reply.as_slice())),
                (1, None),
                (1, Some(b"invalid".as_slice())),
                (1, Some(b"{}".as_slice())),
                (1, Some(br#"{"targetInfo":{"targetId":""}}"#.as_slice())),
            ] {
                events.lock().unwrap().clear();
                observer.on_dev_tools_method_result(None, 1, success, reply);
                assert!(matches!(
                    events.lock().unwrap().as_slice(),
                    [super::BrowserEvent::Error { .. }]
                ));
            }
            events.lock().unwrap().clear();
            page.close();
            observer.on_dev_tools_method_result(None, 1, 1, Some(reply));
            assert!(events.lock().unwrap().is_empty());
        }

        fn request_popup(handler: &cef::LifeSpanHandler, target: Option<&str>) -> i32 {
            let mut units = target
                .unwrap_or_default()
                .encode_utf16()
                .collect::<Vec<_>>();
            let url = cef::CefString::from(cef::sys::_cef_string_utf16_t {
                str_: units.as_mut_ptr(),
                length: units.len(),
                dtor: None,
            });
            handler.on_before_popup(
                None,
                None,
                1,
                target.map(|_| &url),
                None,
                cef::WindowOpenDisposition::NEW_POPUP,
                1,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        }

        #[test]
        fn popups_navigate_the_original_page_without_creating_another_browser() {
            let runtime = Arc::new(super::RuntimeState::new("http://127.0.0.1:1".into()));
            runtime.initialized.store(true, Ordering::Release);
            let page = Arc::new(super::PageState::new(
                0,
                "about:blank",
                runtime,
                Arc::new(|_| {}),
            ));
            let handler = super::MewLifeSpanHandler::new(page.clone());
            assert_eq!(request_popup(&handler, Some("https://example.com/é")), 1);
            assert_eq!(
                page.pending_navigation.lock().unwrap().as_deref(),
                Some("https://example.com/é")
            );
            assert!(!page.closed.load(Ordering::Acquire));
        }

        #[test]
        fn empty_unsupported_and_closed_page_popups_are_cancelled() {
            let runtime = Arc::new(super::RuntimeState::new("http://127.0.0.1:1".into()));
            runtime.initialized.store(true, Ordering::Release);
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = events.clone();
            let page = Arc::new(super::PageState::new(
                0,
                "about:blank",
                runtime,
                Arc::new(move |event| captured.lock().unwrap().push(event)),
            ));
            let handler = super::MewLifeSpanHandler::new(page.clone());
            assert_eq!(request_popup(&handler, None), 1);
            assert!(events.lock().unwrap().is_empty());
            assert_eq!(request_popup(&handler, Some("javascript:alert(1)")), 1);
            assert!(
                matches!(events.lock().unwrap().as_slice(), [super::BrowserEvent::Error { message }] if message.contains("popup"))
            );
            assert!(page.pending_navigation.lock().unwrap().is_none());
            page.close();
            assert_eq!(request_popup(&handler, Some("https://late.example")), 1);
            assert!(page.pending_navigation.lock().unwrap().is_none());
        }

        fn app_page(runtime: Arc<RuntimeState>) -> PageState {
            use cef::wrapper::message_router::{BrowserSideRouter, MessageRouterBrowserSide};
            let mut page = PageState::new(0, "http://127.0.0.1:8080/", runtime, Arc::new(|_| {}));
            page.app = Some(super::AppPage {
                origin: crate::app_bridge::TrustedAppOrigin::new(&page.initial_url).unwrap(),
                handler: Arc::new(|_, _| panic!("no native query was sent")),
                router: BrowserSideRouter::new(crate::app_bridge::router_config()),
                pending: Default::default(),
                drag_view: Default::default(),
            });
            page
        }

        #[test]
        fn app_popups_and_external_navigation_cannot_replace_the_app() {
            let runtime = Arc::new(RuntimeState::new("http://127.0.0.1:1".into()));
            runtime.initialized.store(true, Ordering::Release);
            let page = Arc::new(app_page(runtime));
            let handler = super::MewLifeSpanHandler::new(page.clone());
            assert_eq!(request_popup(&handler, Some("https://example.com/")), 1);
            assert_eq!(request_popup(&handler, Some("http://127.0.0.1:8080/")), 1);
            assert!(page.pending_navigation.lock().unwrap().is_none());
            page.navigate("https://example.com/");
            page.navigate("http://127.0.0.1:8081/");
            assert!(page.pending_navigation.lock().unwrap().is_none());
            page.navigate("http://127.0.0.1:8080/chat/42");
            assert_eq!(
                page.pending_navigation.lock().unwrap().as_deref(),
                Some("http://127.0.0.1:8080/chat/42")
            );
        }

        #[test]
        fn app_navigation_without_a_frame_is_rejected() {
            use cef::ImplRequestHandler;
            let runtime = Arc::new(RuntimeState::new("http://127.0.0.1:1".into()));
            runtime.initialized.store(true, Ordering::Release);
            let handler = super::MewAppRequestHandler::new(Arc::new(app_page(runtime)));
            assert_eq!(handler.on_before_browse(None, None, None, 0, 0), 1);
        }

        #[test]
        fn only_trusted_app_clients_receive_native_drag_regions() {
            use cef::ImplClient;
            let runtime = Arc::new(RuntimeState::new(String::new()));
            let external = Arc::new(PageState::new(
                0,
                "https://example.com/",
                runtime.clone(),
                Arc::new(|_| {}),
            ));
            let client = super::MewBrowserClient::new(external);
            assert!(client.drag_handler().is_none());
            let client = super::MewBrowserClient::new(Arc::new(app_page(runtime)));
            assert!(client.drag_handler().is_some());
        }

        #[test]
        fn embedded_subprocess_keeps_the_same_renderer_handler() {
            use cef::{ImplApp, ImplRenderProcessHandler};
            let runtime = Arc::new(RuntimeState::new(String::new()));
            let app = super::MewCefApp::new(runtime, crate::app_bridge::render_process_handler());
            let first = app.render_process_handler().unwrap();
            let next = app.render_process_handler().unwrap();
            assert_eq!(first.get_raw(), next.get_raw());
        }

        #[test]
        fn denied_app_links_do_not_report_a_runtime_failure() {
            use cef::ImplRequestHandler;
            let runtime = Arc::new(RuntimeState::new("http://127.0.0.1:1".into()));
            runtime.initialized.store(true, Ordering::Release);
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = events.clone();
            let mut page = app_page(runtime);
            page.browser_event = Arc::new(move |event| captured.lock().unwrap().push(event));
            let page = Arc::new(page);
            page.navigate("https://example.com/");
            let popup = super::MewLifeSpanHandler::new(page.clone());
            assert_eq!(request_popup(&popup, Some("https://example.com/")), 1);
            let request = super::MewAppRequestHandler::new(page.clone());
            assert_eq!(
                request.on_open_urlfrom_tab(
                    None,
                    None,
                    None,
                    cef::WindowOpenDisposition::NEW_FOREGROUND_TAB,
                    1
                ),
                1
            );
            assert!(events.lock().unwrap().is_empty());
            assert!(page.pending_navigation.lock().unwrap().is_none());
            assert!(page.accepts_callbacks());
        }

        #[test]
        fn closing_an_uncreated_app_page_disarms_its_pending_response() {
            use cef::wrapper::message_router::BrowserSideCallback;
            struct Replies(Arc<AtomicUsize>);
            impl BrowserSideCallback for Replies {
                fn success_str(&self, _: &str) {
                    self.0.fetch_add(1, Ordering::Relaxed);
                }
                fn success_binary(&self, _: &[u8]) {
                    panic!("not a binary bridge");
                }
                fn failure(&self, _: i32, _: &str) {
                    self.0.fetch_add(1, Ordering::Relaxed);
                }
            }
            let runtime = Arc::new(RuntimeState::new("http://127.0.0.1:1".into()));
            let page = app_page(runtime);
            let replies = Arc::new(AtomicUsize::new(0));
            let response =
                super::AppResponse::new(Arc::new(std::sync::Mutex::new(Replies(replies.clone()))));
            page.app
                .as_ref()
                .unwrap()
                .pending
                .lock()
                .unwrap()
                .insert(7, response.downgrade());
            page.close();
            response.success("stale");
            drop(response);
            assert_eq!(replies.load(Ordering::Relaxed), 0);
            assert!(
                page.app
                    .as_ref()
                    .unwrap()
                    .pending
                    .lock()
                    .unwrap()
                    .is_empty()
            );
        }

        #[test]
        fn closing_an_embedded_page_never_requests_the_parent_window_close() {
            let runtime = Arc::new(RuntimeState::new("http://127.0.0.1:1".into()));
            runtime.initialized.store(true, Ordering::Release);
            let page = Arc::new(PageState::new(0, "about:blank", runtime, Arc::new(|_| {})));
            let handler = super::MewLifeSpanHandler::new(page.clone());
            assert_eq!(handler.do_close(None), 1);
            page.close();
            assert_eq!(handler.do_close(None), 1);
        }

        #[test]
        fn native_close_emits_before_callbacks_become_inactive() {
            let runtime = Arc::new(super::RuntimeState::new("http://127.0.0.1:1".into()));
            runtime.initialized.store(true, Ordering::Release);
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = events.clone();
            let page = Arc::new(super::PageState::new(
                0,
                "about:blank",
                runtime,
                Arc::new(move |event| captured.lock().unwrap().push(event)),
            ));
            let handler = super::MewLifeSpanHandler::new(page.clone());
            handler.on_before_close(None);
            page.emit(super::BrowserEvent::AddressChanged {
                url: "https://late.example".into(),
            });
            let events = events.lock().unwrap();
            assert!(matches!(events.as_slice(), [super::BrowserEvent::Closed]));
            assert!(!page.accepts_callbacks());
        }

        #[test]
        fn navigation_before_native_creation_is_retained() {
            let runtime = Arc::new(super::RuntimeState::new("http://127.0.0.1:1".into()));
            let page = Arc::new(super::PageState::new(
                0,
                "about:blank",
                runtime,
                Arc::new(|_| {}),
            ));
            let controller = super::CefPageController {
                state: page.clone(),
            };
            controller.navigate_on_main_thread("https://early.example");
            assert_eq!(
                page.pending_navigation.lock().unwrap().as_deref(),
                Some("https://early.example")
            );
        }

        #[test]
        fn input_state_before_native_creation_is_retained_without_replacing_the_page() {
            let runtime = Arc::new(RuntimeState::new(String::new()));
            let page = Arc::new(PageState::new(
                0,
                "https://example.com/",
                runtime,
                Arc::new(|_| {}),
            ));
            let controller = super::CefPageController {
                state: page.clone(),
            };
            let client = super::MewBrowserClient::new(page.clone());
            let focus = client
                .focus_handler()
                .expect("browser pages must guard CEF focus requests");
            assert!(page.interactive.load(Ordering::Acquire));
            assert_eq!(focus.on_set_focus(None, cef::FocusSource::NAVIGATION), 0);
            controller.navigate_on_main_thread("https://example.com/retained");
            controller.set_interactive_on_main_thread(false);
            assert!(!page.interactive.load(Ordering::Acquire));
            assert!(!controller.focus_on_main_thread());
            assert_eq!(focus.on_set_focus(None, cef::FocusSource::NAVIGATION), 1);
            assert_eq!(focus.on_set_focus(None, cef::FocusSource::SYSTEM), 1);
            assert!(!page.closed.load(Ordering::Acquire));
            assert_eq!(
                page.pending_navigation.lock().unwrap().as_deref(),
                Some("https://example.com/retained")
            );
            controller.set_interactive_on_main_thread(true);
            assert!(page.interactive.load(Ordering::Acquire));
            assert_eq!(focus.on_set_focus(None, cef::FocusSource::SYSTEM), 0);
            super::MewLifeSpanHandler::new(page.clone()).on_before_close(None);
            assert_eq!(focus.on_set_focus(None, cef::FocusSource::SYSTEM), 1);
        }

        #[test]
        fn cancelling_creation_suppresses_late_native_callbacks() {
            let runtime = Arc::new(RuntimeState::new("http://127.0.0.1:9223".into()));
            runtime.initialized.store(true, Ordering::Release);
            let callbacks = Arc::new(AtomicUsize::new(0));
            let observed = callbacks.clone();
            let page = PageState::new(
                0,
                "about:blank",
                runtime,
                Arc::new(move |_| {
                    observed.fetch_add(1, Ordering::Relaxed);
                }),
            );
            assert!(page.accepts_callbacks());
            page.close();
            assert!(!page.accepts_callbacks());
            page.emit(super::BrowserEvent::AddressChanged {
                url: "https://late.example".into(),
            });
            assert_eq!(callbacks.load(Ordering::Relaxed), 0);
        }

        #[test]
        fn only_cef_process_arguments_enter_the_helper_path() {
            assert!(is_helper_process(vec![
                "mew-desktop".to_owned(),
                "--type=renderer".to_owned(),
            ]));
            assert!(is_helper_process(vec![
                "mew-desktop".to_owned(),
                "--type".to_owned(),
                "gpu-process".to_owned(),
            ]));
            assert!(!is_helper_process(vec!["mew-desktop".to_owned()]));
            assert!(!is_helper_process(vec![
                "mew-desktop".to_owned(),
                "--typeface".to_owned(),
            ]));
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac_impl::{
    BrowserRect, CefPageController, Controller as CefEmbedController, run_subprocess_if_needed,
};

#[cfg(not(target_os = "macos"))]
mod non_macos {
    use super::{AppRequestHandler, BrowserEventCallback};

    #[derive(Clone, Copy, Debug)]
    pub struct BrowserRect {
        pub x: f64,
        pub y: f64,
        pub width: f64,
        pub height: f64,
        pub visible: bool,
    }

    pub struct CefEmbedController;
    pub struct CefPageController;

    impl CefEmbedController {
        pub fn try_initialize() -> Result<Option<Self>, String> {
            Ok(None)
        }
        pub fn create_page(
            &self,
            _parent_view: usize,
            _url: &str,
            _event: BrowserEventCallback,
        ) -> Result<CefPageController, String> {
            Err("native browser is unavailable on this platform".into())
        }
        pub fn create_app_page(
            &self,
            _parent_view: usize,
            _url: &str,
            _event: BrowserEventCallback,
            _handler: AppRequestHandler,
        ) -> Result<CefPageController, String> {
            Err("native app pages are unavailable on this platform".into())
        }
        pub fn shutdown(&mut self) {}
        pub fn prepare_for_process_exit(&mut self) {}
        pub fn do_message_loop_work() {}
    }
    impl CefPageController {
        pub fn set_rect_on_main_thread(&self, _rect: BrowserRect) {}
        pub fn set_visible_on_main_thread(&self, _visible: bool) {}
        pub fn focus_on_main_thread(&self) -> bool {
            false
        }
        pub fn set_interactive_on_main_thread(&self, _interactive: bool) {}
        pub fn blur_on_main_thread(&self) -> bool {
            false
        }
        pub fn navigate_on_main_thread(&self, _url: &str) {}
        pub fn execute_javascript_on_main_thread(&self, _script: &str) {}
        pub fn close(&self) {}
    }

    pub fn run_subprocess_if_needed() -> bool {
        false
    }
}

#[cfg(not(target_os = "macos"))]
pub use non_macos::{BrowserRect, CefEmbedController, CefPageController, run_subprocess_if_needed};
