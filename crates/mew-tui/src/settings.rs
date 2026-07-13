//! In-app settings/config editor state and rendering.
//!
//! Unlike the standalone `config_editor.rs` in the `mew` binary, this lives
//! inside the main TUI `App` so it can be driven by `mew_tui::harness` and
//! captured with `mew tui-capture`.

use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};
use std::collections::BTreeMap;

use crate::theme::ThemeTokens;
use crate::ui::overlays::centered_rect;

/// A discovered plugin entry with its on-disk path and enabled flag.
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub name: String,
    pub path: String,
    pub enabled: bool,
}

/// Top-level category shown in the left panel.
#[derive(Debug, Clone, PartialEq)]
pub enum Category {
    General,
    Accounts,
    Models,
    Permissions,
    Secrets,
    Workspace,
    Tui,
    Plugins,
}

impl Category {
    fn label(&self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Accounts => "Accounts",
            Category::Models => "Models",
            Category::Permissions => "Permissions",
            Category::Secrets => "Secrets",
            Category::Workspace => "Workspace",
            Category::Tui => "TUI",
            Category::Plugins => "Plugins",
        }
    }
}

/// A selectable item in the left panel. Some items are action rows (Add
/// account/model/rule), some open detail fields on the right.
#[derive(Debug, Clone, PartialEq)]
enum LeftItem {
    Category(Category),
    Provider(String),
    AddProvider,
    Model(usize),
    AddModel,
    PermissionRule(usize),
    AddPermissionRule,
    SecretFilesGroup(usize),
    AddSecretFilesGroup,
    SecretWordsGroup(usize),
    AddSecretWordsGroup,
    Plugin(usize),
}

impl LeftItem {
    fn label(&self, state: &SettingsState) -> String {
        match self {
            LeftItem::Category(c) => c.label().to_string(),
            LeftItem::Provider(id) => id.clone(),
            LeftItem::AddProvider => "+ add account".to_string(),
            LeftItem::Model(i) => state
                .config
                .models
                .get(*i)
                .map(|m| m.id.clone())
                .unwrap_or_else(|| format!("model {}", i)),
            LeftItem::AddModel => "+ add model".to_string(),
            LeftItem::PermissionRule(i) => format!("rule {}", i + 1),
            LeftItem::AddPermissionRule => "+ add permission rule".to_string(),
            LeftItem::SecretFilesGroup(i) => format!("files group {}", i + 1),
            LeftItem::AddSecretFilesGroup => "+ add files group".to_string(),
            LeftItem::SecretWordsGroup(i) => format!("words group {}", i + 1),
            LeftItem::AddSecretWordsGroup => "+ add words group".to_string(),
            LeftItem::Plugin(i) => state
                .plugins
                .get(*i)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
        }
    }

    fn has_fields(&self) -> bool {
        matches!(
            self,
            LeftItem::Category(Category::General)
                | LeftItem::Category(Category::Workspace)
                | LeftItem::Category(Category::Tui)
                | LeftItem::Provider(_)
                | LeftItem::Model(_)
                | LeftItem::PermissionRule(_)
                | LeftItem::SecretFilesGroup(_)
                | LeftItem::SecretWordsGroup(_)
        )
    }
}

/// A single editable field shown in the right panel.
#[derive(Debug, Clone)]
enum Field {
    // General
    DefaultModel,
    DefaultPersona,
    PlanPath,

    // Provider
    ProviderId(String),
    ProviderShape(String),
    ProviderBaseUrl(String),
    ProviderCredRef(String),
    ProviderKind(String),
    ProviderNano(String),
    ProviderMicro(String),
    ProviderDeci(String),
    ProviderDisableHashline(String),

    // Model
    ModelId(usize),
    ModelProvider(usize),
    ModelShape(usize),
    ModelContextWindow(usize),
    ModelResponsesLite(usize),
    ModelMerge(usize),

    // Permissions
    ClassifierProvider,
    ClassifierModel,
    PermissionDecision(usize),
    PermissionTool(usize),
    PermissionCommandPrefix(usize),
    PermissionCommandProgram(usize),
    PermissionCommandSubcommand(usize),
    PermissionPathGlob(usize),

    // Secrets
    SecretFilesPaths(usize),
    SecretWordsValues(usize),

    // Workspace
    WorkspaceRoots,

    // TUI
    TuiTheme,
}

