use super::*;

fn set_app_menus(cx: &mut App) {
    cx.set_menus([
        Menu::new("mew").items([MenuItem::action("Quit", Quit)]),
        Menu::new("File").items([
            MenuItem::action("New Conversation", NewConversation),
            MenuItem::action("Close Conversation", CloseConversation),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sessions Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Terminal", ToggleTerminal),
            MenuItem::action("Toggle Workbench", ToggleWorkbench),
        ]),
    ]);
    cx.on_action(quit);
}

fn quit(_: &Quit, cx: &mut App) {
    cx.quit();
}

fn supervisor_config() -> Result<SupervisorConfig> {
    let mut config = SupervisorConfig::from_env()?;
    if config.daemon_binary.is_none() {
        config.daemon_binary = packaged_daemon_binary().or_else(|| Some(PathBuf::from("mew")));
    }
    Ok(config)
}

fn packaged_daemon_binary() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|directory| directory.join("mew")))
        .filter(|binary| binary.is_file())
}

fn persisted_window_bounds() -> Option<Bounds<Pixels>> {
    let state = mew_config::load_state().ok()?;
    let frame = state.desktop_window?;
    if !frame.x.is_finite()
        || !frame.y.is_finite()
        || !frame.width.is_finite()
        || !frame.height.is_finite()
        || frame.width < 720.
        || frame.height < 480.
    {
        return None;
    }
    Some(Bounds::new(
        point(px(frame.x), px(frame.y)),
        gpui::size(px(frame.width), px(frame.height)),
    ))
}

#[cfg(target_os = "macos")]
fn configure_native_cef_environment() {
    if std::env::var_os("MEW_CEF_FRAMEWORK_PATH").is_none() {
        if let Some(framework) = find_native_cef_framework() {
            std::env::set_var("MEW_CEF_FRAMEWORK_PATH", framework);
        }
    }
    if std::env::var_os("MEW_CEF_HELPER_PATH").is_none() {
        if let Some(helper) = std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(PathBuf::from))
            .map(|directory| directory.join("mew-cef-host-helper"))
            .filter(|helper| helper.is_file())
        {
            std::env::set_var("MEW_CEF_HELPER_PATH", helper);
        }
    }
}

