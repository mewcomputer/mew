//! Framework-independent file diff data for native clients.
//!
//! The model deliberately contains no UI or repository process code. Callers
//! can obtain old/new file contents from git, the daemon, or another source and
//! receive the same line-numbered hunks for rendering and review actions.

use similar::{Algorithm, ChangeTag, TextDiff};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};
use thiserror::Error;

const DEFAULT_CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Unchanged,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub hunks: Vec<DiffHunk>,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Error)]
pub enum DiffError {
    #[error("{path} is not a safe relative repository path")]
    InvalidPath { path: String },
    #[error("{root} is not a git repository: {message}")]
    NotRepository { root: String, message: String },
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read git object for {path}: {message}")]
    Git { path: String, message: String },
}

impl FileDiff {
    pub fn binary(path: impl Into<String>, old_path: Option<String>) -> Self {
        Self {
            path: path.into(),
            old_path,
            status: FileStatus::Binary,
            hunks: Vec::new(),
            added: 0,
            removed: 0,
        }
    }

    pub fn is_changed(&self) -> bool {
        !matches!(self.status, FileStatus::Unchanged)
    }

    pub fn lines(&self) -> impl Iterator<Item = &DiffLine> {
        self.hunks.iter().flat_map(|hunk| hunk.lines.iter())
    }
}

/// Load current worktree contents and their `HEAD` counterparts for changed
/// paths reported by the daemon, then compute render-ready diffs.
pub fn load_worktree_diffs(root: &Path, paths: &[String]) -> Result<Vec<FileDiff>, DiffError> {
    verify_git_repository(root)?;
    paths
        .iter()
        .map(|path| load_worktree_diff(root, path))
        .collect()
}

fn load_worktree_diff(root: &Path, path: &str) -> Result<FileDiff, DiffError> {
    let relative = safe_relative_path(path)?;
    let absolute = root.join(&relative);
    let current = match fs::read(&absolute) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(DiffError::Io {
                path: absolute.display().to_string(),
                source,
            })
        }
    };
    let previous = git_blob(root, path)?;

    if previous
        .as_deref()
        .is_some_and(|bytes| !is_utf8_text(bytes))
        || current.as_deref().is_some_and(|bytes| !is_utf8_text(bytes))
    {
        return Ok(FileDiff::binary(path, None));
    }

    let previous_text = previous
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    let current_text = current
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    Ok(diff_file(
        path,
        None,
        previous_text.as_deref(),
        current_text.as_deref(),
    ))
}

fn verify_git_repository(root: &Path) -> Result<(), DiffError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|source| DiffError::Io {
            path: root.display().to_string(),
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(DiffError::NotRepository {
            root: root.display().to_string(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

fn git_blob(root: &Path, path: &str) -> Result<Option<Vec<u8>>, DiffError> {
    let object = format!("HEAD:{path}");
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &object])
        .output()
        .map_err(|source| DiffError::Io {
            path: path.to_owned(),
            source,
        })?;
    if output.status.success() {
        Ok(Some(output.stdout))
    } else {
        Ok(None)
    }
}

fn safe_relative_path(path: &str) -> Result<PathBuf, DiffError> {
    let relative = Path::new(path);
    if path.is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DiffError::InvalidPath {
            path: path.to_owned(),
        });
    }
    Ok(relative.to_owned())
}

fn is_utf8_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Compute a line-oriented file diff using the default three lines of context.
pub fn diff_file(
    path: impl Into<String>,
    old_path: Option<String>,
    old: Option<&str>,
    new: Option<&str>,
) -> FileDiff {
    diff_file_with_context(path, old_path, old, new, DEFAULT_CONTEXT)
}

/// Compute a line-oriented file diff with an explicit context size.
pub fn diff_file_with_context(
    path: impl Into<String>,
    old_path: Option<String>,
    old: Option<&str>,
    new: Option<&str>,
    context: usize,
) -> FileDiff {
    let path = path.into();
    let status = match (old, new, old_path.as_deref()) {
        (None, Some(_), _) => FileStatus::Added,
        (Some(_), None, _) => FileStatus::Deleted,
        (Some(_), Some(_), Some(previous_path)) if previous_path != path => FileStatus::Renamed,
        (Some(previous), Some(current), _) if previous == current => FileStatus::Unchanged,
        (Some(_), Some(_), _) => FileStatus::Modified,
        (None, None, _) => FileStatus::Unchanged,
    };

    let (Some(old), Some(new)) = (old, new) else {
        let hunks = match status {
            FileStatus::Added => {
                new.and_then(|value| one_sided_hunk(value, LineKind::Addition, false))
            }
            FileStatus::Deleted => {
                old.and_then(|value| one_sided_hunk(value, LineKind::Deletion, true))
            }
            _ => None,
        }
        .into_iter()
        .collect();
        return FileDiff {
            path,
            old_path,
            status,
            hunks,
            added: new.map(line_count).unwrap_or_default(),
            removed: old.map(line_count).unwrap_or_default(),
        };
    };

    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_lines(old, new);
    let mut hunks = Vec::new();
    let mut added = 0;
    let mut removed = 0;

    for group in diff.grouped_ops(context) {
        let first = group.first().expect("grouped diff cannot be empty");
        let last = group.last().expect("grouped diff cannot be empty");
        let old_range = first.old_range().start..last.old_range().end;
        let new_range = first.new_range().start..last.new_range().end;
        let mut lines = Vec::new();

        for op in group {
            for change in diff.iter_changes(&op) {
                let kind = match change.tag() {
                    ChangeTag::Equal => LineKind::Context,
                    ChangeTag::Insert => {
                        added += 1;
                        LineKind::Addition
                    }
                    ChangeTag::Delete => {
                        removed += 1;
                        LineKind::Deletion
                    }
                };
                lines.push(DiffLine {
                    kind,
                    old_line: change.old_index().map(|index| index + 1),
                    new_line: change.new_index().map(|index| index + 1),
                    text: trim_line_ending(change.value_ref()),
                });
            }
        }

        hunks.push(DiffHunk {
            old_start: old_range.start + 1,
            old_count: old_range.len(),
            new_start: new_range.start + 1,
            new_count: new_range.len(),
            lines,
        });
    }

    FileDiff {
        path,
        old_path,
        status,
        hunks,
        added,
        removed,
    }
}

