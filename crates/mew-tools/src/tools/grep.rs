use crate::{SecretSet, Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use grep::regex::RegexMatcher;
use grep::searcher::{sinks::UTF8, SearcherBuilder};
use ignore::WalkBuilder;
use serde_json::Value;

const MAX_OUTPUT: usize = 100_000; // 100KB

pub struct Grep;

#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents for a regex pattern. Returns matching lines with file path and line number."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search in (default: current directory)."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob filter for files to search (e.g. '*.rs')."
                    },
                    "include": {
                        "type": "string",
                        "description": "File extension filter without the dot (e.g. 'rs', 'py'). Shorthand for glob."
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
        let include = input.get("include").and_then(|v| v.as_str());

        let base = ctx.cwd.join(path);

        let matcher = RegexMatcher::new(pattern)
            .map_err(|e| ToolError::InvalidInput(format!("invalid regex: {e}")))?;

        let searcher = SearcherBuilder::new()
            .binary_detection(grep::searcher::BinaryDetection::quit(b'\x00'))
            .line_number(true)
            .build();

        let mut results = Vec::new();
        let mut total_bytes = 0usize;

        // Build the directory walker with .gitignore support.
        let mut builder = WalkBuilder::new(&base);
        builder.hidden(false);
        builder.git_ignore(true);
        builder.git_exclude(true);
        builder.git_global(true);

        // Apply glob/include filter via ignore's Override system.
        if let Some(g) = glob.or_else(|| {
            include.map(|e| {
                // Convert include extension to glob pattern.
                // Leak is fine: bounded and only called once per tool invocation.
                Box::leak(format!("*.{e}").into_boxed_str()) as &str
            })
        }) {
            let mut overrides = ignore::overrides::OverrideBuilder::new(&base);
            if overrides.add(g).is_ok() {
                if let Ok(built) = overrides.build() {
                    builder.overrides(built);
                }
            }
        }

        let walker = builder.build();

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }

            let path_str = entry
                .path()
                .strip_prefix(&base)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .to_string();

            let file_path = entry.path().to_path_buf();

            // Run the search in a blocking task since grep's searcher
            // does synchronous I/O (memory-mapped reads).
            let matcher = matcher.clone();
            let searcher = searcher.clone();
            let search_result = tokio::task::spawn_blocking(move || {
                let mut file_searcher = searcher;
                let file_results = std::sync::Mutex::new(Vec::new());

                let sink = UTF8(|lnum, line| {
                    let line_str = if line.len() > 500 {
                        format!("{}...", &line[..500])
                    } else {
                        line.to_string()
                    };
                    let formatted = format!("{}:{}:{}", path_str, lnum, line_str);
                    file_results.lock().unwrap().push(formatted);
                    Ok(true)
                });

                match file_searcher.search_path(&matcher, &file_path, sink) {
                    Ok(()) => Ok(file_results.into_inner().unwrap()),
                    Err(e) => Err(e),
                }
            })
            .await;

            if let Ok(Ok(file_matches)) = search_result {
                for m in file_matches {
                    let line_bytes = m.len();
                    if total_bytes + line_bytes > MAX_OUTPUT {
                        results.push(format!("...[truncated at {} bytes]", MAX_OUTPUT));
                        let output = filter_output(&results.join("\n"), &ctx.secrets);
                        return Ok(ToolOutput {
                            output,
                            error: String::new(),
                            diff: None,
                            metadata: None,
                        });
                    }
                    total_bytes += line_bytes;
                    results.push(m);
                }
            }
        }

        let output = if results.is_empty() {
            String::new()
        } else {
            filter_output(&results.join("\n"), &ctx.secrets)
        };

        Ok(ToolOutput {
            output,
            error: String::new(),
            diff: None,
            metadata: None,
        })
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

    #[tokio::test]
    async fn test_grep_include_filter() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "fn main() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "fn main", "include": "rs"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("a.rs"));
        assert!(!result.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_grep_regex_pattern() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("a.rs"),
            "fn foo() {}\nfn bar() {}\nfn foobar() {}",
        )
        .await
        .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "fn foo"});
        let result = tool.execute(ctx, input).await.unwrap();
        // Should match "fn foo()" and "fn foobar()" (regex, not literal)
        assert!(result.output.contains("fn foo()"));
        assert!(result.output.contains("fn foobar()"));
        assert!(!result.output.contains("fn bar()"));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "nonexistent_pattern"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn main() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "[unclosed"});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid regex"));
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

    #[tokio::test]
    async fn test_grep_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        // Create a .gitignore
        tokio::fs::write(dir.path().join(".gitignore"), "ignored.rs\n")
            .await
            .unwrap();
        // Create a git repo marker
        tokio::fs::create_dir(dir.path().join(".git"))
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("ignored.rs"), "fn ignored() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("visible.rs"), "fn visible() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "fn "});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("visible.rs"));
        assert!(!result.output.contains("ignored.rs"));
    }
}
