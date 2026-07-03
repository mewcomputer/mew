---
title: Comparing Features
description: How mew's overlapping features differ, and when to reach for each.
---

Several mew features look similar from the outside. This page lays out the
real differences so you can pick the right one. Each section links to the
detailed page for the feature.

## Personas vs skills vs subagents

All three are markdown files, all three live under `.mew/` (and friends),
and all three change how the agent behaves. The confusion is earned. They
differ on three axes: who they affect, when they apply, and what they
inject.

|                     | Persona                       | Skill                              | Subagent                       |
|---------------------|-------------------------------|------------------------------------|--------------------------------|
| Affects             | The main agent                | Any agent that loads it            | A fresh child agent            |
| When it applies     | Whole session, until switched | One turn, when the model loads it  | A bounded task, then it exits  |
| What gets injected  | System prompt (every turn)    | A tool result (that turn only)     | Its own separate conversation  |
| Own context window  | No, shares yours               | No, shares yours                    | Yes, isolated                  |
| Can pin a model     | Yes (`mew.model`)             | No                                 | Yes (`model` / tier keyword)   |
| Can restrict tools  | Yes (`mew.tools`)             | No                                 | Yes (`tools`)                  |
| Good for            | A mode of working             | A repeatable procedure             | Heavy or noisy work            |

**Pick by intent:**

- You want to flip how the agent works for the rest of the session
  (e.g. planner vs builder). Use a [persona](/docs/using-mew/personas/).
- You want the agent to follow a checklist only when the task calls for
  it, without paying for it on every turn. Use a
  [skill](/docs/using-mew/skills/).
- You want to hand off a chunk of work, keep its logs and file reads out
  of your main context, and get a summary back. Use a
  [subagent](/docs/using-mew/subagents/).

A subagent can load skills and run under its own persona, so these compose
rather than compete.

## Where do my instructions go?

Three places take free-text instructions: the
[context file](/docs/getting-started/context-files/), a persona body, and a
skill body. They serve different lifetimes.

| Put it in a...                                  | When                                                       |
|-------------------------------------------------|------------------------------------------------------------|
| Context file (`AGENTS.md` / `CLAUDE.md`)        | Always-on facts: build commands, architecture, conventions |
| [Persona](/docs/using-mew/personas/) body       | A switchable mode of working that changes the system prompt |
| [Skill](/docs/using-mew/skills/) body           | A multi-step procedure the agent follows only when relevant |

Rule of thumb: if it should be true on every turn, it goes in the context
file. If it depends on the mode you are in, it goes in a persona. If it is
a procedure you only sometimes need, it goes in a skill.

Context files and persona bodies both end up in the system prompt. Skills
do not: they enter context as a tool result on the turn the model loads
them, which is what keeps your system prompt small.

## Plugins vs MCP servers

Both spawn an external program and both can add tools. Beyond that they
diverge hard.

|                       | Plugin                                  | MCP server                              |
|-----------------------|-----------------------------------------|-----------------------------------------|
| Protocol              | mew's own JSON-RPC                      | The standard MCP protocol               |
| Adds tools            | Yes                                     | Yes                                     |
| Lifecycle hooks       | Yes, ~20 (mutate prompts, params, etc.) | No                                      |
| Registers slash cmds  | Yes                                     | No                                      |
| Mutates requests      | Yes                                     | No                                      |
| Persistent storage    | Yes, per-plugin key-value               | No (state lives inside the server)      |
| Tool sensitivity      | Set per tool                            | Always `Mutating`                       |
| Transport             | stdin/stdout subprocess                 | stdio subprocess or HTTP                |
| Good for              | Deep integration, telemetry, policy     | Adding off-the-shelf tools              |

**Pick by intent:**

- You just want more tools, especially ones someone else already wrote.
  Use an [MCP server](/docs/using-mew/mcp-servers/).
- You need to observe or mutate the agent lifecycle, ship a custom
  permission policy, or keep state across turns. Use a
  [plugin](/docs/using-mew/plugins/).

## Tools vs slash commands

Both extend what mew can do, but they are triggered by different actors.

|                  | Tool                                  | Slash command                    |
|------------------|---------------------------------------|----------------------------------|
| Triggered by     | The model, during a turn              | You, typing in the input box     |
| Model-facing     | Yes, with a JSON-schema and params    | No                               |
| Permission-gated | Yes (sensitivity + rules)             | No                               |
| Examples         | `read`, `bash`, `subagent_start`      | `/model`, `/persona`, `/clear`   |

If the model should do it autonomously while working, it is a
[tool](/docs/using-mew/tools/). If it is a shortcut for you to steer mew,
it is a [slash command](/docs/using-mew/slash-commands/). Plugins can
register slash commands too, which is how user-facing shortcuts get added
without changing mew itself.
