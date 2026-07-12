use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use mew_hooks::Dispatcher;
use mew_message::{SessionId, ToolState, TurnManifest};
use mew_provider::Provider;
use mew_subagents::{
    ModelResolver, SubagentDef, SubagentError, SubagentEvent, SubagentOutcome, SubagentResult,
    SubagentRunOptions, SubagentRunner,
};
use mew_tools::Tool;

/// If the given tool state has completed successfully, return the output
/// string. Used by the runner to pull the final answer out of an `exit_tool`
/// call's completed state.
fn extract_completed_output(state: &ToolState) -> Option<String> {
    if let ToolState::Completed(s) = state {
        return Some(s.output.clone());
    }
    None
}

/// Extract per-turn manifests from a child agent's assistant messages.
/// Each assistant message may carry a `TurnManifest` on its `AssistantMeta`;
/// we collect all of them in order. Takes `&Agent` (shared reference) —
/// since `agent.messages` is `Arc<Mutex<...>>`, the borrow is cheap and
/// the agent remains owned by the caller.
async fn extract_manifests(agent: &crate::Agent) -> Vec<TurnManifest> {
    let messages = agent.messages.lock().await;
    messages
        .iter()
        .filter_map(|m| m.assistant.as_ref())
        .filter_map(|meta| meta.manifest.clone())
        .collect()
}

/// Simple subagent runner that spawns a child agent for each invocation.
pub struct SimpleRunner {
    default_provider: Arc<dyn Provider>,
    tools: HashMap<String, Arc<dyn Tool>>,
    dispatcher: Arc<dyn Dispatcher>,
    /// Optional resolver for per-subagent model overrides. When a subagent def
    /// has `model: "provider/model"`, the runner asks the resolver for a
    /// provider for that model. If no resolver is configured, the override is
    /// ignored and the default provider is used.
    model_resolver: Option<Arc<dyn ModelResolver>>,
    /// Optional override for where subagent session files are written.
    /// `None` means use the global `mew_session::session_dir()`. Set this
    /// from tests to isolate the runner from the user's real sessions.
    session_root: Option<PathBuf>,
}

impl SimpleRunner {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Vec<Arc<dyn Tool>>,
        dispatcher: Arc<dyn Dispatcher>,
    ) -> Self {
        let tools_map = tools
            .into_iter()
            .map(|t| (t.name().to_string(), t))
            .collect();
        Self {
            default_provider: provider,
            tools: tools_map,
            dispatcher,
            model_resolver: None,
            session_root: None,
        }
    }

    /// Builder method to attach a model resolver for per-subagent overrides.
    pub fn with_model_resolver(mut self, resolver: Arc<dyn ModelResolver>) -> Self {
        self.model_resolver = Some(resolver);
        self
    }

    /// Builder method to override the root directory for subagent session
    /// files. Used by tests to isolate from the global session dir.
    pub fn with_session_root(mut self, root: PathBuf) -> Self {
        self.session_root = Some(root);
        self
    }

    /// Resolve which provider to use for this subagent invocation. A
    /// call-time `model` override beats the def's `model`; either beats the
    /// default provider. A literal value of `"micro"`, `"deci"`, or `"nano"`
    /// is resolved by the `ModelResolver` against the active router's tiers.
    async fn resolve_provider(&self, def: &SubagentDef, model: Option<&str>) -> Arc<dyn Provider> {
        let model = model.or(def.model.as_deref());

        let Some(model) = model else {
            return self.default_provider.clone();
        };

        let Some(ref resolver) = self.model_resolver else {
            tracing::warn!(
                subagent = %def.name,
                model = %model,
                "subagent has model override but no resolver configured; using default provider"
            );
            return self.default_provider.clone();
        };
        match resolver.resolve(model).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    subagent = %def.name,
                    model = %model,
                    error = %e,
                    "failed to resolve subagent model override; using default provider"
                );
                self.default_provider.clone()
            }
        }
    }
}

