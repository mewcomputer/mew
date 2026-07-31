//! Tree-sitter-backed block resolution for `SWAP.BLK`, `DEL.BLK`, and
//! `INS.BLK.POST` operations.

use crate::types::{
    Anchor, BlockMode, BlockResolution, BlockResolver, BlockResolverArgs, BlockSpan, Cursor, Edit,
    InsertMode,
};
use tree_sitter::{Language, Node, Parser};

/// A block resolver that never resolves anything. Used when the host does not
/// wire tree-sitter support.
pub fn nop_block_resolver(_args: &BlockResolverArgs) -> Option<BlockSpan> {
    None
}

/// Build a block resolver backed by tree-sitter for Rust, TypeScript, Python,
/// Go, Markdown, and Gleam.
pub fn default_block_resolver() -> Box<BlockResolver> {
    let languages = build_language_table();
    Box::new(move |args| resolve_block(&languages, args.path, args.text, args.line))
}

/// Expand any deferred block edits in `edits` into concrete inserts and deletes.
///
/// `resolutions` is populated with the resolved span for every block op.
/// `INS.BLK.POST` degrades to a plain `INS.POST` with a warning when the block
/// cannot be resolved; `SWAP.BLK` and `DEL.BLK` error in that case.
pub fn resolve_block_edits(
    edits: &[Edit],
    text: &str,
    path: &str,
    resolver: Option<&BlockResolver>,
    resolutions: &mut Vec<BlockResolution>,
) -> crate::Result<Vec<Edit>> {
    let mut out = Vec::with_capacity(edits.len());
    let mut warnings: Vec<String> = Vec::new();

    for edit in edits {
        let Edit::Block {
            anchor,
            payloads,
            mode,
        } = edit
        else {
            out.push(edit.clone());
            continue;
        };

        let span = resolver.and_then(|r| {
            r(&BlockResolverArgs {
                path,
                text,
                line: anchor.line,
            })
        });

        match mode {
            BlockMode::InsertAfter => {
                let (line, block_start) = match span {
                    Some(s) => {
                        resolutions.push(BlockResolution {
                            anchor_line: anchor.line,
                            start: s.start,
                            end: s.end,
                            op: crate::types::BlockOp::InsertAfter,
                        });
                        (s.end, Some(s.start))
                    }
                    None => {
                        warnings.push(format!(
                            "INS.BLK.POST {} could not be resolved; falling back to INS.POST {}",
                            anchor.line, anchor.line
                        ));
                        (anchor.line, None)
                    }
                };
                for payload in payloads {
                    out.push(Edit::Insert {
                        cursor: Cursor::AfterAnchor {
                            anchor: Anchor { line },
                        },
                        text: payload.clone(),
                        mode: InsertMode::Normal,
                        block_start,
                    });
                }
            }
            BlockMode::Replace => {
                let span =
                    span.ok_or(crate::HashlineError::BlockUnresolved { line: anchor.line })?;
                if span.end <= span.start {
                    return Err(crate::HashlineError::BlockUnresolved { line: anchor.line });
                }
                resolutions.push(BlockResolution {
                    anchor_line: anchor.line,
                    start: span.start,
                    end: span.end,
                    op: crate::types::BlockOp::Replace,
                });
                for payload in payloads {
                    out.push(Edit::Insert {
                        cursor: Cursor::BeforeAnchor {
                            anchor: Anchor { line: span.start },
                        },
                        text: payload.clone(),
                        mode: InsertMode::Replacement,
                        block_start: None,
                    });
                }
                for line in span.start..=span.end {
                    out.push(Edit::Delete {
                        anchor: Anchor { line },
                    });
                }
            }
            BlockMode::Delete => {
                let span =
                    span.ok_or(crate::HashlineError::BlockUnresolved { line: anchor.line })?;
                if span.end <= span.start {
                    return Err(crate::HashlineError::BlockUnresolved { line: anchor.line });
                }
                resolutions.push(BlockResolution {
                    anchor_line: anchor.line,
                    start: span.start,
                    end: span.end,
                    op: crate::types::BlockOp::Delete,
                });
                for line in span.start..=span.end {
                    out.push(Edit::Delete {
                        anchor: Anchor { line },
                    });
                }
            }
        }
    }

    // Warnings are not returned here; they should be folded into the patch
    // section warnings by the caller. For now we ignore them because the
    // apply engine does not use them, but we keep the API shape consistent.
    let _ = warnings;

    Ok(out)
}

