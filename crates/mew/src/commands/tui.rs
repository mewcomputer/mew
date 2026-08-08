//! TUI command implementations extracted from `main.rs`.
//!
//! These functions own the terminal UI lifecycle: starting the TUI event loop
//! (`run_tui`), connecting to a daemon frontend (`chat_with_daemon`), and
//! small helpers for mouse capture, clipboard, and context file display.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Maximum agent events processed per drain batch during streaming.
/// Prevents a burst of deltas from blocking the frame — text appears
/// incrementally instead of all at once.
const STREAMING_DRAIN_LIMIT: u32 = 4;

/// How long to wait for a spawned daemon to become healthy before giving up.
const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Run the TUI connected to a mew daemon. The daemon owns the agent;
/// the TUI is a pure frontend that sends prompts and receives AgentEvents.
pub(crate) async fn chat_with_daemon(connect_url: &str, attach: Option<&str>) -> Result<()> {
    let (client, mut notify_rx) = mew_daemon::DaemonClient::connect(connect_url).await?;
    let client = Arc::new(client);

    if let Some(session_id) = attach {
        client.attach_session(session_id).await?;
    } else {
        client.new_session().await?;
    }

    let mut app = mew_tui::App::new();
    app.daemon_mode = true;
    // Load skills for autocomplete and inline skill-reference resolution.
    // Uses the same standard locations as the daemon's skill loader.
    let cwd = std::env::current_dir().unwrap_or_default();
    let skill_loader = mew_skills::Loader::new(cwd);
    app.skill_catalog = skill_loader.load().unwrap_or_default();
    // Set the session ID from the daemon client.
    if let Some(sid) = client.session_id().await {
        app.status.session_id = sid;
    }
    // Load theme from state/config (best-effort; daemon client may not
    // have full config available).
    let state = mew_config::load_state().unwrap_or_default();
    let cfg = mew_config::load().unwrap_or_default();
    let theme_name = if !state.theme.is_empty() {
        &state.theme
    } else {
        &cfg.tui.theme
    };
    app.theme = mew_tui::theme::Theme::load(theme_name);
    app.status.model = "daemon".to_string();
    app.status.provider = "mewd".to_string();
    app.recent_models = state.recent_models.clone();

    // Request the session list so the sidebar rail is populated immediately.
    client.list_sessions().await?;
    // Request the model list so the model picker and thinking-variant
    // picker are populated in daemon mode.
    client.list_models().await?;

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste,
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let (event_loop, mut event_rx) = mew_tui::EventLoop::new();
    event_loop.spawn();
    let event_loop = Arc::new(event_loop);

    let mut last_event_was_tick = false;

    // Construct PluginInfo — the daemon updates active_persona only.
    let plugin_info = Arc::new(std::sync::Mutex::new(crate::PluginInfo {
        active_persona: None,
    }));

    let result = loop {
        if !last_event_was_tick || app.needs_redraw() {
            if let Err(e) = terminal.draw(|f| mew_tui::ui::draw(f, &mut app)) {
                break Err(anyhow::anyhow!("draw error: {}", e));
            }
            mew_tui::title::set_terminal_title(mew_tui::title::title_for_streaming(app.streaming));
        }

        let event = tokio::select! {
            ev = event_rx.recv() => match ev {
                Some(e) => e,
                None => break Ok(()),
            },
            msg = notify_rx.recv() => {
                // Daemon session-management notification. Update App state
                // via the reducer, then continue the loop (don't block on
                // input).
                if let Some(msg) = msg {
                    app.apply_daemon_notification(&msg);
                }
                continue;
            }
        };

        last_event_was_tick = matches!(event, mew_tui::Event::Tick);

        let mut should_break = false;
        match event {
            mew_tui::Event::Input(crossterm_event) => {
                if let Some(action) = mew_tui::events::handle_input_event(&mut app, crossterm_event)
                {
                    let mut target = crate::runtime::daemon::DaemonTarget::new(client.clone());
                    let mut cx = crate::runtime::Ctx {
                        app: &mut app,
                        target: &mut target,
                        event_loop: &event_loop,
                        should_break: &mut should_break,
                        cat: None,
                        loaded_personas: &[],
                        plugin_info: &plugin_info,
                    };
                    let _flow = crate::runtime::handle_action(&mut cx, action).await;
                }
            }
            mew_tui::Event::Agent(agent_event) => {
                app.handle_agent_event(agent_event);
                // After processing agent events, check if a turn just finished
                // and there are queued messages to send.
                if app.pending_queued_send {
                    app.pending_queued_send = false;
                    if let Some(text) = app.pop_queued_message() {
                        let mut target = crate::runtime::daemon::DaemonTarget::new(client.clone());
                        let mut cx = crate::runtime::Ctx {
                            app: &mut app,
                            target: &mut target,
                            event_loop: &event_loop,
                            should_break: &mut should_break,
                            cat: None,
                            loaded_personas: &[],
                            plugin_info: &plugin_info,
                        };
                        crate::runtime::handle_action(
                            &mut cx,
                            mew_tui::events::Action::Submit(text),
                        )
                        .await;
                    }
                }
            }
            mew_tui::Event::Quit => should_break = true,
            mew_tui::Event::Tick => {
                app.tick();
                app.clear_expired_alerts();
            }
        }

        if should_break {
            break Ok(());
        }

        // Drain remaining events before next render (coalesces rapid input).
        // When streaming, limit agent events per drain batch so text
        // appears incrementally instead of all at once after a burst.
        let mut agent_drain_count = 0u32;
        let mut queued_actions: Vec<mew_tui::events::Action> = Vec::new();
        'drain: while let Ok(event) = event_rx.try_recv() {
            if !matches!(event, mew_tui::Event::Tick) {
                last_event_was_tick = false;
            }
            match event {
                mew_tui::Event::Input(crossterm_event) => {
                    if let crossterm::event::Event::Mouse(ref mouse) = crossterm_event {
                        match mouse.kind {
                            crossterm::event::MouseEventKind::ScrollUp => {
                                app.scroll_up(1);
                                continue;
                            }
                            crossterm::event::MouseEventKind::ScrollDown => {
                                app.scroll_down(1);
                                continue;
                            }
                            _ => {}
                        }
                    }
                    if let Some(action) =
                        mew_tui::events::handle_input_event(&mut app, crossterm_event)
                    {
                        queued_actions.push(action);
                    }
                }
                mew_tui::Event::Agent(event) => {
                    app.handle_agent_event(event);
                    agent_drain_count += 1;
                    if app.streaming && agent_drain_count >= STREAMING_DRAIN_LIMIT {
                        break 'drain;
                    }
                }
                mew_tui::Event::Tick => {
                    app.tick();
                }
                mew_tui::Event::Quit => {
                    should_break = true;
                    break 'drain;
                }
            }
        }

        // If a turn just finished and there are queued messages, submit the
        // oldest one as a new turn.
        if app.pending_queued_send {
            app.pending_queued_send = false;
            if let Some(text) = app.pop_queued_message() {
                queued_actions.push(mew_tui::events::Action::Submit(text));
            }
        }

        // Replay queued actions through handle_action.
        for action in queued_actions {
            let mut target = crate::runtime::daemon::DaemonTarget::new(client.clone());
            let mut cx = crate::runtime::Ctx {
                app: &mut app,
                target: &mut target,
                event_loop: &event_loop,
                should_break: &mut should_break,
                cat: None,
                loaded_personas: &[],
                plugin_info: &plugin_info,
            };
            let flow = crate::runtime::handle_action(&mut cx, action).await;
            if matches!(flow, crate::runtime::Flow::Quit) {
                break;
            }
        }

        if should_break {
            break Ok(());
        }

        // Handle pending mouse-capture toggle (needs a Terminal reference).
        if app.pending_mouse_toggle {
            app.pending_mouse_toggle = false;
            toggle_mouse_capture(&mut app, &mut terminal).await;
        }
    };

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
    )?;
    mew_tui::title::set_terminal_title("mew");
    result
}

