use super::*;

impl DesktopShell {
    pub(super) fn render_inline(
        &self,
        inline: &InlineText,
        message_index: usize,
        block_index: usize,
        animate: bool,
        fill_width: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let selection =
            self.transcript_selection_range(message_index, block_index, inline.text.len());
        let highlights = self.inline_highlights(inline, selection);
        let code_font_overrides = inline
            .highlights
            .iter()
            .filter(|highlight| highlight.style == InlineStyle::Code)
            .map(|highlight| (highlight.range.clone(), DEFAULT_FONT_FAMILY.into()));
        let code_font_overrides = code_font_overrides.collect::<Vec<_>>();
        let text = gpui::StyledText::new(inline.text.clone())
            .with_font_family_overrides(code_font_overrides)
            .with_highlights(highlights);
        // Keep the interaction layout tied to the actual element being
        // painted. A separately-created TextLayout is never measured by
        // GPUI, so querying it from a mouse or accessibility event panics.
        let down_layout = text.layout().clone();
        let down_text = inline.text.clone();
        self.transcript_text_registry
            .borrow_mut()
            .push(TranscriptTextEntry {
                message_index,
                block_index,
                text: inline.text.clone(),
                layout: down_layout.clone(),
            });
        let element = div()
            .id(format!(
                "markdown-inline-container-{message_index}-{block_index}"
            ))
            .when(fill_width, |element| element.w_full())
            .min_w_0()
            .whitespace_normal()
            .cursor(gpui::CursorStyle::IBeam)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |shell, event, window, cx| {
                    shell.transcript_mouse_down(
                        message_index,
                        block_index,
                        &down_text,
                        &down_layout,
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
            .child(text);
        if !animate {
            return element.into_any_element();
        }
        element
            .with_animation(
                ElementId::Name(format!("markdown-inline-{message_index}-{block_index}").into()),
                Animation::new(Duration::from_millis(180)).with_easing(gpui::ease_out_quint()),
                |element, delta| element.opacity(0.72 + delta * 0.28),
            )
            .into_any_element()
    }

    fn inline_highlights(
        &self,
        inline: &InlineText,
        selection: Option<Range<usize>>,
    ) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        let mut boundaries = vec![0, inline.text.len()];
        for highlight in &inline.highlights {
            boundaries.push(highlight.range.start);
            boundaries.push(highlight.range.end);
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
                let style = inline
                    .highlights
                    .iter()
                    .find(|highlight| highlight.range.contains(&range.start))
                    .map(|highlight| self.inline_highlight_style(highlight.style))
                    .unwrap_or_default();
                let selected = selection.as_ref().is_some_and(|selection| {
                    selection.start < range.end && range.start < selection.end
                });
                if !selected && style == gpui::HighlightStyle::default() {
                    return None;
                }
                let mut style = style;
                if selected {
                    style.background_color = Some(
                        theme_rgb(&self.theme, "selection.background")
                            .opacity(0.28)
                            .into(),
                    );
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
        text: &str,
        layout: &gpui::TextLayout,
        event: &gpui::MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = transcript_index_for_position(layout, event.position, text);
        let point = TranscriptSelectionPoint {
            message_index,
            block_index,
            offset,
        };
        self.transcript_selection_anchor = Some(point);
        self.update_transcript_selection(point);
        self.transcript_is_selecting = true;
        cx.notify();
    }

    pub(super) fn transcript_mouse_move_at_position(
        &mut self,
        event: &gpui::MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.transcript_is_selecting || !event.dragging() {
            return;
        }
        let Some(anchor) = self.transcript_selection_anchor else {
            return;
        };
        let registry = self.transcript_text_registry.borrow();
        let Some(entry) = registry.iter().find(|entry| {
            let Some(start) = entry.layout.position_for_index(0) else {
                return false;
            };
            let Some(end) = entry.layout.position_for_index(entry.text.len()) else {
                return false;
            };
            let top = start.y.min(end.y);
            let bottom = start.y.max(end.y) + px(24.);
            event.position.y >= top && event.position.y <= bottom
        }) else {
            return;
        };
        let offset = transcript_index_for_position(&entry.layout, event.position, &entry.text);
        let point = TranscriptSelectionPoint {
            message_index: entry.message_index,
            block_index: entry.block_index,
            offset,
        };
        drop(registry);
        if point != anchor {
            self.update_transcript_selection(point);
        }
        cx.notify();
    }

    pub(super) fn transcript_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.transcript_is_selecting = false;
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
        let mut entries = registry.iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| (entry.message_index, entry.block_index));
        let mut selected = String::new();
        for entry in entries {
            let Some(range) = Self::selection_range_for(
                selection,
                entry.message_index,
                entry.block_index,
                entry.text.len(),
            ) else {
                continue;
            };
            if !selected.is_empty() {
                selected.push('\n');
            }
            selected.push_str(&entry.text[range]);
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
        animate: bool,
        fill_width: bool,
        syntax_highlights: &[MarkdownSyntaxHighlight],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let block_id = format!("markdown-block-{message_index}-{block_index}");
        match block {
            MarkdownBlock::Paragraph(inline) => div()
                .id(block_id)
                .when(!continuation, |element| element.mb(px(10.)))
                .child(self.render_inline(
                    inline,
                    message_index,
                    block_index,
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
                    .when(!continuation, |element| element.mb(px(12.)))
                    .text_size(size)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(self.render_inline(
                        content,
                        message_index,
                        block_index,
                        animate,
                        fill_width,
                        cx,
                    ))
                    .into_any_element()
            }
            MarkdownBlock::List(items) => div()
                .id(block_id)
                .flex()
                .flex_col()
                .gap(px(5.))
                .when(!continuation, |element| element.mb(px(10.)))
                .children(items.iter().enumerate().map(|(item_index, item)| {
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                .child("•"),
                        )
                        .child(div().flex_1().min_w_0().child(self.render_inline(
                            item,
                            message_index,
                            block_index * 1000 + item_index,
                            animate,
                            true,
                            cx,
                        )))
                }))
                .into_any_element(),
            MarkdownBlock::Quote(lines) => div()
                .id(block_id)
                .flex()
                .flex_col()
                .gap(px(4.))
                .when(!continuation, |element| element.mb(px(10.)))
                .border_l_2()
                .border_color(theme_rgb(&self.theme, "text.muted"))
                .pl(px(12.))
                .text_color(theme_rgb(&self.theme, "text.muted"))
                .children(lines.iter().enumerate().map(|(line_index, line)| {
                    self.render_inline(
                        line,
                        message_index,
                        block_index * 1000 + line_index,
                        animate,
                        true,
                        cx,
                    )
                }))
                .into_any_element(),
            MarkdownBlock::Code { language, text } => {
                let mut code = div()
                    .id(block_id)
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .when(!continuation, |element| element.mb(px(12.)))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme_rgb(&self.theme, "divider"))
                    .bg(theme_rgb(&self.theme, "muted"))
                    .p(px(12.));
                if let Some(language) = language {
                    code = code.when(!continuation, |element| {
                        element.child(
                            div()
                                .text_xs()
                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                .child(language.clone()),
                        )
                    });
                }
                let code_text = div()
                    .id(format!("markdown-code-text-{message_index}-{block_index}"))
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .whitespace_nowrap()
                    .text_xs()
                    .child(
                        gpui::StyledText::new(text.clone())
                            .with_font_family_overrides(std::iter::once((
                                0..text.len(),
                                gpui::SharedString::from(DEFAULT_FONT_FAMILY),
                            )))
                            .with_highlights(syntax_highlights.iter().map(|highlight| {
                                (
                                    highlight.range.clone(),
                                    self.syntax_highlight_style(highlight),
                                )
                            })),
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
            MarkdownBlock::Table(rows) => div()
                .id(block_id)
                .flex()
                .flex_col()
                .gap(px(4.))
                .when(!continuation, |element| element.mb(px(10.)))
                .rounded(px(8.))
                .border_1()
                .border_color(theme_rgb(&self.theme, "divider"))
                .p(px(10.))
                .children(rows.iter().enumerate().map(|(row_index, row)| {
                    div()
                        .w_full()
                        .min_w_0()
                        .when(row_index == 0, |element| {
                            element.font_weight(gpui::FontWeight::SEMIBOLD)
                        })
                        .child(self.render_inline(
                            row,
                            message_index,
                            block_index * 1000 + row_index,
                            animate,
                            true,
                            cx,
                        ))
                }))
                .into_any_element(),
            MarkdownBlock::Rule => div()
                .id(block_id)
                .h(px(1.))
                .when(!continuation, |element| element.mb(px(12.)))
                .bg(theme_rgb(&self.theme, "divider"))
                .into_any_element(),
            MarkdownBlock::Raw(inline) => div()
                .id(block_id)
                .when(!continuation, |element| element.mb(px(10.)))
                .child(self.render_inline(
                    inline,
                    message_index,
                    block_index,
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

    pub(super) fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        if let Some(request_id) = self.plan_feedback_request.take() {
            let feedback = self.model.ui.take_prompt();
            self.composer_selection = 0..0;
            self.composer_selection_reversed = false;
            self.composer_marked_range = None;
            self.composer_is_selecting = false;
            self.restart_composer_cursor_blink(cx);
            if let Some(feedback) = feedback {
                self.prompt_history.record(&feedback);
                self.send_command(ClientMessage::PlanApprovalResponse {
                    request_id,
                    approved: false,
                    feedback: Some(feedback),
                });
            }
            cx.notify();
            return;
        }
        let awaiting_question = self
            .model
            .ui
            .pending_actions
            .iter()
            .any(|action| matches!(&action.kind, ActionKind::AskUser { .. }));
        if self.model.ui.running && !awaiting_question {
            return;
        }
        let attachments = std::mem::take(&mut self.attachments);
        let Some(text) = self.model.ui.take_prompt() else {
            self.attachments = attachments;
            return;
        };
        self.composer_selection = 0..0;
        self.composer_selection_reversed = false;
        self.composer_marked_range = None;
        self.composer_is_selecting = false;
        self.restart_composer_cursor_blink(cx);
        self.input_animation_id = self.input_animation_id.wrapping_add(1);
        if let Some(action) = self
            .model
            .ui
            .pending_actions
            .iter()
            .find(|action| matches!(&action.kind, ActionKind::AskUser { .. }))
        {
            let ActionKind::AskUser { questions, .. } = &action.kind else {
                return;
            };
            let request_id = action.request_id.clone();
            let question_count = questions.len();
            let mut answers = text
                .lines()
                .map(str::trim)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            answers.resize(question_count, String::new());
            self.send_command(ClientMessage::AskUserResponse {
                request_id,
                answers,
            });
            cx.notify();
            return;
        }
        self.prompt_history.record(&text);
        if self.model.session_is_ready() {
            if let Some(command) = daemon_slash_command(&text) {
                self.send_command(ClientMessage::SlashCommand { command });
            } else {
                self.send_command(ClientMessage::Prompt { text, attachments });
            }
        } else {
            self.pending_prompt = Some(text);
            self.pending_attachments = attachments;
            if self.model.ui.selected_session.is_none() && !self.pending_session_request {
                self.pending_session_request = true;
                self.pending_session_target = None;
                let command = self.model.ui.new_conversation(None);
                self.send_command(command);
            }
        }
        cx.notify();
    }

    pub(super) fn begin_plan_feedback(
        &mut self,
        request_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.plan_feedback_request = Some(request_id);
        self.release_browser_focus();
        window.focus(&self.composer_focus_handle, cx);
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
        let Some(text) = latest_user_prompt(&self.model.ui.transcript) else {
            return;
        };
        self.send_command(ClientMessage::Prompt {
            text,
            attachments: Vec::new(),
        });
        cx.notify();
    }

    pub(super) fn composer_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        if key == "escape" && self.plan_feedback_request.is_some() {
            self.plan_feedback_request = None;
            cx.notify();
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
        if key == "up" && self.recall_prompt_history(true, cx) {
            cx.stop_propagation();
            return;
        }
        if key == "down" && self.recall_prompt_history(false, cx) {
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
        } else if key == "a" && event.keystroke.modifiers.platform {
            self.browser_url_select_all(cx);
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
        if escape_dismisses_shell_popover(
            event.keystroke.key.as_str(),
            self.model_picker_open,
            self.persona_picker_open,
            self.permission_picker_open,
            self.thinking_picker_open,
            self.terminal_font_picker_open,
            self.connection_picker_open,
        ) {
            self.close_shell_popovers();
            window.focus(&self.composer_focus_handle, cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if is_copy_keystroke(event)
            && !self.composer_focus_handle.is_focused(window)
            && self.copy_transcript_selection(cx)
        {
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
