//! `mew auth` subcommand — manage provider authentication.
//!
//! Supports multiple OAuth providers via the `OAuthProvider` trait.
//! Currently only `codex` (ChatGPT OAuth) is registered.

use std::sync::Arc;

use anyhow::Result;

use mew_provider::auth::OAuthProvider;

use crate::cli::AuthCommands;

/// The registry of OAuth-capable providers. Add new providers here.
fn providers() -> Vec<Arc<dyn OAuthProvider>> {
    vec![Arc::new(mew_provider_responses::oauth::CodexOAuth)]
}

/// Find a provider by slug (e.g. "codex").
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
        AuthCommands::Login { provider, headless } => {
            if provider.as_deref() == Some("antigravity") {
                if headless {
                    anyhow::bail!("Antigravity sign-in does not support --headless");
                }
                #[cfg(unix)]
                return crate::setup::antigravity::login().await;
                #[cfg(not(unix))]
                anyhow::bail!("Antigravity local CLI is available only on Unix");
            }
            let registry = providers();
            let selected = select_or_prompt(provider, &registry)?;
            mew_provider::auth::login(selected.as_ref(), headless).await?;
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
            #[cfg(unix)]
            if let Some(line) = antigravity_status_line(
                std::env::var_os("PATH").as_deref(),
                &crate::setup::providers::agy_fallback_dirs(),
            ) {
                println!("{line}");
            }
            Ok(())
        }
        AuthCommands::Logout { provider } => {
            if provider.as_deref() == Some("antigravity") {
                anyhow::bail!(
                    "Antigravity sign-out is not available in Mew. Manage the existing account with the official AGY CLI."
                );
            }
            let registry = providers();
            let selected = select_or_prompt(provider, &registry)?;
            mew_provider::auth::logout(selected.as_ref())?;
            Ok(())
        }
    }
}

#[cfg(unix)]
fn antigravity_status_line(
    path: Option<&std::ffi::OsStr>,
    fallbacks: &[std::path::PathBuf],
) -> Option<&'static str> {
    crate::setup::providers::find_agy_in(path, fallbacks)?;
    Some("antigravity: log in via antigravity CLI (sign-in status unavailable)")
}

/// Resolve a provider from an explicit name, or prompt interactively when
/// multiple providers are registered and stdin is a TTY.
///
/// When stdin is NOT a TTY (scripts, harnesses, pipes) and no provider is
/// specified, fails fast with a fix-forward error listing valid providers
/// instead of hanging on read_line.
fn select_or_prompt(
    provider: Option<String>,
    registry: &[Arc<dyn OAuthProvider>],
) -> Result<Arc<dyn OAuthProvider>> {
    match provider {
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
            })
        }
        None => {
            if registry.len() == 1 {
                // Only one provider, use it directly.
                Ok(registry[0].clone())
            } else {
                // Multiple providers — need interactive selection.
                use std::io::IsTerminal;
                if !std::io::stdin().is_terminal() {
                    // Non-interactive: fail fast instead of hanging.
                    anyhow::bail!(
                        "no provider specified. Pass --provider <name> (valid: {})",
                        registry
                            .iter()
                            .map(|p| p.slug())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
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
                select_provider(input, registry)
                    .ok_or_else(|| anyhow::anyhow!("invalid selection: {input}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_contains_codex() {
        let registry = providers();
        assert!(registry.iter().any(|p| p.slug() == "codex"));
    }

    #[tokio::test]
    async fn antigravity_rejects_unsupported_headless_sign_in() {
        let error = auth_cmd(AuthCommands::Login {
            provider: Some("antigravity".into()),
            headless: true,
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("does not support --headless"));
    }

    #[tokio::test]
    async fn antigravity_logout_reports_the_explicit_limitation() {
        let error = auth_cmd(AuthCommands::Logout {
            provider: Some("antigravity".into()),
        })
        .await
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("sign-out is not available"));
        assert!(message.contains("existing account"));
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_status_is_omitted_when_agy_is_missing() {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(antigravity_status_line(None, &[]), None);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            antigravity_status_line(Some(dir.path().as_os_str()), &[]),
            None
        );

        let non_exec = dir.path().join("agy");
        std::fs::write(&non_exec, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&non_exec, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            antigravity_status_line(Some(dir.path().as_os_str()), &[]),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_status_reports_cli_when_agy_is_present() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let agy = dir.path().join("agy");
        std::fs::write(&agy, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            antigravity_status_line(Some(dir.path().as_os_str()), &[]),
            Some("antigravity: log in via antigravity CLI (sign-in status unavailable)")
        );
        assert_eq!(
            antigravity_status_line(None, &[dir.path().to_path_buf()]),
            Some("antigravity: log in via antigravity CLI (sign-in status unavailable)")
        );
    }

    #[test]
    fn test_select_provider_by_slug() {
        let registry = providers();
        let selected = select_provider("codex", &registry);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().slug(), "codex");
    }

    #[test]
    fn test_select_provider_by_number() {
        let registry = providers();
        let selected = select_provider("1", &registry);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().slug(), "codex");
    }

    #[test]
    fn test_select_provider_invalid_returns_none() {
        let registry = providers();
        assert!(select_provider("nonexistent", &registry).is_none());
        assert!(select_provider("99", &registry).is_none());
    }

    #[test]
    fn test_find_provider_by_slug() {
        let found = find_provider("codex");
        assert!(found.is_some());
        assert!(find_provider("nonexistent").is_none());
    }
}
