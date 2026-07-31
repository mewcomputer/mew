# 2026-07-30 — Re-fix Kimi K3 tool-call response mismatch (jobblock:80)

## Summary

The previous fix changed `content: null` to `content: ""` for assistant messages with `tool_calls`, but Kimi (and other OpenAI-compatible providers) was still rejecting follow-up requests with:

```text
an assistant message with 'toolcalls' must be followed by tool messages
responding to each 'toolcallid'. The following toolcallids did not have
response messages: jobblock:80
```

Two issues could leave the assistant message without a matching tool response:

1. **Cancelled tool turns**: if a user cancelled while a tool was running, the agent aborted the turn without appending the tool-result message, leaving a broken conversation history for the next request.
2. **Ordering bug in the OpenAI adapter**: if a user message somehow carried both text and a `ToolResult`, the adapter emitted the user text *before* the `role: tool` messages, violating the rule that tool messages must immediately follow the assistant message that issued the tool calls.

## Changes

- `crates/mew-agent/src/turn.rs`
  - When a turn is cancelled after the assistant message has been appended but before/while tools finish, the agent now still appends a matching `ToolResult` message for every pending tool call.
  - Unprocessed tool calls are marked as errored, and the updated assistant message is synced back into the store instead of being appended a second time.

- `crates/mew-provider-openai/src/lib.rs`
  - In user messages that contain both `ToolResult` parts and text/image content, emit the `role: tool` messages first, then the user message. This keeps the tool responses immediately after the assistant message even if a message also carries text.
  - Added `test_build_wire_message_tool_results_before_user_text` to lock in the ordering.

- `crates/mew-agent/src/tests.rs`
  - Added `test_cancelled_tool_turn_appends_tool_results` to verify that a cancelled turn leaves a valid history (user, assistant tool-call, tool result).

## Verification

- `cargo test -p mew-agent` — 142 tests pass.
- `cargo test -p mew-provider-openai` — 9 tests pass.
- `cargo test --all` — all pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.
- `just arch-check` — pass.

# 2026-07-26 — Fix duplicated thinking duration across reasoning blocks

## Summary

The TUI was showing the same elapsed thinking time for every reasoning block in the transcript. The elapsed duration was stored in a single global `Option<Duration>` on `App`, so once any reasoning block finished, all collapsed reasoning headers rendered that same duration.

## Changes

- `crates/mew-tui/src/app/mod.rs`
  - Replaced `pub reasoning_elapsed: Option<Duration>` with `HashMap<PartId, Duration>` so each reasoning block keeps its own elapsed time.
  - Added `record_reasoning_elapsed(collapse: bool)` helper that records the active reasoning block's duration and optionally collapses it.
  - Finalize the active reasoning block when a new reasoning part starts, a text/toolcall part starts, the reasoning part ends (`PartEnd`), or the message ends (`MessageEnd`).
  - Only collapse the reasoning block when the model moves on to a text or toolcall part, preserving the existing behavior where a final reasoning block stays expanded.

- `crates/mew-tui/src/ui/chat.rs`
  - Rendering now looks up the elapsed duration for each reasoning part by its `PartId` instead of reading a global value.
  - Added `test_reasoning_headers_use_per_part_elapsed` to verify that two reasoning blocks in the same message show their own recorded durations.

## Verification

- `cargo test -p mew-tui` — 159 unit tests + 5 golden tests pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.

# 2026-07-26 — Fix Kimi K3 (OpenAI adapter) tool-call validation errors

## Summary

Kimi K3 was rejecting follow-up requests after a tool call with:

```text
an assistant message with 'toolcalls' must be followed by tool messages
responding to each 'toolcallid'. The following toolcallids did not have
response messages: bash:14
```

The OpenAI adapter was sending assistant messages with `content: null` and empty `reasoning`/`reasoning_content` fields. Some OpenAI-compatible providers (including Kimi) reject that combination when `tool_calls` is present, causing the server to fail validation before it reaches the tool response message.

## Changes

- `crates/mew-provider-openai/src/lib.rs`
  - For assistant messages with empty content, emit `""` instead of `null`.
  - Omit `reasoning` and `reasoning_content` fields when the reasoning string is empty; only echo them back when non-empty reasoning was actually streamed.
  - Added `test_build_wire_message_tool_result_pair` to verify that an assistant message with a tool call and a user message with the matching tool result produce a valid request body.

## Verification

- `cargo test -p mew-provider-openai` — 8 tests pass.
- `cargo test -p mew-agent` — 141 tests pass.
- `cargo clippy --all -- -D warnings` — clean.

# 2026-07-26 — Fix mew-prompts transclude and make it render nested content

## Summary

Four mew-prompts tests were failing because `transclude(...)` returned raw file contents without rendering the nested Jinja directives inside them. The system prompt (`base.md`) was recently split into provider-specific partials via `{{ transclude(...) }}`, but the `transclude` function only inserted the raw text. This left literal `{{ transclude(...) }}` directives in the rendered system prompt and broke tests that expected text from the rendered partials.

## Changes

- `crates/mew-prompts/src/template.rs`
  - Changed the `transclude` function to take the current minijinja `State` and render the included content against a clone of the original template context, so nested directives resolve recursively.
  - If the included content fails to render, it falls back to the raw content with a warning.

- `crates/mew-prompts/src/persona.rs` and `crates/mew-prompts/src/template.rs`
  - Updated the four transclude tests to assert on text that actually exists in the current rendered system prompt (`"Treat the current prompt context as authoritative"`).

## Verification

- `cargo test -p mew-prompts` — 52 tests pass (was 48 passing + 4 failing).
- `cargo test --all` — all pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.
- `just arch-check` — pass.

# 2026-07-26 — Fix startup crash on unknown provider and make config loading consistent

## Summary

`mew` was crashing on startup with:

```text
Error: build provider

Caused by:
    unknown provider zhipuai-coding-plan
```

The provider resolution in `async_main` was using a config loaded in the top-level startup path, while `chat_cmd` and `run_cmd` reloaded config independently before building the provider. If the two loads ever diverged (e.g. an environment-only provider present during resolution but missing at build time), the resolved provider could be passed to `build_provider` and fail as unknown. The error message also gave no hint about where the config was loaded from or how to fix it.

## Changes

- `crates/mew/src/main.rs`
  - Pass the already-loaded `Config` from `async_main` into `chat_cmd` and `run_cmd` instead of letting them reload it.

- `crates/mew/src/commands/tui.rs`
  - `chat_cmd` now accepts `cfg: mew_config::Config` and uses it directly (no second `mew_config::load()`).

- `crates/mew/src/commands/run.rs`
  - `run_cmd` now accepts `cfg: mew_config::Config` and uses it directly.

- `crates/mew/src/setup/providers.rs`
  - Improved the `build_provider` "unknown provider" error to include the config file path, the list of available providers, and a pointer to `state.toml` so the user can clear stale persisted state.

## Verification

- `cargo test --all` — all pass.
- `cargo clippy --all -- -D warnings` — clean.
- `cargo fmt -- --check` — clean.
- `just arch-check` — pass.

# 2026-07-25 — Add Gleam language support to code highlighter and hashline block resolver

## Summary

