use super::*;

impl DesktopShell {
    fn persist_theme_preferences(&mut self) {
        let Ok(mut state) = mew_config::load_state() else {
            self.model.last_error = Some("could not load theme preferences".into());
            return;
        };
        state.desktop_theme_mode = match self.theme_mode {
            DesktopThemeMode::System => "system",
            DesktopThemeMode::Light => "light",
            DesktopThemeMode::Dark => "dark",
        }
        .into();
        state.desktop_light_theme = self.light_theme.clone();
        state.desktop_dark_theme = self.dark_theme.clone();
        if let Err(error) = mew_config::save_state(&state) {
            self.model.last_error = Some(format!("could not save theme preferences: {error}"));
        }
    }

    pub(super) fn choose_theme_mode(
        &mut self,
        mode: DesktopThemeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.theme_mode = mode;
        self.persist_theme_preferences();
        self.reload_theme(window.appearance(), cx);
    }

    pub(super) fn choose_theme_variant(
        &mut self,
        light: bool,
        theme_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if light {
            self.light_theme = theme_name;
        } else {
            self.dark_theme = theme_name;
        }
        self.persist_theme_preferences();
        self.reload_theme(window.appearance(), cx);
    }

    pub(super) fn select_connection_profile(
        &mut self,
        node_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Ok(mut state) = mew_config::load_state() else {
            self.model.last_error = Some("could not load desktop connection state".into());
            cx.notify();
            return;
        };
        if let Some(node_id) = node_id.as_deref() {
            if let Some(profile) = self
                .remote_profiles
                .iter()
                .find(|profile| profile.node_id == node_id)
            {
                if !state
                    .desktop_remote_profiles
                    .iter()
                    .any(|saved| saved.node_id == node_id)
                {
                    state.desktop_remote_profiles.push(profile.clone());
                }
            }
        }
        state.desktop_active_remote_profile = node_id.clone();
        if let Err(error) = mew_config::save_state(&state) {
            self.model.last_error = Some(format!("could not save connection profile: {error}"));
            cx.notify();
            return;
        }
        self.connection_profile_selection = node_id;
        self.connection_picker_open = false;
        self.model.last_error = Some("connection profile saved · restart mew to apply".into());
        cx.notify();
    }

