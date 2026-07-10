use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

const CATALOG_URL: &str = "https://models.dev/api.json";
/// Umans's public model info endpoint. Returns per-model context windows,
/// capabilities, and reasoning config — used in place of models.dev catalog
/// entries for umans-served models (umans is not in the public models.dev
/// catalog).
const UMANS_MODELS_URL: &str = "https://api.code.umans.ai/v1/models/info";
/// OpenAI's official Codex model catalog (the same `models.json` the codex
/// CLI bundles as a cache fallback for the authed `/models` endpoint). Used
/// to surface ChatGPT-subscription (OAuth) models in the picker, since they
/// aren't in models.dev. Tracked on `main`; parsing ignores unknown fields so
/// shape drift degrades gracefully.
const CODEX_MODELS_URL: &str =
    "https://raw.githubusercontent.com/openai/codex/main/codex-rs/models-manager/models.json";
const CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Error, Debug)]
pub enum CatalogError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("network error: {0}")]
    Network(String),
    #[error("fetch error: status {0}")]
    FetchStatus(u16),
}

/// Per-token cost info.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Pricing {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(rename = "cache_read", default)]
    pub cache_read: f64,
    #[serde(rename = "cache_write", default)]
    pub cache_write: f64,
    #[serde(default)]
    pub reasoning: f64,
}

/// A single model entry from the models.dev catalog.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub provider: String,
    #[serde(rename = "context_window", default)]
    pub context_window: i64,
    #[serde(rename = "max_output", default)]
    pub max_output: i64,
    #[serde(rename = "tool_call", default)]
    pub tool_call: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub shape: String,
    #[serde(default)]
    pub pricing: Pricing,
    /// User-defined thinking variant overrides (from config).
    /// When empty, built-in defaults are used.
    #[serde(default)]
    pub thinking_variants: Vec<ThinkingVariant>,
    /// True for OpenAI Codex models that use the Responses Lite transport
    /// (e.g. gpt-5.6-sol/terra/luna). Drives request-body/ header differences
    /// in the responses adapter.
    #[serde(default)]
    pub responses_lite: bool,
    /// Codex multi-agent schema version, if any. Stored for future use;
    /// the current agent loop does not act on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_agent_version: Option<String>,
}

/// A named thinking/reasoning variant for a model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThinkingVariant {
    pub name: String,
    /// Provider-specific params to merge into the request body.
    /// E.g. OpenAI: `{"reasoning_effort": "high"}`
    /// Anthropic: `{"thinking": {"type": "enabled", "budget_tokens": 16000}}`
    #[serde(default)]
    pub params: serde_json::Value,
}

/// The loaded model registry.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub models: HashMap<String, Model>,
}

impl Catalog {
    /// Creates an empty catalog (for fallback when loading fails).
    pub fn empty() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    /// Returns a model by ID, if known.
    pub fn lookup(&self, id: &str) -> Option<&Model> {
        self.models.get(id)
    }

    /// Returns the adapter shape for a model ID.
    /// Falls back to `"openai"` if the model is unknown.
    pub fn shape_for(&self, id: &str) -> &str {
        self.models
            .get(id)
            .and_then(|m| {
                if m.shape.is_empty() {
                    None
                } else {
                    Some(m.shape.as_str())
                }
            })
            .unwrap_or("openai")
    }

    /// Returns the context window size for a model ID.
    /// Falls back to 128_000 if unknown.
    pub fn context_window(&self, id: &str) -> i64 {
        self.models
            .get(id)
            .and_then(|m| {
                if m.context_window > 0 {
                    Some(m.context_window)
                } else {
                    None
                }
            })
            .unwrap_or(128_000)
    }

    /// Returns the model's max output tokens, or `None` if unknown.
    /// Unlike `context_window`, this returns `Option` rather than a
    /// hard-coded fallback — an unknown `max_output` is meaningfully
    /// different from a known 128K, and the caller may want to fall
    /// back to its own default (or pass through to the provider).
    /// Follows the same "0 means unknown" convention as the field.
    pub fn max_output(&self, id: &str) -> Option<i64> {
        self.models.get(id).and_then(|m| {
            if m.max_output > 0 {
                Some(m.max_output)
            } else {
                None
            }
        })
    }

    /// Reports whether a model supports image input.
    pub fn supports_vision(&self, id: &str) -> bool {
        self.models.get(id).map(|m| m.vision).unwrap_or(false)
    }

    /// Reports whether a model supports tool calling.
    pub fn supports_tool_call(&self, id: &str) -> bool {
        self.models.get(id).map(|m| m.tool_call).unwrap_or(false)
    }

    /// Reports whether a model supports reasoning blocks.
    pub fn supports_reasoning(&self, id: &str) -> bool {
        self.models.get(id).map(|m| m.reasoning).unwrap_or(false)
    }

    /// Merges local model entries, overriding any catalog entries with the same ID.
    pub fn merge_local(&mut self, models: Vec<Model>) {
        for m in models {
            self.models.insert(m.id.clone(), m);
        }
    }

    /// Returns available thinking variants for a model.
    ///
    /// If the model has user-defined variants (from config), those are returned.
    /// Otherwise, built-in defaults are computed from the model ID.
    pub fn thinking_variants(&self, model_id: &str) -> Vec<ThinkingVariant> {
        if let Some(m) = self.models.get(model_id) {
            if !m.thinking_variants.is_empty() {
                return m.thinking_variants.clone();
            }
        }
        Self::builtin_thinking_variants(model_id)
    }

    /// Returns the default thinking variant for a model, if any.
    ///
    /// This is the variant that should be used when the user doesn't specify one.
    /// For models with effort levels, typically "high" or "medium".
    /// For boolean models, the "thinking" variant.
    pub fn default_thinking(&self, model_id: &str) -> Option<ThinkingVariant> {
        let variants = self.thinking_variants(model_id);
        if variants.is_empty() {
            return None;
        }
        // Prefer "high" as default for effort-based models,
        // "thinking" for boolean models, otherwise first variant.
        variants
            .iter()
            .find(|v| v.name == "high")
            .or_else(|| variants.iter().find(|v| v.name == "thinking"))
            .or_else(|| variants.first())
            .cloned()
    }

