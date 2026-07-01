use crate::apply::apply_edits;
use crate::block::resolve_block_edits;
use crate::format::{
    compute_file_hash, detect_line_ending, normalize_to_lf, restore_line_endings, strip_bom,
    LineEnding,
};
use crate::fs::HashlineFs;
use crate::patch::{Patch, PatchSection};
use crate::recovery::try_recover;
use crate::snapshot::SnapshotStore;
use crate::types::{ApplyResult, BlockResolution, BlockResolver, Cursor, Edit, FileOp};

pub struct PatcherOptions {
    pub snapshots: std::sync::Arc<dyn SnapshotStore>,
    pub block_resolver: Option<Box<BlockResolver>>,
}

pub struct Patcher {
    snapshots: std::sync::Arc<dyn SnapshotStore>,
    block_resolver: Option<Box<BlockResolver>>,
}

pub struct PatchSectionResult {
    pub path: String,
    pub canonical_path: String,
    pub op: PatchOp,
    pub before: String,
    pub after: String,
    pub file_hash: String,
    pub first_changed_line: Option<usize>,
    pub warnings: Vec<String>,
    pub block_resolutions: Vec<BlockResolution>,
    pub move_dest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOp {
    Create,
    Update,
    Delete,
    Noop,
}

pub struct PreparedSection {
    section: PatchSection,
    canonical_path: String,
    exists: bool,
    bom: String,
    line_ending: LineEnding,
    normalized: String,
    apply_result: ApplyResult,
    file_op: Option<FileOp>,
    warnings: Vec<String>,
    move_dest: Option<String>,
}

impl Patcher {
    pub fn new(options: PatcherOptions) -> Self {
        Self {
            snapshots: options.snapshots,
            block_resolver: options.block_resolver,
        }
    }

    /// Parse `patch_text`, preflight every section in memory, then commit all
    /// writes. A failure during preparation prevents any write.
    pub async fn apply(
        &self,
        patch_text: &str,
        fs: &dyn HashlineFs,
    ) -> crate::Result<Vec<PatchSectionResult>> {
        let patch = Patch::parse(patch_text)?;

        let mut prepared = Vec::with_capacity(patch.sections.len());
        for section in patch.sections {
            prepared.push(self.prepare(section, fs).await?);
        }

        // Reject no-op patches so the model does not think it changed something
        // when it didn't.
        for p in &prepared {
            if p.is_noop() {
                return Err(crate::HashlineError::execution(format!(
                    "edits to {} resulted in no changes",
                    p.section.path
                )));
            }
        }

        let mut results = Vec::with_capacity(prepared.len());
        for p in prepared {
            results.push(self.commit(p, fs).await?);
        }
        Ok(results)
    }

    /// Preflight only: parse and apply in memory without writing anything.
    pub async fn preflight(
        &self,
        patch_text: &str,
        fs: &dyn HashlineFs,
    ) -> crate::Result<Vec<PreparedSection>> {
        let patch = Patch::parse(patch_text)?;
        let mut prepared = Vec::with_capacity(patch.sections.len());
        for section in patch.sections {
            prepared.push(self.prepare(section, fs).await?);
        }
        for p in &prepared {
            if p.is_noop() {
                return Err(crate::HashlineError::execution(format!(
                    "edits to {} resulted in no changes",
                    p.section.path
                )));
            }
        }
        Ok(prepared)
    }

