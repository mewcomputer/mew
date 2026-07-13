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
