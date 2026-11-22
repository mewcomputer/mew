//! Builds a disposable AGY project and CLI policy. This is not an OS sandbox.
//! No PreToolUse hook is installed by this module.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const AGENT: &str = "---\nname: mew-boundary\ndescription: Client tool bridge\nmainAgent: true\nsubagent: false\nexcludeDefaultComponents: true\ninheritMcp: true\ninheritCustomizations: false\ntools: []\nagents: []\nrules: []\nskills: []\n---\nMew's client tools are available on MCP server `mew`. Use the available MCP interface, including `call_mcp_tool` when provided, to call those tools for workspace operations.\n";

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
    tools: &[&str],
    mcp_url: &str,
    capabilities: Capabilities,
    system: &str,
) -> io::Result<Launch> {
    if capabilities.native_browser
        || capabilities.native_subagents
        || capabilities.attachments
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
    let mut names = HashSet::new();
    for name in tools {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            || !names.insert(*name)
        {
            return Err(invalid("invalid or duplicate MCP tool name"));
        }
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
    let agent_dir = workspace.path().join(".agents/agents/mew-boundary");
    fs::create_dir_all(&agent_dir)?;
    let mut agent = String::with_capacity(AGENT.len() + system.len());
    agent.push_str(AGENT);
    agent.push_str(system);
    write_private(agent_dir.join("agent.md"), agent.as_bytes())?;
    let config = serde_json::json!({ "mcpServers": { "mew": { "serverUrl": mcp_url } } });
    write_private(
        workspace.path().join(".agents/mcp_config.json"),
        config.to_string().as_bytes(),
    )?;
    let allow: Vec<_> = tools
        .iter()
        .map(|name| format!("mcp(mew/{name})"))
        .collect();
    let settings = serde_json::json!({ "permissions": { "allow": allow, "deny": ["read_file(*)", "write_file(*)", "command(*)", "read_url(*)", "execute_url(*)"] } });
    let args = vec![
        "--new-project".into(),
        "--add-dir".into(),
        workspace_arg,
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--disable-slash-commands".into(),
        "--sandbox".into(),
        "--model".into(),
        model.into(),
        "--agent".into(),
        "mew-boundary".into(),
        "--settings".into(),
        settings.to_string(),
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
