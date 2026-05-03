use ratatui::text::{Line, Span};

use crate::{highlight::Highlighter, render, theme::Theme};

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
        let block = mdstream::Block {
            id: mdstream::BlockId(0),
            status: mdstream::BlockStatus::Pending,
            kind,
            raw: text.to_string(),
            display: None,
        };
        render::render_block(&block, width, theme, highlighter)
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
        let block = mdstream::Block {
            id: mdstream::BlockId(0),
            status: mdstream::BlockStatus::Pending,
            kind,
            raw: text.to_string(),
            display: None,
        };
        let lines = render::render_block(&block, width, theme, highlighter);

        // Only elide code fences by default.
        if kind != mdstream::BlockKind::CodeFence || lines.len() <= self.max_lines {
            return lines;
        }

        let total = lines.len();
        let mut kept = Vec::new();
        if let Some(first) = lines.first() {
            kept.push(first.clone());
        }
        let hint = Line::from(Span::styled(
            format!("… generating more … (showing last {} of {} lines)",
                self.max_lines.saturating_sub(2),
                total.saturating_sub(1)),
            theme.pending_indicator,
        ));
        kept.push(hint);

        let tail_start = total.saturating_sub(self.max_lines.saturating_sub(2));
        kept.extend(lines.into_iter().skip(tail_start));

        kept
    }
}
