//! Persona switching helpers extracted from the main event loop.
//!
//! These functions handle persona display summaries and applying a
//! persona switch (state + provider/model swap + synthetic message).

use mew_catalog::Catalog;
use mew_config::Config;

/// Build a display-only summary of a persona for the confirm modal.
pub(crate) fn persona_summary(p: &mew_personas::Persona) -> mew_tui::app::PersonaSummary {
    mew_tui::app::PersonaSummary {
        name: p.name.clone(),
        description: p.description.clone(),
        model: p.config.model.clone(),
        tools: p.config.tools.clone(),
        tools_deny: p.config.tools_deny.clone(),
        skills: p.config.skills.clone(),
        color: p.config.color.clone(),
    }
}

/// Apply a persona switch: set the agent's persona state, swap the
/// provider/model if the persona pins one, and push a synthetic message
/// describing what changed. Factored out of the slash-command handler so
/// the confirm modal can reuse it after the user accepts the diff.
pub(crate) fn apply_persona_switch(
    agent: &mut mew_agent::Agent,
    app: &mut mew_tui::App,
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    raw: bool,
    persona: &mew_personas::Persona,
) {
    let pinned_model = agent.apply_persona(persona);
    app.active_persona = Some(persona.name.clone());
    app.active_persona_color = persona.config.color.clone();
    if let Some(ref model_str) = pinned_model {
        let (new_provider_id, new_model_id) =
            crate::setup::providers::split_provider_model(model_str, provider_id);
        match crate::setup::providers::build_provider(
            cfg,
            cat,
            new_provider_id.as_str(),
            new_model_id.as_str(),
            raw,
        ) {
            Ok(new_provider) => {
                agent.provider = new_provider;
                app.status.model = new_model_id.to_string();
                app.status.provider = new_provider_id.to_string();
                if let Some(c) = cat {
                    app.status.context_window = c.context_window(&new_model_id) as u32;
                    crate::setup::agent::apply_catalog_pricing(agent, cat, &new_model_id);
                }
            }
            Err(e) => {
                tracing::warn!("persona model pin failed: {}", e);
            }
        }
    }
    app.push_synthetic_message(format!(
        "switched to persona: {}{}",
        persona.name,
        if let Some(ref m) = pinned_model {
            format!(" (model: {})", m)
        } else {
            String::new()
        }
    ));
}

/// If the agent's `switch_persona` tool queued a switch and the turn has
/// ended, the TUI receives `AgentEvent::PersonaSwitchRequested` and
/// stashes the name in `app.pending_persona_switch_apply`. This helper
/// drains that field and applies the switch using the same path as the
/// slash-command confirm modal. Called from the main event loop after
/// every agent event.
pub(crate) async fn drain_pending_persona_switch(
    agent: &mut mew_agent::Agent,
    app: &mut mew_tui::App,
    personas: &[mew_personas::Persona],
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    raw: bool,
) {
    let Some(name) = app.pending_persona_switch_apply.take() else {
        return;
    };
    if let Some(persona) = personas.iter().find(|p| p.name == name) {
        // Check if the *current* persona's transition rules require
        // confirmation. If so, open the confirm modal instead of
        // applying the switch directly. The user confirms via the
        // PersonaSwitchConfirmed action, which calls apply_persona_switch.
        let needs_confirm = app
            .active_persona
            .as_ref()
            .and_then(|cur| personas.iter().find(|p| &p.name == cur))
            .and_then(|p| p.config.transitions.as_ref())
            .is_some_and(|t| t.confirm);

        if needs_confirm {
            let target = persona_summary(persona);
            let current = app
                .active_persona
                .as_ref()
                .and_then(|cur_name| personas.iter().find(|p| &p.name == cur_name))
                .map(persona_summary);
            app.request_persona_switch_confirm(target, current);
            return;
        }

        let old = app.active_persona.clone();
        apply_persona_switch(agent, app, cfg, cat, provider_id, raw, persona);
        agent
            .dispatcher
            .on_persona_change(old.as_deref(), &name)
            .await;
    } else {
        tracing::warn!(
            name = %name,
            "PersonaSwitchRequested for unknown persona; ignoring"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_summary_maps_all_fields() {
        let persona = mew_personas::Persona {
            name: "test-persona".into(),
            description: "A test persona".into(),
            body: "body text".into(),
            path: std::path::PathBuf::new(),
            config: mew_personas::PersonaConfig {
                model: Some("z-ai/glm-4.5-air".into()),
                tools: Some(vec!["read".into(), "bash".into()]),
                tools_deny: Some(vec!["write".into()]),
                skills: Some(vec!["coding".into()]),
                color: Some("#ff0000".into()),
                ..Default::default()
            },
        };

        let summary = persona_summary(&persona);
        assert_eq!(summary.name, "test-persona");
        assert_eq!(summary.description, "A test persona");
        assert_eq!(summary.model, Some("z-ai/glm-4.5-air".into()));
        assert_eq!(summary.tools, Some(vec!["read".into(), "bash".into()]));
        assert_eq!(summary.tools_deny, Some(vec!["write".into()]));
        assert_eq!(summary.skills, Some(vec!["coding".into()]));
        assert_eq!(summary.color, Some("#ff0000".into()));
    }
}
