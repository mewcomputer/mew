use super::*;

impl DesktopShell {
    pub(super) fn remember_recent_model(&mut self, provider: &str, model: &str) {
        let full_id = format!("{provider}/{model}");
        push_recent_model(&mut self.recent_models, &full_id);

        let Ok(mut state) = mew_config::load_state() else {
            tracing::warn!(%full_id, "desktop: could not load state while saving recent model");
            return;
        };
        state.recent_models = self.recent_models.clone();
        if let Err(error) = mew_config::save_state(&state) {
            tracing::warn!(%error, "desktop: could not persist recent model");
        }
    }

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
        self.thinking_effort_drag_index = None;
        self.thinking_effort_drag_position = None;
        self.thinking_effort_track_bounds = None;
        self.thinking_effort_animation_from = None;
        self.terminal_font_picker_open = false;
        window.focus(&self.composer_focus_handle, cx);
        if self.model.session_is_ready() {
            self.awaiting_model_switch = Some((provider.clone(), model.clone()));
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
        self.thinking_effort_drag_index = None;
        self.thinking_effort_drag_position = None;
        self.thinking_effort_track_bounds = None;
        self.thinking_effort_animation_from = None;
        self.persona_picker_open = false;
        self.permission_picker_open = false;
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
        self.terminal_font_picker_open = false;
        cx.notify();
    }

