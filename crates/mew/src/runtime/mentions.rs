//! @mention resolution — moved from main.rs.
//!
//! [`process_mentions`] resolves @mentions in user input. Text files are
//! inlined into the model-facing text; image files become `Part::File`
//! attachments.

use mew_message::Part;

/// Determine the MIME type for an image file based on its extension.
pub fn image_mime(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// Resolve @mentions in `text`. Text files are inlined into the model-facing
/// text; image files become `Part::File` attachments. Returns
/// `(enriched, display, attachments)` where `enriched` carries the full file
/// content for the model and `display` carries only a `<path added to
/// context>` notification for the user's visible message — the file contents
/// should not flood the chat.
pub async fn process_mentions(
    text: &str,
    cwd: &std::path::Path,
    context_files: &mut Vec<String>,
) -> (String, String, Vec<Part>) {
    let mentions = mew_tui::app::parse_file_mentions(text);
    let mut enriched = text.to_string();
    let mut display = text.to_string();
    let mut attachments: Vec<Part> = Vec::new();

    for path_str in &mentions {
        let path = cwd.join(path_str);
        if let Some(mime) = image_mime(path_str) {
            let mention = format!("@{}", path_str);
            enriched = enriched.replace(&mention, "");
            display = display.replace(&mention, "");
            if path.exists() {
                let abs = path.canonicalize().unwrap_or(path.clone());
                let filename = std::path::Path::new(path_str)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path_str)
                    .to_string();
                attachments.push(Part::File(mew_message::FilePart {
                    base: mew_message::PartBase {
                        id: ulid::Ulid::new(),
                        message_id: ulid::Ulid::new(),
                        session_id: ulid::Ulid::new(),
                    },
                    mime: mime.to_string(),
                    url: format!("file://{}", abs.display()),
                    filename: Some(filename),
                }));
                if !context_files.contains(path_str) {
                    context_files.push(path_str.clone());
                }
                display.push_str(&format!("\n<{} added to context>", path_str));
            } else {
                let err = format!("\n\n[error reading {}: file not found]", path_str);
                enriched.push_str(&err);
                display.push_str(&err);
            }
        } else {
            match tokio::fs::read_to_string(&path).await {
                Ok(content) => {
                    enriched.push_str(&format!("\n\n--- {} ---\n{}", path_str, content));
                    if !context_files.contains(path_str) {
                        context_files.push(path_str.clone());
                    }
                    display.push_str(&format!("\n<{} added to context>", path_str));
                }
                Err(e) => {
                    let err = format!("\n\n[error reading {}: {}]", path_str, e);
                    enriched.push_str(&err);
                    display.push_str(&err);
                }
            }
        }
    }

    (enriched, display, attachments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_mime_png() {
        assert_eq!(image_mime("foo.png"), Some("image/png"));
    }

    #[test]
    fn image_mime_jpg_and_jpeg() {
        assert_eq!(image_mime("foo.jpg"), Some("image/jpeg"));
        assert_eq!(image_mime("foo.jpeg"), Some("image/jpeg"));
    }

    #[test]
    fn image_mime_gif() {
        assert_eq!(image_mime("foo.gif"), Some("image/gif"));
    }

    #[test]
    fn image_mime_webp() {
        assert_eq!(image_mime("foo.webp"), Some("image/webp"));
    }

    #[test]
    fn image_mime_case_insensitive() {
        assert_eq!(image_mime("foo.PNG"), Some("image/png"));
        assert_eq!(image_mime("foo.Jpg"), Some("image/jpeg"));
    }

    #[test]
    fn image_mime_unknown_extension() {
        assert_eq!(image_mime("foo.txt"), None);
        assert_eq!(image_mime("foo.pdf"), None);
        assert_eq!(image_mime("foo"), None);
        assert_eq!(image_mime(""), None);
    }
}
