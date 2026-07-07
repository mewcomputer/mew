//! `LocalTarget` — `CommandTarget` impl for standalone TUI mode.
//!
//! Wraps the `Agent` and implements all operations directly. Model/persona
//! switching uses the agent's methods plus config/catalog lookups.
//! `attach_session` returns `Unsupported` in standalone mode.

use mew_agent::AgentEvent;
use mew_catalog::Catalog;
use mew_config::Config;
use mew_hooks::PermissionMode;
use mew_message::Part;
use mew_personas::Persona;
use tokio::sync::mpsc::Receiver;

use crate::runtime::target::{CommandTarget, PersonaApplied, SwitchedModel, Unsupported};
use crate::{build_provider, resolve_reasoning};

/// The local (standalone) command target. Borrows the `Agent` and implements
/// all `CommandTarget` methods directly against it. Holds config/catalog
/// for model switching.
pub struct LocalTarget<'a> {
    pub agent: &'a mut mew_agent::Agent,
    pub cfg: Config,
    pub cat: Option<Catalog>,
    pub provider_id: String,
    pub raw: bool,
}

impl<'a> LocalTarget<'a> {
    pub fn new(
        agent: &'a mut mew_agent::Agent,
        cfg: Config,
        cat: Option<Catalog>,
        provider_id: String,
        raw: bool,
    ) -> Self {
        Self {
            agent,
            cfg,
            cat,
            provider_id,
            raw,
        }
    }
}

#[async_trait::async_trait]
impl<'a> CommandTarget for LocalTarget<'a> {
    fn prompt(&mut self, enriched: String, parts: Vec<Part>) -> Receiver<AgentEvent> {
        self.agent.run_with_parts(enriched, parts, None)
    }

    async fn intercept_user_input(&mut self, text: String) -> String {
        self.agent.dispatcher.on_user_input(text).await
    }

    async fn cancel(&mut self) {
        self.agent.cancel_token.cancel();
    }

    async fn clear(&mut self) -> Result<(), Unsupported> {
        self.agent.clear_context().await;
        Ok(())
    }

    async fn compact(&mut self) -> Result<(), Unsupported> {
        self.agent.force_compact().await;
        Ok(())
    }

    async fn todos(&mut self) -> Result<String, Unsupported> {
        let list = self.agent.todos.lock().await;
        Ok(list.render())
    }

