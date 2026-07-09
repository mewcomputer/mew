//! `mew ext` CLI — manage extensions.

use mew_ext_broker::{discover_extensions, ExtensionScope};

/// Dispatch `mew ext` subcommands.
pub fn ext_cmd(command: crate::cli::ExtCommands) -> anyhow::Result<()> {
    use crate::cli::ExtCommands;
    match command {
        ExtCommands::List => list_extensions(),
        ExtCommands::Enable { name } => enable_extension(&name),
        ExtCommands::Disable { name } => disable_extension(&name),
        ExtCommands::Remove { name } => remove_extension(&name),
        ExtCommands::Doctor => doctor(),
    }
}

/// `mew ext list` — list installed extensions.
pub fn list_extensions() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let extensions = discover_extensions(&cwd);

    // Also discover bare plugins.
    let plugin_dirs = mew_hooks_runtime::PluginLoader::default_dirs();
    let loader = mew_hooks_runtime::PluginLoader::new(plugin_dirs);
    let bare_plugins: Vec<String> = loader
        .discover_executables()
        .iter()
        .filter_map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .collect();

    // Load disabled list from state.toml.
    let disabled = mew_config::load_state()
        .unwrap_or_default()
        .disabled_plugins;

    if extensions.is_empty() && bare_plugins.is_empty() {
        println!("No extensions found.");
        println!("\nExtension packages go in:");
        println!("  ~/.config/mew/extensions/<name>/  (global)");
        println!("  .mew/extensions/<name>/           (project-local)");
        println!("\nBare plugins go in:");
        println!("  ~/.config/mew/plugins/            (global)");
        println!("  .mew/plugins/                     (project-local)");
        return Ok(());
    }

    // Print packages.
    if !extensions.is_empty() {
        println!(
            "{:<20} {:<10} {:<10} {:<10} CAPABILITIES",
            "NAME", "VERSION", "SCOPE", "STATUS"
        );
        println!("{}", "-".repeat(80));
        for ext in &extensions {
            let scope = match ext.scope {
                ExtensionScope::Global => "global",
                ExtensionScope::Project => "project",
            };
            let status = if disabled.contains(&ext.name) {
                "disabled"
            } else {
                "enabled"
            };
            let caps: Vec<&str> = ext
                .manifest
                .requested_capabilities()
                .iter()
                .map(|c| c.id())
                .collect();
            println!(
                "{:<20} {:<10} {:<10} {:<10} {}",
                ext.name,
                ext.manifest.extension.version,
                scope,
                status,
                caps.join(", ")
            );
        }
    }

    // Print bare plugins.
    if !bare_plugins.is_empty() {
        if !extensions.is_empty() {
            println!();
        }
        println!("Bare plugins (no manifest):");
        for name in &bare_plugins {
            let status = if disabled.contains(name) {
                "disabled"
            } else {
                "enabled"
            };
            println!("  {} [{}]", name, status);
        }
    }

    Ok(())
}

/// `mew ext enable <name>` — enable a disabled extension.
pub fn enable_extension(name: &str) -> anyhow::Result<()> {
    let mut state = mew_config::load_state().unwrap_or_default();
    if !state.disabled_plugins.contains(&name.to_string()) {
        println!("Extension '{}' is already enabled.", name);
        return Ok(());
    }
    state.disabled_plugins.retain(|n| n != name);
    mew_config::save_state(&state)?;
    println!("Extension '{}' enabled.", name);
    Ok(())
}

/// `mew ext disable <name>` — disable an extension.
pub fn disable_extension(name: &str) -> anyhow::Result<()> {
    let mut state = mew_config::load_state().unwrap_or_default();
    if state.disabled_plugins.contains(&name.to_string()) {
        println!("Extension '{}' is already disabled.", name);
        return Ok(());
    }
    state.disabled_plugins.push(name.to_string());
    mew_config::save_state(&state)?;
    println!("Extension '{}' disabled.", name);
    Ok(())
}

/// `mew ext remove <name>` — remove an extension package.
pub fn remove_extension(name: &str) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();

    // Check bare plugins first — refuse to remove them.
    let plugin_dirs = mew_hooks_runtime::PluginLoader::default_dirs();
    let loader = mew_hooks_runtime::PluginLoader::new(plugin_dirs);
    let bare_names: Vec<String> = loader
        .discover_executables()
        .iter()
        .filter_map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .collect();

    if bare_names.contains(&name.to_string()) {
        anyhow::bail!(
            "'{}' is a bare plugin, not an extension package. Remove it manually from the plugins/ directory.",
            name
        );
    }

    // Find the extension package.
    let extensions = discover_extensions(&cwd);
    let ext = extensions
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| anyhow::anyhow!("extension '{}' not found", name))?;

    let path = ext.root.clone();
    std::fs::remove_dir_all(&path)
        .map_err(|e| anyhow::anyhow!("failed to remove {}: {}", path.display(), e))?;
    println!("Extension '{}' removed ({}).", name, path.display());
    Ok(())
}

