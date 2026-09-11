//! Canonical workspace identity shared by daemon session and project views.

use std::path::{Component, Path, PathBuf};

/// Resolve a session's workspace identity from its working directory and the
/// configured workspace roots.
///
/// The longest configured root containing the working directory wins. When no
/// configured root contains it, the normalized working directory itself is the
/// workspace identity. Invalid or missing working directories are left out of
/// workspace grouping.
pub(crate) fn resolve_workspace_path(
    cwd: Option<&str>,
    configured_roots: &[PathBuf],
) -> Option<String> {
    let cwd = cwd.filter(|path| !path.trim().is_empty())?;
    let cwd = normalize_path(Path::new(cwd))?;
    let roots = configured_roots
        .iter()
        .filter_map(|root| normalize_path(root))
        .collect::<Vec<_>>();

    roots
        .into_iter()
        .filter(|root| cwd.starts_with(root))
        .max_by_key(|root| root.components().count())
        .or(Some(cwd))
        .map(|path| path.to_string_lossy().into_owned())
}

fn normalize_path(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    std::fs::canonicalize(&absolute).ok().or_else(|| {
        let mut normalized = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                other => normalized.push(other.as_os_str()),
            }
        }
        normalized.is_absolute().then_some(normalized)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_the_longest_matching_configured_root() {
        let roots = vec![PathBuf::from("/tmp"), PathBuf::from("/tmp/work")];
        assert_eq!(
            resolve_workspace_path(Some("/tmp/work/product/src"), &roots),
            Some("/tmp/work".into())
        );
    }

    #[test]
    fn falls_back_to_normalized_cwd_when_no_root_matches() {
        let roots = vec![PathBuf::from("/tmp/work")];
        assert_eq!(
            resolve_workspace_path(Some("/tmp/other/../other/product"), &roots),
            Some("/tmp/other/product".into())
        );
    }

    #[test]
    fn missing_or_empty_cwd_has_no_workspace() {
        let roots = vec![PathBuf::from("/tmp")];
        assert_eq!(resolve_workspace_path(None, &roots), None);
        assert_eq!(resolve_workspace_path(Some("  "), &roots), None);
    }
}
