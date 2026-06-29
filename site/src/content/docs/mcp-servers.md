---
title: MCP Servers
description: Connect external tools to mew via the Model Context Protocol.
---

mew supports MCP (Model Context Protocol) servers. Each server exposes
tools that are automatically wrapped as `McpTool` implementations and
registered alongside the built-in tools.

## Configuration

MCP servers are configured in `mcp.json` in your working directory. The
code also checks `.mcp.json`, `.mew/mcp.json`, and `.mew/.mcp.json`.

### Stdio transport

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

### HTTP transport

For servers reachable over HTTP, use `type` and `url` instead of `command`:

```json
{
  "mcpServers": {
    "remote": {
      "type": "http",
      "url": "https://example.com/mcp"
    }
  }
}
```

## How it works

1. At startup, mew reads `mcp.json` and connects to each server.
2. Stdio servers are spawned as subprocesses communicating via stdin/stdout
   using the MCP JSON-RPC protocol. HTTP servers are contacted directly.
3. The server's tools are discovered and registered in the agent's tool map.
4. When the model calls an MCP tool, mew forwards the request to the server
   and streams the response back.

## MCP status

The sidebar shows MCP server connection status and tool count. Press
`Ctrl+3` to toggle the MCP section.

## Permissions

All MCP tools are treated as `Mutating` sensitivity, meaning they always
require a permission prompt (subject to your permission mode and rules).
