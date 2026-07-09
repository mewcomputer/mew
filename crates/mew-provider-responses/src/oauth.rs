//! OpenAI Responses OAuth provider implementation.
//!
//! Implements the `OAuthProvider` trait for OpenAI's ChatGPT subscription
//! OAuth flow. Uses the `openai-auth` crate for the PKCE flow, callback
//! server, and JWT extraction.

use std::path::PathBuf;

use async_trait::async_trait;
use mew_provider::auth::{OAuthProvider, OAuthSession, TokenSet};

pub struct OpenaiResponsesOAuth;

#[async_trait]
impl OAuthProvider for OpenaiResponsesOAuth {
    fn display_name(&self) -> &str {
        "OpenAI Responses (ChatGPT)"
    }

    fn slug(&self) -> &str {
        "openai-responses"
    }

    fn oauth_base_url(&self) -> &str {
        "https://chatgpt.com/backend-api/codex"
    }

    async fn login(&self) -> anyhow::Result<OAuthSession> {
        let client = openai_auth::OAuthClient::new(openai_auth::OAuthConfig::default())?;
        let flow = client.start_flow()?;

        eprintln!("Opening browser for OpenAI login...");
        eprintln!("If the browser doesn't open, visit this URL:");
        eprintln!("{}\n", flow.authorization_url);

        let _ = openai_auth::open_browser(&flow.authorization_url);

        let tokens = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            openai_auth::run_callback_server(1455, &flow.state, &client, &flow.pkce_verifier),
        )
        .await
        .map_err(|_| anyhow::anyhow!("login timed out after 120 seconds"))??;

        // Extract account_id from the access_token JWT.
        let account_id = client.extract_account_id(&tokens.access_token)?;

        let our_tokens = TokenSet {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: tokens.expires_at,
        };

        Ok(OAuthSession {
            tokens: our_tokens,
            extra_headers: vec![("chatgpt-account-id".to_string(), account_id)],
        })
    }

    fn extra_headers(&self, tokens: &TokenSet) -> Vec<(String, String)> {
        let client = match openai_auth::OAuthClient::new(openai_auth::OAuthConfig::default()) {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        match client.extract_account_id(&tokens.access_token) {
            Ok(account_id) => vec![("chatgpt-account-id".to_string(), account_id)],
            Err(e) => {
                tracing::warn!("failed to extract account_id for extra_headers: {e}");
                vec![]
            }
        }
    }

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet> {
        let client = openai_auth::OAuthClient::new(openai_auth::OAuthConfig::default())?;
        let tokens = client.refresh_token(refresh_token).await?;
        Ok(TokenSet {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: tokens.expires_at,
        })
    }

    fn token_file_path(&self) -> PathBuf {
        mew_config::config_dir()
            .join("auth")
            .join("openai-responses.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slug_is_openai_responses() {
        let provider = OpenaiResponsesOAuth;
        assert_eq!(provider.slug(), "openai-responses");
    }

    #[test]
    fn test_token_file_path_ends_with_auth_openai_responses_json() {
        let provider = OpenaiResponsesOAuth;
        let path = provider.token_file_path();
        assert!(path.ends_with("auth/openai-responses.json"));
    }

    #[test]
    fn test_oauth_base_url_is_chatgpt_backend() {
        let provider = OpenaiResponsesOAuth;
        assert_eq!(
            provider.oauth_base_url(),
            "https://chatgpt.com/backend-api/codex"
        );
    }
}
