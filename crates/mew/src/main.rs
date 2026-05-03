use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::Write as _;
use std::sync::Arc;
use tracing::warn;

use mew_agent::Agent;
use mew_catalog::Catalog;
use mew_config::{Config, ProviderConfig};
use mew_hooks::NopDispatcher;
use mew_mcp::McpClient;
use mew_message::{Finish, Part, PartId, Role};
use mew_provider::Provider;
use mew_provider_anthropic::Adapter as AnthropicAdapter;
use mew_provider_openai::Adapter as OpenAIAdapter;
use mew_session::Writer as SessionWriter;
use mew_tools::tools::bash::Bash;
use mew_tools::tools::echo::Echo;
use mew_tools::tools::edit::Edit;
use mew_tools::tools::glob::Glob;
use mew_tools::tools::grep::Grep;
use mew_tools::tools::read::Read;
use mew_tools::tools::skill::Skill;
use mew_tools::tools::write::Write;

#[derive(Parser)]
#[command(name = "mew")]
#[command(about = "A terminal agent harness")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a single prompt non-interactively
    Run {
        /// Provider ID
        #[arg(long, default_value = "opencode-zen")]
        provider: String,

        /// Model ID (overrides provider default)
        #[arg(long)]
        model: Option<String>,

        /// Dump raw request/response to stderr
        #[arg(long)]
        raw: bool,

        /// The prompt to send
        prompt: Vec<String>,
    },
    /// Start an interactive session
    Chat {
        /// Provider ID
        #[arg(long, default_value = "opencode-zen")]
        provider: String,

        /// Model ID (overrides provider default)
        #[arg(long)]
        model: Option<String>,

        /// Dump raw request/response to stderr
        #[arg(long)]
        raw: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Load runtime state for fallback defaults.
    let state = mew_config::load_state().unwrap_or_default();

    match cli.command {
        Some(Commands::Run {
            provider,
            model,
            raw,
            prompt,
        }) => {
            let provider = if provider.is_empty() {
                if state.last_provider.is_empty() {
                    "opencode-zen".to_string()
                } else {
                    state.last_provider
                }
            } else {
                provider
            };
            let model = model.or_else(|| {
                if state.last_model.is_empty() {
                    None
                } else {
                    Some(state.last_model)
                }
            });
            run_cmd(provider, model, raw, prompt).await
        }
        Some(Commands::Chat {
            provider,
            model,
            raw,
        }) => {
            let provider = if provider.is_empty() {
                if state.last_provider.is_empty() {
                    "opencode-zen".to_string()
                } else {
                    state.last_provider
                }
            } else {
                provider
            };
            let model = model.or_else(|| {
                if state.last_model.is_empty() {
                    None
                } else {
                    Some(state.last_model)
                }
            });
            chat_cmd(provider, model, raw).await
        }
        None => {
            let provider = if state.last_provider.is_empty() {
                "opencode-zen".to_string()
            } else {
                state.last_provider
            };
            let model = if state.last_model.is_empty() {
                None
            } else {
                Some(state.last_model)
            };
            chat_cmd(provider, model, false).await
        }
    }
}

fn build_tools(skills: Arc<Vec<mew_skills::Skill>>) -> Vec<Arc<dyn mew_tools::Tool>> {
    let mut tools: Vec<Arc<dyn mew_tools::Tool>> = vec![
        Arc::new(Read),
        Arc::new(Write),
        Arc::new(Edit),
        Arc::new(Bash),
        Arc::new(Glob),
        Arc::new(Grep),
        Arc::new(Echo),
    ];
    if !skills.is_empty() {
        tools.push(Arc::new(Skill::new(skills)));
    }
    tools
}

fn build_permission_engine(cfg: &Config) -> Arc<mew_config::permissions::PermissionEngine> {
    Arc::new(mew_config::permissions::PermissionEngine::new_with_skills(
        cfg.permissions.rules.clone(),
        cfg.permissions.skills.clone(),
    ))
}

