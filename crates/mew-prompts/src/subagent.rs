//! Built-in subagent system prompts.
//!
//! These are the system-prompt bodies for the three subagents mew ships
//! built-in: `researcher`, `reviewer`, and `coder`. User-defined subagents
//! load their bodies from `.mew/agents/*.md` files (handled by
//! `mew-subagents`); only the built-in bodies live here.
//!
//! Centralizing them here means there's one place to look when you want to
//! know "what does the researcher subagent actually get told?" — and the
//! [`crate::inventory`] module can list them alongside every other prompt
//! the system sends.

/// System prompt for the `researcher` subagent.
pub const RESEARCHER_BODY: &str = "You are a research assistant. Your job is to investigate the codebase and answer questions thoroughly. \
        Read files, search for patterns, and gather context before answering. \
        Be thorough but concise. Cite specific file paths and line numbers when referencing code.";

/// System prompt for the `reviewer` subagent.
pub const REVIEWER_BODY: &str = "You are a code reviewer. Examine the provided code or diff for: \
        bugs, security issues, performance problems, style violations, and missing error handling. \
        Be specific about what you find. Reference file paths and line numbers. \
        Rate severity as: critical, warning, or suggestion.";

/// System prompt for the `coder` subagent.
pub const CODER_BODY: &str = "You are a code implementation assistant. Write clean, idiomatic code that follows the project's existing conventions. \
        Read existing code to understand patterns before writing new code. \
        Make minimal, focused changes. Test your changes when possible.";

/// All built-in subagent prompt bodies, paired with their subagent name.
/// Used by [`crate::inventory`] to enumerate the built-in prompts.
pub fn builtin_bodies() -> Vec<(&'static str, &'static str)> {
    vec![
        ("researcher", RESEARCHER_BODY),
        ("reviewer", REVIEWER_BODY),
        ("coder", CODER_BODY),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_researcher_body_mentions_research_role() {
        assert!(RESEARCHER_BODY.contains("research assistant"));
        assert!(RESEARCHER_BODY.contains("Read"));
    }

    #[test]
    fn test_reviewer_body_mentions_severity_rating() {
        assert!(REVIEWER_BODY.contains("critical"));
        assert!(REVIEWER_BODY.contains("warning"));
    }

    #[test]
    fn test_coder_body_mentions_conventions() {
        assert!(CODER_BODY.contains("conventions"));
    }

    #[test]
    fn test_builtin_bodies_lists_all_three() {
        let bodies = builtin_bodies();
        let names: Vec<&str> = bodies.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["researcher", "reviewer", "coder"]);
    }
}
