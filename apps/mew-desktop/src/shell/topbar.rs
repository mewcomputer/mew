use super::*;

impl DesktopShell {
    pub(super) fn render_connection_picker(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let profiles = self.remote_profiles.clone();
        let selected = self.connection_profile_selection.clone();
        div()
            .id("connection-profile-picker")
            .absolute()
            .top(px(30.))
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

    fn render_sidebar_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        div()
            .id("sidebar-toggle")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(26.))
            .rounded(px(6.))
            .cursor_pointer()
            .role(Role::Button)
            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
            .aria_label("Toggle chats sidebar")
            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.dispatch_shell_command(ShellCommand::ToggleSidebar, cx);
            }))
            .child(tabler_icon(TablerIcon::PanelLeft, muted, px(14.)))
    }

    pub(super) fn render_navigation_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("navigation-header")
            .debug_selector(|| "navigation-header".into())
            .flex()
            .flex_none()
            .items_center()
            .justify_end()
            .w_full()
            .h(px(PANE_HEADER_HEIGHT))
            .pl(px(72.))
            .pr(px(8.))
            .border_b_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .child(self.render_sidebar_toggle(cx))
    }

    pub(super) fn render_conversation_header(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        let background = theme_rgb(&self.theme, "background");
        let active_icon = theme_rgb(&self.theme, "text.accent");
        let connection_icon = match self.model.connection.label() {
            "connected" => theme_rgb(&self.theme, "green.fg"),
            "connecting" | "reconnecting" => theme_rgb(&self.theme, "yellow.fg"),
            "connection failed" => theme_rgb(&self.theme, "red.fg"),
            _ => muted,
        };
        let selected = self.model.ui.selected_session.as_deref().and_then(|id| {
            self.model
                .ui
                .conversations
                .iter()
                .find(|chat| chat.session_id == id)
        });
        let title = if self.settings_open {
            "Settings".to_owned()
        } else {
            selected
                .map(|chat| chat.title.clone())
                .unwrap_or_else(|| "New chat".into())
        };
        let workspace = selected.and_then(|chat| {
            let path = chat.workspace_path.as_deref().or(chat.cwd.as_deref())?;
            Some(
                self.model
                    .ui
                    .projects
                    .iter()
                    .find(|project| project.path == path)
                    .map(|project| project.display_name.clone())
                    .unwrap_or_else(|| {
                        Path::new(path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(path)
                            .to_owned()
                    }),
            )
        });
        div()
            .id("conversation-header")
            .debug_selector(|| "conversation-header".into())
            .flex()
            .items_center()
            .h(px(PANE_HEADER_HEIGHT))
            .flex_none()
            .bg(background)
            .px(px(8.))
            .gap(px(8.))
            .border_b_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .when(self.layout.sidebar_collapsed, |element| {
                element.pl(px(72.)).child(self.render_sidebar_toggle(cx))
            })
            .child(
                div()
                    .id("conversation-context")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(8.))
                    .text_sm()
                    .when_some(
                        workspace.filter(|_| !self.settings_open),
                        |element, workspace| {
                            element
                                .child(tabler_icon(TablerIcon::Folder, muted, px(14.)))
                                .child(
                                    div()
                                        .max_w(px(80.))
                                        .min_w_0()
                                        .truncate()
                                        .text_color(muted)
                                        .child(SharedString::from(workspace)),
                                )
                                .child(div().text_color(muted).child("/"))
                        },
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(SharedString::from(title)),
                    ),
            )
            .child(
                div()
                    .id("connection-profile-control")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(6.))
                    .relative()
                    .text_xs()
                    .text_color(muted)
                    .cursor_pointer()
                    .role(Role::Button)
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                    .aria_label(SharedString::from(format!(
                        "Connection profile: {}",
                        self.model.connection.label()
                    )))
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.toggle_connection_picker(cx);
                    }))
                    .child(tabler_icon(TablerIcon::GitBranch, connection_icon, px(13.)))
                    .when(self.connection_picker_open, |element| {
                        element.child(deferred(self.render_connection_picker(cx)).with_priority(3))
                    }),
            )
            .when(
                self.model.session_is_ready()
                    && !self.session_requests.is_pending()
                    && matches!(
                        self.model.connection,
                        crate::model::ShellConnection::Connected
                    ),
                |element| {
                    element.child(
                        div()
                            .id("review-changes")
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(26.))
                            .rounded(px(6.))
                            .text_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .role(Role::Button)
                            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                            .aria_label("Review changes in this conversation")
                            .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                            .on_click(cx.listener(|shell, _, _, cx| shell.open_review(cx)))
                            .child(tabler_icon(TablerIcon::FileCode, muted, px(13.))),
                    )
                },
            )
            .child(
                div()
                    .id("workbench-toggle")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .role(Role::Button)
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                    .aria_label(if self.layout.workbench_collapsed {
                        "Show work pane"
                    } else {
                        "Hide work pane"
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
            .when(
                self.layout.workbench_collapsed || self.settings_open,
                |element| element.child(self.render_terminal_toggle(cx)),
            )
    }

    pub(super) fn render_terminal_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        let active_icon = theme_rgb(&self.theme, "text.accent");
        div()
            .id("terminal-toggle")
            .debug_selector(|| "terminal-toggle".into())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(26.))
            .rounded(px(6.))
            .cursor_pointer()
            .role(Role::Button)
            .desktop_focus(theme_rgb(&self.theme, "text.accent"))
            .aria_label(if self.layout.terminal_collapsed {
                "Show terminal"
            } else {
                "Hide terminal"
            })
            .text_color(if self.layout.terminal_collapsed {
                muted
            } else {
                active_icon
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
            ))
    }
}
