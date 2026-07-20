//! Headless, deterministic driver for the TUI — for tests and for agents that
//! want to exercise the interface without a real terminal, provider, or runtime.
//!
//! A [`Harness`] wraps an [`App`] backed by ratatui's `TestBackend`. It feeds
//! the app synthetic keyboard events and `AgentEvent`s, renders frames to an
//! in-memory buffer, and returns them as plain text you can read back or assert
//! on. There is no network, no async, and no dependency on `main.rs`.
//!
//! # Script format
//!
//! [`run_script`] interprets a line-based script. Blank lines and lines
//! starting with `#` are ignored. Verbs:
//!
//! - `size <w> <h>` — resize the virtual terminal (default 80x24)
//! - `type <text>` — type literal text into the composer
//! - `key <name>` — send a key: `enter`, `esc`, `tab`, `backspace`, `space`,
//!   `up`/`down`/`left`/`right`, `home`/`end`, `ctrl+c`, `alt+x`, or a single char
//! - `submit` — shorthand for `key enter`
//! - `say <text>` — inject a complete assistant text turn (start → deltas → end)
//! - `error <text>` — inject a terminal `AgentEvent::Error`
//! - `snapshot [label]` — render the current frame into the output as text
//! - `screenshot <path>` — render the current frame to a PNG file at `<path>`
//! - `start_recording` — begin capturing frames after each verb
//! - `stop_recording` — stop capturing; returns frame count
//! - `pause <ms>` — duplicate the last frame to simulate elapsed time (30fps)
//! - `record <path> [fps]` — encode recorded frames to mp4 via ffmpeg
//!
//! ```no_run
//! let out = mew_tui::harness::run_script("type hello\nsubmit\nsay hi there\nsnapshot", 80, 24);
//! print!("{out}");
//! ```

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mew_agent::AgentEvent;
use mew_message::{
    Finish, Message, MessageId, Part, PartBase, PartId, Role, SessionId, TextPart, Time, Tokens,
};
use mew_provider::ProviderEvent;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::App;
use crate::events::{handle_key_event, Action};
use crate::ui;

/// Pluggable backend for the headless verb interpreter.
///
/// The `mew-tui` crate intentionally cannot depend on `mew-daemon`, so this
/// trait only uses types already available here. Async-aware verbs such as
/// `send` and `wait_turn` for daemon-capture mode are implemented by the
/// daemon backend inside the `mew` binary crate; the interpreter dispatches
/// those directly and only uses the sync methods below for rendering,
/// screenshots, key injection, and local-only simulation.
pub trait Backend {
    /// Render the current frame as text.
    fn render(&mut self) -> String;
    /// Render the current frame to a PNG file at `path`.
    fn screenshot(&mut self, path: &str) -> io::Result<()>;
    /// Send one key event.
    fn send_key(&mut self, key: KeyEvent);
    /// Type literal text into the composer, one character at a time.
    fn type_str(&mut self, text: &str);
    /// Push text into the composer and submit it (local-only).
    fn send_text(&mut self, text: &str);
    /// Wait until the current turn finishes (local-only; no-op here).
    fn wait_turn(&mut self, _timeout_ms: u64) -> Result<(), String>;
    /// Return whether the app is currently streaming a response.
    fn is_streaming(&self) -> bool;
    /// Non-blocking poll for async events. Local backend is a no-op.
    fn poll_events(&mut self) {}
    /// Start recording frames.
    fn start_recording(&mut self);
    /// Stop recording frames.
    fn stop_recording(&mut self);
    /// Number of recorded frames.
    fn frame_count(&self) -> usize;
    /// Encode recorded frames to an MP4 file.
    fn encode_mp4(&self, path: &str, fps: u32) -> io::Result<String>;
    /// Duplicate the last recorded frame `count` times.
    fn duplicate_last_frame(&mut self, count: usize);
    /// Capture a frame if recording is enabled.
    fn capture_frame(&mut self);
    /// Access the local backend if this is one.
    fn as_local_backend_mut(&mut self) -> Option<&mut LocalBackend> {
        None
    }
    fn as_local_backend_ref(&self) -> Option<&LocalBackend> {
        None
    }
}

