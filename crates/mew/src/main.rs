use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::Write as _;
use std::sync::Arc;
use tracing::{info, warn};

use async_trait::async_trait;

mod config_editor;

use mew_agent::Agent;
use mew_catalog::Catalog;
use mew_config::Config;
use mew_hooks::{Dispatcher, NopDispatcher, PluginHost};
use mew_hooks_runtime::SubprocessDispatcher;
use mew_mcp::McpClient;
use mew_message::{Finish, Part, PartId, Role};
use mew_provider::Provider;
use mew_provider_anthropic::Adapter as AnthropicAdapter;
use mew_provider_openai::Adapter as OpenAIAdapter;
use mew_session::Writer as SessionWriter;
use mew_tools::tools::bash::Bash;
use mew_tools::tools::echo::Echo;
use mew_tools::tools::edit::Edit;
use mew_tools::tools::exit_tool::ExitTool;
use mew_tools::tools::flag_important::{FlagImportant, FlaggedFile};
use mew_tools::tools::glob::Glob;
use mew_tools::tools::grep::Grep;
use mew_tools::tools::progress_update::ProgressUpdate;
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
        /// Provider ID (defaults to last-used or opencode-zen)
        #[arg(long)]
        provider: Option<String>,

        /// Model ID (overrides provider default)
        #[arg(long)]
        model: Option<String>,

        /// Thinking variant (e.g. "high", "max", "low")
        #[arg(long)]
        variant: Option<String>,

        /// Dump raw request/response to stderr
        #[arg(long)]
        raw: bool,

        /// The prompt to send
        prompt: Vec<String>,
    },
    /// Start an interactive session
    Chat {
        /// Provider ID (defaults to last-used or opencode-zen)
        #[arg(long)]
        provider: Option<String>,

        /// Model ID (overrides provider default)
        #[arg(long)]
        model: Option<String>,

        /// Thinking variant (e.g. "high", "max", "low")
        #[arg(long)]
        variant: Option<String>,

        /// Dump raw request/response to stderr
        #[arg(long)]
        raw: bool,

        /// Connect to an external ACP agent
        #[arg(long)]
        acp_agent: Option<String>,
    },
    /// Run as an ACP server (exposes agent core over stdio ACP)
    Acp {
        /// Provider ID (defaults to last-used or opencode-zen)
        #[arg(long)]
        provider: Option<String>,

        /// Model ID
        #[arg(long)]
        model: Option<String>,

        /// Dump raw request/response
        #[arg(long)]
        raw: bool,
    },
    /// View or edit configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Print the current configuration to stdout
    Show,
    /// Open the config file in $EDITOR (or $VISUAL, or vi)
    Edit,
    /// Interactive TUI config editor
    Editor,
    /// Print the path to the config file
    Path,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Load runtime state for fallback defaults.
    let state = mew_config::load_state().unwrap_or_default();

    match cli.command {
        Some(Commands::Run {
            provider,
            model,
            variant,
            raw,
            prompt,
        }) => {
            let provider = resolve_provider(provider, &state);
            let model = resolve_model_opt(model, &state);
            run_cmd(provider, model, variant, raw, prompt).await
        }
        Some(Commands::Chat {
            provider,
            model,
            variant,
            raw,
            acp_agent,
        }) => {
            if let Some(agent_cmd) = acp_agent {
                chat_with_acp(&agent_cmd).await
            } else {
                let provider = resolve_provider(provider, &state);
                let model = resolve_model_opt(model, &state);
                chat_cmd(provider, model, variant, raw).await
            }
        }
        Some(Commands::Acp {
            provider,
            model,
            raw,
        }) => {
            let provider = resolve_provider(provider, &state);
            let model = resolve_model_opt(model, &state);
            run_acp_server(&provider, model, raw).await
        }
        None => {
            let provider = resolve_provider(None, &state);
            let model = resolve_model_opt(None, &state);
            chat_cmd(provider, model, None, false).await
        }
        Some(Commands::Config { command }) => {
            config_cmd(command)?;
            Ok(())
        }
    }
}

fn resolve_provider(cli: Option<String>, state: &mew_config::State) -> String {
    cli.or_else(|| {
        if state.last_provider.is_empty() {
            None
        } else {
            Some(state.last_provider.clone())
        }
    })
    .unwrap_or_else(|| "opencode-zen".to_string())
}

fn resolve_model_opt(cli: Option<String>, state: &mew_config::State) -> Option<String> {
    cli.or_else(|| {
        if state.last_model.is_empty() {
            None
        } else {
            Some(state.last_model.clone())
        }
    })
}