impl Field {
    fn label(&self) -> &'static str {
        match self {
            Field::DefaultModel => "default_model",
            Field::DefaultPersona => "default_persona",
            Field::PlanPath => "plan_path",
            Field::ProviderId(_) => "id",
            Field::ProviderShape(_) => "shape",
            Field::ProviderBaseUrl(_) => "base_url",
            Field::ProviderCredRef(_) => "credential_ref",
            Field::ProviderKind(_) => "kind",
            Field::ProviderNano(_) => "nano",
            Field::ProviderMicro(_) => "micro",
            Field::ProviderDeci(_) => "deci",
            Field::ProviderDisableHashline(_) => "disable_hashline",
            Field::ModelId(_) => "id",
            Field::ModelProvider(_) => "provider",
            Field::ModelShape(_) => "shape",
            Field::ModelContextWindow(_) => "context_window",
            Field::ModelResponsesLite(_) => "responses_lite",
            Field::ModelMerge(_) => "merge",
            Field::ClassifierProvider => "classifier_provider",
            Field::ClassifierModel => "classifier_model",
            Field::PermissionDecision(_) => "decision",
            Field::PermissionTool(_) => "tool",
            Field::PermissionCommandPrefix(_) => "command_prefix",
            Field::PermissionCommandProgram(_) => "command_program",
            Field::PermissionCommandSubcommand(_) => "command_subcommand",
            Field::PermissionPathGlob(_) => "path_glob",
            Field::SecretFilesPaths(_) => "paths",
            Field::SecretWordsValues(_) => "values",
            Field::WorkspaceRoots => "roots",
            Field::TuiTheme => "theme",
        }
    }

    fn value(&self, state: &SettingsState) -> String {
        let cfg = &state.config;
        match self {
            Field::DefaultModel => cfg.default_model.clone(),
            Field::DefaultPersona => cfg.default_persona.clone(),
            Field::PlanPath => cfg.plan_path.clone(),
            Field::ProviderId(id) => id.clone(),
            Field::ProviderShape(id) => cfg
                .providers
                .get(id)
                .map(|p| p.shape.clone())
                .unwrap_or_default(),
            Field::ProviderBaseUrl(id) => cfg
                .providers
                .get(id)
                .map(|p| p.base_url.clone())
                .unwrap_or_default(),
            Field::ProviderCredRef(id) => cfg
                .providers
                .get(id)
                .map(|p| p.credential_ref.clone())
                .unwrap_or_default(),
            Field::ProviderKind(id) => cfg
                .providers
                .get(id)
                .map(|p| p.kind.clone())
                .unwrap_or_default(),
            Field::ProviderNano(id) => cfg
                .providers
                .get(id)
                .map(|p| p.nano.clone())
                .unwrap_or_default(),
            Field::ProviderMicro(id) => cfg
                .providers
                .get(id)
                .map(|p| p.micro_model().to_string())
                .unwrap_or_default(),
            Field::ProviderDeci(id) => cfg
                .providers
                .get(id)
                .map(|p| p.deci_model().to_string())
                .unwrap_or_default(),
            Field::ProviderDisableHashline(id) => cfg
                .providers
                .get(id)
                .map(|p| p.disable_hashline.to_string())
                .unwrap_or_else(|| "false".into()),
            Field::ModelId(i) => cfg.models.get(*i).map(|m| m.id.clone()).unwrap_or_default(),
            Field::ModelProvider(i) => cfg
                .models
                .get(*i)
                .map(|m| m.provider.clone())
                .unwrap_or_default(),
            Field::ModelShape(i) => cfg
                .models
                .get(*i)
                .map(|m| m.shape.clone())
                .unwrap_or_default(),
            Field::ModelContextWindow(i) => cfg
                .models
                .get(*i)
                .map(|m| m.context_window.to_string())
                .unwrap_or_default(),
            Field::ModelResponsesLite(i) => cfg
                .models
                .get(*i)
                .map(|m| m.responses_lite.to_string())
                .unwrap_or_else(|| "false".into()),
            Field::ModelMerge(i) => cfg
                .models
                .get(*i)
                .map(|m| m.merge.to_string())
                .unwrap_or_else(|| "false".into()),
            Field::ClassifierProvider => cfg
                .permissions
                .classifier_provider
                .clone()
                .unwrap_or_default(),
            Field::ClassifierModel => cfg.permissions.classifier_model.clone().unwrap_or_default(),
            Field::PermissionDecision(i) => cfg
                .permissions
                .rules
                .get(*i)
                .map(|r| rule_decision_str(&r.decision).to_string())
                .unwrap_or_default(),
            Field::PermissionTool(i) => cfg
                .permissions
                .rules
                .get(*i)
                .map(|r| r.tool.clone())
                .unwrap_or_default(),
            Field::PermissionCommandPrefix(i) => cfg
                .permissions
                .rules
                .get(*i)
                .map(|r| r.r#match.command_prefix.clone().unwrap_or_default())
                .unwrap_or_default(),
            Field::PermissionCommandProgram(i) => cfg
                .permissions
                .rules
                .get(*i)
                .map(|r| r.r#match.command_program.clone().unwrap_or_default())
                .unwrap_or_default(),
            Field::PermissionCommandSubcommand(i) => cfg
                .permissions
                .rules
                .get(*i)
                .map(|r| r.r#match.command_subcommand.clone().unwrap_or_default())
                .unwrap_or_default(),
            Field::PermissionPathGlob(i) => cfg
                .permissions
                .rules
                .get(*i)
                .map(|r| r.r#match.path_glob.clone().unwrap_or_default())
                .unwrap_or_default(),
            Field::SecretFilesPaths(i) => cfg
                .secrets
                .files
                .get(*i)
                .map(|g| g.paths.join(", "))
                .unwrap_or_default(),
            Field::SecretWordsValues(i) => cfg
                .secrets
                .words
                .get(*i)
                .map(|g| g.values.join(", "))
                .unwrap_or_default(),
            Field::WorkspaceRoots => cfg
                .workspace
                .roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            Field::TuiTheme => cfg.tui.theme.clone(),
        }
    }

    fn apply(&self, state: &mut SettingsState, val: &str) {
        let cfg = &mut state.config;
        match self {
            Field::DefaultModel => cfg.default_model = val.into(),
            Field::DefaultPersona => cfg.default_persona = val.into(),
            Field::PlanPath => cfg.plan_path = val.into(),
            Field::ProviderId(old_id) => {
                if val.is_empty() || val == old_id {
                    return;
                }
                if let Some(pc) = cfg.providers.remove(old_id) {
                    cfg.providers.insert(val.into(), pc);
                }
            }
            Field::ProviderShape(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.shape = val.into();
                }
            }
            Field::ProviderBaseUrl(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.base_url = val.into();
                }
            }
            Field::ProviderCredRef(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.credential_ref = val.into();
                }
            }
            Field::ProviderKind(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.kind = val.into();
                }
            }
            Field::ProviderNano(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.nano = val.into();
                }
            }
            Field::ProviderMicro(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.micro = val.into();
                }
            }
            Field::ProviderDeci(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.deci = val.into();
                }
            }
            Field::ProviderDisableHashline(id) => {
                if let Some(p) = cfg.providers.get_mut(id) {
                    p.disable_hashline = matches!(
                        val.trim().to_ascii_lowercase().as_str(),
                        "true" | "1" | "yes"
                    );
                }
            }
            Field::ModelId(i) => {
                if let Some(m) = cfg.models.get_mut(*i) {
                    m.id = val.into();
                }
            }
            Field::ModelProvider(i) => {
                if let Some(m) = cfg.models.get_mut(*i) {
                    m.provider = val.into();
                }
            }
            Field::ModelShape(i) => {
                if let Some(m) = cfg.models.get_mut(*i) {
                    m.shape = val.into();
                }
            }
            Field::ModelContextWindow(i) => {
                if let Some(m) = cfg.models.get_mut(*i) {
                    m.context_window = val.parse().unwrap_or(0);
                }
            }
            Field::ModelResponsesLite(i) => {
                if let Some(m) = cfg.models.get_mut(*i) {
                    m.responses_lite = matches!(
                        val.trim().to_ascii_lowercase().as_str(),
                        "true" | "1" | "yes"
                    );
                }
            }
            Field::ModelMerge(i) => {
                if let Some(m) = cfg.models.get_mut(*i) {
                    m.merge = matches!(
                        val.trim().to_ascii_lowercase().as_str(),
                        "true" | "1" | "yes"
                    );
                }
            }
            Field::ClassifierProvider => {
                cfg.permissions.classifier_provider = if val.is_empty() {
                    None
                } else {
                    Some(val.into())
                };
            }
            Field::ClassifierModel => {
                cfg.permissions.classifier_model = if val.is_empty() {
                    None
                } else {
                    Some(val.into())
                };
            }
            Field::PermissionDecision(i) => {
                if let Some(r) = cfg.permissions.rules.get_mut(*i) {
                    r.decision = match val.trim().to_ascii_lowercase().as_str() {
                        "allow" => mew_config::permissions::RuleDecision::Allow,
                        "deny" => mew_config::permissions::RuleDecision::Deny,
                        _ => mew_config::permissions::RuleDecision::Ask,
                    };
                }
            }
            Field::PermissionTool(i) => {
                if let Some(r) = cfg.permissions.rules.get_mut(*i) {
                    r.tool = val.into();
                }
            }
            Field::PermissionCommandPrefix(i) => {
                if let Some(r) = cfg.permissions.rules.get_mut(*i) {
                    r.r#match.command_prefix = if val.is_empty() {
                        None
                    } else {
                        Some(val.into())
                    };
                }
            }
            Field::PermissionCommandProgram(i) => {
                if let Some(r) = cfg.permissions.rules.get_mut(*i) {
                    r.r#match.command_program = if val.is_empty() {
                        None
                    } else {
                        Some(val.into())
                    };
                }
            }
            Field::PermissionCommandSubcommand(i) => {
                if let Some(r) = cfg.permissions.rules.get_mut(*i) {
                    r.r#match.command_subcommand = if val.is_empty() {
                        None
                    } else {
                        Some(val.into())
                    };
                }
            }
            Field::PermissionPathGlob(i) => {
                if let Some(r) = cfg.permissions.rules.get_mut(*i) {
                    r.r#match.path_glob = if val.is_empty() {
                        None
                    } else {
                        Some(val.into())
                    };
                }
            }
            Field::SecretFilesPaths(i) => {
                if let Some(g) = cfg.secrets.files.get_mut(*i) {
                    g.paths = split_comma(val);
                }
            }
            Field::SecretWordsValues(i) => {
                if let Some(g) = cfg.secrets.words.get_mut(*i) {
                    g.values = split_comma(val);
                }
            }
            Field::WorkspaceRoots => {
                cfg.workspace.roots = val
                    .split(',')
                    .map(|s| std::path::PathBuf::from(s.trim()))
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect();
            }
            Field::TuiTheme => cfg.tui.theme = val.into(),
        }
    }
}

