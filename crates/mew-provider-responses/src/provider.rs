//! `Provider` implementation: request execution, retry, and model listing.

use crate::Adapter;
use async_trait::async_trait;
use futures::channel::mpsc;
use futures::SinkExt;
use mew_message::MessageError;
use mew_provider::{
    classify_error, classify_reason, EventStream, Provider, ProviderError, ProviderEvent, Request,
    RetryPolicy,
};

#[async_trait]
impl Provider for Adapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn stream(&self, req: Request) -> Result<EventStream, ProviderError> {
        let body = self.build_request_body(&req).await?;

        if self.dump {
            if let Ok(pretty) = serde_json::from_slice::<serde_json::Value>(&body)
                .and_then(|v| serde_json::to_string_pretty(&v))
            {
                eprintln!("\n[RAW REQUEST BODY]\n{pretty}\n");
            } else {
                eprintln!("\n[RAW REQUEST BODY]\n{}\n", String::from_utf8_lossy(&body));
            }
        }

        let url = format!("{}/responses", self.base_url);
        let (auth_header, extra_headers) = self.build_auth_headers().await?;
        let mut request_builder = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", auth_header)
            .header("Accept", "text/event-stream");
        for (name, value) in &extra_headers {
            request_builder = request_builder.header(name, value);
        }
        if self.use_responses_lite {
            request_builder =
                request_builder.header("x-openai-internal-codex-responses-lite", "true");
        }
        let request = request_builder.body(body).build()?;

        let policy = RetryPolicy::default();
        let (tx, rx) = mpsc::channel(128);
        let mut retry_tx = tx.clone();
        let mut resp = None;

        for attempt in 0.. {
            let req = request.try_clone().ok_or_else(|| {
                ProviderError::Message("request cannot be cloned for retry".to_string())
            })?;
            let r = self.client.execute(req).await?;
            if r.status().is_success() {
                resp = Some(r);
                break;
            }
            let status = r.status().as_u16();
            let data = r.text().await.unwrap_or_default();
            let (backoff, retry) = policy.should_retry(status, attempt);
            if !retry {
                let (kind, msg) = classify_error(status, &data);
                let _ = retry_tx
                    .send(ProviderEvent::Error(MessageError {
                        kind,
                        message: msg.clone(),
                    }))
                    .await;
                return Err(ProviderError::Classified { kind, message: msg });
            }
            let _ = retry_tx
                .send(ProviderEvent::RetryWait {
                    attempt: attempt as u32 + 1,
                    max_attempts: 4,
                    delay_secs: backoff.as_secs(),
                    reason: classify_reason(status),
                })
                .await;
            tokio::time::sleep(backoff).await;
        }

        drop(retry_tx);
        let resp = resp.ok_or_else(|| {
            ProviderError::Message("retry loop exited without response".to_string())
        })?;
        let dump = self.dump;
        let use_responses_lite = self.use_responses_lite;
        tokio::spawn(async move {
            Self::read_stream(dump, resp, tx, use_responses_lite).await;
        });

