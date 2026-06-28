use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;
use std::collections::BTreeMap;
use std::io;

use mew_config::{Config, ProviderConfig};

const BG_LEFT: Color = Color::Rgb(28, 28, 31);
const BG_RIGHT: Color = Color::Rgb(30, 30, 33);
const BG_STATUS: Color = Color::Rgb(30, 30, 33);
const BG_POPUP: Color = Color::Rgb(34, 34, 38);

/// A discovered plugin with its enabled state.
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub name: String,
    pub path: String,
    pub enabled: bool,
}

#[derive(Clone, PartialEq)]
enum LeftItem {
    General,
    ProvidersHeader,
    Provider(String),
    NewProvider,
    Workspace,
    Plugins,
}

impl LeftItem {
    fn has_fields(&self) -> bool {
        matches!(
            self,
            Self::General | Self::Provider(_) | Self::Workspace | Self::Plugins
        )
    }
}

fn build_left_items(config: &Config) -> Vec<LeftItem> {
    let mut items = vec![LeftItem::General, LeftItem::ProvidersHeader];
    let providers: BTreeMap<&String, _> = config.providers.iter().collect();
    for id in providers.keys() {
        items.push(LeftItem::Provider((*id).clone()));
    }
    items.push(LeftItem::NewProvider);
    items.push(LeftItem::Workspace);
    items.push(LeftItem::Plugins);
    items
}

#[derive(Clone)]
enum RightField {
    DefaultModel,
    ProviderId(String),
    ProviderShape(String),
    ProviderBaseUrl(String),
    ProviderCredRef(String),
    ProviderKind(String),
    ProviderSmall(String),
    ProviderBig(String),
    WorkspaceRoots,
}

