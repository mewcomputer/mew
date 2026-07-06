# Web UI Parity Plan — mew chat frontend

> Produced 2026-07-05 from a three-way frontend parity audit (see CURRENT.md entry of the same date).
> Companion plans: `tui-parity-plan.md`, `ios-parity-plan.md`. This plan **owns the persona protocol
> design** (`ListPersonas`/`SwitchPersona`); the iOS plan depends on it.

Every audit claim was verified against the code. Two claims needed correction and are flagged with **[audit correction]**.

## Key architecture facts (so steps don't need re-derivation)

- **Wire flow**: `crates/mew-protocol/src/lib.rs` (`ClientMessage`/`ServerMessage`) ⇄ `mew-web-client/src/index.ts` (`MewClient` + `MewClientEvents`) ⇄ `mew-web-ui/src/stores/session.ts` (`bridgeClientToStore` maps every event to a Zustand action) ⇄ components via `useSessionStore(selector)`.
- **Client access in components**: props where available, else `getClient()` from `lib/client-ref.ts`.
- **Daemon dispatch**: `crates/mew-daemon/src/lib.rs` has one big `match ClientMessage` (around lines 590–830). Model/thinking operations are delegated to injected closures on `SessionManager` (`lister`, `switcher`, `thinking_setter`). Persona operations can instead go **directly through the session's `Agent`**, which already carries `agent.personas: Vec<Persona>` and `agent.apply_persona(&persona)` (loaded via `set_personas` in the shared builder at `crates/mew/src/main.rs:1440`).
- **Adding a wire feature** (per `mew-web-ui/CLAUDE.md`): add type in `mew-web-client`, rebuild it (`cd mew-web-client && pnpm build`), add store field+action+bridge handler, then render. For Rust wire types, edit `crates/mew-protocol/src/lib.rs` and keep the TS mirror in sync (there are roundtrip tests in the same file to copy as a pattern).

## Duplicate-implementation cleanup (do opportunistically while touching these areas)

- **`mew-web-ui/src/components/subagent-panel.tsx` is dead code** — zero imports anywhere. It duplicates `SubagentRailPanel` inside `right-rail.tsx` (the "Subs" tab). Delete it when working on item 7 (presence) or item 8 (jobs), since those touch the same rail. No consolidation logic needed; the rail version is strictly better.
- **Two theme pickers**: `components/settings-modal.tsx` (`SettingsModal` dialog) is **unused** (zero imports); the live one is the `/settings` route (`routes/settings.tsx`). Delete `settings-modal.tsx` when touching settings for items 2/10/11. They are byte-for-byte the same grid logic, so there's nothing to merge — just remove the orphan.

---

## Wave 1 — Quick wins (no protocol changes, mostly store + one component each)

### 9. Retry-wait toast — **S**  *(web only)*
`retry_wait` is silently dropped at `stores/session.ts:569`. A `Toaster` component exists (`components/ui/sonner.tsx`) but is **never mounted** and `toast()` is never called.
- Mount `<Toaster />` once in `routes/__root.tsx` (inside `SidebarProvider`).
- In `stores/session.ts` `onProviderEvent` `case "retry_wait"`, call `toast()` from `sonner` (e.g. "Provider retry 2/5 — waiting 4s (rate_limit)"). The event fields are `attempt`, `max_attempts`, `delay_secs`, `reason`.
- Optional: also toast on `case "error"`.

### 11. Daemon version display — **S**  *(web only)*
`client.ping()` returns the version (`pong.version`), but nothing calls it. Daemon replies at `lib.rs:655` with `env!("CARGO_PKG_VERSION")`.
- Add `daemonVersion: string | null` + `setDaemonVersion` to `stores/session.ts`.
- In `lib/hooks.ts` `useMewConnection` (after `client.connect()` succeeds, next to `client.listModels()`), call `client.ping().then(setDaemonVersion)`. Alternatively add a `pong` handler in `bridgeClientToStore`.
- Render in `components/status-footer.tsx` (the currently-empty right side, line 59) or in the `/settings` route header.

