# Design: Artifact rendering (B1.5)

## Goal

Let the model present rich content (HTML, SVG, Mermaid diagrams, formatted markdown) in the chat UI via a tool call. Like Claude Code's "Artifacts" but tool-based: the model calls `present_artifact`, the content renders inline in a sandboxed panel. Web-only — other frontends (TUI) see the tool result as text.

## Why tool-based (not a new Part variant)

Adding a `Part::Artifact` would touch the message model, provider adapters, session persistence, compaction logic, and every `match` on `Part`. Using a **tool call** piggybacks on existing infrastructure:

- The tool returns `ToolOutput { text: "...", metadata: Some(json!({...})) }`.
- `ToolStateCompleted.metadata` already exists on the wire (`mew-message/src/lib.rs:230`) and is `skip_serializing_if = "Option::is_none"`.
- The `ToolCallCard` component (already in the web UI) detects `metadata.artifact` and renders a rich panel instead of the plain text output.
- Session persistence works for free — the tool result is just a `ToolResultPart` with metadata, which the JSONL already stores.

No new `Part` variant, no protocol changes, no provider changes. The only additions are the tool itself and the React rendering component.

## The tool

```rust
pub struct PresentArtifact;

impl Tool for PresentArtifact {
    fn name(&self) -> &str { "present_artifact" }
    fn description(&self) -> &str {
        "Present rich content (HTML, SVG, Mermaid, or markdown) as a visual \
         artifact in the chat UI. Use this when the user would benefit from \
         seeing rendered content rather than raw text — e.g. diagrams, \
         interactive HTML, structured data visualizations.\n\n\
         Only available in web/UI contexts that support rendering. If the \
         frontend cannot render artifacts, the content is shown as plain text."
    }
    fn sensitivity(&self) -> ToolSensitivity { ToolSensitivity::ReadOnly }
    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "artifact_type": {
                    "type": "string",
                    "enum": ["html", "svg", "mermaid", "markdown"],
                    "description": "The type of content to render."
                },
                "title": {
                    "type": "string",
                    "description": "A short title for the artifact panel."
                },
                "content": {
                    "type": "string",
                    "description": "The content to render. For HTML, this is \
                     a full HTML document fragment. For SVG, the raw <svg> \
                     markup. For Mermaid, the diagram source code. For \
                     markdown, the markdown source."
                }
            },
            "required": ["artifact_type", "title", "content"]
        })
    }
}
```

**Execute** returns:
```rust
Ok(ToolOutput {
    text: format!("Artifact '{}' ({}) rendered", title, artifact_type),
    error: None,
    metadata: Some(json!({
        "artifact": {
            "type": artifact_type,
            "title": title,
            "content": content,
        }
    })),
})
```

The `text` field is what the model sees in conversation history (so it knows the artifact was shown). The `metadata.artifact` field is what the web UI reads to render the rich panel.

## Wire flow

```
Model calls present_artifact(html, "Login form", "<form>...")
  ↓
Tool::execute → ToolOutput { text: "Artifact 'Login form' (html) rendered",
                              metadata: { artifact: { type, title, content } } }
  ↓
Agent: ToolStateCompleted { output: text, metadata: Some({artifact:...}), ... }
  ↓
PartUpdated(ToolCallPart { state: Completed, ... })
  ↓
ServerMessage::PartUpdated { part: ToolCall { state: { status: "completed",
                                                         output: "...",
                                                         metadata: {artifact:...} } } }
  ↓
TS client: tool_call part with state.metadata.artifact
  ↓
React: ToolCallCard checks part.state.metadata?.artifact → renders ArtifactPanel
```

**No protocol changes.** The `metadata` field is already on `ToolStateCompleted` in Rust and serializes to the wire. The only TS change is adding `metadata?` to the `completed` ToolState type so the UI can read it.

## Compatibility matrix

