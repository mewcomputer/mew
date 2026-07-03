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
#   --open    launch the system browser once Vite is running
#
# This runs the Vite dev server (port 5173) and the bridge in the
# background. Vite proxies /ws to the bridge so you get hot-module reload
# and no production build step.
dev-web *flags:
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

    # Start the bridge in the background; Vite proxies /ws to it.
    cargo run -p mew-web-bridge --bin mew-web -- ${bridge_args[@]+"${bridge_args[@]}"} &
    bridge_pid=$!
    trap 'kill $bridge_pid 2>/dev/null || true' EXIT

    # Wait for the bridge TCP listener to bind.
    for i in {1..60}; do
        if bash -c '>/dev/tcp/127.0.0.1/9847' 2>/dev/null; then break; fi
        sleep 0.5
    done

    if $do_open; then
        # Give Vite a moment to start, then open the dev URL.
        (sleep 2 && just _open-url "http://127.0.0.1:5173/" >/dev/null 2>&1 &)
    fi

    pnpm --filter mew-web-ui dev
    wait $bridge_pid

# Kills the bridge (port 9847) and the daemon process it spawned.
kill-daemon:
    #!/usr/bin/env bash
    set -euo pipefail
    # Kill the bridge first so it doesn't re-spawn a daemon.
    if lsof -i :9847 >/dev/null 2>&1; then
        echo "killing mew-web-bridge on port 9847"
        lsof -ti :9847 | xargs kill 2>/dev/null || true
    else
        echo "no mew-web-bridge running on port 9847"
    fi
    # Kill any lingering daemon processes.
    pids=$(pgrep -f "target/debug/mew daemon" 2>/dev/null || true)
    if [ -n "$pids" ]; then
        echo "killing mew daemon: $pids"
        echo "$pids" | xargs kill 2>/dev/null || true
    fi
    # Clean up the socket.
    rm -f /tmp/mew.sock 2>/dev/null || true

# Rebuilds the mew binary (daemon + bridge) and restarts everything.
# Use this after changing Rust source files. The bridge auto-spawns
# a fresh daemon from the rebuilt binary.
restart-daemon:
    #!/usr/bin/env bash
    set -euo pipefail
    just kill-daemon
    cargo build -p mew-web-bridge -p mew
    rm -f /tmp/mew.sock 2>/dev/null || true
    nohup target/debug/mew-web > /tmp/mew-bridge.log 2>&1 &
    disown
    sleep 2
    if pgrep -f "target/debug/mew-web" >/dev/null 2>&1; then
        echo "✓ bridge + daemon running"
        tail -2 /tmp/mew-bridge.log
    else
        echo "✗ failed to start — check /tmp/mew-bridge.log"
        cat /tmp/mew-bridge.log
        exit 1
    fi

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

# Full watch dev mode: Vite dev server + cargo-watch for the Rust stack.
# Rebuilds and restarts the bridge/daemon whenever Rust sources change.
# Flags are forwarded to the bridge; use --open to launch the browser.
dev-web-watch *flags:
    {{justfile_directory()}}/scripts/dev-web-watch.sh {{flags}}

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

# Generate a Homebrew formula from a GitHub release.
# Usage: just generate-homebrew-formula v0.2.0
generate-homebrew-formula version:
    scripts/generate-homebrew-formula.sh {{version}}

site-dev:
    pnpm --filter site dev

# ── iOS mobile core ──────────────────────────────────────────────
# Build mew-mobile-core for both iOS targets, generate Swift bindings,
# and create an XCFramework for the SwiftPM package.
#
# Prerequisites:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim
#   cargo install uniffi-bindgen-cli --version 0.32
#
# Output:
#   mew-ios/MewMobileCore/Sources/MewMobileCore/mew_mobile_core.swift  (generated bindings)
#   mew-ios/MewMobileCore/XCFramework/mew_mobile_core.xcframework      (universal framework)
ios-core:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "$(dirname "$0")"
    IOS_DIR="mew-ios/MewMobileCore"
    FRAMEWORK_DIR="${IOS_DIR}/XCFramework"
    BINDINGS_DIR="${IOS_DIR}/Sources/MewMobileCore"

    echo "── Building for aarch64-apple-ios (device) ──"
    cargo build -p mew-mobile-core --release --target aarch64-apple-ios

    echo "── Building for aarch64-apple-ios-sim (simulator) ──"
    cargo build -p mew-mobile-core --release --target aarch64-apple-ios-sim

    echo "── Generating Swift bindings ──"
    mkdir -p "${BINDINGS_DIR}"
    cargo run -p mew-mobile-core --bin uniffi-bindgen -- generate \
        --library target/release/libmew_mobile_core.dylib \
        --language swift --out-dir "${BINDINGS_DIR}"

    # SwiftPM binary targets need module.modulemap (not <name>.modulemap)
    cp "${BINDINGS_DIR}/mew_mobile_coreFFI.modulemap" "${BINDINGS_DIR}/module.modulemap" 2>/dev/null || true

    echo "── Creating XCFramework ──"
    # Use a temp headers dir so the xcframework only gets FFI headers (not the .swift)
    HEADERS_TMP=$(mktemp -d)
    cp "${BINDINGS_DIR}/mew_mobile_coreFFI.h" "${HEADERS_TMP}/"
    cp "${BINDINGS_DIR}/mew_mobile_coreFFI.modulemap" "${HEADERS_TMP}/"
    cp "${HEADERS_TMP}/mew_mobile_coreFFI.modulemap" "${HEADERS_TMP}/module.modulemap"
    rm -rf "${FRAMEWORK_DIR}/mew_mobile_core.xcframework"
    xcodebuild -create-xcframework \
        -library "target/aarch64-apple-ios/release/libmew_mobile_core.a" \
        -headers "${HEADERS_TMP}" \
        -library "target/aarch64-apple-ios-sim/release/libmew_mobile_core.a" \
        -headers "${HEADERS_TMP}" \
        -output "${FRAMEWORK_DIR}/mew_mobile_core.xcframework"
    rm -rf "${HEADERS_TMP}"

    echo ""
    echo "✓ mew-mobile-core built for iOS"
    echo "  Bindings:  ${BINDINGS_DIR}/mew_mobile_core.swift"
    echo "  Framework: ${FRAMEWORK_DIR}/mew_mobile_core.xcframework"

