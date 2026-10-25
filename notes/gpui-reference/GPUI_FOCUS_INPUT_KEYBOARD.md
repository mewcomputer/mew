# mew GPUI focus, text input and keyboard

Verified against the [pinned input and dispatch sources](README.md#source-policy).

## Focus and dispatch

Retain a `FocusHandle` for each field or keyboard scope. At this pin,
`.track_focus(&handle)` registers the focus scope and installs default pointer-down
focus transfer. `window.prevent_default()` can suppress that transfer.
`.manual_focus()` is not an API at this revision.

Use typed actions and key contexts for application commands. Keep composer,
model search, sidebar search, rename, browser URL and terminal routing distinct.
Use the existing `DesktopFocusExt::desktop_focus` for tabbable controls and visible
keyboard focus. An accessible name does not replace keyboard behavior.

Mew's application actions are registered in `shell/platform.rs` and attached in
`shell/render.rs`. Text editing actions belong on the rendered field's container,
so a hidden field with a retained focus handle cannot enable an Edit menu item.
Field editing/routing lives in `shell/composer.rs` and `shell/session.rs`;
model-picker editing also lives in `shell/preferences.rs`.
Terminal keys use Ghostty's encoder in
`terminal/key_map.rs` and `terminal/terminal_view.rs`.

A named key such as `up` is not a text character. Preserve modifiers when routing
named keys to the terminal encoder; control-character shortcuts must only match
the intended single-character input.

## Native editing commands

Register key bindings before creating menus. GPUI uses those bindings to assign
native menu key equivalents. Use `MenuItem::os_action` for Cut, Copy, Paste and
Select All so AppKit can dispatch to the native first responder, with typed GPUI
actions handling the shell fields. Keep clipboard image attachments in the
composer's paste path.

GPUI focus and AppKit's first responder can differ while a CEF page is active.
The native CEF host offers editing key equivalents to its visible focused native
view before GPUI handles them. Chromium's macOS clipboard commands also depend
on the application's native Edit menu. Preserve both routes and keep global
application commands in the existing window handler.

## Native text input

The existing shell fields use `ComposerElement` and the shell's
`EntityInputHandler`. During paint, register
`ElementInputHandler::new(bounds, entity)` with `Window::handle_input`.
The pin only activates that registration for the focused handle, and it lasts
until the next frame.

Adding a shell field means extending `TextInputTarget` and its value, selection,
focus and editing routes together. Retain a field-specific `FocusHandle`.

Key events alone do not cover native text input. Keep committed text,
marked/composition text, selection, text queries and candidate geometry in the
native input-handler path. Do not send an in-progress composition as a command.

Mew currently uses raw key encoding for its terminal. Native terminal IME
composition is an existing unsupported capability; the shell fields' IME support
does not imply terminal support.

## Text ranges and geometry

Platform text ranges use UTF-16 code units. Rust string offsets use UTF-8 bytes.
Convert using the existing `byte_offset_for_utf16` and
`utf16_offset_for_byte` helpers and clamp to valid boundaries before slicing.
Use grapheme-aware movement/deletion for user editing. Terminal selection uses
cell columns, which are a third coordinate system; map columns to row-text ranges
before copying, including wide and combining characters.

`bounds_for_range` supplies native IME candidate positioning. Return window
logical-pixel geometry from the actual shaped, painted field lines, including
a caret rectangle for an empty range. Reusing geometry from a different field,
old text or a different wrapping width can place candidates incorrectly.
Respect explicit newlines, wrapped rows, surrogate pairs and the document end.
The macOS backend converts this window-space result to screen coordinates.

AppKit can query geometry immediately after a composition callback, before GPUI
paints again. Refresh shaped text synchronously with the last painted field's
font, font size and wrapping width. Reject mismatched field/text geometry rather
than returning a stale rectangle. After paint, call
`Window::invalidate_character_coordinates` when the focused field's geometry,
text style or selection changes; avoid invalidating it on every idle frame.

For application-driven text or selection changes, the pin also exposes
`Window::notify_text_input_changed` and `notify_text_selection_changed` for native
services. These notifications complement GPUI redraw; use them when changing the
registered input through application commands.

The composer shapes the complete draft, measures against its available width
and normally caps its editing viewport at 96 px. Long drafts expose a measured
visual-line count, a persistent scrollbar and an Expand/Collapse control. The
expanded viewport is capped at 360 px and half the conversation area, accounting
for measured attachments, errors, controls and a folded terminal. Keep space for
the transcript; omit Expand when the available budget cannot grow the editor.
Paint text, selection and caret through
the same content mask and scroll offset used by pointer lookup and native range
geometry. Retain composer geometry separately from the focused shell field so
scrolling an unfocused draft cannot reuse a URL or search layout. Manual scroll
holds until editing, selection, focus or geometry requires revealing the caret.
Expanding or collapsing preserves an offscreen reading position. When the caret
was visible, keep it visible in the new viewport. Toggling preserves selection
and marked text and returns keyboard focus to the editor. Successful submission
and an actual draft-owner change reset expansion and scrollbar dragging; failed
submissions and reselecting the current chat retain expansion.

Vertical draft movement keeps the preferred horizontal position across short
lines and tracks the downstream side of soft wraps. Refresh retained geometry
when editing precedes the next paint. Native composition and selection callbacks
must refresh caret/scroll geometry even if the text bytes are unchanged; reuse
the shaping in that case.

## Pointer selection

`WrappedLine::paint` paints glyphs and text decorations. Paint selected text's
background separately with `WrappedLine::paint_background` before the glyphs.
Use the theme's paired selection foreground/background colors together; fading
only the background can hide the inverted foreground against the field.

Selection starts, drag motion and release must invalidate the owning view when
the highlight changes. Handle `on_mouse_up_out` as well as release inside the
field, and check whether the pointer is still dragging before extending selection.
Releasing over another panel must not leave a stuck drag.

Transcript selection takes its own focus handle and typed Copy action so an old
composer selection cannot win Cmd-C or the native Edit menu. Code fences and
tool input/output/diffs use the same selectable surface as prose. Register text
layouts after child prepaint, because virtual lists can measure unpainted rows.
Keep source byte offsets for tool lines and reconstruct virtual code chunks
from retained markdown when copying across offscreen content. Copy buttons use
the complete section or fence source, including preview-hidden lines.

List items and table cells retain their parent block identity and use source
byte offsets inside that block. Avoid arithmetic child IDs that can collide
with the next top-level block. Copy structured content from the retained
projection, including offscreen rows; tables separate cells with tabs. Pointer
targeting must distinguish horizontal cells as well as vertical rows. At this
pin, `TextLayout` hit testing ignores text alignment, so position aligned cell
content directly to keep pointer geometry consistent with paint.

## Verification

Exercise each field's focus independently, including switching from CEF back to
GPUI. Test Unicode conversion and actual shaped range geometry. Native smoke
checks should cover typing, selection/copy, shortcuts, and IME candidate
placement. Unit geometry tests do not replace a native IME check.
