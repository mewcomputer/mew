//! Rasterize a ratatui `Buffer` to a PNG image.
//!
//! This is the agent-facing capture path: it turns a TUI frame into a
//! pixel-accurate PNG that a VLM can inspect. Unlike vhs (which drives a
//! real terminal via Chrome), this path is fully deterministic — same
//! buffer in, same pixels out, no fontconfig, no GPU, no network.
//!
//! Uses [`cosmic_text`] for proper glyph rendering — handles box-drawing
//! characters, multi-cell graphemes, bold/italic weights, and font
//! fallback.
//!
//! # Usage
//!
//! ```no_run
//! use mew_raster::{RasterOptions, to_png};
//! use ratatui::buffer::Buffer;
//! use ratatui::layout::Rect;
//!
//! let buffer = Buffer::empty(Rect::new(0, 0, 80, 24));
//! let png_bytes = to_png(&buffer, &RasterOptions::default());
//! std::fs::write("screenshot.png", png_bytes).unwrap();
//! ```

use cosmic_text::{
    Attrs, Buffer as TextBuffer, Color as CtColor, Family, FontSystem, Metrics, Shaping, Style,
    SwashCache, Weight,
};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use tiny_skia::Pixmap;

/// Options controlling raster output.
#[derive(Debug, Clone)]
pub struct RasterOptions {
    /// Pixel scale multiplier. 1.0 = 8×16 px cells, 2.0 = 16×32 (default).
    pub scale: f32,
    /// Background color (RGB) for `Color::Reset` backgrounds.
    pub bg_color: [u8; 3],
    /// Foreground color (RGB) for `Color::Reset` foregrounds.
    pub fg_color: [u8; 3],
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            scale: 2.0,
            // Dark theme defaults matching mew's Theme::dark()
            bg_color: [30, 30, 33],
            fg_color: [255, 255, 255],
        }
    }
}

/// Bundled regular-weight font.
const FONT_REGULAR: &[u8] = include_bytes!("../assets/IoskeleyMono-Regular.ttf");
/// Bundled medium-weight font (used for bold).
const FONT_BOLD: &[u8] = include_bytes!("../assets/IoskeleyMono-Medium.ttf");

/// Base cell dimensions in pixels (before scaling).
const BASE_CELL_W: f32 = 8.0;
const BASE_CELL_H: f32 = 16.0;

/// Span info: (text_start, text_end, fg_rgb, bg_rgb, modifier)
type SpanInfo = (usize, usize, [u8; 3], [u8; 3], Modifier);

