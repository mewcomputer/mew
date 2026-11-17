use super::*;

impl DesktopShell {
    pub(super) fn set_sidebar_filter(&mut self, filter: SidebarFilter, cx: &mut Context<Self>) {
        self.sidebar_filter = filter;
        self.sidebar_keyboard_session = None;
        self.session_menu_session = None;
        self.workspace_open_menu = None;
        self.rebuild_sidebar_rows();
        self.sidebar_list.reset(self.sidebar_rows.len());
        cx.notify();
    }

    fn render_sidebar_filters(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .flex()
            .gap(px(3.))
            .flex_wrap()
            .children(
                [
                    (SidebarFilter::All, "all", "All"),
                    (SidebarFilter::Running, "running", "Running"),
                    (SidebarFilter::Attention, "attention", "Attention"),
                ]
                .into_iter()
                .map(|(filter, id, label)| {
                    let selected = self.sidebar_filter == filter;
                    let title = if filter == SidebarFilter::All {
                        label.to_owned()
                    } else {
                        let count = self
                            .model
                            .ui
                            .conversations
                            .iter()
                            .filter(|conversation| filter.matches(conversation))
                            .count();
                        format!("{label} {count}")
                    };
                    div()
                        .id(format!("sidebar-filter-{id}"))
                        .flex_none()
                        .px(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(28.))
                        .rounded(px(5.))
                        .text_xs()
                        .text_color(theme_rgb(
                            &self.theme,
                            if selected { "text.body" } else { "text.muted" },
                        ))
                        .when(selected, |element| {
                            element.bg(theme_rgb(&self.theme, "muted"))
                        })
                        .cursor_pointer()
                        .role(Role::Button)
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                        .aria_selected(selected)
                        .aria_label(SharedString::from(title.clone()))
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.set_sidebar_filter(filter, cx);
                        }))
                        .child(SharedString::from(title))
                }),
            )
            .into_any_element()
    }

    pub(super) fn sidebar_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape"
            && (self.session_menu_session.is_some() || self.workspace_open_menu.is_some())
        {
            self.close_shell_popovers();
            window.focus(&self.sidebar_focus_handle, cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let session_indices: Vec<usize> = self
            .sidebar_rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| matches!(row, SidebarRow::Session(_)).then_some(index))
            .collect();
        if session_indices.is_empty() {
            return;
        }

        let current_index = self
            .sidebar_keyboard_session
            .as_deref()
            .and_then(|session_id| {
                self.sidebar_rows.iter().position(|row| {
                    matches!(row, SidebarRow::Session(conversation) if conversation.session_id == session_id)
                })
            })
            .or_else(|| {
                self.model.ui.selected_session.as_deref().and_then(|session_id| {
                    self.sidebar_rows.iter().position(|row| {
                        matches!(row, SidebarRow::Session(conversation) if conversation.session_id == session_id)
                    })
                })
            })
            .unwrap_or(session_indices[0]);
        let Some(session_position) = session_indices
            .iter()
            .position(|&index| index == current_index)
        else {
            return;
        };
        let next_position = match event.keystroke.key.as_str() {
            "up" => session_position.saturating_sub(1),
            "down" => (session_position + 1).min(session_indices.len() - 1),
            "home" => 0,
            "end" => session_indices.len() - 1,
            "f10" if event.keystroke.modifiers.shift => {
                if let SidebarRow::Session(conversation) = &self.sidebar_rows[current_index] {
                    self.toggle_session_menu(conversation.session_id.clone(), cx);
                    cx.stop_propagation();
                }
                return;
            }
            "enter" | "return" | "space" => {
                if let SidebarRow::Session(conversation) = &self.sidebar_rows[current_index] {
                    self.sidebar_keyboard_session = Some(conversation.session_id.clone());
                    self.attach_session(conversation.session_id.clone(), cx);
                    cx.stop_propagation();
                }
                return;
            }
            "escape" => {
                window.focus(&self.composer_focus_handle, cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        let next_index = session_indices[next_position];
        if let SidebarRow::Session(conversation) = &self.sidebar_rows[next_index] {
            self.sidebar_keyboard_session = Some(conversation.session_id.clone());
            self.sidebar_list.scroll_to(gpui::ListOffset {
                item_ix: next_index,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn render_sidebar_search(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let search_focus_handle = self.sidebar_search_focus_handle.clone();
        let clear_search = (!self.sidebar_search.is_empty()).then(|| {
            div()
                .id("clear-sidebar-search")
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded(px(5.))
                .cursor_pointer()
                .role(Role::Button)
                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                .aria_label("Clear session search")
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(|shell, _, _, cx| {
                    cx.stop_propagation();
                    shell.clear_sidebar_search(cx);
                }))
                .child(tabler_icon(
                    TablerIcon::X,
                    theme_rgb(&self.theme, "text.muted"),
                    px(12.),
                ))
        });
        div()
            .id("sidebar-search-frame")
            .flex()
            .items_center()
            .gap(px(4.))
            .h(px(28.))
            .px(px(7.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .bg(theme_rgb(&self.theme, "input"))
            .text_xs()
            .text_color(theme_rgb(&self.theme, "text.muted"))
            .child(tabler_icon(
                TablerIcon::Search,
                theme_rgb(&self.theme, "text.muted"),
                px(13.),
            ))
            .child(
                div()
                    .id("sidebar-search-input-frame")
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .track_focus(&search_focus_handle)
                    .key_context("SidebarSearch")
                    .on_action(cx.listener(Self::text_copy))
                    .on_action(cx.listener(Self::text_cut))
                    .on_action(cx.listener(Self::text_paste))
                    .on_action(cx.listener(Self::text_select_all))
                    .role(Role::TextInput)
                    .aria_label("Search sessions")
                    .aria_keyshortcuts("cmd-k")
                    .cursor(gpui::CursorStyle::IBeam)
                    .focus_visible(|element| {
                        element
                            .border_1()
                            .border_color(theme_rgb(&self.theme, "accent"))
                    })
                    .on_key_down(cx.listener(Self::sidebar_search_key_down))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|shell, _, window, cx| {
                            shell.sidebar_search_mouse_down(window, cx);
                        }),
                    )
                    .child(ComposerElement {
                        shell: cx.entity(),
                        target: TextInputTarget::SidebarSearch,
                    }),
            )
            .when_some(clear_search, |element, clear_search| {
                element.child(clear_search)
            })
            .into_any_element()
    }

    pub(super) fn render_session_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
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
                        .child(if self.sidebar_filter == SidebarFilter::Inbox {
                            "UNREAD & REQUESTS"
                        } else if self.sidebar_filter == SidebarFilter::Archived {
                            "ARCHIVED CHATS"
                        } else {
                            "WORKSPACES"
                        })
                        .into_any_element(),
                ),
                SidebarRow::Workspace {
                    path,
                    name,
                    pinned,
                    count,
                    collapsed,
                    collapsible,
                } => {
                    let workspace_id = if path.is_empty() {
                        "other".to_owned()
                    } else {
                        path.clone()
                    };
                    let toggle_path = path.clone();
                    let new_path = path.clone();
                    let open_menu = self.workspace_open_menu.as_deref() == Some(path.as_str());
                    let menu_path = path.clone();
                    Some(
                        div()
                            .id(format!("sidebar-workspace-{workspace_id}"))
                            .flex()
                            .items_center()
                            .w_full()
                            .h(px(30.))
                            .px(px(6.))
                            .gap(px(6.))
                            .rounded(px(6.))
                            .relative()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                            .child(
                                div()
                                    .id(format!("toggle-workspace-{workspace_id}"))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .when(collapsible, |element| {
                                        element
                                            .cursor_pointer()
                                            .role(Role::Button)
                                            .aria_expanded(!collapsed)
                                            .aria_label(SharedString::from(format!(
                                                "{} workspace, {} tasks, {}",
                                                name,
                                                count,
                                                if collapsed { "collapsed" } else { "expanded" }
                                            )))
                                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                            .on_click(cx.listener(move |shell, _, _, cx| {
                                                shell.toggle_group(toggle_path.clone(), cx);
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
                                    })
                                    .child(tabler_icon(
                                        TablerIcon::Folder,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(14.),
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(SharedString::from(name.clone())),
                                    )
                                    .when(pinned, |element| {
                                        element.child(tabler_icon(
                                            TablerIcon::Pin,
                                            theme_rgb(&self.theme, "text.muted"),
                                            px(11.),
                                        ))
                                    }),
                            )
                            .child(
                                div()
                                    .id(format!("workspace-menu-trigger-{workspace_id}"))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22.))
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .role(Role::Button)
                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                    .aria_label("Workspace options")
                                    .hover(|element| element.bg(theme_rgb(&self.theme, "divider")))
                                    .on_click(cx.listener(move |shell, _, _, cx| {
                                        cx.stop_propagation();
                                        shell.toggle_workspace_open_menu(menu_path.clone(), cx);
                                    }))
                                    .child(tabler_icon(
                                        TablerIcon::Dots,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(13.),
                                    )),
                            )
                            .child(
                                div()
                                    .id(format!("new-task-in-workspace-{workspace_id}"))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22.))
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .role(Role::Button)
                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                    .aria_label(SharedString::from(format!("New task in {name}")))
                                    .hover(|element| element.bg(theme_rgb(&self.theme, "divider")))
                                    .on_click(cx.listener(move |shell, _, _, cx| {
                                        if new_path.is_empty() {
                                            shell.new_conversation(cx);
                                        } else {
                                            shell.new_conversation_in_workspace(
                                                new_path.clone(),
                                                cx,
                                            );
                                        }
                                    }))
                                    .child(tabler_icon(
                                        TablerIcon::Plus,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(13.),
                                    )),
                            )
                            .when(open_menu, |element| {
                                element.child(
                                    deferred(self.render_workspace_open_menu(
                                        path.clone(),
                                        workspace_id.clone(),
                                        pinned,
                                        cx,
                                    ))
                                    .with_priority(8),
                                )
                            })
                            .into_any_element(),
                    )
                }
                SidebarRow::ShowMore {
                    workspace_path,
                    count,
                } => {
                    let path = workspace_path.clone();
                    Some(
                        div()
                            .id(format!(
                                "show-more-workspace-{}",
                                if path.is_empty() { "other" } else { &path }
                            ))
                            .h(px(28.))
                            .pl(px(34.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .role(Role::Button)
                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                            .aria_label(SharedString::from(format!("Show {count} more tasks")))
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .hover(|element| {
                                element.text_color(theme_rgb(&self.theme, "text.body"))
                            })
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                shell.show_more_workspace(path.clone(), cx);
                            }))
                            .child(SharedString::from(format!("Show more ({count})")))
                            .into_any_element(),
                    )
                }
                SidebarRow::Archived { count } => Some(
                    div()
                        .id("sidebar-archived")
                        .debug_selector(|| "sidebar-archived".into())
                        .h(px(30.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .cursor_pointer()
                        .role(Role::Button)
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                        .aria_label(SharedString::from(format!("Archived chats, {count}")))
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.set_sidebar_filter(SidebarFilter::Archived, cx);
                        }))
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(tabler_icon(
                            TablerIcon::Archive,
                            theme_rgb(&self.theme, "text.muted"),
                            px(13.),
                        ))
                        .child("Archived")
                        .child(div().flex_none().child(count.to_string()))
                        .into_any_element(),
                ),
                SidebarRow::Group {
                    id,
                    name,
                    color,
                    count,
                    collapsed,
                } => {
                    let drop_group_id = id.clone();
                    let new_session_group_id = id.clone();
                    let drag_over = self.drag_over_group.as_deref() == Some(id.as_str());
                    let group_hovered = self.hovered_group.as_deref() == Some(id.as_str());
                    let delete_group_id = id.clone();
                    let delete_group_name = name.clone();
                    let is_pseudo_group = matches!(
                        id.as_str(),
                        UNGROUPED_GROUP_ID | ARCHIVED_GROUP_ID | PINNED_GROUP_ID
                    );
                    let pending_delete =
                        self.pending_group_deletion.as_deref() == Some(id.as_str());
                    let delete_control = (!is_pseudo_group && group_hovered && !pending_delete)
                        .then(|| {
                            div()
                                .id(format!("delete-group-{delete_group_id}"))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(20.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                .aria_label(SharedString::from(format!(
                                    "Delete group {delete_group_name}"
                                )))
                                .hover(|element| element.bg(theme_rgb(&self.theme, "divider")))
                                .on_click(cx.listener(move |shell, _, _, cx| {
                                    cx.stop_propagation();
                                    shell.request_group_deletion(delete_group_id.clone(), cx);
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
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                    let toggle_group_id = id.clone();
                    let group_toggle_control = div()
                        .id(format!("toggle-sidebar-group-{id}"))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .cursor_pointer()
                        .role(Role::Button)
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                        .aria_expanded(!collapsed)
                        .aria_label(SharedString::from(format!(
                            "{} group, {} sessions, {}",
                            name,
                            count,
                            if collapsed { "collapsed" } else { "expanded" }
                        )))
                        .focus_visible(|element| {
                            element
                                .border_1()
                                .border_color(theme_rgb(&self.theme, "accent"))
                        })
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.toggle_group(toggle_group_id.clone(), cx);
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
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(SharedString::from(name.clone())),
                        )
                        .child(div().flex_none().text_xs().child(count.to_string()));
                    let cancel_delete_id = id.clone();
                    let confirm_delete_id = id.clone();
                    let confirmation_control = pending_delete.then(|| {
                        div()
                            .id(format!("confirm-delete-group-{id}"))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .ml(px(20.))
                            .mt(px(2.))
                            .pb(px(4.))
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .aria_label(SharedString::from(format!(
                                "Remove group {delete_group_name}. Sessions stay ungrouped."
                            )))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(SharedString::from(format!(
                                        "Remove {delete_group_name}?"
                                    ))),
                            )
                            .child(
                                div()
                                    .id(format!("cancel-delete-group-{id}"))
                                    .px(px(5.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .role(Role::Button)
                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                    .aria_label("Cancel group removal")
                                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                    .on_click(cx.listener(move |shell, _, _, cx| {
                                        cx.stop_propagation();
                                        shell.cancel_group_deletion(&cancel_delete_id, cx);
                                    }))
                                    .child("Cancel"),
                            )
                            .child(
                                div()
                                    .id(format!("confirm-delete-group-action-{id}"))
                                    .px(px(5.))
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .role(Role::Button)
                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                    .aria_label(SharedString::from(format!(
                                        "Remove group {delete_group_name}"
                                    )))
                                    .text_color(theme_rgb(&self.theme, "red.fg"))
                                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                    .on_click(cx.listener(move |shell, _, _, cx| {
                                        cx.stop_propagation();
                                        shell.confirm_group_deletion(confirm_delete_id.clone(), cx);
                                    }))
                                    .child("Remove"),
                            )
                    });
                    let group_header = div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .w_full()
                        .h(px(30.))
                        .child(group_toggle_control)
                        .when(
                            group_hovered
                                && !matches!(id.as_str(), ARCHIVED_GROUP_ID | PINNED_GROUP_ID)
                                && !pending_delete,
                            |element| element.child(new_session_control),
                        )
                        .when_some(delete_control, |element, control| element.child(control));
                    Some(
                        div()
                            .id(format!("sidebar-group-{id}"))
                            .flex()
                            .flex_col()
                            .items_stretch()
                            .w_full()
                            .h(px(30.))
                            .px(px(4.))
                            .rounded(px(6.))
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .when(drag_over, |element| {
                                element
                                    .bg(theme_rgb(&self.theme, "accent").opacity(0.14))
                                    .border_1()
                                    .border_color(theme_rgb(&self.theme, "accent").opacity(0.5))
                            })
                            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
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
                                    if matches!(
                                        group_id.as_str(),
                                        ARCHIVED_GROUP_ID | PINNED_GROUP_ID
                                    ) {
                                        return;
                                    }
                                    let target =
                                        (group_id != UNGROUPED_GROUP_ID).then(|| group_id.clone());
                                    shell.assign_session_group(drag.session_id.clone(), target, cx);
                                }
                            }))
                            .child(group_header)
                            .when_some(confirmation_control, |element, control| {
                                element.h_auto().child(control)
                            })
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

                    let session_time =
                        session_time_label(conversation.last_message_at).map(SharedString::from);
                    let status_color = if conversation.needs_attention {
                        theme_rgb(&self.theme, "red.fg")
                    } else if conversation.state == mew_protocol::SessionState::Running {
                        theme_rgb(&self.theme, "yellow.fg")
                    } else if conversation.last_turn_failed {
                        theme_rgb(&self.theme, "red.fg")
                    } else if conversation.unread_completion {
                        theme_rgb(&self.theme, "green.fg")
                    } else if selected {
                        theme_rgb(&self.theme, "text.body")
                    } else {
                        theme_rgb(&self.theme, "text.muted")
                    };
                    let status_label = if conversation.needs_attention {
                        Some("needs input")
                    } else if conversation.state == mew_protocol::SessionState::Running {
                        Some("running")
                    } else if conversation.last_turn_failed {
                        Some("failed")
                    } else if conversation.unread_completion {
                        Some("completed")
                    } else {
                        None
                    };
                    let status_icon = if conversation.needs_attention {
                        Some(TablerIcon::MessageCircle)
                    } else if conversation.state == mew_protocol::SessionState::Running {
                        Some(TablerIcon::Square)
                    } else if conversation.last_turn_failed && conversation.unread_completion {
                        Some(TablerIcon::X)
                    } else {
                        None
                    };
                    let unread_dot = conversation.unread_completion && status_icon.is_none();
                    let row_height = 30.;
                    let details: SharedString = format!(
                        "Chat: {}{}{}{}",
                        title,
                        path.as_deref()
                            .map(|path| format!(" · {path}"))
                            .unwrap_or_default(),
                        status_label
                            .map(|status| format!(" · {status}"))
                            .unwrap_or_default(),
                        session_time
                            .as_deref()
                            .map(|time| format!(" · {time}"))
                            .unwrap_or_default(),
                    )
                    .into();
                    let tooltip_background = theme_rgb(&self.theme, "panel.background");
                    let tooltip_foreground = theme_rgb(&self.theme, "text.body");
                    let tooltip_muted = theme_rgb(&self.theme, "text.muted");
                    let tooltip_border = theme_rgb(&self.theme, "divider");
                    let menu_open = self.session_menu_session.as_deref() == Some(&session_id);
                    let renaming = self.rename_session_id.as_deref() == Some(&session_id);
                    let pinned = conversation.pinned;
                    let archived = conversation.archived;
                    let current_group_id = conversation.group_id.clone();
                    let unread_completion = conversation.unread_completion;
                    let tooltip_conversation = conversation;
                    let session_for_menu = session_id.clone();
                    let session_group = SharedString::from(format!("sidebar-session-{session_id}"));
                    let show_actions = !renaming
                        && (menu_open
                            || self.sidebar_keyboard_session.as_deref()
                                == Some(session_id.as_str())
                                && self.sidebar_focus_handle.contains_focused(window, cx));
                    let actions = div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_end()
                        .invisible()
                        .when(!renaming, |element| {
                            element.group_hover(session_group.clone(), |style| style.visible())
                        })
                        .when(show_actions, |element| element.visible())
                        .children(
                            [
                                (
                                    "pin",
                                    TablerIcon::Pin,
                                    if pinned {
                                        "Unpin conversation"
                                    } else {
                                        "Pin conversation"
                                    },
                                ),
                                (
                                    "archive",
                                    TablerIcon::Archive,
                                    if archived {
                                        "Unarchive conversation"
                                    } else {
                                        "Archive conversation"
                                    },
                                ),
                            ]
                            .into_iter()
                            .map(|(action, icon, label)| {
                                let target = session_id.clone();
                                div()
                                    .id(format!("session-{action}-trigger-{session_id}"))
                                    .debug_selector({
                                        let session_id = session_id.clone();
                                        move || format!("sidebar-{action}-{session_id}")
                                    })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22.))
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .role(Role::Button)
                                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                    .aria_label(SharedString::from(format!("{label}: {title}")))
                                    .hover(|element| element.bg(theme_rgb(&self.theme, "divider")))
                                    .on_click(cx.listener(move |shell, _, _, cx| {
                                        cx.stop_propagation();
                                        match action {
                                            "pin" => shell.pin_session(target.clone(), !pinned, cx),
                                            _ => {
                                                shell.archive_session(target.clone(), !archived, cx)
                                            }
                                        }
                                    }))
                                    .child(tabler_icon(
                                        icon,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(13.),
                                    ))
                            }),
                        );
                    let rename_input_id = session_id.clone();
                    let rename_focus_handle = self.rename_focus_handle.clone();
                    let select_session_id = session_id.clone();
                    let context_session_id = session_id.clone();
                    let row = div()
                        .id(format!("session-{session_id}"))
                        .debug_selector({
                            let session_id = session_id.clone();
                            move || format!("sidebar-chat-{session_id}")
                        })
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
                        .group(session_group.clone())
                        .tooltip(move |_, cx| {
                            cx.new(|_| SidebarChatPreview {
                                conversation: tooltip_conversation.clone(),
                                background: tooltip_background,
                                foreground: tooltip_foreground,
                                muted: tooltip_muted,
                                border: tooltip_border,
                            })
                            .into()
                        })
                        .when(selected, |element| {
                            element.bg(theme_rgb(&self.theme, "accent"))
                        })
                        .when(!selected, |element| {
                            element.hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        })
                        .child(
                            div()
                                .id(format!("select-session-{select_session_id}"))
                                .flex()
                                .flex_col()
                                .w_full()
                                .justify_center()
                                .gap(px(2.))
                                .h(px(row_height))
                                .cursor_pointer()
                                .role(Role::Button)
                                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                .aria_selected(selected)
                                .focus_visible(|element| {
                                    element
                                        .border_1()
                                        .border_color(theme_rgb(&self.theme, "accent"))
                                })
                                .when(
                                    self.sidebar_keyboard_session.as_deref()
                                        == Some(select_session_id.as_str()),
                                    |element| element.aria_active_descendant(),
                                )
                                .aria_label(details)
                                .aria_keyshortcuts("Shift+F10")
                                .on_mouse_down(
                                    gpui::MouseButton::Right,
                                    cx.listener(move |shell, _, window, cx| {
                                        cx.stop_propagation();
                                        window.focus(&shell.sidebar_focus_handle, cx);
                                        shell.sidebar_keyboard_session =
                                            Some(context_session_id.clone());
                                        shell.toggle_session_menu(context_session_id.clone(), cx);
                                    }),
                                )
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
                                .on_click(cx.listener(move |shell, _, window, cx| {
                                    window.focus(&shell.sidebar_focus_handle, cx);
                                    shell.sidebar_keyboard_session =
                                        Some(select_session_id.clone());
                                    shell.attach_session(select_session_id.clone(), cx);
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .flex_nowrap()
                                        .items_center()
                                        .gap(px(7.))
                                        .w_full()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .flex_none()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .size(px(12.))
                                                .when_some(status_icon, |element, icon| {
                                                    element.child(tabler_icon(
                                                        icon,
                                                        status_color,
                                                        px(12.),
                                                    ))
                                                })
                                                .when(unread_dot, |element| {
                                                    element.child(
                                                        div()
                                                            .size(px(5.))
                                                            .rounded_full()
                                                            .bg(status_color),
                                                    )
                                                })
                                                .when(
                                                    status_icon.is_none() && !unread_dot && pinned,
                                                    |element| {
                                                        element.child(tabler_icon(
                                                            TablerIcon::Pin,
                                                            theme_rgb(&self.theme, "text.muted"),
                                                            px(11.),
                                                        ))
                                                    },
                                                ),
                                        )
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
                                                .on_action(cx.listener(Self::text_copy))
                                                .on_action(cx.listener(Self::text_cut))
                                                .on_action(cx.listener(Self::text_paste))
                                                .on_action(cx.listener(Self::text_select_all))
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
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .text_size(px(13.))
                                                .when(unread_completion, |element| {
                                                    element.font_weight(gpui::FontWeight::MEDIUM)
                                                })
                                                .child(title)
                                                .into_any_element()
                                        })
                                        .child(
                                            div()
                                                .relative()
                                                .flex_none()
                                                .w(px(44.))
                                                .h(px(22.))
                                                .when_some(session_time, |element, time| {
                                                    element.child(
                                                        div()
                                                            .absolute()
                                                            .inset_0()
                                                            .flex()
                                                            .items_center()
                                                            .justify_end()
                                                            .text_xs()
                                                            .text_color(theme_rgb(
                                                                &self.theme,
                                                                "text.muted",
                                                            ))
                                                            .group_hover(
                                                                session_group.clone(),
                                                                |style| style.invisible(),
                                                            )
                                                            .when(
                                                                show_actions || renaming,
                                                                |element| element.invisible(),
                                                            )
                                                            .child(time),
                                                    )
                                                })
                                                .child(actions),
                                        ),
                                ),
                        );
                    Some(
                        row.when(menu_open, |element| {
                            let groups = self.model.ui.groups.clone();
                            let rename_session_id = session_for_menu.clone();
                            let pin_session_id = session_for_menu.clone();
                            let archive_session_id = session_for_menu.clone();
                            element.child(
                                deferred(
                                    div()
                                        .id(format!("session-menu-{session_for_menu}"))
                                        .absolute()
                                        .top(px(row_height))
                                        .left(px(14.))
                                        .w(px(224.))
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .p(px(4.))
                                        .rounded(px(7.))
                                        .border_1()
                                        .border_color(theme_rgb(&self.theme, "divider"))
                                        .bg(theme_rgb(&self.theme, "panel.background"))
                                        .role(Role::Menu)
                                        .aria_label("Conversation options")
                                        .on_mouse_down_out(cx.listener(|shell, _, _, cx| {
                                            shell.session_menu_session = None;
                                            cx.notify();
                                        }))
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
                                                shell.pin_session(
                                                    pin_session_id.clone(),
                                                    !pinned,
                                                    cx,
                                                );
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
                                        .child({
                                            let session_id = session_for_menu.clone();
                                            let no_group = div()
                                                .id(format!("group-option-none-{session_for_menu}"))
                                                .h(px(28.))
                                                .flex()
                                                .items_center()
                                                .px(px(7.))
                                                .rounded(px(5.))
                                                .cursor_pointer()
                                                .role(Role::MenuItem)
                                                .desktop_focus(theme_rgb(
                                                    &self.theme,
                                                    "text.accent",
                                                ))
                                                .aria_selected(current_group_id.is_none())
                                                .aria_label("Remove conversation from its group")
                                                .text_xs()
                                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                                .hover(|element| {
                                                    element.bg(theme_rgb(&self.theme, "muted"))
                                                })
                                                .on_click(cx.listener(move |shell, _, _, cx| {
                                                    cx.stop_propagation();
                                                    shell.assign_session_group(
                                                        session_id.clone(),
                                                        None,
                                                        cx,
                                                    );
                                                }))
                                                .child("No group");
                                            div()
                                                .id(format!("group-options-{session_for_menu}"))
                                                .flex()
                                                .flex_col()
                                                .max_h(px(168.))
                                                .overflow_y_scroll()
                                                .children(std::iter::once(no_group).chain(
                                                    groups.into_iter().map(|group| {
                                                        let session_id = session_for_menu.clone();
                                                        let group_id = group.id.clone();
                                                        let selected = current_group_id.as_deref()
                                                            == Some(&group_id);
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
                                                            .desktop_focus(theme_rgb(
                                                                &self.theme,
                                                                "text.accent",
                                                            ))
                                                            .aria_selected(selected)
                                                            .aria_label(SharedString::from(
                                                                format!(
                                                            "Move conversation to group {}{}",
                                                            group.name,
                                                            if selected {
                                                                ", selected"
                                                            } else {
                                                                ""
                                                            }
                                                        ),
                                                            ))
                                                            .text_xs()
                                                            .hover(|element| {
                                                                element.bg(theme_rgb(
                                                                    &self.theme,
                                                                    "muted",
                                                                ))
                                                            })
                                                            .on_click(cx.listener(
                                                                move |shell, _, _, cx| {
                                                                    cx.stop_propagation();
                                                                    shell.assign_session_group(
                                                                        session_id.clone(),
                                                                        Some(group_id.clone()),
                                                                        cx,
                                                                    );
                                                                },
                                                            ))
                                                            .child(
                                                                div()
                                                                    .size(px(6.))
                                                                    .rounded_full()
                                                                    .bg(theme_rgb(
                                                                        &self.theme,
                                                                        "accent",
                                                                    )),
                                                            )
                                                            .child(
                                                                div()
                                                                    .flex_1()
                                                                    .min_w_0()
                                                                    .overflow_hidden()
                                                                    .whitespace_nowrap()
                                                                    .text_ellipsis()
                                                                    .child(SharedString::from(
                                                                        group.name,
                                                                    )),
                                                            )
                                                    }),
                                                ))
                                        }),
                                )
                                .with_priority(9),
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

    fn render_workspace_open_menu(
        &self,
        workspace_path: String,
        workspace_id: String,
        pinned: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let primary = primary_workspace_destination(
            self.remembered_editor.as_deref(),
            &self.workspace_open_destinations,
        );
        let destinations = self.workspace_open_destinations.clone();
        div()
            .id(format!("workspace-open-menu-{workspace_id}"))
            .absolute()
            .top(px(28.))
            .right(px(8.))
            .w(px(196.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(4.))
            .rounded(px(7.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .role(Role::Menu)
            .aria_label("Workspace options")
            .child(
                div()
                    .px(px(7.))
                    .py(px(3.))
                    .text_xs()
                    .text_color(theme_rgb(&self.theme, "text.muted"))
                    .child("Open with"),
            )
            .children(
                destinations
                    .iter()
                    .filter(|destination| {
                        !matches!(destination, WorkspaceOpenDestination::CopyPath)
                    })
                    .cloned()
                    .map(|destination| {
                        let selected = destination == primary;
                        workspace_open_menu_item(
                            &self.theme,
                            workspace_path.clone(),
                            destination,
                            selected,
                            cx,
                        )
                    }),
            )
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
                    .py(px(3.))
                    .text_xs()
                    .text_color(theme_rgb(&self.theme, "text.muted"))
                    .child("Actions"),
            )
            .child(workspace_pin_menu_item(
                &self.theme,
                workspace_path.clone(),
                pinned,
                cx,
            ))
            .children(
                destinations
                    .into_iter()
                    .filter(|destination| matches!(destination, WorkspaceOpenDestination::CopyPath))
                    .map(|destination| {
                        workspace_open_menu_item(
                            &self.theme,
                            workspace_path.clone(),
                            destination,
                            false,
                            cx,
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_sidebar_resize_handle(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .id("sidebar-resizer")
            .absolute()
            .top(px(0.))
            .right(px(0.))
            .bottom(px(0.))
            .w(px(8.))
            .flex()
            .items_center()
            .justify_center()
            .cursor(gpui::CursorStyle::ResizeColumn)
            .role(Role::Splitter)
            .aria_label("Resize sessions sidebar")
            .justify_end()
            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
            .child(
                div()
                    .w(px(1.))
                    .h_full()
                    .bg(theme_rgb(&self.theme, "divider")),
            )
            .on_drag(SidebarResizeDrag, |_, _, _, cx| {
                cx.new(|_| SidebarResizePreview)
            })
            .on_drag_move::<SidebarResizeDrag>(cx.listener(
                |shell, event: &gpui::DragMoveEvent<SidebarResizeDrag>, _, cx| {
                    shell.resize_sidebar(f32::from(event.event.position.x), cx);
                },
            ))
            .into_any_element()
    }

    pub(super) fn sync_sidebar_list(&mut self) {
        let count = self.sidebar_rows.len();
        if self.sidebar_list.item_count() != count {
            self.sidebar_list.reset(count);
        }
    }

    pub(super) fn render_sidebar(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self.sidebar_rows.is_empty() {
            self.rebuild_sidebar_rows();
        }
        self.sync_sidebar_list();
        let inbox_count = self
            .model
            .ui
            .conversations
            .iter()
            .filter(|conversation| SidebarFilter::Inbox.matches(conversation))
            .count();
        let inbox_selected = self.sidebar_filter == SidebarFilter::Inbox;
        let inbox_empty = inbox_selected
            && !self
                .sidebar_rows
                .iter()
                .any(|row| matches!(row, SidebarRow::Session(_)));

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
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
                .w(px(self.sidebar_width))
                .flex_1()
                .min_h_0()
                .gap(px(10.))
                .p(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(28.))
                        .px(px(6.))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("mew"),
                        )
                        .child(
                            div()
                                .id("sidebar-search-toggle")
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(26.))
                                .rounded(px(6.))
                                .cursor_pointer()
                                .role(Role::Button)
                                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                                .aria_label("Search chats")
                                .aria_keyshortcuts("Command+K")
                                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                                .on_click(cx.listener(|shell, _, window, cx| {
                                    shell.action_focus_sidebar_search(
                                        &FocusSidebarSearch,
                                        window,
                                        cx,
                                    );
                                }))
                                .child(tabler_icon(
                                    TablerIcon::Search,
                                    theme_rgb(&self.theme, "text.muted"),
                                    px(14.),
                                )),
                        ),
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
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                        .aria_label("New chat")
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
                        .child("New chat"),
                )
                .child(
                    div()
                        .id("sidebar-inbox")
                        .debug_selector(|| "sidebar-inbox".into())
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(30.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .text_sm()
                        .cursor_pointer()
                        .role(Role::Button)
                        .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                        .aria_selected(inbox_selected)
                        .aria_label(SharedString::from(format!(
                            "Inbox, {inbox_count} unread replies and requests"
                        )))
                        .when(inbox_selected, |element| {
                            element.bg(theme_rgb(&self.theme, "accent"))
                        })
                        .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                        .on_click(cx.listener(|shell, _, _, cx| {
                            let filter = if shell.sidebar_filter == SidebarFilter::Inbox {
                                SidebarFilter::All
                            } else {
                                SidebarFilter::Inbox
                            };
                            shell.set_sidebar_filter(filter, cx);
                        }))
                        .child(tabler_icon(
                            TablerIcon::MessageCircle,
                            theme_rgb(&self.theme, "text.muted"),
                            px(15.),
                        ))
                        .child(div().flex_1().child("Inbox"))
                        .when(inbox_count > 0, |element| {
                            element.child(
                                div()
                                    .debug_selector(move || {
                                        format!("sidebar-inbox-count-{inbox_count}")
                                    })
                                    .text_xs()
                                    .text_color(theme_rgb(&self.theme, "text.muted"))
                                    .child(SharedString::from(if inbox_count > 99 {
                                        "99+".into()
                                    } else {
                                        inbox_count.to_string()
                                    })),
                            )
                        }),
                )
                .when(
                    self.sidebar_search_open
                        || self.sidebar_search_focus_handle.is_focused(window)
                        || !self.sidebar_search.is_empty()
                        || self.sidebar_filter != SidebarFilter::All && !inbox_selected,
                    |element| {
                        element
                            .child(self.render_sidebar_search(cx))
                            .child(self.render_sidebar_filters(cx))
                    },
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .child(
                            div()
                                .id("sidebar-session-list")
                                .flex()
                                .when(inbox_empty, |element| element.hidden())
                                .flex_col()
                                .flex_1()
                                .min_h_0()
                                .track_focus(&self.sidebar_focus_handle)
                                .role(Role::ListBox)
                                .aria_label("Sessions")
                                .on_key_down(cx.listener(Self::sidebar_key_down))
                                .child(
                                    gpui::list(
                                        self.sidebar_list.clone(),
                                        cx.processor(Self::render_sidebar_row),
                                    )
                                    .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
                                    .p(px(6.))
                                    .flex_1()
                                    .min_h_0(),
                                ),
                        )
                        .when(inbox_empty, |element| {
                            element.child(
                                div()
                                    .debug_selector(|| "sidebar-inbox-empty".into())
                                    .p(px(12.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .text_sm()
                                    .child(if self.sidebar_search.trim().is_empty() {
                                        "You're all caught up"
                                    } else {
                                        "No matching notifications"
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme_rgb(&self.theme, "text.muted"))
                                            .child("Unread replies and requests will appear here."),
                                    ),
                            )
                        })
                        .when(
                            !inbox_empty
                                && (!self.sidebar_search.trim().is_empty()
                                    || self.sidebar_filter != SidebarFilter::All)
                                && !self
                                    .sidebar_rows
                                    .iter()
                                    .any(|row| matches!(row, SidebarRow::Session(_))),
                            |element| {
                                element.child(
                                    div()
                                        .px(px(10.))
                                        .pb(px(8.))
                                        .text_xs()
                                        .text_color(theme_rgb(&self.theme, "text.muted"))
                                        .child(if !self.sidebar_search.trim().is_empty() {
                                            "No matching sessions"
                                        } else if self.sidebar_filter == SidebarFilter::Running {
                                            "No running tasks"
                                        } else if self.sidebar_filter == SidebarFilter::Archived {
                                            "No archived chats"
                                        } else {
                                            "No tasks need attention"
                                        }),
                                )
                            },
                        ),
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
                                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
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
            self.sidebar_width
        };
        let sidebar = div()
            .id("sidebar-transition-wrapper")
            .debug_selector(|| "navigation-pane".into())
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .h_full()
            .overflow_hidden()
            .bg(theme_rgb(&self.theme, "sidebar.background"))
            .child(self.render_navigation_header(cx))
            .child(sidebar);
        let sidebar = sidebar.when(!self.layout.sidebar_collapsed, |element| {
            element.child(self.render_sidebar_resize_handle(cx))
        });
        if self.sidebar_animation_id == 0 {
            return sidebar.w(px(sidebar_width)).into_any_element();
        }

        let collapsed = self.layout.sidebar_collapsed;
        let expanded_width = self.sidebar_width;
        let animation_id = self.sidebar_animation_id;
        sidebar
            .with_animation(
                ElementId::Name(format!("sidebar-transition-{animation_id}").into()),
                Animation::new(Duration::from_millis(220)).with_easing(gpui::ease_out_quint()),
                move |element, delta| {
                    element.w(px(sidebar_transition_width(
                        collapsed,
                        delta,
                        expanded_width,
                    )))
                },
            )
            .into_any_element()
    }
}

struct SidebarChatPreview {
    conversation: ConversationItem,
    background: gpui::Rgba,
    foreground: gpui::Rgba,
    muted: gpui::Rgba,
    border: gpui::Rgba,
}

impl Render for SidebarChatPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let conversation = &self.conversation;
        let workspace = conversation.workspace_path.as_deref();
        let status = if conversation.needs_attention {
            Some("Needs input")
        } else if conversation.state == mew_protocol::SessionState::Running {
            Some("Working")
        } else if conversation.last_turn_failed {
            Some("Last turn failed")
        } else if conversation.unread_completion {
            Some("Unread reply")
        } else {
            None
        };
        div()
            .id("sidebar-chat-preview")
            .debug_selector(|| "sidebar-chat-preview".into())
            .w(px(320.))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(self.border)
            .bg(self.background)
            .text_size(px(13.))
            .text_color(self.foreground)
            .role(Role::Tooltip)
            .aria_label(SharedString::from(conversation.title.clone()))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(SharedString::from(
                        compact_session_title(&conversation.title),
                    )))
                    .when_some(
                        session_time_label(conversation.last_message_at),
                        |element, time| {
                            element.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(self.muted)
                                    .child(SharedString::from(time)),
                            )
                        },
                    ),
            )
            .when_some(workspace, |element, path| {
                element.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(self.muted)
                        .child(tabler_icon(TablerIcon::Folder, self.muted, px(13.)))
                        .child(SharedString::from(
                            Path::new(path)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or(path)
                                .to_owned(),
                        )),
                )
            })
            .when_some(conversation.cwd.as_deref(), |element, path| {
                element.child(
                    div()
                        .text_xs()
                        .text_color(self.muted)
                        .child(SharedString::from(display_session_path(path))),
                )
            })
            .when_some(status, |element, status| {
                element.child(div().text_xs().text_color(self.muted).child(status))
            })
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
        .desktop_focus(theme_rgb(theme, "text.accent"))
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

fn workspace_destination_icon(destination: &WorkspaceOpenDestination) -> TablerIcon {
    match destination {
        WorkspaceOpenDestination::Terminal => TablerIcon::Terminal2,
        WorkspaceOpenDestination::CopyPath => TablerIcon::File,
        WorkspaceOpenDestination::DefaultApp | WorkspaceOpenDestination::Application { .. } => {
            TablerIcon::ExternalLink
        }
    }
}

fn workspace_open_menu_item(
    theme: &Theme,
    workspace_path: String,
    destination: WorkspaceOpenDestination,
    selected: bool,
    cx: &mut Context<DesktopShell>,
) -> gpui::AnyElement {
    let label = destination.label().to_owned();
    let icon = workspace_destination_icon(&destination);
    div()
        .id(format!("workspace-open-option-{}", slugify_menu_id(&label)))
        .h(px(28.))
        .flex()
        .items_center()
        .gap(px(7.))
        .px(px(7.))
        .rounded(px(5.))
        .cursor_pointer()
        .role(Role::MenuItem)
        .aria_selected(selected)
        .aria_label(SharedString::from(format!(
            "Open workspace with {}{}",
            label,
            if selected { ", selected" } else { "" }
        )))
        .text_xs()
        .text_color(theme_rgb(theme, "text.muted"))
        .hover(|element| element.bg(theme_rgb(theme, "muted")))
        .on_click(cx.listener(move |shell, _, _, cx| {
            cx.stop_propagation();
            shell.open_workspace_destination(workspace_path.clone(), destination.clone(), cx);
        }))
        .child(tabler_icon(icon, theme_rgb(theme, "text.muted"), px(13.)))
        .child(label)
        .into_any_element()
}

fn workspace_pin_menu_item(
    theme: &Theme,
    workspace_path: String,
    pinned: bool,
    cx: &mut Context<DesktopShell>,
) -> gpui::AnyElement {
    let label = if pinned {
        "Unpin workspace"
    } else {
        "Pin workspace"
    };
    div()
        .id("workspace-pin-option")
        .h(px(28.))
        .flex()
        .items_center()
        .gap(px(7.))
        .px(px(7.))
        .rounded(px(5.))
        .cursor_pointer()
        .role(Role::MenuItem)
        .aria_label(label)
        .text_xs()
        .text_color(theme_rgb(theme, "text.muted"))
        .hover(|element| element.bg(theme_rgb(theme, "muted")))
        .on_click(cx.listener(move |shell, _, _, cx| {
            cx.stop_propagation();
            shell.pin_workspace(workspace_path.clone(), !pinned, cx);
            shell.workspace_open_menu = None;
        }))
        .child(tabler_icon(
            TablerIcon::Pin,
            theme_rgb(theme, "text.muted"),
            px(13.),
        ))
        .child(label)
        .into_any_element()
}

fn slugify_menu_id(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
