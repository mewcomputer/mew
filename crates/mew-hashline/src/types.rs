//! Internal types for parsed edits and patch results.

/// A 1-indexed line anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub line: usize,
}

/// Where to insert content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    BeforeAnchor { anchor: Anchor },
    AfterAnchor { anchor: Anchor },
    Bof,
    Eof,
}

/// A single edit produced by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Insert {
        cursor: Cursor,
        text: String,
        mode: InsertMode,
        /// Set when this insert came from a block op so the applier can
        /// correct the landing line if the payload indentation claims a
        /// different scope.
        block_start: Option<usize>,
    },
    Delete {
        anchor: Anchor,
    },
    /// A deferred block edit. Resolved to concrete inserts/deletes by the
    /// block resolver before applying.
    Block {
        anchor: Anchor,
        payloads: Vec<String>,
        mode: BlockMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertMode {
    Normal,
    Replacement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockMode {
    Replace,
    Delete,
    InsertAfter,
}

/// A file-level operation in a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOp {
    Rem,
    Move { dest: String },
}

/// Resolved span for a block op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockResolution {
    pub anchor_line: usize,
    pub start: usize,
    pub end: usize,
    pub op: BlockOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockOp {
    Replace,
    Delete,
    InsertAfter,
}

/// Resolves a block op to a concrete line span.
pub type BlockResolver = dyn Fn(&BlockResolverArgs) -> Option<BlockSpan> + Send + Sync;

pub struct BlockResolverArgs<'a> {
    pub path: &'a str,
    pub text: &'a str,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSpan {
    pub start: usize,
    pub end: usize,
}

/// Result of applying a list of edits to a text body.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyResult {
    pub text: String,
    pub first_changed_line: Option<usize>,
    pub warnings: Vec<String>,
    pub block_resolutions: Vec<BlockResolution>,
}
