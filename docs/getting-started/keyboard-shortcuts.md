---
title: Keyboard Shortcuts
description: All TUI keyboard shortcuts in one place.
---

Press `?` with an empty composer to open the shortcuts overlay.

## Global

| Shortcut | Action |
|----------|--------|
| `Ctrl+Space` | Open command palette |
| `Ctrl+Alt+N` | Open the attention bell's conversation list |
| `?` | Toggle keyboard shortcuts overlay |
| `Ctrl+L` | Scroll to bottom (re-attach auto-scroll) |
| `Ctrl+C` | Cancel stream / clear input / quit (press twice) |
| `Esc` | Cancel stream (press twice) / close modal / clear selection |
| `Ctrl+D` | Delete character forward / quit when input is empty and idle |
| `Ctrl+Home` | Scroll to top |
| `Ctrl+End` | Scroll to bottom |

## Input editing

| Shortcut | Action |
|----------|--------|
| `Enter` | Submit the composer, confirm a dialog, or insert a newline in a todo draft |
| `Shift+Enter` / `Alt+Enter` / `Ctrl+J` | Insert newline in the composer, goal, question answer or todo draft |
| `Ctrl+A` / `Ctrl+E` | Cursor to start / end of line |
| `Ctrl+F` / `Ctrl+B` | Cursor right / left |
| `Ctrl+H` / `Backspace` | Delete the previous character |
| `Ctrl+W` | Kill the previous whitespace-delimited word |
| `Ctrl+K` | Kill to this line's end, or kill its newline when already at the end |
| `Ctrl+U` | Kill to this line's beginning |
| `Ctrl+Y` | Yank the most recently killed text |
| `Alt+Y` | Replace the last yank with an older kill |
| `Ctrl+T` | Transpose characters at the cursor |
| `Ctrl+Z` / `Ctrl+_` | Undo |
| `Ctrl+Shift+Z` | Redo |
| `Ctrl+P` / `Ctrl+N` | Recall previous / next composer history, or navigate picker results |
| `Ctrl+S` | Search forward while history search is open |
| `Alt+B` / `Alt+F`, `Alt+Left` / `Alt+Right` | Move backward / forward by word |
| `Alt+Backspace` / `Alt+D` | Kill the previous / next word, using alphanumeric word boundaries |
| `Alt+<` / `Alt+>` | Move to the beginning / end of the entire field |
| `Up` / `Down` | Navigate between input lines / history |
| `PageUp` / `PageDown` | Scroll chat up / down by 10 lines |
| `Ctrl+R` | Reverse search through input history |

Editing keys work in the composer, reference input, picker filters, question
answers, goals, todo drafts, history search and settings. Each field has its
own undo history; killed text is shared between fields. Consecutive kills
combine into one entry. Moving, typing, pasting or changing fields ends that
sequence and stops `Alt+Y` from replacing unrelated text. Characters with
combining marks and joined emoji move, delete and transpose together.

In conversation pickers, `Ctrl+Alt+A` archives/restores the selected conversation
and `Ctrl+Alt+P` pins it. In the thinking picker, `Ctrl+Alt+P` cycles variants.
Plain `Ctrl+A` and `Ctrl+P/N` retain their editing/navigation meanings.

## Chat

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+C` | Copy selected text to clipboard |
| `Ctrl+Alt+T` | Toggle reasoning/thinking block expansion |
| `Ctrl+O` | Toggle bash output expansion |
| `Ctrl+1` | Toggle the sidebar Environment section (context files / MCP) |
| `x` | Cancel most recently started subagent (when input empty) |
| `@` | Open the reference picker for @-mentions (files, models, skills, subagents) |

## Mouse

| Action | Effect |
|--------|--------|
| Scroll wheel | Scroll the chat, picker list or details under the pointer |
| Click scrollbar track | Page the corresponding view |
| Drag scrollbar thumb | Scroll the corresponding view |
| Click sidebar task, subagent or job | Open its details |
| Click + drag | Select text (when mouse capture is on) |
