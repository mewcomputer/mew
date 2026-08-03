use super::constants::{FALLBACK_CELL_HEIGHT, FALLBACK_CELL_WIDTH, FONT_SIZE, LINE_HEIGHT_FACTOR};
use gpui::{App, Pixels, Window};

#[derive(Clone, Copy, Debug)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    pub font_size: Pixels,
}

impl CellMetrics {
    pub fn measure(window: &Window, family: &str, _cx: &App) -> Self {
        let text_system = window.text_system();
        let font = gpui::font(family);
        let font_size = gpui::px(FONT_SIZE);

        let (cell_w, cell_h) = {
            let font_id = text_system.resolve_font(&font);
            let advance = text_system
                .advance(font_id, font_size, 'M')
                .map(|s| f32::from(s.width));
            let w = advance.unwrap_or(FALLBACK_CELL_WIDTH);
            let h = f32::from(font_size) * LINE_HEIGHT_FACTOR;
            (w, h)
        };

        CellMetrics {
            width: cell_w,
            height: cell_h,
            font_size,
        }
    }

    pub fn fallback() -> Self {
        Self {
            width: FALLBACK_CELL_WIDTH,
            height: FALLBACK_CELL_HEIGHT,
            font_size: gpui::px(FONT_SIZE),
        }
    }
}
