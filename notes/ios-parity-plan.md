# iOS Parity Implementation Plan

> Produced 2026-07-05 from a three-way frontend parity audit (see CURRENT.md entry of the same date).
> Companion plans: `tui-parity-plan.md`, `web-parity-plan.md`.
> **Cross-plan dependencies:**
> - **Personas (Phase G)** are blocked on the protocol work owned by `web-parity-plan.md` item 2
>   (`ListPersonas`/`SwitchPersona`/`PersonaList`/`PersonaSwitched` — follow that naming, not `PersonaChanged`).
> - **Attachments (Phase C)** also depend on the daemon fix in `web-parity-plan.md` item 1: the daemon
>   currently **discards** `Prompt.attachments` (`mew-daemon/src/lib.rs:601` ignores the field; `run_turn`
>   passes `vec![]` to `run_with_parts`). iOS can send path-based attachments today, but they go nowhere
>   until that daemon change lands. Land the daemon side first (or together).

## Research verification (read before implementing)

Every audit claim was verified against on-disk code. Corrections and precisions:

- **Item 1 (permission modes):** Confirmed. `MobileCore::set_permission_mode` exists (`lib.rs:478`) and sends `ClientMessage::SetPermissionMode`. No AppStore wrapper (AppStore has `switchModel` but nothing for mode). `SessionReady` handler (`lib.rs:803`) only extracts `session_id` and drops `permission_mode`/`model`/`provider`. `PermissionModeChanged` and `ModelSwitched` fall through the catch-all (`lib.rs:1262`). Daemon side is fully wired (`mew-daemon/src/lib.rs:854`, broadcasts `PermissionModeChanged`; `SessionReady.permission_mode` populated at `lib.rs:423/467`). **Wire-complete.**
- **Item 2 (todos):** Confirmed. `ServerMessage::TodosUpdated { .. }` discards `todos` (`lib.rs:1220`), emits a payload-less `CoreEvent::TodosUpdated`. AppStore `case .todosUpdated: break` (`AppStore.swift:423`). Protocol `Todo { id, content, status, depends_on }` exists. **Wire-complete.**
- **Item 3 (subagents):** Confirmed. `ToolStart`/`ToolEnd`/`ToolProgress` and `SubagentStart`/`Status`/`End` all hit the catch-all. Only `SubagentPermissionRequest` is surfaced (as a generic permission, `lib.rs:1245`). No subagent/tool-progress records in `state.rs` or `events.rs`. **Wire-complete** (all six `ServerMessage` variants exist in `mew-protocol`). The TUI reference pattern is documented in CLAUDE.md ("Subagent run display").
- **Item 4 (slash):** Confirmed **and worse than stated.** Core emits `CoreEvent::SlashResult` (`lib.rs:1196`); AppStore no-ops it (`AppStore.swift:429`). But there is **no `slash_command` method on `MobileCore` at all** — `ChatView.send()` routes everything (including `/`-prefixed text) through `store.sendPrompt` → `ClientMessage::Prompt`. The daemon only produces `SlashResult` in response to `ClientMessage::SlashCommand` (`mew-daemon/src/lib.rs:712`), which iOS never sends. So slash commands are currently sent to the model as literal prompt text — broken, not just "results not displayed." **No wire listing exists for autocomplete** (no `ListSlashCommands`/`SlashCommandList`; see web plan item 10 for the proposed protocol addition).
- **Item 5 (attachments):** Confirmed. `MobileCore::prompt` hardcodes `attachments: vec![]` (`lib.rs:384`). `FileBrowserSheet.onPick` appends the path into `inputText` as plain text (`ChatView.swift:107`). Protocol `Attachment { path, mime }` exists and `Prompt` carries `Vec<Attachment>`. Wire types complete, **but see the cross-plan daemon dependency above** — the daemon drops the field today.
- **Item 6 (personas + thinking):** Thinking is **wire-complete** — `SetThinkingVariant`/`ThinkingVariantChanged` exist and are daemon-wired (`mew-daemon/src/lib.rs:822/844`); `ModelInfo.thinking_variants` exists but the core's `ModelSummary`/`ModelInfo` records **drop** it (see `events.rs:196`, `state.rs:88`). Personas: **only `ServerMessage::PersonaSwitchRequested` exists** — no client→server persona messages. Genuine protocol dependency owned by the web plan.
- **Item 7 (usage):** Confirmed. `CoreEvent::TurnEnded` carries `input_tokens`/`output_tokens`/`cost`/`failed` but AppStore ignores the payload (`AppStore.swift:380`). Snapshot `SessionInfo` (`state.rs:22`) only has `usage_cost` — no tokens/turns. `SessionSummary` (rail) already carries `input_tokens`/`output_tokens`/`turns`. `ModelSummary.context_window` exists. **Wire-complete.**
- **Item 8 (alerts):** Confirmed. Core emits `CoreEvent::Alert`; AppStore collects into `alerts` (`AppStore.swift:412`) but nothing renders them. `RootView` is a plain `NavigationStack` with no overlay. **Wire-complete.**
- **Item 9 (rename):** Confirmed. `AppStore.renameSession` fully wired (`AppStore.swift:313`). `SessionRailView` swipe actions have pin/archive/delete but no rename (`SessionRailView.swift:167-191`). **Quick win.**
- **Item 10 (auto-title/summary):** Confirmed. `SetAutoTitle`/`SetAutoSummary` daemon-wired (`mew-daemon/src/lib.rs:593/597`). Core exposes nothing; no UI. Note: neither `SessionReady` nor `SessionInfo` carries the current on/off state, so toggles are write-only (fire-and-forget) unless a getter is added daemon-side (out of scope). **Wire-complete (write path).**
- **Item 11 (retry):** Confirmed. `SessionSummary.last_turn_failed` and `SessionInfo.last_turn_failed` exist and reach the rail. No retry action. "Retry" has **no dedicated protocol message** — it means re-sending the last user prompt via `prompt()`. **Wire-complete.**

