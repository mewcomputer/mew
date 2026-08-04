//! `mew tui-capture` — deterministic TUI screenshots and video.
//!
//! Two modes:
//! - **Harness mode** (default): reads a harness script and runs it against a
//!   headless `TestBackend` + local `App`. No daemon, fully deterministic.
//! - **Daemon mode** (`--connect <url>`): connects to a running mew daemon and
//!   runs the script against the real agent/event loop. Supports async-aware
//!   verbs `send` and `wait_turn`.
//!
//! In both modes, text snapshots go to stdout and screenshots/videos write to
//! files.

use anyhow::{bail, Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, trace, warn};

use mew_protocol::ServerMessage;
use mew_tui::harness::{Backend, Harness};

/// Run the tui-capture command.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    script: Option<&Path>,
    interactive: bool,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
    connect: Option<&str>,
) -> Result<()> {
    let mode = if connect.is_some() {
        "daemon"
    } else {
        "harness"
    };
    let submode = if interactive { "interactive" } else { "script" };
    info!(
        mode,
        submode,
        width,
        height,
        fps,
        ?screenshot_dir,
        ?mp4,
        "starting tui-capture"
    );

    let result = if let Some(url) = connect {
        if interactive {
            run_interactive_daemon(width, height, screenshot_dir, mp4, fps, url).await
        } else {
            let script_path = script.context("either --script or --interactive is required")?;
            run_script_daemon_file(script_path, screenshot_dir, mp4, fps, width, height, url)
                .await?;
            Ok(())
        }
    } else if interactive {
        run_interactive(width, height, screenshot_dir, mp4, fps)
    } else {
        let script_path = script.context("either --script or --interactive is required")?;
        run_script_file(script_path, mp4, fps, width, height)
    };

    match &result {
        Ok(()) => info!(mode, submode, "tui-capture finished"),
        Err(e) => warn!(mode, submode, error = %e, "tui-capture failed"),
    }
    result
}

/// Script mode (harness): read a file and run it all at once.
fn run_script_file(
    script_path: &Path,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
) -> Result<()> {
    info!(script = %script_path.display(), "running harness script");
    let script = std::fs::read_to_string(script_path)
        .with_context(|| format!("failed to read script file: {}", script_path.display()))?;

    let output = run_script_harness(&script, mp4, fps, width, height)?;
    print!("{output}");
    io::stdout().flush().ok();
    info!(script = %script_path.display(), "harness script finished");
    Ok(())
}

/// Wrap or run a harness script and return the accumulated output.
fn run_script_harness(
    script: &str,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
) -> Result<String> {
    debug!(mp4, fps, width, height, "preparing harness script");
    // If --mp4 is given and the script doesn't already handle recording,
    // wrap it automatically.
    let script = if let Some(mp4_path) = mp4 {
        if script.contains("start_recording") {
            script.to_string()
        } else {
            format!("start_recording\n{script}\nstop_recording\nrecord \"{mp4_path}\" {fps}")
        }
    } else {
        script.to_string()
    };

    let output = mew_tui::harness::run_script(&script, width, height);
    debug!(output_len = output.len(), "harness script produced output");
    Ok(output)
}

/// Interactive REPL mode (harness): read verbs from stdin, print frames.
fn run_interactive(
    width: u16,
    height: u16,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
) -> Result<()> {
    info!(
        width,
        height,
        ?screenshot_dir,
        ?mp4,
        fps,
        "starting interactive harness capture"
    );
    let mut harness = Harness::new(width, height);

    // Start recording if --mp4 is set
    if mp4.is_some() {
        info!("starting mp4 recording");
        harness.start_recording();
    }

    // Create screenshot dir if needed
    if let Some(dir) = screenshot_dir {
        info!(dir = %dir.display(), "creating screenshot directory");
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create screenshot dir: {}", dir.display()))?;
    }

    let mut frame_num: u32 = 0;

    // Print initial frame
    print_frame_harness(&mut harness, screenshot_dir, &mut frame_num);

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    for line in lines.by_ref() {
        let line = line.context("failed to read stdin")?;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for quit before executing
        if matches!(trimmed, "quit" | "exit") {
            break;
        }

        // Execute the verb and print any output it produces
        let result = harness.exec_verb(trimmed);
        if !result.is_empty() {
            print!("{result}");
            io::stdout().flush().ok();
        }

        // Print the current frame + optional screenshot
        print_frame_harness(&mut harness, screenshot_dir, &mut frame_num);
    }

    // Stop recording and encode video if --mp4 was set
    if let Some(mp4_path) = mp4 {
        info!("stopping mp4 recording");
        harness.stop_recording();
        match harness.encode_mp4(mp4_path, fps) {
            Ok(_) => {
                info!(path = %mp4_path, "video saved");
                eprintln!("--- video saved to {mp4_path} ---");
            }
            Err(e) => {
                warn!(path = %mp4_path, error = %e, "video encoding failed");
                eprintln!("!! video encoding failed: {e}");
            }
        }
    }

    info!("interactive harness capture finished");
    println!("--- bye ---");
    Ok(())
}

/// Print the current harness state as a text frame.
fn print_frame_harness(harness: &mut Harness, screenshot_dir: Option<&Path>, frame_num: &mut u32) {
    println!("--- frame ---");
    print!("{}", harness.render());
    println!("---");

    if let Some(dir) = screenshot_dir {
        *frame_num += 1;
        let filename = format!("frame_{:04}.png", *frame_num);
        let png_path = dir.join(&filename);
        match harness.screenshot(png_path.to_str().unwrap()) {
            Ok(()) => {
                trace!(path = %png_path.display(), "harness screenshot saved");
                println!("--- screenshot: {} ---", png_path.display());
            }
            Err(e) => {
                warn!(path = %png_path.display(), error = %e, "harness screenshot failed");
                eprintln!("!! screenshot failed: {e}");
            }
        }
    }

    io::stdout().flush().ok();
}

