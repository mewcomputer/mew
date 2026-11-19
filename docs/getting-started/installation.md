---
title: Installation
description: Install mew on macOS and Linux.
---

Mew is an agent harness that runs locally and sends model requests to your
configured provider.

## macOS and Linux

The commands below install the CLI and its terminal UI. Nightly builds are
available for Intel and Apple silicon on macOS, and x86-64 and ARM64 on
Linux. The Linux builds target glibc 2.39 and use the OpenSSL 3 and zlib
runtime libraries.

For the macOS desktop clients, see the [Desktop App](/docs/using-mew/desktop-app/) guide.

### Homebrew

```sh
brew tap mewcomputer/tap
brew install mewcomputer/tap/mew
```

Upgrade later with:

```sh
brew upgrade mewcomputer/tap/mew
```

The [tap](https://github.com/mewcomputer/homebrew-tap) checks for successful
nightly builds twice an hour.

### Install script

```sh
curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | bash -s -- --nightly
```

The script detects your platform, downloads the latest nightly from GitHub,
verifies the SHA256 checksum, and installs `mew` to `~/.local/bin` or
`~/bin`. Re-run the command to upgrade. The stable channel becomes available
when a stable release is published.

Environment variables:

| Variable | Description |
|----------|-------------|
| `MEW_CHANNEL` | `stable` or `nightly` (default: `stable`) |
| `MEW_VERSION` | Install a specific stable release, e.g. `v0.2.0` (default: `latest`) |
| `MEW_INSTALL_DIR` | Override the install directory |
| `MEW_DRY_RUN` | Set to `1` to preview without installing |
| `MEW_VERBOSE` | Set to `1` for extra logging |

### Manual download

Download the archive for your platform and the accompanying `SHA256SUMS`
file from the [nightly release](https://github.com/mewcomputer/mew/releases/tag/nightly).
Run `shasum -a 256 mew-nightly-<target>.tar.gz` on macOS or
`sha256sum mew-nightly-<target>.tar.gz` on Linux and compare the result with
the corresponding line in `SHA256SUMS`. Extract the archive, then move
`bin/mew` to a directory on your `PATH`.

## Build from source

### Prerequisites

- Current stable Rust and Cargo
- Zig 0.15.2 for Ghostty terminal emulation
- On Linux, OpenSSL development headers, `pkg-config`, and libc++ development headers
- A terminal that supports 256 colors and UTF-8

### Clone and build the CLI

```sh
git clone https://github.com/mewcomputer/mew.git
cd mew
cargo install --locked --path crates/mew
```

Cargo installs the binary under `~/.cargo/bin` by default. Make sure that
directory is on your `PATH`. For iroh remote pairing, add `--features iroh`
to the Cargo command. The distributed CLI builds omit that feature.

To include the desktop host's embedded web UI in the binary, install Node.js
24 or later, pnpm 10.33.0, and `just`. From the checkout, run
`pnpm install --frozen-lockfile` and `just build-ui` before the Cargo command.

## Verify the installation

```sh
mew --version
```

You should see a version string. If `mew` isn't found, check that your
install directory (`~/.local/bin`, `~/bin`, `~/.cargo/bin`, or `/usr/local/bin`)
is on your `PATH`.

## What gets installed

| Binary | Source crate | Purpose |
|--------|-------------|---------|
| `mew` | `crates/mew` | The main binary. Runs the TUI, one-shot prompts, and the daemon. |
| `mew-web` | `crates/mew-web-bridge` | Web UI bridge. Serves the browser-based chat interface. Build separately with `just build-web`. |

See the [Web UI](/docs/using-mew/web-ui/) guide for running the browser
interface.

## Subcommands

| Command | Description |
|---------|-------------|
| `mew` | Start an interactive TUI session (default) |
| `mew run "prompt"` | Run a single prompt non-interactively |
| `mew chat` | Start interactive chat with explicit options such as `--provider` or `--connect` |
| `mew daemon` | Start the WebSocket daemon. Builds with iroh support also accept `--remote` for authenticated pairing. |
| `mew pair` | Generate a QR code for pairing. Requires a source build with `--features iroh`. |
| `mew config show` | Print current configuration |
| `mew config edit` | Open the config file in your editor |
| `mew debug` | Debug tools (permission simulator, VFS inspector) |

## Next steps

- [Quick Start](/docs/getting-started/quick-start/): send your first prompt
- [Configuration](/docs/getting-started/configuration/): set up providers and credentials
- [Providers](/docs/using-mew/providers/): available providers and how to configure them
