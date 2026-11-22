---
title: MCP Servers
description: Configure Model Context Protocol tools in Mew.
---

Mew discovers tools from Model Context Protocol servers and registers them for
both direct tool mode and Python code mode. Configure servers in `mcp.json`
beside the `config.toml` printed by `mew config path`, or in your project:

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

A `url` entry selects streamable HTTP; a `command` entry selects stdio. Set
`type` to `"http"` or `"stdio"` explicitly when needed. HTTP servers accept a
`headers` object, and stdio servers accept an `env` object alongside `args`.
Config values are literal strings (Mew does not expand environment variables in
`mcp.json`). Stdio servers inherit the daemon's environment and working
directory.

## Discovery and startup

- **Search order**: Mew loads the global `mcp.json` and the nearest project
  `mcp.json` between the startup directory and the Git root (or the startup
  directory outside Git). Project entries override global entries with the same
  name; set `"disabled": true` to disable an inherited server.
- **Startup timing**: the daemon discovers servers once at startup and shares
  them across sessions, so restart the daemon after editing `mcp.json`.
  `mew run` discovers servers at the start of each run.
- **Timeouts**: servers that fail to launch or fail discovery within 15 seconds
  are skipped with a warning, and server stderr is suppressed. TUI and desktop
  launchers wait up to 30 seconds for daemon startup, including MCP discovery.
- **Trust boundary**: Mew starts configured stdio servers during discovery
  before any tool permission prompt runs. Configure only trusted servers. MCP
  servers execute outside the Python sandbox with full access to their process's
  files, network, and environment.

## Using tools and permissions

Mew names each discovered tool `mcp__<server>__<tool>` (for example,
`mcp__files__read_file`), normalized for model providers. Duplicate names and
names exceeding 64 characters are skipped. In Python code mode, inspect
`ALL_TOOLS` to check available names.

- **Capabilities**: MCP tools return text, structured data, and (when the active
  model supports vision) images. Embedded resources and resource links in tool
  output are included inline. Mew exposes MCP tools only, omitting separate
  resource and prompt endpoints.
- **Permissions**: Mew classifies all MCP tools as `Dangerous` and enforces
  [permission modes and rules](/docs/using-mew/permissions/) on direct and
  Python-nested calls. Even in `permissive` mode, MCP calls prompt unless an
  allow rule matches; `mew run` grants permission prompts automatically unless
  blocked by a `deny` rule.
- **Timeouts and retries**: MCP calls time out after 10 minutes. Cancelling an
  in-flight call sends a best-effort cancellation notice; cancelled stdio calls
  restart the server before the next call. Failed calls are never replayed
  automatically.
- **HTTP and redaction**: supply HTTP credentials through `headers`; Mew follows
  redirects only within the same origin and closes stateful HTTP sessions on
  clean shutdown. Values listed under `[[secrets.words]]` are redacted from text
  and structured MCP results.

See the [MCP specification](https://modelcontextprotocol.io/) for protocol
details.