**Bonus finding (not in audit):** Block-level markdown is already done — `MessageItemView`/`StreamingBubble` use the `SwiftStreamingMarkdown` package (`MarkdownView`/`StreamedMarkdownView`). Improvement-plan item 3's markdown upgrade is complete; do not redo it.

**Net protocol dependency:** only **personas** need a wire change (owned by the web plan). Slash **autocomplete** optionally needs a `ListSlashCommands` addition (also web plan); everything else is wire-complete.

---

## Cross-cutting: the UniFFI regeneration workflow (do this once per batch)

Any change to the FFI surface — a new `CoreEvent` variant, a new `MobileCore` method, or a new/changed `uniffi::Record`/`uniffi::Enum` (`events.rs`, `state.rs`) — requires regenerating the Swift bindings. The generated file `mew-ios/MewMobileCore/Sources/MewMobileCore/mew_mobile_core.swift` is **do-not-hand-edit**.

Regen command (from `justfile:253`, target `ios-core`):
```
just ios-core
```
This: builds `mew-mobile-core` for `aarch64-apple-ios` + `-sim` + host cdylib, runs `uniffi-bindgen generate` from `target/release/libmew_mobile_core.dylib`, and rebuilds the XCFramework at `mew-ios/MewMobileCore/XCFramework/`. Prereqs (comment at `justfile:247`): `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` and `cargo install uniffi-bindgen-cli --version 0.32`.

**Batching rule (from improvement-plan risks):** binding regen is all-or-nothing and churns the generated file. Land all Rust core API changes for a group of items, run `just ios-core` once, then do the Swift work. The phases below are grouped so each triggers at most one regen. Write the Rust state-assembly unit test *before* regen (pattern: `state.rs` tests + `crates/mew-mobile-core/tests/m1_integration.rs`).

---

