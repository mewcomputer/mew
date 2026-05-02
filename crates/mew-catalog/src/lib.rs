use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

const CATALOG_URL: &str = "https://models.dev/api.json";
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
}

/// The loaded model registry.
#[derive(Debug, Clone)]
pub struct Catalog {
    models: HashMap<String, Model>,
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
            .and_then(|m| if m.shape.is_empty() { None } else { Some(m.shape.as_str()) })
            .unwrap_or("openai")
    }

    /// Returns the context window size for a model ID.
    /// Falls back to 128_000 if unknown.
    pub fn context_window(&self, id: &str) -> i64 {
        self.models
            .get(id)
            .and_then(|m| if m.context_window > 0 { Some(m.context_window) } else { None })
            .unwrap_or(128_000)
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
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(|s| s.to_string());

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

fn parse(data: &[u8]) -> Result<Catalog, CatalogError> {
    // Try object format first: {"models": [...]}
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        models: Vec<Model>,
    }

    if let Ok(payload) = serde_json::from_slice::<Payload>(data) {
        let mut models = HashMap::with_capacity(payload.models.len());
        for m in payload.models {
            models.insert(m.id.clone(), m);
        }
        debug!(count = models.len(), "parsed catalog (object format)");
        return Ok(Catalog { models });
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

fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("ai", "mew", "mew")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config").join("mew"))
                .unwrap_or_else(|| PathBuf::from(".").join(".config").join("mew"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> Vec<u8> {
        br#"{"models":[{"id":"test-model","provider":"test","context_window":100000,"max_output":4096,"tool_call":true,"reasoning":false,"vision":true,"shape":"openai","pricing":{"input":0.001,"output":0.002,"cache_read":0.0005,"cache_write":0.001,"reasoning":0.0}}]}"#.to_vec()
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
}
