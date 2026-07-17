//! Embed a windowed CEF browser as a native child of an existing macOS view.

#[cfg(target_os = "macos")]
mod mac_impl {
    use crate::mac;
    use cef::*;
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::{
        ffi::{CString, c_void},
        os::unix::ffi::OsStrExt,
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    pub type PumpCallback = Arc<dyn Fn(i64) + Send + Sync + 'static>;

    #[derive(Clone, Copy, Debug)]
    pub struct BrowserRect {
        pub x: f64,
        pub y: f64,
        pub width: f64,
        pub height: f64,
        pub visible: bool,
    }

    pub struct CefEmbedController {
        state: Arc<EmbedState>,
        owns_runtime: bool,
    }

    struct EmbedState {
        browser: Mutex<Option<Browser>>,
        last_rect: Mutex<Option<BrowserRect>>,
        native_view: AtomicUsize,
        initialized: AtomicBool,
    }

    // CEF objects are only touched from the macOS main thread. The controller
    // itself is passed through Tauri state, which requires Send + Sync.
    unsafe impl Send for EmbedState {}
    unsafe impl Sync for EmbedState {}

    impl Clone for CefEmbedController {
        fn clone(&self) -> Self {
            Self {
                state: self.state.clone(),
                owns_runtime: false,
            }
        }
    }

    impl CefEmbedController {
        pub fn try_initialize(
            parent_view: usize,
            url: &str,
            pump: PumpCallback,
        ) -> Result<Option<Self>, String> {
            let Some(framework_path) = framework_path() else {
                return Ok(None);
            };

            let framework_display = framework_path.display().to_string();
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

            let state = Arc::new(EmbedState {
                browser: Mutex::new(None),
                last_rect: Mutex::new(None),
                native_view: AtomicUsize::new(0),
                initialized: AtomicBool::new(false),
            });
            let app = Box::leak(Box::new(MewCefApp::new(
                parent_view,
                CefString::from(url),
                state.clone(),
                pump,
            )));
            let cache_dir = cache_dir();
            std::fs::create_dir_all(&cache_dir)
                .map_err(|_| "failed to create the CEF cache directory".to_owned())?;
            let cache_dir = CefString::from(cache_dir.to_string_lossy().as_ref());
            let settings = Settings {
                external_message_pump: 1,
                // The Tauri sibling does not have a separate sandbox bootstrap
                // executable yet. Keep the development host usable until the
                // packaged helper/sandbox layout is wired in.
                no_sandbox: 1,
                remote_debugging_port: debug_port(),
                root_cache_path: cache_dir,
                browser_subprocess_path: helper_path()
                    .map(|path| CefString::from(path.to_string_lossy().as_ref()))
                    .unwrap_or_default(),
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

            // With an external message pump, CEF schedules the first work
            // cycle through `on_schedule_message_pump_work`. Do not call
            // `do_message_loop_work` inline here: CEF can still be inside its
            // initialization path, and re-entering the loop at this point
            // crashes Chromium on macOS when the app is launched again.
            state.initialized.store(true, Ordering::Release);
            Ok(Some(Self {
                state,
                owns_runtime: true,
            }))
        }

        pub fn set_rect_on_main_thread(&self, rect: BrowserRect) {
            if let Ok(mut last_rect) = self.state.last_rect.lock() {
                *last_rect = Some(rect);
            }
            let handle = self.state.native_view.load(Ordering::Acquire);
            if handle == 0 {
                return;
            }
            apply_rect(handle, rect);
        }

        pub fn navigate_on_main_thread(&self, url: &str) {
            let url = CefString::from(url);
            let Ok(mut browser) = self.state.browser.lock() else {
                return;
            };
            let Some(browser) = browser.as_mut() else {
                return;
            };
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&url));
            }
        }

        pub fn close_on_main_thread(&self) {
            let Ok(mut browser) = self.state.browser.lock() else {
                return;
            };
            if let Some(browser) = browser.as_mut() {
                if let Some(host) = browser.host() {
                    host.close_browser(true as _);
                }
            }
            self.state.native_view.store(0, Ordering::Release);
        }

        pub fn shutdown(&self) {
            if self.state.initialized.swap(false, Ordering::AcqRel) {
                if let Ok(mut browser) = self.state.browser.lock() {
                    if let Some(browser) = browser.as_mut() {
                        if let Some(host) = browser.host() {
                            host.close_browser(true as _);
                        }
                    }
                    // Release the Rust wrapper before unloading libcef. The
                    // browser process owns the actual close lifecycle, while
                    // keeping this wrapper alive past CefShutdown would let a
                    // later Drop call into an unloaded library.
                    *browser = None;
                }
                self.state.native_view.store(0, Ordering::Release);
                cef::shutdown();
            }
        }

        pub fn set_visible_on_main_thread(&self, visible: bool) {
            if let Ok(mut last_rect) = self.state.last_rect.lock() {
                if let Some(rect) = last_rect.as_mut() {
                    rect.visible = visible;
                }
            }
            let handle = self.state.native_view.load(Ordering::Acquire);
            if handle == 0 {
                return;
            }
            let view = unsafe { &*(handle as *const NSView) };
            view.setHidden(!visible);
        }

