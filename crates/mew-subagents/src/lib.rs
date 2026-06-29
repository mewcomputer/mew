use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use regex::Regex;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::debug;

#[derive(Error, Debug)]
pub enum SubagentError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid subagent name: {0}")]
    InvalidName(String),
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("execution failed: {0}")]
    Execution(String),
}

/// Outcome of a finished subagent run.
#[derive(Debug, Clone)]
pub enum SubagentResult {
    /// Subagent finished normally.
    Complete {
        text: String,
        turns_used: u32,
        /// True if the subagent hit its `max_turns` cap before producing a
        /// final response. The model should treat the result as possibly
        /// incomplete.
        hit_turn_limit: bool,
        /// True if the subagent hit its `max_duration_secs` cap. The model
        /// should treat the result as possibly incomplete.
        hit_time_limit: bool,
        /// True if the subagent's per-run session file could not be opened
        /// (e.g. read-only FS, parent meta unwritable). The subagent still
        /// ran, but no transcript was recorded for this invocation.
        session_unavailable: bool,
    },
    /// Subagent was cancelled before completion.
    Cancelled,
    /// Subagent failed with an error from the provider or tool layer.
    Error { reason: String },
}

/// Final outcome reported alongside the `Finished` event so the UI can
/// distinguish successful runs from failures and cancellations.
#[derive(Debug, Clone)]
pub enum SubagentOutcome {
    Completed,
    Cancelled,
    Failed { reason: String },
}

#[derive(Debug, Clone)]
pub struct SubagentDef {
    pub name: String,
    pub description: String,
    /// Optional model override. May be a fully-qualified `provider/model` or
    /// the tier keywords `micro`/`deci`/`nano` when the active provider is a
    /// router.
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub max_turns: Option<u32>,
    /// Wall-clock cap for the subagent's full run, in seconds. If `None`, the
    /// runner applies a built-in default (see `default_max_duration_secs`).
    pub max_duration_secs: Option<u64>,
    pub body: String,
    pub path: PathBuf,
    /// When true, render the body through minijinja before using it as
    /// the subagent's system prompt.
    pub template: bool,
}

/// Default turn cap applied to any subagent invocation that doesn't set
/// `max_turns` on its def. Intentionally generous; the wall-clock cap is the
/// primary safeguard against runaway runs.
pub const DEFAULT_MAX_TURNS: u32 = 500;

/// Default wall-clock cap (seconds) applied to any subagent invocation that
/// doesn't set `max_duration_secs` on its def.
pub const DEFAULT_MAX_DURATION_SECS: u64 = 300; // 5 minutes

/// A pool of human-friendly names the runner can pick from to give each
/// subagent run a personality. Just for fun. The list is intentionally
/// short — two simultaneous runs colliding on the same name is rare
/// (~1/30 for 2, ~1/900 for 3) and not catastrophic. Users who want
/// determinism can read the session file by id.
pub const DISPLAY_NAMES: &[&str] = &[
    "Curie",
    "Turing",
    "Lovelace",
    "Hopper",
    "Knuth",
    "Euler",
    "Hypatia",
    "Ada",
    "Noether",
    "Shannon",
    "Babbage",
    "Franklin",
    "Meitner",
    "Dijkstra",
    "Liskov",
    "Strathern",
    "Wu",
    "Sagan",
    "Elion",
    "Lamarr",
    "Yalow",
    "Hodgkin",
    "McClintock",
    "Ride",
    "Greenglass",
];

/// Pick a stable, deterministic display name from [`DISPLAY_NAMES`] given
/// a 128-bit seed. The same seed always returns the same name, so two
/// runs of the same subagent in the same session will not collide unless
/// the seed happens to match (very unlikely with 128 bits of input).
pub fn pick_display_name(seed: u128) -> &'static str {
    // Splitmix64 finalizer on the high half XOR'd with the low half.
    // Good distribution for small inputs without dragging in a hash
    // crate. Sourced from the public-domain splitmix reference impl.
    let mut x = (seed as u64) ^ (seed as u64).rotate_left(17) ^ ((seed >> 64) as u64);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;
    let idx = (x as usize) % DISPLAY_NAMES.len();
    DISPLAY_NAMES[idx]
}

