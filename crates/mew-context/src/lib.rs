use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, trace};

#[derive(Error, Debug)]
pub enum ContextError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// A loaded context file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub content: String,
    /// When true, the content should be rendered through minijinja before
    /// being used as a system prompt. Set by `mew: true` or `polytoken: true`
    /// in the context file's YAML frontmatter.
    pub template: bool,
}

/// Discovers and loads project context files.
pub struct Loader {
    cwd: PathBuf,
}

impl Loader {
    /// Creates a loader rooted at the given directory.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Walks from `cwd` up to the git worktree root (or `$HOME`), collecting
    /// context files along the way.
    ///
    /// Precedence (per directory): `AGENTS.md` preferred, `CLAUDE.md` as fallback.
    /// Only one file is loaded per directory level.
    ///
    /// Global: `~/.config/mew/AGENTS.md` first, then `~/.claude/CLAUDE.md` as fallback.
    ///
    /// Files are returned from most-general to most-specific.
    pub fn load(&self) -> Result<Vec<File>, ContextError> {
        let mut files = Vec::new();

        // Global config: AGENTS.md preferred, CLAUDE.md fallback.
        if let Some(cfg_dir) = config_dir() {
            let p = cfg_dir.join("AGENTS.md");
            if let Some(f) = try_read(&p) {
                trace!(?p, "loaded global AGENTS.md");
                files.push(f);
            } else if let Some(home) = home_dir() {
                let cc = home.join(".claude").join("CLAUDE.md");
                if let Some(f) = try_read(&cc) {
                    trace!(?cc, "loaded global CLAUDE.md (fallback)");
                    files.push(f);
                }
            }
        }

        // Determine root: git worktree root or home.
        let root = find_git_root(&self.cwd)
            .unwrap_or_else(|_| home_dir().unwrap_or_else(|| self.cwd.clone()));

        // Collect paths from root down to cwd so most-general comes first.
        let paths = paths_between(&root, &self.cwd);
        for dir in &paths {
            let agents = dir.join("AGENTS.md");
            let claude = dir.join("CLAUDE.md");
            let dot_mew = dir.join(".mew").join("AGENTS.md");

            // Per-level: AGENTS.md preferred, CLAUDE.md fallback.
            if let Some(f) = try_read(&agents) {
                trace!(?agents, "loaded project AGENTS.md");
                files.push(f);
            } else if let Some(f) = try_read(&claude) {
                trace!(?claude, "loaded project CLAUDE.md (fallback)");
                files.push(f);
            }

            // .mew/AGENTS.md is always loaded separately (additive, not a fallback).
            if let Some(f) = try_read(&dot_mew) {
                trace!(?dot_mew, "loaded .mew/AGENTS.md");
                files.push(f);
            }
        }

        debug!(count = files.len(), "loaded context files");
        Ok(files)
    }
}

fn try_read(path: &Path) -> Option<File> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (template, content) = parse_context_frontmatter(&raw);
    Some(File {
        path: path.to_path_buf(),
        content: expand_includes(&content, path),
        template,
    })
}

/// Parse optional YAML frontmatter from a context file. If the file starts
/// with `---` and contains `mew: true` or `polytoken: true`, the frontmatter
/// is stripped and `template: true` is returned. Otherwise the content is
/// returned unchanged with `template: false`.
fn parse_context_frontmatter(raw: &str) -> (bool, String) {
    let body = match raw.strip_prefix("---\n") {
        Some(b) => b,
        None => return (false, raw.to_string()),
    };
    let (yaml, body) = match body.split_once("\n---") {
        Some(split) => split,
        None => return (false, raw.to_string()),
    };

    // Simple check: look for `mew: true` or `polytoken: true` in the YAML.
    // We don't parse the full YAML here to avoid adding serde_yaml as a dep
    // just for this. The check is deliberately conservative: it only
    // matches the exact key-value pair on its own line.
    let template = yaml
        .lines()
        .any(|line| line.trim() == "mew: true" || line.trim() == "polytoken: true");

    let content = body.trim_start_matches('\n').to_string();
    (template, content)
}