Added Gleam (`.gleam`) support to two systems:
1. **`ratatui-mdstream`** — ```` ```gleam ```` code fences are now syntax-highlighted. `two-face` 0.5 does not bundle a Gleam syntax definition, so a custom `gleam.sublime-syntax` is embedded and merged into the `SyntaxSet` at init time via `SyntaxSet::into_builder()` + `SyntaxDefinition::load_from_str()`.
2. **`mew-hashline`** — `SWAP.BLK` / `DEL.BLK` / `INS.BLK.POST` block resolution now works for `.gleam` files via the `tree-sitter-gleam` grammar.

Changes:
- `crates/mew-hashline/Cargo.toml` — added `tree-sitter-gleam = "1.0"` dependency.
- `crates/mew-hashline/src/block.rs` — registered `("gleam", tree_sitter_gleam::LANGUAGE.into())` in `build_language_table()`; added `resolve_gleam_function_block` test.
- `crates/ratatui-mdstream/resources/gleam.sublime-syntax` — new TextMate-style syntax definition covering Gleam keywords, types, strings, numbers, comments, constants, operators, and function calls. Written from scratch (not copied from any GPL source).
- `crates/ratatui-mdstream/src/highlight/syntect.rs` — `syntax_set()` now extends two-face's `SyntaxSet` with the embedded Gleam definition; added `test_gleam_syntax_available` and `test_gleam_highlighting_produces_styles` tests.

Verification:
- `cargo test -p mew-hashline` — 57 passed (including new Gleam block test).
- `cargo test -p ratatui-mdstream` — 22 passed (including 2 new Gleam tests).
- `cargo test --all` — all pass, 0 failures.
- `cargo clippy --all -- -D warnings` — clean.
- `just arch-check` — pass.
- `just theme-codegen-check` — pass.
- `cargo fmt -- --check` — clean on changed files.

Notes:
- `two-face` 0.5 confirmed (via test program) to NOT include Gleam, hence the custom syntax file.
- The sublime-syntax uses standard TextMate scope names (`keyword.control`, `storage.type`, `constant.numeric`, `entity.name.function`, etc.) that the existing `theme.tmTheme` already maps colors for.

# 2026-07-19 — Switch Kimi to OpenAI adapter + fix reasoning_content deserialization

## Summary

Kimi K3 thinking was silently dropped. Two issues, both fixed:

1. **Kimi was wired to the Anthropic adapter**, but Kimi's Anthropic-compatible
   endpoint doesn't reliably stream thinking content blocks. Moonshot's OpenAI
   surface is their primary, fully-tested API. Switched the `kimi` provider
   shape from `"anthropic"` to `"openai"`. Base URL unchanged
   (`https://api.kimi.com/coding/v1`); the OpenAI adapter appends
   `/chat/completions`, which matches.

2. **The OpenAI adapter only deserialized `reasoning` in streaming deltas**,
   but Kimi K3 (and DeepSeek) emit reasoning under `reasoning_content`. Added
   `#[serde(alias = "reasoning_content")]` to the `Delta.reasoning` field so
   both field names are accepted. The outbound path already wrote both names.

## Changes

- `crates/mew-config/src/lib.rs`: kimi provider `shape` → `"openai"`, comment
  updated, test assertion updated.
- `crates/mew/src/setup/providers.rs`: removed `"kimi"` from the anthropic arm
  of `provider_name_to_shape` (falls through to default `"openai"`), shape test
  and fallback description updated.
- `crates/mew-provider-openai/src/lib.rs`: `#[serde(alias = "reasoning_content")]`
  on `Delta.reasoning`. New test `test_fixture_reasoning_content_alias` with
  fixture `src/testdata/reasoning-content.sse` verifying reasoning is captured
  from `reasoning_content` deltas.

## Additional fix: model picker Right-arrow now switches the model

When pressing Right on a model in the picker to open the thinking variant
picker, the agent's active model wasn't switched. So `set_thinking` resolved
the variant against the *old* model, failing with "unknown thinking variant
for model". Now Right fires a `SwitchModel` action alongside opening the
variant picker; since actions are processed sequentially, the model switch
completes before the user selects a variant.

- `crates/mew-tui/src/events.rs`: `handle_picker_key` Right handler returns
  `Action::SwitchModel(id)` instead of `None`.

## Additional feature: "Recent Models" section in model picker

The model picker now shows a "Recent" section at the top with up to 6
previously used models, followed by an "All Models" section with the full
list. Recent models are persisted in `state.toml` and survive restarts.

- `crates/mew-config/src/lib.rs`: added `recent_models: Vec<String>` to `State`.
- `crates/mew-tui/src/app/mod.rs`: added `recent_models` field to `App`,
  `header` field to `PickerItem` (with `Default` derive), `move_selection()`
  on `PickerState` to skip headers, and updated `filtered()` to hide headers
  when a filter is active.
- `crates/mew-tui/src/app/pickers.rs`: `open_model_picker` prepends recent
  models with section headers; `picker_up`/`picker_down` use `move_selection`.
- `crates/mew-tui/src/ui/overlays.rs`: section headers render as muted, dimmed,
  non-selectable lines.
- `crates/mew/src/runtime/dispatch.rs`: `handle_switch_model` records the
  switched model in `recent_models` (move to front, dedupe, cap at 6) and
  persists to state.
- `crates/mew/src/commands/tui.rs`: loads `recent_models` from state at
  startup in both daemon and standalone modes.
- 6 new tests covering recent section rendering, empty state, unknown model
  filtering, header-skipping navigation, and filter behavior.

## Verification

- `cargo test -p mew-provider-openai` — 7/7 pass (including new test).
- `cargo test -p mew-config test_default_kimi_provider` — pass.
- `cargo test -p mew setup::providers::tests` — 45/45 pass.
- `cargo test -p mew-tui` — 156/156 pass + 5 golden tests pass.
- `cargo clippy -p mew-provider-openai -p mew-config -p mew-tui` — clean.
- Pre-existing clippy dead-code error in `crates/mew/src/commands/daemon.rs`
  (`remote_invite_payload`) is unrelated to this change.

---

# 2026-07-18 — Surface real credential diagnostics instead of bare "get credential"

## Summary

Running `mew` with no credential env vars but a `credentials.json` present
failed with an unhelpful two-line error:

```
Error: build provider

Caused by:
    get credential
```

Root cause: `build_direct_provider` called `get_credential(...).ok()`,
discarding the rich `CredentialNotFound` error that `get_credential` returns
(an error that already names the exact env var, the keyring command, and the
`credentials.json` path). Each API-key shape arm then replaced the swallowed
error with a bare `.context("get credential")?`, producing a causeless
"get credential" string with no diagnostic. The user couldn't tell whether
the lookup missed the key, read a malformed file, or referenced the wrong
`credential_ref`.

Fix: stop swallowing the error. `get_credential` now returns the `Result`
directly into the match. The `openai` and `anthropic` arms propagate it with
`?`, so the full `CredentialNotFound` message reaches the user. The
`responses` arm keeps the OAuth-first behavior (a missing API key is
non-fatal there) but captures the credential error message and appends it to
the final "no credentials for codex" error when every auth path fails.

## Changes

- `crates/mew/src/setup/providers.rs`: `build_direct_provider` holds the
  `Result<String, ConfigError>` instead of `.ok()`; `openai`/`anthropic`
  arms use `?` to propagate the cause; `responses` arm captures the error
  message for the all-paths-failed diagnostic.
