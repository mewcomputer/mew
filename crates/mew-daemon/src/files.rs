//! File service for the daemon: directory listing, file preview, git status,
//! and open-in-OS. All paths are scoped to the session's cwd — this is a
//! security boundary since the web bridge is on TCP.

use std::path::{Path, PathBuf};

use mew_protocol::{DirEntry, GitEntry, GitFileStatus, ServerMessage};

use crate::session::SessionManager;

/// Resolve a relative path against the session's cwd, ensuring the result
/// stays within the workspace. Returns an error if the path escapes.
fn resolve_scoped(cwd: &Path, relative: Option<&str>) -> Result<PathBuf, String> {
    let base = cwd;
    let target = match relative {
        None | Some("") | Some(".") => base.to_path_buf(),
        Some(p) => {
            // Absolute paths are checked by the same containment rule below.
            if p.starts_with('/') {
                PathBuf::from(p)
            } else {
                base.join(p)
            }
        }
    };

    // Canonicalize both base and target, then check containment.
    let canon_base = base
        .canonicalize()
        .map_err(|e| format!("cwd canonicalize: {e}"))?;
    let canon_target = target
        .canonicalize()
        .map_err(|e| format!("path canonicalize: {e}"))?;

    if !canon_target.starts_with(&canon_base) {
        return Err(format!(
            "path escapes workspace: {}",
            canon_target.display()
        ));
    }

    Ok(canon_target)
}

fn filesystem_path_allowed(path: &Path, home: &Path) -> bool {
    let protected = ["/System", "/Library", "/private", "/Volumes"];
    path.starts_with(home) && !protected.iter().any(|prefix| path.starts_with(prefix))
}

fn filesystem_root_allowed(path: &Path) -> bool {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    filesystem_path_allowed(path, &home)
}

pub async fn handle_list_filesystem_dir(path: Option<String>) -> Result<ServerMessage, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "home directory unavailable".to_string())?;
    let target = match path.as_deref() {
        None | Some("") | Some("~") => home,
        Some(value) => PathBuf::from(value),
    };
    let target = target
        .canonicalize()
        .map_err(|e| format!("path canonicalize: {e}"))?;
    if !filesystem_root_allowed(&target) {
        return Err(format!(
            "path is outside user workspace: {}",
            target.display()
        ));
    }
    let mut entries = Vec::new();
    let mut reader = tokio::fs::read_dir(&target)
        .await
        .map_err(|e| format!("read_dir: {e}"))?;
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|e| format!("read_dir: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        if is_dir {
            entries.push(DirEntry {
                name,
                is_dir: true,
                size: None,
            });
        }
    }
    entries.sort_by_key(|a| a.name.to_lowercase());
    Ok(ServerMessage::FilesystemDirListing {
        path: target.display().to_string(),
        entries,
    })
}

/// Handle `ListDir`.
pub async fn handle_list_dir(
    sm: &SessionManager,
    session_id: &str,
    path: Option<String>,
) -> Result<ServerMessage, String> {
    let cwd = sm
        .session_cwd(session_id)
        .await
        .ok_or_else(|| "session has no cwd".to_string())?;

    let target = resolve_scoped(&cwd, path.as_deref())?;
    let display_path = target
        .strip_prefix(cwd.canonicalize().unwrap_or(cwd.clone()))
        .unwrap_or(&target)
        .display()
        .to_string();

    let mut entries = Vec::new();
    let mut reader = tokio::fs::read_dir(&target)
        .await
        .map_err(|e| format!("read_dir: {e}"))?;

    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|e| format!("read_dir: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden files (dotfiles).
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        let size = if !is_dir {
            entry.metadata().await.ok().map(|m| m.len())
        } else {
            None
        };
        entries.push(DirEntry { name, is_dir, size });
    }

    // Sort: dirs first, then files, alphabetically.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(ServerMessage::DirListing {
        session_id: session_id.to_owned(),
        path: display_path,
        entries,
    })
}

