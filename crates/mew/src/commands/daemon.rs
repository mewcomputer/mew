//! Daemon-related command functions.
//!
//! Extracted from `main.rs` as pure code motion. These build and run the
//! daemon server, handle daemonization / process control, and the iroh
//! pairing flow.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

use mew_agent::Agent;
use mew_message::SessionId;

#[cfg(feature = "iroh")]
fn remote_invite_payload(node_id: &str, token: &str) -> String {
    format!("computer.mew.mew://{node_id}?token={token}")
}

/// Build a fully-configured `DaemonServer` with the given provider settings.
///
/// This is the shared server-building logic used by both `run_daemon` (Unix/TCP)
/// and `run_daemon_iroh` (iroh p2p). It handles:
/// - Loading config + catalog
/// - Building the agent-builder closure (fake or real provider)
/// - Wiring model switcher, lister, and thinking setter
///
/// The caller is responsible for starting a listener (`run`, `run_tcp`, or `run_iroh`)
/// on the returned server.
pub(crate) async fn build_daemon_server(
    fake_provider: bool,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<mew_daemon::DaemonServer> {
    let cfg = mew_config::load().context("load config")?;
    let cat = crate::setup::providers::load_catalog(&cfg).await;

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
            let mut agent = Agent::new(
                provider,
                dispatcher,
                Some(params.writer),
                Vec::new(),
                session_id,
            );
            agent.set_model_info("fake", "fake");
            let personas = mew_personas::builtin_defaults();
            agent.set_personas(personas.clone());
            if let Some(persona) = personas.iter().find(|p| p.name == "builder") {
                agent.apply_persona(persona);
            }
            Ok((agent, Some("fake".to_string()), Some("fake".to_string())))
        })
    } else {
        let (provider_id, model_id) = crate::setup::providers::resolve_model(
            &cfg,
            (*cat).as_ref(),
            provider_flag,
            model_flag,
        );
        let provider_id = Arc::new(provider_id);
        let model_id = Arc::new(model_id);
        let model_id_display = model_id.clone();
        let provider_id_display = Arc::clone(&provider_id);
        info!("mew daemon starting, model={}", model_id_display);
        Arc::new(move |params: mew_daemon::AgentBuildParams| {
            let cfg = (*cfg).clone();
            let cat = (*cat).clone();
            let provider_id = (*provider_id).clone();
            let model_id = (*model_id).clone();
            let session_id: Option<SessionId> = params
                .session_id
                .strip_prefix("sess_")
                .and_then(|s| ulid::Ulid::from_string(s).ok());
            let mut agent = crate::setup::agent::build_session_agent(
                &cfg,
                cat.as_ref(),
                &provider_id,
                &model_id,
                raw,
                mode,
                Some(params.writer),
                session_id,
                params.cwd,
                params.browser_enabled,
                Arc::new(mew_hooks::NopDispatcher),
                None,
                &[],
            )?;

            // Apply the model's default thinking variant, matching the
            // non-daemon `run` path. The daemon otherwise starts with
            // `reasoning = None` even for models that think by default, making
            // the CLI report thinking as off when it isn't.
            match crate::setup::providers::resolve_reasoning(cat.as_ref(), &model_id, None) {
                Some((config, name)) => {
                    agent.set_reasoning(Some(config));
                    agent.set_active_thinking_variant(Some(name));
                }
                None => {
                    agent.set_reasoning(None);
                    agent.set_active_thinking_variant(None);
                }
            }

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
    if fake_provider {
        let switcher: mew_daemon::ModelSwitcher =
            Arc::new(|agent: &mut Agent, provider: &str, model: &str| {
                if provider != "fake" || model != "fake" {
                    anyhow::bail!("fake daemon only supports fake/fake")
                }
                agent.set_model_info(provider, model);
                Ok((provider.to_owned(), model.to_owned()))
            });
        let lister: mew_daemon::ModelLister = Arc::new(|| {
            vec![mew_protocol::ModelInfo {
                id: "fake/fake".into(),
                provider: "fake".into(),
                model: "fake".into(),
                description: Some("local test model · standard".into()),
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            }]
        });
        server = server.with_model_management(switcher, lister);
    } else {
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

            // Collect provider IDs that are usable. Only show models the user
            // can actually call — an API-key credential OR an OAuth token file
            // (for codex, whose credential is the token file).
            let cred_pids: Vec<String> = cfg
                .providers
                .keys()
                .filter(|pid| crate::setup::providers::provider_available(cfg, pid))
                .cloned()
                .collect();

            if let Some(cat) = cat_lister.as_ref() {
                for provider_id in &cred_pids {
                    for (catalog_provider_id, provider_models) in &cat.providers {
                        if !crate::setup::providers::catalog_provider_matches(
                            provider_id,
                            catalog_provider_id,
                        ) {
                            continue;
                        }
                        for m in provider_models.values() {
                            // Skip image/video/audio generation models — the
                            // agent can't consume their output.
                            if !m.text_output {
                                continue;
                            }
                            let thinking_variants = cat
                                .thinking_variants(&m.id)
                                .into_iter()
                                .map(|v| mew_protocol::ThinkingVariantInfo { name: v.name })
                                .collect();
                            models.push(mew_protocol::ModelInfo {
                                id: format!("{}/{}", provider_id, m.id),
                                provider: provider_id.clone(),
                                model: m.id.clone(),
                                description: Some(format!(
                                    "{} ctx · {}",
                                    m.context_window,
                                    if m.reasoning { "reasoning" } else { "standard" }
                                )),
                                thinking_variants,
                                thinking_budget: cat.thinking_budget(&m.id).map(|b| {
                                    mew_protocol::ThinkingBudgetInfo {
                                        min: b.min,
                                        max: b.max,
                                        step: b.step,
                                        default: b.default,
                                        by_effort: b.by_effort,
                                    }
                                }),
                                context_window: Some(m.context_window),
                            });
                        }
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
                let new_provider = crate::setup::providers::build_provider(
                    &cfg_switcher,
                    cat_ref,
                    provider_id,
                    model_id,
                    raw2,
                    Some(agent.session_id),
                )
                .context("build provider for model switch")?;
                agent.provider = new_provider;
                if let Some(c) = cat_ref {
                    agent.supports_vision = c.supports_vision(model_id);
                    agent.context_window = c.context_window(model_id).max(0) as u32;
                    if let Some(raw_max) = c.max_output(model_id) {
                        agent.set_default_max_output_tokens(raw_max.min(32768));
                    }
                }
                // Reset thinking to the new model's default so the active
                // variant tracks the switched-to model rather than the
                // previous one.
                match crate::setup::providers::resolve_reasoning(cat_ref, model_id, None) {
                    Some((config, name)) => {
                        agent.set_reasoning(Some(config));
                        agent.set_active_thinking_variant(Some(name));
                    }
                    None => {
                        agent.set_reasoning(None);
                        agent.set_active_thinking_variant(None);
                    }
                }
                Ok((provider_id.to_string(), model_id.to_string()))
            },
        );

        server = server.with_model_management(switcher, lister);

        // Thinking variant setter: resolves a variant name via the catalog
        // and applies it to the agent's reasoning config. Returns the
        // canonical resolved name (a clamped/snapped `budget:<n>` for
        // numeric budgets) so the client can display what was applied.
        let cat_thinking = Arc::clone(&cat_for_models);
        let thinking_setter: mew_daemon::ThinkingSetter =
            Arc::new(move |agent: &mut Agent, model_id: &str, variant: &str| {
                let cat_ref = (*cat_thinking).as_ref();
                if variant.is_empty() {
                    agent.set_reasoning(None);
                    return Ok(None);
                }
                match crate::setup::providers::resolve_reasoning(cat_ref, model_id, Some(variant)) {
                    Some((config, resolved_name)) => {
                        agent.set_reasoning(Some(config));
                        agent.set_active_thinking_variant(Some(resolved_name.clone()));
                        Ok(Some(resolved_name))
                    }
                    // Unknown variant — or "off" on a model without an
                    // explicit off variant — is a plain disable, matching
                    // the previous wire behavior.
                    None => {
                        agent.set_reasoning(None);
                        agent.set_active_thinking_variant(None);
                        Ok(None)
                    }
                }
            });
        server = server.with_thinking_setter(thinking_setter);
    }

    Ok(server)
}

/// Run the daemon. Builds an agent per connection via `build_session_agent`.
/// Listens on the Unix socket (if `--socket` is set or by default) AND/OR
/// the TCP address (if `--port` is set). With neither flag, listens on the
/// default Unix socket.
///
/// If `fake_provider` is true, all real-provider setup is bypassed and
/// every connection gets a `FakeProvider`-backed agent. Used for tests
/// and offline demos.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_daemon(
    socket: Option<String>,
    port: Option<String>,
    fake_provider: bool,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
    pidfile: Option<String>,
) -> Result<()> {
    let server = build_daemon_server(fake_provider, provider_flag, model_flag, raw, mode).await?;

    let pidfile = pidfile.unwrap_or_else(default_pidfile);
    write_local_status(Path::new(&pidfile), socket.as_deref(), port.as_deref())?;

    run_local_server(server, socket, port).await
}

async fn run_local_server(
    server: mew_daemon::DaemonServer,
    socket: Option<String>,
    port: Option<String>,
) -> Result<()> {
    // Default to Unix socket at $XDG_RUNTIME_DIR/mew.sock or /tmp/mew.sock,
    // unless `--port` was given and `--socket` was not (in which case the
    // daemon is TCP-only).
    let socket_path = socket.clone().unwrap_or_else(default_socket_path);
    let use_unix = socket.is_some() || port.is_none();

    match (use_unix, port.as_deref()) {
        (true, Some(addr)) => {
            let parsed: std::net::SocketAddr = addr
                .parse()
                .with_context(|| format!("invalid --port address: {addr}"))?;
            let server_unix = server.clone_for_listener();
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
// iroh remote access
// ---------------------------------------------------------------------------

/// Build a daemon server (same as `run_daemon`) but listen on iroh instead
/// of Unix socket / TCP. Used by `mew daemon --iroh`.
///
/// Reuses `build_daemon_server`'s server-building logic, then passes the
/// server components to `run_iroh` which binds an iroh endpoint.
#[cfg(feature = "iroh")]
pub(crate) async fn run_daemon_iroh(
    fake_provider: bool,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
    scoped: bool,
) -> Result<()> {
    eprintln!(
        "WARNING: iroh daemon mode exposes this daemon to paired remote devices. They may access sessions, files, commands, and permission requests."
    );
    let server = build_daemon_server(fake_provider, provider_flag, model_flag, raw, mode).await?;

    let allowlist_path = mew_daemon::iroh_transport::default_allowlist_path();
    let secret_key_path = mew_daemon::iroh_transport::default_secret_key_path();
    info!(allowlist = %allowlist_path.display(), "starting iroh listener");

    let secret_key = mew_daemon::iroh_transport::load_or_create_secret_key(&secret_key_path)?;
    mew_daemon::iroh_transport::run_iroh(
        server.session_manager,
        server.groups_store,
        server.thinking_setter,
        server.auto_summary_enabled,
        mew_daemon::iroh_transport::IrohListenerConfig {
            allowlist_path,
            secret_key,
            remote_scope: scoped.then_some(mew_protocol::RemoteScope::Control),
            remote_token: None,
            remote_store: None,
            remote_mode: mew_daemon::remote::RemoteMode::Daemon,
        },
    )
    .await
}

/// Run the local daemon and an authenticated iroh listener together. This is
/// the app/VPS remote mode; the local WebSocket remains available throughout.
#[cfg(feature = "iroh")]
pub(crate) async fn run_daemon_remote(
    socket: Option<String>,
    port: Option<String>,
    fake_provider: bool,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<()> {
    eprintln!(
        "WARNING: remote access is enabled. A paired device can reach this daemon over the network; relay connections may be used when direct peer-to-peer access fails."
    );
    let server = build_daemon_server(fake_provider, provider_flag, model_flag, raw, mode).await?;
    let remote_server = server.clone_for_listener();
    let allowlist_path = mew_daemon::iroh_transport::default_allowlist_path();
    let secret_key_path = mew_daemon::iroh_transport::default_secret_key_path();
    let secret_key = mew_daemon::iroh_transport::load_or_create_secret_key(&secret_key_path)?;
    let remote_store = Arc::new(mew_daemon::remote::RemoteAccessStore::load(
        mew_daemon::remote::default_state_path(),
    )?);
    let remote_mode = std::env::var("MEW_REMOTE_MODE")
        .ok()
        .filter(|mode| mode.eq_ignore_ascii_case("desktop"))
        .map(|_| mew_daemon::remote::RemoteMode::Desktop)
        .unwrap_or(mew_daemon::remote::RemoteMode::Daemon);

    let local = run_local_server(server, socket, port);
    let remote = mew_daemon::iroh_transport::run_iroh(
        remote_server.session_manager,
        remote_server.groups_store,
        remote_server.thinking_setter,
        remote_server.auto_summary_enabled,
        mew_daemon::iroh_transport::IrohListenerConfig {
            allowlist_path,
            secret_key,
            remote_scope: Some(mew_protocol::RemoteScope::Control),
            remote_token: None,
            remote_store: Some(remote_store.clone()),
            remote_mode,
        },
    );

    let remote_store_for_task = remote_store.clone();
    let remote_task = tokio::spawn(async move {
        if let Err(error) = remote.await {
            tracing::error!(%error, "iroh remote listener stopped; local daemon remains available");
            let _ = remote_store_for_task.set_hosting(false, remote_mode, None);
        }
    });
    let result = local.await;
    let _ = remote_store.set_hosting(false, remote_mode, None);
    remote_task.abort();
    result
}

/// `mew pair` — create a short-lived, single-use remote invite.
///
/// The daemon must be running with `--remote`; this command only creates the
/// pairing record and prints a payload. It never opens a second listener or
/// authorizes the first peer that happens to connect.
#[cfg(feature = "iroh")]
pub(crate) async fn pair_cmd() -> Result<()> {
    eprintln!(
        "WARNING: this invite grants a remote device control access to the daemon. It expires in 120 seconds and can be used once."
    );
    let secret_key_path = mew_daemon::iroh_transport::default_secret_key_path();
    let secret_key = mew_daemon::iroh_transport::load_or_create_secret_key(&secret_key_path)?;
    let node_id_str = secret_key.public().to_string();
    let remote_store = Arc::new(mew_daemon::remote::RemoteAccessStore::load(
        mew_daemon::remote::default_state_path(),
    )?);
    if !remote_store.snapshot().enabled {
        anyhow::bail!("remote access is not active; start `mew daemon --remote` before creating a pairing invite");
    }
    let remote_token = remote_store.create_pairing(
        mew_protocol::RemoteScope::Control,
        mew_daemon::remote::unix_now(),
        120,
    )?;

    println!("Daemon NodeId: {node_id_str}");
    // Keep the mobile QR payload backward-compatible while carrying the
    // one-time credential required by the explicit remote endpoint.
    let payload = remote_invite_payload(&node_id_str, &remote_token);
    let qr = qrcode::QrCode::new(payload).context("generate QR code")?;
    let qr_string = qr
        .render::<qrcode::render::unicode::Dense1x2>()
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .build();
    println!("{qr_string}");

    eprintln!("\nTo connect manually, use this NodeId: {node_id_str}");
    eprintln!("The pairing token is embedded in the QR payload and is not persisted in plaintext.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Daemonization
// ---------------------------------------------------------------------------

/// Default socket path: `$XDG_RUNTIME_DIR/mew.sock` or `/tmp/mew.sock`.
pub(crate) fn default_socket_path() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| format!("{d}/mew.sock"))
        .unwrap_or_else(|_| "/tmp/mew.sock".to_string())
}

/// Default PID file path: `$XDG_RUNTIME_DIR/mew.pid` or `/tmp/mew.pid`.
pub(crate) fn default_pidfile() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| format!("{d}/mew.pid"))
        .unwrap_or_else(|_| "/tmp/mew.pid".to_string())
}

/// Default status file path: next to the pidfile. The daemon writes
/// `{version, pid, socket, port, started_at_unix}` here at startup;
/// `mew daemon --status` reads it for build/endpoint metadata while the
/// pidfile (or the status file's own pid) is the liveness source of truth.
fn status_path_for(pidfile: &Path) -> PathBuf {
    pidfile.with_file_name("mew.status.json")
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Write the daemon status file next to the pidfile at daemon startup.
/// Written before the listeners bind; a stale file with a dead pid is
/// reported as "not running" by `--status`, so a failed bind leaves no
/// misleading state.
pub(crate) fn write_local_status(
    pidfile: &Path,
    socket: Option<&str>,
    port: Option<&str>,
) -> Result<()> {
    // Mirror run_local_server's socket decision: an explicit --socket wins,
    // else the default Unix socket, unless --port alone made it TCP-only.
    let socket_path = match (socket.is_some() || port.is_none(), socket) {
        (true, Some(s)) => Some(s.to_string()),
        (true, None) => Some(default_socket_path()),
        (false, _) => None,
    };
    let status = serde_json::json!({
        "version": crate::version::git_rev(),
        "pid": std::process::id(),
        "socket": socket_path,
        "port": port,
        "started_at_unix": unix_now(),
    });
    let path = status_path_for(pidfile);
    std::fs::write(&path, serde_json::to_vec_pretty(&status)?)
        .with_context(|| format!("write daemon status file {}", path.display()))
}

fn human_age(total_secs: u64) -> String {
    if total_secs < 60 {
        format!("{total_secs}s ago")
    } else if total_secs < 3600 {
        format!("{}m {}s ago", total_secs / 60, total_secs % 60)
    } else {
        format!("{}h {}m ago", total_secs / 3600, total_secs % 3600 / 60)
    }
}

struct DaemonStatusMeta {
    version: String,
    pid: i32,
    socket: Option<String>,
    port: Option<String>,
    started_at_unix: i64,
}

fn read_status(statusfile: &Path) -> Option<DaemonStatusMeta> {
    let raw = std::fs::read_to_string(statusfile).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(DaemonStatusMeta {
        version: v.get("version")?.as_str()?.to_string(),
        pid: v.get("pid")?.as_i64()? as i32,
        socket: v.get("socket").and_then(|s| s.as_str()).map(str::to_string),
        port: v.get("port").and_then(|s| s.as_str()).map(str::to_string),
        started_at_unix: v.get("started_at_unix")?.as_i64()?,
    })
}

fn read_pidfile(pidfile: &Path) -> Option<i32> {
    std::fs::read_to_string(pidfile).ok()?.trim().parse().ok()
}

/// Report daemon liveness and identity as a printable report. Returns
/// `(running, report)`, where `running` doubles as the process exit code
/// state so `mew daemon --status` can exit 0/1 directly.
///
/// The pidfile is the primary liveness source (which is what `--stop`
/// kills); the status file's pid is a fallback so a foreground daemon
/// that never wrote a pidfile is still found.
pub(crate) fn status_daemon(pidfile: &Path) -> Result<(bool, String)> {
    let statusfile = status_path_for(pidfile);

    let pid = read_pidfile(pidfile).or_else(|| read_status(&statusfile).map(|m| m.pid));
    let Some(pid) = pid else {
        return Ok((
            false,
            format!(
                "daemon: not running\npidfile: {} (not found)\n",
                pidfile.display()
            ),
        ));
    };

    let alive = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        None::<nix::sys::signal::Signal>,
    )
    .is_ok();
    if !alive {
        return Ok((
            false,
            format!(
                "daemon: not running (pid {pid} from {} no longer alive)\n",
                pidfile.display()
            ),
        ));
    }

    let mut out = format!("daemon: running\npid: {pid}\n");
    match read_status(&statusfile) {
        Some(meta) => {
            out.push_str(&format!("build: {}\n", meta.version));
            if let Some(socket) = meta.socket {
                out.push_str(&format!("socket: {socket}\n"));
            }
            if let Some(port) = meta.port {
                out.push_str(&format!("port: {port}\n"));
            }
            let age = unix_now().saturating_sub(meta.started_at_unix).max(0) as u64;
            out.push_str(&format!("started: {}\n", human_age(age)));
            let local = crate::version::git_rev();
            out.push_str(&format!("local binary build: {local}\n"));
            if local != meta.version {
                out.push_str(&format!(
                    "warning: daemon build ({}) differs from this binary ({local}); run `mew daemon --stop` to restart it\n",
                    meta.version
                ));
            }
        }
        None => {
            out.push_str(&format!(
                "build: unknown (no status file at {})\n",
                statusfile.display()
            ));
            out.push_str(&format!(
                "local binary build: {}\n",
                crate::version::git_rev()
            ));
            out.push_str(
                "warning: daemon predates the current binary; run `mew daemon --stop` to restart it on this build\n",
            );
        }
    }
    Ok((true, out))
}

/// Detach from the controlling terminal via the standard double-fork +
/// setsid pattern. After this returns, the process is a session leader
/// with no controlling terminal, immune to SIGHUP from logout.
///
/// If `log_file` is given, stdio is redirected there; otherwise to
/// `/dev/null`. The PID is written to `pidfile`.
pub(crate) fn daemonize(log_file: Option<&str>, pidfile: &str) -> Result<()> {
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
pub(crate) fn stop_daemon(pidfile: &str) -> Result<()> {
    let pid_path = Path::new(pidfile);
    let pid_str =
        std::fs::read_to_string(pid_path).with_context(|| format!("read pidfile {pidfile}"))?;
    let pid: i32 = pid_str
        .trim()
        .parse()
        .with_context(|| format!("parse PID from {pidfile}: {pid_str:?}"))?;

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGTERM,
    )
    .context("send SIGTERM")?;

    // Remove the pidfile and the status file.
    let _ = std::fs::remove_file(pid_path);
    let _ = std::fs::remove_file(status_path_for(pid_path));

    println!("daemon (PID {pid}) stopped");
    Ok(())
}

#[cfg(all(test, feature = "iroh"))]
mod tests {
    use super::remote_invite_payload;

    #[test]
    fn remote_invite_payload_carries_the_pairing_token() {
        assert_eq!(
            remote_invite_payload("node-1", "mew_invite"),
            "computer.mew.mew://node-1?token=mew_invite"
        );
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    /// Create an isolated pidfile/statusfile pair in a fresh temp dir.
    /// Returns `(dir, pidfile, statusfile)`.
    fn tmp_files(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("mew-status-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("mew.pid");
        let statusfile = status_path_for(&pidfile);
        (dir, pidfile, statusfile)
    }

    #[test]
    fn status_file_sits_next_to_pidfile() {
        assert_eq!(
            status_path_for(Path::new("/tmp/foo/mew.pid")),
            Path::new("/tmp/foo/mew.status.json")
        );
    }

    #[test]
    fn not_running_when_pidfile_missing() {
        let (dir, pidfile, _) = tmp_files("missing");
        let (running, report) = status_daemon(&pidfile).unwrap();
        assert!(!running);
        assert!(report.contains("daemon: not running"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_running_when_pid_dead() {
        let (dir, pidfile, _) = tmp_files("dead");
        // 999999999 is not a live process in practice.
        std::fs::write(&pidfile, "999999999\n").unwrap();
        let (running, report) = status_daemon(&pidfile).unwrap();
        assert!(!running);
        assert!(report.contains("no longer alive"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn running_reports_version_endpoints_and_local_build() {
        let (dir, pidfile, statusfile) = tmp_files("running");
        // Our own pid is alive, so the liveness probe says "running".
        std::fs::write(&pidfile, format!("{}\n", std::process::id())).unwrap();
        let status = serde_json::json!({
            "version": "0000000",
            "pid": std::process::id(),
            "socket": "/tmp/mew.sock",
            "port": "127.0.0.1:1234",
            "started_at_unix": 0,
        });
        std::fs::write(&statusfile, serde_json::to_vec(&status).unwrap()).unwrap();

        let (running, report) = status_daemon(&pidfile).unwrap();
        assert!(running);
        assert!(report.contains("daemon: running"));
        assert!(report.contains("build: 0000000"));
        assert!(report.contains("socket: /tmp/mew.sock"));
        assert!(report.contains("port: 127.0.0.1:1234"));
        assert!(report.contains("started:"));
        let local = crate::version::git_rev();
        assert!(report.contains(&format!("local binary build: {local}")));
        if local != "0000000" {
            assert!(report.contains("mew daemon --stop"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn running_without_status_file_reports_unknown_build_and_restart_hint() {
        let (dir, pidfile, _) = tmp_files("nostatus");
        std::fs::write(&pidfile, format!("{}\n", std::process::id())).unwrap();

        let (running, report) = status_daemon(&pidfile).unwrap();
        assert!(running);
        assert!(report.contains("build: unknown"));
        assert!(report.contains("mew daemon --stop"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_file_roundtrips_through_write_local_status() {
        let (dir, _pidfile, _) = tmp_files("roundtrip");
        let pidfile = dir.join("custom.pid");
        write_local_status(&pidfile, Some("/tmp/custom.sock"), None).unwrap();
        let meta = read_status(&status_path_for(&pidfile)).expect("status file readable");
        assert_eq!(meta.version, crate::version::git_rev());
        assert_eq!(meta.pid, std::process::id() as i32);
        assert_eq!(meta.socket.as_deref(), Some("/tmp/custom.sock"));
        assert!(meta.port.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
