use mew_agy::launch::{prepare, Capabilities};
use std::fs;

#[test]
fn creates_private_workspace_with_minimal_agent_and_exact_permissions() {
    let parent = tempfile::tempdir().unwrap();
    let launch = prepare(
        parent.path(),
        "gemini-4-argon-low",
        &["read", "write_file"],
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
    let args = launch.args();
    assert!(!args.iter().any(|a| a == "--dangerously-skip-permissions"));
    assert!(args
        .windows(2)
        .any(|pair| pair == ["--agent", "mew-boundary"]));
    let settings_index = args.iter().position(|a| a == "--settings").unwrap();
    let settings: serde_json::Value = serde_json::from_str(&args[settings_index + 1]).unwrap();
    assert_eq!(
        settings["permissions"]["allow"],
        serde_json::json!(["mcp(mew/read)", "mcp(mew/write_file)"])
    );
    assert_eq!(
        settings["permissions"]["deny"],
        serde_json::json!([
            "read_file(*)",
            "write_file(*)",
            "command(*)",
            "read_url(*)",
            "execute_url(*)"
        ])
    );
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
fn rejects_unsafe_names_urls_and_duplicate_grants_before_writing() {
    let parent = tempfile::tempdir().unwrap();
    let url = "http://127.0.0.1:12345/0123456789abcdef0123456789abcdef";
    for tools in [
        vec![""],
        vec!["*"],
        vec!["read)\",\"command(*)"],
        vec!["a/b"],
        vec!["safe", "safe"],
        vec!["é"],
        vec!["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
    ] {
        assert!(
            prepare(
                parent.path(),
                "gemini-4-argon-low",
                &tools,
                url,
                Capabilities::default(),
                ""
            )
            .is_err(),
            "accepted {tools:?}"
        );
    }
    for invalid in [
        "https://example.com/0123456789abcdef",
        "http://127.0.0.1:12345/",
        "http://localhost:12345/0123456789abcdef",
        "http://user@127.0.0.1:12345/0123456789abcdef",
        "http://127.0.0.1:12345/0123456789abcdef?x=1",
        "http://127.0.0.1:12345/0123456789abcdef%2fsecret",
    ] {
        assert!(
            prepare(
                parent.path(),
                "model",
                &["read"],
                invalid,
                Capabilities::default(),
                ""
            )
            .is_err(),
            "accepted {invalid}"
        );
    }
    for model in ["", "-p", "a/b", "space model"] {
        assert!(prepare(
            parent.path(),
            model,
            &["read"],
            url,
            Capabilities::default(),
            ""
        )
        .is_err());
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
            attachments: true,
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
            &[],
            "http://127.0.0.1:12345/0123456789abcdef",
            caps,
            ""
        )
        .is_err());
    }
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn workspace_can_be_transferred_to_the_process_supervisor() {
    let parent = tempfile::tempdir().unwrap();
    let launch = prepare(
        parent.path(),
        "gemini-4-argon-low",
        &["read"],
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
            &[],
            "http://127.0.0.1:12345/0123456789abcdef",
            Capabilities::default(),
            system,
        )
        .unwrap();
        let file = launch
            .workspace()
            .join(".agents/agents/mew-boundary/agent.md");
        let bytes = fs::read(&file).unwrap();
        let marker = b"Use the available client tools to respond to the request.\n";
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
