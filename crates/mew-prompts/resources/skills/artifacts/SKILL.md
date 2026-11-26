---
name: artifacts
description: Reference for designing, building, and publishing interactive HTML/JS/CSS artifacts with present_artifact — covers design with intent, inline vs panel mode selection, injected CSS theme variables, utility classes, Preact runtime, and allowed CDNs.
---

# Interactive Artifacts

Use `present_artifact` to publish HTML/JS/CSS visuals or apps from the session workspace.

## Workflow

1. **Establish the intent** (keep this brief internal unless a proposal helps):
   - **Audience & job**: who is this for, and what question, decision, comparison, or interaction should it support?
   - **Medium & surface**: `inline` in the chat reply (~350px mobile width, ~150–500px height) or `panel` in the workbench/sheet?
   - **Real content**: draft with actual titles, labels, units, and realistic data lengths early. Never invent metrics, citations, or fake saved state to make a layout look complete.
2. Write the HTML/JS/CSS file(s) in the session workspace using `write`.
3. Call `present_artifact` with `title`, `path`, and `mode` (`inline` or `panel`).
4. **Iterating on an artifact**: make targeted edits to the existing workspace file(s) with `edit` rather than rewriting everything from scratch, then call `present_artifact` again to publish the updated snapshot.

## Choosing `inline` vs `panel` vs plain markdown

- **Plain markdown / code block**: static text, tables, formulas, or code snippets that do not need interaction or custom visual layout.
- **`mode='inline'`**: focused, single-purpose visuals embedded directly in the chat reply — charts, interactive SVG/diagrams, calculators, parameter sliders, or small simulators.
  - Design for a narrow column (~350px mobile width up to chat width) with no horizontal overflow.
  - Keep vertical height compact (~150–500px; max initial height 900px).
  - Use local state only; avoid sidebars, multi-pane chrome, or full-page navigation.
- **`mode='panel'`**: full-screen or dense applications that warrant leaving the chat flow — editors, canvases, multi-view dashboards, games, or multi-file apps. Opens in a workbench tab on web/desktop and a modal sheet on iOS.

## Design with Intent

Let the artifact's purpose govern its structure, visual language, and interaction:

- **Organize real content**: establish reading and action order before decoration. Use a table for exact comparisons, a chart for numeric relationships, a diagram for sequence/connections, and controls for actions. Give unequal information unequal emphasis rather than forcing equal cards or symmetric pillars.
- **Informative labels**: prefer specific titles, units, scales, and findings over generic headings like "Overview" or "Key Insights". Use `tabular-nums` for live or aligned numbers.
- **Theme-first visual system**: style surfaces, text, borders, and charts with the injected CSS variables (`--background`, `--card`, `--foreground`, `--muted-foreground`, `--border`, `--primary`, `--chart-1..4`) so the UI adapts to light/dark mode and theme switches. Only override with a custom palette when the subject calls for it (e.g. a game or themed simulation).
- **Accessible & responsive**: use fluid widths (`w-full`, `min-w-0`, `max-w-full`), semantic `<button>` / `<input>` / `<label>` elements, visible focus, readable contrast, and `aria-live="polite"` on dynamic readouts.

### Catch Autopilot Choices

| Suspected habit | Ask and adjust |
| --- | --- |
| Every section becomes an equal card or tile | Does the content have equal weight and independent boundaries? Use hierarchy, rows, or sections where relationships require it. |
| Gradients, glows, glass panels, shadows, or pills accumulate | What does each treatment communicate? Keep purposeful hierarchy; remove competing decoration. |
| An icon accompanies every heading | Does the icon improve recognition? Keep meaningful symbols and drop habitual decoration. |
| Dashboard totals and charts fill space | Which comparison or decision do they support? Show units, scales, and context; remove filler displays. |
| Polished controls imply behavior that doesn't exist | Make the interaction work within scope, or clearly communicate limits. Never pretend data was saved when it wasn't. |

If the artifact could be repurposed for an unrelated subject by changing only its title, revisit the content, hierarchy, and details.

### Final Checks

- **Charts & diagrams**: verify plotted values against the source; include units, scales, and direct labels where helpful; keep diagram labels short and topology readable at target width.
- **Staging & sandbox limits**: artifacts run in a sandboxed `null`-origin iframe (`allow-scripts allow-downloads allow-forms`, no `allow-same-origin`), so `localStorage`, `sessionStorage`, and `indexedDB` throw `SecurityError` — keep state in memory (or offer JSON file download) instead. Deliver complete, working code without `// TODO` placeholders.
- **Interactive UIs**: exercise the primary flow and edge states (empty, error, disabled); check wrapping, contrast, keyboard order, and mobile-width (~350px) overflow.

