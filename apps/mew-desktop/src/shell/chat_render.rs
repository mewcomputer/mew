use super::chat::{ToolTextSection, TranscriptCopySource};
use super::*;

const TRANSCRIPT_SCROLLBAR_MIN_THUMB: f32 = 28.;
const TRANSCRIPT_SCROLLBAR_HIT_WIDTH: f32 = 16.;
const TRANSCRIPT_SCROLLBAR_THUMB_WIDTH: f32 = 6.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TranscriptScrollbarMetrics {
    pub(super) viewport_height: f32,
    pub(super) max_offset: f32,
    pub(super) thumb_height: f32,
    pub(super) thumb_top: f32,
    pub(super) travel: f32,
}

pub(super) fn transcript_scrollbar_metrics(
    list: &gpui::ListState,
) -> Option<TranscriptScrollbarMetrics> {
    let viewport_height = f32::from(list.viewport_bounds().size.height);
    let max_offset = f32::from(list.max_offset_for_scrollbar().y).max(0.);
    if viewport_height <= 1. || max_offset <= 1. {
        return None;
    }

    let thumb_height = (viewport_height * viewport_height / (viewport_height + max_offset))
        .clamp(TRANSCRIPT_SCROLLBAR_MIN_THUMB, viewport_height);
    let travel = (viewport_height - thumb_height).max(0.);
    let scroll_offset = (-f32::from(list.scroll_px_offset_for_scrollbar().y)).clamp(0., max_offset);
    let thumb_top = if travel > 0. {
        scroll_offset / max_offset * travel
    } else {
        0.
    };

    Some(TranscriptScrollbarMetrics {
        viewport_height,
        max_offset,
        thumb_height,
        thumb_top,
        travel,
    })
}

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

fn tool_activity(tool_name: &str) -> (&'static str, TablerIcon) {
    match tool_name.rsplit(['.', ':']).next().unwrap_or(tool_name) {
        "read" | "read_file" => ("Read files", TablerIcon::File),
        "write" | "write_file" | "edit" | "edit_file" | "edit_str_replace" | "apply_patch" => {
            ("Edited files", TablerIcon::Pencil)
        }
        "grep" | "glob" | "search" | "search_files" | "list" | "list_directory" => {
            ("Searched files", TablerIcon::Search)
        }
        "bash" | "sh" | "shell" | "exec" | "exec_command" | "run_command" => {
            ("Ran commands", TablerIcon::Terminal2)
        }
        name if name.starts_with("browser") || name == "web_fetch" => {
            ("Browsed pages", TablerIcon::Browser)
        }
        _ => ("Used tools", TablerIcon::Dots),
    }
}