/// Handle `ReadFilePreview`.
pub async fn handle_read_preview(
    sm: &SessionManager,
    session_id: &str,
    path: &str,
    max_bytes: Option<u64>,
) -> Result<ServerMessage, String> {
    let cwd = sm
        .session_cwd(session_id)
        .await
        .ok_or_else(|| "session has no cwd".to_string())?;

    let limit = max_bytes.unwrap_or(65536).min(1024 * 1024) as usize;

    use tokio::io::AsyncReadExt;
    let requested = path.to_string();
    let read = async move {
        let (file, target) = tokio::task::spawn_blocking(move || {
            let target = resolve_scoped(&cwd, Some(&requested))?;
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(nix::libc::O_NONBLOCK);
            }
            let file = options
                .open(&target)
                .map_err(|e| format!("open file: {e}"))?;
            if !file
                .metadata()
                .map_err(|e| format!("file metadata: {e}"))?
                .is_file()
            {
                return Err("not a regular file".to_string());
            }
            Ok::<_, String>((file, target))
        })
        .await
        .map_err(|e| format!("open file: {e}"))??;
        let file = tokio::fs::File::from_std(file);
        let mut bytes = Vec::new();
        file.take((limit as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| format!("read file: {e}"))?;
        Ok::<_, String>((bytes, target))
    };
    let (bytes, target) = tokio::time::timeout(std::time::Duration::from_secs(5), read)
        .await
        .map_err(|_| "file preview timed out".to_string())??;

    let truncated = bytes.len() > limit;
    let content_bytes = if truncated {
        &bytes[..limit]
    } else {
        &bytes[..]
    };

    // Convert to string, handling non-UTF8 gracefully.
    let content = String::from_utf8_lossy(content_bytes).to_string();

    // Derive language from extension for syntax highlighting.
    let language = target
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_string());

    Ok(ServerMessage::FilePreview {
        session_id: Some(session_id.to_string()),
        path: path.to_string(),
        content,
        truncated,
        language,
    })
}

/// Handle `GitStatus` — shell out to `git status --porcelain=v2 -z`.
pub async fn handle_git_status(
    sm: &SessionManager,
    session_id: &str,
) -> Result<ServerMessage, String> {
    let cwd = sm
        .session_cwd(session_id)
        .await
        .ok_or_else(|| "session has no cwd".to_string())?;

    let prefix = tokio::process::Command::new("git")
        .args(["rev-parse", "--show-prefix"])
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|e| format!("git rev-parse: {e}"))?;
    if !prefix.status.success() {
        return Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&prefix.stderr)
        ));
    }
    let prefix = String::from_utf8_lossy(&prefix.stdout);
    let prefix = prefix.strip_suffix('\n').unwrap_or(&prefix);

    let output = tokio::process::Command::new("git")
        .arg("status")
        .arg("--porcelain=v2")
        .arg("--untracked-files=all")
        .arg("-z")
        .args(["--", "."])
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|e| format!("git status: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries = parse_porcelain_v2(&stdout)
        .into_iter()
        .filter_map(|mut entry| {
            entry.path = entry.path.strip_prefix(prefix)?.to_owned();
            Some(entry)
        })
        .collect();

    Ok(ServerMessage::GitStatusResult {
        session_id: Some(session_id.to_string()),
        entries,
    })
}

