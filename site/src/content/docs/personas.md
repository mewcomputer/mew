---
title: Personas
description: Switchable system prompts with model pinning and tool allowlists.
---

Personas are switchable system prompts that can pin a specific model and
restrict which tools are available. They're loaded at startup from
`PERSONA.md` files.

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

## Built-in personas

mew ships with two built-in personas: `planner` and `builder`. User-defined
personas with the same name override them. Both are always available even
without any persona files on disk.

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

## Frontmatter fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | Yes | Persona identifier (used in `/persona <name>`) |
| `description` | No | Short description shown in the persona list |
| `mew.model` | No | Pin a `provider/model` pair (overrides session default) |
| `mew.tools` | No | Tool allowlist: `[]` (no tools), `[read, bash]` (whitelist), or absent (all tools) |
| `mew.tools_deny` | No | Tools to exclude from the allowlist |
| `mew.skills` | No | Skill allowlist: `null` (all skills), `[skill1, skill2]` (whitelist), or `[]` (hide all) |
| `mew.template` | No | When `true`, render body as a minijinja template. Exposes `supports_vision`, `tools`, `has_tool(name)`, `persona_name` |

## Switching personas

In the TUI:

```
/persona researcher
```

This opens a confirm modal showing the model/toolset diff. The switch only
happens after you confirm. Clearing with `/persona default` bypasses the modal.

The system prompt is rebuilt from scratch every turn, so persona body text
is always injected fresh.
