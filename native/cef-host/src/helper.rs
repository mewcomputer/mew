use cef::args::Args;
use cef::*;

fn main() {
    #[cfg(target_os = "macos")]
    let _loader = {
        let executable = std::env::current_exe().expect("failed to find the CEF helper executable");
        let is_framework_helper = executable
            .components()
            .any(|component| component.as_os_str() == "Helpers");
        let loader = library_loader::LibraryLoader::new(&executable, is_framework_helper);
        assert!(loader.load(), "failed to load Chromium Embedded Framework");
        loader
    };

    // The CEF command-line wrapper calls into the dynamically loaded API, so
    // the library must be loaded before Args is constructed.
    let args = Args::new();

    #[cfg(all(target_os = "macos", feature = "sandbox"))]
    let _sandbox = {
        let mut sandbox = cef::sandbox::Sandbox::new();
        sandbox.initialize(args.as_main_args());
        sandbox
    };

    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let mut app = MewCefHelperApp::new();
    execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
}

wrap_app! {
    struct MewCefHelperApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&cef::CefString>,
            command_line: Option<&mut cef::CommandLine>,
        ) {
            let Some(command_line) = command_line else { return };

            if std::env::var("MEW_CEF_USE_SYSTEM_KEYCHAIN").as_deref() != Ok("1") {
                let switch = cef::CefString::from("use-mock-keychain");
                command_line.append_switch(Some(&switch));
            }

            if std::env::var("MEW_CEF_ENABLE_GPU").as_deref() != Ok("1") {
                for name in ["disable-gpu", "disable-gpu-compositing", "in-process-gpu"] {
                    let switch = cef::CefString::from(name);
                    command_line.append_switch(Some(&switch));
                }
                let renderer = cef::CefString::from("use-angle");
                let value = cef::CefString::from("swiftshader");
                command_line.append_switch_with_value(Some(&renderer), Some(&value));
            }
        }
    }
}
