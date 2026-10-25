use super::session_data::save_clipboard_image;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextInputTarget {
    Composer,
    BrowserUrl,
    Rename,
    SidebarSearch,
    ModelSearch,
}

pub(super) struct ComposerElement {
    pub(super) shell: Entity<DesktopShell>,
    pub(super) target: TextInputTarget,
}

pub(super) struct ComposerPrepaint {
    text: String,
    lines: Vec<WrappedLine>,
    font: gpui::Font,
    font_size: Pixels,
    cursor: Option<Point<Pixels>>,
    line_height: Pixels,
    scroll_offset: Pixels,
    cursor_offset: usize,
    cursor_downstream: bool,
}

pub(super) struct TextInputLayout {
    target: TextInputTarget,
    text: SharedString,
    lines: Rc<[WrappedLine]>,
    font: gpui::Font,
    font_size: Pixels,
    line_height: Pixels,
    bounds: Bounds<Pixels>,
    selection: Range<usize>,
    scroll_offset: Pixels,
    cursor_offset: usize,
    cursor_downstream: bool,
    focused: bool,
}

impl TextInputLayout {
    fn bounds_for_range(
        &self,
        target: TextInputTarget,
        text: &str,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
    ) -> Option<Bounds<Pixels>> {
        if self.target != target || self.text.as_ref() != text {
            return None;
        }
        if text.is_empty() {
            return Some(Bounds::new(
                bounds.origin,
                gpui::size(px(0.), self.line_height),
            ));
        }
        let start = byte_offset_for_utf16(text, range_utf16.start);
        let end = byte_offset_for_utf16(text, range_utf16.end);
        let range = start.min(end)..start.max(end);
        if range.is_empty() && range.start == self.cursor_offset && self.cursor_downstream {
            let position = cursor_position_with_affinity(
                &self.lines,
                text,
                range.start,
                self.line_height,
                true,
            )?;
            return Some(Bounds::new(
                bounds.origin + point(position.x, position.y - self.scroll_offset),
                gpui::size(Pixels::ZERO, self.line_height),
            ));
        }
        let start_position = cursor_position(&self.lines, text, range.start, self.line_height)?;
        let mut line_start = 0;
        for line in self.lines.iter() {
            let line_end = line_start + line.len();
            if range.start <= line_end {
                // AppKit requests the first rectangle when a range spans visual rows.
                let row_end = line
                    .wrap_boundaries()
                    .iter()
                    .map(|boundary| line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index)
                    .find(|&index| index >= range.start - line_start)
                    .unwrap_or(line.len());
                let end_position = cursor_position(
                    &self.lines,
                    text,
                    range.end.min(line_start + row_end),
                    self.line_height,
                )?;
                return Some(Bounds::new(
                    point(
                        bounds.left() + start_position.x.min(end_position.x),
                        bounds.top() + start_position.y - self.scroll_offset,
                    ),
                    gpui::size((end_position.x - start_position.x).abs(), self.line_height),
                ));
            }
            line_start = line_end + 1;
        }
        None
    }
}

fn normalized_browser_url_range(text: &str, range: &Range<usize>) -> Range<usize> {
    let start = snap_to_char_boundary(text, range.start);
    let end = snap_to_char_boundary(text, range.end);
    if start <= end {
        start..end
    } else {
        end..start
    }
}

impl DesktopShell {
    pub(super) fn composer_expanded_max_height(&self, window: &Window) -> f32 {
        let dock_height = if self.layout.workbench_collapsed
            && self.work_tabs.active().kind == WorkTabKind::Terminal
        {
            TERMINAL_EXPANDED_HEIGHT
        } else {
            0.
        };
        let height = f32::from(window.bounds().size.height) - PANE_HEADER_HEIGHT - dock_height;
        let chrome_height = self
            .composer_area_bounds
            .zip(self.composer_bounds)
            .map(|(area, input)| {
                f32::from((area.size.height - input.size.height).max(Pixels::ZERO))
            })
            .unwrap_or(104.);
        // Reserve four transcript lines alongside the measured attachments, errors and controls.
        (height * 0.5)
            .min(height - chrome_height - 96.)
            .clamp(96., 360.)
    }