#[derive(Debug, serde::Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    max_turns: Option<u32>,
    #[serde(default)]
    max_duration_secs: Option<u64>,
    #[serde(default)]
    template: bool,
}

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("valid name regex"));

pub enum SubagentEvent {
    Started {
        child_session_id: String,
        /// Optional human-friendly name for this run ("Curie", "Turing").
        /// Set when the runner picks one at spawn time; the sidebar
        /// surfaces it alongside the def name. `None` for runs that
        /// predate the display-name feature or for callers that opt out.
        display_name: Option<String>,
    },
    Finished {
        child_session_id: String,
        outcome: SubagentOutcome,
    },
    TextDelta {
        text: String,
    },
    ToolStart {
        call_id: String,
        tool_name: String,
    },
    ToolEnd {
        call_id: String,
        success: bool,
    },
    /// The subagent called a "status" tool (currently `progress_update`) to
    /// report what it's working on. The `message` is the human-readable
    /// one-liner the subagent passed in. The runner emits this *only* for
    /// tools whose semantics are "tell the parent my current status" —
    /// not for every tool call.
    Progress {
        call_id: String,
        tool_name: String,
        message: String,
    },
    PermissionRequest {
        tool_name: String,
        call_id: String,
        input: serde_json::Value,
        tx: tokio::sync::oneshot::Sender<mew_hooks::PermissionDecision>,
    },
}

/// Arguments for a single subagent invocation.
#[derive(Debug, Clone)]
pub struct SubagentRunOptions<'a> {
    pub def: &'a SubagentDef,
    pub prompt: String,
    pub parent_call_id: String,
    pub parent_session_id: mew_message::SessionId,
    pub event_tx: mpsc::Sender<SubagentEvent>,
    pub cancel: CancellationToken,
    /// Optional model override chosen by the caller at invocation time. May be
    /// a fully-qualified `provider/model` or the tier keywords
    /// `micro`/`deci`/`nano` when the active provider is a router.
    pub model: Option<String>,
}

#[async_trait::async_trait]
pub trait SubagentRunner: Send + Sync {
    async fn run(&self, opts: SubagentRunOptions<'_>) -> Result<SubagentResult, SubagentError>;
}

/// Resolves a `provider/model` string into a `Provider`. Used by runners to
/// honor per-subagent model overrides without coupling the runner crate to
/// any specific provider-building code.
#[async_trait::async_trait]
pub trait ModelResolver: Send + Sync {
    async fn resolve(&self, model: &str) -> Result<Arc<dyn mew_provider::Provider>, String>;
}

pub struct Loader {
    cwd: PathBuf,
}

impl Loader {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    pub fn load(&self) -> Result<Vec<SubagentDef>, SubagentError> {
        let spec = mew_harness::LoadSpec {
            prefixes: SUBAGENT_PREFIXES,
            file: mew_harness::LoadFileSpec::FlatMd,
        };
        let mut defs = mew_harness::load_markdown_dirs(
            &self.cwd,
            &spec,
            |path| -> Result<_, SubagentError> {
                let def = load_agent_file(path)?;
                let name = def.name.clone();
                Ok(mew_harness::Loaded { value: def, name })
            },
        )?;

        // Add built-in defaults for any not already defined by the user.
        let mut seen: std::collections::HashSet<String> =
            defs.iter().map(|d| d.name.clone()).collect();
        for def in Self::builtin_defaults() {
            if !seen.contains(&def.name) {
                seen.insert(def.name.clone());
                defs.push(def);
            }
        }

        debug!(count = defs.len(), "loaded subagent definitions");
        Ok(defs)
    }

