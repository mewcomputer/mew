use super::*;

impl DesktopShell {
    pub(super) fn render_session_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        range
            .filter_map(|index| match self.sidebar_rows.get(index)?.clone() {
                SidebarRow::Toolbar => Some(
                    div()
                        .id("sidebar-sessions-toolbar")
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(32.))
                        .px(px(4.))
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child("SESSIONS")
                        .child(
                            div().flex().items_center().gap(px(2.)).children([
                                div()
                                    .id("new-session")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(24.))
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .role(Role::Button)
                                    .aria_label("New conversation")
                                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                    .on_click(cx.listener(|shell, _, _, cx| {
                                        shell.new_conversation(cx);
                                    }))
                                    .child(tabler_icon(
                                        TablerIcon::Plus,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )),
                                div()
                                    .id("new-group")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(24.))
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .role(Role::Button)
                                    .aria_label("New session group")
                                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                    .on_click(cx.listener(|shell, _, _, cx| {
                                        shell.create_group(cx);
                                    }))
                                    .child(tabler_icon(
                                        TablerIcon::Folder,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    )),
                            ]),
                        )
                        .into_any_element(),
                ),
                SidebarRow::Group {
                    id,
                    name,
                    color,
                    count,
                    collapsed,
                } => {
                    let group_id = id.clone();
                    let drop_group_id = id.clone();
                    let new_session_group_id = id.clone();
                    let drag_over = self.drag_over_group.as_deref() == Some(id.as_str());
                    let group_hovered = self.hovered_group.as_deref() == Some(id.as_str());
                    let delete_group_id = id.clone();
                    let delete_group_name = name.clone();
                    let is_pseudo_group = id == UNGROUPED_GROUP_ID || id == ARCHIVED_GROUP_ID;
                    let delete_control = (!is_pseudo_group && group_hovered).then(|| {
                        div()
                            .id(format!("delete-group-{delete_group_id}"))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(20.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .role(Role::Button)
                            .aria_label(SharedString::from(format!(
                                "Delete group {delete_group_name}"
                            )))
                            .hover(|element| element.bg(theme_rgb(&self.theme, "divider")))
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                cx.stop_propagation();
                                shell.delete_group(delete_group_id.clone(), cx);
                            }))
                            .child(tabler_icon(
                                TablerIcon::X,
                                theme_rgb(&self.theme, "text.muted"),
                                px(12.),
                            ))
                    });
                    let new_session_control = div()
                        .id(format!("new-session-in-group-{new_session_group_id}"))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .role(Role::Button)
                        .aria_label(SharedString::from(format!("New conversation in {name}")))
                        .hover(|element| element.bg(theme_rgb(&self.theme, "divider")))
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            cx.stop_propagation();
                            if new_session_group_id == UNGROUPED_GROUP_ID {
                                shell.new_conversation(cx);
                            } else {
                                shell.new_conversation_in_group(new_session_group_id.clone(), cx);
                            }
                        }))
                        .child(tabler_icon(
                            TablerIcon::Plus,
                            theme_rgb(&self.theme, "text.muted"),
                            px(12.),
                        ));
                    let enter_group_id = id.clone();
                    let exit_group_id = id.clone();
                    Some(
                        div()
                            .id(format!("sidebar-group-{id}"))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .h(px(30.))
                            .px(px(4.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .role(Role::Button)
                            .aria_expanded(!collapsed)
                            .aria_label(SharedString::from(format!(
                                "{} group, {} sessions, {}",
                                name,
                                count,
                                if collapsed { "collapsed" } else { "expanded" }
                            )))
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .when(drag_over, |element| {
                                element
                                    .bg(theme_rgb(&self.theme, "accent").opacity(0.14))
                                    .border_1()
                                    .border_color(theme_rgb(&self.theme, "accent").opacity(0.5))
                            })
                            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.toggle_group(group_id.clone(), cx);
                            }))
                            .on_mouse_move(cx.listener(move |shell, _, _, cx| {
                                if shell.hovered_group.as_deref() != Some(enter_group_id.as_str()) {
                                    shell.hovered_group = Some(enter_group_id.clone());
                                    cx.notify();
                                }
                            }))
                            .on_mouse_exit(cx.listener(move |shell, _, _, cx| {
                                if shell.hovered_group.as_deref() == Some(exit_group_id.as_str()) {
                                    shell.hovered_group = None;
                                    cx.notify();
                                }
                            }))
                            .on_drag_move::<SessionDrag>(cx.listener(
                                move |shell, event: &gpui::DragMoveEvent<SessionDrag>, _, cx| {
                                    if event.drag(cx).session_id.is_empty() {
                                        return;
                                    }
                                    if shell.drag_over_group.as_deref()
                                        != Some(drop_group_id.as_str())
                                    {
                                        shell.drag_over_group = Some(drop_group_id.clone());
                                        cx.notify();
                                    }
                                },
                            ))
                            .on_mouse_exit(cx.listener({
                                let group_id = id.clone();
                                move |shell, _, _, cx| {
                                    if shell.drag_over_group.as_deref() == Some(group_id.as_str()) {
                                        shell.drag_over_group = None;
                                        cx.notify();
                                    }
                                }
                            }))
                            .on_drop(cx.listener({
                                let group_id = id.clone();
                                move |shell, drag: &SessionDrag, _, cx| {
                                    if group_id == ARCHIVED_GROUP_ID {
                                        return;
                                    }
                                    let target =
                                        (group_id != UNGROUPED_GROUP_ID).then(|| group_id.clone());
                                    shell.assign_session_group(drag.session_id.clone(), target, cx);
                                }
                            }))
                            .child(tabler_icon(
                                if collapsed {
                                    TablerIcon::ChevronRight
                                } else {
                                    TablerIcon::ChevronDown
                                },
                                theme_rgb(&self.theme, "text.muted"),
                                px(13.),
                            ))
                            .child(
                                div().size(px(7.)).rounded_full().bg(color
                                    .as_deref()
                                    .map(|_| theme_rgb(&self.theme, "accent"))
                                    .unwrap_or_else(|| theme_rgb(&self.theme, "divider"))),
                            )
                            .child(div().flex_1().child(SharedString::from(name.clone())))
                            .child(div().text_xs().child(count.to_string()))
                            .when(group_hovered && id != ARCHIVED_GROUP_ID, |element| {
                                element.child(new_session_control)
                            })
                            .when_some(delete_control, |element, control| element.child(control))
                            .into_any_element(),
                    )
                }
                SidebarRow::Session(conversation) => {
                    let session_id = conversation.session_id.clone();
                    let selected = self.model.ui.selected_session.as_deref() == Some(&session_id);
                    let title = SharedString::from(compact_session_title(&conversation.title));
                    let path = conversation
                        .cwd
                        .as_deref()
                        .filter(|path| !path.is_empty())
                        .map(display_session_path)
                        .map(SharedString::from);
                    let path_label = path.clone();
                    let session_time =
                        session_time_label(conversation.last_message_at).map(SharedString::from);
                    let has_meta = path.is_some() || session_time.is_some();
                    let row_height = if has_meta { 48. } else { 36. };
                    let status_color = if conversation.needs_attention {
                        theme_rgb(&self.theme, "red.fg")
                    } else if selected {
                        theme_rgb(&self.theme, "text.body")
                    } else {
                        theme_rgb(&self.theme, "text.muted")
                    };
                    let menu_open = self.session_menu_session.as_deref() == Some(&session_id);
                    let renaming = self.rename_session_id.as_deref() == Some(&session_id);
                    let pinned = conversation.pinned;
                    let archived = conversation.archived;
                    let groups = self.model.ui.groups.clone();
                    let current_group_id = conversation.group_id.clone();
                    let menu_session_id = session_id.clone();
                    let session_for_menu = session_id.clone();
                    let hovered = self.hovered_session.as_deref() == Some(&session_id);
                    let move_control = div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .h(px(28.))
                        .w(px(48.))
                        .flex()
                        .items_center()
                        .justify_end()
                        .pr(px(4.))
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(
                                theme_rgb(&self.theme, "sidebar.background").opacity(0.),
                                0.,
                            ),
                            linear_color_stop(
                                theme_rgb(&self.theme, "sidebar.background").opacity(0.96),
                                1.,
                            ),
                        ))
                        .child(
                            div()
                                .id(format!("session-menu-trigger-{menu_session_id}"))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(22.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_label(SharedString::from(format!(
                                    "Conversation options: {}",
                                    title
                                )))
                                .text_color(theme_rgb(&self.theme, "text.body"))
                                .hover(|element| element.bg(theme_rgb(&self.theme, "divider")))
                                .on_click(cx.listener(move |shell, _, _, cx| {
                                    cx.stop_propagation();
                                    shell.toggle_session_menu(menu_session_id.clone(), cx);
                                }))
                                .child(tabler_icon(
                                    TablerIcon::Dots,
                                    theme_rgb(&self.theme, "text.body"),
                                    px(13.),
                                )),
                        );
                    let enter_session_id = session_id.clone();
                    let exit_session_id = session_id.clone();
                    let rename_input_id = session_id.clone();
                    let rename_focus_handle = self.rename_focus_handle.clone();
                    let row = div()
                        .id(format!("session-{session_id}"))
                        .flex()
                        .flex_col()
                        .w_full()
                        .justify_center()
                        .gap(px(2.))
                        .h(px(row_height))
                        .pl(px(20.))
                        .pr(px(8.))
                        .rounded(px(6.))
                        .relative()
                        .cursor_pointer()
                        .role(Role::Button)
                        .aria_label(SharedString::from(format!(
                            "Conversation: {}{}",
                            title,
                            path_label
                                .as_deref()
                                .map(|path| format!(" · {path}"))
                                .unwrap_or_default()
                        )))
                        .when(selected, |element| {
                            element.bg(theme_rgb(&self.theme, "accent"))
                        })
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_drag(
                            SessionDrag::new(
                                session_id.clone(),
                                title.clone(),
                                theme_rgb(&self.theme, "card"),
                                theme_rgb(&self.theme, "text.body"),
                            ),
                            |drag: &SessionDrag, position, _, cx| {
                                cx.new(|_| drag.clone().positioned(position))
                            },
                        )
                        .on_mouse_move(cx.listener(move |shell, _, _, cx| {
                            if shell.hovered_session.as_deref() != Some(&enter_session_id) {
                                shell.hovered_session = Some(enter_session_id.clone());
                                cx.notify();
                            }
                        }))
                        .on_mouse_exit(cx.listener(move |shell, _, _, cx| {
                            if shell.hovered_session.as_deref() == Some(&exit_session_id) {
                                shell.hovered_session = None;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.attach_session(session_id.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_nowrap()
                                .items_center()
                                .gap(px(7.))
                                .w_full()
                                .min_w_0()
                                .child(if hovered && !renaming {
                                    tabler_icon(
                                        TablerIcon::GripVertical,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(12.),
                                    )
                                    .into_any_element()
                                } else {
                                    div()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(status_color)
                                        .into_any_element()
                                })
                                .child(if renaming {
                                    div()
                                        .id(format!("rename-session-{rename_input_id}"))
                                        .flex_1()
                                        .min_w_0()
                                        .h(px(20.))
                                        .px(px(4.))
                                        .rounded(px(5.))
                                        .border_1()
                                        .border_color(theme_rgb(&self.theme, "divider"))
                                        .bg(theme_rgb(&self.theme, "input"))
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.body"))
                                        .track_focus(&rename_focus_handle)
                                        .key_context("RenameSession")
                                        .role(Role::TextInput)
                                        .aria_label("Rename conversation")
                                        .cursor(gpui::CursorStyle::IBeam)
                                        .on_key_down(cx.listener(Self::rename_key_down))
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            cx.listener(|shell, _, window, cx| {
                                                shell.rename_mouse_down(window, cx);
                                            }),
                                        )
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.stop_propagation();
                                        }))
                                        .child(ComposerElement {
                                            shell: cx.entity(),
                                            target: TextInputTarget::Rename,
                                        })
                                        .into_any_element()
                                } else {
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .w_full()
                                        .pr(px(30.))
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_xs()
                                        .child(title)
                                        .into_any_element()
                                })
                                .when(pinned && !renaming, |element| {
                                    element.child(tabler_icon(
                                        TablerIcon::Pin,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(11.),
                                    ))
                                }),
                        )
                        .when(hovered, |element| element.child(move_control))
                        .when(has_meta, |element| {
                            element.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .pl(px(13.))
                                    .min_w_0()
                                    .text_xs()
                                    .opacity(0.82)
                                    .text_color(theme_rgb(&self.theme, "text.muted"))
                                    .when_some(path, |element, path| {
                                        element.child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .child(path),
                                        )
                                    })
                                    .when_some(session_time, |element, time| {
                                        element.child(
                                            div()
                                                .flex_none()
                                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                                .child(time),
                                        )
                                    }),
                            )
                        });
                    Some(
                        row.when(menu_open, |element| {
                            let rename_session_id = session_for_menu.clone();
                            let pin_session_id = session_for_menu.clone();
                            let archive_session_id = session_for_menu.clone();
                            element.h_auto().child(
                                div()
                                    .id(format!("session-menu-{session_for_menu}"))
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .ml(px(14.))
                                    .p(px(4.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(theme_rgb(&self.theme, "divider"))
                                    .bg(theme_rgb(&self.theme, "panel.background"))
                                    .role(Role::Menu)
                                    .aria_label("Conversation options")
                                    .child(session_menu_item(
                                        &self.theme,
                                        format!("rename-option-{session_for_menu}"),
                                        TablerIcon::Pencil,
                                        "Rename",
                                        "Rename conversation",
                                        move |shell, window, cx| {
                                            shell.begin_rename(
                                                rename_session_id.clone(),
                                                window,
                                                cx,
                                            );
                                        },
                                        cx,
                                    ))
                                    .child(session_menu_item(
                                        &self.theme,
                                        format!("pin-option-{session_for_menu}"),
                                        TablerIcon::Pin,
                                        if pinned { "Unpin" } else { "Pin" },
                                        if pinned {
                                            "Unpin conversation"
                                        } else {
                                            "Pin conversation"
                                        },
                                        move |shell, _, cx| {
                                            shell.pin_session(pin_session_id.clone(), !pinned, cx);
                                        },
                                        cx,
                                    ))
                                    .child(session_menu_item(
                                        &self.theme,
                                        format!("archive-option-{session_for_menu}"),
                                        TablerIcon::Archive,
                                        if archived { "Unarchive" } else { "Archive" },
                                        if archived {
                                            "Unarchive conversation"
                                        } else {
                                            "Archive conversation"
                                        },
                                        move |shell, _, cx| {
                                            shell.archive_session(
                                                archive_session_id.clone(),
                                                !archived,
                                                cx,
                                            );
                                        },
                                        cx,
                                    ))
                                    .child(
                                        div()
                                            .h(px(1.))
                                            .mx(px(4.))
                                            .my(px(2.))
                                            .bg(theme_rgb(&self.theme, "divider")),
                                    )
                                    .child(
                                        div()
                                            .px(px(7.))
                                            .pb(px(2.))
                                            .text_xs()
                                            .text_color(theme_rgb(&self.theme, "text.muted"))
                                            .child("Move to group"),
                                    )
                                    .child(
                                        div()
                                            .id(format!("group-option-none-{session_for_menu}"))
                                            .h(px(28.))
                                            .flex()
                                            .items_center()
                                            .px(px(7.))
                                            .rounded(px(5.))
                                            .cursor_pointer()
                                            .role(Role::MenuItem)
                                            .aria_label("Remove conversation from its group")
                                            .text_xs()
                                            .text_color(theme_rgb(&self.theme, "text.muted"))
                                            .hover(|element| {
                                                element.bg(theme_rgb(&self.theme, "muted"))
                                            })
                                            .on_click(cx.listener({
                                                let session_id = session_for_menu.clone();
                                                move |shell, _, _, cx| {
                                                    cx.stop_propagation();
                                                    shell.assign_session_group(
                                                        session_id.clone(),
                                                        None,
                                                        cx,
                                                    );
                                                }
                                            }))
                                            .child("No group"),
                                    )
                                    .children(groups.into_iter().map(|group| {
                                        let session_id = session_for_menu.clone();
                                        let group_id = group.id.clone();
                                        let selected =
                                            current_group_id.as_deref() == Some(&group_id);
                                        div()
                                            .id(format!("group-option-{group_id}"))
                                            .h(px(28.))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .px(px(7.))
                                            .rounded(px(5.))
                                            .cursor_pointer()
                                            .role(Role::MenuItem)
                                            .aria_label(SharedString::from(format!(
                                                "Move conversation to group {}{}",
                                                group.name,
                                                if selected { ", selected" } else { "" }
                                            )))
                                            .text_xs()
                                            .hover(|element| {
                                                element.bg(theme_rgb(&self.theme, "muted"))
                                            })
                                            .on_click(cx.listener(move |shell, _, _, cx| {
                                                cx.stop_propagation();
                                                shell.assign_session_group(
                                                    session_id.clone(),
                                                    Some(group_id.clone()),
                                                    cx,
                                                );
                                            }))
                                            .child(
                                                div()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(theme_rgb(&self.theme, "accent")),
                                            )
                                            .child(SharedString::from(group.name))
                                    })),
                            )
                        })
                        .into_any_element(),
                    )
                }
            })
            .collect()
    }

    pub(super) fn render_sidebar_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_session_rows(index..index + 1, window, cx)
            .into_iter()
            .next()
            .unwrap_or_else(|| div().into_any_element())
    }

    pub(super) fn sync_sidebar_list(&mut self) {
        let count = self.sidebar_rows.len();
        if self.sidebar_list.item_count() != count {
            self.sidebar_list.reset(count);
        }
    }

    pub(super) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.sidebar_rows.is_empty() {
            self.rebuild_sidebar_rows();
        }
        self.sync_sidebar_list();

        let sidebar = if self.layout.sidebar_collapsed {
            div()
                .id("shell-sidebar-collapsed")
                .flex()
                .flex_col()
                .items_center()
                .w(px(56.))
                .h_full()
                .gap(px(10.))
                .p(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(34.))
                        .rounded(px(10.))
                        .bg(theme_rgb(&self.theme, "accent"))
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("m"),
                )
                .child(
                    div()
                        .id("collapsed-new-conversation")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(34.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .role(Role::Button)
                        .aria_label("New conversation")
                        .text_lg()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.dispatch_shell_command(ShellCommand::NewConversation, cx);
                        }))
                        .child(tabler_icon(
                            TablerIcon::Plus,
                            theme_rgb(&self.theme, "text.muted"),
                            px(16.),
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(self.model.ui.conversations.len().to_string()),
                )
                .child(
                    div()
                        .id("collapsed-settings")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(34.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .role(Role::Button)
                        .aria_label("Open settings")
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.toggle_settings(cx);
                        }))
                        .child(tabler_icon(
                            TablerIcon::Settings,
                            theme_rgb(&self.theme, "text.muted"),
                            px(16.),
                        )),
                )
                .child(div().flex_1())
                .child(div().size(px(8.)).rounded_full().bg(
                    if self.model.connection.label() == "connected" {
                        theme_rgb(&self.theme, "green.fg")
                    } else {
                        theme_rgb(&self.theme, "yellow.fg")
                    },
                ))
        } else {
            div()
                .id("shell-sidebar")
                .flex()
                .flex_col()
                .w(px(264.))
                .h_full()
                .min_h_0()
                .gap(px(10.))
                .p(px(16.))
                .child(
                    gpui::list(
                        self.sidebar_list.clone(),
                        cx.processor(Self::render_sidebar_row),
                    )
                    .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
                    .p(px(6.))
                    .flex_1()
                    .min_h_0(),
                )
                .child(
                    div()
                        .id("new-conversation")
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(34.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .role(Role::Button)
                        .aria_label("New conversation")
                        .text_sm()
                        .text_color(theme_rgb(&self.theme, "text.body"))
                        .border_1()
                        .border_color(theme_rgb(&self.theme, "divider"))
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.dispatch_shell_command(ShellCommand::NewConversation, cx);
                        }))
                        .child(tabler_icon(
                            TablerIcon::Plus,
                            theme_rgb(&self.theme, "text.body"),
                            px(15.),
                        ))
                        .child("New conversation"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .pt(px(6.))
                        .border_t_1()
                        .border_color(theme_rgb(&self.theme, "divider"))
                        .child(
                            div()
                                .id("settings-trigger")
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(28.))
                                .rounded(px(7.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_label("Open settings")
                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                .on_click(cx.listener(|shell, _, _, cx| {
                                    shell.toggle_settings(cx);
                                }))
                                .child(tabler_icon(
                                    TablerIcon::Settings,
                                    theme_rgb(&self.theme, "text.muted"),
                                    px(15.),
                                )),
                        ),
                )
        };

        let sidebar_width = if self.layout.sidebar_collapsed {
            SIDEBAR_COLLAPSED_WIDTH
        } else {
            SIDEBAR_EXPANDED_WIDTH
        };
        let sidebar = div()
            .id("sidebar-transition-wrapper")
            .flex()
            .flex_none()
            .relative()
            .h_full()
            .overflow_hidden()
            .rounded(px(SHELL_SURFACE_RADIUS))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .bg(theme_rgb(&self.theme, "sidebar.background"))
            .child(sidebar);
        if self.sidebar_animation_id == 0 {
            return sidebar.w(px(sidebar_width)).into_any_element();
        }

        let collapsed = self.layout.sidebar_collapsed;
        let animation_id = self.sidebar_animation_id;
        sidebar
            .with_animation(
                ElementId::Name(format!("sidebar-transition-{animation_id}").into()),
                Animation::new(Duration::from_millis(220)).with_easing(gpui::ease_out_quint()),
                move |element, delta| {
                    element
                        .w(px(sidebar_transition_width(collapsed, delta)))
                        .left(px(sidebar_transition_offset(collapsed, delta)))
                },
            )
            .into_any_element()
    }
}

fn session_menu_item(
    theme: &Theme,
    id: String,
    icon: TablerIcon,
    label: &'static str,
    aria: &'static str,
    handler: impl Fn(&mut DesktopShell, &mut Window, &mut Context<DesktopShell>) + 'static,
    cx: &mut Context<DesktopShell>,
) -> gpui::AnyElement {
    div()
        .id(id)
        .h(px(28.))
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(7.))
        .rounded(px(5.))
        .cursor_pointer()
        .role(Role::MenuItem)
        .aria_label(aria)
        .text_xs()
        .hover(|element| element.bg(theme_rgb(theme, "muted")))
        .on_click(cx.listener(move |shell, _, window, cx| {
            cx.stop_propagation();
            handler(shell, window, cx);
        }))
        .child(tabler_icon(icon, theme_rgb(theme, "text.muted"), px(12.)))
        .child(label)
        .into_any_element()
}