fn rule_decision_str(d: &mew_config::permissions::RuleDecision) -> &'static str {
    match d {
        mew_config::permissions::RuleDecision::Allow => "allow",
        mew_config::permissions::RuleDecision::Deny => "deny",
        mew_config::permissions::RuleDecision::Ask => "ask",
    }
}

fn new_permission_rule() -> mew_config::permissions::PermissionRule {
    mew_config::permissions::PermissionRule {
        tool: String::new(),
        decision: mew_config::permissions::RuleDecision::Ask,
        r#match: mew_config::permissions::MatchConditions::default(),
    }
}
fn split_comma(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    Normal,
    Editing,
    Naming,
}

#[derive(Debug)]
pub struct SettingsState {
    pub config: mew_config::Config,
    pub plugins: Vec<PluginEntry>,

    left_items: Vec<LeftItem>,
    left_cursor: usize,
    right_cursor: usize,
    active: Panel,
    edit_mode: EditMode,
    buf: String,
    dirty: bool,
    pub message: Option<String>,
    scroll: usize,
}

const BG_LEFT: Color = Color::Rgb(26, 26, 38);
const BG_RIGHT: Color = Color::Rgb(30, 30, 46);
const BG_STATUS: Color = Color::Rgb(17, 17, 27);
const BG_POPUP: Color = Color::Rgb(24, 24, 37);