- Added two tests guarding the regression for both `openai` and `anthropic`
  shapes: `build_provider_missing_credential_surfaces_diagnostic` and
  `build_provider_missing_credential_anthropic_shape_surfaces_diagnostic`.
  They assert the error chain contains the env var name, "credentials.json",
  and "credential not found" — the swallowed-error regression produced none
  of these.

## Verification

- `cargo test -p mew --bin mew setup::providers::tests` — 44 passed.
- `cargo clippy -p mew --all-targets -- -D warnings` — clean.
- `cargo fmt -p mew -- --check` — clean.
- `cargo test -p mew-config` — 118 passed (no downstream breakage).

## k3 reasoning verification

Traced the full Kimi K3 thinking path end-to-end and confirmed it is correct:

1. Catalog (`mew-catalog`) produces variants `low`/`high`/`max` for any model
   id containing `k3`, each with `params: {"reasoning_effort": <effort>}`.
   `default_thinking` selects `high`.
2. `resolve_reasoning` returns the variant's params as a `ReasoningConfig`.
3. The agent clones it into the `Request.reasoning` field each turn.
4. The Anthropic adapter's `build_request_body` iterates `reasoning.params`
   and inserts each key at the top level of the JSON body — so
   `reasoning_effort` lands top-level, which is where Kimi's API reads it.
5. The `thinking.budget_tokens` bump does not fire for k3 (no such key), so
   no Anthropic-style thinking block is injected.

Added `test_anthropic_adapter_forwards_reasoning_effort_top_level` in
`mew-provider-anthropic` to lock in steps 4–5: asserts `reasoning_effort`
appears top-level in the wire body and that no `thinking` object is injected.

## Kimi tool-call ID sanitization

Kimi's Anthropic-compatible API emits tool-call IDs containing spaces and
colons (e.g. `"handoff plan:29"` — tool name + space + colon + counter).
Anthropic's `tool_use` ID format rules reject these on the next-turn replay,
producing:

```
an assistant message with 'toolcalls' must be followed by tool messages
responding to each 'toolcallid'. The following toolcallids did not have
response messages: handoff plan:29
```

The API generates the non-conformant ID itself, then rejects its own ID when
we replay it. Both mew adapters previously took the provider's call ID
verbatim with no sanitization.

Fix: in the Anthropic adapter's `tool_use` ingest arm, replace every incoming
`content_block.id` with a fresh `toolu_`-prefixed ULID before storing it in
`ToolCallPart.call_id`. The fresh ID round-trips consistently — the agent
matches tool results to calls by `call_id`, and `build_request_body`
serializes both `tool_use.id` and `tool_result.tool_use_id` from that same
field, so the pair always matches.

The OpenAI adapter is unaffected (Kimi uses the `anthropic` shape).

### Changes

- `crates/mew-provider-anthropic/src/lib.rs`: `tool_use` ingest arm now
  assigns `call_id: format!("toolu_{}", ulid::Ulid::new())` instead of
  copying `event.content_block.id`. The `ContentBlock.id` field is still
  deserialized (for raw-dump mode) but marked `#[allow(dead_code)]`.
- `crates/mew-provider-anthropic/src/testdata/tool-call-nonconformant-id.sse`:
  fixture with a `"handoff plan:29"` tool-call ID.
- `test_fixture_tool_call_nonconformant_id_is_sanitized`: asserts the
  resulting `call_id` is `toolu_`-prefixed and contains no spaces or colons.

### Verification

- `cargo test -p mew-provider-anthropic` — 16 passed.
- `cargo clippy -p mew-provider-anthropic --all-targets -- -D warnings` —
  clean.
- `cargo fmt -p mew-provider-anthropic -- --check` — clean.
- `cargo build -p mew` — clean (one pre-existing unrelated warning in
  `daemon.rs::remote_invite_payload`).

## Shift+Tab / Ctrl+Shift+Tab persona cycling

Added keyboard cycling for personas in the TUI:

- **Shift+Tab** — cycle forward through loaded personas, wrapping through
  "default" (no persona) at the end.
- **Ctrl+Shift+Tab** — cycle backward.

Terminals deliver Shift+Tab as `BackTab` and Ctrl+Shift+Tab as `BackTab` with
the `CONTROL` modifier. The normal-mode key handler maps these to
`Action::CyclePersona(+1)` / `Action::CyclePersona(-1)`, which dispatches
through `handle_cycle_persona` → `handle_switch_persona` — reusing the
existing switch path so model pinning, accent color, and the synthetic
display message all fire identically to `/persona <name>`.

The keybinding is suppressed when the input box has text (BackTab is a no-op
there anyway) and in slash-command mode (where Tab does completion). When no
personas are loaded, it sets an "no personas loaded" alert.

### Changes

- `crates/mew-tui/src/events.rs`: `CyclePersona(i32)` Action variant;
  `BackTab` handler in `handle_normal_key`.
- `crates/mew/src/runtime/dispatch.rs`: `handle_cycle_persona` computes the
  next persona from `app.personas` + `app.active_persona` and dispatches
  through `handle_switch_persona`.
- `crates/mew-tui/src/harness.rs`: `parse_key` now supports `shift+` prefix
  and maps `shift+tab` / `ctrl+shift+tab` to `BackTab` for test input.
- `crates/mew/src/dispatch_table_tests.rs`: `CyclePersona` arm in the
  variant table; four new tests covering forward, wrap-to-default,
  backward-from-default, and empty-list alert.
- `crates/mew-tui/src/ui/overlays.rs`: help overlay entry for Shift+Tab.

### Verification

- `cargo test -p mew --bin mew dispatch_table_tests` — 12 passed.
- `cargo test -p mew-tui` — 5 passed + 1 doc test.
- `cargo clippy -p mew-tui --all-targets -- -D warnings` — clean.
- `cargo fmt -p mew -p mew-tui -- --check` — clean.

## Fix: Right key not opening thinking variant picker from model picker

Pressing Right in the model picker to open the thinking variant picker was
silently failing. Two bugs:

1. **Key mismatch (standalone mode):** The model picker uses `provider/model`
   format IDs (e.g. `opencode-zen/claude-sonnet-4-6`), but the
   `thinking_variants` HashMap is keyed by the bare model id (e.g.
   `claude-sonnet-4-6`). The Right-key handler's `contains_key(&selected.id)`
   always missed because of the provider prefix.

2. **Daemon mode never populated:** `ModelList` messages from the daemon were
   forwarded to the notification channel but never parsed into `app.models`
   or `app.thinking_variants`. The model picker was empty in daemon mode.

### Changes

- `crates/mew-tui/src/app/pickers.rs`: Added `open_thinking_variant_picker_for`
  which accepts an optional model id (in `provider/model` format), strips the
  provider prefix via `rsplit('/')`, and looks up that model's variants — not
  the current active model's. The old `open_thinking_variant_picker` delegates
  to it with `None` (uses `self.status.model`).
- `crates/mew-tui/src/events.rs`: Right-key handler now strips the provider
  prefix before the `contains_key` check, and passes the selected model id to
  `open_thinking_variant_picker_for` so the picker shows variants for the
  highlighted model, not the current one.
