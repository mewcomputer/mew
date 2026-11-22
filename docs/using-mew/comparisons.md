---
title: Comparing Features
description: How Mew's overlapping features differ and when to use each.
---

## Personas, skills, and subagents

Personas, skills, and subagents all use Markdown files under `.mew/` and nearby
directories, and all three shape agent behavior. They differ in scope, timing,
and how they enter context:

|                     | Persona                       | Skill                              | Subagent                       |
|---------------------|-------------------------------|------------------------------------|--------------------------------|
| Affects             | Main agent                    | Any agent that loads it            | Child agent                    |
| When it applies     | Whole session, until switched | On the turn it is loaded           | Bounded task until exit        |
| Context mechanism   | Prepended to system prompt    | Tool result or `@skill:` inline    | Isolated child session         |
| Own context window  | Shared with main session      | Shared with caller session         | Isolated                       |
| Can pin a model     | Yes (`mew.model`)             | No                                 | Yes (`model` or tier keyword)  |
| Can restrict tools  | Yes (`mew.tools`)             | No                                 | Yes (`tools`)                  |
| Best used for       | Mode of working               | Repeatable procedure               | Noisy or parallel subtasks     |

Choose by intent:

- **Switch how the main agent works across turns** (for example, planner vs.
  builder): use a [persona](/docs/using-mew/personas/).
- **Load a checklist only when needed** so it stays out of unrelated turns: use
  a [skill](/docs/using-mew/skills/).
- **Delegate a focused subtask** so file reads and command output stay out of
  your main context: use a [subagent](/docs/using-mew/subagents/).

Subagents carry their own Markdown system prompt (independent of personas) and
can load skills whenever `skill` is in their tool list.

## Where instructions live

Context files, persona bodies, and skill bodies accept free-text instructions
with different lifetimes:

| File | Lifetime and use |
|------|------------------|
| [Context file](/docs/getting-started/context-files/) (`AGENTS.md` / `CLAUDE.md`) | Always in the system prompt: build commands, layout, conventions |
| [Persona](/docs/using-mew/personas/) body | Prepended to the system prompt while that persona is active |
| [Skill](/docs/using-mew/skills/) body | Name and description stay in the system prompt; body loads on demand |

Put permanent project facts in a context file, mode-specific instructions in a
persona, and task-specific procedures in a skill.

## Plugins and MCP servers

Both plugins and MCP servers add tools. Plugins run as local subprocesses using
Mew's JSON-RPC protocol; MCP servers run locally or over HTTP. Daemon chat and
`mew run` currently use a no-op hook dispatcher, so bare plugins are
experimental while MCP servers are active today.

|                       | Plugin                                  | MCP server                              |
|-----------------------|-----------------------------------------|-----------------------------------------|
| Protocol              | Mew JSON-RPC                            | Model Context Protocol                  |
| Adds tools            | Yes                                     | Yes                                     |
| Lifecycle hooks       | Yes, including prompt and request hooks | No                                      |
| Registers slash cmds  | Yes                                     | No                                      |
| Mutates requests      | Yes                                     | No                                      |
| Persistent storage    | Yes, per-plugin key-value               | No (managed by server)                  |
| Tool sensitivity      | `Mutating`                              | `Dangerous`                             |
| Transport             | stdin/stdout subprocess                 | stdio subprocess or streamable HTTP     |
| Best used for         | Lifecycle hooks, policy, telemetry      | Connecting external tool servers        |

Choose by intent:

- **Connect existing external tools** such as databases or APIs: use an
  [MCP server](/docs/using-mew/mcp-servers/).
- **Experiment with lifecycle hooks, custom slash commands, or plugin storage**:
  see [Plugins](/docs/using-mew/plugins/).

## Tools and slash commands

Tools and slash commands both extend Mew, triggered by different actors:

|                  | Tool                                  | Slash command                    |
|------------------|---------------------------------------|----------------------------------|
| Triggered by     | The model, during a turn              | You, in the composer             |
| Model-facing     | Yes, with JSON Schema parameters      | No                               |
| Permission-gated | Yes (sensitivity and rules)           | No                               |
| Examples         | `read`, `bash`, `subagent_start`      | `/model`, `/persona`, `/clear`   |

Use a [tool](/docs/using-mew/tools/) for actions the model runs during a turn,
and a [slash command](/docs/using-mew/slash-commands/) for shortcuts you run
directly. Plugins can also register custom slash commands when the hook runtime
is active.
