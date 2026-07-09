//! Extension package discovery.
//!
//! Scans `~/.config/mew/extensions/<name>/` (global) and
//! `.mew/extensions/<name>/` (project-local) for directories containing
//! `mew-ext.toml`. Bare executables in `plugins/` are handled by
//! `PluginLoader` (unchanged).

use std::path::{Path, PathBuf};

use crate::manifest::{parse_manifest, ExtensionManifest};

/// Whether an extension is global or project-local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionScope {
    /// `~/.config/mew/extensions/<name>/` — daemon-scoped, one instance.
    Global,
    /// `.mew/extensions/<name>/` — project-local, one per project root.
    Project,
}

/// A discovered extension package (with a manifest).
#[derive(Debug, Clone)]
pub struct DiscoveredExtension {
    /// The extension name (from the manifest or directory name).
    pub name: String,
    /// Path to the package root (containing mew-ext.toml).
    pub root: PathBuf,
    /// Parsed manifest.
    pub manifest: ExtensionManifest,
    /// Whether this is a global or project-local extension.
    pub scope: ExtensionScope,
}

impl DiscoveredExtension {
    /// Returns the resolved `[provides]` paths for this extension.
    /// Each path is resolved relative to the package root.
    /// Only non-None fields are included.
    pub fn provides_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(skills) = &self.manifest.provides.skills {
            dirs.push(self.root.join(skills));
        }
        if let Some(commands) = &self.manifest.provides.commands {
            dirs.push(self.root.join(commands));
        }
        if let Some(personas) = &self.manifest.provides.personas {
            dirs.push(self.root.join(personas));
        }
        if let Some(subagents) = &self.manifest.provides.subagents {
            dirs.push(self.root.join(subagents));
        }
        dirs
    }

    /// Returns the resolved `provides.skills` path, if any.
    pub fn provides_skills(&self) -> Option<PathBuf> {
        self.manifest
            .provides
            .skills
            .as_ref()
            .map(|p| self.root.join(p))
    }

    /// Returns the resolved `provides.personas` path, if any.
    pub fn provides_personas(&self) -> Option<PathBuf> {
        self.manifest
            .provides
            .personas
            .as_ref()
            .map(|p| self.root.join(p))
    }

    /// Returns the resolved `provides.subagents` path, if any.
    pub fn provides_subagents(&self) -> Option<PathBuf> {
        self.manifest
            .provides
            .subagents
            .as_ref()
            .map(|p| self.root.join(p))
    }

    /// Whether this extension has an `entry.run` (i.e., it spawns a process).
    pub fn has_entry(&self) -> bool {
        self.manifest.extension.entry.is_some()
    }

    /// The spawn command from the manifest's `entry.run`, if present.
    pub fn run_command(&self) -> Option<&[String]> {
        self.manifest
            .extension
            .entry
            .as_ref()
            .map(|e| e.run.as_slice())
    }
}

/// Discover all extension packages in the standard locations.
///
/// Scans:
/// - `~/.config/mew/extensions/<name>/mew-ext.toml` (global)
/// - `.mew/extensions/<name>/mew-ext.toml` (project-local)
///
/// A package is a directory containing `mew-ext.toml`. Directories
/// without `mew-ext.toml` are ignored. Bare executables in `plugins/`
/// are handled by `PluginLoader` (unchanged).
///
/// Dedup precedence: project beats global. If the same extension name
/// appears in both, the project one wins.
pub fn discover_extensions(cwd: &Path) -> Vec<DiscoveredExtension> {
    let mut extensions = Vec::new();

    // Project-local: .mew/extensions/<name>/
    let project_ext_dir = cwd.join(".mew").join("extensions");
    if project_ext_dir.is_dir() {
        for ext in scan_extensions_dir(&project_ext_dir, ExtensionScope::Project) {
            extensions.push(ext);
        }
    }

    // Global: ~/.config/mew/extensions/<name>/
    if let Some(home) = directories::UserDirs::new() {
        let global_ext_dir = home
            .home_dir()
            .join(".config")
            .join("mew")
            .join("extensions");
        if global_ext_dir.is_dir() {
            for ext in scan_extensions_dir(&global_ext_dir, ExtensionScope::Global) {
                extensions.push(ext);
            }
        }
    }

    // Dedup: project beats global. Keep first occurrence (project dirs
    // are scanned first).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    extensions.retain(|ext| seen.insert(ext.name.clone()));

    // Sort by name for deterministic ordering.
    extensions.sort_by(|a, b| a.name.cmp(&b.name));

    extensions
}

