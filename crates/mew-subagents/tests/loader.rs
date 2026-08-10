//! Subagent loader + display-name picker tests.
//!
//! Write `*.md` subagent definitions into a tempdir, point a `Loader` at it,
//! and assert the loaded list, frontmatter fields, and display-name picker
//! behave as documented. Catches breakage in YAML parsing or in the
//! deterministic name distribution.

use mew_subagents::{pick_display_name, Loader, DISPLAY_NAMES};
use std::fs;
use tempfile::TempDir;

fn write_subagent(root: &std::path::Path, file_name: &str, body: &str) {
    let dir = root.join(".mew").join("agents");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(file_name), body).unwrap();
}

#[test]
fn loader_picks_up_user_subagents() {
    let tmp = TempDir::new().unwrap();
    write_subagent(
        tmp.path(),
        "researcher.md",
        "---\nname: researcher\ndescription: searches the web\n---\nresearcher body\n",
    );
    write_subagent(
        tmp.path(),
        "explorer.md",
        "---\nname: explorer\ndescription: looks around\nmodel: opencode-zen\nmax_turns: 25\n---\nexplorer body\n",
    );

    let defs = Loader::new(tmp.path()).load().expect("load");
    let by_name: std::collections::HashMap<_, _> =
        defs.iter().map(|d| (d.name.as_str(), d)).collect();

    let r = by_name.get("researcher").expect("researcher");
    assert_eq!(r.description, "searches the web");
    assert!(r.model.is_none());

    let e = by_name.get("explorer").expect("explorer");
    assert_eq!(e.model.as_deref(), Some("opencode-zen"));
    assert_eq!(e.max_turns, Some(25));
}

#[test]
fn loader_includes_builtin_defaults_unless_user_overrides() {
    let tmp = TempDir::new().unwrap();
    let defs = Loader::new(tmp.path()).load().unwrap();
    // Built-in defaults include "researcher" and "explorer" (per the
    // source). At minimum the loader must return a non-empty list when
    // no user files exist.
    assert!(
        !defs.is_empty(),
        "expected built-in subagent defaults when no user files present"
    );
}

#[test]
fn user_defined_subagent_replaces_builtin_with_same_name() {
    let tmp = TempDir::new().unwrap();
    write_subagent(
        tmp.path(),
        "explorer.md",
        "---\nname: explorer\ndescription: custom user-defined explorer\nmax_turns: 7\n---\ncustom body\n",
    );

    let defs = Loader::new(tmp.path()).load().unwrap();
    let explorers: Vec<_> = defs.iter().filter(|d| d.name == "explorer").collect();
    assert_eq!(
        explorers.len(),
        1,
        "user-defined explorer must replace, not duplicate, the built-in"
    );
    assert_eq!(explorers[0].max_turns, Some(7));
    assert_eq!(explorers[0].description, "custom user-defined explorer");
}

#[test]
fn subagent_with_tool_allowlist_parses_correctly() {
    let tmp = TempDir::new().unwrap();
    write_subagent(
        tmp.path(),
        "scoper.md",
        "---\nname: scoper\ndescription: limited toolset\ntools:\n  - read\n  - glob\n  - grep\n---\nbody\n",
    );

    let defs = Loader::new(tmp.path()).load().unwrap();
    let s = defs.iter().find(|d| d.name == "scoper").expect("scoper");
    let tools = s.tools.as_ref().expect("tools present");
    assert_eq!(
        tools,
        &vec!["read".to_string(), "glob".to_string(), "grep".to_string()]
    );
}

#[test]
fn empty_directory_yields_builtin_defaults() {
    let tmp = TempDir::new().unwrap();
    let defs = Loader::new(tmp.path()).load().unwrap();
    // No user files but built-ins still come through.
    assert!(!defs.is_empty());
}

#[test]
fn display_name_pool_is_nonempty_and_unique() {
    // The pool must be at least 10 entries (per the docs — 25 default names)
    // and no two entries may be identical, since the picker relies on the
    // distribution being uniform across distinct names.
    assert!(
        DISPLAY_NAMES.len() >= 10,
        "display name pool should have at least 10 entries; got {}",
        DISPLAY_NAMES.len()
    );

    let mut sorted: Vec<&str> = DISPLAY_NAMES.to_vec();
    sorted.sort();
    let unique: std::collections::HashSet<_> = sorted.iter().collect();
    assert_eq!(
        unique.len(),
        sorted.len(),
        "display name pool must have no duplicates"
    );
}

