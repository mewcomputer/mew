//! Shared discovery helpers for mew's loadable markdown directories.
//!
//! Three crates (`mew-skills`, `mew-personas`, `mew-subagents`) each scan
//! the same set of standard locations — `.mew/<kind>`, `.opencode/<kind>`,
//! `.claude/<kind>`, `.agents/<kind>` — under the project's directory tree
//! and `~/.config/...` / `~/.claude/...` / `~/.agents/...` under the user's
//! home. This crate centralizes the walking logic so the three loaders
//! share one implementation.
//!
//! Public surface:
//! - [`find_git_root`] — walk up from `cwd` to find the nearest `.git`.
//! - [`paths_between`] — list every directory between a `root` and a `leaf`
//!   (inclusive). Used to walk project trees from cwd up to the git root.
//! - [`dirs_home`] — return the user's home directory if `HOME` is set.
//! - [`global_dirs_for`] — return the four global locations where a
//!   loadable markdown `<kind>` lives under the user's home.
//!
//! The generic [`HarnessLoader`] in this crate walks a directory tree,
//! finds the right file at each location, and hands it to a caller-provided
//! parse function. All three current loaders (`mew-skills::Loader`,
//! `mew-personas::Loader`, `mew-subagents::Loader`) are thin wrappers
//! around it.

use std::path::{Path, PathBuf};

/// Walk up from `dir` to find the nearest ancestor that contains a `.git`
/// directory. Returns `None` if no git root is found before reaching the
/// filesystem root.
///
/// Used to scope loadable-markdown searches to the current project.
pub fn find_git_root(dir: &Path) -> Option<PathBuf> {
    let mut current = dir.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        {
            let parent = current.parent()?;
            current = parent.to_path_buf()
        }
    }
}