| Frontend | What happens | How |
|----------|-------------|-----|
| **Web UI** (React) | Rich panel rendered | `ToolCallCard` → `ArtifactPanel` component |
| **TUI** (ratatui) | Shows `text` output: "Artifact 'X' (html) rendered" | Existing `ToolCallCard` rendering — ignores `metadata` |
| **Future mobile** | Could render or show text | Depends on platform |
| **`mew chat --connect`** (TUI via daemon) | Text output only | Same as TUI |
| **Session resume from disk** | Metadata persists in JSONL | `ToolStateCompleted.metadata` is already serialized |

The model sees the `text` field in conversation history, not the `metadata`. So the model knows the artifact was displayed, but future turns see only the text summary — not the full HTML content. This is correct: the artifact is a UI affordance, not conversation context. (If the model needs to reference the artifact content later, it already has it in the tool call input.)

## Tool registration: web-only

The tool is registered conditionally based on whether the frontend can render artifacts. Two approaches:

### Approach A: Always register, frontend degrades (Recommended)

Register `present_artifact` in all daemon modes. The system prompt includes it in the tool list. If the frontend can't render artifacts, the `metadata` is ignored and the model sees only the text output.

**Pros:** Simple, no capability negotiation needed, the model always knows it has the tool.
**Cons:** The model might call `present_artifact` in a TUI session where it's useless.

### Approach B: Capability-gated

The client sends a `capabilities` field on `NewSession` / `AttachSession` indicating `supports_artifacts: true`. The daemon only registers the tool (and includes it in the system prompt) when the frontend declares support.

**Pros:** No wasted tool calls in TUI mode.
**Cons:** Requires protocol change (`NewSession { capabilities: ... }`), capability negotiation logic, and the system prompt changes per-client (which is complex in shared sessions where multiple clients may have different capabilities).

**Decision: Approach A.** The cost of a wasted `present_artifact` call in TUI mode is one extra tool round-trip — annoying but not harmful. Capability negotiation adds complexity that isn't worth it for Phase 1. If the model overuses the tool in TUI mode, we can add the system prompt instruction "only use `present_artifact` when the user is in a web-based interface" and trust the model to follow it.

## Rendering components

### ArtifactPanel (React)

Detects `metadata.artifact.type` and delegates:

| Type | Renderer | Sandbox |
|------|----------|---------|
| `html` | `<iframe srcdoc={content} sandbox="allow-scripts">` | No access to parent DOM, cookies, network |
| `svg` | `dangerouslySetInnerHTML` (SVG is safe — no script execution) | Inline, scaled to container |
| `mermaid` | `mermaid.render(content)` via `@mermaid-js/mermaid` | Inline, themed |
| `markdown` | `<MarkdownBody>` (existing component with Shiki highlighting) | Inline |

Each artifact renders in a collapsible card with:
- Title bar (the `title` field)
- Type badge (html/svg/mermaid/markdown)
- Copy button (copies raw `content`)
- Expand/collapse toggle
- The rendered content below

### ToolCallCard integration

The existing `ToolCallCard` component checks `part.state.metadata?.artifact` when the tool is `present_artifact` and the state is `completed`. If the artifact metadata is present, it renders `ArtifactPanel` instead of the plain output panel. If not present (e.g., TUI mode where metadata is dropped), it falls back to the text output.

## TS type updates

Add `metadata` to the `completed` ToolState in `mew-web-client/src/index.ts`:

```ts
| {
    status: "completed";
    input: unknown;
    output: string;
    metadata?: { artifact?: ArtifactData } & Record<string, unknown>;
    time: { start: number; end: number | null };
  }
```

```ts
export interface ArtifactData {
  type: "html" | "svg" | "mermaid" | "markdown";
  title: string;
  content: string;
}
```

## System prompt

The tool description (shown to the model) includes guidance:

> Present rich content as a visual artifact. Use this for diagrams, interactive HTML, or formatted content that benefits from rendering. The artifact renders in a panel in the chat UI. The user sees the rendered content, not the raw code. Use sparingly — for regular text responses, just respond normally.

