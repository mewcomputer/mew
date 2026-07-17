//! Small adapter around the native `agent-browser` CLI.
//!
//! The daemon owns the browser session. Keeping this behind a narrow adapter
//! lets the UI use semantic snapshots now while leaving room for a native
//! WKWebView renderer later.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use tokio::process::Command;

fn session_args(session_id: &str) -> Vec<String> {
    vec!["--session".into(), format!("mew-{session_id}")]
}

fn command_args(session_id: &str, args: &[&str], cdp_port: Option<&str>) -> Vec<String> {
    let mut command_args = Vec::with_capacity(args.len() + 4);
    if let Some(port) = cdp_port {
        // CEF owns the browser process in desktop mode. agent-browser's
        // persistent session daemon cannot be combined with an external CDP
        // target, and would otherwise control a second browser.
        command_args.extend(["--cdp".to_owned(), port.to_owned()]);
    } else {
        command_args.extend(session_args(session_id));
    }
    command_args.extend(args.iter().map(|arg| (*arg).to_owned()));
    command_args
}

async fn run(session_id: &str, args: &[&str]) -> Result<String> {
    let cdp_port = std::env::var("MEW_BROWSER_CDP_PORT").ok();
    let output = Command::new("agent-browser")
        .args(command_args(session_id, args, cdp_port.as_deref()))
        .output()
        .await
        .context("run agent-browser; install it with `brew install agent-browser`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("agent-browser {}: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn open(session_id: &str, url: &str) -> Result<(String, String, String)> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("browser navigation only supports http and https URLs");
    }
    run(session_id, &["open", url]).await?;
    state(session_id)
        .await
        .map(|(url, title)| (url, title, String::new()))
}

pub async fn snapshot(session_id: &str) -> Result<(String, String, String)> {
    let snapshot = run(session_id, &["snapshot", "--json"]).await?;
    let (url, title) = state(session_id).await?;
    Ok((snapshot, url, title))
}

pub async fn click(session_id: &str, selector: &str) -> Result<(String, String, String)> {
    run(session_id, &["click", selector]).await?;
    state(session_id)
        .await
        .map(|(url, title)| (url, title, String::new()))
}

pub async fn fill(
    session_id: &str,
    selector: &str,
    text: &str,
) -> Result<(String, String, String)> {
    run(session_id, &["fill", selector, text]).await?;
    state(session_id)
        .await
        .map(|(url, title)| (url, title, String::new()))
}

pub async fn press(session_id: &str, key: &str) -> Result<(String, String, String)> {
    run(session_id, &["press", key]).await?;
    state(session_id)
        .await
        .map(|(url, title)| (url, title, String::new()))
}

pub async fn screenshot(session_id: &str, annotate: bool) -> Result<(String, String)> {
    let path: PathBuf = std::env::temp_dir().join(format!("mew-browser-{}.png", ulid::Ulid::new()));
    let path_string = path.to_string_lossy().to_string();
    let args = if annotate {
        vec!["screenshot", &path_string, "--annotate"]
    } else {
        vec!["screenshot", &path_string]
    };
    run(session_id, &args).await?;
    let bytes = tokio::fs::read(&path)
        .await
        .context("read browser screenshot")?;
    let _ = tokio::fs::remove_file(&path).await;
    let (url, _) = state(session_id).await?;
    Ok((
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
        url,
    ))
}

pub async fn close(session_id: &str) -> Result<()> {
    run(session_id, &["close"]).await.map(|_| ())
}

async fn state(session_id: &str) -> Result<(String, String)> {
    let url = run(session_id, &["get", "url"]).await.unwrap_or_default();
    let title = run(session_id, &["get", "title"]).await.unwrap_or_default();
    Ok((url, title))
}

#[cfg(test)]
mod tests {
    use super::command_args;

    #[test]
    fn cdp_commands_use_the_native_browser_before_the_session() {
        assert_eq!(
            command_args("session-1", &["snapshot", "--json"], Some("9223")),
            vec![
                "--cdp".to_owned(),
                "9223".to_owned(),
                "snapshot".to_owned(),
                "--json".to_owned()
            ]
        );
    }

    #[test]
    fn standalone_commands_keep_the_session_only() {
        assert_eq!(
            command_args("session-1", &["close"], None),
            vec![
                "--session".to_owned(),
                "mew-session-1".to_owned(),
                "close".to_owned(),
            ]
        );
    }
}
