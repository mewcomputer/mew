# Workbench tabs

Status: phase 2 complete; phase 3 in progress

Phase 1 now includes the resizable conversation/workbench shell, unified tab
registry, persisted migration, shared tab strip, browser promotion, and the
initial terminal/file surfaces. Phase 2 carries tab-scoped browser events,
owner-gated native CEF lifecycle calls, packaged-app verification, the
single-instance desktop guard, visible CEF/daemon CDP authority, and
shutdown/descriptor hardening. The remaining work is real PTY/file editing
surfaces.

## Goal

Make the contextual workbench behave like a Codex-style tabbed workspace. The
pinned summary remains an independent surface. The workbench becomes a tab
host where each tab is a concrete workspace item: a browser page, terminal,
file, diff, review, or live activity view.

The main conversation remains the primary surface. Workbench tabs provide
context without replacing the conversation or turning the workbench into a
dashboard of fixed modes.

## Primitive decision

Use the existing shadcn `Tabs` wrapper backed by Radix for the workbench tab
semantics:

- `Tabs` owns the selected tab value and controlled selection changes.
- `TabsList` becomes the horizontal document strip.
- `TabsTrigger` becomes a tab with icon, contextual title, status indicator,
  and an explicit close control.
- `TabsContent` provides the tab-panel relationship and focus semantics.
- The wrapper should gain a document/line presentation, rather than using its
  current pill-style default.

The primitive owns accessible tab behavior. It does not own the workbench
registry, persistence, surface state, native browser lifecycle, or daemon
protocol. Those remain in the workspace layer.

The `+` button is an action, not a tab. It opens a compact surface picker. The
picker creates a tab and focuses it immediately. Keeping the picker outside the
Radix tablist avoids making a non-tab action part of the tab collection.

## Resizable layout decision

Use shadcn's `ResizablePanelGroup`, `ResizablePanel`, and `ResizableHandle`
composition for the conversation/workbench split. Add it through the CLI:

```bash
npx shadcn@latest add resizable
```

The layout should be:

```text
Session rail | ResizablePanelGroup
             ├── conversation panel
             ├── ResizableHandle
             └── workbench panel
```

Recommended behavior:

- default split: roughly 72% conversation / 28% workbench
- workbench minimum: wide enough for tab titles and controls
- workbench maximum: large enough for code and browser work without
  swallowing the conversation
- dragging changes the split; keyboard focus on the handle supports precise
  adjustments
- closing the workbench collapses its panel to zero while remembering the last
  usable width
- reopening restores that width
- persist the split per workspace alongside tab state
- use the current mobile sheet behavior below the desktop breakpoint

The existing `RightRail` should become the workbench panel content rather than
owning its own fixed width. Its CEF viewport `ResizeObserver` remains the final
source of truth for native browser bounds after a drag.

## Information architecture

```text
Workspace
├── pinned summary (independent, attention-first)
└── workbench
    ├── tab strip
    │   ├── Activity (singleton, initially non-closable)
    │   ├── browser page tabs
    │   ├── terminal tabs
    │   ├── file tabs
    │   ├── Changes / Review tabs
    │   └── + surface picker
    └── active tab surface
```

Activity remains a first-class tab during the migration so live todos,
subagents, questions, and jobs do not disappear. The pinned summary continues
to own cross-session attention and orientation. If Activity becomes redundant
after the new summary is exercised, it can later become optional without
changing the tab model.

## Tab model

Replace the fixed `WorkbenchTab` mode enum and nested browser-tab state with a
registry of tab instances:

```ts
type WorkbenchTabKind =
  | "activity"
  | "browser"
  | "terminal"
  | "file"
  | "changes"
  | "review";

interface WorkbenchTab {
  id: string;
  kind: WorkbenchTabKind;
  title: string;
  closable: boolean;
  sessionId?: string;
  cwd?: string;
  payload?: {
    url?: string;
    path?: string;
    jobId?: string;
  };
  status?: "idle" | "running" | "attention" | "error";
}
```

The tab registry owns ordering, active tab, creation, closing, focusing an
existing matching tab, and persistence. Surface components receive a tab and
emit domain actions; they do not create their own competing tab systems.

Tabs should persist per workspace/project. A tab pointing at a file, terminal
cwd, or browser session must not silently appear in an unrelated project.

## Surface behavior

### Browser

- A URL is one top-level workbench tab. Remove BrowserPanel's nested tab strip.
- The tab owns URL, title, loading, error, and snapshot metadata.
- The native CEF host currently exposes one browser instance. Switching browser
  tabs will navigate that shared instance to the selected tab's URL.
- Keep the model compatible with multiple native instances later, but do not
  multiply CEF lifecycle complexity in the first restructuring pass.