## Ordered work plan

Ordering rationale: quick wins first (visible value, no regen or trivial regen), then the medium core-carrying items grouped to minimize regen passes, then the large subagent item, then the protocol-blocked persona work last.

### Phase A — Pure-SwiftUI quick wins (no core change, no regen)

#### A1. Rename session UI (item 9) — S
Only layer: `SessionRailView.swift`.
- Add a rename swipe action alongside pin (leading) or in the trailing group. Pattern already present for delete: a `@State private var sessionToRename: SessionSummary?` + `@State private var renameText` driving an `.alert("Rename Session", ...)` with a `TextField` and Save button, mirroring `SettingsView.swift:43-55`'s rename-daemon alert.
- Save calls the existing `store.renameSession(session.sessionId, title: renameText)`. No AppStore or core change. Title refresh already flows via `SessionTitleChanged` → snapshot title, and rail titles refresh on `SessionList`.

#### A2. Retry affordance (item 11) — S
Layers: `AppStore.swift`, `ChatView.swift` (+ optionally `SessionRailView.swift`).
- AppStore already tracks the last prompt implicitly in core (`last_sent_prompt`) but not in Swift. Add `@Published var lastPrompt: [String: String]` keyed by sessionId (set in `sendPrompt`). Add `func retryLastTurn()` that re-calls `core.prompt` with `lastPrompt[selectedSessionId]`.
- In `ChatView`, when the newest assistant turn failed (surface via `turnEnded.failed`, see B4) show a "Retry" button below the last message or in an inline error banner. Optionally a rail affordance keyed on `SessionSummary.last_turn_failed`.
- No protocol/core change ("retry" = re-send). If `lastPrompt` is empty (fresh attach after failure), disable the button.

### Phase B — Core-carrying medium items, batch 1 (one regen)

This batch adds/changes `CoreEvent` variants, records, and `MobileCore` methods for items 1, 4, 7, 10, 2. Do all Rust first, one `just ios-core`, then Swift.

#### B1. Permission modes (item 1) — S/M
- **Rust `lib.rs`:**
  - `SessionReady` handler: store `permission_mode` (and `model`/`provider`) into `ConnState`. Add `permission_mode: Mutex<Option<String>>` to `ConnState` (mirror `session_title`). Emit a new `CoreEvent::PermissionModeChanged { daemon, mode }`.
  - Add handler arms for `ServerMessage::PermissionModeChanged { mode }` and `ServerMessage::ModelSwitched { provider, model }` (remove them from catch-all) → update `ConnState`, emit `CoreEvent::PermissionModeChanged` / a new `CoreEvent::ModelSwitched { daemon, provider, model }`.
  - `set_permission_mode` method already exists — no change.
  - Add `permission_mode` and `current_model`/`current_provider` to `DaemonSnapshot` (`state.rs`) and populate in `snapshot()`.
- **`events.rs`:** add `PermissionModeChanged` and `ModelSwitched` `CoreEvent` variants.
- **AppStore:** add `@Published var permissionMode: [String: String]` and `currentModel` keyed by daemon; add `func setPermissionMode(_ mode: String)` calling `core.setPermissionMode`; handle the two new events; seed from snapshot in `applySnapshot`.
- **SwiftUI (`ChatView`/`ChatBar`):** add a mode picker `Menu` next to `modelPickerChip` in the chatbar, options `standard/permissive/auto/auto_plus/dangerous` (label them nicely), checkmark on current. This also fixes the pre-existing "current model unknown" hack (`ChatView.swift:22-24`) since `ModelSwitched`/`SessionReady` now give an authoritative model.