/// Load MCP server configs from cwd/mcp.json (Claude-Code-compatible format).
fn load_mcp_configs() -> Vec<mew_mcp::McpServerConfig> {
    let path = std::env::current_dir().unwrap_or_default().join("mcp.json");

    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(v) => {
                let servers = v.get("mcpServers").and_then(|s| s.as_object());
                match servers {
                    Some(servers) => servers
                        .iter()
                        .map(|(name, cfg)| mew_mcp::McpServerConfig {
                            name: name.clone(),
                            url: cfg.get("url").and_then(|v| v.as_str()).map(String::from),
                            command: cfg
                                .get("command")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            args: cfg
                                .get("args")
                                .and_then(|v| v.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|v: &serde_json::Value| {
                                            v.as_str().map(String::from)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                        .collect(),
                    None => Vec::new(),
                }
            }
            Err(e) => {
                tracing::warn!("failed to parse mcp.json: {}", e);
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            tracing::warn!("failed to read mcp.json: {}", e);
            Vec::new()
        }
    }
}

fn build_skills_xml(skills: &[mew_skills::Skill]) -> String {
    let mut buf = String::from("<available_skills>\n");
    for skill in skills {
        buf.push_str(&format!(
            "  <skill>\n    <name>{}</name>\n    <description>{}</description>\n  </skill>\n",
            escape_xml(&skill.name),
            escape_xml(&skill.description),
        ));
    }
    buf.push_str("</available_skills>\n");
    buf
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn connect_mcp_servers(
    configs: &[mew_mcp::McpServerConfig],
) -> (Vec<Arc<dyn mew_tools::Tool>>, Vec<Arc<mew_mcp::McpClient>>) {
    let mut tools: Vec<Arc<dyn mew_tools::Tool>> = Vec::new();
    let mut clients: Vec<Arc<mew_mcp::McpClient>> = Vec::new();

    for cfg in configs {
        let client = if let Some(ref url) = cfg.url {
            match McpClient::connect_http(&cfg.name, url).await {
                Ok(client) => client,
                Err(e) => {
                    tracing::warn!(server = %cfg.name, error = %e, "failed to connect to MCP server");
                    continue;
                }
            }
        } else if let Some(ref command) = cfg.command {
            match McpClient::connect_stdio(&cfg.name, command, &cfg.args).await {
                Ok(client) => client,
                Err(e) => {
                    tracing::warn!(server = %cfg.name, error = %e, "failed to connect to MCP server");
                    continue;
                }
            }
        } else {
            tracing::warn!(server = %cfg.name, "MCP server has no url or command");
            continue;
        };

        match client.list_tools().await {
            Ok(mcp_tools) => {
                let client = Arc::new(client);
                for def in &mcp_tools {
                    let name = def.qualified_name(&cfg.name);
                    tools.push(Arc::new(mew_mcp::McpTool::new(
                        &cfg.name,
                        def,
                        client.clone(),
                    )));
                    tracing::info!(tool = %name, server = %cfg.name, "registered MCP tool");
                }
                clients.push(client);
            }
            Err(e) => {
                tracing::warn!(server = %cfg.name, error = %e, "failed to list MCP tools");
            }
        }
    }

    (tools, clients)
}

async fn run_cmd(
    provider_flag: String,
    model_flag: Option<String>,
    raw: bool,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let prompt = prompt_parts.join(" ");
    if prompt.is_empty() {
        anyhow::bail!("missing prompt");
    }

    let cfg = mew_config::load().context("load config")?;

    let cat = match mew_catalog::load().await {
        Ok(c) => c,
        Err(e) => {
            warn!("catalog load failed, using fallback routing: {}", e);
            return build_and_run(&cfg, None, &provider_flag, model_flag, raw, prompt).await;
        }
    };

    build_and_run(&cfg, Some(&cat), &provider_flag, model_flag, raw, prompt).await
}

async fn chat_cmd(provider_flag: String, model_flag: Option<String>, raw: bool) -> Result<()> {
    let cfg = mew_config::load().context("load config")?;

    let cat = match mew_catalog::load().await {
        Ok(c) => c,
        Err(e) => {
            warn!("catalog load failed, using fallback routing: {}", e);
            return run_tui(&cfg, None, &provider_flag, model_flag, raw).await;
        }
    };

    run_tui(&cfg, Some(&cat), &provider_flag, model_flag, raw).await
}

async fn run_tui(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
) -> Result<()> {
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let provider =
        build_provider(cfg, cat, &provider_id, &model_id, raw).context("build provider")?;

    // For router providers, use the big model for display.
    let (display_provider, display_model) =
        if let Some(pc) = cfg.providers.get(&provider_id) {
            if pc.kind == "router" && !pc.big.is_empty() {
                let (_, big_mid) = resolve_model(cfg, cat, &provider_id, Some(pc.big.clone()));
                let (big_pid, _) = resolve_model(cfg, cat, &provider_id, Some(pc.big.clone()));
                (big_pid, big_mid)
            } else {
                (provider_id.clone(), model_id.clone())
            }
        } else {
            (provider_id.clone(), model_id.clone())
        };

    let session_id = ulid::Ulid::new().to_string();
    let session_writer = SessionWriter::open(&session_id)
        .await
        .context("open session")?;

    let dispatcher = Arc::new(NopDispatcher);

    // Load skills for the skill tool.
    let skills_loader = mew_skills::Loader::new(std::env::current_dir().unwrap_or_default());
    let loaded_skills = skills_loader.load().unwrap_or_default();
    let skills = Arc::new(loaded_skills);

    let mut tools = build_tools(skills.clone());

    // Load MCP tools.
    let mcp_configs = load_mcp_configs();
    let mut _mcp_clients: Vec<Arc<McpClient>> = Vec::new();
    if !mcp_configs.is_empty() {
        let (mcp_tools, mcp_cls) = connect_mcp_servers(&mcp_configs).await;
        tools.extend(mcp_tools);
        _mcp_clients = mcp_cls;
    }

    let permission_engine = build_permission_engine(cfg);

    // Load project context files.
    let ctx_loader = mew_context::Loader::new(std::env::current_dir().unwrap_or_default());
    let ctx_files = ctx_loader.load().unwrap_or_default();

    // Populate sidebar data before moving tools.
    let context_files: Vec<String> = ctx_files
        .iter()
        .map(|f| f.path.to_string_lossy().to_string())
        .collect();
    let tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();

    let mut agent = Agent::new(provider, dispatcher, Some(session_writer), tools, None);
    agent.set_permission_engine(permission_engine);

    // Set vision capability and pricing from catalog.
    if let Some(c) = cat {
        agent.supports_vision = c.supports_vision(&display_model);
        agent.context_window = c.context_window(&display_model).max(0) as u32;
        if let Some(m) = c.lookup(&display_model) {
            agent.input_price = m.pricing.input;
            agent.output_price = m.pricing.output;
            agent.cache_read_price = m.pricing.cache_read;
            agent.cache_write_price = m.pricing.cache_write;
            agent.reasoning_price = m.pricing.reasoning;
        }
    }

    if !ctx_files.is_empty() {
        agent.set_system(mew_context::build_system_prompt(&ctx_files));
    }

    // If skills are loaded, append the skill listing to the system prompt.
    if !skills.is_empty() {
        let mut system = if ctx_files.is_empty() {
            String::new()
        } else {
            mew_context::build_system_prompt(&ctx_files)
        };
        system.push_str(&build_skills_xml(&skills));
        agent.set_system(system);
    }

    let mut app = mew_tui::App::new();
    app.status.model = display_model.clone();
    app.status.provider = display_provider.clone();
    app.status.session_id = session_id.clone();
    if let Some(c) = cat {
        app.status.context_window = c.context_window(&model_id) as u32;
    }
    app.context_files = context_files;
    app.tools = tool_names;

    // Populate model list by querying providers and merging with catalog.
    app.models = discover_models(cfg, cat, raw).await;

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

    // Main loop.
    let result = loop {
        // Render.
        if let Err(e) = terminal.draw(|f| mew_tui::ui::draw(f, &mut app)) {
            break Err(anyhow::anyhow!("draw error: {}", e));
        }

        // Wait for at least one event.
        let event = match event_rx.recv().await {
            Some(e) => e,
            None => break Ok(()),
        };

        // Process the first event.
        let mut should_break = false;
        match event {
            mew_tui::Event::Input(crossterm_event) => {
                if let Some(action) = mew_tui::events::handle_input_event(&mut app, crossterm_event)
                {
                    match action {
                        mew_tui::events::Action::Submit(text) => {
                            let cwd = std::env::current_dir().unwrap_or_default();
                            let (enriched, attachments) =
                                process_mentions(&text, &cwd, &mut app.context_files).await;
                            app.messages
                                .push(user_message(enriched.clone(), attachments.clone()));
                            app.streaming = true;
                            let agent_rx = agent.run_with_parts(enriched, attachments);
                            event_loop.forward_agent_events(agent_rx);
                        }
                        mew_tui::events::Action::SlashCommand(text) => {
                            match app.handle_slash(&text) {
                                mew_tui::SlashResult::Continue => {}
                                mew_tui::SlashResult::Quit => should_break = true,
                                mew_tui::SlashResult::Clear => {
                                    app.clear_messages();
                                }
                                mew_tui::SlashResult::Message(msg) => {
                                    app.messages.push(synthetic_message(msg));
                                }
                                mew_tui::SlashResult::Compact => {
                                    agent.force_compact().await;
                                    app.messages.push(synthetic_message(
                                        "compaction will run on next turn".into(),
                                    ));
                                }
                                mew_tui::SlashResult::SwitchModel(new_model) => {
                                    let (new_provider_id, new_model_id) = if let Some(idx) = new_model.find('/') {
                                        (&new_model[..idx], &new_model[idx + 1..])
                                    } else {
                                        (provider_id.as_str(), new_model.as_str())
                                    };
                                    match build_provider(cfg, cat, new_provider_id, new_model_id, raw) {
                                        Ok(new_provider) => {
                                            agent.provider = new_provider;
                                            app.status.model = new_model_id.to_string();
                                            app.status.provider = new_provider_id.to_string();
                                            if let Some(c) = cat {
                                                app.status.context_window = c.context_window(new_model_id) as u32;
                                                if let Some(m) = c.lookup(new_model_id) {
                                                    agent.input_price = m.pricing.input;
                                                    agent.output_price = m.pricing.output;
                                                    agent.cache_read_price = m.pricing.cache_read;
                                                    agent.cache_write_price = m.pricing.cache_write;
                                                    agent.reasoning_price = m.pricing.reasoning;
                                                }
                                            }
                                            let state = mew_config::State {
                                                last_model: new_model_id.to_string(),
                                                last_provider: new_provider_id.to_string(),
                                            };
                                            if let Err(e) = mew_config::save_state(&state) {
                                                tracing::warn!("failed to save state: {}", e);
                                            }
                                            app.messages.push(synthetic_message(format!("switched to {}", new_model)));
                                        }
                                        Err(e) => {
                                            app.messages.push(synthetic_message(format!("failed to switch: {}", e)));
                                        }
                                    }
                                }
                                mew_tui::SlashResult::ResumeSession(ref id) => {
                                    match mew_session::Reader::load(id).await {
                                        Ok(msgs) => {
                                            agent.load_messages(msgs.clone()).await;
                                            app.clear_messages();
                                            for msg in &msgs {
                                                app.messages.push(msg.clone());
                                            }
                                            app.status.session_id = id.clone();
                                            app.auto_scroll = true;
                                            app.scroll = app.max_scroll;
                                            app.messages.push(synthetic_message(
                                                format!("resumed session {}", id),
                                            ));
                                        }
                                        Err(e) => {
                                            app.messages.push(synthetic_message(
                                                format!("failed to load session {}: {}", id, e),
                                            ));
                                        }
                                    }
                                }
                                mew_tui::SlashResult::OpenModelPicker => {
                                    app.open_command_palette();
                                }
                            }
                        }
                        mew_tui::events::Action::Clear => {
                            app.clear_messages();
                        }
                        mew_tui::events::Action::SwitchModel(new_model) => {
                            let (new_provider_id, new_model_id) =
                                if let Some(idx) = new_model.find('/') {
                                    (&new_model[..idx], &new_model[idx + 1..])
                                } else {
                                    (provider_id.as_str(), new_model.as_str())
                                };
                            match build_provider(cfg, cat, new_provider_id, new_model_id, raw) {
                                Ok(new_provider) => {
                                    agent.provider = new_provider;
                                    app.status.model = new_model_id.to_string();
                                    app.status.provider = new_provider_id.to_string();
                                    if let Some(c) = cat {
                                        app.status.context_window = c.context_window(new_model_id) as u32;
                                        if let Some(m) = c.lookup(new_model_id) {
                                            agent.input_price = m.pricing.input;
                                            agent.output_price = m.pricing.output;
                                            agent.cache_read_price = m.pricing.cache_read;
                                            agent.cache_write_price = m.pricing.cache_write;
                                            agent.reasoning_price = m.pricing.reasoning;
                                        }
                                    }
                                    let state = mew_config::State {
                                        last_model: new_model_id.to_string(),
                                        last_provider: new_provider_id.to_string(),
                                    };
                                    if let Err(e) = mew_config::save_state(&state) {
                                        tracing::warn!("failed to save state: {}", e);
                                    }
                                    app.messages.push(synthetic_message(format!("switched to {}", new_model)));
                                }
                                Err(e) => {
                                    app.messages.push(synthetic_message(format!("failed to switch model: {}", e)));
                                }
                            }
                        }
                        mew_tui::events::Action::Cancel => {
                            agent.cancel_token.cancel();
                            app.streaming = false;
                        }
                        mew_tui::events::Action::InsertAtMention(mention) => {
                            app.input.push_str(&mention);
                            app.cursor += mention.len();
                        }
                        mew_tui::events::Action::Quit => should_break = true,
                    }
                }
            }
            mew_tui::Event::Agent(event) => {
                app.handle_agent_event(event);
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
        const STREAMING_DRAIN_LIMIT: u32 = 4;
        let mut pending_drain_submit: Option<String> = None;
        'drain: while let Ok(event) = event_rx.try_recv() {
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
                        match action {
                            mew_tui::events::Action::Quit => {
                                should_break = true;
                                break 'drain;
                            }
                            mew_tui::events::Action::Submit(text) => {
                                pending_drain_submit = Some(text);
                                break 'drain;
                            }
                            mew_tui::events::Action::Cancel => {
                                agent.cancel_token.cancel();
                                app.streaming = false;
                            }
                            mew_tui::events::Action::Clear => {
                                app.clear_messages();
                            }
                            mew_tui::events::Action::InsertAtMention(mention) => {
                                app.input.push_str(&mention);
                                app.cursor += mention.len();
                            }
                            mew_tui::events::Action::SlashCommand(text) => {
                                match app.handle_slash(&text) {
                                    mew_tui::SlashResult::Continue => {}
                                    mew_tui::SlashResult::Quit => {
                                        should_break = true;
                                        break 'drain;
                                    }
                                    mew_tui::SlashResult::Clear => {
                                        app.clear_messages();
                                    }
                                    mew_tui::SlashResult::Message(msg) => {
                                        app.messages.push(synthetic_message(msg));
                                    }
                                    mew_tui::SlashResult::Compact => {
                                        agent.force_compact().await;
                                        app.messages.push(synthetic_message(
                                            "compaction will run on next turn".into(),
                                        ));
                                    }
                                    mew_tui::SlashResult::SwitchModel(_) => {
                                        // Model switches are deferred; handled in main loop.
                                    }
                                    mew_tui::SlashResult::ResumeSession(_) => {
                                        // Deferred; handled in main loop when not streaming.
                                    }
                                    mew_tui::SlashResult::OpenModelPicker => {
                                        // Deferred to main loop; ignored during drain.
                                    }
                                }
                            }
                            mew_tui::events::Action::SwitchModel(_) => {
                                // Model switches happen via the command palette, which
                                // requires mode=CommandPalette. The palette is never
                                // open during heavy streaming, so this branch is
                                // effectively unreachable in the drain path.
                            }
                        }
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

        // Process a Submit action deferred from the drain (needs async @mention reads).
        if let Some(text) = pending_drain_submit {
            let cwd = std::env::current_dir().unwrap_or_default();
            let (enriched, attachments) =
                process_mentions(&text, &cwd, &mut app.context_files).await;
            app.messages
                .push(user_message(enriched.clone(), attachments.clone()));
            app.streaming = true;
            let agent_rx = agent.run_with_parts(enriched, attachments);
            event_loop.forward_agent_events(agent_rx);
        }

        if should_break {
            break Ok(());
        }

        if app.should_quit {
            break Ok(());
        }
    };

    // Restore terminal.
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
    )?;
    terminal.show_cursor()?;

    // Note: MCP client shutdown is best-effort. Subprocess cleanup
    // on exit is handled by the transport's Drop implementation.

    result
}

fn image_mime(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// Resolve @mentions in `text`. Text files are inlined; image files become `Part::File`
/// attachments. Returns the (possibly extended) text and any image parts.
async fn process_mentions(
    text: &str,
    cwd: &std::path::Path,
    context_files: &mut Vec<String>,
) -> (String, Vec<Part>) {
    let mentions = mew_tui::app::parse_file_mentions(text);
    let mut enriched = text.to_string();
    let mut attachments: Vec<Part> = Vec::new();

    for path_str in &mentions {
        let path = cwd.join(path_str);
        if let Some(mime) = image_mime(path_str) {
            let mention = format!("@{}", path_str);
            enriched = enriched.replace(&mention, "");
            if path.exists() {
                let abs = path.canonicalize().unwrap_or(path.clone());
                let filename = std::path::Path::new(path_str)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path_str)
                    .to_string();
                attachments.push(Part::File(mew_message::FilePart {
                    base: mew_message::PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    mime: mime.to_string(),
                    url: format!("file://{}", abs.display()),
                    filename: Some(filename),
                }));
                if !context_files.contains(path_str) {
                    context_files.push(path_str.clone());
                }
            } else {
                enriched.push_str(&format!("\n\n[error reading {}: file not found]", path_str));
            }
        } else {
            match tokio::fs::read_to_string(&path).await {
                Ok(content) => {
                    enriched.push_str(&format!("\n\n--- {} ---\n{}", path_str, content));
                    if !context_files.contains(path_str) {
                        context_files.push(path_str.clone());
                    }
                }
                Err(e) => {
                    enriched.push_str(&format!("\n\n[error reading {}: {}]", path_str, e));
                }
            }
        }
    }

    (enriched, attachments)
}

fn user_message(text: String, attachments: Vec<Part>) -> mew_message::Message {
    let msg_id = ulid::Ulid::new();
    let mut parts = vec![Part::Text(mew_message::TextPart {
        base: mew_message::PartBase {
            id: ulid::Ulid::new(),
            message_id: msg_id,
            session_id: ulid::Ulid::new(),
        },
        text: text.clone(),
        synthetic: false,
    })];
    parts.extend(attachments);
    mew_message::Message {
        id: msg_id,
        session_id: ulid::Ulid::new(),
        role: Role::User,
        parts,
        time: mew_message::Time {
            created: chrono::Utc::now().timestamp_millis(),
            completed: None,
        },
        assistant: None,
    }
}

fn synthetic_message(text: String) -> mew_message::Message {
    let msg_id = ulid::Ulid::new();
    mew_message::Message {
        id: msg_id,
        session_id: ulid::Ulid::new(),
        role: Role::Assistant,
        parts: vec![Part::Text(mew_message::TextPart {
            base: mew_message::PartBase {
                id: ulid::Ulid::new(),
                message_id: msg_id,
                session_id: ulid::Ulid::new(),
            },
            text,
            synthetic: true,
        })],
        time: mew_message::Time {
            created: chrono::Utc::now().timestamp_millis(),
            completed: Some(chrono::Utc::now().timestamp_millis()),
        },
        assistant: None,
    }
}

async fn discover_models(cfg: &Config, cat: Option<&Catalog>, raw: bool) -> Vec<(String, String)> {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut models = Vec::new();

    // Collect all provider IDs to query: configured + built-in.
    let mut provider_ids: Vec<&str> = cfg.providers.keys().map(|s| s.as_str()).collect();
    for pid in &["opencode-zen", "opencode-go", "z-ai"] {
        if !provider_ids.contains(pid) {
            provider_ids.push(pid);
        }
    }

    // Query each provider.
    for pid in provider_ids {
        let provider = match build_provider(cfg, cat, pid, "", raw) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("discovery: failed to build provider {}: {}", pid, e);
                continue;
            }
        };

        match provider.list_models().await {
            Ok(list) => {
                tracing::info!("discovery: provider {} returned {} models", pid, list.len());
                for m in list {
                    let full_id = if m.id.contains('/') {
                        m.id.clone()
                    } else {
                        format!("{}/{}", pid, m.id)
                    };
                    if seen.insert(full_id.clone()) {
                        let desc = if let Some(c) = cat.and_then(|c| c.lookup(&m.id)) {
                            format!("{} · {} · {} ctx", pid, c.shape, c.context_window)
                        } else {
                            let shape = provider_name_to_shape(pid);
                            format!("{} · {}", pid, shape)
                        };
                        models.push((full_id, desc));
                    }
                }
            }
            Err(e) => {
                tracing::warn!("discovery: provider {} list_models failed: {}", pid, e);
            }
        }
    }

    // Add hardcoded fallbacks if nothing discovered.
    if models.is_empty() {
        tracing::warn!("discovery: no models from any provider, using fallbacks");
        let fallbacks: Vec<(String, String)> = vec![
            (
                "opencode-zen/deepseek-v4-flash".into(),
                "opencode-zen · openai".into(),
            ),
            ("z-ai/glm-5.1".into(), "z-ai · anthropic".into()),
            (
                "opencode-go/minimax-text-01".into(),
                "opencode-go · anthropic".into(),
            ),
        ];
        for (id, desc) in fallbacks {
            if seen.insert(id.clone()) {
                models.push((id, desc));
            }
        }
    }

    models.sort_by(|a, b| a.0.cmp(&b.0));
    models
}

fn provider_name_to_shape(pid: &str) -> &'static str {
    match pid {
        "opencode-zen" | "opencode-go" => "openai",
        "z-ai" => "anthropic",
        _ => "openai",
    }
}

async fn build_and_run(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    prompt: String,
) -> Result<()> {
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let provider =
        build_provider(cfg, cat, &provider_id, &model_id, raw).context("build provider")?;

    // For router providers, use the big model for display.
    let (display_provider, display_model) =
        if let Some(pc) = cfg.providers.get(&provider_id) {
            if pc.kind == "router" && !pc.big.is_empty() {
                let (_, big_mid) = resolve_model(cfg, cat, &provider_id, Some(pc.big.clone()));
                let (big_pid, _) = resolve_model(cfg, cat, &provider_id, Some(pc.big.clone()));
                (big_pid, big_mid)
            } else {
                (provider_id.clone(), model_id.clone())
            }
        } else {
            (provider_id.clone(), model_id.clone())
        };

    let session_id = ulid::Ulid::new().to_string();
    let session_writer = SessionWriter::open(&session_id)
        .await
        .context("open session")?;

    let dispatcher = Arc::new(NopDispatcher);

    // Load skills for the skill tool.
    let skills_loader = mew_skills::Loader::new(std::env::current_dir().unwrap_or_default());
    let loaded_skills = skills_loader.load().unwrap_or_default();
    let skills = Arc::new(loaded_skills);

    let mut tools = build_tools(skills.clone());

    // Load MCP tools.
    let mcp_configs = load_mcp_configs();
    let mut _mcp_clients: Vec<Arc<McpClient>> = Vec::new();
    if !mcp_configs.is_empty() {
        let (mcp_tools, mcp_cls) = connect_mcp_servers(&mcp_configs).await;
        tools.extend(mcp_tools);
        _mcp_clients = mcp_cls;
    }

    let permission_engine = build_permission_engine(cfg);

    let mut agent = Agent::new(provider, dispatcher, Some(session_writer), tools, None);
    agent.set_permission_engine(permission_engine);

    // Set vision capability and pricing from catalog.
    if let Some(c) = cat {
        agent.supports_vision = c.supports_vision(&display_model);
        agent.context_window = c.context_window(&display_model).max(0) as u32;
        if let Some(m) = c.lookup(&display_model) {
            agent.input_price = m.pricing.input;
            agent.output_price = m.pricing.output;
            agent.cache_read_price = m.pricing.cache_read;
            agent.cache_write_price = m.pricing.cache_write;
            agent.reasoning_price = m.pricing.reasoning;
        }
    }

    // Load project context files and prepend to system prompt
    let ctx_loader = mew_context::Loader::new(std::env::current_dir().unwrap_or_default());
    let ctx_files = ctx_loader.load().unwrap_or_default();
    if !ctx_files.is_empty() {
        agent.set_system(mew_context::build_system_prompt(&ctx_files));
    }
    if !skills.is_empty() {
        let mut system = agent.system.clone();
        system.push_str(&build_skills_xml(&skills));
        agent.set_system(system);
    }

    let mut rx = agent.run(prompt);

    let mut part_types: std::collections::HashMap<PartId, &'static str> =
        std::collections::HashMap::new();

    while let Some(event) = rx.recv().await {
        match event {
            mew_agent::AgentEvent::Provider(ev) => match ev {
                mew_provider::ProviderEvent::PartStart { part } => {
                    let id = part.id();
                    match &part {
                        Part::Text(_) => {
                            part_types.insert(id, "text");
                        }
                        Part::Reasoning(_) => {
                            part_types.insert(id, "reasoning");
                            eprintln!("\n[thinking]");
                        }
                        Part::ToolCall(_) => {
                            part_types.insert(id, "tool");
                        }
                        _ => {}
                    }
                }
                mew_provider::ProviderEvent::PartDelta {
                    part_id,
                    field: _,
                    delta,
                } => match part_types.get(&part_id) {
                    Some(&"reasoning") => {
                        eprint!("{}", delta);
                        let _ = std::io::stderr().flush();
                    }
                    Some(&"text") => {
                        print!("{}", delta);
                        let _ = std::io::stdout().flush();
                    }
                    Some(&"tool") => {}
                    _ => {}
                },
                mew_provider::ProviderEvent::PartEnd { part_id } => {
                    match part_types.get(&part_id) {
                        Some(&"reasoning") => eprintln!("\n[/thinking]"),
                        Some(&"tool") => eprintln!(),
                        _ => {}
                    }
                    part_types.remove(&part_id);
                }
                mew_provider::ProviderEvent::MessageEnd { finish, .. } => {
                    if finish == Finish::Stop {
                        println!();
                    }
                }
                _ => {}
            },
            mew_agent::AgentEvent::PermissionRequest { call, tx } => {
                eprintln!("\n[permission] {}: {:?}", call.tool_name, call.input);
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            mew_agent::AgentEvent::ToolStart { call_id } => {
                eprintln!("\n[tool start: {}]", call_id);
            }
            mew_agent::AgentEvent::ToolEnd { call_id, success } => {
                eprintln!("[tool end: {}] success={}", call_id, success);
            }
            mew_agent::AgentEvent::PartUpdated { part_id: _, part } => {
                if let Part::ToolCall(tc) = &part {
                    if let mew_message::ToolState::Completed(c) = &tc.state {
                        if let Some(ref diff) = c.diff {
                            eprintln!("[diff] {}", diff);
                        }
                    }
                }
            }
            mew_agent::AgentEvent::ToolProgress { .. } => {}
            mew_agent::AgentEvent::Error(msg) => {
                anyhow::bail!("agent error: {}", msg);
            }
        }
    }

    Ok(())
}

fn resolve_model(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
) -> (String, String) {
    let mut provider_id = provider_flag.to_string();
    let mut model_id = model_flag.unwrap_or_default();

    if !model_id.is_empty() {
        // Check catalog first for automatic provider/shape selection.
        if let Some(c) = cat {
            if let Some(m) = c.lookup(&model_id) {
                provider_id = m.provider.clone();
            } else if let Some(idx) = model_id.find('/') {
                let candidate = &model_id[..idx];
                if is_known_provider(cfg, candidate) {
                    if provider_id == "opencode-zen" {
                        provider_id = candidate.to_string();
                    }
                    model_id = model_id[idx + 1..].to_string();
                }
            }
        } else if let Some(idx) = model_id.find('/') {
            let candidate = &model_id[..idx];
            if is_known_provider(cfg, candidate) {
                if provider_id == "opencode-zen" {
                    provider_id = candidate.to_string();
                }
                model_id = model_id[idx + 1..].to_string();
            }
        }
    }

    (provider_id, model_id)
}

fn is_known_provider(cfg: &Config, provider_id: &str) -> bool {
    cfg.providers.contains_key(provider_id)
        || matches!(provider_id, "opencode-zen" | "opencode-go" | "z-ai")
}

fn build_provider(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    model_override: &str,
    raw: bool,
) -> Result<Arc<dyn Provider>> {
    let pc = cfg
        .providers
        .get(provider_id)
        .cloned()
        .or_else(|| match provider_id {
            "opencode-zen" => Some(ProviderConfig {
                shape: "openai".to_string(),
                base_url: "https://opencode.ai/zen/v1".to_string(),
                credential_ref: "opencode-zen".to_string(), kind: "direct".into(), small: String::new(), big: String::new(),
            }),
            "opencode-go" => Some(ProviderConfig {
                shape: "openai".to_string(),
                base_url: "https://opencode.ai/zen/go/v1".to_string(),
                credential_ref: "opencode-zen".to_string(), kind: "direct".into(), small: String::new(), big: String::new(),
            }),
            "z-ai" => Some(ProviderConfig {
                shape: "anthropic".to_string(),
                base_url: "https://api.z.ai/api/anthropic/v1".to_string(),
                credential_ref: "z-ai".to_string(), kind: "direct".into(), small: String::new(), big: String::new(),
            }),
            "deepseek" => Some(ProviderConfig {
                shape: "deepseek".to_string(),
                base_url: "https://api.deepseek.ai/v1".to_string(),
                credential_ref: "deepseek".to_string(), kind: "direct".into(), small: String::new(), big: String::new(),
            }),
            _ => None,
        })
        .with_context(|| format!("unknown provider {}", provider_id))?;

    let creds = mew_config::get_credential(&pc.credential_ref).context("get credential")?;

    // Router: build a router provider wrapping small + big models.
    if pc.kind == "router" && !pc.small.is_empty() && !pc.big.is_empty() {
        let (small_pid, small_mid) = resolve_model(cfg, cat, provider_id, Some(pc.small.clone()));
        let (big_pid, big_mid) = resolve_model(cfg, cat, provider_id, Some(pc.big.clone()));

        let small = build_provider(cfg, cat, &small_pid, &small_mid, raw)?;
        let big = build_provider(cfg, cat, &big_pid, &big_mid, raw)?;

        tracing::info!(
            small_provider = %small_pid,
            small_model = %small_mid,
            big_provider = %big_pid,
            big_model = %big_mid,
            "built router provider"
        );

        // Use the big model for display.
        let model = if model_override.is_empty() {
            big_mid.clone()
        } else {
            model_override.to_string()
        };

        let mut router = mew_provider_router::Router::new(small, big);
        router.set_turn_threshold(3);

        return Ok(Arc::new(mew_provider_router::Routed::new(
            router,
            big_pid,
            model,
        )));
    }

    let model = if model_override.is_empty() {
        if cfg.default_model.is_empty() {
            "deepseek-v4-flash".to_string()
        } else {
            cfg.default_model.clone()
        }
    } else {
        model_override.to_string()
    };

    let mut shape = pc.shape;
    if let Some(c) = cat {
        let s = c.shape_for(&model);
        if !s.is_empty() {
            shape = s.to_string();
        }
    }

    let mut base_url = pc.base_url;
    if provider_id == "opencode-go" && model.starts_with("minimax-") {
        shape = "anthropic".to_string();
        base_url = "https://opencode.ai/zen/go/v1".to_string();
    }

    match shape.as_str() {
        "openai" => {
            let mut adapter = OpenAIAdapter::new(provider_id.to_string(), base_url, model, creds);
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        "anthropic" => {
            let mut adapter =
                AnthropicAdapter::new(provider_id.to_string(), base_url, model, creds);
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        _ => anyhow::bail!("unsupported shape {} for provider {}", shape, provider_id),
    }
}
