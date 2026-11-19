---
title: Hashline Edits
description: How mew's line-anchored editing works and why stale edits fail safely.
---

Hashline lets the model edit a file using numbered lines and a content hash.
The hash identifies the version the model read.

1. The model calls `read`. The output includes a `[path#hash]` header and
   numbered lines.
2. The model calls `edit_hashline` with that header and the lines to change.
   In supported languages it can also target a whole function or block.
3. Mew checks the hash before applying the edit. If the file changed and
   Mew has the earlier version in memory, it tries to remap the edit to
   the current file. If it can't recover, it returns a hash mismatch and
   the model should read the file again.

## Disabling hashline

Set `disable_hashline = true` to remove `edit_hashline` from a provider’s tool set.
The string-replace and full-file write tools remain available.

```toml
[providers.my-provider]
shape = "openai"
base_url = "https://api.example.com/v1"
credential_ref = "example"
disable_hashline = true
```

For the patch format syntax and internal architecture, see
[Hashline Internals](/docs/development/dev-hashline/).
