//! Golden-frame tests for the TUI.
//!
//! Each test renders a scripted interaction to a text frame and diffs it
//! against a checked-in `.frame` file. Regenerate all frames with:
//!
//! ```sh
//! MEW_UPDATE_GOLDEN=1 cargo test -p mew-tui
//! ```

use std::fs;
use std::path::PathBuf;

use mew_tui::harness::run_script;

/// Directory containing `.frame` golden files.
fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// Normalize environment-specific data (cwd path, git branch) so golden
/// frames are portable across different checkouts and branches.
fn normalize_frame(frame: &str) -> String {
    // Replace the status bar line: normalize the cwd path and git branch.
    // The status bar looks like:
    //   ~/code/mew/crates/mew-tui   git: main   0 tok  ·  $0.00
    // We normalize to:
    //   ~/mew   git: main   0 tok  ·  $0.00
    //
    // The status bar is always the second-to-last line (before the closing ---).
    let mut lines: Vec<String> = frame.lines().map(|l| l.to_string()).collect();
    for line in &mut lines {
        // Normalize any path containing /mew/ to ~/mew
        if line.contains("git:") {
            // Replace the leading path portion (everything before "git:")
            if let Some(git_idx) = line.find("git:") {
                let after_git = &line[git_idx..];
                *line = format!("  ~/mew   {}", after_git);
            }
        }
    }
    lines.push(String::new()); // re-add trailing newline
    lines.join("\n")
}

/// Run a golden test: execute `script` in an 80×24 terminal, diff the
/// rendered output against `tests/golden/{name}.frame`.
///
/// If `MEW_UPDATE_GOLDEN=1` is set, the frame file is (re)written instead.
fn golden_test(name: &str, script: &str) {
    let rendered = normalize_frame(&run_script(script, 80, 24));
    let dir = golden_dir();
    fs::create_dir_all(&dir).expect("create golden dir");
    let frame_path = dir.join(format!("{name}.frame"));

    let update = std::env::var("MEW_UPDATE_GOLDEN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);

    if update {
        fs::write(&frame_path, &rendered).expect("write golden frame");
        eprintln!("updated: {}", frame_path.display());
        return;
    }

    let expected = fs::read_to_string(&frame_path).unwrap_or_else(|_| {
        panic!(
            "golden frame not found: {}\nRun with MEW_UPDATE_GOLDEN=1 to generate it.",
            frame_path.display()
        )
    });

    let expected_trimmed = expected.trim_end();
    assert!(
        !expected_trimmed.is_empty(),
        "golden frame is empty: {}",
        frame_path.display()
    );

    let actual_trimmed = rendered.trim_end();

    assert_eq!(
        actual_trimmed,
        expected_trimmed,
        "golden frame mismatch for '{name}'\n\
         If this change is intentional, run:\n  \
         MEW_UPDATE_GOLDEN=1 cargo test -p mew-tui -- {name}\n\
         Frame: {}",
        frame_path.display()
    );
}

/// Seed frame: the welcome/empty state of the TUI.
#[test]
fn welcome() {
    golden_test(
        "welcome",
        "# Just open the app and snapshot the empty state\nsnapshot welcome",
    );
}
