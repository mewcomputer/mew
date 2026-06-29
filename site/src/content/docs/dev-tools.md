---
title: Adding a Tool
description: How to implement a new tool for the mew agent.
---

Tools are how the agent interacts with the filesystem, runs commands,
and calls external services. Every tool implements the `Tool` trait.

## The Tool trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;
    fn sensitivity(&self) -> Sensitivity;
    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError>;
}
```

- `name()` — identifier the model uses to call the tool
- `description()` — shown to the model in the tool list
- `schema()` — JSON Schema for the tool's input parameters
- `sensitivity()` — controls the default permission gate
- `execute()` — runs the tool and returns output

## Sensitivity levels

| Level | Behavior |
|-------|----------|
| `ReadOnly` | Auto-allowed (no prompt) |
| `Mutating` | Prompts the user (unless permission mode overrides) |
| `Dangerous` | Prompts the user (highest urgency) |

The `PermissionEngine` in `mew-config` applies declarative rules before
prompting. Deny rules always win. The escape tier inspects shell commands
for paths outside workspace roots.

## Steps

1. **Implement the `Tool` trait** in a new module or existing tool file.

2. **Register in `build_tools()`** (`main.rs`). Add your tool to the `Vec<Arc<dyn Tool>>`:

```rust
tools.push(Arc::new(MyTool::new()));
```

3. **Test** using `ToolCtx::test_new()` (requires the `test-utils` feature
   on `mew-tools`).

## Tool output

`ToolOutput` contains stdout, stderr, and optionally a diff. The TUI
renders tool output inline with expand/collapse. Diffs get syntax-colored
(+/-) lines. Bash output respects the bash-expanded/collapsed toggle.

## MCP tools

MCP server tools are automatically wrapped as `McpTool` implementations
after `connect_mcp_servers()` runs. They register alongside built-in tools
with no manual code needed. All MCP tools are `Mutating` sensitivity.

## Subagent tools

Subagents are spawned via `Agent::start_subagent()`. The `SubagentRunner`
manages child agent lifecycle, progress updates, and cancellation. The
sidebar shows running subagents with display names and last progress messages.
