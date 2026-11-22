---
title: Session handover
description: Continue a daemon session in another Mew client.
---

Multiple clients can attach to the same daemon session simultaneously because
the daemon stores conversation history and policy and runs each turn.

## Open an existing session

- **From any TUI already attached to the same daemon**: run
  `/resume <session-id>`.
- **From a local desktop app or another TUI**: attach the TUI to the daemon's
  WebSocket endpoint (`ws://unix:$socket` for the web bridge socket, or
  `ws://127.0.0.1:25566` for the native desktop daemon):

  ```sh
  socket="${XDG_RUNTIME_DIR:-/tmp}/mew.sock"
  mew chat --connect "ws://unix:$socket" --attach "$session_id"
  ```

- **From the browser**: start `mew-web` (see [Web UI](/docs/using-mew/web-ui/))
  and open `http://localhost:9847/session/<session-id>`.
- **From iOS or a remote profile**: connect to the daemon started with
  `mew daemon --remote` and select the session from the combined chat list.

Run `/yield` in the TUI to emit an advisory `YieldControl` signal so other
attached clients can take over cleanly.

## Attach and detach lifecycle

- `AttachSession` loads an idle session from disk if needed; the daemon sends
  `SessionReady`, `SessionHistoryPage`, and any pending permission or question
  requests to the new client.
- `ClientAttached`, `ClientDetached`, and `ControlYielded` (`YieldControl`)
  broadcast client presence to attached peers while `turn_lock` serializes
  turns across all clients.
- Detached sessions with active turns or background shell jobs continue
  running; the daemon checks detached sessions every 30 seconds and unloads
  idle runtimes while leaving transcripts on disk.

See [Daemon Protocol](/docs/development/dev-protocol/) for wire messages and
[Architecture](/docs/development/dev-architecture/) for session ownership.
