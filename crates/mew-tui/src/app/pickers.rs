//! Picker methods for the App state.
//!
//! All command palette / picker construction and manipulation
//! methods, extracted from App for readability.

use super::*;

use fff_search::file_picker::{FFFMode, FilePicker, FilePickerOptions, FuzzySearchOptions};
use fff_search::{PaginationArgs, QueryParser};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
            budget: None,
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
            hint: Some("⏎ attach · ^A archive · ^P pin".into()),
            budget: None,
        });
    }

    pub fn open_project_picker(&mut self) {
        let items: Vec<PickerItem> = self
            .projects
            .iter()
            .map(|p| {
                let last = p
                    .last_used_at
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.format("%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "—".to_string());
                PickerItem {
                    id: p.path.clone(),
                    label: p.display_name.clone(),
                    description: format!(
                        "{}  ·  {} sessions  ·  last: {}",
                        p.path, p.session_count, last
                    ),
                    ..Default::default()
                }
            })
            .collect();
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "project".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: Some("new session in project".into()),
            budget: None,
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
            budget: None,
        });
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
        // Cancel any queued/debounced file search for a closed @ picker so we
        // don't do background work for a query the user already dismissed.
        self.pending_file_query = None;
        self.file_query_deadline = None;
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
        // Models that accept a numeric token budget get a budget row after
        // the effort rows. The draft is seeded from the active budget
        // variant if set, else the effort mapping of the active effort,
        // else the metadata default.
        let budget = self.thinking_budget.get(bare_model).cloned();
        let picker_budget = budget.map(|info| {
            let seed = self.active_budget_seed(&info);
            crate::app::PickerBudget {
                draft: seed.clone(),
                seed,
                info,
                track_rect: None,
                dragging: false,
            }
        });
        if picker_budget.is_some() {
            items.push(PickerItem {
                id: "budget".into(),
                label: "token budget".into(),
                description: String::new(),
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
            budget: picker_budget,
        });
    }

    /// Seed the budget draft for a picker: the active `budget:<n>` variant
    /// if one is set, else the mapped budget of the active effort level,
    /// else the metadata default.
    fn active_budget_seed(&self, budget: &mew_protocol::ThinkingBudgetInfo) -> String {
        let active = self.active_thinking_variant.as_deref();
        if let Some(n) = active.and_then(|v| v.strip_prefix("budget:")) {
            return n.to_string();
        }
        if let Some((_, n)) = budget
            .by_effort
            .iter()
            .find(|(effort, _)| active == Some(effort.as_str()))
        {
            return n.to_string();
        }
        budget.default.to_string()
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
            budget: None,
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
            budget: None,
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
            budget: None,
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
        }
    }

    pub fn picker_up(&mut self) {
        if let Some(ref mut p) = self.picker {
            p.move_selection(-1);
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

    /// Open the inline `@` mention picker with every referenceable item in one
    /// flat list: files (most recently modified first), models, skills, and
    /// subagents. Candidates are recomputed on each keystroke by
    /// [`App::sync_at_picker`]; files are fuzzy-searched with fff once the
    /// background index is ready.
    pub fn open_at_picker(&mut self) {
        self.at_file_items = file_mention_items();
        self.at_reference_items = self.build_reference_items();
        self.fff_file_results = None;
        self.pending_file_query = None;
        self.file_query_deadline = None;

        let items = self.empty_at_candidates();
        self.mode = Mode::CommandPalette;
        self.picker = Some(PickerState {
            kind: "at".into(),
            items,
            filter: String::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            visible_items: PICKER_VISIBLE_ITEMS,
            hint: None,
            budget: None,
        });

        self.schedule_file_index_build();
    }

    fn build_reference_items(&self) -> Vec<PickerItem> {
        let mut items = Vec::new();
        items.extend(self.models.iter().map(|(id, desc)| PickerItem {
            id: id.clone(),
            label: format!("@model:{id}"),
            description: desc.clone(),
            namespace: Some("model"),
            ..Default::default()
        }));
        items.extend(self.skill_catalog.iter().map(|s| PickerItem {
            id: s.name.clone(),
            label: format!("@skill:{}", s.name),
            description: s.description.clone(),
            namespace: Some("skill"),
            ..Default::default()
        }));
        items.extend(self.subagent_catalog.iter().map(|s| PickerItem {
            id: s.name.clone(),
            label: format!("@subagent:{}", s.name),
            description: s.description.clone(),
            namespace: Some("subagent"),
            ..Default::default()
        }));
        items.sort_by_key(|i| i.label.to_lowercase());
        items
    }

    fn empty_at_candidates(&self) -> Vec<PickerItem> {
        let mut items = self.at_file_items.clone();
        items.extend(self.at_reference_items.iter().cloned());
        items
    }

    /// Re-derive the inline `@` picker candidates from the chat input. The
    /// active token runs from the last `@` at or before the cursor to the
    /// cursor; its text after the `@` is the query (either a namespace
    /// reference like `skill:clarify` or a plain file prefix). When the token
    /// is gone (the `@` was deleted), the picker closes.
    pub fn sync_at_picker(&mut self) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        if !picker.is_at_picker() {
            return;
        }

        let before = &self.input[..self.cursor];
        let Some(at) = before.rfind('@') else {
            self.close_picker();
            return;
        };
        let query = before[at + 1..].to_string();

        let candidates = self.compute_at_candidates(&query);
        if self.needs_file_search(&query) {
            self.pending_file_query = Some(query.clone());
            self.file_query_deadline = Some(Instant::now() + FILE_SEARCH_DEBOUNCE);
        } else if !is_plain_file_query(&query) {
            self.pending_file_query = None;
            self.file_query_deadline = None;
        }

        if let Some(p) = self.picker.as_mut() {
            p.items = candidates;
            p.cursor = query.len();
            p.filter = query;
            p.selected = 0;
        }
    }

    fn compute_at_candidates(&self, query: &str) -> Vec<PickerItem> {
        if query.is_empty() {
            return self.empty_at_candidates();
        }

        if let Some((kind, rest)) = namespace_query(query) {
            let rest = rest.to_lowercase();
            return self
                .at_reference_items
                .iter()
                .filter(|i| {
                    i.namespace == Some(kind)
                        && (rest.is_empty()
                            || i.label.to_lowercase().contains(&rest)
                            || i.description.to_lowercase().contains(&rest))
                })
                .cloned()
                .collect();
        }

        // Plain query: substring-matched references + file candidates (fff
        // results when available, otherwise a substring pass over the walk).
        let q = query.to_lowercase();
        let mut items: Vec<PickerItem> = self
            .at_reference_items
            .iter()
            .filter(|i| {
                i.label.to_lowercase().contains(&q) || i.description.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();

        let files = match &self.fff_file_results {
            Some((cached_q, files)) if cached_q == query => files.clone(),
            _ => self.substring_file_candidates(&q),
        };
        items.extend(files);
        items
    }

    fn substring_file_candidates(&self, q: &str) -> Vec<PickerItem> {
        self.at_file_items
            .iter()
            .filter(|i| i.label.to_lowercase().contains(q))
            .cloned()
            .collect()
    }

    fn needs_file_search(&self, query: &str) -> bool {
        if !is_plain_file_query(query) {
            return false;
        }
        !self
            .fff_file_results
            .as_ref()
            .map(|(q, _)| q == query)
            .unwrap_or(false)
    }

    fn schedule_file_index_build(&mut self) {
        if self.file_index.is_some() || self.file_index_building {
            return;
        }
        // Headless tests/harness have no tokio runtime; keep the walk fallback.
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        self.file_index_building = true;
        let tx = self.file_search_tx.clone();
        let cwd = std::env::current_dir().unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            let result = build_file_index(&cwd);
            let _ = tx.blocking_send(FileSearchEvent::IndexReady(result));
        });
    }

    fn schedule_file_search(&mut self, query: String) {
        let Some(index) = self.file_index.clone() else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        self.file_search_gen = self.file_search_gen.wrapping_add(1);
        let generation = self.file_search_gen;
        let tx = self.file_search_tx.clone();
        tokio::task::spawn_blocking(move || {
            let files = fff_file_search(&index, &query, 50);
            let _ = tx.blocking_send(FileSearchEvent::Results {
                generation,
                query,
                files,
            });
        });
    }

    /// Drain background file-search events and fire debounced searches. Called
    /// from [`App::tick`] so it runs on the main-loop cadence.
    pub fn poll_file_search(&mut self) {
        while let Ok(event) = self.file_search_rx.try_recv() {
            match event {
                FileSearchEvent::IndexReady(Ok(index)) => {
                    self.file_index = Some(index);
                    self.file_index_building = false;
                    if let Some(q) = self.pending_file_query.clone() {
                        self.pending_file_query = None;
                        self.file_query_deadline = None;
                        self.schedule_file_search(q);
                    }
                }
                FileSearchEvent::IndexReady(Err(_)) => {
                    self.file_index_building = false;
                }
                FileSearchEvent::Results {
                    generation,
                    query,
                    files,
                } => {
                    if generation != self.file_search_gen {
                        continue;
                    }
                    self.fff_file_results = Some((query.clone(), files));
                    self.sync_at_picker();
                }
            }
        }

        if let Some(q) = self.pending_file_query.clone() {
            let due = self
                .file_query_deadline
                .map(|d| Instant::now() >= d)
                .unwrap_or(true);
            if due && self.file_index.is_some() {
                self.pending_file_query = None;
                self.file_query_deadline = None;
                self.schedule_file_search(q);
            }
        }
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
            budget: None,
        });
    }
}

