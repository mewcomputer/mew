//! mew-hashline: a line-anchored patch format with file-hash staleness
//! detection, snapshot-store recovery, and optional tree-sitter block ops.
//!
//! This crate is the implementation backing the `edit_hashline` tool in
//! `mew-tools`. It is intentionally independent of any filesystem backend;
//! callers provide a minimal `HashlineFs` trait implementation.

pub mod apply;
pub mod block;
pub mod error;
pub mod format;
pub mod fs;
pub mod parser;
pub mod patch;
pub mod patcher;
pub mod recovery;
pub mod snapshot;
pub mod tokenizer;
pub mod types;

pub use error::{HashlineError, Result};
pub use format::{compute_file_hash, format_hashline_header};
pub use fs::HashlineFs;
pub use patch::Patch;
pub use patcher::{PatchSectionResult, Patcher, PatcherOptions};
pub use snapshot::{InMemorySnapshotStore, SnapshotStore};
