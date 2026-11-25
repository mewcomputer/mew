# Electron desktop UI milestone

Opt-in macOS shell for the shared React, Tailwind and shadcn interface. The Rust
daemon, agent runtime, session formats and theme loader remain authoritative.
The native desktop remains available and stays the default while the remaining
[baseline checklist](../../notes/chat-ui-baseline-checklist.md) is completed.

## Run and package

Install dependencies after checking out the workspace or changing its lockfile:

```sh
just install-js
just desktop-electron-dev
```

The development recipe builds the client, UI and Rust executable, then packages
and runs `target/debug/bundle/macos/mew-electron.app`. No Vite server is needed.
`just desktop-electron-build` produces the release-profile bundle at
`target/release/bundle/macos/mew-electron.app`. Local bundles are ad hoc signed;
release distribution and notarization are outside this milestone.

`just desktop-electron-install` builds the release bundle and installs it as
`/Applications/mew.app`, replacing the app at that path. Close the installed
app before running the command so its daemon restarts with the new binary.

Packaging requires the pinned Electron dependency and a standalone macOS
`agent-browser` executable, version 0.34.0 or newer, on PATH. The default output
can replace an inactive bundle created by this packager. Close it before
rebuilding. An explicit `--output` always requires a fresh destination:

```sh
node apps/mew-electron/package.cjs --profile debug --output /tmp/mew-review.app
```

## Ownership and browser composition

`mew desktop-host` serves embedded assets and a bounded JSONL bootstrap/theme
binding. It reuses `mew-desktop-supervisor` to attach to a healthy local daemon
or own a daemon for the app lifetime. An explicit `MEW_DESKTOP_DAEMON_URL` is
attach-only. Theme inventory and CSS come from the existing Rust loader,
including installed themes; settings are saved through existing state storage.
The Rust host issues a private loopback WebSocket URL for the `mew://app`
renderer. Use a support binary that advertises this relay when setting
`MEW_DESKTOP_DAEMON_BINARY`.

Electron owns the window, menus and DOM browser guests. The trusted app uses
`mew://app/` and its own persistent Session. External guests share a separate
browser Session, receive no preload or Node access, and cannot contact the
asset, daemon, CDP or focus-control ports through HTTP or WebSocket aliases.
Host IPC accepts only the trusted main frame. Permissions, popups and downloads
are denied in this milestone.

Browser guests cannot open while the attached daemon uses port 80 or 443. The
host protects daemon ports across hostname aliases, which would otherwise block
normal web traffic on those ports. The browser panel reports this limitation;
chat remains available. Use a different daemon port to enable browser tabs.

Guests are created on first visit and retained across tab/chat switches and
pane transitions. Closing a tab destroys its guest. App restarts restore tab
URLs and layout; they reload documents rather than serialize live DOM state.
Registration waits for the exact session confirmation and page-generation ACK.

Electron CDP keyboard input can follow the embedder's previously focused
widget despite an exact target ID. Before every Fill or Press, the daemon calls
the page's private loopback focus endpoint. The host transfers focus through
the outer document into that exact guest while preserving selection. Hidden,
inactive, stale and closed pages fail the preflight. The client requires the
daemon's `browser_focus_preflight` Pong capability; update an older daemon to
enable browser automation. Existing CEF clients omit the callback.
The host leaves independently owned daemons running; restart an older attached
daemon with the rebuilt binary when updating.

## Verification recorded on 2026-10-27

```sh
pnpm --filter @mew/electron test
MEW_ELECTRON_NATIVE_PACKAGE_TESTS=1 node --test apps/mew-electron/package.test.cjs
pnpm --filter @mew/web-client test
pnpm --filter mew-web-ui test
cargo test -p mew --test desktop_host
```

The Node suite passed 29 tests; its real-package test is opt-in and separately
passed with all nine packaging tests. The two real Electron scenarios exercise
guest isolation, exact identity, lifecycle races and real `agent-browser` input
against two same-URL guests after chat takes focus. Unicode Fill and
selection-preserving Backspace affect only the selected guest. Negative focus
requests leave chat and the other guest unchanged.

The client passed 40 tests and UI passed 212. Both production builds passed.
Five actual Rust support-process tests cover themes, malformed requests,
startup failures and owned/attached shutdown. Protocol tests (126), client-core
(45), client-local (4), browser tests (19), daemon browser tests (4), and the
daemon Ping advertisement test passed. Existing opt-in Chrome tests were not
run. Affected all-targets clippy, workspace clippy through the enabled commit
hook, formatting, architecture and generated-theme checks passed.

The packaged debug app completed real WebSocket turns with the deterministic
fake provider beside two guests. Browser tools filled distinct Unicode values
through the daemon and bundled CLI without changing the other guest or chat
draft. Menus stayed above the live page; pane close/reopen, tab retention,
keyboard resizing, browser scrolling, light/dark themes and persisted theme
choices were checked. Closing the app reaped its children and left the attached
daemon alive. Pointer dragging was unavailable through the native automation
API in this run; this is not recorded as a passing packaged drag test.

Photos `22-electron-final-browser-tools-light.png` through
`25-electron-final-chat-browser-dark.png` and
`26-electron-resource-spot.json` are in the task's visualization artifacts.
One spot sample totaled 627.1 MiB RSS across eight app/support/daemon/CLI
processes with two visited guests and the visible animation paused. RSS counts
shared mappings, this is a debug build, and there is no matched native baseline;
the sample does not establish extra physical RAM cost or a resource budget.

This milestone does not establish complete replacement parity. Scoped prompt
receipts/reconciliation, direct subagent messaging, stop-and-archive of
descendants, remote profiles, full PTY/workbench support, preference import,
native IME and sustained accessibility/resource testing remain acceptance work.