    async fn switch_model(&mut self, spec: &str) -> Result<SwitchedModel, Unsupported> {
        let (new_provider_id, new_model_id) = if let Some(idx) = spec.find('/') {
            (spec[..idx].to_string(), spec[idx + 1..].to_string())
        } else {
            (self.provider_id.clone(), spec.to_string())
        };
        match build_provider(
            &self.cfg,
            self.cat.as_ref(),
            &new_provider_id,
            &new_model_id,
            self.raw,
        ) {
            Ok(new_provider) => {
                self.agent.provider = new_provider;
                self.agent.set_model_info(&new_model_id, &new_provider_id);
                // Update pricing from catalog
                if let Some(c) = &self.cat {
                    if let Some(m) = c.lookup(&new_model_id) {
                        self.agent.input_price = m.pricing.input;
                        self.agent.output_price = m.pricing.output;
                        self.agent.cache_read_price = m.pricing.cache_read;
                        self.agent.cache_write_price = m.pricing.cache_write;
                        self.agent.reasoning_price = m.pricing.reasoning;
                    }
                }
                Ok(SwitchedModel {
                    provider_id: new_provider_id.clone(),
                    model_id: new_model_id.clone(),
                    display: spec.to_string(),
                })
            }
            Err(e) => {
                tracing::warn!("failed to switch model: {}", e);
                Err(Unsupported("failed to build provider for model"))
            }
        }
    }

    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), Unsupported> {
        self.agent.set_permission_mode(mode);
        Ok(())
    }

    async fn set_thinking(&mut self, variant: &str) -> Result<(), Unsupported> {
        let model_id = &self.agent.model_id;
        if variant.is_empty() || variant == "off" || variant == "none" {
            self.agent.set_reasoning(None);
        } else {
            match resolve_reasoning(self.cat.as_ref(), model_id, Some(variant)) {
                Some(config) => {
                    self.agent.set_reasoning(Some(config));
                }
                None => {
                    return Err(Unsupported("unknown thinking variant for model"));
                }
            }
        }
        Ok(())
    }

    async fn attach_session(&mut self, _id: &str) -> Result<(), Unsupported> {
        Err(Unsupported("session switching is only available in daemon mode"))
    }

    async fn resume(&mut self, id: &str) -> Result<(), Unsupported> {
        match mew_session::Reader::load(id).await {
            Ok(msgs) => {
                self.agent.load_messages(msgs).await;
                let resumed_todos_path =
                    mew_session::session_dir().join(id).join("todos.json");
                if let Ok(list) = mew_agent::TodoList::load(&resumed_todos_path).await {
                    *self.agent.todos.lock().await = list;
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!("failed to load session {}: {}", id, e);
                Err(Unsupported("failed to load session"))
            }
        }
    }

    async fn rewind(&mut self, n: usize) -> Result<(), Unsupported> {
        let mut msgs = self.agent.messages.lock().await;
        if n < msgs.len() {
            msgs.truncate(n);
        }
        Ok(())
    }

    async fn switch_persona(
        &mut self,
        name: &str,
        personas: &[Persona],
    ) -> Result<PersonaApplied, Unsupported> {
        if name == "default" || name == "none" {
            self.agent.clear_persona();
            return Ok(PersonaApplied {
                name: "default".to_string(),
                pinned_model: None,
                display: "persona cleared (default)".to_string(),
            });
        }
        if let Some(persona) = personas.iter().find(|p| p.name == name) {
            let pinned_model = self.agent.apply_persona(persona);
            // If persona pins a model, switch to it
            if let Some(ref model_str) = pinned_model {
                let (new_provider_id, new_model_id) = if let Some(idx) = model_str.find('/') {
                    (model_str[..idx].to_string(), model_str[idx + 1..].to_string())
                } else {
                    (self.provider_id.clone(), model_str.clone())
                };
                if let Ok(new_provider) = build_provider(
                    &self.cfg,
                    self.cat.as_ref(),
                    &new_provider_id,
                    &new_model_id,
                    self.raw,
                ) {
                    self.agent.provider = new_provider;
                    self.agent.set_model_info(&new_model_id, &new_provider_id);
                    if let Some(c) = &self.cat {
                        if let Some(m) = c.lookup(&new_model_id) {
                            self.agent.input_price = m.pricing.input;
                            self.agent.output_price = m.pricing.output;
                            self.agent.cache_read_price = m.pricing.cache_read;
                            self.agent.cache_write_price = m.pricing.cache_write;
                            self.agent.reasoning_price = m.pricing.reasoning;
                        }
                    }
                }
            }
            Ok(PersonaApplied {
                name: persona.name.clone(),
                pinned_model: pinned_model.clone(),
                display: format!(
                    "switched to persona: {}{}",
                    persona.name,
                    if let Some(ref m) = pinned_model {
                        format!(" (model: {})", m)
                    } else {
                        String::new()
                    }
                ),
            })
        } else {
            Err(Unsupported("unknown persona"))
        }
    }

    async fn on_persona_change(&mut self, old: Option<&str>, new: &str) {
        self.agent.dispatcher.on_persona_change(old, new).await;
    }

    async fn plugin_command(&mut self, name: &str, args: &str) -> Result<String, Unsupported> {
        let disp = self.agent.dispatcher.clone();
        match disp.execute_slash_command(name, args).await {
            Some(result) => Ok(result),
            None => Ok(format!("unknown command: {}", name)),
        }
    }

    async fn cancel_subagent(&mut self, task_id: &str) -> Result<bool, Unsupported> {
        Ok(self.agent.cancel_subagent(task_id).await)
    }

    async fn yield_control(&mut self) -> Result<(), Unsupported> {
        Err(Unsupported("yield_control not implemented for local mode"))
    }
}
