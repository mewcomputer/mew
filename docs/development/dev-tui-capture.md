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
| **Dependencies** | ffmpeg (for video only) | running mew daemon + ffmpeg | vhs, ttyd, ffmpeg, Chrome |
| **Terminal chrome** | None — raw rasterized buffer | None — raw rasterized buffer | Full — window decorations, fonts, themes |
| **Network** | Never | Local WebSocket to daemon | Required (LLM or fake-provider daemon) |
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

## Method 2: `mew tui-capture --connect` (real daemon turn)

Connects to a running `mew-daemon` so the recording exercises the real
chat/turn pipeline: streaming, tool calls, subagents, and session rail. The
rendering is still headless through the rasterizer.

### Prerequisites

- A running daemon with a provider credential configured.
- `ffmpeg` on PATH for `--mp4`.

### Start the daemon

```bash
export MEW_CRED_UMANS=<key>
mew daemon \
  --provider umans \
  --model umans/umans-coder \
  --port 127.0.0.1:0 \
  --background \
  --log /tmp/mew-capture.log

# Find the actual port
grep "mew daemon listening (tcp)" /tmp/mew-capture.log
```

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
send "What's the optimised way of reversing a binary string in Rust?"
wait_turn 120000
expect "reverse"
pause 4000
screenshot response.png
```

Run:

```bash
mew tui-capture \
  --script capture.txt \
  --connect ws://127.0.0.1:<port> \
  --mp4 demo.mp4 \
  --width 100 --height 30
```

Stop the daemon when done:

```bash
mew daemon --stop
```

### How it works

`crates/mew/src/commands/tui_capture.rs` implements `DaemonBackend`, which:

1. Connects to the daemon with `DaemonClient::connect(url)`.
2. Creates a session and reads `model`/`provider` from `ServerMessage::SessionReady`.
3. Runs the script verb-by-verb, pumping `AgentEvent`s and `ServerMessage`s into
   a headless `mew_tui::App`.
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

### Start a fake-provider daemon

```bash
mew daemon --fake-provider --port 127.0.0.1:9847 --background --log /tmp/mew-vhs.log
sleep 2
```

### Example tape

```vhs
Output "demo.mp4"
Set FontSize 16
Set Width 1200
Set Height 600
Set Padding 20
Set TypingSpeed 30ms

Type "./target/debug/mew chat --connect ws://127.0.0.1:9847"
Enter
Sleep 3s

Screenshot "welcome.png"

Type "hello, what can you do?"
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
mkfifo /tmp/mew-capture-fifo
./target/debug/mew tui-capture -i --connect ws://127.0.0.1:<port> \
  --screenshot-dir /tmp/frames < /tmp/mew-capture-fifo > /tmp/mew-capture.log 2>&1 &

# Agent sends verbs and reads frames
printf 'send "hi"\n' > /tmp/mew-capture-fifo
printf 'wait_turn 60000\n' > /tmp/mew-capture-fifo
printf 'snapshot response\n' > /tmp/mew-capture-fifo
printf 'quit\n' > /tmp/mew-capture-fifo
```

The agent reads `/tmp/mew-capture.log` for `--- frame ---` blocks and inspects
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