#### B2. Slash commands + results (item 4) — S/M
- **Rust `lib.rs`:** add `pub fn slash_command(&self, id: DaemonId, command: String)` sending `ClientMessage::SlashCommand { command }` (mirror `cancel`). `SlashResult` handler already emits the event.
- **AppStore:** add `func sendSlashCommand(_ command: String)`; handle `.slashResult` by appending a synthetic system/assistant `ChatMessage` (role `"system"`, one text part) so it renders in the transcript, or a dedicated `@Published var slashResults` banner. Simplest: inject a `ChatMessage` into `messages`.
- **`ChatView.send()`:** if `trimmed.hasPrefix("/")`, call `store.sendSlashCommand(trimmed)` instead of `sendPrompt`. Update the empty-state hint text (already mentions slash passthrough).
- **Autocomplete (scope decision):** no wire listing exists. Option (a) hardcode a static list of common daemon slash commands (`/clear`, `/compact`, `/wiki`, …) in Swift for a menu above the composer — no protocol change, S. Option (b) adopt `ListSlashCommands`/`SlashCommandList` once the web plan's protocol addition lands. Recommend (a) for v1; (b) as follow-up.

#### B3. Usage / context display (item 7) — S/M
- **Rust:** thread tokens into the snapshot. Add `input_tokens`/`output_tokens`/`turns`/`context_window` to `SessionInfo` (`state.rs:22`) and to `SessionState` (accumulate from `MessageEnd` usage in `apply_provider_event`, currently only `cost` is accumulated at `state.rs:232`). Populate in `snapshot()`. Add `thinking_variants` and `context_window` to `ModelSummary`/`ModelInfo` (needed for the context-window denominator and for B6 thinking).
- **AppStore:** in `handleEvent(.turnEnded)` capture `input_tokens`/`output_tokens` into `@Published var usage: [String: SessionUsage]`. Provide a computed `contextUsageFraction` = latest turn input tokens ÷ current model `context_window`.
- **SwiftUI:** add a compact context/usage affordance — a small capsule in the chatbar or nav bar showing `~NN% ctx · $cost · N turns`. Tapping expands a small popover with tokens in/out and turn count. Reuse `FormatHelper.swift` / the existing `formatContext` in `ChatBar`.

#### B4. Turn-failure surfacing (supports A2, item 11) — S
- Already emitted via `CoreEvent::TurnEnded { failed }`. AppStore currently ignores it (`AppStore.swift:380`). Set `@Published var lastTurnFailed[sessionId]` from the event so `ChatView` can show the retry button and an inline error. No regen if done in the same batch as B1-B3.

#### B5. Todos panel (item 2) — M
- **Rust `events.rs`:** add a `TodoItem { id: u32, content, status, depends_on: Vec<u32> }` `uniffi::Record`; change `CoreEvent::TodosUpdated` to carry `todos: Vec<TodoItem>`.
- **`lib.rs`:** `TodosUpdated` handler stops discarding — map `todos` → `Vec<TodoItem>`. Store latest todos in `ConnState` (`todos: Mutex<Vec<TodoItem>>`) and add to `DaemonSnapshot` so re-attach restores them.
- **AppStore:** `@Published var todos: [TodoItem]` for the active session; populate in `.todosUpdated` and `applySnapshot`.
- **SwiftUI:** new `TodoPanelView` — a compact collapsible checklist above the composer. Render status as checkbox/spinner/strikethrough; dim items whose `depends_on` are unmet. Place as a `safeAreaInset(edge: .top)` or above the `ChatBar` in `ChatView`.

**→ Run `just ios-core` once after B1-B5 Rust work. Then all Swift work for B1-B5.**

#### B6. Thinking variants (item 6, thinking half) — M
Can ride the same batch as B (its only new record fields — `thinking_variants` — are already added in B3). If it slips, it's a second small regen.
- **Rust `lib.rs`:** add `pub fn set_thinking_variant(&self, id, variant: String)` → `ClientMessage::SetThinkingVariant`. Handle `ServerMessage::ThinkingVariantChanged { variant }` → store in `ConnState`, emit `CoreEvent::ThinkingVariantChanged { daemon, variant: Option<String> }`. Add current variant to snapshot.
- **`events.rs`:** `ThinkingVariantChanged` variant.
- **AppStore:** `func setThinkingVariant(_:)`, `@Published var thinkingVariant`, handle event.
- **SwiftUI:** a thinking-variant sub-menu inside the model picker `Menu` (each `ModelSummary` now carries `thinking_variants`; show them when the selected model has any, plus a "None" option).

