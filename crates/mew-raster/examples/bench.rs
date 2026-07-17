//! Timing harness for the rasterizer: `cargo run -p mew-raster --release --example bench`

use mew_raster::{RasterOptions, Rasterizer};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::time::Instant;

fn build_buffer(width: u16, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let palette = [
        Color::White,
        Color::Rgb(180, 200, 255),
        Color::Rgb(255, 180, 120),
        Color::Green,
        Color::DarkGray,
    ];
    let text = "The quick brown fox jumps over the lazy dog 0123456789 ┌─┐│└┘ ▸ mew ";
    let chars: Vec<char> = text.chars().collect();
    for y in 0..height {
        for x in 0..width {
            let ch = chars[(x as usize + y as usize * 7) % chars.len()];
            let mut style = Style::default()
                .fg(palette[(x as usize / 11 + y as usize) % palette.len()])
                .bg(if y % 9 == 0 {
                    Color::Rgb(40, 40, 48)
                } else {
                    Color::Reset
                });
            if (x / 17) % 3 == 0 {
                style = style.add_modifier(Modifier::BOLD);
            }
            buf.cell_mut((x, y)).unwrap().set_char(ch).set_style(style);
        }
    }
    buf
}

fn main() {
    let buf = build_buffer(160, 48);
    let opts = RasterOptions::default();

    let start = Instant::now();
    let mut rasterizer = Rasterizer::new();
    println!("Rasterizer::new: {:?}", start.elapsed());

    // Warm-up (fills shaping + glyph caches)
    let start = Instant::now();
    let pixmap = rasterizer.rasterize(&buf, &opts);
    println!(
        "rasterize (cold): {:?} ({}x{})",
        start.elapsed(),
        pixmap.width(),
        pixmap.height()
    );

    let iters = 10;
    let start = Instant::now();
    for _ in 0..iters {
        let pm = rasterizer.rasterize(&buf, &opts);
        std::hint::black_box(&pm);
    }
    println!("rasterize (warm avg): {:?}", start.elapsed() / iters);

    let start = Instant::now();
    let png = rasterizer.to_png(&buf, &opts);
    println!(
        "to_png (raster+encode): {:?}, {} bytes",
        start.elapsed(),
        png.len()
    );
}
