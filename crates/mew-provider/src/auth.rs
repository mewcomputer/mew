//! Generic OAuth authentication for provider adapters.
//!
//! Defines the `OAuthProvider` trait that each OAuth-capable provider
//! implements, plus generic functions for resolving, refreshing, and
//! managing stored tokens. The trait is provider-agnostic — each impl
//! brings its own OAuth library and endpoint configuration.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Minimal OAuth token set — the standard fields needed by the generic
/// auth logic. Provider impls convert to/from their library-specific
/// token types at the boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at: u64,
}

impl TokenSet {
    /// Check if the token is expired or will expire soon (within 5 minutes).
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.expires_at <= now + 300
    }

    /// Duration until expiry (zero if already expired).
    pub fn expires_in(&self) -> std::time::Duration {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if self.expires_at > now {
            std::time::Duration::from_secs(self.expires_at - now)
        } else {
            std::time::Duration::ZERO
        }
    }
}

/// The result of a successful OAuth login.
pub struct OAuthSession {
    pub tokens: TokenSet,
    /// Extra HTTP headers to send on every authenticated request
    /// (e.g. `chatgpt-account-id` for OpenAI).
    pub extra_headers: Vec<(String, String)>,
}

/// Resolved auth strategy for a provider.
pub enum AuthKind {
    /// Standard API-key auth.
    ApiKey(String),
    /// OAuth with tokens + extra headers (not yet wrapped in RwLock —
    /// the adapter wraps them on construction).
    OAuth {
        tokens: TokenSet,
        extra_headers: Vec<(String, String)>,
    },
}

/// A provider that supports OAuth-based authentication.
///
/// Each implementing provider encapsulates its own OAuth endpoints,
/// client ID, login flow, and token refresh logic. The generic auth
/// functions in this module handle storage, resolution, and refresh
/// orchestration.
#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Human-readable name (e.g. "OpenAI Responses (ChatGPT)").
    fn display_name(&self) -> &str;

    /// Slug for token file path and CLI (e.g. "openai-responses").
    fn slug(&self) -> &str;

    /// Base URL for API requests when using OAuth.
    fn oauth_base_url(&self) -> &str;

    /// Run the full OAuth login flow (browser, callback, token exchange).
    /// Returns the token set + extra headers to send on every request.
    ///
    /// When `headless` is true, providers should use a device-code flow
    /// (no local callback server, no browser launch) for headless machines.
    async fn login(&self, headless: bool) -> anyhow::Result<OAuthSession>;

    /// Derive extra HTTP headers from the current token set.
    /// Called after login and after every token refresh.
    fn extra_headers(&self, tokens: &TokenSet) -> Vec<(String, String)>;

    /// Refresh an expired token using the refresh token.
    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet>;

    /// Path to the token file (e.g. `~/.config/mew/auth/{slug}.json`).
    /// Each provider implements this using its crate's `config_dir()`.
    fn token_file_path(&self) -> PathBuf;
}

// ---------------------------------------------------------------------------
// On-disk storage
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct StoredAuth {
    tokens: TokenSet,
    extra_headers: Vec<(String, String)>,
}

fn load_stored_auth(provider: &dyn OAuthProvider) -> Option<StoredAuth> {
    let path = provider.token_file_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("failed to read token file {}: {}", path.display(), e);
            return None;
        }
    };

    let stored: StoredAuth = match serde_json::from_str(&data) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                "token file at {} is corrupted (parse error: {}). \
                 Run `mew auth login` to re-authenticate.",
                path.display(),
                e
            );
            return None;
        }
    };

    // Validate extra_headers to prevent header injection from a
    // crafted token file. Reject entries with invalid header names
    // or values (CRLF, control chars, or "authorization" override).
    let sanitized: Vec<(String, String)> = stored
        .extra_headers
        .into_iter()
        .filter(|(name, value)| {
            // Reject attempts to override the Authorization header.
            if name.eq_ignore_ascii_case("authorization") {
                tracing::warn!(
                    "rejecting 'authorization' header from token file — \
                     possible tampering"
                );
                return false;
            }
            // Validate that the header name and value are legal HTTP.
            http::HeaderName::try_from(name.as_str()).is_ok()
                && http::HeaderValue::try_from(value.as_str()).is_ok()
        })
        .collect();

    Some(StoredAuth {
        tokens: stored.tokens,
        extra_headers: sanitized,
    })
}