## Security

- **HTML artifacts** render in a sandboxed iframe (`sandbox="allow-scripts"`). The iframe cannot access the parent page's DOM, cookies, localStorage, or make network requests (no `allow-same-origin`). Scripts inside the iframe *can* run (for interactivity), but they're isolated.
- **SVG artifacts** are injected via `dangerouslySetInnerHTML`. SVG doesn't execute scripts in modern browsers when injected this way (the `<script>` tag in SVG is ignored by the HTML parser when set via `innerHTML`). This is safe for static diagrams.
- **Mermaid artifacts** are parsed by the Mermaid library, which produces safe SVG output. The input is diagram source code, not HTML.
- **Markdown artifacts** go through the existing `MarkdownBody` component, which sanitizes via the markdown renderer. No raw HTML is passed through.
- **Content size limit**: cap at 1 MB to prevent the model from generating enormous artifacts. The tool returns an error if `content.len() > 1_000_000`.

## What stays the same

- **Wire protocol** — no new `ServerMessage` or `ClientMessage` variants.
- **Message model** — no new `Part` variant. Artifacts are tool results with metadata.
- **Session persistence** — `ToolStateCompleted.metadata` is already serialized to JSONL.
- **Provider adapters** — unaware of artifacts. They just see a tool call and result.
- **TUI** — unchanged. Shows the text output, ignores metadata.
- **Compaction** — artifacts survive compaction because they're tool results, not separate parts.

## Plumbing gap (must fix before artifacts work)

The `ToolOutput` type in `mew-hooks` (`lib.rs:169`) currently has `output`, `error`, `diff` — but **no `metadata` field**. And the agent's tool execution loop (`tools.rs:632`) hardcodes `metadata: None` when constructing `ToolStateCompleted`. So even though `ToolStateCompleted.metadata` exists on the wire type and serializes to JSONL, it's never populated.

Fix needed:
1. Add `pub metadata: Option<serde_json::Value>` to `ToolOutput` in `mew-hooks/src/lib.rs`.
2. In `tools.rs:632`, replace `metadata: None` with `metadata: output.metadata.clone()`.
3. All existing `ToolOutput { ... }` constructors need `metadata: None` added (or derive `Default`).

This is a ~5-line change that unblocks the entire artifact pipeline.

## Implementation plan

1. **`present_artifact` tool** (`crates/mew-tools/src/tools/present_artifact.rs`): The tool implementation. Registered in `build_tools()` in `main.rs` (always available).
2. **TS type update** (`mew-web-client/src/index.ts`): Add `metadata` + `ArtifactData` to `ToolState` completed variant.
3. **Store update** (`mew-web-ui/src/stores/session.ts`): `onPartUpdated` already extracts `metadata` from `ToolStateCompleted` — extend it to store artifact data on the `MessagePart`.
4. **`ArtifactPanel` component** (`mew-web-ui/src/components/ArtifactPanel.tsx`): Renders HTML/SVG/Mermaid/Markdown based on type.
5. **`ToolCallCard` integration**: When `toolName === "present_artifact"` and `state === "completed"`, render `ArtifactPanel` instead of the plain output.
6. **Mermaid dependency**: `pnpm add mermaid` in `mew-web-ui`.

## Open questions

1. **Multiple artifacts per turn**: The model could call `present_artifact` multiple times. Each is a separate tool call, so they render as separate cards. This is fine — no special handling needed.

2. **Artifact in shared sessions**: If one client is web and another is TUI, the web client renders the artifact and the TUI sees the text output. This is correct — each frontend renders what it can.

3. **Edit/update artifacts**: Should the model be able to update an existing artifact (e.g., "add a button to the form")? For Phase 1, no — each `present_artifact` call creates a new card. If needed later, the tool could take an optional `update_call_id` field that replaces a previous artifact's content.

4. **Mobile**: The iframe approach works on mobile browsers. Mermaid and SVG render fine. No special handling needed for PWA.
