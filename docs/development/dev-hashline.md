---
title: Hashline Internals
description: Architecture of the mew-hashline crate and edit_hashline tool.
---

`crates/mew-hashline` implements line-anchored file edits, wrapped by the
`edit_hashline` tool in `crates/mew-tools`.

## Crate map

| Module | Purpose |
|--------|---------|
| `types` | Shared public edit, operation, warning, and result types |
| `format` | Hash computation, LF/BOM normalization, line-ending detection, header formatting |
| `tokenizer` | Classifies patch text into `LineToken`s |
| `parser` | Converts tokens into `Edit` items and optional `FileOp`s |
| `patch` | Splits tokens into `PatchSection`s and merges sections sharing a path |
| `patcher` | Preflights, validates, recovers, and commits edits through `HashlineFs` |
| `apply` | Applies concrete edits bottom-up to LF-normalized text |
| `block` | Tree-sitter syntactic block resolution |
| `snapshot` | In-memory file version history keyed by content hash |
| `recovery` | 3-way-merge recovery for stale content hashes |
| `fs` | Minimal async filesystem trait |
| `error` | `HashlineError` variants |

## Patch pipeline

```text
patch text
   |
   v
tokenizer -> LineToken[]
   |
   v
patch splitter -> PatchSection[]
   |
   v
parser -> Vec<Edit> + Option<FileOp> + warnings
   |
   v
block resolver -> concrete edits
   |
   v
patcher prepare -> path recovery / hash check / seen-line guard / apply_edits
   |
   v
commit -> HashlineFs writes, deletes, and renames
```

`Patcher` preflights every section in memory before committing changes. If a
later I/O call fails during commit, earlier commits remain on disk.

## Tokenizer, parser, and patch sections

`tokenizer::tokenize` classifies each line as `Blank`, `Header { path, hash }`,
`Op { target }`, `PayloadLiteral { text }` (`+`-prefixed payload), or `Raw { text }`.
Range separators accept `.=`, `..`, `-`, and `…`, and dotted inserts accept both
`INS.POST 1:` and `INS .POST 1:`.

`parser::parse_section` turns section tokens into ordered `Edit` values plus an
optional `FileOp::Rem` or `FileOp::Move`. `SWAP` decomposes into replacement
`Insert`s followed by `Delete`s over the original lines; block ops become
`Edit::Block` nodes resolved later. The parser rejects unified-diff hunk
headers, `apply_patch` markers, and `-old` lines, and strips accidental `12:`
line-number prefixes from raw payload lines with a warning.

`patch::Patch::parse` splits sections at `[path#hash]` headers and merges
repeated sections targeting the same file, rejecting conflicting hashes or
duplicate file-level operations.

## Patcher and apply engine

`patcher::Patcher` holds a `SnapshotStore` and an optional `BlockResolver`.
`Patcher::apply` runs three steps:

1. **Prepare all sections in memory**: read the target file, recover relative
   path mismatches if needed, validate the hash (or recover via 3-way merge),
   resolve block ops, check seen-line bounds, and run `apply_edits`.
2. **Reject no-ops**: fail the entire patch if any section produces identical text.
3. **Commit**: write modified files and apply deletes and renames.

`apply::apply_edits` buckets edits by anchor line and applies them bottom-up so
earlier line numbers remain valid. `Insert` supports `BeforeAnchor`,
`AfterAnchor`, `Bof`, and `Eof`, preserving trailing-newline conventions and
empty-file semantics.

### Hash validation and recovery

If the authored path is missing on disk and its hash uniquely matches a
snapshot recorded for another path with the same filename, `Patcher` switches
to that path.

When the live file differs from the section hash, `recovery::try_recover`
attempts 3-way-merge recovery from `SnapshotStore`:

1. Replay edits directly if the live file matches the recorded snapshot.
2. Compute a line-level LCS diff between the snapshot and the live file via `similar`.
3. Remap surviving anchor lines (mapping deleted anchors to the nearest
   surviving line, or skipping deleted anchors for deletes) while keeping
   BOF/EOF inserts.
4. Reject the patch with `HashlineError::HashMismatch` if no anchors survive or
   if the result equals the live text; otherwise emit a warning listing the
   remapped anchor count.

### Seen-line validation

When `read` runs with `offset` and `limit`, `SnapshotStore` records `seen_lines`.
`Patcher` rejects edits anchored on lines outside that set so the model only
edits lines it read.

## Block resolver

