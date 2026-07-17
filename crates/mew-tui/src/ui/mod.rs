use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::Block,
    Frame,
};

use crate::app::{App, Mode, SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH};

pub(crate) mod chat;
mod companion;
mod input;
pub(crate) mod overlays;
mod sidebar;
mod spinner;
mod status;
mod welcome;

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
        let sidebar_bg =
            Block::default().style(Style::default().bg(app.theme.resolve("sidebar.background")));
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

    // Landing / start screen: when there's no conversation yet, show the cat +
    // "mew" hero with a centered input directly beneath it instead of the
    // normal bottom-pinned layout. Reverts to the normal layout automatically
    // once the first message lands (app.messages becomes non-empty).
    if app.messages.is_empty() {
        // Keep the status line pinned to the bottom; the hero + centered input
        // live in the region above it.
        let status_rect = Rect::new(
            main_area.x,
            main_area.bottom().saturating_sub(1),
            main_area.width,
            1,
        );
        let hero_area = Rect::new(
            main_area.x,
            main_area.y,
            main_area.width,
            main_area.height.saturating_sub(1),
        );

        let input_rect = welcome::draw_landing(f, app, hero_area, slash_height);
        if app.mode == Mode::UserQuestion {
            if let Some(ref uq) = app.user_question {
                overlays::draw_user_question(f, uq, input_rect, &app.theme);
            }
        } else {
            input::draw_input(f, app, input_rect);
        }

        // Slash autocomplete sits directly above the centered input.
        if show_slash {
            let slash_rect = Rect::new(
                input_rect.x,
                input_rect.y.saturating_sub(slash_height),
                input_rect.width,
                slash_height,
            );
            overlays::draw_slash_autocomplete(f, app, &slash_cmds, slash_rect);
        }

        status::draw_status(f, app, status_rect);

        app.chat_area = hero_area;
        app.input_area = input_rect;
        app.clear_expired_alerts();
        draw_overlays(f, app, main_area);
        return;
    }

    // Estimate the input's content width (the slot minus the 1-cell border
    // on each side and the 2-char prefix) so the layout reserves enough
    // vertical space for wrapped lines. The exact width is computed again
    // in `draw_input` once the slot is laid out.
    let input_content_width = main_area.width.saturating_sub(2).saturating_sub(2);
    let input_height = (app
        .input_visual_line_count(input_content_width)
        .clamp(1, 12)
        + 2) as u16;

    // When an ask_user question is active, the input slot is replaced by the
    // question overlay. Reserve enough vertical space for the prompt + a few
    // options + the hint. Cap at half the terminal so a short terminal still
    // gets to see the chat above.
    let question_height = if app.mode == Mode::UserQuestion {
        let rows = app
            .user_question
            .as_ref()
            .and_then(|uq| uq.questions.get(uq.page))
            .map(|q| {
                // Each option takes 2 rows (label + always-rendered description
                // row, even if empty) plus the freeform row. Plus prompt, blank,
                // hint, and padding.
                let n = q.options.len() as u16;
                n * 2 + 1 + 1 + 1 + 1 + 2
            })
            .unwrap_or(6);
        rows.min(main_area.height / 2).max(6)
    } else {
        input_height
    };

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(slash_height), // slash autocomplete
            Constraint::Length(question_height),
            Constraint::Length(1), // status
        ])
        .split(main_area);

    // Optional chat surface background. The chat widget itself is transparent
    // so this block fills the area behind the conversation.
    {
        let chat_bg = Block::default()
            .style(Style::default().bg(app.theme.resolve("chat.surface.background")));
        f.render_widget(chat_bg, vert[0]);
    }

    chat::draw_chat(f, app, vert[0]);

    // If chat content would sit behind the companion, shrink to input bar.
    if companion::should_use_compact(app, vert[0]) {
        input::draw_companion_compact(f, app, vert[2]);
    } else {
        companion::draw_companion_float(f, app, vert[0]);
    }

    if show_slash {
        overlays::draw_slash_autocomplete(f, app, &slash_cmds, vert[1]);
    }
    if app.mode == Mode::UserQuestion {
        if let Some(ref uq) = app.user_question {
            overlays::draw_user_question(f, uq, vert[2], &app.theme);
        }
    } else {
        input::draw_input(f, app, vert[2]);
    }
    status::draw_status(f, app, vert[3]);

    app.chat_area = vert[0];
    app.input_area = vert[2];

    app.clear_expired_alerts();
    draw_overlays(f, app, main_area);
}

/// Render modal overlays (alerts, permission prompts, persona-switch
/// confirms, command palette). The ask_user question overlay is rendered
/// directly by the layout that hosts it (it occupies the input slot, not a
/// centered popup), so it's not dispatched here.
fn draw_overlays(f: &mut Frame, app: &mut App, area: Rect) {
    if app.mode == Mode::Settings {
        if let Some(ref settings) = app.settings {
            crate::settings::draw_settings(f, settings, area, &app.theme);
        }
    }

    if app.alert.is_some() {
        overlays::draw_alert(f, app, area);
    }

    if app.mode == Mode::PermissionPrompt {
        if let Some(ref perm) = app.permission {
            overlays::draw_permission_modal(f, perm, area, &app.theme);
        }
    }

    if app.mode == Mode::PersonaSwitchConfirm {
        if let Some(ref state) = app.persona_switch_confirm {
            overlays::draw_persona_confirm_modal(f, state, area, &app.theme);
        }
    }

    if app.mode == Mode::PlanApproval {
        if let Some(ref state) = app.plan_approval {
            overlays::draw_plan_approval(f, state, area, &app.theme);
        }
    }

    if app.mode == Mode::CommandPalette {
        if let Some(ref mut picker) = app.picker {
            overlays::draw_picker(f, picker, area, &app.theme);
        }
    }

    if app.mode == Mode::Help {
        overlays::draw_help_overlay(f, area, &app.theme);
    }
}
