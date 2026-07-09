use crate::cli::ConfigCommands;
use crate::config_editor;
use anyhow::{Context, Result};
use mew_config::Config;
use tracing::info;

pub(crate) fn config_cmd(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Path => {
            println!("{}", mew_config::config_dir().join("config.toml").display());
        }
        ConfigCommands::Show => {
            let cfg = mew_config::load().context("load config")?;
            let toml = toml::to_string_pretty(&cfg)
                .map_err(|e| anyhow::anyhow!("serialize config: {}", e))?;
            print!("{}", toml);
        }
        ConfigCommands::Editor => {
            config_editor::run_editor()?;
        }
        ConfigCommands::Edit => {
            let config_dir = mew_config::config_dir();
            let config_path = config_dir.join("config.toml");

            std::fs::create_dir_all(&config_dir).context("create config directory")?;

            if !config_path.exists() {
                let template = "# mew configuration file\n\
                    # Docs: https://github.com/mewcomputer/mew\n\n\
                    # default_model = \"deepseek-v4-flash\"\n\n\
                    # [providers.my-provider]\n\
                    # shape = \"openai\"\n\
                    # base_url = \"https://api.example.com/v1\"\n\
                    # credential_ref = \"my-provider\"\n\n\
                    # [[permissions.rules]]\n\
                    # tool = \"bash\"\n\
                    # decision = \"allow\"\n\
                    # match.command_prefix = \"git \"\n";
                std::fs::write(&config_path, template).context("write config template")?;
                info!("created config template at {}", config_path.display());
            }

            let editor = std::env::var("VISUAL")
                .or_else(|_| std::env::var("EDITOR"))
                .unwrap_or_else(|_| "vi".to_string());

            let mut parts = editor.split_whitespace();
            let cmd = parts.next().unwrap_or("vi");
            let extra_args: Vec<&str> = parts.collect();

            let status = std::process::Command::new(cmd)
                .args(&extra_args)
                .arg(&config_path)
                .status()
                .with_context(|| format!("failed to launch editor '{}'", editor))?;

            if !status.success() {
                anyhow::bail!("editor exited with non-zero status");
            }
        }
    }
    Ok(())
}

pub(crate) fn hashline_enabled_for(cfg: &Config, provider_id: &str) -> bool {
    !cfg.providers
        .get(provider_id)
        .map(|p| p.disable_hashline)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_config::{Config, ProviderConfig};

    #[test]
    fn hashline_enabled_by_default() {
        let cfg = Config::default();
        // Config::default has opencode-zen with disable_hashline=false
        assert!(hashline_enabled_for(&cfg, "opencode-zen"));
    }

    #[test]
    fn hashline_disabled_when_flag_set() {
        let mut cfg = Config::default();
        cfg.providers.insert(
            "test-prov".into(),
            ProviderConfig {
                disable_hashline: true,
                ..Default::default()
            },
        );
        assert!(!hashline_enabled_for(&cfg, "test-prov"));
    }

    #[test]
    fn hashline_enabled_for_unknown_provider() {
        let cfg = Config::default();
        // Unknown provider → unwrap_or(false) → !false = true
        assert!(hashline_enabled_for(&cfg, "nonexistent-provider"));
    }
}
