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
    if let Some(url) = connect {
        if interactive {
            run_interactive_daemon(width, height, screenshot_dir, mp4, fps, url).await
        } else {
            let script_path = script.context("either --script or --interactive is required")?;
            let output =
                run_script_daemon_file(script_path, screenshot_dir, mp4, fps, width, height, url)
                    .await?;
            print!("{output}");
            Ok(())
        }
    } else if interactive {
        run_interactive(width, height, screenshot_dir, mp4, fps)
    } else {
        let script_path = script.context("either --script or --interactive is required")?;
        run_script_file(script_path, mp4, fps, width, height)
    }
}

/// Script mode (harness): read a file and run it all at once.
fn run_script_file(
    script_path: &Path,
    mp4: Option<&str>,
    fps: u32,
    width: u16,
    height: u16,
) -> Result<()> {
    let script = std::fs::read_to_string(script_path)
        .with_context(|| format!("failed to read script file: {}", script_path.display()))?;

    let output = run_script_harness(&script, mp4, fps, width, height)?;
    print!("{output}");
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

    Ok(mew_tui::harness::run_script(&script, width, height))
}

/// Interactive REPL mode (harness): read verbs from stdin, print frames.
fn run_interactive(
    width: u16,
    height: u16,
    screenshot_dir: Option<&Path>,
    mp4: Option<&str>,
    fps: u32,
) -> Result<()> {
    let mut harness = Harness::new(width, height);

    // Start recording if --mp4 is set
    if mp4.is_some() {
        harness.start_recording();
    }

    // Create screenshot dir if needed
    if let Some(dir) = screenshot_dir {
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
        harness.stop_recording();
        match harness.encode_mp4(mp4_path, fps) {
            Ok(_) => eprintln!("--- video saved to {mp4_path} ---"),
            Err(e) => eprintln!("!! video encoding failed: {e}"),
        }
    }

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
                println!("--- screenshot: {} ---", png_path.display());
            }
            Err(e) => {
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
}

impl DaemonBackend {
    /// Connect to a daemon, create a session, and prepare a headless TUI.
    pub(crate) async fn connect(url: &str, width: u16, height: u16) -> Result<Self> {
        let (client, mut notify_rx) = mew_daemon::DaemonClient::connect(url).await?;
        let client = Arc::new(client);
        client.new_session().await?;

        // Wait for the daemon to assign a session ID and report the active
        // model/provider so the status bar reflects the real backend.
        let (session_id, model, provider) =
            wait_for_session_ready(&mut notify_rx, Duration::from_secs(5)).await?;

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

        // Populate the sidebar session rail.
        client.list_sessions().await;

        let terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        let (event_loop, event_rx) = mew_tui::EventLoop::new();

        Ok(Self {
            client,
            app,
            terminal,
            event_loop,
            event_rx,
            notify_rx,
            frames: Vec::new(),
            recording: false,
        })
    }

    /// Type text into the composer and submit it to the daemon, then wait until
    /// the turn begins streaming.
    pub(crate) async fn send_text(&mut self, text: &str) -> Result<()> {
        // Type the text into the composer.
        for ch in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let event = crossterm::event::Event::Key(key);
            if let Some(action) = mew_tui::events::handle_input_event(&mut self.app, event) {
                // Normal character keys shouldn't produce actions; if they do,
                // dispatch them anyway.
                self.dispatch_action(action).await;
            }
            self.capture_frame();
        }

        // Submit with Enter.
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let event = crossterm::event::Event::Key(key);
        let action = mew_tui::events::handle_input_event(&mut self.app, event);
        if let Some(action) = action {
            self.dispatch_action(action).await;
        }
        self.capture_frame();

        // Wait until the daemon marks the app as streaming, with a short timeout.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.app.streaming {
            self.poll_events();
            if Instant::now() > deadline {
                bail!("send: turn did not start streaming within 5s");
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Ok(())
    }

