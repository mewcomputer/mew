---
title: MCP Servers
description: Experimental Model Context Protocol support in the mew-mcp library.
---

The experimental `mew-mcp` Rust library can connect to Model Context
Protocol servers and wrap their tools. Standard daemon chat and
`mew run` use the built-in tool registry and do not read `mcp.json` or
register MCP tools.

## Connecting from Rust

The library exposes two ways to open a connection. Both perform the MCP
handshake before returning the client.

| Transport | Client method | Arguments |
|-----------|---------------|-----------|
| Stdio | `McpClient::connect_stdio` | Server name, executable, and arguments |
| Streamable HTTP | `McpClient::connect_http` | Server name and endpoint URL |

Stdio runs the server as a subprocess. Streamable HTTP connects to a
server that is already listening.

## Discovering and calling tools

Call `list_tools` on the connected client to retrieve tool names,
descriptions, and input schemas. Use `call_tool` to invoke a tool
directly, or wrap it with `McpTool::new` to use the mew tool interface.

The wrapper qualifies the name as `mcp__<server>__<tool>`. A tool
called `search` on a server named `docs` becomes `mcp__docs__search`.

## Permissions

The wrapper declares `Mutating` sensitivity. An integration that
registers it with the mew permission engine can enforce the configured
[mode and rules](/docs/using-mew/permissions/) using that qualified name.

For protocol details, see the [MCP specification](https://modelcontextprotocol.io/).
