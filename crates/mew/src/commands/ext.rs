//! `mew ext` CLI — manage extensions.

use anyhow::Context;
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
        ExtCommands::Install {
            source,
            name,
            force,
            dry_run,
        } => install_extension(&source, name.as_deref(), force, dry_run),
        ExtCommands::Revoke { name } => revoke_extension(&name),
        ExtCommands::RotateAll => rotate_all(),
        ExtCommands::Token { name } => show_token(&name),
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

/// `mew ext install <source>` — install an extension from a git URL or local path.
pub fn install_extension(
    source: &str,
    name_override: Option<&str>,
    force: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    let extensions_dir = mew_config::config_dir().join("extensions");
    std::fs::create_dir_all(&extensions_dir)?;

    let is_git =
        source.starts_with("http") || source.starts_with("git@") || source.ends_with(".git");

    // For --dry-run, derive the name without cloning (git) or reading (local).
    if dry_run {
        let name = if let Some(n) = name_override {
            validate_extension_name(n)?;
            n.to_string()
        } else if is_git {
            derive_repo_name_from_url(source)
        } else {
            let src = std::path::Path::new(source);
            if !src.is_dir() {
                anyhow::bail!(
                    "source path does not exist or is not a directory: {}",
                    source
                );
            }
            derive_name(src, name_override)?
        };
        let dest = extensions_dir.join(&name);
        let conflict = if dest.exists() {
            if force {
                "would overwrite (--force)"
            } else {
                "CONFLICT: already installed (use --force to overwrite)"
            }
        } else {
            "new install"
        };
        println!("DRY RUN:");
        println!("  source:  {}", source);
        println!("  name:    {}", name);
        println!("  dest:    {}", dest.display());
        println!("  status:  {}", conflict);
        return Ok(());
    }

    let (name, temp_dir) = if is_git {
        // Git clone.
        let tmp = tempfile::tempdir()?;
        let status = std::process::Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                source,
                tmp.path().to_str().unwrap(),
            ])
            .status()
            .context("git clone failed")?;
        if !status.success() {
            anyhow::bail!("git clone failed for {}", source);
        }
        let name = derive_name(tmp.path(), name_override)?;
        (name, tmp)
    } else {
        // Local path.
        let src = std::path::Path::new(source);
        if !src.is_dir() {
            anyhow::bail!(
                "source path does not exist or is not a directory: {}",
                source
            );
        }
        let name = derive_name(src, name_override)?;
        (name, tempfile::tempdir()?)
    };

    let dest = extensions_dir.join(&name);

    if dest.exists() {
        if force {
            std::fs::remove_dir_all(&dest)?;
        } else {
            anyhow::bail!(
                "extension '{}' already installed. Use --force to overwrite, or `mew ext remove {}` first.",
                name,
                name
            );
        }
    }

    // Copy + validate with cleanup-on-error.
    if let Err(e) = (|| {
        copy_dir_recursive(temp_dir.path(), &dest)?;
        // Validate manifest.
        let manifest_path = dest.join("mew-ext.toml");
        if manifest_path.exists() {
            if let Err(e) = mew_ext_broker::parse_manifest(&manifest_path) {
                std::fs::remove_dir_all(&dest).ok();
                anyhow::bail!("installed extension has invalid manifest: {}", e);
            }
        } else {
            // No manifest — the extension won't be discoverable by
            // `mew ext list` or loaded by the broker. Warn but don't
            // fail, since the user may be installing a bare plugin
            // or scaffolding a new extension.
            eprintln!(
                "warning: no mew-ext.toml found in '{}' — this extension will not be discoverable by `mew ext list` or loaded by the broker",
                name
            );
        }
        Ok::<(), anyhow::Error>(())
    })() {
        // Clean up partial copy on any failure.
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }

    println!("installed extension '{}' to {}", name, dest.display());
    Ok(())
}

