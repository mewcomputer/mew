use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::Block,
    Frame,
};

use crate::app::{App, Mode, SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH};

mod activity;
pub(crate) mod chat;
mod companion;
pub(crate) mod input;
pub(crate) mod overlays;
mod sidebar;
mod spinner;
mod status;
mod todos;
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
    unicode_width::UnicodeWidthStr::width(s)
}

/// Render the full UI.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(app.theme.resolve("chat.surface.background"))),
        area,
    );

    let show_sidebar = area.width >= SIDEBAR_MIN_WIDTH + SIDEBAR_WIDTH;
    app.sidebar_rect = Rect::default();
    app.sidebar_header_rows.clear();
    app.sidebar_item_rows.clear();

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
    app.autocomplete_area = Rect::default();

    let slash_cmds = app.filtered_slash_commands();
    let show_slash = app.mode == Mode::SlashCommand && !slash_cmds.is_empty();
    // The inline `@`-mention picker uses the same autocomplete slot as slash
    // commands. It only takes space when it has at least one match, so an
    // empty result set simply leaves the input at its normal position.
    let ref_count = app
        .picker
        .as_ref()
        .filter(|p| p.is_at_picker())
        .map(|p| p.filtered().len())
        .unwrap_or(0);
    let show_ref = app.mode == Mode::CommandPalette
        && app.picker.as_ref().is_some_and(|p| p.is_at_picker())
        && ref_count > 0;
    // Long lists can use three fifths of the screen. Each layout clips this
    // budget to preserve its composer; short lists shrink to their content.
    let max_autocomplete_height = (main_area.height as u32 * 3 / 5) as u16;
    let autocomplete_count = if show_slash {
        slash_cmds.len()
    } else {
        ref_count
    };
    let autocomplete_height = if show_slash || show_ref {
        autocomplete_count
            .min(u16::MAX as usize)
            .saturating_add(4)
            .min(max_autocomplete_height as usize) as u16
    } else {
        0
    };
    app.slash_visible = 0;

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

        let input_rect = welcome::draw_landing(f, app, hero_area);
        if app.mode == Mode::UserQuestion {
            if let Some(ref uq) = app.user_question {
                overlays::draw_user_question(f, uq, input_rect, &app.theme);
            }
        } else {
            input::draw_input(f, app, input_rect);
        }

        // The inline autocomplete (slash commands or `@` references) is
        // absolutely positioned directly above the centered input, like the
        // web dropdown: it overlays the logo if needed and never moves or
        // resizes the input box. Clip it to the rows actually available above
        // the box so it can cover the hero but not the input itself.
        let available_above = input_rect.y.saturating_sub(hero_area.y);
        let autocomplete_height = autocomplete_height.min(available_above);
        let autocomplete_rect = Rect::new(
            input_rect.x,
            input_rect.y.saturating_sub(autocomplete_height),
            input_rect.width,
            autocomplete_height,
        );
        if show_slash {
            overlays::draw_slash_autocomplete(f, app, &slash_cmds, autocomplete_rect);
        } else if show_ref {
            overlays::draw_reference_autocomplete(f, app, autocomplete_rect);
        }

        status::draw_status(f, app, status_rect);

        app.chat_area = hero_area;
        app.input_area = input_rect;
        app.clear_expired_alerts();
        draw_overlays(f, app, main_area);
        return;
    }

    let input_height = input::desired_height(app, main_area.width);
    let autocomplete_height = autocomplete_height.min(
        main_area
            .height
            .saturating_sub(input_height.saturating_add(2)),
    );

    // When an ask_user question is active, the input slot is replaced by the
    // question overlay. Reserve enough vertical space for the prompt + a few
    // options + the hint. Cap at half the terminal so a short terminal still
    // gets to see the chat above.
    let question_height = if app.mode == Mode::UserQuestion {
        input_height.min(main_area.height / 2).max(6)
    } else {
        input_height
    };

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(autocomplete_height), // inline autocomplete
            Constraint::Length(question_height),
            Constraint::Length(1), // status
        ])
        .split(main_area);

    chat::draw_chat(f, app, vert[0]);

    // If chat content would sit behind the companion, shrink to input bar.
    if companion::should_use_compact(app, vert[0]) {
        input::draw_companion_compact(f, app, vert[2]);
    } else {
        companion::draw_companion_float(f, app, vert[0]);
    }

    if show_slash {
        overlays::draw_slash_autocomplete(f, app, &slash_cmds, vert[1]);
    } else if show_ref {
        overlays::draw_reference_autocomplete(f, app, vert[1]);
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
    if app.mode == Mode::Activity {
        activity::draw_activity(f, app, area);
    }

    if app.mode == Mode::Settings {
        if let Some(ref settings) = app.settings {
            crate::settings::draw_settings(f, settings, area, &app.theme);
        }
    }

    if app.alert.is_some() {
        overlays::draw_alert(f, app, area);
    }

    if app.mode == Mode::PermissionPrompt {
        if let Some(ref mut perm) = app.permission {
            overlays::draw_permission_modal(f, perm, area, &app.theme);
        }
    }

    if app.mode == Mode::PersonaSwitchConfirm {
        if let Some(ref mut state) = app.persona_switch_confirm {
            overlays::draw_persona_confirm_modal(f, state, area, &app.theme);
        }
    }

    if app.mode == Mode::GoalProposal {
        if let Some(ref mut state) = app.goal_proposal {
            overlays::draw_goal_proposal(f, state, area, &app.theme);
        }
    }

    if app.mode == Mode::GoalCompose {
        if let Some(ref state) = app.goal_compose {
            overlays::draw_goal_compose(f, state, area, &app.theme);
        }
    }

    if app.mode == Mode::TodoEditor {
        todos::draw(f, app, area);
    }

    if app.mode == Mode::CommandPalette {
        if let Some(ref mut picker) = app.picker {
            // The inline `@`-mention picker is drawn above the input by the
            // layout; only centered pickers render as a modal overlay.
            if !picker.is_at_picker() {
                overlays::draw_picker(f, picker, area, &app.theme);
            }
        }
    }

    if app.mode == Mode::Help {
        overlays::draw_help_overlay(f, app, area);
    }
}
