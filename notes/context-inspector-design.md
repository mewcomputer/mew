# Context Window Inspector — Design Doc

Status: draft · Scope: mew (agent core + web UI + mobile) · Last updated: 2026-07-11

## Motivation

An agent's reply is a function of everything in its context window, but the transcript only shows the conversation. When a turn is expensive, wrong, or mysterious, the questions are always the same: what did the model actually see, how big was each piece, and what hit the cache? The inspector answers this per-turn with a collapsible breakdown of the assembled prompt — static scaffolding, tool schemas, context files, persona text, and history — each with a token count and share of the window, plus top-line telemetry (cache warmth, input/output tokens, utilization against the model's context window, remaining compaction headroom).

Beyond debugging, this is a trust and cost tool. The first time a user sees "api reference: 23.3%" they start asking whether that segment should be lazily loaded. Surfacing compaction boundaries also makes the otherwise-invisible "the agent forgot something" failure mode legible.

## Core principle: the transcript is not the request

mew's `Vec<Message>` is the content store. The assembled prompt is a different object: it includes blocks that never appear in the message model (system prompt scaffold, tool schemas, templated context files, persona body text, skills) and it excludes or transforms things that do (compacted history, truncated tool results, provider-stripped reasoning blocks). Deriving the inspector from stored messages alone would lie about what was in context.

Therefore the inspector renders **manifests captured at prompt assembly**, not reconstructions from the transcript. mew rebuilds the system prompt from scratch every turn through a single choke point, so there is exactly one place that knows the complete, serialized, provider-shaped request. That is where segments are emitted. This also resolves the `raw_input` question: `ToolCallPart.raw_input` stays `#[serde(skip)]` because the manifest captures the serialized form's cost at the moment it mattered; the part model never needs to carry it.

Token counts consequently do not live on `Message` or `Part`. A part's cost depends on the tokenizer, the provider's wire serialization, and which request it appeared in (the same part is re-sent every turn and may vanish after compaction). Counts are per-request facts and belong on the per-request snapshot.

## Data model

```rust
pub struct TurnManifest {
    pub turn_id: Ulid,
    pub model: String,                // from Agent.model_id (NOT Request.model, which is empty)
    pub context_window: u32,          // from Agent.context_window (narrowed from catalog i64)
    pub input_tokens: Option<u32>,    // from API usage; None before response/errored
    pub output_tokens: Option<u32>,
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>, // from Tokens.reasoning
    pub segments: Vec<Segment>,
}

pub struct Segment {
    pub label: String,                // "scaffold", "tools", "history", ...
    pub kind: SegmentKind,
    pub source_id: Option<Ulid>,      // message/part ULID for hydration; None for static
    pub tokens: u32,                  // local tokenizer estimate
    pub tokens_scaled: u32,           // scaled so siblings sum to input_tokens
    pub children: Vec<Segment>,       // history → messages → parts
}

pub enum SegmentKind {
    Scaffold,        // system prompt boilerplate
    Persona,         // per-persona body text
    ContextFile,     // CLAUDE.md / AGENTS.md, per-cwd/git-root
    Skill,           // disk-loaded skill injected as tool description
    Tools,           // tool schemas, pre-SDK-transform (known gap)
    Ephemeral,       // current time, environment lines
    Message,
    Part,
    CompactionSummary,
    // Note: no Completion kind — output tokens are tracked on the manifest
    // top-level (output_tokens), not as a segment. The stacked bar shows
    // input context only; output is shown in the summary line.
}
```

Design notes:

`SegmentKind` is deliberately finer-grained than a single `Static` bucket. Scaffold, context files, personas, and skills have different cost profiles and different remediation stories ("trim the persona" vs "lazy-load the API reference"), and those are exactly the actions a user takes after seeing the breakdown. Lumping them defeats the tool's purpose.

`source_id` is the one field kept from the earlier, heavier `SegmentSource` design. Hydrating expandable row bodies by label-matching works until two segments share a label or compaction reorders history; message and part ULIDs already exist, so carrying an `Option<Ulid>` is cheap insurance against a heuristic. Static segments leave it `None` and embed nothing — their content is reproducible from config, and the UI can show the serialized text captured at assembly if we later decide to store it (see Open Questions).

