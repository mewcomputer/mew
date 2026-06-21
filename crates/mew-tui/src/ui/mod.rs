use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::Block,
    Frame,
};

use crate::app::{App, Mode, SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH};

mod chat;
mod companion;
mod input;
mod overlays;
mod sidebar;
mod spinner;
mod status;
mod welcome;

/// Background color for the input and status line surface.
pub(super) const STATUS_BG: Color = Color::Rgb(30, 30, 33);
/// Background color for the sidebar surface.
pub(super) const SIDEBAR_BG: Color = Color::Rgb(28, 28, 31);
/// Background color for tool call blocks. Picked to be clearly lighter than
/// the surrounding chat surface so the block reads as a filled card; the
/// half-block top/bottom edges add a 1-row soft transition into the fill.
pub(super) const TOOL_BG: Color = Color::Rgb(50, 50, 56);
/// Subtle divider color.
pub(super) const DIVIDER: Color = Color::Rgb(50, 50, 55);
/// Max lines of bash output shown when collapsed.
pub(super) const BASH_LINES_COLLAPSED: usize = 10;
/// Max lines of bash output shown when expanded.
pub(super) const BASH_LINES_EXPANDED: usize = 50;
/// Max lines shown for non-bash tool output.
pub(super) const TOOL_LINES_MAX: usize = 20;
/// Max lines of live streaming output shown while a tool is running.
pub(super) const TOOL_LINES_LIVE: usize = 5;
/// Max lines of diff shown inline.
pub(super) const DIFF_LINES_MAX: usize = 30;

/// Compute display width of a string using Unicode standard widths.
pub(super) fn display_width(s: &str) -> usize {
    s.chars()
        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

/// Render the full UI.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    let show_sidebar = area.width >= SIDEBAR_MIN_WIDTH + SIDEBAR_WIDTH;

    let main_chunks: Vec<Rect> = if show_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(SIDEBAR_MIN_WIDTH),
                Constraint::Length(SIDEBAR_WIDTH),
            ])
            .split(area);
        let sidebar_bg = Block::default().style(Style::default().bg(SIDEBAR_BG));
        f.render_widget(sidebar_bg, chunks[1]);
        sidebar::draw_sidebar(f, app, chunks[1]);
        chunks.to_vec()
    } else {
        vec![area]
    };

    let main_area = main_chunks[0];

    let slash_cmds = app.filtered_slash_commands();
    let show_slash = app.mode == Mode::SlashCommand && !slash_cmds.is_empty();
    // Cap the visible list at min(12, half the terminal height) so a long
    // command set doesn't eat the whole screen, and tall terminals get more
    // items. If fewer commands exist than the cap, show them all.
    let max_visible_items: u16 = (area.height / 2).min(12);
    let slash_height = if show_slash {
        (slash_cmds.len() as u16).min(max_visible_items) + 2
    } else {
        0
    };

    // Estimate the input's content width (the slot minus the 1-cell border
    // on each side) so the layout reserves enough vertical space for wrapped
    // lines. The exact width is computed again in `draw_input` once the slot
    // is laid out.
    let input_content_width = main_area.width.saturating_sub(2);
    let input_height = (app
        .input_visual_line_count(input_content_width)
        .clamp(1, 12)
        + 2) as u16;

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),            // divider
            Constraint::Length(slash_height), // slash autocomplete
            Constraint::Length(input_height),
            Constraint::Length(1), // status
        ])
        .split(main_area);

    chat::draw_chat(f, app, vert[0]);

    // If chat content would sit behind the companion, shrink to input bar.
    if companion::should_use_compact(app, vert[0]) {
        input::draw_companion_compact(f, app, vert[3]);
    } else {
        companion::draw_companion_float(f, app, vert[0]);
    }

    status::draw_divider(f, vert[1]);
    if show_slash {
        overlays::draw_slash_autocomplete(f, app, &slash_cmds, vert[2]);
    }
    input::draw_input(f, app, vert[3]);
    status::draw_status(f, app, vert[4]);

    app.chat_area = vert[0];
    app.input_area = vert[3];

    app.clear_expired_alerts();

    if app.alert.is_some() {
        overlays::draw_alert(f, app, main_area);
    }

    if app.mode == Mode::PermissionPrompt {
        if let Some(ref perm) = app.permission {
            overlays::draw_permission_modal(f, perm, main_area);
        }
    }

    if app.mode == Mode::UserQuestion {
        if let Some(ref uq) = app.user_question {
            overlays::draw_user_question_modal(f, uq, main_area);
        }
    }

    if app.mode == Mode::PersonaSwitchConfirm {
        if let Some(ref state) = app.persona_switch_confirm {
            overlays::draw_persona_confirm_modal(f, state, main_area);
        }
    }

    if app.mode == Mode::CommandPalette {
        if let Some(ref mut picker) = app.picker {
            overlays::draw_picker(f, picker, main_area);
        }
    }
}
