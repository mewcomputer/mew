//! Provider-related setup functions.
//!
//! Extracted from `main.rs` as pure code motion. These resolve providers/models
//! from CLI flags and persisted state, build provider adapters, load the model
//! catalog, and wire the Auto/Auto+ classifier provider.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::time::Duration;
use tracing::warn;

use mew_catalog::Catalog;
use mew_config::{Config, ProviderConfig};
use mew_provider::Provider;
use mew_provider_anthropic::Adapter as AnthropicAdapter;
use mew_provider_openai::Adapter as OpenAIAdapter;
use mew_provider_responses::Adapter as ResponsesAdapter;

/// Static fallback for Codex models that require the Responses Lite transport.
/// The catalog is the authoritative source, but this keeps the known lite
/// models working when the catalog is stale or offline.
fn is_known_responses_lite_model(model_id: &str) -> bool {
    matches!(model_id, "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna")
}

/// Discover a project-level `catalog_codex.json` override by walking from
/// `cwd` up to the git root. Returns `None` if no override file exists.
fn discover_codex_catalog(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let candidate = d.join("catalog_codex.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        // Stop at git root so we don't walk the whole filesystem.
        if d.join(".git").is_dir() {
            break;
        }
        dir = d.parent();
    }
    None
}

/// Split a `"provider/model"` spec into `(provider_id, model_id)`.
///
/// If no `/` is present, `default_provider` is used as the provider and the
/// entire spec becomes the model id.
pub(crate) fn split_provider_model(spec: &str, default_provider: &str) -> (String, String) {
    if let Some(idx) = spec.find('/') {
        (spec[..idx].to_string(), spec[idx + 1..].to_string())
    } else {
        (default_provider.to_string(), spec.to_string())
    }
}

/// Resolve the active provider from the CLI flag, falling back to the last-used
/// provider in persisted state, then to the built-in default (`opencode-zen`).
///
/// Validates the persisted value against `cfg.providers` — if the state file
/// holds a provider name that no longer exists (e.g. a previous run wrote
/// a partial or renamed value), ignore it and use the built-in default
/// instead of crashing with "unknown provider <id>" on every launch.
pub(crate) fn resolve_provider(
    cli: Option<String>,
    state: &mew_config::State,
    cfg: &Config,
) -> String {
    cli.or_else(|| {
        let persisted = state.last_provider.as_str();
        if persisted.is_empty() || !is_known_provider(cfg, persisted) {
            None
        } else {
            Some(persisted.to_string())
        }
    })
    .unwrap_or_else(|| "opencode-zen".to_string())
}

/// Resolve the active model from the CLI flag, falling back to the last-used
/// model in persisted state.
pub(crate) fn resolve_model_opt(
    cli: Option<String>,
    state: &mew_config::State,
    cfg: &Config,
) -> Option<String> {
    cli.or_else(|| {
        let persisted = state.last_model.as_str();
        if persisted.is_empty() || !is_known_model(cfg, persisted) {
            None
        } else {
            Some(persisted.to_string())
        }
    })
}

/// True if `model_id` is a recognized model — either configured via
/// `cfg.models`, advertised by a configured provider's `/v1/models`, or
/// present in the built-in catalog of known providers.
pub(crate) fn is_known_model(cfg: &Config, model_id: &str) -> bool {
    if model_id.contains('/') {
        let (provider_part, model_part) = split_provider_model(model_id, "");
        if provider_part.is_empty() || model_part.is_empty() {
            return false;
        }
        if !is_known_provider(cfg, &provider_part) {
            return false;
        }
        // A model id with a known provider prefix is accepted as long as the
        // model itself is non-empty; the provider's list_models endpoint will
        // reject truly bogus ids at runtime.
        return !model_part.is_empty();
    }
    cfg.models.iter().any(|m| m.id == model_id)
}

/// Converts a `[[models]]` config entry into a catalog `Model`.
///
/// When `cm.merge` is set and `existing` (the catalog's current entry for
/// this id, if any) is present, unset fields on `cm` fall back to the
/// catalog's values instead of resetting to defaults — pricing and
/// capability flags survive a config entry that only overrides, say,
/// `context_window`. Without `merge`, the config entry replaces the catalog
/// entry wholesale, as before.
fn build_custom_model(
    cm: &mew_config::CustomModel,
    existing: Option<&mew_catalog::Model>,
) -> mew_catalog::Model {
    let thinking_variants: Vec<mew_catalog::ThinkingVariant> = cm
        .thinking_variants
        .iter()
        .map(|v| mew_catalog::ThinkingVariant {
            name: v.name.clone(),
            params: v.params.clone(),
        })
        .collect();

    let base = match (cm.merge, existing) {
        (true, Some(existing)) => existing.clone(),
        _ => mew_catalog::Model::default(),
    };

    mew_catalog::Model {
        id: cm.id.clone(),
        provider: cm.provider.clone(),
        shape: if cm.shape.is_empty() {
            base.shape
        } else {
            cm.shape.clone()
        },
        context_window: if cm.context_window != 0 {
            cm.context_window
        } else {
            base.context_window
        },
        thinking_variants: if thinking_variants.is_empty() {
            base.thinking_variants
        } else {
            thinking_variants
        },
        responses_lite: cm.responses_lite || base.responses_lite,
        ..base
    }
}