async fn load_catalog(cfg: &Config) -> Option<Catalog> {
    let mut cat = match mew_catalog::load().await {
        Ok(c) => c,
        Err(e) => {
            warn!("catalog load failed: {}", e);
            return None;
        }
    };
    let custom: Vec<mew_catalog::Model> = cfg
        .models
        .iter()
        .map(|cm| mew_catalog::Model {
            id: cm.id.clone(),
            provider: cm.provider.clone(),
            shape: cm.shape.clone(),
            context_window: cm.context_window,
            thinking_variants: cm
                .thinking_variants
                .iter()
                .map(|v| mew_catalog::ThinkingVariant {
                    name: v.name.clone(),
                    params: v.params.clone(),
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    cat.merge_local(custom);
    Some(cat)
}

fn resolve_reasoning(
    cat: Option<&Catalog>,
    model_id: &str,
    variant_name: Option<&str>,
) -> Option<mew_provider::ReasoningConfig> {
    let cat = cat?;
    let variants = cat.thinking_variants(model_id);
    if variants.is_empty() {
        return None;
    }
    let variant = match variant_name {
        Some("none") => return None,
        Some(name) => variants.iter().find(|v| v.name == name)?.clone(),
        None => cat.default_thinking(model_id)?,
    };
    let params = variant.params.as_object().cloned().unwrap_or_default();
    Some(mew_provider::ReasoningConfig { params })
}

fn config_cmd(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Path => {
            println!("{}", mew_config::config_dir().join("config.toml").display());
        }
        ConfigCommands::Show => {
            let cfg = mew_config::load().context("load config")?;
            let toml = toml::to_string_pretty(&cfg)
                .map_err(|e| anyhow::anyhow!("serialize config: {}", e))?;
            print!("{}", toml);
        }
        ConfigCommands::Editor => {
            config_editor::run_editor()?;
        }
        ConfigCommands::Edit => {
            let config_dir = mew_config::config_dir();
            let config_path = config_dir.join("config.toml");

            std::fs::create_dir_all(&config_dir).context("create config directory")?;

            if !config_path.exists() {
                let template = "# mew configuration file\n\
                    # Docs: https://github.com/anomalyco/mew\n\n\
                    # default_model = \"deepseek-v4-flash\"\n\n\
                    # [providers.my-provider]\n\
                    # shape = \"openai\"\n\
                    # base_url = \"https://api.example.com/v1\"\n\
                    # credential_ref = \"my-provider\"\n\n\
                    # [[permissions.rules]]\n\
                    # tool = \"bash\"\n\
                    # decision = \"allow\"\n\
                    # match.command_prefix = \"git \"\n";
                std::fs::write(&config_path, template).context("write config template")?;
                info!("created config template at {}", config_path.display());
            }

            let editor = std::env::var("VISUAL")
                .or_else(|_| std::env::var("EDITOR"))
                .unwrap_or_else(|_| "vi".to_string());

            let mut parts = editor.split_whitespace();
            let cmd = parts.next().unwrap_or("vi");
            let extra_args: Vec<&str> = parts.collect();

            let status = std::process::Command::new(cmd)
                .args(&extra_args)
                .arg(&config_path)
                .status()
                .with_context(|| format!("failed to launch editor '{}'", editor))?;

            if !status.success() {
                anyhow::bail!("editor exited with non-zero status");
            }
        }
    }
    Ok(())
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
        Arc::new(ExitTool),
        Arc::new(ProgressUpdate),
    ];
    if !skills.is_empty() {
        tools.push(Arc::new(Skill::new(skills)));
    }
    tools
}

fn build_permission_engine(cfg: &Config) -> Arc<mew_config::permissions::PermissionEngine> {
    Arc::new(mew_config::permissions::PermissionEngine::new(
        cfg.permissions.rules.clone(),
    ))
}

fn plugin_storage_map() -> std::collections::HashMap<String, String> {
    let dir = plugin_storage_dir();
    if !dir.exists() {
        return std::collections::HashMap::new();
    }
    let mut map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(json_map) =
                        serde_json::from_str::<std::collections::HashMap<String, String>>(&content)
                    {
                        for (k, v) in json_map {
                            map.insert(format!("{}/{}", name, k), v);
                        }
                    }
                }
            }
        }
    }
    map
}

fn plugin_storage_dir() -> std::path::PathBuf {
    mew_config::config_dir().join("plugin-storage")
}

fn write_plugin_storage(
    map: &mut std::collections::HashMap<String, String>,
    key: &str,
    value: &str,
) {
    map.insert(key.to_string(), value.to_string());
    let dir = plugin_storage_dir();
    let _ = std::fs::create_dir_all(&dir);
    // Group by plugin name (first segment of key before /)
    let mut per_plugin: std::collections::HashMap<
        String,
        std::collections::HashMap<String, String>,
    > = std::collections::HashMap::new();
    for (k, v) in map.iter() {
        if let Some((plugin, subkey)) = k.split_once('/') {
            per_plugin
                .entry(plugin.to_string())
                .or_default()
                .insert(subkey.to_string(), v.clone());
        }
    }
    for (plugin, data) in &per_plugin {
        let path = dir.join(format!("{}.json", plugin));
        let tmp = dir.join(format!("{}.tmp", plugin));
        if let Ok(json) = serde_json::to_string(data) {
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn build_dispatcher(
    notify: impl Fn(String) + Send + Sync + 'static,
    config_read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    log_fn: impl Fn(String) + Send + Sync + 'static,
    storage_read: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    storage_write: impl Fn(&str, &str) + Send + Sync + 'static,
    storage_delete: impl Fn(&str) + Send + Sync + 'static,
    set_ui: impl Fn(&str, &str) + Send + Sync + 'static,
    disabled_plugins: &[String],
) -> Arc<dyn Dispatcher> {
    let host = PluginHost {
        notify: Arc::new(notify),
        config_read: Arc::new(config_read),
        log: Arc::new(log_fn),
        storage_read: Arc::new(storage_read),
        storage_write: Arc::new(storage_write),
        storage_delete: Arc::new(storage_delete),
        set_ui: Arc::new(set_ui),
    };

    match SubprocessDispatcher::from_default_dirs_filtered(host.clone(), disabled_plugins).await {
        Ok(d) => {
            tracing::info!("plugin runtime loaded");
            Arc::new(d)
        }
        Err(e) => {
            tracing::warn!("plugin runtime unavailable, using no-op: {}", e);
            Arc::new(NopDispatcher)
        }
    }
}

/// Load MCP server configs from standard locations.
///
/// Searches (in order):
///   cwd/mcp.json, cwd/.mcp.json, cwd/.mew/mcp.json, cwd/.mew/.mcp.json
/// Each file uses Claude-Code-compatible format:
///   { "mcpServers": { "name": { "command": "...", "args": [...], "type": "stdio"|"http" } } }
fn load_mcp_configs() -> Vec<mew_mcp::McpServerConfig> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let paths = [
        cwd.join("mcp.json"),
        cwd.join(".mcp.json"),
        cwd.join(".mew").join("mcp.json"),
        cwd.join(".mew").join(".mcp.json"),
    ];

    let mut configs = Vec::new();

    for path in &paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!("failed to read {}: {}", path.display(), e);
                continue;
            }
        };

        let v: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to parse {}: {}", path.display(), e);
                continue;
            }
        };

        let Some(servers) = v.get("mcpServers").and_then(|s| s.as_object()) else {
            continue;
        };

        for (name, cfg) in servers {
            let command = cfg
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from);
            let url = cfg.get("url").and_then(|v| v.as_str()).map(String::from);
            let type_ = cfg.get("type").and_then(|v| v.as_str()).map(String::from);
            let args = cfg
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v: &serde_json::Value| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            configs.push(mew_mcp::McpServerConfig {
                name: name.clone(),
                type_,
                url,
                command,
                args,
            });
        }
    }

    configs
}

