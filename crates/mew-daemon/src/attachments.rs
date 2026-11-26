use crate::{resolve_mentions, Session};
use mew_message::Part;
use mew_protocol::Attachment;

pub(crate) fn attachment_parts(attachments: Vec<Attachment>) -> Vec<Part> {
    let store = mew_session::BlobStore::default_store();
    attachments
        .into_iter()
        .map(|attachment| {
            let mut mime = attachment.mime.unwrap_or_default();
            let (url, filename) = match mew_session::decode_data_url(&attachment.path) {
                Some((detected_mime, bytes)) => {
                    if mime.is_empty() {
                        mime = detected_mime;
                    }
                    match store.put(&bytes) {
                        Ok(id) => (mew_session::blob_url(&id), None),
                        Err(_) => (attachment.path, None),
                    }
                }
                None if mew_session::parse_blob_id(&attachment.path).is_some() => {
                    (attachment.path, None)
                }
                None => {
                    let filename = attachment.path.rsplit('/').next().map(str::to_owned);
                    (attachment.path, filename)
                }
            };
            Part::File(mew_message::FilePart {
                base: mew_message::PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                mime,
                filename,
                url,
            })
        })
        .collect()
}

fn extension_for_mime(mime: &str) -> &'static str {
    let base = mime.split(';').next().unwrap_or("").trim();
    match base {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/html" => "html",
        "application/json" => "json",
        "font/ttf" | "application/x-font-ttf" => "ttf",
        "font/woff2" => "woff2",
        "font/woff" => "woff",
        _ => "bin",
    }
}

/// Convert protocol attachments into message parts and materialize `mew-blob://`
/// payloads on disk so tools (`bash`, `read`, generated HTML) can access them.
pub(crate) async fn prepare_prompt_attachments(
    session: &Session,
    attachments: Vec<Attachment>,
) -> (Vec<Part>, String) {
    if attachments.is_empty() {
        return (Vec::new(), String::new());
    }
    let parts = tokio::task::spawn_blocking(move || attachment_parts(attachments))
        .await
        .unwrap_or_default();
    if parts.is_empty() {
        return (parts, String::new());
    }

    let is_scratch = session.scratch;
    let uploads_dir = if is_scratch {
        mew_session::scratch_workspace_dir(&session.session_dir, &session.id).join("uploads")
    } else {
        session.session_dir.join(&session.id).join("uploads")
    };

    let session_dir = session.session_dir.clone();
    let mut lines = Vec::new();
    let mut created_dir = false;

    for part in &parts {
        let Part::File(file) = part else {
            continue;
        };
        let Some(blob_id) = mew_session::parse_blob_id(&file.url) else {
            continue;
        };
        let ext = extension_for_mime(&file.mime);
        let short_id = &blob_id[..blob_id.len().min(12)];
        let filename = format!("{short_id}.{ext}");
        let disk_path = uploads_dir.join(&filename);

        if !tokio::fs::try_exists(&disk_path).await.unwrap_or(false) {
            if !created_dir {
                if tokio::fs::create_dir_all(&uploads_dir).await.is_err() {
                    break;
                }
                created_dir = true;
            }
            let blob_id_owned = blob_id.to_string();
            let session_blobs_dir = session_dir.join("blobs");
            let bytes = tokio::task::spawn_blocking(move || {
                mew_session::BlobStore::default_store()
                    .get(&blob_id_owned)
                    .or_else(|_| mew_session::BlobStore::new(session_blobs_dir).get(&blob_id_owned))
            })
            .await;
            let Ok(Ok(bytes)) = bytes else {
                continue;
            };
            if tokio::fs::write(&disk_path, &bytes).await.is_err() {
                continue;
            }
        }

        let display_path = if is_scratch {
            format!("uploads/{filename}")
        } else {
            disk_path.to_string_lossy().into_owned()
        };
        let mime_label = if file.mime.is_empty() {
            "application/octet-stream"
        } else {
            file.mime.as_str()
        };
        lines.push(format!("- {display_path} ({mime_label})"));
    }

    let note = if lines.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n[Attached files available on disk:\n{}]",
            lines.join("\n")
        )
    };
    (parts, note)
}

/// Resolve inline mentions and materialize attachments for a prompt turn.
/// Returns `(enriched_for_model, display_for_ui, attachment_parts, mention_parts)`.
pub(crate) async fn prepare_prompt_input(
    session: &std::sync::Arc<Session>,
    text: &str,
    attachments: Vec<Attachment>,
) -> (String, String, Vec<Part>, Vec<Part>) {
    let (mut enriched, display, mention_parts) = resolve_mentions(session, text).await;
    let (attach_parts, attachment_notes) = prepare_prompt_attachments(session, attachments).await;
    if !attachment_notes.is_empty() {
        enriched.push_str(&attachment_notes);
    }
    (enriched, display, attach_parts, mention_parts)
}

