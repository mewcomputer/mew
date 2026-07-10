---
title: Hashline Internals
description: Architecture of the mew-hashline crate and edit_hashline tool.
---

The hashline implementation lives in `crates/mew-hashline` and is wrapped by
the `edit_hashline` tool in `crates/mew-tools`. This page walks through the
parser, patcher, recovery, and how to extend it.

## Crate map

| Module | Purpose |
|--------|---------|
| `format` | Hash computation, LF/BOM normalization, line-ending detection, header formatting |
| `tokenizer` | Classifies patch text into `LineToken`s |
| `parser` | Turns tokens into a flat list of `Edit`s and optional `FileOp`s |
| `patch` | Splits tokens into `PatchSection`s and merges same-path sections |
| `patcher` | Preflights, validates, recovers, and commits edits to a `HashlineFs` |
| `apply` | Applies concrete edits to LF-normalized text |
| `block` | Tree-sitter-backed block resolution |
| `snapshot` | In-memory version history keyed by content hash |
| `recovery` | 3-way-merge recovery for stale tags |
| `fs` | Minimal async filesystem trait used by the patcher |
| `error` | `HashlineError` enum |

## The patch lifecycle

```
patch text
   │
   ▼
tokenizer → LineToken[]
   │
   ▼
patch splitter → PatchSection[]
   │
   ▼
parser → Vec<Edit> + Option<FileOp> + warnings
   │
   ▼
block resolver → concrete edits (block ops expanded)
   │
   ▼
patcher prepare → hash check / recovery / seen-line guard
   │
   ▼
apply_edits → new text
   │
   ▼
commit → HashlineFs.write_text
```

All-or-nothing behavior is implemented at the section level: every section is
prepared in memory before any write happens.

## Tokenizer

`tokenizer::tokenize` walks patch text line by line and classifies each line
as one of:

- `Blank`
- `Header { path, hash }` - `[path#hash]`
- `Op { target }` - `SWAP`, `DEL`, `INS.*`, `SWAP.BLK`, `DEL.BLK`, `REM`, `MV`, etc.
- `PayloadLiteral { text }` - lines starting with `+`
- `Raw { text }` - everything else

Headers tolerate whitespace before/after the brackets. The hash is optional
but required by the patcher. Operation keywords accept several range
separators (`.=`, `..`, `-`, `…`) and both dotted insert forms (`INS.POST 1:`
and `INS .POST 1:`).

## Parser

`parser::parse_section` consumes the body tokens for one section and produces:

- A `Vec<Edit>` in the order they should be applied.
- An optional `FileOp::Rem` or `FileOp::Move`.
- A list of warnings.

`SWAP` is decomposed into replacement `Insert`s followed by `Delete`s for the
old range. `INS.*` becomes one or more `Insert`s. `Block` operations become
`Edit::Block` nodes that are resolved later.

The parser also detects common contamination (unified-diff hunk headers,
`apply_patch` sentinels, `-old` payload rows) and rejects them with a clear
error.

## Patch sections

`patch::Patch::parse` splits the token stream on headers, calls
`parse_section` for each body, then merges sections that target the same path.
Conflicting hashes or multiple file-level ops for the same path are rejected.

## Patcher

`patcher::Patcher` is the only component that touches the filesystem. It is
constructed with:

- A `SnapshotStore` for recovery and seen-line tracking.
- An optional `BlockResolver` (tree-sitter by default in the tool wrapper).

`Patcher::apply` does three things:

1. **Prepare every section in memory.** This reads the file, checks the hash,
   resolves block ops, applies edits in memory, and validates that anchors are
   within seen lines.
2. **Reject no-ops.** If any section produces no change, the whole patch
   fails so the model doesn't think it changed something it didn't.
3. **Commit.** Only after all sections prepare successfully does it write
   files, delete files, or move files.

### Hash validation

The patcher's `prepare` step compares the section hash against the current
file hash. If they match, edits proceed normally. If not, it tries recovery.

### Path recovery

