use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::*;
use libghostty_vt::key::{Action as KeyAction, Encoder as KeyEncoder, Event as KeyEvent, Mods};
use libghostty_vt::mouse::{
    self, Action as MouseAction, Button, Encoder as MouseEncoder, Event as MouseEvent,
};
use libghostty_vt::render::{CellIterator, CursorViewport, Dirty, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType,
    PrimaryDeviceAttributes, ScrollViewport, SecondaryDeviceAttributes, SizeReportSize,
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
    terminal: Box<Terminal<'static, 'static>>,
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

    fn build(terminal: Terminal<'static, 'static>, cx: &mut Context<Self>) -> Self {
        // Box the terminal before registering handlers. Each on_* call stores
        // &self.vtable (a raw pointer) in the C library as userdata. The terminal
        // must not move after that point — boxing it here ensures the vtable address
        // stays stable when the view struct itself is moved.
        let mut terminal = Box::new(terminal);
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
                        [
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
        cx.notify();
    }

    pub fn set_theme_colors(&mut self, foreground: gpui::Rgba, background: gpui::Rgba) {
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

        if matches!(dirty, Dirty::Full | Dirty::Partial) {
            let num_cols = snapshot.cols().unwrap_or(self.current_cols) as usize;
            let num_rows = snapshot.rows().unwrap_or(self.current_rows) as usize;

            let mut rows: Vec<RowData> = Vec::with_capacity(num_rows);
            let mut row_iter = self
                .row_iter
                .update(&snapshot)
                .expect("row iterator update failed");

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

                    let graphemes = cell.graphemes().unwrap_or_default();
                    let ch = graphemes.first().copied().unwrap_or(' ');
                    let start = text.len();
                    text.push(ch);

                    let is_wide = matches!(cell_wide, Some(CellWide::Wide));
                    if is_wide {
                        text.push(' ');
                    }

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
                        sel.contains(GridPos {
                            col: col_idx,
                            row: row_idx,
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

                    col_idx += col_span;
                }

                // Emit the final bg run, covering through num_cols.
                bg_runs.push(super::data::BgRun::new(run_start, num_cols, run_color));

                rows.push(RowData {
                    text,
                    highlights,
                    bg_runs,
                });
            }

            new_rows = Some(rows);
        }

        // Reset dirty state so the next update reports only actual changes.
        let _ = snapshot.set_dirty(Dirty::Clean);

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

        if !self.selecting {
            return;
        }
        let pos = GridPos::from_pixel(self.local_pos(event.position), &self.cell_metrics);
        if let Some(ref mut sel) = self.selection {
            sel.end = pos;
        }
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            }
        }
    }

    fn copy_selection(&self, cx: &mut Context<Self>) {
        let Some(ref sel) = self.selection else {
            return;
        };
        let (start, end) = sel.ordered();
        let num_rows = self.cached_rows.len();
        if start.row >= num_rows || end.row >= num_rows {
            return;
        }

        let mut text = String::new();
        for row_idx in start.row..=end.row.min(num_rows - 1) {
            let row = &self.cached_rows[row_idx];
            let row_text = row.text.trim_end_matches(' ');
            if row_idx == start.row && row_idx == end.row {
                let s = start.col.min(row_text.len());
                let e = (end.col + 1).min(row_text.len());
                if s < e {
                    text.push_str(&row_text[s..e]);
                }
            } else if row_idx == start.row {
                let s = start.col.min(row_text.len());
                if s < row_text.len() {
                    text.push_str(&row_text[s..]);
                }
                text.push('\n');
            } else if row_idx == end.row {
                let e = (end.col + 1).min(row_text.len());
                if e > 0 {
                    text.push_str(&row_text[..e]);
                }
            } else {
                text.push_str(row_text);
                text.push('\n');
            }
        }

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