impl SettingsState {
    pub fn new(config: mew_config::Config, plugins: Vec<PluginEntry>) -> Self {
        let mut state = Self {
            config,
            plugins,
            left_items: Vec::new(),
            left_cursor: 0,
            right_cursor: 0,
            active: Panel::Left,
            edit_mode: EditMode::Normal,
            buf: String::new(),
            dirty: false,
            message: None,
            scroll: 0,
        };
        state.rebuild_left_items();
        state
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Save config and plugin enable state to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = mew_config::config_dir().join("config.toml");
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml = toml::to_string_pretty(&self.config)
            .map_err(|e| anyhow::anyhow!("serialize config: {e}"))?;
        std::fs::write(&config_path, toml)?;
        tracing::info!(path = ?config_path, "config saved");

        let disabled_plugins: Vec<String> = self
            .plugins
            .iter()
            .filter(|p| !p.enabled)
            .map(|p| p.name.clone())
            .collect();
        let mut state = mew_config::load_state().unwrap_or_default();
        state.disabled_plugins = disabled_plugins;
        mew_config::save_state(&state)?;
        tracing::info!("plugin state saved");
        Ok(())
    }

    fn rebuild_left_items(&mut self) {
        let mut items = vec![LeftItem::Category(Category::General)];

        // Accounts: only providers with credentials, sorted, plus add action.
        let authenticated: BTreeMap<&String, _> = self
            .config
            .providers
            .iter()
            .filter(|(id, _)| self.credential_status(id) == "authenticated")
            .collect();
        items.push(LeftItem::Category(Category::Accounts));
        for id in authenticated.keys() {
            items.push(LeftItem::Provider((*id).clone()));
        }
        items.push(LeftItem::AddProvider);

        // Models
        items.push(LeftItem::Category(Category::Models));
        for i in 0..self.config.models.len() {
            items.push(LeftItem::Model(i));
        }
        items.push(LeftItem::AddModel);

        // Permissions
        items.push(LeftItem::Category(Category::Permissions));
        for i in 0..self.config.permissions.rules.len() {
            items.push(LeftItem::PermissionRule(i));
        }
        items.push(LeftItem::AddPermissionRule);

        // Secrets
        items.push(LeftItem::Category(Category::Secrets));
        for i in 0..self.config.secrets.files.len() {
            items.push(LeftItem::SecretFilesGroup(i));
        }
        items.push(LeftItem::AddSecretFilesGroup);
        for i in 0..self.config.secrets.words.len() {
            items.push(LeftItem::SecretWordsGroup(i));
        }
        items.push(LeftItem::AddSecretWordsGroup);

        // Workspace, TUI
        items.push(LeftItem::Category(Category::Workspace));
        items.push(LeftItem::Category(Category::Tui));

        // Plugins
        items.push(LeftItem::Category(Category::Plugins));
        for i in 0..self.plugins.len() {
            items.push(LeftItem::Plugin(i));
        }

        self.left_items = items;
        if self.left_cursor >= self.left_items.len() {
            self.left_cursor = self.left_items.len().saturating_sub(1);
        }
    }