`block::default_block_resolver` uses tree-sitter grammars for:

- Rust (`.rs`)
- TypeScript / TSX / JavaScript / JSX (`.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`)
- Python (`.py`)
- Go (`.go`)
- Markdown (`.md`, `.markdown`)
- Gleam (`.gleam`)

Given an anchor line, the resolver selects the deepest syntax node starting on
that line and spanning multiple lines. `INS.BLK.POST` anchors after the node's
end line (falling back to `INS.POST` with a warning when unresolved);
`SWAP.BLK` and `DEL.BLK` target the full node span and error when unresolved.

To add a language:

1. Add its tree-sitter grammar crate to `crates/mew-hashline/Cargo.toml`.
2. Import its `LANGUAGE` constant in `crates/mew-hashline/src/block.rs`.
3. Add its extensions to `build_language_table`.
4. Add a unit test in `block.rs`.

## Snapshot store

`snapshot::InMemorySnapshotStore` retains up to four versions per path in
`ToolCtxShared`, shared across `read` and `edit_hashline`:

- `record(path, text, seen_lines)`: stores a snapshot (returns `()`).
- `by_hash(path, hash)`: fetches a version by path and tag.
- `find_by_hash(hash)`: finds all paths matching a tag (for path recovery).
- `invalidate(path)` / `relocate(from, to)`: updates history on deletes and moves.

## Filesystem trait

`fs::HashlineFs` abstracts disk access (`mew-tools` implements it with
`tokio::fs`; tests use an in-memory map):

```rust
#[async_trait]
pub trait HashlineFs: Send + Sync {
    async fn read_text(&self, path: &str) -> std::io::Result<String>;
    async fn write_text(&self, path: &str, content: &str) -> std::io::Result<()>;
    async fn delete(&self, path: &str) -> std::io::Result<()>;
    async fn rename(&self, from: &str, to: &str) -> std::io::Result<()>;
    fn canonical_path(&self, path: &str) -> String;
    async fn exists(&self, path: &str) -> bool { ... }
}
```

Run crate unit tests with:

```sh
cargo test -p mew-hashline
```

## Patch format reference

`read` prepends `[path#hash]` (low 16 bits of xxHash32 after stripping `\r` and
trailing horizontal whitespace while preserving a final newline) and 1-indexed
line numbers:

```text
[src/lib.rs#A1B2]
1:pub fn add(a: i32, b: i32) -> i32 {
2:    a + b
3:}
```

`edit_hashline` accepts one or more `[path#hash]` sections:

```text
[src/lib.rs#A1B2]
SWAP 2.=2:
+    a + b + 1
DEL 5
INS.POST 4:
+pub fn mul(a: i32, b: i32) -> i32 {
+    a * b
+}
```

### Line and block operations

| Operation | Syntax | Description |
|-----------|--------|-------------|
| `SWAP` | `SWAP 2.=3:` | Replace lines 2 to 3 (`2.=2:` replaces one line) |
| `DEL` | `DEL 5` or `DEL 2.=4` | Delete a line or range (no payload) |
| `INS.PRE` | `INS.PRE 3:` | Insert before line 3 |
| `INS.POST` | `INS.POST 3:` | Insert after line 3 |
| `INS.HEAD` | `INS.HEAD:` | Insert at top of file |
| `INS.TAIL` | `INS.TAIL:` | Insert at end of file |
| `SWAP.BLK` | `SWAP.BLK 5:` | Replace the syntactic block starting at line 5 |
| `DEL.BLK` | `DEL.BLK 5` | Delete the syntactic block starting at line 5 |
| `INS.BLK.POST` | `INS.BLK.POST 5:` | Insert after the syntactic block at line 5 (falls back to `INS.POST`) |
| `REM` | `REM` | Delete the file |
| `MV` | `MV src/new_name.rs` | Rename the file |

Payload lines start with `+` (blank lines after the first `+` count as blank
payload lines; unprefixed lines are tolerated for convenience).

### Common errors

| Error | Meaning |
|-------|---------|
| `hash mismatch for path: expected X, found Y` | File changed since `read` and recovery could not remap |
| `line N does not exist (file has M lines)` | Anchor exceeds file length |
| `invalid range: N.M ends before it starts` | Inverted line range |
| `block resolver unavailable or could not resolve block at line N` | `SWAP.BLK` or `DEL.BLK` could not resolve a block |
| `line N was not shown in the read that minted the tag` | Anchor falls outside `offset` / `limit` window |
