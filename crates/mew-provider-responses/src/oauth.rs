//! Codex (ChatGPT subscription) OAuth provider implementation.
//!
//! Implements the `OAuthProvider` trait for the ChatGPT-subscription OAuth
//! flow used by Codex / GPT-5-codex models. The protocol machinery (PKCE,
//! callback server, device flow, JWT extraction) lives in
//! [`crate::openai_oauth`]; this file wires it to the generic auth layer and
//! picks between the browser and headless flows.

use std::path::PathBuf;

use async_trait::async_trait;
use mew_provider::auth::{OAuthProvider, OAuthSession, TokenSet};

use crate::openai_oauth::{self, OAuthConfig};

pub struct CodexOAuth;

/// Path to the stored Codex OAuth token file. Used by the model picker to
/// gate Codex catalog models on login presence (OAuth credentials live in
/// this file, not in a `credential_ref` API-key slot).
///
/// Migrates the legacy `openai-responses.json` path to `codex.json` once,
/// so existing logins survive the provider rename.
pub fn codex_token_path() -> std::path::PathBuf {
    static MIGRATE: std::sync::Once = std::sync::Once::new();
    MIGRATE.call_once(|| {
        let auth_dir = mew_config::config_dir().join("auth");
        let new_path = auth_dir.join("codex.json");
        if !new_path.exists() {
            let old_path = auth_dir.join("openai-responses.json");
            if old_path.exists() {
                if let Err(e) = std::fs::rename(&old_path, &new_path) {
                    tracing::warn!(?e, "failed to migrate openai-responses.json → codex.json");
                } else {
                    tracing::info!("migrated oauth token openai-responses.json → codex.json");
                }
            }
        }
    });
    mew_config::config_dir().join("auth").join("codex.json")
}

impl CodexOAuth {
    /// Shared tail for both flows: exchange the code, extract the account id,
    /// and assemble the `OAuthSession`.
    async fn finalize_login(
        cfg: &OAuthConfig,
        code: String,
        verifier: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<OAuthSession> {
        let tokens = openai_oauth::exchange_code(cfg, &code, verifier, redirect_uri).await?;
        let account_id = openai_oauth::extract_account_id(&tokens.access_token)?;
        Ok(OAuthSession {
            tokens: TokenSet {
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
                expires_at: tokens.expires_at,
            },
            extra_headers: vec![("chatgpt-account-id".to_string(), account_id)],
        })
    }
}

#[async_trait]
impl OAuthProvider for CodexOAuth {
    fn display_name(&self) -> &str {
        "Codex (ChatGPT)"
    }

    fn slug(&self) -> &str {
        "codex"
    }

    fn oauth_base_url(&self) -> &str {
        "https://chatgpt.com/backend-api/codex"
    }

    async fn login(&self, headless: bool) -> anyhow::Result<OAuthSession> {
        let cfg = OAuthConfig::default();
        if headless {
            Self::login_headless(&cfg).await
        } else {
            Self::login_browser(&cfg).await
        }
    }

    fn extra_headers(&self, tokens: &TokenSet) -> Vec<(String, String)> {
        match openai_oauth::extract_account_id(&tokens.access_token) {
            Ok(account_id) => vec![("chatgpt-account-id".to_string(), account_id)],
            Err(e) => {
                tracing::warn!("failed to extract account_id for extra_headers: {e}");
                vec![]
            }
        }
    }

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet> {
        let cfg = OAuthConfig::default();
        let tokens = openai_oauth::refresh_tokens(&cfg, refresh_token).await?;
        Ok(TokenSet {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: tokens.expires_at,
        })
    }

    fn token_file_path(&self) -> PathBuf {
        codex_token_path()
    }
}

impl CodexOAuth {
    /// Browser PKCE flow: bind the callback server, open the platform
    /// browser (never `$BROWSER`), wait for the redirect, exchange.
    async fn login_browser(cfg: &OAuthConfig) -> anyhow::Result<OAuthSession> {
        // Bind BEFORE opening the browser so an in-use port fails fast.
        let server = openai_oauth::CallbackServer::bind(1455).await?;
        // Registered redirect URI — byte-exact, matches the bound port (1455).
        let redirect_uri = openai_oauth::PROD_REDIRECT_URI.to_string();
        let pkce = openai_oauth::generate_pkce();
        let state = openai_oauth::generate_state();
        let url = openai_oauth::build_authorize_url(cfg, &redirect_uri, &pkce.challenge, &state)?;

        eprintln!("Opening browser for OpenAI login...");
        eprintln!("If the browser doesn't open, visit this URL:");
        eprintln!("{url}\n");

        // Launch failure is non-fatal — the URL is already printed.
        let _ = openai_oauth::open_browser(&url);

        let code = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            server.wait_for_code(&state),
        )
        .await
        .map_err(|_| anyhow::anyhow!("login timed out after 120 seconds"))??;

        Self::finalize_login(cfg, code, &pkce.verifier, &redirect_uri).await
    }

    /// Headless device-code flow: request a user code, have the user visit
    /// the device page on any browser (e.g. a phone), poll until authorized,
    /// then exchange with the server-supplied PKCE verifier.
    async fn login_headless(cfg: &OAuthConfig) -> anyhow::Result<OAuthSession> {
        let dc = openai_oauth::request_device_code(cfg).await?;
        eprintln!("Open this URL on any device and enter the code:");
        eprintln!("    {}", cfg.device_page_url());
        eprintln!("Code: {}\n", dc.user_code);
        eprintln!("Waiting for authorization (this can take a few minutes)...\n");

        let (code, verifier) = tokio::time::timeout(
            std::time::Duration::from_secs(900),
            openai_oauth::poll_device_token(cfg, &dc),
        )
        .await
        .map_err(|_| anyhow::anyhow!("device login timed out after 900 seconds"))??;

        let redirect_uri = cfg.device_redirect_uri();
        Self::finalize_login(cfg, code, &verifier, &redirect_uri).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_jwt(payload_json: &str) -> String {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn test_slug_is_codex() {
        let provider = CodexOAuth;
        assert_eq!(provider.slug(), "codex");
    }

    #[test]
    fn test_token_file_path_ends_with_auth_codex_json() {
        let provider = CodexOAuth;
        let path = provider.token_file_path();
        assert!(path.ends_with("auth/codex.json"));
    }

    #[test]
    fn test_oauth_base_url_is_chatgpt_backend() {
        let provider = CodexOAuth;
        assert_eq!(
            provider.oauth_base_url(),
            "https://chatgpt.com/backend-api/codex"
        );
    }

    #[test]
    fn test_extra_headers_extracts_account_id_from_jwt() {
        let provider = CodexOAuth;
        let payload = serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_99" }
        })
        .to_string();
        let tokens = TokenSet {
            access_token: fake_jwt(&payload),
            refresh_token: "r".into(),
            expires_at: 0,
        };
        let headers = provider.extra_headers(&tokens);
        assert_eq!(
            headers,
            vec![("chatgpt-account-id".into(), "acct_99".into())]
        );
    }

    #[test]
    fn test_extra_headers_empty_on_bad_token() {
        let provider = CodexOAuth;
        let tokens = TokenSet {
            access_token: "not-a-jwt".into(),
            refresh_token: "r".into(),
            expires_at: 0,
        };
        let headers = provider.extra_headers(&tokens);
        assert!(headers.is_empty(), "bad token should yield no headers");
    }
}
