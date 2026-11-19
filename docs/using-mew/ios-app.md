---
title: iOS App
description: Connect to mew daemons from your iPhone over iroh.
---

mew has an iOS app that connects to daemons on your machines over iroh
(QUIC with holepunching and relays). Watch sessions, respond to
permissions, send prompts — all from your phone.

The app is in early development. It pairs with any number of daemons,
shows a combined chat list with machine filters ordered by recent activity, attaches to
sessions with recent history replay + live streaming, and supports
sending prompts, canceling turns, answering permission and ask-user
requests, and switching models.

The app requires iOS 26 or later. For source build instructions, see
[Mobile Core Development](/docs/development/dev-mobile/).

## Pairing with a daemon

Remote pairing requires a source build of the CLI with the `iroh` feature.
Follow the [source build prerequisites](/docs/getting-started/installation/)
before running this build.

```sh
pnpm install --frozen-lockfile
just build-ui
cargo build --locked -p mew --features iroh
```

Use `./target/debug/mew` from that checkout for each command below.

1. On your daemon machine, start remote access:

   ```sh
   ./target/debug/mew daemon --remote
   ```

   In another terminal, run `./target/debug/mew pair`. This prints a QR code
   containing a single-use invite that expires after 120 seconds.

2. On your phone, open **Machines** from the Chats toolbar, then tap **+**.

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

### Machines

Open **Machines** in the Chats toolbar to manage saved connections. Each machine
shows its connection state and an aggregate needs-you badge summing pending
permissions and questions across its chats. Machine details include the daemon
version, connection controls and daemon settings.

### Chats

The home screen combines conversations from all saved machines. **All** shows
one chronological timeline; the machine filters narrow it without opening a
chat. Each row identifies its machine and workspace. The list keeps the chosen
filter independently of the open chat, including after returning from it.

Chats appear newest first, grouped into **Today**, **Yesterday**, **Previous 7
days**, **Previous 30 days**, and **Older**. Message timestamps determine
recency, with creation time as a fallback. New alerts move their chat up
immediately. Pending permission/question badges remain visible without
permanently placing older chats above newer ones. Pins mark chats without
overriding chronological order.

Pull down to refresh, including from an empty list. Returning from a chat also
refreshes the visible machines' metadata, so newly created chats appear without
a manual refresh. **Show archived** remains available when every chat is
archived and follows the selected machine filter.

Previously loaded chats remain visible when a machine disconnects, with explicit
**Offline** labels. Its filter offers connection details and retry; mutations
and new chats require a connection. The combined list can still show live chats
from other machines.

Swipe actions pin, archive, rename and delete (with confirmation) on the row's
own machine. Tap **New chat** in the toolbar to create a conversation. From
**All**, choose the machine first; a machine filter uses that machine directly.
The project picker shows recent projects from that machine's previous chat
workspaces and `workspace.roots`, plus a path input. The selected machine is
visible above the picker, with an option to change it. The daemon validates the
chosen workspace and rejects unavailable paths with an error.

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

Tap the machine name in a chat to open the switcher.
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

The app restores the last open chat and the list's machine filter separately.
With no saved location it opens **All**. An offline chat remains selected while
its machine reconnects. A missing chat returns to the filtered list; a removed
filter falls back to **All**. Each chat retains its own draft while switching
within the app. Choosing a machine in the chat's switcher restores that
machine's remembered chat, with its filter selected behind it.

## Connection lifecycle

The app handles reconnection automatically:

- On unexpected disconnect, the app retries with exponential backoff
  (1s, 2s, 4s, up to 30s) with jitter.
- After reconnect, the app attaches to the same session and loads its
  newest history page. Older messages can be loaded on demand.
- If the agent is blocked on a permission or ask-user request when the
  app attaches, the daemon replays those pending requests too (they
  aren't in the message history). The app deduplicates by request ID so
  a re-attach doesn't double up sheets.
- If iOS closes the connection while the app is in the background, the app
  reconnects and reattaches when it returns to the foreground.

Multiple devices can attach to the same session. A client with permission
to respond can resolve a pending question or tool approval. The daemon then
dismisses the prompt on the other attached clients.

## Limitations (v1)

- **Foreground only**: the app does not receive push notifications
  while backgrounded. When the app is open, in-app alerts fire for
  sessions that need your attention.
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
