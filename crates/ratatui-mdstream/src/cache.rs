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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::inline::StyledRun;
    use ratatui::style::Style;

    /// Counting highlighter that tracks how many times `highlight_line` is called.
    struct CountingHighlighter {
        count: usize,
    }

    impl CountingHighlighter {
        fn new() -> Self {
            Self { count: 0 }
        }
    }

    impl Highlighter for CountingHighlighter {
        fn highlight_line(&mut self, _lang: Option<&str>, line: &str) -> Vec<StyledRun> {
            self.count += 1;
            vec![(line.to_string(), Style::default())]
        }
    }

    fn make_block(id: u64, text: &str) -> mdstream::Block {
        mdstream::Block {
            id: mdstream::BlockId(id),
            status: mdstream::BlockStatus::Committed,
            kind: mdstream::BlockKind::CodeFence,
            raw: format!("```rust\n{}\n```", text),
            display: None,
        }
    }

    /// AC.11: Streaming render cost stays flat across a long multi-code-block
    /// answer. Each block should be rendered exactly once — calling `extend`
    /// with progressively more blocks must not re-render previously committed
    /// blocks.
    #[test]
    fn test_render_cache_flat_cost() {
        let theme = Theme::default();
        let mut highlighter = CountingHighlighter::new();
        let mut cache = RenderCache::new();

        // Build 10 code-fence blocks.
        let blocks: Vec<mdstream::Block> = (0..10)
            .map(|i| make_block(i, &format!("let x = {};", i)))
            .collect();

        // Simulate incremental arrival: extend with 1, 2, 3, ..., 10 blocks.
        for n in 1..=10 {
            cache.extend(&blocks[..n], 80, &theme, &mut highlighter);
        }

        // Each code fence has exactly one line of code content, so
        // highlight_line is called once per block per render.
        // If flat-cost: 10 calls total (each block rendered once).
        // If broken (re-rendering all): 1+2+3+...+10 = 55 calls.
        assert_eq!(
            highlighter.count, 10,
            "each block should be rendered exactly once; got {} highlight_line calls \
             (expected 10, a broken cache would give 55)",
            highlighter.count
        );
    }

    /// Extending with the same blocks (no growth) should not re-render.
    #[test]
    fn test_render_cache_no_redundant_render() {
        let theme = Theme::default();
        let mut highlighter = CountingHighlighter::new();
        let mut cache = RenderCache::new();

        let blocks: Vec<mdstream::Block> = (0..5)
            .map(|i| make_block(i, &format!("let v = {};", i)))
            .collect();

        // First extend: renders 5 blocks.
        cache.extend(&blocks, 80, &theme, &mut highlighter);
        assert_eq!(highlighter.count, 5);

        // Second extend with same blocks: should render 0 new blocks.
        cache.extend(&blocks, 80, &theme, &mut highlighter);
        assert_eq!(
            highlighter.count, 5,
            "re-extending with same blocks should not re-render"
        );
    }

    /// Width change invalidates and re-renders everything.
    #[test]
    fn test_render_cache_width_change_invalidates() {
        let theme = Theme::default();
        let mut highlighter = CountingHighlighter::new();
        let mut cache = RenderCache::new();

        let blocks: Vec<mdstream::Block> = (0..3)
            .map(|i| make_block(i, &format!("let z = {};", i)))
            .collect();

        cache.extend(&blocks, 80, &theme, &mut highlighter);
        assert_eq!(highlighter.count, 3);

        // Width change → invalidate → re-render all 3.
        cache.extend(&blocks, 100, &theme, &mut highlighter);
        assert_eq!(highlighter.count, 6, "width change should re-render all");
    }
}
