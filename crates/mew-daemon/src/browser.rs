//! Browser wire responses use the same explicit page runner as agent tools.

use mew_protocol::{ClientMessage, ServerMessage};
use mew_tools::browser::{BrowserAccess, BrowserAction};
use tokio_util::sync::CancellationToken;

pub fn respond(
    access: &BrowserAccess,
    request: ClientMessage,
    cancel: &CancellationToken,
) -> impl std::future::Future<Output = ServerMessage> + Send + 'static {
    let parsed = match request {
        ClientMessage::BrowserOpen { url, tab_id } => Ok((BrowserAction::Open(url), tab_id)),
        ClientMessage::BrowserSnapshot { tab_id } => Ok((BrowserAction::Snapshot, tab_id)),
        ClientMessage::BrowserScreenshot { annotate, tab_id } => {
            Ok((BrowserAction::Screenshot { annotate }, tab_id))
        }
        ClientMessage::BrowserClick { selector, tab_id } => {
            Ok((BrowserAction::Click(selector), tab_id))
        }
        ClientMessage::BrowserFill {
            selector,
            text,
            tab_id,
        } => Ok((BrowserAction::Fill { selector, text }, tab_id)),
        ClientMessage::BrowserPress { key, tab_id } => Ok((BrowserAction::Press(key), tab_id)),
        ClientMessage::BrowserClose { tab_id } => Ok((BrowserAction::Close, tab_id)),
        _ => Err("unknown browser action"),
    };
    let access = access.clone();
    let cancel = cancel.clone();
    let parsed = parsed.map(|(action, tab_id)| (action, tab_id.or_else(|| access.page_instance())));
    async move {
        let (action, tab_id) = match parsed {
            Ok(parsed) => parsed,
            Err(message) => {
                return ServerMessage::BrowserError {
                    message: message.into(),
                    tab_id: None,
                }
            }
        };
        if tab_id.is_none() {
            return ServerMessage::BrowserError {
                message:
                    "in-app browser is unavailable; open this conversation in the mew desktop app"
                        .into(),
                tab_id: None,
            };
        }
        let snapshot = matches!(action, BrowserAction::Snapshot);
        let screenshot = matches!(action, BrowserAction::Screenshot { .. });
        let close = matches!(action, BrowserAction::Close);
        match access.execute(action, tab_id.as_deref(), &cancel).await {
            Err(message) => ServerMessage::BrowserError { message, tab_id },
            Ok(output) if snapshot => ServerMessage::BrowserSnapshot {
                snapshot: output.output,
                url: output.url,
                title: output.title,
                tab_id,
            },
            Ok(output) if screenshot => ServerMessage::BrowserScreenshot {
                data: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    output
                        .screenshot
                        .expect("screenshot action returns image bytes"),
                ),
                url: output.url,
                tab_id,
            },
            Ok(output) => ServerMessage::BrowserState {
                open: !close,
                url: (!close).then_some(output.url),
                title: (!close).then_some(output.title),
                tab_id,
            },
        }
    }
}
