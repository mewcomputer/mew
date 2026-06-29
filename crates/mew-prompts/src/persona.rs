//! Persona body rendering.
//!
//! Persona bodies live on disk in `PERSONA.md` files (loaded by
//! `mew-personas`). When a persona's frontmatter sets `template: true`, the
//! body is rendered through minijinja via [`crate::template`].
//!
//! This module is a thin wrapper that builds a [`TemplateContext`] from the
//! agent's state and delegates to [`crate::template::render`]. The shared
//! module also serves skills and subagents, so all three prompt types expose
//! the same variables and functions.

use std::collections::HashSet;

use crate::template::{render, TemplateContext};

/// Render a persona body through minijinja using the given tool context.
/// Returns the raw body if rendering fails (with a `tracing::warn!`).
///
/// This is the legacy entry point kept for backward compatibility. New
/// callers should use [`crate::template::render`] directly with a full
/// [`TemplateContext`].
pub fn render_template(
    body: &str,
    persona_name: &str,
    supports_vision: bool,
    active_tool_names: &Option<HashSet<String>>,
    all_tool_names: &[String],
    denied_tool_names: &HashSet<String>,
) -> String {
    let (tools, denied_tools) =
        TemplateContext::compute_tools(all_tool_names, active_tool_names, denied_tool_names);
    let ctx = TemplateContext {
        supports_vision,
        persona_name: persona_name.to_string(),
        tools,
        denied_tools,
        current_date: TemplateContext::today(),
        ..Default::default()
    };
    render(body, &ctx)
}

/// Render a persona body with a full [`TemplateContext`] that includes
/// model, provider, session, and cwd information. This is the preferred
/// entry point for callers that have access to the agent's full state.
pub fn render_with_context(body: &str, ctx: &TemplateContext) -> String {
    render(body, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim body (no template syntax) returns unchanged.
    #[test]
    fn test_render_template_verbatim_body() {
        let body = "You are a helpful assistant.";
        let result = render_template(body, "default", false, &None, &[], &HashSet::new());
        assert_eq!(result, body);
    }

    /// Template body is rendered with the available context vars.
    #[test]
    fn test_render_template_renders_vars() {
        let body = "You are {{ persona_name }}. Vision: {{ supports_vision }}. Tools: {{ tools | join(', ') }}.";
        let registry = vec!["read".into(), "grep".into()];
        let result = render_template(
            body,
            "researcher",
            true,
            &Some(HashSet::from(["read".into(), "grep".into()])),
            &registry,
            &HashSet::new(),
        );
        assert!(result.contains("You are researcher."));
        assert!(result.contains("Vision: true"));
        assert!(result.contains("read"));
        assert!(result.contains("grep"));
    }

    /// Denied tools are excluded from the `tools` list.
    #[test]
    fn test_render_template_excludes_denied() {
        let body = "{{ tools | join(',') }}";
        let registry = vec!["read".into(), "write".into(), "bash".into()];
        let result = render_template(
            body,
            "p",
            false,
            &Some(HashSet::from([
                "read".into(),
                "write".into(),
                "bash".into(),
            ])),
            &registry,
            &HashSet::from(["write".into()]),
        );
        assert!(!result.contains("write"));
        assert!(result.contains("read"));
        assert!(result.contains("bash"));
    }

    /// Invalid template syntax falls back to the raw body without crashing.
    #[test]
    fn test_render_template_falls_back_on_error() {
        let body = "unclosed {{ brace";
        let result = render_template(body, "p", false, &None, &[], &HashSet::new());
        assert_eq!(result, body, "render failure must fall back to raw body");
    }

    /// Transclude includes built-in VFS resources at render time.
    #[test]
    fn test_render_template_transclude() {
        let body = "{{ transclude(\"mew://system_prompts/base\") }}";
        let result = render_template(body, "p", false, &None, &[], &HashSet::new());
        assert!(
            result.contains("Save progress frequently"),
            "transclude must inline the base prompt; got: {result}"
        );
    }

    /// Transclude works without the mew:// scheme prefix.
    #[test]
    fn test_render_template_transclude_without_scheme() {
        let body = "{{ transclude(\"system_prompts/base\") }}";
        let result = render_template(body, "p", false, &None, &[], &HashSet::new());
        assert!(
            result.contains("Save progress frequently"),
            "transclude without scheme must still work; got: {result}"
        );
    }

    /// Transclude of a nonexistent path returns empty string (no crash).
    #[test]
    fn test_render_template_transclude_missing() {
        let body = "[{{ transclude(\"mew://nonexistent/path\") }}]";
        let result = render_template(body, "p", false, &None, &[], &HashSet::new());
        assert_eq!(
            result, "[]",
            "transclude of missing path must be empty; got: {result}"
        );
    }

    /// Transclude can be combined with other template variables.
    #[test]
    fn test_render_template_transclude_with_vars() {
        let body = "{{ transclude(\"mew://system_prompts/base\") }}\n\nYou are {{ persona_name }}.";
        let result = render_template(body, "researcher", false, &None, &[], &HashSet::new());
        assert!(result.contains("Save progress frequently"));
        assert!(result.contains("You are researcher."));
    }
}