fn tool_activity_summary<'a>(parts: impl Iterator<Item = &'a TranscriptPart>) -> String {
    let mut activities = Vec::new();
    for part in parts {
        if let TranscriptPart::ToolCall { tool_name, .. } = part {
            let (label, _) = tool_activity(tool_name);
            if !activities.contains(&label) {
                activities.push(label);
            }
        }
    }
    let mut summary = String::new();
    for label in activities {
        if summary.is_empty() {
            summary.push_str(label);
        } else {
            summary.push_str(", ");
            summary.extend(label.chars().map(|ch| ch.to_ascii_lowercase()));
        }
    }
    summary
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

const TOOL_DIFF_LINES_MAX: usize = 40;
const TOOL_OUTPUT_LINES_FOR_DISCLOSURE: usize = 12;

#[derive(Clone, Copy)]
pub(super) enum ActionButtonTone {
    Primary,
    Secondary,
    Destructive,
}

impl DesktopShell {
    pub(super) fn render_action_button(
        &self,
        id: String,
        label: impl Into<SharedString>,
        command: ClientMessage,
        tone: ActionButtonTone,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let label = label.into();
        let (background, foreground, hover) = match tone {
            ActionButtonTone::Primary => (
                theme_rgb(&self.theme, "text.body"),
                theme_rgb(&self.theme, "background"),
                theme_rgb(&self.theme, "text.body").opacity(0.82),
            ),
            ActionButtonTone::Secondary => (
                theme_rgb(&self.theme, "card"),
                theme_rgb(&self.theme, "text.body"),
                theme_rgb(&self.theme, "muted"),
            ),
            ActionButtonTone::Destructive => (
                theme_rgb(&self.theme, "red.fg").opacity(0.14),
                theme_rgb(&self.theme, "red.fg"),
                theme_rgb(&self.theme, "red.fg").opacity(0.24),
            ),
        };
        div()
            .id(id)
            .px(px(10.))
            .py(px(6.))
            .rounded(px(7.))
            .cursor_pointer()
            .bg(background)
            .text_color(foreground)
            .text_xs()
            .role(Role::Button)
            .aria_label(label.clone())
            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
            .hover(move |element| element.bg(hover))
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
                            ActionButtonTone::Primary,
                            cx,
                        ),
                        self.render_action_button(
                            format!("action-{request_id}-allow-session"),
                            "allow session",
                            ClientMessage::PermissionResponse {
                                request_id: request_id.clone(),
                                decision: PermissionDecision::AllowSession,
                            },
                            ActionButtonTone::Secondary,
                            cx,
                        ),
                        self.render_action_button(
                            format!("action-{request_id}-deny"),
                            "deny",
                            ClientMessage::PermissionResponse {
                                request_id,
                                decision: PermissionDecision::Deny,
                            },
                            ActionButtonTone::Destructive,
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
                        ActionButtonTone::Primary,
                        cx,
                    ),
                    self.render_action_button(
                        format!("action-{request_id}-deny"),
                        "deny",
                        ClientMessage::PermissionResponse {
                            request_id,
                            decision: PermissionDecision::Deny,
                        },
                        ActionButtonTone::Destructive,
                        cx,
                    ),
                ]))
                .into_any_element(),
            ActionKind::AskUser {
                questions,
                subagent,
                ..
            } => {
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
                                        cancelled: false,
                                        request_id: request_id.clone(),
                                        answers: vec![answer],
                                    },
                                    ActionButtonTone::Secondary,
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
                    .child(
                        div().text_sm().child(SharedString::from(
                            subagent
                                .as_ref()
                                .map(|source| {
                                    format!(
                                        "input needed from {} ({})",
                                        source.display_name.as_deref().unwrap_or(&source.name),
                                        source.name
                                    )
                                })
                                .unwrap_or_else(|| "input needed".into()),
                        )),
                    )
                    .when(subagent.is_some(), |element| {
                        element.child(self.render_action_button(
                            format!("action-{request_id}-close-question"),
                            "close question",
                            ClientMessage::AskUserResponse {
                                request_id: request_id.clone(),
                                answers: vec![],
                                cancelled: true,
                            },
                            ActionButtonTone::Secondary,
                            cx,
                        ))
                    })
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
                        ActionButtonTone::Primary,
                        cx,
                    ),
                    self.render_action_button(
                        format!("action-{request_id}-decline"),
                        "decline",
                        ClientMessage::GoalResponse {
                            request_id,
                            accepted: false,
                        },
                        ActionButtonTone::Destructive,
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
        let previous = index
            .checked_sub(1)
            .and_then(|index| self.transcript_rows.get(index));
        let next = self.transcript_rows.get(index + 1);
        let grouped_call = row.tool_group_len == 0
            && matches!(
                part,
                TranscriptPart::ToolCall {
                    status: ToolStatus::Completed,
                    ..
                }
            )
            && previous.is_some_and(|previous| {
                previous.tool_group_len > 0
                    || matches!(
                        self.model.ui.transcript[previous.message_index]
                            .parts
                            .get(previous.part_index),
                        Some(TranscriptPart::ToolCall {
                            status: ToolStatus::Completed,
                            ..
                        })
                    )
            });
        let space_after = next.is_some_and(|next| {
            next.message_index == row.message_index && next.part_index == row.part_index
        }) && !cached
            .and_then(|cached| cached.render_blocks.get(row.block_index + 1))
            .is_some_and(|block| block.continuation);
        let selection_block_index = row
            .part_index
            .saturating_mul(1_000_000)
            .saturating_add(row.block_index);
        let content = match part {
            _ if row.tool_group_len > 0 => self.render_tool_group(row, cx),
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
                    space_after,
                    animate,
                    !is_user,
                    &render_block.syntax_highlights,
                    cx,
                )
            }
            TranscriptPart::Reasoning(_) => {
                self.render_reasoning_part(row, cached, selection_block_index, space_after, cx)
            }
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
            TranscriptPart::File { label, .. } => self.render_meta_part(
                row.message_index,
                row.part_index,
                TablerIcon::FileCode,
                "attachment",
                label,
            ),
            TranscriptPart::Compaction {
                auto,
                summary: _,
                removed_count: _,
            } => self.render_meta_part(
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
            .justify_center()
            // Keep the first loaded turn's geometry stable when history is prepended.
            .when(
                !grouped_call
                    && previous.is_none_or(|previous| previous.message_index != row.message_index),
                |element| element.pt(px(24.)),
            )
            .when(
                !grouped_call
                    && previous.is_some_and(|previous| {
                        previous.message_index == row.message_index
                            && previous.part_index != row.part_index
                    }),
                |element| element.pt(px(8.)),
            )
            .when(grouped_call, |element| element.pt(px(6.)))
            .child(
                div()
                    .flex()
                    .w_full()
                    .max_w(px(CHAT_CONTENT_MAX_WIDTH))
                    .min_w_0()
                    .when(is_user, |element| element.justify_end())
                    .child(
                        div()
                            .debug_selector(move || format!("transcript-content-{index}"))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .min_w_0()
                            .when(grouped_call, |element| element.pl(px(20.)))
                            .when(!is_user, |element| element.w_full().min_w_0())
                            .when(is_user, |element| {
                                element
                                    .max_w(relative(0.9))
                                    .p(px(12.))
                                    .when(continues_before, |element| element.pt(px(2.)))
                                    .when(continues_after, |element| element.pb(px(2.)))
                                    .rounded(px(14.))
                                    .bg(theme_rgb(&self.theme, "card"))
                            })
                            .child(
                                div()
                                    .when(!is_user, |element| element.w_full().min_w_0())
                                    .text_size(px(15.))
                                    .line_height(px(24.))
                                    .text_color(theme_rgb(&self.theme, "text.body"))
                                    .child(content),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn toggle_chat_part(&mut self, key: String, cx: &mut Context<Self>) {
        let group_keys = self
            .transcript_rows
            .iter()
            .copied()
            .find(|row| {
                row.tool_group_len > 0 && self.tool_group_key(*row).as_deref() == Some(key.as_str())
            })
            .map(|row| {
                self.tool_group_members(row)
                    .flat_map(|(message, part_index, part)| match part {
                        TranscriptPart::ToolCall { call_id, .. } => [
                            format!("tool-group-{call_id}"),
                            Self::chat_part_key(message, part_index),
                        ],
                        _ => unreachable!("tool groups contain only tool calls"),
                    })
                    .collect::<Vec<_>>()
            });
        if let Some(keys) = group_keys.filter(|keys| {
            keys.iter()
                .any(|key| self.expanded_chat_parts.contains(key))
        }) {
            for key in keys {
                self.expanded_chat_parts.remove(&key);
            }
        } else if !self.expanded_chat_parts.remove(&key) {
            self.expanded_chat_parts.insert(key);
        }
        self.rebuild_transcript_rows_from_cache();
        self.sync_transcript_list();
        self.capture_session_view_state();
        self.persist_layout();
        cx.notify();
    }

    pub(super) fn chat_part_key(message_index: usize, part_index: usize) -> String {
        format!("chat-part-{message_index}-{part_index}")
    }

    pub(super) fn tool_group_key(&self, row: TranscriptRenderRow) -> Option<String> {
        match self
            .model
            .ui
            .transcript
            .get(row.message_index)?
            .parts
            .get(row.part_index)?
        {
            TranscriptPart::ToolCall { call_id, .. } => Some(format!("tool-group-{call_id}")),
            _ => None,
        }
    }

    fn tool_group_members(
        &self,
        row: TranscriptRenderRow,
    ) -> impl Iterator<Item = (usize, usize, &TranscriptPart)> {
        self.model
            .ui
            .transcript
            .iter()
            .enumerate()
            .skip(row.message_index)
            .flat_map(move |(message, item)| {
                item.parts
                    .iter()
                    .enumerate()
                    .skip(if message == row.message_index {
                        row.part_index
                    } else {
                        0
                    })
                    .map(move |(part_index, part)| (message, part_index, part))
            })
            .take(row.tool_group_len)
    }

    fn tool_group_parts(&self, row: TranscriptRenderRow) -> impl Iterator<Item = &TranscriptPart> {
        self.tool_group_members(row).map(|(_, _, part)| part)
    }

    pub(super) fn tool_group_is_expanded(&self, row: TranscriptRenderRow) -> bool {
        !self.expanded_chat_parts.is_empty()
            && self
                .tool_group_members(row)
                .any(|(message, part_index, part)| match part {
                    TranscriptPart::ToolCall { call_id, .. } => {
                        self.expanded_chat_parts
                            .contains(&format!("tool-group-{call_id}"))
                            || self
                                .expanded_chat_parts
                                .contains(&Self::chat_part_key(message, part_index))
                    }
                    _ => false,
                })
    }

    fn render_tool_group(
        &self,
        row: TranscriptRenderRow,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = self
            .tool_group_key(row)
            .expect("tool groups start at a tool call");
        let expanded = self.tool_group_is_expanded(row);
        let (_, icon) = match &self.model.ui.transcript[row.message_index].parts[row.part_index] {
            TranscriptPart::ToolCall { tool_name, .. } => tool_activity(tool_name),
            _ => unreachable!(),
        };
        div()
            .w_full()
            .min_w_0()
            .line_height(px(20.))
            .child(self.render_part_header(
                key,
                icon,
                tool_activity_summary(self.tool_group_parts(row)),
                Some((
                    format!("{} calls", row.tool_group_len),
                    theme_rgb(&self.theme, "text.muted"),
                )),
                !expanded,
                cx,
            ))
            .into_any_element()
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
            .debug_selector(move || format!("{key}-header"))
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap(px(7.))
            .cursor_pointer()
            .role(Role::Button)
            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
            .child(div().min_w_0().truncate().child(SharedString::from(label)))
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
        row: TranscriptRenderRow,
        cached: Option<&CachedMarkdown>,
        selection_block_index: usize,
        space_after: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let TranscriptRenderRow {
            message_index,
            part_index,
            block_index,
            ..
        } = row;
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
                            space_after,
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
        let compact = collapsed && status == ToolStatus::Completed;
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
            tool_activity(tool_name).1,
            display_name,
            Some((status_label.into(), status_color)),
            collapsed,
            cx,
        );
        let summary_view = summary.map(|summary| {
            div()
                .pl(px(20.))
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme_rgb(
                    &self.theme,
                    if compact { "text.muted" } else { "text.body" },
                ))
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
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child("input")
                        .child(self.render_content_copy_button(
                            format!("copy-tool-input-{message_index}-{part_index}"),
                            "Copy tool input",
                            TranscriptCopySource::Tool {
                                message_index,
                                part_index,
                                section: ToolTextSection::Input,
                            },
                            cx,
                        )),
                )
                .child(self.render_tool_text(
                    format!("tool-input-{message_index}-{part_index}"),
                    &formatted_tool_input(input),
                    false,
                    message_index,
                    part_index,
                    ToolTextSection::Input,
                    cx,
                ));
            if let Some(output) = output.filter(|output| !output.is_empty()) {
                let output_key = format!("tool-output-{message_index}-{part_index}");
                let output_expanded = self.expanded_chat_parts.contains(&output_key);
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(div().flex_1().child("output"))
                        .child(self.render_content_copy_button(
                            format!("copy-tool-output-{message_index}-{part_index}"),
                            "Copy tool output",
                            TranscriptCopySource::Tool {
                                message_index,
                                part_index,
                                section: ToolTextSection::Output,
                            },
                            cx,
                        ))
                        .when(
                            output.lines().count() > TOOL_OUTPUT_LINES_FOR_DISCLOSURE,
                            |element| {
                                element.child(self.render_tool_toggle(
                                    format!("{output_key}-toggle"),
                                    output_key.clone(),
                                    output_expanded,
                                    "output",
                                    cx,
                                ))
                            },
                        ),
                );
                body = body.child(self.render_tool_text(
                    output_key,
                    output,
                    output_expanded,
                    message_index,
                    part_index,
                    ToolTextSection::Output,
                    cx,
                ));
            }
            if let Some(diff) = diff.filter(|diff| !diff.is_empty()) {
                let diff_key = format!("tool-diff-{message_index}-{part_index}");
                let diff_expanded = self.expanded_chat_parts.contains(&diff_key);
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(div().flex_1().child("diff"))
                        .child(self.render_content_copy_button(
                            format!("copy-tool-diff-{message_index}-{part_index}"),
                            "Copy tool diff",
                            TranscriptCopySource::Tool {
                                message_index,
                                part_index,
                                section: ToolTextSection::Diff,
                            },
                            cx,
                        ))
                        .when(diff.lines().count() > TOOL_DIFF_LINES_MAX, |element| {
                            element.child(self.render_tool_toggle(
                                format!("{diff_key}-toggle"),
                                diff_key.clone(),
                                diff_expanded,
                                "diff",
                                cx,
                            ))
                        }),
                );
                body = body.child(self.render_tool_diff(
                    diff_key,
                    diff,
                    diff_expanded,
                    message_index,
                    part_index,
                    cx,
                ));
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
            .gap(px(4.))
            .line_height(px(20.))
            .when(!compact, |element| {
                element
                    .p(px(10.))
                    .rounded(px(9.))
                    .bg(theme_rgb(&self.theme, "card").opacity(0.46))
                    .border_1()
                    .border_color(theme_rgb(&self.theme, "divider").opacity(0.7))
            })
            .child(header)
            .when_some(summary_view, |element, summary| element.child(summary))
            .when_some(body, |element, body| element.child(body))
            .into_any_element()
    }

    fn render_tool_toggle(
        &self,
        id: String,
        key: String,
        expanded: bool,
        noun: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let label = if expanded {
            format!("collapse {noun}")
        } else {
            format!("expand {noun}")
        };
        div()
            .id(id)
            .px(px(5.))
            .py(px(2.))
            .rounded(px(4.))
            .cursor_pointer()
            .role(Role::Button)
            .aria_label(SharedString::from(label.clone()))
            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
            .on_click(cx.listener(move |shell, _, _, cx| {
                cx.stop_propagation();
                shell.toggle_chat_part(key.clone(), cx);
            }))
            .child(SharedString::from(label))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_tool_text(
        &self,
        id: String,
        text: &str,
        expanded: bool,
        message_index: usize,
        part_index: usize,
        section: ToolTextSection,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let source_identity = text.as_ptr() as usize;
        let source_len = text.len();
        let (lines, line_offsets) = {
            let mut cache = self.tool_text_cache.borrow_mut();
            let cached = cache.entry(id.clone()).or_insert_with(|| {
                let lines = split_tool_text_lines(text);
                ToolTextCache {
                    source_identity,
                    source_len,
                    line_offsets: Arc::new(tool_line_offsets(&lines)),
                    lines: Arc::new(lines),
                }
            });
            if cached.source_identity != source_identity || cached.source_len != source_len {
                cached.source_identity = source_identity;
                cached.source_len = source_len;
                cached.lines = Arc::new(split_tool_text_lines(text));
                cached.line_offsets = Arc::new(tool_line_offsets(&cached.lines));
            }
            (cached.lines.clone(), cached.line_offsets.clone())
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
            .max_h(if expanded { px(520.) } else { px(180.) })
            .overflow_x_scroll()
            .overflow_y_scroll()
            .bg(theme_rgb(&self.theme, "background"))
            .rounded(px(6.))
            .p(px(8.))
            .child(
                gpui::list(
                    state,
                    cx.processor(move |shell, index: usize, _, cx| {
                        let Some(line) = lines.get(index) else {
                            return div().into_any_element();
                        };
                        let block_index = part_index
                            .saturating_mul(1_000_000)
                            .saturating_add(section.index());
                        let source_offset = line_offsets[index];
                        let highlights = shell.transcript_text_highlights(
                            message_index,
                            block_index,
                            source_offset,
                            line.len(),
                            Vec::new(),
                        );
                        div()
                            .id(format!("{id}-line-{index}"))
                            .debug_selector(|| format!("{id}-line-{index}"))
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .line_height(px(18.))
                            .font_family(DEFAULT_FONT_FAMILY)
                            .text_color(text_color)
                            .whitespace_nowrap()
                            .child(shell.render_selectable_transcript_text(
                                message_index,
                                block_index,
                                source_offset,
                                line,
                                &[],
                                gpui::StyledText::new(line.clone()).with_highlights(highlights),
                                cx,
                            ))
                            .into_any_element()
                    }),
                )
                .with_sizing_behavior(gpui::ListSizingBehavior::Infer),
            )
            .into_any_element()
    }

    fn render_tool_diff(
        &self,
        id: String,
        diff: &str,
        expanded: bool,
        message_index: usize,
        part_index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let line_count = diff.lines().count();
        let truncated = !expanded && line_count > TOOL_DIFF_LINES_MAX;
        let mut source_offset = 0;
        let lines = diff
            .split('\n')
            .take(if expanded {
                line_count
            } else {
                TOOL_DIFF_LINES_MAX
            })
            .map(|line| {
                let offset = source_offset;
                source_offset += line.len() + 1;
                let color = if line.starts_with('+') {
                    theme_rgb(&self.theme, "green.fg")
                } else if line.starts_with('-') {
                    theme_rgb(&self.theme, "red.fg")
                } else {
                    theme_rgb(&self.theme, "text.muted")
                };
                (SharedString::from(line.to_owned()), color, offset)
            })
            .collect::<Arc<[_]>>();
        let visible_line_count = lines.len();
        let state = {
            let mut lists = self.tool_text_lists.borrow_mut();
            lists
                .entry(id.clone())
                .or_insert_with(|| gpui::ListState::new(0, gpui::ListAlignment::Top, px(96.)))
                .clone()
        };
        if state.item_count() != visible_line_count {
            state.reset(visible_line_count);
        }
        div()
            .id(id.clone())
            .max_h(if expanded { px(520.) } else { px(180.) })
            .overflow_x_scroll()
            .overflow_y_scroll()
            .bg(theme_rgb(&self.theme, "background"))
            .rounded(px(6.))
            .p(px(8.))
            .child(
                gpui::list(
                    state,
                    cx.processor(move |shell, index: usize, _, cx| {
                        let Some((line, color, source_offset)) = lines.get(index) else {
                            return div().into_any_element();
                        };
                        let block_index = part_index
                            .saturating_mul(1_000_000)
                            .saturating_add(ToolTextSection::Diff.index());
                        let highlights = shell.transcript_text_highlights(
                            message_index,
                            block_index,
                            *source_offset,
                            line.len(),
                            Vec::new(),
                        );
                        div()
                            .id(format!("{id}-line-{index}"))
                            .debug_selector(|| format!("{id}-line-{index}"))
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .line_height(px(18.))
                            .font_family(DEFAULT_FONT_FAMILY)
                            .text_color(*color)
                            .whitespace_nowrap()
                            .child(shell.render_selectable_transcript_text(
                                message_index,
                                block_index,
                                *source_offset,
                                line,
                                &[],
                                gpui::StyledText::new(line.clone()).with_highlights(highlights),
                                cx,
                            ))
                            .into_any_element()
                    }),
                )
                .with_sizing_behavior(gpui::ListSizingBehavior::Infer),
            )
            .when(truncated, |element| {
                element.child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(format!("… {} more lines", line_count - TOOL_DIFF_LINES_MAX)),
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
        if old_count == count {
            self.transcript_rows_prepend_count = None;
        }
        if old_count != count {
            let was_at_end =
                old_count == 0 || self.transcript_list.is_scrolled_to_end().unwrap_or(false);
            let prepended_rows =
                self.transcript_rows_prepend_count
                    .take()
                    .and_then(|message_count| {
                        Self::transcript_prepend_row_count(
                            &self.transcript_rows,
                            old_count,
                            count,
                            message_count,
                        )
                    });
            if let Some(prepended_rows) = prepended_rows {
                // ListState keeps its logical anchor when rows are inserted before the
                // viewport. Resetting the list here loses that anchor and makes a history
                // page visibly jump while the newly loaded rows are measured.
                self.transcript_list.splice(0..0, prepended_rows);
            } else if self.transcript_rows_append_only && count >= old_count {
                self.transcript_list
                    .splice(old_count..old_count, count - old_count);
            } else {
                self.transcript_list.reset(count);
            }
            if count > 0 && was_at_end {
                self.transcript_list.scroll_to_end();
            }
        }
        self.restore_pending_transcript_scroll_anchor();
    }

    fn transcript_prepend_row_count(
        rows: &[TranscriptRenderRow],
        old_count: usize,
        count: usize,
        added_messages: usize,
    ) -> Option<usize> {
        if added_messages == 0 || count < old_count {
            return None;
        }
        let prepended_rows = rows
            .iter()
            .take_while(|row| row.message_index < added_messages)
            .count();
        (prepended_rows > 0 && old_count + prepended_rows == count).then_some(prepended_rows)
    }

    pub(super) fn render_transcript_scrollbar(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let metrics = transcript_scrollbar_metrics(&self.transcript_list)?;
        let scroll_offset = (-f32::from(self.transcript_list.scroll_px_offset_for_scrollbar().y))
            .clamp(0., metrics.max_offset);
        let percentage = if metrics.max_offset > 0. {
            scroll_offset / metrics.max_offset * 100.
        } else {
            0.
        };
        let thumb = div()
            .id("conversation-scrollbar-thumb")
            .absolute()
            .top(px(metrics.thumb_top))
            .right(px((TRANSCRIPT_SCROLLBAR_HIT_WIDTH
                - TRANSCRIPT_SCROLLBAR_THUMB_WIDTH)
                / 2.))
            .w(px(TRANSCRIPT_SCROLLBAR_THUMB_WIDTH))
            .h(px(metrics.thumb_height))
            .rounded(px(TRANSCRIPT_SCROLLBAR_THUMB_WIDTH / 2.))
            .bg(theme_rgb(&self.theme, "text.muted").opacity(0.48))
            .hover(|element| element.bg(theme_rgb(&self.theme, "text.body").opacity(0.72)))
            .cursor(gpui::CursorStyle::PointingHand)
            .role(Role::ScrollBar)
            .aria_label("Conversation scroll")
            .aria_orientation(gpui::Orientation::Vertical)
            .aria_min_numeric_value(0.)
            .aria_max_numeric_value(100.)
            .aria_numeric_value(f64::from(percentage))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|shell, event, window, cx| {
                    shell.transcript_scrollbar_mouse_down(event, window, cx);
                    cx.stop_propagation();
                }),
            );
        Some(
            div()
                .id("conversation-scrollbar")
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .w(px(TRANSCRIPT_SCROLLBAR_HIT_WIDTH))
                .cursor(gpui::CursorStyle::Arrow)
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(Self::transcript_scrollbar_mouse_down),
                )
                .child(thumb)
                .into_any_element(),
        )
    }

    fn restore_pending_transcript_scroll_anchor(&mut self) {
        let Some(anchor) = self.pending_transcript_scroll_anchor else {
            return;
        };
        let exact = self.transcript_rows.iter().position(|row| {
            row.message_index == anchor.message_index
                && row.part_index == anchor.part_index
                && row.block_index == anchor.block_index
        });
        let grouped = || {
            let part = self
                .model
                .ui
                .transcript
                .get(anchor.message_index)?
                .parts
                .get(anchor.part_index)?;
            self.transcript_rows.iter().position(|row| {
                row.tool_group_len > 0
                    && self
                        .tool_group_parts(*row)
                        .any(|member| std::ptr::eq(member, part))
            })
        };
        let Some(item_ix) = exact.or_else(grouped) else {
            return;
        };
        self.transcript_list
            .set_follow_mode(gpui::FollowMode::Normal);
        self.transcript_list.scroll_to(gpui::ListOffset {
            item_ix,
            offset_in_item: px(if exact.is_some() {
                anchor.offset.max(0.)
            } else {
                0.
            }),
        });
        self.pending_transcript_scroll_anchor = None;
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
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
        if let Some(frame_probe) = self.frame_probe.as_mut() {
            frame_probe.record_transcript_render();
        }
        if self.transcript_selection.is_some() {
            self.transcript_text_registry.borrow_mut().clear();
        }
        let selected_session = self.model.ui.selected_session.is_some();
        let selected_conversation =
            self.model
                .ui
                .selected_session
                .as_deref()
                .and_then(|selected| {
                    self.model
                        .ui
                        .conversations
                        .iter()
                        .find(|conversation| conversation.session_id == selected)
                });
        let last_turn_failed =
            selected_conversation.is_some_and(|conversation| conversation.last_turn_failed);
        let failure_detail =
            selected_conversation.and_then(|conversation| conversation.failure_detail.clone());
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
                            "Choose a chat in the sidebar, or start a new chat."
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
                    div()
                        .id("conversation-transcript-list")
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .px(px(CHAT_TRANSCRIPT_PADDING))
                        .child(
                            gpui::list(
                                self.transcript_list.clone(),
                                cx.processor(Self::render_transcript_row),
                            )
                            .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .pb(px(CHAT_TRANSCRIPT_PADDING)),
                        )
                        .when_some(
                            self.render_transcript_scrollbar(cx),
                            |element, scrollbar| element.child(scrollbar),
                        ),
                )
                .when_some(transcript_attention_state, |element, attention| {
                    element.child(
                        div()
                            .w_full()
                            .max_w(px(CHAT_CONTENT_MAX_WIDTH + CHAT_TRANSCRIPT_PADDING * 2.))
                            .px(px(CHAT_TRANSCRIPT_PADDING))
                            .self_center()
                            .child(self.render_transcript_attention(
                                attention,
                                if matches!(attention, TranscriptAttention::Failed) {
                                    failure_detail.as_deref()
                                } else {
                                    None
                                },
                                cx,
                            )),
                    )
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
        let session_path: SharedString = selected_session_path(
            &self.model.ui.conversations,
            self.model.ui.selected_session.as_deref(),
        )
        .unwrap_or_else(|| "No workspace selected".into())
        .into();
        let usage_label = self.model.ui.usage.and_then(usage_summary_label);
        let presence_count = self.model.ui.presence.len();
        let control_yielded = self.model.ui.control_yielded_by.is_some();

        let composer_is_empty = self.model.ui.composer.is_empty();
        let composer_value = self.model.ui.composer.clone();
        let turn_is_running = self.model.ui.running;
        let composer_focused = self.composer_focus_handle.is_focused(window);
        let attachments = self.attachments.clone();
        let attachment_error = self.attachment_error.clone();
        let composer_focus_handle = self.composer_focus_handle.clone();
        let model_picker_trigger_shell = cx.entity().downgrade();
        let persona_picker_trigger_shell = cx.entity().downgrade();
        let permission_picker_trigger_shell = cx.entity().downgrade();

        let conversation = div()
            .id("conversation-surface")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(theme_rgb(&self.theme, "background"))
            .child(
                div()
                    .id("conversation-transcript")
                    .track_focus(&self.transcript_focus_handle)
                    .key_context("Transcript")
                    .when(self.transcript_selected_text.is_some(), |element| {
                        element.on_action(cx.listener(Self::copy_selected_transcript_text))
                    })
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .w_full()
                    .gap(px(20.))
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
                    .child(transcript)
                    .when(self.transcript_scrolled_away, |element| {
                        element.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(px(12.))
                                .flex()
                                .justify_center()
                                .child(
                                    div()
                                        .id("jump-to-latest")
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .px(px(9.))
                                        .py(px(6.))
                                        .rounded(px(7.))
                                        .cursor_pointer()
                                        .role(Role::Button)
                                        .aria_label("Jump to latest message")
                                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                        .bg(theme_rgb(&self.theme, "card"))
                                        .border_1()
                                        .border_color(theme_rgb(&self.theme, "divider"))
                                        .shadow_sm()
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.body"))
                                        .hover(|element| {
                                            element.bg(theme_rgb(&self.theme, "muted"))
                                        })
                                        .on_click(cx.listener(|shell, _, _, cx| {
                                            cx.stop_propagation();
                                            shell.jump_to_latest(cx);
                                        }))
                                        .child(tabler_icon(
                                            TablerIcon::ChevronDown,
                                            theme_rgb(&self.theme, "text.muted"),
                                            px(13.),
                                        ))
                                        .child("jump to latest"),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .id("composer-area")
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(px(CHAT_CONTENT_MAX_WIDTH + CHAT_TRANSCRIPT_PADDING * 2.))
                    .self_center()
                    .gap(px(8.))
                    .px(px(CHAT_TRANSCRIPT_PADDING))
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
                                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                        .aria_label("Retry daemon connection")
                                        .on_click(cx.listener(|shell, _, _, cx| {
                                            shell.retry_connection(cx);
                                        }))
                                        .child("retry")
                                        .into_any_element()
                                } else if self.session_requests.needs_reconnect()
                                    || self.command_tx.as_ref().is_some_and(UnboundedSender::is_closed)
                                {
                                    div()
                                        .id("reconnect-task-client")
                                        .px(px(7.))
                                        .py(px(4.))
                                        .rounded(px(5.))
                                        .bg(theme_rgb(&self.theme, "card"))
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.body"))
                                        .cursor_pointer()
                                        .role(Role::Button)
                                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                        .aria_label("Reconnect client to confirm task attachment")
                                        .on_click(cx.listener(|shell, _, _, cx| {
                                            shell.retry_connection(cx);
                                        }))
                                        .child("reconnect")
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
                                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                            .debug_selector(|| "composer".into())
                            .flex_col()
                            .items_stretch()
                            .gap(px(8.))
                            .min_h_0()
                            .border_1()
                            .border_color(theme_rgb(&self.theme, "divider"))
                            .rounded(px(16.))
                            .bg(theme_rgb(&self.theme, "panel.background"))
                            .p(px(10.))
                            .text_size(px(15.))
                            .line_height(px(24.))
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
                                    .on_action(cx.listener(Self::text_copy))
                                    .on_action(cx.listener(Self::text_cut))
                                    .on_action(cx.listener(Self::text_paste))
                                    .on_action(cx.listener(Self::text_select_all))
                                    .role(Role::TextInput)
                                    .aria_label("Message composer")
                                    .aria_value(composer_value.clone())
                                    .aria_placeholder("Do anything…")
                                    .aria_description("Type a message, then press Enter to send.")
                                    .cursor(gpui::CursorStyle::IBeam)
                                    .on_click(cx.listener(|shell, _, window, cx| {
                                        shell.release_browser_focus();
                                        window.focus(&shell.composer_focus_handle, cx);
                                        shell.restart_composer_cursor_blink(cx);
                                        cx.stop_propagation();
                                    }))
                                    .on_key_down(cx.listener(Self::composer_key_down))
                                    .on_action(cx.listener(Self::composer_backspace))
                                    .on_action(cx.listener(Self::composer_delete))
                                    .on_action(cx.listener(Self::composer_left))
                                    .on_action(cx.listener(Self::composer_right))
                                    .on_action(cx.listener(Self::composer_select_left))
                                    .on_action(cx.listener(Self::composer_select_right))
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
                                    .id("composer-toolbar")
                                    .flex()
                                    .flex_wrap()
                                    .line_height(px(20.))
                                    .items_center()
                                    .justify_start()
                                    .w_full()
                                    .gap(px(8.))
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
                                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                                                        TablerIcon::Plus,
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
                                                    .id("permission-picker-trigger")
                                                    .debug_selector(|| "permission-picker-trigger".into())
                                                    .relative()
                                                    .role(Role::Button)
                                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                                            .debug_selector(|| "persona-picker-trigger".into())
                                            .min_w_0()
                                            .max_w(px(120.))
                                            .relative()
                                            .role(Role::Button)
                                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                                            .child(div().min_w_0().truncate().child(SharedString::from(persona_display)))
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
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme_rgb(
                                                &self.theme,
                                                "secondary_foreground",
                                            ))
                                            .cursor_pointer()
                                            .id("model-picker-trigger")
                                            .debug_selector(|| "model-picker-trigger".into())
                                            .ml_auto()
                                            .min_w_0()
                                            .relative()
                                            .role(Role::Button)
                                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                            .aria_label(SharedString::from(format!(
                                                "Choose model and effort: {model_display}"
                                            )))
                                            .px(px(6.))
                                            .py(px(4.))
                                            .rounded(px(6.))
                                            .hover(|element| {
                                                element.bg(theme_rgb(&self.theme, "muted"))
                                            })
                                            .on_click(cx.listener(|shell, _, window, cx| {
                                                shell.toggle_model_picker(window, cx);
                                            }))
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .max_w(px(220.))
                                            .child(tabler_icon(
                                                TablerIcon::SlidersHorizontal,
                                                theme_rgb(
                                                    &self.theme,
                                                    "secondary_foreground",
                                                ),
                                                px(13.),
                                            ))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(SharedString::from(model_display)),
                                            )
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
                                            ),
                                    )
                                    .child(if turn_is_running {
                                        div()
                                            .id("cancel-turn")
                                            .flex_none()
                                            .size(px(28.))
                                            .rounded_full()
                                            .bg(theme_rgb(&self.theme, "red.fg"))
                                            .text_color(theme_rgb(&self.theme, "background"))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .role(Role::Button)
                                            .desktop_focus(theme_rgb(&self.theme, "red.fg"))
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
                                            .debug_selector(|| "submit-prompt".into())
                                            .flex_none()
                                            .size(px(28.))
                                            .rounded_full()
                                            .bg(theme_rgb(&self.theme, "text.body"))
                                            .text_color(theme_rgb(&self.theme, "background"))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .role(Role::Button)
                                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                                    .flex_1()
                                    .child(tabler_icon(
                                        TablerIcon::Folder,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(13.),
                                    ))
                                    .child(div().id("checkout-path").debug_selector(|| "checkout-path".into())
                                        .min_w_0().truncate()
                                        .role(Role::Label)
                                        .aria_label(session_path.clone())
                                        .child(session_path)),
                            )
                            .child(
                                div()
                                    .id("session-status")
                                    .debug_selector(|| "session-status".into())
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
                                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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

        div()
            .id("center-workspace")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(conversation)
            .when(
                self.layout.workbench_collapsed
                    && self.work_tabs.active().kind == WorkTabKind::Terminal,
                |element| element.child(self.render_terminal_dock(cx)),
            )
    }

    pub(super) fn render_picker_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let window_width = window.bounds().size.width;
        let window_height = window.bounds().size.height;
        let model_option_count = self
            .model
            .ui
            .models
            .iter()
            .filter(|model| {
                model_matches_query(
                    &model.id,
                    &model.provider,
                    &model.model,
                    model.description.as_deref(),
                    &self.model_picker_query,
                )
            })
            .count();
        let model_effort_option_count = {
            let count = thinking_variants_for_model(
                &self.model.ui.models,
                self.model.ui.current_provider.as_deref(),
                self.model.ui.model.as_deref(),
            )
            .len();
            if count == 0 {
                0
            } else {
                count + 1
            }
        };
        let model_position = self.model_picker_open.then(|| {
            self.model_picker_bounds.map(|bounds| {
                picker_popup_position_in_window_right_aligned(
                    bounds,
                    px(300.),
                    model_picker_height(model_option_count, model_effort_option_count),
                    window_width,
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

fn tool_line_offsets(lines: &[SharedString]) -> Vec<usize> {
    let mut offset = 0;
    lines
        .iter()
        .map(|line| {
            let start = offset;
            offset += line.len() + 1;
            start
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        formatted_tool_input, split_tool_text_lines, tool_activity_summary, tool_display_label,
        tool_summary, TranscriptRenderRow,
    };
    use gpui::SharedString;

    #[test]
    fn activity_summaries_name_each_kind_once_in_order_of_use() {
        let parts: Vec<_> = [
            "read",
            "read_file",
            "bash",
            "edit",
            "browser_open",
            "glob",
            "integration.unknown",
        ]
        .into_iter()
        .map(|name| super::TranscriptPart::ToolCall {
            tool_name: name.into(),
            call_id: name.into(),
            status: super::ToolStatus::Completed,
            input: String::new(),
            output: None,
            error: None,
            diff: None,
        })
        .collect();
        assert_eq!(
            tool_activity_summary(parts.iter()),
            "Read files, ran commands, edited files, browsed pages, searched files, used tools"
        );
        assert_eq!(tool_activity_summary(parts[2..3].iter()), "Ran commands");
    }

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

    #[test]
    fn history_prepend_uses_only_the_new_message_rows() {
        let rows = vec![
            TranscriptRenderRow {
                message_index: 0,
                part_index: 0,
                block_index: 0,
                tool_group_len: 0,
            },
            TranscriptRenderRow {
                message_index: 1,
                part_index: 0,
                block_index: 0,
                tool_group_len: 0,
            },
            TranscriptRenderRow {
                message_index: 2,
                part_index: 0,
                block_index: 0,
                tool_group_len: 0,
            },
            TranscriptRenderRow {
                message_index: 2,
                part_index: 0,
                block_index: 1,
                tool_group_len: 0,
            },
            TranscriptRenderRow {
                message_index: 3,
                part_index: 0,
                block_index: 0,
                tool_group_len: 0,
            },
        ];

        assert_eq!(
            super::DesktopShell::transcript_prepend_row_count(&rows, 3, 5, 2),
            Some(2)
        );
        assert_eq!(
            super::DesktopShell::transcript_prepend_row_count(&rows, 3, 4, 2),
            None
        );
        assert_eq!(
            super::DesktopShell::transcript_prepend_row_count(&rows, 3, 5, 0),
            None
        );
    }
}
