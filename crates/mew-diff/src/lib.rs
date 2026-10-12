//! Framework-independent diffs and bounded git workspace loading.
//!
//! The daemon reads its session workspace and sends line-numbered hunks to clients.

use serde::{Deserialize, Serialize};
use similar::{Algorithm, ChangeTag, TextDiff};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};
use thiserror::Error;

const DEFAULT_CONTEXT: usize = 3;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REVIEW_WORK_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Unchanged,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineKind {
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[error("workspace review exceeds its work limit")]
    ReviewTooLarge,
    #[error("cannot review {path}: file exceeds the 8 MiB review limit")]
    TooLarge { path: String },
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

struct ReviewBudget(usize);

impl Default for ReviewBudget {
    fn default() -> Self {
        Self(MAX_REVIEW_WORK_BYTES)
    }
}

impl ReviewBudget {
    fn charge(&mut self, bytes: usize) -> Result<(), DiffError> {
        self.0 = self.0.checked_sub(bytes).ok_or(DiffError::ReviewTooLarge)?;
        Ok(())
    }

    fn contents(&mut self, contents: Option<&[u8]>) -> Result<(), DiffError> {
        if let Some(bytes) = contents {
            self.charge(bytes.len())?;
            if let Ok(text) = std::str::from_utf8(bytes) {
                // Bound line vectors, hunk vectors and text copies before diffing.
                let line_bytes = std::mem::size_of::<DiffLine>() + std::mem::size_of::<DiffHunk>();
                self.charge(text.lines().count().saturating_mul(line_bytes))?;
                self.charge(bytes.len())?;
            }
        }
        Ok(())
    }
}

struct DiffBase {
    canonical_root: PathBuf,
    prefix: String,
    head: Option<String>,
}

/// Discover the session workspace's changed files and read fresh contents
/// against HEAD, including untracked files and both sides of a rename.
pub fn load_session_diffs(root: &Path) -> Result<Vec<FileDiff>, DiffError> {
    let base = DiffBase::read(root)?;
    let status = git_output(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ],
    )?;
    let mut records = status
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let mut diffs = Vec::new();
    let mut budget = ReviewBudget::default();
    budget.charge(status.len())?;
    while let Some(record) = records.next() {
        if record.len() < 4 {
            return Err(DiffError::Git {
                path: root.display().to_string(),
                message: "invalid git status record".into(),
            });
        }
        let name = std::str::from_utf8(&record[3..]).map_err(|_| DiffError::InvalidPath {
            path: "non-UTF-8 repository path".into(),
        })?;
        let renamed = record[..2].contains(&b'R') || record[..2].contains(&b'C');
        let previous = if renamed {
            records
                .next()
                .map(|record| std::str::from_utf8(record))
                .transpose()
                .map_err(|_| DiffError::InvalidPath { path: name.into() })?
        } else {
            None
        };
        let Some(path) = name.strip_prefix(&base.prefix) else {
            continue;
        };
        let old_path = previous.and_then(|name| name.strip_prefix(&base.prefix));
        let diff = load_named_diff(root, &base, path, old_path, &mut budget)?;
        if diff.is_changed() {
            diffs.push(diff);
        }
    }
    Ok(diffs)
}