/// The default headless backend: an [`App`] backed by ratatui's `TestBackend`.
pub struct LocalBackend {
    /// The app under test. Public so tests can inspect or seed state directly.
    pub app: App,
    /// Actions returned by key handling, in order — useful for assertions.
    pub actions: Vec<Action>,
    terminal: Terminal<TestBackend>,
    /// Recorded frames when video recording is enabled.
    frames: Vec<tiny_skia::Pixmap>,
    /// When true, a frame is captured after each verb.
    recording: bool,
    rasterizer: mew_raster::Rasterizer,
}

impl LocalBackend {
    /// Build a local backend with a virtual terminal of the given size.
    pub fn new(width: u16, height: u16) -> Self {
        let terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        Self {
            app: App::new(),
            actions: Vec::new(),
            terminal,
            frames: Vec::new(),
            recording: false,
            rasterizer: mew_raster::Rasterizer::new(),
        }
    }

    /// Resize the virtual terminal.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.terminal.backend_mut().resize(width, height);
    }

    /// Feed a raw `AgentEvent` to the app, exactly as the real event loop does.
    pub fn agent(&mut self, event: AgentEvent) {
        self.app.handle_agent_event(event);
    }

    fn apply_action(&mut self, action: Action) {
        match &action {
            Action::Submit(text) => {
                self.push_user_message(text.clone());
                self.app.streaming = true;
            }
            Action::Quit => self.app.should_quit = true,
            Action::Clear => self.app.clear_messages(),
            _ => {}
        }
        self.actions.push(action);
    }

    fn push_user_message(&mut self, text: String) {
        let msg_id = MessageId::new();
        let session_id = SessionId::new();
        self.app.push_message(Message {
            id: msg_id,
            session_id,
            role: Role::User,
            parts: vec![Part::Text(TextPart {
                base: PartBase {
                    id: PartId::new(),
                    message_id: msg_id,
                    session_id,
                },
                text,
                synthetic: false,
            })],
            time: Time {
                created: chrono::Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: None,
        });
    }

    fn capture_frame_raw(&mut self) {
        let app = &mut self.app;
        self.terminal.draw(|f| ui::draw(f, app)).expect("draw");
        let buf = self.terminal.backend().buffer();
        let pixmap = self
            .rasterizer
            .rasterize(buf, &mew_raster::RasterOptions::default());
        self.frames.push(pixmap);
    }

    /// Inject a complete assistant text turn: `PartStart` → `PartDelta`s →
    /// `PartEnd` → `MessageEnd`, mirroring how a real provider streams text.
    /// When recording, captures a frame after each delta chunk for a streaming
    /// animation effect.
    pub fn say(&mut self, text: &str) {
        let part_id = PartId::new();
        let base = PartBase {
            id: part_id,
            message_id: MessageId::new(),
            session_id: SessionId::new(),
        };
        self.agent(AgentEvent::Provider(ProviderEvent::PartStart {
            part: Part::Text(TextPart {
                base,
                text: String::new(),
                synthetic: false,
            }),
        }));
        for chunk in text.chars().collect::<Vec<_>>().chunks(8) {
            self.agent(AgentEvent::Provider(ProviderEvent::PartDelta {
                part_id,
                field: "text",
                delta: chunk.iter().collect(),
            }));
            // Capture a frame after each delta to show streaming text
            self.capture_frame();
        }
        self.agent(AgentEvent::Provider(ProviderEvent::PartEnd { part_id }));
        self.agent(AgentEvent::Provider(ProviderEvent::MessageEnd {
            finish: Finish::Stop,
            usage: Tokens::default(),
            cost: 0.0,
        }));
        self.app.streaming = false;
        self.capture_frame();
    }

    /// Inject a terminal error event.
    pub fn error(&mut self, message: &str) {
        self.agent(AgentEvent::Error(message.to_string()));
    }

    /// Inject a completed tool call with `output` and optional `diff`.
    /// Mirrors the real event sequence: PartStart(ToolCall) → ToolStart →
    /// ToolEnd → PartUpdated(Completed). The resulting `ToolDisplayState`
    /// carries the output + diff that the chat renderer reads.
    pub fn say_tool_call(&mut self, tool_name: &str, output: &str, diff: Option<&str>) {
        use mew_message::{ToolCallPart, ToolState, ToolStateCompleted, ToolTime};
        let part_id = PartId::new();
        let msg_id = MessageId::new();
        let session_id = SessionId::new();
        let call_id = format!("call_{}", ulid::Ulid::new());
        let base = PartBase {
            id: part_id,
            message_id: msg_id,
            session_id,
        };
        // Ensure an assistant message exists to hold the tool call part.
        if !self
            .app
            .messages()
            .iter()
            .any(|m| m.role == Role::Assistant)
        {
            self.app.push_message(Message {
                id: msg_id,
                session_id,
                role: Role::Assistant,
                parts: vec![],
                time: Time {
                    created: chrono::Utc::now().timestamp_millis(),
                    completed: None,
                },
                assistant: None,
            });
        }
        let part = Part::ToolCall(ToolCallPart {
            base: base.clone(),
            tool_name: tool_name.to_string(),
            call_id: call_id.clone(),
            state: ToolState::Pending(mew_message::ToolStatePending {
                input: serde_json::json!({}),
                time: ToolTime {
                    start: 0,
                    end: None,
                },
            }),
            raw_input: String::new(),
        });
        self.agent(AgentEvent::Provider(ProviderEvent::PartStart {
            part: part.clone(),
        }));
        self.agent(AgentEvent::ToolStart {
            call_id: call_id.clone(),
        });
        self.agent(AgentEvent::ToolEnd {
            call_id: call_id.clone(),
            success: true,
        });
        let completed = Part::ToolCall(ToolCallPart {
            base: base.clone(),
            tool_name: tool_name.to_string(),
            call_id: call_id.clone(),
            state: ToolState::Completed(ToolStateCompleted {
                input: serde_json::json!({}),
                output: output.to_string(),
                metadata: None,
                diff: diff.map(|s| s.to_string()),
                time: ToolTime {
                    start: 0,
                    end: Some(0),
                },
            }),
            raw_input: String::new(),
        });
        self.agent(AgentEvent::PartUpdated {
            part_id,
            part: completed,
        });
    }

    /// Inject a reasoning/thinking part with `text`. The block renders
    /// collapsed by default (header line only) unless expanded.
    pub fn say_reasoning(&mut self, text: &str) {
        use mew_message::ReasoningPart;
        let part_id = PartId::new();
        let msg_id = MessageId::new();
        let session_id = SessionId::new();
        if !self
            .app
            .messages()
            .iter()
            .any(|m| m.role == Role::Assistant)
        {
            self.app.push_message(Message {
                id: msg_id,
                session_id,
                role: Role::Assistant,
                parts: vec![],
                time: Time {
                    created: chrono::Utc::now().timestamp_millis(),
                    completed: None,
                },
                assistant: None,
            });
        }
        self.agent(AgentEvent::Provider(ProviderEvent::PartStart {
            part: Part::Reasoning(ReasoningPart {
                base: PartBase {
                    id: part_id,
                    message_id: msg_id,
                    session_id,
                },
                text: text.to_string(),
                signature: None,
                encrypted_content: None,
            }),
        }));
        self.agent(AgentEvent::Provider(ProviderEvent::PartEnd { part_id }));
    }
}

