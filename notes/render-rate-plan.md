# turn timer + frame meter + native-refresh-rate rendering

goal: a millisecond turn timer in the status bar, a frame-time readout that doubles as a
render-loop stall detector, and a tick rate that matches the display's refresh rate
(120Hz on promotion) instead of the hardcoded 60fps. plus synchronized-output brackets
so high-rate frames don't tear.

written 2026-07-12. none of this is started.

## how the pipeline works today (the constraints)

- `EventLoop::spawn` (crates/mew-tui/src/events.rs:67) emits `Event::Tick` every
  hardcoded 16ms. this is the only thing that makes mew a 60fps app.
- both main loops (daemon: crates/mew/src/commands/tui.rs:94, local: tui.rs:606) gate
  draws with `if !last_event_was_tick || app.needs_redraw()`. idle = zero draws. during
  a turn `needs_redraw()` returns true because `app.streaming` is set
  (crates/mew-tui/src/app/mod.rs:755), so turns already draw at full tick rate.
- ratatui is immediate mode with a diffed back buffer: every draw recomposes the frame,
  but only changed cells are written to the terminal. the expensive part (chat lines)
  is cached behind `chat_dirty` / `ensure_chat_rendered` (app/mod.rs:1021); frames that
  don't bump it are O(visible).
- draw and event handling share one tokio task. there is no render thread. a wall-clock
  timer sampled at draw time therefore freezes exactly when the loop stalls — that's
  the diagnostic property we want.
- `app.tick()` (app/mod.rs:1208) advances TTLs, the marquee (time-based, 300ms), and
  the spinner (tick-count-based via `SPINNER_TICK_DIVISOR` — this one breaks if the
  tick rate changes; see phase 3).

## hard rules

1. **the timer never lives inside the chat scrollview content.** anything rendered by
   `build_chat_lines` would need `mark_chat_dirty()` per frame → full markdown re-render
   + rewrap + `chat_rows` clone at tick rate. status bar / overlay only.
2. **the frame meter never forces `needs_redraw()` true.** it displays stats *of draws
   that happen anyway*. if it demanded redraws to stay fresh, idle would draw forever.
   stale-while-idle is correct behavior.
3. **the turn timer only animates while a turn is active** (streaming or a tool
   running), which `needs_redraw()` already covers. no new redraw conditions needed for
   phase 1 at all.

## phase 1 — turn timer (ms) in the status bar

state, in `App` (crates/mew-tui/src/app/mod.rs):

- `turn_started_at: Option<Instant>` — set on the streaming false→true transition,
  which happens in `runtime/dispatch.rs:155` and `:175` (local submit) and via agent
  events in daemon mode. cleanest shape: an `App::begin_turn()` helper that sets both
  `streaming = true` and the timestamp, called from dispatch; daemon mode flips
  streaming inside `handle_agent_event`, so set it there on the same transition.
- `last_turn_duration: Option<Duration>` — frozen from `turn_started_at` at every
  `streaming = false` site (app/mod.rs:1427 finish, :1452 error, :1583 cancel — grep
  for `streaming = false` when implementing; a `finish_turn()` helper mirrors
  `begin_turn()`).

display, in `draw_status` (crates/mew-tui/src/ui/status.rs:274):

- extend the right-pinned segment (tokens · cost, status.rs:286) with the timer:
  `12.348s` while running, then the frozen `last_turn_duration` dimmed after finish so
  you can read the final number. format: seconds with 3 decimals under a minute,
  `1m 02.348s` above.
- reasoning already has an elapsed-time precedent (`reasoning_started_at`,
  app/mod.rs:95) — same pattern, coarser clock.

granularity note: the display updates once per tick (16ms today, ~8ms after phase 3/4).
that's the point — the ms digits jumping in tick-sized steps *is* the render-health
signal. no extra redraw machinery.

tests: timer formatting; `begin_turn`/`finish_turn` transitions (start sets, finish
freezes, cancel freezes); harness render (crates/mew-tui/src/harness.rs) asserting the
status line shows a duration while streaming and a dimmed one after.

## phase 2 — frame-time meter

state, in `App`:

- `frame_stats: FrameStats { last_drawn_at: Option<Instant>, last_delta: Duration,
  worst_delta: Duration }`. updated by a `note_frame()` method the main loops call
  right after a successful `terminal.draw` (both loops: tui.rs:95 and tui.rs:607).
  inter-frame delta is the honest number (it includes event handling + compose +
  write); no need to time the draw closure separately.
- `worst_delta` resets at `begin_turn()` so it reads "worst stall this turn", not
  "worst since launch" (a resize will otherwise pin it forever).
- deltas measured across an idle gap are garbage (idle skips draws by design). only
  update stats when the previous frame was also "active" — simplest: `note_frame()`
  ignores deltas above ~1s, or only records while `turn_started_at.is_some()`.

display: appended to the same right-pinned status segment, e.g. `8ms/210ms` (last /
worst-this-turn). gated behind a toggle so the default status bar stays clean:

- config: `tui.frame_meter = false` in `TuiConfig` (crates/mew-config/src/lib.rs:48,
  currently just `theme`).
