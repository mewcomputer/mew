# polytoken parity notes (working, uncommitted)

working notes on features worth borrowing from polytoken, grounded against the
current mew codebase. not committed scope. promote items into `PLAN.md` as
numbered milestones when we decide to do them.

references throughout are to docs.polytoken.dev (read june 2026, v0.2.0) and
the mew tree at HEAD.

---

## priority at a glance

**status legend**: ✅ shipped · ⚠️ partial · 🟡 open · ⛔ deferred

| feature | value | effort | status | notes |
|---|---|---|---|---|
| todos | high | small | ✅ | `mew-agent/src/todos.rs` + 5 tools + `App.todos` + sidecar `todos.json` |
| ask_user_question | high | small | ✅ | `mew-tools/src/tools/ask_user.rs` + `AgentEvent::AskUser` + modal |
| personas | high | medium | ✅ | `mew-personas` + minijinja templating + tools_deny + skills_allow + confirm modal |
| secret files + words | high | small | ✅ | `secrets.files` + `secrets.words` in config; redaction in Read/Bash/Grep/Glob |
| flag_important | medium | small | ✅ | `mew-tools/src/tools/flag_important.rs` + compaction re-injection in `turn.rs` |
| /clear + session/context split | medium | small | ✅ | `Agent::clear_context` writes synthetic marker to session log |
| /rewind | medium | medium | ✅ | non-destructive (in-memory head pointer); `/rewind <n>` slash command |
| shell decomposition | medium | medium | ✅ | `mew-config/src/shell.rs`; opaque detection; `MatchConditions.command_program` |
| hooks runtime parity | medium | medium | ✅ | 21 hooks + matchers + disabled_hooks + timeout_ms + `HookOutcome<T>` generalization (Proceed/Block/Suppress) on the two blocking hooks; **subprocess `Block`/`Suppress` protocol still TODO** (Rust plugins get the full API today) |
| jobs (async subagents + bg shell) | medium | medium | ✅ | `mew-agent/src/agent.rs::ShellJob` + 5 jobs tools + sidebar section |
| autonomous permission mode | low | medium | ✅ | went further than the parity doc — four modes (`Auto`, `Auto+`) on top of Standard/Permissive/Dangerous! |
| daemon/TUI split | low | large | ⛔ | iroh + acp-server cover the remoting case; deferred |
| minijinja templating | low | small *if personas exist* | ✅ | shipped with personas (`template: true` frontmatter) |

**11 of 13 shipped, 1 partial, 1 deferred.** The only remaining tier-1/2 work is the hooks runtime polish (generalize `PermissionDecision` to a `HookOutcome` enum + `!name` negation). Tier 3 is complete.

---

## Status as of 2026-06-21

The parity doc was the original planning surface. What actually shipped, in chronological order:

1. **Personas v2 polish** — sidebar section, confirm modal, `switch_persona` tool, `tools_deny`, `skills_allow`, minijinja templating. Documented in `CURRENT.md`.
2. **Secrets** — config, permission pre-check, redaction in Read/Bash/Grep/Glob.
3. **Shell command decomposition** — `shell.rs`, opaque detection, `MatchConditions.command_program`/`command_subcommand`.
4. **Hooks runtime overhaul** — 21 hooks, `HookId` enum, `PluginHookConfig`, parallel dispatch, plugin health.
5. **Telemetry exporter** example plugin (Prometheus `/metrics` on :9090).
6. **Jobs** — background shell + job control, `ShellJob` registry, 5 tools, sidebar section.
7. **Landing / start page redesign** — centered hero with the cat ASCII + bold "mew" wordmark.
8. **Permission semantic audit** — `/clear` × permission caches pinned, Mutating/Dangerous collapse pinned.
9. **Dangerous! permission mode** → **Permissive** → **Auto** → **Auto+** — four-tier slider beyond what the parity doc proposed.
10. **System prompts centralization** — new `mew-prompts` crate with `system` / `persona` / `skills` / `subagent` / `classifier` / `inventory` modules. Foundation for Auto.
11. **Auto / Auto+ classifier-driven modes** — small LLM routes permissions; Auto escalates to user on uncertainty, Auto+ fails closed.

