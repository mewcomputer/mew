use anyhow::Result;
use std::io::Write;
use std::sync::Arc;

use mew_message::{Part, PartId};

pub(super) struct RunMetrics {
    pub(super) started: std::time::Instant,
    pub(super) secrets: Arc<mew_tools::SecretSet>,
    text_parts: std::collections::HashSet<PartId>,
    tool_names: std::collections::HashMap<String, String>,
    seen_tool_calls: std::collections::HashSet<String>,
    tool_errors: std::collections::HashMap<String, String>,
    tool_output_sizes: std::collections::HashMap<String, usize>,
    responses: u64,
    tool_calls: u64,
    failed_tool_calls: u64,
    nested_tool_calls: u64,
    failed_nested_tool_calls: u64,
    max_tool_output_bytes: usize,
    max_nested_output_bytes: usize,
    usage: [u64; 5],
    cost: f64,
}

impl RunMetrics {
    pub(super) fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            secrets: Arc::new(mew_tools::SecretSet::default()),
            text_parts: Default::default(),
            tool_names: Default::default(),
            seen_tool_calls: Default::default(),
            tool_errors: Default::default(),
            tool_output_sizes: Default::default(),
            responses: 0,
            tool_calls: 0,
            failed_tool_calls: 0,
            nested_tool_calls: 0,
            failed_nested_tool_calls: 0,
            max_tool_output_bytes: 0,
            max_nested_output_bytes: 0,
            usage: [0; 5],
            cost: 0.0,
        }
    }

    fn redact(&self, text: &str) -> String {
        let (text, count) = mew_tools::secrets::redact_secret_words(text, &self.secrets);
        mew_tools::secrets::annotate_redaction(text, count)
    }

    pub(super) fn observe(&mut self, event: &mew_agent::AgentEvent) -> Option<serde_json::Value> {
        use mew_agent::AgentEvent;
        use mew_provider::ProviderEvent;
        match event {
            AgentEvent::Provider(ProviderEvent::PartStart {
                part: Part::Text(part),
            }) => {
                self.text_parts.insert(part.base.id);
                None
            }
            AgentEvent::Provider(ProviderEvent::PartDelta { part_id, delta, .. })
                if self.text_parts.contains(part_id) =>
            {
                Some(serde_json::json!({"type":"text_delta", "text":delta}))
            }
            AgentEvent::Provider(ProviderEvent::PartEnd { part_id }) => {
                self.text_parts.remove(part_id);
                None
            }
            AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish,
                usage,
                cost,
            }) => {
                self.responses += 1;
                for (total, value) in self.usage.iter_mut().zip([
                    usage.input,
                    usage.output,
                    usage.reasoning,
                    usage.cache_read,
                    usage.cache_write,
                ]) {
                    *total += u64::from(value);
                }
                self.cost += cost;
                Some(
                    serde_json::json!({"type":"model_response", "finish":finish, "usage":usage, "cost":cost}),
                )
            }
            AgentEvent::Provider(ProviderEvent::RetryWait {
                attempt,
                max_attempts,
                delay_secs,
                reason,
            }) => Some(
                serde_json::json!({"type":"retry", "attempt":attempt, "max_attempts":max_attempts, "delay_secs":delay_secs, "reason":self.redact(reason)}),
            ),
            AgentEvent::Provider(ProviderEvent::Error(error)) => {
                let mut error = error.clone();
                error.message = self.redact(&error.message);
                Some(serde_json::json!({"type":"provider_error", "error":error}))
            }
            AgentEvent::PartUpdated {
                part: Part::ToolCall(call),
                ..
            } => {
                self.tool_names
                    .insert(call.call_id.clone(), call.tool_name.clone());
                if let mew_message::ToolState::Error(error) = &call.state {
                    self.tool_errors
                        .insert(call.call_id.clone(), self.redact(&error.error));
                }
                if let mew_message::ToolState::Completed(result) = &call.state {
                    self.tool_output_sizes
                        .insert(call.call_id.clone(), result.output.len());
                }
                None
            }
            AgentEvent::ToolStart { call_id } => {
                if self.seen_tool_calls.insert(call_id.clone()) {
                    self.tool_calls += 1;
                }
                Some(
                    serde_json::json!({"type":"tool_start", "call_id":call_id, "tool_name":self.tool_names.get(call_id)}),
                )
            }
            AgentEvent::ToolEnd { call_id, success } => {
                if self.seen_tool_calls.insert(call_id.clone()) {
                    self.tool_calls += 1;
                }
                if !success {
                    self.failed_tool_calls += 1;
                }
                let output_bytes = self.tool_output_sizes.remove(call_id);
                if let Some(size) = output_bytes {
                    self.max_tool_output_bytes = self.max_tool_output_bytes.max(size);
                }
                Some(
                    serde_json::json!({"type":"tool_end", "call_id":call_id, "tool_name":self.tool_names.get(call_id), "success":success, "error":self.tool_errors.remove(call_id), "output_bytes":output_bytes}),
                )
            }
            AgentEvent::NestedToolStart {
                parent_call_id,
                call_id,
                tool_name,
                ..
            } => {
                self.nested_tool_calls += 1;
                Some(
                    serde_json::json!({"type":"nested_tool_start", "parent_call_id":parent_call_id, "call_id":call_id, "tool_name":tool_name}),
                )
            }
            AgentEvent::NestedToolEnd {
                parent_call_id,
                call_id,
                success,
                error,
                output_bytes,
            } => {
                if !success {
                    self.failed_nested_tool_calls += 1;
                }
                if let Some(size) = output_bytes {
                    self.max_nested_output_bytes = self.max_nested_output_bytes.max(*size);
                }
                Some(
                    serde_json::json!({"type":"nested_tool_end", "parent_call_id":parent_call_id, "call_id":call_id, "success":success, "error":error, "output_bytes":output_bytes}),
                )
            }
            AgentEvent::PermissionRequest { call, .. } => Some(serde_json::json!({
                "type":"permission", "call_id":call.call_id, "tool_name":call.tool_name, "decision":"allow_once"
            })),
            AgentEvent::SubagentPermissionRequest {
                parent_call_id,
                call,
                ..
            } => Some(serde_json::json!({
                "type":"permission", "parent_call_id":parent_call_id, "call_id":call.call_id, "tool_name":call.tool_name, "decision":"allow_once"
            })),
            AgentEvent::WorkspacePermissionRequest { path, .. } => Some(serde_json::json!({
                "type":"workspace_permission", "path":self.redact(&path.to_string_lossy()), "decision":"allow_once"
            })),
            AgentEvent::AskUser { call_id, .. } | AgentEvent::SubagentAskUser { call_id, .. } => {
                Some(serde_json::json!({
                    "type":"question", "call_id":call_id, "status":"cancelled"
                }))
            }
            AgentEvent::GoalProposed { call_id, .. } => Some(serde_json::json!({
                "type":"goal", "call_id":call_id, "status":"cancelled"
            })),
            AgentEvent::SubagentStart {
                parent_call_id,
                name,
                child_session_id,
                display_name,
            } => Some(serde_json::json!({
                "type":"subagent_start", "parent_call_id":parent_call_id, "name":name, "child_session_id":child_session_id, "display_name":display_name
            })),
            AgentEvent::SubagentEnd {
                parent_call_id,
                child_session_id,
                outcome,
                ..
            } => {
                let (status, error) = match outcome {
                    mew_subagents::SubagentOutcome::Completed => ("completed", None),
                    mew_subagents::SubagentOutcome::Cancelled => ("cancelled", None),
                    mew_subagents::SubagentOutcome::Failed { reason } => {
                        ("failed", Some(self.redact(reason)))
                    }
                };
                Some(
                    serde_json::json!({"type":"subagent_end", "parent_call_id":parent_call_id, "child_session_id":child_session_id, "status":status, "error":error}),
                )
            }
            AgentEvent::Error(error) => {
                Some(serde_json::json!({"type":"error", "message":self.redact(error)}))
            }
            _ => None,
        }
    }

    pub(super) fn summary(&self, status: &str, error: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "type":"run_finished", "status":status, "error":error.map(|error| self.redact(error)),
            "elapsed_ms":self.started.elapsed().as_millis(),
            "responses":self.responses, "tool_calls":self.tool_calls,
            "failed_tool_calls":self.failed_tool_calls,
            "nested_tool_calls":self.nested_tool_calls,
            "failed_nested_tool_calls":self.failed_nested_tool_calls,
            "max_tool_output_bytes":self.max_tool_output_bytes, "max_nested_output_bytes":self.max_nested_output_bytes,
            "usage":{"input":self.usage[0], "output":self.usage[1], "reasoning":self.usage[2], "cache_read":self.usage[3], "cache_write":self.usage[4]},
            "cost":self.cost,
        })
    }
}