- `crates/mew-tui/src/app/mod.rs`: `apply_daemon_notification` now handles
  `ModelList` — populates `app.models` (picker items) and
  `app.thinking_variants` (keyed by bare model id from `ModelInfo.model`).
- `crates/mew-daemon/src/client.rs`: Added `list_models()` method.
- `crates/mew/src/commands/tui.rs`: Daemon-mode TUI now calls
  `client.list_models()` on startup.
- Tests: `test_thinking_variant_picker_strips_provider_prefix` and
  `test_thinking_variant_picker_for_bare_model_id`.

### Verification

- `cargo test -p mew-tui` — 7 passed (5 existing + 2 new).
- `cargo test -p mew-daemon` — 5 passed.
- `cargo test -p mew-protocol` — 106 passed.
- `cargo clippy -p mew-tui --all-targets -- -D warnings` — clean.
- `cargo fmt -p mew-tui -- --check` — clean.

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

## 2026-07-18 — Browser workbench chrome cleanup

- Removed the generic Workbench title/subtitle when a browser tab is active so
  the surface reads like an actual browser window.
- Restyled the shared workbench tab strip into compact browser chrome with
  rounded active tabs, while keeping Activity, Files, Changes, Review, and
  Terminal tabs in the same registry.
- Made the URL and page-action rows browser-like, centered the page identity,
  and let the native browser viewport fill the remaining surface without a
  nested card inset.
- Kept the generic workbench summary header for non-browser surfaces.

Verified: 101 web UI tests, TypeScript build, production UI build, and
`git diff --check`.

## 2026-07-18 — CEF navigation pump re-entrancy fix

- Diagnosed the Google navigation freeze from a live debug process: the CEF
  renderer helpers stayed alive, but `curl http://127.0.0.1:9223/json/version`
  connected and then hung. A macOS process sample showed CEF recursively
  entering `do_message_loop_work` through Tauri's inline
  `run_on_main_thread`, while CEF already held its browser-process mutex.
- Added a pump gate so nested CEF turns are skipped, and dispatch on-demand
  pump callbacks from a worker before handing them back to the Tauri main
  thread. This prevents callbacks raised during native view visibility or
  navigation work from re-entering CEF synchronously.
- Kept the 30 ms backstop for callbacks coalesced during an active turn and
  added a regression test for the pump gate.

Verified: 12 Tauri host tests, cargo formatting check, and `git diff --check`.
The running debug process must be restarted to load this fix.

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

## 2026-07-18 — Native Tauri workbench menu boundary

- Added a macOS AppKit `NSMenu` path for the workbench `+` menu. It is
  anchored to the React control and renders above the native CEF child view,
  while the regular web host keeps the existing HTML menu and does not expose
  a built-in browser.
- Kept workbench tab state and surface selection in React. Native menu actions
  return through a Tauri event, so Browser, Terminal, Files, Changes, and
  Review continue to share the same tab registry.
- Added native-menu lifecycle commands, close events, and a web-host no-op
  test. Failed native menu setup falls back to the HTML menu.
- Hardened the CEF/Tauri boundary by installing the two CEF macOS application
  selectors onto Tauri's existing `TaoApp` instead of replacing its
  `NSApplication` class. CEF child views now start hidden until an active tab
  claims and sizes them.

Verified: 102 web UI tests, UI TypeScript check, native CEF tests, 12 Tauri
host tests, `cargo fmt --all -- --check`, and `git diff --check`. A debug
desktop launch stayed alive after the selector and initial-visibility fixes;
the native popup click still needs a clean single-instance desktop smoke run.

## 2026-07-18 — Browser connection lifecycle guard

- Fixed restored browser tabs throwing `not connected` during the first render
  while the websocket client was still connecting.
- Browser daemon commands now wait for the shared connection state, and the
  browser controls stay disabled while disconnected instead of surfacing a
  React error boundary.
- Native macOS CEF navigation now takes precedence once CEF availability is
  resolved, so a disconnected daemon does not block the embedded browser.
- Added a regression test for restored browser tabs during daemon disconnect.

