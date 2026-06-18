use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// A file the user or model has marked as important for the session, so it
/// survives context compaction.
#[derive(Debug, Clone)]
pub struct FlaggedFile {
    pub path: PathBuf,
    pub mode: FlagMode,
}

/// How a flagged file is carried across compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagMode {
    /// Inline the file's content into the post-compaction context.
    Included,
    /// Record a pointer to the file without inlining content.
    Referenced,
}

pub struct FlagImportant {
    flagged: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>>,
    schema: Value,
}

impl FlagImportant {
    pub fn new(flagged: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>>) -> Self {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to flag. Relative to the cwd or absolute."
                },
                "mode": {
                    "type": "string",
                    "enum": ["included", "referenced"],
                    "description": "`included` (default) re-injects the file's content into the model's context after each compaction; `referenced` records only a note so the model knows the file matters and can re-read it on demand.",
                    "default": "included"
                }
            },
            "required": ["path"]
        });
        Self { flagged, schema }
    }
}

#[async_trait]
impl Tool for FlagImportant {
    fn name(&self) -> &str {
        "flag_important"
    }

    fn description(&self) -> &str {
        "Mark a file as important for the session so it survives context compaction. \
         Use this on files the ongoing work depends on (a plan, a spec, a key source \
         file) so they are not lost when the conversation is compacted. In `included` \
         mode the file's content is re-injected after compaction; in `referenced` mode \
         only a note is added and the model can re-read the file when needed. Re-flagging \
         the same path updates its mode rather than duplicating it."
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing or non-string `path`".into()))?;
        let mode_str = input
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("included");
        let mode = match mode_str {
            "included" => FlagMode::Included,
            "referenced" => FlagMode::Referenced,
            other => {
                return Err(ToolError::InvalidInput(format!(
                    "invalid mode `{other}`; expected \"included\" or \"referenced\""
                )))
            }
        };

        let path = ctx.cwd.join(path_str);
        if !path.exists() {
            return Err(ToolError::Execution(format!(
                "file not found: {} (resolved to {})",
                path_str,
                path.display()
            )));
        }

        let mut guard = self.flagged.lock().await;
        if let Some(existing) = guard.iter_mut().find(|f| f.path == path) {
            existing.mode = mode;
        } else {
            guard.push(FlaggedFile {
                path: path.clone(),
                mode,
            });
        }
        let count = guard.len();
        drop(guard);

        Ok(ToolOutput {
            output: format!(
                "flagged {} ({}). {} file(s) flagged this session; these survive compaction.",
                path.display(),
                mode_str,
                count,
            ),
            error: String::new(),
            diff: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx_with_cwd(cwd: PathBuf) -> ToolCtx {
        ToolCtx {
            session_id: mew_message::SessionId::from(ulid::Ulid::new()),
            call_id: "test".to_string(),
            cancel: tokio_util::sync::CancellationToken::new(),
            progress_tx: tokio::sync::mpsc::channel(1).0,
            cwd,
            dispatcher: None,
            secrets: Default::default(),
        }
    }

    fn write_temp_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[tokio::test]
    async fn test_flag_included_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "plan.md", "# Plan\nDo things.");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged.clone());
        let input = serde_json::json!({
            "path": file.to_string_lossy(),
            "mode": "included"
        });
        let result = tool
            .execute(dummy_ctx_with_cwd(tmp.path().to_path_buf()), input)
            .await
            .unwrap();
        assert!(result.output.contains("included"));
        let guard = flagged.lock().await;
        assert_eq!(guard.len(), 1);
        assert_eq!(guard[0].path, file);
        assert_eq!(guard[0].mode, FlagMode::Included);
    }

    #[tokio::test]
    async fn test_flag_referenced_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "spec.md", "spec body");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged.clone());
        let input = serde_json::json!({
            "path": file.to_string_lossy(),
            "mode": "referenced"
        });
        tool.execute(dummy_ctx_with_cwd(tmp.path().to_path_buf()), input)
            .await
            .unwrap();
        assert_eq!(flagged.lock().await[0].mode, FlagMode::Referenced);
    }

    #[tokio::test]
    async fn test_flag_default_mode_is_included() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "a.txt", "a");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged.clone());
        let input = serde_json::json!({ "path": file.to_string_lossy() });
        tool.execute(dummy_ctx_with_cwd(tmp.path().to_path_buf()), input)
            .await
            .unwrap();
        assert_eq!(flagged.lock().await[0].mode, FlagMode::Included);
    }

    #[tokio::test]
    async fn test_flag_missing_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged);
        let input = serde_json::json!({});
        let result = tool
            .execute(dummy_ctx_with_cwd(tmp.path().to_path_buf()), input)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_flag_nonexistent_file_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged.clone());
        let input = serde_json::json!({ "path": "nope.md" });
        let result = tool
            .execute(dummy_ctx_with_cwd(tmp.path().to_path_buf()), input)
            .await;
        assert!(result.is_err());
        assert!(
            flagged.lock().await.is_empty(),
            "nothing should be flagged on failure"
        );
    }

    #[tokio::test]
    async fn test_flag_invalid_mode_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "a.txt", "a");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged.clone());
        let input = serde_json::json!({ "path": file.to_string_lossy(), "mode": "banana" });
        let result = tool
            .execute(dummy_ctx_with_cwd(tmp.path().to_path_buf()), input)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_re_flag_same_path_updates_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "shared.md", "body");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged.clone());

        tool.execute(
            dummy_ctx_with_cwd(tmp.path().to_path_buf()),
            serde_json::json!({ "path": file.to_string_lossy(), "mode": "included" }),
        )
        .await
        .unwrap();
        tool.execute(
            dummy_ctx_with_cwd(tmp.path().to_path_buf()),
            serde_json::json!({ "path": file.to_string_lossy(), "mode": "referenced" }),
        )
        .await
        .unwrap();

        let guard = flagged.lock().await;
        assert_eq!(guard.len(), 1, "re-flagging should update, not duplicate");
        assert_eq!(guard[0].mode, FlagMode::Referenced);
    }

    #[test]
    fn test_tool_metadata() {
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new(flagged);
        assert_eq!(tool.name(), "flag_important");
        assert_eq!(tool.sensitivity(), Sensitivity::ReadOnly);
        let required = tool
            .schema()
            .get("required")
            .and_then(|v| v.as_array())
            .unwrap();
        assert!(required.iter().any(|v| v == "path"));
    }
}
