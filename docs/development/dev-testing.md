---
title: Testing
description: How to run and write tests for mew.
---

## Running tests

The full CI gate builds the web assets, checks formatting, lint, architecture
and theme generation, and runs the Rust, JavaScript, and end-to-end tests.
Use Node.js 24, pnpm, Zig 0.15.2, and the matching CEF SDK. The
[`ci.yml` workflow](https://github.com/mewcomputer/mew/blob/main/.github/workflows/ci.yml)
lists system dependencies. The SDK setup script is `scripts/setup-ci-cef.sh`.

Before running the workspace or desktop tests, set the state directories to
temporary paths to isolate local chat and layout data.

```sh
test_root="$(mktemp -d)"
export MEW_CONFIG_DIR="$test_root/config"
export MEW_DATA_DIR="$test_root/data"
export MEW_CACHE_DIR="$test_root/cache"
export MEW_SESSION_DIR="$test_root/sessions"

pnpm install --frozen-lockfile
just build-ui
cargo test --locked --all
```

Use the checks below for individual components.

| Command | Coverage |
|---------|----------|
| `cargo test --locked -p mew-tui` | Terminal UI behavior and rendering |
| `cargo test --locked -p mew-daemon --test e2e` | Daemon session behavior over a real WebSocket |
| `cargo test --locked test_text_turn` | Rust tests matching a name |
| `just test-js` | TypeScript client and React UI |
| `just e2e` | Subprocess round-trip through the web bridge |
| `pnpm --filter site test` | Built homepage and documentation site |

## The fake provider

`FakeProvider::text_response("hello")` emits five events.

```rust
pub fn text_response(text: &str) -> Vec<ProviderEvent> {
    // PartStart { part: TextPart { text: "", ... } }
    // PartDelta { field: "text", delta: "hell" }
    // PartDelta { field: "text", delta: "o" }
    // PartEnd { part_id }
    // MessageEnd { finish: Stop, usage: Tokens::default(), cost: 0.0 }
}
```

Events emit with a 10ms delay so cancellation tests can catch mid-stream
state. `tool_call(name, id, input)` produces a tool-call script ending with
`Finish::ToolUse` so the agent's turn loop picks it up and executes the tool.

`TurnRotatingProvider` (in concurrency tests) pops a different script on
each `stream()` call. When scripts run out, it falls back to
`text_response("(no script)")` so the stream always terminates cleanly.

## Daemon end-to-end tests

`crates/mew-daemon/tests/e2e.rs` creates a temporary Unix socket and a
session directory. It starts a real `DaemonServer` backed by a scripted
`FakeProvider`, performs a WebSocket handshake, and asserts on wire messages.
`recv_until` collects messages until a predicate matches. Extend that harness
for daemon behavior that does not need a production build of the bridge.

## Writing tests

- Use `#[tokio::test]` for async tests (current-thread runtime by default)
- Use `tempfile::tempdir()` for filesystem isolation
- Use `DaemonServer::with_session_dir(builder, temp_session_dir)` for daemon
  test isolation
- The `test-utils` feature on `mew-tools` exposes `ToolCtx::test_new(cwd)`
- For provider tests, use `FakeProvider` with a known script
- For daemon tests, use `make_text_agent_factory` to build agents with
  `FakeProvider::text_response`
