---
title: MCP Servers
description: Experimental Model Context Protocol support in the mew-mcp library.
---

The experimental `mew-mcp` library wraps Model Context Protocol tools as
`McpTool` implementations. Standard daemon chat uses the built-in tool
registry. The configuration below describes the format for developing
against the MCP library.

## Configuration

The library resolver looks for `mcp.json` in the requested working directory.
It also checks `.mcp.json`, `.mew/mcp.json`, and `.mew/.mcp.json`
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

The library connects to stdio servers as subprocesses and contacts HTTP
servers directly. It uses JSON-RPC `tools/list` for discovery and wraps
tool calls so their output can be returned to the agent.

MCP tool names are qualified to avoid collisions with built-in tools:
`mcp__<server>__<tool>`. For example, a tool called `search` on a server
named `context7` becomes `mcp__context7__search`.

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

Examples of servers that expose MCP tools:

| Server | Purpose | Install |
|--------|---------|---------|
| `@modelcontextprotocol/server-filesystem` | Filesystem access | `npx -y @modelcontextprotocol/server-filesystem <path>` |
| `@modelcontextprotocol/server-github` | GitHub API | `npx -y @modelcontextprotocol/server-github` |
| `@modelcontextprotocol/server-postgres` | PostgreSQL queries | `npx -y @modelcontextprotocol/server-postgres <connection-string>` |
| `@upstash/context7-mcp` | Library documentation lookup | `npx -y @upstash/context7-mcp` |

Set the `GITHUB_PERSONAL_ACCESS_TOKEN` env var for the GitHub server.
Check each server's docs for required environment variables.

## Troubleshooting

When testing the library, check the command path and startup logs if a
server fails to start.

**Tools not appearing**: the server may not expose any tools, or the
connection may have failed silently. Try running the server command
manually to see if it starts.

**Tool call hangs**: the MCP server may be slow or unresponsive. mew
waits for the server to respond. If a server is consistently slow,
consider whether it's the right tool for the task.

MCP servers communicate over JSON-RPC, so any language that can read
stdin and write stdout can implement one. The protocol spec is at
[modelcontextprotocol.io](https://modelcontextprotocol.io).
