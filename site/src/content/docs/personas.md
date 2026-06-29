---
title: Personas
description: Switchable system prompts with model pinning and tool allowlists.
---

Personas are switchable system prompts that can pin a specific model and
restrict which tools are available. They're loaded at startup from
`PERSONA.md` files.

## Discovery

Personas are discovered from (earlier wins on duplicate name):

1. `<cwd→git-root>/.mew/personas/<name>/PERSONA.md`
2. `~/.config/mew/personas/<name>/PERSONA.md`

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
| `description` | Yes | Short description shown in the persona list |
| `mew.model` | No | Pin a `provider/model` pair (overrides session default) |
| `mew.tools` | No | Tool allowlist: `[]` (no tools), `[read, bash]` (whitelist), or absent (all tools) |
| `mew.tools_deny` | No | Tools to exclude from the allowlist |

## Switching personas

In the TUI:

```
/persona researcher
```

This opens a confirm modal showing the model/toolset diff. The switch only
happens after you confirm. Clearing with `/persona default` bypasses the modal.

The system prompt is rebuilt from scratch every turn, so persona body text
is always injected fresh.
