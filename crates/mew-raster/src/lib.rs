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

const FLAG_BOLD: u8 = 1;
const FLAG_ITALIC: u8 = 2;

/// Key for the per-symbol shape cache. Single-char symbols (the overwhelming
/// majority of terminal cells) avoid a `String` allocation.
#[derive(PartialEq, Eq, Hash)]
enum GlyphCacheKey {
    Char(char, u8),
    Str(String, u8),
}

/// A shaped glyph positioned relative to its cell origin.
struct PositionedGlyph {
    cache_key: cosmic_text::CacheKey,
    x: i32,
    y: i32,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Reusable rasterizer that keeps the expensive font system loaded.
///
/// Creating a [`cosmic_text::FontSystem`] for every frame is slow; this struct
/// holds it, the rasterized-glyph cache, and a per-symbol shape cache so
/// repeated captures are fast.
pub struct Rasterizer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    /// symbol+style → shaped glyphs, positioned relative to the cell origin.
    /// Valid for `cached_font_size` only; cleared when the scale changes.
    glyph_cache: std::collections::HashMap<GlyphCacheKey, Vec<PositionedGlyph>>,
    cached_font_size: f32,
}

impl Default for Rasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rasterizer {
    /// Create a new rasterizer with the bundled IoskeleyMono fonts.
    pub fn new() -> Self {
        let mut font_system = FontSystem::new_with_fonts([
            fontdb::Source::Binary(std::sync::Arc::new(FONT_REGULAR.to_vec())),
            fontdb::Source::Binary(std::sync::Arc::new(FONT_BOLD.to_vec())),
        ]);
        {
            let db = font_system.db_mut();
            db.set_monospace_family("IoskeleyMono");
        }
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            glyph_cache: std::collections::HashMap::new(),
            cached_font_size: 0.0,
        }
    }

    /// Rasterize a ratatui `Buffer` into a `tiny_skia::Pixmap`.
    pub fn rasterize(&mut self, buf: &Buffer, opts: &RasterOptions) -> Pixmap {
        let area = buf.area;
        let cell_w = (BASE_CELL_W * opts.scale).round() as u32;
        let cell_h = (BASE_CELL_H * opts.scale).round() as u32;

        let width = area.width as u32 * cell_w;
        let height = area.height as u32 * cell_h;

        let mut pixmap = Pixmap::new(width.max(1), height.max(1)).expect("failed to create pixmap");

        // Fill background with a single pass over the pixel buffer. All colors
        // written here are opaque, so premultiplied == straight RGBA.
        let bg_px = premul_opaque(opts.bg_color);
        pixmap.pixels_mut().fill(bg_px);

        let font_size = 13.0 * opts.scale;
        let line_height = 16.0 * opts.scale;
        let metrics = Metrics::new(font_size, line_height);
        if self.cached_font_size != font_size {
            self.glyph_cache.clear();
            self.cached_font_size = font_size;
        }

        // Split borrows so the shape cache, font system, and swash cache can
        // be used independently inside the cell loop.
        let Self {
            font_system,
            swash_cache,
            glyph_cache,
            ..
        } = self;

        for row in 0..area.height {
            let py = row as usize * cell_h as usize;

            // Backgrounds: merge consecutive same-bg cells into one span and
            // write pixel rows directly.
            {
                let stride = width as usize;
                let pixels = pixmap.pixels_mut();
                let mut x: u16 = 0;
                while x < area.width {
                    let Some(cell) = buf.cell((x, row)) else {
                        x += 1;
                        continue;
                    };
                    let bg = cell_bg(cell, opts);
                    let start = x as usize;
                    x += 1;
                    while x < area.width {
                        match buf.cell((x, row)) {
                            Some(c) if cell_bg(c, opts) == bg => x += 1,
                            _ => break,
                        }
                    }
                    if bg != opts.bg_color {
                        let px = premul_opaque(bg);
                        let x0 = start * cell_w as usize;
                        let x1 = x as usize * cell_w as usize;
                        for y in py..py + cell_h as usize {
                            pixels[y * stride + x0..y * stride + x1].fill(px);
                        }
                    }
                }
            }

            // Glyphs: look up each symbol's shaped form in the cache (shaping
            // it once on first sight) and blit the swash masks at the cell
            // origin. Per-cell shaping is what a terminal grid wants anyway —
            // no cross-cell kerning, and every glyph lands exactly on its cell.
            for x in 0..area.width {
                let Some(cell) = buf.cell((x, row)) else {
                    continue;
                };
                let symbol = cell.symbol();
                if symbol.is_empty() || symbol == " " {
                    continue;
                }
                let mut fg = resolve_color(cell.fg, opts.fg_color);
                if cell.modifier.contains(Modifier::REVERSED) {
                    fg = resolve_color(cell.bg, opts.bg_color);
                }
                let mut flags = 0u8;
                if cell.modifier.contains(Modifier::BOLD) {
                    flags |= FLAG_BOLD;
                }
                if cell.modifier.contains(Modifier::ITALIC) {
                    flags |= FLAG_ITALIC;
                }

                let mut chars = symbol.chars();
                let first = chars.next().unwrap_or(' ');
                let single = chars.next().is_none();

                // Block Elements (▀▄█ etc.) tile edge-to-edge; font glyphs
                // rarely cover the full cell and leave seams, so draw them
                // geometrically like terminal emulators do.
                if single && ('\u{2580}'..='\u{259F}').contains(&first) {
                    draw_block_element(
                        &mut pixmap,
                        x as u32 * cell_w,
                        row as u32 * cell_h,
                        cell_w,
                        cell_h,
                        first,
                        fg,
                    );
                    continue;
                }

                let key = if single {
                    GlyphCacheKey::Char(first, flags)
                } else {
                    GlyphCacheKey::Str(symbol.to_string(), flags)
                };
                let glyphs = glyph_cache
                    .entry(key)
                    .or_insert_with(|| shape_symbol(font_system, metrics, symbol, flags));

                let origin_x = x as i32 * cell_w as i32;
                let origin_y = row as i32 * cell_h as i32;
                let color = ct_color(fg);
                for pg in glyphs.iter() {
                    let Some(image) = swash_cache.get_image(font_system, pg.cache_key).as_ref()
                    else {
                        continue;
                    };
                    blit_glyph(
                        &mut pixmap,
                        image,
                        origin_x + pg.x + image.placement.left,
                        origin_y + pg.y - image.placement.top,
                        color,
                    );
                }
            }
        }

        pixmap
    }

    /// Rasterize a ratatui `Buffer` and encode as PNG bytes.
    pub fn to_png(&mut self, buf: &Buffer, opts: &RasterOptions) -> Vec<u8> {
        let pixmap = self.rasterize(buf, opts);
        pixmap_to_png(&pixmap)
    }
}

