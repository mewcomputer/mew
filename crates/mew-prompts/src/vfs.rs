//! Virtual filesystem for built-in resources.
//!
//! The `include_dir!` macro embeds the `resources/` directory at compile
//! time, so every file there is accessible as a `&'static str` at runtime
//! without any filesystem I/O. This is the Rust equivalent of Go's
//! `//go:embed` directive.
//!
//! Resources are addressed by their path relative to the `resources/`
//! root. For example, `resources/personas/builder.md` is read as
//! `"personas/builder.md"`.
//!
//! ## Shadowing
//!
//! The VFS only provides *built-in* resources. User-defined overrides
//! live on disk (e.g. `.mew/personas/builder.md`) and are loaded by
//! the existing disk loaders. A user file shadows the built-in when
//! both exist; the loaders handle this naturally without the VFS
//! needing to know.
//!
//! ## URI scheme
//!
//! The full VFS URI for a resource is `mew://<path>`. The VFS API
//! here takes the path *without* the scheme — callers that parse URIs
//! should strip the `mew://` prefix before calling.

use include_dir::{include_dir, Dir};

static BUILTIN: Dir = include_dir!("$CARGO_MANIFEST_DIR/resources");

/// Read a built-in resource by its path relative to the `resources/`
/// root. The `.md` extension is optional — pass either
/// `"personas/builder"` or `"personas/builder.md"`.
///
/// Example: `read_builtin("personas/builder")` returns the body of
/// `resources/personas/builder.md`.
pub fn read_builtin(path: &str) -> Option<&'static str> {
    // Try the path as-given first, then with `.md` appended. Keeps the
    // API ergonomic while letting the on-disk files keep their extension.
    let file = if let Some(f) = BUILTIN.get_file(path) {
        f
    } else {
        let md_path = format!("{path}.md");
        BUILTIN.get_file(&md_path)?
    };
    file.contents_utf8()
}

/// List the names of all top-level entries in the `resources/` directory.
/// Useful for `mew vfs ls mew://` style discovery.
pub fn top_level() -> Vec<&'static str> {
    BUILTIN
        .entries()
        .iter()
        .map(|e| e.path().display().to_string())
        .filter_map(|p| {
            p.split('/').next().map(|s| {
                // Leak the string so we get `&'static str`; the set is bounded
                // and this is only called for discovery (not in hot paths).
                Box::leak(s.to_string().into_boxed_str()) as &'static str
            })
        })
        .collect()
}

/// List all files under a directory in the VFS. Returns the paths relative
/// to the resources root. E.g. `list_dir("personas")` returns
/// `["personas/builder.md", "personas/planner.md"]`.
pub fn list_dir(dir: &str) -> Vec<String> {
    match BUILTIN.get_dir(dir) {
        Some(d) => d
            .entries()
            .iter()
            .map(|e| e.path().display().to_string())
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_builtin_known_files() {
        // The migration wires the real files; if any of these are missing,
        // the migration is incomplete. These tests double as a checklist.
        assert!(
            read_builtin("personas/builder").is_some(),
            "resources/personas/builder.md must be embedded"
        );
        assert!(
            read_builtin("personas/planner").is_some(),
            "resources/personas/planner.md must be embedded"
        );
        assert!(
            read_builtin("subagents/researcher").is_some(),
            "resources/subagents/researcher.md must be embedded"
        );
        assert!(
            read_builtin("subagents/reviewer").is_some(),
            "resources/subagents/reviewer.md must be embedded"
        );
        assert!(
            read_builtin("subagents/coder").is_some(),
            "resources/subagents/coder.md must be embedded"
        );
    }

    #[test]
    fn test_read_builtin_unknown_path_returns_none() {
        assert_eq!(read_builtin("personas/nonexistent"), None);
        assert_eq!(read_builtin("does/not/exist"), None);
        assert_eq!(read_builtin(""), None);
    }

    #[test]
    fn test_top_level_lists_first_segment() {
        let mut names: Vec<&str> = top_level();
        names.sort();
        names.dedup();
        assert!(names.contains(&"personas"));
        assert!(names.contains(&"subagents"));
    }
}
