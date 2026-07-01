//! Theme system: token-based colors for the TUI.
//!
//! Based on the shadcn semantic token model, extended with mew-specific
//! tokens. Themes are JSON files that can be shared between the TUI and
//! web UI. See `THEMING_PLAN.md` for the full design.
//!
//! The [`Theme`] struct holds every color the TUI uses. [`Theme::dark`]
//! returns the current hardcoded values (the default). [`Theme::from_json`]
//! loads a theme file, merging over `dark()` defaults for partial themes.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Accent colors (persona-specific)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Theme: full token set
// ---------------------------------------------------------------------------

/// A complete theme for the TUI. Every color the UI uses is here.
/// [`Theme::dark`] returns the default (current hardcoded values).
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub mode: ThemeMode,
    pub tokens: ThemeTokens,
}

/// Light or dark mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

/// A three-tier semantic color scale.
/// `fg` — bright, readable on `med` and `bg`. Used for text on pills.
/// `med` — mid-saturation. Used for borders, icons, markers.
/// `bg` — dark, low-saturation. Used for pill backgrounds, block fills.
#[derive(Debug, Clone, Copy)]
pub struct ColorScale {
    pub fg: Color,
    pub med: Color,
    pub bg: Color,
}

/// Every theme token. Fields use `Color` (not hex strings) — the JSON
/// serde layer converts hex → Color during load.
///
/// Core shadcn tokens match shadcn v4. Semantic color scales are
/// mew extensions. See `THEMING_PLAN.md` for the full rationale.
#[derive(Debug, Clone)]
pub struct ThemeTokens {
    // --- Core shadcn ---
    pub background: Color,
    pub foreground: Color,
    pub card: Color,
    pub card_foreground: Color,
    pub popover: Color,
    pub popover_foreground: Color,
    pub primary: Color,
    pub primary_foreground: Color,
    pub secondary: Color,
    pub secondary_foreground: Color,
    pub muted: Color,
    pub muted_foreground: Color,
    pub accent: Color,
    pub accent_foreground: Color,
    pub destructive: Color,
    pub border: Color,
    pub input: Color,
    pub ring: Color,
    pub sidebar: Color,
    pub sidebar_foreground: Color,
    pub sidebar_primary: Color,
    pub sidebar_primary_foreground: Color,
    pub sidebar_accent: Color,
    pub sidebar_accent_foreground: Color,
    pub sidebar_border: Color,
    pub sidebar_ring: Color,
    pub chart_1: Color,
    pub chart_2: Color,
    pub chart_3: Color,
    pub chart_4: Color,

    // --- mew surface extensions ---
    pub status_bg: Color,
    pub tool_bg: Color,
    pub divider: Color,

    // --- Semantic color scales ---
    pub red: ColorScale,
    pub green: ColorScale,
    pub yellow: ColorScale,
    pub blue: ColorScale,
    pub purple: ColorScale,
    pub cyan: ColorScale,
}

