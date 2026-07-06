---
title: Hashline Edits
description: Line-anchored file edits with file-hash staleness detection.
---

Hashline is mew's line-anchored patch format. It lets the model edit files
using exact line numbers instead of fragile string replacement, and every
section is stamped with a short file hash so stale edits fail safely instead
of corrupting the wrong content.

The `read` tool produces hashline-formatted output automatically. The
`edit_hashline` tool consumes that output and applies the changes.

## Why hashline

String-based edits break when the target string appears more than once, when
whitespace changes, or when another edit shifts the file. Hashline avoids
those problems by:

- Anchoring every operation to a **line number**.
- Tagging each file section with a **content hash** so the patch is rejected
  if the file has drifted.
- Keeping a **session snapshot store** so mew can recover from in-session
  drift when the anchor content is still identifiable.
- Supporting **block-aware operations** for common languages, so the model can
  say "replace this function" without counting its closing brace.

## Read output format

`read` returns a `[path#hash]` header followed by numbered lines:

```
[src/lib.rs#A1B2]
1:pub fn add(a: i32, b: i32) -> i32 {
2:    a + b
3:}
4:
5:pub fn sub(a: i32, b: i32) -> i32 {
6:    a - b
7:}
```

- The hash (`A1B2`) is the first four hex digits of a stable xxHash32 of the
  normalized file content.
- Normalization strips trailing spaces/tabs/`\r` but preserves a trailing
  newline, so `file` and `file\n` hash differently.
- Line numbers are 1-indexed.

Use the hash and line numbers directly in an `edit_hashline` patch.

## Patch structure

A patch is one or more file sections. Each section starts with a header and
contains operations:

```
[src/lib.rs#A1B2]
SWAP 2.=2:
+    a + b + 1
DEL 5
INS.POST 4:
+pub fn mul(a: i32, b: i32) -> i32 {
+    a * b
+}
```

Operations are applied in order within a section. Multiple sections for the
same path are merged automatically.

## Line operations

### SWAP - replace a range

```
SWAP 2.=3:
+new line 2
+new line 3
```

Replaces lines 2 through 3 with the payload. `SWAP 2.=2:` replaces a single
line. Range separators can be `.=`, `..`, `-`, `…`, or `:` (e.g. `SWAP 2:5:`).

### DEL - delete a range

```
DEL 5
DEL 2.=4
```

Deletes lines 5, or lines 2 through 4. No payload body. Range separators
can be `.=`, `..`, `-`, `…`, or `:` (e.g. `DEL 2:4`).

### INS - insert lines

```
INS.PRE 3:
+line before line 3

INS.POST 3:
+line after line 3

INS.HEAD:
+lines at the top of the file

INS.TAIL:
+lines at the end of the file
```

### REM - delete the file

```
[src/lib.rs#A1B2]
REM
```

### MV - move/rename the file

```
[src/lib.rs#A1B2]
MV src/new_name.rs
```

## Block-aware operations

For Rust, TypeScript/JavaScript/JSX, Python, Go, and Markdown, you can target
the syntactic block that contains an anchor line instead of raw line numbers.

### SWAP.BLK - replace a block

```
[src/lib.rs#A1B2]
SWAP.BLK 5:
+fn new_fn() {}
```

Replaces the function/struct/impl/etc. whose first line is on or contains line
5.

### DEL.BLK - delete a block

```
DEL.BLK 5
```

### INS.BLK.POST - insert after a block

```
INS.BLK.POST 5:
+fn next_fn() {}
```

Inserts after the block that contains line 5. If tree-sitter cannot resolve
the block, this degrades to a plain `INS.POST 5` with a warning instead of
failing.

## Payload lines

Payload lines start with `+`. Blank lines inside a payload count as part of
the payload if they appear after the first `+` line. Raw lines that don't
start with `+` are also accepted as payload content, which makes pasting from
read output easier, but `+` is the canonical form.

`-old` rows (unified-diff style deletion markers) are ignored with a warning,
since the SWAP/DEL range already specifies what's being deleted. Don't rely
on this, but it won't break your patch.

If a raw payload line accidentally includes a read-output line-number prefix
like `12:content`, mew strips it and adds a warning.

## Staleness and recovery

If the live file no longer matches the section hash, mew first tries to
recover using the session snapshot store:

1. **Exact replay**: if the live file still matches the snapshot the hash
   refers to, the edits are applied directly.
2. **3-way merge**: otherwise, mew diffs the snapshot against the live file,
   remaps the anchor lines, and replays the edits.
3. **Hash mismatch error**: if recovery fails, the patch is rejected and no
   file is modified.

Recovery only works for in-session drift because snapshots are kept in
memory. If you restart mew or edit the file outside the session, you need to
`read` the file again to get a fresh hash.

## Seen-line validation

When `read` returns a sliced view (`offset`/`limit`), the snapshot records
which lines were actually displayed. `edit_hashline` rejects anchors on lines
that were not shown, preventing the model from editing content it never saw.

## Disabling hashline per provider

Some models don't follow line-numbered formats reliably. You can disable the
`edit_hashline` tool for a provider:

```toml
[providers.my-provider]
shape = "openai"
base_url = "https://api.example.com/v1"
credential_ref = "example"
disable_hashline = true
```

When hashline is disabled, the agent falls back to `edit` (string replace)
and `write`.

## Line endings and BOM

mew preserves the file's original line-ending style (`\n` or `\r\n`) and
whether it started with a UTF-8 BOM. Edits are applied on LF-normalized text,
then the original style is restored before writing.

## Common errors

| Error | Meaning |
|-------|---------|
| `hash mismatch for path: expected X, found Y` | The file changed since `read`. `read` it again. |
| `line N does not exist` | The anchor line is past the end of the file. |
| `invalid range: N.M ends before it starts` | The end line is smaller than the start line. |
| `unexpected trailing text: ...` | The operation has extra text after the range. Use `SWAP 2.=5:` for a range or `SWAP 2:` for a single line. |
| `block resolver unavailable` | `SWAP.BLK` or `DEL.BLK` could not resolve the syntax block. |
| `line N was not shown in the read that minted the tag` | The anchor is outside the `offset`/`limit` window the model saw. |