pub(crate) async fn chat_cmd(
    cfg: mew_config::Config,
    provider_flag: String,
    model_flag: Option<String>,
    _variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<()> {
    // Local mode is sunset: the TUI always talks to a daemon. Spawn one if
    // none is running, then connect. The daemon owns the agent and serializes
    // turns, so concurrent-turn races cannot happen. The thinking variant is
    // a per-session UI setting and is not forwarded to the daemon.
    let _ = cfg;
    let url = spawn_or_connect_daemon(&provider_flag, model_flag.as_deref(), raw, mode).await?;
    chat_with_daemon(&url, None).await
}

/// Bind a loopback TCP port and return it, so the spawned daemon can listen
/// on `127.0.0.1:<port>` without colliding with anything.
fn allocate_loopback_port() -> Result<std::net::SocketAddr> {
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", 0)).context("bind loopback port for daemon")?;
    Ok(listener.local_addr()?)
}

/// Probe whether a daemon is already healthy at the given WebSocket URL. A
/// successful connect (and immediate drop) means it's up.
async fn probe_daemon(url: &str) -> bool {
    match mew_daemon::DaemonClient::connect(url).await {
        Ok((_client, _notify)) => true,
        Err(_) => false,
    }
}

/// Ensure a daemon is running and return its WebSocket URL. If one is already
/// healthy on the chosen loopback port, attach to it; otherwise spawn a
/// detached `mew daemon` as a child process and wait for it to come up. The
/// CLI flags are forwarded so the daemon matches the requested provider/model/
/// permission mode.
async fn spawn_or_connect_daemon(
    provider_flag: &str,
    model_flag: Option<&str>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<String> {
    let addr = allocate_loopback_port()?;
    let url = format!("ws://{addr}");

    if probe_daemon(&url).await {
        return Ok(url);
    }

    let exe = std::env::current_exe().context("resolve current executable")?;
    let port_arg = format!("127.0.0.1:{}", addr.port());
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(["daemon", "--port", &port_arg, "--background"]);
    if raw {
        cmd.arg("--raw");
    }
    if !provider_flag.is_empty() {
        cmd.arg("--provider").arg(provider_flag);
    }
    if let Some(model) = model_flag {
        cmd.arg("--model").arg(model);
    }
    match mode {
        mew_hooks::PermissionMode::Permissive => {
            cmd.arg("--permissive");
        }
        mew_hooks::PermissionMode::Auto => {
            cmd.arg("--auto");
        }
        mew_hooks::PermissionMode::AutoPlus => {
            cmd.arg("--auto-plus");
        }
        mew_hooks::PermissionMode::Dangerous => {
            cmd.arg("--dangerously-skip-permissions");
        }
        mew_hooks::PermissionMode::Standard => {}
    }
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("spawn daemon {exe:?} --port {port_arg}"))?;

    // `--background` double-forks and exits the immediate child quickly, so
    // reap it while polling for the real daemon to come up.
    let deadline = Instant::now() + DAEMON_STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if probe_daemon(&url).await {
            return Ok(url);
        }
        tokio::time::sleep(DAEMON_POLL_INTERVAL).await;
    }
    let _ = child.wait();
    anyhow::bail!(
        "spawned daemon did not become healthy at {url} within {DAEMON_STARTUP_TIMEOUT:?}"
    );
}