impl Theme {
    /// The default dark theme — matches the hardcoded values that were
    /// in `ui/mod.rs` and `ui/status.rs` before theming.
    pub fn dark() -> Self {
        Self {
            name: "dark".into(),
            mode: ThemeMode::Dark,
            tokens: ThemeTokens {
                // Core shadcn
                background: Color::Rgb(30, 30, 33),
                foreground: Color::White,
                card: Color::Rgb(50, 50, 56),
                card_foreground: Color::White,
                popover: Color::Rgb(30, 30, 33),
                popover_foreground: Color::White,
                primary: Color::Cyan,
                primary_foreground: Color::Rgb(30, 30, 33),
                secondary: Color::Rgb(50, 50, 56),
                secondary_foreground: Color::White,
                muted: Color::Rgb(40, 40, 44),
                muted_foreground: Color::DarkGray,
                accent: Color::Rgb(50, 50, 56),
                accent_foreground: Color::White,
                destructive: Color::Rgb(140, 30, 30),
                border: Color::Rgb(50, 50, 55),
                input: Color::Rgb(30, 30, 33),
                ring: Color::Cyan,
                sidebar: Color::Rgb(28, 28, 31),
                sidebar_foreground: Color::White,
                sidebar_primary: Color::Cyan,
                sidebar_primary_foreground: Color::Rgb(28, 28, 31),
                sidebar_accent: Color::Rgb(50, 50, 56),
                sidebar_accent_foreground: Color::White,
                sidebar_border: Color::Rgb(50, 50, 55),
                sidebar_ring: Color::Cyan,
                chart_1: Color::Rgb(150, 230, 160),
                chart_2: Color::Rgb(150, 190, 240),
                chart_3: Color::Rgb(245, 200, 80),
                chart_4: Color::Rgb(200, 170, 240),

                // mew surfaces
                status_bg: Color::Rgb(30, 30, 33),
                tool_bg: Color::Rgb(50, 50, 56),
                divider: Color::Rgb(50, 50, 55),

                // Semantic scales — derived from the current pill colors
                red: ColorScale {
                    fg: Color::Rgb(255, 240, 240),
                    med: Color::Rgb(180, 40, 40),
                    bg: Color::Rgb(140, 30, 30),
                },
                green: ColorScale {
                    fg: Color::Rgb(150, 230, 160),
                    med: Color::Rgb(40, 120, 50),
                    bg: Color::Rgb(25, 70, 35),
                },
                yellow: ColorScale {
                    fg: Color::Rgb(245, 210, 110),
                    med: Color::Rgb(200, 160, 40),
                    bg: Color::Rgb(75, 60, 20),
                },
                blue: ColorScale {
                    fg: Color::Rgb(150, 190, 240),
                    med: Color::Rgb(50, 90, 140),
                    bg: Color::Rgb(30, 55, 90),
                },
                purple: ColorScale {
                    fg: Color::Rgb(240, 230, 250),
                    med: Color::Rgb(95, 50, 130),
                    bg: Color::Rgb(55, 35, 75),
                },
                cyan: ColorScale {
                    fg: Color::Cyan,
                    med: Color::Rgb(40, 120, 120),
                    bg: Color::Rgb(20, 50, 50),
                },
            },
        }
    }

    /// A light theme — light backgrounds, dark text, inverted pill tiers
    /// (fg=dark for readability on light pill backgrounds).
    pub fn light() -> Self {
        Self {
            name: "light".into(),
            mode: ThemeMode::Light,
            tokens: ThemeTokens {
                // Core shadcn — mirrors the web UI's `:root` light tokens.
                background: Color::Rgb(255, 255, 255),
                foreground: Color::Rgb(15, 15, 20),
                card: Color::Rgb(245, 245, 248),
                card_foreground: Color::Rgb(15, 15, 20),
                popover: Color::Rgb(255, 255, 255),
                popover_foreground: Color::Rgb(15, 15, 20),
                primary: Color::Rgb(30, 30, 40),
                primary_foreground: Color::Rgb(245, 245, 250),
                secondary: Color::Rgb(240, 240, 244),
                secondary_foreground: Color::Rgb(15, 15, 20),
                muted: Color::Rgb(235, 235, 240),
                muted_foreground: Color::Rgb(100, 100, 110),
                accent: Color::Rgb(230, 230, 236),
                accent_foreground: Color::Rgb(15, 15, 20),
                destructive: Color::Rgb(200, 50, 50),
                border: Color::Rgb(210, 210, 216),
                input: Color::Rgb(245, 245, 248),
                ring: Color::Rgb(30, 30, 40),
                sidebar: Color::Rgb(248, 248, 250),
                sidebar_foreground: Color::Rgb(15, 15, 20),
                sidebar_primary: Color::Rgb(30, 30, 40),
                sidebar_primary_foreground: Color::Rgb(248, 248, 250),
                sidebar_accent: Color::Rgb(230, 230, 236),
                sidebar_accent_foreground: Color::Rgb(15, 15, 20),
                sidebar_border: Color::Rgb(210, 210, 216),
                sidebar_ring: Color::Rgb(30, 30, 40),
                chart_1: Color::Rgb(40, 160, 60),
                chart_2: Color::Rgb(50, 100, 200),
                chart_3: Color::Rgb(200, 160, 30),
                chart_4: Color::Rgb(160, 80, 200),

                // mew surfaces
                status_bg: Color::Rgb(240, 240, 244),
                tool_bg: Color::Rgb(245, 245, 248),
                divider: Color::Rgb(210, 210, 216),

                // Semantic scales — light mode: dark text, mid backgrounds.
                // fg = dark (readable on light bg), med = mid-saturation,
                // bg = light (pill background).
                red: ColorScale {
                    fg: Color::Rgb(140, 20, 20),
                    med: Color::Rgb(200, 80, 80),
                    bg: Color::Rgb(250, 220, 220),
                },
                green: ColorScale {
                    fg: Color::Rgb(20, 100, 30),
                    med: Color::Rgb(80, 160, 90),
                    bg: Color::Rgb(220, 245, 225),
                },
                yellow: ColorScale {
                    fg: Color::Rgb(120, 80, 0),
                    med: Color::Rgb(200, 160, 40),
                    bg: Color::Rgb(250, 240, 210),
                },
                blue: ColorScale {
                    fg: Color::Rgb(20, 50, 120),
                    med: Color::Rgb(80, 120, 200),
                    bg: Color::Rgb(220, 230, 250),
                },
                purple: ColorScale {
                    fg: Color::Rgb(80, 30, 120),
                    med: Color::Rgb(140, 90, 190),
                    bg: Color::Rgb(235, 225, 245),
                },
                cyan: ColorScale {
                    fg: Color::Rgb(0, 90, 90),
                    med: Color::Rgb(40, 140, 140),
                    bg: Color::Rgb(220, 245, 245),
                },
            },
        }
    }

