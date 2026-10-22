---
title: iOS App
description: Connect to mew daemons from your iPhone over iroh.
---

mew has an iOS app that connects to daemons on your machines over iroh
(QUIC with holepunching and relays). Watch sessions, respond to
permissions, send prompts — all from your phone.

The app is in early development. It pairs with any number of daemons,
shows per-daemon session rails with needs-you ordering, attaches to
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

Per-daemon session list, sorted by attention:

1. **Needs attention** (amber) — sessions with pending permissions or
   questions
2. **Running** (blue) — sessions with an active turn
3. **Active** (green) — sessions with recent activity
4. **Idle** (gray) — sessions waiting for input

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
as an empty diff. Review replies are scoped to their daemon, session, and
request, so a late reply cannot replace a newer review.

### Settings

- **Your Node ID** — copyable. This is what daemons need to allowlist.
- **Daemon management** — rename or remove paired daemons.
- **About** — app version.

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

- **Older history**: the app displays the recent history supplied on attach;
  loading earlier pages is not yet exposed in the iOS UI.
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
