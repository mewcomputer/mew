//! Builds a disposable AGY project and CLI policy. This is not an OS sandbox.
//! No PreToolUse hook is installed by this module.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const AGENT_HEAD: &str = "---\nname: mew-boundary\ndescription: Client tool bridge\nmainAgent: true\nsubagent: false\nexcludeDefaultComponents: true\ninheritMcp: true\ninheritCustomizations: false\ntools: []\nagents: []\nrules: []\nskills: []\n---\nMew exposes client workspace tools on MCP server `mew`. Do not use `list_resources` to discover tools: it lists MCP resources, not tools, and `mew` has no resource index. If a `read` result is truncated, re-read the original workspace file with `offset` and `limit` instead of reading `.system_generated/steps/.../output.txt`. When inspecting a spilled command or search `output.txt`, pass `offset` and a small `limit` (e.g. 120 lines) or use `grep` on that file so the result stays below the inline limit.\n";
const AGENT_TAIL: &str = "Use the available MCP interface, including `call_mcp_tool` when provided, to call those tools for workspace operations.\n";

/// Capabilities not supported by this launch policy.
#[derive(Debug, Default, Clone, Copy)]
pub struct Capabilities {
    pub native_browser: bool,
    pub native_subagents: bool,
    pub attachments: bool,
    pub structured_output: bool,
}

/// Keeps the private project alive until the launch is dropped.
pub struct Launch {
    workspace: tempfile::TempDir,
    args: Vec<String>,
}

impl Launch {
    pub fn workspace(&self) -> &Path {
        self.workspace.path()
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Transfer the workspace to the process supervisor for its full lifetime.
    pub fn into_parts(self) -> (tempfile::TempDir, Vec<String>) {
        (self.workspace, self.args)
    }
}

/// Prepare the launch policy without starting the CLI or asserting isolation.
/// A process must be restarted when its system instructions or tool contract changes.
pub fn prepare(
    parent: &Path,
    model: &str,
    mcp_url: &str,
    capabilities: Capabilities,
    system: &str,
) -> io::Result<Launch> {
    prepare_with_tools(parent, model, mcp_url, capabilities, system, &[])
}

/// Same as [`prepare`], but marks every tool eager in `.agents/mcp_config.json`
/// and lists the available MCP tool names in the boundary instruction.
pub fn prepare_with_tools(
    parent: &Path,
    model: &str,
    mcp_url: &str,
    capabilities: Capabilities,
    system: &str,
    tool_names: &[String],
) -> io::Result<Launch> {
    let pairs: Vec<(&str, &str)> = tool_names
        .iter()
        .map(|name| (name.as_str(), name.as_str()))
        .collect();
    prepare_with_tool_aliases(parent, model, mcp_url, capabilities, system, &pairs)
}

/// Same as [`prepare_with_tools`], accepting `(mcp_name, original_name)` pairs
/// so aliased tools record their original Mew name in the boundary instruction
/// while registering the valid MCP name as eager in `.agents/mcp_config.json`.
pub fn prepare_with_tool_aliases(
    parent: &Path,
    model: &str,
    mcp_url: &str,
    capabilities: Capabilities,
    system: &str,
    tools: &[(&str, &str)],
) -> io::Result<Launch> {
    if capabilities.native_browser
        || capabilities.native_subagents
        || capabilities.structured_output
    {
        return Err(invalid("unsupported AGY capability"));
    }
    if model.is_empty()
        || model.len() > 128
        || !model.as_bytes()[0].is_ascii_alphanumeric()
        || !model
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(invalid("invalid AGY model name"));
    }
    let url = url::Url::parse(mcp_url).map_err(|_| invalid("invalid MCP URL"))?;
    let token = url.path().strip_prefix('/').unwrap_or_default();
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || token.len() < 16
        || token.len() > 128
        || !token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    {
        return Err(invalid("MCP URL must use a loopback capability endpoint"));
    }

    let mut builder = tempfile::Builder::new();
    builder.prefix("mew-agy-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    let workspace = builder.tempdir_in(parent)?;
    let workspace_arg = workspace
        .path()
        .to_str()
        .ok_or_else(|| invalid("workspace path is not UTF-8"))?
        .to_owned();
    let mut tools_sorted: Vec<(&str, &str)> = tools.to_vec();
    tools_sorted.sort_unstable_by_key(|(name, _)| *name);
    tools_sorted.dedup_by_key(|(name, _)| *name);

    let roster_line = if tools_sorted.is_empty() {
        "No workspace tools are registered on `mew` for this turn.\n".to_owned()
    } else {
        let labels: Vec<String> = tools_sorted
            .iter()
            .map(|(name, original)| {
                if name == original {
                    (*name).to_owned()
                } else {
                    format!("{name} (alias for {original})")
                }
            })
            .collect();
        format!(
            "Available tools on `mew` (call directly as `mcp_mew_<name>` when declared, or via `call_mcp_tool` with `ServerName: \"mew\"` and `ToolName`): {}\n",
            labels.join(", ")
        )
    };

    let agent_dir = workspace.path().join(".agents/agents/mew-boundary");
    fs::create_dir_all(&agent_dir)?;
    let mut agent = String::with_capacity(
        AGENT_HEAD.len() + roster_line.len() + AGENT_TAIL.len() + system.len(),
    );
    agent.push_str(AGENT_HEAD);
    agent.push_str(&roster_line);
    agent.push_str(AGENT_TAIL);
    agent.push_str(system);
    write_private(agent_dir.join("agent.md"), agent.as_bytes())?;
    let mut server_entry = serde_json::Map::new();
    server_entry.insert("serverUrl".into(), serde_json::json!(mcp_url));
    server_entry.insert("timeoutSeconds".into(), serde_json::json!(-1));
    if !tools_sorted.is_empty() {
        let mut eager_tools = serde_json::Map::new();
        for (name, _) in &tools_sorted {
            eager_tools.insert((*name).to_owned(), serde_json::json!({ "eager": true }));
        }
        server_entry.insert("tools".into(), serde_json::Value::Object(eager_tools));
    }
    let config =
        serde_json::json!({ "mcpServers": { "mew": serde_json::Value::Object(server_entry) } });
    write_private(
        workspace.path().join(".agents/mcp_config.json"),
        config.to_string().as_bytes(),
    )?;
    let args = vec![
        "acp".into(),
        "--add-dir".into(),
        workspace_arg,
        "--agent".into(),
        "mew-boundary".into(),
        "--include-default-customizations=false".into(),
        "--sandbox".into(),
    ];
    Ok(Launch { workspace, args })
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn write_private(path: PathBuf, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}
