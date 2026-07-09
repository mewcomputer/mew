use crate::cli::{CacheCommands, DebugCommands, VfsCommands};
use anyhow::{Context, Result};

pub(crate) async fn debug_cmd(command: DebugCommands) -> Result<()> {
    match command {
        DebugCommands::Permissions {
            tool,
            input,
            sensitivity,
        } => {
            let cfg = mew_config::load().context("load config")?;
            let engine = crate::setup::agent::build_permission_engine(
                &cfg,
                mew_hooks::PermissionMode::Standard,
            );

            let input_json: serde_json::Value = match input {
                Some(s) => serde_json::from_str(&s).context("failed to parse input JSON")?,
                None => serde_json::json!({}),
            };

            let sens = match sensitivity.as_str() {
                "readonly" | "ReadOnly" => mew_tools::Sensitivity::ReadOnly,
                "mutating" | "Mutating" => mew_tools::Sensitivity::Mutating,
                "dangerous" | "Dangerous" => mew_tools::Sensitivity::Dangerous,
                other => anyhow::bail!(
                    "unknown sensitivity '{other}'; expected readonly|mutating|dangerous"
                ),
            };

            let decision = engine
                .check(
                    &tool,
                    &input_json,
                    sens,
                    &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                )
                .await;

            println!("Tool:        {tool}");
            println!("Input:       {input_json}");
            println!("Sensitivity: {sens:?}");
            println!();
            println!("Decision:    {decision:?}");
            Ok(())
        }
        DebugCommands::Vfs { command } => match command {
            VfsCommands::Ls { path } => {
                match path {
                    None => {
                        let entries = mew_prompts::vfs::top_level();
                        for e in entries {
                            println!("{e}/");
                        }
                    }
                    Some(p) => {
                        let entries = mew_prompts::vfs::list_dir(&p);
                        if entries.is_empty() {
                            println!("(empty or not found: {p})");
                        }
                        for e in entries {
                            println!("{e}");
                        }
                    }
                }
                Ok(())
            }
            VfsCommands::Cat { path } => match mew_prompts::vfs::read_builtin(&path) {
                Some(contents) => {
                    print!("{contents}");
                    Ok(())
                }
                None => {
                    println!("not found: {path}");
                    Ok(())
                }
            },
        },
        DebugCommands::Cache { command } => match command {
            CacheCommands::Path => {
                println!("{}", mew_catalog::cache_dir().display());
                Ok(())
            }
            CacheCommands::Clear => {
                let removed = mew_catalog::clear_cache();
                if removed.is_empty() {
                    println!("no catalog cache files to remove");
                } else {
                    println!("removed {} file(s):", removed.len());
                    for p in &removed {
                        println!("  {}", p.display());
                    }
                    println!("next launch will re-fetch the catalog from the network");
                }
                Ok(())
            }
        },
    }
}
