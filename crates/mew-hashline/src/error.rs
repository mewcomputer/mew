use thiserror::Error;

pub type Result<T> = std::result::Result<T, HashlineError>;

#[derive(Debug, Error)]
pub enum HashlineError {
    #[error("parse error at line {line}: {message}")]
    Parse { line: usize, message: String },

    #[error("hash mismatch for {path}: expected {expected}, found {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("line {line} does not exist (file has {file_lines} lines)")]
    LineOutOfBounds { line: usize, file_lines: usize },

    #[error("invalid range: {start}.{end} ends before it starts")]
    InvalidRange { start: usize, end: usize },

    #[error("{0}")]
    Execution(String),

    #[error("block resolver unavailable or could not resolve block at line {line}")]
    BlockUnresolved { line: usize },

    #[error("recovery failed: {0}")]
    RecoveryFailed(String),
}

impl HashlineError {
    pub fn parse(line: usize, message: impl Into<String>) -> Self {
        Self::Parse {
            line,
            message: message.into(),
        }
    }

    pub fn execution(message: impl Into<String>) -> Self {
        Self::Execution(message.into())
    }
}
