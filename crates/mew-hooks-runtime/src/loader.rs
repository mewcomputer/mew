//! Plugin discovery and loading.

use std::path::{Path, PathBuf};

/// Discovers plugin executables in plugin directories.
pub struct PluginLoader {
    dirs: Vec<PathBuf>,
}

impl PluginLoader {
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }

    pub fn default_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        if let Some(home) = directories::UserDirs::new() {
            dirs.push(home.home_dir().join(".config").join("mew").join("plugins"));
        }

        if let Ok(cwd) = std::env::current_dir() {
            dirs.push(cwd.join(".mew").join("plugins"));
        }

        dirs
    }

    /// Scans plugin directories and returns discovered executable paths,
    /// sorted alphabetically by filename for deterministic hook ordering.
    pub fn discover_executables(&self) -> Vec<PathBuf> {
        let mut plugins = Vec::new();

        for dir in &self.dirs {
            if !dir.is_dir() {
                continue;
            }
            match std::fs::read_dir(dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if is_executable(&path) {
                            plugins.push(path);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("cannot read plugin dir {}: {}", dir.display(), e);
                }
            }
        }

        plugins.sort_by(|a, b| {
            a.file_name()
                .unwrap_or_default()
                .cmp(b.file_name().unwrap_or_default())
        });

        plugins
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
        && !path
            .extension()
            .is_some_and(|e| e == "wasm" || e == "dylib" || e == "so" || e == "dll")
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
        && !path
            .extension()
            .map_or(false, |e| e == "wasm" || e == "dll" || e == "exe.lib")
}
