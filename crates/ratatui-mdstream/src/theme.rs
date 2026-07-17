use ratatui::style::{Modifier, Style};

/// Styling for markdown elements.
///
/// This struct is intentionally empty of defaults: every style must be
/// supplied by the caller. In mew-tui the active `Theme::md_theme()` builds
/// these styles from the shared token manifest, so no hardcoded colors live
/// inside the markdown renderer.
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

/// A neutral theme with no foreground/background colors set. Used only as a
/// placeholder when a caller has not supplied a theme (e.g. some tests).
pub fn neutral() -> Theme {
    Theme {
        paragraph: Style::default(),
        heading: [
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().add_modifier(Modifier::BOLD),
        ],
        emphasis: Style::default().add_modifier(Modifier::ITALIC),
        strong: Style::default().add_modifier(Modifier::BOLD),
        strikethrough: Style::default().add_modifier(Modifier::CROSSED_OUT),
        inline_code: Style::default(),
        link_text: Style::default().add_modifier(Modifier::UNDERLINED),
        link_url: Style::default(),
        list_bullet: Style::default(),
        block_quote: Style::default(),
        thematic_break: Style::default(),
        table_header: Style::default().add_modifier(Modifier::BOLD),
        table_cell: Style::default(),
        table_border: Style::default(),
        code_fence_default: Style::default(),
        code_fence_border: Style::default(),
        pending_indicator: Style::default(),
    }
}

impl Default for Theme {
    fn default() -> Self {
        neutral()
    }
}
