//! Query routing restricted to the main document of the local app page.

use cef::wrapper::message_router::BrowserSideCallback;
#[cfg(target_os = "macos")]
use cef::wrapper::message_router::MessageRouterConfig;
use std::sync::{Arc, Mutex};

pub type AppRequestHandler = Arc<dyn Fn(&str, AppResponse) + Send + Sync + 'static>;

/// A single asynchronous reply. Dropping the last unanswered reply fails it.
/// Navigation and page teardown disarm replies retained by the application.
#[derive(Clone)]
pub struct AppResponse {
    inner: Arc<QueryResponse>,
}

pub(crate) struct QueryResponse {
    callback: Mutex<Option<Arc<Mutex<dyn BrowserSideCallback>>>>,
}

impl AppResponse {
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn new(callback: Arc<Mutex<dyn BrowserSideCallback>>) -> Self {
        Self {
            inner: Arc::new(QueryResponse {
                callback: Mutex::new(Some(callback)),
            }),
        }
    }

    pub fn success(&self, response: &str) {
        if let Some(callback) = self.inner.take()
            && let Ok(callback) = callback.lock()
        {
            callback.success_str(response);
        }
    }

    pub fn failure(&self, code: i32, message: &str) {
        if let Some(callback) = self.inner.take()
            && let Ok(callback) = callback.lock()
        {
            callback.failure(code, message);
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn downgrade(&self) -> std::sync::Weak<QueryResponse> {
        Arc::downgrade(&self.inner)
    }

    #[cfg(test)]
    fn cancel(&self) {
        self.inner.cancel();
    }
}

impl QueryResponse {
    fn take(&self) -> Option<Arc<Mutex<dyn BrowserSideCallback>>> {
        self.callback.lock().ok()?.take()
    }

    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn cancel(&self) {
        self.take();
    }
}

impl Drop for QueryResponse {
    fn drop(&mut self) {
        if let Some(callback) = self.take()
            && let Ok(callback) = callback.lock()
        {
            callback.failure(-1, "app request was not handled");
        }
    }
}

#[derive(Clone, Debug)]
#[cfg(any(target_os = "macos", test))]
pub(crate) struct TrustedAppOrigin {
    port: u16,
}

#[cfg(any(target_os = "macos", test))]
impl TrustedAppOrigin {
    pub(crate) fn new(url: &str) -> Result<Self, String> {
        Self::port(url)
            .map(|port| Self { port })
            .ok_or_else(|| "app page requires an explicit http://127.0.0.1:<port> origin".into())
    }

    fn port(url: &str) -> Option<u16> {
        if url
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\')
        {
            return None;
        }
        let authority = url
            .strip_prefix("http://127.0.0.1:")?
            .split(['/', '?', '#'])
            .next()?;
        if !authority.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        authority.parse::<u16>().ok().filter(|port| *port != 0)
    }

    pub(crate) fn allows_navigation(&self, url: &str) -> bool {
        Self::port(url) == Some(self.port)
    }

    pub(crate) fn allows_frame(
        &self,
        expected_browser: i32,
        actual_browser: i32,
        is_main: bool,
        url: &str,
    ) -> bool {
        expected_browser > 0
            && expected_browser == actual_browser
            && is_main
            && self.allows_navigation(url)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn router_config() -> MessageRouterConfig {
    MessageRouterConfig {
        js_query_function: "mewQuery".into(),
        js_cancel_function: "mewQueryCancel".into(),
        ..Default::default()
    }
}

#[cfg(target_os = "macos")]
pub(crate) const APP_URL_KEY: &str = "mew_app_url";

/// Called only after libcef has been loaded in a helper process.
#[cfg(target_os = "macos")]
pub fn render_process_handler() -> cef::RenderProcessHandler {
    use cef::wrapper::message_router::{MessageRouterRendererSide, RendererSideRouter};
    MewAppRenderHandler::new(
        RendererSideRouter::new(router_config()),
        Arc::new(Mutex::new(std::collections::HashMap::new())),
    )
}

#[cfg(target_os = "macos")]
use cef::*;

#[cfg(target_os = "macos")]
use cef::wrapper::message_router::{MessageRouterRendererSideHandlerCallbacks, RendererSideRouter};

#[cfg(target_os = "macos")]
wrap_render_process_handler! {
    struct MewAppRenderHandler {
        router: Arc<RendererSideRouter>,
        origins: Arc<Mutex<std::collections::HashMap<i32, TrustedAppOrigin>>>,
    }

    impl RenderProcessHandler {
        fn on_browser_created(&self, browser: Option<&mut Browser>, extra_info: Option<&mut DictionaryValue>) {
            let (Some(browser), Some(extra_info)) = (browser, extra_info) else { return };
            let url = CefString::from(&extra_info.string(Some(&CefString::from(APP_URL_KEY)))).to_string();
            if let Ok(origin) = TrustedAppOrigin::new(&url)
                && let Ok(mut origins) = self.origins.lock()
            {
                origins.insert(browser.identifier(), origin);
            }
        }

        fn on_browser_destroyed(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser
                && let Ok(mut origins) = self.origins.lock()
            {
                origins.remove(&browser.identifier());
            }
        }

        fn on_context_created(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            let (Some(browser), Some(frame)) = (browser, frame) else { return };
            let allowed = self.origins.lock().ok().and_then(|origins| {
                origins.get(&browser.identifier()).map(|origin| origin.allows_frame(
                    browser.identifier(), browser.identifier(), frame.is_main() != 0,
                    &CefString::from(&frame.url()).to_string(),
                ))
            }).unwrap_or(false);
            if allowed {
                self.router.on_context_created(Some(browser.clone()), Some(frame.clone()), context.cloned());
            }
        }

        fn on_context_released(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            // Context release still cancels requests after the frame URL changes.
            if browser.as_ref().is_some_and(|browser| self.origins.lock().map(|origins| origins.contains_key(&browser.identifier())).unwrap_or(false))
                && frame.as_ref().is_some_and(|frame| frame.is_main() != 0)
            {
                self.router.on_context_released(browser.cloned(), frame.cloned(), context.cloned());
            }
        }

        fn on_process_message_received(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, source_process: ProcessId, message: Option<&mut ProcessMessage>) -> i32 {
            if source_process != ProcessId::BROWSER
                || !browser.as_ref().is_some_and(|browser| self.origins.lock().map(|origins| origins.contains_key(&browser.identifier())).unwrap_or(false))
                || !frame.as_ref().is_some_and(|frame| frame.is_main() != 0)
            {
                return 0;
            }
            i32::from(self.router.on_process_message_received(browser.cloned(), frame.cloned(), Some(source_process), message.cloned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cef::wrapper::message_router::BrowserSideCallback;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Replies(Mutex<Vec<Result<String, (i32, String)>>>);

    impl BrowserSideCallback for Replies {
        fn success_str(&self, response: &str) {
            self.0.lock().unwrap().push(Ok(response.to_owned()));
        }
        fn success_binary(&self, _data: &[u8]) {
            panic!("app bridge only sends text");
        }
        fn failure(&self, code: i32, message: &str) {
            self.0.lock().unwrap().push(Err((code, message.to_owned())));
        }
    }

    #[test]
    fn app_origin_accepts_only_explicit_loopback_http_ports() {
        for url in [
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8080/index.html?view=chat#main",
        ] {
            let origin = TrustedAppOrigin::new(url).unwrap();
            assert!(origin.allows_navigation("http://127.0.0.1:8080/chat/42"));
            assert!(origin.allows_navigation("http://127.0.0.1:8080/?theme=dark"));
        }
        for url in [
            "https://127.0.0.1:8080/",
            "http://localhost:8080/",
            "http://127.0.0.1/",
            "http://127.0.0.1:0/",
            "http://127.0.0.1:65536/",
            "http://127.0.0.1:8080@evil.test/",
            "http://127.0.0.1:8080.evil.test/",
            "http://127.0.0.1:8080\\@evil.test/",
            "file:///Applications/mew.app/index.html",
        ] {
            assert!(TrustedAppOrigin::new(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn queries_require_the_same_browser_main_frame_and_origin() {
        let origin = TrustedAppOrigin::new("http://127.0.0.1:8080/").unwrap();
        assert!(origin.allows_frame(4, 4, true, "http://127.0.0.1:8080/"));
        for (browser, main, url) in [
            (5, true, "http://127.0.0.1:8080/"),
            (4, false, "http://127.0.0.1:8080/"),
            (4, true, "http://127.0.0.1:8081/"),
            (4, true, "http://127.0.0.1:80800/"),
            (4, true, "https://example.com/"),
            (4, true, "about:blank"),
            (4, true, "data:text/html,app"),
        ] {
            assert!(!origin.allows_frame(4, browser, main, url));
        }
        assert!(!origin.allows_frame(0, 0, true, "http://127.0.0.1:8080/"));
    }

    #[test]
    fn app_response_completes_once_and_teardown_disarms_late_replies() {
        let replies = Arc::new(Mutex::new(Replies::default()));
        let response = AppResponse::new(replies.clone());
        response.success("ready");
        response.failure(2, "late");
        drop(response);
        assert_eq!(
            *replies.lock().unwrap().0.lock().unwrap(),
            [Ok("ready".into())]
        );

        let response = AppResponse::new(replies.clone());
        response.cancel();
        response.success("stale");
        response.failure(3, "stale");
        drop(response);
        assert_eq!(replies.lock().unwrap().0.lock().unwrap().len(), 1);
    }

    #[test]
    fn dropping_an_unanswered_request_fails_instead_of_leaking_a_query() {
        let replies = Arc::new(Mutex::new(Replies::default()));
        let response = AppResponse::new(replies.clone());
        let retained = response.clone();
        drop(response);
        assert!(replies.lock().unwrap().0.lock().unwrap().is_empty());
        drop(retained);
        assert!(matches!(
            replies.lock().unwrap().0.lock().unwrap().as_slice(),
            [Err((-1, _))]
        ));
    }

    #[test]
    fn failed_requests_do_not_accept_later_success() {
        let replies = Arc::new(Mutex::new(Replies::default()));
        let response = AppResponse::new(replies.clone());
        response.failure(12, "invalid request");
        response.success("late");
        drop(response);
        assert_eq!(
            *replies.lock().unwrap().0.lock().unwrap(),
            [Err((12, "invalid request".into()))]
        );
    }
}
