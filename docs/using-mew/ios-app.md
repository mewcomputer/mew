---
title: iOS App
description: Connect to mew daemons from your iPhone over iroh.
---

mew has an iOS app that connects to daemons on your machines over iroh
(QUIC with holepunching and relays). Watch sessions, respond to
permissions, send prompts — all from your phone.

The app is in early development. It pairs with any number of daemons,
shows per-daemon session rails ordered by recent activity, attaches to
sessions with recent history replay + live streaming, and supports
sending prompts, canceling turns, answering permission and ask-user
requests, and switching models.

## Pairing with a daemon

1. On your daemon machine, start remote access:

   ```sh
   mew daemon --remote
   ```

   In another terminal, run `mew pair`. This prints a QR code containing a
   single-use invite that expires after 120 seconds.

2. On your phone, open the mew app and tap **+**.

3. Scan the QR code, or paste a complete pairing invite into the text field.
   The invite includes the credential needed to authorize your phone.

4. Name the daemon (e.g. "Homelab", "Work laptop").

5. Tap **Add**. The app connects over iroh and the daemon authorizes your
   phone's identity. Later connections use that identity without another invite.

### Pairing formats

The app accepts:

- **`computer.mew.mew://<node_id>?token=<token>`** — the invite in the
  QR code from `mew pair`.
- **`mew001:{"node_id":"…","name":"…","token":"…"}`** — a
  versioned payload with an optional suggested name.
- **Raw Node ID** or **`mew001:<node_id>`** — supported for previously
  authorized devices and legacy daemons. A Node ID alone cannot authorize a
  new device on a daemon running with `--remote`.

Unknown version prefixes (e.g. `mew002:`) are rejected with a clear
error message.

## What you see

### Daemon list

Each daemon shows a status dot (connected / connecting / unreachable),
the daemon's version, and an aggregate needs-you badge summing pending
permissions and questions across all sessions on that daemon.

### Session rail

Per-daemon chats appear newest first, grouped into **Today**, **Yesterday**,
**Previous 7 days**, **Previous 30 days**, and **Older**. Message timestamps
determine recency, with creation time as a fallback. New alerts received while
connected move their chat up immediately. Pending permission/question badges
remain visible without permanently placing older chats above newer ones.

Rows show the workspace and session state. Pull down to refresh, including
from an empty list. **Show archived** remains available when every chat is
archived. Pins mark chats without overriding chronological order.

Swipe actions: pin, archive, delete (with confirmation). Tap **+** to
create a new session — opens a project picker showing recent projects
(derived from previous session cwds and `workspace.roots`) plus a
free-text path input. The daemon validates the chosen cwd before
creating the session, and rejects bad paths with an error.

### Chat view

- **Messages**: streaming markdown rendered via SwiftStreamingMarkdown
  with full theme coverage (headings, lists, code blocks, blockquotes,
  inline styles). Reasoning parts render as markdown too. Consecutive
  same-tool calls batch into a single row. Text streams incrementally
  with de-jittered autoscroll.
- **Older messages**: tap **Load older messages** above the transcript to
  fetch another page. Loading preserves the current reading position and
  the live turn state; connection failures offer a retry without clearing chat.
- **File browser**: tap the **+** button on the chatbar to browse the
  session's working directory. Folders navigate in, files append their
  path to the composer. Defaults to the session cwd (or daemon default).
- **Chatbar**: liquid glass two-row layout — text field on top, controls
  on the bottom. Includes the model picker and real session titles.
- **Working indicator**: appears immediately on send, before the first
  provider event arrives.
- **Composer**: type a prompt and tap send. While a turn is running,
  the send button becomes a cancel (stop) button. Slash commands
  (`/clear`, `/compact`, etc.) are passed through to the daemon.
- **Model picker**: tap the model name in the toolbar to switch models.
  Pulls the available list from the daemon.
- **Permission sheet**: when the agent requests permission to run a
  tool, a sheet appears with the tool name, pretty-printed input, and
  three buttons: Allow Once, Allow Session, Deny. The sheet is
  un-dismissable while pending — you must answer it.
- **Ask-user sheet**: when the agent asks questions, a sheet appears
  with one text field per question. Also un-dismissable while pending.

### Tools and workspace review

Open the chat's **… → Tools** menu to browse tool calls from the loaded
conversation history. Search by tool name, input, output, or error text.
Expand a row to inspect its input and result; rows show the tool's state
and duration when available.

Tap the **Review changes** button in the chat toolbar to load changes from
that session's workspace on its daemon. The file list includes added,
modified, deleted, renamed, untracked, and binary files. Tap a text file to
read line-numbered additions and deletions. Changes are compared with the
latest commit, including staged and unstaged edits.

Use **Refresh** to read the workspace again. Loading, connection failures,
and repository errors have distinct states; a failed read does not appear
as an empty diff. Refresh keeps the previous file list visible, with progress
or a retry message alongside it. Review replies are scoped to their daemon, session, and
request, so a late reply cannot replace a newer review.

### Workers and background jobs

Open **… → Workers & jobs** to see active workers and background commands.
Workers show their current status and tool. Tap a worker to read its
transcript without leaving the parent conversation, then use **Refresh**
to fetch newer messages. Completed workers leave the active list; an open
transcript remains available.
Refreshing a transcript preserves its messages, including when the read fails.

