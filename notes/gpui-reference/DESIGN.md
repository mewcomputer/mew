# mew native desktop design conventions

Follow the existing desktop shell and [theme contract](THEMING.md). These are
mew product conventions, not GPUI framework requirements.

## Existing composition

The shell has three full-height panes: workspace/chat navigation, conversation,
and work content. Each owns its 38 px top header. Dividers continue through the
headers and content; avoid a global titlebar row spanning the three owners.
Layout constants live in `shell/types.rs`; width constraints and transition math
live in `shell/helpers.rs`.

The workspace-grouped navigation pane owns chat navigation. The conversation
header shows the selected workspace and chat; conversation tabs are not a second
navigation surface. Reserve native window controls in the navigation header, or
in the conversation header while navigation is hidden. Use shared vertical
dividers instead of enclosing rounded panels.

Conversation width starts at 480 px and is retained per chat. Work content fills
the remaining width, including space gained when the window grows. Dragging the
conversation/work divider changes the conversation width. Preserve the 420 px
conversation and 360 px work minima, including the divider. Narrow windows fold
work content; navigation resizing also preserves the conversation minimum.
Legacy saved workbench widths use the new conversation default.

Work content has named horizontal Browser, Changes, Files and Activity tabs.
Changes gives the diff the flexible area, with changed files at the right of a
wide work pane or above the diff in a narrower one. The terminal docks beneath
work content in split view and beneath the conversation when work is folded.
Keep its retained entity and daemon session ownership across that move.

New chat sits above the workspace list and inherits the selected workspace.
Search and task filters open through the sidebar search control or Command-K.
Keep idle workspace-root chats compact; expose their path and age through
accessible labels and tooltips. Nested working directories and attention states
remain visible. Selecting a chat reveals its workspace, and selecting the
already-attached chat preserves its loaded content. Closing a chat returns to
the unbound composer while retaining its draft and view state for later use.

Use shared neutral surfaces and semantic accents from the theme manifest in both
light and dark modes. Existing desktop typography uses the platform UI font for
chrome and the bundled mono font for terminal/code content; do not apply the
old Zedra monospace-first rule to every control.

## Controls and focus

Reuse the existing desktop element patterns, Tabler icons and
`DesktopFocusExt::desktop_focus`. Give icon-only controls an accessible label.
Keep selected, disabled, pending and error states visible without relying on
hover alone. Preserve keyboard actions and field focus when adding pointer
handlers.

Mew's shell fields share `ComposerElement` and the shell input handler.
GPUI does not supply a ready-made text input component for this application.
Keep native composition, selection, range geometry and Unicode handling intact
when styling or laying out those fields.

## Content and errors

Keep sessions and review rows easy to scan. Use the existing virtualized
transcript, review and sidebar structures rather than building an unbounded
element collection. Preserve per-session draft and scroll state during navigation.

Display connection and browser failures through the existing stateful surfaces.
A deferred failure must request a redraw immediately. Use the existing native
dialog APIs only when the interaction calls for them; Zedra's native mobile
sheets are not part of mew's desktop architecture.

## Verification

Check narrow/wide layouts, light/dark themes, long labels, keyboard focus and
Unicode input. Verify behavior with real view/transport tests and native smoke
checks. A screenshot demonstrates layout; it cannot establish callback
ownership, cancellation or render cost.
