use super::*;

impl DesktopShell {
    pub(super) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let muted = theme_rgb(&self.theme, "text.muted");
        let body = theme_rgb(&self.theme, "text.body");
        let panel = theme_rgb(&self.theme, "panel.background");
        let card = theme_rgb(&self.theme, "card");
        let divider = theme_rgb(&self.theme, "divider");
        let system_theme = theme_name(window.appearance());
        let theme_names = Theme::list_available();
        let light_theme_options = theme_names
            .iter()
            .filter(|name| Theme::load(name).mode == mew_tui::theme::ThemeMode::Light)
            .map(|name| {
                let selected = self.light_theme == *name;
                let theme_name = name.clone();
                div()
                    .id(format!("settings-light-theme-{name}"))
                    .px(px(9.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label(SharedString::from(format!("Use light theme {name}")))
                    .aria_selected(selected)
                    .text_xs()
                    .when(selected, |element| element.bg(card).text_color(body))
                    .when(!selected, |element| element.text_color(muted))
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.choose_theme_variant(true, theme_name.clone(), window, cx);
                    }))
                    .child(SharedString::from(name.clone()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let dark_theme_options = theme_names
            .iter()
            .filter(|name| Theme::load(name).mode == mew_tui::theme::ThemeMode::Dark)
            .map(|name| {
                let selected = self.dark_theme == *name;
                let theme_name = name.clone();
                div()
                    .id(format!("settings-dark-theme-{name}"))
                    .px(px(9.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label(SharedString::from(format!("Use dark theme {name}")))
                    .aria_selected(selected)
                    .text_xs()
                    .when(selected, |element| element.bg(card).text_color(body))
                    .when(!selected, |element| element.text_color(muted))
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.choose_theme_variant(false, theme_name.clone(), window, cx);
                    }))
                    .child(SharedString::from(name.clone()))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let theme_modes = [
            DesktopThemeMode::System,
            DesktopThemeMode::Light,
            DesktopThemeMode::Dark,
        ]
        .into_iter()
        .map(|mode| {
            let selected = self.theme_mode == mode;
            div()
                .id(format!(
                    "settings-theme-mode-{}",
                    mode.label().to_lowercase()
                ))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(7.))
                .cursor_pointer()
                .role(Role::Button)
                .aria_label(SharedString::from(format!(
                    "Use {} theme mode",
                    mode.label()
                )))
                .when(selected, |element| element.bg(card).text_color(body))
                .when(!selected, |element| element.text_color(muted))
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.choose_theme_mode(mode, window, cx);
                }))
                .child(mode.label())
                .into_any_element()
        })
        .collect::<Vec<_>>();
        let endpoint = self
            .model
            .endpoint
            .clone()
            .unwrap_or_else(|| "local workspace".into());
        let terminal_options = TERMINAL_FONT_CHOICES.iter().map(|(family, description)| {
            let family = *family;
            let description = *description;
            let selected = self.terminal_font_family == family;
            div()
                .id(format!("settings-terminal-font-{family}"))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .px(px(12.))
                .py(px(9.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(selected, |element| element.bg(card))
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.choose_terminal_font(family, cx);
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_sm().child(SharedString::from(family)))
                        .child(div().text_xs().text_color(muted).child(description)),
                )
                .when(selected, |element| {
                    element.child(
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(theme_rgb(&self.theme, "text.accent")),
                    )
                })
                .into_any_element()
        });
        let sidebar_state = if self.layout.sidebar_collapsed {
            "Hidden"
        } else {
            "Shown"
        };
        let workbench_state = if self.layout.workbench_collapsed {
            "Hidden"
        } else {
            "Shown"
        };
        let terminal_state = if self.layout.terminal_collapsed {
            "Collapsed"
        } else {
            "Expanded"
        };
        let settings_page = self.settings_page;
        let settings_navigation = [
            (SettingsPage::General, TablerIcon::SlidersHorizontal),
            (SettingsPage::Terminal, TablerIcon::Terminal2),
            (SettingsPage::Workspace, TablerIcon::PanelLeft),
            (SettingsPage::Connection, TablerIcon::GitBranch),
        ]
        .into_iter()
        .map(|(page, icon)| {
            let selected = settings_page == page;
            div()
                .id(format!("settings-nav-{}", page.key()))
                .flex()
                .items_center()
                .gap(px(9.))
                .px(px(10.))
                .py(px(9.))
                .rounded(px(8.))
                .cursor_pointer()
                .role(Role::Tab)
                .aria_selected(selected)
                .aria_label(SharedString::from(format!(
                    "Settings: {}{}",
                    page.title(),
                    if selected { ", selected" } else { "" }
                )))
                .when(selected, |element| element.bg(card).text_color(body))
                .when(!selected, |element| element.text_color(muted))
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.select_settings_page(page, cx);
                }))
                .child(tabler_icon(
                    icon,
                    if selected { body } else { muted },
                    px(15.),
                ))
                .child(page.title())
                .into_any_element()
        });

        div()
            .id("settings-page")
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .role(Role::Group)
            .aria_label("Settings")
            .bg(theme_rgb(&self.theme, "sidebar.background"))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .justify_center()
                    .w_full()
                    .min_w_0()
                    .p(px(40.))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(1120.))
                            .flex()
                            .items_start()
                            .gap(px(32.))
                            .child(
                                div()
                                    .id("settings-navigation")
                                    .w(px(196.))
                                    .flex_none()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .p(px(8.))
                                    .rounded(px(12.))
                                    .border_1()
                                    .border_color(divider)
                                    .bg(panel)
                                    .children(settings_navigation),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(820.))
                            .flex()
                            .flex_col()
                            .gap(px(24.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(
                                        div()
                                            .id("settings-back")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(32.))
                                            .rounded(px(8.))
                                            .cursor_pointer()
                                            .role(Role::Button)
                                            .aria_label("Back to conversation")
                                            .text_color(muted)
                                            .hover(|element| element.bg(theme_rgb(
                                                &self.theme,
                                                "muted",
                                            )))
                                            .on_click(cx.listener(|shell, _, _, cx| {
                                                shell.toggle_settings(cx);
                                            }))
                                            .child(tabler_icon(
                                                TablerIcon::ArrowLeft,
                                                muted,
                                                px(16.),
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(3.))
                                            .child(div().text_xl().child(settings_page.title()))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .child(settings_page.description()),
                                            ),
                                    ),
                            )
                            .when(settings_page == SettingsPage::General, |element| {
                                element.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child("APPEARANCE"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(12.))
                                            .px(px(16.))
                                            .py(px(14.))
                                            .rounded(px(12.))
                                            .border_1()
                                            .border_color(divider)
                                            .bg(panel)
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(px(3.))
                                                            .child(div().text_sm().child("Theme"))
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(muted)
                                                                    .child(SharedString::from(
                                                                        format!(
                                                                            "{} · system is currently {}",
                                                                            self.theme_mode.label(),
                                                                            system_theme
                                                                        ),
                                                                    )),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .items_center()
                                                            .gap(px(3.))
                                                            .p(px(3.))
                                                            .rounded(px(9.))
                                                            .bg(theme_rgb(&self.theme, "muted"))
                                                            .children(theme_modes),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(6.))
                                                    .child(div().text_xs().text_color(muted).child("LIGHT THEME"))
                                                    .child(div().flex().flex_wrap().gap(px(4.)).children(light_theme_options))
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(6.))
                                                    .child(div().text_xs().text_color(muted).child("DARK THEME"))
                                                    .child(div().flex().flex_wrap().gap(px(4.)).children(dark_theme_options))
                                            ),
                                    )
                                )
                            })
                            .when(settings_page == SettingsPage::Terminal, |element| {
                                element.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child("TERMINAL"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .p(px(6.))
                                            .rounded(px(12.))
                                            .border_1()
                                            .border_color(divider)
                                            .bg(panel)
                                            .children(terminal_options),
                                    )
                                )
                            })
                            .when(settings_page == SettingsPage::Workspace, |element| {
                                element.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child("WORKSPACE LAYOUT"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .rounded(px(12.))
                                            .border_1()
                                            .border_color(divider)
                                            .bg(panel)
                                            .child(
                                                div()
                                                    .id("settings-sidebar-row")
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(16.))
                                                    .py(px(13.))
                                                    .cursor_pointer()
                                                    .role(Role::Button)
                                                    .aria_label(SharedString::from(format!(
                                                        "Toggle navigation rail, currently {sidebar_state}"
                                                    )))
                                                    .hover(|element| {
                                                        element.bg(theme_rgb(&self.theme, "muted"))
                                                    })
                                                    .on_click(cx.listener(|shell, _, _, cx| {
                                                        shell.toggle_sidebar(cx);
                                                    }))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(px(3.))
                                                            .child(div().text_sm().child("Navigation rail"))
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(muted)
                                                                    .child("Show the conversation list"),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted)
                                                            .child(sidebar_state),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id("settings-workbench-row")
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(16.))
                                                    .py(px(13.))
                                                    .border_t_1()
                                                    .border_color(divider)
                                                    .cursor_pointer()
                                                    .role(Role::Button)
                                                    .aria_label(SharedString::from(format!(
                                                        "Toggle review workbench, currently {workbench_state}"
                                                    )))
                                                    .hover(|element| {
                                                        element.bg(theme_rgb(&self.theme, "muted"))
                                                    })
                                                    .on_click(cx.listener(|shell, _, _, cx| {
                                                        shell.toggle_workbench(cx);
                                                    }))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(px(3.))
                                                            .child(div().text_sm().child("Review workbench"))
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(muted)
                                                                    .child("Show changes, browser, and activity"),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted)
                                                            .child(workbench_state),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id("settings-terminal-row")
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .px(px(16.))
                                                    .py(px(13.))
                                                    .border_t_1()
                                                    .border_color(divider)
                                                    .cursor_pointer()
                                                    .role(Role::Button)
                                                    .aria_label(SharedString::from(format!(
                                                        "Toggle terminal panel, currently {terminal_state}"
                                                    )))
                                                    .hover(|element| {
                                                        element.bg(theme_rgb(&self.theme, "muted"))
                                                    })
                                                    .on_click(cx.listener(|shell, _, _, cx| {
                                                        shell.toggle_terminal(cx);
                                                    }))
                                                    .child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(px(3.))
                                                            .child(div().text_sm().child("Terminal panel"))
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(muted)
                                                                    .child("Keep the terminal open below chat"),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted)
                                                    .child(terminal_state),
                                                ),
                                            ),
                                    ),
                                )
                            })
                            .when(settings_page == SettingsPage::Connection, |element| {
                                element.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .child(div().text_xs().text_color(muted).child("CONNECTION"))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .px(px(16.))
                                            .py(px(14.))
                                            .rounded(px(12.))
                                            .border_1()
                                            .border_color(divider)
                                            .bg(panel)
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(3.))
                                                    .child(div().text_sm().child("Daemon connection"))
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted)
                                                            .child(SharedString::from(endpoint)),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .child(SharedString::from(
                                                        self.model.connection.label().to_owned(),
                                                    )),
                                            ),
                                    )
                                )
                            })
                    ),
            )
            )
            .into_any_element()
    }
}
