---
title: Hashline Edits
description: How Mew's line-anchored editing works and why stale edits fail safely.
---

Hashline lets the model edit files using numbered lines and a content hash that
pins the version it read.

1. The model calls `read`, which returns numbered lines with a `[path#hash]`
   header.
2. The model calls `edit_hashline` with that header and the line anchors to
   change. In supported languages (`Rust`, `TypeScript`, `Python`, `Go`,
   `Markdown`, and `Gleam`), it can also target a named function or block.
3. Mew verifies the hash before writing. If the file changed since the read and
   Mew still holds the original snapshot in memory, it remaps the edit to the
   current lines. When recovery fails, Mew returns a hash mismatch error so the
   model re-reads the file.

Example patch payload:

```text
@@ 12:a1b2..15:c3d4
- old line
+ new line
```

## Disabling hashline

Set `disable_hashline = true` on a provider to omit `edit_hashline` from its
tool registry while keeping `edit_str_replace` and `write`:

```toml
[providers.my-provider]
shape = "openai"
base_url = "https://api.example.com/v1"
credential_ref = "example"
disable_hashline = true
```

See [Hashline Internals](/docs/development/dev-hashline/) for the complete patch
syntax, recovery pipeline, and tree-sitter block resolver.
