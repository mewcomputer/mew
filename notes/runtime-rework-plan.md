# runtime rework: one loop, one dispatch, guarded

Supersedes step 4/6 of notes/tui-ux-fixes-plan.md and steps 1-2 of notes/testing-plan.md —
those described the minimal seam; this is the full redo of the section that produced the
bugs, plus the guards that keep it from regrowing. The other steps of both plans still
stand and slot into the stages below.

## why a redo and not a patch

crates/mew/src/main.rs is 4,718 lines. `run_tui` is one 1,340-line async fn; the drain
loop inside it is a second copy of its own command dispatch; `chat_with_daemon` (~250
lines) is a third; the TUI harness's `apply_action` is a fourth. Every copy was written by
handling the case at hand and stubbing the rest ("deferred to main loop" — with no
deferral), and nothing — not the compiler, not a test, not CI — could object. The review
found the same defect class at four sites because the architecture manufactures it.

The design goal, stated once: **make the lazy path and the correct path the same path.**
One place to add a command handler; a stubbed arm that silently drops input must not
compile; a message push that forgets cache invalidation must not compile. Agent diligence
is not a load-bearing part of the design.

## target structure

```
crates/mew/src/
  main.rs          entrypoint: parse CLI, route to a command fn (~150 lines)
  cli.rs           clap types (Cli, subcommand enums)
  commands/        one file per subcommand: chat.rs, run.rs, daemon.rs (run/stop/pair),
                   theme.rs, debug.rs, config.rs
  setup/           construction, no control flow:
    providers.rs   build_provider, resolve_model/provider, catalog, router lookup
    agent.rs       build_session_agent, build_tools, build_dispatcher, mcp connect,
                   permission engine, secret set
    personas.rs    apply_persona_switch, persona_summary
  runtime/
    mod.rs         run_event_loop<T: CommandTarget> — the single loop (recv, drain,
                   draw policy, coalescing caps)
    target.rs      trait CommandTarget + Unsupported
    local.rs       LocalTarget: owns the Agent + model-switch context
    daemon.rs      DaemonTarget: owns the DaemonClient
    dispatch.rs    handle_action + slash-result routing — the only dispatch
    mentions.rs    process_mentions, image_mime
  config_editor.rs (unchanged)
```

mew-tui changes ride along (stage 3): `App::messages` and the other cache-coupled fields
go private, mutation through methods that maintain `chat_dirty`.

## core decisions

### 1. one event loop

`run_event_loop<T: CommandTarget>` owns: terminal draw policy (idle-skip), `recv().await`,
the drain (tick coalescing, agent-event cap of 4 while streaming), and settings-editor
delegation. Mode differences do not live in the loop.

The daemon loop's extra channels fold into the `Event` enum instead of a `select!`:
`Event::DaemonNotify(ServerMessage)` and `Event::PluginUi(String, String)` (mew-tui
already depends on mew-protocol via `apply_daemon_notification`). Forwarder tasks push
into the one channel; the loop is a plain recv. This deletes the daemon/local drift class
outright: the daemon gets the drain, coalescing, and Settings handling it's currently
missing because there is no second loop to forget them in.

The drain does not interpret anything. It coalesces scrolls/ticks, caps agent events, and
pushes every produced `Action` into a `Vec<Action>` replayed through `handle_action` after
the drain exits. `pending_drain_submit` disappears as a special case.

### 2. one dispatch

```rust
// runtime/dispatch.rs
#![deny(clippy::wildcard_enum_match_arm)]

pub async fn handle_action<T: CommandTarget>(cx: &mut Ctx<'_, T>, action: Action) -> Flow
```

`Ctx` bundles `&mut App`, `&mut T`, the event-loop sender (for forwarding agent event
receivers), and `&mut Option<ConfigEditor>`. `Flow` is `Continue | Quit`. Slash handling
lives here too: `handle_slash` (pure, stays in mew-tui) → `SlashResult` → routed in the
same module.

The lint deny means no `_ =>` arm can be written over `Action` or `SlashResult` in this
module: adding a variant breaks the build until every consumer decides explicitly. That is
the compile-time version of the drain bug.

### 3. CommandTarget: capability honesty instead of silence

```rust
// runtime/target.rs
pub struct Unsupported(pub &'static str);   // human-readable reason

#[async_trait]
pub trait CommandTarget {
    fn prompt(&mut self, enriched: String, parts: Vec<Part>) -> Receiver<AgentEvent>;
    async fn cancel(&mut self);
    async fn clear(&mut self) -> Result<(), Unsupported>;
    async fn compact(&mut self) -> Result<(), Unsupported>;
    async fn todos(&mut self) -> Result<String, Unsupported>;
    async fn switch_model(&mut self, spec: &str) -> Result<SwitchedModel, Unsupported>;
    async fn set_permission_mode(&mut self, mode: PermissionMode) -> Result<(), Unsupported>;
    async fn set_thinking(&mut self, variant: &str) -> Result<(), Unsupported>;
    async fn attach_session(&mut self, id: &str) -> Result<(), Unsupported>;
    async fn resume(&mut self, id: &str) -> Result<(), Unsupported>;
    async fn rewind(&mut self, n: usize) -> Result<(), Unsupported>;
    async fn switch_persona(&mut self, name: &str) -> Result<PersonaApplied, Unsupported>;
    async fn plugin_command(&mut self, name: &str, args: &str) -> Result<String, Unsupported>;
    async fn cancel_subagent(&mut self, task_id: &str) -> Result<bool, Unsupported>;
    async fn yield_control(&mut self) -> Result<(), Unsupported>;
    // shutdown hooks: on_session_save / on_stop equivalents
}
```

