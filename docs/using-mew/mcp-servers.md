---
title: MCP Servers
description: Configure Model Context Protocol tools in Mew.
---

Mew can discover tools from Model Context Protocol servers and make them
available in both direct tool mode and Python code mode. Add servers to
`mcp.json` beside the `config.toml` shown by `mew config path`, or in your
project:

```json
{
  "mcpServers": {
    "docs": {
      "url": "https://mcp.context7.com/mcp"
    },
    "files": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/files"]
    }
  }
}
```

A `url` selects streamable HTTP; a `command` selects stdio. You can also set
`type` to `http` or `stdio`. HTTP servers accept a `headers` object, and stdio
servers accept an `env` object in addition to `args`. Values are literal
strings: Mew does not expand environment-variable references in `mcp.json`.
Stdio servers inherit the daemon's environment and use its startup directory.

Mew reads the global file and the closest project `mcp.json` between the
startup directory and the Git root. Outside a Git repository, it checks the
startup directory. Project entries replace global entries with the same name;
set `"disabled": true` on an entry to disable it. The daemon discovers servers
once at startup and shares them across sessions. Restart the daemon after
changing the configuration or server tools. `mew run` discovers them at the
start of each run. A server that fails to start, fails discovery, or takes
longer than 15 seconds is skipped with a warning. Server stderr is suppressed.
When Mew starts a daemon, the desktop and TUI wait up to 30 seconds for it to
become ready, including server discovery.

Mew trusts project configuration and starts configured stdio commands during
discovery, before a tool permission is requested. Configure only servers you
trust. MCP servers execute outside the Python sandbox and can use the files,
network, and credentials available to their own process. Python isolation does
not restrict what an MCP server can do.

## Using tools and permissions

Tools are named `mcp__<server>__<tool>`, for example
`mcp__files__read_file`. Names are normalized for model providers;
duplicate names and names longer than 64 characters are skipped. In Python
mode, inspect `ALL_TOOLS` to find their exact names. MCP tools can return text,
structured data, and images.

Mew treats MCP tools as dangerous and applies the existing
[permission modes and rules](/docs/using-mew/permissions/) to each call,
including calls from Python. In permissive mode, MCP calls still prompt.
`mew run` follows its existing noninteractive
behavior and automatically allows permission prompts. Cancellation is sent to
the server on a best-effort basis; a server may continue work after cancellation.
MCP calls time out after ten minutes. A cancelled stdio call causes Mew to
restart that server before a later call. Failed calls are not automatically
replayed, since they may have already changed external state.

Configured secret words are redacted from text and structured tool results.
Configure any sensitive values for redaction separately. MCP support exposes
tools but does not expose separate resource or prompt APIs. Embedded resources
and resource links returned by tools are included in their output. HTTP
authentication can be supplied through configured headers. Redirects are
followed only within the same origin.
Mew closes stateful HTTP sessions during normal shutdown; forced termination or
an unresponsive server can leave a session for the server to expire.

For protocol details, see the [MCP specification](https://modelcontextprotocol.io/).
