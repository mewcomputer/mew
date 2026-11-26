---
title: Desktop App
description: Build and use Mew's macOS desktop clients.
---

Mew includes a native macOS app and an experimental Electron app. Both connect to
the same Rust daemon to run turns and store conversations. CLI nightlies from
[Installation](/docs/getting-started/installation/) include only the terminal
client; build the desktop apps from source.

## Run the native app

Clone the [Mew repository](https://github.com/mewcomputer/mew). Install the
Xcode command-line tools, current stable Rust, Node.js 24 or later, pnpm, Just,
Zig 0.15.2, CMake, Ninja, the CEF SDK matching `Cargo.lock`, and the standalone
macOS `agent-browser` binary (version 0.34.0 or later) on your `PATH`.

Set `CEF_PATH` to your SDK directory. On Apple silicon, the CI helper downloads
and verifies the matching SDK:

```sh
export CEF_PATH="$HOME/.cache/mew-cef"
bash scripts/setup-ci-cef.sh "$CEF_PATH"
```

Then run:

```sh
just install-js
just desktop-dev
```

Run `just desktop-build` to build a release bundle at
`target/release/bundle/macos/mew.app`, or `just desktop-package` to write a DMG
and ZIP under `target/release/dist/`. Local builds are ad hoc signed and not
notarized.

## Start a conversation

Choose **File → New Chat** or press `⌘N`. Switch conversations from the
sessions sidebar. Press `⌘T` to open a workbench tab, `⌘J` to toggle the
terminal, or `⌘K` to search sessions.

By default, the app attaches to a healthy local daemon on port `25566` or spawns
its bundled daemon. Existing CLI provider configurations apply automatically.
Otherwise, see [Configuration](/docs/getting-started/configuration/) to set up a
provider. When using credentials in shell environment variables, launch the app
from that shell so a newly started daemon inherits them.

## Use browser tabs

In a local conversation, press `⌘T` to open a workbench tab, enter a URL, and
press Enter. Once the page loads, ask the agent to inspect or interact with it.
Browser tool calls target whichever browser tab is active for that conversation
and require a local daemon.

## Try Electron

From the repository checkout, run `just install-js` followed by
`just desktop-electron-dev`. The Electron shell uses the shared React Web UI and
bundles `mew` (running `mew desktop-host`) along with `agent-browser` (version
0.34.0 or later on `PATH` is required during packaging).

Run `just desktop-electron-build` to build without launching. The packager
writes the bundle to `target/release/bundle/macos/mew-electron.app`.

### Remote access

In **Settings → Remote & Mobile**, **Allow iOS & remote connections** starts the
iroh listener and is remembered by the daemon. It starts again when the Electron
app or daemon restarts; switch it off to stop future launches. Pairing a new
device still needs a fresh single-use invite from the same panel.

See [Guide a running subagent](/docs/using-mew/subagents/#guide-a-running-subagent)
for steering child agents from the Web UI or Electron.