/// Cell info: (symbol, fg_rgb, bg_rgb, modifier, x_position)
type CellInfo = (String, [u8; 3], [u8; 3], Modifier, usize);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Rasterize a ratatui `Buffer` into a `tiny_skia::Pixmap`.
pub fn rasterize(buf: &Buffer, opts: &RasterOptions) -> Pixmap {
    let area = buf.area;
    let cell_w = (BASE_CELL_W * opts.scale).round() as u32;
    let cell_h = (BASE_CELL_H * opts.scale).round() as u32;

    let width = area.width as u32 * cell_w;
    let height = area.height as u32 * cell_h;

    let mut pixmap = Pixmap::new(width.max(1), height.max(1)).expect("failed to create pixmap");

    // Fill background
    fill_rect(&mut pixmap, 0, 0, width, height, opts.bg_color);

    // Create font system with bundled fonts
    let mut font_system = FontSystem::new_with_fonts([
        fontdb::Source::Binary(std::sync::Arc::new(FONT_REGULAR.to_vec())),
        fontdb::Source::Binary(std::sync::Arc::new(FONT_BOLD.to_vec())),
    ]);

    // Set the monospace family to our font
    {
        let db = font_system.db_mut();
        db.set_monospace_family("IoskeleyMono");
    }

    let mut swash_cache = SwashCache::new();

    let font_size = 13.0 * opts.scale;
    let line_height = 16.0 * opts.scale;
    let metrics = Metrics::new(font_size, line_height);

    // Render row by row
    for row in 0..area.height {
        // Collect cells for this row
        let cells: Vec<CellInfo> = (0..area.width)
            .filter_map(|x| {
                let cell = buf.cell((x, row))?;
                let symbol = cell.symbol().to_string();
                if symbol.is_empty() {
                    return None;
                }
                let bg = resolve_color(cell.bg, opts.bg_color);
                let mut fg = resolve_color(cell.fg, opts.fg_color);
                let mut bg = bg;
                if cell.modifier.contains(Modifier::REVERSED) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                Some((symbol, fg, bg, cell.modifier, x as usize))
            })
            .collect();

        if cells.is_empty() {
            continue;
        }

        // Build the full line text and track span boundaries
        let mut line_text = String::new();
        let mut spans: Vec<SpanInfo> = Vec::new();
        // Track the previous cell's attrs to merge consecutive cells with same style
        let mut current_start = 0;
        let mut current_fg = cells[0].1;
        let mut current_bg = cells[0].2;
        let mut current_mod = cells[0].3;

        for (symbol, fg, bg, modifier, _x) in &cells {
            // Check if attrs changed (fg, bg, modifier)
            if *fg != current_fg || *bg != current_bg || *modifier != current_mod {
                // Close current span
                spans.push((
                    current_start,
                    line_text.len(),
                    current_fg,
                    current_bg,
                    current_mod,
                ));
                current_start = line_text.len();
                current_fg = *fg;
                current_bg = *bg;
                current_mod = *modifier;
            }
            line_text.push_str(symbol);
        }
        // Close final span
        spans.push((
            current_start,
            line_text.len(),
            current_fg,
            current_bg,
            current_mod,
        ));

        // Fill cell backgrounds first (always, for clean cell boundaries)
        for (_, _fg, bg, _modifier, x) in &cells {
            let px = *x as u32 * cell_w;
            let py = row as u32 * cell_h;
            fill_rect(&mut pixmap, px, py, cell_w, cell_h, *bg);
        }

        // Create cosmic-text buffer for this line
        let default_attrs = Attrs::new()
            .family(Family::Monospace)
            .color(ct_color(opts.fg_color));
        let mut text_buf = TextBuffer::new(&mut font_system, metrics);
        text_buf.set_size(Some(width as f32), Some(line_height));

        // Set text with per-span attrs
        let span_refs: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(start, end, fg, _bg, modifier)| {
                let text = &line_text[*start..*end];
                let mut attrs = Attrs::new().family(Family::Monospace).color(ct_color(*fg));
                if modifier.contains(Modifier::BOLD) {
                    attrs = attrs.weight(Weight::BOLD);
                }
                if modifier.contains(Modifier::ITALIC) {
                    attrs = attrs.style(Style::Italic);
                }
                (text, attrs)
            })
            .collect();

        text_buf.set_rich_text(
            span_refs.into_iter(),
            &default_attrs,
            Shaping::Advanced,
            None,
        );

        // Render the line into our pixmap via draw() callback
        let py_offset = row as i32 * cell_h as i32;
        let pixmap_ptr: *mut Pixmap = &mut pixmap;
        text_buf.draw(
            &mut font_system,
            &mut swash_cache,
            ct_color(opts.fg_color),
            |x, y, w, h, color| {
                let rgba = color.as_rgba();
                let rgb = [rgba[0], rgba[1], rgba[2]];
                let alpha = rgba[3] as f32 / 255.0;
                // Fill the glyph rectangle at the computed position
                let px = (py_offset + y) as u32;
                for off_y in 0..h {
                    for off_x in 0..w {
                        let target_x = (x + off_x as i32) as u32;
                        let target_y = px + off_y;
                        // SAFETY: pixmap_ptr is valid and not aliased during this callback
                        let pm = unsafe { &mut *pixmap_ptr };
                        if target_x < pm.width() && target_y < pm.height() {
                            blend_pixel(
                                pm,
                                target_x,
                                target_y,
                                rgb,
                                if alpha >= 1.0 { 1.0 } else { alpha },
                            );
                        }
                    }
                }
            },
        );
    }

    pixmap
}

/// Rasterize a ratatui `Buffer` and encode as PNG bytes.
pub fn to_png(buf: &Buffer, opts: &RasterOptions) -> Vec<u8> {
    let pixmap = rasterize(buf, opts);
    pixmap_to_png(&pixmap)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn pixmap_to_png(pixmap: &Pixmap) -> Vec<u8> {
    use png::{BitDepth, ColorType, Encoder};
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, pixmap.width(), pixmap.height());
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(pixmap.data()).expect("png write");
    }
    out
}

