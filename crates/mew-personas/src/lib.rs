use regex::Regex;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use thiserror::Error;
use tracing::{debug, trace};

#[derive(Error, Debug)]
pub enum PersonaError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid persona name: {0}")]
    InvalidName(String),
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// A discovered and loaded persona.
#[derive(Debug, Clone)]
pub struct Persona {
    pub name: String,
    pub description: String,
    /// Verbatim markdown body — becomes part of the system prompt.
    pub body: String,
    pub path: PathBuf,
    /// Persona-specific config from the `mew:` frontmatter key.
    pub config: PersonaConfig,
}

#[derive(Debug, Clone, Default)]
pub struct PersonaConfig {
    /// Pin a specific `provider/model`. None = inherit active model.
    pub model: Option<String>,
    /// If set, only these tools are available while this persona is active.
    /// Empty vec = no tools. None = all tools (subject to `tools_deny`).
    pub tools: Option<Vec<String>>,
    /// Tools to remove from the available set, regardless of `tools`. Applied
    /// after the allowlist, so an entry here is always excluded. Use this when
    /// you want "all tools except X" without enumerating the safe set.
    /// `None` and empty vec both mean "no denials".
    pub tools_deny: Option<Vec<String>>,
    /// Skill names this persona can load via the `skill` tool, and the names
    /// that appear in the system prompt's `<available_skills>` list.
    /// `None` = all discovered skills; `Some(vec)` = only those listed.
    /// An empty vec hides all skills.
    pub skills: Option<Vec<String>>,
    /// When `true`, render the persona body as a minijinja template before
    /// using it as the system prompt. Exposes `supports_vision`, `tools`,
    /// `has_tool(name)`, and `persona_name`. `None` or `false` = verbatim
    /// body (the default, and the safe choice for personas that don't need
    /// dynamic content).
    pub template: Option<bool>,
}

/// Frontmatter parsed from a PERSONA.md file.
#[derive(Debug, serde::Deserialize)]
struct Frontmatter {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    mew: Option<MewFrontmatter>,
}

#[derive(Debug, serde::Deserialize)]
struct MewFrontmatter {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    tools_deny: Option<Vec<String>>,
    #[serde(default)]
    skills: Option<Vec<String>>,
    #[serde(default)]
    template: Option<bool>,
}

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("valid name regex"));

/// Discovers and loads personas from the filesystem.
pub struct Loader {
    cwd: PathBuf,
    /// If true, skip appending built-in personas (planner, builder) in
    /// `load()`. Defaults to `false` (built-ins are included). Tests that
    /// want to verify the scan logic without built-in noise use
    /// `Loader::new(dir).without_builtins()`.
    skip_builtins: bool,
}

impl Loader {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            skip_builtins: false,
        }
    }

    /// Skip the built-in planner/builder personas in `load()`. Useful for
    /// testing the scan logic without the built-in noise, or for callers
    /// that only want user-defined personas.
    pub fn without_builtins(mut self) -> Self {
        self.skip_builtins = true;
        self
    }

    /// Scans for personas in the standard locations and loads them.
    ///
    /// Search order (earlier wins on duplicate name):
    ///   Project paths (walked cwd → git root):
    ///     1. `<dir>/.mew/personas/<name>/PERSONA.md`
    ///     2. `<dir>/.opencode/personas/<name>/PERSONA.md`
    ///     3. `<dir>/.claude/personas/<name>/PERSONA.md`
    ///     4. `<dir>/.agents/personas/<name>/PERSONA.md`
    ///   Global paths:
    ///     5. `~/.config/mew/personas/<name>/PERSONA.md`
    ///     6. `~/.config/opencode/personas/<name>/PERSONA.md`
    ///     7. `~/.claude/personas/<name>/PERSONA.md`
    ///     8. `~/.agents/personas/<name>/PERSONA.md`
    pub fn load(&self) -> Result<Vec<Persona>, PersonaError> {
        let mut personas = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        let root = find_git_root(&self.cwd).unwrap_or_else(|_| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.cwd.clone())
        });

        let project_dirs = paths_between(&root, &self.cwd);
        for dir in project_dirs.iter().rev() {
            self.scan_dir(dir, &mut personas, &mut seen)?;
        }

        if let Some(home) = dirs_home() {
            for dir in global_persona_dirs(&home) {
                self.scan_dir(&dir, &mut personas, &mut seen)?;
            }
        }

        // Append built-in defaults (planner, builder) for any name not
        // already provided by the user. User-defined personas override
        // built-ins by name — the scan above populated `seen`, so any
        // built-in whose name is already taken is skipped.
        if !self.skip_builtins {
            for builtin in builtin_defaults() {
                if !seen.contains(&builtin.name) {
                    seen.insert(builtin.name.clone());
                    personas.push(builtin);
                }
            }
        }

        debug!(count = personas.len(), "loaded personas");
        Ok(personas)
    }

    fn scan_dir(
        &self,
        dir: &Path,
        personas: &mut Vec<Persona>,
        seen: &mut HashSet<String>,
    ) -> Result<(), PersonaError> {
        let prefixes = [
            ".mew/personas",
            ".opencode/personas",
            ".claude/personas",
            ".agents/personas",
        ];

        for prefix in &prefixes {
            let personas_dir = dir.join(prefix);
            if !personas_dir.is_dir() {
                continue;
            }

            let entries = match std::fs::read_dir(&personas_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let persona_dir = entry.path();
                let persona_md = persona_dir.join("PERSONA.md");
                if !persona_md.is_file() {
                    continue;
                }

                match load_persona_file(&persona_md) {
                    Ok(persona) => {
                        let dir_name = persona_dir
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if persona.name != dir_name {
                            debug!(
                                name = %persona.name,
                                dir = %dir_name,
                                "persona name does not match directory name, skipping"
                            );
                            continue;
                        }
                        if seen.contains(&persona.name) {
                            trace!(name = %persona.name, "duplicate persona, skipping later copy");
                            continue;
                        }
                        seen.insert(persona.name.clone());
                        personas.push(persona);
                    }
                    Err(e) => {
                        debug!(path = %persona_md.display(), error = %e, "failed to load persona");
                    }
                }
            }
        }
        Ok(())
    }
}