If the authored path doesn't exist but the hash matches a single snapshot for
a different path with the same filename, the patcher switches to that path.
This handles cases where the model uses a relative path that differs from the
one `read` recorded.

### Seen-line guard

When a snapshot records `seen_lines`, every anchor in the section must fall
inside that set. This prevents edits anchored on lines the model never saw,
which is common when `read` was called with `offset`/`limit`.

## Apply engine

`apply::apply_edits` takes LF-normalized text and a list of concrete edits.

- Edits are bucketed by anchor line and applied **bottom-up** so line numbers
  stay stable.
- `Insert` cursors can be `BeforeAnchor`, `AfterAnchor`, `Bof`, or `Eof`.
- Replacement inserts are combined with deletes to replace a range in place.
- BOF/EOF inserts handle empty files and trailing-newline preservation.

The function returns the new text, the first changed line, and any warnings.

## Block resolver

`block::default_block_resolver` builds a tree-sitter-backed resolver for:

- Rust (`.rs`)
- TypeScript/TSX/JavaScript/JSX (`.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`)
- Python (`.py`)
- Go (`.go`)
- Markdown (`.md`, `.markdown`)

For a given anchor line, it walks the parse tree and picks the deepest node
that starts on that line and spans more than one line. `INS.BLK.POST` uses the
node's end line as the insert anchor; `SWAP.BLK` and `DEL.BLK` delete the full
node span.

If the resolver returns `None`:

- `SWAP.BLK` and `DEL.BLK` error.
- `INS.BLK.POST` degrades to `INS.POST` with a warning.

To add a language, add its tree-sitter `Language` and extension mapping to
`block::build_language_table`.

## Snapshot store

`snapshot::InMemorySnapshotStore` keeps the last few versions of each path,
keyed by content hash. It is shared across all tool calls in a session via
`ToolCtxShared`, so `read`, `edit_hashline`, and any other hashline-aware tool
see the same history.

- `record(path, text, seen_lines)` stores a version and returns its hash.
- `by_hash(path, hash)` looks up a specific version.
- `find_by_hash(hash)` finds all paths that ever had a given hash (used for
  path recovery).
- `invalidate(path)` and `relocate(from, to)` handle deletes and moves.

The default history limit is four versions per path.

## Recovery

`recovery::try_recover` handles stale tags.

1. If the live text exactly matches the snapshot, it applies the edits
   directly.
2. Otherwise it builds a line-level mapping between the snapshot and the live
   text using `similar`'s LCS diff.
3. It remaps anchor lines. Lines that were deleted map to the nearest
   surviving line (or are skipped for deletes). BOF/EOF inserts are preserved.
4. The remapped edits are applied to the live text.

If the result is identical to the live text or no anchors survive, recovery
fails and the patcher returns a hash mismatch error. On success, a warning
explains that the patch was recovered by 3-way merge and how many anchors were
remapped.

## Filesystem trait

`fs::HashlineFs` is intentionally tiny:

```rust
#[async_trait]
pub trait HashlineFs: Send + Sync {
    async fn read_text(&self, path: &str) -> std::io::Result<String>;
    async fn write_text(&self, path: &str, content: &str) -> std::io::Result<()>;
    async fn delete(&self, path: &str) -> std::io::Result<()>;
    async fn rename(&self, from: &str, to: &str) -> std::io::Result<()>;
    fn canonical_path(&self, path: &str) -> String;
}
```

`mew-tools` implements this with direct tokio fs. Tests implement it with
in-memory maps. Keeping the trait small lets the crate stay independent of any
filesystem backend.

## Testing

Unit tests cover each layer:

- `format`: hash stability, normalization rules, header formatting.
- `tokenizer`: header parsing, operation classification, invalid ranges.
- `parser`: payload handling, `REM`/`MV`, bare-row prefix stripping.
- `patch`: multi-section parsing, same-path merging.
- `apply`: single-line replacement, deletion, EOF insertion, bounds checking.
- `block`: Rust function resolution, single-line block rejection, unknown
  extension handling, `INS.BLK.POST` degradation.
