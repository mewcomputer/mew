//! Theme install command tests.

use std::io::Write;

/// Running `mew theme install` with an unknown token in the JSON file
/// should surface a validation error instead of silently installing it.
#[test]
fn theme_install_rejects_unknown_token() {
    let temp = tempfile::tempdir().unwrap();
    let bad = temp.path().join("bad.json");
    let mut f = std::fs::File::create(&bad).unwrap();
    f.write_all(br##"{"name": "bad", "tokens": {"not.a.real.token": "#ff0000"}}"##)
        .unwrap();
    drop(f);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mew"))
        .arg("theme")
        .arg("install")
        .arg(&bad)
        .env("HOME", temp.path())
        .output()
        .expect("failed to run mew theme install");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "install should fail for unknown token: {stderr}"
    );
    assert!(
        stderr.contains("not.a.real.token"),
        "error should name the bad token: {stderr}"
    );
}

/// A valid theme JSON file is copied into the user's themes directory.
#[test]
fn theme_installs_valid_file() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("warm.json");
    std::fs::write(
        &src,
        br##"{"name": "warm", "tokens": {"background": "#2a1f1a"}}"##,
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mew"))
        .arg("theme")
        .arg("install")
        .arg(&src)
        .env("HOME", temp.path())
        .output()
        .expect("failed to run mew theme install");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "install should succeed: {stderr}");

    let dest: std::path::PathBuf = std::fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name() == std::ffi::OsStr::new("warm.json"))
        .map(|e| e.path())
        .expect("theme file should be copied to themes dir");
    let contents = std::fs::read_to_string(&dest).unwrap();
    assert!(contents.contains("#2a1f1a"));
}
