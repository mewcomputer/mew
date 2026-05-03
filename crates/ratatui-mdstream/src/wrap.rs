use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::inline::StyledRun;

/// Wrap styled runs to the given width, preserving style across line breaks.
///
/// Breaks prefer word boundaries (whitespace), falling back to grapheme boundaries
/// only when a single word exceeds the width.
pub fn wrap_styled(runs: &[StyledRun], width: u16) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1) as usize;
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut current_width: usize = 0;

    for (text, style) in runs {
        let words: Vec<&str> = text.split_inclusive(' ').collect();

        for word in words {
            let word_width = word.width();

            // If the word fits on the current line, append it.
            if current_width + word_width <= width {
                current_line.push(Span::styled(word.to_string(), *style));
                current_width += word_width;
                continue;
            }

            // Word doesn't fit. If current line has content, flush it first.
            if !current_line.is_empty() {
                lines.push(std::mem::take(&mut current_line));
                current_width = 0;
            }

            // If the word itself fits on an empty line, place it whole.
            if word_width <= width {
                current_line.push(Span::styled(word.to_string(), *style));
                current_width = word_width;
                continue;
            }

            // Word is wider than the line: break at grapheme boundaries.
            let mut remaining = word;
            while !remaining.is_empty() {
                let mut chunk = String::new();
                let mut chunk_width: usize = 0;

                for grapheme in remaining.graphemes(true) {
                    let g_width = grapheme.width();
                    if chunk_width + g_width > width {
                        break;
                    }
                    chunk.push_str(grapheme);
                    chunk_width += g_width;
                }

                if chunk.is_empty() {
                    // Single grapheme wider than width — take it anyway.
                    let g = remaining.graphemes(true).next().unwrap_or("");
                    chunk = g.to_string();
                }

                let chunk_len = chunk.len();
                current_line.push(Span::styled(chunk, *style));
                lines.push(std::mem::take(&mut current_line));

                remaining = &remaining[chunk_len..];
            }
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // Ensure at least one empty line if input was empty.
    if lines.is_empty() {
        lines.push(Vec::new());
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn run(text: &str, style: Style) -> StyledRun {
        (text.to_string(), style)
    }

    fn line_text(line: &[Span]) -> String {
        line.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn test_no_wrap() {
        let runs = vec![run("hello world", Style::default())];
        let lines = wrap_styled(&runs, 20);
        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), "hello world");
    }

    #[test]
    fn test_simple_wrap() {
        let runs = vec![run("hello world foo", Style::default())];
        let lines = wrap_styled(&runs, 12);
        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "hello world ");
        assert_eq!(line_text(&lines[1]), "foo");
    }

    #[test]
    fn test_long_word_split() {
        let runs = vec![run("supercalifragilistic", Style::default())];
        let lines = wrap_styled(&runs, 10);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn test_style_preserved() {
        let style = Style::default().fg(ratatui::style::Color::Red);
        let runs = vec![run("hello world foo", style)];
        let lines = wrap_styled(&runs, 12);
        assert_eq!(lines[0][0].style, style);
        assert_eq!(lines[1][0].style, style);
    }
}
