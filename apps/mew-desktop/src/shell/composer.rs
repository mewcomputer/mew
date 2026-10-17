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
}

pub(super) struct TextInputLayout {
    target: TextInputTarget,
    text: SharedString,
    lines: Vec<WrappedLine>,
    font: gpui::Font,
    font_size: Pixels,
    line_height: Pixels,
    bounds: Bounds<Pixels>,
    selection: Range<usize>,
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
        let start_position = cursor_position(&self.lines, text, range.start, self.line_height)?;
        let mut line_start = 0;
        for line in &self.lines {
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
                        bounds.top() + start_position.y,
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
    // Native text services can request candidate bounds before the next paint.
    fn refresh_text_input_layout(&mut self, target: TextInputTarget, window: &Window) {
        let text = self.text_input_value(target);
        let Some(layout) = self.text_input_layout.as_ref() else {
            return;
        };
        if layout.target != target || layout.text.as_ref() == text {
            return;
        }
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
            Some(if target == TextInputTarget::Composer {
                3
            } else {
                1
            }),
        ) else {
            return;
        };
        let layout = self.text_input_layout.as_mut().unwrap();
        layout.text = text;
        layout.lines = lines.into_vec();
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
            self.replace_text_in_range(None, &text, window, cx);
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
        self.composer_selection = offset..offset;
        self.composer_selection_reversed = false;
        self.restart_composer_cursor_blink(cx);
    }

    fn select_composer_to(&mut self, offset: usize, cx: &mut Context<Self>) {
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
        if self.composer_selection.is_empty() {
            let cursor = self.composer_cursor_offset();
            let start = previous_utf8_boundary(&self.model.ui.composer, cursor);
            if start == cursor {
                window.play_system_bell();
                return;
            }
            self.select_composer_to(start, cx);
        }
        self.replace_composer_text(None, "", cx);
    }

    pub(super) fn composer_delete(
        &mut self,
        _: &ComposerDelete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_selection.is_empty() {
            let cursor = self.composer_cursor_offset();
            let end = next_utf8_boundary(&self.model.ui.composer, cursor);
            if end == cursor {
                window.play_system_bell();
                return;
            }
            self.select_composer_to(end, cx);
        }
        self.replace_composer_text(None, "", cx);
    }