    pub(super) fn toggle_composer_expanded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_preserve_scroll_on_resize =
            self.composer_text_layout.as_ref().is_some_and(|layout| {
                cursor_position_with_affinity(
                    &layout.lines,
                    &self.model.ui.composer,
                    self.composer_cursor_offset(),
                    layout.line_height,
                    self.composer_cursor_downstream,
                )
                .is_some_and(|caret| {
                    caret.y < self.composer_scroll_offset
                        || caret.y + layout.line_height
                            > self.composer_scroll_offset + layout.bounds.size.height
                })
            });
        self.composer_expanded = !self.composer_expanded;
        self.composer_scrollbar_grab_offset = None;
        window.focus(&self.composer_focus_handle, cx);
        cx.notify();
    }

    pub(super) fn render_composer_scrollbar(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let bounds = self.composer_bounds?;
        let (thumb_height, thumb_top) = composer_scrollbar_thumb(
            bounds.size.height,
            self.composer_scroll_max,
            self.composer_scroll_offset,
        )?;
        Some(
            div()
                .id("composer-scrollbar")
                .debug_selector(|| "composer-scrollbar".into())
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(-10.))
                .w(px(10.))
                .cursor(gpui::CursorStyle::Arrow)
                .role(Role::ScrollBar)
                .aria_label("Draft scroll")
                .aria_orientation(gpui::Orientation::Vertical)
                .aria_min_numeric_value(0.)
                .aria_max_numeric_value(100.)
                .aria_numeric_value(f64::from(
                    (self.composer_scroll_offset / self.composer_scroll_max) * 100.,
                ))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(Self::composer_scrollbar_mouse_down),
                )
                .child(
                    div()
                        .id("composer-scrollbar-thumb")
                        .debug_selector(|| "composer-scrollbar-thumb".into())
                        .absolute()
                        .top(thumb_top)
                        .right(px(2.))
                        .w(px(5.))
                        .h(thumb_height)
                        .rounded(px(2.5))
                        .bg(theme_rgb(&self.theme, "text.muted").opacity(0.48)),
                )
                .into_any_element(),
        )
    }

    fn composer_scrollbar_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.composer_bounds else {
            return;
        };
        let Some((height, top)) = composer_scrollbar_thumb(
            bounds.size.height,
            self.composer_scroll_max,
            self.composer_scroll_offset,
        ) else {
            return;
        };
        let y = event.position.y - bounds.top();
        let grab = if y >= top && y <= top + height {
            y - top
        } else {
            height * 0.5
        };
        self.composer_scrollbar_grab_offset = Some(grab);
        self.composer_is_selecting = false;
        self.update_composer_scrollbar(event.position.y, grab, cx);
        window.prevent_default();
        cx.stop_propagation();
    }

    fn update_composer_scrollbar(
        &mut self,
        pointer_y: Pixels,
        grab: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.composer_bounds else {
            return;
        };
        let Some((height, _)) = composer_scrollbar_thumb(
            bounds.size.height,
            self.composer_scroll_max,
            self.composer_scroll_offset,
        ) else {
            return;
        };
        let travel = bounds.size.height - height;
        if travel > Pixels::ZERO {
            self.composer_scroll_offset = ((pointer_y - bounds.top() - grab) / travel)
                .clamp(0., 1.)
                * self.composer_scroll_max;
            cx.notify();
        }
    }

    pub(super) fn composer_scrollbar_mouse_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(grab) = self.composer_scrollbar_grab_offset {
            if event.dragging() {
                self.update_composer_scrollbar(event.position.y, grab, cx);
                cx.stop_propagation();
            }
        }
    }

    pub(super) fn composer_scrollbar_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.composer_scrollbar_grab_offset = None;
    }

    // Native text services can request candidate bounds before the next paint.
    fn refresh_text_input_layout(&mut self, target: TextInputTarget, window: &Window) {
        let text = self.text_input_value(target);
        let layout = if target == TextInputTarget::Composer {
            self.composer_text_layout
                .as_ref()
                .or(self.text_input_layout.as_ref())
        } else {
            self.text_input_layout.as_ref()
        };
        let Some(layout) = layout else {
            return;
        };
        if layout.target != target {
            return;
        }
        let (text, lines) = if layout.text.as_ref() == text {
            (layout.text.clone(), layout.lines.clone())
        } else {
            let text: SharedString = text.to_owned().into();
            let Ok(lines) = window.text_system().shape_text(
                text.clone(),
                layout.font_size,
                &[TextRun {
                    len: text.len(),
                    font: layout.font.clone(),
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                Some(layout.bounds.size.width),
                (target != TextInputTarget::Composer).then_some(1),
            ) else {
                return;
            };
            (text, lines.into_vec().into())
        };
        let cursor_offset = self.text_input_cursor_offset(target);
        let cursor_downstream =
            target == TextInputTarget::Composer && self.composer_cursor_downstream;
        let scroll_offset = if target == TextInputTarget::Composer {
            let content_height = lines
                .iter()
                .map(|line| line.size(layout.line_height).height)
                .sum::<Pixels>();
            self.composer_scroll_max =
                (content_height - layout.bounds.size.height).max(Pixels::ZERO);
            self.composer_scroll_offset = cursor_position_with_affinity(
                &lines,
                &text,
                cursor_offset,
                layout.line_height,
                cursor_downstream,
            )
            .map(|cursor| {
                revealed_composer_scroll(
                    self.composer_scroll_offset,
                    cursor.y,
                    layout.line_height,
                    layout.bounds.size.height,
                    self.composer_scroll_max,
                )
            })
            .unwrap_or(Pixels::ZERO);
            self.composer_scroll_offset
        } else {
            Pixels::ZERO
        };
        let layout = Rc::new(TextInputLayout {
            target,
            text,
            lines,
            font: layout.font.clone(),
            font_size: layout.font_size,
            line_height: layout.line_height,
            bounds: layout.bounds,
            selection: self.text_input_selection(target),
            scroll_offset,
            cursor_offset,
            cursor_downstream,
            focused: layout.focused,
        });
        if target == TextInputTarget::Composer {
            self.composer_text_layout = Some(layout.clone());
        }
        if self.text_input_focus_handle(target).is_focused(window) {
            self.text_input_layout = Some(layout);
        }
    }

    fn text_input_target(&self, window: &Window) -> TextInputTarget {
        if self.model_picker_open && self.model_picker_focus_handle.is_focused(window) {
            TextInputTarget::ModelSearch
        } else if self.sidebar_search_focus_handle.is_focused(window) {
            TextInputTarget::SidebarSearch
        } else if self.browser_url_focus_handle.is_focused(window) {
            TextInputTarget::BrowserUrl
        } else if self.rename_focus_handle.is_focused(window) {
            TextInputTarget::Rename
        } else {
            TextInputTarget::Composer
        }
    }

    fn text_input_focus_handle(&self, target: TextInputTarget) -> &FocusHandle {
        match target {
            TextInputTarget::Composer => &self.composer_focus_handle,
            TextInputTarget::BrowserUrl => &self.browser_url_focus_handle,
            TextInputTarget::Rename => &self.rename_focus_handle,
            TextInputTarget::SidebarSearch => &self.sidebar_search_focus_handle,
            TextInputTarget::ModelSearch => &self.model_picker_focus_handle,
        }
    }

    fn text_input_value(&self, target: TextInputTarget) -> &str {
        match target {
            TextInputTarget::Composer => &self.model.ui.composer,
            TextInputTarget::BrowserUrl => &self.browser_url,
            TextInputTarget::Rename => &self.rename_draft,
            TextInputTarget::SidebarSearch => &self.sidebar_search,
            TextInputTarget::ModelSearch => &self.model_picker_query,
        }
    }

    fn text_input_selection(&self, target: TextInputTarget) -> Range<usize> {
        match target {
            TextInputTarget::Composer => self.composer_selection.clone(),
            TextInputTarget::BrowserUrl => {
                normalized_browser_url_range(&self.browser_url, &self.browser_url_selection)
            }
            TextInputTarget::Rename => self.rename_selection.clone(),
            TextInputTarget::SidebarSearch => self.sidebar_search_selection.clone(),
            TextInputTarget::ModelSearch => self.model_picker_selection.clone(),
        }
    }

    fn text_input_cursor_offset(&self, target: TextInputTarget) -> usize {
        match target {
            TextInputTarget::Composer => self.composer_cursor_offset(),
            TextInputTarget::BrowserUrl => {
                let selection =
                    normalized_browser_url_range(&self.browser_url, &self.browser_url_selection);
                if self.browser_url_selection_reversed {
                    selection.start
                } else {
                    selection.end
                }
            }
            TextInputTarget::Rename => self.rename_cursor_offset(),
            TextInputTarget::SidebarSearch => {
                if self.sidebar_search_selection_reversed {
                    self.sidebar_search_selection.start
                } else {
                    self.sidebar_search_selection.end
                }
            }
            TextInputTarget::ModelSearch => {
                if self.model_picker_selection_reversed {
                    self.model_picker_selection.start
                } else {
                    self.model_picker_selection.end
                }
            }
        }
    }

    fn set_text_input_selection(&mut self, target: TextInputTarget, selection: Range<usize>) {
        match target {
            TextInputTarget::Composer => self.composer_selection = selection,
            TextInputTarget::BrowserUrl => {
                self.browser_url_selection =
                    normalized_browser_url_range(&self.browser_url, &selection)
            }
            TextInputTarget::Rename => self.rename_selection = selection,
            TextInputTarget::SidebarSearch => self.sidebar_search_selection = selection,
            TextInputTarget::ModelSearch => self.model_picker_selection = selection,
        }
    }

    pub(super) fn text_undo(&mut self, _: &TextUndo, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_composer_edit(false, window, cx);
    }

    pub(super) fn text_redo(&mut self, _: &TextRedo, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_composer_edit(true, window, cx);
    }

    fn restore_composer_edit(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer_focus_handle.is_focused(window) {
            cx.propagate();
            return;
        }
        if !self.task_drafts.edits.restore(
            redo,
            &mut self.model.ui.composer,
            &mut self.composer_selection,
            &mut self.composer_selection_reversed,
        ) {
            return;
        }
        self.initial_auto_attach = false;
        self.session_requests.cancel_initial_prompt();
        self.composer_marked_range = None;
        self.composer_preferred_column = None;
        self.composer_cursor_downstream = false;
        self.composer_is_selecting = false;
        self.composer_preserve_scroll_on_resize = false;
        self.prompt_history.reset_recall();
        self.sync_slash_menu_after_edit();
        self.sync_mention_menu_after_edit();
        self.refresh_text_input_layout(TextInputTarget::Composer, window);
        self.restart_composer_cursor_blink(cx);
        window.notify_text_input_changed(&self.composer_focus_handle);
    }

    fn record_composer_edit(&mut self, kind: DraftEditKind) {
        self.task_drafts.edits.record(
            &self.model.ui.composer,
            self.composer_selection.clone(),
            self.composer_selection_reversed,
            kind,
            std::time::Instant::now(),
        );
    }

    pub(super) fn text_copy(&mut self, _: &TextCopy, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.text_input_target(window);
        if !self.text_input_focus_handle(target).is_focused(window) {
            cx.propagate();
            return;
        }
        let selection = self.text_input_selection(target);
        if !selection.is_empty() {
            if let Some(text) = self.text_input_value(target).get(selection) {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_owned()));
            }
        }
    }

    pub(super) fn text_cut(&mut self, _: &TextCut, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.text_input_target(window);
        if !self.text_input_focus_handle(target).is_focused(window) {
            cx.propagate();
            return;
        }
        let selection = self.text_input_selection(target);
        if selection.is_empty() {
            return;
        }
        let text = self.text_input_value(target);
        let Some(selected_text) = text.get(selection.clone()) else {
            return;
        };
        let range_utf16 = utf16_offset_for_byte(text, selection.start)
            ..utf16_offset_for_byte(text, selection.end);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(selected_text.to_owned()));
        self.replace_text_in_range(Some(range_utf16), "", window, cx);
        window.notify_text_input_changed(self.text_input_focus_handle(target));
    }

    pub(super) fn text_paste(
        &mut self,
        _: &TextPaste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.text_input_target(window);
        if !self.text_input_focus_handle(target).is_focused(window) {
            cx.propagate();
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        if target == TextInputTarget::Composer {
            for entry in item.entries() {
                if let gpui::ClipboardEntry::Image(image) = entry {
                    match save_clipboard_image(image) {
                        Ok(path) => self.add_attachment(path, cx),
                        Err(error) => {
                            self.attachment_error = Some(error);
                            cx.notify();
                        }
                    }
                    return;
                }
            }
        }
        if let Some(text) = item.text() {
            if target == TextInputTarget::Composer {
                self.task_drafts.edits.end_group();
            }
            self.replace_text_in_range(None, &text, window, cx);
            if target == TextInputTarget::Composer {
                self.task_drafts.edits.end_group();
            }
            window.notify_text_input_changed(self.text_input_focus_handle(target));
        }
    }

    pub(super) fn text_select_all(
        &mut self,
        _: &TextSelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.text_input_target(window);
        if !self.text_input_focus_handle(target).is_focused(window) {
            cx.propagate();
            return;
        }
        let length = self.text_input_value(target).encode_utf16().count();
        self.set_selected_text_range(0..length, window, cx);
        window.notify_text_selection_changed(self.text_input_focus_handle(target));
        cx.notify();
    }

    fn composer_cursor_offset(&self) -> usize {
        if self.composer_selection_reversed {
            self.composer_selection.start
        } else {
            self.composer_selection.end
        }
    }

    /// Replaces the whole composer text and moves the cursor to the end.
    /// Used by slash completion and history recall; unlike
    /// `replace_composer_text` this does not reset the recall position.
    pub(super) fn set_composer_text(&mut self, text: String, cx: &mut Context<Self>) {
        if self.model.ui.composer != text {
            self.record_composer_edit(DraftEditKind::Replace);
        }
        self.composer_preferred_column = None;
        self.composer_cursor_downstream = false;
        self.initial_auto_attach = false;
        self.session_requests.cancel_initial_prompt();
        self.model.ui.set_composer(text);
        let end = self.model.ui.composer.len();
        self.composer_selection = end..end;
        self.composer_selection_reversed = false;
        self.composer_marked_range = None;
        self.sync_mention_menu_after_edit();
        self.restart_composer_cursor_blink(cx);
    }

    pub(super) fn slash_menu_matches(&self) -> Vec<&'static SlashCommandDef> {
        filtered_slash_commands(&self.model.ui.composer)
    }

    pub(super) fn slash_menu_is_open(&self) -> bool {
        !self.slash_menu_dismissed && !self.slash_menu_matches().is_empty()
    }

    /// Re-arms the dismissed slash menu once the composer no longer holds a
    /// slash query, and keeps the selected row inside the filtered list.
    fn sync_slash_menu_after_edit(&mut self) {
        if !self.model.ui.composer.starts_with('/') {
            self.slash_menu_dismissed = false;
            self.slash_menu_index = 0;
        }
    }

    pub(super) fn slash_menu_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.slash_menu_matches().len();
        if count == 0 {
            return;
        }
        let index = self.slash_menu_index.min(count - 1) as isize;
        self.slash_menu_index = (index + delta).rem_euclid(count as isize) as usize;
        cx.notify();
    }

    pub(super) fn complete_slash_selection(&mut self, cx: &mut Context<Self>) {
        let matches = self.slash_menu_matches();
        let Some(def) = matches
            .get(self.slash_menu_index.min(matches.len().saturating_sub(1)))
            .copied()
        else {
            return;
        };
        self.complete_slash_command(def.name, cx);
    }

    pub(super) fn complete_slash_command(&mut self, name: &str, cx: &mut Context<Self>) {
        self.set_composer_text(format!("{name} "), cx);
    }

    /// The `@`-mention token ending at the composer cursor, if any.
    pub(super) fn mention_query(&self) -> Option<(usize, String)> {
        mention_query_at_cursor(&self.model.ui.composer, self.composer_cursor_offset())
    }

    pub(super) fn mention_menu_matches(&self) -> Vec<String> {
        let Some((_, query)) = self.mention_query() else {
            return Vec::new();
        };
        filter_mention_candidates(&mention_file_paths(&self.file_tree_entries), &query)
    }

    pub(super) fn mention_menu_is_open(&self) -> bool {
        !self.mention_menu_dismissed && !self.mention_menu_matches().is_empty()
    }

    /// Re-arms the dismissed mention menu once no `@` token sits at the
    /// cursor, and kicks off a root listing the first time a mention is
    /// typed with no fetched tree.
    fn sync_mention_menu_after_edit(&mut self) {
        if self.mention_query().is_none() {
            self.mention_menu_dismissed = false;
            self.mention_menu_index = 0;
        } else if self.file_tree_entries.is_empty() {
            self.request_dir_listing(None);
        }
    }

    pub(super) fn mention_menu_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.mention_menu_matches().len();
        if count == 0 {
            return;
        }
        let index = self.mention_menu_index.min(count - 1) as isize;
        self.mention_menu_index = (index + delta).rem_euclid(count as isize) as usize;
        cx.notify();
    }

    pub(super) fn complete_mention_selection(&mut self, cx: &mut Context<Self>) {
        let matches = self.mention_menu_matches();
        let Some(path) = matches
            .get(self.mention_menu_index.min(matches.len().saturating_sub(1)))
            .cloned()
        else {
            return;
        };
        self.complete_mention_path(path, cx);
    }

    pub(super) fn complete_mention_path(&mut self, path: String, cx: &mut Context<Self>) {
        let Some((start, _)) = self.mention_query() else {
            return;
        };
        let end = self.composer_cursor_offset();
        self.mention_menu_index = 0;
        self.replace_composer_byte_range(start..end, &format!("@{path} "), cx);
    }

    /// Up/Down prompt-history recall. Only engages for a single-line composer
    /// with no active selection (terminal-style recall); Down past the newest
    /// entry restores the stashed in-progress text. Returns true when the key
    /// press was consumed.
    pub(super) fn recall_prompt_history(&mut self, older: bool, cx: &mut Context<Self>) -> bool {
        if self.model.ui.composer.contains('\n') || !self.composer_selection.is_empty() {
            return false;
        }
        if !older && !self.prompt_history.is_recalling() {
            return false;
        }
        let recalled = if older {
            let current = self.model.ui.composer.clone();
            self.prompt_history.recall_older(&current)
        } else {
            self.prompt_history.recall_newer()
        };
        let Some(text) = recalled else {
            return false;
        };
        self.set_composer_text(text, cx);
        self.sync_slash_menu_after_edit();
        true
    }

    pub(super) fn restart_composer_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.composer_cursor_visible = true;
        self.composer_blink_epoch = self.composer_blink_epoch.wrapping_add(1);
        let epoch = self.composer_blink_epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |shell, cx| shell.toggle_composer_cursor(epoch, cx));
            }
        })
        .detach();
    }

    pub(super) fn stop_composer_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.composer_blink_epoch = self.composer_blink_epoch.wrapping_add(1);
        self.composer_cursor_visible = false;
        cx.notify();
    }

    fn toggle_composer_cursor(&mut self, epoch: usize, cx: &mut Context<Self>) {
        if epoch != self.composer_blink_epoch {
            return;
        }
        self.composer_cursor_visible = !self.composer_cursor_visible;
        cx.notify();
        let next_epoch = self.composer_blink_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |shell, cx| shell.toggle_composer_cursor(next_epoch, cx));
            }
        })
        .detach();
    }

    fn move_composer_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.task_drafts.edits.end_group();
        self.composer_preferred_column = None;
        self.composer_cursor_downstream = false;
        self.composer_selection = offset..offset;
        self.composer_selection_reversed = false;
        self.restart_composer_cursor_blink(cx);
    }

    fn select_composer_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.task_drafts.edits.end_group();
        self.composer_preferred_column = None;
        self.composer_cursor_downstream = false;
        if self.composer_selection_reversed {
            self.composer_selection.start = offset;
        } else {
            self.composer_selection.end = offset;
        }
        if self.composer_selection.end < self.composer_selection.start {
            self.composer_selection = self.composer_selection.end..self.composer_selection.start;
            self.composer_selection_reversed = !self.composer_selection_reversed;
        }
        if self.composer_selection.is_empty() {
            self.composer_selection_reversed = false;
        }
        cx.notify();
    }

    pub(super) fn composer_left(
        &mut self,
        _: &ComposerLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = if self.composer_selection.is_empty() {
            previous_utf8_boundary(&self.model.ui.composer, self.composer_cursor_offset())
        } else {
            self.composer_selection.start
        };
        self.move_composer_to(offset, cx);
    }

    pub(super) fn composer_right(
        &mut self,
        _: &ComposerRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = if self.composer_selection.is_empty() {
            next_utf8_boundary(&self.model.ui.composer, self.composer_cursor_offset())
        } else {
            self.composer_selection.end
        };
        self.move_composer_to(offset, cx);
    }

    pub(super) fn composer_select_left(
        &mut self,
        _: &ComposerSelectLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = previous_utf8_boundary(&self.model.ui.composer, self.composer_cursor_offset());
        self.select_composer_to(offset, cx);
        self.restart_composer_cursor_blink(cx);
    }

    pub(super) fn composer_select_right(
        &mut self,
        _: &ComposerSelectRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = next_utf8_boundary(&self.model.ui.composer, self.composer_cursor_offset());
        self.select_composer_to(offset, cx);
        self.restart_composer_cursor_blink(cx);
    }

    pub(super) fn composer_home(
        &mut self,
        _: &ComposerHome,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_composer_to(0, cx);
    }

    pub(super) fn composer_end(
        &mut self,
        _: &ComposerEnd,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_composer_to(self.model.ui.composer.len(), cx);
    }

    pub(super) fn composer_backspace(
        &mut self,
        _: &ComposerBackspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_marked_range.is_some() {
            self.replace_composer_text(None, "", cx);
            return;
        }
        let mut range = self.composer_selection.clone();
        let kind = if range.is_empty() {
            range.start = previous_utf8_boundary(&self.model.ui.composer, range.start);
            DraftEditKind::Backspace
        } else {
            DraftEditKind::Replace
        };
        if range.is_empty() {
            window.play_system_bell();
            return;
        }
        self.apply_composer_edit(range, "", kind, cx);
    }

    pub(super) fn composer_delete(
        &mut self,
        _: &ComposerDelete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_marked_range.is_some() {
            self.replace_composer_text(None, "", cx);
            return;
        }
        let mut range = self.composer_selection.clone();
        let kind = if range.is_empty() {
            range.end = next_utf8_boundary(&self.model.ui.composer, range.end);
            DraftEditKind::Delete
        } else {
            DraftEditKind::Replace
        };
        if range.is_empty() {
            window.play_system_bell();
            return;
        }
        self.apply_composer_edit(range, "", kind, cx);
    }

    fn replace_composer_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        let current = &self.model.ui.composer;
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(current, range.start)
                    ..byte_offset_for_utf16(current, range.end)
            })
            .or_else(|| self.composer_marked_range.clone())
            .unwrap_or_else(|| self.composer_selection.clone());
        let composing = self.composer_marked_range.is_some();
        let kind = if composing {
            DraftEditKind::Composition
        } else if range.is_empty()
            && !replacement.is_empty()
            && replacement.chars().nth(1).is_none()
        {
            DraftEditKind::Insert
        } else {
            DraftEditKind::Replace
        };
        self.apply_composer_edit(range, replacement, kind, cx);
        if composing {
            self.task_drafts
                .edits
                .end_composition(&self.model.ui.composer);
        }
    }

    /// Replaces a byte range of the composer text and moves the cursor to
    /// the end of the replacement.
    pub(super) fn replace_composer_byte_range(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        self.apply_composer_edit(range, replacement, DraftEditKind::Replace, cx);
    }

    fn apply_composer_edit(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        kind: DraftEditKind,
        cx: &mut Context<Self>,
    ) {
        if self.model.ui.composer.get(range.clone()) != Some(replacement) {
            self.record_composer_edit(kind);
        }
        self.composer_preferred_column = None;
        self.composer_cursor_downstream = false;
        let mut updated = std::mem::take(&mut self.model.ui.composer);
        self.initial_auto_attach = false;
        self.session_requests.cancel_initial_prompt();
        updated.replace_range(range.clone(), replacement);
        let cursor = range.start + replacement.len();
        self.composer_selection = cursor..cursor;
        self.composer_selection_reversed = false;
        self.composer_marked_range = None;
        self.model.ui.set_composer(updated);
        self.task_drafts.edits.finish_edit(&self.composer_selection);
        self.prompt_history.reset_recall();
        self.sync_slash_menu_after_edit();
        self.sync_mention_menu_after_edit();
        self.restart_composer_cursor_blink(cx);
    }

    fn replace_composer_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        self.record_composer_edit(DraftEditKind::Composition);
        self.composer_preferred_column = None;
        self.composer_cursor_downstream = false;
        let current = &self.model.ui.composer;
        self.initial_auto_attach = false;
        self.session_requests.cancel_initial_prompt();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(current, range.start)
                    ..byte_offset_for_utf16(current, range.end)
            })
            .or_else(|| self.composer_marked_range.clone())
            .unwrap_or_else(|| self.composer_selection.clone());
        let mut updated = std::mem::take(&mut self.model.ui.composer);
        updated.replace_range(range.clone(), replacement);
        let replacement_end = range.start + replacement.len();
        self.composer_marked_range =
            (!replacement.is_empty()).then_some(range.start..replacement_end);
        self.composer_selection = new_selected_range_utf16
            .map(|new_range| {
                let start = byte_offset_for_utf16(replacement, new_range.start);
                let end = byte_offset_for_utf16(replacement, new_range.end);
                range.start + start..range.start + end
            })
            .unwrap_or(replacement_end..replacement_end);
        self.composer_selection_reversed = false;
        self.model.ui.set_composer(updated);
        if self.composer_marked_range.is_none() {
            self.task_drafts
                .edits
                .end_composition(&self.model.ui.composer);
        }
        self.prompt_history.reset_recall();
        self.sync_slash_menu_after_edit();
        self.sync_mention_menu_after_edit();
        self.restart_composer_cursor_blink(cx);
    }

    pub(super) fn replace_browser_url_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        self.normalize_browser_url_input_state();
        let current = self.browser_url.clone();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                normalized_browser_url_range(
                    &current,
                    &(byte_offset_for_utf16(&current, range.start)
                        ..byte_offset_for_utf16(&current, range.end)),
                )
            })
            .or_else(|| self.browser_url_marked_range.clone())
            .unwrap_or_else(|| self.browser_url_selection.clone());
        let mut updated = current;
        updated.replace_range(range.clone(), replacement);
        let cursor = range.start + replacement.len();
        self.browser_url_selection = cursor..cursor;
        self.browser_url_selection_reversed = false;
        self.browser_url_marked_range = None;
        self.browser_url = updated;
        cx.notify();
    }

    fn replace_browser_url_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        self.normalize_browser_url_input_state();
        let current = self.browser_url.clone();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                normalized_browser_url_range(
                    &current,
                    &(byte_offset_for_utf16(&current, range.start)
                        ..byte_offset_for_utf16(&current, range.end)),
                )
            })
            .or_else(|| self.browser_url_marked_range.clone())
            .unwrap_or_else(|| self.browser_url_selection.clone());
        let mut updated = current;
        updated.replace_range(range.clone(), replacement);
        let replacement_end = range.start + replacement.len();
        self.browser_url_marked_range =
            (!replacement.is_empty()).then_some(range.start..replacement_end);
        self.browser_url_selection = new_selected_range_utf16
            .map(|new_range| {
                let start = byte_offset_for_utf16(replacement, new_range.start);
                let end = byte_offset_for_utf16(replacement, new_range.end);
                range.start + start..range.start + end
            })
            .unwrap_or(replacement_end..replacement_end);
        self.browser_url_selection_reversed = false;
        self.browser_url = updated;
        cx.notify();
    }

    pub(super) fn browser_url_mouse_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release_browser_focus();
        window.focus(&self.browser_url_focus_handle, cx);
        let end = self.browser_url.len();
        self.browser_url_selection = 0..end;
        self.browser_url_selection_reversed = false;
        cx.notify();
    }

    pub(super) fn browser_url_backspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.normalize_browser_url_input_state();
        if self.browser_url_selection.is_empty() {
            let cursor = self.text_input_cursor_offset(TextInputTarget::BrowserUrl);
            let start = previous_utf8_boundary(&self.browser_url, cursor);
            if start == cursor {
                window.play_system_bell();
                return;
            }
            self.browser_url_selection = start..cursor;
        }
        self.replace_browser_url_text(None, "", cx);
    }

    pub(super) fn browser_url_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.normalize_browser_url_input_state();
        if self.browser_url_selection.is_empty() {
            let cursor = self.text_input_cursor_offset(TextInputTarget::BrowserUrl);
            let end = next_utf8_boundary(&self.browser_url, cursor);
            if end == cursor {
                window.play_system_bell();
                return;
            }
            self.browser_url_selection = cursor..end;
        }
        self.replace_browser_url_text(None, "", cx);
    }

    pub(super) fn browser_url_left(&mut self, cx: &mut Context<Self>) {
        self.normalize_browser_url_input_state();
        let offset = if self.browser_url_selection.is_empty() {
            previous_utf8_boundary(
                &self.browser_url,
                self.text_input_cursor_offset(TextInputTarget::BrowserUrl),
            )
        } else {
            self.browser_url_selection.start
        };
        self.browser_url_selection = offset..offset;
        self.browser_url_selection_reversed = false;
        cx.notify();
    }

    pub(super) fn browser_url_right(&mut self, cx: &mut Context<Self>) {
        self.normalize_browser_url_input_state();
        let offset = if self.browser_url_selection.is_empty() {
            next_utf8_boundary(
                &self.browser_url,
                self.text_input_cursor_offset(TextInputTarget::BrowserUrl),
            )
        } else {
            self.browser_url_selection.end
        };
        self.browser_url_selection = offset..offset;
        self.browser_url_selection_reversed = false;
        cx.notify();
    }

    fn composer_index_for_position(&self, position: Point<Pixels>, window: &mut Window) -> usize {
        let Some(bounds) = self.composer_bounds else {
            return 0;
        };
        let content = &self.model.ui.composer;
        if content.is_empty() {
            return 0;
        }
        let style = window.text_style();
        let run = TextRun {
            len: content.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let cached = self.composer_text_layout.as_ref().filter(|layout| {
            layout.target == TextInputTarget::Composer
                && layout.text.as_ref() == content
                && layout.bounds == bounds
        });
        let line_height = cached
            .map(|layout| layout.line_height)
            .unwrap_or(window.line_height());
        let shaped;
        let lines: &[WrappedLine] = if let Some(layout) = cached {
            &layout.lines
        } else {
            shaped = window
                .text_system()
                .shape_text(
                    content.clone().into(),
                    style.font_size.to_pixels(window.rem_size()),
                    &[run],
                    Some(bounds.size.width),
                    None,
                )
                .unwrap_or_default();
            &shaped
        };
        let relative = point(
            position.x - bounds.left(),
            position.y - bounds.top() + self.composer_scroll_offset,
        );
        let mut search_offset = 0;
        let mut line_y = Pixels::ZERO;
        for line in lines {
            let Some(line_start) = content
                .get(search_offset..)
                .and_then(|text| text.find(line.text.as_ref()))
                .map(|offset| search_offset + offset)
            else {
                continue;
            };
            let height = line.size(line_height).height;
            if relative.y <= line_y + height {
                let local = line
                    .closest_index_for_position(point(relative.x, relative.y - line_y), line_height)
                    .unwrap_or_else(|offset| offset);
                return (line_start + local).min(content.len());
            }
            search_offset = line_start + line.len();
            if content.as_bytes().get(search_offset) == Some(&b'\n') {
                search_offset += 1;
            }
            line_y += height;
        }
        content.len()
    }

    pub(super) fn composer_move_vertical(
        &mut self,
        older: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .composer_text_layout
            .as_ref()
            .is_none_or(|layout| layout.text.as_ref() != self.model.ui.composer)
        {
            self.refresh_text_input_layout(TextInputTarget::Composer, window);
        }
        let Some(layout) = self.composer_text_layout.as_ref().filter(|layout| {
            layout.target == TextInputTarget::Composer
                && layout.text.as_ref() == self.model.ui.composer
        }) else {
            return false;
        };
        let height = layout
            .lines
            .iter()
            .map(|line| line.size(layout.line_height).height)
            .sum::<Pixels>();
        if height <= layout.line_height {
            return false;
        }
        let Some(cursor) = cursor_position_with_affinity(
            &layout.lines,
            &self.model.ui.composer,
            self.composer_cursor_offset(),
            layout.line_height,
            self.composer_cursor_downstream,
        ) else {
            return false;
        };
        let target_y = (cursor.y
            + if older {
                -layout.line_height
            } else {
                layout.line_height
            })
        .clamp(Pixels::ZERO, height - layout.line_height)
            + layout.line_height * 0.5;
        let preferred_column = self.composer_preferred_column.unwrap_or(cursor.x);
        let position =
            layout.bounds.origin + point(preferred_column, target_y - self.composer_scroll_offset);
        let offset = self.composer_index_for_position(position, window);
        let upstream = cursor_position(
            &layout.lines,
            &self.model.ui.composer,
            offset,
            layout.line_height,
        );
        let downstream =
            upstream.is_some_and(|position| position.y + layout.line_height * 0.5 < target_y);
        if extend {
            self.select_composer_to(offset, cx);
            self.restart_composer_cursor_blink(cx);
        } else {
            self.move_composer_to(offset, cx);
        }
        self.composer_preferred_column = Some(preferred_column);
        self.composer_cursor_downstream = downstream;
        self.refresh_text_input_layout(TextInputTarget::Composer, window);
        true
    }

    pub(super) fn composer_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.release_browser_focus();
        window.focus(&self.composer_focus_handle, cx);
        self.composer_is_selecting = true;
        let offset = self.composer_index_for_position(event.position, window);
        if event.modifiers.shift {
            self.select_composer_to(offset, cx);
        } else {
            self.move_composer_to(offset, cx);
        }
        self.restart_composer_cursor_blink(cx);
    }

    pub(super) fn release_browser_focus(&self) {
        if let (Some(portal), Some(owner)) =
            (self.browser_portal.as_ref(), self.current_browser_owner())
        {
            let _ = portal.blur(&owner);
        }
    }

    pub(super) fn composer_mouse_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_is_selecting && event.dragging() {
            if let Some(bounds) = self.composer_bounds {
                let delta = if event.position.y < bounds.top() {
                    event.position.y - bounds.top()
                } else if event.position.y > bounds.bottom() {
                    event.position.y - bounds.bottom()
                } else {
                    Pixels::ZERO
                };
                self.composer_scroll_offset = (self.composer_scroll_offset + delta)
                    .clamp(Pixels::ZERO, self.composer_scroll_max);
            }
            let offset = self.composer_index_for_position(event.position, window);
            self.select_composer_to(offset, cx);
        }
    }

    pub(super) fn composer_scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height()).y;
        let offset =
            (self.composer_scroll_offset - delta).clamp(Pixels::ZERO, self.composer_scroll_max);
        if self.composer_scroll_max > Pixels::ZERO {
            self.composer_scroll_offset = offset;
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(super) fn composer_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.composer_is_selecting = false;
    }
}

