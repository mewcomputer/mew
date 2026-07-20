//! Picker methods for the App state.
//!
//! All command palette / picker construction and manipulation
//! methods, extracted from App for readability.

use super::*;

impl App {
    pub fn open_model_picker(&mut self) {
        let mut items: Vec<PickerItem> = Vec::new();

        // Prepend a "Recent" section when there are recent models and no
        // filter is active yet. The items are deduplicated against the full
        // model list and capped at 6.
        if !self.recent_models.is_empty() {
            let all_ids: std::collections::HashSet<&str> =
                self.models.iter().map(|(id, _)| id.as_str()).collect();
            let recent: Vec<&String> = self
                .recent_models
                .iter()
                .filter(|m| all_ids.contains(m.as_str()))
                .take(6)
                .collect();
            if !recent.is_empty() {
                items.push(PickerItem {
                    label: "Recent".into(),
                    header: true,
                    ..Default::default()
                });
                for id in &recent {
                    let desc = self
                        .models
                        .iter()
                        .find(|(mid, _)| mid == *id)
                        .map(|(_, d)| d.as_str())
                        .unwrap_or("");
                    items.push(PickerItem {
                        id: id.to_string(),
                        label: id.to_string(),
                        description: desc.to_string(),
                        ..Default::default()
                    });
                }
                items.push(PickerItem {
                    label: "All Models".into(),
                    header: true,
                    ..Default::default()
                });
            }
        }

        items.extend(self.models.iter().map(|(id, desc)| PickerItem {
            id: id.clone(),
            label: id.clone(),
            description: desc.clone(),
            ..Default::default()
        }));

        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "model".into(),
            items,
            filter: String::new(),
            selected: if !self.recent_models.is_empty() { 1 } else { 0 },
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: Some("→ thinking variants".into()),
        });
    }

    pub fn open_session_picker(&mut self) {
        let active_id = &self.status.session_id;
        let items: Vec<PickerItem> = self
            .daemon_sessions
            .iter()
            .filter(|s| !s.archived)
            .map(|s| {
                let title = self
                    .session_titles
                    .get(&s.session_id)
                    .cloned()
                    .or_else(|| s.summary.clone())
                    .unwrap_or_else(|| s.session_id.chars().take(8).collect());
                let glyph = match s.state {
                    mew_protocol::SessionState::Running => "▶",
                    mew_protocol::SessionState::Active => "●",
                    mew_protocol::SessionState::Idle => "○",
                };
                let cost = s
                    .usage
                    .as_ref()
                    .map(|u| format!("${:.2}", u.cost))
                    .unwrap_or_default();
                let is_active = &s.session_id == active_id;
                let marker = if is_active { " ● active" } else { "" };
                let label = format!("{} {} {}{}", glyph, title, cost, marker);

                let cwd_str = s.cwd.as_deref().unwrap_or("—");
                let last = s
                    .last_message_at
                    .map(|ts| {
                        chrono::DateTime::from_timestamp(ts, 0)
                            .map(|dt| dt.format("%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "—".to_string())
                    })
                    .unwrap_or_else(|| "—".to_string());
                let desc = format!("cwd: {}  ·  last: {}", cwd_str, last);

                PickerItem {
                    id: s.session_id.clone(),
                    label,
                    description: desc,
                    ..Default::default()
}
            })
            .collect();
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "session".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    pub fn open_permission_mode_picker(&mut self) {
        let active = self.permission_mode;
        let marker = |m: mew_hooks::PermissionMode| -> &'static str {
            if m == active {
                " ● active"
            } else {
                ""
            }
        };
        let items = vec![
            PickerItem {
                id: mew_hooks::PermissionMode::Standard.id().into(),
                label: format!("Standard{}", marker(mew_hooks::PermissionMode::Standard)),
                description: "Prompts for Mutating/Dangerous tools. Default.".into(),
            ..Default::default()
            },
            PickerItem {
                id: mew_hooks::PermissionMode::Permissive.id().into(),
                label: format!(
                    "Permissive{}",
                    marker(mew_hooks::PermissionMode::Permissive)
                ),
                description: "Auto-allows Mutating tools (write/edit/etc.). \
                              Still prompts for bash and respects your deny rules, \
                              ask rules, and secret-file guard."
                    .into(),
                ..Default::default()
},
            PickerItem {
                id: mew_hooks::PermissionMode::Auto.id().into(),
                label: format!("Auto{}", marker(mew_hooks::PermissionMode::Auto)),
                description: "Routes every tool call through a small/cheap LLM \
                              classifier. Classifier returns allow/deny/escalate; \
                              escalate falls back to the user modal. Skip the \
                              prompts — let the model decide. Requires a \
                              classifier provider to be configured."
                    .into(),
                ..Default::default()
},
            PickerItem {
                id: mew_hooks::PermissionMode::AutoPlus.id().into(),
                label: format!("Auto+{}", marker(mew_hooks::PermissionMode::AutoPlus)),
                description: "Like Auto, but the classifier CANNOT escalate. \
                              Escalate or any classifier failure → Deny (fail \
                              closed). Hands-off but uncertainty means no. \
                              Use when you trust the model more than your \
                              own attention but don't want a provider outage \
                              to silently run destructive tools."
                    .into(),
                ..Default::default()
},
            PickerItem {
                id: mew_hooks::PermissionMode::Dangerous.id().into(),
                label: format!("Dangerous!{}", marker(mew_hooks::PermissionMode::Dangerous)),
                description: "Every tool auto-runs. Overrides deny rules, ask rules, \
                              secret-file guard, bash decomposition. Pure bypass — \
                              you've said \"don't ask me anything, even the things I \
                              said don't do.\" Output redaction still applies."
                    .into(),
                ..Default::default()
},
        ];
        // Pre-select the active mode so Enter on an unchanged picker is a no-op.
        let pre_selected = match active {
            mew_hooks::PermissionMode::Standard => 0,
            mew_hooks::PermissionMode::Permissive => 1,
            mew_hooks::PermissionMode::Auto => 2,
            mew_hooks::PermissionMode::AutoPlus => 3,
            mew_hooks::PermissionMode::Dangerous => 4,
        };
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "permission_mode".into(),
            items,
            filter: String::new(),
            selected: pre_selected,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
    }

    pub fn open_thinking_variant_picker(&mut self) {
        self.open_thinking_variant_picker_for(None);
    }

    /// Open the thinking variant picker for a specific model. When `model` is
    /// None, uses the active model. The `model` param is a bare model id or a
    /// "provider/model" pair; the provider prefix is stripped since
    /// `thinking_variants` is keyed by bare model id.
    pub fn open_thinking_variant_picker_for(&mut self, model: Option<&str>) {
        let bare_model = model
            .map(|m| m.rsplit('/').next().unwrap_or(m))
            .unwrap_or_else(|| self.status.model.as_str());
        let mut items = vec![PickerItem {
            id: "off".into(),
            label: "Off".into(),
            description: "Disable thinking/reasoning".into(),
            ..Default::default()
        }];
        // Variant names come from the catalog (populated into
        // `thinking_variants` at startup), keyed by model slug. This holds the
        // model's actual levels — e.g. codex models expose
        // low/medium/high/xhigh/max/ultra — rather than a hardcoded list.
        let variant_names = self
            .thinking_variants
            .get(bare_model)
            .cloned()
            .unwrap_or_default();
        for name in variant_names {
            items.push(PickerItem {
                id: name.clone(),
                label: name.clone(),
                description: format!("{} reasoning effort", name),
                ..Default::default()
            });
        }
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "thinking_variant".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    pub fn open_theme_picker(&mut self) {
        let names = crate::theme::Theme::list_available();
        let current = &self.theme.name;
        let items: Vec<PickerItem> = names
            .iter()
            .map(|n| PickerItem {
                id: n.clone(),
                label: if n == current {
                    format!("{n} (active)")
                } else {
                    n.clone()
                },
                description: format!("Switch to {} theme", n),
                ..Default::default()
            })
            .collect();
        let pre_selected = names.iter().position(|n| n == current).unwrap_or(0);
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "theme".into(),
            items,
            filter: String::new(),
            selected: pre_selected,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    pub fn open_persona_picker(&mut self) {
        let items: Vec<PickerItem> = self
            .personas
            .iter()
            .map(|(name, desc)| {
                let active = self.active_persona.as_deref() == Some(name.as_str());
                PickerItem {
                    id: name.clone(),
                    label: if active {
                        format!("● {} (active)", name)
                    } else {
                        name.clone()
                    },
                    description: desc.clone(),
                    ..Default::default()
                }
            })
            .collect();
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "persona".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    pub fn open_rewind_picker(&mut self) {
        let total = self.messages.len();
        let start = total.saturating_sub(15);
        let items: Vec<PickerItem> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(i, _)| *i >= start)
            .map(|(i, msg)| {
                let role = match msg.role {
                    mew_message::Role::User => "user",
                    mew_message::Role::Assistant => "asst",
                    mew_message::Role::System => "sys",
                };
                let snippet: String = msg
                    .parts
                    .iter()
                    .find_map(|p| match p {
                        mew_message::Part::Text(tp) => Some(tp.text.as_str()),
                        _ => None,
                    })
                    .unwrap_or("(no text)")
                    .chars()
                    .take(60)
                    .collect();
                PickerItem {
                    id: i.to_string(),
                    label: format!("[{}] {}: {}", i, role, snippet),
                    description: format!("Keep messages 0..={}", i),
            ..Default::default()
                }
            })
            .collect();
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "rewind".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    pub fn picker_insert(&mut self, c: char) {
        if let Some(ref mut p) = self.picker {
            p.filter.insert(p.cursor, c);
            p.cursor += c.len_utf8();
            p.selected = 0;
        }
    }

    pub fn picker_backspace(&mut self) {
        if let Some(ref mut p) = self.picker {
            if p.cursor > 0 {
                let prev = p.filter[..p.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                p.filter.remove(prev);
                p.cursor = prev;
                p.selected = p.selected.min(p.filtered().len().saturating_sub(1));
            }
        }
    }

    pub fn picker_down(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.move_selection(1);
            p.adjust_scroll();
        }
    }

    pub fn picker_up(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.move_selection(-1);
            p.adjust_scroll();
        }
    }

    pub fn picker_cursor_left(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.cursor = p.filter[..p.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn picker_cursor_right(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.cursor = p.filter[p.cursor..]
                .chars()
                .next()
                .map(|c| p.cursor + c.len_utf8())
                .unwrap_or(p.filter.len());
        }
    }

    pub fn open_file_picker(&mut self, prefix: &str) {
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut items: Vec<PickerItem> = Vec::new();

        let prefix_lower = prefix.to_lowercase();

        for (name, description) in &self.subagent_names {
            if name.to_lowercase().contains(&prefix_lower) {
                items.push(PickerItem {
                    id: name.clone(),
                    label: format!("@{} [subagent]", name),
                    description: description.clone(),
            ..Default::default()
                });
            }
        }

        let walker = ignore::WalkBuilder::new(&cwd)
            .max_depth(Some(4))
            .hidden(false)
            .git_ignore(true)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.len() > 1_048_576 {
                    continue;
                }
            }
            let rel = path
                .strip_prefix(&cwd)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            if !rel.to_lowercase().contains(&prefix_lower) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            items.push(PickerItem {
                id: rel.clone(),
                label: format!("@{}", rel),
                description: if size > 1024 {
                    format!("{} KB", size / 1024)
                } else {
                    format!("{} B", size)
                },
                ..Default::default()
            });
        }

        items.sort_by_key(|i| i.label.len());
        items.truncate(50);

        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "file".into(),
            items,
            filter: prefix.to_string(),
            selected: 0,
            cursor: prefix.len(),
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }

    /// Open the command palette with a list of commands.
    /// Includes all built-in slash commands plus the original palette items.
    pub fn open_command_palette(&mut self) {
        let mut items = vec![
            PickerItem {
                id: "switch-model".into(),
                label: "Switch Model".into(),
                description: "Change the active LLM".into(),
            ..Default::default()
            },
            PickerItem {
                id: "thinking-variant".into(),
                label: "Thinking Variant".into(),
                description: "Set reasoning effort (high, max, off)".into(),
            ..Default::default()
            },
            PickerItem {
                id: "settings".into(),
                label: "Settings".into(),
                description: "Configure mew (providers, plugins)".into(),
            ..Default::default()
            },
            PickerItem {
                id: "clear".into(),
                label: "Clear Chat".into(),
                description: "Remove all messages from the current session".into(),
            ..Default::default()
            },
            PickerItem {
                id: "quit".into(),
                label: "Quit".into(),
                description: "Exit mew".into(),
            ..Default::default()
            },
        ];
        // Add all built-in slash commands as palette items. Selecting one
        // dispatches Action::SlashCommand, which re-enters handle_slash.
        for cmd in Self::builtin_slash_commands() {
            // Skip commands already represented by the hardcoded items above
            // (clear, quit, model, thinking) and /help (the palette IS help).
            if matches!(
                cmd.name.as_str(),
                "/clear" | "/quit" | "/model" | "/thinking" | "/help"
            ) {
                continue;
            }
            items.push(PickerItem {
                id: cmd.name.clone(),
                label: cmd.name,
                description: cmd.description,
            ..Default::default()
            });
        }
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "command".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
        });
    }
}
