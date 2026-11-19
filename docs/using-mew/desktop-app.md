---
title: Desktop App
description: Build and use mew's macOS desktop clients.
---

Mew has a native macOS app and an experimental Electron app. Both use the
same Rust daemon to run the agent and store conversations. The CLI nightlies
from [Installation](/docs/getting-started/installation/) contain the terminal
client. Build the desktop apps from source.

## Run the native app

Start with a checkout of the [mew repository](https://github.com/mewcomputer/mew).
You'll need the Xcode command-line tools, current stable Rust, Node.js 24 or
later, pnpm, Just, Zig 0.15.2, CMake, Ninja, and the CEF SDK matching
`Cargo.lock`. Packaging also requires the standalone macOS
`agent-browser` binary, version 0.34.0 or later, on your `PATH`.

Set `CEF_PATH` to your SDK directory. On Apple silicon, the repository's CI
helper can download and verify the matching SDK for you:

```sh
export CEF_PATH="$HOME/.cache/mew-cef"
bash scripts/setup-ci-cef.sh "$CEF_PATH"
```

Then run:

```sh
just install-js
just desktop-dev
```

To build a release-profile app, run `just desktop-build` and open
`target/release/bundle/macos/mew.app`. To create a DMG and ZIP for that Mac's
architecture, run `just desktop-package`. Look for the files under
`target/release/dist/`. These local builds are not notarized.

## Start a conversation

Choose **File → New Chat** or press `⌘N`. Use the sessions sidebar to return
to a conversation. Open a workbench tab with `⌘T`, toggle the terminal with
`⌘J`, or search sessions with `⌘K`.

By default the app connects to a healthy local daemon on port `25566` or
starts its bundled daemon. If you already configured a provider for the CLI
under the same account, the local daemon uses that configuration. Otherwise,
see [Configuration](/docs/getting-started/configuration/) to set up a provider.
If your credential lives in a shell environment variable, launch from that
shell so a newly started daemon inherits it.

## Use browser tabs

In a local conversation, open a workbench tab with `⌘T`, enter a URL, and
press Enter. Once the browser page connects, you can ask the agent to inspect
that page. The agent's target follows the browser tab you select for that
conversation. Browser tools in the native app require a local daemon.

## Try Electron

From the same checkout, use `just install-js` followed by
`just desktop-electron-dev`. The Electron app uses the shared Web UI and
bundles its own host. To build it without launching it, run
`just desktop-electron-build`. The bundle is written to
`target/release/bundle/macos/mew-electron.app`.

For an example of an Electron workflow, see
[Guide a running subagent in Electron](/docs/using-mew/subagents/#guide-a-running-subagent-in-electron).
