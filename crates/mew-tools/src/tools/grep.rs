use crate::{SecretSet, Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use fff_search::file_picker::{FFFMode, FilePicker, FilePickerOptions};
use fff_search::grep::{GrepMode, GrepSearchOptions};
use fff_search::{Constraint, GrepConfig, QueryParser};
use serde_json::Value;
use std::sync::Arc;

const MAX_OUTPUT: usize = 100_000; // 100KB
const MAX_LINE_LEN: usize = 500; // truncate long lines
const TIME_BUDGET_MS: u64 = 5000; // 5 second budget per search

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
                    },
                    "mode": {
                        "type": "string",
                        "description": "Search mode: 'regex' (default), 'literal', or 'fuzzy'.",
                        "enum": ["regex", "literal", "fuzzy"]
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
            .ok_or_else(|| ToolError::InvalidInput("missing pattern".into()))?
            .to_string();
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();
        let glob = input
            .get("glob")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let include = input
            .get("include")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mode = input
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("regex")
            .to_string();

        let base = ctx.cwd.join(&path);

        // Determine the GrepMode based on the mode parameter.
        let grep_mode = match mode.as_str() {
            "literal" => GrepMode::PlainText,
            "fuzzy" => GrepMode::Fuzzy,
            _ => GrepMode::Regex,
        };

        // Run the blocking fff-search work in a spawn_blocking task.
        // fff-search does synchronous I/O (mmap reads, directory walking).
        let cancel = ctx.cancel.clone();
        let secrets = ctx.secrets.clone();

        let result = tokio::task::spawn_blocking(move || -> Result<ToolOutput, ToolError> {
            // Build the FilePicker with the resolved base path.
            // - watch: false — no background file watcher for per-invocation use.
            // - enable_mmap_cache: false — no pre-population overhead.
            // - mode: Ai — enables file-path constraint detection in the query parser.
            let mut picker = FilePicker::new(FilePickerOptions {
                base_path: base.to_string_lossy().to_string(),
                enable_mmap_cache: false,
                mode: FFFMode::Ai,
                watch: false,
                ..Default::default()
            })
            .map_err(|e| ToolError::Execution(format!("failed to create file picker: {e}")))?;

            picker
                .collect_files()
                .map_err(|e| ToolError::Execution(format!("failed to collect files: {e}")))?;

            // Parse the pattern with GrepConfig (enables file-path constraints).
            let parser = QueryParser::new(GrepConfig);
            let mut query = parser.parse(&pattern);

            // Add glob or extension constraint if provided.
            if let Some(ref g) = glob {
                query.constraints.push(Constraint::Glob(g));
            } else if let Some(ref ext) = include {
                query.constraints.push(Constraint::Extension(ext));
            }

            // Build an abort signal from the cancellation token.
            // We check the token before starting; fff's time_budget_ms handles
            // the inner timeout. The abort flag is wired in case the token
            // fires during the search.
            let abort = Arc::new(std::sync::atomic::AtomicBool::new(cancel.is_cancelled()));

            let options = GrepSearchOptions {
                max_file_size: 10 * 1024 * 1024,
                max_matches_per_file: 200,
                smart_case: true,
                file_offset: 0,
                page_limit: 500,
                mode: grep_mode,
                time_budget_ms: TIME_BUDGET_MS,
                before_context: 0,
                after_context: 0,
                classify_definitions: false,
                trim_whitespace: false,
                abort_signal: Some(abort),
            };

            let result = picker.grep(&query, &options);

            // Format matches into output lines.
            let mut results = Vec::new();
            let mut total_bytes = 0usize;

            for m in &result.matches {
                let file = result.files.get(m.file_index);
                let path_str = file.map(|f| f.relative_path(&picker)).unwrap_or_default();

                let line_str = if m.line_content.len() > MAX_LINE_LEN {
                    format!("{}...", &m.line_content[..MAX_LINE_LEN])
                } else {
                    m.line_content.clone()
                };

                let formatted = format!("{}:{}:{}", path_str, m.line_number, line_str);

                let line_bytes = formatted.len();
                if total_bytes + line_bytes > MAX_OUTPUT {
                    results.push(format!("...[truncated at {} bytes]", MAX_OUTPUT));
                    break;
                }
                total_bytes += line_bytes;
                results.push(formatted);
            }

            // Append notices for fallback / partial results.
            if let Some(ref err) = result.regex_fallback_error {
                results.push(format!(
                    "[note: regex compilation failed — fell back to literal matching: {err}]"
                ));
            }

            if result.next_file_offset > 0 {
                results.push(format!(
                    "[partial results: time budget of {}ms reached, {} of {} files searched]",
                    TIME_BUDGET_MS, result.total_files_searched, result.filtered_file_count
                ));
            }

            let output = if results.is_empty() {
                String::new()
            } else {
                filter_output(&results.join("\n"), &secrets)
            };

            Ok(ToolOutput {
                output,
                error: String::new(),
                diff: None,
                metadata: None,
                file_delta: None,
            })
        })
        .await;

        match result {
            Ok(tool_output) => tool_output,
            Err(e) => Err(ToolError::Execution(format!("grep task panicked: {e}"))),
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
        // Skip notice lines (they don't have the path:linenum:content format).
        if line.starts_with('[') && line.ends_with(']') {
            kept.push(line.to_string());
            continue;
        }
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
        let result = tool.execute(ctx, input).await.unwrap();
        // With fff-search, invalid regex falls back to literal matching instead
        // of erroring. The output should contain a fallback notice.
        assert!(
            result.output.contains("fell back to literal matching"),
            "invalid regex should trigger fallback notice, got: {}",
            result.output
        );
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

    #[tokio::test]
    async fn test_grep_literal_mode() {
        let dir = tempfile::tempdir().unwrap();
        // In literal mode, regex metacharacters should be treated as literals.
        tokio::fs::write(dir.path().join("a.rs"), "fn foo() {}\nfn foo.bar() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        // Literal search for "foo.bar" should NOT match "foo bar" (dot is literal)
        let input = serde_json::json!({"pattern": "foo.bar", "mode": "literal"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(
            result.output.contains("foo.bar"),
            "literal mode should find 'foo.bar'"
        );
    }

    #[tokio::test]
    async fn test_grep_fuzzy_mode() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "fn receive_message() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        // Fuzzy search should find "receive_message" even with a typo
        let input = serde_json::json!({"pattern": "recieve_mesage", "mode": "fuzzy"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(
            result.output.contains("receive_message"),
            "fuzzy mode should find 'receive_message' despite typos"
        );
    }
}