async fn chat_with_acp(agent_cmd: &str) -> Result<()> {
    use std::sync::Arc;

    let parts: Vec<&str> = agent_cmd.split_whitespace().collect();
    if parts.is_empty() {
        anyhow::bail!("empty acp agent command");
    }
    let command = parts[0];
    let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    let cwd_str = cwd.to_string_lossy().to_string();

    let acp_client = mew_acp::AcpClient::connect(command, &args, &cwd_str).await?;
    let acp_client = Arc::new(tokio::sync::Mutex::new(acp_client));

    let session_id = {
        let client = acp_client.lock().await;
        client.session_id().to_string()
    };

    // Setup terminal and run the TUI.
    let mut app = mew_tui::App::new();
    app.status.model = "acp-agent".to_string();
    app.status.provider = "acp".to_string();
    app.status.session_id = session_id;

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

    let result = loop {
        if let Err(e) = terminal.draw(|f| mew_tui::ui::draw(f, &mut app)) {
            break Err(anyhow::anyhow!("draw error: {}", e));
        }

        let event = match event_rx.recv().await {
            Some(e) => e,
            None => break Ok(()),
        };

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
                            let client = acp_client.clone();
                            let ev_loop = event_loop.clone();
                            tokio::spawn(async move {
                                match client.lock().await.run_turn(&enriched).await {
                                    Ok(rx) => ev_loop.forward_agent_events(rx),
                                    Err(e) => tracing::error!("acp turn failed: {e}"),
                                }
                            });
                        }
                        mew_tui::events::Action::Quit => should_break = true,
                        mew_tui::events::Action::Cancel => {
                            app.streaming = false;
                            let client = acp_client.clone();
                            tokio::spawn(async move {
                                if let Err(e) = client.lock().await.cancel().await {
                                    tracing::error!("acp cancel failed: {e}");
                                }
                            });
                        }
                        mew_tui::events::Action::Clear => {
                            app.clear_messages();
                        }
                        mew_tui::events::Action::ToggleSidebarContext => {
                            app.toggle_sidebar_section("context");
                        }
                        mew_tui::events::Action::ToggleSidebarTools => {
                            app.toggle_sidebar_section("tools");
                        }
                        mew_tui::events::Action::ToggleSidebarMcp => {
                            app.toggle_sidebar_section("mcp");
                        }
                        _ => {}
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

        // Drain remaining events.
        loop {
            let ok = match event_rx.try_recv() {
                Ok(mew_tui::Event::Agent(event)) => {
                    app.handle_agent_event(event);
                    true
                }
                Ok(mew_tui::Event::Tick) => {
                    app.tick();
                    true
                }
                Ok(mew_tui::Event::Quit) => {
                    should_break = true;
                    false
                }
                _ => false,
            };
            if !ok {
                break;
            }
        }

        if should_break || app.should_quit {
            break Ok(());
        }
    };

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_acp_server(provider_flag: &str, model_flag: Option<String>, raw: bool) -> Result<()> {
    let cfg = mew_config::load().context("load config")?;
    let cat = load_catalog(&cfg).await;
    let cat_for_resolver = cat.clone();
    let cat_ref = cat.as_ref();

    let (provider_id, model_id) = resolve_model(&cfg, cat_ref, provider_flag, model_flag);

    let provider =
        build_provider(&cfg, cat_ref, &provider_id, &model_id, raw).context("build provider")?;

    let dispatcher = Arc::new(NopDispatcher);
    let skills_loader = mew_skills::Loader::new(std::env::current_dir().unwrap_or_default());
    let skills = Arc::new(skills_loader.load().unwrap_or_default());
    let tools = build_tools(skills.clone());

    let flagged_files: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let mut tools = tools;
    tools.push(Arc::new(FlagImportant::new(flagged_files.clone())));

    let permission_engine = build_permission_engine(&cfg);

    let mut agent = Agent::new(provider, dispatcher.clone(), None, tools, None);
    agent.flagged_files = flagged_files;
    agent.set_permission_engine(permission_engine);
    if cfg.workspace.roots.is_empty() {
        agent.workspace_roots = vec![std::env::current_dir().unwrap_or_default()];
    } else {
        agent.workspace_roots = cfg.workspace.roots.clone();
    }

    // Wire up subagent infrastructure.
    let subagent_defs = {
        let loader = mew_subagents::Loader::new(std::env::current_dir().unwrap_or_default());
        Arc::new(loader.load().unwrap_or_default())
    };
    if !subagent_defs.is_empty() {
        let resolver = Arc::new(MainModelResolver {
            cfg: Arc::new(cfg.clone()),
            cat: cat_for_resolver.map(Arc::new),
            default_provider_id: provider_id.clone(),
            raw,
        });
        let runner = mew_agent::runner::SimpleRunner::new(
            agent.provider.clone(),
            agent.tools.values().cloned().collect(),
            dispatcher.clone(),
        )
        .with_model_resolver(resolver);
        agent.subagent_runner = Some(Arc::new(runner));
        agent.subagent_defs = subagent_defs.to_vec();
        agent.tools.insert(
            "subagent_start".into(),
            Arc::new(mew_tools::tools::subagent_start::SubagentStart::new(
                subagent_defs.clone(),
            )),
        );
        agent.tools.insert(
            "subagent_wait".into(),
            Arc::new(mew_tools::tools::subagent_wait::SubagentWait::new()),
        );
    }

    // Load project context and skills for system prompt.
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

    if let Some(c) = cat_ref {
        agent.supports_vision = c.supports_vision(&model_id);
        agent.context_window = c.context_window(&model_id).max(0) as u32;
        if let Some(m) = c.lookup(&model_id) {
            agent.input_price = m.pricing.input;
            agent.output_price = m.pricing.output;
            agent.cache_read_price = m.pricing.cache_read;
            agent.cache_write_price = m.pricing.cache_write;
            agent.reasoning_price = m.pricing.reasoning;
        }
    }

    info!("mew acp server starting, model={model_id}");
    mew_acp::run_server(agent).await
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
) -> (
    Vec<Arc<dyn mew_tools::Tool>>,
    Vec<Arc<mew_mcp::McpClient>>,
    Vec<String>,
) {
    let mut tools: Vec<Arc<dyn mew_tools::Tool>> = Vec::new();
    let mut clients: Vec<Arc<mew_mcp::McpClient>> = Vec::new();
    let mut status: Vec<String> = Vec::new();

    for cfg in configs {
        info!(server = %cfg.name, "connecting MCP server");

        let client = if let Some(ref url) = cfg.url {
            match McpClient::connect_http(&cfg.name, url).await {
                Ok(client) => {
                    status.push(format!("{} connected (http)", cfg.name));
                    client
                }
                Err(e) => {
                    status.push(format!("{} connection failed", cfg.name));
                    warn!(server = %cfg.name, error = %e, "MCP connect failed");
                    continue;
                }
            }
        } else if let Some(ref command) = cfg.command {
            match McpClient::connect_stdio(&cfg.name, command, &cfg.args).await {
                Ok(client) => {
                    status.push(format!("{} connected (stdio)", cfg.name));
                    client
                }
                Err(e) => {
                    status.push(format!("{} connection failed", cfg.name));
                    warn!(server = %cfg.name, error = %e, "MCP connect failed");
                    continue;
                }
            }
        } else {
            status.push(format!("{} skipped (no url or command)", cfg.name));
            warn!(server = %cfg.name, "MCP server has no url or command");
            continue;
        };

        match client.list_tools().await {
            Ok(mcp_tools) => {
                let count = mcp_tools.len();
                let client = Arc::new(client);
                for def in &mcp_tools {
                    tools.push(Arc::new(mew_mcp::McpTool::new(
                        &cfg.name,
                        def,
                        client.clone(),
                    )));
                }
                clients.push(client);
                status.push(format!("{} ready ({} tools)", cfg.name, count));
                info!(server = %cfg.name, tool_count = count, "MCP tools registered");
            }
            Err(e) => {
                status.push(format!("{} tool listing failed", cfg.name));
                warn!(server = %cfg.name, error = %e, "failed to list MCP tools");
            }
        }
    }

    (tools, clients, status)
}

