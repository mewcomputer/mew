# mew — justfile

set dotenv-load := true
set positional-arguments := true

# Default recipe: build the binary
build:
    cargo build --release -p mew

# Run all tests
test:
    cargo test --all

# Alias for the Rust test recipe (clearer when both Rust and JS exist).
test-rs:
    cargo test --all

# Run tests with verbose output
test-v:
    cargo test --all -- --nocapture

# Install JS/TS dependencies for the pnpm workspace (root + all members).
# Required once on first checkout; subsequent runs are cheap no-ops.
install-js:
    pnpm install

# Run TypeScript tests for mew-web-client via the pnpm workspace.
test-js:
    pnpm --filter @mew/web-client test

# Run tests for both Rust and the TypeScript client.
test-all: test-rs test-js

# Build the TypeScript library (produces dist/ with .d.ts + .js).
build-js:
    pnpm --filter @mew/web-client build

# Build everything needed for the web harness: the bridge binary
# (mew-web), the daemon binary (spawned by the bridge at runtime),
# the mew-web-client TypeScript library, and the React UI (mew-web-ui).
# Idempotent — incremental cargo builds are cheap. `pnpm install` only
# re-runs if lockfile or package.json changed.
build-web:
    cargo build -p mew-web-bridge -p mew
    pnpm install
    pnpm --filter @mew/web-client build
    pnpm --filter mew-web-ui build

# Run the web harness in the foreground. The bridge auto-spawns
# `mew daemon` on the default Unix socket if it isn't already running.
# Open http://127.0.0.1:9847/ in a browser to chat.
#
# Flags forwarded to mew-web-bridge:
#   --port 127.0.0.1:9999  change the listen port
#   --spawn false           don't auto-spawn mew daemon (you started it)
#   --daemon-socket PATH    override the daemon unix socket path
#
# dev-web-specific flags:
#   --open    launch the system browser once the bridge is bound
dev-web *flags: build-web
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    bridge_args=()
    do_open=false
    for arg in "$@"; do
        case "$arg" in
            --open) do_open=true ;;
            *) bridge_args+=("$arg") ;;
        esac
    done
    if $do_open; then
        # Open 2s after the bridge starts; that gives the listener time to bind.
        (sleep 2 && just _open-url "http://127.0.0.1:9847/" >/dev/null 2>&1 &)
    fi
    cargo run -p mew-web-bridge --bin mew-web -- ${bridge_args[@]+"${bridge_args[@]}"}

# Open the chat UI in the system browser. Default URL is the bridge's
# default listen address. Override with:  just web-open http://host:port/
web-open url="http://127.0.0.1:9847/":
    #!/usr/bin/env bash
    case "$(uname -s)" in
        Darwin) open "{{url}}" ;;
        Linux) xdg-open "{{url}}" ;;
        *) echo "open {{url}} in your browser" ;;
    esac

# Internal helper used by `dev-web --open` to launch the default browser
# without piping through the recipe return value.
_open-url url:
    #!/usr/bin/env bash
    case "$(uname -s)" in
        Darwin) open "{{url}}" ;;
        Linux) xdg-open "{{url}}" ;;
        *) echo "open {{url}} in your browser" ;;
    esac

# Dev mode: run Vite's dev server with hot-module reload. The browser
# connects to Vite (default :5173), which proxies WebSocket connections
# to the mew-web bridge (which in turn relays to the daemon). Start the
# bridge separately in another terminal: `just dev-web --spawn false`.
dev-ui:
    pnpm --filter mew-web-ui dev

# Remove all build artifacts: Vite dist, TypeScript library dist, and
# Vite/turbo caches. Useful when switching branches or after dependency
# changes that confuse incremental builds.
clean-web:
    rm -rf mew-web-ui/dist mew-web-client/dist
    rm -rf mew-web-ui/node_modules/.vite

# Run clippy on Rust and type-check TypeScript. CI-style gate without tests.
lint-all: clippy
    pnpm --filter @mew/web-client exec tsc --noEmit

# CI-ready check: format, clippy, unit/integration tests (Rust + JS),
# and the subprocess e2e test. The e2e test needs the binaries built;
# `build-web` depends on the right set.
ci: fmt clippy test-all e2e

# End-to-end check: build binaries and run subprocess e2e tests.
# The e2e test in mew-web-bridge spawns real `mew` and `mew-web`
# subprocesses and verifies the full stack round-trip. Requires
# `cargo build` first; the test gracefully skips if the binaries
# aren't present.
e2e: build-web
    cargo test -p mew-web-bridge --test bin_e2e

mew *args: build
    cargo run -p mew -- "$@"

# Build and run mew. All args after "run" are forwarded to the binary.
# Usage: just run --model deepseek-v4-flash "hello world"
run *args: build
    cargo run -p mew -- run "$@"

# Install to ~/.cargo/bin
install:
    cargo install --path crates/mew

# Install to /usr/local/bin (requires sudo)
install-system: build
    sudo cp target/release/mew /usr/local/bin/mew

# Clean build artifacts
clean:
    cargo clean

# Format all Rust code
fmt:
    cargo fmt

# Run clippy
clippy:
    cargo clippy --all -- -D warnings

# Record a new provider fixture (set MEW_RECORD=1 and provider creds)
record:
    MEW_RECORD=1 cargo test -p mew-provider-openai

# Show module dependencies
deps:
    cargo tree

# Update dependencies
tidy:
    cargo update

site-dev:
    pnpm --filter site dev
