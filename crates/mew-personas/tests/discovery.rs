//! Persona discovery + frontmatter parsing tests.
//!
//! Write `PERSONA.md` files into a tempdir, point a `Loader` at it, and
//! assert the persona list, frontmatter fields, and body text come back
//! intact. Catches breakage in YAML parsing, the search-order rules, or
//! the persona-name regex.

use mew_personas::Loader;
use std::fs;
use tempfile::TempDir;

fn write_persona(root: &std::path::Path, name: &str, body: &str) {
    let dir = root.join(".mew").join("personas").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("PERSONA.md"), body).unwrap();
}

#[test]
fn discovers_a_single_persona_in_dot_mew() {
    let tmp = TempDir::new().unwrap();
    write_persona(
        tmp.path(),
        "explorer",
        "---\nname: explorer\ndescription: looks around\n---\nbody text here\n",
    );

    let personas = Loader::new(tmp.path())
        .without_builtins()
        .load()
        .expect("load");

    assert_eq!(personas.len(), 1);
    let p = &personas[0];
    assert_eq!(p.name, "explorer");
    assert_eq!(p.description, "looks around");
    assert!(p.body.contains("body text here"));
}

#[test]
fn persona_with_model_pin_and_tool_allowlist_parses_correctly() {
    let tmp = TempDir::new().unwrap();
    write_persona(
        tmp.path(),
        "builder",
        "---\nname: builder\ndescription: builds things\nmew:\n  model: z-ai/glm-4.5-air\n  tools:\n    - bash\n    - read\n---\nbuilder body\n",
    );

    let personas = Loader::new(tmp.path()).without_builtins().load().unwrap();
    assert_eq!(personas.len(), 1);
    let p = &personas[0];
    assert_eq!(p.config.model.as_deref(), Some("z-ai/glm-4.5-air"));
    let tools = p.config.tools.as_ref().expect("tools present");
    assert_eq!(tools, &vec!["bash".to_string(), "read".to_string()]);
}

#[test]
fn persona_with_tools_deny_parses_correctly() {
    let tmp = TempDir::new().unwrap();
    write_persona(
        tmp.path(),
        "readonly",
        "---\nname: readonly\ndescription: read-only\nmew:\n  tools_deny:\n    - write\n    - bash\n---\nreadonly body\n",
    );

    let personas = Loader::new(tmp.path()).without_builtins().load().unwrap();
    assert_eq!(personas.len(), 1);
    let deny = personas[0]
        .config
        .tools_deny
        .as_ref()
        .expect("deny present");
    assert_eq!(deny, &vec!["write".to_string(), "bash".to_string()]);
}

#[test]
fn persona_body_preserves_markdown_fences() {
    // The body must come through verbatim — including triple-backtick fences
    // and indented code — so the system prompt doesn't lose formatting.
    let tmp = TempDir::new().unwrap();
    let body = "---\nname: docs\ndescription: writes docs\n---\n# Heading\n\n```rust\nfn main() {}\n```\n\nSome prose.\n";
    write_persona(tmp.path(), "docs", body);

    let personas = Loader::new(tmp.path()).without_builtins().load().unwrap();
    assert_eq!(personas.len(), 1);
    let loaded = &personas[0].body;
    assert!(loaded.contains("# Heading"));
    assert!(loaded.contains("```rust"));
    assert!(loaded.contains("fn main() {}"));
    assert!(loaded.contains("Some prose."));
}

#[test]
fn persona_with_invalid_name_is_skipped_or_errors() {
    // The name regex is `^[a-z0-9]+(-[a-z0-9]+)*$` — uppercase / spaces
    // / underscores must not produce a loadable persona.
    let tmp = TempDir::new().unwrap();
    write_persona(
        tmp.path(),
        "Bad-Name",
        "---\nname: Bad-Name\ndescription: nope\n---\nbody\n",
    );

    let result = Loader::new(tmp.path()).without_builtins().load();
    // Either the load returns an error OR the bad-named persona is filtered
    // out and no personas are returned. Either is acceptable; what matters
    // is no persona with that bad name leaks through.
    match result {
        Ok(personas) => {
            assert!(
                !personas.iter().any(|p| p.name == "Bad-Name"),
                "invalid name must not be loaded"
            );
        }
        Err(_) => { /* error is also acceptable */ }
    }
}

#[test]
fn multiple_personas_all_loaded() {
    let tmp = TempDir::new().unwrap();
    write_persona(
        tmp.path(),
        "alpha",
        "---\nname: alpha\ndescription: first\n---\nalpha body\n",
    );
    write_persona(
        tmp.path(),
        "beta",
        "---\nname: beta\ndescription: second\n---\nbeta body\n",
    );
    write_persona(
        tmp.path(),
        "gamma",
        "---\nname: gamma\ndescription: third\n---\ngamma body\n",
    );

    let personas = Loader::new(tmp.path()).without_builtins().load().unwrap();
    assert_eq!(personas.len(), 3);

    let names: std::collections::HashSet<_> = personas.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains("alpha"));
    assert!(names.contains("beta"));
    assert!(names.contains("gamma"));
}

#[test]
fn persona_with_template_true_is_parsed() {
    let tmp = TempDir::new().unwrap();
    write_persona(
        tmp.path(),
        "templated",
        "---\nname: templated\ndescription: uses templates\nmew:\n  template: true\n---\n{{ persona_name }} says hi\n",
    );

    let personas = Loader::new(tmp.path()).without_builtins().load().unwrap();
    assert_eq!(personas.len(), 1);
    assert_eq!(personas[0].config.template, Some(true));
}

#[test]
fn empty_directory_yields_no_user_personas() {
    let tmp = TempDir::new().unwrap();
    let personas = Loader::new(tmp.path()).without_builtins().load().unwrap();
    assert!(
        personas.is_empty(),
        "expected no personas, got {personas:?}"
    );
}
