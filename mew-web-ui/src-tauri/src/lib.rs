mod supervisor;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use anyhow::anyhow;
use mew_cef_host::embed::{
    BrowserEvent, BrowserEventCallback, BrowserRect, CefEmbedController, PumpCallback,
};
use serde::Serialize;
use serde::Deserialize;
use supervisor::DaemonSupervisor;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct CefEmbedState {
    pump: Mutex<Option<CefPump>>,
    controller: Mutex<Option<CefEmbedController>>,
    owner: Arc<Mutex<Option<String>>>,
}

impl CefEmbedState {
    fn new(
        controller: Option<CefEmbedController>,
        pump: Option<CefPump>,
        owner: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self {
            pump: Mutex::new(pump),
            controller: Mutex::new(controller),
            owner,
        }
    }

    fn controller(&self) -> Result<CefEmbedController, String> {
        self.controller
            .lock()
            .map_err(|_| "CEF state mutex poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "CEF browser is unavailable".to_owned())
    }

    fn claim_owner(&self, owner: &str) -> Result<(), String> {
        self.owner
            .lock()
            .map_err(|_| "CEF owner mutex poisoned".to_owned())
            .map(|mut current| *current = Some(owner.to_owned()))
    }

    fn owns_owner(&self, owner: &str) -> Result<bool, String> {
        self.owner
            .lock()
            .map_err(|_| "CEF owner mutex poisoned".to_owned())
            .map(|current| current.as_deref() == Some(owner))
    }

    fn release_owner(&self, owner: &str) -> Result<(), String> {
        self.owner
            .lock()
            .map_err(|_| "CEF owner mutex poisoned".to_owned())
            .map(|mut current| {
                if current.as_deref() == Some(owner) {
                    *current = None;
                }
            })
    }
}

impl Drop for CefEmbedState {
    fn drop(&mut self) {
        // Stop the external pump before the controller field is dropped and
        // unloads libcef. The explicit order keeps queued callbacks from
        // entering CEF after shutdown.
        if let Ok(mut pump) = self.pump.lock() {
            if let Some(mut pump) = pump.take() {
                pump.stop_and_join();
            }
        }
    }
}

struct CefPump {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CefPump {
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for CefPump {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

#[derive(Debug, Deserialize)]
struct BrowserRectPayload {
    owner: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
}

#[derive(Debug, Deserialize)]
struct BrowserVisibilityPayload {
    owner: String,
    visible: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CefBrowserEventPayload {
    AddressChanged {
        owner: Option<String>,
        url: String,
    },
    TitleChanged {
        owner: Option<String>,
        title: String,
        url: String,
    },
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
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let cef_owner = Arc::new(Mutex::new(None));
            let (cef_controller, cef_pump) = match initialize_cef(app, cef_owner.clone()) {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::warn!(%error, "CEF sibling unavailable; keeping the WKWebView browser");
                    // No tracing subscriber is installed in this binary, so
                    // surface the failure on stderr as well; a silent fallback
                    // makes a broken CEF layout look like a missing feature.
                    eprintln!("CEF sibling unavailable; keeping the WKWebView browser: {error}");
                    (None, None)
                }
            };
            if cef_controller.is_some() {
                // Browser protocol commands run through the daemon, while
                // CEF remains the visible renderer. Point agent-browser at
                // that exact CEF target so semantic actions and pixels cannot
                // diverge into two browser sessions.
                let cdp_port = std::env::var("MEW_CEF_DEBUG_PORT")
                    .ok()
                    .and_then(|value| value.parse::<u16>().ok())
                    .unwrap_or(9223);
                std::env::set_var("MEW_BROWSER_CDP_PORT", cdp_port.to_string());
            }
            app.manage(CefEmbedState::new(cef_controller, cef_pump, cef_owner));

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
    owner: Arc<Mutex<Option<String>>>,
) -> Result<(Option<CefEmbedController>, Option<CefPump>), Box<dyn std::error::Error>> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        return Ok((None, None));
    }

    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("MEW_CEF_FRAMEWORK_PATH").is_none() {
            let executable = std::env::current_exe().ok();
            let bundled_framework = executable.as_ref().and_then(|path| {
                let contents = path.parent()?.parent()?;
                let framework = contents.join(
                    "Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
                );
                framework.is_file().then_some(framework)
            });
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
            if let Some(framework) = bundled_framework.or(development_framework) {
                std::env::set_var("MEW_CEF_FRAMEWORK_PATH", framework);
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
        let pump_active = Arc::new(AtomicBool::new(true));
        let app_handle = app.handle().clone();
        let pump_active_for_callback = pump_active.clone();
        let pump: PumpCallback = Arc::new(move |_delay_ms| {
            if !pump_active_for_callback.load(Ordering::Acquire) {
                return;
            }
            if pending_pump.swap(true, Ordering::AcqRel) {
                return;
            }
            let pending_pump = pending_pump.clone();
            let closure_pump = pending_pump.clone();
            let pump_active = pump_active_for_callback.clone();
            if app_handle
                .run_on_main_thread(move || {
                    closure_pump.store(false, Ordering::Release);
                    if !pump_active.load(Ordering::Acquire) {
                        return;
                    }
                    CefEmbedController::do_message_loop_work();
                })
                .is_err()
            {
                // The queue dropped the callback; clear the flag so the next
                // scheduled work can queue a fresh pump instead of stalling
                // CEF's message loop permanently.
                pending_pump.store(false, Ordering::Release);
            }
        });

        let url = std::env::var("MEW_CEF_URL").unwrap_or_else(|_| "https://example.com".to_owned());
        let app_handle = app.handle().clone();
        let browser_event: BrowserEventCallback = Arc::new(move |event| {
            let owner = owner.lock().ok().and_then(|current| current.clone());
            let payload = match event {
                BrowserEvent::AddressChanged { url } => {
                    CefBrowserEventPayload::AddressChanged { owner, url }
                }
                BrowserEvent::TitleChanged { title, url } => {
                    CefBrowserEventPayload::TitleChanged {
                        owner,
                        title,
                        url,
                    }
                }
            };
            let _ = app_handle.emit("cef-browser-event", payload);
        });
        let controller = CefEmbedController::try_initialize(
            parent_view as usize,
            &url,
            pump,
            browser_event,
        )
        .map_err(|error| anyhow!(error))?;
        if controller.is_some() {
            // CEF does not always schedule its first external-pump turn until
            // after initialization returns. Seed one turn after Tauri has
            // re-entered its event loop, never from inside CefInitialize.
            let _ = app.handle().run_on_main_thread(|| {
                CefEmbedController::do_message_loop_work();
            });

            // Backstop the on-demand pump: a dropped run_on_main_thread
            // callback otherwise starves CEF's message loop silently (the
            // browser stays up but DevTools and child IPC stop making
            // progress). The cadence matches cefclient's own pump timer.
            let timer_handle = app.handle().clone();
            let timer_active = pump_active.clone();
            let thread = std::thread::Builder::new()
                .name("cef-pump".to_owned())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    if !timer_active.load(Ordering::Acquire) {
                        break;
                    }
                    let callback_active = timer_active.clone();
                    if timer_handle
                        .run_on_main_thread(move || {
                            if callback_active.load(Ordering::Acquire) {
                                CefEmbedController::do_message_loop_work();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                })
                .map_err(|error| anyhow!("spawn the CEF pump timer: {error}"))?;
            return Ok((
                controller,
                Some(CefPump {
                    stop: pump_active,
                    thread: Some(thread),
                }),
            ));
        }
        Ok((controller, None))
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
    let owner = rect.owner.clone();
    if rect.visible {
        state.claim_owner(&owner)?;
    } else if !state.owns_owner(&owner)? {
        return Ok(());
    }
    let owner_state = state.owner.clone();
    let scheduled_owner = owner.clone();
    let result = app.run_on_main_thread(move || {
        let owns = owner_state
            .lock()
            .map(|current| current.as_deref() == Some(owner.as_str()))
            .unwrap_or(false);
        if owns {
            controller.set_rect_on_main_thread(rect.into());
        }
    });
    if result.is_err() {
        state.release_owner(&scheduled_owner)?;
    }
    result.map_err(|error| format!("schedule CEF bounds update: {error}"))
}

#[tauri::command]
fn cef_browser_set_visible(
    payload: BrowserVisibilityPayload,
    state: State<'_, CefEmbedState>,
    app: AppHandle,
) -> Result<(), String> {
    let controller = state.controller()?;
    if payload.visible {
        state.claim_owner(&payload.owner)?;
    } else if !state.owns_owner(&payload.owner)? {
        return Ok(());
    }
    let owner = payload.owner;
    let visible = payload.visible;
    let owner_state = state.owner.clone();
    let scheduled_owner = owner.clone();
    let result = app.run_on_main_thread(move || {
        let owns = owner_state
            .lock()
            .map(|current| current.as_deref() == Some(owner.as_str()))
            .unwrap_or(false);
        if !owns {
            return;
        }
        controller.set_visible_on_main_thread(visible);
        if !visible {
            if let Ok(mut current) = owner_state.lock() {
                if current.as_deref() == Some(owner.as_str()) {
                    *current = None;
                }
            }
        }
    });
    if result.is_err() {
        state.release_owner(&scheduled_owner)?;
    }
    result.map_err(|error| format!("schedule CEF visibility update: {error}"))
}

#[tauri::command]
fn cef_browser_navigate(
    url: String,
    owner: String,
    state: State<'_, CefEmbedState>,
    app: AppHandle,
) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("browser navigation only supports http and https URLs".to_owned());
    }
    if !state.owns_owner(&owner)? {
        return Ok(());
    }
    let controller = state.controller()?;
    let owner_state = state.owner.clone();
    let scheduled_owner = owner.clone();
    let result = app.run_on_main_thread(move || {
        let owns = owner_state
            .lock()
            .map(|current| current.as_deref() == Some(owner.as_str()))
            .unwrap_or(false);
        if owns {
            controller.navigate_on_main_thread(&url);
        }
    });
    if result.is_err() {
        state.release_owner(&scheduled_owner)?;
    }
    result.map_err(|error| format!("schedule CEF navigation: {error}"))
}

#[tauri::command]
fn cef_browser_close(
    owner: String,
    state: State<'_, CefEmbedState>,
    app: AppHandle,
) -> Result<(), String> {
    if !state.owns_owner(&owner)? {
        return Ok(());
    }
    let controller = state.controller()?;
    let owner_state = state.owner.clone();
    let scheduled_owner = owner.clone();
    let result = app.run_on_main_thread(move || {
        let owns = owner_state
            .lock()
            .map(|current| current.as_deref() == Some(owner.as_str()))
            .unwrap_or(false);
        if !owns {
            return;
        }
        controller.set_visible_on_main_thread(false);
        if let Ok(mut current) = owner_state.lock() {
            if current.as_deref() == Some(owner.as_str()) {
                *current = None;
            }
        }
    });
    if result.is_err() {
        state.release_owner(&scheduled_owner)?;
    }
    result.map_err(|error| format!("schedule CEF close: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{CefEmbedState, CefPump};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    #[test]
    fn a_new_tab_owner_replaces_the_old_owner() {
        let state = test_state();
        state.claim_owner("tab-a").unwrap();
        assert!(state.owns_owner("tab-a").unwrap());

        state.claim_owner("tab-b").unwrap();
        assert!(!state.owns_owner("tab-a").unwrap());
        assert!(state.owns_owner("tab-b").unwrap());
    }

    #[test]
    fn stale_cleanup_cannot_release_the_new_owner() {
        let state = test_state();
        state.claim_owner("tab-a").unwrap();
        state.claim_owner("tab-b").unwrap();

        assert!(!state.owns_owner("tab-a").unwrap());
        assert!(state.owns_owner("tab-b").unwrap());
    }

    #[test]
    fn releasing_an_owner_is_scoped_to_that_owner() {
        let state = test_state();
        state.claim_owner("tab-a").unwrap();
        state.claim_owner("tab-b").unwrap();

        state.release_owner("tab-a").unwrap();
        assert!(state.owns_owner("tab-b").unwrap());

        state.release_owner("tab-b").unwrap();
        assert!(!state.owns_owner("tab-b").unwrap());
    }

    #[test]
    fn stopping_the_pump_signals_and_joins_the_worker() {
        let stop = Arc::new(AtomicBool::new(false));
        let observed = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_observed = observed.clone();
        let thread = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            worker_observed.store(true, Ordering::Release);
        });

        let mut pump = CefPump {
            stop,
            thread: Some(thread),
        };
        pump.stop_and_join();

        assert!(observed.load(Ordering::Acquire));
        assert!(pump.thread.is_none());
    }

    fn test_state() -> CefEmbedState {
        CefEmbedState::new(None, None, Arc::new(Mutex::new(None)))
    }
}
