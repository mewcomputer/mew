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
