use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use crate::inline::{parse_inline, StyledRun};
use crate::theme::Theme;
use crate::wrap::wrap_styled;

#[derive(Debug, Clone)]
pub(crate) struct ParsedTable {
    pub headers: Vec<Vec<StyledRun>>,
    pub aligns: Vec<Align>,
    pub rows: Vec<Vec<Vec<StyledRun>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Right,
    Center,
}

pub(crate) fn parse_table(text: &str, theme: &Theme) -> ParsedTable {
    let mut lines: Vec<&str> = text.lines().collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return ParsedTable {
            headers: vec![],
            aligns: vec![],
            rows: vec![],
        };
    }

    let header_cells = split_table_row(lines[0]);
    let num_cols = header_cells.len();

    let (aligns, data_start) = if lines.len() >= 2 && is_separator_row(lines[1]) {
        (parse_alignment_row(lines[1], num_cols), 2)
    } else {
        (vec![Align::default(); num_cols], 1)
    };

    let mut rows = vec![];
    for line in &lines[data_start..] {
        let cells = split_table_row(line);
        let mut row_cells = vec![];
        for cell in &cells {
            row_cells.push(parse_inline(cell, theme));
        }
        while row_cells.len() < num_cols {
            row_cells.push(vec![]);
        }
        rows.push(row_cells);
    }

    let headers: Vec<_> = header_cells
        .iter()
        .map(|c| parse_inline(c, theme))
        .collect();

    ParsedTable {
        headers,
        aligns,
        rows,
    }
}

/// Lay out a parsed table into styled ratatui lines with box-drawing borders.
pub(crate) fn compose_table(
    table: &ParsedTable,
    max_width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if table.headers.is_empty() || table.headers[0].is_empty() {
        return vec![];
    }

    let num_cols = table.headers.len().max(table.aligns.len());
    let available = (max_width as usize).saturating_sub(1 + num_cols * 3 + 1); // │·text·│…

    if available < num_cols * 3 {
        // Too narrow — fall back to minimalist rendering.
        return render_narrow(table, max_width, theme);
    }

    // Measure column widths from unwrapped content.
    let raw_widths: Vec<usize> = (0..num_cols)
        .map(|col| {
            let hw = cell_text_width(&table.headers.get(col));
            let rw = table
                .rows
                .iter()
                .map(|r| cell_text_width(&r.get(col)))
                .max()
                .unwrap_or(0);
            hw.max(rw).max(3)
        })
        .collect();

    // Distribute available width proportionally, ensuring 3 column minimums.
    let col_widths = fit_widths(&raw_widths, available);

    // Pre-wrap each cell's content.
    let header_wrapped: Vec<Vec<Vec<Span<'static>>>> = (0..num_cols)
        .map(|col| {
            let runs = table.headers.get(col).cloned().unwrap_or_default();
            wrap_styled(&runs, col_widths[col] as u16)
        })
        .collect();

    let rows_wrapped: Vec<Vec<Vec<Vec<Span<'static>>>>> = table
        .rows
        .iter()
        .map(|row| {
            (0..num_cols)
                .map(|col| {
                    let runs = row.get(col).cloned().unwrap_or_default();
                    wrap_styled(&runs, col_widths[col] as u16)
                })
                .collect()
        })
        .collect();

    // Find the height of each row (max lines across columns).
    let header_height = header_wrapped.iter().map(|c| c.len()).max().unwrap_or(1);
    let row_heights: Vec<usize> = rows_wrapped
        .iter()
        .map(|row| row.iter().map(|c| c.len()).max().unwrap_or(1))
        .collect();

    let border_style = theme.table_border;
    let header_style = theme.table_header;
    let cell_style = theme.table_cell;

    let mut lines = vec![];

    // Top border
    lines.push(border_line(&col_widths, "┌", "┬", "┐", border_style));

    // Header
    for line_idx in 0..header_height {
        lines.push(cell_line(
            &header_wrapped,
            line_idx,
            &col_widths,
            &table.aligns,
            header_style,
            border_style,
        ));
    }

    // Separator
    let sep_h = row_heights.iter().sum::<usize>() + header_height;
    for row_idx in 0..rows_wrapped.len() {
        lines.push(border_line(&col_widths, "├", "┼", "┤", border_style));
        for line_idx in 0..row_heights[row_idx] {
            lines.push(cell_line(
                &rows_wrapped[row_idx],
                line_idx,
                &col_widths,
                &table.aligns,
                cell_style,
                border_style,
            ));
        }
    }

    // Bottom border
    let _ = sep_h;
    lines.push(border_line(&col_widths, "└", "┴", "┘", border_style));

    lines
}

fn cell_text_width(cells: &Option<&Vec<StyledRun>>) -> usize {
    cells
        .and_then(|c| c.first().map(|_| c.iter().map(|(t, _)| t.width()).sum()))
        .unwrap_or(0)
}

fn fit_widths(raw: &[usize], available: usize) -> Vec<usize> {
    let total: usize = raw.iter().sum();
    if total <= available {
        return raw.to_vec();
    }
    let ratio = available as f64 / total as f64;
    raw.iter()
        .map(|w| (*w as f64 * ratio).ceil() as usize)
        .collect()
}