impl Backend for LocalBackend {
    fn render(&mut self) -> String {
        let app = &mut self.app;
        self.terminal.draw(|f| ui::draw(f, app)).expect("draw");
        buffer_to_string(self.terminal.backend().buffer())
    }

    fn screenshot(&mut self, path: &str) -> io::Result<()> {
        let app = &mut self.app;
        self.terminal.draw(|f| ui::draw(f, app)).expect("draw");
        let buf = self.terminal.backend().buffer();
        let png_bytes = self
            .rasterizer
            .to_png(buf, &mew_raster::RasterOptions::default());
        std::fs::write(path, png_bytes)
    }

    fn send_key(&mut self, key: KeyEvent) {
        if let Some(action) = handle_key_event(&mut self.app, key) {
            self.apply_action(action);
        }
        self.capture_frame();
    }

    fn type_str(&mut self, text: &str) {
        // Type without per-character captures; rasterizing every keystroke is
        // too slow for long strings.
        for ch in text.chars() {
            if let Some(action) = handle_key_event(
                &mut self.app,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            ) {
                self.apply_action(action);
            }
        }
        self.capture_frame();
    }

    fn send_text(&mut self, text: &str) {
        self.type_str(text);
        self.send_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    fn wait_turn(&mut self, _timeout_ms: u64) -> Result<(), String> {
        // In local mode the turn is always finished synchronously.
        Ok(())
    }

    fn is_streaming(&self) -> bool {
        self.app.streaming
    }

    fn start_recording(&mut self) {
        self.recording = true;
        self.frames.clear();
        // Capture the initial frame
        self.capture_frame();
    }

    fn stop_recording(&mut self) {
        // Capture a final frame before flipping the flag off
        let was_recording = self.recording;
        self.recording = false;
        if was_recording {
            self.capture_frame_raw();
        }
    }

    fn frame_count(&self) -> usize {
        self.frames.len()
    }

    fn encode_mp4(&self, output_path: &str, fps: u32) -> io::Result<String> {
        mew_raster::encode_frames_mp4(&self.frames, output_path, fps)
    }

    fn duplicate_last_frame(&mut self, count: usize) {
        if let Some(last) = self.frames.last().cloned() {
            for _ in 0..count {
                self.frames.push(last.clone());
            }
        }
    }

    fn capture_frame(&mut self) {
        if !self.recording {
            return;
        }
        self.capture_frame_raw();
    }

    fn as_local_backend_mut(&mut self) -> Option<&mut LocalBackend> {
        Some(self)
    }

    fn as_local_backend_ref(&self) -> Option<&LocalBackend> {
        Some(self)
    }
}

/// A headless TUI driven programmatically.
///
/// The harness owns a [`Backend`] and interprets script verbs against it. The
/// default backend is [`LocalBackend`], preserving the original deterministic
/// behavior and field-level API (`h.app`, `h.actions`).
pub struct Harness<B: Backend = LocalBackend> {
    backend: B,
}

impl Harness<LocalBackend> {
    /// Build a harness with the default local backend.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            backend: LocalBackend::new(width, height),
        }
    }
}

