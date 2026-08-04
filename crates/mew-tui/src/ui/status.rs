use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use super::display_width;
use crate::app::{current_git_branch, App};

fn fmt_tokens(n: u32) -> String {
    if n >= 1_000 {
        format!("{:.1}k", n as f32 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// A pill in the status bar: a single bracketed block of text with its own
/// background color, so each pill reads as a distinct badge against the bar.
/// Brackets are part of the pill style (they're NOT in the text — the pill
/// text is just the label). `build_pills` produces the source list; the
/// segments helpers below interleave gaps and handle the colored marquee.
struct Pill {
    text: String,
    fg: Color,
    bg: Color,
}

/// A single rendered segment: either a pill or a 1-cell gap. The marquee and
/// the static line both render from `Vec<PillSegment>` so a scroll window
/// can keep per-pill colors.
struct PillSegment {
    text: String,
    fg: Color,
    bg: Color,
}

fn gap_segment(width: usize, tokens: &crate::theme::Theme) -> PillSegment {
    PillSegment {
        text: " ".repeat(width),
        fg: Color::Reset,
        bg: tokens.resolve("status_bar.background"),
    }
}

fn build_pills(app: &App, theme: &crate::theme::Theme) -> Vec<Pill> {
    let mut pills = Vec::new();

    // permission mode — prepended so it's the first thing the user sees.
    // Standard mode is implicit (no pill). Permissive gets an amber badge;
    // Auto gets a purple badge (the LLM is the gate); Auto+ gets a deeper
    // purple (same family, signals "more committed"); Dangerous! gets a
    // loud red badge. The visual cue matches the kind of decision-maker:
    // human / LLM / LLM-with-fail-closed / deterministic.
    match app.permission_mode {
        mew_hooks::PermissionMode::Standard => {}
        mew_hooks::PermissionMode::Permissive => pills.push(Pill {
            text: "Permissive".into(),
            fg: theme.resolve("pill.permissive.fg"),
            bg: theme.resolve("pill.permissive.bg"),
        }),
        mew_hooks::PermissionMode::Auto => pills.push(Pill {
            text: "Auto".into(),
            fg: theme.resolve("pill.auto.fg"),
            bg: theme.resolve("pill.auto.bg"),
        }),
        mew_hooks::PermissionMode::AutoPlus => pills.push(Pill {
            text: "Auto+".into(),
            fg: theme.resolve("pill.auto.fg"),
            bg: theme.resolve("pill.auto.bg"),
        }),
        mew_hooks::PermissionMode::Dangerous => pills.push(Pill {
            text: "⚠ Dangerous!".into(),
            fg: theme.resolve("pill.dangerous.fg"),
            bg: theme.resolve("pill.dangerous.bg"),
        }),
    }

    // model — dark green background, light green text.
    let model_label = if !app.status.provider.is_empty() && !app.status.model.is_empty() {
        format!("{}/{}", app.status.provider, app.status.model)
    } else if !app.status.model.is_empty() {
        app.status.model.clone()
    } else {
        String::new()
    };
    if !model_label.is_empty() {
        pills.push(Pill {
            text: model_label,
            fg: theme.resolve("pill.model.fg"),
            bg: theme.resolve("pill.model.bg"),
        });
    }

    // thinking variant — appended after the model pill with a separator.
    // Uses a distinct amber color to differentiate from the model chip.
    // Numeric budgets display as the bare token count (`budget:8192` → "8192").
    if let Some(ref variant) = app.active_thinking_variant {
        let label = variant
            .strip_prefix("budget:")
            .map(|n| n.to_owned())
            .unwrap_or_else(|| variant.clone());
        pills.push(Pill {
            text: label,
            fg: theme.resolve("pill.thinking.fg"),
            bg: theme.resolve("pill.thinking.bg"),
        });
    }

    // persona — uses the persona's accent color (explicit or deterministic).
    if let Some(ref name) = app.active_persona {
        pills.push(Pill {
            text: name.clone(),
            fg: theme.resolve("pill.persona.fg"),
            bg: theme.resolve("pill.persona.bg"),
        });
    }

    // cwd — dark blue background, light blue text.
    if !app.short_cwd.is_empty() {
        pills.push(Pill {
            text: app.short_cwd.clone(),
            fg: theme.resolve("pill.cwd.fg"),
            bg: theme.resolve("pill.cwd.bg"),
        });
    }

    // git — dark amber background, light amber text.
    if let Some(ref branch) = app.git_branch {
        pills.push(Pill {
            text: format!("git: {}", branch),
            fg: theme.resolve("pill.git.fg"),
            bg: theme.resolve("pill.git.bg"),
        });
    }

    // Cross-session attention (daemon mode): show "N need you" in amber
    // when any non-active session has pending permissions or questions.
    if app.daemon_mode {
        let total_attention: u32 = app
            .session_attention
            .iter()
            .filter(|(id, _)| *id != &app.status.session_id)
            .map(|(_, (p, q))| p + q)
            .sum();
        if total_attention > 0 {
            pills.push(Pill {
                text: format!("{} need you", total_attention),
                fg: theme.resolve("pill.attention.fg"),
                bg: theme.resolve("pill.attention.bg"),
            });
        }
    }

    // Session title (daemon mode): show the active session's title
    // next to the model/provider pills so the user can identify which
    // conversation they're in.
    if app.daemon_mode {
        if let Some(title) = app.session_titles.get(&app.status.session_id) {
            if !title.is_empty() {
                pills.push(Pill {
                    text: title.clone(),
                    fg: theme.resolve("pill.custom.fg"),
                    bg: theme.resolve("pill.custom.bg"),
                });
            }
        }
    }

    // Future pills slot in here: persona, perms, plugin-contributed, etc.
    pills
}

/// Interleave pills with 1-cell gaps, and append a trailing gap of
/// `trailing_gap` width so the marquee cycles don't run together.
fn build_segments(
    pills: &[Pill],
    trailing_gap: usize,
    tokens: &crate::theme::Theme,
) -> Vec<PillSegment> {
    let mut segs: Vec<PillSegment> = Vec::with_capacity(pills.len() * 2 + 1);
    for (i, p) in pills.iter().enumerate() {
        if i > 0 {
            segs.push(gap_segment(1, tokens));
        }
        segs.push(PillSegment {
            text: " ".to_string() + &p.text + " ",
            fg: p.fg,
            bg: p.bg,
        });
    }
    if trailing_gap > 0 {
        segs.push(gap_segment(trailing_gap, tokens));
    }
    segs
}

/// Plain concatenated pill text (texts joined by `" "`, no brackets) used
/// for width math.
fn pill_string(pills: &[Pill]) -> String {
    pills
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render segments as a single line (one span per segment). Used for the
/// static (fits) case.
fn segments_line(segs: &[PillSegment]) -> Line<'static> {
    segs.iter()
        .map(|s| Span::styled(s.text.clone(), Style::default().fg(s.fg).bg(s.bg)))
        .collect::<Vec<_>>()
        .into()
}

/// Color-preserving marquee: window `[offset, offset+width)` over the
/// concatenated segment text and emit spans that keep each pill's fg/bg.
/// Consecutive chars from the same segment coalesce into one span. The full
/// sequence already includes a trailing gap (via `build_segments(_, 3)`) so
/// the wrap-around shows a clear cycle separator.
fn segments_window(segs: &[PillSegment], width: usize, offset: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    // Precompute char positions and per-segment char vecs for O(1) lookup.
    let mut starts: Vec<usize> = Vec::with_capacity(segs.len());
    let mut segs_chars: Vec<Vec<char>> = Vec::with_capacity(segs.len());
    let mut pos = 0usize;
    for s in segs {
        starts.push(pos);
        let chars: Vec<char> = s.text.chars().collect();
        pos += chars.len();
        segs_chars.push(chars);
    }
    let total = pos;
    if total == 0 {
        return Line::from("");
    }

    let last_seg = segs.len() - 1;
    let mut spans_v: Vec<(String, Color, Color)> = Vec::new();
    for i in 0..width {
        let global_pos = (offset + i) % total;
        // Find the segment containing global_pos: largest starts[j] <= it.
        let seg_idx = match starts.binary_search(&global_pos) {
            Ok(j) => j,
            Err(j) => j.saturating_sub(1).min(last_seg),
        };
        let local_pos = global_pos - starts[seg_idx];
        let s = &segs[seg_idx];
        let ch = segs_chars[seg_idx][local_pos];
        if let Some(last) = spans_v.last_mut() {
            if last.1 == s.fg && last.2 == s.bg {
                last.0.push(ch);
                continue;
            }
        }
        spans_v.push((ch.to_string(), s.fg, s.bg));
    }
    let line_spans: Vec<Span<'static>> = spans_v
        .into_iter()
        .map(|(t, fg, bg)| Span::styled(t, Style::default().fg(fg).bg(bg)))
        .collect();
    Line::from(line_spans)
}

pub(super) fn draw_status(f: &mut Frame, app: &mut App, area: Rect) {
    let status_bg = app.theme.resolve("status_bar.background");
    let bg = Block::default().style(Style::default().bg(status_bg));
    f.render_widget(bg, area);

    // Resolve git branch once, lazily (avoids per-frame and per-test fs reads).
    if !app.git_branch_resolved {
        app.git_branch = current_git_branch();
        app.git_branch_resolved = true;
    }

    // Build a persona-accented theme clone for pill rendering.
    let pill_theme = if let Some(ref name) = app.active_persona {
        app.theme
            .with_persona_accent(name, app.active_persona_color.as_deref())
    } else {
        app.theme.clone()
    };

    let status = &app.status;
    let used = status.context_tokens;
    let right = if status.context_window > 0 {
        format!(
            "{} / {}k tok  ·  ${:.2}",
            fmt_tokens(used),
            status.context_window / 1_000,
            status.cost,
        )
    } else {
        format!("{} tok  ·  ${:.2}", fmt_tokens(used), status.cost)
    };

    let inner = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    let right_width = display_width(&right) as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(inner);
    let left_area = chunks[0];

    // Render the right side (tokens/cost), always pinned.
    let right_span = Span::styled(
        right,
        Style::default()
            .fg(app.theme.resolve("text.muted"))
            .bg(status_bg),
    );
    f.render_widget(
        Paragraph::new(Line::from(right_span)).alignment(Alignment::Right),
        chunks[1],
    );

    // Left side: transient status overrides take precedence, then pills.
    let left_line: Line<'static> = if app.esc_cancel_pending.is_some() {
        Line::from(Span::styled(
            "esc again to stop agent",
            Style::default()
                .fg(app.theme.resolve("text.warning"))
                .bg(status_bg),
        ))
    } else if app.ctrl_c_quit_pending.is_some() {
        Line::from(Span::styled(
            "ctrl-c again to quit",
            Style::default()
                .fg(app.theme.resolve("text.error"))
                .bg(status_bg),
        ))
    } else if let Some(ref retry) = app.retry_status {
        Line::from(Span::styled(
            retry.clone(),
            Style::default()
                .fg(app.theme.resolve("text.accent"))
                .bg(status_bg),
        ))
    } else {
        let pills = build_pills(app, &pill_theme);
        let pwidth = display_width(&pill_string(&pills)) as u16;
        if pwidth <= left_area.width {
            // Fits: render the pill row, reset the ticker.
            app.status_ticker_offset = 0;
            app.status_ticker_at = None;
            segments_line(&build_segments(&pills, 0, &app.theme))
        } else {
            // Overflow: color-preserving marquee with a trailing cycle gap.
            // Activate the ticker so `tick()` knows to advance it.
            if app.status_ticker_at.is_none() {
                app.status_ticker_at = Some(std::time::Instant::now());
            }
            let marquee_segs = build_segments(&pills, 3, &app.theme);
            segments_window(
                &marquee_segs,
                left_area.width as usize,
                app.status_ticker_offset,
            )
        }
    };

    f.render_widget(Paragraph::new(left_line), left_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pill(text: &str, fg: Color, bg: Color) -> Pill {
        Pill {
            text: text.into(),
            fg,
            bg,
        }
    }

    #[test]
    fn test_pill_string_joins_with_spaces() {
        let pills = vec![
            pill("a", Color::White, Color::Rgb(0, 0, 0)),
            pill("bb", Color::Cyan, Color::Rgb(0, 0, 0)),
        ];
        assert_eq!(pill_string(&pills), "a bb");
    }

    #[test]
    fn test_pill_string_single() {
        let pills = vec![pill("only", Color::White, Color::Rgb(0, 0, 0))];
        assert_eq!(pill_string(&pills), "only");
    }

    #[test]
    fn test_pill_string_empty() {
        assert_eq!(pill_string(&[]), "");
    }

    #[test]
    fn test_build_segments_interleaves_one_cell_gaps() {
        let app = crate::app::App::new();
        let pills = vec![
            pill("a", Color::White, Color::Rgb(0, 0, 0)),
            pill("b", Color::Cyan, Color::Rgb(0, 0, 0)),
        ];
        let segs = build_segments(&pills, 0, &app.theme);
        assert_eq!(segs.len(), 3);
        // Pills now have a leading/trailing space for visual padding.
        assert_eq!(segs[0].text, " a ");
        assert_eq!(segs[1].text, " ");
        assert_eq!(segs[1].bg, app.theme.resolve("status_bar.background"));
        assert_eq!(segs[2].text, " b ");
    }

    #[test]
    fn test_build_segments_trailing_gap_for_marquee() {
        let app = crate::app::App::new();
        let pills = vec![pill("ab", Color::White, Color::Rgb(0, 0, 0))];
        let segs = build_segments(&pills, 3, &app.theme);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, " ab ");
        assert_eq!(segs[1].text, "   ");
        assert_eq!(segs[1].bg, app.theme.resolve("status_bar.background"));
    }

    #[test]
    fn test_segments_line_emits_one_span_per_segment() {
        let app = crate::app::App::new();
        let pills = vec![
            pill("a", Color::White, Color::Rgb(10, 0, 0)),
            pill("b", Color::Cyan, Color::Rgb(0, 10, 0)),
        ];
        let segs = build_segments(&pills, 0, &app.theme);
        let line = segments_line(&segs);
        // 3 spans: pill_a, gap, pill_b
        assert_eq!(line.spans.len(), 3);
    }

    #[test]
    fn test_segments_window_width_and_color_preservation() {
        let app = crate::app::App::new();
        // Two pills with distinct bgs; window should include parts of both
        // and the gap, with each part's style intact.
        let pills = vec![
            pill("AA", Color::White, Color::Rgb(10, 0, 0)),
            pill("BB", Color::Cyan, Color::Rgb(0, 10, 0)),
        ];
        // full sequence: " AA " + " " + " BB " + "   " = " AA  BB    " (12 chars).
        let segs = build_segments(&pills, 3, &app.theme);
        // offset 0, width 10 → first 10 chars = " AA  BB  ".
        let line = segments_window(&segs, 10, 0);
        let total_text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(total_text, " AA   BB  ");
        // The first span carries the first pill's bg, the gap carries
        // app.theme.resolve("status_bar.background"), the second pill carries its own bg.
        assert_eq!(line.spans[0].style.bg, Some(Color::Rgb(10, 0, 0)));
        assert_eq!(
            line.spans[1].style.bg,
            Some(app.theme.resolve("status_bar.background"))
        );
        assert_eq!(line.spans[2].style.bg, Some(Color::Rgb(0, 10, 0)));
    }

    #[test]
    fn test_segments_window_wraps_with_offset() {
        let app = crate::app::App::new();
        let pills = vec![pill("abc", Color::White, Color::Rgb(0, 0, 0))];
        let segs = build_segments(&pills, 3, &app.theme);
        // full text: " abc " + "   " = " abc    " (8 chars).
        // offset 0, width 3 → " ab".
        assert_eq!(text_of(&segments_window(&segs, 3, 0)), " ab");
        // offset 5, width 3 → last 3 trailing spaces.
        let line = segments_window(&segs, 3, 5);
        let t: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(t.chars().count(), 3);
        // The first char at offset 5 should be a space.
        assert!(t.starts_with(' '));
    }

    #[test]
    fn test_segments_window_cycles_end_to_start() {
        let app = crate::app::App::new();
        // offset equal to the total char count must wrap to offset 0.
        let pills = vec![pill("ab", Color::White, Color::Rgb(0, 0, 0))];
        let segs = build_segments(&pills, 3, &app.theme);
        // full text = "ab" + "   " = 5 chars.
        let total: usize = segs.iter().map(|s| s.text.chars().count()).sum();
        let a = segments_window(&segs, 2, 0);
        let b = segments_window(&segs, 2, total);
        let ta: String = a.spans.iter().map(|s| s.content.to_string()).collect();
        let tb: String = b.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(ta, tb);
    }

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn test_build_pills_standard_mode_has_no_perm_pill() {
        let mut app = crate::app::App::new();
        app.permission_mode = mew_hooks::PermissionMode::Standard;
        app.status.model = "test-model".into();
        let pills = build_pills(&app, &app.theme);
        // No pill should contain the "Dangerous" warning text.
        assert!(
            pills.iter().all(|p| !p.text.contains("Dangerous")),
            "Standard mode must not show the Dangerous! pill, got: {:?}",
            pills.iter().map(|p| &p.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_build_pills_dangerous_mode_prepends_warning_pill() {
        let mut app = crate::app::App::new();
        app.permission_mode = mew_hooks::PermissionMode::Dangerous;
        app.status.model = "test-model".into();
        let pills = build_pills(&app, &app.theme);
        assert!(!pills.is_empty(), "expected at least one pill");
        assert!(
            pills[0].text.contains("Dangerous"),
            "Dangerous! pill should be first (prepended), got first: {:?}",
            pills[0].text
        );
    }

    #[test]
    fn test_build_pills_permissive_mode_prepends_amber_pill() {
        let mut app = crate::app::App::new();
        app.permission_mode = mew_hooks::PermissionMode::Permissive;
        app.status.model = "test-model".into();
        let pills = build_pills(&app, &app.theme);
        assert!(!pills.is_empty(), "expected at least one pill");
        assert_eq!(
            pills[0].text, "Permissive",
            "Permissive pill should be first (prepended), got first: {:?}",
            pills[0].text
        );
        // The amber pill should NOT contain the red "Dangerous!" cue.
        assert!(
            !pills[0].text.contains("Dangerous"),
            "Permissive pill must not show Dangerous! cue"
        );
    }

    #[test]
    fn test_build_pills_auto_mode_prepends_purple_pill() {
        let mut app = crate::app::App::new();
        app.permission_mode = mew_hooks::PermissionMode::Auto;
        app.status.model = "test-model".into();
        let pills = build_pills(&app, &app.theme);
        assert!(!pills.is_empty(), "expected at least one pill");
        assert_eq!(
            pills[0].text, "Auto",
            "Auto pill should be first (prepended), got first: {:?}",
            pills[0].text
        );
        // Auto resolves to pill.auto tokens, distinct from amber Permissive and red Dangerous.
        let auto_bg = app.theme.resolve("pill.auto.bg");
        let permissive_bg = app.theme.resolve("pill.permissive.bg");
        let dangerous_bg = app.theme.resolve("pill.dangerous.bg");
        assert_eq!(pills[0].bg, auto_bg, "Auto must use pill.auto.bg");
        assert!(
            pills[0].bg != permissive_bg,
            "Auto must not be amber (permissive bg)"
        );
        assert!(
            pills[0].bg != dangerous_bg,
            "Auto must not be red (dangerous bg)"
        );
    }

    #[test]
    fn test_build_pills_autoplus_mode_prepends_deeper_purple_pill() {
        let mut app = crate::app::App::new();
        app.permission_mode = mew_hooks::PermissionMode::AutoPlus;
        app.status.model = "test-model".into();
        let pills = build_pills(&app, &app.theme);
        assert!(!pills.is_empty(), "expected at least one pill");
        assert_eq!(
            pills[0].text, "Auto+",
            "Auto+ pill should be first (prepended), got first: {:?}",
            pills[0].text
        );
        // Auto+ also resolves to pill.auto tokens.
        let auto_bg = app.theme.resolve("pill.auto.bg");
        assert_eq!(pills[0].bg, auto_bg, "Auto+ must use pill.auto.bg");
    }
}