    /// Load a theme by name. Searches:
    /// 1. `<cwd>/.mew/themes/<name>.json` (project-local)
    /// 2. `~/.config/mew/themes/<name>.json` (user-installed)
    /// 3. Built-in themes: `"dark"`, `"light"`, `"catppuccin-mocha"`,
    ///    `"catppuccin-latte"`, `"tokyo-night"`
    ///
    /// Falls back to `Theme::dark()` if the name is not found.
    pub fn load(name: &str) -> Self {
        // Built-in themes.
        match name {
            "dark" | "" => return Self::dark(),
            "light" => return Self::light(),
            "catppuccin-mocha" => {
                return Self::from_json_str(include_str!(
                    "../resources/themes/catppuccin-mocha.json"
                ))
                .unwrap_or_else(|_| Self::dark());
            }
            "catppuccin-latte" => {
                return Self::from_json_str(include_str!(
                    "../resources/themes/catppuccin-latte.json"
                ))
                .unwrap_or_else(|_| Self::dark());
            }
            "tokyo-night" => {
                return Self::from_json_str(include_str!("../resources/themes/tokyo-night.json"))
                    .unwrap_or_else(|_| Self::dark());
            }
            _ => {}
        }

        // Project-local themes.
        if let Ok(cwd) = std::env::current_dir() {
            let path = cwd.join(".mew").join("themes").join(format!("{name}.json"));
            if path.exists() {
                if let Ok(theme) = Self::from_json(&path) {
                    return theme;
                }
            }
        }

        // User themes.
        if let Some(config_dir) = Self::config_themes_dir() {
            let path = config_dir.join(format!("{name}.json"));
            if path.exists() {
                if let Ok(theme) = Self::from_json(&path) {
                    return theme;
                }
            }
        }

        tracing::warn!(theme = name, "theme not found; falling back to dark");
        Self::dark()
    }

    /// List available theme names: built-ins + any JSON files in the
    /// search paths.
    pub fn list_available() -> Vec<String> {
        let mut names = vec![
            "dark".into(),
            "light".into(),
            "catppuccin-mocha".into(),
            "catppuccin-latte".into(),
            "tokyo-night".into(),
        ];

        // Project-local.
        if let Ok(cwd) = std::env::current_dir() {
            let dir = cwd.join(".mew").join("themes");
            Self::collect_theme_names(&dir, &mut names);
        }

        // User themes.
        if let Some(config_dir) = Self::config_themes_dir() {
            Self::collect_theme_names(&config_dir, &mut names);
        }

        names.sort();
        names.dedup();
        names
    }

    /// The directory where user-installed themes are stored.
    /// `~/.config/mew/themes/` on Linux, `~/Library/Application
    /// Support/ai.mew.mew/themes/` on macOS.
    pub fn themes_dir() -> Option<std::path::PathBuf> {
        Self::config_themes_dir()
    }

