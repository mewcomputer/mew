# Progress — 2026-06-21

## Personas v2 polish (all committed)

- **Sidebar section**: collapsible "Personas" section between Tools and MCP, active persona marked in purple
- **Confirm modal**: `/persona <name>` shows a diff (model, tools, deny, skills changes) before applying. Confirm/Cancel buttons, y/n shortcuts
- **switch_persona tool**: model-callable, queues switch at end-of-turn (not mid-turn), gated by PermissionEngine (Mutating sensitivity)
- **tools_deny**: PersonaConfig extended with denylist. Applied after allowlist in turn.rs tool filter
- **skills allowlist**: PersonaConfig gains `skills` field. Skill tool gates by filter, system prompt skills XML rebuilt on persona change
- **Minijinja templating**: `template: true` in frontmatter renders persona body with `supports_vision`, `tools`, `denied_tools`, `persona_name`

## Secrets (all committed)

- **Config**: `secrets.files.paths` + `secrets.words.values` in config.toml
- **Permission pre-check**: `read` of secret-file glob forces Prompt unless literal allow rule
- **Redaction**: `secrets.rs` module with `redact_secret_words()` — replaces values with `[REDACTED]`, preserves structure
- **Wired into**: Read, Bash (before truncation), Grep (drops secret-file lines + redacts words), Glob (drops secret-file results)

## Shell command decomposition (all committed)

- **`mew-config/src/shell.rs`**: splits compound commands on `|`, `&&`, `;`, respects quotes
- **Opaque detection**: `$(...)`, backticks, `<(...)`, `eval`, `bash -c`, `| sh`, `| xargs` → force Prompt
- **PermissionEngine**: compound commands require all programs allowed; single commands use prefix matching (backward compat)
- **MatchConditions**: `command_program` + `command_subcommand` fields added

## Hooks runtime overhaul (all committed)

- **21 hooks** in Dispatcher trait (was ~12, many unwired)
- **HookId enum**: single source of truth (as_wire, as_config, ALL, Display)
- **PluginHookConfig**: per-plugin `disabled_hooks`, `matchers`, `timeout_ms`
- **on_register_tools**: subprocess plugins can now register async tools (ToolRegistration::execute is now async)
- **Parallel dispatch**: `pipe_json_filtered` uses `join_all` (latency = max, not sum)
- **Plugin health**: `healthy: AtomicBool`, writer drain on crash, user notification
- **config_read**: wired to PluginInfo (session_id, model, provider, workspace, active_persona)
- **ChatParams/headers**: flow through to provider Request, OpenAI adapter uses them
- **New hooks**: on_user_input, on_persona_change, on_session_save, on_model_finish, on_provider_event, on_tool_error, on_subagent_start/end, on_pre_model_turn, on_stop, on_pre/post_compaction

## Telemetry

- **telemetry-exporter.rs** example plugin: Prometheus /metrics endpoint on :9090
- Collects token/cost/tool/turn metrics from hooks
- Zero external deps (std::net only)

## Decisions made

