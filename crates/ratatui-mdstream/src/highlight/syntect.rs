use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use syntect::{
    highlighting::{
        HighlightIterator, HighlightState, Highlighter as SyntectHighlighterInner, Theme,
    },
    parsing::{ParseState, ScopeStack, SyntaxDefinition, SyntaxSet},
};

use crate::highlight::{Highlighter, StyledRun};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(|| {
        // Start from two-face's bundled syntaxes, then merge in any custom
        // definitions that two-face does not ship (e.g. Gleam).
        let mut builder = two_face::syntax::extra_newlines().into_builder();
        let gleam_syntax = include_str!("../../resources/gleam.sublime-syntax");
        if let Ok(def) = SyntaxDefinition::load_from_str(gleam_syntax, false, None) {
            builder.add(def);
        }
        builder.build()
    })
}

/// Syntax highlighter backed by syntect.
pub struct SyntectHighlighter {
    parse_state: Option<ParseState>,
    highlight_state: Option<HighlightState>,
    theme: Theme,
}

impl SyntectHighlighter {
    pub fn new(theme: Theme) -> Self {
        Self {
            parse_state: None,
            highlight_state: None,
            theme,
        }
    }
}

impl Default for SyntectHighlighter {
    fn default() -> Self {
        Self::new(default_theme())
    }
}

fn default_theme() -> Theme {
    let bytes = include_bytes!("../../resources/theme.tmTheme");
    let mut cursor = std::io::Cursor::new(bytes.as_slice());
    syntect::highlighting::ThemeSet::load_from_reader(&mut cursor)
        .expect("embedded theme.tmTheme should be valid")
}

impl Highlighter for SyntectHighlighter {
    fn begin_block(&mut self, lang: Option<&str>) {
        let ss = syntax_set();
        let syntax = lang
            .and_then(|l| ss.find_syntax_by_token(l))
            .unwrap_or_else(|| ss.find_syntax_plain_text());

        self.parse_state = Some(ParseState::new(syntax));

        let highlighter = SyntectHighlighterInner::new(&self.theme);
        self.highlight_state = Some(HighlightState::new(&highlighter, ScopeStack::new()));
    }

    fn end_block(&mut self) {
        self.parse_state = None;
        self.highlight_state = None;
    }

    fn highlight_line(&mut self, _lang: Option<&str>, line: &str) -> Vec<StyledRun> {
        let Some(parse_state) = self.parse_state.as_mut() else {
            return vec![(line.to_string(), Style::default())];
        };
        let Some(highlight_state) = self.highlight_state.as_mut() else {
            return vec![(line.to_string(), Style::default())];
        };

        let ss = syntax_set();
        let ops = match parse_state.parse_line(line, ss) {
            Ok(ops) => ops,
            Err(_) => return vec![(line.to_string(), Style::default())],
        };

        let highlighter = SyntectHighlighterInner::new(&self.theme);

        let iter = HighlightIterator::new(highlight_state, &ops[..], line, &highlighter);
        iter.map(|(style, text)| {
            let mut ratatui_style = Style::default();

            // Foreground only — code fence background is handled by the caller.
            let fg = style.foreground;
            if fg.a > 0 {
                ratatui_style = ratatui_style.fg(Color::Rgb(fg.r, fg.g, fg.b));
            }

            // Font style (bold, italic, underline)
            let font_style = style.font_style;
            if font_style.contains(syntect::highlighting::FontStyle::BOLD) {
                ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
            }
            if font_style.contains(syntect::highlighting::FontStyle::ITALIC) {
                ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
            }
            if font_style.contains(syntect::highlighting::FontStyle::UNDERLINE) {
                ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
            }

            (text.to_string(), ratatui_style)
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wgsl_syntax_available() {
        let ss = syntax_set();
        let syntax = ss.find_syntax_by_token("wgsl");
        assert!(
            syntax.is_some(),
            "wgsl syntax should be available from two-face"
        );
    }

    #[test]
    fn test_gdscript_syntax_available() {
        let ss = syntax_set();
        let syntax = ss
            .find_syntax_by_token("gdscript")
            .or_else(|| ss.find_syntax_by_token("gd"))
            .or_else(|| ss.find_syntax_by_extension("gd"));
        assert!(
            syntax.is_some(),
            "gdscript syntax should be available from two-face (tried: gdscript, gd, .gd)"
        );
    }

    #[test]
    fn test_zig_syntax_available() {
        let ss = syntax_set();
        let syntax = ss.find_syntax_by_token("zig");
        assert!(
            syntax.is_some(),
            "zig syntax should be available from two-face"
        );
    }

    #[test]
    fn test_wgsl_highlighting_produces_styles() {
        let mut highlighter = SyntectHighlighter::default();
        highlighter.begin_block(Some("wgsl"));

        let line = "@vertex\n";
        let runs = highlighter.highlight_line(None, line);

        assert!(!runs.is_empty(), "should produce runs for wgsl");

        let has_styled = runs.iter().any(|(_, style)| {
            style.fg.is_some() || style.add_modifier != ratatui::style::Modifier::empty()
        });
        assert!(has_styled, "wgsl @vertex should be syntax-highlighted");
    }

    #[test]
    fn test_gleam_syntax_available() {
        let ss = syntax_set();
        let syntax = ss
            .find_syntax_by_token("gleam")
            .or_else(|| ss.find_syntax_by_extension("gleam"));
        assert!(
            syntax.is_some(),
            "gleam syntax should be available (custom sublime-syntax)"
        );
    }

    #[test]
    fn test_gleam_highlighting_produces_styles() {
        let mut highlighter = SyntectHighlighter::default();
        highlighter.begin_block(Some("gleam"));

        let line = "pub fn main() {\n";
        let runs = highlighter.highlight_line(None, line);

        assert!(!runs.is_empty(), "should produce runs for gleam");

        let has_styled = runs.iter().any(|(_, style)| {
            style.fg.is_some() || style.add_modifier != ratatui::style::Modifier::empty()
        });
        assert!(has_styled, "gleam `pub fn` should be syntax-highlighted");
    }
}
