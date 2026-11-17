use ratatui::style::Style;

use crate::inline::StyledRun;

/// TextMate scopes and the shared theme tokens that color them.
pub const SYNTAX_SCOPES: &[(&str, &str)] = &[
    ("comment", "syntax.comment"),
    ("string", "syntax.string"),
    ("keyword", "syntax.keyword"),
    ("entity.name.function, support.function", "syntax.function"),
    ("constant.numeric", "syntax.number"),
    ("entity.name.type, storage.type", "syntax.type"),
    ("variable", "syntax.variable"),
    ("keyword.operator", "syntax.operator"),
    ("constant", "syntax.constant"),
];

/// Syntax highlighter for code fences.
pub trait Highlighter {
    /// Return styled runs for one line of code.
    fn highlight_line(&mut self, lang: Option<&str>, line: &str) -> Vec<StyledRun>;

    /// Hint that subsequent calls are for the same code block.
    fn begin_block(&mut self, _lang: Option<&str>) {}
    fn end_block(&mut self) {}
}

/// No-op highlighter that returns plain text.
pub struct NoHighlight;

impl Highlighter for NoHighlight {
    fn highlight_line(&mut self, _lang: Option<&str>, line: &str) -> Vec<StyledRun> {
        vec![(line.to_string(), Style::default())]
    }
}

#[cfg(feature = "syntect")]
pub mod syntect;

#[cfg(feature = "syntect")]
pub use self::syntect::SyntectHighlighter;
