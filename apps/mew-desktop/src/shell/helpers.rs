use super::*;
use unicode_segmentation::UnicodeSegmentation;

pub(super) fn sidebar_transition_width(collapsed: bool, delta: f32) -> f32 {
    let (from, to) = if collapsed {
        (SIDEBAR_EXPANDED_WIDTH, SIDEBAR_COLLAPSED_WIDTH)
    } else {
        (SIDEBAR_COLLAPSED_WIDTH, SIDEBAR_EXPANDED_WIDTH)
    };
    from + (to - from) * delta
}

pub(super) fn sidebar_transition_offset(collapsed: bool, delta: f32) -> f32 {
    if collapsed {
        -SIDEBAR_EXPANDED_WIDTH * delta
    } else {
        -SIDEBAR_EXPANDED_WIDTH * (1. - delta)
    }
}

pub(super) fn workbench_transition_width(collapsed: bool, delta: f32) -> f32 {
    let (from, to) = if collapsed {
        (WORKBENCH_EXPANDED_WIDTH, WORKBENCH_COLLAPSED_WIDTH)
    } else {
        (WORKBENCH_COLLAPSED_WIDTH, WORKBENCH_EXPANDED_WIDTH)
    };
    from + (to - from) * delta
}

pub(super) fn workbench_transition_offset(collapsed: bool, delta: f32, expanded_width: f32) -> f32 {
    if collapsed {
        -expanded_width * delta
    } else {
        -expanded_width * (1. - delta)
    }
}

pub(super) fn workbench_max_width(window_width: f32, sidebar_width: f32) -> f32 {
    (window_width - sidebar_width - CHAT_MIN_WIDTH).max(0.)
}

pub(super) fn workbench_width_from_pointer(
    window_width: f32,
    pointer_x: f32,
    sidebar_width: f32,
) -> f32 {
    let max_width = workbench_max_width(window_width, sidebar_width);
    let min_width = WORKBENCH_MIN_WIDTH.min(max_width);
    (window_width - pointer_x - SHELL_GUTTER).clamp(min_width, max_width)
}
pub(super) fn byte_offset_for_utf16(text: &str, utf16_offset: usize) -> usize {
    let mut byte_offset = 0;
    let mut remaining = utf16_offset;
    for character in text.chars() {
        if remaining < character.len_utf16() {
            break;
        }
        remaining -= character.len_utf16();
        byte_offset += character.len_utf8();
        if remaining == 0 {
            break;
        }
    }
    byte_offset
}

pub(super) fn utf16_offset_for_byte(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset.min(text.len())]
        .chars()
        .map(char::len_utf16)
        .sum()
}

pub(super) fn snap_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub(super) fn previous_utf8_boundary(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    text.grapheme_indices(true)
        .rev()
        .find_map(|(offset, _)| (offset < cursor).then_some(offset))
        .unwrap_or(0)
}

pub(super) fn next_utf8_boundary(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    text.grapheme_indices(true)
        .find_map(|(offset, _)| (offset > cursor).then_some(offset))
        .unwrap_or(text.len())
}

pub(super) fn theme_name(appearance: WindowAppearance) -> &'static str {
    match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => "light",
        WindowAppearance::Dark | WindowAppearance::VibrantDark => "dark",
    }
}

pub(super) fn desktop_theme_name<'a>(
    mode: DesktopThemeMode,
    appearance: WindowAppearance,
    light_theme: &'a str,
    dark_theme: &'a str,
) -> &'a str {
    match mode {
        DesktopThemeMode::System => match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => light_theme,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => dark_theme,
        },
        DesktopThemeMode::Light => light_theme,
        DesktopThemeMode::Dark => dark_theme,
    }
}

pub(super) fn shell_command_for_key(key: &str, platform_modifier: bool) -> Option<ShellCommand> {
    if !platform_modifier {
        return None;
    }
    match key {
        "b" => Some(ShellCommand::ToggleSidebar),
        "j" => Some(ShellCommand::ToggleTerminal),
        "n" => Some(ShellCommand::NewConversation),
        "w" => Some(ShellCommand::CloseActiveTab),
        key if key.chars().count() == 1 => key
            .chars()
            .next()
            .and_then(|character| character.to_digit(10))
            .and_then(|number| number.checked_sub(1))
            .map(|index| ShellCommand::SelectTab(index as usize)),
        _ => None,
    }
}

