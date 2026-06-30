//! Base system prompt assembly.
//!
//! The base system prompt is composed from three fragments in this order:
//!
//! 1. **Context block** — `<context source="...">...</context>` for each
//!    `AGENTS.md` / `CLAUDE.md` file discovered from cwd up to git root.
//!    Built by [`mew_context::build_system_prompt`]; re-exported here as
//!    [`build_context`].
//! 2. **Skills block** — `<available_skills>...</available_skills>` listing
//!    the skills the model can invoke. Built by [`crate::skills::build_xml`].
//! 3. **Persona body** — the active persona's markdown body, optionally
//!    rendered through minijinja. Built by [`crate::persona::render_template`].
//!
//! The agent core glues these together and runs the result through the
//! `on_system_prompt` hook before sending it to the provider. This module
//! just re-exports the pieces and exposes [`assemble`] as a convenience
//! helper that joins them in standard order.

use mew_context::{build_system_prompt as build_context_impl, File};

/// Re-export of [`mew_context::build_system_prompt`] so callers can stay
/// within the `mew-prompts` namespace.
pub use mew_context::build_system_prompt as build_context;

/// Assemble the full base system prompt from context files, the skills XML
/// block, and the (already-rendered) persona body.
///
/// The order is fixed: context → skills → persona body. Hooks get the
/// assembled string and may mutate it.
pub fn assemble(context_files: &[File], skills_xml: &str, persona_body: &str) -> String {
    let mut out = String::new();
    if !context_files.is_empty() {
        out.push_str(&build_context_impl(context_files));
    }
    if !skills_xml.is_empty() {
        out.push_str(skills_xml);
    }
    if !persona_body.is_empty() {
        out.push_str(persona_body);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_assemble_empty_inputs_returns_empty() {
        assert_eq!(assemble(&[], "", ""), "");
    }

    #[test]
    fn test_assemble_context_only() {
        let files = vec![File {
            path: PathBuf::from("/tmp/AGENTS.md"),
            content: "hello".into(),
            template: false,
        }];
        let out = assemble(&files, "", "");
        assert!(out.contains("<context"));
        assert!(out.contains("hello"));
    }

    #[test]
    fn test_assemble_order_is_context_skills_persona() {
        let files = vec![File {
            path: PathBuf::from("/tmp/AGENTS.md"),
            content: "CTX".into(),
            template: false,
        }];
        let skills = "<available_skills>\nSKILLS\n</available_skills>\n";
        let persona = "PERSONA_BODY";
        let out = assemble(&files, skills, persona);
        let ctx_idx = out.find("CTX").expect("context in output");
        let skills_idx = out.find("SKILLS").expect("skills in output");
        let persona_idx = out.find("PERSONA_BODY").expect("persona in output");
        assert!(
            ctx_idx < skills_idx && skills_idx < persona_idx,
            "expected order context < skills < persona, got indices {ctx_idx} {skills_idx} {persona_idx}"
        );
    }

    #[test]
    fn test_assemble_skips_empty_fragments() {
        let out = assemble(&[], "", "");
        assert_eq!(out, "");
        let out = assemble(&[], "<available_skills>\nX\n</available_skills>\n", "");
        assert!(out.contains("X"));
    }
}
