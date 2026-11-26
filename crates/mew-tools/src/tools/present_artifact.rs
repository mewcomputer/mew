use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path};

pub struct PresentArtifact;

pub const ARTIFACT_METADATA_KEY: &str = "mew_artifact";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactMetadata {
    pub artifact_id: String,
    pub title: String,
    pub url: String,
    pub mode: ArtifactModeWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactModeWire {
    Panel,
    Inline,
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".next",
    "dist",
    ".venv",
    "__pycache__",
    ".mew-artifacts",
    "artifacts",
];
const MAX_STAGED_FILES: usize = 256;
const MAX_STAGED_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Default)]
struct CopyBudget {
    files: usize,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ArtifactBlobEntry {
    hash: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ArtifactManifest {
    version: u32,
    files: BTreeMap<String, ArtifactBlobEntry>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn store_blob_bytes(blobs_dir: &Path, bytes: &[u8]) -> Result<ArtifactBlobEntry, ToolError> {
    std::fs::create_dir_all(blobs_dir)
        .map_err(|e| ToolError::Execution(format!("failed to create artifact blobs dir: {e}")))?;
    let hash = sha256_hex(bytes);
    let blob_path = blobs_dir.join(&hash);
    if !blob_path.exists() {
        let tmp = blobs_dir.join(format!("{hash}.{}.tmp", ulid::Ulid::new()));
        std::fs::write(&tmp, bytes)
            .map_err(|e| ToolError::Execution(format!("failed to write artifact blob: {e}")))?;
        if let Err(err) = std::fs::rename(&tmp, &blob_path) {
            let _ = std::fs::remove_file(&tmp);
            if !blob_path.exists() {
                return Err(ToolError::Execution(format!(
                    "failed to commit artifact blob: {err}"
                )));
            }
        }
    }
    Ok(ArtifactBlobEntry {
        hash,
        size: bytes.len() as u64,
    })
}

fn stage_file_to_blobs(
    src: &Path,
    rel_key: String,
    blobs_dir: &Path,
    files: &mut BTreeMap<String, ArtifactBlobEntry>,
    budget: &mut CopyBudget,
) -> Result<(), ToolError> {
    if budget.files >= MAX_STAGED_FILES {
        return Err(ToolError::Execution(format!(
            "artifact exceeds {MAX_STAGED_FILES} file limit"
        )));
    }
    let meta = std::fs::metadata(src)
        .map_err(|e| ToolError::Execution(format!("failed to stat {}: {e}", src.display())))?;
    let next_bytes = budget.bytes.saturating_add(meta.len());
    if next_bytes > MAX_STAGED_BYTES {
        return Err(ToolError::Execution(format!(
            "artifact exceeds {} MiB size limit",
            MAX_STAGED_BYTES / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(src)
        .map_err(|e| ToolError::Execution(format!("failed to read artifact file: {e}")))?;
    let entry = store_blob_bytes(blobs_dir, &bytes)?;
    files.insert(rel_key, entry);
    budget.files += 1;
    budget.bytes = next_bytes;
    Ok(())
}

fn stage_dir_to_blobs(
    src: &Path,
    rel_prefix: &str,
    skip_path: &Path,
    blobs_dir: &Path,
    files: &mut BTreeMap<String, ArtifactBlobEntry>,
    budget: &mut CopyBudget,
) -> Result<(), ToolError> {
    let entries = std::fs::read_dir(src)
        .map_err(|e| ToolError::Execution(format!("failed to read {}: {e}", src.display())))?;
    for entry in entries {
        let entry =
            entry.map_err(|e| ToolError::Execution(format!("failed to read dir entry: {e}")))?;
        let entry_path = entry.path();
        if entry_path == skip_path {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let file_type = entry
            .file_type()
            .map_err(|e| ToolError::Execution(format!("failed to inspect entry: {e}")))?;
        let rel_child = if rel_prefix.is_empty() {
            name_str.to_string()
        } else {
            format!("{rel_prefix}/{name_str}")
        };
        if file_type.is_dir() {
            if SKIP_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            stage_dir_to_blobs(&entry_path, &rel_child, skip_path, blobs_dir, files, budget)?;
        } else if file_type.is_file() {
            stage_file_to_blobs(&entry_path, rel_child, blobs_dir, files, budget)?;
        }
    }
    Ok(())
}

fn encode_uri_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for byte in seg.bytes() {
        if matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        ) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn normalize_encoded_rel_entry(path: &Path) -> Result<String, ToolError> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => parts.push(encode_uri_segment(&s.to_string_lossy())),
            _ => {
                return Err(ToolError::InvalidInput(
                    "invalid artifact entry path".into(),
                ))
            }
        }
    }
    if parts.is_empty() {
        Ok("index.html".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

fn normalize_source_key(entry: &Path, canonical_cwd: &Path) -> String {
    let rel = entry.strip_prefix(canonical_cwd).unwrap_or(entry);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        "index.html".to_string()
    } else {
        parts.join("/")
    }
}

fn allocate_artifact_version(artifacts_root: &Path, source_key: &str) -> Result<u32, ToolError> {
    static VERSION_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = VERSION_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| ToolError::Execution("artifact version lock poisoned".into()))?;
    std::fs::create_dir_all(artifacts_root)
        .map_err(|e| ToolError::Execution(format!("failed to create artifacts root: {e}")))?;
    let versions_path = artifacts_root.join("versions.json");
    let mut versions: std::collections::BTreeMap<String, u32> = std::fs::read(&versions_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let next = versions
        .get(source_key)
        .copied()
        .unwrap_or(0)
        .saturating_add(1);
    versions.insert(source_key.to_string(), next);
    let encoded = serde_json::to_vec_pretty(&versions)
        .map_err(|e| ToolError::Execution(format!("failed to serialize artifact versions: {e}")))?;
    let tmp = versions_path.with_extension("json.tmp");
    std::fs::write(&tmp, encoded)
        .map_err(|e| ToolError::Execution(format!("failed to write artifact versions: {e}")))?;
    std::fs::rename(&tmp, &versions_path)
        .map_err(|e| ToolError::Execution(format!("failed to save artifact versions: {e}")))?;
    Ok(next)
}

const INLINE_HEAD_INJECT: &str = r#"<style id="mew-default-theme">
:root {
  color-scheme: light dark;
  --background: #ffffff;
  --foreground: #18181b;
  --card: #f4f4f5;
  --card-foreground: #18181b;
  --popover: #ffffff;
  --popover-foreground: #18181b;
  --primary: #18181b;
  --primary-foreground: #fafafa;
  --secondary: #f4f4f5;
  --secondary-foreground: #18181b;
  --muted: #f4f4f5;
  --muted-foreground: #71717a;
  --accent: #f4f4f5;
  --accent-foreground: #18181b;
  --destructive: #dc2626;
  --destructive-foreground: #ffffff;
  --border: #e4e4e7;
  --input: #e4e4e7;
  --ring: #a1a1aa;
  --chart-1: #16a34a;
  --chart-2: #2563eb;
  --chart-3: #d97706;
  --chart-4: #9333ea;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme]) {
    --background: #09090b;
    --foreground: #fafafa;
    --card: #18181b;
    --card-foreground: #fafafa;
    --popover: #09090b;
    --popover-foreground: #fafafa;
    --primary: #fafafa;
    --primary-foreground: #18181b;
    --secondary: #27272a;
    --secondary-foreground: #fafafa;
    --muted: #27272a;
    --muted-foreground: #a1a1aa;
    --accent: #27272a;
    --accent-foreground: #fafafa;
    --destructive: #ef4444;
    --destructive-foreground: #ffffff;
    --border: #27272a;
    --input: #27272a;
    --ring: #71717a;
    --chart-1: #22c55e;
    --chart-2: #3b82f6;
    --chart-3: #f59e0b;
    --chart-4: #a855f7;
  }
}
html, body {
  margin: 0;
  padding: 0;
  background: var(--background);
  color: var(--foreground);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
}
</style>
<link rel="stylesheet" href="/artifact-runtime/inline.css" />
<script id="mew-inline-runtime">
(() => {
  const applyTheme = (msg) => {
    if (!msg || msg.type !== "mew:theme") return;
    const root = document.documentElement;
    root.classList.toggle("dark", Boolean(msg.isDark));
    root.style.colorScheme = msg.isDark ? "dark" : "light";
    if (msg.theme) root.setAttribute("data-theme", msg.theme);
    if (msg.vars && typeof msg.vars === "object") {
      for (const [k, v] of Object.entries(msg.vars)) {
        if (typeof v === "string") root.style.setProperty(k, v);
      }
    }
  };
  window.addEventListener("message", (event) => applyTheme(event.data));
  const sendHeight = () => {
    const h = Math.ceil(document.documentElement.scrollHeight || document.body?.scrollHeight || 0);
    if (h > 0) parent.postMessage({ type: "mew:artifact-resize", height: h }, "*");
  };
  if (typeof ResizeObserver !== "undefined") {
    new ResizeObserver(sendHeight).observe(document.documentElement);
  }
  window.addEventListener("load", sendHeight);
  setTimeout(sendHeight, 50);
})();
</script>"#;

fn inject_inline_runtime_html(raw: &str) -> String {
    if raw.contains("mew-inline-runtime") {
        return raw.to_string();
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("<!doctype") || lower.contains("<html") {
        if let Some(pos) = lower.find("</head>") {
            format!("{}{}\n{}", &raw[..pos], INLINE_HEAD_INJECT, &raw[pos..])
        } else if let Some(pos) = lower.find("<body") {
            format!(
                "{}<head>{}</head>\n{}",
                &raw[..pos],
                INLINE_HEAD_INJECT,
                &raw[pos..]
            )
        } else {
            format!("{INLINE_HEAD_INJECT}\n{raw}")
        }
    } else {
        format!(
            "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\" />\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n{INLINE_HEAD_INJECT}\n</head>\n<body>\n<div class=\"mew-inline-root\">\n{raw}\n</div>\n</body>\n</html>\n"
        )
    }
}

#[async_trait]
impl Tool for PresentArtifact {
    fn name(&self) -> &str {
        "present_artifact"
    }

    fn description(&self) -> &str {
        "Publish an HTML/JS/CSS web app or interactive artifact from the session workspace. Workflow: first write the HTML/JS/CSS file(s) in the workspace, then call present_artifact with title, path, and mode. Staging: a file at the workspace root is staged alone, while a subdirectory (or file inside one, e.g. 'sign-maker/index.html') stages the whole subdirectory with its CSS/JS/assets. Choose mode='inline' for compact, single-purpose visuals in chat (charts, diagrams, calculators; bare HTML fragments work) or mode='panel' for full apps/dashboards/games. Load the built-in 'artifacts' skill for CSS theme variables, utility classes, Preact runtime, and layout guidelines."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short human-readable title for the artifact (e.g. 'Sign Maker')."
                    },
                    "path": {
                        "type": "string",
                        "description": "Path to the HTML entry file (or directory containing index.html) inside the workspace."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["panel", "inline"],
                        "description": "Display mode: 'inline' for compact, single-purpose visuals/widgets embedded in chat (charts, diagrams, calculators), or 'panel' (default) for full apps and multi-pane tools."
                    },
                    "height": {
                        "type": "integer",
                        "minimum": 120,
                        "maximum": 900,
                        "description": "Optional initial height in pixels for inline mode."
                    }
                },
                "required": ["title", "path"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::Mutating
    }

    async fn execute(&self, ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let title = input
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidInput("missing 'title'".into()))?
            .to_string();
        let raw_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidInput("missing 'path'".into()))?
            .to_string();
        let mode = match input.get("mode").and_then(|v| v.as_str()) {
            None | Some("panel") => ArtifactModeWire::Panel,
            Some("inline") => ArtifactModeWire::Inline,
            Some(other) => {
                return Err(ToolError::InvalidInput(format!(
                    "unsupported mode {other:?}; expected 'panel' or 'inline'"
                )))
            }
        };
        let height = input
            .get("height")
            .and_then(|v| v.as_u64())
            .map(|h| h.clamp(120, 900) as u32);

        let session_root = ctx
            .session_dir
            .clone()
            .or_else(|| {
                ctx.flagged_files_path
                    .as_ref()
                    .and_then(|p| p.parent().map(Path::to_path_buf))
            })
            .ok_or_else(|| {
                ToolError::Execution("present_artifact requires an active session directory".into())
            })?;
        let session_segment = session_root
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::Execution("invalid session directory".into()))?
            .to_string();

        let cwd = ctx.cwd.clone();
        let artifact_id = format!("art_{}", ulid::Ulid::new());
        let artifacts_root = session_root.join("artifacts");
        let dest_dir = artifacts_root.join(&artifact_id);
        let dest_dir_for_task = dest_dir.clone();
        let artifacts_root_for_task = artifacts_root.clone();

        let (entry_rel, source_path, version) =
            tokio::task::spawn_blocking(move || -> Result<(String, String, u32), ToolError> {
                let canonical_cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
                let candidate = Path::new(&raw_path);
                let joined = if candidate.is_absolute() {
                    candidate.to_path_buf()
                } else {
                    cwd.join(candidate)
                };
                let resolved = joined.canonicalize().map_err(|e| {
                    ToolError::InvalidInput(format!("cannot access {raw_path}: {e}"))
                })?;
                if !resolved.starts_with(&canonical_cwd) {
                    return Err(ToolError::InvalidInput(format!(
                        "artifact path must be inside session workspace: {raw_path}"
                    )));
                }

                let mut budget = CopyBudget::default();
                let blobs_dir = artifacts_root_for_task.join("blobs");
                let mut files = BTreeMap::new();
                let (entry_rel, entry_key, entry_source) = if resolved.is_dir() {
                    let index = resolved.join("index.html");
                    if !index.is_file() {
                        return Err(ToolError::InvalidInput(format!(
                            "directory {raw_path} does not contain index.html"
                        )));
                    }
                    stage_dir_to_blobs(
                        &resolved,
                        "",
                        &dest_dir_for_task,
                        &blobs_dir,
                        &mut files,
                        &mut budget,
                    )?;
                    ("index.html".to_string(), "index.html".to_string(), index)
                } else if resolved.is_file() {
                    let parent = resolved.parent().ok_or_else(|| {
                        ToolError::InvalidInput("invalid artifact file path".into())
                    })?;
                    let (rel_encoded, rel_key) = if parent == canonical_cwd {
                        let file_name = resolved
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("index.html");
                        stage_file_to_blobs(
                            &resolved,
                            file_name.to_string(),
                            &blobs_dir,
                            &mut files,
                            &mut budget,
                        )?;
                        let uploads = canonical_cwd.join("uploads");
                        if uploads.is_dir() {
                            stage_dir_to_blobs(
                                &uploads,
                                "uploads",
                                &dest_dir_for_task,
                                &blobs_dir,
                                &mut files,
                                &mut budget,
                            )?;
                        }
                        (
                            normalize_encoded_rel_entry(Path::new(file_name))?,
                            file_name.to_string(),
                        )
                    } else {
                        stage_dir_to_blobs(
                            parent,
                            "",
                            &dest_dir_for_task,
                            &blobs_dir,
                            &mut files,
                            &mut budget,
                        )?;
                        let rel = resolved
                            .strip_prefix(parent)
                            .unwrap_or(Path::new("index.html"));
                        let rel_key = rel
                            .components()
                            .filter_map(|c| match c {
                                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("/");
                        (normalize_encoded_rel_entry(rel)?, rel_key)
                    };
                    (rel_encoded, rel_key, resolved.clone())
                } else {
                    return Err(ToolError::InvalidInput(format!(
                        "artifact path is neither a file nor a directory: {raw_path}"
                    )));
                };
                if mode == ArtifactModeWire::Inline {
                    let raw = std::fs::read_to_string(&entry_source).map_err(|e| {
                        ToolError::Execution(format!("failed to read staged artifact: {e}"))
                    })?;
                    let wrapped = inject_inline_runtime_html(&raw);
                    let entry_blob = store_blob_bytes(&blobs_dir, wrapped.as_bytes())?;
                    files.insert(entry_key, entry_blob);
                }
                let source_path = normalize_source_key(&entry_source, &canonical_cwd);
                let version = allocate_artifact_version(&artifacts_root_for_task, &source_path)?;
                std::fs::create_dir_all(&dest_dir_for_task).map_err(|e| {
                    ToolError::Execution(format!("failed to create artifact dir: {e}"))
                })?;
                let manifest = ArtifactManifest { version, files };
                let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| {
                    ToolError::Execution(format!("failed to serialize artifact manifest: {e}"))
                })?;
                let manifest_path = dest_dir_for_task.join("manifest.json");
                let manifest_tmp = manifest_path.with_extension("json.tmp");
                std::fs::write(&manifest_tmp, manifest_bytes).map_err(|e| {
                    ToolError::Execution(format!("failed to write artifact manifest: {e}"))
                })?;
                std::fs::rename(&manifest_tmp, &manifest_path).map_err(|e| {
                    ToolError::Execution(format!("failed to commit artifact manifest: {e}"))
                })?;
                Ok((entry_rel, source_path, version))
            })
            .await
            .map_err(|e| ToolError::Execution(format!("artifact staging task failed: {e}")))??;

        let url = format!(
            "/artifacts/{}/{}/{}",
            encode_uri_segment(&session_segment),
            encode_uri_segment(&artifact_id),
            entry_rel
        );
        let artifact_meta = ArtifactMetadata {
            artifact_id,
            title: title.clone(),
            url: url.clone(),
            mode,
            height,
            source_path: Some(source_path),
            version: Some(version),
        };

        Ok(ToolOutput {
            output: format!("Published artifact \"{title}\" (v{version}) at {url}"),
            metadata: Some(json!({ ARTIFACT_METADATA_KEY: artifact_meta })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn test_ctx_with_session(cwd: std::path::PathBuf, session_dir: std::path::PathBuf) -> ToolCtx {
        ToolCtx::new(
            std::sync::Arc::new(crate::ToolCtxShared {
                cwd,
                session_dir: Some(session_dir),
                ..Default::default()
            }),
            "test".into(),
            CancellationToken::new(),
            mpsc::channel(1).0,
        )
    }

    #[tokio::test]
    async fn stages_directory_encodes_url_and_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let session_root = dir.path().join("sess_custom");
        let app_dir = workspace.join("sign-maker");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::create_dir_all(&session_root).unwrap();
        std::fs::write(app_dir.join("my index.html"), "<h1>Sign</h1>").unwrap();
        std::fs::write(app_dir.join("clean.png"), b"png").unwrap();

        let out = PresentArtifact
            .execute(
                test_ctx_with_session(workspace.clone(), session_root.clone()),
                json!({
                    "title": "Sign Maker",
                    "path": "sign-maker/my index.html",
                    "mode": "panel"
                }),
            )
            .await
            .expect("present_artifact");

        let meta: ArtifactMetadata = serde_json::from_value(
            out.metadata
                .unwrap()
                .get(ARTIFACT_METADATA_KEY)
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(meta.title, "Sign Maker");
        assert_eq!(meta.mode, ArtifactModeWire::Panel);
        assert!(meta.url.starts_with("/artifacts/sess_custom/art_"));
        assert!(meta.url.ends_with("/my%20index.html"));
        let staged = session_root.join("artifacts").join(&meta.artifact_id);
        let manifest: ArtifactManifest =
            serde_json::from_slice(&std::fs::read(staged.join("manifest.json")).unwrap()).unwrap();
        let blobs = session_root.join("artifacts").join("blobs");
        let html_entry = manifest.files.get("my index.html").unwrap();
        assert_eq!(
            std::fs::read_to_string(blobs.join(&html_entry.hash)).unwrap(),
            "<h1>Sign</h1>"
        );
        let png_entry = manifest.files.get("clean.png").unwrap();
        assert_eq!(std::fs::read(blobs.join(&png_entry.hash)).unwrap(), b"png");

        assert!(PresentArtifact
            .execute(
                test_ctx_with_session(workspace.clone(), session_root),
                json!({
                    "title": "Escape",
                    "path": "../outside.html"
                }),
            )
            .await
            .is_err());

        // Requires active session_dir.
        assert!(PresentArtifact
            .execute(
                ToolCtx::test_new(workspace),
                json!({
                    "title": "NoSession",
                    "path": "sign-maker/my index.html"
                }),
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn tracks_monotonic_versions_per_entry_path() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let session_root = dir.path().join("sess_ver");
        let app_dir = workspace.join("demo-app");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::create_dir_all(&session_root).unwrap();
        std::fs::write(app_dir.join("index.html"), "<h1>v1</h1>").unwrap();
        std::fs::write(app_dir.join("shared.png"), b"same-bytes").unwrap();

        let out1 = PresentArtifact
            .execute(
                test_ctx_with_session(workspace.clone(), session_root.clone()),
                json!({ "title": "Demo", "path": "demo-app" }),
            )
            .await
            .unwrap();
        let meta1: ArtifactMetadata = serde_json::from_value(
            out1.metadata
                .unwrap()
                .get(ARTIFACT_METADATA_KEY)
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(meta1.version, Some(1));
        assert_eq!(meta1.source_path.as_deref(), Some("demo-app/index.html"));

        std::fs::write(app_dir.join("index.html"), "<h1>v2</h1>").unwrap();
        let out2 = PresentArtifact
            .execute(
                test_ctx_with_session(workspace, session_root.clone()),
                json!({ "title": "Demo Renamed", "path": "demo-app/index.html" }),
            )
            .await
            .unwrap();
        let meta2: ArtifactMetadata = serde_json::from_value(
            out2.metadata
                .unwrap()
                .get(ARTIFACT_METADATA_KEY)
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(meta2.version, Some(2));
        assert_eq!(meta2.source_path.as_deref(), Some("demo-app/index.html"));
        assert_ne!(meta1.artifact_id, meta2.artifact_id);

        let m1: ArtifactManifest = serde_json::from_slice(
            &std::fs::read(
                session_root
                    .join("artifacts")
                    .join(&meta1.artifact_id)
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let m2: ArtifactManifest = serde_json::from_slice(
            &std::fs::read(
                session_root
                    .join("artifacts")
                    .join(&meta2.artifact_id)
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            m1.files.get("shared.png").unwrap().hash,
            m2.files.get("shared.png").unwrap().hash
        );
        assert_ne!(
            m1.files.get("index.html").unwrap().hash,
            m2.files.get("index.html").unwrap().hash
        );
    }
}
