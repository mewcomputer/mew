//! OpenAI Responses API adapter.
//!
//! Implements the `POST /v1/responses` wire protocol used by Codex CLI
//! and GPT-5-codex models. This is a distinct API surface from
//! chat/completions — different request body, different SSE event
//! grammar, different role mapping.
//!
//! Supports both API-key auth (Phase 1) and ChatGPT OAuth (Phase 2).

pub mod oauth;

mod openai_oauth;
mod provider;
mod request;
mod stream;

#[cfg(test)]
mod test_support;

use mew_provider::auth::OAuthProvider;
use mew_provider::ProviderError;

pub struct Adapter {
    name: String,
    base_url: String,
    model: String,
    auth: AdapterAuth,
    /// OAuth provider reference for token refresh. None for API-key auth.
    oauth_provider: Option<std::sync::Arc<dyn OAuthProvider>>,
    client: reqwest::Client,
    dump: bool,
    /// True for Codex models that require the Responses Lite transport
    /// (e.g. gpt-5.6-sol/terra/luna).
    use_responses_lite: bool,
}

impl Adapter {
    pub fn with_responses_lite(mut self, v: bool) -> Self {
        self.use_responses_lite = v;
        self
    }
}

/// The auth state held by the adapter. OAuth tokens are wrapped in
/// RwLock so `&self` methods can refresh them in place.
enum AdapterAuth {
    ApiKey(String),
    OAuth {
        tokens: tokio::sync::RwLock<mew_provider::auth::TokenSet>,
        extra_headers: tokio::sync::RwLock<Vec<(String, String)>>,
    },
}

impl Adapter {
    pub fn new(name: String, base_url: String, model: String, api_key: String) -> Self {
        Self {
            name,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            auth: AdapterAuth::ApiKey(api_key),
            oauth_provider: None,
            client: reqwest::Client::new(),
            dump: false,
            use_responses_lite: false,
        }
    }

    /// Create an adapter authenticated via OAuth.
    pub fn new_oauth(
        name: String,
        model: String,
        tokens: mew_provider::auth::TokenSet,
        extra_headers: Vec<(String, String)>,
        provider: std::sync::Arc<dyn OAuthProvider>,
    ) -> Self {
        Self {
            name,
            base_url: provider.oauth_base_url().to_string(),
            model,
            auth: AdapterAuth::OAuth {
                tokens: tokio::sync::RwLock::new(tokens),
                extra_headers: tokio::sync::RwLock::new(extra_headers),
            },
            oauth_provider: Some(provider),
            client: reqwest::Client::new(),
            dump: false,
            use_responses_lite: false,
        }
    }

    pub fn set_dump(&mut self, v: bool) {
        self.dump = v;
    }

    /// Build the authorization header value(s) for the current auth kind.
    /// For OAuth, also refreshes tokens if expired.
    pub(crate) async fn build_auth_headers(
        &self,
    ) -> Result<(String, Vec<(String, String)>), ProviderError> {
        match &self.auth {
            AdapterAuth::ApiKey(key) => Ok((format!("Bearer {key}"), vec![])),
            AdapterAuth::OAuth {
                tokens,
                extra_headers,
            } => {
                if let Some(provider) = &self.oauth_provider {
                    mew_provider::auth::refresh_if_needed(provider.as_ref(), tokens, extra_headers)
                        .await
                        .map_err(|e| {
                            ProviderError::Message(format!("oauth refresh failed: {e}"))
                        })?;
                }
                let guard = tokens.read().await;
                let headers = extra_headers.read().await;
                Ok((format!("Bearer {}", guard.access_token), headers.clone()))
            }
        }
    }
}