fn load_persona_file(path: &Path) -> Result<Persona, PersonaError> {
    let content = std::fs::read_to_string(path)?;

    let frontmatter = if let Some(body) = content.strip_prefix("---\n") {
        if let Some((yaml, body)) = body.split_once("\n---") {
            let fm: Frontmatter = serde_yaml::from_str(yaml)?;
            let body = body.trim_start_matches('\n').to_string();
            Some((fm, body))
        } else {
            None
        }
    } else {
        None
    };

    let (name, description, body, config) = match frontmatter {
        Some((fm, body)) => {
            validate_name(&fm.name)?;
            let config = match fm.mew {
                Some(mew) => PersonaConfig {
                    model: mew.model,
                    tools: mew.tools,
                    tools_deny: mew.tools_deny,
                    skills: mew.skills,
                    template: mew.template,
                },
                None => PersonaConfig::default(),
            };
            (fm.name, fm.description, body, config)
        }
        None => {
            let dir_name = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            validate_name(&dir_name)?;
            (dir_name, String::new(), content, PersonaConfig::default())
        }
    };

    Ok(Persona {
        name,
        description,
        body,
        path: path.to_path_buf(),
        config,
    })
}

fn validate_name(name: &str) -> Result<(), PersonaError> {
    if name.len() > 64 {
        return Err(PersonaError::InvalidName(format!(
            "name too long (max 64): {name}"
        )));
    }
    if !NAME_RE.is_match(name) {
        return Err(PersonaError::InvalidName(format!(
            "invalid name: {name}. Must match [a-z0-9]+(-[a-z0-9]+)*"
        )));
    }
    Ok(())
}

fn find_git_root(dir: &Path) -> Result<PathBuf, PersonaError> {
    let mut current = dir.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => {
                return Err(PersonaError::Io(io::Error::new(
                    io::ErrorKind::NotFound,
                    "git root not found",
                )))
            }
        }
    }
}