    fn config_themes_dir() -> Option<std::path::PathBuf> {
        let base = if cfg!(target_os = "macos") {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join("Library/Application Support/ai.mew.mew"))
        } else {
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| std::path::PathBuf::from(h).join(".config"))
                })
                .map(|p| p.join("mew"))
        };
        base.map(|p| p.join("themes"))
    }

    fn collect_theme_names(dir: &std::path::Path, names: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Some(stem) = name.strip_suffix(".json") {
                        names.push(stem.to_string());
                    }
                }
            }
        }
    }

    /// Load a theme from a JSON file, merging over `dark()` defaults.
    /// Missing tokens inherit from the dark theme.
    pub fn from_json(path: &std::path::Path) -> Result<Self, ThemeError> {
        let content = std::fs::read_to_string(path).map_err(|e| ThemeError::Io(e.to_string()))?;
        Self::from_json_str(&content)
    }

    /// Parse a theme from a JSON string, merging over `dark()` defaults.
    pub fn from_json_str(content: &str) -> Result<Self, ThemeError> {
        let raw: ThemeFile =
            serde_json::from_str(content).map_err(|e| ThemeError::Parse(e.to_string()))?;

        let mode = raw.mode.unwrap_or_default();
        let base = Self::dark();
        let mut tokens = base.tokens;

        // Merge each provided token over the base.
        if let Some(t) = raw.tokens {
            macro_rules! merge {
                ($field:ident) => {
                    if let Some(v) = t.$field {
                        tokens.$field = parse_hex(&v).unwrap_or(tokens.$field);
                    }
                };
            }

            merge!(background);
            merge!(foreground);
            merge!(card);
            merge!(card_foreground);
            merge!(popover);
            merge!(popover_foreground);
            merge!(primary);
            merge!(primary_foreground);
            merge!(secondary);
            merge!(secondary_foreground);
            merge!(muted);
            merge!(muted_foreground);
            merge!(accent);
            merge!(accent_foreground);
            merge!(destructive);
            merge!(border);
            merge!(input);
            merge!(ring);
            merge!(sidebar);
            merge!(sidebar_foreground);
            merge!(sidebar_primary);
            merge!(sidebar_primary_foreground);
            merge!(sidebar_accent);
            merge!(sidebar_accent_foreground);
            merge!(sidebar_border);
            merge!(sidebar_ring);
            merge!(chart_1);
            merge!(chart_2);
            merge!(chart_3);
            merge!(chart_4);
            merge!(status_bg);
            merge!(tool_bg);
            merge!(divider);

            // Color scales
            merge_scale!(tokens, t, red);
            merge_scale!(tokens, t, green);
            merge_scale!(tokens, t, yellow);
            merge_scale!(tokens, t, blue);
            merge_scale!(tokens, t, purple);
            merge_scale!(tokens, t, cyan);
        }

        Ok(Self {
            name: raw.name.unwrap_or_else(|| "custom".into()),
            mode,
            tokens,
        })
    }
}

/// Merge a `ColorScale` from optional hex strings over the base.
macro_rules! merge_scale {
    ($tokens:expr, $t:expr, $field:ident) => {
        if let Some(s) = $t.$field {
            if let Some(fg) = s.fg.as_deref().and_then(parse_hex) {
                $tokens.$field.fg = fg;
            }
            if let Some(med) = s.med.as_deref().and_then(parse_hex) {
                $tokens.$field.med = med;
            }
            if let Some(bg) = s.bg.as_deref().and_then(parse_hex) {
                $tokens.$field.bg = bg;
            }
        }
    };
}

use merge_scale;

// ---------------------------------------------------------------------------
// JSON serde types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    mode: Option<ThemeMode>,
    tokens: Option<ThemeFileTokens>,
}

