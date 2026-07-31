//! Shared template context and rendering for personas, skills, and subagents.
//!
//! All three prompt sources (persona bodies, skill bodies, subagent bodies)
//! can opt into minijinja templating via `template: true` in their frontmatter.
//! This module provides the shared context struct and rendering function so
//! they all expose the same variables and use the same `transclude` function.
//!
//! ## Variables
//!
//! - `supports_vision` (bool) — whether the active model supports image input
//! - `persona_name` (str) — the active persona's name (empty for subagent/skill)
//! - `subagent_name` (str) — the subagent's name (empty for persona/skill)
//! - `skill_name` (str) — the skill's name (empty for persona/subagent)
//! - `model_id` (str) — the active model ID
//! - `provider_id` (str) — the active provider ID
//! - `model_variant` (str) — the active thinking variant (e.g. "high", empty if none)
//! - `session_id` (str) — the session ID
//! - `cwd` (str) — the current working directory
//! - `current_date` (str) — the current date in ISO 8601 (e.g. "2026-06-29")
//! - `tools` (list of str) — tool names available to the model this turn
//! - `denied_tools` (list of str) — tools removed by the denylist
//! - `skills` (list of str) — skill names available this turn
//! - `mcp_servers` (list of str) — connected MCP server names
//! - `project_vars` (map of str→str) — project-local variables from `.mew/project_vars.yaml`
//!
//! ## Functions
//!
//! - `transclude("mew://path")` — inline a built-in VFS resource, rendered as
//!   a template with the current context. Also accepts bare paths without the
//!   `mew://` prefix.
//! - `has_tool(name)` — returns true if `name` is in the effective tool list.
//! - `has_skill(name)` — returns true if `name` is an available skill.
//! - `has_mcp(name)` — returns true if `name` is a connected MCP server.
//! - `is_model_variant(variant)` — returns true if the active provider matches
//!   a known variant name. Recognizes: "anthropic" (umans), "openai"
//!   (opencode-zen, opencode-go, deepseek, z-ai), "deepseek", "z-ai", "umans",
//!   "opencode" (any opencode-*). Falls back to exact provider ID match.

use std::collections::HashSet;

use minijinja::context;
use minijinja::value::Value as MjValue;

/// Context for rendering prompt templates.
///
/// Carries everything a persona/skill/subagent body might need to adapt its
/// content. Fields are set by the agent before rendering; callers only set
/// the ones relevant to their prompt type (e.g. `persona_name` for personas,
/// `subagent_name` for subagents).
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    pub supports_vision: bool,
    pub persona_name: String,
    pub subagent_name: String,
    pub skill_name: String,
    pub model_id: String,
    pub provider_id: String,
    pub model_variant: String,
    pub session_id: String,
    pub cwd: String,
    pub current_date: String,
    pub tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub skills: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub project_vars: std::collections::HashMap<String, String>,
    /// Subagent defs available to dispatch (name + description). Empty when
    /// no subagents are configured.
    pub available_subagents: Vec<SubagentInfo>,
}

/// Minimal info about a subagent for template rendering.
#[derive(Debug, Clone, Default)]
pub struct SubagentInfo {
    pub name: String,
    pub description: String,
}

impl TemplateContext {
    /// Compute the effective tool list from the full registry, allowlist, and
    /// denylist. This mirrors the logic the agent uses to build the actual
    /// tool set for the turn.
    pub fn compute_tools(
        all_tool_names: &[String],
        active_tool_names: &Option<HashSet<String>>,
        denied_tool_names: &HashSet<String>,
    ) -> (Vec<String>, Vec<String>) {
        let mut effective: Vec<String> = all_tool_names
            .iter()
            .filter(|name| {
                let allowed = active_tool_names
                    .as_ref()
                    .is_none_or(|set| set.contains(*name));
                allowed && !denied_tool_names.contains(*name)
            })
            .cloned()
            .collect();
        effective.sort_unstable();
        let mut denied: Vec<String> = denied_tool_names.iter().cloned().collect();
        denied.sort_unstable();
        (effective, denied)
    }

