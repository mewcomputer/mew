use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

/// Write a plan to the configured path, or create a new file in `plans/` when
/// the default plan location is selected. No arbitrary path input is accepted.
pub struct WritePlan {
    plan_path: String,
    generated: bool,
}

impl WritePlan {
    pub fn new(plan_path: impl Into<String>) -> Self {
        Self {
            plan_path: plan_path.into(),
            generated: false,
        }
    }

    pub fn generated() -> Self {
        Self::generated_in("plans")
    }

    pub fn generated_in(plans_dir: impl Into<PathBuf>) -> Self {
        Self {
            plan_path: plans_dir.into().to_string_lossy().into_owned(),
            generated: true,
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

/// Choose a fresh `<word>-<word>` directory under `plans_root` for a session.
pub fn session_plan_dir(plans_root: &std::path::Path) -> PathBuf {
    let config = resona::Config::default();
    if let (Ok(first), Ok(second)) = (resona::sona(&config), resona::sona(&config)) {
        return plans_root.join(format!("{first}-{second}"));
    }
    plans_root.join(format!(
        "plan-{}",
        ulid::Ulid::new().to_string().to_lowercase()
    ))
}

#[async_trait]
impl Tool for WritePlan {
    fn name(&self) -> &str {
        "write_plan"
    }

    fn description(&self) -> &str {
        "Write the full plan. By default each call creates a new named file in \
         plans/<words>/<name>.md; configured fixed paths are overwritten. The \
         result contains the path to use with edit_plan or to give to the builder."
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
                    },
                    "name": {
                        "type": "string",
                        "description": "Optional short filename slug for the plan (without .md)."
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
        let name = input.get("name").and_then(|v| v.as_str());

        let target = self.resolve(&ctx.cwd);
        let parent_dir = if self.generated {
            target.as_path()
        } else {
            target.parent().unwrap_or(std::path::Path::new("."))
        };
        let old_content = if self.generated {
            None
        } else {
            tokio::fs::read_to_string(&target).await.ok()
        };
        tokio::fs::create_dir_all(parent_dir)
            .await
            .map_err(|e| ToolError::Execution(format!("create dirs failed: {e}")))?;
        if self.generated {
            let metadata = tokio::fs::symlink_metadata(parent_dir).await.map_err(|e| {
                ToolError::Execution(format!("inspect plans directory failed: {e}"))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(ToolError::InvalidInput(
                    "plans directory cannot be a symlink".into(),
                ));
            }
        }

        let result = if self.generated {
            let file_name = plan_filename(name, content);
            let stem = file_name
                .strip_suffix(".md")
                .unwrap_or(&file_name)
                .to_string();
            let tmp = parent_dir.join(format!(".mew-tmp-{}", ulid::Ulid::new()));
            tokio::fs::write(&tmp, content)
                .await
                .map_err(|e| ToolError::Execution(format!("write failed: {e}")))?;
            let res = async {
                for attempt in 1..=16 {
                    let candidate = if attempt == 1 {
                        format!("{stem}.md")
                    } else {
                        format!("{stem}-{attempt}.md")
                    };
                    let path = parent_dir.join(candidate);
                    match tokio::fs::hard_link(&tmp, &path).await {
                        Ok(()) => return Ok(path),
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(e) => {
                            return Err(ToolError::Execution(format!("create plan failed: {e}")))
                        }
                    }
                }
                Err(ToolError::Execution(
                    "could not find an unused plan name".into(),
                ))
            }
            .await;
            let _ = tokio::fs::remove_file(&tmp).await;
            res
        } else {
            // A same-directory temp file keeps fixed-path renames atomic.
            let tmp = parent_dir.join(format!(".mew-tmp-{}", ulid::Ulid::new()));
            tokio::fs::write(&tmp, content)
                .await
                .map_err(|e| ToolError::Execution(format!("write failed: {e}")))?;
            let res = tokio::fs::rename(&tmp, &target)
                .await
                .map(|()| target)
                .map_err(|e| ToolError::Execution(format!("rename failed: {e}")));
            if res.is_err() {
                let _ = tokio::fs::remove_file(&tmp).await;
            }
            res
        };
        let path = result?;

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
            output: if self.generated {
                format!(
                    "wrote {} bytes to {} (plan path: {})",
                    content.len(),
                    path.display(),
                    path.strip_prefix(&ctx.cwd).unwrap_or(&path).display()
                )
            } else {
                format!("wrote {} bytes to {}", content.len(), path.display())
            },
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

fn plan_filename(name: Option<&str>, content: &str) -> String {
    let raw = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            content.lines().find_map(|line| {
                line.trim_start()
                    .strip_prefix("# ")
                    .map(str::trim)
                    .filter(|heading| !heading.is_empty())
            })
        })
        .unwrap_or("plan");
    let raw = raw.strip_suffix(".md").unwrap_or(raw);
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in raw.chars() {
        if slug.len() >= 48 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !slug.is_empty() && !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    let stem = if slug.is_empty() { "plan" } else { slug };
    format!("{stem}.md")
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
    async fn generated_plans_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = session_plan_dir(&dir.path().join("custom-plans"));
        let tool = WritePlan::generated_in(&session_dir);
        for (content, name, expected_file) in [
            ("# Initial Plan\nfirst", None, "initial-plan.md"),
            ("second", Some("My Custom Plan"), "my-custom-plan.md"),
            ("plain body", None, "plan.md"),
            ("plain body two", None, "plan-2.md"),
        ] {
            let input = match name {
                Some(n) => serde_json::json!({"content": content, "name": n}),
                None => serde_json::json!({"content": content}),
            };
            let result = tool
                .execute(ctx(dir.path().to_path_buf()), input)
                .await
                .unwrap();
            assert!(result.output.contains(expected_file), "{}", result.output);
        }
        let mut files: Vec<_> = std::fs::read_dir(&session_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                "initial-plan.md",
                "my-custom-plan.md",
                "plan-2.md",
                "plan.md"
            ]
        );
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
