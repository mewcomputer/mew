//! Web search tool: query a search provider (Tavily or Exa) and return ranked
//! results. One tool name, two backends; the provider is chosen in
//! `config.toml` (`[web_search] provider`) and every provider-specific wire
//! detail stays in this module.
//!
//! Each call tries the configured provider first and falls back to the other
//! one when it has a key. Failures never carry over: the next call starts
//! from the configured provider again.

use crate::{Sensitivity, Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Search backends behind the single `web_search` tool. Serialized lowercase
/// in `config.toml` and on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchProvider {
    #[default]
    Tavily,
    Exa,
}

impl SearchProvider {
    pub const ALL: [SearchProvider; 2] = [SearchProvider::Tavily, SearchProvider::Exa];

    pub fn id(self) -> &'static str {
        match self {
            SearchProvider::Tavily => "tavily",
            SearchProvider::Exa => "exa",
        }
    }

    /// Credential reference resolved through the standard chain:
    /// `MEW_CRED_<REF>`, the `mew` keyring, then `credentials.json`.
    pub fn credential_ref(self) -> &'static str {
        self.id()
    }

    pub fn parse_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.id() == id)
    }

    fn endpoint(self) -> &'static str {
        match self {
            SearchProvider::Tavily => "https://api.tavily.com/search",
            SearchProvider::Exa => "https://api.exa.ai/search",
        }
    }
}

/// One usable backend: a provider and the api key resolved for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAuth {
    pub provider: SearchProvider,
    pub api_key: String,
}

impl ProviderAuth {
    pub fn new(provider: SearchProvider, api_key: String) -> Self {
        Self { provider, api_key }
    }
}

/// Uniform cap across providers so one call cannot flood the context window.
/// Tavily enforces the same ceiling; Exa allows more, but the tool does not.
pub const MAX_RESULTS_LIMIT: u8 = 20;
/// Total output cap; matches `web_fetch`.
const MAX_CONTENT_CHARS: usize = 128_000;
/// Per-result text cap for providers that extract page contents on request.
const EXA_TEXT_CHARS: usize = 4_000;
const TIME_RANGES: [&str; 4] = ["day", "week", "month", "year"];

pub struct WebSearch {
    /// Tried in order: the configured provider, then any other with a key.
    chain: Vec<ProviderAuth>,
    default_max_results: u8,
}

impl WebSearch {
    pub fn new(chain: Vec<ProviderAuth>, default_max_results: u8) -> Self {
        assert!(!chain.is_empty(), "web_search needs at least one provider");
        Self {
            chain,
            default_max_results: default_max_results.clamp(1, MAX_RESULTS_LIMIT),
        }
    }

    pub fn primary(&self) -> SearchProvider {
        self.chain[0].provider
    }

    /// Providers in attempt order, primarily so callers can report which
    /// backends are usable.
    pub fn chain(&self) -> &[ProviderAuth] {
        &self.chain
    }
}