fn fill_rect(pixmap: &mut Pixmap, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3]) {
    let paint = paint_rgb(rgb);
    if let Some(rect) = tiny_skia::Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
        pixmap.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
    }
}

fn blend_pixel(pixmap: &mut Pixmap, x: u32, y: u32, rgb: [u8; 3], coverage: f32) {
    let alpha = coverage.clamp(0.0, 1.0);
    let idx = ((y * pixmap.width() + x) * 4) as usize;
    let data = pixmap.data_mut();
    if idx + 3 >= data.len() {
        return;
    }
    if alpha >= 1.0 {
        data[idx] = rgb[0];
        data[idx + 1] = rgb[1];
        data[idx + 2] = rgb[2];
        data[idx + 3] = 255;
    } else if alpha > 0.0 {
        let inv = 1.0 - alpha;
        data[idx] = (rgb[0] as f32 * alpha + data[idx] as f32 * inv) as u8;
        data[idx + 1] = (rgb[1] as f32 * alpha + data[idx + 1] as f32 * inv) as u8;
        data[idx + 2] = (rgb[2] as f32 * alpha + data[idx + 2] as f32 * inv) as u8;
        data[idx + 3] = 255;
    }
}

fn paint_rgb(rgb: [u8; 3]) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
    paint.anti_alias = true;
    paint
}

fn ct_color(rgb: [u8; 3]) -> CtColor {
    CtColor::rgb(rgb[0], rgb[1], rgb[2])
}

fn resolve_color(color: Color, default: [u8; 3]) -> [u8; 3] {
    match color {
        Color::Reset => default,
        Color::Black => [0, 0, 0],
        Color::Red => [205, 0, 0],
        Color::Green => [0, 205, 0],
        Color::Yellow => [205, 205, 0],
        Color::Blue => [0, 0, 238],
        Color::Magenta => [205, 0, 205],
        Color::Cyan => [0, 205, 205],
        Color::Gray => [229, 229, 229],
        Color::DarkGray => [127, 127, 127],
        Color::LightRed => [255, 85, 85],
        Color::LightGreen => [85, 255, 85],
        Color::LightYellow => [255, 255, 85],
        Color::LightBlue => [85, 85, 255],
        Color::LightMagenta => [255, 85, 255],
        Color::LightCyan => [85, 255, 255],
        Color::White => [255, 255, 255],
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(idx) => indexed_to_rgb(idx),
    }
}

