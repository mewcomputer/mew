use super::chat_render::{transcript_scrollbar_metrics, TranscriptScrollbarMetrics};
use super::*;

#[derive(Clone, Copy)]
pub(super) enum ToolTextSection {
    Input,
    Output,
    Diff,
}

impl ToolTextSection {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Output => 1,
            Self::Diff => 2,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum TranscriptCopySource {
    Code {
        message_index: usize,
        block_index: usize,
    },
    Tool {
        message_index: usize,
        part_index: usize,
        section: ToolTextSection,
    },
}

impl DesktopShell {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_inline(
        &self,
        inline: &InlineText,
        message_index: usize,
        block_index: usize,
        source_offset: usize,
        animate: bool,
        fill_width: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let highlights = self.transcript_text_highlights(
            message_index,
            block_index,
            source_offset,
            inline.text.len(),
            inline
                .highlights
                .iter()
                .map(|highlight| {
                    (
                        highlight.range.clone(),
                        self.inline_highlight_style(highlight.style),
                    )
                })
                .collect(),
        );
        let code_font_overrides = inline
            .highlights
            .iter()
            .filter(|highlight| highlight.style == InlineStyle::Code)
            .map(|highlight| (highlight.range.clone(), DEFAULT_FONT_FAMILY.into()));
        let code_font_overrides = code_font_overrides.collect::<Vec<_>>();
        let mut text = gpui::StyledText::new(inline.text.clone());
        if !code_font_overrides.is_empty() {
            text = text.with_font_family_overrides(code_font_overrides);
        }
        if !highlights.is_empty() {
            text = text.with_highlights(highlights);
        }
        let element = self
            .render_selectable_transcript_text(
                message_index,
                block_index,
                source_offset,
                &inline.text,
                &inline.links,
                text,
                cx,
            )
            .when(fill_width, |element| element.w_full())
            .whitespace_normal();
        if !animate {
            return element.into_any_element();
        }
        element
            .with_animation(
                ElementId::Name(
                    format!("markdown-inline-{message_index}-{block_index}-{source_offset}").into(),
                ),
                Animation::new(Duration::from_millis(180)).with_easing(gpui::ease_out_quint()),
                |element, delta| element.opacity(0.72 + delta * 0.28),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_selectable_transcript_text(
        &self,
        message_index: usize,
        block_index: usize,
        source_offset: usize,
        source: &str,
        links: &[InlineLink],
        text: gpui::StyledText,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        // Keep the interaction layout tied to the actual element being
        // painted. A separately-created TextLayout is never measured by
        // GPUI, so querying it from a mouse or accessibility event panics.
        let down_layout = text.layout().clone();
        let down_text = source.to_owned();
        let down_links = links.to_vec();
        // The registry is only consulted while a transcript selection is active.
        // Avoid rebuilding it for every visible markdown span during ordinary scrolling.
        let container = div().when(self.transcript_selection.is_some(), |element| {
            let registry = self.transcript_text_registry.clone();
            let entry = TranscriptTextEntry {
                message_index,
                block_index,
                source_offset,
                text: source.to_owned(),
                layout: down_layout.clone(),
            };
            element.on_children_prepainted(move |_, _, _| {
                registry.borrow_mut().push(entry.clone());
            })
        });
        let id = if source_offset == 0 {
            format!("markdown-inline-container-{message_index}-{block_index}")
        } else {
            format!("markdown-inline-container-{message_index}-{block_index}-{source_offset}")
        };
        let selector = id.clone();
        container
            .id(id)
            .debug_selector(move || selector.clone())
            .min_w_0()
            .cursor(gpui::CursorStyle::IBeam)
            .when(!links.is_empty(), |element| {
                element.cursor(gpui::CursorStyle::PointingHand)
            })
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |shell, event, window, cx| {
                    shell.transcript_mouse_down(
                        message_index,
                        block_index,
                        source_offset,
                        &down_text,
                        &down_layout,
                        &down_links,
                        event,
                        window,
                        cx,
                    )
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(Self::transcript_mouse_up),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(Self::transcript_mouse_up),
            )
            .child(text)
    }

    fn merge_selection_highlights(
        &self,
        text_len: usize,
        highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
        selection: Option<Range<usize>>,
    ) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        let mut boundaries = vec![0, text_len];
        for (range, _) in &highlights {
            boundaries.extend([range.start, range.end]);
        }
        if let Some(selection) = &selection {
            boundaries.push(selection.start);
            boundaries.push(selection.end);
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        boundaries
            .windows(2)
            .filter_map(|window| {
                let range = window[0]..window[1];
                let style = highlights
                    .iter()
                    .find(|(highlight, _)| highlight.contains(&range.start))
                    .map(|(_, style)| *style)
                    .unwrap_or_default();
                let selected = selection.as_ref().is_some_and(|selection| {
                    selection.start < range.end && range.start < selection.end
                });
                if !selected && style == gpui::HighlightStyle::default() {
                    return None;
                }
                let mut style = style;
                if selected {
                    style.background_color =
                        Some(theme_rgb(&self.theme, "selection.background").into());
                    style.color = Some(theme_rgb(&self.theme, "selection.foreground").into());
                }
                Some((range, style))
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn transcript_mouse_down(
        &mut self,
        message_index: usize,
        block_index: usize,
        source_offset: usize,
        text: &str,
        layout: &gpui::TextLayout,
        links: &[InlineLink],
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = transcript_index_for_position(layout, event.position, text);
        let point = TranscriptSelectionPoint {
            message_index,
            block_index,
            offset: source_offset + offset,
        };
        self.transcript_link_candidate = links
            .iter()
            .find(|link| link.range.contains(&offset))
            .map(|link| link.url.clone());
        self.transcript_selection_anchor = Some(point);
        self.update_transcript_selection(point);
        self.transcript_is_selecting = true;
        window.focus(&self.transcript_focus_handle, cx);
        cx.notify();
    }

    pub(super) fn transcript_scrollbar_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(metrics) = transcript_scrollbar_metrics(&self.transcript_list) else {
            return;
        };
        let viewport = self.transcript_list.viewport_bounds();
        let pointer_y = f32::from(event.position.y) - f32::from(viewport.origin.y);
        if !(0.0..=metrics.viewport_height).contains(&pointer_y) {
            return;
        }

        let grab_offset = if (metrics.thumb_top..=metrics.thumb_top + metrics.thumb_height)
            .contains(&pointer_y)
        {
            pointer_y - metrics.thumb_top
        } else {
            metrics.thumb_height / 2.
        };
        self.transcript_scrollbar_grab_offset = Some(grab_offset);
        self.transcript_is_selecting = false;
        self.transcript_link_candidate = None;
        self.transcript_list.scrollbar_drag_started();
        self.update_transcript_scrollbar(pointer_y, grab_offset, metrics, cx);
    }

    fn update_transcript_scrollbar(
        &mut self,
        pointer_y: f32,
        grab_offset: f32,
        metrics: TranscriptScrollbarMetrics,
        cx: &mut Context<Self>,
    ) {
        let thumb_top = (pointer_y - grab_offset).clamp(0., metrics.travel);
        let scroll_offset = if metrics.travel > 0. {
            thumb_top / metrics.travel * metrics.max_offset
        } else {
            0.
        };
        self.transcript_list
            .set_follow_mode(gpui::FollowMode::Normal);
        self.transcript_list
            .set_offset_from_scrollbar(point(px(0.), px(-scroll_offset)));
        let at_end = scroll_offset >= (metrics.max_offset - 1.).max(0.);
        if at_end {
            self.transcript_list.set_follow_mode(gpui::FollowMode::Tail);
            self.transcript_list.scroll_to_end();
        }
        self.transcript_scrolled_away = !at_end;
        self.capture_session_view_state();
        cx.notify();
    }

    pub(super) fn transcript_mouse_move_at_position(
        &mut self,
        event: &gpui::MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(grab_offset) = self.transcript_scrollbar_grab_offset {
            if event.dragging() {
                if let Some(metrics) = transcript_scrollbar_metrics(&self.transcript_list) {
                    let viewport = self.transcript_list.viewport_bounds();
                    let pointer_y = f32::from(event.position.y) - f32::from(viewport.origin.y);
                    self.update_transcript_scrollbar(pointer_y, grab_offset, metrics, cx);
                }
            }
            return;
        }
        if !self.transcript_is_selecting || !event.dragging() {
            return;
        }
        let Some(anchor) = self.transcript_selection_anchor else {
            return;
        };
        let registry = self.transcript_text_registry.borrow();
        let Some(entry) = registry
            .iter()
            .filter(|entry| {
                let bounds = entry.layout.bounds();
                event.position.y >= bounds.top() && event.position.y <= bounds.bottom()
            })
            .min_by(|a, b| {
                let distance = |entry: &TranscriptTextEntry| {
                    let bounds = entry.layout.bounds();
                    f32::from(
                        (bounds.left() - event.position.x)
                            .max(Pixels::ZERO)
                            .max(event.position.x - bounds.right()),
                    )
                };
                distance(a).total_cmp(&distance(b))
            })
        else {
            return;
        };
        let offset = transcript_index_for_position(&entry.layout, event.position, &entry.text);
        let point = TranscriptSelectionPoint {
            message_index: entry.message_index,
            block_index: entry.block_index,
            offset: entry.source_offset + offset,
        };
        drop(registry);
        if point != anchor {
            self.transcript_link_candidate = None;
            self.update_transcript_selection(point);
        }
        cx.notify();
    }

    pub(super) fn transcript_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.transcript_scrollbar_grab_offset.take().is_some() {
            self.transcript_list.scrollbar_drag_ended();
            self.capture_session_view_state();
            cx.notify();
            return;
        }
        self.transcript_is_selecting = false;
        if let Some(url) = self.transcript_link_candidate.take() {
            self.open_transcript_link(url, cx);
        }
    }

    fn transcript_selection_range(
        &self,
        message_index: usize,
        block_index: usize,
        text_len: usize,
    ) -> Option<Range<usize>> {
        self.transcript_selection.as_ref().and_then(|selection| {
            Self::selection_range_for(selection, message_index, block_index, text_len)
        })
    }

    pub(super) fn selection_range_for(
        selection: &TranscriptSelection,
        message_index: usize,
        block_index: usize,
        text_len: usize,
    ) -> Option<Range<usize>> {
        let (start, end) = if selection.start <= selection.end {
            (selection.start, selection.end)
        } else {
            (selection.end, selection.start)
        };
        let id = (message_index, block_index);
        let start_id = (start.message_index, start.block_index);
        let end_id = (end.message_index, end.block_index);
        if id < start_id || id > end_id {
            return None;
        }
        let range_start = if id == start_id { start.offset } else { 0 };
        let range_end = if id == end_id { end.offset } else { text_len };
        let range = range_start.min(text_len)..range_end.min(text_len);
        (!range.is_empty()).then_some(range)
    }

    fn update_transcript_selection(&mut self, point: TranscriptSelectionPoint) {
        let Some(anchor) = self.transcript_selection_anchor else {
            return;
        };
        let selection = TranscriptSelection {
            start: anchor,
            end: point,
        };
        self.transcript_selected_text = self.transcript_selection_text(&selection);
        self.transcript_selection = Some(selection);
    }

    fn transcript_selection_text(&self, selection: &TranscriptSelection) -> Option<String> {
        let registry = self.transcript_text_registry.borrow();
        let mut sources = BTreeMap::new();
        let (start, end) = if selection.start <= selection.end {
            (selection.start, selection.end)
        } else {
            (selection.end, selection.start)
        };
        let selected_ids =
            (start.message_index, start.block_index)..=(end.message_index, end.block_index);
        for entry in registry.iter() {
            let id = (entry.message_index, entry.block_index);
            if selected_ids.contains(&id) {
                sources.entry(id).or_insert_with(|| {
                    (
                        self.tool_text_for_selection(entry.message_index, entry.block_index)
                            .or_else(|| {
                                self.markdown_text_for_selection(
                                    entry.message_index,
                                    entry.block_index,
                                )
                            })
                            .unwrap_or(Cow::Borrowed(entry.text.as_str())),
                        false,
                    )
                });
            }
        }
        // Structured content can span virtual rows outside the painted registry.
        // Read those chunks from the retained projection when copying.
        let first_row = self
            .transcript_rows
            .partition_point(|row| row.message_index < start.message_index);
        let last_row = self
            .transcript_rows
            .partition_point(|row| row.message_index <= end.message_index);
        for row in &self.transcript_rows[first_row..last_row] {
            let block_index = row
                .part_index
                .saturating_mul(1_000_000)
                .saturating_add(row.block_index);
            let id = (row.message_index, block_index);
            if !selected_ids.contains(&id) {
                continue;
            }
            if matches!(
                self.model
                    .ui
                    .transcript
                    .get(row.message_index)
                    .and_then(|item| item.parts.get(row.part_index)),
                Some(TranscriptPart::Reasoning(_))
            ) && !self
                .expanded_chat_parts
                .contains(&Self::chat_part_key(row.message_index, row.part_index))
            {
                continue;
            }
            let Some(block) = self
                .markdown_cache
                .get(row.message_index)
                .and_then(|parts| parts.get(row.part_index))
                .and_then(|cached| cached.render_blocks.get(row.block_index))
            else {
                continue;
            };
            match &block.block {
                MarkdownBlock::Code { text, .. } => {
                    sources.insert(id, (Cow::Borrowed(text.as_str()), block.continuation));
                }
                MarkdownBlock::List(_) | MarkdownBlock::Table(_) => {
                    if let Some(text) =
                        self.markdown_text_for_selection(row.message_index, block_index)
                    {
                        sources.insert(id, (text, false));
                    }
                }
                _ => {}
            }
        }
        let mut selected = String::new();
        for ((message_index, block_index), (text, continuation)) in sources {
            let Some(range) =
                Self::selection_range_for(selection, message_index, block_index, text.len())
            else {
                continue;
            };
            let Some(text) = text.get(range) else {
                continue;
            };
            if !selected.is_empty() && !continuation {
                selected.push('\n');
            }
            selected.push_str(text);
        }
        (!selected.is_empty()).then_some(selected)
    }

    fn copy_transcript_selection(&self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.transcript_selected_text.as_deref() else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_owned()));
        true
    }

    pub(super) fn copy_selected_transcript_text(
        &mut self,
        _: &TextCopy,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.copy_transcript_selection(cx) {
            cx.propagate();
        }
    }

    fn tool_text_for_selection(
        &self,
        message_index: usize,
        block_index: usize,
    ) -> Option<Cow<'_, str>> {
        let part_index = block_index / 1_000_000;
        let TranscriptPart::ToolCall {
            input,
            output,
            diff,
            ..
        } = self
            .model
            .ui
            .transcript
            .get(message_index)?
            .parts
            .get(part_index)?
        else {
            return None;
        };
        match block_index % 1_000_000 {
            0 => Some(Cow::Owned(super::chat_render::formatted_tool_input(input))),
            1 => output.as_deref().map(Cow::Borrowed),
            2 => diff.as_deref().map(Cow::Borrowed),
            _ => None,
        }
    }

