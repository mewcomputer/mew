use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Arc;

/// A file the user or model has marked as important for the session, so it
/// survives context compaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FlaggedFile {
    pub path: PathBuf,
    pub mode: FlagMode,
}

/// How a flagged file is carried across compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagMode {
    /// Inline the file's content into the post-compaction context.
    Included,
    /// Record a pointer to the file without inlining content.
    Referenced,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct FlaggedFilesState {
    files: Vec<FlaggedFile>,
    #[serde(default)]
    unflagged: Vec<PathBuf>,
}

async fn load_flagged_state(path: &std::path::Path) -> std::io::Result<Option<FlaggedFilesState>> {
    match tokio::fs::read(path).await {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(std::io::Error::other),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub async fn load_flagged_files(
    path: &std::path::Path,
) -> std::io::Result<Option<Vec<FlaggedFile>>> {
    Ok(load_flagged_state(path).await?.map(|state| state.files))
}

pub async fn is_explicitly_unflagged(
    path: &std::path::Path,
    file: &std::path::Path,
) -> std::io::Result<bool> {
    Ok(load_flagged_state(path)
        .await?
        .is_some_and(|state| state.unflagged.iter().any(|item| item == file)))
}

pub async fn save_flagged_files(
    path: &std::path::Path,
    flags: &[FlaggedFile],
) -> std::io::Result<()> {
    let mut state = load_flagged_state(path).await?.unwrap_or_default();
    state.files = flags.to_vec();
    save_flagged_state(path, &state).await
}

pub async fn save_explicit_flag(
    path: &std::path::Path,
    flags: &[FlaggedFile],
    file: &std::path::Path,
) -> std::io::Result<()> {
    let mut state = load_flagged_state(path).await?.unwrap_or_default();
    state.files = flags.to_vec();
    state.unflagged.retain(|item| item != file);
    save_flagged_state(path, &state).await
}

pub async fn save_explicit_unflag(
    path: &std::path::Path,
    flags: &[FlaggedFile],
    file: &std::path::Path,
) -> std::io::Result<()> {
    let mut state = load_flagged_state(path).await?.unwrap_or_default();
    state.files = flags.to_vec();
    if !state.unflagged.iter().any(|item| item == file) {
        state.unflagged.push(file.to_path_buf());
    }
    save_flagged_state(path, &state).await
}

async fn save_flagged_state(
    path: &std::path::Path,
    state: &FlaggedFilesState,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let data = serde_json::to_vec(state).map_err(std::io::Error::other)?;
    let temporary = path.with_file_name(format!(".flagged-{}.tmp", ulid::Ulid::new()));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        file.write_all(&data).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temporary, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

/// Human-readable label for a FlagMode, used in wire protocol and UI.
/// Shared between the agent emit path, daemon unflag handler, and attach replay.
pub fn flag_mode_label(mode: FlagMode) -> &'static str {
    match mode {
        FlagMode::Included => "included",
        FlagMode::Referenced => "referenced",
    }
}

pub struct FlagImportant {
    schema: Value,
}

impl FlagImportant {
    pub fn new() -> Self {
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
        Self { schema }
    }
}

impl Default for FlagImportant {
    fn default() -> Self {
        Self::new()
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

        let mut guard = ctx.flagged_files.lock().await;
        let mut next = guard.clone();
        if let Some(existing) = next.iter_mut().find(|f| f.path == path) {
            existing.mode = mode;
        } else {
            next.push(FlaggedFile {
                path: path.clone(),
                mode,
            });
        }
        if let Some(state_path) = &ctx.flagged_files_path {
            save_explicit_flag(state_path, &next, &path)
                .await
                .map_err(|error| {
                    ToolError::Execution(format!("could not save flagged files: {error}"))
                })?;
        }
        *guard = next;
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
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx_with_cwd(
        cwd: PathBuf,
        flagged: Arc<tokio::sync::Mutex<Vec<FlaggedFile>>>,
    ) -> ToolCtx {
        ToolCtx::new(
            Arc::new(crate::ToolCtxShared {
                cwd,
                flagged_files: flagged,
                ..Default::default()
            }),
            "test".into(),
            tokio_util::sync::CancellationToken::new(),
            tokio::sync::mpsc::channel(1).0,
        )
    }

    fn write_temp_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[tokio::test]
    async fn flags_are_durable_and_failed_writes_do_not_change_them() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "plan.md", "plan");
        let path = tmp.path().join("flags.json");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new();
        let ctx = || {
            ToolCtx::new(
                Arc::new(crate::ToolCtxShared {
                    cwd: tmp.path().to_path_buf(),
                    flagged_files_path: Some(path.clone()),
                    flagged_files: flagged.clone(),
                    ..Default::default()
                }),
                "test".into(),
                tokio_util::sync::CancellationToken::new(),
                tokio::sync::mpsc::channel(1).0,
            )
        };
        tool.execute(ctx(), serde_json::json!({"path": file, "mode": "included"}))
            .await
            .unwrap();
        let restored = load_flagged_files(&path).await.unwrap().unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].mode, FlagMode::Included);
        tool.execute(
            ctx(),
            serde_json::json!({"path": file, "mode": "referenced"}),
        )
        .await
        .unwrap();
        assert_eq!(
            load_flagged_files(&path).await.unwrap().unwrap()[0].mode,
            FlagMode::Referenced
        );
        save_explicit_unflag(&path, &[], &file).await.unwrap();
        flagged.lock().await.clear();
        assert!(is_explicitly_unflagged(&path, &file).await.unwrap());
        tool.execute(ctx(), serde_json::json!({"path": file, "mode": "included"}))
            .await
            .unwrap();
        assert!(!is_explicitly_unflagged(&path, &file).await.unwrap());

        let bad = ToolCtx::new(
            Arc::new(crate::ToolCtxShared {
                cwd: tmp.path().to_path_buf(),
                flagged_files_path: Some(tmp.path().join("missing").join("flags.json")),
                flagged_files: flagged.clone(),
                ..Default::default()
            }),
            "test".into(),
            tokio_util::sync::CancellationToken::new(),
            tokio::sync::mpsc::channel(1).0,
        );
        assert!(tool
            .execute(bad, serde_json::json!({"path": file, "mode": "referenced"}))
            .await
            .is_err());
        assert_eq!(flagged.lock().await[0].mode, FlagMode::Included);
        assert_eq!(
            load_flagged_files(&path).await.unwrap().unwrap()[0].mode,
            FlagMode::Included
        );
    }

    #[tokio::test]
    async fn test_flag_included_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "plan.md", "# Plan\nDo things.");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new();
        let input = serde_json::json!({
            "path": file.to_string_lossy(),
            "mode": "included"
        });
        let result = tool
            .execute(
                dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
                input,
            )
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
        let tool = FlagImportant::new();
        let input = serde_json::json!({
            "path": file.to_string_lossy(),
            "mode": "referenced"
        });
        tool.execute(
            dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
            input,
        )
        .await
        .unwrap();
        assert_eq!(flagged.lock().await[0].mode, FlagMode::Referenced);
    }

    #[tokio::test]
    async fn test_flag_default_mode_is_included() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "a.txt", "a");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new();
        let input = serde_json::json!({ "path": file.to_string_lossy() });
        tool.execute(
            dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
            input,
        )
        .await
        .unwrap();
        assert_eq!(flagged.lock().await[0].mode, FlagMode::Included);
    }

    #[tokio::test]
    async fn test_flag_missing_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new();
        let input = serde_json::json!({});
        let result = tool
            .execute(
                dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
                input,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_flag_nonexistent_file_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new();
        let input = serde_json::json!({ "path": "nope.md" });
        let result = tool
            .execute(
                dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
                input,
            )
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
        let tool = FlagImportant::new();
        let input = serde_json::json!({ "path": file.to_string_lossy(), "mode": "banana" });
        let result = tool
            .execute(
                dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
                input,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_re_flag_same_path_updates_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_temp_file(tmp.path(), "shared.md", "body");
        let flagged = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let tool = FlagImportant::new();

        tool.execute(
            dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
            serde_json::json!({ "path": file.to_string_lossy(), "mode": "included" }),
        )
        .await
        .unwrap();
        tool.execute(
            dummy_ctx_with_cwd(tmp.path().to_path_buf(), flagged.clone()),
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
        let tool = FlagImportant::new();
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