fn indexed_to_rgb(idx: u8) -> [u8; 3] {
    match idx {
        0 => [0, 0, 0],
        1 => [205, 0, 0],
        2 => [0, 205, 0],
        3 => [205, 205, 0],
        4 => [0, 0, 238],
        5 => [205, 0, 205],
        6 => [0, 205, 205],
        7 => [229, 229, 229],
        8 => [127, 127, 127],
        9 => [255, 85, 85],
        10 => [85, 255, 85],
        11 => [255, 255, 85],
        12 => [85, 85, 255],
        13 => [255, 85, 255],
        14 => [85, 255, 255],
        15 => [255, 255, 255],
        16..=231 => {
            let idx = idx - 16;
            let r = idx / 36;
            let g = (idx % 36) / 6;
            let b = idx % 6;
            let lookup = |v: u8| -> u8 {
                match v {
                    0 => 0,
                    1 => 95,
                    2 => 135,
                    3 => 175,
                    4 => 215,
                    5 => 255,
                    _ => 255,
                }
            };
            [lookup(r), lookup(g), lookup(b)]
        }
        232..=255 => {
            let v = 8 + (idx - 232) * 10;
            [v, v, v]
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;
    use ratatui::widgets::Widget;
    use ratatui::Terminal;

    fn render_buffer(width: u16, height: u16, widget: impl Widget) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| f.render_widget(widget, f.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn pixel_rgb(pixmap: &Pixmap, x: u32, y: u32) -> [u8; 3] {
        let px = pixmap.pixel(x, y).expect("pixel in bounds");
        [px.red(), px.green(), px.blue()]
    }

    #[test]
    fn pixmap_dimensions_match_buffer_size() {
        let buf = render_buffer(80, 24, "Hello");
        let opts = RasterOptions::default();
        let pixmap = rasterize(&buf, &opts);
        assert_eq!(pixmap.width(), 80 * 16);
        assert_eq!(pixmap.height(), 24 * 32);
    }

    #[test]
    fn scale_affects_dimensions() {
        let buf = render_buffer(40, 10, "Hi");
        let opts = RasterOptions {
            scale: 1.0,
            ..Default::default()
        };
        let pixmap = rasterize(&buf, &opts);
        assert_eq!(pixmap.width(), 40 * 8);
        assert_eq!(pixmap.height(), 10 * 16);
    }

    #[test]
    fn background_color_is_filled() {
        let buf = render_buffer(10, 4, "test");
        let opts = RasterOptions {
            bg_color: [50, 60, 70],
            ..Default::default()
        };
        let pixmap = rasterize(&buf, &opts);
        assert_eq!(pixel_rgb(&pixmap, 0, 0), [50, 60, 70]);
    }

    #[test]
    fn red_text_produces_red_pixels() {
        let span = Span::styled("RED", Style::default().fg(Color::Red));
        let buf = render_buffer(10, 3, span);
        let opts = RasterOptions::default();
        let pixmap = rasterize(&buf, &opts);

        let mut found_red = false;
        for y in 0..pixmap.height() {
            for x in 0..pixmap.width() {
                let [r, g, b] = pixel_rgb(&pixmap, x, y);
                if r > 150 && g < 50 && b < 50 {
                    found_red = true;
                    break;
                }
            }
            if found_red {
                break;
            }
        }
        assert!(found_red, "expected to find red pixels from red text");
    }

    #[test]
    fn reversed_modifier_swaps_colors() {
        let span = Span::styled(
            "X",
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::REVERSED),
        );
        let buf = render_buffer(10, 3, span);
        let opts = RasterOptions::default();
        let pixmap = rasterize(&buf, &opts);

        let [r, g, b] = pixel_rgb(&pixmap, 8, 16);
        let is_white = r > 200 && g > 200 && b > 200;
        let is_red = r > 150 && g < 80 && b < 80;
        assert!(
            is_white || is_red,
            "expected white or red pixel from reversed style, got ({}, {}, {})",
            r,
            g,
            b
        );
    }

    #[test]
    fn custom_bg_color_appears_in_output() {
        let span = Span::styled(" ", Style::default().bg(Color::Rgb(100, 150, 200)));
        let buf = render_buffer(5, 3, span);
        let opts = RasterOptions::default();
        let pixmap = rasterize(&buf, &opts);
        assert_eq!(pixel_rgb(&pixmap, 8, 16), [100, 150, 200]);
    }

    #[test]
    fn to_png_produces_valid_png() {
        let buf = render_buffer(20, 5, "test");
        let png_bytes = to_png(&buf, &RasterOptions::default());
        assert!(png_bytes.len() > 8);
        assert_eq!(
            &png_bytes[..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn indexed_color_256_palette() {
        assert_eq!(indexed_to_rgb(196), [255, 0, 0]);
        assert_eq!(indexed_to_rgb(21), [0, 0, 255]);
        assert_eq!(indexed_to_rgb(232), [8, 8, 8]);
        assert_eq!(indexed_to_rgb(255), [238, 238, 238]);
    }

    #[test]
    fn empty_buffer_produces_valid_pixmap() {
        let buf = Buffer::empty(Rect::new(0, 0, 5, 3));
        let pixmap = rasterize(&buf, &RasterOptions::default());
        assert_eq!(pixmap.width(), 5 * 16);
        assert_eq!(pixmap.height(), 3 * 32);
    }

    #[test]
    fn glyph_renders_non_bg_pixels() {
        let span = Span::styled("A", Style::default().fg(Color::White));
        let buf = render_buffer(5, 3, span);
        let opts = RasterOptions::default();
        let pixmap = rasterize(&buf, &opts);

        let bg = opts.bg_color;
        let mut found_glyph_pixel = false;
        for y in 0..pixmap.height() {
            for x in 0..pixmap.width() {
                let [r, g, b] = pixel_rgb(&pixmap, x, y);
                if r > bg[0] + 30 || g > bg[1] + 30 || b > bg[2] + 30 {
                    found_glyph_pixel = true;
                    break;
                }
            }
            if found_glyph_pixel {
                break;
            }
        }
        assert!(
            found_glyph_pixel,
            "expected to find non-bg pixels from glyph rendering"
        );
    }
}
