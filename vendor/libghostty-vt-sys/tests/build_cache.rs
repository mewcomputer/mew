//! Rebuild the real pinned Ghostty source with externally configured Zig caches.
//! The shared checkout is read-only; each test owns its clone and build outputs.
//!
//! Run from the repository root with Zig 0.15.2 installed:
//! ```sh
//! MEW_GHOSTTY_TEST_SOURCE_DIR=/path/to/pinned/ghostty \
//! ZIG_GLOBAL_CACHE_DIR=/path/to/populated/zig/cache \
//! cargo test -p libghostty-vt-sys --test build_cache -- --ignored
//! ```

use std::path::PathBuf;
use std::process::Command;

struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
#[ignore = "requires Zig 0.15.2, MEW_GHOSTTY_TEST_SOURCE_DIR and a populated ZIG_GLOBAL_CACHE_DIR"]
fn native_archives_are_resolved_with_external_zig_caches() {
    let existing_source = PathBuf::from(std::env::var_os("MEW_GHOSTTY_TEST_SOURCE_DIR").unwrap());
    let global_cache = PathBuf::from(std::env::var_os("ZIG_GLOBAL_CACHE_DIR").unwrap());
    let temporary = TestDirectory(std::env::temp_dir().join(format!(
        "mew-ghostty-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    std::fs::create_dir(&temporary.0).unwrap();
    let source = temporary.0.join("ghostty-src");
    let cloned = Command::new("git")
        .args(["clone", "--shared"])
        .arg(&existing_source)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        cloned.status.success(),
        "{}",
        String::from_utf8_lossy(&cloned.stderr)
    );
    let output = temporary.0.join("cargo-output");
    let external_local_cache = temporary.0.join("external-local-cache");
    std::fs::create_dir(&output).unwrap();
    let script = temporary.0.join("build-script");

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = Command::new(&rustc).arg("-vV").output().unwrap();
    assert!(version.status.success());
    let version = String::from_utf8(version.stdout).unwrap();
    let target = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    let compiled = Command::new(rustc)
        .arg("--edition=2024")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .arg("-o")
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let built = Command::new(script)
        .env_remove("DOCS_RS")
        .env("GHOSTTY_SOURCE_DIR", &source)
        .env("OUT_DIR", &output)
        .env("HOST", target)
        .env("TARGET", target)
        .env("ZIG_LOCAL_CACHE_DIR", &external_local_cache)
        .env("ZIG_GLOBAL_CACHE_DIR", &global_cache)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    for archive in [
        "libghostty-vt.a",
        "libsimdutf.a",
        "libhighway.a",
        "libutfcpp.a",
    ] {
        let installed = output.join("ghostty-install/lib").join(archive);
        assert!(std::fs::metadata(installed).unwrap().len() > 0, "{archive}");
    }
    assert!(
        output
            .join("ghostty-install/include/ghostty/vt.h")
            .is_file()
    );
    assert!(output.join("zig-cache/o").is_dir());
    assert!(
        !external_local_cache.exists(),
        "build outputs must stay scoped to Cargo"
    );
}