    fn replace_composer_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        let current = self.model.ui.composer.clone();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(&current, range.start)
                    ..byte_offset_for_utf16(&current, range.end)
            })
            .or_else(|| self.composer_marked_range.clone())
            .unwrap_or_else(|| self.composer_selection.clone());
        self.replace_composer_byte_range(range, replacement, cx);
    }

    /// Replaces a byte range of the composer text and moves the cursor to
    /// the end of the replacement.
    pub(super) fn replace_composer_byte_range(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        let mut updated = self.model.ui.composer.clone();
        self.initial_auto_attach = false;
        self.session_requests.cancel_initial_prompt();
        updated.replace_range(range.clone(), replacement);
        let cursor = range.start + replacement.len();
        self.composer_selection = cursor..cursor;
        self.composer_selection_reversed = false;
        self.composer_marked_range = None;
        self.model.ui.set_composer(updated);
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
        let current = self.model.ui.composer.clone();
        self.initial_auto_attach = false;
        self.session_requests.cancel_initial_prompt();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(&current, range.start)
                    ..byte_offset_for_utf16(&current, range.end)
            })
            .or_else(|| self.composer_marked_range.clone())
            .unwrap_or_else(|| self.composer_selection.clone());
        let mut updated = current;
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
        let lines = window
            .text_system()
            .shape_text(
                content.clone().into(),
                style.font_size.to_pixels(window.rem_size()),
                &[run],
                Some(bounds.size.width),
                Some(3),
            )
            .unwrap_or_default();
        let relative = point(position.x - bounds.left(), position.y - bounds.top());
        let mut search_offset = 0;
        let mut line_y = Pixels::ZERO;
        for line in &lines {
            let Some(line_start) = content
                .get(search_offset..)
                .and_then(|text| text.find(line.text.as_ref()))
                .map(|offset| search_offset + offset)
            else {
                continue;
            };
            let line_height = line.size(window.line_height()).height;
            if relative.y <= line_y + line_height {
                let local = line
                    .closest_index_for_position(
                        point(relative.x, relative.y - line_y),
                        window.line_height(),
                    )
                    .unwrap_or_else(|offset| offset);
                return (line_start + local).min(content.len());
            }
            search_offset = line_start + line.len();
            if content.as_bytes().get(search_offset) == Some(&b'\n') {
                search_offset += 1;
            }
            line_y += line_height;
        }
        content.len()
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
            let offset = self.composer_index_for_position(event.position, window);
            self.select_composer_to(offset, cx);
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
        style.size.height = match self.target {
            TextInputTarget::Composer => {
                let shell = self.shell.read(cx);
                let content = shell.text_input_value(self.target).to_owned();
                let width = shell
                    .composer_bounds
                    .map(|bounds| bounds.size.width)
                    .unwrap_or(px(480.));
                let text_style = window.text_style();
                let line_count = if content.is_empty() {
                    1
                } else {
                    let run = TextRun {
                        len: content.len(),
                        font: text_style.font(),
                        color: text_style.color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    window
                        .text_system()
                        .shape_text(
                            content.into(),
                            text_style.font_size.to_pixels(window.rem_size()),
                            &[run],
                            Some(width),
                            Some(3),
                        )
                        .map(|lines| {
                            lines
                                .iter()
                                .map(|line| line.wrap_boundaries().len() + 1)
                                .sum()
                        })
                        .unwrap_or(1)
                };
                px(composer_input_height(
                    line_count,
                    f32::from(window.line_height()),
                ))
            }
            TextInputTarget::BrowserUrl => px(28.),
            TextInputTarget::Rename => px(20.),
            TextInputTarget::SidebarSearch => px(20.),
            TextInputTarget::ModelSearch => px(20.),
        }
        .into();
        (window.request_layout(style, [], cx), ())
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
                Some(if self.target == TextInputTarget::Composer {
                    3
                } else {
                    1
                }),
            )
            .unwrap_or_default();
        let cursor_text = if content.is_empty() {
            &display_text
        } else {
            &content
        };
        let cursor = if !selection.is_empty()
            || (self.target == TextInputTarget::Composer && !shell.composer_cursor_visible)
        {
            None
        } else {
            cursor_position(&lines, cursor_text, cursor, window.line_height())
        };
        ComposerPrepaint {
            text: content,
            lines: lines.into_vec(),
            font: style.font(),
            font_size: style.font_size.to_pixels(window.rem_size()),
            cursor,
            line_height: window.line_height(),
        }
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
        self.shell.update(cx, |shell, _| match target {
            TextInputTarget::Composer => shell.composer_bounds = Some(bounds),
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
        let mut line_y = bounds.top();
        for line in &prepaint.lines {
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
            line_y += line.size(prepaint.line_height).height;
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
                    });
            let layout = TextInputLayout {
                target: self.target,
                text: std::mem::take(&mut prepaint.text).into(),
                lines: std::mem::take(&mut prepaint.lines),
                font: std::mem::take(&mut prepaint.font),
                font_size: prepaint.font_size,
                line_height: prepaint.line_height,
                bounds,
                selection,
            };
            self.shell.update(cx, |shell, _| {
                shell.text_input_layout = Some(layout);
            });
            if geometry_changed {
                window.invalidate_character_coordinates();
            }
        }
    }
}

pub(super) fn composer_input_height(line_count: usize, line_height: f32) -> f32 {
    (line_count.max(1) as f32 * line_height).clamp(48., 96.)
}

fn cursor_position(
    lines: &[WrappedLine],
    text: &str,
    cursor: usize,
    line_height: Pixels,
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
            TextInputTarget::Composer => self.composer_marked_range = None,
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
        _cx: &mut Context<Self>,
    ) {
        let target = self.text_input_target(window);
        let text = self.text_input_value(target);
        let selection = byte_offset_for_utf16(text, range_utf16.start)
            ..byte_offset_for_utf16(text, range_utf16.end);
        self.set_text_input_selection(target, selection);
        match target {
            TextInputTarget::Composer => self.composer_selection_reversed = false,
            TextInputTarget::BrowserUrl => self.browser_url_selection_reversed = false,
            TextInputTarget::Rename => self.rename_selection_reversed = false,
            TextInputTarget::SidebarSearch => self.sidebar_search_selection_reversed = false,
            TextInputTarget::ModelSearch => self.model_picker_selection_reversed = false,
        }
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
            lines,
            font: gpui::font("Helvetica"),
            font_size: px(16.),
            line_height: px(24.),
            bounds: Bounds::new(point(px(0.), px(0.)), gpui::size(px(width), px(96.))),
            selection: 0..0,
        }
    }

    fn input_bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(80.), px(120.)), gpui::size(px(400.), px(96.)))
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
                assert_eq!(layout.lines[0].wrap_boundaries().len(), 2);
                let field = shell.composer_bounds.unwrap();
                assert_eq!(field.size.height, layout.line_height * 3.);
                let caret = shell
                    .bounds_for_range(text.len()..text.len(), field, window, cx)
                    .unwrap();
                assert!(caret.bottom() <= field.bottom());
                shell.quitting = true;
            });
        });
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
                    shell.auxiliary_view = AuxiliaryView::Browser;
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
                shell.auxiliary_view = AuxiliaryView::Browser;
                shell.browser_panel_open = true;
                cx.notify();
                shell.browser_url_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| assert!(window.is_action_available(&TextPaste, cx)));
        shell.update(cx, |shell, cx| shell.close_browser_panel(cx));
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