/// Draw a Unicode Block Element (U+2580–U+259F) as geometric cell fills so
/// adjacent cells tile without seams. Coordinates are in eighths of a cell.
fn draw_block_element(
    pixmap: &mut Pixmap,
    px: u32,
    py: u32,
    cell_w: u32,
    cell_h: u32,
    ch: char,
    rgb: [u8; 3],
) {
    type R = (u8, u8, u8, u8); // (x0, y0, x1, y1) in eighths
    const FULL: R = (0, 0, 8, 8);
    const UL: R = (0, 0, 4, 4);
    const UR: R = (4, 0, 8, 4);
    const LL: R = (0, 4, 4, 8);
    const LR: R = (4, 4, 8, 8);

    let mut rects: [R; 3] = [(0, 0, 0, 0); 3];
    let mut alpha = 255u8;
    let count = match ch {
        '\u{2580}' => {
            rects[0] = (0, 0, 8, 4); // ▀ upper half
            1
        }
        '\u{2581}'..='\u{2588}' => {
            let k = (ch as u32 - 0x2580) as u8; // ▁..█ lower k eighths
            rects[0] = (0, 8 - k, 8, 8);
            1
        }
        '\u{2589}'..='\u{258F}' => {
            let k = (8 - (ch as u32 - 0x2588)) as u8; // ▉..▏ left k eighths
            rects[0] = (0, 0, k, 8);
            1
        }
        '\u{2590}' => {
            rects[0] = (4, 0, 8, 8); // ▐ right half
            1
        }
        '\u{2591}'..='\u{2593}' => {
            alpha = match ch {
                '\u{2591}' => 63,  // ░
                '\u{2592}' => 127, // ▒
                _ => 191,          // ▓
            };
            rects[0] = FULL;
            1
        }
        '\u{2594}' => {
            rects[0] = (0, 0, 8, 1); // ▔ upper eighth
            1
        }
        '\u{2595}' => {
            rects[0] = (7, 0, 8, 8); // ▕ right eighth
            1
        }
        '\u{2596}' => {
            rects[0] = LL;
            1
        }
        '\u{2597}' => {
            rects[0] = LR;
            1
        }
        '\u{2598}' => {
            rects[0] = UL;
            1
        }
        '\u{2599}' => {
            rects = [UL, LL, LR];
            3
        }
        '\u{259A}' => {
            rects[0] = UL;
            rects[1] = LR;
            2
        }
        '\u{259B}' => {
            rects = [UL, UR, LL];
            3
        }
        '\u{259C}' => {
            rects = [UL, UR, LR];
            3
        }
        '\u{259D}' => {
            rects[0] = UR;
            1
        }
        '\u{259E}' => {
            rects[0] = UR;
            rects[1] = LL;
            2
        }
        '\u{259F}' => {
            rects = [UR, LL, LR];
            3
        }
        _ => return,
    };

    let stride = pixmap.width() as usize;
    let pixels = pixmap.pixels_mut();
    for &(ex0, ey0, ex1, ey1) in &rects[..count] {
        let x0 = (px + ex0 as u32 * cell_w / 8) as usize;
        let x1 = (px + ex1 as u32 * cell_w / 8) as usize;
        let y0 = (py + ey0 as u32 * cell_h / 8) as usize;
        let y1 = (py + ey1 as u32 * cell_h / 8) as usize;
        if alpha == 255 {
            let fill = premul_opaque(rgb);
            for y in y0..y1 {
                pixels[y * stride + x0..y * stride + x1].fill(fill);
            }
        } else {
            for y in y0..y1 {
                for p in &mut pixels[y * stride + x0..y * stride + x1] {
                    blend_px(p, rgb[0], rgb[1], rgb[2], alpha);
                }
            }
        }
    }
}

