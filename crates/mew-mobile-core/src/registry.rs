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
    /// Pairing credential supplied by an invite, when present.
    #[serde(default)]
    pub pairing_token: Option<String>,
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
            return DaemonId {
                node_id: existing.node_id.clone(),
            };
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
            pairing_token: None,
        };
        self.entries.push(entry);
        let _ = self.save();
        DaemonId { node_id }
    }

    /// Add a daemon from an invite that includes a pairing credential.
    pub fn add_with_token(&mut self, node_id: String, name: String, token: String) -> DaemonId {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.node_id == node_id) {
            existing.name = name;
            existing.pairing_token = Some(token);
            let _ = self.save();
            return DaemonId { node_id };
        }
        let id = self.add(node_id, name);
        if let Some(existing) = self.entries.iter_mut().find(|e| e.node_id == id.node_id) {
            existing.pairing_token = Some(token);
            let _ = self.save();
        }
        id
    }

    /// Remove a daemon by ID.
    pub fn remove(&mut self, id: &DaemonId) {
        self.entries.retain(|e| e.node_id != id.node_id);
        let _ = self.save();
    }

    /// Update a saved nickname without changing the daemon identity or credentials.
    pub fn rename(&mut self, id: &DaemonId, name: String) -> anyhow::Result<()> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty(), "daemon nickname cannot be empty");
        let index = self
            .entries
            .iter()
            .position(|entry| entry.node_id == id.node_id)
            .ok_or_else(|| anyhow::anyhow!("saved daemon was not found"))?;
        let previous = std::mem::replace(&mut self.entries[index].name, name.to_owned());
        if let Err(error) = self.save() {
            self.entries[index].name = previous;
            return Err(error.context("could not save daemon nickname"));
        }
        Ok(())
    }

    /// Drop a pairing credential the daemon has already consumed.
    ///
    /// Invite tokens are single use, so once a handshake succeeds the stored
    /// value can only replay a credential the daemon no longer knows about.
    /// Reconnects then authorize through the daemon's paired-device record.
    pub fn clear_pairing_token(&mut self, id: &DaemonId) -> anyhow::Result<()> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.node_id == id.node_id)
            .ok_or_else(|| anyhow::anyhow!("saved daemon was not found"))?;
        let previous = self.entries[index].pairing_token.take();
        if let Err(error) = self.save() {
            self.entries[index].pairing_token = previous;
            return Err(error.context("could not save daemon registry"));
        }
        Ok(())
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
    fn renaming_a_daemon_persists_only_its_local_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        let mut registry = DaemonRegistry::load(path.clone()).unwrap();
        let id = registry.add_with_token("node1".into(), "Original".into(), "credential".into());
        registry.add("node2".into(), "Other".into());
        let entry = registry.get_mut(&id).unwrap();
        entry.last_connected_at = Some(123);
        entry.last_known_version = Some("1.2.3".into());
        entry.keep_connected = true;
        let added_at = entry.added_at;

        registry
            .rename(&id, "  Montréal workstation  ".into())
            .unwrap();

        let saved = DaemonRegistry::load(path).unwrap();
        let entry = saved.get(&id).unwrap();
        assert_eq!(entry.name, "Montréal workstation");
        assert_eq!(entry.node_id, "node1");
        assert_eq!(entry.added_at, added_at);
        assert_eq!(entry.last_connected_at, Some(123));
        assert_eq!(entry.last_known_version.as_deref(), Some("1.2.3"));
        assert!(entry.keep_connected);
        assert_eq!(entry.pairing_token.as_deref(), Some("credential"));
        assert_eq!(saved.list().len(), 2);
        assert_eq!(saved.list()[1].name, "Other");
    }

    #[test]
    fn invalid_daemon_renames_leave_the_registry_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        let mut registry = DaemonRegistry::load(path.clone()).unwrap();
        let id = registry.add("node1".into(), "Original".into());
        let before = std::fs::read(&path).unwrap();

        assert!(registry.rename(&id, " \n\t ".into()).is_err());
        assert!(registry
            .rename(
                &DaemonId {
                    node_id: "missing".into()
                },
                "New".into()
            )
            .is_err());

        assert_eq!(registry.get(&id).unwrap().name, "Original");
        assert_eq!(registry.list().len(), 1);
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn a_failed_daemon_rename_preserves_the_saved_and_in_memory_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        let mut registry = DaemonRegistry::load(path.clone()).unwrap();
        let id = registry.add("node1".into(), "Original".into());
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();

        assert!(registry.rename(&id, "Unsaved".into()).is_err());

        assert_eq!(registry.get(&id).unwrap().name, "Original");
        assert_eq!(
            DaemonRegistry::load(path).unwrap().get(&id).unwrap().name,
            "Original"
        );
    }

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
        assert!(reg
            .get(&DaemonId {
                node_id: "node1".into()
            })
            .is_some());
        assert!(reg
            .get(&DaemonId {
                node_id: "node2".into()
            })
            .is_some());
        assert!(reg
            .get(&DaemonId {
                node_id: "node3".into()
            })
            .is_none());
    }

    #[test]
    fn clearing_a_consumed_pairing_token_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        let mut registry = DaemonRegistry::load(path.clone()).unwrap();
        let id = registry.add_with_token("node1".into(), "Homelab".into(), "invite".into());

        registry.clear_pairing_token(&id).unwrap();

        assert_eq!(registry.get(&id).unwrap().pairing_token, None);
        let saved = DaemonRegistry::load(path).unwrap();
        assert_eq!(saved.get(&id).unwrap().pairing_token, None);
        assert_eq!(saved.get(&id).unwrap().name, "Homelab");
    }

    #[test]
    fn a_failed_token_clear_keeps_the_saved_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        let mut registry = DaemonRegistry::load(path.clone()).unwrap();
        let id = registry.add_with_token("node1".into(), "Homelab".into(), "invite".into());
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();

        assert!(registry.clear_pairing_token(&id).is_err());

        assert_eq!(
            registry.get(&id).unwrap().pairing_token.as_deref(),
            Some("invite")
        );
        assert_eq!(
            DaemonRegistry::load(path)
                .unwrap()
                .get(&id)
                .unwrap()
                .pairing_token
                .as_deref(),
            Some("invite")
        );
    }

    #[test]
    fn clearing_a_token_for_an_unknown_daemon_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = DaemonRegistry::load(dir.path().join("daemons.json")).unwrap();

        assert!(registry
            .clear_pairing_token(&DaemonId {
                node_id: "missing".into()
            })
            .is_err());
    }
}
