# 2026-07-17 — Fix dev CEF Mach-port rendezvous by anchoring a main bundle

## Summary

The Tauri dev app spawned CEF helpers that died in a loop with
`bootstrap_look_up org.cef.framework.MachPortRendezvousServer.<pid>: Unknown
service name` followed by `Network service crashed or was terminated,
restarting service`. Root cause: Chromium names the browser's Mach rendezvous
server `<main bundle id>.MachPortRendezvousServer.<pid>` and helpers derive
the same name from their own main bundle. The unbundled dev executable has no
bundle identifier (the server registered as `.MachPortRendezvousServer.<pid>`,
verified with `bootstrap_look_up` probes), while helpers resolve the CEF
framework identifier `org.cef.framework`, so every lookup missed and each
helper terminated on startup.

Fix: point CEF at a real bundle for dev. `scripts/prepare-cef.mjs` now writes
a synthetic `src-tauri/target/debug/mew.app` (Info.plist with the
`ai.mew.mew` identifier) next to the dev executable, and the CEF host sets
`main_bundle_path` to it plus an explicit `framework_dir_path`. CEF appends
both as command-line switches and propagates them to every helper, so browser
and helpers agree on one rendezvous name. Verified in the dev app: browser
registers `ai.mew.mew.MachPortRendezvousServer.<pid>`, helpers stay alive,
CDP answers on 9223, zero rendezvous errors.

## Changes

- `native/cef-host/src/embed.rs`: set `Settings.main_bundle_path` (new
  `main_bundle_path()` resolver: `MEW_CEF_MAIN_BUNDLE_PATH` env override,
  nothing when running packaged, else exe-adjacent `mew.app`) and
  `Settings.framework_dir_path` (parent dir of the resolved framework).
- `mew-web-ui/scripts/prepare-cef.mjs`: write the synthetic development
  bundle `src-tauri/target/debug/mew.app/Contents/Info.plist` on every run.
- `mew-web-ui/src-tauri/src/lib.rs`: also print the CEF fallback reason to
  stderr; the desktop binary installs no tracing subscriber, so
  `tracing::warn` made CEF init failures invisible.
- `native/cef-host/README.md`: document the rendezvous/bundle relationship
  and the new `MEW_CEF_MAIN_BUNDLE_PATH` override; correct the dev prepare
  description (copies by default, `--link` symlinks).

## Notes

- Packaged (release) runs are unchanged: the resolver returns nothing when
  the executable lives inside a real `.app`, so CEF derives the bundle from
  the executable location as before.
- `cargo clippy --all-targets -- -D warnings` previously failed on five
  pre-existing lints in `native/cef-host`; these are now fixed (collapsed
  nested `if let` into let-chains, moved a `pub use` ahead of the test
  module), and the crate and `src-tauri` are both clippy-clean.
- Helpers no longer strictly need the `binaries/Resources` copy for ICU/pak
  files since `framework_dir_path` is propagated, but the copy is kept as a
  fallback.
- Force-killing the dev app leaves CEF's `SingletonLock`/`SingletonSocket`
  in the cache profile, which can make the next launch silently skip CEF
  initialization (WKWebView fallback). The new stderr message surfaces that
  failure; clearing `~/Library/Application Support/ai.mew.mew/cef-desktop-cache`
  recovers.
- Production sandbox/helper `.app` layout under `Contents/Frameworks` remains
  separate hardening, as before.

# 2026-07-17 — Add Kimi (Moonshot AI) provider

## Summary

Added Kimi as a built-in provider using the Anthropic-compatible endpoint.
Kimi serves three models: `k3` (Kimi K3, thinking-capable), `kimi-for-coding`
(Kimi K2.7 Code), and `kimi-for-coding-highspeed` (Kimi K2.7 Code HighSpeed).
K3 supports low/high/max thinking effort via top-level `reasoning_effort`.

## Changes

- `crates/mew-config/src/lib.rs`: Added `kimi` to `Config::default()` with
  `shape = "anthropic"`, `base_url = "https://api.kimi.com/coding/v1"`,
  `credential_ref = "kimi"`. Added `test_default_kimi_provider` and updated
  `test_load_default_when_missing`.
- `crates/mew/src/setup/providers.rs`: Added `"kimi"` to the anthropic arm of
  `provider_name_to_shape()`. Added kimi/k3 to the `discover_models()`
  fallback list gated on credential presence. Updated
  `provider_name_to_shape_known` test.
- `crates/mew-catalog/src/lib.rs`: Added k3 thinking variants (low/high/max
  via `reasoning_effort`) in `builtin_thinking_variants()` before the `kimi`
  catch-all that returns empty.

## Notes

- The `mew` binary does not compile due to a pre-existing error in
  `crates/mew/src/commands/tui_capture.rs:1142` — a non-exhaustive match on
  `ServerMessage` missing `BrowserSnapshot`/`BrowserScreenshot`/`BrowserState`
  variants from in-progress browser work. Not caused by these changes.
- `mew-config` and `mew-catalog` compile and pass all tests (37/37 catalog,
  2/2 config kimi tests).

# 2026-07-14 — Rework mew theming to a flat, aliased token table

## Summary

Replaced the hardcoded `ThemeTokens` struct with a flat, aliased token table
loaded from a single shared manifest. Built a `theme_codegen` binary that emits
the web UI CSS, Rust default/overrides, and a syntect `.tmTheme` from the same
manifest. Migrated TUI color usage to `Theme::resolve`, updated
`ratatui-mdstream` to consume the shared theme and generated syntect theme, and
migrated all 21 web UI themes into the manifest.

## Changes

- `crates/mew-tui/resources/theme_manifest.json`: new single source of truth
  with base tokens, aliases, and 21 selectable themes as sparse overrides.
- `crates/mew-tui/src/bin/theme_codegen.rs`: codegen binary producing
  `mew-web-ui/src/generated-themes.css`,
  `crates/mew-tui/src/theme_generated.rs`, and
  `crates/ratatui-mdstream/resources/theme.tmTheme`.
- `crates/mew-tui/src/theme.rs`: `Theme` backed by `HashMap<String, Color>`,
  `resolve`, `ansi`, `with_persona_accent`, `css_variables`, and manifest
  validation for custom theme files.
- `crates/mew-tui/src/ui/*.rs`, `settings.rs`: migrated `Color::` usage to token
  lookups.
- `crates/ratatui-mdstream/src/*.rs`: consumes the shared token table and the
  generated `.tmTheme` instead of hardcoded colors / `base16-ocean.dark`.
- `mew-web-ui/src/index.css`: imports `generated-themes.css` before the
  `@theme inline` mapping; hand-written theme blocks removed.
- `crates/mew/src/cli.rs` + `commands/theme.rs`: added `mew theme export-css`
  command.
- `crates/mew/tests/theme_install.rs`: integration tests for validation and
  install behavior.
- `docs/THEMING.md`: new design and vocabulary reference.
- `justfile`: `theme-codegen` and `theme-codegen-check` recipes wired into
  `just ci`.

## UI polish follow-up

- `crates/mew-tui/src/ui/input.rs`: input bar now uses `muted` token for its
  background instead of `status_bar.background`.
- `crates/mew-tui/src/ui/mod.rs`: removed the 1-cell divider line between chat
  and input; separation now comes from the muted input background.
- `crates/mew-tui/src/ui/status.rs`: removed the now-unused `draw_divider`
  function.
- Golden frames in `crates/mew-tui/tests/golden/` updated to reflect the removed
  divider line.
- `crates/mew-tui/src/app/mod.rs`: status-bar marquee tick interval halved
  from 300ms to 150ms, so overflow pill text scrolls roughly twice as fast.

## Verification

