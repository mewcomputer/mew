---
title: TUI capture methods
description: How to record mew's TUI for docs, demos, tests, and agent feedback — harness, daemon-connected, and vhs.
---

# TUI capture methods

mew has three ways to capture its terminal UI. They trade off determinism,
fidelity, and setup complexity differently.

| | `mew tui-capture` | `mew tui-capture --connect` | `vhs` |
|---|---|---|---|
| **Determinism** | Fully deterministic (in-process FakeProvider) | Non-deterministic (real daemon turn) | Non-deterministic (real pty + real provider) |
| **Dependencies** | ffmpeg (for video only) | running mew daemon, ffmpeg for video | vhs, ttyd, ffmpeg, Chrome |
| **Terminal chrome** | None — raw rasterized buffer | None — raw rasterized buffer | Full — window decorations, fonts, themes |
| **Network** | None | Local WebSocket to daemon | Local WebSocket in the example below |
| **CI-runnable** | Yes | Yes (if daemon is fake-provider) | No (needs Chrome + display) |
| **Best for** | Regression tests, agent feedback, fast iteration | Real chat/turn behavior for demos | Polished README gifs, social media |

## Method 1: `mew tui-capture` (deterministic, recommended)

Runs a harness script against a headless `TestBackend` with `FakeProvider`. No
network, no credentials, no daemon. Output is byte-for-byte deterministic.

### Prerequisites

- `cargo build -p mew` (produces `./target/debug/mew`)
- `ffmpeg` on PATH (only needed for `--mp4`)

### Usage

```bash
# Screenshot: produce a PNG from a script
mew tui-capture --script capture.txt --width 80 --height 24

# Video: auto-wraps the script in recording and produces an MP4
mew tui-capture --script capture.txt --mp4 demo.mp4 --fps 30

# Interactive puppet mode: agent drives the TUI via stdin
mew tui-capture -i --screenshot-dir tmp/frames
```

### Script verbs

| Verb | Description |
|---|---|
| `type <text>` | Type text into the composer |
| `key <name>` | Send a key: `enter`, `esc`, `tab`, `backspace`, arrows, `ctrl+c`, etc. |
| `submit` | Shorthand for `key enter` |
| `say <text>` | Inject an assistant text turn (streams in chunks) |
| `error <text>` | Inject a terminal error event |
| `snapshot [label]` | Render current frame as text to stdout |
| `screenshot <path>` | Save current frame as a PNG |
| `start_recording` | Begin capturing frames after each verb |
| `stop_recording` | Stop capturing; reports frame count |
| `pause <ms>` | Duplicate the last frame to simulate elapsed time |
| `record <path> [fps]` | Encode recorded frames to MP4 via ffmpeg |
| `size <w> <h>` | Resize the virtual terminal |
| `settings` | Open the settings overlay with the default config |
| `settings_config <path>` | Open the settings overlay from a TOML config |

Example:

```text
# Type a prompt and capture the response
type hello world
submit
say Welcome to mew! This is a deterministic capture.
pause 500
snapshot after-response
screenshot output.png
```

### UX gallery with structured sample state

The capture example renders conversation navigation, notification scope, activity
viewers, dialogs and input states that require more structured data than the script
verbs provide:

```bash
cargo run -p mew-tui --example ux_capture -- notes/capture/tui-current
```

Open `notes/capture/tui-current/index.html` to browse the PNGs and companion text
frames. Desktop captures are 128 columns × 40 rows at 1280 × 800 pixels; narrow
fixtures use their actual smaller viewport. The example uses the current `ui::draw`
renderer and bundled rasterizer fonts, with sample daemon messages applied through
the shared client reducer. It requires no daemon, credentials or image utilities.
Elapsed-time labels and repository context are fixed for repeatable captures.

Fixture sources live in `crates/mew-tui/examples/ux_capture/`. These images verify
layout and projected state. Terminal focus reporting, OSC titles, platform input
and actual daemon connectivity require separate live checks.

Window navigation also has checked-in text goldens covering the quiet single view,
multiple open conversations, scoped notifications and global history:

```bash
cargo test -p mew-tui --test window_golden
# After inspecting intentional visual changes:
MEW_UPDATE_GOLDEN=1 cargo test -p mew-tui --test window_golden
```

Generated galleries stay under the ignored `notes/capture/` directory. Capture
sources and regression frames are tracked, so the gallery can be rebuilt after a
fresh checkout.

## Method 2: `mew tui-capture --connect` (real daemon turn)

Connects to a running `mew-daemon` so the recording exercises the real
chat/turn pipeline: streaming, tool calls, subagents, and session rail. The
rendering is still headless through the rasterizer.

### Prerequisites

- A running daemon. Use the bundled fake provider to run without credentials.
- `ffmpeg` on PATH for `--mp4`.

### Start the daemon

