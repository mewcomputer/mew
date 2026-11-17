use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::*;
use libghostty_vt::fmt::Format;
use libghostty_vt::key::{Action as KeyAction, Encoder as KeyEncoder, Event as KeyEvent, Mods};
use libghostty_vt::mouse::{
    self, Action as MouseAction, Button, Encoder as MouseEncoder, Event as MouseEvent,
};
use libghostty_vt::render::{CellIterator, CursorViewport, Dirty, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::selection::{
    FormatOptions as SelectionFormatOptions, Selection as TerminalSelection,
};
use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Point as TerminalPoint,
    PointCoordinate, PrimaryDeviceAttributes, ScrollViewport, SecondaryDeviceAttributes,
    SizeReportSize,
};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};

use super::constants::*;
use super::data::RowData;
use super::grid::{GridPos, Selection};
use super::key_map::{ctrl_char, gpui_key_to_ghostty};
use super::metrics::CellMetrics;
use super::render::build_row_elements;
use super::styling::{hsla_from_rgb, style_to_highlight};

pub struct TerminalView {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_iter: RowIterator<'static>,
    cell_iter: CellIterator<'static>,
    key_encoder: KeyEncoder<'static>,
    key_event: KeyEvent<'static>,
    mouse_encoder: MouseEncoder<'static>,
    mouse_event: MouseEvent<'static>,
    focus_handle: FocusHandle,
    cached_rows: Vec<RowData>,
    cached_cursor: Option<libghostty_vt::render::CursorViewport>,
    last_default_fg: Hsla,
    last_default_bg: Hsla,
    custom_theme_colors: bool,
    presentation_dirty: bool,
    selection: Option<Selection>,
    selecting: bool,
    protocol_write_buf: Rc<RefCell<Vec<u8>>>,
    #[allow(dead_code)]
    window_handle: Option<AnyWindowHandle>,
    current_cols: u16,
    current_rows: u16,
    cell_metrics: CellMetrics,
    font_family: SharedString,
    view_bounds: Bounds<Pixels>,
    // Shared with on_size callback: (cols, rows, cell_w_px, cell_h_px)
    size_info: Rc<Cell<(u16, u16, u32, u32)>>,
    // Count of mouse buttons currently held, for set_any_button_pressed
    buttons_pressed: u8,
}

impl TerminalView {
    pub fn new_remote(cx: &mut Context<Self>) -> Self {
        let terminal = Terminal::new(TerminalOptions {
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            max_scrollback: 10_000,
        })
        .expect("failed to create terminal");
        Self::build(terminal, cx)
    }

    fn build(mut terminal: Terminal<'static, 'static>, cx: &mut Context<Self>) -> Self {
        let render_state = RenderState::new().expect("failed to create render state");
        let row_iter = RowIterator::new().expect("failed to create row iterator");
        let cell_iter = CellIterator::new().expect("failed to create cell iterator");
        let key_encoder = KeyEncoder::new().expect("failed to create key encoder");
        let key_event = KeyEvent::new().expect("failed to create key event");
        let mouse_encoder = MouseEncoder::new().expect("failed to create mouse encoder");
        let mouse_event = MouseEvent::new().expect("failed to create mouse event");

        let protocol_write_buf = Rc::new(RefCell::new(Vec::new()));
        let write_buf = protocol_write_buf.clone();

        terminal
            .on_pty_write(move |_term, data| {
                write_buf.borrow_mut().extend_from_slice(data);
            })
            .expect("pty_write");

        // Shared grid/cell size so on_size can respond to XTWINOPS queries correctly.
        // Updated in maybe_resize on every frame.
        let size_info = Rc::new(Cell::new((
            DEFAULT_COLS,
            DEFAULT_ROWS,
            FALLBACK_CELL_WIDTH as u32,
            FALLBACK_CELL_HEIGHT as u32,
        )));
        let size_cb = size_info.clone();

        // Respond to XTWINOPS size queries (CSI 14/16/18 t) so programs can
        // discover the terminal geometry in cells and pixels.
        terminal
            .on_size(move |_term| {
                let (cols, rows, cw, ch) = size_cb.get();
                Some(SizeReportSize {
                    rows,
                    columns: cols,
                    cell_width: cw,
                    cell_height: ch,
                })
            })
            .expect("on_size");

        // Respond to DA1/DA2/DA3 queries so programs can identify terminal
        // capabilities. Without this, vim and tmux may hang on startup.
        terminal
            .on_device_attributes(|_term| {
                Some(DeviceAttributes {
                    primary: PrimaryDeviceAttributes::new(
                        ConformanceLevel::VT220,
                        &[
                            DeviceAttributeFeature::COLUMNS_132,
                            DeviceAttributeFeature::SELECTIVE_ERASE,
                            DeviceAttributeFeature::ANSI_COLOR,
                        ],
                    ),
                    secondary: SecondaryDeviceAttributes {
                        device_type: DeviceType::VT220,
                        firmware_version: 1,
                        rom_cartridge: 0,
                    },
                    tertiary: Default::default(),
                })
            })
            .expect("on_device_attributes");

        // Respond to CSI > q with an application name (XTVERSION).
        terminal
            .on_xtversion(|_term| Some("artemis"))
            .expect("on_xtversion");

        // Respond to CSI ? 996 n (color scheme query). We return None since
        // we don't have OS-level dark/light mode detection yet.
        terminal
            .on_color_scheme(|_term| None)
            .expect("on_color_scheme");
        Self {
            terminal,
            render_state,
            row_iter,
            cell_iter,
            key_encoder,
            key_event,
            mouse_encoder,
            mouse_event,
            focus_handle: cx.focus_handle(),
            cached_rows: Vec::new(),
            cached_cursor: None,
            last_default_fg: Hsla::white(),
            last_default_bg: Hsla::black(),
            custom_theme_colors: false,
            presentation_dirty: true,
            selection: None,
            selecting: false,
            protocol_write_buf,
            window_handle: None,
            current_cols: DEFAULT_COLS,
            current_rows: DEFAULT_ROWS,
            cell_metrics: CellMetrics::fallback(),
            font_family: DEFAULT_FONT_FAMILY.into(),
            view_bounds: Bounds::default(),
            size_info,
            buttons_pressed: 0,
        }
    }

