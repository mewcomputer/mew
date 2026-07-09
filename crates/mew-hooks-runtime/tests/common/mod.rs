//! Shared test helpers for plugin integration and restart tests.

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use mew_hooks::PluginHost;

pub fn test_host() -> PluginHost {
    PluginHost {
        notify: Arc::new(|msg| eprintln!("[plugin-notify] {msg}")),
        config_read: Arc::new(|_key| None),
        log: Arc::new(|msg| eprintln!("[plugin-log] {msg}")),
        storage_read: Arc::new(|_key| None),
        storage_write: Arc::new(|_key, _val| {}),
        storage_delete: Arc::new(|_key| {}),
        set_ui: Arc::new(|_key, _val| {}),
    }
}

/// Find the sample plugin binary (built via `cargo build --example sample-plugin`).
pub fn sample_plugin_path() -> PathBuf {
    if let Ok(path) = env::var("CARGO_BIN_EXE_sample-plugin") {
        return PathBuf::from(path);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest)
        .join("target")
        .join("debug")
        .join("examples")
        .join("sample-plugin");

    if target.exists() {
        return target;
    }

    let status = Command::new("cargo")
        .args(["build", "--example", "sample-plugin"])
        .current_dir(&manifest)
        .status()
        .expect("cargo build example");

    assert!(status.success(), "failed to build sample-plugin example");

    assert!(
        target.exists(),
        "sample-plugin binary not found at {:?}",
        target
    );
    target
}

/// Create a temp directory containing a copy of the sample plugin binary.
pub fn make_plugin_dir_with_binary() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = sample_plugin_path();
    let dst = dir.path().join("sample-plugin");
    std::fs::copy(&src, &dst).expect("copy plugin binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dst, perms).unwrap();
    }
    dir
}