        Ok(Box::pin(rx))
    }

    async fn list_models(&self) -> Result<Vec<mew_provider::ModelInfo>, ProviderError> {
        let url = format!("{}/models", self.base_url);
        let (auth_header, extra_headers) = self.build_auth_headers().await?;
        let mut request_builder = self.client.get(&url).header("Authorization", auth_header);
        for (name, value) in &extra_headers {
            request_builder = request_builder.header(name, value);
        }
        let resp = request_builder.send().await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let (kind, msg) = classify_error(status, &body);
            return Err(ProviderError::Classified { kind, message: msg });
        }

        // The ChatGPT (OAuth) backend returns codex's `ModelsResponse` shape
        // { "models": [...] }; the API-key backend returns OpenAI's
        // { "data": [...] }. The OAuth path also refreshes the codex catalog
        // cache with the live, plan-filtered response so the daemon path
        // (which reads the catalog, not list_models) benefits next launch.
        if self.oauth_provider.is_some() {
            let body = resp.text().await.unwrap_or_default();
            let _ = mew_catalog::write_codex_cache(&body);
            let models = mew_catalog::parse_codex(body.as_bytes())
                .map_err(|e| ProviderError::Message(format!("codex models parse failed: {e}")))?;
            return Ok(models
                .into_iter()
                .map(|m| mew_provider::ModelInfo {
                    id: m.id,
                    owned_by: "openai".to_string(),
                })
                .collect());
        }

        #[derive(serde::Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }
        #[derive(serde::Deserialize)]
        struct ModelEntry {
            id: String,
            owned_by: Option<String>,
        }

        let models: ModelsResponse = resp.json().await?;
        Ok(models
            .data
            .into_iter()
            .map(|m| mew_provider::ModelInfo {
                id: m.id,
                owned_by: m.owned_by.unwrap_or_default(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use mew_message::{ErrorKind, Role};
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_auth_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string("{\"error\":{\"message\":\"Invalid API key\"}}"),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "bad-key".to_string(),
        );

        let req = Request {
            model: "gpt-5-codex".to_string(),
            messages: vec![make_message(Role::User, "hi")],
            tools: vec![],
            system: String::new(),
            reasoning: None,
            params: None,
            headers: http::HeaderMap::new(),
            ..Default::default()
        };

        let result = adapter.stream(req).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(matches!(
            err,
            ProviderError::Classified {
                kind: ErrorKind::ProviderAuth,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_list_models() {
        let server = MockServer::start().await;

        let models_body = serde_json::json!({
            "data": [
                {"id": "gpt-5-codex", "owned_by": "openai"},
                {"id": "gpt-5.5", "owned_by": "openai"},
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(serde_json::to_string(&models_body).unwrap()),
            )
            .mount(&server)
            .await;

        let adapter = Adapter::new(
            "test".to_string(),
            server.uri() + "/v1",
            "gpt-5-codex".to_string(),
            "test-key".to_string(),
        );

        let models = adapter.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5-codex");
        assert_eq!(models[0].owned_by, "openai");
        assert_eq!(models[1].id, "gpt-5.5");
    }

    #[tokio::test]
    async fn test_list_models_oauth_parses_codex_response_and_refreshes_cache() {
        let server = MockServer::start().await;

        // Codex ModelsResponse shape: { "models": [...] }.
        let codex_body = json!({
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "visibility": "list",
                    "supported_in_api": true,
                    "context_window": 372000,
                    "default_reasoning_level": "low",
                    "supported_reasoning_levels": [{"effort": "low"}, {"effort": "high"}],
                    "input_modalities": ["text", "image"],
                    "supports_parallel_tool_calls": true
                },
                {"slug": "hidden-one", "visibility": "hidden", "supported_in_api": true, "context_window": 1}
            ]
        });

        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer test-access"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(serde_json::to_string(&codex_body).unwrap()),
            )
            .mount(&server)
            .await;

        let provider = std::sync::Arc::new(TestOAuthProvider {
            base_url: server.uri(),
        });
        let tokens = mew_provider::auth::TokenSet {
            access_token: "test-access".to_string(),
            refresh_token: "test-refresh".to_string(),
            // Far-future expiry so refresh_if_needed is a no-op (no file IO).
            expires_at: 9_999_999_999,
        };
        let adapter = Adapter::new_oauth(
            "test".to_string(),
            "gpt-5.6-sol".to_string(),
            tokens,
            vec![],
            provider,
        );

        let _guard = CodexCacheRestore::new(mew_catalog::codex_cache_path());

        let models = adapter.list_models().await.unwrap();
        // The hidden model is filtered out; only the visible one returns.
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-5.6-sol");
        assert_eq!(models[0].owned_by, "openai");

        // The live response refreshed the codex catalog cache. The cache holds
        // the raw body; filtering happens at parse time (proven above).
        let cached = std::fs::read_to_string(mew_catalog::codex_cache_path()).unwrap();
        assert!(cached.contains("gpt-5.6-sol"));
    }

    #[tokio::test]
    async fn test_list_models_oauth_non_2xx_returns_err() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let provider = std::sync::Arc::new(TestOAuthProvider {
            base_url: server.uri(),
        });
        let tokens = mew_provider::auth::TokenSet {
            access_token: "test-access".to_string(),
            refresh_token: "test-refresh".to_string(),
            expires_at: 9_999_999_999,
        };
        let adapter = Adapter::new_oauth(
            "test".to_string(),
            "gpt-5.6-sol".to_string(),
            tokens,
            vec![],
            provider,
        );

        let _guard = CodexCacheRestore::new(mew_catalog::codex_cache_path());
        let result = adapter.list_models().await;
        assert!(result.is_err(), "non-2xx should surface an error");
    }
}