impl IntoElement for ComposerElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ComposerElement {
    type RequestLayoutState = ();
    type PrepaintState = ComposerPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name(
            match self.target {
                TextInputTarget::Composer => "composer-input",
                TextInputTarget::BrowserUrl => "browser-url-input",
                TextInputTarget::Rename => "rename-input",
                TextInputTarget::SidebarSearch => "sidebar-search-input",
                TextInputTarget::ModelSearch => "model-picker-search-input",
            }
            .into(),
        ))
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if self.target != TextInputTarget::Composer {
            style.size.height = px(match self.target {
                TextInputTarget::BrowserUrl => 28.,
                _ => 20.,
            })
            .into();
            return (window.request_layout(style, [], cx), ());
        }
        let content: SharedString = self.shell.read(cx).model.ui.composer.clone().into();
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let run = text_style.to_run(content.len());
        let max_height = if self.shell.read(cx).composer_expanded {
            self.shell.read(cx).composer_expanded_max_height(window)
        } else {
            96.
        };
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known
                .width
                .or(match available.width {
                    gpui::AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                })
                .unwrap_or(px(480.));
            let line_count = if content.is_empty() {
                1
            } else {
                window
                    .text_system()
                    .shape_text(
                        content.clone(),
                        font_size,
                        std::slice::from_ref(&run),
                        Some(width),
                        None,
                    )
                    .map(|lines| {
                        lines
                            .iter()
                            .map(|line| line.wrap_boundaries().len() + 1)
                            .sum()
                    })
                    .unwrap_or(1)
            };
            gpui::size(
                width,
                px(composer_input_height(
                    line_count,
                    f32::from(line_height),
                    max_height,
                )),
            )
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let shell = self.shell.read(cx);
        let content = shell.text_input_value(self.target).to_owned();
        let display_text = if content.is_empty() {
            match self.target {
                TextInputTarget::Composer => "Do anything…".to_owned(),
                TextInputTarget::SidebarSearch => "Search sessions…".to_owned(),
                TextInputTarget::ModelSearch => "Filter models…".to_owned(),
                _ => String::new(),
            }
        } else {
            content.clone()
        };
        let cursor = shell
            .text_input_cursor_offset(self.target)
            .min(content.len());
        let style = window.text_style();
        let selection = shell.text_input_selection(self.target).clone();
        let selection_foreground: gpui::Hsla =
            theme_rgb(&shell.theme, "selection.foreground").into();
        let selection_background: gpui::Hsla =
            theme_rgb(&shell.theme, "selection.background").into();
        let mut runs = Vec::new();
        let mut run_start = 0;
        let split_points = if content.is_empty() {
            vec![display_text.len()]
        } else {
            vec![selection.start, selection.end, content.len()]
        };
        for run_end in split_points {
            if run_end <= run_start {
                continue;
            }
            let selected =
                !content.is_empty() && run_start >= selection.start && run_end <= selection.end;
            runs.push(TextRun {
                len: run_end - run_start,
                font: style.font(),
                color: if selected {
                    selection_foreground
                } else {
                    style.color
                },
                background_color: selected.then_some(selection_background),
                underline: None,
                strikethrough: None,
            });
            run_start = run_end;
        }
        let lines = window
            .text_system()
            .shape_text(
                display_text.clone().into(),
                style.font_size.to_pixels(window.rem_size()),
                &runs,
                Some(bounds.size.width),
                (self.target != TextInputTarget::Composer).then_some(1),
            )
            .unwrap_or_default();
        let cursor_text = if content.is_empty() {
            &display_text
        } else {
            &content
        };
        let cursor_downstream =
            self.target == TextInputTarget::Composer && shell.composer_cursor_downstream;
        let caret = cursor_position_with_affinity(
            &lines,
            cursor_text,
            cursor,
            window.line_height(),
            cursor_downstream,
        );
        let mut scroll_offset = Pixels::ZERO;
        if self.target == TextInputTarget::Composer {
            let content_height = lines
                .iter()
                .map(|line| line.size(window.line_height()).height)
                .sum::<Pixels>();
            let max_scroll = (content_height - bounds.size.height).max(Pixels::ZERO);
            scroll_offset = shell.composer_scroll_offset.clamp(Pixels::ZERO, max_scroll);
            let reveal = shell.composer_text_layout.as_ref().is_none_or(|layout| {
                layout.target != self.target
                    || layout.text.as_ref() != content
                    || (layout.bounds != bounds && !shell.composer_preserve_scroll_on_resize)
                    || layout.cursor_offset != cursor
                    || layout.selection != selection
                    || (shell.composer_focus_handle.is_focused(window)
                        && !layout.focused
                        && !shell.composer_preserve_scroll_on_resize)
            });
            if reveal {
                if let Some(caret) = caret {
                    scroll_offset = revealed_composer_scroll(
                        scroll_offset,
                        caret.y,
                        window.line_height(),
                        bounds.size.height,
                        max_scroll,
                    );
                }
            }
        }
        let draw_cursor = selection.is_empty()
            && (self.target != TextInputTarget::Composer || shell.composer_cursor_visible);
        let cursor_position = caret
            .filter(|_| draw_cursor)
            .map(|position| point(position.x, position.y - scroll_offset));
        let prepaint = ComposerPrepaint {
            text: content,
            lines: lines.into_vec(),
            font: style.font(),
            font_size: style.font_size.to_pixels(window.rem_size()),
            cursor: cursor_position,
            line_height: window.line_height(),
            scroll_offset,
            cursor_offset: cursor,
            cursor_downstream,
        };
        if self.target == TextInputTarget::Composer {
            let content_height = prepaint
                .lines
                .iter()
                .map(|line| line.size(prepaint.line_height).height)
                .sum::<Pixels>();
            let line_count = prepaint
                .lines
                .iter()
                .map(|line| line.wrap_boundaries().len() + 1)
                .sum();
            self.shell.update(cx, |shell, cx| {
                let max_scroll = (content_height - bounds.size.height).max(Pixels::ZERO);
                let changed = shell.composer_scroll_offset != scroll_offset
                    || shell.composer_scroll_max != max_scroll
                    || shell.composer_line_count != line_count;
                shell.composer_scroll_offset = scroll_offset;
                shell.composer_scroll_max = max_scroll;
                shell.composer_line_count = line_count;
                if content_height <= px(96.) {
                    shell.composer_expanded = false;
                }
                if changed {
                    cx.defer_in(window, |_, _, cx| cx.notify());
                }
            });
        }
        prepaint
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self
            .shell
            .read(cx)
            .text_input_focus_handle(self.target)
            .clone();
        let target = self.target;
        self.shell.update(cx, |shell, cx| match target {
            TextInputTarget::Composer => {
                let was_compact = composer_settings_are_compact(shell.composer_bounds);
                let height_changed = shell
                    .composer_bounds
                    .is_none_or(|previous| previous.size.height != bounds.size.height);
                shell.composer_bounds = Some(bounds);
                shell.composer_preserve_scroll_on_resize = false;
                if height_changed || composer_settings_are_compact(Some(bounds)) != was_compact {
                    cx.defer_in(window, |_, _, cx| cx.notify());
                }
            }
            TextInputTarget::BrowserUrl => shell.browser_url_bounds = Some(bounds),
            TextInputTarget::Rename => {}
            TextInputTarget::SidebarSearch => {}
            TextInputTarget::ModelSearch => {}
        });
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.shell.clone()),
            cx,
        );
        let selection = self.shell.read(cx).text_input_selection(self.target);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            let mut line_y = bounds.top() - prepaint.scroll_offset;
            for line in &prepaint.lines {
                let height = line.size(prepaint.line_height).height;
                if line_y + height > bounds.top() && line_y < bounds.bottom() {
                    if !selection.is_empty() {
                        let _ = line.paint_background(
                            point(bounds.left(), line_y),
                            prepaint.line_height,
                            gpui::TextAlign::Left,
                            Some(bounds),
                            window,
                            cx,
                        );
                    }
                    let _ = line.paint(
                        point(bounds.left(), line_y),
                        prepaint.line_height,
                        gpui::TextAlign::Left,
                        Some(bounds),
                        window,
                        cx,
                    );
                }
                line_y += height;
            }
            if focus_handle.is_focused(window) {
                if let Some(cursor) = prepaint.cursor {
                    let theme = self.shell.read(cx).theme.clone();
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left() + cursor.x, bounds.top() + cursor.y),
                            gpui::size(px(2.), prepaint.line_height),
                        ),
                        theme_rgb(&theme, "foreground"),
                    ));
                }
            }
        });
        let focused = focus_handle.is_focused(window);
        if self.target == TextInputTarget::Composer || focused {
            let geometry_changed =
                self.shell
                    .read(cx)
                    .text_input_layout
                    .as_ref()
                    .is_none_or(|layout| {
                        layout.target != self.target
                            || layout.text.as_ref() != prepaint.text
                            || layout.bounds != bounds
                            || layout.selection != selection
                            || layout.font != prepaint.font
                            || layout.font_size != prepaint.font_size
                            || layout.line_height != prepaint.line_height
                            || layout.scroll_offset != prepaint.scroll_offset
                            || layout.cursor_offset != prepaint.cursor_offset
                            || layout.cursor_downstream != prepaint.cursor_downstream
                    });
            let layout = Rc::new(TextInputLayout {
                target: self.target,
                text: std::mem::take(&mut prepaint.text).into(),
                lines: std::mem::take(&mut prepaint.lines).into(),
                font: std::mem::take(&mut prepaint.font),
                font_size: prepaint.font_size,
                line_height: prepaint.line_height,
                bounds,
                selection,
                scroll_offset: prepaint.scroll_offset,
                cursor_offset: prepaint.cursor_offset,
                cursor_downstream: prepaint.cursor_downstream,
                focused,
            });
            self.shell.update(cx, |shell, _| {
                if self.target == TextInputTarget::Composer {
                    shell.composer_text_layout = Some(layout.clone());
                }
                if focused {
                    shell.text_input_layout = Some(layout);
                }
            });
            if focused && geometry_changed {
                window.invalidate_character_coordinates();
            }
        }
    }
}

