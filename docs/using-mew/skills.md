---
title: Skills
description: Reusable procedure documents that guide the agent.
---

Skills are Markdown files loaded on demand when a task matches a skill's
description. Using skills keeps multi-step workflows and checklists out of the
system prompt until the agent needs them.

## When to use a skill

Use a skill for repeatable procedures with ordered steps:

- Codebase investigation checklists (search patterns, verify call sites, confirm
  assumptions before planning)
- Release checklists (run tests, bump versions, update changelog, tag)
- Deployment runbooks (build, push, verify health checks)
- Project testing conventions (test locations, naming, required assertions)

Put single facts or always-on rules in a
[context file](/docs/getting-started/context-files/); put multi-step procedures
in a skill.

## How skills work

1. At startup, `mew-skills` scans discovery paths and loads every `SKILL.md`
   file along with the built-in `mew-docs` skill.
2. Mew lists each skill's name and description in the system prompt under
   `<available_skills>`.
3. When the model calls the `skill` tool with a skill name, Mew returns the
   skill body as a tool result.
4. Persona `mew.skills` allowlists can filter which skills are visible.

Write descriptions that state clearly what the skill does and when to load it.
In the TUI, type `@skill:<name>` to inline a skill body directly into your
prompt (this inserts the raw body without template rendering; the `skill` tool
still returns the rendered version).

## Discovery

Mew searches directories from the working directory up to the Git root, then
checks extension-provided skill directories, then global directories. When two
skills share a name, the earlier match wins (project skills override global
skills and the built-in `mew-docs` skill).

**Project paths** (cwd up to Git root):

1. `.mew/skills/<name>/SKILL.md`
2. `.opencode/skills/<name>/SKILL.md`
3. `.claude/skills/<name>/SKILL.md`
4. `.agents/skills/<name>/SKILL.md`

**Extension paths:**

5. Extension `[provides]` skill directories (in `mew run`)

**Global paths:**

6. `~/.config/mew/skills/<name>/SKILL.md`
7. `~/.config/opencode/skills/<name>/SKILL.md`
8. `~/.claude/skills/<name>/SKILL.md`
9. `~/.agents/skills/<name>/SKILL.md`

## Format

Create `<name>/SKILL.md` with Markdown instructions and optional YAML
frontmatter:

```markdown
---
name: release-checklist
description: Follow the release procedure before tagging a new version.
---

# Release Checklist

1. Run `cargo test --all` and confirm all tests pass.
2. Run `cargo clippy --all -- -D warnings`.
3. Bump the version in `Cargo.toml`.
4. Update `CHANGELOG.md` with changes since the last tag.
5. Commit the version bump and changelog.
6. Tag and push the release: `git tag v<version> && git push --tags`.

Stop and report any failure. Complete every step in order.
```

### Frontmatter fields

Without frontmatter, Mew derives `name` from the parent directory and sets an
empty description. When frontmatter is present, `name` must be 1 to 64
characters matching `^[a-z0-9]+(-[a-z0-9]+)*$` and must match the parent
directory name `<name>/SKILL.md` (otherwise the loader skips the skill).

| Field | Required | Description |
|-------|----------|-------------|
| `name` | When frontmatter is present | Skill identifier matching the parent directory name |
| `description` | No | One-line summary telling the model when to load the skill |
| `template` | No | Render the body through minijinja before returning when `true` |
| `license` | No | License metadata (informational) |
| `compatibility` | No | Compatibility metadata (informational) |
| `metadata` | No | Custom metadata map (informational) |

### Templated skills

When `template: true` is set, Mew renders the skill body through minijinja
before returning it from the `skill` tool, adapting instructions to the active
model, tools, and project variables:

```markdown
---
name: adaptive-test-runner
description: Run tests using the project's test command.
template: true
---

{% if has_tool("bash") %}
Run tests with: `cargo test --all`
{% else %}
You lack bash access. Ask the user to run `cargo test --all`.
{% endif %}

Model: {{ model_id }}
Date: {{ current_date }}
```

Templated skills share variables and helper functions with persona templates.
When no persona is active or the active persona omits `template: true`,
templated skills fall back to their raw body. See
[Personas](/docs/using-mew/personas/#templates) for all template variables.

### Writing effective skills

- **Make the description concrete**: write `"Follow the release procedure before
  tagging a new version."` instead of `"Release stuff."`
- **Give ordered, imperative steps**: state what to run, in what order, and how
  to handle failures. Avoid vague advice such as `"be careful"`.
- **Keep skills concise**: every loaded skill enters the conversation as a tool
  result and consumes context tokens.

## Persona integration

Personas restrict visible skills through `mew.skills` in frontmatter:

- `null` or omitted: all discovered skills available (default).
- `[skill1, skill2]`: only listed skills available.
- `[]`: no skills available.

See [Personas](/docs/using-mew/personas/) for the full persona reference.
