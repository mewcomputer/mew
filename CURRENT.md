# 2026-09-28 — Responses: surface raw reasoning text and non-lite summaries

mew only created a reasoning part for Responses Lite models, so a non-lite
reasoning item's `reasoning_summary_text` deltas found no part to attach to and
were dropped. The part is now created on `output_item.added` for every model,
and `response.reasoning_text.delta`/`.done` (the model's raw `content`, as
opposed to its `summary`) route to the same handlers as the summary deltas.
`StreamState.use_responses_lite` became write-only and was removed.

Whichever of summary/content arrives is shown; mew's message model has a single
`text` field, so both arriving would concatenate. (Codex has separate
`summary`/`content` and shows the summary unless `show_raw_agent_reasoning` is
set, but its SSE processor does handle both event families.)

Verified `cargo test --all` (101 suites), `cargo clippy --all -- -D warnings`,
and `cargo fmt --all --check`.

# 2026-09-28 — TUI: hide encrypted traces, label them as traces

Responses Lite reasoning arrives as opaque `encrypted_content` with no readable
summary (the replayed item carried `"summary": []`), so the TUI rendered a
"thinking · 0 lines" header that expanded to nothing. Reasoning parts with no
text are now skipped entirely; a trace that *does* carry a readable summary is
still rendered in full. The collapsed header count is relabelled from
"N lines" to "1 trace"/"N traces".

Verified the four new render tests plus `cargo test --all` (101 suites),
`cargo clippy --all -- -D warnings`, and `cargo fmt --all --check`. The five
golden frames are unchanged: the reasoning golden renders the expanded header,
which carries no count.

# 2026-09-28 — ratatui-mdstream: panic on unclosed bold/strikethrough

`parse_inline` scanned for `**`/`__`/`~~` with `while end + 1 < bytes.len()`,
which stops at `end == len - 1`. When the closing marker was absent and the
string ended in a multi-byte character, `&text[start..end]` panicked with
"byte index N is not a char boundary" (hit mid-stream: model output ending in
an em-dash, CJK, or emoji right after an unclosed marker). Even when it did not
panic it dropped the final byte of the text.

Both scans now use a shared `find_marker` helper that returns the marker's
index when present and `None` otherwise; an unclosed marker styles the
remainder of the string, matching how the italic branch already behaved.
Regression tests cover unclosed bold with ASCII, em-dash, CJK, and emoji
tails, plus unclosed strikethrough.

Verified `cargo test --all` (101 suites), `cargo clippy --all -- -D warnings`,
and `cargo fmt --all --check`.

# 2026-09-28 — Codex/Responses: replay reasoning under its original item id

Replaying a reasoning item with `encrypted_content` failed with "Encrypted
content item_id did not match the target item id": the wire builder fabricated
a fresh `rs_<ulid>` for the item `id`, but the API binds the encrypted blob to
the id it issued it under. `ReasoningPart` gains `provider_item_id`, populated
from `response.output_item.added` during streaming and echoed verbatim by the
request builder. When it is absent (parts persisted before this change, or
providers without an item id) the `id` field is omitted rather than invented,
matching Codex, whose `ResponseItem::Reasoning.id` is optional.

Verified `cargo test --all` (101 suites), `cargo clippy --all -- -D warnings`,
and `cargo fmt --all --check`. Not verified against the live Codex backend.

# 2026-09-28 — split mew-provider-responses into modules

`lib.rs` had grown to 2,967 lines, over half of it tests. Split into:

- `lib.rs` (119): crate docs, `Adapter` + `AdapterAuth`, constructors,
  `build_auth_headers`
- `provider.rs` (338): `impl Provider` — retry/stream loop and `list_models`,
  with its tests
- `request.rs` (1,208): request-body and wire-message construction, with its
  tests
- `stream.rs` (1,245): SSE `StreamState`/`ToolCallAccumulator`, the `handle_*`
  event handlers, and part helpers, with its tests
- `test_support.rs` (101, `cfg(test)`): shared `make_adapter`, `make_message`,
  `CodexCacheRestore`, `TestOAuthProvider`

`build_auth_headers`, `build_request_body`, and `read_stream` became
`pub(crate)` so cross-module callers resolve; the public surface (`Adapter`,
`oauth`) is unchanged. Verified `cargo test -p mew-provider-responses` (53
tests), clippy `-D warnings`, fmt, `cargo build -p mew`, and `just arch-check`.

# 2026-09-28 — Codex/Responses 400: reasoning input items need `summary`

`mew-provider-responses` omitted the `summary` field when re-sending a reasoning
item with `encrypted_content`, so the Responses API rejected the request with
`Missing required parameter: 'inputN.summary'`. In the vendored Codex protocol
(`sourc/codex/codex-rs/protocol/src/models.rs`), `summary: Vec<ReasoningItemReasoningSummary>`
has no `skip_serializing_if`, so it is always emitted and the API treats it as
required. The builder now always emits `summary`, carrying the captured summary
text (`ReasoningPart.text`) as `[{"type":"summary_text","text":...}]` when
non-empty and `[]` otherwise; `encrypted_content` still carries the reasoning.

Verified `cargo test -p mew-provider-responses` (53 tests), `cargo clippy
-p mew-provider-responses --all-targets -D warnings`, and `cargo fmt --check`.
Not verified against the live Codex backend (no credentials here).

# 2026-09-28 — TUI Enter accepts the highlighted slash command

The TUI composer submitted the raw slash prefix on Enter, so a partial name like
`/mode` fell through to the model as an unknown command instead of selecting the
highlighted `/model`. Commit `0524c7e` had dropped Enter-completes in favor of
Tab-only completion, but its stated rationale (losing arguments such as
`/goal fix the bug` → `/goal`) never applied: the autocomplete only stays open
for a bare command-name prefix, and any argument introduces a space that closes
it. Enter now runs the highlighted entry in one press when the menu is open and the
input does not already name a command, matching the desktop composer's intent.
A query that names a command (or carries arguments) still submits as typed.

Verified the new regression tests, the full `mew-tui` suite, `cargo clippy
-p mew-tui --all-targets -D warnings`, and `cargo fmt --check`.

# 2026-09-27 — fix where the global AGENTS.md is read from

`mew-context` resolved the global context directory with
`directories::ProjectDirs`, which on macOS is
`~/Library/Application Support/computer.mew.mew`. The rest of mew (and
`mew-config/src/paths.rs`, which says all crates should share its helpers) uses
XDG `~/.config/mew`. So the documented `~/.config/mew/AGENTS.md` was silently
ignored on macOS, `MEW_CONFIG_DIR` had no effect on context, and the
`~/.claude/CLAUDE.md` fallback masked it. Found while reviewing `mew debug
context` output.

`mew-context::config_dir` now mirrors `mew_config::config_dir`: `MEW_CONFIG_DIR`
override, else `$XDG_CONFIG_HOME/mew` (or `~/.config/mew`) on Unix, else the
platform strategy. Dropped the now-unused `directories` dependency and added
`etcetera`.

Resolution and global loading were split into pure helpers (`config_dir_with`,
`default_config_dir`, `load_global(config_dir, home)`) so they can be tested
without mutating process env. That mattered: an earlier env-mutating test proved
flaky because it made a global `AGENTS.md` appear mid-run, and several existing
loader tests matched `find(|f| f.path.ends_with("AGENTS.md"))`, which then
resolved to the global file. Those tests are now scoped to their temp dir —
required anyway, since a developer who creates `~/.config/mew/AGENTS.md` would
otherwise hit the same failure. Tests cover the override, the XDG default, the
global-file precedence and `.claude` fallback; verified end-to-end that
`MEW_CONFIG_DIR=<dir> mew debug context` lists `<dir>/AGENTS.md` and that the
default case is unchanged where `~/.config/mew/AGENTS.md` does not exist.
`cargo test -p mew-context` (30, stable across 12 runs) passes with and without
a global `AGENTS.md`; `-p mew-prompts` (61), `-p mew` (152),
`cargo clippy --all -- -D warnings`, `cargo fmt`, `just arch-check`, and
`just deps` are clean.

# 2026-09-27 — add `mew debug context` to inspect the assembled context

`mew debug context` prints a tree of every source feeding the system prompt:
filesystem context files (global `AGENTS.md`/`CLAUDE.md`, then per-directory
`AGENTS.md`/`CLAUDE.md`, `.mew/AGENTS.md`, `.mew/wiki.md`) plus the built-in VFS
resources (`mew://...`) inlined by templates, grouped under a `mew://` node.
`--full` prints the fully assembled system prompt — base scaffold, context
blocks, and skills listing — via a shared snapshot helper that builds a real
session agent with a `FakeProvider`, so no credentials or network are needed.

Template resolution is real, not scanned: `mew-prompts` gained a thread-local
`record_transclusions` recorder that the `transclude` function feeds, so the VFS
list reflects only the branches actually taken (a single provider-specific
`base_*` variant) and follows nested transclusions. `build_system_prompt_snapshot`
returns `(prompt, vfs_resources)`, so the tree and `--full` are two views of the
same resolve.

Making `build_session_agent` take an injected provider (instead of building one
itself) let the debug path reuse the exact production wiring without a provider.
Also routed non-TUI tracing to stderr so `--full` and other command stdout stay
clean for piping; the daemon already did this, the CLI path was using the
default stdout writer. Removed a duplicate `#[test]` attribute in
`mew-prompts::template` that clippy rejected under `-D warnings`.

Tests cover tree grouping, chain collapsing, `~` abbreviation, the VFS subtree,
combined files+VFS, the empty case, transclusion recording (taken branch,
dedupe, missing paths), and an end-to-end snapshot assertion. `cargo test -p mew`
(152) and `-p mew-prompts` (61), `cargo clippy --all -- -D warnings`, `cargo fmt`,
and `just arch-check` are clean; both `mew debug context` and `--full` were
exercised against this repo.

# 2026-09-12 — make desktop persona and model pickers usable

Persona options now size to their wrapped content inside a bounded scroll area,
so longer descriptions no longer clip the next option. The model picker adds a
focused, IME-capable search field with case-insensitive matching across model
IDs, providers, names, and descriptions, plus clear and empty-result states;
choosing a model restores composer focus instead of leaving the hidden search
field active.
Picker wheel events stop at the overlay, keeping the conversation from scrolling
behind it while the model list remains virtualized. Coverage includes model
matching and updated picker sizing; the rebuilt debug bundle was exercised with
persona selection, model filtering, clearing, empty search, and model-list wheel
scrolling. `cargo test -p mew-desktop` (107), clippy, fmt, `just arch-check`, and
`git diff --check` are clean.

# 2026-09-12 — stabilize transcript prepends and add conversation scrolling

Desktop history loading now splices newly fetched rows into the existing GPUI
list instead of resetting it, shifts saved transcript anchors with prepended
messages, and preserves the same visible message while older pages arrive. The
conversation view also has a native-looking, accessible scrollbar backed by
`ListState`, with track navigation, thumb dragging, bottom-follow restoration,
and live updates during wheel scrolling. Coverage includes the prepend-row
invariant and the full desktop test suite. `cargo test -p mew-desktop`, clippy,
`just arch-check`, formatting, and `just desktop-dev` all pass; the rebuilt
bundle was exercised visually with wheel scrolling, track clicks, and thumb
dragging.

# 2026-09-12 — simplify the desktop composer controls

Refreshed the GPUI composer around one compact model-and-effort control. The
attachment action is now a quiet plus button, permission and persona remain
available without competing with send, and the separate thinking trigger is
gone. The model popover keeps search and virtualized results, adds an effort
track with named snap points, and returns focus to the composer after a
selection. Verified the desktop test suite, clippy, arch-check, formatting and
diff checks, then used `just desktop-dev` to exercise model search, effort
selection, focus return, and isolated picker scrolling over a populated
transcript.

# 2026-09-12 — make effort and picker positioning direct

Made the effort control genuinely draggable with bounded pointer-to-stop
mapping and release handling, while retaining keyboard and click access. Model
rows now use smaller, single-line ellipsized text so long provider names cannot
overlap descriptions or neighboring rows. The composer keeps the model trigger
right-aligned, and the picker itself anchors by its right edge so changing the
selected effort does not move the overlay. Verified drag changes from high to
max and back to off in `just desktop-dev`, with the popup staying fixed.

# 2026-09-12 — give the effort track a direct, themed motion pass

Reworked the effort control to mirror the Codex treatment: a themed accent
progress path, small accent stop markers, a white themed thumb with an accent
center, and shared endpoints between the track and labels. Dragging now follows
the pointer continuously, then eases the thumb and fill into the nearest named
stop on release. The animation uses GPUI's reduced-motion-aware animation
wrapper and all colors resolve through the active theme. Verified the rebuilt
control visually by dragging high → max and max → off in `just desktop-dev`.

# 2026-09-12 — keep outside clicks out of the effort drag

The effort track now ignores mouse-up events unless a pointer drag is active.
This keeps the optimistic selected stop separate from the live drag position,
so clicking elsewhere after a completed drag cannot move the slider. Added a
focused regression assertion and verified the desktop tests, clippy, formatting,
and diff checks.

# 2026-09-12 — align multi-stop effort geometry

Widened the model picker and derived the effort track inset from the number of
available stops, keeping the line, thumb, markers, and labels on one coordinate
system as thinking levels grow beyond four. Added geometry assertions for the
seven-stop case. Verified the full desktop tests, clippy, formatting, diff
checks, and the rebuilt picker with a real drag plus an outside-click guard.

# 2026-09-12 — keep effort options accessible after model switches

Fixed a GPUI accessibility-tree panic when reopening models with a catalog
provided `off` thinking variant. The picker already adds its own `Off` stop, so
both entries previously generated the same accessibility id; effort options
now use stable index-based ids. Added a regression test covering the duplicate
case. Verified all desktop tests, clippy, formatting, diff checks, and a visual
select/reopen pass for qwen3.8-max-preview with the effort picker open.

# 2026-09-12 — center the effort thumb marker

Centered the accent marker inside the effort thumb so it no longer renders as
an offset dot at the thumb's top-left edge. Verified the full desktop tests,
clippy, formatting, diff checks, and a fresh packaged visual pass with
qwen3.8-max-preview selected.

# 2026-09-12 — define separate desktop distribution

Captured the packaging direction in
`notes/mew-desktop-distribution-plan.md`: CLI/TUI artifacts stay independent,
the native client gets architecture-specific macOS DMGs and archives with its
daemon and CEF runtime bundled, and signing/notarization plus auto-updates are
deferred until Apple developer credentials are available.

# 2026-09-12 — package unsigned native desktop releases

Added `just desktop-package` and a macOS release wrapper that stamps the
workspace version into the app and helper plists, stages the bundled daemon,
CEF runtime, and generated notices, emits architecture-qualified DMG and ZIP
artifacts, and writes SHA256 checksums. `mew --version` now exposes the semver
release identity while daemon diagnostics retain the git revision. The tag
workflow publishes desktop assets separately from CLI/TUI tarballs, with
signing and notarization omitted.
Verified the arm64 release build, DMG mount/integrity, archive contents,
checksums, packaged app launch, model selection, focused Rust tests, clippy,
formatting, architecture checks, and shell syntax.

# 2026-09-12 — align effort stop markers and labels

Positioned each effort marker and its hit target from the same inset stop
offset, then used centered label slots on that geometry instead of flex
centers. The track, thumb, dots, labels, click targets, and drag mapping now
share one measured coordinate system at every stop count. Added regression
assertions for stop offsets and label slot widths.
Verified the focused and full desktop tests, clippy, formatting, diff checks,
and a rebuilt `just desktop-dev` visual pass with direct selection and thumb
dragging across the deepseek-v4-pro Off/high/max picker.

# 2026-09-12 — remove duplicate qwen Off variant

Filtered catalog thinking variants that spell `off` (including padded or
capitalized values) before the desktop picker adds its own `Off` stop. Qwen
models now expose Off/low/medium/xhigh without a trailing duplicate, while
the provider catalog still retains its explicit backend off parameters. Added
a regression test for padded metadata. Verified 114 desktop tests, clippy,
formatting, diff checks, and architecture checks. The rebuilt `just
desktop-dev` launch hit the known macOS hiservices signal before the final
process could be attached for an additional visual pass.