`input_tokens` and `output_tokens` are `Option<u32>`, not `u32`. Before the API response arrives (or on errored turns), these are `None`. The manifest is captured at assembly time (before dispatch) so the segment structure is always available; usage is backfilled from the API response after `MessageEnd`. If the turn errors (context overflow, network failure), usage stays `None` and the summary line shows "error · structure below" — the segment tree is still viewable, which is exactly when you need it most.

There is no per-segment `measured: bool`. Neither Anthropic nor OpenAI reports true per-segment token counts — Anthropic reports cache_read/cache_write totals (not per-breakpoint), OpenAI reports a single `cached_tokens`. What we can honestly claim is "apportioned, with cache-derived priors" (below), and the cache totals live once on the manifest rather than as a misleading per-row flag. The UI still communicates estimate-ness (see UI spec).

**Relationship to `Tokens`:** `AssistantMeta` already carries `Tokens { input, output, reasoning, cache_read, cache_write }` (all `u32`, non-optional). The manifest's `input_tokens: Option<u32>` etc. are `None` before the response arrives (or on errored turns) and `Some` after backfill. On successful turns, `manifest.input_tokens == Some(meta.tokens.input)` — the manifest doesn't duplicate the data, it wraps it in `Option` to represent the pre-response/errored state. The UI reads from `manifest.input_tokens` (which handles the `None` case) rather than `meta.tokens.input` (which is `0` on errored turns).

### Persistence

The manifest embeds directly in `AssistantMeta`:

```rust
pub struct AssistantMeta {
    pub provider_id: String,
    pub model_id: String,
    pub cost: f64,
    pub tokens: Tokens,
    pub finish: Option<Finish>,
    pub error: Option<MessageError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<TurnManifest>,   // new
}
```

Old sessions deserialize unchanged (the field is `Option` and absent). The manifest travels with the message in the session JSONL — no sidecar files, no separate query path — and the UI already loads `AssistantMeta` to render usage, so showing the inspector is `if let Some(manifest)`. Manifests are metadata, not content (roughly 1–3 KB/turn post-compaction; see Manifest size below), so per-turn accumulation in the JSONL is acceptable; if session files grow problematic, splitting to a sidecar keyed by the assistant message ULID is a mechanical change later.

### Manifest size

The manifest only contains segments for what's in the active context window. After compaction, old messages are replaced by a single `CompactionSummary` segment — so the manifest size tracks the *post-compaction* context size, not the total session length. A 200-turn session compacted to 30 active messages has a manifest roughly the size of those 30 messages, not all 200. Pre-compaction turns in a long session will have larger manifests, but those turns will eventually be compacted — the bloat is temporary.

### Wire protocol

The manifest travels inside `AssistantMeta`, which is already in the `Message` struct serialized over the wire via `ServerMessage::SessionHistory`. No new protocol message is needed — the manifest is part of the existing message payload. The daemon sends full manifests per turn (no diffing); for v1 the per-turn size (1-3 KB post-compaction) is acceptable on the wire. A future optimization could send a base manifest once and diffs for subsequent turns, since most static segments (scaffold, tools, context files) don't change between consecutive turns. The diff path would need to invalidate cached segments when `cache_read_tokens` drops (indicating the prefix changed).

## Instrumentation

### Capture point

The single best capture point is **`crates/mew-agent/src/turn.rs:297` — after `Request` assembly, before `provider.stream(req)`**. At this point, the `Request` struct holds:

- `system: String` — final system prompt (scaffold + persona + context files + skills, after dispatcher's `on_system_prompt` hook)
- `messages: Vec<Message>` — post-compaction, post-`on_chat_message` hook, post-empty-part-stripping history
- `tools: Vec<ToolDef>` — filtered tool schemas (pre-SDK-transform — the known gap)
- `params` and `headers` — sampling params and HTTP headers

All data is still structured. This is the last point before the provider flattens it into an HTTP request.

**Important:** `Request.model` is an empty string at this point (`String::new()` at turn.rs:281). The manifest's `model` and `context_window` must come from the Agent struct (`self.model_id`, `self.context_window`), NOT from the Request. The build function signature is `build_manifest(&req: &Request, model_id: &str, context_window: u32)`.

**AssistantMeta temporal gap:** `AssistantMeta` is created at the first `PartStart` event (in `start_assistant_message`, events.rs:191-210), which fires AFTER the stream has begun — not at the capture point. The manifest must be stored on the Agent (e.g., `self.pending_manifest: Option<TurnManifest>`) before the stream loop begins, then consumed when `start_assistant_message` creates `AssistantMeta`. After consumption, set it back to `None`.

### System prompt segmentation

At capture time, the system prompt is a single concatenated string. To get per-segment counts, the assembly path at `turn.rs:233-237` needs to carry segment metadata alongside the string:

```rust
// Current (turn.rs:233-237):
let base_system = match &self.persona_prompt {
    Some(persona) => format!("{}\n\n{}", persona, self.system),
    None => self.system.clone(),
};
let system = self.dispatcher.on_system_prompt(base_system).await;

// Instrumented: carry segments alongside the string
let system_segments = build_system_segments(
    &self.persona_prompt,    // Persona segment (if active)
    &self.system,            // Scaffold + ContextFile + Skill segments
);
let system = self.dispatcher.on_system_prompt(system_segments.concat()).await;
// After the hook, the system string may have changed —
// re-derive segment boundaries by length if the hook modified content.
```

For v1, the segments within `self.system` (context files, skills, scaffold) can be split by their known XML tags (`<context source="...">`, `<available_skills>`, etc.) at capture time, avoiding the need to thread a new type through the entire assembly path. This is slightly fragile (breaks if tags change) but avoids a larger refactor. A v2 cleanup would thread `Vec<(String, SegmentKind)>` through assembly properly.

### Known gap: tool schema transformation

The `Vec<ToolDef>` at the capture point is pre-SDK-transform. The provider SDK reformats tool schemas before sending (OpenAI wraps in function-calling format, Anthropic uses a different shape). The manifest counts the pre-transform form. This is the single largest correctness risk — if the base estimates are wrong for tool schemas, the biggest static segments are the most wrong, which is the worst failure mode for a UI that looks precise. The apportionment scaling partially corrects this (the total is always correct), but the per-segment split is approximate.

v1 accepts this gap. v2 could capture at the provider→wire boundary if the SDK exposes the serialized request body.

### Compaction interaction

The compaction code at `turn.rs:139-164` currently creates a synthetic `TextPart` with "Previous conversation has been compacted..." — NOT a `CompactionPart` with `tail_start_id`. The `CompactionPart` type exists in `mew-message` but is not populated by the current compaction logic.

For the manifest, this means:
- The manifest's history subtree only contains messages that are in the active context window (post-compaction)
- The compaction summary appears as a synthetic `TextPart` within a `Message` — but `synthetic: true` is used by THREE different code paths (compaction summary, flagged file re-injection, truncation acknowledgement), so the manifest must disambiguate. Detection heuristic for v1: `synthetic: true` AND message is the first in history (index 0) AND role is User — compaction summaries are always inserted at the front. Flagged file re-injections are also at index 0 but have "Flagged file" in the text; truncation acks are assistant-role. The most robust v1 approach: match on the exact compaction text prefix "Previous conversation has been compacted" (brittle but unambiguous).
- A future fix should make compaction emit `CompactionPart` properly, which would give the manifest a clean `CompactionSummary` segment kind and `tail_start_id` for the boundary marker
- Until then, the manifest treats the compaction summary message as a `CompactionSummary` segment (detected via the text prefix)

### Tool result timing

A tool result produced during turn N is added to the message history for turn N+1's context. The manifest for turn N does NOT include this tool result (it was produced after the prompt was sent). The manifest for turn N+1 includes it as a history segment. This is correct behavior — each manifest is a snapshot of what was in context *for that turn* — but the inspector for "why did the model say X?" doesn't show the tool result that *caused* X if the tool ran during that same turn. The tool result appears in the transcript (as a `ToolCallPart` with `ToolState::Completed`), just not in the manifest's context breakdown.

### Subagent manifests

Subagent manifest inclusion is a v2 feature. In v1, subagent results appear in the parent's history as regular messages with no special segment kind. The manifest builder does not need to identify subagent results in v1. When v2 lands: each subagent turn gets its own manifest (the subagent has its own context, system prompt, tool set), the parent turn's manifest includes a `Segment { kind: Message, label: "subagent: {name}" }` representing the subagent's total token cost, and the web UI subagent panel shows the child's manifest when expanded. This requires a mechanism to thread the manifest from the child agent to the parent — not trivial since `SubagentRunner` is a separate execution context.

## Token accounting

Per segment, run a local tokenizer over the wire-form serialization: tiktoken for OpenAI (exact for the model-appropriate encoding), tiktoken as an approximation for Anthropic and others (clearly labeled with `~` in the UI). The apportionment scaling (below) corrects systematic drift — even if tiktoken is off by 10% on a given segment, segments that are relatively bigger still get proportionally more tokens. The *ratios* are honest even when absolute numbers are approximate, and ratios are what drive the "oh, tools are 23% of my context" insight.

After the response arrives, scale: `tokens_scaled = tokens × (usage.input_tokens / Σ tokens)`. Siblings then sum to the true total and percentages are honest in aggregate even when individual estimates drift.

Cache totals improve the priors without pretending to be measurements. With stable cache breakpoints across turns, `cache_read_tokens` tells you the size of the warm prefix; if segment order is stable (it should be — cache efficiency already demands prompt-prefix stability), the warm prefix boundary falls at a known segment boundary and you can scale the prefix and suffix groups against their respective sub-totals separately. This tightens the estimates for exactly the segments users care most about (the big static ones) while remaining, honestly, apportionment.

Cache warmth for the top line is `cache_read_tokens / input_tokens` (Anthropic) or `cached_tokens / prompt_tokens` (OpenAI).

### Errored turns

The manifest is captured before dispatch (at `turn.rs:297`). If the turn errors (context overflow, network failure), `input_tokens` and `output_tokens` stay `None`. The summary line shows "error · structure below" and the segment tree is still viewable. This is exactly the case where the inspector is most useful — a context overflow error means the user needs to see what's occupying the window.

## UI

Two targets: React + shadcn (web) and SwiftUI (mobile). Same data, same information hierarchy; the manifest is already JSON in the JSONL, so the web client consumes it directly and mobile-core exposes it alongside the usage it already tracks.

### Shared spec

The inspector renders as a collapsed one-line summary under each assistant message, expanding to the full breakdown:

1. **Summary line** — `context 99% warm · 9.7k ↓ · 238 ↑ · 41% (9.7k/24.0k)`: cache warmth, input, output, utilization. All fields come straight off the manifest. There's a 'expand' button next to it, along with other actions (paste, fork, retry (only on latest message))
2. **Stacked bar** — one segment per top-level manifest segment, proportional widths from `tokens_scaled`, plus a visually distinct tail for free space. Color per `SegmentKind`, consistent across turns so users learn the palette.
3. **Tree** — one row per segment: disclosure triangle (if children or hydratable content), label, estimate badge, `tokens_scaled`, percentage. History expands to messages, messages to parts. Tool-call rows render the invocation compactly (monospace); expanding a row with a `source_id` hydrates the actual content from the message/part store.
4. **Free space row** — remaining budget before the compaction threshold, labeled as such.
5. **Estimate signaling** — counts are estimates and must read as estimates. A small `~` prefix or an "apportioned" badge per row, with a tooltip/footnote explaining that per-segment numbers are local-tokenizer estimates scaled to the provider-reported total. Users trust numbers that look precise; the UI's job is to make the precision claim accurate.
6. **Compaction boundary** — when a `CompactionSummary` segment exists, render a visible divider in the history subtree: summary above, live tail below.

### React + shadcn

`Collapsible` for the outer container and per-row disclosure; `Tooltip` for the estimate explanation; `Badge` for kind/estimate tags; the stacked bar is a flex row of divs (no chart library needed — it's a single stacked bar, and Recharts would fight the compact inline layout). Virtualize the history subtree (`@tanstack/react-virtual`) since long sessions can have hundreds of part rows. Percentages right-aligned in a monospace column so the tree scans like the screenshot. Hydration on expand: `source_id → message/part lookup` in the already-loaded session store; no extra fetch for the common case.

### SwiftUI

`DisclosureGroup` inside a `LazyVStack` (not `List` — the inspector nests inside a chat bubble and `List` styling fights that); the stacked bar is an `HStack` of `Rectangle`s in a `GeometryReader`, or a `Canvas` if segment counts get large. `.monospacedDigit()` on counts. Mobile constraint: default the inspector to summary-line-only and open the full tree in a sheet — the tree wants horizontal room that a phone bubble doesn't have.

## Build order

1. **Rust types** — `TurnManifest`, `Segment`, `SegmentKind` in `mew-message`. Add `manifest: Option<TurnManifest>` to `AssistantMeta`.
2. **Manifest builder** — `build_manifest(&Request, model_id, context_window)` in `mew-agent`. Segment the system prompt by XML tags, map messages/parts to segment tree. Token counts are 0 (not the char/4 estimate — wrong numbers that persist in JSONL are worse than zeros).
3. **Capture + threading** — Call `build_manifest` at `turn.rs:297`, store on `self.pending_manifest`. After `MessageEnd` with usage, backfill `input_tokens`/`output_tokens`/`cache_*`/`reasoning_tokens` and scale `tokens_scaled`. Inject into `AssistantMeta` when `start_assistant_message` creates it.
4. **tiktoken integration** — Add tiktoken dependency, select encoding by model, count per segment. Replace the zero counts from step 2 with real estimates. Mark Anthropic-approximate counts with `~` in the UI.
5. **Web UI** — Summary line + stacked bar first (cheap, immediately useful), tree second, hydration third.
6. **Cache-derived prior refinement** — Prefix/suffix split scaling using `cache_read_tokens`.
7. **SwiftUI client (v2)** — Mobile initially shows only the summary line (input/output/cache/utilization), which is already tracked per-session via `SessionUsage`. Full segment tree on mobile requires adding manifest types to `mew-mobile-core` (new UniFFI records, `ChatMessage` extension, translation in `translate_message`). This is a separate batch with its own `just ios-core` regen.
8. **Calibration harness** — Periodically compare local counts against `count_tokens` / known-exact requests, track drift per provider.
9. **Subagent manifests (v2)** — Each subagent turn gets its own manifest; parent shows total cost. Requires a mechanism to thread manifest from child to parent. Deferred from v1 — subagent results appear in parent history as regular messages.

Steps 1-3 ship the infrastructure with structure but no real counts and no UI. Step 4 makes counts real. Steps 5-6 ship the web UI. This avoids shipping confidently-wrong numbers before the tokenizer is working — steps 2-3 persist manifests with zeroed token counts, which degrade gracefully (the UI shows segment structure without numbers).

## Risks and hard parts

**Serialization mismatch** is the time sink: the gap between your serialized form and the provider's wire form (tool schema transforms, system prompt placement, image encoding). v1 captures at the agent→provider boundary (post-assembly, pre-SDK-transform). The known gap is tool schema transformation. v2 captures at the provider→wire boundary if the SDK exposes it.

**Apportionment error looks like precision.** If the local tokenizer is systematically off for one segment type, its percentage is confidently wrong. Mitigation: scaling bounds aggregate error; estimate badges (`~`) bound trust; the calibration harness (step 8) bounds drift. For OpenAI providers, tiktoken is exact — no `~` needed. For Anthropic/others, tiktoken is approximate — the `~` communicates this.

**Provider divergence in cache reporting** limits how much the cache priors help outside Anthropic. Anthropic gives cache_read/cache_write totals (not per-breakpoint); OpenAI gives a single `cached_tokens`. Acceptable: the feature degrades to pure apportionment, which is still useful.

**Manifest staleness on model switch**: a manifest describes the request that produced that turn; if the user switches models mid-session, old manifests correctly reflect old context windows. The UI should render utilization against `manifest.context_window`, never the session's current model.

**CompactionPart not populated**: the current compaction code creates synthetic `TextPart` instead of `CompactionPart`. The manifest detects compaction via `synthetic: true` for now. A future fix should make compaction emit `CompactionPart` properly.

## Open questions

- Should static segments store their serialized text in the manifest (reproducibility, +KBs/turn) or stay count-only (current design)? Leaning count-only until a debugging need proves otherwise.
- Redaction: manifests in JSONL mean context-file names and persona labels persist with the session. Fine for a local tool; revisit if sessions ever sync.
- Wire protocol optimization: v1 sends full manifests per turn. v2 could send a base manifest once and diffs for subsequent turns (most static segments don't change between turns). The diff path needs to handle cache invalidation when `cache_read_tokens` drops.