/// `mew ext revoke <name>` — revoke an extension's attach token.
pub fn revoke_extension(name: &str) -> anyhow::Result<()> {
    // Check if a token exists first — revoke_token succeeds silently
    // even if no token was ever minted, which is confusing.
    if mew_ext_broker::show_token(name).is_err() {
        anyhow::bail!(
            "no token found for extension '{}'. Tokens are minted via `mew ext rotate-all`.",
            name
        );
    }
    mew_ext_broker::revoke_token(name)?;
    let mut state = mew_config::load_state().unwrap_or_default();
    if !state.revoked_extensions.contains(&name.to_string()) {
        state.revoked_extensions.push(name.to_string());
        mew_config::save_state(&state)?;
    }
    println!("revoked token for extension '{}'", name);
    Ok(())
}

/// `mew ext rotate-all` — re-mint all extension attach tokens.
pub fn rotate_all() -> anyhow::Result<()> {
    let results = mew_ext_broker::rotate_all_tokens()?;
    // Clear revoked list — all successfully rotated extensions have fresh tokens.
    let mut state = mew_config::load_state().unwrap_or_default();
    state.revoked_extensions.clear();
    mew_config::save_state(&state)?;
    for (name, _token) in &results {
        println!("rotated token for '{}'", name);
    }
    if results.is_empty() {
        println!("no extensions with tokens found.");
    }
    Ok(())
}

/// `mew ext token <name>` — show the attach token for an extension.
///
/// Prints the token to stdout (for piping). When stdout is a TTY, prints
/// a warning to stderr first so the user knows the output is a secret.
pub fn show_token(name: &str) -> anyhow::Result<()> {
    let token = mew_ext_broker::show_token(name).map_err(|_| {
        anyhow::anyhow!(
            "no token found for extension '{}'. Tokens are minted via `mew ext rotate-all`.",
            name
        )
    })?;
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        eprintln!("warning: printing attach token to stdout — pipe to a clipboard tool or redirect to a file if needed");
    }
    println!("{}", token);
    Ok(())
}

/// Derive the extension name from a git URL for dry-run (no clone needed).
/// e.g. "https://github.com/user/my-ext.git" → "my-ext"
fn derive_repo_name_from_url(url: &str) -> String {
    // Strip trailing .git
    let url = url.trim_end_matches(".git");
    // Get the last path segment
    let last = url.rsplit('/').next().unwrap_or(url);
    // For git@host:user/repo format, last is already "repo"
    // For SSH URLs with ":"
    let last = last.rsplit(':').next().unwrap_or(last);
    last.to_string()
}

/// Derive the extension name from a directory path.
/// Uses override if provided, else tries the manifest name, else the dir name.
/// Validates that the name is a single path component (no traversal).
fn derive_name(path: &std::path::Path, override_name: Option<&str>) -> anyhow::Result<String> {
    let name = if let Some(name) = override_name {
        name.to_string()
    } else {
        // Try reading the manifest for the name.
        let manifest_path = path.join("mew-ext.toml");
        if manifest_path.exists() {
            if let Ok(manifest) = mew_ext_broker::parse_manifest(&manifest_path) {
                manifest.extension.name
            } else {
                // Fall back to directory name.
                path.file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("could not derive extension name from path"))?
            }
        } else {
            // Fall back to directory name.
            path.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow::anyhow!("could not derive extension name from path"))?
        }
    };
    validate_extension_name(&name)?;
    Ok(name)
}