The four open questions at the bottom of this doc are still open. The single partial item (hooks runtime polish) is the only remaining tier-1/2 work.

## Persona

a Persona is a markdown-defined persona that controls system prompt, tool access,
skill access, and model pin. switching personas mid-conversation changes those
without resetting history. this is the feature we explicitly want.

Compare primarily to polytoken's "facets".

why mew-skills makes this cheaper than i first claimed: `crates/mew-skills/`
already implements the 8-path discovery walk, yaml frontmatter parsing, the
`^[a-z0-9]+(-[a-z0-9]+)*$` name regex, and the "frontmatter name must match
directory name" check. personas reuse that machinery verbatim with a different
frontmatter shape.

### discovery (mostly free)

new crate `mew-personas`, or fold skills + personas + subagents into a shared
`mew-harness` crate since their loaders are 90% identical. paths, matching
mew's convention:

```
<project>/.mew/personas/<name>.md
<project>/.opencode/personas/<name>.md
<project>/.claude/personas/<name>.md
<project>/.agents/personas/<name>.md
~/.config/mew/personas/<name>.md      (+ opencode/claude/agents globals)
```

shipped personas we should bundle: `default` (current behavior, no filtering),
`plan`, `execute`. plan/execute can land in a follow-up; v1 ships `default`
plus whatever the user defines.

### frontmatter (mew-flavored)

```markdown
---
name: researcher
description: read-only investigation
mew:
  model: z-ai/glm-4.5-air          # optional; omit to inherit active
  tools: [read, glob, grep, mcp__notion]   # mcp__<server> grants whole server
  tools_deny: []                   # subtract from granted
  skills_allow: []                 # empty list = no skills (closed)
  color: "#7c3aed"                 # sidebar accent
  transitions:
    execute: { allowed: true, confirm: "switch to execute? this grants write tools" }
---

persona body. becomes the system-prompt framing. in v1 this is verbatim text;
templating is a later milestone (see below).
```

using `mew:` as the namespacing key matches polytoken's `polytoken:` convention
and keeps cross-tool frontmatter keys (`name`, `description`) at the top level.

### agent state split

the real work. today `Agent.tools: HashMap<String, Arc<dyn Tool>>`
(`crates/mew-agent/src/agent.rs:39`) is both registry and active set. `turn.rs`
reads it directly at line 90 to build `tool_defs`. personas need:

- `Agent.all_tools: HashMap<String, Arc<dyn Tool>>` — full registry, immutable after startup
- `Agent.active_tools: HashMap<String, Arc<dyn Tool>>` — filtered view, what gets sent
- `Agent.active_persona: Option<persona>` — `None` means "default, no filtering"
- `Agent.apply_persona(&mut self, persona)` — recomputes active_tools, system prompt, provider (if model pinned)

`turn.rs:90` swaps `self.tools` → `self.active_tools`. everything else is additive.

### system prompt becomes persona-aware

today the prompt is assembled once in `main.rs` (`build_skills_xml` +
`mew_context::build_system_prompt`) and set via `agent.set_system(...)`. for
personas, move assembly into a method `Agent::rebuild_system()` that composes:

```
[persona body]
[mew base prompt, if we ship one]
<context source="...">...</context>   # ctx files, existing
<available_skills>...</available_skills>  # existing, filtered by persona.skills_allow
```

call it on startup and on every `switch_persona`. PLAN.md's appendix already
asserts "system prompt is rebuilt from scratch every turn" — currently only the
`on_system_prompt` hook runs per turn; the base is static. personas make the base
dynamic too, which is consistent with the appendix's intent.

### switch_persona tool

