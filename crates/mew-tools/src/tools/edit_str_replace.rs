use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct Edit;

#[async_trait]
impl Tool for Edit {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replace old_string with new_string in a file. Exact match required; fails if ambiguous."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file, relative to the current working directory."
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to replace."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text."
                    }
                },
                "required": ["path", "old_string", "new_string"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Mutating
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing path".into()))?;
        let old = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing old_string".into()))?;
        let new = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing new_string".into()))?;

        let path = ctx.cwd.join(path);
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::Execution(format!("read failed: {}: {}", path.display(), e)))?;

        let count = content.matches(old).count();
        if count == 0 {
            let line_count = content.lines().count();
            let first_line = content.lines().next().unwrap_or("(empty file)");
            let last_line = content.lines().last().unwrap_or("(empty file)");
            return Err(ToolError::Execution(format!(
                "old_string not found in {} ({} lines). \
                 First: {:?}. Last: {:?}. \
                 The file may have changed since it was last read — try reading it again.",
                path.display(),
                line_count,
                first_line.chars().take(120).collect::<String>(),
                last_line.chars().take(120).collect::<String>(),
            )));
        }
        if count > 1 {
            return Err(ToolError::Execution(format!(
                "old_string matched {} times in {}; ambiguous — \
                 include more surrounding context to make the match unique",
                count,
                path.display(),
            )));
        }

        let new_content = content.replacen(old, new, 1);

        // Atomic write: write to a temp file in the same directory, then rename.
        let parent_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let tmp = parent_dir.join(format!(".mew-tmp-{}", ulid::Ulid::new()));
        tokio::fs::write(&tmp, &new_content)
            .await
            .map_err(|e| ToolError::Execution(format!("write failed: {}", e)))?;
        if let Err(e) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(ToolError::Execution(format!("rename failed: {}", e)));
        }

        let diff = make_unified_diff(&content, &new_content, &path);

        Ok(ToolOutput {
            output: "replaced 1 occurrence".to_string(),
            error: String::new(),
            diff: Some(diff),
            ..Default::default()
        })
    }
}

/// Build a compact unified diff of two file contents.
fn make_unified_diff(old: &str, new: &str, path: &std::path::Path) -> String {
    use similar::TextDiff;

    let diff = TextDiff::from_lines(old, new);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    let mut out = String::new();
    for hunk in diff
        .unified_diff()
        .context_radius(3)
        .header(file_name, file_name)
        .iter_hunks()
    {
        out.push_str(&hunk.to_string());
    }

    if out.trim().is_empty() {
        file_name.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    #[tokio::test]
    async fn test_edit_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "world",
            "new_string": "mew"
        });
        let result = tool.execute(ctx, input).await.unwrap();
        assert_eq!(result.output, "replaced 1 occurrence");

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "hello mew");
    }

    #[tokio::test]
    async fn test_edit_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "missing",
            "new_string": "mew"
        });
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
        assert!(err.contains("test.txt"));
        // "hello world" is one line.
        assert!(err.contains("1 lines"));
    }

    #[tokio::test]
    async fn test_edit_ambiguous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "hello",
            "new_string": "hi"
        });
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("test.txt"));
    }

    #[tokio::test]
    async fn test_edit_diff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "test.txt",
            "old_string": "world",
            "new_string": "mew"
        });
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.diff.is_some());
        let diff = result.diff.unwrap();
        assert!(diff.contains("-hello world"));
        assert!(diff.contains("+hello mew"));
    }

    /// Regression: the "old_string not found" error must include first/last
    /// line snippets and a recovery hint, not just "not found". Caught a bug
    /// where the model had no way to figure out why its old_string was wrong.
    #[tokio::test]
    async fn test_edit_not_found_includes_first_last_line_and_recovery_hint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.txt");
        tokio::fs::write(
            &path,
            "alpha line one\nbeta line two\ngamma line three\ndelta line four\n",
        )
        .await
        .unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "multi.txt",
            "old_string": "this string is not in the file",
            "new_string": "replacement"
        });
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
        assert!(err.contains("multi.txt"));
        assert!(err.contains("4 lines"), "expected line count: {err}");
        assert!(
            err.contains("First:"),
            "expected 'First:' snippet in error: {err}"
        );
        assert!(
            err.contains("Last:"),
            "expected 'Last:' snippet in error: {err}"
        );
        assert!(err.contains("alpha line one"));
        assert!(err.contains("delta line four"));
        assert!(
            err.contains("try reading it again"),
            "expected recovery hint: {err}"
        );
    }

    /// Regression: the ambiguous-match error must suggest including more
    /// context to disambiguate.
    #[tokio::test]
    async fn test_edit_ambiguous_error_suggests_more_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dup.txt");
        tokio::fs::write(&path, "x x y x").await.unwrap();

        let tool = Edit;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({
            "path": "dup.txt",
            "old_string": "x",
            "new_string": "z"
        });
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ambiguous"));
        assert!(
            err.contains("more surrounding context") || err.contains("include more context"),
            "expected suggestion to include more context: {err}"
        );
        assert!(err.contains("dup.txt"));
    }
}
