#![cfg_attr(
    all(not(debug_assertions), not(feature = "sandbox")),
    windows_subsystem = "windows"
)]

mod app;

use cef::ImplCommandLine;

#[cfg(target_os = "macos")]
mod mac;

fn main() -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    let _library = {
        let loader = cef::library_loader::LibraryLoader::new(
            &std::env::current_exe().map_err(|_| "failed to find the CEF host executable")?,
            false,
        );
        if !loader.load() {
            return Err("failed to load Chromium Embedded Framework");
        }
        loader
    };

    // CEF's wrapper objects call into libcef, so configure the API version
    // before creating the command-line wrapper.
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

    let args = cef::args::Args::new();
    let Some(command_line) = args.as_cmd_line() else {
        return Err("failed to parse CEF command line arguments");
    };

    let process_type = cef::CefString::from("type");
    let is_browser_process = command_line.has_switch(Some(&process_type)) == 0;
    let exit_code = cef::execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if !is_browser_process {
        return if exit_code >= 0 {
            Ok(())
        } else {
            Err("CEF helper process failed")
        };
    }
    if exit_code != -1 {
        return Err("CEF browser process failed to initialize");
    }

    #[cfg(target_os = "macos")]
    mac::setup_application();

    let mut cef_app = app::MewCefApp::new();
    let port = std::env::var("MEW_CEF_DEBUG_PORT")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(9223);
    let cache_dir = cef_cache_dir();
    std::fs::create_dir_all(&cache_dir).map_err(|_| "failed to create the CEF cache directory")?;
    let cache_dir = cache_dir.to_string_lossy().into_owned();
    let settings = cef::Settings {
        no_sandbox: (!cfg!(feature = "sandbox")) as _,
        root_cache_path: cef::CefString::from(cache_dir.as_str()),
        remote_debugging_port: port,
        ..Default::default()
    };
    if cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut cef_app),
        std::ptr::null_mut(),
    ) != 1
    {
        return Err("CEF initialization failed");
    }

    #[cfg(target_os = "macos")]
    let _delegate = mac::setup_application_delegate();

    println!("mew-cef-host ready on CDP port {port}");
    cef::run_message_loop();
    cef::shutdown();
    Ok(())
}

fn cef_cache_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("MEW_CEF_CACHE_DIR") {
        return path.into();
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home)
            .join("Library/Application Support/ai.mew.mew/cef-cache");
    }

    std::env::temp_dir().join("mew-cef-host")
}
