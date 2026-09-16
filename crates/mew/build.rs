//! Embeds the git revision of this checkout into the binary as
//! `MEW_GIT_HASH` (`<short-hash>-dirty` when the tree has uncommitted
//! changes, or `unknown` when git isn't available). `mew daemon --status`
//! and `mew daemon --status` surfaces it so a running daemon can be checked
//! against the local build. `mew --version` reports the semver release
//! identity separately.
//!
//! No `rerun-if-changed` directives are emitted, so cargo re-runs this
//! script whenever any file in the package changes; the hash can lag a
//! bare `git commit` that touches nothing else until the next rebuild.

fn main() {
    let short = git(&["rev-parse", "--short", "HEAD"]);
    let dirty = git(&["status", "--porcelain"]).map(|s| !s.is_empty());
    let rev = match (short, dirty) {
        (Some(h), Some(true)) => format!("{h}-dirty"),
        (Some(h), _) => h,
        (None, _) => "unknown".to_string(),
    };
    println!("cargo:rustc-env=MEW_GIT_HASH={rev}");
}

fn git(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