#[derive(Debug, Default, Deserialize)]
struct ThemeFileTokens {
    background: Option<String>,
    foreground: Option<String>,
    card: Option<String>,
    card_foreground: Option<String>,
    popover: Option<String>,
    popover_foreground: Option<String>,
    primary: Option<String>,
    primary_foreground: Option<String>,
    secondary: Option<String>,
    secondary_foreground: Option<String>,
    muted: Option<String>,
    muted_foreground: Option<String>,
    accent: Option<String>,
    accent_foreground: Option<String>,
    destructive: Option<String>,
    border: Option<String>,
    input: Option<String>,
    ring: Option<String>,
    sidebar: Option<String>,
    sidebar_foreground: Option<String>,
    sidebar_primary: Option<String>,
    sidebar_primary_foreground: Option<String>,
    sidebar_accent: Option<String>,
    sidebar_accent_foreground: Option<String>,
    sidebar_border: Option<String>,
    sidebar_ring: Option<String>,
    chart_1: Option<String>,
    chart_2: Option<String>,
    chart_3: Option<String>,
    chart_4: Option<String>,
    status_bg: Option<String>,
    tool_bg: Option<String>,
    divider: Option<String>,
    red: Option<ScaleFile>,
    green: Option<ScaleFile>,
    yellow: Option<ScaleFile>,
    blue: Option<ScaleFile>,
    purple: Option<ScaleFile>,
    cyan: Option<ScaleFile>,
}

#[derive(Debug, Default, Deserialize)]
struct ScaleFile {
    fg: Option<String>,
    med: Option<String>,
    bg: Option<String>,
}

/// Errors encountered while loading a theme.
#[derive(Debug)]
pub enum ThemeError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeError::Io(e) => write!(f, "theme io error: {e}"),
            ThemeError::Parse(e) => write!(f, "theme parse error: {e}"),
        }
    }
}

