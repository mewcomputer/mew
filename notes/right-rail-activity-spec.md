# right rail redesign — activity timeline, changes, gauge, alerts

Context: the right rail (`mew-web-ui/src/components/right-rail.tsx`) is
currently three static tabs (Todos / Subagents / Questions) plus a "Pinned
Context" section, rendered in a slide-over `Sheet` opened from
`fake-header.tsx:63`. This spec turns it into a live "what is this session
doing" surface. Ideas and impact-ordering are natalie's; plumbing notes were
verified against the working tree on 2026-07-02.

Depends on `notes/web-ui-fixes-plan.md` landing first — specifically 1.2
(`broadcast_all`), 1.5 (router-ref for navigation outside React), 1.6 (alert
clearing + title badge), and 3.3 (alert titles from session title, not ULID).
Build on top of that branch.

Same ground rules as the fixes plan: WIP branch, small commits, `just ci`,
rebuild `mew-web-client` after any wire change, shadcn components only via
`pnpm dlx shadcn@latest add <item>`, append to CURRENT.md per phase.

---

## layout decision (natalie must confirm before starting)

The rail is a Sheet — closed by default. An "alive" timeline and an alert
banner have no value inside a closed overlay. **Recommended: dock the rail as
a persistent column on wide screens** (e.g. ≥ `xl`), keeping the Sheet
behavior below that. The desktop layout diagram in `mew-web-ui/CLAUDE.md`
already describes it as a column, so this brings the code in line with the
documented design. This is a `session.$sessionId.tsx` / `fake-header.tsx`
restructure; the rail's inner content is shared between docked and sheet
modes.

If natalie says no, everything below still works, but the alert banner (P0.2)
should render at the top of the chat surface instead of inside the rail.

Proposed rail structure, top to bottom:

```
┌───────────────────────────────┐
│ alert banner (if alerts > 0)  │  ← P0.2
│ context gauge (thin bar)      │  ← P0.1
│ pinned context (existing)     │
│ [ Activity | Changes ]        │  ← two tabs replace three
│   activity: filter chips +    │  ← P1
│     chronological feed        │
│   changes: touched files +    │  ← P2
│     expandable diffs          │
└───────────────────────────────┘
```

Todos / Subagents / Questions stop being tabs; they become entry types in the
Activity feed (with filter chips). The existing panels' rendering logic
(`TodoRailRow`, `SubagentRailRow`, `AskUserForm` embedding) gets reused as the
expanded bodies of timeline entries.

---

## P0.1 — context window gauge

Answers "how much headroom does this session have?" with a thin bar at the
top of the rail: green → amber at 70% → red at 90%, tooltip with
`~128k / 200k tokens`.

### wire

Two gaps, both small:

1. `ModelInfo` (verified: `mew-web-client/src/index.ts:199`) has **no context
   window field**. `mew-catalog` has context windows per model (see repo
   CLAUDE.md). Add `context_window?: number` (tokens) to the protocol
   `ModelInfo` and the client mirror, and fill it in the daemon's model-lister
   path from the catalog. Check `crates/mew-protocol` for the ModelInfo wire
   struct and wherever `model_list` is built.
2. Context consumption = prompt tokens of the *most recent* request ≈
   `input + cache_read` from the last `MessageEnd`. Check
   `provider_event_to_wire` in `crates/mew-protocol` — the UI store currently
   only reads `ev.usage.input` / `ev.usage.output` from `message_end`. If
   cache_read/cache_write aren't on the wire shape, add them (the daemon-side
   `ProviderEvent::MessageEnd` already carries them; `translate_event` in
   `crates/mew-daemon/src/lib.rs` uses them for usage aggregation).

### store + ui

- New store field `lastContextTokens: number | null`, set on every
  `message_end` to `usage.input + usage.cache_read`; cleared in `reset()`.
