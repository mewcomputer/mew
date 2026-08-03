# GPUI Focus, Input, and Keyboard Coordination

> **Source note:** This guide was originally written for [tanlethanh/zedra](https://github.com/tanlethanh/zedra) and references zedra-specific code paths. The GPUI concepts generalize, but keep the original project context in mind when reading.

GPUI treats focus, platform text input, and software-keyboard presentation as
separate responsibilities. Normal text inputs can use GPUI's default focus and
keyboard behavior. Elements that need custom tap/focus coordination can opt out
of defaults and manage focus themselves.

## Layers

```
tap / key input
    -> GPUI Window event dispatch
    -> FocusHandle state
    -> window.handle_input(focus_handle, input_handler, cx)
    -> PlatformInputHandler
    -> platform backend (e.g. UITextInput on iOS, wgpu on others)
    -> InputHandler
    -> application text sink
```

## Core Contract

`.track_focus(&focus_handle)` registers the handle in the focus tree, enables
focused styles and key context, and installs the default pointer-down focus
transfer. Suppress that default focus transfer with `.manual_focus()` when the
element owns focus changes itself. A lower-level pointer/mouse-down handler can
also suppress default focus by calling `Window::prevent_default()` before the
default focus listener runs.

`Window::handle_input(...)` normally registers the currently focused text
surface. A handler that owns native selection geometry can also be registered
before focus so the platform can ask whether a native selection gesture should
begin. `InputHandler::accepts_text_input()` only answers whether platform text
and IME callbacks should route to that handler.

`manual_focus()` disables implicit software-keyboard presentation for that
focused surface. The element can still receive text input
(`insertText`, `deleteBackward`, marked text) but must explicitly call
`window.show_soft_keyboard()` / `window.hide_soft_keyboard()` when it wants
the keyboard.

## Normal Input Flow

For normal editable text inputs:

```
focused handler accepts text
    -> handle_input registers PlatformInputHandler
    -> platform may auto request keyboard on a new handler session
    -> native editable text interaction can be enabled
```

## Custom Focus Flow

When an element needs to own its focus/keyboard coordination (e.g. a terminal
or custom input surface):

- Use `.track_focus(&focus_handle).manual_focus()`:
  - `track_focus` keeps focus state, styles, key context, and input registration
  - `manual_focus` prevents pointer-down from focusing before the tap completes
- Use GPUI's `on_press` for keyboard/focus toggling
- The element owns `focus()`, `blur()`, `show_soft_keyboard()`, and
  `hide_soft_keyboard()` calls
- Do not add artificial delays to focus/keyboard activation

## Platform Text Interaction Modes

The platform backend maps focus policies to native behavior:

```
accepts_text_input=true, manual_focus=false
    -> editable text interaction mode
    -> implicit keyboard request allowed

accepts_text_input=true, manual_focus=true
    -> editable text interaction mode once focused, or earlier if the handler
       explicitly owns native selection
    -> explicit keyboard request controls software-keyboard presentation
    -> platform text callbacks still route through the handler

selection handler present
    -> non-editable text interaction mode for read-only surfaces
```

Non-editable selection must not create keyboard focus or disturb the active
input handler.

## Logging

Keep these paths quiet in normal builds. The focus/keyboard path runs during
interaction and draw, so broad frame or text-input logs can mask the actual
timing issue and add debug-build overhead. If this regresses, add short-lived
targeted logs with a clear prefix, reproduce once, then remove them after the
cause is known.

## Key GPUI Source Files

| File | Purpose |
|------|---------|
| `crates/gpui/src/elements/div.rs` | `.track_focus` default focus and `.manual_focus()` opt-out |
| `crates/gpui/src/platform.rs` | `InputHandler` text policy and soft-keyboard auto-request helper |
| `crates/gpui/src/window.rs` | Focused input-handler registration |