model-callable when the current persona lists it in `mew.tools`. executes
`agent.apply_persona(...)`, emits `AgentEvent::personaChanged { name, color }` so
the TUI updates the status line and sidebar. transitions obey
`mew.transitions.<target>.{allowed,confirm}`: a `confirm` string routes through
the existing permission-modal plumbing (`AgentEvent::PermissionRequest`-shaped,
just a yes/no).

### per-persona model pin

`mew.model` resolves through the existing `MainModelResolver` in `main.rs:869`.
on switch, rebuild the provider via `build_provider(...)`. the `SwitchModel`
plumbing in the TUI loop (`main.rs:1344`) is the template for the provider swap.

### what to skip in v1

- **plan/execute workflow + plan-reviewer + BLAKE3 plan hashing.** this is a
  follow-up milestone *built on* personas, not part of the personas milestone.
  ship personas as "switchable personas with tool/prompt/model scoping" first.
- **minijinja templating.** ship personas with verbatim markdown bodies. add a
  `mew.template: bool` frontmatter key later; when any persona opts in, pull
  `minijinja` (small, pure-rust, no proc-macro deps) and expose
  `supports_vision`, `has_tool`, `has_skill`, model id. keeps v1 surface area
  small and avoids committing to a template engine before the persona model
  proves out.
- **`undeferred_tools` / tool_search.** only matters at high tool count.

### size

rough: ~1000 LOC + tests. loader is ~free (port mew-skills), agent state split
is mechanical, system-prompt refactor is the fiddly bit, switch_persona + TUI
wiring is small. one focused milestone, not a saga. i was wrong earlier to
call this multi-month.

---

## todos

session-lived task list, dependency-enforced, survives compaction. polytoken's
version is the model: status (pending/in_progress/done/blocked), dependencies
(can't start until deps done, can't delete with dependents).

### tools

five new files in `crates/mew-tools/src/tools/`, mirroring the per-file
convention (`read.rs`, `edit.rs`, etc.):