Verified: 103 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`. A desktop smoke launch was blocked by an existing Vite
process already listening on port 5173; the active Tauri host did stay alive
and spawned a daemon on port 25566.

## 2026-07-18 — Browser navigation deduplication guard

- Kept restored-tab recovery tied to connection changes without making normal
  URL submission navigate twice.
- Added an assertion covering the single-send web browser open path.

Verified: 103 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Chat rendering stability and overflow guard

- End-anchored the virtualized conversation and batched resize observations so
  measured message heights stop pulling the viewport around during streaming.
- Limited live-store subscriptions to the currently streaming message and
  memoized message rows, reducing token-by-token rerenders across the visible
  conversation.
- Added stable scrollbar space and min-width/overflow boundaries across chat,
  markdown, code, reasoning, and tool surfaces so the conversation cannot
  create page-level horizontal scrolling.
- Added regression coverage for completed rows staying stable during live
  streaming updates.

Verified: 105 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — macOS XDG-style config directory

- Changed macOS global configuration storage to `~/.config/mew`, matching the
  Linux-style location used by other CLI tools.
- Kept the change non-migrating: existing Application Support data remains in
  place and is no longer selected by the default resolver.
- Updated configuration and session path documentation and added a macOS path
  regression test.

Verified: 118 `mew-config` tests, Rust formatting, and `git diff --check`.

## 2026-07-18 — Files root navigation

- Fixed the Files surface sending `/` to the daemon when navigating back to
  the workspace root.
- Kept root navigation represented as an omitted relative path, preserving the
  daemon’s absolute-path safety check.
- Added regression coverage for parent and join path handling.

Verified: 107 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Workbench surface picker

- Replaced the web add-tab dropdown with a searchable shadcn command picker
  showing every workbench surface, icon, description, and shortcut.
- Preserved the native macOS picker for the CEF host so the menu stays above
  the native browser surface, while adding the same complete option set.
- Added jsdom layout shims and picker coverage for keyboard-oriented cmdk
  behavior.

Verified: 105 web UI tests, UI TypeScript check, production UI build, Rust
formatting, and `git diff --check`.

## 2026-07-18 — Browser omnibox and native tools menu

- Combined the browser URL and page chrome into one omnibox-style row with a
  single submit affordance and loading state.
- Moved snapshots, screenshots, selector interaction, and hide-page controls
  into a secondary Browser tools surface so the browser viewport keeps its
  height.
- Added an AppKit browser-tools menu for macOS, routing native menu actions to
  the active CEF tab so the controls stay above the native browser view.
- Kept the web popover fallback for non-native browser rendering and covered
  the new one-row and tools interactions.

Verified: 105 web UI tests, UI TypeScript check, production UI build, Tauri
`cargo check`, Rust formatting, and `git diff --check`.

## 2026-07-18 — Composer surface simplification

- Flattened the composer into one bounded surface, keeping the message field,
  attachment state, persona/model controls, and send/cancel action in a single
  visual hierarchy inspired by the Codex composer.
- Kept session telemetry in the separate status footer so connection and token
  information does not compete with message composition.
- Added a regression test ensuring the composer controls remain inside the
  primary surface.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Floating dock scroll containment fix

- Restored flex-column containment around the session chat after moving the
  composer to the full-surface overlay.
- This gives the virtualized and fallback scroll containers a bounded height
  again, preserving chat scrolling while keeping the dock independently
  positioned.

Verified: 106 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Unified modular workbench tabs

- Promoted Plan, Agents, Questions, and Jobs into first-class workbench tabs
  alongside Browser, Terminal, Files, Changes, and Review.
- Removed the nested Activity tablist and its duplicate tab state, leaving one
  modular tab interface for every right-rail surface.
- Kept core activity tabs pinned, migrated persisted legacy Activity tabs to
  Plan, and preserved actionable-tab selection when the workbench opens.
- Updated tab, persistence, and right-rail regression coverage.

Verified: 107 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Workbench header removal

- Removed the redundant Workbench title/subtitle header so the tab strip is the
  first visible workbench surface.
- Moved the close action into the tab row, preserving access without restoring
  another explanatory panel.

Verified: 106 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Composer containing-block correction

- Restored explicit parent-relative height on the workspace row and both
  conversation insets so the bottom dock resolves against the full app surface
  instead of a content-sized containing block.
- Kept the earlier full-surface fade and bottom dock positioning intact.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Full-surface composer anchoring

- Re-anchored the floating composer to the complete session surface instead of
  the chat column, so it reaches the actual bottom edge beneath the footer.
- Added a non-interactive bottom fade that eases the underlying chat and status
  details into the page background without adding another panel row.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Floating composer treatment

- Removed the full-width composer panel border and background so the chat
  surface continues behind it.
- Kept the composer itself elevated with a focused border and restrained
  shadow, preserving the single-surface hierarchy and all existing controls.

Verified: 106 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — Positioned floating composer

- Moved the composer into an absolute bottom dock inside the session surface,
  removing it from normal chat layout flow while keeping the status footer
  independent below it.
- Added transparent pointer-through space around the dock so only the composer
  and pending interaction cards capture input.
- Added bottom scroll clearance to both virtualized and fallback chat surfaces,
  keeping the latest message visible above the floating dock.

Verified: 106 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Workspace shell sizing correction

- Changed the conversation inset from viewport-relative `h-screen` sizing to
  parent-relative `h-full` sizing in both mobile and desktop layouts.
- Preserved the rounded inset surface while keeping it inside the enclosing
  `h-svh` workspace frame, preventing the inner shell from becoming taller
  than the outer app surface.

Verified: 105 web UI tests, UI TypeScript check, and `git diff --check`.

## 2026-07-18 — On-demand workbench tabs

- Removed Agents and Jobs from the default workbench so the rail opens as an
  intentional empty tool area until the pinned summary exists.
- Kept Agents and Jobs available as explicit optional tabs through both the web
  and native macOS add-tab menus.
- Migrated the previous pinned Agents/Jobs defaults out of persisted state and
  added an empty-workbench affordance instead of rendering a blank surface.

Verified: 105 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Main request flow and focused workbench tabs

- Kept Plan approval and Questions in the main session interface, including
  desktop AskUser rendering beside the composer.
- Reduced the workbench to Agents, Jobs, Browser, Terminal, Files, Changes,
  and Review. Browser retains its own internal browser-tab model.
- Migrated persisted Activity, Plan, and Questions rail tabs out of the active
  tab model while preserving Agents and Jobs as pinned core tabs.
- Moved pinned file context into the Files surface and removed duplicate plan,
  question, and cross-session attention panels from the workbench.

Verified: 105 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Read-only Files workbench

- Replaced the narrow Files inspector with a lightweight editor workbench:
  resizable explorer, lazy directory expansion, file filtering, and internal
  read-only document tabs.
- Kept workspace paths relative at the UI boundary and preserved the existing
  external-editor action.
- Added focused behavior coverage for directory expansion, relative file
  opening, filtering, empty states, and document tabs.
- Kept dotfiles hidden until the daemon preview path is routed through the
  secret-file permission checks.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Bound file viewer code width

- Constrained the inner Shiki `<pre>` element, which was still able to impose
  its intrinsic width on the surrounding workbench.
- Made wrapped lines the default for file previews so long JSON and generated
  files cannot create a horizontal layout jump. The toggle remains available
  for intentional horizontal inspection inside the viewer.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Isolate unwrapped file code

- Prevented the highlighted surface and nested token code from contributing
  intrinsic width to the surrounding flex layout.
- Moved unwrapped horizontal scrolling to the highlighted `<pre>` itself so
  disabling wrapping cannot move the workbench.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Flush file preview surface

- Removed the stacked horizontal insets around file contents so the code
  surface reaches the workbench edges.
- Kept chat code block spacing unchanged and retained a small inset for the
  truncated-preview notice.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Fill file preview height

- Let the file code surface use the available editor height instead of ending
  at the last line of a short preview.
- Kept the viewer scrollable and left chat code blocks content-sized.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Tighten file code leading

- Reduced the file viewer line box from 1.55 to 1.35 and applied it directly
  to each highlighted line so dense source files stay compact.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Restore live response streaming

- Fixed provider part appends to replace the assistant message and parts
  immutably instead of mutating an object already held by memoized message
  rows.
- Kept newly started empty text parts renderable so their live stream buffer
  is visible before the completed text is committed.
- Added a regression test for text streaming that begins after reasoning.

Verified: 111 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Restore configured provider models

- Matched catalog provider aliases such as `opencode`/`opencode-zen` and
  `zai`/`z-ai` when building the daemon model list.
- Kept the configured provider id in the picker and switch request so the
  selected model uses the correct endpoint.
- Added coverage for the alias matching boundary.

Verified: targeted Rust provider test, `cargo fmt --all`, and
`git diff --check`.

## 2026-07-18 — Bound file viewer code width

- Constrained the inner Shiki `<pre>` element, which was still able to impose
  its intrinsic width on the surrounding workbench.
- Made wrapped lines the default for file previews so long JSON and generated
  files cannot create a horizontal layout jump. The toggle remains available
  for intentional horizontal inspection inside the viewer.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Bounded file viewer

- Separated file previews from chat code blocks so the editor uses a compact,
  full-height viewer rather than a card that can dictate the workbench width.
- Added editor-style line numbers and preserved syntax highlighting without
  changing message code-block rendering.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — File viewer line wrapping

- Added a per-file Wrap toggle for long lines.
- Kept horizontal overflow contained by default for code readability, with
  wrapped mode available for prose and configuration files.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — File viewer overflow containment

- Added min-width and overflow boundaries across the nested explorer/editor
  flex surfaces so long highlighted files cannot push the entire workbench
  horizontally.
- Kept horizontal scrolling local to the file viewer when wrapping is off.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Nested file pane sizing

- Forced the editor column and highlighted code surface onto shrinkable flex
  bases so files such as large JSON manifests cannot move the whole workbench
  horizontally.
- Kept overflow local to the viewer while preserving the Wrap toggle.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Restore file viewer width

- Corrected the containment pass so vertical editor children retain full
  width while the surrounding horizontal flex boundaries remain shrinkable.
- Restored visible file content without reintroducing workbench-wide overflow.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Bound file viewer code width

- Constrained the inner Shiki `<pre>` element, which was still able to impose
  its intrinsic width on the surrounding workbench.
- Made wrapped lines the default for file previews so long JSON and generated
  files cannot create a horizontal layout jump. The toggle remains available
  for intentional horizontal inspection inside the viewer.

Verified: 110 web UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Add desktop install and daemon cleanup recipes

- Added `just desktop-install` to build and copy the release app into
  `/Applications/mew.app` on macOS.
- Added `just stop-all-daemons` to stop mew bridges and daemon processes on
  the development ports, including stale desktop sidecars.

Verified: justfile dry-run checks and `git diff --check`.

## 2026-07-18 — Add in-app workspace folder browser

- Replaced the native directory dialog with a shared web/Tauri folder browser
  in the new-session project picker.
- Added a daemon filesystem-listing request that starts at the user’s home
  directory, returns directories only, and rejects protected macOS locations
  such as `/System`, `/Library`, `/private`, and `/Volumes`.
- Kept manual path entry for deliberate paths outside the picker’s normal
  browsing boundary.

Verified: protocol and daemon checks, web-client build, UI TypeScript check,
and `git diff --check`.

## 2026-07-18 — Ignore filesystem browsing in TUI capture

- Added the new filesystem directory listing to the TUI capture message
  naming table as an explicit no-op. Folder browsing is owned by the web and
  desktop surfaces and does not change TUI state.

Verified: `cargo check -p mew`, format check, and `git diff --check`.

## 2026-07-18 — Harden folder picker review findings

- Fixed the trailing comma in the Tauri capability manifest.
- Added filesystem boundary tests, protocol round-trip coverage, and a picker
  interaction test covering folder navigation and the parent-folder action.
- Added an in-app up-arrow control that stays within the picker’s home root.

Verified: daemon and protocol tests, picker tests, UI TypeScript check, JSON
validation, and `git diff --check`.

## 2026-07-18 — Recover from unavailable remembered sessions

- Kept the unavailable-session explanation visible while restoring the
  start-new-session action on the home route.
- A stale or daemon-mismatched remembered session can no longer strand the
  production app before the project/folder flow is reachable.

Verified: 112 UI tests, UI TypeScript check, production UI build, and
`git diff --check`.

## 2026-07-18 — Persist desktop daemon logs

- Stopped discarding stdout and stderr from daemons launched by the Tauri
  host.
- Release and configured daemon launches now append to
  `~/.config/mew/logs/desktop-daemon.log` for provider/config/session startup
  diagnostics.

Verified: Tauri cargo check, Rust formatting, and `git diff --check`.

## 2026-07-19 — Preserve full daemon provider errors

- Session creation and resume failures now log the complete anyhow error chain
  and return it to the frontend, instead of collapsing failures to only
  `build provider`.

Verified: daemon cargo check, Tauri cargo check, formatting, and
`git diff --check`.

## 2026-07-19 — Avoid blocking on an uncredentialed remembered provider

- Provider credential lookup now tries the configured credential reference,
  then the provider id for aliased providers such as `opencode-go`.
- Implicit daemon startup falls back to another configured provider with valid
  credentials when the remembered provider is unavailable.
- Explicit provider selections continue to return a clear credential error.

Verified: provider-resolution tests, `cargo check -p mew`, and
`git diff --check`.

## 2026-07-19 — Gate in-app browser tools to desktop sessions

- Added a distinct desktop client kind and made the shared React client
  advertise it only when running inside Tauri.
- Added explicit browser tools backed by the existing `agent-browser`/CDP
  session, with execution-time capability checks and untrusted-page output
  labeling.
- Browser protocol commands now reject TUI, web, CLI, and mobile clients.
- Desktop attachment enables browser tools for the session; detaching the
  final desktop client disables them again.
- Added a persisted system-level capability notice when desktop browser
  access is introduced to an existing session. Notices are deduplicated by a
  capability marker in the persisted message history.
- Added a `System` message role and provider translation for OpenAI,
  Anthropic, and Responses adapters. TUI rendering hides these notices.

Verified: focused Rust tests for message, protocol, tools, agent, daemon
library, and TUI packages; web-client TypeScript build; formatting and
`git diff --check`. The first combined test attempt hit the machine disk
limit while compiling daemon integration tests, so generated Cargo artifacts
were cleaned before the focused rerun.

## 2026-07-19 — Load the bundled web fonts

- Added `@font-face` declarations for the bundled MiSans, Banga, and
  IoskeleyMono assets.
- Pointed Tailwind, body text, headings, code, and typeset typography at those
  real families instead of unbundled placeholder names.
- Added the missing typeset heading fallback token.

Verified: production UI build, UI TypeScript check, and `git diff --check`.

## 2026-07-19 — Add iOS-matched font preferences to web and desktop

- Added the iOS font choices to web settings: System, Mi Sans, Junicode, and
  OFL Goudy.
- Mi Sans remains the default; the selection persists in local storage and
  applies immediately through the shared typography variables.
- Added the matching Junicode and OFL Goudy assets to the web bundle.

Verified: 112 UI tests, production UI build, UI TypeScript check, and
`git diff --check`.

## 2026-07-19 — Add explicit remote daemon and desktop remote modes

- Added a navigable settings surface with a dedicated Remote access section
  and a prominent warning describing remote filesystem, command, session, and
  relay exposure.
- Added `mew daemon --remote`, which keeps the local Unix/TCP listener and
  runs an authenticated iroh listener beside it. The existing `--iroh` path
  remains available for mobile compatibility.
- Added remote protocol authentication, explicit `RemoteScope` enforcement,
  `client_kind=remote` validation, pairing-token loading, and restrictive
  permissions for persisted pairing material.
- Added desktop supervisor support for enabling/disabling app-lifetime remote
  access and shipped the iroh feature in desktop sidecar builds.

Verified: `cargo clippy -p mew-daemon -p mew --features iroh -- -D warnings`,
focused daemon scope tests, iroh integration-test compilation, Tauri host
check, UI TypeScript check, 112 UI tests, production UI build, and
`git diff --check`.

## 2026-07-19 — Harden remote pairing and settings lifecycle

- Reused the existing `RemoteAccessStore` as the pairing boundary instead of
  introducing a second raw-token store. Pairing tokens are short-lived,
  single-use, and persisted only as SHA-256 digests.
- Remote iroh connections can now resolve their granted scope from a pairing
  token or an already paired device. The daemon rejects protocol messages
  until the remote handshake succeeds.
- `mew pair` now creates an invite without binding a second iroh endpoint, so it
  works while `mew daemon --remote` is already running.
- The settings toggle is wired to the Tauri supervisor and refuses to enable
  desktop remote access when the app is attached to an externally owned
  daemon.

Verified: `cargo check -p mew --features iroh`, `cargo test -p mew-daemon`,
`cargo clippy -p mew-daemon -p mew --features iroh -- -D warnings`, Tauri
`cargo check`, UI TypeScript check, focused UI tests, and `git diff --check`.

The final smoke pass also completed `cargo test --manifest-path
mew-web-ui/src-tauri/Cargo.toml supervisor --quiet`, the full web production
build, and `cargo test -p mew-protocol`.

## 2026-07-19 — Close remote lifecycle and protocol review gaps

- Removed the obsolete raw remote-token persistence path. Pairing now has one
  state store, one-time hashed invites, device metadata, expiry, revocation,
  and explicit hosting state.
- Added protocol roundtrip coverage for the remote handshake and made the
  desktop supervisor restore its prior setting if a remote-mode restart fails.
- Desktop-launched remote daemons now persist desktop ownership mode, while
  the CLI remains the long-lived daemon mode. `--iroh` and `--remote` are
  mutually exclusive and their user-facing guidance is distinct.

Verified: focused daemon, iroh, protocol, mobile-core, Tauri, web-client, and
React UI tests/builds; formatting and `git diff --check`.

Final review follow-up:

- Legacy allowlisted iroh clients can no longer claim the desktop-only client
  kind, and control-scope authorization is an explicit current-message list
  that fails closed for future protocol additions.
- Remote state mutations use a short-lived cross-process lock and refresh from
  disk before read-modify-write operations. Pairing readiness, listener failure
  cleanup, and private key/allowlist permission hardening are covered in the
  implementation path.

Verified: daemon checks, iroh integration tests, protocol tests, Tauri check,
formatting, and `git diff --check` after the final review fixes.

## 2026-07-19 — Fix live remote pairing invites

- Included the one-time pairing token in the QR URL consumed by mobile and
  remote clients.
- Refreshed the daemon's file-backed access state during authorization so a
  `mew pair` run can authorize a live `--remote` daemon without a restart.
- Removed the legacy iroh allowlist bypass from explicit remote mode; it now
  requires a pairing token or an active paired device.
- Added regression coverage for invite payloads and cross-process pairing
  refresh.

Verified: focused iroh, daemon, and CLI tests; Tauri check; formatting; and
`git diff --check`.

## 2026-07-19 — Repair daemon listener structure and observe scope

- Restored the missing close for the Unix daemon accept loop after the remote
  capability guard was misplaced during the lifecycle pass.
- Kept observe-only remote access read-only by rejecting `NewSession` and added
  a regression test for that boundary.

Verified: daemon remote tests, iroh integration tests, daemon and CLI clippy,
Tauri check, TypeScript, 112 React tests, and `git diff --check`.

# 2026-07-19 — Invert CEF/WKWebView layering, delete AppKit menus

## Summary

The embedded CEF browser used to paint above the Tauri WKWebView, so any HTML
overlay (settings dialog, cmd+k palette, surface picker, browser-tools
popover) was clipped by the browser rect, and the two menus that had to sit
above the browser were AppKit `NSMenu`s. Implemented PLAN.md: CEF now sits
permanently below a transparent WKWebView, the React app leaves a transparent
hole where the CEF viewport lives, and the native menu paths are deleted in
favor of the existing HTML surfaces on every host.

## What changed

- `mew-web-ui/src-tauri/src/native_layering.rs` (new): `NativeLayeringGuard`
  (pure, unit-tested — orders each CEF native view handle exactly once,
  re-orders when CEF recreates the view) and `order_cef_below_webview`, which
  resolves the WKWebView via Tauri's `with_webview` and the content view via
  `ns_view()`, verifies they share a superview, and calls
  `addSubview_positioned_relativeTo(cef, Below, wkwebview)`. Failures are
  logged, never fatal.
- `native/cef-host/src/embed.rs`: added `CefEmbedController::native_view_handle()`
  (macOS impl + non-macOS stub returning 0).
- `mew-web-ui/src-tauri/src/lib.rs`: ordering is folded into
  `cef_browser_set_rect` with `visible: true` (the owner-claim moment). The
  guard decision and `mark_ordered` both run outside AppKit calls; the guard
  pointer is captured by the main-thread closure as a raw address of the
  managed `CefEmbedState` field, which outlives scheduled callbacks. Deleted
  the 6 `native_*_menu_*` commands, their impl shims, and
  `NativeMenuRectPayload`; removed `mod native_workbench_menu;` and deleted
  `native_workbench_menu.rs`.
- `mew-web-ui/src-tauri/tauri.conf.json`: `"transparent": true` on the main
  window and `"macOSPrivateApi": true` under `app` (required by Tauri for
  macOS webview transparency; blocks Mac App Store distribution, which is
  fine since mew ships directly). `Cargo.toml` gained the matching
  `macos-private-api` tauri feature; `objc2-app-kit` features trimmed to just
  `NSView` (NSMenu/NSMenuItem/NSResponder went away with the menus).
- Frontend: `host.ts` sets `data-host="desktop"|"web"` on `<html>` during
  `initializeHost()` and lost all 8 native-menu wrappers plus the two event
  types. `index.css` makes `body` transparent only under `[data-host="desktop"]`.
- `browser-panel.tsx`: viewport div is `bg-transparent` once CEF reports
  available (muted placeholder otherwise), the closed-browser empty state
  carries its own `bg-muted/20`, and a comment marks the hole as untouchable.
  The native browser-tools path (state, listener effect, button ref) is gone;
  the tools button always toggles the HTML popover.
- `right-rail.tsx`: the "+" button always opens the `CommandDialog` surface
  picker; native menu state/listener/ref deleted.
- `__tests__/host.test.ts`: dropped the native-menu no-op test, added a
  `data-host` attribute test. `right-rail.test.tsx` needed no changes — the
  picker was already the tested path.

## Verification

- `cargo test` on `mew-web-ui/src-tauri`: 15 passed (3 new layering-guard
  tests). `mew-cef-host`: 1 passed. Clippy `-D warnings` clean on both;
  `cargo fmt --check` clean.
- `pnpm --filter mew-web-ui test`: 112/112. `pnpm --filter mew-web-ui build`
  and `tsc --noEmit` clean.
- Not yet done: the live desktop smoke (`just desktop-dev`; hit-testing
  through the transparent hole is the open risk — if WKWebView swallows
  clicks over the hole, the fallback is dynamic ordering for the browser
  region, per PLAN.md risk #1) and `pnpm desktop:verify:cef`. If a desktop
  flash-through shows during resize, paint the NSWindow background to the
  theme color (PLAN.md step 2 contingency).

# 2026-07-19 — Fix inverted transparency: opaque chrome, truly see-through WKWebView

## Summary

First pass at the layering inversion got the transparency backwards: the
body was made transparent so genuine gaps in the page chrome (margins around
the floating session rail and inset panels) showed the desktop, while the
CEF hole itself stayed opaque white because wry's `transparent` flag only
sets the private `drawsBackground` config key — it never sets
`opaque = false` on the WKWebView or clears its layer background (verified
in wry 0.55.1 `wkwebview/mod.rs`; the `setOpaque(false)` +
`setBackgroundColor` path only runs for `background_color`, not
`transparent`).

Fix, both directions:

- `native_layering.rs::make_webview_transparent` (called at the start of
  `initialize_cef`) uses `with_webview` to set `setOpaque(false)` via
  msg_send (WKWebView responds to it; not a public NSView property) and
  clears the layer background color. Now the view composites nothing where
  the page doesn't paint, so the CEF view below shows through the hole.
- CSS: removed the `[data-host="desktop"] body { background: transparent }`
  rule; the body keeps `var(--background)`. The sidebar wrapper in
  `ui/sidebar.tsx` gained an explicit `bg-background` since it was relying
  on the body paint behind it. Only the `browser-panel` viewport hole is
  transparent, so only the CEF rect shows through.

The `data-host` attribute stays (harmless, tested hook for future
host-specific CSS).

## Verification

- mew-desktop: 15/15 tests, clippy `-D warnings` clean, fmt clean.
- mew-web-ui: 112/112 vitest, `tsc --noEmit` clean.
- Still owed: live `just desktop-dev` smoke — the hole should now show the
  CEF page, the rest of the window should be fully opaque, and clicks in the
  hole must reach CEF (PLAN.md risk #1).

# 2026-07-19 — WKWebView transparency take 3: drawsBackground on the view instance

## Summary

After the opaque-chrome fix, the window background looked right but the CEF
hole still painted the webview's default background. Cause: wry only sets
`drawsBackground = NO` on the WKWebViewConfiguration at construction; on the
live view the property that matters is the same private KVC key set on the
WKWebView *instance* (exactly what wry's own `set_background_color` runtime
path does in `wkwebview/mod.rs`: "On the webview instance (vs config) for
runtime changes"). The previous `setOpaque(false)` msg_send was a no-op.

`make_webview_transparent` now mirrors wry's runtime path: cast the handle to
`WKWebView`, `setValue:forKey:` `drawsBackground = NO` on the instance,
`setUnderPageBackgroundColor(clearColor)`, and clear the layer background
color. Added `objc2-web-kit` 0.3 and the `NSColor`/`NSString` features to the
desktop crate for this.

## Verification

- mew-desktop: 15/15 tests, clippy `-D warnings` clean, fmt clean.
- Still owed: live `just desktop-dev` smoke — hole should finally show the
  CEF page; clicks in the hole must reach CEF (PLAN.md risk #1).

# 2026-07-19 — Re-assert CEF layering on every visible claim

## Summary

Instance-level `drawsBackground = NO` at setup still wasn't enough: WebKit
re-enables background drawing as the page renders, so the hole went opaque
again after the first paint. The layering pass is now a steady-state
re-assertion instead of a one-shot:

- `native_layering::ensure_cef_layering` (renamed from
  `order_cef_below_webview`) re-applies webview transparency AND the
  CEF-below-WKWebView ordering on every call; all AppKit calls inside are
  idempotent.
- `cef_browser_set_rect` with `visible: true` now runs it on every claim
  (React's ResizeObserver fires these continuously while the browser tab is
  visible), not just once per CEF view handle. The `NativeLayeringGuard` is
  retained only as a record of which handles have been seen (and its unit
  tests); it no longer gates the pass.
- `make_webview_transparent` at setup stays as the initial pass before the
  first claim.

## Verification

- mew-desktop: 15/15 tests, clippy `-D warnings` clean, fmt clean.
- Still owed: live smoke — hole should stay transparent while the page
  renders; clicks in the hole must reach CEF.

# 2026-07-19 — Layering inversion abandoned: CEF stays on top (WebKit composites opaque)

## Summary

The PLAN.md goal — CEF below a transparent WKWebView with a transparent React
hole — is **not viable on this WebKit build**. Proven via live hierarchy
dumps: CEF reorders below the WKWebView correctly (`drawsBackground=0`, layer
bg null, CEF at subview [0]), yet the browser stays invisible until the
WKWebView is moved below CEF — at which point CEF composites fine. The
WebContent process paints an opaque background into the webview's remote
layer tree, and no AppKit-level transparency on the view punches through it.
`setDrawsBackground:` is gone from this WebKit (unrecognized selector), and
`setValue:forKey: drawsBackground` only writes wry's KVO subclass mirror.

What shipped instead (the stable baseline):

- `native_layering.rs`: stripped to `ensure_cef_on_top` — asserts CEF is the
  content view's topmost subview on every visible claim (CEF adds new views
  on top; nothing reorders it back). Guard + tests retained.
- `embed.rs`: browser is now created with `window_info.hidden = 1` so no
  window flashes before React claims it (kept from the underlay experiment —
  harmless, and CEF composites on top regardless).
- Reverted everything transparency-related: `tauri.conf.json` back to no
  `transparent`/`macOSPrivateApi`, Cargo back to no `macos-private-api`
  feature, `objc2-app-kit` trimmed to just `NSView`, dropped the
  `objc2`/`objc2-foundation`/`objc2-web-kit` deps. Frontend viewport hole,
  `data-host` hook, CSS transparency, sidebar-wrapper change all reverted.
- The AppKit-menu deletion from the earlier step stands (HTML popover +
  CommandDialog are the only menu implementations).

## Verified

- mew-desktop: 15/15 tests. mew-cef-host: 1/1. Clippy `-D warnings` both.
  fmt both.
- mew-web-ui: 111/111 vitest, `tsc --noEmit`, build. All green.
- App launches clean (the earlier panic was objc2's bool→BOOL encode check
  aborting across the FFI boundary; gone with the transparency code).

## Still open (the real overlay problem)

HTML overlays that overlap the browser rect (browser-tools popover, surface
picker, cmd-K palette, settings, sheets, toasts) are still under CEF. The
working approach, when prioritized: a native click-through NSView shield
above CEF that React drives via IPC with overlay rects/open state. Simpler
intermediate: hide the CEF surface while full-window modals (settings,
palette) are open, since only the tools popover + surface picker overlap the
browser during normal use.

# 2026-01-21 — hidden `mew funfact` easter egg

Implemented the hidden `mew funfact` CLI subcommand per PLAN.md.

Files touched:
- `crates/mew/src/cli.rs` — added `Commands::Funfact` with `#[command(hide = true)]`.
- `crates/mew/src/commands/funfact.rs` — new module with 8 fun facts, `pick_fact()` using `SystemTime`, and `funfact_cmd()`; includes tests for output membership, CLI parsing, and hidden help.
- `crates/mew/src/commands/mod.rs` — registered `pub mod funfact`.
- `crates/mew/src/main.rs` — imported `funfact_cmd` and dispatched the `Funfact` variant.

