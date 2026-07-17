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

pub use theme::Theme;
pub use widget::{StreamView, StreamViewState};

/// Render a complete markdown string into styled ratatui lines.
///
/// This is a convenience for non-streaming use cases where the full
/// markdown is already available (e.g. a committed assistant message).
/// For streaming content, use `render_streaming` instead.
pub fn render_markdown(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
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
    render_doc_state(state, width, theme, highlighter)
}

fn render_doc_state(
    state: &mdstream::DocumentState,
    width: u16,
    theme: &Theme,
    highlighter: &mut dyn highlight::Highlighter,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in state.committed() {
        let block_lines = render::render_block(block, width, theme, highlighter);
        if !lines.is_empty() && !block_lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(block_lines);
    }
    if let Some(pending) = state.pending() {
        let pending_ref = mdstream::PendingBlockRef {
            id: pending.id,
            kind: pending.kind,
            raw: &pending.raw,
            display: pending.display.as_deref(),
        };
        let pending_lines = render::render_pending(
            &pending_ref,
            width,
            theme,
            highlighter,
            &pending::FullPending,
        );
        if !lines.is_empty() && !pending_lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(pending_lines);
    }
    lines
}
