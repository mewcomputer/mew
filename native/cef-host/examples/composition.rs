//! Isolated CPU OSR composition experiment. No daemon or user configuration.

use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
struct PixelFrame {
    width: i32,
    height: i32,
    sequence: u64,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct Frames {
    latest: Option<Arc<PixelFrame>>,
    base: Option<Arc<PixelFrame>>,
    bytes: u64,
    http_bytes: u64,
    http_frames: u64,
}

fn frame_len(width: i32, height: i32) -> Option<usize> {
    if !(1..=4096).contains(&width) || !(1..=4096).contains(&height) {
        return None;
    }
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)
}

impl Frames {
    fn publish(&mut self, width: i32, height: i32, bytes: &[u8]) -> bool {
        if frame_len(width, height) != Some(bytes.len()) {
            return false;
        }
        self.bytes += bytes.len() as u64;
        self.latest = Some(Arc::new(PixelFrame {
            width,
            height,
            bytes: bytes.to_vec(),
            sequence: self.latest.as_ref().map_or(1, |frame| frame.sequence + 1),
        }));
        self.base = self.latest.clone();
        true
    }

    fn popup(&mut self, x: i32, y: i32, width: i32, height: i32, bytes: &[u8]) -> bool {
        if frame_len(width, height) != Some(bytes.len()) {
            return false;
        }
        let Some(base) = self.base.as_ref() else {
            return false;
        };
        let mut frame = (**base).clone();
        frame.sequence = self.latest.as_ref().unwrap().sequence + 1;
        for row in 0..height {
            for col in 0..width {
                let (target_x, target_y) = (x + col, y + row);
                if target_x < 0
                    || target_y < 0
                    || target_x >= frame.width
                    || target_y >= frame.height
                {
                    continue;
                }
                let source = ((row * width + col) * 4) as usize;
                let target = ((target_y * frame.width + target_x) * 4) as usize;
                let alpha = u16::from(bytes[source + 3]);
                for channel in 0..4 {
                    frame.bytes[target + channel] = (u16::from(bytes[source + channel])
                        + u16::from(frame.bytes[target + channel]) * (255 - alpha) / 255)
                        .min(255) as u8;
                }
            }
        }
        self.latest = Some(Arc::new(frame));
        self.bytes += self.latest.as_ref().unwrap().bytes.len() as u64;
        true
    }
}

#[derive(Clone, Copy)]
struct Viewport {
    width: i32,
    height: i32,
    scale: f32,
}

