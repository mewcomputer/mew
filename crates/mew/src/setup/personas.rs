//! Persona switching helpers extracted from the main event loop.
//!
//! These functions handle persona display summaries and applying a
//! persona switch (state + provider/model swap + synthetic message).

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
