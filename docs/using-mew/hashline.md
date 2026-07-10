---
title: Hashline Edits
description: How mew's line-anchored editing works and why stale edits fail safely.
---

Hashline is mew's line-anchored editing system. When the model edits a
file, it uses line numbers instead of fragile string matching — and every
edit is stamped with a file hash so stale edits fail safely instead of
corrupting the wrong content.

## What it does

- **Line-anchored edits**: the model references exact line numbers from
  `read` output, not string patterns that might match multiple locations.
- **Staleness detection**: every edit carries a hash of the file content
  at the time it was read. If the file changed since then, the edit is
  rejected — the model must `read` the file again.
- **Block-aware operations**: for supported languages (Rust, TypeScript,
  Python, Go, Markdown), the model can target an entire function or
  block without counting closing braces.
- **In-session recovery**: if the file drifted during the session (another
  edit shifted lines), mew can remap the anchors automatically.

## How the model uses it

1. The model calls `read` on a file. The output includes a `[path#hash]`
   header and numbered lines.
2. The model calls `edit_hashline` with line numbers from the `read`
   output and the hash.
3. If the hash matches the current file, the edit applies. If not, the
   edit is rejected with a clear error.

You don't need to learn the patch format — the model handles it
automatically. If you see a "hash mismatch" error, it means the file
changed and the model needs to re-read it.

## Disabling hashline

Some models don't follow line-numbered formats reliably. You can disable
hashline for a specific provider in your config:

```toml
[providers.my-provider]
shape = "openai"
base_url = "https://api.example.com/v1"
credential_ref = "example"
disable_hashline = true
```

When disabled, the agent falls back to string-replace edits and full-file
writes.

For the patch format syntax and internal architecture, see
[Hashline Internals](/docs/development/dev-hashline/).
