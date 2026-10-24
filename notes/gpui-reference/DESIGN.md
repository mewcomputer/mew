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

Work content has closable tabs for opened tools and browser pages. The plus
control and Command-T open a new-tab page with URL entry and a launcher for
Changes, Terminal, Files and Activity. Reopening a tool selects its existing tab;
browser pages have independent identities, URLs, titles and retained native pages.
Keep chat navigation in the left pane. Command-W closes a work tab while focus
is in work content, and retains chat-close behavior in the conversation.

Retain the tab order, selection and last browser target per chat. A first visit
to another chat inherits pane sizing with a fresh tab workspace. Keep the active
tab visible when the strip overflows, and keep the plus control accessible.

Changes gives the diff the flexible area, with changed files at the right of a
wide work pane or above the diff in a narrower one. Terminal fills its selected
work tab and falls back beneath the conversation in narrow windows where work
is folded. Keep its retained entity and daemon session ownership across that
move, and mount the entity in one place. Folded work content must not register
hidden input fields or editing actions.

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

The transcript and composer share a centered content column capped at 680 px,
with 24 px pane gutters. User bubbles fit within that column; assistant content
uses its full width. Conversation body text is 15 px with 24 px leading, while
headings and code set their own leading. Separate messages by 24 px and parts
within a message by 8 px. Keep virtual paragraph chunks continuous. The first
loaded turn has the same padding as later turns so history prepending preserves
the visible text position.

Long code fences remain virtualized but read as one surface. Split only between
physical lines, preserve oversized lines for horizontal scrolling, and apply
header/copy controls and outer padding only at the fence ends. Internal chunks
keep the same line leading and share a measured content width and horizontal
scroll position. Retain this state with the markdown cache; invalidate width
when its source, font or syntax theme changes.

Lists retain ordered starts and nesting. Keep markers in a separate aligned
column and indent nested content by 24 px so wrapped lines hang beneath their
item text. Tables use real cells with shared equal-width columns, at least
112 px each, and wrap content inside those columns. Use a quiet header surface
and row dividers. Horizontal overflow belongs inside the reading column, with
one scroll position across virtual chunks and a header only at the table start.

Consecutive completed assistant tools fold into one muted activity summary with
a call count, including runs across provider messages. Prose, reasoning, user
turns and unfinished/failed calls break the group. Disclosure reveals the
original calls as individually virtualized rows; manually opened details remain
visible when a group forms. Keep reading anchors stable as calls finish and
history loads. A replacement summary starts at its top rather than retaining
an offset into a taller call.

Single completed tools use one muted 20 px action row with the command or path
on the same line. Omit the completed badge, retain the full accessible label,
and flatten explicit newlines only in the visual summary. Expanded, running and
failed tools keep a distinct surface. Collapsed failures expose their first
nonempty error line; expanded details retain the complete error and copy actions.
Tool text and diff lists infer their content height inside bounded preview areas.

Running turns use a 24 px status line above the composer, aligned with the
reading column. Keep it mounted through assistant text and tool updates, even
when running metadata arrives before history. The composer owns Stop. Required
actions replace the working line; waiting and failed turns retain their retry
cards. Composer settings may wrap when labels need more space; send keeps its
size, and long workspace paths truncate while preserving the full accessible
label.

Display connection and browser failures through the existing stateful surfaces.
A deferred failure must request a redraw immediately. Use the existing native
dialog APIs only when the interaction calls for them; Zedra's native mobile
sheets are not part of mew's desktop architecture.

## Verification

Check narrow/wide layouts, light/dark themes, long labels, keyboard focus and
Unicode input. Verify behavior with real view/transport tests and native smoke
checks. A screenshot demonstrates layout; it cannot establish callback
ownership, cancellation or render cost.
