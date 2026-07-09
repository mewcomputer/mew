use regex::Regex;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use thiserror::Error;
use tracing::debug;

#[derive(Error, Debug)]
pub enum SkillError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid skill name: {0}")]
    InvalidName(String),
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// A discovered and loaded skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub path: PathBuf,
    /// When true, render the body through minijinja before returning to
    /// the model. The skill tool does the rendering.
    pub template: bool,
}

/// Frontmatter parsed from a SKILL.md file.
#[derive(Debug, serde::Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(default)]
    #[allow(dead_code)]
    license: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    compatibility: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    metadata: Option<serde_yaml::Value>,
    /// When true, render the body through minijinja. The skill tool
    /// handles the rendering with the agent's template context.
    #[serde(default)]
    template: bool,
}

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("valid name regex"));

/// Discovers and loads skills from the filesystem.
pub struct Loader {
    cwd: PathBuf,
    extra_dirs: Vec<PathBuf>,
}

impl Loader {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            extra_dirs: Vec::new(),
        }
    }

    /// Create a loader with additional search dirs (e.g. from extension packages).
    pub fn with_extra_dirs(cwd: impl Into<PathBuf>, extra_dirs: Vec<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            extra_dirs,
        }
    }

    /// Scans for skills in the standard locations and loads them.
    ///
    /// Search order (earlier wins on duplicate name):
    ///   Project paths (walked cwd → git root):
    ///     1. `<dir>/.mew/skills/<name>/SKILL.md`
    ///     2. `<dir>/.opencode/skills/<name>/SKILL.md`
    ///     3. `<dir>/.claude/skills/<name>/SKILL.md`
    ///     4. `<dir>/.agents/skills/<name>/SKILL.md`
    ///   Global paths:
    ///     5. `~/.config/mew/skills/<name>/SKILL.md`
    ///     6. `~/.config/opencode/skills/<name>/SKILL.md`
    ///     7. `~/.claude/skills/<name>/SKILL.md`
    ///     8. `~/.agents/skills/<name>/SKILL.md`
    pub fn load(&self) -> Result<Vec<Skill>, SkillError> {
        let spec = mew_harness::LoadSpec {
            prefixes: SKILL_PREFIXES,
            file: mew_harness::LoadFileSpec::SubdirFile("SKILL.md"),
        };
        let parse_fn = |path: &std::path::Path| -> Result<_, SkillError> {
            let skill = load_skill_file(path)?;
            let name = skill.name.clone();
            Ok(mew_harness::Loaded { value: skill, name })
        };
        let mut skills = if self.extra_dirs.is_empty() {
            mew_harness::load_markdown_dirs(&self.cwd, &spec, parse_fn)?
        } else {
            mew_harness::load_markdown_dirs_with_extra(
                &self.cwd,
                &spec,
                parse_fn,
                &self.extra_dirs,
            )?
        };

        // Append built-in skills for any name not already provided by the
        // user. User-defined skills override built-ins by name.
        let mut seen: std::collections::HashSet<String> =
            skills.iter().map(|s| s.name.clone()).collect();
        for builtin in builtin_skills() {
            if !seen.contains(&builtin.name) {
                seen.insert(builtin.name.clone());
                skills.push(builtin);
            }
        }

        debug!(count = skills.len(), "loaded skills");
        Ok(skills)
    }
}

const SKILL_PREFIXES: &[&str] = &[
    ".mew/skills",
    ".opencode/skills",
    ".claude/skills",
    ".agents/skills",
];

fn load_skill_file(path: &Path) -> Result<Skill, SkillError> {
    let content = std::fs::read_to_string(path)?;

    // Parse YAML frontmatter between --- delimiters.
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

    let (name, description, body, template) = match frontmatter {
        Some((fm, body)) => {
            validate_name(&fm.name)?;
            (fm.name, fm.description, body, fm.template)
        }
        None => {
            let dir_name = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            validate_name(&dir_name)?;
            (dir_name, String::new(), content, false)
        }
    };

    Ok(Skill {
        name,
        description,
        body,
        path: path.to_path_buf(),
        template,
    })
}