async fn run_cmd(
    provider_flag: String,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let prompt = prompt_parts.join(" ");
    if prompt.is_empty() {
        anyhow::bail!("missing prompt");
    }

    let cfg = mew_config::load().context("load config")?;

    let cat = load_catalog(&cfg).await;

    build_and_run(
        &cfg,
        cat.as_ref(),
        &provider_flag,
        model_flag,
        variant_flag,
        raw,
        prompt,
    )
    .await
}

async fn chat_cmd(
    provider_flag: String,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
) -> Result<()> {
    let cfg = mew_config::load().context("load config")?;

    let cat = load_catalog(&cfg).await;

    run_tui(
        &cfg,
        cat.as_ref(),
        &provider_flag,
        model_flag,
        variant_flag,
        raw,
    )
    .await
}

/// Resolves a `provider/model` string into a `Provider`. Used by the
/// subagent runner to honor per-subagent `model:` overrides. Falls back to
/// the agent's current `provider_id` when the override has no `/`.
struct MainModelResolver {
    cfg: Arc<Config>,
    cat: Option<Arc<Catalog>>,
    default_provider_id: String,
    raw: bool,
}

#[async_trait]
impl mew_subagents::ModelResolver for MainModelResolver {
    async fn resolve(&self, model: &str) -> Result<Arc<dyn Provider>, String> {
        let (provider_id, model_id) = if let Some(idx) = model.find('/') {
            (&model[..idx], &model[idx + 1..])
        } else {
            (self.default_provider_id.as_str(), model)
        };
        build_provider(
            &self.cfg,
            self.cat.as_deref(),
            provider_id,
            model_id,
            self.raw,
        )
        .map_err(|e| e.to_string())
    }
}

