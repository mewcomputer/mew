use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Tool that queues a persona switch to be applied at the end of the
/// current turn. The actual swap (model pin, provider rebuild) happens in
/// the main loop, triggered by the `AgentEvent::PersonaSwitchRequested`
/// the agent emits when the turn ends.
///
/// Sharing a `pending` slot with the owning `Agent` (via `Arc<Mutex<...>>`)
/// is the cleanest way to thread the request from a tool call into the
/// turn loop's end-of-turn drain. Mid-turn application is intentionally
/// avoided: the model can chain `switch_persona` with a final text
/// response and the user sees the full answer before the swap.
pub struct SwitchPersona {
    /// All known personas. Used to validate the requested name.
    personas: Arc<Vec<mew_personas::Persona>>,
    /// Shared slot the agent drains at end of turn. `Some(name)` means
    /// "apply this switch when the turn ends".
    pending: Arc<Mutex<Option<String>>>,
    /// The currently active persona name (shared with the agent). Used to
    /// look up transition rules — the *current* persona's `transitions`
    /// field controls which personas it can switch to and whether
    /// confirmation is required. `None` = no persona active (unrestricted).
    current_persona: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl SwitchPersona {
    pub fn new(
        personas: Arc<Vec<mew_personas::Persona>>,
        pending: Arc<Mutex<Option<String>>>,
        current_persona: Arc<tokio::sync::RwLock<Option<String>>>,
    ) -> Self {
        Self {
            personas,
            pending,
            current_persona,
        }
    }
}

#[async_trait]
impl Tool for SwitchPersona {
    fn name(&self) -> &str {
        "switch_persona"
    }

    fn description(&self) -> &str {
        "Queue a persona switch for the end of the current turn. The new \
         persona's system prompt, tool allowlist, and (if pinned) model \
         take effect after the model finishes its current response. Use \
         this when you realize the user's request fits a different \
         persona's specialty — for example switching from a code-writing \
         persona to a read-only researcher once you start investigating. \
         The user is shown the new model's identity and toolset via the \
         status bar after the switch is applied."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The persona to switch to. Must be a persona name visible to /persona. Use 'default' or 'none' to clear the active persona (clears immediately, no end-of-turn deferral)."
                    }
                },
                "required": ["name"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        // Mutating: a persona switch changes the toolset and possibly the
        // model the user is paying for. The PermissionEngine will prompt
        // the user before this tool runs unless they have a rule that
        // auto-allows it.
        Sensitivity::Mutating
    }

    async fn execute(&self, _ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing 'name' field".into()))?;

        // "default" / "none" mean "clear the persona" — that's an
        // idempotent, unambiguous action the user can also trigger via
        // the slash command. We don't defer the clear; the tool layer is
        // the wrong place to model that — if we wanted the same deferral
        // we could route it through the same event, but it's cleaner for
        // clearing to be immediate (and rare).
        if name == "default" || name == "none" {
            return Ok(ToolOutput {
                output: "use `/persona default` (or `none`) to clear — \
                         the slash command clears immediately; this tool \
                         is for switching to a different named persona."
                    .into(),
                error: String::new(),
                diff: None,
                metadata: None,
            });
        }

        let target = self
            .personas
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| {
                ToolError::InvalidInput(format!(
                    "unknown persona '{name}'. Use /persona (with no argument) to list available personas."
                ))
            })?;

        // Check transition rules: the *current* persona's `transitions`
        // field controls which personas it can switch to. If the current
        // persona has no transitions (None), any switch is allowed.
        let current_name = self.current_persona.read().await.clone();
        if let Some(ref current) = current_name {
            if let Some(current_persona) = self.personas.iter().find(|p| &p.name == current) {
                if let Some(ref rules) = current_persona.config.transitions {
                    if let Some(ref allowed) = rules.allowed {
                        if !allowed.iter().any(|a| a == name) {
                            return Err(ToolError::InvalidInput(format!(
                                "persona '{current}' cannot switch to '{name}'. \
                                 Allowed transitions: [{}]. \
                                 Use /persona to switch manually.",
                                allowed.join(", ")
                            )));
                        }
                    }
                    if rules.confirm {
                        // Queue the switch but signal that confirmation is
                        // needed. The main loop's drain path will show a
                        // confirm modal (same as the /persona slash command).
                        // For now, we queue it and include a note in the
                        // output; the main loop checks `confirm` when
                        // draining.
                        let mut slot = self.pending.lock().await;
                        *slot = Some(name.to_string());
                        return Ok(ToolOutput {
                            output: format!(
                                "queued switch to persona '{name}' for end of turn \
                                 (confirmation required)."
                            ),
                            error: String::new(),
                            diff: None,
                            metadata: None,
                        });
                    }
                }
            }
        }