pub(crate) async fn toggle_mouse_capture(
    app: &mut mew_tui::App,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) {
    app.mouse_capture = !app.mouse_capture;
    if app.mouse_capture {
        let _ = crossterm::execute!(terminal.backend_mut(), crossterm::event::EnableMouseCapture);
        app.push_synthetic_message("mouse capture enabled (use /mouse to select text)".into());
    } else {
        let _ = crossterm::execute!(
            terminal.backend_mut(),
            crossterm::event::DisableMouseCapture,
        );
        app.push_synthetic_message(
            "mouse capture disabled \u{2014} native text selection enabled".into(),
        );
    }
}

pub(crate) fn copy_to_clipboard(text: &str) {
    #[cfg(target_os = "macos")]
    {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = std::process::Command::new("osascript")
            .args(["-e", &format!("set the clipboard to \"{}\"", escaped)])
            .output();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("wl-copy").arg(text).output();
        let _ = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .arg(text)
            .output();
    }
    #[cfg(target_os = "windows")]
    {
        let escaped = text.replace('\'', "''");
        let _ = std::process::Command::new("powershell")
            .args([
                "-command",
                &format!(
                    "Add-Type -AssemblyName System.Windows.Forms; \
                     [System.Windows.Forms.Clipboard]::SetText('{}')",
                    escaped
                ),
            ])
            .output();
    }
}

/// Read image data from the system clipboard and save it to a temporary
/// PNG file. Returns the path to the temp file on success.
///
/// Returns `Err(message)` with a human-readable explanation when no image
/// is available or the platform tool is missing.
pub(crate) fn read_clipboard_image() -> Result<std::path::PathBuf, String> {
    let png_data = read_clipboard_image_bytes()?;
    let temp_dir = std::env::temp_dir();
    let filename = format!("mew-clipboard-{}.png", ulid::Ulid::new());
    let path = temp_dir.join(filename);
    std::fs::write(&path, &png_data).map_err(|e| format!("failed to write temp file: {e}"))?;
    Ok(path)
}