pub(super) fn escape_dismisses_shell_popover(
    key: &str,
    model_open: bool,
    persona_open: bool,
    permission_open: bool,
    thinking_open: bool,
    terminal_font_open: bool,
    connection_open: bool,
) -> bool {
    key == "escape"
        && (model_open
            || persona_open
            || permission_open
            || thinking_open
            || terminal_font_open
            || connection_open)
}

pub(super) fn is_copy_keystroke(event: &gpui::KeyDownEvent) -> bool {
    event.keystroke.key == "c" && event.keystroke.modifiers.platform
}

pub(super) fn client_event_is_text_delta(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::TextDelta { .. })
}

pub(super) fn display_session_path(path: &str) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    display_path_from_home(path, home.as_deref())
}

pub(super) fn session_time_label(timestamp_ms: Option<i64>) -> Option<String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()?;
    relative_session_time(timestamp_ms, now_ms)
}

pub(super) fn relative_session_time(timestamp_ms: Option<i64>, now_ms: i64) -> Option<String> {
    let timestamp_ms = timestamp_ms.filter(|timestamp| *timestamp > 0)?;
    let elapsed_ms = now_ms.saturating_sub(timestamp_ms);
    let elapsed_minutes = elapsed_ms / 60_000;
    let elapsed_hours = elapsed_ms / 3_600_000;
    let elapsed_days = elapsed_ms / 86_400_000;
    let label = if elapsed_minutes < 1 {
        "now".to_owned()
    } else if elapsed_hours < 1 {
        format!("{elapsed_minutes}m")
    } else if elapsed_days < 1 {
        format!("{elapsed_hours}h")
    } else if elapsed_days < 7 {
        format!("{elapsed_days}d")
    } else {
        format!("{}w", elapsed_days / 7)
    };
    Some(label)
}

pub(super) fn non_empty_label(value: Option<&str>, fallback: &str) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

pub(super) fn model_picker_list_height(option_count: usize) -> Pixels {
    px((option_count.clamp(1, 5) as f32 * 64.).min(320.))
}

pub(super) fn model_picker_height(option_count: usize) -> Pixels {
    model_picker_list_height(option_count) + px(16.)
}

pub(super) fn persona_picker_list_height(option_count: usize) -> Pixels {
    px((option_count.clamp(1, 5) as f32 * 64.).min(320.))
}

pub(super) fn persona_picker_height(option_count: usize) -> Pixels {
    persona_picker_list_height(option_count) + px(16.)
}

pub(super) fn should_animate_transcript_row(row_index: usize, row_count: usize) -> bool {
    row_count > 0 && row_index + 1 == row_count
}

/// Slash menu entries matching the current composer text. The menu is only
/// relevant while the composer starts with `/`; a query of `/` lists every
/// command, and typing arguments (e.g. `/goal fix the bug`) stops matching so
/// the menu closes on its own.
pub(super) fn filtered_slash_commands(query: &str) -> Vec<&'static SlashCommandDef> {
    if !query.starts_with('/') {
        return Vec::new();
    }
    let query = query.to_lowercase();
    SLASH_COMMANDS
        .iter()
        .filter(|def| query == "/" || def.name.starts_with(query.as_str()))
        .collect()
}

/// Routes a submitted composer text to the daemon's slash handler when its
/// first token is a known daemon-side command. Returns the full trimmed
/// command line (including arguments) for `ClientMessage::SlashCommand`.
pub(super) fn daemon_slash_command(text: &str) -> Option<String> {
    let text = text.trim();
    let command = text.split_whitespace().next()?;
    SLASH_COMMANDS
        .iter()
        .any(|def| def.name == command)
        .then(|| text.to_owned())
}

pub(super) fn slash_menu_height(option_count: usize) -> Pixels {
    px(option_count.clamp(1, 5) as f32 * 40. + 12.)
}

