//! Build-time version identity.
//!
//! Version numbers lie about which code a binary actually contains, so
//! mew identifies builds by git revision instead. `build.rs` stamps
//! `MEW_GIT_HASH` into the binary at compile time.

/// Short git revision of this build: `3f2a1b9`, `3f2a1b9-dirty` when the
/// tree had uncommitted changes at build time, or `unknown` when git
/// wasn't available.
pub fn git_rev() -> &'static str {
    env!("MEW_GIT_HASH")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_rev_is_non_empty() {
        assert!(!git_rev().is_empty());
    }
}
