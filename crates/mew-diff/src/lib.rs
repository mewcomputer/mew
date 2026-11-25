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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLayer {
    Staged,
    Unstaged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,
    CrLf,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending: Option<LineEnding>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<DiffLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
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
            layer: None,
            patch: None,
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
    #[cfg(not(unix))]
    canonical_root: PathBuf,
    git_root: PathBuf,
    #[cfg(unix)]
    root_fd: fs::File,
    prefix: String,
    head: Option<String>,
}

impl DiffBase {
    fn git_root(&self) -> &Path {
        &self.git_root
    }
}

/// Discover the session workspace's changed files and read fresh contents
/// against HEAD, including untracked files and both sides of a rename.
pub fn load_session_diffs(root: &Path) -> Result<Vec<FileDiff>, DiffError> {
    load_selected_diffs(root, None)
}

/// Load staged and unstaged diffs for one changed file.
pub fn load_session_file_diffs(root: &Path, path: &str) -> Result<Vec<FileDiff>, DiffError> {
    safe_relative_path(path)?;
    load_selected_diffs(root, Some(path))
}

fn load_selected_diffs(
    root: &Path,
    selected_path: Option<&str>,
) -> Result<Vec<FileDiff>, DiffError> {
    let base = DiffBase::read(root)?;
    let root = base.git_root();
    let status = if let Some(path) = selected_path {
        // Discover tracked renames without enumerating unrelated untracked files.
        // A second literal pathspec picks up the selected file when it is untracked.
        let mut status = git_output(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=no",
                "--",
                ".",
            ],
        )?;
        let pathspec = format!(":(literal){path}");
        status.extend(git_output(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--",
                &pathspec,
            ],
        )?);
        status
    } else {
        git_output(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--",
                ".",
            ],
        )?
    };
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
        let name_bytes = &record[3..];
        let renamed = record[..2].contains(&b'R') || record[..2].contains(&b'C');
        let previous_bytes = if renamed { records.next() } else { None };
        if let Some(selected) = selected_path {
            if name_bytes.strip_prefix(base.prefix.as_bytes()) != Some(selected.as_bytes()) {
                continue;
            }
        }
        let name = std::str::from_utf8(name_bytes).map_err(|_| DiffError::InvalidPath {
            path: "non-UTF-8 repository path".into(),
        })?;
        let previous = previous_bytes
            .map(std::str::from_utf8)
            .transpose()
            .map_err(|_| DiffError::InvalidPath { path: name.into() })?;
        let Some(path) = name.strip_prefix(&base.prefix) else {
            continue;
        };
        let old_path = previous.and_then(|name| name.strip_prefix(&base.prefix));
        if selected_path.is_some() {
            diffs.extend(load_named_layers(
                root,
                &base,
                path,
                old_path,
                &record[..2],
                &mut budget,
            )?);
            break;
        }
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
    let current = match read_worktree_blob(root, base, path)? {
        Some(GitBlob::File(bytes)) => Some(bytes),
        Some(GitBlob::Gitlink) => return Ok(FileDiff::binary(path, old_path.map(str::to_owned))),
        None => None,
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

#[cfg(unix)]
fn open_workspace_root(root: &Path) -> std::io::Result<fs::File> {
    use nix::fcntl::{open, openat, OFlag};
    use nix::sys::stat::Mode;
    use std::os::fd::{AsRawFd, FromRawFd};
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let fd = open(Path::new("/"), flags, Mode::empty()).map_err(std::io::Error::from)?;
    // SAFETY: open returns a new owned descriptor, transferred to File exactly once.
    let mut directory = unsafe { fs::File::from_raw_fd(fd) };
    for component in root.components() {
        match component {
            Component::Normal(name) => {
                let fd = openat(Some(directory.as_raw_fd()), name, flags, Mode::empty())
                    .map_err(std::io::Error::from)?;
                // SAFETY: openat returns a new owned descriptor, transferred to File exactly once.
                directory = unsafe { fs::File::from_raw_fd(fd) };
            }
            Component::CurDir | Component::RootDir => {}
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "unsafe path",
                ))
            }
        }
    }
    Ok(directory)
}

#[cfg(unix)]
fn open_relative_parent(
    mut directory: fs::File,
    path: &Path,
) -> std::io::Result<(fs::File, std::ffi::OsString)> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    use std::os::fd::{AsRawFd, FromRawFd};
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    for component in path.parent().unwrap_or(Path::new("")).components() {
        match component {
            Component::Normal(name) => {
                let fd = openat(Some(directory.as_raw_fd()), name, flags, Mode::empty())
                    .map_err(std::io::Error::from)?;
                // SAFETY: openat returns a new owned descriptor, transferred to File exactly once.
                directory = unsafe { fs::File::from_raw_fd(fd) };
            }
            Component::CurDir => {}
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "unsafe path",
                ))
            }
        }
    }
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing file name")
    })?;
    Ok((directory, name.to_os_string()))
}

#[cfg(all(unix, test))]
fn open_workspace_parent(
    root: &Path,
    path: &Path,
) -> std::io::Result<(fs::File, std::ffi::OsString)> {
    open_relative_parent(open_workspace_root(root)?, path)
}

