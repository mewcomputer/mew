use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::sync::Arc;
use tracing::warn;

use mew_agent::Agent;
use mew_catalog::Catalog;
use mew_config::{Config, ProviderConfig};
use mew_hooks::NopDispatcher;
use mew_message::{Finish, Part, PartId};
use mew_provider::Provider;
use mew_provider_anthropic::Adapter as AnthropicAdapter;
use mew_provider_openai::Adapter as OpenAIAdapter;
use mew_session::Writer as SessionWriter;
use mew_tools::tools::echo::Echo;

#[derive(Parser)]
#[command(name = "mew")]
#[command(about = "A terminal agent harness")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a single prompt non-interactively
    Run {
        /// Provider ID
        #[arg(long, default_value = "opencode-zen")]
        provider: String,

        /// Model ID (overrides provider default)
        #[arg(long)]
        model: Option<String>,

        /// Dump raw request/response to stderr
        #[arg(long)]
        raw: bool,

        /// The prompt to send
        prompt: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            provider,
            model,
            raw,
            prompt,
        } => run_cmd(provider, model, raw, prompt).await,
    }
}

async fn run_cmd(
    provider_flag: String,
    model_flag: Option<String>,
    raw: bool,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let prompt = prompt_parts.join(" ");
    if prompt.is_empty() {
        anyhow::bail!("missing prompt");
    }

    let cfg = mew_config::load().context("load config")?;

    let cat = match mew_catalog::load().await {
        Ok(c) => c,
        Err(e) => {
            warn!("catalog load failed, using fallback routing: {}", e);
            // Create empty catalog - we can't construct it directly since
            // the field is private, so we'll just handle missing catalog
            // in the provider build logic
            return build_and_run(&cfg, None, &provider_flag, model_flag, raw, prompt).await;
        }
    };

    build_and_run(&cfg, Some(&cat), &provider_flag, model_flag, raw, prompt).await
}

async fn build_and_run(
    cfg: &Config,
    cat: Option<&Catalog>,
    provider_flag: &str,
    model_flag: Option<String>,
    raw: bool,
    prompt: String,
) -> Result<()> {
    let (provider_id, model_id) = resolve_model(cfg, cat, provider_flag, model_flag);

    let provider = build_provider(cfg, cat, &provider_id, &model_id, raw).context("build provider")?;

    let session_id = ulid::Ulid::new().to_string();
    let session_writer = SessionWriter::open(&session_id)
        .await
        .context("open session")?;

    let dispatcher = Arc::new(NopDispatcher);
    let tools: Vec<Arc<dyn mew_tools::Tool>> = vec![Arc::new(Echo)];

    let mut agent = Agent::new(provider, dispatcher, Some(session_writer), tools, None);

    // Load project context files and prepend to system prompt
    let ctx_loader = mew_context::Loader::new(std::env::current_dir().unwrap_or_default());
    let ctx_files = ctx_loader.load().unwrap_or_default();
    if !ctx_files.is_empty() {
        agent.set_system(mew_context::build_system_prompt(&ctx_files));
    }

    let mut rx = agent.run(prompt);

    let mut part_types: std::collections::HashMap<PartId, &'static str> =
        std::collections::HashMap::new();

    while let Some(event) = rx.recv().await {
        match event {
            mew_agent::AgentEvent::Provider(ev) => match ev {
                mew_provider::ProviderEvent::PartStart { part } => {
                    let id = part.id();
                    match &part {
                        Part::Text(_) => {
                            part_types.insert(id, "text");
                        }
                        Part::Reasoning(_) => {
                            part_types.insert(id, "reasoning");
                            eprintln!("\n[thinking]");
                        }
                        Part::ToolCall(_) => {
                            part_types.insert(id, "tool");
                        }
                        _ => {}
                    }
                }
                mew_provider::ProviderEvent::PartDelta {
                    part_id,
                    field: _,
                    delta,
                } => match part_types.get(&part_id) {
                    Some(&"reasoning") => {
                        eprint!("{}", delta);
                        let _ = std::io::stderr().flush();
                    }
                    Some(&"text") => {
                        print!("{}", delta);
                        let _ = std::io::stdout().flush();
                    }
                    Some(&"tool") => {}
                    _ => {}
                },
                mew_provider::ProviderEvent::PartEnd { part_id } => {
                    match part_types.get(&part_id) {
                        Some(&"reasoning") => eprintln!("\n[/thinking]"),
                        Some(&"tool") => eprintln!(),
                        _ => {}
                    }
                    part_types.remove(&part_id);
                }
                mew_provider::ProviderEvent::MessageEnd { finish, .. } => {
                    if finish == Finish::Stop {
                        println!();
                    }
                }
                _ => {}
            },
            mew_agent::AgentEvent::PermissionRequest { call, tx } => {
                eprintln!("\n[permission] {}: {:?}", call.tool_name, call.input);
                let _ = tx.send(mew_hooks::PermissionDecision::AllowOnce);
            }
            mew_agent::AgentEvent::ToolStart { call_id } => {
                eprintln!("\n[tool start: {}]", call_id);
            }
            mew_agent::AgentEvent::ToolEnd { call_id, success } => {
                eprintln!("[tool end: {}] success={}", call_id, success);
            }
            mew_agent::AgentEvent::PartUpdated { .. } => {}
            mew_agent::AgentEvent::Error(msg) => {
                anyhow::bail!("agent error: {}", msg);
            }
        }
    }

    Ok(())
}

