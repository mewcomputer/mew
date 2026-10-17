use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

/// Edit a fixed plan path or a named plan in the default `plans/` directory.
/// Generated plans require an explicit path so prior plans can be revised.
pub struct EditPlan {
    plan_path: String,
    generated: bool,
}

impl EditPlan {
    pub fn new(plan_path: impl Into<String>) -> Self {
        Self {
            plan_path: plan_path.into(),
            generated: false,
        }
    }

    pub fn generated() -> Self {
        Self {
            plan_path: "plans".into(),
            generated: true,
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

    fn generated_path(&self, cwd: &std::path::Path, input: &Value) -> Result<PathBuf, ToolError> {
        use std::path::Component;

        let supplied = input.get("path").and_then(Value::as_str).ok_or_else(|| {
            ToolError::InvalidInput("missing plan path; use the path returned by write_plan".into())
        })?;
        let path = std::path::Path::new(supplied);
        let relative = if path.is_absolute() {
            path.strip_prefix(cwd).map_err(|_| {
                ToolError::InvalidInput(
                    "plan path must be in the workspace plans/ directory".into(),
                )
            })?
        } else {
            path
        };
        let components: Vec<_> = relative.components().collect();
        let [Component::Normal(dir), Component::Normal(file)] = components.as_slice() else {
            return Err(ToolError::InvalidInput(
                "plan path must be plans/<name>.md".into(),
            ));
        };
        if *dir != "plans" || !file.to_string_lossy().ends_with(".md") {
            return Err(ToolError::InvalidInput(
                "plan path must be plans/<name>.md".into(),
            ));
        }
        let name = file.to_string_lossy();
        let stem = name.strip_suffix(".md").unwrap_or_default();
        let words: Vec<_> = stem.split('-').collect();
        if words.len() != 2
            || words
                .iter()
                .any(|word| word.is_empty() || !word.bytes().all(|ch| ch.is_ascii_lowercase()))
        {
            return Err(ToolError::InvalidInput(
                "plan path must be a generated plan filename".into(),
            ));
        }
        Ok(cwd.join(relative))
    }
}

#[async_trait]
impl Tool for EditPlan {
    fn name(&self) -> &str {
        "edit_plan"
    }

    fn description(&self) -> &str {
        "Replace old_string with new_string in a plan. For generated plans, \
         supply the path returned by write_plan; configured fixed paths need \
         no path. Exact match required unless replace_all is set."
    }

    fn schema(&self) -> &Value {
        static FIXED: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        static GENERATED: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        let schema = if self.generated { &GENERATED } else { &FIXED };
        schema.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path returned by write_plan. Required for generated plans in plans/."
                    },
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
                "required": if self.generated { vec!["path", "old_string", "new_string"] } else { vec!["old_string", "new_string"] }
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

        let path = if self.generated {
            let path = self.generated_path(&ctx.cwd, &input)?;
            for check in [&ctx.cwd.join("plans"), &path] {
                let metadata = tokio::fs::symlink_metadata(check).await.map_err(|e| {
                    ToolError::Execution(format!(
                        "plan file {} could not be inspected ({e})",
                        check.display()
                    ))
                })?;
                if metadata.file_type().is_symlink() {
                    return Err(ToolError::InvalidInput(
                        "generated plan paths cannot be symlinks".into(),
                    ));
                }
            }
            path
        } else {
            self.resolve(&ctx.cwd)
        };
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

    #[tokio::test]
    async fn generated_edit_requires_a_plan_path_and_cannot_escape_plans() {
        let dir = tempfile::tempdir().unwrap();
        let writer = crate::tools::write_plan::WritePlan::generated();
        writer
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"content": "first"}),
            )
            .await
            .unwrap();
        let path = std::fs::read_dir(dir.path().join("plans"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let tool = EditPlan::generated();
        let input = serde_json::json!({"old_string": "first", "new_string": "revised"});
        assert!(tool
            .execute(ctx(dir.path().to_path_buf()), input.clone())
            .await
            .is_err());
        assert!(tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"path": "plans/../outside.md", "old_string": "first", "new_string": "revised"}),
            )
            .await
            .is_err());
        assert!(tool
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"path": dir.path().join("outside.md"), "old_string": "first", "new_string": "revised"}),
            )
            .await
            .is_err());
        tool.execute(
            ctx(dir.path().to_path_buf()),
            serde_json::json!({"path": path, "old_string": "first", "new_string": "revised"}),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "revised");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn generated_edit_rejects_a_symlinked_plan() {
        let dir = tempfile::tempdir().unwrap();
        let plans = dir.path().join("plans");
        std::fs::create_dir(&plans).unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "do not change").unwrap();
        std::os::unix::fs::symlink(&outside, plans.join("luma-vare.md")).unwrap();
        let err = EditPlan::generated()
            .execute(
                ctx(dir.path().to_path_buf()),
                serde_json::json!({"path": "plans/luma-vare.md", "old_string": "do not", "new_string": "please"}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("symlink"));
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "do not change");
    }
}
