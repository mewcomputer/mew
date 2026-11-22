---
title: Testing
description: How to run and write tests for Mew.
---

## Running tests

`just ci` runs the full CI gate: formatting, Clippy, architecture and theme
checks, Rust tests, JavaScript tests, and subprocess bridge end-to-end tests.
The environment needs Node.js 24, pnpm, Zig 0.15.2, and the matching CEF SDK
(installed via `scripts/setup-ci-cef.sh`; see
[`.github/workflows/ci.yml`](https://github.com/mewcomputer/mew/blob/main/.github/workflows/ci.yml)).

Before running full workspace or desktop tests, point state directories at
temporary paths so tests do not alter your local layout or sessions:

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

Run narrower checks directly:

| Command | Coverage |
|---------|----------|
| `just ci` | Full repository CI gate |
| `just arch-check` | TUI runtime architecture invariants |
| `just theme-codegen-check` | Generated CSS, Rust, and TextMate theme sync |
| `cargo test --locked -p mew-tui` | TUI behavior and golden frames |
| `cargo test --locked -p mew-daemon --test e2e` | Daemon session flow over WebSocket |
| `cargo test --locked test_text_turn` | Rust tests matching a filter |
| `just test-js` | `@mew/web-client`, `mew-web-ui`, and `@mew/electron` unit tests |
| `just e2e` | Subprocess round-trip through `mew-web-bridge` |
| `just test-evals` | Eval runner suite tests |
| `just ios-test` | Swift unit tests on iOS Simulator |
| `pnpm --filter site test` | Built homepage and documentation checks |

## The fake provider

`FakeProvider::text_response("hello")` emits five events spaced 10ms apart so
cancellation tests can observe mid-stream turns:

1. `PartStart { part: TextPart { text: "", ... } }`
2. `PartDelta { field: "text", delta: "hell" }`
3. `PartDelta { field: "text", delta: "o" }`
4. `PartEnd { part_id }`
5. `MessageEnd { finish: Stop, usage: Tokens::default(), cost: 0.0 }`

`tool_call(name, id, input)` emits a tool-call script ending in `Finish::ToolUse`.
`TurnRotatingProvider` (in concurrency tests) pops one script per `stream()`
call and falls back to `text_response("(no script)")` once scripts run out.

## Daemon end-to-end tests

`crates/mew-daemon/tests/e2e.rs` creates temporary socket and session
directories, starts a real `DaemonServer` with `FakeProvider`, connects over
WebSocket, and collects frames with `recv_until`. Extend this harness for
daemon behavior that does not need a compiled web bundle.

## Writing tests

- Use `#[tokio::test]` for async tests and `tempfile::tempdir()` for disk
  isolation.
- Use `DaemonServer::with_session_dir(builder, temp_session_dir)` to isolate
  daemon tests.
- Enable the `test-utils` feature on `mew-tools` for `ToolCtx::test_new(cwd)`.
- Use `FakeProvider` for scripted provider runs and `make_text_agent_factory`
  for daemon tests.