fn border_line(
    widths: &[usize],
    left: &str,
    mid: &str,
    right: &str,
    style: Style,
) -> Line<'static> {
    let mut spans = vec![Span::styled(left.to_string(), style)];
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(mid.to_string(), style));
        }
        spans.push(Span::styled("─".repeat(*w + 2), style));
    }
    spans.push(Span::styled(right.to_string(), style));
    Line::from(spans)
}

fn cell_line(
    cells: &[Vec<Vec<Span<'static>>>],
    line_idx: usize,
    widths: &[usize],
    aligns: &[Align],
    text_style: Style,
    border_style: Style,
) -> Line<'static> {
    let mut spans = vec![Span::styled("│", border_style)];

    for (col, cell) in cells.iter().enumerate() {
        if col > 0 {
            spans.push(Span::styled("│", border_style));
        }
        let w = widths[col];
        let line_spans = cell.get(line_idx).cloned().unwrap_or_default();
        let tw: usize = line_spans.iter().map(|s| s.width()).sum();

        let align = aligns.get(col).copied().unwrap_or_default();
        let (left, right) = match align {
            Align::Left => (1, (w + 2).saturating_sub(tw + 1)),
            Align::Right => ((w + 2).saturating_sub(tw + 1), 1),
            Align::Center => {
                let extra = (w + 2).saturating_sub(tw + 2);
                let half = extra / 2;
                (1 + half, (w + 2).saturating_sub(tw + 1 + half))
            }
        };

        spans.push(Span::styled(" ".repeat(left), text_style));
        for s in &line_spans {
            let mut styled = s.clone();
            if styled.style.fg.is_none() && styled.style.bg.is_none() {
                styled.style = text_style;
            }
            spans.push(styled);
        }
        spans.push(Span::styled(" ".repeat(right), text_style));
    }
    spans.push(Span::styled("│", border_style));
    Line::from(spans)
}

fn render_narrow(table: &ParsedTable, max_width: u16, theme: &Theme) -> Vec<Line<'static>> {
    // When the terminal is too narrow for a table, render each row as a compact list.
    let mut lines = vec![];
    for (row_idx, row) in table.rows.iter().enumerate() {
        if row_idx > 0 {
            lines.push(Line::from(""));
        }
        for (col, cell) in row.iter().enumerate() {
            let header = table
                .headers
                .get(col)
                .and_then(|h| h.first())
                .map(|(t, _)| format!("{t}: "))
                .unwrap_or_default();
            let runs: Vec<StyledRun> = cell.iter().map(|(t, s)| (t.clone(), *s)).collect();
            let wrapped = wrap_styled(&runs, max_width.saturating_sub(header.len() as u16 + 2));
            for wrap_line in wrapped {
                let mut spans = vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(header.clone(), theme.table_header),
                ];
                spans.extend(wrap_line);
                lines.push(Line::from(spans));
            }
        }
    }
    lines
}

fn split_table_row(line: &str) -> Vec<String> {
    let mut cells = vec![];
    let mut current = String::new();
    for ch in line.chars() {
        if ch == '|' {
            cells.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    cells.push(current.trim().to_string());
    if let Some(first) = cells.first() {
        if first.is_empty() && cells.len() > 1 {
            cells.remove(0);
        }
    }
    if let Some(last) = cells.last() {
        if last.is_empty() && cells.len() > 1 {
            cells.pop();
        }
    }
    cells
}

fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('|')
        && trimmed.contains('-')
        && !trimmed
            .chars()
            .any(|c| c != '|' && c != '-' && c != ':' && c != ' ')
}

fn parse_alignment_row(line: &str, _num_cols: usize) -> Vec<Align> {
    split_table_row(line)
        .iter()
        .map(|cell| {
            let cell = cell.trim();
            match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            }
        })
        .collect()
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
    fn test_parse_simple_table() {
        let text = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
        let table = parse_table(text, &theme());
        let all_headers: Vec<Vec<String>> = table
            .headers
            .iter()
            .map(|cells| cells.iter().map(|(t, _)| t.clone()).collect())
            .collect();
        assert_eq!(all_headers, vec![vec!["A"], vec!["B"]]);
        assert_eq!(table.aligns, vec![Align::Left, Align::Left]);
        assert_eq!(table.rows.len(), 2);
    }

    #[test]
    fn test_parse_alignment() {
        let text = "| A | B | C |\n| :-- | :-: | --: |\n| x | y | z |";
        let table = parse_table(text, &theme());
        assert_eq!(table.aligns, vec![Align::Left, Align::Center, Align::Right]);
    }

    #[test]
    fn test_compose_table() {
        let text = "| A | B |\n| --- | --- |\n| 1 | 2 |";
        let table = parse_table(text, &theme());
        let lines = compose_table(&table, 80, &theme());
        assert!(lines.len() >= 5);
        let first = lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(first.contains('┌'));
        let first_cell = &lines[1].spans;
        assert!(first_cell.iter().any(|s| s.content.contains("A")));
    }
}
