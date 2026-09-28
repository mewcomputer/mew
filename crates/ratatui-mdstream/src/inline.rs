use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::Theme;

/// A styled text run produced by the inline parser.
pub type StyledRun = (String, Style);

/// Byte index of the first `marker` (a two-byte ASCII sequence) at or after
/// `from`, or `None` when it does not occur.
///
/// Both marker bytes are ASCII and UTF-8 continuation bytes are never ASCII,
/// so a returned index is always a char boundary and slicing there is safe.
/// This is why an unclosed marker must be handled by the caller rather than by
/// scanning to the end of the buffer: the final byte may sit inside a
/// multi-byte character.
fn find_marker(bytes: &[u8], from: usize, marker: &[u8; 2]) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == marker[0] && bytes[i + 1] == marker[1] {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse inline markdown syntax into styled runs.
///
/// Handles: `**bold**`, `*italic*`, `` `code` ``, `[text](url)`, `~~strike~~`.
pub fn parse_inline(text: &str, theme: &Theme) -> Vec<StyledRun> {
    let mut runs = Vec::new();
    let mut current = String::new();
    let current_style = Style::default();
    let _stack: Vec<Style> = Vec::new();

    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Check for inline code: `...`
        if bytes[i] == b'`' {
            if !current.is_empty() {
                runs.push((std::mem::take(&mut current), current_style));
            }
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'`' {
                end += 1;
            }
            let code = &text[start..end];
            runs.push((code.to_string(), theme.inline_code));
            i = end + 1;
            continue;
        }

        // Check for ~~strikethrough~~
        if i + 1 < bytes.len() && bytes[i] == b'~' && bytes[i + 1] == b'~' {
            if !current.is_empty() {
                runs.push((std::mem::take(&mut current), current_style));
            }
            let start = i + 2;
            match find_marker(bytes, start, b"~~") {
                Some(end) => {
                    runs.push((text[start..end].to_string(), theme.strikethrough));
                    i = end + 2;
                }
                // No closing marker (normal mid-stream): style the remainder.
                None => {
                    runs.push((text[start..].to_string(), theme.strikethrough));
                    i = bytes.len();
                }
            }
            continue;
        }

        // Check for **bold** or __bold__
        let is_bold_star = i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'*';
        let is_bold_underscore = i + 1 < bytes.len() && bytes[i] == b'_' && bytes[i + 1] == b'_';

        if is_bold_star || is_bold_underscore {
            if !current.is_empty() {
                runs.push((std::mem::take(&mut current), current_style));
            }
            let start = i + 2;
            let close = if is_bold_star { b"**" } else { b"__" };
            match find_marker(bytes, start, close) {
                Some(end) => {
                    runs.push((text[start..end].to_string(), theme.strong));
                    i = end + 2;
                }
                // No closing marker (normal mid-stream): style the remainder.
                None => {
                    runs.push((text[start..].to_string(), theme.strong));
                    i = bytes.len();
                }
            }
            continue;
        }

        // Check for *italic* or _italic_
        // But only if not already handled by ** check above (which requires 2 chars)
        let is_italic_star = bytes[i] == b'*';
        let is_italic_underscore = bytes[i] == b'_';

        if is_italic_star || is_italic_underscore {
            if !current.is_empty() {
                runs.push((std::mem::take(&mut current), current_style));
            }
            let start = i + 1;
            let mut end = start;
            let close_byte = if is_italic_star { b'*' } else { b'_' };
            while end < bytes.len() && bytes[end] != close_byte {
                end += 1;
            }
            let italic_text = &text[start..end];
            runs.push((italic_text.to_string(), theme.emphasis));
            i = end + 1;
            continue;
        }

        // Check for [text](url) or [text][ref]
        if bytes[i] == b'[' {
            if !current.is_empty() {
                runs.push((std::mem::take(&mut current), current_style));
            }
            let mut bracket_end = i + 1;
            while bracket_end < bytes.len() && bytes[bracket_end] != b']' {
                bracket_end += 1;
            }
            if bracket_end < bytes.len() {
                let link_text = &text[i + 1..bracket_end];
                let mut url = None;
                let next = bracket_end + 1;
                // Check for (url)
                if next < bytes.len() && bytes[next] == b'(' {
                    let mut paren_end = next + 1;
                    while paren_end < bytes.len() && bytes[paren_end] != b')' {
                        paren_end += 1;
                    }
                    if paren_end < bytes.len() {
                        url = Some(&text[next + 1..paren_end]);
                        bracket_end = paren_end;
                    }
                }
                // Check for [ref]
                else if next < bytes.len() && bytes[next] == b'[' {
                    let mut ref_end = next + 1;
                    while ref_end < bytes.len() && bytes[ref_end] != b']' {
                        ref_end += 1;
                    }
                    if ref_end < bytes.len() {
                        url = Some(&text[next + 1..ref_end]);
                        bracket_end = ref_end;
                    }
                }

                runs.push((link_text.to_string(), theme.link_text));
                if let Some(u) = url {
                    runs.push((format!(" ({u})"), theme.link_url));
                }
                i = bracket_end + 1;
                continue;
            }
        }

        // Use grapheme iteration for the fallback case to handle
        // multi-byte characters (emoji, CJK, etc.) correctly.
        if let Some(grapheme) = text[i..].graphemes(true).next() {
            current.push_str(grapheme);
            i += grapheme.len(); // Move past the grapheme
        } else {
            break;
        }
    }

    if !current.is_empty() {
        runs.push((current, current_style));
    }

    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        use ratatui::style::{Color, Modifier, Style};
        Theme {
            paragraph: Style::default().fg(Color::White),
            heading: [
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ],
            emphasis: Style::default().add_modifier(Modifier::ITALIC),
            strong: Style::default().add_modifier(Modifier::BOLD),
            strikethrough: Style::default().add_modifier(Modifier::CROSSED_OUT),
            inline_code: Style::default().bg(Color::Rgb(40, 40, 45)),
            link_text: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),
            link_url: Style::default().fg(Color::DarkGray),
            list_bullet: Style::default().fg(Color::White),
            block_quote: Style::default().fg(Color::Gray),
            thematic_break: Style::default().fg(Color::DarkGray),
            table_header: Style::default().add_modifier(Modifier::BOLD),
            table_cell: Style::default(),
            table_border: Style::default().fg(Color::DarkGray),
            code_fence_default: Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 35)),
            code_fence_border: Style::default().fg(Color::DarkGray),
            pending_indicator: Style::default().fg(Color::Yellow),
        }
    }

    #[test]
    fn test_plain_text() {
        let theme = theme();
        let runs = parse_inline("hello world", &theme);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "hello world");
    }

    #[test]
    fn test_bold() {
        let theme = theme();
        let runs = parse_inline("hello **bold** world", &theme);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].0, "hello ");
        assert_eq!(runs[1].0, "bold");
        assert_eq!(runs[1].1, theme.strong);
        assert_eq!(runs[2].0, " world");
    }

    #[test]
    fn test_italic() {
        let theme = theme();
        let runs = parse_inline("hello *italic* world", &theme);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].0, "italic");
        assert_eq!(runs[1].1, theme.emphasis);
    }

    #[test]
    fn test_code() {
        let theme = theme();
        let runs = parse_inline("hello `code` world", &theme);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].0, "code");
        assert_eq!(runs[1].1, theme.inline_code);
    }

    #[test]
    fn test_strikethrough() {
        let theme = theme();
        let runs = parse_inline("hello ~~strike~~ world", &theme);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].0, "strike");
        assert_eq!(runs[1].1, theme.strikethrough);
    }

    #[test]
    fn test_link() {
        let theme = theme();
        let runs = parse_inline("click [here](http://x.com)", &theme);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].0, "click ");
        assert_eq!(runs[1].0, "here");
        assert_eq!(runs[1].1, theme.link_text);
        assert_eq!(runs[2].0, " (http://x.com)");
    }

    #[test]
    fn test_bold_unclosed_keeps_final_char() {
        // An unclosed marker is normal while streaming. The scan used to stop
        // one byte short of the end, silently dropping the last character.
        let theme = theme();
        let runs = parse_inline("**bold", &theme);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "bold");
        assert_eq!(runs[0].1, theme.strong);
    }

    #[test]
    fn test_bold_unclosed_multibyte_tail_does_not_panic() {
        // Regression: the stopping index landed inside the trailing multi-byte
        // character, panicking on `&text[start..end]` with
        // "byte index N is not a char boundary".
        let theme = theme();
        let runs = parse_inline("**bold \u{2014}", &theme);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "bold \u{2014}");
        assert_eq!(runs[0].1, theme.strong);

        // Same for CJK and emoji tails.
        for text in ["**\u{4f60}\u{597d}", "**\u{1f600}"] {
            let runs = parse_inline(text, &theme);
            assert_eq!(runs[0].0, &text[2..]);
        }
    }

    #[test]
    fn test_strikethrough_unclosed_multibyte_tail_does_not_panic() {
        let theme = theme();
        let runs = parse_inline("~~strike \u{2014}", &theme);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "strike \u{2014}");
        assert_eq!(runs[0].1, theme.strikethrough);
    }
}