### Phase C — Attachments (item 5) — M (one regen) — **depends on the daemon fix (web plan item 1)**

- **Rust `lib.rs`:** change `prompt` to `pub fn prompt(&self, id, text: String, attachments: Vec<AttachmentInput>)`; build `Vec<mew_protocol::Attachment>` instead of `vec![]` (`lib.rs:384`). Add an `AttachmentInput { path: String, mime: Option<String> }` `uniffi::Record` in `events.rs`.
- **AppStore:** change `sendPrompt(_ text:)` → `sendPrompt(_ text:, attachments:)`; hold `@Published var pendingAttachments: [AttachmentInput]`.
- **SwiftUI (`ChatView`/`ChatBar`/`FileBrowserSheet`):** `FileBrowserSheet.onPick` appends to `pendingAttachments` (daemon-local path) instead of inserting text into `inputText` (`ChatView.swift:107`). Render pending attachments as removable chips above the composer. `send()` passes them and clears. These are daemon-local paths (daemon owns the filesystem), matching the wire contract — no phone-photo upload (out of scope; would need the inline-bytes protocol option discussed in the web plan).

### Phase D — Subagents (item 3, the big one) — L (one regen)

This mirrors the TUI's runner→event→state→sidebar pattern (CLAUDE.md "Subagent run display" + "Adding a subagent-controlled UI affordance").
- **Rust `state.rs`:** add a `SubagentRun { parent_call_id, name, display_name: Option<String>, child_session_id, last_progress: Option<String>, status: String }` `uniffi::Record`. Add `subagents: Vec<SubagentRun>` to `SessionState` and to `DaemonSnapshot`/`SessionInfo`. Also add live tool-progress tracking: extend `MessagePart` with a running `tool_output` append path fed by `ToolProgress`.
- **Rust `lib.rs` — remove from catch-all and handle:**
  - `ToolStart { call_id }` → mark the matching tool part `running` (transition immediately instead of waiting for `PartUpdated`); emit `CoreEvent::PartUpdated`.
  - `ToolEnd { call_id, success }` → mark part `completed`/`error`; emit `PartUpdated`.
  - `ToolProgress { call_id, chunk }` → append to the tool part's `tool_output`; emit a coalesced `CoreEvent::ToolProgress { daemon, session_id, call_id, chunk }` (or reuse `PartUpdated`).
  - `SubagentStart` → push a `SubagentRun`; emit `CoreEvent::SubagentStarted { … }`.
  - `SubagentStatus { parent_call_id, message }` → update `last_progress`; emit `CoreEvent::SubagentStatus`.
  - `SubagentEnd { outcome }` → set terminal status; emit `CoreEvent::SubagentEnded`.
- **`events.rs`:** add the four/five new `CoreEvent` variants + `SubagentRun` if not in `state.rs`.
- **AppStore:** `@Published var subagents: [SubagentRun]`; handle the new events (upsert by `parent_call_id`); seed from snapshot.
- **SwiftUI (`MessageItemView` + new `SubagentPanelView`):** render subagent runs as sub-rows under their parent tool call using the TUI's `↳ last_progress` pattern (a `SubagentRow` showing `display_name (name)` + spinner + `↳ last_progress`, expandable). `ToolCallRow`/`ToolGroupRow` already model running/error states; extend to show live `ToolProgress` output in the expanded detail. `SubagentPermissionRequest` already surfaces as a labeled permission (`lib.rs:1245`) — keep, but now correlate it to its `SubagentRun` for a better label.

### Phase E — Alerts / in-app banner (item 8) — M (no core change; no regen)