#[cfg(unix)]
fn read_worktree_leaf(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    path: &str,
) -> Result<Option<(GitBlob, u32)>, DiffError> {
    use nix::fcntl::{openat, readlinkat, AtFlags, OFlag};
    use nix::sys::stat::{fstatat, Mode, SFlag};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let fd = parent.as_raw_fd();
    let metadata = match fstatat(Some(fd), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Err(error) => {
            return Err(DiffError::Io {
                path: path.into(),
                source: error.into(),
            })
        }
    };
    let kind = SFlag::from_bits_truncate(metadata.st_mode) & SFlag::S_IFMT;
    if kind == SFlag::S_IFDIR {
        return Ok(Some((GitBlob::Gitlink, 0o160000)));
    }
    if kind == SFlag::S_IFLNK {
        let target = readlinkat(Some(fd), name).map_err(|error| DiffError::Io {
            path: path.into(),
            source: error.into(),
        })?;
        return Ok(Some((
            GitBlob::File(target.as_os_str().as_bytes().to_vec()),
            0o120000,
        )));
    }
    if kind != SFlag::S_IFREG {
        return Err(DiffError::InvalidPath { path: path.into() });
    }
    if metadata.st_size as u64 > MAX_FILE_BYTES {
        return Err(DiffError::TooLarge { path: path.into() });
    }
    let flags = OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let fd = openat(Some(fd), name, flags, Mode::empty()).map_err(|error| DiffError::Io {
        path: path.into(),
        source: error.into(),
    })?;
    // SAFETY: openat returns a new owned descriptor, transferred to File exactly once.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(|source| DiffError::Io {
        path: path.into(),
        source,
    })?;
    if !meta.is_file() {
        return Err(DiffError::InvalidPath { path: path.into() });
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(DiffError::TooLarge { path: path.into() });
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| DiffError::Io {
            path: path.into(),
            source,
        })?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(DiffError::TooLarge { path: path.into() });
    }
    use std::os::unix::fs::PermissionsExt;
    let mode = if meta.permissions().mode() & 0o111 != 0 {
        0o100755
    } else {
        0o100644
    };
    Ok(Some((GitBlob::File(bytes), mode)))
}

#[cfg(unix)]
fn read_worktree_snapshot(
    _root: &Path,
    base: &DiffBase,
    path: &str,
) -> Result<Option<(GitBlob, u32)>, DiffError> {
    let relative = safe_relative_path(path)?;
    let pinned = base.root_fd.try_clone().map_err(|source| DiffError::Io {
        path: path.into(),
        source,
    })?;
    match open_relative_parent(pinned, &relative) {
        Ok((parent, name)) => read_worktree_leaf(&parent, &name, path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(nix::libc::ELOOP | nix::libc::ENOTDIR)
            ) =>
        {
            Err(DiffError::InvalidPath { path: path.into() })
        }
        Err(source) => Err(DiffError::Io {
            path: path.into(),
            source,
        }),
    }
}

#[cfg(unix)]
fn read_worktree_blob(
    _root: &Path,
    base: &DiffBase,
    path: &str,
) -> Result<Option<GitBlob>, DiffError> {
    Ok(read_worktree_snapshot(_root, base, path)?.map(|(blob, _)| blob))
}

#[cfg(not(unix))]
fn read_worktree_blob(
    root: &Path,
    base: &DiffBase,
    path: &str,
) -> Result<Option<GitBlob>, DiffError> {
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
            return Ok(Some(GitBlob::Gitlink));
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::read_link(&absolute).map(|target| target.to_string_lossy().as_bytes().to_vec())
        }
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(DiffError::InvalidPath { path: path.into() });
            }
            if metadata.len() > MAX_FILE_BYTES {
                return Err(DiffError::TooLarge { path: path.into() });
            }
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_NOFOLLOW);
            }
            options.open(&absolute).and_then(|file| {
                if !file.metadata()?.is_file() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "not a regular file",
                    ));
                }
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
    Ok(current.map(GitBlob::File))
}

#[cfg(unix)]
fn index_snapshot(root: &Path, path: &str) -> Result<Option<(Vec<u8>, u32)>, DiffError> {
    let pathspec = format!(":(literal){path}");
    let entries = git_output(root, &["ls-files", "--stage", "-z", "--", &pathspec])?;
    if entries.is_empty() {
        return Ok(None);
    }
    let mut fields = entries
        .split(|byte| *byte == b'\t')
        .next()
        .unwrap_or_default()
        .split(|byte| *byte == b' ');
    let mode = fields
        .next()
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| u32::from_str_radix(value, 8).ok())
        .ok_or_else(|| DiffError::Git {
            path: path.into(),
            message: "invalid index mode".into(),
        })?;
    let object = fields
        .next()
        .and_then(|value| std::str::from_utf8(value).ok())
        .ok_or_else(|| DiffError::Git {
            path: path.into(),
            message: "invalid index object".into(),
        })?;
    if fields.next() != Some(&b"0"[..]) {
        return Err(DiffError::Git {
            path: path.into(),
            message: "unmerged file; resolve the conflict in an editor".into(),
        });
    }
    if mode == 0o160000 {
        return Ok(Some((Vec::new(), mode)));
    }
    let size = git_output(root, &["cat-file", "-s", object])?;
    let size = std::str::from_utf8(&size)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .ok_or_else(|| DiffError::Git {
            path: path.into(),
            message: "invalid index blob size".into(),
        })?;
    if size > MAX_FILE_BYTES {
        return Err(DiffError::TooLarge { path: path.into() });
    }
    Ok(Some((
        git_output(root, &["cat-file", "blob", object])?,
        mode,
    )))
}