fn paths_between(root: &Path, leaf: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !leaf.starts_with(root) {
        out.push(leaf.to_path_buf());
        return out;
    }
    let mut current = root.to_path_buf();
    out.push(current.clone());
    let root_str = root.to_string_lossy();
    let leaf_str = leaf.to_string_lossy();
    let suffix = leaf_str.strip_prefix(&*root_str).unwrap_or("");
    let suffix = suffix.strip_prefix('/').unwrap_or(suffix);
    let suffix = suffix.strip_prefix('\\').unwrap_or(suffix);
    for component in suffix.split(['/', '\\']) {
        if component.is_empty() {
            continue;
        }
        current = current.join(component);
        out.push(current.clone());
    }
    out
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn global_persona_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".config").join("mew").join("personas"),
        home.join(".config").join("opencode").join("personas"),
        home.join(".claude").join("personas"),
        home.join(".agents").join("personas"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_persona(dir: &Path, name: &str, description: &str, body: &str) {
        write_persona_with_mew(dir, name, description, body, "")
    }

    fn write_persona_with_mew(
        dir: &Path,
        name: &str,
        description: &str,
        body: &str,
        mew_yaml: &str,
    ) {
        let persona_dir = dir.join(".mew").join("personas").join(name);
        std::fs::create_dir_all(&persona_dir).unwrap();
        let content = if mew_yaml.is_empty() {
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}")
        } else {
            format!("---\nname: {name}\ndescription: {description}\nmew:\n{mew_yaml}---\n{body}")
        };
        std::fs::write(persona_dir.join("PERSONA.md"), content).unwrap();
    }

    #[test]
    fn test_load_single_persona() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona(
            cwd,
            "researcher",
            "Read-only investigation",
            "You are a researcher.",
        );

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].name, "researcher");
        assert_eq!(personas[0].description, "Read-only investigation");
        assert!(personas[0].body.contains("You are a researcher."));
        assert!(personas[0].config.model.is_none());
        assert!(personas[0].config.tools.is_none());
    }

    #[test]
    fn test_load_persona_with_model_and_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona_with_mew(
            cwd,
            "executor",
            "Write tools",
            "You execute code.",
            "  model: z-ai/glm-4.5-air\n  tools:\n    - read\n    - write\n    - bash\n",
        );

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 1);
        assert_eq!(
            personas[0].config.model.as_deref(),
            Some("z-ai/glm-4.5-air")
        );
        let tools = personas[0].config.tools.as_ref().unwrap();
        assert_eq!(
            tools,
            &vec!["read".to_string(), "write".to_string(), "bash".to_string()]
        );
    }

    #[test]
    fn test_load_multiple_personas() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona(cwd, "researcher", "Research", "body1");
        write_persona(cwd, "executor", "Execute", "body2");

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 2);
    }

    #[test]
    fn test_duplicate_name_first_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona(cwd, "test-persona", "First", "body1");

        let parent = cwd.join(".opencode").join("personas").join("test-persona");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::write(
            parent.join("PERSONA.md"),
            "---\nname: test-persona\ndescription: Second\n---\nbody2",
        )
        .unwrap();

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].description, "First");
    }

    #[test]
    fn test_name_validation_rejects_invalid() {
        assert!(validate_name("valid-name").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("INVALID").is_err());
        assert!(validate_name("has_underscore").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn test_load_no_personas() {
        let tmp = tempfile::tempdir().unwrap();
        let loader = Loader::new(tmp.path()).without_builtins();
        let personas = loader.load().unwrap();
        assert!(personas.is_empty());
    }

    #[test]
    fn test_persona_without_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        let persona_dir = cwd.join(".mew").join("personas").join("no-fm");
        std::fs::create_dir_all(&persona_dir).unwrap();
        std::fs::write(persona_dir.join("PERSONA.md"), "Just a body").unwrap();

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].name, "no-fm");
        assert_eq!(personas[0].description, "");
        assert_eq!(personas[0].body, "Just a body");
    }

    #[test]
    fn test_name_mismatch_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        let persona_dir = cwd.join(".mew").join("personas").join("a-name");
        std::fs::create_dir_all(&persona_dir).unwrap();
        std::fs::write(
            persona_dir.join("PERSONA.md"),
            "---\nname: different-name\ndescription: desc\n---\nbody",
        )
        .unwrap();

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert!(personas.is_empty());
    }

    #[test]
    fn test_persona_tools_empty_vec() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona_with_mew(cwd, "locked", "No tools", "body", "  tools: []\n");

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 1);
        let tools = personas[0].config.tools.as_ref().unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_persona_tools_deny() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona_with_mew(
            cwd,
            "researcher",
            "No mutating tools",
            "body",
            "  tools_deny:\n    - bash\n    - write\n",
        );

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 1);
        let deny = personas[0].config.tools_deny.as_ref().unwrap();
        assert_eq!(deny, &vec!["bash".to_string(), "write".to_string()]);
    }

    #[test]
    fn test_persona_skills_allowlist() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona_with_mew(
            cwd,
            "reviewer",
            "Code review",
            "body",
            "  skills:\n    - git-release\n    - code-review\n",
        );

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        assert_eq!(personas.len(), 1);
        let skills = personas[0].config.skills.as_ref().unwrap();
        assert_eq!(
            skills,
            &vec!["git-release".to_string(), "code-review".to_string()]
        );
    }

    #[test]
    fn test_persona_skills_empty_vec_hides_all() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_persona_with_mew(cwd, "minimal", "No skills", "body", "  skills: []\n");

        let loader = Loader::new(cwd).without_builtins();
        let personas = loader.load().unwrap();
        let skills = personas[0].config.skills.as_ref().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_builtin_defaults_returns_planner_and_builder() {
        let builtins = builtin_defaults();
        let names: Vec<&str> = builtins.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"planner"));
        assert!(names.contains(&"builder"));
    }

    #[test]
    fn test_builtin_planner_has_readonly_tools() {
        let builtins = builtin_defaults();
        let planner = builtins.iter().find(|p| p.name == "planner").unwrap();
        let tools = planner.config.tools.as_ref().expect("planner has tool allowlist");
        // Planner can investigate and write plans, but can't run shell commands.
        assert!(tools.contains(&"read".to_string()));
        assert!(tools.contains(&"grep".to_string()));
        assert!(tools.contains(&"write".to_string()));
        assert!(!tools.contains(&"bash".to_string()));
    }

    #[test]
    fn test_builtin_builder_has_all_tools() {
        let builtins = builtin_defaults();
        let builder = builtins.iter().find(|p| p.name == "builder").unwrap();
        // Builder has no tool restriction (None = all tools).
        assert!(builder.config.tools.is_none());
    }

    #[test]
    fn test_load_includes_builtins_when_not_overridden() {
        let loader = Loader::new("/nonexistent");
        let personas = loader.load().unwrap();
        let names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"planner"), "planner should be a built-in default");
        assert!(names.contains(&"builder"), "builder should be a built-in default");
    }
}