- `cargo clippy --all -- -D warnings` — clean
- `cargo test -p mew-tui` — passes after golden update
- Visual inspection via `mew tui-capture`: input bar shows muted background and
  no divider line.
- `just theme-codegen-check` — generated files up-to-date
- `pnpm build` in `mew-web-ui` — succeeds

---

# 2026-07-14 — Fix slow rasterization by caching the font system

## Summary

Root-caused the slow daemon capture: `mew_raster::rasterize` was creating a
fresh `cosmic_text::FontSystem` for every single frame. Font system creation is
expensive, so captures were taking ~1s per frame and producing very short
videos. Added a reusable `Rasterizer` that caches the font system and updated all
call sites to use it. Also switched shaping from `Advanced` to `Basic` and split
the capture timing log into draw vs rasterize time.

## Changes

- `crates/mew-raster/src/lib.rs`:
  - New `Rasterizer` struct that owns the `FontSystem` and `SwashCache`.
  - `Rasterizer::new()` builds the font system once with the bundled
    IoskeleyMono fonts.
  - `Rasterizer::rasterize()` and `Rasterizer::to_png()` are the cached
    equivalents of the old free functions.
  - The old free `rasterize()` and `to_png()` functions remain as convenience
    wrappers that create a one-off `Rasterizer` for callers that only need a
    single frame.
  - Switched `Shaping::Advanced` to `Shaping::Basic` for faster monospace text
    rendering.

- `crates/mew/src/commands/tui_capture.rs`:
  - `DaemonBackend` now owns a `Rasterizer` and uses it for both frame capture
    and screenshot PNG encoding.
  - Per-frame log now reports `draw_ms` and `rasterize_ms` separately.

- `crates/mew-tui/src/harness.rs`:
  - `LocalBackend` now owns a `Rasterizer` and uses it for frame capture and
    screenshots.

## CI Gate

- `cargo build` — clean
- `cargo test -p mew-raster` — 10 tests + 1 doc-test pass
- `cargo test -p mew-tui harness` — 15 tests pass
- `cargo test -p mew tui_capture` — 5 tests pass
- `cargo clippy -p mew-raster -- -D warnings` — clean
- `cargo clippy -p mew-tui -p mew -- -D warnings` — blocked by an unrelated
  `theme.rs` clippy warning (`manual_strip`) that is not part of these changes

---

# 2026-07-13 — Real-time streaming, flushing, tracing, and fast typing in daemon `tui-capture`

## Summary

Made daemon-connected `mew tui-capture --connect` record thinking and streaming
output progressively, keep pace with the real stream, print output incrementally
instead of buffering it all until exit, added tracing logs at the major flow
points, and fixed the long-prompt typing stall by stopping per-keystroke frame
rasterization.

## Changes

- `crates/mew/src/commands/tui_capture.rs`:
  - `DaemonBackend::send_text` no longer rasterizes a frame after every
    character; it types the whole prompt, then captures one frame. This fixes
    the "really long time" at `send_text: typing prompt` for long prompts.
  - `DaemonBackend::type_str` uses the same fast path (one frame after the full
    string).
  - `DaemonBackend::wait_turn` applies a streaming drain limit of 4
    `AgentEvent`s per frame, the same limit used by the live TUI drain loop.
  - `wait_turn` no longer sleeps after every batch when more events are already
    available, so it keeps pace with the actual stream instead of artificially
    slowing down the capture.
  - Drawing/capturing only happens when a 60 fps frame is due; the sleep is now
    reserved for idle waits when no events are ready.
  - `send_text` skips its 5 ms sleep once `app.streaming` is true.
  - `run_script_daemon` now prints each verb's output immediately and flushes
    stdout, rather than accumulating the entire script's output and printing it
    all at the end.
  - Added `tracing` logs: command start/finish, daemon connect/session ready,
    each script verb, `send`/`wait_turn` lifecycle, frame capture counts,
    per-frame capture timing, typing timing, and failure paths.
  - Added `agent_event_name` and `server_message_type` helpers so trace logs can
    record which event variant arrived without exposing channel payloads.

- `crates/mew-tui/src/harness.rs`:
  - `LocalBackend::type_str` also stops per-character captures and captures one
    frame after the full string, keeping harness mode consistent.

## Notes

- A reported symptom (UI shows "thought for 41.2s" but video is only 10s)
  pointed to slow frame rasterization. This was root-caused and fixed in the
  following entry by caching the `mew_raster` font system.

---

# 2026-07-13 — Daemon-connected `mew tui-capture --connect`

## Summary

Added a real-daemon capture path to `mew tui-capture`. Scripts can now drive
a live mew daemon (e.g. `mew daemon --fake-provider`) headlessly, capturing
true chat/turn behavior — streaming, tool calls, subagents, session rail —
with the same rasterized PNG/text output pipeline.

## Changes

- `crates/mew-tui/src/harness.rs`:
  - New `Backend` trait so the verb interpreter can drive pluggable backends.
  - Existing behavior moved into `LocalBackend`.
  - `Harness` is now generic over `Backend`, defaulting to `LocalBackend`; the
    public API (`h.app`, `h.actions`, `run_script`, etc.) is preserved.
  - Local-only verbs (`say`, `error`, `settings`, `settings_config`) now
    produce a clear error when used with a non-local backend.
  - `parse_key` made public for reuse by the daemon interpreter.

- `crates/mew/src/commands/tui_capture.rs`:
  - New `DaemonBackend` that connects to a mew daemon via `DaemonClient`,
    creates a session, and pumps `AgentEvent`s / `ServerMessage`s into a
    headless `App` + `TestBackend`.
  - New async-aware script verbs for daemon mode: `send`, `wait_turn`, `expect`,
    `screenshot_dir`.
  - New `run_script_daemon` / `run_interactive_daemon` paths.
  - Screenshot/MP4 encoding work in daemon mode the same way as harness mode.

- `crates/mew/src/cli.rs` and `crates/mew/src/main.rs`:
  - Added `--connect <url>` to `TuiCapture`.
  - Made `tui_capture::run` async and wired `.await`.

- `crates/mew/Cargo.toml`:
  - Added `mew-raster`, `tiny-skia`, `png` dependencies for the daemon backend.

- `.mew/skills/tui-capture/SKILL.md`:
  - Documented `--connect` and the daemon-mode verbs.
  - Added a daemon capture example and updated the comparison table.

## Tests

- `test_daemon_capture_fake_provider_end_to_end` — starts an in-process daemon
  with `FakeProvider`, runs `send` / `wait_turn` / `expect`, and verifies the
  response appears in the output.
- `test_daemon_capture_expect_fails_when_missing` — verifies `expect` reports a
  clear error when the expected text is absent.
- `test_daemon_capture_screenshot_dir_writes_png` — verifies numbered PNGs are
  written when `screenshot_dir` is set in daemon mode.

## CI Gate

- `cargo clippy -p mew -p mew-tui -p mew-daemon -- -D warnings` — clean
- `cargo test -p mew -p mew-tui` — 154+ tests pass (mew-tui harness tests +
  new daemon capture tests)
- `cargo test --all` — one pre-existing `mew-daemon` concurrency test
  (`slash_command_during_in_flight_turn_does_not_block_stream`) is failing
  independently of these changes; all other tests pass.

---


# 2026-07-13 — Real-provider `mew tui-capture --connect` improvements

## Summary

Improved daemon-connected `mew tui-capture` while recording a real `umans`
demo: streaming frames are now captured during `wait_turn`, the TUI status bar
shows the real model/provider, and a dev doc was added.

## Changes

