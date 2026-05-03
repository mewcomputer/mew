use regex::Regex;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use thiserror::Error;
use tracing::{debug, trace};

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
}

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("valid name regex"));

/// Discovers and loads skills from the filesystem.
pub struct Loader {
    cwd: PathBuf,
}

impl Loader {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
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
        let mut skills = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let root = find_git_root(&self.cwd).unwrap_or_else(|_| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.cwd.clone())
        });

        // Project paths: walk from root → cwd, but scan in reverse (cwd first, so earlier wins)
        let project_dirs = paths_between(&root, &self.cwd);
        // Reverse: cwd first so project-local beats project-root
        for dir in project_dirs.iter().rev() {
            self.scan_dir(dir, &mut skills, &mut seen)?;
        }

        // Global paths
        if let Some(home) = dirs_home() {
            for dir in global_skill_dirs(&home) {
                self.scan_dir(&dir, &mut skills, &mut seen)?;
            }
        }

        debug!(count = skills.len(), "loaded skills");
        Ok(skills)
    }

    fn scan_dir(
        &self,
        dir: &Path,
        skills: &mut Vec<Skill>,
        seen: &mut std::collections::HashSet<String>,
    ) -> Result<(), SkillError> {
        // Sub-directories to scan for SKILL.md files
        let prefixes = [
            ".mew/skills",
            ".opencode/skills",
            ".claude/skills",
            ".agents/skills",
        ];

        for prefix in &prefixes {
            let skills_dir = dir.join(prefix);
            if !skills_dir.is_dir() {
                continue;
            }

            let entries = match std::fs::read_dir(&skills_dir) {
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
                let skill_dir = entry.path();
                let skill_md = skill_dir.join("SKILL.md");
                if !skill_md.is_file() {
                    continue;
                }

                match load_skill_file(&skill_md) {
                    Ok(skill) => {
                        let dir_name = skill_dir
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if skill.name != dir_name {
                            debug!(
                                name = %skill.name,
                                dir = %dir_name,
                                "skill name does not match directory name, skipping"
                            );
                            continue;
                        }
                        if seen.contains(&skill.name) {
                            trace!(name = %skill.name, "duplicate skill, skipping later copy");
                            continue;
                        }
                        seen.insert(skill.name.clone());
                        skills.push(skill);
                    }
                    Err(e) => {
                        debug!(path = %skill_md.display(), error = %e, "failed to load skill");
                    }
                }
            }
        }
        Ok(())
    }
}

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

    let (name, description, body) = match frontmatter {
        Some((fm, body)) => {
            validate_name(&fm.name)?;
            (fm.name, fm.description, body)
        }
        None => {
            let dir_name = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            validate_name(&dir_name)?;
            (dir_name, String::new(), content)
        }
    };

    Ok(Skill {
        name,
        description,
        body,
        path: path.to_path_buf(),
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
// Path helpers (mirrors mew-context)
// ---------------------------------------------------------------------------

fn find_git_root(dir: &Path) -> Result<PathBuf, SkillError> {
    let mut current = dir.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => {
                return Err(SkillError::Io(io::Error::new(
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

fn global_skill_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".config").join("mew").join("skills"),
        home.join(".config").join("opencode").join("skills"),
        home.join(".claude").join("skills"),
        home.join(".agents").join("skills"),
    ]
}

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

    #[test]
    fn test_load_single_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_skill(
            cwd,
            "git-release",
            "Creates a git release",
            "# Git Release\nDo things.",
        );

        let loader = Loader::new(cwd);
        let skills = loader.load().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "git-release");
        assert_eq!(skills[0].description, "Creates a git release");
        assert!(skills[0].body.contains("# Git Release"));
    }

    #[test]
    fn test_load_multiple_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_skill(cwd, "git-release", "Git release", "body1");
        write_skill(cwd, "code-review", "Review code", "body2");

        let loader = Loader::new(cwd);
        let skills = loader.load().unwrap();
        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn test_duplicate_name_project_wins_over_global() {
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
        let skills = loader.load().unwrap();
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
        let tmp = tempfile::tempdir().unwrap();
        let loader = Loader::new(tmp.path());
        let skills = loader.load().unwrap();
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
        let skills = loader.load().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "no-fm");
        assert_eq!(skills[0].description, ""); // No frontmatter, no description
        assert_eq!(skills[0].body, "Just a body");
    }

    #[test]
    fn test_name_mismatch_rejected() {
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
        let skills = loader.load().unwrap();
        // Should be rejected because name doesn't match directory
        assert!(skills.is_empty());
    }
}