# 2026-09-12 — reduce high-frequency scroll invalidation

Stopped the transcript scroll handler from enqueueing a second full-shell
notification on every wheel or trackpad tick; `ListState` already invalidates
the containing view for its own virtualized repaint. Session-row hover controls
now use GPUI group-hover styling, avoiding shell state updates as the pointer
crosses rows. Sidebar rows also defer cloning the full workspace-group list
until a session menu is actually open. Verified 114 desktop tests, clippy,
formatting, diff checks, architecture checks, and rapid bidirectional scrolling
in the rebuilt packaged app with the transcript scrollbar tracking correctly.

# 2026-09-12 — measure and trim desktop frame work

Added an opt-in GPUI frame probe behind `MEW_DESKTOP_FRAME_TRACE=1`. It reports
60-frame draw average/p95/max, dirty-to-draw p95, invalidation counts, and
writes the same batches to the system temp directory as
`mew-desktop-frame-trace.log`. A baseline long-transcript sample measured about
25 ms average draw time and 27–28 ms p95, making render cost rather than
duplicate scroll notifications the next target. The browser pump now ignores
duplicate address/title events so CEF metadata chatter cannot invalidate the
whole shell repeatedly. The transcript registry is also cleared at the empty
state boundary instead of on every non-empty center render.
Verified 116 desktop tests, the full Rust workspace suite (including CLI,
daemon, bridge, and integration tests), clippy, formatting, diff checks, and
architecture checks. The final `just desktop-dev` visual launch still hit the
known macOS hiservices signal-6 harness failure after prior packaged visual
scroll checks remained stable.

# 2026-09-12 — skip idle transcript selection bookkeeping

Transcript inline spans now populate the selection registry only while a
selection exists. Ordinary scrolling no longer clones and stores every visible
span's text layout just to maintain an unused selection index. Verified 116
desktop tests, clippy, formatting, and architecture checks.

# 2026-09-12 — trim plain transcript span setup

Plain markdown spans now skip empty highlight and code-font override setup
before GPUI receives the text element. This keeps the hot path focused on the
actual text layout work while preserving styled spans and selection behavior.
Verified 116 desktop tests, clippy, formatting, diff checks, and architecture
checks.

# 2026-09-11 — plan GPUI workspace-first sidebar cleanup

Added `notes/gpui-workspace-sidebar-cleanup-plan.md`, an execution-ready plan
for replacing the native desktop's session-group hierarchy with canonical
daemon-owned workspace membership. The plan preserves existing group data,
specifies workspace and task pinning, five-item progressive disclosure,
relevance-first global search, stable anchored menus, local editor handoff,
test-first implementation phases, visual QA states, and acceptance criteria.
It builds on the project discovery and shared reducer work that already landed
instead of duplicating it. Verified with `git diff --check`.

# 2026-09-11 — implement workspace-first GPUI sidebar projection

Added canonical daemon workspace identity resolution with longest-root matching,
mirrored it into `SessionInfo` and `ConversationItem`, and exposed project
metadata in `UiModel`. The native sidebar now renders workspace headers and
groups tasks by workspace, keeps pinned/attention/running/selected tasks
visible, caps normal recent tasks at five with `Show more`, expands search
across archived matches, and supports workspace-scoped new-task actions. The
relevant focused tests pass, `cargo check -p mew-desktop` passes, and
`just desktop-dev` was used for a live screenshot plus collapse, show-more,
search, and new-task interaction smoke pass.

# 2026-09-11 — persist and sync workspace pins

Added the daemon-owned `projects.json` sidecar and `PinProject` protocol
mutation. Workspace pins are validated against the canonical project list,
written atomically, and rebroadcast as `ProjectList` so connected clients
converge. The GPUI workspace header now exposes an accessible pin/unpin control.
Verified protocol roundtrip, sidecar persistence, formatting, diff checks,
daemon/desktop clippy, and a live `just desktop-dev` smoke pass that pinned,
observed reordering, and restored the test state.

# 2026-09-11 — add local workspace open destinations

Added a persisted desktop external-editor preference and a workspace split
control with a remembered primary action plus a separate chevron menu. The
menu detects available local destinations, groups editor choices separately
from actions, supports Terminal and copy-path actions, and refuses local-path
launches for remote daemon profiles. Launch arguments remain structured so
workspace paths with spaces are preserved. Verified config and desktop focused
tests, desktop check and clippy, and a live `just desktop-dev` smoke pass that
restored Zed, opened the destination menu, selected the editor, and copied a
workspace path.

# 2026-09-11 — consolidate workspace actions into a kebab menu

Simplified workspace headers to `folder name · kebab · new task`. Editor,
terminal, copy-path, and pin actions now live in one anchored workspace options
menu, removing the split open control from the row. Verified desktop check and
focused destination tests, then exercised the compact header, menu sections,
pin action, and copy-path interaction in `just desktop-dev`.

# 2026-09-11 — make thread menus floating overlays

Reworked per-thread overflow menus so they render as deferred absolute
overlays instead of inline children that remeasure and push the session list.
The existing rename, pin, archive, and group actions remain available, while
selection and Escape dismiss the menu cleanly. Verified all 105 desktop tests,
arch-check, diff checks, and a live `just desktop-dev` interaction showing the
menu anchored below a thread without row reflow.

# 2026-09-11 — keep desktop chat responsive with bounded history pages

Preserved markdown and tool-output caches across client snapshots, virtualized
tool diffs, fixed the user bubble width regression, and made the composer a
focusable accessible text field. Session attach now loads the newest history
page first; scrolling to the top requests older pages through a cursor, keeping
large sessions out of a single oversized WebSocket frame. The shared protocol,
daemon, TUI, mobile, bridge, and web client all accept the paged history shape.
Verified focused Rust and TypeScript tests, the full desktop and TUI unit
suites, clippy, arch-check, formatting, diff checks, and a fresh `just
desktop-dev` visual smoke pass with composer typing and transcript scrolling.

# 2026-09-10 — rework the TUI goal system

