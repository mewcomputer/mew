use chrono::Utc;
use tokio::sync::mpsc;

use mew_message::{AssistantMeta, Message, Part, PartId, Role, Time, Tokens};
use mew_provider::ProviderEvent;

use crate::agent::{sensitivity_label, Agent};
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
                // Stamp sensitivity from the tool registry onto tool-call
                // parts. Providers don't know the registry; the agent does.
                let part = if let Part::ToolCall(mut tc) = part.clone() {
                    if tc.sensitivity.is_none() {
                        tc.sensitivity = self
                            .tools
                            .get(&tc.tool_name)
                            .map(|t| sensitivity_label(t.sensitivity()).to_string());
                    }
                    Part::ToolCall(tc)
                } else {
                    part.clone()
                };
                if let Some(ref mut msg) = assistant_msg {
                    msg.parts.push(part.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::Provider(ProviderEvent::PartStart { part }))
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
                if let Some(ref mut msg) = assistant_msg {
                    self.reconcile_tool_call_input(msg, *part_id);
                }
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
                let _ = ev_tx.send(AgentEvent::Error(err.message.clone())).await;
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
                Part::ToolCall(ref mut p) => match field {
                    "arguments" => p.raw_input.push_str(delta),
                    "call_id" => p.call_id.push_str(delta),
                    "tool_name" => p.tool_name.push_str(delta),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    /// Materialize the streamed tool-call arguments into `state.input`.
    ///
    /// Both providers send `PartDelta { field: "arguments", ... }` deltas that
    /// the agent appends to `raw_input`. The provider-side accumulator parses
    /// them into a `Value`, but only on a local copy that is never re-emitted
    /// to the agent. Without this hook, `state.input` stays at `Value::Null`
    /// for the entire streaming window — and the JSONL session file, the
    /// downstream tool execution, and any consumer that reads `state.input()`
    /// (e.g. the subagent dispatcher) all see a null/empty input.
    pub fn reconcile_tool_call_input(&self, msg: &mut Message, part_id: PartId) {
        for part in &mut msg.parts {
            if part.id() != part_id {
                continue;
            }
            if let Part::ToolCall(ref mut p) = part {
                if p.raw_input.is_empty() {
                    return;
                }
                match serde_json::from_str(&p.raw_input) {
                    Ok(value) => p.state.set_input(value),
                    Err(e) => tracing::warn!(
                        part_id = %part_id,
                        raw_input = %p.raw_input,
                        error = %e,
                        "failed to parse streamed tool arguments",
                    ),
                }
                return;
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