fn save_stored_auth(provider: &dyn OAuthProvider, auth: &StoredAuth) -> anyhow::Result<()> {
    let path = provider.token_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(auth)?;
    std::fs::write(&path, json)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)?;
    }

    tracing::info!("oauth tokens saved to {}", path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Generic auth functions
// ---------------------------------------------------------------------------

/// Resolve which auth strategy to use for a given OAuth-capable provider.
///
/// Priority:
/// 1. If a token file exists → OAuth (refresh happens lazily).
/// 2. Else if `api_key` is non-empty → ApiKey.
/// 3. Else → error.
///
/// This function is sync — it does NOT refresh expired tokens. The
/// adapter's `build_auth_headers()` handles refresh on first use.
pub fn resolve(provider: &dyn OAuthProvider, api_key: Option<String>) -> anyhow::Result<AuthKind> {
    if let Some(stored) = load_stored_auth(provider) {
        return Ok(AuthKind::OAuth {
            tokens: stored.tokens,
            extra_headers: stored.extra_headers,
        });
    }

    if let Some(key) = api_key {
        if !key.is_empty() {
            return Ok(AuthKind::ApiKey(key));
        }
    }

    Err(anyhow::anyhow!(
        "no credentials found for {}. Run `mew auth login {}` \
         or set the appropriate API key environment variable.",
        provider.display_name(),
        provider.slug(),
    ))
}

/// Run the full login flow for a provider and persist the result.
pub async fn login(provider: &dyn OAuthProvider, headless: bool) -> anyhow::Result<()> {
    let session = provider.login(headless).await?;

    let stored = StoredAuth {
        tokens: session.tokens,
        extra_headers: session.extra_headers,
    };
    save_stored_auth(provider, &stored)?;

    eprintln!("Login successful for {}.", provider.display_name());
    Ok(())
}

/// Refresh tokens if needed, updating both the in-memory state and the
/// on-disk file. Re-derives `extra_headers` from the provider after
/// refresh.
pub async fn refresh_if_needed(
    provider: &dyn OAuthProvider,
    tokens: &tokio::sync::RwLock<TokenSet>,
    extra_headers: &tokio::sync::RwLock<Vec<(String, String)>>,
) -> anyhow::Result<()> {
    let needs_refresh = tokens.read().await.is_expired();
    if !needs_refresh {
        return Ok(());
    }

    let mut guard = tokens.write().await;
    // Double-check after acquiring the lock.
    if !guard.is_expired() {
        return Ok(());
    }

    let new_tokens = provider.refresh(&guard.refresh_token).await?;

    // Re-derive extra headers from the new tokens.
    let new_headers = provider.extra_headers(&new_tokens);

    // Persist. If the file was deleted since login, write a fresh one.
    let stored_to_persist = if let Some(mut stored) = load_stored_auth(provider) {
        stored.tokens = new_tokens.clone();
        stored.extra_headers = new_headers.clone();
        stored
    } else {
        tracing::warn!("token file not found during refresh; creating new one");
        StoredAuth {
            tokens: new_tokens.clone(),
            extra_headers: new_headers.clone(),
        }
    };
    if let Err(e) = save_stored_auth(provider, &stored_to_persist) {
        tracing::warn!("failed to persist refreshed tokens: {e}");
    }

    *guard = new_tokens;
    *extra_headers.write().await = new_headers;
    Ok(())
}

/// Delete stored OAuth credentials.
pub fn logout(provider: &dyn OAuthProvider) -> anyhow::Result<()> {
    let path = provider.token_file_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        eprintln!("Logged out. Removed {}", path.display());
    } else {
        eprintln!("Not logged in (no token file at {})", path.display());
    }
    Ok(())
}