    pub fn ingest(&mut self, data: &[u8], cx: &mut Context<Self>) {
        self.terminal.vt_write(data);
        self.flush_protocol_writes(cx);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.terminal.reset();
        self.cached_rows.clear();
        self.cached_cursor = None;
        self.selection = None;
        self.selecting = false;
        self.presentation_dirty = true;
        cx.notify();
    }

    pub fn set_theme_colors(
        &mut self,
        foreground: gpui::Rgba,
        background: gpui::Rgba,
        cx: &mut Context<Self>,
    ) {
        self.last_default_fg = hsla_from_rgb(
            (foreground.r * 255.0).round() as u8,
            (foreground.g * 255.0).round() as u8,
            (foreground.b * 255.0).round() as u8,
        );
        self.last_default_bg = hsla_from_rgb(
            (background.r * 255.0).round() as u8,
            (background.g * 255.0).round() as u8,
            (background.b * 255.0).round() as u8,
        );
        self.custom_theme_colors = true;
        self.presentation_dirty = true;
        cx.notify();
    }

    pub fn set_font_family(&mut self, family: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.font_family = family.into();
        cx.notify();
    }

    fn flush_protocol_writes(&mut self, cx: &mut Context<Self>) {
        let buf = self.protocol_write_buf.borrow();
        if buf.is_empty() {
            return;
        }
        cx.emit(TerminalEvent::Input(buf.clone()));
        drop(buf);
        self.protocol_write_buf.borrow_mut().clear();
    }

    fn write_input(&mut self, data: &[u8], cx: &mut Context<Self>) {
        cx.emit(TerminalEvent::Input(data.to_vec()));
    }

    fn is_mouse_tracking(&self) -> bool {
        self.terminal.is_mouse_tracking().unwrap_or(false)
    }

    fn maybe_resize(&mut self, window: &Window, cx: &mut Context<Self>) {
        let metrics = CellMetrics::measure(window, self.font_family.as_ref(), cx);
        // Use the captured element bounds directly — this is correct for any
        // pane position, not just the rightmost one.
        if self.view_bounds.size.width <= px(0.) || self.view_bounds.size.height <= px(0.) {
            return;
        }
        let view_w = f32::from(self.view_bounds.size.width) - PADDING_LEFT;
        let view_h = f32::from(self.view_bounds.size.height) - PADDING_TOP;
        let new_cols = (view_w / metrics.width).max(1.0) as u16;
        let new_rows = (view_h / metrics.height).max(1.0) as u16;

        let cw = metrics.width as u32;
        let ch = metrics.height as u32;
        self.size_info.set((new_cols, new_rows, cw, ch));

        if new_cols != self.current_cols || new_rows != self.current_rows {
            self.current_cols = new_cols;
            self.current_rows = new_rows;
            self.cell_metrics = metrics;
            let _ = self.terminal.resize(new_cols, new_rows, cw, ch);
            cx.emit(TerminalEvent::Resize {
                cols: new_cols,
                rows: new_rows,
            });
        } else {
            self.cell_metrics = metrics;
        }
    }