pub(super) fn composer_input_height(line_count: usize, line_height: f32, max_height: f32) -> f32 {
    (line_count.max(1) as f32 * line_height).clamp(48., max_height)
}

fn composer_scrollbar_thumb(
    viewport: Pixels,
    max_scroll: Pixels,
    offset: Pixels,
) -> Option<(Pixels, Pixels)> {
    if viewport <= px(1.) || max_scroll <= px(1.) {
        return None;
    }
    let height = (viewport * (viewport / (viewport + max_scroll))).clamp(px(24.), viewport);
    Some((
        height,
        (viewport - height) * (offset / max_scroll).clamp(0., 1.),
    ))
}

fn revealed_composer_scroll(
    offset: Pixels,
    caret_y: Pixels,
    line_height: Pixels,
    viewport_height: Pixels,
    max_scroll: Pixels,
) -> Pixels {
    let offset = offset
        .min(caret_y)
        .max(caret_y + line_height - viewport_height);
    offset.clamp(Pixels::ZERO, max_scroll)
}

fn cursor_position(
    lines: &[WrappedLine],
    text: &str,
    cursor: usize,
    line_height: Pixels,
) -> Option<Point<Pixels>> {
    cursor_position_with_affinity(lines, text, cursor, line_height, false)
}

fn cursor_position_with_affinity(
    lines: &[WrappedLine],
    text: &str,
    cursor: usize,
    line_height: Pixels,
    downstream: bool,
) -> Option<Point<Pixels>> {
    let mut search_offset = 0;
    let mut line_y = Pixels::ZERO;

    for line in lines {
        let line_start = if line.text.is_empty() {
            search_offset
        } else {
            text.get(search_offset..)?
                .find(line.text.as_ref())
                .map(|offset| search_offset + offset)?
        };
        let line_end = line_start + line.len();
        let cursor_belongs_here = cursor <= line_end;
        if cursor_belongs_here {
            let local_cursor = cursor.saturating_sub(line_start).min(line.len());
            let position = line.position_for_index(local_cursor, line_height)?;
            if downstream
                && line.wrap_boundaries().iter().any(|boundary| {
                    line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index == local_cursor
                })
            {
                return Some(point(Pixels::ZERO, line_y + position.y + line_height));
            }
            return Some(point(position.x, line_y + position.y));
        }

        search_offset = line_end;
        if text.as_bytes().get(search_offset) == Some(&b'\n') {
            search_offset += 1;
        }
        line_y += line.size(line_height).height;
    }

    lines.last().and_then(|line| {
        line.position_for_index(line.len(), line_height)
            .map(|position| {
                point(
                    position.x,
                    line_y - line.size(line_height).height + position.y,
                )
            })
    })
}

impl DesktopShell {
    fn normalize_browser_url_input_state(&mut self) {
        self.browser_url_selection =
            normalized_browser_url_range(&self.browser_url, &self.browser_url_selection);
        self.browser_url_marked_range = self
            .browser_url_marked_range
            .as_ref()
            .map(|range| normalized_browser_url_range(&self.browser_url, range));
        self.browser_url_selection_reversed = false;
    }
}