pub(crate) async fn load_catalog(cfg: &Config) -> Option<Catalog> {
    let mut cat = match mew_catalog::load().await {
        Ok(c) => c,
        Err(e) => {
            warn!("catalog load failed: {}", e);
            return None;
        }
    };
    let custom: Vec<mew_catalog::Model> = cfg
        .models
        .iter()
        .map(|cm| build_custom_model(cm, cat.models.get(&cm.id)))
        .collect();
    cat.merge_local(custom);

    // Umans publishes its own model configs at /v1/models/info — fetch and
    // merge them in only when the umans provider is both configured and has
    // a credential set. Without a credential, every model would be a dead
    // entry in the picker, so we hide the whole provider until a key shows up.
    if provider_has_credential(cfg, "umans") {
        match mew_catalog::load_umans().await {
            Ok(umans_models) => {
                tracing::info!("loaded {} umans model configs", umans_models.len());
                cat.merge_local(umans_models);
            }
            Err(e) => {
                tracing::warn!(?e, "umans models fetch failed; continuing without");
            }
        }
    } else if cfg.providers.contains_key("umans") {
        tracing::debug!("umans provider configured but no credential set; skipping model fetch");
    }

    // Codex (ChatGPT-subscription OAuth) publishes its model catalog at the
    // codex repo's models.json. Merge it in only when the user is logged in
    // via OAuth — without a token file, every model would be a dead picker
    // entry. The standalone TUI additionally refreshes this cache from the
    // live authed /models endpoint (plan-filtered) via list_models.
    if codex_logged_in() {
        match mew_catalog::load_codex().await {
            Ok(codex_models) => {
                tracing::info!("loaded {} codex model configs", codex_models.len());
                cat.merge_local(codex_models);
            }
            Err(e) => {
                tracing::warn!(?e, "codex models fetch failed; continuing without");
            }
        }
    }

    // Load a project-level Codex catalog override last so it wins. This gives
    // API-key users the same model metadata (reasoning levels, responses_lite,
    // …) as OAuth users and lets repos pin the exact model set they expect even
    // when the network cache is stale.
    let cwd = std::env::current_dir().unwrap_or_default();
    if let Some(override_path) = discover_codex_catalog(&cwd) {
        match mew_catalog::load_codex_from_path(&override_path).await {
            Ok(codex_models) => {
                tracing::info!(
                    path = %override_path.display(),
                    count = codex_models.len(),
                    "loaded local codex catalog override"
                );
                cat.merge_local(codex_models);
            }
            Err(e) => {
                tracing::warn!(path = %override_path.display(), ?e, "local codex catalog override failed");
            }
        }
    }

    Some(cat)
}

pub(crate) fn resolve_reasoning(
    cat: Option<&Catalog>,
    model_id: &str,
    variant_name: Option<&str>,
) -> Option<mew_provider::ReasoningConfig> {
    let cat = cat?;
    let variants = cat.thinking_variants(model_id);
    if variants.is_empty() {
        return None;
    }
    let variant = match variant_name {
        Some("none") => return None,
        Some(name) => variants.iter().find(|v| v.name == name)?.clone(),
        None => cat.default_thinking(model_id)?,
    };
    let params = variant.params.as_object().cloned().unwrap_or_default();
    Some(mew_provider::ReasoningConfig { params })
}

/// Return the first provider configured as a router.
///
/// Prefers a provider literally named `router`, otherwise returns the first
/// provider whose `kind` is `"router"`. Router providers are task-only and
/// cannot be selected as the main chat provider.
pub(crate) fn find_router_provider(cfg: &Config) -> Option<(String, &ProviderConfig)> {
    if let Some(pc) = cfg.providers.get("router") {
        if pc.kind == "router" {
            return Some(("router".to_string(), pc));
        }
    }
    cfg.providers
        .iter()
        .find(|(_, pc)| pc.kind == "router")
        .map(|(id, pc)| (id.clone(), pc))
}

/// Wire the Auto/Auto+ classifier provider into the agent.
///
/// If a router provider is configured, the classifier automatically uses the
/// router's `micro` tier. Otherwise, falls back to the explicit
/// `permissions.classifier_provider/classifier_model` config.
pub(crate) fn maybe_set_classifier_provider(
    agent: &mut mew_agent::Agent,
    cfg: &Config,
    cat: Option<&Catalog>,
    raw: bool,
    _active_provider_id: &str,
    _active_model_id: &str,
) {
    // If a router provider is configured, use its micro tier for classification.
    if let Some((router_id, pc)) = find_router_provider(cfg) {
        let micro_model = pc.micro_model().to_string();
        if !micro_model.is_empty() {
            let (micro_pid, micro_mid) = resolve_model(cfg, cat, &router_id, Some(micro_model));
            match build_provider(cfg, cat, &micro_pid, &micro_mid, raw) {
                Ok(provider) => {
                    agent.set_classifier_provider(provider, Some(micro_mid.clone()));
                    tracing::info!(
                        provider = %micro_pid,
                        model = %micro_mid,
                        "router micro tier configured as classifier for Auto/Auto+ modes"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to build router micro tier as classifier; Auto/Auto+ will fall through to user"
                    );
                }
            }
            return;
        }
    }

    // Legacy explicit classifier config.
    if let Some(ref provider_id) = cfg.permissions.classifier_provider {
        let model_id = cfg.permissions.classifier_model.as_deref().unwrap_or("");
        match build_provider(cfg, cat, provider_id, model_id, raw) {
            Ok(provider) => {
                agent.set_classifier_provider(provider, cfg.permissions.classifier_model.clone());
                tracing::info!(
                    provider = %provider_id,
                    model = ?cfg.permissions.classifier_model,
                    "classifier provider configured for Auto/Auto+ modes"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to build classifier provider; Auto/Auto+ will fall through to user"
                );
            }
        }
    }
}