    /// Built-in thinking variant defaults based on model ID patterns.
    ///
    /// Returns an empty vec for models that don't support configurable thinking.
    fn builtin_thinking_variants(model_id: &str) -> Vec<ThinkingVariant> {
        let id = model_id.to_lowercase();

        // Models with configurable thinking (checked first)

        // DeepSeek v4: high and max
        if id.contains("deepseek") && id.contains("v4") {
            return vec![
                ThinkingVariant {
                    name: "high".into(),
                    params: serde_json::json!({"reasoning_effort": "high"}),
                },
                ThinkingVariant {
                    name: "max".into(),
                    params: serde_json::json!({"reasoning_effort": "max"}),
                },
            ];
        }

        // GLM 5.2 only: high and max (other GLM versions have no thinking)
        if id.contains("glm") && (id.contains("5.2") || id.contains("5-2")) {
            return vec![
                ThinkingVariant {
                    name: "high".into(),
                    params: serde_json::json!({"reasoning_effort": "high"}),
                },
                ThinkingVariant {
                    name: "max".into(),
                    params: serde_json::json!({"reasoning_effort": "max"}),
                },
            ];
        }

        // GLM 5/5.1: boolean thinking toggle
        if id.contains("glm")
            && (id.contains("5.1")
                || id.contains("5-1")
                || (!id.contains("4") && !id.contains("3")))
        {
            return vec![ThinkingVariant {
                name: "thinking".into(),
                params: serde_json::json!({"thinking": {"type": "enabled"}}),
            }];
        }

        // MiMo v2.5: boolean thinking toggle
        if id.contains("mimo") && id.contains("2.5") {
            return vec![ThinkingVariant {
                name: "thinking".into(),
                params: serde_json::json!({"thinking": {"type": "enabled"}}),
            }];
        }

        // MiniMax M3: thinking on/off toggle
        if id.contains("minimax") && id.contains("m3") {
            return vec![
                ThinkingVariant {
                    name: "thinking".into(),
                    params: serde_json::json!({"thinking": {"type": "adaptive"}}),
                },
                ThinkingVariant {
                    name: "none".into(),
                    params: serde_json::json!({"thinking": {"type": "disabled"}}),
                },
            ];
        }

        // Models with no configurable thinking
        if id.contains("deepseek")
            || id.contains("glm")
            || id.contains("kimi")
            || id.contains("k2")
            || id.contains("qwen")
            || id.contains("big-pickle")
            || id.contains("mimo")
            || id.contains("minimax")
            || id.contains("nemotron")
            || id.contains("north-mini")
        {
            return Vec::new();
        }

        // Grok: only grok-3-mini variants
        if id.contains("grok") && !id.contains("mini") {
            return Vec::new();
        }

        // Anthropic models
        if id.contains("claude")
            || id.contains("opus")
            || id.contains("sonnet")
            || id.contains("haiku")
            || id.contains("fable")
        {
            // 4.7+ and Fable-5 use adaptive thinking with effort levels
            let is_adaptive = id.contains("fable")
                || id.contains("opus-4-7")
                || id.contains("opus-4.7")
                || id.contains("opus-4-8")
                || id.contains("opus-4.8");

            if is_adaptive {
                return ["low", "medium", "high", "xhigh", "max"]
                    .iter()
                    .map(|effort| ThinkingVariant {
                        name: (*effort).into(),
                        params: serde_json::json!({
                            "thinking": {"type": "adaptive"},
                            "effort": effort,
                        }),
                    })
                    .collect();
            }

            // 4.6 uses budget_tokens
            let budgets: &[(&str, u32)] = if id.contains("opus-4-6")
                || id.contains("opus-4.6")
                || id.contains("sonnet-4-6")
                || id.contains("sonnet-4.6")
            {
                &[
                    ("low", 5_000),
                    ("medium", 10_000),
                    ("high", 16_000),
                    ("max", 32_768),
                ]
            } else {
                &[("low", 5_000), ("medium", 10_000), ("high", 16_000)]
            };

            return budgets
                .iter()
                .map(|(name, budget)| ThinkingVariant {
                    name: (*name).into(),
                    params: serde_json::json!({"thinking": {"type": "enabled", "budget_tokens": budget}}),
                })
                .collect();
        }

        // OpenAI GPT-5 family
        if id.contains("gpt-5") || id.contains("codex") {
            return ["minimal", "low", "medium", "high"]
                .iter()
                .map(|e| ThinkingVariant {
                    name: (*e).into(),
                    params: serde_json::json!({"reasoning_effort": e}),
                })
                .collect();
        }

        // Grok-3-mini
        if id.contains("grok-3-mini") || id.contains("grok") && id.contains("mini") {
            return vec![
                ThinkingVariant {
                    name: "low".into(),
                    params: serde_json::json!({"reasoning_effort": "low"}),
                },
                ThinkingVariant {
                    name: "high".into(),
                    params: serde_json::json!({"reasoning_effort": "high"}),
                },
            ];
        }

        Vec::new()
    }
}

/// Fetches the catalog, using a local cache when fresh.
pub async fn load() -> Result<Catalog, CatalogError> {
    load_with_client(reqwest::Client::new()).await
}

