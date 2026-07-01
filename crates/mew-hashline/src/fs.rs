use async_trait::async_trait;

/// Minimal filesystem seam used by the hashline patcher.
///
/// Callers can back this with tokio fs, an in-memory map for tests, or any
/// other storage. The patcher handles its own BOM stripping and line-ending
/// normalization; implementations deal in raw text strings.
#[async_trait]
pub trait HashlineFs: Send + Sync {
    /// Read the full text content of `path`. Must return an error for which
    /// `is_not_found` is true when the file does not exist.
    async fn read_text(&self, path: &str) -> std::io::Result<String>;

    /// Write `content` to `path`.
    async fn write_text(&self, path: &str, content: &str) -> std::io::Result<()>;

    /// Delete the file at `path`.
    async fn delete(&self, path: &str) -> std::io::Result<()>;

    /// Rename/move `from` to `to`.
    async fn rename(&self, from: &str, to: &str) -> std::io::Result<()>;

    /// Return a canonical form of `path` for snapshot-store keys.
    fn canonical_path(&self, path: &str) -> String;

    /// Return true if `path` exists.
    async fn exists(&self, path: &str) -> bool {
        match self.read_text(path).await {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                // Surface unexpected read errors rather than silently returning false.
                let _ = e;
                false
            }
        }
    }
}

/// Convenience guard for not-found errors.
pub fn is_not_found(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::NotFound
}