impl Viewport {
    fn from_request(request: &serde_json::Value) -> Result<Self, String> {
        let width = request["width"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or("width must be an integer")?;
        let height = request["height"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or("height must be an integer")?;
        let scale = request
            .get("deviceScaleFactor")
            .map_or(Some(1.), serde_json::Value::as_f64)
            .ok_or("invalid scale")?;
        if !(1..=2048).contains(&width)
            || !(1..=2048).contains(&height)
            || !scale.is_finite()
            || !(1. ..=3.).contains(&scale)
            || frame_len(
                (f64::from(width) * scale).ceil() as i32,
                (f64::from(height) * scale).ceil() as i32,
            )
            .is_none()
        {
            return Err("viewport exceeds the experiment's 4096px physical bounds".into());
        }
        Ok(Self {
            width,
            height,
            scale: scale as f32,
        })
    }
}

fn asset_path(target: &str) -> Option<PathBuf> {
    let path = target.split('?').next()?.strip_prefix('/')?;
    if path.is_empty() {
        return Some("index.html".into());
    }
    if path.contains(['%', '\\'])
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(path.into())
}

fn frame_response(frame: &PixelFrame, etag: Option<&str>) -> (u16, String) {
    let tag = format!("\"{}\"", frame.sequence);
    let status = if etag == Some(tag.as_str()) { 304 } else { 200 };
    (
        status,
        format!(
            "Content-Type: application/octet-stream\r\nETag: {tag}\r\nX-Frame-Width: {}\r\nX-Frame-Height: {}\r\nX-Frame-Sequence: {}\r\n",
            frame.width, frame.height, frame.sequence
        ),
    )
}

fn sdk_versions() -> serde_json::Value {
    serde_json::json!({
        "cef": format!("{}.{}.{}", cef::sys::CEF_VERSION_MAJOR, cef::sys::CEF_VERSION_MINOR, cef::sys::CEF_VERSION_PATCH),
        "chromium": format!("{}.{}.{}.{}", cef::sys::CHROME_VERSION_MAJOR, cef::sys::CHROME_VERSION_MINOR, cef::sys::CHROME_VERSION_BUILD, cef::sys::CHROME_VERSION_PATCH),
    })
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use cef::{wrapper::message_router::*, *};
    use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, sel};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSView,
        NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{
        NSDate, NSDefaultRunLoopMode, NSObjectNSDelayedPerforming, NSPoint, NSRect, NSRunLoop,
        NSSize, NSString,
    };
    use std::{
        cell::RefCell,
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
        rc::Rc,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicI32, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    type Reply = Arc<Mutex<dyn BrowserSideCallback>>;
    type Requests = mpsc::Sender<(String, Reply)>;
    type Browsers = Rc<RefCell<Vec<Browser>>>;

    #[derive(Default)]
    struct Popup {
        visible: bool,
        rect: Rect,
        pixels: Option<PixelFrame>,
    }

    #[derive(Clone)]
    struct BrowserContext {
        browsers: Browsers,
        app_id: Arc<AtomicI32>,
        router: Arc<BrowserSideRouter>,
        app_url: String,
        frames: Arc<Mutex<Frames>>,
        viewport: Arc<Mutex<Viewport>>,
        popup: Rc<RefCell<Popup>>,
    }

    fn server(
        assets: PathBuf,
        frames: Arc<Mutex<Frames>>,
        stop: Arc<AtomicBool>,
    ) -> Result<String, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let origin = format!(
            "http://{}",
            listener.local_addr().map_err(|error| error.to_string())?
        );
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = serve(stream, &assets, &frames);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(origin)
    }

    fn serve(
        mut stream: TcpStream,
        assets: &std::path::Path,
        frames: &Mutex<Frames>,
    ) -> std::io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut reader = BufReader::new(&mut stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let mut request = line.split_whitespace();
        let method = request.next().unwrap_or("");
        let target = request.next().unwrap_or("").to_owned();
        if method != "GET" {
            return Ok(());
        }
        let mut etag = None;
        let mut total = line.len();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 || line == "\r\n" {
                break;
            }
            total += line.len();
            if total > 8192 {
                return Ok(());
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("if-none-match")
            {
                etag = Some(value.trim().to_owned());
            }
        }
        drop(reader);
        if target.split('?').next() == Some("/frame") {
            let frame = frames.lock().unwrap().latest.clone();
            if let Some(frame) = frame {
                let (status, headers) = frame_response(&frame, etag.as_deref());
                let result = response(
                    &mut stream,
                    status,
                    &headers,
                    if status == 200 { &frame.bytes } else { &[] },
                );
                if result.is_ok() && status == 200 {
                    let mut frames = frames.lock().unwrap();
                    frames.http_bytes += frame.bytes.len() as u64;
                    frames.http_frames += 1;
                }
                return result;
            }
            return response(&mut stream, 204, "", &[]);
        }
        let path = asset_path(&target)
            .and_then(|path| assets.join(path).canonicalize().ok())
            .filter(|path| path.starts_with(assets));
        let Some(path) = path else {
            return response(&mut stream, 404, "", &[]);
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return response(&mut stream, 404, "", &[]);
        };
        let kind = match path.extension().and_then(|ext| ext.to_str()) {
            Some("html") => "text/html",
            Some("js") => "text/javascript",
            Some("css") => "text/css",
            Some("svg") => "image/svg+xml",
            _ => "application/octet-stream",
        };
        let mut headers = format!("Content-Type: {kind}\r\n");
        if path.file_name().is_some_and(|name| name == "fixture.html") {
            headers.push_str(
                "Content-Security-Policy: frame-ancestors 'none'\r\nX-Frame-Options: DENY\r\n",
            );
        }
        response(&mut stream, 200, &headers, &bytes)
    }

    fn response(
        stream: &mut TcpStream,
        status: u16,
        headers: &str,
        bytes: &[u8],
    ) -> std::io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 {status} {}\r\n{headers}Content-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            match status {
                200 => "OK",
                204 => "No Content",
                304 => "Not Modified",
                _ => "Not Found",
            },
            bytes.len()
        )?;
        stream.write_all(bytes)
    }

    struct Queries {
        app_id: Arc<AtomicI32>,
        app_url: String,
        requests: Requests,
    }
    impl BrowserSideHandler for Queries {
        fn on_query_str(
            &self,
            browser: Option<Browser>,
            frame: Option<Frame>,
            _query_id: i64,
            request: &str,
            persistent: bool,
            callback: Reply,
        ) -> bool {
            if persistent
                || request.len() > 16_384
                || !browser.zip(frame).is_some_and(|(browser, frame)| {
                    browser.identifier() == self.app_id.load(Ordering::Acquire)
                        && frame.is_main() != 0
                        && CefString::from(&frame.url()).to_string() == self.app_url
                })
            {
                return false;
            }
            if self
                .requests
                .send((request.to_owned(), callback.clone()))
                .is_err()
            {
                callback.lock().unwrap().failure(-1, "experiment closed");
            }
            true
        }
    }

    wrap_app! {
        struct ExperimentApp { renderer: RenderProcessHandler, ready: Arc<AtomicBool>, router: Arc<BrowserSideRouter>, queries: Arc<Queries> }
        impl App {
            fn on_before_command_line_processing(&self, _process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
                if let Some(command_line) = command_line
                    && std::env::var("MEW_CEF_USE_SYSTEM_KEYCHAIN").as_deref() != Ok("1")
                {
                    command_line.append_switch(Some(&CefString::from("use-mock-keychain")));
                }
            }
            fn render_process_handler(&self) -> Option<RenderProcessHandler> { Some(self.renderer.clone()) }
            fn browser_process_handler(&self) -> Option<BrowserProcessHandler> { Some(ExperimentProcess::new(self.ready.clone(), self.router.clone(), self.queries.clone())) }
        }
    }
    wrap_browser_process_handler! {
        struct ExperimentProcess { ready: Arc<AtomicBool>, router: Arc<BrowserSideRouter>, queries: Arc<Queries> }
        impl BrowserProcessHandler {
            fn on_context_initialized(&self) {
                self.router.add_handler(self.queries.clone(), true).expect("cannot attach app query handler on the CEF UI thread");
                self.ready.store(true, Ordering::Release);
            }
        }
    }
    wrap_client! {
        struct ExperimentClient { context: BrowserContext, is_app: bool }
        impl Client {
            fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(ExperimentLife::new(self.context.browsers.clone(), self.context.app_id.clone(), self.context.router.clone(), self.is_app)) }
            fn render_handler(&self) -> Option<RenderHandler> { if self.is_app { None } else { Some(ExperimentRender::new(self.context.frames.clone(), self.context.viewport.clone(), self.context.popup.clone())) } }
            fn request_handler(&self) -> Option<RequestHandler> { Some(ExperimentRequest::new(self.context.router.clone(), self.context.app_url.clone(), self.is_app)) }
            fn load_handler(&self) -> Option<LoadHandler> { Some(ExperimentLoad::new(self.is_app)) }
            fn display_handler(&self) -> Option<DisplayHandler> { Some(ExperimentDisplay::new(self.is_app)) }
            fn on_process_message_received(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, source: ProcessId, message: Option<&mut ProcessMessage>) -> i32 {
                if self.is_app && source == ProcessId::RENDERER && frame.as_ref().is_some_and(|frame| frame.is_main() != 0 && CefString::from(&frame.url()).to_string() == self.context.app_url) {
                    return i32::from(self.context.router.on_process_message_received(browser.cloned(), frame.cloned(), source, message.cloned()));
                }
                0
            }
        }
    }
    wrap_life_span_handler! {
        struct ExperimentLife { browsers: Browsers, app_id: Arc<AtomicI32>, router: Arc<BrowserSideRouter>, is_app: bool }
        impl LifeSpanHandler {
            fn on_after_created(&self, browser: Option<&mut Browser>) {
                if let Some(browser) = browser { if self.is_app { self.app_id.store(browser.identifier(), Ordering::Release); if let Some(host) = browser.host() { host.set_accessibility_state(State::ENABLED); } } self.browsers.borrow_mut().push(browser.clone()); }
            }
            fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
                if self.is_app && let Some(host) = browser.and_then(|browser| browser.host()) {
                    let handle = host.window_handle();
                    if !handle.is_null() {
                        let view = unsafe { &*handle.cast::<NSView>() };
                        unsafe { view.performSelector_withObject_afterDelay(sel!(removeFromSuperview), None, 0.); }
                    }
                }
                1
            }
            fn on_before_close(&self, browser: Option<&mut Browser>) { if let Some(browser) = browser { if self.is_app { self.router.on_before_close(Some(browser.clone())); } self.browsers.borrow_mut().retain(|item| item.identifier() != browser.identifier()); } }
            fn on_before_popup(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _id: i32, _url: Option<&CefString>, _name: Option<&CefString>, _disposition: WindowOpenDisposition, _gesture: i32, _features: Option<&PopupFeatures>, _info: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>, _extra: Option<&mut Option<DictionaryValue>>, _no_js: Option<&mut i32>) -> i32 { 1 }
        }
    }
    wrap_request_handler! {
        struct ExperimentRequest { router: Arc<BrowserSideRouter>, app_url: String, is_app: bool }
        impl RequestHandler {
            fn on_before_browse(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>, _gesture: i32, _redirect: i32) -> i32 {
                if !self.is_app { return 0; }
                if !request.as_ref().is_some_and(|request| CefString::from(&request.url()).to_string() == self.app_url) { return 1; }
                self.router.on_before_browse(browser.cloned(), frame.cloned()); 0
            }
        }
    }
    wrap_load_handler! {
        struct ExperimentLoad { is_app: bool }
        impl LoadHandler {
            fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, status: i32) { eprintln!("composition: load_end app={} status={} url={:?}", self.is_app, status, frame.map(|frame| CefString::from(&frame.url()).to_string())); }
            fn on_load_error(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, error: Errorcode, text: Option<&CefString>, url: Option<&CefString>) { eprintln!("composition: load_error app={} code={:?} text={:?} url={:?}", self.is_app, error, text.map(ToString::to_string), url.map(ToString::to_string)); }
        }
    }
    wrap_display_handler! {
        struct ExperimentDisplay { is_app: bool }
        impl DisplayHandler {
            fn on_console_message(&self, _browser: Option<&mut Browser>, level: LogSeverity, message: Option<&CefString>, _source: Option<&CefString>, _line: i32) -> i32 { eprintln!("composition: console app={} level={:?} {}", self.is_app, level, message.map(ToString::to_string).unwrap_or_default()); 0 }
        }
    }
    wrap_render_handler! {
        struct ExperimentRender { frames: Arc<Mutex<Frames>>, viewport: Arc<Mutex<Viewport>>, popup: Rc<RefCell<Popup>> }
        impl RenderHandler {
            fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) { if let Some(rect) = rect { let view = *self.viewport.lock().unwrap(); *rect = Rect { x:0, y:0, width:view.width, height:view.height }; } }
            fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> i32 { if let Some(info) = info { let view = *self.viewport.lock().unwrap(); info.device_scale_factor = view.scale; info.depth = 32; info.depth_per_component = 8; info.rect = Rect { x:0, y:0, width:view.width, height:view.height }; info.available_rect = info.rect.clone(); return 1; } 0 }
            fn on_popup_show(&self, browser: Option<&mut Browser>, show: i32) {
                self.popup.borrow_mut().visible = show != 0;
                if show == 0 {
                    self.popup.borrow_mut().pixels = None;
                    let mut frames = self.frames.lock().unwrap();
                    if let Some(base) = &frames.base {
                        let mut restored = (**base).clone();
                        restored.sequence = frames.latest.as_ref().unwrap().sequence + 1;
                        frames.bytes += restored.bytes.len() as u64;
                        frames.latest = Some(Arc::new(restored));
                    }
                    if let Some(host) = browser.and_then(|browser| browser.host()) { host.invalidate(PaintElementType::VIEW); }
                }
            }
            fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) { if let Some(rect) = rect { self.popup.borrow_mut().rect = rect.clone(); } }
            fn on_paint(&self, _browser: Option<&mut Browser>, kind: PaintElementType, _dirty: Option<&[Rect]>, buffer: *const u8, width: i32, height: i32) {
                let Some(length) = frame_len(width, height) else { return };
                if buffer.is_null() { return; }
                let bytes = unsafe { std::slice::from_raw_parts(buffer, length) };
                let mut frames = self.frames.lock().unwrap();
                if frames.latest.is_none() { eprintln!("composition: first OSR paint {}x{}", width, height); }
                let mut popup = self.popup.borrow_mut();
                if kind == PaintElementType::VIEW { frames.publish(width, height, bytes); }
                else { popup.pixels = Some(PixelFrame { width, height, bytes:bytes.to_vec(), sequence:0 }); }
                if popup.visible && let Some(pixels) = &popup.pixels { let scale = self.viewport.lock().unwrap().scale; frames.popup((popup.rect.x as f32 * scale).round() as i32, (popup.rect.y as f32 * scale).round() as i32, pixels.width, pixels.height, &pixels.bytes); }
            }
        }
    }

    fn modifiers(event: &serde_json::Value) -> u32 {
        use cef::sys::cef_event_flags_t as Flags;
        [
            ("shiftKey", Flags::EVENTFLAG_SHIFT_DOWN),
            ("ctrlKey", Flags::EVENTFLAG_CONTROL_DOWN),
            ("altKey", Flags::EVENTFLAG_ALT_DOWN),
            ("metaKey", Flags::EVENTFLAG_COMMAND_DOWN),
        ]
        .into_iter()
        .filter(|(key, _)| event[*key].as_bool() == Some(true))
        .fold(0, |flags, (_, flag)| flags | flag.0)
    }

    fn input(browser: &Browser, event: &serde_json::Value) -> Result<(), String> {
        let host = browser.host().ok_or("external browser has closed")?;
        let number = |name: &str| {
            event[name]
                .as_f64()
                .filter(|value| value.is_finite() && value.abs() <= 1_000_000.)
                .map(|value| value.round() as i32)
                .ok_or_else(|| format!("invalid {name}"))
        };
        let mouse = || -> Result<MouseEvent, String> {
            Ok(MouseEvent {
                x: number("x")?,
                y: number("y")?,
                modifiers: modifiers(event),
            })
        };
        match event["kind"].as_str().unwrap_or("") {
            "focus" => host.set_focus(i32::from(
                event["focused"].as_bool().ok_or("invalid focused state")?,
            )),
            "pointer" => {
                let mut point = mouse()?;
                let button = match event["button"].as_u64().unwrap_or(0) {
                    0 => MouseButtonType::LEFT,
                    1 => MouseButtonType::MIDDLE,
                    2 => MouseButtonType::RIGHT,
                    _ => return Err("invalid mouse button".into()),
                };
                if event["buttons"].as_u64().unwrap_or(0) & 1 != 0 {
                    point.modifiers |= cef::sys::cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON.0;
                }
                match event["phase"].as_str().unwrap_or("") {
                    "move" | "leave" => host
                        .send_mouse_move_event(Some(&point), i32::from(event["phase"] == "leave")),
                    "down" | "up" => {
                        host.set_focus(1);
                        host.send_mouse_click_event(
                            Some(&point),
                            button,
                            i32::from(event["phase"] == "up"),
                            event["clickCount"].as_i64().unwrap_or(1).clamp(1, 3) as i32,
                        );
                    }
                    _ => return Err("invalid pointer phase".into()),
                }
            }
            "wheel" => {
                host.send_mouse_wheel_event(Some(&mouse()?), -number("deltaX")?, -number("deltaY")?)
            }
            "text" => {
                let text = event["text"]
                    .as_str()
                    .filter(|text| text.len() <= 4096)
                    .ok_or("invalid committed text")?;
                host.ime_commit_text(
                    Some(&CefString::from(text)),
                    Some(&Range {
                        from: u32::MAX,
                        to: u32::MAX,
                    }),
                    0,
                );
            }
            "key" => {
                let down = match event["phase"].as_str() {
                    Some("down") => true,
                    Some("up") => false,
                    _ => return Err("invalid key phase".into()),
                };
                let key = event["key"].as_str().ok_or("missing key")?;
                if down
                    && event["metaKey"].as_bool() == Some(true)
                    && let Some(frame) = browser.focused_frame().or_else(|| browser.main_frame())
                {
                    match key.to_ascii_lowercase().as_str() {
                        "a" => {
                            frame.select_all();
                            return Ok(());
                        }
                        "c" => {
                            frame.copy();
                            return Ok(());
                        }
                        "v" => {
                            frame.paste();
                            return Ok(());
                        }
                        "x" => {
                            frame.cut();
                            return Ok(());
                        }
                        "z" => {
                            if event["shiftKey"].as_bool() == Some(true) {
                                frame.redo();
                            } else {
                                frame.undo();
                            }
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                let (native, character) = match event["code"].as_str().unwrap_or("") {
                    "Backspace" => (51, 0x7f),
                    "Delete" => (117, 0xf728),
                    "Enter" => (36, 13),
                    "Tab" => (48, 9),
                    "Escape" => (53, 27),
                    "ArrowLeft" => (123, 0xf702),
                    "ArrowRight" => (124, 0xf703),
                    "ArrowDown" => (125, 0xf701),
                    "ArrowUp" => (126, 0xf700),
                    "Home" => (115, 0xf729),
                    "End" => (119, 0xf72b),
                    "PageUp" => (116, 0xf72c),
                    "PageDown" => (121, 0xf72d),
                    _ if key.chars().count() == 1 => return Ok(()),
                    _ => return Ok(()),
                };
                host.send_key_event(Some(&KeyEvent {
                    type_: if down {
                        KeyEventType::RAWKEYDOWN
                    } else {
                        KeyEventType::KEYUP
                    },
                    modifiers: modifiers(event),
                    windows_key_code: event["keyCode"].as_i64().unwrap_or(0) as i32,
                    native_key_code: native,
                    character,
                    unmodified_character: character,
                    ..Default::default()
                }));
            }
            _ => {
                return Err(
                    "unsupported input; native IME candidate geometry is not implemented".into(),
                );
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Read;

        #[test]
        #[ignore = "requires MEW_CEF_FRAMEWORK_PATH pointing to the native CEF runtime"]
        fn browser_process_uses_the_production_keychain_policy() {
            use std::os::unix::ffi::OsStrExt;
            let framework = std::env::var_os("MEW_CEF_FRAMEWORK_PATH").unwrap();
            let framework = std::ffi::CString::new(framework.as_bytes()).unwrap();
            assert_eq!(
                unsafe { cef::load_library(Some(&*framework.as_ptr().cast())) },
                1
            );
            let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
            let (requests, _) = mpsc::channel();
            let app = ExperimentApp::new(
                mew_cef_host::app_bridge::render_process_handler(),
                Arc::new(AtomicBool::new(false)),
                BrowserSideRouter::new(MessageRouterConfig::default()),
                Arc::new(Queries {
                    app_id: Arc::new(AtomicI32::new(0)),
                    app_url: "http://127.0.0.1:1234/".into(),
                    requests,
                }),
            );
            let mut command_line = command_line_create().unwrap();
            let switch = CefString::from("use-mock-keychain");
            assert_eq!(command_line.has_switch(Some(&switch)), 0);
            app.on_before_command_line_processing(None, Some(&mut command_line));
            assert_eq!(
                command_line.has_switch(Some(&switch)) != 0,
                std::env::var("MEW_CEF_USE_SYSTEM_KEYCHAIN").as_deref() != Ok("1")
            );
            app.on_before_command_line_processing(None, None);
        }

        #[test]
        fn cef_render_callbacks_publish_popup_pixels_and_restore_the_view() {
            let frames = Arc::new(Mutex::new(Frames::default()));
            let viewport = Arc::new(Mutex::new(Viewport {
                width: 2,
                height: 1,
                scale: 2.,
            }));
            let handler = ExperimentRender::new(
                frames.clone(),
                viewport,
                Rc::new(RefCell::new(Popup::default())),
            );
            let raw = handler.get_raw();
            let mut rect: cef::sys::cef_rect_t = Rect::default().into();
            unsafe {
                (*raw).get_view_rect.unwrap()(raw, std::ptr::null_mut(), &mut rect);
            }
            assert_eq!((rect.width, rect.height), (2, 1));
            let mut info: cef::sys::cef_screen_info_t = ScreenInfo::default().into();
            assert_eq!(
                unsafe { (*raw).get_screen_info.unwrap()(raw, std::ptr::null_mut(), &mut info) },
                1
            );
            assert_eq!(info.device_scale_factor, 2.);
            let base = [20, 40, 60, 255, 5, 6, 7, 255];
            handler.on_paint(None, PaintElementType::VIEW, None, base.as_ptr(), 2, 1);
            assert_eq!(frames.lock().unwrap().latest.as_ref().unwrap().bytes, base);
            handler.on_popup_size(
                None,
                Some(&Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                }),
            );
            handler.on_popup_show(None, 1);
            handler.on_paint(
                None,
                PaintElementType::POPUP,
                None,
                [1, 2, 3, 255].as_ptr(),
                1,
                1,
            );
            assert_eq!(
                frames.lock().unwrap().latest.as_ref().unwrap().bytes[..4],
                [1, 2, 3, 255]
            );
            handler.on_popup_show(None, 0);
            assert_eq!(frames.lock().unwrap().latest.as_ref().unwrap().bytes, base);
            assert_eq!(frames.lock().unwrap().latest.as_ref().unwrap().sequence, 3);
            assert_eq!(frames.lock().unwrap().bytes, 24);
        }

        #[test]
        fn real_http_serves_owned_frames_etags_and_a_non_embeddable_fixture() {
            let assets =
                std::env::temp_dir().join(format!("mew-composition-http-{}", std::process::id()));
            std::fs::create_dir_all(&assets).unwrap();
            let assets = assets.canonicalize().unwrap();
            std::fs::write(assets.join("fixture.html"), "<input value='real fixture'>").unwrap();
            let frames = Arc::new(Mutex::new(Frames::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let origin = server(assets.clone(), frames.clone(), stop.clone()).unwrap();
            let get = |path: &str, extra: &str| {
                let mut stream = TcpStream::connect(origin.trim_start_matches("http://")).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                write!(
                    stream,
                    "GET {path} HTTP/1.1\r\nHost: localhost\r\n{extra}\r\n"
                )
                .unwrap();
                let mut response = Vec::new();
                stream.read_to_end(&mut response).unwrap();
                response
            };
            assert!(get("/frame", "").starts_with(b"HTTP/1.1 204 "));
            frames.lock().unwrap().publish(1, 1, &[1, 2, 3, 255]);
            let frame = get("/frame", "");
            assert!(frame.starts_with(b"HTTP/1.1 200 "));
            assert!(frame.ends_with(&[1, 2, 3, 255]));
            assert!(get("/frame", "If-None-Match: \"1\"\r\n").starts_with(b"HTTP/1.1 304 "));
            assert_eq!(frames.lock().unwrap().http_bytes, 4);
            assert_eq!(frames.lock().unwrap().http_frames, 1);
            let fixture = String::from_utf8(get("/fixture.html", "")).unwrap();
            assert!(fixture.contains("Content-Security-Policy: frame-ancestors 'none'\r\n"));
            assert!(fixture.ends_with("<input value='real fixture'>"));
            assert!(get("/../fixture.html", "").starts_with(b"HTTP/1.1 404 "));
            stop.store(true, Ordering::Release);
            std::fs::remove_dir_all(assets).unwrap();
        }
    }

    pub fn run() -> Result<(), String> {
        let assets = std::env::var_os("MEW_COMPOSITION_ASSETS")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or("MEW_COMPOSITION_ASSETS must be an absolute harness directory")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !assets.join("index.html").is_file() || !assets.join("fixture.html").is_file() {
            return Err("composition assets require index.html and fixture.html".into());
        }
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let framework = std::env::var_os("MEW_CEF_FRAMEWORK_PATH").map(PathBuf::from).unwrap_or_else(|| executable.parent().unwrap().join("../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework"));
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(framework.as_os_str().as_bytes())
            .map_err(|error| error.to_string())?;
        if unsafe { cef::load_library(Some(&*path.as_ptr().cast())) } != 1 {
            return Err("cannot load the experiment's CEF framework".into());
        }
        let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        mew_cef_host::mac::setup_application();
        let mtm = MainThreadMarker::new().ok_or("main thread required")?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let _delegate = mew_cef_host::mac::setup_application_delegate();
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(80., 80.), NSSize::new(1440., 928.)),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setTitle(&NSString::from_str("mew composition · CEF CPU OSR"));
        let parent = NSView::new(mtm);
        window.setContentView(Some(&parent));
        let frames = Arc::new(Mutex::new(Frames::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let origin = server(assets, frames.clone(), stop.clone())?;
        let app_url = format!("{origin}/");
        let fixture_url = format!("{origin}/fixture.html");
        let (requests, incoming) = mpsc::channel();
        let app_id = Arc::new(AtomicI32::new(0));
        let router = BrowserSideRouter::new(MessageRouterConfig {
            js_query_function: "mewQuery".into(),
            js_cancel_function: "mewQueryCancel".into(),
            ..Default::default()
        });
        let ready = Arc::new(AtomicBool::new(false));
        let mut cef_app = ExperimentApp::new(
            mew_cef_host::app_bridge::render_process_handler(),
            ready.clone(),
            router.clone(),
            Arc::new(Queries {
                app_id: app_id.clone(),
                app_url: app_url.clone(),
                requests,
            }),
        );
        let args = cef::args::Args::new();
        let cache =
            std::env::temp_dir().join(format!("mew-composition-cef-{}", std::process::id()));
        std::fs::create_dir_all(&cache).map_err(|error| error.to_string())?;
        let framework_dir = framework.parent().ok_or("invalid framework path")?;
        let settings = Settings {
            windowless_rendering_enabled: 1,
            external_message_pump: 1,
            no_sandbox: 1,
            remote_debugging_port: std::env::var("MEW_CEF_DEBUG_PORT")
                .ok()
                .map(|value| {
                    value
                        .parse::<u16>()
                        .map(i32::from)
                        .map_err(|error| error.to_string())
                })
                .transpose()?
                .unwrap_or(0),
            root_cache_path: CefString::from(cache.to_string_lossy().as_ref()),
            log_file: CefString::from(cache.join("cef.log").to_string_lossy().as_ref()),
            framework_dir_path: CefString::from(framework_dir.to_string_lossy().as_ref()),
            main_bundle_path: std::env::var("MEW_CEF_MAIN_BUNDLE_PATH")
                .map(|value| CefString::from(value.as_str()))
                .unwrap_or_default(),
            browser_subprocess_path: std::env::var("MEW_CEF_HELPER_PATH")
                .map(|value| CefString::from(value.as_str()))
                .unwrap_or_default(),
            ..Default::default()
        };
        if cef::initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut cef_app),
            std::ptr::null_mut(),
        ) != 1
        {
            return Err("CEF initialization failed".into());
        }
        let browsers = Rc::new(RefCell::new(Vec::<Browser>::new()));
        let viewport = Arc::new(Mutex::new(Viewport {
            width: 800,
            height: 600,
            scale: 1.,
        }));
        let popup = Rc::new(RefCell::new(Popup::default()));
        let context = BrowserContext {
            browsers: browsers.clone(),
            app_id: app_id.clone(),
            router: router.clone(),
            app_url: app_url.clone(),
            frames: frames.clone(),
            viewport: viewport.clone(),
            popup,
        };
        let mut created = false;
        let start = Instant::now();
        app.finishLaunching();
        window.makeKeyAndOrderFront(None);
        app.activate();
        println!("composition ready: {origin} · fixture {fixture_url}");
        while window.isVisible() {
            objc2::rc::autoreleasepool(|_| {
                while let Some(event) = unsafe {
                    app.nextEventMatchingMask_untilDate_inMode_dequeue(
                        NSEventMask::Any,
                        Some(&NSDate::distantPast()),
                        NSDefaultRunLoopMode,
                        true,
                    )
                } {
                    app.sendEvent(&event);
                }
                cef::do_message_loop_work();
                if ready.load(Ordering::Acquire) && !created {
                    created = true;
                    for is_app in [true, false] {
                        let mut client = ExperimentClient::new(context.clone(), is_app);
                        let bounds = parent.bounds();
                        let bounds = Rect {
                            x: 0,
                            y: 0,
                            width: bounds.size.width as i32,
                            height: bounds.size.height as i32,
                        };
                        let handle = Retained::as_ptr(&parent) as *mut _;
                        let info = if is_app {
                            WindowInfo::default().set_as_child(handle, &bounds)
                        } else {
                            WindowInfo::default().set_as_windowless(handle)
                        };
                        let mut extra = dictionary_value_create().unwrap();
                        if is_app {
                            extra.set_string(
                                Some(&CefString::from("mew_app_url")),
                                Some(&CefString::from(app_url.as_str())),
                            );
                        }
                        let settings = BrowserSettings {
                            windowless_frame_rate: 60,
                            background_color: 0xff181818,
                            ..Default::default()
                        };
                        if browser_host_create_browser(
                            Some(&info),
                            Some(&mut client),
                            Some(&CefString::from(if is_app {
                                app_url.as_str()
                            } else {
                                fixture_url.as_str()
                            })),
                            Some(&settings),
                            Some(&mut extra),
                            None,
                        ) == 0
                        {
                            eprintln!("failed to create composition browser");
                            window.close();
                        }
                    }
                }
                let current = browsers.borrow().clone();
                let external = current
                    .iter()
                    .find(|browser| browser.identifier() != app_id.load(Ordering::Acquire));
                for (request, reply) in incoming.try_iter() {
                    let result = serde_json::from_str::<serde_json::Value>(&request).map_err(|error| error.to_string()).and_then(|request| {
                        match request["type"].as_str().unwrap_or("") {
                            "bootstrap" => Ok(serde_json::json!({"engine":"cef-osr","fixtureUrl":fixture_url,"versions":sdk_versions(),"versionSource":"cef-sdk","unsupported":["native IME candidate positioning","OSR platform accessibility"],"transport":"CPU BGRA HTTP"})),
                            "resize" => { let value = Viewport::from_request(&request)?; *viewport.lock().unwrap() = value; if let Some(host) = external.and_then(|browser| browser.host()) { host.notify_screen_info_changed(); host.was_resized(); } Ok(serde_json::json!({})) },
                            "input" => { input(external.ok_or("external browser is not ready")?, &request["event"])?; Ok(serde_json::json!({})) },
                            "metrics" => { let frames = frames.lock().unwrap(); Ok(serde_json::json!({"frames":frames.latest.as_ref().map_or(0, |frame| frame.sequence),"frameWidth":frames.latest.as_ref().map_or(0, |frame| frame.width),"frameHeight":frames.latest.as_ref().map_or(0, |frame| frame.height),"publishedBgraBytes":frames.bytes,"httpFrameBodyBytes":frames.http_bytes,"httpFrameResponses":frames.http_frames,"elapsedMs":start.elapsed().as_millis(),"transport":"CPU BGRA HTTP"})) },
                            _ => Err("unsupported experiment request".into()),
                        }
                    });
                    let reply = reply.lock().unwrap();
                    match result {
                        Ok(value) => reply.success_str(&value.to_string()),
                        Err(error) => reply.failure(-1, &error),
                    }
                }
                if let Some(browser) = current
                    .iter()
                    .find(|browser| browser.identifier() == app_id.load(Ordering::Acquire))
                    && let Some(host) = browser.host()
                {
                    let view = unsafe { &*host.window_handle().cast::<NSView>() };
                    if view.frame() != parent.bounds() {
                        view.setFrame(parent.bounds());
                    }
                }
                app.updateWindows();
                unsafe {
                    NSRunLoop::currentRunLoop().runMode_beforeDate(
                        NSDefaultRunLoopMode,
                        &NSDate::dateWithTimeIntervalSinceNow(0.004),
                    );
                }
            });
        }
        let current = browsers.borrow().clone();
        for browser in current {
            if let Some(host) = browser.host() {
                host.close_browser(1);
            }
        }
        let closing = Instant::now();
        while !browsers.borrow().is_empty() && closing.elapsed() < Duration::from_secs(5) {
            objc2::rc::autoreleasepool(|_| {
                cef::do_message_loop_work();
                unsafe {
                    NSRunLoop::currentRunLoop().runMode_beforeDate(
                        NSDefaultRunLoopMode,
                        &NSDate::dateWithTimeIntervalSinceNow(0.004),
                    );
                }
            });
        }
        stop.store(true, Ordering::Release);
        if !browsers.borrow().is_empty() {
            return Err("CEF browser teardown did not finish".into());
        }
        cef::shutdown();
        let _ = std::fs::remove_dir_all(cache);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    native::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("the composition experiment currently requires macOS");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_versions_match_the_pinned_sdk_version_string() {
        let version = std::ffi::CStr::from_bytes_with_nul(cef::sys::CEF_VERSION)
            .unwrap()
            .to_str()
            .unwrap();
        let report = sdk_versions();
        assert!(version.starts_with(&format!("{}+", report["cef"].as_str().unwrap())));
        assert!(version.ends_with(&format!(
            "chromium-{}",
            report["chromium"].as_str().unwrap()
        )));
    }

    #[test]
    fn frame_bytes_are_owned_and_invalid_frames_do_not_replace_the_latest() {
        let mut frames = Frames::default();
        let mut bytes = vec![7; 4 * 3 * 2];
        assert!(frames.publish(3, 2, &bytes));
        bytes.fill(9);
        let first = frames.latest.as_ref().unwrap().clone();
        assert_eq!(first.bytes.as_slice(), &[7; 24]);
        assert!(!frames.publish(0, 2, &[]));
        assert!(!frames.publish(-1, 2, &[]));
        assert!(!frames.publish(4097, 1, &[]));
        assert!(!frames.publish(3, 2, &[1; 23]));
        assert_eq!(frames.latest.as_ref().unwrap().sequence, first.sequence);
        assert!(frames.publish(1, 1, &[1, 2, 3, 255]));
        assert_eq!(frames.latest.as_ref().unwrap().sequence, 2);
        assert_eq!(frames.bytes, 28);
    }

    #[test]
    fn viewport_validation_bounds_physical_allocations_and_keeps_fractional_scale() {
        let view = Viewport::from_request(
            &serde_json::json!({"width":800,"height":600,"deviceScaleFactor":1.5}),
        )
        .unwrap();
        assert_eq!((view.width, view.height, view.scale), (800, 600, 1.5));
        for request in [
            serde_json::json!({"width":0,"height":600}),
            serde_json::json!({"width":800.5,"height":600}),
            serde_json::json!({"width":5000,"height":600}),
            serde_json::json!({"width":2048,"height":2048,"deviceScaleFactor":3}),
            serde_json::json!({"width":800,"height":600,"deviceScaleFactor":0}),
        ] {
            assert!(Viewport::from_request(&request).is_err(), "{request}");
        }
    }

    #[test]
    fn asset_paths_stay_inside_the_harness_and_frames_use_conditional_requests() {
        assert_eq!(asset_path("/").unwrap(), std::path::Path::new("index.html"));
        assert_eq!(
            asset_path("/assets/app.js?v=1").unwrap(),
            std::path::Path::new("assets/app.js")
        );
        for path in [
            "/../private",
            "/%2e%2e/private",
            "/a\\b",
            "//tmp/file",
            "/a/../b",
        ] {
            assert!(asset_path(path).is_none(), "accepted {path}");
        }
        let mut frames = Frames::default();
        frames.publish(1, 1, &[1, 2, 3, 255]);
        let frame = frames.latest.as_ref().unwrap();
        assert_eq!(frame_response(frame, Some("\"1\"")).0, 304);
        assert_eq!(frame_response(frame, Some("\"0\"")).0, 200);
        let (_, headers) = frame_response(frame, None);
        assert!(headers.contains("X-Frame-Width: 1\r\n"));
        assert!(headers.contains("X-Frame-Height: 1\r\n"));
        assert!(headers.contains("X-Frame-Sequence: 1\r\n"));
    }

    #[test]
    fn popup_pixels_clip_and_blend_without_destroying_the_view_frame() {
        let mut frames = Frames::default();
        frames.publish(2, 1, &[20, 40, 60, 255, 5, 6, 7, 255]);
        assert!(!frames.popup(0, 0, 1, 1, &[1; 3]));
        assert!(frames.popup(-1, 0, 2, 1, &[0, 0, 0, 255, 100, 0, 0, 128]));
        assert_eq!(
            frames.latest.as_ref().unwrap().bytes,
            [109, 19, 29, 255, 5, 6, 7, 255]
        );
        assert_eq!(
            frames.base.as_ref().unwrap().bytes,
            [20, 40, 60, 255, 5, 6, 7, 255]
        );
    }
}
