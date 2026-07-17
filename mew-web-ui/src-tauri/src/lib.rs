mod supervisor;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use anyhow::anyhow;
use mew_cef_host::embed::{BrowserRect, CefEmbedController, PumpCallback};
use serde::Deserialize;
use supervisor::DaemonSupervisor;
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
struct CefEmbedState {
    controller: Mutex<Option<CefEmbedController>>,
}

impl CefEmbedState {
    fn new(controller: Option<CefEmbedController>) -> Self {
        Self {
            controller: Mutex::new(controller),
        }
    }

    fn controller(&self) -> Result<CefEmbedController, String> {
        self.controller
            .lock()
            .map_err(|_| "CEF state mutex poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "CEF browser is unavailable".to_owned())
    }
}

#[derive(Debug, Deserialize)]
struct BrowserRectPayload {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
}

impl From<BrowserRectPayload> for BrowserRect {
    fn from(value: BrowserRectPayload) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
            visible: value.visible,
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let cef_controller = match initialize_cef(app) {
                Ok(controller) => controller,
                Err(error) => {
                    tracing::warn!(%error, "CEF sibling unavailable; keeping the WKWebView browser");
                    None
                }
            };
            if cef_controller.is_some() {
                let cdp_port =
                    std::env::var("MEW_CEF_DEBUG_PORT").unwrap_or_else(|_| "9223".to_owned());
                std::env::set_var("MEW_BROWSER_CDP_PORT", cdp_port);
            }
            app.manage(CefEmbedState::new(cef_controller));

            // Resolve and launch the daemon only when the frontend asks for
            // its endpoint. This keeps startup failures recoverable in-app.
            app.manage(DaemonSupervisor::new(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            daemon_ws_url,
            cef_browser_available,
            cef_browser_set_rect,
            cef_browser_set_visible,
            cef_browser_navigate,
            cef_browser_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mew desktop");
}

fn initialize_cef<R: tauri::Runtime>(
    app: &tauri::App<R>,
) -> Result<Option<CefEmbedController>, Box<dyn std::error::Error>> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        return Ok(None);
    }

    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("MEW_CEF_FRAMEWORK_PATH").is_none() {
            let bundled = std::env::current_exe()
                .ok()
                .map(|path| path.to_string_lossy().contains(".app/Contents/"))
                .unwrap_or(false);
            if !bundled {
                let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                let development_framework = manifest_dir
                    .join("target/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework");
                let development_framework = if development_framework.exists() {
                    Some(development_framework)
                } else {
                    let fallback = manifest_dir.join(
                        "cef/Chromium Embedded Framework.framework/Chromium Embedded Framework",
                    );
                    fallback.exists().then_some(fallback)
                };
                if let Some(development_framework) = development_framework {
                    std::env::set_var("MEW_CEF_FRAMEWORK_PATH", development_framework);
                }
            }
        }
        if std::env::var_os("MEW_CEF_HELPER_PATH").is_none() {
            let bundled = std::env::current_exe()
                .ok()
                .map(|path| path.to_string_lossy().contains(".app/Contents/"))
                .unwrap_or(false);
            if !bundled {
                let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                if let Ok(entries) = std::fs::read_dir(manifest_dir.join("binaries")) {
                    if let Some(helper) = entries.flatten().map(|entry| entry.path()).find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with("mew-cef-host-helper-"))
                    }) {
                        std::env::set_var("MEW_CEF_HELPER_PATH", helper);
                    }
                }
            }
        }
        let Some(window) = app.get_webview_window("main") else {
            return Err(anyhow!("main webview window was not created").into());
        };
        let parent_view = window
            .ns_view()
            .map_err(|error| anyhow!("get the Tauri content view: {error}"))?;
        let pending_pump = Arc::new(AtomicBool::new(false));
        let app_handle = app.handle().clone();
        let pump: PumpCallback = Arc::new(move |_delay_ms| {
            if pending_pump.swap(true, Ordering::AcqRel) {
                return;
            }
            let pending_pump = pending_pump.clone();
            let _ = app_handle.run_on_main_thread(move || {
                pending_pump.store(false, Ordering::Release);
                CefEmbedController::do_message_loop_work();
            });
        });

        let url = std::env::var("MEW_CEF_URL").unwrap_or_else(|_| "https://example.com".to_owned());
        let controller = CefEmbedController::try_initialize(parent_view as usize, &url, pump)
            .map_err(|error| anyhow!(error))?;
        if controller.is_some() {
            // CEF does not always schedule its first external-pump turn until
            // after initialization returns. Seed one turn after Tauri has
            // re-entered its event loop, never from inside CefInitialize.
            let _ = app.handle().run_on_main_thread(|| {
                CefEmbedController::do_message_loop_work();
            });
        }
        Ok(controller)
    }
}

#[tauri::command]
fn daemon_ws_url(state: State<'_, DaemonSupervisor>) -> Result<String, String> {
    state.websocket_url().map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn cef_browser_available(state: State<'_, CefEmbedState>) -> bool {
    state
        .controller
        .lock()
        .map(|controller| controller.is_some())
        .unwrap_or(false)
}

#[tauri::command]
fn cef_browser_set_rect(
    rect: BrowserRectPayload,
    state: State<'_, CefEmbedState>,
    app: AppHandle,
) -> Result<(), String> {
    let controller = state.controller()?;
    app.run_on_main_thread(move || controller.set_rect_on_main_thread(rect.into()))
        .map_err(|error| format!("schedule CEF bounds update: {error}"))
}

#[tauri::command]
fn cef_browser_set_visible(
    visible: bool,
    state: State<'_, CefEmbedState>,
    app: AppHandle,
) -> Result<(), String> {
    let controller = state.controller()?;
    app.run_on_main_thread(move || controller.set_visible_on_main_thread(visible))
        .map_err(|error| format!("schedule CEF visibility update: {error}"))
}

#[tauri::command]
fn cef_browser_navigate(
    url: String,
    state: State<'_, CefEmbedState>,
    app: AppHandle,
) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("browser navigation only supports http and https URLs".to_owned());
    }
    let controller = state.controller()?;
    app.run_on_main_thread(move || controller.navigate_on_main_thread(&url))
        .map_err(|error| format!("schedule CEF navigation: {error}"))
}

#[tauri::command]
fn cef_browser_close(state: State<'_, CefEmbedState>, app: AppHandle) -> Result<(), String> {
    let controller = state.controller()?;
    app.run_on_main_thread(move || controller.set_visible_on_main_thread(false))
        .map_err(|error| format!("schedule CEF close: {error}"))
}
