use crate::cli::ThemeCommands;
use anyhow::{Context, Result};

pub(crate) fn theme_cmd(command: ThemeCommands) -> Result<()> {
    match command {
        ThemeCommands::List => {
            let names = mew_tui::theme::Theme::list_available();
            let state = mew_config::load_state().unwrap_or_default();
            let cfg = mew_config::load().unwrap_or_default();
            let active = if !state.theme.is_empty() {
                &state.theme
            } else {
                &cfg.tui.theme
            };
            let active = if active.is_empty() { "dark" } else { active };
            for name in &names {
                if name == active {
                    println!("  * {name} (active)");
                } else {
                    println!("    {name}");
                }
            }
            Ok(())
        }
        ThemeCommands::Current => {
            let state = mew_config::load_state().unwrap_or_default();
            let cfg = mew_config::load().unwrap_or_default();
            let active = if !state.theme.is_empty() {
                &state.theme
            } else {
                &cfg.tui.theme
            };
            let active = if active.is_empty() { "dark" } else { active };
            println!("{active}");
            Ok(())
        }
        ThemeCommands::Install { path } => {
            // Validate the file parses as a theme.
            let theme =
                mew_tui::theme::Theme::from_json(&path).context("failed to parse theme file")?;
            // Determine the install directory.
            let themes_dir = mew_tui::theme::Theme::themes_dir()
                .context("could not determine themes directory")?;
            std::fs::create_dir_all(&themes_dir).context("failed to create themes directory")?;
            let dest = themes_dir.join(format!("{}.json", theme.name));
            std::fs::copy(&path, &dest)
                .with_context(|| format!("failed to copy to {}", dest.display()))?;
            println!("installed theme '{}' to {}", theme.name, dest.display());
            Ok(())
        }
        ThemeCommands::ExportCss { name } => {
            let theme = mew_tui::theme::Theme::load(&name);
            println!(
                "[data-theme=\"{}\"] {{
{}}}",
                theme.name,
                theme.css_variables()
            );
            Ok(())
        }
    }
}