impl RightField {
    fn label(&self) -> &'static str {
        match self {
            Self::DefaultModel => "default_model",
            Self::ProviderId(_) => "id",
            Self::ProviderShape(_) => "shape",
            Self::ProviderBaseUrl(_) => "base_url",
            Self::ProviderCredRef(_) => "credential_ref",
            Self::ProviderKind(_) => "kind",
            Self::ProviderSmall(_) => "small",
            Self::ProviderBig(_) => "big",
            Self::WorkspaceRoots => "roots (comma-separated)",
        }
    }

    fn value(&self, config: &Config) -> String {
        match self {
            Self::DefaultModel => config.default_model.clone(),
            Self::ProviderId(id) => id.clone(),
            Self::ProviderShape(id) => config
                .providers
                .get(id)
                .map(|p| p.shape.clone())
                .unwrap_or_default(),
            Self::ProviderBaseUrl(id) => config
                .providers
                .get(id)
                .map(|p| p.base_url.clone())
                .unwrap_or_default(),
            Self::ProviderCredRef(id) => config
                .providers
                .get(id)
                .map(|p| p.credential_ref.clone())
                .unwrap_or_default(),
            Self::ProviderKind(id) => config
                .providers
                .get(id)
                .map(|p| p.kind.clone())
                .unwrap_or_default(),
            Self::ProviderSmall(id) => config
                .providers
                .get(id)
                .map(|p| p.small.clone())
                .unwrap_or_default(),
            Self::ProviderBig(id) => config
                .providers
                .get(id)
                .map(|p| p.big.clone())
                .unwrap_or_default(),
            Self::WorkspaceRoots => config
                .workspace
                .roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    fn apply(&self, config: &mut Config, val: &str) {
        match self {
            Self::DefaultModel => config.default_model = val.into(),
            Self::ProviderId(old_id) => {
                if val.is_empty() || val == old_id {
                    return;
                }
                if let Some(pc) = config.providers.remove(old_id) {
                    config.providers.insert(val.into(), pc);
                }
            }
            Self::ProviderShape(id) => {
                if let Some(p) = config.providers.get_mut(id) {
                    p.shape = val.into();
                }
            }
            Self::ProviderBaseUrl(id) => {
                if let Some(p) = config.providers.get_mut(id) {
                    p.base_url = val.into();
                }
            }
            Self::ProviderCredRef(id) => {
                if let Some(p) = config.providers.get_mut(id) {
                    p.credential_ref = val.into();
                }
            }
            Self::ProviderKind(id) => {
                if let Some(p) = config.providers.get_mut(id) {
                    p.kind = val.into();
                }
            }
            Self::ProviderSmall(id) => {
                if let Some(p) = config.providers.get_mut(id) {
                    p.small = val.into();
                }
            }
            Self::ProviderBig(id) => {
                if let Some(p) = config.providers.get_mut(id) {
                    p.big = val.into();
                }
            }
            Self::WorkspaceRoots => {
                config.workspace.roots = val
                    .split(',')
                    .map(|s| std::path::PathBuf::from(s.trim()))
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect();
            }
        }
    }
}

fn right_fields_for(item: &LeftItem, config: &Config) -> Vec<RightField> {
    match item {
        LeftItem::General => vec![RightField::DefaultModel],
        LeftItem::Provider(id) => {
            let mut fields = vec![
                RightField::ProviderId(id.clone()),
                RightField::ProviderShape(id.clone()),
                RightField::ProviderBaseUrl(id.clone()),
                RightField::ProviderCredRef(id.clone()),
                RightField::ProviderKind(id.clone()),
            ];
            if config.providers.get(id).map(|p| p.kind.as_str()) == Some("router") {
                fields.push(RightField::ProviderSmall(id.clone()));
                fields.push(RightField::ProviderBig(id.clone()));
            }
            fields
        }
        LeftItem::Workspace => vec![RightField::WorkspaceRoots],
        _ => vec![],
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Editing,
    NamingProvider,
}

/// Two-panel config editor with plugin management.
///
/// Used standalone via `run_editor()` and embedded in the main TUI settings page.
pub struct ConfigEditor {
    config: Config,
    left_items: Vec<LeftItem>,
    left_cursor: usize,
    right_cursor: usize,
    active: Panel,
    mode: Mode,
    buf: String,
    dirty: bool,
    message: Option<String>,
    plugins: Vec<PluginEntry>,
    plugin_cursor: usize,
}

impl ConfigEditor {
    pub fn new(config: Config, plugins: Vec<PluginEntry>) -> Self {
        let left_items = build_left_items(&config);
        Self {
            config,
            left_items,
            left_cursor: 1,
            right_cursor: 0,
            active: Panel::Left,
            mode: Mode::Normal,
            buf: String::new(),
            dirty: false,
            message: None,
            plugins,
            plugin_cursor: 0,
        }
    }

    /// Handle a key event. Returns `false` when the editor should close.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Editing => {
                self.handle_edit_key(key);
                true
            }
            Mode::NamingProvider => self.handle_naming_key(key),
        }
    }

    /// Borrow the current config.
    #[allow(dead_code)]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Whether the config has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The current plugin list (with toggle state).
    #[allow(dead_code)]
    pub fn plugins(&self) -> &[PluginEntry] {
        &self.plugins
    }

    fn run(&mut self) -> Result<()> {
        enable_raw_mode().context("enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context("create terminal")?;

        loop {
            terminal.draw(|f| self.draw(f))?;
            if !event::poll(std::time::Duration::from_millis(500))? {
                continue;
            }
            if let Event::Key(key) = event::read()? {
                if !self.handle_key(key) {
                    break;
                }
            }
        }

        disable_raw_mode().context("disable raw mode")?;
        execute!(io::stdout(), LeaveAlternateScreen).context("leave alternate screen")?;
        Ok(())
    }

    fn current_left_item(&self) -> Option<&LeftItem> {
        self.left_items.get(self.left_cursor)
    }

    fn right_fields(&self) -> Vec<RightField> {
        self.current_left_item()
            .map(|item| right_fields_for(item, &self.config))
            .unwrap_or_default()
    }

    fn rebuild_left_items(&mut self) {
        self.left_items = build_left_items(&self.config);
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> bool {
        self.message = None;
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Esc => return false,

            KeyCode::Tab => {
                if self.active == Panel::Left && !self.right_fields().is_empty() {
                    self.active = Panel::Right;
                    self.right_cursor = 0;
                } else {
                    self.active = Panel::Left;
                }
            }

            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match self.save() {
                    Ok(()) => {
                        self.dirty = false;
                        self.message = Some("Saved.".into());
                    }
                    Err(e) => self.message = Some(format!("Save failed: {}", e)),
                }
            }

            KeyCode::Up | KeyCode::Char('k') => match self.active {
                Panel::Left => self.move_left(-1),
                Panel::Right => self.move_right(-1),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.active {
                Panel::Left => self.move_left(1),
                Panel::Right => self.move_right(1),
            },

            KeyCode::Enter => {
                if self.active == Panel::Left {
                    match self.current_left_item() {
                        Some(LeftItem::NewProvider) => {
                            self.mode = Mode::NamingProvider;
                            self.buf = String::new();
                        }
                        Some(LeftItem::Plugins) => {
                            self.active = Panel::Right;
                            self.right_cursor = self.plugin_cursor;
                        }
                        Some(item) if item.has_fields() => {
                            self.active = Panel::Right;
                            self.right_cursor = 0;
                        }
                        _ => {}
                    }
                } else if matches!(self.current_left_item(), Some(LeftItem::Plugins)) {
                    if let Some(plugin) = self.plugins.get_mut(self.right_cursor) {
                        plugin.enabled = !plugin.enabled;
                        self.dirty = true;
                    }
                } else {
                    let fields = self.right_fields();
                    if let Some(field) = fields.get(self.right_cursor) {
                        self.buf = field.value(&self.config);
                        self.mode = Mode::Editing;
                    }
                }
            }

            KeyCode::Char('n') if self.active == Panel::Left => {
                self.mode = Mode::NamingProvider;
                self.buf = String::new();
            }

            KeyCode::Char('d') if self.active == Panel::Left => {
                if let Some(LeftItem::Provider(id)) = self.current_left_item().cloned() {
                    self.config.providers.remove(&id);
                    self.dirty = true;
                    self.rebuild_left_items();
                    self.message = Some(format!("Removed provider '{}'", id));
                }
            }

            _ => {}
        }
        true
    }

    fn handle_edit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let fields = self.right_fields();
                if let Some(field) = fields.get(self.right_cursor) {
                    let was_id_field = matches!(field, RightField::ProviderId(_));
                    field.apply(&mut self.config, &self.buf);
                    self.dirty = true;
                    if was_id_field {
                        self.rebuild_left_items();
                    }
                }
                self.mode = Mode::Normal;
                self.buf.clear();
            }
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.buf.clear();
            }
            KeyCode::Backspace => {
                self.buf.pop();
            }
            KeyCode::Char(c) => {
                self.buf.push(c);
            }
            _ => {}
        }
    }

    fn handle_naming_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.buf.clear();
            }
            KeyCode::Enter => {
                let name = self.buf.trim().to_string();
                if name.is_empty() {
                    self.message = Some("Provider name cannot be empty.".into());
                } else if self.config.providers.contains_key(&name) {
                    self.message = Some(format!("Provider '{}' already exists", name));
                } else {
                    self.config
                        .providers
                        .insert(name.clone(), ProviderConfig::default());
                    self.dirty = true;
                    self.rebuild_left_items();
                    if let Some(pos) = self
                        .left_items
                        .iter()
                        .position(|i| matches!(i, LeftItem::Provider(id) if id == &name))
                    {
                        self.left_cursor = pos;
                    }
                    self.active = Panel::Right;
                    self.right_cursor = 0;
                    self.mode = Mode::Normal;
                    self.message = Some(format!("Created provider '{}'", name));
                }
                self.buf.clear();
                if self.mode == Mode::NamingProvider {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Backspace => {
                self.buf.pop();
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' {
                    return false;
                }
                if c.is_ascii_graphic() || c == ' ' {
                    self.buf.push(c);
                }
            }
            _ => {}
        }
        true
    }

    fn move_left(&mut self, delta: i32) {
        let mut idx = self.left_cursor as i32 + delta;
        loop {
            if idx < 0 || idx as usize >= self.left_items.len() {
                return;
            }
            let item = &self.left_items[idx as usize];
            if !matches!(item, LeftItem::ProvidersHeader) {
                break;
            }
            idx += delta;
        }
        self.left_cursor = idx as usize;
        self.right_cursor = 0;
    }

    fn move_right(&mut self, delta: i32) {
        let len = if matches!(self.current_left_item(), Some(LeftItem::Plugins)) {
            self.plugins.len()
        } else {
            self.right_fields().len()
        };
        if len == 0 {
            return;
        }
        let mut idx = self.right_cursor as i32 + delta;
        if idx < 0 {
            idx = 0;
        }
        if idx as usize >= len {
            idx = (len - 1) as i32;
        }
        self.right_cursor = idx as usize;
        if matches!(self.current_left_item(), Some(LeftItem::Plugins)) {
            self.plugin_cursor = self.right_cursor;
        }
    }

    /// Save the config and runtime state (including plugin enable/disable) to disk.
    ///
    /// `config.toml` is overwritten with the in-memory config. `state.toml` is
    /// read first and updated in place so that fields owned by other code
    /// paths (`last_model`, `last_provider`, `sidebar_collapsed`) are
    /// preserved.
    pub fn save(&self) -> Result<()> {
        let config_path = mew_config::config_dir().join("config.toml");
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent).context("create config dir")?;
        }
        let toml = toml::to_string_pretty(&self.config)
            .map_err(|e| anyhow::anyhow!("serialize config: {}", e))?;
        std::fs::write(&config_path, toml).context("write config file")?;
        tracing::info!(path = ?config_path, "config saved");

        let disabled_plugins = self
            .plugins
            .iter()
            .filter(|p| !p.enabled)
            .map(|p| p.name.clone())
            .collect();
        let mut state = mew_config::load_state().unwrap_or_default();
        state.disabled_plugins = disabled_plugins;
        if let Err(e) = mew_config::save_state(&state) {
            return Err(anyhow::anyhow!("save state: {}", e));
        }
        tracing::info!("plugin enable/disable state saved");
        Ok(())
    }

    pub fn draw(&self, f: &mut ratatui::Frame) {
        let area = f.area();

        f.render_widget(Block::default().style(Style::default().bg(BG_RIGHT)), area);

        let title = if self.dirty {
            " mew config editor * "
        } else {
            " mew config editor "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(BG_RIGHT))
            .title(Line::from(Span::styled(
                title,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        f.render_widget(block, area);

        let inner = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(3),
        );

        let panes = Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(inner);
        let left_area = panes[0];
        let right_area = panes[1];

        self.draw_left_panel(f, left_area);
        self.draw_right_panel(f, right_area);
        self.draw_status_bar(f, area);
        self.draw_message_popup(f, area);

        if self.mode == Mode::NamingProvider {
            self.draw_naming_popup(f, area);
        }
    }

    fn draw_left_panel(&self, f: &mut ratatui::Frame, area: Rect) {
        let active = self.active == Panel::Left && self.mode == Mode::Normal;
        let block = Block::default()
            .borders(Borders::RIGHT)
            .style(Style::default().bg(BG_LEFT))
            .title(Span::styled(
                " categories ",
                Style::default().fg(Color::DarkGray).bg(BG_LEFT),
            ));
        f.render_widget(block, area);

        let inner = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );

        let mut lines: Vec<Line> = Vec::new();
        for (i, item) in self.left_items.iter().enumerate() {
            let selected = i == self.left_cursor && active;
            let cursor_marker = if selected { "▸ " } else { "  " };

            let line = match item {
                LeftItem::General | LeftItem::Workspace => {
                    let label = match item {
                        LeftItem::General => "General",
                        LeftItem::Workspace => "Workspace",
                        _ => "",
                    };
                    Line::from(vec![
                        Span::raw(cursor_marker),
                        Span::styled(
                            label,
                            Style::default()
                                .fg(if selected {
                                    Color::Yellow
                                } else {
                                    Color::White
                                })
                                .bg(BG_LEFT)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ])
                }
                LeftItem::ProvidersHeader => Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        "Providers",
                        Style::default()
                            .fg(Color::White)
                            .bg(BG_LEFT)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                LeftItem::Provider(id) => {
                    let pc = self.config.providers.get(id);
                    let shape = pc.map(|p| p.shape.as_str()).unwrap_or("?");
                    Line::from(vec![
                        Span::raw(cursor_marker),
                        Span::styled(
                            format!("  {} ", id),
                            Style::default()
                                .fg(if selected { Color::Yellow } else { Color::Gray })
                                .bg(BG_LEFT),
                        ),
                        Span::styled(shape, Style::default().fg(Color::DarkGray).bg(BG_LEFT)),
                    ])
                }
                LeftItem::NewProvider => Line::from(vec![
                    Span::raw(cursor_marker),
                    Span::styled(
                        "  + new provider",
                        Style::default()
                            .fg(if selected {
                                Color::Green
                            } else {
                                Color::DarkGray
                            })
                            .bg(BG_LEFT),
                    ),
                ]),
                LeftItem::Plugins => Line::from(vec![
                    Span::raw(cursor_marker),
                    Span::styled(
                        "Plugins",
                        Style::default()
                            .fg(if selected {
                                Color::Yellow
                            } else {
                                Color::White
                            })
                            .bg(BG_LEFT)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
            };
            lines.push(line);
        }

        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(BG_LEFT)),
            inner,
        );
    }

    fn draw_right_panel(&self, f: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .style(Style::default().bg(BG_RIGHT))
            .title(Span::styled(
                " details ",
                Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
            ));
        f.render_widget(block, area);

        let inner = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );

        let item = self.current_left_item();
        let fields = self.right_fields();

        let mut lines: Vec<Line> = Vec::new();

        match item {
            Some(LeftItem::General) => {
                lines.push(Line::from(Span::styled(
                    "General",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::Provider(id)) => {
                lines.push(Line::from(Span::styled(
                    format!("Provider: {}", id),
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    "  Edit fields below, press Enter to edit.",
                    Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::Workspace) => {
                lines.push(Line::from(Span::styled(
                    "Workspace",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::NewProvider) => {
                lines.push(Line::from(Span::styled(
                    "Create a new provider",
                    Style::default().fg(Color::Green).bg(BG_RIGHT),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from("Press Enter or 'n' to add a new provider."));
                lines.push(Line::from(
                    "Empty roots defaults to current working directory.",
                ));
                lines.push(Line::from(
                    "Workspace roots are checked against path args of bash, shell_background,",
                ));
                lines.push(Line::from(
                    "and shell_monitor commands. Args outside the roots escalate to Prompt.",
                ));
                f.render_widget(
                    Paragraph::new(lines).style(Style::default().bg(BG_RIGHT)),
                    inner,
                );
                return;
            }
            Some(LeftItem::Plugins) => {
                lines.push(Line::from(Span::styled(
                    "Plugins",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    "  Enter to toggle. Disabled plugins are skipped on startup.",
                    Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
                )));
                lines.push(Line::from(""));

                if self.plugins.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  No plugins discovered",
                        Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
                    )));
                } else {
                    let active = self.active == Panel::Right && self.mode == Mode::Normal;
                    for (i, plugin) in self.plugins.iter().enumerate() {
                        let selected = i == self.right_cursor && active;
                        let icon = if plugin.enabled { "✓" } else { "✗" };
                        let icon_color = if plugin.enabled {
                            Color::Green
                        } else {
                            Color::Red
                        };
                        let name_style = if selected {
                            Style::default().fg(Color::Yellow).bg(BG_RIGHT)
                        } else {
                            Style::default().fg(Color::Gray).bg(BG_RIGHT)
                        };
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("  {} ", icon),
                                Style::default().fg(icon_color).bg(BG_RIGHT),
                            ),
                            Span::styled(plugin.name.clone(), name_style),
                            Span::styled(
                                format!("  {}", plugin.path),
                                Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
                            ),
                        ]));
                    }
                }
                f.render_widget(
                    Paragraph::new(lines).style(Style::default().bg(BG_RIGHT)),
                    inner,
                );
                return;
            }
            _ => {
                lines.push(Line::from(Span::styled(
                    "Select an item on the left",
                    Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
                )));
                f.render_widget(
                    Paragraph::new(lines).style(Style::default().bg(BG_RIGHT)),
                    inner,
                );
                return;
            }
        }

        let active = self.active == Panel::Right && self.mode == Mode::Normal;
        for (i, field) in fields.iter().enumerate() {
            let selected = i == self.right_cursor && active;
            let editing = selected && self.mode == Mode::Editing;

            let val = if editing {
                format!("{}_│", self.buf)
            } else {
                let v = field.value(&self.config);
                if v.is_empty() {
                    "(empty)".into()
                } else {
                    v
                }
            };

            let val_style = if editing {
                Style::default().fg(Color::Green).bg(BG_RIGHT)
            } else if selected {
                Style::default().fg(Color::Yellow).bg(BG_RIGHT)
            } else {
                Style::default().fg(Color::Gray).bg(BG_RIGHT)
            };

            let label_style = if selected {
                Style::default().fg(Color::Yellow).bg(BG_RIGHT)
            } else {
                Style::default().fg(Color::Gray).bg(BG_RIGHT)
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {:<16} ", field.label()), label_style),
                Span::styled(val, val_style),
            ]));
        }

        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(BG_RIGHT)),
            inner,
        );
    }

    fn draw_status_bar(&self, f: &mut ratatui::Frame, area: Rect) {
        let bar_area = Rect::new(
            area.x + 1,
            area.y + area.height - 2,
            area.width.saturating_sub(2),
            1,
        );

        let help = match self.mode {
            Mode::Normal => match self.active {
                Panel::Left => " Tab→details · Enter open · n new · d delete · ^S save · Esc quit",
                Panel::Right => " Tab←list · Enter edit · ^S save · Esc quit",
            },
            Mode::Editing => " Enter confirm · Esc cancel",
            Mode::NamingProvider => " Enter create · Esc cancel",
        };

        let panel_indicator = match self.active {
            Panel::Left => " [list]",
            Panel::Right => " [details]",
        };

        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{}{}", panel_indicator, help),
                Style::default().fg(Color::DarkGray).bg(BG_STATUS),
            ))
            .style(Style::default().bg(BG_STATUS)),
            bar_area,
        );
    }

    fn draw_message_popup(&self, f: &mut ratatui::Frame, area: Rect) {
        if self.message.is_none() || self.mode == Mode::NamingProvider {
            return;
        }
        let msg = self.message.as_ref().unwrap();
        let width = msg.len().max(20) as u16 + 4;
        let popup = centered_rect(width, 3, area);
        f.render_widget(Clear, popup);
        f.render_widget(
            Paragraph::new(msg.as_str())
                .style(Style::default().fg(Color::Green).bg(BG_POPUP))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().bg(BG_POPUP))
                        .border_style(Style::default().fg(Color::Green)),
                ),
            popup,
        );
    }

    fn draw_naming_popup(&self, f: &mut ratatui::Frame, area: Rect) {
        let popup = centered_rect(50, 5, area);
        f.render_widget(Clear, popup);

        let lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  name: ", Style::default().fg(Color::Cyan).bg(BG_POPUP)),
                Span::styled(
                    format!("{}_│", self.buf),
                    Style::default().fg(Color::Green).bg(BG_POPUP),
                ),
            ]),
            Line::from(""),
        ];

        f.render_widget(
            Paragraph::new(lines)
                .style(Style::default().bg(BG_POPUP))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().bg(BG_POPUP))
                        .title(Span::styled(
                            " New Provider ",
                            Style::default()
                                .fg(Color::Cyan)
                                .bg(BG_POPUP)
                                .add_modifier(Modifier::BOLD),
                        ))
                        .border_style(Style::default().fg(Color::Cyan)),
                ),
            popup,
        );
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let popup_layout = Layout::vertical([
        Constraint::Length(area.height.saturating_sub(h) / 2),
        Constraint::Length(h),
        Constraint::Length(area.height.saturating_sub(h) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Length(area.width.saturating_sub(w) / 2),
        Constraint::Length(w),
        Constraint::Length(area.width.saturating_sub(w) / 2),
    ])
    .split(popup_layout[1])[1]
}

pub fn run_editor() -> Result<()> {
    let config = mew_config::load().context("load config")?;
    let state = mew_config::load_state().unwrap_or_default();
    let loader =
        mew_hooks_runtime::PluginLoader::new(mew_hooks_runtime::PluginLoader::default_dirs());
    let plugins: Vec<PluginEntry> = loader
        .discover_executables()
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let enabled = !state.disabled_plugins.contains(&name);
            PluginEntry {
                name,
                path: path.display().to_string(),
                enabled,
            }
        })
        .collect();
    let mut editor = ConfigEditor::new(config, plugins);
    editor.run()?;
    Ok(())
}