/// Expand `@path/to/file` static includes in context file content.
///
/// A line starting with `@` followed by a path is treated as a static
/// include: the referenced file is read and inlined as literal text
/// (no template rendering). The path is resolved relative to the
/// directory of the file containing the `@` reference.
///
/// `..` path components are rejected to confine includes to the file's
/// directory subtree. Lines that don't start with `@` or that reference
/// a non-existent file are left unchanged.
///
/// Example: in `/project/AGENTS.md`, the line `@docs/conventions.md`
/// inlines the contents of `/project/docs/conventions.md`.
fn expand_includes(content: &str, source: &Path) -> String {
    if !content.lines().any(|l| l.starts_with('@')) {
        return content.to_string();
    }

    let base_dir = match source.parent() {
        Some(d) => d,
        None => return content.to_string(),
    };

    let mut out = String::new();
    for line in content.lines() {
        if let Some(include_path) = line.strip_prefix('@') {
            let include_path = include_path.trim();
            if include_path.is_empty() {
                out.push_str(line);
                out.push('\n');
                continue;
            }

            // Reject ../ to confine to the file's subtree.
            let resolved = base_dir.join(include_path);
            if !resolved.starts_with(base_dir) {
                tracing::warn!(
                    include = include_path,
                    source = %source.display(),
                    "@include rejected: path escapes file directory"
                );
                out.push_str(line);
                out.push('\n');
                continue;
            }

            match std::fs::read_to_string(&resolved) {
                Ok(included) => {
                    out.push_str(&included);
                    if !included.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        include = include_path,
                        source = %source.display(),
                        error = %e,
                        "@include file not found"
                    );
                    out.push_str(line);
                    out.push('\n');
                }
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }

    // Remove trailing newline if the original didn't have one.
    if !content.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }

    out
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("computer", "mew", "mew").map(|d| d.config_dir().to_path_buf())
}

