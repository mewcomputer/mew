use super::constants::{PADDING_LEFT, PADDING_TOP};
use super::metrics::CellMetrics;
use gpui::Point;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridPos {
    pub col: usize,
    pub row: usize,
}

#[allow(dead_code)]
impl GridPos {
    pub fn new(col: usize, row: usize) -> Self {
        Self { col, row }
    }

    pub fn from_pixel(pos: Point<gpui::Pixels>, metrics: &CellMetrics) -> Self {
        let x = f32::from(pos.x) - PADDING_LEFT;
        let y = f32::from(pos.y) - PADDING_TOP;
        GridPos {
            col: ((x / metrics.width).max(0.0) as usize),
            row: ((y / metrics.height).max(0.0) as usize),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub start: GridPos,
    pub end: GridPos,
}

impl Selection {
    pub fn new(start: GridPos, end: GridPos) -> Self {
        Self { start, end }
    }

    pub fn ordered(&self) -> (GridPos, GridPos) {
        if self.start.row < self.end.row
            || (self.start.row == self.end.row && self.start.col <= self.end.col)
        {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }

    pub fn contains(&self, pos: GridPos) -> bool {
        let (s, e) = self.ordered();
        if pos.row < s.row || pos.row > e.row {
            return false;
        }
        if pos.row == s.row && pos.col < s.col {
            return false;
        }
        if pos.row == e.row && pos.col > e.col {
            return false;
        }
        true
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}