#[cfg(target_os = "macos")]
fn find_native_cef_framework() -> Option<PathBuf> {
    const FRAMEWORK: &str = "Chromium Embedded Framework.framework";
    const BINARY: &str = "Chromium Embedded Framework";

    // Prefer a sibling app bundle when one exists. CEF resolves macOS
    // graphics libraries relative to the app bundle, so loading the framework
    // from the exported distribution directly leaves Chromium looking beside
    // `target/<profile>/mew-desktop` for libGLESv2.dylib.
    if let Ok(executable) = std::env::current_exe() {
        if let Some(profile_dir) = executable.parent() {
            for framework in [
                profile_dir.join(
                    "../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
                ),
                profile_dir.join(
                    "bundle/macos/mew.app/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
                ),
                profile_dir.join(
                    "mew.app/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
                ),
            ] {
                if framework.is_file() {
                    return Some(framework);
                }
            }
        }
    }

    let mut roots = Vec::new();
    for variable in ["MEW_CEF_FRAMEWORK_SOURCE", "CEF_PATH"] {
        if let Some(path) = std::env::var_os(variable) {
            roots.push(PathBuf::from(path));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/cef"));
    }
    if let Some(framework) = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(PathBuf::from))
        .and_then(|macos| macos.parent().map(PathBuf::from))
        .map(|contents| contents.join(format!("Frameworks/{FRAMEWORK}/{BINARY}")))
        .filter(|framework| framework.is_file())
    {
        return Some(framework);
    }

    let architecture = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        _ => return None,
    };
    for root in roots {
        let candidates = [
            root.join(BINARY),
            root.join(FRAMEWORK).join(BINARY),
            root.join(format!("cef_macos_{architecture}/{FRAMEWORK}/{BINARY}")),
        ];
        if let Some(framework) = candidates.into_iter().find(|path| path.is_file()) {
            return Some(framework);
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let version_root = entry.path();
            let candidate =
                version_root.join(format!("cef_macos_{architecture}/{FRAMEWORK}/{BINARY}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn configure_native_cef_environment() {}

fn remote_connection_profile_from_env() -> Option<DesktopConnectionProfile> {
    let (node_id, persisted_device_name) =
        if let Ok(node_id) = std::env::var("MEW_DESKTOP_IROH_NODE_ID") {
            (node_id, None)
        } else {
            let state = mew_config::load_state().ok()?;
            let active = state.desktop_active_remote_profile?;
            let profile = state
                .desktop_remote_profiles
                .into_iter()
                .find(|profile| profile.node_id == active)?;
            (profile.node_id, Some(profile.device_name))
        };
    Some(DesktopConnectionProfile::RemoteIroh {
        node_id,
        pairing_token: std::env::var("MEW_DESKTOP_IROH_TOKEN").ok(),
        device_name: std::env::var("MEW_DESKTOP_IROH_DEVICE_NAME")
            .ok()
            .or(persisted_device_name)
            .unwrap_or_else(|| "mew desktop".to_owned()),
    })
}

pub(crate) fn run() {
    configure_native_cef_environment();
    let remote_profile = remote_connection_profile_from_env();
    let startup = if remote_profile.is_some() {
        None
    } else {
        Some(match supervisor_config() {
            Ok(config) => {
                let mut supervisor = DesktopSupervisor::new(config);
                match supervisor.connect_or_launch() {
                    Ok(endpoint) => Ok((endpoint, supervisor)),
                    Err(error) => Err((error.to_string(), supervisor)),
                }
            }
            Err(error) => Err((
                error.to_string(),
                DesktopSupervisor::new(SupervisorConfig::default()),
            )),
        })
    };

    application()
        .with_assets(IconAssets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            set_app_menus(cx);
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-n", NewConversation, None),
                gpui::KeyBinding::new("cmd-w", CloseConversation, None),
                gpui::KeyBinding::new("cmd-b", ToggleSidebar, None),
                gpui::KeyBinding::new("cmd-j", ToggleTerminal, None),
                gpui::KeyBinding::new("escape", DismissPopovers, None),
                gpui::KeyBinding::new("backspace", ComposerBackspace, Some("Composer")),
                gpui::KeyBinding::new("delete", ComposerDelete, Some("Composer")),
                gpui::KeyBinding::new("left", ComposerLeft, Some("Composer")),
                gpui::KeyBinding::new("right", ComposerRight, Some("Composer")),
                gpui::KeyBinding::new("shift-left", ComposerSelectLeft, Some("Composer")),
                gpui::KeyBinding::new("shift-right", ComposerSelectRight, Some("Composer")),
                gpui::KeyBinding::new("cmd-a", ComposerSelectAll, Some("Composer")),
                gpui::KeyBinding::new("cmd-v", ComposerPaste, Some("Composer")),
                gpui::KeyBinding::new("home", ComposerHome, Some("Composer")),
                gpui::KeyBinding::new("end", ComposerEnd, Some("Composer")),
            ]);
            let (endpoint, startup_error, supervisor) = match startup {
                Some(Ok((endpoint, supervisor))) => (Some(endpoint), None, Some(supervisor)),
                Some(Err((error, supervisor))) => (None, Some(error), Some(supervisor)),
                None => (None, None, None),
            };
            let bounds = persisted_window_bounds().unwrap_or_else(|| {
                Bounds::centered(None, gpui::size(gpui::px(1240.), gpui::px(760.)), cx)
            });
            if let Err(error) = cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: None,
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(12.), px(10.))),
                    }),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |window, cx| {
                    cx.new(|cx| {
                        DesktopShell::new(
                            endpoint,
                            supervisor,
                            startup_error,
                            remote_profile,
                            window,
                            cx,
                        )
                    })
                },
            ) {
                tracing::error!(%error, "could not open mew desktop window");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