impl EntityInputHandler for DesktopShell {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let target = self.text_input_target(window);
        let text = self.text_input_value(target);
        let start = byte_offset_for_utf16(text, range_utf16.start);
        let end = byte_offset_for_utf16(text, range_utf16.end);
        let byte_range = if target == TextInputTarget::BrowserUrl {
            normalized_browser_url_range(text, &(start..end))
        } else {
            start..end
        };
        let actual_range = utf16_offset_for_byte(text, byte_range.start)
            ..utf16_offset_for_byte(text, byte_range.end);
        if actual_range != range_utf16 {
            *adjusted_range = Some(actual_range);
        }
        Some(text[byte_range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let target = self.text_input_target(window);
        if target == TextInputTarget::BrowserUrl {
            self.normalize_browser_url_input_state();
        }
        let text = self.text_input_value(target);
        let selection = self.text_input_selection(target);
        Some(UTF16Selection {
            range: utf16_offset_for_byte(text, selection.start)
                ..utf16_offset_for_byte(text, selection.end),
            reversed: match target {
                TextInputTarget::Composer => self.composer_selection_reversed,
                TextInputTarget::BrowserUrl => self.browser_url_selection_reversed,
                TextInputTarget::Rename => self.rename_selection_reversed,
                TextInputTarget::SidebarSearch => self.sidebar_search_selection_reversed,
                TextInputTarget::ModelSearch => self.model_picker_selection_reversed,
            },
        })
    }

    fn marked_text_range(
        &self,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let target = self.text_input_target(window);
        let text = self.text_input_value(target);
        let marked_range = match target {
            TextInputTarget::Composer => self.composer_marked_range.clone(),
            TextInputTarget::BrowserUrl => self
                .browser_url_marked_range
                .as_ref()
                .map(|range| normalized_browser_url_range(&self.browser_url, range)),
            TextInputTarget::Rename => self.rename_marked_range.clone(),
            TextInputTarget::SidebarSearch => self.sidebar_search_marked_range.clone(),
            TextInputTarget::ModelSearch => self.model_picker_marked_range.clone(),
        };
        marked_range.map(|range| {
            utf16_offset_for_byte(text, range.start)..utf16_offset_for_byte(text, range.end)
        })
    }

    fn unmark_text(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        match self.text_input_target(window) {
            TextInputTarget::Composer => {
                self.composer_marked_range = None;
                self.task_drafts
                    .edits
                    .end_composition(&self.model.ui.composer);
            }
            TextInputTarget::BrowserUrl => self.browser_url_marked_range = None,
            TextInputTarget::Rename => self.rename_marked_range = None,
            TextInputTarget::SidebarSearch => self.sidebar_search_marked_range = None,
            TextInputTarget::ModelSearch => self.model_picker_marked_range = None,
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.text_input_target(window);
        match target {
            TextInputTarget::Composer => self.replace_composer_text(range_utf16, text, cx),
            TextInputTarget::BrowserUrl => self.replace_browser_url_text(range_utf16, text, cx),
            TextInputTarget::Rename => self.replace_rename_text(range_utf16, text, cx),
            TextInputTarget::SidebarSearch => {
                self.replace_sidebar_search_text(range_utf16, text, cx)
            }
            TextInputTarget::ModelSearch => {
                self.replace_model_picker_query_text(range_utf16, text, cx)
            }
        }
        self.refresh_text_input_layout(target, window);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.text_input_target(window);
        match target {
            TextInputTarget::Composer => {
                self.replace_composer_and_mark(range_utf16, text, new_selected_range_utf16, cx)
            }
            TextInputTarget::BrowserUrl => {
                self.replace_browser_url_and_mark(range_utf16, text, new_selected_range_utf16, cx)
            }
            TextInputTarget::Rename => {
                self.replace_rename_and_mark(range_utf16, text, new_selected_range_utf16, cx)
            }
            TextInputTarget::SidebarSearch => self.replace_sidebar_search_and_mark(
                range_utf16,
                text,
                new_selected_range_utf16,
                cx,
            ),
            TextInputTarget::ModelSearch => self.replace_model_picker_query_and_mark(
                range_utf16,
                text,
                new_selected_range_utf16,
                cx,
            ),
        }
        self.refresh_text_input_layout(target, window);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let target = self.text_input_target(window);
        self.text_input_layout.as_ref()?.bounds_for_range(
            target,
            self.text_input_value(target),
            range_utf16,
            element_bounds,
        )
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.text_input_target(window);
        let text = self.text_input_value(target);
        let selection = byte_offset_for_utf16(text, range_utf16.start)
            ..byte_offset_for_utf16(text, range_utf16.end);
        self.set_text_input_selection(target, selection);
        match target {
            TextInputTarget::Composer => {
                self.task_drafts.edits.end_group();
                self.composer_selection_reversed = false;
                self.composer_preferred_column = None;
                self.composer_cursor_downstream = false;
            }
            TextInputTarget::BrowserUrl => self.browser_url_selection_reversed = false,
            TextInputTarget::Rename => self.rename_selection_reversed = false,
            TextInputTarget::SidebarSearch => self.sidebar_search_selection_reversed = false,
            TextInputTarget::ModelSearch => self.model_picker_selection_reversed = false,
        }
        self.refresh_text_input_layout(target, window);
        cx.notify();
    }

    fn text_length_utf16(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        let target = self.text_input_target(window);
        Some(self.text_input_value(target).encode_utf16().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_layout(cx: &mut gpui::TestAppContext, text: &str, width: f32) -> TextInputLayout {
        let display_text = if text.is_empty() { "Placeholder" } else { text };
        let lines = cx.add_empty_window().update(|window, _| {
            window
                .text_system()
                .shape_text(
                    display_text.to_owned().into(),
                    px(16.),
                    &[TextRun {
                        len: display_text.len(),
                        font: gpui::font("Helvetica"),
                        color: gpui::black(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    Some(px(width)),
                    None,
                )
                .unwrap()
                .into_vec()
        });
        TextInputLayout {
            target: TextInputTarget::Composer,
            text: text.to_owned().into(),
            lines: lines.into(),
            font: gpui::font("Helvetica"),
            font_size: px(16.),
            line_height: px(24.),
            bounds: Bounds::new(point(px(0.), px(0.)), gpui::size(px(width), px(96.))),
            selection: 0..0,
            scroll_offset: Pixels::ZERO,
            cursor_offset: 0,
            cursor_downstream: false,
            focused: false,
        }
    }

    fn input_bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(80.), px(120.)), gpui::size(px(400.), px(96.)))
    }

    #[gpui::test]
    fn composer_undo_redo_restores_unicode_replacement_and_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(super::super::platform::set_app_menus);
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, cx| {
                shell.layout.sidebar_collapsed = true;
                shell.layout.workbench_collapsed = true;
                shell.model.ui.composer = "before é中 after".into();
                shell.composer_selection = 7..12;
                shell.composer_selection_reversed = true;
                shell.composer_focus_handle.focus(window, cx);
                shell.replace_text_in_range(None, "replacement", window, cx);
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-z");
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "before é中 after");
            assert_eq!(shell.composer_selection, 7..12);
            assert!(shell.composer_selection_reversed);
        });
        cx.simulate_keystrokes("cmd-shift-z");
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "before replacement after");
            assert_eq!(shell.composer_selection, 18..18);
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn composer_undo_menu_availability_tracks_the_active_drafts_history(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(!window.is_action_available(&TextUndo, cx));
            assert!(!window.is_action_available(&TextRedo, cx));
            shell.update(cx, |shell, cx| {
                shell.replace_text_in_range(None, "draft", window, cx)
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(window.is_action_available(&TextUndo, cx));
            assert!(!window.is_action_available(&TextRedo, cx));
        });
        cx.dispatch_action(TextUndo);
        cx.update(|window, cx| {
            assert!(!window.is_action_available(&TextUndo, cx));
            assert!(window.is_action_available(&TextRedo, cx));
        });
        shell.update(cx, |shell, cx| shell.finish_prompt_submission(true, cx));
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(!window.is_action_available(&TextUndo, cx));
            assert!(!window.is_action_available(&TextRedo, cx));
        });
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn composer_undo_groups_typing_deletion_and_ime_but_keeps_paste_separate(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                window.focus(&shell.composer_focus_handle, cx);
                for text in ["h", "é", "中"] {
                    shell.replace_text_in_range(None, text, window, cx);
                }
            })
        });
        cx.run_until_parked();
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| assert!(shell.model.ui.composer.is_empty()));
        cx.dispatch_action(TextRedo);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                assert_eq!(shell.model.ui.composer, "hé中");
                shell.composer_backspace(&ComposerBackspace, window, cx);
                shell.composer_backspace(&ComposerBackspace, window, cx);
                assert_eq!(shell.model.ui.composer, "h");
            })
        });
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| assert_eq!(shell.model.ui.composer, "hé中"));
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("!".into()));
                shell.text_paste(&TextPaste, window, cx);
                shell.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                shell.replace_and_mark_text_in_range(None, "nihao", Some(5..5), window, cx);
                shell.replace_text_in_range(None, "你好", window, cx);
                assert_eq!(shell.model.ui.composer, "hé中!你好");
            })
        });
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "hé中!");
            assert!(shell.composer_marked_range.is_none());
        });
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| assert_eq!(shell.model.ui.composer, "hé中"));
        cx.dispatch_action(TextRedo);
        cx.dispatch_action(TextRedo);
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "hé中!你好");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn composer_undo_belongs_to_each_draft_and_survives_only_failed_submission(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.switch_draft_owner(Some("a"));
                window.focus(&shell.composer_focus_handle, cx);
                shell.replace_text_in_range(None, "a draft", window, cx);
                shell.switch_draft_owner(Some("b"));
                shell.replace_text_in_range(None, "b draft", window, cx);
            })
        });
        cx.run_until_parked();
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, cx| {
            assert!(shell.model.ui.composer.is_empty());
            shell.switch_draft_owner(Some("a"));
            assert_eq!(shell.model.ui.composer, "a draft");
            cx.notify();
        });
        cx.run_until_parked();
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, cx| {
            assert!(shell.model.ui.composer.is_empty());
            shell.switch_draft_owner(Some("b"));
            assert!(shell.model.ui.composer.is_empty());
            cx.notify();
        });
        cx.run_until_parked();
        cx.dispatch_action(TextRedo);
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.model.ui.composer, "b draft");
            shell.finish_prompt_submission(false, cx);
        });
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| assert!(shell.model.ui.composer.is_empty()));
        cx.dispatch_action(TextRedo);
        shell.update(cx, |shell, cx| shell.finish_prompt_submission(true, cx));
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| {
            assert!(
                shell.model.ui.composer.is_empty(),
                "sent prompts must not return through draft undo"
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn composer_undo_skips_cancelled_ime_and_does_not_edit_other_fields(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                window.focus(&shell.composer_focus_handle, cx);
                shell.replace_text_in_range(None, "before ", window, cx);
                shell.replace_and_mark_text_in_range(None, "nihao", Some(5..5), window, cx);
                shell.composer_backspace(&ComposerBackspace, window, cx);
                assert_eq!(shell.model.ui.composer, "before ");
            })
        });
        cx.run_until_parked();
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| {
            assert!(
                shell.model.ui.composer.is_empty(),
                "cancelled composition must not leave an empty undo step"
            );
            assert!(shell.composer_marked_range.is_none());
        });
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| assert!(shell.model.ui.composer.is_empty()));
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.replace_text_in_range(None, "retained draft", window, cx);
                shell.model_picker_open = true;
                window.focus(&shell.model_picker_focus_handle, cx);
            })
        });
        cx.run_until_parked();
        cx.dispatch_action(TextUndo);
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "retained draft");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn composer_new_edit_after_undo_discards_redo_without_touching_attachments(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                window.focus(&shell.composer_focus_handle, cx);
                shell.attachments.push(Attachment {
                    path: "keep.txt".into(),
                    mime: None,
                });
                shell.replace_text_in_range(None, "old draft", window, cx);
            })
        });
        cx.run_until_parked();
        cx.dispatch_action(TextUndo);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.replace_text_in_range(None, "new draft", window, cx)
            })
        });
        cx.dispatch_action(TextRedo);
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "new draft");
            assert_eq!(shell.attachments[0].path, "keep.txt");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn composer_secondary_controls_follow_peer_presence_and_keep_the_path_readable(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_resize(gpui::size(px(420.), px(800.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("yield-control").is_none(),
            "solo chats should not offer yielding"
        );
        shell.update(cx, |shell, cx| {
            shell.model.ui.presence = vec![mew_client_core::PresenceEntry {
                client_id: 9,
                client_kind: mew_protocol::ClientKind::Tui,
            }];
            shell.model.ui.usage = Some(UsageSummary {
                input_tokens: 2_500_000,
                output_tokens: 999_999,
                cost: 123.45,
                turns: 20,
            });
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("yield-control").is_some());
        let path = cx.debug_bounds("checkout-path").unwrap();
        let status = cx.debug_bounds("session-status").unwrap();
        let footer = cx.debug_bounds("checkout-context").unwrap();
        assert!(
            path.size.width >= px(100.),
            "metadata must leave room for the checkout path: {path:?}"
        );
        assert!(!path.intersects(&status));
        assert!(status.left() >= footer.left() && status.right() <= footer.right());
        shell.update(cx, |shell, cx| {
            shell.model.ui.presence.clear();
            shell.model.ui.control_yielded_by = Some(9);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("yield-control").is_none());
        assert!(cx.debug_bounds("control-yielded").is_some());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn compact_persona_control_reveals_a_hint_without_moving_the_toolbar(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_resize(gpui::size(px(420.), px(800.)));
        shell.update(cx, |shell, _| {
            shell.model.ui.current_persona = Some("builder".into())
        });
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            cx.notify();
        });
        cx.run_until_parked();
        let trigger = cx.debug_bounds("persona-picker-trigger").unwrap();
        assert_eq!(trigger.size.width, px(26.));
        cx.simulate_mouse_move(trigger.center(), None, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.dispatcher.advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(cx.debug_bounds("composer-persona-hint").is_some());
        assert_eq!(cx.debug_bounds("persona-picker-trigger").unwrap(), trigger);
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn ime_caret_in_empty_input_uses_field_origin(cx: &mut gpui::TestAppContext) {
        let layout = input_layout(cx, "", 400.);

        let bounds = layout
            .bounds_for_range(TextInputTarget::Composer, "", 0..0, input_bounds())
            .unwrap();

        assert_eq!(bounds.origin, input_bounds().origin);
        assert_eq!(bounds.size, gpui::size(px(0.), px(24.)));
    }

    #[gpui::test]
    fn ime_range_uses_utf16_offsets_and_painted_glyph_positions(cx: &mut gpui::TestAppContext) {
        let text = "é🙂z";
        let layout = input_layout(cx, text, 400.);
        let line = &layout.lines[0];

        let bounds = layout
            .bounds_for_range(TextInputTarget::Composer, text, 1..3, input_bounds())
            .unwrap();

        assert_eq!(
            bounds.left(),
            input_bounds().left() + line.unwrapped_layout.x_for_index(2)
        );
        assert_eq!(
            bounds.right(),
            input_bounds().left() + line.unwrapped_layout.x_for_index(6)
        );
        assert_eq!(bounds.top(), input_bounds().top());
        assert_eq!(bounds.size.height, px(24.));

        let end = layout
            .bounds_for_range(TextInputTarget::Composer, text, 99..99, input_bounds())
            .unwrap();
        assert_eq!(
            end.left(),
            input_bounds().left() + line.unwrapped_layout.x_for_index(text.len())
        );
        assert_eq!(end.size.width, px(0.));
    }

    #[gpui::test]
    fn ime_ranges_at_newlines_anchor_to_the_first_requested_line(cx: &mut gpui::TestAppContext) {
        let text = "ab\ncdef\n";
        let layout = input_layout(cx, text, 400.);

        let before_newline = layout
            .bounds_for_range(TextInputTarget::Composer, text, 2..2, input_bounds())
            .unwrap();
        assert_eq!(before_newline.top(), input_bounds().top());
        assert_eq!(
            before_newline.left(),
            input_bounds().left() + layout.lines[0].width()
        );

        let multiline = layout
            .bounds_for_range(TextInputTarget::Composer, text, 1..6, input_bounds())
            .unwrap();
        assert_eq!(multiline.top(), input_bounds().top());
        assert_eq!(
            multiline.right(),
            input_bounds().left() + layout.lines[0].width()
        );
        assert_eq!(multiline.size.height, px(24.));

        let trailing_newline = layout
            .bounds_for_range(TextInputTarget::Composer, text, 8..8, input_bounds())
            .unwrap();
        assert_eq!(
            trailing_newline.origin,
            input_bounds().origin + point(px(0.), px(48.))
        );
    }

    #[gpui::test]
    fn painted_caret_before_newline_stays_on_its_logical_line(cx: &mut gpui::TestAppContext) {
        let layout = input_layout(cx, "ab\ncdef\n", 400.);

        let position = cursor_position(&layout.lines, &layout.text, 2, layout.line_height).unwrap();

        assert_eq!(position, point(layout.lines[0].width(), px(0.)));
    }

    #[gpui::test]
    fn ime_wrapped_ranges_use_the_first_visual_row(cx: &mut gpui::TestAppContext) {
        let text = "alpha beta gamma delta";
        let layout = input_layout(cx, text, 80.);
        let line = &layout.lines[0];
        let boundary = line.wrap_boundaries()[0];
        let row_end = line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
        assert!(row_end < text.len());

        let bounds = layout
            .bounds_for_range(
                TextInputTarget::Composer,
                text,
                0..text.len(),
                input_bounds(),
            )
            .unwrap();
        assert_eq!(bounds.origin, input_bounds().origin);
        assert_eq!(bounds.size.height, px(24.));
        assert_eq!(
            bounds.right(),
            input_bounds().left() + line.unwrapped_layout.x_for_index(row_end)
        );

        let boundary_offset = utf16_offset_for_byte(text, row_end);
        let boundary_caret = layout
            .bounds_for_range(
                TextInputTarget::Composer,
                text,
                boundary_offset..boundary_offset,
                input_bounds(),
            )
            .unwrap();
        let painted_caret =
            cursor_position(&layout.lines, text, row_end, layout.line_height).unwrap();
        assert_eq!(painted_caret.y, px(0.));
        assert_eq!(boundary_caret.origin, input_bounds().origin + painted_caret);

        let end = layout
            .bounds_for_range(
                TextInputTarget::Composer,
                text,
                text.len()..text.len(),
                input_bounds(),
            )
            .unwrap();
        assert!(end.top() > input_bounds().top());
        assert_eq!(end.size.width, px(0.));
    }

    #[gpui::test]
    fn ime_geometry_rejects_stale_content_or_another_focused_field(cx: &mut gpui::TestAppContext) {
        let layout = input_layout(cx, "old", 400.);

        assert!(layout
            .bounds_for_range(TextInputTarget::Composer, "new", 0..0, input_bounds())
            .is_none());
        assert!(layout
            .bounds_for_range(TextInputTarget::BrowserUrl, "old", 0..0, input_bounds())
            .is_none());
    }

    #[gpui::test]
    fn ime_geometry_supports_each_shell_input_target(cx: &mut gpui::TestAppContext) {
        let mut layout = input_layout(cx, "field", 400.);

        for target in [
            TextInputTarget::Composer,
            TextInputTarget::BrowserUrl,
            TextInputTarget::Rename,
            TextInputTarget::SidebarSearch,
            TextInputTarget::ModelSearch,
        ] {
            layout.target = target;
            let bounds = layout
                .bounds_for_range(target, "field", 0..0, input_bounds())
                .unwrap();

            assert_eq!(bounds.origin, input_bounds().origin);
            assert_eq!(bounds.size, gpui::size(px(0.), layout.line_height));
        }
    }

    #[gpui::test]
    fn painted_shell_field_supplies_native_ime_geometry(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let text = "é🙂\nend";
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, _| {
                shell.model.ui.composer = text.into();
                shell.composer_selection = 6..6;
                shell.composer_cursor_visible = true;
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        let origin = point(px(80.), px(120.));
        let (_, painted) = cx.draw(origin, gpui::size(px(400.), px(96.)), |_, _| {
            ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            }
        });
        let painted_caret = painted.cursor.unwrap();
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                let field = shell.composer_bounds.unwrap();
                let caret = shell.bounds_for_range(3..3, field, window, cx).unwrap();
                assert_eq!(caret.origin, origin + painted_caret);
                assert_eq!(caret.size, gpui::size(px(0.), painted.line_height));
                let selected = shell.bounds_for_range(1..3, field, window, cx).unwrap();
                assert_eq!(selected.right(), caret.left());
                assert_eq!(selected.top(), caret.top());
                assert!(selected.left() > field.left());
                assert!(selected.size.width > px(0.));

                shell.model.ui.composer = "changed before repaint".into();
                assert!(shell.bounds_for_range(3..3, field, window, cx).is_none());
                shell.model.ui.composer = text.into();
                shell.sidebar_search = text.into();
            });
            let focus = shell.read(cx).sidebar_search_focus_handle.clone();
            window.focus(&focus, cx);
            shell.update(cx, |shell, cx| {
                let field = shell.composer_bounds.unwrap();
                assert!(shell.bounds_for_range(3..3, field, window, cx).is_none());
                shell.quitting = true;
            });
        });
    }

    #[gpui::test]
    fn narrow_composer_allocates_height_for_its_wrapped_visual_rows(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let text = "alpha beta gamma delta";
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, _| {
                shell.model.ui.composer = text.into();
                shell.composer_selection = text.len()..text.len();
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        for _ in 0..2 {
            cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(80.), px(96.)),
                |_, _| ComposerElement {
                    shell: shell.clone(),
                    target: TextInputTarget::Composer,
                },
            );
        }
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                let layout = shell.text_input_layout.as_ref().unwrap();
                assert_eq!(layout.lines.len(), 1);
                assert_eq!(
                    layout.lines[0].text.as_ref(),
                    text,
                    "wrapping preserves the full draft"
                );
                assert_eq!(layout.lines[0].wrap_boundaries().len(), 3);
                let field = shell.composer_bounds.unwrap();
                assert_eq!(
                    field.size.height,
                    px(96.),
                    "long wrapping respects the editor height limit"
                );
                let caret = shell
                    .bounds_for_range(text.len()..text.len(), field, window, cx)
                    .unwrap();
                assert!(caret.bottom() <= field.bottom());
                shell.quitting = true;
            });
        });
    }

    #[gpui::test]
    fn long_drafts_keep_the_caret_and_pointer_mapping_inside_the_editor(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        for text in [
            "first\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\nlast é🙂".to_owned(),
            "wrapped draft é🙂 ".repeat(80),
        ] {
            cx.update(|window, cx| {
                let focus = shell.update(cx, |shell, _| {
                    shell.model.ui.composer = text.clone();
                    shell.composer_selection = text.len()..text.len();
                    shell.composer_cursor_visible = true;
                    shell.composer_focus_handle.clone()
                });
                window.focus(&focus, cx);
            });
            for width in [240., 360.] {
                let (_, painted) = cx.draw(
                    point(px(80.), px(120.)),
                    gpui::size(px(width), px(96.)),
                    |_, _| ComposerElement {
                        shell: shell.clone(),
                        target: TextInputTarget::Composer,
                    },
                );
                let caret = painted.cursor.expect("focused draft has a caret");
                cx.update(|window, cx| {
                    shell.update(cx, |shell, cx| {
                        let field = shell.composer_bounds.unwrap();
                        assert!(field.size.height <= px(96.));
                        assert!(
                            caret.y >= px(0.) && caret.y + painted.line_height <= field.size.height,
                            "the draft tail is visible at width {width}"
                        );
                        let utf16_end = text.encode_utf16().count();
                        let native = shell
                            .bounds_for_range(utf16_end..utf16_end, field, window, cx)
                            .unwrap();
                        assert_eq!(native.origin, field.origin + caret);
                        assert!(native.bottom() <= field.bottom());
                        assert_eq!(
                            shell.composer_index_for_position(
                                native.origin + point(px(1.), px(1.)),
                                window
                            ),
                            text.len(),
                            "pointer lookup follows the scrolled text"
                        );
                    })
                });
            }
            shell.update(cx, |shell, _| {
                shell.composer_selection = 0..text.len();
                shell.composer_selection_reversed = true;
            });
            cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(240.), px(96.)),
                |_, _| ComposerElement {
                    shell: shell.clone(),
                    target: TextInputTarget::Composer,
                },
            );
            shell.read_with(cx, |shell, _| {
                assert_eq!(
                    shell.composer_scroll_offset,
                    Pixels::ZERO,
                    "selection reveals its active edge"
                )
            });
            cx.update(|window, cx| {
                shell.update(cx, |shell, cx| {
                    shell.composer_home(&ComposerHome, window, cx)
                })
            });
            let (_, painted) = cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(240.), px(96.)),
                |_, _| ComposerElement {
                    shell: shell.clone(),
                    target: TextInputTarget::Composer,
                },
            );
            assert_eq!(
                painted.cursor.unwrap().y,
                px(0.),
                "moving to the beginning reveals it"
            );
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn draft_wheel_scrolling_holds_until_editing_and_ime_tracks_the_scrolled_caret(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let text = "first\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\nend é🙂";
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, _| {
                shell.model.ui.composer = text.into();
                shell.composer_selection = text.len()..text.len();
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                assert!(shell.composer_scroll_offset > Pixels::ZERO);
                let field = shell.composer_bounds.unwrap().scale(window.scale_factor());
                assert!(
                    window
                        .painted_quads()
                        .iter()
                        .any(|quad| quad.content_mask.bounds == field),
                    "the caret is clipped to the editor viewport"
                );
                shell.composer_scroll_wheel(
                    &gpui::ScrollWheelEvent {
                        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(1000.))),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
            })
        });
        let (_, painted) = cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        shell.read_with(cx, |shell, _| {
            assert_eq!(
                shell.composer_scroll_offset,
                Pixels::ZERO,
                "manual scrolling can inspect the beginning while the caret stays at the end"
            )
        });
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                let field = shell.composer_bounds.unwrap();
                assert_eq!(
                    shell.composer_index_for_position(field.origin + point(px(1.), px(1.)), window),
                    0
                );
                shell.replace_and_mark_text_in_range(None, "拼音", Some(2..2), window, cx);
                let end = shell.model.ui.composer.encode_utf16().count();
                let caret = shell.bounds_for_range(end..end, field, window, cx).unwrap();
                assert!(
                    caret.top() >= field.top() && caret.bottom() <= field.bottom(),
                    "IME geometry follows editing before the next paint"
                );
            })
        });
        let (_, edited) = cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        assert!(
            painted.cursor.is_some_and(|cursor| cursor.y >= px(96.)),
            "the offscreen caret is clipped rather than forcing manual scroll back"
        );
        assert!(edited.cursor.unwrap().y + edited.line_height <= px(96.));
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn unchanged_marked_text_and_native_selection_reveal_the_active_caret(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                let text = "first\nsecond\nthird\nfourth\nfifth\nsixth\nlast 拼音";
                let end = text.encode_utf16().count();
                let field = shell.composer_bounds.unwrap();
                shell.replace_and_mark_text_in_range(None, text, Some(end..end), window, cx);
                assert!(shell.composer_scroll_offset > Pixels::ZERO);
                shell.replace_and_mark_text_in_range(None, text, Some(0..0), window, cx);
                let caret = shell.bounds_for_range(0..0, field, window, cx).unwrap();
                assert_eq!(
                    caret.top(),
                    field.top(),
                    "unchanged composition reveals its new caret before painting"
                );
                shell.set_selected_text_range(end..end, window, cx);
                let caret = shell.bounds_for_range(end..end, field, window, cx).unwrap();
                assert!(
                    caret.top() >= field.top() && caret.bottom() <= field.bottom(),
                    "native selection updates candidate geometry before painting"
                );
                shell.quitting = true;
            });
        });
    }

    #[gpui::test]
    fn long_drafts_move_vertically_instead_of_recalling_prompt_history(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        for text in [
            "first\nsecond\nthird\nfourth\nfifth\nsixth\nlast é🙂".to_owned(),
            "a wrapped draft é🙂 ".repeat(20),
        ] {
            cx.update(|window, cx| {
                let focus = shell.update(cx, |shell, _| {
                    shell.model.ui.composer = text.clone();
                    shell.composer_selection = text.len()..text.len();
                    shell.composer_selection_reversed = false;
                    shell.composer_focus_handle.clone()
                });
                window.focus(&focus, cx);
            });
            cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(240.), px(96.)),
                |_, _| ComposerElement {
                    shell: shell.clone(),
                    target: TextInputTarget::Composer,
                },
            );
            cx.update(|window, cx| {
                shell.update(cx, |shell, cx| {
                    for key in ["up", "shift-up"] {
                        shell.composer_key_down(
                            &gpui::KeyDownEvent {
                                keystroke: gpui::Keystroke::parse(key).unwrap(),
                                is_held: false,
                                prefer_character_input: false,
                            },
                            window,
                            cx,
                        );
                    }
                    assert_eq!(shell.model.ui.composer, text);
                    assert!(shell.composer_cursor_offset() < text.len());
                    assert!(
                        !shell.composer_selection.is_empty(),
                        "shift-up extends selection"
                    );
                    assert!(shell.composer_selection_reversed);
                })
            });
            let (_, painted) = cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(240.), px(96.)),
                |_, _| ComposerElement {
                    shell: shell.clone(),
                    target: TextInputTarget::Composer,
                },
            );
            shell.read_with(cx, |shell, _| {
                let cursor = cursor_position(
                    &shell.text_input_layout.as_ref().unwrap().lines,
                    &text,
                    shell.composer_cursor_offset(),
                    painted.line_height,
                )
                .unwrap();
                assert!(
                    cursor.y >= shell.composer_scroll_offset
                        && cursor.y + painted.line_height <= shell.composer_scroll_offset + px(96.)
                );
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn vertical_navigation_preserves_the_column_through_short_lines(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        for extend in [false, true] {
            cx.update(|window, cx| {
                let focus = shell.update(cx, |shell, cx| {
                    shell.set_composer_text("abcdefghij\nx\nabcdefghij".into(), cx);
                    shell.move_composer_to(8, cx);
                    shell.composer_focus_handle.clone()
                });
                window.focus(&focus, cx);
            });
            cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(400.), px(96.)),
                |_, _| ComposerElement {
                    shell: shell.clone(),
                    target: TextInputTarget::Composer,
                },
            );
            cx.update(|window, cx| {
                shell.update(cx, |shell, cx| {
                    assert!(shell.composer_move_vertical(false, extend, window, cx));
                    assert_eq!(shell.composer_cursor_offset(), 12);
                    assert!(shell.composer_move_vertical(false, extend, window, cx));
                    assert_eq!(
                        shell.composer_cursor_offset(),
                        21,
                        "the next long line restores column eight"
                    );
                    if extend {
                        assert_eq!(shell.composer_selection, 8..21);
                    }
                    shell.composer_left(&ComposerLeft, window, cx);
                    shell.composer_move_vertical(true, false, window, cx);
                    shell.composer_move_vertical(true, false, window, cx);
                    assert_eq!(
                        shell.composer_cursor_offset(),
                        if extend { 8 } else { 7 },
                        "horizontal movement resets the preferred column"
                    );
                })
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn vertical_navigation_uses_the_target_side_of_a_soft_wrap(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, _| {
                shell.model.ui.composer = "a wrapped draft ".repeat(30);
                shell.composer_selection = 0..0;
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        for (older, row) in [(false, 1.), (false, 2.), (true, 1.)] {
            cx.update(|window, cx| {
                shell.update(cx, |shell, cx| {
                    assert!(shell.composer_move_vertical(older, false, window, cx));
                })
            });
            let (_, painted) = cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(240.), px(96.)),
                |_, _| ComposerElement {
                    shell: shell.clone(),
                    target: TextInputTarget::Composer,
                },
            );
            let caret = painted.cursor.unwrap();
            assert_eq!(caret.y, painted.line_height * row);
            assert_eq!(caret.x, px(0.));
            cx.update(|window, cx| {
                shell.update(cx, |shell, cx| {
                    let offset = utf16_offset_for_byte(
                        &shell.model.ui.composer,
                        shell.composer_cursor_offset(),
                    );
                    let field = shell.composer_bounds.unwrap();
                    let native = shell
                        .bounds_for_range(offset..offset, field, window, cx)
                        .unwrap();
                    assert_eq!(
                        native.origin,
                        field.origin + caret,
                        "native geometry uses the same wrap affinity"
                    );
                    assert_eq!(
                        shell.composer_index_for_position(
                            native.origin + point(px(0.1), px(1.)),
                            window
                        ),
                        shell.composer_cursor_offset()
                    );
                })
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn vertical_navigation_refreshes_a_draft_edited_before_paint(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, _| {
                shell.model.ui.composer = "a wrapped draft ".repeat(30);
                let end = shell.model.ui.composer.len();
                shell.composer_selection = end..end;
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.composer_backspace(&ComposerBackspace, window, cx);
                let edited = shell.model.ui.composer.clone();
                let cursor = shell.composer_cursor_offset();
                assert!(
                    shell.composer_move_vertical(true, false, window, cx),
                    "edited wrapping is available before drawing"
                );
                assert_eq!(shell.model.ui.composer, edited);
                assert!(shell.composer_cursor_offset() < cursor);
                shell.quitting = true;
            })
        });
    }

    #[gpui::test]
    fn scrolling_an_unfocused_draft_does_not_follow_another_fields_layout(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, _| {
                shell.model.ui.composer = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight".into();
                shell.composer_selection =
                    shell.model.ui.composer.len()..shell.model.ui.composer.len();
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        cx.update(|window, cx| window.focus(&shell.read(cx).browser_url_focus_handle.clone(), cx));
        cx.draw(
            point(px(400.), px(20.)),
            gpui::size(px(240.), px(28.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::BrowserUrl,
            },
        );
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.composer_scroll_wheel(
                    &gpui::ScrollWheelEvent {
                        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(1000.))),
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            })
        });
        cx.draw(
            point(px(80.), px(120.)),
            gpui::size(px(240.), px(96.)),
            |_, _| ComposerElement {
                shell: shell.clone(),
                target: TextInputTarget::Composer,
            },
        );
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.composer_scroll_offset, Pixels::ZERO);
            assert_eq!(
                shell.text_input_layout.as_ref().unwrap().target,
                TextInputTarget::BrowserUrl,
                "draft painting does not steal native text input ownership"
            );
        });
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn auxiliary_fields_paste_clipboard_over_selection(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let pasted = "https://example.test/é🙂";
        cx.update(|window, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(pasted.into()));
            shell.update(cx, |shell, cx| {
                for target in [
                    TextInputTarget::BrowserUrl,
                    TextInputTarget::Rename,
                    TextInputTarget::SidebarSearch,
                    TextInputTarget::ModelSearch,
                ] {
                    shell.model_picker_open = target == TextInputTarget::ModelSearch;
                    match target {
                        TextInputTarget::BrowserUrl => shell.browser_url = "old".into(),
                        TextInputTarget::Rename => shell.rename_draft = "old".into(),
                        TextInputTarget::SidebarSearch => shell.sidebar_search = "old".into(),
                        TextInputTarget::ModelSearch => shell.model_picker_query = "old".into(),
                        TextInputTarget::Composer => unreachable!(),
                    }
                    shell.set_text_input_selection(target, 0..3);
                    window.focus(shell.text_input_focus_handle(target), cx);
                    shell.text_paste(&TextPaste, window, cx);
                    assert_eq!(shell.text_input_value(target), pasted, "{target:?}");
                    assert_eq!(
                        shell.text_input_selection(target),
                        pasted.len()..pasted.len()
                    );
                }
                shell.quitting = true;
            });
        });
    }

    #[gpui::test]
    fn native_composition_keeps_candidate_geometry_before_repaint(cx: &mut gpui::TestAppContext) {
        cx.skip_drawing();
        let cx = cx.add_empty_window();
        let shell = cx
            .new_window_entity(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        for target in [
            TextInputTarget::Composer,
            TextInputTarget::BrowserUrl,
            TextInputTarget::Rename,
            TextInputTarget::SidebarSearch,
            TextInputTarget::ModelSearch,
        ] {
            cx.update(|window, cx| {
                let focus = shell.update(cx, |shell, _| {
                    shell.model_picker_open = target == TextInputTarget::ModelSearch;
                    match target {
                        TextInputTarget::Composer => shell.composer_focus_handle.clone(),
                        TextInputTarget::BrowserUrl => shell.browser_url_focus_handle.clone(),
                        TextInputTarget::Rename => shell.rename_focus_handle.clone(),
                        TextInputTarget::SidebarSearch => shell.sidebar_search_focus_handle.clone(),
                        TextInputTarget::ModelSearch => shell.model_picker_focus_handle.clone(),
                    }
                });
                window.focus(&focus, cx);
            });
            cx.draw(
                point(px(80.), px(120.)),
                gpui::size(px(400.), px(96.)),
                |_, _| {
                    div()
                        .w_full()
                        .text_size(px(22.))
                        .line_height(px(30.))
                        .child(ComposerElement {
                            shell: shell.clone(),
                            target,
                        })
                },
            );
            cx.update(|window, cx| {
                shell.update(cx, |shell, cx| {
                    let field = shell.text_input_layout.as_ref().unwrap().bounds;
                    let before = shell.bounds_for_range(0..0, field, window, cx).unwrap();
                    shell.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);

                    let during = shell.bounds_for_range(0..2, field, window, cx).unwrap();
                    assert_eq!(during.origin, before.origin);
                    assert_eq!(during.size.height, px(30.));
                    let expected = window.text_system().shape_line(
                        "ni".into(),
                        px(22.),
                        &[TextRun {
                            len: 2,
                            font: window.text_style().font(),
                            color: gpui::black(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    assert_eq!(during.size.width, expected.width);

                    shell.replace_and_mark_text_in_range(None, "nihao", Some(5..5), window, cx);
                    let expanded = shell.bounds_for_range(0..5, field, window, cx).unwrap();
                    assert_eq!(expanded.origin, before.origin);
                    assert!(expanded.size.width > during.size.width);
                    shell.replace_text_in_range(None, "你好", window, cx);
                    let committed = shell.bounds_for_range(0..2, field, window, cx).unwrap();
                    assert_eq!(committed.origin, before.origin);
                    assert!(committed.size.width > px(0.));
                    assert_eq!(shell.text_input_value(target), "你好", "{target:?}");
                    assert!(shell.marked_text_range(window, cx).is_none());
                });
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn text_actions_copy_cut_and_paste_unicode_in_focused_fields(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let original = "é🙂中";
        for target in [
            TextInputTarget::Composer,
            TextInputTarget::BrowserUrl,
            TextInputTarget::Rename,
            TextInputTarget::SidebarSearch,
            TextInputTarget::ModelSearch,
        ] {
            cx.update(|window, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("unchanged".into()));
                let focus = shell.update(cx, |shell, cx| {
                    shell.model_picker_open = target == TextInputTarget::ModelSearch;
                    shell.settings_open = false;
                    shell.layout.sidebar_collapsed = false;
                    shell.layout.workbench_collapsed = target != TextInputTarget::BrowserUrl;
                    shell.browser_panel_open = target == TextInputTarget::BrowserUrl;
                    shell.work_tabs.active_mut().kind = WorkTabKind::Browser;
                    shell.rename_session_id =
                        (target == TextInputTarget::Rename).then(|| "rename-target".into());
                    if target == TextInputTarget::Rename {
                        shell.model.ui.conversations = vec![ConversationItem {
                            session_id: "rename-target".into(),
                            title: "Rename target".into(),
                            cwd: None,
                            workspace_path: None,
                            last_message_at: None,
                            state: mew_protocol::SessionState::Idle,
                            last_turn_failed: false,
                            failure_detail: None,
                            unread_completion: false,
                            needs_attention: false,
                            archived: false,
                            pinned: false,
                            group_id: None,
                        }];
                        shell.rebuild_sidebar_rows();
                    }
                    match target {
                        TextInputTarget::Composer => shell.model.ui.composer = original.into(),
                        TextInputTarget::BrowserUrl => shell.browser_url = original.into(),
                        TextInputTarget::Rename => shell.rename_draft = original.into(),
                        TextInputTarget::SidebarSearch => shell.sidebar_search = original.into(),
                        TextInputTarget::ModelSearch => shell.model_picker_query = original.into(),
                    }
                    shell.set_text_input_selection(target, 2..6);
                    cx.notify();
                    match target {
                        TextInputTarget::Composer => shell.composer_focus_handle.clone(),
                        TextInputTarget::BrowserUrl => shell.browser_url_focus_handle.clone(),
                        TextInputTarget::Rename => shell.rename_focus_handle.clone(),
                        TextInputTarget::SidebarSearch => shell.sidebar_search_focus_handle.clone(),
                        TextInputTarget::ModelSearch => shell.model_picker_focus_handle.clone(),
                    }
                });
                window.focus(&focus, cx);
            });
            cx.run_until_parked();

            cx.dispatch_action(TextCopy);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("🙂"),
                "{target:?}"
            );
            shell.read_with(cx, |shell, _| {
                assert_eq!(shell.text_input_value(target), original)
            });

            cx.dispatch_action(TextCut);
            shell.read_with(cx, |shell, _| {
                assert_eq!(shell.text_input_value(target), "é中", "{target:?}");
                assert_eq!(shell.text_input_selection(target), 2..2);
            });
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("🙂")
            );

            cx.dispatch_action(TextPaste);
            shell.read_with(cx, |shell, _| {
                assert_eq!(shell.text_input_value(target), original, "{target:?}");
                assert_eq!(shell.text_input_selection(target), 6..6);
            });
            cx.dispatch_action(TextSelectAll);
            cx.refresh().unwrap();
            cx.run_until_parked();
            cx.update(|window, cx| {
                let shell = shell.read(cx);
                assert_eq!(shell.text_input_selection(target), 0..original.len());
                let layout = shell.text_input_layout.as_ref().unwrap();
                assert_eq!(layout.target, target);
                assert_eq!(layout.text.as_ref(), original);
                assert!(
                    !layout.lines.is_empty(),
                    "selected {target:?} must remain painted"
                );
                assert!(layout.lines[0].width() > px(0.));
                let background: gpui::Hsla = theme_rgb(&shell.theme, "selection.background").into();
                let field = layout.bounds.scale(window.scale_factor());
                assert!(
                    window.painted_quads().iter().any(|quad| {
                        quad.background.as_solid().is_some_and(|mut color| {
                            let visible = color.a > 0.8;
                            color.a = 1.;
                            visible && color == background
                        }) && !quad.bounds.intersect(&field).is_empty()
                    }),
                    "selected {target:?} needs its contrasting background"
                );
            });
            cx.update(|_, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("replacement".into()))
            });
            cx.dispatch_action(TextPaste);
            shell.read_with(cx, |shell, _| {
                assert_eq!(shell.text_input_value(target), "replacement", "{target:?}")
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn text_menu_actions_require_a_rendered_focused_field(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            for action in [
                &TextCopy as &dyn gpui::Action,
                &TextCut,
                &TextPaste,
                &TextSelectAll,
            ] {
                assert!(window.is_action_available(action, cx));
            }
        });
        shell.update(cx, |shell, cx| {
            shell.settings_open = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(shell.read(cx).composer_focus_handle.is_focused(window));
            for action in [
                &TextCopy as &dyn gpui::Action,
                &TextCut,
                &TextPaste,
                &TextSelectAll,
            ] {
                assert!(!window.is_action_available(action, cx));
            }
            let focus = shell.update(cx, |shell, cx| {
                shell.settings_open = false;
                shell.layout.workbench_collapsed = false;
                shell.work_tabs.active_mut().kind = WorkTabKind::Browser;
                shell.browser_panel_open = true;
                cx.notify();
                shell.browser_url_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| assert!(window.is_action_available(&TextPaste, cx)));
        shell.update(cx, |shell, cx| {
            shell.open_work_tool(WorkTabKind::Changes, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(shell.read(cx).browser_url_focus_handle.is_focused(window));
            assert!(!window.is_action_available(&TextPaste, cx));
            let focus = shell.read(cx).sidebar_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            for action in [
                &TextCopy as &dyn gpui::Action,
                &TextCut,
                &TextPaste,
                &TextSelectAll,
            ] {
                assert!(!window.is_action_available(action, cx));
            }
        });
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn text_actions_leave_empty_selection_and_other_focus_unchanged(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, cx| {
                shell.model.ui.composer = "draft".into();
                shell.composer_selection = 2..2;
                cx.notify();
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("clipboard".into()));
        });
        cx.run_until_parked();
        cx.dispatch_action(TextCopy);
        cx.dispatch_action(TextCut);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("clipboard")
        );
        cx.update(|window, cx| {
            let focus = shell.read(cx).popover_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.dispatch_action(TextSelectAll);
        cx.dispatch_action(TextPaste);
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "draft");
            assert_eq!(shell.composer_selection, 2..2);
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn text_paste_keeps_composer_image_attachments(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let image_bytes = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../mew-web-ui/public/mew-transparent-closeup.png"),
        )
        .unwrap();
        let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, image_bytes.clone());
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
            cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
        });
        cx.dispatch_action(TextPaste);
        let path = shell.update(cx, |shell, _| {
            assert_eq!(shell.attachments.len(), 1);
            assert_eq!(shell.attachments[0].mime.as_deref(), Some("image/png"));
            assert!(shell.model.ui.composer.is_empty());
            shell.quitting = true;
            PathBuf::from(&shell.attachments[0].path)
        });
        assert_eq!(std::fs::read(&path).unwrap(), image_bytes);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn stale_browser_url_ranges_are_clamped_to_valid_boundaries() {
        assert_eq!(normalized_browser_url_range("https://é", &(2..999)), 2..10);
        assert_eq!(
            normalized_browser_url_range("short", &std::ops::Range { start: 99, end: 3 },),
            3..5
        );
    }
}

#[cfg(test)]
mod long_draft_tests {
    use super::*;

    fn set_draft(shell: &Entity<DesktopShell>, cx: &mut gpui::VisualTestContext, text: &str) {
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.composer = text.into();
            shell.composer_selection = text.len()..text.len();
            cx.notify();
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn expansion_keeps_wrapped_attachments_and_errors_inside_the_pane(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_resize(gpui::size(px(420.), px(800.)));
        set_draft(&shell, cx, &"draft line\n".repeat(40));
        shell.update(cx, |shell, cx| {
            shell.attachments = (0..8).map(|index| mew_protocol::Attachment {
                path: format!("/tmp/{index}-a-very-long-filename-that-wraps-into-its-own-attachment-row-é中.txt"),
                mime: None,
            }).collect();
            shell.model.last_error = Some("The connection was interrupted. Your draft is saved; reconnect to try again.".into());
            cx.notify();
        });
        cx.run_until_parked();
        let first = cx.debug_bounds("attachment-0").unwrap();
        assert!(
            first.top() >= px(PANE_HEADER_HEIGHT),
            "the ordinary draft and its attachments fit"
        );
        let toggle = cx
            .debug_bounds("composer-expand-toggle")
            .expect("there is still some room to grow");
        cx.simulate_click(toggle.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        let first = cx.debug_bounds("attachment-0").unwrap();
        let composer = cx.debug_bounds("composer").unwrap();
        assert!(
            first.top() >= px(PANE_HEADER_HEIGHT),
            "expansion must not push attachments above the pane"
        );
        assert!(composer.bottom() <= px(800.));
        let toggle = cx.debug_bounds("composer-expand-toggle").unwrap();
        assert!(toggle.top() >= composer.top() && toggle.bottom() <= composer.bottom());
        shell.update(cx, |shell, _| {
            assert!(shell.composer_bounds.unwrap().size.height > px(96.));
            assert!(shell.composer_bounds.unwrap().size.height < px(360.));
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn expanded_drafts_fit_short_and_tall_windows_with_the_terminal_folded_below(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        for width in [420., 900.] {
            for height in [480., 800.] {
                for terminal in [false, true] {
                    cx.simulate_resize(gpui::size(px(width), px(height)));
                    shell.update(cx, |shell, cx| {
                        shell.composer_expanded = false;
                        shell.work_tabs.open_tool(if terminal {
                            WorkTabKind::Terminal
                        } else {
                            WorkTabKind::Activity
                        });
                        shell.attachments = vec![mew_protocol::Attachment {
                            path: "/tmp/long-draft-notes-é中.txt".into(),
                            mime: None,
                        }];
                        cx.notify();
                    });
                    set_draft(&shell, cx, &"long draft\n".repeat(80));
                    if let Some(toggle) = cx.debug_bounds("composer-expand-toggle") {
                        cx.simulate_click(toggle.center(), gpui::Modifiers::none());
                        cx.run_until_parked();
                    }
                    let composer = cx.debug_bounds("composer").unwrap();
                    let available = height
                        - PANE_HEADER_HEIGHT
                        - if terminal {
                            TERMINAL_EXPANDED_HEIGHT
                        } else {
                            0.
                        };
                    shell.update(cx, |shell, _| {
                        let field = shell.composer_bounds.unwrap();
                        assert!(field.size.height <= px(360.) && field.size.height <= px(available * 0.5));
                        assert!(field.top() >= px(PANE_HEADER_HEIGHT + 24.), "the transcript retains room: {width}x{height}, terminal {terminal}, {field:?}");
                        assert!(field.bottom() <= px(PANE_HEADER_HEIGHT + available));
                    });
                    assert!(composer.top() >= px(PANE_HEADER_HEIGHT));
                    assert!(composer.bottom() <= px(PANE_HEADER_HEIGHT + available));
                }
            }
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn expansion_survives_failed_send_and_same_chat_but_resets_after_send_or_navigation(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_resize(gpui::size(px(420.), px(800.)));
        shell.update(cx, |shell, _| {
            shell.switch_draft_owner(Some("a"));
            shell.model.ui.selected_session = Some("a".into());
            shell.model.attached_session = Some("a".into());
        });
        let text = "long draft é中\n".repeat(30);
        set_draft(&shell, cx, &text);
        let toggle = cx.debug_bounds("composer-expand-toggle").unwrap();
        cx.simulate_click(toggle.center(), gpui::Modifiers::none());
        shell.update(cx, |shell, cx| {
            assert!(shell.composer_expanded);
            shell.submit_prompt(cx);
            assert!(shell.composer_expanded);
            assert_eq!(shell.model.ui.composer, text);
        });
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.command_tx = Some(sender);
            shell.submit_prompt(cx);
            assert!(!shell.composer_expanded);
            assert!(shell.model.ui.composer.is_empty());
        });
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::Prompt { text: sent, .. } if sent == text)
        );
        set_draft(&shell, cx, &text);
        let toggle = cx.debug_bounds("composer-expand-toggle").unwrap();
        cx.simulate_click(toggle.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            shell.switch_draft_owner(Some("a"));
            assert!(shell.composer_expanded);
        });
        let thumb = cx.debug_bounds("composer-scrollbar-thumb").unwrap();
        cx.simulate_mouse_down(
            thumb.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        shell.update(cx, |shell, cx| {
            assert!(shell.composer_scrollbar_grab_offset.is_some());
            shell.switch_draft_owner(Some("b"));
            assert!(!shell.composer_expanded);
            assert!(shell.composer_scrollbar_grab_offset.is_none());
            cx.notify();
        });
        cx.simulate_mouse_move(
            thumb.origin,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.composer_scroll_offset, Pixels::ZERO);
            assert!(shell.model.ui.composer.is_empty());
            shell.switch_draft_owner(Some("a"));
            assert_eq!(shell.model.ui.composer, text);
            assert!(!shell.composer_expanded);
            shell.quitting = true;
            cx.notify();
        });
    }

    #[gpui::test]
    fn overflow_cues_follow_visual_rows_and_disappear_when_the_draft_fits(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        for width in [420., 900.] {
            cx.simulate_resize(gpui::size(px(width), px(800.)));
            for (text, overflow) in [
                ("one\ntwo\nthree\nfour", false),
                ("one\ntwo\nthree\nfour\n", true),
                ("", false),
            ] {
                set_draft(&shell, cx, text);
                assert_eq!(cx.debug_bounds("composer-scrollbar").is_some(), overflow);
                assert_eq!(
                    cx.debug_bounds("composer-expand-toggle").is_some(),
                    overflow
                );
                assert_eq!(cx.debug_bounds("composer-draft-info").is_some(), overflow);
            }
            set_draft(&shell, cx, &"wrapped draft é中🙂 ".repeat(100));
            let thumb = cx
                .debug_bounds("composer-scrollbar-thumb")
                .expect("soft-wrapped drafts reveal their overflow");
            let track = cx.debug_bounds("composer-scrollbar").unwrap();
            assert!(thumb.top() >= track.top() && thumb.bottom() <= track.bottom());
            let composer = cx.debug_bounds("composer").unwrap();
            assert!(track.left() >= composer.left() && track.right() <= composer.right());
        }
        let text = "line with wrapped text é中 ".repeat(10);
        for (width, overflow) in [(420., true), (900., false), (420., true)] {
            cx.simulate_resize(gpui::size(px(width), px(800.)));
            set_draft(&shell, cx, &text);
            assert_eq!(
                cx.debug_bounds("composer-scrollbar").is_some(),
                overflow,
                "wrapping determines overflow at {width}px"
            );
            shell.update(cx, |shell, _| {
                assert_eq!(shell.composer_line_count > 4, overflow)
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn expansion_and_keyboard_collapse_preserve_unicode_selection_and_native_geometry(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_resize(gpui::size(px(420.), px(800.)));
        let text = "first é🙂\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\nlast 拼音";
        set_draft(&shell, cx, text);
        shell.update(cx, |shell, cx| {
            shell.composer_selection = 0..text.len();
            shell.composer_selection_reversed = true;
            shell.composer_marked_range = Some(0.."first é🙂".len());
            cx.notify();
        });
        cx.run_until_parked();
        let expand = cx
            .debug_bounds("composer-expand-toggle")
            .expect("long drafts can expand");
        cx.simulate_click(expand.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                let field = shell.composer_bounds.unwrap();
                assert_eq!(field.size.height, px(192.));
                assert_eq!(shell.model.ui.composer, text);
                assert_eq!(shell.composer_selection, 0..text.len());
                assert!(shell.composer_selection_reversed);
                assert_eq!(shell.composer_marked_range, Some(0.."first é🙂".len()));
                assert!(shell.composer_focus_handle.is_focused(window));
                let caret = shell.bounds_for_range(0..0, field, window, cx).unwrap();
                assert!(caret.top() >= field.top() && caret.bottom() <= field.bottom());
                window.focus_prev(cx);
            });
        });
        assert!(
            cx.debug_bounds("composer-expand-toggle").is_some(),
            "Collapse remains available when expansion removes scrolling"
        );
        cx.run_until_parked();
        let keystroke = gpui::Keystroke::parse("enter").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.composer_bounds.unwrap().size.height, px(96.));
            assert_eq!(shell.model.ui.composer, text);
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn manual_scroll_survives_toggling_and_scrollbar_drag_does_not_select_text(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_resize(gpui::size(px(420.), px(800.)));
        let text = "a draft line é中\n".repeat(40);
        set_draft(&shell, cx, &text);
        let field = shell.update(cx, |shell, _| shell.composer_bounds.unwrap());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: field.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(10000.))),
            ..Default::default()
        });
        cx.run_until_parked();
        for _ in 0..2 {
            let toggle = cx
                .debug_bounds("composer-expand-toggle")
                .expect("long drafts can expand and collapse");
            cx.simulate_click(toggle.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            shell.update(cx, |shell, _| {
                assert_eq!(
                    shell.composer_scroll_offset,
                    Pixels::ZERO,
                    "reading the beginning must survive toggling"
                )
            });
        }
        let track = cx.debug_bounds("composer-scrollbar").unwrap();
        let thumb = cx.debug_bounds("composer-scrollbar-thumb").unwrap();
        let target = point(track.center().x, track.bottom() + px(40.));
        cx.simulate_mouse_down(
            thumb.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_move(target, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(target, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.composer_scroll_offset, shell.composer_scroll_max);
            assert_eq!(shell.composer_selection, text.len()..text.len());
            assert!(!shell.composer_is_selecting);
            assert_eq!(shell.model.ui.composer, text);
        });
        cx.simulate_mouse_move(
            track.origin,
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.composer_scroll_offset, shell.composer_scroll_max,
                "release outside ends the drag"
            );
        });
        let track = cx.debug_bounds("composer-scrollbar").unwrap();
        cx.simulate_click(
            point(track.center().x, track.top() + px(1.)),
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.composer_scroll_offset,
                Pixels::ZERO,
                "the track can return to the beginning"
            );
            assert_eq!(shell.composer_selection, text.len()..text.len());
            shell.quitting = true;
        });
    }
}