#[cfg(unix)]
fn worktree_snapshot_patch(root: &Path, base: &DiffBase, path: &str) -> Result<String, DiffError> {
    use std::os::unix::fs::PermissionsExt;
    let index = index_snapshot(root, path)?;
    let current = read_worktree_snapshot(root, base, path)?;
    if index.as_ref().is_some_and(|(_, mode)| *mode == 0o160000)
        || current.as_ref().is_some_and(|(_, mode)| *mode == 0o160000)
    {
        return Ok("Subproject commit has working-tree changes.".into());
    }
    let temp = tempfile::tempdir().map_err(|source| DiffError::Io {
        path: path.into(),
        source,
    })?;
    let object_dir = temp.path().join("objects");
    fs::create_dir(&object_dir).map_err(|source| DiffError::Io {
        path: path.into(),
        source,
    })?;
    let current = match current {
        Some((GitBlob::File(bytes), mode)) if mode != 0o120000 => {
            // Apply Git's configured clean filters to the captured bytes. Objects are
            // written only to the private temporary object directory.
            let path_arg = format!("--path={path}");
            let object = run_git(
                root,
                &["hash-object", "-w", "--stdin", &path_arg],
                Some(&object_dir),
                Some(&bytes),
                false,
            )?;
            let object = std::str::from_utf8(&object)
                .map_err(|_| DiffError::Git {
                    path: path.into(),
                    message: "invalid object id".into(),
                })?
                .trim();
            Some((
                run_git(
                    root,
                    &["cat-file", "blob", object],
                    Some(&object_dir),
                    None,
                    false,
                )?,
                mode,
            ))
        }
        Some((GitBlob::File(bytes), mode)) => Some((bytes, mode)),
        Some((GitBlob::Gitlink, _)) => unreachable!(),
        None => None,
    };
    let attr = git_output(root, &["check-attr", "-z", "diff", "--", path])?;
    let attr = attr.split(|byte| *byte == 0).nth(2).unwrap_or_default();
    let driver_binary = match attr {
        b"unset" => Some(true),
        b"set" => Some(false),
        b"unspecified" | b"" => None,
        name => {
            let name = std::str::from_utf8(name).map_err(|_| DiffError::Git {
                path: path.into(),
                message: "invalid diff attribute".into(),
            })?;
            let key = format!("diff.{name}.binary");
            let raw = git_output(root, &["config", "--default=unspecified", "--get", &key])?;
            if raw == b"unspecified\n" {
                None
            } else {
                Some(git_output(root, &["config", "--type=bool", "--get", &key])? == b"true\n")
            }
        }
    };
    let binary = driver_binary == Some(true);
    if binary && index.as_ref().map(|(bytes, _)| bytes) != current.as_ref().map(|(bytes, _)| bytes)
    {
        return Ok("Binary files a/selected and b/selected differ\n".into());
    }
    let mut current = current;
    let filemode = git_output(
        root,
        &[
            "config",
            "--type=bool",
            "--default=true",
            "--get",
            "core.filemode",
        ],
    )?;
    if filemode == b"false\n" {
        if let (
            Some((_, old_mode @ (0o100644 | 0o100755))),
            Some((_, new_mode @ (0o100644 | 0o100755))),
        ) = (&index, &mut current)
        {
            *new_mode = *old_mode;
        }
    }
    let old_dir = temp.path().join("before");
    let new_dir = temp.path().join("after");
    let io_error = |source| DiffError::Io {
        path: path.into(),
        source,
    };
    fs::create_dir(&old_dir).map_err(io_error)?;
    fs::create_dir(&new_dir).map_err(io_error)?;
    let filename = Path::new(path)
        .file_name()
        .ok_or_else(|| DiffError::InvalidPath { path: path.into() })?;
    let old_path = old_dir.join(filename);
    let new_path = new_dir.join(filename);
    for (file_path, snapshot) in [(&old_path, &index), (&new_path, &current)] {
        if let Some((bytes, mode)) = snapshot {
            fs::write(file_path, bytes).map_err(io_error)?;
            let mut permissions = fs::metadata(file_path).map_err(io_error)?.permissions();
            permissions.set_mode(if *mode == 0o100755 { 0o755 } else { 0o644 });
            fs::set_permissions(file_path, permissions).map_err(io_error)?;
        }
    }
    let old_arg = if index.is_some() {
        old_path.to_string_lossy().into_owned()
    } else {
        "/dev/null".into()
    };
    let new_arg = if current.is_some() {
        new_path.to_string_lossy().into_owned()
    } else {
        "/dev/null".into()
    };
    let mut args = vec![
        "diff",
        "--no-index",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
    ];
    if driver_binary == Some(false) {
        args.push("--text");
    }
    args.extend(["--", &old_arg, &new_arg]);
    let patch = run_git(root, &args, None, None, true)?;
    let mut patch =
        String::from_utf8(patch).map_err(|_| DiffError::InvalidPath { path: path.into() })?;
    let old_mode = index.as_ref().map(|(_, mode)| *mode);
    let new_mode = current.as_ref().map(|(_, mode)| *mode);
    if let Some((old_mode, new_mode)) = old_mode
        .zip(new_mode)
        .filter(|(old, new)| old != new && (*old == 0o120000 || *new == 0o120000))
    {
        let mode = format!("old mode {old_mode:o}\nnew mode {new_mode:o}\n");
        if let Some(end) = patch.find('\n') {
            patch.insert_str(end + 1, &mode);
        } else {
            patch = format!("diff --git a/preview b/preview\n{mode}");
        }
    }
    Ok(patch)
}

