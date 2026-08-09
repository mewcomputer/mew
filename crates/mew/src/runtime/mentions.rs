//! @mention resolution — moved from main.rs.
//!
//! [`process_mentions`] resolves @mentions in user input. Text files are
//! inlined into the model-facing text; image files become `Part::File`
//! attachments. Namespace references (`@skill:name`, `@model:provider/model`,
//! `@subagent:name`) are also resolved here: skills inline their body, models
//! inline a reference marker, and subagents inline their description.

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
    skills: &[mew_skills::Skill],
    subagents: &[mew_subagents::SubagentDef],
) -> (String, String, Vec<Part>) {
    // --- Phase 1: Resolve namespace references ---
    // Namespace refs (@skill:, @model:, @subagent:) are resolved and stripped
    // FIRST so that the file-mention pass below never sees them.
    //
    // To prevent inlined content (skill bodies, descriptions) from being
    // corrupted by subsequent token replacements, we strip ALL tokens from
    // the text first, then append the inlined content afterward.
    let refs = mew_tui::app::parse_namespace_refs(text);
    let mut enriched = text.to_string();
    let mut display = text.to_string();
    let mut enriched_tail = String::new();
    let mut display_tail = String::new();

    for nr in &refs {
        match nr.kind.as_str() {
            "skill" => {
                let skill = skills.iter().find(|s| s.name == nr.value);
                match skill {
                    Some(skill) => {
                        enriched = enriched.replace(&nr.raw, "");
                        display = display.replace(&nr.raw, "");
                        enriched_tail.push_str(&format!(
                            "\n\n--- skill: {} ---\n{}",
                            skill.name, skill.body
                        ));
                        display_tail
                            .push_str(&format!("\n<skill '{}' added to context>", skill.name));
                    }
                    None => {
                        let err = format!("[error: skill '{}' not found]", nr.value);
                        enriched = enriched.replace(&nr.raw, &err);
                        display = display.replace(&nr.raw, &err);
                    }
                }
            }
            "model" => {
                enriched = enriched.replace(&nr.raw, "");
                display = display.replace(&nr.raw, "");
                enriched_tail.push_str(&format!("\n\n[model reference: {}]", nr.value));
                display_tail.push_str(&format!("\n<model '{}' referenced>", nr.value));
            }
            "subagent" => {
                let subagent = subagents.iter().find(|s| s.name == nr.value);
                match subagent {
                    Some(sa) => {
                        enriched = enriched.replace(&nr.raw, "");
                        display = display.replace(&nr.raw, "");
                        enriched_tail.push_str(&format!(
                            "\n\n--- subagent: {} ---\n{}",
                            sa.name, sa.description
                        ));
                        display_tail
                            .push_str(&format!("\n<subagent '{}' added to context>", sa.name));
                    }
                    None => {
                        let err = format!("[error: subagent '{}' not found]", nr.value);
                        enriched = enriched.replace(&nr.raw, &err);
                        display = display.replace(&nr.raw, &err);
                    }
                }
            }
            _ => {}
        }
    }

    enriched.push_str(&enriched_tail);
    display.push_str(&display_tail);

    // --- Phase 2: Resolve file @mentions (existing logic) ---
    let mentions = mew_tui::app::parse_file_mentions(&enriched);
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
        assert!(display.contains("<skill 'clarify' added to context>"));
        // The raw token should be stripped from enriched text.
        assert!(!enriched.contains("@skill:clarify"));
    }

    #[tokio::test]
    async fn test_process_mentions_resolves_model() {
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) =
            process_mentions("@model:openai/gpt-4o review this", cwd, &mut ctx, &[], &[]).await;
        assert!(enriched.contains("[model reference: openai/gpt-4o]"));
        assert!(display.contains("<model 'openai/gpt-4o' referenced>"));
        assert!(!enriched.contains("@model:openai/gpt-4o"));
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
        assert!(display.contains("<subagent 'researcher' added to context>"));
        assert!(!enriched.contains("@subagent:researcher"));
    }

    #[tokio::test]
    async fn test_process_mentions_subagent_not_found() {
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) =
            process_mentions("@subagent:nonexistent", cwd, &mut ctx, &[], &[]).await;
        assert!(enriched.contains("[error: subagent 'nonexistent' not found]"));
        assert!(display.contains("[error: subagent 'nonexistent' not found]"));
    }

    #[tokio::test]
    async fn test_process_mentions_skill_not_found() {
        let skills: Vec<mew_skills::Skill> = vec![];
        let mut ctx = Vec::new();
        let cwd = std::path::Path::new(".");
        let (enriched, display, _atts) =
            process_mentions("@skill:nonexistent fix this", cwd, &mut ctx, &skills, &[]).await;
        assert!(enriched.contains("[error: skill 'nonexistent' not found]"));
        assert!(display.contains("[error: skill 'nonexistent' not found]"));
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
        // Skill body inlined.
        assert!(enriched.contains("Be clear."));
        assert!(display.contains("<skill 'clarify' added to context>"));
        // File content inlined.
        assert!(enriched.contains("fn main() {}"));
        assert!(display.contains("<main.rs added to context>"));
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