        // Queue the switch. We over-write any earlier queued switch so the
        // most recent request wins. The turn loop drains the slot at end
        // of turn and emits PersonaSwitchRequested for the main loop.
        {
            let mut slot = self.pending.lock().await;
            *slot = Some(name.to_string());
        }

        // Build a short description of the change for the model. The
        // model's next turn will see the actual switch applied; this
        // message just acknowledges the queue.
        let mut changes = Vec::new();
        if let Some(ref m) = target.config.model {
            changes.push(format!("model → {m}"));
        }
        match target.config.tools.as_ref() {
            Some(v) if !v.is_empty() => {
                changes.push(format!("tools → [{}]", v.join(", ")));
            }
            Some(_) => changes.push("tools → (none)".into()),
            None => {}
        }
        if let Some(ref d) = target.config.tools_deny {
            if !d.is_empty() {
                changes.push(format!("deny → [{}]", d.join(", ")));
            }
        }
        match target.config.skills.as_ref() {
            Some(v) if !v.is_empty() => {
                changes.push(format!("skills → [{}]", v.join(", ")));
            }
            Some(_) => changes.push("skills → (none)".into()),
            None => {}
        }
        let change_summary = if changes.is_empty() {
            "(no model/toolset change)".to_string()
        } else {
            changes.join("; ")
        };

