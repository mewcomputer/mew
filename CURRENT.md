# Current Progress — Consolidate Agent Construction

## 2026-07-12 — Context Window Inspector: Step 6 (Cache-Derived Prior Refinement)

**Status: COMPLETE ✅**

### Summary
Split the single global scaling factor in `backfill_manifest` into warm-prefix and cold-suffix groups. When `cache_read_tokens` (and optionally `cache_write_tokens`) are nonzero, the first N segments whose cumulative local-token proportion best matches the cache proportion are scaled against `cache_read + cache_write`, and the remaining segments are scaled against `input - (cache_read + cache_write)`. This tightens estimates for the big static segments (scaffold, tools, context files) whose true token total is known from the cache read. Degrades gracefully to global scaling when cache info is absent, the split is degenerate (all-warm/all-cold), or the ratio error exceeds tolerance.

### Changes

**`crates/mew-agent/src/manifest.rs`:**
- `backfill_manifest` — rewrote scaling logic to attempt prefix/suffix split before falling back to global scaling. `warm_target = cache_read + cache_write`; if 0, uses single global factor. Uses `saturating_add` to avoid overflow.
- `find_cache_split(segments, warm_target, input_tokens) -> Option<usize>` — new helper. Walks segment boundaries (1..len-1), picks the split index minimizing absolute ratio error between the cumulative local proportion and `warm_target/input_tokens`. Returns `None` when: segments < 2, `warm_target >= input_tokens`, total local tokens == 0, best split would be all-warm/all-cold, or ratio error exceeds tolerance (`max(target_ratio/2, 0.15)`).
- `apply_split_scaling(segments, split_idx, warm_target, input_tokens)` — new helper. Computes warm/cold local subtotals and their respective scale factors, then applies the correct factor to each segment based on its position relative to the split index. Children scale with their parent's group factor.
- 12 new tests covering: perfect ratio match, different scale ratios, degradation to global (no cache, all-warm, ratio mismatch), cache_write inclusion, very small cold suffix, sum invariant with messy numbers, multi-segment best-boundary selection, children scaled with parent group, and `find_cache_split` edge cases (single segment, warm exceeds input).

### Verification
- `cargo build -p mew-agent` ✅
- `cargo test -p mew-agent` — 122 tests pass (27 manifest tests: 15 existing + 12 new) ✅
- `cargo clippy -p mew-agent -- -D warnings` ✅
- `cargo fmt --check -p mew-agent` ✅
- Pre-existing `mew` binary build failures (unwired `PlanApprovalRequest` variants in daemon/tui) are unrelated WIP.

### Acceptance Criteria — All Met
- AC.1 ✅ Prefix/suffix split scaling when `cache_read_tokens > 0` and a clean split is found
- AC.2 ✅ Warm prefix scaled against `cache_read + cache_write`, cold suffix against `input - (cache_read + cache_write)`
- AC.3 ✅ Graceful degradation to global scaling when cache is absent (0), all-warm, or ratio mismatch
- AC.4 ✅ Sum invariant: `Σ tokens_scaled == input_tokens` (±2 for rounding) in all paths
- AC.5 ✅ Children scaled with parent's group factor
- AC.6 ✅ 12 tests covering split behavior and edge cases
- AC.7 ✅ Clippy clean, fmt clean, all tests pass

---

## 2026-07-11 — Context Window Inspector: Steps 4-5 (tiktoken + Web UI)

**Status: COMPLETE ✅**

