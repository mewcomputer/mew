use super::*;

impl Render for DesktopShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_browser_portal(window, cx);
        if self.model_picker_open
            || self.persona_picker_open
            || self.permission_picker_open
            || self.thinking_picker_open
            || self.terminal_font_picker_open
            || self.connection_picker_open
        {
            let focus_handle = self.popover_focus_handle.clone();
            if !focus_handle.is_focused(window) {
                window.defer(cx, move |window, cx| {
                    window.focus(&focus_handle, cx);
                });
            }
        }
        if let Some(portal) = self.browser_portal.as_ref() {
            let visible = self.browser_panel_open && !self.layout.workbench_collapsed;
            let _ = portal.set_visible(BROWSER_OWNER, visible);
        }
        if self.browser_panel_open && self.browser_url_focus_requested {
            self.browser_url_focus_requested = false;
            let focus_handle = self.browser_url_focus_handle.clone();
            window.defer(cx, move |window, cx| {
                window.focus(&focus_handle, cx);
            });
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme_rgb(&self.theme, "background"))
            .text_color(theme_rgb(&self.theme, "text.body"))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|shell, _, _, _| shell.release_browser_focus()),
            )
            .on_key_down(cx.listener(Self::shell_key_down))
            .on_action(cx.listener(Self::action_new_conversation))
            .on_action(cx.listener(Self::action_close_conversation))
            .on_action(cx.listener(Self::action_toggle_sidebar))
            .on_action(cx.listener(Self::action_toggle_terminal))
            .on_action(cx.listener(Self::action_toggle_workbench))
            .on_action(cx.listener(Self::action_dismiss_popovers))
            .child(self.render_topbar(cx))
            .child(if self.settings_open {
                self.render_settings(window, cx)
            } else {
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .gap(px(SHELL_GUTTER))
                    .px(px(SHELL_GUTTER))
                    .pb(px(SHELL_GUTTER))
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .gap(px(SHELL_GUTTER))
                            .child(self.render_center(window, cx))
                            .when(!self.layout.workbench_collapsed, |element| {
                                element.child(self.render_workbench_divider(cx))
                            })
                            .child(self.render_workbench(window, cx)),
                    )
                    .into_any_element()
            })
            .when_some(self.browser_pump.clone(), |element, pump| {
                element.child(pump)
            })
            .child(self.render_picker_overlays(window, cx))
    }
}