/// Everything provider-specific about one search, built before any I/O.
struct Request {
    url: &'static str,
    /// `(header name, value)` pairs carrying the api key.
    auth: (&'static str, String),
    body: Value,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    #[serde(default)]
    title: String,
    url: String,
    /// Tavily's extracted snippet.
    #[serde(default)]
    content: String,
    /// Exa's extracted page text.
    #[serde(default)]
    text: String,
    /// Exa's relevant passages, used when `text` is absent.
    #[serde(default)]
    highlights: Vec<String>,
}

impl SearchResult {
    fn snippet(&self) -> String {
        if !self.content.is_empty() {
            self.content.clone()
        } else if !self.text.is_empty() {
            self.text.clone()
        } else {
            self.highlights.join(" … ")
        }
    }
}

/// Approximate `time_range` as a day count. Months have no fixed length; 30
/// days is close enough for a publication-date filter.
fn start_published_date(time_range: &str) -> String {
    let days = match time_range {
        "day" => 1,
        "week" => 7,
        "month" => 30,
        _ => 365,
    };
    (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
}

fn build_body(
    provider: SearchProvider,
    query: &str,
    max_results: u8,
    time_range: Option<&str>,
) -> Value {
    match provider {
        // `search_depth` is pinned to `basic` (one credit per call) so the
        // tool cannot accidentally spend double.
        SearchProvider::Tavily => {
            let mut body = serde_json::json!({
                "query": query,
                "search_depth": "basic",
                "include_answer": false,
                "max_results": max_results,
            });
            if let Some(range) = time_range {
                body["time_range"] = Value::String(range.to_owned());
            }
            body
        }
        // `type: "auto"` is Exa's balanced default; text is requested so the
        // agent gets the same snippet shape Tavily returns.
        SearchProvider::Exa => {
            let mut body = serde_json::json!({
                "query": query,
                "numResults": max_results,
                "type": "auto",
                "contents": { "text": { "maxCharacters": EXA_TEXT_CHARS } },
            });
            if let Some(range) = time_range {
                body["startPublishedDate"] = Value::String(start_published_date(range));
            }
            body
        }
    }
}

fn build_request(
    provider: SearchProvider,
    api_key: &str,
    query: &str,
    max_results: u8,
    time_range: Option<&str>,
) -> Request {
    let auth = match provider {
        SearchProvider::Tavily => ("Authorization", format!("Bearer {api_key}")),
        SearchProvider::Exa => ("x-api-key", api_key.to_owned()),
    };
    Request {
        url: provider.endpoint(),
        auth,
        body: build_body(provider, query, max_results, time_range),
    }
}

fn format_results(query: &str, results: &[SearchResult], provider: SearchProvider) -> String {
    if results.is_empty() {
        return format!("No results found for: {query}");
    }
    let mut out = format!(
        "Search: {query} ({} results via {})\n\n",
        results.len(),
        provider.id()
    );
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, r.title, r.url));
        let snippet = r.snippet().trim().to_owned();
        if !snippet.is_empty() {
            out.push_str(&format!("   {snippet}\n\n"));
        }
    }
    out
}

fn truncate(content: String) -> String {
    if content.chars().count() <= MAX_CONTENT_CHARS {
        return content;
    }
    let head: String = content.chars().take(MAX_CONTENT_CHARS).collect();
    format!("{head}\n\n[... content truncated at {MAX_CONTENT_CHARS} chars ...]")
}

/// Translate a non-success HTTP status into an actionable error. 401/403 mean
/// the key is wrong; 402, 429, and Tavily's plan-limit code 432 all surface
/// as quota problems for the user.
fn map_status(provider: SearchProvider, status: u16) -> ToolError {
    let id = provider.id();
    match status {
        401 | 403 => ToolError::Execution(format!("{id} rejected the api key")),
        402 | 429 | 432 => ToolError::Execution(format!("{id} quota or rate limit reached")),
        other => ToolError::Execution(format!("HTTP {other}")),
    }
}

/// Try each configured provider in order until one answers. Returns either
/// the winner with its formatted output, or every failure in attempt order.
/// Kept separate from `execute` so fallback behavior can be tested without
/// touching the network.
async fn run_chain<F, Fut>(
    chain: &[ProviderAuth],
    mut attempt: F,
) -> Result<(SearchProvider, String), Vec<(SearchProvider, ToolError)>>
where
    F: FnMut(SearchProvider, String) -> Fut,
    Fut: std::future::Future<Output = Result<String, ToolError>>,
{
    let mut failures = Vec::new();
    for auth in chain {
        match attempt(auth.provider, auth.api_key.clone()).await {
            Ok(output) => return Ok((auth.provider, output)),
            Err(error) => failures.push((auth.provider, error)),
        }
    }
    Err(failures)
}

