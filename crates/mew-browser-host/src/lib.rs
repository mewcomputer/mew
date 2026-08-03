//! A small, Tauri-free browser portal for native desktop clients.
//!
//! The desktop UI deals in owners, rectangles, visibility, and typed browser
//! events. CEF handles and its message pump stay behind this boundary so a
//! future off-screen or non-macOS backend can replace the child-view backend.

use anyhow::{bail, Result};
use mew_cef_host::embed::{BrowserEvent as CefBrowserEvent, BrowserRect as CefBrowserRect};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub type PumpTrigger = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrowserRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserEvent {
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

#[derive(Default)]
struct BrowserState {
    owner: Option<String>,
    events: VecDeque<BrowserEvent>,
}

pub struct BrowserPortal {
    #[cfg(target_os = "macos")]
    controller: mew_cef_host::embed::CefEmbedController,
    state: Arc<Mutex<BrowserState>>,
    #[cfg(target_os = "macos")]
    pump_stop: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(target_os = "macos")]
    pump_thread: Option<std::thread::JoinHandle<()>>,
}

impl BrowserPortal {
    /// Initializes a native child browser under the supplied AppKit view.
    ///
    /// `parent_view` is intentionally an opaque platform handle at this
    /// boundary. The GPUI app obtains it through `raw-window-handle`, while
    /// this crate is the only layer that passes it to CEF.
    pub fn initialize(
        parent_view: usize,
        initial_url: &str,
        pump_trigger: PumpTrigger,
    ) -> Result<Option<Self>> {
        let state = Arc::new(Mutex::new(BrowserState::default()));
        let event_state = state.clone();
        let browser_event = Arc::new(move |event: CefBrowserEvent| {
            let Ok(mut state) = event_state.lock() else {
                return;
            };
            enqueue_event(&mut state, event);
        });
        let pump = {
            let trigger = pump_trigger.clone();
            Arc::new(move |_delay_ms: i64| trigger())
        };
        let Some(controller) = mew_cef_host::embed::CefEmbedController::try_initialize(
            parent_view,
            initial_url,
            pump,
            browser_event,
        )
        .map_err(|error| anyhow::anyhow!(error))?
        else {
            return Ok(None);
        };

        #[cfg(target_os = "macos")]
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::time::Duration;

            let pump_stop = Arc::new(AtomicBool::new(false));
            let timer_stop = pump_stop.clone();
            let timer_trigger = pump_trigger.clone();
            let pump_thread = std::thread::Builder::new()
                .name("mew-browser-pump".to_owned())
                .spawn(move || {
                    while !timer_stop.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(30));
                        if !timer_stop.load(Ordering::Acquire) {
                            timer_trigger();
                        }
                    }
                })
                .map_err(|error| anyhow::anyhow!("spawn browser pump: {error}"))?;

            pump_trigger();
            Ok(Some(Self {
                controller,
                state,
                pump_stop,
                pump_thread: Some(pump_thread),
            }))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = controller;
            Ok(None)
        }
    }

    pub fn available(&self) -> bool {
        true
    }

    pub fn pump(&self) {
        mew_cef_host::embed::CefEmbedController::do_message_loop_work();
    }

    /// Stop native browser work during an ordinary portal shutdown.
    pub fn shutdown(&mut self) {
        #[cfg(target_os = "macos")]
        {
            use std::sync::atomic::Ordering;

            self.pump_stop.store(true, Ordering::Release);
            if let Some(thread) = self.pump_thread.take() {
                let _ = thread.join();
            }
            self.controller.shutdown();
        }
    }

    /// Stop browser pumping while the owning application is already terminating.
    ///
    /// CEF shutdown can re-enter `NSApplication terminate:` on macOS. GPUI
    /// invokes app-quit observers while its application borrow is active, so
    /// app teardown must disarm the controller without calling into CEF.
    pub fn prepare_for_process_exit(&mut self) {
        #[cfg(target_os = "macos")]
        {
            use std::sync::atomic::Ordering;

            self.pump_stop.store(true, Ordering::Release);
            if let Some(thread) = self.pump_thread.take() {
                let _ = thread.join();
            }
            self.controller.prepare_for_process_exit();
        }
    }

    pub fn set_rect(&self, owner: &str, rect: BrowserRect) -> Result<()> {
        self.claim_or_check_owner(owner, rect.visible)?;
        self.controller.set_rect_on_main_thread(CefBrowserRect {
            x: rect.x,
            y: rect.y,
            width: rect.width.max(1.0),
            height: rect.height.max(1.0),
            visible: rect.visible,
        });
        Ok(())
    }

    pub fn set_visible(&self, owner: &str, visible: bool) -> Result<()> {
        self.claim_or_check_owner(owner, visible)?;
        self.controller.set_visible_on_main_thread(visible);
        if !visible {
            self.clear_owner(owner);
        }
        Ok(())
    }

    pub fn focus(&self, owner: &str) -> Result<bool> {
        self.require_owner(owner)?;
        Ok(self.controller.focus_on_main_thread())
    }

    pub fn blur(&self, owner: &str) -> Result<bool> {
        self.require_owner(owner)?;
        Ok(self.controller.blur_on_main_thread())
    }

    pub fn navigate(&self, owner: &str, url: &str) -> Result<()> {
        if url != "about:blank" && !(url.starts_with("http://") || url.starts_with("https://")) {
            bail!("browser navigation only supports about:blank, http, and https URLs");
        }
        self.require_owner(owner)?;
        self.controller.navigate_on_main_thread(url);
        Ok(())
    }

    pub fn close(&self, owner: &str) -> Result<()> {
        if !self.is_owner(owner) {
            return Ok(());
        }
        self.controller.blur_on_main_thread();
        self.controller.set_visible_on_main_thread(false);
        self.clear_owner(owner);
        Ok(())
    }

    pub fn drain_events(&self) -> Vec<BrowserEvent> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        state.events.drain(..).collect()
    }

    fn claim_or_check_owner(&self, owner: &str, visible: bool) -> Result<()> {
        let Ok(mut state) = self.state.lock() else {
            bail!("browser portal state mutex poisoned");
        };
        if visible {
            claim_owner(&mut state, owner);
            return Ok(());
        }
        if state.owner.as_deref() == Some(owner) || state.owner.is_none() {
            Ok(())
        } else {
            bail!("browser portal is owned by another tab");
        }
    }

    fn require_owner(&self, owner: &str) -> Result<()> {
        if self.is_owner(owner) {
            Ok(())
        } else {
            bail!("browser portal is not owned by this tab");
        }
    }

    fn is_owner(&self, owner: &str) -> bool {
        self.state
            .lock()
            .map(|state| state.owner.as_deref() == Some(owner))
            .unwrap_or(false)
    }

    fn clear_owner(&self, owner: &str) {
        if let Ok(mut state) = self.state.lock() {
            release_owner(&mut state, owner);
        }
    }
}