        Ok(ToolOutput {
            output: format!("queued switch to persona '{name}' for end of turn. {change_summary}."),
            error: String::new(),
            diff: None,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx() -> ToolCtx {
        ToolCtx::test_new(PathBuf::from("."))
    }

    /// Default current-persona slot: `None` (no persona active, so no
    /// transition rules apply — all switches allowed).
    fn no_current() -> Arc<tokio::sync::RwLock<Option<String>>> {
        Arc::new(tokio::sync::RwLock::new(None))
    }

    /// Current-persona slot set to `name`.
    fn current_is(name: &str) -> Arc<tokio::sync::RwLock<Option<String>>> {
        Arc::new(tokio::sync::RwLock::new(Some(name.to_string())))
    }

    fn persona(name: &str, model: Option<&str>) -> mew_personas::Persona {
        mew_personas::Persona {
            name: name.into(),
            description: "test".into(),
            body: "test body".into(),
            path: PathBuf::new(),
            config: mew_personas::PersonaConfig {
                model: model.map(String::from),
                tools: None,
                tools_deny: None,
                skills: None,
                ..Default::default()
            },
        }
    }

    /// Persona with transition rules: only `allowed` personas, optional
    /// `confirm`.
    fn persona_with_transitions(
        name: &str,
        allowed: Option<Vec<&str>>,
        confirm: bool,
    ) -> mew_personas::Persona {
        mew_personas::Persona {
            name: name.into(),
            description: "test".into(),
            body: "test body".into(),
            path: PathBuf::new(),
            config: mew_personas::PersonaConfig {
                transitions: Some(mew_personas::TransitionRules {
                    allowed: allowed.map(|v| v.into_iter().map(String::from).collect()),
                    confirm,
                }),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn test_switch_persona_queues_target() {
        let personas = Arc::new(vec![persona("researcher", Some("glm-4.5-air"))]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), no_current());

        let input = serde_json::json!({"name": "researcher"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert!(result.output.contains("queued"));
        assert!(result.output.contains("model → glm-4.5-air"));
        // Slot is set; the turn loop will drain it.
        let guard = pending.lock().await;
        assert_eq!(guard.as_deref(), Some("researcher"));
    }

    #[tokio::test]
    async fn test_switch_persona_rejects_unknown() {
        let personas = Arc::new(vec![persona("researcher", None)]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), no_current());

        let input = serde_json::json!({"name": "ghost"});
        let result = tool.execute(dummy_ctx(), input).await;
        assert!(result.is_err());
        // Nothing queued on failure.
        let guard = pending.lock().await;
        assert!(guard.is_none());
    }

    #[tokio::test]
    async fn test_switch_persona_default_directs_to_slash() {
        let personas = Arc::new(Vec::new());
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), no_current());

        // "default" doesn't queue a switch — the tool's success message
        // tells the model to use the slash command for clears.
        let input = serde_json::json!({"name": "default"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert!(result.output.contains("/persona default"));
        let guard = pending.lock().await;
        assert!(guard.is_none());
    }

    #[tokio::test]
    async fn test_switch_persona_overwrites_previous_queued() {
        let personas = Arc::new(vec![persona("a", None), persona("b", None)]);
        let pending = Arc::new(Mutex::new(Some("a".to_string())));
        let tool = SwitchPersona::new(personas, pending.clone(), no_current());

        // Second call overwrites the first; most recent wins.
        let input = serde_json::json!({"name": "b"});
        tool.execute(dummy_ctx(), input).await.unwrap();
        let guard = pending.lock().await;
        assert_eq!(guard.as_deref(), Some("b"));
    }

    #[test]
    fn test_switch_persona_sensitivity_is_mutating() {
        let tool = SwitchPersona::new(
            Arc::new(Vec::new()),
            Arc::new(Mutex::new(None)),
            no_current(),
        );
        assert_eq!(tool.sensitivity(), Sensitivity::Mutating);
        assert_eq!(tool.name(), "switch_persona");
    }

    #[tokio::test]
    async fn test_transition_blocks_disallowed_switch() {
        // Planner with empty allowed list — cannot switch to anything.
        let personas = Arc::new(vec![
            persona_with_transitions("planner", Some(vec![]), false),
            persona("builder", None),
        ]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), current_is("planner"));

        let input = serde_json::json!({"name": "builder"});
        let result = tool.execute(dummy_ctx(), input).await;
        assert!(result.is_err());
        let guard = pending.lock().await;
        assert!(guard.is_none());
    }

    #[tokio::test]
    async fn test_transition_allows_listed_switch() {
        // Builder can switch to planner and reviewer.
        let personas = Arc::new(vec![
            persona_with_transitions("builder", Some(vec!["planner", "reviewer"]), false),
            persona("planner", None),
            persona("reviewer", None),
        ]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), current_is("builder"));

        let input = serde_json::json!({"name": "planner"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert!(result.output.contains("queued"));
        let guard = pending.lock().await;
        assert_eq!(guard.as_deref(), Some("planner"));
    }

    #[tokio::test]
    async fn test_transition_confirm_adds_note() {
        let personas = Arc::new(vec![
            persona_with_transitions("builder", Some(vec!["planner"]), true),
            persona("planner", None),
        ]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), current_is("builder"));

        let input = serde_json::json!({"name": "planner"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert!(result.output.contains("confirmation required"));
        // The switch is still queued — the main loop shows the confirm modal.
        let guard = pending.lock().await;
        assert_eq!(guard.as_deref(), Some("planner"));
    }

    #[tokio::test]
    async fn test_no_transitions_means_unrestricted() {
        // Builder with no transitions field — can switch to anything.
        let personas = Arc::new(vec![persona("builder", None), persona("reviewer", None)]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), current_is("builder"));

        let input = serde_json::json!({"name": "reviewer"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert!(result.output.contains("queued"));
        assert!(!result.output.contains("confirmation"));
    }

    #[tokio::test]
    async fn test_no_current_persona_means_unrestricted() {
        // No persona active — transitions don't apply.
        let personas = Arc::new(vec![
            persona_with_transitions("planner", Some(vec![]), false),
            persona("builder", None),
        ]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone(), no_current());

        let input = serde_json::json!({"name": "builder"});
        let result = tool.execute(dummy_ctx(), input).await.unwrap();
        assert!(result.output.contains("queued"));
    }
}
