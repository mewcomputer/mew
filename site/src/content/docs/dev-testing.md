---
title: Testing
description: How to run and write tests for mew.
---

## Running tests

```sh
just ci           # fmt + clippy + test-all + e2e (full CI gate)
cargo test --all  # all Rust tests
cargo test -p mew-tui  # one crate
cargo test test_text_turn  # one test by name
just test-v      # verbose output
```

## Test tiers

### Tier 1: Foundation

- **`mew-provider-fake`**: script shape verification, part_id consistency,
  streaming semantics, multi-byte text round-trip
- **`mew-protocol`**: exhaustive roundtrip for every `ClientMessage` and
  `ServerMessage` variant, negative tests (malformed JSON, missing fields)
- **`mew-daemon` e2e**: full daemon lifecycle over real Unix sockets
  (SessionReady, streaming, tool calls, cancel, slash commands, concurrent
  connections)

### Tier 2: Behavior

- **`mew-tools` integration**: composed tool scenarios (write→read, edit→bash
  verification, glob→grep composition)
- **Agent escape-tier integration**: workspace sandbox enforcement through
  `agent.run()` end-to-end
- **Regression tests**: edit error messages include file path + snippets,
  read errors include path, grep extension filtering

### Tier 3: Polish

- **`mew-daemon` concurrency**: 5 concurrent connections get distinct sessions,
  sequential prompts produce distinct part IDs, cross-connection isolation,
  rapid-fire cancel, slash during stream
- **`mew-session` roundtrip**: JSONL write/load, meta persistence, multi-session
  independence
- **`mew-personas` discovery**: single + multi persona, model pin, tool allowlist,
  template flag, invalid name rejection
- **`mew-subagents` loader**: user defs, built-in defaults, display-name pool,
  deterministic per seed

## The fake provider

`FakeProvider::text_response("hello")` returns a script that produces
`PartStart → PartDelta(s) → PartEnd → MessageEnd`. Events emit with a 10ms
delay so cancellation tests can catch mid-stream state.

`FakeProvider::tool_call(name, id, input)` produces a tool-call script that
ends with `Finish::ToolUse` so the agent's turn loop picks it up and executes
the tool.

## E2E test

`just e2e` builds the web bridge + daemon binaries and runs a subprocess
e2e test that verifies the full stack: daemon starts, bridge connects,
session is created, prompt streams, text arrives.

## Writing tests

- Use `#[tokio::test]` for async tests (default: current-thread runtime)
- Use `tempfile::tempdir()` for filesystem isolation
- Use `DaemonServer::with_session_dir(builder, temp_session_dir)` for daemon
  test isolation
- The `test-utils` feature on `mew-tools` exposes `ToolCtx::test_new()`