    /// Get today's date as an ISO 8601 string (e.g. "2026-06-29").
    pub fn today() -> String {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    }
}

/// Render a template body through minijinja using the given context.
/// Returns the raw body if rendering fails (with a `tracing::warn!`).
pub fn render(body: &str, ctx: &TemplateContext) -> String {
    let tools_val: Vec<MjValue> = ctx
        .tools
        .iter()
        .map(|s| MjValue::from(s.as_str()))
        .collect();
    let denied_val: Vec<MjValue> = ctx
        .denied_tools
        .iter()
        .map(|s| MjValue::from(s.as_str()))
        .collect();
    let skills_val: Vec<MjValue> = ctx
        .skills
        .iter()
        .map(|s| MjValue::from(s.as_str()))
        .collect();
    let mcp_val: Vec<MjValue> = ctx
        .mcp_servers
        .iter()
        .map(|s| MjValue::from(s.as_str()))
        .collect();

    let subagents_val: Vec<MjValue> = ctx
        .available_subagents
        .iter()
        .map(|s| {
            let ctx = minijinja::context! {
                name => s.name.as_str(),
                description => s.description.as_str(),
            };
            ctx
        })
        .collect();

    let env_ctx = context! {
        supports_vision => ctx.supports_vision,
        persona_name => &ctx.persona_name,
        subagent_name => &ctx.subagent_name,
        skill_name => &ctx.skill_name,
        model_id => &ctx.model_id,
        provider_id => &ctx.provider_id,
        model_variant => &ctx.model_variant,
        session_id => &ctx.session_id,
        cwd => &ctx.cwd,
        current_date => &ctx.current_date,
        tools => tools_val,
        denied_tools => denied_val,
        skills => skills_val,
        mcp_servers => mcp_val,
        project_vars => &ctx.project_vars,
        available_subagents => subagents_val,
    };

    let mut env = minijinja::Environment::new();

    // transclude: inline a built-in VFS resource, rendering any nested Jinja
    // directives in the included content against the same context.
    // {{ transclude("mew://system_prompts/base") }} or {{ transclude("system_prompts/base") }}
    let transclude_ctx = env_ctx.clone();
    env.add_function(
        "transclude",
        move |state: &minijinja::State, path: String| -> String {
            let stripped = path.strip_prefix("mew://").unwrap_or(&path);
            let content = crate::vfs::read_builtin(stripped).unwrap_or("");
            if content.is_empty() {
                return String::new();
            }
            state
                .env()
                .render_str(content, transclude_ctx.clone())
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        path = %stripped,
                        "transclude render failed, returning raw content"
                    );
                    content.to_string()
                })
        },
    );

    // has_tool: check if a tool is in the effective list.
    // {% if has_tool("bash") %}You can run commands.{% endif %}
    let tools_set: HashSet<String> = ctx.tools.iter().cloned().collect();
    env.add_function("has_tool", move |name: String| -> bool {
        tools_set.contains(&name)
    });

    // has_skill: check if a skill is available.
    // {% if has_skill("release-checklist") %}...{% endif %}
    let skills_set: HashSet<String> = ctx.skills.iter().cloned().collect();
    env.add_function("has_skill", move |name: String| -> bool {
        skills_set.contains(&name)
    });

    // has_mcp: check if an MCP server is connected.
    // {% if has_mcp("filesystem") %}...{% endif %}
    let mcp_set: HashSet<String> = ctx.mcp_servers.iter().cloned().collect();
    env.add_function("has_mcp", move |name: String| -> bool {
        mcp_set.contains(&name)
    });

    // is_model_variant: check if the active provider matches a known name.
    // {% if is_model_variant("anthropic") %}...{% endif %}
    // Recognizes: "anthropic", "openai", "deepseek", "z-ai", "umans", "opencode"
    let provider_id = ctx.provider_id.clone();
    env.add_function("is_model_variant", move |variant: String| -> bool {
        let v = variant.to_lowercase();
        let p = provider_id.to_lowercase();
        match v.as_str() {
            "anthropic" => p == "umans",
            "openai" => matches!(
                p.as_str(),
                "opencode-zen" | "opencode-go" | "deepseek" | "z-ai"
            ),
            "deepseek" => p == "deepseek",
            "z-ai" => p == "z-ai",
            "umans" => p == "umans",
            "opencode" => p.starts_with("opencode"),
            _ => p == v,
        }
    });

    env.render_str(body, env_ctx).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "template render failed, falling back to raw body");
        body.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> TemplateContext {
        TemplateContext {
            persona_name: "researcher".into(),
            model_id: "deepseek-v4-flash".into(),
            provider_id: "deepseek".into(),
            session_id: "01JXK4M3PQ".into(),
            cwd: "/home/user/project".into(),
            current_date: "2026-06-29".into(),
            tools: vec!["read".into(), "grep".into(), "glob".into()],
            ..Default::default()
        }
    }

    #[test]
    fn test_render_verbatim_body() {
        let body = "You are a helpful assistant.";
        let result = render(body, &ctx());
        assert_eq!(result, body);
    }

    #[test]
    fn test_render_renders_vars() {
        let body = "Model: {{ model_id }}. Provider: {{ provider_id }}. Session: {{ session_id }}. CWD: {{ cwd }}. Date: {{ current_date }}.";
        let result = render(body, &ctx());
        assert!(result.contains("Model: deepseek-v4-flash."));
        assert!(result.contains("Provider: deepseek."));
        assert!(result.contains("Session: 01JXK4M3PQ."));
        assert!(result.contains("CWD: /home/user/project."));
        assert!(result.contains("Date: 2026-06-29."));
    }

    #[test]
    fn test_render_persona_name() {
        let body = "You are {{ persona_name }}.";
        let result = render(body, &ctx());
        assert_eq!(result, "You are researcher.");
    }

    #[test]
    fn test_render_tools_list() {
        let body = "Tools: {{ tools | join(', ') }}.";
        let result = render(body, &ctx());
        assert_eq!(result, "Tools: read, grep, glob.");
    }

    #[test]
    fn test_compute_tools_sorts_effective_and_denied_names() {
        let all = vec!["write".into(), "read".into(), "bash".into()];
        let denied = ["read".to_string()].into_iter().collect();

        let (effective, denied) = TemplateContext::compute_tools(&all, &None, &denied);

        assert_eq!(effective, vec!["bash", "write"]);
        assert_eq!(denied, vec!["read"]);
    }

    #[test]
    fn test_render_has_tool() {
        let body = "{% if has_tool(\"read\") %}has read{% endif %}{% if has_tool(\"bash\") %}has bash{% endif %}";
        let result = render(body, &ctx());
        assert_eq!(result, "has read");
    }

    #[test]
    fn test_render_falls_back_on_error() {
        let body = "unclosed {{ brace";
        let result = render(body, &ctx());
        assert_eq!(result, body, "render failure must fall back to raw body");
    }

    #[test]
    fn test_render_transclude() {
        let body = "{{ transclude(\"mew://system_prompts/base\") }}";
        let result = render(body, &ctx());
        assert!(
            result.contains("Treat the current prompt context as authoritative"),
            "transclude must inline the base prompt; got: {result}"
        );
    }

    #[test]
    fn test_render_transclude_without_scheme() {
        let body = "{{ transclude(\"system_prompts/base\") }}";
        let result = render(body, &ctx());
        assert!(!result.is_empty());
    }

    #[test]
    fn base_prompt_is_independent_of_runtime_capabilities() {
        let mut first = ctx();
        first.provider_id = "anthropic".into();
        first.tools = vec!["read".into(), "skill".into()];
        first.skills = vec!["release-checklist".into()];
        first.mcp_servers = vec!["filesystem".into()];
        first.available_subagents = vec![SubagentInfo {
            name: "researcher".into(),
            description: "researches a bounded question".into(),
        }];

        let mut second = first.clone();
        second.tools = vec!["bash".into(), "grep".into(), "mcp__github__search".into()];
        second.skills = vec!["code-review".into(), "release-checklist".into()];
        second.mcp_servers = vec!["github".into(), "filesystem".into()];
        second.available_subagents = vec![SubagentInfo {
            name: "implementer".into(),
            description: "implements a bounded change".into(),
        }];

        let base = crate::vfs::read_builtin("system_prompts/base").unwrap();
        assert_eq!(render(base, &first), render(base, &second));
    }

    #[test]
    fn test_render_subagent_name() {
        let c = TemplateContext {
            subagent_name: "researcher".into(),
            ..Default::default()
        };
        let body = "You are subagent {{ subagent_name }}.";
        let result = render(body, &c);
        assert_eq!(result, "You are subagent researcher.");
    }

    #[test]
    fn test_render_skill_name() {
        let c = TemplateContext {
            skill_name: "release-checklist".into(),
            ..Default::default()
        };
        let body = "Loading skill: {{ skill_name }}";
        let result = render(body, &c);
        assert_eq!(result, "Loading skill: release-checklist");
    }

    #[test]
    fn test_render_has_skill() {
        let c = TemplateContext {
            skills: vec!["release-checklist".into(), "code-review".into()],
            ..Default::default()
        };
        let body = "{% if has_skill(\"release-checklist\") %}has{% else %}missing{% endif %}";
        assert_eq!(render(body, &c), "has");
        let body = "{% if has_skill(\"nonexistent\") %}has{% else %}missing{% endif %}";
        assert_eq!(render(body, &c), "missing");
    }

    #[test]
    fn test_render_has_mcp() {
        let c = TemplateContext {
            mcp_servers: vec!["filesystem".into()],
            ..Default::default()
        };
        let body = "{% if has_mcp(\"filesystem\") %}has{% else %}missing{% endif %}";
        assert_eq!(render(body, &c), "has");
        let body = "{% if has_mcp(\"github\") %}has{% else %}missing{% endif %}";
        assert_eq!(render(body, &c), "missing");
    }

    #[test]
    fn test_render_is_model_variant() {
        let c = TemplateContext {
            provider_id: "deepseek".into(),
            ..Default::default()
        };
        assert_eq!(
            render("{% if is_model_variant(\"deepseek\") %}yes{% endif %}", &c),
            "yes"
        );
        assert_eq!(
            render("{% if is_model_variant(\"openai\") %}yes{% endif %}", &c),
            "yes"
        );
        assert_eq!(
            render("{% if is_model_variant(\"anthropic\") %}yes{% endif %}", &c),
            ""
        );
    }

    #[test]
    fn test_render_is_model_variant_anthropic() {
        let c = TemplateContext {
            provider_id: "umans".into(),
            ..Default::default()
        };
        assert_eq!(
            render("{% if is_model_variant(\"anthropic\") %}yes{% endif %}", &c),
            "yes"
        );
        assert_eq!(
            render("{% if is_model_variant(\"umans\") %}yes{% endif %}", &c),
            "yes"
        );
    }

    #[test]
    fn test_render_project_vars() {
        let mut vars = std::collections::HashMap::new();
        vars.insert("team".into(), "platform".into());
        vars.insert("channel".into(), "#eng-platform".into());
        let c = TemplateContext {
            project_vars: vars,
            ..Default::default()
        };
        let body = "Team: {{ project_vars.team }}. Channel: {{ project_vars.channel }}.";
        let result = render(body, &c);
        assert_eq!(result, "Team: platform. Channel: #eng-platform.");
    }
}