- `crates/mew/src/commands/tui_capture.rs`:
  - `DaemonBackend::wait_turn` now draws and captures a frame roughly every
    100 ms while `app.streaming` is true, so recorded MP4s show the response
    appearing progressively instead of jumping straight to the final frame.
  - `DaemonBackend::connect` reads `model`/`provider` from
    `ServerMessage::SessionReady` and sets `app.status.model`/`provider`, so the
    status bar shows the real backend (e.g. `umans/umans-coder`) instead of
    `mewd/daemon`.
  - Replaced `wait_for_session_id` with `wait_for_session_ready`.

- `crates/mew-daemon/src/client.rs`:
  - `SessionReady` is now forwarded to `notify_tx` so callers can read the
    active model/provider from it.

- `docs/development/dev-tui-capture.md`:
  - New dev doc explaining how to record real-provider captures with
    `mew tui-capture --connect`, including daemon setup, script verbs, tips,
    and troubleshooting.

- `docs/development/dev-tui.md`:
  - Added a cross-reference to the new dev-tui-capture doc.

## Captures produced

- `notes/capture/umans-optimized-reverse-binary-string-demo.mp4`
- `notes/capture/umans-optimized-reverse-binary-string-final.png`

Script used:

```text
send "What's the optimised way of reversing a binary string in Rust?"
wait_turn 120000
expect "reverse"
pause 4000
screenshot /tmp/umans-optimized-final.png
```

Command:

```bash
export MEW_CRED_UMANS=$(grep MEW_CRED_UMANS .env | cut -d= -f2-)
./target/debug/mew daemon --provider umans --model umans/umans-coder \
  --port 127.0.0.1:0 --background --log /tmp/mew-capture.log
./target/debug/mew tui-capture \
  --script /tmp/capture-umans-optimized.txt \
  --connect ws://127.0.0.1:<port> \
  --mp4 notes/capture/umans-optimized-reverse-binary-string-demo.mp4 \
  --width 100 --height 30
./target/debug/mew daemon --stop
```

## CI Gate

- `cargo clippy -p mew -p mew-daemon -- -D warnings` — clean
- `cargo test -p mew tui_capture` — 5 tests pass

---

# 2026-07-13 — Settings overlay capture verb

## Summary

Wired the in-app settings overlay into the TUI harness so it can be captured
deterministically with `mew tui-capture`.

## Changes

- `crates/mew-tui/src/harness.rs`:
  - New `settings` verb — opens the settings overlay with the default config.
  - New `settings_config <path>` verb — opens the overlay loaded from a controlled
    TOML file, making captures independent of the user's real `config.toml`.
  - Updated `help` output to list the new verbs.
  - Added 4 tests covering both verbs, missing-path handling, and help text.
- `.mew/skills/tui-capture/SKILL.md`:
  - Documented `settings` and `settings_config` in the verb table.
  - Added a settings-overlay example script.

## Captures produced

Ran:

```bash
MEW_CRED_Z_AI=fake ./target/debug/mew tui-capture \
  --script notes/settings-capture-zai.txt --width 100 --height 30 \
  > notes/images/settings-capture-zai-output.txt
```

- `notes/images/settings-capture-zai-output.txt` — text snapshots of each frame
- `notes/images/settings-only-zai.png` — settings overlay open
- `notes/images/settings-accounts-only-zai.png` — Accounts category selected
- `notes/images/settings-zai-details.png` — z-ai account details panel

Generated files live in `notes/images/`, which is gitignored.

## CI Gate

- `cargo clippy -p mew-tui -- -D warnings` — clean
- `cargo test -p mew-tui harness` — 15 tests pass
- `cargo fmt` — clean

## Overview

Implemented the full "mew films itself" plan from `notes/mew-tui-self-capture-plan.md`:
the agent can now capture screenshots and record videos of mew's own TUI — both
as human-facing artifacts (demo mp4s/gifs) and as agent-facing feedback (PNG images
for VLM inspection).

## Phase 0 — vhs skill (done)

Created `.mew/skills/tui-capture/SKILL.md` teaching the agent to use charm vhs to
record the real mew binary in a pty. Verified end-to-end: built mew, started
fake-provider daemon, ran a .tape file through vhs producing valid mp4 + png.

## Phase 1 — buffer→png rasterizer (done)

New crate `crates/mew-raster`:
- `rasterize(buf: &Buffer, opts: &RasterOptions) -> Pixmap` — converts ratatui Buffer to pixels
- `to_png(buf: &Buffer, opts: &RasterOptions) -> Vec<u8>` — encodes as PNG bytes
- Bundles IoskeleyMono-Regular.ttf + IoskeleyMono-Medium.ttf via `include_bytes!`
- Full color mapping: ratatui Color enum (named, Rgb, Indexed 256-color) → RGB
- Style modifiers: BOLD (bold font weight), REVERSED (swap fg/bg), UNDERLINED
- RasterOptions with scale (default 2× = 16×32px cells), bg/fg colors
- 10 tests covering dimensions, scaling, colors, reversed modifier, PNG validity

Harness integration (`crates/mew-tui/src/harness.rs`):
- `Harness::screenshot(path)` method — renders current frame to PNG
- `screenshot <path>` verb in the script format

## Phase 2 — video from the harness (done)

Frame recording on `Harness`:
- `start_recording()` / `stop_recording()` — capture frames after each verb
- `capture_frame()` — rasterize current buffer into Pixmap (no-op when not recording)
- `duplicate_last_frame(count)` — clone last frame N times for timing
- `encode_mp4(path, fps)` — write numbered PNGs to temp dir, shell to ffmpeg
- Automatic frame capture: `type_str` emits one frame per keystroke, `say` emits
  one frame per 8-char delta chunk (streaming animation)

New script verbs: `start_recording`, `stop_recording`, `pause <ms>`, `record <path> [fps]`

5 new tests. All 12 harness tests + golden tests pass.

## Phase 3 — expose to the agent (done)

New `mew tui-capture` subcommand (`crates/mew/src/commands/tui_capture.rs` + CLI variant):
- `--script <path>` — reads a harness script file
- `--mp4 <path>` — auto-wraps script in recording/encoding
- `--fps <n>` — framerate (default 30)
- `--width` / `--height` — terminal dimensions (default 80×24)
- No provider needed — doesn't trigger state health check

Updated `.mew/skills/tui-capture/SKILL.md` with both methods documented:
`mew tui-capture` (deterministic) and `vhs` (glamour shots), with comparison table.

## Phase 4 — a/b legibility experiment (done)

Rendered the same TUI frame three ways: vhs screenshot, rasterizer PNG, text dump.
Sent to two VLMs (umans/umans-coder and minimax/MiniMax-M3).

Results:
- Both methods legible to VLMs (both models could read all text)
- Rasterizer scored higher on average (8.0 vs 7.0 across both models)
- Rasterizer wins on determinism (vhs captured wrong frame on first attempt)
- vhs wins on visual fidelity (real terminal chrome)
- Text dump remains most reliable for structural analysis

## cosmic-text renderer upgrade (done)

Replaced ab_glyph with cosmic-text for glyph rendering:
- Uses `cosmic-text`'s `Buffer::draw()` API with `swash` for proper glyph shaping
- Fixes: box-drawing chars (solid divider lines instead of broken/dotted), multi-cell
  graphemes, bold/italic via cosmic-text Attrs system
- VLM comparison after upgrade: minimax legibility jumped from 7/10 → 9.5/10
- Divider line artifact (faint dotted rules) eliminated — confirmed by both VLMs

## Files created/modified

New files:
- `crates/mew-raster/Cargo.toml`
- `crates/mew-raster/src/lib.rs`
- `crates/mew-raster/assets/IoskeleyMono-Regular.ttf`
- `crates/mew-raster/assets/IoskeleyMono-Medium.ttf`
- `crates/mew/src/commands/tui_capture.rs`
- `.mew/skills/tui-capture/SKILL.md`

