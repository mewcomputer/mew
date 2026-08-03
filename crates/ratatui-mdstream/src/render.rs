use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::{
    highlight::Highlighter, inline::parse_inline, pending::PendingPolicy, theme::Theme,
    wrap::wrap_styled,
};

/// Renders a committed `mdstream::Block` into styled ratatui lines.
pub fn render_block(
    block: &mdstream::Block,
    width: u16,
    theme: &Theme,
    highlighter: &mut dyn Highlighter,
) -> Vec<Line<'static>> {
    let text = block.display_or_raw();

    match block.kind {
        mdstream::BlockKind::Heading => render_heading(text, width, theme),
        mdstream::BlockKind::Paragraph => render_paragraph(text, width, theme),
        mdstream::BlockKind::List => render_list(text, width, theme),
        mdstream::BlockKind::BlockQuote => render_block_quote(text, width, theme),
        mdstream::BlockKind::CodeFence => render_code_fence(text, width, theme, highlighter),
        mdstream::BlockKind::ThematicBreak => render_thematic_break(width, theme),
        mdstream::BlockKind::Table => render_table(text, width, theme),
        mdstream::BlockKind::HtmlBlock => render_raw(text, width, theme, "[html]"),
        mdstream::BlockKind::MathBlock => render_raw(text, width, theme, "[math]"),
        mdstream::BlockKind::FootnoteDefinition => render_paragraph(text, width, theme),
        mdstream::BlockKind::Unknown => render_paragraph(text, width, theme),
    }
}

/// Renders a pending block with the given policy.
pub fn render_pending(
    block: &mdstream::PendingBlockRef<'_>,
    width: u16,
    theme: &Theme,
    highlighter: &mut dyn Highlighter,
    policy: &dyn PendingPolicy,
) -> Vec<Line<'static>> {
    policy.render(
        block.kind,
        block.display_or_raw(),
        width,
        theme,
        highlighter,
    )
}

fn render_heading(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let mut level = 0usize;
    let trimmed = text.trim_start();
    for ch in trimmed.chars() {
        if ch == '#' {
            level += 1;
        } else {
            break;
        }
    }
    let level = level.min(6).saturating_sub(1);
    let content = trimmed.trim_start_matches('#').trim_start();

    let runs = parse_inline(content, theme);
    let wrapped = wrap_styled(&runs, width);

    wrapped
        .into_iter()
        .map(|spans| Line::from(spans).style(theme.heading[level].add_modifier(Modifier::BOLD)))
        .collect()
}

fn render_paragraph(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let runs = parse_inline(text.trim(), theme);
    let wrapped = wrap_styled(&runs, width);

    wrapped
        .into_iter()
        .map(|spans| Line::from(spans).style(theme.paragraph))
        .collect()
}

fn render_list(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut continuation_indent = 2usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        let (marker, content) = if trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
        {
            ("• ".to_string(), &trimmed[2..])
        } else if let Some(n) = trimmed.find(". ") {
            if trimmed[..n].parse::<u32>().is_ok() {
                (format!("{}. ", &trimmed[..n]), &trimmed[n + 2..])
            } else {
                (String::new(), trimmed)
            }
        } else {
            (String::new(), trimmed)
        };

        let prefix_width = if marker.is_empty() {
            continuation_indent
        } else {
            continuation_indent = indent.saturating_add(marker.len());
            continuation_indent
        };

        let item_width = width.saturating_sub(prefix_width.min(u16::MAX as usize) as u16);
        let runs = parse_inline(content, theme);
        let wrapped = wrap_styled(&runs, item_width);

        for (i, spans) in wrapped.into_iter().enumerate() {
            let prefix = if i == 0 && !marker.is_empty() {
                Span::styled(format!("{}{marker}", " ".repeat(indent)), theme.list_bullet)
            } else {
                Span::styled(" ".repeat(prefix_width), Style::default())
            };
            let mut line_spans = vec![prefix];
            line_spans.extend(spans);
            lines.push(Line::from(line_spans).style(theme.paragraph));
        }
    }
    lines
}

fn render_block_quote(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let content = line.trim_start_matches('>').trim_start();
        let item_width = width.saturating_sub(2);
        let runs = parse_inline(content, theme);
        let wrapped = wrap_styled(&runs, item_width);

        for spans in wrapped {
            let mut line_spans = vec![Span::styled("> ", theme.block_quote)];
            line_spans.extend(spans);
            lines.push(Line::from(line_spans).style(theme.block_quote));
        }
    }
    lines
}