// ---------------------------------------------------------------------------
// Daemon-connected capture backend
// ---------------------------------------------------------------------------

/// Headless capture backend connected to a real mew daemon.
pub(crate) struct DaemonBackend {
    client: Arc<mew_daemon::DaemonClient>,
    app: mew_tui::App,
    terminal: Terminal<TestBackend>,
    event_loop: mew_tui::EventLoop,
    event_rx: tokio::sync::mpsc::Receiver<mew_tui::Event>,
    notify_rx: tokio::sync::mpsc::Receiver<ServerMessage>,
    frames: Vec<tiny_skia::Pixmap>,
    recording: bool,
    rasterizer: mew_raster::Rasterizer,
}

impl DaemonBackend {
    /// Connect to a daemon, create a session, and prepare a headless TUI.
    pub(crate) async fn connect(url: &str, width: u16, height: u16) -> Result<Self> {
        info!(url, width, height, "connecting to daemon");
        let (client, mut notify_rx) = mew_daemon::DaemonClient::connect(url).await?;
        let client = Arc::new(client);
        info!("creating daemon session");
        client.new_session().await?;

        // Wait for the daemon to assign a session ID and report the active
        // model/provider so the status bar reflects the real backend.
        info!("waiting for SessionReady");
        let (session_id, model, provider) =
            wait_for_session_ready(&mut notify_rx, Duration::from_secs(5)).await?;
        info!(%session_id, ?model, ?provider, "session ready");

        let mut app = mew_tui::App::new();
        app.daemon_mode = true;
        app.status.session_id = session_id;
        app.status.model = model.unwrap_or_else(|| "daemon".to_string());
        app.status.provider = provider.unwrap_or_else(|| "mewd".to_string());

        let state = mew_config::load_state().unwrap_or_default();
        let cfg = mew_config::load().unwrap_or_default();
        let theme_name = if !state.theme.is_empty() {
            &state.theme
        } else {
            &cfg.tui.theme
        };
        app.theme = mew_tui::theme::Theme::load(theme_name);
        debug!(theme = %app.theme.name, "loaded theme");

        // Populate the sidebar session rail.
        client.list_sessions().await?;

        let terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        let (event_loop, event_rx) = mew_tui::EventLoop::new();
        info!("daemon backend ready");

        Ok(Self {
            client,
            app,
            terminal,
            event_loop,
            event_rx,
            notify_rx,
            frames: Vec::new(),
            recording: false,
            rasterizer: mew_raster::Rasterizer::new(),
        })
    }

    /// Type text into the composer and submit it to the daemon, then wait until
    /// the turn begins streaming.
    pub(crate) async fn send_text(&mut self, text: &str) -> Result<()> {
        info!(text_len = text.len(), "send_text: typing prompt");
        let type_start = Instant::now();
        // Type the text into the composer without capturing every keystroke;
        // rasterizing per-character makes long prompts prohibitively slow.
        for ch in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let event = crossterm::event::Event::Key(key);
            if let Some(action) = mew_tui::events::handle_input_event(&mut self.app, event) {
                // Normal character keys shouldn't produce actions; if they do,
                // dispatch them anyway.
                self.dispatch_action(action).await;
            }
        }
        self.capture_frame();
        info!(
            text_len = text.len(),
            elapsed_ms = type_start.elapsed().as_millis(),
            "send_text: finished typing"
        );

        // Submit with Enter.
        info!("send_text: submitting prompt");
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let event = crossterm::event::Event::Key(key);
        let action = mew_tui::events::handle_input_event(&mut self.app, event);
        if let Some(action) = action {
            self.dispatch_action(action).await;
        }
        self.capture_frame();