async fn run_tui(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
) -> Result<()> {
    let cat_for_resolver = cat.cloned();
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let provider =
        build_provider(cfg, cat, &provider_id, &model_id, raw).context("build provider")?;

    // For router providers, use the big model for display.
    let (display_provider, display_model) = if let Some(pc) = cfg.providers.get(&provider_id) {
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

    // Plugin UI channel: plugins push content, we drain in the main loop.
    let (plugin_ui_tx, mut plugin_ui_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, String)>();

    let plugin_storage = Arc::new(std::sync::Mutex::new(plugin_storage_map()));

    let disabled_plugins = mew_config::load_state()
        .unwrap_or_default()
        .disabled_plugins;

    let dispatcher = build_dispatcher(
        |msg| {
            tracing::info!("[plugin-notify] {msg}");
        },
        |_key| None,
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

    // Load skills for the skill tool.
    let skills_loader = mew_skills::Loader::new(std::env::current_dir().unwrap_or_default());
    let loaded_skills = skills_loader.load().unwrap_or_default();
    let skills = Arc::new(loaded_skills);

    let mut tools = build_tools(skills.clone());

    let flagged_files: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    tools.push(Arc::new(FlagImportant::new(flagged_files.clone())));

    // Load MCP tools.
    let mcp_configs = load_mcp_configs();
    let mut _mcp_clients: Vec<Arc<McpClient>> = Vec::new();
    let mut mcp_server_status: Vec<(String, bool, usize)> = Vec::new();
    if !mcp_configs.is_empty() {
        let (mcp_tools, mcp_cls, status) = connect_mcp_servers(&mcp_configs).await;
        for cfg in &mcp_configs {
            let connected = status
                .iter()
                .any(|s| s.starts_with(&cfg.name) && s.contains("ready"));
            let count = if connected {
                mcp_tools
                    .iter()
                    .filter(|t| t.name().starts_with(&format!("{}__", cfg.name)))
                    .count()
            } else {
                0
            };
            mcp_server_status.push((cfg.name.clone(), connected, count));
        }
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
        .map(|f| context_display_name(&f.path))
        .collect();
    let tool_names: Vec<String> = tools
        .iter()
        .map(|t| t.name().to_string())
        .filter(|n| !n.contains("__"))
        .collect();

    let mut agent = Agent::new(
        provider,
        dispatcher.clone(),
        Some(session_writer),
        tools,
        None,
    );
    agent.flagged_files = flagged_files;
    agent.set_permission_engine(permission_engine);
    if cfg.workspace.roots.is_empty() {
        agent.workspace_roots = vec![std::env::current_dir().unwrap_or_default()];
    } else {
        agent.workspace_roots = cfg.workspace.roots.clone();
    }

    // Wire up subagent infrastructure.
    let subagent_defs = {
        let loader = mew_subagents::Loader::new(std::env::current_dir().unwrap_or_default());
        Arc::new(loader.load().unwrap_or_default())
    };
    if !subagent_defs.is_empty() {
        let resolver = Arc::new(MainModelResolver {
            cfg: Arc::new(cfg.clone()),
            cat: cat_for_resolver.map(Arc::new),
            default_provider_id: provider_id.clone(),
            raw,
        });
        let runner = mew_agent::runner::SimpleRunner::new(
            agent.provider.clone(),
            agent.tools.values().cloned().collect(),
            dispatcher.clone(),
        )
        .with_model_resolver(resolver);
        agent.subagent_runner = Some(Arc::new(runner));
        agent.subagent_defs = subagent_defs.to_vec();

        agent.tools.insert(
            "subagent_start".into(),
            Arc::new(mew_tools::tools::subagent_start::SubagentStart::new(
                subagent_defs.clone(),
            )),
        );
        agent.tools.insert(
            "subagent_wait".into(),
            Arc::new(mew_tools::tools::subagent_wait::SubagentWait::new()),
        );
    }

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

    let reasoning = resolve_reasoning(cat, &model_id, variant_flag.as_deref());
    if let Some(r) = reasoning {
        agent.set_reasoning(Some(r));
        info!(variant = ?variant_flag, model = %model_id, "enabled thinking variant");
    }

    let mut app = mew_tui::App::new();

    // Populate MCP server status in sidebar
    for (name, ok, count) in &mcp_server_status {
        if !ok {
            app.messages
                .push(synthetic_message(format!("{name} connection failed")));
        }
        app.mcp_status.push((name.clone(), *ok, *count));
    }

    // Restore sidebar collapsed state from previous session.
    let prev_state = mew_config::load_state().unwrap_or_default();
    app.sidebar_collapsed = prev_state.sidebar_collapsed.clone();

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
    let mut settings_editor: Option<config_editor::ConfigEditor> = None;
    let result = loop {
        // Drain plugin UI updates before each render.
        while let Ok((key, value)) = plugin_ui_rx.try_recv() {
            app.plugin_ui.insert(key.clone(), value.clone());
            if key == "buddy/bubble" {
                app.touch_companion_bubble();
            }
        }

        // Render.
        if let Err(e) = terminal.draw(|f| {
            mew_tui::ui::draw(f, &mut app);
            if let Some(ref editor) = settings_editor {
                editor.draw(f);
            }
        }) {
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
                // Settings mode: delegate to ConfigEditor
                if app.mode == mew_tui::app::Mode::Settings {
                    if let crossterm::event::Event::Key(key) = crossterm_event {
                        if let Some(ref mut editor) = settings_editor {
                            if !editor.handle_key(key) {
                                // Closing settings
                                if editor.is_dirty() {
                                    app.set_alert("Settings closed (unsaved changes discarded)");
                                }
                                settings_editor = None;
                                app.mode = mew_tui::app::Mode::Normal;
                            }
                        }
                    }
                    continue;
                }

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
                                mew_tui::SlashResult::Continue => {
                                    continue;
                                }
                                mew_tui::SlashResult::Quit => should_break = true,
                                mew_tui::SlashResult::Clear => {
                                    agent.clear_context().await;
                                    app.clear_messages();
                                    app.messages
                                        .push(synthetic_message("context cleared".into()));
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
                                    let (new_provider_id, new_model_id) =
                                        if let Some(idx) = new_model.find('/') {
                                            (&new_model[..idx], &new_model[idx + 1..])
                                        } else {
                                            (provider_id.as_str(), new_model.as_str())
                                        };
                                    match build_provider(
                                        cfg,
                                        cat,
                                        new_provider_id,
                                        new_model_id,
                                        raw,
                                    ) {
                                        Ok(new_provider) => {
                                            agent.provider = new_provider;
                                            app.status.model = new_model_id.to_string();
                                            app.status.provider = new_provider_id.to_string();
                                            if let Some(c) = cat {
                                                app.status.context_window =
                                                    c.context_window(new_model_id) as u32;
                                                if let Some(m) = c.lookup(new_model_id) {
                                                    agent.input_price = m.pricing.input;
                                                    agent.output_price = m.pricing.output;
                                                    agent.cache_read_price = m.pricing.cache_read;
                                                    agent.cache_write_price = m.pricing.cache_write;
                                                    agent.reasoning_price = m.pricing.reasoning;
                                                }
                                            }
                                            let mut state =
                                                mew_config::load_state().unwrap_or_default();
                                            state.last_model = new_model_id.to_string();
                                            state.last_provider = new_provider_id.to_string();
                                            if let Err(e) = mew_config::save_state(&state) {
                                                tracing::warn!("failed to save state: {}", e);
                                            }
                                            app.messages.push(synthetic_message(format!(
                                                "switched to {}",
                                                new_model
                                            )));
                                        }
                                        Err(e) => {
                                            app.messages.push(synthetic_message(format!(
                                                "failed to switch: {}",
                                                e
                                            )));
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
                                            app.messages.push(synthetic_message(format!(
                                                "resumed session {}",
                                                id
                                            )));
                                        }
                                        Err(e) => {
                                            app.messages.push(synthetic_message(format!(
                                                "failed to load session {}: {}",
                                                id, e
                                            )));
                                        }
                                    }
                                }
                                mew_tui::SlashResult::OpenModelPicker => {
                                    app.open_command_palette();
                                }
                                mew_tui::SlashResult::ToggleMouseCapture => {
                                    toggle_mouse_capture(&mut app, &mut terminal).await;
                                }
                                mew_tui::SlashResult::PluginCommand { name, args } => {
                                    let disp = agent.dispatcher.clone();
                                    match disp.execute_slash_command(&name, &args).await {
                                        Some(result) => {
                                            app.messages.push(synthetic_message(result));
                                        }
                                        None => {
                                            app.messages.push(synthetic_message(format!(
                                                "unknown command: {}",
                                                name
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        mew_tui::events::Action::Clear => {
                            agent.clear_context().await;
                            app.clear_messages();
                            app.messages
                                .push(synthetic_message("context cleared".into()));
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
                                        app.status.context_window =
                                            c.context_window(new_model_id) as u32;
                                        if let Some(m) = c.lookup(new_model_id) {
                                            agent.input_price = m.pricing.input;
                                            agent.output_price = m.pricing.output;
                                            agent.cache_read_price = m.pricing.cache_read;
                                            agent.cache_write_price = m.pricing.cache_write;
                                            agent.reasoning_price = m.pricing.reasoning;
                                        }
                                    }
                                    let mut state = mew_config::load_state().unwrap_or_default();
                                    state.last_model = new_model_id.to_string();
                                    state.last_provider = new_provider_id.to_string();
                                    state.sidebar_collapsed = app.sidebar_collapsed.clone();
                                    if let Err(e) = mew_config::save_state(&state) {
                                        tracing::warn!("failed to save state: {}", e);
                                    }
                                    app.messages.push(synthetic_message(format!(
                                        "switched to {}",
                                        new_model
                                    )));
                                }
                                Err(e) => {
                                    app.messages.push(synthetic_message(format!(
                                        "failed to switch model: {}",
                                        e
                                    )));
                                }
                            }
                        }
                        mew_tui::events::Action::Cancel => {
                            agent.cancel_token.cancel();
                            app.streaming = false;
                        }
                        mew_tui::events::Action::CancelMostRecentSubagent(task_id) => {
                            if agent.cancel_subagent(&task_id).await {
                                app.set_alert("subagent cancellation requested");
                            } else {
                                app.set_alert("subagent already finished");
                            }
                        }
                        mew_tui::events::Action::InsertAtMention(mention) => {
                            app.input.push_str(&mention);
                            app.cursor += mention.len();
                        }
                        mew_tui::events::Action::InsertSubagentMention(name) => {
                            let mention = format!("@{} ", name);
                            app.input.push_str(&mention);
                            app.cursor += mention.len();
                        }
                        mew_tui::events::Action::CopySelection(text) => {
                            copy_to_clipboard(&text);
                            app.set_alert(format!("copied {} chars", text.len()));
                            app.clear_selection();
                        }
                        mew_tui::events::Action::ToggleSidebarContext => {
                            app.toggle_sidebar_section("context");
                        }
                        mew_tui::events::Action::ToggleSidebarTools => {
                            app.toggle_sidebar_section("tools");
                        }
                        mew_tui::events::Action::ToggleSidebarMcp => {
                            app.toggle_sidebar_section("mcp");
                        }
                        mew_tui::events::Action::OpenSettings => {
                            // Create ConfigEditor with discovered plugins
                            let loader = mew_hooks_runtime::PluginLoader::new(
                                mew_hooks_runtime::PluginLoader::default_dirs(),
                            );
                            let state = mew_config::load_state().unwrap_or_default();
                            let plugins: Vec<config_editor::PluginEntry> = loader
                                .discover_executables()
                                .into_iter()
                                .map(|path| {
                                    let name = path
                                        .file_stem()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string();
                                    let enabled = !state.disabled_plugins.contains(&name);
                                    config_editor::PluginEntry {
                                        name,
                                        path: path.display().to_string(),
                                        enabled,
                                    }
                                })
                                .collect();
                            let cfg = mew_config::load().unwrap_or_default();
                            settings_editor = Some(config_editor::ConfigEditor::new(cfg, plugins));
                            app.mode = mew_tui::app::Mode::Settings;
                        }
                        mew_tui::events::Action::SaveSettings
                        | mew_tui::events::Action::SettingsEditStart
                        | mew_tui::events::Action::SettingsEditComplete => {
                            // Handled by ConfigEditor in settings mode — ignore here
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
                    // Settings mode: delegate to ConfigEditor
                    if app.mode == mew_tui::app::Mode::Settings {
                        if let crossterm::event::Event::Key(key) = crossterm_event {
                            if let Some(ref mut editor) = settings_editor {
                                if !editor.handle_key(key) {
                                    if editor.is_dirty() {
                                        app.set_alert(
                                            "Settings closed (unsaved changes discarded)",
                                        );
                                    }
                                    settings_editor = None;
                                    app.mode = mew_tui::app::Mode::Normal;
                                }
                            }
                        }
                        continue;
                    }

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
                                agent.clear_context().await;
                                app.clear_messages();
                                app.messages
                                    .push(synthetic_message("context cleared".into()));
                            }
                            mew_tui::events::Action::ToggleSidebarContext => {
                                app.toggle_sidebar_section("context");
                            }
                            mew_tui::events::Action::ToggleSidebarTools => {
                                app.toggle_sidebar_section("tools");
                            }
                            mew_tui::events::Action::ToggleSidebarMcp => {
                                app.toggle_sidebar_section("mcp");
                            }
                            mew_tui::events::Action::CancelMostRecentSubagent(task_id) => {
                                if agent.cancel_subagent(&task_id).await {
                                    app.set_alert("subagent cancellation requested");
                                } else {
                                    app.set_alert("subagent already finished");
                                }
                            }
                            mew_tui::events::Action::InsertAtMention(mention) => {
                                app.input.push_str(&mention);
                                app.cursor += mention.len();
                            }
                            mew_tui::events::Action::InsertSubagentMention(name) => {
                                let mention = format!("@{} ", name);
                                app.input.push_str(&mention);
                                app.cursor += mention.len();
                            }
                            mew_tui::events::Action::CopySelection(text) => {
                                copy_to_clipboard(&text);
                                app.set_alert(format!("copied {} chars", text.len()));
                                app.clear_selection();
                            }
                            mew_tui::events::Action::SlashCommand(text) => {
                                match app.handle_slash(&text) {
                                    mew_tui::SlashResult::Continue => {}
                                    mew_tui::SlashResult::Quit => {
                                        should_break = true;
                                        break 'drain;
                                    }
                                    mew_tui::SlashResult::Clear => {
                                        agent.clear_context().await;
                                        app.clear_messages();
                                        app.messages
                                            .push(synthetic_message("context cleared".into()));
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
                                    mew_tui::SlashResult::ToggleMouseCapture => {
                                        // Deferred to main loop; ignored during drain.
                                    }
                                    mew_tui::SlashResult::PluginCommand { name, args } => {
                                        // Deferred to main loop; ignored during drain.
                                        let _ = name;
                                        let _ = args;
                                    }
                                }
                            }
                            mew_tui::events::Action::SwitchModel(_) => {
                                // Model switches happen via the command palette, which
                                // requires mode=CommandPalette. The palette is never
                                // open during heavy streaming, so this branch is
                                // effectively unreachable in the drain path.
                            }
                            mew_tui::events::Action::SaveSettings
                            | mew_tui::events::Action::SettingsEditStart
                            | mew_tui::events::Action::SettingsEditComplete => {
                                // Handled by ConfigEditor in settings mode
                            }
                            mew_tui::events::Action::OpenSettings => {
                                let loader = mew_hooks_runtime::PluginLoader::new(
                                    mew_hooks_runtime::PluginLoader::default_dirs(),
                                );
                                let state = mew_config::load_state().unwrap_or_default();
                                let plugins: Vec<config_editor::PluginEntry> = loader
                                    .discover_executables()
                                    .into_iter()
                                    .map(|path| {
                                        let name = path
                                            .file_stem()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .to_string();
                                        let enabled = !state.disabled_plugins.contains(&name);
                                        config_editor::PluginEntry {
                                            name,
                                            path: path.display().to_string(),
                                            enabled,
                                        }
                                    })
                                    .collect();
                                let cfg = mew_config::load().unwrap_or_default();
                                settings_editor =
                                    Some(config_editor::ConfigEditor::new(cfg, plugins));
                                app.mode = mew_tui::app::Mode::Settings;
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

    // Save sidebar collapsed state for next session.
    {
        let mut save = mew_config::load_state().unwrap_or_default();
        save.last_model = display_model.clone();
        save.last_provider = display_provider.clone();
        save.sidebar_collapsed = app.sidebar_collapsed.clone();
        let _ = mew_config::save_state(&save);
    }

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
    variant_flag: Option<String>,
    raw: bool,
    prompt: String,
) -> Result<()> {
    let cat_for_resolver = cat.cloned();
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let provider =
        build_provider(cfg, cat, &provider_id, &model_id, raw).context("build provider")?;

    let (_display_provider, display_model) = if let Some(pc) = cfg.providers.get(&provider_id) {
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

    let flagged_files: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    tools.push(Arc::new(FlagImportant::new(flagged_files.clone())));

    // Load MCP tools.
    let mcp_configs = load_mcp_configs();
    let mut _mcp_clients: Vec<Arc<McpClient>> = Vec::new();
    if !mcp_configs.is_empty() {
        let (mcp_tools, mcp_cls, _status) = connect_mcp_servers(&mcp_configs).await;
        tools.extend(mcp_tools);
        _mcp_clients = mcp_cls;
    }

    let permission_engine = build_permission_engine(cfg);

    let mut agent = Agent::new(
        provider,
        dispatcher.clone(),
        Some(session_writer),
        tools,
        None,
    );
    agent.flagged_files = flagged_files;
    agent.set_permission_engine(permission_engine);
    if cfg.workspace.roots.is_empty() {
        agent.workspace_roots = vec![std::env::current_dir().unwrap_or_default()];
    } else {
        agent.workspace_roots = cfg.workspace.roots.clone();
    }

    // Wire up subagent infrastructure.
    let subagent_defs = {
        let loader = mew_subagents::Loader::new(std::env::current_dir().unwrap_or_default());
        Arc::new(loader.load().unwrap_or_default())
    };
    if !subagent_defs.is_empty() {
        let resolver = Arc::new(MainModelResolver {
            cfg: Arc::new(cfg.clone()),
            cat: cat_for_resolver.map(Arc::new),
            default_provider_id: provider_id.clone(),
            raw,
        });
        let runner = mew_agent::runner::SimpleRunner::new(
            agent.provider.clone(),
            agent.tools.values().cloned().collect(),
            dispatcher.clone(),
        )
        .with_model_resolver(resolver);
        agent.subagent_runner = Some(Arc::new(runner));
        agent.subagent_defs = subagent_defs.to_vec();
        agent.tools.insert(
            "subagent_start".into(),
            Arc::new(mew_tools::tools::subagent_start::SubagentStart::new(
                subagent_defs.clone(),
            )),
        );
        agent.tools.insert(
            "subagent_wait".into(),
            Arc::new(mew_tools::tools::subagent_wait::SubagentWait::new()),
        );
    }

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

    let reasoning = resolve_reasoning(cat, &model_id, variant_flag.as_deref());
    if let Some(r) = reasoning {
        agent.set_reasoning(Some(r));
        info!(variant = ?variant_flag, model = %model_id, "enabled thinking variant");
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
            mew_agent::AgentEvent::SubagentStart {
                name: _,
                child_session_id: _,
                ..
            } => {}
            mew_agent::AgentEvent::SubagentProgress { .. } => {}
            mew_agent::AgentEvent::SubagentStatus { .. } => {}
            mew_agent::AgentEvent::SubagentEnd {
                child_session_id: _,
                ..
            } => {}
            mew_agent::AgentEvent::SubagentPermissionRequest { call, tx, .. } => {
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
                let _ = call;
            }
            mew_agent::AgentEvent::WorkspacePermissionRequest { tx, .. } => {
                // Non-interactive mode: auto-allow workspace access.
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
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
            router, big_pid, model,
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

async fn toggle_mouse_capture(
    app: &mut mew_tui::App,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) {
    app.mouse_capture = !app.mouse_capture;
    if app.mouse_capture {
        let _ = crossterm::execute!(terminal.backend_mut(), crossterm::event::EnableMouseCapture);
        app.messages.push(synthetic_message(
            "mouse capture enabled (use /mouse to select text)".into(),
        ));
    } else {
        let _ = crossterm::execute!(
            terminal.backend_mut(),
            crossterm::event::DisableMouseCapture,
        );
        app.messages.push(synthetic_message(
            "mouse capture disabled \u{2014} native text selection enabled".into(),
        ));
    }
}

fn copy_to_clipboard(text: &str) {
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

/// Produce a short display label for a context file path.
fn context_display_name(path: &std::path::Path) -> String {
    let leaf = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
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