    fn current_item(&self) -> Option<&LeftItem> {
        self.left_items.get(self.left_cursor)
    }

    fn right_fields(&self) -> Vec<Field> {
        let item = match self.current_item() {
            Some(i) => i,
            None => return Vec::new(),
        };
        match item {
            LeftItem::Category(Category::General) => {
                vec![Field::DefaultModel, Field::DefaultPersona, Field::PlanPath]
            }
            LeftItem::Provider(id) => {
                let mut fields = vec![
                    Field::ProviderId(id.clone()),
                    Field::ProviderShape(id.clone()),
                    Field::ProviderBaseUrl(id.clone()),
                    Field::ProviderCredRef(id.clone()),
                    Field::ProviderKind(id.clone()),
                    Field::ProviderDisableHashline(id.clone()),
                ];
                if self.config.providers.get(id).map(|p| p.kind.as_str()) == Some("router") {
                    fields.push(Field::ProviderNano(id.clone()));
                    fields.push(Field::ProviderMicro(id.clone()));
                    fields.push(Field::ProviderDeci(id.clone()));
                }
                fields
            }
            LeftItem::Model(i) => vec![
                Field::ModelId(*i),
                Field::ModelProvider(*i),
                Field::ModelShape(*i),
                Field::ModelContextWindow(*i),
                Field::ModelResponsesLite(*i),
                Field::ModelMerge(*i),
            ],
            LeftItem::Category(Category::Permissions) => {
                vec![Field::ClassifierProvider, Field::ClassifierModel]
            }
            LeftItem::PermissionRule(i) => vec![
                Field::PermissionDecision(*i),
                Field::PermissionTool(*i),
                Field::PermissionCommandPrefix(*i),
                Field::PermissionCommandProgram(*i),
                Field::PermissionCommandSubcommand(*i),
                Field::PermissionPathGlob(*i),
            ],
            LeftItem::SecretFilesGroup(i) => vec![Field::SecretFilesPaths(*i)],
            LeftItem::SecretWordsGroup(i) => vec![Field::SecretWordsValues(*i)],
            LeftItem::Category(Category::Workspace) => vec![Field::WorkspaceRoots],
            LeftItem::Category(Category::Tui) => vec![Field::TuiTheme],
            _ => Vec::new(),
        }
    }

    fn selected_field(&self) -> Option<Field> {
        self.right_fields().into_iter().nth(self.right_cursor)
    }