#[cfg(not(unix))]
fn worktree_snapshot_patch(
    _root: &Path,
    _base: &DiffBase,
    path: &str,
) -> Result<String, DiffError> {
    Err(DiffError::Git {
        path: path.into(),
        message: "scoped worktree diffs are unavailable on this platform".into(),
    })
}

fn load_named_layers(
    root: &Path,
    base: &DiffBase,
    path: &str,
    old_path: Option<&str>,
    status: &[u8],
    budget: &mut ReviewBudget,
) -> Result<Vec<FileDiff>, DiffError> {
    if status != b"??" {
        // Git owns index, mode, filter, newline and submodule semantics.
        // Validate the worktree entry before asking Git to inspect it.
        let _ = read_worktree_blob(root, base, path)?;
        let mut diffs = Vec::new();
        for (index, layer) in [DiffLayer::Staged, DiffLayer::Unstaged]
            .into_iter()
            .enumerate()
        {
            if status.get(index).copied().unwrap_or(b' ') == b' ' {
                continue;
            }
            let mut args = vec![
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--src-prefix=a/",
                "--dst-prefix=b/",
            ];
            if layer == DiffLayer::Staged {
                args.push("--cached");
            }
            args.push("--");
            let selected = format!(":(literal){path}");
            let previous = old_path.map(|old| format!(":(literal){old}"));
            args.push(&selected);
            if let Some(ref old) = previous {
                args.push(old);
            }
            let patch = if layer == DiffLayer::Staged {
                git_output(root, &args)?
            } else {
                worktree_snapshot_patch(root, base, path)?.into_bytes()
            };
            budget.charge(patch.len())?;
            if patch.is_empty() {
                continue;
            }
            let patch = String::from_utf8(patch)
                .map_err(|_| DiffError::InvalidPath { path: path.into() })?;
            diffs.push(FileDiff {
                path: path.into(),
                old_path: old_path.map(str::to_owned),
                status: match status[index] {
                    b'A' => FileStatus::Added,
                    b'D' => FileStatus::Deleted,
                    b'R' => FileStatus::Renamed,
                    _ => FileStatus::Modified,
                },
                hunks: Vec::new(),
                added: 0,
                removed: 0,
                layer: Some(layer),
                patch: Some(patch),
            });
        }
        return Ok(diffs);
    }
    let current = read_worktree_blob(root, base, path)?;
    let mut diff = match current {
        Some(GitBlob::File(bytes)) => {
            budget.contents(Some(&bytes))?;
            if is_utf8_text(&bytes) {
                diff_file(
                    path,
                    None,
                    None,
                    Some(std::str::from_utf8(&bytes).expect("validated")),
                )
            } else {
                FileDiff::binary(path, None)
            }
        }
        Some(GitBlob::Gitlink) => FileDiff::binary(path, None),
        None => return Ok(Vec::new()),
    };
    diff.layer = Some(DiffLayer::Unstaged);
    Ok(vec![diff])
}

fn verify_git_repository(root: &Path) -> Result<(), DiffError> {
    match git_output(root, &["rev-parse", "--show-toplevel"]) {
        Ok(_) => Ok(()),
        Err(DiffError::Git { message, .. }) if message != "git command timed out" => {
            Err(DiffError::NotRepository {
                root: root.display().to_string(),
                message,
            })
        }
        Err(error) => Err(error),
    }
}

impl DiffBase {
    fn read(root: &Path) -> Result<Self, DiffError> {
        let canonical_root = root.canonicalize().map_err(|source| DiffError::Io {
            path: root.display().to_string(),
            source,
        })?;
        #[cfg(unix)]
        let root_fd = open_workspace_root(&canonical_root).map_err(|source| DiffError::Io {
            path: root.display().to_string(),
            source,
        })?;
        #[cfg(unix)]
        let git_root = {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!("/dev/fd/{}", root_fd.as_raw_fd()))
        };
        #[cfg(not(unix))]
        let git_root = canonical_root.clone();
        verify_git_repository(&git_root)?;
        let prefix = git_output(&git_root, &["rev-parse", "--show-prefix"])?;
        let prefix = String::from_utf8_lossy(&prefix)
            .trim_end_matches('\n')
            .to_owned();
        let head = match git_output(&git_root, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
            Ok(head) => Some(String::from_utf8_lossy(&head).trim().to_owned()),
            Err(DiffError::Git { message, .. }) if message.is_empty() => {
                let reference = git_output(&git_root, &["symbolic-ref", "-q", "HEAD"])?;
                let reference = String::from_utf8_lossy(&reference);
                let reference = reference.trim();
                let refs = git_output(
                    &git_root,
                    &["for-each-ref", "--format=%(refname)", reference],
                )?;
                if String::from_utf8_lossy(&refs)
                    .lines()
                    .any(|value| value == reference)
                {
                    return Err(DiffError::Git {
                        path: root.display().to_string(),
                        message: "could not resolve HEAD".into(),
                    });
                }
                None
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            #[cfg(not(unix))]
            canonical_root,
            git_root,
            #[cfg(unix)]
            root_fd,
            prefix,
            head,
        })
    }
}

