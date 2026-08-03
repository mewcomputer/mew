use super::*;

pub(super) fn tool_display_label(tool_name: &str) -> String {
    let name = tool_name.rsplit(['.', ':']).next().unwrap_or(tool_name);
    match name {
        "bash" | "sh" | "shell" | "exec" | "run_command" => "Terminal".into(),
        name if name.starts_with("browser") => "Browser".into(),
        name if name.contains("plan") => "Plan".into(),
        name if name.contains("handoff") => "Handoff".into(),
        _ => name
            .split(['_', '-'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub(super) fn tool_summary(input: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(input).ok()?;
    let object = value.as_object()?;
    ["command", "path", "url", "query", "selector", "name"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(tool_value_text))
}

fn tool_value_text(value: &serde_json::Value) -> Option<String> {
    let text = value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| (!value.is_null()).then(|| value.to_string()))?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    const MAX_SUMMARY_CHARS: usize = 120;
    Some(if text.chars().count() > MAX_SUMMARY_CHARS {
        format!(
            "{}…",
            text.chars().take(MAX_SUMMARY_CHARS - 1).collect::<String>()
        )
    } else {
        text.to_owned()
    })
}

pub(super) fn formatted_tool_input(input: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| input.to_owned())
}

impl DesktopShell {
    pub(super) fn render_action_button(
        &self,
        id: String,
        label: impl Into<SharedString>,
        command: ClientMessage,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let label = label.into();
        div()
            .id(id)
            .px(px(10.))
            .py(px(6.))
            .rounded(px(7.))
            .cursor_pointer()
            .bg(theme_rgb(&self.theme, "card"))
            .text_xs()
            .role(Role::Button)
            .aria_label(label.clone())
            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.send_command(command.clone());
                cx.notify();
            }))
            .child(label)
            .into_any_element()
    }

    pub(super) fn render_pending_action(
        &self,
        action: PendingAction,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let request_id = action.request_id;
        match action.kind {
            ActionKind::Permission { tool_name, input }
            | ActionKind::SubagentPermission {
                tool_name, input, ..
            } => {
                let input = serde_json::to_string(&input).unwrap_or_else(|_| "unavailable".into());
                div()
                    .id(format!("required-action-{request_id}"))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(theme_rgb(&self.theme, "yellow.fg"))
                    .bg(theme_rgb(&self.theme, "panel.background"))
                    .shadow_md()
                    .child(div().text_sm().child("permission needed"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(format!("{tool_name}: {input}"))),
                    )
                    .child(div().flex().gap(px(6.)).children([
                        self.render_action_button(
                            format!("action-{request_id}-allow-once"),
                            "allow once",
                            ClientMessage::PermissionResponse {
                                request_id: request_id.clone(),
                                decision: PermissionDecision::AllowOnce,
                            },
                            cx,
                        ),
                        self.render_action_button(
                            format!("action-{request_id}-allow-session"),
                            "allow session",
                            ClientMessage::PermissionResponse {
                                request_id: request_id.clone(),
                                decision: PermissionDecision::AllowSession,
                            },
                            cx,
                        ),
                        self.render_action_button(
                            format!("action-{request_id}-deny"),
                            "deny",
                            ClientMessage::PermissionResponse {
                                request_id,
                                decision: PermissionDecision::Deny,
                            },
                            cx,
                        ),
                    ]))
                    .into_any_element()
            }
            ActionKind::WorkspacePermission { path } => div()
                .id(format!("required-action-{request_id}"))
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .rounded(px(12.))
                .border_1()
                .border_color(theme_rgb(&self.theme, "yellow.fg"))
                .bg(theme_rgb(&self.theme, "panel.background"))
                .shadow_md()
                .child(div().text_sm().child("workspace access needed"))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(SharedString::from(path)),
                )
                .child(div().flex().gap(px(6.)).children([
                    self.render_action_button(
                        format!("action-{request_id}-allow-once"),
                        "allow once",
                        ClientMessage::PermissionResponse {
                            request_id: request_id.clone(),
                            decision: PermissionDecision::AllowOnce,
                        },
                        cx,
                    ),
                    self.render_action_button(
                        format!("action-{request_id}-deny"),
                        "deny",
                        ClientMessage::PermissionResponse {
                            request_id,
                            decision: PermissionDecision::Deny,
                        },
                        cx,
                    ),
                ]))
                .into_any_element(),
            ActionKind::AskUser { questions, .. } => {
                let question_views = questions
                    .iter()
                    .enumerate()
                    .map(|(index, question)| {
                        div()
                            .id(format!("action-{request_id}-question-{index}"))
                            .text_sm()
                            .child(SharedString::from(format!(
                                "{}. {}",
                                index + 1,
                                question.prompt
                            )))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>();
                let option_views = (questions.len() == 1).then(|| {
                    div().flex().flex_wrap().gap(px(6.)).children(
                        questions[0]
                            .options
                            .iter()
                            .enumerate()
                            .map(|(index, option)| {
                                let answer = option.label.clone();
                                self.render_action_button(
                                    format!("action-{request_id}-option-{index}"),
                                    answer.clone(),
                                    ClientMessage::AskUserResponse {
                                        request_id: request_id.clone(),
                                        answers: vec![answer],
                                    },
                                    cx,
                                )
                            }),
                    )
                });
                div()
                    .id(format!("required-action-{request_id}"))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(theme_rgb(&self.theme, "accent"))
                    .bg(theme_rgb(&self.theme, "panel.background"))
                    .shadow_md()
                    .child(div().text_sm().child("input needed"))
                    .children(question_views)
                    .when_some(option_views, |element, options| element.child(options))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child("answer each question on a new line, then press Enter"),
                    )
                    .into_any_element()
            }
            ActionKind::PlanApproval {
                plan_path,
                plan_markdown,
                persona,
                ..
            } => {
                let feedback_active =
                    self.plan_feedback_request.as_deref() == Some(request_id.as_str());
                let preview = plan_markdown
                    .lines()
                    .take(8)
                    .enumerate()
                    .map(|(index, line)| {
                        div()
                            .id(format!("action-{request_id}-plan-line-{index}"))
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(line.to_owned()))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>();
                div()
                    .id(format!("required-action-{request_id}"))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(theme_rgb(&self.theme, "accent"))
                    .bg(theme_rgb(&self.theme, "panel.background"))
                    .shadow_md()
                    .child(div().text_sm().child("plan approval"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(format!("{persona} · {plan_path}"))),
                    )
                    .children(preview)
                    .child(div().flex().items_center().gap(px(6.)).children([
                        self.render_action_button(
                            format!("action-{request_id}-approve"),
                            "approve",
                            ClientMessage::PlanApprovalResponse {
                                request_id: request_id.clone(),
                                approved: true,
                                feedback: None,
                            },
                            cx,
                        ),
                        if feedback_active {
                            div()
                                .id(format!("action-{request_id}-cancel-feedback"))
                                .px(px(10.))
                                .py(px(6.))
                                .rounded(px(7.))
                                .cursor_pointer()
                                .bg(theme_rgb(&self.theme, "card"))
                                .text_xs()
                                .role(Role::Button)
                                .aria_label("Cancel plan feedback")
                                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                .on_click(cx.listener(|shell, _, _, cx| {
                                    shell.plan_feedback_request = None;
                                    cx.notify();
                                }))
                                .child("cancel")
                                .into_any_element()
                        } else {
                            let feedback_request_id = request_id.clone();
                            div()
                                .id(format!("action-{request_id}-reject"))
                                .px(px(10.))
                                .py(px(6.))
                                .rounded(px(7.))
                                .cursor_pointer()
                                .bg(theme_rgb(&self.theme, "card"))
                                .text_xs()
                                .role(Role::Button)
                                .aria_label("Request plan changes")
                                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                .on_click(cx.listener(move |shell, _, window, cx| {
                                    shell.begin_plan_feedback(
                                        feedback_request_id.clone(),
                                        window,
                                        cx,
                                    );
                                }))
                                .child("request changes")
                                .into_any_element()
                        },
                    ]))
                    .when(feedback_active, |element| {
                        element.child(
                            div()
                                .text_xs()
                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                .child("type feedback in the composer, then press Enter to send · Esc to cancel"),
                        )
                    })
                    .into_any_element()
            }
            ActionKind::GoalApproval { objective, .. } => div()
                .id(format!("required-action-{request_id}"))
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .rounded(px(12.))
                .border_1()
                .border_color(theme_rgb(&self.theme, "accent"))
                .bg(theme_rgb(&self.theme, "panel.background"))
                .shadow_md()
                .child(div().text_sm().child("goal proposed"))
                .child(div().text_xs().child(SharedString::from(objective)))
                .child(div().flex().gap(px(6.)).children([
                    self.render_action_button(
                        format!("action-{request_id}-accept"),
                        "accept",
                        ClientMessage::GoalResponse {
                            request_id: request_id.clone(),
                            accepted: true,
                        },
                        cx,
                    ),
                    self.render_action_button(
                        format!("action-{request_id}-decline"),
                        "decline",
                        ClientMessage::GoalResponse {
                            request_id,
                            accepted: false,
                        },
                        cx,
                    ),
                ]))
                .into_any_element(),
        }
    }

    pub(super) fn render_pending_actions(
        &self,
        width: Option<Pixels>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let cards = self
            .model
            .ui
            .pending_actions
            .clone()
            .into_iter()
            .map(|action| self.render_pending_action(action, cx))
            .collect::<Vec<_>>();
        let animation_key = self
            .model
            .ui
            .pending_actions
            .iter()
            .map(|action| action.request_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        div()
            .id("required-actions")
            .flex()
            .flex_col()
            .gap(px(8.))
            .max_h(px(280.))
            .overflow_y_scroll()
            .when_some(width, |element, width| element.w(width))
            .children(cards)
            .with_animation(
                ElementId::Name(format!("required-actions-{animation_key}").into()),
                Animation::new(Duration::from_millis(180)).with_easing(gpui::ease_out_quint()),
                |element, delta| element.opacity(delta).relative().top(px((1. - delta) * 8.)),
            )
            .into_any_element()
    }

    pub(super) fn render_transcript_row(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(row) = self.transcript_rows.get(index).copied() else {
            return div().into_any_element();
        };
        let Some(item) = self.model.ui.transcript.get(row.message_index) else {
            return div().into_any_element();
        };
        let is_user = matches!(item.role, TranscriptRole::User);
        let animate = should_animate_transcript_row(index, self.transcript_rows.len());
        let fallback_part;
        let part = if let Some(part) = item.parts.get(row.part_index) {
            part
        } else {
            fallback_part = TranscriptPart::Text(item.text.clone());
            &fallback_part
        };
        let cached = self
            .markdown_cache
            .get(row.message_index)
            .and_then(|parts| parts.get(row.part_index));
        let selection_block_index = row
            .part_index
            .saturating_mul(1_000_000)
            .saturating_add(row.block_index);
        let content = match part {
            TranscriptPart::Text(_) => {
                let Some(render_block) =
                    cached.and_then(|cached| cached.render_blocks.get(row.block_index))
                else {
                    return div().into_any_element();
                };
                self.render_markdown_block(
                    &render_block.block,
                    row.message_index,
                    selection_block_index,
                    render_block.continuation,
                    animate,
                    !is_user,
                    &render_block.syntax_highlights,
                    cx,
                )
            }
            TranscriptPart::Reasoning(_) => self.render_reasoning_part(
                row.message_index,
                row.part_index,
                cached,
                selection_block_index,
                row.block_index,
                cx,
            ),
            TranscriptPart::ToolCall {
                tool_name,
                call_id,
                status,
                input,
                output,
                error,
                diff,
            } => self.render_tool_part(
                row.message_index,
                row.part_index,
                tool_name,
                call_id,
                *status,
                input,
                output.as_deref(),
                error.as_deref(),
                diff.as_deref(),
                cx,
            ),
            TranscriptPart::File(label) => self.render_meta_part(
                row.message_index,
                row.part_index,
                TablerIcon::FileCode,
                "attachment",
                label,
            ),
            TranscriptPart::Compaction { auto } => self.render_meta_part(
                row.message_index,
                row.part_index,
                TablerIcon::Dots,
                "context compacted",
                if *auto { "automatic" } else { "manual" },
            ),
        };

        let continues_before = is_user
            && index > 0
            && self
                .transcript_rows
                .get(index - 1)
                .is_some_and(|previous| previous.message_index == row.message_index);
        let continues_after = is_user
            && self
                .transcript_rows
                .get(index + 1)
                .is_some_and(|next| next.message_index == row.message_index);

        div()
            .id(format!("transcript-row-{index}"))
            .flex()
            .w_full()
            .when(is_user && !continues_after, |element| element.mb(px(8.)))
            .justify_end()
            .when(!is_user, |element| element.justify_start())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .min_w_0()
                    .max_w(px(650.))
                    .when(!is_user, |element| element.w_full())
                    .when(is_user, |element| {
                        element
                            .flex_none()
                            .p(px(12.))
                            .when(continues_before, |element| element.pt(px(2.)))
                            .when(continues_after, |element| element.pb(px(2.)))
                            .rounded(px(14.))
                            .bg(theme_rgb(&self.theme, "card"))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .when(!is_user, |element| element.w_full())
                            .text_sm()
                            .line_height(px(20.))
                            .text_color(theme_rgb(&self.theme, "text.body"))
                            .child(content),
                    ),
            )
            .into_any_element()
    }

    fn toggle_chat_part(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.expanded_chat_parts.remove(&key) {
            self.expanded_chat_parts.insert(key);
        }
        self.rebuild_transcript_rows_from_cache();
        self.sync_transcript_list();
        cx.notify();
    }

    pub(super) fn chat_part_key(message_index: usize, part_index: usize) -> String {
        format!("chat-part-{message_index}-{part_index}")
    }

    fn render_part_header(
        &self,
        key: String,
        icon: TablerIcon,
        label: String,
        status: Option<(String, gpui::Rgba)>,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let click_key = key.clone();
        div()
            .id(format!("{key}-header"))
            .flex()
            .items_center()
            .gap(px(7.))
            .cursor_pointer()
            .role(Role::Button)
            .aria_expanded(!collapsed)
            .aria_label(SharedString::from(format!(
                "{}{}",
                label,
                status
                    .as_ref()
                    .map(|(status, _)| format!(" · {status}"))
                    .unwrap_or_default()
            )))
            .text_xs()
            .text_color(theme_rgb(&self.theme, "text.muted"))
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.toggle_chat_part(click_key.clone(), cx);
            }))
            .child(tabler_icon(
                icon,
                theme_rgb(&self.theme, "text.muted"),
                px(13.),
            ))
            .child(SharedString::from(label))
            .when_some(status, |element, (label, color)| {
                element.child(div().text_color(color).child(SharedString::from(label)))
            })
            .child(tabler_icon(
                if collapsed {
                    TablerIcon::ChevronRight
                } else {
                    TablerIcon::ChevronDown
                },
                theme_rgb(&self.theme, "text.muted"),
                px(12.),
            ))
            .into_any_element()
    }

    fn render_reasoning_part(
        &self,
        message_index: usize,
        part_index: usize,
        cached: Option<&CachedMarkdown>,
        selection_block_index: usize,
        block_index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = Self::chat_part_key(message_index, part_index);
        let collapsed = !self.expanded_chat_parts.contains(&key);
        let block = cached.and_then(|cached| cached.render_blocks.get(block_index));
        let body = (!collapsed)
            .then(|| {
                block.map(|block| {
                    div()
                        .id(format!(
                            "chat-part-{message_index}-{part_index}-body-{block_index}"
                        ))
                        .pl(px(20.))
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(self.render_markdown_block(
                            &block.block,
                            message_index,
                            selection_block_index,
                            block.continuation,
                            false,
                            true,
                            &block.syntax_highlights,
                            cx,
                        ))
                })
            })
            .flatten();
        div()
            .id(format!(
                "chat-part-{message_index}-{part_index}-row-{block_index}"
            ))
            .flex()
            .flex_col()
            .gap(px(6.))
            .opacity(0.82)
            .when(block_index == 0, |element| {
                element.child(self.render_part_header(
                    key,
                    TablerIcon::Dots,
                    "thinking".into(),
                    None,
                    collapsed,
                    cx,
                ))
            })
            .when_some(body, |element, body| element.child(body))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tool_part(
        &self,
        message_index: usize,
        part_index: usize,
        tool_name: &str,
        call_id: &str,
        status: ToolStatus,
        input: &str,
        output: Option<&str>,
        error: Option<&str>,
        diff: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = Self::chat_part_key(message_index, part_index);
        let collapsed = !self.expanded_chat_parts.contains(&key);
        let (status_label, status_color) = match status {
            ToolStatus::Pending => ("pending", theme_rgb(&self.theme, "text.muted")),
            ToolStatus::Running => ("running", theme_rgb(&self.theme, "yellow.fg")),
            ToolStatus::Completed => ("completed", theme_rgb(&self.theme, "green.fg")),
            ToolStatus::Error => ("failed", theme_rgb(&self.theme, "red.fg")),
        };
        let display_name = tool_display_label(tool_name);
        let summary = tool_summary(input);
        let header = self.render_part_header(
            key,
            TablerIcon::Terminal2,
            display_name,
            Some((status_label.into(), status_color)),
            collapsed,
            cx,
        );
        let summary_view = summary.map(|summary| {
            div()
                .pl(px(20.))
                .text_xs()
                .text_color(theme_rgb(&self.theme, "text.body"))
                .child(SharedString::from(summary))
        });
        let body = (!collapsed).then(|| {
            let mut body = div()
                .id(format!("tool-body-{message_index}-{part_index}"))
                .flex()
                .flex_col()
                .gap(px(6.))
                .pl(px(20.))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child("input"),
                )
                .child(self.render_tool_text(
                    format!("tool-input-{message_index}-{part_index}"),
                    &formatted_tool_input(input),
                ));
            if let Some(output) = output.filter(|output| !output.is_empty()) {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child("output"),
                );
                body =
                    body.child(self.render_tool_text(
                        format!("tool-output-{message_index}-{part_index}"),
                        output,
                    ));
            }
            if let Some(diff) = diff.filter(|diff| !diff.is_empty()) {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child("diff"),
                );
                body = body.child(
                    self.render_tool_diff(format!("tool-diff-{message_index}-{part_index}"), diff),
                );
            }
            if let Some(error) = error {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "red.fg"))
                        .child(SharedString::from(error.to_owned())),
                );
            }
            body
        });
        div()
            .id(format!("tool-part-{message_index}-{part_index}-{call_id}"))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(10.))
            .rounded(px(9.))
            .bg(theme_rgb(&self.theme, "card").opacity(0.46))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider").opacity(0.7))
            .child(header)
            .when_some(summary_view, |element, summary| element.child(summary))
            .when_some(body, |element, body| element.child(body))
            .into_any_element()
    }

    fn render_tool_text(&self, id: String, text: &str) -> gpui::AnyElement {
        let source_identity = text.as_ptr() as usize;
        let source_len = text.len();
        let lines = {
            let mut cache = self.tool_text_cache.borrow_mut();
            let cached = cache.entry(id.clone()).or_insert_with(|| ToolTextCache {
                source_identity,
                source_len,
                lines: Arc::new(split_tool_text_lines(text)),
            });
            if cached.source_identity != source_identity || cached.source_len != source_len {
                cached.source_identity = source_identity;
                cached.source_len = source_len;
                cached.lines = Arc::new(split_tool_text_lines(text));
            }
            cached.lines.clone()
        };
        let line_count = lines.len();
        let state = {
            let mut lists = self.tool_text_lists.borrow_mut();
            lists
                .entry(id.clone())
                .or_insert_with(|| gpui::ListState::new(0, gpui::ListAlignment::Top, px(96.)))
                .clone()
        };
        if state.item_count() != line_count {
            state.reset(line_count);
        }
        let text_color = theme_rgb(&self.theme, "text.body");
        div()
            .id(id.clone())
            .max_h(px(180.))
            .overflow_x_scroll()
            .overflow_y_scroll()
            .bg(theme_rgb(&self.theme, "background"))
            .rounded(px(6.))
            .p(px(8.))
            .child(gpui::list(state, move |index, _, _| {
                div()
                    .id(format!("{id}-line-{index}"))
                    .w_full()
                    .min_w_0()
                    .text_xs()
                    .font_family(DEFAULT_FONT_FAMILY)
                    .text_color(text_color)
                    .whitespace_nowrap()
                    .child(lines.get(index).cloned().unwrap_or_default())
                    .into_any_element()
            }))
            .into_any_element()
    }

    fn render_tool_diff(&self, id: String, diff: &str) -> gpui::AnyElement {
        const DIFF_LINES_MAX: usize = 40;
        let line_count = diff.lines().count();
        let truncated = line_count > DIFF_LINES_MAX;
        let lines = diff
            .lines()
            .take(DIFF_LINES_MAX)
            .map(|line| {
                let color = if line.starts_with('+') {
                    theme_rgb(&self.theme, "green.fg")
                } else if line.starts_with('-') {
                    theme_rgb(&self.theme, "red.fg")
                } else {
                    theme_rgb(&self.theme, "text.muted")
                };
                (SharedString::from(line.to_owned()), color)
            })
            .collect::<Vec<_>>();
        div()
            .id(id.clone())
            .max_h(px(180.))
            .overflow_x_scroll()
            .overflow_y_scroll()
            .bg(theme_rgb(&self.theme, "background"))
            .rounded(px(6.))
            .p(px(8.))
            .children(lines.into_iter().enumerate().map(|(index, (line, color))| {
                div()
                    .id(format!("{id}-line-{index}"))
                    .w_full()
                    .min_w_0()
                    .text_xs()
                    .font_family(DEFAULT_FONT_FAMILY)
                    .text_color(color)
                    .whitespace_nowrap()
                    .child(line)
                    .into_any_element()
            }))
            .when(truncated, |element| {
                element.child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(format!("… {} more lines", line_count - DIFF_LINES_MAX)),
                )
            })
            .into_any_element()
    }

    fn render_meta_part(
        &self,
        message_index: usize,
        part_index: usize,
        icon: TablerIcon,
        label: &str,
        detail: &str,
    ) -> gpui::AnyElement {
        div()
            .id(format!("meta-part-{message_index}-{part_index}"))
            .flex()
            .items_center()
            .gap(px(7.))
            .text_xs()
            .text_color(theme_rgb(&self.theme, "text.muted"))
            .child(tabler_icon(
                icon,
                theme_rgb(&self.theme, "text.muted"),
                px(13.),
            ))
            .child(label.to_owned())
            .child(SharedString::from(detail.to_owned()))
            .into_any_element()
    }

    pub(super) fn sync_transcript_list(&mut self) {
        let count = self.transcript_rows.len();
        let old_count = self.transcript_list.item_count();
        if old_count != count {
            let was_at_end =
                old_count == 0 || self.transcript_list.is_scrolled_to_end().unwrap_or(false);
            if self.transcript_rows_append_only && count >= old_count {
                self.transcript_list
                    .splice(old_count..old_count, count - old_count);
            } else {
                self.transcript_list.reset(count);
            }
            if count > 0 && was_at_end {
                self.transcript_list.scroll_to_end();
            }
        }
    }

    fn render_transcript_attention(
        &self,
        attention: TranscriptAttention,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (title, detail, color, icon) = match attention {
            TranscriptAttention::Running => (
                "Working",
                "The assistant is processing this turn.",
                theme_rgb(&self.theme, "yellow.fg"),
                TablerIcon::Square,
            ),
            TranscriptAttention::Failed => (
                "Last turn failed",
                "Retry to send the last prompt again.",
                theme_rgb(&self.theme, "red.fg"),
                TablerIcon::X,
            ),
            TranscriptAttention::Waiting => (
                "Waiting for a response",
                "The turn ended without an assistant message.",
                theme_rgb(&self.theme, "text.muted"),
                TablerIcon::MessageCircle,
            ),
        };
        let body = theme_rgb(&self.theme, "text.body");
        let muted = theme_rgb(&self.theme, "text.muted");
        let mut copy = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .flex_1()
            .child(div().text_xs().text_color(body).child(title))
            .child(div().text_xs().text_color(muted).child(detail));
        if let Some(error) = error.filter(|error| !error.trim().is_empty()) {
            copy = copy.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(SharedString::from(error.to_owned())),
            );
        }

        let mut card = div()
            .id("conversation-attention")
            .flex()
            .items_start()
            .gap(px(8.))
            .w_full()
            .px(px(12.))
            .py(px(9.))
            .rounded(px(10.))
            .border_1()
            .border_color(color.opacity(0.32))
            .bg(color.opacity(0.08))
            .child(tabler_icon(icon, color, px(14.)))
            .child(copy);

        if matches!(attention, TranscriptAttention::Running) {
            card = card.child(
                div()
                    .id("cancel-attention-turn")
                    .px(px(7.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .bg(theme_rgb(&self.theme, "card"))
                    .text_xs()
                    .text_color(body)
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label("Cancel current turn")
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.cancel_turn(cx);
                    }))
                    .child("cancel"),
            );
        } else if matches!(
            attention,
            TranscriptAttention::Failed | TranscriptAttention::Waiting
        ) {
            card = card.child(
                div()
                    .id("retry-attention-turn")
                    .px(px(7.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .bg(theme_rgb(&self.theme, "card"))
                    .text_xs()
                    .text_color(body)
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label(if matches!(attention, TranscriptAttention::Failed) {
                        "Retry last turn"
                    } else {
                        "Retry waiting turn"
                    })
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.retry_last_turn(cx);
                    }))
                    .child("retry"),
            );
        }

        card.into_any_element()
    }

    pub(super) fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.transcript_text_registry.borrow_mut().clear();
        let selected_session = self.model.ui.selected_session.is_some();
        let last_turn_failed = self
            .model
            .ui
            .selected_session
            .as_deref()
            .and_then(|selected| {
                self.model
                    .ui
                    .conversations
                    .iter()
                    .find(|conversation| conversation.session_id == selected)
            })
            .is_some_and(|conversation| conversation.last_turn_failed);
        let last_message_is_user = self
            .model
            .ui
            .transcript
            .last()
            .is_some_and(|item| item.role == TranscriptRole::User);
        let transcript_attention_state = transcript_attention(
            last_message_is_user,
            self.model.ui.running,
            last_turn_failed,
            !self.model.ui.pending_actions.is_empty(),
        );
        let last_error = self.model.last_error.clone();
        let transcript = if self.transcript_rows.is_empty() {
            div()
                .id("conversation-transcript-empty")
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_xl().child(if selected_session {
                    "ready when you are"
                } else {
                    "start a coding session"
                }))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(if selected_session {
                            "Send a message to begin this conversation."
                        } else {
                            "Choose a conversation or create a new one from the rail."
                        }),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(
                    gpui::list(
                        self.transcript_list.clone(),
                        cx.processor(Self::render_transcript_row),
                    )
                    .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
                    .flex_1()
                    .min_h_0(),
                )
                .when_some(transcript_attention_state, |element, attention| {
                    element.child(self.render_transcript_attention(
                        attention,
                        if matches!(attention, TranscriptAttention::Failed) {
                            last_error.as_deref()
                        } else {
                            None
                        },
                        cx,
                    ))
                })
                .into_any_element()
        };
        let model_display_label = non_empty_label(self.model.ui.model.as_deref(), "choose a model");
        let thinking_label = self.model.ui.thinking_variant.as_deref().unwrap_or("");
        let model_display = if thinking_label.is_empty() {
            model_display_label
        } else {
            format!("{model_display_label} · {thinking_label}")
        };
        let persona_display =
            non_empty_label(self.model.ui.current_persona.as_deref(), "choose a persona");
        let permission_display = permission_mode_label(self.model.ui.permission_mode.as_deref());
        let thinking_variants = thinking_variants_for_model(
            &self.model.ui.models,
            self.model.ui.current_provider.as_deref(),
            self.model.ui.model.as_deref(),
        );
        let thinking_display =
            non_empty_label(self.model.ui.thinking_variant.as_deref(), "thinking off");
        let session_path = selected_session_path(
            &self.model.ui.conversations,
            self.model.ui.selected_session.as_deref(),
        );
        let usage_label = self.model.ui.usage.and_then(usage_summary_label);
        let presence_count = self.model.ui.presence.len();
        let control_yielded = self.model.ui.control_yielded_by.is_some();

        let composer_is_empty = self.model.ui.composer.is_empty();
        let turn_is_running = self.model.ui.running;
        let composer_focused = self.composer_focus_handle.is_focused(window);
        let attachments = self.attachments.clone();
        let attachment_error = self.attachment_error.clone();
        let composer_focus_handle = self.composer_focus_handle.clone();
        let model_picker_trigger_shell = cx.entity().downgrade();
        let persona_picker_trigger_shell = cx.entity().downgrade();
        let permission_picker_trigger_shell = cx.entity().downgrade();
        let thinking_picker_trigger_shell = cx.entity().downgrade();

        let conversation = div()
            .id("conversation-surface")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .rounded(px(SHELL_SURFACE_RADIUS))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .overflow_hidden()
            .bg(theme_rgb(&self.theme, "background"))
            .child(
                div()
                    .id("conversation-transcript")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .w_full()
                    .max_w(px(CHAT_CONTENT_MAX_WIDTH))
                    .self_center()
                    .gap(px(20.))
                    .p(px(24.))
                    .overflow_y_hidden()
                    .on_mouse_move(cx.listener(Self::transcript_mouse_move_at_position))
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(Self::transcript_mouse_up),
                    )
                    .on_mouse_up_out(
                        gpui::MouseButton::Left,
                        cx.listener(Self::transcript_mouse_up),
                    )
                    .child(transcript),
            )
            .child(
                div()
                    .id("composer-area")
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(px(CHAT_CONTENT_MAX_WIDTH))
                    .self_center()
                    .gap(px(8.))
                    .px(px(24.))
                    .pb(px(12.))
                    .when_some(last_error, |element, error| {
                        element.child(
                            div()
                                .id("composer-error")
                                .flex()
                                .items_start()
                                .gap(px(8.))
                                .w_full()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(10.))
                                .border_1()
                                .border_color(theme_rgb(&self.theme, "red.fg").opacity(0.32))
                                .bg(theme_rgb(&self.theme, "red.fg").opacity(0.08))
                                .child(tabler_icon(
                                    TablerIcon::X,
                                    theme_rgb(&self.theme, "red.fg"),
                                    px(14.),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .min_w_0()
                                        .flex_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme_rgb(&self.theme, "text.body"))
                                                .child(
                                                    if self.model.connection.label()
                                                        == "connection failed"
                                                    {
                                                        "Connection issue"
                                                    } else {
                                                        "Daemon error"
                                                    },
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                                .child(SharedString::from(error)),
                                        ),
                                )
                                .child(if self.model.connection.label() == "connection failed" {
                                    div()
                                        .id("retry-connection")
                                        .px(px(7.))
                                        .py(px(4.))
                                        .rounded(px(5.))
                                        .bg(theme_rgb(&self.theme, "card"))
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.body"))
                                        .cursor_pointer()
                                        .role(Role::Button)
                                        .aria_label("Retry daemon connection")
                                        .on_click(cx.listener(|shell, _, _, cx| {
                                            shell.retry_connection(cx);
                                        }))
                                        .child("retry")
                                        .into_any_element()
                                } else {
                                    div()
                                        .id("dismiss-composer-error")
                                        .px(px(7.))
                                        .py(px(4.))
                                        .rounded(px(5.))
                                        .bg(theme_rgb(&self.theme, "card"))
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.body"))
                                        .cursor_pointer()
                                        .role(Role::Button)
                                        .aria_label("Dismiss error")
                                        .on_click(cx.listener(|shell, _, _, cx| {
                                            shell.dismiss_error(cx);
                                        }))
                                        .child("dismiss")
                                        .into_any_element()
                                }),
                        )
                    })
                    .when_some(attachment_error, |element, error| {
                        element.child(
                            div()
                                .id("attachment-error")
                                .text_xs()
                                .text_color(theme_rgb(&self.theme, "red.fg"))
                                .child(SharedString::from(error)),
                        )
                    })
                    .when(!attachments.is_empty(), |element| {
                        element.child(
                            div()
                                .id("attachment-chips")
                                .flex()
                                .flex_wrap()
                                .gap(px(6.))
                                .children(attachments.iter().enumerate().map(
                                    |(index, attachment)| {
                                        let label = Path::new(&attachment.path)
                                            .file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or(&attachment.path)
                                            .to_owned();
                                        div()
                                            .id(format!("attachment-{index}"))
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .px(px(8.))
                                            .py(px(4.))
                                            .rounded(px(7.))
                                            .bg(theme_rgb(&self.theme, "card"))
                                            .text_xs()
                                            .child(SharedString::from(label))
                                            .child(
                                                div()
                                                    .id(format!("remove-attachment-{index}"))
                                                    .cursor_pointer()
                                                    .text_color(theme_rgb(
                                                        &self.theme,
                                                        "text.muted",
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |shell, _, _, cx| {
                                                            shell.remove_attachment(index, cx);
                                                        },
                                                    ))
                                                    .child(tabler_icon(
                                                        TablerIcon::X,
                                                        theme_rgb(&self.theme, "text.muted"),
                                                        px(12.),
                                                    )),
                                            )
                                            .into_any_element()
                                    },
                                )),
                        )
                    })
                    .child(
                        div()
                            .id("composer")
                            .flex_col()
                            .items_stretch()
                            .gap(px(12.))
                            .min_h_0()
                            .border_1()
                            .border_color(theme_rgb(&self.theme, "divider"))
                            .rounded(px(16.))
                            .bg(theme_rgb(&self.theme, "panel.background"))
                            .p(px(12.))
                            .text_sm()
                            .role(Role::Group)
                            .aria_label("Message composer. Drop files here to attach.")
                            .when(composer_focused, |element| {
                                element.border_color(
                                    theme_rgb(&self.theme, "text.accent").opacity(0.6),
                                )
                            })
                            .on_drop(cx.listener(|shell, path: &PathBuf, _, cx| {
                                shell.add_attachment(path.clone(), cx);
                            }))
                            .text_color(if composer_is_empty {
                                theme_rgb(&self.theme, "text.muted")
                            } else {
                                theme_rgb(&self.theme, "text.body")
                            })
                            .child(
                                div()
                                    .id("composer-input-surface")
                                    .flex()
                                    .w_full()
                                    .flex_none()
                                    .min_h(px(48.))
                                    .max_h(px(96.))
                                    .items_start()
                                    .key_context("Composer")
                                    .track_focus(&composer_focus_handle)
                                    .role(Role::TextInput)
                                    .aria_label("Message composer")
                                    .aria_description("Type a message, then press Enter to send.")
                                    .cursor(gpui::CursorStyle::IBeam)
                                    .on_key_down(cx.listener(Self::composer_key_down))
                                    .on_action(cx.listener(Self::composer_backspace))
                                    .on_action(cx.listener(Self::composer_delete))
                                    .on_action(cx.listener(Self::composer_left))
                                    .on_action(cx.listener(Self::composer_right))
                                    .on_action(cx.listener(Self::composer_select_left))
                                    .on_action(cx.listener(Self::composer_select_right))
                                    .on_action(cx.listener(Self::composer_select_all))
                                    .on_action(cx.listener(Self::composer_paste))
                                    .on_action(cx.listener(Self::composer_home))
                                    .on_action(cx.listener(Self::composer_end))
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::composer_mouse_down),
                                    )
                                    .on_mouse_up(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::composer_mouse_up),
                                    )
                                    .on_mouse_up_out(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::composer_mouse_up),
                                    )
                                    .on_mouse_move(cx.listener(Self::composer_mouse_move))
                                    .child(ComposerElement {
                                        shell: cx.entity(),
                                        target: TextInputTarget::Composer,
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .w_full()
                                    .gap(px(12.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .child(
                                                div()
                                                    .id("attachment-picker-trigger")
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .size(px(26.))
                                                    .rounded(px(6.))
                                                    .cursor_pointer()
                                                    .role(Role::Button)
                                                    .aria_label("Attach files")
                                                    .text_color(theme_rgb(
                                                        &self.theme,
                                                        "secondary_foreground",
                                                    ))
                                                    .hover(|element| {
                                                        element.bg(theme_rgb(&self.theme, "muted"))
                                                    })
                                                    .on_click(cx.listener(
                                                        |shell, _, window, cx| {
                                                            shell.pick_attachments(window, cx);
                                                        },
                                                    ))
                                                    .child(tabler_icon(
                                                        TablerIcon::Paperclip,
                                                        theme_rgb(
                                                            &self.theme,
                                                            "secondary_foreground",
                                                        ),
                                                        px(14.),
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme_rgb(
                                                        &self.theme,
                                                        "secondary_foreground",
                                                    ))
                                                    .cursor_pointer()
                                                    .id("model-picker-trigger")
                                                    .relative()
                                                    .role(Role::Button)
                                                    .aria_label(SharedString::from(format!(
                                                        "Choose model: {model_display}"
                                                    )))
                                                    .px(px(6.))
                                                    .py(px(4.))
                                                    .rounded(px(6.))
                                                    .hover(|element| {
                                                        element.bg(theme_rgb(&self.theme, "muted"))
                                                    })
                                                    .on_click(cx.listener(|shell, _, _, cx| {
                                                        shell.toggle_model_picker(cx);
                                                    }))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(4.))
                                                    .child(tabler_icon(
                                                        TablerIcon::SlidersHorizontal,
                                                        theme_rgb(
                                                            &self.theme,
                                                            "secondary_foreground",
                                                        ),
                                                        px(13.),
                                                    ))
                                                    .child(SharedString::from(model_display))
                                                    .child(tabler_icon(
                                                        TablerIcon::ChevronDown,
                                                        theme_rgb(
                                                            &self.theme,
                                                            "secondary_foreground",
                                                        ),
                                                        px(12.),
                                                    ))
                                                    .child(
                                                        canvas(
                                                            move |bounds, _, cx| {
                                                                model_picker_trigger_shell
                                                                    .update(cx, |shell, cx| {
                                                                        if shell.model_picker_bounds
                                                                            != Some(bounds)
                                                                        {
                                                                            shell.model_picker_bounds =
                                                                                Some(bounds);
                                                                            cx.notify();
                                                                        }
                                                                    })
                                                                    .ok();
                                                            },
                                                            |_bounds, _state, _window, _cx| {},
                                                        )
                                                        .absolute()
                                                        .inset_0()
                                                        .size_full(),
                                                    )
                                            )
                                            .when(!thinking_variants.is_empty(), |element| {
                                                element.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme_rgb(
                                                            &self.theme,
                                                            "secondary_foreground",
                                                        ))
                                                        .cursor_pointer()
                                                        .id("thinking-picker-trigger")
                                                        .relative()
                                                        .role(Role::Button)
                                                        .aria_label(SharedString::from(format!(
                                                            "Choose thinking variant: {thinking_display}"
                                                        )))
                                                        .px(px(6.))
                                                        .py(px(4.))
                                                        .rounded(px(6.))
                                                        .hover(|element| {
                                                            element.bg(theme_rgb(&self.theme, "muted"))
                                                        })
                                                        .on_click(cx.listener(|shell, _, _, cx| {
                                                            shell.toggle_thinking_picker(cx);
                                                        }))
                                                        .flex()
                                                        .items_center()
                                                        .gap(px(4.))
                                                        .child(tabler_icon(
                                                            TablerIcon::Bulb,
                                                            theme_rgb(
                                                                &self.theme,
                                                                "secondary_foreground",
                                                            ),
                                                            px(13.),
                                                        ))
                                                        .child(SharedString::from(thinking_display.clone()))
                                                        .child(tabler_icon(
                                                            TablerIcon::ChevronDown,
                                                            theme_rgb(
                                                                &self.theme,
                                                                "secondary_foreground",
                                                            ),
                                                            px(12.),
                                                        ))
                                                        .child(
                                                            canvas(
                                                                move |bounds, _, cx| {
                                                                    thinking_picker_trigger_shell
                                                                        .update(cx, |shell, cx| {
                                                                            if shell.thinking_picker_bounds
                                                                                != Some(bounds)
                                                                            {
                                                                                shell.thinking_picker_bounds =
                                                                                    Some(bounds);
                                                                                cx.notify();
                                                                            }
                                                                        })
                                                                        .ok();
                                                                },
                                                                |_bounds, _state, _window, _cx| {},
                                                            )
                                                            .absolute()
                                                            .inset_0()
                                                            .size_full(),
                                                        ),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme_rgb(
                                                        &self.theme,
                                                        "secondary_foreground",
                                                    ))
                                                    .cursor_pointer()
                                                    .id("permission-picker-trigger")
                                                    .relative()
                                                    .role(Role::Button)
                                                    .aria_label(SharedString::from(format!(
                                                        "Choose permission mode: {permission_display}"
                                                    )))
                                                    .px(px(6.))
                                                    .py(px(4.))
                                                    .rounded(px(6.))
                                                    .hover(|element| {
                                                        element.bg(theme_rgb(&self.theme, "muted"))
                                                    })
                                                    .on_click(cx.listener(|shell, _, _, cx| {
                                                        shell.toggle_permission_picker(cx);
                                                    }))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(4.))
                                                    .child(tabler_icon(
                                                        TablerIcon::ShieldLock,
                                                        theme_rgb(
                                                            &self.theme,
                                                            "secondary_foreground",
                                                        ),
                                                        px(13.),
                                                    ))
                                                    .child(SharedString::from(permission_display.clone()))
                                                    .child(tabler_icon(
                                                        TablerIcon::ChevronDown,
                                                        theme_rgb(
                                                            &self.theme,
                                                            "secondary_foreground",
                                                        ),
                                                        px(12.),
                                                    ))
                                                    .child(
                                                        canvas(
                                                            move |bounds, _, cx| {
                                                                permission_picker_trigger_shell
                                                                    .update(cx, |shell, cx| {
                                                                        if shell.permission_picker_bounds
                                                                            != Some(bounds)
                                                                        {
                                                                            shell.permission_picker_bounds =
                                                                                Some(bounds);
                                                                            cx.notify();
                                                                        }
                                                                    })
                                                                    .ok();
                                                            },
                                                            |_bounds, _state, _window, _cx| {},
                                                        )
                                                        .absolute()
                                                        .inset_0()
                                                        .size_full(),
                                                    )
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme_rgb(
                                                &self.theme,
                                                "secondary_foreground",
                                            ))
                                            .cursor_pointer()
                                            .id("persona-picker-trigger")
                                            .relative()
                                            .role(Role::Button)
                                            .aria_label(SharedString::from(format!(
                                                "Choose persona: {persona_display}"
                                            )))
                                            .px(px(6.))
                                            .py(px(4.))
                                            .rounded(px(6.))
                                            .hover(|element| {
                                                element.bg(theme_rgb(&self.theme, "muted"))
                                            })
                                            .on_click(cx.listener(|shell, _, _, cx| {
                                                shell.toggle_persona_picker(cx);
                                            }))
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .child(tabler_icon(
                                                TablerIcon::UserCircle,
                                                theme_rgb(&self.theme, "secondary_foreground"),
                                                px(13.),
                                            ))
                                            .child(SharedString::from(persona_display))
                                            .child(tabler_icon(
                                                TablerIcon::ChevronDown,
                                                theme_rgb(&self.theme, "secondary_foreground"),
                                                px(12.),
                                            ))
                                            .child(
                                                canvas(
                                                    move |bounds, _, cx| {
                                                        persona_picker_trigger_shell
                                                            .update(cx, |shell, cx| {
                                                                if shell.persona_picker_bounds
                                                                    != Some(bounds)
                                                                {
                                                                    shell.persona_picker_bounds =
                                                                        Some(bounds);
                                                                    cx.notify();
                                                                }
                                                            })
                                                            .ok();
                                                    },
                                                    |_bounds, _state, _window, _cx| {},
                                                )
                                                .absolute()
                                                .inset_0()
                                                .size_full(),
                                            )
                                    )
                                    .child(if turn_is_running {
                                        div()
                                            .id("cancel-turn")
                                            .size(px(28.))
                                            .rounded_full()
                                            .bg(theme_rgb(&self.theme, "red.fg"))
                                            .text_color(theme_rgb(&self.theme, "background"))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .role(Role::Button)
                                            .aria_label("Cancel current turn")
                                            .on_click(cx.listener(|shell, _, _, cx| {
                                                shell.cancel_turn(cx);
                                            }))
                                            .child(tabler_icon(
                                                TablerIcon::Square,
                                                theme_rgb(&self.theme, "background"),
                                                px(12.),
                                            ))
                                            .into_any_element()
                                    } else {
                                        div()
                                            .id("submit-prompt")
                                            .size(px(28.))
                                            .rounded_full()
                                            .bg(theme_rgb(&self.theme, "text.body"))
                                            .text_color(theme_rgb(&self.theme, "background"))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .role(Role::Button)
                                            .aria_label("Send message")
                                            .on_click(cx.listener(|shell, _, _, cx| {
                                                shell.submit_prompt(cx);
                                            }))
                                            .child(tabler_icon(
                                                TablerIcon::ArrowUp,
                                                theme_rgb(&self.theme, "background"),
                                                px(15.),
                                            ))
                                            .into_any_element()
                                    }),
                            ),
                    )
                    .with_animation(
                        ElementId::Name(
                            format!("composer-input-{}", self.input_animation_id).into(),
                        ),
                        Animation::new(Duration::from_millis(180))
                            .with_easing(gpui::ease_out_quint()),
                        |element, delta| element.opacity(0.86 + delta * 0.14),
                    )
                    .child(
                        div()
                            .id("checkout-context")
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .px(px(4.))
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .min_w_0()
                                    .child(tabler_icon(
                                        TablerIcon::Folder,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(13.),
                                    ))
                                    .child(SharedString::from(
                                        session_path
                                            .as_deref()
                                            .unwrap_or("No workspace selected")
                                            .to_owned(),
                                    )),
                            )
                            .child(
                                div()
                                    .id("session-status")
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap(px(10.))
                                    .when_some(usage_label, |element, label| {
                                        element.child(
                                            div()
                                                .id("session-usage")
                                                .child(SharedString::from(label)),
                                        )
                                    })
                                    .when(presence_count > 0, |element| {
                                        element.child(
                                            div()
                                                .id("session-presence")
                                                .flex()
                                                .items_center()
                                                .gap(px(4.))
                                                .px(px(6.))
                                                .py(px(3.))
                                                .rounded(px(6.))
                                                .bg(theme_rgb(&self.theme, "card"))
                                                .child(tabler_icon(
                                                    TablerIcon::UserCircle,
                                                    theme_rgb(&self.theme, "text.muted"),
                                                    px(12.),
                                                ))
                                                .child(SharedString::from(format!(
                                                    "{presence_count} client{}",
                                                    if presence_count == 1 { "" } else { "s" }
                                                ))),
                                        )
                                    })
                                    .child(if control_yielded {
                                        div()
                                            .id("control-yielded")
                                            .text_color(theme_rgb(&self.theme, "text.warning"))
                                            .child("control yielded")
                                            .into_any_element()
                                    } else {
                                        div()
                                            .id("yield-control")
                                            .px(px(6.))
                                            .py(px(3.))
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .role(Role::Button)
                                            .aria_label("Yield control to another client")
                                            .hover(|element| {
                                                element.bg(theme_rgb(&self.theme, "muted"))
                                            })
                                            .on_click(cx.listener(|shell, _, _, cx| {
                                                shell.yield_control(cx);
                                            }))
                                            .child("Yield")
                                            .into_any_element()
                                    }),
                            ),
                    ),
            );

        let terminal_expanded = !self.layout.terminal_collapsed;
        let terminal_animation_id = self.terminal_animation_id;
        let terminal_surface = div()
            .id("terminal-surface")
            .flex()
            .flex_col()
            .flex_none()
            .min_w_0()
            .rounded(px(12.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .overflow_hidden()
            .bg(theme_rgb(&self.theme, "panel.background"))
            .child(self.render_remote_terminal_strip(cx));
        let terminal_surface = if terminal_animation_id == 0 {
            terminal_surface
                .h(px(if terminal_expanded {
                    TERMINAL_EXPANDED_HEIGHT
                } else {
                    0.
                }))
                .into_any_element()
        } else {
            terminal_surface
                .with_animation(
                    ElementId::Name(format!("terminal-transition-{terminal_animation_id}").into()),
                    Animation::new(Duration::from_millis(180)).with_easing(gpui::ease_out_quint()),
                    move |element, delta| {
                        let progress = if terminal_expanded { delta } else { 1. - delta };
                        element
                            .h(px(TERMINAL_EXPANDED_HEIGHT * progress))
                            .opacity(progress)
                            .relative()
                            .top(px((1. - progress) * 10.))
                    },
                )
                .into_any_element()
        };

        div()
            .id("center-workspace")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .gap(px(if terminal_expanded { SHELL_GUTTER } else { 0. }))
            .child(conversation)
            .child(terminal_surface)
    }

    pub(super) fn render_picker_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let window_height = window.bounds().size.height;
        let model_position = self.model_picker_open.then(|| {
            self.model_picker_bounds.map(|bounds| {
                picker_popup_position_in_window(
                    bounds,
                    model_picker_height(self.model.ui.models.len()),
                    window_height,
                    px(8.),
                    px(8.),
                )
            })
        });
        let persona_position = self.persona_picker_open.then(|| {
            self.persona_picker_bounds.map(|bounds| {
                picker_popup_position_in_window(
                    bounds,
                    persona_picker_height(self.model.ui.personas.len()),
                    window_height,
                    px(8.),
                    px(8.),
                )
            })
        });
        let permission_position = self.permission_picker_open.then(|| {
            self.permission_picker_bounds.map(|bounds| {
                picker_popup_position_in_window(
                    bounds,
                    persona_picker_height(PERMISSION_MODES.len()),
                    window_height,
                    px(8.),
                    px(8.),
                )
            })
        });
        let thinking_position = self.thinking_picker_open.then(|| {
            let option_count = thinking_variants_for_model(
                &self.model.ui.models,
                self.model.ui.current_provider.as_deref(),
                self.model.ui.model.as_deref(),
            )
            .len()
                + 1;
            self.thinking_picker_bounds.map(|bounds| {
                picker_popup_position_in_window(
                    bounds,
                    persona_picker_height(option_count),
                    window_height,
                    px(8.),
                    px(8.),
                )
            })
        });
        let slash_match_count = if self.slash_menu_is_open() {
            self.slash_menu_matches().len()
        } else {
            0
        };
        let slash_position = (slash_match_count > 0).then(|| {
            self.composer_bounds.map(|bounds| {
                picker_popup_position_in_window(
                    bounds,
                    slash_menu_height(slash_match_count),
                    window_height,
                    px(8.),
                    px(8.),
                )
            })
        });
        let mention_match_count = if self.mention_menu_is_open() {
            self.mention_menu_matches().len()
        } else {
            0
        };
        let mention_position = (mention_match_count > 0).then(|| {
            self.composer_bounds.map(|bounds| {
                picker_popup_position_in_window(
                    bounds,
                    mention_menu_height(mention_match_count),
                    window_height,
                    px(8.),
                    px(8.),
                )
            })
        });
        let pending_actions_position = (self.model.session_is_ready()
            && !self.model.ui.pending_actions.is_empty())
        .then(|| self.composer_bounds.map(pending_actions_anchor));
        let pending_actions_width = self.composer_bounds.map(pending_actions_width);

        div()
            .id("picker-overlays")
            .absolute()
            .inset_0()
            .when_some(model_position.flatten(), |element, position| {
                element.child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .position(position)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_model_picker(cx)),
                    )
                    .with_priority(4),
                )
            })
            .when_some(persona_position.flatten(), |element, position| {
                element.child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .position(position)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_persona_picker(cx)),
                    )
                    .with_priority(4),
                )
            })
            .when_some(permission_position.flatten(), |element, position| {
                element.child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .position(position)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_permission_picker(cx)),
                    )
                    .with_priority(4),
                )
            })
            .when_some(thinking_position.flatten(), |element, position| {
                element.child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .position(position)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_thinking_picker(cx)),
                    )
                    .with_priority(4),
                )
            })
            .when_some(slash_position.flatten(), |element, position| {
                element.child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .position(position)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_slash_menu(cx)),
                    )
                    .with_priority(5),
                )
            })
            .when_some(mention_position.flatten(), |element, position| {
                element.child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .position(position)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_mention_menu(cx)),
                    )
                    .with_priority(5),
                )
            })
            .when_some(pending_actions_position.flatten(), |element, position| {
                element.child(
                    deferred(
                        anchored()
                            .anchor(Anchor::BottomLeft)
                            .position(position)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.render_pending_actions(pending_actions_width, cx)),
                    )
                    .with_priority(6),
                )
            })
            .into_any_element()
    }
}

