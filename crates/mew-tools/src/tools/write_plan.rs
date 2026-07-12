use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

/// Write the full content of the configured plan file. Unlike `write`, this
/// tool has no `path` parameter — it always targets the plan path configured
/// for the session (`config.toml: plan_path`, default `PLAN.md`), so the
/// planner cannot write arbitrary files.
pub struct WritePlan {
    plan_path: String,
}

impl WritePlan {
    pub fn new(plan_path: impl Into<String>) -> Self {
        Self {
            plan_path: plan_path.into(),
        }
    }

    /// Resolve the plan path against the tool's cwd. Absolute paths are used
    /// as-is; relative paths are joined onto `cwd`.
    fn resolve(&self, cwd: &std::path::Path) -> PathBuf {
        let p = PathBuf::from(&self.plan_path);
        if p.is_absolute() {
            p
        } else {
            cwd.join(p)
        }
    }
}

#[async_trait]
impl Tool for WritePlan {
    fn name(&self) -> &str {
        "write_plan"
    }

    fn description(&self) -> &str {
        "Write the full content of the configured plan file (creates or \
         overwrites). Intended for the planning workflow: draft the plan here, \
         then submit it with handoff_plan."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The full markdown content of the plan file."
                    }
                },
                "required": ["content"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing content".into()))?;

        let path = self.resolve(&ctx.cwd);

        let old_content = tokio::fs::read_to_string(&path).await.ok();

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Execution(format!("create dirs failed: {}", e)))?;
        }

        // Atomic write: write to a temp file in the same directory, then rename.
        let parent_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let tmp = parent_dir.join(format!(".mew-tmp-{}", ulid::Ulid::new()));
        tokio::fs::write(&tmp, content)
            .await
            .map_err(|e| ToolError::Execution(format!("write failed: {}", e)))?;
        if let Err(e) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(ToolError::Execution(format!("rename failed: {}", e)));
        }

        let diff = if let Some(ref old) = old_content {
            let mut diff_text = format!(
                "overwrote {} (was {} bytes, now {} bytes)\n",
                path.display(),
                old.len(),
                content.len()
            );
            diff_text.push_str(&super::make_unified_diff(old, content, &path));
            Some(diff_text)
        } else {
            let preview: String = content
                .lines()
                .take(6)
                .map(|l| format!("+ {}", l))
                .collect::<Vec<_>>()
                .join("\n");
            let more = if content.lines().count() > 6 {
                format!("\n  ... ({} more lines)", content.lines().count() - 6)
            } else {
                String::new()
            };
            Some(format!("created {}\n{}{}", path.display(), preview, more))
        };

        Ok(ToolOutput {
            output: format!("wrote {} bytes to {}", content.len(), path.display()),
            error: String::new(),
            diff,
            file_delta: Some(super::compute_file_delta(
                old_content.as_deref(),
                content,
                &path,
            )),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    #[tokio::test]
    async fn test_write_plan_creates() {
        let dir = tempfile::tempdir().unwrap();
        let tool = WritePlan::new("PLAN.md");
        let result = tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"content": "goal\nstep 1"}),
            )
            .await
            .unwrap();
        assert!(result.output.contains("PLAN.md"));
        let written = tokio::fs::read_to_string(dir.path().join("PLAN.md"))
            .await
            .unwrap();
        assert_eq!(written, "goal\nstep 1");
        let diff = result.diff.unwrap();
        assert!(diff.contains("created"));
    }

    #[tokio::test]
    async fn test_write_plan_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("PLAN.md"), "old plan")
            .await
            .unwrap();
        let tool = WritePlan::new("PLAN.md");
        let result = tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"content": "new plan"}),
            )
            .await
            .unwrap();
        let diff = result.diff.unwrap();
        assert!(diff.contains("overwrote"));
        let written = tokio::fs::read_to_string(dir.path().join("PLAN.md"))
            .await
            .unwrap();
        assert_eq!(written, "new plan");
    }

    #[tokio::test]
    async fn test_write_plan_nested_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = WritePlan::new(".mew/plans/current.md");
        tool.execute(
            ctx(dir.path().to_path_buf()),
            serde_json::json!({"content": "nested"}),
        )
        .await
        .unwrap();
        let written = tokio::fs::read_to_string(dir.path().join(".mew/plans/current.md"))
            .await
            .unwrap();
        assert_eq!(written, "nested");
    }

    #[tokio::test]
    async fn test_write_plan_absolute_path_honored() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join("absolute-plan.md");
        let tool = WritePlan::new(abs.to_str().unwrap());
        // cwd is a different, throwaway dir — the absolute path must win.
        let other = tempfile::tempdir().unwrap();
        tool.execute(
            ctx(other.path().to_path_buf()),
            serde_json::json!({"content": "abs"}),
        )
        .await
        .unwrap();
        let written = tokio::fs::read_to_string(&abs).await.unwrap();
        assert_eq!(written, "abs");
    }

    #[tokio::test]
    async fn test_write_plan_missing_content_errors() {
        let dir = tempfile::tempdir().unwrap();
        let tool = WritePlan::new("PLAN.md");
        let result = tool
            .execute(ctx(dir.path().to_path_buf()), serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }
}
