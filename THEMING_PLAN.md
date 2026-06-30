# Theming Plan — Token-Based (v2)

## Design principles

1. **Extended shadcn model** — use all current shadcn tokens (including
   the newer `sidebar-*`, `popover-*`, `chart-*` sets), plus mew-specific
   extensions.
2. **Three-tier color scales** for semantic colors — each semantic color
   (red, green, yellow, blue, purple, cyan) has `fg`, `med`, and `bg`
   tiers. Pills, badges, and accents compose from these instead of having
   per-use token pairs. This means a theme author sets 3 values per color
   instead of N pill pairs.
3. **JSON files, installable** — themes are JSON files dropped in
   `~/.config/mew/themes/`. Partial themes work (missing tokens fall back
   to the built-in `dark` theme).

## Token model

### Core shadcn tokens (30)

These match shadcn's current token set exactly (v4). Both TUI and web UI
use them.

| Token | Purpose |
|-------|---------|
| `background` | App background (chat surface) |
| `foreground` | Primary text |
| `card` | Cards (tool calls, modals) |
| `card_foreground` | Text on cards |
| `popover` | Popover background (picker, dropdown) |
| `popover_foreground` | Text on popovers |
| `primary` | Primary accent (buttons, focus) |
| `primary_foreground` | Text on primary |
| `secondary` | Secondary surface |
| `secondary_foreground` | Text on secondary |
| `muted` | Muted background |
| `muted_foreground` | Labels, descriptions, hints |
| `accent` | Accent surface |
| `accent_foreground` | Text on accent |
| `destructive` | Destructive actions |
| `border` | Borders, dividers |
| `input` | Input field background |
| `ring` | Focus ring / cursor |
| `sidebar` | Sidebar background |
| `sidebar_foreground` | Sidebar text |
| `sidebar_primary` | Sidebar active item |
| `sidebar_primary_foreground` | Text on sidebar primary |
| `sidebar_accent` | Sidebar hover/accent |
| `sidebar_accent_foreground` | Text on sidebar accent |
| `sidebar_border` | Sidebar dividers |
| `sidebar_ring` | Sidebar focus ring |
| `chart_1` | Chart color 1 |
| `chart_2` | Chart color 2 |
| `chart_3` | Chart color 3 |
| `chart_4` | Chart color 4 |

(5 chart colors — `chart_5` omitted since we don't have charts in the
TUI yet, but can be added later.)

### mew extensions: three-tier semantic colors

Instead of `model_pill_fg`, `model_pill_bg`, `perm_auto_fg`, etc., define
6 semantic colors, each with 3 tiers:

```
red_fg    red_med    red_bg      # errors, dangerous, destructive pills
green_fg  green_med  green_bg    # success, completed tools, model pill
yellow_fg yellow_med yellow_bg   # warnings, permissive, slash commands
blue_fg   blue_med   blue_bg     # info, cwd pill, links
purple_fg purple_med purple_bg   # personas (default accent fallback)
cyan_fg   cyan_med   cyan_bg     # prompts, active cursors, info accents
```

**`fg`** — bright, readable on `med` and `bg`. Used for text on pills.
**`med`** — mid-saturation. Used for borders, icons, markers.
**`bg`** — dark, low-saturation. Used for pill backgrounds, block fills.

### How pills map to the scale

| UI element | fg | bg |
|-----------|-----|-----|
| Model pill | `green_fg` | `green_bg` |
| Cwd pill | `blue_fg` | `blue_bg` |
| Persona pill (default) | `purple_fg` | `purple_bg` |
| Persona pill (accent) | computed accent fg | computed accent bg |
| Permissive badge | `yellow_fg` | `yellow_bg` |
| Auto badge | `purple_fg` | `purple_med` |
| Auto+ badge | `purple_fg` | `purple_bg` (darker) |
| Dangerous badge | `red_fg` | `red_bg` |