async fn load_with_client(client: reqwest::Client) -> Result<Catalog, CatalogError> {
    let cache_dir = cache_dir();
    tokio::fs::create_dir_all(&cache_dir).await?;

    let cache_path = cache_dir.join("catalog.json");
    let etag_path = cache_dir.join("catalog.etag");

    // Try cached copy first.
    if let Ok(meta) = tokio::fs::metadata(&cache_path).await {
        if let Ok(modified) = meta.modified() {
            if modified.elapsed().unwrap_or(Duration::MAX) < CACHE_MAX_AGE {
                if let Ok(data) = tokio::fs::read(&cache_path).await {
                    debug!("using fresh cached catalog");
                    return parse(&data);
                }
            }
        }
    }

    // Fetch fresh copy.
    let mut req = client.get(CATALOG_URL);

    if let Ok(etag) = tokio::fs::read_to_string(&etag_path).await {
        req = req.header("If-None-Match", etag.trim());
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(?e, "network error fetching catalog");
            if let Ok(data) = tokio::fs::read(&cache_path).await {
                debug!("falling back to stale cache after network error");
                return parse(&data);
            }
            return Err(CatalogError::Network(e.to_string()));
        }
    };

    let status = resp.status();

    if status == reqwest::StatusCode::NOT_MODIFIED {
        if let Ok(data) = tokio::fs::read(&cache_path).await {
            debug!("catalog not modified, using cached copy");
            return parse(&data);
        }
    }

    if !status.is_success() {
        warn!(%status, "non-success status fetching catalog");
        if let Ok(data) = tokio::fs::read(&cache_path).await {
            debug!("falling back to stale cache after non-success status");
            return parse(&data);
        }
        return Err(CatalogError::FetchStatus(status.as_u16()));
    }

    // Capture ETag before consuming the body.
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let data = match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            warn!(?e, "error reading catalog body");
            if let Ok(stale) = tokio::fs::read(&cache_path).await {
                return parse(&stale);
            }
            return Err(CatalogError::Network(e.to_string()));
        }
    };

    // Write cache and etag.
    let _ = tokio::fs::write(&cache_path, &data).await;
    if let Some(etag) = etag {
        let _ = tokio::fs::write(&etag_path, etag).await;
    }

    parse(&data)
}

/// Parse a single model entry from the models.dev nested format.
/// The models.dev API uses different field names than our `Model` struct:
///   - `limit.context` → `context_window`
///   - `limit.output` → `max_output`
///   - `modalities.input` contains "image" → `vision = true`
///   - `cost` → `pricing`
///   - `provider` is the top-level key, not a field on the model
fn parse_models_dev_model(val: &serde_json::Value, provider_id: &str) -> Option<Model> {
    let id = val
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let context_window = val
        .get("limit")
        .and_then(|l| l.get("context"))
        .and_then(|c| c.as_i64())
        .unwrap_or(0);

    let max_output = val
        .get("limit")
        .and_then(|l| l.get("output"))
        .and_then(|o| o.as_i64())
        .unwrap_or(0);

    let tool_call = val
        .get("tool_call")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let reasoning = val
        .get("reasoning")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let vision = val
        .get("modalities")
        .and_then(|m| m.get("input"))
        .and_then(|i| i.as_array())
        .map(|arr| arr.iter().any(|v| v.as_str() == Some("image")))
        .unwrap_or(false);

    let pricing = val
        .get("cost")
        .map(|c| Pricing {
            input: c.get("input").and_then(|v| v.as_f64()).unwrap_or(0.0),
            output: c.get("output").and_then(|v| v.as_f64()).unwrap_or(0.0),
            cache_read: c.get("cache_read").and_then(|v| v.as_f64()).unwrap_or(0.0),
            cache_write: c.get("cache_write").and_then(|v| v.as_f64()).unwrap_or(0.0),
            reasoning: c.get("reasoning").and_then(|v| v.as_f64()).unwrap_or(0.0),
        })
        .unwrap_or_default();

    Some(Model {
        id,
        provider: provider_id.to_string(),
        context_window,
        max_output,
        tool_call,
        reasoning,
        vision,
        shape: String::new(),
        pricing,
        thinking_variants: Vec::new(),
        responses_lite: false,
        multi_agent_version: None,
    })
}

fn parse(data: &[u8]) -> Result<Catalog, CatalogError> {
    // Try object format first: {"models": [...]}
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        models: Vec<Model>,
    }

    if let Ok(payload) = serde_json::from_slice::<Payload>(data) {
        // Only accept the object format if it actually has models.
        // The models.dev catalog is `{ "provider": { "models": {...} } }`
        // which deserializes to an empty `models` Vec via `#[serde(default)]`
        // — we must fall through to the nested parser in that case.
        if !payload.models.is_empty() {
            let mut models = HashMap::with_capacity(payload.models.len());
            for m in payload.models {
                models.insert(m.id.clone(), m);
            }
            debug!(count = models.len(), "parsed catalog (object format)");
            return Ok(Catalog { models });
        }
    }

    // Try models.dev nested format: { "provider_id": { "models": { "model_id": {...} } } }
    if let Ok(providers) = serde_json::from_slice::<serde_json::Value>(data) {
        if let Some(obj) = providers.as_object() {
            // Only treat it as the nested format if at least one key has a
            // nested "models" object — this distinguishes it from other shapes.
            let is_nested = obj.values().any(|v| {
                v.get("models")
                    .and_then(|m| m.as_object())
                    .map(|m| !m.is_empty())
                    .unwrap_or(false)
            });
            if is_nested {
                let mut models = HashMap::new();
                for (provider_id, provider_val) in obj {
                    if let Some(provider_models) =
                        provider_val.get("models").and_then(|m| m.as_object())
                    {
                        for (_model_key, model_val) in provider_models {
                            if let Some(m) = parse_models_dev_model(model_val, provider_id) {
                                models.insert(m.id.clone(), m);
                            }
                        }
                    }
                }
                if !models.is_empty() {
                    debug!(
                        count = models.len(),
                        "parsed catalog (models.dev nested format)"
                    );
                    return Ok(Catalog { models });
                }
            } else if obj.is_empty() {
                // Empty object {} — return empty catalog.
                return Ok(Catalog {
                    models: HashMap::new(),
                });
            }
        }
    }

    // Fall back to array format: [...]
    let models_vec: Vec<Model> = serde_json::from_slice(data)?;
    let mut models = HashMap::with_capacity(models_vec.len());
    for m in models_vec {
        models.insert(m.id.clone(), m);
    }
    debug!(count = models.len(), "parsed catalog (array format)");
    Ok(Catalog { models })
}

/// Fetches umans's authoritative model configs from their public endpoint
/// and converts them to catalog `Model` entries ready for `Catalog::merge_local`.
///
/// The cache and ETag handling mirror `load()` — the response is treated as
/// a separate, independently cached resource because it isn't part of models.dev.
///
/// `pricing` is left at zero: umans does not publish pricing through this
/// endpoint. Surface this as a known limitation rather than guessing.
pub async fn load_umans() -> Result<Vec<Model>, CatalogError> {
    load_umans_with_client(reqwest::Client::new()).await
}