pub(super) const MENTION_MENU_LIMIT: usize = 8;

/// Byte offset of the `@` and the typed query for a mention token ending at
/// the cursor. Only a whitespace-delimited token starting with `@` counts, so
/// `email@example` does not open the picker.
pub(super) fn mention_query_at_cursor(text: &str, cursor: usize) -> Option<(usize, String)> {
    let cursor = snap_to_char_boundary(text, cursor);
    let prefix = &text[..cursor];
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    let query = prefix[start..].strip_prefix('@')?;
    Some((start, query.to_owned()))
}

pub(super) fn join_tree_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// Workspace-relative paths of every file in the fetched directory listings.
pub(super) fn mention_file_paths(entries: &BTreeMap<String, Vec<DirEntry>>) -> Vec<String> {
    let mut paths = Vec::new();
    for (dir, entries) in entries {
        for entry in entries {
            if !entry.is_dir {
                paths.push(join_tree_path(dir, &entry.name));
            }
        }
    }
    paths.sort();
    paths
}

pub(super) fn filter_mention_candidates(paths: &[String], query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    paths
        .iter()
        .filter(|path| query.is_empty() || path.to_lowercase().contains(&query))
        .take(MENTION_MENU_LIMIT)
        .cloned()
        .collect()
}

pub(super) fn mention_menu_height(option_count: usize) -> Pixels {
    px(option_count.clamp(1, MENTION_MENU_LIMIT) as f32 * 28. + 12.)
}

/// One visible row in the workbench file tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileTreeRow {
    pub(super) path: String,
    pub(super) name: String,
    pub(super) is_dir: bool,
    pub(super) depth: usize,
    pub(super) expanded: bool,
    pub(super) loaded: bool,
}

