# GPUI Theming

> **Source note:** This guide was originally written for [tanlethanh/zedra](https://github.com/tanlethanh/zedra) and references zedra-specific code paths. The GPUI concepts generalize, but keep the original project context in mind when reading.

Supports **dark** and **light** appearance. New UI must use shared theme tokens so colors stay consistent and update when the user toggles appearance or when the app follows the system theme on launch.

## Source Of Truth

| Layer | Role |
|-------|------|
| App settings + appearance | Loads/saves app settings; a `ThemeState` entity manages `ThemePreference`, builds `ThemeBundle`, emits `ThemeStateEvent::Changed` |
| Token definitions | `ThemePalette`, `EditorTheme`, `ThemeBundle`, layout constants, `theme::palette(cx)` accessors |
| User control | Settings UI calls `ThemeState::set_preference` |

`ThemeState` is registered as a GPUI global at app startup. Views read the active bundle through `theme::palette(cx)` / `theme::bundle(cx)`, which delegate to the global entity.

```
Settings / system on launch
        ↓
   ThemeState (preference + ThemeBundle)
        ↓
   ┌────┴────┬──────────────┐
   │         │              │
GPUI views  Editor        Terminal
theme::*    EditorTheme   TerminalTheme
```

## Rules For New UI

**Required**

- Read **UI colors** from `theme::palette(cx)` or the `theme::bg_primary(cx)`-style accessors in `render()`. Wrap hex tokens with `rgb(...)` (or `Hsla` fields on the palette).
- Use **layout and typography constants** (`SPACING_*`, `FONT_*`, `ICON_*`, etc.). Those are appearance-independent; do not duplicate magic numbers in views.
- Use **semantic accents** only for meaning (connected, warning, destructive, focus).
- If a surface must react to a theme toggle, subscribe to `ThemeStateEvent::Changed` and call `cx.notify()` on the owning entity.

**Forbidden in view `render()` and leaf components**

- Hardcoded `0xRRGGBB` / `rgb(0x...)` for product chrome, text, borders, or backgrounds.
- Per-component light/dark branches (`if light { ... } else { ... }`) unless you are implementing theme infrastructure itself.
- Render-time hacks that only fix contrast for one screen (tint overlays, one-off HSLA nudges). Add or adjust a token instead.

## Using Tokens During Render

Pass `cx` and use accessors:

```rust
use crate::theme;

div()
    .bg(rgb(theme::bg_primary(cx)))
    .child(
        Label::new("Title")
            .text_color(rgb(theme::text_primary(cx)))
            .text_size(px(theme::FONT_BODY)),
    )
```

When several fields are needed together (e.g. badges), take `let palette = theme::palette(cx);` once.

`ThemePalette` fields:

| Field | Typical use |
|-------|-------------|
| `bg_primary`, `bg_surface` | Full-height shells, workspace background |
| `bg_card`, `bg_overlay` | Raised panels, cards, sheets |
| `bg_card_dim` | Lower-contrast card fill |
| `text_primary`, `text_secondary`, `text_muted` | Labels, body, metadata |
| `border_subtle`, `border_default`, `border_active`, `border_highlight` | 1px separators and control edges |
| `accent_green`, `accent_blue`, `accent_yellow`, `accent_red`, `accent_dim` | Status and semantic emphasis only |
| `row_pressed_bg`, `overlay_backdrop` | Pressed rows, modal backdrops (`Hsla`) |

Adding a new UI color: extend `ThemePalette` for both `dark()` and `light()`, add an accessor if it will be used widely, then use it from views.

## Subscribing To Theme Changes

**App shell** — subscribe to `ThemeState` and call `cx.notify()` so top-level screens re-render.

**Feature-owned surfaces** — subscribe where the entity owns the subtree that must update:

```rust
cx.subscribe(&theme_state, |this, _, _: &ThemeStateEvent, cx| {
    this.sync_theme(cx); // or cx.notify()
});
```

Call the sync helper once after subscribe setup so the initial preference is applied.

Changing preference:

```rust
theme_state.update(cx, |state, cx| {
    state.set_preference(ThemePreference::Light, cx);
});
```

## Checklist For New Screens

1. All backgrounds, text, and borders use `theme::` accessors (or `theme::palette(cx)` fields).
2. Spacing and font sizes use `theme::` layout constants.
3. Parent entity subscribes to `ThemeStateEvent::Changed` if the screen is not under the app shell's notify path.
4. Manual test: toggle light/dark and verify the screen updates.

## Related Docs

- [DESIGN.md](DESIGN.md) — visual tone and component patterns
- [CONVENTIONS.md](CONVENTIONS.md) — GPUI `render()` purity and UI design pointer
