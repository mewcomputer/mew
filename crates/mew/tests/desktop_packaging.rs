#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn framework(directory: &Path, version: &str) -> PathBuf {
    let framework = directory.join("Chromium Embedded Framework.framework");
    std::fs::create_dir_all(framework.join("Resources")).unwrap();
    std::fs::write(
        framework.join("Resources/Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>{version}.0</string></dict></plist>"#
        ),
    )
    .unwrap();
    std::fs::write(directory.join("CREDITS.html"), version).unwrap();
    framework
}

fn resolve(target: &Path, cache: &Path, explicit: &Path) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/cef-runtime.sh");
    Command::new("bash")
        .args([
            "-euc",
            "source \"$1\"; find_cef_framework 150.0.14 aarch64 \"$2\" \"$3\" \"$4\"",
            "desktop-packaging-test",
        ])
        .arg(script)
        .arg(target)
        .arg(cache)
        .arg(explicit)
        .output()
        .unwrap()
}

fn selected(output: Output) -> PathBuf {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

#[test]
fn packaging_uses_the_pinned_sdk_instead_of_an_older_cached_framework() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("release");
    let cache = temp.path().join("cached SDKs");
    framework(&cache.join("150.0.10/cef_macos_aarch64"), "150.0.10");
    framework(&cache.join("150.0.11/cef_macos_aarch64"), "150.0.11");
    framework(&cache.join("150.0.14/cef_macos_x86_64"), "150.0.14");
    let expected = framework(
        &target.join("build/cef-dll-sys-current/out/cef_macos_aarch64"),
        "150.0.14",
    );
    assert_eq!(selected(resolve(&target, &cache, Path::new(""))), expected);
}

#[test]
fn packaging_accepts_matching_direct_and_versioned_sdk_locations() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("release");
    let cache = temp.path().join("cache");
    let sdk = cache.join("150.0.14/cef_macos_aarch64");
    let expected = framework(&sdk, "150.0.14");
    for location in [&cache, &sdk, &expected] {
        assert_eq!(
            selected(resolve(&target, location, Path::new(""))),
            expected
        );
    }
    for location in [&sdk, &expected] {
        assert_eq!(selected(resolve(&target, &cache, location)), expected);
    }
}

#[test]
fn packaging_rejects_an_explicit_mismatched_sdk_even_with_a_valid_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    framework(&cache.join("150.0.14/cef_macos_aarch64"), "150.0.14");
    let stale = framework(&temp.path().join("override"), "150.0.11");
    let output = resolve(&temp.path().join("release"), &cache, &stale);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("150.0.14"), "{error}");
    assert!(error.contains("override"), "{error}");
}

#[test]
fn packaging_rejects_missing_or_unversioned_frameworks() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let malformed = framework(&cache.join("150.0.14/cef_macos_aarch64"), "150.0.14");
    std::fs::remove_file(malformed.join("Resources/Info.plist")).unwrap();
    let output = resolve(&temp.path().join("release"), &cache, Path::new(""));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("150.0.14"));
}

fn release_fixture(root: &Path) -> PathBuf {
    let scripts = root.join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for script in ["package-desktop-release.sh", "cef-runtime.sh"] {
        std::fs::copy(repo.join("scripts").join(script), scripts.join(script)).unwrap();
    }
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"cef-dll-sys\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    let package = root.join("cef-dll-sys");
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::write(package.join("src/lib.rs"), "").unwrap();
    std::fs::write(
        package.join("Cargo.toml"),
        "[package]\nname = \"cef-dll-sys\"\nversion = \"150.2.1+150.0.14\"\nedition = \"2024\"\n",
    )
    .unwrap();
    let lockfile = Command::new("cargo")
        .args(["generate-lockfile", "--offline", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(
        lockfile.status.success(),
        "{}",
        String::from_utf8_lossy(&lockfile.stderr)
    );
    let app = root.join("target/release/bundle/macos/mew.app");
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    for binary in ["mew", "mew-desktop"] {
        std::fs::write(app.join("Contents/MacOS").join(binary), "").unwrap();
    }
    app
}

#[test]
fn release_rejects_stale_runtime_and_requires_the_bundled_credits() {
    let temp = tempfile::tempdir().unwrap();
    let app = release_fixture(temp.path());
    let frameworks = app.join("Contents/Frameworks");
    let bundled = framework(&frameworks, "150.0.11");
    let cache = temp.path().join("external SDK");
    framework(&cache, "150.0.14");
    let run = || {
        Command::new("bash")
            .arg(temp.path().join("scripts/package-desktop-release.sh"))
            .env("MEW_VERSION", "0.2.0")
            .env("MEW_DESKTOP_PROFILE", "release")
            .env("MEW_DESKTOP_OUTPUT_DIR", temp.path().join("dist"))
            .env("MEW_CEF_FRAMEWORK_SOURCE", &cache)
            .output()
            .unwrap()
    };
    let output = run();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("requires CEF runtime 150.0.14"), "{error}");

    std::fs::remove_dir_all(bundled).unwrap();
    framework(&frameworks, "150.0.14");
    std::fs::remove_file(frameworks.join("CREDITS.html")).unwrap();
    let output = run();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("bundled CEF CREDITS.html"), "{error}");
}
