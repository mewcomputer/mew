---
title: Personas
description: Switchable system prompts with model pinning and tool allowlists.
---

Personas are switchable system prompts that can pin a model, restrict tools,
and render templates dynamically.

## Built-in personas

Mew bundles two built-in personas for a two-phase workflow:

| Persona | Tools | Purpose |
|---------|-------|---------|
| `builder` | All tools | Default persona. Reads an approved plan when asked and applies edits step by step. |
| `planner` | `read`, `glob`, `grep`, `write_plan`, `edit_plan`, `ask_user_question`, `flag_important`, `subagent`, and `todo_*` | Investigates the codebase and writes a plan for review. |

Typical workflow:

1. Run `/persona planner` to enter planning mode.
2. Ask the planner to investigate and write a plan (written to `plans/` by
   default, or a fixed file configured via `plan_path`).
3. Run `/persona builder` to switch back to editing.
4. Ask the builder to implement the reviewed plan.

User-defined personas with the same name override the built-ins. Set
`default_persona` in `config.toml` to `"builder"` (default), `"planner"`, or
`"none"` / `"default"` to start without a persona.

## Discovery

Mew searches project directories (walked from the working directory up to the
Git root) before global directories. Earlier matches win on duplicate names.

**Project paths** (cwd up to Git root):

1. `.mew/personas/<name>/PERSONA.md`
2. `.opencode/personas/<name>/PERSONA.md`
3. `.claude/personas/<name>/PERSONA.md`
4. `.agents/personas/<name>/PERSONA.md`

**Global paths:**

5. `~/.config/mew/personas/<name>/PERSONA.md`
6. `~/.config/opencode/personas/<name>/PERSONA.md`
7. `~/.claude/personas/<name>/PERSONA.md`
8. `~/.agents/personas/<name>/PERSONA.md`

## Format

A persona is a Markdown file with optional YAML frontmatter followed by
instructions:

```markdown
---
name: researcher
description: Focused research assistant
mew:
  model: z-ai/glm-4.5-air
  tools:
    - read
    - bash
    - grep
    - glob
---

You are a research assistant. Focus on gathering information and
synthesizing findings. Be thorough but concise.
```

Mew prepends the persona body before the base system prompt (followed by context
files and `<available_skills>`) and rebuilds the system prompt from scratch on
every turn.

## Frontmatter fields

Frontmatter is optional; without it, Mew uses the directory name and an empty
description. When frontmatter is present, `name` must match the directory name
(1 to 64 lowercase letters, digits, or internal hyphens matching
`^[a-z0-9]+(-[a-z0-9]+)*$`).

| Field | Required | Description |
|-------|----------|-------------|
| `name` | When frontmatter is present | Persona identifier (must match directory name) |
| `description` | No | Short description shown in `/persona` |
| `mew.model` | No | Pin `provider/model` (or a bare model ID on the active provider) |
| `mew.fallback_models` | No | Ordered fallback model IDs if the pinned model is unavailable |
| `mew.tools` | No | Tool allowlist: `[]` (no tools), list of tool names/tags, or omitted (all tools) |
| `mew.tools_deny` | No | Tools excluded after applying `mew.tools` |
| `mew.skills` | No | Skill allowlist: `null` (all skills), list of names, or `[]` (hide all) |
| `mew.template` | No | Render the body with minijinja when `true` |
| `mew.autonomous_hint` | No | Guidance passed to the `auto` / `auto_plus` permission classifier |
| `mew.color` | No | Status-bar accent hex color (for example `"#3b6b3b"`) |

### `polytoken:` alias

Mew accepts `polytoken:` as an alias for `mew:` in frontmatter; when both appear,
`mew:` takes precedence:

```yaml
---
name: researcher
description: Read-only investigation
polytoken:
  model: z-ai/glm-4.5-air
  tools: [read, grep]
---
```

## Tool allowlisting

`mew.tools` accepts individual tool names and expansion tags:

