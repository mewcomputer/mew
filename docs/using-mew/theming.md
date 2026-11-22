---
title: Theming
description: Switch built-in themes, install custom themes, and edit theme tokens.
---

Mew shares a single theme token vocabulary across the Rust TUI, the native macOS
desktop app, and the React Web UI, generated from
`crates/mew-tui/resources/theme_manifest.json`.

## Switching and installing themes

In the TUI, run `/theme <name>` to apply a theme, or `/theme` to open the theme
picker. From the CLI, list themes, check the active theme, install a JSON theme,
or print a theme's CSS variables:

```sh
mew theme list
mew theme current
mew theme install /path/to/my-theme.json
mew theme export-css my-theme
```

## Custom themes

Mew loads custom JSON themes from `.mew/themes/<name>.json` (checked first) and
`~/.config/mew/themes/<name>.json`:

```json
{
  "name": "my-theme",
  "mode": "dark",
  "tokens": {
    "background": "#18181b",
    "foreground": "#f4f4f5",
    "accent": "#38bdf8"
  }
}
```

Mew validates every token key in `tokens` against the manifest vocabulary and
rejects unknown keys. Custom JSON themes do not support `base` inheritance.

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

- `tokens`: default values for all supported tokens.
- `themes`: sparse per-theme overrides on top of `tokens`. Manifest themes may
  declare `"base": "<id>"` for runtime loading via `Theme::from_manifest`, though
  `theme_codegen` ignores `base`.
- `@alias`: values prefixed with `@` reference another token (for example,
  `"text.body": "@foreground"`). Runtime loading rejects alias cycles; CSS
  codegen warns and skips the theme, and Rust codegen stops resolving on a
  cycle.
- `_metadata`: keys prefixed with `_` supply CSS-only metadata and do not emit
  color variables.

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
- `chart_1` through `chart_4`

Extended TUI, desktop, and web tokens:

- **Surface**: `panel.background`, `panel.overlay`, `panel.overlay_hover`,
  `chat.surface.background`, `chat.user.background`, `tool.background`,
  `tool.border`, `status_bar.background`, `sidebar.background`, `divider`,
  `surface.success`, `surface.error`, `surface.inverse`
- **Selection**: `selection.foreground`, `selection.background`
- **Text**: `text.body`, `text.muted`, `text.placeholder`, `text.disabled`,
  `text.accent`, `text.success`, `text.warning`, `text.error`, `text.inverse`
- **Color scales**: `{red,orange,yellow,green,blue,purple,cyan}.{fg,med,bg}`
- **Terminal ANSI**: `terminal.{black,red,green,yellow,blue,magenta,cyan,white,
  bright_*,dim_*}`
- **Pills**: `pill.{auto,permissive,dangerous,model,thinking,cwd,git,attention,
  persona,custom}.{fg,bg}`
- **Markdown**: `markdown.{paragraph,heading.foreground,heading.h1..h6,emphasis,
  strong,strikethrough,inline_code.{fg,bg},link_text,link_url,list_bullet,
  block_quote,thematic_break,table_header,table_cell,table_border,
  code_fence.{fg,bg,border},pending_indicator}`
- **Syntax**: `syntax.{comment,string,keyword,function,number,type,variable,
  operator,constant}`
- **Dynamic persona accent**: `persona.accent.{fg,bg}`

## Codegen and adding a built-in theme

`theme_codegen` generates all derived assets from `theme_manifest.json`:

```sh
just theme-codegen          # regenerate assets
just theme-codegen-check    # check generated files (also run by just ci)
```

Outputs:

- `mew-web-ui/src/generated-themes.css`: `[data-theme="..."]` CSS blocks for
  built-in web themes (skips `dark`, `light`, and `terminal.*` / `pill.*` /
  `syntax.*` tokens).
- `crates/mew-tui/src/theme_generated.rs`: compile-time base token table and
  built-in theme overrides.
- `crates/ratatui-mdstream/resources/theme.tmTheme`: syntect TextMate theme built
  from the `dark` theme.

To add a built-in theme:

1. Add an entry under `themes` in `theme_manifest.json` with `"mode": "dark"` or
   `"mode": "light"` and the overridden tokens.
2. Add Web UI metadata in `mew-web-ui/src/themes.json` if the theme should
   appear in the browser switcher.
3. Run `just theme-codegen`.
4. Verify with `just build-ui` (or `pnpm --filter mew-web-ui build`) and
   `cargo test -p mew-tui`.

## Persona accent colors

Mew derives persona accent colors dynamically from the persona name or an
explicit `mew.color` (or `polytoken.color`) value in `PERSONA.md`. Rendering code
calls `Theme::with_persona_accent` once per render and reads `persona.accent.fg`
and `persona.accent.bg` from the returned theme clone.

## ANSI colors

Mew reserves `terminal.*` tokens for future ANSI escape rendering of tool
output; streaming Markdown syntax highlighting uses the generated
`theme.tmTheme` file.