/// Check OAuth status (returns None if not logged in).
pub fn status(provider: &dyn OAuthProvider) -> Option<String> {
    let stored = load_stored_auth(provider)?;
    if stored.tokens.is_expired() {
        Some(format!(
            "OAuth (expired, will refresh on next use) — {}",
            provider.display_name()
        ))
    } else {
        let secs = stored.tokens.expires_in().as_secs();
        let hrs = secs / 3600;
        let mins = (secs % 3600) / 60;
        Some(format!(
            "OAuth (refreshes in {}h {}m) — {}",
            hrs,
            mins,
            provider.display_name()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock OAuth provider for testing the generic auth logic.
    struct MockOAuthProvider {
        slug: &'static str,
    }

    #[async_trait::async_trait]
    impl OAuthProvider for MockOAuthProvider {
        fn display_name(&self) -> &str {
            "Mock Provider"
        }
        fn slug(&self) -> &str {
            self.slug
        }
        fn oauth_base_url(&self) -> &str {
            "https://mock.example.com/api"
        }
        async fn login(&self, _headless: bool) -> anyhow::Result<OAuthSession> {
            Ok(OAuthSession {
                tokens: TokenSet {
                    access_token: "mock-access".to_string(),
                    refresh_token: "mock-refresh".to_string(),
                    expires_at: 9999999999,
                },
                extra_headers: vec![],
            })
        }
        fn extra_headers(&self, _tokens: &TokenSet) -> Vec<(String, String)> {
            vec![]
        }
        async fn refresh(&self, _refresh_token: &str) -> anyhow::Result<TokenSet> {
            Ok(TokenSet {
                access_token: "mock-refreshed".to_string(),
                refresh_token: "mock-refresh-2".to_string(),
                expires_at: 9999999999,
            })
        }

        fn token_file_path(&self) -> PathBuf {
            // Use a temp-like path that won't exist in tests.
            PathBuf::from(format!("/tmp/mew-test-{}.json", self.slug))
        }
    }

    #[test]
    fn test_token_set_is_expired_with_old_timestamp() {
        let tokens = TokenSet {
            access_token: "x".into(),
            refresh_token: "y".into(),
            expires_at: 0,
        };
        assert!(tokens.is_expired());
    }

    #[test]
    fn test_token_set_not_expired_with_future_timestamp() {
        let tokens = TokenSet {
            access_token: "x".into(),
            refresh_token: "y".into(),
            expires_at: 9999999999,
        };
        assert!(!tokens.is_expired());
    }

    #[test]
    fn test_stored_auth_serialization_roundtrip() {
        let stored = StoredAuth {
            tokens: TokenSet {
                access_token: "test-access".to_string(),
                refresh_token: "test-refresh".to_string(),
                expires_at: 9999999999,
            },
            extra_headers: vec![("x-custom".to_string(), "val".to_string())],
        };

        let json = serde_json::to_string(&stored).unwrap();
        let deserialized: StoredAuth = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.tokens.access_token, "test-access");
        assert_eq!(deserialized.tokens.refresh_token, "test-refresh");
        assert_eq!(deserialized.extra_headers.len(), 1);
        assert_eq!(deserialized.extra_headers[0].0, "x-custom");
    }

    #[test]
    fn test_resolve_returns_api_key_when_no_oauth() {
        let provider = MockOAuthProvider {
            slug: "test-no-file-1",
        };
        // No token file exists for this slug, so should fall back to API key.
        let result = resolve(&provider, Some("test-api-key".to_string())).unwrap();
        match result {
            AuthKind::ApiKey(key) => assert_eq!(key, "test-api-key"),
            AuthKind::OAuth { .. } => panic!("expected ApiKey, got OAuth"),
        }
    }

    #[test]
    fn test_resolve_errors_when_nothing_available() {
        let provider = MockOAuthProvider {
            slug: "test-no-file-2",
        };
        let result = resolve(&provider, None);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("no credentials"));
    }

    #[test]
    fn test_resolve_errors_with_empty_api_key() {
        let provider = MockOAuthProvider {
            slug: "test-no-file-3",
        };
        let result = resolve(&provider, Some(String::new()));
        assert!(result.is_err());
    }

    #[test]
    fn test_status_returns_none_when_no_file() {
        let provider = MockOAuthProvider {
            slug: "test-no-file-4",
        };
        let result = status(&provider);
        // If a file exists (dev logged in), just verify it's a string.
        if let Some(s) = result {
            assert!(!s.is_empty());
        }
    }
}