    fn credential_status(&self, id: &str) -> &'static str {
        let Some(pc) = self.config.providers.get(id) else {
            return "not configured";
        };
        if pc.credential_ref.is_empty() {
            return "no credential ref";
        }
        match mew_config::get_credential(&pc.credential_ref) {
            Ok(_) => "authenticated",
            Err(_) => "not authenticated",
        }
    }

    // ------------------------------------------------------------------
    // Key handling
    // ------------------------------------------------------------------

    /// Handle a key in settings mode. Returns true while settings stays open.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match self.edit_mode {
            EditMode::Normal => self.handle_normal_key(key),
            EditMode::Editing => {
                self.handle_edit_key(key);
                true
            }
            EditMode::Naming => self.handle_naming_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        self.message = None;
        match key.code {
            crossterm::event::KeyCode::Char('c')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                return false
            }
            crossterm::event::KeyCode::Esc => return false,

            crossterm::event::KeyCode::Tab => {
                if self.active == Panel::Left && !self.right_fields().is_empty() {
                    self.active = Panel::Right;
                    self.right_cursor = 0;
                } else {
                    self.active = Panel::Left;
                }
            }

            crossterm::event::KeyCode::Char('s')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                match self.save() {
                    Ok(()) => {
                        self.dirty = false;
                        self.message = Some("Saved.".into());
                    }
                    Err(e) => self.message = Some(format!("Save failed: {e}")),
                }
            }

            crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                self.move_cursor(-1)
            }
            crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                self.move_cursor(1)
            }

            crossterm::event::KeyCode::Enter => self.handle_enter(),
            crossterm::event::KeyCode::Char('n') if self.active == Panel::Left => {
                self.handle_add_item()
            }
            crossterm::event::KeyCode::Char('d') if self.active == Panel::Left => {
                self.handle_delete_item()
            }
            _ => {}
        }
        true
    }

    fn handle_edit_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            crossterm::event::KeyCode::Esc => {
                self.edit_mode = EditMode::Normal;
                self.buf.clear();
            }
            crossterm::event::KeyCode::Enter => {
                if let Some(field) = self.selected_field() {
                    let val = self.buf.clone();
                    field.apply(self, &val);
                    self.dirty = true;
                    // Rebuild in case id renames changed ordering.
                    self.rebuild_left_items();
                }
                self.edit_mode = EditMode::Normal;
                self.buf.clear();
            }
            crossterm::event::KeyCode::Char(c) => self.buf.push(c),
            crossterm::event::KeyCode::Backspace => {
                self.buf.pop();
            }
            _ => {}
        }
    }

    fn handle_naming_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        match key.code {
            crossterm::event::KeyCode::Esc => {
                self.edit_mode = EditMode::Normal;
                self.buf.clear();
            }
            crossterm::event::KeyCode::Enter => {
                let name = self.buf.trim().to_string();
                if !name.is_empty() && !self.config.providers.contains_key(&name) {
                    self.config
                        .providers
                        .insert(name.clone(), mew_config::ProviderConfig::default());
                    self.dirty = true;
                    self.rebuild_left_items();
                    // Move selection to the newly created provider.
                    if let Some(idx) = self
                        .left_items
                        .iter()
                        .position(|i| matches!(i, LeftItem::Provider(id) if id == &name))
                    {
                        self.left_cursor = idx;
                    }
                }
                self.edit_mode = EditMode::Normal;
                self.buf.clear();
            }
            crossterm::event::KeyCode::Char(c) => self.buf.push(c),
            crossterm::event::KeyCode::Backspace => {
                self.buf.pop();
            }
            _ => {}
        }
        true
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.active == Panel::Left {
            let len = self.left_items.len();
            if len == 0 {
                return;
            }
            let new = (self.left_cursor as isize + delta).rem_euclid(len as isize) as usize;
            self.left_cursor = new;
            self.right_cursor = 0;
            self.adjust_scroll();
        } else {
            let len = self.right_fields().len();
            if len == 0 {
                return;
            }
            let new = (self.right_cursor as isize + delta).rem_euclid(len as isize) as usize;
            self.right_cursor = new;
        }
    }

    fn adjust_scroll(&mut self) {
        // Simple scroll-into-view; exact visible count computed at draw time.
        // We keep a target scroll here and clamp in draw.
    }

    fn handle_enter(&mut self) {
        if self.active == Panel::Left {
            match self.current_item().cloned() {
                Some(LeftItem::AddProvider) => {
                    self.edit_mode = EditMode::Naming;
                    self.buf.clear();
                }
                Some(LeftItem::AddModel) => {
                    self.config.models.push(mew_config::CustomModel::default());
                    self.dirty = true;
                    self.rebuild_left_items();
                }
                Some(LeftItem::AddPermissionRule) => {
                    self.config.permissions.rules.push(new_permission_rule());
                    self.dirty = true;
                    self.rebuild_left_items();
                }
                Some(LeftItem::AddSecretFilesGroup) => {
                    self.config
                        .secrets
                        .files
                        .push(mew_config::SecretFilesRule::default());
                    self.dirty = true;
                    self.rebuild_left_items();
                }
                Some(LeftItem::AddSecretWordsGroup) => {
                    self.config
                        .secrets
                        .words
                        .push(mew_config::SecretWordsRule::default());
                    self.dirty = true;
                    self.rebuild_left_items();
                }
                Some(LeftItem::Plugin(i)) => {
                    if let Some(p) = self.plugins.get_mut(i) {
                        p.enabled = !p.enabled;
                        self.dirty = true;
                    }
                }
                Some(item) if item.has_fields() => {
                    self.active = Panel::Right;
                    self.right_cursor = 0;
                }
                _ => {}
            }
        } else {
            // Start editing the selected field.
            if let Some(field) = self.selected_field() {
                self.buf = field.value(self);
                self.edit_mode = EditMode::Editing;
            }
        }
    }

    fn handle_add_item(&mut self) {
        match self.current_item() {
            Some(LeftItem::Category(Category::Accounts) | LeftItem::AddProvider) => {
                self.edit_mode = EditMode::Naming;
                self.buf.clear();
            }
            Some(LeftItem::Category(Category::Models) | LeftItem::AddModel) => {
                self.config.models.push(mew_config::CustomModel::default());
                self.dirty = true;
                self.rebuild_left_items();
            }
            Some(LeftItem::Category(Category::Permissions) | LeftItem::AddPermissionRule) => {
                self.config.permissions.rules.push(new_permission_rule());
                self.dirty = true;
                self.rebuild_left_items();
            }
            Some(LeftItem::SecretWordsGroup(_) | LeftItem::AddSecretWordsGroup) => {
                self.config
                    .secrets
                    .words
                    .push(mew_config::SecretWordsRule::default());
                self.dirty = true;
                self.rebuild_left_items();
            }
            Some(
                LeftItem::Category(Category::Secrets)
                | LeftItem::SecretFilesGroup(_)
                | LeftItem::AddSecretFilesGroup,
            ) => {
                self.config
                    .secrets
                    .files
                    .push(mew_config::SecretFilesRule::default());
                self.dirty = true;
                self.rebuild_left_items();
            }
            _ => {}
        }
    }

    fn handle_delete_item(&mut self) {
        match self.current_item().cloned() {
            Some(LeftItem::Provider(id)) => {
                self.config.providers.remove(&id);
                self.dirty = true;
                self.rebuild_left_items();
            }
            Some(LeftItem::Model(i)) => {
                if i < self.config.models.len() {
                    self.config.models.remove(i);
                    self.dirty = true;
                    self.rebuild_left_items();
                }
            }
            Some(LeftItem::PermissionRule(i)) => {
                if i < self.config.permissions.rules.len() {
                    self.config.permissions.rules.remove(i);
                    self.dirty = true;
                    self.rebuild_left_items();
                }
            }
            Some(LeftItem::SecretFilesGroup(i)) => {
                if i < self.config.secrets.files.len() {
                    self.config.secrets.files.remove(i);
                    self.dirty = true;
                    self.rebuild_left_items();
                }
            }
            Some(LeftItem::SecretWordsGroup(i)) => {
                if i < self.config.secrets.words.len() {
                    self.config.secrets.words.remove(i);
                    self.dirty = true;
                    self.rebuild_left_items();
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Drawing
    // ------------------------------------------------------------------

    pub fn draw(&self, f: &mut Frame, area: Rect, tokens: &ThemeTokens) {
        f.render_widget(Clear, area);
        f.render_widget(Block::default().style(Style::default().bg(BG_RIGHT)), area);

        let title = if self.dirty {
            " mew settings * "
        } else {
            " mew settings "
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
        self.draw_left_panel(f, panes[0], tokens);
        self.draw_right_panel(f, panes[1], tokens);
        self.draw_status_bar(f, area);
        self.draw_message_popup(f, area);

        if self.edit_mode == EditMode::Naming {
            self.draw_naming_popup(f, area);
        }
    }

    fn draw_left_panel(&self, f: &mut Frame, area: Rect, _tokens: &ThemeTokens) {
        let active = self.active == Panel::Left && self.edit_mode == EditMode::Normal;
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

        let visible = inner.height as usize;
        let start = self.scroll.min(self.left_items.len().saturating_sub(1));
        let start = start.min(self.left_items.len().saturating_sub(visible));

        let mut lines: Vec<Line> = Vec::new();
        for (i, item) in self.left_items.iter().enumerate().skip(start).take(visible) {
            let selected = i == self.left_cursor && active;
            let marker = if selected { "▸ " } else { "  " };

            let line = match item {
                LeftItem::Category(cat) => {
                    let style = Style::default()
                        .fg(if selected {
                            Color::Yellow
                        } else {
                            Color::White
                        })
                        .bg(BG_LEFT)
                        .add_modifier(Modifier::BOLD);
                    Line::from(vec![Span::raw(marker), Span::styled(cat.label(), style)])
                }
                LeftItem::Provider(id) => {
                    let status = self.credential_status(id);
                    let (fg, status_text) = match status {
                        "authenticated" => (Color::Green, "✓"),
                        _ => (Color::DarkGray, "✗"),
                    };
                    let name_style = if selected {
                        Style::default()
                            .fg(Color::Yellow)
                            .bg(BG_LEFT)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(fg).bg(BG_LEFT)
                    };
                    let dim = Style::default().fg(Color::DarkGray).bg(BG_LEFT);
                    Line::from(vec![
                        Span::raw(marker),
                        Span::styled(format!("  {status_text} "), dim),
                        Span::styled(format!("{} ", id), name_style),
                        Span::styled(
                            self.config
                                .providers
                                .get(id)
                                .map(|p| p.shape.as_str())
                                .unwrap_or("?"),
                            dim,
                        ),
                    ])
                }
                LeftItem::Plugin(i) => {
                    let plugin = self.plugins.get(*i);
                    let enabled = plugin.map(|p| p.enabled).unwrap_or(false);
                    let icon = if enabled { "✓" } else { "✗" };
                    let icon_color = if enabled { Color::Green } else { Color::Red };
                    let name_style = if selected {
                        Style::default().fg(Color::Yellow).bg(BG_LEFT)
                    } else {
                        Style::default().fg(Color::Gray).bg(BG_LEFT)
                    };
                    Line::from(vec![
                        Span::raw(marker),
                        Span::styled(
                            format!("  {icon} "),
                            Style::default().fg(icon_color).bg(BG_LEFT),
                        ),
                        Span::styled(
                            plugin.map(|p| p.name.clone()).unwrap_or_default(),
                            name_style,
                        ),
                    ])
                }
                _ => {
                    // Add actions and sub-items.
                    let style = if selected {
                        Style::default().fg(Color::Green).bg(BG_LEFT)
                    } else {
                        Style::default().fg(Color::DarkGray).bg(BG_LEFT)
                    };
                    Line::from(vec![
                        Span::raw(marker),
                        Span::styled(format!("  {} ", item.label(self)), style),
                    ])
                }
            };
            lines.push(line);
        }

        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(BG_LEFT)),
            inner,
        );

        if self.left_items.len() > visible {
            let mut state = ScrollbarState::new(self.left_items.len())
                .viewport_content_length(visible)
                .position(self.scroll);
            let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("█");
            f.render_stateful_widget(
                sb,
                area.inner(Margin {
                    horizontal: 0,
                    vertical: 0,
                }),
                &mut state,
            );
        }
    }

    fn draw_right_panel(&self, f: &mut Frame, area: Rect, _tokens: &ThemeTokens) {
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

        let item = self.current_item();
        let fields = self.right_fields();
        let mut lines: Vec<Line> = Vec::new();

        match item {
            Some(LeftItem::Provider(id)) => {
                lines.push(Line::from(Span::styled(
                    format!("Account: {id}"),
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                let status = self.credential_status(id);
                let status_fg = if status == "authenticated" {
                    Color::Green
                } else {
                    Color::Red
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        "  status: ",
                        Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
                    ),
                    Span::styled(status, Style::default().fg(status_fg).bg(BG_RIGHT)),
                ]));
                lines.push(Line::from(""));
            }
            Some(LeftItem::Category(cat)) => {
                lines.push(Line::from(Span::styled(
                    cat.label(),
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::Model(_)) => {
                lines.push(Line::from(Span::styled(
                    "Custom Model",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::PermissionRule(_)) => {
                lines.push(Line::from(Span::styled(
                    "Permission Rule",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::SecretFilesGroup(_)) => {
                lines.push(Line::from(Span::styled(
                    "Secret Files Group",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::SecretWordsGroup(_)) => {
                lines.push(Line::from(Span::styled(
                    "Secret Words Group",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
            }
            Some(LeftItem::Plugin(_)) => {
                lines.push(Line::from(Span::styled(
                    "Plugin",
                    Style::default()
                        .fg(Color::White)
                        .bg(BG_RIGHT)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(vec![Span::styled(
                    "  Enter on the left toggles enabled/disabled.",
                    Style::default().fg(Color::DarkGray).bg(BG_RIGHT),
                )]));
                lines.push(Line::from(""));
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

        let active = self.active == Panel::Right && self.edit_mode == EditMode::Normal;
        for (i, field) in fields.iter().enumerate() {
            let selected = i == self.right_cursor && active;
            let editing = selected && self.edit_mode == EditMode::Editing;

            let val = if editing {
                format!("{}_│", self.buf)
            } else {
                let v = field.value(self);
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
                Span::styled(format!("  {:<18} ", field.label()), label_style),
                Span::styled(val, val_style),
            ]));
        }

        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(BG_RIGHT)),
            inner,
        );
    }

    fn draw_status_bar(&self, f: &mut Frame, area: Rect) {
        let bar_area = Rect::new(
            area.x + 1,
            area.y + area.height - 2,
            area.width.saturating_sub(2),
            1,
        );
        let help = match self.edit_mode {
            EditMode::Normal => match self.active {
                Panel::Left => {
                    " Tab→details · Enter open/add · n new · d delete · ^S save · Esc close"
                }
                Panel::Right => " Tab←list · Enter edit · ^S save · Esc close",
            },
            EditMode::Editing => " Enter confirm · Esc cancel",
            EditMode::Naming => " Enter create · Esc cancel",
        };
        let panel = match self.active {
            Panel::Left => " [list]",
            Panel::Right => " [details]",
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{}{}", panel, help),
                Style::default().fg(Color::DarkGray).bg(BG_STATUS),
            ))
            .style(Style::default().bg(BG_STATUS)),
            bar_area,
        );
    }

    fn draw_message_popup(&self, f: &mut Frame, area: Rect) {
        if self.message.is_none() || self.edit_mode == EditMode::Naming {
            return;
        }
        let msg = self.message.as_ref().unwrap();
        let popup = centered_rect((msg.len() as u16).max(20) + 4, 3, area);
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

    fn draw_naming_popup(&self, f: &mut Frame, area: Rect) {
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

/// Helper used by the TUI overlay system to render the settings screen.
pub fn draw_settings(f: &mut Frame, state: &SettingsState, area: Rect, tokens: &ThemeTokens) {
    state.draw(f, area, tokens);
}
