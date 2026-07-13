---
title: Recording real-provider TUI captures
description: How to use mew tui-capture --connect to record videos and screenshots against a live daemon with a real model.
---

# Recording real-provider TUI captures

Most `mew tui-capture` usage is deterministic: a harness script drives a headless
`TestBackend` with canned `FakeProvider` responses. That is great for regression
tests and fast iteration, but it does not exercise the real chat/turn pipeline.

The `--connect <url>` mode connects `mew tui-capture` to a running `mew-daemon`
so the recording uses a real provider: real streaming, real tool calls, real
subagents, and the real session rail. The output is still headless (rasterized to
PNG/MP4), so it is scriptable and CI-friendly.

> This is **real model + coarsely real-time**, not a real terminal recording.
> Tokens arrive from the real provider and are captured roughly every 100 ms
> during streaming, and `pause` duplicates frames. For full terminal chrome,
> fonts, and wall-clock real-time, use [charm vhs](https://github.com/charmbracelet/vhs).

## When to use this

| Goal | Use |
|------|-----|
| Deterministic regression tests, no network | `mew tui-capture --script` |
| Real model behavior for docs/demos | `mew tui-capture --script --connect` |
| Full terminal chrome (fonts, window decorations) | [charm vhs](https://github.com/charmbracelet/vhs) |

## Prerequisites

- A built `mew` binary: `cargo build -p mew`
- `ffmpeg` on PATH (only for `--mp4` output)
- A provider credential configured. For example, with `umans`:
  - Set `MEW_CRED_UMANS` in your environment or keyring.
  - Or load it from a `.env` file: `export $(grep MEW_CRED_UMANS .env | xargs)`.

## Start a daemon for capture

Use `--port 127.0.0.1:0` to bind an ephemeral TCP port. The log line will show
the actual port.

```bash
export MEW_CRED_UMANS=<key>
./target/debug/mew daemon \
  --provider umans \
  --model umans/umans-coder \
  --port 127.0.0.1:0 \
  --background \
  --log /tmp/mew-capture.log

# Find the port
grep "mew daemon listening (tcp)" /tmp/mew-capture.log
# -> addr=127.0.0.1:61982
```

Stop the daemon when you are done:

```bash
./target/debug/mew daemon --stop
```

If `--background` detached the process and `--stop` cannot find the PID, find it
with `ps aux | grep mew` and `kill <pid>`.

## Write a capture script

Daemon-mode scripts use the same one-verb-per-line format as harness scripts,
but support a few extra verbs:

| Verb | Description |
|------|-------------|
| `send "<text>"` | Type and submit a user message to the daemon |
| `wait_turn [timeout_ms]` | Block until the response stream finishes (default 30s) |
| `expect "<text>"` | Fail if the current text frame does not contain `<text>` |
| `pause <ms>` | Duplicate the last frame for `<ms>` milliseconds |
| `screenshot <path>` | Save the current frame as a PNG |
| `screenshot_dir "<path>"` | Save a numbered PNG after every subsequent verb |

`pause` is useful for making the final response readable: after `wait_turn`
returns, the response appears instantly in the recording, so add `pause 4000`
to hold the last frame for a few seconds.

Example `capture.txt`:

```text
send "What's the optimised way of reversing a binary string in Rust?"
wait_turn 120000
expect "reverse"
pause 4000
screenshot /tmp/response.png
```

## Run the capture

```bash
./target/debug/mew tui-capture \
  --script capture.txt \
  --connect ws://127.0.0.1:61982 \
  --mp4 demo.mp4 \
  --width 100 \
  --height 30
```

Outputs:

- `demo.mp4` — MP4 video of the session, including streaming response frames.
- `/tmp/response.png` — final frame screenshot (if the script requested one).
- Text frames printed to stdout.

## How it works

`crates/mew/src/commands/tui_capture.rs` implements `DaemonBackend`. It:

1. Connects to the daemon with `mew_daemon::DaemonClient::connect(url)`.
2. Creates a session and reads `model`/`provider` from `ServerMessage::SessionReady`.
3. Runs the script verb-by-verb, pumping `AgentEvent`s and `ServerMessage`s into a
   headless `mew_tui::App` backed by `TestBackend`.
4. During `wait_turn`, while `app.streaming` is true, it draws and captures a
   frame roughly every 100 ms so the recorded video shows the response appearing
   progressively.
5. Encodes the captured frames to MP4 with ffmpeg.

## Tips

- **Timing is non-deterministic.** The daemon controls when tokens arrive. Use
  generous `wait_turn` timeouts and `pause` verbs.
- **Model shows in the status bar.** `DaemonBackend` sets `app.status.model` and
  `app.status.provider` from `SessionReady`, so the rendered TUI shows the actual
  backend instead of `mewd/daemon`.
- **Avoid expensive prompts during iteration.** Use a short, reliable prompt
  while tuning the script, then switch to the real demo prompt for the final
  recording.
- **Check the daemon log** if the capture hangs: `/tmp/mew-capture.log`.
- **Harness-only verbs are rejected.** `say`, `error`, `settings`, and
  `settings_config` are not available in daemon mode and will produce a clear
  error.

## Troubleshooting

### `daemon did not send SessionReady within 5s`

The daemon is reachable but did not reply with a session ready message. Check
that the daemon log shows `mew daemon listening (tcp)` and that the URL matches
the bound port.

### `wait_turn timed out after ...`

The response did not finish within the timeout. Either the prompt genuinely
takes longer, or `app.streaming` got stuck. Check the daemon log for provider
errors.

### `state.toml contains unrecognized values`

The daemon startup health-check rejects unknown persisted providers/models.
Either heal the state interactively (`mew` from a terminal) or edit
`~/Library/Application Support/computer.mew.mew/state.toml` (macOS) or the
XDG equivalent on Linux to contain valid values, e.g.:

```toml
last_model = "umans/umans-coder"
last_provider = "umans"
```

### Video encoding fails

Ensure `ffmpeg` is installed and on PATH. `mew tui-capture` shells out to ffmpeg
for MP4 encoding.