Worker history currently loads as a single transcript. Transcripts larger
than 8 MiB return a size-limit error; they do not disconnect the conversation.

Tap a background job to read its captured output. **Load more output**
loads another chunk, and **Reload output** starts again from the beginning.
Connection errors preserve output already read. If the daemon could not
capture all output, the detail view says so. **End of output** marks a
completed read, including jobs with no captured output.

### Settings

**App settings** contains the chat font, this phone's copyable connection
identity, saved-machine links, and app information. Use the complete invite
from `mew pair` when pairing, including its authorization credential.

Appearance follows iOS by default, with light and dark overrides. Haptic
feedback acknowledges accepted messages (including retries), completed turns,
and requests or errors. Foreground alert preferences control completion,
turn/delivery errors, and requests, with individual machine mutes. These affect
banners and alert haptics on this phone while the app is open. Requests and
conversation activity remain visible when alerts are muted; direct action
errors still appear. Background events do not replay banners on reopening.

Review readability controls line wrapping, line numbers, and diff text size,
with a live preview. Diff text also follows iOS text size; these controls do
not change conversation text. About contains a shareable connection diagnostic
report with versions and connection states. The report excludes names, Node
IDs, credentials, raw errors, paths, and conversation content.

Tap the machine name in a chat or conversation list to open the switcher.
It shows connection state, the selected machine, and pending requests. Tap
the information button for that machine's details:

- **Nickname** changes the saved name on this phone.
- **Connection** offers Retry connection or Pair again when needed.
- **Auto-title** affects prompts sent through this phone's current connection.
- **Auto-summary** affects every session and client on the daemon.
- **New chat defaults** reads the daemon's saved model, provider, persona,
  thinking and permission policy. Use the open chat's settings as defaults,
  or reset to the daemon's startup configuration. This affects new chats from
  every client; existing chats retain their own settings.
- **Paired devices** lists authorized devices and their access level. Open a
  device to revoke it, including this phone. Revocation disconnects the device
  and requires a new pairing invite; conversations remain on the machine.
- **Advanced** contains the daemon version and copyable Node ID.

Automation controls display values confirmed by the daemon. Update the daemon
to use these controls and scoped chat-opening errors; older versions show
settings as unavailable rather than assuming they are off. New-chat defaults
and paired-device controls require an updated daemon and full-control access.
They show confirmed results, retain prior values on failure and expire requests
when connections change. Some token-only remote endpoints cannot list devices;
the app explicitly reports that limitation. Removing a saved
connection disconnects this phone and removes its local entry. It keeps the
daemon's conversations and does not revoke the phone's authorization.

The app reopens the last explicitly selected machine and its working chat or
list. An offline machine stays selected while it reconnects. A missing chat
falls back to its machine's list; a removed machine falls back to the picker.
With no saved choice, one machine opens directly and several show the picker.
Each chat retains its own draft while switching within the app.

## Connection lifecycle

The app handles reconnection automatically:

- On unexpected disconnect, the app retries with exponential backoff
  (1s, 2s, 4s, up to 30s) with jitter.
- After reconnect, the app re-sends `AttachSession` so the daemon
  replays the full message history. The UI swaps state in one
  operation — no delta storm.
- If the agent is blocked on a permission or ask-user request when the
  app attaches, the daemon replays those pending requests too (they
  aren't in the message history). The app deduplicates by request ID so
  a re-attach doesn't double up sheets.
- When the app returns to the foreground after being backgrounded, iOS
  kills QUIC connections within ~30 seconds. The app redials and
  re-attaches automatically.

Multiple devices can attach to the same session. Permission and
ask-user prompts go to all connected clients. Any client can respond,
and `RequestResolved` dismisses the prompt on all devices.

## Limitations (v1)

- **Foreground only**: the app does not receive push notifications
  while backgrounded. When the app is open, in-app alerts fire for
  sessions that need your attention. Background push is planned for a
  future release via a push relay.
- **No prompt attachments**: sending a photo from the phone has no
  protocol support yet.
- **No syntax highlighting in code blocks**: markdown renders via
  SwiftStreamingMarkdown, which does not theme code blocks with
  language-specific highlighting (unlike the web UI's Shiki).

## Architecture

```
┌────────────────────────────────────────────┐
│ SwiftUI app (mew-ios)                      │
│   daemon list · session rail · chat ·      │
│   permission/ask sheets · settings        │
├────────────────────────────────────────────┤
│ mew-mobile-core (Rust, UniFFI)             │
│   iroh endpoint + phone identity           │
│   per-daemon connections + reconnect       │
│   WS framing + mew-protocol codec          │
│   session state assembly (parts→messages)  │
├────────────────────────────────────────────┤
│ iroh (QUIC, holepunching, relays)          │
└────────────────────────────────────────────┘
```

All protocol knowledge lives in Rust. Swift never sees wire JSON or
provider event deltas — it receives typed, app-shaped events
(`CoreEvent`) through a `CoreListener` callback. This mirrors the web
UI's layering (`mew-web-client` + Zustand store) in a single crate.

See [Mobile Core Development](/docs/development/dev-mobile/) for the
developer guide.
