# TUI command + streaming UX fixes

> update 2026-07-06: steps 2, 4, and 6 are subsumed by notes/runtime-rework-plan.md
> (full redo of the main.rs dispatch/loop section + recurrence guards). Steps 1, 3, 5,
> and 7 still land as written.

Review of the TUI (mew-tui + the two event loops in crates/mew/src/main.rs), scoped to the
reported problems: slash/palette commands misbehaving, and janky streaming. Severity labels
follow .mew/agents/code-reviewer.md (P0 must fix, P1 should fix, P2 nice to have).

## findings

### commands

**[P0] command feedback never triggers a chat rebuild.**
`app.messages.push(synthetic_message(...))` appears ~30 times in crates/mew/src/main.rs
(e.g. main.rs:3074, 3084, 3400) and `App::push_synthetic_message` (app.rs:2063) also skips
`mark_chat_dirty()`. `ensure_chat_rendered` (app.rs:2017) only rebuilds when `chat_dirty`
bumps, so the output of `/cost`, `/todo`, "switched to <model>", "context cleared", theme
confirmations, and provider-error text is invisible until some unrelated event dirties the
chat — often not until the next agent turn. The user's own submitted message has the same
problem (main.rs:3055, 2156, 3890): it only appears when the first provider event lands, so
on a slow model the app looks like it ignored Enter. This alone explains most of
"commands feel broken" and part of "streaming feels janky" (your prompt pops in together
with the first token burst).

