---
title: Session handover
description: Continue a daemon session in another Mew client.
---

Multiple clients can attach to the same session on one daemon. The daemon
owns the history and turn, so opening another client leaves the conversation
in place.

## Open an existing session

Start the [browser bridge](/docs/using-mew/web-ui/) and connect both
clients to its daemon. In the browser, open
`http://localhost:9847/session/<session-id>` to attach directly. To go from
the browser to the terminal, copy the session ID from that URL.

Set `session_id` to the conversation ID before running the command.

```sh
socket="${XDG_RUNTIME_DIR:-/tmp}/mew.sock"
mew chat --connect "ws://unix:$socket" --attach "$session_id"
```

If the bridge uses a custom socket, use that path instead. If the same
daemon also has a local TCP listener, its WebSocket URL works too. In a TUI
already connected to that daemon, run `/resume <session-id>`.

## What happens on attach and detach

- `AttachSession` loads an existing session from disk if needed. The daemon
  sends `SessionReady`, recent history, and pending requests to the new client.
- `ClientAttached` and `ClientDetached` report presence. `YieldControl`
  broadcasts `ControlYielded` as an advisory event. The daemon serializes
  turns even when several authorized clients can send prompts.
- An active turn or shell job keeps the session running after the last client
  leaves. The daemon checks detached sessions every 30 seconds and unloads
  unused runtimes. Saved history remains available for later attachment.

See [Daemon Protocol](/docs/development/dev-protocol/) for wire messages and
[Architecture](/docs/development/dev-architecture/) for session ownership.