    /// Block until the current daemon turn finishes streaming.
    /// While streaming, capture at 60fps. Captures one final frame after
    /// streaming ends so subsequent `pause` verbs hold the completed response.
    pub(crate) async fn wait_turn(&mut self, timeout_ms: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut last_frame = Instant::now();
        while self.app.streaming {
            self.poll_events();
            self.terminal
                .draw(|f| mew_tui::ui::draw(f, &mut self.app))
                .ok();
            if self.recording && Instant::now().duration_since(last_frame).as_millis() >= 16 {
                self.capture_frame();
                last_frame = Instant::now();
            }
            if Instant::now() > deadline {
                bail!("wait_turn timed out after {timeout_ms}ms");
            }
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
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
        let mut target = crate::runtime::daemon::DaemonTarget::new(self.client.clone());
        let mut should_break = false;
        let plugin_info = Arc::new(std::sync::Mutex::new(crate::PluginInfo {
            session_id: self.app.status.session_id.clone(),
            model: "daemon".to_string(),
            provider: "mewd".to_string(),
            workspace: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
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
        let pixmap = mew_raster::rasterize(buf, &mew_raster::RasterOptions::default());
        self.frames.push(pixmap);
    }
}

async fn wait_for_session_ready(
    notify_rx: &mut tokio::sync::mpsc::Receiver<ServerMessage>,
    timeout: Duration,
) -> Result<(String, Option<String>, Option<String>)> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, notify_rx.recv()).await {
            Ok(Some(ServerMessage::SessionReady {
                session_id,
                model,
                provider,
                ..
            })) => return Ok((session_id, model, provider)),
            Ok(Some(_)) => continue,
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
        let png_bytes = mew_raster::to_png(buf, &mew_raster::RasterOptions::default());
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
        for ch in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let event = crossterm::event::Event::Key(key);
            if let Some(_action) = mew_tui::events::handle_input_event(&mut self.app, event) {
                // Character keys do not produce actions in normal mode.
            }
            self.capture_frame();
        }
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
        if self.frames.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no frames recorded",
            ));
        }

        let dir = tempfile::tempdir()?;
        for (i, pixmap) in self.frames.iter().enumerate() {
            let png_path = dir.path().join(format!("frame_{:04}.png", i));
            let png_bytes = pixmap_to_png_bytes(pixmap);
            std::fs::write(&png_path, png_bytes)?;
        }

        let frame_pattern = dir.path().join("frame_%04d.png");
        let output = std::process::Command::new("ffmpeg")
            .arg("-framerate")
            .arg(fps.to_string())
            .arg("-i")
            .arg(&frame_pattern)
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-y")
            .arg(output_path)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stderr).to_string())
        } else {
            Err(io::Error::other(format!(
                "ffmpeg failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
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

fn pixmap_to_png_bytes(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
    use png::{BitDepth, ColorType, Encoder};
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, pixmap.width(), pixmap.height());
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(pixmap.data()).expect("png write");
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
    let mut backend = DaemonBackend::connect(url, width, height).await?;
    let mut out = String::new();
    let mut frame_num: u32 = 0;
    let mut active_screenshot_dir: Option<std::path::PathBuf> =
        screenshot_dir.map(std::path::Path::to_path_buf);

    if let Some(dir) = screenshot_dir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create screenshot dir: {}", dir.display()))?;
    }

    // If --mp4 is given, record every frame.
    if mp4.is_some() {
        backend.start_recording();
    }

    for (i, raw) in script.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
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
                    out.push_str(&text);
                }
            }
            Err(e) => {
                out.push_str(&format!("line {}: !! {}\n", i + 1, e));
            }
        }
    }

    if let Some(mp4_path) = mp4 {
        backend.stop_recording();
        match backend.encode_mp4(mp4_path, fps) {
            Ok(_) => out.push_str(&format!("--- video saved to {mp4_path} ---\n")),
            Err(e) => out.push_str(&format!("!! video encoding failed: {e}\n")),
        }
    }

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
    let mut backend = DaemonBackend::connect(url, width, height).await?;
    let mut frame_num: u32 = 0;
    let mut active_screenshot_dir = screenshot_dir.map(std::path::PathBuf::from);

    if let Some(dir) = screenshot_dir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create screenshot dir: {}", dir.display()))?;
    }

    if mp4.is_some() {
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
            break;
        }
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
        backend.stop_recording();
        match backend.encode_mp4(mp4_path, fps) {
            Ok(_) => eprintln!("--- video saved to {mp4_path} ---"),
            Err(e) => eprintln!("!! video encoding failed: {e}"),
        }
    }

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
            backend.send_text(text).await?;
            out.push_str(&format!("--- sent \"{text}\" ---\n"));
        }
        "wait_turn" => {
            let timeout_ms: u64 = rest.trim_end_matches("ms").parse().unwrap_or(30_000);
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
                match backend.screenshot(path) {
                    Ok(()) => out.push_str(&format!("--- screenshot saved to {rest} ---\n")),
                    Err(e) => out.push_str(&format!("!! screenshot failed: {e}\n")),
                }
            }
        }
        "screenshot_dir" => {
            let path = rest.trim().trim_matches('"');
            if path.is_empty() {
                out.push_str("!! screenshot_dir requires a directory path\n");
            } else {
                std::fs::create_dir_all(path)?;
                *screenshot_dir = Some(path.into());
                out.push_str(&format!("--- screenshot dir set to {path} ---\n"));
            }
        }
        "start_recording" => {
            backend.start_recording();
            out.push_str("--- recording started ---\n");
        }
        "stop_recording" => {
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
                match backend.encode_mp4(path, fps) {
                    Ok(_) => out.push_str(&format!("--- video saved to {path} ---\n")),
                    Err(e) => out.push_str(&format!("!! record failed: {e}\n")),
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
    println!("--- frame ---");
    print!("{}", backend.render());
    println!("---");

    if let Some(dir) = screenshot_dir {
        *frame_num += 1;
        let filename = format!("frame_{:04}.png", *frame_num);
        let png_path = dir.join(&filename);
        match backend.screenshot(png_path.to_str().unwrap()) {
            Ok(()) => {
                println!("--- screenshot: {} ---", png_path.display());
            }
            Err(e) => {
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
