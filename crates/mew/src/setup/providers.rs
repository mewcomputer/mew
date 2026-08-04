//! Provider-related setup functions.
//!
//! Extracted from `main.rs` as pure code motion. These resolve providers/models
//! from CLI flags and persisted state, build provider adapters, load the model
//! catalog, and wire the Auto/Auto+ classifier provider.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
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
    if let Some(provider) = cli {
        return provider;
    }

    let remembered = state.last_provider.as_str();
    let candidate = if !remembered.is_empty() && is_known_provider(cfg, remembered) {
        remembered.to_string()
    } else {
        "opencode-zen".to_string()
    };

    if provider_available(cfg, &candidate) {
        return candidate;
    }

    // A remembered provider without credentials should not prevent the
    // daemon from starting when another configured provider is usable.
    [
        "opencode-zen",
        "opencode-go",
        "z-ai",
        "umans",
        "deepseek",
        "kimi-for-coding",
        "codex",
        "alibaba-token-plan",
        "alibaba-token-plan-cn",
    ]
    .into_iter()
    .find(|provider| cfg.providers.contains_key(*provider) && provider_available(cfg, provider))
    .map(str::to_owned)
    .unwrap_or(candidate)
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
        if persisted.is_empty() {
            return None;
        }
        if is_known_model(cfg, persisted) {
            return Some(persisted.to_string());
        }
        // Bare model ID (no '/') from a built-in provider — accept if the
        // companion last_provider names a configured provider. The model
        // list for that provider isn't statically known (it comes from the
        // daemon or /v1/models at runtime), so we trust the provider check.
        if !persisted.contains('/')
            && !state.last_provider.is_empty()
            && is_known_provider(cfg, &state.last_provider)
        {
            return Some(persisted.to_string());
        }
        None
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
    // Bare model ID — valid if it's a custom model. Built-in models from
    // configured providers (e.g. "k3" from "kimi-for-coding") are also valid when the
    // companion last_provider names a known provider, but that cross-check
    // is done by the caller (resolve_model_opt) which has access to both
    // last_model and last_provider.
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

    let thinking_budget: Option<mew_catalog::ThinkingBudget> =
        cm.thinking_budget
            .as_ref()
            .map(|b| mew_catalog::ThinkingBudget {
                min: b.min,
                max: b.max,
                step: b.step,
                default: b.default,
                by_effort: b.by_effort.clone(),
            });

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
        thinking_budget: thinking_budget.or(base.thinking_budget),
        responses_lite: cm.responses_lite || base.responses_lite,
        prompt_cache_retention_secs: cm
            .prompt_cache_retention_secs
            .or(base.prompt_cache_retention_secs),
        // Custom models are chat-completions targets; a merged override of a
        // known image/video-only catalog model keeps its text_output flag.
        text_output: match (cm.merge, existing) {
            (true, Some(_)) => base.text_output,
            _ => true,
        },
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

/// Resolve a thinking variant name to provider params, returning the params
/// plus the canonical resolved name (a clamped/snapped `budget:<n>` when the
/// input was a numeric budget, the variant name otherwise).
///
/// Returns `None` when the model has no configurable thinking, the name is
/// unknown, or the name is an explicit off request on a model that has no
/// explicit off variant (callers treat that as a plain disable).
pub(crate) fn resolve_reasoning(
    cat: Option<&Catalog>,
    model_id: &str,
    variant_name: Option<&str>,
) -> Option<(mew_provider::ReasoningConfig, String)> {
    let cat = cat?;
    let variants = cat.thinking_variants(model_id);
    if variants.is_empty() {
        return None;
    }
    let (variant, resolved_name) = match variant_name {
        // Explicit off: only models with an explicit off/none variant can
        // disable thinking with real params (qwen3.8-max sends
        // `enable_thinking: false` because thinking is on by default;
        // MiniMax M3 sends `thinking: disabled`).
        Some(name) if name == "off" || name == "none" => {
            let off = variants
                .iter()
                .find(|v| v.name == "off" || v.name == "none")?;
            (off.clone(), off.name.clone())
        }
        // Numeric token budget: clamped and snapped to the model's declared
        // range. Requires budget metadata; unknown budgets resolve to `None`
        // (treated as unknown variants by callers).
        Some(name) if name.starts_with("budget:") => {
            let budget = cat.thinking_budget(model_id)?;
            let n = name
                .strip_prefix("budget:")
                .and_then(|s| s.parse::<i64>().ok())?;
            let snapped = budget.snap(n);
            let variant = mew_catalog::ThinkingVariant {
                name: format!("budget:{snapped}"),
                params: serde_json::json!({
                    "enable_thinking": true,
                    "thinking_budget": snapped,
                }),
            };
            (variant, format!("budget:{snapped}"))
        }
        Some(name) => {
            let variant = variants.iter().find(|v| v.name == name)?.clone();
            (variant, name.to_string())
        }
        None => {
            let default = cat.default_thinking(model_id)?;
            (default.clone(), default.name.clone())
        }
    };
    let params = variant.params.as_object().cloned().unwrap_or_default();
    Some((mew_provider::ReasoningConfig { params }, resolved_name))
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
                // The catalog may use a different provider name than the
                // config (e.g. catalog may use a different name). Map it back
                // to the configured provider if a known mapping exists. If the
                // catalog provider isn't configured at all, keep the current
                // provider: models.dev dedups model ids across resellers, so
                // the catalog entry may name a provider the user has no
                // credentials for ("unknown provider" at startup otherwise).
                if let Some(configured) = config_provider_for_catalog(cfg, &m.provider) {
                    provider_id = configured;
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
        Some(pc) => credential_for_provider(provider_id, pc).is_ok(),
        None => false,
    }
}