#[async_trait]
impl SubagentRunner for SimpleRunner {
    async fn run(&self, opts: SubagentRunOptions<'_>) -> Result<SubagentResult, SubagentError> {
        let def = opts.def;
        let prompt = opts.prompt;
        let parent_session_id = opts.parent_session_id;
        let event_tx = opts.event_tx;
        let cancel = opts.cancel;
        let model = opts.model.as_deref();
        let session_id = SessionId::new();

        // Build tool subset if the subagent restricts tools.
        let tools: Vec<Arc<dyn Tool>> = if let Some(ref allowed) = def.tools {
            allowed
                .iter()
                .filter_map(|name| self.tools.get(name).cloned())
                .collect()
        } else {
            self.tools.values().cloned().collect()
        };

        // Open a subagent session file nested under the parent. If this fails
        // the subagent still runs, but we surface the failure via
        // `session_unavailable` in the result so the user can see it instead
        // of silently getting a subagent with no transcript.
        let open_result = match &self.session_root {
            Some(root) => {
                mew_session::Writer::open_subagent_at(
                    root,
                    &parent_session_id.to_string(),
                    &session_id.to_string(),
                    &def.name,
                )
                .await
            }
            None => {
                mew_session::Writer::open_subagent(
                    &parent_session_id.to_string(),
                    &session_id.to_string(),
                    &def.name,
                )
                .await
            }
        };
        let (session_writer, session_unavailable) = match open_result {
            Ok(w) => (Some(w), false),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    parent_session_id = %parent_session_id,
                    subagent = %def.name,
                    "could not open subagent session; running without transcript"
                );
                (None, true)
            }
        };

        let mut agent = crate::Agent::new(
            self.resolve_provider(def, model).await,
            self.dispatcher.clone(),
            session_writer,
            tools,
            Some(session_id),
        );

        // Set the subagent's body as the system prompt. If the def has
        // `template: true`, render it through minijinja with the subagent's
        // context (subagent_name, tools, model, etc).
        if !def.body.is_empty() {
            let body = if def.template {
                let tool_names: Vec<String> = agent.tools.keys().cloned().collect();
                let ctx = mew_prompts::template::TemplateContext {
                    subagent_name: def.name.clone(),
                    model_id: agent.model_id.clone(),
                    provider_id: agent.provider_id.clone(),
                    session_id: session_id.to_string(),
                    cwd: std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    current_date: mew_prompts::template::TemplateContext::today(),
                    tools: tool_names,
                    ..Default::default()
                };
                mew_prompts::template::render(&def.body, &ctx)
            } else {
                def.body.clone()
            };
            agent.set_system(body);
        }

        // Set max_turns if specified, else apply the built-in default.
        // The turn count is the secondary safeguard; the wall-clock cap is
        // the primary one (catches stuck subagents in tight tool loops).
        agent.max_turns = Some(def.max_turns.unwrap_or(mew_subagents::DEFAULT_MAX_TURNS));

        let max_duration_secs = def
            .max_duration_secs
            .unwrap_or(mew_subagents::DEFAULT_MAX_DURATION_SECS);

        let display_name = mew_subagents::pick_display_name(session_id.0);
        let _ = event_tx
            .send(SubagentEvent::Started {
                child_session_id: session_id.to_string(),
                display_name: Some(display_name.to_string()),
            })
            .await;

        // Run the prompt through the agent.
        let mut rx = agent.run(prompt);
        let mut result_text = String::new();
        let mut turns_used: u32 = 0;
        let mut last_error: Option<String> = None;
        let started_at = std::time::Instant::now();
        let max_duration = std::time::Duration::from_secs(max_duration_secs);
        let mut hit_time_limit = false;
        // Map call_id → tool_name so we can emit SubagentEvent::ToolStart /
        // ToolEnd (the AgentEvent variants don't carry the tool name; we
        // pick it up from PartStart).
        let mut tool_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // Live copy of every tool-call part the subagent has emitted. We
        // track these so we can (a) parse streamed arguments into `input`
        // ourselves (mirroring the agent's reconcile) and (b) read the
        // input for `progress_update` parts at PartEnd to emit a
        // SubagentEvent::Progress.
        let mut tool_parts: std::collections::HashMap<
            mew_message::PartId,
            mew_message::ToolCallPart,
        > = std::collections::HashMap::new();
        // If the subagent called `exit_tool`, capture its output here and
        // break the loop after the tool completes.
        let mut exit_answer: Option<String> = None;

        while let Some(event) = rx.recv().await {
            if cancel.is_cancelled() {
                let manifests = extract_manifests(&agent).await;
                let _ = event_tx
                    .send(SubagentEvent::Finished {
                        child_session_id: session_id.to_string(),
                        outcome: SubagentOutcome::Cancelled,
                        manifests,
                    })
                    .await;
                return Ok(SubagentResult::Cancelled);
            }
            match event {
                crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartStart {
                    part: mew_message::Part::ToolCall(tc),
                }) => {
                    tool_names.insert(tc.call_id.clone(), tc.tool_name.clone());
                    tool_parts.insert(tc.base.id, tc.clone());
                    let _ = event_tx
                        .send(SubagentEvent::ToolStart {
                            call_id: tc.call_id,
                            tool_name: tc.tool_name,
                        })
                        .await;
                }
                crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartDelta {
                    part_id,
                    field: "arguments",
                    delta,
                }) => {
                    if let Some(tc) = tool_parts.get_mut(&part_id) {
                        tc.raw_input.push_str(&delta);
                    }
                }
                crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartDelta {
                    field: "text",
                    delta,
                    ..
                }) => {
                    result_text.push_str(&delta);
                    let _ = event_tx
                        .send(SubagentEvent::TextDelta { text: delta })
                        .await;
                }
                crate::AgentEvent::Provider(mew_provider::ProviderEvent::PartEnd { part_id }) => {
                    // Reconcile streamed arguments into `state.input` and
                    // surface `progress_update` messages.
                    if let Some(tc) = tool_parts.get_mut(&part_id) {
                        if !tc.raw_input.is_empty() {
                            if let Ok(value) =
                                serde_json::from_str::<serde_json::Value>(&tc.raw_input)
                            {
                                tc.state.set_input(value);
                            }
                        }
                        if tc.tool_name == "progress_update" {
                            let message = tc
                                .state
                                .input()
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !message.is_empty() {
                                let _ = event_tx
                                    .send(SubagentEvent::Progress {
                                        call_id: tc.call_id.clone(),
                                        tool_name: tc.tool_name.clone(),
                                        message,
                                    })
                                    .await;
                            }
                        }
                    }
                }
                crate::AgentEvent::Provider(mew_provider::ProviderEvent::MessageEnd { .. }) => {
                    turns_used += 1;
                    if let Some(limit) = def.max_turns.or(Some(mew_subagents::DEFAULT_MAX_TURNS)) {
                        if turns_used >= limit {
                            // Hit the cap. Stop accumulating text after this
                            // turn; return what we have.
                            break;
                        }
                    }
                    if started_at.elapsed() >= max_duration {
                        hit_time_limit = true;
                        break;
                    }
                }
                crate::AgentEvent::ToolEnd { call_id, success } => {
                    let _ = event_tx
                        .send(SubagentEvent::ToolEnd {
                            call_id: call_id.clone(),
                            success,
                        })
                        .await;
                }
                crate::AgentEvent::PartUpdated {
                    part: mew_message::Part::ToolCall(tc),
                    ..
                } => {
                    // Keep our copy of the part in sync with the agent's.
                    // This catches the Completed state for exit_tool; for
                    // progress_update we already handled it at PartEnd.
                    tool_parts.insert(tc.base.id, tc.clone());
                    if tc.tool_name == "exit_tool" {
                        if let Some(answer) = extract_completed_output(&tc.state) {
                            exit_answer = Some(answer);
                            break;
                        }
                    }
                }
                crate::AgentEvent::Error(msg) => {
                    last_error = Some(msg);
                    break;
                }
                _ => {}
            }
        }

        if let Some(answer) = exit_answer {
            result_text = answer;
        }

        if let Some(reason) = last_error {
            let manifests = extract_manifests(&agent).await;
            let _ = event_tx
                .send(SubagentEvent::Finished {
                    child_session_id: session_id.to_string(),
                    outcome: SubagentOutcome::Failed {
                        reason: reason.clone(),
                    },
                    manifests,
                })
                .await;
            return Ok(SubagentResult::Error { reason });
        }

        let hit_turn_limit = def
            .max_turns
            .map(|limit| turns_used >= limit)
            .unwrap_or(false);

        let child_manifests = extract_manifests(&agent).await;

        let _ = event_tx
            .send(SubagentEvent::Finished {
                child_session_id: session_id.to_string(),
                outcome: SubagentOutcome::Completed,
                manifests: child_manifests.clone(),
            })
            .await;

        Ok(SubagentResult::Complete {
            text: result_text,
            turns_used,
            hit_turn_limit,
            hit_time_limit,
            session_unavailable,
            manifests: child_manifests,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the time/turn limit behavior of `SimpleRunner`.
    //!
    //! These exercise the wall-clock and turn-count caps independently and
    //! together. The runner is what enforces both — the agent's own
    //! `max_turns` is a secondary check; the wall-clock is the primary
    //! safeguard against a subagent in a tight tool loop.

    use super::*;
    use async_trait::async_trait;
    use futures::{stream, StreamExt};
    use mew_provider::{EventStream, ProviderError};
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::time::sleep;

    /// A scripted provider that yields each event in order, optionally
    /// sleeping before the next one. Lets the test control wall-clock pace
    /// of the agent's stream so the runner's time-based cap can fire
    /// deterministically.
    struct ScriptedProvider {
        script: StdMutex<Vec<(Duration, Vec<mew_provider::ProviderEvent>)>>,
    }

    impl ScriptedProvider {
        fn new(script: Vec<(Duration, Vec<mew_provider::ProviderEvent>)>) -> Self {
            Self {
                script: StdMutex::new(script),
            }
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn stream(&self, _req: mew_provider::Request) -> Result<EventStream, ProviderError> {
            // Atomically consume the script for this call. The first
            // `stream()` invocation gets the events; subsequent calls
            // (i.e. the agent's later turns) get an empty stream, so the
            // agent's `turn_loop` exits naturally after one turn. This
            // matches the shape tests need: drive a single turn with a
            // known script, then let the loop end.
            //
            // The previous behavior (replay the script on every call)
            // happened to work for single-turn tests but produced
            // surprising "stuck in a loop" failures for tests that emit
            // `SubagentEvent::Progress` etc. — the same call was emitted
            // once per replay.
            let script = {
                let mut guard = self.script.lock().unwrap();
                if guard.is_empty() {
                    Vec::new()
                } else {
                    std::mem::take(&mut *guard)
                }
            };
            let s = stream::unfold(script.into_iter(), |mut iter| async move {
                match iter.next() {
                    Some((delay, events)) => {
                        if !delay.is_zero() {
                            sleep(delay).await;
                        }
                        Some((stream::iter(events), iter))
                    }
                    None => None,
                }
            })
            .flatten();
            Ok(Box::pin(s))
        }
    }

    /// Build a single-turn text script: PartStart → text delta → PartEnd →
    /// MessageEnd. The text delta content is irrelevant for these tests.
    fn turn(label: &str) -> Vec<mew_provider::ProviderEvent> {
        let part_id = mew_message::PartId::new();
        let message_id = mew_message::MessageId::new();
        let session_id = mew_message::SessionId::new();
        vec![
            mew_provider::ProviderEvent::PartStart {
                part: mew_message::Part::Text(mew_message::TextPart {
                    base: mew_message::PartBase {
                        id: part_id,
                        message_id,
                        session_id,
                    },
                    text: String::new(),
                    synthetic: false,
                }),
            },
            mew_provider::ProviderEvent::PartDelta {
                part_id,
                field: "text",
                delta: label.to_string(),
            },
            mew_provider::ProviderEvent::PartEnd { part_id },
            mew_provider::ProviderEvent::MessageEnd {
                finish: mew_message::Finish::Stop,
                usage: mew_message::Tokens::default(),
                cost: 0.0,
            },
        ]
    }

    fn make_def(max_turns: Option<u32>, max_duration_secs: Option<u64>) -> SubagentDef {
        SubagentDef {
            name: "test-agent".into(),
            description: "test".into(),
            model: None,
            tools: None,
            max_turns,
            max_duration_secs,
            body: String::new(),
            path: PathBuf::from("(test)"),
            template: false,
        }
    }

    /// Build a script that calls `exit_tool` with a given final answer in
    /// turn 0, followed by a turn-1 text response. The second turn should
    /// never be reached if `exit_tool` is wired correctly.
    fn exit_tool_script(call_id: &str, final_answer: &str) -> Vec<mew_provider::ProviderEvent> {
        let part_id = mew_message::PartId::new();
        let message_id = mew_message::MessageId::new();
        let session_id = mew_message::SessionId::new();
        let now = chrono::Utc::now().timestamp_millis();
        vec![
            mew_provider::ProviderEvent::PartStart {
                part: mew_message::Part::ToolCall(mew_message::ToolCallPart {
                    base: mew_message::PartBase {
                        id: part_id,
                        message_id,
                        session_id,
                    },
                    tool_name: "exit_tool".into(),
                    call_id: call_id.into(),
                    state: mew_message::ToolState::Pending(mew_message::ToolStatePending {
                        input: serde_json::json!({"final_answer": final_answer}),
                        time: mew_message::ToolTime {
                            start: now,
                            end: None,
                        },
                    }),
                    raw_input: String::new(),
                }),
            },
            mew_provider::ProviderEvent::PartEnd { part_id },
            mew_provider::ProviderEvent::MessageEnd {
                finish: mew_message::Finish::ToolUse,
                usage: mew_message::Tokens::default(),
                cost: 0.0,
            },
        ]
    }

    fn exit_tool() -> Arc<dyn mew_tools::Tool> {
        Arc::new(mew_tools::tools::exit_tool::ExitTool)
    }

    /// Run a subagent with the given script and def, returning the result.
    /// Each script entry is (delay_before, events). Drains the
    /// `SubagentEvent` channel concurrently so the runner can produce
    /// without blocking on the (unread) UI side.
    async fn run_subagent(
        script: Vec<(Duration, Vec<mew_provider::ProviderEvent>)>,
        def: SubagentDef,
    ) -> SubagentResult {
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
            })
            .await
            .unwrap();
        drain.abort();
        result
    }

    #[tokio::test]
    async fn test_turn_limit_trips_before_time_limit() {
        // 5 turns available, generous time budget.
        let script: Vec<_> = (0..5)
            .map(|i| (Duration::from_millis(0), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(Some(3), Some(60));
        let result = run_subagent(script, def).await;
        match result {
            SubagentResult::Complete {
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                ..
            } => {
                assert_eq!(turns_used, 3);
                assert!(hit_turn_limit, "should have hit the turn cap");
                assert!(!hit_time_limit, "should not have hit the time cap");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_time_limit_trips_when_each_turn_sleeps() {
        // max_turns is huge, but each turn takes 50ms and the cap is 0s —
        // any non-zero elapsed time at MessageEnd trips it.
        let script: Vec<_> = (0..20)
            .map(|i| (Duration::from_millis(50), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(Some(1000), Some(0));
        let result = run_subagent(script, def).await;
        match result {
            SubagentResult::Complete {
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                ..
            } => {
                assert!(
                    (1..20).contains(&turns_used),
                    "should bail mid-run, got {turns_used}"
                );
                assert!(!hit_turn_limit, "should not have hit the turn cap (1000)");
                assert!(hit_time_limit, "should have hit the time cap (0s)");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_neither_limit_trips_on_short_run() {
        // 2 turns, plenty of both budgets.
        let script: Vec<_> = (0..2)
            .map(|i| (Duration::from_millis(0), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(Some(500), Some(60));
        let result = run_subagent(script, def).await;
        match result {
            SubagentResult::Complete {
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                text,
                ..
            } => {
                assert_eq!(turns_used, 2);
                assert!(!hit_turn_limit);
                assert!(!hit_time_limit);
                assert!(text.contains("t0") && text.contains("t1"));
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_defaults_apply_when_def_omits_both_caps() {
        // No caps on the def. The runner should apply built-in defaults
        // (500 turns, 300s) — neither will trip on a short script.
        let script: Vec<_> = (0..2)
            .map(|i| (Duration::from_millis(0), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(None, None);
        let result = run_subagent(script, def).await;
        match result {
            SubagentResult::Complete {
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                ..
            } => {
                assert_eq!(turns_used, 2);
                assert!(!hit_turn_limit);
                assert!(!hit_time_limit);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    /// End-to-end: running a subagent through `SimpleRunner` should write
    /// the subagent's `session.jsonl` and `meta.json` to disk, AND update
    /// the parent's `meta.json` to register the child. This is the bug
    /// that hid the cause of `session 01KV48M8R0CAN7BXB1RG75EWCD` —
    /// the subagent ran but left no transcript.
    #[tokio::test]
    async fn test_subagent_writes_session_and_updates_parent_meta() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();

        // Set up the parent's session dir + meta by hand (avoids touching
        // the global session_dir).
        let parent_id = ulid::Ulid::new().to_string();
        let parent_session_id = ulid::Ulid::from_string(&parent_id).expect("valid ulid");
        let parent_dir = root.join(&parent_id);
        std::fs::create_dir_all(&parent_dir).expect("mkdir parent");
        let meta = mew_session::Meta::new(&parent_id);
        let meta_path = parent_dir.join("meta.json");
        std::fs::write(&meta_path, serde_json::to_vec_pretty(&meta).unwrap())
            .expect("write parent meta");

        let script: Vec<_> = (0..1)
            .map(|i| (Duration::from_millis(0), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(Some(10), Some(60));

        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
            .with_session_root(root.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id,
                event_tx: tx,
                cancel,
                model: None,
            })
            .await
            .expect("runner ok");
        drain.abort();
        assert!(matches!(result, SubagentResult::Complete { .. }));

        // The subagent's session directory should exist with both files.
        let subagents_root = root.join(&parent_id).join("subagents");
        assert!(
            subagents_root.exists(),
            "subagents/ dir should exist under parent"
        );
        let mut child_dirs: Vec<_> = std::fs::read_dir(&subagents_root)
            .expect("read subagents")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            child_dirs.len(),
            1,
            "expected exactly one child dir, got {}",
            child_dirs.len()
        );
        let child_dir = child_dirs.pop().unwrap().path();
        assert!(
            child_dir.join("meta.json").exists(),
            "child meta.json missing"
        );
        assert!(
            child_dir.join("session.jsonl").exists(),
            "child session.jsonl missing"
        );

        // And the parent's meta.json should now reference the child.
        let parent_meta_path = root.join(&parent_id).join("meta.json");
        let parent_meta_bytes = std::fs::read(&parent_meta_path).expect("read parent meta");
        let parent_meta: mew_session::Meta =
            serde_json::from_slice(&parent_meta_bytes).expect("parse parent meta");
        assert_eq!(parent_meta.children_session_ids.len(), 1);
        let child_id = child_dir.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(parent_meta.children_session_ids[0], child_id);
    }

    /// When the subagent session file cannot be opened, the runner should
    /// still complete the run and surface `session_unavailable = true` in
    /// the result so the user sees a "transcript could not be written"
    /// warning rather than silently losing the transcript.
    #[tokio::test]
    async fn test_session_unavailable_when_cannot_open() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Place a regular file at <tmp>/sessions, so the runner's
        // `create_dir_all(<root>/<parent_id>)` will fail because the
        // parent component is not a directory.
        let blocker = tmp.path().join("sessions");
        std::fs::write(&blocker, b"not a directory").expect("write blocker");

        let script: Vec<_> = (0..1)
            .map(|i| (Duration::from_millis(0), turn(&format!("t{i}"))))
            .collect();
        let def = make_def(Some(10), Some(60));
        let parent_session_id = ulid::Ulid::new();

        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher))
            .with_session_root(blocker.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id,
                event_tx: tx,
                cancel,
                model: None,
            })
            .await
            .expect("runner ok");
        drain.abort();

        match result {
            SubagentResult::Complete {
                session_unavailable,
                ..
            } => {
                assert!(
                    session_unavailable,
                    "session_unavailable should be true when open fails"
                );
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    /// When the subagent calls `exit_tool(final_answer)` the runner should
    /// break the loop, set `result_text` to the final answer, and not
    /// continue with subsequent turns.
    #[tokio::test]
    async fn test_exit_tool_short_circuits_with_final_answer() {
        let script = vec![(
            Duration::from_millis(0),
            exit_tool_script("c1", "the answer is 42"),
        )];
        let def = make_def(Some(10), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(
            provider,
            vec![exit_tool()],
            Arc::new(mew_hooks::NopDispatcher),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move { while let Some(_ev) = rx.recv().await {} });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
            })
            .await
            .unwrap();
        drain.abort();

        match result {
            SubagentResult::Complete {
                text,
                turns_used,
                hit_turn_limit,
                hit_time_limit,
                ..
            } => {
                assert_eq!(text, "the answer is 42");
                assert_eq!(turns_used, 1, "should have stopped after one turn");
                assert!(!hit_turn_limit);
                assert!(!hit_time_limit);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    /// The runner should forward every tool call as a `SubagentEvent::ToolStart`
    /// so the parent (and any UI watching the channel) can see what the
    /// subagent is doing. Previously this was unwired — the parent's
    /// `ToolStart`/`ToolEnd` pump received events that the runner never
    /// actually sent. This test guards the wiring for a tool that does
    /// not short-circuit the run (progress_update).
    #[tokio::test]
    async fn test_subagent_tool_start_end_events_emitted() {
        let progress_part_id = mew_message::PartId::new();
        let progress_mid = mew_message::MessageId::new();
        let progress_sid = mew_message::SessionId::new();
        let now = chrono::Utc::now().timestamp_millis();

        let script: Vec<(Duration, Vec<mew_provider::ProviderEvent>)> = vec![(
            Duration::from_millis(0),
            vec![
                mew_provider::ProviderEvent::PartStart {
                    part: mew_message::Part::ToolCall(mew_message::ToolCallPart {
                        base: mew_message::PartBase {
                            id: progress_part_id,
                            message_id: progress_mid,
                            session_id: progress_sid,
                        },
                        tool_name: "progress_update".into(),
                        call_id: "p1".into(),
                        state: mew_message::ToolState::Pending(mew_message::ToolStatePending {
                            input: serde_json::json!({"message": "starting work"}),
                            time: mew_message::ToolTime {
                                start: now,
                                end: None,
                            },
                        }),
                        raw_input: String::new(),
                    }),
                },
                mew_provider::ProviderEvent::PartEnd {
                    part_id: progress_part_id,
                },
                mew_provider::ProviderEvent::MessageEnd {
                    finish: mew_message::Finish::ToolUse,
                    usage: mew_message::Tokens::default(),
                    cost: 0.0,
                },
            ],
        )];
        let def = make_def(Some(10), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(
            provider,
            vec![Arc::new(mew_tools::tools::progress_update::ProgressUpdate)],
            Arc::new(mew_hooks::NopDispatcher),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move {
            let mut events: Vec<SubagentEvent> = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
            })
            .await
            .unwrap();
        drop(result);
        let events = drain.await.unwrap();

        let tool_starts: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::ToolStart { call_id, tool_name } => {
                    Some((call_id.clone(), tool_name.clone()))
                }
                _ => None,
            })
            .collect();
        let tool_ends: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::ToolEnd { call_id, success } => Some((call_id.clone(), *success)),
                _ => None,
            })
            .collect();

        assert!(
            tool_starts
                .iter()
                .any(|(_, name)| name == "progress_update"),
            "expected a ToolStart for progress_update, got {tool_starts:?}"
        );
        assert!(
            tool_ends.iter().any(|(id, _)| id == "p1"),
            "expected a ToolEnd for p1, got {tool_ends:?}"
        );
    }

    /// When the subagent calls `progress_update(message: "...")`, the
    /// runner should emit a `SubagentEvent::Progress` carrying that
    /// message so the parent (and the TUI's sidebar) can show what the
    /// subagent is working on.
    #[tokio::test]
    async fn test_progress_update_emits_subagent_progress_event() {
        let progress_part_id = mew_message::PartId::new();
        let progress_mid = mew_message::MessageId::new();
        let progress_sid = mew_message::SessionId::new();
        let now = chrono::Utc::now().timestamp_millis();

        let script: Vec<(Duration, Vec<mew_provider::ProviderEvent>)> = vec![(
            Duration::from_millis(0),
            vec![
                mew_provider::ProviderEvent::PartStart {
                    part: mew_message::Part::ToolCall(mew_message::ToolCallPart {
                        base: mew_message::PartBase {
                            id: progress_part_id,
                            message_id: progress_mid,
                            session_id: progress_sid,
                        },
                        tool_name: "progress_update".into(),
                        call_id: "p1".into(),
                        state: mew_message::ToolState::Pending(mew_message::ToolStatePending {
                            input: serde_json::json!({"message": "scanning the repo"}),
                            time: mew_message::ToolTime {
                                start: now,
                                end: None,
                            },
                        }),
                        raw_input: String::new(),
                    }),
                },
                mew_provider::ProviderEvent::PartEnd {
                    part_id: progress_part_id,
                },
                mew_provider::ProviderEvent::MessageEnd {
                    finish: mew_message::Finish::ToolUse,
                    usage: mew_message::Tokens::default(),
                    cost: 0.0,
                },
            ],
        )];
        let def = make_def(Some(10), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(
            provider,
            vec![Arc::new(mew_tools::tools::progress_update::ProgressUpdate)],
            Arc::new(mew_hooks::NopDispatcher),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move {
            let mut events: Vec<SubagentEvent> = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        });
        let result = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
            })
            .await
            .unwrap();
        drop(result);
        let events = drain.await.unwrap();

        let progress: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::Progress { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            progress,
            vec!["scanning the repo".to_string()],
            "expected one Progress event with the message"
        );
    }

    /// The runner should pick a human-friendly display name from
    /// `mew_subagents::DISPLAY_NAMES` and emit it on `SubagentEvent::Started`.
    /// The same def can be spawned multiple times; each run should get its
    /// own (potentially-colliding) name.
    #[tokio::test]
    async fn test_started_event_includes_display_name() {
        let script = vec![(Duration::from_millis(0), turn("hi"))];
        let def = make_def(Some(2), Some(60));
        let provider = Arc::new(ScriptedProvider::new(script));
        let runner = SimpleRunner::new(provider, vec![], Arc::new(mew_hooks::NopDispatcher));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = tokio_util::sync::CancellationToken::new();
        let drain = tokio::spawn(async move {
            let mut events: Vec<SubagentEvent> = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        });
        let _ = runner
            .run(SubagentRunOptions {
                def: &def,
                prompt: "prompt".into(),
                parent_call_id: "call_0".into(),
                parent_session_id: SessionId::new(),
                event_tx: tx,
                cancel,
                model: None,
            })
            .await;
        let events = drain.await.unwrap();

        let started: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SubagentEvent::Started {
                    child_session_id,
                    display_name,
                } => Some((child_session_id.clone(), display_name.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(started.len(), 1, "expected exactly one Started event");
        let (_, display_name) = &started[0];
        let name = display_name
            .as_deref()
            .expect("Started should carry a display_name");
        assert!(
            mew_subagents::DISPLAY_NAMES.contains(&name),
            "display_name {name:?} should be from DISPLAY_NAMES"
        );
    }
}
