//! Parsing and resolution shared by the TUI and the daemon.

use mew_message::Part;

/// Extract `@path` file mentions from input text.
/// Returns a list of path strings (without the `@` prefix).
///
/// Tokens starting with a known namespace prefix (`skill:`, `model:`,
/// `subagent:`) are skipped — they are handled by their respective
/// namespace parsers.
pub fn parse_file_mentions(text: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    for word in text.split_whitespace() {
        if let Some(path) = word.strip_prefix('@') {
            let path = path.trim_end_matches(|c: char| c.is_ascii_punctuation());
            if path.is_empty() || is_namespace_prefix(path) {
                continue;
            }
            mentions.push(path.to_string());
        }
    }
    mentions
}

/// Known namespace prefixes for inline references.
pub const NAMESPACE_PREFIXES: &[(&str, &str)] = &[
    ("skill", "skill:"),
    ("model", "model:"),
    ("subagent", "subagent:"),
];

/// Returns true if the path (after the `@`) begins with a known namespace prefix.
pub fn is_namespace_prefix(path: &str) -> bool {
    NAMESPACE_PREFIXES.iter().any(|(_, p)| path.starts_with(p))
}

/// A parsed namespace reference from user input (e.g. `@skill:clarify`).
#[derive(Debug, Clone, PartialEq)]
pub struct NamespaceRef {
    pub raw: String,
    pub kind: String,
    pub value: String,
}

