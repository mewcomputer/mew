use chrono::Utc;
use tokio::sync::mpsc;

use mew_message::{
    AssistantMeta, Message, Part, PartId, Role, Time, Tokens,
};
use mew_provider::ProviderEvent;

use crate::agent::Agent;
use crate::AgentEvent;

impl Agent {
    pub(crate) async fn handle_provider_event(
        &self,
        ev: &ProviderEvent,
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
    ) {
        match ev {
            ProviderEvent::PartStart { part } => {
                if assistant_msg.is_none() {
                    *assistant_msg = Some(self.start_assistant_message());
                }
                if let Some(ref mut msg) = assistant_msg {
                    msg.parts.push(part.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::PartStart {
                        part: part.clone(),
                    }))
                    .await;
            }
            ProviderEvent::PartDelta {
                part_id,
                field,
                delta,
            } => {
                if let Some(ref mut msg) = assistant_msg {
                    self.apply_delta(msg, *part_id, field, delta);
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::PartDelta {
                        part_id: *part_id,
                        field,
                        delta: delta.clone(),
                    }))
                    .await;
            }
            ProviderEvent::PartEnd { part_id } => {
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::PartEnd {
                        part_id: *part_id,
                    }))
                    .await;
            }
            ProviderEvent::MessageEnd {
                finish,
                usage,
                cost: _,
            } => {
                if let Some(ref mut msg) = assistant_msg {
                    let now = Utc::now().timestamp_millis();
                    msg.time.completed = Some(now);
                    let computed_cost = usage.input as f64 / 1_000_000.0 * self.input_price
                        + usage.output as f64 / 1_000_000.0 * self.output_price
                        + usage.reasoning as f64 / 1_000_000.0 * self.reasoning_price
                        + usage.cache_read as f64 / 1_000_000.0 * self.cache_read_price
                        + usage.cache_write as f64 / 1_000_000.0 * self.cache_write_price;
                    if let Some(ref mut meta) = msg.assistant {
                        meta.finish = Some(*finish);
                        meta.tokens = *usage;
                        meta.cost = computed_cost;
                    }
                    self.append_message(msg.clone()).await;
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::MessageEnd {
                        finish: *finish,
                        usage: *usage,
                        cost: 0.0,
                    }))
                    .await;
            }
            ProviderEvent::RetryWait {
                attempt,
                max_attempts,
                delay_secs,
                reason,
            } => {
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::RetryWait {
                        attempt: *attempt,
                        max_attempts: *max_attempts,
                        delay_secs: *delay_secs,
                        reason: reason.clone(),
                    }))
                    .await;
            }
            ProviderEvent::Error(err) => {
                if let Some(ref mut msg) = assistant_msg {
                    let now = Utc::now().timestamp_millis();
                    msg.time.completed = Some(now);
                    if let Some(ref mut meta) = msg.assistant {
                        meta.error = Some(err.clone());
                    }
                    self.append_message(msg.clone()).await;
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::Error(err.clone())))
                    .await;
                let _ = ev_tx
                    .send(AgentEvent::Error(err.message.clone()))
                    .await;
            }
        }
    }

    pub fn apply_delta(&self, msg: &mut Message, part_id: PartId, field: &str, delta: &str) {
        for part in &mut msg.parts {
            if part.id() != part_id {
                continue;
            }
            match part {
                Part::Text(ref mut p) => {
                    if field == "text" || field.is_empty() {
                        p.text.push_str(delta);
                    }
                }
                Part::Reasoning(ref mut p) => {
                    if field == "text" || field.is_empty() {
                        p.text.push_str(delta);
                    } else if field == "signature" {
                        p.signature = Some(delta.to_string());
                    }
                }
                Part::ToolCall(ref mut p) => {
                    match field {
                        "arguments" => p.raw_input.push_str(delta),
                        "call_id" => p.call_id.push_str(delta),
                        "tool_name" => p.tool_name.push_str(delta),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) async fn append_message(&self, msg: Message) {
        let mut messages = self.messages.lock().await;
        messages.push(msg.clone());
        drop(messages);

        if let Some(session) = &self.session {
            let mut session = session.lock().await;
            if let Err(e) = session.write_message(&msg).await {
                tracing::error!("session write failed: {}", e);
            }
        }
    }

    pub(crate) fn start_assistant_message(&self) -> Message {
        Message {
            id: ulid::Ulid::new(),
            session_id: self.session_id,
            role: Role::Assistant,
            parts: Vec::new(),
            time: Time {
                created: Utc::now().timestamp_millis(),
                completed: None,
            },
            assistant: Some(AssistantMeta {
                provider_id: String::new(),
                model_id: String::new(),
                cost: 0.0,
                tokens: Tokens::default(),
                finish: None,
                error: None,
            }),
        }
    }
}