    /// Built-in subagent definitions shipped with mew.
    pub fn builtin_defaults() -> Vec<SubagentDef> {
        vec![
            SubagentDef {
                name: "researcher".into(),
                description: "Researches topics by reading files, searching code, and finding relevant documentation.".into(),
                model: None,
                tools: Some(vec!["read".into(), "glob".into(), "grep".into()]),
                max_turns: Some(DEFAULT_MAX_TURNS),
                max_duration_secs: Some(DEFAULT_MAX_DURATION_SECS),
                body: mew_prompts::vfs::read_builtin("subagents/researcher")
                    .unwrap_or("")
                    .to_string(),
                path: PathBuf::from("(built-in)"),
                template: false,
            },
            SubagentDef {
                name: "reviewer".into(),
                description: "Reviews code changes for issues, style, and correctness.".into(),
                model: None,
                tools: Some(vec!["read".into(), "glob".into(), "grep".into()]),
                max_turns: Some(DEFAULT_MAX_TURNS),
                max_duration_secs: Some(DEFAULT_MAX_DURATION_SECS),
                body: mew_prompts::vfs::read_builtin("subagents/reviewer")
                    .unwrap_or("")
                    .to_string(),
                path: PathBuf::from("(built-in)"),
                template: false,
            },
            SubagentDef {
                name: "coder".into(),
                description: "Writes code implementations based on requirements.".into(),
                model: None,
                tools: Some(vec!["read".into(), "write".into(), "edit".into(), "glob".into(), "grep".into(), "bash".into()]),
                max_turns: Some(DEFAULT_MAX_TURNS),
                max_duration_secs: Some(DEFAULT_MAX_DURATION_SECS),
                body: mew_prompts::vfs::read_builtin("subagents/coder")
                    .unwrap_or("")
                    .to_string(),
                path: PathBuf::from("(built-in)"),
                template: false,
            },
        ]
    }
}

fn load_agent_file(path: &Path) -> Result<SubagentDef, SubagentError> {
    let content = std::fs::read_to_string(path)?;

    let parsed = if let Some(body) = content.strip_prefix("---\n") {
        if let Some((yaml, body)) = body.split_once("\n---") {
            let fm: Frontmatter = serde_yaml::from_str(yaml)?;
            let body = body.trim_start_matches('\n').to_string();
            Some((fm, body))
        } else {
            None
        }
    } else {
        None
    };

    let (name, description, model, tools, max_turns, max_duration_secs, body, template) =
        match parsed {
            Some((fm, body)) => {
                validate_name(&fm.name)?;
                (
                    fm.name,
                    fm.description,
                    fm.model,
                    fm.tools,
                    fm.max_turns,
                    fm.max_duration_secs,
                    body,
                    fm.template,
                )
            }
            None => {
                let file_stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                validate_name(&file_stem)?;
                (
                    file_stem,
                    String::new(),
                    None,
                    None,
                    None,
                    None,
                    content,
                    false,
                )
            }
        };

    Ok(SubagentDef {
        name,
        description,
        model,
        tools,
        max_turns,
        max_duration_secs,
        body,
        path: path.to_path_buf(),
        template,
    })
}

fn validate_name(name: &str) -> Result<(), SubagentError> {
    if name.len() > 64 {
        return Err(SubagentError::InvalidName(format!(
            "name too long (max 64): {name}"
        )));
    }
    if !NAME_RE.is_match(name) {
        return Err(SubagentError::InvalidName(format!(
            "invalid name: {name}. Must match [a-z0-9]+(-[a-z0-9]+)*"
        )));
    }
    Ok(())
}

const SUBAGENT_PREFIXES: &[&str] = &[
    ".mew/agents",
    ".opencode/agents",
    ".claude/agents",
    ".agents",
];

pub fn apply_config_overrides(
    defs: &mut [SubagentDef],
    overrides: &std::collections::HashMap<String, AgentConfigOverride>,
) {
    for def in defs.iter_mut() {
        if let Some(ov) = overrides.get(&def.name) {
            if let Some(ref model) = ov.model {
                def.model = Some(model.clone());
            }
            if let Some(max_turns) = ov.max_turns {
                def.max_turns = Some(max_turns);
            }
            if let Some(max_duration_secs) = ov.max_duration_secs {
                def.max_duration_secs = Some(max_duration_secs);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AgentConfigOverride {
    pub model: Option<String>,
    pub max_turns: Option<u32>,
    pub max_duration_secs: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_agent(dir: &Path, name: &str, description: &str, body: &str) {
        let agents_dir = dir.join(".mew").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        let content = format!("---\nname: {name}\ndescription: {description}\n---\n{body}");
        std::fs::write(agents_dir.join(format!("{name}.md")), content).unwrap();
    }

    fn write_agent_full(
        dir: &Path,
        name: &str,
        description: &str,
        model: Option<&str>,
        tools: Option<&[&str]>,
        max_turns: Option<u32>,
        max_duration_secs: Option<u64>,
        body: &str,
    ) {
        let agents_dir = dir.join(".mew").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        let mut fm = format!("---\nname: {name}\ndescription: {description}\n");
        if let Some(m) = model {
            fm.push_str(&format!("model: \"{m}\"\n"));
        }
        if let Some(t) = tools {
            fm.push_str(&format!("tools: {:?}\n", t));
        }
        if let Some(mt) = max_turns {
            fm.push_str(&format!("max_turns: {mt}\n"));
        }
        if let Some(mds) = max_duration_secs {
            fm.push_str(&format!("max_duration_secs: {mds}\n"));
        }
        fm.push_str(&format!("---\n{body}"));
        std::fs::write(agents_dir.join(format!("{name}.md")), fm).unwrap();
    }

    #[test]
    fn test_load_single_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_agent(
            cwd,
            "code-reviewer",
            "Reviews code changes",
            "You are a code reviewer.",
        );

        let loader = Loader::new(cwd);
        let defs = loader.load().unwrap();
        let user_defs: Vec<_> = defs
            .iter()
            .filter(|d| d.path != PathBuf::from("(built-in)"))
            .collect();
        assert_eq!(user_defs.len(), 1);
        assert_eq!(user_defs[0].name, "code-reviewer");
        assert_eq!(user_defs[0].description, "Reviews code changes");
        assert!(user_defs[0].body.contains("You are a code reviewer."));
        assert!(user_defs[0].model.is_none());
        assert!(user_defs[0].tools.is_none());
    }

    #[test]
    fn test_load_agent_with_all_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_agent_full(
            cwd,
            "explore",
            "Fast exploration",
            Some("inherit"),
            Some(&["read", "glob", "grep"]),
            Some(15),
            Some(120),
            "You are a codebase explorer.",
        );

        let loader = Loader::new(cwd);
        let defs = loader.load().unwrap();
        let user_defs: Vec<_> = defs
            .iter()
            .filter(|d| d.path != PathBuf::from("(built-in)"))
            .collect();
        assert_eq!(user_defs.len(), 1);
        let def = &user_defs[0];
        assert_eq!(def.model.as_deref(), Some("inherit"));
        let tools = def.tools.as_ref().unwrap();
        assert_eq!(tools, &["read", "glob", "grep"].map(String::from));
        assert_eq!(def.max_turns, Some(15));
        assert_eq!(def.max_duration_secs, Some(120));
    }

    #[test]
    fn test_duplicate_name_project_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_agent(cwd, "test-agent", "First", "body1");

        let agents_dir = cwd.join(".opencode").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(
            agents_dir.join("test-agent.md"),
            "---\nname: test-agent\ndescription: Second\n---\nbody2",
        )
        .unwrap();

        let loader = Loader::new(cwd);
        let defs = loader.load().unwrap();
        let user_defs: Vec<_> = defs
            .iter()
            .filter(|d| d.path != PathBuf::from("(built-in)"))
            .collect();
        assert_eq!(user_defs.len(), 1);
        assert_eq!(user_defs[0].description, "First");
    }

    #[test]
    fn test_name_validation() {
        assert!(validate_name("valid-name").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("code-reviewer").is_ok());
        assert!(validate_name("INVALID").is_err());
        assert!(validate_name("has_underscore").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn test_name_too_long() {
        let long = "a".repeat(65);
        assert!(validate_name(&long).is_err());
    }

    #[test]
    fn test_name_mismatch_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        let agents_dir = cwd.join(".mew").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(
            agents_dir.join("a-name.md"),
            "---\nname: different-name\ndescription: desc\n---\nbody",
        )
        .unwrap();

        let loader = Loader::new(cwd);
        let defs = loader.load().unwrap();
        let user_defs: Vec<_> = defs
            .iter()
            .filter(|d| d.path != PathBuf::from("(built-in)"))
            .collect();
        assert!(user_defs.is_empty());
    }

    #[test]
    fn test_load_no_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let loader = Loader::new(tmp.path());
        let defs = loader.load().unwrap();
        // Built-in defaults are always present.
        assert_eq!(defs.len(), Loader::builtin_defaults().len());
    }

    #[test]
    fn test_agent_without_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        let agents_dir = cwd.join(".mew").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("no-fm.md"), "Just a body").unwrap();

        let loader = Loader::new(cwd);
        let defs = loader.load().unwrap();
        let user_defs: Vec<_> = defs
            .iter()
            .filter(|d| d.path != PathBuf::from("(built-in)"))
            .collect();
        assert_eq!(user_defs.len(), 1);
        assert_eq!(user_defs[0].name, "no-fm");
        assert_eq!(user_defs[0].description, "");
        assert_eq!(user_defs[0].body, "Just a body");
    }

