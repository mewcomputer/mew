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
Chat rows keep a fixed 30 px height across activity states and working folders.
Let layout truncate the normalized full title, with age at the right. Reserve a
44 px trailing slot so pin/archive actions replace the age on hover or keyboard
focus without moving the title. Those actions preserve the current selection.
Right-click or Shift-F10 opens rename, pin, archive and group actions; Escape and
outside clicks dismiss the menu. Full titles, working directories, workspace and
status remain available through accessible labels and a hover preview. Read
failures stay quiet in the list; unread failures and current requests retain
distinct markers. Selecting a chat reveals its workspace, and selecting the
already-attached chat preserves its loaded content. Closing a chat returns to
the unbound composer while retaining its draft and view state for later use.

Inbox is a permanent sidebar entry with a live count of nonarchived requests and
unread finished results. Its flat list puts requests first, then recent results,
independent of workspace collapse, pins and recent-chat limits. Reading uses the
shared client's acknowledgement state; opening a request does not answer it.
Search also matches workspace identity. Keep a clear empty state and retain the
workspace collapse state when leaving Inbox. The Archived entry opens archived
chats with the same row actions for restoring them. Starting a chat returns
either mode to the workspace list.

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
cards. Composer settings share one 28 px toolbar row, with 11 px labels and
16 px leading. Setting controls stay 26 px tall and Send/Stop retain their 28 px
targets. Names truncate inside bounded controls rather than wrapping the toolbar;
retain complete accessible labels and measured picker anchors. Keep the model
control at its natural width beside Send, without stretching a short name into
an empty button. Effort has a separate fixed-width suffix so only the model name
truncates. Below 480 px of conversation space, replace the persona label with a
26 px icon control that opens the same picker and retains its full accessible
label. Follow the measured composer width through pane animations and resizing.
Wider panes show the selected persona text. Unset values use concise Model/Persona placeholders.
Composer footer labels are also 11 px; draft text remains 15 px with 24 px leading.
Long workspace paths truncate while preserving the full accessible label.
Use provider-scoped catalog model names in the composer and picker, falling back
to the raw model ID when no name is supplied. Search matches names and IDs;
picker details retain the fully qualified identity used for switching models.

Attachment chips wrap inside the reading column. Truncate the filename while
keeping its named 24 px remove button visible, tabbable and independently
clickable. Removing a file preserves the draft and returns focus to the editor.

Empty or whitespace-only drafts and pending model switches show a disabled Send
surface, without click or Tab activation. Running questions expose Send answer
or Send answers; ordinary running turns retain Stop. Pointer submission and
Enter use the same question ownership and submission path.

The model picker uses one row inventory and popup size for layout and placement,
including Recent entries and headers. Constrain its 400 px width to the window
and its height to space above or below the trigger. Keep search and effort fixed
while the model list scrolls. Picker trigger measurements request their redraw
after painting so open menus follow resizing.

Display connection and browser failures through the existing stateful surfaces.
A deferred failure must request a redraw immediately. Use the existing native
dialog APIs only when the interaction calls for them; Zedra's native mobile
sheets are not part of mew's desktop architecture.

## Verification

Check narrow/wide layouts, light/dark themes, long labels, keyboard focus and
Unicode input. Verify behavior with real view/transport tests and native smoke
checks. A screenshot demonstrates layout; it cannot establish callback
ownership, cancellation or render cost.