    async fn prepare(
        &self,
        section: PatchSection,
        fs: &dyn HashlineFs,
    ) -> crate::Result<PreparedSection> {
        let canonical_path = fs.canonical_path(&section.path);

        if section.hash.is_none() {
            return Err(crate::HashlineError::parse(
                0,
                format!(
                    "section for {} is missing the required [path#hash] content hash",
                    section.path
                ),
            ));
        }
        let expected_hash = section.hash.clone().unwrap();

        // Read the file, with optional path recovery if the authored path is
        // missing but the filename + tag match a file read this session.
        let (target_path, read) = match self.try_read(&section.path, fs).await {
            Ok(read) => (section.path.clone(), read),
            Err(_) => {
                if let Some(recovered) = self.recover_path(&section, &canonical_path) {
                    let target = recovered.clone();
                    (target, self.try_read(&recovered, fs).await?)
                } else {
                    return Err(crate::HashlineError::execution(format!(
                        "file not found: {}. Use the write tool to create new files.",
                        section.path
                    )));
                }
            }
        };

        let (bom, normalized) = normalize_content(&read.content);
        let line_ending = detect_line_ending(&read.content);
        let current_hash = compute_file_hash(&normalized);

        // Validate hash and recover if needed.
        let live_matches = current_hash == expected_hash;
        let edits = if live_matches {
            section.edits.clone()
        } else {
            let snapshot = self.snapshots.by_hash(&target_path, &expected_hash);
            let previous = snapshot.map(|s| s.text);
            match previous {
                Some(previous) => {
                    let recovered = try_recover(&normalized, &previous, &section.edits);
                    match recovered {
                        Some(r) => {
                            // Build a single synthetic "replace whole file" edit
                            // so the commit path writes the recovered text.
                            vec![Edit::Insert {
                                cursor: Cursor::Bof,
                                text: r.text,
                                mode: crate::types::InsertMode::Normal,
                                block_start: None,
                            }]
                        }
                        None => {
                            return Err(crate::HashlineError::HashMismatch {
                                path: section.path,
                                expected: expected_hash,
                                actual: current_hash,
                            });
                        }
                    }
                }
                None => {
                    return Err(crate::HashlineError::HashMismatch {
                        path: section.path,
                        expected: expected_hash,
                        actual: current_hash,
                    });
                }
            }
        };

        // Seen-lines guard: when the snapshot records which lines were actually
        // displayed, reject edits anchored on unseen lines.
        if live_matches {
            if let Some(snapshot) = self.snapshots.by_hash(&target_path, &expected_hash) {
                if let Some(seen) = snapshot.seen_lines {
                    for line in section.collect_anchor_lines() {
                        if !seen.contains(&line) {
                            return Err(crate::HashlineError::execution(format!(
                                "line {line} in {} was not shown in the read that minted the tag",
                                section.path
                            )));
                        }
                    }
                }
            }
        }

        // Resolve block edits before applying.
        let mut block_resolutions = Vec::new();
        let resolved = resolve_block_edits(
            &edits,
            &normalized,
            &target_path,
            self.block_resolver.as_deref(),
            &mut block_resolutions,
        )?;

        let apply_result = apply_edits(&normalized, &resolved)?;

        let file_op = section.file_op.clone();
        let move_dest = match &file_op {
            Some(FileOp::Move { dest }) => Some(dest.clone()),
            _ => None,
        };

        let mut warnings = section.warnings.clone();
        warnings.extend(apply_result.warnings.clone());

        Ok(PreparedSection {
            section,
            canonical_path,
            exists: read.exists,
            bom,
            line_ending,
            normalized,
            apply_result,
            file_op,
            warnings,
            move_dest,
        })
    }