        // Wait until the daemon marks the app as streaming, with a short timeout.
        let deadline = Instant::now() + Duration::from_secs(5);
        let wait_start = Instant::now();
        while !self.app.streaming {
            self.poll_events();
            if Instant::now() > deadline {
                warn!("send_text: turn did not start streaming within 5s");
                bail!("send: turn did not start streaming within 5s");
            }
            if !self.app.streaming {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        debug!(
            elapsed_ms = wait_start.elapsed().as_millis(),
            "send_text: streaming started"
        );
        Ok(())
    }

    /// Block until the current daemon turn finishes streaming.
    /// While streaming, capture at 60fps. Captures one final frame after
    /// streaming ends so subsequent `pause` verbs hold the completed response.
    pub(crate) async fn wait_turn(&mut self, timeout_ms: u64) -> Result<()> {
        // Match the real TUI's streaming drain limit: process agent events in
        // small batches so text/reasoning appears incrementally instead of
        // jumping straight to the final state when a burst of deltas arrives.
        const STREAMING_DRAIN_LIMIT: u32 = 4;
        let start = Instant::now();
        let deadline = start + Duration::from_millis(timeout_ms);
        let initial_frame_count = self.frames.len();
        info!(timeout_ms, "wait_turn: waiting for turn to finish");
        let mut last_frame = Instant::now();
        // The headless backend has no tick generator task (the real TUI's
        // EventLoop spawns one), so drive app.tick() on the same 16ms cadence
        // here — it advances the spinner and other time-based UI state.
        let mut last_tick = Instant::now();
        let mut total_agent_events = 0u32;
        while self.app.streaming {
            let mut had_event = false;
            let mut agent_count = 0u32;
            loop {
                match self.event_rx.try_recv() {
                    Ok(mew_tui::Event::Agent(ev)) => {
                        trace!(
                            variant = agent_event_name(&ev),
                            "wait_turn: received agent event"
                        );
                        self.app.handle_agent_event(ev);
                        had_event = true;
                        agent_count += 1;
                        total_agent_events += 1;
                        if self.app.streaming && agent_count >= STREAMING_DRAIN_LIMIT {
                            break;
                        }
                    }
                    Ok(mew_tui::Event::Tick) => {
                        self.app.tick();
                        self.app.clear_expired_alerts();
                        had_event = true;
                    }
                    Ok(mew_tui::Event::Input(_) | mew_tui::Event::Quit) => {}
                    Err(_) => break,
                }
                while let Ok(msg) = self.notify_rx.try_recv() {
                    trace!(
                        msg_type = server_message_type(&msg),
                        "wait_turn: received server message"
                    );
                    self.app.apply_daemon_notification(&msg);
                    had_event = true;
                }
            }

            let now = Instant::now();
            if now.duration_since(last_tick).as_millis() >= 16 {
                self.app.tick();
                last_tick = now;
            }
            let frame_due = now.duration_since(last_frame).as_millis() >= 16;
            if frame_due {
                let draw_start = Instant::now();
                self.terminal
                    .draw(|f| mew_tui::ui::draw(f, &mut self.app))
                    .ok();
                let draw_ms = draw_start.elapsed().as_millis();
                if self.recording {
                    let rasterize_start = Instant::now();
                    self.capture_frame();
                    let rasterize_ms = rasterize_start.elapsed().as_millis();
                    info!(
                        frame = self.frames.len(),
                        draw_ms,
                        rasterize_ms,
                        ms_since_last_frame = now.duration_since(last_frame).as_millis(),
                        "captured frame"
                    );
                    last_frame = Instant::now();
                }
            }
            if now > deadline {
                warn!(
                    timeout_ms,
                    elapsed_ms = start.elapsed().as_millis(),
                    total_agent_events,
                    frames_captured = self.frames.len() - initial_frame_count,
                    "wait_turn timed out"
                );
                bail!("wait_turn timed out after {timeout_ms}ms");
            }
            // Only sleep when no events are ready; otherwise keep pace with the
            // actual stream instead of adding artificial delay.
            if !had_event {
                tokio::time::sleep(Duration::from_millis(4)).await;
            }
        }
        info!(
            elapsed_ms = start.elapsed().as_millis(),
            total_agent_events,
            frames_captured = self.frames.len() - initial_frame_count,
            "wait_turn: turn finished"
        );
        // Ensure the final completed frame is captured before returning, so a
        // following `pause` duplicates the finished response rather than a
        // mid-stream frame.
        self.poll_events();
        self.terminal
            .draw(|f| mew_tui::ui::draw(f, &mut self.app))
            .ok();
        if self.recording {
            self.capture_frame();
        }
        Ok(())
    }

    fn poll_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                mew_tui::Event::Agent(ev) => self.app.handle_agent_event(ev),
                mew_tui::Event::Tick => {
                    self.app.tick();
                    self.app.clear_expired_alerts();
                }
                mew_tui::Event::Input(_) | mew_tui::Event::Quit => {}
            }
        }
        while let Ok(msg) = self.notify_rx.try_recv() {
            self.app.apply_daemon_notification(&msg);
        }
    }

    async fn dispatch_action(&mut self, action: mew_tui::events::Action) {
        debug!(?action, "dispatching action");
        let mut target = crate::runtime::daemon::DaemonTarget::new(self.client.clone());
        let mut should_break = false;
        let plugin_info = Arc::new(std::sync::Mutex::new(crate::PluginInfo {
            active_persona: None,
        }));
        let mut cx = crate::runtime::Ctx {
            app: &mut self.app,
            target: &mut target,
            event_loop: &self.event_loop,
            should_break: &mut should_break,
            cat: None,
            loaded_personas: &[],
            plugin_info: &plugin_info,
        };
        let _flow = crate::runtime::handle_action(&mut cx, action).await;
    }

    fn capture_frame_raw(&mut self) {
        let app = &mut self.app;
        self.terminal
            .draw(|f| mew_tui::ui::draw(f, app))
            .expect("draw");
        let buf = self.terminal.backend().buffer();
        let pixmap = self
            .rasterizer
            .rasterize(buf, &mew_raster::RasterOptions::default());
        trace!(frame = self.frames.len(), "captured frame");
        self.frames.push(pixmap);
    }
}

async fn wait_for_session_ready(
    notify_rx: &mut tokio::sync::mpsc::Receiver<ServerMessage>,
    timeout: Duration,
) -> Result<(String, Option<String>, Option<String>)> {
    let deadline = Instant::now() + timeout;
    debug!(?timeout, "waiting for SessionReady");
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, notify_rx.recv()).await {
            Ok(Some(ServerMessage::SessionReady {
                session_id,
                model,
                provider,
                ..
            })) => {
                debug!(%session_id, ?model, ?provider, "received SessionReady");
                return Ok((session_id, model, provider));
            }
            Ok(Some(other)) => {
                trace!(?other, "ignoring non-SessionReady message while waiting");
                continue;
            }
            Ok(None) => bail!("daemon notification channel closed before SessionReady"),
            Err(_) => bail!("daemon did not send SessionReady within {:?}", timeout),
        }
    }
}

impl Backend for DaemonBackend {
    fn render(&mut self) -> String {
        let app = &mut self.app;
        self.terminal
            .draw(|f| mew_tui::ui::draw(f, app))
            .expect("draw");
        buffer_to_string(self.terminal.backend().buffer())
    }

