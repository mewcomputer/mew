use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::SessionError;

/// Runtime choices persisted independently of history and usage metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPolicy {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub persona: Option<String>,
    pub thinking_variant: Option<String>,
    pub permission_mode: String,
}

impl SessionPolicy {
    pub async fn read(root: &Path, session_id: &str) -> Result<Option<Self>, SessionError> {
        match tokio::fs::read(root.join(session_id).join("policy.json")).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn write(&self, root: &Path, session_id: &str) -> Result<(), SessionError> {
        let directory = root.join(session_id);
        tokio::fs::create_dir_all(&directory).await?;
        let temporary = directory.join("policy.json.tmp");
        tokio::fs::write(&temporary, serde_json::to_vec_pretty(self)?).await?;
        tokio::fs::rename(temporary, directory.join("policy.json")).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn absent_policy_preserves_legacy_defaults() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            SessionPolicy::read(root.path(), "legacy").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn policy_roundtrip_preserves_explicit_thinking_off_and_metadata() {
        let root = tempfile::tempdir().unwrap();
        let mut meta = crate::Meta::new("session");
        meta.custom_title = Some("my task".into());
        meta.write(root.path()).await.unwrap();
        let policy = SessionPolicy {
            provider: Some("test-provider".into()),
            model: Some("test-model".into()),
            persona: Some("planner".into()),
            thinking_variant: None,
            permission_mode: "permissive".into(),
        };
        policy.write(root.path(), "session").await.unwrap();
        assert_eq!(
            SessionPolicy::read(root.path(), "session").await.unwrap(),
            Some(policy)
        );
        assert_eq!(
            crate::Meta::read(root.path(), "session")
                .await
                .unwrap()
                .unwrap()
                .custom_title,
            meta.custom_title
        );
    }

    #[tokio::test]
    async fn malformed_policy_is_visible_and_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("session");
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("policy.json");
        tokio::fs::write(&path, b"{invalid").await.unwrap();
        assert!(SessionPolicy::read(root.path(), "session").await.is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"{invalid");
    }

    #[tokio::test]
    async fn updates_replace_the_complete_policy() {
        let root = tempfile::tempdir().unwrap();
        let mut policy = SessionPolicy {
            provider: None,
            model: None,
            persona: None,
            thinking_variant: Some("high".into()),
            permission_mode: "standard".into(),
        };
        policy.write(root.path(), "session").await.unwrap();
        policy.thinking_variant = None;
        policy.permission_mode = "auto_plus".into();
        policy.write(root.path(), "session").await.unwrap();
        assert_eq!(
            SessionPolicy::read(root.path(), "session").await.unwrap(),
            Some(policy)
        );
        assert!(!root.path().join("session/policy.json.tmp").exists());
    }
}
