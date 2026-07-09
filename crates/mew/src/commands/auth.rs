//! `mew auth` subcommand — manage provider authentication.
//!
//! Supports multiple OAuth providers via the `OAuthProvider` trait.
//! Currently only `openai-responses` (ChatGPT OAuth) is registered.

use std::sync::Arc;

use anyhow::Result;

use mew_provider::auth::OAuthProvider;

use crate::cli::AuthCommands;

/// The registry of OAuth-capable providers. Add new providers here.
fn providers() -> Vec<Arc<dyn OAuthProvider>> {
    vec![Arc::new(
        mew_provider_responses::oauth::OpenaiResponsesOAuth,
    )]
}

/// Find a provider by slug (e.g. "openai-responses").
fn find_provider(slug: &str) -> Option<Arc<dyn OAuthProvider>> {
    providers().into_iter().find(|p| p.slug() == slug)
}

/// Select a provider by name or number. Returns None if the input
/// doesn't match any provider.
fn select_provider(
    input: &str,
    registry: &[Arc<dyn OAuthProvider>],
) -> Option<Arc<dyn OAuthProvider>> {
    // Try exact slug match first.
    if let Some(p) = registry.iter().find(|p| p.slug() == input) {
        return Some(p.clone());
    }
    // Try 1-based index.
    if let Ok(n) = input.parse::<usize>() {
        if n >= 1 && n <= registry.len() {
            return Some(registry[n - 1].clone());
        }
    }
    None
}

pub async fn auth_cmd(command: AuthCommands) -> Result<()> {
    match command {
        AuthCommands::Login { provider } => {
            let registry = providers();
            let selected = match provider {
                Some(name) => {
                    // Provider specified — look it up directly.
                    find_provider(&name).ok_or_else(|| {
                        anyhow::anyhow!(
                            "unknown provider: {name}. Available: {}",
                            registry
                                .iter()
                                .map(|p| p.slug())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?
                }
                None => {
                    // No provider specified — list and prompt.
                    if registry.len() == 1 {
                        // Only one provider, use it directly.
                        registry[0].clone()
                    } else {
                        eprintln!("Available providers:");
                        for (i, p) in registry.iter().enumerate() {
                            eprintln!("  {}. {}", i + 1, p.display_name());
                        }
                        eprint!("Select provider [1]: ");
                        use std::io::Write;
                        let _ = std::io::stderr().flush();
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input)?;
                        let input = input.trim();
                        let input = if input.is_empty() { "1" } else { input };
                        select_provider(input, &registry)
                            .ok_or_else(|| anyhow::anyhow!("invalid selection: {input}"))?
                    }
                }
            };
            mew_provider::auth::login(selected.as_ref()).await?;
            Ok(())
        }
        AuthCommands::Status => {
            for provider in providers() {
                let slug = provider.slug();
                let display = provider.display_name();
                if let Some(status) = mew_provider::auth::status(provider.as_ref()) {
                    println!("{slug}: {status}");
                } else {
                    // Check if an API key is available.
                    let cfg = mew_config::load().unwrap_or_default();
                    if let Some(pc) = cfg.providers.get(slug) {
                        if mew_config::get_credential(&pc.credential_ref).is_ok() {
                            println!("{slug}: API key ✓ ({display})");
                        } else {
                            println!("{slug}: not logged in ({display})");
                        }
                    } else {
                        println!("{slug}: not configured");
                    }
                }
            }
            Ok(())
        }
        AuthCommands::Logout { provider } => {
            let registry = providers();
            let selected = match provider {
                Some(name) => find_provider(&name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "unknown provider: {name}. Available: {}",
                        registry
                            .iter()
                            .map(|p| p.slug())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?,
                None => {
                    if registry.len() == 1 {
                        registry[0].clone()
                    } else {
                        eprintln!("Available providers:");
                        for (i, p) in registry.iter().enumerate() {
                            eprintln!("  {}. {}", i + 1, p.display_name());
                        }
                        eprint!("Select provider [1]: ");
                        use std::io::Write;
                        let _ = std::io::stderr().flush();
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input)?;
                        let input = input.trim();
                        let input = if input.is_empty() { "1" } else { input };
                        select_provider(input, &registry)
                            .ok_or_else(|| anyhow::anyhow!("invalid selection: {input}"))?
                    }
                }
            };
            mew_provider::auth::logout(selected.as_ref())?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_contains_openai_responses() {
        let registry = providers();
        assert!(registry.iter().any(|p| p.slug() == "openai-responses"));
    }

    #[test]
    fn test_select_provider_by_slug() {
        let registry = providers();
        let selected = select_provider("openai-responses", &registry);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().slug(), "openai-responses");
    }

    #[test]
    fn test_select_provider_by_number() {
        let registry = providers();
        let selected = select_provider("1", &registry);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().slug(), "openai-responses");
    }

    #[test]
    fn test_select_provider_invalid_returns_none() {
        let registry = providers();
        assert!(select_provider("nonexistent", &registry).is_none());
        assert!(select_provider("99", &registry).is_none());
    }

    #[test]
    fn test_find_provider_by_slug() {
        let found = find_provider("openai-responses");
        assert!(found.is_some());
        assert!(find_provider("nonexistent").is_none());
    }
}