fn render_code_fence(
    text: &str,
    width: u16,
    theme: &Theme,
    highlighter: &mut dyn Highlighter,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut all_lines: Vec<&str> = text.lines().collect();

    // Extract language from first line if it's a fence opener.
    let lang = if let Some(first) = all_lines.first() {
        if first.trim_start().starts_with("```") || first.trim_start().starts_with("~~~") {
            let lang = extract_fence_lang(first);
            all_lines = all_lines.into_iter().skip(1).collect();
            lang
        } else {
            None
        }
    } else {
        None
    };

    // Drop closing fence if present.
    if let Some(last) = all_lines.last() {
        if last.trim_start().starts_with("```") || last.trim_start().starts_with("~~~") {
            all_lines.pop();
        }
    }

    highlighter.begin_block(lang);

    for line in all_lines {
        let hl_runs = highlighter.highlight_line(lang, line);
        let mut spans: Vec<Span<'static>> = hl_runs
            .into_iter()
            .map(|(text, style)| Span::styled(text, style))
            .collect();

        // Pad to width with background color.
        let used = spans.iter().map(|s| s.width()).sum::<usize>() as u16;
        if used < width {
            spans.push(Span::styled(
                " ".repeat((width - used) as usize),
                theme.code_fence_default,
            ));
        }

        lines.push(Line::from(spans).style(theme.code_fence_default));
    }

    highlighter.end_block();

    lines
}

fn render_thematic_break(width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let w = width.max(1) as usize;
    vec![Line::from(Span::styled(
        "─".repeat(w),
        theme.thematic_break,
    ))]
}

fn render_table(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let parsed = crate::table::parse_table(text, theme);
    crate::table::compose_table(&parsed, width, theme)
}

fn render_raw(text: &str, width: u16, theme: &Theme, prefix: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let content = format!("{prefix} {line}");
        let runs = vec![(content, theme.paragraph)];
        let wrapped = wrap_styled(&runs, width);
        for spans in wrapped {
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// Extract language from a code fence opening line.
pub fn extract_fence_lang(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("```")
        .or_else(|| trimmed.strip_prefix("~~~"))
    {
        rest.split_whitespace().next()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bullet_theme() -> Theme {
        Theme {
            list_bullet: Style::default(),
            ..Theme::default()
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn unordered_list_items_get_bullet_glyphs() {
        let theme = bullet_theme();
        let lines = render_list("- one\n* two\n+ three", 40, &theme);

        assert_eq!(lines.len(), 3);
        assert_eq!(line_text(&lines[0]), "• one");
        assert_eq!(line_text(&lines[1]), "• two");
        assert_eq!(line_text(&lines[2]), "• three");
        assert_eq!(lines[0].spans[0].style, theme.list_bullet);
    }

    #[test]
    fn ordered_list_items_keep_their_numbers() {
        let theme = bullet_theme();
        let lines = render_list("1. one\n2. two", 40, &theme);

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "1. one");
        assert_eq!(line_text(&lines[1]), "2. two");
    }

    #[test]
    fn wrapped_list_continuations_align_with_item_text() {
        let theme = bullet_theme();
        let lines = render_list("10. alpha beta gamma", 12, &theme);

        assert!(lines.len() > 1);
        assert!(line_text(&lines[0]).starts_with("10. alpha"));
        for wrapped in &lines[1..] {
            assert!(line_text(wrapped).starts_with("    "));
        }
    }

    #[test]
    fn nested_list_items_keep_their_indentation() {
        let theme = bullet_theme();
        let lines = render_list("- parent\n  - child", 40, &theme);

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "• parent");
        assert_eq!(line_text(&lines[1]), "  • child");
    }

    #[test]
    fn renders_nested_mdstream_lists_with_bullets_and_indent() {
        let theme = bullet_theme();
        let mut stream = mdstream::MdStream::new(mdstream::Options::default());
        let mut state = mdstream::DocumentState::new();
        state.apply(stream.append("- parent\n  - child\n- sibling"));
        state.apply(stream.finalize());

        let blocks: Vec<_> = state.blocks().collect();
        assert_eq!(blocks.len(), 1);
        let mut highlighter = crate::highlight::NoHighlight;
        let lines = render_block(blocks[0], 40, &theme, &mut highlighter);

        assert_eq!(lines.len(), 3);
        assert_eq!(line_text(&lines[0]), "• parent");
        assert_eq!(line_text(&lines[1]), "  • child");
        assert_eq!(line_text(&lines[2]), "• sibling");
    }
}
