//! Retained native browser pages scoped to a confirmed connection and session.

use anyhow::{bail, Result};
use mew_cef_host::embed::{
    BrowserEvent as CefBrowserEvent, BrowserRect as CefBrowserRect, CefEmbedController,
    CefPageController,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrowserRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BrowserOwner {
    pub connection_profile: String,
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BrowserPage {
    pub owner: BrowserOwner,
    pub page_instance: String,
}

impl BrowserPage {
    pub fn new(owner: BrowserOwner) -> Self {
        Self {
            owner,
            page_instance: ulid::Ulid::new().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserReady {
    pub cdp_endpoint: String,
    pub target_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserPageState {
    pub page: BrowserPage,
    pub url: String,
    pub title: String,
    pub visible: bool,
    pub closed: bool,
    pub ready: Option<BrowserReady>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserEvent {
    Closed { page: BrowserPage },
    AddressChanged { page: BrowserPage },
    TitleChanged { page: BrowserPage },
    Ready { page: BrowserPage },
    Error { page: BrowserPage },
}

impl BrowserEvent {
    fn page(&self) -> &BrowserPage {
        match self {
            Self::AddressChanged { page, .. }
            | Self::TitleChanged { page, .. }
            | Self::Ready { page, .. }
            | Self::Error { page, .. }
            | Self::Closed { page } => page,
        }
    }
}

#[derive(Default)]
struct BrowserState {
    pages: BTreeMap<BrowserOwner, BrowserPageState>,
    events: VecDeque<BrowserEvent>,
}

impl BrowserState {
    fn insert_page(&mut self, page: BrowserPage) {
        self.pages.insert(
            page.owner.clone(),
            BrowserPageState {
                page,
                url: "about:blank".into(),
                title: String::new(),
                visible: false,
                closed: false,
                ready: None,
                error: None,
            },
        );
    }

    fn visibility_changes<'a>(
        &'a self,
        visible_owner: Option<&'a BrowserOwner>,
    ) -> impl Iterator<Item = (&'a BrowserOwner, bool)> {
        self.pages.iter().filter_map(move |(owner, state)| {
            let visible = !state.closed && visible_owner == Some(owner);
            (state.visible != visible).then_some((owner, visible))
        })
    }

    fn set_visible(&mut self, owner: &BrowserOwner, visible: bool) {
        for (page_owner, page) in &mut self.pages {
            if page_owner == owner {
                page.visible = visible && !page.closed;
            } else if visible {
                page.visible = false;
            }
        }
    }

    fn remove_page(&mut self, page: &BrowserPage) -> bool {
        if !self.is_current(page) {
            return false;
        }
        self.pages.remove(&page.owner);
        self.events.retain(|event| event.page() != page);
        true
    }

    fn set_error(&mut self, page: &BrowserPage, error: Option<String>) -> bool {
        let Some(current) = self
            .pages
            .get_mut(&page.owner)
            .filter(|current| current.page == *page && !current.closed)
        else {
            return false;
        };
        current.error = error;
        true
    }

    fn is_current(&self, page: &BrowserPage) -> bool {
        self.pages
            .get(&page.owner)
            .is_some_and(|state| state.page == *page)
    }
}

pub struct BrowserPortal {
    pages: BTreeMap<BrowserOwner, CefPageController>,
    controller: CefEmbedController,
    state: Arc<Mutex<BrowserState>>,
}

impl BrowserPortal {
    /// Initialize the process runtime once. Pages are created separately under
    /// their native parent view and remain alive when the panel is hidden.
    pub fn initialize() -> Result<Option<Self>> {
        let Some(controller) =
            CefEmbedController::try_initialize().map_err(|error| anyhow::anyhow!(error))?
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            pages: BTreeMap::new(),
            controller,
            state: Arc::new(Mutex::new(BrowserState::default())),
        }))
    }

    pub fn create_page(&mut self, parent_view: usize, page: BrowserPage) -> Result<()> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("browser state mutex poisoned"))?;
            if state.is_current(&page) {
                return Ok(());
            }
            if state.pages.contains_key(&page.owner) {
                bail!("close the previous browser page before recreating it");
            }
            state.insert_page(page.clone());
        }
        let event_state = self.state.clone();
        let event_page = page.clone();
        let callback = Arc::new(move |event: CefBrowserEvent| {
            if let Ok(mut state) = event_state.lock() {
                enqueue_event(&mut state, &event_page, event);
            }
        });
        match self
            .controller
            .create_page(parent_view, "about:blank", callback)
        {
            Ok(controller) => {
                self.pages.insert(page.owner, controller);
                Ok(())
            }
            Err(error) => {
                if let Ok(mut state) = self.state.lock() {
                    state.remove_page(&page);
                }
                Err(anyhow::anyhow!(error))
            }
        }
    }

    pub fn contains_page(&self, owner: &BrowserOwner) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.pages.contains_key(owner))
    }

    pub fn page_is_live(&self, owner: &BrowserOwner) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.pages.get(owner).is_some_and(|page| !page.closed))
    }

    pub fn page_state(&self, owner: &BrowserOwner) -> Option<BrowserPageState> {
        self.state.lock().ok()?.pages.get(owner).cloned()
    }

    pub fn page_for_instance(&self, page_instance: &str) -> Option<BrowserPage> {
        self.state
            .lock()
            .ok()?
            .pages
            .values()
            .find(|state| state.page.page_instance == page_instance && !state.closed)
            .map(|state| state.page.clone())
    }

    pub fn is_current(&self, page: &BrowserPage) -> bool {
        self.state.lock().is_ok_and(|state| state.is_current(page))
    }

    pub fn pump(&self) {
        CefEmbedController::do_message_loop_work();
    }

    pub fn shutdown(&mut self) {
        self.pages.clear();
        self.controller.shutdown();
    }

    pub fn prepare_for_process_exit(&mut self) {
        self.controller.prepare_for_process_exit();
    }

    pub fn set_rect(&self, owner: &BrowserOwner, rect: BrowserRect) -> Result<()> {
        self.set_visible(owner, rect.visible)?;
        self.require_page(owner)?
            .set_rect_on_main_thread(CefBrowserRect {
                x: rect.x,
                y: rect.y,
                width: rect.width.max(1.0),
                height: rect.height.max(1.0),
                visible: rect.visible,
            });
        Ok(())
    }

    pub fn set_visible(&self, owner: &BrowserOwner, visible: bool) -> Result<()> {
        self.require_page(owner)?;
        self.apply_visibility(visible.then_some(owner));
        Ok(())
    }

    pub fn hide_all(&self) {
        self.apply_visibility(None);
    }

    fn apply_visibility(&self, visible_owner: Option<&BrowserOwner>) {
        let changes: Vec<_> = match self.state.lock() {
            Ok(mut state) => {
                let changes = state
                    .visibility_changes(visible_owner)
                    .map(|(owner, visible)| (owner.clone(), visible))
                    .collect();
                if let Some(owner) = visible_owner {
                    state.set_visible(owner, true);
                } else {
                    for page in state.pages.values_mut() {
                        page.visible = false;
                    }
                }
                changes
            }
            Err(_) => return,
        };
        for (owner, visible) in changes {
            if let Some(page) = self.pages.get(&owner) {
                page.set_visible_on_main_thread(visible);
            }
        }
    }

    pub fn focus(&self, owner: &BrowserOwner) -> Result<bool> {
        Ok(self.require_page(owner)?.focus_on_main_thread())
    }

    pub fn blur(&self, owner: &BrowserOwner) -> Result<bool> {
        Ok(self.require_page(owner)?.blur_on_main_thread())
    }

    pub fn navigate(&self, owner: &BrowserOwner, url: &str) -> Result<()> {
        if url != "about:blank" && !(url.starts_with("http://") || url.starts_with("https://")) {
            bail!("browser navigation only supports about:blank, http, and https URLs");
        }
        self.require_page(owner)?.navigate_on_main_thread(url);
        Ok(())
    }

    pub fn close(&mut self, page: &BrowserPage) -> bool {
        if !self
            .state
            .lock()
            .is_ok_and(|mut state| state.remove_page(page))
        {
            return false;
        }
        self.pages.remove(&page.owner);
        true
    }

    pub fn set_error(&self, page: &BrowserPage, error: Option<String>) -> bool {
        self.state
            .lock()
            .is_ok_and(|mut state| state.set_error(page, error))
    }

    pub fn drain_events(&mut self) -> Vec<BrowserEvent> {
        let events: Vec<_> = self
            .state
            .lock()
            .map(|mut state| state.events.drain(..).collect())
            .unwrap_or_default();
        for event in &events {
            if let BrowserEvent::Closed { page } = event {
                if self.is_current(page) {
                    self.pages.remove(&page.owner);
                }
            }
        }
        events
    }

    fn require_page(&self, owner: &BrowserOwner) -> Result<&CefPageController> {
        self.pages
            .get(owner)
            .ok_or_else(|| anyhow::anyhow!("browser page is not available for this session"))
    }
}

