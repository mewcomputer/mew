//! Per-session snapshot store for hashline tags.

use crate::format::compute_file_hash;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub trait SnapshotStore: Send + Sync {
    /// Record a full-file version and return its content hash tag.
    fn record(&self, path: &str, text: &str, seen_lines: Option<&[usize]>);

    /// Look up the recorded version for `path` whose tag equals `hash`.
    fn by_hash(&self, path: &str, hash: &str) -> Option<Snapshot>;

    /// The most recently recorded version for `path`.
    fn head(&self, path: &str) -> Option<Snapshot>;

    /// Every retained version (across paths) whose tag equals `hash`.
    fn find_by_hash(&self, hash: &str) -> Vec<Snapshot>;

    /// Drop the version history for a single path.
    fn invalidate(&self, path: &str);

    /// Move retained version history from `from` to `to`.
    fn relocate(&self, from: &str, to: &str);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub path: String,
    pub text: String,
    pub hash: String,
    /// 1-indexed lines the producer actually displayed under this tag.
    pub seen_lines: Option<Vec<usize>>,
}

/// In-memory snapshot store with bounded per-path history.
pub struct InMemorySnapshotStore {
    histories: Mutex<HashMap<String, Vec<InternalSnapshot>>>,
    max_versions_per_path: usize,
}

#[derive(Debug, Clone)]
struct InternalSnapshot {
    text: String,
    hash: String,
    seen_lines: Option<HashSet<usize>>,
}

impl InMemorySnapshotStore {
    pub fn new() -> Self {
        Self {
            histories: Mutex::new(HashMap::new()),
            max_versions_per_path: 4,
        }
    }

    pub fn with_max_versions_per_path(mut self, n: usize) -> Self {
        self.max_versions_per_path = n.max(1);
        self
    }
}

impl Default for InMemorySnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotStore for InMemorySnapshotStore {
    fn record(&self, path: &str, text: &str, seen_lines: Option<&[usize]>) {
        let hash = compute_file_hash(text);
        let mut histories = self.histories.lock().unwrap();
        let history = histories.entry(path.to_string()).or_default();

        if let Some(existing) = history.iter_mut().find(|v| v.hash == hash) {
            merge_seen_lines(existing, seen_lines);
            return;
        }

        let mut snap = InternalSnapshot {
            text: text.to_string(),
            hash,
            seen_lines: None,
        };
        merge_seen_lines(&mut snap, seen_lines);
        history.insert(0, snap);
        history.truncate(self.max_versions_per_path);
    }

    fn by_hash(&self, path: &str, hash: &str) -> Option<Snapshot> {
        let histories = self.histories.lock().unwrap();
        histories
            .get(path)
            .and_then(|h| h.iter().find(|v| v.hash == hash))
            .map(|v| to_snapshot(path, v))
    }

    fn head(&self, path: &str) -> Option<Snapshot> {
        let histories = self.histories.lock().unwrap();
        histories
            .get(path)
            .and_then(|h| h.first())
            .map(|v| to_snapshot(path, v))
    }

    fn find_by_hash(&self, hash: &str) -> Vec<Snapshot> {
        let histories = self.histories.lock().unwrap();
        let mut out = Vec::new();
        for (path, history) in histories.iter() {
            for v in history.iter().filter(|v| v.hash == hash) {
                out.push(to_snapshot(path, v));
            }
        }
        out
    }

    fn invalidate(&self, path: &str) {
        let mut histories = self.histories.lock().unwrap();
        histories.remove(path);
    }

    fn relocate(&self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let mut histories = self.histories.lock().unwrap();
        let Some(mut history) = histories.remove(from) else {
            return;
        };
        for v in &mut history {
            v.text = v.text.clone();
        }
        let dest = histories.entry(to.to_string()).or_default();
        dest.extend(history);
        dest.sort_by(|a, b| b.hash.cmp(&a.hash));
        dest.dedup_by(|a, b| a.hash == b.hash);
        dest.truncate(self.max_versions_per_path);
    }
}

fn merge_seen_lines(snap: &mut InternalSnapshot, seen_lines: Option<&[usize]>) {
    let Some(lines) = seen_lines else { return };
    if snap.seen_lines.is_none() {
        snap.seen_lines = Some(HashSet::new());
    }
    if let Some(set) = snap.seen_lines.as_mut() {
        for &line in lines {
            set.insert(line);
        }
    }
}

fn to_snapshot(path: &str, v: &InternalSnapshot) -> Snapshot {
    Snapshot {
        path: path.to_string(),
        text: v.text.clone(),
        hash: v.hash.clone(),
        seen_lines: v.seen_lines.as_ref().map(|set| {
            let mut lines: Vec<usize> = set.iter().copied().collect();
            lines.sort_unstable();
            lines
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_head() {
        let store = InMemorySnapshotStore::new();
        store.record("a.rs", "hello\nworld\n", None);
        let head = store.head("a.rs").unwrap();
        assert_eq!(head.text, "hello\nworld\n");
        assert_eq!(head.hash, compute_file_hash("hello\nworld\n"));
    }

    #[test]
    fn seen_lines_merge() {
        let store = InMemorySnapshotStore::new();
        store.record("a.rs", "1\n2\n3\n", Some(&[1, 2]));
        store.record("a.rs", "1\n2\n3\n", Some(&[2, 3]));
        let snap = store.head("a.rs").unwrap();
        assert_eq!(snap.seen_lines, Some(vec![1, 2, 3]));
    }

    #[test]
    fn history_bound() {
        let store = InMemorySnapshotStore::new().with_max_versions_per_path(2);
        store.record("a.rs", "v1\n", None);
        store.record("a.rs", "v2\n", None);
        store.record("a.rs", "v3\n", None);
        assert!(store.by_hash("a.rs", &compute_file_hash("v1\n")).is_none());
        assert!(store.by_hash("a.rs", &compute_file_hash("v3\n")).is_some());
    }

    #[test]
    fn relocate() {
        let store = InMemorySnapshotStore::new();
        store.record("a.rs", "text\n", None);
        store.relocate("a.rs", "b.rs");
        assert!(store.head("a.rs").is_none());
        assert!(store.head("b.rs").is_some());
    }
}
