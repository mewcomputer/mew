//! Persona accent colors.
//!
//! Each persona gets a deterministic accent color derived from its name
//! (via a hash → HSV → RGB mapping). This color is used for the persona
//! pill in the status bar, the persona entry in the sidebar, and the
//! confirm modal border. Users can override the color via frontmatter
//! `color: "#rrggbb"`.

use ratatui::style::Color;

/// A persona accent color pair: foreground (for text on the accent) and
/// background (the accent itself).
#[derive(Debug, Clone, Copy)]
pub struct AccentColor {
    pub fg: Color,
    pub bg: Color,
}

/// Compute a deterministic accent color for a persona name.
///
/// The name is hashed with a simple but well-distributed FNV-1a variant,
/// then mapped to an HSV color with fixed saturation (0.65) and value (0.85)
/// to keep colors visually distinct but consistently "muted enough" for
/// text backgrounds. The hash determines the hue (0–360°).
///
/// Returns (fg, bg) where `fg` is a light color readable on `bg`.
pub fn accent_for_name(name: &str) -> AccentColor {
    let hash = fnv1a(name.as_bytes());
    let hue = (hash % 360) as f64;
    let (r, g, b) = hsv_to_rgb(hue, 0.65, 0.85);
    let bg = Color::Rgb(r, g, b);
    // Foreground: white or near-white text on the colored background.
    let fg = Color::Rgb(250, 245, 255);
    AccentColor { fg, bg }
}

/// Parse a hex color string like "#rrggbb" or "rrggbb" into a `Color`.
/// Returns `None` if the string is not a valid 6-digit hex color.
pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// Build an `AccentColor` from an explicit hex background color.
/// The foreground is derived as a light or dark color depending on the
/// background's luminance, for readability.
pub fn accent_from_hex(hex: &str) -> Option<AccentColor> {
    let bg = parse_hex(hex)?;
    if let Color::Rgb(r, g, b) = bg {
        // Relative luminance (perception-weighted).
        let lum = 0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64);
        let fg = if lum > 140.0 {
            Color::Rgb(20, 20, 25) // dark text on light bg
        } else {
            Color::Rgb(250, 245, 255) // light text on dark bg
        };
        Some(AccentColor { fg, bg })
    } else {
        None
    }
}

/// Get the accent color for a persona, preferring an explicit `color`
/// field and falling back to the deterministic hash-based color.
pub fn persona_accent(name: &str, explicit_color: Option<&str>) -> AccentColor {
    if let Some(hex) = explicit_color {
        if let Some(a) = accent_from_hex(hex) {
            return a;
        }
    }
    accent_for_name(name)
}

// ---- Internal helpers ----

/// FNV-1a hash (32-bit). Simple, fast, well-distributed for short strings.
fn fnv1a(data: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for &b in data {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// HSV to RGB conversion. Returns (r, g, b) as u8 values.
fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_colors() {
        let a = accent_for_name("planner");
        let b = accent_for_name("planner");
        assert_eq!(a.bg, b.bg, "same name must produce same color");
    }

    #[test]
    fn test_different_names_different_colors() {
        let a = accent_for_name("planner");
        let b = accent_for_name("builder");
        assert_ne!(
            a.bg, b.bg,
            "different names should produce different colors"
        );
    }

    #[test]
    fn test_parse_hex_valid() {
        assert_eq!(parse_hex("#ff8800"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_hex("ff8800"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_hex("#aabbcc"), Some(Color::Rgb(170, 187, 204)));
    }

    #[test]
    fn test_parse_hex_invalid() {
        assert_eq!(parse_hex("#ff"), None);
        assert_eq!(parse_hex("xyzxyz"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn test_accent_from_hex_dark_bg() {
        let a = accent_from_hex("#3a1f55").unwrap();
        // Dark background → light foreground.
        if let Color::Rgb(r, _, _) = a.fg {
            assert!(r > 200, "fg should be light on dark bg");
        }
    }

    #[test]
    fn test_accent_from_hex_light_bg() {
        let a = accent_from_hex("#f0e68c").unwrap();
        // Light background → dark foreground.
        if let Color::Rgb(r, _, _) = a.fg {
            assert!(r < 50, "fg should be dark on light bg");
        }
    }

    #[test]
    fn test_persona_accent_prefers_explicit() {
        let explicit = persona_accent("planner", Some("#ff0000"));
        let implicit = persona_accent("planner", None);
        assert_ne!(explicit.bg, implicit.bg);
        if let Color::Rgb(r, _, _) = explicit.bg {
            assert_eq!(r, 255);
        }
    }

    #[test]
    fn test_persona_accent_falls_back_on_invalid_hex() {
        let bad = persona_accent("planner", Some("not-a-color"));
        let none = persona_accent("planner", None);
        assert_eq!(
            bad.bg, none.bg,
            "invalid hex should fall back to hash color"
        );
    }
}