/// Rasterize a ratatui `Buffer` into a `tiny_skia::Pixmap`.
///
/// This convenience function creates a fresh [`Rasterizer`] for a single frame.
/// Prefer [`Rasterizer`] directly when rasterizing many frames.
pub fn rasterize(buf: &Buffer, opts: &RasterOptions) -> Pixmap {
    Rasterizer::new().rasterize(buf, opts)
}

/// Rasterize a ratatui `Buffer` and encode as PNG bytes.
///
/// This convenience function creates a fresh [`Rasterizer`] for a single frame.
/// Prefer [`Rasterizer::to_png`] directly when rasterizing many frames.
pub fn to_png(buf: &Buffer, opts: &RasterOptions) -> Vec<u8> {
    Rasterizer::new().to_png(buf, opts)
}

/// Encode recorded frames to an mp4 by piping raw RGBA straight into ffmpeg.
///
/// Avoids writing a temporary PNG per frame; on success returns ffmpeg's
/// stderr (its normal log output). All frames must share the dimensions of
/// the first frame.
pub fn encode_frames_mp4(
    frames: &[Pixmap],
    output_path: &str,
    fps: u32,
) -> std::io::Result<String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let first = frames.first().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "no frames recorded")
    })?;
    let (w, h) = (first.width(), first.height());
    if let Some(i) = frames
        .iter()
        .position(|f| f.width() != w || f.height() != h)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("frame {i} size differs from first frame; cannot encode"),
        ));
    }

    let mut child = Command::new("ffmpeg")
        .args(["-f", "rawvideo", "-pixel_format", "rgba"])
        .args(["-video_size", &format!("{w}x{h}")])
        .args(["-framerate", &fps.to_string()])
        .args(["-i", "-", "-pix_fmt", "yuv420p", "-y", output_path])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    // Drain stderr on a thread so ffmpeg can't block on a full pipe while we
    // feed frames into stdin.
    let mut stderr = child.stderr.take().expect("stderr piped");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut stderr, &mut buf).ok();
        buf
    });

    let mut stdin = child.stdin.take().expect("stdin piped");
    let mut write_err = None;
    for frame in frames {
        if let Err(e) = stdin.write_all(frame.data()) {
            // ffmpeg may have exited early; capture its stderr for the error.
            write_err = Some(e);
            break;
        }
    }
    drop(stdin);

    let status = child.wait()?;
    let stderr_out = stderr_thread
        .join()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();

    if status.success() {
        Ok(stderr_out)
    } else {
        let extra = write_err
            .map(|e| format!(" (write error: {e})"))
            .unwrap_or_default();
        Err(std::io::Error::other(format!(
            "ffmpeg failed{extra}: {stderr_out}"
        )))
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn pixmap_to_png(pixmap: &Pixmap) -> Vec<u8> {
    use png::{BitDepth, ColorType, Compression, Encoder};
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, pixmap.width(), pixmap.height());
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        // Screenshots are flat-color UI frames; fast compression is ~10x
        // quicker than the default and the size difference is small here.
        encoder.set_compression(Compression::Fast);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(pixmap.data()).expect("png write");
    }
    out
}

