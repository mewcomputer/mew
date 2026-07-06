use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use std::io::Write as _;
use std::sync::Arc;
use tracing::{info, warn};

use async_trait::async_trait;

mod config_editor;

use mew_agent::Agent;
use mew_catalog::Catalog;
use mew_config::{Config, ProviderConfig};
use mew_hooks::{Dispatcher, NopDispatcher, PluginHost};
use mew_hooks_runtime::SubprocessDispatcher;
use mew_mcp::McpClient;
use mew_message::{Finish, Part, PartId, Role, SessionId};
use mew_provider::Provider;
use mew_provider_anthropic::Adapter as AnthropicAdapter;
use mew_provider_openai::Adapter as OpenAIAdapter;
use mew_session::Writer as SessionWriter;
use mew_tools::tools::ask_user::AskUser;
use mew_tools::tools::bash::Bash;
use mew_tools::tools::echo::Echo;
use mew_tools::tools::edit_hashline::EditHashline;
use mew_tools::tools::edit_str_replace::EditStrReplace;
use mew_tools::tools::exit_tool::ExitTool;
use mew_tools::tools::flag_important::{FlagImportant, FlaggedFile};
use mew_tools::tools::glob::Glob;
use mew_tools::tools::grep::Grep;
use mew_tools::tools::jobs::{JobBlock, JobCancel, JobStatus, ShellBackground, ShellMonitor};
use mew_tools::tools::progress_update::ProgressUpdate;
use mew_tools::tools::read::Read;
use mew_tools::tools::skill::Skill;
use mew_tools::tools::switch_persona::SwitchPersona as SwitchPersonaTool;
use mew_tools::tools::todo::{TodoComplete, TodoCreate, TodoDelete, TodoListTool, TodoUpdate};
use mew_tools::tools::web_fetch::WebFetch;
use mew_tools::tools::write::Write;
use mew_tools::SecretSet;

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

        /// Auto-allow Mutating tools (write/edit/etc.); bash still prompts.
        /// Equivalent to `/permissions permissive`. `-D` and `-A` win if set.
        #[arg(long, short = 'P', env = "MEW_PERMISSIVE")]
        permissive: bool,

        /// Route every tool call through a small LLM classifier
        /// (allow/deny/escalate). Equivalent to `/permissions auto`.
        /// `-D` wins if both are set. Requires a classifier provider.
        #[arg(long, short = 'A', env = "MEW_AUTO")]
        auto: bool,

        /// Like `--auto`, but the classifier CANNOT escalate — escalate or
        /// failure means Deny (fail closed). Equivalent to
        /// `/permissions auto_plus`. Wins over `-A` if both are set.
        #[arg(long, env = "MEW_AUTO_PLUS")]
        auto_plus: bool,

        /// Skip all permission prompts. Every tool auto-runs and overrides
        /// deny rules, ask rules, and the secret-file guard. Equivalent to
        /// `/permissions dangerous`. Wins over `-P`, `-A`, and
        /// `--auto-plus` if set.
        #[arg(long, short = 'D', env = "MEW_DANGEROUS")]
        dangerously_skip_permissions: bool,

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

        /// Auto-allow Mutating tools (write/edit/etc.); bash still prompts.
        /// Equivalent to `/permissions permissive`.
        #[arg(long, short = 'P', env = "MEW_PERMISSIVE")]
        permissive: bool,

        /// Route every tool call through a small LLM classifier
        /// (allow/deny/escalate). Equivalent to `/permissions auto`.
        #[arg(long, short = 'A', env = "MEW_AUTO")]
        auto: bool,

        /// Like `--auto`, but the classifier CANNOT escalate — escalate or
        /// failure means Deny (fail closed). Equivalent to
        /// `/permissions auto_plus`.
        #[arg(long, env = "MEW_AUTO_PLUS")]
        auto_plus: bool,

        /// Skip all permission prompts. Every tool auto-runs and overrides
        /// deny rules, ask rules, and the secret-file guard. Equivalent to
        /// `/permissions dangerous`. Can be toggled at runtime via the
        /// `/permissions` slash command.
        #[arg(long, short = 'D', env = "MEW_DANGEROUS")]
        dangerously_skip_permissions: bool,

        /// Connect to a mew daemon at the given WebSocket URL
        /// (e.g. "ws://unix:/tmp/mew.sock")
        #[arg(long)]
        connect: Option<String>,

        /// Attach to an existing session by ID instead of creating a new one.
        /// Requires --connect. Use `/resume <id>` in daemon mode to switch
        /// sessions at runtime.
        #[arg(long)]
        attach: Option<String>,
    },
    /// Run as a daemon (WebSocket server). Frontends connect to run sessions.
    Daemon {
        /// Unix socket path (default: $XDG_RUNTIME_DIR/mew.sock or /tmp/mew.sock)
        #[arg(long)]
        socket: Option<String>,

        /// TCP address to listen on, e.g. 127.0.0.1:9847. Browser-based
        /// frontends connect to this. Defaults to off — pass explicitly
        /// to enable. May be combined with --socket to listen on both.
        #[arg(long)]
        port: Option<String>,

        /// Detach from the terminal and run in the background. The daemon
        /// survives logout. Writes its PID to the pidfile (default:
        /// $XDG_RUNTIME_DIR/mew.pid). Combine with --log to redirect
        /// output to a file instead of /dev/null.
        #[arg(long)]
        background: bool,

        /// Redirect logs to this file (implies --background behavior for
        /// stdio redirection). Defaults to /dev/null when --background
        /// is set without --log.
        #[arg(long)]
        log: Option<String>,

        /// Path to write the daemon PID. Defaults to
        /// $XDG_RUNTIME_DIR/mew.pid or /tmp/mew.pid.
        #[arg(long)]
        pidfile: Option<String>,

        /// Stop a running background daemon. Reads the PID from the
        /// pidfile and sends SIGTERM. Exits 0 on success.
        #[arg(long)]
        stop: bool,

        /// Use the bundled `FakeProvider` instead of a real model.
        /// Responds to any prompt with a fixed streaming text. Intended
        /// for tests, demos, and offline experimentation — do not use
        /// in production. Overrides `--provider` and `--model`.
        #[arg(long)]
        fake_provider: bool,

        /// Provider ID (defaults to last-used or opencode-zen)
        #[arg(long)]
        provider: Option<String>,

        /// Model ID
        #[arg(long)]
        model: Option<String>,

        /// Dump raw request/response
        #[arg(long)]
        raw: bool,

        /// Auto-allow Mutating tools; bash still prompts.
        #[arg(long, short = 'P', env = "MEW_PERMISSIVE")]
        permissive: bool,

        /// Route every tool call through a small LLM classifier.
        #[arg(long, short = 'A', env = "MEW_AUTO")]
        auto: bool,

        /// Like `--auto`, but fail-closed on classifier uncertainty.
        #[arg(long, env = "MEW_AUTO_PLUS")]
        auto_plus: bool,

        /// Skip all permission prompts. Every tool auto-runs.
        #[arg(long, short = 'D', env = "MEW_DANGEROUS")]
        dangerously_skip_permissions: bool,
    },
    /// View or edit configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Debug tools: permission simulator, VFS inspector.
    Debug {
        #[command(subcommand)]
        command: DebugCommands,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Option<String>,
    },
    /// Manage TUI themes
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
}

