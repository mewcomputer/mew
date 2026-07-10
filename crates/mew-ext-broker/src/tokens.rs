//! Extension attach tokens — minted at install time, stored in the
//! system keyring with file fallback (matching credential resolution
//! in mew-config).
//!
//! **Note:** Token validation is not yet wired into the broker IPC path.
//! Tokens are minted and rotated via CLI (`mew ext token`, `mew ext
//! rotate-all`), but the daemon socket-attach path that would consume
//! `validate_token` is deferred to a separate plan. Until that lands,
//! tokens exist in storage but are not checked on extension attach.

use keyring::Entry;

const TOKEN_SERVICE: &str = "mew-ext-tokens";

/// Mint a new attach token for an extension. Stores it in the keyring
/// (file fallback). Returns the token string.
///
/// **Not yet wired:** `install_extension` does not currently mint tokens.
/// Tokens are created via `mew ext rotate-all` or manual `mint_token` calls.
pub fn mint_token(name: &str) -> anyhow::Result<String> {
    let token = generate_token();
    store_token(name, &token)?;
    Ok(token)
}

/// Validate a token against the stored value for an extension.
/// Uses constant-time comparison to avoid timing side-channels.
///
/// **Not yet wired:** No codepath currently calls this function. It will
/// be consumed by the daemon socket-attach path when that ships.
pub fn validate_token(name: &str, token: &str) -> bool {
    match load_token(name) {
        Ok(stored) => constant_time_eq(stored.as_bytes(), token.as_bytes()),
        Err(_) => false,
    }
}

/// Revoke a token (delete from keyring + file).
pub fn revoke_token(name: &str) -> anyhow::Result<()> {
    // Keyring.
    if let Ok(entry) = Entry::new(TOKEN_SERVICE, name) {
        let _ = entry.delete_credential();
    }
    // File fallback.
    let path = token_file_path(name);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Rotate all tokens (revoke + re-mint).
///
/// Collects per-extension results instead of aborting on the first failure.
/// If some extensions fail, the returned vec contains only the successfully
/// rotated ones; errors are logged. The caller can check the returned vec
/// against the expected set to detect partial failures.
pub fn rotate_all_tokens() -> anyhow::Result<Vec<(String, String)>> {
    let names = list_tokened_extensions()?;
    let mut results = Vec::new();
    for name in &names {
        match (|| {
            revoke_token(name)?;
            mint_token(name)
        })() {
            Ok(token) => results.push((name.clone(), token)),
            Err(e) => {
                tracing::error!("failed to rotate token for extension '{}': {}", name, e);
            }
        }
    }
    Ok(results)
}

/// Constant-time byte comparison to prevent timing attacks on token validation.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn generate_token() -> String {
    // 26-character ULID (128 bits of entropy). No `rand` dep.
    ulid::Ulid::new().to_string()
}

fn store_token(name: &str, token: &str) -> anyhow::Result<()> {
    // Keyring first (same pattern as mew-config credential resolution).
    if let Ok(entry) = Entry::new(TOKEN_SERVICE, name) {
        if entry.set_password(token).is_ok() {
            // Write a marker file (containing just the name) so
            // list_tokened_extensions can find keyring-stored tokens.
            // The actual token lives in the keyring; the file is just
            // an index for rotate_all to discover.
            write_token_marker(name)?;
            return Ok(());
        }
    }
    // File fallback: write the actual token to the file.
    let path = token_file_path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        set_dir_private(parent);
    }
    std::fs::write(&path, token)?;
    set_file_private(&path);
    Ok(())
}

/// Write an empty marker file so list_tokened_extensions can discover
/// keyring-stored tokens. The file contains the extension name only.
fn write_token_marker(name: &str) -> anyhow::Result<()> {
    let path = token_file_path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        set_dir_private(parent);
    }
    // Only create if it doesn't exist (don't overwrite file-fallback tokens).
    if !path.exists() {
        std::fs::write(&path, name)?;
        set_file_private(&path);
    }
    Ok(())
}

/// Set file permissions to 0600 (owner read/write only) on Unix.
/// On non-Unix platforms, this is a no-op.
#[cfg(unix)]
fn set_file_private(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// Set directory permissions to 0700 (owner only) on Unix.
#[cfg(unix)]
fn set_dir_private(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_file_private(_path: &std::path::Path) {}

#[cfg(not(unix))]
fn set_dir_private(_path: &std::path::Path) {}

fn load_token(name: &str) -> anyhow::Result<String> {
    // Keyring first.
    if let Ok(entry) = Entry::new(TOKEN_SERVICE, name) {
        if let Ok(token) = entry.get_password() {
            return Ok(token);
        }
    }
    // File fallback.
    let path = token_file_path(name);
    let token = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("token not found for '{}': {}", name, e))?;
    Ok(token.trim().to_string())
}

fn token_file_path(name: &str) -> std::path::PathBuf {
    mew_config::config_dir()
        .join("extensions")
        .join("tokens")
        .join(format!("{}.token", name))
}

fn list_tokened_extensions() -> anyhow::Result<Vec<String>> {
    let dir = mew_config::config_dir().join("extensions").join("tokens");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|e| e.to_str()) == Some("token") {
            if let Some(name) = entry.path().file_stem().and_then(|s| s.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

/// Print the token for an extension (for `mew ext token <name>`).
pub fn show_token(name: &str) -> anyhow::Result<String> {
    load_token(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"helloo"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_generate_token_is_ulid() {
        let token = generate_token();
        // ULIDs are 26 chars.
        assert_eq!(token.len(), 26);
        // Two calls produce different tokens.
        let token2 = generate_token();
        assert_ne!(token, token2);
    }
}
