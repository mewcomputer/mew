use anyhow::Result;
use clap::{CommandFactory, Parser};

mod cli;
mod commands;
mod config_editor;
mod runtime;
mod setup;

use cli::*;

use setup::providers::{resolve_model_opt, resolve_provider};

use commands::run::{resolve_mode, run_cmd};
use commands::tui::{chat_cmd, chat_with_daemon};

/// Ask the user a yes/no question on stderr. Returns:
/// - `Some(true)` / `Some(false)` for explicit y/n.
/// - `None` if stdin isn't a TTY (e.g. piped / CI) or the read fails —
///   callers should treat None as "can't ask, bail out".
pub(crate) fn prompt_yn(question: &str) -> Option<bool> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return None;
    }
    eprint!("{} [y/N] ", question);
    let _ = std::io::stderr().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return None;
    }
    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => Some(true),
        "" | "n" | "no" => Some(false),
        _ => None,
    }
}

/// Detect persisted state that references providers/models no longer in
/// the active config, prompt the user to heal, and act on their answer.
/// Called once at startup, before subcommand dispatch.
fn startup_state_health_check(cfg: &mew_config::Config, state: &mew_config::State) -> Result<()> {
    let issues = mew_config::validate_state(cfg, state);
    if issues.is_empty() {
        return Ok(());
    }

    eprintln!(
        "Warning: state.toml at {} contains unrecognized values:",
        mew_config::state_file_path().display()
    );
    for issue in &issues {
        eprintln!("  - {}", issue);
    }
    eprintln!(
        "These were probably written by an older or partial run. Healing will clear the invalid fields (keeping theme, sidebar, and plugin preferences) and back up the current file."
    );

    match prompt_yn("Heal state.toml now?") {
        Some(true) => {
            let backup = mew_config::backup_state_file()?;
            let healed = mew_config::heal_state(cfg, state);
            mew_config::save_state(&healed)?;
            eprintln!(
                "Healed. Backup saved to {}.",
                backup.file_name().unwrap_or_default().to_string_lossy()
            );
            Ok(())
        }
        Some(false) => {
            eprintln!(
                "Exiting without changes. Fix state.toml manually, or rerun and accept the heal."
            );
            std::process::exit(0);
        }
        None => {
            // Non-interactive session (piped stdin, CI, etc.). We can't ask,
            // and silently dropping the bogus values would mask the problem
            // from the user. Surface the issues and exit with a distinct
            // code so scripts can detect the situation.
            eprintln!("Non-interactive session — cannot prompt. Re-run from a terminal to heal.");
            std::process::exit(2);
        }
    }
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
        let pidfile = pidfile
            .clone()
            .unwrap_or_else(commands::daemon::default_pidfile);
        return commands::daemon::stop_daemon(&pidfile);
    }

    // Handle --background: double-fork + setsid before the runtime starts.
    // The parent exits immediately; the child continues into the runtime.
    // Returns true if we daemonized (and already inited tracing).
    let daemonized = if let Some(Commands::Daemon {
        background: true,
        log,
        pidfile,
        socket,
        port,
        #[cfg(feature = "iroh")]
        iroh,
        ..
    }) = &cli.command
    {
        #[cfg(feature = "iroh")]
        let is_iroh = *iroh;
        #[cfg(not(feature = "iroh"))]
        let is_iroh = false;

        // Pre-daemonize socket liveness check — error reaches the terminal.
        // Only guard the Unix socket if this daemon will actually bind one;
        // `--iroh` and TCP-only (`--port` without `--socket`) modes do not.
        if !is_iroh && (socket.is_some() || port.is_none()) {
            let socket_path = socket
                .clone()
                .unwrap_or_else(commands::daemon::default_socket_path);
            mew_daemon::check_socket_liveness(&socket_path)?;
        }

        let pidfile = pidfile
            .clone()
            .unwrap_or_else(commands::daemon::default_pidfile);
        commands::daemon::daemonize(log.as_deref(), &pidfile)?;
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
    // Load config early so resolve_provider / resolve_model_opt can validate
    // persisted state values against the configured provider set before using
    // them. If the load fails (e.g. corrupt TOML), fall back to defaults —
    // subcommands re-load the config on their own.
    let cfg = mew_config::load().unwrap_or_default();

    // Heal persisted state if it references providers/models that no longer
    // exist in the active config. Interactive prompt; non-TTY exits 2.
    startup_state_health_check(&cfg, &state)?;

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
            let provider = resolve_provider(provider, &state, &cfg);
            let model = resolve_model_opt(model, &state, &cfg);
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
                let provider = resolve_provider(provider, &state, &cfg);
                let model = resolve_model_opt(model, &state, &cfg);
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
            #[cfg(feature = "iroh")]
            iroh,
        }) => {
            // --background and --stop are handled before the tokio runtime
            // starts (in main()). By the time we reach here, we're already
            // in the background daemon process (or running foreground).
            let provider = resolve_provider(provider, &state, &cfg);
            let model = resolve_model_opt(model, &state, &cfg);
            let mode = resolve_mode(permissive, auto, auto_plus, dangerously_skip_permissions);
            #[cfg(feature = "iroh")]
            if iroh {
                return commands::daemon::run_daemon_iroh(
                    fake_provider,
                    &provider,
                    model,
                    raw,
                    mode,
                )
                .await;
            }
            commands::daemon::run_daemon(socket, port, fake_provider, &provider, model, raw, mode)
                .await
        }
        #[cfg(feature = "iroh")]
        Some(Commands::Pair) => commands::daemon::pair_cmd().await,
        Some(Commands::Auth { command }) => commands::auth::auth_cmd(command).await,
        None => {
            let provider = resolve_provider(None, &state, &cfg);
            let model = resolve_model_opt(None, &state, &cfg);
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
            commands::config::config_cmd(command)?;
            Ok(())
        }
        Some(Commands::Debug { command }) => commands::debug::debug_cmd(command).await,
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
        Some(Commands::Theme { command }) => commands::theme::theme_cmd(command),
        Some(Commands::Ext { command }) => commands::ext::ext_cmd(command),
    }
}

/// Session-level info exposed to plugins via the `config_read` callback.
/// Plugins can query `session_id`, `model`, `provider`, `workspace`, and
/// `active_persona`. Updated when persona/model changes.
pub(crate) struct PluginInfo {
    pub(crate) session_id: String,
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) workspace: String,
    pub(crate) active_persona: Option<String>,
}

#[cfg(test)]
#[path = "dispatch_table_tests.rs"]
mod dispatch_table_tests;