- Look up the active model's `context_window` from `availableModels` by
  `currentProvider/currentModel`. No window known → hide the gauge entirely
  (don't guess).
- Compaction needs no special handling: the next `message_end` after a
  compaction reflects the smaller context, so the bar drops on its own.
- Component: `ContextGauge`, a 3-4px bar with a tooltip. Also render it in
  `status-footer.tsx` as a small percentage text if trivial — decide in
  review, not upfront.

### tests

Vitest: message_end updates `lastContextTokens`; gauge hidden when the model
has no `context_window`; thresholds pick the right color class.

---

## P0.2 — cross-session alert banner

Makes `SessionAlert` discoverable without OS notifications. A slim banner at
the top of the rail (or chat surface, per the layout decision):
`⚠ fix-auth-flow needs approval (bash)` — click switches session, ✕ dismisses.

### plumbing

Entirely client-side; the store's `alerts` array already exists and the fixes
plan adds `clearAlertsForSession` + title sync. Additions:

- Store: `dismissAlert(sessionId, timestamp)` (remove one entry; re-run the
  title-badge sync from fixes 1.6).
- Component `AlertBanner`: shows the most recent alert; if more exist,
  a "+N more" affordance expands the list in place. Kind → icon/severity:
  `permission_needed` / `input_needed` amber (these are the actionable ones),
  `turn_failed` red, `turn_complete` neutral.
- Click: `localStorage.setItem(SESSION_ID_KEY, …)` + navigate via the
  router-ref from fixes 1.5. Navigation already clears that session's alerts
  via `useSessionAttach` → `clearAlertsForSession`.
- Session names: alerts carry `title` (session title after fixes 3.3). Prefer
  `sessionTitles.get(sessionId)` when available, falling back to the wire
  title.

### tests

Vitest: banner renders newest alert; dismiss removes exactly one; navigate
path called with the right session id.

---

## P1 — activity timeline (single feed, replaces tabs; subagents inline)

The core of the redesign: one chronological feed of what the agent is doing —

```
12:34  ⚙ Read session.rs
12:35  ✎ Edit lib.rs  (+5 −2)
12:36  $ cargo test              3.2s ✓
12:37  ▸ Curie (researcher)      running
         ↳ scanning the repo
12:38  ☐ todo done: wire the store
12:39  ? waiting: which approach?   [answer inline]
```

Entries are clickable → the chat surface scrolls to that tool call. Filter
chips (All / Tools / Todos / Subagents / Questions) replace the tab bar.
Subagents render inline as expandable entries whose body is the child's
progress feed (natalie's idea 5 — this is not a separate phase, it's how
subagent entries work).

### data sources — almost everything already flows

All client-side today: `part_start` (tool_call carries `call_id`,
`tool_name`, `state.input`), `part_updated` (running/completed/error +
`state.time` for duration), `subagent-start/status/end`, `todos-updated`,
`ask-user-request`, `permission-request`, and `addUserMessage` for turn
boundaries.

One wire addition for the `(+5 −2)` labels: the agent emits
`AgentEvent::FileDelta { path, added, removed }` per edit
(`crates/mew-agent/src/tools.rs:686`), but the daemon collapses it into the
aggregate `SessionStatsChanged`. In `translate_event`'s `FileDelta` arm, emit
an additional session-local `ServerMessage::FileDelta { session_id, path,
added, removed }` alongside the aggregate. Mirror in the client + store; the
timeline attaches the delta to the most recent edit/write entry matching
`path`. If no entry matches, drop it silently.

### store shape

```ts
interface TimelineEntry {
  id: string;                 // call_id for tools, synthetic otherwise
  at: number;                 // ms epoch
  kind: "tool" | "todo" | "subagent" | "question" | "prompt";
  label: string;              // derived, see below
  status?: "running" | "done" | "error" | "pending";
  durationMs?: number;
  delta?: { added: number; removed: number };   // from FileDelta
  callId?: string;            // for click-to-jump
  parentCallId?: string;      // subagent children only
  children?: TimelineEntry[]; // subagent progress sub-feed
}
```

- `timeline: TimelineEntry[]` appended by the *existing* store handlers
  (`onProviderEvent` tool_call branch, `onPartUpdated`, `onSubagentStatus`,
  `onTodosUpdated` diffing old vs new statuses, `onAskUserRequest`,
  `addUserMessage`). Do not add new bridge listeners; extend the actions.
- Cap at ~500 entries (drop oldest), clear in `reset()`.
- Label derivation, keep dumb: read/edit/write/glob/grep → basename of
  `input.path ?? input.file_path ?? input.pattern`; bash → first ~60 chars of
  `input.command`; everything else → tool name. A `labelForTool(name, input)`
  helper with unit tests.
- Todos: emit an entry only on status *transitions* (pending→in_progress,
  →done, →blocked) and on first plan appearance ("plan: N items"), not on
  every `todos-updated` payload.
- Questions: entry stays `pending` with the `AskUserForm` embedded in its
  expanded body (reuse the existing form + responder flow); flips to `done`
  on resolve.

### history rebuild on attach

`SessionHistory` messages contain tool_call parts with `state.time`
(start/end) — rebuild tool entries from them in `onSessionHistory` so a
reload isn't an empty feed. Todo/subagent/question history isn't persisted as
events; accept the gap (feed shows tools only for past turns). Don't try to
reconstruct it.

### click-to-jump

`VirtualChatSurface` needs an imperative `scrollToCallId(callId)`:
find the message index containing the tool-call part with that `callId`,
scroll the virtual list to it, and briefly highlight the card (a
`data-call-id` attr + a transient class is enough). Expose via a module-level
ref (same pattern as `client-ref.ts`) or a store field `scrollTarget` that the
surface consumes and clears — implementer's choice, note it in the PR.

### component

`ActivityFeed` replaces the three tab panels inside the rail. Filter chips
are local `useState`. Reuse `todoStatusMeta`, the subagent dot colors, and
`AskUserForm` from the current file. Keep `TodoRailPanel` rendering the *full
current plan* available behind the "Todos" filter chip (the feed shows
transitions; the chip view shows the plan snapshot) — cheapest way to not
lose the existing at-a-glance plan view.

### tests

Vitest: `labelForTool` table test; todo diffing emits transitions only;
subagent status appends to children; cap eviction; history rebuild produces
entries with timestamps from `state.time`.

---

## P2 — changes panel

"Review before accepting": a Changes tab listing every file the agent touched
this session, expandable to line-level diffs, plus a "Review changes" button
that pre-fills a review prompt.

### approach: drive diffs from git, not from stored edits

We do not store per-edit diffs anywhere, and re-deriving them from tool
outputs is unreliable. Git already has the truth, and the daemon already has
a scoped file service (`crates/mew-daemon/src/files.rs`) with exactly the
right shape (session cwd resolution + `git status` shelling). Extend it:

- `ClientMessage::GitDiff { session_id, path?: string }`
  - without `path`: run `git diff --numstat -z` (plus `git status` for
    untracked) → `ServerMessage::GitDiffStats { files: [{ path, added,
    removed, status }] }`
  - with `path`: run `git diff -- <path>` (and for untracked files,
    `git diff --no-index /dev/null <path>`) → `ServerMessage::GitDiffResult
    { path, diff: string, truncated: bool }` — cap the diff at ~256KB.
  - both scoped through `session_cwd` like the existing handlers; `path` goes
    through `resolve_scoped`.
- Mirror in `mew-web-client` + store (`diffStats`, `fileDiffs: Map<string,
  string>`), clear both in `reset()`.

Known approximation, surface it honestly in the UI: git shows *all*
working-tree changes, not only the agent's. Default the list to the
intersection with `meta.change_stats.files` (already on `SessionInfo` as
`change_stats`), with a "show all working-tree changes" toggle.

### ui

- `ChangesPanel` under the Changes tab: rows of `status-icon path +A −R`,
  sorted by total churn. Expand → fetch diff on demand → render.
- Diff rendering v1: split the unified diff into lines, color `+`/`-`/`@@`
  with theme tokens (green/red/muted). No syntax highlighting inside diffs
  yet — shiki integration is a follow-up, don't block on it.
- "Review changes" button: composes a prompt listing the changed files
  ("review the uncommitted changes in: …, focusing on correctness") and sends
  it via the existing `client.prompt()`. The richer attach-files flow is in
  `notes/mew-ade-integration-plan.md` — don't build that here, just leave the
  prompt text in one exported constant so the ADE work can replace it.

### tests

- Rust: `parse` of `--numstat -z` output (mixed renames + untracked);
  scoping rejects paths outside cwd (reuse the `resolve_scoped` tests
  pattern); truncation flag on a large diff.
- Vitest: expand fetches once and caches; agent-touched filter vs show-all.

---

## sequencing + effort

| phase | scope | new wire messages | est. size |
|-------|-------|-------------------|-----------|
| P0.1 gauge | store + 1 component | `ModelInfo.context_window`, maybe cache fields on `message_end` | S |
| P0.2 banner | store + 1 component | none | S |
| P1 timeline | store rework + feed component + scroll-to | `FileDelta` (session-local) | L |
| P2 changes | files.rs + protocol + panel | `GitDiff` / `GitDiffStats` / `GitDiffResult` | M |

P0.1 and P0.2 are independent of each other and of P1/P2 — good first PRs.
P1 and P2 are independent of each other. The docked-rail layout change should
land with (or before) P1, since the timeline is the payoff for making the
rail persistent.

## open decisions for natalie

1. **Dock the rail on ≥xl screens?** (recommended yes; see layout section.)
2. Banner placement if the rail stays a sheet: top of chat surface?
3. Does the fixes-plan bell dropdown (1.6) survive once the banner exists, or
   does the banner + title badge cover it? (recommendation: keep the bell —
   it's the only place to see alert *history*; the banner only shows current.)
4. Changes panel default filter: agent-touched files only (recommended) or
   all working-tree changes?

## out of scope

- Persisting timeline events to the session JSONL (would give full history
  rebuild for todos/subagents; big protocol/storage change).
- Syntax-highlighted or side-by-side diffs.
- Accept/revert-per-file actions in the changes panel (needs a write-path
  story and probably jj — see `notes/mew-jj-integration-spec.md`).
- Any left-rail work ("needs you" ordering lives in the fixes plan, phase 6).