    #[test]
    fn test_config_override() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_agent_full(cwd, "explore", "desc", None, None, None, None, "body");

        let loader = Loader::new(cwd);
        let mut defs = loader.load().unwrap();

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "explore".to_string(),
            AgentConfigOverride {
                model: Some("opencode-zen/deepseek-v4-flash".to_string()),
                max_turns: Some(10),
                max_duration_secs: Some(45),
                ..Default::default()
            },
        );
        apply_config_overrides(&mut defs, &overrides);

        assert_eq!(
            defs[0].model.as_deref(),
            Some("opencode-zen/deepseek-v4-flash")
        );
        assert_eq!(defs[0].max_turns, Some(10));
        assert_eq!(defs[0].max_duration_secs, Some(45));
    }

    #[test]
    fn test_config_override_router_model() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        write_agent(cwd, "explore", "desc", "body");

        let loader = Loader::new(cwd);
        let mut defs = loader.load().unwrap();

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "explore".to_string(),
            AgentConfigOverride {
                model: Some("micro".to_string()),
                ..Default::default()
            },
        );
        apply_config_overrides(&mut defs, &overrides);

        assert_eq!(defs[0].model.as_deref(), Some("micro"));
    }

    #[test]
    fn test_pick_display_name_is_deterministic() {
        let s = 0xdeadbeefu128;
        assert_eq!(pick_display_name(s), pick_display_name(s));
    }

    #[test]
    fn test_pick_display_name_returns_known_value() {
        // Same seed → same name. Pin the name so we notice if anyone
        // reorders the list (which would silently break user muscle
        // memory like "the Curie run").
        let s: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
        let n = pick_display_name(s);
        assert!(DISPLAY_NAMES.contains(&n));
    }

    #[test]
    fn test_pick_display_name_covers_full_pool() {
        // Across 1000 fresh ulids, every name in the pool should be hit
        // at least once. With 25 names and a 64-bit hash, expected
        // collision-free count for any single name is ~40. The smallest
        // count should be well above zero.
        use std::collections::HashSet;
        let mut seen: HashSet<&'static str> = HashSet::new();
        for i in 0u128..1000 {
            seen.insert(pick_display_name(i));
        }
        assert_eq!(
            seen.len(),
            DISPLAY_NAMES.len(),
            "all names should be reachable; got {seen:?}"
        );
    }

    #[test]
    fn test_pick_display_name_avoids_immediate_collisions() {
        // Two adjacent seeds should rarely collide. With 25 names and a
        // 64-bit hash, expected probability of any single pair colliding
        // is ~1/25. Across 1000 adjacent pairs, we expect ~40 collisions
        // — generous. Set a ceiling so a broken hash function (e.g. one
        // that returns the same name for every input) trips this test.
        use std::collections::HashMap;
        let mut counts: HashMap<&'static str, u32> = HashMap::new();
        for i in 0u128..1000 {
            *counts.entry(pick_display_name(i)).or_insert(0) += 1;
        }
        let max = counts.values().copied().max().unwrap();
        // No name should be picked more than 80 times in 1000 draws; a
        // uniform distribution over 25 names would give 40.
        assert!(max < 80, "distribution too skewed: {counts:?}");
    }
}