- `todo_create.rs` — `todo_create(todos: [{ content, depends_on? }])`, **batchable** (polytoken runs several from one model response; worth copying so the model doesn't burn a turn per item)
- `todo_update.rs` — change content or status of one item
- `todo_complete.rs` — mark done (rejected if deps incomplete)
- `todo_delete.rs` — remove (rejected if others depend on it)
- `todo_list.rs` — return current list

all `ReadOnly` sensitivity (they mutate session state, not the filesystem; the
`Sensitivity` enum is about fs/shell risk, so `ReadOnly` fits).

dependency enforcement lives inside `execute` — validate against current state
before mutating. the tool holds the list; enforcement is pure logic, easy to
unit-test exhaustively (this is exactly the kind of code where a subtle bug
becomes "agent silently dropped my task").

### storage (the decision that matters)

todos must survive compaction. compaction (`turn.rs:106-156`) mutates
`agent.messages` in memory; the session file is append-only and unaffected. two
options:

- **(a) sidecar `<session>/todos.json`**, written on each mutation, read on resume.
  simple, decoupled from the message log, trivially survives compaction since
  compaction never touches it.
- **(b) encode as a new `Part::TodoMutation`** inside messages. more "event
  log"-y but breaks under compaction: old mutations get dropped when their
  carrying message is compacted away, and reconstructing current state requires
  replaying the whole log.

(a) is right. the session folder layout (`<id>/session.jsonl` + `meta.json`)
already exists; add `<id>/todos.json` as a sibling.

### TUI

- `App.todos: Vec<Todo>` in `crates/mew-tui/src/app.rs`, alongside the existing
  `subagents`, `mcp_status`, etc.
- new sidebar pane in `ui/sidebar.rs` (the sidebar already has collapsible
  sections via `sidebar_collapsed`).
- `/todo` slash command opens a full-screen editable view; both user and model
  edit the same list. `handle_slash` (`app.rs:978`) gains a `Todo` variant.
- `AgentEvent::TodosUpdated(Vec<Todo>)` new variant in `mew-agent/src/lib.rs`
  for agent→TUI sync; tool pushes through `ev_tx` after each mutation.

### relationship to CURRENT.md

todos subsume the "what's left" function of `CURRENT.md`. `CURRENT.md` keeps
its append-only "what was done, where, decisions made" function
(`CLAUDE.md:117` already describes it that way). two different things sharing a
file today; the split is healthy.

### size

~500 LOC + tests. small because the surface is bounded and the storage decision
is clean.

---

## ask_user_question

structured Q&A. model calls it when its next step depends on an answer only the
user can give. 1-4 questions, all free-text.

### tool

`crates/mew-tools/src/tools/ask_user.rs`. `ReadOnly` sensitivity. schema:

```json
{
  "questions": [
    { "prompt": "which branch should I target?", "default": "main" }
  ]
}
```

cap at 4 questions per call.

### plumbing (mirror permission requests)

the existing pattern is `AgentEvent::PermissionRequest { call, tx: oneshot::Sender<PermissionDecision> }` (`mew-agent/src/lib.rs:18`). copy it:

```rust
AgentEvent::UserQuestion {
    call_id: String,
    questions: Vec<UserQuestion>,
    tx: oneshot::Sender<Vec<String>>,
}
```

tool `execute` builds the oneshot, sends the event through `ctx.progress_tx`
(no — through the agent's `ev_tx`; needs the same channel the permission
requests use, which means `ToolCtx` may need a handle to it, or the agent
intercepts `ask_user_question` by name the way it intercepts `subagent_start`
and `exit_tool`). the latter is cleaner and matches existing precedent.

### TUI

new modal `draw_user_question_modal` in `ui/overlays.rs`, shaped like
`draw_permission_modal` but with a small input field per question. reuse the
input editing from `ui/input.rs`. on submit, send answers back through the
oneshot; tool formats them as Q&A text and returns as the tool result.

render inline-as-card (polytoken style) vs modal: modal is less work given
existing overlays; inline is better UX. recommend modal for v1, inline as
polish later.

### size

~300 LOC. small. the oneshot pattern is proven and the modal template exists.

---

## secret files + secret words

mew has workspace sandboxing (geography) but nothing for content. this is a
real safety hole.

### config

new section in `config.toml`, parsed in `mew-config`:

```toml
[[secrets.files]]
paths = [".env", "**/*.pem", "**/credentials.json"]

[[secrets.words]]
values = ["AKIA...", "ghp_..."]
```

### wiring

`PermissionEngine` (`mew-config/src/permissions.rs`) gains a pre-check: if the
tool is `read`/`grep`/`glob` and the resolved target matches a secret-file
pattern, force `Prompt` regardless of any allow rule — *unless* an allow rule
matches the exact command/path with no glob. polytoken's "only an exact-command
rule lifts the secret-file guard" maps cleanly onto mew's existing rule engine
by treating secret matches as a tier above deny.

for secret words in `grep`/`glob` output: filtering happens in the tool's
`execute` after collecting results, before returning. needs the secrets list
available to tools — add `secrets: Arc<SecretsConfig>` to `ToolCtx`
(`mew-tools/src/lib.rs:19`). redacted lines get a `[redacted]` marker and the
raw count is noted so the model knows something was hidden.

### size

~250 LOC. mostly in `mew-config` and the search tools.

---

## flag_important

mark a file as important for the session so it survives compaction. small tool,
big payoff for long sessions.

### tool

`flag_important(path, mode: "included" | "referenced")`. `included` inlines
content into post-compaction context; `referenced` records a pointer.

### wiring

`Agent.flagged_files: Vec<(PathBuf, FlagMode)>`. compaction in `turn.rs:116-156`
gains an injection step: after computing the kept tail, prepend each included
flagged file as a `FilePart` and each referenced one as a text note. flagged
files are part of the agent state, not the message log, so they're naturally
immune to compaction (which only mutates `messages`).

`AgentEvent::FlaggedFilesUpdated` for the TUI sidebar (polytoken shows a
"flagged files" pane).

### size

~150 LOC. tiny.

---

## /clear + the session/context split

polytoken draws a hard line: the *session* is the immutable event log on disk;
the *context* is what the model sees this turn. `/compact` and `/clear` mutate
context only. mew already has the raw material (append-only JSONL, in-memory
`agent.messages`) but doesn't name the distinction.

### /clear (cheap, land first)

reset `agent.messages` to empty, write a clear marker to the session log,
display clears. today `app.clear_messages()` (`app.rs:886`) only clears the
display store. add `agent.clear_context()` that also resets `messages` and
writes a synthetic marker message. resume reconstructs forward from the marker.

shell env reset (polytoken resets exported vars on clear) is a follow-up; mew's
bash tool doesn't currently persist env across calls in a way clear would need
to undo, so probably a non-issue.

### /rewind (medium, builds on the split)

needs event-indexed restore. load the session, find the message at the rewind
point, truncate `agent.messages` to that point, reload into the TUI. two
variants:

- **non-destructive (recommended):** keep the file intact, just move an
  in-memory "head" pointer. safer; user can rewind-forward if we later support
  branching. polytoken is destructive; we don't have to be.
- **destructive:** truncate the file at the rewind point. matches polytoken,
  simpler mental model, loses data.

can't rewind into the middle of a tool call — snap to the boundary before the
call or after its result, same as polytoken.

### size

/clear ~100 LOC. /rewind ~400 LOC (the indexing + boundary-snapping is the
work).

---

## shell command decomposition

today `bash` is gated as a whole tool: sensitivity + `command_prefix` glob
rules. polytoken parses the command into programs/subcommands/flags and matches
each independently. finer-grained, and it catches the
`eval`/`xargs`/`$(...)`/`python -c` evasion cases that whole-command gating
structurally misses.

### wiring

new helper (probably in a small `mew-shell` crate or a module under `mew-tools`)
that takes a command string and returns a parsed program list. `shell-words`
for tokenization; small hand-written pass for pipe/and/sequence operators.

`PermissionEngine::check` for `bash` changes: instead of one match against the
whole command, iterate parsed programs. each program is checked against the
rules; any program that doesn't hit an allow rule → prompt.

opaque-construct detection: `$(...)`, `<(...)`, `| xargs`, `| sh`, `eval`,
`bash -c`, `sh -c`, `python -c`, dynamic `cd "$VAR"`. when detected, skip rule
matching entirely and force prompt — the rule engine can't see what those will
run, so it can't responsibly auto-allow.

extend `MatchConditions` (`permissions.rs:26`):

```rust
pub struct MatchConditions {
    pub command_prefix: Option<String>,      // back-compat
    pub command_program: Option<String>,     // e.g. "git"
    pub command_subcommand: Option<String>,  // e.g. "push"
    pub path_glob: Option<String>,
}
```

### size

~500 LOC + a meaningful test suite. the parser is the hard part; rule wiring is
mechanical.

---

## hooks runtime parity

mew's `Dispatcher` trait (`mew-hooks/src/lib.rs:88`) is actually *wider* than
polytoken's hooks in coverage: `on_system_prompt`, `on_register_tools`,
`on_register_slash_commands`, `execute_slash_command`, plugin storage, plugin
UI, notifications. the gap is on the *runtime* side: polytoken's bash-handler
hooks have matchers, blocking outcomes, deadlines, and `!name` negation. mew's
`SubprocessDispatcher` (in `mew-hooks-runtime`) is thinner.

### event mapping (mostly already covered)

| polytoken event | mew hook |
|---|---|
| `pre_tool_use` | `on_permission_ask` + `on_tool_execute_before` |
| `post_tool_use` | `on_tool_execute_after` |
| `pre_user_prompt` | `on_chat_message` |
| `pre_model_turn` | *(new)* `on_pre_model_turn` |
| `post_model_turn` | `on_turn_end` |
| `stop` | *(new)* `on_stop` |
| `pre_compaction` | *(new)* `on_pre_compaction` |
| `post_compaction` | *(new)* `on_post_compaction` |
| `session_start` | `init` |
| `notification` | `PluginHost::notify` |
| `subagent_start` / `subagent_stop` | `on_event` (filtered) |

four new trait methods. all are `NopDispatcher` pass-throughs by default.

### runtime features to add (in `mew-hooks-runtime`)

these belong in the subprocess dispatcher, not the trait:

- **glob matchers per hook entry.** `hooks.json` entries gain a `matcher` field;
  the dispatcher fires only on match. tool events match on tool name, subagent
  events on subagent name.
- **blocking outcome protocol.** today `on_permission_ask` returns a
  `PermissionDecision`; generalize the blocking hooks to return a small outcome
  enum (`allow`/`deny`/`retry`/`suppress`) like polytoken. JSON on stdout, or
  exit-code shorthand (0 = proceed, 2 = stop with captured output).
- **deadlines.** a hung handler can't stall the loop. tokio timeout, treat
  over-deadline as error.
- **`!name` negation.** project `hooks.json` can disable an inherited global
  hook by name. cheap to add at load time.

the `ShellHookDispatcher` planned for m11 (cc plugin compat, `PLAN.md:1113`)
already needs most of this; doing it as a real m7.5 milestone means m11 becomes
truly thin.

### size

~600 LOC, mostly runtime. trait additions are small.

---

## jobs (async subagents + background shell)

promotes "job" to a first-class concept with lifecycle states
(working/completed/failed/cancelled) and observable progress.

### subagent side

`CURRENT.md` (M9.1, phase 5) already planned this: `subagent_start` gains
`async: bool = false`, default blocks. flip the default to `true`, make
`subagent_wait` the primary path, and surface the existing
`subagent_tasks` (`agent.rs:62`) as a job registry. the infrastructure is
already there — `SubagentTask` already has a cancel token and result oneshot.

### shell side

new `shell_background` tool (or `bash` with `background: true`) that launches
and returns a `job_id`. reuses the same job registry. plus
`job_status`/`job_block`/`job_result`/`job_cancel` tools.

### registry

`Agent.jobs: HashMap<JobId, JobState>`. new `AgentEvent::JobUpdate { id, state }`
for the sidebar. polytoken's `shell_monitor` (readiness polling until success
or deadline) is a nice addition for "wait for the dev server to be up."