pub(super) fn emit_jsonl(value: &serde_json::Value) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer(&mut out, value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod structured_tests {
    use super::*;
    use mew_agent::AgentEvent;
    use mew_message::{Finish, Tokens};
    use mew_provider::ProviderEvent;

    #[test]
    fn structured_run_counts_rejected_calls_without_a_start_event() {
        let mut metrics = RunMetrics::new();
        metrics.observe(&AgentEvent::ToolEnd {
            call_id: "rejected".into(),
            success: false,
        });
        let summary = metrics.summary("completed", None);
        assert_eq!(summary["tool_calls"], 1);
        assert_eq!(summary["failed_tool_calls"], 1);
    }

    #[test]
    fn structured_output_reports_tool_output_size_without_the_content() {
        let mut metrics = RunMetrics::new();
        let id = ulid::Ulid::new();
        let part = Part::ToolCall(mew_message::ToolCallPart {
            base: mew_message::PartBase {
                id,
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            tool_name: "python".into(),
            call_id: "call".into(),
            raw_input: String::new(),
            state: mew_message::ToolState::Completed(mew_message::ToolStateCompleted {
                input: serde_json::json!({}),
                output: "café".into(),
                metadata: None,
                diff: None,
                images: vec![],
                time: mew_message::ToolTime {
                    start: 0,
                    end: Some(1),
                },
            }),
        });
        metrics.observe(&AgentEvent::PartUpdated { part_id: id, part });
        let event = metrics
            .observe(&AgentEvent::ToolEnd {
                call_id: "call".into(),
                success: true,
            })
            .unwrap();
        assert_eq!(event["output_bytes"], 5);
        assert!(!event.to_string().contains("café"));
        assert_eq!(
            metrics.summary("completed", None)["max_tool_output_bytes"],
            5
        );
    }

    #[test]
    fn structured_tool_errors_redact_configured_secret_words() {
        let mut metrics = RunMetrics::new();
        metrics.secrets = Arc::new(mew_tools::SecretSet {
            words: vec!["synthetic-secret".into()],
            globs: vec![],
        });
        let id = ulid::Ulid::new();
        let part = Part::ToolCall(mew_message::ToolCallPart {
            base: mew_message::PartBase {
                id,
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            tool_name: "read".into(),
            call_id: "call".into(),
            raw_input: String::new(),
            state: mew_message::ToolState::Error(mew_message::ToolStateError {
                input: serde_json::json!({}),
                error: "synthetic-secret missing".into(),
                time: mew_message::ToolTime {
                    start: 0,
                    end: Some(1),
                },
            }),
        });
        metrics.observe(&AgentEvent::PartUpdated { part_id: id, part });
        let event = metrics
            .observe(&AgentEvent::ToolEnd {
                call_id: "call".into(),
                success: false,
            })
            .unwrap();
        assert!(!event.to_string().contains("synthetic-secret"));
        assert!(event["error"].as_str().unwrap().contains("[REDACTED]"));
    }

    #[test]
    fn structured_output_records_subagent_lifecycle_without_counting_child_usage() {
        let mut metrics = RunMetrics::new();
        let start = metrics
            .observe(&AgentEvent::SubagentStart {
                parent_call_id: "parent".into(),
                name: "researcher".into(),
                child_session_id: "child".into(),
                display_name: None,
            })
            .unwrap();
        assert_eq!(start["type"], "subagent_start");
        let end = metrics
            .observe(&AgentEvent::SubagentEnd {
                parent_call_id: "parent".into(),
                child_session_id: "child".into(),
                outcome: mew_subagents::SubagentOutcome::Failed {
                    reason: "failed".into(),
                },
                manifests: vec![],
            })
            .unwrap();
        assert_eq!(end["status"], "failed");
        assert_eq!(metrics.summary("completed", None)["responses"], 0);
    }

    #[test]
    fn structured_run_counts_responses_and_nested_tool_failures() {
        let mut metrics = RunMetrics::new();
        let event = metrics
            .observe(&AgentEvent::Provider(ProviderEvent::MessageEnd {
                finish: Finish::ToolUse,
                usage: Tokens {
                    input: 20,
                    output: 3,
                    reasoning: 1,
                    cache_read: 5,
                    cache_write: 0,
                },
                cost: 0.01,
            }))
            .unwrap();
        assert_eq!(event["type"], "model_response");
        assert_eq!(event["usage"]["input"], 20);
        metrics
            .observe(&AgentEvent::NestedToolStart {
                parent_call_id: "outer".into(),
                call_id: "nested".into(),
                tool_name: "mcp__files__read".into(),
                input: serde_json::json!({}),
            })
            .unwrap();
        let end = metrics
            .observe(&AgentEvent::NestedToolEnd {
                parent_call_id: "outer".into(),
                call_id: "nested".into(),
                success: false,
                error: Some("missing file".into()),
                output_bytes: None,
            })
            .unwrap();
        assert_eq!(end["success"], false);
        assert_eq!(end["error"], "missing file");
        let summary = metrics.summary("completed", None);
        assert_eq!(summary["responses"], 1);
        assert_eq!(summary["nested_tool_calls"], 1);
        assert_eq!(summary["failed_nested_tool_calls"], 1);
        assert_eq!(summary["usage"]["input"], 20);
        assert_eq!(summary["usage"]["cache_read"], 5);
    }
}