    fn screenshot(&mut self, path: &str) -> io::Result<()> {
        let app = &mut self.app;
        self.terminal
            .draw(|f| mew_tui::ui::draw(f, app))
            .expect("draw");
        let buf = self.terminal.backend().buffer();
        let png_bytes = self
            .rasterizer
            .to_png(buf, &mew_raster::RasterOptions::default());
        std::fs::write(path, png_bytes)
    }

    fn send_key(&mut self, key: KeyEvent) {
        let event = crossterm::event::Event::Key(key);
        if let Some(action) = mew_tui::events::handle_input_event(&mut self.app, event) {
            // Submit actions are handled by the async send_text method; ignore
            // them here to keep the sync trait free of nested async work.
            let _ = action;
        }
        self.capture_frame();
    }

    fn type_str(&mut self, text: &str) {
        // Type without per-character captures; rasterizing every keystroke is
        // too slow for long strings.
        for ch in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let event = crossterm::event::Event::Key(key);
            if let Some(_action) = mew_tui::events::handle_input_event(&mut self.app, event) {
                // Character keys do not produce actions in normal mode.
            }
        }
        self.capture_frame();
    }

    fn send_text(&mut self, text: &str) {
        // Sync trait version: type the text and submit via Enter, but do not
        // wait for the async turn. Daemon scripts should use the `send` verb
        // which calls the async method instead.
        self.type_str(text);
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let event = crossterm::event::Event::Key(key);
        if let Some(_action) = mew_tui::events::handle_input_event(&mut self.app, event) {
            // Submits are handled by the async send_text; ignore here.
        }
        self.capture_frame();
    }

    fn wait_turn(&mut self, _timeout_ms: u64) -> Result<(), String> {
        // Sync trait version is a no-op for the daemon backend; use the async
        // wait_turn method in daemon scripts.
        Ok(())
    }

    fn is_streaming(&self) -> bool {
        self.app.streaming
    }

    fn poll_events(&mut self) {
        self.poll_events();
    }

    fn start_recording(&mut self) {
        self.recording = true;
        self.frames.clear();
        self.capture_frame();
    }

    fn stop_recording(&mut self) {
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

    fn as_local_backend_mut(&mut self) -> Option<&mut mew_tui::harness::LocalBackend> {
        None
    }

    fn as_local_backend_ref(&self) -> Option<&mew_tui::harness::LocalBackend> {
        None
    }
}

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

// ---------------------------------------------------------------------------
// Daemon script interpreter
// ---------------------------------------------------------------------------

async fn run_script_daemon_file(
    script_path: &Path,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
    url: &str,
) -> Result<String> {
    let script = std::fs::read_to_string(script_path)
        .with_context(|| format!("failed to read script file: {}", script_path.display()))?;
    run_script_daemon(&script, screenshot_dir, mp4, fps, width, height, url).await
}

/// Run a line-based script against a daemon-connected backend.
async fn run_script_daemon(
    script: &str,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
    url: &str,
) -> Result<String> {
    info!(
        url,
        width,
        height,
        ?screenshot_dir,
        ?mp4,
        fps,
        "starting daemon script"
    );
    let mut backend = DaemonBackend::connect(url, width, height).await?;
    let mut out = String::new();
    let mut frame_num: u32 = 0;
    let mut active_screenshot_dir: Option<std::path::PathBuf> =
        screenshot_dir.map(std::path::Path::to_path_buf);

    if let Some(dir) = screenshot_dir {
        info!(dir = %dir.display(), "creating screenshot directory");
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create screenshot dir: {}", dir.display()))?;
    }

    // If --mp4 is given, record every frame.
    if mp4.is_some() {
        info!("starting mp4 recording");
        backend.start_recording();
    }

    for (i, raw) in script.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        info!(line_no = i + 1, verb = %line, "executing script line");
        let result = exec_verb_daemon(
            &mut backend,
            line,
            &mut active_screenshot_dir,
            &mut frame_num,
        )
        .await;
        match result {
            Ok(text) => {
                if !text.is_empty() {
                    print!("{text}");
                    out.push_str(&text);
                }
            }
            Err(e) => {
                let err = format!("line {}: !! {}\n", i + 1, e);
                print!("{err}");
                out.push_str(&err);
            }
        }
        io::stdout().flush().ok();
    }

    if let Some(mp4_path) = mp4 {
        info!("stopping mp4 recording");
        backend.stop_recording();
        match backend.encode_mp4(mp4_path, fps) {
            Ok(_) => {
                let msg = format!("--- video saved to {mp4_path} ---\n");
                info!(path = %mp4_path, "video saved");
                print!("{msg}");
                out.push_str(&msg);
            }
            Err(e) => {
                let msg = format!("!! video encoding failed: {e}\n");
                warn!(path = %mp4_path, error = %e, "video encoding failed");
                print!("{msg}");
                out.push_str(&msg);
            }
        }
        io::stdout().flush().ok();
    }

    info!(
        output_len = out.len(),
        total_frames = backend.frames.len(),
        "daemon script finished"
    );
    Ok(out)
}