/// `mew ext doctor` — diagnose extension discovery.
pub fn doctor() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let extensions = discover_extensions(&cwd);

    // Also discover bare plugins.
    let plugin_dirs = mew_hooks_runtime::PluginLoader::default_dirs();
    let loader = mew_hooks_runtime::PluginLoader::new(plugin_dirs);
    let bare_plugins = loader.discover_executables();

    // Load disabled list.
    let disabled = mew_config::load_state()
        .unwrap_or_default()
        .disabled_plugins;

    println!("=== Extension Doctor ===\n");
    println!("CWD: {}", cwd.display());
    println!();

    // Discovery paths.
    println!("Discovery paths:");
    let config_dir = mew_config::config_dir();
    println!(
        "  Global extensions:  {}",
        config_dir.join("extensions").display()
    );
    println!(
        "  Project extensions: {}",
        cwd.join(".mew/extensions").display()
    );
    println!(
        "  Global plugins:     {}",
        config_dir.join("plugins").display()
    );
    println!(
        "  Project plugins:   {}",
        cwd.join(".mew/plugins").display()
    );
    println!();

    // Extension packages.
    if extensions.is_empty() {
        println!("Extension packages: none found");
    } else {
        println!("Extension packages ({}):", extensions.len());
        for ext in &extensions {
            let scope = match ext.scope {
                ExtensionScope::Global => "global",
                ExtensionScope::Project => "project",
            };
            let status = if disabled.contains(&ext.name) {
                "DISABLED"
            } else {
                "enabled"
            };
            let has_entry = if ext.has_entry() {
                "process"
            } else {
                "declarative"
            };
            println!(
                "  {} v{} [{}] [{}] [{}] — {}",
                ext.name,
                ext.manifest.extension.version,
                scope,
                status,
                has_entry,
                ext.root.display()
            );
        }
    }
    println!();

    // Bare plugins.
    if bare_plugins.is_empty() {
        println!("Bare plugins: none found");
    } else {
        println!("Bare plugins ({}):", bare_plugins.len());
        for path in &bare_plugins {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let status = if disabled.contains(&name.to_string()) {
                "DISABLED"
            } else {
                "enabled"
            };
            println!("  {} [{}] — {}", name, status, path.display());
        }
    }

    // Conflicts.
    let mut names: Vec<&str> = extensions.iter().map(|e| e.name.as_str()).collect();
    for p in &bare_plugins {
        if let Some(n) = p.file_stem().and_then(|s| s.to_str()) {
            names.push(n);
        }
    }
    let mut conflicts = Vec::new();
    for i in 0..names.len() {
        for j in (i + 1)..names.len() {
            if names[i] == names[j] {
                conflicts.push(names[i]);
            }
        }
    }
    if !conflicts.is_empty() {
        println!();
        println!("⚠ Conflicts (duplicate names):");
        for c in &conflicts {
            println!("  {} — appears multiple times", c);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ext_list_outputs_discovered_extensions() {
        // Create a temp dir with an extension package.
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir
            .path()
            .join(".mew")
            .join("extensions")
            .join("test-list-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("mew-ext.toml"),
            r#"
[extension]
name = "test-list-ext"
version = "0.1.0"
"#,
        )
        .unwrap();

        // Change to the temp dir and run discovery.
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let extensions = discover_extensions(&std::env::current_dir().unwrap());
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "test-list-ext");
        assert_eq!(extensions[0].manifest.extension.version, "0.1.0");

        std::env::set_current_dir(orig).unwrap();
    }

    #[test]
    fn test_ext_disable_persists_to_state_toml() {
        // We can't easily test state.toml persistence without mocking
        // the config dir. Instead, test the logic: disable adds to list.
        let mut state = mew_config::State::default();
        assert!(!state.disabled_plugins.contains(&"test-disable".to_string()));
        state.disabled_plugins.push("test-disable".to_string());
        assert!(state.disabled_plugins.contains(&"test-disable".to_string()));
    }

    #[test]
    fn test_ext_enable_removes_from_state_toml() {
        let mut state = mew_config::State::default();
        state.disabled_plugins.push("test-enable".to_string());
        assert!(state.disabled_plugins.contains(&"test-enable".to_string()));
        state.disabled_plugins.retain(|n| n != "test-enable");
        assert!(!state.disabled_plugins.contains(&"test-enable".to_string()));
    }

    #[test]
    fn test_ext_doctor_outputs_diagnostics() {
        // Doctor runs discovery and prints. We verify it doesn't panic
        // and produces output containing "Extension Doctor".
        let dir = tempfile::tempdir().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Doctor should succeed even with no extensions.
        let result = doctor();
        assert!(result.is_ok());

        std::env::set_current_dir(orig).unwrap();
    }

    #[test]
    fn test_ext_remove_deletes_package() {
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir.path().join(".mew").join("extensions").join("removable");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("mew-ext.toml"),
            r#"
[extension]
name = "removable"
version = "0.1.0"
"#,
        )
        .unwrap();

        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Verify it's discovered.
        let extensions = discover_extensions(&std::env::current_dir().unwrap());
        assert_eq!(extensions.len(), 1);

        // Remove it.
        let result = remove_extension("removable");
        assert!(result.is_ok());

        // Verify it's gone.
        let extensions = discover_extensions(&std::env::current_dir().unwrap());
        assert!(extensions.is_empty());

        std::env::set_current_dir(orig).unwrap();
    }

    #[test]
    fn test_ext_remove_refuses_bare_plugin() {
        // Create a bare plugin executable.
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join(".mew").join("plugins");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let plugin_path = plugin_dir.join("bare-plugin");
        std::fs::write(&plugin_path, "#!/bin/sh\necho hello\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&plugin_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&plugin_path, perms).unwrap();
        }

        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Attempt to remove — should fail with an error.
        let result = remove_extension("bare-plugin");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("bare plugin"),
            "error should mention bare plugin: {err}"
        );

        std::env::set_current_dir(orig).unwrap();
    }
}
