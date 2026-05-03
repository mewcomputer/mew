use ratatui::style::{Color, Modifier, Style};

/// Styling for markdown elements.
#[derive(Clone)]
pub struct Theme {
    pub paragraph: Style,
    pub heading: [Style; 6],
    pub emphasis: Style,
    pub strong: Style,
    pub strikethrough: Style,
    pub inline_code: Style,
    pub link_text: Style,
    pub link_url: Style,
    pub list_bullet: Style,
    pub block_quote: Style,
    pub thematic_break: Style,
    pub table_header: Style,
    pub table_cell: Style,
    pub table_border: Style,
    pub code_fence_default: Style,
    pub code_fence_border: Style,
    pub pending_indicator: Style,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
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

    pub fn light() -> Self {
        Self::dark()
    }

    pub fn monochrome() -> Self {
        Self::dark()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}
