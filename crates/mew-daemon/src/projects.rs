//! Daemon-owned workspace preferences.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProjectsState {
    #[serde(default)]
    pinned_paths: BTreeSet<String>,
}

/// Persisted workspace pin state. The sidecar is deliberately separate from
/// desktop layout state so all connected clients observe the same ordering.
pub struct ProjectPinsStore {
    state: Mutex<ProjectsState>,
    session_dir: PathBuf,
}

impl ProjectPinsStore {
    pub(crate) fn from_session_dir(session_dir: PathBuf) -> Self {
        let path = session_dir.join("projects.json");
        let state = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            state: Mutex::new(state),
            session_dir,
        }
    }

    pub(crate) async fn is_pinned(&self, path: &str) -> bool {
        self.state.lock().await.pinned_paths.contains(path)
    }

    pub(crate) async fn set_pinned(&self, path: String, pinned: bool) -> std::io::Result<()> {
        let mut state = self.state.lock().await;
        if pinned {
            state.pinned_paths.insert(path);
        } else {
            state.pinned_paths.remove(&path);
        }
        let target = self.session_dir.join("projects.json");
        let temp = self.session_dir.join("projects.json.tmp");
        let bytes = serde_json::to_vec_pretty(&*state)
            .map_err(|error| std::io::Error::other(format!("serialize projects: {error}")))?;
        tokio::fs::write(&temp, bytes).await?;
        tokio::fs::rename(&temp, &target).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pins_round_trip_through_atomic_sidecar() {
        let directory = tempfile::tempdir().unwrap();
        let store = ProjectPinsStore::from_session_dir(directory.path().to_path_buf());
        store.set_pinned("/work/app".into(), true).await.unwrap();
        assert!(store.is_pinned("/work/app").await);

        let reloaded = ProjectPinsStore::from_session_dir(directory.path().to_path_buf());
        assert!(reloaded.is_pinned("/work/app").await);
        reloaded
            .set_pinned("/work/app".into(), false)
            .await
            .unwrap();
        assert!(!reloaded.is_pinned("/work/app").await);
    }
}
