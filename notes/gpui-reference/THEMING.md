# mew GPUI theming

Mew uses `mew_tui::theme::Theme` in its GPUI desktop. GPUI supplies colors, styles
and appearance observation; the token names and loading policy are mew's.

## Source of truth

The manifest at
[theme_manifest.json](../../crates/mew-tui/resources/theme_manifest.json) owns
the shared tokens. Change that manifest and run `just theme-codegen` when adding
a shared token; never edit generated theme files by hand.
See [the shared theme guide](../../docs/using-mew/theming.md).

`DesktopShell` retains `theme`, `theme_mode`, `light_theme` and `dark_theme`.
`desktop_theme_name` selects the configured light/dark name from the preference
and `WindowAppearance`. `reload_theme` in `shell/lifecycle.rs` loads it and updates
child state. `shell/preferences.rs` persists the desktop preference.
The appearance subscription is retained on the shell.

Use the existing `theme_rgb(&self.theme, token)` adapter in desktop views.
Mew does not expose Zedra's `ThemePalette`, `ThemeBundle`,
`theme::palette(cx)` or `ThemeStateEvent::Changed` APIs.

## Tokens in desktop views

| Purpose | Existing token |
| --- | --- |
| Root chrome | `background` |
| Body text | `text.body` |
| Secondary text | `text.muted` |
| Panels | `panel.background` |
| Sidebar | `sidebar.background` |
| Inputs and quiet controls | `input`, `muted` |
| Borders | `divider` |
| Focus and active state | `text.accent`, `accent` |
| Selection | `selection.foreground`, `selection.background` |

Resolve semantic colors at render time through the existing adapter. Reuse the
existing shell layout constants and `DesktopFocusExt`. Keep literal colors in
conversion/infrastructure code or protocol-provided terminal styles where they
represent data, not product chrome.

## Child entities and caches

Reloading the shell theme does not automatically update a child entity's retained
colors or application-owned row cache. Pass changed colors to the terminal,
invalidate its row projection, and notify that entity. Default-color cells,
cursor and selection must update even when no new PTY bytes arrive. Keep explicit
ANSI foreground/background colors and inverse styles semantically correct.

Check both system appearance changes and explicit light/dark preferences.
Switch theme while the terminal is idle, with a selection, and after font changes;
waiting for terminal output can hide a missing invalidation.

## References

- [Rendering and caches](GPUI_RENDERING_MODEL.md)
- [Design conventions](DESIGN.md)
- [Pinned appearance/window API](https://github.com/zed-industries/zed/blob/322cd509dcccaee1f6bc44725d270598ae8171ad/crates/gpui/src/window.rs)
