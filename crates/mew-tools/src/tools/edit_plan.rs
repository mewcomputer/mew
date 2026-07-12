use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

/// Edit the configured plan file with an exact string replacement. Like
/// `edit_str_replace` but pinned to the session's plan path — the planner
/// cannot edit arbitrary files. Supports `replace_all` for repeated strings.
pub struct EditPlan {
    plan_path: String,
}

impl EditPlan {
    pub fn new(plan_path: impl Into<String>) -> Self {
        Self {
            plan_path: plan_path.into(),
        }
    }

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
impl Tool for EditPlan {
    fn name(&self) -> &str {
        "edit_plan"
    }

    fn description(&self) -> &str {
        "Replace old_string with new_string in the configured plan file. Exact \
         match required; fails if ambiguous unless replace_all is set. Intended \
         for the planning workflow: use write_plan first, then edit_plan to \
         revise (e.g. after handoff_plan returns change requests)."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to replace."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace every occurrence instead of requiring a unique match. Defaults to false."
                    }
                },
                "required": ["old_string", "new_string"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let old = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing old_string".into()))?;
        let new = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing new_string".into()))?;
        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = self.resolve(&ctx.cwd);
        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            ToolError::Execution(format!(
                "plan file {} could not be read ({e}) — use write_plan first",
                path.display()
            ))
        })?;

        let count = content.matches(old).count();
        if count == 0 {
            return Err(ToolError::Execution(format!(
                "old_string not found in {} — the plan may have changed; \
                 read it again or use write_plan to rewrite it",
                path.display()
            )));
        }
        if count > 1 && !replace_all {
            return Err(ToolError::Execution(format!(
                "old_string matched {} times in {}; ambiguous — include more \
                 surrounding context or set replace_all to true",
                count,
                path.display()
            )));
        }

        let new_content = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };

        let parent_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let tmp = parent_dir.join(format!(".mew-tmp-{}", ulid::Ulid::new()));
        tokio::fs::write(&tmp, &new_content)
            .await
            .map_err(|e| ToolError::Execution(format!("write failed: {}", e)))?;
        if let Err(e) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(ToolError::Execution(format!("rename failed: {}", e)));
        }

        let occurrences = if replace_all { count } else { 1 };
        let diff = super::make_unified_diff(&content, &new_content, &path);

        Ok(ToolOutput {
            output: format!("replaced {occurrences} occurrence(s) in {}", path.display()),
            error: String::new(),
            diff: Some(diff),
            file_delta: Some(super::compute_file_delta(
                Some(&content),
                &new_content,
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

    async fn seed(dir: &std::path::Path, content: &str) {
        tokio::fs::write(dir.join("PLAN.md"), content)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_edit_plan_unique() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "goal\nstep one\nstep two").await;
        let tool = EditPlan::new("PLAN.md");
        let result = tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"old_string": "step one", "new_string": "step 1"}),
            )
            .await
            .unwrap();
        assert!(result.output.contains("replaced 1"));
        let written = tokio::fs::read_to_string(dir.path().join("PLAN.md"))
            .await
            .unwrap();
        assert_eq!(written, "goal\nstep 1\nstep two");
    }

    #[tokio::test]
    async fn test_edit_plan_no_match() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "goal").await;
        let tool = EditPlan::new("PLAN.md");
        let err = tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"old_string": "missing", "new_string": "x"}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_plan_ambiguous() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "step\nstep\nstep").await;
        let tool = EditPlan::new("PLAN.md");
        let err = tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"old_string": "step", "new_string": "x"}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("ambiguous"));
        assert!(err.contains("replace_all"));
    }

    #[tokio::test]
    async fn test_edit_plan_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "step\nstep\nstep").await;
        let tool = EditPlan::new("PLAN.md");
        let result = tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"old_string": "step", "new_string": "done", "replace_all": true}),
            )
            .await
            .unwrap();
        assert!(result.output.contains("replaced 3"));
        let written = tokio::fs::read_to_string(dir.path().join("PLAN.md"))
            .await
            .unwrap();
        assert_eq!(written, "done\ndone\ndone");
    }

    #[tokio::test]
    async fn test_edit_plan_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let tool = EditPlan::new("PLAN.md");
        let err = tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"old_string": "a", "new_string": "b"}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("write_plan"));
    }
}