- optional `/fps` slash command to flip it at runtime. **invariant 4 applies**: new
  command = `CommandTarget` method + dispatch arm + test in the same change
  (runtime/target.rs, runtime/dispatch.rs). if that's too much ceremony for a debug
  toggle, ship config-only first and add `/fps` when it earns it.

tests: `note_frame` delta/worst accounting; the idle-gap guard; config default off;
harness render with the flag on.

## phase 3 — configurable tick rate

- config: `tui.refresh_rate` in `TuiConfig`: `"auto"` | integer hz. default `60`
  (today's behavior, exactly). clamp to 30..=240. serde untagged enum or a string
  parsed at load — match however theme handles config today (keep it boring).
- `EventLoop::spawn` (events.rs:37) takes the interval:
  `spawn(tick: Duration)` — only two call sites (tui.rs:76 daemon, tui.rs:585 local)
  plus the harness. compute `Duration::from_nanos(1_000_000_000 / hz)`.
- set `MissedTickBehavior::Skip` on the interval while in there. the default (Burst)
  replays missed ticks after a stall, which at 8ms turns every stall into a tick
  flood. the drain loop coalesces them anyway, but skip is the correct semantic and
  it's one line.
- **spinner fix**: `SPINNER_TICK_DIVISOR` (app/mod.rs:1232) counts ticks, so 120Hz
  doubles the spinner speed. convert to elapsed-time like the marquee already is
  (300ms at app/mod.rs:1225): `spinner_advanced_at: Instant` + a fixed
  `Duration` per frame. audit `tick()` for anything else tick-count-based while there
  (nothing else found in current code — TTLs and marquee are already time-based).

tests: interval math from config values incl. clamping; spinner advances at the same
wall-clock rate under a 16ms-tick and an 8ms-tick simulation (harness drives ticks
manually, so simulate by calling `tick()` at different counts with mocked elapsed —
the time-based rewrite makes this trivial).

## phase 4 — refresh-rate autodetect (`"auto"`)

- macOS: `CGDisplayCopyDisplayMode(CGMainDisplayID())` →
  `CGDisplayModeGetRefreshRate` via the `core-graphics` crate. runs once at startup.
  returns 0.0 on some external displays — fall back to 60.
- known ambiguity: this reads the *main* display; we can't know which display the
  terminal window is on without appkit spelunking that isn't worth it. document the
  limitation in the config comment and move on. the user can pin an integer.
- non-macOS: `"auto"` = 60 with a debug-level log. no wayland/x11 heroics.
- dependency check before committing to `core-graphics`: it pulls core-foundation,
  which may already be in the tree via keyring/etc — check `just deps` first. if it's
  not already transitively present and the tree cost is ugly, the fallback
  implementation is parsing `system_profiler SPDisplaysDataType -json` once at startup
  in a `tokio::spawn` (it's slow, ~hundreds of ms — must not block first paint; start
  at 60 and retune the interval when the answer arrives, which means the tick task
  needs a way to receive a new interval… only build that if we're forced off
  core-graphics).

tests: parse/fallback unit tests. the CG call itself gets a smoke test behind
`#[cfg(target_os = "macos")]`.

## phase 5 — synchronized output (the actual anti-tearing bit)

- without vsync, a frame write can land mid-composite and tear. DEC mode 2026 makes
  the emulator apply a frame atomically. crossterm 0.28 ships it as
  `terminal::BeginSynchronizedUpdate` / `EndSynchronizedUpdate` commands.
- first check whether ratatui 0.30's crossterm backend already emits 2026 (it grew
  synchronized-output awareness somewhere around 0.29/0.30 — verify against the
  actual dependency before hand-rolling). if it does, this phase is a config no-op.
- if not: `execute!(BeginSynchronizedUpdate)` before / `EndSynchronizedUpdate` after
  each `terminal.draw` at all draw sites: the two main loops (tui.rs:95, tui.rs:607)
  and the settings editor loop (crates/mew/src/config_editor.rs:306). wrap it in a
  small helper so the bracket can't be forgotten at one site. must be
  panic-safe-ish: if `draw` errors, still emit the End (a guard struct, not two bare
  `execute!` calls).
- unsupported emulators ignore the sequences (private mode set/reset passthrough), so
  no capability detection needed.

## phase 6 — cleanup

docs (`docs/`), config example for `tui.refresh_rate` / `tui.frame_meter`, CURRENT.md
entry, and a line in CLAUDE.md's config section if the shape of `TuiConfig` docs live
there. last, after everything above is verified.

## sequencing + effort

1 → 2 are independent of 3 → 4 → 5; land as two stacks. rough sizes: phase 1 small,
phase 2 small, phase 3 small-medium (spinner rewrite is the real content), phase 4
medium (dependency decision), phase 5 small (possibly zero). each phase is its own
commit(s) on a WIP branch.

## what we deliberately don't do

- no timer inside chat content (rule 1).
- no always-on idle animation — idle stays at zero draws.
- no per-window display detection, no linux refresh detection.
- no attempt to "lock" to vsync: the pty has no vblank. we match the frequency and
  free-run; whether frames past 60 are ever composited is the emulator's business
  (kitty/ghostty/alacritty: yes; iTerm2: capped ~60 by default; terminal.app: lol).