async fn run_interactive_daemon(
    width: u16,
    height: u16,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
    url: &str,
) -> Result<()> {
    info!(
        url,
        width,
        height,
        ?screenshot_dir,
        ?mp4,
        fps,
        "starting interactive daemon capture"
    );
    let mut backend = DaemonBackend::connect(url, width, height).await?;
    let mut frame_num: u32 = 0;
    let mut active_screenshot_dir = screenshot_dir.map(std::path::PathBuf::from);

    if let Some(dir) = screenshot_dir {
        info!(dir = %dir.display(), "creating screenshot directory");
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create screenshot dir: {}", dir.display()))?;
    }

    if mp4.is_some() {
        info!("starting mp4 recording");
        backend.start_recording();
    }

    print_frame_daemon(
        &mut backend,
        active_screenshot_dir.as_deref(),
        &mut frame_num,
    );

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    for line in lines.by_ref() {
        let line = line.context("failed to read stdin")?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if matches!(trimmed, "quit" | "exit") {
            info!(verb = %trimmed, "quitting interactive daemon capture");
            break;
        }
        info!(verb = %trimmed, "executing interactive verb");
        match exec_verb_daemon(
            &mut backend,
            trimmed,
            &mut active_screenshot_dir,
            &mut frame_num,
        )
        .await
        {
            Ok(text) => {
                if !text.is_empty() {
                    print!("{text}");
                    io::stdout().flush().ok();
                }
            }
            Err(e) => {
                warn!(error = %e, "interactive verb failed");
                eprintln!("!! {e}");
            }
        }
        print_frame_daemon(
            &mut backend,
            active_screenshot_dir.as_deref(),
            &mut frame_num,
        );
    }

    if let Some(mp4_path) = mp4 {
        info!("stopping mp4 recording");
        backend.stop_recording();
        match backend.encode_mp4(mp4_path, fps) {
            Ok(_) => {
                info!(path = %mp4_path, "video saved");
                eprintln!("--- video saved to {mp4_path} ---");
            }
            Err(e) => {
                warn!(path = %mp4_path, error = %e, "video encoding failed");
                eprintln!("!! video encoding failed: {e}");
            }
        }
    }

    info!("interactive daemon capture finished");
    println!("--- bye ---");
    Ok(())
}