async fn load_umans_with_client(client: reqwest::Client) -> Result<Vec<Model>, CatalogError> {
    let cache_dir = cache_dir();
    tokio::fs::create_dir_all(&cache_dir).await?;
    let cache_path = cache_dir.join("catalog_umans.json");
    let etag_path = cache_dir.join("catalog_umans.etag");

    // Try cached copy first.
    if let Ok(meta) = tokio::fs::metadata(&cache_path).await {
        if let Ok(modified) = meta.modified() {
            if modified.elapsed().unwrap_or(Duration::MAX) < CACHE_MAX_AGE {
                if let Ok(data) = tokio::fs::read(&cache_path).await {
                    debug!("using fresh cached umans catalog");
                    return parse_umans(&data);
                }
            }
        }
    }

    // Fetch fresh copy.
    let mut req = client.get(UMANS_MODELS_URL);

    if let Ok(etag) = tokio::fs::read_to_string(&etag_path).await {
        req = req.header("If-None-Match", etag.trim());
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(?e, "network error fetching umans models");
            if let Ok(data) = tokio::fs::read(&cache_path).await {
                debug!("falling back to stale umans cache after network error");
                return parse_umans(&data);
            }
            return Err(CatalogError::Network(e.to_string()));
        }
    };

    let status = resp.status();

    if status == reqwest::StatusCode::NOT_MODIFIED {
        if let Ok(data) = tokio::fs::read(&cache_path).await {
            debug!("umans models not modified, using cached copy");
            return parse_umans(&data);
        }
    }

    if !status.is_success() {
        warn!(%status, "non-success status fetching umans models");
        if let Ok(data) = tokio::fs::read(&cache_path).await {
            debug!("falling back to stale umans cache after non-success status");
            return parse_umans(&data);
        }
        return Err(CatalogError::FetchStatus(status.as_u16()));
    }

    // Capture ETag before consuming the body.
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let data = match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            warn!(?e, "error reading umans models body");
            if let Ok(stale) = tokio::fs::read(&cache_path).await {
                return parse_umans(&stale);
            }
            return Err(CatalogError::Network(e.to_string()));
        }
    };

    let _ = tokio::fs::write(&cache_path, &data).await;
    if let Some(etag) = etag {
        let _ = tokio::fs::write(&etag_path, etag).await;
    }

    parse_umans(&data)
}

fn parse_umans(data: &[u8]) -> Result<Vec<Model>, CatalogError> {
    #[derive(serde::Deserialize)]
    struct UmansCapabilities {
        #[serde(default)]
        context_window: i64,
        #[serde(default)]
        max_completion_tokens: i64,
        #[serde(default)]
        supports_tools: bool,
        #[serde(default)]
        supports_vision: UmansVision,
        #[serde(default)]
        reasoning: Option<UmansReasoning>,
    }

    /// umans's `supports_vision` can be a boolean (`true`/`false`) or a string
    /// marker like `"via-handoff"` (e.g. GLM 5.x: text generated by GLM but
    /// image preprocessing routed to Kimi). Treat any truthy value as
    /// vision-capable. The `Str` payload is informational only — we don't
    /// need to read which handoff target was used.
    #[derive(serde::Deserialize, Default)]
    #[serde(untagged)]
    enum UmansVision {
        Bool(bool),
        #[allow(dead_code)]
        Str(String),
        #[default]
        None,
    }

    #[derive(serde::Deserialize)]
    struct UmansEntry {
        name: String,
        capabilities: UmansCapabilities,
    }

    let map: std::collections::HashMap<String, UmansEntry> = serde_json::from_slice(data)?;

    let mut out = Vec::with_capacity(map.len());
    for (_, entry) in map {
        let vision = matches!(
            entry.capabilities.supports_vision,
            UmansVision::Bool(true) | UmansVision::Str(_)
        );
        let model = Model {
            id: entry.name.clone(),
            provider: "umans".into(),
            context_window: entry.capabilities.context_window,
            max_output: entry.capabilities.max_completion_tokens,
            tool_call: entry.capabilities.supports_tools,
            reasoning: entry
                .capabilities
                .reasoning
                .as_ref()
                .map(|r| r.supported)
                .unwrap_or(false),
            vision,
            shape: "anthropic".into(),
            pricing: Pricing::default(),
            thinking_variants: build_umans_thinking_variants(entry.capabilities.reasoning.as_ref()),
            responses_lite: false,
            multi_agent_version: None,
        };
        out.push(model);
    }
    Ok(out)
}

/// Subset of umans's per-model `reasoning` config that's relevant for
/// building thinking variants. Hoisted to module scope so tests can construct
/// it directly.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct UmansReasoning {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    levels: Vec<String>,
}

