//! On-device daemon registry. JSON sidecar, never synced.
//!
//! Each entry is a daemon the user has paired with. Keyed by the daemon's
//! NodeId (which is now stable across restarts thanks to the persistent
//! secret key).

use std::path::PathBuf;

/// Opaque ID for a daemon in the registry. This is the daemon's NodeId string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct DaemonId {
    pub node_id: String,
}

/// A daemon entry in the registry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct DaemonEntry {
    /// The daemon's iroh NodeId (public key string).
    pub node_id: String,
    /// User-assigned name.
    pub name: String,
    /// When the daemon was added (unix timestamp ms).
    pub added_at: u64,
    /// Last successful connection time (unix timestamp ms).
    pub last_connected_at: Option<u64>,
    /// Daemon version from last Pong.
    pub last_known_version: Option<String>,
    /// Whether to keep the connection alive while foregrounded.
    pub keep_connected: bool,
}

/// The registry store. JSON file, loaded at startup, saved on change.
pub struct DaemonRegistry {
    entries: Vec<DaemonEntry>,
    path: PathBuf,
}

impl DaemonRegistry {
    /// Load from disk, or create empty if the file doesn't exist.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let entries = if path.exists() {
            let bytes = std::fs::read(&path)?;
            match serde_json::from_slice::<Vec<DaemonEntry>>(&bytes) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "corrupted daemon registry, starting empty");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        Ok(Self { entries, path })
    }

    /// Add a daemon. Returns the assigned ID.
    pub fn add(&mut self, node_id: String, name: String) -> DaemonId {
        // Check if already exists by node_id.
        if let Some(existing) = self.entries.iter().find(|e| e.node_id == node_id) {
            return DaemonId { node_id: existing.node_id.clone() };
        }

        let entry = DaemonEntry {
            node_id: node_id.clone(),
            name,
            added_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            last_connected_at: None,
            last_known_version: None,
            keep_connected: false,
        };
        self.entries.push(entry);
        let _ = self.save();
        DaemonId { node_id }
    }

    /// Remove a daemon by ID.
    pub fn remove(&mut self, id: &DaemonId) {
        self.entries.retain(|e| e.node_id != id.node_id);
        let _ = self.save();
    }

    /// Get a daemon entry by ID.
    pub fn get(&self, id: &DaemonId) -> Option<&DaemonEntry> {
        self.entries.iter().find(|e| e.node_id == id.node_id)
    }

    /// Get a mutable daemon entry by ID.
    pub fn get_mut(&mut self, id: &DaemonId) -> Option<&mut DaemonEntry> {
        self.entries.iter_mut().find(|e| e.node_id == id.node_id)
    }

    /// List all daemons.
    pub fn list(&self) -> Vec<DaemonEntry> {
        self.entries.clone()
    }

    /// Update last connected time and version.
    pub fn touch(&mut self, id: &DaemonId, version: Option<String>) {
        if let Some(entry) = self.get_mut(id) {
            entry.last_connected_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            );
            if let Some(v) = version {
                entry.last_known_version = Some(v);
            }
        }
        let _ = self.save();
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let json = serde_json::to_string_pretty(&self.entries)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_add_get_remove() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut reg = DaemonRegistry::load(tmp.path().to_path_buf()).unwrap();

        let id = reg.add("node123".into(), "Homelab".into());
        assert_eq!(reg.list().len(), 1);

        let entry = reg.get(&id).unwrap();
        assert_eq!(entry.name, "Homelab");
        assert_eq!(entry.node_id, "node123");

        // Adding same node_id returns same ID.
        let id2 = reg.add("node123".into(), "Other".into());
        assert_eq!(id.node_id, id2.node_id);
        assert_eq!(reg.list().len(), 1);

        reg.remove(&id);
        assert_eq!(reg.list().len(), 0);
    }

    #[test]
    fn test_registry_persistence() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        {
            let mut reg = DaemonRegistry::load(path.clone()).unwrap();
            reg.add("node1".into(), "Daemon A".into());
            reg.add("node2".into(), "Daemon B".into());
        }

        let reg = DaemonRegistry::load(path).unwrap();
        assert_eq!(reg.list().len(), 2);
        assert!(reg.get(&DaemonId { node_id: "node1".into() }).is_some());
        assert!(reg.get(&DaemonId { node_id: "node2".into() }).is_some());
        assert!(reg.get(&DaemonId { node_id: "node3".into() }).is_none());
    }
}
