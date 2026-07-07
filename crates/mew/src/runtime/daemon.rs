//! `DaemonTarget` — `CommandTarget` impl for daemon-connected TUI mode.
//!
//! Wraps a `DaemonClient` and forwards operations to the daemon over the
//! WebSocket protocol. Operations that aren't meaningful in daemon mode
//! (rewind, plugin commands) return `Unsupported`.

use std::sync::Arc;

use mew_agent::AgentEvent;
use mew_hooks::PermissionMode;
use mew_message::Part;
use mew_personas::Persona;
use tokio::sync::mpsc::Receiver;

use crate::runtime::target::{CommandTarget, PersonaApplied, SwitchedModel, Unsupported};

/// The daemon-connected command target. Wraps an `Arc<DaemonClient>` and
/// forwards operations over the WebSocket protocol.
pub struct DaemonTarget {
    pub client: Arc<mew_daemon::DaemonClient>,
}

impl DaemonTarget {
    pub fn new(client: Arc<mew_daemon::DaemonClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl CommandTarget for DaemonTarget {
    fn prompt(&mut self, enriched: String, _parts: Vec<Part>) -> Receiver<AgentEvent> {
        // Daemon client's prompt is async but returns a Receiver synchronously
        // via a spawn. We need to block, but since this is called from an async
        // context, we can't. Instead, we use the client's prompt method which
        // spawns internally.
        //
        // Actually, client.prompt() is async and returns Receiver<AgentEvent>.
        // We can't await in a non-async fn. Let's use a channel.
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut recv = client.prompt(enriched).await;
            // Forward events from recv to tx
            while let Some(event) = recv.recv().await {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        rx
    }

    async fn cancel(&mut self) {
        self.client.cancel().await;
    }

    async fn clear(&mut self) -> Result<(), Unsupported> {
        let client = self.client.clone();
        client.slash_command("/clear".into()).await;
        Ok(())
    }

    async fn compact(&mut self) -> Result<(), Unsupported> {
        let client = self.client.clone();
        client.slash_command("/compact".into()).await;
        Ok(())
    }

    async fn todos(&mut self) -> Result<String, Unsupported> {
        // Todos come via TodosUpdated events from the daemon, not as a
        // direct response. Return a placeholder — the app has the snapshot.
        Err(Unsupported("todos are pushed by the daemon as events"))
    }

    async fn switch_model(&mut self, spec: &str) -> Result<SwitchedModel, Unsupported> {
        let (provider_id, model_id) = if let Some(idx) = spec.find('/') {
            (spec[..idx].to_string(), spec[idx + 1..].to_string())
        } else {
            (String::new(), spec.to_string())
        };
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SwitchModel {
            provider: provider_id.clone(),
            model: model_id.clone(),
        };
        if let Ok(json) = mew_protocol::encode_json(&msg) {
            let _ = client.send_raw(&json).await;
        }
        Ok(SwitchedModel {
            provider_id,
            model_id,
            display: spec.to_string(),
        })
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), Unsupported> {
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SetPermissionMode {
            mode: mode.id().to_string(),
        };
        if let Ok(json) = mew_protocol::encode_json(&msg) {
            let _ = client.send_raw(&json).await;
        }
        Ok(())
    }

    async fn set_thinking(&mut self, variant: &str) -> Result<(), Unsupported> {
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SetThinkingVariant {
            variant: variant.to_string(),
        };
        if let Ok(json) = mew_protocol::encode_json(&msg) {
            let _ = client.send_raw(&json).await;
        }
        Ok(())
    }

    async fn attach_session(&mut self, id: &str) -> Result<(), Unsupported> {
        let client = self.client.clone();
        let _ = client.attach_session(id).await;
        client.list_sessions().await;
        Ok(())
    }

    async fn resume(&mut self, id: &str) -> Result<(), Unsupported> {
        // In daemon mode, resume is the same as attach.
        self.attach_session(id).await
    }

    async fn rewind(&mut self, _n: usize) -> Result<(), Unsupported> {
        Err(Unsupported("rewind not available in daemon mode"))
    }

    async fn switch_persona(
        &mut self,
        name: &str,
        _personas: &[Persona],
    ) -> Result<PersonaApplied, Unsupported> {
        let client = self.client.clone();
        let msg = mew_protocol::ClientMessage::SwitchPersona {
            name: name.to_string(),
        };
        if let Ok(json) = mew_protocol::encode_json(&msg) {
            let _ = client.send_raw(&json).await;
        }
        Ok(PersonaApplied {
            name: name.to_string(),
            pinned_model: None,
            display: format!("switched to persona: {}", name),
        })
    }

    async fn plugin_command(&mut self, _name: &str, _args: &str) -> Result<String, Unsupported> {
        Err(Unsupported("plugin commands not available in daemon mode"))
    }

    async fn cancel_subagent(&mut self, _task_id: &str) -> Result<bool, Unsupported> {
        Err(Unsupported(
            "subagent cancellation not available in daemon mode",
        ))
    }
}