/// Platform-specific extraction of raw PNG bytes from the clipboard.
fn read_clipboard_image_bytes() -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        read_clipboard_image_macos()
    }
    #[cfg(target_os = "linux")]
    {
        read_clipboard_image_linux()
    }
    #[cfg(target_os = "windows")]
    {
        read_clipboard_image_windows()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err("clipboard image paste is not supported on this platform".to_string())
    }
}

#[cfg(target_os = "macos")]
fn read_clipboard_image_macos() -> Result<Vec<u8>, String> {
    // Try `pngpaste` first — it's a clean, single-purpose tool.
    if let Ok(output) = std::process::Command::new("pngpaste").args(["-"]).output() {
        if output.status.success() && !output.stdout.is_empty() {
            return Ok(output.stdout);
        }
    }

    // Fall back to `osascript` which is always available on macOS.
    // The AppleScript reads the clipboard's PNG data («class PNGf»)
    // and writes the raw bytes to a temp file, which we then read back.
    let script = r#"
set tmpPath to (POSIX path of (path to temporary items)) & "mew-clip-" & (do shell script "uuidgen") & ".png"
set pngData to the clipboard as «class PNGf»
set fh to open for access tmpPath as «class furl» with write permission
try
    set eof fh to 0
    write pngData to fh
    close access fh
    return tmpPath
on error
    try
        close access fh
    end try
    error "no image in clipboard"
end try
"#;
    let output = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .map_err(|e| format!("osascript failed: {e}"))?;
    if !output.status.success() {
        return Err("no image in clipboard".to_string());
    }
    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path_str.is_empty() {
        return Err("no image in clipboard".to_string());
    }
    let path = std::path::PathBuf::from(&path_str);
    let data =
        std::fs::read(&path).map_err(|e| format!("failed to read clipboard temp file: {e}"))?;
    let _ = std::fs::remove_file(&path);
    Ok(data)
}

#[cfg(target_os = "linux")]
fn read_clipboard_image_linux() -> Result<Vec<u8>, String> {
    // Try tools in order: wl-paste (Wayland), xclip (X11), xsel (X11).
    // Each outputs raw image bytes to stdout when available.
    let mut tried = Vec::new();

    // wl-paste
    if let Ok(output) = std::process::Command::new("wl-paste")
        .args(["-t", "image/png"])
        .output()
    {
        tried.push("wl-paste");
        if output.status.success() && !output.stdout.is_empty() {
            return Ok(output.stdout);
        }
    } else {
        tried.push("wl-paste");
    }

    // xclip
    if let Ok(output) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
    {
        tried.push("xclip");
        if output.status.success() && !output.stdout.is_empty() {
            return Ok(output.stdout);
        }
    } else {
        tried.push("xclip");
    }

    // xsel — unlike xclip, xsel can't target a specific content type,
    // so it returns whatever is in the clipboard selection.  We guard
    // with a PNG magic-number check to avoid treating text as image data.
    if let Ok(output) = std::process::Command::new("xsel")
        .args(["--clipboard", "--output"])
        .output()
    {
        tried.push("xsel");
        if output.status.success()
            && !output.stdout.is_empty()
            && output.stdout.starts_with(b"\x89PNG")
        {
            return Ok(output.stdout);
        }
    } else {
        tried.push("xsel");
    }

    Err(format!(
        "no image in clipboard (tried {})",
        tried.join(", ")
    ))
}

#[cfg(target_os = "windows")]
fn read_clipboard_image_windows() -> Result<Vec<u8>, String> {
    // PowerShell: read clipboard image, save to temp as PNG, read back.
    // This is a two-step dance because PowerShell's clipboard API only
    // deals with files or streams, not raw stdout bytes easily.
    let ps = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$img = [System.Windows.Forms.Clipboard]::GetImage()
if ($img -eq $null) { exit 1 }
$temp = [System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), "mew-clip-" + [guid]::NewGuid().ToString() + ".png")
$img.Save($temp, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output $temp
"#;
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", ps])
        .output()
        .map_err(|e| format!("powershell failed: {e}"))?;
    if !output.status.success() {
        return Err("no image in clipboard".to_string());
    }
    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path_str.is_empty() {
        return Err("no image in clipboard".to_string());
    }
    let path = std::path::PathBuf::from(path_str);
    let data =
        std::fs::read(&path).map_err(|e| format!("failed to read clipboard temp file: {e}"))?;
    let _ = std::fs::remove_file(&path);
    Ok(data)
}