    fn render_terminal(&mut self) -> (Option<Vec<RowData>>, Option<CursorViewport>, Hsla, Hsla) {
        let snapshot = self
            .render_state
            .update(&self.terminal)
            .expect("render state update failed");

        let colors = snapshot.colors().expect("failed to read colors");
        let cursor_visible = snapshot.cursor_visible().unwrap_or(true);
        let cursor = snapshot
            .cursor_viewport()
            .ok()
            .flatten()
            .filter(|_| cursor_visible);

        let default_fg = hsla_from_rgb(
            colors.foreground.r,
            colors.foreground.g,
            colors.foreground.b,
        );
        let default_bg = hsla_from_rgb(
            colors.background.r,
            colors.background.g,
            colors.background.b,
        );

        let (default_fg, default_bg) = if self.custom_theme_colors {
            (self.last_default_fg, self.last_default_bg)
        } else {
            (default_fg, default_bg)
        };

        let sel_bg = hsla_from_rgb(240, 153, 179);

        let dirty = snapshot.dirty().ok().unwrap_or(Dirty::Full);

        let cached_cursor = cursor;
        let mut new_rows: Option<Vec<RowData>> = None;

        if self.presentation_dirty || matches!(dirty, Dirty::Full | Dirty::Partial) {
            let num_cols = snapshot.cols().unwrap_or(self.current_cols) as usize;
            let num_rows = snapshot.rows().unwrap_or(self.current_rows) as usize;

            let mut rows: Vec<RowData> = Vec::with_capacity(num_rows);
            let mut row_iter = self
                .row_iter
                .update(&snapshot)
                .expect("row iterator update failed");
            let mut cell_text = String::with_capacity(4);

            for row_idx in 0..num_rows {
                let Some(row) = row_iter.next() else { break };

                let mut text = String::with_capacity(num_cols);
                let mut highlights = Vec::new();
                let mut bg_runs: Vec<super::data::BgRun> = Vec::new();
                let mut run_start = 0usize;
                let mut run_color = default_bg;

                let mut cell_iter = self
                    .cell_iter
                    .update(row)
                    .expect("cell iterator update failed");

                let mut col_idx = 0;
                while col_idx < num_cols {
                    let Some(cell) = cell_iter.next() else {
                        // Missing cell: bg = default_bg, no highlight needed.
                        if default_bg != run_color {
                            bg_runs.push(super::data::BgRun::new(run_start, col_idx, run_color));
                            run_start = col_idx;
                            run_color = default_bg;
                        }
                        text.push(' ');
                        col_idx += 1;
                        continue;
                    };

                    let cell_wide = cell.raw_cell().ok().and_then(|c| c.wide().ok());

                    if matches!(cell_wide, Some(CellWide::SpacerTail)) {
                        col_idx += 1;
                        continue;
                    }

                    let start = text.len();
                    cell_text.clear();
                    let _ = cell.graphemes_utf8(&mut cell_text);
                    if cell_text.is_empty() {
                        text.push(' ');
                    } else {
                        text.push_str(&cell_text);
                    }

                    let is_wide = matches!(cell_wide, Some(CellWide::Wide));
                    let end = text.len();
                    let col_span = if is_wide { 2 } else { 1 };

                    let style = cell.style().unwrap_or_default();
                    let fg = cell
                        .fg_color()
                        .ok()
                        .flatten()
                        .map(|c| hsla_from_rgb(c.r, c.g, c.b));
                    let bg = cell
                        .bg_color()
                        .ok()
                        .flatten()
                        .map(|c| hsla_from_rgb(c.r, c.g, c.b));

                    let is_cursor = cursor
                        .as_ref()
                        .is_some_and(|c| c.x as usize == col_idx && c.y as usize == row_idx);

                    let is_selected = self.selection.is_some_and(|sel| {
                        (0..col_span).any(|offset| {
                            sel.contains(GridPos {
                                col: col_idx + offset,
                                row: row_idx,
                            })
                        })
                    });

                    let effective_bg = if is_cursor {
                        default_fg
                    } else if is_selected {
                        sel_bg
                    } else if style.inverse {
                        fg.unwrap_or(default_fg)
                    } else {
                        bg.unwrap_or(default_bg)
                    };

                    if effective_bg != run_color {
                        bg_runs.push(super::data::BgRun::new(run_start, col_idx, run_color));
                        run_start = col_idx;
                        run_color = effective_bg;
                    }

                    let highlight = if is_cursor {
                        HighlightStyle {
                            color: Some(default_bg),
                            ..Default::default()
                        }
                    } else {
                        let mut h = style_to_highlight(
                            &style,
                            fg,
                            bg,
                            default_fg,
                            default_bg,
                            if is_selected { Some(sel_bg) } else { None },
                            is_selected,
                        );
                        h.background_color = None;
                        h
                    };

                    if highlight != HighlightStyle::default() {
                        highlights.push((start..end, highlight));
                    }

                    if is_wide {
                        cell_iter.next();
                    }
                    col_idx += col_span;
                }

                // Emit the final bg run, covering through num_cols.
                bg_runs.push(super::data::BgRun::new(run_start, num_cols, run_color));
                row.set_dirty(false)
                    .expect("failed to reset render row dirty tracking");

                rows.push(RowData {
                    text,
                    highlights,
                    bg_runs,
                });
            }

            new_rows = Some(rows);
            self.presentation_dirty = false;
        }

        // Reset dirty state so the next update reports only actual changes.
        snapshot
            .set_dirty(Dirty::Clean)
            .expect("failed to reset render state dirty tracking");

        (new_rows, cached_cursor, default_fg, default_bg)
    }

    fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = match event.delta {
            ScrollDelta::Lines(lines) => lines.y as isize,
            ScrollDelta::Pixels(pixels) => {
                let y = f32::from(pixels.y);
                -(y / self.cell_metrics.height).round() as isize
            }
        };
        if delta == 0 {
            return;
        }

        if self.is_mouse_tracking() {
            use libghostty_vt::mouse::Button;
            let button = if delta > 0 {
                Button::Four
            } else {
                Button::Five
            };
            let pos = self.local_pos(event.position);
            for _ in 0..delta.abs().min(5) {
                self.encode_mouse_event(
                    MouseAction::Press,
                    Some(button),
                    pos,
                    &event.modifiers,
                    cx,
                );
                self.encode_mouse_event(
                    MouseAction::Release,
                    Some(button),
                    pos,
                    &event.modifiers,
                    cx,
                );
            }
            return;
        }

        self.terminal.scroll_viewport(ScrollViewport::Delta(delta));
        cx.notify();
    }

    fn local_pos(&self, window_pos: Point<Pixels>) -> Point<Pixels> {
        point(
            window_pos.x - self.view_bounds.origin.x,
            window_pos.y - self.view_bounds.origin.y,
        )
    }

    fn encode_mouse_event(
        &mut self,
        action: MouseAction,
        button: Option<Button>,
        pos: Point<Pixels>,
        mods: &Modifiers,
        cx: &mut Context<Self>,
    ) {
        let mut m = Mods::empty();
        if mods.control {
            m |= Mods::CTRL;
        }
        if mods.alt {
            m |= Mods::ALT;
        }
        if mods.shift {
            m |= Mods::SHIFT;
        }
        if mods.platform {
            m |= Mods::SUPER;
        }

        let x = f32::from(pos.x);
        let y = f32::from(pos.y);

        self.mouse_event.set_action(action);
        self.mouse_event.set_button(button);
        self.mouse_event.set_mods(m);
        self.mouse_event.set_position(mouse::Position { x, y });

        self.mouse_encoder.set_options_from_terminal(&self.terminal);
        self.mouse_encoder
            .set_any_button_pressed(self.buttons_pressed > 0)
            .set_track_last_cell(true)
            .set_size(libghostty_vt::mouse::EncoderSize {
                screen_width: (self.current_cols as f32 * self.cell_metrics.width) as u32,
                screen_height: (self.current_rows as f32 * self.cell_metrics.height) as u32,
                cell_width: self.cell_metrics.width as u32,
                cell_height: self.cell_metrics.height as u32,
                padding_top: PADDING_TOP as u32,
                padding_bottom: 0,
                padding_left: PADDING_LEFT as u32,
                padding_right: 0,
            });

        let mut buf = Vec::with_capacity(64);
        if self
            .mouse_encoder
            .encode_to_vec(&self.mouse_event, &mut buf)
            .is_ok()
            && !buf.is_empty()
        {
            self.write_input(&buf, cx);
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gpui_mods = &event.keystroke.modifiers;
        let key_str = &event.keystroke.key;

        if gpui_mods.platform && key_str == "c" {
            self.copy_selection(cx);
            return;
        }

        if gpui_mods.platform && key_str == "v" {
            self.paste_from_clipboard(cx);
            return;
        }

        let key_char = event.keystroke.key_char.as_deref();

        let mut mods = Mods::empty();
        if gpui_mods.control {
            mods |= Mods::CTRL;
        }
        if gpui_mods.alt {
            mods |= Mods::ALT;
        }
        if gpui_mods.shift {
            mods |= Mods::SHIFT;
        }
        if gpui_mods.platform {
            mods |= Mods::SUPER;
        }

        if mods.contains(Mods::CTRL) && !mods.contains(Mods::ALT) && !mods.contains(Mods::SUPER) {
            if let Some(byte) = ctrl_char(key_str) {
                self.write_input(&[byte], cx);
                cx.notify();
                return;
            }
        }

        if let Some(mapped_key) = gpui_key_to_ghostty(key_str) {
            // The unshifted codepoint is the character the key produces with no
            // modifiers (required by the Kitty keyboard protocol to identify keys
            // independent of shift state). Single-character keys have a natural
            // codepoint; named keys (arrows, F-keys, etc.) use NUL.
            let unshifted_codepoint = if key_str.chars().count() == 1 {
                key_str.chars().next().unwrap_or('\0')
            } else {
                '\0'
            };

            // Consumed mods are modifiers the platform text input already
            // accounted for when producing the UTF-8 text. Shift is consumed
            // for character keys (it turns 'a' → 'A'); nothing is consumed for
            // special keys.
            let mut consumed = Mods::empty();
            if unshifted_codepoint != '\0' && mods.contains(Mods::SHIFT) {
                consumed |= Mods::SHIFT;
            }

            self.key_event.set_action(if event.is_held {
                KeyAction::Repeat
            } else {
                KeyAction::Press
            });
            self.key_event.set_key(mapped_key);
            self.key_event.set_mods(mods);
            self.key_event.set_consumed_mods(consumed);
            self.key_event.set_unshifted_codepoint(unshifted_codepoint);
            self.key_event.set_utf8(key_char.map(|s| s.to_string()));
            self.key_encoder.set_options_from_terminal(&self.terminal);

            let mut buf = Vec::with_capacity(64);
            if self
                .key_encoder
                .encode_to_vec(&self.key_event, &mut buf)
                .is_ok()
                && !buf.is_empty()
            {
                self.write_input(&buf, cx);
            }
        } else if let Some(ch) = key_char {
            self.write_input(ch.as_bytes(), cx);
        } else if let Some(ch) = key_str.chars().next() {
            self.write_input(&[ch as u8], cx);
        }

        cx.notify();
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.buttons_pressed = self.buttons_pressed.saturating_add(1);

        if self.is_mouse_tracking() {
            let button = Self::gpui_button_to_ghostty(event.button);
            let pos = self.local_pos(event.position);
            self.encode_mouse_event(MouseAction::Press, button, pos, &event.modifiers, cx);
            return;
        }

        if event.button != MouseButton::Left {
            return;
        }
        let pos = GridPos::from_pixel(self.local_pos(event.position), &self.cell_metrics);
        self.selecting = true;
        self.selection = Some(Selection::new(pos, pos));
        self.presentation_dirty = true;
        cx.notify();
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_mouse_tracking() {
            if event.pressed_button.is_some() {
                let button = event.pressed_button.and_then(Self::gpui_button_to_ghostty);
                let pos = self.local_pos(event.position);
                self.encode_mouse_event(MouseAction::Motion, button, pos, &event.modifiers, cx);
            }
            return;
        }

        if !self.selecting || !event.dragging() {
            return;
        }
        let pos = GridPos::from_pixel(self.local_pos(event.position), &self.cell_metrics);
        if let Some(ref mut sel) = self.selection {
            if sel.end != pos {
                sel.end = pos;
                self.presentation_dirty = true;
                cx.notify();
            }
        }
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.buttons_pressed == 0 {
            return;
        }
        self.buttons_pressed = self.buttons_pressed.saturating_sub(1);

        if self.is_mouse_tracking() {
            let button = Self::gpui_button_to_ghostty(event.button);
            let pos = self.local_pos(event.position);
            self.encode_mouse_event(MouseAction::Release, button, pos, &event.modifiers, cx);
            return;
        }

        if event.button != MouseButton::Left {
            return;
        }
        self.selecting = false;
        if let Some(ref sel) = self.selection {
            if sel.is_empty() {
                self.selection = None;
                self.presentation_dirty = true;
                cx.notify();
            }
        }
    }

    fn copy_selection(&self, cx: &mut Context<Self>) {
        let Some(ref sel) = self.selection else {
            return;
        };
        let (start, end) = sel.ordered();
        if start.row >= self.current_rows as usize || end.row >= self.current_rows as usize {
            return;
        }

        let point = |pos: GridPos| {
            TerminalPoint::Viewport(PointCoordinate {
                x: pos.col.min(self.current_cols as usize - 1) as u16,
                y: pos.row as u32,
            })
        };
        let (Ok(start), Ok(end)) = (
            self.terminal.grid_ref(point(start)),
            self.terminal.grid_ref(point(end)),
        ) else {
            return;
        };
        let selection = TerminalSelection::new(start, end, false);
        let options = SelectionFormatOptions::new()
            .with_selection(&selection)
            .with_emit_format(Format::Plain)
            .with_unwrap(true)
            .with_trim(true);
        let Ok(Some(bytes)) = self.terminal.format_selection_alloc(None, options) else {
            return;
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();

        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let text = item.text().unwrap_or_default();
        if text.is_empty() {
            return;
        }
        self.write_input(text.as_bytes(), cx);
        cx.notify();
    }

    fn gpui_button_to_ghostty(button: MouseButton) -> Option<Button> {
        match button {
            MouseButton::Left => Some(Button::Left),
            MouseButton::Right => Some(Button::Right),
            MouseButton::Middle => Some(Button::Middle),
            _ => None,
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.maybe_resize(window, cx);
        let (new_rows, cached_cursor, fg, bg) = self.render_terminal();

        if let Some(rows) = new_rows {
            self.cached_rows = rows;
        }
        self.cached_cursor = cached_cursor;

        if !self.custom_theme_colors {
            self.last_default_fg = fg;
            self.last_default_bg = bg;
        }

        let row_elements =
            build_row_elements(&self.cached_rows, &self.cell_metrics, self.last_default_bg);

        // Capture the view's bounds (origin + size) during prepaint. A full-size
        // flex child is used so bounds.size reflects the actual allocated area.
        // We previously used a zero-height child and computed size from the window
        // width, which was wrong for non-rightmost panes in a split.
        let weak = cx.entity().downgrade();
        let origin_capture = canvas(
            move |bounds, _window, cx| {
                weak.update(cx, |view, cx| {
                    if view.view_bounds != bounds {
                        view.view_bounds = bounds;
                        cx.notify();
                    }
                })
                .ok();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .size_full()
            .overflow_x_hidden()
            .bg(bg)
            .text_color(fg)
            .font_family(self.font_family.clone())
            .text_size(self.cell_metrics.font_size)
            .flex()
            .flex_col()
            .pt(px(PADDING_TOP))
            .pl(px(PADDING_LEFT))
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_scroll_wheel(cx.listener(Self::handle_scroll))
            .child(origin_capture)
            .children(row_elements)
    }
}

pub enum TerminalEvent {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

impl EventEmitter<TerminalEvent> for TerminalView {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn project_rows(view: &mut TerminalView) -> bool {
        let (rows, _, _, _) = view.render_terminal();
        if let Some(rows) = rows {
            view.cached_rows = rows;
            true
        } else {
            false
        }
    }

    fn cell_position(view: &TerminalView, col: usize) -> Point<Pixels> {
        point(
            px(PADDING_LEFT + (col as f32 + 0.5) * view.cell_metrics.width),
            px(PADDING_TOP + view.cell_metrics.height / 2.0),
        )
    }

    #[gpui::test]
    fn named_control_keys_emit_terminal_escape_sequences(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        let input = Rc::new(RefCell::new(Vec::new()));
        let _subscribe = cx.update(|cx| {
            let input = input.clone();
            cx.subscribe(&terminal, move |_, event, _| {
                if let TerminalEvent::Input(bytes) = event {
                    input.borrow_mut().extend_from_slice(bytes);
                }
            })
        });
        let cx = cx.add_empty_window();
        for (key, expected) in [("up", "\u{1b}[1;5A"), ("left", "\u{1b}[1;5D")] {
            input.borrow_mut().clear();
            cx.update(|window, cx| {
                terminal.update(cx, |view, cx| {
                    view.handle_key_down(
                        &KeyDownEvent {
                            keystroke: Keystroke {
                                key: key.to_string(),
                                modifiers: Modifiers {
                                    control: true,
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                            is_held: false,
                            prefer_character_input: false,
                        },
                        window,
                        cx,
                    );
                })
            });
            assert_eq!(input.borrow().as_slice(), expected.as_bytes(), "ctrl-{key}");
        }
    }

    #[gpui::test]
    fn terminal_protocol_callbacks_write_after_view_construction(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        let input = Rc::new(RefCell::new(Vec::new()));
        let _subscribe = cx.update(|cx| {
            let input = input.clone();
            cx.subscribe(&terminal, move |_, event, _| {
                if let TerminalEvent::Input(bytes) = event {
                    input.borrow_mut().extend_from_slice(bytes);
                }
            })
        });
        terminal.update(cx, |view, cx| view.ingest(b"\x1b[6n\x1b[18t", cx));
        assert_eq!(input.borrow().as_slice(), b"\x1b[1;1R\x1b[8;24;80t");
    }

    #[gpui::test]
    fn mouse_tracking_ignores_releases_without_a_terminal_press(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        let input = Rc::new(RefCell::new(Vec::new()));
        let _subscribe = cx.update(|cx| {
            let input = input.clone();
            cx.subscribe(&terminal, move |_, event, _| {
                if let TerminalEvent::Input(bytes) = event {
                    input.borrow_mut().extend_from_slice(bytes);
                }
            })
        });
        terminal.update(cx, |view, cx| {
            view.ingest(b"\x1b[?1002h\x1b[?1006h", cx);
            assert!(view.is_mouse_tracking());
        });
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            terminal.update(cx, |view, cx| {
                view.handle_mouse_up(
                    &MouseUpEvent {
                        button: MouseButton::Left,
                        position: cell_position(view, 0),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            })
        });
        assert!(
            input.borrow().is_empty(),
            "a release over another pane must not reach the PTY"
        );
        cx.update(|window, cx| {
            terminal.update(cx, |view, cx| {
                view.handle_mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Left,
                        position: cell_position(view, 0),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                view.handle_mouse_up(
                    &MouseUpEvent {
                        button: MouseButton::Left,
                        position: cell_position(view, 0),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            })
        });
        assert_eq!(input.borrow().as_slice(), b"\x1b[<0;1;1M\x1b[<0;1;1m");
    }

    #[gpui::test]
    fn unchanged_terminal_frames_reuse_rows(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        terminal.update(cx, |view, cx| {
            view.ingest(b"hello", cx);
            assert!(project_rows(view));
            assert!(view.cached_rows[0].text.starts_with("hello"));
            assert!(!project_rows(view));
        });
    }

    #[test]
    fn ghostty_snapshot_dirty_state_can_be_reset() {
        let terminal = Terminal::new(TerminalOptions {
            cols: 8,
            rows: 3,
            max_scrollback: 0,
        })
        .unwrap();
        let mut render_state = RenderState::new().unwrap();
        {
            let snapshot = render_state.update(&terminal).unwrap();
            snapshot.set_dirty(Dirty::Clean).unwrap();
            assert_eq!(snapshot.dirty().unwrap(), Dirty::Clean);
        }
        assert_eq!(
            render_state.update(&terminal).unwrap().dirty().unwrap(),
            Dirty::Clean
        );
    }

    #[gpui::test]
    fn selection_reprojects_and_notifies_an_idle_terminal(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        terminal.update(cx, |view, cx| {
            view.ingest(b"hello", cx);
            assert!(project_rows(view));
        });
        let notifications = Rc::new(Cell::new(0));
        let _observe = cx.update(|cx| {
            let notifications = notifications.clone();
            cx.observe(&terminal, move |_, _| {
                notifications.set(notifications.get() + 1)
            })
        });
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            terminal.update(cx, |view, cx| {
                view.handle_mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Left,
                        position: cell_position(view, 0),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            })
        });
        assert_eq!(notifications.get(), 1);
        cx.update(|window, cx| {
            terminal.update(cx, |view, cx| {
                assert!(
                    project_rows(view),
                    "selection must refresh a clean VT snapshot"
                );
                assert!(view.cached_rows[0]
                    .bg_runs
                    .iter()
                    .any(|run| run.start_col == 0
                        && run.end_col == 1
                        && run.color == hsla_from_rgb(240, 153, 179)));
                assert!(!project_rows(view));
                view.handle_mouse_up(
                    &MouseUpEvent {
                        button: MouseButton::Left,
                        position: cell_position(view, 0),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert!(
                    project_rows(view),
                    "releasing an empty selection clears its highlight"
                );
                assert!(!view.cached_rows[0]
                    .bg_runs
                    .iter()
                    .any(|run| run.color == hsla_from_rgb(240, 153, 179)));
            })
        });
        assert_eq!(notifications.get(), 2);
    }

    #[gpui::test]
    fn theme_reprojects_and_notifies_an_idle_terminal(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        terminal.update(cx, |view, cx| {
            view.ingest(b"hello", cx);
            assert!(project_rows(view));
        });
        let notifications = Rc::new(Cell::new(0));
        let _observe = cx.update(|cx| {
            let notifications = notifications.clone();
            cx.observe(&terminal, move |_, _| {
                notifications.set(notifications.get() + 1)
            })
        });
        terminal.update(cx, |view, cx| {
            view.set_theme_colors(rgb(0x336699), rgb(0x112233), cx)
        });
        assert_eq!(notifications.get(), 1);
        terminal.update(cx, |view, _| {
            assert!(project_rows(view), "theme must refresh a clean VT snapshot");
            assert_eq!(
                view.cached_rows[0].highlights[0].1.color,
                Some(hsla_from_rgb(0x33, 0x66, 0x99))
            );
            assert_eq!(
                view.cached_rows[0].bg_runs.last().unwrap().color,
                hsla_from_rgb(0x11, 0x22, 0x33)
            );
            assert!(!project_rows(view));
        });
    }

    #[gpui::test]
    fn clipboard_uses_terminal_columns_and_preserves_graphemes(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        for (text, start, end, expected) in [
            ("café", 3, 3, "é"),
            ("A界B", 1, 2, "界"),
            ("A界B", 2, 2, "界"),
            ("A界B", 3, 3, "B"),
            ("A界B", 0, 3, "A界B"),
            ("e\u{301}!", 0, 0, "e\u{301}"),
            ("hello", 1, 3, "ell"),
        ] {
            terminal.update(cx, |view, cx| {
                view.clear(cx);
                view.ingest(text.as_bytes(), cx);
                assert!(project_rows(view));
                view.selection = Some(Selection::new(GridPos::new(start, 0), GridPos::new(end, 0)));
                view.copy_selection(cx);
            });
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(expected.to_string()),
                "{text:?} at columns {start}..={end}"
            );
        }
    }

    #[gpui::test]
    fn rendered_rows_preserve_full_unicode_cell_text(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        for text in ["café", "A界B", "e\u{301}!"] {
            terminal.update(cx, |view, cx| {
                view.clear(cx);
                view.ingest(text.as_bytes(), cx);
                assert!(project_rows(view));
                assert_eq!(view.cached_rows[0].text.trim_end_matches(' '), text);
            });
        }
    }

    #[gpui::test]
    fn selection_of_a_wide_character_tail_highlights_the_whole_character(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        terminal.update(cx, |view, cx| {
            view.ingest("A界B".as_bytes(), cx);
            view.selection = Some(Selection::new(GridPos::new(2, 0), GridPos::new(2, 0)));
            assert!(project_rows(view));
            assert!(view.cached_rows[0]
                .bg_runs
                .iter()
                .any(|run| run.start_col == 1
                    && run.end_col == 3
                    && run.color == hsla_from_rgb(240, 153, 179)));
        });
    }

    #[gpui::test]
    fn clipboard_preserves_multiline_selection_and_trims_row_padding(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        terminal.update(cx, |view, cx| {
            view.ingest("café\r\n界!\r\nlast".as_bytes(), cx);
            assert!(project_rows(view));
            view.selection = Some(Selection::new(GridPos::new(2, 2), GridPos::new(3, 0)));
            view.copy_selection(cx);
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("é\n界!\nlas".to_string())
        );
    }

    #[gpui::test]
    fn clipboard_preserves_blank_lines_and_ignores_outside_rows(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        terminal.update(cx, |view, cx| {
            view.ingest(b"x  \r\n\r\ny", cx);
            view.selection = Some(Selection::new(GridPos::new(0, 0), GridPos::new(79, 2)));
            view.copy_selection(cx);
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("x\n\ny".to_string())
        );
        cx.write_to_clipboard(ClipboardItem::new_string("unchanged".to_string()));
        terminal.update(cx, |view, cx| {
            view.selection = Some(Selection::new(
                GridPos::new(0, 0),
                GridPos::new(0, DEFAULT_ROWS as usize),
            ));
            view.copy_selection(cx);
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("unchanged".to_string())
        );
    }

    #[gpui::test]
    fn unpressed_mouse_move_does_not_extend_selection(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            terminal.update(cx, |view, cx| {
                view.ingest(b"hello", cx);
                assert!(project_rows(view));
                view.handle_mouse_down(
                    &MouseDownEvent {
                        button: MouseButton::Left,
                        position: cell_position(view, 0),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                project_rows(view);
                view.handle_mouse_move(
                    &MouseMoveEvent {
                        position: cell_position(view, 3),
                        pressed_button: Some(MouseButton::Left),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert!(project_rows(view), "dragging must refresh the selection");
                assert!(view.cached_rows[0]
                    .bg_runs
                    .iter()
                    .any(|run| run.start_col == 0
                        && run.end_col == 4
                        && run.color == hsla_from_rgb(240, 153, 179)));
                view.handle_mouse_move(
                    &MouseMoveEvent {
                        position: cell_position(view, 8),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert_eq!(view.selection.unwrap().end.col, 3);
                assert!(!project_rows(view));
            })
        });
    }

    struct TerminalFixture(Entity<TerminalView>);

    impl Render for TerminalFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().h(px(100.)).child(self.0.clone()))
                .child(div().id("composer").flex_1())
        }
    }

    #[gpui::test]
    fn mouse_release_over_composer_finishes_terminal_selection(cx: &mut TestAppContext) {
        let terminal = cx.new(TerminalView::new_remote);
        let (_, cx) = cx.add_window_view(|_, _| TerminalFixture(terminal.clone()));
        let start = terminal.read_with(cx, |view, _| cell_position(view, 0));
        let end = terminal.read_with(cx, |view, _| cell_position(view, 3));
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(
            point(end.x, px(150.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        terminal.read_with(cx, |view, _| {
            assert!(
                !view.selecting,
                "release outside the terminal must finish the drag"
            );
            assert_eq!(view.buttons_pressed, 0);
        });
        cx.simulate_mouse_move(start, None, Modifiers::default());
        terminal.read_with(cx, |view, _| assert_eq!(view.selection.unwrap().end.col, 3));
    }
}