#[derive(Subcommand)]
enum ThemeCommands {
    /// List available themes
    List,
    /// Print the currently active theme name
    Current,
    /// Install a theme JSON file to ~/.config/mew/themes/
    Install {
        /// Path to the JSON theme file to install
        path: std::path::PathBuf,
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

#[derive(Subcommand)]
enum DebugCommands {
    /// Simulate a permission check for a tool call. Shows the decision the
    /// engine would make without running the agent.
    Permissions {
        /// Tool name (e.g. "bash", "read", "write").
        tool: String,
        /// Tool input as JSON (e.g. '{"command": "rm -rf /"}').
        /// Defaults to empty object `{}`.
        input: Option<String>,
        /// Sensitivity tier: readonly, mutating, or dangerous.
        /// Defaults to "dangerous" (the strictest — worst-case check).
        #[arg(long, default_value = "dangerous")]
        sensitivity: String,
    },
    /// Inspect built-in resources via the mew:// virtual filesystem.
    Vfs {
        #[command(subcommand)]
        command: VfsCommands,
    },
    /// Inspect or clear the on-disk model catalog cache.
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
}

#[derive(Subcommand)]
enum CacheCommands {
    /// Print the directory that holds cached catalog files.
    Path,
    /// Remove the cached catalog files (main + umans). The next launch will
    /// re-fetch from the network. Use this when a provider added or removed
    /// models and the picker hasn't picked it up yet.
    Clear,
}

#[derive(Subcommand)]
enum VfsCommands {
    /// List resources at a path (or top-level if no path given).
    Ls {
        /// Path relative to the VFS root (e.g. "personas", "subagents").
        /// Omit to list top-level directories.
        path: Option<String>,
    },
    /// Print a resource's contents.
    Cat {
        /// Path relative to the VFS root (e.g. "personas/builder").
        path: String,
    },
}

fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    // Parse CLI args in a sync context — before any tokio runtime starts.
    // This lets us handle --stop and --background before the runtime's FDs
    // exist, so daemonize()'s dup2 calls can't clobber tokio internals.
    let cli = Cli::parse();

    // Handle --stop before anything else: read PID, send SIGTERM, exit.
    if let Some(Commands::Daemon {
        stop: true,
        pidfile,
        ..
    }) = &cli.command
    {
        let pidfile = pidfile.clone().unwrap_or_else(default_pidfile);
        return stop_daemon(&pidfile);
    }

    // Handle --background: double-fork + setsid before the runtime starts.
    // The parent exits immediately; the child continues into the runtime.
    // Returns true if we daemonized (and already inited tracing).
    let daemonized = if let Some(Commands::Daemon {
        background: true,
        log,
        pidfile,
        ..
    }) = &cli.command
    {
        let pidfile = pidfile.clone().unwrap_or_else(default_pidfile);
        daemonize(log.as_deref(), &pidfile)?;
        true
    } else {
        false
    };

    // Now safe to start the tokio runtime.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async_main(cli, daemonized))
}

async fn async_main(cli: Cli, daemonized: bool) -> Result<()> {
    // Only init tracing here if daemonize() didn't already do it.
    if !daemonized {
        // In TUI mode (Run/Chat without --connect), redirect tracing to
        // a log file so log output doesn't corrupt the terminal display.
        // Daemon and non-TUI commands keep the default stderr writer.
        let is_tui = matches!(
            cli.command,
            Some(Commands::Run { .. }) | Some(Commands::Chat { connect: None, .. }) | None
        );
        if is_tui {
            let log_path = std::env::temp_dir().join(format!("mew-{}.log", std::process::id()));
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .map(std::io::BufWriter::new)
                .map_err(|e| {
                    anyhow::anyhow!("failed to open log file {}: {}", log_path.display(), e)
                })?;
            eprintln!("logging to {}", log_path.display());
            tracing_subscriber::fmt()
                .with_writer(std::sync::Mutex::new(file))
                .init();
        } else {
            tracing_subscriber::fmt().init();
        }
    }

    // Load runtime state for fallback defaults.
    let state = mew_config::load_state().unwrap_or_default();

    match cli.command {
        Some(Commands::Run {
            provider,
            model,
            variant,
            raw,
            permissive,
            auto,
            auto_plus,
            dangerously_skip_permissions,
            prompt,
        }) => {
            let provider = resolve_provider(provider, &state);
            let model = resolve_model_opt(model, &state);
            run_cmd(
                provider,
                model,
                variant,
                raw,
                resolve_mode(permissive, auto, auto_plus, dangerously_skip_permissions),
                prompt,
            )
            .await
        }
        Some(Commands::Chat {
            provider,
            model,
            variant,
            raw,
            permissive,
            auto,
            auto_plus,
            dangerously_skip_permissions,
            connect,
            attach,
        }) => {
            let mode = resolve_mode(permissive, auto, auto_plus, dangerously_skip_permissions);
            if let Some(connect_url) = connect {
                chat_with_daemon(&connect_url, attach.as_deref()).await
            } else {
                let provider = resolve_provider(provider, &state);
                let model = resolve_model_opt(model, &state);
                chat_cmd(provider, model, variant, raw, mode).await
            }
        }
        Some(Commands::Daemon {
            socket,
            port,
            background: _,
            log: _,
            pidfile: _,
            stop: _,
            fake_provider,
            provider,
            model,
            raw,
            permissive,
            auto,
            auto_plus,
            dangerously_skip_permissions,
        }) => {
            // --background and --stop are handled before the tokio runtime
            // starts (in main()). By the time we reach here, we're already
            // in the background daemon process (or running foreground).
            let provider = resolve_provider(provider, &state);
            let model = resolve_model_opt(model, &state);
            let mode = resolve_mode(permissive, auto, auto_plus, dangerously_skip_permissions);
            run_daemon(socket, port, fake_provider, &provider, model, raw, mode).await
        }
        None => {
            let provider = resolve_provider(None, &state);
            let model = resolve_model_opt(None, &state);
            chat_cmd(
                provider,
                model,
                None,
                false,
                mew_hooks::PermissionMode::Standard,
            )
            .await
        }
        Some(Commands::Config { command }) => {
            config_cmd(command)?;
            Ok(())
        }
        Some(Commands::Debug { command }) => debug_cmd(command).await,
        Some(Commands::Completions { shell }) => {
            let shell = shell.as_deref().unwrap_or("");
            let shell = match shell.to_lowercase().as_str() {
                "bash" => clap_complete::Shell::Bash,
                "zsh" => clap_complete::Shell::Zsh,
                "fish" => clap_complete::Shell::Fish,
                "elvish" => clap_complete::Shell::Elvish,
                "powershell" | "pwsh" => clap_complete::Shell::PowerShell,
                "" => {
                    // Detect from environment
                    if let Some(shell_var) = std::env::var_os("SHELL") {
                        let shell_str = shell_var.to_string_lossy();
                        if shell_str.contains("zsh") {
                            clap_complete::Shell::Zsh
                        } else if shell_str.contains("fish") {
                            clap_complete::Shell::Fish
                        } else if shell_str.contains("elvish") {
                            clap_complete::Shell::Elvish
                        } else {
                            clap_complete::Shell::Bash
                        }
                    } else {
                        clap_complete::Shell::Bash
                    }
                }
                other => {
                    eprintln!("unknown shell: {other}");
                    eprintln!("supported: bash, zsh, fish, elvish, powershell");
                    std::process::exit(1);
                }
            };
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "mew", &mut std::io::stdout());
            Ok(())
        }
        Some(Commands::Theme { command }) => theme_cmd(command),
    }
}

