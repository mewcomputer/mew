---
title: Skills
description: Reusable procedure documents that guide the agent.
---

Skills are Markdown files with instructions the agent can load on demand.
When a task matches a skill's description, the agent calls the `skill`
tool to pull the full instructions into its context. Skills let you
codify workflows, conventions, and procedures without bloating the system
prompt on every turn.

## When to use a skill

Skills are for multi-step procedures the agent should follow consistently.
A few examples:

- A codebase investigation checklist (find patterns, verify locations,
  confirm assumptions before planning)
- A release procedure (run tests, bump version, update changelog, tag)
- A deployment runbook (build, push, verify health check)
- A testing convention (where tests go, naming, what to assert)

If something is a single fact or rule, put it in your
[context file](/docs/context-files/). If it's a procedure with steps,
make it a skill.

## How skills work

1. At startup, the `mew-skills` loader scans discovery paths and loads
   all `SKILL.md` files.
2. The `skill` tool registers only when at least one skill is discovered.
3. The model sees each skill's name and description in the tool list. It
   does not see the skill body until it calls the tool.
4. When the model calls `skill` with a name, the full body is loaded
   into context as a tool result.
5. Persona `mew.skills` allowlists can restrict which skills are
   available.

This means skill descriptions are important. The model decides whether
to load a skill based solely on its `description` field. Write
descriptions that clearly state when the skill applies.

## Discovery

Skills are discovered from standard directories, walked from cwd up to
the git root:

**Project paths** (walked cwd to git root, earlier wins):

1. `.mew/skills/<name>/SKILL.md`
2. `.opencode/skills/<name>/SKILL.md`
3. `.claude/skills/<name>/SKILL.md`
4. `.agents/skills/<name>/SKILL.md`

**Global paths:**

5. `~/.config/mew/skills/<name>/SKILL.md`
6. `~/.config/opencode/skills/<name>/SKILL.md`
7. `~/.claude/skills/<name>/SKILL.md`
8. `~/.agents/skills/<name>/SKILL.md`

When two skills have the same name, the one from the earlier path wins.
User-defined skills can shadow project skills by placing them in a
higher-priority directory.

## Format

A skill is a Markdown file with YAML frontmatter:

```markdown
---
name: release-checklist
description: Follow the release procedure before tagging a new version.
---

# Release Checklist

1. Run `cargo test --all` and confirm all tests pass
2. Run `cargo clippy --all -- -D warnings`
3. Bump the version in Cargo.toml
4. Update CHANGELOG.md with the changes since the last tag
5. Commit the version bump and changelog
6. Tag the release: `git tag v<version>`
7. Push tags: `git push --tags`

If any step fails, stop and report the error. Don't skip steps.
```

### Frontmatter fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Skill identifier. Lowercase letters, digits, and hyphens. Must match `^[a-z0-9]+(-[a-z0-9]+)*$` |
| `description` | yes | One-line description the model uses to decide when to load this skill |
| `template` | no | When `true`, render the body through minijinja before returning (see below) |
| `license` | no | License info (not currently used) |
| `compatibility` | no | Compatibility info (not currently used) |

### Templated skills

When `template: true` is set in the frontmatter, the skill body is
rendered through minijinja before being returned to the model. This lets
skill instructions adapt to the active model, available tools, and
project state.

```markdown
---
name: adaptive-test-runner
description: Run tests using the appropriate test command for the project.
template: true
---

{% if has_tool("bash") %}
Run tests with: cargo test --all
{% else %}
You don't have bash access. Ask the user to run `cargo test --all`.
{% endif %}

Model: {{ model_id }}
Date: {{ current_date }}
```

Templated skills use the same variables and functions as persona
templates. When no persona is active or the active persona doesn't use
templating, templated skills fall back to their raw body. See
[Personas](/docs/personas/#templates) for the full variable reference.

### Writing effective skills

The description is the most important field. The model uses it to decide
whether to load the skill, so be specific:

- **Good**: "Follow the release procedure before tagging a new version."
- **Bad**: "Release stuff."

The body should be imperative and concrete. Tell the agent what to do,
in what order, and what to do if something goes wrong. Avoid vague
instructions like "be careful" or "consider the options."

Skills are loaded into context as tool results, so they consume tokens.
Keep them as short as possible while remaining actionable. A skill that
is a page of precise steps is better than five pages of prose.

## Persona integration

Personas can restrict which skills are available via `mew.skills` in
the frontmatter:

- `null` or absent: all discovered skills available (default)
- `[skill1, skill2]`: only listed skills available
- `[]`: no skills available

See [Personas](/docs/personas/) for the full frontmatter reference.