pub(crate) fn resolve_model(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
) -> (String, String) {
    let mut provider_id = provider_flag.to_string();
    let mut model_id = model_flag.unwrap_or_default();

    if !model_id.is_empty() {
        // Check catalog first for automatic provider/shape selection.
        if let Some(c) = cat {
            if let Some(m) = c.lookup(&model_id) {
                provider_id = m.provider.clone();
            } else if let Some(idx) = model_id.find('/') {
                let candidate = &model_id[..idx];
                if is_known_provider(cfg, candidate) {
                    if provider_id == "opencode-zen" {
                        provider_id = candidate.to_string();
                    }
                    model_id = model_id[idx + 1..].to_string();
                }
            }
        } else if let Some(idx) = model_id.find('/') {
            let candidate = &model_id[..idx];
            if is_known_provider(cfg, candidate) {
                if provider_id == "opencode-zen" {
                    provider_id = candidate.to_string();
                }
                model_id = model_id[idx + 1..].to_string();
            }
        }
    }

    (provider_id, model_id)
}

pub(crate) fn is_known_provider(cfg: &Config, provider_id: &str) -> bool {
    cfg.providers.contains_key(provider_id)
}

/// Returns true if a credential is configured for the given provider.
///
/// Used to gate built-in providers on credential presence so the model picker
/// doesn't advertise models the user can't actually call. Silent on miss —
/// `get_credential` logs at debug level only, so this is cheap to call from
/// startup paths without spamming the log for users who never use a given
/// provider.
pub(crate) fn provider_has_credential(cfg: &Config, provider_id: &str) -> bool {
    match cfg.providers.get(provider_id) {
        Some(pc) => mew_config::get_credential(&pc.credential_ref).is_ok(),
        None => false,
    }
}

/// Whether a provider is usable right now: an API-key credential OR an OAuth
/// token file (for OAuth-only providers like `codex`, whose
/// credential is the token file at `auth/codex.json`, not a
/// `credential_ref` slot). Used to gate which catalog models the picker shows.
pub(crate) fn provider_available(cfg: &Config, provider_id: &str) -> bool {
    if provider_has_credential(cfg, provider_id) {
        return true;
    }
    if provider_id == "codex" {
        return mew_provider_responses::oauth::codex_token_path().exists();
    }
    false
}

/// Whether the OpenAI Responses OAuth user is logged in (token file present).
fn codex_logged_in() -> bool {
    mew_provider_responses::oauth::codex_token_path().exists()
}

pub(crate) fn provider_name_to_shape(pid: &str) -> &'static str {
    match pid {
        "opencode-zen" | "opencode-go" => "openai",
        "z-ai" | "umans" | "kimi" => "anthropic",
        "codex" => "responses",
        _ => "openai",
    }
}