impl<B: Backend> Harness<B> {
    /// Build a harness with a custom backend.
    pub fn with_backend(backend: B) -> Self {
        Self { backend }
    }

    /// Resize the virtual terminal.
    pub fn resize(&mut self, width: u16, height: u16) {
        if let Some(l) = self.backend.as_local_backend_mut() {
            l.resize(width, height);
        }
    }

    /// Send one key event, applying the harness-visible effect of any returned
    /// action (echoing the user message on submit, quitting, clearing).
    /// When recording, captures a frame after the key is processed.
    pub fn key(&mut self, key: KeyEvent) {
        self.backend.send_key(key);
    }

    /// Type literal text into the composer, one character at a time.
    /// When recording, captures a frame after each keystroke for a natural
    /// typing animation.
    pub fn type_str(&mut self, text: &str) {
        self.backend.type_str(text);
    }

    /// Inject a complete assistant text turn.
    pub fn say(&mut self, text: &str) {
        self.backend
            .as_local_backend_mut()
            .expect("say() only supported on LocalBackend")
            .say(text);
    }

    /// Inject a terminal error event.
    pub fn error(&mut self, message: &str) {
        self.backend
            .as_local_backend_mut()
            .expect("error() only supported on LocalBackend")
            .error(message);
    }

    /// Inject a completed tool call.
    pub fn say_tool_call(&mut self, tool_name: &str, output: &str, diff: Option<&str>) {
        self.backend
            .as_local_backend_mut()
            .expect("say_tool_call() only supported on LocalBackend")
            .say_tool_call(tool_name, output, diff);
    }

