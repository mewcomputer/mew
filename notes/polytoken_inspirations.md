# What mew can learn from Polytoken

## Executive summary

Polytoken is a terminal-first coding agent that emphasizes a templated harness: facets (personas), skills, subagents, and built-in resources are all authored as [MiniJinja](https://github.com/mitsuhiko/minijinja) templates. It ships with a `polytoken://` virtual filesystem, a `plan`/`execute` facet handoff workflow, and a concise set of built-in tools. The design goal is to make the harness itself editable in the same language the model understands.

mew already shares a lot of DNA with Polytoken. Personas were modeled heavily on facets, mew also uses MiniJinja, and both projects discover `.mew`/`.opencode`/`.claude` style config trees. So this doc focuses less on "does mew have X?" and more on "what refinements does Polytoken have that mew is missing?"

Two big themes keep showing up:

1. **Templates are first-class everywhere.** Polytoken renders facets, skills, subagents, AGENTS.md, and built-in prompts as templates. mew templates persona bodies but leaves skills, subagents, and AGENTS.md mostly verbatim.
2. **Tool naming and surface are model-oriented.** Polytoken's tool names (`file_read`, `file_edit_search_replace`, `shell_exec`) are verbose but unambiguous, and it natively supports multiple edit formats (search/replace, hashline, patch). mew's tools are simpler but less precise.

Note: this repository already contains `.polytoken/permissions.local.yaml`, so the team is clearly already exploring Polytoken. This doc is meant to capture what is worth bringing back into mew.

---

## 1. Templates & rendering

### 1.1 MiniJinja as the harness language

**What Polytoken does.** Facets, subagents, skills, and built-in prompt fragments are all MiniJinja templates rendered before the model sees them. The same template engine shapes shipped defaults and user overrides.

**mew's current state.** mew uses MiniJinja for persona bodies when `template: true` is set (`crates/mew-prompts/src/persona.rs`). Skills, subagent bodies, and AGENTS.md are currently verbatim text.

**Pro adoption.**
- One language for all prompt authoring; skills and subagents can adapt to model, tools, and project state.
- User overrides and built-in defaults are the same kind of file.

**Anti adoption.**
- More files to validate at load time; a broken skill can crash the agent.
- Adds complexity for users who just want plain markdown instructions.

**Verdict.** Medium effort, high value. Extend MiniJinja rendering to skills and subagents behind an opt-in flag.

### 1.2 Strict missing-variable errors

**What Polytoken does.** Referencing a non-existent template variable raises an error at render time, so a typo never silently becomes blank text. Optional values must be guarded with conditionals or `default()`.

**mew's current state.** mew's persona renderer falls back to the raw body on any render error (`unwrap_or_else` returns the raw body) and logs a warning. This is lenient but can hide broken templates.

**Pro adoption.**
- Fail fast on broken personas/skills; users notice typos immediately.
- Encourages explicit handling of optional values.

**Anti adoption.**
- A strict failure in a built-in prompt could brick a session.
- mew's current fallback is safer for end users.

**Verdict.** Small effort, medium value. Add a strict mode or surface render warnings in the UI.

### 1.3 Template variable inventory

**What Polytoken exposes.** A rich set of variables: `model_name`, `model_variant`, `model_id`, `max_tool_batch_size`, `supports_vision`, `can_read_images`, `facet_name`, `subagent_name`, `project_path`, `cwd`, `session_id`, `current_date`, `is_non_interactive`, `prompt`, `available_tools`, `available_undeferred_tools`, `available_deferred_tools`, `tool_library`, `available_mcp_servers`, `available_subagents`, `available_skills`, `project_vars`, `auto_drain_notifications`, `source_control`.

**mew's current state.** mew exposes only `supports_vision`, `persona_name`, `tools`, and `denied_tools` in persona templates. Skills and subagents get no template context.

**Pro adoption.**
- More contextual prompts without hardcoding assumptions.
- `is_model_variant("claude")` is a clean way to vary guidance per provider.
- `source_control` lets prompts adapt to git/jj/sapling conventions.

**Anti adoption.**
- Large variable surface increases the chance of subtle prompt differences across environments.
- Requires documenting every variable.

**Verdict.** Medium effort, high value. Expand the template context gradually; start with model/facet/session variables.

### 1.4 Template functions

**What Polytoken adds.** `has_tool(name)`, `get_tool(name)`, `has_mcp(name)`, `is_model_variant(variant)`, `has_skill(name)`, and `transclude(uri)`.

**mew's current state.** mew has `transclude()` for persona templates but no query functions for tools/skills/MCP.

**Pro adoption.**
- Prompts can include instructions only when a specific tool or skill is present.
- Cleaner than manually maintaining tool lists in prompts.

**Anti adoption.**
- These functions are easy to add but easy to overuse.

**Verdict.** Small effort, medium value.

### 1.5 `@file` static includes

**What Polytoken does.** `@path/to/file` is resolved at load time and inlined as literal text before Jinja runs. Useful for sharing static prompt fragments without rendering them.

**mew's current state.** mew has `transclude()` which renders the included file as a template. There is no direct equivalent for static, non-rendered includes.

**Pro adoption.**
- Static includes prevent accidental template evaluation of shared fragments.
- Useful for code examples or literal prompts that contain `{{` braces.

**Anti adoption.**
- `transclude()` covers most cases; static includes are a niche need.

**Verdict.** Small effort, low-to-medium value.

### 1.6 `transclude()` semantics

**What Polytoken does.** `transclude("path.md")` runs at render time, renders the included file as a template with the current context, and confines paths to the calling file's directory subtree (`../` is rejected). It also supports `polytoken://` URIs for shipped resources.

**mew's current state.** mew's `transclude()` supports `mew://` URIs for built-in resources and falls back to treating the argument as a built-in path. It does not currently support project file paths or subtree confinement.

**Pro adoption.**
- Project-local prompt fragments let teams share conventions.
- Subtree confinement is a sensible security boundary.

**Anti adoption.**
- Adds filesystem path handling to the prompt renderer.

**Verdict.** Medium effort, medium value.

---

## 2. Facets vs Personas

mew personas were based heavily on Polytoken facets, so most of the concepts overlap. The differences are in details and a few extra knobs.

### 2.1 Frontmatter schema

**Polytoken frontmatter.** Conventional keys (`name`, `description`) sit at the top level; Polytoken-specific keys live under `polytoken`. Example:

```yaml
---
name: scribe
polytoken:
  tools: [mcp__notion]
  color: "#7c3aed"
---
```

**mew frontmatter.** mew uses `mew:` as the nested key instead of `polytoken:`:

```yaml
---
name: researcher
description: Read-only investigation
mew:
  model: z-ai/glm-4.5-air
  tools: [read, grep, write]
---
```

**Pro adoption (moving to `polytoken:` or supporting both).**
- Aligning with Polytoken would make imports easier for users coming from Polytoken.

**Anti adoption.**
- mew already has its own convention; changing it is churn.
- Could support both as aliases, but that adds complexity.

**Verdict.** Small value, small effort. Consider accepting `polytoken:` as an alias for `mew:` in persona/skill frontmatter.

### 2.2 Model pinning & fallback models

**What Polytoken does.** `polytoken.model` pins a facet to a model. `polytoken.fallback_models` lists alternatives to try if the primary is unavailable.

**mew's current state.** mew personas can pin `model` via the `mew:` block but have no fallback model list.

**Pro adoption.**
- Fallback models improve reliability without user intervention.
- Fits naturally with mew's provider-router work.

**Anti adoption.**
- Switching models mid-session can produce inconsistent behavior.

**Verdict.** Small effort, medium value.

### 2.3 Tool access shorthands

**What Polytoken does.** Tool lists accept:
- literal tool names
- `mcp__<server>` to grant every tool from an MCP server
- `tag!ALL` for every tool
- `tag!ALL_MCP` for every MCP tool
- `tools_deny` removes specific tools after expansion

**mew's current state.** mew personas accept literal tool names and `tools_deny`. There is no `tag!ALL` or `mcp__<server>` shorthand.

**Pro adoption.**
- `tag!ALL` is convenient for "give me everything" facets.
- `mcp__<server>` avoids enumerating MCP tool names.

**Anti adoption.**
- Explicit tool lists are safer; `tag!ALL` can accidentally expose dangerous tools.
- MCP server names can collide with built-in tool names.

**Verdict.** Small effort, medium value. Add carefully with clear precedence rules.

### 2.4 Skill allow/deny with tags

**What Polytoken does.** `polytoken.skills_allow` accepts skill names or `tag!<name>` groups. `polytoken.skills_deny` removes skills. A non-empty allowlist that matches nothing fails closed (no skills).

**mew's current state.** mew personas have a `skills` list (names only) but no tag groups or deny list.

**Pro adoption.**
- Tag groups make skill management scalable.
- Fail-closed behavior prevents silent over-permissioning.

**Anti adoption.**
- mew skills currently have no `tags` field, so this requires adding one.

**Verdict.** Small-to-medium effort, medium value.

### 2.5 Facet transitions

**What Polytoken does.** `polytoken.facet_transitions` controls whether and how a facet can switch to another facet, including optional confirmation prompts.

**mew's current state.** mew has a `switch_persona` tool, but personas do not declare transition rules.

**Pro adoption.**
- Prevents accidental persona escapes (e.g., planner switching to builder without handoff).
- Enables "ask before leaving" workflows.

**Anti adoption.**
- Adds another configuration surface.

**Verdict.** Small effort, medium value.

### 2.6 Colors

**What Polytoken does.** Facets can set `color`, `color_light`, and `color_dark`. If omitted, Polytoken generates deterministic accent colors from the facet name.

**mew's current state.** mew personas have no color fields.

**Pro adoption.**
- Visual distinction in the sidebar makes facet/persona switching clearer.
- Deterministic fallback avoids configuration burden.

**Anti adoption.**
- The TUI and web UI need new theming support.

**Verdict.** Small effort, low-to-medium value.

### 2.7 Autonomous hints

**What Polytoken does.** `polytoken.autonomous_hint` provides guidance text for the autonomous permissions classifier when evaluating tool calls in this facet.

**mew's current state.** mew's permission engine supports rules and auto/auto+ modes, but personas do not carry classifier hints.

**Pro adoption.**
- Lets a persona say "I am a read-only researcher; be stricter about shell commands."
- Improves classifier accuracy.

**Anti adoption.**
- Only useful when auto mode is enabled.

**Verdict.** Small effort, medium value.

### 2.8 Undeferred tools

**What Polytoken does.** `polytoken.undeferred_tools` lists tools whose full definitions are always sent up front, even when native deferred tool loading is active.

**mew's current state.** mew has no deferred tool loading concept; all tools are sent every turn.

**Pro adoption.**
- Deferred loading reduces token usage when many tools/MCP servers are registered.
- Critical tools can still be forced into context.

**Anti adoption.**
- mew's tool surface is small enough that deferred loading is not urgent.
- Requires provider support for tool search/on-demand loading.

**Verdict.** Large effort, medium value. Ideally, defer until tool/MCP count grows.

---

## 3. Plan / execute handoff workflow

**What Polytoken does.** Polytoken ships built-in `plan` and `execute` facets. The `plan` facet investigates without edit/shell tools, writes a plan via `write_plan`, optionally runs a `plan-reviewer` subagent, and calls `handoff_plan` to submit the plan for user approval. The user then switches to `execute`, which gets the full tool surface. `switch_facet` is a regular tool; `plan` deliberately does not list it, so the model cannot leave planning on its own.

**mew's current state.** mew ships built-in `planner` and `builder` personas. `planner` has read-only tools plus `write`/`edit` for plan files; `builder` has all tools. There is no dedicated `handoff_plan` tool or plan-review subagent. Switching personas is done via the `switch_persona` tool, which any persona can call if personas exist.

**Pro adoption.**
- A formal handoff tool makes the plan-to-execution transition explicit.
- Built-in plan reviewer improves plan quality before code is written.
- Preventing the planner from switching personas autonomously reduces accidental escapes.

**Anti adoption.**
- mew's existing planner/builder already capture much of this.
- Extra handoff friction can slow down simple tasks.

**Verdict.** Medium effort, high value. Add a `handoff_plan` tool and tighten planner transition controls.

---

## 4. Skills

### 4.1 Skill templating

**What Polytoken does.** A skill's body is rendered as a template when `polytoken: true` is set in frontmatter. Skills can use the same variables/functions as facets except `transclude`.

**mew's current state.** mew skill bodies are always verbatim.

**Pro adoption.**
- Skills can adapt to active model or available tools.

**Anti adoption.**
- Skills are often shared across tools; making them Polytoken-specific reduces portability.
- mew already supports the Agent Skills format.

**Verdict.** Small effort, medium value. Add opt-in templating.

### 4.2 Skill tags and invocation controls

**What Polytoken does.** Skills can have `polytoken.tags` for grouping and `polytoken.disable_model_invocation` to hide a skill from the model. Hidden skills can still be invoked explicitly via `@skill:<name>` in a prompt.

**mew's current state.** mew skills have no tags or invocation controls. The model sees all loaded skills and can call any via the `skill` tool (subject to persona allowlists).

**Pro adoption.**
- Tags enable facet-level skill groups.
- Hidden skills are useful for operator-only workflows.

**Anti adoption.**
- Adds another frontmatter field to maintain.

**Verdict.** Small-to-medium effort, medium value.

### 4.3 `@skill:<name>` prompt references

**What Polytoken does.** Users can load a skill by mentioning `@skill:<name>` in their prompt.

**mew's current state.** mew has a `skill` tool the model calls; users cannot directly invoke a skill from their prompt.

**Pro adoption.**
- Faster user-driven skill loading.
- Natural for hidden skills.

**Anti adoption.**
- Requires parsing `@` references in user input.

**Verdict.** Small effort, medium value.

---

## 5. Subagents

### 5.1 Subagent frontmatter controls

**What Polytoken does.** Subagents have frontmatter similar to facets, plus `allow_subagent_spawn`, `exit_tool_schema`, and `inherit_tools`.

**mew's current state.** mew subagents are loaded from `.mew/agents/*.md` via `mew-subagents`. The frontmatter support is lighter; `allow_subagent_spawn` and `exit_tool_schema` are not exposed.

**Pro adoption.**
- `inherit_tools` avoids duplicating tool lists for subagents that do the same work as the parent.
- Custom exit schemas let subagents return typed JSON.

**Anti adoption.**
- mew's subagent runner already works; these are refinements.

**Verdict.** Medium effort, medium value.

### 5.2 Plan handoff subagent

**What Polytoken does.** The built-in `plan-reviewer` subagent reviews plans before handoff.

**mew's current state.** mew has a `reviewer` subagent but no explicit plan-review workflow.

**Pro adoption.**
- Cheap way to catch bad plans before execution.

**Anti adoption.**
- Adds latency to planning.

**Verdict.** Small effort, medium value.

---

## 6. Tools

### 6.1 File read

**What Polytoken does.** `file_read` and `file_read_hashline`. Large files return a structural outline; the model requests specific ranges when needed. `file_read_hashline` is the variant used by models on the hashline edit format.

**mew's current state.** mew has `read` with `offset`/`limit` but no structural outline or hashline variant.

**Pro adoption.**
- Outlines keep context windows small.
- Per-format read tools match the model's expected edit format.

**Anti adoption.**
- Outlines require tree-sitter or heuristics.

**Verdict.** Medium effort, high value.

### 6.2 File edit

**What Polytoken does.** Three edit tools: `file_edit_search_replace`, `file_edit_hashline`, `patch_edit`. The model gets the one matching its configured edit format.

**mew's current state.** mew has one `edit` tool using exact `old_string`/`new_string` replacement.

**Pro adoption.**
- Multiple edit formats let mew experiment with hashline/patch without breaking existing behavior.
- Format-specific tools make the schema explicit.

**Anti adoption.**
- More tools to maintain and document.

**Verdict.** Medium-to-large effort, high value. See also the omp doc for hashline discussion.

### 6.3 File write

**What Polytoken does.** `file_write` creates or overwrites. Live cards show a diff-style preview of the first 10 lines.

**mew's current state.** mew has `write` with similar behavior.

**Pro adoption.**
- Preview the first N lines in the TUI/web card is a nice UX polish.

**Anti adoption.**
- mew already has the core capability.

**Verdict.** Small effort, low value. Nice-to-have polish.

### 6.4 Glob

**What Polytoken does.** `glob` honors ignore files, returns project-relative paths sorted by mtime, accepts one root or an array of roots, deduplicates overlaps, and handles permission checks for directory prefixes.

**mew's current state.** mew's `glob` uses `ignore`/`globset`, sorts alphabetically, and accepts a single `path` root.

**Pro adoption.**
- Sorting by mtime surfaces recently changed files first, which is often what the model wants.
- Multiple roots are useful for monorepos.

**Anti adoption.**
- Alphabetical sorting is predictable.

**Verdict.** Small effort, medium value.

### 6.5 Grep

**What Polytoken does.** `grep` searches file contents with regex and returns matching lines with surrounding context. Supports project-relative or absolute paths for approved external directories.

**mew's current state.** mew's `grep` shells out to `rg`/`grep` and returns filename:line:content. No surrounding context option.

**Pro adoption.**
- In-process implementation (see omp doc) plus context lines would make grep more useful.

**Anti adoption.**
- `rg` already supports context via flags; mew could add a `context` parameter.

**Verdict.** Small effort, medium value.

### 6.6 `flag_important`

**What Polytoken does.** `flag_important` marks a file as important so it survives compaction. `included` mode inlines content; `referenced` mode records a pointer.

**mew's current state.** mew has `flag_important` with a similar purpose. The implementation differs but the concept is present.

**Pro adoption.**
- mew is already close; verify parity with included/referenced modes.

**Anti adoption.**
- None.

**Verdict.** Verify parity; small effort if gaps exist.

### 6.7 Shell exec

**What Polytoken does.** `shell_exec` runs a shell command in a non-login Bash shell with the working directory set to the project root.

**mew's current state.** mew's `bash` tool runs in the session `cwd`, uses a login `bash -c`, supports timeout, and returns partial output on timeout.

**Pro adoption.**
- Consistent project-root cwd avoids subtle directory bugs.
- Non-login shell matches most CI/automation expectations.

**Anti adoption.**
- mew's cwd-relative behavior is flexible for multi-directory work.

**Verdict.** Small effort, low value. Could be a configurable default.

### 6.8 `pushd` / `popd`

**What Polytoken does.** `pushd`/`popd` change the agent's effective working directory. Relative paths in file/glob/grep/shell tools resolve from the new directory.

**mew's current state.** mew has no equivalent. Each `bash` call is independent, and file tools resolve from `ctx.cwd`.

**Pro adoption.**
- Lets the model work in multiple directories within one session.
- Cleaner than prefixing every path.

**Anti adoption.**
- mew's `cwd` is already per-session; changing it dynamically could confuse the UI.

**Verdict.** Small effort, medium value.

### 6.9 Job management

**What Polytoken does.** `job_status`, `job_block`, `job_result`, `job_cancel`, and `shell_monitor` for waiting on background work.

**mew's current state.** mew has `ShellBackground`, `ShellMonitor`, `JobStatus`, `JobBlock`, `JobCancel`. `job_result` is not a separate tool; results are returned via `JobStatus`/`JobBlock`.

**Pro adoption.**
- A dedicated `job_result` tool is cleaner for fetching completed output.
- `shell_monitor` is a useful specialization of `job_block`.

**Anti adoption.**
- mew's existing tools cover most cases.

**Verdict.** Small effort, low-to-medium value.

### 6.10 Plan tools (`write_plan`, `edit_plan`, `handoff_plan`)

**What Polytoken does.** Plan-specific tools for recording, editing, and handing off plans.

**mew's current state.** mew has no plan-specific tools. Plans are plain files written via `write`/`edit`.

**Pro adoption.**
- Formalizes the planner/builder workflow.
- Enables plan review and approval gates.

**Anti adoption.**
- Adds tools the model must learn.

**Verdict.** Medium effort, high value. Tight integration with personas.

### 6.11 `switch_facet`

**What Polytoken does.** `switch_facet` is a regular tool that changes the active facet. Whether it is available depends on the current facet's tool list.

**mew's current state.** mew has `switch_persona` with similar behavior.

**Pro adoption.**
- mew is at parity; the gap is in transition rules (see 2.5).

**Verdict.** No major new work; add transition rules if desired.

### 6.12 Web search & fetch

**What Polytoken does.** `web_search` searches the web; `web_fetch` fetches a page or extracts an answer when given a query.

**mew's current state.** mew has no built-in web search or fetch. The `.polytoken/permissions.local.yaml` file references `web_fetch`, suggesting the team uses Polytoken for this today.

**Pro adoption.**
- High-value feature; see omp doc for web search discussion.
- `web_fetch` is simpler than full search and useful for reading docs/URLs.

**Anti adoption.**
- Requires third-party provider or crawler integration.

**Verdict.** Medium-to-large effort, high value. Consider `web_fetch` as a smaller first step.

### 6.13 MCP resource tools

**What Polytoken does.** `mcp_list_resources` and `mcp_read_resource` expose MCP resources to the model.

**mew's current state.** mew connects to MCP servers and exposes MCP tools, but not MCP resources.

**Pro adoption.**
- Resources are part of the MCP spec; some servers expose read-only data as resources rather than tools.

**Anti adoption.**
- Most MCP servers use tools; resources are less common.

**Verdict.** Small-to-medium effort, low-to-medium value.

### 6.14 `tool_search`

**What Polytoken does.** `tool_search` looks up the full definition of a tool by name. Used when deferred tool loading is active or MCP servers are configured.

**mew's current state.** mew has no deferred tool loading or `tool_search`.

**Pro adoption.**
- Enables deferred loading, reducing context usage with many tools.

**Anti adoption.**
- Not useful until deferred loading is implemented.

**Verdict.** Medium effort, medium value. Bundle with deferred tool loading.

### 6.15 `ask_user_question`

**What Polytoken does.** `ask_user_question` asks one to four structured questions and waits for answers. Every question allows free-text response.

**mew's current state.** mew has `ask_user_question` (named `AskUser`) with similar behavior.

**Pro adoption.**
- mew is at parity.

**Verdict.** No major new work.

---

## 7. Project context / AGENTS.md

### 7.1 Templated AGENTS.md

**What Polytoken does.** AGENTS.md can include a frontmatter block with `polytoken: true` to render the body as a template. Without it, the file is verbatim.

**mew's current state.** mew loads AGENTS.md/CLAUDE.md verbatim and wraps them in `<context>` tags. No templating.

**Pro adoption.**
- Project instructions can adapt to model family or active tools.
- Backwards compatible: omit `polytoken: true` and behavior is unchanged.

**Anti adoption.**
- AGENTS.md is often written by humans; adding template syntax may confuse.

**Verdict.** Small effort, medium value.

### 7.2 `@file` includes in AGENTS.md

**What Polytoken does.** `@path/to/file` in AGENTS.md inlines another file as literal text.

**mew's current state.** mew does not support includes in AGENTS.md.

**Pro adoption.**
- Split large project context into multiple files.

**Anti adoption.**
- Most projects keep AGENTS.md self-contained.

**Verdict.** Small effort, low value.

### 7.3 Discovery precedence

**What Polytoken does.** Polytoken reads global config directory first, then project root. Within a project, it uses `AGENTS.md`, falls back to `CLAUDE.md`, then `GEMINI.md`.

**mew's current state.** mew walks from cwd up to git root, loading `AGENTS.md` preferred and `CLAUDE.md` fallback per level, plus `.mew/AGENTS.md` additively. Global: `~/.config/mew/AGENTS.md` then `~/.claude/CLAUDE.md`.

**Pro adoption.**
- Polytoken's "global first, then project" ordering means project files override globals naturally.
- mew's "most general to most specific" ordering is similar but worth verifying for edge cases.

**Anti adoption.**
- The two approaches are close; no urgent change.

**Verdict.** Review and document precedence; small effort.

---

## 8. VFS / built-in resources

**What Polytoken does.** `polytoken vfs ls` and `polytoken vfs cat` expose built-in resources under `polytoken://`. Users can copy shipped facets as starting points.

**mew's current state.** mew has `mew debug vfs ls/cat` for the `mew://` VFS (`crates/mew-prompts/src/vfs.rs`). Built-in personas and subagents live in `resources/`.

**Pro adoption.**
- mew is already at parity. Consider improving discoverability and making it easier to copy/edit built-ins.

**Anti adoption.**
- None.

**Verdict.** Small polish effort.

---

## 9. Project variables

**What Polytoken does.** `.polytoken/project_vars.yaml` holds project-local template data accessible as `project_vars` in templates. Missing paths render empty with a warning rather than failing.

**mew's current state.** mew has no project variables file. Personas/skills can use template variables only from the runtime context.

**Pro adoption.**
- Teams can keep project-specific constants (team name, escalation channel, feature flags) out of prompts.
- Natural companion to templated AGENTS.md/personas.

**Anti adoption.**
- Another file to maintain.

**Verdict.** Small effort, medium value.

---

## 10. Permissions / autonomy

**What Polytoken does.** Autonomous mode classifies tool calls. Facets can provide `autonomous_hint` to steer the classifier. Permissions can be configured per tool and per project in `.polytoken/permissions.local.yaml`.

**mew's current state.** mew has Standard/Permissive/Auto/Auto+/Dangerous modes, declarative permission rules in `config.toml`, and a classifier provider for Auto modes. The `.polytoken/permissions.local.yaml` file in this repo suggests the team uses Polytoken's local permission file for some workflows.

**Pro adoption.**
- Project-local permission files (like `.polytoken/permissions.local.yaml`) are convenient and version-controlled.
- Facet-level classifier hints improve auto-mode decisions.

**Anti adoption.**
- mew already has a config-based permission system; adding a second file format adds complexity.

**Verdict.** Medium effort, medium value. Consider supporting project-local permission files.

---

## Prioritized shortlist

| Priority | Feature | Effort | Why now / why later |
|----------|---------|--------|---------------------|
| P0 | Accept `polytoken:` alias in persona/skill frontmatter | Tiny | Low-risk compatibility for users coming from Polytoken. |
| P0 | Template skills and subagents (opt-in) | Small | Natural extension of existing persona templating. |
| P0 | Expand persona template variables (`model_variant`, `session_id`, `cwd`, etc.) | Small | More contextual prompts without hardcoding. |
| P1 | Templated AGENTS.md (`mew: true` or `polytoken: true` frontmatter) | Small | Backwards-compatible; enables adaptive project context. |
| P1 | Plan handoff tools (`write_plan`, `edit_plan`, `handoff_plan`) | Medium | Formalizes mew's existing planner/builder workflow. |
| P1 | `web_fetch` tool | Medium | Smaller first step than full web search; already used by the team (per `.polytoken/permissions.local.yaml`). |
| P1 | Project variables file (`.mew/project_vars.yaml`) | Small | Companion to templating. |
| P1 | Tool/skill tag groups (`tag!ALL`, `tag!<name>`) | Small | Scales tool/skill management as the surface grows. |
| P2 | Multiple edit formats (`file_edit_search_replace`, `file_edit_hashline`, `patch_edit`) | Medium-Large | High value but requires new tool schemas and UI. |
| P2 | Structural file outlines in `read` | Medium | Keeps context windows manageable for large files. |
| P2 | Facet transition rules / autonomous hints | Small | Nice controls for plan/execute workflows. |
| P2 | Fallback models per persona | Small | Reliability improvement. |
| P3 | Deferred tool loading + `tool_search` | Large | Only valuable when tool/MCP count is high. |
| P3 | MCP resource tools | Small-Medium | Low priority until a concrete MCP server needs them. |
| P3 | Persona colors | Small | UI polish, not core functionality. |
| P3 | `@file` static includes | Small | Niche need; `transclude()` covers most cases. |

---

## Appendix: sources and mew files

**Polytoken sources**
- Docs homepage: https://docs.polytoken.dev
- Template reference: https://docs.polytoken.dev/reference/template-reference/
- Tool reference: https://docs.polytoken.dev/reference/tools/
- Templating guide: https://docs.polytoken.dev/harness-engineering/templating/
- Facets: https://docs.polytoken.dev/harness-engineering/facets/
- Skills: https://docs.polytoken.dev/harness-engineering/skills/
- VFS: https://docs.polytoken.dev/extending-polytoken/polytoken-vfs/
- Project context: https://docs.polytoken.dev/using-polytoken/project-context/

**Relevant mew files**
- `crates/mew-prompts/src/persona.rs` — persona MiniJinja rendering
- `crates/mew-prompts/src/system.rs` — system prompt assembly
- `crates/mew-prompts/src/vfs.rs` — built-in VFS
- `crates/mew-prompts/src/skills.rs` — skills XML block
- `crates/mew-prompts/src/subagent.rs` — built-in subagent prompts
- `crates/mew-personas/src/lib.rs` — persona discovery and config
- `crates/mew-skills/src/lib.rs` — skill discovery
- `crates/mew-context/src/lib.rs` — AGENTS.md/CLAUDE.md discovery
- `crates/mew-tools/src/tools/skill.rs` — skill tool
- `crates/mew-tools/src/tools/switch_persona.rs` — persona switching
- `crates/mew-tools/src/tools/{read,edit,write,glob,grep,bash}.rs` — core tools
- `crates/mew/src/main.rs` — tool registration and agent builder
- `.polytoken/permissions.local.yaml` — existing Polytoken permission file in this repo