Dispatch renders every `Err(Unsupported(reason))` as an alert through one code path. A
lazy target impl that returns `Err(Unsupported("not implemented"))` produces visible UX,
not a swallowed keypress — the failure mode is downgraded from "silent data loss" to
"honest error message." `handle_slash_result_local` and the daemon's ad-hoc intercept
list (`/clear`, `/web`, `/yield`, ...) both dissolve into `DaemonTarget`.

Today's daemon gaps become one-line decisions in `DaemonTarget`: `/quit` works (loop
concern, not target), `switch_model`/`set_thinking` return `Unsupported` until the
protocol grows them, `resume`/`attach` forward to the client.

### 4. App mutation goes through methods (mew-tui)

`messages`, `chat_dirty`, `rendered_chat`, `rendered_md_cache`, `tool_states` become
`pub(crate)` or private with accessors:

- `push_message(Message)`, `push_synthetic(text)`, `push_user(text, attachments)` — all
  mark dirty. The free functions `user_message`/`synthetic_message` in main.rs move here.
- read access via `messages(&self) -> &[Message]`.
- `handle_agent_event` stays the internal mutator and keeps its own dirty discipline
  (fixing the PartStart early-return miss while we're in there).

After this, the dirty-mark bug class is unwritable from outside mew-tui: the field isn't
reachable. This is the compiler doing the job the review did by hand.

## guards, ordered by strength

1. **compiler** — field privacy (App), exhaustive matches (`deny(wildcard_enum_match_arm)`
   in dispatch.rs), `Unsupported` forcing an alert path, one generic loop so a mode can't
   lack features by omission.
2. **tests** — from notes/testing-plan.md: dispatch regression tests for each found bug
   class; a table test iterating every `Action` variant (strum `EnumDiscriminants` +
   `EnumIter`) asserting dispatch produces an observable effect (state change, target
   call, or alert — never nothing); every builtin slash command routes to non-`Continue`;
   harness wired to the real dispatch; golden frames.
3. **CI ratchet** — a `just arch-check` recipe run inside `just ci`:
   - `Action::`/`SlashResult::` match arms outside `runtime/dispatch.rs` → fail
   - `messages.push` / `\.messages\b.*push` outside mew-tui's app.rs → fail
   - `todo!\(|unimplemented!\(` anywhere in crates/mew → fail
   Grep is crude but it fires exactly when a future agent starts a fifth dispatch copy,
   which is the moment we want the build red.
4. **CLAUDE.md** — a short "runtime invariants" section, written for the next agent (or
   the next lazy pass of us): never match `Action`/`SlashResult` outside
   `runtime/dispatch.rs`; the drain never interprets events; messages are pushed only
   through `App` methods; a new command means a `CommandTarget` method + dispatch arm +
   test in the same change; `Unsupported` is the only sanctioned way to not implement
   something. Docs are the weakest guard, so everything above must hold without them —
   but they route the agent to the right place before it improvises.

## stages (each lands green, reviewable diffs, no big-bang)

- **stage 0 — pin current behavior.** Land the regression tests from testing-plan step 1
  written against the bugs (expected-fail or asserting today's broken behavior with
  FIXME markers, whichever keeps CI green). These define "done" for stage 1. Also the
  golden-frame scaffold, so stages can't silently change rendering.
- **stage 1 — extract dispatch.** `runtime/dispatch.rs` + `CommandTarget` + `LocalTarget`;
  local loop and its drain both call it; the P0 fixes (action drops, dirty marks via new
  App methods' first callers) land here because the extraction *is* the fix. Stage-0
  tests flip to passing.
- **stage 2 — unify the loops.** `run_event_loop<T>`; `chat_with_daemon` shrinks to
  connection setup + `DaemonTarget` + the shared loop. Daemon gains drain/coalescing and
  Settings mode for free. Delete `handle_slash_result_local`.
- **stage 3 — encapsulate App.** Privatize the cache-coupled fields, migrate read sites
  to accessors. Mechanical, big-ish diff, zero behavior change — keep it its own commit.
- **stage 4 — install the guards.** Lint denies, `just arch-check`, CLAUDE.md invariants,
  harness switched to real dispatch, the every-variant table test.
- **stage 5 — split the rest of main.rs** (cli/commands/setup modules). Pure code motion,
  last, so it never blocks the behavioral stages.

Branch per stage off a shared WIP branch; `just ci` gates each. Stages 1-2 subsume
ux-fixes steps 2/4/6; ux steps 1/3/5/7 and testing steps 3-8 proceed independently.

## non-goals

- no new crate for the runtime (modules in `mew` suffice; extract later if a second
  binary ever wants the loop — yagni).
- no rewrite of App/event handling internals beyond the privacy pass — the streaming
  render fixes stay in the ux plan.
- no plugin/dispatcher (mew-hooks) changes; `PluginCommand` just becomes a
  `CommandTarget` method.

## open questions

- ~~protocol gaps surfaced by `DaemonTarget`~~ resolved by notes/silent-drop-audit.md:
  the protocol and daemon already implement `SwitchModel`, `SetThinkingVariant`,
  `SetPermissionMode`, and `SwitchPersona` — the TUI's "not available in daemon mode"
  alerts are stale stubs. `DaemonTarget` wires these up in stage 2; `Unsupported` is only
  for genuinely missing ops (`rewind`, plugin commands).
- strum as a dev-dependency for the every-variant test: fine? (tiny, derive-only,
  test-scoped). Alternative is a hand-maintained variant list with a compile-guard
  match — works, slightly more ceremony.
