use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;

pub struct Read;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

/// Returns the image MIME type for a path if its extension is a recognized
/// image format.
fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

#[async_trait]
impl Tool for Read {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Output includes a [path#hash] header and \
         line-numbered content so follow-up hashline edits can target exact \
         lines. Image files (PNG, JPEG, GIF, WebP, BMP) are returned as image \
         content when the model supports vision."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file, relative to the current working directory."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (0-indexed)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read."
                    }
                },
                "required": ["path"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("missing path".into()))?;
        let abs_path = ctx.cwd.join(path);
        let display_path = abs_path
            .strip_prefix(&ctx.cwd)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string());

        // Check file size before reading.
        let metadata = tokio::fs::metadata(&abs_path)
            .await
            .map_err(|e| ToolError::Execution(format!("{}: {}", abs_path.display(), e)))?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(ToolError::Execution(format!(
                "{}: file too large ({} bytes, max {})",
                abs_path.display(),
                metadata.len(),
                MAX_FILE_SIZE
            )));
        }

        // If the file has a recognized image extension, return it as image
        // content for vision-capable models.
        if let Some(mime) = image_mime(&abs_path) {
            if ctx.supports_vision {
                let bytes = tokio::fs::read(&abs_path)
                    .await
                    .map_err(|e| ToolError::Execution(format!("{}: {}", abs_path.display(), e)))?;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                return Ok(ToolOutput {
                    output: format!(
                        "[Image: {} ({} bytes, {})]",
                        display_path,
                        bytes.len(),
                        mime
                    ),
                    error: String::new(),
                    diff: None,
                    metadata: None,
                    file_delta: None,
                    images: vec![mew_hooks::ToolImage {
                        mime: mime.to_string(),
                        data: b64,
                    }],
                });
            } else {
                // No vision support — return a helpful text message so the
                // model knows the file exists but can't be viewed.
                return Ok(ToolOutput {
                    output: format!(
                        "[Binary image file: {} ({} bytes, {}). \
                         This model does not support image input.]",
                        display_path,
                        metadata.len(),
                        mime
                    ),
                    error: String::new(),
                    diff: None,
                    metadata: None,
                    file_delta: None,
                    images: vec![],
                });
            }
        }

        let content = tokio::fs::read_to_string(&abs_path)
            .await
            .map_err(|e| ToolError::Execution(format!("{}: {}", abs_path.display(), e)))?;

        // Detect binary files via null byte.
        if content.contains('\0') {
            return Err(ToolError::Execution(format!(
                "{}: cannot read binary file",
                abs_path.display()
            )));
        }

        let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        // Normalize and record a snapshot for hashline edits before any
        // redaction or slicing changes the visible text.
        let normalized =
            mew_hashline::format::normalize_to_lf(mew_hashline::format::strip_bom(&content).0);
        let hash = mew_hashline::format::compute_file_hash(&normalized);

        let all_lines: Vec<&str> = content.lines().collect();
        let start = offset.min(all_lines.len());
        let end = limit
            .map(|l| (start + l).min(all_lines.len()))
            .unwrap_or(all_lines.len());
        let displayed = all_lines[start..end].join("\n");

        let seen_lines: Vec<usize> = (start..end).map(|i| i + 1).collect();
        let canonical = abs_path.to_string_lossy().to_string();
        ctx.snapshot_store
            .record(&canonical, &normalized, Some(&seen_lines));

        // Strip configured secret words from the returned content. This is
        // a second line of defense behind the permission-engine pre-check:
        // even when the user approves reading a file, individual secret
        // values (API keys, tokens) are still scrubbed so they do not land
        // in the model's context. The structure (variable names, line
        // shape) is preserved so the model can still reason about the file.
        let (displayed, redacted) = crate::secrets::redact_secret_words(&displayed, &ctx.secrets);
        let displayed = crate::secrets::annotate_redaction(displayed, redacted);

        let start_line = start + 1;
        let numbered = mew_hashline::format::format_numbered_lines(&displayed, start_line);
        let header = mew_hashline::format::format_hashline_header(&display_path, &hash);
        let output = format!("{header}\n{numbered}");

        Ok(ToolOutput {
            output,
            error: String::new(),
            diff: None,
            metadata: None,
            file_delta: None,
            images: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretSet;
    use std::path::PathBuf;

    fn dummy_ctx(cwd: PathBuf) -> ToolCtx {
        ToolCtx::test_new(cwd)
    }

    fn ctx_with_secret_words(cwd: PathBuf, words: Vec<&str>) -> ToolCtx {
        let secrets = std::sync::Arc::new(SecretSet {
            words: words.iter().map(|s| s.to_string()).collect(),
            globs: vec![],
        });
        ToolCtx::test_with_secrets(cwd, secrets)
    }

    #[tokio::test]
    async fn test_read_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "test.txt"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("hello world"));
        assert!(result.output.starts_with("[test.txt#"));
        assert!(result.output.contains("1:hello world"));
    }

    #[tokio::test]
    async fn test_read_offset_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "line1\nline2\nline3\nline4")
            .await
            .unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "test.txt", "offset": 1, "limit": 2});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("2:line2"));
        assert!(result.output.contains("3:line3"));
        assert!(!result.output.contains("line1"));
    }

    #[tokio::test]
    async fn test_read_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "missing.txt"});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_redacts_secret_words() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.txt");
        tokio::fs::write(&path, "API_KEY=AKIAIOSFODNN7EXAMPLE\nPORT=3000")
            .await
            .unwrap();

        let tool = Read;
        let ctx = ctx_with_secret_words(dir.path().to_path_buf(), vec!["AKIAIOSFODNN7EXAMPLE"]);
        let input = serde_json::json!({"path": "config.txt"});
        let result = tool.execute(ctx, input).await.unwrap();
        // The secret value is gone; the variable name and surrounding
        // structure survive so the model can still reason about the file.
        assert!(!result.output.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(result.output.contains("API_KEY=[REDACTED]"));
        assert!(result.output.contains("PORT=3000"));
        assert!(result.output.contains("redacted"));
    }

    #[tokio::test]
    async fn test_read_no_redaction_without_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.txt");
        tokio::fs::write(&path, "just text").await.unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "plain.txt"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.output.contains("just text"));
        assert!(!result.output.contains("redacted"));
    }

    /// Regression: every read error must include the file path so the model
    /// can fix its `path` argument without guessing.
    #[tokio::test]
    async fn test_read_missing_error_includes_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "does_not_exist.txt"});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does_not_exist.txt"),
            "expected the missing path in error: {err}"
        );
    }

    /// Regression: the binary-file error must include the path so the model
    /// knows which file it accidentally targeted.
    #[tokio::test]
    async fn test_read_binary_error_includes_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        // 4 NUL bytes — enough to trip the binary detector.
        tokio::fs::write(&path, [0u8, 1, 0, 1]).await.unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "blob.bin"});
        let result = tool.execute(ctx, input).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("blob.bin"),
            "expected the binary path in error: {err}"
        );
        assert!(err.contains("binary"), "expected 'binary' in error: {err}");
    }

    #[tokio::test]
    async fn test_read_image_with_vision() {
        let dir = tempfile::tempdir().unwrap();
        // Minimal valid PNG: 1x1 red pixel.
        let png_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00,
            0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00,
            0x03, 0x00, 0x01, 0x5E, 0x99, 0xF5, 0x4A, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
            0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let path = dir.path().join("photo.png");
        tokio::fs::write(&path, png_bytes).await.unwrap();

        let tool = Read;
        let ctx = ToolCtx::test_with_vision(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "photo.png"});
        let result = tool.execute(ctx, input).await.unwrap();

        // The text output mentions the image.
        assert!(result.output.contains("[Image:"));
        assert!(result.output.contains("photo.png"));
        assert!(result.output.contains("image/png"));

        // One image is returned.
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].mime, "image/png");
        // The base64 data should decode back to the original bytes.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&result.images[0].data)
            .unwrap();
        assert_eq!(decoded, png_bytes);
    }

    #[tokio::test]
    async fn test_read_image_without_vision() {
        let dir = tempfile::tempdir().unwrap();
        let png_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let path = dir.path().join("photo.jpg");
        tokio::fs::write(&path, png_bytes).await.unwrap();

        let tool = Read;
        let ctx = dummy_ctx(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "photo.jpg"});
        let result = tool.execute(ctx, input).await.unwrap();

        // No image data — model can't see it.
        assert!(result.images.is_empty());

        // But the text output tells the model what happened.
        assert!(result.output.contains("Binary image file"));
        assert!(result.output.contains("photo.jpg"));
        assert!(result.output.contains("does not support image input"));
    }

    #[tokio::test]
    async fn test_read_image_jpeg_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img.jpeg");
        tokio::fs::write(&path, b"fake jpeg").await.unwrap();

        let tool = Read;
        let ctx = ToolCtx::test_with_vision(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "img.jpeg"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0].mime, "image/jpeg");
    }

    #[tokio::test]
    async fn test_read_non_image_unchanged() {
        // Ensure text files still go through the normal path with vision on.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code.rs");
        tokio::fs::write(&path, "fn main() {}").await.unwrap();

        let tool = Read;
        let ctx = ToolCtx::test_with_vision(dir.path().to_path_buf());
        let input = serde_json::json!({"path": "code.rs"});
        let result = tool.execute(ctx, input).await.unwrap();
        assert!(result.images.is_empty());
        assert!(result.output.contains("fn main()"));
    }
}
