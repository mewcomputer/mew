---
name: tui-capture
description: "Record mew's TUI as mp4, gif, or png. Two paths: (1) mew tui-capture — deterministic, in-process, no external deps beyond ffmpeg; (2) charm vhs — glamour shots with real terminal chrome. For demos, docs, and agent self-inspection of UI state."
---

# TUI Capture

Two capture paths, each with different tradeoffs:

| | `mew tui-capture` | `vhs` |
|---|---|---|
| **Determinism** | Fully deterministic (FakeProvider in-process) | Non-deterministic (real pty, real provider) |
| **Dependencies** | ffmpeg (for video only) | vhs, ttyd, ffmpeg, Chrome |
| **Terminal chrome** | None — raw rasterized buffer | Full — window decorations, fonts, themes |
| **Network** | Never | Required (LLM or fake-provider daemon) |
| **CI-runnable** | Yes | No (needs Chrome + display) |
| **Best for** | Agent self-inspection, regression tests, fast iteration | Demo videos, README gifs, social media |

## Method 1: `mew tui-capture` (deterministic, recommended)

Runs a harness script against a headless `TestBackend` with `FakeProvider`.
No network, no credentials, no daemon — the output is byte-for-byte
deterministic across runs.

### Prerequisites

- `cargo build -p mew` (produces `./target/debug/mew`)
- `ffmpeg` on PATH (only needed for `--mp4` video output)

### Usage

```bash
# Screenshot: produce a PNG from a script
mew tui-capture --script capture.txt --width 80 --height 24

# Video: auto-wraps the script in recording and produces an MP4
mew tui-capture --script capture.txt --mp4 demo.mp4 --fps 30

# Custom terminal size
mew tui-capture --script capture.txt --width 120 --height 35 --mp4 demo.mp4

# Interactive puppet mode: agent drives the TUI via stdin
echo -e "key ctrl+p\nkey down\nkey esc" | mew tui-capture -i
```

### Interactive mode (`--interactive` / `-i`)

Reads verbs from stdin one line at a time against a persistent harness.
After each verb, prints:
1. A `--- frame ---` text block showing the current TUI state
2. If `--screenshot-dir` is set, a `--- screenshot: <path> ---` line with
   the path to a PNG of the current frame

This enables an agent-driven feedback loop — the agent sends an action,
reads the text frame from stdout, and can inspect the PNG for visual details
(colors, layout, cursor position):

```bash
# Puppet the TUI with screenshots for visual feedback
echo -e "key ctrl+p\nkey down\nkey esc" | \
  mew tui-capture -i --screenshot-dir tmp/frames

# Also record a video of the session
echo -e "key ctrl+p\nkey down\nkey esc" | \
  mew tui-capture -i --screenshot-dir tmp/frames --mp4 tmp/demo.mp4
```

The agent can:
1. Send a key (`key ctrl+p`, `key down`, `key enter`, `key esc`)
2. Read the text frame from stdout for structure
3. Read the screenshot path and inspect the PNG for visual state
4. Decide what to do next based on what's on screen
5. On `quit`/EOF, if `--mp4` was set, a video is encoded from all frames

### Script verbs

The script file uses the harness format — one verb per line, `#` for comments:

| Verb | Description |
|---|---|
| `type <text>` | Type text into the composer (one frame per keystroke when recording) |
| `key <name>` | Send a key: `enter`, `esc`, `tab`, `backspace`, `up`/`down`/`left`/`right`, `ctrl+c`, `alt+x` |
| `submit` | Shorthand for `key enter` |
| `say <text>` | Inject an assistant text turn (streams in chunks, one frame per chunk when recording) |
| `error <text>` | Inject a terminal error event |
| `snapshot [label]` | Render current frame as text to stdout |
| `screenshot <path>` | Save current frame as a PNG file |
| `start_recording` | Begin capturing frames after each verb |
| `stop_recording` | Stop capturing; reports frame count |
| `pause <ms>` | Duplicate the last frame to simulate elapsed time (at 30fps) |
| `record <path> [fps]` | Encode recorded frames to MP4 via ffmpeg |
| `size <w> <h>` | Resize the virtual terminal |

### Example script

```
# Type a prompt and capture the response
type hello world
submit
say Welcome to mew! This is a deterministic capture.
pause 500
snapshot after-response
screenshot output.png
```

### Example: video with `--mp4`

When `--mp4` is passed, the script is automatically wrapped in
`start_recording` / `stop_recording` / `record` — you don't need to add
those verbs yourself. Just write the interaction:

```
# This script + --mp4 demo.mp4 produces a video
type what can you do?
submit
say I can capture the TUI deterministically! No network needed.
pause 1000
```

```bash
mew tui-capture --script demo.txt --mp4 demo.mp4 --width 80 --height 24
```

### Full procedure

```bash
# 1. Build mew
cargo build -p mew

# 2. Write a script file (see examples above)

# 3. Run the capture
./target/debug/mew tui-capture --script capture.txt --mp4 demo.mp4

# 4. Verify outputs
ls -la demo.mp4 *.png
```

### Common pitfalls

- **`say` uses FakeProvider** — responses are canned text, not real LLM output.
  This is by design for determinism. Use vhs if you need real provider content.
- **`pause <ms>` duplicates frames** — at 30fps, `pause 1000` adds 30 identical
  frames. Use this to add natural timing between actions.
- **Screenshots and text snapshots are independent** — `screenshot` writes a PNG
  file; `snapshot` prints text to stdout. Use both for the same frame to get
  a text dump (for LLM structure analysis) + image (for visual inspection).
- **ffmpeg must be on PATH** for `--mp4` or `record` to work. Install with
  `brew install ffmpeg` if missing.

---

## Method 2: charm vhs (glamour shots)

Use [charm vhs](https://github.com/charmbracelet/vhs) to record mew's
terminal UI as video (mp4/webm), animated gif, or static png screenshots.
VHS drives a real terminal via headless Chrome + ttyd, so captures include
full terminal chrome, fonts, and colors — exactly what a user sees.

### Prerequisites

VHS needs these on the system. Check before recording:

| Dependency | How to verify | Install |
|---|---|---|
| `vhs` | `vhs --version` | `brew install vhs` |
| `ttyd` | `which ttyd` | `brew install ttyd` |
| `ffmpeg` | `which ffmpeg` | `brew install ffmpeg` |
| Chrome | `ls "/Applications/Google Chrome.app"` | `brew install --cask google-chrome` |

If any are missing, tell the user what to install before proceeding.

### Core Workflow

1. **Build mew** — `cargo build -p mew`
2. **Start a fake-provider daemon** — for deterministic, offline captures
3. **Write and run a `.tape` file** — vhs drives the terminal

### Starting the daemon

`mew chat` has no `--fake-provider` flag. Instead, start a daemon with the
fake provider and connect to it:

```bash
mew daemon --fake-provider --port 127.0.0.1:9847 --background --log /tmp/mew-capture.log
sleep 2 && cat /tmp/mew-capture.log | grep "listening"
```

The daemon stays in the background. Capture scripts connect to it via
`mew chat --connect ws://127.0.0.1:9847`.

**Always clean up when done:**

```bash
mew daemon --stop
```

### Tape Syntax Reference

A `.tape` file is a line-based script. Comments start with `#`. All `Set`
commands (except `TypingSpeed`) must appear at the top of the file, before
any other directives.

#### Output — specify what to produce

```
Output "demo.mp4"     # video (H.264)
Output "demo.gif"     # animated gif
Output "demo.webm"    # webm video
```

Multiple `Output` lines produce all formats from one recording. **Paths must be
quoted.**

To capture a single static screenshot, use `Screenshot` instead of `Output` —
`Output` with a `.png` path creates a directory of numbered frames, not a
single image.

#### Screenshot — capture one frame as PNG

```
Screenshot "welcome.png"
```

#### Set — terminal appearance and behavior

```
Set FontSize 16              # font size in pixels (default 22)
Set Width 1200               # terminal width in pixels
Set Height 600               # terminal height in pixels
Set Padding 20               # padding around terminal content
Set FontFamily "JetBrains Mono"  # font family
Set Theme "Catppuccin Mocha"     # theme name (run `vhs themes` to list)
Set TypingSpeed 30ms          # delay per keystroke (default 50ms)
Set Framerate 60              # recording framerate (default 50)
Set PlaybackSpeed 1.0         # playback speed multiplier
Set CursorBlink false         # disable cursor blink
Set Margin 60                # margin in pixels
Set MarginFill "#1e1e2e"      # margin color (hex) or image file
Set BorderRadius 10           # rounded corners
Set LetterSpacing 1.0         # letter spacing (default 1.0)
Set LineHeight 1.0            # line height (default 1.0)
```

#### Type — type text into the terminal

```
Type "echo hello"           # type a string
Type@100ms "slow typing"     # type slowly (per-char delay override)
```

Quote text with backticks if it contains both single and double quotes:

```
Type `VAR="It's quoted"`
```

#### Key commands — send keystrokes

```
Enter                    # press Enter
Enter 3                  # press Enter 3 times
Enter@500ms              # press Enter with 500ms hold
Tab                      # press Tab
Backspace                # press Backspace
Escape                   # press Escape
Space                    # press Space
Up / Down / Left / Right # arrow keys
Ctrl+C                   # send Ctrl+C
Ctrl+D                   # send Ctrl+D
Alt+x                    # send Alt+X
```

All key commands accept an optional `@<time>` suffix and an optional repeat count.

#### Sleep — wait for a duration

```
Sleep 3s         # 3 seconds
Sleep 500ms      # 500 milliseconds
Sleep 2          # 2 seconds (bare numbers = seconds)
```

**Timing is critical.** After launching mew, after submitting a prompt, or
after any action that triggers async work, add a generous `Sleep`. Too short
and you capture an incomplete render. 2–4 seconds is usually right; use more
for streaming text.

#### Hide / Show — control capture

```
Hide                     # stop recording (setup happens off-camera)
Show                     # resume recording
```

#### Wait — wait for screen content

```
Wait /pattern/           # wait for regex on last line (default pattern: />$/)
Wait+Screen /pattern/    # wait for regex anywhere on screen
Wait@10s /pattern/       # custom timeout (default 15s)
```

#### Env — set environment variables

```
Env MEW_DANGEROUS "1"    # skip all permission prompts
Env MEW_PERMISSIVE "1"   # auto-allow mutating tools
```

### Recommended Tape Settings for mew

For clear, legible captures:

```
Set FontSize 16
Set Width 1200
Set Height 600
Set Padding 20
Set TypingSpeed 30ms
```

For demo-quality glamour shots:

```
Set FontSize 22
Set Width 1400
Set Height 700
Set Padding 40
Set Margin 40
Set MarginFill "#1e1e2e"
Set BorderRadius 12
Set TypingSpeed 50ms
```

### Full Example Tape

```
# mew welcome + prompt demo
Output "demo.mp4"
Set FontSize 16
Set Width 1200
Set Height 600
Set Padding 20
Set TypingSpeed 30ms

# Launch mew connected to a fake-provider daemon
Type "./target/debug/mew chat --connect ws://127.0.0.1:9847"
Enter
Sleep 3s

# Capture the welcome screen
Screenshot "welcome.png"

# Type a prompt and wait for the fake response
Type "hello, what can you do?"
Sleep 3s

# Capture the response
Screenshot "response.png"

# Quit
Ctrl+C
Sleep 1s
```

### Complete vhs Procedure

```bash
# 1. Build mew
cargo build -p mew

# 2. Start fake-provider daemon
./target/debug/mew daemon --fake-provider --port 127.0.0.1:9847 --background --log /tmp/mew-capture.log
sleep 2

# 3. Verify daemon is listening
grep "listening" /tmp/mew-capture.log

# 4. Write the .tape file (see examples above)

# 5. Run vhs
vhs capture.tape

# 6. Verify outputs exist
ls -la *.mp4 *.gif *.png

# 7. Stop the daemon
./target/debug/mew daemon --stop
```

### vhs Common Pitfalls

- **Paths must be quoted** in tape files: `Output "demo.mp4"`, not `Output demo.mp4`.
- **`Output "*.png"` creates a frame directory**, not a single image. Use
  `Screenshot "file.png"` for a single static capture.
- **Too-short Sleeps** are the #1 cause of blank or partial captures. After
  launching mew, after submitting a prompt, and before a screenshot, add
  `Sleep 2s` minimum.
- **`Set` commands must be at the top** of the tape file, before `Type`, `Enter`,
  `Output`, or any other directive. `Set TypingSpeed` is the only exception.
- **mew's state.toml** can get corrupted if a previous session wrote invalid
  provider/model names. If the daemon fails to start with a "state.toml
  contains unrecognized values" error, heal it by clearing `last_provider`
  and `last_model` from the state file (at
  `~/Library/Application Support/computer.mew.mew/state.toml` on macOS).

## Choosing Output Format

| Format | When to use |
|---|---|
| `.png` (via `Screenshot` or `screenshot`) | Static docs, README images, UI inspection. Single frame. |
| `.mp4` | Video demos, embedding in docs/sites. Smaller than gif for long recordings. |
| `.gif` | Short loops for READMEs, social media. Large files; keep under ~10 seconds. |
| `.webm` | Web embedding where you want smaller files than mp4. |

## Where to place finished files

Place all finished files in ./notes/capture (relative to project root)