**[P0] the drain loop silently drops command actions it claims to defer.**
The local event loop processes one event, then drains the queue (main.rs:3590). The drain's
`Action::SlashCommand` match (main.rs:3724-3808) and action match (3810-3826) stub out most
variants with comments like "Deferred to main loop; ignored during drain" — but there is no
deferral mechanism (only `Submit` gets one, via `pending_drain_submit`). Dropped outright:
`/model` (picker), `/permissions`, `/persona`, `/rewind`, `/resume`, `/sessions`, `/mouse`,
plugin commands, `SetThinkingVariant`, `SwitchModel` (model-picker Enter), `AttachSession`,
`SetPermissionMode` (permission-picker Enter). Because the 16ms tick keeps the channel
non-empty, a keypress that arrives while the loop is mid-iteration lands in the drain — so
these commands fail intermittently, and near-deterministically while streaming (agent events
flood the queue). The drain comment at main.rs:3810 ("palette is never open during heavy
streaming") is wrong: Ctrl+P works while streaming, so picking a model then does nothing and
the picker just closes.

**[P1] daemon-mode loop drops a different set of actions.**
`chat_with_daemon`'s match ends in `_ => {}` (main.rs:2288). Dropped there: `CopySelection`
(mouse-select copy does nothing), `InsertAtMention` / `InsertSubagentMention` (@-picker Enter
does nothing), `SetPermissionMode` (picker selection ignored), `ToggleSidebar*`,
`CancelMostRecentSubagent`, `PersonaSwitchConfirmed`, `OpenSettings`.

**[P1] `/quit` is a no-op in daemon mode.**
`handle_slash_result_local` lumps `Quit` into the do-nothing arm (main.rs:2365). Only
Ctrl+C/Ctrl+D exit.

**[P1] unknown slash input is silently swallowed.**
`SlashResult::Continue` is documented "fall through to the model" (app.rs:31) but the local
loop does `continue` (main.rs:3063) and the daemon loop does nothing (main.rs:2365). Typing
any message starting with `/` that isn't a known command deletes it: it's already popped from
the input by `submit_input`, lands in history, and never reaches the model or the transcript.
No error, no echo.

**[P1] root cause: three divergent copies of command dispatch.**
Main-loop match (main.rs:3061), drain-loop match (main.rs:3724), daemon-loop match
(main.rs:2177 + handle_slash_result_local). Every new SlashResult/Action variant must be
wired three times; the P0/P1s above are all drift between the copies.

**[P2] slash autocomplete scroll window is wrong.**
`adjust_slash_scroll` hardcodes `visible = 3` (app.rs:2458) but the rendered list shows up to
`min(area.height/2, 12)` items (ui/mod.rs:66, overlays.rs:28). Arrowing down starts shifting
the list after the 3rd item even though 12 are visible — the selection appears to jump.

**[P2] stale copy.** `/permissions` description says "(Standard or Dangerous!)"
(app.rs:2232) but five modes exist. `/help` opens the palette rather than showing the
shortcuts overlay `?` gives — pick one story.

### streaming

**[P1] the streaming message is fully re-rendered every frame.**
`build_chat_lines` calls `ratatui_mdstream::render_streaming` (chat.rs:426-434) which walks
*all* committed blocks and syntect-highlights every code block from scratch on each draw
(ratatui-mdstream/src/lib.rs:46-79). ratatui-mdstream already ships `RenderCache`
(src/cache.rs) that caches committed blocks and only renders the pending one — the TUI just
doesn't use it. Long answers with several code blocks get progressively laggier as they
stream; this is the classic "typing feels fine at the start, chugs by the end" jank.

**[P1] markdown cache is keyed by message id, but multi-part messages have several text parts.**
`rendered_md_cache` uses `msg.id` as key (chat.rs:440-469). An agentic turn (text → tools →
text) has 2+ `Part::Text` in one message; each part's lookup sees the other part's cached
text, misses, re-renders, and overwrites the entry. Those messages re-render (markdown parse
+ syntect) on *every* rebuild forever — and during streaming a rebuild happens per delta
batch. Key the cache by `PartId` instead.

**[P1] daemon-mode loop has no drain/coalescing.**
The local loop coalesces bursts and caps agent events at 4 per frame; `chat_with_daemon`
(main.rs:2118-2305) draws once per received event. Char-granularity deltas over the socket
mean one O(transcript) rebuild + draw per delta. Daemon TUI streaming is strictly jankier
than local for the same content.

**[P2] `PartStart` on an existing assistant message doesn't mark the chat dirty.**
app.rs:2499-2511 pushes the part and returns before `mark_chat_dirty()` (only the
new-message branch at app.rs:2528 marks). First token / tool box shows up one event late.

**[P2] cached lines are deep-cloned every rebuild.**
`Rc::unwrap_or_clone(Rc::clone(cached_lines))` (chat.rs:445) always clones: the cache still
holds a strong ref, so `unwrap_or_clone` never gets to unwrap. Every rebuild deep-clones the
rendered lines of the entire transcript. Either keep `Rc<[Line]>` end-to-end in `BuiltChat`
or accept the clone and drop the misleading Rc dance.

**[P2] scroll math can drift during streaming.**
Lines are pre-wrapped so each cache entry is one visual row, but the em-dash fixup during
streaming can push a line past chat width; the `Wrap` safety net (chat.rs:193-200) then makes
actual rows exceed `rc.lines.len()`, so `max_scroll` undercounts and the pinned bottom sits a
row or two high until finalize. Wrap after the em-dash fixup instead of relying on the net.

**[P2] markdown always renders with the dark theme.**
`MdTheme::dark()` is hardcoded in both the streaming and static paths (chat.rs:432, 450,
463). `/theme light` restyles the chrome but not message bodies.

## plan

Ordered so each step lands independently; 1-3 are the visible-pain fixes, 4 is the
structural one that prevents recurrence, 5+ are polish. TDD via the headless harness
(`mew_tui::harness`, exercised with `cargo run -p mew-tui --example tui_driver`) where the
behavior is observable; `just ci` gates each step.

### 1. make message pushes render (P0, small)

- add `App::push_message(msg)` that pushes and calls `mark_chat_dirty()`; make
  `push_synthetic_message` call it.
- replace every direct `app.messages.push(...)` in main.rs (both loops + drain +
  `drain_pending_persona_switch`) with the new method. Grep to zero.
- test: harness-level — push a synthetic message via the slash path, assert `chat_dirty`
  bumped and the rendered rows contain the text on the next draw; regression test that
  `Action::Submit` echoes the user message before any agent event arrives.

### 2. stop dropping actions in the drain (P0, medium)

- extract the local loop's action handling into one `async fn handle_action(ctx, action)`
  (ctx bundles `&mut app`, `&mut agent`, cfg/cat/provider bits, `&mut settings_editor`,
  `&mut should_break`). Main path calls it directly.
- the drain no longer interprets actions at all: it only handles scroll coalescing and
  agent-event batching; any `Action` produced during the drain is pushed to a
  `Vec<Action>` processed after the drain by the same `handle_action` (the existing
  `pending_drain_submit` special case folds into this).
- test: harness feeding a tick-then-key sequence (forcing the key into the drain) for each
  formerly-dropped action: `/permissions` picker Enter applies the mode, model-picker Enter
  switches, `/rewind` opens the picker, etc.

### 3. unknown `/` input falls through to the model (P1, small)

- `SlashResult::Continue` → submit the original text as a normal prompt (both loops),
  matching the enum doc. Alternative (if we'd rather not send typos to the model): echo it
  back with "unknown command" — but the doc comment says fall through, so do that unless
  you'd rather flip it.
- daemon mode: route `Quit` (`should_break = true`) and make `/todo` render the last
  `TodosUpdated` snapshot instead of nothing.
- tests: `handle_slash` unit tests already exist in app.rs — add loop-level ones for the
  fall-through and daemon `/quit`.

### 4. single command dispatcher (P1, medium — the recurrence guard)

- fold the daemon loop onto the same `handle_action` shape: one
  `enum CommandTarget { Local(...), Daemon(...) }` or a small trait with the agent-touching
  operations (`clear_context`, `force_compact`, `switch_model`, `cancel`, ...), implemented
  by the local agent and the daemon client. `handle_slash_result_local` disappears into it;
  daemon-unsupported ops keep their alerts in exactly one place.
- this is the step that deletes the three-way drift. Keep the diff reviewable: move code,
  don't rewrite semantics; the tests from steps 1-3 must pass unchanged.

### 5. incremental streaming render (P1, medium)

- hold a `ratatui_mdstream::cache::RenderCache` on `App` for the active streaming text part;
  on each rebuild call `cache.extend(state.committed(), ...)` and render only the pending
  block, then `collect_lines()`. Reset the cache in the same places `md_stream` is reset
  (PartStart(Text), MessageEnd).
- key `rendered_md_cache` by `PartId` (evict by message id on clear/rewind — keep a
  `MessageId → Vec<PartId>` walk or just retain on live part ids).
- move `mark_chat_dirty()` above the early return in the PartStart existing-message branch.
- test: unit test in ratatui-mdstream asserting `RenderCache.extend` renders each committed
  block once across repeated calls; TUI-side test that a message with two text parts
  cache-hits on the second rebuild (count `render_markdown` invocations via the cache map's
  stability, or assert the cache holds two entries with distinct part ids).

### 6. daemon loop coalescing (P1, small-medium)

- port the local loop's drain skeleton (tick coalescing + 4-agent-event cap) to
  `chat_with_daemon`. After step 4 the action handling is shared, so this is mostly
  copying the drain scaffold.

### 7. polish (P2, small each)

- `adjust_slash_scroll`: pass the actual visible count (the `min(area.height/2, 12)`
  computation) instead of hardcoded 3; store it on `App` at draw time like other layout
  facts (`chat_area` precedent).
- fix `/permissions` description; decide `/help` (palette vs `?` overlay) and align.
- `Rc` clone in the md cache: keep `Rc<Vec<Line>>` in `BuiltChat` lines or drop the Rc.
- wrap after em-dash fixup in the streaming path so the safety-net `Wrap` never fires.
- thread the active theme into `MdTheme` selection (needs a light variant in
  ratatui-mdstream or a `Theme → MdTheme` mapping).

### verification

- `just ci` after each step.
- manual smoke via the harness driver: stream a long multi-code-block answer and check
  frame cost stays flat (step 5), fire `/cost` mid-stream and confirm the report renders
  immediately (steps 1-2), daemon session over `mew chat --connect` for step 6.

### open questions

- step 3 alternative (fall through vs "unknown command" error) — plan assumes fall through
  per the enum doc.
- while streaming, double-Ctrl+C quits the whole app while double-Esc cancels the turn.
  Shell muscle memory says Ctrl+C should cancel first. Not counted as a defect (the hint is
  shown), but worth deciding while we're in this code.