- `on_event(&dyn Any)` removed entirely — replaced with specific typed hooks (was !Send, couldn't forward to subprocess)
- `pipe_json_filtered` changed from sequential (pipe) to parallel (broadcast, last alphabetical wins) — documented as behavior change
- Plugin restart deferred — needs Arc<Mutex<Option<PluginProcess>>> per slot (documented as TODO)
- ProviderEvent derives Serialize/Deserialize (field: &'static str limitation noted for Rust in-process plugins)

## Jobs (background shell + job control)

- **Shell job registry** (`mew-agent/src/agent.rs`): `ShellJob` struct (id, command, started_at, cancel token, accumulated output, state, done Notify) + `ShellJobState` enum (Running / Completed{exit_code} / Failed{reason} / Cancelled). Stored in `Agent.shell_jobs: Arc<Mutex<HashMap<String, ShellJob>>>`, mirroring the subagent_tasks registry.
- **Five tools** (`mew-tools/src/tools/jobs.rs`, intercepted by agent core — same pattern as `subagent_start`):
  - `shell_background` — launches detached, returns job ID immediately (Dangerous)
  - `job_status` — state + accumulated output so far (ReadOnly)
  - `job_block` — wait for terminal state up to timeout_secs, returns final output (ReadOnly)
  - `job_cancel` — kills the process (Mutating)
  - `shell_monitor` — readiness polling: launches via shell_background, blocks until exit 0 or timeout (Dangerous)
- **AgentEvent::JobUpdate** { job_id, command, state }: emitted from job lifecycle (start, status-check, cancel). Emitted as string `state` ("running"/"completed"/"failed"/"cancelled") for clean serialization across the ACP boundary.
- **Sidebar "Background Jobs" section** (`mew-tui/src/ui/sidebar.rs`): mirrors the Subagents block. Shows icon (▸/✓/✗/⊘), truncated command, elapsed time, and status label. `BackgroundJobState` on App updated via `handle_agent_event` — existing entries transition state in place (preserving started_at); unknown job_ids are inserted.

## Next up

- Jobs milestone complete. No outstanding feature work queued; pending items are housekeeping (this doc) and any polish the user requests.

## Landing / start page redesign (2026-06-21)

- **Centered start screen** instead of the old "welcome content floats up top, input pinned to bottom" layout. When `app.messages.is_empty()`, `ui/mod.rs::draw` now takes an early landing branch: a centered cat + bold "mew" hero with the input directly beneath it, plus the status line still pinned to the bottom. Reverts to the normal bottom-pinned layout automatically once the first message lands (transition trigger: first sent message, per operator decision).
- **Hero composition** (`ui/welcome.rs::draw_landing`, replaces the old `draw_welcome`): the pre-existing-but-unused `CAT` ASCII const is now wired in, rendered green (matching the companion sprite) with the bold "mew" wordmark to its right on the cat's face line. Hero is centered as a fixed-width, left-aligned block so the cat art stays coherent.
- **Centered input**: ~60% width clamped to [30, 80], rendered via the existing `draw_input` (unchanged — it already takes a rect and sets the cursor). Hero + input are centered as one vertical unit so the cat reads as hovering just above the field. Slash autocomplete is supported on the landing screen too (drawn into the rows directly above the centered input).
- **Overlay refactor**: factored the alert/permission/user-question/persona-confirm/command-palette block out of `draw()` into a shared `draw_overlays(f, app, area)` helper used by both the landing and normal branches, so both respond identically to those modes.
- **chat.rs cleanup**: removed the now-unreachable `draw_welcome` call + import from `draw_chat` (mod.rs intercepts the empty-messages case before `draw_chat` runs).
- **Incidental clippy fix**: `chat.rs` had a pre-existing `useless_format` lint on the "thinking [Ctrl-T to collapse]" header (present in HEAD, flagged under toolchain 1.94.0). Applied clippy's suggested `.to_string()` so the `-D warnings` gate stays green. Unrelated to the feature; called out for transparency.
- **Verified**: `cargo build -p mew`, `cargo clippy --all -- -D warnings`, `cargo test --all` all pass.

## Permission semantic audit (2026-06-21)

Audit of the three permission paths before any new feature (autonomous mode, plan/execute, worktree-scoped allows) lands on top of the foundation.

### Three paths into the permission prompt

1. **Regular tool call → `PermissionRequest`** (`mew-agent/src/tools.rs:140-230`): engine decides `AllowOnce` / `AllowSession` / `Deny` / `Prompt` via `PermissionEngine::check`. If `Prompt`, the dispatcher hook `on_permission_ask` runs first; if it doesn't bypass, an `AgentEvent::PermissionRequest` is emitted and the agent blocks on the oneshot. TUI consumer at `app.rs:1828` renders the standard modal; keypress `1` sends `AllowSession` (`events.rs:305, 323`). On `AllowSession`, the agent calls `engine.add_session_allow(&tool_name)` at `tools.rs:186-190`. Test: `tests.rs:687-714`.
2. **Workspace sandbox → `WorkspacePermissionRequest`** (`mew-agent/src/workspace.rs:31-58`): triggered by `ensure_workspace_path` at `tools.rs:349` for path-based tools only (`read`/`write`/`edit` from `input.path`; `glob`/`grep` defaulting to `.`). Bash and echo are explicitly skipped (the `workspace.rs:67-77` path extractor returns `None` for them). Fires when resolved path is outside `workspace_roots` AND not in `workspace_allowances`. The *containing directory* (not the file) is added to `workspace_allowances` on `AllowOnce`/`AllowSession`. TUI consumer at `app.rs:1828` sets `tool_name = "workspace"` and reuses the same modal.
3. **Subagent's tool call → `SubagentPermissionRequest`** (`mew-agent/src/tools.rs:723-741`): the subagent emits `mew_subagents::SubagentEvent::PermissionRequest { tool_name, call_id, input, tx }`; the parent forwards it as `AgentEvent::SubagentPermissionRequest`. Each subagent has its own `PermissionEngine`, so an `AllowSession` decision updates the subagent's `session_allows`, not the parent's. TUI consumer at `app.rs:1818` reuses the standard `PermissionState` modal.

### Two product decisions resolved

- **`/clear` keeps both permission caches.** `permission_engine.session_allows` and `workspace_allowances` are tied to the *session* lifetime (the JSONL log on disk), not the *context* (what the model sees this turn). `/clear` resets the visible context and writes a synthetic marker to the log, but prior `AllowSession` grants and prior outside-workspace directory allowances persist within the session. The rationale: a "session" really is the JSONL log; a "context" is the visible turn. Clearing the latter doesn't invalidate the user's prior grants within the former. Resolved with a doc comment on `Agent::clear_context` (`agent.rs:378-394`) and a new test `tests.rs::test_clear_context_preserves_permission_caches` that pins the behavior.
- **Mutating and Dangerous collapse to `Prompt` in the default cascade.** `PermissionEngine::check` only differentiates `ReadOnly` (`AllowOnce`); both `Mutating` and `Dangerous` fall through to `Prompt`. The three-way sensitivity split is informational only — used in modal labels but not in the default decision. Existing tests already pin this: `permissions.rs::test_default_mutating_prompt` (line 354) and `test_dangerous_prompt` (line 367). Users can opt into auto-allow via session-allow or declarative rules in `config.toml`.

### Audit gaps still open (deferred)

- **No bypass / YOLO mode.** No `--dangerously-skip-permissions` or `MEW_BYPASS_PERMISSIONS=1`. The CLI ergonomics gap users coming from `claude --dangerously-skip-permissions` will notice. Small addition; doesn't affect the resolved semantics above.
- **No shared `HookOutcome` enum across blocking hooks.** `PermissionDecision` (`AllowOnce` / `AllowSession` / `Deny` / `Prompt`) is essentially the outcome enum for `on_permission_ask`, but other blocking hooks (`on_tool_execute_before`, `on_chat_message`, `on_chat_params`, `on_chat_headers`) have their own return types. Generalizing is a small-medium refactor in `mew-hooks` + `mew-hooks-runtime`.
- **Plan mode would benefit from a "read-only toolset" permission stance** (per the parity doc), but plan mode doesn't exist yet. When it lands, gating should go through the permission engine, not a parallel code path.

### Verified

- `cargo test -p mew-agent clear_context` — 3/3 pass (including the new `test_clear_context_preserves_permission_caches`).
- `cargo clippy -p mew-agent -- -D warnings` — clean.

## Dangerous! permission mode (2026-06-21)

Adds an opt-in permission slider with three modes (`Standard` / `Permissive` / `Dangerous!`), wired through a `PermissionMode` enum, a runtime mode field on `PermissionEngine`, a `/permissions` slash command with a picker, status-line badges, and two CLI flags.

### Three-mode hierarchy

The modes form a permission slider from most to least restrictive:

| Mode | ReadOnly | Mutating | Dangerous | Deny rules | Ask rules | Secret guard | Bash decomp |
|---|---|---|---|---|---|---|---|
| **Standard** (default) | AllowOnce | Prompt | Prompt | Fire | Fire | Fire | Fire |
| **Permissive** | AllowOnce | **AllowOnce** | Prompt | Fire | Fire | Fire | Fire |
| **Dangerous!** | AllowOnce | AllowOnce | AllowOnce | **Skip** | **Skip** | **Skip** | **Skip** |

The naming distinguishes `Permissive` from the parity doc's "Auto" / classifier mode (which would route prompts through a small LLM instead of a human — separate future feature).

### Data layer

- **`PermissionMode` enum** (`mew-hooks/src/lib.rs:165-208`): `Standard` (default), `Permissive`, `Dangerous`. Includes `from_id()` / `id()` / `picker_label()` helpers.
- **`PermissionEngine` mode field** (`mew-config/src/permissions.rs:52, 62, 75-104`): `Arc<AtomicU8>` for lock-free reads on the hot path. `with_mode(mode)` constructor variant for initial mode, `set_mode(mode)` for runtime toggling, `mode()` getter (handles all three variants explicitly).
- **Mode-aware cascade in `check()`** (`permissions.rs:118-247`):
  - **Dangerous**: short-circuits to `AllowOnce` for everything at step 0.
  - **Standard + Permissive**: secret-file guard fires (step 1), bash decomposition fires (step 2), deny rules fire (step 3).
  - **Permissive only**: skips allow/ask/session-allow/default cascade; falls into `check_permissive_mode(sensitivity)` which returns `AllowOnce` for ReadOnly/Mutating, `Prompt` for Dangerous.
  - **Standard only**: full cascade — allow rules, ask rules, session-allow cache, sensitivity default.

### `Agent::set_permission_mode(mode)` forwarder

`mew-agent/src/agent.rs:236-249` — calls `engine.set_mode(mode)` if a permission engine is set; no-op otherwise. Cheap atomic store; takes effect on the next `check()` call.

### `/permissions` slash command

- Added to `App::builtin_slash_commands()` (`mew-tui/src/app.rs:1234-1237`).
- Routes through `handle_slash` (`app.rs:1373-1386`): `/permissions` opens the picker; `/permissions standard|permissive|dangerous` switches directly. Unknown args produce a `SlashResult::Message` with usage help.
- `App::open_permission_mode_picker()` (`app.rs:524-568`) reuses the cmdk-style picker pattern (mirrors `open_model_picker`). Three items, ordered from most to least restrictive. Each item marked with `● active` for the current mode; pre-selects the active item so Enter on unchanged state is a no-op.
- `SlashResult::PermissionModeMenu` and `SlashResult::SetPermissionMode(PermissionMode)` variants added; handled in all three dispatch sites in `main.rs` (lines 1657-1681, 1949-1964, and 2010-2024).
- `Action::SetPermissionMode(PermissionMode)` added to `events.rs:761-764`; picker dispatch at `events.rs:670-672`; main-loop handlers in `main.rs:1759-1784` and `1985-2006`. Each handler calls `agent.set_permission_mode()`, updates `app.permission_mode`, and shows a mode-specific alert.

### Status-line badges

`mew-tui/src/ui/status.rs:56-77` — `build_pills()` prepends a mode pill:
- **Standard**: no pill (implicit default)
- **Permissive**: amber "Permissive" pill (medium-risk cue)
- **Dangerous!**: red "⚠ Dangerous!" pill (high-risk cue)

The pill is the first item, before the model/persona/cwd/git pills, so the user always sees the mode state before anything else.

### CLI flags

`mew chat/run/acp`:
- `-D` / `--dangerously-skip-permissions` / `MEW_DANGEROUS=1` → starts in Dangerous! mode
- `-P` / `--permissive` / `MEW_PERMISSIVE=1` → starts in Permissive mode
- `-D` wins over `-P` if both are set (Dangerous is the stronger override)

A `resolve_mode(permissive, dangerous)` helper (`main.rs:1015-1023`) folds the two flags into a single `PermissionMode`. Threads through `run_cmd` / `chat_cmd` / `run_acp_server` / `run_tui` / `build_and_run` and into `build_permission_engine(cfg, mode)`. The mode can be toggled at runtime via `/permissions`, so the CLI flags just set the initial state.

### Tests

- `mew-config/src/permissions.rs` — 17 new tests across Standard / Permissive / Dangerous, plus the cascade interaction tests (e.g., `test_permissive_mode_respects_deny_rules`, `test_dangerous_mode_overrides_deny_rules`, `test_permissive_mode_respects_secret_guard`).
- `mew-tui/src/app.rs` — 7 new tests for the slash command + picker (3 items, perm/danger/standard routing, active-mode marker, pre-selection at each of the three indices).
- `mew-tui/src/ui/status.rs` — 3 new tests covering Standard (no pill), Permissive (amber pill), and Dangerous! (red pill).

### Verified

- `cargo test --all` — 458/458 pass.
- `cargo clippy --all -- -D warnings` — clean.
- `mew chat --help` shows `-P, --permissive ... [env: MEW_PERMISSIVE=]` and `-D, --dangerously-skip-permissions ... [env: MEW_DANGEROUS=]`.

### Decisions worth noting

- **Dangerous! mode overrides EVERYTHING, including user-configured deny rules.** The user has explicitly opted into "no holds barred." Half-override (some rules fire, some don't) is confusing — either you're in "trust me, don't ask anything" mode or you're not. Pinned by `test_dangerous_mode_overrides_deny_rules`.
- **Dangerous! mode does NOT override secret redaction in tool output.** It only skips the prompt; `Read` / `Bash` / `Grep` still redact values when displaying. Out of scope for the permission gate; lives in tool `execute()`.
- **Permissive mode still respects deny rules, ask rules, secret-file guard, and bash decomposition.** Only the prompt-for-Mutating tier is lifted. "I trust the agent with file edits but bash and my safety rules still apply."
- **`Auto` (classifier-driven approval) is a separate future feature**, per the parity doc. Naming was reserved to keep the door open. Pinned in the `PermissionMode` enum's doc comment.
- **`chat_with_acp` doesn't propagate the flag yet** (`_mode` prefixed). ACP client mode is a thin wrapper around an external subprocess agent — the subprocess's own permissions apply. Could revisit if we add mew-as-ACP-server tests later.

## System prompts centralization (`mew-prompts` crate, 2026-06-21)

Created a new crate `crates/mew-prompts/` as the single home for every prompt mew sends to the LLM (system, persona, skills, subagent, classifier). One crate to look at when you ask "where does the system say X to the model?" Sets up Auto (the next milestone) so its classifier prompt has a place to land.

### Submodules

- `mew-prompts::system` — base system prompt assembly. Re-exports `mew_context::build_system_prompt` as `build_context`; adds `assemble(ctx_files, skills_xml, persona_body) -> String` that joins context → skills → persona in standard order.
- `mew-prompts::persona` — persona body rendering. Owns `render_template(body, persona_name, supports_vision, active_tool_names, all_tool_names, denied_tool_names) -> String`. Wraps `minijinja::Environment::new().render_str(...)` with a fall-back to the raw body on render error. Signature uses `&[String]` (just tool names) to avoid pulling in `mew-tools` and creating a workspace cycle.
- `mew-prompts::skills` — `<available_skills>` XML block. Owns `build_xml(skills: &[&Skill]) -> String` and the XML-escape helper.
- `mew-prompts::subagent` — built-in subagent system prompts. Owns `RESEARCHER_BODY`, `REVIEWER_BODY`, `CODER_BODY` constants and `builtin_bodies()` returning `Vec<(&'static str, &'static str)>` for the inventory.
- `mew-prompts::classifier` — permission-decision prompt for Auto mode (stub). Owns `permission_decision(tool_name, input, sensitivity, cwd, recent_action) -> String` and a `ClassifierDecision` enum with a `parse(&str) -> Option<Self>` for the classifier LLM's response.
- `mew-prompts::inventory` — `PromptSource { id, location, kind, description, preview }`, `PromptKind { System, User, Classifier }`, and `inventory() -> Vec<PromptSource>` listing every prompt the crate knows about.

### Migrations

| From | To | Notes |
|---|---|---|
| `mew-agent/src/agent.rs::render_persona_template` | `mew_prompts::persona::render_template` | Signature simplified: `all_tools: &HashMap<String, Arc<dyn Tool>>` → `all_tool_names: &[String]` (avoids workspace cycle). Thin re-export kept in `mew-agent` so call sites at `rebuild_system()` work unchanged. |
| `mew-agent/src/agent.rs::build_skills_xml` | `mew_prompts::skills::build_xml` | Thin re-export kept in `mew-agent` for the same reason. |
| `mew-subagents::builtin_defaults` body strings (researcher/reviewer/coder) | `mew_prompts::subagent::{RESEARCHER_BODY, REVIEWER_BODY, CODER_BODY}` | `builtin_defaults` now references the constants via `mew_prompts::subagent::RESEARCHER_BODY.into()`. `SubagentDef` struct stays in `mew-subagents`. |

### Workspace cycle resolution

`mew-tools` depends on `mew-subagents` (for `SubagentDef`). When `mew-subagents` started depending on `mew-prompts`, and `mew-prompts` was considering `mew-tools` for `Sensitivity`, cargo rejected the cycle:

```
mew-tools → mew-subagents → mew-prompts → mew-tools  ❌
```

Resolution: `mew-prompts` does NOT depend on `mew-tools`. The `persona::render_template` signature takes `&[String]` for tool names (no `Tool` trait needed). The `classifier::permission_decision` signature takes `sensitivity: &str` (caller converts from `mew_tools::Sensitivity` if it has it).

### What this gives Auto

When Auto lands later, the classifier prompt is already in `mew-prompts::classifier::permission_decision(...)`. The classifier-side response parsing is `ClassifierDecision::parse(&str)`. Auto only needs to:
1. Wire up the provider call (small LLM).
2. Call `permission_decision(...)`, send the result, parse the response with `ClassifierDecision::parse(...)`.
3. Translate the decision into the existing `PermissionDecision` enum and route through the current cascade.

No new prompt format work; no new text scattered around the codebase.

### Verified

- `cargo test --all` — 484/484 pass (was 458; +26 in `mew-prompts`).
- `cargo clippy --all -- -D warnings` — clean.
- `cargo build -p mew` — full binary builds.

### Files touched

- New: `crates/mew-prompts/{Cargo.toml, src/lib.rs, src/system.rs, src/persona.rs, src/skills.rs, src/subagent.rs, src/classifier.rs, src/inventory.rs}`
- Edited: `Cargo.toml` (workspace member + path dep), `crates/mew-agent/Cargo.toml` (added mew-prompts, removed minijinja), `crates/mew-agent/src/agent.rs` (replaced two functions with one-line wrappers, updated call site), `crates/mew-subagents/Cargo.toml` (added mew-prompts), `crates/mew-subagents/src/lib.rs` (replaced inline body strings with constants)

## Auto permission mode (classifier-driven, 2026-06-21)

A fourth mode that routes every tool call through a small/cheap LLM instead of the user. The classifier returns `allow` / `deny` / `escalate`; `escalate` falls back to the existing user modal. Per the parity doc's framing — "Auto mode is effectively Dangerous mode with a LLM in front" — the classifier is the only gate; deny rules, ask rules, the secret-file guard, and bash decomposition are all skipped.

### Behavior matrix

| | Standard | Permissive | **Auto (new)** | Dangerous! |
|---|---|---|---|---|
| ReadOnly | AllowOnce | AllowOnce | **classifier** | AllowOnce |
| Mutating | Prompt | AllowOnce | **classifier** | AllowOnce |
| Dangerous | Prompt | Prompt | **classifier** | AllowOnce |
| Deny rules | Fire | Fire | Skip | Skip |
| Ask rules | Fire | Skip | Skip | Skip |
| Secret guard | Fire | Fire | Skip | Skip |
| Bash decomp | Fire | Fire | Skip | Skip |
| Escalate (classifier "ask") | n/a | n/a | → user modal | n/a |

### Data layer

- **`PermissionMode::Auto`** variant (`mew-hooks/src/lib.rs:165-205`). `from_id("auto")` and `id() == "auto"`. `picker_label() == "Auto"`. `mode()` getter in `PermissionEngine` extended to handle all four variants explicitly.

### `PermissionEngine` cascade

`permissions.rs:check()` now has two short-circuits at the top:
- Mode is `Dangerous` → `AllowOnce`.
- Mode is `Auto` → `Prompt`. Every tool call requires a classifier decision; the engine just signals "this needs a decision" and the agent routes to the classifier. If the classifier is unconfigured or errors, the agent falls through to the user modal — the safe default.

### `Agent::classify_permission`

`mew-agent/src/agent.rs` — new method that:

1. Builds the classifier prompt via `mew_prompts::classifier::permission_decision(tool, input, sensitivity, cwd, recent_action)`.
2. Sends a single-turn message to the classifier provider with `temperature: 0.0, max_tokens: 8` (a classification prompt shouldn't ramble).
3. Collects text from the `PartDelta` events.
4. Parses with `mew_prompts::classifier::ClassifierDecision::parse(text)` (handles `allow`/`deny`/`escalate` plus synonyms like `approved`, `block`, `unsure`).
5. Returns `None` on provider error / timeout / unparseable response — callers treat `None` as escalate-to-user.

### Wiring into the permission flow

`mew-agent/src/tools.rs:170-194` — between `on_permission_ask` and `PermissionRequest`:

```rust
let decision = if decision == PermissionDecision::Prompt
    && self.permission_mode() == mew_hooks::PermissionMode::Auto
{
    match self.classify_permission(&hook_call).await {
        Some(Allow)    => AllowOnce,
        Some(Deny)     => Deny,
        Some(Escalate) | None => Prompt,  // falls through to user modal
    }
} else { decision };
```

### `Agent` classifier provider storage

- `classifier_provider: Option<Arc<dyn Provider>>` and `classifier_model: Option<String>` fields on `Agent`.
- `set_classifier_provider(provider, model)` setter.
- If unset when Auto is active, `classify_permission` returns `None` and the call falls through to the user modal.

### Picker + status + CLI

- **Picker** (`mew-tui/src/app.rs::open_permission_mode_picker`): 4 items now, ordered Standard → Permissive → Auto → Dangerous!. Each item has its description; Auto's says "Routes every tool call through a small/cheap LLM classifier. Classifier returns allow/deny/escalate; escalate falls back to the user modal."
- **Status pill** (`mew-tui/src/ui/status.rs::build_pills`): purple "Auto" pill (RGB 240,230,250 on 95,50,130). Distinct from amber Permissive and red Dangerous! — visual cue matches the *kind of decision-maker*: human / LLM / deterministic.
- **CLI flag**: `-A` / `--auto` / `MEW_AUTO=1` on `chat`/`run`/`acp`. `resolve_mode(permissive, auto, dangerous)` precedence: **Dangerous > Auto > Permissive > Standard**. Documented in the help text and the resolver doc comment.

### Subcommand routing

Updated `/permissions auto` slash command routing and all three main-loop `Action::SetPermissionMode` alert sites to mention Auto. Updated `/permissions <unknown>` error message to list the four valid values.

### Tests

- `mew-config/src/permissions.rs` — 5 new engine tests: `test_auto_mode_short_circuits_to_prompt`, `test_auto_mode_prompts_even_for_readonly`, `test_auto_mode_skips_deny_rules`, `test_auto_mode_skips_secret_guard`, `test_auto_mode_skips_opaque_bash_prompt`.
- `mew-tui/src/app.rs` — `test_permissions_slash_with_auto_arg`, `test_permission_mode_picker_has_four_items`. Updated `test_permission_mode_picker_preselects_active` (Dangerous now at index 3, was 2).
- `mew-tui/src/ui/status.rs` — `test_build_pills_auto_mode_prepends_purple_pill` (asserts the pill is purple, not amber or red).

### Verified

- `cargo test --all` — **492/492 pass** (was 484; +8 from Auto mode).
- `cargo clippy --all -- -D warnings` — clean.
- `mew chat --help` shows `-A, --auto ... [env: MEW_AUTO=]` alongside `-P` and `-D`.

### Decisions worth noting

- **Classifier is the only gate in Auto mode.** Deny rules, ask rules, secret-file guard, and bash decomposition all skip — the user explicitly chose "let the model decide," so we don't second-guess it. If the user wants safety tiers, they pick Standard or Permissive.
- **Failure modes default to the user.** Provider error, timeout, malformed response → `classify_permission` returns `None` → escalate to user modal. Auto is never more permissive than the user.
- **Dangerous wins over Auto when both flags are set.** Both bypass all gates, but Dangerous is the deterministic-bypass signal and Auto is the LLM-mediated one. The user probably didn't intend to combine them; Dangerous is the stronger override.
- **Subagent inheritance not yet wired.** Subagents currently inherit the parent's permission mode by virtue of the same `permission_engine` reference, but classifier calls would currently hit the parent's classifier provider. If a subagent needs its own classifier (different model, different cost profile), that's a follow-up.
- **No caching yet.** Each tool call = one classifier call. Could be optimized later with a hash-keyed session cache (tool_name + input hash → last decision for N seconds) if latency/cost becomes a concern.

### Files touched

- `mew-hooks/src/lib.rs` — `PermissionMode::Auto` variant + `from_id`/`id`/`picker_label` for it.
- `mew-config/src/permissions.rs` — `mode()` handles Auto; cascade short-circuits Auto to `Prompt`; 5 new tests.
- `mew-agent/src/agent.rs` — `classifier_provider` / `classifier_model` fields; `set_classifier_provider` setter; `classify_permission` method; `sensitivity_label` helper.
- `mew-agent/src/tools.rs` — Auto routing in the permission flow (between `on_permission_ask` and `PermissionRequest`).
- `mew-tui/src/app.rs` — picker has 4 items; slash command routes `auto`; tests.
- `mew-tui/src/ui/status.rs` — purple "Auto" pill; status pill test.
- `mew/src/main.rs` — `--auto` / `-A` / `MEW_AUTO=1` flag on all three subcommands; `resolve_mode(permissive, auto, dangerous)`; alert text for Auto in all three dispatch sites.

## Auto+ permission mode (fail-closed classifier, 2026-06-21)

A fifth mode that runs the classifier like Auto but cannot escalate. If the classifier returns "escalate" or the call fails (provider error / timeout / malformed response), the call is **denied** — fail closed. The user picked this when they said "auto+ mode where the classifier can not escalate," choosing the safer "uncertainty means no" semantic over Auto's "fall back to user."

### Behavior matrix (Auto vs Auto+)

| | Auto | **Auto+ (new)** |
|---|---|---|
| Classifier returns `allow` | AllowOnce | AllowOnce |
| Classifier returns `deny` | Deny | Deny |
| Classifier returns `escalate` | → user modal | **Deny** |
| Classifier error / timeout / malformed | → user modal | **Deny** |
| Classifier unconfigured | → user modal | **Deny** (safe default) |
| Retry on failure | none | none |

Auto+ is identical to Auto at the engine level (both short-circuit to `Prompt` in `PermissionEngine::check()`). The difference lives in the agent's classifier wiring.

### Data layer

- **`PermissionMode::AutoPlus`** variant (`mew-hooks/src/lib.rs`). `from_id("auto_plus")` and `from_id("autoplus")` both parse (so users can type either). `id() == "auto_plus"`. `picker_label() == "Auto+"`. `mode()` getter in `PermissionEngine` extended to handle 5 variants explicitly.

### `Agent` classifier wiring

`mew-agent/src/tools.rs:170-201` — restructured the Auto routing into a single match that branches on mode for the Escalate and None cases:

```rust
match self.classify_permission(&hook_call).await {
    Some(Allow) => AllowOnce,
    Some(Deny)  => Deny,
    Some(Escalate) => match self.permission_mode() {
        AutoPlus => Deny,
        _        => Prompt,  // Auto → user modal
    },
    None => match self.permission_mode() {
        AutoPlus => Deny,
        _        => Prompt,  // Auto → user modal
    },
}
```

No retry — the user wanted fail-closed, not fail-closed-after-retry.

### Picker + status + CLI

- **Picker** (`mew-tui/src/app.rs::open_permission_mode_picker`): 5 items now, ordered Standard → Permissive → Auto → **Auto+** → Dangerous!. Each item has its description; Auto+'s reads "Like Auto, but the classifier CANNOT escalate. Escalate or any classifier failure → Deny (fail closed). Hands-off but uncertainty means no."
- **Status pill** (`mew-tui/src/ui/status.rs::build_pills`): deeper purple "Auto+" pill (RGB 250,235,255 on 70,25,110) — same purple hue as Auto but darker/more saturated. Distinct from amber Permissive, regular-purple Auto, and red Dangerous!. Visual cue: "Auto family, more committed."
- **CLI flag**: `--auto-plus` / `MEW_AUTO_PLUS=1` on `chat`/`run`/`acp`. No short alias. `resolve_mode(permissive, auto, auto_plus, dangerous)` precedence: **Dangerous! > Auto+ > Auto > Permissive > Standard**.

### Subcommand routing

Updated `/permissions auto_plus` slash command routing (also accepts `autoplus`). Updated all three main-loop `Action::SetPermissionMode` alert sites to mention Auto+. Updated `/permissions <unknown>` error message to list the five valid values.

### Tests

- `mew-config/src/permissions.rs` — 2 new engine tests: `test_auto_plus_mode_short_circuits_to_prompt`, `test_auto_plus_mode_skips_deny_rules`.
- `mew-tui/src/app.rs` — 3 new tests: `test_permissions_slash_with_auto_plus_arg`, `test_permission_mode_picker_has_five_items`, `test_permission_mode_picker_preselects_autoplus`. Updated `test_permission_mode_picker_preselects_active` (Dangerous now at index 4, was 3).
- `mew-tui/src/ui/status.rs` — `test_build_pills_autoplus_mode_prepends_deeper_purple_pill` (asserts the pill is distinct from Auto / Permissive / Dangerous colors).

### Verified

- `cargo test --all` — **498/498 pass** (was 492; +6 from Auto+).
- `cargo clippy --all -- -D warnings` — clean.
- `mew chat --help` shows `--auto-plus ... [env: MEW_AUTO_PLUS=]` alongside `-P`, `-A`, `-D`.

### Decisions worth noting

- **Auto+ is fail-closed (Deny), not fail-open.** The user explicitly chose this. A flaky classifier will stall the session, but never silently allows a destructive tool.
- **No retry.** Picked the simpler semantics over "retry once then deny." A retry adds latency and a partial mitigation at best; if the classifier is broken, retrying just gives it more chances to fail.
- **`from_id` accepts both `auto_plus` and `autoplus`.** Polls in advance for the underscore form (which matches the CLI and other enum id conventions) but tolerates the joined form for users who type `/permissions autoplus` without thinking.
- **Status pill is deeper purple, not a different hue.** Same color family as Auto signals "Auto with more commitment." A different hue (e.g. cyan or magenta) would be visually too distant — Auto+ is conceptually adjacent to Auto, just stricter.

### Files touched

- `mew-hooks/src/lib.rs` — `PermissionMode::AutoPlus` variant + `from_id`/`id`/`picker_label` for it (also accepts `autoplus`).
- `mew-config/src/permissions.rs` — `mode()` handles AutoPlus; cascade short-circuits both Auto and Auto+ to `Prompt`; 2 new tests.
- `mew-agent/src/tools.rs` — restructured classifier match to branch Escalate / None on mode (Auto → user modal, Auto+ → Deny).
- `mew-tui/src/app.rs` — picker has 5 items; slash command routes `auto_plus`; tests.
- `mew-tui/src/ui/status.rs` — deeper-purple "Auto+" pill; status pill test.
- `mew/src/main.rs` — `--auto-plus` / `MEW_AUTO_PLUS=1` flag on all three subcommands; `resolve_mode(permissive, auto, auto_plus, dangerous)`; alert text for Auto+ in all three dispatch sites.

## HookOutcome generalization (2026-06-21)

Generalized the two blocking hooks (`on_permission_ask`, `on_tool_execute_before`) to return a `HookOutcome<T>` enum instead of the bare transformed value. Plugins can now veto an action, not just transform it. Closes the last partial item from `POLYTOKEN_PARITY.md`'s hooks-runtime-parity row.

### `HookOutcome<T>` enum

`mew-hooks/src/lib.rs:170-220` — three variants:

```rust
pub enum HookOutcome<T> {
    Proceed(T),         // let the action run with this (possibly modified) value
    Block(String),      // don't run; the string is the reason shown/logged
    Suppress,           // don't run, don't log, don't surface
}
```

Helpers: `proceed(value)` constructor, `is_proceed()` / `is_blocked()` predicates, `map(f)` for value transformation.

**`Retry` is intentionally not included.** No concrete use case wired; adding it would expose API surface that isn't connected to anything. Add when there's a real retry path.

### Trait signature changes

```rust
// Before:
async fn on_permission_ask(&self, call, current: PermissionDecision) -> PermissionDecision;
async fn on_tool_execute_before(&self, call, input: Value) -> Value;

// After:
async fn on_permission_ask(&self, call, current: PermissionDecision) -> HookOutcome<PermissionDecision>;
async fn on_tool_execute_before(&self, call, input: Value) -> HookOutcome<Value>;
```

The transformation hooks (`on_chat_message`, `on_system_prompt`, `on_tool_execute_after`, `on_shell_env`, `on_user_input`, `on_chat_params`, `on_chat_headers`) stay returning their transformed value directly. Only the blocking hooks gain the outcome type — they're the ones where "don't do this" is a meaningful response.

### Agent call-site wiring

`mew-agent/src/tools.rs`:

- **`on_permission_ask`**: match on the outcome. `Proceed(d)` → use the decision as before. `Block(reason)` → force `Deny` with an `info!` log. `Suppress` → force `Deny` silently.
- **`on_tool_execute_before`**: match on the outcome. `Proceed(v)` → use the input. `Block(reason)` → skip the tool, emit a `ToolState::Error` with `"blocked by hook: {reason}"`, emit `ToolEnd(success=false)`, `continue` the loop. `Suppress` → same shape but at `debug!` log level with a generic `"tool call suppressed"` message (the model still needs to see a result).

### SubprocessDispatcher

`mew-hooks-runtime/src/runtime.rs:812-857` — both impls wrap their existing `pipe_json_filtered` result in `HookOutcome::Proceed(...)`. **TODO** (left as code comments): inspect plugin exit codes or a `block:` prefix on stdout to support `Block` / `Suppress` from subprocess plugins. Today subprocess plugins can only transform the input — they can't veto. Rust in-process plugins can use the full outcome API immediately.

### `NopDispatcher` defaults

Both blocking hooks default to `HookOutcome::Proceed(input)` — preserves the existing "no plugin, no change" behavior.

### Tests

- `mew-hooks/src/lib.rs` — 7 new tests for `HookOutcome`: `proceed`/`block`/`suppress` predicates, `map` semantics for all three variants, `proceed()` helper.
- `mew-hooks-runtime/tests/plugin_integration.rs` — updated assertions to wrap expected values in `HookOutcome::Proceed(...)`.
- `mew-agent/src/hooks_tests.rs` — both mock impls updated to return `HookOutcome::Proceed(...)`.

### Verified

- `cargo test --all` — all pass (was 498, now 505 with the 7 new HookOutcome tests).
- `cargo clippy --all -- -D warnings` — clean.
- `cargo build --all` — full workspace compiles.

### Still open (deferred)

- **Subprocess `Block` / `Suppress` protocol.** Rust plugins get the full API today; subprocess plugins can only transform. The exit-code mapping (0 → Proceed, 2 → Block with stderr as reason, 3 → Suppress) is documented as a TODO in `runtime.rs` but not wired.
- **`!name` negation for matchers.** Mew's `PluginHookConfig` is per-plugin, not per-(global-vs-project), so polytoken's "disable inherited global hook" concept doesn't directly map. Could add `!`-prefix support to matcher strings (`"!bash"` = "fire for everything except bash") if needed.
- **`Retry` variant.** Reserved in the parity doc but not implemented. No concrete retry path at the agent layer yet.

### Files touched

- `mew-hooks/src/lib.rs` — `HookOutcome<T>` enum + helpers + 7 tests; trait signatures for the two blocking hooks updated; `NopDispatcher` defaults updated.
- `mew-agent/src/tools.rs` — both call sites updated to match on `HookOutcome` and handle `Block` / `Suppress`.
- `mew-hooks-runtime/src/runtime.rs` — both `SubprocessDispatcher` impls wrap their result in `HookOutcome::Proceed(...)`.
- `mew-hooks-runtime/tests/plugin_integration.rs` — assertions updated.
- `mew-agent/src/hooks_tests.rs` — two mock impls updated.

## Polish items (classifier config, !name negation, subprocess Block/Suppress) (2026-06-22)

Three independent polish items, each closing a gap from earlier milestones.

### 1. Classifier config in config.toml — fixes a broken feature

Auto/Auto+ modes were shipped but `set_classifier_provider` was never called in `main.rs` — both modes silently fell through to the user modal on every call. Now wired via config:

```toml
[permissions]
classifier_provider = "opencode-go"
classifier_model = "deepseek-v4-flash"   # optional; uses provider default if unset
```

New `maybe_set_classifier_provider(agent, cfg, cat, raw)` helper (`mew/src/main.rs:582`) builds the classifier provider via the existing `build_provider(...)` and calls `agent.set_classifier_provider(provider, model)`. Called at all three startup sites (`build_and_run`, `run_tui`, `run_acp_server`). If the provider build fails, logs a warning and Auto falls through to the user modal — the safe default.

### 2. `!name` matcher negation in PluginHookConfig

`PluginHookConfig::matches()` now supports `!`-prefix negation:

- `"bash"` → fire only for bash (existing behavior)
- `"!bash"` → fire for everything except bash
- `"!bash|!write"` → fire for everything except bash and write
- `"bash|write|!rm"` → fire for bash or write, but never rm (mixed)
- `"!*"` → fire for nothing (exclude all)

Logic: negative entries (starting with `!`) are checked first — if subject matches any negative, return false. If all entries are negative, return true (default include). If positives exist, subject must match one. 4 new tests in `mew-hooks/src/lib.rs`.

### 3. Subprocess Block/Suppress protocol

`SubprocessDispatcher` can now return `HookOutcome::Block` and `HookOutcome::Suppress` from subprocess plugins. Protocol:

- Plugin responds with `"block"` or `"block: <reason>"` → `HookOutcome::Block(reason)`
- Plugin responds with `"suppress"` → `HookOutcome::Suppress`
- Anything else → parsed as the modified value → `HookOutcome::Proceed(value)`

New `pipe_json_raw(hook, initial, subject) -> Option<String>` method returns the raw last plugin response before parsing. `detect_outcome(raw) -> Option<HookOutcome<()>>` checks for Block/Suppress markers. Both blocking hooks (`on_permission_ask`, `on_tool_execute_before`) now use `pipe_json_raw` + `detect_outcome` instead of the old `pipe_json_filtered` wrapper. Backward compatible — existing plugins that return bare values still work (Proceed).

### Verified

- `cargo test --all` — all pass (no failures)
- `cargo clippy --all -- -D warnings` — clean
- `cargo build --all` — full workspace compiles

### Files touched

- `mew-config/src/lib.rs` — `PermissionsConfig` gains `classifier_provider` + `classifier_model` fields
- `mew-hooks/src/lib.rs` — `PluginHookConfig::matches()` supports `!`-prefix negation; 4 new tests
- `mew-hooks-runtime/src/runtime.rs` — `pipe_json_raw` + `detect_outcome` helpers; both blocking hooks rewritten to use them; TODOs removed
- `mew/src/main.rs` — `maybe_set_classifier_provider(...)` helper; called at all three startup sites
