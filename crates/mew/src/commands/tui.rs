//! TUI command implementations extracted from `main.rs`.
//!
//! These functions own the terminal UI lifecycle: starting the TUI event loop
//! (`run_tui`), connecting to a daemon frontend (`chat_with_daemon`), and
//! small helpers for mouse capture, clipboard, and context file display.

use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::info;

/// Maximum agent events processed per drain batch during streaming.
/// Prevents a burst of deltas from blocking the frame — text appears
/// incrementally instead of all at once.
const STREAMING_DRAIN_LIMIT: u32 = 4;

use mew_catalog::Catalog;
use mew_config::Config;
use mew_hooks::PluginHost;
use mew_mcp::McpClient;
use mew_session::Writer as SessionWriter;

use crate::setup::agent::{
    build_dispatcher, build_session_agent, connect_mcp_servers, load_mcp_configs,
    plugin_storage_map, wire_subagents, write_plugin_storage,
};
use crate::setup::personas::drain_pending_persona_switch;
use crate::setup::providers::{discover_models, load_catalog, resolve_model, resolve_reasoning};
use crate::PluginInfo;

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

    // Construct PluginInfo once — the daemon session_id is set after
    // the first prompt; model/provider are "daemon" placeholders.
    let plugin_info = Arc::new(std::sync::Mutex::new(crate::PluginInfo {
        session_id: String::new(),
        model: "daemon".to_string(),
        provider: "mewd".to_string(),
        workspace: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
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
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<()> {
    let cat = load_catalog(&cfg).await;

    run_tui(
        &cfg,
        cat.as_ref(),
        &provider_flag,
        model_flag,
        variant_flag,
        raw,
        mode,
    )
    .await
}

pub(crate) async fn run_tui(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<()> {
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let (display_provider, display_model) = (provider_id.clone(), model_id.clone());

    let session_id = ulid::Ulid::new().to_string();
    let session_writer = SessionWriter::open(&session_id)
        .await
        .context("open session")?;
    let todos_path = session_writer.path().parent().map(|p| p.join("todos.json"));

    // Plugin UI channel: plugins push content, we drain in the main loop.
    let (plugin_ui_tx, mut plugin_ui_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, String)>();

    let plugin_storage = Arc::new(std::sync::Mutex::new(plugin_storage_map()));

    let disabled_plugins = mew_config::load_state()
        .unwrap_or_default()
        .disabled_plugins;

    // Discover manifest-based extension packages (shared between the broker
    // and build_session_agent for [provides] paths).
    let cwd = std::env::current_dir().unwrap_or_default();
    let discovered = mew_ext_broker::discover_extensions(&cwd);
    if !discovered.is_empty() {
        tracing::info!("discovered {} extension package(s)", discovered.len());
    }

    // Shared session info that plugins can query via config_read.
    // Updated when persona/model changes; static values are set once.
    let plugin_info: Arc<std::sync::Mutex<PluginInfo>> =
        Arc::new(std::sync::Mutex::new(PluginInfo {
            session_id: session_id.clone(),
            model: model_id.clone(),
            provider: provider_id.clone(),
            workspace: std::env::current_dir()
                .unwrap_or_default()
                .display()
                .to_string(),
            active_persona: None,
        }));

    let info_for_config = plugin_info.clone();
    let dispatcher = build_dispatcher(
        |msg| {
            tracing::info!("[plugin-notify] {msg}");
        },
        move |key: &str| -> Option<String> {
            let info = info_for_config.lock().unwrap();
            match key {
                "session_id" => Some(info.session_id.clone()),
                "model" => Some(info.model.clone()),
                "provider" => Some(info.provider.clone()),
                "workspace" => Some(info.workspace.clone()),
                "active_persona" => info.active_persona.clone(),
                _ => None,
            }
        },
        |msg| {
            tracing::info!(target: "plugin", "{}", msg);
        },
        {
            let storage = plugin_storage.clone();
            move |key| storage.lock().unwrap().get(key).cloned()
        },
        {
            let storage = plugin_storage.clone();
            move |key, value| {
                write_plugin_storage(&mut storage.lock().unwrap(), key, value);
            }
        },
        {
            let storage = plugin_storage.clone();
            move |key| {
                storage.lock().unwrap().remove(key);
            }
        },
        move |key: &str, value: &str| {
            let _ = plugin_ui_tx.send((key.to_string(), value.to_string()));
        },
        &disabled_plugins,
        cfg.plugins.clone(),
        &discovered,
    )
    .await;

    {
        let host = PluginHost {
            notify: Arc::new(|_| {}),
            config_read: Arc::new(|_| None),
            log: Arc::new(|msg| {
                tracing::info!(target: "plugin", "{}", msg);
            }),
            storage_read: Arc::new(|_| None),
            storage_write: Arc::new(|_, _| {}),
            storage_delete: Arc::new(|_| {}),
            set_ui: Arc::new(|_, _| {}),
        };
        dispatcher.init(&host).await;
    }

    // Load personas for the event loop (the Cx struct and
    // drain_pending_persona_switch both reference this slice).
    let persona_loader = mew_personas::Loader::new(std::env::current_dir().unwrap_or_default());
    let loaded_personas = persona_loader.load().unwrap_or_default();

    // Load MCP tools (TUI-specific: sidebar status tracking).
    let mcp_configs = load_mcp_configs();
    let mut _mcp_clients: Vec<Arc<McpClient>> = Vec::new();
    let mut mcp_server_status: Vec<(String, bool, usize)> = Vec::new();
    let mut mcp_tools: Vec<Arc<dyn mew_tools::Tool>> = Vec::new();
    if !mcp_configs.is_empty() {
        let (tools, mcp_cls, status) = connect_mcp_servers(&mcp_configs).await;
        for cfg in &mcp_configs {
            let connected = status
                .iter()
                .any(|s| s.starts_with(&cfg.name) && s.contains("ready"));
            let count = if connected {
                tools
                    .iter()
                    .filter(|t| t.name().starts_with(&format!("{}__", cfg.name)))
                    .count()
            } else {
                0
            };
            mcp_server_status.push((cfg.name.clone(), connected, count));
        }
        mcp_tools = tools;
        _mcp_clients = mcp_cls;
    }

    // Load project context files for sidebar display names.
    let ctx_loader = mew_context::Loader::new(std::env::current_dir().unwrap_or_default());
    let ctx_files = ctx_loader.load().unwrap_or_default();
    let context_files: Vec<String> = ctx_files
        .iter()
        .map(|f| {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            context_display_name(&f.path, home.as_deref())
        })
        .collect();

    // Build the full session agent (provider, tools, personas, skills,
    // subagents, context files, pricing, etc.) via the shared builder.
    let mut agent = build_session_agent(
        cfg,
        cat,
        &provider_id,
        &model_id,
        raw,
        mode,
        Some(session_writer),
        None,
        None,
        false,
        dispatcher.clone(),
        todos_path.clone(),
        &discovered,
    )?;

    // Collect non-MCP tool names for the sidebar (before MCP tools are
    // inserted so the sidebar shows only built-in + plugin tools).
    let tool_names: Vec<String> = agent
        .tools
        .keys()
        .filter(|n| !n.contains("__"))
        .cloned()
        .collect();

    // Insert MCP tools before plugin tools so plugins can't overwrite them.
    for tool in mcp_tools {
        agent.tools.insert(tool.name().to_string(), tool);
    }

    // Register plugin-discovered tools.
    agent.register_plugin_tools().await;

    // Refresh subagent wiring now that plugin tools are registered.
    wire_subagents(
        &mut agent,
        cfg,
        cat,
        &provider_id,
        raw,
        dispatcher.clone(),
        &discovered,
    );

    // Load the saved todo list (if any) into the agent.
    if let Some(ref tp) = todos_path {
        if let Ok(list) = mew_agent::TodoList::load(tp).await {
            *agent.todos.lock().await = list;
        }
    }

    // Load persisted state for theme, recent models, and thinking variant.
    let state = mew_config::load_state().unwrap_or_default();

    // Apply reasoning variant: CLI flag > persisted state > catalog default.
    // Produces the provider params plus the canonical resolved name (a
    // clamped/snapped `budget:<n>` for numeric budgets) for the status bar.
    let variant_source = variant_flag.as_deref().map(|_| "cli").or_else(|| {
        if state.last_thinking_variant.is_some() {
            Some("state")
        } else {
            None
        }
    });
    let variant_name = variant_flag
        .as_deref()
        .or(state.last_thinking_variant.as_deref());
    let resolved = if let Some(name) = variant_name {
        if name == "off" || name == "none" {
            // Off is a plain disable unless the model has an explicit off
            // variant that needs real params (qwen3.8-max sends
            // `enable_thinking: false` because thinking is on by default).
            cat.as_ref().and_then(|c| {
                c.map_variant(name, &model_id)
                    .filter(|v| v.name == "off" || v.name == "none")
                    .map(|v| {
                        (
                            mew_provider::ReasoningConfig {
                                params: v.params.as_object().cloned().unwrap_or_default(),
                            },
                            v.name,
                        )
                    })
            })
        } else {
            match cat.as_ref() {
                Some(c) => c.map_variant(name, &model_id).map(|v| {
                    (
                        mew_provider::ReasoningConfig {
                            params: v.params.as_object().cloned().unwrap_or_default(),
                        },
                        v.name,
                    )
                }),
                None => resolve_reasoning(cat, &model_id, Some(name)),
            }
        }
    } else {
        resolve_reasoning(cat, &model_id, None)
    };
    if let Some((r, resolved_name)) = &resolved {
        agent.set_reasoning(Some(r.clone()));
        if resolved_name != "off" && resolved_name != "none" {
            info!(source = ?variant_source, model = %model_id, "enabled thinking variant");
        }
    }

    // Apply the default persona on startup (builder by default). The agent's
    // system prompt and tool set are configured now; `app` state is synced
    // below after the App is created.
    let startup_persona = if cfg.default_persona != "none" && cfg.default_persona != "default" {
        match loaded_personas
            .iter()
            .find(|p| p.name == cfg.default_persona)
        {
            Some(persona) => {
                agent.apply_persona(persona);
                tracing::info!(persona = %persona.name, "applied default persona on startup");
                Some(persona.name.clone())
            }
            None => {
                tracing::warn!(persona = %cfg.default_persona, "default_persona not found; skipping");
                None
            }
        }
    } else {
        None
    };

    let mut app = mew_tui::App::new();
    // Theme: state.toml overrides config.toml, falling back to "dark".
    let theme_name = if !state.theme.is_empty() {
        &state.theme
    } else {
        &cfg.tui.theme
    };
    app.theme = mew_tui::theme::Theme::load(theme_name);
    app.recent_models = state.recent_models.clone();

    // Restore the thinking variant display so the status bar shows it.
    // `resolved` carries the canonical name (a clamped/snapped
    // `budget:<n>` for numeric budgets) from the resolution above.
    if let Some((_, resolved_name)) = &resolved {
        app.active_thinking_variant = Some(resolved_name.clone());
    }

    // Seed the sidebar's todos pane from whatever was loaded at startup.
    app.todos = agent.todos.lock().await.items.clone();

    // Populate MCP server status in sidebar
    for (name, ok, count) in &mcp_server_status {
        if !ok {
            app.push_synthetic_message(format!("{name} connection failed"));
        }
        app.mcp_status.push((name.clone(), *ok, *count));
    }

    // Restore sidebar collapsed state from previous session.
    let prev_state = mew_config::load_state().unwrap_or_default();
    app.sidebar_collapsed = prev_state.sidebar_collapsed.clone();

    app.status.model = display_model.clone();
    app.status.provider = display_provider.clone();
    app.status.session_id = session_id.clone();
    // Sync the startup persona into the App so the sidebar and status
    // line show the right state from the first frame.
    if let Some(ref name) = startup_persona {
        app.active_persona = Some(name.clone());
    }
    if let Some(c) = cat {
        app.status.context_window = c.context_window(&model_id) as u32;
    }
    app.context_files = context_files;
    app.tools = tool_names;
    app.personas = loaded_personas
        .iter()
        .map(|p| (p.name.clone(), p.description.clone()))
        .collect();

    // Populate model list by querying providers and merging with catalog.
    app.models = discover_models(cfg, cat, raw).await;

    // Populate per-model thinking-variant names from the catalog so the
    // `/thinking` picker shows each model's actual levels (e.g. codex
    // low/medium/high/xhigh/max/ultra) instead of a hardcoded list.
    if let Some(c) = cat {
        app.thinking_variants = c
            .models
            .values()
            .map(|m| {
                (
                    m.id.clone(),
                    c.thinking_variants(&m.id)
                        .into_iter()
                        .map(|v| v.name)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        // Same for numeric budget ranges (qwen3.8-max etc.), so the picker
        // can offer a budget slider for models that accept one.
        app.thinking_budget = c
            .models
            .values()
            .filter_map(|m| {
                c.thinking_budget(&m.id).map(|budget| {
                    (
                        m.id.clone(),
                        mew_protocol::ThinkingBudgetInfo {
                            min: budget.min,
                            max: budget.max,
                            step: budget.step,
                            default: budget.default,
                            by_effort: budget.by_effort,
                        },
                    )
                })
            })
            .collect();
    }

    // Register dynamic slash commands from plugins.
    let dynamic_cmds = agent.dispatcher.on_register_slash_commands().await;
    let dynamic_slash: Vec<mew_tui::app::SlashCommand> = dynamic_cmds
        .into_iter()
        .map(|d| mew_tui::app::SlashCommand {
            name: d.name,
            description: d.description,
        })
        .collect();
    app.add_dynamic_slash_commands(dynamic_slash);

    // Setup terminal.
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

    // Create event loop.
    let (event_loop, mut event_rx) = mew_tui::EventLoop::new();
    event_loop.spawn();
    let event_loop = Arc::new(event_loop);

    // Main loop.
    // Tracks whether the most recently received event was a pure tick.
    // Used by the idle-aware render skip: if the event was a tick and
    // `app.needs_redraw()` is false, we skip the draw to save CPU.
    let mut last_event_was_tick = false;
    let result = loop {
        // Drain plugin UI updates before each render.
        while let Ok((key, value)) = plugin_ui_rx.try_recv() {
            app.plugin_ui.insert(key.clone(), value.clone());
            if key == "buddy/bubble" {
                app.touch_companion_bubble();
            }
        }

        // Render. Skip the draw when idle (tick with no visible changes)
        // to avoid burning CPU on a static screen. Input and agent events
        // always trigger a draw; ticks only draw when `needs_redraw()`
        // returns true (streaming, spinner, alerts, modals, etc).
        if !last_event_was_tick || app.needs_redraw() {
            if let Err(e) = terminal.draw(|f| {
                mew_tui::ui::draw(f, &mut app);
            }) {
                break Err(anyhow::anyhow!("draw error: {}", e));
            }
            mew_tui::title::set_terminal_title(mew_tui::title::title_for_streaming(app.streaming));
        }

        // Wait for at least one event.
        let event = match event_rx.recv().await {
            Some(e) => e,
            None => break Ok(()),
        };

        // Track whether this event is a pure tick (for idle-aware rendering).
        // If the drain loop processes any non-tick events, we'll reset this.
        last_event_was_tick = matches!(event, mew_tui::Event::Tick);

        // Process the first event.
        let mut should_break = false;
        match event {
            mew_tui::Event::Input(crossterm_event) => {
                if let Some(action) = mew_tui::events::handle_input_event(&mut app, crossterm_event)
                {
                    let mut target = crate::runtime::local::LocalTarget::new(
                        &mut agent,
                        cfg.clone(),
                        cat.cloned(),
                        provider_id.clone(),
                        raw,
                    );
                    let mut cx = crate::runtime::Ctx {
                        app: &mut app,
                        target: &mut target,
                        event_loop: &event_loop,
                        should_break: &mut should_break,
                        cat,
                        loaded_personas: &loaded_personas,
                        plugin_info: &plugin_info,
                    };
                    let _flow = crate::runtime::handle_action(&mut cx, action).await;
                }
            }

            mew_tui::Event::Agent(event) => {
                app.handle_agent_event(event);
                drain_pending_persona_switch(
                    &mut agent,
                    &mut app,
                    &loaded_personas,
                    cfg,
                    cat,
                    provider_id.as_str(),
                    raw,
                )
                .await;
                plugin_info.lock().unwrap().active_persona = app.active_persona.clone();
            }
            mew_tui::Event::Tick => {
                app.tick();
            }
            mew_tui::Event::Quit => should_break = true,
        }

        if should_break {
            break Ok(());
        }

        // Drain remaining events before next render (coalesces rapid input).
        // When streaming, limit agent events per drain batch so text
        // appears incrementally instead of all at once after a burst.
        //
        // Submit actions from the drain are deferred so @mention file reads
        // (which are async) can run after the drain finishes.
        let mut agent_drain_count = 0u32;
        let mut queued_actions: Vec<mew_tui::events::Action> = Vec::new();
        'drain: while let Ok(event) = event_rx.try_recv() {
            // Any non-tick event in the drain means we need a redraw.
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
                        // Queue the action for replay after the drain exits.
                        // The drain no longer interprets actions — it just coalesces
                        // and queues. This prevents the drop/defer bug class.
                        queued_actions.push(action);
                    }
                }
                mew_tui::Event::Agent(event) => {
                    app.handle_agent_event(event);
                    drain_pending_persona_switch(
                        &mut agent,
                        &mut app,
                        &loaded_personas,
                        cfg,
                        cat,
                        provider_id.as_str(),
                        raw,
                    )
                    .await;
                    plugin_info.lock().unwrap().active_persona = app.active_persona.clone();
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

        // Replay queued actions through handle_action (the single dispatch path).
        // The drain no longer interprets actions — it just coalesces and queues.
        for action in queued_actions {
            let mut target = crate::runtime::local::LocalTarget::new(
                &mut agent,
                cfg.clone(),
                cat.cloned(),
                provider_id.clone(),
                raw,
            );
            let mut cx = crate::runtime::Ctx {
                app: &mut app,
                target: &mut target,
                event_loop: &event_loop,
                should_break: &mut should_break,
                cat,
                loaded_personas: &loaded_personas,
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

        if app.should_quit {
            break Ok(());
        }

        // Handle pending mouse-capture toggle (needs a Terminal reference).
        if app.pending_mouse_toggle {
            app.pending_mouse_toggle = false;
            toggle_mouse_capture(&mut app, &mut terminal).await;
        }
    };

    // Notify plugins the session is saving so they can flush state.
    agent.dispatcher.on_session_save().await;

    // Save sidebar collapsed state for next session.
    {
        let mut save = mew_config::load_state().unwrap_or_default();
        // Use the app's current model/provider, which reflects any switches
        // made during the session (handle_switch_model updates app.status).
        save.last_model = app.status.model.clone();
        save.last_provider = app.status.provider.clone();
        save.last_thinking_variant = app.active_thinking_variant.clone();
        save.sidebar_collapsed = app.sidebar_collapsed.clone();
        save.recent_models = app.recent_models.clone();
        let _ = mew_config::save_state(&save);
    }

    // Notify plugins the session is stopping before tearing down.
    agent.dispatcher.on_stop().await;

    // Restore terminal.
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
    )?;
    terminal.show_cursor()?;
    mew_tui::title::set_terminal_title("mew");

    // Note: MCP client shutdown is best-effort. Subprocess cleanup
    // on exit is handled by the transport's Drop implementation.

    result
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

/// Produce a short display label for a context file path.
pub(crate) fn context_display_name(
    path: &std::path::Path,
    home: Option<&std::path::Path>,
) -> String {
    let leaf = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if let Some(home) = home {
        let config_base = home.join(".config").join("mew");
        if path.starts_with(&config_base) {
            return format!("global {leaf}");
        }
        let claude_base = home.join(".claude");
        if path.starts_with(&claude_base) {
            return format!("global {leaf}");
        }
    }

    if let Some(parent) = path.parent() {
        if parent.file_name().is_some_and(|n| n == ".mew") {
            if let Some(gp) = parent.parent() {
                let dir = gp.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if dir.is_empty() {
                    return format!(".mew/{leaf}");
                }
                return format!("{dir}/.mew/{leaf}");
            }
            return format!(".mew/{leaf}");
        }
    }

    let dir = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if dir.is_empty() {
        leaf.to_string()
    } else {
        format!("{leaf} in {dir}/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn context_display_name_global_config_mew() {
        let home = PathBuf::from("/home/user");
        let path = home.join(".config/mew/AGENTS.md");
        assert_eq!(context_display_name(&path, Some(&home)), "global AGENTS.md");
    }

    #[test]
    fn context_display_name_global_claude() {
        let home = PathBuf::from("/home/user");
        let path = home.join(".claude/CLAUDE.md");
        assert_eq!(context_display_name(&path, Some(&home)), "global CLAUDE.md");
    }

    #[test]
    fn context_display_name_mew_parent() {
        let path = PathBuf::from("/projects/myapp/.mew/PERSONA.md");
        assert_eq!(context_display_name(&path, None), "myapp/.mew/PERSONA.md");
    }

    #[test]
    fn context_display_name_plain_parent() {
        let path = PathBuf::from("/projects/myapp/README.md");
        assert_eq!(context_display_name(&path, None), "README.md in myapp/");
    }
}