    pub(super) fn choose_model(
        &mut self,
        provider: String,
        model: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model_picker_open = false;
        self.terminal_font_picker_open = false;
        window.focus(&self.composer_focus_handle, cx);
        if self.model.session_is_ready() {
            self.send_command(ClientMessage::SwitchModel { provider, model });
        } else {
            self.pending_model = Some((provider, model));
            if !self.pending_session_request {
                self.pending_session_request = true;
                self.pending_session_target = None;
                let command = self.model.ui.new_conversation(None);
                self.send_command(command);
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model_picker_open = !self.model_picker_open;
        self.persona_picker_open = false;
        self.permission_picker_open = false;
        self.thinking_picker_open = false;
        self.terminal_font_picker_open = false;
        if self.model_picker_open {
            self.model_picker_query.clear();
            self.model_picker_selection = 0..0;
            self.model_picker_selection_reversed = false;
            self.model_picker_marked_range = None;
            window.focus(&self.model_picker_focus_handle, cx);
        } else {
            window.focus(&self.composer_focus_handle, cx);
        }
        cx.notify();
    }

    pub(super) fn model_picker_search_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.close_shell_popovers();
            window.focus(&self.composer_focus_handle, cx);
            cx.notify();
            cx.stop_propagation();
        } else if key == "a" && event.keystroke.modifiers.platform {
            self.model_picker_selection = 0..self.model_picker_query.len();
            self.model_picker_selection_reversed = false;
            cx.notify();
            cx.stop_propagation();
        } else if key == "backspace" {
            self.model_picker_query_backspace(window, cx);
            cx.stop_propagation();
        } else if key == "delete" {
            self.model_picker_query_delete(window, cx);
            cx.stop_propagation();
        } else if key == "left" {
            self.model_picker_query_move_left(cx);
            cx.stop_propagation();
        } else if key == "right" {
            self.model_picker_query_move_right(cx);
            cx.stop_propagation();
        } else if key == "enter" {
            if let Some(index) = self.model_picker_filtered_indices.first().copied() {
                if let Some(model) = self.model.ui.models.get(index) {
                    self.choose_model(model.provider.clone(), model.model.clone(), window, cx);
                }
            }
            cx.stop_propagation();
        } else if key == "v" && event.keystroke.modifiers.platform {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    self.replace_model_picker_query_text(None, &text, cx);
                }
            }
            cx.stop_propagation();
        } else if !event.keystroke.modifiers.platform
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
        {
            if let Some(text) = event.keystroke.key_char.as_deref() {
                self.replace_model_picker_query_text(None, text, cx);
                cx.stop_propagation();
            }
        }
    }

    pub(super) fn model_picker_search_mouse_down(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.model_picker_focus_handle, cx);
        self.model_picker_selection = 0..self.model_picker_query.len();
        self.model_picker_selection_reversed = false;
        cx.notify();
    }

    fn model_picker_query_cursor_offset(&self) -> usize {
        if self.model_picker_selection_reversed {
            self.model_picker_selection.start
        } else {
            self.model_picker_selection.end
        }
    }

    fn model_picker_query_backspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_picker_selection.is_empty() {
            let cursor = self.model_picker_query_cursor_offset();
            let start = previous_utf8_boundary(&self.model_picker_query, cursor);
            if start == cursor {
                window.play_system_bell();
                return;
            }
            self.model_picker_selection = start..cursor;
        }
        self.replace_model_picker_query_text(None, "", cx);
    }

    fn model_picker_query_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_picker_selection.is_empty() {
            let cursor = self.model_picker_query_cursor_offset();
            let end = next_utf8_boundary(&self.model_picker_query, cursor);
            if end == cursor {
                window.play_system_bell();
                return;
            }
            self.model_picker_selection = cursor..end;
        }
        self.replace_model_picker_query_text(None, "", cx);
    }

    fn model_picker_query_move_left(&mut self, cx: &mut Context<Self>) {
        let offset = if self.model_picker_selection.is_empty() {
            previous_utf8_boundary(
                &self.model_picker_query,
                self.model_picker_query_cursor_offset(),
            )
        } else {
            self.model_picker_selection.start
        };
        self.model_picker_selection = offset..offset;
        self.model_picker_selection_reversed = false;
        cx.notify();
    }

    fn model_picker_query_move_right(&mut self, cx: &mut Context<Self>) {
        let offset = if self.model_picker_selection.is_empty() {
            next_utf8_boundary(
                &self.model_picker_query,
                self.model_picker_query_cursor_offset(),
            )
        } else {
            self.model_picker_selection.end
        };
        self.model_picker_selection = offset..offset;
        self.model_picker_selection_reversed = false;
        cx.notify();
    }

    pub(super) fn replace_model_picker_query_text(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        cx: &mut Context<Self>,
    ) {
        let current = self.model_picker_query.clone();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(&current, range.start)
                    ..byte_offset_for_utf16(&current, range.end)
            })
            .or_else(|| self.model_picker_marked_range.clone())
            .unwrap_or_else(|| self.model_picker_selection.clone());
        let mut updated = current;
        updated.replace_range(range.clone(), replacement);
        let cursor = range.start + replacement.len();
        self.model_picker_selection = cursor..cursor;
        self.model_picker_selection_reversed = false;
        self.model_picker_marked_range = None;
        self.model_picker_query = updated;
        cx.notify();
    }

    pub(super) fn replace_model_picker_query_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        replacement: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        let current = self.model_picker_query.clone();
        let range = range_utf16
            .as_ref()
            .map(|range| {
                byte_offset_for_utf16(&current, range.start)
                    ..byte_offset_for_utf16(&current, range.end)
            })
            .or_else(|| self.model_picker_marked_range.clone())
            .unwrap_or_else(|| self.model_picker_selection.clone());
        let mut updated = current;
        updated.replace_range(range.clone(), replacement);
        let replacement_end = range.start + replacement.len();
        self.model_picker_marked_range =
            (!replacement.is_empty()).then_some(range.start..replacement_end);
        self.model_picker_selection = new_selected_range_utf16
            .map(|new_range| {
                let start = byte_offset_for_utf16(replacement, new_range.start);
                let end = byte_offset_for_utf16(replacement, new_range.end);
                range.start + start..range.start + end
            })
            .unwrap_or(replacement_end..replacement_end);
        self.model_picker_selection_reversed = false;
        self.model_picker_query = updated;
        cx.notify();
    }

    pub(super) fn choose_persona(&mut self, name: String, cx: &mut Context<Self>) {
        self.persona_picker_open = false;
        self.terminal_font_picker_open = false;
        if self.model.session_is_ready() {
            self.send_command(ClientMessage::SwitchPersona { name });
        }
        cx.notify();
    }

    pub(super) fn toggle_persona_picker(&mut self, cx: &mut Context<Self>) {
        self.persona_picker_open = !self.persona_picker_open;
        self.model_picker_open = false;
        self.permission_picker_open = false;
        self.thinking_picker_open = false;
        self.terminal_font_picker_open = false;
        cx.notify();
    }

    pub(super) fn toggle_permission_picker(&mut self, cx: &mut Context<Self>) {
        self.permission_picker_open = !self.permission_picker_open;
        self.model_picker_open = false;
        self.persona_picker_open = false;
        self.thinking_picker_open = false;
        self.terminal_font_picker_open = false;
        cx.notify();
    }

    pub(super) fn choose_permission_mode(&mut self, mode: String, cx: &mut Context<Self>) {
        self.permission_picker_open = false;
        if self.model.session_is_ready() {
            self.send_command(ClientMessage::SetPermissionMode { mode });
        }
        cx.notify();
    }

    pub(super) fn toggle_thinking_picker(&mut self, cx: &mut Context<Self>) {
        self.thinking_picker_open = !self.thinking_picker_open;
        self.model_picker_open = false;
        self.persona_picker_open = false;
        self.permission_picker_open = false;
        self.terminal_font_picker_open = false;
        cx.notify();
    }

    /// `variant` of `None` disables thinking (the daemon treats an empty
    /// string the same as "none").
    pub(super) fn choose_thinking_variant(
        &mut self,
        variant: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.thinking_picker_open = false;
        if self.model.session_is_ready() {
            self.send_command(ClientMessage::SetThinkingVariant {
                variant: variant.unwrap_or_default(),
            });
        }
        cx.notify();
    }

    pub(super) fn toggle_terminal_font_picker(&mut self, cx: &mut Context<Self>) {
        self.terminal_font_picker_open = !self.terminal_font_picker_open;
        self.model_picker_open = false;
        self.persona_picker_open = false;
        self.permission_picker_open = false;
        self.thinking_picker_open = false;
        cx.notify();
    }

    pub(super) fn choose_terminal_font(&mut self, family: &'static str, cx: &mut Context<Self>) {
        self.terminal_font_family = family.to_owned();
        self.terminal_font_picker_open = false;
        self.persist_layout();
        self.terminal_view.update(cx, |view, cx| {
            view.set_font_family(family, cx);
        });
        cx.notify();
    }

    fn render_model_option(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let model = self.model.ui.models.get(index)?;
        let provider = model.provider.clone();
        let model_id = model.model.clone();
        let selected = self.model.ui.current_provider.as_deref() == Some(model.provider.as_str())
            && self.model.ui.model.as_deref() == Some(model.model.as_str());
        let label = SharedString::from(model.id.clone());
        let description = model.description.clone();
        Some(
            div()
                .id(format!("model-option-{}", model.id))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(3.))
                .h_auto()
                .min_h(px(54.))
                .w_full()
                .min_w_0()
                .p(px(10.))
                .rounded(px(7.))
                .cursor_pointer()
                .role(Role::MenuItem)
                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                .aria_label(label.clone())
                .aria_selected(selected)
                .when(selected, |element| {
                    element.bg(theme_rgb(&self.theme, "accent"))
                })
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.choose_model(provider.clone(), model_id.clone(), window, cx);
                }))
                .child(div().min_w_0().whitespace_normal().text_sm().child(label))
                .when_some(
                    description.map(SharedString::from),
                    |element, description| {
                        element.child(
                            div()
                                .min_w_0()
                                .whitespace_normal()
                                .text_xs()
                                .text_color(theme_rgb(&self.theme, "text.muted"))
                                .child(description),
                        )
                    },
                )
                .into_any_element(),
        )
    }

    fn render_model_option_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        range
            .filter_map(|index| {
                self.model_picker_filtered_indices
                    .get(index)
                    .copied()
                    .and_then(|model_index| self.render_model_option(model_index, cx))
            })
            .collect()
    }

    pub(super) fn render_model_picker(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.model_picker_query.clone();
        self.model_picker_filtered_indices = self
            .model
            .ui
            .models
            .iter()
            .enumerate()
            .filter(|(_, model)| {
                model_matches_query(
                    &model.id,
                    &model.provider,
                    &model.model,
                    model.description.as_deref(),
                    &query,
                )
            })
            .map(|(index, _)| index)
            .collect();
        let option_count = self.model_picker_filtered_indices.len();
        let options = if option_count > 0 {
            gpui::uniform_list(
                "model-picker-list",
                option_count,
                cx.processor(Self::render_model_option_rows),
            )
            .flex_1()
            .min_h_0()
            .into_any_element()
        } else {
            let message = if query.trim().is_empty() {
                "No models reported by the daemon"
            } else {
                "No models match your search"
            };
            div()
                .p(px(8.))
                .text_sm()
                .text_color(theme_rgb(&self.theme, "text.muted"))
                .child(message)
                .into_any_element()
        };
        let search_focus_handle = self.model_picker_focus_handle.clone();
        let clear_search = (!self.model_picker_query.is_empty()).then(|| {
            div()
                .id("clear-model-picker-search")
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded(px(5.))
                .cursor_pointer()
                .role(Role::Button)
                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                .aria_label("Clear model search")
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(|shell, _, _, cx| {
                    cx.stop_propagation();
                    shell.model_picker_query.clear();
                    shell.model_picker_selection = 0..0;
                    shell.model_picker_selection_reversed = false;
                    shell.model_picker_marked_range = None;
                    cx.notify();
                }))
                .child(tabler_icon(
                    TablerIcon::X,
                    theme_rgb(&self.theme, "text.muted"),
                    px(12.),
                ))
        });
        div()
            .id("model-picker")
            .w(px(300.))
            .flex()
            .flex_col()
            .h(model_picker_height(option_count))
            .p(px(8.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .rounded(px(10.))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .role(Role::Menu)
            .aria_label("Choose model")
            .on_key_down(cx.listener(Self::shell_key_down))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .track_focus(&self.popover_focus_handle)
            .child(
                div()
                    .id("model-picker-search-frame")
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
                            .id("model-picker-search-input-frame")
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .track_focus(&search_focus_handle)
                            .key_context("ModelPickerSearch")
                            .role(Role::TextInput)
                            .aria_label("Filter models")
                            .cursor(gpui::CursorStyle::IBeam)
                            .focus_visible(|element| {
                                element
                                    .border_1()
                                    .border_color(theme_rgb(&self.theme, "accent"))
                            })
                            .on_key_down(cx.listener(Self::model_picker_search_key_down))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|shell, _, window, cx| {
                                    shell.model_picker_search_mouse_down(window, cx);
                                }),
                            )
                            .child(ComposerElement {
                                shell: cx.entity(),
                                target: TextInputTarget::ModelSearch,
                            }),
                    )
                    .when_some(clear_search, |element, clear_search| {
                        element.child(clear_search)
                    }),
            )
            .child(options)
    }

    fn render_persona_option(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let persona = self.model.ui.personas.get(index)?;
        let name = persona.name.clone();
        let description = persona.description.clone();
        let persona_name = name.clone();
        let selected = self.model.ui.current_persona.as_deref() == Some(name.as_str());
        Some(
            div()
                .id(format!("persona-option-{name}"))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(3.))
                .h_auto()
                .min_h(px(54.))
                .min_w_0()
                .p(px(10.))
                .rounded(px(7.))
                .cursor_pointer()
                .role(Role::MenuItem)
                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                .aria_label(SharedString::from(name.clone()))
                .aria_selected(selected)
                .when(selected, |element| {
                    element.bg(theme_rgb(&self.theme, "accent"))
                })
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.choose_persona(persona_name.clone(), cx);
                }))
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .text_sm()
                        .child(SharedString::from(name)),
                )
                .child(
                    div()
                        .min_w_0()
                        .whitespace_normal()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(SharedString::from(description)),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_persona_picker(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let option_count = self.model.ui.personas.len();
        let options = self
            .model
            .ui
            .personas
            .iter()
            .enumerate()
            .filter_map(|(index, _)| self.render_persona_option(index, cx))
            .collect::<Vec<_>>();
        let has_options = !options.is_empty();
        div()
            .id("persona-picker")
            .w(px(280.))
            .flex()
            .flex_col()
            .h_auto()
            .max_h(persona_picker_height(option_count))
            .overflow_y_scroll()
            .p(px(8.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .rounded(px(10.))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .role(Role::Menu)
            .aria_label("Choose persona")
            .on_key_down(cx.listener(Self::shell_key_down))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .track_focus(&self.popover_focus_handle)
            .when(has_options, |element| element.children(options))
            .when(!has_options, |element| {
                element.child(
                    div()
                        .p(px(8.))
                        .text_sm()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child("No personas reported by the daemon"),
                )
            })
    }

    pub(super) fn render_permission_picker(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.model.ui.permission_mode.clone();
        let options = PERMISSION_MODES
            .iter()
            .map(|(id, label, description)| {
                let selected = current.as_deref() == Some(*id);
                let mode = (*id).to_owned();
                div()
                    .id(format!("permission-option-{id}"))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(3.))
                    .h_auto()
                    .min_h(px(54.))
                    .min_w_0()
                    .p(px(10.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .role(Role::MenuItem)
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                    .aria_label(SharedString::from(*label))
                    .aria_selected(selected)
                    .when(selected, |element| {
                        element.bg(theme_rgb(&self.theme, "accent"))
                    })
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.choose_permission_mode(mode.clone(), cx);
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .whitespace_normal()
                            .text_sm()
                            .child(SharedString::from(*label)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(*description)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .id("permission-picker")
            .w(px(300.))
            .flex()
            .flex_col()
            .h_auto()
            .max_h(persona_picker_height(PERMISSION_MODES.len()))
            .overflow_y_scroll()
            .p(px(8.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .rounded(px(10.))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .role(Role::Menu)
            .aria_label("Choose permission mode")
            .on_key_down(cx.listener(Self::shell_key_down))
            .track_focus(&self.popover_focus_handle)
            .children(options)
    }

    pub(super) fn render_thinking_picker(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.model.ui.thinking_variant.clone();
        let variants = thinking_variants_for_model(
            &self.model.ui.models,
            self.model.ui.current_provider.as_deref(),
            self.model.ui.model.as_deref(),
        );
        let render_option = |label: String, variant: Option<String>, cx: &mut Context<Self>| {
            let selected = current == variant;
            div()
                .id(format!(
                    "thinking-option-{}",
                    variant.as_deref().unwrap_or("off")
                ))
                .flex()
                .items_center()
                .min_h(px(34.))
                .p(px(9.))
                .rounded(px(7.))
                .cursor_pointer()
                .role(Role::MenuItem)
                .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                .aria_label(SharedString::from(label.clone()))
                .aria_selected(selected)
                .when(selected, |element| {
                    element.bg(theme_rgb(&self.theme, "accent"))
                })
                .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.choose_thinking_variant(variant.clone(), cx);
                }))
                .child(div().text_sm().child(SharedString::from(label)))
                .into_any_element()
        };
        let mut options = vec![render_option("off".into(), None, cx)];
        options.extend(
            variants
                .iter()
                .map(|variant| render_option(variant.clone(), Some(variant.clone()), cx)),
        );
        let option_count = options.len();

        div()
            .id("thinking-picker")
            .w(px(220.))
            .flex()
            .flex_col()
            .h_auto()
            .max_h(persona_picker_height(option_count))
            .overflow_y_scroll()
            .p(px(8.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .rounded(px(10.))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .role(Role::Menu)
            .aria_label("Choose thinking variant")
            .on_key_down(cx.listener(Self::shell_key_down))
            .track_focus(&self.popover_focus_handle)
            .when(option_count > 1, |element| element.children(options))
            .when(option_count == 1, |element| {
                element.child(
                    div()
                        .p(px(8.))
                        .text_sm()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child("No thinking variants for this model"),
                )
            })
    }

    pub(super) fn render_slash_menu(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = self.slash_menu_matches();
        let selected = self.slash_menu_index.min(matches.len().saturating_sub(1));
        div()
            .id("slash-menu")
            .w(px(320.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .rounded(px(10.))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .role(Role::Menu)
            .aria_label("Slash commands")
            .on_key_down(cx.listener(Self::shell_key_down))
            .track_focus(&self.popover_focus_handle)
            .children(matches.iter().enumerate().map(|(index, def)| {
                let name = def.name;
                let description = def.description;
                div()
                    .id(format!("slash-option-{}", name.trim_start_matches('/')))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .p(px(9.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .role(Role::MenuItem)
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                    .aria_label(SharedString::from(name))
                    .aria_selected(index == selected)
                    .when(index == selected, |element| {
                        element.bg(theme_rgb(&self.theme, "accent"))
                    })
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.complete_slash_command(name, cx);
                    }))
                    .child(div().text_sm().child(SharedString::from(name)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(description)),
                    )
                    .into_any_element()
            }))
    }

    pub(super) fn render_mention_menu(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = self.mention_menu_matches();
        let selected = self.mention_menu_index.min(matches.len().saturating_sub(1));
        div()
            .id("mention-menu")
            .w(px(360.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .rounded(px(10.))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .role(Role::Menu)
            .aria_label("Mention a workspace file")
            .on_key_down(cx.listener(Self::shell_key_down))
            .track_focus(&self.popover_focus_handle)
            .children(matches.iter().enumerate().map(|(index, path)| {
                let path = path.clone();
                let click_path = path.clone();
                let name = path.rsplit('/').next().unwrap_or(path.as_str()).to_owned();
                div()
                    .id(format!("mention-option-{index}"))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(8.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .role(Role::MenuItem)
                    .desktop_focus(theme_rgb(&self.theme, "text.accent"))
                    .aria_label(SharedString::from(format!("Mention {path}")))
                    .aria_selected(index == selected)
                    .when(index == selected, |element| {
                        element.bg(theme_rgb(&self.theme, "accent"))
                    })
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.complete_mention_path(click_path.clone(), cx);
                    }))
                    .child(tabler_icon(
                        TablerIcon::File,
                        theme_rgb(&self.theme, "text.muted"),
                        px(13.),
                    ))
                    .child(div().text_xs().child(SharedString::from(name)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(path)),
                    )
                    .into_any_element()
            }))
    }

    pub(super) fn render_terminal_font_picker(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current_family = self.terminal_font_family.clone();
        let options = TERMINAL_FONT_CHOICES
            .iter()
            .map(|(family, description)| {
                let selected = current_family == *family;
                div()
                    .id(format!("terminal-font-option-{family}"))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .p(px(9.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .when(selected, |element| {
                        element.bg(theme_rgb(&self.theme, "accent"))
                    })
                    .hover(|element| element.bg(theme_rgb(&self.theme, "muted")))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.choose_terminal_font(family, cx);
                    }))
                    .child(div().text_sm().child(SharedString::from(*family)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme_rgb(&self.theme, "text.muted"))
                            .child(SharedString::from(*description)),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .id("terminal-font-picker")
            .absolute()
            .right(px(12.))
            .bottom(px(38.))
            .w(px(230.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .border_1()
            .border_color(theme_rgb(&self.theme, "divider"))
            .rounded(px(10.))
            .bg(theme_rgb(&self.theme, "panel.background"))
            .on_key_down(cx.listener(Self::shell_key_down))
            .track_focus(&self.popover_focus_handle)
            .children(options)
    }
}