`/goal <text>` no longer sets the goal sight-unseen and no longer loses
text on Enter. The Enter-with-autocomplete behavior was the bug behind
"no active goal" replies: pressing Enter while the slash completion list
was showing applied the completion (replacing `/goal fix the bug` with
`/goal `) instead of submitting the typed text. Enter now always submits
what's typed; Tab applies completions. `/goal <text>` opens a new
GoalCompose modal — an editable objective using the shared readline
editor (with "edited from: <original>" when changed, and a "will
replace: <active goal>" warning when one exists), Enter to set, Esc to
cancel; nothing reaches the daemon until Accept. A local goal view
(`App.active_goal`) mirrors the daemon's state and renders a goal
section in the sidebar (objective + status) and a 🎯 status-bar pill
while a goal is in flight; it updates on set/pause/resume/clear/
complete and when an agent `propose_goal` proposal is accepted. Agent
side untouched. Coverage: 5 new tests — Enter submits multi-word slash
args, compose edit→SetGoal, Esc/empty-Enter cancel, replacing-goal
capture, and proposal-accept registering the active goal (reject leaves
it). `cargo test -p mew-tui` (258 + integration), `cargo test -p mew`,
clippy `-D warnings`, and fmt clean.

# 2026-09-10 — undo and visible cursors in every TUI text field

Follow-up to the shared readline editor. Undo is no longer
chat-input-only: `editor::UndoHistory` (state snapshots with the chat
input's 500 ms coalescing and 100-entry cap) backs the question
freeform, plan feedback, history search query, settings buffer, and
picker filter; `handle_key_with_undo` routes edits through it and binds
Ctrl+Z / Ctrl+Y on every field, and the chat input itself now uses the
same `UndoHistory` type (its three ad-hoc stack fields were replaced
with one `undo_history` field, behavior unchanged). Cursor rendering
now follows the edit position on all surfaces: the question freeform and
history-search cursors point into the text (was always the end), the
picker filter cursor measures display width instead of byte offset
(fixes drift on multibyte filters), the settings buffer's `│` marker
sits at the cursor instead of after the text, and the plan feedback
editor replaced its end-of-text `▏` glyph with a terminal cursor placed
through the same word-wrap the renderer uses. All cursor placement uses
`set_cursor_position`, so golden frames are unaffected. Coverage: 3 new
editor unit tests (coalescing, undo/redo round trip, ^Z/^Y routing) and
a freeform ^Z/^Y key-event test; the App undo tests were migrated to the
shared type's accessors. `cargo test -p mew-tui` (253 + integration),
clippy `-D warnings`, and fmt clean.

# 2026-09-10 — one readline editor for every TUI text field

The chat input had a complete readline-style editor (^A/^E/^F/^B/^D/
^K/^U/^W, Alt word moves, Home/End, undo) embedded in `App`, but every
other text field was push/pop-only: the question overlay's freeform
answer, plan feedback, history search query, the settings buffer, and
the picker filter (the last had arrow cursor moves only). All of those
now share one editing core, `crates/mew-tui/src/app/editor.rs`, which
owns the edit operations (insert/backspace/delete, char + word cursor
moves, line-aware Home/End, kill-to-end, clear) and the key router
`handle_key`. `app/input.rs` now delegates to it, so the chat input and
thin fields are driven by identical code and bindings; Enter/Esc/Tab/
arrows stay surface-specific. Each surface gained a cursor field
(freeform/feedback/history-search/settings), the picker's budget-row and
model-picker-Right special cases were hoisted ahead of the shared
router, and mutation still resets the picker selection as before. Undo
(^Z/^Y) remains chat-input-only, and the small overlays don't draw their
cursor glyph yet; both are follow-ups. Coverage: 9 unit tests for the
editor ops + router, plus key-event tests for readline keys in the
question freeform (^B/^U) and plan feedback (^A/^K). `cargo test -p
mew-tui` (249 + integration), clippy `-D warnings`, and fmt clean.

# 2026-09-10 — modal text editors stop swallowing editing/navigation keys

When the "Type your own answer" row of a user question (multi-choice
prompt) was selected, `handle_user_question_key` unconditionally treated
j/k/h/l as vim navigation and digits 1–9 as row-jump shortcuts, so those
keys couldn't be typed into the freeform answer. The handler now detects
when the freeform row is selected and routes printable keys (including
n/y) and backspace into the text while Enter/Esc (submit/cancel) and
Tab/arrows (navigation back onto option rows) keep working. The plan
approval card had the same class of bug in its feedback editor: letters
already typed (a/r/s are gated on `!editing`/Ctrl), but Tab/Left/Right
still cycled the Approve/Request-changes/Submit selection mid-edit,
which could approve the plan or submit changes while dropping the typed
feedback. Those are now inert while the feedback editor is open. Other
text surfaces (chat input, `@` picker) were already safe: their
shortcuts are modifier-gated or gated on an empty input. Coverage: four
key-event-level regression tests in `mew-tui` (j/k/h/l/digits/n/y type
into the question freeform, backspace + Enter submit, j/digit still
navigate on option rows, and the plan feedback editor keeps its
selection under Tab/Left/Right and types every shortcut letter).
`cargo test -p mew-tui` (243), clippy `-D warnings`, and fmt clean.

# 2026-09-10 — `mew daemon --status` introspection command

`mew daemon --status` reports whether the daemon is running, its PID,
build revision, socket/TCP endpoints, and uptime (exit 0 running / 1
not). Build identity is the git short hash (`<hash>-dirty` when the
tree had uncommitted changes), embedded by a new `crates/mew/build.rs`
as `MEW_GIT_HASH` and exposed via `crate::version::git_rev()`. The
daemon writes a `mew.status.json` status file next to the pidfile at
startup (version, pid, socket, port, start time); `--status` uses the
pidfile (or the status file's pid) for liveness and the status file for
metadata, then compares the daemon build to the local binary's, printing
a restart hint when they differ or when the daemon predates the current
binary. `--stop` now removes the status file too. Custom `--pidfile` is
respected on both sides. Coverage: status-file placement/round-trip,
not-running (missing pidfile and dead pid), running with status file
(version/endpoints/stale hint), and running without status file (unknown
build + restart hint); verified live against a background fake daemon.
Builds are identified by git hash rather than crate version for now.
`cargo test -p mew`, clippy `-D warnings`, and fmt clean.

# 2026-09-10 — identify OpenCode traffic with a stable conversation session

`opencode-zen`/`opencode-go` (and any provider pointed at an `opencode.ai`
base URL) now send a `User-Agent: mew/<version>` and a stable
`x-opencode-session: <session ulid>` on every chat-completions and /models
request, so the gateway can route and reuse prompt caches per conversation.
The conversation id is the mew session id, threaded into the provider build
path (`build_session_agent`, daemon model switcher, Auto/Auto+ classifier,
subagent `MainModelResolver`, and the fallback-model builder); the header is
scoped inside the OpenAI-compatible adapter, keyed off the base URL, so
non-OpenCode endpoints are untouched. Coverage: adapter header unit tests for
OpenCode vs non-OpenCode base URLs and the no-session case, plus the
existing provider/daemon suites; also dropped a pre-existing needless
`..Default::default()` in an adapter test struct literal. `cargo test -p
mew-provider-openai`, `cargo test -p mew`, clippy `-D warnings`, and fmt
clean.

# Earlier work (summarized)

Entries before 2026-09-10, newest first. Bodies dropped in the
2026-09-28 compaction; full detail is in git history.

- 2026-08-29 — shared subagent base prompt
- 2026-08-29 — expose skill descriptions to prompt templates
- 2026-08-29 — widen native GPUI chat scroll target
- 2026-08-29 — polish native GPUI session rail
- 2026-08-29 — deep UX pass for native GPUI chat and workbench
- 2026-08-29 — defer transcript scroll persistence outside GPUI list callbacks
- 2026-08-28 — improve native GPUI session rail
- 2026-08-28 — keep native GPUI sidebar visibility app-wide
- 2026-08-28 — remove completed feature plans
- 2026-08-28 — remove obsolete frontend parity plans
- 2026-08-28 — verification baseline and release checks
- 2026-08-12 — inline @-mention picker uses the chat input
- 2026-08-12 — unified @ reference picker, drop "no results"
- 2026-08-12 — landing autocomplete gets a visible surface
- 2026-08-12 — Landing autocomplete surface
- 2026-08-11 — `@` picker docks at the bottom; unknown namespace refs are silent
- 2026-08-10 — compaction renders as a collapsible tool-call-style block
- 2026-08-10 — wire builders demote images for non-vision models
- 2026-08-10 — reasoning truncator no longer breaks tool-pairing
- 2026-08-10 — skip Running tool calls in wire builders
- 2026-08-10 — read tool returns images for vision-capable models
- 2026-08-10 — namespace picker: truncate list lines so row math stays exact
- 2026-08-10 — @ picker and permission modal: display-row scrolling with real wrapping
- 2026-08-10 — scrollbar thumb reaches track bottom (all four TUI scrollbars)
- 2026-08-09 — sidebar retention for finished subagents and done todos
- 2026-08-09 — review findings fixed
- 2026-08-09 — code-review pass on orchestration branch (2 subagents)
- 2026-08-09 — orchestration phase 5: typed handoffs (plan complete)
- 2026-08-09 — orchestration phase 4: durable task registry
- 2026-08-09 — orchestration phase 3: todo ↔ subagent links
- 2026-08-09 — orchestration phase 2: concurrency cap + depth policy
- 2026-08-09 — orchestration phase 1: fan-in + leak reminder
- 2026-08-09 — orchestration assessment doc
- 2026-08-08 — namespace references: `@skill:`, `@model:`, `@subagent:` input syntax
- 2026-08-05 — TUI sidebar restructure: activity-first, static info demoted
- 2026-08-05 — fix sidebar click-to-toggle drift
- 2026-08-04 — openai provider: tolerate usage chunks with no `choices` field
- 2026-08-04 — grep tool: expose ignore/size/context controls and surface empty-result hints
- 2026-08-04 — daemon mode now reports the active thinking variant
- 2026-08-03 — Fix model picker dropping providers with shared model ids
- 2026-08-03 — daemon broadcasts context occupancy to all frontends
- 2026-08-03 — status bar shows context occupancy, not lifetime totals
- 2026-08-03 — sort TUI session rail/picker by last activity
- 2026-08-03 — fix char-boundary panics when truncating multibyte text
- 2026-08-03 — filter non-text-output models from the picker
- 2026-08-03 — verify Alibaba Token Plan setup, fix local config override
- 2026-08-03 — add Alibaba Token Plan built-in providers
- 2026-08-03 — native shell correctness and virtualization pass
- 2026-08-03 — sort sidebar sessions by recent activity
- 2026-08-03 — apply permission modes without waiting for active turns
- 2026-08-03 — render visible list bullets in TUI markdown
- 2026-08-02 — desktop session management, @-mentions, file tree, inline diffs (parity plan phase 4 batch C)
- 2026-08-02 — desktop composer parity: slash menu, history, pickers, plan feedback (parity plan phase 4 batch B)
- 2026-08-02 — desktop activity, usage, and presence parity (parity plan phase 4 batch A)
- 2026-08-02 — client-core parity items (parity plan phase 3)
- 2026-08-02 — TUI parity items 8, 5, 2 (parity plan phase 2)
- 2026-08-02 — TUI daemon-mode wiring fixes (parity plan phase 1)
- 2026-08-02 — daemon/client parity correctness pass
- 2026-08-02 — native shell UX review pass
- 2026-08-02 — float required-action prompts above the composer
- 2026-08-02 — bound desktop streaming and browser invalidation work
- 2026-08-01 — Put the native top bar on the base surface
- 2026-08-01 — Populate native pickers, theme preferences, and browser URLs
- 2026-08-01 — Let persona options size to their content
- 2026-08-01 — Keep streaming tool rows cacheable
- 2026-08-01 — Align the model picker to its trigger
- 2026-08-01 — Make daemon restart and native recovery complete
- 2026-08-01 — Bound responsive workbench and restore browser sessions
- 2026-08-01 — Make native client teardown cancellable
- 2026-08-01 — Make stalled turns retryable
- 2026-08-01 — Finish the native shell performance and overlay pass
- 2026-08-01 — Keep session identifiers stable across history loading
- 2026-08-01 — Clarify native conversation state and shell affordances
- 2026-08-01 — Finish the native visual audit pass
- 2026-08-01 — Refine native shell hierarchy and density
- 2026-08-01 — Harden review-critical native shell paths
- 2026-08-01 — Defer native browser initialization out of render
- 2026-08-01 — Make session rows fill the navigation rail
- 2026-08-01 — Move collapsed rails off-canvas
- 2026-08-01 — Finish session interaction state and tool containment
- 2026-08-01 — Make workbench sizing viewport-aware
- 2026-08-01 — Make conversation attention states explicit
- 2026-08-01 — Harden CEF app-exit teardown and native commands
- 2026-08-01 — Remove stale workspace placeholders from the shell
- 2026-08-01 — Verify native connection timing before changing transport code
- 2026-08-01 — Harden process-exit browser teardown and real-session tabs
- 2026-08-01 — Correct variable-height sidebar rows and attachment identity
- 2026-08-01 — Native bundle verification for the review pass
- 2026-08-01 — Complete native lifecycle and interaction review pass
- 2026-08-01 — Anchor preference menus above their triggers
- 2026-08-01 — Restore metadata-only catalogs and composer-safe shortcuts
- 2026-08-01 — Let model picker rows size to their content
- 2026-08-01 — Remove horizontal borders from the workbench rail
- 2026-08-01 — Preserve view state while switching sessions
- 2026-08-01 — Restore GPUI focus after native browser interaction
- 2026-08-01 — Resolve CEF assets from native app bundles
- 2026-08-01 — Remove duplicate browser labels
- 2026-07-31 — Add open-conversation tabs and collapsible shell regions
- 2026-07-31 — Reduce native shell stream update churn
- 2026-07-31 — Render markdown and virtualize native shell lists
- 2026-07-31 — Turn the composer into a real chat input
- 2026-07-31 — Split the desktop shell into focused modules
- 2026-07-31 — Stabilize conversation switching and shell motion
- 2026-07-31 — Package the native CEF helper topology
- 2026-07-31 — Animate native sidebar collapse
- 2026-07-31 — Bring required actions into the native shell
- 2026-07-31 — Add the framework-independent native diff model
- 2026-07-31 — Connect the native workbench to local diffs
- 2026-07-31 — Complete native turn controls and persona selection
- 2026-07-31 — Add native file attachments
- 2026-07-31 — Centralize native shell commands
- 2026-07-31 — Bootstrap a native PTY terminal surface
- 2026-07-31 — Move native terminal ownership into the daemon
- 2026-07-31 — Make native GPUI the default desktop workflow
- 2026-07-31 — Animate the workbench rail
- 2026-07-31 — Smoke-test the native launch path
- 2026-07-31 — Persist the native window frame
- 2026-07-31 — Persist native shell layout
- 2026-07-31 — Expose terminal search and copy controls
- 2026-07-31 — Add a stateful native terminal grid
- 2026-07-31 — Verify native rail motion visually
- 2026-07-31 — Add the native browser portal boundary
- 2026-07-31 — Decouple native browser packaging from the Tauri shell
- 2026-07-31 — Complete the native browser focus boundary
- 2026-07-31 — Restore markdown wrapping inside native transcript rows
- 2026-07-31 — Compact native chrome and Tabler controls
- 2026-07-31 — Bundle and persist native terminal typography
- 2026-07-31 — Bound transcript markdown work per virtualized row
- 2026-07-31 — Move native terminal rendering to libghostty
- 2026-07-31 — Keep streaming updates on the lightweight path
- 2026-07-31 — Make native CEF development self-contained
- 2026-07-31 — Extract the iroh desktop transport
- 2026-07-31 — Wire native iroh profiles into the desktop client
- 2026-07-31 — Persist native connection profiles
- 2026-07-31 — Keep mobile protocol translation compatible
- 2026-07-31 — Remove the legacy Tauri desktop shell
- 2026-07-31 — Make user transcript turns content-sized
- 2026-07-31 — Inset the conversation and remove collapsed rails
- 2026-07-31 — Inset the full native workbench
- 2026-07-31 — Tighten the tab-to-shell edge
- 2026-07-31 — Add the native settings page
- 2026-07-31 — DeepSeek V4 Flash over the Responses API
- 2026-07-31 — Split settings into pages and a top-bar tab
- 2026-07-31 — Replace local workspace rail with daemon-backed groups
- 2026-07-31 — Fix grouped sidebar row overlap
- 2026-07-31 — Make session rows single-line
- 2026-07-31 — Constrain title truncation and pad the rail scroll surface
- 2026-07-31 — Guarantee visible session title ellipses
- 2026-07-31 — Show session paths beneath width-truncated titles
- 2026-07-31 — Render session paths relative to home
- 2026-07-31 — Hover session actions over compact titles
- 2026-07-31 — Center chat content at a readable measure
- 2026-07-31 — Add an auxiliary bar and hideable terminal surface
- 2026-07-31 — Move auxiliary navigation to a vertical rail
- 2026-07-31 — Clarify session hierarchy in the navigation rail
- 2026-07-31 — Add grouped-session creation and drag/drop
- 2026-07-31 — Restore native composer deletion actions
- 2026-07-31 — Align composer editing with GPUI's native input model
- 2026-07-31 — Add composer selection and caret motion polish
- 2026-07-31 — Show the focused empty composer caret
- 2026-07-31 — Add native transcript text selection
- 2026-07-31 — Extend transcript selection across rendered rows
- 2026-07-31 — Tune caret and selection contrast
- 2026-07-31 — Add syntax coloring to fenced code
- 2026-07-31 — Render reasoning and tool parts in chat
- 2026-07-31 — Use the bundled mono face for code
- 2026-07-31 — Make conversation tabs horizontally scrollable
- 2026-07-30 — Load sessions and expose model selection
- 2026-07-30 — Distinguish daemon errors from connection failures
- 2026-07-30 — Wire the native composer to daemon prompts
- 2026-07-30 — Establish the Codex-style native shell composition
- 2026-07-30 — Isolate Tokio transport from the GPUI executor
- 2026-07-30 — Add framework-independent conversation navigation model
- 2026-07-30 — Allow native desktop session commands on local daemon transport
- 2026-07-30 — Bootstrap the native GPUI desktop shell
- 2026-07-30 — Route Tauri daemon lifecycle through shared supervisor
- 2026-07-30 — Introduce typed mobile client events
- 2026-07-30 — Introduce shared daemon-wide mobile state shadow
- 2026-07-30 — Route mobile session projection through shared core
- 2026-07-30 — Add Tauri-free desktop supervisor boundary
- 2026-07-30 — Add headless client lifecycle and reconnect coverage
- 2026-07-30 — Start framework-independent desktop client core
- 2026-07-30 — Re-fix Kimi K3 tool-call response mismatch (jobblock:80)
- 2026-07-26 — Fix duplicated thinking duration across reasoning blocks
- 2026-07-26 — Fix Kimi K3 (OpenAI adapter) tool-call validation errors
- 2026-07-26 — Fix mew-prompts transclude and make it render nested content
- 2026-07-26 — Fix startup crash on unknown provider and make config loading consistent
- 2026-07-25 — Add Gleam language support to code highlighter and hashline block resolver
- 2026-07-24 — hashline test + fun facts refresh
- 2026-07-19 — Switch Kimi to OpenAI adapter + fix reasoning_content deserialization
- 2026-07-19 — Preserve full daemon provider errors
- 2026-07-19 — Avoid blocking on an uncredentialed remembered provider
- 2026-07-19 — Gate in-app browser tools to desktop sessions
- 2026-07-19 — Load the bundled web fonts
- 2026-07-19 — Add iOS-matched font preferences to web and desktop
- 2026-07-19 — Add explicit remote daemon and desktop remote modes
- 2026-07-19 — Harden remote pairing and settings lifecycle
- 2026-07-19 — Close remote lifecycle and protocol review gaps
- 2026-07-19 — Fix live remote pairing invites
- 2026-07-19 — Repair daemon listener structure and observe scope
- 2026-07-19 — Invert CEF/WKWebView layering, delete AppKit menus
- 2026-07-19 — Fix inverted transparency: opaque chrome, truly see-through WKWebView
- 2026-07-19 — WKWebView transparency take 3: drawsBackground on the view instance
- 2026-07-19 — Re-assert CEF layering on every visible claim
- 2026-07-19 — Layering inversion abandoned: CEF stays on top (WebKit composites opaque)
- 2026-07-18 — Surface real credential diagnostics instead of bare "get credential"
- 2026-07-18 — Codex-style browser workbench tabs
- 2026-07-18 — Packaged Tauri native smoke test
- 2026-07-18 — CEF renderer startup unblocked
- 2026-07-18 — Shared native browser session verification
- 2026-07-18 — Tauri dev framework preparation fix
- 2026-07-18 — Browser protocol mismatch recovery
- 2026-07-18 — Workbench tab restructuring plan
- 2026-07-18 — Resizable workbench decision
- 2026-07-18 — Resizable workbench shell implemented
- 2026-07-18 — Unified workbench tabs implemented
- 2026-07-18 — Browser lifecycle and tab routing hardening
- 2026-07-18 — Desktop browser soak and lifecycle hardening
- 2026-07-18 — Desktop browser authority and shutdown fixes
- 2026-07-18 — UI motion and surface polish
- 2026-07-18 — Browser workbench chrome cleanup
- 2026-07-18 — CEF navigation pump re-entrancy fix
- 2026-07-18 — iOS motion and surface parity
- 2026-07-18 — Native Tauri workbench menu boundary
- 2026-07-18 — Browser connection lifecycle guard
- 2026-07-18 — Browser navigation deduplication guard
- 2026-07-18 — Chat rendering stability and overflow guard
- 2026-07-18 — macOS XDG-style config directory
- 2026-07-18 — Files root navigation
- 2026-07-18 — Workbench surface picker
- 2026-07-18 — Browser omnibox and native tools menu
- 2026-07-18 — Composer surface simplification
- 2026-07-18 — Floating dock scroll containment fix
- 2026-07-18 — Unified modular workbench tabs
- 2026-07-18 — Workbench header removal
- 2026-07-18 — Composer containing-block correction
- 2026-07-18 — Full-surface composer anchoring
- 2026-07-18 — Floating composer treatment
- 2026-07-18 — Positioned floating composer
- 2026-07-18 — Workspace shell sizing correction
- 2026-07-18 — On-demand workbench tabs
- 2026-07-18 — Main request flow and focused workbench tabs
- 2026-07-18 — Read-only Files workbench
- 2026-07-18 — Bound file viewer code width
- 2026-07-18 — Isolate unwrapped file code
- 2026-07-18 — Flush file preview surface
- 2026-07-18 — Fill file preview height
- 2026-07-18 — Tighten file code leading
- 2026-07-18 — Restore live response streaming
- 2026-07-18 — Restore configured provider models
- 2026-07-18 — Bound file viewer code width
- 2026-07-18 — Bounded file viewer
- 2026-07-18 — File viewer line wrapping
- 2026-07-18 — File viewer overflow containment
- 2026-07-18 — Nested file pane sizing
- 2026-07-18 — Restore file viewer width
- 2026-07-18 — Bound file viewer code width
- 2026-07-18 — Add desktop install and daemon cleanup recipes
- 2026-07-18 — Add in-app workspace folder browser
- 2026-07-18 — Ignore filesystem browsing in TUI capture
- 2026-07-18 — Harden folder picker review findings
- 2026-07-18 — Recover from unavailable remembered sessions
- 2026-07-18 — Persist desktop daemon logs
- 2026-07-17 — Fix dev CEF Mach-port rendezvous by anchoring a main bundle
- 2026-07-17 — Add Kimi (Moonshot AI) provider
- 2026-07-17 — Tauri desktop shell scaffold
- 2026-07-17 — Tauri daemon supervision and shared host bootstrap
- 2026-07-17 — Tauri sidecar packaging and daemon ownership
- 2026-07-17 — Desktop daemon rendezvous and adversarial UX pass
- 2026-07-17 — Desktop release verification
- 2026-07-17 — Final daemon lifecycle and UX verification
- 2026-07-17 — Session rail overlap and radius restoration
- 2026-07-17 — Interactive activity panel and workspace lifecycle pass
- 2026-07-17 — Keyboard-first session and project search
- 2026-07-17 — Attention-first notification hierarchy
- 2026-07-17 — Browser-use vertical slice
- 2026-07-17 — macOS CEF authoritative-browser proof of concept
- 2026-07-17 — Tauri native sibling integration
- 2026-07-17 — CEF reopen lifecycle hardening
- 2026-07-17 — Daemon sidecar rebuild
- 2026-07-17 — Workspace surfaces design direction
- 2026-07-17 — Independent workspace surfaces implementation
- 2026-07-17 — Tauri CEF dev preparation fix
- 2026-07-17 — CEF development runtime assets
- 2026-07-17 — CEF diagnostic cleanup
- 2026-07-14 — Rework mew theming to a flat, aliased token table
- 2026-07-14 — Fix slow rasterization by caching the font system
- 2026-07-13 — Real-time streaming, flushing, tracing, and fast typing in daemon `tui-capture`
- 2026-07-13 — Daemon-connected `mew tui-capture --connect`
- 2026-07-13 — Real-provider `mew tui-capture --connect` improvements
- 2026-07-13 — Settings overlay capture verb
- 2026-07-13 — mew-raster: ~190x faster frame rasterization
- 2026-07-13 — mew-raster: fix tofu glyphs and block-element seams
- 2026-07-13 — tui-capture: animate spinner in daemon-mode recordings
- 2026-07-12 — Context Window Inspector: Steps 7, 8, 9
- 2026-01-21 — hidden `mew funfact` easter egg

# 2026-09-29 — TUI: deduplicate model picker search results

Model picker search hid section headers but kept both the Recent shortcut and
the full-list entry. Non-empty model searches now deduplicate matching rows by
full model ID. Empty searches retain both browsing sections, and identical model
names under different providers remain separate choices.

Verified the regression failed with two rows before the fix, then passed with
`cargo test -p mew-tui --lib` (266 tests). Added coverage for label and
case-insensitive description searches, distinct providers, and clearing search.
`cargo fmt --all -- --check` and `git diff --check` pass.

# 2026-10-03 — TUI: keep queued prompts behind the full daemon turn

Daemon prompt streams now close when their attached session reports Idle, after
all provider and tool events have drained. The TUI keeps its busy state until
that stream closes, including errors and cancellation shutdown. Queued prompts
and unknown slash commands use the same submission path; Up-Up prioritizes a
queued prompt after cancellation finishes. Disconnects report an error and close
both current and subsequent prompt streams.

Verified `cargo test -p mew-daemon --lib` (26 tests), `cargo test -p mew-tui --lib`
(271 tests), and `cargo test -p mew --bin mew dispatch_table_tests` (22 tests).
Scoped clippy for all targets in `mew-daemon`, `mew-tui`, and `mew` passes with
warnings denied, as do formatting, architecture, generated-theme, and diff
checks. Loopback socket tests require execution outside the filesystem sandbox.

# 2026-10-03 — Desktop: task-flow and browser UX assessment

Completed a source-based assessment of the native desktop task flow and browser,
with independent task-flow and browser assessments and current official OpenAI
documentation for projects, browser interaction, and local review. The requested
priority is projects, conversations, progress, review, and follow-ups.

The highest-impact gaps are whole-run lifecycle state in client-core, incomplete
per-session operational cwd, dropped completion alerts, active-run submission
without guidance, and review data loaded locally against HEAD with count-based
cache invalidation. Preserve the existing workspace grouping, session view state,
agent runtime, and protocol while addressing those boundaries.

Browser automation selects the native CEF endpoint only when the separate
MEW_BROWSER_CDP_PORT variable is configured. The shell restores conversation URLs
into one global page, and hides the browser by stopping its message pump. A
focused browser redesign needs explicit ready endpoints and task/page ownership,
runtime lifetime independent of panel visibility, navigation state and recovery,
and coordinated native focus/overlay visibility.

Recommended proof slice: start a task in a second project, keep status accurate
through a long tool call, send guidance, switch away and receive completion,
review fresh changes, and let human and agent use the same page while it is
hidden. Evaluate native browser composition and performance on that slice before
choosing a shell replacement. Source inspection does not justify a full rewrite.

Verified findings against the desktop, client-core, daemon, agent builder, browser
tools, and CEF implementation. Live macOS inspection remains blocked by the locked
Mac; visual scores and performance conclusions are deferred. No product code was
changed by this assessment. Existing TUI changes were verified and committed
separately as d8fd27a.

# 2026-10-03 — Cross-client runtime and recovery assessment

Expanded the UX assessment beyond desktop to the daemon, TUI adapter, shared Rust
client reducer, TypeScript SDK/web store, mobile core, and Swift application. Three
independent review lenses checked session correctness, client/protocol drift, and
runtime integration. No product source was changed. Findings below distinguish
execution defects from display and recovery defects; a shell replacement alone
would retain these failures.

## Task ownership and recovery

- The daemon cancels a running turn and removes its agent 30 seconds after its
  last client leaves. Switching conversations uses that same detach path, so a
  task cannot reliably continue while the user works in another conversation.
  Source: `crates/mew-daemon/src/lib.rs:468`.
- Attaching replays history and selected settings, but no outstanding permission,
  question, plan, or goal request payload. Pending maps retain response senders
  only. A newly attached client cannot recover the action blocking the task.
  Sources: daemon `lib.rs:953`, `session.rs:69`.
- Resume reconstructs the agent from startup provider/model, persona, thinking,
  and permission defaults, restoring messages and cwd rather than per-session
  policy. Ordinary conversation switching can trigger this reset after the same
  30-second unload. Sources: daemon `session.rs:351`, CLI `commands/daemon.rs:92`.
- Daemon sessions pass `todos_path=None`, so todo state is not persisted and
  disappears on unload. Noninteractive `run` does persist it. Sources: CLI
  `commands/daemon.rs:119`, agent `tools.rs:2165`, CLI `commands/run.rs:91`.
- The earlier per-session cwd finding affects every daemon client: context may
  load from the selected project while shell and ToolCtx still use daemon cwd.
  Sources: CLI `setup/agent.rs:508/580`, agent `tools.rs:566`.

## Session state differs between clients

- TUI's DaemonClient forwards translated events only through the channel created
  by its own `prompt()`. Attach and turns initiated by another client have no
  persistent event subscription. Assistant/tool activity and actionable requests
  are dropped while sidebar notifications can still arrive. Source: daemon
  `client.rs:161/217`.
- Desktop's shared client reducer and mobile's compatibility event layer confuse
  provider MessageEnd with completion of the full agent turn. Desktop can offer
  Send and iOS can stop showing progress while tools/continuations remain active.
  The recent TUI full-turn fix does not repair these separate reducers. Sources:
  client-core `reducer.rs:1035`, mobile-core `lib.rs:1342`.
- RequestResolved dismisses permissions in web but leaves questions and plan
  approvals interactive; TUI ignores the notification and leaves its modal open
  even after finish_turn. Sources: web `stores/session.ts:1411`, TUI
  `app/mod.rs:896/2012`. Rust client-core already resolves all required actions.
- Attach now sends only the newest 24 canonical messages. Desktop supports
  pagination; TUI, web, and iOS discard or do not expose the cursor and cannot
  reach older history. Tool continuations count against that limit. Web SDK's
  independent history cache also omits live arrivals when merging a later page.
  Sources: protocol `lib.rs:16`, daemon `lib.rs:1016`, web SDK `index.ts:1388`,
  TUI `app/mod.rs:981`, mobile-core `lib.rs:1273`.

## Runtime transitions and capabilities

- Approved handoff_plan emits PersonaSwitchRequested, but no daemon transition
  applies the builder persona. Web only changes its displayed persona; mobile
  reports PersonaSwitched; TUI stores a pending name with no consumer. The agent
  keeps its existing persona restrictions. Sources: daemon `lib.rs:3208`, web
  `stores/session.ts:1314`, mobile-core `lib.rs:1801`, TUI `app/mod.rs:2243`.
- Model switching replaces the top-level provider without updating model identity,
  catalog pricing, or the subagent runner's captured default provider. The picker,
  manifests, computed costs, and unpinned child requests can disagree. Source:
  CLI `commands/daemon.rs:261`, agent `events.rs:65`, `runner.rs:203`.
- Default-persona model pins are ignored. Manual switching announces the persona
  before building its pinned provider and silently suppresses provider failures.
  Sources: CLI `setup/agent.rs:601`, daemon `lib.rs:2351`.
- Canonical chat startup uses NopDispatcher and no discovered extensions. Plugin
  registration is skipped; declarative extension capabilities are absent. No
  production MCP discovery/connection call exists in `crates/mew`, despite the
  documented mcp.json behavior. Debug context and noninteractive run do not match
  chat capability loading. Sources: CLI `commands/daemon.rs:118`,
  `setup/agent.rs:590`, `docs/using-mew/mcp-servers.md:62`.

## Web and iOS-specific discrepancies

- OpenAI streams tool identity after PartStart. Web discards identity deltas and
  later searches by finalized call_id rather than stable part_id, leaving a
  completed live tool unnamed/pending with empty input/output. Replaying its
  finalized history renders it correctly. Sources: OpenAI provider
  `lib.rs:737/947`, web `stores/session.ts:570/727`.
- The TypeScript protocol omits GoalProposed/GoalResponse. A valid daemon goal
  request throws an unhandled-message error and has no approval UI or response
  method. Rust clients have these wire cases. Source: web SDK `index.ts:1543`.
- Web command discovery lists /help without implementing its client-side handler;
  daemon silently ignores it. Supported commands such as /goal and /wiki can
  become ordinary model prompts. SlashResult also has two transcript owners,
  appending /compact output twice. Sources: web `components/input-area.tsx:30`,
  `routes/session.$sessionId.tsx:33`, `stores/session.ts:1267`, daemon
  `lib.rs:1704`.
- Checked-in iOS AppStore does not compile against checked-in CoreEvent bindings:
  its switch lacks subagentEnd, planApprovalRequested, and goalProposed. Plan and
  goal requests also lack application handling. Source: `mew-ios/mew/AppStore.swift:407`.
- iOS applies history/stream/permission events from other connected daemons to
  the single visible conversation, discarding event identity. Responses can then
  target the selected daemon rather than the request's daemon. Source: AppStore
  `:446/467/479/503`.
- iOS sets isStreaming before sending, but disconnects and protocol errors do not
  clear it; sending on an absent connection silently does nothing. Its composer
  can remain Stop-only, with active-session errors excluded from its banner.
  Sources: AppStore `:296/408/538`, mobile-core `lib.rs:1746`.

## Verification and repair direction

A scratch executable used real daemon WebSocket handlers over in-memory duplex
streams, temporary session storage, and the existing deterministic FakeProvider
to emit an actual ask_user_question call. It confirmed that a second attach gets
history but no pending question, then that switching conversations cancels and
unloads the waiting turn after 30 seconds. The scratch Cargo project is at
`/private/tmp/mew-cross-client-audit`; it makes no external provider calls.

Review probes against actual web reducers/SDK reproduced live-versus-history tool
identity, missing goal dispatch, stale resolved questions/plans, and pagination
losing intervening live messages. A scratch executable linked to the TUI library
confirmed stale permission state after RequestResolved and finish_turn. Direct
Swift typechecking of real AppStore source against the checked-in binding module
confirmed the three missing cases. Full xcodebuild was blocked by sandboxed
SwiftPM/cache access; no successful iOS build or simulator test is claimed.
Reproduction sources are `/private/tmp/mew-cross-client-web-traces.mjs` and
`/private/tmp/mew-tui-resolved.rs`; exact TUI and Swift commands/logs are saved in
`/private/tmp/mew-tui-resolved-command.txt`, `/private/tmp/mew-tui-resolved.log`,
`/private/tmp/mew-ios-typecheck-command.txt`, and `/private/tmp/mew-ios-typecheck.log`.

Existing narrow suites pass: web store 21 tests, client-core 28 tests, and agent
handoff_plan 4 tests. These suites do not cover the failing cross-client traces;
handoff coverage asserts the request event rather than the applied transition.
An existing test-only unused_mut warning was observed in agent `turn.rs:1479`.
Native macOS visual, focus, and performance QA remains blocked by the locked Mac.

Repair the daemon's task lifetime, per-session operational state, and recoverable
required actions first. Make persona/model/capability transitions authoritative
and persistent. Then enforce one protocol contract and canonical message identity
across client projections, with a common trace corpus for tool continuations,
attach-mid-request, resolution from another client, paginated live history, and
switch/unload/resume. After these behaviors hold, build the project/task shell,
daemon-backed review, and shared human/agent browser slice identified above.
Keep the useful engine and tools while replacing duplicated state plumbing in
small verified changes. A complete engine rewrite is not justified by this audit.

# 2026-10-03 — Native-first session reliability triage

Prioritize the macOS native application and its Rust daemon/client dependencies.
Treat native mobile build/binding compatibility as a required compatibility gate,
and make only the TUI/web changes required by shared-contract changes in this
batch. Broader frontend parity and visual redesign remain queued. This entry is
triage, not an implementation record.

## First batch: reliable native sessions

1. **Remove the automatic handoff workflow.** Proposed scope is handoff_plan,
   its blocking plan approval, and the deferred automatic persona switch shared
   with switch_persona. Keep explicit persona selection and plan files. The user
   was asked to clarify whether switch_persona or personas should remain; that
   removal boundary is provisional until answered. Update built-in planner,
   builder, plan-reviewer, write_plan/edit_plan guidance and tool allowlists along
   with implementation. Do not add a replacement handoff primitive. Verification:
   planning can finish without waiting on a handoff approval, manual selection
   still works, and older history containing removed tool names remains readable.
   Retire native/wire consumers together, accounting for old running daemons and
   regenerating UniFFI bindings rather than editing generated Swift manually.

2. **Separate task lifetime from the open conversation.** Keep executing and
   waiting-for-input sessions alive while their daemon is alive; unload only
   eligible idle sessions. Switching or losing a viewer must not cancel work.
   Preserve explicit cancel/delete behavior. This does not promise that work
   survives quitting an app-owned daemon. Verification: start in A, switch to B
   beyond 30 seconds, return to A and observe the same live task; include a
   pending question, disconnect/reconnect, explicit cancellation, and idle unload.

3. **Make whole-run state authoritative in native clients.** Synchronize the
   attached session's running state with daemon activity and initial attach
   hydration rather than provider MessageEnd. Native composer, sidebar and mobile
   progress must agree during tool continuations, errors, and cancellation. Add
   desktop completion/failure feedback so work completed while viewing B remains
   discoverable. Verification: a multi-message tool turn never offers Send early;
   attach mid-run reports busy immediately; failures and disconnects release the
   composer and surface an actionable error.

4. **Recover pending input on attach.** Retain request payloads as well as response
   channels, hydrate the current actionable state, and resolve requests by stable
   IDs across clients and cancellation. Once handoff is removed, do this for the
   remaining permissions, questions, and goal approvals. Verification: reconnect
   while blocked, answer from another client exactly once, and cancel without
   leaving an actionable stale card. Any minimal TUI/web resolution fixes belong
   with this contract change; unrelated presentation work does not.

5. **Use the selected project throughout execution.** Thread session cwd through
   shell, ToolCtx, prompts, discovery, subagents and permission/workspace defaults.
   Verify with two real temporary repositories and a daemon launched in A while
   executing a native task in B. Relative reads/writes, shell pwd, loaded context,
   and child tasks must consistently refer to B.

## Second batch: operational state and compatibility

6. **Persist and apply session policy completely.** Restore model/provider,
   thinking, explicit persona, permission mode and todos after idle unload or
   daemon restart. Apply model identity, pricing and unpinned child providers
   together; a failed persona pin must report failure without claiming success.
   Verification: switch settings, create dependent todos, unload/reload, then
   inspect actual next top-level and child requests and operate on retained IDs.

7. **Close native compatibility gaps.** Native and iOS build/binding checks gate
   every earlier shared-contract change. Handle remaining subagent/goal events;
   isolate state by daemon and
   session; recover failed sends; implement usable older-history retrieval. Use
   canonical daemon traces to check native core/mobile projections. Before
   removing wire cases, exercise native app versus independently running older
   daemon and choose an explicit supported-version policy, with a visible
   incompatibility result rather than a silent missing feature. Avoid introducing
   a broad capability-negotiation framework without a concrete need.

## Next: native browser and review, then broader parity

After the first batch passes, fix the native browser endpoint/ready handshake,
task/page ownership and message-pump lifetime. Verify that hiding the panel or
switching conversations preserves the correct page, and that human and agent
operate on the same tab. Native navigation/focus and visual performance require
an unlocked Mac. Address review freshness and host provenance before expanding
review controls: same-count edits must refresh, and remote changes must come from
their daemon workspace rather than a coincidentally matching local path.

Queue full MCP/plugin/extension startup integration, web tool-identity/history/
command fixes, general TUI subscription/pagination work, and larger project/task
navigation redesign after these reliability gates. They remain tracked defects;
native-first narrows order rather than discarding them. Do not rewrite the shell
or engine as part of this triage.

Verified ordering and removal dependencies against tool registration, handoff and
switch_persona execution, built-in persona/subagent prompts, native action
consumers, daemon detach/attach, client-core activity handling, native version
handling, and mobile compatibility projection. Worktree was clean before this
append-only journal update; no product source changes or new test results.

# 2026-10-03 — Remove automatic planning handoffs

Removed handoff_plan and model-driven switch_persona, blocking plan approvals,
deferred persona events/state, transition metadata and client approval UI.
Explicit persona selection, read-only planning, plan files, write_plan/edit_plan,
and normal plan review remain. Built-in prompts now present a completed plan and
finish; selecting an execution persona is an explicit user action. Old saved tool
names and legacy persona frontmatter remain readable.

Added wire revision 1 to Ping/Pong compatibility. Native desktop, mobile and TUI
check the revision before attaching or sending session commands, with bounded
timeouts and visible update/restart errors. Missing legacy revision defaults to
0. The native supervisor refuses an incompatible existing daemon without
launching a second process. Unknown additive events remain tolerant, but malformed
handshake payloads fail explicitly. TypeScript exposes the normalized revision
while preserving ping()'s string result.

Regenerated Swift bindings and both iOS XCFramework slices from matching native
libraries. Added the remaining Swift event cases and a goal-response surface that
keeps daemon/session identity. ios-core now reuses its release generator and skips
unneeded dependency metadata rather than rebuilding the mobile library in debug.

Red regressions reproduced blocking retired calls, obsolete planner requirements,
automatic web persona changes, and acceptance of a legacy daemon before the fix.
Verified actual plan writing/ordinary completion, explicit persona selection, old
history replay, legacy/future/malformed/timeout handshakes, and authorization event
ordering. Passed agent/tools/personas/prompts suites, protocol 112, client-core 32,
local transport 4, supervisor 8, daemon 27, TUI 265, mobile 27, SDK 22 and web UI
116 tests, plus TypeScript/build checks and the tool-registry regression.
Native macOS cargo check and iOS device/simulator/host release builds pass.
The iOS simulator app compiles, links and launches; all 18 application tests pass
with clean compiler output. Workspace clippy, architecture and theme checks pass.

Concurrent native builds exhausted disk space. Removed only ignored, disposable
target/debug/incremental caches, preserved all source and build evidence, and
resumed checks with incremental compilation disabled. Native visual/focus QA
still requires an unlocked Mac. Next work is active-session lifetime.

# 2026-10-03 — Preserve active sessions without viewers

Conversation switches and final socket disconnects no longer cancel a task after
30 seconds. Idle reaping holds a weak reference and retries while a running or
queued operation owns the runtime. Unloading checks the exact active instance
and ownership under the manager lock, so a delayed attach cannot receive an
orphaned runtime and an old reaper cannot remove a newly loaded session.
Explicit cancellation remains effective; unused completed sessions still unload
and resume their persisted history. The per-session resume lock survives idle
unloading to prevent overlapping loaders.

Real duplex WebSocket regressions reproduced four failures before the fix:
switch-away cancellation, final-viewer cancellation, delayed-attach orphaning,
and stale-reaper eviction. All five lifetime tests now pass, including idle
workspace watcher shutdown, reattachment, explicit cancellation, and unloading
after cancellation. Full daemon checks passed: 27 unit, 6 command, 23 end-to-end,
5 lifetime and 6 session-management tests. Next work is authoritative run state.

# 2026-10-03 — Make native task activity authoritative

The daemon replays Running/Idle immediately after SessionReady and holds its
state lock until both replies are queued. Native projections now use session
activity for task lifetime. Provider starts and message ends only describe a
model response; token usage and manifests still update without ending the task.
Mobile no longer applies provider events twice. Desktop metadata-only snapshots
preserve the current task and required actions when the attachment is unchanged.
Swift ignores background text/part/activity events for other daemon/session
identities and restores running state from the matching snapshot.

Real compatibility tests uncovered a trusted legacy iroh peer rejecting modern
RemoteHello. Already allowlisted peers now receive RemoteReady; local TCP hello
still fails, untrusted peers remain rejected and legacy desktop capabilities
remain denied. No trust or capability restriction was removed.

Red regressions reproduced provider-boundary completion, doubled mobile usage,
metadata-only run clearing, missing attach activity and trusted-peer handshake
failure. Passed shared core 36, mobile 29, UI model 12, actual native model-source
8, daemon end-to-end 23, lifetime 6, iroh 2, TCP 7 and real mobile iroh 2 tests.
Normal macOS cargo check and scoped clippy pass. Swift parses and bindings were
regenerated from matching host metadata. Final device/simulator archives and
application tests will run after the remaining mobile recovery changes; those
results are not claimed here. Canonical generated Swift retains generator
whitespace. Next work is required-action recovery.

# 2026-10-03 — Recover required actions after attachment changes

The daemon retains each live permission, workspace approval, child approval,
question and goal payload with its original response channel. Attach resets the
target projection, sends history and then replays pending actions. Publication,
replay, resolution and cancellation are serialized. Wrong-kind and duplicate
responses do not dismiss requests; cancellation resolves all cards and suppresses
buffered requests from the cancelled turn. Closed channels are not replayed.

Native and web projections deduplicate request IDs, preserve origin daemon and
session identity, and use authoritative attention counts. History pagination
preserves live cards. Mobile snapshots include pending actions. TUI clients receive
actions even without a prompt stream and queue concurrent dialogs; resolving a
card elsewhere sends no extra response or cancellation. Escape rejects a goal or
cancels a waiting question, and matching Idle clears its cancellation indicator.

Red regressions reproduced missing replay, duplicate native cards, wrong-session
resolution, inferred mobile counts, missing snapshot actions and question Escape
without cancellation. Passed full daemon checks: 34 unit, 6 concurrency, 23
end-to-end, 6 lifetime and 7 TCP tests. Shared core 38, mobile 31, real mobile iroh
2, TUI 271 plus 5 goldens, SDK 23 and web UI 123 pass, along with client/UI builds
and scoped clippy. Swift bindings match current host metadata; handwritten Swift
parses. Matching device/simulator archives and application tests remain in the
final mobile gate. Next work is operational project isolation.

# 2026-10-03 — Execute tools in the session's project

Session builders now pass the operational cwd through tools, plans, templates,
permission resolution, background jobs and persistent shells. Relative shell cwd
is resolved against the session project. Each session retains its own shell state.
Child discovery and execution inherit the project, workspace rules and permission
engine; ordinary, nested and workspace approvals reach the parent client.
Children inherit the configured classifier with independent caches, so a cached
decision for one project's command cannot authorize the same command elsewhere.
The missing shell_monitor interception now uses the agent's existing job path.

Ten project behavior regressions pass after reproducing wrong-project paths,
dropped child approvals and absent child classification. Passed agent 184,
permissions 73, subagents 14 unit plus 12 integration, and the full CLI suite
(165 unit, 1 version, 2 integration), with scoped all-target clippy.
The broader config backup test writes a sentinel into the real user config
directory and fails under sandbox restrictions; it was not granted access to
overwrite user state. Separate shell issues remain recorded for another pass:
newline-free output can merge the completion sentinel, and already-completed
jobs can miss their completion notification. Next work is saved session policy.

# 2026-10-03 — Restore effective session policy and todos

Sessions save complete effective model, provider, persona, thinking and permission
settings in an atomic policy sidecar. Resume uses saved settings ahead of changed
defaults and reloads todo dependency state. Malformed policy or todos produce a
visible error. Idle session listings also use the saved model and provider.
Setting changes serialize with agent updates; failed persistence restores owned
and shared prompt state. Persona pins resolve before acknowledgement, bare pins
inherit the current provider, and clearing a persona survives reload.

Model switches refresh capabilities, pricing, context, default thinking, rendered
project prompts and child runners. Changed identities detach prompt caches while
preserving shared skill/template handles. Explicit thinking off retains required
model-specific disable payloads.

Behavior regressions reproduced lost settings/todos, failed pin acknowledgement,
lost concurrent updates, stale caches/prompts/child providers and persistence
rollback errors. Policy tests 11, session 11 unit plus 7 integration, CLI policy 6,
setup 22 and agent 185 pass. Full daemon checks passed 44 unit, 6 concurrency,
23 end-to-end, 6 lifetime and 7 TCP tests before the final bare-pin regression;
the updated policy suite passes. Full CLI passed 172 unit and 3 integration tests.
Scoped all-target clippy, format and diff checks pass. Next work is final native
reconnect and delivery feedback.

# 2026-10-03 — Recover native connections and show task outcomes

Mobile owns and cancels connection tasks, separates requested from confirmed
attachments and rejects commands queued for an obsolete connection/session.
Offline submissions retain the composer and do not create phantom messages.
Accepted submissions reserve their daemon/session until acknowledgement, closing
the double-send window without inventing Running activity. Delivery failures and
reconnect reset that reservation. History and deltas retain their original session.
The redundant compatibility history conversion was removed; canonical state is
projected once.

Native desktop retains completion/failure outcomes for background conversations,
marks unseen completions in the rail and shows the correct session's failure with
retry even after a partial assistant response. Activity and metadata stay
authoritative. iOS routes action navigation and replies to the origin daemon and
session, and clears input only after successful enqueue.

Regression tests reproduced offline/stale delivery, history leakage, lost
background outcomes, partial-turn retry and duplicate submissions before Running.
Mobile 41, real iroh reconnect 3, desktop 122, shared core 38 and UI model 12 pass.
Matching host, device and simulator libraries, generated bindings and XCFramework
were rebuilt; actual iOS 31 tests pass with no skipped tests. Scoped clippy passes.
The three independent code review lenses found the submission race now fixed and
redundant history work now removed; no other required findings remain.
Next work is native browser ownership/readiness/pumping and daemon-backed fresh
review. Actual CEF focus, bounds and rendering still require an unlocked Mac.

# 2026-10-03 — Fresh daemon workspace diff loading

Added a serializable diff model and fresh git workspace reader for daemon-owned
review. The reader captures one HEAD baseline, includes untracked files, follows
rename sources, and handles nested workspace paths, literal pathspec characters,
symlinks and gitlinks. Repository/object failures stay visible instead of looking
like an empty or added-file diff.

File reads and git stdout are bounded at 8 MiB. A shared 32 MiB work budget stops
large collections and bounds line/hunk allocation before computing diffs; the
wire response has its own smaller limit in the following native schema chunk.
The former path-list loader remains while the existing native caller is migrated.

Verified 16 real-repository/model tests and strict all-target mew-diff clippy.
Regressions first reproduced aggregate loading past its budget, dense newline
allocation and unbounded git output, then passed with bounded loading. No new
review UI is claimed by this prerequisite chunk.

# 2026-10-03 — Native browser leases and daemon review contract

Replaced the daemon's process-wide browser flag with a shared, revocable page
lease for each session. Only a confirmed local desktop connection can register
its explicit CDP endpoint, target and page generation. Registration becomes
available after binding; replacement, failed preparation, unregister, session
deletion and disconnect release the lease and retained runtime. Queued commands
keep their original generation and cannot move to another page.

Both browser wire commands and agent tools use the same runner. It requires
agent-browser 0.34 or newer, pins the exact target, preserves snapshot references,
and passes navigation/interaction data through the CLI's JSON batch input.
Literal flags remain literal page data. Close targets the actual CDP page;
cancelled processes and screenshot files are cleaned up. Named CLI connections
are disconnected on release and have an idle timeout for abrupt daemon exit.

Protocol revision 2 includes browser registration and identified session-diff
requests/responses together. Review reads the requested daemon workspace and
replies only to its requester; oversized/error responses retain both IDs.
Workspace change events retain session origin and observe HEAD/index changes,
including linked worktrees and external commits without a file rewrite. Previous
native protocol revisions are rejected before session commands. Extension wire
revision remains 1.

Verified real Chrome and daemon WebSocket fixtures, all seven browser actions,
reserved input text, same-URL page isolation, reference retention, replacement,
cancellation and weak runtime release. Final daemon/iroh integration gate passed
104 tests, with the manual browser fixture separately executed. Native core 41,
protocol 115, local transport and supervisor compatibility gates passed. Matching
mobile archives/bindings and all 31 iOS tests passed; SDK 23 and web UI 123 tests
and both builds passed. Scoped strict clippy, formatting, architecture and theme
checks passed. Native surface/UI integration follows as its own chunk.

# 2026-10-03 — Retained native conversation browser pages

The native portal retains an actual CEF page for each confirmed local conversation,
with immutable connection/session/generation callbacks and exact target readiness.
Switching conversations or hiding the panel preserves DOM state. One retained
GPUI main-thread pump serves the CEF runtime until app exit; the unused pump
callback and portal timer thread are removed. Unchanged renders no longer toggle
native visibility. Navigation arriving before page creation is applied afterward.

CEF target information comes from the page's own DevTools observer. Explicit
close callbacks leave generation-safe tombstones and release the daemon lease;
retry recreates only the selected owner's page. Ordinary HTTP(S) popup targets
navigate the owned page rather than reusing its client for a second browser.
Empty script-created popups requiring later window control remain unsupported.
The matched Alloy embedding has no default DevTools popup entry point.

Packaging includes the compatible standalone browser CLI. Repeated packaging
first replaces executable inodes, avoiding a reproduced macOS cached-signature
failure where an in-place copied browser binary was killed with exit 137. The
freshly packaged binary reports 0.34.0 and passes the real browser/daemon fixtures.

Verified native browser-host 9, CEF callback/lifecycle 8 and full desktop 129 tests,
strict all-target native clippy, formatting and architecture checks. Built the
actual macOS desktop, daemon and CEF helper and packaged the debug app with valid
bundle metadata and runtime assets. Browser methods are extracted into the
consumed shell/browser.rs module. The protocol/runtime foundation is e151238.

Live focus, bounds and rendering remain unverified. Automatic approval review
rejected launching the locally built app as software from an unrecognized source;
explicit launch confirmation was requested. A separate fixture app/config/session
store/browser cache is prepared for that check, without changing installed apps.

# 2026-10-03 — Native review uses fresh daemon diffs

The Changes panel requests current diffs from its attached daemon session, including
untracked files and edits outside a run. Removed the local HEAD worker and the
path/count signature cache. Pending requests coalesce invalidations; completed
payloads move into the panel only when both session and request IDs match. Late
responses, stale errors and previous attachments cannot replace the current view.

Visible Local/Changes views own workspace watches. Worktree and git-baseline
changes refresh review; reopening, reattaching and terminal activity also refresh
it. Directory-list responses remain separate from filesystem invalidations. File
rows and counts come from the accepted diff payload, and failures keep their
actual detail. Removed the obsolete public path-list diff loader, retaining its
path-safety, object-failure and symlink regressions on the consumed reader/helpers.

Verified the full native desktop suite (129), diff suite (16), eight real daemon
review/watcher tests, six watcher regressions and native all-target clippy. The
final daemon/iroh gate passed 104 tests; SDK/web tests and builds, generated themes,
formatting and architecture checks passed. Rebuilt and packaged the final native
source with the compatible standalone browser executable. Normal commit hooks
remain enabled. Review/control expansion, broader client parity, MCP/plugin
startup and larger task-navigation design stay in the previously triaged queue.

Live native visual testing still awaits explicit app-launch confirmation after
its automatic approval rejection. No focus, bounds or native rendering proof is
claimed by the source and integration gates.

### 2026-10-03: native task navigation and direct review

Added All, Running and Attention sidebar views with live counts from the existing
conversation projection. Running excludes tasks waiting for input; Attention
includes pending requests, idle failures and unseen completions. Filtered views
show every match across projects, including archived tasks with pending work,
without changing saved collapse preferences or the recent-task cap in All.
Search intersects the selected view. Filtered/search workspace headers have no
collapse control, so clicks cannot silently alter the saved All layout.

The attached conversation has a direct Review changes action that selects the
existing daemon-backed Changes panel and requests fresh review. Opening review
or the workbench folds the sidebar when needed to preserve the chat's minimum
width; an impossible review layout reports an error without altering layout.
Navigation leaves completion acknowledgement with the confirmed attachment.

Verified 135 native tests, strict all-target native clippy, formatting,
architecture and generated-theme checks. Read-only breaker/simplifier review
caught the ineffective filtered collapse control; its behavior is now covered.
Live rendering and interaction QA remain pending the previously requested
explicit launch confirmation. Task-owned draft/submission correctness follows
as a separate change.

### 2026-10-03: conversation-owned native drafts

Native text and attachments now move with their conversation and connection
profile. Switching tasks restores that task's draft and resets the transient
prompt-history recall stash. Startup attachment uses the same navigation path;
entering a draft or opening the file chooser preserves the new-task intent.
Closing the final tab keeps an unbound composer even when the daemon still has
its previous attachment and publishes background metadata. Explicit native
selection controls projection; daemon activity still populates the task list.
Reconnect retains the selected draft, and navigation cancels deferred model
choices from the previous task.

Verified 143 native tests, seven focused draft regressions, the real core/model
unbound-selection trace, strict native all-target clippy and formatting. Source
review found and verified the startup owner, last-tab metadata and recall-stash
fixes. An earlier non-escalated native run hit the loopback sandbox restriction;
the final escalated suite passed. Deferred prompt submission and attachment
acknowledgement ordering remain the next separate chunk.

### 2026-10-04: native attachment ordering and safe submission

Native attachment and creation now share one in-flight request. Navigation
retains its desired conversation and coalesces further switches until the
current acknowledgement arrives. Creation replies bind to their original draft;
navigation cancels automatic submission. An initial prompt stays visible until
the created task is confirmed, and edits or file selection cancel its automatic
send. Existing tasks still attaching require an explicit resubmit. A closed
local command queue preserves exact text, files and prompt history. Queue
acceptance does not guarantee daemon receipt.

Replaced client event batches are rejected before shell or browser mutation.
An ambiguous daemon error requires client reconnection; later replies and tab
navigation retain that recovery action. Retry preserves a live app-owned daemon,
reaps an exited child and remains retryable after a failed relaunch.

Verified all 153 native tests and 11 supervisor tests, strict all-target native
and supervisor clippy, formatting, architecture and generated themes. Tests use
real command queues, core event projection, child processes and loopback protocol
probes. Breaker, simplifier and operator source review caught recovery visibility
and retry routing gaps; final rechecks found no remaining required findings.
Live native interaction QA still requires the earlier explicit launch approval.

### 2026-10-04: native submission feedback

A rejected new-task request keeps its queue failure message and draft. Accepted
prompt submission clears provisional attachment feedback, and a closed command
sender offers client reconnection. Initial-prompt snapshots are confined to new
task submission. Verified all 154 native tests, nine focused request/submission
tests, strict all-target clippy and formatting. The feedback tests use real
accepted and closed queues; source review confirmed the render and submit routes.

### 2026-10-06: review subagent workspace access coverage

Added an integration test for built-in plan-reviewer and project code-reviewer
subagents launched through subagent_start. Both receive read-only file tools
and successfully discover, search and read source files in their parent
session's project without requesting workspace permission. The test uses real
tools and temporary projects, with scripted responses at the provider boundary.

Verified all 12 session-project setup tests. The reported review access failure
was not reproduced on this branch; the agent name and failure message are still
needed to distinguish a custom tool configuration from a path or permission
failure.

### 2026-10-06: complete TUI session resumption

TUI attachment now receives every history page instead of only the newest 24
messages. History comes from the persisted transcript with current in-memory
message updates, preserving turns removed from model context by compaction.
Desktop attachment keeps cursor-based pagination. TUI page handling rejects
other sessions, follows the latest message on resume and preserves a manually
scrolled viewport when older messages arrive alongside new tail content.

History-read failures remain visible without an active prompt, and attachment
still replays pending permission requests after a read failure. Added real
loopback client and daemon tests across empty, single-page and multi-page
histories, compaction, active/reloaded sessions and error handling, plus TUI
reducer and rendered-scroll regressions. Verified 383 daemon and TUI tests,
with one existing Chrome-dependent test ignored; strict all-target clippy for
mew, daemon and TUI, formatting, architecture and generated-theme checks passed.
The review investigation also passed all 212 agent and subagent tests.

### 2026-10-06: traced the reported reviewer access failure

Located the report in the streamplace latency-branch review session. The parent
launched all three review lenses as the built-in researcher and reported denied
reads for the repository, staged diffs and lens instructions. No custom
researcher definition was found. The historical runner did not inherit workspace
roots or forward permission requests, which explains that denial pattern; commit
3a22b63 already fixed both on October 3. The installed ~/.cargo/bin/mew executable
is dated September 29, pointing to an older runtime. Its daemon status reports no
active process, so the recorded session's actual executable remains unconfirmed.

Extended the existing real-file review regression to cover researcher as well
as plan-reviewer and project code-reviewer. Verified all 12 session-workspace
tests, including child permission forwarding for files outside the project.

### 2026-10-10: TUI composer visibility and cursor geometry

Queued follow-ups and history/paste prompts have reserved rows. Long drafts
keep the edited row visible through movement, recall, clicks and resize. The
editor and renderer share borrowed grapheme-aware rows, fixing odd-width CJK,
joined emoji, combining marks and exact-wrap boundaries. A glyph wider than a
one-cell text column has a visible replacement without modifying submitted
input. Narrow Ctrl+R searches keep their query caret inside the terminal.

Verified all 14 composer behavior tests with red/green regressions, all 309 TUI
tests including the doc test, strict all-target TUI clippy and the CLI build.
Source review caught the odd-width wrapping mismatch; the shared row geometry
resolved it. Headless captures confirmed the long-draft viewport.

### 2026-10-10: opaque TUI surfaces and contextual pickers

Modals and autocomplete use the raised panel overlay token, backed by each
theme's popover. Default dark and light popovers differ from the canvas. The
full frame is painted, tool half-block edges inherit the conversation surface,
and question slots and toast padding no longer expose terminal background.

Centered pickers identify their purpose, retain navigation hints during search,
and explain empty results without clearing the query. Short layouts keep an
item row visible; tiny layouts never place the picker caret outside the frame.
Verified five surface tests and five picker behavior tests, the 309-test TUI
gate, strict all-target clippy, CLI build, architecture and generated themes.
Reviewed headless captures and resolved the tiny-caret regression before commit.

### 2026-10-10: reachable TUI help and task-flow assessment

Help now scrolls with arrows, page keys, Home/End and the mouse wheel. Its
footer stays visible, resizing clamps the viewport, reopening starts at the
top, and narrow layouts retain the close hint. Key labels no longer run into
their descriptions. Help uses the same raised surface as the other overlays.

Verified four help behavior tests at 24/40/80/160 columns, all 309 TUI tests,
strict all-target clippy, CLI build, formatting, architecture and theme codegen.
Breaker, simplifier and operator rechecks found no required issues after fixes.
Normal commit hooks also passed workspace clippy; its existing dependency
future-compatibility notices remain unrelated to this pass.

Recorded the independent task-flow/rendering assessment and design context.
Remaining priorities are conversation-owned follow-ups, persistent shared task
observation and recovery, confirmed project context, attention navigation and
review. The shared-client-core versus existing-adapter choice remains pending;
no architecture migration or live native interaction was performed.

### 2026-10-10: TUI popover and activity rework

Centered pickers now show one row per choice and separate selected details in a
page-scrollable viewport. Search follows the Unicode caret, selection fills its
row, narrow layouts retain actions, and the thinking budget track uses actual
zero-based mouse coordinates. Search undo/redo resets details even when query
length does not change.

Permission, persona and goal dialogs use content-based sizing and pinned
decisions. Long and multiline bodies remain reachable through keys and the
wheel, resizing preserves/clamps offsets, and close hints fit narrow terminals.
The goal editor shares the composer's visual rows, reserves its insertion caret,
and owns pasted text and undo without touching the conversation draft.

Inline autocomplete measures the space actually drawn, including the clipped
landing screen. Its heading, completion keys and full selected row stay useful
through scrolling and resizing. Truncation keeps grapheme clusters whole.

The sidebar appears from 112 columns and leads with current work, waiting,
retrying or stopping state. Successful/cancelled work leaves the activity view
immediately; recent failures retain their reason for the configured TTL. Todos
and goals keep their authoritative records. Remaining/blocked counts, independent
scrolling, accurate header hit rows and clean restarts replace stale clutter.
Secondary environment and companion sections respect saved expansion choices.

Verified 343 TUI tests, strict all-target TUI clippy, CLI build, architecture and
theme checks. Headless captures covered 24/30/40/80/120 columns. Independent source
reviews caught full-row caret and equal-length undo cases; regressions pass.
Buffer and raster pixel checks confirmed selection colors. Shared transport
migration, passive cross-client observation and live native QA remain pending.

### 2026-10-10: authoritative todo edits and attachment snapshots

Todo edits now carry the session, request and expected original task. The daemon
preserves IDs/dependencies, rejects stale edits and invalid completions, persists
atomically, and acknowledges the saved list. Tool saves and publication share the
canonical todo lock so older snapshots cannot overwrite or publish stale work.
Attaching or creating a session sends its current todo snapshot after readiness.
Idle todo updates also reach the daemon client's notification channel.

Verified 29 agent todo tests, 63 daemon library tests, four real WebSocket
regressions, protocol roundtrips, 26 TypeScript client tests and its build.
Relevant strict clippy passed; mobile/shared clients compile. Protocol revision
remains 2, with a bounded failure when an older daemon cannot confirm an edit.

### 2026-10-10: visible pasted prompts and question decisions

The composer shares its layout with mouse hit testing and gives long drafts
priority on short landing screens. Paste normalizes line endings, inserts in one
undo step, and goes to the visible editor. Large-paste confirmation counts
characters and preserves command mode. Read-only popovers hide the chat caret.

Questions follow the Unicode answer caret, preserve full multiline answers,
and reserve the answer and cancel rows under long prompts/choices. Review pins
submit/cancel below clipped previews. The allow-tool panel is opaque and raised
without an extra outline.

Verified 14 paste regressions, existing question and surface tests, and the full
382-test TUI suite. Workspace strict clippy, CLI build and architecture checks
passed. Deterministic captures were retaken at 128 columns by 40 rows, producing
1280 by 800 images with normal terminal text.

### 2026-10-10: inspectable activity and editable todos

Sidebar task, subagent and job rows open session-owned detail views. `/todo`,
`/agents` and `/jobs` provide keyboard access at every width. Completed activity
leaves the sidebar while its open viewer retains the outcome. Details use actual
received progress, command, timing and failure data; unavailable/session-changed
states are explicit. Mouse hits follow visible wrapped rows and scrolling.

The todo browser includes finished records and edits description/status against
the expected original task. Save acknowledgment replaces canonical state;
errors retain the draft. Unicode/multiline editing, undo, caret scrolling,
status/save hints and visible-row selection work on narrow terminals. Permission
and question interruptions preserve edits. Idle todo notifications refresh the
list, and confirmed attachment clears the old session's sidebar work.

Verified 17 todo regressions, eight activity regressions and 22 runtime dispatch
tests. The full 382-test TUI suite, strict workspace/all-target TUI clippy, CLI
build, architecture and generated-theme checks pass. Independent reviews covered
backend publication and editor interruption/session ownership. The 16-view local
screenshot gallery is notes/capture/tui-20261010/index.html at 1280 by 800 pixels.
Shared transport migration and live native cross-client QA remain pending.

### 2026-10-10: subagent event delivery and durable tool traces

The current subagent_start event pump forwards child tool events, progress and
permission requests instead of dropping them. Permission replies retain their
original channels. Child text is scoped to the subagent rather than an unmatched
parent tool ID. Completed, denied and aborted tool states are recorded as
assistant revisions; session readers keep the latest revision at its original
position so resuming does not duplicate messages.

An explicit exit_tool now ends the child turn after recording its final tool
state and result. The runner drains that turn before reporting completion,
avoiding an extra provider request and a final-trace race. Orphan recovery uses
the recorded exit result. Verified 192 agent tests, 19 session tests and strict
all-target clippy. Real tools and temporary files exercised permission decisions
and saved results. Parent/child mailboxes and worker resume remain absent.

### 2026-10-10: scoped child transcript reads

The daemon exposes read-only child history requests scoped to the attached
parent, child and request ID. Parent and child metadata must agree on membership;
child IDs are validated before path access. The reader supports the runtime's
bare-ULID parent storage and checked external daemon-ID records, ignores only an
incomplete trailing append, and rejects malformed complete records. Replies go
only to the requesting connection, including during an active parent prompt.

Verified protocol roundtrips, read scopes, two real WebSocket regressions, 64
daemon library tests and 27 TypeScript client tests/build. Mobile/shared clients
compile and scoped strict clippy passes. Reads do not wait for the turn lock or
switch attachment. Older daemons fail through the viewer's bounded timeout.

### 2026-10-10: borderless subagent conversation viewer

Activity panels keep their raised color without an extra outline. Subagent
details include the recorded assignment, child messages, available reasoning,
tool arguments, results and errors. Running views refresh saved turns every two
seconds; repeated refresh presses coalesce behind one request. Errors/timeouts
retain the last snapshot and allow explicit retry. Replies must match session,
child and request, and an old child's completion cannot finish a replacement.

Trace wrapping is cached across frames and rebuilt for new records or width.
Refresh preserves scroll; narrow layouts retain refresh/close hints. The real
TUI drain now routes wheel bursts to the visible editor/view. Daemon capture
Enter dispatches the visible action and quiet turns process trace notifications
without needing another agent event.

Verified 17 activity tests and the full 391-test TUI suite, 22 dispatch tests,
seven real daemon-capture tests, workspace/all-target TUI strict clippy, CLI
build, architecture and generated-theme checks. The gallery's subagent and job
views were retaken at 1280 by 800 pixels. Recorded-turn viewing does not provide
a parent/child mailbox or restore missing outputs in older transcripts.

### 2026-10-10: adaptive picker height and wheel routing

Inline command/reference choices grow to three fifths of the terminal when
needed, preserving composer/status space and shrinking for fewer matches.
Landing autocomplete clips above the anchored input. Centered pickers use their
available content budget instead of stopping at eight rows; selected details
and navigation remain reserved. Existing proportional scrollbars indicate
overflow and reach both ends. Wheel input over slash choices now navigates
those choices instead of scrolling the conversation underneath.

Verified five new behavior regressions and the full 396-test TUI suite,
all-target TUI strict clippy, CLI build, architecture and generated-theme checks.
Three independent review lenses found no required issues. The screenshot
gallery includes expanded commands, overflow at the end, file references,
short results, landing suggestions and a short terminal. Desktop captures
remain 1280 by 800 pixels. Live interaction remains pending.

### 2026-10-10: shared question data and option descriptions

Question and option data live in the shared message crate so subagent runners
can carry them without depending on the agent runtime. Existing agent imports
continue to work. The actual question tool retains option descriptions,
including Unicode and multiple lines, while absent descriptions remain empty.
Selected and freeform answers keep their existing result format.

Verified the new real-tool regression failing on the prior description loss and
passing after the fix. An isolated staged snapshot passed all 193 agent tests,
strict agent/message clippy, and CLI, daemon, mobile and shared-core compilation.

### 2026-10-10: rendered mouse geometry and picker interactions

Terminal mouse coordinates stay zero-based and hit testing uses the visible
chat, choice rows, detail viewport and scrollbar thumbs from each frame.
Wrapped reasoning headers keep subsequent selection rows aligned. Picker
clicks select visible choices, wheels stay local to the list or details under
the pointer, and scrollbar track paging and thumb dragging reach both ends.
Resizing clears stale hits; modal ownership and release end prior grabs.

Verified 17 actual-frame mouse regressions, rendered budget and picker wheel
fixtures, related picker, autocomplete and activity behavior tests, and strict
all-target TUI clippy. The tests cover narrow and resized terminals, Unicode,
wrapped text, sidebar widths, landing and chat composers, and modal ownership.

### 2026-10-10: scoped submissions and retained daemon activity

Wire revision 3 accepts prompts with explicit session and request identities.
Duplicate submissions return their existing receipt, and reconnects can query
the original session before deciding whether to retry. Busy sessions reject
without echoing the draft. Cancellation is registered before acceptance and
agent locking; cancelled accepted input persists without requesting a provider
or generating a title. Follow-ups wait for turn bookkeeping, while optional
title generation releases that turn's lock. Receipts remain runtime metadata.

Session attachment restores authoritative subagent and shell-job activity,
including child identity, current status and missed completions. Snapshot
publication is serialized with activity updates. The wire also retains optional
child-question source data and an explicit close flag, with legacy constructor
and match compatibility and TypeScript event forwarding.

Verified the isolated commit snapshot: 194 agent tests, 73 daemon tests, 122
protocol tests and 29 TypeScript client tests passed. One existing daemon browser
test requires Chrome and remains ignored. Formatting and all-target daemon,
mobile, shared-core and UI-model compilation passed with a separate build target.

### 2026-10-10: shared client activity and error ownership

The shared client retains shell jobs, provider retry status, task child identity,
request-scoped todo/history replies and authoritative activity replacements.
Typed events preserve exact prompt provenance and part updates. Turn errors
carry their session identity; command errors keep their existing connection
semantics. Session lists keep latest activity order and attention counts even
when attention arrives before the list. Background failures stay unread until
their session is viewed. Display metadata snapshots omit accumulated transcript
and streaming text while retaining their session's activity.

The engine coalesces pending session-list refreshes, requests fresh catalog data
after confirmed attachment and refreshes lists on turn and metadata changes.
Failed prompt sends no longer create optimistic history. Native activity
snapshots trigger the existing session projection, with minimal raw mobile
constructor compatibility for the shared question-source field.

An isolated commit snapshot passed all 66 shared-core tests, 12 UI-model tests,
formatting and strict all-target shared-core, UI-model and mobile clippy. The
native activity snapshot projection regression passed with the installed CEF
SDK. Thirteen actor tests passed separately, including real daemon offscreen
conversation and completed-turn ordering against persisted metadata.

### 2026-10-10: persistent terminal conversations and child questions

The TUI and daemon capture now use one persistent shared-core client and typed
commands. Confirmed conversations own their drafts, queued follow-ups, undo
history and workspace discovery. Prompt presentation waits for exact acceptance
and user-message provenance. Reconnect restores activity and pending questions,
checks uncertain receipts without resubmission, and keeps transport failures
visible. Passive updates use canonical history with incremental display parts.

Conversation links and notifications show current work and unread outcomes,
retain archived recovery, and use authoritative activity order. Conversation and
project details format the daemon's millisecond timestamps. Child questions keep
their actual owner through forwarding and reconnect; explicitly closing one
resolves its channel without cancelling the parent turn. Noninteractive handling
and the native answer constructor follow the same protocol contract.

Verified an isolated foundation snapshot with its own build target: 13 persistent
client tests, 27 dispatch tests and 52 client/draft/navigation/todo tests passed.
The real daemon regression checks offscreen discovery, completed-turn order and
project catalog timestamps against persisted metadata. Formatting and strict
all-target CLI/TUI clippy passed. The complete current TUI pass also passed 461
tests across 18 targets, with architecture and generated-theme checks green.

### 2026-10-10: child question ownership across native clients

Native desktop, mobile and browser question prompts retain the worker name and
child identity. Closing a child question sends its request's explicit close flag;
the parent turn and unrelated input requests remain pending. Mobile snapshots
restore the same source data after reconnect. Browser activity snapshots replace
stale task and job state only for the confirmed conversation.

Verified the mobile replay/close regression with a real endpoint, all 125 web UI
tests and its production build, and all 154 native desktop tests across the suite
and the socket test's permitted rerun. Rebuilt matching iOS device, simulator and
host libraries; regenerated bindings matched the prepared changes byte for byte.
All 32 AppStore tests passed on iPhone 17 with iOS 26.4, including child ownership
and preserving the parent request when closing the child question.

### 2026-10-10: bounded job output and idle completion

Job output reads preserve UTF-8 byte offsets, session and request identity, and
copy at most 64 KiB per response. Shell completion waits for final output with a
one-second reader drain bound, reports incomplete output explicitly, and retains
nonzero exit codes while presenting failed jobs as failed. Idle sessions publish
job completion and preserve the full command through status updates and replay.
Weak completion observers stop on session teardown. Detached runtimes retain live
jobs, while reattached clients stop obsolete reapers. Ownership confirmation now
precedes implicit activity for new and attached conversations.

Verified six real shell regressions, the chunk wire roundtrip and all 30 client
tests. The isolated backend candidate passed the full daemon suite, including
real idle success/cancellation/exit-7, reconnect, reaper and attach-order tests,
and compiled the CLI and mobile client without the pending frontend changes.

### 2026-10-10: full output and session review in the terminal companion

The activity panel now opens full background-job output, canonical tool
arguments/results/errors and workspace diffs. /tools selects recorded tool calls;
/review and /diff request changes from the confirmed conversation's workspace.
Chunk replies, errors and review invalidation stay bound to their session and
request. Long output scrolls beyond 65,535 rows, Unicode chunk boundaries continue
without delay, and empty polls preserve the cached rendering while refreshing
status. The existing composer draft and conversation history remain intact.

Verified the exact isolated frontend candidate with eight viewer tests, 19 client
projection tests, 29 dispatch tests, two shared output-event tests, the actual
frame/cache regression and command registry parity. The complete current TUI
suite passed all 462 tests across 18 targets; renderer captures are 1280 by 800.

### 2026-10-10: acknowledged prompts and scoped child questions

This foundation checkpoint predates the later native and iOS verification.

Wire revision 3 adds explicit session/request identities and acceptance receipts
for prompt submissions. Duplicate IDs return the original result, reconnects can
query the original conversation, and an unknown receipt requires manual history
inspection. Busy sessions reject without echoing unsent text. Follow-ups wait for
finished turn bookkeeping, and automatic title generation releases the turn lock.
Receipts are runtime metadata and do not survive daemon restart or session reaping.

Child questions now retain their original answer channel, option descriptions,
and task/child identity through runner, daemon and every client. Closing a child
question preserves parent actions; cancelling a waiting child persists its tool
result and resolves matching pending cards. Shared client state retains jobs,
retry waits, exact part updates, latest-active ordering and unread attention.

Verified the real runner regressions, 194 agent tests, 13 daemon request tests,
63 shared-core tests, protocol roundtrips, 28 TypeScript client tests and 124 web
UI tests. Strict workspace clippy, architecture and theme checks passed. Native
compilation used the installed CEF 150.0.11 SDK; regenerated Swift bindings, app
module and changed tests typechecked at the project's simulator target. Native
and mobile runtime interaction remain pending. UniFFI's emitted Swift/header
whitespace is preserved rather than rewriting unrelated generated lines.

### 2026-10-10: task-flow verification and screenshot gallery

The terminal companion now preserves conversation drafts and follow-ups, uses
confirmed project context, and exposes latest-active conversations through the
sidebar and attention bell. Full output and workspace review replies are
projected only into their matching session and request. The user-facing command
and shortcut documentation describes the new navigation and viewers; runtime
and protocol guidance follows the persistent client actor and scoped receipts.

Verified 462 TUI tests across 18 targets, architecture rules and generated theme
checks. Native desktop passed 154 tests across the regular run and an isolated
socket regression rerun. The web UI passed 125 tests and its production build.
The actual iOS simulator suite passed all 32 tests after rebuilding matching
Rust libraries and regenerating UniFFI bindings. Thirty renderer captures are
available in notes/capture/tui-20261010/index.html, including eight new task-flow,
output, review and child-question views at 1280 by 800 pixels. Captures use sample
state; paired live interaction remains pending.

### 2026-10-10: compact conversation states and picker chrome

Conversation rows in the picker and sidebar use one state icon and their title.
Quiet/read conversations use an empty circle, unread updates a filled circle,
questions a question mark, approvals an exclamation mark and running work a play
marker. Selected details retain project/time context, pending counts and failures,
including persisted failures without an error message. Repeated quiet-state,
unread and cost copy is removed from the conversation picker.

Centered pickers retain special actions and detail scrolling hints while omitting
the generic arrow/Enter/Escape navigation footer. Conversation choice counts and
the project picker's duplicate title footer are removed. Overflow fixtures now
use enough choices to exercise scrolling with the recovered space.

Observed the new icon, detail and footer regressions fail before their fixes.
All 467 TUI tests across 18 targets passed, with generated themes up to date.
Refreshed the eight task-flow captures and visually checked the conversation and
attention views at 1280 by 800 pixels. Updated command documentation with the
icon meanings; live interaction remains pending.

### 2026-10-10: project context on every conversation

Conversation pickers and the sidebar place project paths beneath each title,
with pending action counts and time aligned on the right. Home paths use `~/`
with component-safe boundaries; search accepts home aliases and absolute paths.
Unknown projects remain explicit. Shortened paths preserve their project suffix,
and selected details disclose clipped context without repeating ordinary rows.

Both lines route to their conversation. Picker offsets and scrollbar paging count
choices rather than physical rows, and resizing retains complete pairs where
space permits. Short terminals reserve scrollable context and failure details;
the scrolling hint follows the actual detail viewport. Selected context retains
the shared highlight with lighter typography than its title.

All 481 TUI tests across 18 targets passed, including path boundaries, Unicode,
project search, row clicks, sidebar scrolling, track paging and narrow details.
Generated themes are up to date. Refreshed eight desktop captures with several
projects and added a 400 by 320 pixel short-terminal view to the screenshot
gallery. Visually checked conversation, attention and narrow views using the
actual renderer with sample state; paired live interaction remains pending.

### 2026-10-10: shaded user messages and composer

User messages use an opaque full-width surface instead of a `>` prefix. The
composer uses the same `chat.user.background` token without its idle marker;
cursor layout, multiline editing and the active-turn spinner retain their
existing geometry. Attachments share the user surface, while guidance keeps its
steering arrow. Selection colors remain visible over user messages.

Wide glyphs now paint their covered cells with their leading cell's style so
message and selection backgrounds remain continuous in rendered frames. The new
theme token aliases `muted`, with palette overrides for Catppuccin Latte and
Tokyo Night to maintain at least 4.5:1 body-text contrast in every built-in theme.
Theme outputs were regenerated from the manifest and documentation updated.

All 486 TUI tests across 18 targets passed, including new surface, attachment,
guidance, wide-character selection and palette contrast checks. Refreshed the
text goldens and renderer captures, adding dark, light and short user-message
views to the gallery. Generated theme checks passed; live interaction remains
pending.

Follow-up found during capture: `/theme` replaces the theme without invalidating
transcript or markdown caches, which can retain colors from the previous theme.
This needs a separate pass; the capture fixture clears those caches explicitly.

### 2026-10-10: shared Unix input editing

The composer and inline reference picker now use the same editor as picker
filters, question answers, goals, todo drafts, history search and settings.
Line kills preserve other lines, word movement skips separators, Ctrl+W uses
whitespace boundaries, and movement/deletion/transpose preserve whole graphemes.
A shared twenty-entry kill ring supports Ctrl+Y yank and Alt+Y rotation;
consecutive kills combine without crossing fields. Undo remains local to each
field, and cursor movement separates typing groups without copying the buffer.

Ctrl+P/N recall composer history and navigate picker results; Ctrl+T transposes.
The palette uses Ctrl+Space, the bell Ctrl+Alt+N, reasoning Ctrl+Alt+T, and
conversation archive/pin actions Ctrl+Alt+A/P. Help, visible hints, the reasoning
golden and shortcut documentation match those bindings. Shift/Alt+Enter and
Ctrl+J insert newlines in multiline editors; modified Enter cannot accidentally
confirm filters or dialogs. Unknown modified characters do not become prompt
text, and modified digits cannot replace thinking budgets.

Regression fixes cover Unicode-whitespace Ctrl+W panics, inline-reference
Ctrl+D editing the wrong buffer, stale history-search cursors, settings caret
initialization and undo leaking between fields. Empty Ctrl+D keeps an active
turn open; Ctrl+X cannot trigger the unmodified child-cancellation shortcut.
All 531 TUI tests across 19 targets passed, including 45 added behavior tests;
focused clippy passed. Refreshed renderer captures and added three desktop and
narrow help images to the gallery. Live interaction remains pending.

### 2026-10-10: copy transcript text without terminal layout

Chat selection now copies source content bounds instead of padded screen rows.
Soft wraps join without adding spaces or newlines, including long Unicode words;
real paragraph breaks, code indentation and tool-output whitespace remain intact.
Partial and reversed selections exclude generated margins and surface fill.
List and quote continuations omit repeated display prefixes, and redundant blank
separators do not add extra copy breaks.

The markdown renderer records copy bounds and break provenance while rendering,
including pending blocks and incremental caches, without rendering a second time.
Existing rendering APIs remain available, and visual goldens are unchanged.
All 541 TUI tests across 20 targets and 42 markdown-renderer tests passed, with
22 new regression tests covering copy content, streaming, resizing and mouse
release. Strict checks passed for both crates. Rendered tables retain their grid
formatting and interior column spacing. Live interaction remains pending.

### 2026-10-10: remove the composer prompt gutter

The composer no longer reserves two columns for the removed prompt marker.
Text retains one cell of surface padding, and the streaming spinner uses that
padding without moving the draft. Rendering, wrapping, mouse positioning and
vertical keyboard movement all use the same input layout width.

All 543 TUI tests across 20 targets passed, including landing/chat alignment
across idle and streaming states, the recovered wrap space, Unicode boundaries,
one-column input, queued drafts and resizing. Existing visual goldens passed.
Refreshed renderer captures and inspected desktop and short composer views.
Live interaction remains pending.

### 2026-10-11: expose authoritative turn outcomes to presentation clients

The shared client reducer emits session-scoped completion and failure events,
including for the attached conversation. Presentation clients can distinguish
attachment from actually viewing a result without changing the existing unread
behavior used by other frontends. Authoritative failure details retain precedence;
permission and question alerts do not become turn outcomes.

All 72 client-core tests passed, including outcome delivery, failure-detail
reconciliation and existing activity parity. Client-core Clippy passed with
warnings denied.

### 2026-10-11: terminal-local conversations, attention and titles

Each TUI process keeps a confirmed list of conversations open in its window.
A single view stays quiet; multiple views expose an Open here sidebar and picker.
Global history remains available through Tab. Ctrl+Alt+O opens the local list,
Ctrl+Alt+Left/Right switch views, Ctrl+Alt+Enter starts a conversation in the
current project, and Ctrl+Alt+W closes a view. Closing a background view leaves
daemon work running and retains its draft, attachments and queued follow-ups
within the TUI process. Closing the current view waits for its replacement to
attach successfully; closing the last view exits the frontend.

The bell counts only this window's conversations and separates work needing
input from successful unread results. Failures remain actionable until another
turn starts. Focus events acknowledge visible results at the latest transcript
position; overlays, scrollback and pending attached history prevent premature
acknowledgement. Terminals without focus reporting wait for actual interaction.
Raw notifications from other windows no longer create local toasts.

Terminal titles identify the current conversation, home-relative project and
work, question, approval, failure, unread or connection state. Background counts
include only views open here. Titles change only when their text changes;
control sequences and bidi controls are removed and Unicode titles remain
bounded without splitting graphemes. Help and shortcut documentation match.

All 564 TUI tests across 21 targets and 199 CLI tests across three targets passed.
A real daemon/WebSocket regression covers two independent windows, unfocused
completion and closing an accepted running turn without cancelling or losing
its persisted response. Strict Clippy, architecture, generated-theme and format
checks passed, and the CLI built. Inspected wide and narrow renderer captures
under /private/tmp/mew-window-ux-captures. Live terminal focus reporting and OSC
title display remain pending interactive verification.

### 2026-10-11: restore build prerequisites and native release packaging

CI, nightly and release builds install the pinned Zig 0.15.2 toolchain used by
Ghostty. Web checks run on Node 24, matching the TypeScript test runner and the
workspace engine requirement. Linux ARM CLI artifacts now build on a native
Ubuntu ARM runner, avoiding cross-target OpenSSL discovery. Intel macOS builds
use the supported macos-15-intel runner. Workspace Linux checks install the
terminal and native desktop link dependencies. Desktop releases install CMake,
Ninja and checksum-verified standalone agent-browser 0.34.0 assets for each
architecture before assembling the app.

Native packaging selects the CEF version pinned by cef-dll-sys, skips older
cached frameworks and rejects incompatible explicit overrides. The selected
SDK's credits stay inside the app; release archives use those bundled credits
and reject stale frameworks or missing notices. Five filesystem packaging
regressions and five workflow prerequisite checks cover these failure modes.
The companion-plugin example's unused placeholder and lint warnings were removed;
two behavior tests retain its seeded identities and one-time intro behavior.
Native build and archive prerequisites are documented in the CEF host README.

The release CLI, GPUI client and CEF helper built on ARM macOS. The assembled
app includes CEF 150.0.14, standalone agent-browser and five executable helper
bundles. ZIP and DMG exports are under target/release/dist. SHA256 hashes, ZIP
CRCs and notices, DMG checksum and read-only mounted app contents all passed.
The assets remain unsigned and unnotarized; nothing was installed or published.

The complete just ci gate passed: 2,463 Rust test cases across 133 targets,
30 web-client tests, format, strict Clippy, architecture, generated themes,
web production builds and the real daemon/bridge subprocess round trip. Four
existing Rust tests remain ignored. Targeted packaging and companion-example
Clippy checks also passed. GitHub Linux and Intel runner execution remains
pending a fresh remote workflow run; terminal focus and title live checks are
still separate from build verification.

### 2026-10-11: rerunnable TUI UX gallery and window snapshots

The ux_capture example renders 51 current UI states through the actual TUI
renderer and bundled rasterizer fonts. Fixture modules cover terminal-local
conversation navigation, scoped notifications, global history, activity and
trace viewers, todo and goal editors, approval and question dialogs, paste,
readline input, review and both dark and light user-message surfaces. Desktop
captures are 1280 by 800 pixels; narrow captures use actual smaller viewports.
The dated gallery and companion text frames are under the ignored directory
notes/capture/tui-20261011, preserving prior captures. The example and build
command are tracked and documented so a fresh checkout can regenerate them.

Four new checked-in goldens pin the quiet single-view layout, multiple local
views, scoped notification picker and global history. The related golden and
surface suites passed, and all TUI tests passed in the full CI gate. Targeted
example and golden Clippy checks passed. All PNG signatures, CRCs, decoded
streams and dimensions were verified; two gallery builds had identical image
hashes. Representative wide and narrow images were inspected, including the
permission surface and detailed subagent viewer. These captures use deterministic
sample state; live focus, title, input and real browser interaction checks remain
pending.

### 2026-10-11: validate the documentation site in CI

The orchestration assessment lacked Starlight's required title metadata, causing
an actual site build to fail while the Rust and web-client gates passed. Added
title and description frontmatter without changing the assessment text. GitHub
web CI and just ci now build the documentation site as well as the application
clients, covering schema validation for the repository docs symlink.

The site build reproduced the missing-title error before the fix, then passed
and produced 38 pages, search and sitemap. The new workflow regression failed
before the site-build step was added; all six workflow tests and targeted strict
Clippy now pass. Cloudflare's check exposes only a generic failure and its
dashboard requires login, so the exact remote failure is not yet confirmed.

### 2026-10-11: respect externally configured Zig caches

Both PR Rust jobs installed Zig successfully but then failed to find libsimdutf.a.
setup-zig sets both cache environment variables to the workspace cache, while
the vendored Ghostty build searched only the source checkout's default cache.
The build now passes an explicit Cargo-owned local cache directory to Zig and
searches that same directory for dependency archives. The global cache remains
shared for downloaded dependencies; target build artifacts stay scoped to
OUT_DIR. Cargo tracks the relevant Zig environment and the correct build.rs path.

A real fresh-source regression reproduced the exact missing-archive panic before
the fix, then installed all four static libraries and vt.h after it. Direct and
normal Cargo runs passed with externally configured local and global caches;
the external local cache was untouched. The opt-in native regression includes
run instructions and requires Zig, the pinned source checkout and a populated
global dependency cache. Changed source and test files passed strict Clippy and
format checks. The workspace Rust and JavaScript suites passed under the CI cache
configuration; a separate subprocess startup timeout is being diagnosed before
claiming a clean full gate.

### 2026-10-11: deterministic fake-daemon startup and subprocess diagnostics

The fake-provider daemon no longer loads real model catalogs before binding its
listener. Its provider and model list do not consume catalog data, so this removes
unnecessary network and cache work from the deterministic subprocess harness.
A cold-cache real-daemon regression failed first because catalog storage was
created, then passed without creating it after the fix.

Subprocess readiness now reports early child exit status and stderr. Logs use
owned temporary files so a timeout can report a still-running child's output
without waiting for inherited pipes to close. Real shell-child regressions cover
early exit and a live stalled process, alongside the actual daemon/bridge/WebSocket
round trip. No startup timeout was increased. The original local timeout's exact
cause was not captured by the old harness, so it is not attributed to a specific
permission or catalog failure.

All three subprocess tests, 12 daemon-policy/status tests and targeted strict
Clippy passed. The complete just ci gate then passed with both Zig cache variables
configured as in GitHub, including the new site build. The separate fresh-source
Ghostty regression also passed through Cargo. Remote checks will run after this
push; Cloudflare deployment diagnostics remain unavailable without dashboard
access if the new preview still fails.

Separate follow-up: the workspace ignores Cargo.lock, and CI resolved some newer
dependency versions than the local build. No dependency-locking policy was changed
in this fix.

### 2026-10-11: keep native window-handle imports platform-scoped

The next Ubuntu Rust job built Ghostty and CEF successfully, then exposed unused
window-handle imports in the desktop shell. Those types are consumed only by the
macOS parent-view helper. Moved the imports inside its existing macOS-gated
function, keeping the Linux shell free of unused platform names.

All 154 desktop tests and all-targets Clippy with warnings denied passed locally;
format and diff checks passed. The real lifecycle listener test required loopback
socket permission. Cloudflare deployment and web CI passed on the preceding
9ded149 commit; the scoped import fix will be verified in the next Rust CI run.

### 2026-10-11: stabilize boolean template output across MiniJinja versions

macOS CI reached tests and exposed a fresh dependency behavior change: MiniJinja
2.24 renders scalar booleans as True/False, while the local 2.21 resolution and
existing prompt templates use lowercase true/false. The shared renderer now
writes lowercase boolean literals directly and delegates other values to the
normal formatter. Context values remain typed booleans, preserving conditionals.
No dependency version was pinned and existing assertions were not relaxed.

The model-switch regression reproduced the exact initial persona prompt on 2.24
before the fix. Tests cover both boolean values, both predicate branches, valid
JSON-shaped authored templates, strings, numbers and explicit escaping. All 476
related prompt, agent and CLI tests and strict Clippy passed with 2.24. All 64
prompt tests and the original model-switch regression passed with 2.21; both
versions support the final direct-write API. Assertion diagnostics retain the
actual initial and switched prompts if this regresses.

The complete just ci gate passed with the fresh 2.24 local resolution and the
external Zig cache environment. Its ignored lockfile was backed up and now uses
2.24 for reproducible local checks against this CI failure. Source changes do not
alter the repository's dependency-locking policy. The preceding remote run passed
Clippy on both Rust platforms; the updated formatter will be checked after push.

### 2026-10-11: check existing shell paths by their canonical destination

Ubuntu CI exposed an unsafe lexical fallback in shell workspace checks. After an
existing path resolved outside the configured roots, the original path could
still be accepted because its prefix began inside a root. Removed that fallback;
existing paths and roots are compared by their canonical destinations.

Both parent-directory and outside-target symlink regressions failed locally
before the fix after canonicalizing their temporary fixture roots. All 130
configuration tests and all-targets strict Clippy passed. Coverage includes bash,
background and monitor commands, aliased workspace roots, and legitimate direct
files and in-workspace symlinks that must remain allowed. Missing-path policy is
unchanged.

Separate follow-up: the existing configuration backup test writes an unused
state.toml.test-original file beside the user's config. It does not change
state.toml, but its fixture should use isolated storage in a separate pass.

### 2026-10-11: make the desktop home-path fixture portable

macOS CI correctly displayed an absolute session path belonging to another user,
exposing a fixture that assumed HOME was /Users/natalie. The fixture now reads the
same HOME value as the path renderer and uses different session and workspace
paths, preserving the exact ~/code/mew assertion and proving cwd takes precedence.

The focused fixture and all 154 desktop tests passed. The complete just ci gate
passed with both path fixes: 2,472 Rust test executions, 30 web-client tests,
strict Clippy, formatting, architecture, generated themes, subprocess end-to-end
checks and the 38-page documentation build. Native checks used the existing
matching CEF 150.0.14 SDK. The next remote run will verify both platforms.

### 2026-10-14: per-plan names and no CURRENT.md agent directives

Removed the active CURRENT.md instructions from the repository guidance and
built-in builder persona. The default plan location is now `plans/`: each
`write_plan` call creates a collision-safe two-word filename using resona,
returns its path, and `edit_plan` takes that path to revise any generated plan.
Explicit fixed plan paths retain their overwrite/edit behavior. Generated plan
paths are kept as references across context compaction.

Verified the plan-tool tests, workspace-scoped agent tests, mew-agent,
mew-config and prompt tests, formatting, architecture checks, and strict
Clippy for the changed Rust crates. The full workspace CI gate was not run.

### 2026-10-17: update pinned GPUI and preserve frame tracing

Updated gpui and gpui_platform from ae394f3d to upstream revision 322cd509.
Enabled GPUI's profiler feature and adapted the desktop frame probe to the
unified trace toggle and draw/present event collector. A regression test uses
real profiler events to verify draw durations, absent invalidation timestamps,
invalidation counts, presentation exclusion, and collection without duplicates.

The baseline passed all 154 desktop tests; the updated suite passes 155. The
native desktop build and full macOS just ci gate passed: 2,477 Rust test
executions, 30 JS tests, strict Clippy, formatting, architecture, generated
themes, subprocess end-to-end checks, and the 38-page documentation build.
An isolated native smoke test verified rendering, composer text entry, sidebar
mouse and keyboard actions, retained text, and actual 60-frame trace reports.

Native checks used the matching CEF SDK at target/cef-sdk. The first full gate
exhausted disk space during test compilation; removing only generated artifacts
created during this task and retrying with two compiler jobs completed the gate.
The existing block crate future-compatibility and web bundle-size warnings remain.


## 2026-10-17 - Ghostty 0.2.2 native API

Upgraded the safe terminal wrapper to exactly 0.2.2 and aligned the existing
vendored bindings and generator with its pinned Ghostty C revision. This takes
the upstream render-setter pointer fix instead of masking invalid dirty-state
values in the desktop. Unused Kitty graphics features remain disabled.

The native archive now includes its SIMD dependencies, so the build links the
single archive and preserves Cargo-scoped Zig outputs. Validation matched all
173 exported bindings to the native headers and passed the real external-cache
rebuild plus a linked, executed FFI probe. The production desktop build passed.


## 2026-10-17 - Native terminal input and projection

Restricted direct control bytes to single ASCII keys, retained named-key
modifiers, and made idle selection/theme changes invalidate and notify the
terminal. Ghostty now formats selections by terminal cells; projection retains
full Unicode clusters and consumes wide tails without shifting later columns.
Outside release ends selection without sending unrelated releases to the PTY.
The new wrapper owns callback stability, so the extra terminal box is removed.

All 15 terminal regressions passed, including dirty-state reset, Unicode and
wide-cell copy, cached-row reuse, real GPUI notification and outside release.
Native smoke also rendered Unicode PTY output and retained a composer draft
after dragging a terminal selection into another pane.


## 2026-10-17 - Cancellable desktop connection lifecycle

Client shutdown now cancels blocked WebSocket command writes. Supervisor retry
runs outside the GPUI foreground thread, preserves current drafts, ignores
obsolete profile/generation completions and prevents duplicate retries. Shared
child ownership lets quit synchronously kill and reap a daemon during startup
or before completion reaches the UI. Failed OS worker creation restores the
supervisor configuration for another attempt.

The 14 supervisor tests and eight desktop lifecycle tests passed with real
localhost handshakes, socket backpressure, subprocesses and an actual failed
thread creation followed by a successful retry. Existing daemons remain
attach-only. Tests explicitly permit external-thread wakeups in GPUI's harness.


## 2026-10-17 - Painted text geometry and wrapped composer sizing

Native IME range/caret bounds now reuse the focused shell field's painted
shaping, converting UTF-16 ranges and rejecting stale text or field geometry.
Caret placement before explicit newlines stays on its logical line. Composer
height counts visual wrapped rows before applying its existing height cap.

All ten composer tests passed, including actual paint-to-input-handler geometry
and a narrow paragraph that previously received 48px instead of 78px. Native
smoke verified Unicode paste, visible wrapped rows and retained drafts. Native
OS IME candidate UI has not been exercised; terminal IME remains unsupported.


## 2026-10-17 - Native failure feedback and quit shortcut

Deferred browser initialization now notifies before returning an unavailable
runtime or startup error. Expected missing-editor probes are silent. The native
Quit menu now registers Cmd-Q; GUI smoke found that the shortcut was absent.

Real GPUI/subprocess regressions cover visible browser failure notification and
quiet application discovery. The missing quit binding reproduced in the real
GPUI key map before the binding fix passed. An isolated fake-provider native
conversation streamed successfully and displayed the browser-unavailable state.


## 2026-10-17 - Isolated desktop CI configuration

The existing macOS default-config-path test now checks the actual resolver in
a subprocess without MEW_CONFIG_DIR or XDG_CONFIG_HOME overrides. This preserves
the default-path assertion while letting full CI isolate desktop settings and
sessions from the user's data. The prior assertion failed under the isolated
environment; the focused regression and full isolated CI gate then passed.


## 2026-10-17 - Mew-specific GPUI reference and project skill

Refreshed the active guides against GPUI revision
322cd509dcccaee1f6bc44725d270598ae8171ad and current desktop ownership, APIs and
verification paths. The gpui-desktop project skill routes to those maintained
guides, and AGENTS.md directs native desktop work through it. GPUI Community
Edition is a secondary example source; historical Zedra-only effects, mobile
presentations and HTTP devtools remain clearly separated from supported APIs.

The skill validator, an independent field/theme use-case check and all local
reference links passed. The full isolated macOS CI gate passed 2,511 Rust test
executions, 30 JS tests, strict workspace Clippy, architecture/generated-theme
checks, subprocess end-to-end checks and the 38-page documentation build. Final
checks additionally passed all 187 desktop tests, 14 supervisor tests, three
native ABI tests, strict Clippy across affected test targets and the native
production build. The external Zig-cache regression rebuilt the real library
and executed a statically linked FFI probe.

Isolated GUI smoke covered Unicode paste and PTY output, wrapped composer sizing,
fake-provider streaming, selection release into another pane, retained drafts,
visible browser unavailability and Cmd-Q from a focused field. Quit exited
cleanly without stopping the attach-only daemon; the disposable daemon and
app bundle were then removed. Full packaged CEF rendering and native OS IME
candidate UI were not exercised. The minimal temporary bundle initially found
a CEF framework without helper assets; the successful smoke explicitly selected
the unavailable-runtime path. Existing dependency/tooling build warnings remain.

Optional follow-ups are to measure settings theme discovery and shell/transcript
render reuse in a release build before changing their caching boundaries.

## 2026-10-17 - Packaged CEF pump and native editing ownership

The fully bundled macOS app reproduced a SIGTRAP when its first confirmed
conversation initialized CEF inside GPUI's running AppKit loop. Embedded CEF now
uses the external message pump, matching the retained GPUI pump and avoiding
native run-loop observers installed halfway through an iteration.

CEF's visible native first responder now receives editing key equivalents before
the GPUI host view. Command arrows and deletion follow the same route as the
clipboard shortcuts, including AppKit's Function flag on navigation events.
Global mew commands retain their original window handler. Browser roots are weak
references and only a focused visible descendant can receive the handoff.

The real AppKit regression reproduced host interception before the fix and now
passes focus, visibility, declined-command, repeated-registration and global
shortcut cases. All 17 host/browser tests and strict Clippy passed. Packaged CEF
painted a local fixture, accepted native input/clicks and scrolling, resized with
the host window, retained distinct page state across conversations and exited
cleanly. Clipboard commands additionally require the native Edit menu; that
application-menu correction is verified separately from the responder bridge.
