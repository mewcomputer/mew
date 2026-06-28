//! Integration test for the subprocess plugin runtime.
//!
//! Uses the sample-plugin example binary (cargo build --example sample-plugin).

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use mew_hooks::{Dispatcher, PermissionDecision, PluginHost, ToolCall, ToolOutput};
use mew_hooks_runtime::SubprocessDispatcher;

fn test_host() -> PluginHost {
    PluginHost {
        notify: Arc::new(|msg| eprintln!("[plugin-notify] {msg}")),
        config_read: Arc::new(|_key| None),
        log: Arc::new(|msg| eprintln!("[plugin-log] {msg}")),
        storage_read: Arc::new(|_key| None),
        storage_write: Arc::new(|_key, _val| {}),
        storage_delete: Arc::new(|_key| {}),
        set_ui: Arc::new(|_key, _val| {}),
    }
}

/// Find the sample plugin binary.
fn sample_plugin_path() -> PathBuf {
    // Try the target directory first (cargo test --examples sets this up)
    if let Ok(path) = env::var("CARGO_BIN_EXE_sample-plugin") {
        return PathBuf::from(path);
    }
    // Fallback: look relative to CARGO_MANIFEST_DIR (tests/..)
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = manifest
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(&manifest)
        .join("target")
        .join("debug")
        .join("examples")
        .join("sample-plugin");

    if target.exists() {
        return target;
    }

    // Last resort: build it on the fly
    let status = Command::new("cargo")
        .args(["build", "--example", "sample-plugin"])
        .current_dir(&manifest)
        .status()
        .expect("cargo build example");

    assert!(status.success(), "failed to build sample-plugin example");

    assert!(
        target.exists(),
        "sample-plugin binary not found at {:?}",
        target
    );
    target
}

fn make_plugin_dir_with_binary() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = sample_plugin_path();
    let dst = dir.path().join("sample-plugin");
    std::fs::copy(&src, &dst).expect("copy plugin binary");
    // Make executable on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dst, perms).unwrap();
    }
    dir
}

#[tokio::test]
async fn test_plugin_init_and_shutdown() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;
    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_adds_header() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("application/json"),
    );

    let result = dispatcher.on_chat_headers(headers).await;
    let plugin_header = result.get("x-plugin");
    assert!(plugin_header.is_some(), "plugin should add x-plugin header");
    assert_eq!(plugin_header.unwrap().to_str().unwrap(), "sample-plugin");

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_denies_permission() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    let call = ToolCall {
        tool_name: "bash".to_string(),
        call_id: "call-1".to_string(),
        input: serde_json::json!({"command": "rm -rf /"}),
    };

    let decision = dispatcher
        .on_permission_ask(&call, PermissionDecision::AllowOnce)
        .await;
    assert_eq!(
        decision,
        mew_hooks::HookOutcome::Proceed(PermissionDecision::Deny),
        "plugin should deny AllowOnce"
    );

    let decision2 = dispatcher
        .on_permission_ask(&call, PermissionDecision::AllowSession)
        .await;
    assert_eq!(
        decision2,
        mew_hooks::HookOutcome::Proceed(PermissionDecision::AllowSession),
        "AllowSession should pass through"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_redacts_secrets() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    dispatcher.init(&test_host()).await;

    let call = ToolCall {
        tool_name: "read".to_string(),
        call_id: "call-2".to_string(),
        input: serde_json::json!({"path": "/tmp/config"}),
    };

    let output = ToolOutput {
        output: "api_key=SECRET, host=localhost".to_string(),
        error: String::new(),
        diff: None,
        metadata: None,
    };

    let result = dispatcher.on_tool_execute_after(&call, output).await;
    assert!(
        !result.output.contains("SECRET"),
        "plugin should redact SECRET, got: {}",
        result.output
    );
    assert!(
        result.output.contains("***REDACTED***"),
        "plugin should replace SECRET with ***REDACTED***"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_register_tools() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");

    let tools = dispatcher.on_register_tools().await;
    assert_eq!(tools.len(), 1, "sample-plugin registers one tool");
    assert_eq!(tools[0].name, "sample-echo");
    assert!(!tools[0].description.is_empty());
}

#[tokio::test]
async fn test_no_plugins_graceful() {
    let empty_dir = tempfile::tempdir().expect("tempdir");
    let dispatcher =
        SubprocessDispatcher::from_dirs(vec![empty_dir.path().to_path_buf()], test_host())
            .await
            .expect("dispatcher creation with empty dir");

    // Should not crash
    dispatcher.init(&test_host()).await;
    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_transforms_system_prompt() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    let result = dispatcher.on_system_prompt("hello world".into()).await;
    assert!(
        result.contains("[sample-plugin]"),
        "plugin should prepend its tag to system prompt, got: {result}"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_on_turn_end_notification() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    // Should not panic; notification is fire-and-forget.
    dispatcher.on_turn_end(&[]).await;

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_registers_slash_commands() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    let cmds = dispatcher.on_register_slash_commands().await;
    assert!(
        !cmds.is_empty(),
        "sample-plugin should register slash commands"
    );
    assert!(
        cmds.iter().any(|c| c.name == "/sample-plugin"),
        "should register /sample-plugin command"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_executes_slash_command() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    let result = dispatcher
        .execute_slash_command("/sample-plugin", "greet")
        .await;
    assert!(result.is_some(), "slash command should return a result");
    assert!(
        result.unwrap().contains("sample-plugin"),
        "slash command result should include plugin name"
    );

    dispatcher.shutdown().await;
}

#[tokio::test]
async fn test_plugin_slash_command_returns_result_for_any_input() {
    let dir = make_plugin_dir_with_binary();
    let dispatcher = SubprocessDispatcher::from_dirs(vec![dir.path().to_path_buf()], test_host())
        .await
        .expect("dispatcher creation");
    dispatcher.init(&test_host()).await;

    // The sample plugin handles execute-slash-command for any command.
    let result = dispatcher
        .execute_slash_command("/anything", "arg1 arg2")
        .await;
    assert!(
        result.is_some(),
        "plugin should return a result for any slash command"
    );
    assert!(
        result.unwrap().contains("sample-plugin"),
        "result should identify the plugin"
    );

    dispatcher.shutdown().await;
}
