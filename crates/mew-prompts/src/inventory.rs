//! Inventory of every prompt in the mew-prompts crate.
//!
//! Lets callers enumerate every prompt the system can send — to the LLM, to
//! the user, to a future classifier — in one place. Useful for docs, for
//! tools that audit "what does the system actually say?", and for tests
//! that want to assert no prompt was silently added or removed.

/// What kind of prompt this is. Drives the section heading in any docs or
/// inventory reports built from [`PromptSource`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// Sent to the model as part of the system message.
    System,
    /// Sent to the model as a user message (none currently, but reserved).
    User,
    /// Sent to a classifier LLM (the future Auto mode).
    Classifier,
}

impl PromptKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromptKind::System => "system",
            PromptKind::User => "user",
            PromptKind::Classifier => "classifier",
        }
    }
}

/// A single known prompt source — one row in the inventory.
#[derive(Debug, Clone)]
pub struct PromptSource {
    /// Stable identifier (e.g. `"base_system_prompt"`, `"subagent:researcher"`).
    pub id: &'static str,
    /// Which submodule of `mew-prompts` owns the prompt.
    pub location: &'static str,
    /// What kind of prompt this is.
    pub kind: PromptKind,
    /// Short human-readable description for docs.
    pub description: &'static str,
    /// Short preview of the prompt's first line or template (for docs).
    pub preview: &'static str,
}

/// Enumerate every prompt this crate knows about.
pub fn inventory() -> Vec<PromptSource> {
    let mut out = vec![
        PromptSource {
            id: "base_context_block",
            location: "mew_prompts::system",
            kind: PromptKind::System,
            description: "<context source=\"...\"> blocks from AGENTS.md / CLAUDE.md files.",
            preview: "<context source=\"/path/to/AGENTS.md\">\n...\n</context>",
        },
        PromptSource {
            id: "available_skills_block",
            location: "mew_prompts::skills",
            kind: PromptKind::System,
            description: "<available_skills> listing each skill's name and description.",
            preview: "<available_skills>\n  <skill>...</skill>\n</available_skills>",
        },
        PromptSource {
            id: "persona_body",
            location: "mew_prompts::persona",
            kind: PromptKind::System,
            description: "Active persona's markdown body, optionally rendered through minijinja.",
            preview: "You are {{ persona_name }}. Vision: {{ supports_vision }}.",
        },
        PromptSource {
            id: "classifier_permission_decision",
            location: "mew_prompts::classifier",
            kind: PromptKind::Classifier,
            description: "Permission-decision prompt for Auto mode (small LLM).",
            preview: "You are a permission classifier ... allow | deny | escalate",
        },
    ];
    for (name, body) in crate::subagent::builtin_bodies() {
        let preview = body.lines().next().unwrap_or("").trim();
        let id_static: &'static str = Box::leak(format!("subagent:{name}").into_boxed_str());
        let location_static: &'static str =
            Box::leak("mew_prompts::subagent".to_string().into_boxed_str());
        let description_static: &'static str = Box::leak(
            format!("Built-in system prompt for the `{name}` subagent.").into_boxed_str(),
        );
        out.push(PromptSource {
            id: id_static,
            location: location_static,
            kind: PromptKind::System,
            description: description_static,
            preview,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inventory_is_non_empty() {
        assert!(!inventory().is_empty());
    }

    #[test]
    fn test_inventory_has_unique_ids() {
        let inv = inventory();
        let mut ids: Vec<&str> = inv.iter().map(|p| p.id).collect();
        ids.sort();
        let original_len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "inventory ids must be unique");
    }

    #[test]
    fn test_inventory_lists_all_three_subagents() {
        let inv = inventory();
        let names: Vec<&str> = inv
            .iter()
            .filter(|p| p.id.starts_with("subagent:"))
            .map(|p| p.id)
            .collect();
        assert!(names.contains(&"subagent:researcher"));
        assert!(names.contains(&"subagent:reviewer"));
        assert!(names.contains(&"subagent:coder"));
    }

    #[test]
    fn test_inventory_includes_classifier() {
        let inv = inventory();
        assert!(inv
            .iter()
            .any(|p| p.id == "classifier_permission_decision" && p.kind == PromptKind::Classifier));
    }

    #[test]
    fn test_prompt_kind_as_str() {
        assert_eq!(PromptKind::System.as_str(), "system");
        assert_eq!(PromptKind::User.as_str(), "user");
        assert_eq!(PromptKind::Classifier.as_str(), "classifier");
    }
}
