//! Theme system: token-based colors for the TUI.
//!
//! Based on the shadcn semantic token model, extended with a flat,
//! aliased token table. Themes are sparse overrides on top of a shared
//! manifest. See `docs/THEMING.md` for the full design.
//!
//! The [`Theme`] struct holds every color the TUI uses. [`Theme::dark`]
//! returns the default dark theme loaded from `theme_manifest.json`.
//! [`Theme::from_json`] loads a theme file, merging over the manifest
//! defaults for partial themes.

use ratatui::style::Color;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

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
// Theme
// ---------------------------------------------------------------------------

/// A complete theme for the TUI.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub mode: ThemeMode,
    table: HashMap<String, Color>,
}

/// Light or dark mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// The default dark theme.
    pub fn dark() -> Self {
        Self::from_manifest("dark").unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to load dark theme from manifest");
            Self::fallback_dark()
        })
    }

    /// The default light theme.
    pub fn light() -> Self {
        Self::from_manifest("light").unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to load light theme from manifest");
            Self::fallback_dark()
        })
    }

    /// Load a theme from the embedded manifest.
    fn from_manifest(name: &str) -> Result<Self, ThemeError> {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../resources/theme_manifest.json"))?;
        let theme_def = manifest
            .themes
            .get(name)
            .ok_or_else(|| ThemeError::UnknownTheme(name.to_string()))?;

        let mut merged = manifest.tokens.clone();
        // Apply the referenced base theme first (if any) so sparse overrides
        // only need to declare the tokens that differ from the base.
        if let Some(ref base_name) = theme_def.base {
            if base_name != name {
                if let Some(base_def) = manifest.themes.get(base_name) {
                    for (k, v) in &base_def.tokens {
                        merged.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        for (k, v) in &theme_def.tokens {
            merged.insert(k.clone(), v.clone());
        }
        let table = resolve_table(&merged)?;

        Ok(Self {
            name: name.to_string(),
            mode: theme_def.mode,
            table,
        })
    }

    /// A hardcoded fallback used when the manifest cannot be parsed.
    fn fallback_dark() -> Self {
        let mut table = HashMap::new();
        for (k, v) in [
            ("background", Color::Rgb(30, 30, 33)),
            ("foreground", Color::White),
            ("text.body", Color::White),
            ("card", Color::Rgb(50, 50, 56)),
            ("primary", Color::Cyan),
        ] {
            table.insert(k.to_string(), v);
        }
        Self {
            name: "dark".to_string(),
            mode: ThemeMode::Dark,
            table,
        }
    }

    /// Resolve a token to a color. Unknown tokens fall back to a sensible
    /// default (background for surface tokens, foreground for text tokens).
    pub fn resolve(&self, key: &str) -> Color {
        self.get(key)
            .unwrap_or_else(|| fallback_color(key, &self.table))
    }

    /// Look up a token without falling back.
    pub fn get(&self, key: &str) -> Option<Color> {
        self.table.get(key).copied()
    }

    /// Resolve an ANSI color through the token table.
    pub fn ansi(&self, color: ratatui::style::Color) -> Color {
        // Map ANSI colors to manifest tokens.
        let key = match color {
            Color::Black => "terminal.black",
            Color::Red => "terminal.red",
            Color::Green => "terminal.green",
            Color::Yellow => "terminal.yellow",
            Color::Blue => "terminal.blue",
            Color::Magenta => "terminal.magenta",
            Color::Cyan => "terminal.cyan",
            Color::Gray => "terminal.white",
            Color::DarkGray => "terminal.bright_black",
            Color::LightRed => "terminal.bright_red",
            Color::LightGreen => "terminal.bright_green",
            Color::LightYellow => "terminal.bright_yellow",
            Color::LightBlue => "terminal.bright_blue",
            Color::LightMagenta => "terminal.bright_magenta",
            Color::LightCyan => "terminal.bright_cyan",
            Color::White => "terminal.bright_white",
            other => return other,
        };
        self.resolve(key)
    }

    /// Return a clone of this theme with persona accent tokens injected.
    pub fn with_persona_accent(&self, name: &str, explicit_color: Option<&str>) -> Self {
        let accent = persona_accent(name, explicit_color);
        let mut clone = self.clone();
        clone
            .table
            .insert("persona.accent.fg".to_string(), accent.fg);
        clone
            .table
            .insert("persona.accent.bg".to_string(), accent.bg);
        clone
    }

    /// Convert this theme to the markdown renderer's theme.
    pub fn md_theme(&self) -> ratatui_mdstream::Theme {
        use ratatui::style::{Modifier, Style};
        let resolve = |key: &str| self.resolve(key);
        let style = |key: &str| Style::default().fg(resolve(key));
        let mut heading = [Style::default(); 6];
        for (i, h) in heading.iter_mut().enumerate() {
            *h = Style::default()
                .fg(resolve(&format!("markdown.heading.h{}", i + 1)))
                .add_modifier(Modifier::BOLD);
        }
        ratatui_mdstream::Theme {
            paragraph: style("markdown.paragraph"),
            heading,
            emphasis: style("markdown.emphasis").add_modifier(Modifier::ITALIC),
            strong: style("markdown.strong").add_modifier(Modifier::BOLD),
            strikethrough: style("markdown.strikethrough").add_modifier(Modifier::CROSSED_OUT),
            inline_code: Style::default()
                .fg(resolve("markdown.inline_code.fg"))
                .bg(resolve("markdown.inline_code.bg")),
            link_text: style("markdown.link_text").add_modifier(Modifier::UNDERLINED),
            link_url: style("markdown.link_url"),
            list_bullet: style("markdown.list_bullet"),
            block_quote: style("markdown.block_quote"),
            thematic_break: style("markdown.thematic_break"),
            table_header: Style::default()
                .fg(resolve("markdown.table_header"))
                .add_modifier(Modifier::BOLD),
            table_cell: style("markdown.table_cell"),
            table_border: style("markdown.table_border"),
            code_fence_default: Style::default()
                .fg(resolve("markdown.code_fence.fg"))
                .bg(resolve("markdown.code_fence.bg")),
            code_fence_border: style("markdown.code_fence.border"),
            pending_indicator: style("markdown.pending_indicator"),
        }
    }

    /// Load a theme by name. Searches:
    /// 1. The manifest (built-in themes)
    /// 2. `<cwd>/.mew/themes/<name>.json` (project-local)
    /// 3. `~/.config/mew/themes/<name>.json` (user-installed)
    ///
    /// Falls back to `Theme::dark()` if the name is not found.
    pub fn load(name: &str) -> Self {
        // Built-in themes.
        match Self::from_manifest(name) {
            Ok(theme) => return theme,
            Err(ThemeError::UnknownTheme(_)) => {}
            Err(e) => tracing::warn!(error = %e, theme = name, "failed to load built-in theme"),
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
        let manifest: Manifest =
            serde_json::from_str(include_str!("../resources/theme_manifest.json"))
                .expect("embedded manifest should be valid");
        let mut names: Vec<String> = manifest.themes.keys().cloned().collect();

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
    pub fn themes_dir() -> Option<std::path::PathBuf> {
        Self::config_themes_dir()
    }

    fn config_themes_dir() -> Option<std::path::PathBuf> {
        // Themes are config — they live in the config directory.
        Some(mew_config::config_dir().join("themes"))
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

    /// Load a theme from a JSON file, merging over manifest defaults.
    pub fn from_json(path: &std::path::Path) -> Result<Self, ThemeError> {
        let content = std::fs::read_to_string(path).map_err(|e| ThemeError::Io(e.to_string()))?;
        Self::from_json_str(&content)
    }

    /// Parse a theme from a JSON string, merging over manifest defaults.
    /// Unknown tokens in the user file are rejected.
    pub fn from_json_str(content: &str) -> Result<Self, ThemeError> {
        let raw: ThemeFile =
            serde_json::from_str(content).map_err(|e| ThemeError::Parse(e.to_string()))?;

        let manifest: Manifest =
            serde_json::from_str(include_str!("../resources/theme_manifest.json"))?;

        if let Some(ref tokens) = raw.tokens {
            for key in tokens.keys() {
                if key.starts_with('_') {
                    continue;
                }
                if !manifest.tokens.contains_key(key) {
                    return Err(ThemeError::UnknownToken(format!(
                        "unknown token '{}' in custom theme; see docs/THEMING.md",
                        key
                    )));
                }
            }
        }

        let mut merged = manifest.tokens.clone();
        if let Some(t) = raw.tokens {
            for (k, v) in t {
                merged.insert(k, v);
            }
        }
        let table = resolve_table(&merged)?;

        Ok(Self {
            name: raw.name.unwrap_or_else(|| "custom".into()),
            mode: raw.mode.unwrap_or_default(),
            table,
        })
    }

    /// Return a CSS variable block for this theme, suitable for `export-css`.
    pub fn css_variables(&self) -> String {
        let mut keys: Vec<&String> = self.table.keys().collect();
        keys.sort();
        let mut out = String::new();
        for key in keys {
            if key.starts_with('_') {
                continue;
            }
            let var = format!("--{}", key.replace(['.', '_'], "-"));
            let value = color_to_hex(self.table[key]);
            out.push_str(&format!("  {var}: {value};\n"));
        }
        out
    }
}

fn color_to_hex(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => format!("{other:?}"),
    }
}

fn resolve_table(raw: &HashMap<String, String>) -> Result<HashMap<String, Color>, ThemeError> {
    validate_no_cycles(raw)?;
    let mut resolved = HashMap::with_capacity(raw.len());
    for key in raw.keys() {
        // CSS-only tokens (e.g. radius) are not colors; skip them.
        if key.starts_with('_') {
            continue;
        }
        let color = resolve_key(raw, key, &mut HashSet::new(), 0)?;
        resolved.insert(key.clone(), color);
    }
    Ok(resolved)
}

fn resolve_key(
    raw: &HashMap<String, String>,
    key: &str,
    stack: &mut HashSet<String>,
    depth: usize,
) -> Result<Color, ThemeError> {
    const MAX_DEPTH: usize = 32;
    if depth > MAX_DEPTH {
        return Err(ThemeError::AliasCycle(key.to_string()));
    }
    let value = raw
        .get(key)
        .ok_or_else(|| ThemeError::UnknownToken(key.to_string()))?;
    if let Some(alias) = value.strip_prefix('@') {
        if !stack.insert(alias.to_string()) {
            return Err(ThemeError::AliasCycle(alias.to_string()));
        }
        let color = resolve_key(raw, alias, stack, depth + 1)?;
        stack.remove(alias);
        Ok(color)
    } else {
        parse_hex(value).ok_or_else(|| ThemeError::InvalidColor(value.to_string()))
    }
}

fn validate_no_cycles(raw: &HashMap<String, String>) -> Result<(), ThemeError> {
    // For every token that is an alias, verify the chain terminates. This
    // catches cycles like a -> @b, b -> @a that would otherwise only surface
    // when the specific token is resolved.
    for key in raw.keys() {
        // Skip CSS-only metadata tokens.
        if key.starts_with('_') {
            continue;
        }
        let mut seen = HashSet::new();
        let mut cursor = key.as_str();
        loop {
            let value = raw.get(cursor).ok_or_else(|| {
                // Only report unknown tokens for the starting key; aliases
                // that point to a missing token are handled at resolve time.
                if cursor == key {
                    ThemeError::UnknownToken(cursor.to_string())
                } else {
                    ThemeError::AliasCycle(cursor.to_string())
                }
            })?;
            if !value.starts_with('@') {
                break;
            }
            let next = &value[1..];
            if !seen.insert(next.to_string()) {
                return Err(ThemeError::AliasCycle(next.to_string()));
            }
            cursor = next;
        }
    }
    Ok(())
}

fn fallback_color(key: &str, table: &HashMap<String, Color>) -> Color {
    if key.starts_with("text.") || key.starts_with("markdown.") || key.starts_with("syntax.") {
        table.get("foreground").copied().unwrap_or(Color::White)
    } else if key.starts_with("terminal.") {
        // If we somehow don't know a terminal color, return a mid-gray.
        Color::Gray
    } else {
        table.get("background").copied().unwrap_or(Color::Black)
    }
}

// ---------------------------------------------------------------------------
// Manifest serde types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Manifest {
    #[allow(dead_code)]
    version: u32,
    tokens: HashMap<String, String>,
    themes: HashMap<String, ThemeDef>,
}

#[derive(Debug, Deserialize)]
struct ThemeDef {
    mode: ThemeMode,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    tokens: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    mode: Option<ThemeMode>,
    tokens: Option<HashMap<String, String>>,
}

/// Errors encountered while loading a theme.
#[derive(Debug)]
pub enum ThemeError {
    Io(String),
    Parse(String),
    UnknownTheme(String),
    UnknownToken(String),
    AliasCycle(String),
    InvalidColor(String),
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeError::Io(e) => write!(f, "theme io error: {e}"),
            ThemeError::Parse(e) => write!(f, "theme parse error: {e}"),
            ThemeError::UnknownTheme(e) => write!(f, "unknown theme: {e}"),
            ThemeError::UnknownToken(e) => write!(f, "unknown token: {e}"),
            ThemeError::AliasCycle(e) => write!(f, "alias cycle: {e}"),
            ThemeError::InvalidColor(e) => write!(f, "invalid color: {e}"),
        }
    }
}

impl std::error::Error for ThemeError {}

impl From<serde_json::Error> for ThemeError {
    fn from(e: serde_json::Error) -> Self {
        ThemeError::Parse(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn fnv1a(data: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for &b in data {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

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
        if let Color::Rgb(r, _, _) = a.fg {
            assert!(r > 200, "fg should be light on dark bg");
        }
    }

    #[test]
    fn test_accent_from_hex_light_bg() {
        let a = accent_from_hex("#f0e68c").unwrap();
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

    // --- Theme tests ---

    #[test]
    fn test_dark_theme_loads() {
        let t = Theme::dark();
        assert_eq!(t.name, "dark");
        assert_eq!(t.mode, ThemeMode::Dark);
        assert_eq!(t.resolve("background"), Color::Rgb(30, 30, 33));
    }

    #[test]
    fn test_light_theme_loads() {
        let t = Theme::light();
        assert_eq!(t.name, "light");
        assert_eq!(t.mode, ThemeMode::Light);
        assert_eq!(t.resolve("background"), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn test_alias_resolution() {
        let t = Theme::dark();
        assert_eq!(t.resolve("text.body"), t.resolve("foreground"));
        assert_eq!(t.resolve("panel.background"), t.resolve("background"));
    }

    #[test]
    fn test_unknown_token_returns_fallback() {
        let t = Theme::dark();
        // Known tokens resolve to manifest values.
        let fg = t.resolve("foreground");
        assert_eq!(fg, Color::Rgb(255, 255, 255), "foreground should be white");
        let bg = t.resolve("background");
        assert_eq!(bg, Color::Rgb(30, 30, 33), "background should be dark gray");
        // Tokens under the semantic namespaces fall back to the matching text/surface color.
        assert_eq!(t.resolve("text.unknown"), fg, "text.* uses foreground");
        assert_eq!(
            t.resolve("markdown.unknown"),
            fg,
            "markdown.* uses foreground"
        );
        assert_eq!(t.resolve("syntax.unknown"), fg, "syntax.* uses foreground");
        assert_eq!(
            t.resolve("unknown.surface.token"),
            bg,
            "other tokens use background"
        );
    }

    #[test]
    fn test_custom_override_visible() {
        let json = r##"{
            "name": "warm",
            "tokens": {
                "panel.overlay_hover": "#2a1f1a"
            }
        }"##;
        let t = Theme::from_json_str(json).unwrap();
        assert_eq!(t.resolve("panel.overlay_hover"), Color::Rgb(42, 31, 26));
        // Unspecified tokens still resolve.
        assert_eq!(
            t.resolve("panel.background"),
            Theme::dark().resolve("background")
        );
    }

    #[test]
    fn test_with_persona_accent_injects_tokens() {
        let t = Theme::dark().with_persona_accent("planner", None);
        assert!(t.get("persona.accent.fg").is_some());
        assert!(t.get("persona.accent.bg").is_some());
    }

    #[test]
    fn test_load_builtin_tokyo_night() {
        let t = Theme::load("tokyo-night");
        assert_eq!(t.name, "tokyo-night");
        assert_eq!(t.mode, ThemeMode::Dark);
    }

    #[test]
    fn test_load_builtin_catppuccin_mocha() {
        let t = Theme::load("catppuccin-mocha");
        assert_eq!(t.name, "catppuccin-mocha");
        assert_eq!(t.mode, ThemeMode::Dark);
    }

    #[test]
    fn test_load_builtin_catppuccin_latte() {
        let t = Theme::load("catppuccin-latte");
        assert_eq!(t.name, "catppuccin-latte");
        assert_eq!(t.mode, ThemeMode::Light);
    }

    #[test]
    fn test_load_unknown_falls_back_to_dark() {
        let t = Theme::load("nonexistent-theme-xyz");
        assert_eq!(t.name, "dark");
    }

    #[test]
    fn test_list_available_includes_builtins() {
        let names = Theme::list_available();
        assert!(names.contains(&"dark".to_string()));
        assert!(names.contains(&"light".to_string()));
        assert!(names.contains(&"catppuccin-mocha".to_string()));
        assert!(names.contains(&"tokyo-night".to_string()));
        assert!(names.contains(&"dracula".to_string()));
    }

    #[test]
    fn test_cycle_detection() {
        let json = r##"{
            "name": "bad",
            "tokens": {
                "text.body": "@text.muted",
                "text.muted": "@text.body"
            }
        }"##;
        let err = Theme::from_json_str(json).unwrap_err();
        assert!(matches!(err, ThemeError::AliasCycle(_)));
    }

    #[test]
    fn test_invalid_color_errors() {
        let json = r##"{"name": "bad", "tokens": {"background": "not-a-color"}}"##;
        let err = Theme::from_json_str(json).unwrap_err();
        assert!(matches!(err, ThemeError::InvalidColor(_)));
    }
}