#[derive(PartialEq, Eq)]
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

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    #[cfg(unix)]
    if let Some(fd) = root
        .to_str()
        .and_then(|path| path.strip_prefix("/dev/fd/"))
        .and_then(|value| value.parse::<std::os::fd::RawFd>().ok())
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: fchdir is async-signal-safe, and DiffBase keeps the descriptor
        // alive until the child has spawned. No other state is accessed here.
        unsafe {
            command.pre_exec(move || {
                if nix::libc::fchdir(fd) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        return command;
    }
    command.current_dir(root);
    command
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, DiffError> {
    run_git(root, args, None, None, false)
}

#[cfg(unix)]
fn run_git(
    root: &Path,
    args: &[&str],
    object_dir: Option<&Path>,
    input: Option<&[u8]>,
    allow_difference: bool,
) -> Result<Vec<u8>, DiffError> {
    run_git_with_timeout(
        root,
        args,
        object_dir,
        input,
        allow_difference,
        std::time::Duration::from_secs(10),
    )
}

#[cfg(unix)]
fn run_git_with_timeout(
    root: &Path,
    args: &[&str],
    object_dir: Option<&Path>,
    input: Option<&[u8]>,
    allow_difference: bool,
    timeout: std::time::Duration,
) -> Result<Vec<u8>, DiffError> {
    use nix::fcntl::{fcntl, FcntlArg, OFlag};
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    use std::io::{ErrorKind, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + timeout;
    let io_error = |source| DiffError::Io {
        path: root.display().to_string(),
        source,
    };
    let mut command = git_command(root);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(dir) = object_dir {
        command.env("GIT_OBJECT_DIRECTORY", dir);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().map_err(io_error)?;
    let group = Pid::from_raw(child.id() as i32);
    let nonblocking = |fd| -> std::io::Result<()> {
        let flags =
            OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL).map_err(std::io::Error::from)?);
        fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).map_err(std::io::Error::from)?;
        Ok(())
    };
    let stdout = child.stdout.take().expect("piped git stdout");
    let stderr = child.stderr.take().expect("piped git stderr");
    if let Err(source) = nonblocking(stdout.as_raw_fd())
        .and_then(|()| nonblocking(stderr.as_raw_fd()))
        .and_then(|()| {
            if let Some(stdin) = &child.stdin {
                nonblocking(stdin.as_raw_fd())
            } else {
                Ok(())
            }
        })
    {
        let _ = killpg(group, Signal::SIGKILL);
        let _ = child.kill();
        let _ = child.wait();
        return Err(io_error(source));
    }
    let overflow = AtomicBool::new(false);
    let stdout_done = AtomicBool::new(false);
    let stderr_done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let read_pipe = |mut pipe: Box<dyn Read + Send>,
                         cap_is_error: bool|
         -> std::io::Result<Vec<u8>> {
            let mut output = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => return Ok(output),
                    Ok(count) => {
                        let remaining = (MAX_FILE_BYTES as usize + 1).saturating_sub(output.len());
                        output.extend_from_slice(&chunk[..count.min(remaining)]);
                        if cap_is_error && output.len() as u64 > MAX_FILE_BYTES {
                            overflow.store(true, Ordering::Relaxed);
                            return Ok(output);
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return Err(std::io::Error::new(
                                ErrorKind::TimedOut,
                                "git command timed out",
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
        };
        let stdout_done = &stdout_done;
        let stderr_done = &stderr_done;
        let out = scope.spawn(move || {
            let result = read_pipe(Box::new(stdout), true);
            stdout_done.store(true, Ordering::Release);
            result
        });
        let err = scope.spawn(move || {
            let result = read_pipe(Box::new(stderr), false);
            stderr_done.store(true, Ordering::Release);
            result
        });
        let writer = child.stdin.take().zip(input).map(|(mut stdin, input)| {
            scope.spawn(move || -> std::io::Result<()> {
                let mut offset = 0;
                while offset < input.len() {
                    match stdin.write(&input[offset..]) {
                        Ok(0) => {
                            return Err(std::io::Error::new(
                                ErrorKind::WriteZero,
                                "git stdin closed",
                            ))
                        }
                        Ok(count) => offset += count,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            if Instant::now() >= deadline {
                                return Err(std::io::Error::new(
                                    ErrorKind::TimedOut,
                                    "git command timed out",
                                ));
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error),
                    }
                }
                Ok(())
            })
        });
        let mut timed_out = false;
        let status = loop {
            if Instant::now() >= deadline || overflow.load(Ordering::Relaxed) {
                timed_out = !overflow.load(Ordering::Relaxed);
                // Keep the leader unreaped until the pipes close or the deadline
                // expires, so the process-group id cannot be reused here.
                let _ = killpg(group, Signal::SIGKILL);
                let _ = child.kill();
                break child.wait();
            }
            if stdout_done.load(Ordering::Acquire) && stderr_done.load(Ordering::Acquire) {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) => {}
                    Err(error) => {
                        let _ = killpg(group, Signal::SIGKILL);
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(error);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = out.join().expect("git stdout reader");
        let stderr = err.join().expect("git stderr reader");
        let written = writer.map(|writer| writer.join().expect("git stdin writer"));
        if overflow.load(Ordering::Relaxed) {
            return Err(DiffError::ReviewTooLarge);
        }
        if timed_out {
            return Err(DiffError::Git {
                path: root.display().to_string(),
                message: "git command timed out".into(),
            });
        }
        let stdout = stdout.map_err(io_error)?;
        let stderr = stderr.map_err(io_error)?;
        let status = status.map_err(io_error)?;
        if status.success() || (allow_difference && status.code() == Some(1)) {
            if let Some(written) = written {
                written.map_err(io_error)?;
            }
            Ok(stdout)
        } else {
            Err(DiffError::Git {
                path: root.display().to_string(),
                message: String::from_utf8_lossy(&stderr).trim().into(),
            })
        }
    })
}

#[cfg(not(unix))]
fn run_git(
    root: &Path,
    args: &[&str],
    object_dir: Option<&Path>,
    input: Option<&[u8]>,
    allow_difference: bool,
) -> Result<Vec<u8>, DiffError> {
    use std::io::Write;
    let mut command = git_command(root);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = object_dir {
        command.env("GIT_OBJECT_DIRECTORY", dir);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().map_err(|source| DiffError::Io {
        path: root.display().to_string(),
        source,
    })?;
    std::thread::scope(|scope| {
        let writer = child
            .stdin
            .take()
            .zip(input)
            .map(|(mut stdin, input)| scope.spawn(move || stdin.write_all(input)));
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
        let written = writer.map(|writer| writer.join().expect("git stdin writer"));
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
        if status.success() || (allow_difference && status.code() == Some(1)) {
            if let Some(written) = written {
                written.map_err(io_error)?;
            }
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
            layer: None,
            patch: None,
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
                    ending: Some(line_ending(change.value_ref())),
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
        layer: None,
        patch: None,
    }
}

fn line_ending(value: &str) -> LineEnding {
    if value.ends_with("\r\n") {
        LineEnding::CrLf
    } else if value.ends_with('\n') {
        LineEnding::Lf
    } else {
        LineEnding::None
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
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, line)| DiffLine {
            kind,
            old_line: deletion.then_some(index + 1),
            new_line: (!deletion).then_some(index + 1),
            text: trim_line_ending(line),
            ending: Some(line_ending(line)),
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
                    ..
                },
                DiffLine {
                    kind: LineKind::Deletion,
                    old_line: Some(2),
                    new_line: None,
                    text: deleted,
                    ..
                },
                DiffLine {
                    kind: LineKind::Addition,
                    old_line: None,
                    new_line: Some(2),
                    text: added,
                    ..
                },
                DiffLine {
                    kind: LineKind::Context,
                    old_line: Some(3),
                    new_line: Some(3),
                    text: closing,
                    ..
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
    fn loads_only_the_selected_file_and_retains_rename_information() {
        let directory = repository();
        git(directory.path(), &["mv", "tracked.txt", "renamed.txt"]);
        fs::write(
            directory.path().join("unrelated-large.txt"),
            vec![b'x'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let selected = load_session_file_diffs(directory.path(), "renamed.txt")
            .unwrap()
            .remove(0);
        assert_eq!(selected.status, FileStatus::Renamed);
        assert_eq!(selected.old_path.as_deref(), Some("tracked.txt"));
        assert!(load_session_file_diffs(directory.path(), "missing.txt")
            .unwrap()
            .is_empty());
        assert!(matches!(
            load_session_file_diffs(directory.path(), "../outside"),
            Err(DiffError::InvalidPath { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn selected_diff_git_reads_stay_with_the_opened_workspace() {
        let holder = tempfile::tempdir().unwrap();
        let original = holder.path().join("workspace");
        let replacement = holder.path().join("replacement");
        fs::create_dir(&original).unwrap();
        fs::create_dir(&replacement).unwrap();
        for (root, value) in [(&original, "inside\n"), (&replacement, "outside secret\n")] {
            git(root, &["init", "-q"]);
            git(root, &["config", "user.name", "test"]);
            git(root, &["config", "user.email", "test@example.invalid"]);
            fs::write(root.join("file.txt"), value).unwrap();
            git(root, &["add", "file.txt"]);
            git(root, &["commit", "-qm", "base"]);
        }
        let base = DiffBase::read(&original).unwrap();
        fs::rename(&original, holder.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(&replacement, &original).unwrap();
        let blob = git_blob(base.git_root(), &base, "file.txt").unwrap();
        assert!(matches!(blob, Some(GitBlob::File(bytes)) if bytes == b"inside\n"));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_root_rejects_a_replaced_ancestor() {
        let holder = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let ancestor = holder.path().join("owner");
        let root = ancestor.join("workspace");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(outside.path().join("workspace")).unwrap();
        fs::write(outside.path().join("workspace/file.txt"), "outside secret").unwrap();
        let root = root.canonicalize().unwrap();
        fs::rename(&ancestor, holder.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(outside.path(), &ancestor).unwrap();
        assert!(open_workspace_parent(&root, Path::new("file.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn selected_diff_rejects_a_socket() {
        let directory = repository();
        let path = directory.path().join("tracked.txt");
        fs::remove_file(&path).unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(load_session_file_diffs(directory.path(), "tracked.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn opened_parent_remains_scoped_when_directory_is_replaced() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let nested = workspace.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("file.txt"), "inside").unwrap();
        fs::write(outside.path().join("file.txt"), "outside secret").unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let (parent, name) = open_workspace_parent(&root, Path::new("nested/file.txt")).unwrap();
        fs::rename(&nested, workspace.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(outside.path(), &nested).unwrap();
        let result = read_worktree_leaf(&parent, &name, "nested/file.txt").unwrap();
        assert!(matches!(result, Some((GitBlob::File(bytes), _)) if bytes == b"inside"));
    }

    #[cfg(unix)]
    #[test]
    fn selected_diff_rejects_a_tracked_file_replaced_by_a_fifo() {
        let directory = repository();
        let path = directory.path().join("tracked.txt");
        fs::remove_file(&path).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(load_session_file_diffs(directory.path(), "tracked.txt"));
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("diff should not block on a FIFO");
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn selected_diff_reports_executable_bit_changes() {
        use std::os::unix::fs::PermissionsExt;
        let directory = repository();
        let path = directory.path().join("tracked.txt");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
        let diffs = load_session_file_diffs(directory.path(), "tracked.txt").unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].layer, Some(DiffLayer::Unstaged));
        assert!(diffs[0]
            .patch
            .as_deref()
            .unwrap()
            .contains("new mode 100755"));
    }

    #[cfg(unix)]
    #[test]
    fn repository_discovery_times_out_on_blocked_metadata() {
        let directory = repository();
        let head = directory.path().join(".git/HEAD");
        fs::remove_file(&head).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(&head)
            .status()
            .unwrap()
            .success());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(verify_git_repository(directory.path()));
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(12))
            .expect("repository discovery must have a deadline");
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn git_commands_time_out_and_stop_a_stalled_clean_filter() {
        let directory = repository();
        fs::write(
            directory.path().join(".gitattributes"),
            "tracked.txt filter=slow\n",
        )
        .unwrap();
        git(
            directory.path(),
            &["config", "filter.slow.clean", "sleep 8; cat"],
        );
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = run_git_with_timeout(
                directory.path(),
                &["hash-object", "--stdin", "--path=tracked.txt"],
                None,
                Some(b"data"),
                false,
                std::time::Duration::from_millis(500),
            );
            let _ = tx.send(result);
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(4))
            .expect("Git command must have a deadline");
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[cfg(unix)]
    #[test]
    fn git_deadline_stops_filter_children_holding_a_pipe_after_git_exits() {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        let directory = repository();
        let marker = tempfile::tempdir().unwrap();
        let pid_path = marker.path().join("child.pid");
        fs::write(
            directory.path().join(".gitattributes"),
            "tracked.txt filter=background\n",
        )
        .unwrap();
        git(
            directory.path(),
            &[
                "config",
                "filter.background.clean",
                &format!("sleep 8 >&2 & echo $! > '{}'; cat", pid_path.display()),
            ],
        );
        let result = run_git_with_timeout(
            directory.path(),
            &["hash-object", "--stdin", "--path=tracked.txt"],
            None,
            Some(b"data"),
            false,
            std::time::Duration::from_millis(500),
        );
        assert!(result.is_err());
        let pid: i32 = fs::read_to_string(pid_path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut alive = true;
        for _ in 0..100 {
            if kill(Pid::from_raw(pid), None).is_err() {
                alive = false;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if alive {
            let _ = kill(Pid::from_raw(pid), nix::sys::signal::Signal::SIGKILL);
        }
        assert!(!alive, "background filter child was not stopped");
    }

    #[test]
    fn selected_diff_respects_path_specific_binary_attributes() {
        let directory = repository();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(directory.path().join("src/file.txt"), "before\n").unwrap();
        fs::write(
            directory.path().join(".gitattributes"),
            "src/file.txt -diff\n",
        )
        .unwrap();
        git(directory.path(), &["add", ".gitattributes", "src/file.txt"]);
        git(directory.path(), &["commit", "-qm", "attributes"]);
        fs::write(directory.path().join("src/file.txt"), "changed\n").unwrap();
        let diffs = load_session_file_diffs(directory.path(), "src/file.txt").unwrap();
        let patch = diffs[0].patch.as_deref().unwrap();
        assert!(patch.contains("Binary files "), "{patch}");
        assert!(!patch.contains("+changed"));
    }

    #[test]
    fn selected_diff_respects_custom_driver_forcing_text() {
        let directory = repository();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(directory.path().join("src/data.dat"), b"before\0\n").unwrap();
        fs::write(
            directory.path().join(".gitattributes"),
            "src/data.dat diff=force\n",
        )
        .unwrap();
        git(directory.path(), &["config", "diff.force.binary", "false"]);
        git(directory.path(), &["add", ".gitattributes", "src/data.dat"]);
        git(directory.path(), &["commit", "-qm", "attributes"]);
        fs::write(directory.path().join("src/data.dat"), b"after\0\n").unwrap();
        let diffs = load_session_file_diffs(directory.path(), "src/data.dat").unwrap();
        let patch = diffs[0].patch.as_deref().unwrap();
        assert!(patch.contains("+after"), "{patch:?}");
        assert!(!patch.contains("Binary files "));
    }

    #[cfg(unix)]
    #[test]
    fn selected_diff_respects_git_filemode_setting() {
        use std::os::unix::fs::PermissionsExt;
        let directory = repository();
        git(directory.path(), &["config", "core.fileMode", "false"]);
        let path = directory.path().join("tracked.txt");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        fs::write(&path, "changed\n").unwrap();
        let diffs = load_session_file_diffs(directory.path(), "tracked.txt").unwrap();
        let patch = diffs[0].patch.as_deref().unwrap();
        assert!(patch.contains("+changed"));
        assert!(!patch.contains("new mode"), "{patch}");
    }

    #[test]
    fn selected_diff_respects_git_line_ending_conversion() {
        let directory = repository();
        git(directory.path(), &["config", "core.autocrlf", "true"]);
        let path = directory.path().join("tracked.txt");
        fs::write(&path, "staged\r\n").unwrap();
        git(directory.path(), &["add", "tracked.txt"]);
        let diffs = load_session_file_diffs(directory.path(), "tracked.txt").unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].layer, Some(DiffLayer::Staged));
    }

    #[test]
    fn selected_unstaged_patch_preserves_filename_for_syntax_detection() {
        let directory = repository();
        fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let diffs = load_session_file_diffs(directory.path(), "tracked.txt").unwrap();
        let patch = diffs[0].patch.as_deref().unwrap();
        assert!(patch.contains("/tracked.txt"), "{patch}");
    }

    #[test]
    fn selected_unstaged_diff_uses_git_clean_filters_without_writing_repo_objects() {
        let directory = repository();
        git(directory.path(), &["config", "core.autocrlf", "true"]);
        let path = directory.path().join("tracked.txt");
        fs::write(&path, "staged\r\n").unwrap();
        git(directory.path(), &["add", "tracked.txt"]);
        fs::write(&path, "unstaged\r\n").unwrap();
        let count_objects = || {
            let output = git(directory.path(), &["count-objects", "-v"]);
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned()
        };
        let before = count_objects();
        let diffs = load_session_file_diffs(directory.path(), "tracked.txt").unwrap();
        let unstaged = diffs
            .iter()
            .find(|diff| diff.layer == Some(DiffLayer::Unstaged))
            .unwrap();
        let patch = unstaged.patch.as_deref().unwrap();
        assert!(patch.contains("-staged\n"), "{patch:?}");
        assert!(patch.contains("+unstaged\n"), "{patch:?}");
        assert_eq!(count_objects(), before);
    }

    #[test]
    fn selected_diff_keeps_staged_and_unstaged_changes_separate() {
        let directory = repository();
        let path = directory.path().join("tracked.txt");
        let original = fs::read_to_string(&path).unwrap();
        fs::write(&path, "staged\n").unwrap();
        git(directory.path(), &["add", "tracked.txt"]);
        fs::write(&path, &original).unwrap();
        let diffs = load_session_file_diffs(directory.path(), "tracked.txt").unwrap();
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].layer, Some(DiffLayer::Staged));
        assert!(diffs[0].patch.as_deref().unwrap().contains("+staged"));
        assert_eq!(diffs[1].layer, Some(DiffLayer::Unstaged));
        assert!(diffs[1].patch.as_deref().unwrap().contains("-staged"));
    }

    #[test]
    fn selected_diff_handles_staged_renames_and_untracked_files() {
        let directory = repository();
        git(directory.path(), &["mv", "tracked.txt", "renamed.txt"]);
        let renamed = load_session_file_diffs(directory.path(), "renamed.txt").unwrap();
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].layer, Some(DiffLayer::Staged));
        assert_eq!(renamed[0].old_path.as_deref(), Some("tracked.txt"));
        assert!(renamed[0]
            .patch
            .as_deref()
            .unwrap()
            .contains("rename from tracked.txt"));
        fs::write(directory.path().join("[file].txt"), "new\n").unwrap();
        let added = load_session_file_diffs(directory.path(), "[file].txt").unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].layer, Some(DiffLayer::Unstaged));
        assert_eq!(added[0].status, FileStatus::Added);
    }

    #[test]
    fn selected_diff_reports_both_staged_addition_and_unstaged_deletion() {
        let directory = repository();
        let path = directory.path().join("added.txt");
        fs::write(&path, "staged\n").unwrap();
        git(directory.path(), &["add", "added.txt"]);
        fs::remove_file(path).unwrap();
        let diffs = load_session_file_diffs(directory.path(), "added.txt").unwrap();
        assert_eq!(diffs.len(), 2);
        assert_eq!(
            (diffs[0].layer, diffs[0].status),
            (Some(DiffLayer::Staged), FileStatus::Added)
        );
        assert_eq!(
            (diffs[1].layer, diffs[1].status),
            (Some(DiffLayer::Unstaged), FileStatus::Deleted)
        );
    }

    #[test]
    fn diff_lines_preserve_eof_and_crlf_information() {
        let diff = diff_file("file.txt", None, Some("hello\r\n"), Some("hello"));
        let lines: Vec<_> = diff.lines().collect();
        assert_eq!(lines[0].ending, Some(LineEnding::CrLf));
        assert_eq!(lines[1].ending, Some(LineEnding::None));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn selected_diff_ignores_unrelated_non_utf8_paths() {
        use std::os::unix::ffi::OsStrExt;
        let directory = repository();
        fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let invalid = std::ffi::OsStr::from_bytes(b"unrelated-\xff");
        fs::write(directory.path().join(invalid), "data").unwrap();
        assert_eq!(
            load_session_file_diffs(directory.path(), "tracked.txt")
                .unwrap()
                .remove(0)
                .status,
            FileStatus::Modified
        );
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
        let selected = load_session_file_diffs(directory.path(), "child").unwrap();
        assert_eq!(selected.len(), 1);
        assert!(selected[0]
            .patch
            .as_deref()
            .unwrap()
            .contains("Subproject commit"));
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
