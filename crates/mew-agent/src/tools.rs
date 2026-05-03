use chrono::Utc;
use tokio::sync::{mpsc, oneshot};

use mew_hooks::{
    PermissionDecision, ToolCall as HookToolCall, ToolOutput,
};
use mew_message::{
    Message, Part, PartBase, PartId,
    ToolCallPart, ToolResultPart, ToolState, ToolStateCompleted, ToolStateError,
    ToolStateRunning, ToolTime,
};
use mew_tools::{Sensitivity, ToolCtx, ToolProgress};

use crate::agent::{Agent, ToolInput};
use crate::AgentEvent;

impl Agent {
    pub(crate) fn pending_tool_calls(&self, msg: &Message) -> Vec<ToolCallPart> {
        msg.parts
            .iter()
            .filter_map(|p| {
                if let Part::ToolCall(tc) = p {
                    match &tc.state {
                        ToolState::Pending(_) | ToolState::Running(_) => Some(tc.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    pub(crate) fn update_tool_call(&self, msg: &mut Message, part_id: PartId, state: ToolState) {
        for part in &mut msg.parts {
            if let Part::ToolCall(ref mut tc) = part {
                if tc.base.id == part_id {
                    tc.state = state;
                    return;
                }
            }
        }
    }

    pub(crate) async fn execute_pending_tool_calls(
        &self,
        pending: &[ToolCallPart],
        assistant_msg: &mut Option<Message>,
        ev_tx: &mpsc::Sender<AgentEvent>,
    ) -> Vec<Part> {
        let mut result_parts: Vec<Part> = Vec::with_capacity(pending.len());
        for tc in pending {
            let call_id = tc.call_id.clone();
            let part_id = tc.base.id;

            // Mark as running.
            let running_state = ToolState::Running(ToolStateRunning {
                input: tc.input().clone(),
                output: String::new(),
                time: ToolTime {
                    start: Utc::now().timestamp_millis(),
                    end: None,
                },
            });

            if let Some(ref mut msg) = assistant_msg {
                self.update_tool_call(msg, part_id, running_state.clone());
            }
            let _ = ev_tx
                .send(AgentEvent::PartUpdated {
                    part_id,
                    part: Part::ToolCall(ToolCallPart {
                        base: tc.base.clone(),
                        tool_name: tc.tool_name.clone(),
                        call_id: tc.call_id.clone(),
                        state: running_state,
                        raw_input: tc.raw_input.clone(),
                    }),
                })
                .await;
            let _ = ev_tx
                .send(AgentEvent::ToolStart {
                    call_id: call_id.clone(),
                })
                .await;

            let hook_call = HookToolCall {
                tool_name: tc.tool_name.clone(),
                call_id: tc.call_id.clone(),
                input: tc.input().clone(),
            };

            // Permission check.
            let sensitivity = self
                .tools
                .get(&tc.tool_name)
                .map(|t| t.sensitivity())
                .unwrap_or(Sensitivity::Dangerous);
            let default_decision = if let Some(ref engine) = self.permission_engine {
                engine
                    .check(&tc.tool_name, &hook_call.input, sensitivity)
                    .await
            } else {
                match sensitivity {
                    Sensitivity::ReadOnly => PermissionDecision::AllowOnce,
                    _ => PermissionDecision::Prompt,
                }
            };
            let decision = self
                .dispatcher
                .on_permission_ask(&hook_call, default_decision)
                .await;

            let decision = if decision == PermissionDecision::Prompt {
                let (perm_tx, perm_rx) = oneshot::channel();
                let _ = ev_tx
                    .send(AgentEvent::PermissionRequest {
                        call: hook_call.clone(),
                        tx: perm_tx,
                    })
                    .await;
                match perm_rx.await {
                    Ok(d) => d,
                    Err(_) => PermissionDecision::Deny,
                }
            } else {
                decision
            };

            if decision == PermissionDecision::AllowSession {
                if let Some(ref engine) = self.permission_engine {
                    engine.add_session_allow(&tc.tool_name).await;
                }
            }

            if decision == PermissionDecision::Deny {
                let error_state = ToolState::Error(ToolStateError {
                    input: hook_call.input.clone(),
                    error: "permission denied".into(),
                    time: ToolTime {
                        start: Utc::now().timestamp_millis(),
                        end: Some(Utc::now().timestamp_millis()),
                    },
                });
                if let Some(ref mut msg) = assistant_msg {
                    self.update_tool_call(msg, part_id, error_state.clone());
                }
                let _ = ev_tx
                    .send(AgentEvent::PartUpdated {
                        part_id,
                        part: Part::ToolCall(ToolCallPart {
                            base: tc.base.clone(),
                            tool_name: tc.tool_name.clone(),
                            call_id: tc.call_id.clone(),
                            state: error_state,
                            raw_input: tc.raw_input.clone(),
                        }),
                    })
                    .await;
                let _ = ev_tx
                    .send(AgentEvent::ToolEnd {
                        call_id: call_id.clone(),
                        success: false,
                    })
                    .await;
                result_parts.push(Part::ToolResult(ToolResultPart {
                    base: PartBase {
                        id: ulid::Ulid::new(),
                        message_id: assistant_msg.as_ref().unwrap().id,
                        session_id: self.session_id,
                    },
                    call_id: tc.call_id.clone(),
                }));
                continue;
            }

            let tool = match self.tools.get(&tc.tool_name) {
                Some(t) => t,
                None => {
                    let error_state = ToolState::Error(ToolStateError {
                        input: hook_call.input.clone(),
                        error: format!("unknown tool {:?}", tc.tool_name),
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    });
                    if let Some(ref mut msg) = assistant_msg {
                        self.update_tool_call(msg, part_id, error_state.clone());
                    }
                    let _ = ev_tx
                        .send(AgentEvent::PartUpdated {
                            part_id,
                            part: Part::ToolCall(ToolCallPart {
                                base: tc.base.clone(),
                                tool_name: tc.tool_name.clone(),
                                call_id: tc.call_id.clone(),
                                state: error_state,
                                raw_input: tc.raw_input.clone(),
                            }),
                        })
                        .await;
                    let _ = ev_tx
                        .send(AgentEvent::ToolEnd {
                            call_id: call_id.clone(),
                            success: false,
                        })
                        .await;
                    result_parts.push(Part::ToolResult(ToolResultPart {
                        base: PartBase {
                            id: ulid::Ulid::new(),
                            message_id: assistant_msg.as_ref().unwrap().id,
                            session_id: self.session_id,
                        },
                        call_id: tc.call_id.clone(),
                    }));
                    continue;
                }
            };

            let input = if hook_call.input.is_null() && !tc.raw_input.is_empty() {
                serde_json::from_str(&tc.raw_input).unwrap_or_else(|_| hook_call.input.clone())
            } else {
                hook_call.input.clone()
            };
            let input = self
                .dispatcher
                .on_tool_execute_before(&hook_call, input)
                .await;

            let (progress_tx, mut progress_rx) = mpsc::channel::<ToolProgress>(16);
            // Forward progress chunks to the TUI with 50ms debounce.
            let ev_tx2 = ev_tx.clone();
            let call_id2 = call_id.clone();
            tokio::spawn(async move {
                let mut buf = String::new();
                let mut tick = tokio::time::interval(
                    tokio::time::Duration::from_millis(50),
                );
                // Skip immediate first tick so first chunk isn't delayed.
                tick.tick().await;
                loop {
                    tokio::select! {
                        progress = progress_rx.recv() => {
                            match progress {
                                Some(ToolProgress::OutputChunk(chunk)) => {
                                    buf.push_str(&chunk);
                                }
                                Some(ToolProgress::Metadata(_)) => {}
                                None => {
                                    // Channel closed; flush remaining.
                                    if !buf.is_empty() {
                                        let _ = ev_tx2
                                            .send(AgentEvent::ToolProgress {
                                                call_id: call_id2.clone(),
                                                chunk: std::mem::take(&mut buf),
                                            })
                                            .await;
                                    }
                                    break;
                                }
                            }
                        }
                        _ = tick.tick() => {
                            if !buf.is_empty() {
                                let _ = ev_tx2
                                    .send(AgentEvent::ToolProgress {
                                        call_id: call_id2.clone(),
                                        chunk: std::mem::take(&mut buf),
                                    })
                                    .await;
                            }
                        }
                    }
                }
            });

            let ctx = ToolCtx {
                session_id: self.session_id,
                call_id: tc.call_id.clone(),
                cancel: self.cancel_token.child_token(),
                progress_tx,
                cwd: std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                dispatcher: Some(self.dispatcher.clone()),
            };

            tracing::info!(tool = %tc.tool_name, call_id = %call_id, input = %input, "executing tool");
            let exec_result = tool.execute(ctx, input.clone()).await;
            let tool_output = match exec_result {
                Ok(out) => {
                    tracing::info!(tool = %tc.tool_name, call_id = %call_id, output = %out.output, error = %out.error, "tool executed successfully");
                    out
                }
                Err(e) => {
                    tracing::warn!(tool = %tc.tool_name, call_id = %call_id, error = %e, "tool execution failed");
                    ToolOutput {
                        output: String::new(),
                        error: e.to_string(),
                        diff: None,
                    }
                }
            };

            // Update hook_call with parsed input so hooks and final state are correct.
            let hook_call = HookToolCall {
                tool_name: hook_call.tool_name,
                call_id: hook_call.call_id,
                input: input.clone(),
            };

            let output = self
                .dispatcher
                .on_tool_execute_after(&hook_call, tool_output)
                .await;

            tracing::info!(tool = %tc.tool_name, call_id = %call_id, success = %output.error.is_empty(), "tool finished");
            let (success, final_state) = if !output.error.is_empty() {
                (
                    false,
                    ToolState::Error(ToolStateError {
                        input: input.clone(),
                        error: output.error.clone(),
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    }),
                )
            } else {
                (
                    true,
                    ToolState::Completed(ToolStateCompleted {
                        input: input.clone(),
                        output: output.output.clone(),
                        metadata: None,
                        diff: output.diff.clone(),
                        time: ToolTime {
                            start: Utc::now().timestamp_millis(),
                            end: Some(Utc::now().timestamp_millis()),
                        },
                    }),
                )
            };

            if let Some(ref mut msg) = assistant_msg {
                self.update_tool_call(msg, part_id, final_state.clone());
            }
            let _ = ev_tx
                .send(AgentEvent::PartUpdated {
                    part_id,
                    part: Part::ToolCall(ToolCallPart {
                        base: tc.base.clone(),
                        tool_name: tc.tool_name.clone(),
                        call_id: tc.call_id.clone(),
                        state: final_state,
                        raw_input: tc.raw_input.clone(),
                    }),
                })
                .await;
            let _ = ev_tx
                .send(AgentEvent::ToolEnd {
                    call_id: call_id.clone(),
                    success,
                })
                .await;

            result_parts.push(Part::ToolResult(ToolResultPart {
                base: PartBase {
                    id: ulid::Ulid::new(),
                    message_id: assistant_msg.as_ref().unwrap().id,
                    session_id: self.session_id,
                },
                call_id: tc.call_id.clone(),
            }));
        }
        result_parts
    }
}
