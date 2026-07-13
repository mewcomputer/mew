# mew: tui self-capture plan

Goal: let the agent take screenshots and record videos of mew's own TUI — both as
human-facing artifacts (demo mp4s/gifs, à la the opencode "slackbot" post) and as
agent-facing feedback (images + text dumps it can inspect while iterating on TUI code).
Discord/chat bridge is explicitly out of scope for now; everything here should produce
`Part::File` outputs so a future bridge can upload them unchanged.

## What already exists (verified in repo)

- `crates/mew-tui/src/harness.rs` — headless `Harness` over ratatui `TestBackend`.
  Already has a line-based script format with verbs: `size`, `type`, `key`, `submit`,
  `say`, `error`, `snapshot [label]`. `run_script(script, w, h)` returns plain text
  frames via `buffer_to_string`. No network, no async.
- `crates/mew-provider-fake` — `FakeProvider` with scripted `ProviderEvent`s
  (`text_response` helper exists). Deterministic runs are already possible in-process.
- `crates/mew-message` — `Part::File(FilePart)` exists; captures should be returned
  as File parts on tool results.
- `crates/mew-skills` — skills are markdown with YAML frontmatter (`name`,
  `description`, body), names must match `[a-z0-9]+(-[a-z0-9]+)*`.
- `crates/mew-tools/src/tools/` — one-file-per-tool pattern to follow for any new tool.

## Phase 0 — vhs skill (no code, ~an hour)

Get "mew films itself" working today using [charm vhs](https://github.com/charmbracelet/vhs)
(deps: vhs binary, ttyd, ffmpeg, headless chrome).

1. Write a skill at `.mew/skills/tui-capture/SKILL.md` teaching the agent:
   - tape syntax: `Output demo.mp4` / `Output demo.gif`, `Set FontSize/Width/Height`,
     `Type "..."`, `Enter`, `Sleep 2s`, `Screenshot out.png`.
   - workflow: build mew (`cargo build`), write a `.tape` that launches the built
     binary and exercises the target screen, run `vhs demo.tape`, attach outputs.
   - timing guidance: generous `Sleep`s after actions that trigger async work.
2. Determinism caveat: vhs drives the *real* binary in a real pty, so an inner mew
   session talks to a real provider unless there's a CLI path to run against
   `FakeProvider`. **Open question for implementation: add a `--provider fake`
   (or env var) escape hatch to the mew binary** so tapes aren't racing an LLM.
   Until then, tapes should target screens that don't require provider round-trips
   (settings, pickers, session list) or accept nondeterministic content.

Deliverable: the agent can produce a real mp4/gif/png of the TUI from a prompt.

## Phase 1 — buffer→png rasterizer (the core new code)

New crate `crates/mew-raster` (or a `raster` feature on `mew-tui`) that turns a
ratatui `Buffer` into an image. This is agent-facing: controlled rendering beats
"whatever chrome drew" for VLM legibility.

- Iterate `Buffer` cells: each cell = symbol + fg/bg `Color` + `Modifier`s.
- Glyph rendering: `fontdue` or `ab_glyph` + `tiny-skia` (all pure rust, no system
  deps). Bundle one monospace font with full box-drawing coverage — JetBrains Mono
  or Iosevka Term — via `include_bytes!`.
- Cell metrics: fixed cell width = advance of `M`; render at **2x scale, ≥16px glyph
  height**. VLM legibility on dense terminal text is mostly a resolution problem.
- Style mapping: fg/bg fills, BOLD via the bold font weight (bundle both weights),
  UNDERLINED/REVERSED/DIM as paint effects. Map ratatui `Color::Indexed`/named to a
  palette; support an optional **high-contrast capture theme** override, independent
  of the user theme.
- API sketch:
  ```rust
  pub struct RasterOptions { pub scale: f32, pub theme: CaptureTheme }
  pub fn rasterize(buf: &ratatui::buffer::Buffer, opts: &RasterOptions) -> tiny_skia::Pixmap
  pub fn to_png(buf: &Buffer, opts: &RasterOptions) -> Vec<u8>
  ```
- Harness integration: add a `screenshot <path>` verb to the existing script format
  (keep `snapshot` for text). Every capture should be available in **both encodings
  from the same frame** — text dump for structure (LLMs can count columns), png for
  color/emphasis/gestalt. The pair is the product, not the image alone.
- Tests: golden-image tests are brittle across font rendering changes; prefer
  asserting on pixmap dimensions + sampled cell colors, and keep text-dump goldens
  as the real regression net.

## Phase 2 — video from the harness

- Add a frame-recording mode to `Harness`: after each script verb (or each injected
  event), rasterize a frame into an in-memory sequence.
- Encode by writing numbered pngs to a temp dir and shelling to
  `ffmpeg -framerate N -i frame_%04d.png -pix_fmt yuv420p out.mp4`. Don't bother
  with an in-process encoder; ffmpeg is already a vhs dependency.
- Pacing: script verbs are instantaneous in the harness, so synthesize timing —
  either a `pause <ms>` verb that duplicates frames, or a fixed per-verb frame count.
  Typing can emit one frame per keystroke for a natural feel.
- This gives deterministic, ci-runnable videos (FakeProvider works in-process here,
  unlike vhs). vhs remains the tool for glamour shots with real terminal chrome.

## Phase 3 — expose to the agent

Two options; do the first, consider the second later:

1. **Skill-first** (recommended): add a small binary/subcommand
   `mew tui-capture --script <file> [--png-dir d] [--mp4 out.mp4] [--size 120x35]`
   that runs the harness script and writes artifacts. Extend the Phase 0 skill to
   document it. The agent uses bash; no new tool plumbing.
2. **Dedicated tool**: a `tui_capture` tool in `mew-tools` that takes the script
   inline, runs the harness, and returns results with `Part::File` attachments —
   nicer UX, needed eventually for File parts to flow to future chat bridges, but
   not required for the loop to work.

## Phase 4 — a/b legibility experiment (~30 min, do early)

Render the same frame three ways: (a) vhs screenshot, (b) rasterizer png at 2x
high-contrast, (c) text dump. Feed each to the model with "describe this UI in
detail; report any misalignment or truncation" and compare accuracy. There's a real
possibility VLMs read vhs shots better (closer to training distribution) — this
test settles whether the rasterizer needs tuning and how much weight to put on the
text channel. Cheap enough to run as soon as Phase 1 renders anything.

## Suggested order

Phase 0 → Phase 1 → Phase 4 → Phase 2 → Phase 3. Each is independently demoable.

## Open questions

- CLI/env escape hatch to run the real binary against `FakeProvider` (needed for
  deterministic vhs tapes).
- Whether `say` in the harness script is expressive enough to stage realistic
  sessions (tool calls, reasoning parts, compaction) — may want a `seed <jsonl>`
  verb that loads a prerecorded session file for rich screenshots.
- Capture theme: reuse `mew-tui/src/theme.rs` types or define a separate minimal
  palette in the raster crate to avoid a dependency cycle.
- Font licensing: JetBrains Mono (OFL) and Iosevka (OFL) are both fine to embed;
  pick one and commit it.