fn build_language_table() -> Vec<(&'static str, Language)> {
    vec![
        ("rs", tree_sitter_rust::LANGUAGE.into()),
        ("ts", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        ("tsx", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        ("js", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        ("jsx", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        ("mjs", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        ("cjs", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        ("py", tree_sitter_python::LANGUAGE.into()),
        ("go", tree_sitter_go::LANGUAGE.into()),
        ("md", tree_sitter_md::LANGUAGE.into()),
        ("markdown", tree_sitter_md::LANGUAGE.into()),
        ("gleam", tree_sitter_gleam::LANGUAGE.into()),
    ]
}

fn resolve_block(
    languages: &[(&'static str, Language)],
    path: &str,
    text: &str,
    line: usize,
) -> Option<BlockSpan> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?;
    let (_, language) = languages.iter().find(|(e, _)| *e == ext)?;

    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();

    let target_row = line.saturating_sub(1);
    let mut best: Option<(usize, Node)> = None;
    find_best_block(root, target_row, 0, &mut best);

    let (_, node) = best?;
    let start = node.start_position().row + 1;
    let end_row = node.end_position().row;
    let end_col = node.end_position().column;
    // Tree-sitter end positions are exclusive. A node ending at column 0 of
    // row N ends at the previous line; otherwise it ends on row N.
    let end = if end_col == 0 { end_row } else { end_row + 1 };
    if end <= start {
        return None;
    }
    Some(BlockSpan { start, end })
}

fn find_best_block<'a>(
    node: Node<'a>,
    target_row: usize,
    depth: usize,
    best: &mut Option<(usize, Node<'a>)>,
) {
    // Never resolve to the root document node; blocks must be real syntactic
    // constructs inside it.
    if depth > 0 {
        let end_row = node.end_position().row;
        let end_col = node.end_position().column;
        let end = if end_col == 0 { end_row } else { end_row + 1 };
        if node.start_position().row == target_row
            && end > target_row + 1
            && best.is_none_or(|(d, _)| depth > d)
        {
            *best = Some((depth, node));
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_best_block(child, target_row, depth + 1, best);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rust_function_block() {
        let text = "fn a() {}\nfn b() {\n    println!(\"hi\");\n}\nfn c() {}\n";
        let resolver = default_block_resolver();
        let span = resolver(&BlockResolverArgs {
            path: "src/lib.rs",
            text,
            line: 2,
        });
        assert_eq!(span, Some(BlockSpan { start: 2, end: 4 }));
    }

    #[test]
    fn single_line_block_returns_none() {
        let text = "fn a() {}\nfn b() {}\n";
        let resolver = default_block_resolver();
        let span = resolver(&BlockResolverArgs {
            path: "src/lib.rs",
            text,
            line: 1,
        });
        assert_eq!(span, None);
    }

    #[test]
    fn unknown_extension_returns_none() {
        let resolver = default_block_resolver();
        let span = resolver(&BlockResolverArgs {
            path: "file.unknown",
            text: "a\nb\n",
            line: 1,
        });
        assert_eq!(span, None);
    }

    #[test]
    fn resolve_gleam_function_block() {
        let text = "pub fn a() {}\npub fn b() {\n  io.println(\"hi\")\n}\npub fn c() {}\n";
        let resolver = default_block_resolver();
        let span = resolver(&BlockResolverArgs {
            path: "src/main.gleam",
            text,
            line: 2,
        });
        assert_eq!(span, Some(BlockSpan { start: 2, end: 4 }));
    }

    #[test]
    fn ins_blk_post_degrades_without_resolver() {
        let edits = vec![Edit::Block {
            anchor: Anchor { line: 1 },
            payloads: vec!["x".to_string()],
            mode: BlockMode::InsertAfter,
        }];
        let mut resolutions = Vec::new();
        let resolved =
            resolve_block_edits(&edits, "a\nb\n", "x.unknown", None, &mut resolutions).unwrap();
        assert_eq!(resolutions.len(), 0);
        assert!(matches!(
            resolved[0],
            Edit::Insert {
                cursor: Cursor::AfterAnchor {
                    anchor: Anchor { line: 1 }
                },
                ..
            }
        ));
    }
}
