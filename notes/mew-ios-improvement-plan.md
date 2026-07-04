# mew ios improvement plan

Context: the iOS app has outrun its spec (`mew-ios-app-spec.md`) — m0 through most of m3 exist: pairing with QR scan, daemon list, session rail with needs-you ordering, streaming chat, permission/ask sheets, model switching, cancel. This plan is the gap list between what exists and a solid v1, ordered by what hurts most. Companion plans: `mew-clients-multi-workspace-plan.md` (project picker, sequenced after this), `mew-mobile-iroh-plan.md` (transport history).

Status: planned, not started. ~570 lines of uncommitted work in the tree must land first.

---

## Current state (audited 2026-07-03)

Working, against the spec's v1 goals:
- Pairing: QR scan (`QRScannerView`, in flight) + paste; `computer.mew.mew://<node_id>` URL scheme both sides (in flight); stable daemon NodeId via persisted secret key (`load_or_create_secret_key`, in flight) — the spec's blocker #1 is fixed.
- Daemon list with status, swipe actions (in flight); session rail with pending > running > active > idle ordering, archive filter, rename/pin/delete.
- Chat: history replay + live streaming. Core honors the spec's hard-won notes: lenient codec (`codec.rs`), TextDelta coalescing before FFI (`FLUSH_INTERVAL`), UserMessage dedupe, PartUpdated-authoritative.
- Permission, workspace-permission, subagent-permission, and ask-user sheets; `RequestResolved` dismissal.
- Model list/switch, permission mode set, cancel, custom fonts (in flight).
- Core tests: `m0_spike.rs`, `m1_integration.rs`, unit tests in codec/state/registry.

## Gap analysis

### 1. state correctness in mew-mobile-core (the big one)

The core handles 19 `ServerMessage` variants; the spec's must-handle list is ~30. Missing, by consequence:

- **Turn lifecycle is broken.** `SessionState.running` is never set `true` anywhere, and `MessageEnd` sets it `false` (`state.rs:225`) — but `MessageEnd` fires after *every* assistant message in an agentic loop, mid-turn. `TurnComplete` / `TurnFailed` are unhandled. Consequences: no reliable send/cancel swap, no turn-failure surfacing, usage only accumulates from per-message `cost`.
- **Rail staleness.** `SessionActivityChanged`, `SessionUsageChanged`, `SessionSummaryChanged`, `SessionMetaChanged` unhandled — session state strings, costs, and titles in the rail only update on a full `ListSessions` refresh.
- **Errors are silent.** `Error` / `ErrorEvent` unhandled: a failed prompt, a rejected message, a daemon-side error — nothing reaches the UI.
- **No cross-device confirmation.** `ModelSwitched` / `PermissionModeChanged` unhandled: switching a model from the TUI or web while the phone is attached leaves the phone's picker stale.
- **Tool state lags.** `ToolStart` / `ToolEnd` / `ToolProgress` unhandled; tool rows only move when `PartUpdated` arrives.
- **Subagents invisible.** `SubagentStart` / `SubagentStatus` / `SubagentEnd` unhandled — the spec's `↳ last progress` affordance has no data.

### 2. app lifecycle (spec m4, entirely absent)

- No `scenePhase` handling in `MewApp`. On background, the core's reconnect loop keeps dialing against a radio iOS is killing (battery, log noise). On foreground, a dead connection can sit in backoff up to 30s before redial. Spec behavior: background → suspend reconnection; foreground → immediate redial, re-attach, snapshot swap.
- No local notifications, no badge. `SessionAlert` is handled by the core but the app never asks for notification permission or posts `UNUserNotificationCenter` alerts; the headline "phone tells me when the agent needs me" (foreground version) doesn't exist. Badge = pending permissions + questions across daemons, cleared on attach.
- No one-time explainer that background push doesn't exist yet (spec risk: v1 reads as broken without it).

### 3. chat rendering

- Markdown is `AttributedString(markdown:)` — inline-only. Code blocks, lists, headings flatten to plain text. Agent output is code-heavy; this is the spec's open question 3 answered by experience: upgrade. Options: swift-markdown-ui (dependency, full GFM) vs a small block-splitter (fences/headings/lists) feeding AttributedString runs. Lean swift-markdown-ui unless binary-size objections appear.
- Todos: `TodosUpdated` reaches `AppStore` and stops — no UI. A compact collapsible checklist above the composer matches the web treatment.
- Subagent status lines (needs the events from gap 1).

### 4. hygiene

- No CI job builds the iOS targets (`aarch64-apple-ios{,-sim}`); a `cargo check` job catches core breakage before Xcode does. (Spec open question 4 — answer: yes, dedicated job.)
- Every new `ServerMessage` handler in gap 1 needs a state-assembly unit test; `m1_integration.rs` is the pattern for the turn-lifecycle round-trip.

## Work items

### 0. land the in-flight work

Commit the ~570 uncommitted lines (QR pairing scheme + scanner, stable daemon key, runtime-handle and reconnect-loop fixes, fonts, swipe actions, settings). Split sensibly: core/daemon changes and app changes at minimum. Everything below stacks on this.

### 1. turn + session state correctness (mew-mobile-core, then bindings)

- `running = true` on prompt send (core-side, in `prompt()`) and on observing an active turn in replay; clear on `TurnComplete`/`TurnFailed`, not `MessageEnd`.
- Handle `TurnComplete { usage }` / `TurnFailed` → `CoreEvent::TurnEnded { usage, failed }` (spec shape).
- Handle the rail-staleness quartet → targeted `CoreEvent`s updating the session summaries.
- Handle `Error`/`ErrorEvent` → `CoreEvent::ErrorOccurred`; Swift shows a toast/inline error.
- Handle `ModelSwitched`/`PermissionModeChanged` → update snapshot state, event for pickers.
- Handle `ToolStart/ToolEnd/ToolProgress` for immediate tool-row transitions.
- Tests first for each handler (delta stream → expected state), then regenerate bindings once.

### 2. lifecycle + notifications (SwiftUI, small core hook)

- `scenePhase` observer: background → `core.suspend()` (new: parks reconnect loops); foreground → `core.resume()` (immediate redial, reset backoff), snapshot swap on reattach.
- Notification permission ask on first pairing; local notification for `SessionAlert` on non-visible sessions; app badge from aggregate attention counts; clear on attach. One-time explainer sheet about foreground-only alerts.

### 3. chat rendering

- Block-level markdown (decision above).
- Todos panel.
- Subagent status sub-rows in tool call rows (data from item 1).

### 4. CI

- Workflow job: `cargo check -p mew-mobile-core --target aarch64-apple-ios` (+ sim target) on the existing toolchain matrix.

Then: `mew-clients-multi-workspace-plan.md` items 3 (iOS project picker) slot in after item 1 here, since both touch `AppStore`/`SessionRailView`/bindings.

## Risks / watch items

- Bindings regeneration is all-or-nothing: batch core API changes (items 1 + multi-workspace core additions) to avoid repeated `mew_mobile_core.swift` churn across parallel work.
- `running` on prompt-send is optimistic — a rejected prompt (daemon error) must clear it via the `Error` handler or the composer wedges. Test that path explicitly.
- swift-markdown-ui pins a dependency in an otherwise dependency-light app; if it's heavy, the block-splitter fallback is a day's work, not a rewrite.
