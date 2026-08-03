use super::*;

fn activity_row_count(subagent_count: usize, todo_count: usize) -> usize {
    (if subagent_count > 0 {
        subagent_count + 1
    } else {
        0
    }) + if todo_count > 0 { todo_count + 1 } else { 0 }
}

impl DesktopShell {
    pub(super) fn render_remote_terminal_strip(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let terminal_status = self.terminal_status.clone();
        if self.layout.terminal_collapsed {
            return div()
                .id("terminal-strip-collapsed")
                .flex()
                .items_center()
                .h(px(34.))
                .px(px(14.))
                .cursor_pointer()
                .text_xs()
                .text_color(theme_rgb(&self.theme, "text.muted"))
                .gap(px(7.))
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.dispatch_shell_command(ShellCommand::ToggleTerminal, cx);
                }))
                .child(tabler_icon(
                    TablerIcon::Terminal2,
                    theme_rgb(&self.theme, "text.muted"),
                    px(14.),
                ))
                .child(SharedString::from(format!(
                    "Terminal 1 · {terminal_status}"
                )))
                .into_any_element();
        }

        div()
            .id("terminal-strip")
            .flex()
            .flex_col()
            .h(px(190.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(40.))
                    .px(px(12.))
                    .relative()
                    .text_sm()
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(7.))
                            .bg(theme_rgb(&self.theme, "card"))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(tabler_icon(
                                TablerIcon::Terminal2,
                                theme_rgb(&self.theme, "text.body"),
                                px(14.),
                            ))
                            .child("Terminal 1"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("terminal-font-picker-toggle")
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                            .on_click(cx.listener(|shell, _, _, cx| {
                                shell.toggle_terminal_font_picker(cx);
                            }))
                            .child(SharedString::from(format!(
                                "font · {}",
                                self.terminal_font_family
                            )))
                            .into_any_element(),
                    )
                    .child(
                        div()
                            .id("terminal-collapse")
                            .px(px(7.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .on_click(cx.listener(|shell, _, _, cx| {
                                shell.dispatch_shell_command(ShellCommand::ToggleTerminal, cx);
                            }))
                            .child(tabler_icon(
                                TablerIcon::ChevronDown,
                                theme_rgb(&self.theme, "text.muted"),
                                px(14.),
                            )),
                    )
                    .when(self.terminal_font_picker_open, |element| {
                        element
                            .child(deferred(self.render_terminal_font_picker(cx)).with_priority(3))
                    }),
            )
            .child(
                div()
                    .id("terminal-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .p(px(10.))
                    .child(self.terminal_view.clone()),
            )
            .into_any_element()
    }

    pub(super) fn render_review_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let loaded = !self.review_diffs.is_empty();
        range
            .filter_map(|index| {
                let (file, status) = if let Some(diff) = self.review_diffs.get(index) {
                    (diff.path.clone(), diff.status)
                } else {
                    (
                        self.model.ui.review.files.get(index)?.clone(),
                        FileStatus::Modified,
                    )
                };
                let selected = self.review_selected_file == Some(index);
                Some(
                    div()
                        .id(format!("review-file-{index}"))
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(46.))
                        .p(px(10.))
                        .border_b_1()
                        .border_color(theme_rgb(&self.theme, "divider"))
                        .when(selected, |element| {
                            element.bg(theme_rgb(&self.theme, "card"))
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.select_review_file(index, cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_sm()
                                .child(tabler_icon(
                                    TablerIcon::FileCode,
                                    theme_rgb(&self.theme, "text.muted"),
                                    px(14.),
                                ))
                                .child(SharedString::from(file)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(if loaded {
                                    match status {
                                        FileStatus::Added => theme_rgb(&self.theme, "green.fg"),
                                        FileStatus::Deleted => theme_rgb(&self.theme, "red.fg"),
                                        FileStatus::Binary => {
                                            theme_rgb(&self.theme, "text.warning")
                                        }
                                        _ => theme_rgb(&self.theme, "text.muted"),
                                    }
                                } else {
                                    theme_rgb(&self.theme, "text.muted")
                                })
                                .child(file_status_label(status)),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }

    pub(super) fn render_diff_line_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        range
            .filter_map(|index| {
                let line = self.review_lines.get(index)?;
                let (marker, color) = match line.kind {
                    mew_diff::LineKind::Context => (" ", theme_rgb(&self.theme, "text.muted")),
                    mew_diff::LineKind::Addition => ("+", theme_rgb(&self.theme, "green.fg")),
                    mew_diff::LineKind::Deletion => ("−", theme_rgb(&self.theme, "red.fg")),
                };
                Some(
                    div()
                        .id(format!("diff-line-{index}"))
                        .flex()
                        .items_start()
                        .w_full()
                        .px(px(8.))
                        .text_xs()
                        .text_color(color)
                        .child(
                            div()
                                .w(px(28.))
                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                .child(SharedString::from(
                                    line.old_line
                                        .map(|line| line.to_string())
                                        .unwrap_or_default(),
                                )),
                        )
                        .child(
                            div()
                                .w(px(28.))
                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                .child(SharedString::from(
                                    line.new_line
                                        .map(|line| line.to_string())
                                        .unwrap_or_default(),
                                )),
                        )
                        .child(div().w(px(14.)).child(marker))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(SharedString::from(line.text.clone())),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }

    pub(super) fn render_diff_preview(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(index) = self.review_selected_file else {
            return div().into_any_element();
        };
        let Some(diff) = self.review_diffs.get(index) else {
            return div().into_any_element();
        };
        let path = diff.path.clone();
        let status = diff.status;
        if self.review_lines.is_empty() {
            return div()
                .id("diff-preview")
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(10.))
                .border_t_1()
                .border_color(theme_rgb(&self.theme, "divider"))
                .child(div().text_xs().child(SharedString::from(path)))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(match status {
                            FileStatus::Binary => "binary file",
                            FileStatus::Unchanged => "no textual changes",
                            _ => "no diff lines",
                        }),
                )
                .into_any_element();
        }
        gpui::uniform_list(
            "diff-preview-lines",
            self.review_lines.len(),
            cx.processor(Self::render_diff_line_rows),
        )
        .id("diff-preview")
        .h(px(240.))
        .min_h_0()
        .border_t_1()
        .border_color(theme_rgb(&self.theme, "divider"))
        .into_any_element()
    }

    pub(super) fn render_browser_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let shell = cx.entity();
        let browser_error = self.browser_error.clone();
        div()
            .id("native-browser-panel")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .border_b_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .child(
                div()
                    .id("native-browser-toolbar")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(38.))
                    .px(px(10.))
                    .text_xs()
                    .text_color(theme_rgb(&self.theme, "text.muted"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(28.))
                            .px(px(8.))
                            .border_1()
                            .border_color(theme_rgb(&self.theme, "divider"))
                            .rounded(px(7.))
                            .bg(theme_rgb(&self.theme, "input"))
                            .text_color(theme_rgb(&self.theme, "text.body"))
                            .id("browser-url-field")
                            .track_focus(&self.browser_url_focus_handle)
                            .key_context("BrowserUrl")
                            .role(Role::TextInput)
                            .aria_label("Browser URL")
                            .cursor(gpui::CursorStyle::IBeam)
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|shell, _, window, cx| {
                                    shell.browser_url_mouse_down(window, cx);
                                }),
                            )
                            .on_click(cx.listener(|shell, _, window, cx| {
                                shell.browser_url_mouse_down(window, cx);
                            }))
                            .on_key_down(cx.listener(Self::browser_url_key_down))
                            .child(ComposerElement {
                                shell: cx.entity(),
                                target: TextInputTarget::BrowserUrl,
                            }),
                    )
                    .child(
                        div()
                            .id("navigate-native-browser")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(26.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .role(Role::Button)
                            .aria_label("Navigate browser")
                            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                            .on_click(cx.listener(|shell, _, _, cx| {
                                shell.navigate_browser_to(cx);
                            }))
                            .child(tabler_icon(
                                TablerIcon::ChevronRight,
                                theme_rgb(&self.theme, "text.muted"),
                                px(14.),
                            )),
                    )
                    .child(
                        div()
                            .id("close-native-browser")
                            .cursor_pointer()
                            .role(Role::Button)
                            .aria_label("Close browser")
                            .text_lg()
                            .on_click(cx.listener(|shell, _, _, cx| {
                                shell.close_browser_panel(cx);
                            }))
                            .child(tabler_icon(
                                TablerIcon::X,
                                theme_rgb(&self.theme, "text.muted"),
                                px(13.),
                            )),
                    ),
            )
            .child(
                div()
                    .id("native-browser-viewport")
                    .flex_1()
                    .min_h_0()
                    .bg(theme_rgb(&self.theme, "background"))
                    .child(
                        canvas(
                            move |bounds, window, app| {
                                shell.update(app, |shell, _cx| {
                                    shell.update_browser_bounds(bounds, window);
                                });
                            },
                            |_bounds, _state, _window, _app| {},
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    )
                    .when_some(browser_error, |element, _error| {
                        element.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(8.))
                                .min_w(px(220.))
                                .p(px(16.))
                                .bg(theme_rgb(&self.theme, "background"))
                                .child(div().text_sm().child("Browser couldn't start"))
                                .child(
                                    div()
                                        .max_w(px(260.))
                                        .text_center()
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.muted"))
                                        .child("The in-app browser is unavailable right now. Try again or continue in the conversation."),
                                )
                                .child(
                                    div()
                                        .id("retry-browser")
                                        .px(px(10.))
                                        .py(px(6.))
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .role(Role::Button)
                                        .aria_label("Retry browser")
                                        .bg(theme_rgb(&self.theme, "card"))
                                        .hover(|element| {
                                            element.bg(theme_rgb(&self.theme, "muted"))
                                        })
                                        .on_click(cx.listener(|shell, _, _, cx| {
                                            shell.retry_browser(cx);
                                        }))
                                        .text_xs()
                                        .child("try again"),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_activity_body(
        &mut self,
        empty_detail: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        let subagent_count = self.model.ui.subagents.len();
        let todo_count = self.model.ui.todos.len();
        if subagent_count == 0 && todo_count == 0 {
            return div()
                .px(px(19.))
                .pb(px(12.))
                .text_xs()
                .text_color(muted)
                .child(empty_detail)
                .into_any_element();
        }
        div()
            .id("activity-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                gpui::uniform_list(
                    "activity-list",
                    activity_row_count(subagent_count, todo_count),
                    cx.processor(Self::render_activity_rows),
                )
                .flex_1()
                .min_h_0()
                .px(px(14.))
                .pb(px(12.)),
            )
            .into_any_element()
    }

    fn render_activity_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let subagent_count = self.model.ui.subagents.len();
        let todo_count = self.model.ui.todos.len();
        let subagent_header = subagent_count > 0;
        let todo_header = todo_count > 0;
        let todo_start = (if subagent_header { 1 } else { 0 }) + subagent_count;
        let todo_items_start = todo_start + if todo_header { 1 } else { 0 };
        let todo_done = self
            .model
            .ui
            .todos
            .iter()
            .filter(|todo| todo.status == ActivityTodoStatus::Done)
            .count();
        let muted = theme_rgb(&self.theme, "text.muted");
        let body = theme_rgb(&self.theme, "text.body");
        let running = theme_rgb(&self.theme, "yellow.fg");
        let blocked = theme_rgb(&self.theme, "red.fg");

        range
            .filter_map(|index| {
                let row = if subagent_header && index == 0 {
                    div()
                        .id("activity-subagents-heading")
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .text_xs()
                        .text_color(muted)
                        .child("Subagents")
                        .into_any_element()
                } else if subagent_header && index < todo_start {
                    let subagent_index = index - 1;
                    let entry = self.model.ui.subagents.get(subagent_index)?;
                    let name = entry
                        .display_name
                        .clone()
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| {
                            if entry.name.is_empty() {
                                "subagent".to_owned()
                            } else {
                                entry.name.clone()
                            }
                        });
                    let status = if entry.status.is_empty() {
                        "running".to_owned()
                    } else {
                        entry.status.clone()
                    };
                    div()
                        .id(format!("activity-subagent-{subagent_index}"))
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().size(px(6.)).rounded_full().flex_none().bg(running))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(body)
                                .child(SharedString::from(name)),
                        )
                        .child(
                            div()
                                .max_w(px(110.))
                                .truncate()
                                .text_xs()
                                .text_color(muted)
                                .child(SharedString::from(status)),
                        )
                        .into_any_element()
                } else if todo_header && index == todo_start {
                    div()
                        .id("activity-todos-heading")
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("Todos ({todo_done}/{todo_count})"))
                        .into_any_element()
                } else if todo_header && index >= todo_items_start {
                    let todo_index = index - todo_items_start;
                    let todo = self.model.ui.todos.get(todo_index)?;
                    let (marker, marker_color) = match todo.status {
                        ActivityTodoStatus::Done => ("✓", muted),
                        ActivityTodoStatus::InProgress => ("▸", running),
                        ActivityTodoStatus::Pending => ("○", muted),
                        ActivityTodoStatus::Blocked => ("!", blocked),
                        ActivityTodoStatus::Unknown => ("•", muted),
                    };
                    let text_color = if todo.status == ActivityTodoStatus::Done {
                        muted
                    } else {
                        body
                    };
                    div()
                        .id(format!("activity-todo-{todo_index}"))
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(px(12.))
                                .flex_none()
                                .text_xs()
                                .text_color(marker_color)
                                .child(marker),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(text_color)
                                .child(SharedString::from(todo.content.clone())),
                        )
                        .into_any_element()
                } else {
                    return None;
                };
                Some(row)
            })
            .collect()
    }

    pub(super) fn render_file_tree(
        &mut self,
        has_workspace: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        if !has_workspace {
            return div()
                .px(px(19.))
                .pb(px(12.))
                .text_xs()
                .text_color(muted)
                .child("This session has no working directory.")
                .into_any_element();
        }
        let row_count = file_tree_row_count(&self.file_tree_entries, &self.file_tree_expanded);
        if row_count == 0 {
            return div()
                .px(px(19.))
                .pb(px(12.))
                .text_xs()
                .text_color(muted)
                .child("Loading workspace files…")
                .into_any_element();
        }
        div()
            .id("file-tree")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                gpui::uniform_list(
                    "file-tree-list",
                    row_count,
                    cx.processor(Self::render_file_tree_rows),
                )
                .flex_1()
                .min_h_0()
                .px(px(10.))
                .pb(px(12.)),
            )
            .into_any_element()
    }

    fn render_file_tree_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let muted = theme_rgb(&self.theme, "text.muted");
        file_tree_rows_in_range(
            &self.file_tree_entries,
            &self.file_tree_expanded,
            range.clone(),
        )
        .into_iter()
        .enumerate()
        .map(|(offset, row)| {
            let index = range.start + offset;
            let row_path = row.path.clone();
            let indent = px(6. + row.depth as f32 * 14.);
            let chevron = if row.is_dir {
                if row.expanded {
                    TablerIcon::ChevronDown
                } else {
                    TablerIcon::ChevronRight
                }
            } else {
                TablerIcon::ChevronRight
            };
            let entry_icon = if row.is_dir {
                TablerIcon::Folder
            } else {
                TablerIcon::File
            };
            let label = format!(
                "{}{}",
                row.name,
                if row.is_dir {
                    if row.expanded {
                        ", expanded"
                    } else {
                        ", collapsed"
                    }
                } else {
                    ""
                }
            );
            div()
                .id(format!("file-tree-row-{index}"))
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(24.))
                .rounded(px(5.))
                .pl(indent)
                .pr(px(6.))
                .cursor_pointer()
                .role(Role::Button)
                .aria_label(SharedString::from(label))
                .text_xs()
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    if row.is_dir {
                        shell.toggle_file_tree_dir(row_path.clone(), cx);
                    } else {
                        shell.open_workspace_path(row_path.clone(), cx);
                    }
                }))
                .child(div().w(px(11.)).flex_none().when(row.is_dir, |element| {
                    element.child(tabler_icon(chevron, muted, px(11.)))
                }))
                .child(tabler_icon(entry_icon, muted, px(13.)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(theme_rgb(&self.theme, "text.body"))
                        .child(SharedString::from(row.name)),
                )
                .into_any_element()
        })
        .collect()
    }

    pub(super) fn render_workbench_divider(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .id("workbench-resizer")
            .w(px(6.))
            .h_full()
            .cursor(gpui::CursorStyle::ResizeColumn)
            .flex()
            .items_center()
            .justify_center()
            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
            .child(
                div()
                    .w(px(2.))
                    .h(px(36.))
                    .rounded_full()
                    .bg(theme_rgb(&self.theme, "divider")),
            )
            .on_drag(WorkbenchResizeDrag, |_, _, _, cx| {
                cx.new(|_| WorkbenchResizePreview)
            })
            .on_drag_move::<WorkbenchResizeDrag>(cx.listener(
                |shell, event: &gpui::DragMoveEvent<WorkbenchResizeDrag>, window, cx| {
                    let available_width = f32::from(window.bounds().size.width);
                    let pointer_x = f32::from(event.event.position.x);
                    let sidebar_width = if shell.layout.sidebar_collapsed {
                        SIDEBAR_COLLAPSED_WIDTH
                    } else {
                        SIDEBAR_EXPANDED_WIDTH
                    };
                    let width =
                        workbench_width_from_pointer(available_width, pointer_x, sidebar_width);
                    if (shell.workbench_width - width).abs() > 0.5 {
                        shell.workbench_width = width;
                        shell.capture_session_view_state();
                        cx.notify();
                    }
                },
            ))
            .into_any_element()
    }

    pub(super) fn render_workbench(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let sidebar_width = if self.layout.sidebar_collapsed {
            SIDEBAR_COLLAPSED_WIDTH
        } else {
            SIDEBAR_EXPANDED_WIDTH
        };
        let expanded_workbench_width = self.workbench_width.min(workbench_max_width(
            f32::from(window.bounds().size.width),
            sidebar_width,
        ));
        let review = self.model.ui.review.clone();
        let file_count = if self.review_diffs.is_empty() {
            review.files.len()
        } else {
            self.review_diffs.len()
        };
        let added = if self.review_diffs.is_empty() {
            review.added
        } else {
            self.review_diffs.iter().map(|diff| diff.added as u64).sum()
        };
        let removed = if self.review_diffs.is_empty() {
            review.removed
        } else {
            self.review_diffs
                .iter()
                .map(|diff| diff.removed as u64)
                .sum()
        };
        let changes_expanded = self.layout.changes_expanded;
        let review_message = if self.review_loading {
            "Loading workspace changes…"
        } else if self.review_error.is_some() {
            "Could not load workspace changes"
        } else {
            "No workspace changes yet"
        };
        let review_detail = self
            .review_error
            .clone()
            .unwrap_or_else(|| "Changed files will appear here during a run.".into());
        let change_label = if file_count == 0 {
            "Changes".to_owned()
        } else {
            format!("Changes · {file_count} files")
        };
        let active_view = self.auxiliary_view;
        let session_path = selected_session_path(
            &self.model.ui.conversations,
            self.model.ui.selected_session.as_deref(),
        );
        let subagent_count = self.model.ui.subagents.len();
        let todo_total = self.model.ui.todos.len();
        let todo_done = self
            .model
            .ui
            .todos
            .iter()
            .filter(|todo| todo.status == ActivityTodoStatus::Done)
            .count();
        let activity_label = if !self.model.ui.pending_actions.is_empty() {
            "Needs input".to_owned()
        } else if subagent_count > 0 {
            format!(
                "{subagent_count} subagent{}",
                if subagent_count == 1 { "" } else { "s" }
            )
        } else if todo_total > 0 {
            format!("{todo_done}/{todo_total} todos")
        } else if self.model.ui.running {
            "Working".to_owned()
        } else {
            "No active tasks".to_owned()
        };
        let activity_detail = if !self.model.ui.pending_actions.is_empty() {
            "The session is waiting for a response."
        } else if self.model.ui.running {
            "The session is processing a turn."
        } else {
            "Activity will appear while a turn is running."
        };
        let auxiliary_tabs = [
            (AuxiliaryView::Browser, TablerIcon::Browser, "Browser"),
            (AuxiliaryView::Changes, TablerIcon::FileCode, "Changes"),
            (AuxiliaryView::Local, TablerIcon::Folder, "Local"),
            (AuxiliaryView::Activity, TablerIcon::GitBranch, "Activity"),
        ]
        .into_iter()
        .map(|(view, icon, label)| {
            let selected = active_view == view;
            div()
                .id(format!("auxiliary-view-{}", label.to_lowercase()))
                .flex()
                .flex_none()
                .w_full()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .h(px(30.))
                .rounded(px(6.))
                .cursor_pointer()
                .role(Role::Tab)
                .aria_selected(selected)
                .aria_label(SharedString::from(format!(
                    "{} auxiliary panel{}",
                    label,
                    if selected { ", selected" } else { "" }
                )))
                .text_xs()
                .when(selected, |element| {
                    element.bg(theme_rgb(&self.theme, "card"))
                })
                .text_color(if selected {
                    theme_rgb(&self.theme, "text.accent")
                } else {
                    theme_rgb(&self.theme, "text.muted")
                })
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.select_auxiliary_view(view, cx);
                }))
                .child(tabler_icon(
                    icon,
                    if selected {
                        theme_rgb(&self.theme, "text.accent")
                    } else {
                        theme_rgb(&self.theme, "text.muted")
                    },
                    px(13.),
                ))
                .into_any_element()
        })
        .collect::<Vec<_>>();

        let workbench = if self.layout.workbench_collapsed {
            div()
                .id("review-workbench-collapsed")
                .flex()
                .flex_col()
                .items_center()
                .w(px(56.))
                .h_full()
                .gap(px(10.))
                .p(px(10.))
                .child(
                    div()
                        .id("expand-workbench")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(34.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .role(Role::Button)
                        .aria_label("Show workbench")
                        .text_lg()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.dispatch_shell_command(ShellCommand::ToggleWorkbench, cx);
                        }))
                        .child(tabler_icon(
                            TablerIcon::PanelRight,
                            theme_rgb(&self.theme, "text.muted"),
                            px(15.),
                        )),
                )
                .child(div().size(px(8.)).rounded_full().bg(if file_count == 0 {
                    theme_rgb(&self.theme, "divider")
                } else {
                    theme_rgb(&self.theme, "green.fg")
                }))
                .into_any_element()
        } else {
            div()
                .id("review-workbench")
                .flex()
                .flex_col()
                .w(px(expanded_workbench_width))
                .h_full()
                .min_h_0()
                .relative()
                .pl(px(44.))
                .child(
                    div()
                        .id("auxiliary-view-tabs")
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(3.))
                        .w(px(44.))
                        .p(px(5.))
                        .border_r_1()
                        .border_color(theme_rgb(&self.theme, "divider"))
                        .children(auxiliary_tabs),
                )
                .when(active_view == AuxiliaryView::Browser, |element| {
                    element
                        .child(
                            div()
                                .id("browser-launch")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .h(px(38.))
                                .px(px(7.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_label("Open browser")
                                .text_sm()
                                .on_click(cx.listener(|shell, _, _, cx| {
                                    shell.open_browser_panel(cx);
                                }))
                                .child(tabler_icon(
                                    TablerIcon::Browser,
                                    theme_rgb(&self.theme, "text.muted"),
                                    px(15.),
                                ))
                                .child(div().flex_1())
                                .child(if self.browser_panel_open {
                                    "open"
                                } else {
                                    "open page"
                                }),
                        )
                        .when(self.browser_panel_open, |element| {
                            element.child(self.render_browser_panel(window, cx))
                        })
                })
                .when(active_view == AuxiliaryView::Changes, |element| {
                    element
                        .child(
                            div()
                                .id("changes-disclosure")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .h(px(44.))
                                .px(px(7.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_expanded(self.layout.changes_expanded)
                                .aria_label(SharedString::from(change_label.clone()))
                                .border_b_1()
                                .border_color(theme_rgb(&self.theme, "divider"))
                                .text_sm()
                                .on_click(cx.listener(|shell, _, _, cx| shell.toggle_changes(cx)))
                                .child(if self.layout.changes_expanded {
                                    tabler_icon(
                                        TablerIcon::ChevronDown,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )
                                } else {
                                    tabler_icon(
                                        TablerIcon::ChevronRight,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )
                                })
                                .child(SharedString::from(change_label))
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_color(theme_rgb(&self.theme, "green.fg"))
                                        .child(format!("+{added}")),
                                )
                                .child(
                                    div()
                                        .text_color(theme_rgb(&self.theme, "red.fg"))
                                        .child(format!("−{removed}")),
                                ),
                        )
                        .when(changes_expanded, |element| {
                            element.child(if file_count == 0 {
                                div()
                                    .id("review-file-list-empty")
                                    .flex_1()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(8.))
                                    .child(div().text_sm().child(review_message))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme_rgb(&self.theme, "text.muted"))
                                            .child(SharedString::from(review_detail)),
                                    )
                                    .into_any_element()
                            } else {
                                gpui::uniform_list(
                                    "review-file-list",
                                    file_count,
                                    cx.processor(Self::render_review_rows),
                                )
                                .flex_1()
                                .min_h_0()
                                .into_any_element()
                            })
                        })
                        .child(self.render_diff_preview(cx))
                })
                .when(active_view == AuxiliaryView::Local, |element| {
                    element
                        .child(
                            div()
                                .id("local-disclosure")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .h(px(42.))
                                .px(px(14.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_expanded(self.layout.local_expanded)
                                .aria_label(if self.layout.local_expanded {
                                    "Collapse local checkout"
                                } else {
                                    "Expand local checkout"
                                })
                                .border_t_1()
                                .border_color(theme_rgb(&self.theme, "divider"))
                                .text_sm()
                                .on_click(cx.listener(|shell, _, _, cx| shell.toggle_local(cx)))
                                .child(if self.layout.local_expanded {
                                    tabler_icon(
                                        TablerIcon::ChevronDown,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )
                                } else {
                                    tabler_icon(
                                        TablerIcon::ChevronRight,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )
                                })
                                .child(tabler_icon(
                                    TablerIcon::Folder,
                                    theme_rgb(&self.theme, "text.muted"),
                                    px(14.),
                                ))
                                .child("Workspace")
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .max_w(px(150.))
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.muted"))
                                        .child(SharedString::from(
                                            session_path
                                                .as_deref()
                                                .unwrap_or("path not set")
                                                .to_owned(),
                                        )),
                                ),
                        )
                        .when(self.layout.local_expanded, |element| {
                            element.child(self.render_file_tree(session_path.is_some(), cx))
                        })
                })
                .when(active_view == AuxiliaryView::Activity, |element| {
                    element
                        .child(
                            div()
                                .id("activity-disclosure")
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .h(px(42.))
                                .px(px(7.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_expanded(self.layout.activity_expanded)
                                .aria_label(if self.layout.activity_expanded {
                                    "Collapse activity"
                                } else {
                                    "Expand activity"
                                })
                                .border_t_1()
                                .border_color(theme_rgb(&self.theme, "divider"))
                                .text_sm()
                                .on_click(cx.listener(|shell, _, _, cx| shell.toggle_activity(cx)))
                                .child(if self.layout.activity_expanded {
                                    tabler_icon(
                                        TablerIcon::ChevronDown,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )
                                } else {
                                    tabler_icon(
                                        TablerIcon::ChevronRight,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )
                                })
                                .child(tabler_icon(
                                    TablerIcon::GitBranch,
                                    theme_rgb(&self.theme, "text.muted"),
                                    px(14.),
                                ))
                                .child("Activity")
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.muted"))
                                        .child(activity_label),
                                ),
                        )
                        .when(self.layout.activity_expanded, |element| {
                            element.child(self.render_activity_body(activity_detail, cx))
                        })
                })
                .into_any_element()
        };

        let workbench = div()
            .id("workbench-transition-wrapper")
            .flex()
            .flex_none()
            .relative()
            .h_full()
            .overflow_hidden()
            .rounded(px(SHELL_SURFACE_RADIUS))
            .border_l_1()
            .border_r_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .child(workbench);
        let workbench_width = if self.layout.workbench_collapsed {
            WORKBENCH_COLLAPSED_WIDTH
        } else {
            expanded_workbench_width
        };
        if self.workbench_animation_id == 0 {
            return workbench.w(px(workbench_width)).into_any_element();
        }
        let collapsed = self.layout.workbench_collapsed;
        let animation_id = self.workbench_animation_id;
        let expanded_width = workbench_width;
        workbench
            .with_animation(
                ElementId::Name(format!("workbench-transition-{animation_id}").into()),
                Animation::new(Duration::from_millis(220)).with_easing(gpui::ease_out_quint()),
                move |element, delta| {
                    element
                        .w(px(workbench_transition_width(collapsed, delta)))
                        .right(px(workbench_transition_offset(
                            collapsed,
                            delta,
                            expanded_width,
                        )))
                },
            )
            .into_any_element()
    }
}