- `recovery`: exact replay, drifted anchors, deleted anchors, multi-edit
  remapping.
- `snapshot`: record/head, seen-line merging, history bound, relocation.

Run them with:

```bash
cargo test -p mew-hashline
```

## Adding a language to block ops

1. Add the tree-sitter crate to `crates/mew-hashline/Cargo.toml`.
2. Import its `LANGUAGE` constant in `crates/mew-hashline/src/block.rs`.
3. Add the extension mapping to `build_language_table`.
4. Add a unit test in `block.rs` covering a realistic construct.

No other module needs to change.

## Patch format reference

The hashline patch format is what the `edit_hashline` tool consumes. The
`read` tool produces hashline-formatted output automatically; the model
uses the line numbers and hash from `read` output to construct patches.

### Read output format

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

- The hash (`A1B2`) is the first four hex digits of a stable xxHash32 of
  the normalized file content.
- Normalization strips trailing spaces/tabs/`\r` but preserves a
  trailing newline.
- Line numbers are 1-indexed.

### Patch structure

A patch is one or more file sections. Each section starts with a header
and contains operations:

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

Operations are applied in order within a section. Multiple sections for
the same path are merged automatically.

### Line operations

| Operation | Syntax | Description |
|-----------|--------|-------------|
| SWAP | `SWAP 2.=3:` | Replace lines 2–3 with payload. `2.=2:` replaces one line. Separators: `.=`, `..`, `-`, `…` |
| DEL | `DEL 5` or `DEL 2.=4` | Delete a line or range. No payload. |
| INS.PRE | `INS.PRE 3:` | Insert before line 3. |
| INS.POST | `INS.POST 3:` | Insert after line 3. |
| INS.HEAD | `INS.HEAD:` | Insert at top of file. |
| INS.TAIL | `INS.TAIL:` | Insert at end of file. |
| REM | `REM` | Delete the file. |
| MV | `MV src/new_name.rs` | Rename the file. |

### Block-aware operations

For Rust, TypeScript/JavaScript/JSX, Python, Go, and Markdown, block
ops target the syntactic block containing an anchor line:

| Operation | Syntax | Description |
|-----------|--------|-------------|
| SWAP.BLK | `SWAP.BLK 5:` | Replace the block containing line 5. |
| DEL.BLK | `DEL.BLK 5` | Delete the block containing line 5. |
| INS.BLK.POST | `INS.BLK.POST 5:` | Insert after the block containing line 5. Degrades to `INS.POST` if tree-sitter can't resolve. |

### Payload rules

- Payload lines start with `+`.
- Blank lines inside a payload count as part of the payload if they
  appear after the first `+` line.
- Raw lines (without `+`) are accepted as payload for paste
  convenience, but `+` is canonical.
- If a raw payload line accidentally includes a read-output line-number
  prefix like `12:content`, mew strips it and adds a warning.

### Staleness and recovery

If the live file no longer matches the section hash:

1. **Exact replay**: if the live file matches the snapshot, apply
   directly.
2. **3-way merge**: diff snapshot against live file, remap anchors,
   replay edits.
3. **Hash mismatch error**: if recovery fails, the patch is rejected and
   no file is modified.

Recovery only works for in-session drift (snapshots are in-memory). If
mew is restarted or the file is edited externally, `read` the file again
for a fresh hash.

### Seen-line validation

When `read` returns a sliced view (`offset`/`limit`), the snapshot
records which lines were displayed. `edit_hashline` rejects anchors on
lines that were not shown, preventing edits to content the model never
saw.

### Common errors

| Error | Meaning |
|-------|---------|
| `hash mismatch for path: expected X, found Y` | File changed since `read`. `read` it again. |
| `line N does not exist` | Anchor line is past end of file. |
| `invalid range: N.M ends before it starts` | End line < start line. |
| `block resolver unavailable` | `SWAP.BLK`/`DEL.BLK` couldn't resolve syntax block. |
| `line N was not shown in the read that minted the tag` | Anchor outside the `offset`/`limit` window. |
