---
title: iOS App
description: Connect to Mew daemons from your iPhone over iroh.
---

Mew's experimental iOS app connects to daemons over iroh using QUIC with hole
punching and relays. It requires iOS 26 or later. See
[Mobile Core Development](/docs/development/dev-mobile/) for build instructions.

## Pairing with a daemon

Remote pairing requires a source build of the CLI with the `iroh` feature.
Install the [source build prerequisites](/docs/getting-started/installation/)
first, then build `mew`:

```sh
pnpm install --frozen-lockfile
just build-ui
cargo build --locked -p mew --features iroh
```

Run these commands with `./target/debug/mew`:

1. Start remote access on the daemon machine:

   ```sh
   ./target/debug/mew daemon --remote
   ```

   In another terminal, run `./target/debug/mew pair` to print a QR code
   containing a single-use invite that expires after 120 seconds.

2. On your iPhone, open **Machines** from the Chats toolbar and tap **+**.

3. Tap **Scan QR Code** and scan the QR code, or paste the invite string into
   the pairing field. A Node ID alone is insufficient for a new device because
   the invite includes the one-time authorization token.

4. Optionally give the daemon a display name such as `Homelab` or `Work laptop`.

5. Tap **Add** and wait for **Connected** in the **Machines** list. The daemon
   authorizes the phone's identity and consumes the invite.

:::caution[Current limits with remote pairing]
After invite-based pairing, relaunching the app can stick on **Reconnecting**
because it resends the consumed invite. Remove the machine from **Machines** and
add it again with the daemon's Node ID. Local removal keeps the daemon's device
authorization intact and restores the connection.

Starting a chat from iOS still fails with
`remote connections must identify as client_kind=remote` in the current build.
:::

### Pairing formats

The app accepts:

- `computer.mew.mew://<node_id>?token=<token>`: QR invite printed by `mew pair`.
- `mew001:{"node_id":"...","name":"...","token":"..."}`: versioned invite JSON
  with an optional display name.
- Raw Node ID or `mew001:<node_id>`: reconnects an already authorized device or
  legacy daemon. Node IDs alone cannot authorize a new device on `mew daemon --remote`.

Unrecognized prefixes fall through to Node ID parsing and fail with an invalid
Node ID error.

## App screens

### Machines

Open **Machines** from the Chats toolbar to manage saved daemons. Each row
shows connection status and an aggregate needs-you badge counting pending
permissions and questions across its chats. Opening a machine displays the
daemon version, connection controls, and daemon settings.

### Chats

The home screen merges conversations from all saved machines into one timeline
under **All**, with per-machine filters that preserve your selection when
returning from a chat. Conversations are sorted newest first (by latest message
timestamp, then creation time) under **Today**, **Yesterday**, **Previous 7
days**, **Previous 30 days**, and **Older**.

- **Refresh and offline views**: pull down to refresh, including when the list
  is empty; returning from a chat also refreshes visible machine metadata. **Show
  archived** remains available per filter even when all chats are archived.
  Disconnected machines keep cached chats visible with **Offline** badges and a
  retry action.
- **Row actions**: swipe rows to pin, archive, rename, or delete (with
  confirmation). Pins mark chats without changing chronological sort order.
- **Creating a chat**: tap **New chat** in the toolbar. Under **All**, pick a
  machine first; under a machine filter, that machine is preselected. The
  workspace picker lists recent chat workspaces and `workspace.roots` alongside
  a manual path input, validated by the daemon.

### Chat view

- **Messages and tool batches**: streaming Markdown renders with theme support
  for headings, lists, code blocks, blockquotes, inline formatting, and
  reasoning blocks. Consecutive calls to the same tool batch into one row, and
  text streams with smooth autoscroll.
- **History pagination**: tap **Load older messages** above the transcript to
  load earlier pages while preserving scroll position and live turns. Failed
  loads offer a retry without clearing the visible transcript.
- **File browser**: tap **+** on the chatbar to browse the session's working
  directory and append file paths to the composer.
- **Chatbar and model picker**: the two-row liquid glass chatbar places the text
  input above controls for the model picker, thinking variant, permission mode,
  and session usage.
- **Composer and alerts**: a working indicator appears on send before the first
  provider event arrives, the send button becomes a cancel button while active,
  and slash commands (`/clear`, `/compact`, etc.) forward to the daemon.