### size

~700 LOC. the subagent async machinery mostly exists; the work is the job
registry surface and the shell-background tool.

---

## autonomous permission mode

> **STATUS (2026-06-21): ✅ shipped — went further than this section proposed.**
> See `CURRENT.md` sections "Dangerous! permission mode", "Auto permission mode (classifier-driven)",
> and "Auto+ permission mode (fail-closed classifier)". Five-mode slider: Standard / Permissive /
> Auto / Auto+ / Dangerous!. Classifier prompt and parser live in `mew-prompts::classifier`.

a third permission mode (after standard and bypass): a classifier model handles
routine approvals, escalates to human when unsure or after repeated refusals.

### wiring

new field on the agent or config: `permission_mode: Standard | Autonomous | Bypass`.
when `Autonomous`, a `Prompt` decision from `PermissionEngine` routes to a
classifier call instead of the TUI. classifier is a small `Provider` call with
tool name + input + (if personas exist) `mew.autonomous_hint`, returns
allow/ask/deny. "ask" or N refusals → escalate to human.

reuses the existing provider infra — point it at the router's small model or a
dedicated `classifier` provider in config.

### caveats

cost and latency per gated call. must be opt-in, never default. repeated-
refusal escalation logic is easy to get wrong; needs careful tests. worth
prototyping behind a flag, not shipping as the out-of-box mode.

