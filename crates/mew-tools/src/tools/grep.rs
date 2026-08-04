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
        "Search file contents for a regex pattern. Returns matching lines with file path and line number. \
         By default files excluded by .gitignore/.ignore and hidden files are skipped; set \
         include_ignored or include_hidden to search them."
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
                    },
                    "include_ignored": {
                        "type": "boolean",
                        "description": "Also search files excluded by .gitignore/.ignore/git-exclude rules. Default false."
                    },
                    "include_hidden": {
                        "type": "boolean",
                        "description": "Also search hidden files and directories (dotfiles). Default false."
                    },
                    "max_file_size_mb": {
                        "type": "integer",
                        "description": "Skip files larger than this many megabytes. Default 10. 0 = no limit."
                    },
                    "context_lines": {
                        "type": "integer",
                        "description": "Number of context lines to show before and after each match. Default 0."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of matches to return. Default 500."
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "Match case-sensitively. Default false (smart case: case-insensitive unless the pattern has uppercase)."
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

        // Filtering / search-shape overrides (see schema for semantics).
        let include_ignored = input
            .get("include_ignored")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let include_hidden = input
            .get("include_hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // Either bypass flag switches us off fff's `collect_files` (which
        // hard-codes ignore/hidden filtering) onto a manual `ignore` walk.
        let bypass_filters = include_ignored || include_hidden;
        // 0 == no limit; absent == default 10 MiB ceiling.
        let max_file_size: u64 = match input.get("max_file_size_mb").and_then(|v| v.as_u64()) {
            Some(0) => u64::MAX,
            Some(mb) => mb.saturating_mul(1024 * 1024),
            None => 10 * 1024 * 1024,
        };
        let context_lines = input
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(0);
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(500);
        let case_sensitive = input
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

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

            if bypass_filters {
                collect_files_with_overrides(&mut picker, include_ignored, include_hidden)?;
            } else {
                picker
                    .collect_files()
                    .map_err(|e| ToolError::Execution(format!("failed to collect files: {e}")))?;
            }

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
                max_file_size,
                max_matches_per_file: 200,
                smart_case: !case_sensitive,
                file_offset: 0,
                page_limit: max_results,
                mode: grep_mode,
                time_budget_ms: TIME_BUDGET_MS,
                before_context: context_lines,
                after_context: context_lines,
                classify_definitions: false,
                trim_whitespace: false,
                abort_signal: Some(abort),
            };

            let result = picker.grep(&query, &options);

            // Format matches into output lines. When context_lines > 0 each
            // match is surrounded by its before/after context, emitted with the
            // same `path:linenum:content` shape so `filter_output` (which keys
            // off the first colon) still redacts/drops secret lines correctly.
            let mut results = Vec::new();
            let mut total_bytes = 0usize;
            let mut had_matches = false;

            'outer: for m in &result.matches {
                had_matches = true;
                let file = result.files.get(m.file_index);
                let path_str = file.map(|f| f.relative_path(&picker)).unwrap_or_default();

                let before = &m.context_before;
                for (i, ctx) in before.iter().enumerate() {
                    let ln = m.line_number.saturating_sub((before.len() - i) as u64);
                    if push_line(&mut results, &mut total_bytes, &path_str, ln, ctx) {
                        results.push(format!("...[truncated at {} bytes]", MAX_OUTPUT));
                        break 'outer;
                    }
                }
                if push_line(
                    &mut results,
                    &mut total_bytes,
                    &path_str,
                    m.line_number,
                    &m.line_content,
                ) {
                    results.push(format!("...[truncated at {} bytes]", MAX_OUTPUT));
                    break 'outer;
                }
                for (i, ctx) in m.context_after.iter().enumerate() {
                    let ln = m.line_number.saturating_add(1 + i as u64);
                    if push_line(&mut results, &mut total_bytes, &path_str, ln, ctx) {
                        results.push(format!("...[truncated at {} bytes]", MAX_OUTPUT));
                        break 'outer;
                    }
                }
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

            // A normal (non-bypass) search that scanned files but matched
            // nothing used to look like a silent "no matches" bug: the matches
            // were simply in gitignored/hidden files. Point the caller at the
            // escape hatches so it can retry informedly.
            if !had_matches
                && !bypass_filters
                && result.regex_fallback_error.is_none()
                && result.filtered_file_count > 0
            {
                results.push(
                    "[no matches in tracked files — if matches may live in gitignored \
                     or hidden files, retry with \"include_ignored\": true or \
                     \"include_hidden\": true]"
                        .to_string(),
                );
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

/// Truncate long match lines to `MAX_LINE_LEN` chars, not bytes: byte
/// slicing panics when a multibyte char straddles the limit.
fn truncate_match_line(line: &str) -> String {
    if line.len() <= MAX_LINE_LEN {
        return line.to_string();
    }
    let head: String = line.chars().take(MAX_LINE_LEN).collect();
    if head.len() == line.len() {
        return line.to_string();
    }
    format!("{head}...")
}

/// Append one formatted `path:linenum:content` line, respecting the overall
/// output byte cap. Returns `true` when the cap would be exceeded — in that
/// case the line is *not* pushed and the caller emits the truncation marker
/// and stops. Used for both match lines and their surrounding context.
fn push_line(
    results: &mut Vec<String>,
    total_bytes: &mut usize,
    path: &str,
    linenum: u64,
    content: &str,
) -> bool {
    let content = truncate_match_line(content);
    let formatted = format!("{}:{}:{}", path, linenum, content);
    let line_bytes = formatted.len();
    if *total_bytes + line_bytes > MAX_OUTPUT {
        return true;
    }
    *total_bytes += line_bytes;
    results.push(formatted);
    false
}

/// Populate the picker by walking the filesystem directly with the `ignore`
/// crate, bypassing the ignore/hidden filters that `FilePicker::collect_files`
/// hard-codes. `fff-search` exposes no knob for this, so we discover the files
/// ourselves and inject each through `FilePicker::add_new_file` (which marks
/// them as overflow files that the grep engine searches normally). Grep's own
/// prefilter still skips binary and oversize files, so we add everything and
/// let it classify.
fn collect_files_with_overrides(
    picker: &mut FilePicker,
    include_ignored: bool,
    include_hidden: bool,
) -> Result<(), ToolError> {
    // Clone the base path out of the picker so the mutable `add_new_file`
    // borrow below is not aliased with the walk root.
    let root = picker.base_path().to_path_buf();
    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .hidden(!include_hidden)
        .git_ignore(!include_ignored)
        .git_exclude(!include_ignored)
        .git_global(!include_ignored)
        .ignore(!include_ignored)
        .follow_links(false);
    for entry in builder.build() {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        // Never index `.git` internals, even when hidden files are included.
        if path.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        let _ = picker.add_new_file(path);
    }
    Ok(())
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
        // No real match line, but the tool now surfaces an escape-hatch hint
        // since it scanned files and found nothing.
        assert!(!result.output.contains("fn main"));
        assert!(
            result.output.contains("include_ignored"),
            "empty result should hint at ignore bypass, got: {}",
            result.output
        );
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

    #[tokio::test]
    async fn test_grep_long_line_multibyte_truncation() {
        let dir = tempfile::tempdir().unwrap();
        // Em dash (3 bytes) at bytes 499..502 straddles the 500-char
        // truncation point; the match lives past the cut so the truncated
        // head must stand in for the line content.
        let mut line = "a".repeat(MAX_LINE_LEN - 1);
        line.push('\u{2014}');
        line.push_str("needle-tail");
        tokio::fs::write(dir.path().join("long.txt"), &line)
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"pattern": "needle"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(
            result.output.contains("long.txt:1:"),
            "match line reported: {}",
            result.output
        );
        assert!(
            result.output.ends_with("..."),
            "over-long line truncated: {}",
            result.output
        );
        // The content past the cut point is dropped with the truncation.
        assert!(!result.output.contains("needle-tail"));
    }

    #[tokio::test]
    async fn test_grep_include_ignored() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join(".gitignore"), "ignored.rs\n")
            .await
            .unwrap();
        tokio::fs::create_dir(dir.path().join(".git"))
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("ignored.rs"), "fn only_in_ignored() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("visible.rs"), "fn visible() {}")
            .await
            .unwrap();

        let tool = Grep;
        // Default: the gitignored file is excluded.
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(ctx, serde_json::json!({"pattern": "only_in_ignored"}))
            .await
            .unwrap();
        assert!(!result.output.contains("ignored.rs"));

        // include_ignored: the gitignored file is searched.
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(
                ctx,
                serde_json::json!({"pattern": "only_in_ignored", "include_ignored": true}),
            )
            .await
            .unwrap();
        assert!(
            result.output.contains("ignored.rs"),
            "include_ignored should surface gitignored matches, got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_grep_include_hidden() {
        let dir = tempfile::tempdir().unwrap();
        // No .git → non-git dir, where hidden files are skipped by default.
        tokio::fs::write(dir.path().join(".hiddenfile"), "fn in_dotfile() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("visible.rs"), "fn visible() {}")
            .await
            .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(ctx, serde_json::json!({"pattern": "in_dotfile"}))
            .await
            .unwrap();
        assert!(!result.output.contains(".hiddenfile"));

        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(
                ctx,
                serde_json::json!({"pattern": "in_dotfile", "include_hidden": true}),
            )
            .await
            .unwrap();
        assert!(
            result.output.contains(".hiddenfile"),
            "include_hidden should surface dotfile matches, got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("a.txt"),
            "line1\nline2\nNEEDLE\nline4\nline5",
        )
        .await
        .unwrap();

        let tool = Grep;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(
                ctx,
                serde_json::json!({"pattern": "NEEDLE", "context_lines": 1}),
            )
            .await
            .unwrap();
        assert!(
            result.output.contains("a.txt:3:NEEDLE"),
            "match: {}",
            result.output
        );
        assert!(
            result.output.contains("a.txt:2:line2"),
            "before context: {}",
            result.output
        );
        assert!(
            result.output.contains("a.txt:4:line4"),
            "after context: {}",
            result.output
        );
        assert!(
            !result.output.contains("line1") && !result.output.contains("line5"),
            "no extra context: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_grep_max_file_size_mb() {
        let dir = tempfile::tempdir().unwrap();
        // ~1.5 MB file with the needle on its own short second line.
        let mut content = "x".repeat(1_500_000);
        content.push_str("\nNEEDLE\n");
        tokio::fs::write(dir.path().join("big.txt"), &content)
            .await
            .unwrap();

        let tool = Grep;
        // Default 10 MB ceiling → 1.5 MB file is searched.
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(ctx, serde_json::json!({"pattern": "NEEDLE"}))
            .await
            .unwrap();
        assert!(
            result.output.contains("NEEDLE"),
            "default size cap allows 1.5MB: {}",
            result.output
        );

        // 1 MB ceiling → 1.5 MB file is skipped.
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(
                ctx,
                serde_json::json!({"pattern": "NEEDLE", "max_file_size_mb": 1}),
            )
            .await
            .unwrap();
        assert!(
            !result.output.contains("NEEDLE"),
            "1MB cap should skip 1.5MB file: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_grep_case_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "Hello\nhello\nHELLO")
            .await
            .unwrap();

        let tool = Grep;
        // Smart case (default): lowercase pattern → case-insensitive → all three.
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(ctx, serde_json::json!({"pattern": "hello"}))
            .await
            .unwrap();
        assert!(result.output.contains("a.txt:1:Hello"));
        assert!(result.output.contains("a.txt:2:hello"));
        assert!(result.output.contains("a.txt:3:HELLO"));

        // case_sensitive: only the exact-case line 2.
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let result = tool
            .execute(
                ctx,
                serde_json::json!({"pattern": "hello", "case_sensitive": true}),
            )
            .await
            .unwrap();
        assert!(
            result.output.contains("a.txt:2:hello"),
            "exact match present: {}",
            result.output
        );
        assert!(
            !result.output.contains("a.txt:1:"),
            "uppercase variant excluded: {}",
            result.output
        );
        assert!(
            !result.output.contains("a.txt:3:"),
            "uppercase variant excluded: {}",
            result.output
        );
    }
}