fn is_safe_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg != "."
        && seg != ".."
        && !seg.contains('/')
        && !seg.contains('\\')
        && !seg.contains('\0')
}

fn artifact_mime_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Read a staged artifact file for a remote/mobile client, enforcing safe path
/// segments and a 10 MiB limit.
#[derive(serde::Deserialize)]
struct ArtifactManifestFile {
    hash: String,
}

#[derive(serde::Deserialize)]
struct ArtifactManifest {
    files: std::collections::HashMap<String, ArtifactManifestFile>,
}

fn is_valid_blob_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) async fn read_session_artifact_file(
    session_dir: &std::path::Path,
    request_id: String,
    session_id: &str,
    artifact_id: &str,
    rel_path: &str,
) -> mew_protocol::ServerMessage {
    use base64::Engine;

    let fail = |msg: String| mew_protocol::ServerMessage::ArtifactFile {
        request_id: request_id.clone(),
        mime: String::new(),
        data_base64: String::new(),
        error: Some(msg),
    };

    if !is_safe_segment(session_id) || !is_safe_segment(artifact_id) {
        return fail("invalid artifact identifier".into());
    }
    let artifacts_root = session_dir.join(session_id).join("artifacts");
    let artifact_root = artifacts_root.join(artifact_id);
    let Ok(canonical_root) = tokio::fs::canonicalize(&artifact_root).await else {
        return fail("artifact not found".into());
    };

    let rel = if rel_path.trim().is_empty() {
        "index.html"
    } else {
        rel_path
    };
    let mut rel_segments = Vec::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if !is_safe_segment(seg) {
            return fail("invalid artifact path".into());
        }
        rel_segments.push(seg.to_string());
    }

    let manifest_path = canonical_root.join("manifest.json");
    let (canonical_file, mime_key) = if let Ok(manifest_bytes) =
        tokio::fs::read(&manifest_path).await
    {
        let Ok(manifest) = serde_json::from_slice::<ArtifactManifest>(&manifest_bytes) else {
            return fail("invalid artifact manifest".into());
        };
        let rel_key = rel_segments.join("/");
        let resolved_key = if rel_key.is_empty() {
            "index.html".to_string()
        } else if manifest.files.contains_key(&rel_key) {
            rel_key
        } else {
            let dir_index = format!("{rel_key}/index.html");
            if manifest.files.contains_key(&dir_index) {
                dir_index
            } else {
                rel_key
            }
        };
        let Some(entry) = manifest.files.get(&resolved_key) else {
            return fail("artifact file not found".into());
        };
        if !is_valid_blob_hash(&entry.hash) {
            return fail("invalid artifact blob".into());
        }
        let blobs_root = artifacts_root.join("blobs");
        let Ok(canonical_blobs) = tokio::fs::canonicalize(&blobs_root).await else {
            return fail("artifact file not found".into());
        };
        let Ok(canonical_blob) = tokio::fs::canonicalize(canonical_blobs.join(&entry.hash)).await
        else {
            return fail("artifact file not found".into());
        };
        if !canonical_blob.starts_with(&canonical_blobs) {
            return fail("forbidden artifact path".into());
        }
        (canonical_blob, resolved_key)
    } else {
        let mut candidate = canonical_root.clone();
        for seg in &rel_segments {
            candidate.push(seg);
        }
        if tokio::fs::metadata(&candidate)
            .await
            .is_ok_and(|m| m.is_dir())
        {
            candidate.push("index.html");
        }
        let Ok(canonical_file) = tokio::fs::canonicalize(&candidate).await else {
            return fail("artifact file not found".into());
        };
        if !canonical_file.starts_with(&canonical_root) {
            return fail("forbidden artifact path".into());
        }
        let key = canonical_file.to_string_lossy().into_owned();
        (canonical_file, key)
    };
    let Ok(meta) = tokio::fs::metadata(&canonical_file).await else {
        return fail("artifact file not found".into());
    };
    if !meta.is_file() {
        return fail("artifact path is not a file".into());
    }
    if meta.len() > mew_session::MAX_BLOB_BYTES as u64 {
        return fail("artifact file exceeds size limit".into());
    }
    let Ok(bytes) = tokio::fs::read(&canonical_file).await else {
        return fail("failed to read artifact file".into());
    };
    let mime = artifact_mime_type(&mime_key).to_string();
    let data_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    mew_protocol::ServerMessage::ArtifactFile {
        request_id,
        mime,
        data_base64,
        error: None,
    }
}