### Terminal

The target is a real interactive PTY tab with cwd, streaming output, resize,
interrupt, and close. The current background-job state is useful for Activity
badges but is not a terminal surface.

The first implementation may use a read-only job-output surface while the
daemon protocol gains PTY support, but it must be labeled as job output rather
than presented as a terminal that cannot accept input.

### File

- Opening a file from the tree creates or focuses a path-bound file tab.
- Start with a read-only preview/editor surface using the existing file
  preview path.
- Add dirty state, save, reload, and external-change conflict handling before
  treating it as a full editor.
- Store the workspace root and absolute path with the tab.

### Changes and Review

- Convert the existing fixed surfaces into normal tab instances.
- They should be created by the surface picker or focused from relevant
  actions, rather than always occupying four permanent slots.
- Their badges reflect changed-file count or review attention.

### Activity

- Keep one singleton Activity tab during migration.
- Preserve the existing attention-first ordering inside it.
- Show running/attention counts in the tab without duplicating the full
  summary content in the tab strip.

## Interaction rules

- Clicking an existing matching file, review, or browser target focuses its
  current tab instead of creating duplicates where practical.
- `+` opens the surface picker: Browser, Terminal, File, Changes, Review.
- `Cmd/Ctrl+W` closes the active closable tab.
- `Cmd/Ctrl+T` creates a browser tab when Browser is active; otherwise it opens
  the surface picker.
- `Cmd/Ctrl+1…9` selects a visible tab by position.
- Switching tabs is immediate and preserves each tab's logical state.
- Closing the workbench hides it without closing its tabs.
- Attention is communicated through explicit labels and restrained status
  badges, not unexplained dots.
- The active tab gets the strongest visual treatment; inactive tabs remain
  readable and horizontally scroll when necessary.

## Delivery phases

### Phase 1: tab host and migration

- [x] Add the registry/reducer and persistence format.
- [x] Extend the local shadcn Tabs wrapper with the document/line style.
- [x] Build the shared tab strip and surface picker.
- [x] Lift browser tab state into the registry and remove nested BrowserPanel tabs.
- [x] Migrate Activity, Changes, Review, and Browser onto the shared host.
- [x] Add reducer, persistence, accessibility, and RightRail interaction tests.

### Phase 2: browser lifecycle hardening

- [x] Re-navigate the shared CEF browser when the active browser tab changes.
- [x] Ensure inactive browser tabs do not steal native bounds or visibility.
- [x] Preserve protocol errors per tab and clear loading on every error path.
- [x] Echo browser tab identity through the daemon and TypeScript client.
- [x] Gate native bounds, visibility, navigation, and cleanup by active tab
  ownership, including stale queued callbacks.
- [x] Keep browser surfaces mounted through tab switches while hiding inactive
  panels from the accessibility tree and preventing inactive navigation.
- [x] Verify visible app rendering and agent-browser CDP control remain the same
  user-facing browser session.

### Phase 3: terminal surface

- Specify and implement PTY protocol messages, session ownership, output
  streaming, input, resize, interrupt, and cleanup.
- Add the terminal tab with explicit connection, running, exited, and failed
  states.
- Keep background jobs in Activity and let a job action open/focus its terminal
  tab when an interactive process exists.

### Phase 4: file surface

- Open files from the tree into path-bound tabs.
- Implement preview first, then editing and save/conflict behavior.
- Add dirty/unsaved status to the tab title and close confirmation only when
  data could be lost.

### Phase 5: persistence and polish

- Scope persisted tabs to workspace and session context.
- Migrate old `workbenchTab` state without losing the user's selected surface.
- Add keyboard navigation, overflow behavior, reduced-motion handling, and
  macOS interaction polish.
- Exercise the complete flow in the Tauri app at narrow, normal, and wide
  workbench widths.

## Non-goals for the first pass

- Multiple simultaneous native CEF browser instances.
- A fake terminal that only looks interactive.
- A full IDE/editor implementation before the tab host is stable.
- Replacing the pinned summary with another tab system.
- Adding a new UI dependency when the existing shadcn/Radix primitives cover
  the interaction model.

## Verification gates

- Tab reducer covers creation, focus deduplication, close behavior, ordering,
  active-tab fallback, and persistence migration.
- Tabs expose correct roles, labels, selected state, close affordances, and
  keyboard behavior.
- Browser, file, terminal, changes, review, and activity surfaces can coexist
  without duplicate local tab state.
- Closing/reopening the workbench preserves tabs and active selection.
- The macOS app preserves native browser visibility, CEF navigation, and CDP
  behavior when switching workbench tabs.

Reference: [shadcn Tabs](https://ui.shadcn.com/docs/components/base/tabs)