impl std::error::Error for ThemeError {}

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

    // --- Theme struct tests ---

    #[test]
    fn test_dark_theme_returns_current_values() {
        let t = Theme::dark();
        assert_eq!(t.name, "dark");
        assert_eq!(t.mode, ThemeMode::Dark);
        // These must match the old hardcoded constants.
        assert_eq!(t.tokens.status_bg, Color::Rgb(30, 30, 33));
        assert_eq!(t.tokens.sidebar, Color::Rgb(28, 28, 31));
        assert_eq!(t.tokens.tool_bg, Color::Rgb(50, 50, 56));
        assert_eq!(t.tokens.divider, Color::Rgb(50, 50, 55));
    }

    #[test]
    fn test_dark_theme_pill_colors() {
        let t = Theme::dark();
        // Model pill = green scale
        assert_eq!(t.tokens.green.fg, Color::Rgb(150, 230, 160));
        assert_eq!(t.tokens.green.bg, Color::Rgb(25, 70, 35));
        // Dangerous = red scale
        assert_eq!(t.tokens.red.bg, Color::Rgb(140, 30, 30));
        // Auto = purple scale
        assert_eq!(t.tokens.purple.med, Color::Rgb(95, 50, 130));
    }

    #[test]
    fn test_from_json_str_partial_theme() {
        let json = r##"{
            "name": "warm",
            "mode": "dark",
            "tokens": {
                "background": "#2a1f1a",
                "status_bg": "#1f1611"
            }
        }"##;
        let t = Theme::from_json_str(json).unwrap();
        assert_eq!(t.name, "warm");
        assert_eq!(t.mode, ThemeMode::Dark);
        assert_eq!(t.tokens.background, Color::Rgb(42, 31, 26));
        assert_eq!(t.tokens.status_bg, Color::Rgb(31, 22, 17));
        // Unspecified tokens inherit from dark.
        let dark = Theme::dark();
        assert_eq!(t.tokens.sidebar, dark.tokens.sidebar);
        assert_eq!(t.tokens.green.fg, dark.tokens.green.fg);
    }

    #[test]
    fn test_from_json_str_color_scale() {
        let json = r##"{
            "name": "custom",
            "tokens": {
                "red": {
                    "fg": "#ff0000",
                    "med": "#cc0000",
                    "bg": "#660000"
                }
            }
        }"##;
        let t = Theme::from_json_str(json).unwrap();
        assert_eq!(t.tokens.red.fg, Color::Rgb(255, 0, 0));
        assert_eq!(t.tokens.red.med, Color::Rgb(204, 0, 0));
        assert_eq!(t.tokens.red.bg, Color::Rgb(102, 0, 0));
        // Other scales unchanged.
        let dark = Theme::dark();
        assert_eq!(t.tokens.green.fg, dark.tokens.green.fg);
    }

    #[test]
    fn test_from_json_str_invalid_json_errors() {
        let result = Theme::from_json_str("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_from_json_str_invalid_hex_falls_back() {
        let json = r##"{
            "name": "badhex",
            "tokens": {
                "background": "not-a-color"
            }
        }"##;
        let t = Theme::from_json_str(json).unwrap();
        // Invalid hex → keeps dark default.
        let dark = Theme::dark();
        assert_eq!(t.tokens.background, dark.tokens.background);
    }

    #[test]
    fn test_from_json_str_minimal_theme() {
        // Just a name, no tokens at all.
        let json = r#"{"name": "empty"}"#;
        let t = Theme::from_json_str(json).unwrap();
        assert_eq!(t.name, "empty");
        let dark = Theme::dark();
        assert_eq!(t.tokens.background, dark.tokens.background);
        assert_eq!(t.tokens.divider, dark.tokens.divider);
    }

    #[test]
    fn test_light_theme_is_light() {
        let t = Theme::light();
        assert_eq!(t.name, "light");
        assert_eq!(t.mode, ThemeMode::Light);
        // Background should be light (high luminance).
        if let Color::Rgb(r, g, b) = t.tokens.background {
            let lum = 0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64);
            assert!(lum > 200.0, "light theme background should be bright");
        }
        // Foreground should be dark (low luminance).
        if let Color::Rgb(r, g, b) = t.tokens.foreground {
            let lum = 0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64);
            assert!(lum < 50.0, "light theme foreground should be dark");
        }
    }

    #[test]
    fn test_light_theme_pill_tiers_inverted() {
        let t = Theme::light();
        // In light mode, fg is dark (for text), bg is light (pill background).
        if let (Color::Rgb(fg_r, _, _), Color::Rgb(bg_r, _, _)) =
            (t.tokens.green.fg, t.tokens.green.bg)
        {
            assert!(
                fg_r < bg_r,
                "green.fg should be darker than green.bg in light mode"
            );
        }
    }

    #[test]
    fn test_load_builtin_dark() {
        let t = Theme::load("dark");
        assert_eq!(t.name, "dark");
        assert_eq!(t.mode, ThemeMode::Dark);
    }

    #[test]
    fn test_load_builtin_light() {
        let t = Theme::load("light");
        assert_eq!(t.name, "light");
        assert_eq!(t.mode, ThemeMode::Light);
    }

    #[test]
    fn test_load_unknown_falls_back_to_dark() {
        let t = Theme::load("nonexistent-theme-xyz");
        assert_eq!(t.name, "dark");
    }

    #[test]
    fn test_load_empty_name_returns_dark() {
        let t = Theme::load("");
        assert_eq!(t.name, "dark");
    }

    #[test]
    fn test_list_available_includes_builtins() {
        let names = Theme::list_available();
        assert!(names.contains(&"dark".to_string()));
        assert!(names.contains(&"light".to_string()));
        assert!(names.contains(&"catppuccin-mocha".to_string()));
        assert!(names.contains(&"catppuccin-latte".to_string()));
        assert!(names.contains(&"tokyo-night".to_string()));
    }

    #[test]
    fn test_load_catppuccin_mocha() {
        let t = Theme::load("catppuccin-mocha");
        assert_eq!(t.name, "catppuccin-mocha");
        assert_eq!(t.mode, ThemeMode::Dark);
        // Background should be the mocha base color.
        assert_eq!(t.tokens.background, Color::Rgb(30, 30, 46));
    }

    #[test]
    fn test_load_catppuccin_latte() {
        let t = Theme::load("catppuccin-latte");
        assert_eq!(t.name, "catppuccin-latte");
        assert_eq!(t.mode, ThemeMode::Light);
        // Latte background is light.
        if let Color::Rgb(r, g, b) = t.tokens.background {
            let lum = 0.299 * (r as f64) + 0.587 * (g as f64) + 0.114 * (b as f64);
            assert!(lum > 200.0, "latte background should be bright");
        }
    }

    #[test]
    fn test_load_tokyo_night() {
        let t = Theme::load("tokyo-night");
        assert_eq!(t.name, "tokyo-night");
        assert_eq!(t.mode, ThemeMode::Dark);
        assert_eq!(t.tokens.background, Color::Rgb(26, 27, 38));
    }
}
