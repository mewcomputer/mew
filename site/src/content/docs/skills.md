---
title: Skills
description: Reusable procedure documents that guide the agent.
---

Skills are operator-curated procedure documents. Each skill is a Markdown
file with instructions the agent can load when a task matches the skill's
purpose. Skills are loaded by the `mew-skills` crate at startup.

## Discovery

Skills are discovered from standard directories, walked from cwd up to
the git root:

1. `.mew/skills/<name>/SKILL.md`
2. `.opencode/skills/<name>/SKILL.md`
3. `.claude/skills/<name>/SKILL.md`
4. `.agents/skills/<name>/SKILL.md`

Earlier paths win on duplicate names.

## Format

A skill is a Markdown file with YAML frontmatter:

```markdown
---
name: investigating-a-codebase
description: Investigate the local codebase to ground planning in reality.
---

# Investigating a Codebase

Find existing patterns, verify file locations, and confirm what exists
before assuming. Use grep to locate relevant lines before reading files...
```

## How skills work

- At startup, `mew-skills::Loader` scans the discovery paths and loads
  all `SKILL.md` files.
- The `Skill` tool registers only when at least one skill is discovered.
- The agent can call the `skill` tool with a skill name to load that skill's
  instructions into its context.
- Persona `mew.skills` allowlists can restrict which skills are available:
  `null` (all skills), `[skill1, skill2]` (whitelist), or `[]` (hide all).

## Built-in skills

mew includes several built-in skills (embedded at compile time):

- `investigating-a-codebase` — grounding planning in real code
- `researching-on-the-internet` — search + cross-reference patterns
- `modifying-polytoken` — fetching docs when modifying Polytoken

Built-in skills are always available. User-defined skills with the same
name override them.