Core already emits `Alert`; AppStore already collects `alerts` (`AppStore.swift:127,412`).
- **AppStore:** add `@Published var activeBanner: AlertItem?` set on `.alert` when the alert's session isn't the one on screen (compare to `selectedSessionId`); a "clear on attach" step in `selectSession` mirroring web's `clearAlertsForSession`. Aggregate a `needsYouCount` for a future badge.
- **SwiftUI (`RootView`):** overlay a transient banner/toast at the top (`.overlay(alignment: .top)`), tap navigates via `store.path.append(.chat(...))`. Auto-dismiss after a few seconds.
- **Flag as future (out of v1 scope):** APNs/background push and `UNUserNotificationCenter` local notifications + app badge + `scenePhase` suspend/resume (improvement-plan item 2). Note it explicitly so the banner isn't mistaken for push.

### Phase F — Auto-title / auto-summary toggles (item 10) — S/M (rides a regen batch)

- **Rust `lib.rs`:** add `pub fn set_auto_title(&self, id, enabled: bool)` and `set_auto_summary` → `ClientMessage::SetAutoTitle`/`SetAutoSummary`.
- **AppStore:** thin wrappers; persist the user's chosen default in `UserDefaults` (these are write-only — no current-state getter on the wire; the toggle reflects local intent, not confirmed daemon state, unless a daemon-side getter is added later, which is out of scope).
- **SwiftUI (`SettingsView` or a per-session chat menu):** two toggles. Per-session placement is more correct (the messages are per-session), but Settings is simpler; recommend a chat-view overflow menu.

### Phase G — Personas (item 6, personas half) — M, **BLOCKED on protocol (web plan item 2)**

Do not start core/UI until `ListPersonas`/`SwitchPersona`/`PersonaList`/`PersonaSwitched` land in `mew-protocol` and the daemon.
- **When unblocked — Rust:** `list_personas`/`switch_persona` methods; handle `PersonaList`/`PersonaSwitched` → `CoreEvent::PersonaList`/`PersonaSwitched`; store current persona + list in `ConnState`/snapshot.
- **AppStore + SwiftUI:** persona picker `Menu` in the chatbar or a session menu, mirroring the model picker. Also handle the existing `PersonaSwitchRequested` (currently catch-all) to reflect tool-driven switches.

---

## Suggested landing sequence

1. **A1, A2** (quick wins, ship immediately — no regen).
2. **Batch 1 regen:** B1 + B2 + B3 + B4 + B5 (+ B6 if ready) Rust → `just ios-core` → Swift for all.
3. **C** (attachments) Rust → regen → Swift — after/with the daemon-side attachment fix (web plan item 1).
4. **D** (subagents) Rust → regen → Swift — the large one; isolate it.
5. **E** (alerts banner — no regen), **F** (auto-title/summary — fold into the next available regen).
6. **G** (personas) once the web plan lands the protocol.

Every Rust handler added in B/C/D needs a `state.rs`-style unit test (delta/message stream → expected snapshot) plus, where a round-trip matters, an addition to `crates/mew-mobile-core/tests/m1_integration.rs`, written before the regen.

## Size summary
S: A1, A2, B1, B2, B4, F(core). M: B3, B5, B6, C, E, G. L: D.

## Critical files

- `crates/mew-mobile-core/src/lib.rs`
- `crates/mew-mobile-core/src/events.rs`
- `crates/mew-mobile-core/src/state.rs`
- `mew-ios/mew/AppStore.swift`
- `mew-ios/mew/ChatView.swift`

Secondary (per item): `mew-ios/mew/SessionRailView.swift` (A1), `mew-ios/mew/MessageItemView.swift` (D), `mew-ios/mew/RootView.swift` (E), `mew-ios/mew/SettingsView.swift` (F), `justfile` (regen target `ios-core`), `crates/mew-protocol/src/lib.rs` (personas only).