        pub fn do_message_loop_work() {
            cef::do_message_loop_work();
        }
    }

    impl Drop for CefEmbedController {
        fn drop(&mut self) {
            if self.owns_runtime {
                self.shutdown();
            }
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

        let helper_state = Arc::new(EmbedState {
            browser: Mutex::new(None),
            last_rect: Mutex::new(None),
            native_view: AtomicUsize::new(0),
            initialized: AtomicBool::new(false),
        });
        let helper_pump: PumpCallback = Arc::new(|_| {});
        let mut helper_app =
            MewCefApp::new(0, CefString::from("about:blank"), helper_state, helper_pump);
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

    fn cache_dir() -> PathBuf {
        if let Some(path) = std::env::var_os("MEW_CEF_CACHE_DIR") {
            return path.into();
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/ai.mew.mew/cef-desktop-cache"))
            .unwrap_or_else(|| std::env::temp_dir().join("mew-cef-host"))
    }

    fn debug_port() -> i32 {
        std::env::var("MEW_CEF_DEBUG_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(9223)
    }

    fn apply_rect(handle: usize, rect: BrowserRect) {
        let view = unsafe { &*(handle as *const NSView) };
        view.setFrame(NSRect {
            origin: NSPoint::new(rect.x, rect.y),
            size: NSSize::new(rect.width.max(1.0), rect.height.max(1.0)),
        });
        view.setHidden(!rect.visible);
    }

    wrap_app! {
        struct MewCefApp {
            parent_view: usize,
            url: CefString,
            state: Arc<EmbedState>,
            pump: PumpCallback,
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
                Some(MewBrowserProcessHandler::new(
                    self.parent_view,
                    self.url.clone(),
                    self.state.clone(),
                    self.pump.clone(),
                ))
            }
        }
    }

    wrap_browser_process_handler! {
        struct MewBrowserProcessHandler {
            parent_view: usize,
            url: CefString,
            state: Arc<EmbedState>,
            pump: PumpCallback,
        }

        impl BrowserProcessHandler {
            fn on_context_initialized(&self) {
                let client = MewBrowserClient::new(self.state.clone());
                let mut client = Some(client);
                let bounds = Rect { x: 0, y: 0, width: 1, height: 1 };
                let window_info = WindowInfo::default().set_as_child(
                    self.parent_view as *mut c_void,
                    &bounds,
                );
                let settings = BrowserSettings::default();
                let browser = browser_host_create_browser_sync(
                    Some(&window_info),
                    client.as_mut(),
                    Some(&self.url),
                    Some(&settings),
                    None,
                    None,
                );
                if let Some(browser) = browser {
                    if let Some(host) = browser.host() {
                        self.state
                            .native_view
                            .store(host.window_handle() as usize, Ordering::Release);
                    }
                    *self.state.browser.lock().expect("CEF browser mutex poisoned") = Some(browser);
                }
            }

            fn on_schedule_message_pump_work(&self, delay_ms: i64) {
                (self.pump)(delay_ms);
            }
        }
    }

    wrap_client! {
        struct MewBrowserClient {
            state: Arc<EmbedState>,
        }

        impl Client {
            fn life_span_handler(&self) -> Option<LifeSpanHandler> {
                Some(MewLifeSpanHandler::new(self.state.clone()))
            }
        }
    }

    wrap_life_span_handler! {
        struct MewLifeSpanHandler {
            state: Arc<EmbedState>,
        }

        impl LifeSpanHandler {
            fn on_after_created(&self, browser: Option<&mut Browser>) {
                let Some(browser) = browser else { return };
                if let Some(host) = browser.host() {
                    let handle = host.window_handle() as usize;
                    self.state.native_view.store(handle, Ordering::Release);
                    if let Ok(last_rect) = self.state.last_rect.lock() {
                        if let Some(rect) = *last_rect {
                            apply_rect(handle, rect);
                        }
                    }
                }
            }

            fn on_before_close(&self, _browser: Option<&mut Browser>) {
                self.state.native_view.store(0, Ordering::Release);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::is_helper_process;

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

    pub use CefEmbedController as Controller;
}

#[cfg(target_os = "macos")]
pub use mac_impl::{
    BrowserRect, Controller as CefEmbedController, PumpCallback, run_subprocess_if_needed,
};

#[cfg(not(target_os = "macos"))]
mod non_macos {
    use std::sync::Arc;

    pub type PumpCallback = Arc<dyn Fn(i64) + Send + Sync + 'static>;

    #[derive(Clone, Copy, Debug)]
    pub struct BrowserRect {
        pub x: f64,
        pub y: f64,
        pub width: f64,
        pub height: f64,
        pub visible: bool,
    }

    #[derive(Clone, Default)]
    pub struct CefEmbedController;

    impl CefEmbedController {
        pub fn try_initialize(
            _parent_view: usize,
            _url: &str,
            _pump: PumpCallback,
        ) -> Result<Option<Self>, String> {
            Ok(None)
        }

        pub fn set_rect_on_main_thread(&self, _rect: BrowserRect) {}
        pub fn set_visible_on_main_thread(&self, _visible: bool) {}
        pub fn navigate_on_main_thread(&self, _url: &str) {}
        pub fn do_message_loop_work() {}
    }

    pub fn run_subprocess_if_needed() -> bool {
        false
    }
}

#[cfg(not(target_os = "macos"))]
pub use non_macos::{BrowserRect, CefEmbedController, PumpCallback, run_subprocess_if_needed};