/// Scan a single extensions directory for packages.
fn scan_extensions_dir(dir: &Path, scope: ExtensionScope) -> Vec<DiscoveredExtension> {
    let mut results = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("cannot read extensions dir {}: {}", dir.display(), e);
            return results;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("mew-ext.toml");
        if !manifest_path.exists() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        match parse_manifest(&manifest_path) {
            Ok(manifest) => {
                // Use the manifest's name if it differs from the directory name.
                let name = if manifest.extension.name.is_empty() {
                    name
                } else {
                    manifest.extension.name.clone()
                };
                results.push(DiscoveredExtension {
                    name,
                    root: path,
                    manifest,
                    scope,
                });
            }
            Err(e) => {
                tracing::warn!(
                    "failed to parse manifest at {}: {}",
                    manifest_path.display(),
                    e
                );
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_package_with_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir.path().join(".mew").join("extensions").join("test-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("mew-ext.toml"),
            r#"
[extension]
name = "test-ext"
version = "0.1.0"
"#,
        )
        .unwrap();

        let extensions = discover_extensions(dir.path());
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "test-ext");
        assert_eq!(extensions[0].scope, ExtensionScope::Project);
        assert_eq!(extensions[0].manifest.extension.version, "0.1.0");
    }

    #[test]
    fn test_discover_ignores_dirs_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir
            .path()
            .join(".mew")
            .join("extensions")
            .join("no-manifest");
        std::fs::create_dir_all(&ext_dir).unwrap();
        // No mew-ext.toml — should be ignored.

        let extensions = discover_extensions(dir.path());
        assert!(extensions.is_empty());
    }

    #[test]
    fn test_dedup_precedence() {
        // Create both a project and global extension with the same name.
        let dir = tempfile::tempdir().unwrap();

        // Project: .mew/extensions/dup-ext/
        let project_ext = dir.path().join(".mew").join("extensions").join("dup-ext");
        std::fs::create_dir_all(&project_ext).unwrap();
        std::fs::write(
            project_ext.join("mew-ext.toml"),
            r#"
[extension]
name = "dup-ext"
version = "0.2.0"
"#,
        )
        .unwrap();

        // Global: ~/.config/mew/extensions/dup-ext/
        // We can't easily set HOME in a unit test, so we test the dedup
        // logic directly by simulating two extensions with the same name.
        let project = DiscoveredExtension {
            name: "dup-ext".into(),
            root: project_ext.clone(),
            manifest: parse_manifest(&project_ext.join("mew-ext.toml")).unwrap(),
            scope: ExtensionScope::Project,
        };

        // Simulate a global extension with the same name.
        let global_dir = tempfile::tempdir().unwrap();
        let global_ext = global_dir.path().join("dup-ext");
        std::fs::create_dir_all(&global_ext).unwrap();
        std::fs::write(
            global_ext.join("mew-ext.toml"),
            r#"
[extension]
name = "dup-ext"
version = "0.1.0"
"#,
        )
        .unwrap();
        let global = DiscoveredExtension {
            name: "dup-ext".into(),
            root: global_ext.clone(),
            manifest: parse_manifest(&global_ext.join("mew-ext.toml")).unwrap(),
            scope: ExtensionScope::Global,
        };

        // Simulate the dedup: project first, then global.
        let mut all = vec![project.clone(), global];
        let mut seen = std::collections::HashSet::new();
        all.retain(|ext| seen.insert(ext.name.clone()));

        assert_eq!(all.len(), 1);
        assert_eq!(all[0].scope, ExtensionScope::Project);
        assert_eq!(all[0].manifest.extension.version, "0.2.0");
    }

    #[test]
    fn test_provides_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir.path().join("my-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("mew-ext.toml"),
            r#"
[extension]
name = "my-ext"
version = "0.1.0"

[provides]
skills = "skills/"
personas = "personas/"
subagents = "agents/"
"#,
        )
        .unwrap();

        let manifest = parse_manifest(&ext_dir.join("mew-ext.toml")).unwrap();
        let ext = DiscoveredExtension {
            name: "my-ext".into(),
            root: ext_dir.clone(),
            manifest,
            scope: ExtensionScope::Project,
        };

        assert_eq!(ext.provides_skills(), Some(ext_dir.join("skills/")));
        assert_eq!(ext.provides_personas(), Some(ext_dir.join("personas/")));
        assert_eq!(ext.provides_subagents(), Some(ext_dir.join("agents/")));
    }
}