fn credential_for_provider(
    provider_id: &str,
    pc: &ProviderConfig,
) -> Result<String, mew_config::ConfigError> {
    match mew_config::get_credential(&pc.credential_ref) {
        Ok(value) => Ok(value),
        Err(reference_error) if pc.credential_ref != provider_id => {
            mew_config::get_credential(provider_id).map_err(|_| reference_error)
        }
        Err(error) => Err(error),
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

/// Per-model transport overrides that the models.dev catalog cannot express
/// (it has no per-model transport field). Applied after the provider default
/// and any catalog shape, so the hardcode always wins.
fn hardcoded_shape_override(provider_id: &str, model: &str) -> Option<&'static str> {
    if provider_id == "opencode-go" && model.starts_with("minimax-") {
        return Some("anthropic");
    }
    // DeepSeek V4 Flash speaks the OpenAI Responses API (`POST /v1/responses`);
    // the rest of the DeepSeek lineup is chat-completions only.
    if provider_id == "deepseek" && model == "deepseek-v4-flash" {
        return Some("responses");
    }
    None
}

/// Whether a responses-shaped provider may authenticate with the stored Codex
/// OAuth tokens. Only codex itself: those tokens are OpenAI credentials and
/// must never be sent to a third-party endpoint.
fn responses_uses_codex_oauth(provider_id: &str) -> bool {
    provider_id == "codex"
}

/// Return whether a catalog provider id can supply models for a configured
/// provider. models.dev and mew use different names for a few compatible
/// endpoints, so exact string equality would hide otherwise usable models.
pub(crate) fn catalog_provider_matches(configured: &str, catalog: &str) -> bool {
    configured == catalog
        || matches!(
            (configured, catalog),
            ("opencode-zen", "opencode")
                | ("z-ai", "zai")
                | ("z-ai", "zai-coding-plan")
                | ("umans", "umans-ai")
                | ("umans", "umans-ai-coding-plan")
        )
}

/// Reverse mapping of `catalog_provider_matches`: given a catalog provider
/// name, return the matching configured provider name if one exists.
fn config_provider_for_catalog(cfg: &Config, catalog: &str) -> Option<String> {
    cfg.providers
        .keys()
        .find(|configured| catalog_provider_matches(configured, catalog))
        .cloned()
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
    // Non-fatal: some providers (codex with OAuth) don't need an API key, so we
    // resolve lazily per shape arm. For API-key shapes (`openai`/`anthropic`)
    // we propagate `get_credential`'s error directly instead of swallowing it
    // via `.ok()`, so the user sees the real diagnostic (env var, keyring
    // command, credentials.json path) rather than a bare "get credential".
    let creds = credential_for_provider(provider_id, pc);

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
    if let Some(overridden) = hardcoded_shape_override(provider_id, &model) {
        shape = overridden.to_string();
    }
    if provider_id == "opencode-go" && model.starts_with("minimax-") {
        base_url = "https://opencode.ai/zen/go/v1".to_string();
    }

    let responses_lite = cat
        .and_then(|c| c.lookup(&model))
        .map(|m| m.responses_lite)
        .unwrap_or_else(|| is_known_responses_lite_model(&model));

    match shape.as_str() {
        "openai" => {
            let creds = creds?;
            let mut adapter = OpenAIAdapter::new(provider_id.to_string(), base_url, model, creds);
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        "anthropic" => {
            let creds = creds?;
            let mut adapter =
                AnthropicAdapter::new(provider_id.to_string(), base_url, model, creds);
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        "responses" => {
            // Non-codex responses providers (e.g. deepseek's V4 Flash)
            // authenticate with their own API key only — the stored Codex
            // OAuth tokens are OpenAI credentials and must not be sent to a
            // third-party endpoint.
            if !responses_uses_codex_oauth(provider_id) {
                let key = creds?;
                let mut adapter =
                    ResponsesAdapter::new(provider_id.to_string(), base_url, model, key)
                        .with_responses_lite(responses_lite);
                if raw {
                    adapter.set_dump(true);
                }
                return Ok(Arc::new(adapter));
            }
            // Try OAuth first, then fall back to API key. A missing API key
            // credential is not fatal here because OAuth may succeed, so we
            // drop the error if auth::resolve finds another path — but we keep
            // its message around to surface a useful diagnostic if every path
            // fails.
            let creds_err_msg = creds.as_ref().err().map(|e| e.to_string());
            let creds = creds.ok();
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
                Err(e) => {
                    let mut msg = format!(
                        "no credentials for codex: {e}. \
                         Run `mew auth login codex` or set OPENAI_API_KEY."
                    );
                    if let Some(ce) = creds_err_msg {
                        msg.push_str(&format!("\n\n{}", ce));
                    }
                    Err(anyhow::anyhow!(msg))
                }
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
        .with_context(|| {
            let config_path = mew_config::config_dir().join("config.toml");
            let available: Vec<&str> = cfg.providers.keys().map(|s| s.as_str()).collect();
            format!(
                "unknown provider '{}'.\n\
                 Config loaded from: {}\n\
                 Available providers: {}\n\
                 Fix: add a [providers.{}] entry for '{}', or clear the stale provider from state.toml at {}",
                provider_id,
                config_path.display(),
                if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available.join(", ")
                },
                provider_id,
                provider_id,
                mew_config::state_file_path().display()
            )
        })?;

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
        assert_eq!(built.prompt_cache_retention_secs, None);
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
    fn build_custom_model_merge_preserves_and_overrides_cache_retention() {
        let mut existing = sample_catalog_model();
        existing.prompt_cache_retention_secs = Some(300);

        let preserved = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            merge: true,
            ..Default::default()
        };
        assert_eq!(
            build_custom_model(&preserved, Some(&existing)).prompt_cache_retention_secs,
            Some(300)
        );

        let overridden = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            prompt_cache_retention_secs: Some(14_400),
            merge: true,
            ..Default::default()
        };
        assert_eq!(
            build_custom_model(&overridden, Some(&existing)).prompt_cache_retention_secs,
            Some(14_400)
        );
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

    #[test]
    fn build_custom_model_threads_thinking_budget() {
        // Explicit budget range without merge: replaces wholesale.
        let cm = mew_config::CustomModel {
            id: "qwen3.8-max".into(),
            provider: "qwen".into(),
            thinking_budget: Some(mew_config::ThinkingBudgetDef {
                min: 1000,
                max: 10_000,
                step: 500,
                default: 5000,
                by_effort: vec![("low".into(), 1000)],
            }),
            ..Default::default()
        };
        let built = build_custom_model(&cm, None);
        let budget = built.thinking_budget.expect("budget threaded");
        assert_eq!(
            (budget.min, budget.max, budget.step, budget.default),
            (1000, 10_000, 500, 5000)
        );
        assert_eq!(budget.by_effort, vec![("low".to_owned(), 1000)]);

        // Merge preserves an unset budget from the catalog entry.
        let mut existing = sample_catalog_model();
        existing.thinking_budget = Some(mew_catalog::ThinkingBudget {
            min: 0,
            max: 262_144,
            step: 1024,
            default: 131_072,
            by_effort: Vec::new(),
        });
        let preserved = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            merge: true,
            ..Default::default()
        };
        assert!(build_custom_model(&preserved, Some(&existing))
            .thinking_budget
            .is_some());

        // Merge overrides an existing budget when set.
        let overridden = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            thinking_budget: Some(mew_config::ThinkingBudgetDef {
                min: 0,
                max: 4096,
                step: 128,
                default: 2048,
                by_effort: Vec::new(),
            }),
            merge: true,
            ..Default::default()
        };
        let built = build_custom_model(&overridden, Some(&existing));
        let budget = built.thinking_budget.expect("budget overridden");
        assert_eq!((budget.min, budget.max), (0, 4096));

        // Without merge, the catalog's budget is dropped unless set here.
        let replaced = mew_config::CustomModel {
            id: "glm-5.3".into(),
            provider: "z-ai".into(),
            ..Default::default()
        };
        assert!(build_custom_model(&replaced, Some(&existing))
            .thinking_budget
            .is_none());
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
    fn catalog_provider_matches_configured_aliases() {
        assert!(catalog_provider_matches("opencode-zen", "opencode"));
        assert!(catalog_provider_matches("z-ai", "zai-coding-plan"));
        assert!(catalog_provider_matches(
            "kimi-for-coding",
            "kimi-for-coding"
        ));
        assert!(!catalog_provider_matches("deepseek", "opencode"));
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
        let (config, name) = result.unwrap();
        assert_eq!(name, "high");
        assert!(config.params.is_empty());
    }

    #[test]
    fn resolve_reasoning_none_variant_name() {
        // "test-model" has no explicit off/none variant, so an off request
        // is a plain disable (None) rather than an error.
        assert!(resolve_reasoning(Some(&cat_with_variant()), "test-model", Some("none")).is_none());
        assert!(resolve_reasoning(Some(&cat_with_variant()), "test-model", Some("off")).is_none());
    }

    #[test]
    fn resolve_reasoning_default_when_no_name() {
        let cat = cat_with_variant();
        let result = resolve_reasoning(Some(&cat), "test-model", None);
        assert!(result.is_some(), "should find the default variant");
        let (_, name) = result.unwrap();
        assert_eq!(name, "high");
    }

    #[test]
    fn resolve_reasoning_qwen38_off_sends_enable_thinking_false() {
        // qwen3.8-max thinks by default, so "off" must resolve to an
        // explicit enable_thinking: false rather than a plain disable.
        let cat = Catalog::empty();
        let (config, name) = resolve_reasoning(Some(&cat), "qwen3.8-max", Some("off")).unwrap();
        assert_eq!(name, "off");
        assert_eq!(config.params["enable_thinking"], serde_json::json!(false));
        assert!(config.params.get("thinking_budget").is_none());
    }

    #[test]
    fn resolve_reasoning_qwen38_effort_variants_enable_thinking() {
        let cat = Catalog::empty();
        let (config, name) = resolve_reasoning(Some(&cat), "qwen3.8-max", Some("xhigh")).unwrap();
        assert_eq!(name, "xhigh");
        assert_eq!(config.params["enable_thinking"], serde_json::json!(true));
        assert_eq!(config.params["reasoning_effort"], "xhigh");
    }

    #[test]
    fn resolve_reasoning_budget_parse_clamp_snap() {
        let cat = Catalog::empty();
        // Exact multiple passes through.
        let (config, name) = resolve_reasoning(Some(&cat), "qwen3.8-max", Some("budget:8192"))
            .expect("budget resolves");
        assert_eq!(name, "budget:8192");
        assert_eq!(config.params["thinking_budget"], 8192);
        assert_eq!(config.params["enable_thinking"], serde_json::json!(true));
        // Out-of-range clamps to the declared range.
        let (_, name) =
            resolve_reasoning(Some(&cat), "qwen3.8-max", Some("budget:999999999")).unwrap();
        assert_eq!(name, "budget:262144");
        // Off-step values snap to the nearest step.
        let (config, name) =
            resolve_reasoning(Some(&cat), "qwen3.8-max", Some("budget:200001")).unwrap();
        assert_eq!(name, "budget:199680");
        assert_eq!(config.params["thinking_budget"], 199680);
    }

    #[test]
    fn resolve_reasoning_budget_unknown_or_unsupported_returns_none() {
        let cat = Catalog::empty();
        // Unparseable budget.
        assert!(resolve_reasoning(Some(&cat), "qwen3.8-max", Some("budget:abc")).is_none());
        // Model without budget metadata (deepseek-v4 has no thinking_budget).
        assert!(resolve_reasoning(Some(&cat), "deepseek-v4", Some("budget:8192")).is_none());
        // Model with no thinking at all.
        assert!(resolve_reasoning(Some(&cat), "kimi-for-coding", Some("budget:8192")).is_none());
        // Unknown variant name still resolves to None.
        assert!(resolve_reasoning(Some(&cat), "qwen3.8-max", Some("bogus")).is_none());
    }

    #[test]
    fn resolve_reasoning_off_requires_explicit_off_variant() {
        let cat = Catalog::empty();
        // deepseek-v4 has effort variants but no off variant: off → None
        // (callers treat it as a plain disable).
        assert!(resolve_reasoning(Some(&cat), "deepseek-v4", Some("off")).is_none());
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
    fn resolve_model_opt_accepts_bare_model_with_known_provider() {
        // Bare model id "k3" with last_provider="kimi-for-coding" — the model
        // list for "kimi-for-coding" isn't statically known, but the provider
        // is configured, so the bare id is accepted as a starting point.
        let state = mew_config::State {
            last_model: "k3".into(),
            last_provider: "kimi-for-coding".into(),
            ..Default::default()
        };
        let cfg = cfg_with_default_providers();
        assert_eq!(resolve_model_opt(None, &state, &cfg), Some("k3".into()));
    }

    #[test]
    fn resolve_model_opt_rejects_bare_model_with_unknown_provider() {
        // Bare model id with an unknown provider should still be rejected.
        let state = mew_config::State {
            last_model: "k3".into(),
            last_provider: "bogus".into(),
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

    // --- resolve_model ---

    #[test]
    fn resolve_model_maps_catalog_provider_to_configured_name() {
        let cfg = cfg_with_default_providers();
        let mut cat = Catalog::empty();
        cat.models.insert(
            "glm-5.2".into(),
            mew_catalog::Model {
                id: "glm-5.2".into(),
                provider: "zai".into(),
                ..Default::default()
            },
        );
        // "zai" is a known alias for configured "z-ai".
        let (pid, mid) = resolve_model(&cfg, Some(&cat), "opencode-zen", Some("glm-5.2".into()));
        assert_eq!(pid, "z-ai");
        assert_eq!(mid, "glm-5.2");
    }

    #[test]
    fn resolve_model_keeps_provider_when_catalog_provider_not_configured() {
        // models.dev lists popular models under many resellers; the catalog
        // dedup keeps whichever provider sorts last (e.g. venice). Adopting an
        // unconfigured catalog provider crashed startup with "unknown provider"
        // even when the resolved provider serves the model just fine.
        let cfg = cfg_with_default_providers();
        let mut cat = Catalog::empty();
        cat.models.insert(
            "deepseek-v4-flash".into(),
            mew_catalog::Model {
                id: "deepseek-v4-flash".into(),
                provider: "venice".into(),
                ..Default::default()
            },
        );
        let (pid, mid) = resolve_model(
            &cfg,
            Some(&cat),
            "deepseek",
            Some("deepseek-v4-flash".into()),
        );
        assert_eq!(pid, "deepseek");
        assert_eq!(mid, "deepseek-v4-flash");
    }

    #[test]
    fn resolve_model_adopts_catalog_provider_when_configured() {
        let cfg = cfg_with_default_providers();
        let mut cat = Catalog::empty();
        cat.models.insert(
            "deepseek-v4-flash".into(),
            mew_catalog::Model {
                id: "deepseek-v4-flash".into(),
                provider: "deepseek".into(),
                ..Default::default()
            },
        );
        let (pid, _) = resolve_model(
            &cfg,
            Some(&cat),
            "opencode-zen",
            Some("deepseek-v4-flash".into()),
        );
        assert_eq!(pid, "deepseek");
    }

    // --- hardcoded_shape_override ---

    #[test]
    fn hardcoded_shape_override_deepseek_v4_flash_uses_responses() {
        assert_eq!(
            hardcoded_shape_override("deepseek", "deepseek-v4-flash"),
            Some("responses")
        );
    }

    #[test]
    fn hardcoded_shape_override_leaves_other_models_on_provider_default() {
        // The rest of the DeepSeek lineup stays on chat completions.
        assert_eq!(
            hardcoded_shape_override("deepseek", "deepseek-v4-pro"),
            None
        );
        assert_eq!(hardcoded_shape_override("deepseek", "deepseek-chat"), None);
        assert_eq!(
            hardcoded_shape_override("deepseek", "deepseek-reasoner"),
            None
        );
        // The same model id under a different provider is not overridden.
        assert_eq!(
            hardcoded_shape_override("openai", "deepseek-v4-flash"),
            None
        );
    }

    #[test]
    fn hardcoded_shape_override_minimax_stays_anthropic() {
        assert_eq!(
            hardcoded_shape_override("opencode-go", "minimax-text-01"),
            Some("anthropic")
        );
    }

    // --- responses_uses_codex_oauth ---

    #[test]
    fn responses_oauth_only_for_codex() {
        assert!(responses_uses_codex_oauth("codex"));
        assert!(!responses_uses_codex_oauth("deepseek"));
    }

    // --- build_provider credential error surfacing ---

    /// When a provider's credential is missing (no env var, no keyring, no
    /// credentials.json entry), `build_provider` must surface the rich
    /// `CredentialNotFound` diagnostic — the env var name and credentials.json
    /// path — rather than a bare "get credential" with no cause.
    ///
    /// This guards against the regression where `.ok()` swallowed the
    /// underlying error and `.context("get credential")` replaced it.
    #[test]
    fn build_provider_missing_credential_surfaces_diagnostic() {
        // Use a credential_ref that is extremely unlikely to be set in the
        // ambient environment or present in the real credentials.json.
        let ref_name = "mew-test-missing-cred-ref-7f3a";
        let env_key = format!(
            "MEW_CRED_{}",
            ref_name
                .to_uppercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        // Ensure the env var is genuinely unset for this test.
        std::env::remove_var(&env_key);

        let mut cfg = Config::default();
        cfg.providers.insert(
            "test-prov".into(),
            ProviderConfig {
                shape: "openai".into(),
                base_url: "https://example.invalid".into(),
                credential_ref: ref_name.into(),
                ..Default::default()
            },
        );

        let err = match build_provider(&cfg, None, "test-prov", "", false) {
            Ok(_) => panic!("build_provider should have failed for a missing credential"),
            Err(e) => e,
        };
        let chain: Vec<String> = err.chain().map(|e| e.to_string()).collect();
        let joined = chain.join("\n--\n");

        // The bare-context regression produced only "build provider" and
        // "get credential" with no cause. Assert the real diagnostic leaked.
        assert!(
            joined.contains(&env_key),
            "error should mention the env var {}\ngot: {}",
            env_key,
            joined
        );
        assert!(
            joined.contains("credentials.json"),
            "error should mention credentials.json\ngot: {}",
            joined
        );
        // The swallowed-error regression never included "credential not found".
        assert!(
            joined.contains("credential not found"),
            "error should include the CredentialNotFound message\ngot: {}",
            joined
        );
    }

    /// Same as above but for the `anthropic` shape, which had the identical
    /// `.ok()` + `.context("get credential")` pattern.
    #[test]
    fn build_provider_missing_credential_anthropic_shape_surfaces_diagnostic() {
        let ref_name = "mew-test-missing-cred-anthropic-7f3a";
        let env_key = format!(
            "MEW_CRED_{}",
            ref_name
                .to_uppercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        std::env::remove_var(&env_key);

        let mut cfg = Config::default();
        cfg.providers.insert(
            "test-prov-anthropic".into(),
            ProviderConfig {
                shape: "anthropic".into(),
                base_url: "https://example.invalid".into(),
                credential_ref: ref_name.into(),
                ..Default::default()
            },
        );

        let err = match build_provider(&cfg, None, "test-prov-anthropic", "", false) {
            Ok(_) => panic!("build_provider should have failed for a missing credential"),
            Err(e) => e,
        };
        let joined = err
            .chain()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n--\n");

        assert!(
            joined.contains(&env_key),
            "error should mention the env var {}\ngot: {}",
            env_key,
            joined
        );
        assert!(
            joined.contains("credential not found"),
            "error should include the CredentialNotFound message\ngot: {}",
            joined
        );
    }
}
