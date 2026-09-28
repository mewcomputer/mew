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