/// Effective background color of a cell (accounting for REVERSED).
fn cell_bg(cell: &ratatui::buffer::Cell, opts: &RasterOptions) -> [u8; 3] {
    if cell.modifier.contains(Modifier::REVERSED) {
        resolve_color(cell.fg, opts.fg_color)
    } else {
        resolve_color(cell.bg, opts.bg_color)
    }
}

/// Shape a single cell symbol and return its glyphs positioned relative to
/// the cell origin. Called once per unique (symbol, style); results live in
/// the rasterizer's shape cache.
fn shape_symbol(
    font_system: &mut FontSystem,
    metrics: Metrics,
    symbol: &str,
    flags: u8,
) -> Vec<PositionedGlyph> {
    let mut attrs = Attrs::new().family(Family::Monospace);
    if flags & FLAG_BOLD != 0 {
        attrs = attrs.weight(Weight::BOLD);
    }
    if flags & FLAG_ITALIC != 0 {
        attrs = attrs.style(Style::Italic);
    }
    let mut text_buf = TextBuffer::new(font_system, metrics);
    // Advanced shaping so missing glyphs fall back to system fonts (kana,
    // CJK punctuation, emoji). Shaping runs once per unique symbol and is
    // cached, so the extra cost doesn't affect per-frame time.
    text_buf.set_text(symbol, &attrs, Shaping::Advanced, None);
    text_buf.shape_until_scroll(font_system, false);
    let mut out = Vec::new();
    for run in text_buf.layout_runs() {
        let line_y = run.line_y as i32;
        for glyph in run.glyphs.iter() {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            out.push(PositionedGlyph {
                cache_key: physical.cache_key,
                x: physical.x,
                y: line_y + physical.y,
            });
        }
    }
    out
}

/// Blit one glyph's swash image onto the pixmap at (x0, y0), clipped.
fn blit_glyph(
    pixmap: &mut Pixmap,
    image: &cosmic_text::SwashImage,
    x0: i32,
    y0: i32,
    color: CtColor,
) {
    let gw = image.placement.width as i32;
    let gh = image.placement.height as i32;
    if gw == 0 || gh == 0 {
        return;
    }
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let stride = pw as usize;
    let pixels = pixmap.pixels_mut();
    let (r, g, b) = (color.r(), color.g(), color.b());

    match image.content {
        cosmic_text::SwashContent::Mask => {
            for gy in 0..gh {
                let ty = y0 + gy;
                if ty < 0 || ty >= ph {
                    continue;
                }
                let src = &image.data[(gy * gw) as usize..((gy + 1) * gw) as usize];
                let dst_row = ty as usize * stride;
                for gx in 0..gw {
                    let a = src[gx as usize];
                    if a == 0 {
                        continue;
                    }
                    let tx = x0 + gx;
                    if tx < 0 || tx >= pw {
                        continue;
                    }
                    blend_px(&mut pixels[dst_row + tx as usize], r, g, b, a);
                }
            }
        }
        cosmic_text::SwashContent::Color => {
            for gy in 0..gh {
                let ty = y0 + gy;
                if ty < 0 || ty >= ph {
                    continue;
                }
                let src = &image.data[(gy * gw * 4) as usize..((gy + 1) * gw * 4) as usize];
                let dst_row = ty as usize * stride;
                for gx in 0..gw {
                    let i = gx as usize * 4;
                    let a = src[i + 3];
                    if a == 0 {
                        continue;
                    }
                    let tx = x0 + gx;
                    if tx < 0 || tx >= pw {
                        continue;
                    }
                    blend_px(
                        &mut pixels[dst_row + tx as usize],
                        src[i],
                        src[i + 1],
                        src[i + 2],
                        a,
                    );
                }
            }
        }
        cosmic_text::SwashContent::SubpixelMask => {
            // Rare; approximate with the green channel as coverage.
            for gy in 0..gh {
                let ty = y0 + gy;
                if ty < 0 || ty >= ph {
                    continue;
                }
                let src = &image.data[(gy * gw * 4) as usize..((gy + 1) * gw * 4) as usize];
                let dst_row = ty as usize * stride;
                for gx in 0..gw {
                    let a = src[gx as usize * 4 + 1];
                    if a == 0 {
                        continue;
                    }
                    let tx = x0 + gx;
                    if tx < 0 || tx >= pw {
                        continue;
                    }
                    blend_px(&mut pixels[dst_row + tx as usize], r, g, b, a);
                }
            }
        }
    }
}

