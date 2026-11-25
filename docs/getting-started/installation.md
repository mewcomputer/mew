---
title: Installation
description: Install Mew on macOS and Linux.
---

Mew runs on your machine and sends model requests to the provider you configure.

## macOS and Linux

These commands install the `mew` CLI and its terminal UI. Nightly builds target
Intel and Apple silicon on macOS, and x86-64 and ARM64 on Linux. Linux builds
require glibc 2.39 or later, OpenSSL 3, and zlib.

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

The [tap](https://github.com/mewcomputer/homebrew-tap) checks for new nightly
builds twice an hour.

### Install script

```sh
curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | bash -s -- --nightly
```

The script detects your platform, downloads the archive from GitHub, verifies
its SHA256 checksum, and installs `mew` to whichever of `~/.local/bin` or
`~/bin` appears first on your `PATH` (defaulting to `~/.local/bin`). Re-run
the command to upgrade. Pass `--stable` for a stable release, or `-h` (`--help`)
to print options.

Environment variables:

| Variable | Description |
|----------|-------------|
| `MEW_CHANNEL` | `stable` or `nightly` (default: `stable`) |
| `MEW_VERSION` | Specific stable version such as `v0.2.0` (default: `latest`) |
| `MEW_INSTALL_DIR` | Override the install directory |
| `MEW_DRY_RUN` | Set to `1` to print the install plan without copying files |
| `MEW_VERBOSE` | Set to `1` for extra logging |

### Manual download

Download the archive for your platform and the `SHA256SUMS` file from the
[nightly release](https://github.com/mewcomputer/mew/releases/tag/nightly).
Verify the checksum with `shasum -a 256 mew-nightly-<target>.tar.gz` on macOS
or `sha256sum mew-nightly-<target>.tar.gz` on Linux against the matching line
in `SHA256SUMS`. Extract the archive, then move
`mew-nightly-<YYYYMMDD>-<target>/bin/mew` (or `mew-<version>-<target>/bin/mew`
for a tagged release) to a directory on your `PATH`.

## Build from source

### Prerequisites

- Current stable Rust and Cargo
- Zig 0.15.2 for Ghostty terminal emulation
- On Linux: OpenSSL development headers, `pkg-config`, and libc++ development headers
- A terminal supporting 256 colors and UTF-8

### Clone and build the CLI

```sh
git clone https://github.com/mewcomputer/mew.git
cd mew
cargo install --locked --path crates/mew
```

Cargo installs `mew` to `~/.cargo/bin` by default. Ensure that directory is on
your `PATH`. Distributed CLI builds omit iroh remote pairing; pass
`--features iroh` to include it.

To embed the desktop host's web UI in the binary, install Node.js 24 or later,
pnpm 10.33.0, and `just`. Run `pnpm install --frozen-lockfile` and `just build-ui`
from the checkout before running `cargo install`.

## Verify the installation

```sh
mew --version
```

This prints the installed version. If the shell cannot find `mew`, add your
install directory (`~/.local/bin`, `~/bin`, `~/.cargo/bin`, `/opt/homebrew/bin`,
or `/usr/local/bin`) to `PATH`.

### Shell completions

Generate completion scripts for `bash`, `zsh`, `fish`, `elvish`, or
`powershell` (omitting the shell detects `$SHELL`):

```sh
mew completions zsh > ~/.zfunc/_mew
```

## What gets installed

| Binary | Source crate | Purpose |
|--------|-------------|---------|
| `mew` | `crates/mew` | Main CLI: runs the TUI, one-shot prompts, and the daemon. |
| `mew-web` | `crates/mew-web-bridge` | Browser bridge and static UI server. Build separately with `just build-web`. |

See the [Web UI](/docs/using-mew/web-ui/) guide to run the browser interface.

## Subcommands

| Command | Description |
|---------|-------------|
| `mew` | Start an interactive TUI session (default) |
| `mew run "prompt"` | Run a single prompt non-interactively |
| `mew chat` | Start interactive chat with flags such as `--provider` or `--connect` |
| `mew daemon` | Start the WebSocket daemon (`--remote` and `--iroh` require `--features iroh`) |
| `mew pair` | Print a pairing QR code for `mew daemon --remote` (requires `--features iroh`) |
| `mew auth` | Log in, check status, or log out of provider OAuth (`login`, `status`, `logout`) |
| `mew config` | View or edit config (`show`, `edit`, `editor`, `path`) |
| `mew theme` | Manage TUI themes (`list`, `current`, `install`, `export-css`) |
| `mew ext` | Manage extensions and tokens (`list`, `enable`, `disable`, `remove`, `doctor`, `install`, `revoke`, `rotate-all`, `token`) |
| `mew debug` | Inspect permissions, VFS resources, model cache, and context (`permissions`, `vfs`, `cache`, `context`) |
| `mew completions` | Generate shell completions |
| `mew tui-capture` | Capture deterministic TUI PNGs or MP4 video with the headless harness |

## Next steps

- [Quick Start](/docs/getting-started/quick-start/): send your first prompt
- [Configuration](/docs/getting-started/configuration/): set up providers and credentials
- [Providers](/docs/using-mew/providers/): configure built-in and custom providers