/// Built-in personas shipped with mew. User-defined personas (loaded from
/// `.mew/personas/<name>/PERSONA.md` etc.) override these by name.
///
/// - **planner** — read-only investigation + plan writing. Writes a plan to
///   `PLAN.md` (or the configured `plan_path`), flag_important's it, then
///   hands off to the builder. No bash or dangerous tools.
/// - **builder** — the default persona. All tools available. Reads the plan
///   from `PLAN.md` at the start of work and executes it step by step.
pub fn builtin_defaults() -> Vec<Persona> {
    vec![
        Persona {
            name: "builder".into(),
            description: "Executes plans step by step. The default persona — all tools available.".into(),
            body: BUILDER_BODY.into(),
            path: PathBuf::from("(built-in)"),
            config: PersonaConfig::default(),
        },
        Persona {
            name: "planner".into(),
            description: "Investigates the codebase and writes a plan. Read-only tools plus plan writing.".into(),
            body: PLANNER_BODY.into(),
            path: PathBuf::from("(built-in)"),
            config: PersonaConfig {
                tools: Some(vec![
                    "read".into(),
                    "glob".into(),
                    "grep".into(),
                    "write".into(),
                    "edit".into(),
                    "ask_user_question".into(),
                    "flag_important".into(),
                    "todo_create".into(),
                    "todo_update".into(),
                    "todo_complete".into(),
                    "todo_list".into(),
                ]),
                ..Default::default()
            },
        },
    ]
}

const BUILDER_BODY: &str = "\
You are a builder. Your job is to execute plans step by step, making real \
changes to the codebase.

## Workflow

1. If a plan exists (check PLAN.md or the plan path configured in your \
environment), read it first. It contains the steps you should follow.
2. Work through the plan one step at a time. Use `todo_list` to track \
progress if the plan has explicit steps.
3. Make focused, minimal changes. Read the relevant code before editing.
4. Test your changes when possible.
5. Update the plan or todos as you complete each step.

## Principles

- Prefer the smallest change that solves the problem.
- Read before you write. Understand existing patterns before adding new ones.
- If you're stuck or unsure, use `ask_user_question` rather than guessing.
- Save progress to CURRENT.md frequently (append-only, dated sections).

You have access to all tools: file reads/writes/edits, shell commands, \
search, subagents, and more. Use them responsibly.
";

const PLANNER_BODY: &str = "\
You are a planner. Your job is to investigate the codebase, understand the \
problem, and write a clear, actionable plan. You do NOT make changes — \
you produce the plan that a builder will execute.

## Workflow

1. Read the relevant code, configs, and documentation. Use `glob`, `grep`, \
and `read` liberally.
2. Ask clarifying questions with `ask_user_question` when the requirements \
are ambiguous.
3. Write the plan to PLAN.md (or the configured plan path). The plan should \
have:
   - A clear goal statement
   - Numbered steps, each with a concrete description
   - Files that will be touched
   - Risks or tradeoffs called out
4. Call `flag_important` on the plan file so it survives context compaction.
5. Use `todo_create` to create session todos from the plan steps.
6. Hand off to the builder persona when the plan is ready.

## Principles

- Investigate before planning. A plan built on assumptions is worse than \
asking one question.
- Be concrete. \"Update the config parser\" is not a step; \"add a `ports` \
field to the ServerConfig struct in config.rs and parse it in load_config\" \
is.
- Flag risks. If a step could break something, say so.
- Keep the plan skimmable. The builder will read it start-to-finish.

You do NOT have bash or other dangerous tools. You can read, search, write \
the plan file, and create todos. That's intentional — planning is a \
read-only phase.
";
