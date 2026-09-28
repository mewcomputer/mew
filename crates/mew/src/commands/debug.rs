use crate::cli::{CacheCommands, DebugCommands, VfsCommands};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

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
        DebugCommands::Context { full } => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let cfg = mew_config::load().context("load config")?;
            let cat = crate::setup::providers::load_catalog(&cfg).await;
            let state = mew_config::load_state().unwrap_or_default();
            let provider_flag = crate::setup::providers::resolve_provider(None, &state, &cfg);
            let model_flag = crate::setup::providers::resolve_model_opt(None, &state, &cfg);
            let (provider_id, model_id) = crate::setup::providers::resolve_model(
                &cfg,
                cat.as_ref(),
                &provider_flag,
                model_flag,
            );
            // Assemble once; both the tree and --full are views of the same
            // resolved build, so VFS transclusions match the printed prompt.
            let (prompt, vfs) = crate::setup::agent::build_system_prompt_snapshot(
                &cfg,
                cat.as_ref(),
                &provider_id,
                &model_id,
                &cwd,
            )
            .context("assemble system prompt")?;

            if full {
                print!("{prompt}");
                if !prompt.is_empty() && !prompt.ends_with('\n') {
                    println!();
                }
                return Ok(());
            }

            let files = mew_context::Loader::new(&cwd)
                .load()
                .context("load context files")?;
            if files.is_empty() && vfs.is_empty() {
                println!("(no context files found)");
                return Ok(());
            }
            let home = std::env::var_os("HOME").map(PathBuf::from);
            print!("{}", render_context_tree(&files, &vfs, home.as_deref()));
            Ok(())
        }
    }
}

/// One node in the source tree: a directory grouping or a file leaf.
#[derive(Default)]
struct ContextNode {
    children: std::collections::BTreeMap<String, ContextNode>,
    is_file: bool,
    template: bool,
}

/// Render the resolved context sources as a directory tree with box-drawing
/// guides: the filesystem context files plus the built-in VFS resources
/// inlined through `transclude`, grouped under a `mew://` node.
///
/// Shared path prefixes are grouped, and runs of single-child directories
/// collapse onto one line. `home` abbreviates the home directory as `~`.
fn render_context_tree(files: &[mew_context::File], vfs: &[String], home: Option<&Path>) -> String {
    let mut root = ContextNode::default();

    for file in files {
        let display = abbreviate_home(&file.path, home);
        let node = insert_components(&mut root, display.components().filter_map(component_name));
        node.is_file = true;
        node.template = file.template;
    }

    for resource in vfs {
        let node = insert_components(
            &mut root,
            std::iter::once("mew://".to_string()).chain(
                resource
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            ),
        );
        node.is_file = true;
    }

    let mut out = String::new();
    write_tree_children(&root, "", &mut out);
    out
}

/// Insert `components` as a path under `root`, creating nodes along the way,
/// and return the leaf node.
fn insert_components(
    root: &mut ContextNode,
    components: impl Iterator<Item = String>,
) -> &mut ContextNode {
    let mut node = root;
    for name in components {
        node = node.children.entry(name).or_default();
    }
    node
}

fn component_name(component: std::path::Component<'_>) -> Option<String> {
    match component {
        std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        std::path::Component::Prefix(prefix) => {
            Some(prefix.as_os_str().to_string_lossy().into_owned())
        }
        // RootDir / CurDir / ParentDir carry no name worth showing.
        _ => None,
    }
}

fn abbreviate_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(relative) => Path::new("~").join(relative),
        None => path.to_path_buf(),
    }
}

fn write_tree_children(node: &ContextNode, prefix: &str, out: &mut String) {
    let mut children = node.children.iter().peekable();
    while let Some((name, child)) = children.next() {
        let last = children.peek().is_none();
        write_tree_node(name, child, prefix, last, out);
    }
}

fn write_tree_node(name: &str, node: &ContextNode, prefix: &str, last: bool, out: &mut String) {
    let connector = if last { "└── " } else { "├── " };

    // Collapse a run of single-child directories into one joined label so
    // deep paths stay on a single line. A label already ending in `/` (the
    // `mew://` VFS root) is left as-is to avoid `mew:///...`.
    let mut label = name.to_string();
    let mut current = node;
    while !current.is_file && current.children.len() == 1 && !label.ends_with('/') {
        let (child_name, child) = current.children.iter().next().expect("len == 1");
        label.push('/');
        label.push_str(child_name);
        current = child;
    }

    out.push_str(prefix);
    out.push_str(connector);
    out.push_str(&label);
    if current.is_file && current.template {
        out.push_str("  (template)");
    }
    out.push('\n');

    let child_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
    write_tree_children(current, &child_prefix, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_file(path: &str, template: bool) -> mew_context::File {
        mew_context::File {
            path: PathBuf::from(path),
            content: String::new(),
            template,
        }
    }

    #[test]
    fn tree_groups_by_directory_and_marks_templates() {
        let files = vec![
            ctx_file("/proj/AGENTS.md", false),
            ctx_file("/proj/.mew/AGENTS.md", true),
            ctx_file("/proj/.mew/wiki.md", false),
        ];
        assert_eq!(
            render_context_tree(&files, &[], None),
            "└── proj\n    ├── .mew\n    │   ├── AGENTS.md  (template)\n    │   └── wiki.md\n    └── AGENTS.md\n"
        );
    }

    #[test]
    fn tree_collapses_single_child_chains() {
        let files = vec![ctx_file("/a/b/c/file.md", false)];
        assert_eq!(
            render_context_tree(&files, &[], None),
            "└── a/b/c/file.md\n"
        );
    }

    #[test]
    fn tree_abbreviates_home() {
        let files = vec![ctx_file("/home/u/.config/mew/AGENTS.md", false)];
        assert_eq!(
            render_context_tree(&files, &[], Some(Path::new("/home/u"))),
            "└── ~/.config/mew/AGENTS.md\n"
        );
    }

    #[test]
    fn tree_renders_resolved_vfs_resources_under_mew_scheme() {
        let vfs = vec![
            "system_prompts/base_openai".to_string(),
            "system_prompts/_tool_library".to_string(),
            "personas/builder".to_string(),
        ];
        assert_eq!(
            render_context_tree(&[], &vfs, None),
            "└── mew://\n    ├── personas/builder\n    └── system_prompts\n        ├── _tool_library\n        └── base_openai\n"
        );
    }

    #[test]
    fn tree_combines_files_and_vfs() {
        let files = vec![
            ctx_file("/proj/AGENTS.md", false),
            ctx_file("/proj/CLAUDE.md", false),
        ];
        let vfs = vec!["system_prompts/base_openai".to_string()];
        let tree = render_context_tree(&files, &vfs, None);
        assert!(tree.contains("mew://"), "{tree}");
        assert!(tree.contains("base_openai"), "{tree}");
        assert!(tree.contains("proj"), "{tree}");
    }

    #[test]
    fn tree_is_empty_without_sources() {
        assert_eq!(render_context_tree(&[], &[], None), "");
    }
}