```bash
mew daemon --fake-provider --port 127.0.0.1:19847
```

Keep this terminal open while you run the capture in a second one. For a
live model, configure [a provider](/docs/using-mew/providers/) and replace
`--fake-provider` with `--provider` followed by its ID.

### Daemon-mode verbs

All harness verbs work, plus:

| Verb | Description |
|---|---|
| `send "<text>"` | Submit a user message to the daemon and wait for the turn to start streaming |
| `wait_turn [timeout_ms]` | Block until the response stream finishes (default 30s) |
| `expect "<text>"` | Fail the script if the current text frame does not contain `<text>` |
| `screenshot_dir "<path>"` | Save a numbered PNG after every subsequent verb |

`pause` is especially useful in daemon mode because the response arrives
instantly in the final frame. Add `pause 4000` after `wait_turn` to hold the
last frame for a few seconds.

Example:

```text
send "Explain what this project does."
wait_turn 30000
snapshot response
screenshot response.png
```

Run:

```bash
mew tui-capture \
  --script capture.txt \
  --connect ws://127.0.0.1:19847 \
  --width 100 --height 30
```

Add `--mp4 demo.mp4` for video. When you're done, press `Ctrl+C` in the
daemon's terminal.

### How it works

`crates/mew/src/commands/tui_capture.rs` implements `DaemonBackend`, which:

1. Connects to the daemon with the same `TuiClient` as interactive chat.
2. Creates a session and reads `model`/`provider` from `ServerMessage::SessionReady`.
3. Runs the script verb by verb and applies the client events, state,
   and required actions to a headless `mew_tui::App`.
4. During `wait_turn`, while `app.streaming` is true, draws and captures a frame
   roughly every 16 ms so the recorded video shows the response appearing
   progressively.
5. Encodes the captured frames to MP4 with ffmpeg.

## Method 3: charm vhs (glamour shots)

Use [charm vhs](https://github.com/charmbracelet/vhs) to record mew's terminal
UI with full chrome: window decorations, fonts, colors, and cursor blink. vhs
drives a real terminal via headless Chrome + ttyd.

### Prerequisites

| Dependency | How to verify |
|---|---|
| `vhs` | `vhs --version` |
| `ttyd` | `which ttyd` |
| `ffmpeg` | `which ffmpeg` |
| Chrome | `ls "/Applications/Google Chrome.app"` |

Use the fake-provider daemon from [method 2](#method-2-mew-tui-capture---connect-real-daemon-turn).

### Example tape

```vhs
Output "demo.mp4"
Set FontSize 16
Set Width 1200
Set Height 600
Set Padding 20
Set TypingSpeed 30ms

Type "mew chat --connect ws://127.0.0.1:19847"
Enter
Sleep 3s

Screenshot "welcome.png"

Type "hello, what can you do?"
Enter
Sleep 3s

Screenshot "response.png"

Ctrl+C
Sleep 1s
```

Run with `vhs demo.tape`.

### vhs tips

- Paths must be quoted: `Output "demo.mp4"`.
- Use `Screenshot "file.png"` for a single image; `Output "*.png"` creates a frame directory.
- Add generous `Sleep` after launch, after submitting a prompt, and before screenshots.
- `Set` commands (except `TypingSpeed`) must appear at the top of the tape.

## Agent-driven interactive mode

Both `mew tui-capture -i` (harness) and `mew tui-capture -i --connect <url>`
(daemon) read verbs from stdin and print a text frame + optional screenshot
path after each verb. This lets an agent drive the TUI in a feedback loop:

```bash
printf '%s\n' 'send "hi"' 'wait_turn 30000' 'snapshot response' 'quit' |
  mew tui-capture -i --connect ws://127.0.0.1:19847 \
    --screenshot-dir /tmp/frames
```

Read the `--- frame ---` blocks on stdout and inspect
`/tmp/frames/frame_*.png` for visual state.

## Choosing a method

- **Regression tests / agent feedback**: Method 1. Deterministic, fast, no setup.
- **Real model demos where headless is fine**: Method 2. Uses real provider,
  still scriptable and CI-friendly.
- **Polished human-facing videos**: Method 3. Full terminal chrome, but needs
  Chrome + display.

## Common pitfalls

- **ffmpeg must be on PATH** for any MP4 output in methods 1 and 2.
- **`pause <ms>` duplicates frames** — at 30fps, `pause 1000` adds 30 identical
  frames. Use it to add natural timing.
- **Screenshots and text snapshots are independent** — `screenshot` writes a PNG,
  `snapshot` prints text. Use both for the same frame when debugging.
- **Daemon mode is non-deterministic.** The daemon controls when tokens arrive;
  always use `wait_turn` after `send` and generous timeouts.
- **State.toml corruption** can make the daemon fail to start with "unrecognized
  values". Heal it interactively or edit `last_provider`/`last_model` manually.
