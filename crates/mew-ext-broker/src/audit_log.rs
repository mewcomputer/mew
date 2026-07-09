//! Append-only gate audit log writer.
//!
//! Every gate hook decision (`on_tool_execute_before`, `on_permission_ask`)
//! writes a [`GateAuditEntry`](crate::audit::GateAuditEntry) to an append-only
//! JSONL file. Each extension gets its own file to avoid cross-extension
//! write contention.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::audit::GateAuditEntry;

/// Append-only audit log writer. Thread-safe via `std::sync::Mutex`.
///
/// Writes are short (single JSONL line + flush) and never held across
/// `.await`, so `std::sync::Mutex` is safe.
pub struct AuditLog {
    /// The directory containing per-extension audit files.
    dir: PathBuf,
    /// Inner writer state — mutex-protected. Each extension name maps to
    /// its own BufWriter. Lazily opened on first write for that extension.
    writers: Mutex<HashMap<String, BufWriter<File>>>,
}

use std::collections::HashMap;

impl AuditLog {
    /// Create an `AuditLog` that writes to `<dir>/<extension-name>.jsonl`.
    pub fn with_audit_dir(dir: PathBuf) -> Self {
        // Best-effort: create the directory if it doesn't exist.
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            writers: Mutex::new(HashMap::new()),
        }
    }

    /// Append a gate audit entry. Best-effort: errors are logged via
    /// `tracing::warn!`, not returned to the caller.
    pub fn log(&self, entry: GateAuditEntry) {
        let ext_name = entry.extension.clone();
        let path = self
            .dir
            .join(format!("{}.jsonl", sanitize_filename(&ext_name)));

        let json = match serde_json::to_string(&entry) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("failed to serialize audit entry: {}", e);
                return;
            }
        };

        let mut writers = match self.writers.lock() {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("audit log mutex poisoned: {}", e);
                return;
            }
        };

        // Get or create the writer for this extension.
        let writer = writers.entry(ext_name.clone()).or_insert_with(|| {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to open audit file {:?}: {}", path, e);
                    // Create a dummy file to /dev/null equivalent — we can't
                    // return a default BufWriter easily, so panic-free fallback.
                    // In practice this only fails if the dir is unwritable.
                    File::create("/dev/null").unwrap_or_else(|_| {
                        // Last resort: open the original path for reading (will fail on write).
                        OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                            .unwrap()
                    })
                });
            BufWriter::new(file)
        });

        if let Err(e) = writeln!(writer, "{}", json) {
            tracing::warn!("failed to write audit entry: {}", e);
        }
        if let Err(e) = writer.flush() {
            tracing::warn!("failed to flush audit log: {}", e);
        }
    }

    /// Read all audit entries for a given extension.
    /// Opens the file fresh — safe because append-mode writes and reads
    /// use different file handles.
    pub fn read_all(&self, extension_name: &str) -> Vec<GateAuditEntry> {
        let path = self
            .dir
            .join(format!("{}.jsonl", sanitize_filename(extension_name)));

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        content
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|line| serde_json::from_str::<GateAuditEntry>(line).ok())
            .collect()
    }
}

/// Sanitize a filename — replace path separators and special chars.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{GateAuditEntry, GateOutcome};

    #[test]
    fn test_audit_log_write_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::with_audit_dir(dir.path().to_path_buf());

        let entry = GateAuditEntry::new(
            "test-ext",
            "session-1",
            "bash",
            "sha256:abc123",
            GateOutcome::Block,
        );
        log.log(entry);

        let entries = log.read_all("test-ext");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].extension, "test-ext");
        assert_eq!(entries[0].tool, "bash");
        assert_eq!(entries[0].outcome, GateOutcome::Block);
    }

    #[test]
    fn test_audit_log_multiple_entries() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::with_audit_dir(dir.path().to_path_buf());

        log.log(GateAuditEntry::new(
            "ext-a",
            "sess-1",
            "bash",
            "hash1",
            GateOutcome::Proceed,
        ));
        log.log(GateAuditEntry::new(
            "ext-a",
            "sess-1",
            "write",
            "hash2",
            GateOutcome::Block,
        ));
        log.log(GateAuditEntry::new(
            "ext-b",
            "sess-1",
            "bash",
            "hash3",
            GateOutcome::Mutated,
        ));

        let a_entries = log.read_all("ext-a");
        assert_eq!(a_entries.len(), 2);

        let b_entries = log.read_all("ext-b");
        assert_eq!(b_entries.len(), 1);
        assert_eq!(b_entries[0].outcome, GateOutcome::Mutated);
    }

    #[test]
    fn test_read_nonexistent_extension() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::with_audit_dir(dir.path().to_path_buf());
        let entries = log.read_all("nonexistent");
        assert!(entries.is_empty());
    }
}