    /// Inject a reasoning/thinking part.
    pub fn say_reasoning(&mut self, text: &str) {
        self.backend
            .as_local_backend_mut()
            .expect("say_reasoning() only supported on LocalBackend")
            .say_reasoning(text);
    }

    /// Feed a raw `AgentEvent` to the app.
    pub fn agent(&mut self, event: AgentEvent) {
        self.backend
            .as_local_backend_mut()
            .expect("agent() only supported on LocalBackend")
            .agent(event);
    }

    /// Render the current state and return the frame as text.
    pub fn render(&mut self) -> String {
        self.backend.render()
    }

    /// Render the current frame to a PNG file at `path`.
    pub fn screenshot(&mut self, path: &str) -> io::Result<()> {
        self.backend.screenshot(path)
    }

    /// Start recording frames.
    pub fn start_recording(&mut self) {
        self.backend.start_recording();
    }

    /// Stop recording frames.
    pub fn stop_recording(&mut self) {
        self.backend.stop_recording();
    }

    /// Number of recorded frames.
    pub fn frame_count(&self) -> usize {
        self.backend.frame_count()
    }

    /// Encode recorded frames to an MP4 file.
    pub fn encode_mp4(&self, path: &str, fps: u32) -> io::Result<String> {
        self.backend.encode_mp4(path, fps)
    }

    /// Duplicate the last recorded frame.
    pub fn duplicate_last_frame(&mut self, count: usize) {
        self.backend.duplicate_last_frame(count);
    }

    /// Execute a single script verb against this harness.
    ///
    /// Returns any text output (snapshots, error messages, status lines).
    /// This is the shared logic between `run_script` and the interactive
    /// `mew tui-capture --interactive` REPL.
    pub fn exec_verb(&mut self, line: &str) -> String {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return String::new();
        }

        let (verb, rest) = match line.split_once(char::is_whitespace) {
            Some((v, r)) => (v, r.trim()),
            None => (line, ""),
        };