fn load_named_diff(
    root: &Path,
    base: &DiffBase,
    path: &str,
    old_path: Option<&str>,
    budget: &mut ReviewBudget,
) -> Result<FileDiff, DiffError> {
    budget.charge(std::mem::size_of::<FileDiff>() + path.len() + old_path.map_or(0, str::len))?;
    let relative = safe_relative_path(path)?;
    let absolute = root.join(&relative);
    let mut parent = absolute.parent().unwrap_or(root);
    while !parent.exists() {
        parent = parent
            .parent()
            .ok_or_else(|| DiffError::InvalidPath { path: path.into() })?;
    }
    if !parent
        .canonicalize()
        .map_err(|source| DiffError::Io {
            path: parent.display().to_string(),
            source,
        })?
        .starts_with(&base.canonical_root)
    {
        return Err(DiffError::InvalidPath { path: path.into() });
    }
    let contents = match fs::symlink_metadata(&absolute) {
        Ok(metadata) if metadata.is_dir() => {
            return Ok(FileDiff::binary(path, old_path.map(str::to_owned)));
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::read_link(&absolute).map(|target| target.to_string_lossy().as_bytes().to_vec())
        }
        Ok(metadata) => {
            if metadata.len() > MAX_FILE_BYTES {
                return Err(DiffError::TooLarge { path: path.into() });
            }
            fs::File::open(&absolute).and_then(|file| {
                let mut bytes = Vec::with_capacity(metadata.len() as usize);
                file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
                Ok(bytes)
            })
        }
        Err(error) => Err(error),
    };
    let current = match contents {
        Ok(bytes) if bytes.len() as u64 > MAX_FILE_BYTES => {
            return Err(DiffError::TooLarge { path: path.into() });
        }
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(DiffError::Io {
                path: absolute.display().to_string(),
                source,
            })
        }
    };
    let previous = match git_blob(root, base, old_path.unwrap_or(path))? {
        Some(GitBlob::File(bytes)) => Some(bytes),
        Some(GitBlob::Gitlink) => return Ok(FileDiff::binary(path, old_path.map(str::to_owned))),
        None => None,
    };
    budget.contents(current.as_deref())?;
    budget.contents(previous.as_deref())?;

    if previous
        .as_deref()
        .is_some_and(|bytes| !is_utf8_text(bytes))
        || current.as_deref().is_some_and(|bytes| !is_utf8_text(bytes))
    {
        return Ok(FileDiff::binary(path, old_path.map(str::to_owned)));
    }

    let previous_text = previous
        .as_deref()
        .map(|bytes| std::str::from_utf8(bytes).expect("validated text"));
    let current_text = current
        .as_deref()
        .map(|bytes| std::str::from_utf8(bytes).expect("validated text"));
    Ok(diff_file(
        path,
        old_path.map(str::to_owned),
        previous_text,
        current_text,
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

impl DiffBase {
    fn read(root: &Path) -> Result<Self, DiffError> {
        verify_git_repository(root)?;
        let canonical_root = root.canonicalize().map_err(|source| DiffError::Io {
            path: root.display().to_string(),
            source,
        })?;
        let prefix = git_output(root, &["rev-parse", "--show-prefix"])?;
        let prefix = String::from_utf8_lossy(&prefix)
            .trim_end_matches('\n')
            .to_owned();
        let head = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--verify", "--quiet", "HEAD"])
            .output()
            .map_err(|source| DiffError::Io {
                path: root.display().to_string(),
                source,
            })?;
        if !head.status.success() {
            let reference = git_output(root, &["symbolic-ref", "-q", "HEAD"])?;
            let reference = String::from_utf8_lossy(&reference);
            let exists = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["show-ref", "--verify", "--quiet", reference.trim()])
                .output()
                .map_err(|source| DiffError::Io {
                    path: root.display().to_string(),
                    source,
                })?;
            if exists.status.code() == Some(1) {
                return Ok(Self {
                    canonical_root,
                    prefix,
                    head: None,
                });
            }
            return Err(DiffError::Git {
                path: root.display().to_string(),
                message: "could not resolve HEAD".into(),
            });
        }
        Ok(Self {
            canonical_root,
            prefix,
            head: Some(String::from_utf8_lossy(&head.stdout).trim().to_owned()),
        })
    }
}

enum GitBlob {
    File(Vec<u8>),
    Gitlink,
}

fn git_blob(root: &Path, base: &DiffBase, path: &str) -> Result<Option<GitBlob>, DiffError> {
    safe_relative_path(path)?;
    let Some(head) = base.head.as_deref() else {
        return Ok(None);
    };
    let object = format!("{}{path}", base.prefix);
    let pathspec = format!(":(top,literal){object}");
    let entries = git_output(
        root,
        &["ls-tree", "-z", "-l", "--full-tree", head, "--", &pathspec],
    )?;
    if entries.is_empty() {
        return Ok(None);
    }
    let header = entries
        .split(|byte| *byte == b'\t')
        .next()
        .unwrap_or_default();
    let mut fields = header
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty());
    fields.next();
    if fields.next() == Some(b"commit") {
        return Ok(Some(GitBlob::Gitlink));
    }
    let oid = fields
        .next()
        .and_then(|oid| std::str::from_utf8(oid).ok())
        .ok_or_else(|| DiffError::Git {
            path: path.into(),
            message: "invalid git tree entry".into(),
        })?;
    let size = fields
        .next()
        .and_then(|size| std::str::from_utf8(size).ok())
        .and_then(|size| size.parse::<u64>().ok())
        .ok_or_else(|| DiffError::Git {
            path: path.into(),
            message: "invalid git blob size".into(),
        })?;
    if size > MAX_FILE_BYTES {
        return Err(DiffError::TooLarge { path: path.into() });
    }
    git_output(root, &["cat-file", "blob", oid]).map(|bytes| Some(GitBlob::File(bytes)))
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, DiffError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| DiffError::Io {
            path: root.display().to_string(),
            source,
        })?;
    std::thread::scope(|scope| {
        let stderr = child.stderr.take().expect("piped git stderr");
        let errors = scope.spawn(move || {
            let mut stderr = stderr;
            let mut bytes = Vec::new();
            let result = stderr
                .by_ref()
                .take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes);
            // Keep draining after the diagnostic cap so git can always exit.
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            result.map(|_| bytes)
        });
        let mut bytes = Vec::new();
        let read = child
            .stdout
            .take()
            .expect("piped git stdout")
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes);
        let oversized = bytes.len() as u64 > MAX_FILE_BYTES;
        if read.is_err() || oversized {
            let _ = child.kill();
        }
        let status = child.wait();
        let stderr = errors.join().expect("git stderr reader");
        if oversized {
            return Err(DiffError::ReviewTooLarge);
        }
        let io_error = |source| DiffError::Io {
            path: root.display().to_string(),
            source,
        };
        read.map_err(io_error)?;
        let status = status.map_err(io_error)?;
        let stderr = stderr.map_err(io_error)?;
        if status.success() {
            Ok(bytes)
        } else {
            Err(DiffError::Git {
                path: root.display().to_string(),
                message: String::from_utf8_lossy(&stderr).trim().into(),
            })
        }
    })
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
        let diffs = load_session_diffs(directory.path()).unwrap();

        assert_eq!(diffs.len(), 2);
        let tracked = diffs
            .iter()
            .find(|diff| diff.path == "tracked.txt")
            .unwrap();
        let added = diffs.iter().find(|diff| diff.path == "new.txt").unwrap();
        assert_eq!(tracked.status, FileStatus::Modified);
        assert_eq!(tracked.added, 1);
        assert_eq!(tracked.removed, 1);
        assert_eq!(added.status, FileStatus::Added);
        assert_eq!(added.hunks[0].lines[0].text, "new file");
    }

    #[test]
    fn rejects_paths_that_escape_the_repository() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-q"]);
        let base = DiffBase::read(directory.path()).unwrap();
        let error = load_named_diff(
            directory.path(),
            &base,
            "../outside.txt",
            None,
            &mut ReviewBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(error, DiffError::InvalidPath { .. }));
    }

    #[test]
    fn git_discovery_output_is_bounded_before_collecting_all_records() {
        let directory = repository();
        fs::write(
            directory.path().join("large.txt"),
            vec![b'x'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let object = git(directory.path(), &["hash-object", "-w", "large.txt"]);
        let object = String::from_utf8(object.stdout).unwrap();
        let result = git_output(directory.path(), &["cat-file", "blob", object.trim()]);
        assert!(matches!(result, Err(DiffError::ReviewTooLarge)));
        assert!(git_output(directory.path(), &["rev-parse", "HEAD"]).is_ok());
    }

    #[test]
    fn review_stops_when_individually_valid_files_exceed_the_work_budget() {
        let directory = repository();
        for index in 0..5 {
            fs::write(
                directory.path().join(format!("a{index}.txt")),
                vec![b'x'; 4 * 1024 * 1024],
            )
            .unwrap();
        }
        fs::write(
            directory.path().join("z-too-large.txt"),
            vec![b'x'; 9 * 1024 * 1024],
        )
        .unwrap();
        let error = match load_session_diffs(directory.path()) {
            Err(error) => error,
            Ok(_) => panic!("oversized workspace must fail"),
        };
        assert!(
            error.to_string().contains("workspace review exceeds"),
            "{error}"
        );
    }

    #[test]
    fn review_bounds_line_allocation_before_building_hunks() {
        let directory = repository();
        fs::write(directory.path().join("dense.txt"), vec![b'\n'; 1024 * 1024]).unwrap();
        let error = match load_session_diffs(directory.path()) {
            Err(error) => error,
            Ok(_) => panic!("dense lines must fail before allocation"),
        };
        assert!(
            error.to_string().contains("workspace review exceeds"),
            "{error}"
        );
    }

    #[test]
    fn discovers_changes_and_refreshes_same_count_edits() {
        let directory = repository();
        fs::write(directory.path().join("tracked.txt"), "after\n").unwrap();
        fs::write(directory.path().join("new file.txt"), "new\n").unwrap();
        let first = load_session_diffs(directory.path()).unwrap();
        assert_eq!(first.len(), 2);
        let tracked = first
            .iter()
            .find(|diff| diff.path == "tracked.txt")
            .unwrap();
        assert_eq!((tracked.added, tracked.removed), (1, 1));
        fs::write(directory.path().join("tracked.txt"), "other\n").unwrap();
        let second = load_session_diffs(directory.path()).unwrap();
        let changed = second
            .iter()
            .find(|diff| diff.path == "tracked.txt")
            .unwrap();
        assert_eq!((changed.added, changed.removed), (1, 1));
        assert!(changed.lines().any(|line| line.text == "other"));
        assert_ne!(tracked, changed);
    }

    #[test]
    fn discovers_renamed_deleted_binary_and_unborn_files() {
        let directory = repository();
        git(directory.path(), &["mv", "tracked.txt", "renamed.txt"]);
        let renamed = load_session_diffs(directory.path()).unwrap();
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].status, FileStatus::Renamed);
        assert_eq!(renamed[0].old_path.as_deref(), Some("tracked.txt"));
        fs::remove_file(directory.path().join("renamed.txt")).unwrap();
        fs::write(directory.path().join("binary.bin"), [0, 1, 2]).unwrap();
        let changed = load_session_diffs(directory.path()).unwrap();
        assert!(changed
            .iter()
            .any(|diff| diff.status == FileStatus::Deleted));
        assert!(changed.iter().any(|diff| diff.status == FileStatus::Binary));
        let unborn = tempfile::tempdir().unwrap();
        git(unborn.path(), &["init", "-q"]);
        fs::write(unborn.path().join("new.txt"), "first\n").unwrap();
        assert_eq!(
            load_session_diffs(unborn.path()).unwrap()[0].status,
            FileStatus::Added
        );
    }

    #[cfg(unix)]
    #[test]
    fn renders_symlink_contents_and_rejects_symlink_parent_escape() {
        let directory = repository();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "private content\n").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            directory.path().join("link.txt"),
        )
        .unwrap();
        let diffs = load_session_diffs(directory.path()).unwrap();
        assert!(!diffs[0].lines().any(|line| line.text == "private content"));
        std::os::unix::fs::symlink(outside.path(), directory.path().join("escape")).unwrap();
        let base = DiffBase::read(directory.path()).unwrap();
        assert!(matches!(
            load_named_diff(
                directory.path(),
                &base,
                "escape/secret.txt",
                None,
                &mut ReviewBudget::default()
            ),
            Err(DiffError::InvalidPath { .. })
        ));
    }

    #[test]
    fn subdirectory_reviews_use_their_own_head_paths() {
        let directory = repository();
        fs::create_dir(directory.path().join("nested")).unwrap();
        fs::write(directory.path().join("nested/file.txt"), "nested before\n").unwrap();
        git(directory.path(), &["add", "nested/file.txt"]);
        git(directory.path(), &["commit", "-qm", "nested"]);
        fs::write(directory.path().join("nested/file.txt"), "nested after\n").unwrap();
        fs::write(directory.path().join("tracked.txt"), "outside nested\n").unwrap();
        let diffs = load_session_diffs(&directory.path().join("nested")).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "file.txt");
        assert_eq!(diffs[0].status, FileStatus::Modified);
        assert!(diffs[0].lines().any(|line| line.text == "nested before"));
    }

    #[test]
    fn git_object_failures_are_visible_instead_of_added_files() {
        let directory = repository();
        let blob = git(directory.path(), &["rev-parse", "HEAD:tracked.txt"]);
        let blob = String::from_utf8(blob.stdout).unwrap();
        let blob = blob.trim();
        fs::remove_file(
            directory
                .path()
                .join(".git/objects")
                .join(&blob[..2])
                .join(&blob[2..]),
        )
        .unwrap();
        fs::write(directory.path().join("tracked.txt"), "after\n").unwrap();
        assert!(matches!(
            load_session_diffs(directory.path()),
            Err(DiffError::Git { .. })
        ));
    }

    #[test]
    fn changed_gitlinks_do_not_break_review_of_other_files() {
        let directory = repository();
        let child = directory.path().join("child");
        fs::create_dir(&child).unwrap();
        git(&child, &["init", "-q"]);
        git(&child, &["config", "user.name", "mew tests"]);
        git(
            &child,
            &["config", "user.email", "mew-tests@example.invalid"],
        );
        fs::write(child.join("child.txt"), "before\n").unwrap();
        git(&child, &["add", "child.txt"]);
        git(&child, &["commit", "-qm", "child baseline"]);
        git(directory.path(), &["add", "child"]);
        git(directory.path(), &["commit", "-qm", "gitlink"]);
        fs::write(child.join("child.txt"), "after\n").unwrap();
        fs::write(directory.path().join("tracked.txt"), "after\n").unwrap();
        let diffs = load_session_diffs(directory.path()).unwrap();
        assert!(diffs
            .iter()
            .any(|diff| diff.path == "child" && diff.status == FileStatus::Binary));
        assert!(diffs
            .iter()
            .any(|diff| diff.path == "tracked.txt" && diff.status == FileStatus::Modified));
        fs::remove_dir_all(child).unwrap();
        assert!(load_session_diffs(directory.path())
            .unwrap()
            .iter()
            .any(|diff| diff.path == "child" && diff.status == FileStatus::Binary));
    }

    #[test]
    fn oversized_current_and_head_files_fail_before_diffing() {
        let directory = repository();
        fs::write(
            directory.path().join("tracked.txt"),
            vec![b'x'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(
            matches!(load_session_diffs(directory.path()), Err(DiffError::TooLarge { path }) if path == "tracked.txt")
        );
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-qm", "large baseline"]);
        fs::write(directory.path().join("tracked.txt"), "small\n").unwrap();
        assert!(
            matches!(load_session_diffs(directory.path()), Err(DiffError::TooLarge { path }) if path == "tracked.txt")
        );
    }

    #[test]
    fn whitespace_directories_and_git_pathspec_characters_are_literal() {
        let directory = repository();
        let nested = directory.path().join("  nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("[abc].txt"), "literal before\n").unwrap();
        fs::write(nested.join("a.txt"), "neighbor before\n").unwrap();
        git(directory.path(), &["add", "  nested"]);
        git(directory.path(), &["commit", "-qm", "literal paths"]);
        fs::write(nested.join("[abc].txt"), "literal after\n").unwrap();
        let diffs = load_session_diffs(&nested).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "[abc].txt");
        assert_eq!(diffs[0].status, FileStatus::Modified);
        assert!(diffs[0].lines().any(|line| line.text == "literal before"));
        assert!(!diffs[0].lines().any(|line| line.text == "neighbor before"));
    }

    fn repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-q"]);
        git(directory.path(), &["config", "user.name", "mew tests"]);
        git(
            directory.path(),
            &["config", "user.email", "mew-tests@example.invalid"],
        );
        fs::write(directory.path().join("tracked.txt"), "before\n").unwrap();
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-qm", "baseline"]);
        directory
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
