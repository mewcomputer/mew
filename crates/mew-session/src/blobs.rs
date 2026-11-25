use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::session_dir;

pub const BLOB_URL_PREFIX: &str = "mew-blob://";
pub const MAX_BLOB_BYTES: usize = 10 * 1024 * 1024;

/// Content-addressed blob storage under `<session_dir>/blobs/<sha256>`.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn default_store() -> Self {
        Self::new(session_dir().join("blobs"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn put(&self, bytes: &[u8]) -> io::Result<String> {
        if bytes.len() > MAX_BLOB_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("blob exceeds {MAX_BLOB_BYTES} byte limit"),
            ));
        }

        let digest = Sha256::digest(bytes);
        let id = hex_lower(&digest);
        let path = self.root.join(&id);
        if path.exists() {
            return Ok(id);
        }

        fs::create_dir_all(&self.root)?;
        let mut tmp = NamedTempFile::new_in(&self.root)?;
        tmp.write_all(bytes)?;
        tmp.flush()?;
        match tmp.persist_noclobber(&path) {
            Ok(_) => Ok(id),
            Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => Ok(id),
            Err(err) => Err(err.error),
        }
    }

    pub fn get(&self, id: &str) -> io::Result<Vec<u8>> {
        if !is_valid_blob_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid blob id",
            ));
        }
        fs::read(self.root.join(id))
    }
}

pub fn is_valid_blob_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn parse_blob_id(url: &str) -> Option<&str> {
    let rest = url.strip_prefix(BLOB_URL_PREFIX)?;
    is_valid_blob_id(rest).then_some(rest)
}

pub fn blob_url(id: &str) -> String {
    format!("{BLOB_URL_PREFIX}{id}")
}

pub fn decode_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()?;
    Some((mime.to_string(), bytes))
}

pub fn encode_data_url(mime: &str, bytes: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_and_deduplicate() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        let data = b"hello attachment";

        let id1 = store.put(data).unwrap();
        let id2 = store.put(data).unwrap();
        assert_eq!(id1, id2);
        assert!(is_valid_blob_id(&id1));
        assert_eq!(store.get(&id1).unwrap(), data);
    }

    #[test]
    fn rejects_invalid_ids_and_oversized_blobs() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());

        assert!(!is_valid_blob_id("short"));
        assert!(!is_valid_blob_id(&"A".repeat(64)));
        assert!(!is_valid_blob_id("../etc/passwd"));
        assert_eq!(
            store.get("../etc/passwd").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let oversized = vec![0u8; MAX_BLOB_BYTES + 1];
        assert_eq!(
            store.put(&oversized).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn parses_blob_urls_and_data_urls() {
        let id = "a".repeat(64);
        let url = blob_url(&id);
        assert_eq!(parse_blob_id(&url), Some(id.as_str()));
        assert_eq!(parse_blob_id("mew-blob://../bad"), None);

        let encoded = encode_data_url("image/png", b"png-bytes");
        assert_eq!(
            decode_data_url(&encoded),
            Some(("image/png".to_string(), b"png-bytes".to_vec()))
        );
        assert_eq!(decode_data_url("https://example.com/a.png"), None);
    }
}
