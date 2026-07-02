# CURRENT.md — mew Fixes + Right Rail Redesign Progress

## 2026-07-02: Web UI Fixes Plan (6 Phases Complete)

### Phase 1: Crashes and alert correctness
- **1.1**: Moved `useState` above early return in `TodoRailPanel` (hooks crash fix)
- **1.2**: Added `SessionManager::broadcast_all()` — permission/input alerts now reach ALL sessions' clients via `forward_events` diverting `SessionAlert` messages
- **1.3**: Hoisted turn-end alert broadcast out of the usage block — failed turns before any `MessageEnd` now produce `TurnFailed` alerts
- **1.4**: `Notification.requestPermission()` called when permission is "default" (was never requested)
- **1.5**: Created `lib/router-ref.ts` with `navigateToSession()` — notification clicks use router-ref instead of broken `window.location.hash`
- **1.6**: Added `clearAlertsForSession(sessionId)` + `dismissAlert(sessionId, timestamp)` to store. `syncTitleBadge()` helper updates `document.title`. Called from `useSessionAttach` on session switch.
- **1.7**: Moved composer focus from ⌘K to ⌘L. Added ⌘N for new session in `__root.tsx`.
- **1.8**: Command palette theme toggle now uses `useTheme()` context + cycles through `THEMES` instead of writing `data-theme` directly.

### Phase 2: Flagged files lifecycle
- **2.1**: `reset()` now clears `flaggedFiles`, `dirListing`, `dirListingPath`, `filePreview`, `gitStatus`
- **2.2**: `flagged-files-changed` bridge handler guards with `data.session_id === sessionId`
- **2.3**: `AttachSession` handler replays current flagged-files set via `FlaggedFilesChanged`
- **2.4**: Shared `flag_mode_label()` helper in `mew-tools/flag_important.rs` — all three emit sites (agent, daemon unflag, attach replay) use the same function

### Phase 3: Session rail truthfulness
- **3.1**: `list()` checks `session.is_running` and reports `Running` state
- **3.2**: Added `SessionMetaChanged` broadcast for archive/pin/assign-group — all clients update their rail. Removed no-op `broadcast_groups` from archive.
- **3.3**: Alert titles use `session.display_title()` (custom title > summary > id) instead of raw ULID

### Phase 4: /wiki turn management refactor
- Extracted `run_turn()` helper encapsulating: cancel token, is_running, activity broadcasts, forward_events, meta/usage updates, alert broadcast
- Both Prompt handler and /wiki handler route through `run_turn()`
- Title generation stays Prompt-only (gated on `!had_error`)
- /wiki now reports failure in SlashResult text

### Phase 5: Tests
- Vitest store tests: alert lifecycle (push/clear/dismiss), flagged files (set/reset), session meta changes, attention changes, activity/usage updates (9 tests, all passing)

### Phase 6: "Needs you" ordering
- Added `pending_permissions`/`pending_questions` to `SessionInfo`
- `SessionAttentionChanged` broadcast on permission/ask-user create + resolve
- `statePriority()` tier 0 = needs attention (amber pulsing dot), excludes current session
- Session rail sorts needs-attention above running above active above idle