fn enqueue_event(state: &mut BrowserState, page: &BrowserPage, event: CefBrowserEvent) {
    let Some(current) = state
        .pages
        .get_mut(&page.owner)
        .filter(|current| current.page == *page && !current.closed)
    else {
        return;
    };
    let event = match event {
        CefBrowserEvent::Closed => {
            current.closed = true;
            current.visible = false;
            current.ready = None;
            current.error = Some("native browser page closed; retry to create a new page".into());
            state.events.retain(|event| event.page() != page);
            BrowserEvent::Closed { page: page.clone() }
        }
        CefBrowserEvent::AddressChanged { url } => {
            current.url = url;
            BrowserEvent::AddressChanged { page: page.clone() }
        }
        CefBrowserEvent::TitleChanged { title, url } => {
            current.title = title;
            current.url = url;
            BrowserEvent::TitleChanged { page: page.clone() }
        }
        CefBrowserEvent::Ready {
            cdp_endpoint,
            target_id,
        } => {
            current.ready = Some(BrowserReady {
                cdp_endpoint,
                target_id,
            });
            BrowserEvent::Ready { page: page.clone() }
        }
        CefBrowserEvent::Error { message } => {
            current.error = Some(message);
            BrowserEvent::Error { page: page.clone() }
        }
    };
    state.events.push_back(event);
}

impl Drop for BrowserPortal {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
