pub mod cache;
pub mod highlight;
pub mod inline;
pub mod pending;
pub mod render;
pub mod table;
pub mod theme;
pub mod widget;
pub mod wrap;

use ratatui::text::Line;
use std::ops::Range;

pub use theme::Theme;
pub use widget::{StreamView, StreamViewState};

/// Copyable bytes in the concatenated spans of a rendered row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMetadata {
    /// Excludes generated continuation prefixes and right padding.
    pub content: Range<usize>,
    /// Continues the preceding copy line without introducing a newline.
    /// Used for soft wraps and redundant generated blank separators.
    pub continues_previous: bool,
}

/// Styled rows and their copy bounds, in matching order.
#[derive(Debug, Clone, Default)]
pub struct RenderedLines {
    pub lines: Vec<Line<'static>>,
    pub metadata: Vec<LineMetadata>,
}

impl RenderedLines {
    pub(crate) fn from_lines(lines: Vec<Line<'static>>) -> Self {
        let metadata = lines
            .iter()
            .map(|line| LineMetadata {
                content: 0..line.spans.iter().map(|span| span.content.len()).sum(),
                continues_previous: false,
            })
            .collect();
        Self { lines, metadata }
    }

    pub(crate) fn push(&mut self, line: Line<'static>, metadata: LineMetadata) {
        self.lines.push(line);
        self.metadata.push(metadata);
    }

    pub(crate) fn push_blank(&mut self) {
        let continues_previous = self
            .metadata
            .last()
            .is_some_and(|row| row.content.is_empty());
        self.push(
            Line::from(""),
            LineMetadata {
                content: 0..0,
                continues_previous,
            },
        );
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        self.lines.append(&mut other.lines);
        self.metadata.append(&mut other.metadata);
    }
}

/// Render a complete markdown string into styled ratatui lines.
///
/// This is a convenience for non-streaming use cases where the full
/// markdown is already available (e.g. a committed assistant message).
/// For streaming content, use `render_streaming` instead.
pub fn render_markdown(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    render_markdown_with_metadata(text, width, theme).lines
}

/// Render markdown with copy bounds and soft-wrap information for every row.
pub fn render_markdown_with_metadata(text: &str, width: u16, theme: &Theme) -> RenderedLines {
    let mut stream = mdstream::MdStream::new(mdstream::Options::default());
    let mut doc_state = mdstream::DocumentState::new();
    let update = stream.append(text);
    doc_state.apply(update);
    let finalize = stream.finalize();
    doc_state.apply(finalize);

    let mut highlighter = highlight::SyntectHighlighter::default();
    render_doc_state(&doc_state, width, theme, &mut highlighter)
}

/// Render a `DocumentState` including any pending (incomplete) blocks.
///
/// Use this for incremental/streaming rendering where the markdown is
/// still being received.
pub fn render_streaming(
    state: &mdstream::DocumentState,
    width: u16,
    theme: &Theme,
    highlighter: &mut dyn highlight::Highlighter,
) -> Vec<Line<'static>> {
    render_doc_state(state, width, theme, highlighter).lines
}

fn render_doc_state(
    state: &mdstream::DocumentState,
    width: u16,
    theme: &Theme,
    highlighter: &mut dyn highlight::Highlighter,
) -> RenderedLines {
    let mut lines = RenderedLines::default();
    for block in state.committed() {
        let block_lines = render::render_block_with_metadata(block, width, theme, highlighter);
        if !lines.lines.is_empty() && !block_lines.lines.is_empty() {
            lines.push_blank();
        }
        lines.append(block_lines);
    }
    if let Some(pending) = state.pending() {
        let pending_ref = mdstream::PendingBlockRef {
            id: pending.id,
            kind: pending.kind,
            raw: &pending.raw,
            display: pending.display.as_deref(),
        };
        let pending_lines = render::render_pending_with_metadata(
            &pending_ref,
            width,
            theme,
            highlighter,
            &pending::FullPending,
        );
        if !lines.lines.is_empty() && !pending_lines.lines.is_empty() {
            lines.push_blank();
        }
        lines.append(pending_lines);
    }
    lines
}