async fn exec_verb_daemon(
    backend: &mut DaemonBackend,
    line: &str,
    screenshot_dir: &mut Option<std::path::PathBuf>,
    frame_num: &mut u32,
) -> Result<String> {
    let line = line.trim();
    let (verb, rest) = match line.split_once(char::is_whitespace) {
        Some((v, r)) => (v, r.trim()),
        None => (line, ""),
    };
    debug!(%verb, rest, "dispatching daemon verb");

    let mut out = String::new();
    match verb {
        "size" | "resize" => match parse_size(rest) {
            Some((w, h)) => {
                backend.terminal.backend_mut().resize(w, h);
            }
            None => out.push_str(&format!("!! bad size '{rest}'\n")),
        },
        "type" => backend.type_str(rest),
        "key" => match mew_tui::harness::parse_key(rest) {
            Some(key) => {
                if key.code == KeyCode::Enter {
                    backend.send_text("").await?;
                    out.push_str("--- submitted ---\n");
                } else {
                    backend.send_key(key);
                }
            }
            None => out.push_str(&format!("!! unknown key '{rest}'\n")),
        },
        "submit" => {
            backend.send_text("").await?;
            out.push_str("--- submitted ---\n");
        }
        "send" => {
            let text = parse_quoted(rest).unwrap_or(rest);
            info!(text_len = text.len(), "send verb");
            backend.send_text(text).await?;
            out.push_str(&format!("--- sent \"{text}\" ---\n"));
        }
        "wait_turn" => {
            let timeout_ms: u64 = rest.trim_end_matches("ms").parse().unwrap_or(30_000);
            info!(timeout_ms, "wait_turn verb");
            backend.wait_turn(timeout_ms).await?;
            out.push_str("--- turn finished ---\n");
        }
        "expect" => {
            let text = parse_quoted(rest).unwrap_or(rest);
            let rendered = backend.render();
            if !rendered.contains(text) {
                bail!("expected text not found: {text}");
            }
            out.push_str(&format!("--- expect ok \"{text}\" ---\n"));
        }
        "say" | "error" | "settings" | "settings_config" => {
            out.push_str(&format!(
                "!! '{verb}' is not available in daemon-capture mode (use the real daemon turn flow with send/wait_turn)\n"
            ));
        }
        "snapshot" => {
            let label = if rest.is_empty() {
                String::new()
            } else {
                format!(" {rest}")
            };
            out.push_str(&format!("--- snapshot{label} ---\n"));
            out.push_str(&backend.render());
            out.push_str("---\n");
        }
        "screenshot" => {
            if rest.is_empty() {
                out.push_str("!! screenshot requires a file path\n");
            } else {
                let path = rest.trim_matches('"');
                info!(path, "screenshot verb");
                match backend.screenshot(path) {
                    Ok(()) => out.push_str(&format!("--- screenshot saved to {rest} ---\n")),
                    Err(e) => {
                        warn!(path, error = %e, "screenshot failed");
                        out.push_str(&format!("!! screenshot failed: {e}\n"));
                    }
                }
            }
        }
        "screenshot_dir" => {
            let path = rest.trim().trim_matches('"');
            if path.is_empty() {
                out.push_str("!! screenshot_dir requires a directory path\n");
            } else {
                info!(path, "screenshot_dir verb");
                std::fs::create_dir_all(path)?;
                *screenshot_dir = Some(path.into());
                out.push_str(&format!("--- screenshot dir set to {path} ---\n"));
            }
        }
        "start_recording" => {
            info!("start_recording verb");
            backend.start_recording();
            out.push_str("--- recording started ---\n");
        }
        "stop_recording" => {
            info!("stop_recording verb");
            backend.stop_recording();
            out.push_str(&format!(
                "--- recording stopped ({} frames) ---\n",
                backend.frame_count()
            ));
        }
        "pause" => {
            let fps: u32 = 30;
            match rest.trim_end_matches("ms").parse::<u32>() {
                Ok(ms) => {
                    let frames = (ms as f32 / (1000.0 / fps as f32)).round() as usize;
                    info!(ms, frames, "pause verb: duplicating frames");
                    backend.duplicate_last_frame(frames);
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
                info!(path, fps, "record verb");
                match backend.encode_mp4(path, fps) {
                    Ok(_) => out.push_str(&format!("--- video saved to {path} ---\n")),
                    Err(e) => {
                        warn!(path, fps, error = %e, "record failed");
                        out.push_str(&format!("!! record failed: {e}\n"));
                    }
                }
            }
        }
        "help" | "?" => {
            out.push_str("daemon verbs: type, key, submit, send, wait_turn, expect, ");
            out.push_str("snapshot, screenshot, screenshot_dir, ");
            out.push_str("start_recording, stop_recording, pause, record, size, quit\n");
        }
        "quit" | "exit" => {
            out.push_str("--- bye ---\n");
        }
        other => out.push_str(&format!("!! unknown verb '{other}'\n")),
    }

    // If a screenshot_dir is active, write a numbered PNG after every verb.
    if let Some(dir) = screenshot_dir.as_ref() {
        *frame_num += 1;
        let filename = format!("frame_{:04}.png", *frame_num);
        let png_path = dir.join(&filename);
        match backend.screenshot(png_path.to_str().unwrap()) {
            Ok(()) => {
                out.push_str(&format!("--- screenshot: {} ---\n", png_path.display()));
            }
            Err(e) => {
                out.push_str(&format!("!! screenshot failed: {e}\n"));
            }
        }
    }

    Ok(out)
}

fn print_frame_daemon(
    backend: &mut DaemonBackend,
    screenshot_dir: Option<&Path>,
    frame_num: &mut u32,
) {
    trace!("printing daemon frame");
    println!("--- frame ---");
    print!("{}", backend.render());
    println!("---");

    if let Some(dir) = screenshot_dir {
        *frame_num += 1;
        let filename = format!("frame_{:04}.png", *frame_num);
        let png_path = dir.join(&filename);
        match backend.screenshot(png_path.to_str().unwrap()) {
            Ok(()) => {
                trace!(path = %png_path.display(), "daemon screenshot saved");
                println!("--- screenshot: {} ---", png_path.display());
            }
            Err(e) => {
                warn!(path = %png_path.display(), error = %e, "daemon screenshot failed");
                eprintln!("!! screenshot failed: {e}");
            }
        }
    }

    io::stdout().flush().ok();
}

fn parse_size(s: &str) -> Option<(u16, u16)> {
    let mut parts = s
        .split(|c: char| c == 'x' || c.is_whitespace())
        .filter(|p| !p.is_empty());
    let w = parts.next()?.parse().ok()?;
    let h = parts.next()?.parse().ok()?;
    Some((w, h))
}

fn parse_quoted(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

/// Human-readable name for an `AgentEvent` variant (no sensitive payload).
fn agent_event_name(ev: &mew_agent::AgentEvent) -> &'static str {
    match ev {
        mew_agent::AgentEvent::Provider(_) => "Provider",
        mew_agent::AgentEvent::PermissionRequest { .. } => "PermissionRequest",
        mew_agent::AgentEvent::ToolStart { .. } => "ToolStart",
        mew_agent::AgentEvent::ToolEnd { .. } => "ToolEnd",
        mew_agent::AgentEvent::PartUpdated { .. } => "PartUpdated",
        mew_agent::AgentEvent::ToolProgress { .. } => "ToolProgress",
        mew_agent::AgentEvent::Error(_) => "Error",
        mew_agent::AgentEvent::WorkspacePermissionRequest { .. } => "WorkspacePermissionRequest",
        mew_agent::AgentEvent::SubagentStart { .. } => "SubagentStart",
        mew_agent::AgentEvent::SubagentProgress { .. } => "SubagentProgress",
        mew_agent::AgentEvent::SubagentStatus { .. } => "SubagentStatus",
        mew_agent::AgentEvent::SubagentEnd { .. } => "SubagentEnd",
        mew_agent::AgentEvent::SubagentPermissionRequest { .. } => "SubagentPermissionRequest",
        mew_agent::AgentEvent::AskUser { .. } => "AskUser",
        mew_agent::AgentEvent::PlanApprovalRequest { .. } => "PlanApprovalRequest",
        mew_agent::AgentEvent::TodosUpdated { .. } => "TodosUpdated",
        mew_agent::AgentEvent::PersonaSwitchRequested { .. } => "PersonaSwitchRequested",
        mew_agent::AgentEvent::JobUpdate { .. } => "JobUpdate",
        mew_agent::AgentEvent::FileDelta { .. } => "FileDelta",
        mew_agent::AgentEvent::FlaggedFilesChanged { .. } => "FlaggedFilesChanged",
        mew_agent::AgentEvent::GoalProposed { .. } => "GoalProposed",
    }
}

/// Human-readable type name for a `ServerMessage` variant (no payload).
fn server_message_type(msg: &mew_protocol::ServerMessage) -> &'static str {
    match msg {
        // Remote authentication is a daemon/web concern; the standalone TUI
        // never renders this handshake acknowledgement.
        mew_protocol::ServerMessage::RemoteReady { .. } => "RemoteReady",
        mew_protocol::ServerMessage::SessionReady { .. } => "SessionReady",
        mew_protocol::ServerMessage::Error { .. } => "Error",
        mew_protocol::ServerMessage::Provider { .. } => "Provider",
        mew_protocol::ServerMessage::UserMessage { .. } => "UserMessage",
        mew_protocol::ServerMessage::ToolStart { .. } => "ToolStart",
        mew_protocol::ServerMessage::ToolEnd { .. } => "ToolEnd",
        mew_protocol::ServerMessage::PartUpdated { .. } => "PartUpdated",
        mew_protocol::ServerMessage::ToolProgress { .. } => "ToolProgress",
        mew_protocol::ServerMessage::ErrorEvent { .. } => "ErrorEvent",
        mew_protocol::ServerMessage::PermissionRequest { .. } => "PermissionRequest",
        mew_protocol::ServerMessage::WorkspacePermissionRequest { .. } => {
            "WorkspacePermissionRequest"
        }
        mew_protocol::ServerMessage::AskUserRequest { .. } => "AskUserRequest",
        mew_protocol::ServerMessage::PlanApprovalRequest { .. } => "PlanApprovalRequest",
        mew_protocol::ServerMessage::GoalProposed { .. } => "GoalProposed",
        mew_protocol::ServerMessage::SubagentStart { .. } => "SubagentStart",
        mew_protocol::ServerMessage::SubagentStatus { .. } => "SubagentStatus",
        mew_protocol::ServerMessage::SubagentEnd { .. } => "SubagentEnd",
        mew_protocol::ServerMessage::SubagentPermissionRequest { .. } => {
            "SubagentPermissionRequest"
        }
        mew_protocol::ServerMessage::TodosUpdated { .. } => "TodosUpdated",
        mew_protocol::ServerMessage::PersonaSwitchRequested { .. } => "PersonaSwitchRequested",
        mew_protocol::ServerMessage::JobUpdate { .. } => "JobUpdate",
        mew_protocol::ServerMessage::SessionList { .. } => "SessionList",
        mew_protocol::ServerMessage::SessionHistory { .. } => "SessionHistory",
        mew_protocol::ServerMessage::SessionTitleChanged { .. } => "SessionTitleChanged",
        mew_protocol::ServerMessage::SessionSummaryChanged { .. } => "SessionSummaryChanged",
        mew_protocol::ServerMessage::SessionAlert { .. } => "SessionAlert",
        mew_protocol::ServerMessage::ModelSwitched { .. } => "ModelSwitched",
        mew_protocol::ServerMessage::ThinkingVariantChanged { .. } => "ThinkingVariantChanged",
        mew_protocol::ServerMessage::PermissionModeChanged { .. } => "PermissionModeChanged",
        mew_protocol::ServerMessage::SessionMetaChanged { .. } => "SessionMetaChanged",
        mew_protocol::ServerMessage::FlaggedFilesChanged { .. } => "FlaggedFilesChanged",
        mew_protocol::ServerMessage::SessionActivityChanged { .. } => "SessionActivityChanged",
        mew_protocol::ServerMessage::SessionStatsChanged { .. } => "SessionStatsChanged",
        mew_protocol::ServerMessage::SessionUsageChanged { .. } => "SessionUsageChanged",
        mew_protocol::ServerMessage::SessionAttentionChanged { .. } => "SessionAttentionChanged",
        mew_protocol::ServerMessage::ModelList { .. } => "ModelList",
        mew_protocol::ServerMessage::Pong { .. } => "Pong",
        mew_protocol::ServerMessage::PersonaList { .. } => "PersonaList",
        mew_protocol::ServerMessage::PersonaSwitched { .. } => "PersonaSwitched",
        mew_protocol::ServerMessage::ProjectList { .. } => "ProjectList",
        mew_protocol::ServerMessage::BrowserSnapshot { .. } => "BrowserSnapshot",
        mew_protocol::ServerMessage::BrowserScreenshot { .. } => "BrowserScreenshot",
        mew_protocol::ServerMessage::BrowserState { .. } => "BrowserState",
        mew_protocol::ServerMessage::BrowserError { .. } => "BrowserError",
        mew_protocol::ServerMessage::GroupList { .. } => "GroupList",
        mew_protocol::ServerMessage::GroupsChanged { .. } => "GroupsChanged",
        mew_protocol::ServerMessage::DirListing { .. } => "DirListing",
        // Folder browsing is a web/desktop concern and has no TUI state.
        mew_protocol::ServerMessage::FilesystemDirListing { .. } => "FilesystemDirListing",
        mew_protocol::ServerMessage::FilePreview { .. } => "FilePreview",
        mew_protocol::ServerMessage::GitStatusResult { .. } => "GitStatusResult",
        mew_protocol::ServerMessage::FsChanged { .. } => "FsChanged",
        mew_protocol::ServerMessage::SlashResult { .. } => "SlashResult",
        mew_protocol::ServerMessage::RequestResolved { .. } => "RequestResolved",
        mew_protocol::ServerMessage::SessionCleared => "SessionCleared",
        mew_protocol::ServerMessage::ClientAttached { .. } => "ClientAttached",
        mew_protocol::ServerMessage::ClientDetached { .. } => "ClientDetached",
        mew_protocol::ServerMessage::ControlYielded { .. } => "ControlYielded",
        mew_protocol::ServerMessage::TerminalOpened { .. } => "TerminalOpened",
        mew_protocol::ServerMessage::TerminalOutput { .. } => "TerminalOutput",
        mew_protocol::ServerMessage::TerminalExited { .. } => "TerminalExited",
        mew_protocol::ServerMessage::TerminalError { .. } => "TerminalError",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    async fn spawn_daemon_tcp(server: mew_daemon::DaemonServer) -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral");
        let addr = listener.local_addr().expect("local addr");
        drop(listener);
        let handle = tokio::spawn(async move {
            let _ = server.run_tcp(addr).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        (addr, handle)
    }

    fn fake_provider_builder(response: &'static str) -> mew_daemon::AgentBuilder {
        Arc::new(move |params: mew_daemon::AgentBuildParams| {
            let provider = mew_provider_fake::FakeProvider::new(
                mew_provider_fake::FakeProvider::text_response(response),
            );
            let session_id = params
                .session_id
                .strip_prefix("sess_")
                .and_then(|s| ulid::Ulid::from_string(s).ok());
            let agent = mew_agent::Agent::new(
                Arc::new(provider),
                Arc::new(mew_hooks::NopDispatcher),
                Some(params.writer),
                vec![],
                session_id,
            );
            Ok((
                agent,
                Some("fake-model".to_string()),
                Some("fake".to_string()),
            ))
        })
    }

    #[tokio::test]
    async fn test_daemon_capture_fake_provider_end_to_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = mew_daemon::DaemonServer::with_session_dir(
            fake_provider_builder("hello from fake provider"),
            dir.path().to_path_buf(),
        );
        let (addr, handle) = spawn_daemon_tcp(server).await;
        let url = format!("ws://{addr}");

        let output = run_script_daemon(
            "send \"hi\"\nwait_turn\nexpect \"hello from fake provider\"",
            None,
            None,
            30,
            80,
            24,
            &url,
        )
        .await
        .expect("run daemon script");

        handle.abort();
        assert!(
            output.contains("hello from fake provider"),
            "expected response in output:\n{output}"
        );
    }

    #[tokio::test]
    async fn test_daemon_capture_expect_fails_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = mew_daemon::DaemonServer::with_session_dir(
            fake_provider_builder("hello from fake provider"),
            dir.path().to_path_buf(),
        );
        let (addr, handle) = spawn_daemon_tcp(server).await;
        let url = format!("ws://{addr}");

        let result = run_script_daemon(
            "send \"hi\"\nwait_turn\nexpect \"nope\"",
            None,
            None,
            30,
            80,
            24,
            &url,
        )
        .await;

        handle.abort();
        let output = result.expect("run_script_daemon returns Ok with error line");
        assert!(
            output.contains("expected text not found"),
            "output: {output}"
        );
    }

    #[tokio::test]
    async fn test_daemon_capture_screenshot_dir_writes_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let screenshot_dir = dir.path().join("frames");
        let server = mew_daemon::DaemonServer::with_session_dir(
            fake_provider_builder("hello from fake provider"),
            dir.path().to_path_buf(),
        );
        let (addr, handle) = spawn_daemon_tcp(server).await;
        let url = format!("ws://{addr}");

        let script = format!(
            "send \"hi\"\nwait_turn\nscreenshot_dir \"{}\"\nsnapshot done",
            screenshot_dir.display()
        );
        let output = run_script_daemon(&script, None, None, 30, 80, 24, &url)
            .await
            .expect("run daemon script");

        handle.abort();
        assert!(output.contains("frame_0001.png"), "output: {output}");
        let png = screenshot_dir.join("frame_0001.png");
        assert!(png.exists(), "PNG file should exist: {png:?}");
    }

    #[tokio::test]
    async fn test_daemon_capture_harness_verb_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = mew_daemon::DaemonServer::with_session_dir(
            fake_provider_builder("hello from fake provider"),
            dir.path().to_path_buf(),
        );
        let (addr, handle) = spawn_daemon_tcp(server).await;
        let url = format!("ws://{addr}");

        let output = run_script_daemon("say hello", None, None, 30, 80, 24, &url)
            .await
            .expect("run daemon script");

        handle.abort();
        assert!(
            output.contains("not available in daemon-capture mode"),
            "output: {output}"
        );
    }

    #[tokio::test]
    async fn test_spinner_advances_while_streaming() {
        // A long response streams for ~500ms (fake provider: 4 chars per
        // delta, 10ms apart), long enough for the 16ms tick cadence in
        // wait_turn to advance the spinner several times.
        let dir = tempfile::tempdir().expect("tempdir");
        let server = mew_daemon::DaemonServer::with_session_dir(
            fake_provider_builder(
                "a fairly long response so that streaming lasts long enough for the \
                 spinner to visibly advance across several animation frames while \
                 wait_turn is capturing video frames at its sixty fps cadence",
            ),
            dir.path().to_path_buf(),
        );
        let (addr, handle) = spawn_daemon_tcp(server).await;
        let url = format!("ws://{addr}");

        let mut backend = DaemonBackend::connect(&url, 80, 24).await.expect("connect");
        Backend::start_recording(&mut backend);
        backend.send_text("hi").await.expect("send");
        backend.wait_turn(30_000).await.expect("wait_turn");
        handle.abort();

        assert!(
            backend.app.spinner_frame >= 1,
            "spinner should advance during a ~500ms streamed turn, got frame {}",
            backend.app.spinner_frame
        );
    }

    #[tokio::test]
    async fn test_daemon_capture_mp4_writes_video() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mp4_path = dir.path().join("demo.mp4");
        let mp4_str = mp4_path.to_str().unwrap();
        let server = mew_daemon::DaemonServer::with_session_dir(
            fake_provider_builder("hello from fake provider"),
            dir.path().to_path_buf(),
        );
        let (addr, handle) = spawn_daemon_tcp(server).await;
        let url = format!("ws://{addr}");

        let output = run_script_daemon(
            "send \"hi\"\nwait_turn\n",
            None,
            Some(mp4_str),
            30,
            80,
            24,
            &url,
        )
        .await
        .expect("run daemon script");

        handle.abort();
        if output.contains("video saved") {
            assert!(mp4_path.exists(), "MP4 should exist");
        } else {
            // ffmpeg may be unavailable; skip the file assertion gracefully.
            eprintln!("mp4 test skipped (ffmpeg unavailable?): {output}");
        }
    }
}
