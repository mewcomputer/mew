use ratatui::text::{Line, Span};

use crate::{highlight::Highlighter, render, theme::Theme, LineMetadata, RenderedLines};

/// Policy for rendering pending (incomplete) blocks.
pub trait PendingPolicy: Send + Sync {
    fn render(
        &self,
        kind: mdstream::BlockKind,
        text: &str,
        width: u16,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
    ) -> Vec<Line<'static>>;

    /// Custom policies default to copying each rendered row as a hard line.
    /// Built-in policies preserve the underlying block's source bounds.
    fn render_with_metadata(
        &self,
        kind: mdstream::BlockKind,
        text: &str,
        width: u16,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
    ) -> RenderedLines {
        RenderedLines::from_lines(self.render(kind, text, width, theme, highlighter))
    }
}

/// Render the full pending block with no truncation.
pub struct FullPending;

impl PendingPolicy for FullPending {
    fn render(
        &self,
        kind: mdstream::BlockKind,
        text: &str,
        width: u16,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
    ) -> Vec<Line<'static>> {
        self.render_with_metadata(kind, text, width, theme, highlighter)
            .lines
    }

    fn render_with_metadata(
        &self,
        kind: mdstream::BlockKind,
        text: &str,
        width: u16,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
    ) -> RenderedLines {
        let block = mdstream::Block {
            id: mdstream::BlockId(0),
            status: mdstream::BlockStatus::Pending,
            kind,
            raw: text.to_string(),
            display: None,
        };
        render::render_block_with_metadata(&block, width, theme, highlighter)
    }
}

/// Elide the head and tail of long pending blocks.
pub struct ElideHeadAndTail {
    pub max_lines: usize,
}

impl Default for ElideHeadAndTail {
    fn default() -> Self {
        Self { max_lines: 40 }
    }
}

impl PendingPolicy for ElideHeadAndTail {
    fn render(
        &self,
        kind: mdstream::BlockKind,
        text: &str,
        width: u16,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
    ) -> Vec<Line<'static>> {
        self.render_with_metadata(kind, text, width, theme, highlighter)
            .lines
    }

    fn render_with_metadata(
        &self,
        kind: mdstream::BlockKind,
        text: &str,
        width: u16,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
    ) -> RenderedLines {
        let block = mdstream::Block {
            id: mdstream::BlockId(0),
            status: mdstream::BlockStatus::Pending,
            kind,
            raw: text.to_string(),
            display: None,
        };
        let mut rendered = render::render_block_with_metadata(&block, width, theme, highlighter);

        // Only elide code fences by default.
        if kind != mdstream::BlockKind::CodeFence || rendered.lines.len() <= self.max_lines {
            return rendered;
        }

        let total = rendered.lines.len();
        let mut kept = RenderedLines::default();
        if let Some(first) = rendered.lines.first() {
            kept.push(first.clone(), rendered.metadata[0].clone());
        }
        let hint = Line::from(Span::styled(
            format!(
                "… generating more … (showing last {} of {} lines)",
                self.max_lines.saturating_sub(2),
                total.saturating_sub(1)
            ),
            theme.pending_indicator,
        ));
        let hint_end = hint.spans.iter().map(|span| span.content.len()).sum();
        kept.push(
            hint,
            LineMetadata {
                content: 0..hint_end,
                continues_previous: false,
            },
        );

        let tail_start = total.saturating_sub(self.max_lines.saturating_sub(2));
        kept.lines.extend(rendered.lines.drain(tail_start..));
        kept.metadata.extend(rendered.metadata.drain(tail_start..));

        kept
    }
}