fn split_tool_text_lines(text: &str) -> Vec<SharedString> {
    if text.is_empty() {
        vec![SharedString::from("")]
    } else {
        text.split('\n').map(SharedString::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{formatted_tool_input, split_tool_text_lines, tool_display_label, tool_summary};
    use gpui::SharedString;

    #[test]
    fn tool_names_are_presented_as_human_facing_categories() {
        assert_eq!(tool_display_label("bash"), "Terminal");
        assert_eq!(tool_display_label("browser_open"), "Browser");
        assert_eq!(tool_display_label("write_plan"), "Plan");
        assert_eq!(
            tool_display_label("mew.agents.code-reviewer"),
            "Code Reviewer"
        );
    }

    #[test]
    fn tool_summary_surfaces_the_action_without_dumping_json() {
        assert_eq!(
            tool_summary(r#"{"command":"cargo test"}"#).as_deref(),
            Some("cargo test")
        );
        assert_eq!(
            tool_summary(r#"{"path":"src/main.rs"}"#).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(tool_summary(r#"{"plan":["one"]}"#), None);
        assert_eq!(tool_summary("not json"), None);
    }

    #[test]
    fn tool_input_is_pretty_printed_when_it_is_json() {
        assert_eq!(
            formatted_tool_input(r#"{"command":"pwd","cwd":"/tmp"}"#),
            "{\n  \"command\": \"pwd\",\n  \"cwd\": \"/tmp\"\n}"
        );
        assert_eq!(formatted_tool_input("not json"), "not json");
    }

    #[test]
    fn tool_text_lines_keep_empty_output_and_newlines() {
        assert_eq!(split_tool_text_lines(""), vec![SharedString::from("")]);
        assert_eq!(
            split_tool_text_lines("one\n\ntwo"),
            vec![
                SharedString::from("one"),
                SharedString::from(""),
                SharedString::from("two")
            ]
        );
    }
}