fn error_detail(error: &ToolError) -> String {
    match error {
        ToolError::Execution(message) | ToolError::InvalidInput(message) => message.clone(),
        other => other.to_string(),
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web and return ranked results with titles, URLs, and extracted \
         content. Use for current documentation, library APIs, error messages, and \
         anything where the exact URL is not yet known. Follow up with web_fetch to \
         read a result in full."
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results to return (1-20).",
                        "minimum": 1,
                        "maximum": 20
                    },
                    "time_range": {
                        "type": "string",
                        "enum": ["day", "week", "month", "year"],
                        "description": "Optional publication-date filter relative to today."
                    },
                    "description": {
                        "type": "string",
                        "description": "Short (3-8 word) user-facing summary of why this search is being run."
                    }
                },
                "required": ["query"]
            })
        })
    }

    fn sensitivity(&self) -> Sensitivity {
        Sensitivity::ReadOnly
    }

    async fn execute(&self, _ctx: ToolCtx, input: Value) -> Result<ToolOutput, ToolError> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| ToolError::InvalidInput("missing or empty 'query' field".into()))?;

        let max_results = match input.get("max_results") {
            None | Some(Value::Null) => self.default_max_results,
            Some(v) => v
                .as_u64()
                .map(|n| n.clamp(1, u64::from(MAX_RESULTS_LIMIT)) as u8)
                .ok_or_else(|| {
                    ToolError::InvalidInput("'max_results' must be a positive integer".into())
                })?,
        };

        let time_range = match input.get("time_range") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_str() {
                Some(range) if TIME_RANGES.contains(&range) => Some(range),
                _ => {
                    return Err(ToolError::InvalidInput(
                        "'time_range' must be one of day, week, month, year".into(),
                    ));
                }
            },
        };

        let result = run_chain(&self.chain, |provider, api_key| async move {
            let request = build_request(provider, &api_key, query, max_results, time_range);
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| ToolError::Execution(format!("failed to build HTTP client: {e}")))?;

            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
            let auth_name = reqwest::header::HeaderName::from_bytes(request.auth.0.as_bytes())
                .map_err(|e| ToolError::Execution(format!("invalid auth header name: {e}")))?;
            let mut auth_value = reqwest::header::HeaderValue::from_str(&request.auth.1)
                .map_err(|_| ToolError::Execution("api key is not a valid header value".into()))?;
            // Keep the key out of any debug formatting of the request headers.
            auth_value.set_sensitive(true);
            headers.insert(auth_name, auth_value);

            let response =
                client
                    .post(request.url)
                    .headers(headers)
                    .header(
                        "User-Agent",
                        "mew/0.1 (terminal coding assistant; +https://github.com/natalie/mew)",
                    )
                    .body(serde_json::to_vec(&request.body).map_err(|e| {
                        ToolError::Execution(format!("failed to encode request: {e}"))
                    })?)
                    .send()
                    .await
                    .map_err(|e| ToolError::Execution(format!("request failed: {e}")))?;

            let status = response.status();
            if !status.is_success() {
                return Err(map_status(provider, status.as_u16()));
            }
            let body = response
                .text()
                .await
                .map_err(|e| ToolError::Execution(format!("failed to read response body: {e}")))?;
            let parsed: SearchResponse = serde_json::from_str(&body).map_err(|e| {
                ToolError::Execution(format!("unexpected {provider:?} response: {e}"))
            })?;
            Ok(format_results(query, &parsed.results, provider))
        })
        .await;

        match result {
            Ok((_provider, output)) => Ok(ToolOutput {
                output: truncate(output),
                error: String::new(),
                diff: None,
                ..Default::default()
            }),
            Err(failures) => {
                let details = failures
                    .iter()
                    .map(|(provider, error)| format!("{}: {}", provider.id(), error_detail(error)))
                    .collect::<Vec<_>>()
                    .join("; ");
                Err(ToolError::Execution(format!(
                    "all search providers failed: {details}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "secret-key";

    fn auth(provider: SearchProvider) -> ProviderAuth {
        ProviderAuth::new(provider, format!("{}-key", provider.id()))
    }

    #[test]
    fn provider_ids_roundtrip_through_serde() {
        for provider in SearchProvider::ALL {
            assert_eq!(SearchProvider::parse_id(provider.id()), Some(provider));
            let json = serde_json::to_string(&provider).unwrap();
            assert_eq!(json, format!("\"{}\"", provider.id()));
            assert_eq!(
                serde_json::from_str::<SearchProvider>(&json).unwrap(),
                provider
            );
            assert_eq!(provider.credential_ref(), provider.id());
        }
        assert_eq!(SearchProvider::parse_id("brave"), None);
        assert!(serde_json::from_str::<SearchProvider>("\"Tavily\"").is_err());
    }

    #[test]
    fn constructor_clamps_default_results_and_keeps_chain_order() {
        let tool = WebSearch::new(
            vec![auth(SearchProvider::Exa), auth(SearchProvider::Tavily)],
            99,
        );
        assert_eq!(tool.primary(), SearchProvider::Exa);
        assert_eq!(tool.chain.len(), 2);
        assert_eq!(tool.default_max_results, MAX_RESULTS_LIMIT);
        assert_eq!(
            WebSearch::new(vec![auth(SearchProvider::Tavily)], 0).default_max_results,
            1
        );
    }

    #[test]
    #[should_panic(expected = "at least one provider")]
    fn constructor_rejects_an_empty_chain() {
        WebSearch::new(Vec::new(), 5);
    }

    #[test]
    fn schema_requires_only_query() {
        let tool = WebSearch::new(vec![auth(SearchProvider::Exa)], 5);
        assert_eq!(tool.schema()["required"], serde_json::json!(["query"]));
    }

    #[test]
    fn tavily_request_uses_bearer_and_basic_depth() {
        let request = build_request(SearchProvider::Tavily, KEY, "rust async trait", 5, None);
        assert_eq!(request.url, "https://api.tavily.com/search");
        assert_eq!(request.auth, ("Authorization", "Bearer secret-key".into()));
        assert_eq!(request.body["search_depth"], "basic");
        assert_eq!(request.body["include_answer"], false);
        assert_eq!(request.body["max_results"], 5);
        assert!(request.body.get("time_range").is_none());
    }

    #[test]
    fn tavily_request_passes_time_range_through() {
        let request = build_request(SearchProvider::Tavily, KEY, "q", 3, Some("week"));
        assert_eq!(request.body["max_results"], 3);
        assert_eq!(request.body["time_range"], "week");
    }

    #[test]
    fn exa_request_uses_x_api_key_and_requests_text() {
        let request = build_request(SearchProvider::Exa, KEY, "q", 7, None);
        assert_eq!(request.url, "https://api.exa.ai/search");
        assert_eq!(request.auth, ("x-api-key", "secret-key".into()));
        assert_eq!(request.body["numResults"], 7);
        assert_eq!(request.body["type"], "auto");
        assert_eq!(
            request.body["contents"]["text"]["maxCharacters"],
            EXA_TEXT_CHARS
        );
        assert!(request.body.get("startPublishedDate").is_none());
    }

    #[test]
    fn exa_time_range_becomes_an_iso_date_in_the_past() {
        let body = build_body(SearchProvider::Exa, "q", 5, Some("day"));
        let date =
            chrono::DateTime::parse_from_rfc3339(body["startPublishedDate"].as_str().unwrap())
                .unwrap();
        let age = chrono::Utc::now().signed_duration_since(date);
        assert!(age.num_hours() >= 23 && age.num_hours() <= 25, "got {age}");
    }

    #[test]
    fn parses_tavily_snippets_and_exa_text_or_highlights() {
        let tavily: SearchResponse = serde_json::from_str(
            r#"{"results":[{"title":"A","url":"https://a","content":"snippet"},{"url":"https://b"}]}"#,
        )
        .unwrap();
        assert_eq!(tavily.results[0].snippet(), "snippet");
        assert_eq!(tavily.results[1].snippet(), "");

        let exa: SearchResponse = serde_json::from_str(
            r#"{"requestId":"r","results":[{"title":"A","url":"https://a","text":"full text"},{"title":"B","url":"https://b","highlights":["one","two"]}]}"#,
        )
        .unwrap();
        assert_eq!(exa.results[0].snippet(), "full text");
        assert_eq!(exa.results[1].snippet(), "one … two");
    }

    #[test]
    fn format_results_names_the_serving_provider_and_reports_empty() {
        let results = vec![SearchResult {
            title: "Async Traits".into(),
            url: "https://example.com/a".into(),
            content: "  trait internals  ".into(),
            text: String::new(),
            highlights: Vec::new(),
        }];
        let out = format_results("q", &results, SearchProvider::Exa);
        assert!(out.starts_with("Search: q (1 results via exa)"));
        assert!(out.contains("1. Async Traits\n   https://example.com/a\n   trait internals\n"));
        assert_eq!(
            format_results("nothing", &[], SearchProvider::Tavily),
            "No results found for: nothing"
        );
    }

    #[test]
    fn status_mapping_names_the_provider() {
        for provider in SearchProvider::ALL {
            let id = provider.id();
            assert!(
                matches!(map_status(provider, 401), ToolError::Execution(m) if m == format!("{id} rejected the api key"))
            );
            assert!(
                matches!(map_status(provider, 402), ToolError::Execution(m) if m.contains("quota"))
            );
            assert!(
                matches!(map_status(provider, 432), ToolError::Execution(m) if m.contains("quota"))
            );
            assert!(
                matches!(map_status(provider, 500), ToolError::Execution(m) if m == "HTTP 500")
            );
        }
    }

    #[tokio::test]
    async fn chain_falls_back_only_after_a_failure() {
        let chain = vec![
            auth(SearchProvider::Tavily),
            auth(SearchProvider::Exa),
            auth(SearchProvider::Tavily),
        ];
        let mut attempted = Vec::new();
        let result = run_chain(&chain, |provider, _key| {
            attempted.push(provider);
            async move {
                if provider == SearchProvider::Tavily {
                    Err(ToolError::Execution("tavily quota".into()))
                } else {
                    Ok("via exa".into())
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), (SearchProvider::Exa, "via exa".into()));
        assert_eq!(
            attempted,
            vec![SearchProvider::Tavily, SearchProvider::Exa],
            "the chain stops at the first success"
        );
    }

    #[tokio::test]
    async fn chain_reports_every_failure_when_none_work() {
        let chain = vec![auth(SearchProvider::Tavily), auth(SearchProvider::Exa)];
        let failures = run_chain(&chain, |provider, key| async move {
            Err(ToolError::Execution(format!("{provider:?} rejected {key}")))
        })
        .await
        .unwrap_err();
        let detail_pairs: Vec<_> = failures
            .iter()
            .map(|(provider, error)| (provider.id(), error_detail(error)))
            .collect();
        assert_eq!(
            detail_pairs,
            vec![
                ("tavily", "Tavily rejected tavily-key".to_owned()),
                ("exa", "Exa rejected exa-key".to_owned()),
            ]
        );
        let details = detail_pairs
            .iter()
            .map(|(provider, detail)| format!("{provider}: {detail}"))
            .collect::<Vec<_>>()
            .join("; ");
        assert_eq!(
            details,
            "tavily: Tavily rejected tavily-key; exa: Exa rejected exa-key"
        );
    }

    fn ctx() -> ToolCtx {
        ToolCtx::test_new(std::path::PathBuf::from("."))
    }

    #[tokio::test]
    async fn rejects_bad_input_before_any_network_call() {
        let tool = WebSearch::new(vec![auth(SearchProvider::Exa)], 5);
        let err = tool
            .execute(ctx(), serde_json::json!({"query": "   "}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(m) if m.contains("query")));

        let err = tool
            .execute(
                ctx(),
                serde_json::json!({"query": "q", "max_results": "many"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(m) if m.contains("max_results")));

        let err = tool
            .execute(
                ctx(),
                serde_json::json!({"query": "q", "time_range": "decade"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput(m) if m.contains("time_range")));
    }
}