### 8. Background jobs surfacing — **S/M**  *(web only)*
`job-update` is only `console.debug`'d (`stores/session.ts:1046`). The `ServerMessage::JobUpdate { job_id, command, state }` and client event already exist — **no protocol change needed**. **[audit correction]** there is no "list jobs" wire call, so the UI can only reflect the update stream (accumulate a map), not query a snapshot; that's the same limitation the events impose and is fine for parity.
- Add `jobs: Map<string, { jobId; command; state }>` + `onJobUpdate` action to the store; wire it in `bridgeClientToStore` (replace the console.debug). Drop entries when `state` is `done`/`failed` after a short delay, or keep and style them.
- Render: add a **"Jobs" tab** to `components/right-rail.tsx` (`TabKey` union + `TabButton` + panel), mirroring `SubagentRailPanel`. This is where you should also delete `subagent-panel.tsx`.

### 3. Input history recall (↑/↓) — **S/M**  *(web only)*
`components/input-area.tsx` `handleKeyDown` uses ArrowUp/Down only for the slash/persona menu (lines 68–75); no prompt recall exists.
- Keep a history array. Simplest: local `useRef<string[]>` in `InputArea`, pushed on `handleSubmit`. Better (survives remounts/route changes): add `promptHistory: string[]` + `pushPromptHistory` to the store, populated in the session route's `handleSend` or in `addUserMessage`.
- In `handleKeyDown`, when the menu is closed: ArrowUp at caret-start (or when textarea is empty/single-line) walks back through history; ArrowDown walks forward; typing resets the cursor. Guard so it doesn't fight multiline editing (only recall when caret is on the first line for Up / last line for Down).

---

## Wave 2 — Larger web-only features (client methods already exist, unused)

### 7. Presence / yield control — **M**  *(web only)*
`attachedClients` and `yieldedByClient` are tracked in the store (lines 185–188, updated by bridge handlers) but never rendered; `client.yieldControl()` exists and is uncalled.
- Render attached clients as small presence chips (e.g. in `status-footer.tsx` or `fake-header.tsx`) using `attachedClients` (`{id, kind}`).
- Add a "Yield control" affordance (button in footer or command palette) calling `getClient().yieldControl()`. When `yieldedByClient` is set, show a banner/badge and expose "take control" (any prompt implicitly takes control; a `cancel`/`prompt` re-activates — no dedicated wire message needed, advisory only per the client doc).
- Delete `subagent-panel.tsx` here if not already removed in item 8.

