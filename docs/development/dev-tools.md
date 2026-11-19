---
title: Adding a Tool
description: How to implement a new tool for the mew agent.
---

Tools are how the agent interacts with the filesystem, runs commands, and
calls external services. Every tool implements the `Tool` trait.

## The Tool trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;
    fn sensitivity(&self) -> Sensitivity;
    async fn execute(&self, ctx: ToolCtx, input: serde_json::Value)
        -> Result<ToolOutput, ToolError>;
}
```

- `name()` — identifier the model uses to call the tool. Must be unique.
- `description()` — shown to the model in the tool list. This is how the
  model decides when to call your tool, so be specific.
- `schema()` — JSON Schema for the tool's input parameters. Must return
  a `serde_json::Value` describing the expected `input` object.
- `sensitivity()` — controls the default permission gate (see below).
- `execute()` — runs the tool and returns output. Receives the parsed
  input and a `ToolCtx` with workspace roots, secrets, and shared state.

## Sensitivity levels

| Level | Behavior | Examples |
|-------|----------|----------|
| `ReadOnly` | Normally allowed in Standard mode | Read, Glob, Grep |
| `Mutating` | Normally prompts in Standard mode | Write, Edit, Bash |
| `Dangerous` | Prompts in Standard mode | High-risk operations |

The permission engine in `mew-config` evaluates the active mode, rules, and
workspace or secret protections. See [Permissions](/docs/using-mew/permissions/)
for how these settings interact.

## ToolCtx

`ToolCtx` carries the call ID, cancellation token, and progress channel
for each call. It dereferences to `ToolCtxShared`, which holds the session ID,
working directory, secrets, hashline snapshot store, and other session-scoped
state. Access these through fields such as `ctx.cwd` and `ctx.snapshot_store`.

## ToolOutput

Return `mew_hooks::ToolOutput`, which carries `output`, `error`, and an optional
`diff`. It can also carry structured metadata, per-file change counts, and
images. Return a `ToolError` for invalid input, cancellation, or an execution
failure that prevents producing a result.

## A real tool: Read

The `Read` tool in `crates/mew-tools/src/tools/read.rs` shows path resolution,
line-based pagination, and how to feed the hashline snapshot store. Its
schema uses `OnceLock` to compute the JSON schema once, and its sensitivity
is `ReadOnly`.

## Registering a tool

Add your tool to `build_tools()` in `crates/mew/src/setup/agent.rs`.

```rust
let mut tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(Read),
    Arc::new(Write),
    // ... built-in tools ...
];

// Conditional registration:
if !skills.is_empty() {
    tools.push(Arc::new(Skill::new(skills, skill_filter, template_ctx)));
}
// MCP tools are added after connection:
tools.extend(mcp_tools);
```

The agent's tool map is `HashMap<String, Arc<dyn Tool>>`, keyed by `name()`.
Persona tool allowlists filter this map at turn time via `active_tool_names`.

## MCP tools

MCP server tools are automatically wrapped as `McpTool` implementations
after `connect_mcp_servers()` runs. No manual code needed. Key details:

- MCP tool names are qualified: `mcp__<server>__<tool>` to avoid collisions
  with built-in tools.
- MCP tools declare `Sensitivity::Mutating` and use the active permission policy.
- The `McpTool::execute()` method sends a JSON-RPC request to the MCP server
  subprocess and waits for the response.
- HTTP-based MCP servers use `connect_http()` instead of `connect_stdio()`.

## Secret redaction

`SecretSet` holds configured secret words and file globs. Use helpers in
`mew_tools::secrets` when handling file content or command output. Check
`read`, `grep`, and `bash` for examples of the protections appropriate to
each type of output.

## Subagent tools

`mew-agent` owns the subagent task registry and runner. The runner emits
progress and completion events, which the daemon forwards to clients.
See [Tasks and orchestration](/docs/development/dev-orchestration/) for
the task registry and its lifecycle.