        let mut out = String::new();
        match verb {
            "size" | "resize" => match parse_size(rest) {
                Some((w, hh)) => self.resize(w, hh),
                None => out.push_str(&format!("!! bad size '{rest}'\n")),
            },
            "type" => self.type_str(rest),
            "key" => match parse_key(rest) {
                Some(key) => self.key(key),
                None => out.push_str(&format!("!! unknown key '{rest}'\n")),
            },
            "submit" => self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            "say" => {
                if self.backend.as_local_backend_mut().is_none() {
                    out.push_str("!! 'say' is not available in daemon-capture mode\n");
                } else {
                    self.say(rest);
                }
            }
            "error" => {
                if self.backend.as_local_backend_mut().is_none() {
                    out.push_str("!! 'error' is not available in daemon-capture mode\n");
                } else {
                    self.error(rest);
                }
            }
            "settings" => {
                if self.backend.as_local_backend_mut().is_none() {
                    out.push_str("!! 'settings' is not available in daemon-capture mode\n");
                } else {
                    let cfg = mew_config::Config::default();
                    let app = &mut self
                        .backend
                        .as_local_backend_mut()
                        .expect("settings only supported on LocalBackend")
                        .app;
                    app.settings = Some(crate::settings::SettingsState::new(cfg, Vec::new()));
                    app.mode = crate::app::Mode::Settings;
                }
            }
            "settings_config" => {
                if self.backend.as_local_backend_mut().is_none() {
                    out.push_str("!! 'settings_config' is not available in daemon-capture mode\n");
                } else {
                    let path = rest.trim().trim_matches('"');
                    if path.is_empty() {
                        out.push_str("!! settings_config requires a file path\n");
                    } else {
                        match std::fs::read_to_string(path) {
                            Ok(text) => match toml::from_str::<mew_config::Config>(&text) {
                                Ok(cfg) => {
                                    let app = &mut self
                                        .backend
                                        .as_local_backend_mut()
                                        .expect("settings_config only supported on LocalBackend")
                                        .app;
                                    app.settings =
                                        Some(crate::settings::SettingsState::new(cfg, Vec::new()));
                                    app.mode = crate::app::Mode::Settings;
                                }
                                Err(e) => out.push_str(&format!("!! bad config toml: {e}\n")),
                            },
                            Err(e) => out.push_str(&format!("!! cannot read config: {e}\n")),
                        }
                    }
                }
            }
            "snapshot" => {
                let label = if rest.is_empty() {
                    String::new()
                } else {
                    format!(" {rest}")
                };
                out.push_str(&format!("--- snapshot{label} ---\n"));
                out.push_str(&self.render());
                out.push_str("---\n");
            }
            "screenshot" => {
                if rest.is_empty() {
                    out.push_str("!! screenshot requires a file path\n");
                } else {
                    match self.screenshot(rest.trim_matches('"')) {
                        Ok(()) => out.push_str(&format!("--- screenshot saved to {rest} ---\n")),
                        Err(e) => out.push_str(&format!("!! screenshot failed: {e}\n")),
                    }
                }
            }
            "start_recording" => {
                self.start_recording();
                out.push_str("--- recording started ---\n");
            }
            "stop_recording" => {
                self.stop_recording();
                out.push_str(&format!(
                    "--- recording stopped ({} frames) ---\n",
                    self.frame_count()
                ));
            }
            "pause" => {
                let fps: u32 = 30;
                match rest.trim_end_matches("ms").parse::<u32>() {
                    Ok(ms) => {
                        let frames = (ms as f32 / (1000.0 / fps as f32)).round() as usize;
                        self.duplicate_last_frame(frames);
                    }
                    Err(_) => {
                        out.push_str(&format!("!! bad pause duration '{rest}'\n"));
                    }
                }
            }
            "record" => {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.is_empty() {
                    out.push_str("!! record requires an output path\n");
                } else {
                    let path = parts[0].trim_matches('"');
                    let fps: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
                    match self.encode_mp4(path, fps) {
                        Ok(_) => out.push_str(&format!("--- video saved to {path} ---\n")),
                        Err(e) => out.push_str(&format!("!! record failed: {e}\n")),
                    }
                }
            }
            "help" | "?" => {
                out.push_str("verbs: type, key, submit, say, error, ");
                out.push_str("settings, settings_config, snapshot, screenshot, ");
                out.push_str("start_recording, stop_recording, pause, record, size, quit\n");
            }
            "quit" | "exit" => {
                out.push_str("--- bye ---\n");
            }
            other => out.push_str(&format!("!! unknown verb '{other}'\n")),
        }
        out
    }
}

impl std::ops::Deref for Harness<LocalBackend> {
    type Target = LocalBackend;

    fn deref(&self) -> &Self::Target {
        &self.backend
    }
}

impl std::ops::DerefMut for Harness<LocalBackend> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.backend
    }
}

/// Render a ratatui buffer to text, one line per row, trailing spaces trimmed.
fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area();
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let start = out.len();
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        while out.len() > start && out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}

/// Run a line-based script against a fresh harness and return the accumulated
/// snapshots as text. See the module docs for the verb list.
pub fn run_script(script: &str, width: u16, height: u16) -> String {
    let mut h = Harness::new(width, height);
    let mut out = String::new();

    for (i, raw) in script.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Prefix error messages with line number for script mode
        let result = h.exec_verb(line);
        if result.starts_with("!!") {
            out.push_str(&format!("line {}: {}", i + 1, result));
        } else {
            out.push_str(&result);
        }
    }
    out
}

