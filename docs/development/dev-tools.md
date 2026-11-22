---
title: Adding a Tool
description: How to implement and register a new tool for the Mew agent.
---

Every agent tool implements the `Tool` trait in `crates/mew-tools/src/lib.rs`.

## The Tool trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &Value;
    fn sensitivity(&self) -> Sensitivity;
    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError>;
    async fn shutdown(&self) -> Result<(), ToolError> { Ok(()) }
}
```

- `name()`: unique identifier used by the model and permission rules.
- `description()`: prompt-visible summary telling the model when to call the tool.
- `schema()`: JSON Schema describing the expected `input` object.
- `sensitivity()`: default permission tier (`ReadOnly`, `Mutating`, or `Dangerous`).
- `execute()`: runs the tool with `ToolCtx` and parsed JSON arguments.
- `shutdown()`: optional cleanup hook called when a session shuts down.

## Sensitivity levels

| Level | Standard-mode default | Built-in examples |
|-------|-----------------------|-------------------|
| `ReadOnly` | Auto-allow | `read`, `glob`, `grep` |
| `Mutating` | Prompt | `write`, `edit_str_replace`, `edit_hashline` |
| `Dangerous` | Prompt (also in `permissive`) | `bash`, `shell_background`, `shell_monitor`, MCP tools |

`mew-config` combines sensitivity with the permission mode, declarative rules,
workspace checks, and secret-file guards. See
[Permissions](/docs/using-mew/permissions/).

## ToolCtx and ToolOutput

`ToolCtx` holds the call's `call_id`, `cancel` (`CancellationToken`), and
`progress_tx` channel, and dereferences to `ToolCtxShared`. `ToolCtxShared`
stores session-wide state: `session_id`, `cwd`, `secrets`, `snapshot_store`,
`browser_access`, `shell_session`, `dispatcher`, and `supports_vision`.

Return `mew_hooks::ToolOutput` with `output`, `error`, an optional `diff`, and
optional structured metadata, file stats, or images. Return `ToolError` for
invalid arguments, cancellation, or execution failures.

## Reference example: Read

`Read` (`crates/mew-tools/src/tools/read.rs`) shows path resolution, line
pagination, image gating via `supports_vision`, and feeding the hashline
`SnapshotStore`. Its schema uses `OnceLock` and declares `Sensitivity::ReadOnly`.

## Registering a tool

Register built-in tools in `build_tools()` (`crates/mew/src/setup/agent.rs`):

```rust
let mut tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(Read),
    Arc::new(Write),
    // ... built-in tools ...
];

if !skills.is_empty() {
    tools.push(Arc::new(Skill::new(skills, skill_filter, template_ctx)));
}
if hashline_enabled {
    tools.push(Arc::new(EditHashline));
} else {
    tools.push(Arc::new(EditStrReplace));
}
```

MCP tools (`connect_mcp_servers()`), browser tools, and `python` code-mode tools
are appended by the agent builders (`build_session_agent` / `build_agent`)
around `build_tools()`. The agent indexes tools in a
`HashMap<String, Arc<dyn Tool>>` keyed by `name()`, which persona allowlists
filter per turn.

## MCP tools

`McpTool` automatically wraps discovered MCP server tools (`mcp__<server>__<tool>`)
over stdio or streamable HTTP (`connect_stdio()` / `connect_http()`), declares
`Sensitivity::Dangerous`, forwards JSON-RPC tool calls, and applies secret
redaction.

## Secret redaction

Use helpers in `mew_tools::secrets` against `ctx.secrets` (`SecretSet`) when
reading files or capturing command output. See `read.rs`, `grep.rs`, and
`bash.rs` for reference implementations.

## Subagent tools

`mew-agent` tracks subagent tasks, runs child agents, and forwards their progress
and completion events to attached clients. See
[Tasks and orchestration](/docs/development/dev-orchestration/).