### Summary
Shipped the context window inspector to the web UI. Integrated `tiktoken` (v3.5.1) for real per-segment token estimates (supports DeepSeek, Qwen, Llama, Mistral encodings). Added `manifest` to `ProviderEventWire::MessageEnd` so the web client gets manifests during live streaming. Built `MessageInspector` component with collapsed summary line, stacked bar, and expandable segment tree. All counts prefixed with `~` (per user request — most models use compatible endpoints but aren't actually OpenAI/Anthropic).

### Changes

**Rust — tiktoken integration (Step 4):**
- `Cargo.toml` + `crates/mew-agent/Cargo.toml` — added `tiktoken = "3.5"` workspace dep
- `crates/mew-agent/src/manifest.rs` — added `count_tokens(text, model_id)` using `tiktoken::encoding_for_model()` with `cl100k_base` fallback. Threaded `model_id` through all segment builders (`build_system_segments`, `build_tools_segment`, `build_history_segment`, `build_message_segment`, `build_part_segment`). Each segment now counts the text it segments and stores the result in `tokens`. Tool calls use `state.input()` (not `raw_input`, which is `#[serde(skip)]`). Updated existing tests: `test_manifest_tokens_zero_before_backfill` → `test_manifest_tokens_nonzero_after_build`; `test_backfill_manifest_no_scaling_when_tokens_zero` now manually zeroes tokens. Added `test_count_tokens_returns_nonzero` + `test_count_tokens_unknown_model_falls_back`.

**Rust — wire protocol (Step 3):**
- `crates/mew-message/src/lib.rs` — added `manifest: Option<TurnManifest>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` to `ProviderEventWire::MessageEnd`
- `crates/mew-protocol/src/lib.rs` — `provider_event_to_wire` sets `manifest: None`; daemon overrides. Added `message_end_with_manifest_roundtrip` test. Fixed pre-existing clippy `large_enum_variant` warnings (boxed `ExtensionEvent` and `MessageEnd.message`). Updated existing test to include `manifest: None`.
- `crates/mew-daemon/src/lib.rs` — `translate_event` extracts manifest from last assistant message in `agent.messages` after `MessageEnd` and attaches it to the wire event
- `crates/mew-daemon/src/client.rs` — `wire_to_provider_event` ignores the manifest field
- `crates/mew-mobile-core/src/lib.rs` + `state.rs` — added `..` to match arms, `manifest: None` to test constructions

**TypeScript — web client (Step 4):**
- `mew-web-client/src/index.ts` — added `TurnManifest` + `Segment` types, `manifest?: TurnManifest | null` on `AssistantMeta`, fixed `message_end` usage type to include all 5 fields (`reasoning`, `cache_read`, `cache_write`), added `manifest?` to `message_end`

**TypeScript — web UI store (Step 5):**
- `mew-web-ui/src/stores/session.ts` — extended `ChatMessage` with `assistantMeta?: AssistantMeta`, preserved `m.assistant` in `onSessionHistory`, attached `assistantMeta` (with manifest) to last assistant message on `message_end`

**TypeScript — MessageInspector component (Steps 7-8):**
- `mew-web-ui/src/components/message-inspector.tsx` (new) — collapsed summary line (`~9.7k ↓ · ~238 ↑ · 8% (~9.7k/128.0k)` with cache warmth prefix), expandable stacked bar (proportional segment widths, CSS variable colors per `SegmentKind`), segment tree (disclosure triangles, `~tokens_scaled`, label). Uses plain `useState` toggle (not Radix Collapsible — jsdom compatibility). Error case shows "error · structure below".
- `mew-web-ui/src/components/message-item.tsx` — renders `<MessageInspector>` below assistant messages with a manifest
- `mew-web-ui/src/index.css` — added `:root` CSS variables for inspector color palette (10 segment kinds)
- `mew-web-ui/src/components/ui/collapsible.tsx` — installed via shadcn (not used in final implementation)

**Tests:**
- `mew-web-ui/src/__tests__/message-inspector.test.tsx` (new) — 5 tests: summary line with `~` prefix, error state, expand to segment tree, token counts visible, child segment expansion
- `mew-web-ui/src/__tests__/store.test.ts` — 2 new tests: `onSessionHistory` preserves `assistantMeta`, `message_end` attaches `assistantMeta`

### Verification
- `cargo build -p mew` ✅
- `cargo test -p mew-agent` — 107 tests pass (12 manifest tests) ✅
- `cargo test -p mew-protocol` — 96 tests pass ✅
- `cargo clippy` clean across all affected crates ✅
- `cargo fmt --check` clean ✅
- `pnpm build` (web-client + web-ui) ✅
- `pnpm test` — 45 tests pass (5 inspector + 2 store) ✅

### Acceptance Criteria — All Met
- AC.1 ✅ `count_tokens` returns nonzero, falls back to cl100k_base
- AC.2 ✅ `build_manifest` produces nonzero token segments
- AC.3 ✅ `backfill_manifest` scales `tokens_scaled` to sum to `input_tokens`
- AC.4 ✅ `ProviderEventWire::MessageEnd` includes `manifest` with serde attrs, round-trips
- AC.5 ✅ Web client exports `TurnManifest`/`Segment` types, `message_end` usage has 5 fields
- AC.6 ✅ Store preserves `assistantMeta` in `onSessionHistory`, attaches on `message_end`
- AC.7 ✅ `MessageInspector` renders summary line with `~` prefix
- AC.8 ✅ `MessageInspector` shows "error · structure below" for `None` input
- AC.9 ✅ `MessageInspector` expands to show segment tree with token counts
- AC.10 ✅ Clippy clean, fmt clean, builds clean, tests pass

---

## 2026-07-10 — Phase 2 Completion: Fix Tests, Install Command, OS Sandbox, Token Management

**Status: COMPLETE ✅**

### Summary
Finished the four remaining Phase 2 deliverables: fixed broken `commands::ext` tests, added `mew ext install`, implemented macOS Seatbelt sandbox enforcement for extension processes, and built token management infrastructure (keyring + file fallback) with CLI commands. The daemon socket-attach path is deferred to a separate plan (requires daemon to own a broker — currently constructed per-session).

### Changes

**`crates/mew/src/commands/ext.rs`** — Fixed 4 broken tests with `CWD_LOCK` mutex (serializes cwd-mutating tests). Added `install_extension` (git clone + local path copy + manifest validation + `--force` flag). Added `revoke_extension`, `rotate_all`, `show_token` CLI functions. Added `derive_name`, `copy_dir_recursive` helpers. Updated `doctor()` to show sandbox status (`[sandboxed]` / `[unsandboxed (platform)]` / `[unsandboxed (legacy)]`). Added 4 install tests.

**`crates/mew/src/cli.rs`** — Added `Install`, `Revoke`, `RotateAll`, `Token` variants to `ExtCommands`.

**`crates/mew-ext-broker/src/sandbox.rs`** (new) — `build_sandbox_profile()` generates a Seatbelt S-expression profile (default-deny: package dir + storage dir + system libs, network denied unless `sandbox.net = true`). `sandbox_available()` returns true on macOS. `escape_path()` prevents profile injection. 5 unit tests.

**`crates/mew-ext-broker/src/tokens.rs`** (new) — `mint_token`, `validate_token` (constant-time comparison), `revoke_token`, `rotate_all_tokens`, `show_token`. Keyring-first with file fallback + marker file for keyring-stored token discovery. 2 unit tests.

**`crates/mew-hooks-runtime/src/transport.rs`** — `SpawnSpec::Command` gains `sandbox: Option<(String, Vec<(String, String)>)>`. `to_command()` wraps with `sandbox-exec -p <profile> -D KEY=VALUE` on macOS.

**`crates/mew-ext-broker/src/broker.rs`** — Manifest extension spawn site builds sandbox profile and passes to `SpawnSpec::Command`.

**`crates/mew-ext-broker/src/discovery.rs`** — Added `discover_extensions_from_dirs(project_dir, global_dir)` for testable discovery. `discover_extensions(cwd)` delegates using `UserDirs` (not ProjectDirs — matches existing behavior).

**`crates/mew-ext-broker/src/lib.rs`** — Exported `sandbox`, `tokens` modules, `parse_manifest`, `discover_extensions_from_dirs`.

**`crates/mew-ext-broker/Cargo.toml`** — Added `keyring = "3"`, `mew-config = { workspace = true }`.

**`crates/mew/Cargo.toml`** — Added `tempfile = "3"` to production deps (was dev-only).

**`crates/mew-config/src/lib.rs`** — Added `revoked_extensions: Vec<String>` to `State` (with `#[serde(default)]` for backward compat). Fixed test struct literal.

### Acceptance Criteria — All Met
- AC.1 ✅ All `commands::ext` tests pass under parallel execution (10 tests, `--test-threads=4`)
- AC.2 ✅ `mew ext install <local-path>` copies valid extension, appears in discovery (`test_install_from_local_path`)
- AC.3 ✅ Install rejects invalid manifests, name conflicts, nonexistent paths (3 tests)
- AC.4 ✅ macOS Seatbelt sandbox: denies network, restricts filesystem (5 sandbox tests + integration test passes)
- AC.5 ✅ `mew ext doctor` shows sandbox status per extension
- AC.6 ✅ `mew ext revoke`/`rotate-all` manage tokens (token tests pass)
- AC.7 ✅ `mew ext token` prints attach token; revoke persists to `revoked_extensions` in state
- AC.8 ✅ Clippy clean, fmt clean, arch-check passes, all tests pass (except pre-existing e2e daemon test)

### Deferred
- Daemon socket-attach path (requires daemon to own a broker — separate plan)
- `ExtensionHello` token field (not needed until socket-attach ships)
- Linux Landlock/seccomp sandbox (macOS only for now)
- Windows sandbox

---

## 2026-07-10 — Phase 3: Upgrade-Delta Re-Prompting + Per-Capability Individual Consent

**Status: COMPLETE ✅**

### Summary
When a manifest extension's requested capabilities grow (manifest upgrade), the resolver now detects the delta and re-prompts for only the new capabilities instead of silently clamping them away. Sensitive capabilities (High/Highest risk tier) now require individual y/N confirmation rather than being batch-approved.

### Changes

**`crates/mew-ext-broker/src/consent.rs`** — Added `last_requested: Vec<String>` field to `ConsentEntry` (with `#[serde(default)]` for backward compat). Added `get_last_requested()` and `set_consent(name, granted_ids, requested_ids)` methods on `ConsentState`. Updated `set_granted_caps` to preserve existing `last_requested`. Added 2 tests: `test_set_consent_round_trip`, `test_backward_compat_last_requested_default`. Updated 2 existing test struct literals.

**`crates/mew-ext-broker/src/capabilities.rs`** — Added `CapabilitySet::to_ids()` convenience method + `test_to_ids` unit test.

**`crates/mew-ext-broker/src/capability_descriptions.rs`** — Modified `build_consent_prompt()` to split non-sensitive (batch) and sensitive (individual) sections. Added `build_delta_prompt()` for upgrade case and `build_sensitive_cap_prompt()` for individual sensitive-cap prompts. Updated `test_build_consent_prompt_content`; added `test_build_delta_prompt`, `test_build_delta_prompt_all_sensitive`, `test_build_sensitive_cap_prompt`.

**`crates/mew/src/setup/agent.rs`** — Restructured `build_consent_resolver` for delta detection + individual consent. Three manifest sub-cases: no-persisted (first run), persisted-no-change (clamp), persisted-with-delta (re-prompt new caps). Added `prompt_and_consent_caps()` helper implementing two-phase consent (batch non-sensitive + individual sensitive). Added 6 resolver tests.

**`crates/mew-ext-broker/src/lib.rs`** — Exported `build_delta_prompt`, `build_sensitive_cap_prompt`.

**`crates/mew-ext-broker/tests/broker_integration.rs`** — Added `test_manifest_upgrade_reprompts` integration test.

### Acceptance Criteria — All Met
- AC.1 ✅ Delta re-prompting (`test_upgrade_delta_reprompts` + `test_manifest_upgrade_reprompts`)
- AC.2 ✅ No-change no-reprompt (`test_upgrade_no_change_no_reprompt`)
- AC.3 ✅ Individual sensitive consent (`test_first_run_individual_consent`)
- AC.4 ✅ Non-interactive denies (`test_first_run_noninteractive_denies_sensitive` + `test_upgrade_noninteractive_keeps_existing`)
- AC.5 ✅ Backward compat (`test_upgrade_backward_compat` + `test_backward_compat_last_requested_default`)
- AC.6 ✅ Two-phase prompt split (`test_build_consent_prompt_content`)
- AC.7 ✅ Delta prompt (`test_build_delta_prompt`)
- AC.8 ✅ Clippy clean, fmt clean, arch-check passes

### Verification
- `cargo fmt --check` ✅
- `cargo clippy --all -- -D warnings` ✅
- `just arch-check` ✅
- `cargo test -p mew-ext-broker` — 72 lib + 13 integration = 85 tests pass ✅
- `cargo test -p mew -- setup::agent` — 9 tests pass ✅
- Implementation review by `general-purpose` subagent: no bugs, no correctness issues
- Pre-existing failures: 2 `commands::ext` tests (cwd-isolation), 1 `bin_e2e_daemon` test — all unrelated to this change

---

## 2026-07-09 — Codex `responses_lite` / catalog override wiring

**Status: COMPLETE ✅**

### Problem
`gpt-5.6-luna` (and siblings) need the `x-openai-internal-codex-responses-lite`
header and theResponses Lite request-body shape. The adapter already had the
logic; the question was whether `responses_lite` reached the adapter at runtime.

### What changed
- **mew-catalog**: added `load_codex_from_path(path)` so a local catalog can be
  parsed with the same `parse_codex()` path used for the network cache.
- **mew setup/providers.rs**: `load_catalog` now discovers `catalog_codex.json`
  from cwd up to the git root and merges it into the catalog before the
  OAuth-gated network fetch. This gives API-key users the same `responses_lite`
  and reasoning-level metadata as OAuth users, and lets repos pin model config.
- **mew-config**: `CustomModel` gained `responses_lite`, and the config→catalog
  conversion in setup/providers.rs now copies it through.

### Verification
- `cargo clippy -p mew-catalog -p mew-config -p mew-provider-responses -p mew -- -D warnings` clean.
- `cargo fmt --check` clean.
- New tests:
  - `mew-catalog::test_load_codex_from_path_reads_override`
  - `mew::setup::providers::discover_codex_catalog_*` (3 cases)
  - `mew-config::test_custom_model_parse` now asserts `responses_lite` round-trips.
- Existing responses-lite tests still pass:
  - `mew-provider-responses::test_stream_responses_lite_header`
  - `mew-provider-responses::test_build_request_body_responses_lite`
- Pre-existing `commands::ext::tests::test_ext_remove_deletes_package` failure is
  unrelated ext-broker WIP.

---

## 2026-07-09 — Phase 2c: Consent Hardening (Fail-Closed + Clamping + Cleanup)

**Status: COMPLETE ✅**

### Summary
Closed two critical security findings and five required findings from the Phase 2b code review.

### Changes

**`capabilities.rs`** — Added `CapabilitySet::intersect()` (exact-match set intersection, no hierarchy). Used for security clamping of persisted consent caps. Added `test_intersect` unit test. Updated `test_from_id_unknown` to use exported `LEGACY_FULL_SENTINEL`.

**`consent.rs`** — Made `LEGACY_FULL_SENTINEL` pub. Added `ConsentDecision::to_granted_ids()` (serializes decision → capability ID strings for persistence, eliminates inline sentinel literal). Added `ConsentDecision::to_caps(approved_fallback)` (shared mapping from ConsentDecision → CapabilitySet, eliminates duplicated match blocks in broker). Hardened `read_file` to log `tracing::warn!` for corrupt/unreadable consent state instead of silently swallowing errors. Added 6 unit tests.

**`broker.rs`** — Both spawn loops now use `decision.to_caps()`. Manifest-extension `Approved` fallback changed from `legacy_full()` → `observe_only()` (closes privilege escalation). Bare-plugin loop keeps `legacy_full()` fallback.

**`agent.rs`** — Restructured `build_consent_resolver`: (1) persisted manifest caps clamped via `intersect()` with current `requested_capabilities()`, (2) stale `__legacy_full__` sentinel for manifest extension detected and falls through to re-prompt, (3) both first-run branches use `to_granted_ids()` for persistence. No more inline `"__legacy_full__"` literal.

**`lib.rs`** — Exported `LEGACY_FULL_SENTINEL`.

**`broker_integration.rs`** — Added `test_manifest_approved_fallback_observe_only` (AC.1). Updated `test_consent_persisted` to use exported const. Fixed pre-existing clippy `ptr_arg` lint.

### Acceptance Criteria — All Met
- AC.1 ✅ Manifest `Approved` → `observe_only()` (integration test)
- AC.2 ✅ Persisted caps clamped via `intersect()` (unit test)
- AC.3 ✅ Stale sentinel re-prompts (unit test)
- AC.4 ✅ No bare `"__legacy_full__"` literals outside const def (grep verified)
- AC.5 ✅ `to_granted_ids()` used for persistence (unit test)
- AC.6 ✅ `to_caps()` eliminates duplicated match (unit tests)
- AC.7 ✅ `read_file` warns on corrupt state (unit test)
- AC.8 ✅ Clippy clean, fmt clean, arch-check passes, 78 tests pass

## 2026-07-09 — Codex provider: store=false fix + rename openai-responses → codex

**Status: COMPLETE ✅**

### The 400 bug
`mew` against the ChatGPT subscription backend 400'd with
`{"detail":"Store must be set to false"}`. The Responses adapter never set
`store`; the ChatGPT (OAuth) backend rejects requests without `store=false`.
Fixed in `build_request_body` (`mew-provider-responses/src/lib.rs`): OAuth path
sets `"store": false`; API-key path omits it (api.openai.com allows the
default). Tests: oauth sets store=false; api-key omits store.

### Provider rename openai-responses → codex
Models were prefixed `openai-responses/...`; the user wants `codex/...` (the
ChatGPT-subscription product is Codex). Renamed the provider throughout:
- `OpenaiResponsesOAuth` → `CodexOAuth` (slug `codex`, display "Codex (ChatGPT)").
- Config default provider + `credential_ref`: `openai-responses` → `codex`
  (env var is now `MEW_CRED_CODEX`).
- Catalog: `parse_codex` sets `provider: "codex"`.
- `provider_name_to_shape`, `provider_available`, `codex_logged_in`,
  `discover_models` codex seed, daemon lister gate, `commands/auth`, `cli` help.
- **Token migration**: `codex_token_path()` one-time renames
  `auth/openai-responses.json` → `auth/codex.json` (via `Once`) so existing
  logins survive. (The dev's token was already at codex.json from a prior run
  of the renamed code, so the migration is a no-op here.)

### Not done (flagged by user, deferred)
- **Split `mew-provider-responses` crate** ("we probably should split it up
  anyways"). The crate still serves both the ChatGPT-OAuth (codex) and
  OpenAI-API-key Responses paths under one provider. A future split would
  separate `codex` (subscription/OAuth) from an OpenAI API-key Responses
  provider. The crate name and the `openai_oauth` protocol module are
  unchanged for now (the protocol IS OpenAI's; the provider is Codex).

### Verification
- `cargo clippy --all -- -D warnings` clean; `cargo fmt --check` clean (my
  files); `cargo build -p mew` clean.
- mew-catalog 36, mew-provider-responses 44 (+2 store tests), mew auth/setup
  tests pass. Only pre-existing `ext_remove` failures remain (ext-broker WIP,
  fail on clean HEAD).

---

## 2026-07-09 — Codex models in the picker (hybrid catalog + live /models)

**Status: COMPLETE ✅**

ChatGPT-subscription (Codex) models now appear in the model picker when logged
in, on both the standalone TUI and the daemon (web/iOS/`--connect`) path.

### Root cause
- `Adapter::list_models` returned empty for OAuth (comment claimed "no /models
  endpoint" — disproven: `https://chatgpt.com/backend-api/codex/models` exists).
- The daemon path never calls `list_models`; it reads the catalog only, gated by
  `provider_has_credential` — which checks an API-key `credential_ref`. OAuth's
  credential is the token file, so the gate hid Codex even if catalog-seeded.

### What was done (hybrid: static catalog + live authed /models, hardcoded source)
- **mew-catalog**: `CODEX_MODELS_URL` (codex repo `models.json`, tracked on
  `main`), `load_codex()` (24h cache+etag, mirrors `load_umans`), `parse_codex()`
  (codex `ModelsResponse` → `Model`, provider=`openai-responses`, shape=
  `responses`; filters `visibility=="list" && supported_in_api`; thinking_variants
  from `supported_reasoning_levels` — needed because new slugs like `gpt-5.6-sol`
  no longer contain "codex" so the builtin gpt-5 arm doesn't fire). Exposed
  `codex_cache_path()`/`write_codex_cache()`; added codex files to `clear_cache()`.
- **mew-provider-responses**: `list_models` OAuth branch now calls authed
  `{base_url}/models`, parses via `mew_catalog::parse_codex`, and best-effort
  writes the live (plan-filtered) response to the codex cache so the daemon
  benefits next launch. Exposed `openai_responses_token_path()`.
- **setup/providers.rs**: `provider_available()` (credential OR OAuth token file);
  `openai_responses_logged_in()`; `load_catalog` merges codex models gated on
  login; `discover_models` seeds codex from the catalog as an offline baseline
  (dedups against the live list_models result).
- **commands/daemon.rs**: lister gate swapped `provider_has_credential` →
  `provider_available` so OAuth-logged-in providers surface.
- Cargo: `mew-catalog` added to `mew-provider-responses` (parser reuse + cache write).

### Tests
- mew-catalog: `parse_codex` maps a visible model (provider/shape/thinking_variants
  /vision/reasoning); filters hidden + api-only; empty cases. (36 pass.)
- mew-provider-responses: `list_models` OAuth wiremock — parses codex response +
  refreshes cache (with a `CodexCacheRestore` guard so the dev's real cache isn't
  polluted); non-2xx → Err. (42 pass.)
- clippy `--all -D warnings` clean; `cargo fmt --check` clean (my files);
  `just arch-check` clean; `mew` builds.

### Watch-outs / not done
- **Confirm the live `/models` response shape against the real authed endpoint.**
  Implemented as codex `ModelsResponse { models: [...] }` per the codex
  `model_list.rs` test. If the real shape differs, the live parse fails → Err →
  `discover_models` skips live and the static catalog baseline still covers the
  picker. Resilient, but the plan-filtered path won't work until confirmed.
- Static catalog shows all `visibility=="list"` models (not plan-filtered); the
  live endpoint filters by plan. Plan-filtering the static catalog via
  `chatgpt_plan_type` from the id_token is a future Phase 3.
- `CODEX_MODELS_URL` tracks `main`; lenient parsing (ignore unknown fields)
  mitigates shape drift.
- Live e2e smoke (picker shows `openai-responses/gpt-5.6-sol` etc., selecting
  one runs a chat turn) not run — needs interactive TUI + real ChatGPT creds.
- Pre-existing failure `commands::ext::tests::test_ext_remove_deletes_package`
  is unrelated ext-broker WIP (fails on clean HEAD without my changes).

---

## 2026-07-09 — Phase 2b: Manifest-Based Extension Spawning + Capability-Delta Consent

**Status: COMPLETE ✅**

Wired the broker to spawn manifest-based extensions and added capability-delta consent UX.

### What was done

**New file: `crates/mew-ext-broker/src/capability_descriptions.rs`**
- `capability_description(cap)` — plain-language one-liner for each Capability variant
- `is_sensitive(cap)` — delegates to `requires_individual_consent()`
- `build_consent_prompt(name, manifest)` — formats the capability-delta prompt string with descriptions and ⚠ markers for sensitive caps

**Modified: `crates/mew-ext-broker/src/capabilities.rs`**
- Added `Capability::from_id(id) -> Option<Capability>` — reverse-maps ID strings (e.g. `"hooks:gate:mutate"`) back to typed Capability values
- Added `reconstruct_caps(ids: &[String]) -> CapabilitySet` — rebuilds a CapabilitySet from stored ID strings, skipping unknowns with `tracing::warn`
- Added round-trip tests: `test_from_id_round_trip`, `test_from_id_unknown`, `test_reconstruct_caps`

**Modified: `crates/mew-ext-broker/src/consent.rs`**
- `ConsentDecision` gains `ApprovedWithCaps(CapabilitySet)` variant; removed `Copy` and serde derives (CapabilitySet isn't serializable)
- `ConsentEntry` restructured: `granted_capabilities: Vec<String>` (capability IDs) replaces `decision: ConsentDecision` — decouples persisted state from enum shape
- `ConsentState` API: `get`/`set` → `get_granted_caps`/`set_granted_caps`
- Added `is_legacy_full()` helper for the `"__legacy_full__"` sentinel check
- `ConsentResolver` signature changed: `Fn(&str, Option<&ExtensionManifest>) -> ConsentDecision`
- All consent tests updated to new API; added `test_consent_entry_serialization`, `test_consent_approved_with_caps`

**Modified: `crates/mew-ext-broker/src/broker.rs`**
- `from_dirs_filtered_with_config` gains `discovered_extensions: &[DiscoveredExtension]` parameter
- After spawning bare plugins, iterates discovered extensions: skips disabled + declarative-only, spawns manifest-based ones via `SpawnSpec::Command`, calls consent resolver with `Some(&manifest)`, maps `ConsentDecision` → `CapabilitySet`
- Merges bare plugin slots and manifest extension slots into one sorted Vec

**Modified: `crates/mew/src/setup/agent.rs`**
- `build_dispatcher` gains `discovered_extensions` parameter, passes to broker
- `build_consent_resolver` rewritten for new `Fn(&str, Option<&ExtensionManifest>)` signature: shows capability-delta prompt for manifest extensions, legacy prompt for bare plugins, persists capability IDs

**Modified: `crates/mew/src/commands/tui.rs` + `run.rs`**
- Both call `discover_extensions(cwd)` before constructing broker/agent
- Pass `&discovered` to `build_dispatcher`, `build_session_agent`, and `wire_subagents`

**Modified: `crates/mew-ext-broker/tests/broker_integration.rs`**
- All existing tests updated for new broker signature (`&[]` for discovered_extensions)
- Updated `test_legacy_plugin_restricted` and `test_consent_persisted` for new resolver signature
- Added: `test_manifest_extension_spawns`, `test_manifest_extension_scoped_caps`, `test_manifest_extension_consent_prompt`

### Verification
- `cargo build --all` ✅
- `cargo test -p mew-ext-broker` — 59 unit + 11 integration = 70 tests pass ✅
- `cargo clippy --all -- -D warnings` ✅
- `cargo fmt` ✅
- `just arch-check` ✅

## 2026-07-09 — Own the OpenAI/Codex OAuth (in-tree PKCE + device flow)

**Status: COMPLETE ✅**

Replaced the external `openai-auth` crate with an in-tree implementation so we
own the auth path and can add a headless device-code flow. Root bug being
fixed: `openai-auth`/`webbrowser` honored `$BROWSER`, so a `$BROWSER=vim`
env opened an editor instead of a browser at `mew auth login`.

### What was done
- **New** `crates/mew-provider-responses/src/openai_oauth.rs` — all protocol
  machinery: `OAuthConfig`, PKCE (`generate_pkce`/`pkce_challenge`), state,
  `build_authorize_url`, JWT `extract_account_id` (manual b64url+json, no
  `jsonwebtoken`), `exchange_code`/`refresh_tokens` (reuses omitted refresh
  token), `CallbackServer` (tokio TcpListener, 8 KiB request-line parse,
  404+keep-waiting for non-callback paths), `open_browser` (platform launcher,
  never `$BROWSER`), and the device flow (`request_device_code`/
  `poll_device_token`).
- `oauth.rs` — rewired `OAuthProvider` impl to `crate::openai_oauth`; splits
  into `login_browser` (bind 1455 → PKCE → open_browser → 120s callback →
  exchange) and `login_headless` (device code → 900s poll → exchange with
  server-supplied verifier + `{issuer}/deviceauth/callback` redirect).
- `mew-provider/src/auth.rs` — `OAuthProvider::login(&self, headless: bool)`;
  `login(provider, headless)` threads it through. Mock updated.
- `cli.rs` + `commands/auth.rs` — `--headless` flag on `auth login`. Bind(1455)
  failure hints at `--headless`.
- Cargo: removed `openai-auth` (workspace + crate); added `base64 = "0.22"`,
  `sha2 = "0.10"` (workspace), `rand` (workspace) to `mew-provider-responses`.
  Lockfile drops `openai-auth`, `webbrowser`, `tiny_http`, `jsonwebtoken`,
  `querystring` (verified empty via grep).

### Protocol grounding
Confirmed against vendored `openai-auth-1.0.0` source (client_id
`app_EMoamEEZ73f0CkXaXp7hrann`, issuer `https://auth.openai.com`, S256 PKCE,
the three codex params) and opencode's `codex.ts` (device flow endpoints +
server-held PKCE). One deviation from the plan: the device `interval` field
comes back as a **string** (opencode `parseInt`s it), so it's deserialized
flexibly (number-or-string) rather than as a bare `u64`.

### Tests (20 new in `openai_oauth`, 2 new in `oauth.rs`; all green)
- RFC 7636 PKCE vector; generated verifier/challenge/state shape + uniqueness.
- `build_authorize_url` contains every param incl. encoded redirect_uri +
  the three codex-specific params.
- `extract_account_id` from fake JWT; missing-claim + malformed → Err.
- wiremock: exchange form fields + `expires_at`≈now+`expires_in`; default
  3600; refresh reuses omitted refresh_token; non-2xx surfaces status+body;
  device usercode JSON+headers; poll 403×2→200; poll 500→Err.
- localhost callback server (port 0): happy path, state mismatch→400+Err,
  `error`+`error_description` percent-decoded, favicon→404+keep-waiting,
  no-request→outer timeout.

### Verification
- `cargo clippy --all -- -D warnings` clean; `cargo fmt --check` clean (my
  files); `just arch-check` clean; `mew` binary builds.
- `cargo test -p mew-provider-responses` (40 pass), `-p mew-provider` (auth
  7 pass), `-p mew --bin mew commands::auth` (5 pass).
- Note: 2 pre-existing failures in `commands/ext.rs` (`ext_remove_*`) are
  from the concurrent ext-broker WIP, confirmed failing on clean HEAD
  without my changes.
- Not yet done: live e2e smoke (`mew auth login` with `$BROWSER=vim`,
  `--headless` against a phone) — needs real ChatGPT creds; no live-API
  test is possible for oauth.

---

## 2026-07-09 — Phase 2a: Manifest Parser + Discovery (in progress)

**Status: IN PROGRESS**

### Completed
- **Phase 1 ✅** — Manifest parser: `parse_manifest()` + `validate_manifest()` in manifest.rs. Added `toml` dep. 3 tests (parse valid, parse invalid, validate denylist).
- **Phase 2 ✅** — Extension discovery: `discovery.rs` with `DiscoveredExtension`, `ExtensionScope`, `discover_extensions(cwd)`. Scans `~/.config/mew/extensions/` and `.mew/extensions/`. Dedup: project beats global. 4 tests.

### Next
- **Phase 3** — SpawnSpec enum + broker integration (manifest-based extensions get scoped capabilities)
- **Phase 4** — Loader changes (load_markdown_dirs_with_extra) + [provides] integration
- **Phase 5** — `mew ext` CLI (list/enable/disable/remove/doctor)
- **Phase 6** — Integration tests + verify

---

## Previous: W4 + W5 (Phase 1 complete)

**Status: COMPLETE ✅**

Implemented the `ExtensionBroker` that implements `mew_hooks::Dispatcher`, routing hook calls to extension processes with capability enforcement, concurrency, timeouts, audit logging, and event delivery. Replaces `SubprocessDispatcher` as the runtime's `Dispatcher` impl.

### What was done

**Phase 1 — Move routing logic into the broker:**
- Created `crates/mew-ext-broker/src/broker.rs` with `ExtensionBroker` struct + full `Dispatcher` impl (all 26 methods)
- `ExtensionBroker::from_dirs_filtered_with_config()` — same signature as `SubprocessDispatcher`'s, creates `Principal::extension()` with `CapabilitySet::legacy_full()` per slot
- Routing helpers (`should_fire`, `notify_all_filtered`, `pipe_json_filtered`, `pipe_json_raw`, `detect_outcome`) moved into the broker
- `build_dispatcher` in `setup/agent.rs` switched from `SubprocessDispatcher` to `ExtensionBroker`
- `SubprocessDispatcher` left as dead code for rollback safety
- `call_via_handles` made `pub` in transport.rs for broker consumption

**Phase 2 — Capability enforcement:**
- `hook_capability(HookId) -> Option<Capability>` maps each hook to its required capability
- `check_capability()` checks `Principal.has_capability()` + `should_fire()` before routing
- Legacy extensions get `CapabilitySet::legacy_full()` (all caps) — no-ops for them, active for future manifest-based extensions

**Phase 3 — Gate audit logging:**
- Created `crates/mew-ext-broker/src/audit_log.rs` with `AuditLog` (Mutex<BufWriter<File>> + PathBuf)
- `on_tool_execute_before` and `on_permission_ask` write `GateAuditEntry` per extension
- `set_session_id()` method for audit session context
- `audit_entries()` public accessor for tests/future CLI

**Phase 4 — Event queues (scaffolding):**
- Created `crates/mew-ext-broker/src/event_queue.rs` with `EventQueue` (bounded mpsc, drop-oldest, Lagged)
- Not wired — Phase 2 activates it when socket transport lands

**Phase 5 — Collision-rejecting registration:**
- `registered_tools`/`registered_commands` as `Mutex<HashMap<String, String>>` (interior mutability)
- Duplicate tool/command names from different extensions are skipped with a warning
- Same-extension re-registration allowed (restart case)

**Phase 6 — Tests:**
- Created `conflicting-plugin.rs` example binary (registers `sample-echo`, transforms `on-system-prompt` with `[conflicting-plugin]`)
- 6 integration tests: e2e_hook_delivery, noop_equivalence, collision_rejection, gate_audit, last_writer_wins, capability_enforcement
- 35 unit tests (capabilities, audit_log, event_queue, manifest, principal)
- All 41 tests pass, clippy clean, fmt clean

### Acceptance Criteria
- AC.1 ✅ — `cargo build -p mew` compiles with `ExtensionBroker`
- AC.2 ✅ — Existing `SubprocessDispatcher` tests pass (11 + 5)
- AC.3 ✅ — `test_e2e_hook_delivery` passes
- AC.4 ✅ — `test_noop_equivalence` passes
- AC.5 ✅ — `test_collision_rejection` passes
- AC.6 ✅ — `test_gate_audit` passes
- AC.7 ✅ — `test_capability_enforcement` passes (HooksGate + sub-scope non-implication)
- AC.8 ✅ — clippy clean, fmt clean
- AC.9 ✅ — `test_last_writer_wins` passes (sample-plugin wins, alphabetically last)
- AC.10 (stretch) — Not implemented (Phase 7 lifecycle hardening deferred)

### Files
- New: `crates/mew-ext-broker/src/broker.rs`, `audit_log.rs`, `event_queue.rs`
- New: `crates/mew-ext-broker/tests/broker_integration.rs`
- New: `crates/mew-hooks-runtime/examples/conflicting-plugin.rs`
- Modified: `crates/mew-ext-broker/Cargo.toml`, `src/lib.rs`, `src/capabilities.rs`
- Modified: `crates/mew-hooks-runtime/src/lib.rs`, `src/transport.rs`
- Modified: `crates/mew/src/setup/agent.rs`, `crates/mew/Cargo.toml`

---

## Previous: Consolidate Agent Construction

## Status: COMPLETE ✅

## What was done

Eliminated ~420 lines of triplicated agent-construction code by making `run_tui` and `build_and_run` delegate to `build_session_agent`, and extracting shared helpers.

### Phase 1 ✅ — build_session_agent accepts dispatcher
- Added `dispatcher: Arc<dyn Dispatcher>` and `todos_path: Option<PathBuf>` params
- Kept sync (daemon AgentBuilder closure is sync)
- Updated daemon.rs call site to pass `NopDispatcher` + `None`

### Phase 2 ✅ — make_provider_builder helper
- Extracted to `setup/providers.rs`
- Returns `Box<dyn Fn(&str) -> Result<Arc<dyn Provider>, String> + Send + Sync>`
- Replaced 3 inline closure sites (agent.rs, chat.rs x2)

### Phase 3 ✅ — wire_subagents helper
- Extracted to `setup/agent.rs`
- Called inside `build_session_agent` (for daemon path)
- Called again by `run_tui`/`build_and_run` after `register_plugin_tools` (refresh with plugin tools)
- Replaced 3 inline blocks (agent.rs, chat.rs x2)

### Phase 4 ✅ — run_tui delegates to build_session_agent
- Replaced ~130 lines of inlined construction with single call
- TUI-specific steps remain: dispatcher construction, MCP status, sidebar, App state

### Phase 5 ✅ — build_and_run delegates to build_session_agent
- Replaced ~80 lines of inlined construction with single call
- Dropped unused MCP tool loading (was only keeping clients alive, never read)

### Phase 6 ✅ — Verification
- All 8 mew tests pass
- All 137 mew-tui tests pass
- clippy clean, fmt clean, arch-check passes

## Acceptance Criteria
- AC.1 ✅ — Agent::new count in chat.rs = 0
- AC.2 ✅ — Same (both run_tui and build_and_run delegate)
- AC.3 ✅ — set_provider_builder = 0 inline closures (all use make_provider_builder)
- AC.4 ✅ — SubagentStart::new count in chat.rs = 0 (only in wire_subagents)
- AC.5 ✅ — build_session_agent is sync, register_plugin_tools called by callers
- AC.6 ✅ — No behavior change (all tests pass)
- AC.7 ⚠️ — chat.rs at 1148 lines (target was <1000, but remaining code is non-duplicated)

## 2026-07-08 — Heal corrupted state.toml on startup

**Problem:** `mew` crashed with `unknown provider t` when `state.toml` had
stale `last_provider = "t"` / `last_model = "t"` values (likely written by
an earlier partial run during refactoring). Resolvers trusted state blindly.

**Fix (two layers):**

1. **Resilient read** — `setup::providers::resolve_provider` /
   `resolve_model_opt` now validate persisted state against `cfg.providers`
   before using it. Falls back to the built-in default when the persisted
   value is unknown, so a corrupted state file doesn't crash startup.

2. **Startup heal prompt** — `mew-config` gained `validate_state`,
   `heal_state`, and `backup_state_file`. `main.rs` calls
   `startup_state_health_check` before subcommand dispatch:
   - clean state → no prompt, continue.
   - dirty state + interactive TTY → warn + `[y/N]` prompt. `y` → back up
     to `state.toml.bak.<unix-epoch-seconds>` and heal; `n` → exit 0.
   - dirty state + non-TTY (piped stdin, CI) → exit 2 with a message to
     re-run from a terminal.

**Files touched:**
- `crates/mew-config/src/lib.rs` — `validate_state`, `heal_state`,
  `backup_state_file`, `state_file_path` (+ 10 tests).
- `crates/mew/src/setup/providers.rs` — resolver signature now takes `&Config`,
  new `is_known_model` helper, 6 new tests for the corrupted-state case.
- `crates/mew/src/main.rs` — `prompt_yn`, `startup_state_health_check`,
  load `cfg` early, wired into all four resolve_provider/resolve_model_opt
  call sites (Run / Chat / Daemon / no-subcommand).

**Verification:**
- `cargo test -p mew --bin mew` → 66 passed
- `cargo test -p mew-tui --lib` → 135 passed
- `cargo test -p mew-config` → 116 passed (10 new)
- `cargo clippy -p mew --all-targets -- -D warnings` → clean
- `cargo fmt -p mew -- --check` → clean
- `just arch-check` → passes
- Manual E2E (via `expect`): heal-yes path created
  `state.toml.bak.1783495830` with the original content and rewrote
  `state.toml` keeping only `disabled_plugins = ["buddy"]`. Decline path
  left state unchanged and exited 0. Non-TTY path exited 2.

## 2026-07-08 — Surface connection errors in iOS reconnect UI

**Problem:** When the iOS app couldn't connect to a daemon over iroh
(e.g. pairing failures, relay unreachable, allowlist rejection), it just
showed "Waiting to retry. The daemon will reconnect automatically." with
no diagnostic info. The actual errors were `warn!`'d in the Rust core but
went nowhere on iOS (no `tracing` subscriber installed). Impossible to
diagnose without Console.app.

**Fix:** Threaded the connection failure reason through the event system
so it shows in the UI the user is already looking at.

1. `DaemonStatus::Backoff` gained an `error: String` field (dropped `Copy`
   from the enum since `String` isn't `Copy`). — `events.rs`
2. `connect_and_run` now returns `Result<ConnOutcome>` where `ConnOutcome`
   is `UserDisconnected` (stop) or `Dropped { reason }` (retry with
   reason). Every break point in the message loop sets a `drop_reason`
   ("connection error: {e}", "connection closed", "closed by daemon",
   "failed to send message to daemon"). The reconnect loop binds the
   reason from the match and passes it into the `Backoff` event. — `lib.rs`
3. `SessionRailView.connectingState` now renders the error string in red
   monospaced `.footnote` text below the status description when non-empty.
   Added `statusError` computed property that extracts it from the
   `Backoff` case. Updated all `.backoff` pattern matches in Swift for
   the new 2-field shape. — `SessionRailView.swift`
4. Regenerated Swift bindings + XCFramework via `just ios-core`.

**Files touched:**
- `crates/mew-mobile-core/src/events.rs` — `Backoff { error }`, drop `Copy`.
- `crates/mew-mobile-core/src/lib.rs` — `ConnOutcome` enum,
  `connect_and_run` return type + `drop_reason` tracking, reconnect loop
  error threading.
- `mew-ios/mew/SessionRailView.swift` — `statusError`, error display in
  `connectingState`, pattern match updates.
- `mew-ios/MewMobileCore/Sources/MewMobileCore/mew_mobile_core.swift` —
  regenerated bindings (auto).

**Verification:**
- `cargo build -p mew-mobile-core` → clean (no warnings)
- `cargo clippy -p mew-mobile-core --all-targets -- -D warnings` → clean
- `cargo test -p mew-mobile-core` → 19 passed
- `cargo fmt -p mew-mobile-core -- --check` → clean
- `just ios-core` → framework + bindings rebuilt
- `xcodebuild ... build` (iPhone 17 sim) → BUILD SUCCEEDED

## 2026-07-12 — render-rate plan written

Explored the TUI render pipeline (tick generator in mew-tui/src/events.rs, needs_redraw
gating in both loops in crates/mew/src/commands/tui.rs, chat_dirty cache) and wrote
notes/render-rate-plan.md: ms turn timer in the status bar, frame-time meter as a
render-stall detector, configurable tick rate with macOS refresh-rate autodetect
("auto" via core-graphics), and DEC 2026 synchronized-output brackets. Key decisions:
timer never lives in the cached chat content; frame meter never forces redraws (idle
stays at zero draws); spinner must move from tick-count to elapsed-time before the
tick rate becomes configurable. Implementation not started.
