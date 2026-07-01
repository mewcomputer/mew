use crate::parser::parse_section;
use crate::tokenizer::{tokenize, LineToken, Token};
use crate::types::{Edit, FileOp};

/// A parsed hashline patch.
#[derive(Debug, Clone)]
pub struct Patch {
    pub sections: Vec<PatchSection>,
}

/// One file section within a patch.
#[derive(Debug, Clone)]
pub struct PatchSection {
    pub path: String,
    pub hash: Option<String>,
    pub edits: Vec<Edit>,
    pub file_op: Option<FileOp>,
    pub warnings: Vec<String>,
}

impl PatchSection {
    /// Anchor lines touched by this section, sorted and deduplicated.
    pub fn collect_anchor_lines(&self) -> Vec<usize> {
        let mut lines: Vec<usize> = self
            .edits
            .iter()
            .filter_map(|edit| match edit {
                crate::types::Edit::Delete { anchor } => Some(anchor.line),
                crate::types::Edit::Block { anchor, .. } => Some(anchor.line),
                crate::types::Edit::Insert { cursor, .. } => match cursor {
                    crate::types::Cursor::BeforeAnchor { anchor }
                    | crate::types::Cursor::AfterAnchor { anchor } => Some(anchor.line),
                    _ => None,
                },
            })
            .collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }
}

impl Patch {
    /// Parse a hashline patch string into sections.
    pub fn parse(input: &str) -> crate::Result<Self> {
        let input = input.strip_prefix('\u{FEFF}').unwrap_or(input);
        let tokens = tokenize(input)?;

        let raw_sections = split_sections(&tokens)?;
        let mut sections: Vec<PatchSection> = Vec::new();
        for (path, hash, body_tokens) in raw_sections {
            let (edits, file_op, warnings) = parse_section(&body_tokens)?;
            sections.push(PatchSection {
                path,
                hash,
                edits,
                file_op,
                warnings,
            });
        }

        sections = merge_same_path_sections(sections)?;

        if sections.is_empty() {
            return Err(crate::HashlineError::parse(
                0,
                "input did not produce any hashline sections",
            ));
        }

        Ok(Self { sections })
    }
}

type RawSection = (String, Option<String>, Vec<LineToken>);

fn split_sections(tokens: &[LineToken]) -> crate::Result<Vec<RawSection>> {
    let mut sections: Vec<(String, Option<String>, Vec<LineToken>)> = Vec::new();
    let mut current: Option<(String, Option<String>, Vec<LineToken>)> = None;

    for token in tokens {
        if let Token::Header { path, hash } = &token.token {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some((path.clone(), hash.clone(), Vec::new()));
            continue;
        }

        if let Some(section) = current.as_mut() {
            section.2.push(token.clone());
        } else {
            // Content before the first header is an error.
            return Err(crate::HashlineError::parse(
                token.line,
                "content before the first [path#hash] header",
            ));
        }
    }

    if let Some(section) = current.take() {
        sections.push(section);
    }

    Ok(sections)
}

fn merge_same_path_sections(sections: Vec<PatchSection>) -> crate::Result<Vec<PatchSection>> {
    let mut out: Vec<PatchSection> = Vec::with_capacity(sections.len());
    for section in sections {
        if let Some(existing) = out.iter_mut().find(|s| s.path == section.path) {
            if existing.hash.is_some() && section.hash.is_some() && existing.hash != section.hash {
                return Err(crate::HashlineError::execution(format!(
                    "conflicting hashes for {}: #{:?} and #{:?}",
                    section.path, existing.hash, section.hash
                )));
            }
            if section.file_op.is_some() && existing.file_op.is_some() {
                return Err(crate::HashlineError::execution(format!(
                    "multiple file-level ops for {}",
                    section.path
                )));
            }
            if existing.hash.is_none() {
                existing.hash = section.hash;
            }
            if section.file_op.is_some() {
                existing.file_op = section.file_op;
            }
            existing.edits.extend(section.edits);
            existing.warnings.extend(section.warnings);
        } else {
            out.push(section);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_section() {
        let patch = Patch::parse("[src/lib.rs#A1B2]\nSWAP 2.=2:\n+new\n").unwrap();
        assert_eq!(patch.sections.len(), 1);
        assert_eq!(patch.sections[0].path, "src/lib.rs");
        assert_eq!(patch.sections[0].hash, Some("A1B2".to_string()));
        assert_eq!(patch.sections[0].edits.len(), 2);
    }

    #[test]
    fn parse_multi_section() {
        let patch = Patch::parse("[a.rs#1111]\nDEL 1\n[b.rs#2222]\nINS.HEAD:\n+header\n").unwrap();
        assert_eq!(patch.sections.len(), 2);
        assert_eq!(patch.sections[0].path, "a.rs");
        assert_eq!(patch.sections[1].path, "b.rs");
    }

    #[test]
    fn rejects_content_before_header() {
        let err = Patch::parse("random\n[a.rs#1111]\nDEL 1\n").unwrap_err();
        assert!(err.to_string().contains("before the first"));
    }
}
