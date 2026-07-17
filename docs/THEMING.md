# mew theming

mew shares a single token vocabulary between the Rust TUI and the React web UI.
The source of truth is `crates/mew-tui/resources/theme_manifest.json`.

## Manifest format

```json
{
  "version": 1,
  "tokens": {
    "background": "#1e1e21",
    "foreground": "#ffffff",
    "text.body": "@foreground",
    "panel.overlay_hover": "@background",
    "_radius_note": "radius is 0.625rem; ..."
  },
  "themes": {
    "dark": { "mode": "dark", "tokens": {} },
    "light": { "mode": "light", "tokens": { "background": "#ffffff" } }
  }
}
```

- `tokens` — base values for every supported token.
- `themes` — sparse overrides on top of the base table. A theme can also set
  `"base": "<id>"` to inherit from another theme before applying its own tokens.
- Values starting with `@` are aliases to another token, e.g. `"text.body":
  "@foreground"`. Alias chains are resolved at load/codegen time and cycles are
  rejected.
- Keys starting with `_` are CSS-only metadata and are not emitted as color
  variables.

## Token vocabulary

Core shadcn-style tokens:

- `background`, `foreground`
- `card`, `card_foreground`
- `popover`, `popover_foreground`
- `primary`, `primary_foreground`
- `secondary`, `secondary_foreground`
- `muted`, `muted_foreground`
- `accent`, `accent_foreground`
- `destructive`, `destructive_foreground`
- `border`, `input`, `ring`
- `sidebar` and `sidebar_*` variants
- `chart_1` … `chart_4`

Extended TUI/web tokens:

- Surface: `panel.background`, `panel.overlay`, `panel.overlay_hover`,
  `tool.background`, `tool.border`, `status_bar.background`, `sidebar.background`,
  `divider`, `surface.success`, `surface.error`, `surface.inverse`
- Selection: `selection.foreground`, `selection.background`
- Text: `text.body`, `text.muted`, `text.placeholder`, `text.disabled`,
  `text.accent`, `text.success`, `text.warning`, `text.error`, `text.inverse`,
  `surface.inverse`
- Color scales: `{red,orange,yellow,green,blue,purple,cyan}.{fg,med,bg}`
- Terminal ANSI: `terminal.{black,red,green,yellow,blue,magenta,cyan,white,
  bright_*,dim_*}`
- Pills: `pill.{auto,permissive,dangerous,model,thinking,cwd,git,attention,
  persona,custom}.{fg,bg}`
- Markdown: `markdown.{paragraph,heading.foreground,heading.h1..h6,emphasis,
  strong,strikethrough,inline_code.{fg,bg},link_text,link_url,list_bullet,
  block_quote,thematic_break,table_header,table_cell,table_border,
  code_fence.{fg,bg,border},pending_indicator}`
- Syntax: `syntax.{comment,string,keyword,function,number,type,variable,
  operator,constant}`
- Dynamic persona accent: `persona.accent.{fg,bg}`

## Codegen

The `theme_codegen` binary is the single generator for all derived files:

```bash
just theme-codegen          # regenerate
just theme-codegen-check    # verify generated files are up-to-date
```

Outputs:

- `mew-web-ui/src/generated-themes.css` — one `[data-theme="..."]` block per
  selectable theme.
- `crates/mew-tui/src/theme_generated.rs` — compile-time base table and
  per-theme overrides.
- `crates/ratatui-mdstream/resources/theme.tmTheme` — syntect TextMate theme.

Generated files are checked in; `just theme-codegen-check` is part of `just ci`.

## Adding a new theme

1. Add an entry to `theme_manifest.json` under `themes` with `"mode": "dark"`
   or `"mode": "light"` and only the tokens that differ from the base.
2. For web UI themes, add metadata to `mew-web-ui/src/themes.json`.
3. Run `just theme-codegen`.
4. Verify with `pnpm build` and `cargo test -p mew-tui`.

## Persona accent colors

Persona accent colors are computed dynamically from the persona name (or an
explicit `color` field in `PERSONA.md`) and injected into a cloned theme table
via `Theme::with_persona_accent`. UI code that renders persona-accented surfaces
should call this once per render and resolve `persona.accent.fg` /
`persona.accent.bg` from the clone.

## Custom themes

Users can install a custom theme JSON file with:

```bash
mew theme install /path/to/my-theme.json
```

Custom themes are validated against the manifest: every token key must exist in
the base vocabulary. Unknown tokens are rejected. Use `mew theme export-css
<name>` to inspect the resolved CSS block for any installed or built-in theme.

## ANSI colors

`terminal.*` tokens are reserved for future ANSI escape rendering of tool
output. They are not currently used by the streaming markdown path, which is
styled by the generated syntect theme instead.