/// Flattens fetched directory listings into visible tree rows. Only expanded
/// directories with a fetched listing contribute children.
#[cfg(test)]
pub(super) fn collect_file_tree_rows(
    entries: &BTreeMap<String, Vec<DirEntry>>,
    expanded: &BTreeSet<String>,
) -> Vec<FileTreeRow> {
    fn walk(
        dir: &str,
        depth: usize,
        entries: &BTreeMap<String, Vec<DirEntry>>,
        expanded: &BTreeSet<String>,
        rows: &mut Vec<FileTreeRow>,
    ) {
        let Some(children) = entries.get(dir) else {
            return;
        };
        for entry in children {
            let path = join_tree_path(dir, &entry.name);
            let is_expanded = expanded.contains(&path);
            rows.push(FileTreeRow {
                path: path.clone(),
                name: entry.name.clone(),
                is_dir: entry.is_dir,
                depth,
                expanded: is_expanded,
                loaded: entries.contains_key(&path),
            });
            if entry.is_dir && is_expanded {
                walk(&path, depth + 1, entries, expanded, rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk("", 0, entries, expanded, &mut rows);
    rows
}

/// Counts the visible rows in the fetched file tree without allocating a
/// flattened representation. The uniform list needs the total count, while
/// its row callback only materializes the visible range.
pub(super) fn file_tree_row_count(
    entries: &BTreeMap<String, Vec<DirEntry>>,
    expanded: &BTreeSet<String>,
) -> usize {
    fn count(
        dir: &str,
        entries: &BTreeMap<String, Vec<DirEntry>>,
        expanded: &BTreeSet<String>,
    ) -> usize {
        let Some(children) = entries.get(dir) else {
            return 0;
        };
        children
            .iter()
            .map(|entry| {
                let path = join_tree_path(dir, &entry.name);
                1 + if entry.is_dir && expanded.contains(&path) {
                    count(&path, entries, expanded)
                } else {
                    0
                }
            })
            .sum()
    }

    count("", entries, expanded)
}

/// Returns only the requested visible file-tree rows. Traversal stops after
/// the end of the range, so scrolling does not allocate the whole expanded
/// tree on every render.
pub(super) fn file_tree_rows_in_range(
    entries: &BTreeMap<String, Vec<DirEntry>>,
    expanded: &BTreeSet<String>,
    range: Range<usize>,
) -> Vec<FileTreeRow> {
    fn walk(
        dir: &str,
        depth: usize,
        entries: &BTreeMap<String, Vec<DirEntry>>,
        expanded: &BTreeSet<String>,
        range: &Range<usize>,
        index: &mut usize,
        rows: &mut Vec<FileTreeRow>,
    ) -> bool {
        let Some(children) = entries.get(dir) else {
            return false;
        };
        for entry in children {
            if *index >= range.end {
                return true;
            }
            let path = join_tree_path(dir, &entry.name);
            let is_expanded = expanded.contains(&path);
            let row_index = *index;
            *index += 1;
            if range.contains(&row_index) {
                rows.push(FileTreeRow {
                    path: path.clone(),
                    name: entry.name.clone(),
                    is_dir: entry.is_dir,
                    depth,
                    expanded: is_expanded,
                    loaded: entries.contains_key(&path),
                });
            }
            if entry.is_dir
                && is_expanded
                && walk(&path, depth + 1, entries, expanded, range, index, rows)
            {
                return true;
            }
        }
        false
    }

    if range.is_empty() {
        return Vec::new();
    }
    let mut rows = Vec::with_capacity(range.len());
    let mut index = 0;
    walk("", 0, entries, expanded, &range, &mut index, &mut rows);
    rows
}

pub(super) fn clipboard_image_file_name(extension: &str, timestamp_ms: u64) -> String {
    format!("mew-paste-{timestamp_ms}.{extension}")
}

pub(super) fn permission_mode_label(id: Option<&str>) -> String {
    let Some(id) = id.filter(|value| !value.trim().is_empty()) else {
        return "permissions".into();
    };
    PERMISSION_MODES
        .iter()
        .find(|(mode_id, ..)| *mode_id == id)
        .map(|(_, label, _)| (*label).to_owned())
        .unwrap_or_else(|| id.to_owned())
}

/// Thinking variants offered by the session's current model, empty when the
/// model doesn't support configurable thinking.
pub(super) fn thinking_variants_for_model(
    models: &[mew_protocol::ModelInfo],
    provider: Option<&str>,
    model: Option<&str>,
) -> Vec<String> {
    let (Some(provider), Some(model)) = (provider, model) else {
        return Vec::new();
    };
    models
        .iter()
        .find(|entry| entry.provider == provider && entry.model == model)
        .map(|entry| {
            entry
                .thinking_variants
                .iter()
                .map(|variant| variant.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn picker_popup_position_in_window(
    trigger_bounds: Bounds<Pixels>,
    popup_height: Pixels,
    window_height: Pixels,
    gap: Pixels,
    margin: Pixels,
) -> Point<Pixels> {
    let trigger_top = f32::from(trigger_bounds.origin.y);
    let trigger_bottom = trigger_top + f32::from(trigger_bounds.size.height);
    let popup_height = f32::from(popup_height);
    let gap = f32::from(gap);
    let margin = f32::from(margin);
    let window_height = f32::from(window_height);
    let above = trigger_top - popup_height - gap;
    let below = trigger_bottom + gap;
    let min_y = margin;
    let max_y = (window_height - popup_height - margin).max(min_y);
    let y = if above >= min_y {
        above
    } else if below <= max_y {
        below
    } else {
        above.clamp(min_y, max_y)
    };

    point(trigger_bounds.origin.x, px(y))
}

pub(super) fn pending_actions_anchor(composer_bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(
        composer_bounds.origin.x - px(12.),
        composer_bounds.origin.y - px(20.),
    )
}

pub(super) fn pending_actions_width(composer_bounds: Bounds<Pixels>) -> Pixels {
    composer_bounds.size.width + px(24.)
}

pub(super) fn cached_markdown_source_unchanged(
    cached: &CachedMarkdown,
    source: &str,
    source_identity: usize,
) -> bool {
    cached.source_len == source.len()
        && (cached.source_identity == source_identity || cached.source == source)
}

pub(super) fn selected_session_path(
    conversations: &[ConversationItem],
    selected_session: Option<&str>,
) -> Option<String> {
    selected_session
        .and_then(|selected| {
            conversations
                .iter()
                .find(|conversation| conversation.session_id == selected)
        })
        .and_then(|conversation| conversation.cwd.as_deref())
        .filter(|path| !path.is_empty())
        .map(display_session_path)
}

pub(super) fn show_ungrouped_group(group_count: usize, session_count: usize) -> bool {
    group_count > 0 && session_count > 0
}

/// Builds the sidebar row model: toolbar, groups with their visible
/// (non-archived) sessions, ungrouped sessions, and a collapsed-by-default
/// "Archived" section so archived conversations stay reachable.
pub(super) fn build_sidebar_rows(
    conversations: &[ConversationItem],
    groups: &[mew_protocol::GroupInfo],
    collapsed_groups: &BTreeSet<String>,
) -> Vec<SidebarRow> {
    let mut rows = vec![SidebarRow::Toolbar];
    let mut grouped_session_ids = BTreeSet::new();
    let mut conversations = conversations.to_vec();
    conversations.sort_by(|a, b| {
        b.last_message_at
            .cmp(&a.last_message_at)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });

    for group in groups {
        let sessions: Vec<_> = conversations
            .iter()
            .filter(|conversation| {
                !conversation.archived
                    && conversation.group_id.as_deref() == Some(group.id.as_str())
            })
            .cloned()
            .collect();
        grouped_session_ids.extend(
            sessions
                .iter()
                .map(|conversation| conversation.session_id.clone()),
        );
        let collapsed = collapsed_groups.contains(&group.id);
        rows.push(SidebarRow::Group {
            id: group.id.clone(),
            name: group.name.clone(),
            color: group.color.clone(),
            count: sessions.len(),
            collapsed,
        });
        if !collapsed {
            rows.extend(sessions.into_iter().map(SidebarRow::Session));
        }
    }

    let ungrouped: Vec<_> = conversations
        .iter()
        .filter(|conversation| {
            !conversation.archived && !grouped_session_ids.contains(&conversation.session_id)
        })
        .cloned()
        .collect();
    if show_ungrouped_group(groups.len(), ungrouped.len()) {
        let collapsed = collapsed_groups.contains(UNGROUPED_GROUP_ID);
        rows.push(SidebarRow::Group {
            id: UNGROUPED_GROUP_ID.into(),
            name: "Ungrouped".into(),
            color: None,
            count: ungrouped.len(),
            collapsed,
        });
        if !collapsed {
            rows.extend(ungrouped.into_iter().map(SidebarRow::Session));
        }
    } else {
        rows.extend(ungrouped.into_iter().map(SidebarRow::Session));
    }

    let archived: Vec<_> = conversations
        .iter()
        .filter(|conversation| conversation.archived)
        .cloned()
        .collect();
    if !archived.is_empty() {
        let collapsed = collapsed_groups.contains(ARCHIVED_GROUP_ID);
        rows.push(SidebarRow::Group {
            id: ARCHIVED_GROUP_ID.into(),
            name: "Archived".into(),
            color: None,
            count: archived.len(),
            collapsed,
        });
        if !collapsed {
            rows.extend(archived.into_iter().map(SidebarRow::Session));
        }
    }
    rows
}

pub(super) fn display_path_from_home(path: &str, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.to_owned();
    };
    let path = Path::new(path);
    let Ok(relative) = path.strip_prefix(home) else {
        return path.to_string_lossy().into_owned();
    };
    if relative.as_os_str().is_empty() {
        "~".into()
    } else {
        format!("~/{}", relative.to_string_lossy())
    }
}

pub(super) fn compact_session_title(title: &str) -> String {
    const MAX_TITLE_CHARS: usize = 28;

    let mut normalized = String::with_capacity(title.len());
    for word in title.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(word);
    }
    let mut chars = normalized.chars();
    let compact = chars.by_ref().take(MAX_TITLE_CHARS).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}…")
    } else {
        compact
    }
}

pub(super) fn latest_user_prompt(transcript: &[TranscriptItem]) -> Option<String> {
    transcript
        .iter()
        .rev()
        .find(|item| item.role == TranscriptRole::User)
        .map(|item| item.text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TranscriptAttention {
    Running,
    Failed,
    Waiting,
}

pub(super) fn transcript_attention(
    last_message_is_user: bool,
    running: bool,
    last_turn_failed: bool,
    has_pending_action: bool,
) -> Option<TranscriptAttention> {
    if !last_message_is_user || has_pending_action {
        return None;
    }
    if running {
        Some(TranscriptAttention::Running)
    } else if last_turn_failed {
        Some(TranscriptAttention::Failed)
    } else {
        Some(TranscriptAttention::Waiting)
    }
}

pub(super) fn client_events_require_shell_render(events: &[ClientEvent]) -> bool {
    events.is_empty()
        || events
            .iter()
            .any(|event| !matches!(event, ClientEvent::TerminalOutput { .. }))
}

pub(super) fn client_events_are_streaming_only(events: &[ClientEvent]) -> bool {
    !events.is_empty()
        && events.iter().all(|event| {
            matches!(
                event,
                ClientEvent::TextDelta { .. } | ClientEvent::ToolProgress { .. }
            )
        })
}

pub(super) fn client_event_requires_metadata_sync(event: &ClientEvent) -> bool {
    !matches!(
        event,
        ClientEvent::TextDelta { .. }
            | ClientEvent::ToolProgress { .. }
            | ClientEvent::TerminalOpened { .. }
            | ClientEvent::TerminalOutput { .. }
            | ClientEvent::TerminalExited { .. }
            | ClientEvent::TerminalError { .. }
            | ClientEvent::PermissionModeChanged { .. }
    )
}

pub(super) fn client_update_requires_metadata_sync(events: &[ClientEvent]) -> bool {
    events.is_empty() || events.iter().any(client_event_requires_metadata_sync)
}

pub(super) fn client_event_requires_transcript_snapshot(event: &ClientEvent) -> bool {
    matches!(
        event,
        ClientEvent::SessionReady { .. }
            | ClientEvent::SessionHistoryLoaded { .. }
            | ClientEvent::MessageChanged { .. }
            | ClientEvent::RequiredActionChanged { .. }
            | ClientEvent::RequestResolved { .. }
    )
}

/// Activity state (subagents, todos, usage) lives on the attached session,
/// which metadata-only snapshots omit, so these events need the full snapshot.
pub(super) fn client_event_requires_session_snapshot(event: &ClientEvent) -> bool {
    matches!(
        event,
        ClientEvent::SubagentsChanged { .. }
            | ClientEvent::TodosChanged { .. }
            | ClientEvent::UsageChanged { .. }
            | ClientEvent::FlaggedFilesChanged { .. }
    )
}

pub(super) fn format_token_count(count: u64) -> String {
    if count < 1_000 {
        count.to_string()
    } else if count < 1_000_000 {
        format!("{:.1}k", count as f64 / 1_000.)
    } else {
        format!("{:.2}M", count as f64 / 1_000_000.)
    }
}

pub(super) fn usage_summary_label(usage: UsageSummary) -> Option<String> {
    if usage.is_empty() {
        return None;
    }
    Some(format!(
        "{} in · {} out · ${:.4}",
        format_token_count(usage.input_tokens),
        format_token_count(usage.output_tokens),
        usage.cost
    ))
}

pub(super) fn file_status_label(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Added => "added",
        FileStatus::Modified => "modified",
        FileStatus::Deleted => "deleted",
        FileStatus::Renamed => "renamed",
        FileStatus::Unchanged => "unchanged",
        FileStatus::Binary => "binary",
    }
}

pub(super) fn attachment_mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("png") {
        Some("image/png")
    } else if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        Some("image/jpeg")
    } else if extension.eq_ignore_ascii_case("gif") {
        Some("image/gif")
    } else if extension.eq_ignore_ascii_case("webp") {
        Some("image/webp")
    } else if extension.eq_ignore_ascii_case("svg") {
        Some("image/svg+xml")
    } else if extension.eq_ignore_ascii_case("pdf") {
        Some("application/pdf")
    } else if extension.eq_ignore_ascii_case("txt") {
        Some("text/plain")
    } else if extension.eq_ignore_ascii_case("md") {
        Some("text/markdown")
    } else if extension.eq_ignore_ascii_case("json") {
        Some("application/json")
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
pub(super) fn parent_view_handle(window: &Window) -> Option<usize> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr() as usize),
        _ => None,
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn parent_view_handle(_window: &Window) -> Option<usize> {
    None
}

pub(super) fn browser_native_rect(
    bounds: Bounds<Pixels>,
    window_size: gpui::Size<Pixels>,
    visible: bool,
) -> BrowserRect {
    BrowserRect {
        x: f64::from(bounds.origin.x),
        y: f64::from(window_size.height)
            - f64::from(bounds.origin.y)
            - f64::from(bounds.size.height),
        width: f64::from(bounds.size.width).max(1.0),
        height: f64::from(bounds.size.height).max(1.0),
        visible,
    }
}

pub(super) fn browser_initialization_is_needed(
    panel_open: bool,
    portal_present: bool,
    initialization_pending: bool,
    has_error: bool,
) -> bool {
    panel_open && !portal_present && !initialization_pending && !has_error
}

pub(super) fn browser_url_is_navigable(url: &str) -> bool {
    url == "about:blank" || url.starts_with("http://") || url.starts_with("https://")
}

pub(super) fn normalize_browser_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return None;
    }
    if value == "about:blank" {
        return Some(value.to_owned());
    }
    if !value.starts_with("http://") && !value.starts_with("https://") {
        if let Some(colon) = value.find(':') {
            let scheme_or_host = &value[..colon];
            let remainder = &value[colon + 1..];
            let looks_like_host_port = remainder
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit());
            if !looks_like_host_port
                && scheme_or_host
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '.')
            {
                return None;
            }
        }
    }
    let url = if value.starts_with("http://") || value.starts_with("https://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    browser_url_is_navigable(&url).then_some(url)
}

pub(super) fn theme_rgb(theme: &Theme, token: &str) -> gpui::Rgba {
    let color = match theme.resolve(token) {
        Color::Rgb(red, green, blue) => ((red as u32) << 16) | ((green as u32) << 8) | blue as u32,
        Color::Black => 0x000000,
        Color::DarkGray => 0x555555,
        Color::Gray => 0xAAAAAA,
        Color::White => 0xFFFFFF,
        Color::Red => 0xFF0000,
        Color::Green => 0x00FF00,
        Color::Yellow => 0xFFFF00,
        Color::Blue => 0x0000FF,
        Color::Magenta => 0xFF00FF,
        Color::Cyan => 0x00FFFF,
        Color::LightRed => 0xFF5555,
        Color::LightGreen => 0x55FF55,
        Color::LightYellow => 0xFFFF55,
        Color::LightBlue => 0x5555FF,
        Color::LightMagenta => 0xFF55FF,
        Color::LightCyan => 0x55FFFF,
        Color::Indexed(index) => {
            let value = index as u32;
            (value << 16) | (value << 8) | value
        }
        Color::Reset => 0x000000,
    };
    rgb(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_tree_visible_ranges_match_flattened_rows() {
        let entries = BTreeMap::from([
            (
                String::new(),
                vec![
                    DirEntry {
                        name: "src".into(),
                        is_dir: true,
                        size: None,
                    },
                    DirEntry {
                        name: "README.md".into(),
                        is_dir: false,
                        size: Some(1),
                    },
                ],
            ),
            (
                "src".into(),
                vec![DirEntry {
                    name: "main.rs".into(),
                    is_dir: false,
                    size: Some(2),
                }],
            ),
        ]);
        let expanded = BTreeSet::from(["src".to_owned()]);
        let flattened = collect_file_tree_rows(&entries, &expanded);

        assert_eq!(file_tree_row_count(&entries, &expanded), flattened.len());
        assert_eq!(
            file_tree_rows_in_range(&entries, &expanded, 1..3),
            flattened[1..3].to_vec()
        );
        assert!(file_tree_rows_in_range(&entries, &expanded, 3..5).is_empty());
    }
}
