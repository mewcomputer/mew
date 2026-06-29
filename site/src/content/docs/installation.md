---
title: Installation
description: How to install mew on macOS and Linux.
---

mew is a terminal-based AI coding assistant. It runs entirely on your machine
and connects to LLM providers you configure.

## Prerequisites

- **Rust 1.75+** (for building from source)
- A terminal that supports 256 colors and UTF-8

## Build from source

```sh
git clone https://github.com/natalie/mew.git
cd mew
cargo build --release -p mew
```

The binary will be at `target/release/mew`. Copy it to your PATH:

```sh
cp target/release/mew /usr/local/bin/
```

Or use the install recipe:

```sh
just install
```

## Verify the installation

```sh
mew --version
```

## Next steps

- [Quick Start](/docs/quick-start/): send your first prompt
- [Configuration](/docs/configuration/): set up providers and credentials