fn validate_name(name: &str) -> Result<(), SkillError> {
    if name.len() > 64 {
        return Err(SkillError::InvalidName(format!(
            "name too long (max 64): {name}"
        )));
    }
    if !NAME_RE.is_match(name) {
        return Err(SkillError::InvalidName(format!(
            "invalid name: {name}. Must match [a-z0-9]+(-[a-z0-9]+)*"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Built-in skills
// ---------------------------------------------------------------------------

/// Built-in skills shipped with mew. User-defined skills (loaded from
/// `.mew/skills/<name>/SKILL.md` etc.) override these by name.
///
/// - **mew-docs** — documentation sitemap so the agent can `web_fetch` the
///   right docs page at mew.computer instead of guessing from memory.
pub fn builtin_skills() -> Vec<Skill> {
    let content = include_str!("../../mew-prompts/resources/skills/mew-docs/SKILL.md");
    match load_skill_from_str(content) {
        Ok(skill) => vec![skill],
        Err(e) => {
            tracing::warn!(error = %e, "failed to load built-in skill mew-docs");
            vec![]
        }
    }
}

/// Parse a SKILL.md from a string (used for built-in skills that are
/// embedded via `include_str!` rather than read from disk).
fn load_skill_from_str(content: &str) -> Result<Skill, SkillError> {
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

    let (name, description, body, template) = match frontmatter {
        Some((fm, body)) => {
            validate_name(&fm.name)?;
            (fm.name, fm.description, body, fm.template)
        }
        None => {
            return Err(SkillError::InvalidName(
                "built-in skill missing frontmatter".into(),
            ));
        }
    };

    Ok(Skill {
        name,
        description,
        body,
        path: PathBuf::from("(built-in)"),
        template,
    })
}

// ---------------------------------------------------------------------------
// Path helpers (mirrors mew-context)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, description: &str, body: &str) {
        let skill_dir = dir.join(".mew").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = format!("---\nname: {name}\ndescription: {description}\n---\n{body}");
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    /// Tests that exercise the global scan need an isolated HOME so they
    /// don't pick up the developer's real ~/.config/mew/skills. Holds a
    /// static mutex to serialize env-var access (env::set_var is not
    /// thread-safe per Rust's docs). Holds the temp dir alive for the test's
    /// lifetime; when the guard is dropped, HOME is restored.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn with_test_home() -> impl Drop {
        use std::sync::MutexGuard;
        struct Guard {
            _lock: MutexGuard<'static, ()>,
            _dir: tempfile::TempDir,
            prev: Option<std::ffi::OsString>,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                // SAFETY: serialized via HOME_LOCK.
                unsafe {
                    match &self.prev {
                        Some(v) => std::env::set_var("HOME", v),
                        None => std::env::remove_var("HOME"),
                    }
                }
            }
        }
        let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("HOME");
        // SAFETY: serialized via HOME_LOCK above.
        unsafe {
            std::env::set_var("HOME", dir.path());
        }
        Guard {
            _lock: lock,
            _dir: dir,
            prev,
        }
    }

    /// Filter out built-in skills so tests can assert on user-defined ones.
    fn user_skills(skills: Vec<Skill>) -> Vec<Skill> {
        skills
            .into_iter()
            .filter(|s| s.path != Path::new("(built-in)"))
            .collect()
    }

    #[test]
    fn test_load_single_skill() {
        let _home = with_test_home();
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_skill(
            cwd,
            "git-release",
            "Creates a git release",
            "# Git Release\nDo things.",
        );

        let loader = Loader::new(cwd);
        let skills = user_skills(loader.load().unwrap());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "git-release");
        assert_eq!(skills[0].description, "Creates a git release");
        assert!(skills[0].body.contains("# Git Release"));
    }

    #[test]
    fn test_load_multiple_skills() {
        let _home = with_test_home();
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_skill(cwd, "git-release", "Git release", "body1");
        write_skill(cwd, "code-review", "Review code", "body2");

        let loader = Loader::new(cwd);
        let skills = user_skills(loader.load().unwrap());
        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn test_duplicate_name_project_wins_over_global() {
        let _home = with_test_home();
        // This test verifies that project-level skills take precedence.
        // Since we can't easily mock HOME, we test that the project path
        // works correctly and that seen set prevents duplicates.
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_skill(cwd, "test-skill", "First", "body1");

        // Create a second copy that would be discovered later (simulating
        // a parent dir)
        let parent = cwd.join(".opencode").join("skills").join("test-skill");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::write(
            parent.join("SKILL.md"),
            "---\nname: test-skill\ndescription: Second\n---\nbody2",
        )
        .unwrap();

        let loader = Loader::new(cwd);
        let skills = user_skills(loader.load().unwrap());
        // Only one should survive (first one wins via seen set)
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "First");
    }

    #[test]
    fn test_name_validation_rejects_invalid() {
        assert!(validate_name("valid-name").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("a-b-c").is_ok());
        assert!(validate_name("INVALID").is_err());
        assert!(validate_name("has_underscore").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn test_name_too_long() {
        let long = "a".repeat(65);
        assert!(validate_name(&long).is_err());
    }

    #[test]
    fn test_load_no_skills() {
        let _home = with_test_home();
        let tmp = tempfile::tempdir().unwrap();
        let loader = Loader::new(tmp.path());
        let skills = user_skills(loader.load().unwrap());
        assert!(skills.is_empty());
    }

    #[test]
    fn test_skill_without_frontmatter() {
        // Skill without YAML frontmatter should use directory name
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        let skill_dir = cwd.join(".mew").join("skills").join("no-fm");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Just a body").unwrap();

        let loader = Loader::new(cwd);
        let skills = user_skills(loader.load().unwrap());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "no-fm");
        assert_eq!(skills[0].description, ""); // No frontmatter, no description
        assert_eq!(skills[0].body, "Just a body");
    }

    #[test]
    fn test_name_mismatch_rejected() {
        let _home = with_test_home();
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        let skill_dir = cwd.join(".mew").join("skills").join("a-name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: different-name\ndescription: desc\n---\nbody",
        )
        .unwrap();

        let loader = Loader::new(cwd);
        let skills = user_skills(loader.load().unwrap());
        // Should be rejected because name doesn't match directory
        assert!(skills.is_empty());
    }

    #[test]
    fn test_builtin_skills_loaded() {
        let builtins = builtin_skills();
        assert!(
            builtins.iter().any(|s| s.name == "mew-docs"),
            "mew-docs should be a built-in skill"
        );
    }

    #[test]
    fn test_builtin_mew_docs_has_sitemap() {
        let builtins = builtin_skills();
        let docs = builtins.iter().find(|s| s.name == "mew-docs").unwrap();
        assert!(docs.body.contains("https://mew.computer/docs/"));
        assert!(docs.body.contains("web_fetch"));
    }

    #[test]
    fn test_builtin_skills_included_in_load() {
        let _home = with_test_home();
        let tmp = tempfile::tempdir().unwrap();
        let loader = Loader::new(tmp.path());
        let skills = loader.load().unwrap();
        // Built-in skills should appear even in an empty directory.
        assert!(
            skills.iter().any(|s| s.name == "mew-docs"),
            "mew-docs should be included as a built-in skill"
        );
    }
}