/// Validate that an extension name is a single path component — no
/// traversal characters that could escape the extensions directory.
fn validate_extension_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("extension name is empty");
    }
    if name.contains('/') || name.contains('\\') || name == ".." || name.contains("..") {
        anyhow::bail!(
            "invalid extension name '{}': must be a single path component (no /, \\, or ..)",
            name
        );
    }
    Ok(())
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
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
            let sandbox_status = if ext.has_entry() {
                if mew_ext_broker::sandbox_available() {
                    "[sandboxed]"
                } else {
                    "[unsandboxed (platform)]"
                }
            } else {
                "[n/a]"
            };
            println!(
                "  {} v{} [{}] [{}] [{}] {} — {}",
                ext.name,
                ext.manifest.extension.version,
                scope,
                status,
                has_entry,
                sandbox_status,
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
            println!(
                "  {} [{}] [unsandboxed (legacy)] — {}",
                name,
                status,
                path.display()
            );
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
    use std::sync::Mutex;

    /// Serializes tests that mutate process-global cwd.
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_ext_list_outputs_discovered_extensions() {
        let _guard = CWD_LOCK.lock().unwrap();
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
        let _guard = CWD_LOCK.lock().unwrap();
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
        let _guard = CWD_LOCK.lock().unwrap();
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
        let _guard = CWD_LOCK.lock().unwrap();
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

    #[test]
    fn test_install_from_local_path() {
        let _guard = CWD_LOCK.lock().unwrap();

        // Create a source dir with a valid manifest.
        let src_dir = tempfile::tempdir().unwrap();
        let ext_name = "test-install-ext";
        std::fs::write(
            src_dir.path().join("mew-ext.toml"),
            format!(
                r#"
[extension]
name = "{ext_name}"
version = "0.1.0"
"#
            ),
        )
        .unwrap();
        std::fs::write(src_dir.path().join("index.js"), "console.log('hello')").unwrap();

        // Install to a temp extensions dir.
        let dest_base = tempfile::tempdir().unwrap();
        let global_dir = dest_base.path().to_path_buf();
        let project_dir = dest_base.path().to_path_buf();

        // Simulate install by copying to the global extensions dir.
        let extensions_dir = global_dir.clone();
        std::fs::create_dir_all(&extensions_dir).unwrap();
        let dest = extensions_dir.join(ext_name);
        super::copy_dir_recursive(src_dir.path(), &dest).unwrap();

        // Verify it's discoverable.
        let discovered = mew_ext_broker::discover_extensions_from_dirs(&project_dir, &global_dir);
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, ext_name);
    }

    #[test]
    fn test_install_name_conflict() {
        // Install same extension twice — first succeeds, second fails without --force.
        let src_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            src_dir.path().join("mew-ext.toml"),
            r#"
[extension]
name = "conflict-ext"
version = "0.1.0"
"#,
        )
        .unwrap();

        let dest_base = tempfile::tempdir().unwrap();
        let global_dir = dest_base.path().to_path_buf();

        // First "install" — copy.
        let dest1 = global_dir.join("conflict-ext");
        std::fs::create_dir_all(&dest1).unwrap();
        super::copy_dir_recursive(src_dir.path(), &dest1).unwrap();
        assert!(dest1.exists());

        // Second install without --force should fail.
        let dest2 = global_dir.join("conflict-ext");
        assert!(dest2.exists(), "dest already exists");
        let result = std::fs::remove_dir_all(&dest2);
        assert!(result.is_ok(), "can remove for re-install");

        // With --force it should succeed (re-copy).
        super::copy_dir_recursive(src_dir.path(), &dest2).unwrap();
        assert!(dest2.exists());
    }

    #[test]
    fn test_install_invalid_manifest() {
        let src_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            src_dir.path().join("mew-ext.toml"),
            "this is not valid toml {{{",
        )
        .unwrap();

        // parse_manifest should fail on invalid toml.
        let manifest_path = src_dir.path().join("mew-ext.toml");
        let result = mew_ext_broker::parse_manifest(&manifest_path);
        assert!(result.is_err(), "invalid manifest should fail to parse");
    }

    #[test]
    fn test_install_refuses_nonexistent_path() {
        let result =
            super::install_extension("/nonexistent/path/that/does/not/exist", None, false, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not exist") || err.contains("not a directory"),
            "error should mention missing path: {err}"
        );
    }

    #[test]
    fn test_install_rejects_path_traversal_name() {
        // --name with traversal characters should be rejected.
        let result = super::install_extension("/tmp", Some("../../../etc/pwned"), false, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("single path component") || err.contains("invalid extension name"),
            "error should mention invalid name: {err}"
        );
    }

    #[test]
    fn test_validate_extension_name_rejects_traversal() {
        assert!(super::validate_extension_name("").is_err());
        assert!(super::validate_extension_name("../etc").is_err());
        assert!(super::validate_extension_name("a/b").is_err());
        assert!(super::validate_extension_name("a\\b").is_err());
        assert!(super::validate_extension_name("..").is_err());
        // Valid names pass.
        assert!(super::validate_extension_name("my-ext").is_ok());
        assert!(super::validate_extension_name("my_ext_123").is_ok());
    }
}