/// Build the file portion of the `@` mention catalog.
///
/// This is a fresh, bounded directory walk on every open, not an indexed or
/// fuzzy finder. It uses `ignore::WalkBuilder` (the same walker ripgrep and
/// `fd` use), so it is `.gitignore`-aware and skips hidden entries for free.
/// Depth is capped at 4 and files over 1 MiB are skipped, then the result is
/// truncated to 50 entries sorted by modification time (newest first), with
/// shortest path breaking ties. The fff index, when ready, supersedes this
/// walk for non-empty plain queries.
fn file_mention_items() -> Vec<PickerItem> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut entries: Vec<(PickerItem, std::time::SystemTime)> = Vec::new();

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
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() > 1_048_576 {
            continue;
        }
        let rel = path
            .strip_prefix(&cwd)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        entries.push((
            PickerItem {
                id: rel.clone(),
                label: format!("@{rel}"),
                description: format_file_size(meta.len()),
                ..Default::default()
            },
            modified,
        ));
    }

    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.len().cmp(&b.0.id.len())));
    entries.truncate(50);
    entries.into_iter().map(|(item, _)| item).collect()
}

fn format_file_size(size: u64) -> String {
    if size > 1024 {
        format!("{} KB", size / 1024)
    } else {
        format!("{} B", size)
    }
}

