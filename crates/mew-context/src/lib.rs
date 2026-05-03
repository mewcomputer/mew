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
    /// `AGENTS.md`, `CLAUDE.md`, and `.mew/AGENTS.md` along the way. Also loads
    /// `~/.config/mew/AGENTS.md` if present.
    ///
    /// Files are returned from most-general to most-specific.
    pub fn load(&self) -> Result<Vec<File>, ContextError> {
        let mut files = Vec::new();

        // Global config file first.
        if let Some(cfg_dir) = config_dir() {
            let p = cfg_dir.join("AGENTS.md");
            if let Some(f) = try_read(&p) {
                trace!(?p, "loaded global context file");
                files.push(f);
            }
        }

        // Determine root: git worktree root or home.
        let root = find_git_root(&self.cwd).unwrap_or_else(|_| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.cwd.clone())
        });

        // Collect paths from root down to cwd so most-general comes first.
        let paths = paths_between(&root, &self.cwd);
        for dir in &paths {
            for name in &["AGENTS.md", "CLAUDE.md"] {
                let p = dir.join(name);
                if let Some(f) = try_read(&p) {
                    trace!(?p, "loaded project context file");
                    files.push(f);
                }
            }
            let p = dir.join(".mew").join("AGENTS.md");
            if let Some(f) = try_read(&p) {
                trace!(?p, "loaded .mew context file");
                files.push(f);
            }
        }

        debug!(count = files.len(), "loaded context files");
        Ok(files)
    }
}

fn try_read(path: &Path) -> Option<File> {
    std::fs::read_to_string(path).ok().map(|content| File {
        path: path.to_path_buf(),
        content,
    })
}

fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("ai", "mew", "mew").map(|d| d.config_dir().to_path_buf())
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
            },
            File {
                path: PathBuf::from("/tmp/CLAUDE.md"),
                content: "world".to_string(),
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
            .filter(|f| f.path.ends_with("AGENTS.md"))
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
        assert!(files.is_empty());
    }

    #[test]
    fn test_loader_skips_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Loader::new(dir.path());
        let files = loader.load().unwrap();
        assert!(files.is_empty());
    }
}