### size

~400 LOC + classifier prompt iteration. the risky part is the prompt, not the
code.

---

## explicit non-goals for now

> **STATUS (2026-06-21):** daemon/TUI split still deferred; `tool_search` still
> deferred; plan/execute never built (personas shipped standalone instead).
> All three remain non-goals — none have hit the "worth the cost" bar yet.

- **daemon/TUI split with `/detach`.** real architecture cost (ipc, lifecycle,
  re-attach state sync). iroh already gives remote access; acp-server already
  gives headless. marginal value is low for us. defer.
- **`tool_search` / deferred tool loading.** only matters at high tool count or
  with heavy mcp servers. skip until it hurts.
- **personas + plan/execute + plan-reviewer as one bundle.** ship personas
  standalone first; plan mode is a follow-up milestone that depends on personas.
  **(done: personas shipped standalone. plan mode remains unbuilt.)**

---

## suggested sequencing

three tiers. within each tier, order is flexible.

**status:** all three tiers shipped as of 2026-06-21. Tier 1 + Tier 2 fully done; Tier 3 done with extras (Auto+ mode). The hooks-runtime-parity item in Tier 2 is the only partial item.

**tier 1 (small, high-value, mostly independent) — ✅ all shipped:**
1. ✅ todos
2. ✅ ask_user_question
3. ✅ secret files + words
4. ✅ flag_important
5. ✅ /clear