/// Build a direct provider adapter from a concrete provider config.
pub(crate) fn build_direct_provider(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    pc: &ProviderConfig,
    model_override: &str,
    raw: bool,
) -> Result<Arc<dyn Provider>> {
    // Non-fatal: some providers (codex with OAuth) don't
    // need an API key. Each shape arm handles the Option as needed.
    let creds = mew_config::get_credential(&pc.credential_ref).ok();

    let model = if model_override.is_empty() {
        if cfg.default_model.is_empty() {
            "deepseek-v4-flash".to_string()
        } else {
            cfg.default_model.clone()
        }
    } else {
        model_override.to_string()
    };

    let mut shape = pc.shape.clone();
    if let Some(c) = cat {
        let s = c.shape_for(&model);
        if !s.is_empty() {
            shape = s.to_string();
        }
    }

    let mut base_url = pc.base_url.clone();
    if provider_id == "opencode-go" && model.starts_with("minimax-") {
        shape = "anthropic".to_string();
        base_url = "https://opencode.ai/zen/go/v1".to_string();
    }

    let responses_lite = cat
        .and_then(|c| c.lookup(&model))
        .map(|m| m.responses_lite)
        .unwrap_or_else(|| is_known_responses_lite_model(&model));

    match shape.as_str() {
        "openai" => {
            let creds = creds.context("get credential")?;
            let mut adapter = OpenAIAdapter::new(provider_id.to_string(), base_url, model, creds);
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        "anthropic" => {
            let creds = creds.context("get credential")?;
            let mut adapter =
                AnthropicAdapter::new(provider_id.to_string(), base_url, model, creds);
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        "responses" => {
            // Try OAuth first, then fall back to API key.
            let oauth_provider: std::sync::Arc<dyn mew_provider::auth::OAuthProvider> =
                std::sync::Arc::new(mew_provider_responses::oauth::CodexOAuth);
            match mew_provider::auth::resolve(oauth_provider.as_ref(), creds) {
                Ok(mew_provider::auth::AuthKind::OAuth {
                    tokens,
                    extra_headers,
                }) => {
                    let mut adapter = ResponsesAdapter::new_oauth(
                        provider_id.to_string(),
                        model,
                        tokens,
                        extra_headers,
                        oauth_provider,
                    )
                    .with_responses_lite(responses_lite);
                    if raw {
                        adapter.set_dump(true);
                    }
                    Ok(Arc::new(adapter))
                }
                Ok(mew_provider::auth::AuthKind::ApiKey(key)) => {
                    let mut adapter =
                        ResponsesAdapter::new(provider_id.to_string(), base_url, model, key)
                            .with_responses_lite(responses_lite);
                    if raw {
                        adapter.set_dump(true);
                    }
                    Ok(Arc::new(adapter))
                }
                Err(e) => Err(anyhow::anyhow!(
                    "no credentials for codex: {e}. \
                     Run `mew auth login codex` or set OPENAI_API_KEY."
                )),
            }
        }
        _ => anyhow::bail!("unsupported shape {} for provider {}", shape, provider_id),
    }
}

pub(crate) fn build_provider(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_id: &str,
    model_override: &str,
    raw: bool,
) -> Result<Arc<dyn Provider>> {
    let pc = cfg
        .providers
        .get(provider_id)
        .cloned()
        .with_context(|| format!("unknown provider {}", provider_id))?;

    // Router providers are task-only primitives used by subagents and the
    // permission classifier. They cannot be selected as the main chat provider.
    if pc.kind == "router" {
        anyhow::bail!(
            "provider '{}' is a router; router providers cannot be used as the main chat provider",
            provider_id
        );
    }

    build_direct_provider(cfg, cat, provider_id, &pc, model_override, raw)
}

/// Build a closure that resolves a `provider/model` string into a Provider.
/// Used as the fallback-model builder on Agent. When no `/` is present,
/// the whole string is treated as the provider name (inverted from
/// `split_provider_model` which treats it as the model).
///
/// Returns `Box<dyn Fn(&str) -> Result<Arc<dyn Provider>, String> + Send + Sync>`
/// to match `Agent::set_provider_builder`'s expected `ProviderBuilderFn` type.
#[allow(clippy::type_complexity)]
pub(crate) fn make_provider_builder(
    cfg: Config,
    cat: Option<Catalog>,
    raw: bool,
) -> Box<dyn Fn(&str) -> Result<Arc<dyn Provider>, String> + Send + Sync> {
    Box::new(move |model_str: &str| {
        let (pid, mid) = if let Some(idx) = model_str.find('/') {
            (&model_str[..idx], &model_str[idx + 1..])
        } else {
            (model_str, "")
        };
        build_provider(&cfg, cat.as_ref(), pid, mid, raw).map_err(|e| e.to_string())
    })
}

pub(crate) async fn discover_models(
    cfg: &Config,
    cat: Option<&Catalog>,
    raw: bool,
) -> Vec<(String, String)> {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    let mut models = Vec::new();

    // Collect all provider IDs to query: configured + built-in.
    let mut provider_ids: Vec<&str> = cfg.providers.keys().map(|s| s.as_str()).collect();
    for pid in &["opencode-zen", "opencode-go", "z-ai"] {
        if !provider_ids.contains(pid) {
            provider_ids.push(pid);
        }
    }

    // Query all providers concurrently with a 5s timeout per provider.
    // This prevents one slow provider from blocking startup — total time
    // is max(5s, slowest) instead of sum(all_provider_times).
    let mut join_set = tokio::task::JoinSet::new();
    for pid in provider_ids {
        let provider = match build_provider(cfg, cat, pid, "", raw) {
            Ok(p) => p,
            Err(e) => {
                warn!("discovery: failed to build provider {}: {}", pid, e);
                continue;
            }
        };
        let pid = pid.to_string();
        join_set.spawn(async move {
            match tokio::time::timeout(Duration::from_secs(5), provider.list_models()).await {
                Ok(Ok(list)) => Some((pid, list)),
                Ok(Err(e)) => {
                    warn!("discovery: provider {} list_models failed: {}", pid, e);
                    None
                }
                Err(_) => {
                    warn!("discovery: provider {} list_models timed out after 5s", pid);
                    None
                }
            }
        });
    }

    // Collect results as they complete.
    while let Some(result) = join_set.join_next().await {
        let Some((pid, list)) = result.ok().flatten() else {
            continue;
        };
        tracing::info!("discovery: provider {} returned {} models", pid, list.len());
        for m in list {
            let full_id = if m.id.contains('/') {
                m.id.clone()
            } else {
                format!("{}/{}", pid, m.id)
            };
            if seen.insert(full_id.clone()) {
                let desc = if let Some(c) = cat.and_then(|c| c.lookup(&m.id)) {
                    format!("{} · {} · {} ctx", pid, c.shape, c.context_window)
                } else {
                    let shape = provider_name_to_shape(&pid);
                    format!("{} · {}", pid, shape)
                };
                models.push((full_id, desc));
            }
        }
    }

    // Pull umans models from the catalog. umans only documents an OpenAI-shaped
    // /v1/models/info (used by load_catalog above) and does not expose an
    // Anthropic-shaped /v1/models endpoint, so `provider.list_models()` for
    // umans returns nothing. The catalog has the authoritative entries
    // (context windows, capabilities) — seed the picker from there.
    //
    // Gated on credential presence for the same reason as the catalog load:
    // no key, no picker entries.
    if provider_has_credential(cfg, "umans") {
        if let Some(c) = cat {
            for (model_id, model_info) in &c.models {
                if model_info.provider != "umans" {
                    continue;
                }
                let full_id = format!("umans/{}", model_id);
                if seen.insert(full_id.clone()) {
                    let desc = format!("umans · anthropic · {} ctx", model_info.context_window);
                    models.push((full_id, desc));
                }
            }
        }
    }

    // Pull codex (ChatGPT OAuth) models from the catalog as a baseline. The
    // live list_models call above may have already added them (plan-filtered)
    // — `seen` dedups. This seeds the picker when the live /models endpoint is
    // unreachable (offline / auth failure) so standalone matches the daemon,
    // which reads the catalog directly. Gated on OAuth login.
    if codex_logged_in() {
        if let Some(c) = cat {
            for (model_id, model_info) in &c.models {
                if model_info.provider != "codex" {
                    continue;
                }
                let full_id = format!("codex/{}", model_id);
                if seen.insert(full_id.clone()) {
                    let desc = format!("codex · responses · {} ctx", model_info.context_window);
                    models.push((full_id, desc));
                }
            }
        }
    }

    // Add hardcoded fallbacks if nothing discovered.
    if models.is_empty() {
        tracing::warn!("discovery: no models from any provider, using fallbacks");
        let mut fallbacks: Vec<(String, String)> = vec![
            (
                "opencode-zen/deepseek-v4-flash".into(),
                "opencode-zen · openai".into(),
            ),
            ("z-ai/glm-5.1".into(), "z-ai · anthropic".into()),
            (
                "opencode-go/minimax-text-01".into(),
                "opencode-go · anthropic".into(),
            ),
        ];
        // Only advertise umans in the fallback list when a credential is set.
        if provider_has_credential(cfg, "umans") {
            fallbacks.push(("umans/umans-coder".into(), "umans · anthropic".into()));
        }
        // Only advertise kimi in the fallback list when a credential is set.
        if provider_has_credential(cfg, "kimi") {
            fallbacks.push(("kimi/k3".into(), "kimi · anthropic".into()));
        }
        for (id, desc) in fallbacks {
            if seen.insert(id.clone()) {
                models.push((id, desc));
            }
        }
    }

    models.sort_by(|a, b| a.0.cmp(&b.0));
    models
}

// ---------------------------------------------------------------------------
// MainModelResolver
// ---------------------------------------------------------------------------

/// Resolves a `provider/model` string into a `Provider`. Used by the
/// subagent runner to honor per-subagent `model:` overrides. Falls back to
/// the agent's current `provider_id` when the override has no `/`.
///
/// Tier keywords (`nano`, `micro`, `deci`) are resolved against the first
/// router provider in the config, not the active chat provider.
pub(crate) struct MainModelResolver {
    pub cfg: Arc<Config>,
    pub cat: Option<Arc<Catalog>>,
    pub default_provider_id: String,
    pub router_provider_id: Option<String>,
    pub raw: bool,
}

#[async_trait]
impl mew_subagents::ModelResolver for MainModelResolver {
    async fn resolve(&self, model: &str) -> Result<Arc<dyn Provider>, String> {
        let resolved_model = self.resolve_tier_keyword(model);

        let (provider_id, model_id) =
            split_provider_model(&resolved_model, &self.default_provider_id);
        build_provider(
            &self.cfg,
            self.cat.as_deref(),
            &provider_id,
            &model_id,
            self.raw,
        )
        .map_err(|e| e.to_string())
    }
}

impl MainModelResolver {
    /// If a router provider is configured and `model` is a tier keyword,
    /// return the configured tier model ID. Falls back to the keyword itself
    /// so that literal model names still work when no router is configured.
    fn resolve_tier_keyword(&self, model: &str) -> String {
        let router_id = match self.router_provider_id.as_ref() {
            Some(id) => id,
            None => return model.to_string(),
        };
        let pc = match self.cfg.providers.get(router_id) {
            Some(pc) => pc,
            None => return model.to_string(),
        };
        match model {
            "nano" => {
                if pc.nano.is_empty() {
                    pc.micro_model().to_string()
                } else {
                    pc.nano.clone()
                }
            }
            "micro" => pc.micro_model().to_string(),
            "deci" => pc.deci_model().to_string(),
            _ => model.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- build_custom_model ---

    fn sample_catalog_model() -> mew_catalog::Model {
        mew_catalog::Model {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            context_window: 128_000,
            max_output: 8_000,
            tool_call: true,
            reasoning: true,
            vision: true,
            shape: "openai".into(),
            pricing: mew_catalog::Pricing {
                input: 1.0,
                output: 2.0,
                cache_read: 0.1,
                cache_write: 0.2,
                reasoning: 0.0,
            },
            ..Default::default()
        }
    }

    #[test]
    fn build_custom_model_replace_without_merge_flag_ignores_existing() {
        let cm = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            context_window: 64_000,
            ..Default::default()
        };
        let existing = sample_catalog_model();
        let built = build_custom_model(&cm, Some(&existing));

        assert_eq!(built.context_window, 64_000);
        // Fields not carried by CustomModel reset to defaults, losing catalog data.
        assert_eq!(built.pricing.input, 0.0);
        assert!(!built.tool_call);
        assert!(!built.reasoning);
    }

    #[test]
    fn build_custom_model_merge_keeps_unset_catalog_fields() {
        let cm = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            context_window: 64_000,
            merge: true,
            ..Default::default()
        };
        let existing = sample_catalog_model();
        let built = build_custom_model(&cm, Some(&existing));

        // Overridden field wins.
        assert_eq!(built.context_window, 64_000);
        // Unset fields fall back to the catalog entry.
        assert_eq!(built.pricing.input, 1.0);
        assert_eq!(built.pricing.output, 2.0);
        assert!(built.tool_call);
        assert!(built.reasoning);
        assert!(built.vision);
        assert_eq!(built.shape, "openai");
    }

    #[test]
    fn build_custom_model_merge_overrides_shape_when_set() {
        let cm = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            shape: "anthropic".into(),
            merge: true,
            ..Default::default()
        };
        let existing = sample_catalog_model();
        let built = build_custom_model(&cm, Some(&existing));

        assert_eq!(built.shape, "anthropic");
        // Everything else still falls back to the catalog entry.
        assert_eq!(built.context_window, 128_000);
        assert!(built.tool_call);
    }

    #[test]
    fn build_custom_model_merge_with_no_existing_entry_behaves_like_default() {
        let cm = mew_config::CustomModel {
            id: "brand-new-model".into(),
            provider: "z-ai".into(),
            context_window: 32_000,
            merge: true,
            ..Default::default()
        };
        let built = build_custom_model(&cm, None);

        assert_eq!(built.id, "brand-new-model");
        assert_eq!(built.context_window, 32_000);
        assert_eq!(built.pricing.input, 0.0);
    }

    // --- split_provider_model ---

    #[test]
    fn discover_codex_catalog_finds_file_in_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let catalog = tmp.path().join("catalog_codex.json");
        std::fs::write(&catalog, b"{}").unwrap();
        assert_eq!(discover_codex_catalog(tmp.path()), Some(catalog));
    }

    #[test]
    fn discover_codex_catalog_walks_up_to_git_root() {
        let tmp = tempfile::tempdir().unwrap();
        let git_root = tmp.path().join("repo");
        let subdir = git_root.join("nested");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::create_dir(git_root.join(".git")).unwrap();
        let catalog = git_root.join("catalog_codex.json");
        std::fs::write(&catalog, b"{}").unwrap();
        assert_eq!(discover_codex_catalog(&subdir), Some(catalog));
    }

    #[test]
    fn discover_codex_catalog_stops_at_git_root() {
        let tmp = tempfile::tempdir().unwrap();
        let git_root = tmp.path().join("repo");
        let subdir = git_root.join("nested");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::create_dir(git_root.join(".git")).unwrap();
        let outside = tmp.path().join("catalog_codex.json");
        std::fs::write(&outside, b"{}").unwrap();
        assert_eq!(discover_codex_catalog(&subdir), None);
    }

    #[test]
    fn split_provider_model_basic() {
        assert_eq!(split_provider_model("a/b", "d"), ("a".into(), "b".into()));
    }

    #[test]
    fn split_provider_model_multiple_slashes() {
        assert_eq!(
            split_provider_model("a/b/c", "d"),
            ("a".into(), "b/c".into())
        );
    }

    #[test]
    fn split_provider_model_no_slash() {
        assert_eq!(
            split_provider_model("no-slash", "d"),
            ("d".into(), "no-slash".into())
        );
    }

    #[test]
    fn split_provider_model_empty() {
        assert_eq!(split_provider_model("", "d"), ("d".into(), "".into()));
    }

    #[test]
    fn split_provider_model_trailing_slash() {
        assert_eq!(split_provider_model("a/", "d"), ("a".into(), "".into()));
    }

    // --- resolve_reasoning ---

    #[test]
    fn resolve_reasoning_none_catalog() {
        assert!(resolve_reasoning(None, "test-model", None).is_none());
    }

    #[test]
    fn resolve_reasoning_empty_variants_nonbuiltin_model() {
        // "test-model" doesn't match any builtin pattern, so thinking_variants returns empty.
        let cat = Catalog::empty();
        assert!(resolve_reasoning(Some(&cat), "test-model", None).is_none());
    }

    #[test]
    fn resolve_reasoning_finds_named_variant() {
        let mut cat = Catalog::empty();
        cat.models.insert(
            "test-model".into(),
            mew_catalog::Model {
                id: "test-model".into(),
                thinking_variants: vec![mew_catalog::ThinkingVariant {
                    name: "high".into(),
                    params: serde_json::json!({}),
                }],
                ..Default::default()
            },
        );
        let result = resolve_reasoning(Some(&cat), "test-model", Some("high"));
        assert!(result.is_some(), "should find the 'high' variant");
    }

    #[test]
    fn resolve_reasoning_none_variant_name() {
        assert!(resolve_reasoning(Some(&cat_with_variant()), "test-model", Some("none")).is_none());
    }

    #[test]
    fn resolve_reasoning_default_when_no_name() {
        let cat = cat_with_variant();
        let result = resolve_reasoning(Some(&cat), "test-model", None);
        assert!(result.is_some(), "should find the default variant");
    }

    fn cat_with_variant() -> Catalog {
        let mut cat = Catalog::empty();
        cat.models.insert(
            "test-model".into(),
            mew_catalog::Model {
                id: "test-model".into(),
                thinking_variants: vec![mew_catalog::ThinkingVariant {
                    name: "high".into(),
                    params: serde_json::json!({}),
                }],
                ..Default::default()
            },
        );
        cat
    }

    // --- resolve_tier_keyword ---

    fn make_resolver(router: Option<ProviderConfig>) -> MainModelResolver {
        let router_provider_id = router.as_ref().map(|_| "router-1".to_string());
        let mut cfg = Config::default();
        if let Some(pc) = router {
            cfg.providers.insert("router-1".into(), pc);
        }
        MainModelResolver {
            cfg: Arc::new(cfg),
            cat: None,
            default_provider_id: "default-prov".into(),
            router_provider_id,
            raw: false,
        }
    }

    #[test]
    fn resolve_tier_keyword_no_router_passthrough() {
        let resolver = make_resolver(None);
        assert_eq!(resolver.resolve_tier_keyword("nano"), "nano");
        assert_eq!(resolver.resolve_tier_keyword("anything"), "anything");
    }

    #[test]
    fn resolve_tier_keyword_nano() {
        let pc = ProviderConfig {
            kind: "router".into(),
            nano: "nano-model".into(),
            micro: "micro-model".into(),
            deci: "deci-model".into(),
            ..Default::default()
        };
        let resolver = make_resolver(Some(pc));
        assert_eq!(resolver.resolve_tier_keyword("nano"), "nano-model");
    }

    #[test]
    fn resolve_tier_keyword_nano_empty_falls_back_to_micro() {
        let pc = ProviderConfig {
            kind: "router".into(),
            nano: "".into(),
            micro: "micro-model".into(),
            deci: "deci-model".into(),
            ..Default::default()
        };
        let resolver = make_resolver(Some(pc));
        assert_eq!(resolver.resolve_tier_keyword("nano"), "micro-model");
    }

    #[test]
    fn resolve_tier_keyword_micro() {
        let pc = ProviderConfig {
            kind: "router".into(),
            nano: "nano-model".into(),
            micro: "micro-model".into(),
            deci: "deci-model".into(),
            ..Default::default()
        };
        let resolver = make_resolver(Some(pc));
        assert_eq!(resolver.resolve_tier_keyword("micro"), "micro-model");
    }

    #[test]
    fn resolve_tier_keyword_deci() {
        let pc = ProviderConfig {
            kind: "router".into(),
            nano: "nano-model".into(),
            micro: "micro-model".into(),
            deci: "deci-model".into(),
            ..Default::default()
        };
        let resolver = make_resolver(Some(pc));
        assert_eq!(resolver.resolve_tier_keyword("deci"), "deci-model");
    }

    #[test]
    fn resolve_tier_keyword_non_keyword_passthrough() {
        let pc = ProviderConfig {
            kind: "router".into(),
            nano: "nano-model".into(),
            micro: "micro-model".into(),
            deci: "deci-model".into(),
            ..Default::default()
        };
        let resolver = make_resolver(Some(pc));
        assert_eq!(
            resolver.resolve_tier_keyword("some-literal-model"),
            "some-literal-model"
        );
    }

    // --- resolve_provider ---

    fn cfg_with_default_providers() -> Config {
        Config::default()
    }

    #[test]
    fn resolve_provider_cli_wins() {
        let state = mew_config::State {
            last_provider: "opencode-zen".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(
            resolve_provider(Some("opencode-go".into()), &state, &cfg),
            "opencode-go"
        );
    }

    #[test]
    fn resolve_provider_uses_state_when_no_cli() {
        let state = mew_config::State {
            last_provider: "z-ai".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_provider(None, &state, &cfg), "z-ai");
    }

    #[test]
    fn resolve_provider_default_when_no_cli_no_state() {
        let state = mew_config::State::default();
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_provider(None, &state, &cfg), "opencode-zen");
    }

    #[test]
    fn resolve_provider_empty_state_falls_to_default() {
        let state = mew_config::State::default();
        let cfg = cfg_with_default_providers();
        // state.last_provider is empty by default
        assert_eq!(
            resolve_provider(Some("opencode-go".into()), &state, &cfg),
            "opencode-go"
        );
        assert_eq!(resolve_provider(None, &state, &cfg), "opencode-zen");
    }

    #[test]
    fn resolve_provider_ignores_unknown_state_value() {
        // State holds a provider name that isn't configured — fall back to
        // the built-in default instead of propagating the bogus value.
        // Regression for the "unknown provider t" startup crash where a
        // partial id was persisted by a prior run.
        let state = mew_config::State {
            last_provider: "t".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_provider(None, &state, &cfg), "opencode-zen");
    }

    #[test]
    fn resolve_provider_keeps_cli_even_when_state_unknown() {
        // CLI flag wins over a bogus persisted state.
        let state = mew_config::State {
            last_provider: "stale-id".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(
            resolve_provider(Some("deepseek".into()), &state, &cfg),
            "deepseek"
        );
    }

    // --- resolve_model_opt ---

    #[test]
    fn resolve_model_opt_cli_wins() {
        let state = mew_config::State {
            last_model: "opencode-zen/deepseek-v4-flash".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(
            resolve_model_opt(Some("cli-model".into()), &state, &cfg),
            Some("cli-model".into())
        );
    }

    #[test]
    fn resolve_model_opt_uses_state_when_no_cli() {
        let state = mew_config::State {
            last_model: "opencode-zen/deepseek-v4-flash".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(
            resolve_model_opt(None, &state, &cfg),
            Some("opencode-zen/deepseek-v4-flash".into())
        );
    }

    #[test]
    fn resolve_model_opt_none_when_no_cli_no_state() {
        let state = mew_config::State::default();
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_model_opt(None, &state, &cfg), None);
    }

    #[test]
    fn resolve_model_opt_none_when_state_empty() {
        let state = mew_config::State::default();
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_model_opt(None, &state, &cfg), None);
    }

    #[test]
    fn resolve_model_opt_ignores_unknown_state_value() {
        // Bare, non-empty model id with no `/` and no entry in cfg.models is
        // treated as unknown; fall through to None.
        let state = mew_config::State {
            last_model: "t".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_model_opt(None, &state, &cfg), None);
    }

    #[test]
    fn resolve_model_opt_ignores_state_with_unknown_provider_prefix() {
        // `bogus/foo` — provider part is not in cfg.providers.
        let state = mew_config::State {
            last_model: "bogus/foo".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_model_opt(None, &state, &cfg), None);
    }

    #[test]
    fn is_known_model_accepts_known_provider_prefix() {
        let cfg = cfg_with_default_providers();
        assert!(is_known_model(&cfg, "opencode-zen/deepseek-v4-flash"));
    }

    #[test]
    fn is_known_model_rejects_empty_parts() {
        let cfg = cfg_with_default_providers();
        assert!(!is_known_model(&cfg, "/foo"));
        assert!(!is_known_model(&cfg, "opencode-zen/"));
        assert!(!is_known_model(&cfg, "t"));
    }

    // --- find_router_provider ---

    #[test]
    fn find_router_provider_named_router() {
        let mut cfg = Config::default();
        cfg.providers.insert(
            "router".into(),
            ProviderConfig {
                kind: "router".into(),
                ..Default::default()
            },
        );
        let (id, _) = find_router_provider(&cfg).expect("should find router");
        assert_eq!(id, "router");
    }

    #[test]
    fn find_router_provider_named_not_kind_router_falls_through() {
        // A provider named "router" but with kind != "router" is not a router;
        // falls through to the first kind="router" provider.
        let mut cfg = Config::default();
        cfg.providers.insert(
            "router".into(),
            ProviderConfig {
                kind: "direct".into(),
                ..Default::default()
            },
        );
        cfg.providers.insert(
            "my-router".into(),
            ProviderConfig {
                kind: "router".into(),
                ..Default::default()
            },
        );
        let (id, _) = find_router_provider(&cfg).expect("should find router");
        assert_eq!(id, "my-router");
    }

    #[test]
    fn find_router_provider_none() {
        let cfg = Config::default();
        assert!(find_router_provider(&cfg).is_none());
    }

    // --- provider_name_to_shape ---

    #[test]
    fn provider_name_to_shape_known() {
        assert_eq!(provider_name_to_shape("opencode-zen"), "openai");
        assert_eq!(provider_name_to_shape("opencode-go"), "openai");
        assert_eq!(provider_name_to_shape("z-ai"), "anthropic");
        assert_eq!(provider_name_to_shape("umans"), "anthropic");
        assert_eq!(provider_name_to_shape("kimi"), "anthropic");
        assert_eq!(provider_name_to_shape("codex"), "responses");
    }

    #[test]
    fn provider_name_to_shape_unknown_defaults_to_openai() {
        assert_eq!(provider_name_to_shape("nonexistent-provider"), "openai");
    }
}