fn resolve_model(
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

fn is_known_provider(cfg: &Config, provider_id: &str) -> bool {
    cfg.providers.contains_key(provider_id)
        || matches!(provider_id, "opencode-zen" | "opencode-go" | "z-ai")
}

fn build_provider(
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
        .or_else(|| match provider_id {
            "opencode-zen" => Some(ProviderConfig {
                shape: "openai".to_string(),
                base_url: "https://opencode.ai/zen/v1".to_string(),
                credential_ref: "opencode-zen".to_string(),
            }),
            "opencode-go" => Some(ProviderConfig {
                shape: "openai".to_string(),
                base_url: "https://opencode.ai/zen/go/v1".to_string(),
                credential_ref: "opencode-zen".to_string(),
            }),
            "z-ai" => Some(ProviderConfig {
                shape: "anthropic".to_string(),
                base_url: "https://api.z.ai/api/anthropic/v1".to_string(),
                credential_ref: "z-ai".to_string(),
            }),
            _ => None,
        })
        .with_context(|| format!("unknown provider {}", provider_id))?;

    let creds = mew_config::get_credential(&pc.credential_ref).context("get credential")?;

    let model = if model_override.is_empty() {
        if cfg.default_model.is_empty() {
            "deepseek-v4-flash".to_string()
        } else {
            cfg.default_model.clone()
        }
    } else {
        model_override.to_string()
    };

    let mut shape = pc.shape;
    if let Some(c) = cat {
        let s = c.shape_for(&model);
        if !s.is_empty() {
            shape = s.to_string();
        }
    }

    let mut base_url = pc.base_url;
    if provider_id == "opencode-go" && model.starts_with("minimax-") {
        shape = "anthropic".to_string();
        base_url = "https://opencode.ai/zen/go/v1".to_string();
    }

    match shape.as_str() {
        "openai" => {
            let mut adapter = OpenAIAdapter::new(
                provider_id.to_string(),
                base_url,
                model,
                creds,
            );
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        "anthropic" => {
            let mut adapter = AnthropicAdapter::new(
                provider_id.to_string(),
                base_url,
                model,
                creds,
            );
            if raw {
                adapter.set_dump(true);
            }
            Ok(Arc::new(adapter))
        }
        _ => anyhow::bail!("unsupported shape {} for provider {}", shape, provider_id),
    }
}
