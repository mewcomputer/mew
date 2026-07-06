//! Built-in subagent system prompts.
//!
//! These are the system-prompt bodies for the three subagents mew ships
//! built-in: `researcher`, `plan-reviewer`, and `coder`. User-defined
//! subagents load their bodies from `.mew/agents/*.md` files (handled by
//! `mew-subagents`); only the built-in bodies live here.
//!
//! Centralizing them here means there's one place to look when you want to
//! know "what does the researcher subagent actually get told?" — and the
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
