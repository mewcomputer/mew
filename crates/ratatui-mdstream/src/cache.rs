use ratatui::text::Line;

use crate::highlight::Highlighter;
use crate::render::render_block;
use crate::theme::Theme;

/// Incremental render cache keyed by width.
pub struct RenderCache {
    pub width: u16,
    pub committed: Vec<CachedBlock>,
    pub committed_count: usize,
}

impl RenderCache {
    pub fn new() -> Self {
        Self {
            width: 0,
            committed: Vec::new(),
            committed_count: 0,
        }
    }

    pub fn invalidate(&mut self) {
        self.width = 0;
        self.committed.clear();
        self.committed_count = 0;
    }

    /// Extend the cache with any new committed blocks beyond `committed_count`.
    pub fn extend(
        &mut self,
        blocks: &[mdstream::Block],
        width: u16,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
    ) {
        if self.width != width {
            self.invalidate();
            self.width = width;
        }

        if self.committed_count > blocks.len() {
            self.invalidate();
            self.width = width;
        }

        for block in &blocks[self.committed_count..] {
            let lines = render_block(block, width, theme, highlighter);
            self.committed.push(CachedBlock {
                id: block.id,
                lines,
            });
        }
        self.committed_count = blocks.len();
    }

    /// Total number of lines in the cache.
    pub fn total_lines(&self) -> usize {
        self.committed
            .iter()
            .map(|b| b.lines.len() + 1)
            .sum::<usize>()
            .saturating_sub(1)
    }

    /// Collect all cached lines into a flat Vec.
    pub fn collect_lines(&self) -> Vec<Line<'static>> {
        let mut out = Vec::with_capacity(self.total_lines());
        for (i, block) in self.committed.iter().enumerate() {
            if i > 0 {
                out.push(Line::from(""));
            }
            out.extend(block.lines.iter().cloned());
        }
        out
    }
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CachedBlock {
    pub id: mdstream::BlockId,
    pub lines: Vec<Line<'static>>,
}
