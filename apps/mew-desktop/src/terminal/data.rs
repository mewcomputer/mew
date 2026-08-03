use gpui::{HighlightStyle, Hsla};

#[derive(Default, Clone)]
pub struct RowData {
    pub text: String,
    pub highlights: Vec<(std::ops::Range<usize>, HighlightStyle)>,
    pub bg_runs: Vec<BgRun>,
}

#[allow(dead_code)]
impl RowData {
    pub fn new(text: String) -> Self {
        Self {
            text,
            highlights: Vec::new(),
            bg_runs: Vec::new(),
        }
    }

    pub fn with_highlight(mut self, range: std::ops::Range<usize>, style: HighlightStyle) -> Self {
        if style != HighlightStyle::default() {
            self.highlights.push((range, style));
        }
        self
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct BgRun {
    pub start_col: usize,
    pub end_col: usize,
    pub color: Hsla,
}

#[allow(dead_code)]
impl BgRun {
    pub fn new(start_col: usize, end_col: usize, color: Hsla) -> Self {
        Self {
            start_col,
            end_col,
            color,
        }
    }
}
