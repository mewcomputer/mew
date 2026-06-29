---
title: MCP Servers
description: Connect external tools to mew via the Model Context Protocol.
---

mew supports MCP (Model Context Protocol) servers. Each server exposes
tools that are automatically wrapped as `McpTool` implementations and
registered alongside the built-in tools.

## Configuration

MCP servers are configured in `mcp.json` in your working directory (Claude
Code format):

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_TOKEN": "ghp_..."
      }
    }
  }
}
```

## How it works

1. At startup, mew reads `mcp.json` and spawns each server as a subprocess.
2. Each server communicates via stdin/stdout using the MCP JSON-RPC protocol.
3. The server's tools are discovered and registered in the agent's tool map.
4. When the model calls an MCP tool, mew forwards the request to the server
   and streams the response back.

## MCP status

The sidebar shows MCP server connection status and tool count. Press
`Ctrl+3` to toggle the MCP section.

## Permissions

MCP tools follow the same permission system as built-in tools. The
`sensitivity()` returned by the MCP server determines whether a tool is
auto-allowed (`ReadOnly`) or requires a prompt (`Mutating`/`Dangerous`).