/// List every directory between `root` and `leaf` (inclusive), with `root`
/// as the first element. If `leaf` is not under `root`, returns just
/// `[leaf]`.
///
/// Used to walk a project tree from cwd up to the git root, in order to
/// give project-local definitions priority over project-root definitions.
pub fn paths_between(root: &Path, leaf: &Path) -> Vec<PathBuf> {
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

/// Return the user's home directory if `HOME` is set, otherwise `None`.
pub fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Return the four global locations where a loadable markdown `<kind>` lives
/// under the user's home:
///
/// - `~/.config/mew/<kind>`     (mew's own location)
/// - `~/.config/opencode/<kind>` (opencode's location)
/// - `~/.claude/<kind>`         (claude code's location)
/// - `~/.agents/<kind>`         (agents.sh's location)
///
/// The list is in priority order — earlier wins on duplicate names.
pub fn global_dirs_for(home: &Path, kind: &str) -> Vec<PathBuf> {
    vec![
        home.join(".config").join("mew").join(kind),
        home.join(".config").join("opencode").join(kind),
        home.join(".claude").join(kind),
        home.join(".agents").join(kind),
    ]
}

/// What file to look for at each prefix location.
#[derive(Debug, Clone, Copy)]
pub enum LoadFileSpec {
    /// Look in subdirectories of the prefix dir; load `<subdir>/<filename>`.
    /// Used by skills (subdir/SKILL.md) and personas (subdir/PERSONA.md).
    SubdirFile(&'static str),
    /// Look for `.md` files directly inside the prefix dir. Used by
    /// subagents (flat `.md` files with name = file stem).
    FlatMd,
}

/// Describes where to look for loadable markdown files.
#[derive(Debug, Clone)]
pub struct LoadSpec {
    /// Directory names to look under each project / global parent.
    /// E.g. `[".mew/skills", ".opencode/skills", ".claude/skills", ".agents/skills"]`.
    pub prefixes: &'static [&'static str],
    /// What file at each location is the loadable one.
    pub file: LoadFileSpec,
}

/// Generic loader result: a parsed value paired with the "name" used for
/// dedup. Returned by `parse_and_name`; the loader uses `name` to filter
/// out entries whose name doesn't match the directory or file stem, and to
/// skip duplicates across the standard discovery walk.
pub struct Loaded<T> {
    pub value: T,
    pub name: String,
}

/// Walk the standard discovery paths (project cwd → git root, then `~/.config`
/// / `~/.claude` / `~/.agents` under home) and call `parse_and_name` on every
/// loadable file found. Earlier wins on duplicate names.
///
/// The `name_match` argument controls how the parsed `name` is compared to
/// the directory or file name on disk:
/// - `NameMatch::Subdir` — the subdirectory's name must equal `name`.
///   Used by skills and personas.
/// - `NameMatch::FileStem` — the file's stem must equal `name`. Used by
///   subagents.
pub fn load_markdown_dirs<T, E, F>(
    cwd: &Path,
    spec: &LoadSpec,
    parse_and_name: F,
) -> Result<Vec<T>, E>
where
    F: Fn(&Path) -> Result<Loaded<T>, E>,
    E: From<std::io::Error> + std::fmt::Display,
{
    let mut results: Vec<T> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let root =
        find_git_root(cwd).unwrap_or_else(|| dirs_home().unwrap_or_else(|| cwd.to_path_buf()));
    let project_dirs = paths_between(&root, cwd);

    // Project paths: walk cwd → git root (reverse), so project-local beats
    // project-root.
    for dir in project_dirs.iter().rev() {
        scan_project_dir(dir, spec, &parse_and_name, &mut results, &mut seen)?;
    }

    // Global paths.
    if let Some(home) = dirs_home() {
        for global in global_dirs_for(&home, dir_kind_from_prefix(spec.prefixes)) {
            scan_global_dir(&global, spec, &parse_and_name, &mut results, &mut seen)?;
        }
    }

    Ok(results)
}

/// Like `load_markdown_dirs` but also scans `extra_dirs` (extension-provided
/// paths). Extra dirs are scanned after project dirs but before global dirs,
/// so project-local wins, then extension-provided, then global.
pub fn load_markdown_dirs_with_extra<T, E, F>(
    cwd: &Path,
    spec: &LoadSpec,
    parse_and_name: F,
    extra_dirs: &[PathBuf],
) -> Result<Vec<T>, E>
where
    F: Fn(&Path) -> Result<Loaded<T>, E>,
    E: From<std::io::Error> + std::fmt::Display,
{
    let mut results: Vec<T> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let root =
        find_git_root(cwd).unwrap_or_else(|| dirs_home().unwrap_or_else(|| cwd.to_path_buf()));
    let project_dirs = paths_between(&root, cwd);

    // Project paths: walk cwd → git root (reverse), so project-local beats
    // project-root.
    for dir in project_dirs.iter().rev() {
        scan_project_dir(dir, spec, &parse_and_name, &mut results, &mut seen)?;
    }

    // Extension-provided extra dirs (scanned after project, before global).
    for extra in extra_dirs {
        if extra.is_dir() {
            scan_location(extra, spec, &parse_and_name, &mut results, &mut seen)?;
        }
    }

    // Global paths.
    if let Some(home) = dirs_home() {
        for global in global_dirs_for(&home, dir_kind_from_prefix(spec.prefixes)) {
            scan_global_dir(&global, spec, &parse_and_name, &mut results, &mut seen)?;
        }
    }

    Ok(results)
}

fn dir_kind_from_prefix<'a>(prefixes: &'a [&'a str]) -> &'a str {
    // E.g. ".mew/skills" → "skills"
    prefixes
        .first()
        .and_then(|p| p.split('/').next_back())
        .unwrap_or("")
}

fn scan_project_dir<T, E, F>(
    dir: &Path,
    spec: &LoadSpec,
    parse: &F,
    results: &mut Vec<T>,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), E>
where
    F: Fn(&Path) -> Result<Loaded<T>, E>,
    E: From<std::io::Error> + std::fmt::Display,
{
    for prefix in spec.prefixes {
        let target = dir.join(prefix);
        if !target.is_dir() {
            continue;
        }
        scan_location(&target, spec, parse, results, seen)?;
    }
    Ok(())
}

fn scan_global_dir<T, E, F>(
    dir: &Path,
    spec: &LoadSpec,
    parse: &F,
    results: &mut Vec<T>,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), E>
where
    F: Fn(&Path) -> Result<Loaded<T>, E>,
    E: From<std::io::Error> + std::fmt::Display,
{
    if !dir.is_dir() {
        return Ok(());
    }
    scan_location(dir, spec, parse, results, seen)
}

fn scan_location<T, E, F>(
    location: &Path,
    spec: &LoadSpec,
    parse: &F,
    results: &mut Vec<T>,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), E>
where
    F: Fn(&Path) -> Result<Loaded<T>, E>,
    E: From<std::io::Error> + std::fmt::Display,
{
    let entries = match std::fs::read_dir(location) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_to_load = match spec.file {
            LoadFileSpec::SubdirFile(filename) => {
                // Look for subdirs containing a file with the given name.
                let entry_type = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if !entry_type.is_dir() {
                    continue;
                }
                let subdir = entry.path();
                let target = subdir.join(filename);
                if !target.is_file() {
                    continue;
                }
                let dir_name = subdir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                (target, dir_name)
            }
            LoadFileSpec::FlatMd => {
                // Look for .md files at the top level.
                let path = entry.path();
                let is_md = path.extension().map(|e| e == "md").unwrap_or(false);
                if !is_md {
                    continue;
                }
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string());
                let stem = match stem {
                    Some(s) => s,
                    None => continue,
                };
                (path, stem)
            }
        };
        let (path, on_disk_name) = file_to_load;
        match parse(&path) {
            Ok(loaded) => {
                // Enforce name match: loaded.name must equal the directory or
                // file stem on disk. This catches typos where a user wrote
                // `name: foo` in the frontmatter but the directory is `bar`.
                if loaded.name != on_disk_name {
                    tracing::debug!(
                        name = %loaded.name,
                        on_disk = %on_disk_name,
                        path = %path.display(),
                        "name does not match on-disk identifier, skipping"
                    );
                    continue;
                }
                if seen.contains(&loaded.name) {
                    tracing::trace!(name = %loaded.name, "duplicate, skipping later copy");
                    continue;
                }
                seen.insert(loaded.name.clone());
                results.push(loaded.value);
            }
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "failed to load");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_git_root_returns_none_outside_repo() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(find_git_root(tmp.path()), None);
    }

    #[test]
    fn test_find_git_root_finds_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let git_root = tmp.path();
        std::fs::create_dir(git_root.join(".git")).unwrap();
        let sub = git_root.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(find_git_root(&sub).unwrap(), git_root);
    }

    #[test]
    fn test_paths_between_root_only() {
        let root = PathBuf::from("/repo");
        let paths = paths_between(&root, &root);
        assert_eq!(paths, vec![PathBuf::from("/repo")]);
    }

    #[test]
    fn test_paths_between_walks_down() {
        let root = PathBuf::from("/repo");
        let leaf = PathBuf::from("/repo/a/b/c");
        let paths = paths_between(&root, &leaf);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/repo"),
                PathBuf::from("/repo/a"),
                PathBuf::from("/repo/a/b"),
                PathBuf::from("/repo/a/b/c"),
            ]
        );
    }

    #[test]
    fn test_paths_between_returns_leaf_when_outside_root() {
        let root = PathBuf::from("/repo");
        let leaf = PathBuf::from("/other");
        let paths = paths_between(&root, &leaf);
        assert_eq!(paths, vec![PathBuf::from("/other")]);
    }

    #[test]
    fn test_global_dirs_for_includes_all_four_locations() {
        let home = PathBuf::from("/home/user");
        let dirs = global_dirs_for(&home, "skills");
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/home/user/.config/mew/skills"),
                PathBuf::from("/home/user/.config/opencode/skills"),
                PathBuf::from("/home/user/.claude/skills"),
                PathBuf::from("/home/user/.agents/skills"),
            ]
        );
    }

    #[test]
    fn test_load_markdown_dirs_subdir_file() {
        // Isolate HOME so the global scan doesn't pick up real user files.
        let home = tempfile::tempdir().unwrap();
        // SAFETY: tests run single-threaded for this crate; setting HOME
        // here is the standard pattern for env isolation in Rust tests.
        unsafe {
            std::env::set_var("HOME", home.path());
        }
        let tmp = tempfile::tempdir().unwrap();
        // Set up `.mew/skills/alpha/SKILL.md` and `.mew/skills/beta/SKILL.md`
        // with `name: alpha` and `name: beta` respectively. Also a misnamed
        // entry (`name: gamma` in a dir called `delta`) that must be skipped.
        let skills_dir = tmp.path().join(".mew").join("skills");
        std::fs::create_dir_all(skills_dir.join("alpha")).unwrap();
        std::fs::create_dir_all(skills_dir.join("beta")).unwrap();
        std::fs::create_dir_all(skills_dir.join("delta")).unwrap();
        std::fs::write(skills_dir.join("alpha/SKILL.md"), "name: alpha\n").unwrap();
        std::fs::write(skills_dir.join("beta/SKILL.md"), "name: beta\n").unwrap();
        std::fs::write(skills_dir.join("delta/SKILL.md"), "name: gamma\n").unwrap();

        // No git root in the temp dir, so it falls back to cwd.
        let cwd = tmp.path();
        let spec = LoadSpec {
            prefixes: &[".mew/skills"],
            file: LoadFileSpec::SubdirFile("SKILL.md"),
        };
        let result = load_markdown_dirs::<_, std::io::Error, _>(cwd, &spec, |path| {
            let body = std::fs::read_to_string(path)?;
            let name = body
                .lines()
                .find_map(|l| l.strip_prefix("name: "))
                .unwrap_or("")
                .trim()
                .to_string();
            Ok(Loaded {
                name,
                value: path.display().to_string(),
            })
        });
        let loaded = result.unwrap();
        // alpha + beta loaded; delta skipped because name mismatch.
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|p| p.contains("alpha")));
        assert!(loaded.iter().any(|p| p.contains("beta")));
        assert!(!loaded.iter().any(|p| p.contains("delta")));
    }

    #[test]
    fn test_load_markdown_dirs_flat_md() {
        let home = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("HOME", home.path());
        }
        let tmp = tempfile::tempdir().unwrap();
        // Set up `.mew/agents/researcher.md` with `name: researcher`.
        // Plus a misnamed entry to verify the name-match check.
        let agents_dir = tmp.path().join(".mew").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("researcher.md"), "name: researcher\n").unwrap();
        std::fs::write(agents_dir.join("reviewer.md"), "name: code_reviewer\n").unwrap();

        let cwd = tmp.path();
        let spec = LoadSpec {
            prefixes: &[".mew/agents"],
            file: LoadFileSpec::FlatMd,
        };
        let result = load_markdown_dirs::<_, std::io::Error, _>(cwd, &spec, |path| {
            let body = std::fs::read_to_string(path)?;
            let name = body
                .lines()
                .find_map(|l| l.strip_prefix("name: "))
                .unwrap_or("")
                .trim()
                .to_string();
            Ok(Loaded {
                name,
                value: path.file_name().unwrap().to_string_lossy().to_string(),
            })
        });
        let loaded = result.unwrap();
        assert_eq!(
            loaded.len(),
            1,
            "researcher.md loads; reviewer.md skipped (name mismatch)"
        );
        assert_eq!(loaded[0], "researcher.md");
    }
}
