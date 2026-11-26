use mew_agy::launch::{prepare, prepare_with_tool_aliases, prepare_with_tools, Capabilities};
use std::fs;

#[test]
fn agent_instructions_identify_the_client_mcp_tool_server() {
    let parent = tempfile::tempdir().unwrap();
    let launch = prepare(
        parent.path(),
        "model",
        "http://127.0.0.1:12345/0123456789abcdef",
        Capabilities::default(),
        "",
    )
    .unwrap();
    let instructions = fs::read_to_string(
        launch
            .workspace()
            .join(".agents/agents/mew-boundary/agent.md"),
    )
    .unwrap();
    assert!(instructions.contains("MCP server `mew`"));
    assert!(instructions.contains("call_mcp_tool"));
    assert!(instructions.contains("Do not use `list_resources` to discover tools"));
    assert!(instructions.contains(".system_generated/steps/.../output.txt"));
    assert!(instructions.contains("No workspace tools are registered on `mew` for this turn."));
    assert!(!instructions.contains("web_search"));
    assert!(!instructions.contains("web_fetch"));
    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(launch.workspace().join(".agents/mcp_config.json")).unwrap(),
    )
    .unwrap();
    assert!(config["mcpServers"]["mew"].get("tools").is_none());
}

#[test]
fn creates_private_workspace_with_minimal_agent_and_exact_permissions() {
    let parent = tempfile::tempdir().unwrap();
    let launch = prepare(
        parent.path(),
        "gemini-4-argon-low",
        "http://127.0.0.1:12345/0123456789abcdef0123456789abcdef",
        Capabilities::default(),
        "",
    )
    .unwrap();
    let workspace = launch.workspace();
    let agent = fs::read_to_string(workspace.join(".agents/agents/mew-boundary/agent.md")).unwrap();
    for setting in [
        "mainAgent: true",
        "subagent: false",
        "excludeDefaultComponents: true",
        "inheritMcp: true",
        "inheritCustomizations: false",
        "tools: []",
        "agents: []",
        "rules: []",
        "skills: []",
    ] {
        assert!(agent.contains(setting), "missing {setting}");
    }
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(workspace.join(".agents/mcp_config.json")).unwrap())
            .unwrap();
    assert_eq!(
        config["mcpServers"]["mew"]["serverUrl"],
        "http://127.0.0.1:12345/0123456789abcdef0123456789abcdef"
    );
    assert_eq!(config["mcpServers"]["mew"]["timeoutSeconds"], -1);
    let args = launch.args();
    assert!(!args.iter().any(|a| a == "--dangerously-skip-permissions"));
    assert_eq!(args.first().map(String::as_str), Some("acp"));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["--agent", "mew-boundary"]));
    assert!(args
        .iter()
        .any(|a| a == "--include-default-customizations=false"));
    assert!(args.iter().any(|a| a == "--sandbox"));
    assert!(!args.iter().any(|a| a == "--settings"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(workspace).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(workspace.join(".agents/mcp_config.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let path = workspace.to_path_buf();
    drop(launch);
    assert!(!path.exists());
}

#[test]
fn rejects_unsafe_models_and_urls_before_writing() {
    let parent = tempfile::tempdir().unwrap();
    let url = "http://127.0.0.1:12345/0123456789abcdef0123456789abcdef";
    for invalid in [
        "https://example.com/0123456789abcdef",
        "http://127.0.0.1:12345/",
        "http://localhost:12345/0123456789abcdef",
        "http://user@127.0.0.1:12345/0123456789abcdef",
        "http://127.0.0.1:12345/0123456789abcdef?x=1",
        "http://127.0.0.1:12345/0123456789abcdef%2fsecret",
    ] {
        assert!(
            prepare(parent.path(), "model", invalid, Capabilities::default(), "").is_err(),
            "accepted {invalid}"
        );
    }
    for model in ["", "-p", "a/b", "space model"] {
        assert!(prepare(parent.path(), model, url, Capabilities::default(), "").is_err());
    }
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn unsupported_capabilities_never_create_workspace() {
    let parent = tempfile::tempdir().unwrap();
    for caps in [
        Capabilities {
            native_browser: true,
            ..Default::default()
        },
        Capabilities {
            native_subagents: true,
            ..Default::default()
        },
        Capabilities {
            structured_output: true,
            ..Default::default()
        },
    ] {
        assert!(prepare(
            parent.path(),
            "model",
            "http://127.0.0.1:12345/0123456789abcdef",
            caps,
            ""
        )
        .is_err());
    }
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
    let supported = prepare(
        parent.path(),
        "model",
        "http://127.0.0.1:12345/0123456789abcdef",
        Capabilities {
            attachments: true,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    drop(supported);
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn workspace_can_be_transferred_to_the_process_supervisor() {
    let parent = tempfile::tempdir().unwrap();
    let launch = prepare(
        parent.path(),
        "gemini-4-argon-low",
        "http://127.0.0.1:12345/0123456789abcdef0123456789abcdef",
        Capabilities::default(),
        "",
    )
    .unwrap();
    let path = launch.workspace().to_path_buf();
    let (workspace, args) = launch.into_parts();
    assert_eq!(workspace.path(), path);
    assert!(path.exists());
    assert!(args.iter().any(|arg| arg == "mew-boundary"));
    drop(workspace);
    assert!(!path.exists());
}

#[test]
fn system_instructions_are_preserved_verbatim_after_fixed_agent_preamble() {
    let parent = tempfile::tempdir().unwrap();
    let special = "\n---\nname: fake\n``` \"quoted\" Ω </system>\n\0tail";
    for system in [special, ""] {
        let launch = prepare(
            parent.path(),
            "model",
            "http://127.0.0.1:12345/0123456789abcdef",
            Capabilities::default(),
            system,
        )
        .unwrap();
        let file = launch
            .workspace()
            .join(".agents/agents/mew-boundary/agent.md");
        let bytes = fs::read(&file).unwrap();
        let marker = b"Use the available MCP interface, including `call_mcp_tool` when provided, to call those tools for workspace operations.\n";
        let offset = bytes
            .windows(marker.len())
            .position(|b| b == marker)
            .unwrap()
            + marker.len();
        assert_eq!(&bytes[offset..], system.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn tool_roster_is_in_context_and_the_marker_still_precedes_system() {
    let parent = tempfile::tempdir().unwrap();
    let system = "SYSTEM PROMPT";
    let launch = prepare_with_tools(
        parent.path(),
        "model",
        "http://127.0.0.1:12345/0123456789abcdef",
        Capabilities::default(),
        system,
        &["web_search".into(), "read".into(), "web_search".into()],
    )
    .unwrap();
    let text = fs::read_to_string(
        launch
            .workspace()
            .join(".agents/agents/mew-boundary/agent.md"),
    )
    .unwrap();

    let roster =
        "Available tools on `mew` (call directly as `mcp_mew_<name>` when declared, or via `call_mcp_tool` with `ServerName: \"mew\"` and `ToolName`): read, web_search\n";
    assert!(text.contains(roster), "got {text}");
    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(launch.workspace().join(".agents/mcp_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["mcpServers"]["mew"]["tools"]["read"]["eager"], true);
    assert_eq!(
        config["mcpServers"]["mew"]["tools"]["web_search"]["eager"],
        true
    );
    let marker = "Use the available MCP interface, including `call_mcp_tool` when provided, to call those tools for workspace operations.\n";
    let tail = text.split_once(marker).unwrap().1;
    assert_eq!(
        tail, system,
        "the system prompt must follow the marker verbatim"
    );
}

#[test]
fn aliased_tools_document_original_names_and_mark_mcp_aliases_eager() {
    let parent = tempfile::tempdir().unwrap();
    let launch = prepare_with_tool_aliases(
        parent.path(),
        "model",
        "http://127.0.0.1:12345/0123456789abcdef",
        Capabilities::default(),
        "SYS",
        &[("mew_tool_0", "pkg.read"), ("read", "read")],
    )
    .unwrap();
    let text = fs::read_to_string(
        launch
            .workspace()
            .join(".agents/agents/mew-boundary/agent.md"),
    )
    .unwrap();
    assert!(
        text.contains("mew_tool_0 (alias for pkg.read), read"),
        "got {text}"
    );
    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(launch.workspace().join(".agents/mcp_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        config["mcpServers"]["mew"]["tools"]["mew_tool_0"]["eager"],
        true
    );
    assert!(config["mcpServers"]["mew"]["tools"]
        .get("pkg.read")
        .is_none());
}
