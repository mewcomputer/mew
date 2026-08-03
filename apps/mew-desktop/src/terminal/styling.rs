use gpui::{px, FontStyle, FontWeight, HighlightStyle, Hsla, StrikethroughStyle, UnderlineStyle};
use libghostty_vt::style::Style;

pub fn hsla_from_rgb(r: u8, g: u8, b: u8) -> Hsla {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;

    if (max - min).abs() < f32::EPSILON {
        return Hsla {
            h: 0.0,
            s: 0.0,
            l,
            a: 1.0,
        };
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f32::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;

    Hsla { h, s, l, a: 1.0 }
}

pub fn style_to_highlight(
    style: &Style,
    fg: Option<Hsla>,
    bg: Option<Hsla>,
    default_fg: Hsla,
    default_bg: Hsla,
    selection_bg: Option<Hsla>,
    is_selected: bool,
) -> HighlightStyle {
    let (resolved_fg, resolved_bg) = if style.inverse {
        let inv_fg = bg.unwrap_or(default_bg);
        let inv_bg = fg.unwrap_or(default_fg);
        (Some(inv_fg), Some(inv_bg))
    } else {
        let normal_fg = fg;
        let normal_bg = if is_selected { selection_bg.or(bg) } else { bg };
        (normal_fg, normal_bg)
    };

    HighlightStyle {
        color: resolved_fg.or(Some(default_fg)),
        background_color: resolved_bg.or(Some(default_bg)),
        font_weight: if style.bold {
            Some(FontWeight::BOLD)
        } else if style.faint {
            Some(FontWeight::LIGHT)
        } else {
            None
        },
        font_style: if style.italic {
            Some(FontStyle::Italic)
        } else {
            None
        },
        underline: if !matches!(style.underline, libghostty_vt::style::Underline::None) {
            Some(UnderlineStyle {
                color: None,
                thickness: px(1.0),
                wavy: false,
            })
        } else {
            None
        },
        strikethrough: if style.strikethrough {
            Some(StrikethroughStyle {
                color: None,
                thickness: px(1.0),
            })
        } else {
            None
        },
        fade_out: if style.blink { Some(0.5) } else { None },
    }
}
