use super::session_data::save_clipboard_image;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextInputTarget {
    Composer,
    BrowserUrl,
    Rename,
}

pub(super) struct ComposerElement {
    pub(super) shell: Entity<DesktopShell>,
    pub(super) target: TextInputTarget,
}

pub(super) struct ComposerPrepaint {
    lines: Vec<WrappedLine>,
    cursor: Option<Point<Pixels>>,
    line_height: Pixels,
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
    fn text_input_target(&self, window: &Window) -> TextInputTarget {
        if self.browser_url_focus_handle.is_focused(window) {
            TextInputTarget::BrowserUrl
        } else if self.rename_focus_handle.is_focused(window) {
            TextInputTarget::Rename
        } else {
            TextInputTarget::Composer
        }
    }

    fn text_input_value(&self, target: TextInputTarget) -> &str {
        match target {
            TextInputTarget::Composer => &self.model.ui.composer,
            TextInputTarget::BrowserUrl => &self.browser_url,
            TextInputTarget::Rename => &self.rename_draft,
        }
    }

    fn text_input_selection(&self, target: TextInputTarget) -> Range<usize> {
        match target {
            TextInputTarget::Composer => self.composer_selection.clone(),
            TextInputTarget::BrowserUrl => {
                normalized_browser_url_range(&self.browser_url, &self.browser_url_selection)
            }
            TextInputTarget::Rename => self.rename_selection.clone(),
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
        }
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

    /// Paste into the composer: clipboard image data is written to a temp
    /// file and attached; text is inserted at the cursor.
    pub(super) fn composer_paste(
        &mut self,
        _: &ComposerPaste,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
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
        if let Some(text) = item.text() {
            self.replace_composer_text(None, &text, cx);
        }
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

    pub(super) fn composer_select_all(
        &mut self,
        _: &ComposerSelectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_composer_to(0, cx);
        self.select_composer_to(self.model.ui.composer.len(), cx);
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

    pub(super) fn browser_url_select_all(&mut self, cx: &mut Context<Self>) {
        self.browser_url_selection = 0..self.browser_url.len();
        self.browser_url_selection_reversed = false;
        cx.notify();
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
        if let Some(portal) = self.browser_portal.as_ref() {
            let _ = portal.blur(BROWSER_OWNER);
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
                        .map(|lines| lines.len())
                        .unwrap_or(1)
                };
                px(composer_input_height(
                    line_count,
                    f32::from(window.line_height()),
                ))
            }
            TextInputTarget::BrowserUrl => px(28.),
            TextInputTarget::Rename => px(20.),
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
        let display_text = if content.is_empty() && self.target == TextInputTarget::Composer {
            if shell.plan_feedback_request.is_some() {
                "Plan feedback… (Enter to send, Esc to cancel)".to_owned()
            } else {
                "Do anything…".to_owned()
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
        let selection_background: gpui::Hsla = theme_rgb(&shell.theme, "selection.background")
            .opacity(0.18)
            .into();
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
            lines: lines.into_vec(),
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
        let focus_handle = match self.target {
            TextInputTarget::Composer => self.shell.read(cx).composer_focus_handle.clone(),
            TextInputTarget::BrowserUrl => self.shell.read(cx).browser_url_focus_handle.clone(),
            TextInputTarget::Rename => self.shell.read(cx).rename_focus_handle.clone(),
        };
        let target = self.target;
        self.shell.update(cx, |shell, _| match target {
            TextInputTarget::Composer => shell.composer_bounds = Some(bounds),
            TextInputTarget::BrowserUrl => shell.browser_url_bounds = Some(bounds),
            TextInputTarget::Rename => {}
        });
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.shell.clone()),
            cx,
        );
        let mut line_y = bounds.top();
        for line in &prepaint.lines {
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
        let cursor_belongs_here = cursor < line_end
            || (cursor == line_end && text.as_bytes().get(cursor) != Some(&b'\n'));
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
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.text_input_target(window) {
            TextInputTarget::Composer => self.replace_composer_text(range_utf16, text, cx),
            TextInputTarget::BrowserUrl => self.replace_browser_url_text(range_utf16, text, cx),
            TextInputTarget::Rename => self.replace_rename_text(range_utf16, text, cx),
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.text_input_target(window) {
            TextInputTarget::Composer => {
                self.replace_composer_and_mark(range_utf16, text, new_selected_range_utf16, cx)
            }
            TextInputTarget::BrowserUrl => {
                self.replace_browser_url_and_mark(range_utf16, text, new_selected_range_utf16, cx)
            }
            TextInputTarget::Rename => {
                self.replace_rename_and_mark(range_utf16, text, new_selected_range_utf16, cx)
            }
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
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
        }
    }

    fn text_length_utf16(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        let target = self.text_input_target(window);
        Some(self.text_input_value(target).encode_utf16().count())
    }
}

#[cfg(test)]
mod tests {
    use super::normalized_browser_url_range;

    #[test]
    fn stale_browser_url_ranges_are_clamped_to_valid_boundaries() {
        assert_eq!(normalized_browser_url_range("https://é", &(2..999)), 2..10);
        assert_eq!(
            normalized_browser_url_range("short", &std::ops::Range { start: 99, end: 3 },),
            3..5
        );
    }
}
