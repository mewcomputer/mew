#!/usr/bin/env bash
# Run the full mew web dev stack with Rust auto-reload.
#
#   - Vite dev server runs in the background for hot-module reload.
#   - cargo-watch rebuilds/restarts the bridge (and the daemon it spawns)
#     whenever Rust sources change.
#
# Usage: scripts/dev-web-watch.sh [bridge-args...] [--open]

set -euo pipefail
cd "$(dirname "$0")/.."

do_open=false
bridge_args=()
for arg in "$@"; do
  case "$arg" in
    --open) do_open=true ;;
    *) bridge_args+=("$arg") ;;
  esac
done

# Start Vite dev server in the background.
pnpm --filter mew-web-ui dev &
vite_pid=$!
trap 'kill $vite_pid 2>/dev/null || true' EXIT

# Wait for Vite to bind.
for _ in {1..60}; do
  if bash -c '>/dev/tcp/127.0.0.1/5173' 2>/dev/null; then break; fi
  sleep 0.2
done

if $do_open; then
  (sleep 1 && just _open-url "http://127.0.0.1:5173/" >/dev/null 2>&1 &)
fi

# The bridge spawns the daemon on this socket by default.
DAEMON_SOCKET="/tmp/mew.sock"

cleanup_daemon() {
  # Best-effort: kill whichever process is holding the daemon socket, then
  # remove the socket so a freshly restarted daemon can bind.
  for pid in $(lsof -t "$DAEMON_SOCKET" 2>/dev/null || true); do
    kill "$pid" 2>/dev/null || true
  done
  rm -f "$DAEMON_SOCKET"
}

run_bridge() {
  cleanup_daemon
  cargo run -p mew-web-bridge --bin mew-web -- ${bridge_args[@]+"${bridge_args[@]}"}
}

export -f cleanup_daemon run_bridge

# Watch Rust sources and rerun the bridge each time something changes.
cargo watch \
  -w crates \
  -x 'build --bin mew' \
  -x 'build -p mew-web-bridge --bin mew-web' \
  -s 'bash -c run_bridge'
