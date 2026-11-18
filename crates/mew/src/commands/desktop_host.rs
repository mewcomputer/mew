//! JSONL support process for desktop shells. The daemon and theme rules stay in Rust.

use anyhow::{Context, Result};
use mew_desktop_supervisor::{DesktopSupervisor, SupervisorConfig};
use mew_tui::theme::Theme;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

const DEFAULT_DAEMON_PORT: u16 = 25566;
const MAX_REQUEST_BYTES: usize = 16_384;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    daemon_url: String,
    websocket_url: String,
    theme_mode: ThemeMode,
    light_theme: String,
    dark_theme: String,
    themes: Vec<HostTheme>,
}

#[derive(Serialize)]
struct HostTheme {
    id: String,
    mode: &'static str,
    css: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ThemeMode {
    System,
    Light,
    Dark,
}

impl Bootstrap {
    fn load(daemon_url: String, websocket_url: String) -> Self {
        let state = mew_config::load_state().unwrap_or_default();
        Self {
            daemon_url,
            websocket_url,
            theme_mode: match state.desktop_theme_mode.as_str() {
                "light" => ThemeMode::Light,
                "dark" => ThemeMode::Dark,
                _ => ThemeMode::System,
            },
            light_theme: if state.desktop_light_theme.is_empty() {
                "codex-light".into()
            } else {
                state.desktop_light_theme
            },
            dark_theme: if state.desktop_dark_theme.is_empty() {
                "codex-dark".into()
            } else {
                state.desktop_dark_theme
            },
            themes: Theme::list_available()
                .into_iter()
                .map(|id| {
                    let theme = Theme::load(&id);
                    HostTheme {
                        id,
                        mode: if theme.mode == mew_tui::theme::ThemeMode::Light {
                            "light"
                        } else {
                            "dark"
                        },
                        css: theme.css_variables(),
                    }
                })
                .collect(),
        }
    }

    fn set_theme(
        &mut self,
        theme_mode: ThemeMode,
        light_theme: String,
        dark_theme: String,
    ) -> Result<()> {
        for id in [&light_theme, &dark_theme] {
            anyhow::ensure!(
                self.themes.iter().any(|theme| &theme.id == id),
                "unknown theme: {id}"
            );
        }
        let mut state = mew_config::load_state()?;
        state.desktop_theme_mode = match theme_mode {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
        .into();
        state.desktop_light_theme.clone_from(&light_theme);
        state.desktop_dark_theme.clone_from(&dark_theme);
        mew_config::save_state(&state)?;
        self.theme_mode = theme_mode;
        self.light_theme = light_theme;
        self.dark_theme = dark_theme;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    id: Value,
    request: HostRequest,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum HostRequest {
    Bootstrap {},
    SetTheme {
        theme_mode: ThemeMode,
        light_theme: String,
        dark_theme: String,
    },
}

#[derive(Serialize)]
#[serde(untagged)]
enum Reply<'a> {
    Bootstrap { id: Value, result: &'a Bootstrap },
    Saved { id: Value, result: Value },
    Error { id: Value, error: String },
}

fn parse(input: &[u8]) -> std::result::Result<Envelope, (Value, String)> {
    let value: Value =
        serde_json::from_slice(input).map_err(|error| (Value::Null, error.to_string()))?;
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    if !id.is_u64()
        && !id
            .as_str()
            .is_some_and(|id| !id.is_empty() && id.len() <= 128)
    {
        return Err((
            Value::Null,
            "request id must be an unsigned integer or a nonempty string".into(),
        ));
    }
    serde_json::from_value(value).map_err(|error| (id, error.to_string()))
}

async fn write(output: &mut (impl AsyncWrite + Unpin), value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await?;
    Ok(())
}

/// Drain an oversized line without retaining its bytes, preserving the next frame.
async fn read(
    input: &mut (impl AsyncBufRead + Unpin),
    bytes: &mut Vec<u8>,
) -> Result<Option<bool>> {
    bytes.clear();
    let mut oversized = false;
    loop {
        let available = input.fill_buf().await?;
        if available.is_empty() {
            return Ok((oversized || !bytes.is_empty()).then_some(oversized));
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let count = end.unwrap_or(available.len());
        if !oversized {
            if bytes.len() + count > MAX_REQUEST_BYTES {
                oversized = true;
                bytes.clear();
            } else {
                bytes.extend_from_slice(&available[..count]);
            }
        }
        input.consume(count + usize::from(end.is_some()));
        if end.is_some() {
            return Ok(Some(oversized));
        }
    }
}

async fn requests(mut bootstrap: Bootstrap, mut output: tokio::io::Stdout) -> Result<()> {
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut bytes = Vec::new();
    while let Some(oversized) = read(&mut input, &mut bytes).await? {
        let reply = if oversized {
            Reply::Error {
                id: Value::Null,
                error: "desktop request exceeds 16 KiB".into(),
            }
        } else {
            match parse(&bytes) {
                Err((id, error)) => Reply::Error { id, error },
                Ok(Envelope { id, request }) => match request {
                    HostRequest::Bootstrap {} => Reply::Bootstrap {
                        id,
                        result: &bootstrap,
                    },
                    HostRequest::SetTheme {
                        theme_mode,
                        light_theme,
                        dark_theme,
                    } => match bootstrap.set_theme(theme_mode, light_theme, dark_theme) {
                        Ok(()) => Reply::Saved {
                            id,
                            result: serde_json::json!({}),
                        },
                        Err(error) => Reply::Error {
                            id,
                            error: error.to_string(),
                        },
                    },
                },
            }
        };
        write(&mut output, &reply).await?;
    }
    Ok(())
}

pub async fn run() -> Result<()> {
    let mut config = SupervisorConfig::from_env()?;
    if config.local_port == 0 {
        config.local_port = DEFAULT_DAEMON_PORT;
    }
    if config.daemon_binary.is_none() {
        config.daemon_binary = Some(std::env::current_exe().context("resolve daemon executable")?);
    }
    let (supervisor, endpoint) = tokio::task::spawn_blocking(move || {
        let mut supervisor = DesktopSupervisor::new(config);
        let endpoint = supervisor.connect_or_launch()?;
        Ok::<_, anyhow::Error>((supervisor, endpoint))
    })
    .await??;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let app_url = format!("http://{}/", listener.local_addr()?);
    let relay = mew_web_bridge::DesktopRelay::bind(endpoint.websocket_url.clone(), "mew://app")?;
    let bootstrap = Bootstrap::load(endpoint.websocket_url, relay.url().to_owned());
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Ready<'a> {
        r#type: &'static str,
        app_url: &'a str,
        bootstrap: &'a Bootstrap,
    }
    let mut output = tokio::io::stdout();
    write(
        &mut output,
        &Ready {
            r#type: "ready",
            app_url: &app_url,
            bootstrap: &bootstrap,
        },
    )
    .await?;
    let result = tokio::select! {
        result = mew_web_bridge::serve_assets(listener) => result.context("desktop UI asset server stopped"),
        result = relay.run() => result.context("desktop UI WebSocket relay stopped"),
        result = requests(bootstrap, output) => result,
    };
    drop(supervisor);
    result
}
