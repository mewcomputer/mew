---
title: MCP Servers
description: Connect external tools to mew via the Model Context Protocol.
---

mew supports MCP (Model Context Protocol) servers. Each server exposes
tools that are automatically wrapped as `McpTool` implementations and
registered alongside the built-in tools. This lets you extend the agent
with domain-specific capabilities without writing Rust code.

## Configuration

MCP servers are configured in `mcp.json` in your working directory. The
code also checks `.mcp.json`, `.mew/mcp.json`, and `.mew/.mcp.json`
(in that order, first match wins).

### Stdio transport

Stdio servers are spawned as subprocesses. mew communicates with them
over stdin/stdout using the MCP JSON-RPC protocol:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    }
  }
}
```

| Field | Required | Description |
|-------|----------|-------------|
| `command` | yes | Executable to run |
| `args` | no | Command-line arguments |
| `type` | no | Transport type. Defaults to `"stdio"` when `command` is present |

### HTTP transport

For servers reachable over HTTP, use `type` and `url` instead of
`command`:

```json
{
  "mcpServers": {
    "context7": {
      "type": "http",
      "url": "https://mcp.context7.com/mcp"
    }
  }
}
```

| Field | Required | Description |
|-------|----------|-------------|
| `type` | yes | Must be `"http"` |
| `url` | yes | Server URL |

## How it works

1. At startup, mew reads `mcp.json` and connects to each server.
2. Stdio servers are spawned as subprocesses. HTTP servers are contacted
   directly. Both use the MCP JSON-RPC protocol for communication.
3. The server's tools are discovered via `tools/list` and registered in
   the agent's tool map.
4. When the model calls an MCP tool, mew forwards the request to the
   server and waits for the response.
5. Tool output is returned to the model as a tool result.

MCP tool names are qualified to avoid collisions with built-in tools:
`mcp__<server>__<tool>`. For example, a tool called `search` on a server
named `context7` becomes `mcp__context7__search`.

## MCP status in the sidebar

The sidebar shows MCP server connection status and tool count. Press
`Ctrl+3` to toggle the MCP section. Each server shows its name, whether
it connected successfully, and how many tools it exposes.

## Permissions

All MCP tools are treated as `Mutating` sensitivity, meaning they always
require a permission prompt (subject to your [permission mode](/docs/using-mew/permissions/)
and rules). This is conservative: mew can't know what an MCP tool does,
so it asks before running it.

You can add permission rules to auto-allow specific MCP tools:

```toml
[[permissions.rules]]
tool = "mcp__filesystem__read_file"
decision = "allow"
```

## Common MCP servers

These are popular MCP servers that work with mew:

| Server | Purpose | Install |
|--------|---------|---------|
| `@modelcontextprotocol/server-filesystem` | Filesystem access | `npx -y @modelcontextprotocol/server-filesystem <path>` |
| `@modelcontextprotocol/server-github` | GitHub API | `npx -y @modelcontextprotocol/server-github` |
| `@modelcontextprotocol/server-postgres` | PostgreSQL queries | `npx -y @modelcontextprotocol/server-postgres <connection-string>` |
| `@upstash/context7-mcp` | Library documentation lookup | `npx -y @upstash/context7-mcp` |

Set the `GITHUB_PERSONAL_ACCESS_TOKEN` env var for the GitHub server.
Check each server's docs for required environment variables.

## Troubleshooting

**Server not connecting**: check the sidebar MCP section (Ctrl+3) for
error messages. Common causes: wrong command path, missing `npx`, server
crashing on startup.

**Tools not appearing**: the server may not expose any tools, or the
connection may have failed silently. Try running the server command
manually to see if it starts.

**Tool call hangs**: the MCP server may be slow or unresponsive. mew
waits for the server to respond. If a server is consistently slow,
consider whether it's the right tool for the task.

MCP servers communicate over JSON-RPC, so any language that can read
stdin and write stdout can implement one. The protocol spec is at
[modelcontextprotocol.io](https://modelcontextprotocol.io).

## MCP servers vs plugins

MCP servers and [plugins](/docs/using-mew/plugins/) both spawn an external
program and both can add tools, so they get confused. MCP servers only
expose tools over a standard protocol; plugins can also hook the agent
lifecycle, mutate requests, and hold state. See
[Comparing Features](/docs/using-mew/comparisons/) for the full
breakdown.