## Staging & Multi-File Rules

- **Workspace root file** (e.g. `chart.html`): only that single file (plus `uploads/` if present) is staged.
- **Subdirectory** (e.g. `path: "my-app"` or `path: "my-app/index.html"`): the entire subdirectory is staged (up to 256 files / 50 MiB, skipping `.git`, `node_modules`, `target`, `.next`, `dist`, `.venv`, `__pycache__`), so relative CSS, JS/ES modules, JSON, and image assets are served alongside `index.html`.
- Put multi-file apps in their own subdirectory.

## `mode='inline'` Runtime & Styling

For `mode='inline'`, you can write a bare HTML fragment (no `<!doctype>`, `<html>`, `<head>`, or `<body>` needed). `present_artifact` wraps it and injects:
- Auto-resize via `ResizeObserver` (`mew:artifact-resize`).
- Default light/dark fallback theme (`prefers-color-scheme: dark` when opened outside mew).
- Live theme syncing from mew (`mew:theme` updates `:root` CSS variables and `.dark` class on theme or dark/light switch).
- `/artifact-runtime/inline.css` with theme-aware variables and common utility classes.

### Injected CSS Variables

- Surfaces & text: `--background`, `--foreground`, `--card`, `--card-foreground`, `--popover`, `--popover-foreground`, `--muted`, `--muted-foreground`, `--accent`, `--accent-foreground`
- Actions & status: `--primary`, `--primary-foreground`, `--secondary`, `--secondary-foreground`, `--destructive`, `--destructive-foreground`
- Borders & focus: `--border`, `--input`, `--ring`
- Charts: `--chart-1`, `--chart-2`, `--chart-3`, `--chart-4`

### Available Utility Classes (`/artifact-runtime/inline.css`)

- **Colors**: `bg-background`, `text-foreground`, `bg-card`, `text-card-foreground`, `bg-muted`, `text-muted-foreground`, `bg-primary`, `text-primary-foreground`, `bg-secondary`, `text-secondary-foreground`, `bg-accent`, `text-accent-foreground`, `bg-destructive`, `text-destructive`, `text-destructive-foreground`, `border-border`, `border-input`, `bg-transparent`
- **Layout**: `block`, `inline-block`, `inline`, `flex`, `inline-flex`, `grid`, `hidden`, `flex-col`, `flex-row`, `flex-wrap`, `flex-1`, `shrink-0`, `items-start`, `items-center`, `items-end`, `justify-start`, `justify-center`, `justify-between`, `justify-end`, `grid-cols-1`..`grid-cols-4`, `w-full`, `w-auto`, `h-full`, `min-w-0`, `max-w-full`, `overflow-hidden`, `overflow-x-auto`
- **Spacing**: `gap-1`, `gap-1.5`, `gap-2`, `gap-3`, `gap-4`, `gap-6`, `p-0`..`p-6`, `px-2`, `px-2.5`, `px-3`, `px-4`, `py-1`, `py-1.5`, `py-2`, `py-3`, `m-0`, `mt-1`..`mt-4`, `mb-1`..`mb-4`, `space-y-2`..`space-y-4`
- **Typography & borders**: `text-xs`, `text-sm`, `text-base`, `text-lg`, `text-xl`, `text-2xl`, `font-normal`, `font-medium`, `font-semibold`, `font-bold`, `font-mono`, `tabular-nums`, `text-left`, `text-center`, `text-right`, `border`, `border-b`, `border-t`, `rounded`

Note: `/artifact-runtime/inline.css` provides these specific utility classes, not a full JIT Tailwind compiler — if you need custom layout/animation rules, write them in a `<style>` tag using the CSS variables above.

## `mode='panel'` Runtime & External Packages (CDNs)

- Write a complete HTML document (e.g. `my-app/index.html`), optionally with separate `.css` and `.js`/`.mjs` files in the same directory.
- **Preact** is available locally at `/artifact-runtime/preact.mjs` (no external CDN needed):
  ```html
  <script type="module">
    import { h, render } from '/artifact-runtime/preact.mjs';
  </script>
  ```
- Panel artifacts can also listen for `window.addEventListener('message', e => { if (e.data?.type === 'mew:theme') ... })` if you want them to react to mew theme switches.
- For small `inline` widgets, prefer zero-dependency HTML/CSS/JS so they load immediately and work offline.
- When a richer library genuinely helps (e.g. `chart.js`, `d3`, `three`, `lucide`), you may load version-pinned packages from allowed CDNs such as `unpkg.com` (`https://unpkg.com/...`) or `cdnjs.cloudflare.com` (`https://cdnjs.cloudflare.com/...`). Use `/artifact-runtime/preact.mjs` for Preact instead of fetching it from a CDN.
