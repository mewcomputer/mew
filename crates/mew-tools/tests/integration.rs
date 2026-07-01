//! Composed tool scenarios that catch interface bugs the per-tool unit tests
//! miss. Each scenario chains two or more tools to exercise the
//! interactions between them: write+read, edit+bash, glob+grep, etc.

use mew_tools::tools::bash::Bash;
use mew_tools::tools::edit_str_replace::EditStrReplace;
use mew_tools::tools::glob::Glob;
use mew_tools::tools::grep::Grep;
use mew_tools::tools::read::Read;
use mew_tools::tools::write::Write;
use mew_tools::{Tool, ToolCtx};

fn ctx(dir: &tempfile::TempDir) -> ToolCtx {
    ToolCtx::test_new(dir.path().to_path_buf())
}

#[tokio::test]
async fn write_then_read_round_trip_preserves_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    let content = "first line\nsecond line\nthird line\n";

    // Write
    let write = Write;
    write
        .execute(
            ctx(&dir),
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content": content,
            }),
        )
        .await
        .expect("write succeeds");

    // Read back
    let read = Read;
    let result = read
        .execute(
            ctx(&dir),
            serde_json::json!({"path": path.to_string_lossy()}),
        )
        .await
        .expect("read succeeds");
    assert!(
        result.output.contains("1:first line"),
        "read output should include numbered content: {}",
        result.output
    );
    assert!(
        result.output.starts_with("[hello.txt#"),
        "read output should start with a hashline header: {}",
        result.output
    );
}

#[tokio::test]
async fn write_then_edit_then_bash_cat_verifies_edit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.txt");

    let write = Write;
    write
        .execute(
            ctx(&dir),
            serde_json::json!({
                "path": path.to_string_lossy(),
                "content": "version=1\n",
            }),
        )
        .await
        .unwrap();

    let edit = EditStrReplace;
    edit.execute(
        ctx(&dir),
        serde_json::json!({
            "path": "data.txt",
            "old_string": "version=1",
            "new_string": "version=2",
        }),
    )
    .await
    .unwrap();

    // Cross-tool verification: bash cat should see the edit.
    let bash = Bash;
    let result = bash
        .execute(ctx(&dir), serde_json::json!({"command": "cat data.txt"}))
        .await
        .unwrap();
    assert_eq!(result.output.trim(), "version=2");
}

#[tokio::test]
async fn glob_then_grep_composes_discovery_with_content_search() {
    let dir = tempfile::tempdir().unwrap();

    // Create a mix of files: 2 with a marker word, 1 without.
    for (name, body) in [
        ("a.rs", "fn main() { /* marker */ }\n"),
        ("b.rs", "// plain code without the keyword\n"),
        ("c.txt", "this has the marker too\n"),
    ] {
        let write = Write;
        write
            .execute(
                ctx(&dir),
                serde_json::json!({"path": dir.path().join(name).to_string_lossy(), "content": body}),
            )
            .await
            .unwrap();
    }

    // Glob for *.rs files — only a.rs and b.rs.
    let glob = Glob;
    let g = glob
        .execute(ctx(&dir), serde_json::json!({"pattern": "*.rs"}))
        .await
        .unwrap();
    let discovered: Vec<String> = g
        .output
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    assert_eq!(discovered.len(), 2, "expected 2 .rs files: {discovered:?}");

    // Grep the discovered paths for "marker" — only a.rs should match.
    let grep = Grep;
    let r = grep
        .execute(
            ctx(&dir),
            serde_json::json!({
                "pattern": "marker",
                "path": dir.path().to_string_lossy(),
                "glob": "*.rs",
            }),
        )
        .await
        .unwrap();
    assert!(
        r.output.contains("a.rs"),
        "expected a.rs in grep output: {}",
        r.output
    );
    assert!(
        !r.output.contains("b.rs"),
        "b.rs has no marker: {}",
        r.output
    );
    assert!(
        !r.output.contains("c.txt"),
        "c.txt is .txt not .rs: {}",
        r.output
    );
}

#[tokio::test]
async fn bash_nonzero_exit_surfaces_error_with_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let bash = Bash;
    let result = bash
        .execute(ctx(&dir), serde_json::json!({"command": "exit 42"}))
        .await
        .unwrap();
    assert!(
        result.error.contains("42"),
        "expected exit code 42 in error: {:?}",
        result.error
    );
    assert!(
        !result.output.contains("marker-not-present"),
        "no spurious stdout"
    );
}

