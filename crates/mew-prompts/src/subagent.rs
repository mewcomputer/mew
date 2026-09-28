//! Built-in subagent system prompts.
//!
//! These are the role bodies for the three subagents mew ships built-in:
//! `researcher`, `plan-reviewer`, and `coder`. User-defined subagents load
//! their bodies from `.mew/agents/*.md` files (handled by `mew-subagents`);
//! only the built-in bodies live here.
//!
//! Bodies are role content only. The runner composes every subagent's full
//! system prompt as the shared `system_prompts/subagent` base (base prompt +
//! subagent contract) followed by the def's body. Centralizing the role
//! bodies here means there's one place to look when you want to know "what
//! does the researcher subagent actually get told?" — and the
//! [`crate::inventory`] module can list them alongside every other prompt
//! the system sends.

/// All built-in subagent prompt bodies, paired with their subagent name.
/// Used by [`crate::inventory`] to enumerate the built-in prompts.
pub fn builtin_bodies() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "researcher",
            crate::vfs::read_builtin("subagents/researcher").unwrap_or(""),
        ),
        (
            "plan-reviewer",
            crate::vfs::read_builtin("subagents/plan-reviewer").unwrap_or(""),
        ),
        (
            "coder",
            crate::vfs::read_builtin("subagents/coder").unwrap_or(""),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{render, TemplateContext};

    #[test]
    fn subagent_base_renders_subagent_contract() {
        let base = crate::vfs::read_builtin("system_prompts/subagent")
            .expect("shared subagent base resource present");
        let ctx = TemplateContext {
            subagent_name: "scout".into(),
            cwd: "/tmp/proj".into(),
            ..Default::default()
        };
        let rendered = render(base, &ctx);
        assert!(
            rendered.contains("focused mew subagent named scout"),
            "base must resolve subagent_name, got: {rendered}"
        );
        assert!(rendered.contains("## Subagent"));
        assert!(rendered.contains("exit_tool"));
        assert!(rendered.contains("progress_update"));
        assert!(
            !rendered.contains("{{ "),
            "base must render cleanly, got raw template syntax"
        );
    }

    #[test]
    fn builtin_bodies_are_role_only() {
        for (name, body) in builtin_bodies() {
            assert!(
                !body.contains("## Subagent"),
                "{name} must not inline the shared subagent section"
            );
            assert!(
                !body.contains("transclude("),
                "{name} must not inline the base; the runner composes it"
            );
            assert!(
                !body.contains("exit_tool"),
                "{name} should leave the exit_tool contract to the shared base"
            );
        }
    }

    #[test]
    fn test_researcher_body_mentions_research_role() {
        let bodies = builtin_bodies();
        let body = bodies
            .iter()
            .find(|(n, _)| *n == "researcher")
            .map(|(_, b)| *b)
            .expect("researcher body present");
        assert!(body.contains("research assistant"));
        assert!(body.contains("Read"));
    }

    #[test]
    fn test_plan_reviewer_body_mentions_severity_rating() {
        let bodies = builtin_bodies();
        let body = bodies
            .iter()
            .find(|(n, _)| *n == "plan-reviewer")
            .map(|(_, b)| *b)
            .expect("plan-reviewer body present");
        assert!(body.contains("critical"));
        assert!(body.contains("high"));
    }

    #[test]
    fn test_coder_body_mentions_conventions() {
        let bodies = builtin_bodies();
        let body = bodies
            .iter()
            .find(|(n, _)| *n == "coder")
            .map(|(_, b)| *b)
            .expect("coder body present");
        assert!(body.contains("conventions"));
    }

    #[test]
    fn test_builtin_bodies_lists_all_three() {
        let bodies = builtin_bodies();
        let names: Vec<&str> = bodies.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["researcher", "plan-reviewer", "coder"]);
    }
}
