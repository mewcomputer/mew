---
title: Keyboard Shortcuts
description: Keyboard and mouse shortcuts for the terminal client.
---

Press `?` with an empty composer to open the shortcuts overlay.

## Global

| Shortcut | Action |
|----------|--------|
| `Ctrl+Space` | Open command palette |
| `Ctrl+Alt+N` | Open the attention bell's conversation list |
| `Ctrl+Alt+O` | Show conversations open in this terminal window |
| `Ctrl+Alt+Left` / `Ctrl+Alt+Right` | Switch between open conversations |
| `Ctrl+Alt+Enter` | Start a conversation in the current project |
| `Ctrl+Alt+W` | Close the current view, or the selected view in a picker |
| `?` | Toggle keyboard shortcuts overlay (when input is empty) |
| `Ctrl+L` | Scroll to bottom and re-enable auto-scroll |
| `Ctrl+C` | Cancel stream (press twice to quit while streaming), clear non-empty input, or quit when idle and empty |
| `Esc` | Cancel stream (press twice), close modal, or clear selection |
| `Ctrl+D` | Delete character forward, or quit when idle and empty |
| `Ctrl+Home` | Scroll to top |
| `Ctrl+End` | Scroll to bottom |

Closing a conversation view retains its draft and queued follow-ups for the
current TUI process while daemon work keeps running. Closing the last view exits
the TUI.

## Input editing

| Shortcut | Action |
|----------|--------|
| `Enter` | Submit the composer, confirm a dialog, or insert a newline in a todo draft |
| `Shift+Enter` / `Alt+Enter` / `Ctrl+J` | Insert a newline in the composer, goal, question answer, or todo draft |
| `Ctrl+A` / `Ctrl+E` | Move cursor to start / end of line |
| `Ctrl+F` / `Ctrl+B` | Move cursor right / left |
| `Ctrl+H` / `Backspace` | Delete previous character |
| `Ctrl+W` | Kill previous whitespace-delimited word |
| `Ctrl+K` | Kill to line end, or kill trailing newline at line end |
| `Ctrl+U` / `Cmd+Backspace` | Kill to line start |
| `Ctrl+Y` | Yank most recently killed text |
| `Alt+Y` | Replace last yank with older kill |
| `Ctrl+T` | Transpose characters at cursor |
| `Ctrl+Z` / `Ctrl+_` | Undo |
| `Ctrl+Shift+Z` | Redo |
| `Ctrl+P` / `Ctrl+N` | Recall previous / next composer history, or navigate picker results |
| `Ctrl+R` | Open reverse history search |
| `Ctrl+S` | Search forward while history search is open |
| `Alt+B` / `Alt+F`, `Alt+Left` / `Alt+Right` | Move backward / forward by word |
| `Alt+Backspace` / `Alt+D` | Kill previous / next word at alphanumeric boundaries |
| `Alt+<` / `Alt+>` | Move to start / end of field |
| `Up` / `Down` | Move between visual lines, browse history, or scroll chat 1 line when input is empty |
| `PageUp` / `PageDown` | Scroll chat up / down by 10 lines |
| `Ctrl+V` / `Ctrl+Shift+V` / `Cmd+V` | Paste an image from the system clipboard |

Editing keys work in the composer, reference filter, picker filters, question
answers, goals, todo drafts, history search, and settings. Each field keeps its
own undo stack while sharing the kill ring. Consecutive kills combine into one
yank entry; moving, typing, pasting, or switching fields breaks the kill
sequence. Combining marks and joined emoji move, delete, and transpose as single
units.

## Pickers

| Shortcut | Scope | Action |
|----------|-------|--------|
| `Ctrl+Alt+A` | Conversation pickers | Archive or restore the selected conversation |
| `Ctrl+Alt+P` | Conversation pickers | Toggle pin on the selected conversation |
| `Tab` | Open-view / attention pickers | Switch to global session history |
| `Tab` | Session history picker | Toggle between active and archived conversations |
| `Right` | Model picker (empty filter) | Open thinking variants for the selected model |
| `Ctrl+Alt+P` | Thinking-variant picker | Cycle through variants for the active model |
| `PageUp` / `PageDown` | Centered pickers | Scroll the details pane |

Inside pickers, plain `Ctrl+A` and `Ctrl+P` / `Ctrl+N` retain their standard
cursor and navigation behavior.

## Chat and queue

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+C` | Copy selected text to clipboard |
| `Ctrl+Alt+T` | Toggle reasoning block expansion |
| `Ctrl+O` | Toggle bash, Python cell, and nested tool output expansion |
| `Ctrl+G` | Toggle tool-call batch and compaction block expansion |
| `Ctrl+1` | Toggle the sidebar Environment section |
| `Shift+Tab` / `Ctrl+Shift+Tab` | Cycle personas forward / backward when input is empty |
| `Ctrl+Up` | Inject the oldest queued message as guidance into the running turn |
| `Up` `Up` | While streaming with queued messages and empty input, cancel the turn and send the oldest queued message |
| `@` | Open the reference picker for files, models, skills, and subagents |

## Mouse

| Action | Effect |
|--------|--------|
| Scroll wheel | Scroll the chat, picker list, or details pane under the pointer |
| Click scrollbar track | Page the corresponding view |
| Drag scrollbar thumb | Scroll the corresponding view |
| Click + drag | Select text and copy it to the clipboard on release |
| Click reasoning, tool batch, or compaction header | Expand or collapse the block |
| Click sidebar header or conversation/attention/task/subagent/job row | Collapse section, switch conversation, or open details |
| Click status bar bell | Open the attention list |
| Click input box | Focus and position the cursor |