fn parse_size(s: &str) -> Option<(u16, u16)> {
    let mut parts = s
        .split(|c: char| c == 'x' || c.is_whitespace())
        .filter(|p| !p.is_empty());
    let w = parts.next()?.parse().ok()?;
    let h = parts.next()?.parse().ok()?;
    Some((w, h))
}

/// Parse a key name into a `KeyEvent`. Supports named keys, `ctrl+`/`alt+`/
/// `shift+` modifier prefixes, and single characters.
pub fn parse_key(name: &str) -> Option<KeyEvent> {
    let name = name.trim();
    let (mods, key) = if let Some(rest) = name.strip_prefix("ctrl+shift+") {
        (KeyModifiers::CONTROL, rest)
    } else if let Some(rest) = name.strip_prefix("ctrl+") {
        (KeyModifiers::CONTROL, rest)
    } else if let Some(rest) = name.strip_prefix("alt+") {
        (KeyModifiers::ALT, rest)
    } else if let Some(rest) = name.strip_prefix("shift+") {
        (KeyModifiers::SHIFT, rest)
    } else {
        (KeyModifiers::NONE, name)
    };
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        // "tab" with the SHIFT modifier (from "shift+tab") or with CONTROL
        // (from "ctrl+shift+tab") maps to BackTab, matching how terminals
        // deliver Shift+Tab / Ctrl+Shift+Tab to crossterm. For the CONTROL
        // case we keep the CONTROL bit so the handler can distinguish the
        // backward cycle from the forward one.
        "tab" if mods.contains(KeyModifiers::SHIFT) || mods.contains(KeyModifiers::CONTROL) => {
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" | "bs" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None; // multi-char, not a known name
            }
            KeyCode::Char(c)
        }
    };
    Some(KeyEvent::new(code, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_appears_in_composer() {
        let mut h = Harness::new(80, 24);
        h.type_str("hello world");
        assert!(h.render().contains("hello world"));
        assert_eq!(h.app.input, "hello world");
    }

    #[test]
    fn submit_echoes_user_message_and_streams() {
        let mut h = Harness::new(80, 24);
        h.type_str("do the thing");
        h.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(h.app.streaming);
        assert!(matches!(h.actions.last(), Some(Action::Submit(_))));
        assert!(h.render().contains("do the thing"));
    }

    #[test]
    fn assistant_text_turn_renders() {
        let mut h = Harness::new(80, 24);
        h.say("the answer is 42");
        assert!(!h.app.streaming);
        assert!(h.render().contains("the answer is 42"));
    }

    #[test]
    fn script_runs_and_snapshots() {
        let out = run_script(
            "# a quick session\ntype hi\nsubmit\nsay hello back\nsnapshot result",
            80,
            24,
        );
        assert!(out.contains("--- snapshot result ---"));
        assert!(out.contains("hello back"));
    }

    #[test]
    fn parse_key_handles_modifiers_and_names() {
        assert_eq!(
            parse_key("ctrl+c"),
            Some(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        );
        assert_eq!(
            parse_key("enter"),
            Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        );
        assert_eq!(parse_key("nope-not-a-key"), None);
    }

    #[test]
    fn screenshot_verb_writes_png_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png_path = dir.path().join("frame.png");
        let png_str = png_path.to_str().unwrap();

        let script = format!("type hello\nsubmit\nsay hello back\nscreenshot {png_str}");
        let out = run_script(&script, 80, 24);

        // The script should report success
        assert!(out.contains("screenshot saved"), "output was: {out}");

        // The file should exist and be a valid PNG
        assert!(png_path.exists(), "PNG file should exist");
        let bytes = std::fs::read(&png_path).expect("read png");
        assert!(bytes.len() > 100, "PNG should have content");
        // PNG magic bytes
        assert_eq!(
            &bytes[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn screenshot_verb_requires_path() {
        let out = run_script("screenshot", 80, 24);
        assert!(out.contains("requires a file path"));
    }

    #[test]
    fn recording_captures_frames_per_verb() {
        let mut h = Harness::new(40, 10);
        h.start_recording();
        // start_recording captures 1 initial frame
        assert_eq!(h.frame_count(), 1);

        // type_str: "hi" = 2 keystrokes, captured as 1 final frame
        h.type_str("hi");
        assert_eq!(h.frame_count(), 2);

        // key: enter = 1 frame
        h.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(h.frame_count(), 3);

        // say: "hello" has 5 chars, chunked by 8 = 1 delta + 1 final = 2 frames
        h.say("hello");
        assert_eq!(h.frame_count(), 5);

        h.stop_recording();
        // stop_recording captures 1 final frame
        assert_eq!(h.frame_count(), 6);
    }

    #[test]
    fn pause_duplicates_frames() {
        let mut h = Harness::new(40, 10);
        h.start_recording();
        assert_eq!(h.frame_count(), 1);
        h.duplicate_last_frame(15); // ~500ms at 30fps
        assert_eq!(h.frame_count(), 16);
        h.stop_recording();
        assert_eq!(h.frame_count(), 17);
    }

    #[test]
    fn pause_verb_in_script() {
        let out = run_script("start_recording\npause 1000\nstop_recording", 40, 10);
        // 1000ms at 30fps = 30 frames duplicated
        // start_recording: 1 frame + 30 pause + stop_recording: 1 = 32
        assert!(out.contains("32 frames"));
    }

    #[test]
    fn record_verb_produces_mp4() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mp4_path = dir.path().join("test.mp4");
        let mp4_str = mp4_path.to_str().unwrap();

        let script = format!(
            "start_recording\ntype hello\nsubmit\nsay hi there\npause 500\nstop_recording\nrecord {mp4_str} 30"
        );
        let out = run_script(&script, 60, 12);

        if !out.contains("video saved") {
            // ffmpeg might not be available in CI — skip gracefully
            eprintln!("record_verb_produces_mp4 skipped (ffmpeg unavailable?): {out}");
            return;
        }

        assert!(mp4_path.exists(), "MP4 file should exist");
        let bytes = std::fs::read(&mp4_path).expect("read mp4");
        assert!(bytes.len() > 100, "MP4 should have content");
    }

    #[test]
    fn settings_verb_opens_overlay() {
        let mut h = Harness::new(80, 24);
        h.exec_verb("settings");
        assert!(matches!(h.app.mode, crate::app::Mode::Settings));
        assert!(h.app.settings.is_some());
        assert!(h.render().contains("mew settings"));
    }

    #[test]
    fn settings_config_verb_opens_overlay_with_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg_path = dir.path().join("cfg.toml");
        std::fs::write(
            &cfg_path,
            r#"
[providers.z-ai]
shape = "openai"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential_ref = "z-ai"
"#,
        )
        .unwrap();
        let cfg_str = cfg_path.to_str().unwrap();

        std::env::set_var("MEW_CRED_Z_AI", "fake-key");
        let mut h = Harness::new(80, 24);
        h.exec_verb(&format!("settings_config \"{cfg_str}\""));
        let rendered = h.render();
        std::env::remove_var("MEW_CRED_Z_AI");
        assert!(rendered.contains("mew settings"));
        assert!(rendered.contains("z-ai"));
    }

    #[test]
    fn settings_config_verb_reports_missing_path() {
        let out = run_script("settings_config", 80, 24);
        assert!(out.contains("requires a file path"));
    }

    #[test]
    fn help_includes_settings_verbs() {
        let out = run_script("help", 80, 24);
        assert!(out.contains("settings"));
        assert!(out.contains("settings_config"));
    }
}