    pub(super) fn toggle_permission_picker(&mut self, cx: &mut Context<Self>) {
        self.permission_picker_open = !self.permission_picker_open;
        self.model_picker_open = false;
        self.persona_picker_open = false;
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

    /// `variant` of `None` disables thinking (the daemon treats an empty
    /// string the same as "none").
    pub(super) fn choose_thinking_variant(
        &mut self,
        variant: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model_picker_open = false;
        self.thinking_effort_drag_index = None;
        self.thinking_effort_drag_position = None;
        self.thinking_effort_track_bounds = None;
        self.thinking_effort_animation_from = None;
        window.focus(&self.composer_focus_handle, cx);
        self.send_thinking_variant(variant);
        cx.notify();
    }

    fn send_thinking_variant(&mut self, variant: Option<String>) {
        if self.model.session_is_ready() {
            self.send_command(ClientMessage::SetThinkingVariant {
                variant: variant.unwrap_or_default(),
            });
        }
    }

    fn update_thinking_effort_drag(
        &mut self,
        pointer_x: f32,
        track_width: f32,
        variants: &[Option<String>],
        cx: &mut Context<Self>,
    ) {
        let Some(position) = effort_position_for_pointer(pointer_x, track_width, variants.len())
        else {
            return;
        };
        let Some(index) = effort_index_for_pointer(pointer_x, track_width, variants.len()) else {
            return;
        };
        let position_changed = self
            .thinking_effort_drag_position
            .is_none_or(|previous| (previous - position).abs() > f32::EPSILON);
        let index_changed = self.thinking_effort_drag_index != Some(index);
        self.thinking_effort_drag_position = Some(position);
        self.thinking_effort_drag_index = Some(index);
        if index_changed {
            self.send_thinking_variant(variants[index].clone());
        }
        if position_changed {
            cx.notify();
        }
    }

    fn finish_thinking_effort_drag(
        &mut self,
        pointer: Point<Pixels>,
        variants: &[Option<String>],
        cx: &mut Context<Self>,
    ) {
        if !thinking_effort_drag_is_active(self.thinking_effort_drag_position, cx.has_active_drag())
        {
            return;
        }
        if let Some(bounds) = self.thinking_effort_track_bounds {
            let pointer_x = f32::from(pointer.x) - f32::from(bounds.origin.x);
            let track_width = f32::from(bounds.size.width);
            if let Some(position) =
                effort_position_for_pointer(pointer_x, track_width, variants.len())
            {
                self.update_thinking_effort_drag(pointer_x, track_width, variants, cx);
                self.thinking_effort_drag_position = None;
                self.thinking_effort_animation_from = Some(position);
                self.thinking_effort_animation_id =
                    self.thinking_effort_animation_id.wrapping_add(1);
                cx.notify();
            }
        }
        // Keep the optimistic stop selected until the daemon broadcasts the
        // change. Clearing it here makes the thumb jump back when the round
        // trip takes longer than the mouse-up event.
    }

    pub(super) fn toggle_terminal_font_picker(&mut self, cx: &mut Context<Self>) {
        self.terminal_font_picker_open = !self.terminal_font_picker_open;
        self.model_picker_open = false;
        self.persona_picker_open = false;
        self.permission_picker_open = false;
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
        recent: bool,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let model = self.model.ui.models.get(index)?;
        let provider = model.provider.clone();
        let model_id = model.model.clone();
        let selected = self.model.ui.current_provider.as_deref() == Some(model.provider.as_str())
            && self.model.ui.model.as_deref() == Some(model.model.as_str());
        let label = SharedString::from(model.id.clone());
        let description = model.description.clone();
        let section = if recent { "recent" } else { "all" };
        Some(
            div()
                .id(format!("model-option-{section}-{}", model.id))
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
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_xs()
                        .child(label),
                )
                .when_some(
                    description.map(SharedString::from),
                    |element, description| {
                        element.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
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
            .filter_map(
                |row_index| match self.model_picker_rows.get(row_index).cloned()? {
                    ModelPickerRow::Header(label) => Some(self.render_model_picker_header(label)),
                    ModelPickerRow::Model { index, recent } => {
                        self.render_model_option(index, recent, cx)
                    }
                },
            )
            .collect()
    }

    fn render_model_picker_header(&self, label: &'static str) -> gpui::AnyElement {
        div()
            .id(format!(
                "model-picker-header-{}",
                label.to_lowercase().replace(' ', "-")
            ))
            .flex()
            .items_end()
            .h(px(64.))
            .w_full()
            .px(px(10.))
            .pb(px(8.))
            .text_xs()
            .text_color(theme_rgb(&self.theme, "text.muted"))
            .child(label)
            .into_any_element()
    }

    pub(super) fn render_model_picker(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.model_picker_query.clone();
        let (filtered_indices, rows) =
            build_model_picker_rows(&self.model.ui.models, &self.recent_models, &query);
        self.model_picker_filtered_indices = filtered_indices;
        self.model_picker_rows = rows;
        let option_count = self.model_picker_rows.len();
        let thinking_variants = thinking_variants_for_model(
            &self.model.ui.models,
            self.model.ui.current_provider.as_deref(),
            self.model.ui.model.as_deref(),
        );
        let current_thinking_variant = self.model.ui.thinking_variant.clone();
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
                .flex_1()
                .p(px(8.))
                .text_sm()
                .text_color(theme_rgb(&self.theme, "text.muted"))
                .child(message)
                .into_any_element()
        };
        let effort_option_count = if thinking_variants.is_empty() {
            0
        } else {
            thinking_variants.len() + 1
        };
        let effort_picker = if effort_option_count > 0 {
            let mut effort_values = vec![("Off".to_owned(), None)];
            effort_values.extend(
                thinking_variants
                    .iter()
                    .map(|variant| (variant.clone(), Some(variant.clone()))),
            );
            let effort_variants = effort_values
                .iter()
                .map(|(_, variant)| variant.clone())
                .collect::<Vec<_>>();
            let selected_effort_index = self
                .thinking_effort_drag_index
                .filter(|index| *index < effort_values.len())
                .or_else(|| {
                    effort_values.iter().position(|(_, variant)| {
                        current_thinking_variant.as_ref() == variant.as_ref()
                    })
                })
                .unwrap_or(0);
            let effort_accent = theme_rgb(&self.theme, "text.accent");
            let effort_muted = theme_rgb(&self.theme, "muted");
            let effort_track_muted = effort_muted.opacity(0.78);
            let effort_track_width = self
                .thinking_effort_track_bounds
                .map(|bounds| f32::from(bounds.size.width))
                .unwrap_or(300.);
            let effort_track_inset = effort_track_inset(effort_track_width, effort_values.len());
            let render_effort_dot = |index: usize,
                                     label: String,
                                     variant: Option<String>,
                                     stop_offset: f32,
                                     cx: &mut Context<Self>| {
                let selected = selected_effort_index == index;
                div()
                    .id(thinking_effort_option_id(index))
                    .absolute()
                    .left(px(stop_offset))
                    .ml(px(-12.))
                    .top_0()
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .role(Role::MenuItem)
                    .desktop_focus(effort_accent)
                    .aria_label(SharedString::from(label))
                    .aria_selected(selected)
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.choose_thinking_variant(variant.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .size(px(if selected { 6. } else { 5. }))
                            .rounded_full()
                            .bg(effort_accent.opacity(if selected { 1. } else { 0.72 })),
                    )
                    .into_any_element()
            };
            let effort_dots = effort_values
                .iter()
                .enumerate()
                .map(|(index, (label, variant))| {
                    let stop_offset =
                        effort_stop_offset(index, effort_track_width, effort_values.len())
                            .unwrap_or(0.);
                    render_effort_dot(index, label.clone(), variant.clone(), stop_offset, cx)
                })
                .collect::<Vec<_>>();
            let effort_label_slot_width =
                effort_stop_slot_width(effort_track_width, effort_values.len());
            let effort_labels = effort_values
                .iter()
                .enumerate()
                .map(|(index, (label, _))| {
                    let stop_offset =
                        effort_stop_offset(index, effort_track_width, effort_values.len())
                            .unwrap_or(0.);
                    div()
                        .absolute()
                        .left(px(stop_offset))
                        .ml(px(-effort_label_slot_width / 2.))
                        .w(px(effort_label_slot_width))
                        .text_center()
                        .text_xs()
                        .text_color(theme_rgb(&self.theme, "text.muted"))
                        .child(SharedString::from(label.clone()))
                })
                .collect::<Vec<_>>();
            let effort_display = effort_values
                .get(selected_effort_index)
                .map(|(label, _)| label.clone())
                .unwrap_or_else(|| "Off".to_owned());
            let target_effort_position =
                effort_position_for_index(selected_effort_index, effort_values.len());
            let drag_effort_position = self.thinking_effort_drag_position;
            let display_effort_position = drag_effort_position.unwrap_or(target_effort_position);
            let animation_from = self.thinking_effort_animation_from;
            let animation_id = self.thinking_effort_animation_id;
            let drag_variants = effort_variants.clone();
            let direct_drag_variants = effort_variants.clone();
            let release_drag_variants = effort_variants.clone();
            let release_drag_out_variants = effort_variants.clone();
            let track_bounds_shell = cx.entity();
            let effort_track = div()
                .absolute()
                .left(px(effort_track_inset))
                .right(px(effort_track_inset))
                .top(px(10.))
                .h(px(5.))
                .rounded_full()
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(effort_accent, display_effort_position),
                    linear_color_stop(effort_track_muted, display_effort_position),
                ));
            let effort_track = match (drag_effort_position, animation_from) {
                (None, Some(from)) => effort_track
                    .with_animation(
                        ElementId::Name(
                            format!("model-picker-effort-track-animation-{animation_id}").into(),
                        ),
                        Animation::new(Duration::from_millis(180))
                            .with_easing(gpui::ease_out_quint()),
                        move |element, delta| {
                            let position = from + (target_effort_position - from) * delta;
                            element.bg(linear_gradient(
                                90.,
                                linear_color_stop(effort_accent, position),
                                linear_color_stop(effort_track_muted, position),
                            ))
                        },
                    )
                    .into_any_element(),
                _ => effort_track.into_any_element(),
            };
            let effort_thumb = div()
                .absolute()
                .top_0()
                .left(relative(display_effort_position))
                .ml(px(-12.))
                .size(px(24.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme_rgb(&self.theme, "text.body"))
                .border_1()
                .border_color(theme_rgb(&self.theme, "text.body"))
                .child(div().size(px(5.)).rounded_full().bg(effort_accent));
            let effort_thumb = match (drag_effort_position, animation_from) {
                (None, Some(from)) => effort_thumb
                    .with_animation(
                        ElementId::Name(
                            format!("model-picker-effort-thumb-animation-{animation_id}").into(),
                        ),
                        Animation::new(Duration::from_millis(180))
                            .with_easing(gpui::ease_out_quint()),
                        move |element, delta| {
                            let position = from + (target_effort_position - from) * delta;
                            element.left(relative(position))
                        },
                    )
                    .into_any_element(),
                _ => effort_thumb.into_any_element(),
            };
            let effort_thumb = div()
                .absolute()
                .left(px(effort_track_inset))
                .right(px(effort_track_inset))
                .top_0()
                .bottom_0()
                .child(effort_thumb);
            Some(
                div()
                    .id("model-picker-effort")
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .pt(px(8.))
                    .border_t_1()
                    .border_color(theme_rgb(&self.theme, "divider"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .text_color(theme_rgb(&self.theme, "text.muted"))
                                    .child(tabler_icon(
                                        TablerIcon::Bulb,
                                        theme_rgb(&self.theme, "text.muted"),
                                        px(13.),
                                    ))
                                    .child("Effort"),
                            )
                            .child(
                                div()
                                    .text_color(theme_rgb(&self.theme, "text.body"))
                                    .child(SharedString::from(effort_display)),
                            ),
                    )
                    .child(
                        div()
                            .id("model-picker-effort-track")
                            .relative()
                            .flex()
                            .items_center()
                            .w_full()
                            .h(px(24.))
                            .cursor(gpui::CursorStyle::PointingHand)
                            .on_drag(ThinkingEffortDrag, |_, _, _, cx| {
                                cx.new(|_| ThinkingEffortPreview)
                            })
                            .on_drag_move::<ThinkingEffortDrag>(cx.listener(
                                move |shell,
                                      event: &gpui::DragMoveEvent<ThinkingEffortDrag>,
                                      _,
                                      cx| {
                                    let pointer_x = f32::from(event.event.position.x)
                                        - f32::from(event.bounds.origin.x);
                                    let track_width = f32::from(event.bounds.size.width);
                                    shell.update_thinking_effort_drag(
                                        pointer_x,
                                        track_width,
                                        &drag_variants,
                                        cx,
                                    );
                                },
                            ))
                            .on_mouse_move(cx.listener(
                                move |shell, event: &gpui::MouseMoveEvent, _, cx| {
                                    if !event.dragging() {
                                        return;
                                    }
                                    let Some(bounds) = shell.thinking_effort_track_bounds else {
                                        return;
                                    };
                                    let pointer_x =
                                        f32::from(event.position.x) - f32::from(bounds.origin.x);
                                    shell.update_thinking_effort_drag(
                                        pointer_x,
                                        f32::from(bounds.size.width),
                                        &direct_drag_variants,
                                        cx,
                                    );
                                },
                            ))
                            .on_mouse_up(
                                gpui::MouseButton::Left,
                                cx.listener(move |shell, event: &gpui::MouseUpEvent, _, cx| {
                                    shell.finish_thinking_effort_drag(
                                        event.position,
                                        &release_drag_variants,
                                        cx,
                                    );
                                }),
                            )
                            .on_mouse_up_out(
                                gpui::MouseButton::Left,
                                cx.listener(move |shell, event: &gpui::MouseUpEvent, _, cx| {
                                    shell.finish_thinking_effort_drag(
                                        event.position,
                                        &release_drag_out_variants,
                                        cx,
                                    );
                                }),
                            )
                            .child(effort_track)
                            .children(effort_dots)
                            .child(effort_thumb)
                            .child(
                                canvas(
                                    move |bounds, _, cx| {
                                        track_bounds_shell.update(cx, |shell, cx| {
                                            if shell.thinking_effort_track_bounds != Some(bounds) {
                                                shell.thinking_effort_track_bounds = Some(bounds);
                                                cx.notify();
                                            }
                                        });
                                    },
                                    |_bounds, _state, _window, _cx| {},
                                )
                                .absolute()
                                .inset_0()
                                .size_full(),
                            ),
                    )
                    .child(div().relative().w_full().h(px(16.)).children(effort_labels))
                    .into_any_element(),
            )
        } else {
            None
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
            .w(px(400.))
            .flex()
            .flex_col()
            .h(model_picker_height(option_count, effort_option_count))
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
            .when_some(effort_picker, |element, effort_picker| {
                element.child(effort_picker)
            })
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