/// Convert umans's `reasoning.levels` list into Anthropic-style thinking
/// variants.
///
/// - Empty `levels` with `can_disable: false` means the model always thinks
///   and the user can't toggle it. Emit no variants (the adapter should not
///   send a thinking param; the model decides on its own).
/// - Non-empty `levels` means the user picks an effort. Map each level to an
///   Anthropic `thinking: {type: adaptive}` + top-level `effort` pair, the
///   same shape the main catalog uses for Claude 4.7+/Fable-5.
fn build_umans_thinking_variants(reasoning: Option<&UmansReasoning>) -> Vec<ThinkingVariant> {
    let Some(r) = reasoning else {
        return Vec::new();
    };
    if !r.supported || r.levels.is_empty() {
        return Vec::new();
    }
    r.levels
        .iter()
        .map(|level| {
            let level_lower = level.to_lowercase();
            ThinkingVariant {
                name: level_lower.clone(),
                params: serde_json::json!({
                    "thinking": {"type": "adaptive"},
                    "effort": level_lower,
                }),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Codex (ChatGPT-subscription OAuth) catalog source
// ---------------------------------------------------------------------------

/// Fetches OpenAI's Codex model catalog (`models.json`) and converts it to
/// catalog `Model` entries for `Catalog::merge_local`.
///
/// The cache/ETag handling mirrors `load_umans` — the response is a separate,
/// independently cached resource. `pricing` is zero (ChatGPT subscription;
/// like opencode, cost is reported as 0 for OAuth-served models).
///
/// This is the daemon/offline baseline. The standalone TUI additionally calls
/// the authed `/models` endpoint live (see `mew-provider-responses`), which
/// refreshes this cache with the user's plan-filtered model set.
pub async fn load_codex() -> Result<Vec<Model>, CatalogError> {
    load_codex_with_client(reqwest::Client::new()).await
}

async fn load_codex_with_client(client: reqwest::Client) -> Result<Vec<Model>, CatalogError> {
    let cache_dir = cache_dir();
    tokio::fs::create_dir_all(&cache_dir).await?;
    let cache_path = codex_cache_path();
    let etag_path = cache_dir.join("catalog_codex.etag");

    // Try cached copy first.
    if let Ok(meta) = tokio::fs::metadata(&cache_path).await {
        if let Ok(modified) = meta.modified() {
            if modified.elapsed().unwrap_or(Duration::MAX) < CACHE_MAX_AGE {
                if let Ok(data) = tokio::fs::read(&cache_path).await {
                    debug!("using fresh cached codex catalog");
                    return parse_codex(&data);
                }
            }
        }
    }

    // Fetch fresh copy.
    let mut req = client.get(CODEX_MODELS_URL);

    if let Ok(etag) = tokio::fs::read_to_string(&etag_path).await {
        req = req.header("If-None-Match", etag.trim());
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(?e, "network error fetching codex models");
            if let Ok(data) = tokio::fs::read(&cache_path).await {
                debug!("falling back to stale codex cache after network error");
                return parse_codex(&data);
            }
            return Err(CatalogError::Network(e.to_string()));
        }
    };

    let status = resp.status();

    if status == reqwest::StatusCode::NOT_MODIFIED {
        if let Ok(data) = tokio::fs::read(&cache_path).await {
            debug!("codex models not modified, using cached copy");
            return parse_codex(&data);
        }
    }

    if !status.is_success() {
        warn!(%status, "non-success status fetching codex models");
        if let Ok(data) = tokio::fs::read(&cache_path).await {
            debug!("falling back to stale codex cache after non-success status");
            return parse_codex(&data);
        }
        return Err(CatalogError::FetchStatus(status.as_u16()));
    }

    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let data = match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            warn!(?e, "error reading codex models body");
            if let Ok(stale) = tokio::fs::read(&cache_path).await {
                return parse_codex(&stale);
            }
            return Err(CatalogError::Network(e.to_string()));
        }
    };

    let _ = tokio::fs::write(&cache_path, &data).await;
    if let Some(etag) = etag {
        let _ = tokio::fs::write(&etag_path, etag).await;
    }

    parse_codex(&data)
}

/// Load a Codex catalog from a local path. Used for project-level overrides
/// (`catalog_codex.json` discovered from cwd up to git root) so users can
/// ship model metadata with a repo without waiting for the network cache.
pub async fn load_codex_from_path(path: &std::path::Path) -> Result<Vec<Model>, CatalogError> {
    let data = tokio::fs::read(path).await?;
    parse_codex(&data)
}

/// Parse the Codex `models.json` / authed `/models` response into catalog
/// `Model` entries. Both sources share the `ModelsResponse { models: [...] }`
/// shape. Public so the Responses adapter can reuse it for the live `/models`
/// call.
pub fn parse_codex(data: &[u8]) -> Result<Vec<Model>, CatalogError> {
    let payload: CodexModelsResponse = serde_json::from_slice(data)?;
    let mut out = Vec::with_capacity(payload.models.len());
    for m in payload.models {
        // Only picker-visible, API-supported models. `visibility: "list"` is
        // codex's marker for "show in picker"; hidden/api-only entries are
        // dropped.
        if m.visibility != "list" || !m.supported_in_api {
            continue;
        }
        let thinking_variants = m
            .supported_reasoning_levels
            .iter()
            .map(|r| ThinkingVariant {
                name: r.effort.clone(),
                params: serde_json::json!({"reasoning_effort": r.effort}),
            })
            .collect();
        out.push(Model {
            id: m.slug,
            provider: "codex".into(),
            context_window: m.context_window,
            max_output: 0,
            tool_call: m.supports_parallel_tool_calls,
            reasoning: m.default_reasoning_level.is_some(),
            vision: m.input_modalities.iter().any(|x| x == "image"),
            shape: "responses".into(),
            pricing: Pricing::default(),
            thinking_variants,
            responses_lite: m.use_responses_lite,
            multi_agent_version: m.multi_agent_version.clone(),
        });
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
struct CodexModelsResponse {
    #[serde(default)]
    models: Vec<CodexModelInfo>,
}

#[derive(serde::Deserialize)]
struct CodexModelInfo {
    slug: String,
    #[serde(default)]
    context_window: i64,
    #[serde(default)]
    supported_reasoning_levels: Vec<CodexReasoningLevel>,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    supports_parallel_tool_calls: bool,
    #[serde(default)]
    visibility: String,
    #[serde(default)]
    supported_in_api: bool,
    #[serde(default)]
    use_responses_lite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    multi_agent_version: Option<String>,
}

#[derive(serde::Deserialize)]
struct CodexReasoningLevel {
    effort: String,
}

/// Path to the Codex catalog cache file. Exposed so the Responses adapter's
/// live `/models` fetch can refresh it (plan-filtered) for the daemon path.
pub fn codex_cache_path() -> PathBuf {
    cache_dir().join("catalog_codex.json")
}

/// Overwrite the Codex catalog cache with a fresh (typically live, authed)
/// response body. Best-effort: callers ignore the error so a failed cache
/// write never breaks a successful model fetch.
pub fn write_codex_cache(body: &str) -> Result<(), CatalogError> {
    let path = codex_cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, body)?;
    Ok(())
}

/// Returns the directory used for caching catalog files (main models.dev +
/// umans model info). Exposed so CLI commands can show or clear the cache.
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("computer", "mew", "mew")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config").join("mew"))
                .unwrap_or_else(|| PathBuf::from(".").join(".config").join("mew"))
        })
}