#[tokio::test]
async fn glob_no_matches_returns_empty_not_error() {
    let dir = tempfile::tempdir().unwrap();
    let glob = Glob;
    let result = glob
        .execute(
            ctx(&dir),
            serde_json::json!({"pattern": "*.this-extension-does-not-exist"}),
        )
        .await
        .expect("glob must succeed even when nothing matches");
    assert!(
        result.output.trim().is_empty(),
        "expected empty output, got {:?}",
        result.output
    );
    assert!(result.error.is_empty());
}

#[tokio::test]
async fn read_offset_and_limit_pages_through_long_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lines.txt");
    let content: String = (1..=20).map(|i| format!("line {i}\n")).collect();

    let write = Write;
    write
        .execute(
            ctx(&dir),
            serde_json::json!({"path": path.to_string_lossy(), "content": content.clone()}),
        )
        .await
        .unwrap();

    let read = Read;
    let page = read
        .execute(
            ctx(&dir),
            serde_json::json!({
                "path": path.to_string_lossy(),
                "offset": 5,
                "limit": 3,
            }),
        )
        .await
        .unwrap();
    let lines: Vec<&str> = page.output.lines().collect();
    // First line is the hashline header; the next three are numbered content.
    assert_eq!(
        lines.len(),
        4,
        "limit=3 must give header + 3 lines: {lines:?}"
    );
    assert_eq!(lines[1], "6:line 6", "offset=5 skips lines 1-5");
    assert_eq!(lines[3], "8:line 8");
}

#[tokio::test]
async fn grep_filters_by_extension_across_many_files() {
    let dir = tempfile::tempdir().unwrap();
    let write = Write;

    // 5 .md files: 3 with "TODO", 2 without.
    for i in 0..5 {
        let body = if i < 3 {
            format!("doc {i}: TODO clean this up\n")
        } else {
            format!("doc {i}: all done\n")
        };
        write
            .execute(
                ctx(&dir),
                serde_json::json!({
                    "path": dir.path().join(format!("doc{i}.md")).to_string_lossy(),
                    "content": body,
                }),
            )
            .await
            .unwrap();
    }
    // 2 .rs files, neither with TODO (sanity check extension filter).
    for i in 0..2 {
        write
            .execute(
                ctx(&dir),
                serde_json::json!({
                    "path": dir.path().join(format!("code{i}.rs")).to_string_lossy(),
                    "content": format!("// TODO: write code {i}\nfn x() {{}}\n"),
                }),
            )
            .await
            .unwrap();
    }

    let grep = Grep;
    let result = grep
        .execute(
            ctx(&dir),
            serde_json::json!({
                "pattern": "TODO",
                "path": dir.path().to_string_lossy(),
                "glob": "*.md",
            }),
        )
        .await
        .unwrap();

    // 3 matches across 3 files, but extension filter excludes the .rs files.
    let match_count = result.output.lines().filter(|l| l.contains("doc")).count();
    assert_eq!(
        match_count, 3,
        "expected 3 .md matches (one per doc file); got {} in: {}",
        match_count, result.output
    );
    assert!(
        !result.output.contains("code0.rs"),
        ".rs files must be filtered out: {}",
        result.output
    );
}

#[tokio::test]
async fn edit_then_glob_finds_edited_file_by_pattern() {
    // Edit must not change the file name — only its contents. So a glob for
    // the original name must still find it after the edit.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("greeting.txt");
    let write = Write;
    write
        .execute(
            ctx(&dir),
            serde_json::json!({"path": path.to_string_lossy(), "content": "hello\n"}),
        )
        .await
        .unwrap();

    let edit = EditStrReplace;
    edit.execute(
        ctx(&dir),
        serde_json::json!({
            "path": "greeting.txt",
            "old_string": "hello",
            "new_string": "goodbye",
        }),
    )
    .await
    .unwrap();

    let glob = Glob;
    let g = glob
        .execute(ctx(&dir), serde_json::json!({"pattern": "greeting.*"}))
        .await
        .unwrap();
    assert!(
        g.output.contains("greeting.txt"),
        "edited file must still match glob: {}",
        g.output
    );
}