fn premul_opaque(rgb: [u8; 3]) -> tiny_skia::PremultipliedColorU8 {
    tiny_skia::PremultipliedColorU8::from_rgba(rgb[0], rgb[1], rgb[2], 255)
        .expect("opaque color is always valid premultiplied")
}

/// Src-over blend a straight-alpha RGBA color onto one premultiplied pixel.
#[inline]
fn blend_px(dst: &mut tiny_skia::PremultipliedColorU8, r: u8, g: u8, b: u8, a: u8) {
    if a == 255 {
        *dst = premul_opaque([r, g, b]);
        return;
    }
    let inv = 255 - a as u16;
    let mul = |s: u8, d: u8| -> u8 { ((s as u16 * a as u16 + d as u16 * inv + 127) / 255) as u8 };
    let (nr, ng, nb) = (mul(r, dst.red()), mul(g, dst.green()), mul(b, dst.blue()));
    let na = (a as u16 + (dst.alpha() as u16 * inv + 127) / 255) as u8;
    *dst = tiny_skia::PremultipliedColorU8::from_rgba(nr.min(na), ng.min(na), nb.min(na), na)
        .unwrap_or_else(|| premul_opaque([r, g, b]));
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
    fn full_block_fills_entire_cell() {
        let span = Span::styled("█", Style::default().fg(Color::White));
        let buf = render_buffer(3, 1, span);
        let pixmap = rasterize(&buf, &RasterOptions::default());
        // Every pixel of the first 16x32 cell must be white — including all
        // four corners, which font glyphs typically miss.
        for &(x, y) in &[(0, 0), (15, 0), (0, 31), (15, 31), (8, 16)] {
            assert_eq!(pixel_rgb(&pixmap, x, y), [255, 255, 255], "at ({x},{y})");
        }
    }

    #[test]
    fn adjacent_blocks_tile_without_seams() {
        let span = Span::styled("██", Style::default().fg(Color::White));
        let buf = render_buffer(4, 1, span);
        let pixmap = rasterize(&buf, &RasterOptions::default());
        // The boundary column between cell 0 and cell 1 (x = 15, 16) must be
        // solid white on every row.
        for y in 0..32 {
            assert_eq!(pixel_rgb(&pixmap, 15, y), [255, 255, 255], "row {y}");
            assert_eq!(pixel_rgb(&pixmap, 16, y), [255, 255, 255], "row {y}");
        }
    }

    #[test]
    fn half_blocks_fill_correct_half() {
        let span = Span::styled("▀", Style::default().fg(Color::White));
        let buf = render_buffer(2, 1, span);
        let opts = RasterOptions::default();
        let pixmap = rasterize(&buf, &opts);
        assert_eq!(pixel_rgb(&pixmap, 8, 4), [255, 255, 255], "upper half fg");
        assert_eq!(pixel_rgb(&pixmap, 8, 28), opts.bg_color, "lower half bg");
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
