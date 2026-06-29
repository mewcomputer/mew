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
    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx)
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
| `ReadOnly` | Auto-allowed (no prompt) | Read, Glob, Grep |
| `Mutating` | Prompts the user | Write, Edit, Bash |
| `Dangerous` | Prompts the user (highest urgency) | (reserved for future use) |

The `PermissionEngine` in `mew-config` applies declarative rules before
prompting. Deny rules always win. The escape tier inspects shell commands
for paths outside workspace roots and escalates from `AllowOnce` to `Prompt`.
In `Dangerous` mode, all tools auto-run (bypasses everything except output
redaction).

## ToolCtx

`ToolCtx` carries shared state that tools need at execution time:

```rust
pub struct ToolCtx {
    pub workspace_roots: Vec<PathBuf>,
    pub secrets: Arc<SecretSet>,
    // ... plus a Deref<Target = ToolCtxShared> for static fields
}

pub struct ToolCtxShared {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    // ... shared state that doesn't change per-execution
}
```

`ToolCtx` uses a `Deref` pattern so tools can access both per-execution
fields (workspace roots, secrets) and static fields (session ID, cwd)
through the same reference.

## ToolOutput

```rust
pub struct ToolOutput {
    pub stdout: String,
    pub stderr: String,
    pub diff: Option<String>,
}
```

The TUI renders `stdout` and `stderr` inline in a tool call card. If
`diff` is present, it gets syntax-colored (+/- lines). Bash output
respects the bash-expanded/collapsed toggle (`Ctrl+O`).

## A real tool: Read

The `Read` tool is a good reference implementation:

- `name()`: `"read"`
- `description()`: detailed instructions for the model about how to use it
- `schema()`: uses `OnceLock` to compute the JSON schema once and cache it
- `sensitivity()`: `ReadOnly` (reading files is safe)
- `execute()`:
  1. Parse `path`, `offset`, `limit` from input
  2. `ensure_workspace_path()` checks the path is inside workspace roots
  3. Read the file, detect binary content (null bytes)
  4. Apply secret redaction via `secrets.redact(&content)`
  5. Return `ToolOutput { stdout: content, stderr: "", diff: None }`

Error cases: file not found, permission denied, binary file, too large,
outside workspace. Each error message includes the file path for context.

## Registering a tool

Add your tool to `build_tools()` in `main.rs`:

```rust
let mut tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(Read),
    Arc::new(Write),
    // ... built-in tools ...
];

// Conditional registration:
if !loaded_skills.is_empty() {
    tools.push(Arc::new(Skill::new(skills.clone(), skill_filter.clone())));
}
if !loaded_personas.is_empty() {
    tools.push(Arc::new(SwitchPersonaTool::new(
        personas_arc.clone(),
        pending_persona_switch.clone(),
    )));
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
- All MCP tools return `Sensitivity::Mutating` (always prompt).
- The `McpTool::execute()` method sends a JSON-RPC request to the MCP server
  subprocess and waits for the response.
- HTTP-based MCP servers use `connect_http()` instead of `connect_stdio()`.

## Secret redaction

`SecretSet` provides two-layer defense:

1. **Pattern-based**: regex patterns for common secret formats (API keys,
   tokens, passwords).
2. **File-based**: paths listed in the config's `secret_files` are loaded
   and their contents are redacted from all tool output.

Every tool that returns file content or command output should call
`secrets.redact(&content)` before returning. The `Read` and `Bash` tools
both do this.

## Subagent tools

Subagents are spawned via `Agent::start_subagent()`. The `SubagentRunner`
manages child agent lifecycle, progress updates, and cancellation. The
sidebar shows running subagents with display names (picked from a pool of
25 names via splitmix64 hash) and last progress messages.

To add subagent-controlled UI affordances: runner emits a new
`SubagentEvent` variant → pump translates to `AgentEvent` →
`App::handle_agent_event` stores it → sidebar renders it.
