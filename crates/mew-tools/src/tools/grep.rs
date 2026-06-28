use crate::{SecretSet, Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

const MAX_OUTPUT: usize = 100_000; // 100KB

pub struct Grep;

#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents for a pattern. Prefers ripgrep if available."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Pattern to search for."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (default: current directory)."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob filter for files to search (e.g. '*.rs')."
                    }
                },
                "required": ["pattern"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing pattern".into()))?;
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let glob = input.get("glob").and_then(|v| v.as_str());
        let base = ctx.cwd.join(path);

        // Try ripgrep first
        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("--line-number")
            .arg("--with-filename")
            .arg("-H") // always show filename
            .arg(pattern)
            .current_dir(&base);

        if let Some(g) = glob {
            cmd.arg("--glob").arg(g);
        }

        let output = cmd.output().await;

        match output {
            Ok(output) => {
                let stdout = format_output(&output.stdout, MAX_OUTPUT);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // rg exits 1 when no matches found, which is not an error
                if output.status.success() || output.status.code() == Some(1) {
                    Ok(ToolOutput {
                        output: filter_output(&stdout, &ctx.secrets),
                        error: String::new(),
                        diff: None,
                        metadata: None,
                    })
                } else {
                    Err(ToolError::Execution(format!("rg failed: {}", stderr)))
                }
            }
            Err(_) => {
                // Fallback to grep -r
                let mut cmd = tokio::process::Command::new("grep");
                cmd.arg("-r")
                    .arg("-n")
                    .arg("-H")
                    .arg(pattern)
                    .current_dir(&base);

                if let Some(g) = glob {
                    cmd.arg("--include").arg(g);
                }

                let output = cmd
                    .output()
                    .await
                    .map_err(|e| ToolError::Execution(format!("grep failed: {}", e)))?;

                Ok(ToolOutput {
                    output: filter_output(&format_output(&output.stdout, MAX_OUTPUT), &ctx.secrets),
                    error: String::new(),
                    diff: None,
                    metadata: None,
                })
            }
        }
    }
}

/// Drop results from secret files and redact lines containing secret words.
/// grep output format is `path:linenum:content`; the path is the segment
/// before the first colon.
fn filter_output(output: &str, secrets: &SecretSet) -> String {
    if secrets.is_empty() {
        return output.to_string();
    }
    let matchers: Vec<globset::GlobMatcher> = secrets
        .globs
        .iter()
        .filter_map(|g| globset::Glob::new(g).ok().map(|g| g.compile_matcher()))
        .collect();
    let has_globs = !matchers.is_empty();
    let has_words = secrets.words.iter().any(|w| !w.is_empty());
    if !has_globs && !has_words {
        return output.to_string();
    }

    let mut kept = Vec::new();
    let mut redacted = 0usize;
    let mut dropped = 0usize;
    for line in output.lines() {
        if has_globs {
            let path = line.split(':').next().unwrap_or("");
            if !path.is_empty() && matchers.iter().any(|m| m.is_match(path)) {
                dropped += 1;
                continue;
            }
        }
        if has_words
            && secrets
                .words
                .iter()
                .any(|w| !w.is_empty() && line.contains(w.as_str()))
        {
            redacted += 1;
            kept.push(redact_line(line));
            continue;
        }
        kept.push(line.to_string());
    }

    let mut result = kept.join("\n");
    if redacted > 0 || dropped > 0 {
        result.push_str(&format!(
            "\n[{} secret-bearing line(s) redacted, {} result(s) from secret files dropped]",
            redacted, dropped
        ));
    }
    result
}

/// Preserve the `path:linenum:` prefix, redact the matched content.
fn redact_line(line: &str) -> String {
    let mut parts = line.splitn(3, ':');
    match (parts.next(), parts.next()) {
        (Some(path), Some(linenum)) => {
            format!("{}:{}:[redacted — secret value]", path, linenum)
        }
        _ => "[redacted — secret value]".to_string(),
    }
}

fn format_output(raw: &[u8], max: usize) -> String {
    let text = String::from_utf8_lossy(raw);
    if text.len() > max {
        let truncated = &text[..max];
        format!("{}\n...[truncated {} bytes]", truncated, text.len() - max)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    fn ctx_with_secrets(cwd: PathBuf, words: Vec<&str>, globs: Vec<&str>) -> ToolCtx {
        let secrets = std::sync::Arc::new(SecretSet {
            words: words.iter().map(|s| s.to_string()).collect(),
            globs: globs.iter().map(|s| s.to_string()).collect(),
        });
        ToolCtx::test_with_secrets(cwd, secrets)
    }

    #[tokio::test]
    async fn test_grep() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}\nfn foo() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.rs"), "fn bar() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("c.txt"), "fn baz() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "fn foo"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("a.rs"));
        assert!(result.output.contains("fn foo"));
    }

    #[tokio::test]
    async fn test_grep_glob_filter() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "fn main() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "fn main", "glob": "*.rs"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("a.rs"));
        assert!(!result.output.contains("b.txt"));
    }

    #[test]
    fn test_filter_output_redacts_secret_words() {
        let secrets = SecretSet {
            words: vec!["ghp_supersecret123".to_string()],
            globs: vec![],
        };
        let input = "a.rs:1:api_key: ghp_supersecret123\na.rs:2:fn foo() {}";
        let out = filter_output(input, &secrets);
        assert!(!out.contains("ghp_supersecret123"));
        assert!(out.contains("[redacted"), "redaction marker present");
        assert!(
            out.contains("fn foo() {}"),
            "non-secret line passes through unchanged"
        );
    }

    #[test]
    fn test_filter_output_drops_secret_files() {
        let secrets = SecretSet {
            words: vec![],
            globs: vec![".env".to_string(), "**/credentials.json".to_string()],
        };
        let input =
            ".env:1:SECRET=value\nsrc/credentials.json:5:token\nsrc/main.rs:42:fn main() {}";
        let out = filter_output(input, &secrets);
        assert!(!out.contains(".env:1"), "literal secret file dropped");
        assert!(
            !out.contains("credentials.json:5"),
            "glob-matched secret file dropped"
        );
        assert!(
            out.contains("src/main.rs:42"),
            "non-secret file passes through"
        );
    }

    #[test]
    fn test_filter_output_noop_when_empty() {
        let secrets = SecretSet::default();
        let input = "a.rs:1:hello\nb.rs:2:world";
        let out = filter_output(input, &secrets);
        assert_eq!(out, input);
    }

    #[tokio::test]
    async fn test_grep_applies_secret_filter_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("config.rs"),
            "let key = \"ghp_supersecret123\";\nfn ok() {}",
        )
        .await
        .unwrap();
        let tool = Grep;
        let ctx = ctx_with_secrets(dir.path().to_path_buf(), vec!["ghp_supersecret123"], vec![]);
        let input = serde_json::json!({"pattern": "ghp_|fn ok"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(
            !result.output.contains("ghp_supersecret123"),
            "secret word must be redacted in real grep output"
        );
        assert!(
            result.output.contains("[redacted"),
            "redaction marker present"
        );
    }
}
