---
title: TUI capture methods
description: Record Mew's terminal UI using the deterministic harness, daemon mode, or charm vhs.
---

Mew supports three TUI capture methods with different tradeoffs between
determinism, realism, and setup.

| | `mew tui-capture` | `mew tui-capture --connect` | `vhs` |
|---|---|---|---|
| **Determinism** | Fully deterministic (`FakeProvider`) | Live daemon turn | Live PTY and provider |
| **Dependencies** | `ffmpeg` for `--mp4` | Running daemon, `ffmpeg` for `--mp4` | `vhs`, `ttyd`, `ffmpeg`, Chrome |
| **Terminal chrome** | Raw rasterized buffer | Raw rasterized buffer | Window frame, fonts, themes |
| **Network** | None | Local WebSocket | Local WebSocket |
| **CI-runnable** | Yes | Yes (with `--fake-provider`) | No (needs Chrome and display) |
| **Best for** | Regression tests and agent feedback | Scripted live-turn captures | Terminal videos with full chrome |

## Method 1: `mew tui-capture`

Runs a script against a headless `TestBackend` and `FakeProvider` with no
network or daemon, producing byte-for-byte repeatable output.

### Prerequisites

- `cargo build -p mew` (`./target/debug/mew`)
- `ffmpeg` on `PATH` (for `--mp4`)

### Usage

```sh
# Screenshot from a script
mew tui-capture --script capture.txt --width 80 --height 24

# MP4 video from a script
mew tui-capture --script capture.txt --mp4 demo.mp4 --fps 30

# Interactive line-by-line mode
mew tui-capture -i --screenshot-dir tmp/frames
```

### Script verbs

| Verb | Description |
|---|---|
| `type <text>` | Type text into the composer |
| `key <name>` | Send a key (`enter`, `esc`, `tab`, `backspace`, arrows, `ctrl+c`) |
| `submit` | Shorthand for `key enter` |
| `say <text>` | Inject a streamed assistant response (harness mode only) |
| `error <text>` | Inject a terminal error event (harness mode only) |
| `snapshot [label]` | Print the current frame as text to stdout |
| `screenshot <path>` | Save the current frame as a PNG |
| `start_recording` | Capture frames after every verb |
| `stop_recording` | Stop capturing and print frame count |
| `pause <ms>` | Repeat the last frame for `<ms>` milliseconds |
| `record <path> [fps]` | Encode captured frames to MP4 via `ffmpeg` |
| `size <w> <h>` | Resize the virtual terminal |
| `settings` | Open the settings overlay with default config (harness mode only) |
| `settings_config <path>` | Open the settings overlay from a TOML config (harness mode only) |

Example script:

```txt
# Type a prompt and capture the response
type hello world
submit
say Welcome to mew! This is a deterministic capture.
pause 500
snapshot after-response
screenshot output.png
```

### UX gallery with structured fixtures

Render multi-conversation views, notification badges, activity viewers, and
dialogs from structured fixture state:

```sh
cargo run -p mew-tui --example ux_capture -- notes/capture/tui-current
```

Open `notes/capture/tui-current/index.html` to inspect the PNGs and text frames
(128x40 desktop captures at 1280x800 pixels, plus narrow fixtures). Sources
live in `crates/mew-tui/examples/ux_capture/`.

Run the checked-in window navigation goldens with:

```sh
cargo test -p mew-tui --test window_golden
# Regenerate after intentional UI changes:
MEW_UPDATE_GOLDEN=1 cargo test -p mew-tui --test window_golden
```

## Method 2: `mew tui-capture --connect`

Connects to a running `mew daemon` so captures exercise real streaming, tool
calls, subagents, and the sidebar through the headless rasterizer.

### Start the daemon

```sh
mew daemon --fake-provider --port 127.0.0.1:19847
```

Keep that terminal running. To test a real model, configure a
[provider](/docs/using-mew/providers/) and pass `--provider <id>` instead of
`--fake-provider`.

### Daemon-mode verbs

Harness input and capture verbs work except `say`, `error`, `settings`, and
`settings_config` (which daemon-capture mode rejects). Daemon mode adds:

| Verb | Description |
|---|---|
| `send "<text>"` | Submit a prompt to the daemon and wait for streaming to start |
| `wait_turn [timeout_ms]` | Wait until the turn finishes (default 30,000 ms) |
| `expect "<text>"` | Fail the script if the current frame omits `<text>` |
| `screenshot_dir "<path>"` | Write a numbered PNG after each verb |

Add `pause 4000` after `wait_turn` when recording video so the final frame
remains visible.

Example script:

```txt
send "Explain what this project does."
wait_turn 30000
snapshot response
screenshot response.png
```

Run it against the daemon:

```sh
mew tui-capture \
  --script capture.txt \
  --connect ws://127.0.0.1:19847 \
  --width 100 --height 30
```

Add `--mp4 demo.mp4` for video, and press `Ctrl+C` in the daemon terminal when
done.

### How it works

`DaemonBackend` (`crates/mew/src/commands/tui_capture.rs`) connects via
`TuiClient`, creates a session, applies incoming updates to `mew_tui::App`,
samples frames roughly every 16 ms during `wait_turn` while streaming, and
encodes MP4s with `ffmpeg`.

## Method 3: `vhs`

Use [charm vhs](https://github.com/charmbracelet/vhs) to record the TUI inside
headless Chrome + `ttyd` with full terminal decorations and fonts.

| Dependency | Check |
|---|---|
| `vhs` | `vhs --version` |
| `ttyd` | `which ttyd` |
| `ffmpeg` | `which ffmpeg` |
| Chrome | `ls "/Applications/Google Chrome.app"` |

Start the daemon from [Method 2](#method-2-mew-tui-capture---connect), then run
`vhs demo.tape`:

```txt
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

Quote file paths (`Output "demo.mp4"`), place `Set` commands (except
`TypingSpeed`) at the top of the tape, and leave generous `Sleep` delays before
screenshots.

## Interactive puppet mode

`mew tui-capture -i` and `mew tui-capture -i --connect <url>` read verbs from
stdin and print a `--- frame ---` block (plus PNG path when `--screenshot-dir` is
set) after every verb:

```sh
printf '%s\n' 'send "hi"' 'wait_turn 30000' 'snapshot response' 'quit' |
  mew tui-capture -i --connect ws://127.0.0.1:19847 \
    --screenshot-dir /tmp/frames
```

## Choosing a method

- **Tests and fast iteration**: use `mew tui-capture` (Method 1) for
  deterministic, zero-setup output.
- **Headless live turns**: use `mew tui-capture --connect` (Method 2).
- **Windowed terminal video**: use `vhs` (Method 3).

## Troubleshooting

- **`pause <ms>` and framerate**: at 30 fps, `pause 1000` appends 30 identical
  frames.
- **Snapshots vs. screenshots**: `snapshot` writes text and `screenshot` writes
  a PNG; capture both when debugging layout or glyphs.
- **Daemon timing**: always follow `send` with `wait_turn` and an adequate
  timeout.
- **Corrupted `state.toml`**: unrecognized `last_provider` or `last_model`
  values can block startup; clear or update those keys in `state.toml`.