Modified files:
- `Cargo.toml` — added `mew-raster` to workspace members
- `crates/mew-tui/Cargo.toml` — added `mew-raster`, `tiny-skia`, `png`, `tempfile` deps
- `crates/mew-tui/src/harness.rs` — screenshot verb, frame recording, video encoding
- `crates/mew/src/cli.rs` — `TuiCapture` subcommand
- `crates/mew/src/main.rs` — dispatch arm
- `crates/mew/src/commands/mod.rs` — module declaration

## CI Gate

- `cargo clippy -p mew-raster -p mew-tui -p mew -- -D warnings` — clean
- `cargo test -p mew-raster` — 10 tests + 1 doctest pass
- `cargo test -p mew-tui` — 12 harness tests + 5 golden tests + 1 doctest pass
- End-to-end: `mew tui-capture --script capture.txt --mp4 demo.mp4` — valid MP4 produced

---

# 2026-07-12 — Context Window Inspector: Steps 7, 8, 9

## Step 8 — Calibration Harness (done)

Added `#[cfg(test)]` calibration module at the end of `crates/mew-agent/src/manifest.rs`.
Uses direct `assert_eq!` equality assertions against known-exact token counts
for `cl100k_base` and `o200k_base` encodings. No `DriftEntry` struct or ratio
tracking.

- 6 tests: `test_calibration_cl100k_short_string`, `test_calibration_cl100k_long_text`,
  `test_calibration_o200k_short_string`, `test_calibration_o200k_code_snippet`,
  `test_calibration_json_schema_text`, `test_model_encoding_routing`.
- Independently verified baseline: "Hello world" = 2 tokens for cl100k_base.
- JSON fixture distinguishes cl100k (36) from o200k (38) for routing tests.

## Step 9 — Subagent Manifests (done)

Threaded manifests from child agent → `SubagentResult::Complete` →
`ToolStateCompleted.metadata` → wire protocol → web client/store.

### Changes by file

- `crates/mew-subagents/src/lib.rs` — Added `manifests: Vec<TurnManifest>` to
  `SubagentResult::Complete` and `SubagentEvent::Finished`.
- `crates/mew-agent/src/runner.rs` — Added `extract_manifests` helper.
  Threaded manifests through Cancelled, Error, and Completed paths (both
  `SubagentEvent::Finished` and `SubagentResult::Complete`).
- `crates/mew-agent/src/tools.rs` — Updated 3 destructure sites
  (`execute_subagent_call`, `execute_subagent_start`, `execute_subagent_wait`)
  to capture manifests and store on `ToolStateCompleted.metadata` as JSON.
- `crates/mew-agent/src/manifest.rs` — Added `tool_call_label` helper and
  subagent label detection (`"subagent: {name}"` instead of `"tool: subagent"`).
  Updated `part_label_kind` and `build_part_segment`.
- `crates/mew-agent/src/lib.rs` — Added `manifests: Vec<TurnManifest>` to
  `AgentEvent::SubagentEnd`. Updated manual Debug impl (uses `..` to omit).
- `crates/mew-agent/src/agent.rs` — Threaded manifests through async subagent
  pump's `SubagentEvent::Finished` → `AgentEvent::SubagentEnd`.
- `crates/mew-protocol/src/lib.rs` — Added `manifests: Vec<TurnManifest>` with
  `#[serde(default)]` to `ServerMessage::SubagentEnd`. Added round-trip tests.
- `crates/mew-daemon/src/lib.rs` — Updated `translate_event` to pass manifests
  from `AgentEvent::SubagentEnd` to `ServerMessage::SubagentEnd`.
- `crates/mew-daemon/src/client.rs` — Updated `ServerMessage::SubagentEnd`
  handler to thread manifests to `AgentEvent::SubagentEnd`.
- `mew-web-client/src/index.ts` — Added `manifests?: TurnManifest[]` to
  `subagent-end` event type and wire `ServerMessage` type.
- `mew-web-ui/src/stores/session.ts` — Added `manifests: TurnManifest[]` to
  `SubagentInfo`, populated in `onSubagentEnd`, initialized in `onSubagentStart`.

### Tests added

- `test_manifest_labels_subagent_tool_calls` (mew-agent manifest)
- `subagent_end_manifests_roundtrip`, `subagent_end_no_manifests_roundtrip` (mew-protocol)

## Step 7 — Mobile Manifests (done)

### Changes by file

- `crates/mew-mobile-core/src/state.rs` — Added `MobileTurnManifest`,
  `MobileSegment`, `MobileSegmentKind`, `MobileAssistantMeta` UniFFI records.
  Added `to_mobile_manifest` conversion function. Extended `ChatMessage` with
  `assistant_meta: Option<MobileAssistantMeta>`. Updated `apply_provider_event`
  to extract manifest from `MessageEnd` and attach to last assistant message.
  Added `last_manifest` to `SessionState`. Updated all `ChatMessage` construction
  sites with `assistant_meta`.
- `crates/mew-mobile-core/src/events.rs` — Added `manifest` field to
  `CoreEvent::TurnEnded`. Added `CoreEvent::SubagentEnd` variant with
  `parent_call_id`, `child_session_id`, `outcome`, `manifests`.
- `crates/mew-mobile-core/src/lib.rs` — Updated `translate_message` to extract
  manifest from `MessageEnd` and emit in `CoreEvent::TurnEnded`. Extracted
  `SubagentEnd` from the no-op match group into its own arm that emits
  `CoreEvent::SubagentEnd`. Fixed `SessionHistory` handler to populate
  `assistant_meta` from `msg.assistant`.

### Tests added

- `test_to_mobile_manifest_preserves_fields` — verifies field mapping + source_id drop
- `test_message_end_extracts_manifest` — verifies manifest extraction + attachment to assistant message
- `test_chat_message_has_assistant_meta` — verifies ChatMessage field
- `test_message_end_no_manifest` — verifies graceful handling when manifest is None

## CI Gate

- `cargo fmt --check` — clean
- `cargo clippy -p mew-agent -p mew-subagents -p mew-protocol -p mew-mobile-core -p mew-daemon -- -D warnings` — clean
- `cargo test -p mew-agent -p mew-subagents -p mew-protocol -p mew-mobile-core` — 284 tests pass
- `pnpm build` (web-client) — clean
- `pnpm build` (web-ui) — clean
- `pnpm test` (web-ui) — 50 tests pass
- `just ios-core` (Swift binding regen) — manual step (AC.12), not run

## 2026-07-13 — mew-raster: ~190x faster frame rasterization

`Rasterizer::rasterize` went from ~410ms to ~2.2ms warm per frame (160x48, scale 2, release), measured with the new `cargo run -p mew-raster --release --example bench`.

