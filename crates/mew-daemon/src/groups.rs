//! Sidecar store for session groups. Lives at `<session_dir>/groups.json`.
//! Loaded at daemon start, written atomically on every mutation.
//!
//! Groups are daemon-side (multi-client), not per-session. They are NOT
//! stuffed into `Meta` — a session's `group_id` on `Meta` is the only link.

use std::path::{Path, PathBuf};

use mew_protocol::GroupInfo;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::debug;

/// One session group definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub order: u32,
}

/// The full groups sidecar state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupsState {
    #[serde(default)]
    pub groups: Vec<Group>,
    /// session_id → group_id
    #[serde(default)]
    pub membership: std::collections::HashMap<String, String>,
}

impl GroupsState {
    fn path(session_dir: &Path) -> PathBuf {
        session_dir.join("groups.json")
    }

    /// Load from disk. Returns empty state if the file doesn't exist.
    pub async fn load(session_dir: &Path) -> Self {
        let path = Self::path(session_dir);
        match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice::<GroupsState>(&bytes) {
                Ok(state) => state,
                Err(e) => {
                    tracing::warn!(%e, ?path, "failed to parse groups.json; starting fresh");
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!(%e, ?path, "failed to read groups.json; starting fresh");
                Self::default()
            }
        }
    }

    /// Write atomically (temp file + rename).
    async fn write(&self, session_dir: &Path) -> std::io::Result<()> {
        let path = Self::path(session_dir);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::other(format!("serialize groups: {e}")))?;
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        debug!(?path, "wrote groups.json");
        Ok(())
    }
}

/// Thread-safe wrapper around `GroupsState`.
pub struct GroupsStore {
    state: Mutex<GroupsState>,
    session_dir: PathBuf,
}

impl GroupsStore {
    pub fn new(session_dir: PathBuf) -> Self {
        Self {
            state: Mutex::new(GroupsState::default()),
            session_dir,
        }
    }

    /// Construct with a pre-loaded state (synchronous init path).
    pub fn from_state(state: GroupsState, session_dir: PathBuf) -> Self {
        Self {
            state: Mutex::new(state),
            session_dir,
        }
    }

    /// Return a snapshot of all groups as `GroupInfo` (sorted by order).
    pub async fn list(&self) -> Vec<GroupInfo> {
        let state = self.state.lock().await;
        let mut groups: Vec<GroupInfo> = state
            .groups
            .iter()
            .map(|g| GroupInfo {
                id: g.id.clone(),
                name: g.name.clone(),
                color: g.color.clone(),
                order: g.order,
            })
            .collect();
        groups.sort_by_key(|g| g.order);
        groups
    }

    /// Return the group_id for a session, if any.
    pub async fn group_for_session(&self, session_id: &str) -> Option<String> {
        self.state.lock().await.membership.get(session_id).cloned()
    }

    pub async fn contains(&self, group_id: &str) -> bool {
        self.state
            .lock()
            .await
            .groups
            .iter()
            .any(|group| group.id == group_id)
    }

    /// Create a new group. Returns the group list.
    pub async fn create_group(
        &self,
        name: String,
        color: Option<String>,
    ) -> std::io::Result<Vec<GroupInfo>> {
        let mut state = self.state.lock().await;
        let id = format!("grp_{}", ulid::Ulid::new());
        let order = state.groups.len() as u32;
        state.groups.push(Group {
            id: id.clone(),
            name,
            color,
            order,
        });
        state.write(&self.session_dir).await?;
        drop(state);
        Ok(self.list().await)
    }

    /// Update an existing group. Returns the group list.
    pub async fn update_group(
        &self,
        group_id: &str,
        name: Option<String>,
        color: Option<Option<String>>,
        order: Option<u32>,
    ) -> std::io::Result<Vec<GroupInfo>> {
        let mut state = self.state.lock().await;
        if let Some(g) = state.groups.iter_mut().find(|g| g.id == group_id) {
            if let Some(n) = name {
                g.name = n;
            }
            if let Some(c) = color {
                g.color = c;
            }
            if let Some(o) = order {
                g.order = o;
            }
        }
        state.write(&self.session_dir).await?;
        drop(state);
        Ok(self.list().await)
    }

    /// Delete a group. Members survive, ungrouped.
    pub async fn delete_group(&self, group_id: &str) -> std::io::Result<Vec<GroupInfo>> {
        let mut state = self.state.lock().await;
        state.groups.retain(|g| g.id != group_id);
        // Remove all memberships pointing to this group.
        state.membership.retain(|_, gid| gid != group_id);
        state.write(&self.session_dir).await?;
        drop(state);
        Ok(self.list().await)
    }

    /// Assign (or unassign) a session to a group.
    pub async fn assign_session(
        &self,
        session_id: &str,
        group_id: Option<String>,
    ) -> std::io::Result<Vec<GroupInfo>> {
        let mut state = self.state.lock().await;
        match &group_id {
            Some(gid) => {
                state.membership.insert(session_id.to_string(), gid.clone());
            }
            None => {
                state.membership.remove(session_id);
            }
        }
        state.write(&self.session_dir).await?;
        drop(state);
        Ok(self.list().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn deleting_a_group_ungroups_members_and_persists() {
        let directory = tempfile::tempdir().unwrap();
        let store = GroupsStore::from_state(
            GroupsState {
                groups: vec![Group {
                    id: "grp_1".into(),
                    name: "Project".into(),
                    color: None,
                    order: 0,
                }],
                membership: HashMap::from([
                    ("session-1".into(), "grp_1".into()),
                    ("session-2".into(), "grp_1".into()),
                ]),
            },
            directory.path().to_owned(),
        );

        let groups = store.delete_group("grp_1").await.unwrap();

        assert!(groups.is_empty());
        assert_eq!(store.group_for_session("session-1").await, None);
        assert_eq!(store.group_for_session("session-2").await, None);

        let persisted = GroupsState::load(directory.path()).await;
        assert!(persisted.groups.is_empty());
        assert!(persisted.membership.is_empty());
    }
}
