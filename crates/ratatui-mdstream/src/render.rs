use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::{
    highlight::Highlighter,
    inline::parse_inline,
    pending::PendingPolicy,
    theme::Theme,
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
    policy.render(block.kind, block.display_or_raw(), width, theme, highlighter)
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
        .map(|spans| {
            Line::from(spans).style(
                theme.heading[level]
                    .add_modifier(Modifier::BOLD),
            )
        })
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
    for line in text.lines() {
        let trimmed = line.trim_start();
        let _indent = line.len() - trimmed.len();
        let content = if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            &trimmed[2..]
        } else if let Some(n) = trimmed.find(". ") {
            if trimmed[..n].parse::<u32>().is_ok() {
                &trimmed[n + 2..]
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        let bullet = Span::styled("  ", theme.list_bullet);
        let item_width = width.saturating_sub(2);
        let runs = parse_inline(content, theme);
        let wrapped = wrap_styled(&runs, item_width);

        for (i, spans) in wrapped.into_iter().enumerate() {
            let mut line_spans = Vec::new();
            if i == 0 {
                line_spans.push(bullet.clone());
            } else {
                line_spans.push(Span::styled("  ", Style::default()));
            }
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

fn render_raw(text: &str, width: u16, _theme: &Theme, prefix: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let content = format!("{prefix} {line}");
        let runs = vec![(content, Style::default().fg(Color::DarkGray))];
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
    if trimmed.starts_with("```") {
        let rest = &trimmed[3..].trim();
        rest.split_whitespace().next()
    } else if trimmed.starts_with("~~~") {
        let rest = &trimmed[3..].trim();
        rest.split_whitespace().next()
    } else {
        None
    }
}