any of these can land in any order. todos and ask_user_question are the ones
we already want; secrets and flag_important are cheap safety/quality wins; /clear
is the foundation for the session/context vocabulary.

**tier 2 (medium, some dependencies) — ⚠️ 3/4 shipped:**
6. ✅ personas (standalone, no templating, no plan mode)
7. ✅ /rewind (builds on /clear's vocabulary)
8. ✅ shell decomposition (independent)
9. ⚠️ hooks runtime parity (independent; sets up m11) — 21 hooks + matchers + deadlines landed; `HookOutcome` enum + `!name` negation still open

personas is the centerpiece. the others can interleave.

**tier 3 (larger or riskier) — ✅ all shipped:**
10. ✅ jobs (async subagents + background shell)
11. ✅ minijinja templating (only if personas shipped and someone needs it)
12. ✅ autonomous permission mode (opt-in, behind a flag) — went further: `Auto` (LLM routes, escalates) + `Auto+` (fail-closed variant) both shipped
13. ⛔ plan/execute workflow (builds on personas) — deferred indefinitely; personas shipped standalone per the original plan, no plan-reviewer subagent or BLAKE3 hashing built yet

---

## open questions

> **STATUS (2026-06-21): all four still open.** No urgency — the current shapes
> are working in production. Worth revisiting if any of them starts to hurt.

- does `mew-skills` get refactored into a shared `mew-harness` crate with
  loaders for skills + personas + subagents, or do we port the loader per-type?
  the three are 90% identical; a shared crate kills duplication. **(still three
  separate crates; no refactor yet. not painful enough to warrant the work.)**
- tool trait currently takes `ToolCtx` by value in `execute`. secrets +
  flagged-files + active-persona all want richer context. is it time to make
  `ToolCtx` carry an `Arc<ToolCtxShared>` with the growing shared state, or
  keep adding fields? **(still adding fields. `ToolCtx` now carries `secrets`,
  `dispatcher`, etc. Field count is creeping up but no concrete pain yet.)**
- `/rewind` destructive or non-destructive? i lean non-destructive (in-memory
  head pointer) but it's a real product decision. **(resolved: non-destructive.
  see `App::rewind_to` and `test_rewind_to_*` tests.)**
- where does the mew base system prompt live once personas exist? today it's
  implicit (ctx files + skills). personas force the question of whether mew
  ships a default persona body the way polytoken does
  (`transclude("polytoken://system_prompts/persona.md")`).
  **(still implicit. `mew-prompts::system::assemble` joins ctx + skills +
  persona body in a fixed order, but there's no shipped "mew base" body.
  Could ship one as a built-in `default` persona when this becomes urgent.)**