/// Handle `mew theme` subcommands.
fn theme_cmd(command: ThemeCommands) -> Result<()> {
    match command {
        ThemeCommands::List => {
            let names = mew_tui::theme::Theme::list_available();
            let state = mew_config::load_state().unwrap_or_default();
            let cfg = mew_config::load().unwrap_or_default();
            let active = if !state.theme.is_empty() {
                &state.theme
            } else {
                &cfg.tui.theme
            };
            let active = if active.is_empty() { "dark" } else { active };
            for name in &names {
                if name == active {
                    println!("  * {name} (active)");
                } else {
                    println!("    {name}");
                }
            }
            Ok(())
        }
        ThemeCommands::Current => {
            let state = mew_config::load_state().unwrap_or_default();
            let cfg = mew_config::load().unwrap_or_default();
            let active = if !state.theme.is_empty() {
                &state.theme
            } else {
                &cfg.tui.theme
            };
            let active = if active.is_empty() { "dark" } else { active };
            println!("{active}");
            Ok(())
        }
        ThemeCommands::Install { path } => {
            // Validate the file parses as a theme.
            let theme =
                mew_tui::theme::Theme::from_json(&path).context("failed to parse theme file")?;
            // Determine the install directory.
            let themes_dir = mew_tui::theme::Theme::themes_dir()
                .context("could not determine themes directory")?;
            std::fs::create_dir_all(&themes_dir).context("failed to create themes directory")?;
            let dest = themes_dir.join(format!("{}.json", theme.name));
            std::fs::copy(&path, &dest)
                .with_context(|| format!("failed to copy to {}", dest.display()))?;
            println!("installed theme '{}' to {}", theme.name, dest.display());
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

    // Umans publishes its own model configs at /v1/models/info — fetch and
    // merge them in only when the umans provider is both configured and has
    // a credential set. Without a credential, every model would be a dead
    // entry in the picker, so we hide the whole provider until a key shows up.
    if provider_has_credential(cfg, "umans") {
        match mew_catalog::load_umans().await {
            Ok(umans_models) => {
                tracing::info!("loaded {} umans model configs", umans_models.len());
                cat.merge_local(umans_models);
            }
            Err(e) => {
                tracing::warn!(?e, "umans models fetch failed; continuing without");
            }
        }
    } else if cfg.providers.contains_key("umans") {
        tracing::debug!("umans provider configured but no credential set; skipping model fetch");
    }

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

/// Handle `mew debug` subcommands.
async fn debug_cmd(command: DebugCommands) -> Result<()> {
    match command {
        DebugCommands::Permissions {
            tool,
            input,
            sensitivity,
        } => {
            let cfg = mew_config::load().context("load config")?;
            let engine = build_permission_engine(&cfg, mew_hooks::PermissionMode::Standard);

            let input_json: serde_json::Value = match input {
                Some(s) => serde_json::from_str(&s).context("failed to parse input JSON")?,
                None => serde_json::json!({}),
            };

            let sens = match sensitivity.as_str() {
                "readonly" | "ReadOnly" => mew_tools::Sensitivity::ReadOnly,
                "mutating" | "Mutating" => mew_tools::Sensitivity::Mutating,
                "dangerous" | "Dangerous" => mew_tools::Sensitivity::Dangerous,
                other => anyhow::bail!(
                    "unknown sensitivity '{other}'; expected readonly|mutating|dangerous"
                ),
            };

            let decision = engine
                .check(
                    &tool,
                    &input_json,
                    sens,
                    &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                )
                .await;

            println!("Tool:        {tool}");
            println!("Input:       {input_json}");
            println!("Sensitivity: {sens:?}");
            println!();
            println!("Decision:    {decision:?}");
            Ok(())
        }
        DebugCommands::Vfs { command } => match command {
            VfsCommands::Ls { path } => {
                match path {
                    None => {
                        let entries = mew_prompts::vfs::top_level();
                        for e in entries {
                            println!("{e}/");
                        }
                    }
                    Some(p) => {
                        let entries = mew_prompts::vfs::list_dir(&p);
                        if entries.is_empty() {
                            println!("(empty or not found: {p})");
                        }
                        for e in entries {
                            println!("{e}");
                        }
                    }
                }
                Ok(())
            }
            VfsCommands::Cat { path } => match mew_prompts::vfs::read_builtin(&path) {
                Some(contents) => {
                    print!("{contents}");
                    Ok(())
                }
                None => {
                    println!("not found: {path}");
                    Ok(())
                }
            },
        },
        DebugCommands::Cache { command } => match command {
            CacheCommands::Path => {
                println!("{}", mew_catalog::cache_dir().display());
                Ok(())
            }
            CacheCommands::Clear => {
                let removed = mew_catalog::clear_cache();
                if removed.is_empty() {
                    println!("no catalog cache files to remove");
                } else {
                    println!("removed {} file(s):", removed.len());
                    for p in &removed {
                        println!("  {}", p.display());
                    }
                    println!("next launch will re-fetch the catalog from the network");
                }
                Ok(())
            }
        },
    }
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
                    # Docs: https://github.com/mewcomputer/mew\n\n\
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

fn hashline_enabled_for(cfg: &Config, provider_id: &str) -> bool {
    !cfg.providers
        .get(provider_id)
        .map(|p| p.disable_hashline)
        .unwrap_or(false)
}

fn build_tools(
    skills: Arc<Vec<mew_skills::Skill>>,
    skill_filter: Arc<tokio::sync::RwLock<Option<std::collections::HashSet<String>>>>,
    template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>>,
    personas: Arc<Vec<mew_personas::Persona>>,
    pending_persona_switch: Arc<tokio::sync::Mutex<Option<String>>>,
    current_persona_name: Arc<tokio::sync::RwLock<Option<String>>>,
    hashline_enabled: bool,
) -> Vec<Arc<dyn mew_tools::Tool>> {
    let mut tools: Vec<Arc<dyn mew_tools::Tool>> = vec![
        Arc::new(Read),
        Arc::new(Write),
        Arc::new(EditStrReplace),
        Arc::new(Bash),
        Arc::new(Glob),
        Arc::new(Grep),
        Arc::new(Echo),
        Arc::new(ExitTool),
        Arc::new(ProgressUpdate),
        Arc::new(AskUser),
        Arc::new(ShellBackground),
        Arc::new(ShellMonitor),
        Arc::new(JobStatus),
        Arc::new(JobBlock),
        Arc::new(JobCancel),
        Arc::new(TodoCreate),
        Arc::new(TodoUpdate),
        Arc::new(TodoComplete),
        Arc::new(TodoDelete),
        Arc::new(TodoListTool),
        Arc::new(WebFetch),
    ];
    if hashline_enabled {
        tools.insert(3, Arc::new(EditHashline));
    }
    if !skills.is_empty() {
        tools.push(Arc::new(Skill::new(skills, skill_filter, template_ctx)));
    }
    // The switch_persona tool is only useful when there's at least one
    // persona to switch to. With zero discovered personas the tool would
    // be a permanent dead-end for the model.
    if !personas.is_empty() {
        tools.push(Arc::new(SwitchPersonaTool::new(
            personas,
            pending_persona_switch,
            current_persona_name,
        )));
    }
    tools
}

/// Session-level info exposed to plugins via the `config_read` callback.
/// Plugins can query `session_id`, `model`, `provider`, `workspace`, and
/// `active_persona`. Updated when persona/model changes.
struct PluginInfo {
    session_id: String,
    model: String,
    provider: String,
    workspace: String,
    active_persona: Option<String>,
}

/// Build a display-only summary of a persona for the confirm modal.
fn persona_summary(p: &mew_personas::Persona) -> mew_tui::app::PersonaSummary {
    mew_tui::app::PersonaSummary {
        name: p.name.clone(),
        description: p.description.clone(),
        model: p.config.model.clone(),
        tools: p.config.tools.clone(),
        tools_deny: p.config.tools_deny.clone(),
        skills: p.config.skills.clone(),
        color: p.config.color.clone(),
    }
}

/// Apply a persona switch: set the agent's persona state, swap the
/// provider/model if the persona pins one, and push a synthetic message
/// describing what changed. Factored out of the slash-command handler so
/// the confirm modal can reuse it after the user accepts the diff.
fn apply_persona_switch(
    agent: &mut mew_agent::Agent,
    app: &mut mew_tui::App,
    cfg: &Config,
    cat: Option<&mew_catalog::Catalog>,
    provider_id: &str,
    raw: bool,
    persona: &mew_personas::Persona,
) {
    let pinned_model = agent.apply_persona(persona);
    app.active_persona = Some(persona.name.clone());
    app.active_persona_color = persona.config.color.clone();
    if let Some(ref model_str) = pinned_model {
        let (new_provider_id, new_model_id) = if let Some(idx) = model_str.find('/') {
            (&model_str[..idx], &model_str[idx + 1..])
        } else {
            (provider_id, model_str.as_str())
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
            }
            Err(e) => {
                tracing::warn!("persona model pin failed: {}", e);
            }
        }
    }
    app.messages.push(synthetic_message(format!(
        "switched to persona: {}{}",
        persona.name,
        if let Some(ref m) = pinned_model {
            format!(" (model: {})", m)
        } else {
            String::new()
        }
    )));
}

/// If the agent's `switch_persona` tool queued a switch and the turn has
/// ended, the TUI receives `AgentEvent::PersonaSwitchRequested` and
/// stashes the name in `app.pending_persona_switch_apply`. This helper
/// drains that field and applies the switch using the same path as the
/// slash-command confirm modal. Called from the main event loop after
/// every agent event.
async fn drain_pending_persona_switch(
    agent: &mut mew_agent::Agent,
    app: &mut mew_tui::App,
    personas: &[mew_personas::Persona],
    cfg: &Config,
    cat: Option<&mew_catalog::Catalog>,
    provider_id: &str,
    raw: bool,
) {
    let Some(name) = app.pending_persona_switch_apply.take() else {
        return;
    };
    if let Some(persona) = personas.iter().find(|p| p.name == name) {
        // Check if the *current* persona's transition rules require
        // confirmation. If so, open the confirm modal instead of
        // applying the switch directly. The user confirms via the
        // PersonaSwitchConfirmed action, which calls apply_persona_switch.
        let needs_confirm = app
            .active_persona
            .as_ref()
            .and_then(|cur| personas.iter().find(|p| &p.name == cur))
            .and_then(|p| p.config.transitions.as_ref())
            .is_some_and(|t| t.confirm);

        if needs_confirm {
            let target = persona_summary(persona);
            let current = app
                .active_persona
                .as_ref()
                .and_then(|cur_name| personas.iter().find(|p| &p.name == cur_name))
                .map(persona_summary);
            app.request_persona_switch_confirm(target, current);
            return;
        }

        let old = app.active_persona.clone();
        apply_persona_switch(agent, app, cfg, cat, provider_id, raw, persona);
        agent
            .dispatcher
            .on_persona_change(old.as_deref(), &name)
            .await;
    } else {
        tracing::warn!(
            name = %name,
            "PersonaSwitchRequested for unknown persona; ignoring"
        );
    }
}

fn build_permission_engine(
    cfg: &Config,
    mode: mew_hooks::PermissionMode,
) -> Arc<mew_config::permissions::PermissionEngine> {
    let secret_globs: Vec<String> = cfg
        .secrets
        .files
        .iter()
        .flat_map(|f| f.paths.iter().cloned())
        .collect();
    // Default cwd for the escape tier: the process's current directory.
    // The 4 call sites all hand the same `cfg` to this helper, so the
    // escape tier gets the same default cwd everywhere.
    let default_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    Arc::new(
        mew_config::permissions::PermissionEngine::new(cfg.permissions.rules.clone())
            .with_secret_files(secret_globs)
            .with_mode(mode)
            .with_workspace_roots(cfg.workspace.roots.clone(), default_cwd),
    )
}

/// Return the first provider configured as a router.
///
/// Prefers a provider literally named `router`, otherwise returns the first
/// provider whose `kind` is `"router"`. Router providers are task-only and
/// cannot be selected as the main chat provider.
fn find_router_provider(cfg: &Config) -> Option<(String, &ProviderConfig)> {
    if let Some(pc) = cfg.providers.get("router") {
        if pc.kind == "router" {
            return Some(("router".to_string(), pc));
        }
    }
    cfg.providers
        .iter()
        .find(|(_, pc)| pc.kind == "router")
        .map(|(id, pc)| (id.clone(), pc))
}

/// Wire the Auto/Auto+ classifier provider into the agent.
///
/// If a router provider is configured, the classifier automatically uses the
/// router's `micro` tier. Otherwise, falls back to the explicit
/// `permissions.classifier_provider/classifier_model` config.
fn maybe_set_classifier_provider(
    agent: &mut mew_agent::Agent,
    cfg: &Config,
    cat: Option<&Catalog>,
    raw: bool,
    _active_provider_id: &str,
    _active_model_id: &str,
) {
    // If a router provider is configured, use its micro tier for classification.
    if let Some((router_id, pc)) = find_router_provider(cfg) {
        let micro_model = pc.micro_model().to_string();
        if !micro_model.is_empty() {
            let (micro_pid, micro_mid) = resolve_model(cfg, cat, &router_id, Some(micro_model));
            match build_provider(cfg, cat, &micro_pid, &micro_mid, raw) {
                Ok(provider) => {
                    agent.set_classifier_provider(provider, Some(micro_mid.clone()));
                    tracing::info!(
                        provider = %micro_pid,
                        model = %micro_mid,
                        "router micro tier configured as classifier for Auto/Auto+ modes"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to build router micro tier as classifier; Auto/Auto+ will fall through to user"
                    );
                }
            }
            return;
        }
    }

    // Legacy explicit classifier config.
    if let Some(ref provider_id) = cfg.permissions.classifier_provider {
        let model_id = cfg.permissions.classifier_model.as_deref().unwrap_or("");
        match build_provider(cfg, cat, provider_id, model_id, raw) {
            Ok(provider) => {
                agent.set_classifier_provider(provider, cfg.permissions.classifier_model.clone());
                tracing::info!(
                    provider = %provider_id,
                    model = ?cfg.permissions.classifier_model,
                    "classifier provider configured for Auto/Auto+ modes"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to build classifier provider; Auto/Auto+ will fall through to user"
                );
            }
        }
    }
}

/// Build the `SecretSet` shared with every tool call: words to redact from
/// output, and file globs whose results get dropped from search tools.
fn build_secret_set(cfg: &Config) -> Arc<SecretSet> {
    Arc::new(SecretSet {
        words: cfg
            .secrets
            .words
            .iter()
            .flat_map(|w| w.values.iter().cloned())
            .collect(),
        globs: cfg
            .secrets
            .files
            .iter()
            .flat_map(|f| f.paths.iter().cloned())
            .collect(),
    })
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
    plugin_configs: std::collections::HashMap<String, mew_hooks::PluginHookConfig>,
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

    let dirs = mew_hooks_runtime::PluginLoader::default_dirs();
    match SubprocessDispatcher::from_dirs_filtered_with_config(
        dirs,
        host.clone(),
        disabled_plugins,
        plugin_configs,
        SubprocessDispatcher::default_timeout(),
    )
    .await
    {
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

// ACP client and server were removed as part of the daemon architecture
// migration. See DAEMON_PLAN.md — `mew daemon` + `mew chat --connect` is
// the single transport now. The TUI daemon-client mode covers the
// previous "drive an external agent" use case by spawning a second mew
// daemon for that agent's model and connecting locally.

/// Render any context files marked with `template: true` through minijinja
/// using the agent's template context. Non-templated files are left as-is.
/// Returns a new Vec with rendered content.
fn render_templated_context_files(
    files: &[mew_context::File],
    agent: &mew_agent::Agent,
) -> Vec<mew_context::File> {
    let has_templated = files.iter().any(|f| f.template);
    if !has_templated {
        return files.to_vec();
    }

    // Build a template context from the agent's current state.
    let tool_names: Vec<String> = agent.tools.keys().cloned().collect();
    let ctx = mew_prompts::template::TemplateContext {
        model_id: agent.model_id.clone(),
        provider_id: agent.provider_id.clone(),
        session_id: agent.session_id.to_string(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        current_date: mew_prompts::template::TemplateContext::today(),
        tools: tool_names,
        skills: agent.skills.iter().map(|s| s.name.clone()).collect(),
        project_vars: agent.project_vars.clone(),
        ..Default::default()
    };

    files
        .iter()
        .map(|f| {
            if f.template {
                mew_context::File {
                    path: f.path.clone(),
                    content: mew_prompts::template::render(&f.content, &ctx),
                    template: false,
                }
            } else {
                f.clone()
            }
        })
        .collect()
}

/// Build a full agent for a session. Used by `run_daemon` (and the TUI's
/// `--connect` daemon-client mode goes through the daemon side). Sets up
/// the provider, tools, MCP, personas, skills, subagents, context files,
/// and pricing.
///
/// `writer` / `session_id` come from the daemon's `SessionManager`, which
/// owns the session directory. The agent is wired to append to that writer.
#[allow(clippy::too_many_arguments)]
fn build_session_agent(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    model_id: &str,
    raw: bool,
    mode: mew_hooks::PermissionMode,
    writer: Option<mew_session::Writer>,
    session_id: Option<mew_message::SessionId>,
) -> Result<Agent> {
    let provider =
        build_provider(cfg, cat, provider_id, model_id, raw).context("build provider")?;

    let dispatcher = Arc::new(NopDispatcher);
    let cwd = std::env::current_dir().unwrap_or_default();
    let skills_loader = mew_skills::Loader::new(cwd.clone());
    let skills = Arc::new(skills_loader.load().unwrap_or_default());
    let skill_filter = Arc::new(tokio::sync::RwLock::new(None));
    let persona_loader = mew_personas::Loader::new(cwd.clone());
    let personas_arc = Arc::new(persona_loader.load().unwrap_or_default());
    let pending_persona_switch = Arc::new(tokio::sync::Mutex::new(None));
    let current_persona_name = Arc::new(tokio::sync::RwLock::new(None));
    let template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>> =
        Arc::new(tokio::sync::RwLock::new(None));
    let tools = build_tools(
        skills.clone(),
        skill_filter.clone(),
        template_ctx.clone(),
        personas_arc.clone(),
        pending_persona_switch.clone(),
        current_persona_name.clone(),
        hashline_enabled_for(cfg, provider_id),
    );

    let flagged_files: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let mut tools = tools;
    tools.push(Arc::new(FlagImportant::new(flagged_files.clone())));

    let permission_engine = build_permission_engine(cfg, mode);

    let mut agent = Agent::new(provider, dispatcher.clone(), writer, tools, session_id);
    agent.set_model_info(model_id, provider_id);
    agent.template_ctx = template_ctx;
    agent.flagged_files = flagged_files;
    agent.secrets = build_secret_set(cfg);
    agent.set_permission_engine(permission_engine);
    maybe_set_classifier_provider(&mut agent, cfg, cat, raw, provider_id, model_id);
    agent.set_plan_path(&cfg.plan_path);
    agent.set_personas((*personas_arc).clone());
    agent.set_pending_persona_switch(pending_persona_switch.clone());
    agent.set_current_persona_name(current_persona_name.clone());

    // Enable a persistent shell session so `cd`, `export`, and other
    // state survive across bash tool calls.
    let shell_session = mew_tools::tools::shell_session::shared_session(
        std::env::current_dir().unwrap_or_default(),
    );
    agent.set_shell_session(shell_session);

    // Wire the fallback-model provider builder. When the primary provider
    // returns a stream error and the active persona has `fallback_models`,
    // the turn loop calls this closure to build a new provider for each
    // fallback `provider/model` string.
    let cfg_clone = cfg.clone();
    let cat_clone = cat.cloned();
    agent.set_provider_builder(Box::new(move |model_str: &str| {
        let (pid, mid) = if let Some(idx) = model_str.find('/') {
            (&model_str[..idx], &model_str[idx + 1..])
        } else {
            (model_str, "")
        };
        build_provider(&cfg_clone, cat_clone.as_ref(), pid, mid, raw).map_err(|e| e.to_string())
    }));
    // Plugin tools: register_plugin_tools is async but we're in a sync
    // builder. The daemon's agent builder closure must be sync. Plugin tool
    // registration is a no-op for NopDispatcher (the default), so skipping
    // the call is safe. When a real dispatcher is wired, this will need to
    // become async — at which point the AgentBuilder type should change.
    // agent.register_plugin_tools().await;
    // Apply the default persona on startup (non-interactive path — no TUI app).
    if cfg.default_persona != "none" && cfg.default_persona != "default" {
        if let Some(persona) = personas_arc.iter().find(|p| p.name == cfg.default_persona) {
            agent.apply_persona(persona);
            tracing::info!(persona = %persona.name, "applied default persona on startup");
        }
    }
    if cfg.workspace.roots.is_empty() {
        agent.workspace_roots = vec![cwd.clone()];
    } else {
        agent.workspace_roots = cfg.workspace.roots.clone();
    }

    // Wire up subagent infrastructure.
    let subagent_defs = {
        let loader = mew_subagents::Loader::new(cwd.clone());
        Arc::new(loader.load().unwrap_or_default())
    };
    if !subagent_defs.is_empty() {
        let resolver = Arc::new(MainModelResolver {
            cfg: Arc::new(cfg.clone()),
            cat: cat.cloned().map(Arc::new),
            default_provider_id: provider_id.to_string(),
            router_provider_id: find_router_provider(cfg).map(|(id, _)| id),
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
    let ctx_loader = mew_context::Loader::new(&cwd);
    let ctx_files = ctx_loader.load().unwrap_or_default();
    let project_vars = mew_context::load_project_vars(&cwd);
    agent.project_vars = project_vars;
    if !ctx_files.is_empty() {
        let rendered_ctx = render_templated_context_files(&ctx_files, &agent);
        agent.set_system(mew_context::build_system_prompt(&rendered_ctx));
    }
    if !skills.is_empty() {
        agent.set_skills((*skills).clone());
    }

    if let Some(c) = cat {
        agent.supports_vision = c.supports_vision(model_id);
        agent.context_window = c.context_window(model_id).max(0) as u32;
        // Default `max_output_tokens` from the catalog, capped at 32K so
        // models with very large total context (e.g. GPT-5-Codex at 400K
        // with 128K max output) leave more room for input. 0 means
        // "unknown" — the agent keeps its existing default of 0 (no
        // override) so the provider's own default applies.
        if let Some(raw_max_output) = c.max_output(model_id) {
            agent.default_max_output_tokens = raw_max_output.min(32_768);
        }
        if let Some(m) = c.lookup(model_id) {
            agent.input_price = m.pricing.input;
            agent.output_price = m.pricing.output;
            agent.cache_read_price = m.pricing.cache_read;
            agent.cache_write_price = m.pricing.cache_write;
            agent.reasoning_price = m.pricing.reasoning;
        }
    }

    Ok(agent)
}

/// Run the daemon. Builds an agent per connection via `build_session_agent`.
/// Listens on the Unix socket (if `--socket` is set or by default) AND/OR
/// the TCP address (if `--port` is set). With neither flag, listens on the
/// default Unix socket.
///
/// If `fake_provider` is true, all real-provider setup is bypassed and
/// every connection gets a `FakeProvider`-backed agent. Used for tests
/// and offline demos.
async fn run_daemon(
    socket: Option<String>,
    port: Option<String>,
    fake_provider: bool,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<()> {
    let cfg = mew_config::load().context("load config")?;
    let cat = load_catalog(&cfg).await;

    // Default to Unix socket at $XDG_RUNTIME_DIR/mew.sock or /tmp/mew.sock,
    // unless `--port` was given and `--socket` was not (in which case the
    // daemon is TCP-only).
    let socket_path = socket.clone().unwrap_or_else(|| {
        std::env::var("XDG_RUNTIME_DIR")
            .map(|d| format!("{d}/mew.sock"))
            .unwrap_or_else(|_| "/tmp/mew.sock".to_string())
    });
    let use_unix = socket.is_some() || port.is_none();

    let cfg = Arc::new(cfg);
    let cat = Arc::new(cat);

    // Clone Arcs for the model switcher/lister before the builder closure
    // moves them. These are only used when !fake_provider.
    let cfg_for_models = Arc::clone(&cfg);
    let cat_for_models = Arc::clone(&cat);

    // The agent-builder closure. `fake_provider=true` skips all real-
    // provider setup and wires in `FakeProvider` so the daemon runs
    // without network access.
    let builder: mew_daemon::AgentBuilder = if fake_provider {
        Arc::new(|params: mew_daemon::AgentBuildParams| {
            use mew_provider_fake::FakeProvider;
            let provider = Arc::new(FakeProvider::new(FakeProvider::text_response(
                "hello from fake provider",
            )));
            let dispatcher = Arc::new(mew_hooks::NopDispatcher);
            let session_id: Option<SessionId> = params
                .session_id
                .strip_prefix("sess_")
                .and_then(|s| ulid::Ulid::from_string(s).ok());
            Ok((
                {
                    let mut a = Agent::new(
                        provider,
                        dispatcher,
                        Some(params.writer),
                        Vec::new(),
                        session_id,
                    );
                    a.set_model_info("fake", "fake");
                    a
                },
                Some("fake".to_string()),
                Some("fake".to_string()),
            ))
        })
    } else {
        let (provider_id, model_id) =
            resolve_model(&cfg, (*cat).as_ref(), provider_flag, model_flag);
        let provider_id = Arc::new(provider_id);
        let model_id = Arc::new(model_id);
        let model_id_display = model_id.clone();
        let provider_id_display = Arc::clone(&provider_id);
        info!(
            "mew daemon starting, model={}, unix={}, tcp={:?}",
            model_id_display, use_unix, port
        );
        Arc::new(move |params: mew_daemon::AgentBuildParams| {
            let cfg = (*cfg).clone();
            let cat = (*cat).clone();
            let provider_id = (*provider_id).clone();
            let model_id = (*model_id).clone();
            let session_id: Option<SessionId> = params
                .session_id
                .strip_prefix("sess_")
                .and_then(|s| ulid::Ulid::from_string(s).ok());
            let agent = build_session_agent(
                &cfg,
                cat.as_ref(),
                &provider_id,
                &model_id,
                raw,
                mode,
                Some(params.writer),
                session_id,
            )?;
            Ok((
                agent,
                Some((*model_id_display).clone()),
                Some((*provider_id_display).clone()),
            ))
        })
    };

    // Two listeners can run concurrently via `tokio::try_join!`. The builder
    // closure is wrapped in an Arc (so the daemon API takes Arc<dyn Fn>),
    // letting both servers share the same one.
    let mut server = mew_daemon::DaemonServer::new(builder);

    // Enable model switching for non-fake providers.
    if !fake_provider {
        let raw2 = raw;

        // Clone Arcs for each closure before either captures them.
        let cfg_lister = Arc::clone(&cfg_for_models);
        let cat_lister = Arc::clone(&cat_for_models);
        let cfg_switcher = Arc::clone(&cfg_for_models);
        let cat_switcher = Arc::clone(&cat_for_models);

        // Model lister: returns all catalog models that belong to
        // configured providers with credentials.
        let lister: mew_daemon::ModelLister = Arc::new(move || {
            let cfg = &*cfg_lister;
            let mut models = Vec::new();

            // Collect provider IDs that have credentials. Only show models
            // the user can actually call.
            let cred_pids: Vec<String> = cfg
                .providers
                .keys()
                .filter(|pid| provider_has_credential(cfg, pid))
                .cloned()
                .collect();

            if let Some(cat) = cat_lister.as_ref() {
                for m in cat.models.values() {
                    if cred_pids.contains(&m.provider) {
                        let thinking_variants = cat
                            .thinking_variants(&m.id)
                            .into_iter()
                            .map(|v| mew_protocol::ThinkingVariantInfo { name: v.name })
                            .collect();
                        models.push(mew_protocol::ModelInfo {
                            id: format!("{}/{}", m.provider, m.id),
                            provider: m.provider.clone(),
                            model: m.id.clone(),
                            description: Some(format!(
                                "{} ctx · {}",
                                m.context_window,
                                if m.reasoning { "reasoning" } else { "standard" }
                            )),
                            thinking_variants,
                        });
                    }
                }
            }

            models.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.model.cmp(&b.model)));
            models
        });

        // Model switcher: rebuilds the provider on the agent.
        let switcher: mew_daemon::ModelSwitcher = Arc::new(
            move |agent: &mut Agent, provider_id: &str, model_id: &str| {
                let cat_ref = (*cat_switcher).as_ref();
                let new_provider =
                    build_provider(&cfg_switcher, cat_ref, provider_id, model_id, raw2)
                        .context("build provider for model switch")?;
                agent.provider = new_provider;
                if let Some(c) = cat_ref {
                    agent.supports_vision = c.supports_vision(model_id);
                    agent.context_window = c.context_window(model_id).max(0) as u32;
                    if let Some(raw_max) = c.max_output(model_id) {
                        agent.set_default_max_output_tokens(raw_max.min(32768));
                    }
                }
                Ok((provider_id.to_string(), model_id.to_string()))
            },
        );

        server = server.with_model_management(switcher, lister);

        // Thinking variant setter: resolves a variant name via the catalog
        // and applies it to the agent's reasoning config.
        let cat_thinking = Arc::clone(&cat_for_models);
        let thinking_setter: mew_daemon::ThinkingSetter =
            Arc::new(move |agent: &mut Agent, model_id: &str, variant: &str| {
                let cat_ref = (*cat_thinking).as_ref();
                if variant.is_empty() || variant == "none" {
                    agent.set_reasoning(None);
                    return Ok(None);
                }
                let config = resolve_reasoning(cat_ref, model_id, Some(variant));
                let resolved = config.is_some().then(|| variant.to_string());
                agent.set_reasoning(config);
                Ok(resolved)
            });
        server = server.with_thinking_setter(thinking_setter);
    }

    match (use_unix, port.as_deref()) {
        (true, Some(addr)) => {
            let parsed: std::net::SocketAddr = addr
                .parse()
                .with_context(|| format!("invalid --port address: {addr}"))?;
            let server_unix = mew_daemon::DaemonServer::new(Arc::clone(&server.builder))
                .with_model_management(
                    Arc::clone(server.model_switcher.as_ref().unwrap()),
                    Arc::clone(server.model_lister.as_ref().unwrap()),
                )
                .with_thinking_setter(Arc::clone(server.thinking_setter.as_ref().unwrap()));
            tokio::try_join!(
                async move { server_unix.run(&socket_path).await },
                async move { server.run_tcp(parsed).await }
            )
            .map(|_| ())
        }
        (true, None) => server.run(&socket_path).await,
        (false, Some(addr)) => {
            let parsed: std::net::SocketAddr = addr
                .parse()
                .with_context(|| format!("invalid --port address: {addr}"))?;
            server.run_tcp(parsed).await
        }
        (false, None) => unreachable!("use_unix implies socket_path is meaningful"),
    }
}

// ---------------------------------------------------------------------------
// Daemonization
// ---------------------------------------------------------------------------

/// Default PID file path: `$XDG_RUNTIME_DIR/mew.pid` or `/tmp/mew.pid`.
fn default_pidfile() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| format!("{d}/mew.pid"))
        .unwrap_or_else(|_| "/tmp/mew.pid".to_string())
}

/// Detach from the controlling terminal via the standard double-fork +
/// setsid pattern. After this returns, the process is a session leader
/// with no controlling terminal, immune to SIGHUP from logout.
///
/// If `log_file` is given, stdio is redirected there; otherwise to
/// `/dev/null`. The PID is written to `pidfile`.
fn daemonize(log_file: Option<&str>, pidfile: &str) -> Result<()> {
    use nix::unistd::{dup2, fork, setsid, ForkResult};
    use std::os::fd::AsRawFd;

    // Redirect stdio BEFORE forking. This way all processes (parent,
    // intermediate, daemon) have consistent FDs.
    let devnull = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
        .context("open /dev/null")?;

    let logfile = match log_file {
        Some(path) => std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .context("open log file")?,
        None => devnull.try_clone().context("clone /dev/null for stderr")?,
    };

    let null_fd = devnull.as_raw_fd();
    let log_fd = logfile.as_raw_fd();

    dup2(null_fd, 0).context("dup2 stdin")?;
    dup2(log_fd, 1).context("dup2 stdout")?;
    dup2(log_fd, 2).context("dup2 stderr")?;

    // Close the originals (they're now duplicated into 0/1/2).
    drop(devnull);
    drop(logfile);

    // First fork: parent exits, child continues.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            let _ = std::fs::write(pidfile, child.as_raw().to_string());
            std::process::exit(0);
        }
        ForkResult::Child => {}
    }

    // Become session leader.
    setsid()?;

    // Second fork: the real daemon can never re-acquire a terminal.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            let _ = std::fs::write(pidfile, child.as_raw().to_string());
            std::process::exit(0);
        }
        ForkResult::Child => {}
    }

    // Re-init tracing so logs go to stderr (which is now the log file).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    Ok(())
}

/// Read a PID from the pidfile and send SIGTERM. Returns an error if the
/// file is missing or the process doesn't exist.
fn stop_daemon(pidfile: &str) -> Result<()> {
    let pid_str =
        std::fs::read_to_string(pidfile).with_context(|| format!("read pidfile {pidfile}"))?;
    let pid: i32 = pid_str
        .trim()
        .parse()
        .with_context(|| format!("parse PID from {pidfile}: {pid_str:?}"))?;

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .context("send SIGTERM")?;

    // Remove the pidfile.
    let _ = std::fs::remove_file(pidfile);

    println!("daemon (PID {pid}) stopped");
    Ok(())
}

/// Run the TUI connected to a mew daemon. The daemon owns the agent;
/// the TUI is a pure frontend that sends prompts and receives AgentEvents.
async fn chat_with_daemon(connect_url: &str, attach: Option<&str>) -> Result<()> {
    use std::sync::Arc;

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

    // Request the session list so the sidebar rail is populated immediately.
    client.list_sessions().await;

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
                    match action {
                        mew_tui::events::Action::Submit(text) => {
                            let (enriched, display, attachments) = process_mentions(
                                &text,
                                &std::env::current_dir().unwrap_or_default(),
                                &mut app.context_files,
                            )
                            .await;
                            app.messages.push(user_message(display, attachments));
                            app.streaming = true;
                            let client = client.clone();
                            let ev_loop = event_loop.clone();
                            tokio::spawn(async move {
                                let rx = client.prompt(enriched).await;
                                ev_loop.forward_agent_events(rx);
                            });
                        }
                        mew_tui::events::Action::Quit => should_break = true,
                        mew_tui::events::Action::SetThinkingVariant(_) => {
                            app.set_alert(
                                "thinking variant switching not available in daemon mode",
                            );
                        }
                        mew_tui::events::Action::Cancel => {
                            let client = client.clone();
                            tokio::spawn(async move {
                                client.cancel().await;
                            });
                        }
                        mew_tui::events::Action::SlashCommand(text) => {
                            // Forward slash commands that mutate agent state
                            // to the daemon. Built-in display commands
                            // (/help, /cost) are handled locally by the TUI.
                            let (cmd, _arg) = match text.split_once(' ') {
                                Some((c, a)) => (c, Some(a)),
                                None => (text.as_str(), None),
                            };
                            match cmd {
                                "/clear" | "/compact" => {
                                    let client = client.clone();
                                    tokio::spawn(async move {
                                        client.slash_command(text.clone()).await;
                                    });
                                }
                                "/web" => {
                                    // Print the web URL for the current session.
                                    let sid = &app.status.session_id;
                                    if !sid.is_empty() {
                                        let port = std::env::var("MEW_PORT")
                                            .unwrap_or_else(|_| "9847".into());
                                        let url = format!("http://localhost:{port}/session/{sid}");
                                        app.set_alert(format!("Web URL: {url}"));
                                    } else {
                                        app.set_alert("no active session");
                                    }
                                }
                                "/resume" => {
                                    if let Some(arg) = _arg {
                                        let client = client.clone();
                                        let sid = arg.to_string();
                                        tokio::spawn(async move {
                                            let _ = client.attach_session(&sid).await;
                                        });
                                        app.set_alert(format!("attaching to {arg}…"));
                                    } else {
                                        app.set_alert("usage: /resume <session-id>");
                                    }
                                }
                                "/yield" => {
                                    let client = client.clone();
                                    tokio::spawn(async move {
                                        let msg = mew_protocol::ClientMessage::YieldControl {};
                                        if let Ok(json) = mew_protocol::encode_json(&msg) {
                                            let _ = client.send_raw(&json).await;
                                        }
                                    });
                                    app.set_alert("control yielded");
                                }
                                _ => {
                                    // Let the TUI handle it locally.
                                    let result = app.handle_slash(&text);
                                    handle_slash_result_local(&mut app, result);
                                }
                            }
                        }
                        mew_tui::events::Action::Clear => {
                            let client = client.clone();
                            tokio::spawn(async move {
                                client.slash_command("/clear".into()).await;
                            });
                        }
                        _ => {}
                    }
                }
            }
            mew_tui::Event::Agent(agent_event) => {
                app.handle_agent_event(agent_event);
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

/// Handle slash command results that the TUI processes locally (when
/// connected to a daemon). Only display-oriented commands — the
/// state-mutating ones are forwarded to the daemon.
fn handle_slash_result_local(app: &mut mew_tui::App, result: mew_tui::app::SlashResult) {
    use mew_tui::app::SlashResult;
    match result {
        SlashResult::Message(msg) => {
            app.push_synthetic_message(msg);
        }
        SlashResult::OpenModelPicker => {
            app.set_alert("model switching not available in daemon mode");
        }
        SlashResult::SwitchModel(_) => {
            app.set_alert("use the daemon's --model flag to switch models");
        }
        SlashResult::SetThinkingVariant(_) => {
            app.set_alert("thinking variant switching not available in daemon mode");
        }
        SlashResult::SetTheme(name) => {
            app.theme = mew_tui::theme::Theme::load(&name);
            app.set_alert(format!("theme: {}", app.theme.name));
        }
        SlashResult::PermissionModeMenu => {
            app.open_permission_mode_picker();
        }
        SlashResult::SetPermissionMode(m) => {
            app.permission_mode = m;
            app.set_alert(format!("permission mode: {}", m.id()));
        }
        SlashResult::PersonaSwitchConfirm(_) => {
            app.set_alert("persona switching not available in daemon mode");
        }
        SlashResult::SwitchPersona(_) => {
            app.set_alert("persona switching not available in daemon mode");
        }
        SlashResult::ResumeSession(_) => {
            app.set_alert("session resume not available in daemon mode");
        }
        SlashResult::Rewind(_) => {
            app.set_alert("rewind not available in daemon mode");
        }
        SlashResult::ToggleMouseCapture => {
            app.set_alert("toggle mouse capture");
        }
        SlashResult::Todo => {
            // TodosUpdated events come from the daemon.
        }
        SlashResult::Clear | SlashResult::Compact | SlashResult::Continue | SlashResult::Quit => {}
        SlashResult::PluginCommand { .. } => {
            app.set_alert("plugin commands not available in daemon mode");
        }
    }
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

/// Resolve the initial permission mode from CLI flags. Precedence:
/// `-D` (Dangerous) > `--auto-plus` (Auto+) > `-A` (Auto) >
/// `-P` (Permissive) > Standard. Dangerous and the Auto family both
/// bypass all gates but Dangerous is the stronger "trust me" signal.
fn resolve_mode(
    permissive: bool,
    auto: bool,
    auto_plus: bool,
    dangerous: bool,
) -> mew_hooks::PermissionMode {
    if dangerous {
        mew_hooks::PermissionMode::Dangerous
    } else if auto_plus {
        mew_hooks::PermissionMode::AutoPlus
    } else if auto {
        mew_hooks::PermissionMode::Auto
    } else if permissive {
        mew_hooks::PermissionMode::Permissive
    } else {
        mew_hooks::PermissionMode::Standard
    }
}

async fn run_cmd(
    provider_flag: String,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
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
        mode,
        prompt,
    )
    .await
}

async fn chat_cmd(
    provider_flag: String,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
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
        mode,
    )
    .await
}

/// Resolves a `provider/model` string into a `Provider`. Used by the
/// subagent runner to honor per-subagent `model:` overrides. Falls back to
/// the agent's current `provider_id` when the override has no `/`.
///
/// Tier keywords (`nano`, `micro`, `deci`) are resolved against the first
/// router provider in the config, not the active chat provider.
struct MainModelResolver {
    cfg: Arc<Config>,
    cat: Option<Arc<Catalog>>,
    default_provider_id: String,
    router_provider_id: Option<String>,
    raw: bool,
}

#[async_trait]
impl mew_subagents::ModelResolver for MainModelResolver {
    async fn resolve(&self, model: &str) -> Result<Arc<dyn Provider>, String> {
        let resolved_model = self.resolve_tier_keyword(model);

        let (provider_id, model_id) = if let Some(idx) = resolved_model.find('/') {
            (&resolved_model[..idx], &resolved_model[idx + 1..])
        } else {
            (self.default_provider_id.as_str(), resolved_model.as_str())
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

impl MainModelResolver {
    /// If a router provider is configured and `model` is a tier keyword,
    /// return the configured tier model ID. Falls back to the keyword itself
    /// so that literal model names still work when no router is configured.
    fn resolve_tier_keyword(&self, model: &str) -> String {
        let router_id = match self.router_provider_id.as_ref() {
            Some(id) => id,
            None => return model.to_string(),
        };
        let pc = match self.cfg.providers.get(router_id) {
            Some(pc) => pc,
            None => return model.to_string(),
        };
        match model {
            "nano" => {
                if pc.nano.is_empty() {
                    pc.micro_model().to_string()
                } else {
                    pc.nano.clone()
                }
            }
            "micro" => pc.micro_model().to_string(),
            "deci" => pc.deci_model().to_string(),
            _ => model.to_string(),
        }
    }
}

async fn run_tui(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<()> {
    let cat_for_resolver = cat.cloned();
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let provider =
        build_provider(cfg, cat, &provider_id, &model_id, raw).context("build provider")?;

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

    // Load personas.
    let persona_loader = mew_personas::Loader::new(std::env::current_dir().unwrap_or_default());
    let loaded_personas = persona_loader.load().unwrap_or_default();
    let personas_arc = Arc::new(loaded_personas.clone());

    let skill_filter = Arc::new(tokio::sync::RwLock::new(None));
    let template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>> =
        Arc::new(tokio::sync::RwLock::new(None));
    let pending_persona_switch = Arc::new(tokio::sync::Mutex::new(None));
    let current_persona_name = Arc::new(tokio::sync::RwLock::new(None));
    let mut tools = build_tools(
        skills.clone(),
        skill_filter.clone(),
        template_ctx.clone(),
        personas_arc.clone(),
        pending_persona_switch.clone(),
        current_persona_name.clone(),
        hashline_enabled_for(cfg, &provider_id),
    );

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

    let permission_engine = build_permission_engine(cfg, mode);

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
    agent.set_model_info(&model_id, &provider_id);
    agent.template_ctx = template_ctx;
    agent.flagged_files = flagged_files;
    agent.set_pending_persona_switch(pending_persona_switch.clone());
    agent.set_current_persona_name(current_persona_name.clone());
    agent.set_personas(loaded_personas.clone());
    agent.set_pending_persona_switch(pending_persona_switch.clone());
    {
        let cfg_clone = cfg.clone();
        let cat_clone = cat.cloned();
        agent.set_provider_builder(Box::new(move |model_str: &str| {
            let (pid, mid) = if let Some(idx) = model_str.find('/') {
                (&model_str[..idx], &model_str[idx + 1..])
            } else {
                (model_str, "")
            };
            build_provider(&cfg_clone, cat_clone.as_ref(), pid, mid, raw).map_err(|e| e.to_string())
        }));
    }
    agent.secrets = build_secret_set(cfg);
    agent.todos_path = todos_path.clone();
    {
        let shell_session = mew_tools::tools::shell_session::shared_session(
            std::env::current_dir().unwrap_or_default(),
        );
        agent.set_shell_session(shell_session);
    }
    if let Some(ref tp) = todos_path {
        if let Ok(list) = mew_agent::TodoList::load(tp).await {
            *agent.todos.lock().await = list;
        }
    }
    agent.set_permission_engine(permission_engine);
    maybe_set_classifier_provider(&mut agent, cfg, cat, raw, &provider_id, &model_id);
    agent.set_plan_path(&cfg.plan_path);
    agent.register_plugin_tools().await;
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
            router_provider_id: find_router_provider(cfg).map(|(id, _)| id),
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

    let project_vars = mew_context::load_project_vars(&std::env::current_dir().unwrap_or_default());

    if !ctx_files.is_empty() {
        let rendered_ctx = render_templated_context_files(&ctx_files, &agent);
        agent.set_system(mew_context::build_system_prompt(&rendered_ctx));
    }
    agent.project_vars = project_vars;

    // If skills are loaded, push them to the agent. The agent rebuilds its
    // system prompt so the skills XML reflects the active persona's filter.
    if !skills.is_empty() {
        agent.set_skills((*skills).clone());
    }

    let reasoning = resolve_reasoning(cat, &model_id, variant_flag.as_deref());
    if let Some(r) = reasoning {
        agent.set_reasoning(Some(r));
        info!(variant = ?variant_flag, model = %model_id, "enabled thinking variant");
    }

    let mut app = mew_tui::App::new();
    // Load theme: state.toml overrides config.toml, falling back to "dark".
    let state = mew_config::load_state().unwrap_or_default();
    let theme_name = if !state.theme.is_empty() {
        &state.theme
    } else {
        &cfg.tui.theme
    };
    app.theme = mew_tui::theme::Theme::load(theme_name);

    // Seed the sidebar's todos pane from whatever was loaded at startup.
    app.todos = agent.todos.lock().await.items.clone();

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
                if let Some(ref editor) = settings_editor {
                    editor.draw(f);
                }
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
                            let text = agent.dispatcher.on_user_input(text).await;
                            let cwd = std::env::current_dir().unwrap_or_default();
                            let (enriched, display, attachments) =
                                process_mentions(&text, &cwd, &mut app.context_files).await;
                            app.messages
                                .push(user_message(display, attachments.clone()));
                            app.streaming = true;
                            let agent_rx = agent.run_with_parts(enriched, attachments, None);
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
                                mew_tui::SlashResult::Todo => {
                                    let list = agent.todos.lock().await;
                                    app.messages.push(synthetic_message(list.render()));
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
                                            agent.set_model_info(new_model_id, new_provider_id);
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
                                mew_tui::SlashResult::SetThinkingVariant(ref variant) => {
                                    let model_id = &app.status.model;
                                    let variant_name = variant.as_str();
                                    if variant_name.is_empty()
                                        || variant_name == "off"
                                        || variant_name == "none"
                                    {
                                        agent.set_reasoning(None);
                                        app.messages
                                            .push(synthetic_message("thinking disabled".into()));
                                    } else {
                                        match resolve_reasoning(cat, model_id, Some(variant_name)) {
                                            Some(config) => {
                                                agent.set_reasoning(Some(config));
                                                app.messages.push(synthetic_message(format!(
                                                    "thinking variant: {}",
                                                    variant_name
                                                )));
                                            }
                                            None => {
                                                app.messages.push(synthetic_message(format!(
                                                    "unknown thinking variant '{variant_name}' for model '{model_id}'"
                                                )));
                                            }
                                        }
                                    }
                                }
                                mew_tui::SlashResult::SetTheme(ref name) => {
                                    app.theme = mew_tui::theme::Theme::load(name);
                                    // Persist to state.toml so it survives restart.
                                    {
                                        let mut save = mew_config::load_state().unwrap_or_default();
                                        save.theme = app.theme.name.clone();
                                        let _ = mew_config::save_state(&save);
                                    }
                                    app.messages.push(synthetic_message(format!(
                                        "theme: {}",
                                        app.theme.name
                                    )));
                                }
                                mew_tui::SlashResult::SwitchPersona(ref name) => {
                                    if name == "default" || name == "none" {
                                        let old = app.active_persona.clone();
                                        agent.clear_persona();
                                        app.active_persona = None;
                                        plugin_info.lock().unwrap().active_persona = None;
                                        app.messages.push(synthetic_message(
                                            "persona cleared (default)".into(),
                                        ));
                                        agent
                                            .dispatcher
                                            .on_persona_change(old.as_deref(), "default")
                                            .await;
                                    } else if let Some(persona) =
                                        loaded_personas.iter().find(|p| p.name == *name)
                                    {
                                        apply_persona_switch(
                                            &mut agent,
                                            &mut app,
                                            cfg,
                                            cat,
                                            provider_id.as_str(),
                                            raw,
                                            persona,
                                        );
                                    } else {
                                        app.messages.push(synthetic_message(format!(
                                            "unknown persona: {}. use /persona to list available.",
                                            name
                                        )));
                                    }
                                }
                                mew_tui::SlashResult::PersonaSwitchConfirm(ref name) => {
                                    if let Some(persona) =
                                        loaded_personas.iter().find(|p| p.name == *name)
                                    {
                                        let target = persona_summary(persona);
                                        let current = app
                                            .active_persona
                                            .as_ref()
                                            .and_then(|cur_name| {
                                                loaded_personas.iter().find(|p| &p.name == cur_name)
                                            })
                                            .map(persona_summary);
                                        app.request_persona_switch_confirm(target, current);
                                    } else {
                                        app.messages.push(synthetic_message(format!(
                                            "unknown persona: {}. use /persona to list available.",
                                            name
                                        )));
                                    }
                                }
                                mew_tui::SlashResult::Rewind(n) => {
                                    if app.streaming {
                                        app.messages.push(synthetic_message(
                                            "cannot rewind while streaming".into(),
                                        ));
                                    } else if n > app.messages.len() {
                                        app.messages.push(synthetic_message(format!(
                                            "only {} messages exist",
                                            app.messages.len()
                                        )));
                                    } else {
                                        let removed = app.messages.len() - n;
                                        app.rewind_to(n);
                                        {
                                            let mut msgs = agent.messages.lock().await;
                                            if n < msgs.len() {
                                                msgs.truncate(n);
                                            }
                                        }
                                        app.messages.push(synthetic_message(format!(
                                            "rewound to message {} (removed {})",
                                            n, removed
                                        )));
                                    }
                                }
                                mew_tui::SlashResult::ResumeSession(ref id) => {
                                    match mew_session::Reader::load(id).await {
                                        Ok(msgs) => {
                                            agent.load_messages(msgs.clone()).await;
                                            // Carry forward the resumed session's todos.
                                            let resumed_todos_path = mew_session::session_dir()
                                                .join(id)
                                                .join("todos.json");
                                            if let Ok(list) =
                                                mew_agent::TodoList::load(&resumed_todos_path).await
                                            {
                                                *agent.todos.lock().await = list;
                                            }
                                            app.todos = agent.todos.lock().await.items.clone();
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
                                mew_tui::SlashResult::PermissionModeMenu => {
                                    app.open_permission_mode_picker();
                                }
                                mew_tui::SlashResult::SetPermissionMode(mode) => {
                                    agent.set_permission_mode(mode);
                                    app.permission_mode = mode;
                                    let alert = match mode {
                                        mew_hooks::PermissionMode::Standard => {
                                            "Standard permission mode — prompts restored."
                                                .to_string()
                                        }
                                        mew_hooks::PermissionMode::Permissive => {
                                            "Permissive mode — Mutating tools auto-allow; \
                                             bash still prompts and your rules still apply."
                                                .to_string()
                                        }
                                        mew_hooks::PermissionMode::Auto => {
                                            "Auto mode — small LLM classifier decides each \
                                             tool call. Falls back to user on escalate."
                                                .to_string()
                                        }
                                        mew_hooks::PermissionMode::AutoPlus => {
                                            "Auto+ mode — classifier decides, but escalate or \
                                             failure means Deny (fail closed). No human in \
                                             the loop."
                                                .to_string()
                                        }
                                        mew_hooks::PermissionMode::Dangerous => {
                                            "⚠ Dangerous! mode — every tool auto-runs; \
                                             overrides deny rules, ask rules, and the \
                                             secret-file guard."
                                                .to_string()
                                        }
                                    };
                                    app.set_alert(alert);
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
                                    agent.set_model_info(new_model_id, new_provider_id);
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
                        mew_tui::events::Action::PersonaSwitchConfirmed(name) => {
                            if let Some(persona) = loaded_personas.iter().find(|p| p.name == name) {
                                let old = app.active_persona.clone();
                                apply_persona_switch(
                                    &mut agent,
                                    &mut app,
                                    cfg,
                                    cat,
                                    provider_id.as_str(),
                                    raw,
                                    persona,
                                );
                                plugin_info.lock().unwrap().active_persona = Some(name.clone());
                                agent
                                    .dispatcher
                                    .on_persona_change(old.as_deref(), &name)
                                    .await;
                            }
                        }
                        mew_tui::events::Action::SetPermissionMode(mode) => {
                            agent.set_permission_mode(mode);
                            app.permission_mode = mode;
                            let alert = match mode {
                                mew_hooks::PermissionMode::Standard => {
                                    "Standard permission mode — prompts restored for \
                                     Mutating/Dangerous tools."
                                        .to_string()
                                }
                                mew_hooks::PermissionMode::Permissive => {
                                    "Permissive mode — Mutating tools auto-allow; \
                                     bash still prompts and your rules still apply."
                                        .to_string()
                                }
                                mew_hooks::PermissionMode::Auto => {
                                    "Auto mode — small LLM classifier decides each \
                                     tool call. Falls back to user on escalate."
                                        .to_string()
                                }
                                mew_hooks::PermissionMode::AutoPlus => {
                                    "Auto+ mode — classifier decides, but escalate or \
                                     failure means Deny (fail closed). No human in \
                                     the loop."
                                        .to_string()
                                }
                                mew_hooks::PermissionMode::Dangerous => {
                                    "⚠ Dangerous! mode — every tool auto-runs; \
                                     overrides deny rules, ask rules, and the \
                                     secret-file guard."
                                        .to_string()
                                }
                            };
                            app.set_alert(alert);
                        }
                        mew_tui::events::Action::InsertAtMention(mention) => {
                            app.insert_mention(&mention);
                        }
                        mew_tui::events::Action::InsertSubagentMention(name) => {
                            app.insert_mention(&format!("@{} ", name));
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
                        mew_tui::events::Action::SetThinkingVariant(variant) => {
                            let model_id = &app.status.model;
                            if variant == "off" || variant == "none" {
                                agent.set_reasoning(None);
                                app.set_alert("thinking disabled");
                            } else {
                                match resolve_reasoning(cat, model_id, Some(&variant)) {
                                    Some(config) => {
                                        agent.set_reasoning(Some(config));
                                        app.set_alert(format!("thinking: {}", variant));
                                    }
                                    None => {
                                        app.set_alert(format!(
                                            "unknown thinking variant '{}' for model '{}'",
                                            variant, model_id
                                        ));
                                    }
                                }
                            }
                        }
                    }
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
        const STREAMING_DRAIN_LIMIT: u32 = 4;
        let mut pending_drain_submit: Option<String> = None;
        'drain: while let Ok(event) = event_rx.try_recv() {
            // Any non-tick event in the drain means we need a redraw.
            if !matches!(event, mew_tui::Event::Tick) {
                last_event_was_tick = false;
            }
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
                            mew_tui::events::Action::PersonaSwitchConfirmed(name) => {
                                if let Some(persona) =
                                    loaded_personas.iter().find(|p| p.name == name)
                                {
                                    apply_persona_switch(
                                        &mut agent,
                                        &mut app,
                                        cfg,
                                        cat,
                                        provider_id.as_str(),
                                        raw,
                                        persona,
                                    );
                                }
                            }
                            mew_tui::events::Action::SetPermissionMode(mode) => {
                                agent.set_permission_mode(mode);
                                app.permission_mode = mode;
                                let alert = match mode {
                                    mew_hooks::PermissionMode::Standard => {
                                        "Standard permission mode — prompts restored.".to_string()
                                    }
                                    mew_hooks::PermissionMode::Permissive => {
                                        "Permissive mode — Mutating tools auto-allow; \
                                         bash still prompts and your rules still apply."
                                            .to_string()
                                    }
                                    mew_hooks::PermissionMode::Auto => {
                                        "Auto mode — small LLM classifier decides each \
                                         tool call. Falls back to user on escalate."
                                            .to_string()
                                    }
                                    mew_hooks::PermissionMode::AutoPlus => {
                                        "Auto+ mode — classifier decides, but escalate or \
                                         failure means Deny (fail closed). No human in \
                                         the loop."
                                            .to_string()
                                    }
                                    mew_hooks::PermissionMode::Dangerous => {
                                        "⚠ Dangerous! mode — every tool auto-runs; \
                                         overrides deny rules, ask rules, and the \
                                         secret-file guard."
                                            .to_string()
                                    }
                                };
                                app.set_alert(alert);
                            }
                            mew_tui::events::Action::InsertAtMention(mention) => {
                                app.insert_mention(&mention);
                            }
                            mew_tui::events::Action::InsertSubagentMention(name) => {
                                app.insert_mention(&format!("@{} ", name));
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
                                    mew_tui::SlashResult::Todo => {
                                        let list = agent.todos.lock().await;
                                        app.messages.push(synthetic_message(list.render()));
                                    }
                                    mew_tui::SlashResult::SwitchModel(_) => {
                                        // Model switches are deferred; handled in main loop.
                                    }
                                    mew_tui::SlashResult::SetThinkingVariant(_) => {
                                        // Deferred; handled in main loop.
                                    }
                                    mew_tui::SlashResult::SetTheme(ref name) => {
                                        app.theme = mew_tui::theme::Theme::load(name);
                                        {
                                            let mut save =
                                                mew_config::load_state().unwrap_or_default();
                                            save.theme = app.theme.name.clone();
                                            let _ = mew_config::save_state(&save);
                                        }
                                        app.messages.push(synthetic_message(format!(
                                            "theme: {}",
                                            app.theme.name
                                        )));
                                    }
                                    mew_tui::SlashResult::SwitchPersona(_) => {
                                        // Handled inline above (direct clear).
                                    }
                                    mew_tui::SlashResult::PersonaSwitchConfirm(_) => {
                                        // Deferred; opens the confirm modal
                                        // in the main loop.
                                    }
                                    mew_tui::SlashResult::ResumeSession(_) => {
                                        // Deferred; handled in main loop when not streaming.
                                    }
                                    mew_tui::SlashResult::Rewind(_) => {
                                        // Deferred; handled in main loop when not streaming.
                                    }
                                    mew_tui::SlashResult::OpenModelPicker => {
                                        // Deferred to main loop; ignored during drain.
                                    }
                                    mew_tui::SlashResult::PermissionModeMenu => {
                                        // Deferred to main loop; ignored during drain.
                                    }
                                    mew_tui::SlashResult::SetPermissionMode(mode) => {
                                        // Deferred to main loop; ignored during drain.
                                        let _ = mode;
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
                            mew_tui::events::Action::SetThinkingVariant(_) => {
                                // Deferred to main loop; ignored during drain.
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

        // Process a Submit action deferred from the drain (needs async @mention reads).
        if let Some(text) = pending_drain_submit {
            let cwd = std::env::current_dir().unwrap_or_default();
            let (enriched, display, attachments) =
                process_mentions(&text, &cwd, &mut app.context_files).await;
            app.messages
                .push(user_message(display, attachments.clone()));
            app.streaming = true;
            let agent_rx = agent.run_with_parts(enriched, attachments, None);
            event_loop.forward_agent_events(agent_rx);
        }

        if should_break {
            break Ok(());
        }

        if app.should_quit {
            break Ok(());
        }
    };

    // Notify plugins the session is saving so they can flush state.
    agent.dispatcher.on_session_save().await;

    // Save sidebar collapsed state for next session.
    {
        let mut save = mew_config::load_state().unwrap_or_default();
        save.last_model = display_model.clone();
        save.last_provider = display_provider.clone();
        save.sidebar_collapsed = app.sidebar_collapsed.clone();
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

/// Resolve @mentions in `text`. Text files are inlined into the model-facing
/// text; image files become `Part::File` attachments. Returns
/// `(enriched, display, attachments)` where `enriched` carries the full file
/// content for the model and `display` carries only a `<path added to
/// context>` notification for the user's visible message — the file contents
/// should not flood the chat.
async fn process_mentions(
    text: &str,
    cwd: &std::path::Path,
    context_files: &mut Vec<String>,
) -> (String, String, Vec<Part>) {
    let mentions = mew_tui::app::parse_file_mentions(text);
    let mut enriched = text.to_string();
    let mut display = text.to_string();
    let mut attachments: Vec<Part> = Vec::new();

    for path_str in &mentions {
        let path = cwd.join(path_str);
        if let Some(mime) = image_mime(path_str) {
            let mention = format!("@{}", path_str);
            enriched = enriched.replace(&mention, "");
            display = display.replace(&mention, "");
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
                display.push_str(&format!("\n<{} added to context>", path_str));
            } else {
                let err = format!("\n\n[error reading {}: file not found]", path_str);
                enriched.push_str(&err);
                display.push_str(&err);
            }
        } else {
            match tokio::fs::read_to_string(&path).await {
                Ok(content) => {
                    enriched.push_str(&format!("\n\n--- {} ---\n{}", path_str, content));
                    if !context_files.contains(path_str) {
                        context_files.push(path_str.clone());
                    }
                    display.push_str(&format!("\n<{} added to context>", path_str));
                }
                Err(e) => {
                    let err = format!("\n\n[error reading {}: {}]", path_str, e);
                    enriched.push_str(&err);
                    display.push_str(&err);
                }
            }
        }
    }

    (enriched, display, attachments)
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

    // Pull umans models from the catalog. umans only documents an OpenAI-shaped
    // /v1/models/info (used by load_catalog above) and does not expose an
    // Anthropic-shaped /v1/models endpoint, so `provider.list_models()` for
    // umans returns nothing. The catalog has the authoritative entries
    // (context windows, capabilities) — seed the picker from there.
    //
    // Gated on credential presence for the same reason as the catalog load:
    // no key, no picker entries.
    if provider_has_credential(cfg, "umans") {
        if let Some(c) = cat {
            for (model_id, model_info) in &c.models {
                if model_info.provider != "umans" {
                    continue;
                }
                let full_id = format!("umans/{}", model_id);
                if seen.insert(full_id.clone()) {
                    let desc = format!("umans · anthropic · {} ctx", model_info.context_window);
                    models.push((full_id, desc));
                }
            }
        }
    }

    // Add hardcoded fallbacks if nothing discovered.
    if models.is_empty() {
        tracing::warn!("discovery: no models from any provider, using fallbacks");
        let mut fallbacks: Vec<(String, String)> = vec![
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
        // Only advertise umans in the fallback list when a credential is set.
        if provider_has_credential(cfg, "umans") {
            fallbacks.push(("umans/umans-coder".into(), "umans · anthropic".into()));
        }
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
        "z-ai" | "umans" => "anthropic",
        _ => "openai",
    }
}

#[allow(clippy::too_many_arguments)]
async fn build_and_run(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    variant_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
    prompt: String,
) -> Result<()> {
    let cat_for_resolver = cat.cloned();
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let provider =
        build_provider(cfg, cat, &provider_id, &model_id, raw).context("build provider")?;

    let display_model = model_id.clone();

    let session_id = ulid::Ulid::new().to_string();
    let session_writer = SessionWriter::open(&session_id)
        .await
        .context("open session")?;
    let todos_path = session_writer.path().parent().map(|p| p.join("todos.json"));

    let dispatcher = Arc::new(NopDispatcher);

    // Load skills for the skill tool.
    let skills_loader = mew_skills::Loader::new(std::env::current_dir().unwrap_or_default());
    let loaded_skills = skills_loader.load().unwrap_or_default();
    let skills = Arc::new(loaded_skills);

    // Load personas for the switch_persona tool.
    let persona_loader = mew_personas::Loader::new(std::env::current_dir().unwrap_or_default());
    let loaded_personas = persona_loader.load().unwrap_or_default();
    let personas_arc = Arc::new(loaded_personas.clone());

    let skill_filter = Arc::new(tokio::sync::RwLock::new(None));
    let template_ctx: Arc<tokio::sync::RwLock<Option<mew_prompts::template::TemplateContext>>> =
        Arc::new(tokio::sync::RwLock::new(None));
    let pending_persona_switch = Arc::new(tokio::sync::Mutex::new(None));
    let current_persona_name = Arc::new(tokio::sync::RwLock::new(None));
    let mut tools = build_tools(
        skills.clone(),
        skill_filter.clone(),
        template_ctx.clone(),
        personas_arc.clone(),
        pending_persona_switch.clone(),
        current_persona_name.clone(),
        hashline_enabled_for(cfg, &provider_id),
    );

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

    let permission_engine = build_permission_engine(cfg, mode);

    let mut agent = Agent::new(
        provider,
        dispatcher.clone(),
        Some(session_writer),
        tools,
        None,
    );
    agent.set_model_info(&model_id, &provider_id);
    agent.template_ctx = template_ctx;
    agent.set_pending_persona_switch(pending_persona_switch.clone());
    agent.set_current_persona_name(current_persona_name.clone());
    agent.set_personas(loaded_personas.clone());
    agent.set_pending_persona_switch(pending_persona_switch.clone());
    {
        let cfg_clone = cfg.clone();
        let cat_clone = cat.cloned();
        agent.set_provider_builder(Box::new(move |model_str: &str| {
            let (pid, mid) = if let Some(idx) = model_str.find('/') {
                (&model_str[..idx], &model_str[idx + 1..])
            } else {
                (model_str, "")
            };
            build_provider(&cfg_clone, cat_clone.as_ref(), pid, mid, raw).map_err(|e| e.to_string())
        }));
    }
    agent.flagged_files = flagged_files;
    {
        let shell_session = mew_tools::tools::shell_session::shared_session(
            std::env::current_dir().unwrap_or_default(),
        );
        agent.set_shell_session(shell_session);
    }
    agent.secrets = build_secret_set(cfg);
    agent.todos_path = todos_path.clone();
    if let Some(ref tp) = todos_path {
        if let Ok(list) = mew_agent::TodoList::load(tp).await {
            *agent.todos.lock().await = list;
        }
    }
    agent.register_plugin_tools().await;
    agent.set_permission_engine(permission_engine);
    maybe_set_classifier_provider(&mut agent, cfg, cat, raw, &provider_id, &model_id);
    agent.set_plan_path(&cfg.plan_path);
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
            router_provider_id: find_router_provider(cfg).map(|(id, _)| id),
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
    let project_vars = mew_context::load_project_vars(&std::env::current_dir().unwrap_or_default());
    agent.project_vars = project_vars;
    if !ctx_files.is_empty() {
        let rendered_ctx = render_templated_context_files(&ctx_files, &agent);
        agent.set_system(mew_context::build_system_prompt(&rendered_ctx));
    }
    if !skills.is_empty() {
        agent.set_skills((*skills).clone());
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
            mew_agent::AgentEvent::AskUser { tx, .. } => {
                // Non-interactive mode: no TUI to answer. Dropping `tx`
                // cancels the call so the model gets a clear "cancelled"
                // result instead of hanging.
                eprintln!("\n[ask_user_question: cancelled — no TUI in non-interactive mode]");
                drop(tx);
            }
            mew_agent::AgentEvent::TodosUpdated { .. } => {
                // No sidebar in non-interactive mode; nothing to update.
            }
            mew_agent::AgentEvent::PersonaSwitchRequested { .. } => {
                // Non-interactive mode: no TUI to confirm. The tool layer
                // already gates switch_persona via the permission engine,
                // and the switch is harmless on its own (no model pin
                // means just a system prompt + tool allowlist change), so
                // we silently drop the apply.
            }
            mew_agent::AgentEvent::JobUpdate { .. } => {
                // No sidebar in non-interactive mode; the job's output is
                // surfaced through its own tool result when it finishes.
            }
            mew_agent::AgentEvent::FileDelta { .. } => {
                // Diff stats are accumulated daemon-side; no-op in CLI mode.
            }
            mew_agent::AgentEvent::FlaggedFilesChanged { .. } => {
                // Flagged files visibility is web-UI only.
            }
        }
    }

    agent.dispatcher.on_stop().await;
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

/// Returns true if a credential is configured for the given provider.
///
/// Used to gate built-in providers on credential presence so the model picker
/// doesn't advertise models the user can't actually call. Silent on miss —
/// `get_credential` logs at debug level only, so this is cheap to call from
/// startup paths without spamming the log for users who never use a given
/// provider.
fn provider_has_credential(cfg: &Config, provider_id: &str) -> bool {
    match cfg.providers.get(provider_id) {
        Some(pc) => mew_config::get_credential(&pc.credential_ref).is_ok(),
        None => false,
    }
}

/// Build a direct provider adapter from a concrete provider config.
fn build_direct_provider(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    pc: &ProviderConfig,
    model_override: &str,
    raw: bool,
) -> Result<Arc<dyn Provider>> {
    let creds = mew_config::get_credential(&pc.credential_ref).context("get credential")?;

    let model = if model_override.is_empty() {
        if cfg.default_model.is_empty() {
            "deepseek-v4-flash".to_string()
        } else {
            cfg.default_model.clone()
        }
    } else {
        model_override.to_string()
    };

    let mut shape = pc.shape.clone();
    if let Some(c) = cat {
        let s = c.shape_for(&model);
        if !s.is_empty() {
            shape = s.to_string();
        }
    }

    let mut base_url = pc.base_url.clone();
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

    // Router providers are task-only primitives used by subagents and the
    // permission classifier. They cannot be selected as the main chat provider.
    if pc.kind == "router" {
        anyhow::bail!(
            "provider '{}' is a router; router providers cannot be used as the main chat provider",
            provider_id
        );
    }

    build_direct_provider(cfg, cat, provider_id, &pc, model_override, raw)
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
