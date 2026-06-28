# mew-web-ui design direction

We want a UI identity that is independent of the shadcn themes you intend to import. The look can stay shadcn/neutral; the differentiator is **information architecture + signature components + motion conventions**. This doc lists the direction and concrete examples of each differentiator. The goal is not to redesign the visual system but to give mew a recognizable shape.

## What we have right now

- **Stack:** Vite + React 19 + Tailwind 3 + shadcn-style tokens. `light | dark | system` theme toggle persisted to `localStorage`.
- **Layout:** top bar + single main column (chat → todo → subagent → ask-user → input). Sessions are in a left slide-over drawer (`SessionListDrawer`).
- **TopBar:** connection dot + state, session id button, model picker, token counts, cost, theme toggle.
- **ChatSurface:** centered max-w-3xl feed, auto-scroll, empty state.
- **MessageItem:** user bubble right (primary fill), assistant left (card border). Tool calls are grouped and collapsed; reasoning is a `<details>` block; copy on hover for assistant text.
- **ToolCallCard:** header shows tool name, input summary, state icon + label. Expand for full JSON. States: pending / running / completed / error.
- **InputArea:** bottom textarea, cmd/ctrl+enter send, auto-resize, send/stop button. No attachment, no slash commands, no model/persona pills.
- **ModelPicker:** top-bar dropdown grouped by provider, searchable.
- **SessionListDrawer:** slide-over with New session, state badge, model, client count, created date. No preview text.
- **PermissionToast:** floating bottom-right card with allow-once / allow-session / deny.
- **SubagentPanel:** horizontal strip under chat showing running/completed subagents with progress.
- **AskUserCard / TodoPanel:** render as horizontal strips above the input.

## Design differentiators (no warmth required)

### 1. Persistent left rail
Replace the session drawer with a narrow, collapsible left rail. This moves mew from “chat app” toward “workspace.”

**Examples:**
- Header section: workspace path/project label + New session button.
- “Continue latest” hero button when there is a previous non-empty session.
- Session rows show: generated title (or first user prompt), one-line preview of last message, model, token count, age, state badge (active/idle).
- Active session highlighted with a vertical accent bar.

```tsx
// Example row shape
<SessionRow
  title="pi-web UI review"
  preview="audit muted-text contrast first since it's a 10-minute job"
  model="umans-glm-5.2" tokens={3300} age="9m" state="active"
/>
```

### 2. Signature status footer
A bottom status bar is cheap, scales with information, and gives mew a “tool-like” identity.

**Examples:**
- Left side: connection dot + provider/model shortcut. Middle: tokens in / out, cost, active subagent count, pending permissions count. Right: last event latency, current mode/persona.
- Use monospaced figures (tabular-nums) for numbers so they don’t jitter as counts update.
- Clicking model opens the existing `ModelPicker`. Clicking subagent count focuses the subagent panel. Clicking permission count opens the permission toast.

```tsx
// Example footer shape
<StatusFooter
  connection="connected"
  model="umans/umans-kimi-k2.7"
  tokens={{ in: 12_431, out: 3_892 }}
  cost={0.0042}
  subagents={{ running: 2, total: 4 }}
  permissions={1}
/>
```

### 3. Tool-call surface language
Keep the existing card but add stronger state language and sensitivity color coding.

**Examples:**
- Running state: pulsing blue dot + elapsed time.
- Completed: green check with a “copied” flash on click.
- Error: red border + inline error reason.
- Sensitivity badge on each card: `ReadOnly` (subtle), `Mutating` (amber), `Dangerous` (red). This maps directly to mew’s `Sensitivity` enum.
- Hover reveals a “run args diff” view for `edit` / `write` tools.

```tsx
// Example tool-card header
<ToolCallHeader
  tool="write"
  summary="src/main.rs"
  sensitivity="mutating"
  state="completed"
  durationMs={340}
/>
```

### 4. Input composer as command surface
Move model/provider and persona selection closer to the input.

**Examples:**
- Left of textarea: active model pill (click → model picker) and active persona pill (click → persona switcher).
- Right of textarea: attach file, show available slash commands on `/`.
- Placeholder text adapts: “Ask mew anything… · / for commands · @ for personas”.
- Keyboard shortcut hints appear on focus (Cmd+Enter to send, Esc to cancel).

```tsx
// Example composer shape
<InputComposer>
  <ModelPill model="umans/umans-kimi-k2.7" />
  <PersonaPill persona="code-reviewer" />
  <textarea … />
  <AttachButton />
  <SendButton />
</InputComposer>
```

### 5. Reasoning as a native timeline
Replace the plain `<details>` block with a collapsed “Reasoning” chip that expands into a step-oriented stream.

**Examples:**
- Collapsed: small pill “Reasoning · 4 steps”.
- Expanded: vertical timeline of reasoning chunks, each with a timestamp/sequence number. Keeps the reading area calm.
- Later: streaming reasoning updates append to the timeline with a live typing indicator.

```tsx
// Example reasoning component
<ReasoningBlock steps={[
  { index: 1, text: "User wants design differentiators with examples." },
  { index: 2, text: "Persistent left rail fits mew’s workspace metaphor." },
]}/>
```

### 6. Subagents and todos as a right rail or bottom drawer
Right now todo / subagent / ask-user stack under chat and can push the input up. A dedicated right rail (collapsible) keeps the chat stable.

**Examples:**
- Right rail tabs: Todos · Subagents · Questions.
- Active count badges per tab.
- Subagent row shows display name, tool name, progress, runtime duration, outcome.
- Todo row shows dependency graph with connecting lines when expanded.

```tsx
// Example right-rail tab shape
<RightRail activeTab="subagents">
  <SubagentRow name="Curie (researcher)" tool="skill: investigate" progress="scanning the repo" elapsedMs={3400} />
</RightRail>
```

### 7. Empty states with purpose
Current empty state is just text. Make each empty surface feel intentional.

**Examples:**
- Chat empty state: large icon (mew “/” command suggestions, recent sessions quick actions.
- Session rail empty state: “Start a new session” + drag-drop hint for importing sessions.
- Footer empty state: hide footer until connected; then fade in with initial metrics.

## What to avoid

- Do not add a bespoke palette; let shadcn themes handle warmth/coldness.
- Do not invent gradients, backdrop blur, or grain unless there is a functional reason.
- Avoid decoration that competes with code blocks and tool output. The above differentiators borrow from pi-web and Claude Code: Claude for restraint and hierarchy, pi-web for richer metadata and workspace metaphors. Implementations should be initialized.  > >

## Recommended order of implementation

1. Persistent left rail — biggest layout change; gives the UI its shape.
2. Status footer — high value, low risk.
3. Input composer pills — builds on existing `ModelPicker`.
4. Tool-call state + sensitivity — uses existing `toolState` data.
5. Reasoning timeline — UI-only upgrade.
6. Right rail for todos/subagents/questions — larger refactor.

---

*This doc is a snapshot. Update when a differentiator is implemented or rejected.*