What changed in `crates/mew-raster/src/lib.rs`:
- Replaced per-pixel `tiny_skia::fill_rect` calls (one per glyph pixel via cosmic-text's `draw()` callback) with direct blends into the pixmap buffer (`blit_glyph`). This alone: 410ms → 92ms.
- Discovered the remaining cost was cosmic-text shaping+layout per row per frame (was hidden — lazy layout inside `draw()`). Replaced per-line shaping with a per-symbol shape cache: each unique (symbol, bold, italic) is shaped once, then frames are pure mask blitting at cell origins. 92ms → 2.2ms. Also removes per-cell String allocs and per-row TextBuffer creation.
- Backgrounds: single `pixels_mut().fill()` for the canvas + merged same-bg cell spans written as row slices.
- PNG encode: `png::Compression::Fast` (104ms → 11ms per screenshot; files ~40% larger, fine for capture).
- New shared `mew_raster::encode_frames_mp4` pipes raw RGBA to ffmpeg (`-f rawvideo`) instead of writing a temp PNG per frame; both `mew-tui/src/harness.rs` and `mew/src/commands/tui_capture.rs` `encode_mp4` impls now delegate to it. Dropped the now-unused `png` dep from both crates.

Verified: mew-raster tests (10), harness tests (15), tui_capture daemon tests (5), plus e2e smoke: harness script → mp4 (ffprobe-valid h264) and PNG screenshot visually checked (bold, bg spans, box-drawing, colors all correct).

Note: capture perf logging in `wait_turn` (`rasterize_ms`) should now read ~2ms; if it's still slow, check for a debug build — the blend loops are heavily penalized without optimization.

## 2026-07-13 — mew-raster: fix tofu glyphs and block-element seams

Follow-up to the rasterizer rewrite; user screenshot showed the welcome-screen cat rendering as tofu boxes and the block "mew" wordmark with grid seams.

- Tofu: `Shaping::Basic` (`shape_skip` in cosmic-text) never font-falls-back for `Family::Monospace`, so kana/CJK punctuation missing from IoskeleyMono rendered as .notdef. Pre-existing bug, not a regression. Switched `shape_symbol` to `Shaping::Advanced` — per-glyph system-font fallback (Hiragino Sans for kana on macOS). Cost is once per unique symbol thanks to the shape cache; warm frame time unchanged (2.2ms).
- Seams: block elements (U+2580–U+259F) from the font don't cover the full cell (advance ≈15.6px < 16px cell), so per-cell placement left background lines between cells. Added `draw_block_element` — geometric fills in cell-eighths (halves, eighth-blocks, shades with alpha, quadrants), same approach as terminal emulators. Blocks now tile edge-to-edge.
- New tests: full block fills entire cell incl. corners, adjacent blocks tile without seams, half block covers exactly its half. 13 mew-raster tests pass; harness (15) and tui_capture (5) still green; welcome screen visually verified (cat + seamless wordmark).

## 2026-07-13 — tui-capture: animate spinner in daemon-mode recordings

The input-bar spinner was static in captured videos: `app.tick()` (which advances `spinner_frame` every 5th tick) is driven by the EventLoop's 60fps tick task, which the headless `DaemonBackend` never spawns. `wait_turn` now calls `app.tick()` on its own 16ms cadence, matching the real TUI's animation rate. Local harness mode is unaffected by design (turns are synchronous; `pause` duplicates frames).

Test: `test_spinner_advances_while_streaming` — long fake-provider response (~500ms stream), asserts `spinner_frame` advanced during `wait_turn`. Written failing-first, passes after the fix; all 6 tui_capture tests green.

## 2026-07-17 — Tauri desktop shell scaffold

- Added `mew-web-ui/src-tauri`, a thin Tauri 2 shell around the existing React/Vite app.
- Kept the shell outside the root Cargo workspace so desktop-only dependencies do not enter the core workspace checks.
- Added `just desktop-dev` and `just desktop-build`, plus Tauri-aware Vite settings for fixed-port dev/HMR and WebKit-compatible production builds.
- Generated the initial app icons from `mew-web-ui/public/favicon.svg`.
- Daemon supervision and the desktop host/runtime adapter are intentionally left for the next slice.

Verified: `cargo check --manifest-path mew-web-ui/src-tauri/Cargo.toml`, `pnpm --filter mew-web-ui build`, `pnpm --filter mew-web-ui exec tauri info`, and `pnpm --filter mew-web-ui desktop:build`.

## 2026-07-17 — Tauri daemon supervision and shared host bootstrap

- Added `DaemonSupervisor` to the Tauri host. It supports an explicit
  `MEW_DESKTOP_DAEMON_URL`, otherwise reserves a loopback port, launches
  `mew daemon --port ...`, waits for TCP readiness, and kills the child when
  the desktop process exits.
- Added `mew-web-ui/src/lib/host.ts`. Browser mode still derives `/ws` from the
  current origin; Tauri mode resolves the daemon URL through the native
  `daemon_ws_url` command before mounting the same React tree.
- Updated daemon startup so a stale persisted provider/model state cannot block
  a non-interactive desktop-owned daemon with an interactive healing prompt.
- Release sidecar packaging remains a follow-up. The current host discovers
  `mew` from `MEW_DESKTOP_DAEMON_BINARY`, a sibling/debug/release path, or
  `PATH`.

Verified: `pnpm --filter mew-web-ui test` (52 tests), `pnpm --filter mew-web-ui build`,
desktop-host Rust tests (4), desktop-host clippy, `cargo test -p mew` (105 tests),
and a Tauri dev smoke with a real loopback daemon and established WebSocket
connection. `cargo fmt --all -- --check` still reports unrelated pre-existing
formatting differences in several dirty files; the changed desktop and daemon
files are formatted.

## 2026-07-17 — Tauri sidecar packaging and daemon ownership

- Added the Tauri shell plugin and `bundle.externalBin` configuration for an
  architecture-specific `mew` sidecar.
- Added `mew-web-ui/scripts/build-sidecar.mjs`; `desktop:dev` prepares a debug
  sidecar and `desktop:build` prepares a release sidecar before Tauri runs.
- Made ownership explicit: `MEW_DESKTOP_DAEMON_URL` attaches without owning or
  killing the process; bundled sidecars and binaries launched through
  `MEW_DESKTOP_DAEMON_BINARY` are owned by the current desktop process.
- Removed the release fallback that could mistake the Tauri executable itself
  for the `mew` daemon, and corrected the repository target paths used by the
  development fallback.

Verified: sidecar unit tests, browser tests (52), Tauri dev with a bundled debug
sidecar, explicit attach to an existing daemon on port 25566, clean child
shutdown in both modes, and the packaged release executable. The release app
bundle contains `Contents/MacOS/mew` alongside the desktop host.

## 2026-07-17 — Desktop daemon rendezvous and adversarial UX pass

- Reworked the Tauri supervisor around a shared loopback rendezvous port
  (25566 by default, with MEW_DESKTOP_DAEMON_PORT as an override). It performs
  a real WebSocket ping/pong check before launching anything, attaches to an
  existing mew daemon without owning it, rejects occupied non-mew ports, and
  lazily starts owned sidecars/processes only when the frontend requests the
  endpoint.
- Made native host bootstrap failures recoverable in the UI with a clear error
  message and retry action. The frontend connection manager now reports
  connection errors, retries with bounded backoff, and recovers after both
  pre-open failures and established socket closes.
- Removed automatic session creation from the home route. Reopening the last
  session is explicit and failure leaves a useful new-session path instead of
  silently replacing the session.
- Added workspace context to the header, session search, clickable recent
  sessions, a labeled view dropdown for timeline/workspace/grouped modes, and
  mobile dock spacing that keeps the composer, status footer, permission toast,
  and bottom navigation from overlapping.
- Added shared press/focus behavior and reduced-motion handling, replaced the
  reasoning bounce indicator, and added labels to icon-only controls.

Verified: web UI tests (52), web UI production build, web-client tests (12),
Tauri host tests (7), Tauri host clippy, and a real ping/pong probe against the
existing daemon on 127.0.0.1:25566.

## 2026-07-17 — Desktop release verification

Verified the updated host and shared React app through
pnpm --filter mew-web-ui desktop:build. The macOS app and aarch64 DMG both
bundled successfully with the architecture-specific mew sidecar.

## 2026-07-17 — Final daemon lifecycle and UX verification

- Corrected the interim daemon notes above: desktop startup now uses the shared
  25566 rendezvous port, protocol-level WebSocket ping/pong health checks, lazy
  startup, attach-without-ownership, and an explicit occupied-port failure.
- Added retry coverage for both pre-open failures and errors emitted after an
  established WebSocket connection.
- Kept the final frontend pass focused on recoverable states, session discovery,
  workspace context, labeled view switching, mobile dock spacing, keyboard
  focus, reduced motion, and icon-only control labels.

Verified: web UI tests (54), web-client tests (13), web UI production build,
Tauri host tests (7), Tauri host clippy, Tauri formatting, scoped diff checks,
and the packaged macOS app plus aarch64 DMG from the release build.

## 2026-07-17 — Session rail overlap and radius restoration

- Reserved the hover-action column inside each session title row so regenerate,
  pin, archive, and grouping controls cannot cover session text.
- Restored the shared `--radius` CSS token at `0.625rem`; the theme generator
  emits color variables only, so this dimension belongs in the web base layer.
- Added a regression test for the reserved session-action space and explicit
  cleanup between rail renders.

Verified: web UI tests (55), the production web build, generated CSS containing
the radius token and rounded utilities, and scoped diff checks.

## 2026-07-17 — Interactive activity panel and workspace lifecycle pass

- Rebuilt the Activity sheet around six visible icon-and-label sections, with
  actionable-first opening, wrapped mobile tabs, richer empty states, pinned
  context, stable question resolution, and rounded desktop/mobile treatment.
- Prevented Files and Changes from probing sessions without a workspace; they
  now explain the missing context instead of injecting daemon errors into chat.
- Made browser-created sessions inherit and persist the daemon launch cwd,
  exposed it through `SessionReady`, kept writer metadata synchronized, and
  threaded it into resumed agent construction so chat tools, file browsing, and
  git status share one workspace.
- Serialized overlapping client session lifecycle requests. This prevents an
  old failed attach from rejecting a new-session request and fixes the live
  `session not found` route handoff.
- Limited the default session timeline to the 40 newest sessions while keeping
  search access to older history, replaced raw ids with useful titles, and
  added accessible labels to session/group actions.

Verified: interactive browser smoke tests for fresh-session creation, Activity,
Files, Changes, recovery, and workspace-backed git status; web UI tests (64),
web-client tests (14), session metadata regression coverage, bridge e2e, daemon
and workspace builds, production web build, and scoped diff checks.

## 2026-07-17 — Keyboard-first session and project search

- Added the shadcn Command primitive via `npx shadcn@latest add command --yes
  --overwrite` and moved the existing palette onto the shared UI component.
- Expanded cmd/ctrl+k into a searchable workspace launcher for actions, sessions,
  projects, titles, summaries, first-message content, loaded current-session
  content, folders, and dates.
- Added `project:`, `folder:`, `before:`, and `after:` query operators, with
  project results opening a fresh session in that directory.
- Added focused search tests covering content, path, combined filters, inclusive
  dates, and project matching. The index tolerates both seconds and milliseconds
  from older and newer daemon metadata.

Verified: browser interaction for cmd/ctrl+k, content search, no-match state, and
project results; web UI tests (68) and production web build.

## 2026-07-17 — Attention-first notification hierarchy

- Added a shared attention taxonomy with explicit labels: `Permissions needed`,
  `Question · needs input`, and `Turn failed`.
- Session ordering now puts attention ahead of recency, running state, and pin
  state in the timeline, grouped view, and workspace folders. The current
  session no longer gets an exception.
- Added a persistent `Needs attention` queue at the top of Activity, with
  session-level badges and a header indicator. Queue items navigate directly to
  the session that needs action.
- Stopped treating ordinary turn completion as an in-app notification. A
  successful follow-up clears the previous failure alert, while permission and
  question items remain driven by their live pending counts.
- Added regression coverage for precedence, explicit labels, queue rendering,
  current-session ordering, and alert lifecycle cleanup.

Verified: interactive Activity-panel smoke test, web UI tests (73), production
web build, and scoped diff checks.

## 2026-07-17 — Browser-use vertical slice

- Added daemon-owned browser commands backed by the installed native
  `agent-browser` CLI. Browser sessions are keyed to mew sessions and support
  HTTP(S) navigation, accessibility snapshots, screenshots, click, fill, key
  press, and close.
- Extended the Rust and TypeScript wire clients with browser state, snapshot,
  and screenshot messages.
- Added a Browser tab to the Activity rail with URL navigation, semantic text
  inspection, screenshot capture, and basic element actions.

Verified: `agent-browser` opened and inspected example.com, Rust daemon and
protocol tests (101 protocol tests, 3 daemon tests), web-client build, and
production web build.

## 2026-07-17 — macOS CEF authoritative-browser proof of concept

- Added `native/cef-host`, a standalone Rust `cef-rs` host that creates a
  visible Chromium window and exposes loopback CDP for `agent-browser`.
- Added the macOS helper target and CEF bundle metadata required to produce a
  real `.app` with the framework and helper app bundles.
- Added a stable per-user CEF cache path and defaulted the unsigned development
  host to Chromium's mock keychain to avoid repeated macOS Keychain prompts.
  `MEW_CEF_USE_SYSTEM_KEYCHAIN=1` opts back into real browser Keychain storage.

Verified: native all-targets check, official CEF bundle generation, launch of
the bundled app, CDP `/json/version`, accessibility snapshot, and an
`agent-browser --cdp 9223` click against the same visible page.

## 2026-07-17 — Tauri native sibling integration

- Exposed `mew-cef-host` as a reusable macOS embedding library. It creates a
  CEF child `NSView` inside the Tauri content view, keeps bounds and visibility
  on the main thread, and uses Tauri's external message-pump callback.
- Added Tauri commands for CEF availability, bounds, navigation, visibility,
  and close/hide behavior. React now positions the native surface over the
  Browser panel viewport while retaining the existing WKWebView controls and
  text snapshot fallback.
- Routed desktop daemon `agent-browser` calls to the same CEF CDP endpoint and
  removed the incompatible persistent agent-browser session flag in that mode.
- Added CEF framework preparation for development links and release copies,
  macOS framework bundling, CEF helper-process dispatch, mock-keychain defaults,
  and graceful fallback when the native sibling is unavailable.

Verified: native all-targets check, Tauri cargo check, web UI tests (73), web
production build, daemon CDP argument tests, Tauri debug `.app` bundle build,
and standalone CEF CDP control with `agent-browser`. The bundled Tauri process
reaches CEF DevTools, but its GPU subprocess is still unstable in this local
environment even though embedded mode requests software rendering; GPU remains
opt-in for experiments via `MEW_CEF_ENABLE_GPU=1`. macOS sandbox/helper
hardening is still a follow-up before release signing.

## 2026-07-17 — CEF reopen lifecycle hardening

- Fixed helper startup ordering so the CEF command-line wrapper is never
  constructed before `libcef` is loaded.
- Packaged and selected the dedicated `mew-cef-host-helper` for Tauri CEF
  subprocesses. The helper now resolves both nested CEF helper bundles and the
  flat `Contents/MacOS` layout used by Tauri.
- Removed the inline message-loop call from `CefInitialize`, seeded the first
  external-pump turn after Tauri re-enters its event loop, isolated the embedded
  cache from the standalone host, and close/release the browser before
  `CefShutdown`.
- Split native browser layout and visibility effects so tab unmounts and
  reopenings cannot enqueue stale hide/show work against the native view.

Verified: native helper and embedding tests, Tauri cargo check, web UI tests
(73), frontend build, Tauri debug `.app` bundle build, CEF page startup, and
two launches against the same cache with the page available at the CDP target.

## 2026-07-17 — Daemon sidecar rebuild

- Added the three browser message variants to the TUI capture message-name
  formatter so the daemon binary remains exhaustive as the wire protocol
  evolves.
- Rebuilt the debug daemon sidecar and bundled debug `.app` after the CEF
  lifecycle changes.

Verified: `cargo test -p mew --bin mew tui_capture` (6 passed),
`pnpm desktop:prepare:dev`, and `pnpm tauri build --debug --bundles app
--no-sign`.

## 2026-07-17 — Workspace surfaces design direction

- Established the frontend direction as a Codex-style desktop workspace with
  two independent surfaces: a default-on pinned summary for project/session
  orientation and a separately toggled workbench for activity, browser,
  changes, and review.
- Captured the design context in `.impeccable.md`: the product should feel
  fast, focused, and alive, with attention surfaced before ordinary session
  navigation and keyboard-first, low-latency interactions.
- Identified the main implementation constraint: the native CEF browser must
  remain mounted while switching workbench tabs so its visible page and CDP
  target survive tab changes.

## 2026-07-17 — Independent workspace surfaces implementation

- Added persisted workspace-surface state with a pinned summary defaulting on
  and a workbench defaulting off. `⌘B` controls the summary and `⌘⇧B` controls
  the workbench independently.
- Added a root-level `WorkspaceFrame` so the desktop workbench is a dock beside
  the chat surface. Mobile keeps the existing sheet behavior.
- Reworked the activity rail into top-level Activity, Browser, Changes, and
  Review tabs. Browser lazy-mounts on first use and stays mounted while tabs
  switch, while CEF visibility follows the active surface.
- Added a first local working-tree Review surface and removed duplicate Changes
  navigation from the Activity sub-tabs.

Verified: web UI tests (80), production web build, and `git diff --check`.
The in-app browser smoke pass could not start because this sandbox rejects
binding the local Vite development port.

## 2026-07-17 — Tauri CEF dev preparation fix

- Isolated the opaque Tauri `os error 2`: sidecars were present; the failing
  resource was the CEF framework path and dev preparation was linking it.
- Updated the dev CEF preparation command to copy a real framework directory,
  matching Tauri's macOS framework resource copier.
- Confirmed `cargo check --manifest-path mew-web-ui/src-tauri/Cargo.toml` and
  `just desktop-dev` reach a successful `mew-desktop` build and launch.

## 2026-07-17 — CEF development runtime assets

- Added explicit CEF resource and locale paths for the embedded browser.
- Prepared `icudtl.dat`, CEF resources, and GPU libraries for the unbundled
  macOS debug executable.
- Updated the helper loader to use `MEW_CEF_FRAMEWORK_PATH` when running
  outside the packaged `.app` helper layout.
- The ICU and missing-library errors are gone. Remaining dev-only output is
  CEF Mach port rendezvous noise from the unbundled helper/runtime layout.

Verified: `cargo check --manifest-path native/cef-host/Cargo.toml`,
`cargo fmt --all -- --check`, and desktop launch through `just desktop-dev`.

## 2026-07-17 — CEF diagnostic cleanup

- Kept the confirmed synthetic bundle/rendezvous fix and external-pump
  backstop.
- Removed the confirmed no-op occlusion switches and reverted the diagnostic
  800×600 initial bounds to 1×1.
- Removed inert `was_hidden`/`was_resized` host notifications from the embed
  path after verification showed they do not affect this CEF build.
- Added the dev helper framework/resource loading path and development CEF
  asset preparation needed by the unbundled macOS executable.

Verified: native and Tauri cargo checks, cargo formatting, and diff hygiene.

## 2026-07-18 — Codex-style browser workbench tabs

- Added a tested browser-tab reducer with add, select, update, and close
  behavior that always leaves one usable new-tab surface.
- Added a compact tab strip inside the Browser workbench with explicit close
  and new-tab controls, hostname labels, and `⌘T`/`⌘W` shortcuts.
- Kept the browser surface mounted while switching workbench modes and made
  the native CEF visibility follow both the active workbench mode and the
  active browser tab.
- Exercised the interaction in the local app: opened the workbench, created a
  second tab, navigated it to example.com, and switched back to the new tab.

Verified: browser-tab and RightRail tests (12 passed), TypeScript build,
production web build, and `git diff --check`. The local smoke app still reports
the pre-existing daemon connection outage, but the workbench remains usable.

## 2026-07-18 — Packaged Tauri native smoke test

- Built and launched the debug `.app` bundle with the real macOS bundle
  identity, including the bundled CEF framework and helper.
- Confirmed the clean packaged launch reaches the native event loop and starts
  the CEF helper processes without the earlier ICU/resource errors.
- The embedded CEF target still advertises a DevTools page but has no renderer
  child; `Page.enable` times out. This is the remaining native browser blocker
  before the browser workbench can be exercised visually in the packaged app.
- The Computer Use accessibility probe could not retrieve the native AX tree in
  this environment, so no source changes were made from this smoke pass.

Verified: Tauri debug bundle, clean native launch, process/helper inspection,
and `git diff --check`. `desktop:verify:cef` reaches the DevTools endpoint but
fails at `Page.enable` timeout.

## 2026-07-18 — CEF renderer startup unblocked

- Switched embedded browser creation to CEF's asynchronous API and retained the
  browser handle from `on_after_created`, removing the create-time race.
- Added the macOS helper app bundle layout CEF expects under
  `Contents/Frameworks`: renderer, GPU, plugin, alerts, and base helpers named
  from the Tauri executable (`mew-desktop Helper*.app`).
- Kept the flat helper for fallback/dev preparation, but let packaged CEF use
  the nested helper apps and propagate the bundled framework path to children.
- Re-sign the generated app after adding nested helpers so the debug and
  release bundle workflows remain verifiable on macOS.

Verified: web tests (86), native and Tauri cargo checks/clippy, a rebuilt and
launched packaged `.app`, live renderer helper processes, `codesign --verify
--deep --strict`, and all 7 `desktop:verify:cef` checks including a compositor
screenshot.

## 2026-07-18 — Shared native browser session verification

- Launched the packaged Tauri app and attached `agent-browser --cdp` to the
  same CEF target used by the visible app surface.
- Navigated from `https://example.com/` to `https://example.org/` through the
  agent path and captured a screenshot, confirming the user-facing renderer
and agent control path share one browser session.

## 2026-07-18 — Tauri dev framework preparation fix

- Restored the normal `desktop:dev` CEF preparation step to copy the framework
  instead of symlinking it.
- Tauri's macOS build script walks and recopies the framework, and its symlink
  failure surfaced only as `No such file or directory (os error 2)` during the
  custom build command.

Verified: `desktop:prepare:dev`, `desktop:prepare:cef:dev`, `tauri dev`, live
CEF renderer helper processes, and `agent-browser --cdp 9223` reading the
embedded page title and URL.

## 2026-07-18 — Browser protocol mismatch recovery

- Made both desktop preparation paths rebuild `@mew/web-client` before Vite
  or Tauri starts, so the generated SDK distribution cannot lag behind its
  browser protocol source.
- Added SDK coverage for browser response dispatch.
- Made the Browser workbench listen for browser-scoped daemon errors, clear
  its loading state, and show the protocol error inline instead of appearing
  frozen.
- The existing daemon on the shared port was started from an older binary and
  cannot decode the browser variants. Restart that daemon after browser
  protocol changes, or use `MEW_DESKTOP_DAEMON_PORT` to attach to a fresh
  instance during development.

Verified: 15 web-client tests, 87 web UI tests, production web build, and
`git diff --check`.

## 2026-07-18 — Workbench tab restructuring plan

- Decided to use the existing shadcn/Radix Tabs primitive for accessible tab
  semantics and keyboard behavior, with a custom document-strip presentation.
- Defined a unified workbench tab registry for browser pages, terminals, files,
  changes, reviews, and activity. The pinned summary remains independent.
- Recorded the staged architecture and migration plan in
  `docs/development/workbench-tabs.md`.

Verified: local shadcn Tabs implementation review, official shadcn Tabs
reference review, and `git diff --check`.

## 2026-07-18 — Resizable workbench decision

- Chose shadcn's Resizable panel composition for the conversation/workbench
  split, with a draggable and keyboard-adjustable divider.
- The workbench will collapse to zero when closed and restore its last usable
  width when reopened; mobile keeps the existing sheet behavior.
- Recorded the CLI convention for new shadcn components in `AGENTS.md`:
  `npx shadcn@latest add <component>`.

Verified: local dependency/component inventory, official shadcn Resizable
reference review, and `git diff --check`.

## 2026-07-18 — Resizable workbench shell implemented

- Added shadcn's `resizable` component through the CLI and adapted its wrapper
  to the installed `react-resizable-panels` v4 exports (`Group` and
  `Separator`).
- Replaced the fixed desktop workbench width with a keyboard- and pointer-
  adjustable conversation/workbench split.
- Persisted the workbench width, restored it after collapse/reopen, and kept
  the mobile sheet path unchanged.
- Added reducer coverage for width clamping and visibility synchronization.

Verified: 89 web UI tests, production web build, and `git diff --check`.

## 2026-07-18 — Unified workbench tabs implemented

- Added a persistent workbench tab registry for Activity, browser pages,
  terminal/job output, files, Changes, and Review.
- Migrated the old single `workbenchTab` preference into the new registry while
  keeping the pinned summary independent from the workbench.
- Replaced the fixed workbench mode buttons and nested browser tabs with one
  shared shadcn/Radix Tabs strip, close controls, a surface picker, and
  Codex-style Cmd/Ctrl+T, Cmd/Ctrl+W, and Cmd/Ctrl+1–9 shortcuts.
- Promoted browser URL/title state into top-level tabs and kept native CEF
  navigation scoped to the active browser tab. Terminal copy is deliberately
  labeled as background job output until the daemon has PTY support.
- Added reducer, migration, persistence, accessibility, browser-tab, and
  RightRail interaction coverage.

Verified: 95 web UI tests, TypeScript check, production web build, and
`git diff --check`.

## 2026-07-18 — Browser lifecycle and tab routing hardening

- Added optional `tab_id` fields to browser commands and responses, plus a
  typed `browser_error` response so late failures cannot overwrite the active
  browser tab.
- Updated the daemon and TypeScript client to echo browser tab identity while
  retaining decode compatibility with older messages that omit it.
- Filtered browser events in the workbench by tab identity, scoped browser
  close requests, and cleared loading state for every typed browser error.
- Kept browser surfaces mounted through tab switches so snapshots and errors
  survive selection changes, while inactive panels remain hidden from the
  accessibility tree and cannot trigger navigation.
- Added owner-gated native CEF bounds, visibility, navigation, and cleanup
  calls. Stale queued callbacks cannot hide or release a newly active tab, and
  failed main-thread scheduling releases only the owner that claimed it.
- Documented phase 2 as in progress. The remaining lifecycle work is a live
  unlocked-screen CEF soak covering repeated tab switches, close/reopen, and
  agent-browser CDP control.

Verified: 103 protocol tests, 15 web-client tests, 101 web UI tests, web-client
build, UI TypeScript check, production UI build, `cargo check -p mew-daemon -p
mew`, 10 Tauri shell tests, `cargo fmt --all -- --check`, and `git diff
--check`.

## 2026-07-18 — Desktop browser soak and lifecycle hardening

- Ran the live browser soak against the Tauri app: 12 rapid switches between
  two browser tabs stayed on the expected URL, close/reopen preserved the
  workbench, and a newly created tab navigated successfully.
- Verified `agent-browser --cdp 9223` against the packaged CEF page. Native CEF
  address/title events now update the active React tab, so CDP navigation and
  the visible tab strip share one authority.
- Removed the Tauri host's inherited browser-CDP environment override so the
  daemon cannot be configured to launch a second browser session. Directly
  spawned daemon children close inherited descriptors; a fresh packaged launch
  now leaves the daemon on 25566 without a copied listener on 9223. The desktop
  shell also uses Tauri's single-instance plugin to focus the existing app
  instead of creating a competing host.
- Added a web-host no-op listener regression test for native CEF events.

Verified: 102 web UI tests, 15 web-client tests, 103 protocol tests, web-client
build, UI TypeScript check, production UI build, packaged `pnpm desktop:build`,
`pnpm desktop:verify:cef` with all 7 checks passing, 10 Tauri shell tests,
`cargo check -p mew-daemon -p mew`, `cargo fmt --all -- --check`, and
`git diff --check`.

## 2026-07-18 — Desktop browser authority and shutdown fixes

- Restored the desktop browser transport contract: when CEF is available, the
  daemon receives `MEW_BROWSER_CDP_PORT` and `agent-browser` targets the same
  visible CEF page used by the workbench.
- Added an explicit CEF pump lifecycle with a stop token, callback guards, and
  a joined worker so queued message-loop work cannot run after `libcef`
  shutdown.
- Added URL-aware native CEF event filtering, native title URL context, and
  reducer-action routing for controlled workbench state so rapid navigation
  events cannot overwrite newer tab state.
- Closed inherited file descriptors inside daemon startup as well as the
  direct desktop spawn path, covering the shell-sidecar fallback.
- Removed the unused duplicate browser-tab registry and the obsolete native
  close helper.

Verified: 101 web UI tests, UI TypeScript check, production UI build, 103
protocol tests, daemon and Tauri checks, packaged `pnpm desktop:build`, fresh
packaged launch/relaunch, `pnpm desktop:verify:cef` with all 7 checks passing,
`cargo fmt --all -- --check`, and `git diff --check`.

## 2026-07-18 — UI motion and surface polish

- Added shared motion tokens and custom easing curves for press feedback,
  menus, drawers, panels, and reduced-motion behavior in the web UI.
- Replaced generic shadcn animation utilities on dialogs, alert dialogs,
  sheets, tooltips, dropdowns, and selects with explicit property-scoped
  opacity/transform transitions and origin-aware popover behavior.
- Added consistent press feedback to the shared Button primitive and the
  highest-frequency raw controls, plus restrained entry motion for activity,
  attention, permission, connection, and plan-request surfaces.
- Added rounded desktop conversation/workbench surfaces and a clearer resize
  handle treatment. Native CEF visibility remains discrete and is not CSS
  transformed or faded.
- Reduced streaming-adjacent motion noise by keeping tab selection and token
  streaming immediate and collapsing reasoning activity to one live indicator.

Verified: 101 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — iOS motion and surface parity

- Added native motion tokens for press feedback, disclosures, and surface
  changes, using SwiftUI springs/snappy timing instead of generic easing.
- Added a shared press style for frequent controls, with a restrained 0.97
  touch-down scale and reduced-motion support.
- Applied the motion vocabulary to connection banners, retry states, todo and
  tool disclosures, scrolling, typing indicators, and daemon activity pulses.
- Made custom fonts Dynamic Type-aware and corrected the MiSans runtime font
  name used by the UIKit fallback configuration.
- Standardized the main message and todo panel geometry around continuous,
  rounded surfaces while preserving native sheets and navigation.

Verified: arm64 iOS Simulator `xcodebuild` succeeded with
`CODE_SIGNING_ALLOWED=NO`, and `git diff --check` passed. The default
multi-architecture simulator build remains blocked by the checked-in
`mew_mobile_core.xcframework` missing an x86_64 simulator slice.
