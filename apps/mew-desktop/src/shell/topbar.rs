use super::*;

impl DesktopShell {
    pub(super) fn render_connection_picker(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let profiles = self.remote_profiles.clone();
        let selected = self.connection_profile_selection.clone();
        div()
            .id("connection-profile-picker")
            .absolute()
            .bottom(px(34.))
            .right(px(0.))
            .w(px(300.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .on_key_down(cx.listener(Self::shell_key_down))
            .track_focus(&self.popover_focus_handle)
            .child(
                div()
                    .px(px(8.))
                    .py(px(5.))
                    .text_xs()
                    .text_color(theme_rgb(&self.theme, "text.muted"))
                    .child("connection profile · applies on next launch"),
            )
            .child(
                div()
                    .id("connection-profile-local")
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .p(px(8.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .when(selected.is_none(), |element| {
                        element.bg(theme_rgb(&self.theme, "accent"))
                    })
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.select_connection_profile(None, cx);
                    }))
                    .child(div().text_sm().child("Local daemon"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child("app-owned WebSocket"),
                    ),
            )
            .children(profiles.into_iter().map(|profile| {
                let node_id = profile.node_id.clone();
                let selected = selected.as_deref() == Some(profile.node_id.as_str());
                div()
                    .id(format!("connection-profile-{}", profile.node_id))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .p(px(8.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .when(selected, |element| {
                        element.bg(theme_rgb(&self.theme, "accent"))
                    })
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.select_connection_profile(Some(node_id.clone()), cx);
                    }))
                    .child(div().text_sm().child(SharedString::from(profile.name)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(format!(
                                "{} · {}",
                                profile.device_name, profile.node_id
                            ))),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    pub(super) fn render_topbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(session_id) = self.model.ui.selected_session.as_deref() {
            if let Some(index) = self
                .open_tabs
                .iter()
                .position(|tab| tab.session_id.as_deref() == Some(session_id))
            {
                self.active_tab = Some(index);
            }
        }
        let muted = theme_rgb(&self.theme, "text.muted");
        let body = theme_rgb(&self.theme, "text.body");
        let background = theme_rgb(&self.theme, "background");
        let card = theme_rgb(&self.theme, "card");
        let active_icon = theme_rgb(&self.theme, "text.accent");
        let connection_icon = match self.model.connection.label() {
            "connected" => theme_rgb(&self.theme, "green.fg"),
            "connecting" | "reconnecting" => theme_rgb(&self.theme, "yellow.fg"),
            "connection failed" => theme_rgb(&self.theme, "red.fg"),
            _ => muted,
        };
        if let Some(index) = self.active_tab {
            self.tab_scroll_handle.scroll_to_item(index);
        }
        let tab_scroll_offset = self.tab_scroll_handle.offset().x;
        let tab_scroll_max = self.tab_scroll_handle.max_offset().x;
        let tabs_have_overflow = tab_scroll_max > px(0.);
        let tabs_have_left_overflow = tab_scroll_offset < px(-0.5);
        let tabs_have_right_overflow = tab_scroll_offset > -tab_scroll_max + px(0.5);
        let tabs = self
            .open_tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let selected = self.active_tab == Some(index);
                let title = SharedString::from(tab.title.clone());
                div()
                    .id(format!("conversation-tab-{index}"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .max_w(px(200.))
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .role(Role::Tab)
                    .aria_selected(selected)
                    .aria_label(SharedString::from(format!("Conversation: {}", tab.title)))
                    .aria_keyshortcuts(format!("Command+{}", index + 1))
                    .when(selected, |element| element.bg(card).text_color(body))
                    .when(!selected, |element| element.text_color(muted))
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.select_tab(index, cx);
                    }))
                    .child(tabler_icon(
                        TablerIcon::MessageCircle,
                        if selected { active_icon } else { muted },
                        px(13.),
                    ))
                    .child(div().flex_1().min_w_0().truncate().text_xs().child(title))
                    .child(
                        div()
                            .id(format!("conversation-tab-close-{index}"))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(18.))
                            .rounded(px(4.))
                            .text_color(muted)
                            .role(Role::Button)
                            .aria_label(SharedString::from(format!(
                                "Close conversation: {}",
                                tab.title
                            )))
                            .hover(|element| {
                                element.bg(theme_rgb(&self.theme, "muted")).text_color(body)
                            })
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                cx.stop_propagation();
                                shell.close_tab(index, cx);
                            }))
                            .child(tabler_icon(TablerIcon::X, muted, px(12.))),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .id("shell-header")
            .flex()
            .items_center()
            .h(px(38.))
            .bg(background)
            .pl(px(72.))
            .pr(px(8.))
            .gap(px(6.))
            .child(
                div()
                    .id("sidebar-toggle")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label("Toggle sessions sidebar")
                    .text_color(muted)
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.dispatch_shell_command(ShellCommand::ToggleSidebar, cx);
                    }))
                    .child(tabler_icon(
                        if self.layout.sidebar_collapsed {
                            TablerIcon::PanelLeft
                        } else {
                            TablerIcon::PanelRight
                        },
                        muted,
                        px(14.),
                    )),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div()
                            .id("conversation-tabs")
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .w_full()
                            .h_full()
                            .role(Role::Group)
                            .aria_label("Open conversations")
                            .overflow_x_scroll()
                            .restrict_scroll_to_axis()
                            .track_scroll(&self.tab_scroll_handle)
                            .children(tabs),
                    )
                    .when(tabs_have_overflow && tabs_have_left_overflow, |element| {
                        element.child(div().absolute().left_0().top_0().bottom_0().w(px(24.)).bg(
                            linear_gradient(
                                90.,
                                linear_color_stop(background, 0.),
                                linear_color_stop(background.opacity(0.), 1.),
                            ),
                        ))
                    })
                    .when(tabs_have_overflow && tabs_have_right_overflow, |element| {
                        element.child(div().absolute().right_0().top_0().bottom_0().w(px(24.)).bg(
                            linear_gradient(
                                90.,
                                linear_color_stop(background.opacity(0.), 0.),
                                linear_color_stop(background, 1.),
                            ),
                        ))
                    }),
            )
            .child(
                div()
                    .id("connection-profile-control")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .h(px(26.))
                    .px(px(7.))
                    .rounded(px(6.))
                    .relative()
                    .text_xs()
                    .text_color(muted)
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label("Connection profile")
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.toggle_connection_picker(cx);
                    }))
                    .child(tabler_icon(TablerIcon::GitBranch, connection_icon, px(13.)))
                    .child(SharedString::from(self.model.connection.label().to_owned()))
                    .when(self.connection_picker_open, |element| {
                        element.child(deferred(self.render_connection_picker(cx)).with_priority(3))
                    }),
            )
            .child(
                div()
                    .id("workbench-toggle")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label(if self.layout.workbench_collapsed {
                        "Show workbench"
                    } else {
                        "Hide workbench"
                    })
                    .text_color(muted)
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.dispatch_shell_command(ShellCommand::ToggleWorkbench, cx);
                    }))
                    .child(tabler_icon(
                        if self.layout.workbench_collapsed {
                            TablerIcon::PanelLeft
                        } else {
                            TablerIcon::PanelRight
                        },
                        if self.layout.workbench_collapsed {
                            muted
                        } else {
                            active_icon
                        },
                        px(14.),
                    )),
            )
            .child(
                div()
                    .id("terminal-toggle")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label(if self.layout.terminal_collapsed {
                        "Show terminal"
                    } else {
                        "Hide terminal"
                    })
                    .text_color(if self.layout.terminal_collapsed {
                        muted
                    } else {
                        body
                    })
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.toggle_terminal(cx);
                    }))
                    .child(tabler_icon(
                        TablerIcon::Terminal2,
                        if self.layout.terminal_collapsed {
                            muted
                        } else {
                            active_icon
                        },
                        px(14.),
                    )),
            )
    }
}