const FILE_SEARCH_DEBOUNCE: Duration = Duration::from_millis(60);

/// True when a query is a plain file query (not a namespace reference like
/// `model:` or `skill:`), so it should drive an fff fuzzy file search.
fn is_plain_file_query(query: &str) -> bool {
    !query.is_empty()
        && !NAMESPACE_PREFIXES
            .iter()
            .any(|(_, prefix)| query.starts_with(prefix))
}

/// Split a namespace reference query into `(kind, rest)`, e.g.
/// `"skill:clarify"` -> `("skill", "clarify")`.
fn namespace_query(query: &str) -> Option<(&'static str, &str)> {
    NAMESPACE_PREFIXES
        .iter()
        .find_map(|(kind, prefix)| query.strip_prefix(*prefix).map(|rest| (*kind, rest)))
}

/// Build a fff file index for `cwd` (synchronous; run on a blocking thread).
fn build_file_index(cwd: &Path) -> Result<Arc<Mutex<FilePicker>>, String> {
    let mut picker = FilePicker::new(FilePickerOptions {
        base_path: cwd.to_string_lossy().to_string(),
        enable_mmap_cache: false,
        mode: FFFMode::Ai,
        watch: false,
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;
    picker.collect_files().map_err(|e| e.to_string())?;
    Ok(Arc::new(Mutex::new(picker)))
}

/// Run a fuzzy filename search against a cached fff index and convert the
/// ranked results into `@`-picker items.
fn fff_file_search(index: &Arc<Mutex<FilePicker>>, query: &str, limit: usize) -> Vec<PickerItem> {
    let picker = index.lock().unwrap();
    let parser = QueryParser::default();
    let query = parser.parse(query);
    let options = FuzzySearchOptions {
        pagination: PaginationArgs { offset: 0, limit },
        ..Default::default()
    };
    let result = picker.fuzzy_search(&query, None, options);
    result
        .items
        .iter()
        .take(limit)
        .map(|item| {
            let rel = item.relative_path(&*picker);
            PickerItem {
                id: rel.clone(),
                label: format!("@{rel}"),
                description: format_file_size(item.size),
                ..Default::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(1024), "1024 B");
        assert_eq!(format_file_size(2048), "2 KB");
    }

    #[test]
    fn test_namespace_query() {
        assert_eq!(namespace_query("skill:clarify"), Some(("skill", "clarify")));
        assert_eq!(
            namespace_query("model:openai/gpt-4o"),
            Some(("model", "openai/gpt-4o"))
        );
        assert_eq!(
            namespace_query("subagent:researcher"),
            Some(("subagent", "researcher"))
        );
        assert_eq!(namespace_query("src/main.rs"), None);
        assert_eq!(namespace_query("skill"), None);
    }

    #[test]
    fn test_is_plain_file_query() {
        assert!(!is_plain_file_query(""));
        assert!(is_plain_file_query("src/main.rs"));
        assert!(!is_plain_file_query("skill:clarify"));
        assert!(!is_plain_file_query("model:gpt"));
    }

    #[test]
    fn test_fff_file_search_returns_ranked_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn lib() {}").unwrap();
        std::fs::write(dir.path().join("README.md"), "# readme").unwrap();

        let index = build_file_index(dir.path()).expect("build index");
        let results = fff_file_search(&index, "main", 10);

        assert!(
            results.iter().any(|i| i.label == "@src/main.rs"),
            "expected src/main.rs in results: {results:?}"
        );
        assert!(
            !results.iter().any(|i| i.label == "@README.md"),
            "README should not rank for `main`: {results:?}"
        );
    }
}