    async fn commit(
        &self,
        prepared: PreparedSection,
        fs: &dyn HashlineFs,
    ) -> crate::Result<PatchSectionResult> {
        let PreparedSection {
            section,
            canonical_path,
            exists,
            bom,
            line_ending,
            normalized,
            apply_result,
            file_op,
            warnings,
            move_dest,
        } = prepared;

        if let Some(FileOp::Rem) = file_op {
            fs.delete(&section.path).await.map_err(|e| {
                crate::HashlineError::execution(format!("delete failed for {}: {e}", section.path))
            })?;
            self.snapshots.invalidate(&canonical_path);
            let hash = compute_file_hash(&normalized);
            return Ok(PatchSectionResult {
                path: section.path.clone(),
                canonical_path,
                op: PatchOp::Delete,
                before: normalized.clone(),
                after: normalized,
                file_hash: hash,
                first_changed_line: None,
                warnings,
                block_resolutions: Vec::new(),
                move_dest: None,
            });
        }

        let after = apply_result.text;
        if after == normalized && move_dest.is_none() {
            let hash = compute_file_hash(&normalized);
            self.snapshots.record(&canonical_path, &normalized, None);
            return Ok(PatchSectionResult {
                path: section.path.clone(),
                canonical_path,
                op: PatchOp::Noop,
                before: normalized.clone(),
                after,
                file_hash: hash,
                first_changed_line: None,
                warnings,
                block_resolutions: Vec::new(),
                move_dest: None,
            });
        }

        let persisted = bom + &restore_line_endings(&after, line_ending);

        if let Some(dest) = &move_dest {
            let dest_canonical = fs.canonical_path(dest);
            if dest_canonical == canonical_path {
                return Err(crate::HashlineError::execution(
                    "move destination is the same as the source",
                ));
            }
            fs.write_text(dest, &persisted).await.map_err(|e| {
                crate::HashlineError::execution(format!("write failed for {dest}: {e}"))
            })?;
            fs.delete(&section.path).await.map_err(|e| {
                crate::HashlineError::execution(format!("delete failed for {}: {e}", section.path))
            })?;
            self.snapshots.relocate(&canonical_path, &dest_canonical);
            self.snapshots.record(&dest_canonical, &after, None);
            let file_hash = compute_file_hash(&after);
            return Ok(PatchSectionResult {
                path: section.path,
                canonical_path: dest_canonical,
                op: PatchOp::Update,
                before: normalized.clone(),
                after,
                file_hash,
                first_changed_line: apply_result.first_changed_line,
                warnings,
                block_resolutions: apply_result.block_resolutions,
                move_dest,
            });
        }

        fs.write_text(&section.path, &persisted)
            .await
            .map_err(|e| {
                crate::HashlineError::execution(format!("write failed for {}: {e}", section.path))
            })?;
        self.snapshots.record(&canonical_path, &after, None);
        let file_hash = compute_file_hash(&after);

        Ok(PatchSectionResult {
            path: section.path,
            canonical_path,
            op: if exists {
                PatchOp::Update
            } else {
                PatchOp::Create
            },
            before: normalized.clone(),
            after,
            file_hash,
            first_changed_line: apply_result.first_changed_line,
            warnings,
            block_resolutions: apply_result.block_resolutions,
            move_dest: None,
        })
    }

    async fn try_read(&self, path: &str, fs: &dyn HashlineFs) -> crate::Result<ReadResult> {
        match fs.read_text(path).await {
            Ok(content) => Ok(ReadResult {
                content,
                exists: true,
            }),
            Err(e) if std::io::ErrorKind::NotFound == e.kind() => Err(
                crate::HashlineError::execution(format!("file not found: {path}")),
            ),
            Err(e) => Err(crate::HashlineError::execution(format!(
                "read failed for {path}: {e}"
            ))),
        }
    }

    fn recover_path(&self, section: &PatchSection, original_canonical: &str) -> Option<String> {
        let hash = section.hash.as_ref()?;
        let authored_name = std::path::Path::new(&section.path)
            .file_name()
            .and_then(|n| n.to_str())?
            .to_string();
        let candidates: Vec<String> = self
            .snapshots
            .find_by_hash(hash)
            .into_iter()
            .filter(|s| {
                std::path::Path::new(&s.path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    == Some(&authored_name)
                    && s.path != section.path
                    && s.path != original_canonical
            })
            .map(|s| s.path)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        if candidates.len() == 1 {
            Some(candidates.into_iter().next().unwrap())
        } else {
            None
        }
    }
}

struct ReadResult {
    content: String,
    exists: bool,
}

impl PreparedSection {
    fn is_noop(&self) -> bool {
        self.file_op.is_none() && self.apply_result.text == self.normalized
    }
}

fn normalize_content(content: &str) -> (String, String) {
    let (text, had_bom) = strip_bom(content);
    (
        if had_bom {
            "\u{FEFF}".to_string()
        } else {
            String::new()
        },
        normalize_to_lf(text),
    )
}