/// Parse `git status --porcelain=v2 -z` output.
fn parse_porcelain_v2(output: &str) -> Vec<GitEntry> {
    let mut entries = Vec::new();

    // -z uses NUL as separator instead of newline.
    let mut records = output.split('\0');
    while let Some(line) = records.next() {
        if line.is_empty() {
            continue;
        }

        // Porcelain v2 formats:
        // Changed: "1 <xy> <sub> <mH> <mI> <mW> <hH> <hI> <path>"
        // Renamed: "2 <xy> <sub> <mH> <mI> <mW> <hH> <hI> <R%> <path>\0<orig>"
        // Unmerged: "u <xy> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>"
        // Untracked: "? <path>"
        if let Some(path) = line.strip_prefix("? ") {
            entries.push(GitEntry {
                path: path.to_string(),
                status: GitFileStatus::Untracked,
            });
        } else if let Some(rest) = line.strip_prefix("1 ") {
            let parts: Vec<&str> = rest.splitn(8, ' ').collect();
            if parts.len() >= 8 {
                let xy = parts[0];
                let path = parts[7];
                let status = classify_xy(xy);
                entries.push(GitEntry {
                    path: path.to_string(),
                    status,
                });
            }
        } else if let Some(rest) = line.strip_prefix("2 ") {
            let _original_path = records.next();
            let parts: Vec<&str> = rest.splitn(9, ' ').collect();
            if parts.len() >= 9 {
                let xy = parts[0];
                let path = parts[8];
                let status = classify_xy(xy);
                entries.push(GitEntry {
                    path: path.to_string(),
                    status,
                });
            }
        } else if let Some(rest) = line.strip_prefix("u ") {
            let parts: Vec<&str> = rest.splitn(10, ' ').collect();
            if parts.len() >= 10 {
                entries.push(GitEntry {
                    path: parts[9].to_string(),
                    status: GitFileStatus::Modified,
                });
            }
        }
    }

    entries
}

/// Classify the XY status pair from porcelain v2 into our simplified enum.
fn classify_xy(xy: &str) -> GitFileStatus {
    let x = xy.chars().next().unwrap_or(' ');
    let y = xy.chars().nth(1).unwrap_or(' ');
    match (x, y) {
        ('A', _) | (_, 'A') => GitFileStatus::Added,
        ('D', _) | (_, 'D') => GitFileStatus::Deleted,
        ('R', _) => GitFileStatus::Renamed,
        _ => GitFileStatus::Modified,
    }
}

/// Handle `OpenPath` — open a file or directory in the OS default app.
pub async fn handle_open_path(
    sm: &SessionManager,
    session_id: &str,
    path: &str,
) -> Result<(), String> {
    let cwd = sm
        .session_cwd(session_id)
        .await
        .ok_or_else(|| "session has no cwd".to_string())?;

    let target = resolve_scoped(&cwd, Some(path))?;

    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    #[cfg(windows)]
    let cmd = "cmd";

    #[cfg(windows)]
    {
        tokio::process::Command::new(cmd)
            .args(["/C", "start", "", &target.display().to_string()])
            .spawn()
            .map_err(|e| format!("open: {e}"))?;
    }
    #[cfg(not(windows))]
    {
        tokio::process::Command::new(cmd)
            .arg(&target)
            .spawn()
            .map_err(|e| format!("open: {e}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{filesystem_path_allowed, parse_porcelain_v2, resolve_scoped};
    use std::path::Path;

    #[test]
    fn status_parser_handles_rename_sources_and_conflicts() {
        let status = "2 R. N... 100644 100644 100644 abc def R100 new.txt\0? ghost\0u UU N... 100644 100644 100644 100644 abc def ghi conflicted.txt\0";
        let entries = parse_porcelain_v2(status);
        assert_eq!(
            entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            vec!["new.txt", "conflicted.txt"]
        );
    }

    #[test]
    fn filesystem_browser_stays_inside_home_and_skips_protected_roots() {
        let home = Path::new("/Users/tester");
        assert!(filesystem_path_allowed(
            Path::new("/Users/tester/projects"),
            home
        ));
        assert!(!filesystem_path_allowed(Path::new("/Users/other"), home));
        assert!(!filesystem_path_allowed(Path::new("/System"), home));
        assert!(!filesystem_path_allowed(Path::new("/Library"), home));
        assert!(!filesystem_path_allowed(Path::new("/private/tmp"), home));
    }

    #[test]
    fn absolute_paths_inside_the_cwd_resolve_but_outside_paths_do_not() {
        let root = tempfile::tempdir().unwrap();
        let inside = root.path().join("note.md");
        std::fs::write(&inside, "hello").unwrap();
        assert_eq!(
            resolve_scoped(root.path(), Some(inside.to_str().unwrap())).unwrap(),
            inside.canonicalize().unwrap()
        );

        let error = resolve_scoped(root.path(), Some("/etc/passwd")).unwrap_err();
        assert!(error.contains("path escapes workspace"), "{error}");
    }
}
