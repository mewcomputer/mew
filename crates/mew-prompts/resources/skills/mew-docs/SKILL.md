---
name: mew-docs
description: Find and fetch mew documentation on demand. Provides a sitemap of docs at mew.computer so you can web_fetch the right page instead of guessing from memory.
---

# mew Documentation

This skill helps you understand and modify mew. It provides a sitemap of
the documentation at `https://mew.computer/docs` so you can find the right
page with `web_fetch` rather than guessing from memory.

## How to use this skill

Each entry in the sitemap below includes its full URL. When you need to
understand a mew-specific detail, use the `web_fetch` tool to fetch the
relevant page, read it, and then proceed. Prefer fetching the
documentation page over relying on your memory for mew-specific behavior,
keys, or conventions. Fetch only the page that covers your task; the
descriptions below help you pick the right one without fetching multiple
pages.

## Documentation sitemap

### Getting Started

- [Installation](https://mew.computer/docs/installation/) — Installing mew (cargo, from source, install recipes).
- [Quick Start](https://mew.computer/docs/quick-start/) — Getting a first session running, TUI layout, cost review.
- [Configuration](https://mew.computer/docs/configuration/) — Full config.toml field reference, env vars, credential resolution, state.toml.
- [Context Files](https://mew.computer/docs/context-files/) — AGENTS.md / CLAUDE.md loading, @-includes, templating.
- [Sessions](https://mew.computer/docs/sessions/) — Session lifecycle, storage format, /sessions /resume /rewind /clear /compact.

### Using mew

- [Slash Commands](https://mew.computer/docs/slash-commands/) — All slash commands (/persona, /model, /thinking, /theme, etc.).
- [Keyboard Shortcuts](https://mew.computer/docs/keyboard-shortcuts/) — Keybindings reference.
- [Tips & Tricks](https://mew.computer/docs/tips-and-tricks/) — Prompting patterns, workflow guidance.
- [Providers](https://mew.computer/docs/providers/) — Provider setup, credentials, router, thinking variants, catalog.
- [Permissions](https://mew.computer/docs/permissions/) — Permission modes, declarative rules, workspace sandboxing, auto classifier.
- [Tools](https://mew.computer/docs/tools/) — Built-in tool reference (bash, read, write, edit, grep, glob, etc.).
- [Personas](https://mew.computer/docs/personas/) — Persona authoring, frontmatter, model pinning, transitions, autonomous hints, accent colors.
- [Skills](https://mew.computer/docs/skills/) — Skill authoring, frontmatter, templating, when to use a skill vs context file.
- [Subagents](https://mew.computer/docs/subagents/) — Subagent definitions, async vs blocking, nesting.
- [Plugins](https://mew.computer/docs/plugins/) — Plugin authoring, JSON-RPC protocol, host functions.
- [MCP Servers](https://mew.computer/docs/mcp-servers/) — MCP server configuration, common servers, troubleshooting.
- [Web UI](https://mew.computer/docs/web-ui/) — Web UI features, connection lifecycle, reconnection.

### Development

- [Architecture](https://mew.computer/docs/dev-architecture/) — Three-layer pipeline (TUI → Agent → Provider), event flow, message model.
- [Adding a Provider](https://mew.computer/docs/dev-providers/) — Implementing the Provider trait, adapter shapes.
- [Adding a Tool](https://mew.computer/docs/dev-tools/) — Tool trait, sensitivity, registration.
- [Daemon Protocol](https://mew.computer/docs/dev-protocol/) — Wire message types, JSON codec.
- [Testing](https://mew.computer/docs/dev-testing/) — Test conventions, CI gate.
- [Web UI Development](https://mew.computer/docs/dev-web/) — Frontend stack, build, e2e.