/// Extract `@namespace:value` references from input text.
pub fn parse_namespace_refs(text: &str) -> Vec<NamespaceRef> {
    let mut refs = Vec::new();
    for word in text.split_whitespace() {
        if let Some(rest) = word.strip_prefix('@') {
            let rest = rest.trim_end_matches(|c: char| c.is_ascii_punctuation());
            for &(kind, prefix) in NAMESPACE_PREFIXES {
                if let Some(value) = rest.strip_prefix(prefix) {
                    if !value.is_empty() {
                        refs.push(NamespaceRef {
                            raw: format!("@{prefix}{value}"),
                            kind: kind.to_string(),
                            value: value.to_string(),
                        });
                    }
                    break;
                }
            }
        }
    }
    refs
}

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
    skills: &[mew_skills::Skill],
    subagents: &[mew_subagents::SubagentDef],
) -> (String, String, Vec<Part>) {
    // --- Phase 1: Resolve namespace references ---
    // Namespace refs (@skill:, @model:, @subagent:) stay inline in both
    // `display` and `enriched`, while their resolved context blocks are
    // appended to `enriched_tail` for the model. `parse_file_mentions` skips
    // known namespace prefixes so inline tokens never trigger file resolution.
    let refs = parse_namespace_refs(text);
    let mut enriched = text.to_string();
    let mut display = text.to_string();
    let mut enriched_tail = String::new();

    for nr in &refs {
        match nr.kind.as_str() {
            "skill" => {
                if let Some(skill) = skills.iter().find(|s| s.name == nr.value) {
                    enriched_tail.push_str(&format!(
                        "\n\n--- skill: {} ---\n{}",
                        skill.name, skill.body
                    ));
                }
            }
            "model" => {
                enriched_tail.push_str(&format!("\n\n[model reference: {}]", nr.value));
            }
            "subagent" => {
                if let Some(sa) = subagents.iter().find(|s| s.name == nr.value) {
                    enriched_tail.push_str(&format!(
                        "\n\n--- subagent: {} ---\n{}",
                        sa.name, sa.description
                    ));
                }
            }
            _ => {}
        }
    }

    enriched.push_str(&enriched_tail);

    // --- Phase 2: Resolve file @mentions (existing logic) ---
    let mentions = parse_file_mentions(&enriched);
    let mut attachments: Vec<Part> = Vec::new();

    for path_str in &mentions {
        let path = cwd.join(path_str);
        if let Some(mime) = image_mime(path_str) {
            let mention = format!("@{}", path_str);
            enriched = enriched.replace(&mention, "");
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

    // --- Skill reference resolution tests ---

    fn test_skill(name: &str, body: &str) -> mew_skills::Skill {
        mew_skills::Skill {
            name: name.to_string(),
            description: format!("Description for {name}"),
            body: body.to_string(),
            path: std::path::PathBuf::from("(test)"),
            template: false,
        }
    }

    fn test_subagent(name: &str, description: &str) -> mew_subagents::SubagentDef {
        mew_subagents::SubagentDef {
            name: name.to_string(),
            description: description.to_string(),
            model: None,
            tools: None,
            max_turns: None,
            max_duration_secs: None,
            body: String::new(),
            path: std::path::PathBuf::from("(test)"),
            template: false,
            can_spawn: false,
            output_schema: None,
        }
    }

    #[tokio::test]
    async fn test_process_mentions_resolves_skill() {
        let skills = vec![test_skill("clarify", "Be clear and concise.")];
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) =
            process_mentions("@skill:clarify fix this", cwd, &mut ctx, &skills, &[]).await;
        assert!(enriched.contains("Be clear and concise."));
        assert!(enriched.starts_with("@skill:clarify fix this"));
        assert_eq!(display, "@skill:clarify fix this");
    }

    #[tokio::test]
    async fn test_process_mentions_resolves_model() {
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) =
            process_mentions("@model:openai/gpt-4o review this", cwd, &mut ctx, &[], &[]).await;
        assert!(enriched.contains("[model reference: openai/gpt-4o]"));
        assert!(enriched.starts_with("@model:openai/gpt-4o review this"));
        assert_eq!(display, "@model:openai/gpt-4o review this");
    }

    #[tokio::test]
    async fn test_process_mentions_resolves_subagent() {
        let subagents = vec![test_subagent(
            "researcher",
            "Investigate research questions.",
        )];
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) = process_mentions(
            "@subagent:researcher look into this",
            cwd,
            &mut ctx,
            &[],
            &subagents,
        )
        .await;
        assert!(enriched.contains("--- subagent: researcher ---"));
        assert!(enriched.contains("Investigate research questions."));
        assert!(enriched.starts_with("@subagent:researcher look into this"));
        assert_eq!(display, "@subagent:researcher look into this");
    }

    #[tokio::test]
    async fn test_process_mentions_subagent_not_found() {
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) =
            process_mentions("@subagent:nonexistent", cwd, &mut ctx, &[], &[]).await;
        // Unknown subagent refs stay as literal text without errors.
        assert_eq!(enriched, "@subagent:nonexistent");
        assert_eq!(display, "@subagent:nonexistent");
    }

    #[tokio::test]
    async fn test_process_mentions_skill_not_found() {
        let skills: Vec<mew_skills::Skill> = vec![];
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) =
            process_mentions("@skill:nonexistent fix this", cwd, &mut ctx, &skills, &[]).await;
        // Unknown skill refs stay as literal text without errors.
        assert_eq!(enriched, "@skill:nonexistent fix this");
        assert_eq!(display, "@skill:nonexistent fix this");
    }

    #[tokio::test]
    async fn test_process_mentions_mixed_skill_and_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        std::fs::write(cwd.join("main.rs"), "fn main() {}").unwrap();

        let skills = vec![test_skill("clarify", "Be clear.")];
        let mut ctx = Vec::new();
        let (enriched, display, _atts) = process_mentions(
            "@skill:clarify review @main.rs",
            cwd,
            &mut ctx,
            &skills,
            &[],
        )
        .await;
        // Skill body inlined and tokens preserved inline without context tails.
        assert!(enriched.contains("Be clear."));
        assert_eq!(display, "@skill:clarify review @main.rs");
        // File content inlined for model.
        assert!(enriched.contains("fn main() {}"));
    }

    #[tokio::test]
    async fn test_process_mentions_skill_body_inlined() {
        let skills = vec![test_skill(
            "audit",
            "# Audit Instructions\nCheck accessibility and performance.",
        )];
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, _display, _atts) =
            process_mentions("@skill:audit", cwd, &mut ctx, &skills, &[]).await;
        assert!(enriched.contains("--- skill: audit ---"));
        assert!(enriched.contains("# Audit Instructions"));
        assert!(enriched.contains("Check accessibility and performance."));
    }
}