fn release_owner(state: &mut BrowserState, owner: &str) {
    if state.owner.as_deref() == Some(owner) {
        state.owner = None;
        state.events.clear();
    }
}

fn claim_owner(state: &mut BrowserState, owner: &str) {
    if state.owner.as_deref() != Some(owner) {
        state.events.clear();
    }
    state.owner = Some(owner.to_owned());
}

fn enqueue_event(state: &mut BrowserState, event: CefBrowserEvent) {
    let Some(owner) = state.owner.clone() else {
        return;
    };
    let event = match event {
        CefBrowserEvent::AddressChanged { url } => BrowserEvent::AddressChanged {
            owner: Some(owner),
            url,
        },
        CefBrowserEvent::TitleChanged { title, url } => BrowserEvent::TitleChanged {
            owner: Some(owner),
            title,
            url,
        },
    };
    state.events.push_back(event);
}

impl Drop for BrowserPortal {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            use std::sync::atomic::Ordering;

            self.pump_stop.store(true, Ordering::Release);
            if let Some(thread) = self.pump_thread.take() {
                let _ = thread.join();
            }
            self.controller.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    use super::BrowserPortal;
    use super::{
        claim_owner, enqueue_event, release_owner, BrowserEvent, BrowserRect, BrowserState,
    };
    use mew_cef_host::embed::BrowserEvent as CefBrowserEvent;
    use std::collections::VecDeque;
    #[cfg(not(target_os = "macos"))]
    use std::sync::Arc;

    #[test]
    fn browser_rect_never_sends_zero_sized_dimensions() {
        assert_eq!(
            BrowserRect {
                x: 2.,
                y: 3.,
                width: 0.,
                height: -1.,
                visible: true
            }
            .width
            .max(1.0),
            1.
        );
        assert_eq!(
            BrowserRect {
                x: 2.,
                y: 3.,
                width: 0.,
                height: -1.,
                visible: true
            }
            .height
            .max(1.0),
            1.
        );
    }

    #[test]
    fn unavailable_platform_does_not_claim_browser_ownership() {
        #[cfg(not(target_os = "macos"))]
        {
            let portal = BrowserPortal::initialize(0, "about:blank", Arc::new(|| {})).unwrap();
            assert!(portal.is_none());
        }
    }

    #[test]
    fn native_events_are_dropped_while_the_portal_is_unowned() {
        let mut state = BrowserState::default();

        enqueue_event(
            &mut state,
            CefBrowserEvent::AddressChanged {
                url: "https://stale.example".to_owned(),
            },
        );

        assert!(state.events.is_empty());
    }

    #[test]
    fn releasing_an_owner_discards_queued_native_events() {
        let mut state = BrowserState {
            owner: Some("tab-a".to_owned()),
            events: VecDeque::from([BrowserEvent::AddressChanged {
                owner: Some("tab-a".to_owned()),
                url: "https://stale.example".to_owned(),
            }]),
        };

        release_owner(&mut state, "tab-a");

        assert!(state.owner.is_none());
        assert!(state.events.is_empty());
    }

    #[test]
    fn claiming_a_new_owner_discards_events_left_by_the_previous_owner() {
        let mut state = BrowserState {
            owner: None,
            events: VecDeque::from([BrowserEvent::TitleChanged {
                owner: None,
                title: "stale".to_owned(),
                url: "https://stale.example".to_owned(),
            }]),
        };

        claim_owner(&mut state, "tab-b");

        assert_eq!(state.owner.as_deref(), Some("tab-b"));
        assert!(state.events.is_empty());
    }
}