Verification:
- `cargo build -p mew` succeeded.
- `cargo run -p mew -- funfact` printed a random fact.
- `cargo run -p mew -- --help` does not list `funfact`.
- `cargo test -p mew` passed (118 unit tests + 3 integration tests).
- `cargo clippy -p mew` only reports the pre-existing `remote_invite_payload` dead-code warning; our new code is clean.

Notes:
- No new dependencies were added.
- The command uses `SystemTime` for low-quality randomness as specified.
- The `None` default arm was already placed in the middle of the match in `main.rs`, so the `Funfact` arm was inserted right before it there rather than at the end of the match.

# 2026-07-24 — hashline test + fun facts refresh

Tested the hashline patch editor on the hidden `mew funfact` easter egg.

Changes:
- `crates/mew/src/commands/funfact.rs` — replaced the frisbee fact with a technical one about the first computer bug, and added three more technical facts (ARPANET `LOGIN`, IPv6 address size, `sudo` etymology). Used `edit_hashline` for the patches; the first insert landed after the closing `];`, so a second patch corrected the array structure.

Verification:
- `cargo test -p mew-hashline` — 56 passed.
- `cargo test -p mew funfact` — 3 passed (membership, CLI parsing, hidden help).
- `cargo run -p mew -- funfact` — printed one of the new technical facts.

Notes:
- The only compiler warning is the pre-existing `remote_invite_payload` dead-code warning in `crates/mew/src/commands/daemon.rs`, unrelated to this work.