This means a theme author defines 6 colors × 3 tiers = 18 values for all
pills and semantic accents, instead of ~20 individual pill pairs.

### Other mew extensions

| Token | Purpose |
|-------|---------|
| `status_bg` | Status bar + input area background (distinct from `background`) |
| `tool_bg` | Tool call block background (maps to `card` if unset) |
| `divider` | Subtle divider lines (maps to `border` if unset) |

These 3 are surfaces that don't have a clean shadcn equivalent. They
default to the nearest shadcn token if omitted.

## Token count

- 30 core shadcn tokens
- 18 semantic color tiers (6 colors × 3)
- 3 mew surface extensions
- **= 51 tokens total**

A minimal theme can override just a few:

```json
{
  "name": "Warm Dark",
  "mode": "dark",
  "tokens": {
    "background": "#2a1f1a",
    "status_bg": "#1f1611",
    "sidebar": "#1f1611"
  }
}
```

Everything else inherits from the built-in `dark` theme.

## Theme file format

```json
{
  "name": "Catppuccin Mocha",
  "mode": "dark",
  "tokens": {
    "background": "#1e1e2e",
    "foreground": "#cdd6f4",
    "card": "#313244",
    "card_foreground": "#cdd6f4",
    "popover": "#313244",
    "popover_foreground": "#cdd6f4",
    "primary": "#89b4fa",
    "primary_foreground": "#1e1e2e",
    "secondary": "#313244",
    "secondary_foreground": "#cdd6f4",
    "muted": "#45475a",
    "muted_foreground": "#a6adc8",
    "accent": "#313244",
    "accent_foreground": "#cdd6f4",
    "destructive": "#f38ba8",
    "border": "#45475a",
    "input": "#313244",
    "ring": "#89b4fa",
    "sidebar": "#181825",
    "sidebar_foreground": "#cdd6f4",
    "sidebar_primary": "#89b4fa",
    "sidebar_primary_foreground": "#1e1e2e",
    "sidebar_accent": "#313244",
    "sidebar_accent_foreground": "#cdd6f4",
    "sidebar_border": "#45475a",
    "sidebar_ring": "#89b4fa",
    "status_bg": "#181825",
    "tool_bg": "#313244",
    "divider": "#45475a",
    "red_fg": "#f38ba8",
    "red_med": "#d4004a",
    "red_bg": "#4a1c2e",
    "green_fg": "#a6e3a1",
    "green_med": "#1e8a17",
    "green_bg": "#1e3a2e",
    "yellow_fg": "#f9e2af",
    "yellow_med": "#e0a800",
    "yellow_bg": "#3a2e0f",
    "blue_fg": "#89b4fa",
    "blue_med": "#2a6dd4",
    "blue_bg": "#1e2e4a",
    "purple_fg": "#cba6f7",
    "purple_med": "#6b3fa0",
    "purple_bg": "#2e1f4a",
    "cyan_fg": "#94e2d5",
    "cyan_med": "#0d9488",
    "cyan_bg": "#0f3a3a"
  }
}
```

## Discovery, loading, config, CLI — same as v1 plan

- Search paths: `~/.config/mew/themes/`, `.mew/themes/`, built-ins
- Config: `[tui] theme = "catppuccin-mocha"`
- `/theme` slash command for runtime switching
- `mew theme install/list/current` CLI subcommand
- Partial themes merge over built-in `dark`

## Implementation phases

1. **Theme struct + dark defaults + JSON loading** — Create struct,
   `Theme::dark()` with current values, JSON parsing with serde, search
   paths. Replace all hardcoded colors. Ship `dark` + `light`.

2. **Config + `/theme` command + `mew theme` CLI** — Wire config, runtime
   switching, CLI subcommands.

3. **Built-in themes** — Embed catppuccin-mocha, catppuccin-latte,
   tokyo-night as JSON via `include_str!`.

4. **Web UI bridge** — Web UI consumes same JSON (hex→HSL conversion).