fn find_git_root(dir: &Path) -> Result<PathBuf, ContextError> {
    let mut current = dir.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => {
                return Err(ContextError::Io(io::Error::new(
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

/// Concatenates loaded files into a system prompt fragment.
/// Note: files with `template: true` should be rendered by the caller
/// before passing to this function. This function inlines content as-is.
pub fn build_system_prompt(files: &[File]) -> String {
    let mut buf = String::new();
    for f in files {
        buf.push_str(&format!(
            "<context source=\"{}\">\n{}\n</context>\n",
            escape_xml(&f.path.to_string_lossy()),
            f.content
        ));
    }
    buf
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Load project-local template variables from `.mew/project_vars.yaml`.
///
/// The file is a flat YAML map of string keys to string values. These are
/// accessible as `project_vars` in persona/skill/subagent templates.
///
/// Search order (first match wins, walked cwd to git root):
///   1. `<dir>/.mew/project_vars.yaml`
///   2. `<dir>/.opencode/project_vars.yaml`
///   3. `<dir>/.claude/project_vars.yaml`
///   4. `<dir>/.agents/project_vars.yaml`
///
/// Returns an empty map if no file is found. Missing keys in the YAML
/// render as empty strings in templates (minijinja default behavior).
pub fn load_project_vars(cwd: &Path) -> std::collections::HashMap<String, String> {
    let root =
        find_git_root(cwd).unwrap_or_else(|_| home_dir().unwrap_or_else(|| cwd.to_path_buf()));

    for dir in paths_between(&root, cwd) {
        for prefix in &[".mew", ".opencode", ".claude", ".agents"] {
            let path = dir.join(prefix).join("project_vars.yaml");
            if let Ok(content) = std::fs::read_to_string(&path) {
                match serde_yaml::from_str::<std::collections::HashMap<String, String>>(&content) {
                    Ok(map) => {
                        debug!(?path, vars = map.len(), "loaded project_vars");
                        return map;
                    }
                    Err(e) => {
                        tracing::warn!(?path, error = %e, "failed to parse project_vars.yaml");
                    }
                }
            }
        }
    }

    std::collections::HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_paths_between() {
        let root = PathBuf::from("/home/user/project");
        let leaf = PathBuf::from("/home/user/project/src/deep");
        let paths = paths_between(&root, &leaf);
        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0], PathBuf::from("/home/user/project"));
        assert_eq!(paths[1], PathBuf::from("/home/user/project/src"));
        assert_eq!(paths[2], PathBuf::from("/home/user/project/src/deep"));
    }

    #[test]
    fn test_build_system_prompt() {
        let files = vec![
            File {
                path: PathBuf::from("/tmp/AGENTS.md"),
                content: "hello".to_string(),
                template: false,
            },
            File {
                path: PathBuf::from("/tmp/CLAUDE.md"),
                content: "world".to_string(),
                template: false,
            },
        ];
        let prompt = build_system_prompt(&files);
        assert!(prompt.contains("<context source=\"/tmp/AGENTS.md\">\nhello\n</context>"));
        assert!(prompt.contains("<context source=\"/tmp/CLAUDE.md\">\nworld\n</context>"));
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("<foo>"), "&lt;foo&gt;");
        assert_eq!(escape_xml("\"bar\""), "&quot;bar&quot;");
    }

    #[test]
    fn test_paths_between_leaf_not_under_root() {
        let root = PathBuf::from("/a/b");
        let leaf = PathBuf::from("/x/y");
        let paths = paths_between(&root, &leaf);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], leaf);
    }

    #[test]
    fn test_paths_between_same() {
        let root = PathBuf::from("/home/user/project");
        let leaf = PathBuf::from("/home/user/project");
        let paths = paths_between(&root, &leaf);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], root);
    }

    #[test]
    fn test_loader_finds_agents_md() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("AGENTS.md");
        std::fs::write(&agents, "global context").unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();

        // Should find the AGENTS.md in the directory
        assert!(
            files
                .iter()
                .any(|f| f.path.ends_with("AGENTS.md") && f.content == "global context"),
            "expected to find AGENTS.md with 'global context', got: {:?}",
            files
        );
    }

    #[test]
    fn test_loader_finds_claude_md() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        std::fs::write(&claude, "claude context").unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();

        assert!(files
            .iter()
            .any(|f| f.path.ends_with("CLAUDE.md") && f.content == "claude context"));
    }

    #[test]
    fn test_loader_finds_dot_mew_agents_md() {
        let dir = tempfile::tempdir().unwrap();
        let dot_mew = dir.path().join(".mew");
        std::fs::create_dir(&dot_mew).unwrap();
        std::fs::write(dot_mew.join("AGENTS.md"), "dot mew context").unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();

        assert!(files
            .iter()
            .any(|f| f.path.ends_with(".mew/AGENTS.md") && f.content == "dot mew context"));
    }

    #[test]
    fn test_loader_order_most_general_first() {
        let root = tempfile::tempdir().unwrap();
        let subdir = root.path().join("src");
        std::fs::create_dir(&subdir).unwrap();

        // Create a .git directory so find_git_root returns root
        std::fs::create_dir(root.path().join(".git")).unwrap();

        std::fs::write(root.path().join("AGENTS.md"), "root").unwrap();
        std::fs::write(subdir.join("AGENTS.md"), "src").unwrap();

        let loader = Loader::new(&subdir);
        let files = loader.load().unwrap();

        let agents_files: Vec<_> = files
            .iter()
            .filter(|f| f.path.ends_with("AGENTS.md") && f.path.starts_with(root.path()))
            .collect();
        assert_eq!(agents_files.len(), 2);
        assert_eq!(agents_files[0].content, "root");
        assert_eq!(agents_files[1].content, "src");
    }

    #[test]
    fn test_loader_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        // Project files should be empty; global files may exist on dev machine.
        let local: Vec<_> = files
            .iter()
            .filter(|f| f.path.starts_with(dir.path()))
            .collect();
        assert!(local.is_empty(), "expected no project files, got {local:?}");
    }

    #[test]
    fn test_loader_skips_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        let local: Vec<_> = files
            .iter()
            .filter(|f| f.path.starts_with(dir.path()))
            .collect();
        assert!(local.is_empty(), "expected no project files, got {local:?}");
    }

    #[test]
    fn test_expand_includes_inlines_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("conventions.md"), "Use 4 spaces.").unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "Project rules.\n@conventions.md\nEnd.",
        )
        .unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        let agents = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .expect("AGENTS.md found");
        assert!(agents.content.contains("Project rules."));
        assert!(agents.content.contains("Use 4 spaces."));
        assert!(agents.content.contains("End."));
    }

    #[test]
    fn test_expand_includes_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs").join("style.md"), "Be concise.").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Rules.\n@docs/style.md\n").unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        let agents = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .expect("AGENTS.md found");
        assert!(agents.content.contains("Be concise."));
    }

    #[test]
    fn test_expand_includes_rejects_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret.txt"), "password").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Rules.\n@../secret.txt\n").unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        let agents = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .expect("AGENTS.md found");
        // The @include line should be left as-is, not inlined.
        assert!(agents.content.contains("@../secret.txt"));
        assert!(!agents.content.contains("password"));
    }

    #[test]
    fn test_expand_includes_missing_file_left_as_is() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "Rules.\n@nonexistent.md\nEnd.",
        )
        .unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        let agents = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .expect("AGENTS.md found");
        assert!(agents.content.contains("@nonexistent.md"));
        assert!(agents.content.contains("End."));
    }

    #[test]
    fn test_expand_includes_no_at_prefix_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let content = "No includes here.\nJust text.";
        std::fs::write(dir.path().join("AGENTS.md"), content).unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        let agents = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .expect("AGENTS.md found");
        assert_eq!(agents.content, content);
    }

    #[test]
    fn test_load_project_vars_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".mew")).unwrap();
        std::fs::write(
            dir.path().join(".mew").join("project_vars.yaml"),
            "team: platform\nchannel: \"#eng\"\n",
        )
        .unwrap();

        let vars = load_project_vars(dir.path());
        assert_eq!(vars.get("team").unwrap(), "platform");
        assert_eq!(vars.get("channel").unwrap(), "#eng");
    }

    #[test]
    fn test_load_project_vars_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let vars = load_project_vars(dir.path());
        assert!(vars.is_empty());
    }

    #[test]
    fn test_load_project_vars_invalid_yaml_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".mew")).unwrap();
        std::fs::write(
            dir.path().join(".mew").join("project_vars.yaml"),
            "team: [unclosed\n",
        )
        .unwrap();

        let vars = load_project_vars(dir.path());
        assert!(vars.is_empty());
    }

    #[test]
    fn test_load_project_vars_opencode_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".opencode")).unwrap();
        std::fs::write(
            dir.path().join(".opencode").join("project_vars.yaml"),
            "framework: astro\n",
        )
        .unwrap();

        let vars = load_project_vars(dir.path());
        assert_eq!(vars.get("framework").unwrap(), "astro");
    }

    #[test]
    fn test_parse_context_frontmatter_mew_true() {
        let raw = "---\nmew: true\n---\nHello {{ model_id }}";
        let (template, content) = parse_context_frontmatter(raw);
        assert!(template);
        assert_eq!(content, "Hello {{ model_id }}");
    }

    #[test]
    fn test_parse_context_frontmatter_polytoken_true() {
        let raw = "---\npolytoken: true\n---\nHello {{ persona_name }}";
        let (template, content) = parse_context_frontmatter(raw);
        assert!(template);
        assert_eq!(content, "Hello {{ persona_name }}");
    }

    #[test]
    fn test_parse_context_frontmatter_no_frontmatter() {
        let raw = "Just a regular AGENTS.md file.";
        let (template, content) = parse_context_frontmatter(raw);
        assert!(!template);
        assert_eq!(content, raw);
    }

    #[test]
    fn test_parse_context_frontmatter_other_keys() {
        let raw = "---\nauthor: someone\n---\nContent here";
        let (template, content) = parse_context_frontmatter(raw);
        assert!(!template);
        assert_eq!(content, "Content here");
    }

    #[test]
    fn test_templated_context_file_detected_via_loader() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "---\nmew: true\n---\nProject: {{ project_vars.name }}",
        )
        .unwrap();

        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        let agents = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .expect("AGENTS.md found");
        assert!(agents.template);
        assert!(agents.content.contains("{{ project_vars.name }}"));
    }
}
