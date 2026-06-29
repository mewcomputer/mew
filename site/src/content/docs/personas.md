---
title: Personas
description: Switchable system prompts with model pinning and tool allowlists.
---

Personas are switchable system prompts. Each persona can pin a specific
model, restrict which tools are available, and optionally use a template
for dynamic prompt generation. They let you switch the agent's behavior
without rewriting your context file.

## Built-in personas

mew ships with two built-in personas designed for a two-phase workflow:

| Persona | Tools | Purpose |
|---------|-------|---------|
| `builder` | All tools | The default. Reads a plan from `PLAN.md` and executes it step by step. |
| `planner` | Read-only + write/edit/flag/todos | Investigates the codebase, writes a plan to `PLAN.md`, flags it important, and hands off to the builder. No bash or dangerous tools. |

The intended flow:

1. `/persona planner` to start in planning mode
2. The planner investigates, writes `PLAN.md`, and flags it
3. `/persona builder` to switch to execution
4. The builder reads the plan and works through it

User-defined personas with the same name override the built-ins. Both are
always available even without any persona files on disk.

`default_persona` in config controls which one loads at startup
(defaults to `"builder"`). Set it to `"planner"` to start in planning
mode, or `"none"`/`"default"` to start without a persona.

## Discovery

Personas are searched in project-scoped and global-scoped directories.
Project paths are walked from cwd up to the git root. Earlier paths win
on duplicate names.

**Project paths** (walked cwd to git root):

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

A persona is a YAML frontmatter block followed by markdown body text:

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

The body becomes the system prompt. The system prompt is rebuilt from
scratch every turn, so the body text is always injected fresh.

## Frontmatter fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Persona identifier (used in `/persona <name>`) |
| `description` | no | Short description shown in the persona list |
| `mew.model` | no | Pin a `provider/model` pair (overrides session default) |
| `mew.tools` | no | Tool allowlist: `[]` (no tools), `[read, bash]` (whitelist), or absent (all tools) |
| `mew.tools_deny` | no | Tools to exclude from the allowlist |
| `mew.skills` | no | Skill allowlist: `null` (all skills), `[skill1, skill2]` (whitelist), or `[]` (hide all) |
| `mew.template` | no | When `true`, render body as a minijinja template |

## Tool allowlisting

`mew.tools` controls which tools the model can call. The allowlist
applies on top of the full tool registry:

- Absent: all registered tools available
- `[read, bash, grep, glob]`: only these four tools available
- `[]`: no tools available (text-only conversation)

`mew.tools_deny` removes specific tools from the available set, applied
after the allowlist. This is useful when you want "all tools except X":

```yaml
mew:
  tools_deny:
    - bash
    - write
```

## Model pinning

`mew.model` pins a `provider/model` pair that overrides the session
default. This lets a persona use a different model without changing the
session-wide setting:

```yaml
mew:
  model: z-ai/glm-4.5-air
```

When you switch to this persona, the model changes. When you switch back
or to another persona, the model changes again.

## Templates

When `mew.template: true`, the body is rendered through minijinja before
being used as the system prompt. Four variables are available:

| Variable | Type | Description |
|----------|------|-------------|
| `supports_vision` | bool | Whether the active model supports image input |
| `persona_name` | str | The active persona's name |
| `tools` | list of str | Tool names available this turn (after allowlist + denylist) |
| `denied_tools` | list of str | Tools removed by the denylist |

Example:

```markdown
---
name: adaptive
description: Adapts behavior based on model capabilities.
mew:
  template: true
---

{% if supports_vision %}
You can see images. When the user shares a screenshot, analyze it
before responding.
{% else %}
You cannot see images. If the user references an image, ask them to
describe it.
{% endif %}

Available tools: {{ tools | join(", ") }}.
{% if denied_tools %}Denied: {{ denied_tools | join(", ") }}.{% endif %}
```

If rendering fails (syntax error, missing variable), mew falls back to
the raw body and logs a warning.

### Transclusion

Templates can include built-in prompt fragments using the `transclude`
function:

```markdown
{{ transclude("mew://system_prompts/base") }}

You are {{ persona_name }}. Additional instructions here.
```

This pulls in shared prompt content that mew bundles at compile time.

## Switching personas

In the TUI:

```
/persona researcher
```

This opens a confirm modal showing the model and toolset diff between
the current and new persona. The switch only happens after you confirm.

```
/persona default
```

Clears the active persona and returns to the session default model and
full toolset. This bypasses the confirm modal.

```
/persona
```

Lists all available personas with their descriptions and whether they're
built-in or user-defined.