### 6. File service (tree + preview + git) — **L**  *(web only)*
`client.listDir/readFilePreview/gitStatus/watchWorkspace/openPath` all exist and are **completely uncalled**. Store already has `dirListing`, `dirListingPath`, `filePreview`, `gitStatus` state and their `on*` handlers wired in `bridgeClientToStore` (lines 1129–1134). This is purely a missing UI.
- New component `components/file-tree.tsx` (+ a `file-preview.tsx` or inline): on mount call `client.listDir(sessionId)`; render `DirEntry[]`; clicking a dir re-calls `listDir` with its path; clicking a file calls `readFilePreview` and shows `filePreview.content` (use existing `components/code-block.tsx`/shiki for highlighting via `filePreview.language`).
- Add a **"Files" tab** to `right-rail.tsx` (same `TabKey`/`TabButton` pattern), or a new left/right panel. Given the rail is already tab-dense, consider gating behind the existing tab row.
- Wire `gitStatus` into the existing **"Changes" tab** (`ChangesPanel` currently only reads `change_stats`; augment with live `gitStatus` entries).
- Optional: call `watchWorkspace(sessionId, true)` on tab open to get `fs_changed` events (store's `onFsChanged` is currently a no-op — make it re-fetch `listDir`/`gitStatus`).
- `openPath` can back a "reveal in editor" button per file.

### 5. Session-group management — **M/L**  *(web only)*
`GroupInfo[]` is in the store; `session-rail.tsx` renders a read-only "grouped" view (lines 141–160). `client.createGroup/updateGroup/deleteGroup/assignSessionGroup` all exist and are uncalled.
- In `session-rail.tsx` grouped view: add a "New group" button (`createGroup(name, color)`), inline rename (`updateGroup(id, {name})`), color swatch picker (`updateGroup(id, {color})`), delete (`deleteGroup(id)`).
- Assign sessions via the existing per-session hover actions (line 286 area) — a dropdown "Move to group" calling `assignSessionGroup(sessionId, groupId, position)`. Reuse `components/ui/dropdown-menu.tsx`.
- Reorder: `updateGroup(id, {order})` and `assignSessionGroup(..., position)` support it; a lightweight up/down button pair is enough for parity (full drag-and-drop is optional and would pull in a dnd dep — recommend deferring).
- All mutations already broadcast back via `groups-changed`/`session-meta-changed`, so no optimistic-update bookkeeping is required.

---

## Wave 3 — Requires protocol / daemon (and bridge) changes

### 2. Personas — real server-backed switching — **M (web) + M (Rust)**
**[verified]** There is currently **no** persona `ClientMessage` at all. `store.setCurrentPersona` and both `PersonaPill`/`InputArea` `@`-menus are hardcoded 3-item local stubs (`["default","code-reviewer","explainer"]`) that never touch the server. The only wire artifact is server→client `PersonaSwitchRequested`. Daemon session agents already hold `agent.personas` and `agent.apply_persona()`.

**Precise wire changes** (`crates/mew-protocol/src/lib.rs`):
- Add `ClientMessage::ListPersonas`.
- Add `ClientMessage::SwitchPersona { name: String }`.
- Add `ServerMessage::PersonaList { personas: Vec<PersonaInfo> }` where `PersonaInfo { name: String, description: String, color: Option<String>, active: bool }`.
- Add `ServerMessage::PersonaSwitched { name: String }` (confirmation distinct from the existing tool-queued `PersonaSwitchRequested`). Add roundtrip tests mirroring the existing `server_message_persona_switch_roundtrip` test.

**Daemon handlers** (`crates/mew-daemon/src/lib.rs`, in the `match ClientMessage`):
- `ListPersonas`: lock `attached_session.agent`, map `agent.personas` → `PersonaInfo` (mark `active` by comparing to `agent.persona_name`), `reply(PersonaList{..})`.
- `SwitchPersona { name }`: lock agent, find in `agent.personas` by name, clone, call `agent.apply_persona(&persona)`; if it returns a pinned `provider/model`, apply via the same `switcher` path used by `SwitchModel` (mirror the `apply_persona_switch` helper at `crates/mew/src/main.rs:98`, minus TUI bits). Broadcast `PersonaSwitched { name }` (and `ModelSwitched` if the pin changed the model). Take `turn_lock` like the other mutating handlers.

**web-client** (`mew-web-client/src/index.ts`): add the two `ClientMessage` variants, `listPersonas()`/`switchPersona(name)` methods, `PersonaInfo` type, `persona-list`/`persona-switched` events + dispatch cases.

**web-ui**:
- Store: add `availablePersonas: PersonaInfo[]` + `setAvailablePersonas`; change `persona-switch-requested`/new `persona-switched` handlers to update `currentPersona`.
- `lib/hooks.ts`: call `client.listPersonas()` on connect (next to `listModels()`).
- Replace the hardcoded arrays in `components/persona-pill.tsx` (line 7) and `components/input-area.tsx` (`PERSONA_OPTIONS` line 15) with `availablePersonas`; selecting one calls `getClient().switchPersona(name)` instead of the local-only `setPersona`.

### 1. Attachments — **L (web) + M (bridge) + M (daemon)**  *the big one*
**[audit correction / deeper than stated]** The gap is not only web-UI wiring. Three layers drop attachments:
1. `components/input-area.tsx` `handleSubmit` calls `onSend(trimmed)` with **no** files and immediately `setFiles([])` (lines 139–141) — the picker chips are decorative. (Note: the session route's `handleSend` *does* already accept and forward an `attachments` param to `prompt()`, so only `InputArea` needs to pass them up.)
2. The daemon **discards attachments**: `ClientMessage::Prompt { text, .. }` (`lib.rs:601`) ignores the field, and `run_turn` calls `agent.run_with_parts(prompt_text, vec![], ...)` (`lib.rs:1163`) with empty parts. So even the existing path-based `Attachment{path,mime}` never reaches the agent.
3. `Attachment` is **path-based** on the wire, but a browser `File` has no daemon-accessible path — the browser must ship bytes.

**Decision to make first (blocks the rest):** how bytes reach the daemon. Two options:
- **(A) Bridge upload endpoint (recommended).** `crates/mew-web-bridge/src/main.rs` currently only serves static assets + proxies WS (`serve_http`, line 159). Add a `POST /upload` handler that writes the body to a temp dir the daemon can read and returns a path; the UI then sends that path in `Attachment.path`. Keeps the wire path-based. Requires the bridge and daemon to share a filesystem (already true for local use; note this breaks the iroh/remote transport in `iroh_transport.rs`).
- **(B) Inline bytes in the protocol.** Extend `Attachment` with an optional `data: Option<String>` (base64) or `bytes`, teach the daemon to materialize it. Transport-agnostic (works over iroh) but bloats WS frames and needs a size cap. This is a wire change to `crates/mew-protocol/src/lib.rs`.

**Daemon work (needed under either option):** in `ClientMessage::Prompt`, stop discarding `attachments`; convert each `Attachment` into a `Part::file` (read the path/bytes, infer mime) and pass the `Vec<Part>` through `run_turn` → `agent.run_with_parts(text, parts, token)` (the agent signature already accepts attachments). Verify `mew-provider-*` adapters actually forward image parts to the model.

**web-ui work:** in `input-area.tsx`, thread `files` into `onSend` (upload via bridge → paths under option A, or base64-encode under option B), build `Attachment[]`, pass to `onSend(trimmed, attachments)`; the route's `handleSend` already forwards them.

Recommend option A for scope, with a note that remote/iroh transport needs option B later.

### 4. Paste / drag-drop images — **S** *(web)*, **depends on item 1**
No `onPaste`/`onDrop` handlers exist on the textarea/composer. Once item 1's file→attachment pipeline works, add `onPaste` (read `clipboardData.files`/image items) and `onDrop`/`onDragOver` on the composer container in `input-area.tsx`, funneling into the same `handleAttach` path. Trivial after item 1; near-zero value before it.

### 10. Dynamic slash commands — **S (web) + M (Rust) if done properly**
`SLASH_COMMANDS` is hardcoded to `/clear /compact /help` in `input-area.tsx` (line 29). The daemon actually handles `/clear`, `/compact`, `/wiki` (`lib.rs:729`) plus hook-registered commands via the `on_register_slash_commands` dispatcher hook — so the hardcoded list is both incomplete and static.
- **Minimal (web-only, S):** update the hardcoded list to match what the daemon really supports (`/clear`, `/compact`, `/wiki`; drop `/help` which the daemon returns `None` for).
- **Proper (needs protocol, M):** add `ClientMessage::ListSlashCommands` + `ServerMessage::SlashCommandList { commands: Vec<{name, description}> }`; daemon builds the list from its built-ins plus `dispatcher.on_register_slash_commands()` (`SlashCommandDef` already exists in `mew-hooks`). Then store `availableSlashCommands` and render dynamically. Recommend shipping the minimal fix now and the protocol version alongside item 2 (same protocol/daemon touch).

---

## Suggested execution order

1. **Wave 1** (9 → 11 → 8 → 3): each is a self-contained S/M, immediate value, no cross-deps. Mount `Toaster` first (unblocks 9 and any future toasts). Delete `subagent-panel.tsx` during item 8.
2. **Wave 2** (7 → 6 → 5): web-only, larger. Delete `settings-modal.tsx` opportunistically. Item 6 is the biggest; can be deferred if capacity is tight.
3. **Wave 3**: do **2 (personas)** first — cleanest protocol change, high value, and it lets you land the item-10 protocol addition in the same daemon edit. Then **1 (attachments)** after deciding bridge option A vs B; **4 (paste/drop)** immediately follows 1.

Size roll-up: 9=S, 11=S, 8=S/M, 3=S/M, 7=M, 6=L, 5=M/L, 2=M+M, 1=L+M+M, 4=S, 10=S (or M if protocol).

## Separation summary

- **Web-only** (no Rust): 3, 5, 6, 7, 8, 9, 11, and the minimal form of 10.
- **Protocol + daemon**: 2 (`ListPersonas`/`SwitchPersona`/`PersonaList`/`PersonaSwitched`), proper 10 (`ListSlashCommands`/`SlashCommandList`).
- **Protocol/bridge + daemon**: 1 (daemon must stop dropping `attachments` and forward `Part::file`; bridge upload endpoint or inline-bytes protocol change to get browser bytes server-side). 4 rides on 1.

## Critical files

- `mew-web-ui/src/stores/session.ts`
- `mew-web-ui/src/components/input-area.tsx`
- `mew-web-ui/src/components/right-rail.tsx`
- `crates/mew-protocol/src/lib.rs`
- `crates/mew-daemon/src/lib.rs`