    fn markdown_text_for_selection(
        &self,
        message_index: usize,
        block_index: usize,
    ) -> Option<Cow<'_, str>> {
        let block = &self
            .markdown_cache
            .get(message_index)?
            .get(block_index / 1_000_000)?
            .render_blocks
            .get(block_index % 1_000_000)?
            .block;
        let mut text = String::new();
        match block {
            MarkdownBlock::List(items) => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        text.push('\n');
                    }
                    text.push_str(&item.content.text);
                }
            }
            MarkdownBlock::Table(table) => {
                for (row_index, row) in table.rows.iter().enumerate() {
                    if row_index > 0 {
                        text.push('\n');
                    }
                    for (column_index, cell) in row.iter().enumerate() {
                        if column_index > 0 {
                            text.push('\t');
                        }
                        text.push_str(&cell.text);
                    }
                }
            }
            _ => return None,
        }
        Some(Cow::Owned(text))
    }

    fn transcript_copy_text(&self, source: TranscriptCopySource) -> Option<String> {
        match source {
            TranscriptCopySource::Code {
                message_index,
                block_index,
            } => {
                let blocks = &self
                    .markdown_cache
                    .get(message_index)?
                    .get(block_index / 1_000_000)?
                    .render_blocks;
                let mut index = block_index % 1_000_000;
                while blocks.get(index)?.continuation {
                    index = index.checked_sub(1)?;
                }
                let mut code = String::new();
                for (offset, block) in blocks[index..].iter().enumerate() {
                    if offset > 0 && !block.continuation {
                        break;
                    }
                    let MarkdownBlock::Code { text, .. } = &block.block else {
                        break;
                    };
                    code.push_str(text);
                }
                Some(code)
            }
            TranscriptCopySource::Tool {
                message_index,
                part_index,
                section,
            } => {
                let TranscriptPart::ToolCall {
                    input,
                    output,
                    diff,
                    ..
                } = self
                    .model
                    .ui
                    .transcript
                    .get(message_index)?
                    .parts
                    .get(part_index)?
                else {
                    return None;
                };
                match section {
                    ToolTextSection::Input => Some(super::chat_render::formatted_tool_input(input)),
                    ToolTextSection::Output => output.clone(),
                    ToolTextSection::Diff => diff.clone(),
                }
            }
        }
    }

    pub(super) fn render_content_copy_button(
        &self,
        id: String,
        label: &'static str,
        source: TranscriptCopySource,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let selector = id.clone();
        div()
            .id(id)
            .debug_selector(move || selector.clone())
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(5.))
            .py(px(3.))
            .rounded(px(4.))
            .cursor_pointer()
            .role(Role::Button)
            .aria_label(label)
            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
            .text_xs()
            .text_color(theme_rgb(&self.theme, "text.muted"))
            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
            .on_click(cx.listener(move |shell, _, _, cx| {
                cx.stop_propagation();
                if let Some(text) = shell.transcript_copy_text(source) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }))
            .child("copy")
            .into_any_element()
    }

    pub(super) fn transcript_text_highlights(
        &self,
        message_index: usize,
        block_index: usize,
        source_offset: usize,
        text_len: usize,
        highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    ) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        let Some(range) =
            self.transcript_selection_range(message_index, block_index, source_offset + text_len)
        else {
            return highlights;
        };
        let start = range.start.max(source_offset) - source_offset;
        let end = range
            .end
            .min(source_offset + text_len)
            .saturating_sub(source_offset);
        if start >= end {
            return highlights;
        }
        self.merge_selection_highlights(text_len, highlights, Some(start..end))
    }

    pub(super) fn inline_highlight_style(&self, style: InlineStyle) -> gpui::HighlightStyle {
        match style {
            InlineStyle::Emphasis => gpui::HighlightStyle {
                font_style: Some(gpui::FontStyle::Italic),
                ..Default::default()
            },
            InlineStyle::Strong => gpui::HighlightStyle {
                font_weight: Some(gpui::FontWeight::BOLD),
                ..Default::default()
            },
            InlineStyle::Strikethrough => gpui::HighlightStyle {
                strikethrough: Some(gpui::StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(theme_rgb(&self.theme, "text.muted").into()),
                }),
                ..Default::default()
            },
            InlineStyle::Code => gpui::HighlightStyle {
                background_color: Some(theme_rgb(&self.theme, "muted").into()),
                ..Default::default()
            },
            InlineStyle::Link => gpui::HighlightStyle {
                color: Some(theme_rgb(&self.theme, "primary").into()),
                underline: Some(gpui::UnderlineStyle {
                    thickness: px(1.),
                    color: Some(theme_rgb(&self.theme, "primary").into()),
                    wavy: false,
                }),
                ..Default::default()
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_markdown_block(
        &self,
        block: &MarkdownBlock,
        message_index: usize,
        block_index: usize,
        continuation: bool,
        space_after: bool,
        animate: bool,
        fill_width: bool,
        syntax_highlights: &[MarkdownSyntaxHighlight],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let block_id = format!("markdown-block-{message_index}-{block_index}");
        match block {
            MarkdownBlock::Paragraph(inline) => div()
                .id(block_id)
                .when(space_after, |element| element.mb(px(12.)))
                .child(self.render_inline(
                    inline,
                    message_index,
                    block_index,
                    0,
                    animate,
                    fill_width,
                    cx,
                ))
                .into_any_element(),
            MarkdownBlock::Heading { level, content } => {
                let size = match level {
                    1 => px(24.),
                    2 => px(20.),
                    3 => px(18.),
                    _ => px(16.),
                };
                div()
                    .id(block_id)
                    .when(space_after, |element| element.mb(px(12.)))
                    .text_size(size)
                    .line_height(size + px(8.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(self.render_inline(
                        content,
                        message_index,
                        block_index,
                        0,
                        animate,
                        fill_width,
                        cx,
                    ))
                    .into_any_element()
            }
            MarkdownBlock::List(items) => {
                let mut source_offset = 0;
                div()
                    .id(block_id)
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .when(continuation, |element| element.pt(px(4.)))
                    .when(space_after, |element| element.mb(px(12.)))
                    .children(items.iter().enumerate().map(|(item_index, item)| {
                        let offset = source_offset;
                        source_offset += item.content.text.len() + 1;
                        let marker_id = format!(
                            "markdown-list-marker-{message_index}-{block_index}-{item_index}"
                        );
                        let content_id = format!(
                            "markdown-list-content-{message_index}-{block_index}-{item_index}"
                        );
                        div()
                            .flex()
                            .items_start()
                            .pl(px(item.depth as f32 * 24.))
                            .gap(px(8.))
                            .child(
                                div()
                                    .debug_selector(move || marker_id.clone())
                                    .w(px(32.))
                                    .flex_shrink_0()
                                    .text_align(gpui::TextAlign::Right)
                                    .text_color(theme_rgb(&self.theme, "text.muted"))
                                    .child(item.marker.clone()),
                            )
                            .child(
                                div()
                                    .debug_selector(move || content_id.clone())
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.render_inline(
                                        &item.content,
                                        message_index,
                                        block_index,
                                        offset,
                                        animate,
                                        true,
                                        cx,
                                    )),
                            )
                    }))
                    .into_any_element()
            }
            MarkdownBlock::Quote(lines) => div()
                .id(block_id)
                .flex()
                .flex_col()
                .gap(px(4.))
                .when(space_after, |element| element.mb(px(12.)))
                .border_l_2()
                .border_color(theme_rgb(&self.theme, "text.muted"))
                .pl(px(12.))
                .text_color(theme_rgb(&self.theme, "text.muted"))
                .children(lines.iter().enumerate().map(|(line_index, line)| {
                    self.render_inline(
                        line,
                        message_index,
                        block_index * 1000 + line_index,
                        0,
                        animate,
                        true,
                        cx,
                    )
                }))
                .into_any_element(),
            MarkdownBlock::Code { language, text } => {
                let final_chunk = self
                    .markdown_scroll_group(message_index, block_index)
                    .is_none_or(|(_, group)| group.blocks.end == block_index % 1_000_000 + 1);
                let displayed_text = if final_chunk {
                    text.as_str()
                } else {
                    text.strip_suffix('\n').unwrap_or(text)
                };
                let common_width = self
                    .markdown_code_width(message_index, block_index, window)
                    .unwrap_or(px(0.));
                let mut code = div()
                    .id(block_id)
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .when(space_after, |element| element.mb(px(12.)))
                    .line_height(px(20.))
                    .border_x_1()
                    .border_color(theme_rgb(&self.theme, "divider"))
                    .bg(theme_rgb(&self.theme, "muted"))
                    .px(px(12.))
                    .when(!continuation, |element| {
                        element.rounded_t(px(8.)).border_t_1().pt(px(12.))
                    })
                    .when(final_chunk, |element| {
                        element.rounded_b(px(8.)).border_b_1().pb(px(12.))
                    });
                code = code.when(!continuation, |element| {
                    element.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(language.clone().unwrap_or_default())
                            .child(self.render_content_copy_button(
                                format!("copy-code-{message_index}-{block_index}"),
                                "Copy code",
                                TranscriptCopySource::Code {
                                    message_index,
                                    block_index,
                                },
                                cx,
                            )),
                    )
                });
                let highlights = self
                    .transcript_text_highlights(
                        message_index,
                        block_index,
                        0,
                        text.len(),
                        syntax_highlights
                            .iter()
                            .map(|highlight| {
                                (
                                    highlight.range.clone(),
                                    self.syntax_highlight_style(highlight),
                                )
                            })
                            .collect(),
                    )
                    .into_iter()
                    .filter_map(|(mut range, style)| {
                        range.end = range.end.min(displayed_text.len());
                        (range.start < range.end).then_some((range, style))
                    });
                let code_text = div()
                    .id(format!("markdown-code-text-{message_index}-{block_index}"))
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .when_some(
                        self.markdown_scroll_handle(message_index, block_index),
                        |element, handle| element.track_scroll(handle),
                    )
                    .whitespace_nowrap()
                    .text_xs()
                    .child(
                        self.render_selectable_transcript_text(
                            message_index,
                            block_index,
                            0,
                            text,
                            &[],
                            gpui::StyledText::new(displayed_text.to_owned())
                                .with_font_family_overrides(std::iter::once((
                                    0..displayed_text.len(),
                                    gpui::SharedString::from(DEFAULT_FONT_FAMILY),
                                )))
                                .with_highlights(highlights),
                            cx,
                        )
                        .w_full()
                        .min_w(common_width),
                    );
                if animate {
                    code.child(
                        code_text.with_animation(
                            ElementId::Name(
                                format!("markdown-code-{message_index}-{block_index}").into(),
                            ),
                            Animation::new(Duration::from_millis(180))
                                .with_easing(gpui::ease_out_quint()),
                            |element, delta| element.opacity(0.72 + delta * 0.28),
                        ),
                    )
                } else {
                    code.child(code_text)
                }
                .into_any_element()
            }
            MarkdownBlock::Table(table) => {
                let columns = table.alignments.len().max(1);
                let viewport_id = format!("markdown-table-viewport-{message_index}-{block_index}");
                let mut source_offset = 0;
                div()
                    .id(block_id)
                    .debug_selector(move || viewport_id.clone())
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .when_some(self.markdown_scroll_handle(message_index, block_index), |element, handle| element.track_scroll(handle))
                    .when(space_after, |element| element.mb(px(12.)))
                    .child(div()
                        .flex()
                        .flex_col()
                        .w_full()
                        .min_w(px(columns as f32 * 112.))
                        .children(table.rows.iter().enumerate().map(|(row_index, row)| {
                            let header = table.row_start == 0 && row_index == 0;
                            let mut cells = Vec::with_capacity(row.len());
                            for (column_index, cell) in row.iter().enumerate() {
                                let offset = source_offset;
                                source_offset += cell.text.len() + 1;
                                let cell_id = format!("markdown-table-cell-{message_index}-{block_index}-{row_index}-{column_index}");
                                let align = match table.alignments.get(column_index) {
                                    Some(crate::markdown::MarkdownTableAlignment::Right) => gpui::TextAlign::Right,
                                    Some(crate::markdown::MarkdownTableAlignment::Center) => gpui::TextAlign::Center,
                                    _ => gpui::TextAlign::Left,
                                };
                                // GPUI's TextLayout hit testing ignores text alignment.
                                // Position the content itself so selection matches paint.
                                cells.push(div()
                                    .debug_selector(move || cell_id.clone())
                                    .flex()
                                    .items_start()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(12.))
                                    .py(px(8.))
                                    .when(align == gpui::TextAlign::Right, |element| element.justify_end())
                                    .when(align == gpui::TextAlign::Center, |element| element.justify_center())
                                    .child(self.render_inline(cell, message_index, block_index, offset, animate, false, cx)));
                            }
                            div()
                                .flex()
                                .items_stretch()
                                .w_full()
                                .border_b_1()
                                .border_color(theme_rgb(&self.theme, "divider"))
                                .when(header, |element| element.font_weight(gpui::FontWeight::SEMIBOLD).bg(theme_rgb(&self.theme, "muted")))
                                .children(cells)
                        })))
                    .into_any_element()
            }
            MarkdownBlock::Rule => div()
                .id(block_id)
                .h(px(1.))
                .when(space_after, |element| element.mb(px(12.)))
                .bg(theme_rgb(&self.theme, "divider"))
                .into_any_element(),
            MarkdownBlock::Raw(inline) => div()
                .id(block_id)
                .when(space_after, |element| element.mb(px(12.)))
                .child(self.render_inline(
                    inline,
                    message_index,
                    block_index,
                    0,
                    animate,
                    fill_width,
                    cx,
                ))
                .into_any_element(),
        }
    }

    fn syntax_highlight_style(&self, highlight: &MarkdownSyntaxHighlight) -> gpui::HighlightStyle {
        let mut style = gpui::HighlightStyle::default();
        if let Some([red, green, blue]) = highlight.color {
            style.color = Some(
                rgb((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)).into(),
            );
        }
        if highlight.bold {
            style.font_weight = Some(gpui::FontWeight::BOLD);
        }
        if highlight.italic {
            style.font_style = Some(gpui::FontStyle::Italic);
        }
        if highlight.underline {
            style.underline = Some(gpui::UnderlineStyle {
                thickness: px(1.),
                color: style.color,
                wavy: false,
            });
        }
        style
    }

    pub(super) fn pending_question(&self) -> Option<&PendingAction> {
        self.model
            .ui
            .pending_actions
            .iter()
            .find(|action| matches!(&action.kind, ActionKind::AskUser { .. }))
    }

    pub(super) fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        if self.model.ui.running && self.pending_question().is_none() {
            return;
        }
        if self.model.ui.composer.trim().is_empty() {
            return;
        }
        if !self.model.session_is_ready() || self.session_requests.is_pending() {
            if self.task_drafts.owner().session_id.is_none() {
                if !self.session_requests.is_pending() {
                    self.initial_auto_attach = false;
                    if !self.create_session(None, None) {
                        self.finish_prompt_submission(false, cx);
                        return;
                    }
                }
                let snapshot = PromptDraft {
                    text: self.model.ui.composer.clone(),
                    attachments: self.attachments.clone(),
                };
                if !self
                    .session_requests
                    .set_initial_prompt(self.task_drafts.owner(), snapshot)
                {
                    self.report_pending_attachment();
                }
            } else {
                self.report_pending_attachment();
            }
            cx.notify();
            return;
        }
        if self.awaiting_model_switch.is_some() {
            self.model.last_error = Some(
                "The model is still switching. Your draft is saved; submit again when it is ready."
                    .into(),
            );
            cx.notify();
            return;
        }
        let accepted = if let Some(action) = self.pending_question() {
            let ActionKind::AskUser { questions, .. } = &action.kind else {
                return;
            };
            let draft = self.model.ui.composer.trim();
            let answers = if questions.len() == 1 {
                vec![draft.to_owned()]
            } else {
                let lines = draft.lines();
                if lines.clone().count() != questions.len()
                    || lines.clone().any(|line| line.trim().is_empty())
                {
                    cx.notify();
                    return;
                }
                lines.map(|line| line.trim().to_owned()).collect()
            };
            self.send_command(ClientMessage::AskUserResponse {
                cancelled: false,
                request_id: action.request_id.clone(),
                answers,
            })
        } else if let Some(command) = daemon_slash_command(&self.model.ui.composer) {
            self.send_command(ClientMessage::SlashCommand { command })
        } else {
            session_requests::queue_prompt(
                self.command_tx.as_ref(),
                &mut self.model.ui.composer,
                &mut self.attachments,
                &mut self.prompt_history,
            )
        };
        self.finish_prompt_submission(accepted, cx);
    }

    pub(super) fn finish_prompt_submission(&mut self, accepted: bool, cx: &mut Context<Self>) {
        self.model.finish_prompt_submission(accepted);
        if accepted {
            self.task_drafts.edits.clear();
            self.composer_expanded = false;
            self.composer_scrollbar_grab_offset = None;
            self.composer_preserve_scroll_on_resize = false;
            self.composer_selection = 0..0;
            self.composer_selection_reversed = false;
            self.composer_marked_range = None;
            self.composer_is_selecting = false;
            self.restart_composer_cursor_blink(cx);
            self.input_animation_id = self.input_animation_id.wrapping_add(1);
        }
        cx.notify();
    }

    pub(super) fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        if !self.model.ui.running {
            return;
        }
        self.send_command(ClientMessage::Cancel);
        cx.notify();
    }

    pub(super) fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.model.last_error = None;
        cx.notify();
    }

    pub(super) fn retry_last_turn(&mut self, cx: &mut Context<Self>) {
        if self.model.ui.running || !self.model.session_is_ready() {
            return;
        }
        let Some(submission) = latest_user_submission(&self.model.ui.transcript) else {
            return;
        };
        self.send_command(ClientMessage::Prompt {
            text: submission.text,
            attachments: submission.attachments,
        });
        cx.notify();
    }

    pub(super) fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.pending_transcript_scroll_anchor = None;
        self.transcript_scrollbar_grab_offset = None;
        self.transcript_scrolled_away = false;
        self.transcript_list.set_follow_mode(gpui::FollowMode::Tail);
        self.transcript_list.scroll_to_end();
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn open_transcript_link(&mut self, url: String, cx: &mut Context<Self>) {
        let Some(url) = normalize_browser_url(&url) else {
            self.browser_error = Some("only http(s) links can open in the in-app browser".into());
            cx.notify();
            return;
        };
        if !self.new_work_tab(cx) {
            return;
        }
        self.work_tabs.active_mut().kind = WorkTabKind::Browser;
        self.work_tabs.active_mut().url.clone_from(&url);
        self.browser_url = url;
        self.browser_panel_open = true;
        self.browser_error = None;
        self.capture_session_view_state();
        cx.notify();
    }

    pub(super) fn composer_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        if self.composer_marked_range.is_some() {
            return;
        }
        if self.composer_modified_key_down(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.mention_menu_is_open() {
            match key {
                "down" => {
                    self.mention_menu_move(1, cx);
                    cx.stop_propagation();
                    return;
                }
                "up" => {
                    self.mention_menu_move(-1, cx);
                    cx.stop_propagation();
                    return;
                }
                "tab" => {
                    self.complete_mention_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                "escape" => {
                    self.mention_menu_dismissed = true;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                "enter" if !shift => {
                    self.complete_mention_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        if self.slash_menu_is_open() {
            match key {
                "down" => {
                    self.slash_menu_move(1, cx);
                    cx.stop_propagation();
                    return;
                }
                "up" => {
                    self.slash_menu_move(-1, cx);
                    cx.stop_propagation();
                    return;
                }
                "tab" => {
                    self.complete_slash_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                "escape" => {
                    self.slash_menu_dismissed = true;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                "enter" if !shift => {
                    // A fully typed command submits directly; a partial query
                    // completes the highlighted menu entry first.
                    let matches = self.slash_menu_matches();
                    let exact =
                        matches.len() == 1 && self.model.ui.composer.trim() == matches[0].name;
                    if exact {
                        self.submit_prompt(cx);
                    } else {
                        self.complete_slash_selection(cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        if key == "enter" && !shift {
            self.submit_prompt(cx);
            cx.stop_propagation();
            return;
        }
        if matches!(key, "up" | "down")
            && !event.keystroke.modifiers.platform
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
            && self.composer_move_vertical(key == "up", shift, window, cx)
        {
            cx.stop_propagation();
            return;
        }
        let unmodified = !event.keystroke.modifiers.platform
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
            && !shift;
        if unmodified && key == "up" && self.recall_prompt_history(true, cx) {
            cx.stop_propagation();
            return;
        }
        if unmodified && key == "down" && self.recall_prompt_history(false, cx) {
            cx.stop_propagation();
        }
    }

    pub(super) fn browser_url_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        if matches!(key, "enter" | "return") && !event.keystroke.modifiers.shift {
            self.navigate_browser_to(cx);
            cx.stop_propagation();
        } else if key == "backspace" {
            self.browser_url_backspace(window, cx);
            cx.stop_propagation();
        } else if key == "delete" {
            self.browser_url_delete(window, cx);
            cx.stop_propagation();
        } else if key == "left" {
            self.browser_url_left(cx);
            cx.stop_propagation();
        } else if key == "right" {
            self.browser_url_right(cx);
            cx.stop_propagation();
        } else if !event.keystroke.modifiers.platform
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
        {
            if let Some(text) = event.keystroke.key_char.as_deref() {
                self.replace_browser_url_text(None, text, cx);
                cx.stop_propagation();
            }
        }
    }

    pub(super) fn shell_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.platform
            && event.keystroke.key == "w"
            && !self.layout.workbench_collapsed
            && self.workbench_focus_handle.contains_focused(window, cx)
        {
            self.dispatch_shell_command(ShellCommand::CloseWorkTab, cx);
            window.focus(&self.workbench_focus_handle, cx);
            cx.stop_propagation();
            return;
        }
        if escape_dismisses_shell_popover(
            event.keystroke.key.as_str(),
            self.model_picker_open,
            self.persona_picker_open,
            self.permission_picker_open,
            false,
            self.terminal_font_picker_open,
            self.connection_picker_open,
        ) {
            self.close_shell_popovers();
            window.focus(&self.composer_focus_handle, cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let Some(command) = shell_command_for_key(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.platform,
        ) else {
            return;
        };
        self.dispatch_shell_command(command, cx);
        cx.stop_propagation();
    }
}

fn transcript_index_for_position(
    layout: &gpui::TextLayout,
    position: Point<Pixels>,
    text: &str,
) -> usize {
    let offset = match layout.index_for_position(position) {
        Ok(offset) | Err(offset) => offset.min(text.len()),
    };
    snap_to_char_boundary(text, offset)
}
