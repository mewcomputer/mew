//! Shared fixtures for the adapter unit tests.

use crate::Adapter;
use async_trait::async_trait;
use mew_message::{Message, Part, PartBase, Role, TextPart};

pub(crate) fn make_adapter() -> Adapter {
    Adapter::new(
        "test".to_string(),
        "https://api.openai.com/v1".to_string(),
        "gpt-5-codex".to_string(),
        "test-key".to_string(),
    )
}

pub(crate) fn make_message(role: Role, text: &str) -> Message {
    Message {
        id: ulid::Ulid::new(),
        session_id: ulid::Ulid::new(),
        role,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: text.to_string(),
            synthetic: false,
        })],
        time: mew_message::Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    }
}

/// Saves and restores the codex cache file so the best-effort cache
/// write inside `list_models` (OAuth path) can't pollute the developer's
/// real cache — even if the test panics.
pub(crate) struct CodexCacheRestore {
    path: std::path::PathBuf,
    original: Option<Vec<u8>>,
}

impl CodexCacheRestore {
    pub(crate) fn new(path: std::path::PathBuf) -> Self {
        let original = std::fs::read(&path).ok();
        Self { path, original }
    }
}

impl Drop for CodexCacheRestore {
    fn drop(&mut self) {
        match &self.original {
            Some(data) => {
                let _ = std::fs::write(&self.path, data);
            }
            None => {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

/// A minimal `OAuthProvider` whose `oauth_base_url` is the mock server,
/// so `Adapter::new_oauth` points at the wiremock instance.
pub(crate) struct TestOAuthProvider {
    pub(crate) base_url: String,
}

#[async_trait]
impl mew_provider::auth::OAuthProvider for TestOAuthProvider {
    fn display_name(&self) -> &str {
        "test"
    }

    fn slug(&self) -> &str {
        "test"
    }

    fn oauth_base_url(&self) -> &str {
        &self.base_url
    }

    async fn login(&self, _: bool) -> anyhow::Result<mew_provider::auth::OAuthSession> {
        anyhow::bail!("not used in tests")
    }

    fn extra_headers(&self, _: &mew_provider::auth::TokenSet) -> Vec<(String, String)> {
        vec![]
    }

    async fn refresh(&self, _: &str) -> anyhow::Result<mew_provider::auth::TokenSet> {
        anyhow::bail!("not used in tests")
    }

    fn token_file_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/mew-test-oauth-not-used")
    }
}
