---
title: Installation
description: Install mew on macOS and Linux.
---

mew is a terminal-based AI coding assistant. It runs on your machine and
connects to LLM providers you configure. No data leaves your system except
API calls to your chosen provider.

## macOS and Linux

### Homebrew

```sh
brew tap mewcomputer/mew
brew install mew
```

Upgrade later with:

```sh
brew upgrade mew
```

### Install script

```sh
curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | sh
```

The script detects your platform, downloads the latest release from GitHub,
verifies the SHA256 checksum, and installs `mew` to `~/.local/bin` or
`~/bin`. Re-run the script to upgrade.

Install the latest nightly build instead:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | sh -s -- --nightly
```

Environment variables:

| Variable | Description |
|----------|-------------|
| `MEW_CHANNEL` | `stable` or `nightly` (default: `stable`) |
| `MEW_VERSION` | Install a specific stable release, e.g. `v0.2.0` (default: `latest`) |
| `MEW_INSTALL_DIR` | Override the install directory |
| `MEW_DRY_RUN` | Set to `1` to preview without installing |
| `MEW_VERBOSE` | Set to `1` for extra logging |

### Manual download

Grab the tarball for your platform from the [GitHub Releases](https://github.com/mewcomputer/mew/releases)
page. Each release includes a `SHA256SUMS` file for verification.

## Build from source

### Prerequisites

- **Rust 1.75+** with the `2021` edition
- A terminal that supports 256 colors and UTF-8
- An API key for at least one provider (see [Providers](/docs/using-mew/providers/))

### Install with cargo

```sh
cargo install --git https://github.com/mewcomputer/mew mew
```

This builds and installs the `mew` binary to `~/.cargo/bin/mew`. Make sure
`~/.cargo/bin` is on your `PATH`.

### Clone and build

```sh
git clone https://github.com/mewcomputer/mew.git
cd mew
cargo build --release -p mew
```

The binary lands at `target/release/mew`. Copy it somewhere on your `PATH`:

```sh
cp target/release/mew /usr/local/bin/
```

Or use the install recipe:

```sh
just install
```

For a system-wide install that survives across cargo toolchain updates:

```sh
just install-system   # builds, then sudo cp to /usr/local/bin
```

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

The `mew` binary covers every subcommand: interactive chat (default),
one-shot prompts (`mew run`), the daemon (`mew daemon`), and config
editing (`mew config`). You only need `mew-web` if you want the browser
interface.

## Subcommands

| Command | Description |
|---------|-------------|
| `mew` | Start an interactive TUI session (default) |
| `mew run "prompt"` | Run a single prompt non-interactively |
| `mew chat` | Same as the default, but accepts `--connect` to attach to a remote daemon |
| `mew daemon` | Start the WebSocket daemon (for web UI or remote clients). Add `--iroh` for P2P transport. |
| `mew pair` | Generate a QR code for pairing mobile/remote clients over iroh. |
| `mew config show` | Print current configuration |
| `mew config edit` | Open the config file in your editor |
| `mew debug` | Debug tools (permission simulator, VFS inspector) |

## Next steps

- [Quick Start](/docs/getting-started/quick-start/): send your first prompt
- [Configuration](/docs/getting-started/configuration/): set up providers and credentials
- [Providers](/docs/using-mew/providers/): available providers and how to configure them
