//! Persona body rendering.
//!
//! Persona bodies live on disk in `PERSONA.md` files (loaded by
//! `mew-personas`). When a persona's frontmatter sets `template: true`, the
//! body is rendered through [`minijinja`] with the following variables:
//!
//! - `supports_vision` (bool) — whether the active model supports image input
//! - `persona_name` (str) — the active persona's name
//! - `tools` (list of str) — tool names available to the model this turn
//!   (after applying the persona's `tools` allowlist and `tools_deny`
//!   denylist)
//! - `denied_tools` (list of str) — tools removed by the denylist
//!
//! Falls back to the raw body on any render error so a typo in the template
//! never bricks the persona.

use std::collections::HashSet;

use minijinja::context;

/// Render a persona body through minijinja using the given tool context.
/// Returns the raw body if rendering fails (with a `tracing::warn!`).
///
/// `all_tool_names` is the full registry of tool names available to the
/// agent. The function only consults the names — the actual `Tool` trait
/// objects aren't needed here, which keeps `mew-prompts` free of the
/// `mew-tools` dependency (avoiding a workspace cycle).
pub fn render_template(
    body: &str,
    persona_name: &str,
    supports_vision: bool,
    active_tool_names: &Option<HashSet<String>>,
    all_tool_names: &[String],
    denied_tool_names: &HashSet<String>,
) -> String {
    // Compute the effective tool list the model will see this turn:
    // start from the full registry, apply the allowlist if set, then
    // subtract the denylist.
    let effective: Vec<String> = all_tool_names
        .iter()
        .filter(|name| {
            let allowed = active_tool_names
                .as_ref()
                .is_none_or(|set| set.contains(*name));
            allowed && !denied_tool_names.contains(*name)
        })
        .cloned()
        .collect();

    let denied: Vec<String> = denied_tool_names.iter().cloned().collect();

    let ctx = context! {
        supports_vision => supports_vision,
        persona_name => persona_name,
        tools => effective,
        denied_tools => denied,
    };

    minijinja::Environment::new()
        .render_str(body, ctx)
        .unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                persona = %persona_name,
                "persona template render failed, falling back to raw body"
            );
            body.to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim body (no template syntax) returns unchanged.
    #[test]
    fn test_render_template_verbatim_body() {
        let body = "You are a helpful assistant.";
        let result = render_template(
            body,
            "default",
            false,
            &None,
            &[],
            &HashSet::new(),
        );
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
            &Some(HashSet::from(["read".into(), "write".into(), "bash".into()])),
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
        let result = render_template(
            body,
            "p",
            false,
            &None,
            &[],
            &HashSet::new(),
        );
        assert_eq!(result, body, "render failure must fall back to raw body");
    }
}
