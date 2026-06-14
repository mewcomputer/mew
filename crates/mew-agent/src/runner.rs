use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use mew_hooks::Dispatcher;
use mew_message::SessionId;
use mew_provider::Provider;
use mew_subagents::{
    ModelResolver, SubagentDef, SubagentError, SubagentEvent, SubagentOutcome, SubagentResult,
    SubagentRunner,
};
use mew_tools::Tool;

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
        }
    }

    /// Builder method to attach a model resolver for per-subagent overrides.
    pub fn with_model_resolver(mut self, resolver: Arc<dyn ModelResolver>) -> Self {
        self.model_resolver = Some(resolver);
        self
    }

    /// Resolve which provider to use for this subagent invocation. If the
    /// def has a `model` override and a resolver is configured, build a
    /// provider for that model; otherwise fall back to the default.
    async fn resolve_provider(&self, def: &SubagentDef) -> Arc<dyn Provider> {
        let Some(ref model) = def.model else {
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
    async fn run(
        &self,
        def: &SubagentDef,
        prompt: String,
        _parent_call_id: String,
        parent_session_id: SessionId,
        event_tx: mpsc::Sender<SubagentEvent>,
        cancel: CancellationToken,
    ) -> Result<SubagentResult, SubagentError> {
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

        // Open a subagent session file nested under the parent. Persistence is
        // best-effort: if the file can't be created (e.g. read-only FS), the
        // subagent still runs without a transcript.
        let session_writer = match mew_session::Writer::open_subagent(
            &parent_session_id.to_string(),
            &session_id.to_string(),
            &def.name,
        )
        .await
        {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(error = %e, "could not open subagent session, running without persistence");
                None
            }
        };

        let mut agent = crate::Agent::new(
            self.resolve_provider(def).await,
            self.dispatcher.clone(),
            session_writer,
            tools,
            Some(session_id),
        );

        // Set the subagent's body as the system prompt.
        if !def.body.is_empty() {
            agent.set_system(def.body.clone());
        }

        // Set max_turns if specified.
        agent.max_turns = def.max_turns;

        let _ = event_tx
            .send(SubagentEvent::Started {
                child_session_id: session_id.to_string(),
            })
            .await;

        // Run the prompt through the agent.
        let mut rx = agent.run(prompt);
        let mut result_text = String::new();
        let mut turns_used: u32 = 0;
        let mut last_error: Option<String> = None;

        while let Some(event) = rx.recv().await {
            if cancel.is_cancelled() {
                let _ = event_tx
                    .send(SubagentEvent::Finished {
                        child_session_id: session_id.to_string(),
                        outcome: SubagentOutcome::Cancelled,
                    })
                    .await;
                return Ok(SubagentResult::Cancelled);
            }
            match event {
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
                crate::AgentEvent::Provider(mew_provider::ProviderEvent::MessageEnd { .. }) => {
                    turns_used += 1;
                    if let Some(limit) = def.max_turns {
                        if turns_used >= limit {
                            // Hit the cap. Stop accumulating text after this
                            // turn; return what we have.
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

        if let Some(reason) = last_error {
            let _ = event_tx
                .send(SubagentEvent::Finished {
                    child_session_id: session_id.to_string(),
                    outcome: SubagentOutcome::Failed {
                        reason: reason.clone(),
                    },
                })
                .await;
            return Ok(SubagentResult::Error { reason });
        }

        let hit_turn_limit = def
            .max_turns
            .map(|limit| turns_used >= limit)
            .unwrap_or(false);

        let _ = event_tx
            .send(SubagentEvent::Finished {
                child_session_id: session_id.to_string(),
                outcome: SubagentOutcome::Completed,
            })
            .await;

        Ok(SubagentResult::Complete {
            text: result_text,
            turns_used,
            hit_turn_limit,
        })
    }
}