- `tag!ALL`: every registered tool.
- `tag!ALL_MCP`: every connected MCP tool (`mcp__*`).
- `mcp__<server>`: every tool from a specific MCP server.

Omitting `mew.tools` enables all registered tools, while `[]` disables all
tools for a text-only persona. `mew.tools_deny` runs after allowlist expansion
to subtract specific tools:

```yaml
mew:
  tools_deny:
    - bash
    - write
```

## Model pinning

`mew.model` pins a `provider/model` pair (or a bare model ID on the active
provider) for that persona:

```yaml
mew:
  model: z-ai/glm-4.5-air
```

Switching to a persona with `mew.model` switches to that model; switching to a
persona without `mew.model` or clearing the persona with `/persona default` keeps
the current model.

## Templates

When `mew.template: true`, Mew renders the persona body through minijinja before
building the system prompt.

### Template variables

| Variable | Type | Description |
|----------|------|-------------|
| `supports_vision` | bool | Whether the active model supports image input |
| `persona_name` | str | Active persona name |
| `subagent_name` | str | Active subagent name (in subagent templates) |
| `skill_name` | str | Active skill name (in skill templates) |
| `model_id` | str | Active model ID (for example `deepseek-v4-flash`) |
| `provider_id` | str | Active provider ID (for example `deepseek`) |
| `model_variant` | str | Active thinking variant (for example `high`, or empty) |
| `session_id` | str | Session ID |
| `cwd` | str | Working directory |
| `current_date` | str | Current date in ISO 8601 (for example `2026-06-29`) |
| `tools` | list of str | Effective tool names after allowlist and denylist |
| `denied_tools` | list of str | Tool names excluded by `mew.tools_deny` |
| `skills` | list of str | Available skill names |
| `available_skills` | list of objects | Skills with `name` and `description` |
| `available_subagents` | list of objects | Subagents with `name` and `description` |
| `mcp_servers` | list of str | Connected MCP server names |
| `project_vars` | map | Project variables from `project_vars.yaml` (in `.mew/`, `.opencode/`, `.claude/`, or `.agents/`) |

### Template functions

| Function | Description |
|----------|-------------|
| `has_tool(name)` | True if `name` is in the effective tool list |
| `has_skill(name)` | True if `name` is an available skill |
| `has_mcp(name)` | True if `name` is in `mcp_servers` |
| `is_model_variant(variant)` | True for matching provider or family: `anthropic` (`umans`), `openai` (`opencode-zen`, `opencode-go`, `deepseek`, `z-ai`, `alibaba-token-plan`, `alibaba-token-plan-cn`), `deepseek`, `z-ai`, `umans`, `opencode`, or exact `provider_id` match |
| `transclude("mew://path")` | Inline a built-in VFS prompt resource |

Example template:

```markdown
---
name: adaptive
description: Adapts behavior based on model capabilities.
mew:
  template: true
---

{% if supports_vision %}
You can see images. Analyze screenshots before responding.
{% else %}
You cannot see images. Ask the user to describe any referenced image.
{% endif %}

Model: {{ model_id }} on {{ provider_id }}.
Available tools: {{ tools | join(", ") }}.
{% if has_tool("bash") %}You can run shell commands.{% endif %}
{% if denied_tools %}Denied: {{ denied_tools | join(", ") }}.{% endif %}
{% for s in available_skills %}- {{ s.name }}: {{ s.description }}
{% endfor %}
```

On template errors, Mew logs a warning and falls back to the raw persona body.

### Transclusion

Include built-in compiled prompt fragments with `transclude`:

```markdown
{{ transclude("mew://system_prompts/base") }}

You are {{ persona_name }}. Additional instructions here.
```

## Switching personas

- `/persona <name>`: opens a confirmation modal previewing the model and toolset
  changes before switching.
- `/persona default` or `/persona none`: clears the active persona and restores
  the full toolset immediately while keeping the active model.
- `/persona`: opens the persona picker showing each persona's name, active
  marker, and description.
