# testing plan: seams, fakes, and golden frames

> update 2026-07-06: steps 1-2 are subsumed by notes/runtime-rework-plan.md (the
> dispatch seam grew into a full runtime rework with compile-time guards). Steps 3-8
> still land as written; step-1 regression tests become that plan's stage 0.

Prompted by "we don't mock as much as we could be." Position after surveying the repo:
more mocking is the wrong lever — the project rules ("mocks hide problems", "never test
mocked behavior") are right, and the existing infrastructure already follows them. The
real gap is that the code where bugs actually live (the main.rs event loops and command
dispatch) has no seams, so it can't be tested with *anything* — mock or fake. This plan
adds seams and real-integration coverage, not mock objects.

## what exists today (and is worth keeping as-is)

| layer | mechanism | state |
|---|---|---|
| provider (LLM) | `FakeProvider` scripted fake + `StatefulFakeProvider` | good; ~50 mew-agent tests drive real turn loops |
| provider adapters | recorded SSE fixtures (`src/testdata/*.sse`, `just record` / `MEW_RECORD=1`) | good pattern, thin coverage (2 fixtures each) |
| daemon | real `DaemonServer` + real WS over unix socket, fake provider only | good; e2e (16) + concurrency (6) + tcp (5) + iroh (2) |
| tools | integration tests against real fs (tempdir) | good |
| TUI | headless `Harness` (TestBackend + script runner) + app.rs unit tests (~80) | good bones, but see gap 2 |
| md renderer | unit tests in ratatui-mdstream (inline/table/wrap) | fine |
| main.rs | — | **nothing** (~4600 lines, both event loops, dispatch, drain) |

## the gaps

1. **main.rs is untestable, not under-mocked.** Command dispatch exists in three
   divergent inline copies (main loop, drain loop, daemon loop). All of the P0s from
   notes/tui-ux-fixes-plan.md live here. No test can reach this code today.
2. **the harness simulates dispatch instead of exercising it.** `Harness::apply_action`
   (mew-tui/src/harness.rs:74) handles `Submit`/`Quit`/`Clear` and drops everything else —
   a fourth divergent copy of dispatch. Harness tests verify App + rendering, but the
   keypress→action→effect path is faked, so action-drop bugs are structurally invisible.
3. **no rendering regression net.** Streaming/layout jank can only be caught by eyeball.
   `run_script` already renders deterministic plain-text frames; nothing pins them.
4. **fake provider only exercises happy paths.** Fixed 4-char chunking, no reasoning
   parts in scripts, no retry/error sequences, no adversarial chunk boundaries
   (mid-UTF-8, mid-markdown-token, mid-code-fence).
5. **time-dependent behavior is untested.** Undo coalescing (500ms window), esc-cancel
   (2s) / ctrl-c (1s) windows, toast TTLs, marquee ticks — all call `Instant::now()`
   inline.
6. **no whole-binary test.** Terminal raw-mode setup/teardown, panic hooks, CLI parsing,
   the daemon handshake from a real client — none exercised end to end.

## plan

### 1. dispatch seam (pairs with tui-ux-fixes-plan step 4 — do together)

Extract command handling from main.rs into one testable unit:

- `trait AgentOps` (or a small enum-dispatch struct) covering the operations dispatch
  needs: `clear_context`, `force_compact`, `switch_model`, `cancel`, `todos`,
  `set_permission_mode`, `attach_session`, … Implemented by the local `Agent` wiring and
  by `DaemonClient`.
- `async fn handle_action(ctx: &mut DispatchCtx, action: Action)` — the single copy.
  Main loop, drain (via queued `Vec<Action>`), and daemon loop all call it.
- Tests: FakeProvider-backed agent + `handle_action` directly. Regression tests for each
  bug class found in the review: action produced during a drain is not dropped; every
  message-pushing arm leaves `chat_dirty` bumped; `/quit` quits under the daemon impl;
  unknown `/x` falls through to the model.

This is the highest-value step; everything in gap 1 becomes reachable.

### 2. harness drives real dispatch

Replace `Harness::apply_action`'s stub with the real `handle_action` over a test
`AgentOps` impl backed by FakeProvider (or a recording impl that logs calls for
assertion — a *fake with a log*, not a mock with expectations). Then a harness script
like `type /permissions⏎ key down key enter` asserts the mode actually changed.

### 3. golden-frame snapshots

- `crates/mew-tui/tests/golden/` holding `*.script` + `*.frame` pairs; a test walks the
  directory, runs `run_script`, diffs against the checked-in frame.
- `MEW_UPDATE_GOLDEN=1 cargo test -p mew-tui` regenerates.
- Recommendation: hand-rolled rather than `insta` — the renderer already emits trimmed
  plain text, the diff is a plain diff, zero new deps. Revisit if we want redactions.
- Seed set: welcome screen, user+assistant turn, streaming mid-turn (say with snapshot
  between deltas — needs a `say-partial` verb), tool-call block collapsed/expanded,
  reasoning block, slash autocomplete open, permission modal, narrow (40-col) layout.

### 4. adversarial FakeProvider scripts

Extend the builders (keep the type dumb — scripts stay `Vec<ProviderEvent>`):

- `text_response_chunked(text, sizes: &[usize])` for hostile boundaries: mid-UTF-8
  grapheme, mid-`**bold**`, mid-code-fence, em-dash at wrap column.
- `reasoning_then_text`, `parallel_tool_calls`, `retry_then_text` (RetryWait events),
  `error_mid_stream`.
- Optional timed variant (`sleep` between events — the import is already there) for the
  drain-coalescing tests once dispatch is extracted.

These directly verify the streaming fixes (RenderCache adoption, part-keyed md cache,
scroll math) instead of only the happy path.

### 5. more recorded provider fixtures

Grow `src/testdata/` via the existing `just record` flow: reasoning deltas (anthropic
content-block thinking), parallel tool calls, a provider error frame, a usage/cost frame.
One test per fixture asserting the decoded `ProviderEvent` sequence. This is where "test
against real APIs" lives — recorded once, replayed forever, re-recordable when providers
drift.

### 6. one real-binary e2e smoke

The daemon already supports `--fake-provider` (main.rs:188). Add one test (probably in
crates/mew/tests/) that:

- spawns the built `mew daemon --fake-provider` on a temp socket,
- runs `mew chat --connect <socket>` under a PTY (`portable-pty`),
- types a prompt, asserts the fake response appears, sends `/quit`, asserts clean exit
  and terminal restore.

Slow test — mark `#[ignore]` and run it in CI + `just ci`, not on every local `cargo
test`. This is the only place raw-mode/teardown/CLI wiring gets covered; one test is
enough (yagni on more).

### 7. time seam (smallest possible)

Pass `now: Instant` as a parameter to `push_undo`, the esc/ctrl-c pending checks, and
toast expiry (callers pass `Instant::now()`; tests pass fabricated instants). No clock
trait, no global — a parameter is the whole seam. Tests: undo coalescing inside/outside
the 500ms window, esc-cancel expiry, toast TTL ordering.

### 8. coverage audit (one-off, not a gate)

`cargo llvm-cov --all --html` once after steps 1-2 land, to find remaining blind spots
and direct any further effort. Explicitly not a CI threshold — coverage gates breed
assertion-free tests.

## explicitly not doing

- **mockall / expectation-style mocks** — they verify call sequences we invented, and
  the project rules forbid tests that only check mocked behavior.
- **mocking the filesystem** — tempdir is real and fast; keep it.
- **mocking the daemon protocol** — the e2e tests' real-socket approach is strictly
  better and already exists.
- **a coverage percentage gate in CI.**

## sequencing

Steps 1-2 ride along with the tui-ux-fixes work (same refactor). 3 and 4 next — they
protect the streaming fixes as they land. 5-7 are independent and small; 6 last since
it depends on nothing here. 8 after 1-2 to measure what's left.