fn trim_line_ending(value: &str) -> String {
    let value = value.strip_suffix('\n').unwrap_or(value);
    let value = value.strip_suffix('\r').unwrap_or(value);
    value.to_owned()
}

fn line_count(value: &str) -> usize {
    if value.is_empty() {
        0
    } else {
        value.lines().count()
    }
}

fn one_sided_hunk(value: &str, kind: LineKind, deletion: bool) -> Option<DiffHunk> {
    let lines = value
        .lines()
        .enumerate()
        .map(|(index, line)| DiffLine {
            kind,
            old_line: deletion.then_some(index + 1),
            new_line: (!deletion).then_some(index + 1),
            text: line.to_owned(),
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then_some(DiffHunk {
        old_start: 1,
        old_count: if deletion { lines.len() } else { 0 },
        new_start: 1,
        new_count: if deletion { 0 } else { lines.len() },
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Output;

    #[test]
    fn builds_numbered_hunks_and_counts_changes() {
        let diff = diff_file(
            "src/lib.rs",
            None,
            Some("fn main() {\n    old();\n}\n"),
            Some("fn main() {\n    new();\n}\n"),
        );

        assert_eq!(diff.status, FileStatus::Modified);
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 1);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].old_start, 1);
        assert_eq!(diff.hunks[0].new_start, 1);
        assert_eq!(diff.hunks[0].old_count, 3);
        assert_eq!(diff.hunks[0].new_count, 3);
        assert!(matches!(
            diff.hunks[0].lines.as_slice(),
            [
                DiffLine {
                    kind: LineKind::Context,
                    old_line: Some(1),
                    new_line: Some(1),
                    text,
                },
                DiffLine {
                    kind: LineKind::Deletion,
                    old_line: Some(2),
                    new_line: None,
                    text: deleted,
                },
                DiffLine {
                    kind: LineKind::Addition,
                    old_line: None,
                    new_line: Some(2),
                    text: added,
                },
                DiffLine {
                    kind: LineKind::Context,
                    old_line: Some(3),
                    new_line: Some(3),
                    text: closing,
                },
            ] if text == "fn main() {"
                && deleted == "    old();"
                && added == "    new();"
                && closing == "}"
        ));
    }

    #[test]
    fn handles_added_deleted_renamed_and_unchanged_files() {
        assert_eq!(
            diff_file("new.txt", None, None, Some("one\ntwo\n")).status,
            FileStatus::Added
        );
        let added = diff_file("new.txt", None, None, Some("one\ntwo\n"));
        assert_eq!(added.hunks[0].lines.len(), 2);
        assert!(added.lines().all(|line| line.kind == LineKind::Addition));
        assert_eq!(
            diff_file("gone.txt", None, Some("one\n"), None).status,
            FileStatus::Deleted
        );
        let deleted = diff_file("gone.txt", None, Some("one\ntwo\n"), None);
        assert_eq!(deleted.hunks[0].lines.len(), 2);
        assert!(deleted.lines().all(|line| line.kind == LineKind::Deletion));
        assert_eq!(
            diff_file(
                "new-name.txt",
                Some("old-name.txt".into()),
                Some("same\n"),
                Some("same\n")
            )
            .status,
            FileStatus::Renamed
        );
        assert!(!diff_file("same.txt", None, Some("same\n"), Some("same\n")).is_changed());
    }

    #[test]
    fn context_size_keeps_separate_change_regions_separate() {
        let old = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
        let new = "one\nTWO\nthree\nfour\nfive\nSIX\nseven\neight\n";
        let diff = diff_file_with_context("file.txt", None, Some(old), Some(new), 0);

        assert_eq!(diff.hunks.len(), 2);
        assert_eq!(diff.hunks[0].lines.len(), 2);
        assert_eq!(diff.hunks[1].lines.len(), 2);
    }

    #[test]
    fn loads_tracked_and_untracked_worktree_files_against_head() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-q"]);
        git(directory.path(), &["config", "user.name", "mew tests"]);
        git(
            directory.path(),
            &["config", "user.email", "mew-tests@example.invalid"],
        );
        std::fs::write(directory.path().join("tracked.txt"), "before\n").unwrap();
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-qm", "baseline"]);

        std::fs::write(directory.path().join("tracked.txt"), "after\n").unwrap();
        std::fs::write(directory.path().join("new.txt"), "new file\n").unwrap();
        let paths = vec!["tracked.txt".into(), "new.txt".into()];
        let diffs = load_worktree_diffs(directory.path(), &paths).unwrap();

        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].status, FileStatus::Modified);
        assert_eq!(diffs[0].added, 1);
        assert_eq!(diffs[0].removed, 1);
        assert_eq!(diffs[1].status, FileStatus::Added);
        assert_eq!(diffs[1].hunks[0].lines[0].text, "new file");
    }

    #[test]
    fn rejects_paths_that_escape_the_repository() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-q"]);
        let paths = vec!["../outside.txt".into()];
        let error = load_worktree_diffs(directory.path(), &paths).unwrap_err();
        assert!(matches!(error, DiffError::InvalidPath { .. }));
    }

    fn git(directory: &Path, args: &[&str]) -> Output {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}
