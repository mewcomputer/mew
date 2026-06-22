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
}

impl SwitchPersona {
    pub fn new(
        personas: Arc<Vec<mew_personas::Persona>>,
        pending: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self { personas, pending }
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

    #[tokio::test]
    async fn test_switch_persona_queues_target() {
        let personas = Arc::new(vec![persona("researcher", Some("glm-4.5-air"))]);
        let pending = Arc::new(Mutex::new(None));
        let tool = SwitchPersona::new(personas, pending.clone());

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
        let tool = SwitchPersona::new(personas, pending.clone());

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
        let tool = SwitchPersona::new(personas, pending.clone());

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
        let tool = SwitchPersona::new(personas, pending.clone());

        // Second call overwrites the first; most recent wins.
        let input = serde_json::json!({"name": "b"});
        tool.execute(dummy_ctx(), input).await.unwrap();
        let guard = pending.lock().await;
        assert_eq!(guard.as_deref(), Some("b"));
    }

    #[test]
    fn test_switch_persona_sensitivity_is_mutating() {
        let tool = SwitchPersona::new(Arc::new(Vec::new()), Arc::new(Mutex::new(None)));
        assert_eq!(tool.sensitivity(), Sensitivity::Mutating);
        assert_eq!(tool.name(), "switch_persona");
    }
}
