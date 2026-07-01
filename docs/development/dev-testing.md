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

**`mew-provider-fake`** (13 tests): script shape verification, part_id
consistency across deltas, streaming semantics, empty script, multi-byte
text round-trip, delta count matches chunked text.

**`mew-protocol`** (63 tests): exhaustive roundtrip for every `ClientMessage`
and `ServerMessage` variant, nested structures (SubagentStart, TodosUpdated,
AskUserRequest), negative tests (malformed JSON, missing fields, wrong types,
unknown tags), `PermissionDecision` ↔ hooks conversions.

**`mew-daemon` e2e** (12 tests): full daemon lifecycle over real Unix sockets.
Uses a `spawn_daemon` helper that creates a temp socket, builds an agent with
`FakeProvider`, and starts the daemon in a background task. Tests cover
SessionReady, prompt streaming, tool-call finish, prompt without session,
invalid JSON, `/clear` + `/compact` slash commands, cancel mid-stream,
sequential prompts, concurrent connections.

### Tier 2: Behavior

**`mew-tools` integration** (8 tests): composed tool scenarios verifying
cross-tool composition. Write → read round-trip, write → edit → bash cat
verification, glob → grep composition, bash nonzero exit surfaces code,
glob no-match returns empty, read offset/limit pagination, grep extension
filter, edit preserves filename for glob.

**Agent escape-tier integration** (2 tests): `cat /etc/passwd` against
`workspace_roots = [tempdir]` escalates to `Prompt` even in Permissive mode.
With empty roots and an Allow rule, no prompt fires. Tests the escape tier
end-to-end through `agent.run()`.

**Regression tests** (4 tests): edit "not found" includes first/last line
snippets + recovery hint, edit "ambiguous" suggests more context, read
errors (missing + binary) include the file path. Pins down UX fixes so a
refactor can't silently regress them.

### Tier 3: Polish

**`mew-daemon` concurrency** (6 tests): 5 concurrent connections get
distinct sessions, sequential prompts on one connection produce distinct
part IDs (via `TurnRotatingProvider`), concurrent cross-connection prompts
have disjoint part IDs, prompt during in-flight turn serializes, rapid-fire
Cancel doesn't crash, slash command during stream doesn't block.

**`mew-session` roundtrip** (6 tests): JSONL write/load, empty session
loads empty, meta persists (model + subagent_name), multi-session
independence, unknown session errors, reopen appends without truncating.

**`mew-personas` discovery** (8 tests): single + multi persona discovery,
model pin + tool allowlist, `tools_deny`, markdown fence preservation,
invalid name rejected, template flag parsed.

**`mew-subagents` loader** (9 tests): user defs picked up, built-in defaults
included unless overridden, user override replaces built-in, tool allowlist
parses, empty dir yields built-ins, display-name pool has 10+ entries,
deterministic per seed, distribution varies, all names from pool.

## The fake provider

`FakeProvider::text_response("hello")` produces a 4-event script:

```rust
pub fn text_response(text: &str) -> Vec<ProviderEvent> {
    // PartStart { part: TextPart { text: "", ... } }
    // PartDelta { field: "text", delta: "he" }  (4 chars per delta)
    // PartDelta { field: "text", delta: "llo" }
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

## Daemon e2e test harness

```rust
async fn spawn_daemon<F>(agent_factory: F) -> (TempDir, String)
where F: Fn(AgentBuildParams) -> Result<(Agent, Option<String>, Option<String>)>
    + Send + Sync + 'static
{
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("mew.sock");
    let session_dir = dir.path().join("sessions");
    let builder: AgentBuilder = Arc::new(agent_factory);
    let server = DaemonServer::with_session_dir(builder, session_dir);
    tokio::spawn(async move { let _ = server.run(&socket_str).await; });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (dir, socket_str)
}
```

Tests connect via `UnixStream::connect`, perform a WebSocket handshake, and
use `recv_until` to collect messages until a predicate matches:

```rust
async fn recv_until<F>(ws: &mut Ws, pred: F) -> Vec<ServerMessage>
where F: FnMut(&ServerMessage) -> bool
```

## Writing tests

- Use `#[tokio::test]` for async tests (current-thread runtime by default)
- Use `tempfile::tempdir()` for filesystem isolation
- Use `DaemonServer::with_session_dir(builder, temp_session_dir)` for daemon
  test isolation
- The `test-utils` feature on `mew-tools` exposes `ToolCtx::test_new()`
- For provider tests, use `FakeProvider` with a known script
- For daemon tests, use `make_text_agent_factory` to build agents with
  `FakeProvider::text_response`