/// Removes all on-disk catalog cache files (main models.dev catalog + ETag +
/// umans model info + ETag + codex model info + ETag). Next launch will
/// re-fetch from the network. Returns the list of files that were removed.
pub fn clear_cache() -> Vec<PathBuf> {
    let dir = cache_dir();
    let names = [
        "catalog.json",
        "catalog.etag",
        "catalog_umans.json",
        "catalog_umans.etag",
        "catalog_codex.json",
        "catalog_codex.etag",
    ];
    let mut removed = Vec::new();
    for name in names {
        let path = dir.join(name);
        if path.exists() && std::fs::remove_file(&path).is_ok() {
            removed.push(path);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> Vec<u8> {
        br#"{"models":[{"id":"test-model","provider":"test","context_window":100000,"max_output":4096,"tool_call":true,"reasoning":false,"vision":true,"shape":"openai","pricing":{"input":0.001,"output":0.002,"cache_read":0.0005,"cache_write":0.001,"reasoning":0.0}}]}"#.to_vec()
    }

    #[test]
    fn test_parse_umans() {
        // Abbreviated sample mirroring the real umans /v1/models/info shape.
        let json = br#"{
            "umans-coder": {
                "name": "umans-coder",
                "display_name": "Umans Coder",
                "capabilities": {
                    "max_completion_tokens": 262144,
                    "recommended_max_tokens": 32768,
                    "context_window": 262144,
                    "supports_vision": true,
                    "supports_tools": true,
                    "reasoning": {
                        "supported": true,
                        "can_disable": false,
                        "levels": [],
                        "default_level": null
                    }
                }
            },
            "umans-glm-5.2": {
                "name": "umans-glm-5.2",
                "capabilities": {
                    "max_completion_tokens": 131072,
                    "context_window": 405504,
                    "supports_vision": "via-handoff",
                    "supports_tools": true,
                    "reasoning": {
                        "supported": true,
                        "can_disable": true,
                        "levels": ["none", "high", "max"],
                        "default_level": "high"
                    }
                }
            },
            "umans-flash": {
                "name": "umans-flash",
                "capabilities": {
                    "context_window": 262144,
                    "supports_vision": false,
                    "supports_tools": true,
                    "reasoning": {
                        "supported": true,
                        "can_disable": true,
                        "levels": ["none", "low", "medium", "high"]
                    }
                }
            }
        }"#;
        let models = parse_umans(json).expect("parse umans");
        assert_eq!(models.len(), 3);

        let coder = models.iter().find(|m| m.id == "umans-coder").unwrap();
        assert_eq!(coder.provider, "umans");
        assert_eq!(coder.context_window, 262_144);
        assert_eq!(coder.max_output, 262_144);
        assert!(coder.tool_call);
        assert!(coder.vision);
        assert!(coder.reasoning);
        assert_eq!(coder.shape, "anthropic");
        // Always-thinks model: no user-controllable toggle, so no variants.
        assert!(coder.thinking_variants.is_empty());

        let glm = models.iter().find(|m| m.id == "umans-glm-5.2").unwrap();
        assert_eq!(glm.context_window, 405_504);
        assert!(
            glm.vision,
            "via-handoff should still count as vision-capable"
        );
        assert_eq!(glm.shape, "anthropic");
        // Three effort levels map to three thinking variants.
        let names: Vec<&str> = glm
            .thinking_variants
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(names, vec!["none", "high", "max"]);
        // Adaptive thinking + effort param, matching the Anthropic shape used
        // by the rest of the catalog.
        assert_eq!(
            glm.thinking_variants[1].params,
            serde_json::json!({
                "thinking": {"type": "adaptive"},
                "effort": "high",
            })
        );

        let flash = models.iter().find(|m| m.id == "umans-flash").unwrap();
        assert!(!flash.vision);
        let names: Vec<&str> = flash
            .thinking_variants
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(names, vec!["none", "low", "medium", "high"]);
    }

    #[test]
    fn test_parse_umans_empty_object() {
        let json = b"{}";
        let models = parse_umans(json).expect("parse umans empty");
        assert!(models.is_empty());
    }

    #[test]
    fn test_parse_umans_malformed() {
        let json = b"not json";
        assert!(parse_umans(json).is_err());
    }

    #[test]
    fn test_build_umans_thinking_variants_none_when_not_supported() {
        let r = UmansReasoning {
            supported: false,
            levels: vec!["none".into(), "high".into()],
        };
        assert!(build_umans_thinking_variants(Some(&r)).is_empty());
    }

    #[test]
    fn test_build_umans_thinking_variants_none_when_empty_levels() {
        // Always-thinks model: levels empty, no user toggle.
        let r = UmansReasoning {
            supported: true,
            levels: vec![],
        };
        assert!(build_umans_thinking_variants(Some(&r)).is_empty());
    }

    #[test]
    fn test_build_umans_thinking_variants_lowercases_levels() {
        let r = UmansReasoning {
            supported: true,
            levels: vec!["None".into(), "High".into(), "MAX".into()],
        };
        let variants = build_umans_thinking_variants(Some(&r));
        let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["none", "high", "max"]);
    }

    #[test]
    fn test_clear_cache_removes_only_existing_files() {
        // Use a temp dir as the cache dir by creating files inside it and
        // pointing clear_cache at it via a manual loop. The public clear_cache
        // uses the global cache_dir, so this test exercises the equivalent
        // removal logic without polluting the user's actual cache.
        let tmp = tempfile::tempdir().unwrap();
        let present = tmp.path().join("catalog.json");
        let etag = tmp.path().join("catalog.etag");
        let umans = tmp.path().join("catalog_umans.json");
        let codex = tmp.path().join("catalog_codex.json");
        std::fs::write(&present, b"{}").unwrap();
        std::fs::write(&etag, b"W/\"1\"").unwrap();
        std::fs::write(&umans, b"{}").unwrap();
        std::fs::write(&codex, b"{}").unwrap();

        let names = [
            "catalog.json",
            "catalog.etag",
            "catalog_umans.json",
            "catalog_codex.json",
        ];
        let mut removed = Vec::new();
        for name in names {
            let path = tmp.path().join(name);
            if path.exists() {
                std::fs::remove_file(&path).unwrap();
                removed.push(path);
            }
        }
        assert_eq!(removed.len(), 4);
        assert!(!present.exists());
        assert!(!etag.exists());
        assert!(!umans.exists());
        assert!(!codex.exists());
        // catalog_codex.etag never existed → not in the removed list.
        let nonexistent = tmp.path().join("catalog_codex.etag");
        assert!(!nonexistent.exists());
        assert!(!removed.contains(&nonexistent));
    }

    #[test]
    fn test_parse_catalog() {
        let cat = parse(&sample_json()).unwrap();
        assert_eq!(cat.models.len(), 1);
        let m = cat.lookup("test-model").unwrap();
        assert_eq!(m.provider, "test");
        assert_eq!(m.context_window, 100_000);
        assert!(m.tool_call);
        assert!(m.vision);
        assert!(!m.reasoning);
    }

    #[test]
    fn test_lookup_unknown() {
        let cat = parse(&sample_json()).unwrap();
        assert!(cat.lookup("unknown").is_none());
    }

    #[test]
    fn test_shape_for_fallback() {
        let cat = parse(&sample_json()).unwrap();
        assert_eq!(cat.shape_for("test-model"), "openai");
        assert_eq!(cat.shape_for("unknown"), "openai");
    }

    #[test]
    fn test_context_window_fallback() {
        let cat = parse(&sample_json()).unwrap();
        assert_eq!(cat.context_window("test-model"), 100_000);
        assert_eq!(cat.context_window("unknown"), 128_000);
    }

    #[test]
    fn test_supports_methods() {
        let cat = parse(&sample_json()).unwrap();
        assert!(cat.supports_vision("test-model"));
        assert!(!cat.supports_vision("unknown"));
        assert!(cat.supports_tool_call("test-model"));
        assert!(!cat.supports_tool_call("unknown"));
        assert!(!cat.supports_reasoning("test-model"));
        assert!(!cat.supports_reasoning("unknown"));
    }

    #[test]
    fn test_parse_array_format() {
        let json = br#"[{"id":"array-model","provider":"test","context_window":100000,"max_output":4096,"tool_call":true,"reasoning":false,"vision":false,"shape":"anthropic","pricing":{"input":0.001,"output":0.002,"cache_read":0.0005,"cache_write":0.001,"reasoning":0.0}}]"#;
        let cat = parse(json).unwrap();
        assert_eq!(cat.models.len(), 1);
        let m = cat.lookup("array-model").unwrap();
        assert_eq!(m.shape, "anthropic");
    }

    #[test]
    fn test_parse_codex_maps_visible_model() {
        // Mirrors the codex models.json shape: { "models": [ { slug, ... } ] }.
        // Unknown fields (model_messages, base_instructions, …) must be ignored.
        let json = br#"{
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "display_name": "GPT-5.6-Sol",
                    "context_window": 372000,
                    "max_context_window": 372000,
                    "default_reasoning_level": "low",
                    "supported_reasoning_levels": [
                        {"effort": "low", "description": "Fast"},
                        {"effort": "high", "description": "Deep"},
                        {"effort": "ultra", "description": "Max"}
                    ],
                    "input_modalities": ["text", "image"],
                    "supports_parallel_tool_calls": true,
                    "visibility": "list",
                    "supported_in_api": true,
                    "use_responses_lite": true,
                    "multi_agent_version": "v2",
                    "base_instructions": "ignored blob",
                    "model_messages": {"instructions_template": "also ignored"}
                }
            ]
        }"#;
        let models = parse_codex(json).unwrap();
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.id, "gpt-5.6-sol");
        assert_eq!(m.provider, "codex");
        assert_eq!(m.shape, "responses");
        assert_eq!(m.context_window, 372_000);
        assert!(m.tool_call);
        assert!(m.vision);
        assert!(m.reasoning);
        assert!(m.responses_lite);
        assert_eq!(m.multi_agent_version.as_deref(), Some("v2"));
        assert_eq!(m.pricing.input, 0.0);
        // Reasoning levels → thinking variants (codex slugs no longer contain
        // "codex", so the builtin gpt-5 arm must NOT be relied on).
        let names: Vec<&str> = m
            .thinking_variants
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(names, vec!["low", "high", "ultra"]);
        assert_eq!(m.thinking_variants[0].params["reasoning_effort"], "low");
    }

    #[test]
    fn test_parse_codex_filters_hidden_and_api_only() {
        let json = br#"{
            "models": [
                {"slug": "visible", "visibility": "list", "supported_in_api": true, "context_window": 100},
                {"slug": "hidden", "visibility": "hidden", "supported_in_api": true, "context_window": 100},
                {"slug": "api-only", "visibility": "list", "supported_in_api": false, "context_window": 100}
            ]
        }"#;
        let models = parse_codex(json).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["visible"]);
    }

    #[test]
    fn test_parse_codex_empty() {
        assert!(parse_codex(b"{}").unwrap().is_empty());
        assert!(parse_codex(b"{\"models\":[]}").unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_load_codex_from_path_reads_override() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("catalog_codex.json");
        std::fs::write(
            &path,
            br#"{"models":[{"slug":"gpt-5.6-luna","context_window":372000,"visibility":"list","supported_in_api":true,"use_responses_lite":true,"supported_reasoning_levels":[{"effort":"medium"}]}]}"#,
        )
        .unwrap();
        let models = load_codex_from_path(&path).await.unwrap();
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.id, "gpt-5.6-luna");
        assert!(m.responses_lite);
        assert_eq!(m.thinking_variants.len(), 1);
        assert_eq!(m.thinking_variants[0].name, "medium");
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = parse(b"not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_object() {
        let cat = parse(b"{}").unwrap();
        assert!(cat.models.is_empty());
    }

    #[test]
    fn test_parse_empty_array() {
        let cat = parse(b"[]").unwrap();
        assert!(cat.models.is_empty());
    }

    #[test]
    fn test_parse_missing_optional_fields() {
        // Model without reasoning, vision, tool_call, shape, pricing
        let json = br#"{"models":[{"id":"minimal","provider":"test","context_window":1000,"max_output":100}]}"#;
        let cat = parse(json).unwrap();
        assert_eq!(cat.models.len(), 1);
        let m = cat.lookup("minimal").unwrap();
        assert_eq!(m.context_window, 1000);
        assert!(!m.tool_call);
        assert!(!m.vision);
        assert!(!m.reasoning);
        assert_eq!(m.shape, ""); // serde default for String
        assert_eq!(cat.shape_for("minimal"), "openai"); // fallback in lookup
    }

    #[test]
    fn test_parse_pricing_defaults() {
        let json = br#"{"models":[{"id":"no-price","provider":"test","context_window":1000,"max_output":100,"tool_call":false,"reasoning":false,"vision":false}]}"#;
        let cat = parse(json).unwrap();
        let m = cat.lookup("no-price").unwrap();
        assert_eq!(m.pricing.input, 0.0);
        assert_eq!(m.pricing.output, 0.0);
    }

    #[test]
    fn test_context_window_unknown() {
        let cat = parse(&sample_json()).unwrap();
        assert_eq!(cat.context_window("unknown-model"), 128_000);
    }

    #[test]
    fn test_supports_methods_unknown() {
        let cat = parse(&sample_json()).unwrap();
        assert!(!cat.supports_vision("unknown"));
        assert!(!cat.supports_tool_call("unknown"));
        assert!(!cat.supports_reasoning("unknown"));
    }

    #[test]
    fn test_merge_local_adds_new() {
        let mut cat = parse(&sample_json()).unwrap();
        cat.merge_local(vec![Model {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            shape: "anthropic".into(),
            context_window: 128_000,
            ..Default::default()
        }]);
        assert_eq!(cat.lookup("glm-5.3").unwrap().provider, "z-ai");
        assert_eq!(cat.shape_for("glm-5.3"), "anthropic");
        assert_eq!(cat.context_window("glm-5.3"), 128_000);
    }

    #[test]
    fn test_merge_local_overrides_existing() {
        let mut cat = parse(&sample_json()).unwrap();
        assert_eq!(cat.shape_for("test-model"), "openai");
        cat.merge_local(vec![Model {
            id: "test-model".into(),
            provider: "custom".into(),
            shape: "anthropic".into(),
            ..Default::default()
        }]);
        assert_eq!(cat.shape_for("test-model"), "anthropic");
        assert_eq!(cat.lookup("test-model").unwrap().provider, "custom");
    }

    #[test]
    fn test_thinking_variants_anthropic_opus_47() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("claude-opus-4-7");
        assert_eq!(variants.len(), 5);
        assert_eq!(variants[0].name, "low");
        assert_eq!(variants[4].name, "max");
        // 4.7+ uses adaptive thinking
        assert_eq!(
            variants[0].params["thinking"]["type"].as_str().unwrap(),
            "adaptive"
        );
        assert_eq!(variants[0].params["effort"].as_str().unwrap(), "low");
    }

    #[test]
    fn test_thinking_variants_anthropic_sonnet_46() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("claude-sonnet-4-6");
        assert_eq!(variants.len(), 4);
        assert_eq!(variants[3].name, "max");
        // xhigh should not be present for 4.6
        assert!(!variants.iter().any(|v| v.name == "xhigh"));
    }

    #[test]
    fn test_thinking_variants_openai() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("gpt-5.2");
        assert_eq!(variants.len(), 4);
        assert_eq!(variants[0].name, "minimal");
        // Params should contain reasoning_effort
        let effort = variants[0].params["reasoning_effort"].as_str().unwrap();
        assert_eq!(effort, "minimal");
    }

    #[test]
    fn test_thinking_variants_none_for_deepseek_non_v4() {
        let cat = Catalog::empty();
        assert!(cat.thinking_variants("deepseek-v3").is_empty());
        assert!(cat.thinking_variants("deepseek-r1").is_empty());
    }

    #[test]
    fn test_thinking_variants_deepseek_v4() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("deepseek-v4-flash");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].name, "high");
        assert_eq!(variants[1].name, "max");
        assert_eq!(
            variants[0].params["reasoning_effort"].as_str().unwrap(),
            "high"
        );
    }

    #[test]
    fn test_thinking_variants_glm_52() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("glm-5.2");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].name, "high");
        assert_eq!(variants[1].name, "max");
        assert_eq!(
            variants[0].params["reasoning_effort"].as_str().unwrap(),
            "high"
        );
        assert_eq!(
            variants[1].params["reasoning_effort"].as_str().unwrap(),
            "max"
        );
    }

    #[test]
    fn test_thinking_variants_glm_51_boolean() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("glm-5.1");
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].name, "thinking");
        assert_eq!(
            variants[0].params["thinking"]["type"].as_str().unwrap(),
            "enabled"
        );
    }

    #[test]
    fn test_thinking_variants_none_for_glm_4() {
        let cat = Catalog::empty();
        assert!(cat.thinking_variants("glm-4.6").is_empty());
    }

    #[test]
    fn test_thinking_variants_mimo_v25() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("mimo-v2.5");
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].name, "thinking");
        assert_eq!(
            variants[0].params["thinking"]["type"].as_str().unwrap(),
            "enabled"
        );
    }

    #[test]
    fn test_thinking_variants_minimax_m3() {
        let cat = Catalog::empty();
        let variants = cat.thinking_variants("minimax-m3");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].name, "thinking");
        assert_eq!(
            variants[0].params["thinking"]["type"].as_str().unwrap(),
            "adaptive"
        );
        assert_eq!(variants[1].name, "none");
        assert_eq!(
            variants[1].params["thinking"]["type"].as_str().unwrap(),
            "disabled"
        );
    }

    #[test]
    fn test_thinking_variants_config_override() {
        let mut cat = Catalog::empty();
        cat.merge_local(vec![Model {
            id: "custom-model".into(),
            thinking_variants: vec![ThinkingVariant {
                name: "turbo".into(),
                params: serde_json::json!({"reasoning_effort": "high"}),
            }],
            ..Default::default()
        }]);
        let variants = cat.thinking_variants("custom-model");
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].name, "turbo");
        assert_eq!(
            variants[0].params["reasoning_effort"].as_str().unwrap(),
            "high"
        );
    }
}