#[test]
fn display_name_is_deterministic_for_same_seed() {
    let n1 = pick_display_name(42);
    let n2 = pick_display_name(42);
    assert_eq!(n1, n2, "same seed must produce the same display name");
}

#[test]
fn different_seeds_produce_varied_display_names() {
    // Sample 10 different seeds; we should see at least 2 distinct names
    // (very likely 5+ with a pool of 25, but 2 is the conservative floor
    // to keep the test robust against pool changes).
    let names: std::collections::HashSet<_> = (0..10)
        .map(|seed| pick_display_name(seed as u128))
        .collect();
    assert!(
        names.len() >= 2,
        "10 different seeds should yield at least 2 distinct names; got {}",
        names.len()
    );
}

#[test]
fn display_name_is_from_the_known_pool() {
    for seed in 0..50 {
        let name = pick_display_name(seed as u128);
        assert!(
            DISPLAY_NAMES.contains(&name),
            "seed {seed} produced {name:?} which is not in DISPLAY_NAMES"
        );
    }
}

#[test]
fn loader_parses_can_spawn_frontmatter() {
    let tmp = TempDir::new().unwrap();
    write_subagent(
        tmp.path(),
        "orchestrator.md",
        "---\nname: orchestrator\ndescription: nests\ncan_spawn: true\n---\nbody\n",
    );
    write_subagent(
        tmp.path(),
        "plain.md",
        "---\nname: plain\ndescription: no nest\n---\nbody\n",
    );

    let defs = Loader::new(tmp.path()).load().expect("load");
    let by_name: std::collections::HashMap<_, _> =
        defs.iter().map(|d| (d.name.as_str(), d)).collect();

    assert!(by_name.get("orchestrator").expect("orchestrator").can_spawn);
    assert!(!by_name.get("plain").expect("plain").can_spawn);
}

#[test]
fn loader_resolves_output_schema_yaml_map_and_at_path() {
    let tmp = TempDir::new().unwrap();
    write_subagent(
        tmp.path(),
        "typed.md",
        "---\nname: typed\ndescription: typed output\noutput_schema:\n  type: object\n  required: [answer]\n---\nbody\n",
    );
    let schema_dir = tmp.path().join(".mew").join("agents").join("schemas");
    fs::create_dir_all(&schema_dir).unwrap();
    fs::write(
        schema_dir.join("report.json"),
        r#"{"type":"object","properties":{"title":{"type":"string"}}}"#,
    )
    .unwrap();
    write_subagent(
        tmp.path(),
        "referenced.md",
        "---\nname: referenced\ndescription: file schema\noutput_schema: \"@schemas/report.json\"\n---\nbody\n",
    );

    let defs = Loader::new(tmp.path()).load().expect("load");
    let by_name: std::collections::HashMap<_, _> =
        defs.iter().map(|d| (d.name.as_str(), d)).collect();

    let typed = by_name.get("typed").expect("typed");
    let schema = typed.output_schema.as_ref().expect("schema");
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"][0], "answer");

    let referenced = by_name.get("referenced").expect("referenced");
    let schema = referenced.output_schema.as_ref().expect("schema");
    assert_eq!(schema["properties"]["title"]["type"], "string");

    // Defs without output_schema stay None.
    assert!(by_name
        .get("researcher")
        .expect("researcher")
        .output_schema
        .is_none());
}

#[test]
fn loader_skips_def_with_missing_output_schema_file() {
    // Loader policy: a def that fails to load (bad YAML, missing @path
    // schema) is skipped with a debug log rather than failing the whole load.
    let tmp = TempDir::new().unwrap();
    write_subagent(
        tmp.path(),
        "broken.md",
        "---\nname: broken\ndescription: bad ref\noutput_schema: \"@schemas/nope.json\"\n---\nbody\n",
    );
    let defs = Loader::new(tmp.path()).load().expect("load");
    assert!(!defs.iter().any(|d| d.name == "broken"));
}
