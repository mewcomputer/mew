//! Daemon-related command functions.
//!
//! Extracted from `main.rs` as pure code motion. These build and run the
//! daemon server, handle daemonization / process control, and the iroh
//! pairing flow.

use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::info;

use mew_agent::Agent;
use mew_message::SessionId;

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
            let agent = crate::setup::agent::build_session_agent(
                &cfg,
                cat.as_ref(),
                &provider_id,
                &model_id,
                raw,
                mode,
                Some(params.writer),
                session_id,
                Arc::new(mew_hooks::NopDispatcher),
                None,
                &[],
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
                .filter(|pid| crate::setup::providers::provider_has_credential(cfg, pid))
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
                            context_window: Some(m.context_window),
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
                let new_provider = crate::setup::providers::build_provider(
                    &cfg_switcher,
                    cat_ref,
                    provider_id,
                    model_id,
                    raw2,
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
                let config =
                    crate::setup::providers::resolve_reasoning(cat_ref, model_id, Some(variant));
                let resolved = config.is_some().then(|| variant.to_string());
                agent.set_reasoning(config);
                Ok(resolved)
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
pub(crate) async fn run_daemon(
    socket: Option<String>,
    port: Option<String>,
    fake_provider: bool,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    mode: mew_hooks::PermissionMode,
) -> Result<()> {
    let server = build_daemon_server(fake_provider, provider_flag, model_flag, raw, mode).await?;

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
) -> Result<()> {
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
        allowlist_path,
        secret_key,
    )
    .await
}

/// `mew pair` — print the daemon's iroh NodeId and enter pairing mode.
///
/// This starts an iroh endpoint with pairing enabled. The next peer that
/// connects will be added to the allowlist automatically.
#[cfg(feature = "iroh")]
pub(crate) async fn pair_cmd() -> Result<()> {
    use iroh::Endpoint;

    println!("Starting mew pairing mode...\n");

    // Load the daemon's persistent secret key so the NodeId shown here
    // matches the one the daemon will use when running with --iroh.
    let secret_key_path = mew_daemon::iroh_transport::default_secret_key_path();
    let secret_key = mew_daemon::iroh_transport::load_or_create_secret_key(&secret_key_path)?;

    let endpoint = Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret_key)
        .alpns(vec![mew_daemon::iroh_transport::MEW_ALPN.to_vec()])
        .bind()
        .await
        .context("bind iroh endpoint")?;

    info!("connecting to iroh relay servers...");
    tokio::time::timeout(std::time::Duration::from_secs(15), endpoint.online())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "iroh endpoint failed to come online within 15s — check network connectivity"
            )
        })?;

    let node_id = endpoint.id();
    let node_id_str = node_id.to_string();

    // Print pairing info with QR code
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  mew pairing mode                                       ║");
    println!("║                                                         ║");
    println!("║  Daemon NodeId:                                         ║");
    println!("║  {node_id_str}  ║");
    println!("║                                                         ║");
    println!("║  Scan the QR code below with your mobile client:        ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // Generate ASCII QR code containing the NodeId.
    // The payload uses a URL-scheme prefix so iOS camera/QR apps recognize it:
    // `computer.mew.mew://<node_id>`
    let payload = format!("computer.mew.mew://{node_id_str}");
    let qr = qrcode::QrCode::new(payload).context("generate QR code")?;
    let qr_string = qr
        .render::<qrcode::render::unicode::Dense1x2>()
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .build();
    println!("{qr_string}");

    eprintln!("\nTo connect manually, use this NodeId: {node_id_str}");
    eprintln!("Press Ctrl+C to cancel.\n");

    // Load or create allowlist
    let allowlist_path = mew_daemon::iroh_transport::default_allowlist_path();
    let allowlist = Arc::new(mew_daemon::iroh_transport::NodeIdAllowlist::load(
        allowlist_path.clone(),
    )?);
    let pairing_mode = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let allowlist_clone = allowlist.clone();
    let pairing_mode_clone = pairing_mode.clone();

    println!("Listening for connections...");

    // Simple accept loop for pairing. When a peer connects, their NodeId is
    // added to the allowlist and the pairing completes. Times out after 120s.
    let mut sig_term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;

    loop {
        tokio::select! {
            conn_result = endpoint.accept() => {
                let Some(conn) = conn_result else {
                    break;
                };
                let conn = conn.await.context("accept connection")?;
                let remote_id = conn.remote_id();
                let id_str = remote_id.to_string();

                println!("\n✓ Connection from: {id_str}");

                if pairing_mode_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    match allowlist_clone.add(&id_str) {
                        Ok(()) => {
                            println!("  Added to allowlist: {}", allowlist_path.display());
                            println!("  Pairing complete!");
                            pairing_mode_clone.store(false, std::sync::atomic::Ordering::Relaxed);
                            conn.close(0u32.into(), b"pairing complete");
                            break;
                        }
                        Err(e) => {
                            println!("  Failed to persist allowlist: {e}");
                            conn.close(1u32.into(), b"pairing failed");
                            break;
                        }
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                println!("\nPairing timed out after 120s. Run `mew pair` again to retry.");
                break;
            }
            _ = sig_term.recv() => {
                println!("\nPairing cancelled (SIGTERM).");
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nPairing cancelled.");
                break;
            }
        }
    }

    endpoint.close().await;
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