- **Permission and ask-user sheets**: pending tool approvals (with **Allow
  Once**, **Allow Session**, and **Deny**) and question forms stay open and
  un-dismissable until answered.

### Tools and workspace review

Open **… → Tools** in a chat to search loaded tool calls by tool name, input,
output, or error and expand rows for full arguments, output, and duration.

Tap **Review changes** in the chat toolbar to diff the session's workspace
against `HEAD` (including staged, unstaged, and untracked files across added,
modified, deleted, renamed, untracked, and binary categories).

- Tapping a text file opens line-numbered additions and deletions.
- **Refresh** preserves the previous file list while reloading and separates
  connection or git errors from an empty repository diff.
- Review replies carry daemon, session, and request IDs so stale replies do not
  overwrite newer results.

### Workers and background jobs

Open **… → Workers & jobs** to inspect active subagents and background shell
commands.

- **Workers**: tap a worker to read its transcript without leaving the parent
  chat, then tap **Refresh** for newer messages (existing messages stay visible
  if a refresh fails). Transcripts exceeding 8 MiB return a size-limit error
  without disconnecting the chat.
- **Background jobs**: tap a job to read its captured output. **Load more
  output** fetches another chunk, **Reload output** restarts from byte zero, and
  **End of output** marks the end of stream.

### Settings

**App settings** configures the chat font, appearance overrides (system, light,
or dark), and review formatting (line wrapping, line numbers, and diff text
size), and displays this phone's copyable Node ID, saved machine links, and
diagnostics. The shareable connection report in **About** includes versions and
connection states while stripping names, Node IDs, credentials, raw errors,
paths, and messages.

Haptic feedback fires on accepted prompts, completed turns, permission/question
requests, and errors. Foreground alert preferences let you mute completion,
error, or request notifications per machine while keeping pending sheets and
direct action errors visible.

Tap the machine name inside a chat to open the switcher and tap its info button
for per-machine settings:

- **Nickname**: update the display label saved on this phone.
- **Connection**: trigger **Retry connection** or **Pair again**.
- **Auto-title and auto-summary**: toggle session title generation (current
  connection) and summaries (daemon-wide).
- **New chat defaults**: read and update the daemon's default provider, model,
  persona, thinking variant, and permission policy for future chats across
  clients.
- **Paired devices**: inspect authorized devices and revoke access (including
  this phone), which disconnects the device while preserving daemon chats.
- **Advanced**: view the daemon version and copy its Node ID.

Removing a saved machine deletes its local entry and disconnects the phone while
leaving daemon conversations and authorization intact. Each chat preserves its
draft when switching views, and the app restores both the last open chat and
selected machine filter separately.

## Connection lifecycle

- Unexpected disconnects retry with exponential backoff (1s, 2s, 4s, up to 30s)
  and jitter.
- On reconnect, the app reattaches to the session, fetches the newest history
  page, and replays any pending permission or question prompts deduplicated by
  request ID.
- Returning from iOS background reconnects and reattaches automatically.
- Any authorized attached client can resolve a pending permission or question,
  and the daemon dismisses the sheet on other clients.

## Current limitations

- **Foreground only**: iOS receives alerts only while the app is open; push
  notifications are unimplemented.
- **No phone photo uploads**: `Prompt` attachments accept only daemon-local
  paths on the wire, so iOS cannot upload images from the phone.
- **Plain code blocks**: SwiftStreamingMarkdown renders code blocks without
  language-specific syntax highlighting.

## Architecture

```text
┌────────────────────────────────────────────┐
│ SwiftUI app (mew-ios)                      │
│   daemon list · session rail · chat ·      │
│   permission/ask sheets · settings        │
├────────────────────────────────────────────┤
│ mew-mobile-core (Rust, UniFFI)             │
│   iroh endpoint + phone identity           │
│   per-daemon connections + reconnect       │
│   WS framing + mew-protocol codec          │
│   session state assembly (parts -> msgs)   │
├────────────────────────────────────────────┤
│ iroh (QUIC, hole punching, relays)         │
└────────────────────────────────────────────┘
```

Protocol parsing and state assembly live in Rust; `mew-mobile-core` delivers
typed `CoreEvent` values to Swift through `CoreListener`. See
[Mobile Core Development](/docs/development/dev-mobile/) for internals.
